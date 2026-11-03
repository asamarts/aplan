//! The pushed-run monitor distinguishes an absent run from an absent observer (`A1-621`).
//!
//! GitHub can take minutes to create a run, so an empty successful query must
//! keep polling. A failed query is different: its JSON error body is not a run
//! row and must never be printed as a workflow status. Drive the real script
//! with a fake `gh` so shell error handling is what this test proves.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("temporary repository");
    let root = dir.path();
    std::fs::create_dir_all(root.join("ci/scripts")).expect("script directory");
    std::fs::create_dir_all(root.join("fake-bin")).expect("fake command directory");
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-q", "-b", "main"]);
    std::fs::write(root.join("tracked"), "fixture\n").expect("tracked file");
    git(root, &["add", "tracked"]);
    git(
        root,
        &[
            "-c",
            "user.name=aplan test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-q",
            "-m",
            "fixture",
        ],
    );
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/project.git",
        ],
    );
    git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);

    let monitor = root.join("ci/scripts/pushed.sh");
    install_executable_copy(&workspace_root().join("ci/scripts/pushed.sh"), &monitor);
    let gh = root.join("fake-bin/gh");
    install_executable(
        &gh,
        r#"#!/bin/sh
case "$FAKE_GH_RESULT" in
  api-error)
    printf '%s\n' '{"message":"Not Found"}'
    printf '%s\n' 'gh: Not Found (HTTP 404)' >&2
    exit 1
    ;;
  absent) exit 0 ;;
  queued) printf '%s\n' 'queued - https://example.invalid/run' ;;
  success) printf '%s\n' 'completed success https://example.invalid/run' ;;
  failure) printf '%s\n' 'completed failure https://example.invalid/run' ;;
  multiple)
    printf '%s\n' 'completed failure https://example.invalid/old'
    printf '%s\n' 'completed success https://example.invalid/new'
    ;;
  malformed) printf '%s\n' 'queued -' ;;
  *) exit 64 ;;
esac
"#,
    );
    dir
}

fn run(root: &Path, result: &str) -> Output {
    let current_path = std::env::var("PATH").expect("PATH");
    let mut command = Command::new(root.join("ci/scripts/pushed.sh"));
    command
        .current_dir(root)
        .env(
            "PATH",
            format!("{}:{current_path}", root.join("fake-bin").display()),
        )
        .env("FAKE_GH_RESULT", result)
        .env("PUSHED_TIMEOUT", "0")
        .env("PUSHED_INTERVAL", "1");
    clear_local_git_env(&mut command);
    command.output().expect("run pushed monitor")
}

fn run_with_polling(root: &Path, timeout: &str, interval: &str) -> Output {
    let current_path = std::env::var("PATH").expect("PATH");
    let mut command = Command::new(root.join("ci/scripts/pushed.sh"));
    command
        .current_dir(root)
        .env(
            "PATH",
            format!("{}:{current_path}", root.join("fake-bin").display()),
        )
        .env("FAKE_GH_RESULT", "success")
        .env("PUSHED_TIMEOUT", timeout)
        .env("PUSHED_INTERVAL", interval);
    clear_local_git_env(&mut command);
    command.output().expect("run pushed monitor")
}

#[test]
fn pushed_monitor_distinguishes_api_failure_absence_success_and_workflow_failure() {
    let dir = fixture();

    let api_error = run(dir.path(), "api-error");
    assert_eq!(api_error.status.code(), Some(2));
    let api_stdout = String::from_utf8_lossy(&api_error.stdout);
    let api_stderr = String::from_utf8_lossy(&api_error.stderr);
    assert!(!api_stdout.contains("is {\"message\""), "{api_stdout}");
    assert!(
        api_stderr.contains("GitHub API query failed"),
        "{api_stderr}"
    );
    assert!(api_stderr.contains("Not Found"), "{api_stderr}");

    let absent = run(dir.path(), "absent");
    assert_eq!(absent.status.code(), Some(3));
    let absent_stdout = String::from_utf8_lossy(&absent.stdout);
    let absent_stderr = String::from_utf8_lossy(&absent.stderr);
    assert!(absent_stdout.contains("no run object"), "{absent_stdout}");
    assert!(
        absent_stderr.contains("no CI run appeared"),
        "{absent_stderr}"
    );

    let success = run(dir.path(), "success");
    assert!(success.status.success());
    assert!(String::from_utf8_lossy(&success.stdout).contains("-> success"));

    let failure = run(dir.path(), "failure");
    assert_eq!(failure.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&failure.stdout).contains("-> failure"));
}

#[test]
fn pushed_monitor_distinguishes_incomplete_timeout_from_absent_run() {
    let dir = fixture();

    let incomplete = run(dir.path(), "queued");
    assert_eq!(incomplete.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&incomplete.stdout);
    let stderr = String::from_utf8_lossy(&incomplete.stderr);
    assert!(stdout.contains("is queued"), "{stdout}");
    assert!(stderr.contains("appeared but did not complete"), "{stderr}");
    assert!(stderr.contains("Last status: queued"), "{stderr}");
    assert!(stderr.contains("https://example.invalid/run"), "{stderr}");
    assert!(!stderr.contains("no CI run appeared"), "{stderr}");

    let absent = run(dir.path(), "absent");
    assert_eq!(absent.status.code(), Some(3));
    let absent_stderr = String::from_utf8_lossy(&absent.stderr);
    assert!(
        absent_stderr.contains("no CI run appeared"),
        "{absent_stderr}"
    );
    assert!(
        !absent_stderr.contains("appeared but did not complete"),
        "{absent_stderr}"
    );
}

#[test]
fn pushed_monitor_rejects_ambiguous_or_malformed_selected_runs() {
    let dir = fixture();

    for result in ["multiple", "malformed"] {
        let output = run(dir.path(), result);
        assert_eq!(output.status.code(), Some(2), "{result}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("status is unknown"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let script =
        std::fs::read_to_string(dir.path().join("ci/scripts/pushed.sh")).expect("read monitor");
    assert!(
        script.contains("sort_by(.created_at, .id) | last"),
        "{script}"
    );
    assert!(
        script.contains("select(.name == \"CI\" and .event == \"push\")"),
        "a manual full run on the same SHA must not stand in for the push verdict: {script}"
    );
    assert!(!script.contains("head -1"), "{script}");
}

#[test]
fn pushed_monitor_rejects_polling_values_that_cannot_make_progress() {
    let dir = fixture();

    for (timeout, interval, expected) in [
        ("nope", "1", "PUSHED_TIMEOUT"),
        ("-1", "1", "PUSHED_TIMEOUT"),
        ("1234567890", "1", "PUSHED_TIMEOUT"),
        ("0", "0", "PUSHED_INTERVAL"),
        ("0", "-1", "PUSHED_INTERVAL"),
        ("0", "1.5", "PUSHED_INTERVAL"),
        ("0", "1234567890", "PUSHED_INTERVAL"),
    ] {
        let output = run_with_polling(dir.path(), timeout, interval);
        assert_eq!(output.status.code(), Some(2), "{timeout}/{interval}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let leading_zeroes = run_with_polling(dir.path(), "00", "01");
    assert!(leading_zeroes.status.success());
}
