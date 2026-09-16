//! Concurrent writers against one cut (`A1-226`).
//!
//! The defect: `add --cut` read the milestone, computed a new `blocked_by`, and
//! wrote it. Two agents doing that at once left the second one's membership only,
//! and **both were told `wrote <id>`**. Measured with twenty-four agents against a
//! 2,000-member cut: 23 of 24 landed, then 22 of 24, then two clean rounds. With
//! the default configuration the plan afterwards reported `0 warning(s)`.
//!
//! The invariant these tests hold is deliberately not "every writer succeeds".
//! A writer that would have clobbered somebody is *refused* now, which is a
//! correct outcome and a retryable one. What must never happen is a writer being
//! told it succeeded while its membership is not there:
//!
//! > memberships added == writers that reported success
//!
//! That assertion cannot flake. It holds when the race never triggers, and it
//! fails loudly the moment an update is lost.

use std::path::Path;

mod common;

/// A plan with one cut already holding `members` tasks, and ULID ids so two
/// concurrent `add`s cannot derive the same id -- that is a different race
//  (`A1-230`), and letting it fire here would measure the wrong thing.
fn fixture(dir: &Path, members: usize) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\nid_strategy: ulid\n").expect("config");

    let mut ids = Vec::new();
    for i in 0..members {
        let id = format!("T{i:04}");
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: seed {i}\nphase: 0\nstatus: todo\nowner: t\n\
                 effort_days: 1\n---\n\n## Why\n\nx\n"
            ),
        )
        .expect("seed task");
        ids.push(id);
    }
    std::fs::write(
        dir.join("aplan/tasks/M-c1.md"),
        format!(
            "---\nid: M-c1\ntitle: Cut one\nphase: 0\nstatus: todo\ntags: [milestone]\n\
             blocked_by: [{}]\n---\n\n## Why\n\nx\n",
            ids.join(", ")
        ),
    )
    .expect("milestone");
}

/// How many ids the milestone lists.
fn members(dir: &Path) -> usize {
    let text = std::fs::read_to_string(dir.join("aplan/tasks/M-c1.md")).expect("read milestone");
    let line = text
        .lines()
        .find(|l| l.starts_with("blocked_by:"))
        .expect("blocked_by line");
    let inner = line
        .trim_start_matches("blocked_by:")
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    inner.split(',').filter(|s| !s.trim().is_empty()).count()
}

/// Run `writers` concurrent `add --cut` against one cut; return how many
/// reported success.
fn race(dir: &Path, writers: usize) -> usize {
    let handles: Vec<_> = (0..writers)
        .map(|i| {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                common::output(
                    &dir,
                    &[
                        "add",
                        &format!("Concurrent writer {i}"),
                        "--cut",
                        "M-c1",
                        "--effort",
                        "1",
                    ],
                )
                .status
                .success()
            })
        })
        .collect();
    handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .filter(|ok| *ok)
        .count()
}

/// The defect, as an invariant: nobody is told it succeeded while its
/// membership is missing.
#[test]
fn every_writer_that_reported_success_is_in_the_cut() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), 200);
    let before = members(dir.path());

    let succeeded = race(dir.path(), 16);
    let after = members(dir.path());

    assert!(
        succeeded > 0,
        "no writer succeeded at all, so this proves nothing"
    );
    assert_eq!(
        after - before,
        succeeded,
        "{succeeded} writer(s) reported success and {} membership(s) landed: a write \
         reported success and lost what it wrote",
        after - before
    );
}

/// The same, run repeatedly, because the loss was intermittent: it appeared in
/// two of four rounds when it was found.
#[test]
fn the_invariant_holds_over_repeated_rounds() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), 100);

    for round in 1..=3 {
        let before = members(dir.path());
        let succeeded = race(dir.path(), 12);
        let after = members(dir.path());
        assert_eq!(
            after - before,
            succeeded,
            "round {round}: {succeeded} reported success, {} landed",
            after - before
        );
    }
}

/// "By construction" is the criterion, so it is checked rather than asserted in
/// a commit message.
///
/// `add --cut`, `mv` and `drop` all reach the filesystem through `cut.rs`, and
/// the fix is only true for all three while that file has no write of its own.
/// A future writer added beside `save` would be silently unsafe again, and this
/// is what notices. Structural, in the shape `hermetic.rs` uses: read the source
/// and refuse a name.
#[test]
fn no_milestone_write_bypasses_the_guarded_path() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../aplan-interop/src/cut.rs"),
    )
    .expect("read cut.rs");

    let production: String = src
        .lines()
        .take_while(|l| !l.trim_start().starts_with("#[cfg(test)]"))
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !production.contains("fs::write"),
        "cut.rs writes a milestone directly. Every write here must go through \
         `save`, or `add --cut`, `mv` and `drop` are not covered by construction \
         and the next concurrent writer loses somebody's membership again (`A1-226`)."
    );
    assert!(
        production.contains("aplan_core::save::replace"),
        "cut.rs no longer routes through the guarded writer; this gate is now \
         checking nothing"
    );
}

/// A refused writer must leave the plan valid, not half-edited: the task file it
/// already wrote is the caller's to retry with, and the milestone must still
/// parse and still list exactly what it listed.
#[test]
fn a_plan_survives_a_round_of_contention() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path(), 50);
    race(dir.path(), 12);

    let out = common::output(dir.path(), &["check"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("DAG acyclic"),
        "the plan must still load and validate after contention: {text}"
    );

    let left: Vec<_> = std::fs::read_dir(dir.path().join("aplan/tasks"))
        .expect("read tasks")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(
        left.is_empty(),
        "a temporary file survived contention: {left:?}"
    );
}
