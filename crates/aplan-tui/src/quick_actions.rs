//! Reusable TUI workflows for claim, release, start and stop.
//!
//! This module owns form vocabulary and a terminal-independent review
//! projection. Domain meaning and publication stay in `aplan-app`; shell
//! composition can therefore schedule these requests without parsing CLI
//! output or teaching widgets claim/status policy.

use aplan_app::{
    ChangeDisposition, SemanticEffect, WorkActionAvailability, WorkActionCatalogue,
    WorkActionChange, WorkActionKind, WorkActionPreview, WorkActionRequest,
};

use crate::form::{DefinitionError, FieldSpec, FormSubmission, MutationForm};
use crate::text;

/// The complete quick-action surface. Keeping this exhaustive makes a new
/// work action expand the form/projection tests instead of disappearing from a
/// hand-maintained menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QuickActionWorkflow {
    Claim,
    Release,
    Start,
    Stop,
}

impl QuickActionWorkflow {
    pub const ALL: [Self; 4] = [Self::Claim, Self::Release, Self::Start, Self::Stop];

    #[must_use]
    pub const fn kind(self) -> WorkActionKind {
        match self {
            Self::Claim => WorkActionKind::Claim,
            Self::Release => WorkActionKind::Release,
            Self::Start => WorkActionKind::Start,
            Self::Stop => WorkActionKind::Stop,
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Claim => "Claim task",
            Self::Release => "Release claim",
            Self::Start => "Start task",
            Self::Stop => "Stop task",
        }
    }
}

impl From<WorkActionKind> for QuickActionWorkflow {
    fn from(value: WorkActionKind) -> Self {
        match value {
            WorkActionKind::Claim => Self::Claim,
            WorkActionKind::Release => Self::Release,
            WorkActionKind::Start => Self::Start,
            WorkActionKind::Stop => Self::Stop,
        }
    }
}

/// A quick action that is unavailable in the current context or whose form no
/// longer describes that context.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Unavailable(String),
    #[error("the quick-action form belongs to input `{expected}`, not `{actual}`")]
    StaleForm { expected: String, actual: String },
    #[error("the quick-action form is missing field `{0}`")]
    MissingField(&'static str),
    #[error("field `{field}` contains more than one value")]
    RepeatedScalar { field: &'static str },
}

/// Construct the optional input form for one available action.
///
/// Release/start/stop have no input beyond the selected task, so they proceed
/// directly to preview. Claim alone exposes the CLI's `--as` and `--from`
/// semantics. The selected task and effective actor remain read-only guidance,
/// not editable copies of application state.
pub fn form(
    catalogue: &WorkActionCatalogue,
    workflow: QuickActionWorkflow,
) -> Result<Option<MutationForm>, DefinitionError> {
    let entry = catalogue.action(workflow.kind());
    let WorkActionAvailability::Available { request } = &entry.availability else {
        return Ok(None);
    };
    if workflow != QuickActionWorkflow::Claim {
        return Ok(None);
    }
    let from = request.from.clone();
    let mut from_field = FieldSpec::new(
        "from",
        "Take from",
        "Required only for takeover; must match the currently displayed holder",
    );
    if let Some(from) = from {
        from_field = from_field.required().initial_value(from);
    }
    MutationForm::new(
        workflow.title(),
        &catalogue.input_sha256,
        request.from.is_some(),
        vec![
            FieldSpec::new(
                "as",
                "Act as",
                "Blank uses the displayed host identity; a value is the CLI --as override",
            ),
            from_field,
        ],
    )
    .map(|form| {
        Some(form.with_guidance(vec![
            format!("task: {}", catalogue.task),
            format!("status: {}", catalogue.status),
            format!("ready: {}", if catalogue.ready { "yes" } else { "no" }),
            format!(
                "effective actor: {}",
                catalogue.actor.as_deref().unwrap_or("unavailable")
            ),
            format!(
                "current claim: {}",
                catalogue.claim.as_ref().map_or_else(
                    || "unclaimed".to_string(),
                    |claim| format!(
                        "{} since Unix time {}",
                        claim.holder, claim.claimed_at_epoch_seconds
                    )
                )
            ),
        ]))
    })
}

/// Decode an available catalogue entry and optional form submission into the
/// exact typed application request.
pub fn request(
    catalogue: &WorkActionCatalogue,
    workflow: QuickActionWorkflow,
    submission: Option<&FormSubmission>,
) -> Result<WorkActionRequest, Error> {
    let entry = catalogue.action(workflow.kind());
    let WorkActionAvailability::Available { request } = &entry.availability else {
        let WorkActionAvailability::Unavailable { reason } = &entry.availability else {
            unreachable!("availability has exactly two states")
        };
        return Err(Error::Unavailable(reason.clone()));
    };
    let mut request = request.clone();
    if workflow != QuickActionWorkflow::Claim {
        return Ok(request);
    }
    let submission = submission.ok_or(Error::MissingField("as/from submission"))?;
    if submission.input_sha256() != catalogue.input_sha256 {
        return Err(Error::StaleForm {
            expected: catalogue.input_sha256.clone(),
            actual: submission.input_sha256().to_string(),
        });
    }
    if let Some(as_name) = scalar(submission, "as")?.filter(|value| !value.trim().is_empty()) {
        request.as_name = Some(as_name);
    }
    request.from = scalar(submission, "from")?.filter(|value| !value.trim().is_empty());
    Ok(request)
}

fn scalar(submission: &FormSubmission, field: &'static str) -> Result<Option<String>, Error> {
    let values = submission.values(field).ok_or(Error::MissingField(field))?;
    if values.len() > 1 {
        return Err(Error::RepeatedScalar { field });
    }
    Ok(values.first().cloned())
}

/// A complete text projection of a reviewed quick action.
///
/// Values are made inert before being returned. A shell may wrap or window the
/// rows but must retain the row count when clipping, following the generic form
/// contract.
#[must_use]
pub fn review_rows(preview: &WorkActionPreview) -> Vec<String> {
    let mut rows = vec![format!(
        "PREVIEW: {} `{}`",
        preview.request().action.label(),
        preview.request().task
    )];
    rows.push(format!("Preview input: {}", preview.input().sha256));
    match preview.change() {
        WorkActionChange::Coordination(change) => {
            let verdict = if change.before == change.after {
                "NO-OP"
            } else {
                "READY"
            };
            rows.push(format!("Verdict: {verdict}"));
            rows.push(format!(
                "Coordination scope: {}",
                change.scope.id.as_deref().unwrap_or("unavailable")
            ));
            append_claim(&mut rows, "Before", change.before.as_ref());
            append_claim(&mut rows, "After", change.after.as_ref());
            rows.push("Findings (0)".to_string());
            for warning in &change.warnings {
                rows.push(format!("Warning: {warning}"));
            }
        }
        WorkActionChange::Plan(change) => {
            let verdict = match change.disposition() {
                ChangeDisposition::Ready => "READY",
                ChangeDisposition::NoOp => "NO-OP",
                ChangeDisposition::Denied => "DENIED",
            };
            rows.push(format!("Verdict: {verdict}"));
            rows.push(format!("Files ({})", change.files().len()));
            for file in change.files() {
                let path = file
                    .path()
                    .strip_prefix(&change.input().plan_root)
                    .unwrap_or(file.path());
                rows.push(format!("  replace: {}", path.display()));
            }
            rows.push(format!("Field changes ({})", change.effects().len()));
            for effect in change.effects() {
                match effect {
                    SemanticEffect::TaskStatusChanged { task, from, to } => {
                        rows.push(format!("  {task}.status: {from} -> {to}"));
                    }
                    other => rows.push(format!("  {other:?}")),
                }
            }
            rows.push(format!("Findings ({})", change.diagnostics().added.len()));
            for finding in &change.diagnostics().added {
                let task = finding
                    .task
                    .as_deref()
                    .map_or(String::new(), |task| format!(" {task}"));
                rows.push(format!(
                    "  {} {}{task}: {}",
                    finding.severity, finding.code, finding.message
                ));
                if let Some(instruction) = &finding.agent_instruction {
                    rows.push(format!("    fix: {instruction}"));
                }
                rows.extend(
                    finding
                        .detail
                        .iter()
                        .map(|detail| format!("    detail: {detail}")),
                );
            }
        }
    }
    rows.into_iter().map(|row| text::safe(&row)).collect()
}

fn append_claim(rows: &mut Vec<String>, label: &str, claim: Option<&aplan_core::ClaimRecord>) {
    let Some(claim) = claim else {
        rows.push(format!("{label}: unclaimed"));
        return;
    };
    rows.push(format!("{label}.holder: {}", claim.holder));
    rows.push(format!(
        "{label}.identity_source: {}",
        claim.identity_source
    ));
    rows.push(format!(
        "{label}.claimed_at_epoch_seconds: {}",
        claim.claimed_at_epoch_seconds
    ));
    rows.push(format!(
        "{label}.origin.host: {}",
        claim.origin.host.as_deref().unwrap_or("unknown")
    ));
    rows.push(format!(
        "{label}.origin.worktree: {}",
        claim.origin.worktree.as_deref().unwrap_or("unknown")
    ));
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use aplan_app::{ContextInputs, PlanContext, Trust, WorkActionError};

    use super::*;
    use crate::form::{FormAction, FormTransition};

    struct Fixture {
        _dir: tempfile::TempDir,
        repository: PathBuf,
        plan: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("fixture");
            let repository = dir.path().to_path_buf();
            let plan = repository.join("plan");
            std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
            for (id, status, blocked) in [
                ("A-1", "todo", ""),
                ("A-2", "todo", "blocked_by: [A-1]\n"),
                ("A-3", "in_progress", ""),
                ("A-4", "blocked", ""),
                ("A-5", "done", ""),
                ("A-6", "dropped", ""),
            ] {
                let evidence = if status == "done" {
                    "evidence:\n  - kind: note\n    ref: 2026-10-25 reviewed\n"
                } else {
                    ""
                };
                std::fs::write(
                    plan.join(format!("tasks/{id}.md")),
                    format!(
                        "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: {status}\neffort_days: 1\n{blocked}paths: [src/**]\ntags: [cross-cutting]\n{evidence}---\n"
                    ),
                )
                .expect("task");
            }
            std::fs::create_dir_all(repository.join("src")).expect("source");
            std::fs::write(repository.join("src/lib.rs"), "pub fn one() {}\n").expect("source");
            git(&repository, &["init", "-q"]);
            git(&repository, &["config", "user.email", "test@example.com"]);
            git(&repository, &["config", "user.name", "Fixture"]);
            git(&repository, &["add", "."]);
            git(&repository, &["commit", "-qm", "fixture"]);
            Self {
                _dir: dir,
                repository,
                plan,
            }
        }

        fn context(&self) -> PlanContext {
            PlanContext::new(
                aplan_core::load_plan_identified(&self.plan),
                ContextInputs {
                    repository_root: self.repository.clone(),
                    configuration_file: None,
                    trust: Trust::RepositoryRead,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            )
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    fn submission(mut form: MutationForm, as_name: &str) -> FormSubmission {
        if !as_name.is_empty() {
            let _ = form.reduce(FormAction::Insert(as_name.to_string()));
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("complete form must request preview")
        };
        submission
    }

    #[test]
    fn every_quick_action_has_one_exhaustive_workflow_and_contextual_disposition() {
        assert_eq!(
            QuickActionWorkflow::ALL.map(QuickActionWorkflow::kind),
            WorkActionKind::ALL
        );
        let fixture = Fixture::new();
        let context = fixture.context();
        for id in ["A-1", "A-2", "A-3", "A-4", "A-5", "A-6"] {
            let catalogue =
                aplan_app::work_action_catalogue(&context, id, Some("alex")).expect("catalogue");
            assert_eq!(catalogue.actions().len(), QuickActionWorkflow::ALL.len());
            for workflow in QuickActionWorkflow::ALL {
                assert_eq!(catalogue.action(workflow.kind()).action, workflow.kind());
            }
        }
    }

    #[test]
    fn claim_form_preserves_cli_identity_and_takeover_semantics() {
        let fixture = Fixture::new();
        let context = fixture.context();
        let catalogue =
            aplan_app::work_action_catalogue(&context, "A-1", Some("alex")).expect("catalogue");
        let claim_form = form(&catalogue, QuickActionWorkflow::Claim)
            .expect("definition")
            .expect("claim form");
        let claim = request(
            &catalogue,
            QuickActionWorkflow::Claim,
            Some(&submission(claim_form, "casey")),
        )
        .expect("request");
        assert_eq!(claim.as_name.as_deref(), Some("casey"));
        assert!(claim.from.is_none());
        let preview = aplan_app::preview_work_action(&context, &claim).expect("preview");
        let rows = review_rows(&preview).join("\n");
        assert!(rows.contains("After.holder: casey"), "{rows}");
        assert!(rows.contains("Before: unclaimed"), "{rows}");

        let default_form = form(&catalogue, QuickActionWorkflow::Claim)
            .expect("definition")
            .expect("claim form");
        let default_actor = request(
            &catalogue,
            QuickActionWorkflow::Claim,
            Some(&submission(default_form, "")),
        )
        .expect("default actor");
        assert_eq!(default_actor.as_name.as_deref(), Some("alex"));

        let applied = aplan_app::commit_work_action(&context, preview);
        assert!(matches!(
            applied,
            aplan_app::WorkActionCommitOutcome::Coordination(
                aplan_app::CoordinationOutcome::Applied(_)
            )
        ));
        let claimed = fixture.context();
        let takeover =
            aplan_app::work_action_catalogue(&claimed, "A-1", Some("alex")).expect("catalogue");
        let mut takeover_form = form(&takeover, QuickActionWorkflow::Claim)
            .expect("definition")
            .expect("takeover form");
        assert!(takeover_form.is_destructive());
        let _ = takeover_form.reduce(FormAction::Insert("alex".to_string()));
        let FormTransition::PreviewRequested(submission) = takeover_form.reduce(FormAction::Review)
        else {
            panic!("prepopulated holder makes takeover complete")
        };
        let takeover =
            request(&takeover, QuickActionWorkflow::Claim, Some(&submission)).expect("takeover");
        assert_eq!(takeover.from.as_deref(), Some("casey"));
        assert_eq!(takeover.as_name.as_deref(), Some("alex"));
    }

    #[test]
    fn zero_input_actions_reach_exact_application_previews_without_fake_fields() {
        let fixture = Fixture::new();
        let context = fixture.context();
        let claim = aplan_app::preview_work_action(
            &context,
            &aplan_app::WorkActionRequest {
                task: "A-1".to_string(),
                action: WorkActionKind::Claim,
                as_name: Some("alex".to_string()),
                from: None,
                claimed_at_epoch_seconds: Some(10),
            },
        )
        .expect("claim");
        assert!(matches!(
            aplan_app::commit_work_action(&context, claim),
            aplan_app::WorkActionCommitOutcome::Coordination(
                aplan_app::CoordinationOutcome::Applied(_)
            )
        ));
        let context = fixture.context();

        for workflow in [QuickActionWorkflow::Release, QuickActionWorkflow::Start] {
            let catalogue =
                aplan_app::work_action_catalogue(&context, "A-1", Some("alex")).expect("catalogue");
            assert!(form(&catalogue, workflow).expect("definition").is_none());
            let request = request(&catalogue, workflow, None).expect("request");
            let preview = aplan_app::preview_work_action(&context, &request).expect("preview");
            let rows = review_rows(&preview).join("\n");
            assert!(rows.contains(&format!("PREVIEW: {} `A-1`", workflow.kind().label())));
            if workflow == QuickActionWorkflow::Start {
                assert!(rows.contains("A-1.status: todo -> in_progress"), "{rows}");
                assert!(rows.contains("Findings (0)"), "{rows}");
            } else {
                assert!(rows.contains("Before.holder: alex"), "{rows}");
                assert!(rows.contains("After: unclaimed"), "{rows}");
            }
        }
    }

    #[test]
    fn unavailable_and_stale_forms_cannot_bypass_context() {
        let fixture = Fixture::new();
        let context = fixture.context();
        let blocked =
            aplan_app::work_action_catalogue(&context, "A-2", Some("alex")).expect("blocked");
        assert!(matches!(
            request(&blocked, QuickActionWorkflow::Start, None),
            Err(Error::Unavailable(reason)) if reason.contains("not ready")
        ));
        let terminal =
            aplan_app::work_action_catalogue(&context, "A-5", Some("alex")).expect("terminal");
        assert!(QuickActionWorkflow::ALL.into_iter().all(|workflow| {
            matches!(
                request(&terminal, workflow, None),
                Err(Error::Unavailable(_))
            )
        }));

        let ready = aplan_app::work_action_catalogue(&context, "A-1", Some("alex")).expect("ready");
        let form = form(&ready, QuickActionWorkflow::Claim)
            .expect("definition")
            .expect("form");
        let submission = submission(form, "alex");
        // The fields are private by design; construct a different catalogue to
        // prove the input binding is checked at decode rather than relying on
        // widget routing.
        let mut moved = ready.clone();
        moved.input_sha256 = "new input".to_string();
        assert!(matches!(
            request(&moved, QuickActionWorkflow::Claim, Some(&submission)),
            Err(Error::StaleForm { .. })
        ));
    }

    #[test]
    fn direct_tui_requests_match_application_cli_semantics_for_all_statuses() {
        let fixture = Fixture::new();
        let context = fixture.context();
        for (id, action, disposition) in [
            ("A-1", WorkActionKind::Start, Some(ChangeDisposition::Ready)),
            ("A-2", WorkActionKind::Stop, Some(ChangeDisposition::NoOp)),
            ("A-3", WorkActionKind::Start, Some(ChangeDisposition::NoOp)),
            ("A-3", WorkActionKind::Stop, Some(ChangeDisposition::Ready)),
            ("A-4", WorkActionKind::Start, Some(ChangeDisposition::Ready)),
            ("A-4", WorkActionKind::Stop, None),
            ("A-5", WorkActionKind::Start, None),
            ("A-6", WorkActionKind::Start, None),
        ] {
            let result =
                aplan_app::preview_work_action(&context, &WorkActionRequest::simple(id, action));
            match disposition {
                Some(expected) => assert_eq!(
                    result
                        .expect("CLI-compatible preview")
                        .change_set()
                        .expect("status change")
                        .disposition(),
                    expected,
                    "{id} {}",
                    action.label()
                ),
                None => assert!(
                    matches!(result, Err(WorkActionError::Arguments(_))),
                    "{id} {}: {result:?}",
                    action.label()
                ),
            }
        }
    }

    #[test]
    fn workflow_shapes_and_review_rows_fail_closed_at_every_public_boundary() {
        assert_eq!(
            QuickActionWorkflow::ALL.map(QuickActionWorkflow::title),
            ["Claim task", "Release claim", "Start task", "Stop task"]
        );
        assert_eq!(
            WorkActionKind::ALL.map(QuickActionWorkflow::from),
            QuickActionWorkflow::ALL
        );

        let fixture = Fixture::new();
        let context = fixture.context();
        let blocked =
            aplan_app::work_action_catalogue(&context, "A-2", Some("alex")).expect("blocked");
        assert!(
            form(&blocked, QuickActionWorkflow::Start)
                .expect("definition")
                .is_none()
        );

        let ready = aplan_app::work_action_catalogue(&context, "A-1", Some("alex")).expect("ready");
        assert_eq!(
            request(&ready, QuickActionWorkflow::Claim, None),
            Err(Error::MissingField("as/from submission"))
        );

        let wrong_form = MutationForm::new(
            "wrong form",
            &ready.input_sha256,
            false,
            vec![FieldSpec::new("other", "Other", "not a claim field")],
        )
        .expect("wrong form");
        let wrong_submission = submission(wrong_form, "");
        assert_eq!(
            request(&ready, QuickActionWorkflow::Claim, Some(&wrong_submission)),
            Err(Error::MissingField("as"))
        );

        let repeated_form = MutationForm::new(
            "repeated actor",
            &ready.input_sha256,
            false,
            vec![
                FieldSpec::new("as", "Act as", "invalid repeated shape")
                    .repeated()
                    .initial_values(["alex", "blair"]),
                FieldSpec::new("from", "Take from", "holder"),
            ],
        )
        .expect("repeated form");
        let repeated_submission = submission(repeated_form, "");
        assert_eq!(
            request(
                &ready,
                QuickActionWorkflow::Claim,
                Some(&repeated_submission)
            ),
            Err(Error::RepeatedScalar { field: "as" })
        );

        let release = aplan_app::preview_work_action(
            &context,
            &WorkActionRequest::simple("A-1", WorkActionKind::Release),
        )
        .expect("empty release preview");
        let release_rows = review_rows(&release).join("\n");
        assert!(release_rows.contains("Verdict: NO-OP"), "{release_rows}");
        assert!(release_rows.contains("Before: unclaimed"), "{release_rows}");
        assert!(release_rows.contains("After: unclaimed"), "{release_rows}");

        let start = aplan_app::preview_work_action(
            &context,
            &WorkActionRequest::simple("A-3", WorkActionKind::Start),
        )
        .expect("idempotent start preview");
        let start_rows = review_rows(&start).join("\n");
        assert!(start_rows.contains("Verdict: NO-OP"), "{start_rows}");
        assert!(start_rows.contains("Files (0)"), "{start_rows}");
        assert!(start_rows.contains("Field changes (0)"), "{start_rows}");

        let warning = aplan_app::preview_work_action(
            &context,
            &WorkActionRequest {
                task: "A-1".to_string(),
                action: WorkActionKind::Claim,
                as_name: Some("root".to_string()),
                from: None,
                claimed_at_epoch_seconds: Some(10),
            },
        )
        .expect("placeholder preview");
        let warning_rows = review_rows(&warning).join("\n");
        assert!(
            warning_rows.contains("Warning: identity label `root`"),
            "{warning_rows}"
        );
        assert!(
            warning_rows.contains("Coordination scope: sha256:"),
            "{warning_rows}"
        );
    }
}
