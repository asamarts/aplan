//! Rollups over a plan, for `aplan summary`.
//!
//! A plan is only useful for sequencing if you can see where the days are. This
//! rolls the tasks up by phase (count, how many are done, total estimated
//! effort) and reports how many tasks are ready to pick up now.

use std::collections::BTreeMap;

use crate::graph::Graph;
use crate::task::Status;

/// A per-phase rollup plus the overall count of ready tasks.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// One entry per phase, in ascending phase order.
    pub phases: Vec<PhaseSummary>,
    /// Tasks that can be worked now: workable status, every blocker `done`.
    pub ready: usize,
    /// Effort days a week absorbs, if the repository declared one (`A1-147`,
    /// ADR-0023). `None` means it did not, and nothing about duration is
    /// reported.
    pub effort_days_per_week: Option<f64>,
}

/// The rollup for a single phase.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseSummary {
    pub phase: u32,
    /// Tasks in this phase that can still happen: every status but `dropped`.
    ///
    /// `dropped` is excluded so this agrees with `roadmap`, which has always
    /// counted live work. The two disagreeing was the finding: on a plan whose
    /// headline feature is a kill criterion, `summary` reported the
    /// pre-cancellation scope forever, and the plan's most consequential
    /// decision was invisible in the place people look (`A1-074`).
    pub tasks: usize,
    /// Of those, how many are `done`.
    pub done: usize,
    /// Sum of `effort_days` over the live tasks that carry an estimate. Tasks
    /// without one contribute nothing here (and are flagged by `APLAN009`).
    pub effort_days: f64,
    /// Tasks in this phase that were `dropped`.
    ///
    /// Reported rather than merely excluded: cancelling work is a decision, and
    /// a total that silently omits it is how this defect happened the first
    /// time. `dropped` is chosen over deletion precisely so the measurement that
    /// rejected the work stays findable.
    pub dropped: usize,
    /// Sum of `effort_days` over those dropped tasks.
    pub dropped_effort_days: f64,
    /// Of `effort_days`, the part already claimed `done` (`A1-147`).
    ///
    /// Carried so a duration can be reported for the work that is *left* as well
    /// as for the whole plan. Both are derived from it; neither is stored.
    pub done_effort_days: f64,
}

/// A summary rolled up by one declared tag family rather than by phase.
///
/// Group rows may overlap: tags are a vocabulary, not containment, and a task
/// may carry more than one member unless the repository separately declares an
/// `APLAN030` cardinality policy. `overall` is consequently computed from the
/// selected plan tasks themselves, never by summing these rows (`A1-186`).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupedSummary {
    /// The declared family name, as written in `summary.group_by`.
    pub family: String,
    /// Non-empty member rows in declaration order, then the ungrouped row.
    pub groups: Vec<GroupSummary>,
    overall: Summary,
}

/// One tag-family member's rollup. `label: None` is the tasks carrying no
/// member of the family.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupSummary {
    pub label: Option<String>,
    pub tasks: usize,
    pub done: usize,
    pub ready: usize,
    pub effort_days: f64,
    pub dropped: usize,
    pub dropped_effort_days: f64,
    pub done_effort_days: f64,
}

/// Weeks of work `effort` represents at `per_week`, or `None` when the
/// repository declared no throughput (`A1-147`, ADR-0023).
///
/// **Derived, never declared**, which is the whole decision: a duration written
/// down beside the effort it summarises is a second copy of a number, and the
/// sixth pilot's schedule drifted from its plan three times in nine audits.
/// Deriving it means the two cannot disagree, which is stronger than a rule that
/// notices when they do -- and it is ADR-0017's argument applied to a second
/// quantity.
///
/// It says nothing about dates. A week here is a unit of capacity, not a
/// position in a calendar: "eleven weeks of work at your declared throughput" is
/// checkable arithmetic, where "week 46" is a claim about the future that
/// nothing can verify until it arrives.
#[must_use]
pub fn weeks_of_work(effort: f64, per_week: Option<f64>) -> Option<f64> {
    // A non-positive or non-finite ratio divides into nonsense; a repository
    // that wrote one gets nothing rather than an infinity rendered as a
    // duration.
    let per_week = per_week.filter(|w| w.is_finite() && *w > 0.0)?;
    Some(effort / per_week)
}

impl Summary {
    /// The whole plan as weeks of work, at the declared throughput. Live work
    /// only: `dropped` is not time (`A1-074`).
    #[must_use]
    pub fn total_weeks(&self) -> Option<f64> {
        weeks_of_work(self.total_effort_days(), self.effort_days_per_week)
    }

    /// The same for the work not yet claimed `done`.
    #[must_use]
    pub fn remaining_weeks(&self) -> Option<f64> {
        weeks_of_work(
            self.total_effort_days() - self.total_done_effort_days(),
            self.effort_days_per_week,
        )
    }

    /// Sum of `effort_days` over live tasks already claimed `done`.
    #[must_use]
    pub fn total_done_effort_days(&self) -> f64 {
        self.phases.iter().map(|p| p.done_effort_days).sum::<f64>() + 0.0
    }

    /// Attach the repository's declared throughput, so the derived durations can
    /// be reported. Absent, every duration is `None` and no surface says
    /// anything about time.
    #[must_use]
    pub fn with_throughput(mut self, effort_days_per_week: Option<f64>) -> Self {
        self.effort_days_per_week = effort_days_per_week;
        self
    }

    pub fn total_tasks(&self) -> usize {
        self.phases.iter().map(|p| p.tasks).sum()
    }

    pub fn total_done(&self) -> usize {
        self.phases.iter().map(|p| p.done).sum()
    }

    /// `+ 0.0` normalises negative zero, here and below. `Sum` for floats folds
    /// from `-0.0`, the identity that preserves signed-zero semantics, so a plan
    /// with no phases totals `-0.0` and an empty plan reported `-0.0d`. Every
    /// float sum in this crate needs it; the one in `critical_path_within` was
    /// fixed first and these two were missed (`A1-074`).
    pub fn total_effort_days(&self) -> f64 {
        self.phases.iter().map(|p| p.effort_days).sum::<f64>() + 0.0
    }

    pub fn total_dropped(&self) -> usize {
        self.phases.iter().map(|p| p.dropped).sum()
    }

    pub fn total_dropped_effort_days(&self) -> f64 {
        self.phases
            .iter()
            .map(|p| p.dropped_effort_days)
            .sum::<f64>()
            + 0.0
    }
}

impl GroupedSummary {
    /// Attach the same repository throughput as the ordinary phase summary.
    #[must_use]
    pub fn with_throughput(mut self, effort_days_per_week: Option<f64>) -> Self {
        self.overall = self.overall.with_throughput(effort_days_per_week);
        self
    }

    pub fn ready(&self) -> usize {
        self.overall.ready
    }

    pub fn effort_days_per_week(&self) -> Option<f64> {
        self.overall.effort_days_per_week
    }

    pub fn total_tasks(&self) -> usize {
        self.overall.total_tasks()
    }

    pub fn total_done(&self) -> usize {
        self.overall.total_done()
    }

    pub fn total_effort_days(&self) -> f64 {
        self.overall.total_effort_days()
    }

    pub fn total_dropped(&self) -> usize {
        self.overall.total_dropped()
    }

    pub fn total_dropped_effort_days(&self) -> f64 {
        self.overall.total_dropped_effort_days()
    }

    pub fn total_done_effort_days(&self) -> f64 {
        self.overall.total_done_effort_days()
    }

    pub fn total_weeks(&self) -> Option<f64> {
        self.overall.total_weeks()
    }

    pub fn remaining_weeks(&self) -> Option<f64> {
        self.overall.remaining_weeks()
    }

    /// Whether at least one selected task appears in more than one group row.
    ///
    /// Every selected task appears in one or more rows: unmatched tasks land in
    /// the ungrouped row. A larger row sum can therefore only be overlap.
    pub fn groups_overlap(&self) -> bool {
        let row_memberships: usize = self
            .groups
            .iter()
            .map(|group| group.tasks + group.dropped)
            .sum();
        row_memberships > self.total_tasks() + self.total_dropped()
    }
}

impl Graph<'_> {
    /// Roll the plan up by phase, with the count of ready tasks. Phases come out
    /// in ascending order (`BTreeMap`), so the output is deterministic.
    pub fn summary(&self) -> Summary {
        self.summary_of(&|_| true)
    }

    /// The same rollup over the tasks `keep` admits.
    ///
    /// **`ready` is still counted over the whole graph**, intersected with the
    /// subset. Rolling up a filtered *graph* would recompute readiness inside
    /// it, and a task whose blocker sits outside the subset would be reported as
    /// ready when it is not: a false answer rather than a narrow one. Selection
    /// narrows what is shown, never what is computed (`A1-214`).
    #[must_use]
    pub fn summary_of(&self, keep: &dyn Fn(&str) -> bool) -> Summary {
        let mut by_phase: BTreeMap<u32, PhaseSummary> = BTreeMap::new();
        for t in self.by_id.values().filter(|t| keep(t.id.as_str())) {
            // Milestones are checkpoints, not work: they carry no effort and
            // would inflate a phase's task count, so they are not rolled up.
            if t.is_milestone() {
                continue;
            }
            let entry = by_phase.entry(t.phase).or_insert(PhaseSummary {
                phase: t.phase,
                tasks: 0,
                done: 0,
                effort_days: 0.0,
                dropped: 0,
                dropped_effort_days: 0.0,
                done_effort_days: 0.0,
            });
            if t.status == Status::Dropped {
                entry.dropped += 1;
                entry.dropped_effort_days += t.effort_days.unwrap_or(0.0);
                continue;
            }
            entry.tasks += 1;
            if t.status == Status::Done {
                entry.done += 1;
            }
            if let Some(days) = t.effort_days {
                entry.effort_days += days;
                if t.status == Status::Done {
                    entry.done_effort_days += days;
                }
            }
        }
        Summary {
            phases: by_phase.into_values().collect(),
            ready: self.ready().iter().filter(|t| keep(t.id.as_str())).count(),
            // Filled in by `with_throughput`: the graph knows the plan, not the
            // repository's configuration.
            effort_days_per_week: None,
        }
    }

    /// Roll the plan up by the members of one declared tag family.
    #[must_use]
    pub fn summary_grouped_by(&self, family: &str, members: &[String]) -> GroupedSummary {
        self.summary_grouped_by_of(family, members, &|_| true)
    }

    /// The grouped rollup over the tasks `keep` admits.
    ///
    /// Readiness is computed over the whole graph before the selection, for the
    /// same reason [`Graph::summary_of`] does so: excluding a blocker must not
    /// turn the task it blocks into ready work.
    #[must_use]
    pub fn summary_grouped_by_of(
        &self,
        family: &str,
        members: &[String],
        keep: &dyn Fn(&str) -> bool,
    ) -> GroupedSummary {
        let ready: std::collections::BTreeSet<&str> = self
            .ready()
            .into_iter()
            .filter(|task| keep(task.id.as_str()))
            .map(|task| task.id.as_str())
            .collect();
        let tasks: Vec<_> = self
            .by_id
            .values()
            .copied()
            .filter(|task| keep(task.id.as_str()) && !task.is_milestone())
            .collect();
        let mut groups = Vec::new();
        for label in members {
            let matching: Vec<_> = tasks
                .iter()
                .copied()
                .filter(|task| task.tags.iter().any(|tag| tag == label))
                .collect();
            if !matching.is_empty() {
                groups.push(group_summary(Some(label.clone()), matching, &ready));
            }
        }
        let ungrouped: Vec<_> = tasks
            .iter()
            .copied()
            .filter(|task| {
                !members
                    .iter()
                    .any(|member| task.tags.iter().any(|tag| tag == member))
            })
            .collect();
        if !ungrouped.is_empty() {
            groups.push(group_summary(None, ungrouped, &ready));
        }
        GroupedSummary {
            family: family.to_string(),
            groups,
            overall: self.summary_of(keep),
        }
    }
}

fn group_summary(
    label: Option<String>,
    tasks: Vec<&crate::task::Task>,
    ready: &std::collections::BTreeSet<&str>,
) -> GroupSummary {
    let mut out = GroupSummary {
        label,
        tasks: 0,
        done: 0,
        ready: 0,
        effort_days: 0.0,
        dropped: 0,
        dropped_effort_days: 0.0,
        done_effort_days: 0.0,
    };
    for task in tasks {
        if task.status == Status::Dropped {
            out.dropped += 1;
            out.dropped_effort_days += task.effort_days.unwrap_or(0.0);
            continue;
        }
        out.tasks += 1;
        out.done += usize::from(task.status == Status::Done);
        out.ready += usize::from(ready.contains(task.id.as_str()));
        if let Some(days) = task.effort_days {
            out.effort_days += days;
            if task.status == Status::Done {
                out.done_effort_days += days;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;

    fn task(
        id: &str,
        phase: u32,
        status: Status,
        effort: Option<f64>,
        blocked_by: &[&str],
    ) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase,
            status,
            owner: None,
            effort_days: effort,
            blocked_by: blocked_by.iter().map(|s| (*s).to_string()).collect(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    #[test]
    fn rolls_up_by_phase_with_effort_done_and_ready() {
        let done0 = task("A", 0, Status::Done, Some(1.0), &[]);
        let ready0 = task("B", 0, Status::Todo, Some(0.5), &["A"]); // ready: blocker done
        let blocked1 = task("C", 1, Status::Todo, Some(2.0), &["B"]); // not ready: B not done
        let noeffort1 = task("D", 1, Status::Todo, None, &[]); // ready, but no estimate
        let graph = Graph::new([&done0, &ready0, &blocked1, &noeffort1]);
        let summary = graph.summary();

        assert_eq!(summary.phases.len(), 2);
        assert_eq!(
            summary.phases[0],
            PhaseSummary {
                phase: 0,
                tasks: 2,
                done: 1,
                effort_days: 1.5,
                dropped: 0,
                dropped_effort_days: 0.0,
                done_effort_days: 1.0
            }
        );
        assert_eq!(
            summary.phases[1],
            PhaseSummary {
                phase: 1,
                tasks: 2,
                done: 0,
                effort_days: 2.0,
                dropped: 0,
                dropped_effort_days: 0.0,
                done_effort_days: 0.0
            }
        );
        assert_eq!(summary.total_tasks(), 4);
        assert_eq!(summary.total_done(), 1);
        assert!((summary.total_effort_days() - 3.5).abs() < 1e-9);
        // B (blocker A done) and D (no blockers) are ready; C is not (B not done).
        assert_eq!(summary.ready, 2);
    }

    #[test]
    fn an_empty_plan_totals_positive_zero() {
        // `Sum` for floats folds from `-0.0`, so a plan with no phases reported
        // `total 0 0 -0.0d` and emitted `-0.0` in JSON. The same bug as the
        // critical path's, in the two places the first fix did not reach.
        let tasks: [Task; 0] = [];
        let g = Graph::new(tasks.iter());
        let s = g.summary();
        assert!(s.total_effort_days().is_sign_positive(), "effort");
        assert!(
            s.total_dropped_effort_days().is_sign_positive(),
            "dropped effort"
        );
    }

    #[test]
    fn cancelled_work_is_named_rather_than_counted_as_scope() {
        // `roadmap` counted live tasks and `summary` counted every task, so on a
        // plan whose headline feature is a kill criterion the command an
        // operator reaches for first reported pre-cancellation scope forever
        // (`A1-074`).
        let tasks = [
            task("L1", 0, Status::Todo, Some(2.0), &[]),
            task("D1", 1, Status::Dropped, Some(3.0), &[]),
            task("D2", 1, Status::Dropped, Some(3.0), &[]),
        ];
        let g = Graph::new(tasks.iter());
        let s = g.summary();
        assert_eq!(s.total_tasks(), 1, "only live work is scope");
        assert!((s.total_effort_days() - 2.0).abs() < f64::EPSILON);
        // ... and the cancellation is visible, not merely absent.
        assert_eq!(s.total_dropped(), 2);
        assert!((s.total_dropped_effort_days() - 6.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_phase_of_only_dropped_work_still_reports_it() {
        // The phase must not vanish: a cut cancelled in full is the case worth
        // seeing, and an absent row reads as work that never existed.
        let tasks = [task("D1", 3, Status::Dropped, Some(4.0), &[])];
        let g = Graph::new(tasks.iter());
        let s = g.summary();
        let p = s
            .phases
            .iter()
            .find(|p| p.phase == 3)
            .expect("phase 3 present");
        assert_eq!((p.tasks, p.dropped), (0, 1));
        assert!((p.dropped_effort_days - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn summary_excludes_milestones() {
        // A milestone is excluded by its tag, not by having zero effort: even a
        // mis-authored milestone carrying effort must not inflate a phase's task
        // count or effort, and it is never counted as ready work.
        let work = task("W", 1, Status::Done, Some(2.0), &[]);
        let mut milestone = task("M", 1, Status::Todo, Some(9.0), &["W"]);
        milestone.tags = vec!["milestone".into()];
        let summary = Graph::new([&work, &milestone]).summary();
        assert_eq!(summary.phases.len(), 1);
        assert_eq!(
            summary.phases[0],
            PhaseSummary {
                phase: 1,
                tasks: 1,
                done: 1,
                effort_days: 2.0,
                dropped: 0,
                dropped_effort_days: 0.0,
                done_effort_days: 2.0
            }
        );
        assert_eq!(summary.ready, 0);
    }

    #[test]
    fn an_empty_plan_summarises_to_nothing() {
        let summary = Graph::new([]).summary();
        assert!(summary.phases.is_empty());
        assert_eq!(summary.ready, 0);
        assert!(summary.total_effort_days().abs() < 1e-9);
    }

    #[test]
    fn a_declared_family_rolls_up_overlaps_without_double_counting_the_total() {
        let mut both = task("A", 0, Status::Done, Some(2.0), &[]);
        both.tags = vec!["kind-build".into(), "kind-review".into()];
        let mut review = task("B", 1, Status::Todo, Some(3.0), &[]);
        review.tags = vec!["kind-review".into()];
        let ungrouped = task("C", 1, Status::Todo, Some(4.0), &["B"]);
        let mut dropped = task("D", 1, Status::Dropped, Some(5.0), &[]);
        dropped.tags = vec!["kind-build".into()];
        let graph = Graph::new([&both, &review, &ungrouped, &dropped]);

        let summary = graph.summary_grouped_by(
            "kind",
            &["kind-build".to_string(), "kind-review".to_string()],
        );

        assert_eq!(summary.family, "kind");
        assert_eq!(summary.groups.len(), 3);
        assert_eq!(summary.groups[0].label.as_deref(), Some("kind-build"));
        assert_eq!(
            (
                summary.groups[0].tasks,
                summary.groups[0].done,
                summary.groups[0].ready,
                summary.groups[0].dropped,
            ),
            (1, 1, 0, 1)
        );
        assert!((summary.groups[0].effort_days - 2.0).abs() < f64::EPSILON);
        assert!((summary.groups[0].dropped_effort_days - 5.0).abs() < f64::EPSILON);

        assert_eq!(summary.groups[1].label.as_deref(), Some("kind-review"));
        assert_eq!(
            (
                summary.groups[1].tasks,
                summary.groups[1].done,
                summary.groups[1].ready,
            ),
            (2, 1, 1)
        );
        assert!((summary.groups[1].effort_days - 5.0).abs() < f64::EPSILON);

        assert_eq!(summary.groups[2].label, None);
        assert_eq!((summary.groups[2].tasks, summary.groups[2].ready), (1, 0));
        assert!((summary.groups[2].effort_days - 4.0).abs() < f64::EPSILON);

        // A is in two rows, but it is one task in the plan. The total is a
        // distinct rollup, not the sum of overlapping group rows.
        assert_eq!(summary.total_tasks(), 3);
        assert_eq!(summary.total_done(), 1);
        assert_eq!(summary.ready(), 1);
        assert!((summary.total_effort_days() - 9.0).abs() < f64::EPSILON);
        assert_eq!(summary.total_dropped(), 1);
        assert!((summary.total_dropped_effort_days() - 5.0).abs() < f64::EPSILON);
        assert!(summary.groups_overlap());
    }

    #[test]
    fn a_grouped_selection_does_not_recompute_readiness_inside_the_subset() {
        let blocker = task("A", 0, Status::Todo, Some(1.0), &[]);
        let mut dependent = task("B", 0, Status::Todo, Some(2.0), &["A"]);
        dependent.tags = vec!["kind-build".into()];
        let graph = Graph::new([&blocker, &dependent]);
        let summary =
            graph.summary_grouped_by_of("kind", &["kind-build".to_string()], &|id| id == "B");

        assert_eq!(summary.groups[0].tasks, 1);
        assert_eq!(summary.groups[0].ready, 0);
        assert_eq!(summary.ready(), 0);
    }
}

#[cfg(test)]
mod duration_tests {
    use super::*;
    use crate::task::Task;

    fn t(id: &str, status: Status, effort: f64) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 1,
            status,
            owner: None,
            effort_days: Some(effort),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    /// The division, and the four inputs that are not one (`A1-147`).
    #[test]
    fn a_duration_is_effort_divided_by_a_declared_throughput() {
        assert_eq!(weeks_of_work(15.0, Some(7.5)), Some(2.0));
        // Undeclared is silence, not zero: a repository that never said what a
        // week absorbs gets no duration at all.
        assert_eq!(weeks_of_work(15.0, None), None);
        // The three unusable declarations. `validate_throughput` refuses these at
        // load, and this is the second line of defence: a duration rendered
        // `inf`, `-2.1` or `NaN` is worse than none.
        assert_eq!(weeks_of_work(15.0, Some(0.0)), None);
        assert_eq!(weeks_of_work(15.0, Some(-7.5)), None);
        assert_eq!(weeks_of_work(15.0, Some(f64::INFINITY)), None);
        assert_eq!(weeks_of_work(15.0, Some(f64::NAN)), None);
    }

    /// The whole plan and the part of it that is left, from the same numbers.
    #[test]
    fn the_remainder_counts_only_work_not_yet_done() {
        let a = t("A", Status::Done, 10.0);
        let b = t("B", Status::Todo, 5.0);
        let s = Graph::new([&a, &b]).summary().with_throughput(Some(5.0));
        assert!((s.total_effort_days() - 15.0).abs() < 1e-9);
        assert!((s.total_done_effort_days() - 10.0).abs() < 1e-9);
        assert_eq!(s.total_weeks(), Some(3.0));
        assert_eq!(s.remaining_weeks(), Some(1.0));
    }

    /// The sixth pilot's third drift: a calendar never rescaled after scope grew
    /// by 20% (pre-public adoption evidence item 2).
    ///
    /// It cannot happen to a derived duration, and that is the claim this test
    /// exists to hold: nothing is written down to go stale, so growing the plan
    /// moves the number by exactly the growth. A tool that *checked* a declared
    /// duration would instead report a finding here and wait for someone to edit
    /// the declaration, which is the same maintenance the pilot was doing by
    /// hand.
    #[test]
    fn growing_the_plan_rescales_the_duration_with_no_edit() {
        let a = t("A", Status::Todo, 100.0);
        let before = Graph::new([&a])
            .summary()
            .with_throughput(Some(10.0))
            .total_weeks();
        assert_eq!(before, Some(10.0));

        let grown = t("B", Status::Todo, 20.0);
        let after = Graph::new([&a, &grown])
            .summary()
            .with_throughput(Some(10.0))
            .total_weeks();
        assert_eq!(after, Some(12.0), "20% more scope, 20% longer, unattended");
    }

    /// Cancelled work is not work, on this surface as on every other (`A1-074`).
    #[test]
    fn a_dropped_task_is_not_in_the_duration() {
        let a = t("A", Status::Todo, 10.0);
        let gone = t("B", Status::Dropped, 90.0);
        let s = Graph::new([&a, &gone]).summary().with_throughput(Some(5.0));
        assert_eq!(s.total_weeks(), Some(2.0), "the dropped task is not time");
    }
}
