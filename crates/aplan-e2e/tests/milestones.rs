//! End-to-end coverage of the milestone convention (ADR-0008), through the real
//! loader. The unit tests in `aplan-core` pin the graph algorithms in isolation;
//! this pins that a milestone *file on disk* loads, validates, and is excluded
//! from the work-views, and that a milestone's completeness is a checked proof
//! (`APLAN007`), not a label.

use aplan_core::{Graph, Options, Plan, code, load_plan, validate};
use std::fmt::Write as _;

/// Render one task file. `evidence` is a commit ref (required when `done`).
fn mk(
    id: &str,
    status: &str,
    effort: Option<f64>,
    blocked_by: &[&str],
    tags: &[&str],
    evidence: Option<&str>,
) -> String {
    let mut s = format!("---\nid: {id}\ntitle: {id}\nphase: 0\nstatus: {status}\n");
    if let Some(e) = effort {
        writeln!(s, "effort_days: {e}").unwrap();
    }
    if !blocked_by.is_empty() {
        writeln!(s, "blocked_by: [{}]", blocked_by.join(", ")).unwrap();
    }
    if !tags.is_empty() {
        writeln!(s, "tags: [{}]", tags.join(", ")).unwrap();
    }
    if let Some(sha) = evidence {
        write!(s, "evidence:\n  - kind: commit\n    ref: {sha}\n").unwrap();
    }
    s.push_str("---\nbody\n");
    s
}

/// Materialise `(relative path, contents)` under a fresh temp root and load it.
/// The `TempDir` is returned so the caller keeps it alive while borrowing `Plan`.
fn load_tree(files: &[(&str, String)]) -> (tempfile::TempDir, Plan) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, contents) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(&path, contents).expect("write");
    }
    let plan = load_plan(dir.path());
    (dir, plan)
}

#[test]
fn a_milestone_loads_validates_and_is_excluded_from_the_work_views() {
    let (_dir, plan) = load_tree(&[
        (
            "tasks/work.md",
            mk("work", "todo", Some(2.0), &[], &[], None),
        ),
        (
            "tasks/memberA.md",
            mk("memberA", "todo", Some(3.0), &[], &[], None),
        ),
        // A milestone with NO effort_days, blocked_by its one member.
        (
            "tasks/mile.md",
            mk("mile", "todo", None, &["memberA"], &["milestone"], None),
        ),
        // A task that depends on the whole cut (blocked_by the milestone).
        (
            "tasks/xdep.md",
            mk("xdep", "todo", Some(1.0), &["mile"], &[], None),
        ),
    ]);

    // Validates clean with effort required: the milestone is exempt from APLAN009
    // despite having no estimate, and its `blocked_by` membership resolves.
    let report = validate(
        &plan,
        &Options {
            require_effort: true,
            ..Default::default()
        },
    );
    assert!(
        report.is_clean(),
        "a milestone with no effort must not warn; got {:?}",
        report
            .diagnostics
            .iter()
            .map(|d| (d.code, d.message.as_str()))
            .collect::<Vec<_>>()
    );

    let g = Graph::new(plan.tasks.iter().map(|lt| &lt.task));

    // `ready` excludes the milestone (a checkpoint is never workable).
    let ready: Vec<&str> = g.ready().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ready,
        vec!["memberA", "work"],
        "milestone must not be ready"
    );

    // `critical-path` treats the milestone as a zero-weight passthrough: memberA
    // still gates xdep through the cut, and the checkpoint itself is hidden.
    let cp = g.critical_path().expect("acyclic");
    let path: Vec<&str> = cp.tasks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(path, vec!["memberA", "xdep"]);
    assert!((cp.total_effort_days - 4.0).abs() < 1e-9);
    assert!(
        !path.contains(&"mile"),
        "the checkpoint must not appear on the spine"
    );

    // `summary` excludes the milestone from the rollup (3 work tasks, not 4).
    assert_eq!(
        g.summary().total_tasks(),
        3,
        "milestone must not inflate the phase rollup"
    );
}

#[test]
fn a_cut_cannot_be_done_while_a_member_is_undone() {
    // "The cut is done" is a proof, not a label: a `done` milestone over a `todo`
    // member fires APLAN007, the same rule that stops any done task from
    // depending on unfinished work. This is what makes cut completeness checked.
    let (_dir, plan) = load_tree(&[
        (
            "tasks/memberB.md",
            mk("memberB", "todo", Some(1.0), &[], &[], None),
        ),
        (
            "tasks/mileDone.md",
            mk(
                "mileDone",
                "done",
                None,
                &["memberB"],
                &["milestone"],
                Some("9eabb17"),
            ),
        ),
    ]);
    let report = validate(&plan, &Options::default());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == code::DONE_BEFORE_BLOCKER),
        "a cut marked done over an undone member must fire APLAN007; got {:?}",
        report
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn roadmap_groups_the_loaded_plan_by_cut() {
    // A1-015 through the real loader: the roadmap groups a milestone plan by cut
    // (numeric version order), rolls each cut up, computes its critical path,
    // collects tasks in no cut, and cites evidence without resolving it.
    let (_dir, plan) = load_tree(&[
        (
            "tasks/a.md",
            mk("A", "done", Some(1.0), &[], &[], Some("abc1234")),
        ),
        ("tasks/b.md", mk("B", "todo", Some(2.0), &["A"], &[], None)),
        (
            "tasks/m1.md",
            mk("M-v0.1", "todo", None, &["A"], &["milestone", "v0.1"], None),
        ),
        (
            "tasks/m2.md",
            mk("M-v0.2", "todo", None, &["B"], &["milestone", "v0.2"], None),
        ),
        ("tasks/x.md", mk("X", "todo", Some(1.0), &[], &[], None)), // in no cut
    ]);
    let roadmap = Graph::new(plan.tasks.iter().map(|lt| &lt.task)).roadmap();

    let versions: Vec<&str> = roadmap.cuts.iter().map(|c| c.version.as_str()).collect();
    assert_eq!(versions, vec!["v0.1", "v0.2"]);
    assert_eq!((roadmap.cuts[0].done, roadmap.cuts[0].total), (1, 1)); // v0.1: A done
    assert_eq!((roadmap.cuts[1].ready, roadmap.cuts[1].total), (1, 1)); // v0.2: B ready (A done)

    let cross: Vec<&str> = roadmap
        .cross_cutting
        .iter()
        .map(|t| t.id.as_str())
        .collect();
    assert_eq!(cross, vec!["X"], "only the task in no cut is cross-cutting");

    // Evidence is cited, not resolved: A's commit shows as a short ref.
    assert_eq!(
        roadmap.cuts[0].members[0].evidence,
        vec!["commit:abc1234".to_string()]
    );
}
