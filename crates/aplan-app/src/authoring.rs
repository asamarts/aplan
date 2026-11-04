//! Structural task-authoring operations shared by interactive clients.
//!
//! Each entry point consumes an identified [`PlanContext`], computes the whole
//! post-state, and returns one immutable [`ChangeSet`]. No entry point writes.
//! The generic guarded publisher in [`crate::commit_change_set`] is the only
//! publication boundary, including for broad imports and rekeys.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use aplan_core::task::VersionImpact;
use aplan_core::{Evidence, IdStrategy, Plan, Status, Task};
pub use aplan_interop::ImplementedAs;
use aplan_interop::ProposalSource;

use crate::{ChangeEvidence, ChangeSet, FileChange, PlanContext, SemanticEffect, Trust, absolute};

/// Input for one native task-creation form.
#[derive(Debug, Clone, PartialEq)]
pub struct AddRequest {
    pub title: String,
    pub id: Option<String>,
    pub phase: Option<u32>,
    pub cut: Option<String>,
    pub effort_days: Option<f64>,
    pub owner: Option<String>,
    /// Initial state. `None` is the ordinary `todo` default.
    pub status: Option<Status>,
    pub blocked_by: Vec<String>,
    pub prefer_after: Vec<String>,
    pub ratified_by: Vec<String>,
    pub contains: Vec<String>,
    /// Existing live work to move from its current cut into the new milestone
    /// this request creates. Empty keeps ordinary single-task add semantics.
    pub members: Vec<String>,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    pub traces: Vec<String>,
    pub revisit_at: Option<String>,
    pub version_impact: Option<VersionImpact>,
    pub version_impact_reason: Option<String>,
    /// Accepted only with `status: done`; resolved before publication.
    pub evidence: Vec<Evidence>,
    /// Markdown returned by the external-editor handoff. `None` keeps the
    /// ordinary scaffold; a blank edited body is refused.
    pub body: Option<String>,
}

/// Captured `aplan add --from` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestRequest {
    pub source_name: String,
    pub contents: String,
}

/// Captured V1 blueprint input plus its explicit application values.
///
/// Delivery adapters capture the selected file before previewing it, just as
/// they do for [`ManifestRequest`]. A stale publication consumes no part of
/// this request: the adapter re-reads the plan and previews the whole graph
/// again so every sequential id is re-minted from one current snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlueprintRequest {
    pub source_name: String,
    pub contents: String,
    pub bindings: BTreeMap<String, String>,
    /// One existing cut which receives every task created by this application.
    pub cut: String,
}

/// Captured proposal import input. There is intentionally no destination:
/// native authoring can only write inside the identified plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    pub source_name: String,
    pub contents: String,
    pub format: String,
    pub phase: u32,
    pub implemented_as: ImplementedAs,
    pub cut: Option<String>,
    pub id_prefix: Option<String>,
    pub tags: Vec<String>,
}

/// Input for filing a successor to one completed outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct SupersedeRequest {
    pub predecessor: String,
    pub title: String,
    pub cut: Option<String>,
    pub effort_days: Option<f64>,
    pub owner: Option<String>,
    pub tags: Vec<String>,
    /// `None` inherits; `Some([])` deliberately clears.
    pub paths: Option<Vec<String>>,
    /// `None` inherits; `Some([])` deliberately clears.
    pub traces: Option<Vec<String>>,
    /// `None` keeps the generated successor template.
    pub body: Option<String>,
}

/// Input for one stable-id migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RekeyRequest {
    pub old: String,
    pub new: String,
    pub allow_terminal: bool,
}

/// One possible reference that a rekey deliberately leaves for review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalReference {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

/// Structural preview plus operation-specific review data.
#[derive(Debug)]
pub struct AuthoringPreview {
    change: ChangeSet,
    created: Vec<String>,
    notices: Vec<aplan_interop::Notice>,
    external_references: Vec<ExternalReference>,
}

impl AuthoringPreview {
    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    #[must_use]
    pub fn created(&self) -> &[String] {
        &self.created
    }

    #[must_use]
    pub fn notices(&self) -> &[aplan_interop::Notice] {
        &self.notices
    }

    #[must_use]
    pub fn external_references(&self) -> &[ExternalReference] {
        &self.external_references
    }

    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }
}

/// A request problem or an input that cannot safely become a preview.
#[derive(Debug, thiserror::Error)]
pub enum AuthoringError {
    #[error("{0}")]
    Arguments(String),
    #[error("{0}")]
    Plan(String),
    #[error("{0}")]
    Refused(String),
    #[error("could not rewrite `{id}`: {source}")]
    Rewrite {
        id: String,
        #[source]
        source: aplan_core::CloseError,
    },
    #[error("{0}")]
    Interop(String),
    #[error("reading {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Preview a single new task and optional cut placement.
pub fn preview_add(
    context: &PlanContext,
    request: &AddRequest,
) -> Result<AuthoringPreview, AuthoringError> {
    require_clean(context)?;
    validate_add_request(context, request)?;

    let cut = resolve_cut_for_status(
        context.plan(),
        request.cut.as_deref(),
        request.status.unwrap_or(Status::Todo),
    )?;
    let phase = request
        .phase
        .unwrap_or_else(|| aplan_interop::ids::phase_for(context.plan(), cut.as_deref()));
    let id = match &request.id {
        Some(id) => id.clone(),
        None => aplan_interop::ids::mint(context.plan(), phase, id_strategy(context))
            .map_err(|error| AuthoringError::Refused(error.to_string()))?,
    };
    validate_new_id(context.plan(), &id)?;
    for (what, relations) in [
        ("blocked_by", &request.blocked_by),
        ("prefer_after", &request.prefer_after),
        ("ratified_by", &request.ratified_by),
        ("contains", &request.contains),
    ] {
        validate_relations(what, context.plan(), relations, &BTreeSet::new())?;
    }

    let mut blocked_by = request.blocked_by.clone();
    blocked_by.extend(request.members.iter().cloned());
    let task = new_task(NewTask {
        id: id.clone(),
        title: request.title.clone(),
        phase,
        owner: request.owner.clone(),
        effort_days: request.effort_days,
        status: request.status.unwrap_or(Status::Todo),
        blocked_by,
        prefer_after: request.prefer_after.clone(),
        ratified_by: request.ratified_by.clone(),
        contains: request.contains.clone(),
        evidence: request.evidence.clone(),
        paths: request.paths.clone(),
        traces: request.traces.clone(),
        tags: request.tags.clone(),
        revisit_at: request.revisit_at.clone(),
        version_impact: request.version_impact,
        version_impact_reason: request.version_impact_reason.clone(),
    });
    if request.members.is_empty() {
        preview_created_tasks(
            context,
            vec![(task, request.body.clone())],
            cut.as_deref(),
            Vec::new(),
        )
    } else {
        preview_new_cut(context, task, request.body.as_deref(), &request.members)
    }
}

fn validate_add_request(context: &PlanContext, request: &AddRequest) -> Result<(), AuthoringError> {
    validate_title(&request.title)?;
    validate_effort(request.effort_days)?;
    validate_strings("tag", &request.tags)?;
    validate_strings("path", &request.paths)?;
    validate_strings("trace", &request.traces)?;
    validate_optional("owner", request.owner.as_deref())?;
    validate_optional(
        "version impact reason",
        request.version_impact_reason.as_deref(),
    )?;
    validate_version_claim(
        request.version_impact,
        request.version_impact_reason.as_deref(),
    )?;
    validate_creation_state(
        request.status,
        request.revisit_at.as_deref(),
        &request.evidence,
    )?;

    if !request.members.is_empty() && request.cut.is_some() {
        return Err(AuthoringError::Arguments(
            "`members` creates a destination cut, so it cannot also join `cut`".to_string(),
        ));
    }
    if !request.members.is_empty() && (request.id.is_none() || request.phase.is_none()) {
        return Err(AuthoringError::Arguments(
            "`members` creates a new cut whose identity is a decision; supply both `id` and `phase`"
                .to_string(),
        ));
    }
    if !request.members.is_empty()
        && !request
            .tags
            .iter()
            .any(|tag| tag == aplan_core::task::MILESTONE_TAG)
    {
        return Err(AuthoringError::Arguments(
            "`members` creates a new cut, so the new task must carry the `milestone` tag"
                .to_string(),
        ));
    }
    if !request.members.is_empty()
        && request
            .tags
            .iter()
            .any(|tag| tag == aplan_core::task::MILESTONE_TAG)
        && (request.effort_days.is_some() || !request.paths.is_empty())
    {
        return Err(AuthoringError::Refused(
            "APLAN021: a milestone cannot carry `effort_days` or `paths`; those fields describe work"
                .to_string(),
        ));
    }

    if !request.members.is_empty() {
        for predecessor in &request.blocked_by {
            let task = unique_task(context.plan(), predecessor)?;
            if !task.task.is_milestone() {
                return Err(AuthoringError::Arguments(format!(
                    "`blocked_by` names work `{predecessor}` while `members` is in use; put work in `members` and reserve `blocked_by` for milestone sequencing"
                )));
            }
        }
    }
    if request
        .body
        .as_deref()
        .is_some_and(aplan_core::task::is_vacuous_ref)
    {
        return Err(AuthoringError::Arguments(
            "the edited body is blank; cancel the editor to keep the scaffold, or write the Markdown the task should carry"
                .to_string(),
        ));
    }
    Ok(())
}

fn preview_new_cut(
    context: &PlanContext,
    task: Task,
    body: Option<&str>,
    members: &[String],
) -> Result<AuthoringPreview, AuthoringError> {
    let moving = aplan_interop::cut::plan_new_cut(context.plan(), members)
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    let path = aplan_interop::emit::task_path(&task_dir(context), &task.id);
    let contents =
        aplan_interop::emit::task_file(&task, body.unwrap_or(aplan_interop::emit::ADD_BODY))
            .map_err(|error| AuthoringError::Interop(error.to_string()))?;
    let mut files = vec![FileChange::creation(path.clone(), contents)];
    let mut effects = vec![
        SemanticEffect::TaskCreated {
            task: task.id.clone(),
            title: task.title.clone(),
        },
        SemanticEffect::CutMembershipAdded {
            cut: task.id.clone(),
            tasks: moving.members().to_vec(),
        },
    ];
    for (source_cut, removal) in moving.removals() {
        let replacement = removal
            .replacement()
            .expect("a planned source membership always produces a replacement");
        let captured = context.source_text(&replacement.path).ok_or_else(|| {
            AuthoringError::Refused(format!(
                "`{source_cut}` changed while the edit was being prepared; reload and try again"
            ))
        })?;
        if captured != replacement.expected {
            return Err(AuthoringError::Refused(format!(
                "`{source_cut}` changed while the edit was being prepared; reload and try again"
            )));
        }
        files.push(FileChange::replacement(
            replacement.path,
            replacement.expected,
            replacement.updated,
        ));
        effects.push(SemanticEffect::CutMembershipRemoved {
            cut: source_cut.clone(),
            tasks: removal.removed().to_vec(),
        });
    }

    let mut after = aplan_core::mutation::with_added(context.plan(), path, task.clone());
    for (source_cut, removal) in moving.removals() {
        if let Some(slot) = after
            .tasks
            .iter_mut()
            .find(|loaded| loaded.task.id == *source_cut)
        {
            slot.task
                .blocked_by
                .retain(|id| !removal.removed().contains(id));
        }
    }
    aplan_interop::cut::check_task_id_label(&after, &task.id, Some(&task.id))
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    Ok(preview(
        context,
        &after,
        files,
        effects,
        vec![task.id],
        Vec::new(),
        Vec::new(),
    ))
}

/// Preview a captured bulk-authoring manifest as one guarded change.
pub fn preview_manifest(
    context: &PlanContext,
    request: &ManifestRequest,
) -> Result<AuthoringPreview, AuthoringError> {
    require_clean(context)?;
    let label = Path::new(&request.source_name);
    let entries = aplan_interop::manifest::parse(label, &request.contents)
        .map_err(|error| AuthoringError::Interop(error.to_string()))?;
    plan_manifest(context, &entries)
}

/// Preview one bound blueprint as an ordinary guarded manifest mutation.
///
/// All sibling ids are minted against one scratch post-state, then every
/// `local:` relationship is resolved before the ordinary manifest planner sees
/// the entries. The planner remains the single owner of creation validation,
/// evidence resolution, cut placement and the complete post-state gate.
///
/// # Errors
///
/// [`AuthoringError`] when parsing, binding, reference resolution, id minting,
/// cut lookup or ordinary authoring validation refuses the complete request.
pub fn preview_blueprint(
    context: &PlanContext,
    request: &BlueprintRequest,
) -> Result<AuthoringPreview, AuthoringError> {
    require_clean(context)?;
    let label = Path::new(&request.source_name);
    let parsed = aplan_interop::blueprint::parse(label, &request.contents)
        .map_err(|error| AuthoringError::Interop(error.to_string()))?;
    let bound = aplan_interop::blueprint::bind_parameters(&parsed, &request.bindings)
        .map_err(|error| AuthoringError::Arguments(error.to_string()))?;
    let keys = aplan_interop::blueprint::application_keys(&bound)
        .map_err(|error| AuthoringError::Arguments(error.to_string()))?;
    let cut = aplan_interop::cut::resolve_id(context.plan(), &request.cut)
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    let assigned = assign_blueprint_ids(context, &bound, &keys, &cut)?;
    let entries = aplan_interop::blueprint::resolve_application(&bound, &assigned, &cut)
        .map_err(|error| AuthoringError::Arguments(error.to_string()))?;
    plan_manifest(context, &entries)
}

/// Preview any currently registered proposal source as one guarded change.
pub fn preview_import(
    context: &PlanContext,
    request: &ImportRequest,
) -> Result<AuthoringPreview, AuthoringError> {
    require_clean(context)?;
    if !aplan_interop::FLAVORS
        .iter()
        .any(|flavor| flavor.name == request.format)
    {
        return Err(AuthoringError::Arguments(format!(
            "unknown import format `{}` (this build reads: {})",
            request.format,
            aplan_interop::flavor_names().join(", ")
        )));
    }
    validate_strings("tag", &request.tags)?;
    if let ImplementedAs::Done { as_of } = &request.implemented_as
        && !aplan_core::is_iso_date(as_of)
    {
        return Err(AuthoringError::Arguments(format!(
            "implemented-as done needs an ISO-8601 YYYY-MM-DD date, not `{as_of}`"
        )));
    }
    let carries_ids = matches!(request.format.as_str(), "spec-kit" | "openspec");
    let id_prefix = request.id_prefix.clone().unwrap_or_else(|| {
        if carries_ids {
            String::new()
        } else {
            "T".to_string()
        }
    });
    if request.id_prefix.is_some()
        && !matches!(
            request.format.as_str(),
            "checklist" | "spec-kit" | "openspec"
        )
    {
        return Err(AuthoringError::Arguments(
            "an id prefix applies only to checklist, spec-kit, and openspec imports".to_string(),
        ));
    }
    if !(carries_ids && id_prefix.is_empty())
        && (id_prefix.trim().is_empty()
            || !id_prefix
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-'))
    {
        return Err(AuthoringError::Arguments(format!(
            "id prefix `{id_prefix}` must be non-blank and alphanumeric (hyphens are allowed)"
        )));
    }
    let options = aplan_interop::Options {
        phase: request.phase,
        implemented_as: request.implemented_as.clone(),
        id_prefix,
    };
    let mut imported = parse_import(request, &options)?;
    if !request.tags.is_empty() {
        let tags = aplan_interop::emit::deduped(&request.tags);
        for task in &mut imported.tasks {
            task.tags.clone_from(&tags);
        }
    }
    validate_imported_tasks(context.plan(), &request.format, &imported)?;
    let cut = resolve_cut(context.plan(), request.cut.as_deref())?;
    let notices = imported.report.notices().to_vec();
    let tasks = imported
        .tasks
        .into_iter()
        .zip(imported.bodies)
        .map(|(task, body)| (task, Some(body)))
        .collect();
    preview_created_tasks(context, tasks, cut.as_deref(), notices)
}

/// Preview a successor, predecessor re-stamp and optional placement together.
pub fn preview_supersede(
    context: &PlanContext,
    request: &SupersedeRequest,
) -> Result<AuthoringPreview, AuthoringError> {
    require_clean(context)?;
    validate_supersede_request(request)?;
    let predecessor = unique_task(context.plan(), &request.predecessor)?;
    if predecessor.task.is_milestone() {
        return Err(AuthoringError::Refused(format!(
            "`{}` is a milestone; supersede one of its members instead",
            request.predecessor
        )));
    }
    if predecessor.task.status != Status::Done {
        return Err(AuthoringError::Refused(format!(
            "`{}` is `{}`; only a done outcome can be superseded",
            request.predecessor, predecessor.task.status
        )));
    }
    let cut = resolve_cut(context.plan(), request.cut.as_deref())?;
    let id = aplan_interop::ids::mint(context.plan(), predecessor.task.phase, id_strategy(context))
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    let successor = successor_task(&predecessor.task, request, &id);
    let fingerprints = if context.options().detect_drift {
        aplan_core::trace_fingerprints(&predecessor.task, context.options().traces_root.as_deref())
    } else {
        BTreeMap::new()
    };
    let original = source(context, &predecessor.path, &request.predecessor)?;
    let restamped = aplan_core::restamp(&original, &fingerprints).map_err(|source| {
        AuthoringError::Rewrite {
            id: request.predecessor.clone(),
            source,
        }
    })?;
    let restamped_task = aplan_core::mutation::task_of(&restamped).map_err(AuthoringError::Plan)?;
    let mut after =
        aplan_core::mutation::with_replaced(context.plan(), &request.predecessor, restamped_task);
    let path = aplan_interop::emit::task_path(&task_dir(context), &id);
    after = aplan_core::mutation::with_added(&after, path.clone(), successor.clone());
    if let Some(cut) = &cut {
        add_membership(&mut after, cut, std::slice::from_ref(&id));
    }
    let body = request
        .body
        .clone()
        .unwrap_or_else(|| successor_body(&predecessor.task, &request.predecessor));
    let mut files = vec![
        FileChange::creation(
            path,
            aplan_interop::emit::task_file(&successor, &body)
                .map_err(|error| AuthoringError::Interop(error.to_string()))?,
        ),
        FileChange::replacement(predecessor.path.clone(), original, restamped),
    ];
    if let Some(cut_id) = &cut
        && let Some(change) = cut_change(context.plan(), cut_id, std::slice::from_ref(&id))?
    {
        files.push(change);
    }
    let mut effects = vec![
        SemanticEffect::TaskCreated {
            task: id.clone(),
            title: successor.title.clone(),
        },
        SemanticEffect::TraceFingerprintsRestamped {
            task: request.predecessor.clone(),
            count: fingerprints.len(),
        },
    ];
    if let Some(cut) = cut {
        effects.push(SemanticEffect::CutMembershipAdded {
            cut,
            tasks: vec![id.clone()],
        });
    }
    Ok(preview(
        context,
        &after,
        files,
        effects,
        vec![id],
        Vec::new(),
        Vec::new(),
    ))
}

fn validate_supersede_request(request: &SupersedeRequest) -> Result<(), AuthoringError> {
    validate_title(&request.title)?;
    validate_effort(request.effort_days)?;
    validate_strings("tag", &request.tags)?;
    validate_optional("owner", request.owner.as_deref())?;
    if request
        .body
        .as_deref()
        .is_some_and(aplan_core::task::is_vacuous_ref)
    {
        return Err(AuthoringError::Arguments(
            "successor body is blank; omit it for the generated template, or supply Markdown"
                .to_string(),
        ));
    }
    if let Some(paths) = &request.paths {
        validate_strings("path", paths)?;
    }
    if let Some(traces) = &request.traces {
        validate_strings("trace", traces)?;
    }
    Ok(())
}

fn successor_task(predecessor: &Task, request: &SupersedeRequest, id: &str) -> Task {
    let tags = if request.tags.is_empty() {
        successor_tags(&predecessor.tags)
    } else {
        aplan_interop::emit::deduped(&request.tags)
    };
    new_task(NewTask {
        id: id.to_string(),
        title: request.title.clone(),
        phase: predecessor.phase,
        owner: request.owner.clone().or_else(|| predecessor.owner.clone()),
        effort_days: request.effort_days,
        status: Status::Todo,
        blocked_by: Vec::new(),
        prefer_after: Vec::new(),
        ratified_by: Vec::new(),
        contains: Vec::new(),
        evidence: Vec::new(),
        paths: request
            .paths
            .clone()
            .unwrap_or_else(|| predecessor.paths.clone()),
        traces: request
            .traces
            .clone()
            .unwrap_or_else(|| predecessor.traces.clone()),
        tags,
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
    })
}

/// Preview a stable-id migration and enumerate unstructured references without
/// rewriting them.
pub fn preview_rekey(
    context: &PlanContext,
    request: &RekeyRequest,
) -> Result<AuthoringPreview, AuthoringError> {
    let (source_task, destination) = validate_rekey_request(context, request)?;
    let (expected, after, mut files) = prepare_rekey(context, request, &destination)?;
    let mut effects = vec![SemanticEffect::TaskRekeyed {
        old: request.old.clone(),
        new: request.new.clone(),
    }];
    effects.extend(files.iter().filter_map(|file| {
        let loaded = context
            .plan()
            .tasks
            .iter()
            .find(|loaded| loaded.path == file.path())?;
        (loaded.task.id != request.old).then(|| SemanticEffect::TaskReferenceRekeyed {
            task: loaded.task.id.clone(),
            old: request.old.clone(),
            new: request.new.clone(),
        })
    }));
    append_rekey_policy(
        context,
        &source_task.task,
        request,
        &mut files,
        &mut effects,
    )?;
    let external_references = rekey_references(context, &request.old, &files);
    let diagnostics = aplan_core::mutation::delta(&expected, &after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    Ok(AuthoringPreview {
        change: ChangeSet::from_parts(
            context.identity().clone(),
            files,
            effects,
            diagnostics,
            denied,
            ChangeEvidence::NotRequired,
        ),
        created: Vec::new(),
        notices: Vec::new(),
        external_references,
    })
}

fn validate_rekey_request<'a>(
    context: &'a PlanContext,
    request: &RekeyRequest,
) -> Result<(&'a aplan_core::load::LoadedTask, PathBuf), AuthoringError> {
    require_clean(context)?;
    if context.identity().trust == Trust::Hermetic {
        return Err(AuthoringError::Refused(
            "rekey reviews possible repository prose references, which requires a repository-read context"
                .to_string(),
        ));
    }
    if request.old == request.new {
        return Err(AuthoringError::Arguments(format!(
            "`{}` is already the requested id",
            request.old
        )));
    }
    if !aplan_interop::emit::is_usable_id(&request.new) {
        return Err(AuthoringError::Arguments(format!(
            "`{}` cannot be used as a task id and file stem",
            request.new
        )));
    }
    let source_task = unique_task(context.plan(), &request.old)?;
    if context
        .plan()
        .tasks
        .iter()
        .any(|task| task.task.id == request.new)
    {
        return Err(AuthoringError::Refused(format!(
            "task `{}` already exists",
            request.new
        )));
    }
    aplan_interop::cut::check_task_id_label(context.plan(), &request.new, Some(&request.old))
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    if source_task.task.status.is_terminal() && !request.allow_terminal {
        return Err(AuthoringError::Refused(format!(
            "`{}` is `{}`; rekeying terminal history requires the explicit terminal override",
            request.old, source_task.task.status
        )));
    }
    let claims = crate::claims::snapshot(&context.identity().plan_root, context.plan())
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    if let Some(claim) = claims.claim(&request.old) {
        return Err(AuthoringError::Refused(format!(
            "`{}` is claimed by `{}`; release the claim before rekeying it",
            request.old, claim.holder
        )));
    }
    let destination = conventional_rekey_path(&source_task.path, &request.old, &request.new);
    if destination != source_task.path && destination.symlink_metadata().is_ok() {
        return Err(AuthoringError::Refused(format!(
            "destination {} already exists",
            destination.display()
        )));
    }
    Ok((source_task, destination))
}

fn prepare_rekey(
    context: &PlanContext,
    request: &RekeyRequest,
    destination: &Path,
) -> Result<(Plan, Plan, Vec<FileChange>), AuthoringError> {
    let expected = rekey_plan_model(context.plan(), &request.old, &request.new, destination);
    let mut after = aplan_core::mutation::clone_of(context.plan());
    let mut files = Vec::new();
    for loaded in &context.plan().tasks {
        let original = source(context, &loaded.path, &loaded.task.id)?;
        let updated = rekey_task_file(&original, &loaded.task, &request.old, &request.new)?;
        let produced = aplan_core::mutation::task_of(&updated).map_err(AuthoringError::Plan)?;
        let slot = after
            .tasks
            .iter_mut()
            .find(|candidate| candidate.path == loaded.path)
            .expect("a cloned plan retains every loaded path");
        slot.task = produced;
        if loaded.task.id == request.old {
            slot.path = destination.to_path_buf();
            files.push(FileChange::relocation(
                loaded.path.clone(),
                destination.to_path_buf(),
                original,
                updated,
            ));
        } else if updated != original {
            files.push(FileChange::replacement(
                loaded.path.clone(),
                original,
                updated,
            ));
        }
    }
    validate_rekey_model(&expected, &after, &request.old)?;
    Ok((expected, after, files))
}

fn validate_rekey_model(expected: &Plan, produced: &Plan, old: &str) -> Result<(), AuthoringError> {
    for wanted in &expected.tasks {
        let actual = produced
            .tasks
            .iter()
            .find(|task| task.path == wanted.path)
            .ok_or_else(|| {
                AuthoringError::Plan(format!(
                    "the rekeyed plan lost {} while validating its rewrite",
                    wanted.path.display()
                ))
            })?;
        if actual.task != wanted.task {
            return Err(AuthoringError::Plan(format!(
                "the rekey rewrite of `{old}` did not produce its planned structured state"
            )));
        }
    }
    Ok(())
}

fn append_rekey_policy(
    context: &PlanContext,
    source_task: &Task,
    request: &RekeyRequest,
    files: &mut Vec<FileChange>,
    effects: &mut Vec<SemanticEffect>,
) -> Result<(), AuthoringError> {
    let policy_names_old = source_task.is_milestone()
        && aplan_core::cut_label(source_task) != request.old
        && context
            .report_config()
            .and_then(|config| config.policy.as_deref())
            .unwrap_or_default()
            .iter()
            .any(|policy| policy.when.cut.iter().any(|cut| cut == &request.old));
    if !policy_names_old {
        return Ok(());
    }
    let path = context
        .identity()
        .configuration
        .path
        .as_deref()
        .ok_or_else(|| {
            AuthoringError::Refused(format!(
                "the selected policy names `{}`, but no configuration file can be updated",
                request.old
            ))
        })?;
    if !is_within(path, &context.identity().repository_root) {
        return Err(AuthoringError::Refused(format!(
            "the selected configuration {} is outside the repository; rekey will not rewrite it",
            path.display()
        )));
    }
    let original = std::fs::read_to_string(path).map_err(|source| AuthoringError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let updated = aplan_config::rekey_policy_cuts(&original, &request.old, &request.new)
        .map_err(AuthoringError::Interop)?
        .ok_or_else(|| {
            AuthoringError::Refused(format!(
                "the parsed policy names `{}`, but its source location could not be rewritten",
                request.old
            ))
        })?;
    files.push(FileChange::replacement(
        path.to_path_buf(),
        original,
        updated,
    ));
    effects.push(SemanticEffect::PolicyReferenceRekeyed {
        path: path.to_path_buf(),
        old: request.old.clone(),
        new: request.new.clone(),
    });
    Ok(())
}

fn rekey_references(
    context: &PlanContext,
    old: &str,
    files: &[FileChange],
) -> Vec<ExternalReference> {
    let planned_paths = files
        .iter()
        .flat_map(|file| {
            file.source_path()
                .into_iter()
                .chain(std::iter::once(file.path()))
        })
        .map(absolute)
        .collect::<BTreeSet<_>>();
    let mut found = prose_occurrences(&context.identity().repository_root, old, &planned_paths);
    for document in &context.documents().documents {
        if is_within(&document.path, &context.identity().repository_root) {
            continue;
        }
        if let Ok(text) = &document.content {
            found.extend(text_occurrences(&document.path, text, old));
        }
    }
    found.sort_by(|left, right| {
        (&left.path, left.line, left.column).cmp(&(&right.path, right.line, right.column))
    });
    found.dedup();
    found
}

fn preview_created_tasks(
    context: &PlanContext,
    tasks: Vec<(Task, Option<String>)>,
    cut: Option<&str>,
    notices: Vec<aplan_interop::Notice>,
) -> Result<AuthoringPreview, AuthoringError> {
    let mut after = aplan_core::mutation::clone_of(context.plan());
    let mut files = Vec::new();
    let mut effects = Vec::new();
    let mut created = Vec::new();
    let mut pending = Vec::new();
    let dir = task_dir(context);
    for (task, body) in tasks {
        validate_new_id(&after, &task.id)?;
        let path = aplan_interop::emit::task_path(&dir, &task.id);
        effects.push(SemanticEffect::TaskCreated {
            task: task.id.clone(),
            title: task.title.clone(),
        });
        created.push(task.id.clone());
        after = aplan_core::mutation::with_added(&after, path.clone(), task.clone());
        pending.push((task, body, path));
    }
    for id in &created {
        aplan_interop::cut::check_task_id_label(&after, id, Some(id))
            .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    }
    let evidence = resolve_created_ledger(context, &mut after, &created)?;
    for (task, body, path) in pending {
        let final_task = after
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == task.id)
            .map(|loaded| &loaded.task)
            .expect("created task remains present");
        let contents = aplan_interop::emit::task_file(
            final_task,
            body.as_deref().unwrap_or(aplan_interop::emit::ADD_BODY),
        )
        .map_err(|error| AuthoringError::Interop(error.to_string()))?;
        files.push(FileChange::creation(path, contents));
    }
    if let Some(cut_id) = cut {
        add_membership(&mut after, cut_id, &created);
        if let Some(change) = cut_change_for_members(context.plan(), &after, cut_id, &created)? {
            files.push(change);
        }
        effects.push(SemanticEffect::CutMembershipAdded {
            cut: cut_id.to_string(),
            tasks: created.clone(),
        });
    }
    effects.extend(notices.iter().map(|notice| SemanticEffect::ImportNotice {
        source: notice.source.clone(),
        kind: notice.kind.label().to_string(),
        message: notice.message.clone(),
    }));
    Ok(preview_with_evidence(
        context,
        &after,
        files,
        effects,
        created,
        notices,
        Vec::new(),
        evidence,
    ))
}

fn assign_blueprint_ids(
    context: &PlanContext,
    blueprint: &aplan_interop::blueprint::Blueprint,
    keys: &[String],
    cut: &str,
) -> Result<BTreeMap<String, String>, AuthoringError> {
    debug_assert_eq!(blueprint.tasks.len(), keys.len());
    let mut allocation = aplan_core::mutation::clone_of(context.plan());
    let mut assigned = BTreeMap::new();
    for (index, (entry, key)) in blueprint.tasks.iter().zip(keys).enumerate() {
        let at = |message: String| {
            AuthoringError::Arguments(format!("blueprint task {} (`{key}`): {message}", index + 1))
        };
        let phase = entry
            .task
            .phase
            .unwrap_or_else(|| aplan_interop::ids::phase_for(&allocation, Some(cut)));
        let id = aplan_interop::ids::mint(&allocation, phase, id_strategy(context))
            .map_err(|error| at(error.to_string()))?;
        validate_new_id(&allocation, &id).map_err(|error| at(error.to_string()))?;
        let scratch = manifest_task(&entry.task, &id, phase);
        let path = aplan_interop::emit::task_path(&task_dir(context), &id);
        allocation = aplan_core::mutation::with_added(&allocation, path, scratch);
        assigned.insert(key.clone(), id);
    }
    Ok(assigned)
}

fn plan_manifest(
    context: &PlanContext,
    entries: &[aplan_interop::manifest::Entry],
) -> Result<AuthoringPreview, AuthoringError> {
    let ManifestPlan {
        mut after,
        tasks,
        joined,
        reverse_edges,
    } = prepare_manifest(context, entries)?;
    let created_ids = tasks
        .iter()
        .map(|(task, _)| task.id.clone())
        .collect::<Vec<_>>();
    let evidence = resolve_created_ledger(context, &mut after, &created_ids)?;
    let dir = task_dir(context);
    let mut files = Vec::new();
    let mut effects = Vec::new();
    let mut created = Vec::new();
    for (task, body) in tasks {
        let final_task = after
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == task.id)
            .map(|loaded| &loaded.task)
            .expect("the manifest post-state retains every created task");
        files.push(FileChange::creation(
            aplan_interop::emit::task_path(&dir, &task.id),
            aplan_interop::emit::task_file(
                final_task,
                body.as_deref().unwrap_or(aplan_interop::emit::ADD_BODY),
            )
            .map_err(|error| AuthoringError::Interop(error.to_string()))?,
        ));
        effects.push(SemanticEffect::TaskCreated {
            task: task.id.clone(),
            title: task.title,
        });
        created.push(task.id);
    }
    files.extend(existing_blocked_by_changes(context, &after)?);
    for (cut, members) in joined {
        effects.push(SemanticEffect::CutMembershipAdded {
            cut,
            tasks: members,
        });
    }
    effects.extend(
        reverse_edges
            .into_iter()
            .map(|(task, after)| SemanticEffect::DependencyAdded { task, after }),
    );
    Ok(preview_with_evidence(
        context,
        &after,
        files,
        effects,
        created,
        Vec::new(),
        Vec::new(),
        evidence,
    ))
}

struct ManifestPlan {
    after: Plan,
    tasks: Vec<(Task, Option<String>)>,
    joined: BTreeMap<String, Vec<String>>,
    reverse_edges: Vec<(String, String)>,
}

fn prepare_manifest(
    context: &PlanContext,
    entries: &[aplan_interop::manifest::Entry],
) -> Result<ManifestPlan, AuthoringError> {
    let declared = entries
        .iter()
        .filter_map(|entry| entry.id.as_ref().map(|id| (id.clone(), entry)))
        .collect::<BTreeMap<_, _>>();
    let declared_ids = declared.keys().cloned().collect::<BTreeSet<_>>();
    let mut after = aplan_core::mutation::clone_of(context.plan());
    let mut tasks = Vec::new();
    let mut joined: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let at = |message: String| {
            AuthoringError::Arguments(format!(
                "manifest entry {} (`{}`): {message}",
                index + 1,
                entry.title
            ))
        };
        validate_manifest_entry(entry).map_err(|error| at(error.to_string()))?;
        let cut = match entry.cut.as_deref() {
            None => None,
            Some(name) => match aplan_interop::cut::resolve_id(&after, name) {
                Ok(id) => {
                    aplan_interop::cut::check_target_for_status(
                        &after,
                        &id,
                        entry.status.unwrap_or(Status::Todo),
                    )
                    .map_err(|error| at(error.to_string()))?;
                    Some(id)
                }
                Err(aplan_interop::cut::CutError::NoSuchCut { .. })
                    if declared.get(name).is_some_and(|candidate| {
                        candidate
                            .tags
                            .iter()
                            .any(|tag| tag == aplan_core::task::MILESTONE_TAG)
                    }) =>
                {
                    let target = declared.get(name).expect("the guarded declaration exists");
                    aplan_interop::cut::check_status_compatibility(
                        name,
                        target.status.unwrap_or(Status::Todo),
                        entry.status.unwrap_or(Status::Todo),
                    )
                    .map_err(|error| at(error.to_string()))?;
                    Some(name.to_string())
                }
                Err(error) => return Err(at(error.to_string())),
            },
        };
        let phase = entry
            .phase
            .unwrap_or_else(|| aplan_interop::ids::phase_for(&after, cut.as_deref()));
        let id = match &entry.id {
            Some(id) => id.clone(),
            None => aplan_interop::ids::mint(&after, phase, id_strategy(context))
                .map_err(|error| at(error.to_string()))?,
        };
        validate_new_id(&after, &id).map_err(|error| at(error.to_string()))?;
        for (what, relations) in [
            ("blocked_by", &entry.blocked_by),
            ("blocks", &entry.blocks),
            ("prefer_after", &entry.prefer_after),
            ("ratified_by", &entry.ratified_by),
            ("contains", &entry.contains),
        ] {
            validate_relations(what, &after, relations, &declared_ids)
                .map_err(|error| at(error.to_string()))?;
        }
        let task = manifest_task(entry, &id, phase);
        let path = aplan_interop::emit::task_path(&task_dir(context), &id);
        after = aplan_core::mutation::with_added(&after, path, task.clone());
        if let Some(cut) = cut {
            joined.entry(cut).or_default().push(id.clone());
        }
        tasks.push((task, entry.body.clone()));
    }
    for (cut, members) in &joined {
        add_membership(&mut after, cut, members);
    }
    let reverse_edges = add_manifest_reverse_edges(&mut after, entries, &tasks)?;
    for (task, _) in &tasks {
        aplan_interop::cut::check_task_id_label(&after, &task.id, Some(&task.id))
            .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    }
    Ok(ManifestPlan {
        after,
        tasks,
        joined,
        reverse_edges,
    })
}

fn add_manifest_reverse_edges(
    after: &mut Plan,
    entries: &[aplan_interop::manifest::Entry],
    tasks: &[(Task, Option<String>)],
) -> Result<Vec<(String, String)>, AuthoringError> {
    let mut added = Vec::new();
    for (index, (entry, (blocker, _))) in entries.iter().zip(tasks).enumerate() {
        let mut seen = BTreeSet::new();
        for target in &entry.blocks {
            if !seen.insert(target) {
                return Err(AuthoringError::Arguments(format!(
                    "manifest entry {} (`{}`): `blocks` names `{target}` more than once",
                    index + 1,
                    entry.title
                )));
            }
            let target_task = after
                .tasks
                .iter()
                .find(|loaded| loaded.task.id == *target)
                .ok_or_else(|| {
                    AuthoringError::Arguments(format!(
                        "manifest entry {} (`{}`): `blocks` names task `{target}` that is not declared",
                        index + 1,
                        entry.title
                    ))
                })?;
            if target_task.task.status.is_terminal() {
                return Err(AuthoringError::Refused(format!(
                    "cannot make settled task `{target}` wait on `{}`; revive it before changing its dependencies",
                    blocker.id
                )));
            }
            if let Some(why) =
                aplan_core::mutation::membership_edit(after, target, &blocker.id, true)
            {
                return Err(AuthoringError::Refused(why));
            }
            let target_task = after
                .tasks
                .iter_mut()
                .find(|loaded| loaded.task.id == *target)
                .expect("the reverse dependency target was just found");
            if !target_task.task.blocked_by.contains(&blocker.id) {
                target_task.task.blocked_by.push(blocker.id.clone());
                added.push((target.clone(), blocker.id.clone()));
            }
        }
    }
    Ok(added)
}

fn validate_manifest_entry(entry: &aplan_interop::manifest::Entry) -> Result<(), AuthoringError> {
    validate_title(&entry.title)?;
    validate_effort(entry.effort)?;
    validate_strings("tag", &entry.tags)?;
    validate_strings("path", &entry.paths)?;
    validate_strings("trace", &entry.traces)?;
    validate_optional("owner", entry.owner.as_deref())?;
    validate_creation_state(entry.status, entry.revisit_at.as_deref(), &entry.evidence)?;
    validate_optional(
        "version_impact_reason",
        entry.version_impact_reason.as_deref(),
    )?;
    validate_version_claim(entry.version_impact, entry.version_impact_reason.as_deref())?;
    if entry
        .body
        .as_deref()
        .is_some_and(aplan_core::task::is_vacuous_ref)
    {
        return Err(AuthoringError::Arguments(
            "body is blank; omit it to keep the scaffold, or provide Markdown".to_string(),
        ));
    }
    Ok(())
}

fn manifest_task(entry: &aplan_interop::manifest::Entry, id: &str, phase: u32) -> Task {
    new_task(NewTask {
        id: id.to_string(),
        title: entry.title.clone(),
        phase,
        owner: entry.owner.clone(),
        effort_days: entry.effort,
        status: entry.status.unwrap_or(Status::Todo),
        blocked_by: entry.blocked_by.clone(),
        prefer_after: entry.prefer_after.clone(),
        ratified_by: entry.ratified_by.clone(),
        contains: entry.contains.clone(),
        evidence: entry.evidence.clone(),
        paths: entry.paths.clone(),
        traces: entry.traces.clone(),
        tags: entry.tags.clone(),
        revisit_at: entry.revisit_at.clone(),
        version_impact: entry.version_impact,
        version_impact_reason: entry.version_impact_reason.clone(),
    })
}

fn preview(
    context: &PlanContext,
    after: &Plan,
    files: Vec<FileChange>,
    effects: Vec<SemanticEffect>,
    created: Vec<String>,
    notices: Vec<aplan_interop::Notice>,
    external_references: Vec<ExternalReference>,
) -> AuthoringPreview {
    preview_with_evidence(
        context,
        after,
        files,
        effects,
        created,
        notices,
        external_references,
        ChangeEvidence::NotRequired,
    )
}

#[allow(clippy::too_many_arguments)]
fn preview_with_evidence(
    context: &PlanContext,
    after: &Plan,
    files: Vec<FileChange>,
    effects: Vec<SemanticEffect>,
    created: Vec<String>,
    notices: Vec<aplan_interop::Notice>,
    external_references: Vec<ExternalReference>,
    evidence: ChangeEvidence,
) -> AuthoringPreview {
    let diagnostics = aplan_core::mutation::delta(context.plan(), after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    AuthoringPreview {
        change: ChangeSet::from_parts(
            context.identity().clone(),
            files,
            effects,
            diagnostics,
            denied,
            evidence,
        ),
        created,
        notices,
        external_references,
    }
}

fn parse_import(
    request: &ImportRequest,
    options: &aplan_interop::Options,
) -> Result<aplan_interop::Import, AuthoringError> {
    use aplan_interop::{
        checklist::Checklist,
        kep::Kep,
        roadmap_json::RoadmapJson,
        spec_kit::{OpenSpec, SpecKit},
    };
    let parsed = match request.format.as_str() {
        "kep" => Kep.parse(&request.source_name, &request.contents, options),
        "checklist" => Checklist.parse(&request.source_name, &request.contents, options),
        "roadmap-json" => RoadmapJson.parse(&request.source_name, &request.contents, options),
        "spec-kit" => SpecKit.parse(&request.source_name, &request.contents, options),
        "openspec" => OpenSpec.parse(&request.source_name, &request.contents, options),
        other => {
            return Err(AuthoringError::Interop(format!(
                "registered import format `{other}` has no application parser"
            )));
        }
    };
    parsed.map_err(|error| AuthoringError::Interop(error.to_string()))
}

fn validate_imported_tasks(
    plan: &Plan,
    format: &str,
    imported: &aplan_interop::Import,
) -> Result<(), AuthoringError> {
    if imported.tasks.len() != imported.bodies.len() {
        return Err(AuthoringError::Interop(format!(
            "{format} importer returned {} task(s) but {} body value(s)",
            imported.tasks.len(),
            imported.bodies.len()
        )));
    }
    let mut seen = BTreeSet::new();
    for task in &imported.tasks {
        validate_title(&task.title)?;
        validate_new_id(plan, &task.id)?;
        if !seen.insert(task.id.clone()) {
            return Err(AuthoringError::Arguments(format!(
                "the source declares `{}` more than once",
                task.id
            )));
        }
    }
    Ok(())
}

fn cut_change(
    plan: &Plan,
    cut: &str,
    ids: &[String],
) -> Result<Option<FileChange>, AuthoringError> {
    let pending = aplan_interop::cut::plan_placement(plan, cut, ids)
        .map_err(|error| AuthoringError::Interop(error.to_string()))?;
    Ok(pending.replacement().map(|replacement| {
        FileChange::replacement(replacement.path, replacement.expected, replacement.updated)
    }))
}

fn cut_change_for_members(
    plan: &Plan,
    after: &Plan,
    cut: &str,
    ids: &[String],
) -> Result<Option<FileChange>, AuthoringError> {
    let members = ids
        .iter()
        .map(|id| {
            after
                .tasks
                .iter()
                .find(|loaded| loaded.task.id == *id)
                .map(|loaded| (id.clone(), loaded.task.status))
                .ok_or_else(|| {
                    AuthoringError::Plan(format!(
                        "manifest post-state lost created task `{id}` before cut placement"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pending = aplan_interop::cut::plan_placement_for_members(plan, cut, &members)
        .map_err(|error| AuthoringError::Interop(error.to_string()))?;
    Ok(pending.replacement().map(|replacement| {
        FileChange::replacement(replacement.path, replacement.expected, replacement.updated)
    }))
}

fn existing_blocked_by_changes(
    context: &PlanContext,
    after: &Plan,
) -> Result<Vec<FileChange>, AuthoringError> {
    let mut changes = Vec::new();
    for before in &context.plan().tasks {
        let Some(final_task) = after
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == before.task.id)
        else {
            continue;
        };
        if before.task.blocked_by == final_task.task.blocked_by {
            continue;
        }
        let original = context.source_text(&before.path).ok_or_else(|| {
            AuthoringError::Plan(format!(
                "`{}` changed while the manifest was being prepared",
                before.task.id
            ))
        })?;
        let updated = aplan_core::close::set_blocked_by(original, &final_task.task.blocked_by)
            .map_err(|source| AuthoringError::Rewrite {
                id: before.task.id.clone(),
                source,
            })?;
        changes.push(FileChange::replacement(
            before.path.clone(),
            original.to_string(),
            updated,
        ));
    }
    Ok(changes)
}

fn resolve_created_ledger(
    context: &PlanContext,
    after: &mut Plan,
    ids: &[String],
) -> Result<ChangeEvidence, AuthoringError> {
    let mut unresolved = Vec::new();
    let mut observed = Vec::new();
    let mut found = false;
    for id in ids {
        let Some(loaded) = after
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == *id && loaded.task.status == Status::Done)
            .cloned()
        else {
            continue;
        };
        found = true;
        let answer = aplan_evidence::resolve_proposed(
            &loaded.task,
            &loaded.path,
            &loaded.task.evidence,
            &context.identity().repository_root,
            &crate::evidence_mutation::close_options(context, false),
        )
        .map_err(|error| AuthoringError::Interop(format!("resolving `{id}` evidence: {error}")))?;
        if answer.refuses(context.verdict()) {
            let details = answer
                .report
                .diagnostics
                .iter()
                .map(|finding| format!("{}: {}", finding.code, finding.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AuthoringError::Refused(format!(
                "ledger entry `{id}` evidence would fail verification: {details}"
            )));
        }
        unresolved.extend(
            loaded
                .task
                .evidence
                .iter()
                .filter(|citation| answer.unchecked.contains(&citation.kind))
                .cloned(),
        );
        observed.extend(answer.citations);
        let traces = if context.options().detect_drift {
            aplan_core::trace_fingerprints(&loaded.task, context.options().traces_root.as_deref())
        } else {
            BTreeMap::new()
        };
        let evidence = if context.options().detect_drift {
            aplan_evidence::update_evidence_fingerprints(
                &[],
                &BTreeMap::new(),
                &loaded.task.evidence,
                &context.identity().repository_root,
                false,
            )
            .map_err(|error| {
                AuthoringError::Interop(format!("fingerprinting `{id}` evidence: {error}"))
            })?
        } else {
            BTreeMap::new()
        };
        let task = &mut after
            .tasks
            .iter_mut()
            .find(|candidate| candidate.task.id == *id)
            .expect("created ledger entry remains present")
            .task;
        task.trace_fingerprints = traces;
        task.evidence_fingerprints = evidence;
    }
    Ok(if found {
        ChangeEvidence::Resolution {
            unresolved,
            observed,
        }
    } else {
        ChangeEvidence::NotRequired
    })
}

fn resolve_cut(plan: &Plan, cut: Option<&str>) -> Result<Option<String>, AuthoringError> {
    let Some(cut) = cut else {
        return Ok(None);
    };
    let id = aplan_interop::cut::resolve_id(plan, cut)
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    aplan_interop::cut::check_target(plan, &id)
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    Ok(Some(id))
}

fn resolve_cut_for_status(
    plan: &Plan,
    cut: Option<&str>,
    status: Status,
) -> Result<Option<String>, AuthoringError> {
    let Some(cut) = cut else {
        return Ok(None);
    };
    let id = aplan_interop::cut::resolve_id(plan, cut)
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    aplan_interop::cut::check_target_for_status(plan, &id, status)
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    Ok(Some(id))
}

fn add_membership(plan: &mut Plan, cut: &str, ids: &[String]) {
    if let Some(slot) = plan.tasks.iter_mut().find(|loaded| loaded.task.id == cut) {
        for id in ids {
            if !slot.task.blocked_by.contains(id) {
                slot.task.blocked_by.push(id.clone());
            }
        }
    }
}

struct NewTask {
    id: String,
    title: String,
    phase: u32,
    owner: Option<String>,
    effort_days: Option<f64>,
    status: Status,
    blocked_by: Vec<String>,
    prefer_after: Vec<String>,
    ratified_by: Vec<String>,
    contains: Vec<String>,
    evidence: Vec<Evidence>,
    paths: Vec<String>,
    traces: Vec<String>,
    tags: Vec<String>,
    revisit_at: Option<String>,
    version_impact: Option<VersionImpact>,
    version_impact_reason: Option<String>,
}

fn new_task(spec: NewTask) -> Task {
    let NewTask {
        id,
        title,
        phase,
        owner,
        effort_days,
        status,
        blocked_by,
        prefer_after,
        ratified_by,
        contains,
        evidence,
        paths,
        traces,
        tags,
        revisit_at,
        version_impact,
        version_impact_reason,
    } = spec;
    Task {
        id,
        title,
        phase,
        status,
        owner,
        effort_days,
        blocked_by: aplan_interop::emit::deduped(&blocked_by),
        prefer_after: aplan_interop::emit::deduped(&prefer_after),
        ratified_by: aplan_interop::emit::deduped(&ratified_by),
        contains: aplan_interop::emit::deduped(&contains),
        evidence,
        paths: aplan_interop::emit::deduped(&paths),
        traces: aplan_interop::emit::deduped(&traces),
        trace_fingerprints: BTreeMap::new(),
        evidence_fingerprints: BTreeMap::new(),
        tags: aplan_interop::emit::deduped(&tags),
        revisit_at,
        version_impact,
        version_impact_reason,
        version: None,
    }
}

fn require_clean(context: &PlanContext) -> Result<(), AuthoringError> {
    if context.plan().load_diagnostics.is_empty() {
        Ok(())
    } else {
        Err(AuthoringError::Plan(
            "the plan does not load cleanly; repair it before structural authoring".to_string(),
        ))
    }
}

fn validate_title(title: &str) -> Result<(), AuthoringError> {
    if aplan_core::task::is_vacuous_ref(title) {
        Err(AuthoringError::Arguments(
            "a task needs a title: it is how anyone finds the task again".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_effort(effort: Option<f64>) -> Result<(), AuthoringError> {
    if effort.is_some_and(|days| !days.is_finite() || days < 0.0) {
        Err(AuthoringError::Arguments(
            "effort must be a finite, non-negative number of days".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_creation_state(
    status: Option<Status>,
    revisit_at: Option<&str>,
    evidence: &[Evidence],
) -> Result<(), AuthoringError> {
    let status = status.unwrap_or(Status::Todo);
    if status == Status::Dropped {
        return Err(AuthoringError::Refused(
            "`dropped` records a decision about existing work; create live work and use the guarded drop operation if that decision is later made"
                .to_string(),
        ));
    }
    if status == Status::Done && evidence.is_empty() {
        return Err(AuthoringError::Refused(
            "a `done` ledger entry needs evidence that can be resolved before it is written"
                .to_string(),
        ));
    }
    if status != Status::Done && !evidence.is_empty() {
        return Err(AuthoringError::Refused(
            "creation evidence is accepted only for a `done` ledger entry; live work proves completion later through `close`"
                .to_string(),
        ));
    }
    if let Some(at) = revisit_at {
        if status != Status::Blocked {
            return Err(AuthoringError::Refused(
                "`revisit_at` belongs only to an initially parked (`blocked`) task".to_string(),
            ));
        }
        if aplan_core::validate::rfc3339_to_epoch(at).is_none() {
            return Err(AuthoringError::Arguments(format!(
                "`revisit_at` must be an RFC 3339 instant, not `{at}`"
            )));
        }
    }
    Ok(())
}

fn validate_version_claim(
    impact: Option<VersionImpact>,
    reason: Option<&str>,
) -> Result<(), AuthoringError> {
    if impact.is_none() && reason.is_some() {
        Err(AuthoringError::Arguments(
            "a version impact reason requires the version impact it explains".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_strings(what: &str, values: &[String]) -> Result<(), AuthoringError> {
    if values
        .iter()
        .any(|value| aplan_core::task::is_vacuous_ref(value))
    {
        Err(AuthoringError::Arguments(format!(
            "a {what} may not be blank"
        )))
    } else {
        Ok(())
    }
}

fn validate_optional(what: &str, value: Option<&str>) -> Result<(), AuthoringError> {
    if value.is_some_and(aplan_core::task::is_vacuous_ref) {
        Err(AuthoringError::Arguments(format!(
            "{what} may not be blank"
        )))
    } else {
        Ok(())
    }
}

fn validate_new_id(plan: &Plan, id: &str) -> Result<(), AuthoringError> {
    if !aplan_interop::emit::is_usable_id(id) {
        return Err(AuthoringError::Arguments(format!(
            "`{id}` cannot be used as a task id and file stem"
        )));
    }
    if let Some(existing) = plan.tasks.iter().find(|task| task.task.id == id) {
        return Err(AuthoringError::Refused(format!(
            "task `{id}` already exists in {}",
            existing.path.display()
        )));
    }
    aplan_interop::cut::check_task_id_label(plan, id, None)
        .map_err(|error| AuthoringError::Refused(error.to_string()))?;
    Ok(())
}

fn validate_relations(
    what: &str,
    plan: &Plan,
    relations: &[String],
    declared: &BTreeSet<String>,
) -> Result<(), AuthoringError> {
    if relations
        .iter()
        .any(|id| aplan_core::task::is_vacuous_ref(id))
    {
        return Err(AuthoringError::Arguments(
            "a related task id may not be blank or visually empty".to_string(),
        ));
    }
    let unknown = relations
        .iter()
        .filter(|id| {
            !plan.tasks.iter().any(|task| task.task.id == ***id) && !declared.contains(*id)
        })
        .cloned()
        .collect::<Vec<_>>();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(AuthoringError::Arguments(format!(
            "`{what}` names task(s) that are not declared: {}",
            unknown.join(", ")
        )))
    }
}

fn unique_task<'a>(
    plan: &'a Plan,
    id: &str,
) -> Result<&'a aplan_core::load::LoadedTask, AuthoringError> {
    let mut matches = plan.tasks.iter().filter(|task| task.task.id == id);
    let Some(found) = matches.next() else {
        return Err(AuthoringError::Arguments(format!(
            "no task `{id}` in this plan"
        )));
    };
    if matches.next().is_some() {
        return Err(AuthoringError::Refused(format!(
            "task id `{id}` is ambiguous; repair the duplicate before mutating it"
        )));
    }
    Ok(found)
}

fn source(context: &PlanContext, path: &Path, id: &str) -> Result<String, AuthoringError> {
    context
        .source_text(path)
        .map(str::to_string)
        .ok_or_else(|| {
            AuthoringError::Refused(format!(
                "`{id}` changed while its preview was being prepared; reload and try again"
            ))
        })
}

fn id_strategy(context: &PlanContext) -> IdStrategy {
    context
        .report_config()
        .and_then(|config| config.id_strategy)
        .unwrap_or_default()
}

fn task_dir(context: &PlanContext) -> PathBuf {
    let conventional = context.identity().plan_root.join("tasks");
    if conventional.is_dir() {
        conventional
    } else {
        context.identity().plan_root.clone()
    }
}

fn successor_tags(tags: &[String]) -> Vec<String> {
    aplan_interop::emit::deduped(
        &tags
            .iter()
            .filter(|tag| !aplan_core::validate::RESERVED_TAGS.contains(&tag.as_str()))
            .cloned()
            .collect::<Vec<_>>(),
    )
}

fn successor_body(predecessor: &Task, id: &str) -> String {
    let mut body = String::new();
    let _ = writeln!(body, "## Why\n");
    let _ = writeln!(
        body,
        "This succeeds `{id}` (\"{}\"), whose traced outcome no longer holds.\n",
        predecessor.title
    );
    let _ = writeln!(
        body,
        "That task stays `done` and keeps its evidence: the work was finished, and what \
         changed afterwards is the ground under it. This task is what now needs doing.\n"
    );
    if !predecessor.traces.is_empty() {
        let _ = writeln!(
            body,
            "It traces the same section(s), which is what drifted:\n"
        );
        for trace in &predecessor.traces {
            let _ = writeln!(body, "- `{trace}`");
        }
        body.push('\n');
    }
    let _ = writeln!(
        body,
        "(Say what changed, and what is different once this is done.)\n"
    );
    let _ = writeln!(body, "## Acceptance criteria\n");
    let _ = writeln!(
        body,
        "- [ ] state what done means for this task, in this repository's terms"
    );
    body
}

fn rekeyed(values: &[String], old: &str, new: &str) -> Vec<String> {
    values
        .iter()
        .map(|value| if value == old { new } else { value }.to_string())
        .collect()
}

fn rekey_task_file(
    original: &str,
    task: &Task,
    old: &str,
    new: &str,
) -> Result<String, AuthoringError> {
    let rewrite = |source| AuthoringError::Rewrite {
        id: task.id.clone(),
        source,
    };
    let mut updated = if task.id == old {
        aplan_core::close::set_id(original, new).map_err(rewrite)?
    } else {
        original.to_string()
    };
    if task.blocked_by.iter().any(|id| id == old) {
        updated =
            aplan_core::close::rekey_blocked_by(&updated, &rekeyed(&task.blocked_by, old, new))
                .map_err(rewrite)?;
    }
    if task.prefer_after.iter().any(|id| id == old) {
        updated =
            aplan_core::close::rekey_prefer_after(&updated, &rekeyed(&task.prefer_after, old, new))
                .map_err(rewrite)?;
    }
    if task.ratified_by.iter().any(|id| id == old) {
        updated =
            aplan_core::close::set_ratified_by(&updated, &rekeyed(&task.ratified_by, old, new))
                .map_err(rewrite)?;
    }
    if task.contains.iter().any(|id| id == old) {
        updated = aplan_core::close::set_contains(&updated, &rekeyed(&task.contains, old, new))
            .map_err(rewrite)?;
    }
    Ok(updated)
}

fn rekey_plan_model(plan: &Plan, old: &str, new: &str, destination: &Path) -> Plan {
    let mut expected = aplan_core::mutation::clone_of(plan);
    for loaded in &mut expected.tasks {
        loaded.task.blocked_by = rekeyed(&loaded.task.blocked_by, old, new);
        loaded.task.prefer_after = rekeyed(&loaded.task.prefer_after, old, new);
        loaded.task.ratified_by = rekeyed(&loaded.task.ratified_by, old, new);
        loaded.task.contains = rekeyed(&loaded.task.contains, old, new);
        if loaded.task.id == old {
            loaded.task.id = new.to_string();
            loaded.path = destination.to_path_buf();
        }
    }
    expected
}

fn conventional_rekey_path(path: &Path, old: &str, new: &str) -> PathBuf {
    if path
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(&format!("{old}.md")))
    {
        path.with_file_name(format!("{new}.md"))
    } else {
        path.to_path_buf()
    }
}

fn is_within(path: &Path, boundary: &Path) -> bool {
    let resolved = |candidate: &Path| {
        candidate
            .canonicalize()
            .or_else(|_| std::path::absolute(candidate))
            .unwrap_or_else(|_| candidate.to_path_buf())
    };
    resolved(path).starts_with(resolved(boundary))
}

fn prose_occurrences(
    repository: &Path,
    old: &str,
    excluded: &BTreeSet<PathBuf>,
) -> Vec<ExternalReference> {
    let mut paths = BTreeSet::new();
    collect_text_files(repository, &mut paths);
    let mut found = Vec::new();
    for path in paths {
        if excluded.contains(&absolute(&path)) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        found.extend(text_occurrences(
            path.strip_prefix(repository).unwrap_or(&path),
            text,
            old,
        ));
    }
    found
}

fn text_occurrences(path: &Path, text: &str, old: &str) -> Vec<ExternalReference> {
    let mut found = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        for (column, _) in line.match_indices(old).filter(|(column, _)| {
            let before = line[..*column].chars().next_back();
            let tail = &line[*column + old.len()..];
            let after = tail.chars().next();
            let id_character = |character: char| {
                character.is_alphanumeric() || matches!(character, '-' | '_' | '.')
            };
            let moved_filename = tail
                .strip_prefix(".md")
                .is_some_and(|rest| !rest.chars().next().is_some_and(id_character));
            !before.is_some_and(id_character)
                && (!after.is_some_and(id_character) || moved_filename)
        }) {
            found.push(ExternalReference {
                path: path.to_path_buf(),
                line: line_index + 1,
                column: line[..column].chars().count() + 1,
            });
        }
    }
    found
}

fn collect_text_files(directory: &Path, out: &mut BTreeSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if name != ".git" && name != "target" {
                collect_text_files(&path, out);
            }
        } else if kind.is_file() {
            out.insert(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChangeDisposition, CommitOutcome, ContextInputs, Trust, commit_change_set};

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, text).expect("fixture");
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let tasks = dir.path().join("aplan/tasks");
        write(
            &tasks.join("A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/one.rs]\n---\n",
        );
        write(
            &tasks.join("A-2.md"),
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1]\npaths: [src/two.rs]\n---\n",
        );
        write(
            &tasks.join("M-v1.md"),
            "---\nid: M-v1\ntitle: first cut\nphase: 1\nstatus: todo\nblocked_by: [A-1, A-2]\ntags: [milestone, v1]\n---\n",
        );
        dir
    }

    fn context(dir: &Path, mut options: aplan_core::Options) -> PlanContext {
        options.require_effort = true;
        PlanContext::new(
            aplan_core::load_plan_identified(&dir.join("aplan")),
            ContextInputs {
                repository_root: dir.to_path_buf(),
                configuration_file: None,
                trust: Trust::RepositoryRead,
                options,
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(aplan_config::Config::default())
    }

    fn add_request() -> AddRequest {
        AddRequest {
            title: "third".to_string(),
            id: Some("A-3".to_string()),
            phase: Some(1),
            cut: None,
            effort_days: Some(1.0),
            owner: None,
            status: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            members: Vec::new(),
            tags: Vec::new(),
            paths: vec!["src/three.rs".to_string()],
            traces: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            evidence: Vec::new(),
            body: None,
        }
    }

    fn import_request(format: &str, contents: &str) -> ImportRequest {
        ImportRequest {
            source_name: "captured.source".to_string(),
            contents: contents.to_string(),
            format: format.to_string(),
            phase: 1,
            implemented_as: ImplementedAs::Todo,
            cut: None,
            id_prefix: None,
            tags: Vec::new(),
        }
    }

    fn message(result: Result<AuthoringPreview, AuthoringError>) -> String {
        match result {
            Ok(_) => panic!("request unexpectedly produced a preview"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn single_add_derives_cut_phase_preserves_repeated_fields_and_commits_exact_preview() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        let request = AddRequest {
            title: "third".to_string(),
            id: None,
            phase: None,
            cut: Some("v1".to_string()),
            effort_days: Some(2.0),
            owner: Some("owner".to_string()),
            status: None,
            blocked_by: vec!["A-1".to_string(), "A-1".to_string()],
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            members: Vec::new(),
            tags: vec!["kind-build".to_string(), "kind-build".to_string()],
            paths: vec!["src/three.rs".to_string(), "src/three.rs".to_string()],
            traces: vec!["HLD.md#three".to_string()],
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            evidence: Vec::new(),
            body: Some("## Why\n\nReal body.\n".to_string()),
        };
        let preview = preview_add(&context, &request).expect("preview");
        assert_eq!(preview.created(), ["A-3"]);
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        assert_eq!(preview.change_set().files().len(), 2);
        let created = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.before().is_none())
            .expect("created task");
        assert!(created.after().contains("phase: 1"));
        assert_eq!(created.after().matches("src/three.rs").count(), 1);
        assert!(created.after().contains("Real body."));
        let exact = created.after().to_string();
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("aplan/tasks/A-3.md")).expect("created"),
            exact
        );
        let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-v1.md")).expect("cut");
        assert!(cut.contains("A-3"));
    }

    /// Executable parity audit for `add.accepts_creation_time_model_fields`.
    /// Adding a field to `AddRequest` breaks this constructor; adding one to
    /// the manifest breaks `manifest::ENTRY_FIELDS`. The two previews must then
    /// still produce the same task for every shared creation-time value.
    #[test]
    fn creation_field_parity_audit_drives_single_and_manifest_previews() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        let request = AddRequest {
            title: "release container".to_string(),
            id: Some("M-new".to_string()),
            phase: Some(2),
            cut: None,
            effort_days: None,
            owner: Some("release team".to_string()),
            status: Some(Status::Blocked),
            blocked_by: vec!["M-v1".to_string()],
            prefer_after: vec!["A-1".to_string()],
            ratified_by: vec!["A-2".to_string()],
            contains: vec!["M-v1".to_string()],
            members: Vec::new(),
            tags: vec!["milestone".to_string()],
            paths: Vec::new(),
            traces: Vec::new(),
            revisit_at: Some("2030-01-02T03:04:05Z".to_string()),
            version_impact: Some(VersionImpact::Feature),
            version_impact_reason: None,
            evidence: Vec::new(),
            body: Some("## Why\n\nOne shared contract.\n".to_string()),
        };
        let single = preview_add(&context, &request).expect("single preview");
        let manifest = preview_manifest(
            &context,
            &ManifestRequest {
                source_name: "plan.yml".to_string(),
                contents: "- title: release container\n  id: M-new\n  phase: 2\n  owner: release team\n  status: blocked\n  blocked_by: [M-v1]\n  prefer_after: [A-1]\n  ratified_by: [A-2]\n  contains: [M-v1]\n  tags: [milestone]\n  revisit_at: 2030-01-02T03:04:05Z\n  version_impact: feature\n  body: |\n    ## Why\n\n    One shared contract.\n"
                    .to_string(),
            },
        )
        .expect("manifest preview");
        let task_of = |preview: &AuthoringPreview| {
            let file = preview
                .change_set()
                .files()
                .iter()
                .find(|file| file.before().is_none())
                .expect("creation");
            aplan_core::mutation::task_of(file.after()).expect("created task")
        };
        assert_eq!(task_of(&single), task_of(&manifest));
    }

    #[test]
    fn typed_manifest_resolves_done_ledger_evidence_and_refuses_the_complete_bad_batch() {
        let dir = fixture();
        write(&dir.path().join("proof.txt"), "historical delivery\n");
        write(
            &dir.path().join("DOC.md"),
            "# Contract\n\n## Ledger\n\nHistorical record.\n",
        );
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("git command");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ]);
        let options = aplan_core::Options {
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            traces_repository: Some(dir.path().to_path_buf()),
            ..aplan_core::Options::default()
        };
        let context = context(dir.path(), options);
        let preview = preview_manifest(
            &context,
            &ManifestRequest {
                source_name: "ledger.yml".to_string(),
                contents: "- title: reconstructed delivery\n  id: HIST-1\n  phase: 1\n  status: done\n  effort: 1\n  paths: [proof.txt]\n  traces: [DOC.md#ledger]\n  evidence:\n  - kind: file\n    ref: proof.txt\n"
                    .to_string(),
            },
        )
        .expect("typed ledger preview");
        let created = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.before().is_none())
            .expect("ledger creation")
            .after();
        assert!(created.contains("status: done"), "{created}");
        assert!(created.contains("trace_fingerprints:"), "{created}");
        assert!(created.contains("evidence_fingerprints:"), "{created}");

        preview_manifest(
            &self::context(dir.path(), aplan_core::Options::default()),
            &ManifestRequest {
                source_name: "ledger-without-drift.yml".to_string(),
                contents: "- title: unstamped history\n  id: HIST-0\n  phase: 1\n  status: done\n  effort: 1\n  paths: [proof.txt]\n  evidence:\n  - kind: file\n    ref: proof.txt\n"
                    .to_string(),
            },
        )
        .expect("ledger preview with drift baselines disabled");

        let error = preview_manifest(
            &context,
            &ManifestRequest {
                source_name: "bad-ledger.yml".to_string(),
                contents: "- title: valid sibling\n  id: A-3\n  effort: 1\n  paths: [src/three.rs]\n- title: broken history\n  id: HIST-2\n  status: done\n  effort: 1\n  paths: [missing.txt]\n  evidence:\n  - kind: file\n    ref: missing.txt\n"
                    .to_string(),
            },
        )
        .expect_err("unresolved evidence refuses the complete typed batch");
        assert!(
            error.to_string().contains("ledger entry `HIST-2`")
                && error.to_string().contains("APLAN015"),
            "{error}"
        );
        assert!(!dir.path().join("aplan/tasks/A-3.md").exists());
        assert!(!dir.path().join("aplan/tasks/HIST-2.md").exists());
    }

    #[test]
    fn new_cut_preview_names_both_membership_directions_and_stale_input_publishes_nothing() {
        // Proves `add.creates_cut_around_existing_members` at the typed preview
        // and guarded publication boundary.
        let dir = fixture();
        let initial = context(dir.path(), aplan_core::Options::default());
        let request = AddRequest {
            title: "second cut".to_string(),
            id: Some("M-v2".to_string()),
            phase: Some(2),
            cut: None,
            effort_days: None,
            owner: None,
            status: None,
            blocked_by: vec!["M-v1".to_string()],
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            members: vec!["A-2".to_string()],
            tags: vec!["milestone".to_string(), "v2".to_string()],
            paths: Vec::new(),
            traces: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            evidence: Vec::new(),
            body: Some("## Why\n\nA distinct delivery gate.\n".to_string()),
        };
        let preview = preview_add(&initial, &request).expect("new-cut preview");
        assert_eq!(preview.change_set().files().len(), 2);
        assert!(preview.change_set().effects().iter().any(|effect| matches!(
            effect,
            SemanticEffect::CutMembershipAdded { cut, tasks }
                if cut == "M-v2" && tasks == &["A-2"]
        )));
        assert!(preview.change_set().effects().iter().any(|effect| matches!(
            effect,
            SemanticEffect::CutMembershipRemoved { cut, tasks }
                if cut == "M-v1" && tasks == &["A-2"]
        )));

        let source = dir.path().join("aplan/tasks/M-v1.md");
        let mut externally_changed = std::fs::read_to_string(&source).expect("source");
        externally_changed.push_str("\n<!-- concurrent review -->\n");
        write(&source, &externally_changed);
        assert!(matches!(
            commit_change_set(&initial, preview.into_change_set()),
            CommitOutcome::Stale { .. }
        ));
        assert!(!dir.path().join("aplan/tasks/M-v2.md").exists());
        assert_eq!(
            std::fs::read_to_string(&source).expect("source after stale"),
            externally_changed
        );

        let refreshed = context(dir.path(), aplan_core::Options::default());
        let preview = preview_add(&refreshed, &request).expect("refreshed preview");
        assert!(matches!(
            commit_change_set(&refreshed, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        assert!(
            std::fs::read_to_string(&source)
                .expect("source after commit")
                .contains("blocked_by: [A-1]")
        );
        assert!(
            std::fs::read_to_string(dir.path().join("aplan/tasks/M-v2.md"))
                .expect("new cut")
                .contains("- A-2")
        );
    }

    #[test]
    fn authoring_validation_refuses_each_malformed_scalar_and_relation() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());

        let mut request = add_request();
        request.title = " \t".to_string();
        assert!(message(preview_add(&context, &request)).contains("needs a title"));

        let mut request = add_request();
        request.effort_days = Some(f64::NAN);
        assert!(message(preview_add(&context, &request)).contains("finite"));
        request.effort_days = Some(-1.0);
        assert!(message(preview_add(&context, &request)).contains("non-negative"));

        let mut request = add_request();
        request.tags = vec![" ".to_string()];
        assert!(message(preview_add(&context, &request)).contains("tag may not be blank"));

        let mut request = add_request();
        request.owner = Some("\t".to_string());
        assert!(message(preview_add(&context, &request)).contains("owner may not be blank"));

        let mut request = add_request();
        request.id = Some("../escape".to_string());
        assert!(message(preview_add(&context, &request)).contains("task id and file stem"));
        request.id = Some("A-1".to_string());
        assert!(message(preview_add(&context, &request)).contains("already exists"));
        request.id = Some("v1".to_string());
        assert!(message(preview_add(&context, &request)).contains("make that label unusable"));

        let mut request = add_request();
        request.blocked_by = vec!["missing".to_string()];
        assert!(message(preview_add(&context, &request)).contains("not declared: missing"));

        let mut request = add_request();
        request.prefer_after = vec!["missing".to_string()];
        assert!(
            message(preview_add(&context, &request))
                .contains("`prefer_after` names task(s) that are not declared: missing")
        );
    }

    #[test]
    fn creation_lifecycle_and_new_cut_neighbours_are_refused_by_name() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());

        let mut request = add_request();
        request.status = Some(Status::Dropped);
        assert!(message(preview_add(&context, &request)).contains("guarded drop operation"));

        let mut request = add_request();
        request.status = Some(Status::Done);
        assert!(message(preview_add(&context, &request)).contains("needs evidence"));

        let mut request = add_request();
        request.evidence = vec![Evidence {
            kind: aplan_core::EvidenceKind::Note,
            reference: "not completion proof at creation".to_string(),
            captured: Some("2026-10-30".to_string()),
        }];
        assert!(message(preview_add(&context, &request)).contains("live work"));

        let mut request = add_request();
        request.revisit_at = Some("2030-01-02T03:04:05Z".to_string());
        assert!(message(preview_add(&context, &request)).contains("initially parked"));

        let mut request = add_request();
        request.status = Some(Status::Blocked);
        request.revisit_at = Some("next Thursday".to_string());
        assert!(message(preview_add(&context, &request)).contains("RFC 3339"));

        let mut request = add_request();
        request.version_impact_reason = Some("No impact selected".to_string());
        assert!(message(preview_add(&context, &request)).contains("requires the version impact"));

        let mut request = add_request();
        request.contains = vec!["\u{200b}".to_string()];
        assert!(message(preview_add(&context, &request)).contains("visually empty"));

        let mut request = add_request();
        request.members = vec!["A-1".to_string()];
        request.cut = Some("M-v1".to_string());
        assert!(message(preview_add(&context, &request)).contains("cannot also join `cut`"));

        let mut request = add_request();
        request.members = vec!["A-1".to_string()];
        request.id = None;
        assert!(message(preview_add(&context, &request)).contains("supply both `id` and `phase`"));

        let mut request = add_request();
        request.members = vec!["A-1".to_string()];
        assert!(message(preview_add(&context, &request)).contains("carry the `milestone` tag"));

        let mut request = add_request();
        request.members = vec!["A-1".to_string()];
        request.tags = vec!["milestone".to_string()];
        assert!(message(preview_add(&context, &request)).contains("APLAN021"));

        let mut request = add_request();
        request.members = vec!["A-2".to_string()];
        request.tags = vec!["milestone".to_string()];
        request.effort_days = None;
        request.paths.clear();
        request.blocked_by = vec!["A-1".to_string()];
        assert!(
            message(preview_add(&context, &request))
                .contains("reserve `blocked_by` for milestone sequencing")
        );
    }

    #[test]
    fn bodies_successors_and_incomplete_plans_are_refused_before_publication() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());

        let mut request = add_request();
        request.body = Some(" \n\t".to_string());
        assert!(message(preview_add(&context, &request)).contains("edited body is blank"));

        let mut request = add_request();
        request.cut = Some("missing".to_string());
        assert!(message(preview_add(&context, &request)).contains("no cut"));
        request.cut = Some("A-1".to_string());
        assert!(message(preview_add(&context, &request)).contains("not tagged `milestone`"));

        assert!(
            message(preview_supersede(
                &context,
                &SupersedeRequest {
                    predecessor: "missing".to_string(),
                    title: "successor".to_string(),
                    cut: None,
                    effort_days: Some(1.0),
                    owner: None,
                    tags: Vec::new(),
                    paths: None,
                    traces: None,
                    body: None,
                },
            ))
            .contains("no task `missing`")
        );

        for request in [
            SupersedeRequest {
                predecessor: "A-1".to_string(),
                title: "successor".to_string(),
                cut: None,
                effort_days: Some(1.0),
                owner: None,
                tags: Vec::new(),
                paths: None,
                traces: None,
                body: Some("\u{200b}".to_string()),
            },
            SupersedeRequest {
                predecessor: "A-1".to_string(),
                title: "successor".to_string(),
                cut: None,
                effort_days: Some(1.0),
                owner: None,
                tags: Vec::new(),
                paths: Some(vec!["\u{200b}".to_string()]),
                traces: Some(vec!["\u{200b}".to_string()]),
                body: None,
            },
        ] {
            assert!(message(preview_supersede(&context, &request)).contains("blank"));
        }

        write(
            &dir.path().join("aplan/tasks/broken.md"),
            "---\nid: [not valid\n---\n",
        );
        let dirty = self::context(dir.path(), aplan_core::Options::default());
        assert!(message(preview_add(&dirty, &add_request())).contains("does not load cleanly"));

        let duplicate = fixture();
        write(
            &duplicate.path().join("aplan/tasks/duplicate.md"),
            "---\nid: A-1\ntitle: duplicate\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/duplicate.rs]\n---\n",
        );
        let duplicate = self::context(duplicate.path(), aplan_core::Options::default());
        assert!(matches!(
            unique_task(duplicate.plan(), "A-1"),
            Err(AuthoringError::Refused(error)) if error.contains("ambiguous")
        ));
    }

    #[test]
    fn import_validation_covers_dispatch_provenance_prefix_and_cardinality() {
        let dir = fixture();
        let context = self::context(dir.path(), aplan_core::Options::default());

        let request = import_request("unknown", "anything");
        assert!(message(preview_import(&context, &request)).contains("unknown import format"));

        let mut request =
            import_request("kep", "title: Work\nkep-number: 7\nstatus: implemented\n");
        request.implemented_as = ImplementedAs::Done {
            as_of: "yesterday".to_string(),
        };
        assert!(message(preview_import(&context, &request)).contains("YYYY-MM-DD"));

        let mut request = import_request("roadmap-json", r#"{"phases":[]}"#);
        request.id_prefix = Some("R".to_string());
        assert!(message(preview_import(&context, &request)).contains("applies only"));

        let mut request = import_request("checklist", "## Work\n\n- [ ] Do it\n");
        request.id_prefix = Some(" \t".to_string());
        assert!(message(preview_import(&context, &request)).contains("non-blank"));
        request.id_prefix = Some("bad/prefix".to_string());
        assert!(message(preview_import(&context, &request)).contains("alphanumeric"));

        let mut request = import_request("checklist", "## Work\n\n- [ ] Do it\n");
        request.tags = vec![String::new()];
        assert!(message(preview_import(&context, &request)).contains("tag may not be blank"));

        let request = import_request("spec-kit", "## Phase 1\n\n- [ ] T001 Do it\n");
        assert_eq!(
            preview_import(&context, &request)
                .expect("source-carried id needs no prefix")
                .created(),
            ["T001"]
        );

        let malformed = import_request("kep", "not: a kep\n");
        assert!(message(preview_import(&context, &malformed)).contains("captured.source"));

        let invalid_dispatch = import_request("future", "anything");
        assert!(matches!(
            parse_import(&invalid_dispatch, &aplan_interop::Options::default()),
            Err(AuthoringError::Interop(error)) if error.contains("no application parser")
        ));

        let task = new_task(NewTask {
            id: "I-1".to_string(),
            title: "imported".to_string(),
            phase: 1,
            owner: None,
            effort_days: Some(1.0),
            status: Status::Todo,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: vec!["src/imported.rs".to_string()],
            traces: Vec::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        });
        let mut imported = aplan_interop::Import {
            tasks: vec![task.clone()],
            report: aplan_interop::Report::new(),
            bodies: Vec::new(),
        };
        assert!(matches!(
            validate_imported_tasks(context.plan(), "test", &imported),
            Err(AuthoringError::Interop(error)) if error.contains("1 task(s) but 0 body")
        ));
        imported.bodies.push("body".to_string());
        imported.tasks.push(task);
        imported.bodies.push("body".to_string());
        assert!(matches!(
            validate_imported_tasks(context.plan(), "test", &imported),
            Err(AuthoringError::Arguments(error)) if error.contains("more than once")
        ));
        imported.tasks[0].title = " ".to_string();
        assert!(matches!(
            validate_imported_tasks(context.plan(), "test", &imported),
            Err(AuthoringError::Arguments(error)) if error.contains("needs a title")
        ));
        imported.tasks.truncate(1);
        imported.bodies.truncate(1);
        imported.tasks[0].title = "imported".to_string();
        imported.tasks[0].id = "A-1".to_string();
        assert!(matches!(
            validate_imported_tasks(context.plan(), "test", &imported),
            Err(AuthoringError::Refused(error)) if error.contains("already exists")
        ));
    }

    #[test]
    fn manifest_forward_references_and_new_cut_publish_as_one_batch() {
        let dir = fixture();
        let first_context = context(dir.path(), aplan_core::Options::default());
        let preview = preview_manifest(
            &first_context,
            &ManifestRequest {
                source_name: "plan.yml".to_string(),
                contents: concat!(
                    "- id: B-2\n",
                    "  title: forward member\n",
                    "  phase: 2\n",
                    "  effort: 1\n",
                    "  cut: M-v2\n",
                    "  blocked_by: [B-3]\n",
                    "  paths: [src/b.rs]\n",
                    "- id: B-3\n",
                    "  title: later dependency\n",
                    "  phase: 2\n",
                    "  effort: 1\n",
                    "  cut: M-v2\n",
                    "  paths: [src/c.rs]\n",
                    "- id: M-v2\n",
                    "  title: second cut\n",
                    "  phase: 2\n",
                    "  tags: [milestone, v2]\n",
                    "  blocked_by: [B-2]\n",
                )
                .to_string(),
            },
        )
        .expect("manifest preview");
        assert_eq!(preview.created(), ["B-2", "B-3", "M-v2"]);
        assert_eq!(preview.change_set().files().len(), 3);
        let cut = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.path().ends_with("M-v2.md"))
            .expect("new cut");
        assert!(cut.after().contains("- B-2"));
        assert!(cut.after().contains("- B-3"));
        assert!(matches!(
            commit_change_set(&first_context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        assert!(dir.path().join("aplan/tasks/B-3.md").is_file());

        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        let error = message(preview_manifest(
            &context,
            &ManifestRequest {
                source_name: "shadow.yml".to_string(),
                contents: concat!(
                    "- id: v2\n",
                    "  title: looks ordinary before the later cut\n",
                    "  phase: 2\n",
                    "  effort: 1\n",
                    "- id: M-v2\n",
                    "  title: later cut\n",
                    "  phase: 2\n",
                    "  tags: [milestone, v2]\n",
                    "  blocked_by: [v2]\n",
                )
                .to_string(),
            },
        ));
        assert!(error.contains("make that label unusable"), "{error}");
    }

    #[test]
    fn manifest_reverse_edges_preview_existing_and_created_targets_together() {
        let dir = fixture();
        let task = dir.path().join("aplan/tasks/A-1.md");
        write(
            &dir.path().join("aplan/tasks/A-0.md"),
            "---\nid: A-0\ntitle: completed\nphase: 1\nstatus: done\nevidence:\n- kind: note\n  ref: reviewed\n  captured: 2026-10-30\n---\n",
        );
        let source = std::fs::read_to_string(&task)
            .expect("task")
            .replace("paths:", "blocked_by: [ 'A-0'] # source\npaths:");
        std::fs::write(&task, &source).expect("decorated task");
        let context = context(dir.path(), aplan_core::Options::default());
        let request = ManifestRequest {
            source_name: "reverse.yml".to_string(),
            contents: concat!(
                "- id: B-1\n",
                "  title: new blocker\n",
                "  phase: 2\n",
                "  effort: 1\n",
                "  paths: [src/b.rs]\n",
                "  blocks: [A-1, B-2]\n",
                "- id: B-2\n",
                "  title: new target\n",
                "  phase: 2\n",
                "  effort: 1\n",
                "  paths: [src/c.rs]\n",
            )
            .to_string(),
        };
        let preview = preview_manifest(&context, &request).expect("reverse edge preview");
        assert_eq!(preview.created(), ["B-1", "B-2"]);
        assert_eq!(preview.change_set().files().len(), 3);
        let existing = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.path().ends_with("A-1.md"))
            .expect("existing target rewrite");
        assert!(
            existing
                .after()
                .contains("blocked_by: [ 'A-0', 'B-1'] # source"),
            "{}",
            existing.after()
        );
        let created = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.path().ends_with("B-2.md"))
            .expect("created target");
        assert!(created.after().contains("blocked_by:\n- B-1"));
        assert_eq!(
            preview
                .change_set()
                .effects()
                .iter()
                .filter(|effect| matches!(effect, SemanticEffect::DependencyAdded { .. }))
                .count(),
            2
        );
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));

        let context = self::context(dir.path(), aplan_core::Options::default());
        let error = message(preview_manifest(
            &context,
            &ManifestRequest {
                source_name: "duplicate.yml".to_string(),
                contents: "- id: B-3\n  title: duplicate\n  phase: 2\n  effort: 1\n  paths: [src/d.rs]\n  blocks: [A-1, A-1]\n"
                    .to_string(),
            },
        ));
        assert!(error.contains("more than once"), "{error}");
        assert!(!dir.path().join("aplan/tasks/B-3.md").exists());
    }

    #[test]
    fn a_manifest_cut_may_report_its_own_id_as_its_label() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        let preview = preview_manifest(
            &context,
            &ManifestRequest {
                source_name: "own-label.yml".to_string(),
                contents: concat!(
                    "- id: A-3\n",
                    "  title: member\n",
                    "  phase: 2\n",
                    "  effort: 1\n",
                    "- id: M-own\n",
                    "  title: self-labelled cut\n",
                    "  phase: 2\n",
                    "  tags: [milestone, M-own]\n",
                    "  blocked_by: [A-3]\n",
                )
                .to_string(),
            },
        )
        .expect("one cut identity has two equivalent spellings");
        assert_eq!(preview.created(), ["A-3", "M-own"]);
    }

    #[test]
    fn manifest_validation_is_entry_scoped_and_existing_cut_edits_are_previewed() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        let preview = preview_manifest(
            &context,
            &ManifestRequest {
                source_name: "captured.yml".to_string(),
                contents: concat!(
                    "- title: minted member\n",
                    "  phase: 1\n",
                    "  effort: 1\n",
                    "  cut: M-v1\n",
                    "  paths: [src/minted.rs]\n",
                )
                .to_string(),
            },
        )
        .expect("existing cut preview");
        assert_eq!(preview.created(), ["A-3"]);
        assert!(
            preview
                .change_set()
                .files()
                .iter()
                .any(|file| file.path().ends_with("M-v1.md"))
        );

        for (source, expected) in [
            (
                "- id: B-1\n  title: ' '\n  phase: 2\n  effort: 1\n",
                "manifest entry 1 (` `): a task needs a title",
            ),
            (
                "- id: B-1\n  title: work\n  phase: 2\n  effort: -1\n",
                "manifest entry 1 (`work`): effort must be",
            ),
            (
                "- id: B-1\n  title: work\n  phase: 2\n  effort: 1\n  tags: [' ']\n",
                "manifest entry 1 (`work`): a tag may not be blank",
            ),
            (
                "- id: B-1\n  title: work\n  phase: 2\n  effort: 1\n  owner: ' '\n",
                "manifest entry 1 (`work`): owner may not be blank",
            ),
            (
                "- id: B-1\n  title: work\n  phase: 2\n  effort: 1\n  cut: missing\n",
                "manifest entry 1 (`work`): no cut",
            ),
            (
                "- id: B-1\n  title: work\n  phase: 2\n  effort: 1\n  cut: A-1\n",
                "manifest entry 1 (`work`): `A-1` is not tagged `milestone`",
            ),
            (
                "- id: B-1\n  title: work\n  phase: 2\n  effort: 1\n  body: ' '\n",
                "manifest entry 1 (`work`): body is blank",
            ),
        ] {
            let error = message(preview_manifest(
                &context,
                &ManifestRequest {
                    source_name: "captured.yml".to_string(),
                    contents: source.to_string(),
                },
            ));
            assert!(
                error.contains(expected),
                "expected {expected:?} in {error:?}"
            );
        }

        let not_a_cut = concat!(
            "- id: B-1\n",
            "  title: member\n",
            "  phase: 2\n",
            "  effort: 1\n",
            "  cut: B-cut\n",
            "- id: B-cut\n",
            "  title: ordinary task\n",
            "  phase: 2\n",
            "  effort: 1\n",
        );
        let error = message(preview_manifest(
            &context,
            &ManifestRequest {
                source_name: "captured.yml".to_string(),
                contents: not_a_cut.to_string(),
            },
        ));
        assert!(error.contains("manifest entry 1"));
        assert!(error.contains("no cut"));
    }

    #[derive(Clone, Copy)]
    struct ImportCase {
        format: &'static str,
        contents: &'static str,
        prefix: Option<&'static str>,
    }

    fn import_cases() -> [ImportCase; 5] {
        [
            ImportCase {
                format: "kep",
                contents: "title: Work\nkep-number: 7\nstatus: provisional\n",
                prefix: None,
            },
            ImportCase {
                format: "checklist",
                contents: "## Work\n\n- [ ] Do it\n",
                prefix: Some("C"),
            },
            ImportCase {
                format: "roadmap-json",
                contents: r#"{"phases":[{"id":"source-1","title":"Source phase"}]}"#,
                prefix: None,
            },
            ImportCase {
                format: "spec-kit",
                contents: "## Phase 1\n\n- [ ] T001 Do it\n",
                prefix: Some("S"),
            },
            ImportCase {
                format: "openspec",
                contents: "## 1. Work\n\n- [ ] 1.1 Do it\n",
                prefix: Some("O"),
            },
        ]
    }

    fn import_request_for(case: ImportCase) -> ImportRequest {
        ImportRequest {
            source_name: format!("{}.source", case.format),
            contents: case.contents.to_string(),
            format: case.format.to_string(),
            phase: 1,
            implemented_as: ImplementedAs::Todo,
            cut: Some("M-v1".to_string()),
            id_prefix: case.prefix.map(str::to_string),
            tags: vec!["kind-imported".to_string()],
        }
    }

    #[test]
    fn every_registered_import_shape_drives_a_reviewable_atomic_batch() {
        let cases = import_cases();
        assert_eq!(
            cases.iter().map(|case| case.format).collect::<Vec<_>>(),
            aplan_interop::flavor_names(),
            "a new registered importer must gain a real authoring case"
        );
        for case in cases {
            let dir = fixture();
            let ordinary_context = context(dir.path(), aplan_core::Options::default());
            let request = import_request_for(case);
            let preview = preview_import(&ordinary_context, &request)
                .unwrap_or_else(|error| panic!("{}: {error}", case.format));
            assert!(
                !preview.created().is_empty(),
                "{} drove no task",
                case.format
            );
            assert_eq!(
                preview
                    .change_set()
                    .files()
                    .iter()
                    .filter(|file| file.before().is_none())
                    .count(),
                preview.created().len(),
                "{} did not preview every imported task",
                case.format
            );
            assert_eq!(
                preview
                    .change_set()
                    .effects()
                    .iter()
                    .filter(|effect| matches!(effect, SemanticEffect::ImportNotice { .. }))
                    .count(),
                preview.notices().len(),
                "{} omitted a loss-report row from semantic review",
                case.format
            );
            assert!(
                preview
                    .change_set()
                    .effects()
                    .iter()
                    .any(|effect| matches!(effect, SemanticEffect::CutMembershipAdded { .. })),
                "{} omitted cut review",
                case.format
            );
            assert!(
                preview.change_set().files().len() > preview.created().len(),
                "{} did not include its cut edit in the batch",
                case.format
            );
        }
    }

    #[test]
    fn every_registered_import_shape_preserves_the_cut_label_namespace() {
        for case in import_cases() {
            let request = import_request_for(case);
            let baseline = fixture();
            let baseline_context = context(baseline.path(), aplan_core::Options::default());
            let preview = preview_import(&baseline_context, &request)
                .unwrap_or_else(|error| panic!("{} baseline: {error}", case.format));
            let colliding_id = preview.created()[0].clone();
            let collision = fixture();
            let cut = collision.path().join("aplan/tasks/M-v1.md");
            let text = std::fs::read_to_string(&cut).expect("cut");
            write(
                &cut,
                &text.replace(
                    "tags: [milestone, v1]",
                    &format!("tags: [milestone, {colliding_id}]"),
                ),
            );
            let collision_context = context(collision.path(), aplan_core::Options::default());
            let error = message(preview_import(&collision_context, &request));
            assert!(
                error.contains("make that label unusable"),
                "{} bypassed the cut-label boundary: {error}",
                case.format
            );
        }
    }

    #[test]
    fn supersede_previews_inheritance_restamp_and_placement_together() {
        let dir = fixture();
        write(
            &dir.path().join("DESIGN.md"),
            "# Design\n\n## Contract\n\nCurrent.\n",
        );
        let predecessor = dir.path().join("aplan/tasks/A-1.md");
        write(
            &predecessor,
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: done\neffort_days: 1\nevidence:\n- kind: note\n  ref: reviewed\n  captured: 2026-10-20\npaths: [src/one.rs]\ntraces: [DESIGN.md#contract]\ntags: [kind-build, cross-cutting]\n---\n",
        );
        let options = aplan_core::Options {
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..aplan_core::Options::default()
        };
        let context = context(dir.path(), options);
        let preview = preview_supersede(
            &context,
            &SupersedeRequest {
                predecessor: "A-1".to_string(),
                title: "repair first".to_string(),
                cut: Some("M-v1".to_string()),
                effort_days: Some(1.0),
                owner: None,
                tags: Vec::new(),
                paths: None,
                traces: None,
                body: None,
            },
        )
        .expect("supersede preview");
        assert_eq!(preview.created(), ["A-3"]);
        assert_eq!(preview.change_set().files().len(), 3);
        let successor = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.path().ends_with("A-3.md"))
            .expect("successor");
        assert!(successor.after().contains("kind-build"));
        assert!(!successor.after().contains("cross-cutting"));
        assert!(
            successor
                .after()
                .contains("It traces the same section(s), which is what drifted:")
        );
        assert!(preview.change_set().effects().iter().any(|effect| {
            matches!(
                effect,
                SemanticEffect::TraceFingerprintsRestamped { count: 1, .. }
            )
        }));
    }

    #[test]
    fn supersede_refuses_non_outcomes_and_supports_an_unplaced_explicit_successor() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        let request = |predecessor: &str| SupersedeRequest {
            predecessor: predecessor.to_string(),
            title: "successor".to_string(),
            cut: None,
            effort_days: Some(1.0),
            owner: Some("new owner".to_string()),
            tags: vec!["kind-explicit".to_string()],
            paths: None,
            traces: None,
            body: None,
        };
        assert!(message(preview_supersede(&context, &request("M-v1"))).contains("milestone"));
        assert!(
            message(preview_supersede(&context, &request("A-1"))).contains("only a done outcome")
        );

        write(
            &dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: done\neffort_days: 1\nevidence:\n- kind: note\n  ref: reviewed\n  captured: 2026-10-20\npaths: [src/one.rs]\n---\n",
        );
        let context = self::context(dir.path(), aplan_core::Options::default());
        let preview = preview_supersede(&context, &request("A-1")).expect("unplaced successor");
        assert_eq!(preview.created(), ["A-3"]);
        assert_eq!(preview.change_set().files().len(), 2);
        let successor = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.path().ends_with("A-3.md"))
            .expect("successor");
        assert!(successor.after().contains("owner: new owner"));
        assert!(successor.after().contains("kind-explicit"));
        assert!(
            !preview
                .change_set()
                .effects()
                .iter()
                .any(|effect| { matches!(effect, SemanticEffect::CutMembershipAdded { .. }) })
        );
    }

    #[test]
    fn rekey_relocates_structured_references_and_only_reports_prose() {
        let dir = fixture();
        write(
            &dir.path().join("README.md"),
            "A-1 remains prose and [links](aplan/tasks/A-1.md).\n",
        );
        let external = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("external tempdir");
        let external_doc = external.path().join("PLAN.md");
        write(&external_doc, "External A-1 reference.\n");
        let context = context(dir.path(), aplan_core::Options::default()).with_documents(
            crate::DocumentInputs {
                state: crate::DocumentInputState::Configured,
                documents: vec![
                    crate::DocumentInput {
                        label: "internal plan".to_string(),
                        path: dir.path().join("README.md"),
                        content: Ok("A-1 is already found by the repository scan.\n".to_string()),
                        membership: None,
                    },
                    crate::DocumentInput {
                        label: "external plan".to_string(),
                        path: external_doc.clone(),
                        content: Ok("External A-1 reference.\n".to_string()),
                        membership: None,
                    },
                ],
            },
        );
        let preview = preview_rekey(
            &context,
            &RekeyRequest {
                old: "A-1".to_string(),
                new: "A-9".to_string(),
                allow_terminal: false,
            },
        )
        .expect("rekey preview");
        assert_eq!(preview.external_references().len(), 3);
        assert_eq!(
            preview
                .external_references()
                .iter()
                .filter(|occurrence| occurrence.path == Path::new("README.md"))
                .count(),
            2
        );
        assert!(
            preview
                .external_references()
                .iter()
                .any(|occurrence| occurrence.path == external_doc)
        );
        let relocation = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.source_path().is_some())
            .expect("relocation");
        assert!(relocation.source_path().unwrap().ends_with("A-1.md"));
        assert!(relocation.path().ends_with("A-9.md"));
        assert!(preview.change_set().files().iter().any(|file| {
            file.path().ends_with("A-2.md") && file.after().contains("blocked_by: [A-9]")
        }));
        assert!(preview.change_set().effects().iter().any(|effect| matches!(
            effect,
            SemanticEffect::TaskReferenceRekeyed { task, old, new }
                if task == "A-2" && old == "A-1" && new == "A-9"
        )));
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        assert!(!dir.path().join("aplan/tasks/A-1.md").exists());
        assert!(dir.path().join("aplan/tasks/A-9.md").exists());
        assert!(
            std::fs::read_to_string(dir.path().join("README.md"))
                .expect("prose")
                .contains("A-1")
        );
    }

    #[test]
    fn rekey_updates_in_repository_cut_policy_and_refuses_external_policy() {
        let dir = fixture();
        let config_path = dir.path().join(".aplan.yml");
        let config_text = "policy:\n  - when:\n      cut: [M-v1] # keep\n    evidence:\n      require_min_strength: [test]\n";
        write(&config_path, config_text);
        let config = aplan_config::parse_config(config_text, &config_path).expect("config");
        let context = PlanContext::new(
            aplan_core::load_plan_identified(&dir.path().join("aplan")),
            ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(config_path.clone()),
                trust: Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_captured_report_config(config.clone())
        .expect("capture policy config");
        let request = RekeyRequest {
            old: "M-v1".to_string(),
            new: "M-v9".to_string(),
            allow_terminal: false,
        };
        let preview = preview_rekey(&context, &request).expect("policy preview");
        let policy = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.path() == config_path)
            .expect("policy edit");
        assert!(policy.after().contains("cut: [M-v9] # keep"));
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        assert!(
            std::fs::read_to_string(&config_path)
                .expect("updated config")
                .contains("M-v9")
        );

        let external = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("external tempdir");
        let external_path = external.path().join("policy.yml");
        let external_text = config_text.replace("M-v1", "M-v9");
        write(&external_path, &external_text);
        let external_config =
            aplan_config::parse_config(&external_text, &external_path).expect("external config");
        let external_context = PlanContext::new(
            aplan_core::load_plan_identified(&dir.path().join("aplan")),
            ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(external_path.clone()),
                trust: Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(external_config);
        assert!(matches!(
            preview_rekey(&external_context, &RekeyRequest {
                old: "M-v9".to_string(),
                new: "M-v10".to_string(),
                allow_terminal: false,
            }),
            Err(AuthoringError::Refused(message)) if message.contains("outside the repository")
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let linked_path = dir.path().join("linked-policy.yml");
            symlink(&external_path, &linked_path).expect("external policy symlink");
            let linked_context = PlanContext::new(
                aplan_core::load_plan_identified(&dir.path().join("aplan")),
                ContextInputs {
                    repository_root: dir.path().to_path_buf(),
                    configuration_file: Some(linked_path),
                    trust: Trust::RepositoryRead,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            )
            .with_report_config(
                aplan_config::parse_config(&external_text, &external_path)
                    .expect("linked external config"),
            );
            assert!(matches!(
                preview_rekey(&linked_context, &RekeyRequest {
                    old: "M-v9".to_string(),
                    new: "M-v10".to_string(),
                    allow_terminal: false,
                }),
                Err(AuthoringError::Refused(message))
                    if message.contains("outside the repository")
            ));
        }
    }

    #[test]
    fn rekey_refuses_an_id_that_is_already_a_cut_label() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        assert!(matches!(
            preview_rekey(&context, &RekeyRequest {
                old: "A-1".to_string(),
                new: "v1".to_string(),
                allow_terminal: false,
            }),
            Err(AuthoringError::Refused(message))
                if message == "`v1` is already the label of cut(s) M-v1; using it as a task id would make that label unusable"
        ));
    }

    #[test]
    fn rekey_refuses_collisions_terminal_history_claims_and_stale_destinations() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        for (request, expected) in [
            (
                RekeyRequest {
                    old: "A-1".to_string(),
                    new: "A-1".to_string(),
                    allow_terminal: false,
                },
                "already the requested id",
            ),
            (
                RekeyRequest {
                    old: "A-1".to_string(),
                    new: "../A-9".to_string(),
                    allow_terminal: false,
                },
                "task id and file stem",
            ),
            (
                RekeyRequest {
                    old: "A-1".to_string(),
                    new: "A-2".to_string(),
                    allow_terminal: false,
                },
                "already exists",
            ),
        ] {
            assert!(
                message(preview_rekey(&context, &request)).contains(expected),
                "missing {expected:?}"
            );
        }

        write(
            &dir.path().join("aplan/tasks/A-9.md"),
            "occupied but deliberately not part of the captured plan\n",
        );
        assert!(
            message(preview_rekey(
                &context,
                &RekeyRequest {
                    old: "A-1".to_string(),
                    new: "A-9".to_string(),
                    allow_terminal: false,
                },
            ))
            .contains("destination")
        );

        let terminal = fixture();
        write(
            &terminal.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: done\neffort_days: 1\nevidence:\n- kind: note\n  ref: reviewed\n  captured: 2026-10-20\npaths: [src/one.rs]\n---\n",
        );
        let terminal_context = self::context(terminal.path(), aplan_core::Options::default());
        let terminal_request = RekeyRequest {
            old: "A-1".to_string(),
            new: "A-9".to_string(),
            allow_terminal: false,
        };
        assert!(message(preview_rekey(&terminal_context, &terminal_request)).contains("terminal"));
        assert!(
            preview_rekey(
                &terminal_context,
                &RekeyRequest {
                    allow_terminal: true,
                    ..terminal_request
                },
            )
            .is_ok()
        );
    }

    #[test]
    fn rekey_refuses_a_locally_claimed_task() {
        let claimed = fixture();
        let status = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(claimed.path())
            .status()
            .expect("git init");
        assert!(status.success());
        let claimed_context = self::context(claimed.path(), aplan_core::Options::default());
        crate::claims::claim(
            &claimed_context.identity().plan_root,
            claimed_context.plan(),
            "A-1",
            &aplan_interop::identity::ResolvedIdentity {
                name: "holder".to_string(),
                source: aplan_interop::identity::IdentitySource::Argument,
            },
            None,
            1,
        )
        .expect("claim");
        assert!(
            message(preview_rekey(
                &claimed_context,
                &RekeyRequest {
                    old: "A-1".to_string(),
                    new: "A-9".to_string(),
                    allow_terminal: false,
                },
            ))
            .contains("claimed by `holder`")
        );
    }

    #[test]
    fn rekey_repairs_every_structured_edge_and_keeps_custom_filenames() {
        let dir = fixture();
        std::fs::rename(
            dir.path().join("aplan/tasks/A-1.md"),
            dir.path().join("aplan/tasks/custom-name.md"),
        )
        .expect("custom task filename");
        write(
            &dir.path().join("aplan/tasks/A-2.md"),
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1]\nprefer_after: [A-1]\nratified_by: [A-1]\ncontains: [A-1]\npaths: [src/two.rs]\n---\n",
        );
        let context = context(dir.path(), aplan_core::Options::default());
        let preview = preview_rekey(
            &context,
            &RekeyRequest {
                old: "A-1".to_string(),
                new: "A-9".to_string(),
                allow_terminal: false,
            },
        )
        .expect("structured rewrite");
        let relocation = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.source_path().is_some())
            .expect("in-place relocation record");
        assert_eq!(relocation.source_path(), Some(relocation.path()));
        let dependent = preview
            .change_set()
            .files()
            .iter()
            .find(|file| file.path().ends_with("A-2.md"))
            .expect("dependent rewrite")
            .after();
        for field in ["blocked_by", "prefer_after", "ratified_by", "contains"] {
            assert!(
                dependent.contains(&format!("{field}: [A-9]")),
                "{field} was not rekeyed in {dependent}"
            );
        }
    }

    #[test]
    fn rekey_policy_failures_are_typed_before_publication() {
        let config_text = "policy:\n  - when:\n      cut: [M-v1]\n    evidence:\n      require_min_strength: [test]\n";

        let no_path = fixture();
        let virtual_path = no_path.path().join("virtual.yml");
        let config = aplan_config::parse_config(config_text, &virtual_path).expect("config");
        let context = context(no_path.path(), aplan_core::Options::default())
            .with_report_config(config.clone());
        assert!(
            message(preview_rekey(
                &context,
                &RekeyRequest {
                    old: "M-v1".to_string(),
                    new: "M-v9".to_string(),
                    allow_terminal: false,
                },
            ))
            .contains("no configuration file")
        );

        let missing = fixture();
        let missing_path = missing.path().join("missing.yml");
        let expected_missing_path = absolute(&missing_path);
        let context = PlanContext::new(
            aplan_core::load_plan_identified(&missing.path().join("aplan")),
            ContextInputs {
                repository_root: missing.path().to_path_buf(),
                configuration_file: Some(missing_path.clone()),
                trust: Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(config.clone());
        assert!(matches!(
            preview_rekey(&context, &RekeyRequest {
                old: "M-v1".to_string(),
                new: "M-v9".to_string(),
                allow_terminal: false,
            }),
            Err(AuthoringError::Read { path, .. }) if path == expected_missing_path
        ));

        let stale = fixture();
        let stale_path = stale.path().join(".aplan.yml");
        write(&stale_path, "policy: []\n");
        let context = PlanContext::new(
            aplan_core::load_plan_identified(&stale.path().join("aplan")),
            ContextInputs {
                repository_root: stale.path().to_path_buf(),
                configuration_file: Some(stale_path),
                trust: Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(config);
        assert!(
            message(preview_rekey(
                &context,
                &RekeyRequest {
                    old: "M-v1".to_string(),
                    new: "M-v9".to_string(),
                    allow_terminal: false,
                },
            ))
            .contains("source location could not be rewritten")
        );
    }

    #[test]
    fn rekey_model_validation_and_reference_scan_fail_closed_at_boundaries() {
        let dir = fixture();
        let context = context(dir.path(), aplan_core::Options::default());
        let expected = aplan_core::mutation::clone_of(context.plan());
        let mut missing = aplan_core::mutation::clone_of(context.plan());
        missing.tasks.remove(0);
        assert!(matches!(
            validate_rekey_model(&expected, &missing, "A-1"),
            Err(AuthoringError::Plan(error)) if error.contains("lost")
        ));
        let mut different = aplan_core::mutation::clone_of(context.plan());
        different.tasks[0].task.title = "different".to_string();
        assert!(matches!(
            validate_rekey_model(&expected, &different, "A-1"),
            Err(AuthoringError::Plan(error)) if error.contains("planned structured state")
        ));

        assert!(matches!(
            source(&context, &dir.path().join("not-captured.md"), "A-1"),
            Err(AuthoringError::Refused(error)) if error.contains("changed while")
        ));
        let mut paths = BTreeSet::new();
        collect_text_files(&dir.path().join("does-not-exist"), &mut paths);
        assert!(paths.is_empty());

        let binary = dir.path().join("binary.dat");
        std::fs::write(&binary, [0xff, 0xfe, 0xfd]).expect("binary fixture");
        let references = prose_occurrences(dir.path(), "A-1", &BTreeSet::new());
        assert!(!references.iter().any(|reference| reference.path == binary));

        let boundary = text_occurrences(
            Path::new("README.md"),
            "xA-1 A-1x A-1.md A-1. A-1_more A-1\n",
            "A-1",
        );
        assert_eq!(boundary.len(), 2);
        assert_eq!(boundary[0].column, 11);
    }

    #[test]
    fn rekey_cannot_scan_repository_prose_through_a_hermetic_context() {
        let dir = fixture();
        let context = PlanContext::new(
            aplan_core::load_plan_identified(&dir.path().join("aplan")),
            ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        );
        assert!(matches!(
            preview_rekey(&context, &RekeyRequest {
                old: "A-1".to_string(),
                new: "A-9".to_string(),
                allow_terminal: false,
            }),
            Err(AuthoringError::Refused(message)) if message.contains("repository-read")
        ));
    }

    #[test]
    fn an_id_race_is_stale_before_any_batch_member_is_published() {
        let dir = fixture();
        let loaded_context = context(dir.path(), aplan_core::Options::default());
        let request = AddRequest {
            title: "racing task".to_string(),
            id: None,
            phase: None,
            cut: Some("M-v1".to_string()),
            effort_days: Some(1.0),
            owner: None,
            status: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            members: Vec::new(),
            tags: Vec::new(),
            paths: vec!["src/race.rs".to_string()],
            traces: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            evidence: Vec::new(),
            body: None,
        };
        let preview = preview_add(&loaded_context, &request).expect("preview");
        assert_eq!(preview.created(), ["A-3"]);
        write(
            &dir.path().join("aplan/tasks/A-3.md"),
            "---\nid: A-3\ntitle: winner\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/winner.rs]\n---\n",
        );
        assert!(matches!(
            commit_change_set(&loaded_context, preview.into_change_set()),
            CommitOutcome::Stale { .. }
        ));
        let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-v1.md")).expect("cut");
        assert!(!cut.contains("- A-3"));
        let refreshed = context(dir.path(), aplan_core::Options::default());
        let reminted = preview_add(&refreshed, &request).expect("recomputed preview");
        assert_eq!(reminted.created(), ["A-4"]);
    }
}
