//! Executable contract for the shared nextest-plus-doctest correctness gate.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const OBSERVER: &str =
    "timings::evidence_gate_corpus_exercises_the_rolestat_shape_and_fault_control";
const ROLLOUT_OBSERVER: &str =
    "timings::evidence_rollout_artifact_reconstructs_trust_and_keeps_direct_rollback";
const MUTATOR: &str = "untracked::the_generators_note_an_untracked_plan_file";

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable fixture");
    let mut permissions = fs::metadata(path).expect("fixture metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fixture executable");
}

struct GateFixture {
    temp: tempfile::TempDir,
    bin: PathBuf,
    target: PathBuf,
    log: PathBuf,
    cache_record: PathBuf,
    groups: PathBuf,
    nextest_json: PathBuf,
    cargo_json: PathBuf,
}

fn write_fake_tools(bin: &Path) {
    executable(
        &bin.join("cargo-nextest"),
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${APLAN_FAKE_NEXTEST_VERSION:-correct}" == wrong ]]; then
    cat <<'EOF'
cargo-nextest 0.9.142 (wrong)
release: 0.9.142
commit-hash: 0000000000000000000000000000000000000000
commit-date: 2026-01-01
host: x86_64-unknown-linux-gnu
EOF
    exit 0
fi
cat <<'EOF'
cargo-nextest 0.9.143 (60fa45f63 2026-08-04)
release: 0.9.143
commit-hash: 60fa45f638ffc3f35e74afa65737f45fcd32db2a
commit-date: 2026-08-04
host: x86_64-unknown-linux-gnu
EOF
"#,
    );
    executable(
        &bin.join("cargo"),
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$APLAN_FAKE_CARGO_LOG"
case "$*" in
    nextest\ list*)
        [[ "${APLAN_FAKE_FAILURE_STAGE:-}" != nextest-list ]] || exit 31
        cat "$APLAN_FAKE_NEXTEST_JSON"
        ;;
    test\ --no-run\ --message-format\ json*)
        [[ "${APLAN_FAKE_FAILURE_STAGE:-}" != cargo-list ]] || exit 32
        cat "$APLAN_FAKE_CARGO_JSON"
        ;;
    nextest\ show-config\ test-groups*)
        [[ "${APLAN_FAKE_FAILURE_STAGE:-}" != group-census ]] || exit 33
        cat "$APLAN_FAKE_GROUPS"
        ;;
    nextest\ run*)
        [[ "${APLAN_FAKE_FAILURE_STAGE:-}" != nextest-run ]] || exit 34
        : "${APLAN_CACHE_DIR:?test gate did not export APLAN_CACHE_DIR}"
        : "${APLAN_TEST_INVOCATIONS:?test gate did not export APLAN_TEST_INVOCATIONS}"
        [[ ! -e "$APLAN_CACHE_DIR" ]]
        printf '%s\n' "$APLAN_CACHE_DIR" > "$APLAN_CACHE_TEST_RECORD"
        mkdir -p "$APLAN_CACHE_DIR"
        : > "$APLAN_CACHE_DIR/published-by-suite"
        ;;
    test*--doc)
        [[ "${APLAN_FAKE_FAILURE_STAGE:-}" != doctest ]] || exit 35
        [[ -f "$APLAN_CACHE_DIR/published-by-suite" ]]
        ;;
    run\ -q\ -p\ xtask\ --\ cli-surface\ --check)
        [[ "${APLAN_FAKE_FAILURE_STAGE:-}" != cli-audit ]] || exit 36
        [[ -f "$APLAN_CACHE_DIR/published-by-suite" ]]
        ;;
    clippy*|doc*)
        ;;
    *)
        echo "unexpected cargo invocation: $*" >&2
        exit 64
        ;;
esac
"#,
    );
}

fn write_test_binaries(root: &Path) -> (PathBuf, PathBuf) {
    let aplan_binary = root.join("aplan-tests");
    executable(
        &aplan_binary,
        &format!(
            "#!/usr/bin/env bash\nset -euo pipefail\ncase \"$*\" in\n  '--list --format terse') printf '%s: test\\n' '{OBSERVER}' ;;\n  '--ignored --list --format terse') printf '%s: test\\n' '{OBSERVER}' ;;\n  *) exit 64 ;;\nesac\n"
        ),
    );
    let xtask_binary = root.join("xtask-tests");
    executable(
        &xtask_binary,
        &format!(
            "#!/usr/bin/env bash\nset -euo pipefail\ncase \"$*\" in\n  '--list --format terse') printf '%s: test\\n' '{MUTATOR}' ;;\n  '--ignored --list --format terse') : ;;\n  *) exit 64 ;;\nesac\n"
        ),
    );
    (aplan_binary, xtask_binary)
}

fn write_inventories(
    root: &Path,
    nextest_json: &Path,
    cargo_json: &Path,
    aplan_binary: &Path,
    xtask_binary: &Path,
) {
    let nextest = serde_json::json!({
        "test-count": 2,
        "rust-suites": {
            "aplan::integration": {
                "package-id": "fixture-aplan",
                "package-name": "aplan",
                "binary-id": "aplan::integration",
                "binary-name": "integration",
                "kind": "test",
                "cwd": root,
                "testcases": {OBSERVER: {"kind": "test", "ignored": true}}
            },
            "xtask::integration": {
                "package-id": "fixture-xtask",
                "package-name": "xtask",
                "binary-id": "xtask::integration",
                "binary-name": "integration",
                "kind": "test",
                "cwd": root,
                "testcases": {MUTATOR: {"kind": "test", "ignored": false}}
            }
        }
    });
    fs::write(
        nextest_json,
        serde_json::to_vec(&nextest).expect("nextest JSON"),
    )
    .expect("write nextest inventory");
    let artifacts = [
        serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": "fixture-aplan",
            "target": {"name": "integration", "kind": ["test"]},
            "profile": {"test": true},
            "executable": aplan_binary,
        }),
        serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": "fixture-xtask",
            "target": {"name": "integration", "kind": ["test"]},
            "profile": {"test": true},
            "executable": xtask_binary,
        }),
    ];
    fs::write(
        cargo_json,
        artifacts
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .expect("Cargo JSON")
            .join("\n")
            + "\n",
    )
    .expect("write Cargo inventory");
}

impl GateFixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary test-gate root");
        let bin = temp.path().join("bin");
        let target = temp.path().join("target");
        let log = temp.path().join("cargo.log");
        let cache_record = temp.path().join("cache-path");
        let groups = temp.path().join("groups.txt");
        let nextest_json = temp.path().join("nextest.json");
        let cargo_json = temp.path().join("cargo.jsonl");
        fs::create_dir(&bin).expect("fake binary directory");
        fs::create_dir(&target).expect("isolated target directory");
        fs::write(&log, "").expect("create Cargo log");
        write_fake_tools(&bin);
        let (aplan_binary, xtask_binary) = write_test_binaries(temp.path());
        write_inventories(
            temp.path(),
            &nextest_json,
            &cargo_json,
            &aplan_binary,
            &xtask_binary,
        );

        let fixture = Self {
            temp,
            bin,
            target,
            log,
            cache_record,
            groups,
            nextest_json,
            cargo_json,
        };
        fixture.write_groups(&[OBSERVER, ROLLOUT_OBSERVER, MUTATOR]);
        fixture
    }

    fn write_groups(&self, members: &[&str]) {
        let mut text = String::from("group: workspace-source (max threads = 1)\n");
        for member in members {
            text.push_str("          ");
            text.push_str(member);
            text.push('\n');
        }
        fs::write(&self.groups, text).expect("write group census");
    }

    fn remove_nextest(&self) {
        fs::remove_file(self.bin.join("cargo-nextest")).expect("remove fake nextest");
    }

    fn run(&self, arguments: &[&str], failure_stage: Option<&str>) -> Output {
        let path = std::env::join_paths([
            self.bin.clone(),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
        ])
        .expect("joined PATH");
        let mut command = Command::new("bash");
        command
            .arg(workspace().join("ci/scripts/test.sh"))
            .args(arguments)
            .current_dir(workspace())
            .env("PATH", path)
            .env("CARGO_TARGET_DIR", &self.target)
            .env("APLAN_CACHE_TEST_RECORD", &self.cache_record)
            .env("APLAN_FAKE_CARGO_LOG", &self.log)
            .env("APLAN_FAKE_NEXTEST_JSON", &self.nextest_json)
            .env("APLAN_FAKE_CARGO_JSON", &self.cargo_json)
            .env("APLAN_FAKE_GROUPS", &self.groups);
        if let Some(stage) = failure_stage {
            command.env("APLAN_FAKE_FAILURE_STAGE", stage);
        }
        command.output().expect("run test gate")
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).expect("read fake Cargo invocations")
    }

    fn assert_cleaned(&self) {
        let cache = fs::read_to_string(&self.cache_record)
            .expect("fake Cargo recorded the cache path")
            .trim()
            .to_owned();
        let owner = Path::new(&cache).parent().expect("cache has an owner");
        assert!(
            !owner.exists(),
            "test gate left its cache owner behind at {}",
            owner.display()
        );
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "test gate failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn full_test_gate_starts_from_an_absent_isolated_cache_and_cleans_it() {
    let fixture = GateFixture::new();
    let output = fixture.run(&[], None);
    assert_success(&output);
    assert_eq!(
        fixture.log(),
        "nextest list --workspace --message-format json\n\
         test --no-run --message-format json --workspace\n\
         nextest show-config test-groups --groups workspace-source --no-pager --color never --workspace\n\
         nextest run --workspace --test-threads num-cpus --retries 0 --flaky-result fail\n\
         test --workspace --doc\n\
         run -q -p xtask -- cli-surface --check\n"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("nextest-inventory: sha256="), "{stdout}");
    assert!(stdout.contains("tests=2 ignored=1 suites=2"), "{stdout}");
    assert!(
        stdout.contains("nextest-contract: config_sha256="),
        "{stdout}"
    );
    assert!(
        stdout.contains("test-latency: runner_plus_doctests_seconds="),
        "{stdout}"
    );
    assert!(
        stdout.contains("test-latency: cli_surface_audit_seconds="),
        "{stdout}"
    );
    fixture.assert_cleaned();
}

#[test]
fn development_rust_gates_scope_cargo_to_the_selected_packages() {
    let fixture = GateFixture::new();
    let path = std::env::join_paths([
        fixture.bin.clone(),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ])
    .expect("joined PATH");
    for script in ["clippy.sh", "doc.sh"] {
        let output = Command::new("bash")
            .arg(workspace().join(format!("ci/scripts/{script}")))
            .args(["--package", "aplan", "--package", "xtask"])
            .current_dir(workspace())
            .env("PATH", &path)
            .env("CARGO_TARGET_DIR", &fixture.target)
            .env("APLAN_FAKE_CARGO_LOG", &fixture.log)
            .output()
            .expect("run package-scoped gate");
        assert_success(&output);
    }
    let output = fixture.run(&["--package", "aplan", "--package", "xtask"], None);
    assert_success(&output);
    assert_eq!(
        fixture.log(),
        "clippy -p aplan -p xtask --all-targets -- -D warnings\n\
         doc -p aplan -p xtask --no-deps\n\
         nextest list -p aplan -p xtask --message-format json\n\
         test --no-run --message-format json -p aplan -p xtask\n\
         nextest show-config test-groups --groups workspace-source --no-pager --color never -p aplan -p xtask\n\
         nextest run -p aplan -p xtask --test-threads num-cpus --retries 0 --flaky-result fail\n\
         test -p aplan -p xtask --doc\n\
         run -q -p xtask -- cli-surface --check\n"
    );
}

#[test]
fn test_gate_refuses_absent_and_wrong_nextest_before_cargo_runs() {
    let absent = GateFixture::new();
    absent.remove_nextest();
    let output = absent.run(&[], None);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cargo-nextest 0.9.143 is required"),
        "{stderr}"
    );
    assert!(absent.log().is_empty());

    let wrong = GateFixture::new();
    let inherited_path = std::env::var_os("PATH").expect("test process has PATH");
    let path = std::env::join_paths(
        std::iter::once(wrong.bin.clone()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("joined PATH");
    let output = Command::new("bash")
        .arg(workspace().join("ci/scripts/test.sh"))
        .current_dir(workspace())
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", &wrong.target)
        .env("APLAN_FAKE_NEXTEST_VERSION", "wrong")
        .env("APLAN_FAKE_CARGO_LOG", &wrong.log)
        .output()
        .expect("run wrong-version gate");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("is not the pinned build"), "{stderr}");
    assert!(stderr.contains("observed:"), "{stderr}");
    assert!(wrong.log().is_empty());
}

#[test]
fn nextest_installer_refuses_an_archive_before_extraction_when_checksum_differs() {
    let fixture = GateFixture::new();
    executable(
        &fixture.bin.join("curl"),
        r#"#!/usr/bin/env bash
set -euo pipefail
output=''
while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == --output ]]; then
        output="$2"
        shift 2
    else
        shift
    fi
done
: "${output:?curl fixture saw no output path}"
printf 'not the pinned archive\n' > "$output"
"#,
    );
    let install_dir = fixture.temp.path().join("install");
    let path = std::env::join_paths([
        fixture.bin.clone(),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ])
    .expect("installer PATH");
    let output = Command::new("bash")
        .arg(workspace().join("ci/scripts/install-nextest.sh"))
        .arg("install")
        .arg(&install_dir)
        .env("PATH", path)
        .output()
        .expect("run nextest installer with corrupt archive");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("computed checksum did NOT match"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!install_dir.join("cargo-nextest").exists());
}

#[test]
fn test_gate_preserves_each_stage_failure_and_stops_before_weaker_proof() {
    for (stage, reached, not_reached) in [
        ("nextest-run", "nextest run", "test --workspace --doc"),
        ("doctest", "test --workspace --doc", "cli-surface --check"),
        ("cli-audit", "cli-surface --check", "not-a-command"),
    ] {
        let fixture = GateFixture::new();
        let output = fixture.run(&[], Some(stage));
        assert!(!output.status.success(), "{stage} was swallowed");
        let log = fixture.log();
        assert!(log.contains(reached), "{stage} never ran:\n{log}");
        assert!(
            !log.contains(not_reached),
            "{stage} ran a later proof after failure:\n{log}"
        );
    }
}

#[test]
fn mutex_census_rejects_missing_stale_and_non_serial_membership() {
    for (members, expected) in [
        (vec![OBSERVER, MUTATOR], "membership does not match"),
        (
            vec![
                OBSERVER,
                ROLLOUT_OBSERVER,
                MUTATOR,
                "stale::workspace_actor",
            ],
            "membership does not match",
        ),
    ] {
        let fixture = GateFixture::new();
        fixture.write_groups(&members);
        let output = fixture.run(&[], None);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!fixture.log().contains("nextest run"));
    }

    let fixture = GateFixture::new();
    fs::write(
        &fixture.groups,
        "group: workspace-source (max threads = 2)\n",
    )
    .expect("non-serial group");
    let output = fixture.run(&[], None);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not serialize"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn inventory_parity_rejects_changed_ignored_semantics() {
    let fixture = GateFixture::new();
    executable(
        &fixture.temp.path().join("aplan-tests"),
        &format!(
            "#!/usr/bin/env bash\nset -euo pipefail\ncase \"$*\" in\n  '--list --format terse') printf '%s: test\\n' '{OBSERVER}' ;;\n  '--ignored --list --format terse') : ;;\n  *) exit 64 ;;\nesac\n"
        ),
    );
    let output = fixture.run(&[], None);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("inventories differ"), "{stderr}");
    assert!(!fixture.log().contains("nextest run"));
}

#[test]
fn inventory_parity_refuses_malformed_suite_identity_without_a_traceback() {
    let fixture = GateFixture::new();
    let malformed = serde_json::json!({
        "test-count": 0,
        "rust-suites": {
            "broken": {
                "package-id": null,
                "binary-name": "integration",
                "kind": "test",
                "cwd": fixture.temp.path(),
                "testcases": {}
            }
        }
    });
    fs::write(
        &fixture.nextest_json,
        serde_json::to_vec(&malformed).expect("malformed nextest JSON"),
    )
    .expect("replace nextest inventory");
    let output = fixture.run(&[], None);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid identity or cwd"), "{stderr}");
    assert!(!stderr.contains("Traceback"), "{stderr}");
    assert!(!fixture.log().contains("nextest run"));
}
