//! `test.covers_by_test` reaches `APLAN025` from a real `.aplan.yml` (`A1-069`).
//!
//! The library tests construct a `TestRunner` in Rust, which skips the half of
//! this feature an adopter actually writes: the YAML. Nothing proved the key
//! parsed, survived config loading, and changed a verdict, and that is the whole
//! path a repository adopting it depends on.

use std::path::Path;
use std::process::Command;

use crate::common;
use common::aplan;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running git")
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

/// A repository whose only harness that can read the tree lives in `xtask`: one
/// repository-wide invariant, and one ordinary unit test, listed by one runner.
fn repo(covers_by_test: bool) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::create_dir_all(root.join("web")).expect("web dir");
    std::fs::write(root.join("web/package.json"), "{}").expect("web file");

    let subset = if covers_by_test {
        "    covers_by_test:\n      - tests: [invariants::*]\n        paths: ['**']\n"
    } else {
        ""
    };
    std::fs::write(
        root.join(".aplan.yml"),
        format!(
            "test:\n  - name: xtask\n    covers: [xtask/**]\n{subset}\
             \n    list: [printf, \"invariants::tests::the_web_skeleton_exists\\nunit::tests::a_helper\\n\"]\
             \n    run: ['true', '{{test}}']\n"
        ),
    )
    .expect("config");

    for (id, title, test) in [
        (
            "T-001",
            "proved by a repository invariant",
            "invariants::tests::the_web_skeleton_exists",
        ),
        (
            "T-002",
            "citing an ordinary xtask unit test",
            "unit::tests::a_helper",
        ),
    ] {
        std::fs::write(
            root.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: done\npaths: [web/**]\n\
                 evidence:\n  - kind: test\n    ref: {test}\n---\n"
            ),
        )
        .expect("task");
    }

    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@t"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "init"]);
    dir
}

#[test]
fn a_declared_invariant_parses_from_yaml_and_changes_the_verdict() {
    let dir = repo(true);
    let (out, code) = aplan(dir.path(), &["verify"]);

    // The invariant asserts a fact about a tree it does not live in, so it is
    // proof rather than borrowed.
    assert!(
        !out.contains("T-001"),
        "the declared invariant must not be reported: {out}"
    );
    // ... and the narrowing is not a blanket exemption: an ordinary test from the
    // same runner still falls back to `covers`.
    assert!(
        out.contains("APLAN025") && out.contains("T-002"),
        "the ordinary test must still be reported: {out}"
    );
    assert_eq!(code, 0, "advisory by default: {out}");
}

#[test]
fn without_the_declaration_the_same_plan_reports_the_invariant_too() {
    // The control. Without this, the test above would pass just as well if the
    // key were ignored entirely.
    let dir = repo(false);
    let (out, _) = aplan(dir.path(), &["verify"]);
    assert!(
        out.contains("T-001") && out.contains("T-002"),
        "both citations look borrowed without the declaration: {out}"
    );
}

#[test]
fn a_covers_by_test_the_tool_cannot_act_on_is_refused_at_load() {
    // Config the tool cannot act on is an error, never a silent no-op, and the
    // message has to name the key so the author can find it.
    let dir = repo(false);
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "test:\n  - name: xtask\n    covers_by_test:\n      - tests: []\n        paths: ['**']\n\
         \n    list: [printf, \"a\\n\"]\n    run: ['true', '{test}']\n",
    )
    .expect("config");

    let (out, code) = aplan(dir.path(), &["verify"]);
    assert!(out.contains("covers_by_test"), "{out}");
    assert_eq!(code, 2, "a config error is usage, not a finding: {out}");
}
