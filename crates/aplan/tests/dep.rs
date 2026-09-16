//! A dependency edge can be added and removed through the tool (`A1-124`).
//!
//! `blocked_by` is the only per-task ordering the model has, and it was written
//! by `add` and never again. Checked across every write path before this: `add`
//! set it at creation, `mv` edited a *milestone's* membership, and nothing edited
//! a task's. So the one mechanism this tool uses to say "this comes first" was
//! maintainable only in a text editor, in a repository whose premise is that the
//! tool owns those files (pre-public adoption evidence, item 6).
//!
//! Every refusal here is `A1-113`'s post-state check rather than a private copy:
//! a cycle is `APLAN004`, an unknown id `APLAN003`, a cancelled blocker
//! `APLAN020`, and a `done` task acquiring an unfinished one `APLAN007`. That is
//! why this task waited for that one.

use std::path::Path;

mod common;
use common::aplan;

fn write_task(root: &Path, id: &str, title: &str, status: &str, extra: &str) {
    std::fs::write(
        root.join(format!("aplan/tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: {status}\nowner: kaminsod\n\
             effort_days: 1\n{extra}---\n\n## Why\n\nSomething.\n"
        ),
    )
    .expect("task");
}

/// Two live tasks, one cancelled, and one closed.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::write(root.join(".aplan.yml"), "").expect("config");
    write_task(root, "A-1", "The first task", "todo", "");
    write_task(root, "A-2", "The second task", "todo", "");
    write_task(root, "A-3", "A cancelled task", "dropped", "");
    write_task(
        root,
        "A-4",
        "A closed task",
        "done",
        "evidence:\n  - kind: note\n    ref: reviewed\n    captured: 2026-08-15\n",
    );
    dir
}

/// The thing that could not be done at all: say that one task comes after
/// another, after both exist.
#[test]
fn an_edge_is_added_and_removed_without_touching_the_file_by_hand() {
    let dir = repo();

    let (out, code) = aplan(
        dir.path(),
        &["dep", "add", "A-2", "--after", "A-1", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("now `A-2` comes after `A-1`"), "{out}");

    // `why` is the read side of the same fact, so it is what proves the edge is
    // real rather than a message about one.
    let (why, code) = aplan(dir.path(), &["why", "A-2", "--root", "aplan"]);
    assert_eq!(code, 0, "{why}");
    assert!(why.contains("A-1"), "{why}");

    let (out, code) = aplan(
        dir.path(),
        &["dep", "rm", "A-2", "--after", "A-1", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no longer `A-2` comes after `A-1`"), "{out}");

    let (why, _) = aplan(dir.path(), &["why", "A-2", "--root", "aplan"]);
    assert!(why.contains("nothing blocks it"), "{why}");

    // And the plan is clean throughout, which is the point of editing it here
    // rather than in an editor.
    let (check, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{check}");
}

/// Each of the four cases the criterion names, refused before anything is
/// written, each naming the rule that would have rejected the result.
#[test]
fn each_illegal_edge_is_refused_and_names_its_rule() {
    let cases = [
        // A cycle: `A-2` already comes after `A-1`.
        ("A-1", "A-2", "APLAN004", "dependency cycle"),
        // An id nobody has.
        ("A-2", "NOPE", "APLAN003", "references unknown task `NOPE`"),
        // A cancelled blocker: the task could never legitimately start.
        ("A-2", "A-3", "APLAN020", "is `dropped`"),
        // A closed task acquiring an unfinished blocker.
        ("A-4", "A-1", "APLAN007", "is `done` but blocker"),
    ];

    for (id, after, rule, phrase) in cases {
        let dir = repo();
        // The cycle case needs the first edge to exist.
        if rule == "APLAN004" {
            let (out, code) = aplan(
                dir.path(),
                &["dep", "add", "A-2", "--after", "A-1", "--root", "aplan"],
            );
            assert_eq!(code, 0, "{out}");
        }
        let before =
            std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{id}.md"))).expect("read");

        let (out, code) = aplan(
            dir.path(),
            &["dep", "add", id, "--after", after, "--root", "aplan"],
        );
        assert_ne!(code, 0, "`{id}` after `{after}` was accepted:\n{out}");
        assert!(out.contains(rule), "it must name {rule}:\n{out}");
        assert!(out.contains(phrase), "{out}");
        // And the remedy, because the refusal carries what the rule carries.
        assert!(out.contains("fix: "), "{out}");

        assert_eq!(
            before,
            std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{id}.md"))).expect("read"),
            "`{id}` was written despite the refusal"
        );
    }
}

/// A task cannot come after itself, which no rule would catch: `APLAN006` is
/// about a task blocking itself and would fire, but the message an author wants
/// is about what they just typed.
#[test]
fn a_task_cannot_come_after_itself() {
    let dir = repo();
    let (out, code) = aplan(
        dir.path(),
        &["dep", "add", "A-1", "--after", "A-1", "--root", "aplan"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("cannot come after itself"), "{out}");
}

/// Saying twice what is already true changes nothing and says so, rather than
/// writing the file again or reporting a change that did not happen.
#[test]
fn repeating_an_edge_is_a_no_op_that_says_so() {
    let dir = repo();
    aplan(
        dir.path(),
        &["dep", "add", "A-2", "--after", "A-1", "--root", "aplan"],
    );
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/A-2.md")).expect("read");

    let (out, code) = aplan(
        dir.path(),
        &["dep", "add", "A-2", "--after", "A-1", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("already comes after"), "{out}");
    assert_eq!(
        before,
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-2.md")).expect("read")
    );

    // And removing an edge that is not there.
    let (out, code) = aplan(
        dir.path(),
        &["dep", "rm", "A-1", "--after", "A-2", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("does not come after"), "{out}");
}

/// A cut's `blocked_by` is which tasks are in it, not what it comes after, so
/// editing one end of it here would be `mv` with its guards removed.
///
/// Both holes were measured before this refusal existed: `dep rm` on the last
/// work member of a *sequenced* cut left `blocked_by` holding its predecessor,
/// so `APLAN021` saw a non-empty list and `check` printed `ok`; and `dep add`
/// put a task in two cuts, which only `APLAN022` catches and only when
/// `require_cut_membership` is set, which by default it is not.
#[test]
fn a_cuts_membership_is_not_its_ordering() {
    let dir = repo();
    let root = dir.path();
    write_task(root, "M-0", "The first cut", "todo", "tags:\n- milestone\n");
    write_task(
        root,
        "M-1",
        "The second cut",
        "todo",
        "tags:\n- milestone\nblocked_by: [A-1, M-0]\n",
    );

    let cut =
        |id: &str| std::fs::read_to_string(root.join(format!("aplan/tasks/{id}.md"))).unwrap();
    let before = cut("M-1");

    // Taking the last work member out, and putting a second one in.
    for args in [
        ["dep", "rm", "M-1", "--after", "A-1"],
        ["dep", "add", "M-1", "--after", "A-2"],
    ] {
        let (out, code) = aplan(
            root,
            &[
                args[0], args[1], args[2], args[3], args[4], "--root", "aplan",
            ],
        );
        assert_ne!(code, 0, "{out}");
        assert!(out.contains("is a cut"), "{out}");
        // Named, because a refusal whose remedy is "edit YAML" is this task's
        // own complaint one command over.
        assert!(out.contains("aplan mv"), "{out}");
    }
    assert_eq!(before, cut("M-1"), "a cut's membership was rewritten");

    // Sequencing one cut after another is the same field and is the supported
    // way to order cuts (`A1-073`), so it must survive the refusal above. It is
    // also what `aplan mv` now sends a caller here to do.
    let (out, code) = aplan(root, &["mv", "M-1", "--cut", "M-0", "--root", "aplan"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("aplan dep add M-1 --after"), "{out}");

    let (out, code) = aplan(
        root,
        &["dep", "rm", "M-1", "--after", "M-0", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    let (out, code) = aplan(
        root,
        &["dep", "add", "M-1", "--after", "M-0", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");

    // And a member no task answers to can still be removed, because that is a
    // repair of `APLAN003` rather than a membership edit.
    std::fs::write(
        root.join("aplan/tasks/M-0.md"),
        cut("M-0").replace(
            "tags:\n- milestone\n",
            "tags:\n- milestone\nblocked_by: [GONE]\n",
        ),
    )
    .expect("write");
    let (check, code) = aplan(root, &["check", "--root", "aplan"]);
    assert_ne!(code, 0, "{check}");
    let (out, code) = aplan(
        root,
        &["dep", "rm", "M-0", "--after", "GONE", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    let (check, code) = aplan(root, &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{check}");
}

/// The written file is the author's, not the tool's idea of one: line endings
/// survive, an inline list becomes the block form without losing an entry, and
/// nothing outside the frontmatter is touched.
#[test]
fn the_rest_of_the_file_is_left_alone() {
    let dir = repo();
    let root = dir.path();
    // CRLF throughout, an inline list carrying quotes, and a body line that
    // looks exactly like the key being rewritten.
    let body = "---\nid: A-5\ntitle: Five\nphase: 1\nstatus: todo\nowner: kaminsod\n\
                effort_days: 1\nblocked_by: [\"A-1\", 'A-2']\n---\n\n## Why\n\n\
                ```yaml\nblocked_by: [A-9]\n```\n";
    std::fs::write(root.join("aplan/tasks/A-5.md"), body.replace('\n', "\r\n")).expect("write");

    let (out, code) = aplan(
        root,
        &["dep", "rm", "A-5", "--after", "A-1", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");

    let text = std::fs::read_to_string(root.join("aplan/tasks/A-5.md")).expect("read");
    assert_eq!(
        text.matches('\n').count(),
        text.matches("\r\n").count(),
        "a CRLF file gained a bare LF:\n{text:?}"
    );
    // Inline in, inline out: `A1-179` stopped `dep` reformatting a list `mv`
    // would have left alone. The quotes go, because the ids are re-emitted from
    // the parsed list rather than copied as written -- and an id that *needs*
    // quoting is refused by the round-trip guard rather than written bare.
    assert!(text.contains("blocked_by: [A-2]\r\n"), "{text:?}");
    assert!(
        !text.contains("A-1"),
        "the remaining entry was lost or kept wrong:\n{text:?}"
    );
    assert!(
        text.contains("```yaml\r\nblocked_by: [A-9]\r\n```"),
        "{text:?}"
    );

    let (check, code) = aplan(root, &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{check}");
}

/// The edge survives a round trip through the loader, which is what makes it a
/// dependency rather than a line in a file: `ready` is the read side that acts
/// on it.
#[test]
fn the_edge_changes_what_ready_offers() {
    let dir = repo();
    let (ready, _) = aplan(dir.path(), &["ready", "--root", "aplan"]);
    assert!(ready.contains("A-2"), "{ready}");

    aplan(
        dir.path(),
        &["dep", "add", "A-2", "--after", "A-1", "--root", "aplan"],
    );
    let (ready, _) = aplan(dir.path(), &["ready", "--root", "aplan"]);
    assert!(
        !ready.contains("A-2"),
        "a blocked task was still offered:\n{ready}"
    );
    assert!(ready.contains("A-1"), "{ready}");
}

// --- the discretionary order (`A1-150`, ADR-0025) ---------------------------

/// A preference is written to `prefer_after`, never to `blocked_by`, and it
/// changes the offer without changing what blocks anything.
/// Proves the named behaviour `ready.honours_a_declared_preference` (`A1-211`).
#[test]
fn a_preference_orders_the_offer_and_blocks_nothing() {
    let dir = repo();
    let (out, code) = aplan(
        dir.path(),
        &["dep", "add", "A-1", "--after", "A-2", "--prefer"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("prefers to follow"), "{out}");

    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
    assert!(written.contains("prefer_after:"), "{written}");
    assert!(
        !written.contains("blocked_by:"),
        "a preference was written as a dependency: {written}"
    );

    // `why` answers about blocking, and a preference must not appear in it.
    let (why, _) = aplan(dir.path(), &["why", "A-1"]);
    assert!(
        !why.contains("A-2"),
        "a preference showed up in the answer about what blocks a task: {why}"
    );

    // And the work is still offered, after the task it prefers to follow.
    let (ready, code) = aplan(dir.path(), &["ready"]);
    assert_eq!(code, 0, "{ready}");
    let a1 = ready.find("A-1").expect("A-1 is still offered");
    let a2 = ready.find("A-2").expect("A-2 is offered");
    assert!(a2 < a1, "the preference did not order the offer:\n{ready}");

    let (explain, _) = aplan(dir.path(), &["ready", "--explain"]);
    assert!(
        explain.contains("prefers to follow A-2"),
        "the reason is not explained, so an order looks arbitrary:\n{explain}"
    );
}

/// The refusals: a preference nobody could act on is `APLAN033`, and the
/// mutation is refused before anything is written, like every other write.
#[test]
fn a_preference_that_cannot_be_honoured_is_refused_before_writing() {
    let dir = repo();
    let (out, code) = aplan(
        dir.path(),
        &["dep", "add", "A-1", "--after", "GHOST", "--prefer"],
    );
    assert_ne!(
        code, 0,
        "a preference on an unknown task was written:\n{out}"
    );
    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
    assert!(!written.contains("prefer_after"), "{written}");

    // A pair that is already a dependency: the plan would say two things.
    let (out, code) = aplan(dir.path(), &["dep", "add", "A-2", "--after", "A-1"]);
    assert_eq!(code, 0, "{out}");
    let (out, code) = aplan(
        dir.path(),
        &["dep", "add", "A-2", "--after", "A-1", "--prefer"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN033"), "{out}");
}

/// Removing one, and the no-op that says so, on the same footing as a
/// dependency edge.
#[test]
fn a_preference_is_removed_by_the_same_command_that_wrote_it() {
    let dir = repo();
    aplan(
        dir.path(),
        &["dep", "add", "A-1", "--after", "A-2", "--prefer"],
    );
    let (out, code) = aplan(
        dir.path(),
        &["dep", "rm", "A-1", "--after", "A-2", "--prefer"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("no longer"), "{out}");

    let (out, code) = aplan(
        dir.path(),
        &["dep", "rm", "A-1", "--after", "A-2", "--prefer"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("does not prefer to follow"),
        "the no-op must read as a sentence: {out}"
    );
}
