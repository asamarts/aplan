//! Native capture and inbox forms.
//!
//! This module owns only interactive vocabulary and decoding. Capture
//! lifecycle, policy, task construction, preview identity and guarded writes
//! remain in `aplan-app`.

use aplan_app::{
    CaptureAmendRequest, CaptureDiscardRequest, CaptureMutationPreview, CapturePromotionPreview,
    CapturePromotionRequest, CaptureSelector, ExactCaptureId, PlanContext,
};
use aplan_core::capture::{
    Capture, CaptureCreate, CaptureOrigin, CaptureState, PromotionDefaults, SystemClock,
};

use crate::form::{FieldSpec, FormSubmission, MutationForm};

/// Every native capture mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CaptureWorkflow {
    Create,
    Amend,
    Discard,
    Restore,
    Promote,
}

impl CaptureWorkflow {
    pub const ALL: [Self; 5] = [
        Self::Create,
        Self::Amend,
        Self::Discard,
        Self::Restore,
        Self::Promote,
    ];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Create => "Quick capture",
            Self::Amend => "Amend capture",
            Self::Discard => "Discard capture",
            Self::Restore => "Restore capture",
            Self::Promote => "Promote capture to task",
        }
    }

    #[must_use]
    pub const fn needs_capture(self) -> bool {
        !matches!(self, Self::Create)
    }
}

/// A typed preview together with review context the generic `ChangeSet`
/// projection cannot infer.
#[derive(Debug)]
pub enum CapturePreview {
    Mutation(CaptureMutationPreview),
    Promotion(CapturePromotionPreview),
}

impl CapturePreview {
    #[must_use]
    pub fn capture(&self) -> &Capture {
        match self {
            Self::Mutation(preview) => preview.capture(),
            Self::Promotion(preview) => preview.capture(),
        }
    }

    #[must_use]
    pub fn task_id(&self) -> Option<&str> {
        match self {
            Self::Mutation(_) => None,
            Self::Promotion(preview) => Some(preview.task_id()),
        }
    }

    #[must_use]
    pub fn idempotent_retry(&self) -> bool {
        match self {
            Self::Mutation(preview) => preview.idempotent_retry(),
            Self::Promotion(preview) => preview.idempotent_retry(),
        }
    }

    #[must_use]
    pub fn into_change_set(self) -> aplan_app::ChangeSet {
        match self {
            Self::Mutation(preview) => preview.into_change_set(),
            Self::Promotion(preview) => preview.into_change_set(),
        }
    }
}

/// Build one form against the current identified input.
pub fn form(
    context: &PlanContext,
    workflow: CaptureWorkflow,
    selected_capture: Option<&str>,
) -> Result<MutationForm, String> {
    let capture = if workflow.needs_capture() {
        Some(read_selected(context, selected_capture)?)
    } else {
        None
    };
    let fields = match workflow {
        CaptureWorkflow::Create => create_fields(),
        CaptureWorkflow::Amend => amend_fields(capture.as_ref().expect("capture loaded")),
        CaptureWorkflow::Discard => vec![
            capture_id(capture.as_ref().expect("capture loaded")),
            FieldSpec::new(
                "reason",
                "Reason",
                "Optional retained disposition explanation",
            ),
        ],
        CaptureWorkflow::Restore => {
            vec![capture_id(capture.as_ref().expect("capture loaded"))]
        }
        CaptureWorkflow::Promote => promotion_fields(capture.as_ref().expect("capture loaded")),
    };
    let mut guidance = Vec::new();
    if let Some(capture) = &capture {
        guidance.push(format!(
            "Selected capture: {} [{}; {}; origin {:?}]",
            capture.id,
            state_name(capture.state),
            capture.kind,
            capture.origin
        ));
    }
    if matches!(workflow, CaptureWorkflow::Create | CaptureWorkflow::Amend) {
        let policy = context
            .report_config()
            .ok_or_else(|| "capture forms require resolved repository configuration".to_string())?
            .capture_policy();
        guidance.push(format!(
            "Kinds: {}. Profiles are explicit defaults, not lifecycle policy.",
            policy
                .kinds()
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
        guidance.push(if policy.profiles().is_empty() {
            "Profiles: none configured.".to_string()
        } else {
            format!(
                "Profiles: {}.",
                policy
                    .profiles()
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    }
    if workflow == CaptureWorkflow::Promote {
        guidance.push(
            "Title and body come from the retained capture. Placement fields reuse the ordinary Add form vocabulary; preview is one atomic task-and-capture ChangeSet."
                .to_string(),
        );
    }
    let form = MutationForm::new(
        workflow.title(),
        &context.identity().sha256,
        matches!(
            workflow,
            CaptureWorkflow::Discard | CaptureWorkflow::Promote
        ),
        fields,
    )
    .map_err(|error| error.to_string())?
    .with_guidance(guidance);
    let form = if let Some(capture) = &capture {
        form.with_subject(capture.id.clone(), aplan_app::capture_fingerprint(capture))
    } else {
        form
    };
    Ok(form)
}

fn create_fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("title", "Title", "Required short intake title").required(),
        FieldSpec::new(
            "kind",
            "Kind",
            "Built-in or configured kind; blank means task",
        ),
        FieldSpec::new(
            "profile",
            "Profile",
            "Optional configured repository profile",
        ),
        FieldSpec::new("body", "Body", "Optional short detail; paste is supported"),
        FieldSpec::new("labels", "Labels", "Sorted intake labels, not task tags").repeated(),
        FieldSpec::new(
            "revisit_at",
            "Revisit at",
            "Optional RFC 3339 instant for deferred review",
        ),
    ]
}

fn amend_fields(capture: &Capture) -> Vec<FieldSpec> {
    let defaults = capture.promotion_defaults.clone().unwrap_or_default();
    vec![
        capture_id(capture),
        FieldSpec::new("title", "Title", "Required short intake title")
            .required()
            .initial_value(&capture.title),
        FieldSpec::new("kind", "Kind", "Built-in or configured kind")
            .required()
            .initial_value(&capture.kind),
        option_field(
            "body",
            "Body",
            "Blank clears the optional body",
            capture.body.as_deref(),
        ),
        FieldSpec::new("labels", "Labels", "Replacement intake labels")
            .repeated()
            .initial_values(capture.labels.iter()),
        option_field(
            "revisit_at",
            "Revisit at",
            "Blank makes the capture actionable immediately",
            capture.revisit_at.as_deref(),
        ),
        option_field(
            "phase",
            "Promotion phase",
            "Blank clears this snapshotted default",
            defaults.phase.map(|value| value.to_string()).as_deref(),
        ),
        option_field(
            "cut",
            "Promotion cut",
            "Blank clears this snapshotted default",
            defaults.cut.as_deref(),
        ),
        option_field(
            "effort",
            "Promotion effort",
            "Blank clears this snapshotted default",
            defaults
                .effort_days
                .map(|value| value.to_string())
                .as_deref(),
        ),
        option_field(
            "owner",
            "Promotion owner",
            "Blank clears this snapshotted default",
            defaults.owner.as_deref(),
        ),
        FieldSpec::new(
            "paths",
            "Promotion paths",
            "Replacement snapshotted task paths",
        )
        .repeated()
        .initial_values(defaults.paths.iter()),
        FieldSpec::new(
            "tags",
            "Promotion tags",
            "Replacement snapshotted task tags",
        )
        .repeated()
        .initial_values(defaults.tags.iter()),
    ]
}

fn promotion_fields(capture: &Capture) -> Vec<FieldSpec> {
    let defaults = capture.promotion_defaults.clone().unwrap_or_default();
    let admitted = ["id", "phase", "cut", "effort", "owner", "tags", "paths"];
    let mut fields = vec![capture_id(capture)];
    fields.extend(
        crate::authoring::add_fields()
            .into_iter()
            .filter(|field| admitted.contains(&field.key())),
    );
    fields.push(FieldSpec::new(
        "clear_paths",
        "Clear default paths",
        "Type yes to replace snapshotted paths with none",
    ));
    fields.push(FieldSpec::new(
        "clear_tags",
        "Clear default tags",
        "Type yes to replace snapshotted tags with none",
    ));
    fields.push(FieldSpec::new(
        "reason",
        "Reason",
        "Optional explanation retained on the promoted capture",
    ));
    for field in &mut fields {
        *field = match field.key() {
            "phase" => initial_option(field.clone(), defaults.phase.map(|value| value.to_string())),
            "cut" => initial_option(field.clone(), defaults.cut.clone()),
            "effort" => initial_option(
                field.clone(),
                defaults.effort_days.map(|value| value.to_string()),
            ),
            "owner" => initial_option(field.clone(), defaults.owner.clone()),
            "paths" => field.clone().initial_values(defaults.paths.iter()),
            "tags" => field.clone().initial_values(defaults.tags.iter()),
            _ => field.clone(),
        };
    }
    fields
}

fn initial_option(field: FieldSpec, value: Option<String>) -> FieldSpec {
    value.map_or(field.clone(), |value| field.initial_value(value))
}

fn capture_id(capture: &Capture) -> FieldSpec {
    FieldSpec::new("capture", "Capture", "Stable full capture id")
        .required()
        .initial_value(&capture.id)
}

fn option_field(key: &str, label: &str, help: &str, value: Option<&str>) -> FieldSpec {
    value.map_or_else(
        || FieldSpec::new(key, label, help),
        |value| FieldSpec::new(key, label, help).initial_value(value),
    )
}

fn read_selected(context: &PlanContext, selected: Option<&str>) -> Result<Capture, String> {
    let id = selected.ok_or_else(|| "select a capture in Inbox first".to_string())?;
    let selector = exact_selector(id)?;
    aplan_app::show_capture(context, &selector, &SystemClock)
        .map(|read| read.capture)
        .map_err(|error| error.to_string())
}

/// Decode and preview one form through the shared application boundary.
pub fn preview(
    context: &PlanContext,
    workflow: CaptureWorkflow,
    submission: &FormSubmission,
) -> Result<CapturePreview, String> {
    let clock = SystemClock;
    let selected = if workflow.needs_capture() {
        Some(bound_selector(context, submission, &clock)?)
    } else {
        None
    };
    match workflow {
        CaptureWorkflow::Create => {
            let labels = values(submission, "labels");
            let request = CaptureCreate {
                title: required(submission, "title")?,
                kind: optional(submission, "kind"),
                body: optional(submission, "body"),
                labels: (!labels.is_empty()).then_some(labels),
                revisit_at: optional(submission, "revisit_at"),
                profile: optional(submission, "profile"),
                promotion_defaults: None,
                source: None,
                origin: CaptureOrigin::Tui,
            };
            aplan_app::preview_capture_create(context, &request, &clock)
                .map(CapturePreview::Mutation)
                .map_err(|error| error.to_string())
        }
        CaptureWorkflow::Amend => {
            let defaults = promotion_defaults(submission)?;
            let request = CaptureAmendRequest {
                selector: selected.clone(),
                title: Some(required(submission, "title")?),
                kind: Some(required(submission, "kind")?),
                body: Some(optional(submission, "body")),
                labels: Some(values(submission, "labels")),
                revisit_at: Some(optional(submission, "revisit_at")),
                promotion_defaults: Some((!defaults.is_empty()).then_some(defaults)),
                expected_sha256: submission.subject_sha256().map(str::to_string),
            };
            aplan_app::preview_capture_amend(context, &request, &clock)
                .map(CapturePreview::Mutation)
                .map_err(|error| error.to_string())
        }
        CaptureWorkflow::Discard => {
            let request = CaptureDiscardRequest {
                selector: selected.expect("selected workflow was bound above"),
                reason: optional(submission, "reason"),
            };
            aplan_app::preview_capture_discard(context, &request, &clock)
                .map(CapturePreview::Mutation)
                .map_err(|error| error.to_string())
        }
        CaptureWorkflow::Restore => {
            let selector = selected.expect("selected workflow was bound above");
            aplan_app::preview_capture_restore(context, &selector, &clock)
                .map(CapturePreview::Mutation)
                .map_err(|error| error.to_string())
        }
        CaptureWorkflow::Promote => {
            let clear_paths = yes(submission, "clear_paths")?;
            let clear_tags = yes(submission, "clear_tags")?;
            let paths = values(submission, "paths");
            let tags = values(submission, "tags");
            if clear_paths && !paths.is_empty() {
                return Err("field `clear_paths` cannot be yes when paths are present".to_string());
            }
            if clear_tags && !tags.is_empty() {
                return Err("field `clear_tags` cannot be yes when tags are present".to_string());
            }
            let request = CapturePromotionRequest {
                selector: selected.expect("selected workflow was bound above"),
                task_id: optional(submission, "id"),
                phase: number(submission, "phase")?,
                cut: optional(submission, "cut"),
                effort_days: number(submission, "effort")?,
                owner: optional(submission, "owner"),
                paths: if clear_paths {
                    Some(Vec::new())
                } else if paths.is_empty() {
                    None
                } else {
                    Some(paths)
                },
                tags: if clear_tags {
                    Some(Vec::new())
                } else if tags.is_empty() {
                    None
                } else {
                    Some(tags)
                },
                reason: optional(submission, "reason"),
            };
            aplan_app::preview_capture_promotion(context, &request, &clock)
                .map(CapturePreview::Promotion)
                .map_err(|error| error.to_string())
        }
    }
}

fn bound_selector(
    context: &PlanContext,
    submission: &FormSubmission,
    clock: &impl aplan_core::capture::Clock,
) -> Result<CaptureSelector, String> {
    let id = required(submission, "capture")?;
    let subject_id = submission.subject_id().ok_or_else(|| {
        "the selected capture form lost its immutable record identity; close and reopen it"
            .to_string()
    })?;
    if id != subject_id {
        return Err(format!(
            "this form was opened for capture `{subject_id}`, but its capture field now names `{id}`; close it and open the intended record from Inbox"
        ));
    }
    let selector = exact_selector(&id)?;
    let expected = submission.subject_sha256().ok_or_else(|| {
        "the selected capture form lost its immutable record binding; close and reopen it"
            .to_string()
    })?;
    let current = aplan_app::show_capture(context, &selector, clock)
        .map_err(|error| error.to_string())?
        .capture;
    if aplan_app::capture_fingerprint(&current) != expected {
        return Err(format!(
            "capture `{id}` changed after this form opened; close and reopen it before previewing a mutation"
        ));
    }
    Ok(selector)
}

#[must_use]
pub fn review_rows(preview: &CapturePreview) -> Vec<String> {
    let capture = preview.capture();
    let mut rows = vec![
        format!("Capture: {}", capture.id),
        format!("Capture state after preview: {}", state_name(capture.state)),
        format!("Origin retained: {:?}", capture.origin),
        format!(
            "Retry: {}",
            if preview.idempotent_retry() {
                "equivalent request; no new write"
            } else {
                "new guarded change"
            }
        ),
    ];
    if let Some(task) = preview.task_id() {
        rows.push(format!("Resulting task: {task}"));
        rows.push("The task and retained capture disposition publish atomically.".to_string());
    }
    rows
}

fn promotion_defaults(submission: &FormSubmission) -> Result<PromotionDefaults, String> {
    Ok(PromotionDefaults {
        phase: number(submission, "phase")?,
        cut: optional(submission, "cut"),
        effort_days: number(submission, "effort")?,
        owner: optional(submission, "owner"),
        paths: values(submission, "paths"),
        tags: values(submission, "tags"),
    })
}

fn exact_selector(id: &str) -> Result<CaptureSelector, String> {
    ExactCaptureId::new(id.to_string())
        .map(CaptureSelector::Exact)
        .map_err(|error| error.to_string())
}

fn required(submission: &FormSubmission, key: &'static str) -> Result<String, String> {
    optional(submission, key).ok_or_else(|| format!("field `{key}` is required"))
}

fn optional(submission: &FormSubmission, key: &str) -> Option<String> {
    submission
        .value(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn values(submission: &FormSubmission, key: &str) -> Vec<String> {
    submission
        .values(key)
        .into_iter()
        .flatten()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn number<T>(submission: &FormSubmission, key: &'static str) -> Result<Option<T>, String>
where
    T: std::str::FromStr,
{
    optional(submission, key)
        .map(|value| {
            value
                .parse()
                .map_err(|_| format!("field `{key}` is not a valid number"))
        })
        .transpose()
}

fn yes(submission: &FormSubmission, key: &'static str) -> Result<bool, String> {
    match optional(submission, key).as_deref() {
        None | Some("no") => Ok(false),
        Some("yes") => Ok(true),
        Some(_) => Err(format!("field `{key}` expects `yes` or `no`")),
    }
}

const fn state_name(state: CaptureState) -> &'static str {
    match state {
        CaptureState::Open => "OPEN",
        CaptureState::Promoted => "PROMOTED",
        CaptureState::Discarded => "DISCARDED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::form::{FormAction, FormFocus, FormTransition};

    const FIRST: &str = "I-01K8Z3A4H00000000000000000";
    const SECOND: &str = "I-01K8Z3A4H00000000000000001";

    struct Fixture {
        _directory: tempfile::TempDir,
        root: PathBuf,
        plan: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::Builder::new()
                .prefix("tui-inbox")
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("fixture");
            let root = directory.path().to_path_buf();
            let plan = root.join("aplan");
            std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
            std::fs::create_dir_all(root.join("src")).expect("source");
            std::fs::write(root.join("src/lib.rs"), "pub fn proof() {}\n").expect("source");
            std::fs::write(
                root.join(".aplan.yml"),
                concat!(
                    "root: aplan\n",
                    "capture:\n",
                    "  kinds: [question]\n",
                    "  profiles:\n",
                    "    triage:\n",
                    "      kind: question\n",
                    "      labels: [profiled]\n",
                    "      promotion:\n",
                    "        phase: 1\n",
                    "        effort_days: 1\n",
                    "        paths: [src/**]\n",
                    "        tags: [cross-cutting]\n",
                ),
            )
            .expect("configuration");
            Self {
                _directory: directory,
                root,
                plan,
            }
        }

        fn context(&self) -> PlanContext {
            let config = aplan_config::load_config(&self.root).expect("configuration");
            PlanContext::new(
                aplan_core::load_plan_identified(&self.plan),
                aplan_app::ContextInputs {
                    repository_root: self.root.clone(),
                    configuration_file: Some(self.root.join(".aplan.yml")),
                    trust: aplan_app::Trust::RepositoryRead,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            )
            .with_captured_report_config(config)
            .expect("captured configuration")
        }

        fn create(&self, id: &str, title: &str) -> Capture {
            let context = self.context();
            let preview = aplan_app::preview_capture_create_with_id(
                &context,
                &CaptureCreate {
                    title: title.to_string(),
                    kind: None,
                    body: Some("retained body".to_string()),
                    labels: Some(vec!["seed".to_string()]),
                    revisit_at: None,
                    profile: None,
                    promotion_defaults: None,
                    source: None,
                    origin: CaptureOrigin::Tui,
                },
                &aplan_core::capture::SystemClock,
                id,
            )
            .expect("capture preview");
            assert!(matches!(
                aplan_app::commit_change_set(&context, preview.into_change_set()),
                aplan_app::CommitOutcome::Applied(_)
            ));
            aplan_app::show_capture(
                &self.context(),
                &exact_selector(id).expect("selector"),
                &aplan_core::capture::SystemClock,
            )
            .expect("created capture")
            .capture
        }
    }

    fn replace_current(form: &mut MutationForm, values: &[&str]) {
        let FormFocus::Field(index) = form.focus() else {
            panic!("field value changed while a control was focused")
        };
        let current = form.fields()[index]
            .values()
            .next()
            .unwrap_or_default()
            .to_string();
        form.reduce(FormAction::CursorHome);
        for _ in current.chars() {
            form.reduce(FormAction::Delete);
        }
        if let Some(first) = values.first() {
            form.reduce(FormAction::Insert((*first).to_string()));
            for value in &values[1..] {
                form.reduce(FormAction::AddItem);
                form.reduce(FormAction::Insert((*value).to_string()));
            }
        }
    }

    fn submission(
        context: &PlanContext,
        workflow: CaptureWorkflow,
        selected: Option<&str>,
        values: &[(&str, &[&str])],
    ) -> FormSubmission {
        let mut form = form(context, workflow, selected).expect("capture form");
        let count = form.fields().len();
        for index in 0..count {
            assert_eq!(form.focus(), FormFocus::Field(index));
            let key = form.fields()[index].spec().key().to_string();
            if let Some((_, values)) = values.iter().find(|(candidate, _)| *candidate == key) {
                replace_current(&mut form, values);
            }
            form.reduce(FormAction::NextFocus);
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("complete capture form did not request a preview")
        };
        submission
    }

    fn commit(preview: CapturePreview, context: &PlanContext) {
        assert!(matches!(
            aplan_app::commit_change_set(context, preview.into_change_set()),
            aplan_app::CommitOutcome::Applied(_)
        ));
    }

    #[test]
    fn workflow_inventory_is_closed_and_titles_are_visible() {
        assert_eq!(CaptureWorkflow::ALL.len(), 5);
        for workflow in CaptureWorkflow::ALL {
            assert!(!workflow.title().is_empty());
            assert_eq!(
                workflow.needs_capture(),
                workflow != CaptureWorkflow::Create
            );
        }
    }

    #[test]
    fn every_capture_form_reaches_the_shared_guarded_preview_and_atomic_promotion() {
        let fixture = Fixture::new();
        let context = fixture.context();
        let create = submission(
            &context,
            CaptureWorkflow::Create,
            None,
            &[
                ("title", &["profiled intake"]),
                ("profile", &["triage"]),
                ("body", &["pasted \u{754c} detail"]),
            ],
        );
        let created = preview(&context, CaptureWorkflow::Create, &create).expect("create preview");
        assert_eq!(created.capture().origin, CaptureOrigin::Tui);
        assert_eq!(created.capture().profile.as_deref(), Some("triage"));
        assert_eq!(created.capture().kind, "question");
        commit(created, &context);

        let config = aplan_config::load_config(&fixture.root).expect("configuration");
        let loaded = aplan_core::capture::load_inbox(
            &fixture.plan,
            &config.capture_policy(),
            &aplan_core::capture::SystemClock,
        );
        assert!(loaded.is_clean(), "{:?}", loaded.diagnostics);
        let created_id =
            aplan_app::capture_workspace(&fixture.context(), &aplan_core::capture::SystemClock)
                .expect("workspace")
                .captures
                .into_iter()
                .find(|capture| capture.title == "profiled intake")
                .expect("created capture")
                .id;
        let context = fixture.context();
        let amend = submission(
            &context,
            CaptureWorkflow::Amend,
            Some(&created_id),
            &[("title", &["amended intake"]), ("labels", &["a", "b"])],
        );
        let amended = preview(&context, CaptureWorkflow::Amend, &amend).expect("amend preview");
        assert_eq!(amended.capture().title, "amended intake");
        assert_eq!(amended.capture().origin, CaptureOrigin::Tui);
        commit(amended, &context);

        fixture.create(SECOND, "promotion candidate");
        let context = fixture.context();
        let promote = submission(
            &context,
            CaptureWorkflow::Promote,
            Some(SECOND),
            &[
                ("id", &["T-9"]),
                ("phase", &["1"]),
                ("effort", &["1"]),
                ("paths", &["src/**"]),
                ("tags", &["cross-cutting"]),
                ("reason", &["triaged"]),
            ],
        );
        let promoted =
            preview(&context, CaptureWorkflow::Promote, &promote).expect("promotion preview");
        assert_eq!(promoted.task_id(), Some("T-9"));
        assert_eq!(promoted.capture().state, CaptureState::Promoted);
        let rows = review_rows(&promoted);
        assert!(rows.iter().any(|row| row.contains("publish atomically")));
        commit(promoted, &context);
        assert!(fixture.plan.join("tasks/T-9.md").is_file());
        let retained = aplan_app::show_capture(
            &fixture.context(),
            &exact_selector(SECOND).expect("selector"),
            &aplan_core::capture::SystemClock,
        )
        .expect("retained capture")
        .capture;
        assert_eq!(retained.origin, CaptureOrigin::Tui);
        assert_eq!(retained.promoted_to.as_deref(), Some("T-9"));
    }

    #[test]
    fn lifecycle_forms_refuse_the_wrong_state_and_invalid_values() {
        let fixture = Fixture::new();
        fixture.create(FIRST, "lifecycle candidate");

        let context = fixture.context();
        let restore_open = submission(&context, CaptureWorkflow::Restore, Some(FIRST), &[]);
        let error = preview(&context, CaptureWorkflow::Restore, &restore_open)
            .expect_err("open capture cannot be restored");
        assert!(error.contains("open"), "{error}");

        let invalid = submission(
            &context,
            CaptureWorkflow::Promote,
            Some(FIRST),
            &[("effort", &["not-a-number"])],
        );
        let invalid_error =
            preview(&context, CaptureWorkflow::Promote, &invalid).expect_err("invalid effort");
        assert!(invalid_error.contains("effort"), "{invalid_error}");

        let conflicting = submission(
            &context,
            CaptureWorkflow::Promote,
            Some(FIRST),
            &[("paths", &["src/**"]), ("clear_paths", &["yes"])],
        );
        assert!(
            preview(&context, CaptureWorkflow::Promote, &conflicting)
                .expect_err("conflicting path controls")
                .contains("clear_paths")
        );

        let discard = submission(
            &context,
            CaptureWorkflow::Discard,
            Some(FIRST),
            &[("reason", &["later"])],
        );
        commit(
            preview(&context, CaptureWorkflow::Discard, &discard).expect("discard preview"),
            &context,
        );
        let discarded = fixture.context();
        let discard_again = form(&discarded, CaptureWorkflow::Discard, Some(FIRST))
            .expect("the retained record remains inspectable");
        assert!(
            discard_again
                .render(100, 30)
                .snapshot()
                .contains("DISCARDED")
        );
        let restore = submission(&discarded, CaptureWorkflow::Restore, Some(FIRST), &[]);
        commit(
            preview(&discarded, CaptureWorkflow::Restore, &restore).expect("restore preview"),
            &discarded,
        );
        let restored = aplan_app::show_capture(
            &fixture.context(),
            &exact_selector(FIRST).expect("selector"),
            &aplan_core::capture::SystemClock,
        )
        .expect("restored capture")
        .capture;
        assert_eq!(restored.state, CaptureState::Open);
        assert!(restored.reason.is_none());
    }

    /// Proof for `tui.capture_forms_are_record_bound`: more than one lifecycle
    /// form seeded from the same retained version refuses after another writer
    /// amends that record, and no stale task or disposition is published.
    #[test]
    fn selected_capture_forms_refuse_a_record_changed_after_the_form_was_seeded() {
        let fixture = Fixture::new();
        fixture.create(FIRST, "original intake");
        fixture.create(SECOND, "different intake");
        let seeded = fixture.context();
        let redirected = submission(
            &seeded,
            CaptureWorkflow::Discard,
            Some(FIRST),
            &[("capture", &[SECOND])],
        );
        let redirect_error = preview(&seeded, CaptureWorkflow::Discard, &redirected)
            .expect_err("an editable id must not redirect the immutable form subject");
        assert!(
            redirect_error.contains("was opened for capture")
                && redirect_error.contains("now names"),
            "{redirect_error}"
        );
        let amend = submission(
            &seeded,
            CaptureWorkflow::Amend,
            Some(FIRST),
            &[("title", &["stale amendment"])],
        );
        let discard = submission(
            &seeded,
            CaptureWorkflow::Discard,
            Some(FIRST),
            &[("reason", &["stale disposition"])],
        );
        let promote = submission(
            &seeded,
            CaptureWorkflow::Promote,
            Some(FIRST),
            &[
                ("id", &["T-9"]),
                ("phase", &["1"]),
                ("effort", &["1"]),
                ("paths", &["src/**"]),
                ("tags", &["cross-cutting"]),
            ],
        );

        let external = aplan_app::preview_capture_amend(
            &fixture.context(),
            &CaptureAmendRequest {
                selector: Some(exact_selector(FIRST).expect("selector")),
                title: Some("newer external amendment".to_string()),
                ..CaptureAmendRequest::default()
            },
            &aplan_core::capture::SystemClock,
        )
        .expect("external amendment preview");
        assert!(matches!(
            aplan_app::commit_change_set(&fixture.context(), external.into_change_set()),
            aplan_app::CommitOutcome::Applied(_)
        ));

        for (workflow, submission) in [
            (CaptureWorkflow::Amend, amend),
            (CaptureWorkflow::Discard, discard),
            (CaptureWorkflow::Promote, promote),
        ] {
            let error = preview(&fixture.context(), workflow, &submission)
                .expect_err("a form seeded from an older capture must refuse");
            assert!(error.contains("changed after this form opened"), "{error}");
        }
        let retained = aplan_app::show_capture(
            &fixture.context(),
            &exact_selector(FIRST).expect("selector"),
            &aplan_core::capture::SystemClock,
        )
        .expect("retained capture")
        .capture;
        assert_eq!(retained.title, "newer external amendment");
        assert_eq!(retained.state, CaptureState::Open);
        assert!(!fixture.plan.join("tasks/T-9.md").exists());
    }
}
