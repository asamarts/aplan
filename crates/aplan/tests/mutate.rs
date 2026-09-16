//! The mutations that maintain an invariant (`A1-077`).
//!
//! `add --cut` proves the tool can keep cut membership correct, and that value
//! stopped at create: moving a task between cuts meant editing two milestone
//! files and hoping, and dropping a cut meant forty hand edits. Both are checked
//! here through the binary, because the property that matters is what the plan
//! looks like afterwards.

use std::path::Path;

mod common;
use common::aplan;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).expect("dirs");
    std::fs::write(path, body).expect("write");
}

/// Two sequential cuts, membership enforced, in the inline `[a, b]` form this
/// tool writes.
fn plan() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    write(dir.path(), ".aplan.yml", "require_cut_membership: true\n");
    for (id, phase) in [("A-01", 0), ("A-02", 0), ("B-01", 1), ("B-02", 1)] {
        write(
            dir.path(),
            &format!("aplan/tasks/{id}.md"),
            &format!(
                "---\nid: {id}\ntitle: work {id}\nphase: {phase}\nstatus: todo\n\
                 effort_days: 1\n---\n\nThe body, which must survive untouched.\n"
            ),
        );
    }
    write(
        dir.path(),
        "aplan/tasks/M-s0.md",
        "---\nid: M-s0\ntitle: Cut 0\nphase: 0\nstatus: todo\n\
         tags: [milestone, s0]\nblocked_by: [A-01, A-02]\n---\n",
    );
    // Sequenced, because that is the shape these commands are used on and the
    // shape neither the drop tests nor the sequencing tests covered: the drop
    // fixture had no `M-s0` here and the sequencing tests dropped nothing, so
    // nothing exercised the pair (`A1-084`).
    write(
        dir.path(),
        "aplan/tasks/M-s1.md",
        "---\nid: M-s1\ntitle: Cut 1\nphase: 1\nstatus: todo\n\
         tags: [milestone, s1]\nblocked_by: [B-01, B-02, M-s0]\n---\n",
    );
    dir
}

/// A cut can be created, filled and sequenced without opening an editor
/// (`A1-170`).
///
/// The whole operation rather than its steps, because every step passed on its
/// own while the sequence did not: `add` writes a milestone with no
/// `blocked_by` key at all, and an absent key was not a list `mv` could append
/// to, so a cut this tool created could not be filled by this tool. The only
/// way through was to edit YAML by hand, which is what `mv` exists to stop.
#[test]
fn a_new_cut_can_be_created_filled_and_sequenced_by_the_tool() {
    let dir = plan();

    // Create. `add` writes no `blocked_by`, which is the state under test.
    let (out, code) = aplan(
        dir.path(),
        &["add", "Cut 2", "--id", "M-s2", "--tag", "milestone"],
    );
    assert_eq!(code, 0, "{out}");
    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s2.md")).expect("read");
    assert!(
        !written.contains("blocked_by"),
        "the case only exists while `add` writes no key: {written}"
    );

    // Fill, with a member that is *not* `cross-cutting`. That qualifier is the
    // point: `add --blocked-by` could already seed a cut with a cross-cutting
    // task, and refuses an ordinary one because it would briefly be in two cuts.
    let (out, code) = aplan(dir.path(), &["mv", "B-02", "--cut", "M-s2"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("added B-02 to `M-s2`"), "{out}");
    assert!(
        out.contains("removed B-02 from `M-s1`"),
        "moved, not copied: {out}"
    );

    // Sequence, which is the third thing a new cut needs and a different edge.
    let (out, code) = aplan(dir.path(), &["dep", "add", "M-s2", "--after", "M-s1"]);
    assert_eq!(code, 0, "{out}");

    // And the plan is clean afterwards, which is the only claim worth making
    // about a sequence of edits.
    let (out, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{out}");
    let m2 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s2.md")).expect("read");
    assert!(m2.contains("B-02"), "{m2}");
    assert!(
        m2.contains("M-s1"),
        "the predecessor edge is in the same list: {m2}"
    );
    let m1 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).expect("read");
    assert!(!m1.contains("B-02"), "left its old cut: {m1}");
}

/// A task's phase can be changed by a command, and `summary` says so
/// (`A1-171`).
///
/// The rollup is read back through the binary rather than asserted against the
/// file, because the claim is that `summary` groups by the phase this tool
/// wrote -- not that a byte changed.
#[test]
fn a_task_can_be_moved_to_another_phase_and_the_rollup_follows() {
    let dir = plan();
    let (before, code) = aplan(dir.path(), &["summary"]);
    assert_eq!(code, 0, "{before}");
    assert!(
        before.contains("    0      2"),
        "two tasks in phase 0: {before}"
    );

    // Cut and phase together: one operation, one post-state, two writes.
    let (out, code) = aplan(
        dir.path(),
        &["add", "Cut 7", "--id", "M-s7", "--tag", "milestone"],
    );
    assert_eq!(code, 0, "{out}");
    let (out, code) = aplan(dir.path(), &["mv", "A-01", "--cut", "M-s7", "--phase", "3"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("added A-01 to `M-s7`"), "{out}");
    assert!(out.contains("moved A-01 to phase 3"), "{out}");

    let (after, code) = aplan(dir.path(), &["summary"]);
    assert_eq!(code, 0, "{after}");
    assert!(
        after.contains("    3      1"),
        "one task in phase 3: {after}"
    );

    // `--phase` alone, with no cut named at all.
    let (out, code) = aplan(dir.path(), &["mv", "A-02", "--phase", "3"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("moved A-02 to phase 3"), "{out}");
    let (after, code) = aplan(dir.path(), &["summary"]);
    assert_eq!(code, 0, "{after}");
    assert!(after.contains("    3      2"), "both, now: {after}");

    // A task already there is said so rather than rewritten, so a re-run is a
    // no-op and not a diff.
    let path = dir.path().join("aplan/tasks/A-02.md");
    let before_file = std::fs::read_to_string(&path).expect("read");
    let (out, code) = aplan(dir.path(), &["mv", "A-02", "--phase", "3"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("already in phase 3"), "{out}");
    assert_eq!(
        before_file,
        std::fs::read_to_string(&path).expect("read"),
        "byte-identical"
    );

    // And the plan is still clean, which is the only claim worth making about a
    // sequence of edits.
    let (out, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{out}");
}

/// A task file that cannot be written refuses the whole move, cut half included
/// (`A1-171`).
///
/// The phase half writes the *task's* file, which the cut half never touches, so
/// the probe that made `mv` all-or-nothing had a new set of files to cover. With
/// it skipped, a move that changes both would write the cut and then fail on the
/// task, leaving a plan that says one thing about membership and another about
/// the rollup.
#[test]
fn a_task_file_that_cannot_be_written_refuses_the_move_that_changes_its_phase() {
    let dir = plan();
    let (out, code) = aplan(
        dir.path(),
        &["add", "Cut 8", "--id", "M-s8", "--tag", "milestone"],
    );
    assert_eq!(code, 0, "{out}");

    let path = dir.path().join("aplan/tasks/A-01.md");
    let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms).expect("chmod");

    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).expect("read");
    let (out, code) = aplan(dir.path(), &["mv", "A-01", "--cut", "M-s8", "--phase", "4"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("is not writable"), "{out}");
    assert_eq!(
        before,
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).expect("read"),
        "the cut half must not have been written"
    );
    let m8 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s8.md")).expect("read");
    assert!(!m8.contains("A-01"), "nor the destination: {m8}");

    restore_owner_write(&path);
}

/// A refusal from the cut half stops the phase half too (`A1-171`, audit).
///
/// The two halves are one operation, so a `--cut` that cannot be honoured must
/// not leave the phase changed behind it. `mv` refuses a *milestone* by name,
/// which is the cheapest refusal to reach and lands before anything is written.
#[test]
fn a_cut_refusal_leaves_the_phase_alone() {
    let dir = plan();
    let path = dir.path().join("aplan/tasks/M-s0.md");
    let before = std::fs::read_to_string(&path).expect("read");

    let (out, code) = aplan(dir.path(), &["mv", "M-s0", "--cut", "M-s1", "--phase", "9"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("is a cut, not a task in one"), "{out}");
    assert_eq!(
        before,
        std::fs::read_to_string(&path).expect("read"),
        "the phase half must not have run"
    );

    // And a milestone's *own* phase is an ordinary move: a cut has a phase and
    // `summary` rolls it up like anything else.
    let (out, code) = aplan(dir.path(), &["mv", "M-s0", "--phase", "9"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        std::fs::read_to_string(&path)
            .expect("read")
            .contains("phase: 9"),
        "a cut's phase is not special"
    );
}

/// `mv` with neither `--cut` nor `--phase` is not a move (`A1-171`).
#[test]
fn a_move_that_names_no_destination_is_refused_by_the_parser() {
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["mv", "A-01"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("--cut"), "it names the flag it wanted: {out}");
}

/// `add --cut` and `import --cut` fill a new cut too (`A1-170`, audit).
///
/// The task named two shut doors and there were four: everything that reaches
/// `place` was refused by the same absent key. `add --cut` was the worst of
/// them, because it wrote the task file, failed to place it, and then deleted
/// what it had written -- so the natural way to put the first piece of work into
/// a new cut left no trace that anything had been attempted.
#[test]
fn a_new_cut_can_be_filled_by_add_and_by_import() {
    let dir = plan();
    let (out, code) = aplan(
        dir.path(),
        &["add", "Cut 4", "--id", "M-s4", "--tag", "milestone"],
    );
    assert_eq!(code, 0, "{out}");

    // `add --cut`: a brand-new task straight into the brand-new cut.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "First work",
            "--id",
            "C-01",
            "--cut",
            "M-s4",
            "--effort",
            "1",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        dir.path().join("aplan/tasks/C-01.md").exists(),
        "the task it wrote survives being placed: {out}"
    );
    let m4 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s4.md")).expect("read");
    assert!(m4.contains("blocked_by: [C-01]"), "{m4}");

    // `import --cut`: the same placement, from the other writer.
    let (out, code) = aplan(
        dir.path(),
        &["add", "Cut 5", "--id", "M-s5", "--tag", "milestone"],
    );
    assert_eq!(code, 0, "{out}");
    std::fs::write(
        dir.path().join("tasks.md"),
        "# Feature\n\n## Tasks\n\n- [ ] T001 Something imported\n",
    )
    .expect("write");
    let (out, code) = aplan(
        dir.path(),
        &[
            "import",
            "--format",
            "spec-kit",
            "tasks.md",
            "--cut",
            "M-s5",
            "--id-prefix",
            "IMP",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let m5 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s5.md")).expect("read");
    assert!(m5.contains("blocked_by: [IMP-T001]"), "{m5}");
}

/// A `mv` that changes nothing writes nothing, so the key is not created for a
/// task that was already there (`A1-170`, audit).
#[test]
fn a_no_op_mv_does_not_create_the_key_it_would_have_appended_to() {
    let dir = plan();
    let (out, code) = aplan(
        dir.path(),
        &["add", "Cut 6", "--id", "M-s6", "--tag", "milestone"],
    );
    assert_eq!(code, 0, "{out}");
    let empty = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s6.md")).expect("read");
    let target = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).expect("read");

    // `A-01` is already in `M-s0`, so moving it there moves nothing: the guard
    // that skips an id already present runs before the file is opened.
    let (out, code) = aplan(dir.path(), &["mv", "A-01", "--cut", "M-s0"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("already in"), "{out}");
    assert_eq!(
        target,
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).expect("read"),
        "the cut it was already in is byte-identical"
    );
    let after = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s6.md")).expect("read");
    assert_eq!(empty, after, "and the empty cut beside it is untouched");
    assert!(!after.contains("blocked_by"), "so it gains no key: {after}");
}

/// The fix does not buy itself a hole: a task may not be in two cuts, even for
/// the instant it takes to seed a new one (`A1-170`).
#[test]
fn seeding_a_cut_with_a_task_that_already_has_one_is_still_refused() {
    let dir = plan();
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Cut 3",
            "--id",
            "M-s3",
            "--tag",
            "milestone",
            "--blocked-by",
            "B-01",
        ],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN022"), "{out}");
    assert!(out.contains("belongs to 2 cuts"), "{out}");
    assert!(
        !dir.path().join("aplan/tasks/M-s3.md").exists(),
        "a refused mutation writes nothing"
    );
}

#[test]
fn dropping_a_cut_leaves_the_cut_it_follows_alone() {
    // `A1-073` put the predecessor cut in a milestone's `blocked_by`, and
    // `drop --cut` read the whole list as membership, so cancelling one cut
    // cancelled the one before it and left that cut's work live. `check` called
    // the result clean.
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["drop", "M-s1", "--cut"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("3 task(s) dropped"),
        "the cut and its work: {out}"
    );

    let s0 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).unwrap();
    assert!(
        s0.contains("status: todo"),
        "the predecessor cut survives: {s0}"
    );
    for id in ["A-01", "A-02"] {
        let t = std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{id}.md"))).unwrap();
        assert!(t.contains("status: todo"), "{id} is cut zero's work: {t}");
    }
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
}

#[test]
fn a_cancelled_cut_with_live_work_is_reported() {
    // Dropping a milestone alone cancels the checkpoint and nothing else, which
    // is a plan saying the cut is cancelled and its work is going ahead.
    let dir = plan();
    aplan(dir.path(), &["drop", "M-s1"]);
    let (check, _) = aplan(dir.path(), &["check"]);
    assert!(check.contains("APLAN027"), "{check}");
    assert!(check.contains("B-01"), "it names the live members: {check}");
}

#[test]
fn a_milestone_cannot_be_moved_between_cuts() {
    // Moving one rewrites which cut follows which, not membership: the source
    // stops following it and the destination claims it as a member, silently.
    let dir = plan();
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).unwrap();
    let (out, code) = aplan(dir.path(), &["mv", "M-s0", "--cut", "M-s1"]);
    assert_ne!(code, 0, "{out}");
    let after = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).unwrap();
    assert_eq!(before, after, "nothing may change");
}

#[test]
fn moving_a_task_edits_both_milestones_and_leaves_the_plan_clean() {
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["mv", "B-01", "--cut", "M-s0"]);
    assert_eq!(code, 0, "{out}");

    let s0 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).unwrap();
    let s1 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).unwrap();
    assert!(s0.contains("[A-01, A-02, B-01]"), "{s0}");
    // The sequencing edge stays: it is not membership, and a move must not
    // touch it (`A1-084`).
    assert!(s1.contains("[B-02, M-s0]"), "{s1}");

    // The invariant is the point: exactly one cut, still, and `check` agrees.
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
}

/// A group moves in one command, and the command names what it moved (`A1-127`).
///
/// Re-prioritising nine tasks was nine invocations and nine chances to stop half
/// way, which the fourth pilot measured as "the dominant cost after authoring"
/// at 105 tasks (pre-public adoption evidence item 6). A count would be a worse answer
/// than a list: the set is exactly what the author is unsure of.
#[test]
fn several_tasks_move_in_one_command_and_it_names_them() {
    let dir = plan();
    // A third member, so the source keeps work after two leave: emptying a cut
    // is a refusal of its own and is tested where that is the subject.
    write(
        dir.path(),
        "aplan/tasks/B-03.md",
        "---\nid: B-03\ntitle: work B-03\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
    );
    write(
        dir.path(),
        "aplan/tasks/M-s1.md",
        "---\nid: M-s1\ntitle: Cut 1\nphase: 1\nstatus: todo\n\
         tags: [milestone, s1]\nblocked_by: [B-01, B-02, B-03, M-s0]\n---\n",
    );

    let (out, code) = aplan(dir.path(), &["mv", "B-01", "B-02", "--cut", "M-s0"]);
    assert_eq!(code, 0, "{out}");
    for id in ["B-01", "B-02"] {
        assert!(out.contains(&format!("added {id} to `M-s0`")), "{out}");
        assert!(out.contains(&format!("removed {id} from `M-s1`")), "{out}");
    }
    assert!(!out.contains("2 task"), "a count is not an answer: {out}");

    let s0 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).unwrap();
    let s1 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).unwrap();
    assert!(s0.contains("[A-01, A-02, B-01, B-02]"), "{s0}");
    // The sequencing edge is untouched, because it was never membership
    // (`A1-084`), and the member that stayed is still there.
    assert!(s1.contains("[B-03, M-s0]"), "{s1}");

    // `require_cut_membership` is on in this fixture, so exactly-one-cut is the
    // invariant the whole move has to keep, not just each step of it.
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
}

/// One member that cannot move refuses the move, rather than leaving the ones
/// before it in the new cut (`A1-127`).
///
/// This is the property `A1-030` fixed and `A1-084` called `mv`'s reason for
/// being trustworthy. A bulk form that applied what it could would re-create
/// exactly the half-written state the single form was careful to avoid, and at
/// nine times the size.
#[test]
fn a_bulk_move_is_all_or_nothing() {
    let cases: [(&[&str], &str); 3] = [
        // An id nobody has.
        (&["mv", "B-01", "NOPE", "--cut", "M-s0"], "no task `NOPE`"),
        // A cut, which is a different edit wearing this one's clothes.
        (
            &["mv", "B-01", "M-s1", "--cut", "M-s0"],
            "is a cut, not a task in one",
        ),
        // Both of the source's work members, which empties it of work while
        // leaving `blocked_by` non-empty. A rule sees that since `A1-134`, so it
        // is refused by the repository's policy rather than by this command --
        // and the set must still be all-or-nothing when the policy stops it.
        (&["mv", "A-01", "A-02", "--cut", "M-s1"], "APLAN021"),
    ];

    for (args, expected) in cases {
        let dir = plan();
        if expected == "APLAN021" {
            std::fs::write(
                dir.path().join(".aplan.yml"),
                "require_cut_membership: true\ndeny_warnings: true\n",
            )
            .expect("config");
        }
        let args: Vec<&str> = args.to_vec();
        let before: Vec<String> = ["M-s0", "M-s1"]
            .iter()
            .map(|m| {
                std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{m}.md"))).unwrap()
            })
            .collect();

        let (out, code) = aplan(dir.path(), &args);
        assert_ne!(code, 0, "{args:?} was accepted:\n{out}");
        assert!(out.contains(expected), "{out}");

        let after: Vec<String> = ["M-s0", "M-s1"]
            .iter()
            .map(|m| {
                std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{m}.md"))).unwrap()
            })
            .collect();
        assert_eq!(before, after, "a refusal wrote something: {args:?}");
    }
}

/// A file this move must write and cannot is a refusal, not half a move
/// (`A1-127`).
///
/// Measured before the probe existed: with the destination writable and one
/// source read-only, `mv` wrote the destination, failed on the source, and left
/// the task in **two cuts** while `check` still said `ok`, because `APLAN022` is
/// off unless a repository asks for it. Every file the move will write is proved
/// writable before any of them is, the way `supersede` does it (`A1-130`).
#[test]
fn a_source_that_cannot_be_written_refuses_the_whole_move() {
    for target in ["M-s1", "M-s0"] {
        let dir = plan();
        let path = dir.path().join(format!("aplan/tasks/{target}.md"));
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).expect("chmod");

        let before: Vec<String> = ["M-s0", "M-s1"]
            .iter()
            .map(|m| {
                std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{m}.md"))).unwrap()
            })
            .collect();

        let (out, code) = aplan(dir.path(), &["mv", "B-01", "--cut", "M-s0"]);
        assert_ne!(code, 0, "{out}");
        assert!(out.contains("is not writable"), "{out}");
        // Once, not twice: the error's `source` is printed by the chain, so
        // naming it in the message as well said "Permission denied" twice.
        assert_eq!(
            out.matches("Permission denied").count(),
            1,
            "the reason is printed once: {out}"
        );

        let after: Vec<String> = ["M-s0", "M-s1"]
            .iter()
            .map(|m| {
                std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{m}.md"))).unwrap()
            })
            .collect();
        assert_eq!(
            before, after,
            "`{target}` unwritable, and something was written"
        );

        // Restored by mode rather than `set_readonly(false)`, which on Unix
        // makes the file world-writable.
        restore_owner_write(&path);
    }
}

/// The same probe for `drop`, which writes a file per member (`A1-127`).
///
/// `drop --cut` cancels the milestone and everything in it, so one unwritable
/// file left half a cut cancelled and half of it live. The refusal is the whole
/// point of working the rewrites out in memory first, and it was missing the
/// last step.
#[test]
fn an_unwritable_member_refuses_the_whole_drop() {
    let dir = plan();
    // The later cut, since dropping the one others follow is refused earlier, by
    // `APLAN020` on the tasks that would then wait on a cancelled cut.
    let path = dir.path().join("aplan/tasks/B-02.md");
    let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms).expect("chmod");

    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/B-01.md")).unwrap();
    let (out, code) = aplan(dir.path(), &["drop", "M-s1", "--cut"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("is not writable"), "{out}");
    assert_eq!(
        before,
        std::fs::read_to_string(dir.path().join("aplan/tasks/B-01.md")).unwrap(),
        "another member was cancelled before the failure"
    );

    restore_owner_write(&path);
}

/// Put a file back to `rw-r--r--` after a read-only probe. By mode rather than
/// `set_readonly(false)`, which on Unix makes the file world-writable.
fn restore_owner_write(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
}

/// Naming a task twice is one move, and must not count twice when the command
/// asks whether a cut is being emptied of its work (`A1-127`).
#[test]
fn a_task_named_twice_is_moved_once() {
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["mv", "B-01", "B-01", "--cut", "M-s0"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out.matches("added B-01").count(),
        1,
        "said twice, done once: {out}"
    );
    let s0 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).unwrap();
    assert!(s0.contains("[A-01, A-02, B-01]"), "{s0}");

    // And `M-s1` still has `B-02`, so the duplicate did not make the command
    // think both members were leaving.
    let s1 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).unwrap();
    assert!(s1.contains("B-02"), "{s1}");
}

#[test]
fn moving_into_a_cut_that_does_not_exist_changes_nothing() {
    // A refusal must leave the plan exactly as it was: a half-applied move is a
    // plan that says something nobody decided.
    let dir = plan();
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).unwrap();
    let (out, code) = aplan(dir.path(), &["mv", "B-01", "--cut", "M-nope"]);
    assert_ne!(code, 0, "{out}");
    let after = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).unwrap();
    assert_eq!(before, after, "the source milestone must be untouched");
}

#[test]
fn a_move_whose_source_cannot_be_edited_changes_nothing() {
    // The failure the criterion names: adding to the destination and only then
    // discovering the source cannot be edited surgically leaves the task in two
    // cuts and the plan failing `APLAN022`. A partial write is worse than a
    // refusal, so every rewrite is worked out before any is saved.
    let dir = plan();
    write(
        dir.path(),
        "aplan/tasks/M-s1.md",
        "---\nid: M-s1\ntitle: Cut 1\nphase: 1\nstatus: todo\n\
         tags: [milestone, s1]\nblocked_by: &anchor [B-01, B-02]\n---\n",
    );
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).unwrap();

    let (out, code) = aplan(dir.path(), &["mv", "B-01", "--cut", "M-s0"]);
    assert_ne!(code, 0, "{out}");
    let after = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s0.md")).unwrap();
    assert_eq!(before, after, "the destination must not have been written");

    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "the plan must still validate: {check}");
}

#[test]
fn a_task_named_twice_is_dropped_once() {
    // One task named twice, or named alongside the cut that contains it, is one
    // cancellation. `APLAN026` had the same defect for a repeated tag.
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["drop", "A-01", "A-01"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 task(s) dropped"), "{out}");
}

#[test]
fn dropping_a_cut_is_one_command_and_the_plan_stays_clean() {
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["drop", "M-s1", "--cut"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("3 task(s) dropped"), "{out}");

    for id in ["B-01", "B-02", "M-s1"] {
        let text =
            std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{id}.md"))).unwrap();
        assert!(text.contains("status: dropped"), "{id}: {text}");
    }
    // Cut 0 is untouched, and the plan still validates.
    let a1 = std::fs::read_to_string(dir.path().join("aplan/tasks/A-01.md")).unwrap();
    assert!(a1.contains("status: todo"), "{a1}");
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
}

#[test]
fn a_body_survives_a_drop_byte_for_byte() {
    // The edit is surgical: one line of frontmatter, and nothing else.
    let dir = plan();
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/A-01.md")).unwrap();
    aplan(dir.path(), &["drop", "A-01"]);
    let after = std::fs::read_to_string(dir.path().join("aplan/tasks/A-01.md")).unwrap();
    assert_eq!(
        before.replace("status: todo", "status: dropped"),
        after,
        "only the status line may change"
    );
}

#[test]
fn dropping_a_terminal_task_is_refused_rather_than_laundered() {
    // `dropped` records a decision and `done` records evidence. Rewriting either
    // silently would launder something the plan said, which is why `close`
    // refuses them too.
    let dir = plan();
    aplan(dir.path(), &["drop", "A-01"]);
    let (out, code) = aplan(dir.path(), &["drop", "A-01"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("already"), "{out}");
}

#[test]
fn one_bad_id_stops_the_whole_batch() {
    // Every id is resolved before any file is written, so a typo in the third of
    // four does not leave two tasks dropped.
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["drop", "A-01", "GHOST"]);
    assert_ne!(code, 0, "{out}");
    let a1 = std::fs::read_to_string(dir.path().join("aplan/tasks/A-01.md")).unwrap();
    assert!(a1.contains("status: todo"), "nothing was written: {a1}");
}

#[test]
fn dropping_a_milestone_without_cut_leaves_its_members_alone() {
    // `--cut` is the destructive one, and it has to be asked for by name.
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["drop", "M-s1"]);
    assert_eq!(code, 0, "{out}");
    let b1 = std::fs::read_to_string(dir.path().join("aplan/tasks/B-01.md")).unwrap();
    assert!(b1.contains("status: todo"), "{b1}");
}

#[test]
fn a_declared_family_groups_the_roadmap_view() {
    // The command path: `roadmap.group_by` read from a real `.aplan.yml`,
    // through the binary, which the library tests cannot reach (`A1-078`).
    let dir = plan();
    write(
        dir.path(),
        ".aplan.yml",
        "require_cut_membership: true\ntag_families:\n  epic: [epic-a, epic-b]\n\
         roadmap:\n  group_by: epic\n",
    );
    for (id, tag) in [("A-01", "epic-b"), ("A-02", "epic-a")] {
        write(
            dir.path(),
            &format!("aplan/tasks/{id}.md"),
            &format!(
                "---\nid: {id}\ntitle: work {id}\nphase: 0\nstatus: todo\n\
                 effort_days: 1\ntags: [{tag}]\n---\n"
            ),
        );
    }
    let (out, code) = aplan(dir.path(), &["roadmap"]);
    assert_eq!(code, 0, "{out}");
    let a = out.find("epic-a").expect("epic-a heading");
    let b = out.find("epic-b").expect("epic-b heading");
    assert!(a < b, "declared order, not id order: {out}");
}

#[test]
fn grouping_by_an_undeclared_family_is_refused_at_load() {
    // Config the tool cannot act on is an error, never a silent no-op: grouping
    // by a vocabulary nothing validates would render a heading a typo invented.
    let dir = plan();
    write(
        dir.path(),
        ".aplan.yml",
        "require_cut_membership: true\nroadmap:\n  group_by: epic\n",
    );
    let (out, code) = aplan(dir.path(), &["roadmap"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("tag_families"), "{out}");
}

/// A cut emptied of its work is the rule's finding, not the command's refusal
/// (`A1-134`).
///
/// `mv` used to hand-roll this check, because `APLAN021` asked whether
/// `blocked_by` was *empty* and a sequenced cut keeps its predecessor there, so
/// no rule could see it. The rule counts live work members now, which puts the
/// judgement in one place: the mutation goes through, the post-state check names
/// the finding it introduces (`A1-113`), and a repository that fails on warnings
/// is the one that stops it.
/// Proves the named behaviour `milestone.members_are_live_work` (`A1-211`).
#[test]
fn a_cut_emptied_of_work_is_reported_rather_than_refused() {
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["mv", "B-01", "B-02", "--cut", "M-s0"]);
    assert_eq!(code, 0, "the move was refused by a command again:\n{out}");
    assert!(
        out.contains("APLAN021") && out.contains("M-s1"),
        "the mutation must say what it introduces:\n{out}"
    );

    // And the plan says it afterwards, which is the half a command's refusal
    // could never leave behind.
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "a warning is not an error:\n{check}");
    assert!(
        check.contains("APLAN021") && check.contains("has no work in it"),
        "{check}"
    );

    // Under a policy that fails on warnings, the same move is refused -- by the
    // repository's own policy rather than by this command's opinion.
    let dir = plan();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "require_cut_membership: true\ndeny_warnings: true\n",
    )
    .expect("config");
    let (out, code) = aplan(dir.path(), &["mv", "B-01", "B-02", "--cut", "M-s0"]);
    assert_ne!(code, 0, "the policy did not stop it:\n{out}");
    assert!(out.contains("APLAN021"), "{out}");
    assert!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md"))
            .expect("read")
            .contains("B-01"),
        "the refusal moved it anyway"
    );
}

/// Every surface that takes a `--cut` takes the label too (`A1-139`).
///
/// `roadmap` prints the label, `tasks --format json` carries both, and the
/// commands that write used to accept only the id -- so the first command an
/// adopter runs refused the name the first document they read had given them.
/// One test per surface, because a shared resolver is only shared while every
/// caller uses it (`A1-107`).
/// Proves the named behaviour `cut.named_by_id_or_label` (`A1-211`).
#[test]
fn every_cut_taking_command_accepts_the_label() {
    // `add`, and the task lands in the cut the label named.
    let dir = plan();
    let (out, code) = aplan(
        dir.path(),
        &["add", "By label", "--cut", "s1", "--effort", "1"],
    );
    assert_eq!(code, 0, "{out}");
    let s1 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).expect("read");
    assert!(s1.contains("B-03") || s1.contains("A-03"), "{s1}");

    // `mv`, moving into the cut the label named.
    let (out, code) = aplan(dir.path(), &["mv", "A-01", "--cut", "s1"]);
    assert_eq!(code, 0, "{out}");
    let s1 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).expect("read");
    assert!(s1.contains("A-01"), "{s1}");

    // `graph`, which had the rule first and now shares it. `--membership`
    // because the question here is whether the *label* resolves: this cut's
    // members do not depend on one another, and since `A1-216` the default view
    // is the dependency graph, which for this cut is empty.
    let (out, code) = aplan(dir.path(), &["graph", "--cut", "s1", "--membership"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("A-01"), "{out}");

    // `drop --cut`, the command that names cuts positionally: the audit found it
    // still refusing the label every other surface had learned to take.
    let dir = plan();
    let (out, code) = aplan(dir.path(), &["drop", "s1", "--cut"]);
    assert_eq!(code, 0, "`drop --cut` refused a label:\n{out}");
    let s1 = std::fs::read_to_string(dir.path().join("aplan/tasks/M-s1.md")).expect("read");
    assert!(s1.contains("status: dropped"), "{s1}");

    // And a name nobody answers to is refused with the cuts this plan has.
    let dir = plan();
    let (out, code) = aplan(
        dir.path(),
        &["add", "Nowhere", "--cut", "nope", "--effort", "1"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("M-s0") && out.contains("M-s1"),
        "the refusal must answer with the cuts that exist: {out}"
    );
}
