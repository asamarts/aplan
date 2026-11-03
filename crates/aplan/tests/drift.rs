//! The drift loop, as an adopter walks it (`A1-107`).
//!
//! Four of the last six pilot findings were about trace drift, and every one was
//! proved by a unit test on `anchor_fingerprint` or by a run in a throwaway
//! directory that nobody kept. Those prove the function and the afternoon; they do
//! not prove the command. If `close` stopped stamping tomorrow, every test in
//! `anchor.rs` would still pass.
//!
//! So this drives the built binary through the sequence a repository actually
//! performs: close a task, edit something else, edit the traced thing, re-stamp,
//! retarget. Both anchor shapes, because the table row (ADR-0015) is the one with
//! no end-to-end proof at all.
//!
//! **Not a `trycmd` case**, and the reason is measured rather than assumed: the
//! corpus runs a case inside its own `.in` directory with no sandbox, so a command
//! that writes edits the fixture and the next run starts somewhere else. A
//! throwaway case that closed a task changed its fixture's hash, which is why
//! `close.trycmd` pins only the guards that refuse.

use std::path::Path;

use crate::common;

/// A repository with one document in two shapes: a prose section, and a register
/// kept as a table. Both are things real plans trace, and only the second was
/// unproved.
fn fixture(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "detect_drift: true\n").expect("config");
    std::fs::write(
        dir.join("DOC.md"),
        "# Overview\n\nthe prose a task traces\n\n\
         # Invariants\n\n| # | Invariant | Status |\n|---|---|---|\n\
         | 7 | The seventh | Planned |\n| 11 | The eleventh | Planned |\n",
    )
    .expect("doc");
    // Two tasks: one traces a heading, one traces a row. Deliberately the
    // *second* row: with the first, "resolved the anchor" and "took whichever row
    // came first" produce the same answer, and a fixture that cannot tell them
    // apart is the one-candidate blindness `AGENTS.md` warns about. Caught by
    // mutating the resolver and watching this pass.
    for (id, trace) in [("A-1", "DOC.md#overview"), ("A-2", "DOC.md#11")] {
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {id}\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 paths:\n- DOC.md\ntraces:\n- {trace}\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
    "d213c9abcdef".to_string()
}

fn run(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = common::output(dir, args);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

fn task(dir: &Path, id: &str) -> String {
    std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md"))).expect("read")
}

#[test]
fn closing_stamps_both_anchor_shapes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());

    for id in ["A-1", "A-2"] {
        let (stdout, stderr, ok) = run(
            dir.path(),
            &["close", id, "--evidence", &format!("commit:{sha}")],
        );
        assert!(ok, "close {id} failed: {stderr}");
        assert!(
            stdout.contains("1 trace fingerprint(s) stamped"),
            "close must say it stamped: {stdout}"
        );
    }

    assert!(
        task(dir.path(), "A-1").contains("DOC.md#overview:"),
        "a heading section is stamped"
    );
    // The regression the unit tests cannot see: `anchor_fingerprint` knowing about
    // rows is not the same as `close` stamping one.
    assert!(
        task(dir.path(), "A-2").contains("DOC.md#11:"),
        "a table row is stamped, which is what ADR-0015 is for: {}",
        task(dir.path(), "A-2")
    );
}

#[test]
fn an_edit_to_a_sibling_row_is_not_this_tasks_drift() {
    // p00020's case exactly: twelve invariants under one heading, and closing a
    // task that edits invariant 11 used to re-flag every task tracing the table.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    run(
        dir.path(),
        &["close", "A-2", "--evidence", &format!("commit:{sha}")],
    );

    let doc = std::fs::read_to_string(dir.path().join("DOC.md")).expect("read");
    std::fs::write(
        dir.path().join("DOC.md"),
        doc.replace(
            "| 7 | The seventh | Planned |",
            "| 7 | The seventh | Live |",
        ),
    )
    .expect("edit");

    let (stdout, _, _) = run(dir.path(), &["check"]);
    assert!(
        !stdout.contains("APLAN018"),
        "invariant 11 did not change, so nothing may say it did: {stdout}"
    );
}

#[test]
fn an_edit_to_the_traced_row_fires_and_names_the_commit_to_diff_from() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    run(
        dir.path(),
        &["close", "A-2", "--evidence", &format!("commit:{sha}")],
    );

    let doc = std::fs::read_to_string(dir.path().join("DOC.md")).expect("read");
    std::fs::write(
        dir.path().join("DOC.md"),
        doc.replace(
            "| 11 | The eleventh | Planned |",
            "| 11 | The eleventh | Live |",
        ),
    )
    .expect("edit");

    let (stdout, _, _) = run(dir.path(), &["check"]);
    assert!(
        stdout.contains("APLAN018") && stdout.contains("DOC.md#11"),
        "the traced row changed and must be reported by name: {stdout}"
    );
    // `A1-096` is pinned as *wording* in the catalogue, which normalises the SHA
    // to a placeholder on purpose. Nothing asserted that the commit interpolated
    // is the one the task cites, and that is the half a reader pastes.
    assert!(
        stdout.contains(&format!("git diff {sha} -- DOC.md")),
        "the fix must name the commit this task cites: {stdout}"
    );
}

#[test]
fn restamp_clears_it_which_is_the_remedy_the_instruction_names() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    run(
        dir.path(),
        &["close", "A-2", "--evidence", &format!("commit:{sha}")],
    );
    let doc = std::fs::read_to_string(dir.path().join("DOC.md")).expect("read");
    std::fs::write(
        dir.path().join("DOC.md"),
        doc.replace(
            "| 11 | The eleventh | Planned |",
            "| 11 | The eleventh | Live |",
        ),
    )
    .expect("edit");

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-2", "--restamp"]);
    assert!(ok, "restamp failed: {stderr}");
    assert!(stdout.contains("restamped"), "{stdout}");

    let (stdout, _, ok) = run(dir.path(), &["check"]);
    assert!(
        ok && !stdout.contains("APLAN018"),
        "still drifting: {stdout}"
    );
}

#[test]
fn retargeting_a_trace_raises_aplan028_from_the_command_line() {
    // `A1-098` had a scenario and a unit test and no run of the sequence that
    // produces it, which is the migration ADR-0015 recommends: retarget a closed
    // task's trace from the section to the row.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    run(
        dir.path(),
        &["close", "A-1", "--evidence", &format!("commit:{sha}")],
    );

    let before = task(dir.path(), "A-1");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        before.replace("- DOC.md#overview\n", "- DOC.md#11\n"),
    )
    .expect("retarget");

    let (stdout, _, ok) = run(dir.path(), &["check"]);
    assert!(!ok, "a partially stamped task is an error: {stdout}");
    assert!(
        stdout.contains("APLAN028") && stdout.contains("DOC.md#11"),
        "the unstamped trace must be named: {stdout}"
    );
    assert!(
        stdout.contains("DOC.md#overview"),
        "and the orphan it still stamps, which is the only record of what it was \
         closed against: {stdout}"
    );

    // The remedy the instruction names repairs both halves.
    run(dir.path(), &["close", "A-1", "--restamp"]);
    let (stdout, _, ok) = run(dir.path(), &["check"]);
    assert!(ok, "restamp must clear it: {stdout}");
    assert!(
        !task(dir.path(), "A-1").contains("DOC.md#overview:"),
        "and drop the key no trace names"
    );
}
