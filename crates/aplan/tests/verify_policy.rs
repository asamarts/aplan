//! `verify` honours the policy flags `check` does (`A1-208`).
//!
//! `--deny-warnings`, `--no-deny-warnings` and `--traces-root` are declared on
//! both commands and the corpus pins all three on `check` alone. That is the
//! shape a surface audit is for: two commands, one behaviour, and a suite that
//! exercises one of them -- so `verify` could stop honouring any of the three
//! and every test would still pass.
//!
//! `verify` is the half that opens the repository, which is why these need a
//! real one and cannot live in the `trycmd` corpus beside `check`'s cases.

use std::path::Path;

use crate::common;

/// A plan with one warning and nothing wrong: no `effort_days`, which is
/// `APLAN009`. The same shape `deny-warnings.trycmd` uses for `check`.
fn repo_with_a_warning(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.join("aplan/tasks/W-001.md"),
        "---\nid: W-001\ntitle: A task with nothing wrong but no estimate\nphase: 1\n\
         status: todo\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    let repo = git2::Repository::init(dir).expect("git init");
    let mut index = repo.index().expect("index");
    index
        .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add");
    index.write().expect("write index");
    let tree = repo
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree");
    let sig = git2::Signature::now("t", "t@e.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "in", &tree, &[])
        .expect("commit");
}

/// **The warning gate applies to `verify`, in both directions.**
///
/// Exit `1` under `--deny-warnings`, because a warning became a finding and that
/// is what `1` already means rather than a new code a consumer's CI would have
/// to learn (ADR-0007).
#[test]
fn the_warning_gate_reaches_verify_and_a_flag_beats_the_file() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    repo_with_a_warning(dir.path());

    let plain = common::run(dir.path(), &["verify"]);
    assert_eq!(plain.code, 0, "a warning alone is not a failure: {plain:?}");
    assert!(
        plain.text().contains("APLAN009"),
        "the fixture stopped producing a warning, so nothing below is being gated: {}",
        plain.text()
    );

    let denied = common::run(dir.path(), &["verify", "--deny-warnings"]);
    assert_eq!(
        denied.code,
        1,
        "`verify --deny-warnings` passed a warning: {}",
        denied.text()
    );
    assert!(
        denied.text().contains("denied:"),
        "`verify` failed without saying the warning is why: {}",
        denied.text()
    );

    // The other direction, against a file that turns it on: a flag that only
    // works when it agrees with the config is not a flag.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\n",
    )
    .expect("config");
    let by_file = common::run(dir.path(), &["verify"]);
    assert_eq!(
        by_file.code,
        1,
        "the config did not reach `verify`: {}",
        by_file.text()
    );
    let lenient = common::run(dir.path(), &["verify", "--no-deny-warnings"]);
    assert_eq!(
        lenient.code,
        0,
        "`--no-deny-warnings` did not beat the file: {}",
        lenient.text()
    );
}

/// **`--traces-root` moves where `verify` resolves an anchor.**
///
/// Pinned on `check` by `traces.rs` and on `verify` by nothing, though `verify`
/// is the command that resolves a traced section against its content. A flag
/// honoured by one of two commands that both declare it is the drift this audit
/// exists to find.
#[test]
fn the_traces_root_reaches_verify() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("docs/design.md"),
        "# Design\n\n## The rail\n\nFixed at three points.\n",
    )
    .expect("doc");
    // Written *without* the `docs/` prefix, so it resolves only when the root
    // moves. That is the whole behaviour of the flag, stated as a fixture.
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: Traces a section\nphase: 1\nstatus: todo\neffort_days: 1\n\
         traces:\n- design.md#the-rail\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    let repo = git2::Repository::init(dir.path()).expect("git init");
    let mut index = repo.index().expect("index");
    index
        .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add");
    index.write().expect("write index");
    let tree = repo
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree");
    let sig = git2::Signature::now("t", "t@e.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "in", &tree, &[])
        .expect("commit");

    let default = common::run(dir.path(), &["verify"]);
    assert!(
        default.text().contains("APLAN008"),
        "the anchor resolved from the repository root, so moving the root proves nothing: {}",
        default.text()
    );
    let moved = common::run(dir.path(), &["verify", "--traces-root", "docs"]);
    assert!(
        !moved.text().contains("APLAN008"),
        "`verify --traces-root docs` did not move where the anchor resolves: {}",
        moved.text()
    );
}
