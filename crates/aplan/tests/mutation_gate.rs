//! Every mutating command is judged by the rules before it writes (`A1-113`).
//!
//! The rule engine had four call sites outside `aplan-core`'s own tests, and all
//! four were read commands. Each mutation hand-rolled a partial pre-check
//! instead, and the fourth pilot's complaint about `mv` was never that the plan
//! stayed broken, it was that the command which broke it said nothing
//! (pre-public adoption evidence).
//!
//! These run the commands, because that is the only place the wiring exists: a
//! library test of the delta would pass on a binary that never calls it.

use std::path::Path;
use std::process::Command;

mod common;
use common::aplan;

fn write_task(root: &Path, id: &str, title: &str, extra: &str) {
    std::fs::write(
        root.join(format!("aplan/tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: todo\nowner: kaminsod\n{extra}\n---\n\n\
             ## Why\n\nSomething.\n"
        ),
    )
    .expect("task");
}

/// Two cuts, one task each.
fn repo(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::write(root.join(".aplan.yml"), config).expect("config");
    write_task(
        root,
        "M-0",
        "The first cut",
        "tags: [milestone]\nblocked_by: [A-1]",
    );
    write_task(
        root,
        "M-1",
        "The second cut",
        "tags: [milestone]\nblocked_by: [A-2]",
    );
    write_task(root, "A-1", "Work in cut zero", "effort_days: 1");
    write_task(root, "A-2", "Work in cut one", "effort_days: 1");
    dir
}

/// **Emptying a cut leaves a plan that still loads** (`A1-363`).
///
/// It did not. The removal rewrote the list in place and kept the
/// `blocked_by:` key, so the last member taken out left a key with no value:
/// YAML `null`, where the schema wants an array, and the file stopped loading.
/// `aplan mv` could write a plan `aplan check` could not read.
///
/// Both list forms, because only one was broken -- the flow branch rewrote to
/// `blocked_by: []`, a valid empty array, and the block branch is the form `add`
/// and the importer write.
#[test]
fn emptying_a_cut_leaves_a_plan_that_loads_in_either_list_form() {
    for members in ["blocked_by: [A-1]", "blocked_by:\n  - A-1"] {
        let dir = repo("root: aplan\n");
        let root = dir.path();
        // Re-write the first cut with the list form under test.
        write_task(
            root,
            "M-0",
            "The first cut",
            &format!("tags: [milestone]\n{members}"),
        );

        let out = common::run(root, &["mv", "A-1", "--cut", "M-1"]);
        assert_eq!(out.code, 0, "the move was refused: {}", out.text());

        // The one thing that was wrong: the plan the move left behind.
        let after = common::run(root, &["check"]);
        assert_eq!(
            after.code,
            0,
            "`mv` wrote a plan that does not load ({members:?}): {}",
            after.text()
        );
        assert!(
            !after.text().contains("APLAN001"),
            "the emptied cut no longer parses ({members:?}): {}",
            after.text()
        );
        // And the cut that lost its work is reported, at the severity `A1-134`
        // chose for it rather than as a refusal.
        assert!(
            after.text().contains("APLAN021"),
            "an emptied cut should still be reported ({members:?}): {}",
            after.text()
        );
    }
}

/// **A member can leave a settled cut; it cannot be added to one** (`A1-362`).
///
/// One `resolve` carried "never extend a settled cut" for three callers, and
/// `plan_removal` is not one that extends. So moving finished work *out* of a
/// finished cut was refused, and told the caller that "adding an unfinished task
/// to a settled cut would break it" about the cut it was leaving.
#[test]
fn a_settled_cut_can_be_left_but_not_extended() {
    let dir = repo("root: aplan\n");
    let root = dir.path();
    // `M-0` is finished, and so is the work in it.
    write_task(
        root,
        "A-1",
        "Finished work",
        "effort_days: 1\npaths:\n- \"*.yml\"\nevidence:\n  - kind: file\n\
         \x20   ref: .aplan.yml",
    );
    let a1 = root.join("aplan/tasks/A-1.md");
    let body = std::fs::read_to_string(&a1).expect("A-1");
    std::fs::write(&a1, body.replace("status: todo", "status: done")).expect("A-1 done");
    let m0 = root.join("aplan/tasks/M-0.md");
    let body = std::fs::read_to_string(&m0).expect("M-0");
    std::fs::write(
        &m0,
        body.replace(
            "status: todo",
            "status: done\nevidence:\n  - kind: file\n    ref: .aplan.yml",
        ),
    )
    .expect("M-0 done");

    // Leaving is allowed, and what it leaves is the gate's to report.
    let out = common::run(root, &["mv", "A-1", "--cut", "M-1"]);
    assert_eq!(
        out.code,
        0,
        "a member could not leave a settled cut: {}",
        out.text()
    );

    // Extending one is still refused, which is what the guard is for.
    let out = common::run(root, &["mv", "A-2", "--cut", "M-0"]);
    assert_ne!(out.code, 0, "a settled cut was extended: {}", out.text());
    assert!(
        out.text().contains("settled cut") && out.text().contains("M-0"),
        "the refusal should name the cut being added to: {}",
        out.text()
    );
}

/// The first measured case, and the one that has now closed (`A1-134`).
///
/// `APLAN021` used to ask whether a milestone's `blocked_by` was *empty*, and a
/// sequenced cut keeps its predecessor there, so a cut emptied of work was the
/// one consequence this gate could not report and `mv` hand-rolled a refusal for
/// it. The rule counts live work members now, so the gate reports it like every
/// other finding a mutation introduces -- which is what this file is about, and
/// the reason `A1-113` was worth building.
#[test]
fn mv_emptying_a_cut_of_work_is_reported_by_the_gate() {
    let dir = repo("");
    // Sequenced, which is the shape that used to defeat `APLAN021`.
    write_task(
        dir.path(),
        "M-1",
        "The second cut",
        "tags: [milestone]\nblocked_by: [M-0, A-2]",
    );

    let (out, code) = aplan(
        dir.path(),
        &["mv", "A-2", "--cut", "M-0", "--root", "aplan"],
    );
    assert_eq!(code, 0, "a warning must not refuse the move:\n{out}");
    assert!(
        out.contains("APLAN021") && out.contains("no work in it"),
        "the gate must name what the mutation introduces:\n{out}"
    );

    // And under a policy that fails on warnings, the same move is refused and
    // writes nothing -- by the repository's policy rather than by `mv`.
    let dir = repo("deny_warnings: true\n");
    write_task(
        dir.path(),
        "M-1",
        "The second cut",
        "tags: [milestone]\nblocked_by: [M-0, A-2]",
    );
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-1.md")).expect("read");
    let (out, code) = aplan(
        dir.path(),
        &["mv", "A-2", "--cut", "M-0", "--root", "aplan"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN021"), "{out}");
    assert_eq!(
        before,
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-1.md")).expect("read"),
        "a refusal wrote to the source cut"
    );
}

/// A new *warning* is reported and the mutation goes ahead: `add` without an
/// estimate is `APLAN009`, and refusing it would make the scaffold refuse the
/// thing it exists to scaffold.
#[test]
fn a_new_warning_is_reported_and_the_mutation_proceeds() {
    let dir = repo("");
    let (out, code) = aplan(dir.path(), &["add", "Unestimated", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("APLAN009"), "{out}");
    assert!(out.contains("fix: Add an `effort_days` estimate"), "{out}");
}

/// And `deny_warnings` escalates that to a refusal, which is the mechanism this
/// repository already has rather than a second one invented here.
#[test]
fn deny_warnings_turns_that_report_into_a_refusal() {
    let dir = repo("deny_warnings: true\n");
    let before = std::fs::read_dir(dir.path().join("aplan/tasks"))
        .expect("tasks")
        .count();

    let (out, code) = aplan(dir.path(), &["add", "Unestimated", "--root", "aplan"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN009"), "{out}");
    assert_eq!(
        before,
        std::fs::read_dir(dir.path().join("aplan/tasks"))
            .expect("tasks")
            .count(),
        "a refused add left a file behind"
    );
}

/// The second measured case: `drop --cut` and the cut sequenced before it.
///
/// `A1-084` fixed the bug where dropping a cut took its predecessor with it, and
/// this is the regression test for that living beside the mechanism that would
/// now catch a relapse: the predecessor stays live, and the plan stays clean.
#[test]
fn dropping_a_cut_leaves_the_one_before_it_alone() {
    let dir = repo("");
    // Sequenced: M-1 follows M-0.
    write_task(
        dir.path(),
        "M-1",
        "The second cut",
        "tags: [milestone]\nblocked_by: [M-0, A-2]",
    );

    let (out, code) = aplan(dir.path(), &["drop", "M-1", "--cut", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");

    let predecessor = std::fs::read_to_string(dir.path().join("aplan/tasks/M-0.md")).expect("read");
    assert!(
        predecessor.contains("status: todo"),
        "the predecessor was cancelled too:\n{predecessor}"
    );
    let (check, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{check}");
}

/// A mutation on an already-dirty plan is judged on what *it* adds. Reporting the
/// whole report would make every mutation on an imperfect plan refuse, and would
/// be right about the rule and wrong about the cause.
#[test]
fn a_mutation_on_a_dirty_plan_is_not_blamed_for_what_was_already_wrong() {
    let dir = repo("");
    // A fault this mutation has nothing to do with.
    write_task(
        dir.path(),
        "A-2",
        "Work in cut one",
        "effort_days: 1\nblocked_by: [GHOST]",
    );
    let (check, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_ne!(code, 0, "the fixture must be dirty: {check}");
    assert!(check.contains("APLAN003"), "{check}");

    // Moving an unrelated task goes ahead, and says nothing about `GHOST`.
    write_task(
        dir.path(),
        "M-0",
        "The first cut",
        "tags: [milestone]\nblocked_by: [A-1, A-3]",
    );
    write_task(
        dir.path(),
        "A-3",
        "A second task in cut zero",
        "effort_days: 1",
    );
    let (out, code) = aplan(
        dir.path(),
        &["mv", "A-1", "--cut", "M-1", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("APLAN003"),
        "the mutation was blamed for it:\n{out}"
    );
}

/// Several commands, each producing a post-state fault of a different rule.
///
/// The first version of this test was named for every command and ran `add`
/// twice, with a comment describing a `close` case it never performed: a test
/// passing for the wrong reason under a name that overstated it, which is what
/// this file is otherwise about.
/// Proves the named behaviour `mutation.refused_when_check_would_reject` (`A1-211`).
#[test]
fn commands_other_than_add_refuse_a_post_state_check_would_reject() {
    // `drop`, leaving a live task blocked by a cancelled one: `APLAN020`.
    let dir = repo("");
    write_task(
        dir.path(),
        "A-2",
        "Work in cut one",
        "effort_days: 1\nblocked_by: [A-1]",
    );
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
    let (out, code) = aplan(dir.path(), &["drop", "A-1", "--root", "aplan"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN020 A-2:"), "{out}");
    assert!(out.contains("fix: Either drop `A-2` too"), "{out}");
    assert_eq!(
        before,
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read"),
        "a refused drop wrote anyway"
    );

    // `close`, on a task whose blocker is unfinished: `APLAN007`. This was a
    // bespoke check whose own comment said it existed to prevent the `APLAN007`
    // the rules now catch, so it is gone and this is what replaced it.
    let dir = repo("");
    write_task(
        dir.path(),
        "A-2",
        "Work in cut one",
        "effort_days: 1\nblocked_by: [A-1]",
    );
    git_init(dir.path());
    let (out, code) = aplan(dir.path(), &["close", "A-2", "--root", "aplan"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN007 A-2:"), "{out}");
    assert!(out.contains("fix: Either close `A-1`"), "{out}");
    let task = std::fs::read_to_string(dir.path().join("aplan/tasks/A-2.md")).expect("read");
    assert!(task.contains("status: todo"), "{task}");

    // `supersede`, whose successor would be unplaced: `APLAN022`.
    let dir = repo("require_cut_membership: true\n");
    write_task(
        dir.path(),
        "A-1",
        "Work in cut zero",
        "effort_days: 1\nevidence:\n  - kind: note\n    ref: r\n    captured: 2026-08-15",
    );
    let path = dir.path().join("aplan/tasks/A-1.md");
    let done = std::fs::read_to_string(&path)
        .expect("read")
        .replace("status: todo", "status: done");
    std::fs::write(&path, done).expect("write");
    let before = std::fs::read_dir(dir.path().join("aplan/tasks"))
        .expect("tasks")
        .count();

    let (out, code) = aplan(
        dir.path(),
        &[
            "supersede",
            "A-1",
            "The work that now needs doing",
            "--root",
            "aplan",
        ],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN022"), "{out}");
    assert_eq!(
        before,
        std::fs::read_dir(dir.path().join("aplan/tasks"))
            .expect("tasks")
            .count(),
        "a refused supersede filed the successor anyway"
    );
}

/// A repository, for the commands that read `HEAD`.
fn git_init(root: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.email=a@b.c",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ],
    ] {
        let out = Command::new("git")
            .args(&args)
            .current_dir(root)
            .env(
                "GIT_CEILING_DIRECTORIES",
                aplan_core::fixture::fixture_root(),
            )
            .output()
            .expect("git");
        // This loop is the exact shape `A1-257` is about: `init`, then `add`,
        // then `commit`, with nothing between them noticing that the first
        // failed. Unchecked, the second and third find the checkout above.
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// The post-state `mv` is judged on is the plan it actually writes.
///
/// Every other command builds its post-state by parsing the frontmatter it
/// produced, so a command whose write is wrong is judged on what it would really
/// write. `mv` cannot: `cut::place` writes as it goes, so the check runs against
/// a model of the move rather than its output. That model is only trustworthy if
/// it agrees with the result, and nothing said it did.
#[test]
fn the_move_that_is_judged_is_the_move_that_happens() {
    let dir = repo("");
    // Two work items in the source, so the move is legal and completes.
    write_task(
        dir.path(),
        "M-0",
        "The first cut",
        "tags: [milestone]\nblocked_by: [A-1, A-3]",
    );
    write_task(
        dir.path(),
        "A-3",
        "A second task in cut zero",
        "effort_days: 1",
    );

    let (out, code) = aplan(
        dir.path(),
        &["mv", "A-1", "--cut", "M-1", "--root", "aplan"],
    );
    assert_eq!(code, 0, "{out}");

    // What the simulation predicted: `A-1` appended to the destination, removed
    // from the source, and every other edge untouched.
    let source = std::fs::read_to_string(dir.path().join("aplan/tasks/M-0.md")).expect("read");
    let dest = std::fs::read_to_string(dir.path().join("aplan/tasks/M-1.md")).expect("read");
    assert!(source.contains("blocked_by: [A-3]"), "{source}");
    assert!(dest.contains("A-1"), "{dest}");
    assert!(
        dest.contains("A-2"),
        "the destination lost an edge:\n{dest}"
    );

    // And the plan the rules would now see is clean, which is what the check
    // predicted when it let the move through.
    let (check, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(
        code, 0,
        "the judged post-state was not the written one:\n{check}"
    );
    assert!(!check.contains("APLAN0"), "{check}");
}

/// A mutation cannot know which file it would edit when two tasks declare one
/// id, so it refuses (`A1-135`).
///
/// Measured while auditing `A1-124`: `dep add A-2` edited one file and `drop
/// A-2` edited the other, neither said anything, and the plan afterwards held a
/// `dropped` task and a live one sharing an id. Not something the post-state
/// check can catch -- `APLAN002` is in the *pre*-state too, so it is not a
/// finding the mutation added.
#[test]
fn every_id_addressed_mutation_refuses_a_duplicate_id() {
    let dir = repo("");
    let root = dir.path();
    write_task(root, "A-2", "The duplicate", "effort_days: 1");
    std::fs::write(
        root.join("aplan/tasks/dup.md"),
        "---\nid: A-2\ntitle: Declared twice\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\n\
         ## Why\n\nx\n",
    )
    .expect("dup");

    let before: Vec<String> = ["A-2", "dup"]
        .iter()
        .map(|f| std::fs::read_to_string(root.join(format!("aplan/tasks/{f}.md"))).expect("read"))
        .collect();

    for args in [
        vec!["dep", "add", "A-2", "--after", "A-1", "--root", "aplan"],
        vec!["drop", "A-2", "--root", "aplan"],
        vec!["mv", "A-2", "--cut", "M-0", "--root", "aplan"],
        vec!["close", "A-2", "--evidence", "note:x", "--root", "aplan"],
        vec![
            "amend",
            "A-2",
            "--add-evidence",
            "note:x",
            "--root",
            "aplan",
        ],
        vec!["supersede", "A-2", "A successor", "--root", "aplan"],
    ] {
        let (out, code) = aplan(root, &args);
        assert_ne!(code, 0, "`{}` guessed:\n{out}", args.join(" "));
        assert!(
            out.contains("declared by 2 files") && out.contains("dup.md"),
            "`{}` must name both files:\n{out}",
            args.join(" ")
        );
    }

    // Neither file changed, which is the property a refusal is for.
    let after: Vec<String> = ["A-2", "dup"]
        .iter()
        .map(|f| std::fs::read_to_string(root.join(format!("aplan/tasks/{f}.md"))).expect("read"))
        .collect();
    assert_eq!(before, after, "a refusal wrote to one of the two files");
}
