//! The task model. These Rust types are the single source of truth: the JSON
//! Schema in `schemas/v1/task.json` is *derived* from them (`cargo xtask
//! gen-schema`), and CI fails if the committed schema and the types disagree.
//! Prose never becomes a regenerable source of truth.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lifecycle of a task. `Done` is the only status that requires evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not started. Eligible to be worked once its blockers clear.
    Todo,
    /// Actively being worked.
    InProgress,
    /// Explicitly parked for a reason outside the dependency graph.
    Blocked,
    /// Complete. Requires at least one `evidence` entry.
    Done,
    /// Abandoned. Kept for the record rather than deleted.
    Dropped,
}

impl std::fmt::Display for Status {
    /// Renders as the serialised (`snake_case`) form. Diagnostics must show the
    /// value the user typed, not the Rust variant name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::Blocked => "blocked",
            Status::Done => "done",
            Status::Dropped => "dropped",
        };
        f.write_str(s)
    }
}

impl Status {
    /// A task that will never be worked again.
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Done | Status::Dropped)
    }

    /// Only `Done` satisfies a dependency. `Dropped` does not, a dropped
    /// blocker means its dependents can never legitimately start.
    pub fn satisfies_dependency(self) -> bool {
        self == Status::Done
    }

    /// Whether a task in this status is available to be picked up.
    ///
    /// `Blocked` is not: it means *explicitly parked for a reason outside the
    /// dependency graph*, so it is workable by no one until a human unparks it.
    /// Answering "what can I work on now" with a task whose status is literally
    /// `blocked` is the tool contradicting itself.
    pub fn is_workable(self) -> bool {
        matches!(self, Status::Todo | Status::InProgress)
    }
}

// Generated via `str_enum!` so `EvidenceKind::ALL` (which `facts.json` reads)
// can never omit a variant, and `as_str` agrees with the `snake_case` serde name
// by construction. The doc comment below is the schema description, kept free of
// codegen detail.
crate::str_enum! {
    /// What kind of proof closes a task.
    ///
    /// The point of typing this is that "I finished it" is not evidence. A commit,
    /// a URL, a committed artefact, a named test, or a dated note is.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum EvidenceKind {
        /// A git commit SHA in this repository.
        Commit = "commit",
        /// An external URL (a filing receipt, a regulator page, a vendor quote).
        Url = "url",
        /// A repo-relative path to a committed artefact (a saved PDF, a transcript).
        File = "file",
        /// A named test that must pass.
        Test = "test",
        /// A dated human note, the weakest kind. Use when nothing better exists,
        /// e.g. the outcome of a phone call.
        Note = "note",
    }
}

/// A single piece of proof attached to a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// How to interpret `reference`.
    pub kind: EvidenceKind,
    /// The pointer: a commit SHA, a URL, a repo-relative path, a test name, or
    /// the note text itself.
    #[serde(rename = "ref")]
    pub reference: String,
    /// ISO-8601 date the evidence was captured. Required for `note`, because an
    /// undated note is a rumour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured: Option<String>,
}

/// One unit of work. Serialised as YAML frontmatter in a markdown file under
/// the configured tasks glob; the markdown body is free prose and is not parsed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Stable identifier, unique across the plan. Convention: `P<phase>-<nnn>`.
    pub id: String,
    /// One line. What done looks like, not what the work is.
    pub title: String,
    /// Phase number. Used for rollups and ordering, not for gating.
    pub phase: u32,
    /// Lifecycle status.
    pub status: Status,
    /// Who owns it. Absent means unassigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Estimated effort in days. Typed so phases can be rolled up. Non-negative:
    /// a negative estimate is meaningless and would corrupt a rollup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0))]
    pub effort_days: Option<f64>,
    /// Task IDs that must be `done` before this one can start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// Proof of completion. Must be non-empty when `status: done`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// Repo-relative globs naming the files this task is responsible for.
    ///
    /// This is what lets `aplan verify` ask whether a cited `commit` actually
    /// touches anything the task claims (`APLAN012`, L2 "Relevant"), without it,
    /// a commit SHA is a citation no one can bind to the task (HLD §5.2). Empty
    /// means relevance is unverifiable, not that everything is fair game.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Anchors this task resolves, as `path/to/doc.md#anchor`. Closing the task
    /// obliges you to update that anchor, this is the link that stops a
    /// research document and a plan from silently diverging.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<String>,
    /// Content fingerprints of `traces` anchors, captured when the task was closed
    /// (`aplan close`, with `detect_drift` on). Keyed by the full trace string.
    /// `APLAN018` fires when a `done` task's traced section no longer matches its
    /// stamp. Absent means unverifiable, not a violation (zero false positives).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub trace_fingerprints: std::collections::BTreeMap<String, String>,
    /// Free-form labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Task {
    /// A task is *ready* when its status is workable and every blocker is `done`.
    ///
    /// Note the status test is [`Status::is_workable`], not `!is_terminal`: a
    /// `blocked` task has no unmet *graph* blockers, so a `!is_terminal` test
    /// reported it as ready, advertising work the plan explicitly parked.
    pub fn is_ready(&self, is_done: impl Fn(&str) -> bool) -> bool {
        self.status.is_workable() && self.blocked_by.iter().all(|b| is_done(b))
    }

    /// Whether this task is a *milestone*: a checkpoint over other tasks rather
    /// than a unit of work. A milestone is `blocked_by` the tasks in its cut, so
    /// it cannot be `done` while any listed member is undone (`APLAN007`), and its
    /// body carries the cut's narrative. The `milestone` tag is the marker; the
    /// work-views (`ready`, `critical_path`, `summary`, and the `APLAN009` effort
    /// warning) exclude it, because a checkpoint is not itself workable, weighs
    /// nothing, and is not on the plan's spine. `APLAN021` keeps the tag honest (a
    /// milestone must have members and no effort or paths). No new field or edge
    /// type: a milestone is an ordinary task.
    pub fn is_milestone(&self) -> bool {
        self.tags.iter().any(|t| t == "milestone")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `ready` and `why` both announced an explicitly parked task
    /// (`status: blocked`) as ready, because the test was `!is_terminal` and a
    /// parked task has no unmet graph blockers.
    #[test]
    fn parked_task_is_never_ready() {
        let parked = Task {
            id: "X".into(),
            title: "parked".into(),
            phase: 0,
            status: Status::Blocked,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
        };
        assert!(
            !parked.is_ready(|_| true),
            "a `blocked` task must never be reported as ready"
        );
        for s in [Status::Todo, Status::InProgress] {
            let mut t = parked.clone();
            t.status = s;
            assert!(t.is_ready(|_| true), "{s} with no blockers is ready");
        }
        for s in [Status::Done, Status::Dropped] {
            let mut t = parked.clone();
            t.status = s;
            assert!(!t.is_ready(|_| true), "{s} is terminal, not ready");
        }
    }

    #[test]
    fn is_milestone_reads_the_milestone_tag() {
        let mut t = Task {
            id: "M".into(),
            title: "cut".into(),
            phase: 0,
            status: Status::Todo,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
        };
        assert!(!t.is_milestone(), "no tags: not a milestone");
        t.tags = vec!["cli".into(), "milestone".into()];
        assert!(t.is_milestone(), "the `milestone` tag marks a milestone");
        t.tags = vec!["milestones".into()];
        assert!(!t.is_milestone(), "only the exact `milestone` tag counts");
    }
}
