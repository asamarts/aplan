//! The scale gate: `check` must not become sensitive to a cut's width again
//! (`A1-229`).
//!
//! `A1-227` found `aplan check` quadratic in a single cut's membership -- 25,000
//! tasks in ten cuts took 24,992ms against 942ms for the same tasks in 2,500
//! cuts -- and nothing in this repository would have noticed. Coverage is
//! ratcheted, rule messages are pinned, the CLI's bytes are pinned; performance
//! was the one property with a claim attached (`A1-225` argues for a pre-commit
//! hook on the strength of `check` running in 70ms) and no gate.
//!
//! **This asserts a shape, not a budget.** A millisecond floor on a shared
//! CI runner is a flaky test, and `A1-228` proved the point the
//! expensive way: it published an 11% difference between two plan shapes that
//! was CI contention on the same box, and re-measuring with the runner idle
//! reversed the sign. What survives a busy machine is the *ratio* between shapes
//! measured in the same run, because contention slows all of them together.
//!
//! `graph.rs`'s `finding_a_cycle_costs_the_same_however_wide_a_cut_is` is the
//! same assertion for one function, in-process, cheap enough to run on every
//! commit. This is the whole-command version: a real plan on disk, loaded and
//! validated the way `check` does it, at a size that makes a regression obvious.
//! It is `#[ignore]`d for that reason -- see `ci/README.md` for where it runs.

use aplan_e2e::scale::{Shape, cut_id, write_plan};
use std::time::{Duration, Instant};

/// Tasks held constant across every shape, so the only variable is how wide a
/// cut is. Small enough to generate in seconds, wide enough that the quadratic
/// this defends against is unmissable.
const TASKS: usize = 6_000;

/// Cut counts, from four very wide cuts to fifteen hundred narrow ones.
const SHAPES: [usize; 4] = [4, 20, 120, 1500];

/// Best of this many, round-robin across the shapes.
const ROUNDS: usize = 3;

/// The bound on slowest-against-fastest. Generous on purpose: the shapes do the
/// same work and should land within noise of each other, and the quadratic this
/// exists to catch exceeds this by an order of magnitude rather than a hair.
const FACTOR: u32 = 4;

#[test]
#[ignore = "generates 6,000 task files; runs in the scale job, not on every commit"]
fn check_does_not_care_how_wide_a_cut_is() {
    let dirs: Vec<(usize, tempfile::TempDir)> = SHAPES
        .into_iter()
        .map(|cuts| {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("tempdir");
            let shape = Shape {
                tasks: TASKS,
                cuts,
                // Both shapes must reach the same code path; see `Shape::deps`.
                deps: true,
            };
            write_plan(dir.path(), &shape).expect("write plan");
            (cuts, dir)
        })
        .collect();

    // **The fixtures are what they claim to be**, checked before anything is
    // timed. Everything below is a duration, and a duration is just as green
    // against a plan that failed to generate.
    for (cuts, dir) in &dirs {
        let plan = aplan_core::load_plan(&dir.path().join("aplan"));
        assert!(
            plan.load_diagnostics.is_empty(),
            "{cuts} cuts: the generated plan does not load: {:?}",
            plan.load_diagnostics.first()
        );
        assert_eq!(
            plan.tasks.len(),
            TASKS + cuts,
            "{cuts} cuts: wrong task count"
        );
        let wanted = TASKS / cuts;
        let cut = plan
            .tasks
            .iter()
            .find(|t| t.task.id == cut_id(0))
            .expect("the first cut");
        assert_eq!(
            cut.task.blocked_by.len(),
            wanted,
            "{cuts} cuts: the milestone holds {} members, not {wanted}, so this is not \
             measuring what made `check` quadratic",
            cut.task.blocked_by.len()
        );
    }

    // Round-robin rather than shape by shape, so a machine that gets busy for a
    // stretch slows every row instead of the one under measurement at the time.
    //
    // **The two phases are timed apart, and this is the difference between a
    // gate and a formality.** `check` is `load_plan` then `validate`, and load
    // is O(tasks): it is the same ~920ms in every shape here and 87% of the
    // total. Asserting a ratio on the sum buries the term that actually varies
    // -- measured, the widest shape would have had to regress **25x** in
    // `validate` before the total moved 4x, so a five- or ten-fold return of
    // `A1-227`'s defect would have passed green. Attributed, a 4x regression in
    // the phase that varies fails immediately.
    let opts = aplan_core::validate::Options::default();
    let mut load_best = [Duration::MAX; SHAPES.len()];
    let mut check_best = [Duration::MAX; SHAPES.len()];
    for _ in 0..ROUNDS {
        for (i, (cuts, dir)) in dirs.iter().enumerate() {
            let root = dir.path().join("aplan");
            let started = Instant::now();
            let plan = aplan_core::load_plan(&root);
            let loaded = started.elapsed();
            let validating = Instant::now();
            let report = aplan_core::validate(&plan, &opts);
            let validated = validating.elapsed();
            load_best[i] = load_best[i].min(loaded);
            check_best[i] = check_best[i].min(validated);
            // The timed run is the one that has to be real. Asserting the
            // fixture above says nothing about *this* load, and an empty plan
            // validates instantly with no diagnostics at all -- which is a fast
            // green run measuring nothing.
            assert_eq!(
                plan.tasks.len(),
                TASKS + cuts,
                "{cuts} cuts: the timed load did not read the plan"
            );
            assert!(
                report
                    .diagnostics
                    .iter()
                    .all(|d| d.severity != aplan_core::Severity::Error),
                "the generated plan does not validate cleanly, so this is measuring a \
                 refusal: {:?}",
                report.diagnostics.first()
            );
        }
    }

    ratio_holds("validate", &check_best);
    // Loading is O(tasks) and every shape holds the same number, so this is flat
    // by construction and exists to catch a loader that starts caring how long a
    // `blocked_by` list is. The narrow shapes carry more milestone *files*, so
    // it is not exactly 1.
    ratio_holds("load", &load_best);
}

/// Fail when the slowest shape is more than [`FACTOR`] times the fastest,
/// naming every row so the failure says which shape moved.
fn ratio_holds(phase: &str, best: &[Duration; SHAPES.len()]) {
    let slowest = best.iter().max().expect("four shapes");
    let fastest = best.iter().min().expect("four shapes");
    let shown: Vec<String> = SHAPES
        .into_iter()
        .zip(*best)
        .map(|(c, d)| format!("{c} cuts of {} = {d:?}", TASKS / c))
        .collect();
    assert!(
        *slowest <= *fastest * FACTOR,
        "`{phase}` is following a cut's width, which is the shape `A1-227` removed: {}",
        shown.join(", ")
    );
}
