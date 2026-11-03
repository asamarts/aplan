//! `aplan add`'s write path, against a real temp tree (`A1-029`).
//!
//! Separate from the trycmd corpus on purpose. A trycmd case runs inside its own
//! `.in` fixture, so a command that *writes* leaves the file behind and the next
//! run sees a different plan; `add.trycmd` therefore pins only the guards. What
//! has to be checked here is the thing those cannot reach: that the file lands,
//! that the plan it produces is one `check` accepts, and that a second `add`
//! continues the sequence rather than repeating it.

use crate::common;
use common::aplan;

/// A plan with one task and one milestone, the shape every real plan has.
fn plan_tree() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("create tasks dir");
    std::fs::write(
        tasks.join("A1-004.md"),
        "---\nid: A1-004\ntitle: An existing task\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("write task");
    std::fs::write(
        tasks.join("M-v0.1.md"),
        "---\nid: M-v0.1\ntitle: First cut\nphase: 1\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [A1-004]\n---\n",
    )
    .expect("write milestone");
    dir
}

#[test]
fn a_scaffolded_task_is_one_check_accepts() {
    let dir = plan_tree();
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Wire the sequencer",
            "--effort",
            "2",
            "--paths",
            "src/seq/**",
        ],
    );
    assert_eq!(code, 0, "{out}");

    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-005.md"))
        .expect("the file the command said it wrote");
    assert!(written.contains("id: A1-005"), "{written}");
    assert!(written.contains("title: Wire the sequencer"), "{written}");
    assert!(written.contains("status: todo"), "{written}");
    // It must not invent the things only a person can decide.
    assert!(!written.contains("evidence"), "{written}");
    assert!(written.contains("## Why"), "the body skeleton: {written}");

    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
    assert!(check.contains("0 warning(s)"), "{check}");
}

#[test]
fn a_second_add_continues_the_sequence() {
    // The failure this guards against is an `add` that reads the plan from disk
    // once: the second call would derive A1-005 again and be refused, or worse,
    // overwrite.
    let dir = plan_tree();
    for (title, expected) in [("First", "A1-005"), ("Second", "A1-006")] {
        let (out, code) = aplan(
            dir.path(),
            &["add", title, "--effort", "1", "--paths", "src/**"],
        );
        assert_eq!(code, 0, "{out}");
        assert!(out.contains(expected), "expected {expected}: {out}");
        assert!(
            dir.path()
                .join(format!("aplan/tasks/{expected}.md"))
                .exists(),
            "{expected} was announced but not written"
        );
    }
}

/// Proves `add.accepts_creation_time_model_fields` on the single-task surface.
#[test]
fn single_add_authors_live_state_and_every_creation_relationship() {
    let dir = plan_tree();
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Park the next release question",
            "--id",
            "A1-005",
            "--status",
            "blocked",
            "--tag",
            "milestone",
            "--blocked-by",
            "M-v0.1",
            "--prefer-after",
            "A1-004",
            "--ratified-by",
            "A1-004",
            "--contains",
            "M-v0.1",
            "--revisit-at",
            "2030-01-02T03:04:05Z",
            "--version-impact",
            "feature",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let task = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-005.md")).expect("task");
    for expected in [
        "status: blocked",
        "blocked_by:\n- M-v0.1",
        "prefer_after:\n- A1-004",
        "ratified_by:\n- A1-004",
        "contains:\n- M-v0.1",
        "revisit_at: 2030-01-02T03:04:05Z",
        "version_impact: feature",
    ] {
        assert!(task.contains(expected), "missing {expected:?}:\n{task}");
    }

    for status in ["done", "dropped"] {
        let (out, code) = aplan(
            dir.path(),
            &["add", "Bad terminal claim", "--status", status],
        );
        assert_eq!(code, 2, "{status}: {out}");
        assert!(
            out.contains("add --from") || out.contains("guarded `drop`"),
            "{out}"
        );
        assert!(!dir.path().join("aplan/tasks/A1-006.md").exists());
    }
}

/// Proves `authoring.refuses_visually_empty_creation_values` on ordinary add.
#[test]
fn creation_refuses_every_visually_empty_model_value() {
    let invisible = "\u{200b}";
    let cases = [
        vec!["add", invisible],
        vec!["add", "task", "--id", invisible],
        vec!["add", "task", "--tag", invisible],
        vec!["add", "task", "--paths", invisible],
        vec!["add", "task", "--traces", invisible],
        vec!["add", "task", "--blocked-by", invisible],
        vec![
            "add",
            "task",
            "--version-impact",
            "none",
            "--version-impact-reason",
            invisible,
        ],
    ];
    for args in cases {
        let dir = plan_tree();
        let (out, code) = aplan(dir.path(), &args);
        assert_eq!(code, 2, "{args:?}: {out}");
        assert!(
            !dir.path().join("aplan/tasks/A1-005.md").exists(),
            "{args:?} wrote a task"
        );
    }
}

#[test]
fn creation_state_and_version_neighbours_are_refused_before_single_add_writes() {
    for (args, expected) in [
        (
            vec!["add", "task", "--status", "unknown"],
            "is not a status",
        ),
        (
            vec![
                "add",
                "task",
                "--status",
                "todo",
                "--revisit-at",
                "2030-01-02T03:04:05Z",
            ],
            "initially `blocked`",
        ),
        (
            vec![
                "add",
                "task",
                "--status",
                "blocked",
                "--revisit-at",
                "next Thursday",
            ],
            "RFC 3339",
        ),
        (
            vec!["add", "task", "--version-impact", "unknown"],
            "is not an impact",
        ),
    ] {
        let dir = plan_tree();
        let (out, code) = aplan(dir.path(), &args);
        assert_eq!(code, 2, "{args:?}: {out}");
        assert!(out.contains(expected), "{args:?}: {out}");
        assert!(!dir.path().join("aplan/tasks/A1-005.md").exists());
    }

    let dir = plan_tree();
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Breaking contract",
            "--effort",
            "1",
            "--paths",
            "src/contract.rs",
            "--version-impact",
            "breaking",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let task = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-005.md")).expect("task");
    assert!(task.contains("version_impact: breaking"), "{task}");
}

/// Proof for `authoring.task_ids_preserve_cut_labels`.
#[test]
fn a_new_task_id_cannot_shadow_a_cut_label() {
    let dir = plan_tree();
    let cut = dir.path().join("aplan/tasks/M-v0.1.md");
    let text = std::fs::read_to_string(&cut).expect("cut");
    std::fs::write(
        &cut,
        text.replace("tags: [milestone]", "tags: [milestone, s0]"),
    )
    .expect("label cut");

    let (out, code) = aplan(
        dir.path(),
        &["add", "Collision", "--id", "s0", "--effort", "1"],
    );
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("label of cut(s) M-v0.1"), "{out}");
    assert!(out.contains("make that label unusable"), "{out}");
    assert!(!dir.path().join("aplan/tasks/s0.md").exists());

    // The refusal preserved the vocabulary: the same label still places an
    // ordinary task in the intended cut.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add", "Later", "--id", "A1-005", "--cut", "s0", "--effort", "1",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let cut = std::fs::read_to_string(cut).expect("updated cut");
    assert!(cut.contains("A1-005"), "{cut}");
}

#[test]
fn it_refuses_input_that_would_produce_a_file_check_rejects() {
    // The first version wrote all of these. `--effort=-1` produced a file that
    // failed APLAN001 immediately, and `inf`/`nan` were worse: they loaded
    // cleanly and poisoned every rollup that adds `effort_days` up.
    let dir = plan_tree();
    for args in [
        vec!["add", "Bad", "--effort=-1"],
        vec!["add", "Bad", "--effort", "inf"],
        vec!["add", "Bad", "--effort", "nan"],
        vec!["add", ""],
        vec!["add", "   "],
    ] {
        let (out, code) = aplan(dir.path(), &args);
        assert_eq!(code, 2, "{args:?} should be refused: {out}");
        assert!(
            !dir.path().join("aplan/tasks/A1-005.md").exists(),
            "{args:?} was refused but wrote the file anyway"
        );
    }
    // A refusal that also rejects the ordinary case would be worse than none.
    let (out, code) = aplan(
        dir.path(),
        &["add", "Fine", "--effort", "0", "--paths", "s/**"],
    );
    assert_eq!(code, 0, "zero is a real estimate: {out}");
}

/// A milestone is a checkpoint, so fields that describe work are invalid add
/// input rather than a warning the command writes into the plan (`A1-369`).
///
/// Proves the named behaviour `add.refuses_milestone_work_fields` on the
/// single-task surface. The zero estimate is the adopter's exact case: with
/// `require_effort` enabled it looks like the least committal value, but a
/// milestone is exempt from that policy and `APLAN021` forbids the field.
#[test]
fn a_milestone_with_work_fields_is_refused_before_plain_add_writes() {
    for (flag, value, message, policy) in [
        ("--effort", "0", "carries `effort_days`", ""),
        (
            "--paths",
            "src/**",
            "declares `paths`",
            "advisory: [APLAN021]\n",
        ),
    ] {
        let dir = plan_tree();
        std::fs::remove_file(dir.path().join("aplan/tasks/M-v0.1.md")).expect("remove cut");
        std::fs::write(
            dir.path().join(".aplan.yml"),
            format!("root: aplan\nrequire_effort: true\n{policy}"),
        )
        .expect("config");

        let (out, code) = aplan(
            dir.path(),
            &[
                "add",
                "A bad cut",
                "--id",
                "M-bad",
                "--tag",
                "milestone",
                "--blocked-by",
                "A1-004",
                flag,
                value,
                "--no-deny-warnings",
            ],
        );
        assert_eq!(code, 2, "{flag} must be refused: {out}");
        assert!(out.contains("APLAN021") && out.contains(message), "{out}");
        assert!(
            !out.contains("A cut is created with its work"),
            "a populated milestone was misdiagnosed as empty: {out}"
        );
        assert!(
            !dir.path().join("aplan/tasks/M-bad.md").exists(),
            "{flag} was refused but wrote the milestone"
        );

        let (check, code) = aplan(dir.path(), &["check"]);
        assert_eq!(code, 0, "the refusal must leave the plan clean: {check}");
        assert!(check.contains("0 warning(s)"), "{check}");

        let (out, code) = aplan(
            dir.path(),
            &[
                "add",
                "A clean cut",
                "--id",
                "M-good",
                "--tag",
                "milestone",
                "--blocked-by",
                "A1-004",
            ],
        );
        assert_eq!(
            code, 0,
            "a milestone without work fields stays valid: {out}"
        );
    }
}

/// A1-369 refuses work fields on the milestone this command is adding, not on
/// an existing malformed cut whose warning merely changes wording (`A1-406`).
/// Proves `add.milestone_work_refusal_is_new_input_only` on plain `add`.
#[test]
fn an_improving_plain_add_to_an_existing_malformed_milestone_is_not_hard_refused() {
    let dir = plan_tree();
    let cut_path = dir.path().join("aplan/tasks/M-v0.1.md");
    std::fs::write(
        &cut_path,
        "---\nid: M-v0.1\ntitle: First cut\nphase: 1\nstatus: todo\neffort_days: 1\n\
         tags: [milestone]\n---\n",
    )
    .expect("write malformed milestone");

    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "A new member",
            "--id",
            "A1-005",
            "--effort",
            "1",
            "--cut",
            "M-v0.1",
            "--no-deny-warnings",
        ],
    );
    assert_eq!(
        code, 0,
        "an improving add was mistaken for a new bad milestone: {out}"
    );
    assert!(
        out.contains("APLAN021") && out.contains("carries `effort_days`"),
        "the remaining pre-existing warning must still be reported: {out}"
    );
    assert!(
        !out.contains("has no members"),
        "the fixed clause survived: {out}"
    );
    assert!(
        dir.path().join("aplan/tasks/A1-005.md").exists(),
        "the successful add did not write its task"
    );
    let cut = std::fs::read_to_string(cut_path).expect("read cut");
    assert!(
        cut.contains("A1-005"),
        "the cut did not gain its member: {cut}"
    );

    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "a warning is lenient by default: {check}");
    assert!(
        check.contains("1 warning(s)") && !check.contains("has no members"),
        "the post-state did not strictly improve the warning: {check}"
    );
}

#[test]
fn a_dry_run_writes_nothing() {
    let dir = plan_tree();
    let (out, code) = aplan(dir.path(), &["add", "Ghost", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !dir.path().join("aplan/tasks/A1-005.md").exists(),
        "--dry-run wrote the file it only promised"
    );
}

/// The field that binds a task to its cause, set at creation (`A1-116`).
///
/// 67 of this plan's tasks carry `traces:`, `APLAN008` checks the anchors, and
/// `close` stamps a fingerprint per trace so `APLAN018` and `APLAN028` can see
/// drift -- so a trace added by hand afterwards is a drift detector that was off
/// in between. `add` could not write the field, and every findings-derived task
/// was created and then hand-edited, in a repository whose premise is that the
/// tool owns those files.
#[test]
fn a_task_can_be_created_with_the_trace_that_caused_it() {
    let dir = plan_tree();
    std::fs::write(
        dir.path().join("DOC.md"),
        "# A document\n\n## The cause\n\nWhy this task exists.\n",
    )
    .expect("doc");

    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "traced",
            "--traces",
            "DOC.md#the-cause",
            "--cut",
            "M-v0.1",
            "--effort",
            "1",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-005.md")).expect("read");
    assert!(written.contains("traces:\n- DOC.md#the-cause"), "{written}");

    // The point is not that the field is present but that the plan it produces is
    // one `check` accepts, `APLAN008` included.
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
}

/// A repeatable input given twice is one value, on every field `add` collects
/// (`A1-116`).
///
/// Nothing rejects a duplicate, and each one costs something quietly: a repeated
/// tag is what `APLAN026` had to be taught not to double-count, a repeated glob
/// is judged twice by `APLAN012` and `APLAN019`, and a repeated trace stamps one
/// fingerprint under two entries, so `APLAN028` compares a set against a list.
/// `drop` and `mv` already treat an id named twice as one.
#[test]
fn a_field_given_twice_is_written_once() {
    let dir = plan_tree();
    std::fs::write(
        dir.path().join("DOC.md"),
        "# A document\n\n## The cause\n\nWhy this task exists.\n",
    )
    .expect("doc");

    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "repeated",
            "--traces",
            "DOC.md#the-cause",
            "--traces",
            "DOC.md#the-cause",
            "--tag",
            "cli",
            "--tag",
            "cli",
            "--paths",
            "src/**",
            "--paths",
            "src/**",
            "--effort",
            "1",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-005.md")).expect("read");
    for (field, value) in [
        ("traces", "DOC.md#the-cause"),
        ("tags", "cli"),
        ("paths", "src/**"),
    ] {
        assert_eq!(
            written.matches(&format!("- {value}")).count(),
            1,
            "`{field}` carries the value twice:\n{written}"
        );
    }

    // And the order is the author's: the first mention wins its place.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add", "ordered", "--tag", "b", "--tag", "a", "--tag", "b", "--effort", "1",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-006.md")).expect("read");
    assert!(written.contains("tags:\n- b\n- a\n"), "{written}");
}

/// An anchor that does not resolve is refused at creation, and the refusal is
/// `APLAN008` itself rather than a copy of it (`A1-116`, `A1-113`).
#[test]
fn a_trace_that_does_not_resolve_is_refused_before_the_file_exists() {
    let cases = [
        ("DOC.md#nope", "matches no heading or table row"),
        ("NOPE.md#x", "does not exist"),
    ];
    for (trace, expected) in cases {
        let dir = plan_tree();
        std::fs::write(
            dir.path().join("DOC.md"),
            "# A document\n\n## The cause\n\nWhy this task exists.\n",
        )
        .expect("doc");

        let (out, code) = aplan(
            dir.path(),
            &["add", "traced", "--traces", trace, "--effort", "1"],
        );
        assert_ne!(code, 0, "`{trace}` was accepted:\n{out}");
        assert!(out.contains("APLAN008"), "{out}");
        assert!(out.contains(expected), "{out}");
        assert!(
            out.contains("fix: "),
            "the rule's own remedy comes with it: {out}"
        );
        assert!(
            !dir.path().join("aplan/tasks/A1-005.md").exists(),
            "the refusal wrote the file anyway"
        );
    }
}

/// The cut is a better answer than the highest phase in use, and `add` has it
/// (`A1-108`).
///
/// The fourth pilot's case, reproduced: adding to cut 1 of a plan whose highest
/// phase is 6 produced `R6-09`, `phase: 6`, a member of `M-s1`. Nothing rejects
/// that -- the phase and the release cut are different axes on purpose -- and it
/// still misleads: `summary` bills the effort to phase 6, and a plan whose ids
/// encode the cut gets an id that misstates it, on the first step the pilot's own
/// instructions document (pre-public adoption evidence item 22).
#[test]
fn a_cut_decides_the_phase_it_was_given_rather_than_the_highest_in_use() {
    let dir = plan_tree();
    let tasks = dir.path().join("aplan/tasks");
    // A later cut, so "the highest phase in use" and "the cut asked for" differ.
    std::fs::write(
        tasks.join("A6-001.md"),
        "---\nid: A6-001\ntitle: Later work\nphase: 6\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("write task");
    std::fs::write(
        tasks.join("M-v0.6.md"),
        "---\nid: M-v0.6\ntitle: Sixth cut\nphase: 6\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [A6-001]\n---\n",
    )
    .expect("write milestone");

    let (out, code) = aplan(
        dir.path(),
        &["add", "probe", "--cut", "M-v0.1", "--effort", "0.5"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("A1-005"),
        "the id follows the cut's phase: {out}"
    );
    let written = std::fs::read_to_string(tasks.join("A1-005.md")).expect("read");
    assert!(written.contains("phase: 1"), "{written}");

    // An explicit `--phase` still wins: an author who says so means it.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add", "explicit", "--cut", "M-v0.1", "--phase", "6", "--effort", "0.5",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("A6-002"), "{out}");

    // And with no cut, the highest phase in use is still the best guess there is.
    let (out, code) = aplan(dir.path(), &["add", "no cut", "--effort", "0.5"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("A6-003"), "{out}");

    // An explicit `--id` still wins for the *id*, and the phase still follows the
    // cut: they are two decisions, and only one of them was given.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add", "named", "--cut", "M-v0.1", "--id", "ZZ-9", "--effort", "0.5",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let written = std::fs::read_to_string(tasks.join("ZZ-9.md")).expect("read");
    assert!(written.contains("phase: 1"), "{written}");

    // And `--help` says so. A default that the help describes wrongly is how an
    // author learns the tool by being surprised by it.
    let (help, code) = aplan(dir.path(), &["add", "--help"]);
    assert_eq!(code, 0, "{help}");
    // Clap renders `--help` with each flag's description on its own line(s), so
    // read the entry rather than the single line the flag name sits on.
    let entry: String = help
        .lines()
        .skip_while(|l| !l.contains("--phase"))
        .take_while(|l| !l.trim().starts_with("--blocked-by"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        entry.contains("--cut"),
        "the help still says the old default: {entry}"
    );

    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
}

#[test]
fn placing_in_a_cut_edits_the_milestone_and_keeps_the_plan_valid() {
    let dir = plan_tree();
    // `require_cut_membership` makes the placement load-bearing: without it the
    // new task is an APLAN022 error, which is the case an adopter hits first.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "require_cut_membership: true\n",
    )
    .expect("write config");

    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "In the cut",
            "--cut",
            "M-v0.1",
            "--effort",
            "1",
            "--paths",
            "src/**",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("added A1-005 to `M-v0.1`"), "{out}");

    let milestone = std::fs::read_to_string(dir.path().join("aplan/tasks/M-v0.1.md")).unwrap();
    assert!(
        milestone.contains("blocked_by: [A1-004, A1-005]"),
        "the placement must keep the file's existing style: {milestone}"
    );
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "{check}");
}

#[test]
fn without_a_cut_the_command_says_the_plan_will_reject_the_task() {
    // The whole difference between a scaffold and a trap: this plan requires cut
    // membership, so an unplaced task fails `check`, and the author learns it
    // here rather than two commands later. It was a note until `A1-113` ran the
    // rules on the post-state, and `APLAN022` is an error, so it is a refusal
    // now: the same intent, enforced rather than announced.
    let dir = plan_tree();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "require_cut_membership: true\n",
    )
    .expect("write config");
    let (out, code) = aplan(
        dir.path(),
        &["add", "Unplaced", "--effort", "1", "--paths", "s/**"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("APLAN022"), "the rule is named: {out}");
    assert!(
        out.contains("or tag it `cross-cutting`"),
        "and the way out: {out}"
    );

    // Nothing was written, so the plan `check` sees is the one it saw before.
    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "a refused add left a task behind: {check}");

    // And the two ways past it both work.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add", "Placed", "--cut", "M-v0.1", "--effort", "1", "--paths", "s/**",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Everywhere",
            "--tag",
            "cross-cutting",
            "--effort",
            "1",
            "--paths",
            "s/**",
        ],
    );
    assert_eq!(code, 0, "{out}");
}

#[test]
fn the_first_task_of_an_empty_plan_is_refused_then_sets_the_convention() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("create dir");

    let (out, code) = aplan(dir.path(), &["add", "First ever"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("--id"),
        "the refusal must name the way out: {out}"
    );

    // The way out has to actually work, or the message is a lie.
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "First ever",
            "--id",
            "AL0-001",
            "--effort",
            "1",
            "--paths",
            "src/**",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let (out, code) = aplan(
        dir.path(),
        &[
            "add", "Second", "--phase", "0", "--effort", "1", "--paths", "s/**",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("AL0-002"),
        "the convention set by the first task must be the one continued: {out}"
    );
}

// --- `id_strategy: ulid` (`A1-047`) -----------------------------------------

/// A plan carrying a config, so the strategy is read from where an adopter sets
/// it rather than from a flag that does not exist.
fn plan_tree_with(config: &str) -> tempfile::TempDir {
    let dir = plan_tree();
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("write config");
    dir
}

/// The two properties an adopter is buying, end to end: an id nothing derived
/// from the plan, and a plan `check` still accepts.
#[test]
fn under_ulid_two_adds_mint_two_ids_neither_continuing_the_other() {
    let dir = plan_tree_with("id_strategy: ulid\n");
    let mut minted = Vec::new();
    for title in ["First", "Second"] {
        let (out, code) = aplan(
            dir.path(),
            &["add", title, "--effort", "1", "--paths", "src/**"],
        );
        assert_eq!(code, 0, "{out}");
        let id = out
            .split_whitespace()
            .nth(1)
            .expect("`wrote <id> ...`")
            .to_string();
        assert!(
            id.starts_with("A1-") && id.len() == "A1-".len() + 26,
            "the plan's stem, then a ULID: {id}"
        );
        minted.push(id);
    }
    assert_ne!(minted[0], minted[1], "the second add repeated the first");
    // Sequential would have written these two, which is what the strategy is
    // being asked to stop doing.
    assert!(
        !minted.iter().any(|id| id == "A1-005" || id == "A1-006"),
        "an id was derived from the plan's counter after all: {minted:?}"
    );

    let (check, code) = aplan(dir.path(), &["check"]);
    assert_eq!(
        code, 0,
        "a plan of ULIDs is not a plan `check` accepts:\n{check}"
    );
}

/// The guard the strategy does not replace: `APLAN002` still owns identity, and
/// a hand-passed `--id` that already exists is still refused.
#[test]
fn the_duplicate_guard_still_applies_under_ulid() {
    let dir = plan_tree_with("id_strategy: ulid\n");
    let (out, code) = aplan(
        dir.path(),
        &["add", "First", "--effort", "1", "--paths", "src/**"],
    );
    assert_eq!(code, 0, "{out}");
    let id = out.split_whitespace().nth(1).expect("`wrote <id>`");

    let (out, code) = aplan(
        dir.path(),
        &[
            "add", "Again", "--id", id, "--effort", "1", "--paths", "src/**",
        ],
    );
    assert_ne!(code, 0, "a duplicate id was accepted:\n{out}");
    assert!(out.contains(id), "the refusal names the id: {out}");
}

/// Adding a ULID does not cost the repository its convention: a plan holding
/// both still continues the sequence it declares, so a repository can turn the
/// key off again.
#[test]
fn a_plan_holding_ulids_still_continues_its_own_sequence_afterwards() {
    let dir = plan_tree_with("id_strategy: ulid\n");
    let (out, code) = aplan(
        dir.path(),
        &["add", "Minted", "--effort", "1", "--paths", "src/**"],
    );
    assert_eq!(code, 0, "{out}");

    std::fs::write(dir.path().join(".aplan.yml"), "id_strategy: sequential\n").expect("config");
    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Back to counting",
            "--effort",
            "1",
            "--paths",
            "src/**",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("A1-005"),
        "the ULID was read as a counter to continue: {out}"
    );
}

/// The default is not "whatever the file says": a plan with no `.aplan.yml` and
/// a plan that says `sequential` mint the same id, and it is the one every
/// existing plan already gets.
#[test]
fn the_default_is_sequential_and_an_absent_config_agrees_with_a_stated_one() {
    for config in [None, Some("id_strategy: sequential\n")] {
        let dir = match config {
            Some(c) => plan_tree_with(c),
            None => plan_tree(),
        };
        let (out, code) = aplan(
            dir.path(),
            &["add", "Ordinary", "--effort", "1", "--paths", "src/**"],
        );
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("A1-005"), "{config:?}: {out}");
    }
}

/// A misspelled value is refused, and the refusal says what the words are: a
/// config key whose bad value silently means "default" is a repository that
/// thinks it made a choice (`A1-133` learned this on `tag_families`).
#[test]
fn a_misspelled_strategy_is_refused_and_names_the_ones_that_exist() {
    let dir = plan_tree_with("id_strategy: uild\n");
    let (out, code) = aplan(
        dir.path(),
        &["add", "Ordinary", "--effort", "1", "--paths", "src/**"],
    );
    assert_ne!(
        code, 0,
        "a value this tool does not know was accepted:\n{out}"
    );
    assert!(
        out.contains("sequential") && out.contains("ulid"),
        "the refusal does not name the strategies that exist: {out}"
    );
    assert!(
        !dir.path().join("aplan/tasks/A1-005.md").exists(),
        "the refusal wrote the task anyway"
    );
}

/// The table a person reads still lines up when the ids are ULIDs (`A1-047`,
/// found by its audit).
///
/// `aplan tasks` padded its id column to a fixed twelve, which was right while
/// every id was `A1-047` and wrong the moment a repository turned this key on:
/// the long row's status, effort, cut and ready columns all shifted, while every
/// other row stayed put, so the table looked correct and was not.
#[test]
fn the_task_table_lines_up_under_ulid() {
    let dir = plan_tree_with("id_strategy: ulid\n");
    let (out, code) = aplan(
        dir.path(),
        &["add", "Minted", "--effort", "1", "--paths", "src/**"],
    );
    assert_eq!(code, 0, "{out}");

    let (table, code) = aplan(dir.path(), &["tasks"]);
    assert_eq!(code, 0, "{table}");
    let rows: Vec<&str> = table
        .lines()
        .filter(|l| l.contains("todo") && !l.starts_with("id "))
        .collect();
    assert!(rows.len() >= 3, "a task per row was expected:\n{table}");
    let status_at: Vec<usize> = rows
        .iter()
        .map(|l| l.find("todo").expect("the status column"))
        .collect();
    assert!(
        status_at.windows(2).all(|w| w[0] == w[1]),
        "the status column starts at {status_at:?}, so a row is out of line:\n{table}"
    );
}

/// `--owner` and `--out`, which no test invoked (`A1-208`).
///
/// `--owner` was invoked by no test anywhere in the suite, on any command, and
/// `--out` decides which directory a new task file lands in -- the one thing
/// `add` does that a reader would notice going wrong. Both are asserted on the
/// file that results, not on what the command said about it.
#[test]
fn owner_and_out_land_where_they_say_they_do() {
    let dir = plan_tree();
    let elsewhere = dir.path().join("aplan/elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");

    let (out, code) = aplan(
        dir.path(),
        &[
            "add",
            "Owned and placed",
            "--effort",
            "1",
            "--owner",
            "ada",
            "--out",
            elsewhere.to_str().expect("utf-8 path"),
        ],
    );
    assert_eq!(code, 0, "{out}");

    let written: Vec<std::path::PathBuf> = std::fs::read_dir(&elsewhere)
        .expect("reading the --out directory")
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(
        written.len(),
        1,
        "`--out` did not put the task where it was told: {written:?}"
    );
    let body = std::fs::read_to_string(&written[0]).expect("reading the new task");
    assert!(
        body.contains("owner: ada"),
        "`--owner` did not reach the file:\n{body}"
    );

    // And it is not in the default directory, which is the half that says the
    // flag *moved* the file rather than merely also writing one.
    let defaults: Vec<String> = std::fs::read_dir(dir.path().join("aplan/tasks"))
        .expect("reading the default directory")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !defaults.iter().any(|n| n.starts_with("A1-005")),
        "`--out` wrote to the default directory as well: {defaults:?}"
    );
}
