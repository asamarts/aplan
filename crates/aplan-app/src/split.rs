//! Atomic work-task decomposition (`split`) shared across CLI, MCP, and TUI (`A2-150`).
//!
//! Splits one live non-milestone work task into bounded child increments wired through
//! parent-side `integrates` paired with `blocked_by` (`ADR-0076`, `A2-148`) inside one
//! guarded snapshot and one atomic [`ChangeSet`] publication (`commit_change_set`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use aplan_core::{Cardinality, IdStrategy, Plan, Status, StatusScope, TagFamily, Task};
use serde::{Deserialize, Serialize};

use crate::authoring::AuthoringError;
use crate::{
    ChangeEvidence, ChangeSet, CommitOutcome, FileChange, PlanContext, SemanticEffect,
    commit_change_set,
};

/// How the parent task's own `effort_days` is updated during a split.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SplitParentEffort {
    /// Clear the parent's `effort_days` (valid under `A2-148`'s `APLAN009` exemption
    /// for decomposed parents with live `integrates` children).
    #[default]
    Omit,
    /// Retain an explicit positive finite integration effort on the parent task.
    Keep(f64),
}

/// One child increment proposed by a [`SplitRequest`].
#[derive(Debug, Clone, PartialEq)]
pub struct SplitChildRequest {
    pub title: String,
    pub id: Option<String>,
    /// Optional local reference key (`local:<key>`) for wiring sibling edges.
    pub key: Option<String>,
    pub effort_days: Option<f64>,
    /// `None` inherits the parent's `owner`.
    pub owner: Option<String>,
    /// `None` inherits the parent's `tags`; `Some` replaces while still inheriting
    /// required tag-family members (unless explicitly overridden) and `cross-cutting`
    /// when the parent has no owning cut.
    pub tags: Option<Vec<String>>,
    /// `None` inherits the parent's `paths`.
    pub paths: Option<Vec<String>>,
    /// `None` inherits the parent's `traces`; `Some([])` clears inherited traces.
    pub traces: Option<Vec<String>>,
    /// Hard prerequisites: existing task IDs, explicit sibling IDs, or `local:<key>`.
    pub blocked_by: Vec<String>,
    /// Advisory preferences: existing task IDs, explicit sibling IDs, or `local:<key>`.
    pub prefer_after: Vec<String>,
    /// Optional child Markdown body; `None` generates a clean scaffold referencing the parent.
    pub body: Option<String>,
}

/// Typed request for previewing and atomically applying a work-task split.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitRequest {
    pub target: String,
    pub children: Vec<SplitChildRequest>,
    pub parent_effort: SplitParentEffort,
    /// When `false` (default) and the parent declares `effort_days`, the sum of child
    /// estimates plus `parent_effort` must reconcile exactly to the parent's prior effort.
    pub reestimate: bool,
    /// Optional explicit cut assertion; must match the parent's owning cut.
    pub cut: Option<String>,
    /// Optional snapshot digest (`InputIdentity::sha256`) from an earlier preview.
    pub expected_identity: Option<String>,
}

/// Deterministic summary of one child task in a split preview or applied result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitChildSummary {
    pub id: String,
    pub title: String,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_days: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    pub traces: Vec<String>,
    pub blocked_by: Vec<String>,
    pub prefer_after: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<String>,
}

/// Deterministic summary of the parent task's transition during a split.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitParentResult {
    pub id: String,
    pub file: String,
    pub before_status: Status,
    pub after_status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_effort_days: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_effort_days: Option<f64>,
    pub added_blocked_by: Vec<String>,
    pub added_integrates: Vec<String>,
    pub body_retained: bool,
    pub checked_criteria_retained: usize,
}

/// Effort reconciliation summary for a split.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitEffortSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_parent_days: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_parent_days: Option<f64>,
    pub children_total_days: f64,
    pub after_total_days: f64,
    pub reestimated: bool,
}

/// Shared deterministic machine-readable split result across CLI, MCP, and TUI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitMachineResult {
    pub schema: String,
    pub applied: bool,
    pub preview_identity: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<String>,
    pub parent: SplitParentResult,
    pub children: Vec<SplitChildSummary>,
    pub effort: SplitEffortSummary,
    pub cut_memberships_added: Vec<String>,
    pub files: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Guarded dry-run preview of a split, bound to one immutable [`PlanContext`] identity.
#[derive(Debug)]
pub struct SplitPreview {
    change: ChangeSet,
    machine: SplitMachineResult,
}

impl SplitPreview {
    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }

    #[must_use]
    pub fn machine_result(&self, applied: bool) -> SplitMachineResult {
        let mut out = self.machine.clone();
        out.applied = applied;
        out
    }

    /// Concise human/TUI review lines describing every predicted effect before apply.
    #[must_use]
    pub fn review_rows(&self) -> Vec<String> {
        let m = &self.machine;
        let mut rows = vec![
            format!(
                "split `{}` into {} child increment(s) (cut: {})",
                m.target,
                m.children.len(),
                m.cut.as_deref().unwrap_or("cross-cutting")
            ),
            format!(
                "parent `{}`: status {} -> {}, effort {} -> {} (body & {} checked criteria retained verbatim)",
                m.parent.id,
                m.parent.before_status,
                m.parent.after_status,
                fmt_opt_days(m.parent.before_effort_days),
                fmt_opt_days(m.parent.after_effort_days),
                m.parent.checked_criteria_retained
            ),
            format!(
                "effort allocation: children={}d + parent={} = {}d{}",
                m.effort.children_total_days,
                fmt_opt_days(m.effort.after_parent_days),
                m.effort.after_total_days,
                if m.effort.reestimated {
                    " (reestimated)"
                } else {
                    " (reconciled)"
                }
            ),
            format!("preview identity: {}", m.preview_identity),
        ];
        for child in &m.children {
            let deps = if child.blocked_by.is_empty() {
                String::new()
            } else {
                format!(" [blocked_by: {}]", child.blocked_by.join(", "))
            };
            rows.push(format!(
                "  child `{}`: {} ({}){deps}",
                child.id,
                child.title,
                fmt_opt_days(child.effort_days)
            ));
        }
        rows
    }
}

fn fmt_opt_days(days: Option<f64>) -> String {
    days.map_or_else(|| "omitted".to_string(), |d| format!("{d}d"))
}

/// Parsed YAML or JSON split specification document (shared by CLI `--from` and TUI).
#[derive(Debug, Clone, PartialEq)]
pub struct SplitSpecDocument {
    pub target: Option<String>,
    pub children: Vec<SplitChildRequest>,
    pub parent_effort: Option<SplitParentEffort>,
    pub reestimate: Option<bool>,
    pub cut: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSplitChild {
    title: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default, alias = "effort")]
    effort_days: Option<f64>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    paths: Option<Vec<String>>,
    #[serde(default)]
    traces: Option<Vec<String>>,
    #[serde(default)]
    blocked_by: Vec<String>,
    #[serde(default)]
    prefer_after: Vec<String>,
    #[serde(default)]
    body: Option<String>,
}

impl From<RawSplitChild> for SplitChildRequest {
    fn from(raw: RawSplitChild) -> Self {
        Self {
            title: raw.title,
            id: raw.id,
            key: raw.key,
            effort_days: raw.effort_days,
            owner: raw.owner,
            tags: raw.tags,
            paths: raw.paths,
            traces: raw.traces,
            blocked_by: raw.blocked_by,
            prefer_after: raw.prefer_after,
            body: raw.body,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSplitMapping {
    #[serde(default)]
    target: Option<String>,
    children: Vec<RawSplitChild>,
    #[serde(default, alias = "keep_effort")]
    parent_effort: Option<f64>,
    #[serde(default)]
    reestimate: Option<bool>,
    #[serde(default)]
    cut: Option<String>,
}

/// Parse a YAML or JSON split specification (either a list of children or a mapping).
///
/// # Errors
/// Returns [`AuthoringError::Arguments`] when the input is blank or malformed.
pub fn parse_split_spec(
    source_name: &str,
    contents: &str,
) -> Result<SplitSpecDocument, AuthoringError> {
    if contents.trim().is_empty() {
        return Err(AuthoringError::Arguments(format!(
            "split specification `{source_name}` is empty"
        )));
    }
    let trimmed = contents.trim_start();
    if trimmed.starts_with('-') || trimmed.starts_with('[') {
        let items: Vec<RawSplitChild> = aplan_core::yaml::from_str(contents).map_err(|err| {
            AuthoringError::Arguments(format!("parsing split spec `{source_name}`: {err}"))
        })?;
        return Ok(SplitSpecDocument {
            target: None,
            children: items.into_iter().map(Into::into).collect(),
            parent_effort: None,
            reestimate: None,
            cut: None,
        });
    }
    let doc: RawSplitMapping = aplan_core::yaml::from_str(contents).map_err(|err| {
        AuthoringError::Arguments(format!("parsing split spec `{source_name}`: {err}"))
    })?;
    Ok(SplitSpecDocument {
        target: doc.target,
        children: doc.children.into_iter().map(Into::into).collect(),
        parent_effort: doc.parent_effort.map(SplitParentEffort::Keep),
        reestimate: doc.reestimate,
        cut: doc.cut,
    })
}

/// Parse one compact inline `--child` CLI argument: `"Title"` or `"Title=1.5"`.
///
/// If the trailing `=...` parses as a number (or non-finite token like `NaN`/`inf`),
/// it is decoded as `effort_days`; otherwise the entire string is kept as the title.
///
/// # Errors
/// Returns [`AuthoringError::Arguments`] when the title is blank or the effort suffix is invalid.
pub fn parse_inline_child(spec: &str) -> Result<SplitChildRequest, AuthoringError> {
    let (title, effort_days) = if let Some((head, tail)) = spec.rsplit_once('=') {
        let trimmed_tail = tail.trim();
        let parsed = trimmed_tail.parse::<f64>().map_err(|_| {
            AuthoringError::Arguments(format!(
                "invalid effort `{trimmed_tail}` in `--child {spec:?}`; expected a positive finite number of days"
            ))
        })?;
        (head.trim().to_string(), Some(parsed))
    } else {
        (spec.trim().to_string(), None)
    };
    if aplan_core::task::is_vacuous_ref(&title) {
        return Err(AuthoringError::Arguments(format!(
            "child title in `{spec}` may not be blank"
        )));
    }
    Ok(SplitChildRequest {
        title,
        id: None,
        key: None,
        effort_days,
        owner: None,
        tags: None,
        paths: None,
        traces: None,
        blocked_by: Vec::new(),
        prefer_after: Vec::new(),
        body: None,
    })
}

/// Preview an atomic work-task split without reserving IDs or writing any file.
///
/// # Errors
/// Returns [`AuthoringError`] for load failures, stale preview identity, ineligible
/// targets, invalid/ambiguous/cyclic/cross-cut splits, effort reconciliation mismatches,
/// or source-preserving rewrite failures.
pub fn preview_split(
    context: &PlanContext,
    request: &SplitRequest,
) -> Result<SplitPreview, AuthoringError> {
    if !context.plan().load_diagnostics.is_empty() {
        return Err(AuthoringError::Plan(
            "the plan does not load cleanly; repair load errors before splitting".to_string(),
        ));
    }
    check_expected_identity(context, request.expected_identity.as_deref())?;
    let target = resolve_split_target(context.plan(), &request.target)?;
    let owning_cut = resolve_target_cut(context.plan(), &target.task.id, request.cut.as_deref())?;
    let parent_after_effort = validate_parent_effort(request.parent_effort)?;
    let children_total_days = validate_children_and_effort(
        context.plan(),
        &target.task,
        owning_cut.as_deref(),
        &request.children,
        parent_after_effort,
        request.reestimate,
    )?;

    let planned = materialize_children(
        context,
        &target.task,
        owning_cut.as_deref(),
        &request.children,
    )?;
    let child_ids: Vec<String> = planned.iter().map(|c| c.task.id.clone()).collect();
    let parent_after_status = if target.task.status == Status::InProgress {
        Status::Todo
    } else {
        target.task.status
    };

    let (parent_change, updated_parent, new_blocked_by) =
        rewrite_parent_file(target, &child_ids, parent_after_effort, parent_after_status)?;

    let mut after =
        aplan_core::mutation::with_replaced(context.plan(), &target.task.id, updated_parent);
    let mut files = Vec::with_capacity(planned.len() + 2);
    let mut effects = Vec::with_capacity(planned.len() + 4);
    let child_summaries = stage_planned_children(
        &mut after,
        &mut files,
        &mut effects,
        planned,
        owning_cut.as_deref(),
    )?;
    files.push(parent_change);

    let cut_memberships_added = stage_cut_and_split_effects(
        context.plan(),
        &mut after,
        &mut files,
        &mut effects,
        &target.task,
        parent_after_status,
        owning_cut.as_deref(),
        &child_ids,
    )?;

    let delta = evaluate_split_delta(
        context,
        &after,
        &target.task.id,
        &child_ids,
        owning_cut.as_deref(),
    );
    let denied = delta.refuses(context.verdict());
    let machine = build_split_machine(
        context,
        target,
        owning_cut,
        parent_after_status,
        parent_after_effort,
        children_total_days,
        request.reestimate,
        new_blocked_by,
        child_ids,
        child_summaries,
        cut_memberships_added,
        &files,
        delta.lines(),
    );
    let change = ChangeSet::from_parts(
        context.identity().clone(),
        files,
        effects,
        delta,
        denied,
        ChangeEvidence::NotRequired,
    );
    Ok(SplitPreview { change, machine })
}

fn stage_planned_children(
    after: &mut Plan,
    files: &mut Vec<FileChange>,
    effects: &mut Vec<SemanticEffect>,
    planned: Vec<PlannedChild>,
    owning_cut: Option<&str>,
) -> Result<Vec<SplitChildSummary>, AuthoringError> {
    let mut summaries = Vec::with_capacity(planned.len());
    for item in planned {
        let contents = aplan_interop::emit::task_file(&item.task, &item.body)
            .map_err(|error| AuthoringError::Interop(error.to_string()))?;
        *after = aplan_core::mutation::with_added(after, item.path.clone(), item.task.clone());
        files.push(FileChange::creation(item.path.clone(), contents));
        effects.push(SemanticEffect::TaskCreated {
            task: item.task.id.clone(),
            title: item.task.title.clone(),
        });
        summaries.push(SplitChildSummary {
            id: item.task.id,
            title: item.task.title,
            file: item.path.display().to_string(),
            effort_days: item.task.effort_days,
            owner: item.task.owner,
            tags: item.task.tags,
            paths: item.task.paths,
            traces: item.task.traces,
            blocked_by: item.task.blocked_by,
            prefer_after: item.task.prefer_after,
            cut: owning_cut.map(str::to_string),
        });
    }
    Ok(summaries)
}

#[allow(clippy::too_many_arguments)]
fn stage_cut_and_split_effects(
    before: &Plan,
    after: &mut Plan,
    files: &mut Vec<FileChange>,
    effects: &mut Vec<SemanticEffect>,
    target: &Task,
    parent_after_status: Status,
    owning_cut: Option<&str>,
    child_ids: &[String],
) -> Result<Vec<String>, AuthoringError> {
    let mut cut_memberships_added = Vec::new();
    if let Some(cut_id) = owning_cut {
        add_cut_members(after, cut_id, child_ids);
        if let Some(cut_change) = rewrite_cut_file(before, cut_id, child_ids)? {
            files.push(cut_change);
        }
        cut_memberships_added.extend_from_slice(child_ids);
        effects.push(SemanticEffect::CutMembershipAdded {
            cut: cut_id.to_string(),
            tasks: child_ids.to_vec(),
        });
    }
    if parent_after_status != target.status {
        effects.push(SemanticEffect::TaskStatusChanged {
            task: target.id.clone(),
            from: target.status,
            to: parent_after_status,
        });
    }
    effects.push(SemanticEffect::TaskSplit {
        parent: target.id.clone(),
        children: child_ids.to_vec(),
    });
    Ok(cut_memberships_added)
}

#[allow(clippy::too_many_arguments)]
fn build_split_machine(
    context: &PlanContext,
    target: &aplan_core::load::LoadedTask,
    owning_cut: Option<String>,
    parent_after_status: Status,
    parent_after_effort: Option<f64>,
    children_total_days: f64,
    reestimated: bool,
    new_blocked_by: Vec<String>,
    child_ids: Vec<String>,
    child_summaries: Vec<SplitChildSummary>,
    cut_memberships_added: Vec<String>,
    files: &[FileChange],
    notes: Vec<String>,
) -> SplitMachineResult {
    let planned_files: Vec<String> = files
        .iter()
        .map(|f| f.path().display().to_string())
        .collect();
    let added_blocked_by: Vec<String> = new_blocked_by
        .into_iter()
        .filter(|id| !target.task.blocked_by.iter().any(|old| old == id))
        .collect();
    SplitMachineResult {
        schema: "aplan/split/v1".to_string(),
        applied: false,
        preview_identity: context.identity().sha256.clone(),
        target: target.task.id.clone(),
        cut: owning_cut,
        parent: SplitParentResult {
            id: target.task.id.clone(),
            file: target.path.display().to_string(),
            before_status: target.task.status,
            after_status: parent_after_status,
            before_effort_days: target.task.effort_days,
            after_effort_days: parent_after_effort,
            added_blocked_by,
            added_integrates: child_ids,
            body_retained: true,
            checked_criteria_retained: target.criteria.ticked,
        },
        children: child_summaries,
        effort: SplitEffortSummary {
            before_parent_days: target.task.effort_days,
            after_parent_days: parent_after_effort,
            children_total_days,
            after_total_days: children_total_days + parent_after_effort.unwrap_or(0.0),
            reestimated,
        },
        cut_memberships_added,
        files: planned_files,
        notes,
    }
}

/// Atomically publish a previously reviewed [`SplitPreview`] through the shared guarded writer.
#[must_use]
pub fn commit_split(context: &PlanContext, preview: SplitPreview) -> CommitOutcome {
    commit_change_set(context, preview.into_change_set())
}

fn check_expected_identity(
    context: &PlanContext,
    expected: Option<&str>,
) -> Result<(), AuthoringError> {
    let Some(raw) = expected else {
        return Ok(());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AuthoringError::Arguments(
            "expected preview identity may not be blank".to_string(),
        ));
    }
    if trimmed != context.identity().sha256 {
        return Err(AuthoringError::Refused(format!(
            "stale preview: expected plan identity `{trimmed}`, but current plan identity is `{}`; re-run preview before applying",
            context.identity().sha256
        )));
    }
    Ok(())
}

fn resolve_split_target<'a>(
    plan: &'a Plan,
    target_id: &str,
) -> Result<&'a aplan_core::load::LoadedTask, AuthoringError> {
    if aplan_core::task::is_vacuous_ref(target_id) {
        return Err(AuthoringError::Arguments(
            "split target task id may not be blank".to_string(),
        ));
    }
    let mut matches = plan.tasks.iter().filter(|lt| lt.task.id == target_id);
    let Some(first) = matches.next() else {
        return Err(AuthoringError::Refused(format!(
            "unknown task `{target_id}`; cannot split a task that does not exist"
        )));
    };
    if matches.next().is_some() {
        return Err(AuthoringError::Refused(format!(
            "ambiguous split target `{target_id}`: multiple task files declare this id; repair duplicate ids first"
        )));
    }
    let t = &first.task;
    if t.is_milestone() {
        return Err(AuthoringError::Refused(format!(
            "`{}` is a milestone; milestones are cut checkpoints and cannot be split into work increments",
            t.id
        )));
    }
    if t.status.is_terminal() {
        return Err(AuthoringError::Refused(format!(
            "cannot split terminal task `{}` (`{}`); historical claims cannot be rewritten",
            t.id, t.status
        )));
    }
    if !t.integrates.is_empty() {
        return Err(AuthoringError::Refused(format!(
            "`{}` is already decomposed via `integrates` ({}); add or edit child increments explicitly instead",
            t.id,
            t.integrates.join(", ")
        )));
    }
    if !t.evidence.is_empty() || !t.evidence_fingerprints.is_empty() || t.closed_at.is_some() {
        return Err(AuthoringError::Refused(format!(
            "`{}` already carries evidence or closure metadata; split refuses to strip or duplicate proof",
            t.id
        )));
    }
    Ok(first)
}

fn resolve_target_cut(
    plan: &Plan,
    target_id: &str,
    requested_cut: Option<&str>,
) -> Result<Option<String>, AuthoringError> {
    let owning: Vec<String> = plan
        .tasks
        .iter()
        .filter(|lt| lt.task.is_milestone() && lt.task.blocked_by.iter().any(|b| b == target_id))
        .map(|lt| lt.task.id.clone())
        .collect();
    if owning.len() > 1 {
        return Err(AuthoringError::Refused(format!(
            "ambiguous cut membership for `{target_id}`: listed in multiple cuts ({}); repair cut membership before splitting",
            owning.join(", ")
        )));
    }
    let owning_cut = owning.into_iter().next();
    if let Some(cut_id) = &owning_cut
        && let Some(ms) = plan.tasks.iter().find(|lt| lt.task.id == *cut_id)
        && ms.task.status.is_terminal()
    {
        return Err(AuthoringError::Refused(format!(
            "cannot split `{target_id}` inside terminal cut `{cut_id}` (`{}`)",
            ms.task.status
        )));
    }
    if let Some(raw_cut) = requested_cut {
        if aplan_core::task::is_vacuous_ref(raw_cut) {
            return Err(AuthoringError::Arguments(
                "cut selector may not be blank".to_string(),
            ));
        }
        let resolved = aplan_interop::cut::resolve_id(plan, raw_cut)
            .map_err(|err| AuthoringError::Refused(err.to_string()))?;
        if owning_cut.as_deref() != Some(resolved.as_str()) {
            return Err(AuthoringError::Refused(format!(
                "cross-cut split refused: `{target_id}` belongs to {}, not `{resolved}`",
                owning_cut.as_deref().map_or_else(
                    || "no cut (`cross-cutting`)".to_string(),
                    |c| format!("`{c}`")
                )
            )));
        }
    }
    Ok(owning_cut)
}

fn validate_parent_effort(effort: SplitParentEffort) -> Result<Option<f64>, AuthoringError> {
    match effort {
        SplitParentEffort::Omit => Ok(None),
        SplitParentEffort::Keep(days) => {
            if !days.is_finite() || days <= 0.0 {
                return Err(AuthoringError::Arguments(format!(
                    "parent integration effort must be a positive finite number of days, not `{days}` (omit parent effort instead of using zero)"
                )));
            }
            Ok(Some(days))
        }
    }
}

fn validate_children_and_effort(
    plan: &Plan,
    target: &Task,
    owning_cut: Option<&str>,
    children: &[SplitChildRequest],
    parent_after_effort: Option<f64>,
    reestimate: bool,
) -> Result<f64, AuthoringError> {
    if children.len() < 2 {
        return Err(AuthoringError::Arguments(format!(
            "a split requires at least two child increments (received {})",
            children.len()
        )));
    }
    let mut seen_ids = BTreeSet::new();
    let mut seen_keys = BTreeSet::new();
    let mut seen_titles = BTreeSet::new();
    let mut children_total = 0.0_f64;

    for (idx, child) in children.iter().enumerate() {
        validate_one_child(
            plan,
            owning_cut,
            idx,
            child,
            &mut seen_ids,
            &mut seen_keys,
            &mut seen_titles,
        )?;
        if let Some(days) = child.effort_days {
            children_total += days;
        } else if target.effort_days.is_some() && !reestimate {
            return Err(AuthoringError::Arguments(format!(
                "child #{} (`{}`) omits `effort_days`; supply positive estimates reconciling to `{}`'s {}d or pass `--reestimate`",
                idx + 1,
                child.title,
                target.id,
                target.effort_days.unwrap_or(0.0)
            )));
        }
    }

    if let Some(orig) = target.effort_days
        && !reestimate
    {
        let after_total = children_total + parent_after_effort.unwrap_or(0.0);
        if (after_total - orig).abs() > 1e-6 {
            return Err(AuthoringError::Refused(format!(
                "effort reconciliation mismatch for `{}`: original effort is {orig}d, but children ({children_total}d) + parent integration effort ({}) = {after_total}d; adjust allocations to total {orig}d or pass `--reestimate`",
                target.id,
                fmt_opt_days(parent_after_effort)
            )));
        }
    }
    Ok(children_total)
}

fn validate_one_child(
    plan: &Plan,
    owning_cut: Option<&str>,
    idx: usize,
    child: &SplitChildRequest,
    seen_ids: &mut BTreeSet<String>,
    seen_keys: &mut BTreeSet<String>,
    seen_titles: &mut BTreeSet<String>,
) -> Result<(), AuthoringError> {
    if aplan_core::task::is_vacuous_ref(&child.title) {
        return Err(AuthoringError::Arguments(format!(
            "child #{} title may not be blank",
            idx + 1
        )));
    }
    if !seen_titles.insert(child.title.trim().to_string()) {
        return Err(AuthoringError::Arguments(format!(
            "duplicate child title `{}` in split; each increment must name a distinct outcome",
            child.title.trim()
        )));
    }
    if let Some(days) = child.effort_days
        && (!days.is_finite() || days <= 0.0)
    {
        return Err(AuthoringError::Arguments(format!(
            "child `{}` effort must be a positive finite number of days, not `{days}`",
            child.title
        )));
    }
    if child
        .owner
        .as_deref()
        .is_some_and(aplan_core::task::is_vacuous_ref)
    {
        return Err(AuthoringError::Arguments(format!(
            "child `{}` owner may not be blank",
            child.title
        )));
    }
    if child
        .body
        .as_deref()
        .is_some_and(aplan_core::task::is_vacuous_ref)
    {
        return Err(AuthoringError::Arguments(format!(
            "child `{}` body may not be blank; omit it for the generated template",
            child.title
        )));
    }
    if let Some(id) = &child.id {
        validate_child_id(plan, id, seen_ids)?;
    }
    if let Some(key) = &child.key {
        validate_child_key(key, seen_keys)?;
    }
    validate_child_lists(owning_cut, child)
}

fn validate_child_id(
    plan: &Plan,
    id: &str,
    seen_ids: &mut BTreeSet<String>,
) -> Result<(), AuthoringError> {
    if !aplan_interop::emit::is_usable_id(id) {
        return Err(AuthoringError::Arguments(format!(
            "`{id}` cannot be used as a task id and file stem"
        )));
    }
    if !seen_ids.insert(id.to_string()) {
        return Err(AuthoringError::Arguments(format!(
            "duplicate child id `{id}` in split"
        )));
    }
    if let Some(existing) = plan.tasks.iter().find(|lt| lt.task.id == id) {
        return Err(AuthoringError::Refused(format!(
            "child id `{id}` already exists in {}",
            existing.path.display()
        )));
    }
    Ok(())
}

fn validate_child_key(key: &str, seen_keys: &mut BTreeSet<String>) -> Result<(), AuthoringError> {
    if aplan_core::task::is_vacuous_ref(key)
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AuthoringError::Arguments(format!(
            "child key `{key}` must be non-blank ASCII alphanumeric, `-`, or `_`"
        )));
    }
    if !seen_keys.insert(key.to_string()) {
        return Err(AuthoringError::Arguments(format!(
            "duplicate local key `{key}` in split"
        )));
    }
    Ok(())
}

fn validate_child_lists(
    owning_cut: Option<&str>,
    child: &SplitChildRequest,
) -> Result<(), AuthoringError> {
    if let Some(tags) = &child.tags {
        if tags.iter().any(|t| aplan_core::task::is_vacuous_ref(t)) {
            return Err(AuthoringError::Arguments(format!(
                "child `{}` has a blank tag",
                child.title
            )));
        }
        if tags.iter().any(|t| t == "milestone") {
            return Err(AuthoringError::Refused(format!(
                "child `{}` cannot carry the `milestone` tag; split increments must be work tasks",
                child.title
            )));
        }
        if owning_cut.is_some() && tags.iter().any(|t| t == "cross-cutting") {
            return Err(AuthoringError::Refused(format!(
                "cross-cut split refused: child `{}` cannot be tagged `cross-cutting` while its parent belongs to `{}`",
                child.title,
                owning_cut.unwrap_or_default()
            )));
        }
    }
    if let Some(paths) = &child.paths
        && paths.iter().any(|p| aplan_core::task::is_vacuous_ref(p))
    {
        return Err(AuthoringError::Arguments(format!(
            "child `{}` has a blank path",
            child.title
        )));
    }
    if let Some(traces) = &child.traces
        && traces.iter().any(|t| aplan_core::task::is_vacuous_ref(t))
    {
        return Err(AuthoringError::Arguments(format!(
            "child `{}` has a blank trace",
            child.title
        )));
    }
    Ok(())
}

struct PlannedChild {
    task: Task,
    body: String,
    path: PathBuf,
}

fn materialize_children(
    context: &PlanContext,
    target: &Task,
    owning_cut: Option<&str>,
    children: &[SplitChildRequest],
) -> Result<Vec<PlannedChild>, AuthoringError> {
    let dir = split_task_dir(context);
    let strategy = context
        .report_config()
        .and_then(|c| c.id_strategy)
        .unwrap_or(IdStrategy::Sequential);
    let mut mint_plan = aplan_core::mutation::clone_of(context.plan());
    let mut minted_ids = Vec::with_capacity(children.len());
    let mut local_keys: BTreeMap<String, String> = BTreeMap::new();
    let mut new_id_set = BTreeSet::new();

    for child in children {
        let id = match &child.id {
            Some(explicit) => explicit.clone(),
            None => aplan_interop::ids::mint_with_policy(
                &mint_plan,
                target.phase,
                owning_cut,
                strategy,
                context.options().id_families.as_ref(),
            )
            .map_err(|err| AuthoringError::Refused(err.to_string()))?,
        };
        aplan_interop::cut::check_task_id_label(&mint_plan, &id, None)
            .map_err(|err| AuthoringError::Refused(err.to_string()))?;
        let placeholder = empty_placeholder_task(&id, &child.title, target.phase);
        let path = aplan_interop::emit::task_path(&dir, &id);
        mint_plan = aplan_core::mutation::with_added(&mint_plan, path, placeholder);
        if let Some(key) = &child.key {
            local_keys.insert(key.clone(), id.clone());
        }
        new_id_set.insert(id.clone());
        minted_ids.push(id);
    }

    let mut out = Vec::with_capacity(children.len());
    for (child, id) in children.iter().zip(minted_ids) {
        let blocked_by = resolve_child_refs(
            context.plan(),
            &target.id,
            &id,
            "blocked_by",
            &child.blocked_by,
            &local_keys,
            &new_id_set,
        )?;
        let prefer_after = resolve_child_refs(
            context.plan(),
            &target.id,
            &id,
            "prefer_after",
            &child.prefer_after,
            &local_keys,
            &new_id_set,
        )?;
        let tags = resolve_child_tags(
            &target.tags,
            child.tags.as_deref(),
            owning_cut.is_none(),
            &context.options().tag_families,
        );
        let task = Task {
            id: id.clone(),
            title: child.title.trim().to_string(),
            phase: target.phase,
            status: Status::Todo,
            owner: child.owner.clone().or_else(|| target.owner.clone()),
            effort_days: child.effort_days,
            blocked_by,
            prefer_after,
            ratified_by: Vec::new(),
            contains: Vec::new(),
            integrates: Vec::new(),
            evidence: Vec::new(),
            evidence_fingerprints: BTreeMap::new(),
            paths: child.paths.clone().unwrap_or_else(|| target.paths.clone()),
            traces: child
                .traces
                .clone()
                .unwrap_or_else(|| target.traces.clone()),
            trace_fingerprints: BTreeMap::new(),
            tags,
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
            closed_at: None,
            closed_at_source: None,
        };
        let body = child
            .body
            .clone()
            .unwrap_or_else(|| default_child_body(&target.id, &target.title));
        let path = aplan_interop::emit::task_path(&dir, &id);
        out.push(PlannedChild { task, body, path });
    }
    Ok(out)
}

fn resolve_child_refs(
    plan: &Plan,
    parent_id: &str,
    child_id: &str,
    field: &str,
    refs: &[String],
    local_keys: &BTreeMap<String, String>,
    new_ids: &BTreeSet<String>,
) -> Result<Vec<String>, AuthoringError> {
    let mut resolved = Vec::with_capacity(refs.len());
    for raw in refs {
        if aplan_core::task::is_vacuous_ref(raw) {
            return Err(AuthoringError::Arguments(format!(
                "child `{child_id}` has a blank `{field}` reference"
            )));
        }
        let target_ref = if let Some(key) = raw.strip_prefix("local:") {
            local_keys.get(key).cloned().ok_or_else(|| {
                AuthoringError::Arguments(format!(
                    "child `{child_id}` `{field}` names unknown sibling key `local:{key}`"
                ))
            })?
        } else {
            raw.clone()
        };
        if target_ref == child_id {
            return Err(AuthoringError::Refused(format!(
                "cyclic split: child `{child_id}` cannot reference itself in `{field}`"
            )));
        }
        if target_ref == parent_id {
            return Err(AuthoringError::Refused(format!(
                "cyclic split: child `{child_id}` cannot reference parent `{parent_id}` in `{field}` because `{parent_id}` integrates `{child_id}`"
            )));
        }
        let known =
            new_ids.contains(&target_ref) || plan.tasks.iter().any(|lt| lt.task.id == target_ref);
        if !known {
            return Err(AuthoringError::Arguments(format!(
                "child `{child_id}` `{field}` names unknown task `{target_ref}`"
            )));
        }
        resolved.push(target_ref);
    }
    Ok(aplan_interop::emit::deduped(&resolved))
}

fn resolve_child_tags(
    parent_tags: &[String],
    explicit: Option<&[String]>,
    cutless_parent: bool,
    tag_families: &BTreeMap<String, TagFamily>,
) -> Vec<String> {
    let Some(explicit) = explicit else {
        return aplan_interop::emit::deduped(parent_tags);
    };
    let mut tags = aplan_interop::emit::deduped(explicit);
    for family in tag_families.values() {
        let required = matches!(
            family.require,
            Cardinality::AtLeastOne | Cardinality::ExactlyOne
        );
        let applies_to_live = matches!(family.status, StatusScope::Any | StatusScope::Live);
        if !required || !applies_to_live {
            continue;
        }
        let has_override = tags
            .iter()
            .any(|tag| family.members.iter().any(|m| m == tag));
        if !has_override {
            for parent_tag in parent_tags {
                if family.members.iter().any(|m| m == parent_tag)
                    && !tags.iter().any(|t| t == parent_tag)
                {
                    tags.push(parent_tag.clone());
                }
            }
        }
    }
    if cutless_parent
        && parent_tags.iter().any(|t| t == "cross-cutting")
        && !tags.iter().any(|t| t == "cross-cutting")
    {
        tags.push("cross-cutting".to_string());
    }
    tags
}

fn rewrite_parent_file(
    target: &aplan_core::load::LoadedTask,
    child_ids: &[String],
    parent_after_effort: Option<f64>,
    parent_after_status: Status,
) -> Result<(FileChange, Task, Vec<String>), AuthoringError> {
    let original =
        std::fs::read_to_string(&target.path).map_err(|source| AuthoringError::Read {
            path: target.path.clone(),
            source,
        })?;
    let mut new_blocked_by = target.task.blocked_by.clone();
    for id in child_ids {
        if !new_blocked_by.iter().any(|b| b == id) {
            new_blocked_by.push(id.clone());
        }
    }
    let rewrite_err = |source| AuthoringError::Rewrite {
        id: target.task.id.clone(),
        source,
    };
    let mut updated =
        aplan_core::close::set_blocked_by(&original, &new_blocked_by).map_err(rewrite_err)?;
    updated = aplan_core::close::set_integrates(&updated, child_ids).map_err(rewrite_err)?;
    updated =
        aplan_core::close::set_effort_days(&updated, parent_after_effort).map_err(rewrite_err)?;
    if parent_after_status != target.task.status {
        updated =
            aplan_core::close::set_status(&updated, parent_after_status).map_err(rewrite_err)?;
    }
    let parsed = aplan_core::mutation::task_of(&updated).map_err(AuthoringError::Plan)?;
    Ok((
        FileChange::replacement(target.path.clone(), original, updated),
        parsed,
        new_blocked_by,
    ))
}

fn add_cut_members(plan: &mut Plan, cut_id: &str, ids: &[String]) {
    if let Some(ms) = plan.tasks.iter_mut().find(|lt| lt.task.id == cut_id) {
        for id in ids {
            if !ms.task.blocked_by.iter().any(|b| b == id) {
                ms.task.blocked_by.push(id.clone());
            }
        }
    }
}

fn rewrite_cut_file(
    plan: &Plan,
    cut_id: &str,
    child_ids: &[String],
) -> Result<Option<FileChange>, AuthoringError> {
    let Some(ms) = plan.tasks.iter().find(|lt| lt.task.id == cut_id) else {
        return Ok(None);
    };
    let original = std::fs::read_to_string(&ms.path).map_err(|source| AuthoringError::Read {
        path: ms.path.clone(),
        source,
    })?;
    let mut blocked = ms.task.blocked_by.clone();
    for id in child_ids {
        if !blocked.iter().any(|b| b == id) {
            blocked.push(id.clone());
        }
    }
    if blocked == ms.task.blocked_by {
        return Ok(None);
    }
    let updated = aplan_core::close::set_blocked_by(&original, &blocked).map_err(|source| {
        AuthoringError::Rewrite {
            id: cut_id.to_string(),
            source,
        }
    })?;
    Ok(Some(FileChange::replacement(
        ms.path.clone(),
        original,
        updated,
    )))
}

fn evaluate_split_delta(
    context: &PlanContext,
    after: &Plan,
    target_id: &str,
    child_ids: &[String],
    owning_cut: Option<&str>,
) -> aplan_core::mutation::Delta {
    let mut delta = aplan_core::mutation::delta(context.plan(), after, context.options());
    let after_report = aplan_core::validate(after, context.options());
    let mut touched: BTreeSet<&str> = child_ids.iter().map(String::as_str).collect();
    touched.insert(target_id);
    if let Some(cut) = owning_cut {
        touched.insert(cut);
    }
    for diag in after_report.diagnostics {
        let on_touched = diag.task.as_deref().is_some_and(|t| touched.contains(t));
        let blocks = !context.verdict().is_advisory(diag.code)
            && (diag.severity == aplan_core::Severity::Error
                || (context.verdict().deny_warnings
                    && diag.severity == aplan_core::Severity::Warning));
        if on_touched
            && blocks
            && !delta.added.iter().any(|existing| {
                existing.code == diag.code
                    && existing.task == diag.task
                    && existing.message == diag.message
            })
        {
            delta.added.push(diag);
        }
    }
    delta
}

fn split_task_dir(context: &PlanContext) -> PathBuf {
    let conventional = context.identity().plan_root.join("tasks");
    if conventional.is_dir() {
        conventional
    } else {
        context.identity().plan_root.clone()
    }
}

fn default_child_body(parent_id: &str, parent_title: &str) -> String {
    format!(
        "## Why\n\nBounded increment split from `{parent_id}` ({parent_title}).\n\n\
         ## Acceptance criteria\n\n- [ ] (state the observable outcome this increment delivers)\n"
    )
}

fn empty_placeholder_task(id: &str, title: &str, phase: u32) -> Task {
    Task {
        id: id.to_string(),
        title: title.to_string(),
        phase,
        status: Status::Todo,
        owner: None,
        effort_days: None,
        blocked_by: Vec::new(),
        prefer_after: Vec::new(),
        ratified_by: Vec::new(),
        contains: Vec::new(),
        integrates: Vec::new(),
        evidence: Vec::new(),
        evidence_fingerprints: BTreeMap::new(),
        paths: Vec::new(),
        traces: Vec::new(),
        trace_fingerprints: BTreeMap::new(),
        tags: Vec::new(),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
        version: None,
        closed_at: None,
        closed_at_source: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChangeDisposition, ContextInputs, Trust};

    fn fixture(extra_parent: &str, extra_tasks: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("aplan-split-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks dir");
        std::fs::write(
            dir.path().join("aplan/tasks/M-v1.md"),
            "---\nid: M-v1\ntitle: Release one\nphase: 1\nstatus: todo\n\
             tags: [milestone, v1]\nblocked_by: [A1-01]\n---\n\n## Why\n\nCut.\n",
        )
        .expect("cut");
        std::fs::write(
            dir.path().join("aplan/tasks/A1-01.md"),
            format!(
                "---\n# preserve this comment\nid: A1-01\ntitle: Oversized feature\nphase: 1\n\
                 status: in_progress\nowner: ada\neffort_days: 6.0\npaths: [src/feature/**]\n\
                 tags: [core, feature]\n{extra_parent}---\n\n## Why\n\nBig work.\n\n\
                 ## Acceptance criteria\n\n- [x] initial spike notes kept on parent\n- [ ] shipped\n"
            ),
        )
        .expect("parent");
        for (name, body) in extra_tasks {
            std::fs::write(dir.path().join(format!("aplan/tasks/{name}.md")), body).expect("extra");
        }
        dir
    }

    fn ctx(root: &std::path::Path, options: aplan_core::Options) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(&root.join("aplan")),
            ContextInputs {
                repository_root: root.to_path_buf(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options,
                verdict: aplan_core::Verdict {
                    deny_warnings: true,
                    ..aplan_core::Verdict::default()
                },
            },
        )
    }

    fn child(title: &str, effort: f64) -> SplitChildRequest {
        SplitChildRequest {
            title: title.to_string(),
            id: None,
            key: None,
            effort_days: Some(effort),
            owner: None,
            tags: None,
            paths: None,
            traces: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            body: None,
        }
    }

    #[test]
    fn split_preview_and_commit_reconcile_effort_preserve_yaml_and_detect_stale_preview() {
        let dir = fixture("", &[]);
        let mut families = BTreeMap::new();
        families.insert(
            "area".to_string(),
            TagFamily {
                members: vec!["core".to_string(), "cli".to_string()],
                require: Cardinality::ExactlyOne,
                status: StatusScope::Any,
                milestones: aplan_core::MilestoneScope::Exclude,
            },
        );
        let opts = aplan_core::Options {
            require_effort: true,
            max_effort_days: Some(3.0),
            require_cut_membership: true,
            tag_families: families,
            ..aplan_core::Options::default()
        };
        let context = ctx(dir.path(), opts.clone());

        let mut c1 = child("Core engine increment", 2.5);
        c1.key = Some("eng".to_string());
        c1.tags = Some(vec!["perf".to_string()]); // inherits required `core` from parent!
        let mut c2 = child("CLI wiring increment", 3.0);
        c2.blocked_by = vec!["local:eng".to_string()];
        c2.tags = Some(vec!["cli".to_string()]); // overrides `core` with `cli`!

        let req = SplitRequest {
            target: "A1-01".to_string(),
            children: vec![c1, c2],
            parent_effort: SplitParentEffort::Keep(0.5),
            reestimate: false,
            cut: Some("v1".to_string()),
            expected_identity: None,
        };
        let preview = preview_split(&context, &req).expect("valid split preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        // Dry-run does not create files or modify parent.
        assert!(!dir.path().join("aplan/tasks/A1-02.md").exists());
        let machine = preview.machine_result(false);
        assert_eq!(machine.children.len(), 2);
        assert_eq!(machine.children[0].id, "A1-02");
        assert_eq!(machine.children[1].id, "A1-03");
        assert_eq!(machine.children[0].tags, vec!["perf", "core"]);
        assert_eq!(machine.children[1].tags, vec!["cli"]);
        assert_eq!(machine.children[1].blocked_by, vec!["A1-02"]);
        assert_eq!(machine.parent.before_status, Status::InProgress);
        assert_eq!(machine.parent.after_status, Status::Todo);
        assert_eq!(machine.parent.after_effort_days, Some(0.5));
        assert_eq!(machine.parent.checked_criteria_retained, 1);

        // Stale preview guard refuses after any disk mutation.
        let stale_req = SplitRequest {
            expected_identity: Some("0000000000000000".to_string()),
            ..req.clone()
        };
        assert!(preview_split(&context, &stale_req).is_err());

        // Atomic commit succeeds and leaves an immediately clean graph with preserved YAML comments.
        let outcome = commit_split(&context, preview);
        assert!(matches!(outcome, CommitOutcome::Applied(_)), "{outcome:?}");
        let parent_text = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-01.md")).unwrap();
        assert!(
            parent_text.contains("# preserve this comment"),
            "{parent_text}"
        );
        assert!(
            parent_text.contains("- [x] initial spike notes kept on parent"),
            "{parent_text}"
        );
        assert!(parent_text.contains("effort_days: 0.5"), "{parent_text}");
        assert!(parent_text.contains("status: todo"), "{parent_text}");
        let child_text = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-02.md")).unwrap();
        assert!(
            !child_text.contains("- [x]"),
            "never copy checked criteria to child: {child_text}"
        );

        let reloaded = aplan_core::load_plan(&dir.path().join("aplan"));
        let report = aplan_core::validate(&reloaded, &opts);
        assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    }

    #[test]
    fn split_refuses_empty_duplicate_cyclic_cross_cut_terminal_and_already_decomposed() {
        let dir = fixture(
            "",
            &[
                (
                    "M-v2",
                    "---\nid: M-v2\ntitle: Second cut\nphase: 2\nstatus: todo\n\
                     tags: [milestone, v2]\nblocked_by: [A1-05]\n---\n",
                ),
                (
                    "A1-05",
                    "---\nid: A1-05\ntitle: Done task\nphase: 2\nstatus: done\neffort_days: 2\n\
                     paths: [src/d.rs]\nevidence:\n  - kind: note\n    ref: ok\n    captured: 2026-10-01\n---\n",
                ),
            ],
        );
        let context = ctx(dir.path(), aplan_core::Options::default());

        // Empty / single-child split refused.
        let empty = SplitRequest {
            target: "A1-01".to_string(),
            children: vec![child("Only one", 6.0)],
            parent_effort: SplitParentEffort::Omit,
            reestimate: false,
            cut: None,
            expected_identity: None,
        };
        assert!(preview_split(&context, &empty).is_err());

        // Effort reconciliation mismatch refused unless `--reestimate`.
        let mismatch = SplitRequest {
            target: "A1-01".to_string(),
            children: vec![child("One", 2.0), child("Two", 2.0)],
            parent_effort: SplitParentEffort::Omit,
            reestimate: false,
            cut: None,
            expected_identity: None,
        };
        assert!(preview_split(&context, &mismatch).is_err());

        // Cross-cut assertion refused.
        let cross_cut = SplitRequest {
            target: "A1-01".to_string(),
            children: vec![child("One", 3.0), child("Two", 3.0)],
            parent_effort: SplitParentEffort::Omit,
            reestimate: false,
            cut: Some("M-v2".to_string()),
            expected_identity: None,
        };
        assert!(preview_split(&context, &cross_cut).is_err());

        // Terminal task refused.
        let terminal = SplitRequest {
            target: "A1-05".to_string(),
            children: vec![child("One", 1.0), child("Two", 1.0)],
            parent_effort: SplitParentEffort::Omit,
            reestimate: false,
            cut: None,
            expected_identity: None,
        };
        assert!(preview_split(&context, &terminal).is_err());

        // Milestone refused.
        let ms = SplitRequest {
            target: "M-v1".to_string(),
            children: vec![child("One", 1.0), child("Two", 1.0)],
            parent_effort: SplitParentEffort::Omit,
            reestimate: false,
            cut: None,
            expected_identity: None,
        };
        assert!(preview_split(&context, &ms).is_err());

        // Cyclic sibling split refused (parent-ref and sibling-cycle).
        let mut cyc1 = child("One", 3.0);
        cyc1.key = Some("a".to_string());
        cyc1.blocked_by = vec!["local:b".to_string()];
        let mut cyc2 = child("Two", 3.0);
        cyc2.key = Some("b".to_string());
        cyc2.blocked_by = vec!["local:a".to_string()];
        let cyclic = SplitRequest {
            target: "A1-01".to_string(),
            children: vec![cyc1, cyc2],
            parent_effort: SplitParentEffort::Omit,
            reestimate: false,
            cut: None,
            expected_identity: None,
        };
        let cyc_preview = preview_split(&context, &cyclic).expect("builds denied preview");
        assert_eq!(
            cyc_preview.change_set().disposition(),
            ChangeDisposition::Denied
        );
    }

    #[test]
    fn parse_split_spec_and_inline_child_cover_list_mapping_and_invalid_inputs() {
        assert!(parse_split_spec("empty.yml", "   \n").is_err());
        assert!(parse_split_spec("bad-list.yml", "- [1, 2").is_err());
        assert!(parse_split_spec("bad-map.yml", "unknown_field: true\n").is_err());

        let list = parse_split_spec(
            "list.yml",
            "- title: Part one\n  effort: 2.5\n- title: Part two\n  effort_days: 3.5\n",
        )
        .expect("list spec");
        assert!(list.target.is_none());
        assert_eq!(list.children.len(), 2);
        assert_eq!(list.children[0].effort_days, Some(2.5));

        let mapping = parse_split_spec(
            "map.yml",
            "target: A1-01\nkeep_effort: 0.5\nreestimate: true\ncut: v1\n\
             children:\n  - title: First\n    effort: 2\n  - title: Second\n    effort: 2\n",
        )
        .expect("mapping spec");
        assert_eq!(mapping.target.as_deref(), Some("A1-01"));
        assert_eq!(mapping.parent_effort, Some(SplitParentEffort::Keep(0.5)));
        assert_eq!(mapping.reestimate, Some(true));
        assert_eq!(mapping.cut.as_deref(), Some("v1"));

        let plain = parse_inline_child("Just a title").expect("plain inline");
        assert_eq!(plain.title, "Just a title");
        assert_eq!(plain.effort_days, None);
        let sized = parse_inline_child("Bounded=1.5").expect("sized inline");
        assert_eq!(sized.title, "Bounded");
        assert_eq!(sized.effort_days, Some(1.5));
        assert!(parse_inline_child("Bad=not-a-number").is_err());
        assert!(parse_inline_child("  =1.5").is_err());
        assert!(parse_inline_child("   ").is_err());
    }

    fn base_req(target: &str, children: Vec<SplitChildRequest>) -> SplitRequest {
        SplitRequest {
            target: target.to_string(),
            children,
            parent_effort: SplitParentEffort::Omit,
            reestimate: false,
            cut: None,
            expected_identity: None,
        }
    }

    #[test]
    fn preview_split_refuses_malformed_plan_target_cut_and_parent_effort_cases() {
        let dir = fixture(
            "",
            &[
                (
                    "A1-02",
                    "---\nid: A1-02\ntitle: Already split\nphase: 1\nstatus: todo\n\
                     blocked_by: [A1-03]\nintegrates: [A1-03]\n---\n",
                ),
                (
                    "A1-03",
                    "---\nid: A1-03\ntitle: Live with evidence\nphase: 1\nstatus: todo\n\
                     effort_days: 2\nevidence:\n  - kind: note\n    ref: proof\n    captured: 2026-10-01\n---\n",
                ),
                (
                    "A1-04",
                    "---\nid: A1-04\ntitle: Multi-cut\nphase: 1\nstatus: todo\neffort_days: 2\n---\n",
                ),
                (
                    "A1-05",
                    "---\nid: A1-05\ntitle: In terminal cut\nphase: 1\nstatus: todo\neffort_days: 2\n---\n",
                ),
                (
                    "M-dup1",
                    "---\nid: M-dup1\ntitle: Cut A\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [A1-04]\n---\n",
                ),
                (
                    "M-dup2",
                    "---\nid: M-dup2\ntitle: Cut B\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [A1-04]\n---\n",
                ),
                (
                    "M-done",
                    "---\nid: M-done\ntitle: Closed cut\nphase: 1\nstatus: done\ntags: [milestone]\nblocked_by: [A1-05]\n---\n",
                ),
            ],
        );
        let context = ctx(dir.path(), aplan_core::Options::default());
        let pair = || vec![child("One", 3.0), child("Two", 3.0)];

        for target in ["   ", "NOPE", "A1-02", "A1-03", "A1-04", "A1-05"] {
            assert!(
                preview_split(&context, &base_req(target, pair())).is_err(),
                "{target}"
            );
        }
        let mut req = base_req("A1-01", pair());
        req.expected_identity = Some("   ".to_string());
        assert!(preview_split(&context, &req).is_err());

        for bad_cut in ["   ", "NOPE"] {
            let mut r = base_req("A1-01", pair());
            r.cut = Some(bad_cut.to_string());
            assert!(preview_split(&context, &r).is_err());
        }
        for bad_effort in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut r = base_req("A1-01", pair());
            r.parent_effort = SplitParentEffort::Keep(bad_effort);
            assert!(preview_split(&context, &r).is_err());
        }

        // Duplicate target ID and broken load diagnostics are both refused.
        std::fs::write(
            dir.path().join("aplan/tasks/A1-01-copy.md"),
            "---\nid: A1-01\ntitle: Dup\nphase: 1\nstatus: todo\neffort_days: 6\n---\n",
        )
        .expect("dup");
        let dup_ctx = ctx(dir.path(), aplan_core::Options::default());
        assert!(preview_split(&dup_ctx, &base_req("A1-01", pair())).is_err());

        std::fs::write(
            dir.path().join("aplan/tasks/BROKEN.md"),
            "---\nid: [invalid\n---\n",
        )
        .expect("broken");
        let broken_ctx = ctx(dir.path(), aplan_core::Options::default());
        assert!(preview_split(&broken_ctx, &base_req("A1-01", pair())).is_err());
    }

    #[test]
    fn preview_split_refuses_malformed_children_ids_keys_lists_and_refs() {
        let dir = fixture("", &[]);
        let context = ctx(dir.path(), aplan_core::Options::default());
        let check_err = |c1: SplitChildRequest, c2: SplitChildRequest, label: &str| {
            let res = preview_split(&context, &base_req("A1-01", vec![c1, c2]));
            assert!(res.is_err(), "expected refusal for {label}");
        };

        let mut no_eff = child("One", 3.0);
        no_eff.effort_days = None;
        check_err(no_eff, child("Two", 3.0), "omitted child effort");
        check_err(child("   ", 3.0), child("Two", 3.0), "blank title");
        check_err(child("Same", 3.0), child("Same", 3.0), "duplicate title");
        check_err(child("One", 0.0), child("Two", 6.0), "zero child effort");
        check_err(child("One", f64::NAN), child("Two", 6.0), "NaN effort");

        let mut blank_owner = child("One", 3.0);
        blank_owner.owner = Some("  ".to_string());
        check_err(blank_owner, child("Two", 3.0), "blank owner");
        let mut blank_body = child("One", 3.0);
        blank_body.body = Some(" \n ".to_string());
        check_err(blank_body, child("Two", 3.0), "blank body");

        let mut bad_id = child("One", 3.0);
        bad_id.id = Some("../escape".to_string());
        check_err(bad_id, child("Two", 3.0), "unusable id");
        let mut dup_id1 = child("One", 3.0);
        dup_id1.id = Some("A1-90".to_string());
        let mut dup_id2 = child("Two", 3.0);
        dup_id2.id = Some("A1-90".to_string());
        check_err(dup_id1, dup_id2, "duplicate child id");
        let mut existing_id = child("One", 3.0);
        existing_id.id = Some("A1-01".to_string());
        check_err(existing_id, child("Two", 3.0), "existing child id");

        let mut bad_key = child("One", 3.0);
        bad_key.key = Some("bad key!".to_string());
        check_err(bad_key, child("Two", 3.0), "invalid local key");
        let mut dup_k1 = child("One", 3.0);
        dup_k1.key = Some("k".to_string());
        let mut dup_k2 = child("Two", 3.0);
        dup_k2.key = Some("k".to_string());
        check_err(dup_k1, dup_k2, "duplicate local key");

        for (bad_tags, label) in [
            (vec!["  ".to_string()], "blank tag"),
            (vec!["milestone".to_string()], "milestone tag"),
            (vec!["cross-cutting".to_string()], "cross-cutting in cut"),
        ] {
            let mut c = child("One", 3.0);
            c.tags = Some(bad_tags);
            check_err(c, child("Two", 3.0), label);
        }
        let mut bad_path = child("One", 3.0);
        bad_path.paths = Some(vec!["  ".to_string()]);
        check_err(bad_path, child("Two", 3.0), "blank path");
        let mut bad_trace = child("One", 3.0);
        bad_trace.traces = Some(vec!["  ".to_string()]);
        check_err(bad_trace, child("Two", 3.0), "blank trace");

        for (bad_ref, label) in [
            ("  ", "blank dep"),
            ("local:missing", "unknown local key"),
            ("local:selfk", "self dep"),
            ("A1-01", "parent dep"),
            ("GHOST-99", "unknown task dep"),
        ] {
            let mut c = child("One", 3.0);
            c.key = Some("selfk".to_string());
            c.blocked_by = vec![bad_ref.to_string()];
            check_err(c, child("Two", 3.0), label);
        }
    }

    #[test]
    fn preview_split_covers_cross_cutting_inheritance_review_rows_and_flat_plan_dir() {
        let dir = tempfile::Builder::new()
            .prefix("aplan-split-flat-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("aplan")).expect("flat plan dir");
        std::fs::write(
            dir.path().join("aplan/A1-01.md"),
            "---\nid: A1-01\ntitle: Cross-cutting work\nphase: 1\nstatus: todo\n\
             owner: ada\neffort_days: 5\npaths: [src/**]\ntags: [cross-cutting, core]\n---\n\n## Why\n\nX.\n",
        )
        .expect("parent");
        let mut families = BTreeMap::new();
        families.insert(
            "optional_dim".to_string(),
            TagFamily {
                members: vec!["opt1".to_string()],
                require: Cardinality::AtMostOne,
                status: StatusScope::Any,
                milestones: aplan_core::MilestoneScope::Exclude,
            },
        );
        let opts = aplan_core::Options {
            max_effort_days: Some(3.0),
            tag_families: families,
            ..aplan_core::Options::default()
        };
        let context = ctx(dir.path(), opts);

        // Cross-cut assertion against a cutless (`cross-cutting`) parent is refused.
        let mut wrong_cut = base_req("A1-01", vec![child("One", 2.5), child("Two", 2.5)]);
        wrong_cut.cut = Some("M-none".to_string());
        assert!(preview_split(&context, &wrong_cut).is_err());

        // Retaining an oversized parent integration effort still triggers APLAN067 via touched-task check.
        let oversized_keep = SplitRequest {
            target: "A1-01".to_string(),
            children: vec![child("One", 0.5), child("Two", 0.5)],
            parent_effort: SplitParentEffort::Keep(4.0),
            reestimate: false,
            cut: None,
            expected_identity: None,
        };
        let denied_keep = preview_split(&context, &oversized_keep).expect("denied keep preview");
        assert_eq!(
            denied_keep.change_set().disposition(),
            ChangeDisposition::Denied
        );
        assert!(
            denied_keep
                .change_set()
                .diagnostics()
                .added
                .iter()
                .any(|d| d.code == "APLAN067"),
            "{:?}",
            denied_keep.change_set().diagnostics().lines()
        );

        let mut c1 = child("First slice", 2.0);
        c1.key = Some("first".to_string());
        c1.tags = Some(vec!["backend".to_string()]);
        c1.body = Some("## Why\n\nCustom child body.\n".to_string());
        c1.traces = Some(Vec::new());
        let mut c2 = child("Second slice", 2.0);
        c2.blocked_by = vec!["local:first".to_string()];
        let mut c3 = child("Third slice", 1.0);
        c3.prefer_after = vec!["local:first".to_string()];

        let req = SplitRequest {
            target: "A1-01".to_string(),
            children: vec![c1, c2, c3],
            parent_effort: SplitParentEffort::Omit,
            reestimate: true,
            cut: None,
            expected_identity: None,
        };
        let preview = preview_split(&context, &req).expect("cross-cutting preview");
        let rows = preview.review_rows();
        assert!(rows.iter().any(|r| r.contains("cross-cutting")), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains("(reestimated)")), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains("[blocked_by:")), "{rows:?}");
        let machine = preview.machine_result(false);
        assert!(
            machine.children[0]
                .tags
                .iter()
                .any(|t| t == "cross-cutting")
        );
        assert_eq!(machine.cut, None);
        assert!(matches!(
            commit_split(&context, preview),
            CommitOutcome::Applied(_)
        ));
        assert!(dir.path().join("aplan/A1-02.md").exists());
    }
}
