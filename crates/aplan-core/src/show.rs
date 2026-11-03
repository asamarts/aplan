//! One task, whole, with what the file cannot say (`A1-118`).
//!
//! There was no answer to "show me this task". `why` gives blockers and nothing
//! else, `tasks --format json` omits `paths`, `traces` and `evidence` entirely,
//! and `cat` gives the file, which cannot know whether its own citations still
//! resolve. So the two fields carrying this tool's whole argument -- the evidence
//! that closed a task and the trace that says what caused it -- were reachable
//! only by opening the file by hand, in a repository whose premise is that the
//! tool owns those files (pre-public adoption evidence item 6, on the read surface).
//!
//! What this adds to `cat` is **resolution**: the cut and whether the task is
//! ready come from the graph, what it releases comes from the graph's other
//! direction, and the findings come from the rule engine and the evidence ladder
//! filtered to one id. A closed task with a renamed test and a drifted trace
//! reads as perfectly healthy in its own file, and the whole point of this
//! project is that it is not.
//!
//! The projection lives here rather than in the renderer for the reason
//! [`crate::TaskExport`] does: two surfaces serialise it (the CLI's
//! `--format json` and the MCP tool), and only one renders it as text. A view
//! assembled twice is two answers that agree until they do not (`A1-104`).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::Repair;
use crate::export::TaskExport;
use crate::graph::Graph;
use crate::task::{Evidence, Task, VersionImpact};
use crate::{ClaimRecord, ClaimScope, ClaimSnapshot};

/// One task and everything resolved about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Show {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: u32,
    pub owner: Option<String>,
    /// Ephemeral local holder, distinct from durable `owner` responsibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<ClaimRecord>,
    /// Whole seconds the snapshot had observed the claim to be held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_age_seconds: Option<i64>,
    /// Coordination boundary for `claim`, including an explicit degraded state.
    pub coordination: ClaimScope,
    pub effort_days: Option<f64>,
    pub tags: Vec<String>,
    /// The cut it belongs to, resolved from the graph rather than the file.
    pub cut: Option<String>,
    pub milestone: Option<String>,
    /// Whether every blocker is done, the answer `ready` gives.
    pub ready: bool,
    /// Whether the task is work somebody could pick up, so a reader is not told
    /// "ready: no" about a checkpoint or a finished task (`A1-105`).
    pub workable: bool,
    pub paths: Vec<String>,
    pub traces: Vec<String>,
    pub trace_fingerprints: BTreeMap<String, String>,
    pub evidence: Vec<Cited>,
    pub evidence_fingerprints: BTreeMap<String, String>,
    /// What it waits on, and what waits on it: the graph in both directions,
    /// where the file has only one.
    pub waits_on: Vec<Related>,
    pub releases: Vec<Related>,
    /// Advisory ordering, ratification and containment are not dependency
    /// edges, but omitting them would make this projection less complete than
    /// the task it represents.
    pub prefer_after: Vec<Related>,
    pub ratified_by: Vec<Related>,
    pub contains: Vec<Related>,
    /// Every finding currently attached to this id.
    pub findings: Vec<Finding>,
    /// Whether the evidence ladder ran. `false` under `--no-resolve`, and the
    /// view says so rather than showing an unresolved task as a clean one.
    pub resolved: bool,
    /// Freshness of the evidence answer behind `resolved`.
    ///
    /// `resolved` remains for consumers of the previous schema and is true
    /// exactly when this is [`EvidenceObservationState::Current`].
    pub evidence_observation: EvidenceObservation,
    /// The body, when the caller asked for it.
    pub body: Option<String>,
    /// When a parked task is worth looking at again (`A1-264`).
    ///
    /// Here because this view is what a reader opens to see one task whole, and
    /// a field that exists on the task and not in the answer is a field they
    /// have to open the file for -- which is what the view exists to avoid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revisit_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_impact: Option<VersionImpact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_impact_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// One citation, as the file states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cited {
    pub kind: String,
    pub reference: String,
    pub captured: Option<String>,
    /// The observation used to judge this exact citation.
    pub observation: EvidenceObservation,
}

/// Whether an evidence answer can be applied to the inputs being displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceObservationState {
    Current,
    Stale,
    NotObserved,
    RefreshFailed,
    Incompatible,
    Untrusted,
    Fault,
    Unsupported,
}

/// Scope over which an evidence answer was derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceObservationScope {
    WholePlan,
    SelectedTasks,
    SelectedProviders,
}

impl EvidenceObservationScope {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::WholePlan => "whole_plan",
            Self::SelectedTasks => "selected_tasks",
            Self::SelectedProviders => "selected_providers",
        }
    }
}

/// Where configured-provider facts came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceProviderSource {
    Direct,
    Receipt,
    IncrementalRefresh,
    Deferred,
    None,
}

impl EvidenceProviderSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Receipt => "receipt",
            Self::IncrementalRefresh => "incremental_refresh",
            Self::Deferred => "deferred",
            Self::None => "none",
        }
    }
}

/// Whether the provider observation covers the answer it accompanies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceObservationCompleteness {
    Complete,
    Partial,
    Cancelled,
}

/// Deterministic explanation of one explicit incremental receipt refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceRefreshSummary {
    pub mode: String,
    pub base: String,
    pub head: String,
    pub changed_paths: usize,
    pub reused_providers: usize,
    pub reused: Vec<String>,
    pub refreshed_providers: usize,
    pub refreshed: Vec<String>,
    pub selected_tests: usize,
    pub widened_providers: usize,
    pub widened: Vec<String>,
    pub unavailable_providers: usize,
    pub unavailable: Vec<String>,
    pub shadow_compared: bool,
}

impl EvidenceObservationCompleteness {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Cancelled => "cancelled",
        }
    }
}

impl EvidenceObservationState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Stale => "stale",
            Self::NotObserved => "not observed",
            Self::RefreshFailed => "refresh failed",
            Self::Incompatible => "incompatible",
            Self::Untrusted => "untrusted",
            Self::Fault => "fault",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Freshness and provenance for an aggregate answer or one citation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceObservation {
    pub state: EvidenceObservationState,
    pub scope: EvidenceObservationScope,
    pub provider_source: EvidenceProviderSource,
    pub completeness: EvidenceObservationCompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at_epoch_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<EvidenceRefreshSummary>,
}

impl EvidenceObservation {
    #[must_use]
    pub fn current() -> Self {
        Self {
            state: EvidenceObservationState::Current,
            scope: EvidenceObservationScope::SelectedTasks,
            provider_source: EvidenceProviderSource::None,
            completeness: EvidenceObservationCompleteness::Complete,
            identity: None,
            resolution_identity: None,
            observed_at_epoch_seconds: None,
            resolver: None,
            strength: None,
            outcome: None,
            detail: None,
            refresh: None,
        }
    }

    #[must_use]
    pub fn not_observed(detail: impl Into<String>) -> Self {
        Self {
            state: EvidenceObservationState::NotObserved,
            scope: EvidenceObservationScope::SelectedTasks,
            provider_source: EvidenceProviderSource::Deferred,
            completeness: EvidenceObservationCompleteness::Partial,
            identity: None,
            resolution_identity: None,
            observed_at_epoch_seconds: None,
            resolver: None,
            strength: None,
            outcome: None,
            detail: Some(detail.into()),
            refresh: None,
        }
    }
}

/// A task on the other end of an edge, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Related {
    pub id: String,
    pub status: String,
    pub availability: RelatedAvailability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Whether the task at the other end of a declared relationship was loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelatedAvailability {
    Loaded,
    Missing,
    Unloadable,
}

/// A finding, flattened: the fields a reader acts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub fix: Option<String>,
    /// An exact edit the tool computed, distinct from prose advice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair: Option<Repair>,
}

/// The coherent derived inputs used to project one task.
///
/// Keeping these sources together makes a caller supply the same graph, row
/// projection, findings, unloadable-task catalogue and claim snapshot instead
/// of growing a positional constructor each time the report gains context.
#[derive(Debug, Clone, Copy)]
pub struct ShowSources<'a> {
    pub graph: &'a Graph<'a>,
    pub rows: &'a TaskExport,
    pub diagnostics: &'a [Diagnostic],
    pub unloadable: &'a BTreeMap<String, std::path::PathBuf>,
    pub claims: &'a ClaimSnapshot,
}

fn task_row<'a>(rows: &'a TaskExport, id: &str) -> Option<&'a crate::export::TaskRow> {
    // `Graph::task_rows` is ordered by the graph's `BTreeMap`. A complete TUI
    // snapshot asks for every inspector, so a linear search here made that
    // otherwise shared projection quadratic (`A1-447`).
    rows.tasks
        .binary_search_by(|row| row.id.as_str().cmp(id))
        .ok()
        .map(|index| &rows.tasks[index])
        // `ShowSources` is public and an adapter may provide its own row
        // ordering. Preserve that accepted input while keeping the canonical
        // graph-derived path logarithmic.
        .or_else(|| rows.tasks.iter().find(|row| row.id == id))
}

/// The plan reduced to one task, for the evidence ladder (`A1-118`).
///
/// `show` answers about a single task and the ladder is the expensive half of
/// the answer: resolving the whole plan to report on one task cost a full
/// `verify` -- 1.77s on this repository's own plan, the same as `verify` itself,
/// and it scales with every other task's citations rather than with this one's.
///
/// Sound because the ladder is per-task by construction: every rung asks about
/// *this* task's citations against the repository (`APLAN011`-`APLAN016`), or
/// about them against *this* task's `paths` (`APLAN012`, `APLAN019`, `APLAN024`,
/// `APLAN025`). The rules that need the other tasks are the hermetic ones, and
/// they run over the whole plan, as they must.
#[must_use]
pub fn only(plan: &crate::load::Plan, id: &str) -> crate::load::Plan {
    crate::load::Plan {
        tasks: plan
            .tasks
            .iter()
            .filter(|lt| lt.task.id == id)
            .map(|lt| crate::load::LoadedTask {
                task: lt.task.clone(),
                path: lt.path.clone(),
                fields: lt.fields.clone(),
                criteria: lt.criteria,
            })
            .collect(),
        load_diagnostics: Vec::new(),
        unloadable: std::collections::BTreeMap::new(),
    }
}

impl Show {
    /// Build the view from what each source already knows.
    ///
    /// `diagnostics` is whatever the caller has run: the hermetic rules alone
    /// under `--no-resolve`, or those plus the ladder. `resolved` says which,
    /// because "nothing to report" and "nothing was checked" are different
    /// answers and only the caller knows which one it has.
    #[must_use]
    pub fn assemble(
        sources: ShowSources<'_>,
        id: &str,
        resolved: bool,
        body: Option<String>,
    ) -> Option<Self> {
        let ShowSources {
            graph,
            rows,
            diagnostics,
            unloadable,
            claims,
        } = sources;
        let task: &Task = graph.get(id)?;
        let row = task_row(rows, id);
        let related = |other: &str| {
            if let Some(task) = graph.get(other) {
                Related {
                    id: other.to_string(),
                    status: task.status.to_string(),
                    availability: RelatedAvailability::Loaded,
                    source: None,
                }
            } else if let Some(path) = unloadable.get(other) {
                Related {
                    id: other.to_string(),
                    status: "unknown".to_string(),
                    availability: RelatedAvailability::Unloadable,
                    source: Some(path.display().to_string()),
                }
            } else {
                Related {
                    id: other.to_string(),
                    status: "unknown".to_string(),
                    availability: RelatedAvailability::Missing,
                    source: None,
                }
            }
        };
        Some(Self {
            schema: "aplan/show/v3".to_string(),
            id: task.id.clone(),
            title: task.title.clone(),
            status: task.status.to_string(),
            phase: task.phase,
            owner: task.owner.clone(),
            claim: claims.claim(id).cloned(),
            claim_age_seconds: claims.age_seconds(id),
            coordination: claims.scope.clone(),
            effort_days: task.effort_days,
            tags: task.tags.clone(),
            cut: row.and_then(|r| r.cut.clone()),
            milestone: row.and_then(|r| r.milestone.clone()),
            ready: row.is_some_and(|r| r.ready),
            workable: task.status.is_workable() && !task.is_milestone(),
            revisit_at: task.revisit_at.clone(),
            paths: task.paths.clone(),
            traces: task.traces.clone(),
            trace_fingerprints: task.trace_fingerprints.clone(),
            evidence: task
                .evidence
                .iter()
                .map(|evidence| {
                    let mut cited = Cited::from(evidence);
                    if resolved {
                        cited.observation = EvidenceObservation::current();
                    }
                    cited
                })
                .collect(),
            evidence_fingerprints: task.evidence_fingerprints.clone(),
            waits_on: graph.blockers_of(id).into_iter().map(related).collect(),
            releases: graph
                .dependents_of(id)
                .into_iter()
                .map(|t| related(&t.id))
                .collect(),
            prefer_after: task
                .prefer_after
                .iter()
                .map(|other| related(other))
                .collect(),
            ratified_by: task
                .ratified_by
                .iter()
                .map(|other| related(other))
                .collect(),
            contains: task.contains.iter().map(|other| related(other)).collect(),
            findings: diagnostics
                .iter()
                .filter(|d| d.task.as_deref() == Some(id))
                .map(Finding::from)
                .collect(),
            resolved,
            evidence_observation: if resolved {
                EvidenceObservation::current()
            } else {
                EvidenceObservation::not_observed("evidence resolution was not requested")
            },
            body,
            version_impact: task.version_impact,
            version_impact_reason: task.version_impact_reason.clone(),
            version: task.version.clone(),
        })
    }
}

impl From<&Evidence> for Cited {
    fn from(e: &Evidence) -> Self {
        Self {
            kind: e.kind.as_str().to_string(),
            reference: e.reference.clone(),
            captured: e.captured.clone(),
            observation: EvidenceObservation::not_observed("no citation observation was attached"),
        }
    }
}

impl From<&Diagnostic> for Finding {
    fn from(d: &Diagnostic) -> Self {
        Self {
            code: d.code.to_string(),
            severity: d.severity.to_string(),
            message: d.message.clone(),
            fix: d.agent_instruction.clone(),
            repair: d.repair.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EvidenceObservationCompleteness, EvidenceObservationScope, EvidenceObservationState,
        EvidenceProviderSource,
    };

    #[test]
    fn every_observation_dimension_has_a_distinct_machine_label() {
        assert_eq!(EvidenceObservationScope::WholePlan.label(), "whole_plan");
        assert_eq!(
            EvidenceObservationScope::SelectedTasks.label(),
            "selected_tasks"
        );
        assert_eq!(
            EvidenceObservationScope::SelectedProviders.label(),
            "selected_providers"
        );
        assert_eq!(EvidenceProviderSource::Direct.label(), "direct");
        assert_eq!(EvidenceProviderSource::Receipt.label(), "receipt");
        assert_eq!(
            EvidenceProviderSource::IncrementalRefresh.label(),
            "incremental_refresh"
        );
        assert_eq!(EvidenceProviderSource::Deferred.label(), "deferred");
        assert_eq!(EvidenceProviderSource::None.label(), "none");
        assert_eq!(
            EvidenceObservationCompleteness::Complete.label(),
            "complete"
        );
        assert_eq!(EvidenceObservationCompleteness::Partial.label(), "partial");
        assert_eq!(
            EvidenceObservationCompleteness::Cancelled.label(),
            "cancelled"
        );
    }

    #[test]
    fn every_observation_state_has_a_distinct_human_label() {
        let labels = [
            (EvidenceObservationState::Current, "current"),
            (EvidenceObservationState::Stale, "stale"),
            (EvidenceObservationState::NotObserved, "not observed"),
            (EvidenceObservationState::RefreshFailed, "refresh failed"),
            (EvidenceObservationState::Incompatible, "incompatible"),
            (EvidenceObservationState::Untrusted, "untrusted"),
            (EvidenceObservationState::Fault, "fault"),
            (EvidenceObservationState::Unsupported, "unsupported"),
        ];
        for (state, expected) in labels {
            assert_eq!(state.label(), expected);
        }
    }
}
