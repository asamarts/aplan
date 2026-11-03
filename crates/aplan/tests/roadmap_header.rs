//! A cut's heading and its rollup are two lines, and **no header line is
//! indented at all** (`A1-341`).
//!
//! That is the invariant rather than a chosen depth, and it is what the grouped
//! layout forced: a plan setting `roadmap.group_by` puts its group labels at two
//! and its member rows at *four*, so a rollup indented four landed back on the
//! member column for exactly the readers who had asked for more structure. A row
//! of work is always indented and a header line never is.
//!
//! **It has to use a pty.** `fold::out_width()` is unbounded into a pipe, so a
//! test that redirected would measure a line that was never folded -- and the
//! defect only exists at a terminal, where the continuation of a folded header
//! landed at the member-row indent and read as a task inside the cut.

use std::path::Path;

use crate::common;

use common::on_a_terminal;

/// A cut whose header was 126 characters on one line: a long title, a full
/// rollup, and members to be confused with.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\neffort_days_per_week: 27\n",
    )
    .expect("config");
    std::fs::write(
        dir.join("aplan/tasks/M-v0.9.md"),
        "---\nid: M-v0.9\ntitle: The plan decides the version it ships and says why\n\
         phase: 1\nstatus: todo\ntags:\n  - milestone\n  - v0.9\n\
         blocked_by:\n  - W-1\n  - W-2\n---\n\nbody\n",
    )
    .expect("cut");
    for (id, status) in [("W-1", "todo"), ("W-2", "in_progress")] {
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: a member of the cut\nphase: 1\nstatus: {status}\n\
                 effort_days: 8.5\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
}

/// **The heading and the rollup are two lines, and the rollup is not at the
/// member indent.**
///
/// Two was the member-row indent and the whole finding: the tail of a cut's
/// rollup read as a task inside it, and the break point was arbitrary -- it
/// separated a number from the word that labels it.
#[test]
fn a_cuts_rollup_is_its_own_line_and_clears_the_member_column() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let Some(out) = on_a_terminal(dir.path(), 80, &["roadmap"]) else {
        eprintln!("no pty available; skipping");
        return;
    };
    let lines: Vec<&str> = out.lines().collect();
    let heading = lines
        .iter()
        .position(|l| l.starts_with("v0.9 "))
        .unwrap_or_else(|| panic!("no cut heading in:\n{out}"));

    // The heading carries the version, the title and the status, and stops.
    let head = lines[heading];
    assert!(
        head.contains("The plan decides the version") && head.ends_with("[todo]"),
        "the heading is version, title and status: {head:?}"
    );
    assert!(
        !head.contains("done,") && !head.contains("effort"),
        "the rollup must not be on it: {head:?}"
    );

    // The rollup is the next line, and it is not indented.
    let rollup = lines[heading + 1];
    assert!(
        rollup.starts_with("0/2 done,") && rollup.contains("effort"),
        "the rollup is the next line, flush left: {rollup:?}"
    );

    // **Nothing in the header block reaches the member column.** A member row is
    // indented two, so a header line at two is indistinguishable from one.
    let members: Vec<&str> = lines[heading + 2..]
        .iter()
        .take_while(|l| !l.trim().is_empty())
        .copied()
        .collect();
    assert!(
        members.iter().any(|l| l.starts_with("  W-1")),
        "the members follow the rollup: {members:?}"
    );
    for l in [head, rollup] {
        assert!(
            !l.starts_with(' '),
            "a header line is never indented, and a row of work always is: {l:?}"
        );
    }

    // And at eighty columns neither header line folds at all, which is what
    // splitting them bought.
    assert!(
        head.len() <= 80,
        "the heading wrapped: {} {head:?}",
        head.len()
    );
    assert!(
        rollup.len() <= 80,
        "the rollup wrapped: {} {rollup:?}",
        rollup.len()
    );
}

/// **A rollup that does fold still clears the member column**, because its
/// continuation hangs at six.
#[test]
fn a_folded_rollup_never_reaches_the_member_indent() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let Some(out) = on_a_terminal(dir.path(), 40, &["roadmap"]) else {
        eprintln!("no pty available; skipping");
        return;
    };
    let lines: Vec<&str> = out.lines().collect();
    let heading = lines
        .iter()
        .position(|l| l.starts_with("v0.9 "))
        .unwrap_or_else(|| panic!("no cut heading in:\n{out}"));
    // Every line between the heading and the first member row belongs to the
    // header, and none of them may look like a member.
    let after: Vec<&str> = lines[heading + 1..]
        .iter()
        .take_while(|l| !l.starts_with("  W-"))
        .copied()
        .collect();
    assert!(!after.is_empty(), "nothing folded at forty columns:\n{out}");
    for l in &after {
        assert!(
            !l.starts_with(' '),
            "a folded header line landed at a member indent: {l:?}\n{out}"
        );
    }
}

/// **The grouped layout is the one that forced the invariant** (`A1-341`,
/// audit).
///
/// `roadmap.group_by` puts group labels at two and member rows at *four*, so a
/// rollup indented four -- which is what this shipped as first -- landed back on
/// the member column. This repository declares no family, so nothing here would
/// have caught it.
#[test]
fn a_grouped_cut_keeps_its_header_clear_of_both_member_columns() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        concat!(
            "root: aplan\n",
            "tag_families:\n  epic: [epic-alpha, epic-beta]\n",
            "roadmap:\n  group_by: epic\n",
        ),
    )
    .expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/M-v0.1.md"),
        "---\nid: M-v0.1\ntitle: The first cut\nphase: 1\nstatus: todo\n\
         tags:\n  - milestone\n  - v0.1\nblocked_by:\n  - W-1\n  - W-2\n---\n\nbody\n",
    )
    .expect("cut");
    for (id, epic) in [("W-1", "epic-alpha"), ("W-2", "epic-beta")] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: a task\nphase: 1\nstatus: todo\neffort_days: 3\n\
                 tags:\n  - {epic}\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
    let Some(out) = on_a_terminal(dir.path(), 80, &["roadmap"]) else {
        eprintln!("no pty available; skipping");
        return;
    };
    let lines: Vec<&str> = out.lines().collect();
    let heading = lines
        .iter()
        .position(|l| l.starts_with("v0.1 "))
        .unwrap_or_else(|| panic!("no cut heading in:\n{out}"));

    // The layout that forced this: labels at two, members at four.
    assert!(
        lines.iter().any(|l| l == &"  epic-alpha"),
        "the group label is at two: {out}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("    W-1")),
        "a grouped member row is at four: {out}"
    );
    // And the header block is clear of both.
    assert!(
        !lines[heading].starts_with(' ') && !lines[heading + 1].starts_with(' '),
        "a header line is indented: {:?} {:?}",
        lines[heading],
        lines[heading + 1]
    );
    assert!(
        lines[heading + 1].starts_with("0/2 done,"),
        "the rollup follows the heading: {:?}",
        lines[heading + 1]
    );
}
