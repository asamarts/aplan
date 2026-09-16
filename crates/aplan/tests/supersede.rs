//! `aplan supersede`, the repair `APLAN018` used to name and not have (`A1-130`).
//!
//! The rule told an adopter to "reopen the task". There is no reopen, `drop`
//! refuses to revive terminal work, and hand-editing `status` is what this tool
//! exists to stop, so the published remedy was not executable
//! (pre-public adoption evidence). ADR-0020 decides it never will be: the work *was*
//! done, and what changed is the ground under it.

use std::path::Path;

mod common;
use common::aplan;

/// A closed task tracing a document section, in a cut, with drift already
/// present: the pilot's own shape.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::create_dir_all(root.join("docs")).expect("docs");
    std::fs::write(root.join(".aplan.yml"), "detect_drift: true\n").expect("config");
    std::fs::write(
        root.join("docs/design.md"),
        "# Design\n\n## The rail\n\nThe rail is fixed at three points.\n",
    )
    .expect("doc");
    std::fs::write(
        root.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: Fix the rail at three points\nphase: 1\nstatus: done\n\
         owner: kaminsod\neffort_days: 1\npaths:\n- docs/**\ntraces:\n- docs/design.md#the-rail\n\
         evidence:\n  - kind: note\n    ref: reviewed\n    captured: 2026-08-14\n---\n\n\
         ## Why\n\nSomething.\n",
    )
    .expect("task");
    // Stamped against the current content, then the document moves under it.
    let (out, code) = aplan(root, &["close", "T-1", "--root", "aplan", "--restamp"]);
    assert_eq!(code, 0, "{out}");
    std::fs::write(
        root.join("docs/design.md"),
        "# Design\n\n## The rail\n\nThe rail is fixed at five points, and it moved.\n",
    )
    .expect("doc");
    dir
}

/// The pilot's case, end to end: the warning names an operation, the operation
/// exists, and running it clears the warning.
#[test]
fn the_remedy_the_rule_names_exists_and_clears_the_warning() {
    let dir = repo();
    let (out, _) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert!(out.contains("APLAN018"), "{out}");
    assert!(
        out.contains("`aplan supersede` it with the task that now needs doing"),
        "the rule must name a real operation:\n{out}"
    );
    assert!(!out.contains("reopen"), "{out}");

    let (out, code) = aplan(
        dir.path(),
        &[
            "supersede",
            "T-1",
            "Fix the rail at five points",
            "--root",
            "aplan",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("superseded T-1: filed T-2"), "{out}");

    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("APLAN018"),
        "the drift must be dispositioned:\n{out}"
    );
}

/// The predecessor is untouched apart from its fingerprint. Its status and its
/// evidence are still true about the moment they describe, which is the whole
/// argument for superseding rather than reopening.
#[test]
fn the_predecessor_keeps_its_status_and_its_evidence() {
    let dir = repo();
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read");
    let (out, code) = aplan(
        dir.path(),
        &["supersede", "T-1", "Fix the rail again", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    let after = std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read");

    assert!(after.contains("status: done"), "{after}");
    assert!(after.contains("ref: reviewed"), "{after}");
    assert!(after.contains("captured: 2026-08-14"), "{after}");
    // Exactly one thing changed: the stamp.
    let strip = |s: &str| {
        s.lines()
            .filter(|l| {
                !l.trim_start().starts_with("docs/design.md#") && *l != "trace_fingerprints:"
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(strip(&before), strip(&after), "more than the stamp changed");
}

/// The successor inherits the two fields that are the predecessor's own answers
/// to the same questions, and nothing that would be a guess.
#[test]
fn the_successor_inherits_paths_and_traces_and_no_evidence() {
    let dir = repo();
    let (out, code) = aplan(
        dir.path(),
        &["supersede", "T-1", "Fix the rail again", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    let successor = std::fs::read_to_string(dir.path().join("aplan/tasks/T-2.md")).expect("read");

    assert!(successor.contains("status: todo"), "{successor}");
    assert!(successor.contains("- docs/**"), "{successor}");
    assert!(
        successor.contains("- docs/design.md#the-rail"),
        "{successor}"
    );
    // The frontmatter only: the body says "keeps its evidence", which is prose
    // about the predecessor rather than a field on this task.
    let front = successor
        .split("\n---\n")
        .next()
        .expect("frontmatter")
        .to_string();
    assert!(!front.contains("evidence:"), "{front}");
    assert!(!front.contains("blocked_by:"), "{front}");
    assert!(!front.contains("trace_fingerprints:"), "{front}");
    // And it says what it succeeds, since nothing in the model records the link.
    assert!(
        successor.contains("This succeeds `T-1` (\"Fix the rail at three points\")"),
        "{successor}"
    );
    assert!(successor.contains("## Acceptance criteria"), "{successor}");
}

/// Only a `done` task has an outcome that can stop holding, and each refusal
/// leaves the plan byte-identical.
#[test]
fn a_task_that_is_not_done_is_refused_and_nothing_is_written() {
    let dir = repo();
    for (status, word) in [("todo", "todo"), ("dropped", "dropped")] {
        let path = dir.path().join("aplan/tasks/T-1.md");
        let original = std::fs::read_to_string(&path).expect("read");
        std::fs::write(
            &path,
            original.replace("status: done", &format!("status: {status}")),
        )
        .expect("write");
        let before = plan_bytes(dir.path());

        let (out, code) = aplan(
            dir.path(),
            &["supersede", "T-1", "Something else", "--root", "aplan"],
        );
        assert_ne!(code, 0, "`{status}` was accepted:\n{out}");
        assert!(out.contains(word), "the message names the status:\n{out}");
        assert!(
            out.contains("only a `done` task has an outcome that can stop holding"),
            "{out}"
        );
        assert_eq!(before, plan_bytes(dir.path()), "a refusal wrote something");
        std::fs::write(&path, original).expect("restore");
    }
}

/// An unknown id is a refusal, not a task filed against nothing.
#[test]
fn an_unknown_id_is_refused_and_nothing_is_written() {
    let dir = repo();
    let before = plan_bytes(dir.path());
    let (out, code) = aplan(
        dir.path(),
        &["supersede", "NOPE", "Something else", "--root", "aplan"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("no task `NOPE` in this plan"), "{out}");
    assert_eq!(before, plan_bytes(dir.path()));
}

/// An empty title and a nonsense estimate are both refused before anything is
/// written, exactly as `add` refuses them.
#[test]
fn a_successor_that_check_would_reject_is_refused_first() {
    let dir = repo();
    let before = plan_bytes(dir.path());
    for args in [
        vec!["supersede", "T-1", "   ", "--root", "aplan"],
        vec![
            "supersede",
            "T-1",
            "Fine",
            "--root",
            "aplan",
            "--effort",
            "-1",
        ],
    ] {
        let (out, code) = aplan(dir.path(), &args);
        assert_ne!(code, 0, "{out}");
        assert_eq!(before, plan_bytes(dir.path()), "a refusal wrote something");
    }
}

/// A finished cut stays finished. Reopening a member would have made the
/// milestone's `done` false and every downstream cut's premise with it; this is
/// the case that decided the design, so it is asserted rather than argued.
#[test]
fn a_completed_cut_is_untouched_and_stays_valid() {
    let dir = repo();
    // A milestone that lists T-1 and is itself closed: the cut completed.
    std::fs::write(
        dir.path().join("aplan/tasks/M-1.md"),
        "---\nid: M-1\ntitle: The cut that T-1 completed\nphase: 1\nstatus: done\n\
         owner: kaminsod\ntags: [milestone]\nblocked_by: [T-1]\n\
         evidence:\n  - kind: note\n    ref: cut shipped\n    captured: 2026-08-14\n---\n\n\
         ## Exit criteria\n\nSomething.\n",
    )
    .expect("milestone");
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-1.md")).expect("read");

    let (out, code) = aplan(
        dir.path(),
        &["supersede", "T-1", "Fix the rail again", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");

    assert_eq!(
        before,
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-1.md")).expect("read"),
        "the completed cut was edited"
    );
    // And the plan is still valid: no `APLAN007` about a done milestone whose
    // member became unfinished, because the member did not.
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("APLAN007"), "{out}");
}

/// Every file in the plan, so a refusal can be shown to have written nothing.
fn plan_bytes(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(root.join("aplan/tasks"))
        .expect("tasks")
        .map(|e| {
            let p = e.expect("entry").path();
            (
                p.file_name().expect("name").to_string_lossy().into_owned(),
                std::fs::read(&p).expect("read"),
            )
        })
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// The cases the criterion names, three of which this suite did not have when
// the criterion was ticked: downstream cut, milestone, and the write ordering.
// ---------------------------------------------------------------------------

/// A cut is not superseded, its members are. A milestone's outcome is that its
/// members are terminal, so filing a successor to one files work the cut does
/// not contain, into no cut, under a title nobody asked for. It did exactly that.
#[test]
fn a_milestone_is_refused_and_the_alternative_is_named() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/tasks/M-1.md"),
        "---\nid: M-1\ntitle: The cut that shipped\nphase: 1\nstatus: done\n\
         owner: kaminsod\ntags: [milestone]\nblocked_by: [T-1]\n\
         evidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-08-14\n---\n\n\
         ## Exit criteria\n\nSomething.\n",
    )
    .expect("milestone");
    let before = plan_bytes(dir.path());

    let (out, code) = aplan(
        dir.path(),
        &["supersede", "M-1", "Re-ship the cut", "--root", "aplan"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("a cut is not superseded: its members are"),
        "{out}"
    );
    assert!(
        out.contains("Supersede the member whose outcome stopped holding"),
        "the alternative has to be named:\n{out}"
    );
    assert_eq!(before, plan_bytes(dir.path()), "a refusal wrote something");
}

/// A cut sequenced after the completed one is untouched too. Reopening would
/// have invalidated the premise of everything downstream; this asserts that
/// superseding does not, rather than arguing it.
#[test]
fn a_downstream_cut_is_untouched_and_the_plan_stays_valid() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/tasks/M-1.md"),
        "---\nid: M-1\ntitle: The cut that T-1 completed\nphase: 1\nstatus: done\n\
         owner: kaminsod\ntags: [milestone]\nblocked_by: [T-1]\n\
         evidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-08-14\n---\n\n\
         ## Exit criteria\n\nSomething.\n",
    )
    .expect("milestone");
    // Sequenced after it (ADR-0013): a cut whose premise is that M-1 shipped.
    std::fs::write(
        dir.path().join("aplan/tasks/M-2.md"),
        "---\nid: M-2\ntitle: The cut that comes after it\nphase: 1\nstatus: todo\n\
         owner: kaminsod\ntags: [milestone]\nblocked_by: [M-1]\n---\n\n\
         ## Exit criteria\n\nSomething.\n",
    )
    .expect("downstream");
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-2.md")).expect("read");

    let (out, code) = aplan(
        dir.path(),
        &["supersede", "T-1", "Fix the rail again", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        before,
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-2.md")).expect("read"),
        "the downstream cut was edited"
    );
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("APLAN007"), "{out}");
}

/// Both writes are prepared before either happens, so a predecessor that cannot
/// be written is a refusal rather than half a mutation. It was half a mutation:
/// the successor was filed and the stamp was not taken, which is the state
/// criterion six exists to forbid.
#[test]
fn an_unwritable_predecessor_is_a_refusal_that_writes_nothing() {
    let dir = repo();
    let path = dir.path().join("aplan/tasks/T-1.md");
    let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms).expect("chmod");
    let before = plan_bytes(dir.path());

    let (out, code) = aplan(
        dir.path(),
        &["supersede", "T-1", "Fix the rail again", "--root", "aplan"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("is not writable"), "{out}");
    assert_eq!(
        before,
        plan_bytes(dir.path()),
        "the successor was filed and the stamp was not taken"
    );

    // Restored by mode rather than `set_readonly(false)`, which on Unix makes the
    // file world-writable.
    restore_owner_write(&path);
}

/// Put a file back to `rw-r--r--` after a read-only probe.
fn restore_owner_write(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
}

/// A repository with drift off, or a task with no `traces`, has nothing to
/// re-stamp, and the message must not report a repair that did not happen.
#[test]
fn nothing_to_restamp_is_not_reported_as_a_restamp_of_nothing() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: A closed task with no traces at all\nphase: 1\nstatus: done\n\
         owner: kaminsod\neffort_days: 1\n\
         evidence:\n  - kind: note\n    ref: reviewed\n    captured: 2026-08-14\n---\n\n\
         ## Why\n\nSomething.\n",
    )
    .expect("task");

    let (out, code) = aplan(
        dir.path(),
        &["supersede", "T-1", "Do it differently", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("superseded T-1: filed T-2"), "{out}");
    assert!(!out.contains("re-stamped"), "{out}");
    // And the successor has no traces to inherit, so it says nothing about them.
    let successor = std::fs::read_to_string(dir.path().join("aplan/tasks/T-2.md")).expect("read");
    assert!(
        !successor.contains("It traces the same section"),
        "{successor}"
    );
}

/// `--cut`, `--effort` and `--owner`, none of which any test invoked
/// (`A1-208`).
///
/// The successor is a new task, and these three decide where it lands, what it
/// costs and who holds it. A repair that filed the successor into no cut, with
/// no estimate and no owner, would leave the plan worse than the outcome it was
/// correcting -- and nothing in the suite would have noticed.
#[test]
fn the_successor_takes_the_cut_effort_and_owner_it_is_given() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/tasks/M-c1.md"),
        "---\nid: M-c1\ntitle: A cut\nphase: 1\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [T-1]\n---\n\n## Why\n\nx\n",
    )
    .expect("milestone");

    let (out, code) = aplan(
        dir.path(),
        &[
            "supersede",
            "T-1",
            "Fix the rail at five points",
            "--root",
            "aplan",
            "--cut",
            "M-c1",
            "--effort",
            "2.5",
            "--owner",
            "ada",
        ],
    );
    assert_eq!(code, 0, "{out}");

    // Read the successor from the plan rather than from what the command said,
    // because what it said is the thing under test.
    let files = std::fs::read_dir(dir.path().join("aplan/tasks")).expect("tasks");
    let successor = files
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .find(|body| body.contains("title: Fix the rail at five points"))
        .expect("the successor was written");

    assert!(
        successor.contains("owner: ada"),
        "`--owner` did not reach the successor:\n{successor}"
    );
    assert!(
        successor.contains("effort_days: 2.5"),
        "`--effort` did not reach the successor:\n{successor}"
    );

    // By its id, not by a substring that the milestone already contains: the
    // cut names `T-1` before this runs, so "does the file mention a task" is
    // answered yes whatever `--cut` did.
    let id = successor
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .expect("the successor has an id")
        .trim()
        .to_string();
    assert_ne!(
        id, "T-1",
        "the successor is a new task, not the predecessor"
    );
    let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-c1.md")).expect("cut");
    assert!(
        cut.contains(&id),
        "`--cut` did not place `{id}` in the milestone:\n{cut}"
    );
}
