//! Completion-time and historical-provenance domain types (`A2-108`, `A2-110`,
//! ADR-0074).
//!
//! These types are shared across the Git history walker (`aplan-evidence`),
//! the application query layer (`aplan-app`), and all presentation surfaces
//! (CLI, MCP, TUI) so provenance, history policy, and scoped completeness never
//! drift between layers.

use std::collections::BTreeMap;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::task::ClosedAtSource;

crate::str_enum! {
    /// How a completion-history query uses Git history for legacy `done` tasks
    /// that lack a persisted `closed_at` (`ADR-0074`).
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum HistoryPolicy {
        /// Consult Git history only when at least one in-scope `done` task
        /// lacks `closed_at`; return best-effort results with explicit
        /// completeness status if history is incomplete or unavailable.
        #[default]
        Auto = "auto",
        /// Never consult Git history. Legacy `done` tasks without `closed_at`
        /// remain `unknown`.
        Never = "never",
        /// Require complete reachable history from `basis_revision` whenever
        /// any in-scope `done` task lacks `closed_at`; refuse with a typed
        /// error rather than returning a partial historical answer.
        Required = "required",
    }
}

crate::str_enum! {
    /// Non-overlapping provenance discriminator for a task's completion instant
    /// (`ADR-0074`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CompletionTimeSource {
        /// Exact persisted `closed_at` claim on the task, with durable
        /// `local` vs. `imported` origin.
        Recorded = "recorded",
        /// Best-effort selected committed observation of a transition to
        /// `done` in reachable Git history; never persisted as `closed_at`.
        HistoryFirstSeen = "history_first_seen",
        /// No defensible completion instant is available.
        Unknown = "unknown",
    }
}

crate::str_enum! {
    /// Completeness of a completion-history pass, always scoped to history
    /// reachable from the named `basis_revision` (`ADR-0074`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum HistoryCompleteness {
        /// All in-scope tasks resolved from recorded claims and/or exhaustive
        /// reachable history from `basis_revision`.
        Complete = "complete",
        /// `HistoryPolicy::Never` skipped Git inference while legacy tasks were
        /// present.
        HistorySkipped = "history_skipped",
        /// Traversal hit a shallow-clone graft boundary before resolving all
        /// legacy tasks.
        IncompleteShallow = "incomplete_shallow",
        /// Traversal hit an explicit commit, byte, or duration budget before
        /// resolving all legacy tasks.
        IncompleteBounded = "incomplete_bounded",
        /// Unparseable historical task shapes or missing/corrupt repository
        /// objects prevented full observation.
        IncompleteUnobservable = "incomplete_unobservable",
        /// No Git repository or no commits exist at `basis_revision`.
        Unavailable = "unavailable",
    }
}

/// Resolved completion instant and provenance for one `done` task (`ADR-0074`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompletionTime {
    /// Canonical RFC 3339 UTC whole-second instant, or `None` when `unknown`.
    pub instant: Option<String>,
    /// Unix epoch seconds corresponding to `instant`, or `None` when `unknown`.
    pub epoch_seconds: Option<i64>,
    /// Provenance discriminator (`recorded`, `history_first_seen`, or `unknown`).
    pub source: CompletionTimeSource,
    /// Durable origin (`local` or `imported`) when `source` is `recorded`;
    /// `None` otherwise.
    pub recorded_origin: Option<ClosedAtSource>,
    /// Full commit OID when `source` is `history_first_seen`; `None` otherwise.
    pub commit: Option<String>,
    /// Number of incomparable minimal transition candidates on parallel DAG
    /// branches when > 1; `0` when linear, recorded, or unknown.
    pub incomparable_candidates: usize,
}

impl CompletionTime {
    /// Construct a `recorded` completion time from a canonical `closed_at`
    /// instant and durable origin.
    #[must_use]
    pub fn recorded(instant: String, epoch_seconds: i64, origin: ClosedAtSource) -> Self {
        Self {
            instant: Some(instant),
            epoch_seconds: Some(epoch_seconds),
            source: CompletionTimeSource::Recorded,
            recorded_origin: Some(origin),
            commit: None,
            incomparable_candidates: 0,
        }
    }

    /// Construct a `history_first_seen` completion time from a selected commit.
    #[must_use]
    pub fn history_first_seen(
        instant: String,
        epoch_seconds: i64,
        commit: String,
        incomparable_candidates: usize,
    ) -> Self {
        Self {
            instant: Some(instant),
            epoch_seconds: Some(epoch_seconds),
            source: CompletionTimeSource::HistoryFirstSeen,
            recorded_origin: None,
            commit: Some(commit),
            incomparable_candidates,
        }
    }

    /// Construct an `unknown` completion time.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            instant: None,
            epoch_seconds: None,
            source: CompletionTimeSource::Unknown,
            recorded_origin: None,
            commit: None,
            incomparable_candidates: 0,
        }
    }
}

/// Explicit work, memory, and latency bounds for one Git completion-history
/// pass (`A2-110`, ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLimits {
    /// Maximum commits inspected in one pass.
    pub max_commits: usize,
    /// Maximum byte size of any single historical task blob parsed.
    pub max_blob_bytes: usize,
    /// Maximum cumulative historical blob bytes parsed across the pass.
    pub max_total_blob_bytes: usize,
    /// Optional wall-clock duration budget for the pass.
    pub max_duration: Option<Duration>,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            max_commits: 10_000,
            max_blob_bytes: 1_048_576,
            max_total_blob_bytes: 64 * 1024 * 1024,
            max_duration: None,
        }
    }
}

/// Output of one bounded completion-history resolution pass (`A2-110`,
/// ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionHistoryReport {
    /// Named basis revision to which completeness is scoped (full commit OID,
    /// `working_tree@<oid>`, or `unversioned`).
    pub basis_revision: String,
    /// Policy requested by the caller.
    pub policy: HistoryPolicy,
    /// Scoped completeness of this pass.
    pub completeness: HistoryCompleteness,
    /// Explicit human-readable limitation reasons when `completeness != Complete`.
    pub limitations: Vec<String>,
    /// Number of commits inspected during the single history walk.
    pub commits_walked: usize,
    /// Resolved completion instant and provenance per requested task id.
    pub resolved: BTreeMap<String, CompletionTime>,
}

/// Typed failure returned when completion-history resolution cannot satisfy
/// the requested policy or basis revision (`A2-110`, ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompletionHistoryError {
    /// The caller named a basis revision that does not resolve in the repository.
    #[error("basis revision `{revision}` could not be resolved: {detail}")]
    InvalidRevision { revision: String, detail: String },
    /// `HistoryPolicy::Required` was requested, but Git history is unavailable.
    #[error("completion history is required, but Git history is unavailable: {detail}")]
    Unavailable { detail: String },
    /// `HistoryPolicy::Required` was requested, but reachable history from
    /// `basis_revision` was not complete.
    #[error(
        "completion history is required, but reachable history from `{basis_revision}` is \
         `{completeness}`: {detail}"
    )]
    Incomplete {
        basis_revision: String,
        completeness: HistoryCompleteness,
        detail: String,
    },
}

impl std::fmt::Display for HistoryCompleteness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::fmt::Display for HistoryPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::fmt::Display for CompletionTimeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Selector dimensions applied to a completed-task history query (`A2-111`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AppliedCompletedSelector {
    pub cuts: Vec<String>,
    pub phases: Vec<u32>,
    pub owners: Vec<String>,
    pub tags: Vec<String>,
}

/// Normalized request parameters echoed in [`CompletedResponse`] (`A2-111`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AppliedCompletedRequest {
    pub since: Option<String>,
    pub until: Option<String>,
    pub oldest_first: bool,
    pub history: HistoryPolicy,
    pub limit: Option<usize>,
    pub selector: AppliedCompletedSelector,
}

/// One completed work task row in [`CompletedResponse`] (`A2-111`, ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompletedTaskRow {
    pub id: String,
    pub title: String,
    pub phase: u32,
    pub owner: Option<String>,
    pub cut: Option<String>,
    pub milestone: Option<String>,
    pub tags: Vec<String>,
    /// Canonical RFC 3339 UTC whole-second instant, or `None` when `unknown`.
    pub instant: Option<String>,
    /// Non-overlapping provenance discriminator.
    pub source: CompletionTimeSource,
    /// Durable origin (`local` or `imported`) when `source == recorded`.
    pub recorded_origin: Option<ClosedAtSource>,
    /// Full commit OID when `source == history_first_seen`.
    pub commit: Option<String>,
    /// Number of incomparable minimal transition candidates when > 1, else 0.
    pub incomparable_candidates: usize,
}

/// Stable typed response shared across CLI, MCP, and TUI completion-history
/// views (`A2-111`, ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompletedResponse {
    pub schema: String,
    pub input_identity: String,
    pub basis_revision: String,
    pub completeness: HistoryCompleteness,
    pub limitations: Vec<String>,
    pub commits_walked: usize,
    pub applied: AppliedCompletedRequest,
    pub total_matched: usize,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub items: Vec<CompletedTaskRow>,
}
