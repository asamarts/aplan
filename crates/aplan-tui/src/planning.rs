//! Guided forms for dependencies, preferences, tags, cut/phase moves and drops.
//!
//! The application layer owns candidate context, validation and exact file
//! deltas. This module only defines form vocabulary, decodes complete input and
//! projects the typed review without parsing command output.

use aplan_app::{
    DependencyOperation, EdgeMode, PlanContext, PlanningDependencyRequest, PlanningPreview,
    PlanningRequest, PlanningReview,
};

use crate::form::{DefinitionError, FieldSpec, FormSubmission, MutationForm};
use crate::text;

/// Every native planning operation delivered by this increment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlanningWorkflow {
    DependencyAdd,
    DependencyRemove,
    PreferenceAdd,
    PreferenceRemove,
    TagAdd,
    TagRemove,
    TagRename,
    Move,
    DropTasks,
    DropCuts,
}

impl PlanningWorkflow {
    pub const ALL: &'static [Self] = &[
        Self::DependencyAdd,
        Self::DependencyRemove,
        Self::PreferenceAdd,
        Self::PreferenceRemove,
        Self::TagAdd,
        Self::TagRemove,
        Self::TagRename,
        Self::Move,
        Self::DropTasks,
        Self::DropCuts,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::DependencyAdd => "Add blocking dependency",
            Self::DependencyRemove => "Remove blocking dependency",
            Self::PreferenceAdd => "Add ordering preference",
            Self::PreferenceRemove => "Remove ordering preference",
            Self::TagAdd => "Add task tag",
            Self::TagRemove => "Remove task tag",
            Self::TagRename => "Rename tag across plan",
            Self::Move => "Move tasks",
            Self::DropTasks => "Drop tasks",
            Self::DropCuts => "Drop cuts and live members",
        }
    }

    pub const fn destructive(self) -> bool {
        matches!(
            self,
            Self::DependencyRemove
                | Self::PreferenceRemove
                | Self::TagRemove
                | Self::TagRename
                | Self::Move
                | Self::DropTasks
                | Self::DropCuts
        )
    }

    const fn edge(self) -> Option<(EdgeMode, DependencyOperation)> {
        match self {
            Self::DependencyAdd => Some((EdgeMode::Blocking, DependencyOperation::Add)),
            Self::DependencyRemove => Some((EdgeMode::Blocking, DependencyOperation::Remove)),
            Self::PreferenceAdd => Some((EdgeMode::Preference, DependencyOperation::Add)),
            Self::PreferenceRemove => Some((EdgeMode::Preference, DependencyOperation::Remove)),
            _ => None,
        }
    }
}

/// Construct one workflow form against an immutable application snapshot.
pub fn form(
    context: &PlanContext,
    workflow: PlanningWorkflow,
    selected_task: Option<&str>,
) -> Result<MutationForm, FormError> {
    let form = MutationForm::new(
        workflow.title(),
        &context.identity().sha256,
        workflow.destructive(),
        fields(workflow, selected_task),
    )?;
    Ok(form.with_guidance(form_guidance(context, workflow, selected_task)?))
}

fn form_guidance(
    context: &PlanContext,
    workflow: PlanningWorkflow,
    selected_task: Option<&str>,
) -> Result<Vec<String>, FormError> {
    if workflow.edge().is_some() {
        let Some(task) = selected_task else {
            return Ok(vec![
                "Select a loaded task to enumerate edge candidates.".to_string(),
            ]);
        };
        let catalogue = dependency_catalogue(context, task, workflow)?;
        let mut rows = vec![format!(
            "Selected: {} [{}] cut(s): {} - {}",
            catalogue.selected.id,
            catalogue.selected.status,
            display_list(&catalogue.selected.cuts),
            catalogue.selected.title
        )];
        rows.push(format!(
            "Candidates ({}; selected task excluded)",
            catalogue.choices.len()
        ));
        rows.extend(catalogue.choices.iter().map(|choice| {
            format!(
                "  {} [{}] cut(s): {} - {}; current {:?} edge: {}",
                choice.task.id,
                choice.task.status,
                display_list(&choice.task.cuts),
                choice.task.title,
                catalogue.mode,
                if choice.current { "yes" } else { "no" }
            )
        }));
        return Ok(rows);
    }
    if matches!(
        workflow,
        PlanningWorkflow::TagAdd | PlanningWorkflow::TagRemove | PlanningWorkflow::TagRename
    ) {
        let catalogue = tag_catalogue(context, selected_task)?;
        let mut rows = Vec::new();
        if let Some(selected) = &catalogue.selected {
            rows.push(format!(
                "Selected: {} [{}] cut(s): {} - {}",
                selected.id,
                selected.status,
                display_list(&selected.cuts),
                selected.title
            ));
        }
        rows.push(format!(
            "Declared tag families ({})",
            catalogue.families.len()
        ));
        rows.extend(catalogue.families.iter().map(|family| {
            let milestones = match family.milestones {
                aplan_core::MilestoneScope::Include => "including milestones",
                aplan_core::MilestoneScope::Exclude => "excluding milestones",
                aplan_core::MilestoneScope::Only => "milestones only",
            };
            format!(
                "  {}: {}; status scope {}; {}; members [{}]; selected carries [{}]",
                family.name,
                family.require.as_str(),
                family.status.as_str(),
                milestones,
                display_list(&family.members),
                display_list(&family.carried)
            )
        }));
        rows.push(format!(
            "Observed arbitrary/reserved tags: {}",
            display_list(&catalogue.arbitrary)
        ));
        return Ok(rows);
    }
    Ok(Vec::new())
}

fn fields(workflow: PlanningWorkflow, selected_task: Option<&str>) -> Vec<FieldSpec> {
    if workflow.edge().is_some() {
        return vec![
            selected(
                "task",
                "Task",
                "Selected task; candidate context includes status and cut",
                selected_task,
            )
            .required(),
            FieldSpec::new(
                "after",
                "After",
                "Another task; self-selection is excluded from the candidate catalogue",
            )
            .required(),
        ];
    }
    match workflow {
        PlanningWorkflow::TagAdd | PlanningWorkflow::TagRemove => vec![
            selected(
                "task",
                "Task",
                "Task to classify; declared families and arbitrary tags remain visible",
                selected_task,
            )
            .required(),
            FieldSpec::new("tag", "Tag", "Declared family member or arbitrary tag").required(),
        ],
        PlanningWorkflow::TagRename => vec![
            FieldSpec::new("from", "Current tag", "Ordinary tag to replace everywhere")
                .required(),
            FieldSpec::new("to", "New tag", "Replacement shown against every affected task")
                .required(),
        ],
        PlanningWorkflow::Move => vec![
            selected(
                "tasks",
                "Tasks",
                "One atomic move; repeat for every task",
                selected_task,
            )
            .required()
            .repeated(),
            FieldSpec::new("cut", "Destination cut", "Milestone id or displayed cut label"),
            FieldSpec::new("phase", "Destination phase", "Optional unsigned reporting phase"),
        ],
        PlanningWorkflow::DropTasks => vec![
            selected(
                "tasks",
                "Tasks",
                "Every selected live task will become dropped",
                selected_task,
            )
            .required()
            .repeated(),
        ],
        PlanningWorkflow::DropCuts => vec![
            FieldSpec::new(
                "cuts",
                "Cuts",
                "Milestone ids or labels; live members and retained terminal members are enumerated",
            )
            .required()
            .repeated(),
        ],
        PlanningWorkflow::DependencyAdd
        | PlanningWorkflow::DependencyRemove
        | PlanningWorkflow::PreferenceAdd
        | PlanningWorkflow::PreferenceRemove => unreachable!("edge forms returned above"),
    }
}

fn selected(key: &str, label: &str, help: &str, value: Option<&str>) -> FieldSpec {
    value.map_or_else(
        || FieldSpec::new(key, label, help),
        |value| FieldSpec::new(key, label, help).initial_value(value),
    )
}

/// Decode a complete form submission into the typed application request.
pub fn request(
    workflow: PlanningWorkflow,
    submission: &FormSubmission,
) -> Result<PlanningRequest, InputError> {
    if let Some((mode, operation)) = workflow.edge() {
        return Ok(PlanningRequest::Dependency(PlanningDependencyRequest {
            id: required(submission, "task")?,
            after: required(submission, "after")?,
            operation,
            mode,
        }));
    }
    match workflow {
        PlanningWorkflow::TagAdd => Ok(PlanningRequest::Tag(aplan_app::TagRequest::Add {
            id: required(submission, "task")?,
            tag: required(submission, "tag")?,
        })),
        PlanningWorkflow::TagRemove => Ok(PlanningRequest::Tag(aplan_app::TagRequest::Remove {
            id: required(submission, "task")?,
            tag: required(submission, "tag")?,
        })),
        PlanningWorkflow::TagRename => Ok(PlanningRequest::Tag(aplan_app::TagRequest::Rename {
            from: required(submission, "from")?,
            to: required(submission, "to")?,
        })),
        PlanningWorkflow::Move => {
            let cut = optional(submission, "cut");
            let phase = optional_number(submission, "phase")?;
            if cut.is_none() && phase.is_none() {
                return Err(InputError::Invalid {
                    field: "cut",
                    message: "enter a destination cut, phase, or both".to_string(),
                });
            }
            Ok(PlanningRequest::Move(aplan_app::MoveRequest {
                ids: repeated(submission, "tasks"),
                cut,
                phase,
            }))
        }
        PlanningWorkflow::DropTasks => Ok(PlanningRequest::Drop(aplan_app::DropRequest {
            ids: repeated(submission, "tasks"),
            by_cut: false,
        })),
        PlanningWorkflow::DropCuts => Ok(PlanningRequest::Drop(aplan_app::DropRequest {
            ids: repeated(submission, "cuts"),
            by_cut: true,
        })),
        PlanningWorkflow::DependencyAdd
        | PlanningWorkflow::DependencyRemove
        | PlanningWorkflow::PreferenceAdd
        | PlanningWorkflow::PreferenceRemove => unreachable!("edge request returned above"),
    }
}

/// Build the domain preview for one decoded request.
pub fn preview(
    context: &PlanContext,
    request: &PlanningRequest,
) -> Result<PlanningPreview, aplan_app::PlanningError> {
    aplan_app::preview_planning(context, request)
}

/// Candidate context for an edge form, obtained from the same application
/// snapshot as the form itself.
pub fn dependency_catalogue(
    context: &PlanContext,
    task: &str,
    workflow: PlanningWorkflow,
) -> Result<aplan_app::DependencyCatalogue, CatalogueError> {
    let Some((mode, _)) = workflow.edge() else {
        return Err(CatalogueError::NotAnEdge(workflow));
    };
    aplan_app::dependency_catalogue(context, task, mode).map_err(CatalogueError::Planning)
}

/// Declared and observed tag vocabulary for a single-task operation or broad
/// rename.
pub fn tag_catalogue(
    context: &PlanContext,
    selected_task: Option<&str>,
) -> Result<aplan_app::TagCatalogue, aplan_app::PlanningError> {
    aplan_app::tag_catalogue(context, selected_task)
}

/// Review rows supplementing the generic exact `ChangeSet` projection.
#[must_use]
pub fn review_rows(preview: &PlanningPreview) -> Vec<String> {
    let change = preview.change_set();
    let mut rows = vec![format!("Preview input: {}", change.input().sha256)];
    rows.push(format!("Files ({})", change.files().len()));
    for file in change.files() {
        let path = file
            .path()
            .strip_prefix(&change.input().plan_root)
            .unwrap_or(file.path());
        rows.push(format!("  {}", path.display()));
    }
    append_operation_review(&mut rows, preview.review());
    let readiness = preview.readiness();
    rows.push(format!("Ready before: {}", display_list(&readiness.before)));
    rows.push(format!("Ready after: {}", display_list(&readiness.after)));
    rows.push(format!(
        "Becomes ready: {}",
        display_list(&readiness.became_ready)
    ));
    rows.push(format!(
        "Becomes unready: {}",
        display_list(&readiness.became_unready)
    ));
    rows.into_iter().map(|row| text::safe(&row)).collect()
}

fn append_operation_review(rows: &mut Vec<String>, review: &PlanningReview) {
    match review {
        PlanningReview::Dependency(review) => {
            rows.push(format!(
                "Edge: {:?} {:?}: {} after {}",
                review.mode,
                review.operation,
                review.target.id,
                review
                    .prerequisite
                    .as_ref()
                    .map_or("<missing>", |task| task.id.as_str())
            ));
            append_task(rows, "Target", &review.target);
            if let Some(prerequisite) = &review.prerequisite {
                append_task(rows, "After", prerequisite);
            }
        }
        PlanningReview::Tag(review) => {
            rows.push(format!("Affected tasks ({})", review.affected.len()));
            for task in &review.affected {
                append_task(rows, "  task", task);
            }
            rows.push(format!(
                "Declared families ({})",
                review.catalogue.families.len()
            ));
            for family in &review.catalogue.families {
                rows.push(format!(
                    "  {}: {} [{}]; carried: {}",
                    family.name,
                    family.require.as_str(),
                    family.members.join(", "),
                    display_list(&family.carried)
                ));
            }
            rows.push(format!(
                "Arbitrary/reserved observed tags: {}",
                display_list(&review.catalogue.arbitrary)
            ));
        }
        PlanningReview::Move(review) => {
            rows.push(format!("Task moves ({})", review.transitions.len()));
            for transition in &review.transitions {
                rows.push(format!(
                    "  {}: cuts [{}] -> {}; phase {} -> {}",
                    transition.task.id,
                    display_list(&transition.source_cuts),
                    transition.destination_cut.as_deref().unwrap_or("unchanged"),
                    transition.phase_before,
                    transition.phase_after
                ));
            }
            rows.push(format!(
                "Milestone files ({})",
                review.milestone_files.len()
            ));
            rows.extend(
                review
                    .milestone_files
                    .iter()
                    .map(|path| format!("  {}", path.display())),
            );
        }
        PlanningReview::Drop(review) => {
            rows.push(format!("Requested cuts ({})", review.requested_cuts.len()));
            for task in &review.requested_cuts {
                append_task(rows, "  cut", task);
            }
            rows.push(format!(
                "Tasks becoming dropped ({})",
                review.affected.len()
            ));
            for task in &review.affected {
                append_task(rows, "  drop", task);
            }
            rows.push(format!(
                "Terminal members retained ({})",
                review.retained_terminal_members.len()
            ));
            for task in &review.retained_terminal_members {
                append_task(rows, "  retain", task);
            }
        }
    }
}

fn append_task(rows: &mut Vec<String>, label: &str, task: &aplan_app::TaskChoice) {
    rows.push(format!(
        "{label}: {} [{}] cut(s): {} - {}",
        task.id,
        task.status,
        display_list(&task.cuts),
        task.title
    ));
}

fn display_list(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

fn required(submission: &FormSubmission, field: &'static str) -> Result<String, InputError> {
    optional(submission, field).ok_or(InputError::Required(field))
}

fn optional(submission: &FormSubmission, field: &str) -> Option<String> {
    submission
        .value(field)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn repeated(submission: &FormSubmission, field: &str) -> Vec<String> {
    submission
        .values(field)
        .into_iter()
        .flatten()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn optional_number<T>(
    submission: &FormSubmission,
    field: &'static str,
) -> Result<Option<T>, InputError>
where
    T: std::str::FromStr,
{
    optional(submission, field)
        .map(|value| {
            value.parse().map_err(|_| InputError::Invalid {
                field,
                message: "not a valid number".to_string(),
            })
        })
        .transpose()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error("field `{0}` is required")]
    Required(&'static str),
    #[error("field `{field}` is invalid: {message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    #[error("{0:?} is not an edge workflow")]
    NotAnEdge(PlanningWorkflow),
    #[error(transparent)]
    Planning(#[from] aplan_app::PlanningError),
}

#[derive(Debug, thiserror::Error)]
pub enum FormError {
    #[error(transparent)]
    Definition(#[from] DefinitionError),
    #[error(transparent)]
    Catalogue(#[from] CatalogueError),
    #[error(transparent)]
    Planning(#[from] aplan_app::PlanningError),
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use aplan_app::{ChangeDisposition, ContextInputs, Trust};

    use super::*;
    use crate::form::{FormAction, FormFocus, FormTransition};

    fn write_task(root: &Path, name: &str, frontmatter: &str) {
        std::fs::write(root.join(name), format!("---\n{frontmatter}---\n")).expect("write task");
    }

    fn context(root: &Path) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(root),
            ContextInputs {
                repository_root: root.to_path_buf(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
    }

    fn fixture(root: &Path) {
        write_task(
            root,
            "a.md",
            "id: A-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: [custom]\n",
        );
        write_task(
            root,
            "b.md",
            "id: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: []\n",
        );
        write_task(
            root,
            "m.md",
            "id: M-1\ntitle: cut\nphase: 1\nstatus: todo\nblocked_by: [A-1, A-2]\ntags: [milestone, v1]\n",
        );
    }

    fn submission(mut form: MutationForm, edits: &[(&str, &str)]) -> FormSubmission {
        for (key, value) in edits {
            let index = form
                .fields()
                .iter()
                .position(|field| field.spec().key() == *key)
                .expect("field");
            while form.focus() != FormFocus::Field(index) {
                let _ = form.reduce(FormAction::NextFocus);
            }
            let _ = form.reduce(FormAction::Insert((*value).to_string()));
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("complete form")
        };
        submission
    }

    #[test]
    fn every_workflow_has_a_complete_distinct_form_and_typed_request() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        fixture(dir.path());
        let context = context(dir.path());
        for workflow in PlanningWorkflow::ALL {
            let form = form(&context, *workflow, Some("A-1")).expect("form");
            assert_eq!(form.title(), workflow.title());
            assert_eq!(form.is_destructive(), workflow.destructive());
        }

        for workflow in [
            PlanningWorkflow::DependencyAdd,
            PlanningWorkflow::DependencyRemove,
            PlanningWorkflow::PreferenceAdd,
            PlanningWorkflow::PreferenceRemove,
        ] {
            let submission = submission(
                form(&context, workflow, Some("A-1")).expect("edge form"),
                &[("after", "A-2")],
            );
            let PlanningRequest::Dependency(request) =
                request(workflow, &submission).expect("edge")
            else {
                panic!("dependency request")
            };
            assert_eq!(request.id, "A-1");
            assert_eq!(request.after, "A-2");
            assert_eq!(request.mode, workflow.edge().expect("edge").0);
            assert_eq!(request.operation, workflow.edge().expect("edge").1);
        }

        let move_submission = submission(
            form(&context, PlanningWorkflow::Move, Some("A-1")).expect("move form"),
            &[("cut", "v1"), ("phase", "2")],
        );
        assert!(matches!(
            request(PlanningWorkflow::Move, &move_submission),
            Ok(PlanningRequest::Move(aplan_app::MoveRequest {
                phase: Some(2),
                ..
            }))
        ));
        let invalid_move = submission(
            form(&context, PlanningWorkflow::Move, Some("A-1")).expect("move form"),
            &[],
        );
        assert!(matches!(
            request(PlanningWorkflow::Move, &invalid_move),
            Err(InputError::Invalid { .. })
        ));
    }

    /// Proof for `tui.planning_forms_render_typed_choice_context`.
    #[test]
    fn catalogues_and_review_rows_expose_context_and_readiness_without_cli_text() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        fixture(dir.path());
        let context = context(dir.path());
        let catalogue = dependency_catalogue(&context, "A-1", PlanningWorkflow::DependencyAdd)
            .expect("catalogue");
        assert!(
            catalogue
                .choices
                .iter()
                .all(|choice| choice.task.id != "A-1")
        );
        assert!(
            catalogue
                .choices
                .iter()
                .any(|choice| choice.task.id == "A-2" && choice.task.cuts == ["M-1"])
        );
        let edge_rows = form(&context, PlanningWorkflow::DependencyAdd, Some("A-1"))
            .expect("edge form")
            .render(200, 200)
            .rows
            .join("\n");
        assert!(edge_rows.contains("Candidates (2; selected task excluded)"));
        assert!(edge_rows.contains("A-2 [todo] cut(s): M-1 - second"));
        assert!(!edge_rows.contains("  A-1 ["), "selected task was offered");
        assert!(matches!(
            dependency_catalogue(&context, "A-1", PlanningWorkflow::Move),
            Err(CatalogueError::NotAnEdge(PlanningWorkflow::Move))
        ));

        let preview = preview(
            &context,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
        )
        .expect("preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        let rows = review_rows(&preview);
        assert!(
            rows.iter()
                .any(|row| row.contains("Target: A-1 [todo] cut(s): M-1"))
        );
        assert!(rows.iter().any(|row| row == "Becomes unready: A-1"));
        assert!(rows.iter().any(|row| row.contains("a.md")));
    }

    #[test]
    fn tag_move_and_drop_forms_preserve_repeated_scope_and_selected_defaults() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        fixture(dir.path());
        let context = context(dir.path());

        let tag = submission(
            form(&context, PlanningWorkflow::TagAdd, Some("A-1")).expect("tag form"),
            &[("tag", "idea")],
        );
        assert_eq!(
            request(PlanningWorkflow::TagAdd, &tag).expect("tag"),
            PlanningRequest::Tag(aplan_app::TagRequest::Add {
                id: "A-1".to_string(),
                tag: "idea".to_string(),
            })
        );

        let drop = submission(
            form(&context, PlanningWorkflow::DropCuts, None).expect("drop form"),
            &[("cuts", "v1")],
        );
        assert_eq!(
            request(PlanningWorkflow::DropCuts, &drop).expect("drop"),
            PlanningRequest::Drop(aplan_app::DropRequest {
                ids: vec!["v1".to_string()],
                by_cut: true,
            })
        );
        let tags = tag_catalogue(&context, Some("A-1")).expect("tags");
        assert!(tags.arbitrary.contains(&"custom".to_string()));
        let tag_rows = form(&context, PlanningWorkflow::TagAdd, Some("A-1"))
            .expect("tag form")
            .render(200, 200)
            .rows
            .join("\n");
        assert!(tag_rows.contains("Declared tag families"));
        assert!(tag_rows.contains("Observed arbitrary/reserved tags: custom"));
    }

    #[test]
    fn every_non_edge_form_decodes_its_distinct_request_and_rejects_bad_numbers() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        fixture(dir.path());
        let context = context(dir.path());

        let remove = submission(
            form(&context, PlanningWorkflow::TagRemove, Some("A-1")).expect("remove form"),
            &[("tag", "custom")],
        );
        assert_eq!(
            request(PlanningWorkflow::TagRemove, &remove).expect("remove request"),
            PlanningRequest::Tag(aplan_app::TagRequest::Remove {
                id: "A-1".to_string(),
                tag: "custom".to_string(),
            })
        );

        let rename = submission(
            form(&context, PlanningWorkflow::TagRename, None).expect("rename form"),
            &[("from", "custom"), ("to", "idea")],
        );
        assert_eq!(
            request(PlanningWorkflow::TagRename, &rename).expect("rename request"),
            PlanningRequest::Tag(aplan_app::TagRequest::Rename {
                from: "custom".to_string(),
                to: "idea".to_string(),
            })
        );

        let drop = submission(
            form(&context, PlanningWorkflow::DropTasks, Some("A-2")).expect("drop form"),
            &[],
        );
        assert_eq!(
            request(PlanningWorkflow::DropTasks, &drop).expect("drop request"),
            PlanningRequest::Drop(aplan_app::DropRequest {
                ids: vec!["A-2".to_string()],
                by_cut: false,
            })
        );

        let bad_phase = submission(
            form(&context, PlanningWorkflow::Move, Some("A-1")).expect("move form"),
            &[("phase", "not-a-number")],
        );
        assert!(matches!(
            request(PlanningWorkflow::Move, &bad_phase),
            Err(InputError::Invalid { field: "phase", .. })
        ));
    }

    #[test]
    fn review_rows_cover_missing_edges_tags_moves_and_cut_drops_without_controls() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        fixture(dir.path());
        let context = context(dir.path());

        let missing = preview(
            &context,
            &PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-1".to_string(),
                after: "missing".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
        )
        .expect("missing endpoint remains a denied preview");
        assert!(
            review_rows(&missing)
                .iter()
                .any(|row| row == "Edge: Blocking Add: A-1 after <missing>")
        );

        let tag = preview(
            &context,
            &PlanningRequest::Tag(aplan_app::TagRequest::Add {
                id: "A-2".to_string(),
                tag: "idea".to_string(),
            }),
        )
        .expect("tag preview");
        let tag_rows = review_rows(&tag);
        assert!(tag_rows.iter().any(|row| row == "Affected tasks (1)"));
        assert!(
            tag_rows
                .iter()
                .any(|row| row.contains("Arbitrary/reserved observed tags"))
        );

        let moved = preview(
            &context,
            &PlanningRequest::Move(aplan_app::MoveRequest {
                ids: vec!["A-1".to_string()],
                cut: None,
                phase: Some(2),
            }),
        )
        .expect("move preview");
        let move_rows = review_rows(&moved);
        assert!(
            move_rows
                .iter()
                .any(|row| row == "  A-1: cuts [M-1] -> unchanged; phase 1 -> 2")
        );
        assert!(move_rows.iter().any(|row| row == "Milestone files (0)"));

        let dropped = preview(
            &context,
            &PlanningRequest::Drop(aplan_app::DropRequest {
                ids: vec!["M-1".to_string()],
                by_cut: true,
            }),
        )
        .expect("cut drop preview");
        let drop_rows = review_rows(&dropped);
        assert!(drop_rows.iter().any(|row| row == "Requested cuts (1)"));
        assert!(
            drop_rows
                .iter()
                .any(|row| row == "Tasks becoming dropped (3)")
        );
        assert!(
            drop_rows
                .iter()
                .any(|row| row == "Terminal members retained (0)")
        );
        assert!(drop_rows.iter().any(|row| row.contains("  cut: M-1")));
        assert!(drop_rows.iter().any(|row| row.contains("  drop: A-1")));
    }
}
