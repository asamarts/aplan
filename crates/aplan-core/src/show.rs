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

use serde::Serialize;

use crate::diagnostic::Diagnostic;
use crate::export::TaskExport;
use crate::graph::Graph;
use crate::task::{Evidence, Task};

/// One task and everything resolved about it.
#[derive(Debug, Clone, Serialize)]
pub struct Show {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: u32,
    pub owner: Option<String>,
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
    pub evidence: Vec<Cited>,
    /// What it waits on, and what waits on it: the graph in both directions,
    /// where the file has only one.
    pub waits_on: Vec<Related>,
    pub releases: Vec<Related>,
    /// Every finding currently attached to this id.
    pub findings: Vec<Finding>,
    /// Whether the evidence ladder ran. `false` under `--no-resolve`, and the
    /// view says so rather than showing an unresolved task as a clean one.
    pub resolved: bool,
    /// The body, when the caller asked for it.
    pub body: Option<String>,
    /// When a parked task is worth looking at again (`A1-264`).
    ///
    /// Here because this view is what a reader opens to see one task whole, and
    /// a field that exists on the task and not in the answer is a field they
    /// have to open the file for -- which is what the view exists to avoid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revisit_at: Option<String>,
}

/// One citation, as the file states it.
#[derive(Debug, Clone, Serialize)]
pub struct Cited {
    pub kind: String,
    pub reference: String,
    pub captured: Option<String>,
}

/// A task on the other end of an edge, and where it stands.
#[derive(Debug, Clone, Serialize)]
pub struct Related {
    pub id: String,
    pub status: String,
}

/// A finding, flattened: the fields a reader acts on.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub fix: Option<String>,
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
        graph: &Graph<'_>,
        rows: &TaskExport,
        diagnostics: &[Diagnostic],
        id: &str,
        resolved: bool,
        body: Option<String>,
    ) -> Option<Self> {
        let task: &Task = graph.get(id)?;
        let row = rows.tasks.iter().find(|r| r.id == id);
        let status_of = |other: &str| {
            graph
                .get(other)
                .map_or_else(|| "unknown".to_string(), |t| t.status.to_string())
        };
        Some(Self {
            schema: "aplan/show/v1".to_string(),
            id: task.id.clone(),
            title: task.title.clone(),
            status: task.status.to_string(),
            phase: task.phase,
            owner: task.owner.clone(),
            effort_days: task.effort_days,
            tags: task.tags.clone(),
            cut: row.and_then(|r| r.cut.clone()),
            milestone: row.and_then(|r| r.milestone.clone()),
            ready: row.is_some_and(|r| r.ready),
            workable: task.status.is_workable() && !task.is_milestone(),
            revisit_at: task.revisit_at.clone(),
            paths: task.paths.clone(),
            traces: task.traces.clone(),
            evidence: task.evidence.iter().map(Cited::from).collect(),
            waits_on: graph
                .blockers_of(id)
                .into_iter()
                .map(|b| Related {
                    id: b.to_string(),
                    status: status_of(b),
                })
                .collect(),
            releases: graph
                .dependents_of(id)
                .into_iter()
                .map(|t| Related {
                    id: t.id.clone(),
                    status: t.status.to_string(),
                })
                .collect(),
            findings: diagnostics
                .iter()
                .filter(|d| d.task.as_deref() == Some(id))
                .map(Finding::from)
                .collect(),
            resolved,
            body,
        })
    }
}

impl From<&Evidence> for Cited {
    fn from(e: &Evidence) -> Self {
        Self {
            kind: e.kind.as_str().to_string(),
            reference: e.reference.clone(),
            captured: e.captured.clone(),
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
        }
    }
}
