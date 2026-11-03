//! A file that fails to parse must not manufacture errors about other files
//! (`A1-072`).
//!
//! The fourth pilot's first contact with the tool was 85 errors across 72 tasks,
//! of which 52 were phantoms: 33 titles contained an unquoted `: `, and every
//! `blocked_by` pointing at one of those files reported an unknown task. The
//! task was not unknown. Its file was right there and failed to load.
//!
//! Driven through the binary because the number an adopter reads is the summary
//! line, and that is produced by the CLI rather than by either library it calls.

use crate::common;
use common::aplan;

/// `broken` files carrying the pilot's exact defect, and `dependents` tasks each
/// depending on every one of them plus one id that genuinely does not exist.
fn plan(broken: usize, dependents: usize) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("plan dir");

    for n in 0..broken {
        std::fs::write(
            tasks.join(format!("RD0-{n:02}.md")),
            format!(
                "---\nid: RD0-{n:02}\n\
                 title: ci/scripts/plan.sh: check, verify, roadmap diff; wire the workflow job\n\
                 phase: 0\nstatus: todo\neffort_days: 1\n---\n"
            ),
        )
        .expect("broken task");
    }

    let blockers: Vec<String> = (0..broken).map(|n| format!("RD0-{n:02}")).collect();
    for n in 0..dependents {
        std::fs::write(
            tasks.join(format!("RD1-{n:02}.md")),
            format!(
                "---\nid: RD1-{n:02}\ntitle: a downstream task\nphase: 1\nstatus: todo\n\
                 effort_days: 1\nblocked_by: [{}, GHOST]\n---\n",
                blockers.join(", ")
            ),
        )
        .expect("dependent task");
    }
    dir
}

#[test]
fn the_error_count_is_the_number_of_real_faults() {
    // Three unparseable files and four tasks depending on all of them is twelve
    // edges into files that did not load. Before this, all twelve were reported
    // as unknown tasks and the adopter read "19 errors" for three mistakes.
    let dir = plan(3, 4);
    let (out, code) = aplan(dir.path(), &["check"]);

    let parse_errors = out.matches("APLAN001").count();
    let unknown = out.matches("APLAN003").count();
    assert_eq!(parse_errors, 3, "one per unparseable file: {out}");
    assert_eq!(
        unknown, 4,
        "one per task, for the id that genuinely does not exist: {out}"
    );
    assert!(
        // The cause travels with the verdict now (`A1-112`), and the count it
        // carries is still the real faults rather than the cascade.
        out.contains("failed: 3 task file(s) did not load, 7 error(s)"),
        "the count must be the real faults, not the cascade: {out}"
    );
    assert_eq!(code, 1, "{out}");
}

#[test]
fn the_cascade_does_not_grow_with_the_plan() {
    // The defect scaled with how well connected the plan was, which punished
    // exactly the plans worth writing. Doubling the dependents must not change
    // the number of findings about the broken files.
    let small = aplan(plan(3, 4).path(), &["check"]).0;
    let large = aplan(plan(3, 40).path(), &["check"]).0;
    assert_eq!(
        small.matches("APLAN001").count(),
        large.matches("APLAN001").count(),
        "the real fault count is a property of the broken files, not the graph"
    );
    assert_eq!(
        large.matches("APLAN003").count(),
        40,
        "still one genuine dangling reference per dependent, and nothing more"
    );
}

#[test]
fn the_parse_error_says_what_to_do_about_it() {
    // `line 2 column 26` is inside the title and names neither the field nor the
    // fix. An adopter who has never seen this cannot act on it.
    let dir = plan(1, 0);
    let (out, _) = aplan(dir.path(), &["check"]);
    assert!(out.contains("`title`"), "name the field: {out}");
    assert!(out.contains("quoted"), "name the fix: {out}");
}
