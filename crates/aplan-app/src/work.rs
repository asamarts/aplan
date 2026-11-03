//! Typed work-discovery projection for interactive clients (`A1-429`).
//!
//! `ready`, `why` and `due` answer adjacent parts of one question. Delivery
//! adapters should not rebuild their graph semantics or turn a revisit instant
//! into an assertion that an external condition became true, so this module
//! carries those answers together over one identified input.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use aplan_core::Graph;
use aplan_core::graph::Walk;
use aplan_core::task::{Status, Task};

use crate::{InputIdentity, PlanContext};

/// How far the blocking explanation walks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WhyTraversal {
    /// Stop at a cut and state how much unmet work it folds.
    #[default]
    CutBounded,
    /// Walk the complete transitive blocker closure.
    Complete,
}

impl WhyTraversal {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::CutBounded => "cut-bounded",
            Self::Complete => "complete",
        }
    }

    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::CutBounded => Self::Complete,
            Self::Complete => Self::CutBounded,
        }
    }
}

impl From<WhyTraversal> for Walk {
    fn from(value: WhyTraversal) -> Self {
        match value {
            WhyTraversal::CutBounded => Self::CutBounded,
            WhyTraversal::Complete => Self::Complete,
        }
    }
}

/// Inputs that can change a work-discovery answer without changing plan bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkDiscoveryRequest {
    pub traversal: WhyTraversal,
    /// Explicit evaluation clock. No projection in this module reads the
    /// process clock, so the same request is reproducible.
    pub as_of_epoch_seconds: i64,
}

/// Why a task with no unmet graph blocker is not ordinary ready work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkVerdict {
    Ready,
    Blocked,
    /// Explicitly parked by status, whether or not it also has graph blockers.
    Parked,
    /// A milestone whose members are complete and which now needs closing.
    CloseableCut,
    /// A milestone with no unmet blocker, but not ordinary pick-up work.
    UnblockedCut,
    /// Work already completed or abandoned.
    Terminal(Status),
}

/// Whether a named relation can be opened from this snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRelationAvailability {
    Loaded,
    Missing,
    Unloadable(PathBuf),
}

/// One task at the other end of a work-discovery relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRelation {
    pub id: String,
    pub title: Option<String>,
    pub status: Option<Status>,
    pub is_cut: bool,
    pub availability: WorkRelationAvailability,
    /// Present only when a cut-bounded answer stopped at this cut and omitted
    /// unmet work behind it.
    pub folded_unmet: Option<usize>,
}

/// The exact factors behind a ready row's final position.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadyRank {
    /// One-based position after declared preferences have been honoured.
    pub position: usize,
    pub releases: usize,
    pub critical: bool,
    pub downstream_depth: usize,
    /// The effective ranking value. It is `0.0` when the task had no estimate,
    /// matching [`aplan_core::graph::Rank::key`].
    pub effort_days: f64,
    /// The subset that actually constrained this ready answer.
    pub active_preferences: BTreeSet<String>,
}

/// How a task's revisit metadata relates to the explicit evaluation clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisitState {
    None,
    Waiting {
        at: String,
        seconds_until: i64,
    },
    /// Due means only that it is time to re-read the parked task. It does not
    /// assert that the external condition in its body is satisfied.
    Due {
        at: String,
        overdue_seconds: i64,
    },
    /// Validation reports this separately; the interactive answer must still
    /// retain the malformed value rather than make it disappear.
    Invalid {
        at: String,
    },
    /// `revisit_at` is defined only for parked work. Retain an invalidly placed
    /// value without presenting it as due work.
    NotParked {
        at: String,
    },
}

/// One scheduled revisit in a plan-wide due projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledRevisit {
    pub id: String,
    pub title: String,
    pub at: String,
    /// Negative means the revisit is still this many seconds away; zero and
    /// positive values mean it is due by this many seconds.
    pub overdue_seconds: i64,
}

/// Plan-wide revisit membership at one explicit instant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RevisitSchedule {
    pub due: Vec<ScheduledRevisit>,
    pub waiting: Vec<ScheduledRevisit>,
}

/// `ready`, `why` and `due` for one task, from one identified plan and clock.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkDiscoveryProjection {
    pub input_sha256: String,
    pub task_id: String,
    pub title: String,
    pub status: Status,
    pub verdict: WorkVerdict,
    pub traversal: WhyTraversal,
    pub as_of_epoch_seconds: i64,
    pub rank: Option<ReadyRank>,
    /// Every declared ordering preference, including those that are inert in
    /// the current ready answer or belong to work that is not currently ready.
    pub preferences: Vec<WorkRelation>,
    pub blockers: Vec<WorkRelation>,
    /// Direct dependents, using the same graph inversion as canonical `show`.
    pub releases: Vec<WorkRelation>,
    pub revisit: RevisitState,
}

/// Immutable graph inputs retained by an interactive snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkDiscoveryWorkspace {
    input: InputIdentity,
    tasks: Vec<Task>,
    unloadable: BTreeMap<String, PathBuf>,
}

impl WorkDiscoveryWorkspace {
    #[must_use]
    pub fn from_context(context: &PlanContext) -> Self {
        Self::new(
            context.identity().clone(),
            context
                .plan()
                .tasks
                .iter()
                .map(|loaded| loaded.task.clone())
                .collect(),
            context.plan().unloadable.clone(),
        )
    }

    #[must_use]
    pub const fn input(&self) -> &InputIdentity {
        &self.input
    }

    #[must_use]
    pub fn new(
        input: InputIdentity,
        tasks: Vec<Task>,
        unloadable: BTreeMap<String, PathBuf>,
    ) -> Self {
        Self {
            input,
            tasks,
            unloadable,
        }
    }

    /// Project one task. An unknown id is not an empty answer.
    #[must_use]
    pub fn project(
        &self,
        id: &str,
        request: WorkDiscoveryRequest,
    ) -> Option<WorkDiscoveryProjection> {
        let graph = Graph::new(self.tasks.iter());
        let task = graph.get(id)?;
        let ranked = graph.ready_ranked();
        let offered: BTreeSet<&str> = ranked.iter().map(|(task, _)| task.id.as_str()).collect();
        let rank = ranked
            .iter()
            .enumerate()
            .find(|(_, (candidate, _))| candidate.id == id)
            .map(|(position, (_, rank))| ReadyRank {
                position: position + 1,
                releases: rank.releases,
                critical: rank.critical,
                downstream_depth: rank.downstream_depth,
                effort_days: rank.effort_days,
                active_preferences: task
                    .prefer_after
                    .iter()
                    .filter(|preference| offered.contains(preference.as_str()))
                    .cloned()
                    .collect(),
            });
        let walk = Walk::from(request.traversal);
        let mut blockers = graph
            .why_blocked(id, walk)
            .into_iter()
            .map(|blocker| {
                let folded = (walk == Walk::CutBounded && blocker.is_milestone())
                    .then(|| graph.folded_behind(&blocker.id))
                    .filter(|count| *count > 0);
                self.relation(&graph, &blocker.id, folded)
            })
            .collect::<Vec<_>>();
        blockers.extend(
            graph
                .unknown_blockers(id, walk)
                .into_iter()
                .map(|unknown| self.relation(&graph, unknown, None)),
        );
        let closeable = graph
            .closeable_milestones()
            .iter()
            .any(|milestone| milestone.id == id);
        let verdict = if task.status.is_terminal() {
            WorkVerdict::Terminal(task.status)
        } else if !task.status.is_workable() {
            WorkVerdict::Parked
        } else if closeable {
            WorkVerdict::CloseableCut
        } else if task.is_milestone() && blockers.is_empty() {
            WorkVerdict::UnblockedCut
        } else if blockers.is_empty() {
            WorkVerdict::Ready
        } else {
            WorkVerdict::Blocked
        };
        Some(WorkDiscoveryProjection {
            input_sha256: self.input.sha256.clone(),
            task_id: task.id.clone(),
            title: task.title.clone(),
            status: task.status,
            verdict,
            traversal: request.traversal,
            as_of_epoch_seconds: request.as_of_epoch_seconds,
            rank,
            preferences: task
                .prefer_after
                .iter()
                .map(|preference| self.relation(&graph, preference, None))
                .collect(),
            blockers,
            releases: graph
                .dependents_of(id)
                .into_iter()
                .map(|dependent| self.relation(&graph, &dependent.id, None))
                .collect(),
            revisit: revisit_state(task, request.as_of_epoch_seconds),
        })
    }

    /// Every valid revisit split around one explicit clock, in the same stable
    /// order as `aplan due` and Overview.
    #[must_use]
    pub fn revisit_schedule(&self, as_of_epoch_seconds: i64) -> RevisitSchedule {
        let mut schedule = RevisitSchedule::default();
        for task in &self.tasks {
            match revisit_state(task, as_of_epoch_seconds) {
                RevisitState::Due {
                    at,
                    overdue_seconds,
                } => schedule.due.push(ScheduledRevisit {
                    id: task.id.clone(),
                    title: task.title.clone(),
                    at,
                    overdue_seconds,
                }),
                RevisitState::Waiting { at, seconds_until } => {
                    schedule.waiting.push(ScheduledRevisit {
                        id: task.id.clone(),
                        title: task.title.clone(),
                        at,
                        overdue_seconds: -seconds_until,
                    });
                }
                RevisitState::None
                | RevisitState::Invalid { .. }
                | RevisitState::NotParked { .. } => {}
            }
        }
        schedule.due.sort_by(|left, right| {
            right
                .overdue_seconds
                .cmp(&left.overdue_seconds)
                .then_with(|| left.id.cmp(&right.id))
        });
        schedule.waiting.sort_by(|left, right| {
            right
                .overdue_seconds
                .cmp(&left.overdue_seconds)
                .then_with(|| left.id.cmp(&right.id))
        });
        schedule
    }

    fn relation(&self, graph: &Graph<'_>, id: &str, folded_unmet: Option<usize>) -> WorkRelation {
        if let Some(task) = graph.get(id) {
            WorkRelation {
                id: task.id.clone(),
                title: Some(task.title.clone()),
                status: Some(task.status),
                is_cut: task.is_milestone(),
                availability: WorkRelationAvailability::Loaded,
                folded_unmet,
            }
        } else {
            WorkRelation {
                id: id.to_string(),
                title: None,
                status: None,
                is_cut: false,
                availability: self
                    .unloadable
                    .get(id)
                    .map_or(WorkRelationAvailability::Missing, |path| {
                        WorkRelationAvailability::Unloadable(path.clone())
                    }),
                folded_unmet: None,
            }
        }
    }
}

/// Parse an interactive due-work clock. RFC 3339 is the persisted/CLI form;
/// signed epoch seconds are also accepted because the TUI displays that exact
/// value and it is convenient for deterministic tests and copy/paste.
#[must_use]
pub fn parse_work_as_of(input: &str, snapshot_epoch: i64) -> Option<i64> {
    let value = input.trim();
    if value.is_empty() {
        return Some(snapshot_epoch);
    }
    value
        .parse::<i64>()
        .ok()
        .or_else(|| aplan_core::validate::rfc3339_to_epoch(value))
}

fn revisit_state(task: &Task, as_of_epoch_seconds: i64) -> RevisitState {
    let Some(at) = &task.revisit_at else {
        return RevisitState::None;
    };
    if task.status != Status::Blocked {
        return RevisitState::NotParked { at: at.clone() };
    }
    let Some(epoch) = aplan_core::validate::rfc3339_to_epoch(at) else {
        return RevisitState::Invalid { at: at.clone() };
    };
    if epoch > as_of_epoch_seconds {
        RevisitState::Waiting {
            at: at.clone(),
            seconds_until: epoch.saturating_sub(as_of_epoch_seconds),
        }
    } else {
        RevisitState::Due {
            at: at.clone(),
            overdue_seconds: as_of_epoch_seconds.saturating_sub(epoch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigurationIdentity, RepositoryIdentity, Trust};

    fn task(id: &str, status: Status, blocked_by: &[&str], tags: &[&str]) -> Task {
        Task {
            id: id.to_string(),
            title: format!("title {id}"),
            phase: 1,
            status,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: blocked_by.iter().map(|id| (*id).to_string()).collect(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: BTreeMap::new(),
            evidence_fingerprints: BTreeMap::new(),
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    fn workspace(tasks: Vec<Task>) -> WorkDiscoveryWorkspace {
        WorkDiscoveryWorkspace::new(
            InputIdentity {
                source: "test",
                plan_root: "plan".into(),
                plan_sha256: "plan".to_string(),
                repository_root: "repo".into(),
                repository: RepositoryIdentity::NotObserved {
                    reason: "test".to_string(),
                },
                configuration: ConfigurationIdentity {
                    path: None,
                    sha256: "config".to_string(),
                    content: crate::ConfigurationContentIdentity::Absent,
                },
                trust: Trust::Hermetic,
                sha256: "input".to_string(),
            },
            tasks,
            BTreeMap::from([("BROKEN".to_string(), PathBuf::from("BROKEN.md"))]),
        )
    }

    fn request(traversal: WhyTraversal, as_of_epoch_seconds: i64) -> WorkDiscoveryRequest {
        WorkDiscoveryRequest {
            traversal,
            as_of_epoch_seconds,
        }
    }

    /// Proof for `tui.work_discovery_is_one_typed_flow`: rank factors and both
    /// declared and active preferences travel with the exact ready row.
    #[test]
    fn ready_rank_factors_and_preferences_are_explicit() {
        let first = task("A", Status::Todo, &[], &[]);
        let mut second = task("B", Status::Todo, &[], &[]);
        second.effort_days = Some(3.0);
        second.prefer_after = vec!["A".to_string(), "DONE".to_string(), "GHOST".to_string()];
        let done = task("DONE", Status::Done, &[], &[]);
        let mut child = task("CHILD", Status::Todo, &["B"], &[]);
        child.prefer_after = vec!["A".to_string()];
        let workspace = workspace(vec![first, second, done, child]);

        let projection = workspace
            .project("B", request(WhyTraversal::CutBounded, 0))
            .expect("known task");
        let rank = projection.rank.expect("ready task is ranked");
        assert_eq!(rank.position, 2);
        assert_eq!(rank.releases, 1);
        assert!(rank.critical);
        assert_eq!(rank.downstream_depth, 1);
        assert!(
            (rank.effort_days - 3.0).abs() < f64::EPSILON,
            "unexpected effort: {}",
            rank.effort_days
        );
        assert_eq!(projection.preferences.len(), 3);
        assert_eq!(rank.active_preferences, BTreeSet::from(["A".to_string()]));
        assert_eq!(projection.verdict, WorkVerdict::Ready);

        let child = workspace
            .project("CHILD", request(WhyTraversal::CutBounded, 0))
            .expect("blocked child");
        assert_eq!(child.verdict, WorkVerdict::Blocked);
        assert!(child.rank.is_none());
        assert_eq!(child.preferences.len(), 1);
        assert_eq!(child.preferences[0].id, "A");
    }

    /// Proof for `tui.work_discovery_is_one_typed_flow` and
    /// `why.complete_retains_nested_undefined_blockers`: bounded traversal
    /// states its fold, complete traversal expands it through unresolved leaf
    /// ids, and undefined input is never confused with an empty answer.
    #[test]
    fn why_traversal_labels_folds_and_undefined_blockers() {
        let leaf = task("LEAF", Status::Todo, &["NESTED-GHOST"], &[]);
        let cut = task("CUT", Status::Todo, &["LEAF"], &["milestone"]);
        let subject = task("SUBJECT", Status::Todo, &["CUT", "GHOST", "BROKEN"], &[]);
        let workspace = workspace(vec![leaf, cut, subject]);

        let bounded = workspace
            .project("SUBJECT", request(WhyTraversal::CutBounded, 0))
            .expect("known task");
        assert_eq!(bounded.traversal.label(), "cut-bounded");
        assert_eq!(bounded.blockers.len(), 3);
        assert_eq!(bounded.blockers[0].id, "CUT");
        assert_eq!(bounded.blockers[0].folded_unmet, Some(1));
        assert_eq!(bounded.blockers[1].id, "GHOST");
        assert_eq!(
            bounded.blockers[1].availability,
            WorkRelationAvailability::Missing
        );
        assert_eq!(bounded.blockers[2].id, "BROKEN");
        assert!(matches!(
            bounded.blockers[2].availability,
            WorkRelationAvailability::Unloadable(_)
        ));
        assert!(
            bounded
                .blockers
                .iter()
                .all(|blocker| blocker.id != "NESTED-GHOST"),
            "the cut-bounded projection exposed a blocker behind the cut"
        );

        let complete = workspace
            .project("SUBJECT", request(WhyTraversal::Complete, 0))
            .expect("known task");
        assert_eq!(complete.traversal.label(), "complete");
        assert_eq!(
            complete
                .blockers
                .iter()
                .map(|relation| relation.id.as_str())
                .collect::<Vec<_>>(),
            vec!["CUT", "LEAF", "GHOST", "BROKEN", "NESTED-GHOST"]
        );
        assert_eq!(
            complete.blockers[4].availability,
            WorkRelationAvailability::Missing
        );
        assert!(
            complete
                .blockers
                .iter()
                .all(|relation| relation.folded_unmet.is_none())
        );
        assert!(
            workspace
                .project("NO-SUCH-TASK", request(WhyTraversal::CutBounded, 0))
                .is_none()
        );
    }

    /// Proof for `tui.work_discovery_is_one_typed_flow`: a due instant means
    /// re-read, the explicit clock is inclusive, and status verdicts do not
    /// collapse milestones or terminal work into ready/blocked.
    #[test]
    fn revisit_and_status_verdicts_are_exact_at_the_boundary() {
        let done_member = task("DONE", Status::Done, &[], &[]);
        let cut = task("CUT", Status::Todo, &["DONE"], &["milestone"]);
        let terminal = task("DROPPED", Status::Dropped, &[], &[]);
        let active = task("ACTIVE", Status::InProgress, &[], &[]);
        let empty_cut = task("EMPTY-CUT", Status::Todo, &[], &["milestone"]);
        let mut parked = task("PARKED", Status::Blocked, &[], &[]);
        parked.revisit_at = Some("1970-01-01T00:00:10Z".to_string());
        let workspace = workspace(vec![done_member, cut, terminal, active, empty_cut, parked]);

        let waiting = workspace
            .project("PARKED", request(WhyTraversal::CutBounded, 9))
            .expect("parked task");
        assert_eq!(waiting.verdict, WorkVerdict::Parked);
        assert_eq!(
            waiting.revisit,
            RevisitState::Waiting {
                at: "1970-01-01T00:00:10Z".to_string(),
                seconds_until: 1,
            }
        );
        let due = workspace
            .project("PARKED", request(WhyTraversal::CutBounded, 10))
            .expect("parked task");
        assert_eq!(due.as_of_epoch_seconds, 10);
        assert_eq!(
            due.revisit,
            RevisitState::Due {
                at: "1970-01-01T00:00:10Z".to_string(),
                overdue_seconds: 0,
            }
        );
        assert_eq!(
            workspace
                .project("CUT", request(WhyTraversal::CutBounded, 10))
                .expect("cut")
                .verdict,
            WorkVerdict::CloseableCut
        );
        assert_eq!(
            workspace
                .project("DROPPED", request(WhyTraversal::CutBounded, 10))
                .expect("terminal")
                .verdict,
            WorkVerdict::Terminal(Status::Dropped)
        );
        let done = workspace
            .project("DONE", request(WhyTraversal::CutBounded, 10))
            .expect("done task");
        assert_eq!(done.verdict, WorkVerdict::Terminal(Status::Done));
        assert!(done.blockers.is_empty());
        let active = workspace
            .project("ACTIVE", request(WhyTraversal::CutBounded, 10))
            .expect("active task");
        assert_eq!(active.verdict, WorkVerdict::Ready);
        assert!(active.blockers.is_empty());
        assert_eq!(
            workspace
                .project("EMPTY-CUT", request(WhyTraversal::CutBounded, 10))
                .expect("empty cut")
                .verdict,
            WorkVerdict::UnblockedCut
        );
        assert_eq!(parse_work_as_of("10", 99), Some(10));
        assert_eq!(parse_work_as_of("1970-01-01T00:00:10Z", 99), Some(10));
        assert_eq!(parse_work_as_of("", 99), Some(99));
        assert_eq!(parse_work_as_of(" 10 ", 99), Some(10));
        assert_eq!(parse_work_as_of("tomorrow", 99), None);
    }

    #[test]
    fn revisit_schedule_retains_invalid_placement_and_extreme_clocks() {
        let mut parked = task("PARKED", Status::Blocked, &[], &[]);
        parked.revisit_at = Some("1970-01-01T00:00:10Z".to_string());
        let mut future = task("FUTURE", Status::Blocked, &[], &[]);
        future.revisit_at = Some("1970-01-01T00:00:20Z".to_string());
        let mut invalid = task("INVALID", Status::Blocked, &[], &[]);
        invalid.revisit_at = Some("tomorrow".to_string());
        let mut misplaced = task("MISPLACED", Status::Todo, &[], &[]);
        misplaced.revisit_at = Some("1970-01-01T00:00:10Z".to_string());
        let workspace = workspace(vec![parked, future, invalid, misplaced]);
        assert_eq!(workspace.input().sha256, "input");

        assert_eq!(
            workspace
                .project("INVALID", request(WhyTraversal::CutBounded, 10))
                .expect("invalid revisit task")
                .revisit,
            RevisitState::Invalid {
                at: "tomorrow".to_string(),
            }
        );
        assert_eq!(
            workspace
                .project("MISPLACED", request(WhyTraversal::CutBounded, 10))
                .expect("misplaced revisit task")
                .revisit,
            RevisitState::NotParked {
                at: "1970-01-01T00:00:10Z".to_string(),
            }
        );

        assert_eq!(
            workspace
                .project("PARKED", request(WhyTraversal::CutBounded, i64::MIN))
                .expect("minimum clock")
                .revisit,
            RevisitState::Waiting {
                at: "1970-01-01T00:00:10Z".to_string(),
                seconds_until: i64::MAX,
            }
        );
        assert_eq!(
            workspace
                .project("PARKED", request(WhyTraversal::CutBounded, i64::MAX))
                .expect("maximum clock")
                .revisit,
            RevisitState::Due {
                at: "1970-01-01T00:00:10Z".to_string(),
                overdue_seconds: i64::MAX - 10,
            }
        );

        let schedule = workspace.revisit_schedule(10);
        assert_eq!(schedule.due.len(), 1);
        assert_eq!(schedule.waiting.len(), 1);
        assert_eq!(schedule.due[0].id, "PARKED");
        assert_eq!(schedule.due[0].overdue_seconds, 0);
        assert_eq!(schedule.waiting[0].id, "FUTURE");
        assert_eq!(schedule.waiting[0].overdue_seconds, -10);
    }

    #[test]
    fn revisit_schedule_orders_overdue_then_identity_on_both_sides_of_now() {
        let mut due_later = task("DUE-B", Status::Blocked, &[], &[]);
        due_later.revisit_at = Some("1970-01-01T00:00:05Z".into());
        let mut due_tie = task("DUE-A", Status::Blocked, &[], &[]);
        due_tie.revisit_at = Some("1970-01-01T00:00:05Z".into());
        let mut due_recent = task("DUE-C", Status::Blocked, &[], &[]);
        due_recent.revisit_at = Some("1970-01-01T00:00:09Z".into());
        let mut waiting_far = task("WAIT-B", Status::Blocked, &[], &[]);
        waiting_far.revisit_at = Some("1970-01-01T00:00:20Z".into());
        let mut waiting_tie = task("WAIT-A", Status::Blocked, &[], &[]);
        waiting_tie.revisit_at = Some("1970-01-01T00:00:20Z".into());
        let mut waiting_near = task("WAIT-C", Status::Blocked, &[], &[]);
        waiting_near.revisit_at = Some("1970-01-01T00:00:11Z".into());
        let schedule = workspace(vec![
            waiting_far,
            due_recent,
            due_later,
            waiting_near,
            due_tie,
            waiting_tie,
        ])
        .revisit_schedule(10);

        assert_eq!(
            schedule
                .due
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["DUE-A", "DUE-B", "DUE-C"]
        );
        assert_eq!(
            schedule
                .waiting
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["WAIT-C", "WAIT-A", "WAIT-B"]
        );
    }
}
