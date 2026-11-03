//! The executable validation profiles agree across manifest, generated table,
//! local preflight and GitHub Actions (`A1-611`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use crate::fixture_executable::install_executable as write_executable;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Start an authoritative profile without inheriting the observational
/// latest-stable lane's process-local compiler override (`A1-675`).
///
/// The fixture's committed `rust-toolchain.toml` remains the source of truth.
/// Keeping this boundary in one constructor prevents a new profile journey
/// from quietly repeating the leak caught by scheduled run 40490555697.
fn authoritative_validation(path: impl AsRef<Path>) -> Command {
    let mut command = Command::new(path.as_ref());
    command.env_remove("RUSTUP_TOOLCHAIN");
    command
}

fn validate(args: &[&str]) -> Output {
    authoritative_validation(workspace_root().join("ci/scripts/validate.sh"))
        .args(args)
        .env_remove("PREFLIGHT_SKIP")
        .current_dir(workspace_root())
        .output()
        .expect("run validation entry point")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("validation stdout is UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("validation stderr is UTF-8")
}

#[cfg(unix)]
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git for release fixture");
    assert!(output.status.success(), "{}", stderr(&output));
}

#[cfg(unix)]
fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git for release fixture");
    assert!(output.status.success(), "{}", stderr(&output));
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .trim()
        .to_string()
}

#[cfg(unix)]
fn sha256(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("hash release fixture input");
    assert!(output.status.success(), "{}", stderr(&output));
    String::from_utf8(output.stdout)
        .expect("sha256sum output is UTF-8")
        .split_ascii_whitespace()
        .next()
        .expect("sha256sum emitted a digest")
        .to_string()
}

#[cfg(unix)]
fn release_fixture() -> tempfile::TempDir {
    let fixture = tempfile::Builder::new()
        .prefix("aplan-validation-release-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create release validation fixture");
    let scripts = fixture.path().join("ci/scripts");
    std::fs::create_dir_all(&scripts).expect("create fixture scripts directory");
    write_executable(
        &scripts.join("validate.sh"),
        include_str!("../../ci/scripts/validate.sh"),
    );
    write_executable(
        &scripts.join("validation-receipt.py"),
        include_str!("../../ci/scripts/validation-receipt.py"),
    );
    std::fs::write(
        scripts.join("validation-generated.sh"),
        include_str!("../../ci/scripts/validation-generated.sh"),
    )
    .expect("write generated inventory fixture");
    write_executable(
        &scripts.join("toolchain.sh"),
        include_str!("../../ci/scripts/toolchain.sh"),
    );
    write_executable(
        &scripts.join("release-source.sh"),
        include_str!("../../ci/scripts/release-source.sh"),
    );
    write_executable(
        &scripts.join("current-version.sh"),
        include_str!("../../ci/scripts/current-version.sh"),
    );
    std::fs::write(
        fixture.path().join("ci/validation.yml"),
        include_str!("../../ci/validation.yml"),
    )
    .expect("write validation inventory fixture");
    std::fs::write(
        fixture.path().join("rust-toolchain.toml"),
        include_str!("../../rust-toolchain.toml"),
    )
    .expect("write toolchain contract fixture");
    std::fs::write(
        fixture.path().join("Cargo.toml"),
        "[workspace.package]\nversion = \"0.5.0-dev\"\n",
    )
    .expect("write workspace version fixture");
    for gate in [
        "fmt",
        "clippy",
        "doc",
        "test",
        "spec",
        "msrv-build",
        "style",
        "supply-chain",
        "coverage",
        "structural",
        "evidence",
    ] {
        write_executable(
            &scripts.join(format!("{gate}.sh")),
            &format!(
                "#!/usr/bin/env bash\nset -euo pipefail\nfor variable in APLAN_VALIDATION_INPUT_KIND APLAN_VALIDATION_BASE_REV APLAN_VALIDATION_HEAD_REV APLAN_VALIDATION_QUEUE_SECONDS APLAN_VALIDATION_SETUP_SECONDS APLAN_VALIDATION_SETUP_STARTED_EPOCH APLAN_VALIDATION_RECEIPT GITHUB_STEP_SUMMARY; do [[ -z \"${{!variable-}}\" ]] || {{ echo \"leaked profile variable: $variable\" >&2; exit 99; }}; done\nprintf '%s\\n' '{gate}' >> \"${{APLAN_TEST_GATE_LOG:-/dev/null}}\"\n"
            ),
        );
    }
    git(fixture.path(), &["init", "-q"]);
    git(
        fixture.path(),
        &["config", "user.email", "validation@example.invalid"],
    );
    git(
        fixture.path(),
        &["config", "user.name", "Validation Fixture"],
    );
    git(fixture.path(), &["add", "."]);
    git(fixture.path(), &["commit", "-q", "-m", "fixture"]);
    fixture
}

#[cfg(unix)]
fn release(root: &Path) -> Output {
    release_with(root, &[])
}

#[cfg(unix)]
fn release_with(root: &Path, environment: &[(&str, &str)]) -> Output {
    let source = git_stdout(root, &["rev-parse", "HEAD"]);
    let mut command = authoritative_validation(root.join("ci/scripts/validate.sh"));
    command
        .args(["--profile", "release"])
        .env_remove("PREFLIGHT_SKIP")
        .env_remove("TAG")
        .env_remove("GITHUB_STEP_SUMMARY")
        .env("APLAN_RELEASE_SHA", source)
        .current_dir(root);
    for (name, value) in environment {
        command.env(name, value);
    }
    command.output().expect("run release validation fixture")
}

fn line<'a>(output: &'a str, prefix: &str) -> &'a str {
    output
        .lines()
        .find(|candidate| candidate.starts_with(prefix))
        .unwrap_or_else(|| panic!("missing `{prefix}` in:\n{output}"))
}

#[cfg(unix)]
fn assert_hostile_receipt_targets_refused(fixture: &Path, artifacts: &Path) {
    use std::os::unix::fs::symlink;

    let outside = artifacts.join("outside");
    std::fs::write(&outside, "untouched\n").expect("write hostile receipt target");
    let hostile = artifacts.join("hostile.json");
    symlink(&outside, &hostile).expect("link hostile receipt target");
    let refused = authoritative_validation(fixture.join("ci/scripts/validate.sh"))
        .args(["--profile", "development"])
        .env_remove("PREFLIGHT_SKIP")
        .env("APLAN_VALIDATION_RECEIPT", &hostile)
        .current_dir(fixture)
        .output()
        .expect("refuse hostile receipt output");
    assert_eq!(refused.status.code(), Some(2));
    assert!(stderr(&refused).contains("receipt output already exists"));
    assert_eq!(
        std::fs::read_to_string(outside).expect("hostile target remains"),
        "untouched\n"
    );

    let real_parent = artifacts.join("real-parent");
    std::fs::create_dir(&real_parent).expect("create real receipt parent");
    let linked_parent = artifacts.join("linked-parent");
    symlink(&real_parent, &linked_parent).expect("link hostile receipt parent");
    let through_parent = linked_parent.join("receipt.json");
    let refused = authoritative_validation(fixture.join("ci/scripts/validate.sh"))
        .args(["--profile", "development"])
        .env_remove("PREFLIGHT_SKIP")
        .env("APLAN_VALIDATION_RECEIPT", &through_parent)
        .current_dir(fixture)
        .output()
        .expect("refuse receipt output through a symlinked ancestor");
    assert_eq!(refused.status.code(), Some(2));
    assert!(stderr(&refused).contains("parent must be a real directory"));
    assert!(!real_parent.join("receipt.json").exists());
}

#[cfg(unix)]
#[test]
fn every_executed_profile_emits_one_bounded_host_neutral_receipt() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = release_fixture();
    let source = git_stdout(fixture.path(), &["rev-parse", "HEAD"]);
    let inventory = sha256(&fixture.path().join("ci/validation.yml"));
    let artifacts = tempfile::Builder::new()
        .prefix("aplan-validation-receipt-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create receipt artifact directory");
    let receipt = artifacts.path().join("receipt.json");
    let summary = artifacts.path().join("summary.md");
    let setup_started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("validation clock follows the Unix epoch")
        .as_secs()
        .to_string();
    let output = authoritative_validation(fixture.path().join("ci/scripts/validate.sh"))
        .args([
            "--profile",
            "development",
            "--select",
            "test",
            "--package",
            "aplan",
        ])
        .env_remove("PREFLIGHT_SKIP")
        .env("APLAN_VALIDATION_RECEIPT", &receipt)
        .env("GITHUB_STEP_SUMMARY", &summary)
        .env("APLAN_VALIDATION_INPUT_KIND", "base-head")
        .env("APLAN_VALIDATION_BASE_REV", &source)
        .env("APLAN_VALIDATION_HEAD_REV", &source)
        .env("APLAN_VALIDATION_QUEUE_SECONDS", "3")
        .env("APLAN_VALIDATION_SETUP_STARTED_EPOCH", setup_started)
        .env("RUNNER_ENVIRONMENT", "self-hosted")
        .env("RUNNER_OS", "Linux")
        .env("RUNNER_ARCH", "X64")
        .current_dir(fixture.path())
        .output()
        .expect("run development receipt fixture");
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = stdout(&output);
    assert_eq!(
        stdout.matches("validation: receipt=").count(),
        1,
        "{stdout}"
    );

    let bytes = std::fs::read(&receipt).expect("read profile receipt");
    assert!(bytes.len() <= 32_768);
    assert_eq!(
        std::fs::metadata(&receipt)
            .expect("receipt metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let document: serde_json::Value =
        serde_json::from_slice(&bytes).expect("parse profile receipt");
    assert_eq!(document["schema"], "aplan/validation-receipt/v1");
    assert_eq!(document["profile"], "development");
    assert_eq!(document["assurance"], "selected");
    assert_eq!(document["outcome"], "passed");
    assert_eq!(document["source"]["commit"], source);
    assert_eq!(document["source"]["input"], "base-head");
    assert_eq!(document["inventory"]["sha256"], inventory);
    assert_eq!(document["runner"]["environment"], "self-hosted");
    assert_eq!(document["phases"]["queue"]["duration_ms"], 3000);
    assert_eq!(document["phases"]["setup"]["state"], "observed");
    assert!(document["phases"]["setup"]["duration_ms"].is_number());
    assert_eq!(document["phases"]["compile"]["state"], "deferred");
    assert_eq!(document["phases"]["test"]["state"], "observed");
    assert_eq!(document["phases"]["provider"]["state"], "deferred");
    assert_eq!(document["phases"]["coverage"]["state"], "deferred");
    assert!(
        !String::from_utf8_lossy(&bytes).contains(&fixture.path().to_string_lossy().to_string()),
        "receipt leaked its host fixture root"
    );
    let summary = std::fs::read_to_string(summary).expect("read profile summary");
    assert!(summary.contains("### aplan validation receipt"));
    assert!(summary.contains("Machine receipt"));
    assert_hostile_receipt_targets_refused(fixture.path(), artifacts.path());
}

#[test]
fn validation_inventory_is_complete_and_orders_assurance() {
    let generated = Command::new(crate::common::xtask())
        .args(["validation", "--check"])
        .current_dir(workspace_root())
        .output()
        .expect("audit generated validation inventory");
    assert!(generated.status.success(), "{}", stderr(&generated));

    let full = validate(&["--profile", "full", "--plan"]);
    assert!(full.status.success(), "{}", stderr(&full));
    let full_stdout = stdout(&full);
    assert!(full_stdout.contains("profile=full assurance=full"));
    assert_eq!(
        line(&full_stdout, "validation: selected gates:"),
        "validation: selected gates: fmt clippy doc test spec msrv-build style supply-chain coverage structural evidence"
    );
    assert_eq!(
        line(&full_stdout, "validation: widened gates:"),
        "validation: widened gates: (none)"
    );
    assert_eq!(
        line(&full_stdout, "validation: deferred gates:"),
        "validation: deferred gates: (none)"
    );

    let release = validate(&["--profile", "release", "--plan"]);
    assert!(release.status.success(), "{}", stderr(&release));
    let release_stdout = stdout(&release);
    assert!(release_stdout.contains("profile=release assurance=release"));
    assert_eq!(
        line(&release_stdout, "validation: selected gates:"),
        line(&full_stdout, "validation: selected gates:")
    );

    let preflight = std::fs::read_to_string(workspace_root().join("ci/scripts/preflight.sh"))
        .expect("read preflight");
    assert!(preflight.contains("validate.sh\" --profile full"));
    assert!(preflight.contains("--profile development"));
    assert!(!preflight.contains("PREFLIGHT_SKIP"));
}

#[test]
fn validation_entry_point_reports_development_dispositions_and_rejects_subtraction() {
    let development = validate(&[
        "--profile",
        "development",
        "--select",
        "test",
        "--package",
        "aplan",
        "--widen",
        "unknown input",
        "--plan",
    ]);
    assert!(development.status.success(), "{}", stderr(&development));
    let output = stdout(&development);
    assert!(output.contains("profile=development assurance=selected"));
    assert_eq!(
        line(&output, "validation: selected gates:"),
        "validation: selected gates: fmt test style structural"
    );
    assert_eq!(
        line(&output, "validation: widened gates:"),
        "validation: widened gates: clippy doc spec msrv-build supply-chain"
    );
    assert_eq!(
        line(&output, "validation: deferred gates:"),
        "validation: deferred gates: coverage evidence"
    );
    assert_eq!(
        line(&output, "validation: selected packages:"),
        "validation: selected packages: aplan"
    );
    assert_eq!(
        line(&output, "validation: widening reasons:"),
        "validation: widening reasons: unknown input"
    );

    let mut skip = authoritative_validation(workspace_root().join("ci/scripts/validate.sh"));
    let skip = skip
        .args(["--profile", "full", "--plan"])
        .env("PREFLIGHT_SKIP", "test")
        .current_dir(workspace_root())
        .output()
        .expect("try legacy subtraction");
    assert_eq!(skip.status.code(), Some(2));
    assert!(stderr(&skip).contains("cannot subtract"));

    for args in [
        vec!["--profile", "full", "--select", "test", "--plan"],
        vec!["--profile", "full", "--package", "aplan", "--plan"],
        vec!["--profile", "release", "--widen", "anything", "--plan"],
        vec![
            "--profile",
            "development",
            "--package",
            "aplan",
            "--package",
            "aplan",
            "--plan",
        ],
        vec!["--profile", "invented", "--plan"],
        vec!["--gate", "invented"],
        vec!["--unknown"],
    ] {
        let rejected = validate(&args);
        assert_eq!(rejected.status.code(), Some(2), "{args:?}");
    }
}

#[cfg(unix)]
#[test]
fn release_profile_requires_one_clean_immutable_source() {
    let fixture = release_fixture();
    let clean = release(fixture.path());
    assert!(clean.status.success(), "{}", stderr(&clean));
    assert!(stdout(&clean).contains("profile=release passed 11 selected"));

    std::fs::write(fixture.path().join("untracked-source"), "dirty\n")
        .expect("write untracked release input");
    let dirty = release(fixture.path());
    assert_eq!(dirty.status.code(), Some(2));
    assert!(stderr(&dirty).contains("requires a clean checkout"));
    std::fs::remove_file(fixture.path().join("untracked-source"))
        .expect("remove untracked release input");

    write_executable(
        &fixture.path().join("ci/scripts/fmt.sh"),
        "#!/usr/bin/env bash\nset -euo pipefail\ntouch changed-during-validation\n",
    );
    git(fixture.path(), &["add", "ci/scripts/fmt.sh"]);
    git(fixture.path(), &["commit", "-q", "-m", "mutating gate"]);
    let changed = release(fixture.path());
    assert_eq!(changed.status.code(), Some(1));
    assert!(stderr(&changed).contains("source changed during full validation"));
}

#[cfg(unix)]
#[test]
fn release_profile_binds_receipt_to_source_tag_toolchain_and_inventory() {
    let fixture = release_fixture();
    let source = git_stdout(fixture.path(), &["rev-parse", "HEAD"]);
    let inventory = sha256(&fixture.path().join("ci/validation.yml"));
    let summary = tempfile::NamedTempFile::new_in(aplan_core::fixture::fixture_root())
        .expect("create Actions summary fixture");
    let summary_path = summary.path().to_string_lossy().to_string();
    let output = release_with(
        fixture.path(),
        &[("GITHUB_STEP_SUMMARY", summary_path.as_str())],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let output = stdout(&output);
    assert!(output.contains(&format!(
        "release receipt outcome=passed source={source} version=0.5.0-dev tag=none inventory_sha256={inventory}"
    )));
    assert!(output.contains("toolchain: channel="));
    for gate in [
        "fmt",
        "clippy",
        "doc",
        "test",
        "spec",
        "msrv-build",
        "style",
        "supply-chain",
        "coverage",
        "structural",
        "evidence",
    ] {
        assert!(
            output.contains(&format!(
                "validation: gate={gate} outcome=passed duration_seconds="
            )),
            "missing result for {gate}:\n{output}"
        );
    }

    let summary = std::fs::read_to_string(summary.path()).expect("read Actions summary fixture");
    for expected in [
        "### aplan release validation",
        "- Outcome: passed",
        &format!("- Source: `{source}`"),
        "- Version: `0.5.0-dev`",
        "- Tag: `none`",
        &format!("- Gate inventory: `sha256:{inventory}`"),
        "- Toolchain: `toolchain: channel=",
        "| `structural` | passed |",
        "| `evidence` | passed |",
    ] {
        assert!(
            summary.contains(expected),
            "missing {expected:?}:\n{summary}"
        );
    }

    std::fs::write(
        fixture.path().join("Cargo.toml"),
        "[workspace.package]\nversion = \"0.5.0\"\n",
    )
    .expect("write tagged version fixture");
    git(fixture.path(), &["add", "Cargo.toml"]);
    git(fixture.path(), &["commit", "-q", "-m", "tagged version"]);
    let tagged = release_with(fixture.path(), &[("TAG", "v0.5.0")]);
    assert!(tagged.status.success(), "{}", stderr(&tagged));
    assert!(stdout(&tagged).contains("version=0.5.0 tag=v0.5.0"));
}

#[cfg(unix)]
#[test]
fn release_profile_refuses_foreign_source_bad_tag_and_failed_gate() {
    let fixture = release_fixture();
    let gate_log = tempfile::NamedTempFile::new_in(aplan_core::fixture::fixture_root())
        .expect("create gate log fixture");
    let gate_log_path = gate_log.path().to_string_lossy().to_string();
    let wrong_source = "0000000000000000000000000000000000000000";
    let foreign = release_with(
        fixture.path(),
        &[
            ("APLAN_RELEASE_SHA", wrong_source),
            ("APLAN_TEST_GATE_LOG", gate_log_path.as_str()),
        ],
    );
    assert_eq!(foreign.status.code(), Some(2));
    assert!(stderr(&foreign).contains("does not match expected release source"));
    assert_eq!(
        std::fs::read_to_string(gate_log.path()).expect("read gate log"),
        "",
        "a foreign source must fail before a gate runs"
    );

    let abbreviated = release_with(
        fixture.path(),
        &[
            ("APLAN_RELEASE_SHA", "HEAD"),
            ("APLAN_TEST_GATE_LOG", gate_log_path.as_str()),
        ],
    );
    assert_eq!(abbreviated.status.code(), Some(2));
    assert!(stderr(&abbreviated).contains("full commit ID"));

    std::fs::write(
        fixture.path().join("Cargo.toml"),
        "[workspace.package]\nversion = \"0.5.0\"\n",
    )
    .expect("write release version fixture");
    git(fixture.path(), &["add", "Cargo.toml"]);
    git(fixture.path(), &["commit", "-q", "-m", "release version"]);
    let bad_tag = release_with(
        fixture.path(),
        &[
            ("TAG", "v0.5.1"),
            ("APLAN_TEST_GATE_LOG", gate_log_path.as_str()),
        ],
    );
    assert_eq!(bad_tag.status.code(), Some(2));
    assert!(stderr(&bad_tag).contains("does not match workspace version"));
    assert_eq!(
        std::fs::read_to_string(gate_log.path()).expect("read gate log"),
        "",
        "a bad tag must fail before a gate runs"
    );

    write_executable(
        &fixture.path().join("ci/scripts/fmt.sh"),
        "#!/usr/bin/env bash\nset -euo pipefail\nexit 7\n",
    );
    git(fixture.path(), &["add", "ci/scripts/fmt.sh"]);
    git(fixture.path(), &["commit", "-q", "-m", "failing gate"]);
    let failed = release(fixture.path());
    assert_eq!(failed.status.code(), Some(7));
    let failed_output = stdout(&failed);
    assert!(failed_output.contains("gate=fmt outcome=failed duration_seconds="));
    assert!(failed_output.contains("release receipt outcome=failed"));
    assert!(!failed_output.contains("gate=clippy"));
}
