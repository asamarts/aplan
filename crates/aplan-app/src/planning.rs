//! Guided dependency, classification, placement and cancellation operations.
//!
//! These operations retain the context a person needs to choose an edit and
//! the exact [`crate::ChangeSet`] the common guarded writer can publish. They do
//! not write while constructing a catalogue or preview.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use aplan_core::{Cardinality, Graph, MilestoneScope, Plan, Status, StatusScope};

use crate::{
    ChangeEvidence, ChangeSet, DependencyOperation, DependencyRequest, FileChange, PlanContext,
    SemanticEffect, absolute,
};

/// Whether an ordering edge blocks work or only ranks concurrently ready work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    Blocking,
    Preference,
}

/// One guided edge edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDependencyRequest {
    pub id: String,
    pub after: String,
    pub operation: DependencyOperation,
    pub mode: EdgeMode,
}

/// Add, remove or broadly rename a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagRequest {
    Add { id: String, tag: String },
    Remove { id: String, tag: String },
    Rename { from: String, to: String },
}

/// Move one or more tasks between a cut, a phase, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveRequest {
    pub ids: Vec<String>,
    pub cut: Option<String>,
    pub phase: Option<u32>,
}

/// Drop named tasks, or named cuts together with their live work members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropRequest {
    pub ids: Vec<String>,
    pub by_cut: bool,
}

/// Every planning mutation supported by the guided surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningRequest {
    Dependency(PlanningDependencyRequest),
    Tag(TagRequest),
    Move(MoveRequest),
    Drop(DropRequest),
}

/// Task context shown beside a candidate or affected item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskChoice {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub cuts: Vec<String>,
    pub milestone: bool,
}

/// One possible edge endpoint, including whether the selected task already
/// carries that edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyChoice {
    pub task: TaskChoice,
    pub current: bool,
}

/// Candidate endpoints for one selected task. The selected task is never in
/// `choices`, so a picker cannot offer a self-edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyCatalogue {
    pub selected: TaskChoice,
    pub mode: EdgeMode,
    pub choices: Vec<DependencyChoice>,
}

/// One configured tag vocabulary, including its scope and what the selected
/// task currently carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFamilyChoice {
    pub name: String,
    pub members: Vec<String>,
    pub require: Cardinality,
    pub status: StatusScope,
    pub milestones: MilestoneScope,
    pub carried: Vec<String>,
}

/// Tag choices retain declared vocabularies and every tag actually present in
/// the plan but outside those vocabularies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagCatalogue {
    pub selected: Option<TaskChoice>,
    pub families: Vec<TagFamilyChoice>,
    pub arbitrary: Vec<String>,
}

/// How the complete ready set changes if a preview is committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessImpact {
    pub before: Vec<String>,
    pub after: Vec<String>,
    pub became_ready: Vec<String>,
    pub became_unready: Vec<String>,
}

/// Context specific to an edge preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyReview {
    pub target: TaskChoice,
    pub prerequisite: Option<TaskChoice>,
    pub mode: EdgeMode,
    pub operation: DependencyOperation,
}

/// Context specific to a tag preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagReview {
    pub affected: Vec<TaskChoice>,
    pub catalogue: TagCatalogue,
}

/// One task's placement before and after a move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveTransition {
    pub task: TaskChoice,
    pub source_cuts: Vec<String>,
    pub destination_cut: Option<String>,
    pub phase_before: u32,
    pub phase_after: u32,
}

/// Context specific to a cut/phase move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveReview {
    pub transitions: Vec<MoveTransition>,
    pub milestone_files: Vec<PathBuf>,
}

/// Context specific to a task or cut cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropReview {
    pub requested_cuts: Vec<TaskChoice>,
    pub affected: Vec<TaskChoice>,
    /// Terminal members of a cut are retained and shown explicitly; direct
    /// terminal targets are refused rather than appearing here.
    pub retained_terminal_members: Vec<TaskChoice>,
}

/// Operation-specific review data carried beside the generic exact preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningReview {
    Dependency(DependencyReview),
    Tag(TagReview),
    Move(MoveReview),
    Drop(DropReview),
}

/// A planning preview, including the selection an interactive reload should
/// retain after publishing it.
#[derive(Debug)]
pub struct PlanningPreview {
    change: ChangeSet,
    review: PlanningReview,
    readiness: ReadinessImpact,
    selection: Option<String>,
}

impl PlanningPreview {
    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    #[must_use]
    pub const fn review(&self) -> &PlanningReview {
        &self.review
    }

    #[must_use]
    pub const fn readiness(&self) -> &ReadinessImpact {
        &self.readiness
    }

    /// Task id to reselect after the shell reloads the committed snapshot.
    #[must_use]
    pub fn selection_after_commit(&self) -> Option<&str> {
        self.selection.as_deref()
    }

    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }
}

/// A malformed request or a domain refusal discovered before publication.
#[derive(Debug, thiserror::Error)]
pub enum PlanningError {
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
}

/// Candidate endpoints for a dependency or preference form.
pub fn dependency_catalogue(
    context: &PlanContext,
    id: &str,
    mode: EdgeMode,
) -> Result<DependencyCatalogue, PlanningError> {
    ensure_clean(context.plan())?;
    let selected = unique_task(context.plan(), id)?;
    let selected_choice = task_choice(context.plan(), &selected.task);
    let edges = match mode {
        EdgeMode::Blocking => &selected.task.blocked_by,
        EdgeMode::Preference => &selected.task.prefer_after,
    };
    let choices = context
        .plan()
        .tasks
        .iter()
        .filter(|loaded| loaded.task.id != id)
        .map(|loaded| DependencyChoice {
            task: task_choice(context.plan(), &loaded.task),
            current: edges.contains(&loaded.task.id),
        })
        .collect();
    Ok(DependencyCatalogue {
        selected: selected_choice,
        mode,
        choices,
    })
}

/// Declared families plus arbitrary tags for a selected task, or for a broad
/// rename when `id` is absent.
pub fn tag_catalogue(
    context: &PlanContext,
    id: Option<&str>,
) -> Result<TagCatalogue, PlanningError> {
    ensure_clean(context.plan())?;
    let selected_task = id.map(|id| unique_task(context.plan(), id)).transpose()?;
    let selected = selected_task.map(|loaded| task_choice(context.plan(), &loaded.task));
    let selected_model = selected_task.map(|loaded| &loaded.task);
    let declared = context
        .options()
        .tag_families
        .values()
        .flat_map(|family| family.members.iter().cloned())
        .collect::<BTreeSet<_>>();
    let families = context
        .options()
        .tag_families
        .iter()
        .map(|(name, family)| TagFamilyChoice {
            name: name.clone(),
            members: family.members.clone(),
            require: family.require,
            status: family.status,
            milestones: family.milestones,
            carried: selected_model.map_or_else(Vec::new, |task| {
                family
                    .carried(task)
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            }),
        })
        .collect();
    let arbitrary = context
        .plan()
        .tasks
        .iter()
        .flat_map(|loaded| loaded.task.tags.iter().cloned())
        .filter(|tag| !declared.contains(tag))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(TagCatalogue {
        selected,
        families,
        arbitrary,
    })
}

/// Compute one exact guided mutation without writing.
pub fn preview_planning(
    context: &PlanContext,
    request: &PlanningRequest,
) -> Result<PlanningPreview, PlanningError> {
    match request {
        PlanningRequest::Dependency(request) => preview_edge(context, request),
        PlanningRequest::Tag(request) => preview_tag(context, request),
        PlanningRequest::Move(request) => preview_move(context, request),
        PlanningRequest::Drop(request) => preview_drop(context, request),
    }
}

fn preview_edge(
    context: &PlanContext,
    request: &PlanningDependencyRequest,
) -> Result<PlanningPreview, PlanningError> {
    ensure_clean(context.plan())?;
    if request.id == request.after {
        return Err(PlanningError::Arguments(format!(
            "`{}` cannot come after itself",
            request.id
        )));
    }
    let target = unique_task(context.plan(), &request.id)?;
    let review = DependencyReview {
        target: task_choice(context.plan(), &target.task),
        prerequisite: context
            .plan()
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == request.after)
            .map(|loaded| task_choice(context.plan(), &loaded.task)),
        mode: request.mode,
        operation: request.operation,
    };
    let (change, after) = match request.mode {
        EdgeMode::Blocking => {
            let preview = crate::preview_dependency(
                context,
                &DependencyRequest {
                    id: request.id.clone(),
                    after: request.after.clone(),
                    operation: request.operation,
                },
            )
            .map_err(map_dependency_error)?;
            let after = edge_post_state(context.plan(), request);
            (preview.into_change_set(), after)
        }
        EdgeMode::Preference => preview_preference(context, request)?,
    };
    Ok(PlanningPreview {
        readiness: readiness_impact(context.plan(), &after),
        change,
        review: PlanningReview::Dependency(review),
        selection: Some(request.id.clone()),
    })
}

fn preview_preference(
    context: &PlanContext,
    request: &PlanningDependencyRequest,
) -> Result<(ChangeSet, Plan), PlanningError> {
    let target = unique_task(context.plan(), &request.id)?;
    let mut edges = target.task.prefer_after.clone();
    let present = edges.contains(&request.after);
    let adding = request.operation == DependencyOperation::Add;
    if present == adding {
        return Ok((
            empty_change(context),
            aplan_core::mutation::clone_of(context.plan()),
        ));
    }
    if adding {
        edges.push(request.after.clone());
    } else {
        edges.retain(|edge| edge != &request.after);
    }
    let original = source(context, &target.path, &request.id)?;
    let updated = aplan_core::close::set_prefer_after(&original, &edges).map_err(|source| {
        PlanningError::Rewrite {
            id: request.id.clone(),
            source,
        }
    })?;
    let produced = parse_task(&request.id, &updated)?;
    let after = aplan_core::mutation::with_replaced(context.plan(), &request.id, produced);
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    let effect = if adding {
        SemanticEffect::PreferenceAdded {
            task: request.id.clone(),
            after: request.after.clone(),
        }
    } else {
        SemanticEffect::PreferenceRemoved {
            task: request.id.clone(),
            after: request.after.clone(),
        }
    };
    Ok((
        ChangeSet::from_parts(
            context.identity().clone(),
            vec![FileChange::replacement(
                target.path.clone(),
                original,
                updated,
            )],
            vec![effect],
            diagnostics,
            denied,
            ChangeEvidence::NotRequired,
        ),
        after,
    ))
}

fn edge_post_state(plan: &Plan, request: &PlanningDependencyRequest) -> Plan {
    let mut after = aplan_core::mutation::clone_of(plan);
    if let Some(target) = after
        .tasks
        .iter_mut()
        .find(|loaded| loaded.task.id == request.id)
    {
        let edges = &mut target.task.blocked_by;
        match request.operation {
            DependencyOperation::Add if !edges.contains(&request.after) => {
                edges.push(request.after.clone());
            }
            DependencyOperation::Remove => edges.retain(|edge| edge != &request.after),
            DependencyOperation::Add => {}
        }
    }
    after
}

fn preview_tag(
    context: &PlanContext,
    request: &TagRequest,
) -> Result<PlanningPreview, PlanningError> {
    ensure_clean(context.plan())?;
    match request {
        TagRequest::Add { id, tag } => preview_tag_one(context, id, tag, true),
        TagRequest::Remove { id, tag } => preview_tag_one(context, id, tag, false),
        TagRequest::Rename { from, to } => preview_tag_rename(context, from, to),
    }
}

fn preview_tag_one(
    context: &PlanContext,
    id: &str,
    tag: &str,
    adding: bool,
) -> Result<PlanningPreview, PlanningError> {
    refuse_blank_tag(tag)?;
    let target = unique_task(context.plan(), id)?;
    let catalogue = tag_catalogue(context, Some(id))?;
    let affected = vec![task_choice(context.plan(), &target.task)];
    let held = target.task.tags.iter().any(|candidate| candidate == tag);
    if held == adding {
        let after = aplan_core::mutation::clone_of(context.plan());
        return Ok(finish(
            context,
            empty_change(context),
            &after,
            PlanningReview::Tag(TagReview {
                affected,
                catalogue,
            }),
            Some(id.to_string()),
        ));
    }
    if !adding && tag == "milestone" {
        let members = target
            .task
            .blocked_by
            .iter()
            .filter(|member| {
                context
                    .plan()
                    .tasks
                    .iter()
                    .any(|loaded| loaded.task.id == **member && !loaded.task.is_milestone())
            })
            .count();
        if members > 0 {
            return Err(PlanningError::Refused(format!(
                "`{id}` is a cut holding {members} task(s), and taking `milestone` off it would leave them in no cut while nothing recorded that they were ever in one. Move the work first, or cancel the cut itself if it is not going to happen."
            )));
        }
    }
    let mut tags = target.task.tags.clone();
    if adding {
        tags.push(tag.to_string());
    } else {
        tags.retain(|candidate| candidate != tag);
    }
    let original = source(context, &target.path, id)?;
    let updated =
        aplan_core::close::set_tags(&original, &tags).map_err(|source| PlanningError::Rewrite {
            id: id.to_string(),
            source,
        })?;
    let after = aplan_core::mutation::with_replaced(context.plan(), id, parse_task(id, &updated)?);
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    let effect = if adding {
        SemanticEffect::TagAdded {
            task: id.to_string(),
            tag: tag.to_string(),
        }
    } else {
        SemanticEffect::TagRemoved {
            task: id.to_string(),
            tag: tag.to_string(),
        }
    };
    let change = ChangeSet::from_parts(
        context.identity().clone(),
        vec![FileChange::replacement(
            target.path.clone(),
            original,
            updated,
        )],
        vec![effect],
        diagnostics,
        denied,
        ChangeEvidence::NotRequired,
    );
    Ok(finish(
        context,
        change,
        &after,
        PlanningReview::Tag(TagReview {
            affected,
            catalogue,
        }),
        Some(id.to_string()),
    ))
}

fn preview_tag_rename(
    context: &PlanContext,
    from: &str,
    to: &str,
) -> Result<PlanningPreview, PlanningError> {
    refuse_blank_tag(to)?;
    if from == to {
        return Err(PlanningError::Arguments(format!(
            "`{from}` is already what it would be renamed to"
        )));
    }
    for word in [from, to] {
        if aplan_core::validate::RESERVED_TAGS.contains(&word) {
            return Err(PlanningError::Refused(format!(
                "`{word}` is not a label this tool renames: it decides what a plan is; use structural operations instead"
            )));
        }
    }
    let catalogue = tag_catalogue(context, None)?;
    let targets = context
        .plan()
        .tasks
        .iter()
        .filter(|loaded| loaded.task.tags.iter().any(|tag| tag == from))
        .collect::<Vec<_>>();
    let affected = targets
        .iter()
        .map(|loaded| task_choice(context.plan(), &loaded.task))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        let after = aplan_core::mutation::clone_of(context.plan());
        return Ok(finish(
            context,
            empty_change(context),
            &after,
            PlanningReview::Tag(TagReview {
                affected,
                catalogue,
            }),
            None,
        ));
    }
    let mut files = Vec::with_capacity(targets.len());
    let mut after = aplan_core::mutation::clone_of(context.plan());
    let mut ids = Vec::with_capacity(targets.len());
    for target in targets {
        let mut tags = Vec::new();
        for tag in &target.task.tags {
            let next = if tag == from { to } else { tag };
            if !tags.iter().any(|kept| kept == next) {
                tags.push(next.to_string());
            }
        }
        let original = source(context, &target.path, &target.task.id)?;
        let updated = aplan_core::close::set_tags(&original, &tags).map_err(|source| {
            PlanningError::Rewrite {
                id: target.task.id.clone(),
                source,
            }
        })?;
        after = aplan_core::mutation::with_replaced(
            &after,
            &target.task.id,
            parse_task(&target.task.id, &updated)?,
        );
        files.push(FileChange::replacement(
            target.path.clone(),
            original,
            updated,
        ));
        ids.push(target.task.id.clone());
    }
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    let change = ChangeSet::from_parts(
        context.identity().clone(),
        files,
        vec![SemanticEffect::TagRenamed {
            from: from.to_string(),
            to: to.to_string(),
            tasks: ids.clone(),
        }],
        diagnostics,
        denied,
        ChangeEvidence::NotRequired,
    );
    Ok(finish(
        context,
        change,
        &after,
        PlanningReview::Tag(TagReview {
            affected,
            catalogue,
        }),
        ids.first().cloned(),
    ))
}

fn preview_move(
    context: &PlanContext,
    request: &MoveRequest,
) -> Result<PlanningPreview, PlanningError> {
    let prepared = prepare_move(context, request)?;
    let replacements = move_replacements(context, request.phase, &prepared)?;
    let after = apply_replacements(context.plan(), &replacements)?;
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    let files: Vec<FileChange> = replacements
        .into_iter()
        .map(|replacement| {
            FileChange::replacement(replacement.path, replacement.expected, replacement.updated)
        })
        .collect();
    let milestone_files = files
        .iter()
        .filter(|file| {
            context.plan().tasks.iter().any(|loaded| {
                loaded.task.is_milestone() && absolute(&loaded.path) == absolute(file.path())
            })
        })
        .map(|file| file.path().to_path_buf())
        .collect();
    let effects = move_effects(request.phase, &prepared);
    let selection = prepared.ids.first().cloned();
    let review = PlanningReview::Move(MoveReview {
        transitions: prepared.transitions,
        milestone_files,
    });
    let change = ChangeSet::from_parts(
        context.identity().clone(),
        files,
        effects,
        diagnostics,
        denied,
        ChangeEvidence::NotRequired,
    );
    Ok(finish(context, change, &after, review, selection))
}

struct PreparedMove {
    ids: Vec<String>,
    destination: Option<String>,
    placement: Option<aplan_interop::cut::PendingPlacement>,
    removals: Vec<(String, aplan_interop::cut::PendingRemoval)>,
    transitions: Vec<MoveTransition>,
}

fn prepare_move(
    context: &PlanContext,
    request: &MoveRequest,
) -> Result<PreparedMove, PlanningError> {
    ensure_clean(context.plan())?;
    if request.ids.is_empty() {
        return Err(PlanningError::Arguments(
            "move needs at least one task id".to_string(),
        ));
    }
    if request.cut.is_none() && request.phase.is_none() {
        return Err(PlanningError::Arguments(
            "move needs a destination cut, phase, or both".to_string(),
        ));
    }
    let mut ids = request.ids.clone();
    ids.sort();
    ids.dedup();
    for id in &ids {
        let moving = unique_task(context.plan(), id)?;
        if moving.task.is_milestone() {
            return Err(PlanningError::Refused(format!(
                "`{id}` is a cut, not a task in one: sequence cuts with a dependency instead"
            )));
        }
    }
    let destination = request
        .cut
        .as_deref()
        .map(|name| aplan_interop::cut::resolve_id(context.plan(), name))
        .transpose()
        .map_err(|error| PlanningError::Refused(error.to_string()))?;
    if let Some(cut) = &destination {
        aplan_interop::cut::check_target(context.plan(), cut)
            .map_err(|error| PlanningError::Refused(error.to_string()))?;
    }

    let mut leaving = BTreeMap::<String, Vec<String>>::new();
    if let Some(destination) = &destination {
        for cut in context
            .plan()
            .tasks
            .iter()
            .filter(|loaded| loaded.task.is_milestone() && loaded.task.id != *destination)
        {
            let going = ids
                .iter()
                .filter(|id| cut.task.blocked_by.contains(id))
                .cloned()
                .collect::<Vec<_>>();
            if !going.is_empty() {
                leaving.insert(cut.task.id.clone(), going);
            }
        }
    }

    let placement = destination
        .as_deref()
        .map(|cut| aplan_interop::cut::plan_placement(context.plan(), cut, &ids))
        .transpose()
        .map_err(|error| PlanningError::Refused(error.to_string()))?;
    let mut removals = Vec::new();
    for (cut, going) in &leaving {
        removals.push((
            cut.clone(),
            aplan_interop::cut::plan_removal(context.plan(), cut, going)
                .map_err(|error| PlanningError::Refused(error.to_string()))?,
        ));
    }

    let transitions = ids
        .iter()
        .map(|id| {
            let task = unique_task(context.plan(), id)?;
            Ok(MoveTransition {
                task: task_choice(context.plan(), &task.task),
                source_cuts: cuts_for(context.plan(), id),
                destination_cut: destination.clone(),
                phase_before: task.task.phase,
                phase_after: request.phase.unwrap_or(task.task.phase),
            })
        })
        .collect::<Result<Vec<_>, PlanningError>>()?;

    Ok(PreparedMove {
        ids,
        destination,
        placement,
        removals,
        transitions,
    })
}

fn move_replacements(
    context: &PlanContext,
    phase: Option<u32>,
    prepared: &PreparedMove,
) -> Result<Vec<aplan_core::save::Replacement>, PlanningError> {
    let mut replacements = Vec::new();
    if let Some(replacement) = prepared
        .placement
        .as_ref()
        .and_then(aplan_interop::cut::PendingPlacement::replacement)
    {
        replacements.push(replacement_from_identified_input(context, replacement)?);
    }
    for (_, removal) in &prepared.removals {
        if let Some(replacement) = removal.replacement() {
            replacements.push(replacement_from_identified_input(context, replacement)?);
        }
    }
    if let Some(phase) = phase {
        for id in &prepared.ids {
            let target = unique_task(context.plan(), id)?;
            let original = source(context, &target.path, id)?;
            let updated = aplan_core::close::set_phase(&original, phase).map_err(|source| {
                PlanningError::Rewrite {
                    id: id.clone(),
                    source,
                }
            })?;
            if updated != original {
                replacements.push(aplan_core::save::Replacement {
                    path: target.path.clone(),
                    expected: original,
                    updated,
                });
            }
        }
    }
    Ok(replacements)
}

fn move_effects(phase: Option<u32>, prepared: &PreparedMove) -> Vec<SemanticEffect> {
    let mut effects = Vec::new();
    if let (Some(cut), Some(placement)) = (&prepared.destination, &prepared.placement)
        && !placement.added.is_empty()
    {
        effects.push(SemanticEffect::CutMembershipAdded {
            cut: cut.clone(),
            tasks: placement.added.clone(),
        });
    }
    for (cut, removal) in &prepared.removals {
        if !removal.removed().is_empty() {
            effects.push(SemanticEffect::CutMembershipRemoved {
                cut: cut.clone(),
                tasks: removal.removed().to_vec(),
            });
        }
    }
    if let Some(phase) = phase {
        for transition in &prepared.transitions {
            if transition.phase_before != phase {
                effects.push(SemanticEffect::TaskPhaseChanged {
                    task: transition.task.id.clone(),
                    from: transition.phase_before,
                    to: phase,
                });
            }
        }
    }
    effects
}

fn preview_drop(
    context: &PlanContext,
    request: &DropRequest,
) -> Result<PlanningPreview, PlanningError> {
    let scope = prepare_drop(context, request)?;
    let mut files = Vec::with_capacity(scope.target_ids.len());
    let mut after = aplan_core::mutation::clone_of(context.plan());
    let mut effects = Vec::with_capacity(scope.target_ids.len());
    for id in &scope.target_ids {
        let target = unique_task(context.plan(), id)?;
        let original = source(context, &target.path, id)?;
        let updated =
            aplan_core::close::set_status(&original, Status::Dropped).map_err(|source| {
                PlanningError::Rewrite {
                    id: id.clone(),
                    source,
                }
            })?;
        after = aplan_core::mutation::with_replaced(&after, id, parse_task(id, &updated)?);
        files.push(FileChange::replacement(
            target.path.clone(),
            original,
            updated,
        ));
        effects.push(SemanticEffect::TaskStatusChanged {
            task: id.clone(),
            from: target.task.status,
            to: Status::Dropped,
        });
    }
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    let change = ChangeSet::from_parts(
        context.identity().clone(),
        files,
        effects,
        diagnostics,
        denied,
        ChangeEvidence::NotRequired,
    );
    Ok(finish(
        context,
        change,
        &after,
        PlanningReview::Drop(DropReview {
            requested_cuts: scope.cuts,
            affected: scope.affected,
            retained_terminal_members: scope.retained,
        }),
        scope.selection,
    ))
}

struct DropScope {
    selection: Option<String>,
    target_ids: Vec<String>,
    cuts: Vec<TaskChoice>,
    affected: Vec<TaskChoice>,
    retained: Vec<TaskChoice>,
}

fn prepare_drop(context: &PlanContext, request: &DropRequest) -> Result<DropScope, PlanningError> {
    ensure_clean(context.plan())?;
    if request.ids.is_empty() {
        return Err(PlanningError::Arguments(
            "drop needs at least one task or cut".to_string(),
        ));
    }
    let ids = if request.by_cut {
        request
            .ids
            .iter()
            .map(|name| aplan_interop::cut::resolve_id(context.plan(), name))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| PlanningError::Refused(error.to_string()))?
    } else {
        request.ids.clone()
    };
    let by_id = context
        .plan()
        .tasks
        .iter()
        .map(|loaded| (loaded.task.id.as_str(), loaded))
        .collect::<BTreeMap<_, _>>();
    let mut targets = Vec::new();
    let mut cuts = Vec::new();
    let mut retained = Vec::new();
    for id in &ids {
        let target = unique_task(context.plan(), id)?;
        if request.by_cut {
            if !target.task.is_milestone() {
                return Err(PlanningError::Refused(format!(
                    "`{id}` is not a milestone; drop it as a task instead"
                )));
            }
            cuts.push(task_choice(context.plan(), &target.task));
            for member in &target.task.blocked_by {
                let Some(member) = by_id.get(member.as_str()) else {
                    continue;
                };
                if member.task.is_milestone() {
                    continue;
                }
                if member.task.status.is_terminal() {
                    retained.push(*member);
                } else {
                    targets.push(*member);
                }
            }
        }
        if target.task.status.is_terminal() {
            return Err(PlanningError::Refused(format!(
                "`{id}` is already `{}`; reviving a terminal task would launder a decision the plan recorded",
                target.task.status
            )));
        }
        targets.push(target);
    }
    let mut seen = BTreeSet::new();
    targets.retain(|target| seen.insert(target.task.id.clone()));
    let mut cut_seen = BTreeSet::new();
    cuts.retain(|cut| cut_seen.insert(cut.id.clone()));
    let mut retained_seen = BTreeSet::new();
    retained.retain(|target| retained_seen.insert(target.task.id.clone()));

    let affected = targets
        .iter()
        .map(|target| task_choice(context.plan(), &target.task))
        .collect();
    let retained = retained
        .iter()
        .map(|target| task_choice(context.plan(), &target.task))
        .collect();
    Ok(DropScope {
        selection: ids.first().cloned(),
        target_ids: targets
            .iter()
            .map(|target| target.task.id.clone())
            .collect(),
        cuts,
        affected,
        retained,
    })
}

fn finish(
    context: &PlanContext,
    change: ChangeSet,
    after: &Plan,
    review: PlanningReview,
    selection: Option<String>,
) -> PlanningPreview {
    PlanningPreview {
        readiness: readiness_impact(context.plan(), after),
        change,
        review,
        selection,
    }
}

fn readiness_impact(before: &Plan, after: &Plan) -> ReadinessImpact {
    let before = ready_ids(before);
    let after = ready_ids(after);
    let before_set = before.iter().collect::<BTreeSet<_>>();
    let after_set = after.iter().collect::<BTreeSet<_>>();
    let became_ready = after
        .iter()
        .filter(|id| !before_set.contains(id))
        .cloned()
        .collect();
    let became_unready = before
        .iter()
        .filter(|id| !after_set.contains(id))
        .cloned()
        .collect();
    ReadinessImpact {
        before,
        after,
        became_ready,
        became_unready,
    }
}

fn ready_ids(plan: &Plan) -> Vec<String> {
    Graph::new(plan.tasks.iter().map(|loaded| &loaded.task))
        .ready()
        .into_iter()
        .map(|task| task.id.clone())
        .collect()
}

fn empty_change(context: &PlanContext) -> ChangeSet {
    ChangeSet::from_parts(
        context.identity().clone(),
        Vec::new(),
        Vec::new(),
        aplan_core::mutation::Delta::default(),
        false,
        ChangeEvidence::NotRequired,
    )
}

fn ensure_clean(plan: &Plan) -> Result<(), PlanningError> {
    if plan.load_diagnostics.is_empty() {
        Ok(())
    } else {
        Err(PlanningError::Plan(
            "the plan does not load cleanly; fix it before changing its structure".to_string(),
        ))
    }
}

fn unique_task<'a>(
    plan: &'a Plan,
    id: &str,
) -> Result<&'a aplan_core::load::LoadedTask, PlanningError> {
    if let Some(reason) = aplan_core::mutation::ambiguous_id(plan, id) {
        return Err(PlanningError::Refused(reason));
    }
    plan.tasks
        .iter()
        .find(|loaded| loaded.task.id == id)
        .ok_or_else(|| PlanningError::Arguments(format!("no task `{id}` in this plan")))
}

fn task_choice(plan: &Plan, task: &aplan_core::Task) -> TaskChoice {
    TaskChoice {
        id: task.id.clone(),
        title: task.title.clone(),
        status: task.status,
        cuts: cuts_for(plan, &task.id),
        milestone: task.is_milestone(),
    }
}

fn cuts_for(plan: &Plan, id: &str) -> Vec<String> {
    plan.tasks
        .iter()
        .filter(|loaded| {
            loaded.task.is_milestone() && loaded.task.blocked_by.iter().any(|m| m == id)
        })
        .map(|loaded| loaded.task.id.clone())
        .collect()
}

fn source(context: &PlanContext, path: &Path, id: &str) -> Result<String, PlanningError> {
    context
        .source_text(path)
        .map(str::to_string)
        .ok_or_else(|| {
            PlanningError::Refused(format!(
                "`{id}` changed while the edit was being prepared; reload and try again"
            ))
        })
}

fn parse_task(id: &str, content: &str) -> Result<aplan_core::Task, PlanningError> {
    aplan_core::mutation::task_of(content)
        .map_err(|error| PlanningError::Plan(format!("`{id}`: {error}")))
}

fn apply_replacements(
    plan: &Plan,
    replacements: &[aplan_core::save::Replacement],
) -> Result<Plan, PlanningError> {
    let mut after = aplan_core::mutation::clone_of(plan);
    for replacement in replacements {
        let Some(target) = plan
            .tasks
            .iter()
            .find(|loaded| absolute(&loaded.path) == absolute(&replacement.path))
        else {
            return Err(PlanningError::Plan(format!(
                "{} is not a task in the identified plan",
                replacement.path.display()
            )));
        };
        after = aplan_core::mutation::with_replaced(
            &after,
            &target.task.id,
            parse_task(&target.task.id, &replacement.updated)?,
        );
    }
    Ok(after)
}

fn replacement_from_identified_input(
    context: &PlanContext,
    replacement: aplan_core::save::Replacement,
) -> Result<aplan_core::save::Replacement, PlanningError> {
    let Some(identified) = context.source_text(&replacement.path) else {
        return Err(PlanningError::Refused(format!(
            "{} is no longer readable in the identified plan; reload and try again",
            replacement.path.display()
        )));
    };
    if identified != replacement.expected {
        return Err(PlanningError::Refused(format!(
            "{} changed while the edit was being prepared; reload and try again",
            replacement.path.display()
        )));
    }
    Ok(replacement)
}

fn refuse_blank_tag(tag: &str) -> Result<(), PlanningError> {
    if tag.trim().is_empty() {
        Err(PlanningError::Arguments(format!(
            "a tag is a word, and `{tag}` is blank"
        )))
    } else {
        Ok(())
    }
}

fn map_dependency_error(error: crate::Error) -> PlanningError {
    match error {
        crate::Error::Arguments(message) => PlanningError::Arguments(message),
        crate::Error::Plan(message) => PlanningError::Plan(message),
        crate::Error::Refused(message) => PlanningError::Refused(message),
        crate::Error::Rewrite { id, source } => PlanningError::Rewrite { id, source },
        crate::Error::Write(error) => PlanningError::Refused(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChangeDisposition, CommitOutcome, ContextInputs, Trust, commit_change_set};

    fn write_task(root: &Path, name: &str, frontmatter: &str) {
        std::fs::write(
            root.join(name),
            format!("---\n{frontmatter}---\n\n## Why\n\nFixture.\n"),
        )
        .expect("write task");
    }

    fn load_context(
        root: &Path,
        options: aplan_core::Options,
        verdict: aplan_core::Verdict,
    ) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(root),
            ContextInputs {
                repository_root: root.to_path_buf(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options,
                verdict,
            },
        )
    }

    fn basic_plan(root: &Path) {
        write_task(
            root,
            "a.md",
            "id: A-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: [priority-high, custom]\n",
        );
        write_task(
            root,
            "b.md",
            "id: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: [priority-low]\n",
        );
        write_task(
            root,
            "m.md",
            "id: M-1\ntitle: first cut\nphase: 1\nstatus: todo\nblocked_by: [A-1, A-2]\ntags: [milestone, v1]\n",
        );
    }

    #[test]
    fn catalogues_exclude_self_and_keep_status_cut_families_and_arbitrary_tags() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        basic_plan(dir.path());
        let mut options = aplan_core::Options::default();
        options.tag_families.insert(
            "priority".to_string(),
            aplan_core::TagFamily {
                members: vec!["priority-high".to_string(), "priority-low".to_string()],
                require: Cardinality::ExactlyOne,
                status: StatusScope::Live,
                milestones: MilestoneScope::Exclude,
            },
        );
        let context = load_context(dir.path(), options, aplan_core::Verdict::default());

        let dependencies =
            dependency_catalogue(&context, "A-1", EdgeMode::Blocking).expect("catalogue");
        assert_eq!(dependencies.selected.cuts, ["M-1"]);
        assert!(
            dependencies
                .choices
                .iter()
                .all(|choice| choice.task.id != "A-1")
        );
        let second = dependencies
            .choices
            .iter()
            .find(|choice| choice.task.id == "A-2")
            .expect("second task");
        assert_eq!(second.task.status, Status::Todo);
        assert_eq!(second.task.cuts, ["M-1"]);

        let tags = tag_catalogue(&context, Some("A-1")).expect("tag catalogue");
        assert_eq!(tags.families.len(), 1);
        assert_eq!(tags.families[0].require, Cardinality::ExactlyOne);
        assert_eq!(tags.families[0].status, StatusScope::Live);
        assert_eq!(tags.families[0].milestones, MilestoneScope::Exclude);
        assert_eq!(tags.families[0].carried, ["priority-high"]);
        assert!(tags.arbitrary.contains(&"custom".to_string()));
        assert!(tags.arbitrary.contains(&"milestone".to_string()));
        assert!(tags.arbitrary.contains(&"v1".to_string()));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn edge_previews_cover_readiness_preferences_and_every_structural_refusal() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        basic_plan(dir.path());
        let context = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let request = PlanningRequest::Dependency(PlanningDependencyRequest {
            id: "A-1".to_string(),
            after: "A-2".to_string(),
            operation: DependencyOperation::Add,
            mode: EdgeMode::Blocking,
        });
        let preview = preview_planning(&context, &request).expect("dependency preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        assert!(preview.readiness().before.contains(&"A-1".to_string()));
        assert_eq!(preview.readiness().became_unready, ["A-1"]);
        assert_eq!(preview.selection_after_commit(), Some("A-1"));
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        assert!(
            aplan_core::load_plan(dir.path())
                .tasks
                .iter()
                .find(|task| task.task.id == "A-1")
                .expect("A-1")
                .task
                .blocked_by
                .contains(&"A-2".to_string())
        );

        let refreshed = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let repeated = preview_planning(
            &refreshed,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
        )
        .expect("repeated blocking edge");
        assert!(repeated.change_set().files().is_empty());
        let removed = preview_planning(
            &refreshed,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Remove,
                mode: EdgeMode::Blocking,
            }),
        )
        .expect("blocking edge removal");
        assert_eq!(removed.readiness().became_ready, ["A-1"]);
        let preference = preview_planning(
            &refreshed,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Preference,
            }),
        )
        .expect("preference preview");
        assert_eq!(preference.readiness().before, preference.readiness().after);
        assert!(matches!(
            preference.change_set().effects(),
            [SemanticEffect::PreferenceAdded { .. }]
        ));

        let self_edge = preview_planning(
            &refreshed,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-1".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
        )
        .expect_err("self edge refused");
        assert!(self_edge.to_string().contains("after itself"));

        let cycle = preview_planning(
            &refreshed,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
        )
        .expect("cycle preview");
        assert_eq!(cycle.change_set().disposition(), ChangeDisposition::Denied);
        assert!(
            cycle
                .change_set()
                .diagnostics()
                .added
                .iter()
                .any(|d| d.code == "APLAN004")
        );

        for (after, code) in [("missing", "APLAN003"), ("D-1", "APLAN020")] {
            if after == "D-1" {
                write_task(
                    dir.path(),
                    "d.md",
                    "id: D-1\ntitle: dropped\nphase: 1\nstatus: dropped\nblocked_by: []\ntags: []\n",
                );
            }
            let current = load_context(
                dir.path(),
                aplan_core::Options::default(),
                aplan_core::Verdict::default(),
            );
            let denied = preview_planning(
                &current,
                &PlanningRequest::Dependency(PlanningDependencyRequest {
                    id: "A-1".to_string(),
                    after: after.to_string(),
                    operation: DependencyOperation::Add,
                    mode: EdgeMode::Blocking,
                }),
            )
            .expect("denied preview");
            assert!(
                denied
                    .change_set()
                    .diagnostics()
                    .added
                    .iter()
                    .any(|d| d.code == code)
            );
        }

        let membership = preview_planning(
            &refreshed,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "M-1".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
                mode: EdgeMode::Blocking,
            }),
        )
        .expect_err("membership edit refused");
        assert!(membership.to_string().contains("which tasks are in it"));
    }

    #[test]
    fn tag_previews_gate_families_and_review_broad_exact_changes() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        basic_plan(dir.path());
        let mut options = aplan_core::Options::default();
        options.tag_families.insert(
            "priority".to_string(),
            aplan_core::TagFamily {
                members: vec!["priority-high".to_string(), "priority-low".to_string()],
                require: Cardinality::ExactlyOne,
                status: StatusScope::Live,
                milestones: MilestoneScope::Exclude,
            },
        );
        let context = load_context(dir.path(), options.clone(), aplan_core::Verdict::default());
        let denied = preview_planning(
            &context,
            &PlanningRequest::Tag(TagRequest::Add {
                id: "A-1".to_string(),
                tag: "priority-low".to_string(),
            }),
        )
        .expect("family preview");
        assert_eq!(denied.change_set().disposition(), ChangeDisposition::Denied);
        assert!(
            denied
                .change_set()
                .diagnostics()
                .added
                .iter()
                .any(|d| d.code == "APLAN030")
        );

        let rename = preview_planning(
            &context,
            &PlanningRequest::Tag(TagRequest::Rename {
                from: "priority-high".to_string(),
                to: "priority-low".to_string(),
            }),
        )
        .expect("rename preview");
        assert_eq!(rename.change_set().files().len(), 1);
        assert!(matches!(
            rename.change_set().effects(),
            [SemanticEffect::TagRenamed { tasks, .. }] if tasks == &["A-1"]
        ));
        assert_eq!(rename.selection_after_commit(), Some("A-1"));
        assert!(matches!(
            commit_change_set(&context, rename.into_change_set()),
            CommitOutcome::Applied(_)
        ));

        let refreshed = load_context(dir.path(), options, aplan_core::Verdict::default());
        let no_duplicate = &unique_task(refreshed.plan(), "A-1")
            .expect("task")
            .task
            .tags;
        assert_eq!(
            no_duplicate
                .iter()
                .filter(|tag| tag.as_str() == "priority-low")
                .count(),
            1
        );
        let milestone = preview_planning(
            &refreshed,
            &PlanningRequest::Tag(TagRequest::Remove {
                id: "M-1".to_string(),
                tag: "milestone".to_string(),
            }),
        )
        .expect_err("structural tag removal refused");
        assert!(milestone.to_string().contains("cut holding 2 task(s)"));
        for request in [
            TagRequest::Add {
                id: "A-1".to_string(),
                tag: "  ".to_string(),
            },
            TagRequest::Rename {
                from: "milestone".to_string(),
                to: "ordinary".to_string(),
            },
        ] {
            assert!(preview_planning(&refreshed, &PlanningRequest::Tag(request)).is_err());
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn move_previews_every_milestone_and_phase_file_as_one_guarded_change() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        write_task(
            dir.path(),
            "a.md",
            "id: A-1\ntitle: task\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: []\n",
        );
        write_task(
            dir.path(),
            "m1.md",
            "id: M-1\ntitle: source\nphase: 1\nstatus: todo\nblocked_by: [A-1]\ntags: [milestone, v1]\n",
        );
        write_task(
            dir.path(),
            "m2.md",
            "id: M-2\ntitle: destination\nphase: 2\nstatus: todo\nblocked_by: []\ntags: [milestone, v2]\n",
        );
        let context = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let preview = preview_planning(
            &context,
            &PlanningRequest::Move(MoveRequest {
                ids: vec!["A-1".to_string(), "A-1".to_string()],
                cut: Some("v2".to_string()),
                phase: Some(2),
            }),
        )
        .expect("move preview");
        assert_eq!(preview.change_set().files().len(), 3);
        let PlanningReview::Move(review) = preview.review() else {
            panic!("move review")
        };
        assert_eq!(review.transitions.len(), 1);
        assert_eq!(review.transitions[0].source_cuts, ["M-1"]);
        assert_eq!(
            review.transitions[0].destination_cut.as_deref(),
            Some("M-2")
        );
        assert_eq!(review.transitions[0].phase_before, 1);
        assert_eq!(review.transitions[0].phase_after, 2);
        assert_eq!(review.milestone_files.len(), 2);
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        let moved = aplan_core::load_plan(dir.path());
        assert_eq!(unique_task(&moved, "A-1").expect("task").task.phase, 2);
        assert!(
            !unique_task(&moved, "M-1")
                .expect("source")
                .task
                .blocked_by
                .contains(&"A-1".to_string())
        );
        assert!(
            unique_task(&moved, "M-2")
                .expect("destination")
                .task
                .blocked_by
                .contains(&"A-1".to_string())
        );

        let strict = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict {
                deny_warnings: true,
                advisory: Vec::new(),
            },
        );
        let broad = preview_planning(
            &strict,
            &PlanningRequest::Move(MoveRequest {
                ids: vec!["A-1".to_string()],
                cut: Some("M-1".to_string()),
                phase: None,
            }),
        )
        .expect("broad move preview");
        assert_eq!(broad.change_set().disposition(), ChangeDisposition::Denied);
        assert!(
            broad
                .change_set()
                .diagnostics()
                .added
                .iter()
                .any(|d| d.code == "APLAN021")
        );

        for bad in [
            MoveRequest {
                ids: vec!["missing".to_string()],
                cut: Some("M-1".to_string()),
                phase: None,
            },
            MoveRequest {
                ids: vec!["M-1".to_string()],
                cut: Some("M-2".to_string()),
                phase: None,
            },
            MoveRequest {
                ids: vec!["A-1".to_string()],
                cut: None,
                phase: None,
            },
        ] {
            assert!(preview_planning(&strict, &PlanningRequest::Move(bad)).is_err());
        }
        let stale_context = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let source_cut = dir.path().join("m1.md");
        let mut changed = std::fs::read_to_string(&source_cut).expect("source cut");
        changed.push('\n');
        std::fs::write(&source_cut, changed).expect("concurrent edit");
        assert!(
            preview_planning(
                &stale_context,
                &PlanningRequest::Move(MoveRequest {
                    ids: vec!["A-1".to_string()],
                    cut: Some("M-1".to_string()),
                    phase: None,
                })
            )
            .expect_err("mixed-snapshot move refused")
            .to_string()
            .contains("changed while the edit was being prepared")
        );
        std::fs::write(
            dir.path().join("m2.md"),
            std::fs::read_to_string(dir.path().join("m2.md"))
                .expect("read")
                .replace("status: todo", "status: done"),
        )
        .expect("settle cut");
        let settled = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        assert!(
            preview_planning(
                &settled,
                &PlanningRequest::Move(MoveRequest {
                    ids: vec!["A-1".to_string()],
                    cut: Some("M-2".to_string()),
                    phase: None,
                })
            )
            .expect_err("settled cut refused")
            .to_string()
            .contains("settled cut")
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn cut_drop_enumerates_live_changes_retained_terminal_members_and_reloads_selection() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        write_task(
            dir.path(),
            "a.md",
            "id: A-1\ntitle: live\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: []\n",
        );
        write_task(
            dir.path(),
            "b.md",
            "id: A-2\ntitle: completed\nphase: 1\nstatus: done\nblocked_by: []\ntags: []\nevidence:\n- kind: note\n  ref: reviewed\n  captured: 2026-10-25\n",
        );
        write_task(
            dir.path(),
            "m.md",
            "id: M-1\ntitle: cut\nphase: 1\nstatus: todo\nblocked_by: [A-1, A-2]\ntags: [milestone, v1]\n",
        );
        let context = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let preview = preview_planning(
            &context,
            &PlanningRequest::Drop(DropRequest {
                ids: vec!["v1".to_string()],
                by_cut: true,
            }),
        )
        .expect("cut drop");
        let PlanningReview::Drop(review) = preview.review() else {
            panic!("drop review")
        };
        assert_eq!(
            review
                .requested_cuts
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["M-1"]
        );
        assert_eq!(
            review
                .affected
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["A-1", "M-1"]
        );
        assert_eq!(
            review
                .retained_terminal_members
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["A-2"]
        );
        assert_eq!(preview.change_set().files().len(), 2);
        assert_eq!(preview.selection_after_commit(), Some("M-1"));
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        let reloaded = aplan_core::load_plan(dir.path());
        assert_eq!(
            unique_task(&reloaded, "A-1").expect("live").task.status,
            Status::Dropped
        );
        assert_eq!(
            unique_task(&reloaded, "M-1").expect("cut").task.status,
            Status::Dropped
        );
        assert_eq!(
            unique_task(&reloaded, "A-2").expect("done").task.status,
            Status::Done
        );

        let current = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let terminal = preview_planning(
            &current,
            &PlanningRequest::Drop(DropRequest {
                ids: vec!["A-2".to_string()],
                by_cut: false,
            }),
        )
        .expect_err("terminal task refused");
        assert!(terminal.to_string().contains("already `done`"));
        assert!(
            preview_planning(
                &current,
                &PlanningRequest::Drop(DropRequest {
                    ids: Vec::new(),
                    by_cut: false,
                })
            )
            .is_err()
        );
    }

    #[test]
    fn no_op_and_inverse_edges_and_tags_remain_exact_and_reviewable() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        basic_plan(dir.path());
        let context = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );

        let preference_catalogue =
            dependency_catalogue(&context, "A-1", EdgeMode::Preference).expect("catalogue");
        assert!(
            preference_catalogue
                .choices
                .iter()
                .all(|choice| !choice.current)
        );

        let remove_absent = preview_planning(
            &context,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Remove,
                mode: EdgeMode::Preference,
            }),
        )
        .expect("absent preference is an exact no-op");
        assert!(remove_absent.change_set().files().is_empty());
        assert!(remove_absent.change_set().effects().is_empty());

        let remove_absent_tag = preview_planning(
            &context,
            &PlanningRequest::Tag(TagRequest::Remove {
                id: "A-1".to_string(),
                tag: "not-carried".to_string(),
            }),
        )
        .expect("absent tag is an exact no-op");
        assert!(remove_absent_tag.change_set().files().is_empty());

        let add_held_tag = preview_planning(
            &context,
            &PlanningRequest::Tag(TagRequest::Add {
                id: "A-1".to_string(),
                tag: "custom".to_string(),
            }),
        )
        .expect("held tag is an exact no-op");
        assert!(add_held_tag.change_set().files().is_empty());

        let remove_tag = preview_planning(
            &context,
            &PlanningRequest::Tag(TagRequest::Remove {
                id: "A-1".to_string(),
                tag: "custom".to_string(),
            }),
        )
        .expect("remove tag");
        assert!(matches!(
            remove_tag.change_set().effects(),
            [SemanticEffect::TagRemoved { task, tag }]
                if task == "A-1" && tag == "custom"
        ));

        let add_tag = preview_planning(
            &context,
            &PlanningRequest::Tag(TagRequest::Add {
                id: "A-2".to_string(),
                tag: "custom".to_string(),
            }),
        )
        .expect("add tag");
        assert!(matches!(
            add_tag.change_set().effects(),
            [SemanticEffect::TagAdded { task, tag }]
                if task == "A-2" && tag == "custom"
        ));

        let absent_rename = preview_planning(
            &context,
            &PlanningRequest::Tag(TagRequest::Rename {
                from: "not-carried".to_string(),
                to: "replacement".to_string(),
            }),
        )
        .expect("absent rename is an exact no-op");
        assert!(absent_rename.change_set().files().is_empty());
        assert_eq!(absent_rename.selection_after_commit(), None);

        for request in [
            TagRequest::Rename {
                from: "same".to_string(),
                to: "same".to_string(),
            },
            TagRequest::Rename {
                from: "ordinary".to_string(),
                to: "milestone".to_string(),
            },
            TagRequest::Rename {
                from: "ordinary".to_string(),
                to: "\t".to_string(),
            },
        ] {
            assert!(preview_planning(&context, &PlanningRequest::Tag(request)).is_err());
        }
    }

    #[test]
    fn phase_only_moves_and_direct_drops_cover_argument_and_terminal_boundaries() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        basic_plan(dir.path());
        let context = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );

        let phase_only = preview_planning(
            &context,
            &PlanningRequest::Move(MoveRequest {
                ids: vec!["A-2".to_string(), "A-2".to_string()],
                cut: None,
                phase: Some(3),
            }),
        )
        .expect("phase-only move");
        assert_eq!(phase_only.change_set().files().len(), 1);
        assert!(matches!(
            phase_only.change_set().effects(),
            [SemanticEffect::TaskPhaseChanged {
                task,
                from: 1,
                to: 3
            }] if task == "A-2"
        ));
        let PlanningReview::Move(review) = phase_only.review() else {
            panic!("move review")
        };
        assert_eq!(review.transitions[0].destination_cut, None);
        assert_eq!(review.transitions[0].source_cuts, ["M-1"]);
        assert!(review.milestone_files.is_empty());

        let same_phase = preview_planning(
            &context,
            &PlanningRequest::Move(MoveRequest {
                ids: vec!["A-2".to_string()],
                cut: None,
                phase: Some(1),
            }),
        )
        .expect("same phase");
        assert!(same_phase.change_set().files().is_empty());
        assert!(same_phase.change_set().effects().is_empty());

        let direct_drop = preview_planning(
            &context,
            &PlanningRequest::Drop(DropRequest {
                ids: vec!["A-1".to_string(), "A-1".to_string()],
                by_cut: false,
            }),
        )
        .expect("direct task drop");
        assert_eq!(direct_drop.change_set().files().len(), 1);
        assert!(matches!(
            direct_drop.change_set().effects(),
            [SemanticEffect::TaskStatusChanged {
                task,
                from: Status::Todo,
                to: Status::Dropped
            }] if task == "A-1"
        ));

        let non_cut = preview_planning(
            &context,
            &PlanningRequest::Drop(DropRequest {
                ids: vec!["A-1".to_string()],
                by_cut: true,
            }),
        )
        .expect_err("non-cut refused by cut mode");
        assert!(non_cut.to_string().contains("not a milestone"));

        let empty_move = preview_planning(
            &context,
            &PlanningRequest::Move(MoveRequest {
                ids: Vec::new(),
                cut: None,
                phase: Some(2),
            }),
        )
        .expect_err("empty move refused");
        assert!(empty_move.to_string().contains("at least one task id"));

        let outside = aplan_core::save::Replacement {
            path: dir.path().join("outside.md"),
            expected: "old".to_string(),
            updated: "new".to_string(),
        };
        let foreign = apply_replacements(context.plan(), std::slice::from_ref(&outside))
            .expect_err("a move cannot apply a file outside its identified plan");
        assert!(foreign.to_string().contains("is not a task"));
        let missing = replacement_from_identified_input(&context, outside)
            .expect_err("a move cannot adopt bytes outside its identified input");
        assert!(missing.to_string().contains("no longer readable"));
    }

    #[test]
    fn stale_planning_inputs_and_dependency_errors_keep_their_exact_types() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        basic_plan(dir.path());
        let context = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );

        let stale = source(&context, &dir.path().join("missing.md"), "A-missing")
            .expect_err("a missing identified source is stale");
        assert!(stale.to_string().contains("`A-missing` changed"));
        let malformed = parse_task("A-bad", "---\nid: [\n---\n")
            .expect_err("malformed replacement bytes are a plan error");
        assert!(malformed.to_string().contains("`A-bad`"));

        let target = context
            .plan()
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == "A-1")
            .expect("fixture target");
        let malformed_replacement = aplan_core::save::Replacement {
            path: target.path.clone(),
            expected: context
                .source_text(&target.path)
                .expect("identified source")
                .into(),
            updated: "---\nid: [\n---\n".into(),
        };
        assert!(matches!(
            apply_replacements(context.plan(), &[malformed_replacement]),
            Err(PlanningError::Plan(_))
        ));

        let translated = [
            map_dependency_error(crate::Error::Arguments("arguments".into())),
            map_dependency_error(crate::Error::Plan("plan".into())),
            map_dependency_error(crate::Error::Rewrite {
                id: "A-1".into(),
                source: aplan_core::CloseError::NotATask,
            }),
            map_dependency_error(crate::Error::Write(aplan_core::save::SaveError::Changed {
                path: target.path.clone(),
            })),
        ];
        assert!(matches!(translated[0], PlanningError::Arguments(_)));
        assert!(matches!(translated[1], PlanningError::Plan(_)));
        assert!(matches!(translated[2], PlanningError::Rewrite { .. }));
        assert!(matches!(translated[3], PlanningError::Refused(_)));
    }

    #[test]
    fn preference_removal_empty_cut_reclassification_and_bad_plans_fail_exactly() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        write_task(
            dir.path(),
            "a.md",
            "id: A-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\nprefer_after: [A-2]\ntags: []\n",
        );
        write_task(
            dir.path(),
            "b.md",
            "id: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: []\n",
        );
        write_task(
            dir.path(),
            "empty.md",
            "id: M-empty\ntitle: empty cut\nphase: 1\nstatus: todo\nblocked_by: []\ntags: [milestone, empty]\n",
        );
        let context = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );

        let catalogue =
            dependency_catalogue(&context, "A-1", EdgeMode::Preference).expect("catalogue");
        assert!(
            catalogue
                .choices
                .iter()
                .any(|choice| choice.task.id == "A-2" && choice.current)
        );
        let removed = preview_planning(
            &context,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Remove,
                mode: EdgeMode::Preference,
            }),
        )
        .expect("preference removal");
        assert!(matches!(
            removed.change_set().effects(),
            [SemanticEffect::PreferenceRemoved { task, after }]
                if task == "A-1" && after == "A-2"
        ));

        let ordinary = preview_planning(
            &context,
            &PlanningRequest::Tag(TagRequest::Remove {
                id: "M-empty".to_string(),
                tag: "milestone".to_string(),
            }),
        )
        .expect("an empty cut can become an ordinary task");
        assert!(matches!(
            ordinary.change_set().effects(),
            [SemanticEffect::TagRemoved { task, tag }]
                if task == "M-empty" && tag == "milestone"
        ));

        write_task(
            dir.path(),
            "duplicate.md",
            "id: A-1\ntitle: duplicate\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: []\n",
        );
        let ambiguous = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let refusal = dependency_catalogue(&ambiguous, "A-1", EdgeMode::Blocking)
            .expect_err("an ambiguous id is refused before candidates are offered");
        assert!(matches!(refusal, PlanningError::Refused(_)));

        std::fs::write(dir.path().join("bad.md"), "---\nid: [\n---\n")
            .expect("write malformed task");
        let unclean = load_context(
            dir.path(),
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let refusal = tag_catalogue(&unclean, None)
            .expect_err("an unclean plan is refused before tags are offered");
        assert!(matches!(refusal, PlanningError::Plan(_)));
    }
}
