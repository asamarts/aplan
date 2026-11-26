//! Shared application operation for completed-task history (`A2-111`,
//! ADR-0074).
//!
//! Owns filtering, time-bound normalization, deterministic non-causal sorting,
//! and snapshot-consistent continuation cursors so CLI, MCP (`A2-119`), and
//! TUI (`A2-112`) consume one concrete typed contract.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aplan_core::select::Selector;
use aplan_core::{
    AppliedCompletedRequest, AppliedCompletedSelector, CompletedResponse, CompletedTaskRow,
    CompletionHistoryError, CompletionTime, Graph, HistoryLimits, HistoryPolicy, Plan, Status,
    format_closed_at, normalize_closed_at,
};
use sha2::{Digest as _, Sha256};

use crate::PlanContext;

/// Identified workspace for querying completed-task history through the shared
/// [`completed_tasks`] operation (`A2-112`, ADR-0074).
#[derive(Debug, Clone)]
pub struct CompletedWorkspace {
    plan: Plan,
    input_identity: String,
    repo_root: PathBuf,
    plan_root_rel: PathBuf,
}

impl PartialEq for CompletedWorkspace {
    fn eq(&self, other: &Self) -> bool {
        self.input_identity == other.input_identity
            && self.repo_root == other.repo_root
            && self.plan_root_rel == other.plan_root_rel
            && self.plan.tasks.len() == other.plan.tasks.len()
    }
}

impl CompletedWorkspace {
    /// Build a completed-history workspace bound to one identified [`PlanContext`].
    #[must_use]
    pub fn from_context(context: &PlanContext) -> Self {
        let identity = context.identity();
        let repo_root = identity.repository_root.clone();
        let plan_root_rel = identity
            .plan_root
            .strip_prefix(&repo_root)
            .unwrap_or(&identity.plan_root)
            .to_path_buf();
        Self {
            plan: context.plan().clone(),
            input_identity: identity.sha256.clone(),
            repo_root,
            plan_root_rel,
        }
    }

    /// Build a completed-history workspace from explicit plan, identity, and
    /// repository paths (for tests and synthetic fixtures).
    #[must_use]
    pub fn from_parts(
        plan: Plan,
        input_identity: String,
        repo_root: PathBuf,
        plan_root_rel: PathBuf,
    ) -> Self {
        Self {
            plan,
            input_identity,
            repo_root,
            plan_root_rel,
        }
    }

    /// Inert empty workspace used during onboarding or synthetic snapshots with
    /// no completed work.
    #[must_use]
    pub fn empty(input_identity: String) -> Self {
        Self {
            plan: Plan {
                tasks: Vec::new(),
                load_diagnostics: Vec::new(),
                unloadable: BTreeMap::new(),
            },
            input_identity,
            repo_root: PathBuf::new(),
            plan_root_rel: PathBuf::from("aplan"),
        }
    }

    /// The identified snapshot digest bound to this workspace.
    #[must_use]
    pub fn input_identity(&self) -> &str {
        &self.input_identity
    }

    /// Execute one typed completed-task history query against this snapshot.
    ///
    /// # Errors
    /// Returns [`CompletedError`] for invalid limits/bounds, stale or invalid
    /// cursors, or `HistoryPolicy::Required` failures.
    pub fn query(&self, request: &CompletedRequest) -> Result<CompletedResponse, CompletedError> {
        completed_tasks(
            &self.plan,
            &self.input_identity,
            &self.repo_root,
            &self.plan_root_rel,
            request,
        )
    }
}

/// Maximum accepted continuation cursor length in bytes.
const MAX_CURSOR_BYTES: usize = 512;

/// Concrete typed request for completed-task history (`A2-111`, ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedRequest {
    /// Selector dimensions (`cut`, `phase`, `owner`, `tag`) applied before
    /// limiting.
    pub selector: Selector,
    /// Optional closed/inclusive lower bound (`since <= instant`). Accepts
    /// `YYYY-MM-DD` (normalized to `T00:00:00Z`) or RFC 3339.
    pub since: Option<String>,
    /// Optional closed/inclusive upper bound (`instant <= until`). Accepts
    /// `YYYY-MM-DD` (normalized to `T23:59:59Z`) or RFC 3339.
    pub until: Option<String>,
    /// Reverse chronological direction (`false` = newest-first default,
    /// `true` = oldest-first). Unknown instants always sort after known ones.
    pub oldest_first: bool,
    /// Git history policy for legacy `done` tasks without `closed_at`.
    pub history: HistoryPolicy,
    /// Maximum rows to return after filtering and cursor advancement. `Some(0)`
    /// is refused.
    pub limit: Option<usize>,
    /// Optional opaque continuation cursor returned by a previous page.
    pub cursor: Option<String>,
    /// Explicit basis revision (`--at <REV>`), or `None` for the working tree
    /// backed by `HEAD`.
    pub basis_revision: Option<String>,
    /// Explicit work/memory/latency bounds for the single Git history pass.
    pub limits: HistoryLimits,
}

impl Default for CompletedRequest {
    fn default() -> Self {
        Self {
            selector: Selector::default(),
            since: None,
            until: None,
            oldest_first: false,
            history: HistoryPolicy::Auto,
            limit: None,
            cursor: None,
            basis_revision: None,
            limits: HistoryLimits::default(),
        }
    }
}

/// Typed refusal for a completed-task history request (`A2-111`, ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompletedError {
    /// Zero is refused rather than silently becoming unlimited.
    #[error("`--limit` must be at least 1 (0 is not allowed)")]
    InvalidLimit,
    /// A `--since` or `--until` bound could not be parsed.
    #[error("invalid `{flag}` bound `{value}`: {reason}")]
    InvalidBound {
        flag: &'static str,
        value: String,
        reason: String,
    },
    /// `--since` is strictly later than `--until`.
    #[error("reversed time bounds: `--since` ({since}) is later than `--until` ({until})")]
    ReversedBounds { since: String, until: String },
    /// The continuation cursor is malformed, oversized, or belongs to a
    /// different query.
    #[error("invalid continuation cursor: {0}")]
    InvalidCursor(String),
    /// The plan snapshot or basis revision moved since the cursor was issued.
    #[error(
        "stale continuation cursor: plan input identity or basis revision changed \
         (expected `{expected}`, current `{actual}`); restart from page 1"
    )]
    StaleCursor { expected: String, actual: String },
    /// Underlying Git completion-history error (invalid revision or
    /// `HistoryPolicy::Required` failure).
    #[error("{0}")]
    History(#[from] CompletionHistoryError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCursor {
    input_identity: String,
    basis_revision: String,
    query_fingerprint: String,
    known: bool,
    epoch: Option<i64>,
    last_id: String,
}

/// Parse one inclusive time bound (`YYYY-MM-DD` or RFC 3339) into
/// `(epoch_seconds, canonical_utc_string)`.
fn parse_bound(
    flag: &'static str,
    raw: &str,
    is_upper: bool,
) -> Result<(i64, String), CompletedError> {
    let trimmed = raw.trim();
    if trimmed.len() == 10 && aplan_core::is_iso_date(trimmed) {
        let suffix = if is_upper { "T23:59:59Z" } else { "T00:00:00Z" };
        let canonical = format!("{trimmed}{suffix}");
        let epoch = aplan_core::parse_canonical_closed_at(&canonical).ok_or_else(|| {
            CompletedError::InvalidBound {
                flag,
                value: raw.to_string(),
                reason: "date is outside the valid UTC range (`1970-01-01`..=`9999-12-31`)"
                    .to_string(),
            }
        })?;
        return Ok((epoch, canonical));
    }
    let canonical =
        normalize_closed_at(trimmed).map_err(|reason| CompletedError::InvalidBound {
            flag,
            value: raw.to_string(),
            reason,
        })?;
    let epoch = aplan_core::parse_canonical_closed_at(&canonical).ok_or_else(|| {
        CompletedError::InvalidBound {
            flag,
            value: raw.to_string(),
            reason: "instant is outside the valid UTC range".to_string(),
        }
    })?;
    Ok((epoch, canonical))
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn query_fingerprint(applied: &AppliedCompletedRequest) -> String {
    let payload = serde_json::to_string(applied).unwrap_or_default();
    let digest = Sha256::digest(payload.as_bytes());
    let hex = hex_digest(&digest);
    hex[..16].to_string()
}

fn encode_cursor(
    input_identity: &str,
    basis_revision: &str,
    query_fp: &str,
    epoch: Option<i64>,
    last_id: &str,
) -> String {
    let known = if epoch.is_some() { "1" } else { "0" };
    let epoch_str = epoch.map_or_else(|| "-".to_string(), |e| e.to_string());
    format!("v1|{input_identity}|{basis_revision}|{query_fp}|{known}|{epoch_str}|{last_id}")
}

fn decode_cursor(raw: &str) -> Result<ParsedCursor, CompletedError> {
    if raw.is_empty() || raw.len() > MAX_CURSOR_BYTES {
        return Err(CompletedError::InvalidCursor(format!(
            "cursor length must be 1..={MAX_CURSOR_BYTES} bytes"
        )));
    }
    if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(CompletedError::InvalidCursor(
            "cursor contains forbidden control or whitespace characters".to_string(),
        ));
    }
    let parts: Vec<&str> = raw.split('|').collect();
    if parts.len() != 7 || parts[0] != "v1" {
        return Err(CompletedError::InvalidCursor(
            "expected `v1` cursor format".to_string(),
        ));
    }
    if parts[1].is_empty() || parts[2].is_empty() || parts[3].is_empty() {
        return Err(CompletedError::InvalidCursor(
            "cursor snapshot or query binding must not be empty".to_string(),
        ));
    }
    let known = match parts[4] {
        "1" => true,
        "0" => false,
        _ => {
            return Err(CompletedError::InvalidCursor(
                "invalid cursor known-instant marker".to_string(),
            ));
        }
    };
    let epoch = if known {
        let parsed = parts[5].parse::<i64>().map_err(|_| {
            CompletedError::InvalidCursor("invalid cursor epoch timestamp".to_string())
        })?;
        Some(parsed)
    } else {
        if parts[5] != "-" {
            return Err(CompletedError::InvalidCursor(
                "unknown cursor must carry `-` epoch".to_string(),
            ));
        }
        None
    };
    if parts[6].is_empty() {
        return Err(CompletedError::InvalidCursor(
            "cursor task id must not be empty".to_string(),
        ));
    }
    let canonical = encode_cursor(parts[1], parts[2], parts[3], epoch, parts[6]);
    if canonical != raw {
        return Err(CompletedError::InvalidCursor(
            "non-canonical cursor encoding".to_string(),
        ));
    }
    Ok(ParsedCursor {
        input_identity: parts[1].to_string(),
        basis_revision: parts[2].to_string(),
        query_fingerprint: parts[3].to_string(),
        known,
        epoch,
        last_id: parts[6].to_string(),
    })
}

/// Shorten an already-computed [`CompletedResponse`] to its first `keep` rows
/// (e.g., to stay within a transport byte cap) while keeping `applied`
/// unchanged and recomputing a valid `next_cursor` pointing at the new last
/// returned row (`A2-119`).
///
/// Returns `true` if `response.items` was shortened, or `false` if `keep == 0`
/// or `keep >= response.items.len()`.
pub fn truncate_completed_response(response: &mut CompletedResponse, keep: usize) -> bool {
    if keep == 0 || keep >= response.items.len() {
        return false;
    }
    response.items.truncate(keep);
    response.truncated = true;
    let query_fp = query_fingerprint(&response.applied);
    if let Some(last) = response.items.last() {
        let epoch = last
            .instant
            .as_deref()
            .and_then(aplan_core::parse_canonical_closed_at);
        response.next_cursor = Some(encode_cursor(
            &response.input_identity,
            &response.basis_revision,
            &query_fp,
            epoch,
            &last.id,
        ));
    }
    true
}

/// Compute a stable hex digest identifying a loaded plan's exact input files
/// (`A2-111`, ADR-0074).
///
/// Treats [`aplan_core::PlanFileContent::Bytes`] and
/// [`aplan_core::PlanFileContent::Sha256`] equivalently and sorts by absolute
/// path so CLI (`load_plan_identified`), MCP (`PlanStore`), and TUI snapshots
/// of the same working tree produce the exact same `input_identity` and
/// continuation cursors (`A2-119`).
#[must_use]
pub fn plan_inputs_identity(inputs: &[aplan_core::PlanFileInput]) -> String {
    let mut entries: Vec<(std::path::PathBuf, &aplan_core::PlanFileInput)> = inputs
        .iter()
        .map(|input| {
            let abs = std::path::absolute(&input.path).unwrap_or_else(|_| input.path.clone());
            (abs, input)
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (abs_path, input) in entries {
        hasher.update(abs_path.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        match &input.content {
            aplan_core::PlanFileContent::Bytes(bytes) => {
                let sha = hex_digest(&Sha256::digest(bytes));
                hasher.update(b"S");
                hasher.update(sha.as_bytes());
            }
            aplan_core::PlanFileContent::Sha256(sha) => {
                hasher.update(b"S");
                hasher.update(sha.as_bytes());
            }
            aplan_core::PlanFileContent::ReadError { message, .. } => {
                hasher.update(b"E");
                hasher.update(message.as_bytes());
            }
        }
        hasher.update(b"\0");
    }
    let digest = hasher.finalize();
    hex_digest(&digest)
}

/// Compare two `(epoch_seconds, task_id)` sort keys per `ADR-0074` section 8:
/// known instants ALWAYS sort before unknown instants in both newest-first and
/// oldest-first views, with `task_id ASC` as deterministic tie-break.
fn cmp_completion_key(
    left_epoch: Option<i64>,
    left_id: &str,
    right_epoch: Option<i64>,
    right_id: &str,
    oldest_first: bool,
) -> Ordering {
    match (left_epoch, right_epoch) {
        (Some(a), Some(b)) => {
            let time_ord = if oldest_first { a.cmp(&b) } else { b.cmp(&a) };
            time_ord.then_with(|| left_id.cmp(right_id))
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left_id.cmp(right_id),
    }
}

/// Execute the shared completed-task history query over `plan` (`A2-111`,
/// ADR-0074).
///
/// # Errors
/// Returns [`CompletedError`] for zero limits, invalid or reversed bounds,
/// malformed/stale cursors, or `HistoryPolicy::Required` failures.
#[allow(clippy::too_many_lines)]
pub fn completed_tasks(
    plan: &Plan,
    input_identity: &str,
    repo_root: &Path,
    plan_root_rel: &Path,
    request: &CompletedRequest,
) -> Result<CompletedResponse, CompletedError> {
    if request.limit == Some(0) {
        return Err(CompletedError::InvalidLimit);
    }
    let since = match request.since.as_deref() {
        Some(raw) => Some(parse_bound("--since", raw, false)?),
        None => None,
    };
    let until = match request.until.as_deref() {
        Some(raw) => Some(parse_bound("--until", raw, true)?),
        None => None,
    };
    if let (Some((s_epoch, s_str)), Some((u_epoch, u_str))) = (&since, &until)
        && s_epoch > u_epoch
    {
        return Err(CompletedError::ReversedBounds {
            since: s_str.clone(),
            until: u_str.clone(),
        });
    }

    let applied = AppliedCompletedRequest {
        since: since.as_ref().map(|(_, s)| s.clone()),
        until: until.as_ref().map(|(_, u)| u.clone()),
        oldest_first: request.oldest_first,
        history: request.history,
        limit: request.limit,
        selector: AppliedCompletedSelector {
            cuts: request.selector.cuts.clone(),
            phases: request.selector.phases.clone(),
            owners: request.selector.owners.clone(),
            tags: request.selector.tags.clone(),
        },
    };
    let query_fp = query_fingerprint(&applied);
    let parsed_cursor = match request.cursor.as_deref() {
        Some(raw) => Some(decode_cursor(raw)?),
        None => None,
    };
    if let Some(cursor) = &parsed_cursor {
        if cursor.input_identity != input_identity {
            return Err(CompletedError::StaleCursor {
                expected: cursor.input_identity.clone(),
                actual: input_identity.to_string(),
            });
        }
        if cursor.query_fingerprint != query_fp {
            return Err(CompletedError::InvalidCursor(
                "cursor does not match the requested filters, bounds, ordering, or limit"
                    .to_string(),
            ));
        }
    }

    // Build graph & cut membership map for selector matching and row metadata.
    let all_tasks: Vec<&aplan_core::Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(all_tasks.iter().copied());
    let roadmap = graph.roadmap();
    let mut task_cut: BTreeMap<&str, (String, String)> = BTreeMap::new();
    for cut in &roadmap.cuts {
        if cut.release || cut.borrows {
            continue;
        }
        for member in &cut.members {
            task_cut
                .entry(member.id.as_str())
                .or_insert_with(|| (cut.version.clone(), cut.milestone_id.clone()));
        }
    }

    // Keep only completed work tasks (exclude milestones and non-`done` tasks
    // including `dropped`) that match the requested selector.
    let eligible: Vec<&aplan_core::Task> = all_tasks
        .into_iter()
        .filter(|task| {
            let (cut, milestone) = task_cut
                .get(task.id.as_str())
                .map_or((None, None), |(c, m)| (Some(c.as_str()), Some(m.as_str())));
            task.status == Status::Done
                && !task.is_milestone()
                && request.selector.matches(task, cut, milestone)
        })
        .collect();

    let history_report = aplan_evidence::resolve_completion_times(
        repo_root,
        plan_root_rel,
        &eligible,
        request.basis_revision.as_deref(),
        request.history,
        &request.limits,
    )?;

    if let Some(cursor) = &parsed_cursor
        && cursor.basis_revision != history_report.basis_revision
    {
        return Err(CompletedError::StaleCursor {
            expected: cursor.basis_revision.clone(),
            actual: history_report.basis_revision,
        });
    }

    // Pair each eligible task with its resolved completion time and apply
    // inclusive time bounds (unknown instants NEVER pass a time predicate).
    let mut matched: Vec<(&aplan_core::Task, CompletionTime)> = Vec::with_capacity(eligible.len());
    for task in eligible {
        let ct = history_report
            .resolved
            .get(&task.id)
            .cloned()
            .unwrap_or_else(CompletionTime::unknown);
        if since.is_some() || until.is_some() {
            let Some(epoch) = ct.epoch_seconds else {
                continue;
            };
            if let Some((s_epoch, _)) = &since
                && epoch < *s_epoch
            {
                continue;
            }
            if let Some((u_epoch, _)) = &until
                && epoch > *u_epoch
            {
                continue;
            }
        }
        matched.push((task, ct));
    }

    // Deterministic non-causal sort per ADR-0074 section 8.
    matched.sort_by(|(lt, lct), (rt, rct)| {
        cmp_completion_key(
            lct.epoch_seconds,
            &lt.id,
            rct.epoch_seconds,
            &rt.id,
            request.oldest_first,
        )
    });

    let total_matched = matched.len();

    // Verify cursor points at an actual non-terminal row in this immutable
    // snapshot and advance strictly past its sort key.
    let after_cursor: Vec<(&aplan_core::Task, CompletionTime)> = if let Some(cursor) = parsed_cursor
    {
        let anchor = matched
            .iter()
            .position(|(task, ct)| task.id == cursor.last_id && ct.epoch_seconds == cursor.epoch);
        match anchor {
            Some(pos) if pos + 1 < matched.len() => {}
            _ => {
                return Err(CompletedError::InvalidCursor(
                    "cursor position does not match any truncated page boundary for this snapshot"
                        .to_string(),
                ));
            }
        }
        matched
            .into_iter()
            .filter(|(task, ct)| {
                cmp_completion_key(
                    ct.epoch_seconds,
                    &task.id,
                    cursor.epoch,
                    &cursor.last_id,
                    request.oldest_first,
                ) == Ordering::Greater
            })
            .collect()
    } else {
        matched
    };

    let (page, truncated) = match request.limit {
        Some(limit) if after_cursor.len() > limit => (after_cursor[..limit].to_vec(), true),
        _ => (after_cursor, false),
    };

    let next_cursor = if truncated {
        page.last().map(|(task, ct)| {
            encode_cursor(
                input_identity,
                &history_report.basis_revision,
                &query_fp,
                ct.epoch_seconds,
                &task.id,
            )
        })
    } else {
        None
    };

    let items = page
        .into_iter()
        .map(|(task, ct)| {
            let (cut, milestone) = task_cut
                .get(task.id.as_str())
                .map_or((None, None), |(c, m)| (Some(c.clone()), Some(m.clone())));
            let instant = ct.instant.or_else(|| {
                ct.epoch_seconds
                    .and_then(|epoch| format_closed_at(epoch).ok())
            });
            CompletedTaskRow {
                id: task.id.clone(),
                title: task.title.clone(),
                phase: task.phase,
                owner: task.owner.clone(),
                cut,
                milestone,
                tags: task.tags.clone(),
                instant,
                source: ct.source,
                recorded_origin: ct.recorded_origin,
                commit: ct.commit,
                incomparable_candidates: ct.incomparable_candidates,
            }
        })
        .collect();

    Ok(CompletedResponse {
        schema: "aplan/completed/v1".to_string(),
        input_identity: input_identity.to_string(),
        basis_revision: history_report.basis_revision,
        completeness: history_report.completeness,
        limitations: history_report.limitations,
        commits_walked: history_report.commits_walked,
        applied,
        total_matched,
        truncated,
        next_cursor,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_core::load::LoadedTask;
    use aplan_core::{ClosedAtSource, CompletionTimeSource, HistoryCompleteness, Task};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn done_task(id: &str, closed_at: Option<&str>, milestone: bool) -> LoadedTask {
        LoadedTask {
            path: PathBuf::from(format!("aplan/tasks/{id}.md")),
            task: Task {
                id: id.to_string(),
                title: format!("Task {id}"),
                phase: 1,
                status: Status::Done,
                owner: Some("alice".to_string()),
                effort_days: if milestone { None } else { Some(1.0) },
                blocked_by: Vec::new(),
                prefer_after: Vec::new(),
                ratified_by: Vec::new(),
                contains: Vec::new(),
                evidence: Vec::new(),
                paths: Vec::new(),
                traces: Vec::new(),
                trace_fingerprints: BTreeMap::new(),
                evidence_fingerprints: BTreeMap::new(),
                tags: if milestone {
                    vec!["milestone".to_string()]
                } else {
                    vec!["cli".to_string()]
                },
                revisit_at: None,
                version_impact: None,
                version_impact_reason: None,
                version: None,
                closed_at: closed_at.map(str::to_string),
                closed_at_source: closed_at.map(|_| ClosedAtSource::Local),
            },
            criteria: aplan_core::load::Criteria::default(),
            fields: BTreeMap::new(),
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn completed_tasks_orders_filters_pages_and_refuses_stale_or_invalid_inputs() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let mut dropped = done_task("D-DROP", None, false);
        dropped.task.status = Status::Dropped;
        let plan = Plan {
            tasks: vec![
                done_task("A-2", Some("2026-10-10T12:00:00Z"), false),
                done_task("A-1", Some("2026-10-10T12:00:00Z"), false),
                done_task("A-3", Some("2026-10-12T00:00:00Z"), false),
                done_task("A-UNK-2", None, false),
                done_task("A-UNK-1", None, false),
                done_task("M-1", Some("2026-10-13T00:00:00Z"), true),
                dropped,
            ],
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        };

        // Zero limit and reversed bounds are refused clearly.
        assert!(matches!(
            completed_tasks(
                &plan,
                "sha-1",
                dir.path(),
                Path::new("aplan"),
                &CompletedRequest {
                    limit: Some(0),
                    ..Default::default()
                },
            ),
            Err(CompletedError::InvalidLimit)
        ));
        assert!(matches!(
            completed_tasks(
                &plan,
                "sha-1",
                dir.path(),
                Path::new("aplan"),
                &CompletedRequest {
                    since: Some("2026-10-12".into()),
                    until: Some("2026-10-10".into()),
                    ..Default::default()
                },
            ),
            Err(CompletedError::ReversedBounds { .. })
        ));

        // Default newest-first with `--history never`: milestones & dropped excluded;
        // equal timestamps break by id ASC; unknowns sort AFTER knowns by id ASC.
        let full = completed_tasks(
            &plan,
            "sha-1",
            dir.path(),
            Path::new("aplan"),
            &CompletedRequest {
                history: HistoryPolicy::Never,
                ..Default::default()
            },
        )
        .expect("newest-first");
        let ids: Vec<&str> = full.items.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["A-3", "A-1", "A-2", "A-UNK-1", "A-UNK-2"]);
        assert_eq!(full.completeness, HistoryCompleteness::HistorySkipped);

        // `--oldest` reverses knowns while keeping unknowns AFTER knowns.
        let oldest = completed_tasks(
            &plan,
            "sha-1",
            dir.path(),
            Path::new("aplan"),
            &CompletedRequest {
                oldest_first: true,
                history: HistoryPolicy::Never,
                ..Default::default()
            },
        )
        .expect("oldest-first");
        let oldest_ids: Vec<&str> = oldest.items.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(oldest_ids, vec!["A-1", "A-2", "A-3", "A-UNK-1", "A-UNK-2"]);

        // Inclusive time bounds exclude unknowns and include boundary instants.
        let bounded = completed_tasks(
            &plan,
            "sha-1",
            dir.path(),
            Path::new("aplan"),
            &CompletedRequest {
                since: Some("2026-10-10T12:00:00Z".into()),
                until: Some("2026-10-12T00:00:00Z".into()),
                history: HistoryPolicy::Never,
                ..Default::default()
            },
        )
        .expect("inclusive bounds");
        let bounded_ids: Vec<&str> = bounded.items.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(bounded_ids, vec!["A-3", "A-1", "A-2"]);
        assert!(
            bounded
                .items
                .iter()
                .all(|r| r.source != CompletionTimeSource::Unknown)
        );

        // Snapshot-consistent pagination across equal and unknown timestamps:
        // page size 2 yields [A-3, A-1], [A-2, A-UNK-1], [A-UNK-2] with zero duplicates/omissions.
        let mut collected = Vec::new();
        let mut cursor = None;
        loop {
            let page = completed_tasks(
                &plan,
                "sha-1",
                dir.path(),
                Path::new("aplan"),
                &CompletedRequest {
                    limit: Some(2),
                    cursor: cursor.clone(),
                    history: HistoryPolicy::Never,
                    ..Default::default()
                },
            )
            .expect("page");
            collected.extend(page.items.iter().map(|r| r.id.clone()));
            if !page.truncated {
                assert_eq!(page.next_cursor, None);
                break;
            }
            cursor = page.next_cursor;
            assert!(cursor.is_some());
        }
        assert_eq!(collected, vec!["A-3", "A-1", "A-2", "A-UNK-1", "A-UNK-2"]);

        // Stale cursor (changed input_identity) is refused cleanly.
        let first_page = completed_tasks(
            &plan,
            "sha-1",
            dir.path(),
            Path::new("aplan"),
            &CompletedRequest {
                limit: Some(2),
                history: HistoryPolicy::Never,
                ..Default::default()
            },
        )
        .expect("first page");
        assert!(matches!(
            completed_tasks(
                &plan,
                "sha-2-changed",
                dir.path(),
                Path::new("aplan"),
                &CompletedRequest {
                    limit: Some(2),
                    cursor: first_page.next_cursor.clone(),
                    history: HistoryPolicy::Never,
                    ..Default::default()
                },
            ),
            Err(CompletedError::StaleCursor { .. })
        ));

        // Tampered cursor position (forged epoch or unknown id) is refused.
        let raw_cursor = first_page.next_cursor.as_deref().expect("raw cursor");
        let forged_epoch = raw_cursor.replacen("|1|", "|1|9999999999|", 1);
        assert!(matches!(
            completed_tasks(
                &plan,
                "sha-1",
                dir.path(),
                Path::new("aplan"),
                &CompletedRequest {
                    limit: Some(2),
                    cursor: Some(forged_epoch),
                    history: HistoryPolicy::Never,
                    ..Default::default()
                },
            ),
            Err(CompletedError::InvalidCursor(_))
        ));

        // Truncating a larger response keeps `applied.limit` unchanged and
        // yields a valid continuation cursor across both known and unknown rows.
        let mut big = completed_tasks(
            &plan,
            "sha-1",
            dir.path(),
            Path::new("aplan"),
            &CompletedRequest {
                limit: Some(5),
                history: HistoryPolicy::Never,
                ..Default::default()
            },
        )
        .expect("full page");
        assert_eq!(big.items.len(), 5);
        assert!(!big.truncated);
        assert!(truncate_completed_response(&mut big, 2));
        assert_eq!(big.items.len(), 2);
        assert!(big.truncated);
        assert_eq!(big.applied.limit, Some(5));
        let mut tail = completed_tasks(
            &plan,
            "sha-1",
            dir.path(),
            Path::new("aplan"),
            &CompletedRequest {
                limit: Some(5),
                cursor: big.next_cursor.clone(),
                history: HistoryPolicy::Never,
                ..Default::default()
            },
        )
        .expect("continued page");
        assert_eq!(
            tail.items.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["A-2", "A-UNK-1", "A-UNK-2"]
        );
        // Truncating again at an unknown-instant boundary also continues cleanly.
        assert!(truncate_completed_response(&mut tail, 2));
        let last = completed_tasks(
            &plan,
            "sha-1",
            dir.path(),
            Path::new("aplan"),
            &CompletedRequest {
                limit: Some(5),
                cursor: tail.next_cursor.clone(),
                history: HistoryPolicy::Never,
                ..Default::default()
            },
        )
        .expect("final page after unknown boundary");
        assert_eq!(
            last.items.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["A-UNK-2"]
        );
        assert!(!last.truncated);
    }
}
