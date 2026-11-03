//! The pre-push gate sheds Git's hook-local repository identity (`A1-279`).
//!
//! Git exports variables such as `GIT_DIR` to hooks. That is correct for a hook
//! locating its own checkout and wrong for a test suite which creates nested
//! repositories: every Git CLI child otherwise keeps addressing the checkout
//! while libgit2 opens the nested path. The real hook is copied into a tiny
//! repository so this test exercises its ordering rather than a Rust duplicate.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::{ffi::OsString, fs};

use crate::fixture_executable::{install_executable, install_executable_copy};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
fn local_git_env_names() -> Vec<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--local-env-vars"])
        .output()
        .expect("ask Git for its local environment");
    assert!(out.status.success(), "git rev-parse --local-env-vars");
    String::from_utf8(out.stdout)
        .expect("Git's environment names are UTF-8")
        .lines()
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn clear_local_git_env(command: &mut Command) {
    for name in local_git_env_names() {
        command.env_remove(name);
    }
}
fn git(root: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command.args(args).current_dir(root);
    clear_local_git_env(&mut command);
    let out = command.output().expect("run Git");
    assert!(
        out.status.success(),
        "git {}: {}{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn run_hook(root: &Path, input: &str) -> Output {
    let hook = root.join("ci/githooks/pre-push");
    let git_dir = root.join(".git");
    let mut command = Command::new(hook);
    command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    clear_local_git_env(&mut command);
    command
        .env("GIT_DIR", &git_dir)
        .env("GIT_WORK_TREE", root)
        .env("GIT_INDEX_FILE", git_dir.join("index"));

    let mut child = command.spawn().expect("run pre-push hook");
    child
        .stdin
        .take()
        .expect("hook stdin")
        .write_all(input.as_bytes())
        .expect("write ref update");
    child.wait_with_output().expect("wait for hook")
}

#[test]
fn pre_push_clears_git_local_environment_before_the_gate() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("temporary repository");
    let root = dir.path();
    std::fs::create_dir_all(root.join("ci/githooks")).expect("hook directory");
    std::fs::create_dir_all(root.join("ci/scripts")).expect("script directory");
    std::fs::create_dir_all(root.join("fixture")).expect("fixture directory");
    git(root, &["init", "-q"]);
    git(&root.join("fixture"), &["init", "-q"]);

    let hook = root.join("ci/githooks/pre-push");
    install_executable_copy(&workspace_root().join("ci/githooks/pre-push"), &hook);

    let gate = root.join("ci/scripts/preflight.sh");
    install_executable(
        &gate,
        r#"#!/usr/bin/env bash
set -euo pipefail
while IFS= read -r name; do
    if [[ -n "${!name-}" ]]; then
        echo "leaked Git-local variable: $name" >&2
        exit 41
    fi
done < <(git rev-parse --local-env-vars)
test "$(git -C fixture rev-parse --show-toplevel)" = "$PWD/fixture"
printf 'gate ran\n' > gate-ran
: > preflight-args
for argument in "$@"; do
    printf '%s\0' "$argument" >> preflight-args
done
"#,
    );

    let update = format!(
        "refs/heads/main {} refs/heads/main {}\n",
        "1".repeat(40),
        "2".repeat(40)
    );
    let out = run_hook(root, &update);
    assert!(
        out.status.success(),
        "hook failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.join("gate-ran")).expect("gate marker"),
        "gate ran\n",
        "an ordinary update delegates to preflight"
    );
    assert_eq!(
        nul_arguments(&root.join("preflight-args")),
        vec!["--pre-push".to_owned(), "2".repeat(40), "1".repeat(40)],
        "the exact remote and local object ids reach preflight"
    );

    assert_hook_input_routing(root);
}

fn assert_hook_input_routing(root: &Path) {
    std::fs::remove_file(root.join("gate-ran")).expect("remove marker");
    std::fs::remove_file(root.join("preflight-args")).expect("remove argument record");
    let deletion = format!(
        "refs/heads/old {} refs/heads/old {}\n",
        "0".repeat(40),
        "2".repeat(40)
    );
    let out = run_hook(root, &deletion);
    assert!(out.status.success(), "deletion-only hook failed");
    assert!(
        !root.join("gate-ran").exists(),
        "a deletion-only push must remain a no-op"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("deletion-only update"));

    let out = run_hook(root, "");
    assert!(out.status.success(), "empty-input hook failed");
    assert!(String::from_utf8_lossy(&out.stdout).contains("no ref updates supplied"));
    assert!(nul_arguments(&root.join("preflight-args")).is_empty());

    let multiple = format!(
        "refs/heads/one {} refs/heads/one {}\nrefs/heads/gone {} refs/heads/gone {}\nrefs/heads/two {} refs/heads/two {}\n",
        "3".repeat(40),
        "4".repeat(40),
        "0".repeat(40),
        "7".repeat(40),
        "5".repeat(40),
        "6".repeat(40),
    );
    let out = run_hook(root, &multiple);
    assert!(out.status.success(), "multi-ref hook failed");
    assert_eq!(
        nul_arguments(&root.join("preflight-args")),
        vec![
            "--pre-push".to_owned(),
            "4".repeat(40),
            "3".repeat(40),
            "--pre-push".to_owned(),
            "6".repeat(40),
            "5".repeat(40),
        ],
        "each non-deletion update reaches one union selection in input order"
    );

    std::fs::remove_file(root.join("gate-ran")).expect("remove marker before malformed input");
    let malformed = run_hook(root, "not enough fields\n");
    assert_eq!(malformed.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("malformed ref update"));
    assert!(
        !root.join("gate-ran").exists(),
        "malformed Git input must not produce a profile verdict"
    );
}

fn nul_arguments(path: &Path) -> Vec<String> {
    std::fs::read(path)
        .expect("read preflight arguments")
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| String::from_utf8(record.to_vec()).expect("argument is UTF-8"))
        .collect()
}

struct PreflightFixture {
    directory: tempfile::TempDir,
    preflight: PathBuf,
    path: OsString,
    gate_log: PathBuf,
    source: String,
}

impl PreflightFixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary preflight fixture");
        let root = directory.path();
        let scripts = root.join("ci/scripts");
        let bin = root.join("bin");
        fs::create_dir_all(&scripts).expect("script directory");
        fs::create_dir_all(&bin).expect("binary directory");
        let preflight = scripts.join("preflight.sh");
        let validate = scripts.join("validate.sh");
        install_executable(&preflight, include_str!("../../ci/scripts/preflight.sh"));
        install_executable(&validate, include_str!("../../ci/scripts/validate.sh"));
        install_executable(
            &scripts.join("validation-receipt.py"),
            include_str!("../../ci/scripts/validation-receipt.py"),
        );
        install_executable(&scripts.join("validation-select.py"), FAKE_SELECTOR);
        fs::write(
            scripts.join("validation-generated.sh"),
            include_str!("../../ci/scripts/validation-generated.sh"),
        )
        .expect("copy generated inventory");
        fs::write(
            root.join("ci/validation.yml"),
            include_str!("../../ci/validation.yml"),
        )
        .expect("copy validation inventory");
        install_executable(
            &scripts.join("toolchain.sh"),
            "#!/usr/bin/env bash\nset -euo pipefail\necho 'toolchain: channel=1.99.1 host=x86_64-unknown-linux-gnu commit=1111111111111111111111111111111111111111 components=rustfmt,clippy cache-key=rust-1.99.1-rustfmt-clippy'\n",
        );
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
            install_executable(
                &scripts.join(format!("{gate}.sh")),
                &format!(
                    "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s' '{gate}' >> \"$APLAN_TEST_GATE_LOG\"\nfor argument in \"$@\"; do printf ' <%s>' \"$argument\" >> \"$APLAN_TEST_GATE_LOG\"; done\nprintf '\\n' >> \"$APLAN_TEST_GATE_LOG\"\n"
                ),
            );
        }
        let gate_log = root.join("gates.log");
        fs::write(&gate_log, "").expect("create gate log");
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "preflight@example.invalid"]);
        git(root, &["config", "user.name", "Preflight Fixture"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-q", "-m", "fixture"]);
        let source = {
            let mut command = Command::new("git");
            command.args(["rev-parse", "HEAD"]).current_dir(root);
            clear_local_git_env(&mut command);
            let output = command.output().expect("read preflight fixture source");
            assert!(output.status.success());
            String::from_utf8(output.stdout)
                .expect("fixture source is UTF-8")
                .trim()
                .to_owned()
        };
        let inherited_path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(bin).chain(std::env::split_paths(&inherited_path)),
        )
        .expect("construct fixture PATH");
        Self {
            directory,
            preflight,
            path,
            gate_log,
            source,
        }
    }

    fn run(&self, environment: &[(&str, &str)]) -> Output {
        self.run_args(&[], environment)
    }

    fn run_args(&self, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
        let mut command = Command::new(&self.preflight);
        command
            .args(arguments)
            .current_dir(self.directory.path())
            .env("PATH", &self.path)
            .env("APLAN_TEST_GATE_LOG", &self.gate_log)
            .env_remove("APLAN_TEST_SELECTOR_FAIL")
            .env_remove("APLAN_TEST_SELECTOR_BAD_RECORD")
            .env_remove("APLAN_TEST_SELECTOR_NO_CHANGES")
            .env_remove("APLAN_TEST_SELECTOR_WIDEN");
        for (name, value) in environment {
            command.env(name, value);
        }
        command.output().expect("run development preflight")
    }

    fn reset(&self) {
        fs::write(&self.gate_log, "").expect("reset gate log");
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.gate_log).expect("read gate log")
    }
}

const FAKE_SELECTOR: &str = r#"#!/usr/bin/env python3
import os
import sys

arguments = sys.argv[1:]
working = ["--working-tree", "--format", "arguments", "--timings"]
base_head = (
    len(arguments) == 7
    and arguments[0] == "--base"
    and arguments[2] == "--head"
    and arguments[4:] == ["--format", "arguments", "--timings"]
)
if arguments != working and not base_head:
    print(f"unexpected selector invocation: {arguments!r}", file=sys.stderr)
    raise SystemExit(97)
if os.environ.get("APLAN_TEST_SELECTOR_FAIL"):
    print("fixture selector failed", file=sys.stderr)
    raise SystemExit(42)
if os.environ.get("APLAN_TEST_SELECTOR_BAD_RECORD"):
    records = ["schema", "2", "gate"]
elif os.environ.get("APLAN_TEST_SELECTOR_NO_CHANGES"):
    records = ["schema", "2", "no-changes", "true"]
elif os.environ.get("APLAN_TEST_SELECTOR_WIDEN"):
    records = ["schema", "2", "package", "aplan", "widen", "unknown input", "no-changes", "false"]
else:
    records = ["schema", "2", "gate", "test", "package", "aplan", "no-changes", "false"]
sys.stdout.buffer.write(b"\0".join(value.encode() for value in records) + b"\0")
"#;

#[test]
fn development_preflight_consumes_typed_selection_and_fails_closed() {
    let fixture = PreflightFixture::new();
    let selected = fixture.run(&[]);
    assert!(
        selected.status.success(),
        "{}{}",
        String::from_utf8_lossy(&selected.stdout),
        String::from_utf8_lossy(&selected.stderr)
    );
    let output = String::from_utf8(selected.stdout).expect("preflight output is UTF-8");
    assert!(output.contains("classifier outcome=passed no_changes=false"));
    assert!(output.contains("selected gates: fmt test style structural"));
    assert!(output.contains("selected packages: aplan"));
    assert!(
        output
            .contains("deferred gates: clippy doc spec msrv-build supply-chain coverage evidence")
    );
    assert_eq!(
        fixture.log(),
        "fmt\ntest <--package> <aplan>\nstyle\nstructural\n"
    );

    fixture.reset();
    let no_changes = fixture.run(&[("APLAN_TEST_SELECTOR_NO_CHANGES", "1")]);
    assert!(no_changes.status.success());
    assert!(
        String::from_utf8_lossy(&no_changes.stdout)
            .contains("classifier outcome=passed no_changes=true")
    );
    assert_eq!(fixture.log(), "fmt\nstyle\nstructural\n");

    fixture.reset();
    let widened = fixture.run(&[("APLAN_TEST_SELECTOR_WIDEN", "1")]);
    assert!(widened.status.success());
    let widened_output = String::from_utf8_lossy(&widened.stdout);
    assert!(widened_output.contains("selected gates: fmt style structural"));
    assert!(widened_output.contains("widened gates: clippy doc test spec msrv-build supply-chain"));
    assert!(widened_output.contains("deferred gates: coverage evidence"));
    assert_eq!(
        fixture.log(),
        "fmt\nstyle\nstructural\nclippy\ndoc\ntest\nspec\nmsrv-build\nsupply-chain\n",
        "widening must discard package narrowing and run every selectable gate"
    );

    for (variable, expected_status, expected_error) in [
        (
            "APLAN_TEST_SELECTOR_FAIL",
            Some(42),
            "classifier outcome=failed",
        ),
        (
            "APLAN_TEST_SELECTOR_BAD_RECORD",
            Some(2),
            "truncated argument record",
        ),
    ] {
        fixture.reset();
        let failed = fixture.run(&[(variable, "1")]);
        assert_eq!(failed.status.code(), expected_status);
        assert!(String::from_utf8_lossy(&failed.stderr).contains(expected_error));
        assert_eq!(
            fixture.log(),
            "",
            "a classifier failure must not produce a profile verdict"
        );
    }
}

#[test]
fn development_preflight_accepts_one_exact_revision_pair() {
    let fixture = PreflightFixture::new();
    let selected = fixture.run_args(&["--base", &fixture.source, "--head", &fixture.source], &[]);
    assert!(
        selected.status.success(),
        "{}{}",
        String::from_utf8_lossy(&selected.stdout),
        String::from_utf8_lossy(&selected.stderr)
    );
    let output = String::from_utf8(selected.stdout).expect("preflight output is UTF-8");
    assert!(output.contains("classifier input=base-head update_count=0"));
    assert!(output.contains("classifier outcome=passed no_changes=false"));
    assert_eq!(
        fixture.log(),
        "fmt\ntest <--package> <aplan>\nstyle\nstructural\n"
    );
}

#[test]
fn one_profile_value_routes_the_same_source_through_every_full_gate() {
    let fixture = PreflightFixture::new();
    let full = fixture.run_args(
        &[
            "--profile",
            "full",
            "--base",
            &fixture.source,
            "--head",
            &fixture.source,
        ],
        &[],
    );
    assert!(
        full.status.success(),
        "{}{}",
        String::from_utf8_lossy(&full.stdout),
        String::from_utf8_lossy(&full.stderr)
    );
    assert_eq!(
        fixture.log(),
        "fmt\nclippy\ndoc\ntest\nspec\nmsrv-build\nstyle\nsupply-chain\ncoverage\nstructural\nevidence\n",
        "the rollback route must execute the inventory's complete full profile"
    );

    fixture.reset();
    git(
        fixture.directory.path(),
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "new checked-out source",
        ],
    );
    let foreign = fixture.run_args(
        &[
            "--profile",
            "full",
            "--base",
            &fixture.source,
            "--head",
            &fixture.source,
        ],
        &[],
    );
    assert!(!foreign.status.success());
    assert!(
        String::from_utf8_lossy(&foreign.stderr)
            .contains("full rollback head is not the checked-out source")
    );
    assert_eq!(
        fixture.log(),
        "",
        "a foreign rollback head must not validate the checked-out source"
    );
}

#[test]
fn development_preflight_rejects_ambiguous_revision_modes_before_selection() {
    let fixture = PreflightFixture::new();
    for arguments in [
        vec!["--base", "base-revision"],
        vec!["--head", "head-revision"],
        vec![
            "--base",
            "base-revision",
            "--head",
            "head-revision",
            "--pre-push",
            "remote-revision",
            "local-revision",
        ],
        vec![
            "--full",
            "--base",
            "base-revision",
            "--head",
            "head-revision",
        ],
    ] {
        fixture.reset();
        let rejected = fixture.run_args(&arguments, &[]);
        assert_eq!(rejected.status.code(), Some(2), "arguments: {arguments:?}");
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("usage:"),
            "arguments: {arguments:?}"
        );
        assert_eq!(fixture.log(), "", "arguments: {arguments:?}");
    }
}
