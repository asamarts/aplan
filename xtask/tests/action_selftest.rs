//! The action self-test tests the action, not the last release (`A1-278`).
//!
//! `Action self-test` runs `action.yml` from the checkout against a **pinned
//! release**, and it used to point that release at this repository's own plan.
//! `.aplan.yml` tracks `main` and has grown eight keys since the newest tag, so
//! the job had quietly become a compatibility test between today's configuration
//! and the last one released. It only triggers on a change to `action.yml`, so
//! nobody saw it for a month, and then it failed on `unknown field
//! effort_days_per_week` with nothing about the action wrong.
//!
//! The fix is a fixture plan written to what the pinned release can parse. What
//! makes it stay fixed is here: the two ways it comes undone are re-pointing the
//! job at `aplan/`, and adding a modern key to the fixture's config. Both are
//! one-line edits that look harmless and turn the job red weeks later.
//!
//! Read from the repository's own files rather than by running anything: a
//! GitHub workflow cannot be executed here, and asserting on the YAML is what is
//! actually available. It is narrow on purpose.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::{fs, os::unix::fs::PermissionsExt};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits under the workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn write_executable(path: &Path, text: &str) {
    fs::write(path, text).expect("write executable fixture");
    let mut permissions = fs::metadata(path)
        .expect("read executable fixture metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fixture executable");
}

fn run_release_verifier(
    tag: &str,
    archive_bytes: &[u8],
    checksum: &str,
) -> (Output, Option<String>) {
    run_release_verifier_with(tag, "asamarts/aplan", archive_bytes, checksum, 0)
}

fn run_release_verifier_with(
    tag: &str,
    repository: &str,
    archive_bytes: &[u8],
    checksum: &str,
    gh_status: u8,
) -> (Output, Option<String>) {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let archive = dir
        .path()
        .join(format!("aplan-{tag}-x86_64-unknown-linux-musl.tar.gz"));
    let sidecar = dir.path().join(format!(
        "{}.sha256",
        archive.file_name().unwrap().to_string_lossy()
    ));
    let bin = dir.path().join("bin");
    let log = dir.path().join("gh.log");
    fs::create_dir(&bin).expect("create mock bin");
    fs::write(&archive, archive_bytes).expect("write archive");
    fs::write(&sidecar, checksum).expect("write checksum");
    write_executable(
        &bin.join("gh"),
        "#!/usr/bin/env bash\nset -eu\nprintf '%s\\n' \"$@\" > \"$GH_LOG\"\nexit \"${GH_STATUS:-0}\"\n",
    );

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH is set")
    );
    let output = Command::new("bash")
        .arg(repo_root().join("ci/scripts/verify-action-release.sh"))
        .args([
            tag,
            repository,
            archive.to_str().expect("archive path is UTF-8"),
            sidecar.to_str().expect("checksum path is UTF-8"),
        ])
        .env("PATH", path)
        .env("GH_TOKEN", "fixture-token")
        .env("GH_LOG", &log)
        .env("GH_STATUS", gh_status.to_string())
        .output()
        .expect("run action release verifier");
    let log = fs::read_to_string(log).ok();
    (output, log)
}

fn fixture_checksum(tag: &str, digest: &str) -> String {
    format!("{digest}  aplan-{tag}-x86_64-unknown-linux-musl.tar.gz\n")
}

#[test]
fn attested_action_releases_bind_the_archive_to_the_exact_tag_before_extraction() {
    let digest = "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb";
    let (output, log) = run_release_verifier("v0.6.0", b"a", &fixture_checksum("v0.6.0", digest));
    assert!(
        output.status.success(),
        "attested release failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let arguments: Vec<_> = log
        .expect("gh was invoked")
        .lines()
        .map(str::to_owned)
        .collect();
    let archive = arguments[2].clone();
    assert_eq!(
        arguments,
        [
            "attestation",
            "verify",
            archive.as_str(),
            "--repo",
            "asamarts/aplan",
            "--predicate-type",
            "https://slsa.dev/provenance/v1",
            "--cert-oidc-issuer",
            "https://token.actions.githubusercontent.com",
            "--signer-workflow",
            "asamarts/aplan/.github/workflows/release.yml",
            "--source-ref",
            "refs/tags/v0.6.0",
            "--deny-self-hosted-runners",
        ],
        "the consumer verifier's identity policy drifted"
    );

    let action = read("action.yml");
    let verify = action
        .find("verify-action-release.sh")
        .expect("action invokes release verifier");
    let extract = action.find("tar -xzf").expect("action extracts archive");
    assert!(
        verify < extract,
        "the action extracts before provenance succeeds"
    );
}

#[test]
fn historical_action_releases_have_one_finite_checksum_only_exemption() {
    let digest = "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb";
    for tag in [
        "v0.1.0", "v0.2.0", "v0.3.0", "v0.4.0", "v0.5.0", "v0.5.1", "v0.5.2",
    ] {
        let (output, log) = run_release_verifier(tag, b"a", &fixture_checksum(tag, digest));
        assert!(
            output.status.success(),
            "historical release {tag} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            log.is_none(),
            "historical release {tag} asked for absent provenance"
        );
    }

    let (output, log) = run_release_verifier("v0.5.3", b"a", &fixture_checksum("v0.5.3", digest));
    assert!(output.status.success(), "later releases require provenance");
    assert!(
        log.is_some(),
        "an unlisted later release fell back to checksum-only verification"
    );
}

#[test]
fn action_release_verification_refuses_bad_identity_and_bytes_before_gh() {
    let digest = "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb";
    for tag in ["v0.6", "v0.6.0-dev", "v00.6.0", "main"] {
        let (output, log) = run_release_verifier(tag, b"a", &fixture_checksum(tag, digest));
        assert!(!output.status.success(), "malformed tag {tag} passed");
        assert!(log.is_none(), "malformed tag {tag} reached gh");
    }

    let wrong_subject = fixture_checksum("v0.6.1", digest);
    let (output, log) = run_release_verifier("v0.6.0", b"a", &wrong_subject);
    assert!(
        !output.status.success(),
        "checksum for another archive passed"
    );
    assert!(log.is_none(), "bad checksum subject reached gh");

    let (output, log) =
        run_release_verifier("v0.6.0", b"changed", &fixture_checksum("v0.6.0", digest));
    assert!(
        !output.status.success(),
        "changed archive passed its checksum"
    );
    assert!(log.is_none(), "changed archive reached gh");

    for repository in ["asamarts", "/aplan", "asamarts/aplan/other"] {
        let (output, log) = run_release_verifier_with(
            "v0.6.0",
            repository,
            b"a",
            &fixture_checksum("v0.6.0", digest),
            0,
        );
        assert!(
            !output.status.success(),
            "malformed repository {repository} passed"
        );
        assert!(
            log.is_none(),
            "malformed repository {repository} reached gh"
        );
    }

    let (output, log) = run_release_verifier_with(
        "v0.6.0",
        "asamarts/aplan",
        b"a",
        &fixture_checksum("v0.6.0", digest),
        19,
    );
    assert!(
        !output.status.success(),
        "a failed provenance lookup passed"
    );
    assert!(
        log.is_some(),
        "the failed provenance lookup never reached gh"
    );
}

fn run_release_downloader(tag: &str, gh_script: &str) -> (Output, usize) {
    run_release_downloader_with_backoff(tag, Some("0 0 0"), gh_script)
}

fn run_release_downloader_with_backoff(
    tag: &str,
    backoff: Option<&str>,
    gh_script: &str,
) -> (Output, usize) {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let bin = dir.path().join("bin");
    let dest = dir.path().join("dest");
    let count_file = dir.path().join("attempts");
    fs::create_dir(&bin).expect("create mock bin");
    fs::create_dir(&dest).expect("create dest");
    fs::write(&count_file, "0\n").expect("init attempt counter");
    write_executable(&bin.join("gh"), gh_script);

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH is set")
    );
    let asset = format!("aplan-{tag}-x86_64-unknown-linux-musl.tar.gz");
    let mut cmd = Command::new("bash");
    cmd.arg(repo_root().join("ci/scripts/download-action-release.sh"))
        .args([
            tag,
            "asamarts/aplan",
            asset.as_str(),
            dest.to_str().expect("dest path is UTF-8"),
        ])
        .env("PATH", path)
        .env("ATTEMPT_FILE", &count_file);
    match backoff {
        Some(spec) => {
            cmd.env("APLAN_DOWNLOAD_BACKOFF_SECONDS", spec);
        }
        None => {
            cmd.env_remove("APLAN_DOWNLOAD_BACKOFF_SECONDS");
        }
    }
    let output = cmd.output().expect("run action release downloader");
    let attempts = fs::read_to_string(&count_file)
        .expect("read attempt counter")
        .trim()
        .parse::<usize>()
        .expect("integer attempt count");
    (output, attempts)
}

#[test]
fn action_release_download_retries_transient_5xx_and_network_errors_with_backoff() {
    let transient_500 = r#"#!/usr/bin/env bash
set -eu
count=$(( $(cat "$ATTEMPT_FILE") + 1 ))
printf '%s\n' "$count" > "$ATTEMPT_FILE"
if (( count < 3 )); then
    echo "HTTP 500 (https://api.github.com/repos/asamarts/aplan/releases/assets/711204551)" >&2
    exit 1
fi
exit 0
"#;
    let (output, attempts) = run_release_downloader("v0.6.0", transient_500);
    assert!(
        output.status.success(),
        "transient HTTP 500 should succeed on retry: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(attempts, 3, "expected 2 failed attempts + 1 success");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Transient error downloading aplan v0.6.0"),
        "retry attempt should be logged: {stderr}"
    );

    let transient_network = r#"#!/usr/bin/env bash
set -eu
count=$(( $(cat "$ATTEMPT_FILE") + 1 ))
printf '%s\n' "$count" > "$ATTEMPT_FILE"
if (( count == 1 )); then
    echo 'Post "https://api.github.com/graphql": dial tcp 140.82.121.6:443: i/o timeout' >&2
    exit 1
fi
exit 0
"#;
    let (output, attempts) = run_release_downloader("v0.6.0", transient_network);
    assert!(
        output.status.success(),
        "transient network error should succeed on retry: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(attempts, 2);

    let persistent_503 = r#"#!/usr/bin/env bash
set -eu
count=$(( $(cat "$ATTEMPT_FILE") + 1 ))
printf '%s\n' "$count" > "$ATTEMPT_FILE"
echo "HTTP 503 Service Unavailable" >&2
exit 1
"#;
    let (output, attempts) = run_release_downloader("v0.6.0", persistent_503);
    assert!(!output.status.success(), "exhausted retries must fail");
    assert_eq!(attempts, 4, "1 initial try + 3 retries");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Install failed:") && stderr.contains("no plan check ran"),
        "final failure must state install failed and no plan check ran: {stderr}"
    );

    // `gh` connectivity wrapper message is also classified as transient (`A2-190`).
    let connect_wrapper = r#"#!/usr/bin/env bash
set -eu
count=$(( $(cat "$ATTEMPT_FILE") + 1 ))
printf '%s\n' "$count" > "$ATTEMPT_FILE"
if (( count == 1 )); then
    echo "error connecting to api.github.com" >&2
    echo "check your internet connection or https://githubstatus.com" >&2
    exit 1
fi
exit 0
"#;
    let (output, attempts) = run_release_downloader("v0.6.0", connect_wrapper);
    assert!(
        output.status.success(),
        "gh connectivity wrapper error should succeed on retry: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(attempts, 2);

    // Explicit empty backoff means single attempt (0 retries) without set -u errors (`A2-190`).
    let (output, attempts) =
        run_release_downloader_with_backoff("v0.6.0", Some(""), persistent_503);
    assert!(!output.status.success(), "single attempt 503 must fail");
    assert_eq!(attempts, 1, "empty backoff spec must run one attempt only");

    // Non-numeric backoff is rejected before invoking gh (`A2-190`).
    let (output, attempts) =
        run_release_downloader_with_backoff("v0.6.0", Some("abc"), persistent_503);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(attempts, 0, "invalid backoff must fail before running gh");
}

#[test]
fn action_release_download_never_retries_permanent_errors_and_action_verifies_without_retry() {
    for message in [
        "HTTP 404: Not Found (https://api.github.com/repos/asamarts/aplan/releases/tags/v0.9.9)",
        "release not found",
        "HTTP 403: Forbidden",
    ] {
        let script = format!(
            "#!/usr/bin/env bash\nset -eu\ncount=$(( $(cat \"$ATTEMPT_FILE\") + 1 ))\nprintf '%s\\n' \"$count\" > \"$ATTEMPT_FILE\"\necho '{message}' >&2\nexit 1\n"
        );
        let (output, attempts) = run_release_downloader("v0.9.9", &script);
        assert!(
            !output.status.success(),
            "permanent error `{message}` passed"
        );
        assert_eq!(
            attempts, 1,
            "permanent error `{message}` was retried ({attempts} attempts)"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Install failed:") && stderr.contains("no plan check ran"),
            "permanent failure must state install failed and no plan check ran: {stderr}"
        );
    }

    let action = read("action.yml");
    let download = action
        .find("download-action-release.sh")
        .expect("action invokes retrying download helper");
    let verify = action
        .find("verify-action-release.sh")
        .expect("action invokes release verifier");
    let extract = action.find("tar -xzf").expect("action extracts archive");
    assert!(
        download < verify && verify < extract,
        "action must download, then verify without retry, then extract"
    );
    assert!(
        action.contains("no plan check ran"),
        "action must tell the user no plan check ran when install verification fails"
    );
}

/// Every top-level `.aplan.yml` key the pinned release knows.
///
/// Read out of the released binary rather than out of this checkout, which is
/// the whole point: `aplan facts` on the artifact published for the pinned tag
/// reports what *it* accepts, and the source tree reports what the next release
/// will. Taking it from the wrong one is how the fixture drifts ahead of the
/// release and the workflow goes red a month later with nothing wrong in
/// `action.yml` (`A1-278`).
///
/// ```console
/// $ ./aplan --version
/// aplan 0.3.0
/// $ ./aplan facts | jq -r '.config_keys[] | split(".")[0]' | sort -u
/// ```
///
/// Twenty top-level keys of the sixty-one `v0.3.0` accepts; the rest are nested
/// under one of these and arrive with it.
const KEYS_V0_3_0: &[&str] = &[
    "advisory",
    "deny_warnings",
    "detect_drift",
    "docs",
    "effort_days_per_week",
    "evidence",
    "id_strategy",
    "no_traces",
    "policy",
    "require_cut_membership",
    "require_effort",
    "roadmap",
    "root",
    "stale_after_days",
    "tag_families",
    "test",
    "traces_external",
    "traces_root",
    "url",
    "version",
];

/// The workflow checks the fixture, and not this repository's plan.
#[test]
fn the_self_test_runs_against_the_fixture_plan() {
    let wf = read(".github/workflows/action-selftest.yml");
    assert!(
        wf.contains("working-directory: ci/fixtures/action-selftest"),
        "the self-test no longer points at the fixture, so a pinned release is \
         reading a configuration that tracks `main` again"
    );
    // Every invocation of the action, not just the first: the second run is the
    // `verify` case, and leaving that one on the repository's plan would fail
    // for the same reason with half the symptom. Counted rather than fixed at a
    // number, so adding a step does not need this test edited -- only the
    // property kept.
    let runs = wf.matches("uses: ./").count();
    let in_fixture = wf
        .matches("working-directory: ci/fixtures/action-selftest")
        .count();
    assert!(
        runs >= 2,
        "the self-test stopped running the action twice:\n{wf}"
    );
    assert!(
        in_fixture >= runs,
        "{runs} step(s) run the action and only {in_fixture} name the fixture, so one reads a \
         plan outside it:\n{wf}"
    );
    assert!(
        !wf.contains("root: aplan\n"),
        "an action step still names this repository's plan root:\n{wf}"
    );
    assert!(
        wf.contains("  attestations: read\n"),
        "the live action self-test cannot fetch provenance with its restricted token:\n{wf}"
    );
}

/// And the fixture stays parseable by the version the workflow pins.
///
/// The failure this prevents is the original one, reintroduced: a key added here
/// out of habit is invisible until the workflow next runs, which may be a month.
#[test]
fn the_fixture_config_uses_only_keys_the_pinned_release_knows() {
    let cfg = read("ci/fixtures/action-selftest/.aplan.yml");
    let unknown: Vec<&str> = cfg
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        // Top-level keys only: a nested one belongs to a key already checked.
        .filter(|l| !l.starts_with([' ', '\t', '-']))
        .filter_map(|l| l.split_once(':').map(|(k, _)| k))
        .filter(|k| !KEYS_V0_3_0.contains(k))
        .collect();
    assert!(
        unknown.is_empty(),
        "the fixture's config uses {unknown:?}, which the release the workflow pins cannot \
         parse. Widen `KEYS_V0_3_0` only when the pinned version is raised too"
    );
}

/// The fixture's plan is one the pinned release can load.
///
/// From the same source as the keys above, and for the same reason: `aplan
/// schema` on the published artifact reports the task shape *it* validates
/// against. `v0.3.0` added `prefer_after`, `revisit_at`, `version_impact` and
/// `version_impact_reason` to what `v0.2.0` knew.
#[test]
fn the_fixture_tasks_carry_only_fields_the_pinned_release_knows() {
    const FIELDS: &[&str] = &[
        "blocked_by",
        "effort_days",
        "evidence",
        "id",
        "owner",
        "paths",
        "phase",
        "prefer_after",
        "revisit_at",
        "status",
        "tags",
        "title",
        "trace_fingerprints",
        "traces",
        "version_impact",
        "version_impact_reason",
    ];
    let dir = repo_root().join("ci/fixtures/action-selftest/tasks");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("fixture tasks") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&path).expect("read task");
        let front = text
            .split("---")
            .nth(1)
            .unwrap_or_else(|| panic!("{} has no frontmatter", path.display()));
        for line in front.lines() {
            if line.starts_with([' ', '\t', '-']) || line.trim().is_empty() {
                continue;
            }
            let Some((key, _)) = line.split_once(':') else {
                continue;
            };
            assert!(
                FIELDS.contains(&key),
                "{} declares `{key}`, which the release the workflow pins does not know",
                path.display()
            );
        }
    }
    assert!(
        seen >= 2,
        "the fixture lost its tasks: {seen} found in {dir:?}"
    );
}
