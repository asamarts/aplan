//! **A plan with a vocabulary can import into itself** (`A1-357`).
//!
//! `import` named two of the three dials that decide whether what it writes is
//! acceptable -- cut membership and `effort_days` -- and was silent about
//! `tag_families`, the one that makes `check` *fail* rather than warn. It also
//! took `--cut`, so the membership it warned about could be fixed in the same
//! invocation, and took no `--tag`, so the family it did not warn about could
//! not be satisfied at all.
//!
//! That is `A1-251` one command over, and this is the command an adopter with an
//! existing roadmap runs first.

use std::path::Path;

mod common;

/// A plan declaring a mandatory family and requiring cut membership: the shape
/// an adopter with a real vocabulary has.
fn strict_plan(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\nrequire_cut_membership: true\ntag_families:\n  kind:\n\
         \x20   members: [kind-fix, kind-feature]\n    require: exactly-one\n\
         \x20   milestones: exclude\n",
    )
    .expect("config");
    std::fs::write(
        dir.join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 1\nstatus: todo\ntags:\n  - milestone\n\
         blocked_by:\n  - W-1\n---\n\n## Why\n\nA cut.\n",
    )
    .expect("cut");
    std::fs::write(
        dir.join("aplan/tasks/W-1.md"),
        "---\nid: W-1\ntitle: existing work\nphase: 1\nstatus: todo\neffort_days: 1\n\
         tags:\n  - kind-fix\n---\n\n## Why\n\nBecause.\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("PLAN.md"),
        "# Plan\n\n- [ ] first imported item\n- [ ] second imported item\n",
    )
    .expect("doc");
}

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("import-vocab-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

/// The import says what the plan will make of it, and takes the answer.
///
/// Proves the named behaviour `import.carries_the_plans_vocabulary` (`A1-357`).
#[test]
fn an_import_names_the_family_it_cannot_satisfy_and_takes_the_answer() {
    let dir = tmp();
    strict_plan(dir.path());
    let before = common::run(dir.path(), &["check"]);
    assert_eq!(
        before.code,
        0,
        "the fixture must start clean: {}",
        before.text()
    );

    // Said, before anything is written.
    let dry = common::run(
        dir.path(),
        &["import", "PLAN.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(dry.code, 0, "{}", dry.text());
    assert!(
        dry.text().contains("carry no member of `kind`") && dry.text().contains("APLAN030"),
        "the dial that makes `check` fail was not named: {}",
        dry.text()
    );

    // And answerable in the same invocation, which is the half `--cut` already
    // had and this did not.
    let run = common::run(
        dir.path(),
        &[
            "import",
            "PLAN.md",
            "--format",
            "checklist",
            "--cut",
            "M-a",
            "--tag",
            "kind-fix",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    assert!(
        !run.text().contains("carry no member of"),
        "the notice should go quiet once the invocation answers it: {}",
        run.text()
    );

    // The plan it left behind holds no error. `APLAN009` remains, and is the one
    // thing `import` has always said it cannot derive.
    let after = common::run(dir.path(), &["check"]);
    assert_eq!(
        after.code,
        0,
        "importing with both answers still left a plan that fails: {}",
        after.text()
    );
    assert!(
        after.text().contains("APLAN009") || after.text().contains("warning"),
        "expected only the effort warning: {}",
        after.text()
    );
}

/// A plan declaring no family reads exactly as it did.
///
/// Proves the named behaviour `import.carries_the_plans_vocabulary` (`A1-357`).
#[test]
fn a_plan_with_no_family_is_told_nothing_new() {
    let dir = tmp();
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.path().join("PLAN.md"), "# Plan\n\n- [ ] one item\n").expect("doc");

    let out = common::run(
        dir.path(),
        &["import", "PLAN.md", "--format", "checklist", "--dry-run"],
    );
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        !out.text().contains("carry no member of"),
        "a plan with no vocabulary was told about one: {}",
        out.text()
    );
}

/// Every imported task carries what was asked for, once.
///
/// Proves the named behaviour `import.carries_the_plans_vocabulary` (`A1-357`).
#[test]
fn the_tag_reaches_every_imported_task_and_is_written_once() {
    let dir = tmp();
    strict_plan(dir.path());
    let out = common::run(
        dir.path(),
        &[
            "import",
            "PLAN.md",
            "--format",
            "checklist",
            "--tag",
            "kind-fix",
            "--tag",
            "kind-fix",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.text());

    let mut seen = 0;
    for entry in std::fs::read_dir(dir.path().join("aplan/tasks")).expect("tasks") {
        let body = std::fs::read_to_string(entry.expect("entry").path()).unwrap_or_default();
        if !body.contains("imported item") {
            continue;
        }
        seen += 1;
        assert_eq!(
            body.matches("- kind-fix").count(),
            1,
            "the tag was written twice:\n{body}"
        );
    }
    assert_eq!(seen, 2, "both imported tasks should carry it");
}
