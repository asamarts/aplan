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
}

/// The rollup for a single phase.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseSummary {
    pub phase: u32,
    /// Tasks in this phase, all statuses.
    pub tasks: usize,
    /// Of those, how many are `done`.
    pub done: usize,
    /// Sum of `effort_days` over the tasks that carry an estimate. Tasks without
    /// one contribute nothing here (and are flagged by `APLAN009`).
    pub effort_days: f64,
}

impl Summary {
    pub fn total_tasks(&self) -> usize {
        self.phases.iter().map(|p| p.tasks).sum()
    }

    pub fn total_done(&self) -> usize {
        self.phases.iter().map(|p| p.done).sum()
    }

    pub fn total_effort_days(&self) -> f64 {
        self.phases.iter().map(|p| p.effort_days).sum()
    }
}

impl Graph<'_> {
    /// Roll the plan up by phase, with the count of ready tasks. Phases come out
    /// in ascending order (`BTreeMap`), so the output is deterministic.
    pub fn summary(&self) -> Summary {
        let mut by_phase: BTreeMap<u32, PhaseSummary> = BTreeMap::new();
        for t in self.by_id.values() {
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
            });
            entry.tasks += 1;
            if t.status == Status::Done {
                entry.done += 1;
            }
            if let Some(days) = t.effort_days {
                entry.effort_days += days;
            }
        }
        Summary {
            phases: by_phase.into_values().collect(),
            ready: self.ready().len(),
        }
    }
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
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
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
                effort_days: 1.5
            }
        );
        assert_eq!(
            summary.phases[1],
            PhaseSummary {
                phase: 1,
                tasks: 2,
                done: 0,
                effort_days: 2.0
            }
        );
        assert_eq!(summary.total_tasks(), 4);
        assert_eq!(summary.total_done(), 1);
        assert!((summary.total_effort_days() - 3.5).abs() < 1e-9);
        // B (blocker A done) and D (no blockers) are ready; C is not (B not done).
        assert_eq!(summary.ready, 2);
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
                effort_days: 2.0
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
}
