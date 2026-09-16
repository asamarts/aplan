//! The pre-push gate sheds Git's hook-local repository identity (`A1-279`).
//!
//! Git exports variables such as `GIT_DIR` to hooks. That is correct for a hook
//! locating its own checkout and wrong for a test suite which creates nested
//! repositories: every Git CLI child otherwise keeps addressing the checkout
//! while libgit2 opens the nested path. The real hook is copied into a tiny
//! repository so this test exercises its ordering rather than a Rust duplicate.

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

fn make_executable(path: &Path) {
    let mut permissions = std::fs::metadata(path)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("make script executable");
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
    std::fs::copy(workspace_root().join("ci/githooks/pre-push"), &hook)
        .expect("copy the real hook");
    make_executable(&hook);

    let gate = root.join("ci/scripts/preflight.sh");
    std::fs::write(
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
"#,
    )
    .expect("write fake gate");
    make_executable(&gate);

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

    std::fs::remove_file(root.join("gate-ran")).expect("remove marker");
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
}
