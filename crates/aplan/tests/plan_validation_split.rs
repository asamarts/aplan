//! The repository's structural and evidence gates are separate assurance
//! operations (`A1-615`).
//!
//! A table saying one is always-on and the other deferred is not enough: the
//! commands themselves could still call each other, or a profile could acquire
//! an accidental label. This fixture drives the real dispatcher and real aplan
//! binary over one plan whose structure is valid but whose file evidence has
//! rotted, then over a structural defect.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::common;

const GATE_NAMES: &[&str] = &[
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
];

struct Fixture {
    root: tempfile::TempDir,
    bin: PathBuf,
    log: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("plan-validation-split-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("create validation fixture");
        let scripts = root.path().join("ci/scripts");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&scripts).expect("create scripts directory");
        std::fs::create_dir_all(&bin).expect("create bin directory");

        executable(
            &scripts.join("validate.sh"),
            include_str!("../../../ci/scripts/validate.sh"),
        );
        executable(
            &scripts.join("validation-receipt.py"),
            include_str!("../../../ci/scripts/validation-receipt.py"),
        );
        std::fs::write(
            scripts.join("validation-generated.sh"),
            include_str!("../../../ci/scripts/validation-generated.sh"),
        )
        .expect("write generated validation inventory");
        std::fs::write(
            root.path().join("ci/validation.yml"),
            include_str!("../../../ci/validation.yml"),
        )
        .expect("write validation inventory");
        executable(
            &scripts.join("structural.sh"),
            include_str!("../../../ci/scripts/structural.sh"),
        );
        executable(
            &scripts.join("evidence.sh"),
            include_str!("../../../ci/scripts/evidence.sh"),
        );
        executable(
            &scripts.join("toolchain.sh"),
            "#!/usr/bin/env bash\nset -euo pipefail\necho 'toolchain: channel=1.99.1 host=x86_64-unknown-linux-gnu commit=1111111111111111111111111111111111111111 components=rustfmt,clippy cache-key=rust-1.99.1-rustfmt-clippy'\n",
        );
        for gate in GATE_NAMES {
            if matches!(*gate, "structural" | "evidence") {
                continue;
            }
            executable(
                &scripts.join(format!("{gate}.sh")),
                &format!(
                    "#!/usr/bin/env bash\nset -euo pipefail\necho '{gate}' >> \"$APLAN_GATE_LOG\"\n"
                ),
            );
        }

        let log = root.path().join("gate.log");
        std::fs::write(&log, "").expect("create gate log");
        executable(
            &bin.join("cargo"),
            r#"#!/usr/bin/env bash
set -euo pipefail
[[ "$#" -ge 6 && "$1" == run && "$2" == -q && "$3" == -p && "$4" == aplan && "$5" == -- ]] || {
    echo "unexpected cargo invocation: $*" >&2
    exit 97
}
shift 5
printf 'aplan:%s\n' "$1" >> "$APLAN_GATE_LOG"
exec "$APLAN_TEST_BINARY" "$@"
"#,
        );

        std::fs::create_dir_all(root.path().join("aplan/tasks")).expect("create plan directory");
        std::fs::write(
            root.path().join(".aplan.yml"),
            "root: aplan\ndeny_warnings: true\n",
        )
        .expect("write plan configuration");
        write_task(root.path(), "");
        git(root.path(), &["init", "-q"]);
        git(
            root.path(),
            &["config", "user.email", "validation@example.invalid"],
        );
        git(root.path(), &["config", "user.name", "Validation Fixture"]);
        git(root.path(), &["add", "."]);
        git(root.path(), &["commit", "-q", "-m", "fixture"]);

        Self { root, bin, log }
    }

    fn run(&self, args: &[&str]) -> Output {
        let inherited_path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.bin.clone()];
        paths.extend(std::env::split_paths(&inherited_path));
        let path = std::env::join_paths(paths).expect("construct fixture PATH");
        Command::new(self.root.path().join("ci/scripts/validate.sh"))
            .args(args)
            .current_dir(self.root.path())
            .env("PATH", path)
            .env("APLAN_TEST_BINARY", common::binary())
            .env("APLAN_GATE_LOG", &self.log)
            .env("APLAN_CACHE_DIR", self.root.path().join("cache"))
            .env_remove("PREFLIGHT_SKIP")
            .output()
            .expect("run validation fixture")
    }

    fn reset_log(&self) {
        std::fs::write(&self.log, "").expect("reset gate log");
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).expect("read gate log")
    }
}

fn executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write executable fixture");
    let mut permissions = std::fs::metadata(path)
        .expect("read executable fixture metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("make fixture executable");
}

fn write_task(root: &Path, extra: &str) {
    std::fs::write(
        root.join("aplan/tasks/W-1.md"),
        format!(
            "---\nid: W-1\ntitle: stale evidence\nphase: 1\nstatus: done\neffort_days: 1\n{extra}paths: [missing.txt]\nevidence:\n  - kind: file\n    ref: missing.txt\n---\n"
        ),
    )
    .expect("write task");
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run fixture git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Development proves only the structural result; full additionally catches
/// evidence rot, and the two inventory labels drive disjoint commands.
#[test]
fn development_structural_and_full_evidence_gates_are_semantically_separate() {
    let fixture = Fixture::new();

    let development = fixture.run(&["--profile", "development"]);
    let development_text = text(&development);
    assert!(development.status.success(), "{development_text}");
    assert!(
        development_text.contains(
            "deferred gates: clippy doc test spec msrv-build supply-chain coverage evidence"
        ),
        "{development_text}"
    );
    assert!(!development_text.contains("verified"), "{development_text}");
    let development_log = fixture.log();
    assert_eq!(development_log.matches("aplan:check").count(), 1);
    assert!(
        !development_log.contains("aplan:verify"),
        "{development_log}"
    );

    fixture.reset_log();
    let full = fixture.run(&["--profile", "full"]);
    let full_text = text(&full);
    assert!(!full.status.success(), "{full_text}");
    assert!(full_text.contains("APLAN015"), "{full_text}");
    let full_log = fixture.log();
    assert_eq!(full_log.matches("aplan:check").count(), 1);
    assert_eq!(full_log.matches("aplan:verify").count(), 1);

    write_task(fixture.root.path(), "blocked_by: [GHOST]\n");
    fixture.reset_log();
    let broken = fixture.run(&["--profile", "development"]);
    let broken_text = text(&broken);
    assert!(!broken.status.success(), "{broken_text}");
    assert!(broken_text.contains("APLAN003"), "{broken_text}");
    assert!(
        broken_text.contains("structural check failed; the complete findings are listed above"),
        "{broken_text}"
    );
    assert_eq!(fixture.log().matches("aplan:check").count(), 1);
    assert!(!fixture.log().contains("aplan:verify"));

    write_task(fixture.root.path(), "");
    fixture.reset_log();
    let evidence = fixture.run(&["--gate", "evidence"]);
    assert!(!evidence.status.success(), "{}", text(&evidence));
    assert_eq!(fixture.log(), "aplan:verify\n");
}
