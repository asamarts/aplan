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
