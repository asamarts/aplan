//! Placing imported tasks into a real cut, through the real loader.
//!
//! The unit tests in `cut` cover the text rewrite. These cover the part that
//! matters to a user: after `--cut`, does the plan `aplan check` sees actually
//! satisfy `APLAN022`, and do the guards refuse *before* touching anything?

use aplan_core::{Status, load_plan, validate};
use aplan_interop::{Options, ProposalSource, cut, emit, kep::Kep};

const KEP: &str = "title: A proposal\nkep-number: 3063\nstatus: implementable\n";

/// A plan directory with one milestone, plus one imported task.
fn plan_dir(milestone: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let tasks = dir.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(tasks.join("M-v0.6.md"), milestone).unwrap();

    let out = Kep.parse("kep.yaml", KEP, &Options::default()).unwrap();
    emit::write_task(&tasks, &out.tasks[0], "## Why\n\nimported\n").unwrap();
    dir
}

const OPEN_MILESTONE: &str = "---\nid: M-v0.6\ntitle: Surfaces\nphase: 1\nstatus: todo\ntags: [milestone, v0.6]\nblocked_by: [A1-003]\n---\n\n## Exit criteria\n\n- [ ] ship it\n";

/// A milestone's members must exist, so the cut needs its original member too.
fn with_member(dir: &std::path::Path) {
    let tasks = dir.join("tasks");
    std::fs::write(
        tasks.join("A1-003.md"),
        "---\nid: A1-003\ntitle: An existing member\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\n## Why\n\nx\n",
    )
    .unwrap();
}

#[test]
fn placing_satisfies_membership_totality() {
    // The whole point of `--cut`: without it an imported task belongs to no cut
    // and `APLAN022` fires wherever `require_cut_membership` is set.
    let dir = plan_dir(OPEN_MILESTONE);
    with_member(dir.path());

    let strict = validate::Options {
        require_cut_membership: true,
        ..validate::Options::default()
    };

    let before = validate(&load_plan(dir.path()), &strict);
    assert!(
        before
            .diagnostics
            .iter()
            .any(|d| d.code == aplan_core::code::MEMBERSHIP_TOTALITY),
        "the imported task should start in no cut: {:#?}",
        before.diagnostics
    );

    let plan = load_plan(dir.path());
    let placed = cut::place(&plan, "M-v0.6", &["KEP-3063".to_string()]).expect("places");
    assert_eq!(placed.added, vec!["KEP-3063".to_string()]);
    assert!(placed.path.ends_with("M-v0.6.md"));

    let after = validate(&load_plan(dir.path()), &strict);
    assert_eq!(
        after.errors(),
        0,
        "after placement the plan must be clean: {:#?}",
        after.diagnostics
    );
}

#[test]
fn the_edited_milestone_reloads_with_the_new_member() {
    let dir = plan_dir(OPEN_MILESTONE);
    with_member(dir.path());
    let plan = load_plan(dir.path());
    cut::place(&plan, "M-v0.6", &["KEP-3063".to_string()]).unwrap();

    let reloaded = load_plan(dir.path());
    let milestone = reloaded
        .tasks
        .iter()
        .find(|t| t.task.id == "M-v0.6")
        .expect("milestone");
    assert_eq!(
        milestone.task.blocked_by,
        vec!["A1-003".to_string(), "KEP-3063".to_string()],
        "the original member is kept and the new one appended"
    );
    assert!(
        reloaded.load_diagnostics.is_empty(),
        "the edit must leave a parseable file: {:#?}",
        reloaded.load_diagnostics
    );
}

#[test]
fn a_settled_cut_is_refused_and_left_untouched() {
    // The guard an audit added: appending an unfinished task to a `done`
    // milestone would make it violate APLAN007.
    let done = "---\nid: M-v0.6\ntitle: Surfaces\nphase: 1\nstatus: done\ntags: [milestone, v0.6]\nblocked_by: [A1-003]\nevidence:\n  - kind: commit\n    ref: deadbeef\n---\n";
    let dir = plan_dir(done);
    let before = std::fs::read_to_string(dir.path().join("tasks/M-v0.6.md")).unwrap();

    let plan = load_plan(dir.path());
    let err = cut::place(&plan, "M-v0.6", &["KEP-3063".to_string()]).expect_err("must refuse");
    assert!(matches!(err, cut::CutError::Settled { .. }), "{err:?}");
    assert!(err.to_string().contains("APLAN007"), "{err}");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("tasks/M-v0.6.md")).unwrap(),
        before,
        "a refusal must not write anything"
    );
}

#[test]
fn a_task_that_is_not_a_cut_is_refused() {
    let dir = plan_dir(OPEN_MILESTONE);
    let plan = load_plan(dir.path());
    // KEP-3063 is a unit of work, not a checkpoint.
    let err = cut::place(&plan, "KEP-3063", &["x".to_string()]).expect_err("must refuse");
    assert!(
        matches!(err, cut::CutError::NotAMilestone { .. }),
        "{err:?}"
    );
}

#[test]
fn an_unknown_cut_is_refused() {
    let dir = plan_dir(OPEN_MILESTONE);
    let plan = load_plan(dir.path());
    let err = cut::place(&plan, "M-nope", &["KEP-3063".to_string()]).expect_err("must refuse");
    assert!(matches!(err, cut::CutError::NotFound { .. }), "{err:?}");
}

#[test]
fn placing_the_same_id_twice_is_a_no_op() {
    let dir = plan_dir(OPEN_MILESTONE);
    with_member(dir.path());
    let plan = load_plan(dir.path());
    cut::place(&plan, "M-v0.6", &["KEP-3063".to_string()]).unwrap();
    let after_first = std::fs::read_to_string(dir.path().join("tasks/M-v0.6.md")).unwrap();

    // Re-running must not duplicate the member, which would be an APLAN022
    // violation ("exactly one cut") the second time around.
    let plan = load_plan(dir.path());
    let placed = cut::place(&plan, "M-v0.6", &["KEP-3063".to_string()]).unwrap();
    assert!(placed.added.is_empty(), "nothing new to add");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("tasks/M-v0.6.md")).unwrap(),
        after_first,
        "an idempotent run must not rewrite the file"
    );
}

#[test]
fn the_milestone_body_and_formatting_survive() {
    let dir = plan_dir(OPEN_MILESTONE);
    with_member(dir.path());
    let plan = load_plan(dir.path());
    cut::place(&plan, "M-v0.6", &["KEP-3063".to_string()]).unwrap();

    let text = std::fs::read_to_string(dir.path().join("tasks/M-v0.6.md")).unwrap();
    assert!(text.contains("## Exit criteria"), "body kept: {text}");
    assert!(text.contains("- [ ] ship it"), "body kept: {text}");
    assert!(
        text.contains("tags: [milestone, v0.6]"),
        "style kept: {text}"
    );
    assert!(
        text.contains("blocked_by: [A1-003, KEP-3063]"),
        "inline style kept: {text}"
    );
}

#[test]
fn a_dropped_cut_is_refused_too() {
    let dropped = "---\nid: M-v0.6\ntitle: Surfaces\nphase: 1\nstatus: dropped\ntags: [milestone, v0.6]\nblocked_by: [A1-003]\n---\n";
    let dir = plan_dir(dropped);
    let plan = load_plan(dir.path());
    let err = cut::place(&plan, "M-v0.6", &["KEP-3063".to_string()]).expect_err("must refuse");
    match err {
        cut::CutError::Settled { status, .. } => assert_eq!(status, "dropped"),
        other => panic!("expected Settled, got {other:?}"),
    }
}

#[test]
fn the_imported_task_is_still_a_todo_after_placement() {
    // Placement is about membership, not status: it must not touch the task.
    let dir = plan_dir(OPEN_MILESTONE);
    with_member(dir.path());
    let plan = load_plan(dir.path());
    cut::place(&plan, "M-v0.6", &["KEP-3063".to_string()]).unwrap();

    let reloaded = load_plan(dir.path());
    let imported = reloaded
        .tasks
        .iter()
        .find(|t| t.task.id == "KEP-3063")
        .unwrap();
    assert_eq!(imported.task.status, Status::Todo);
}
