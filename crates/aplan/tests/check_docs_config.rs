//! Config-declared document gates (`A1-203`).
//!
//! A list passed by CI is another copy of which prose the repository promises
//! to keep honest. The declaration is useful only if the no-argument command
//! consumes it end to end, globs grow with the directory, and an explicit path
//! remains a narrow one-off check rather than silently adding the whole gate.

use std::path::Path;

use crate::common;

fn task(dir: &Path, id: &str, extra: &str) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("plan directory");
    let effort = if extra.contains("tags: [milestone]") {
        ""
    } else {
        "effort_days: 1\n"
    };
    std::fs::write(
        dir.join(format!("aplan/tasks/{id}.md")),
        format!("---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\n{effort}{extra}---\n"),
    )
    .expect("task");
}

#[test]
fn no_arguments_expand_declared_globs_and_see_new_documents() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    task(dir.path(), "T-001", "");
    std::fs::create_dir_all(dir.path().join("docs")).expect("docs");
    std::fs::write(dir.path().join("docs/one.md"), "Handled by T-001.\n").expect("first doc");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndocs:\n  paths: ['docs/*.md']\n",
    )
    .expect("config");

    let first = common::run(dir.path(), &["check-docs"]);
    assert!(first.ok(), "the declaration was not used: {first:?}");
    assert!(first.stdout.contains("ok: 1 document(s)"), "{first:?}");

    // The declaration is a pattern, not the expansion somebody happened to
    // observe when they wrote CI. A later document joins the gate with no
    // command-line edit.
    std::fs::write(
        dir.path().join("docs/two.md"),
        "Looks handled by T-999, but is not.\n",
    )
    .expect("second doc");
    let second = common::run(dir.path(), &["check-docs"]);
    assert_eq!(
        second.code, 1,
        "the new document stayed outside: {second:?}"
    );
    assert!(second.stdout.contains("docs/two.md"), "{second:?}");
    assert!(second.stdout.contains("`T-999`"), "{second:?}");
    assert!(
        second.stdout.contains("failed: 2 document(s)"),
        "{second:?}"
    );
}

#[test]
fn explicit_paths_override_the_declaration_and_help_states_that_rule() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    task(dir.path(), "T-001", "");
    std::fs::write(dir.path().join("ONE.md"), "Handled by T-001.\n").expect("document");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndocs:\n  paths: ['missing/*.md']\n",
    )
    .expect("config");

    let explicit = common::run(dir.path(), &["check-docs", "ONE.md"]);
    assert!(
        explicit.ok(),
        "the explicit path added the broken declaration instead of overriding it: {explicit:?}"
    );
    assert!(
        explicit.stdout.contains("ok: 1 document(s)"),
        "{explicit:?}"
    );

    let implicit = common::run(dir.path(), &["check-docs"]);
    assert_eq!(implicit.code, 2, "an unmatched glob passed: {implicit:?}");
    assert!(implicit.stderr.contains("matched no files"), "{implicit:?}");

    let help = common::run(dir.path(), &["check-docs", "--help"]);
    assert!(help.ok(), "{help:?}");
    assert!(help.stdout.contains("override `docs.paths`"), "{help:?}");
    assert!(
        help.stdout
            .contains("including every `docs.membership[].doc`"),
        "{help:?}"
    );
}

#[test]
fn membership_documents_join_the_implicit_set_without_a_second_path() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    task(dir.path(), "D1-01", "");
    task(dir.path(), "D1-02", "");
    task(
        dir.path(),
        "M-v0.1",
        "tags: [milestone]\nblocked_by: [D1-01, D1-02]\n",
    );
    std::fs::write(
        dir.path().join("PLAN.md"),
        "### v0.1\n\n**Tasks:** `D1-01`\n",
    )
    .expect("plan document");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndocs:\n  membership:\n    - doc: PLAN.md\n      declared_by: line\n      marker: \"**Tasks:**\"\n",
    )
    .expect("config");

    let run = common::run(dir.path(), &["check-docs"]);
    assert_eq!(run.code, 1, "the membership document was not read: {run:?}");
    assert!(run.stdout.contains("PLAN.md"), "{run:?}");
    assert!(run.stdout.contains("`D1-02`"), "{run:?}");
    assert!(
        run.stdout.contains("1 cut(s) whose membership disagrees"),
        "{run:?}"
    );
}

#[test]
fn no_arguments_refuse_when_configuration_declares_no_documents() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    task(dir.path(), "T-001", "");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    let run = common::run(dir.path(), &["check-docs"]);
    assert_eq!(run.code, 2, "a gate over nothing passed: {run:?}");
    assert!(
        run.stderr.contains("no documents were passed") && run.stderr.contains("`docs.paths`"),
        "{run:?}"
    );
    assert!(run.stdout.is_empty(), "{run:?}");
}
