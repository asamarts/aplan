//! Guided close, evidence repair, restamp and verification-fix forms.
//!
//! The form registry is deliberately driven by [`aplan_core::EvidenceKind::ALL`].
//! A new evidence kind cannot disappear behind a hand-written four-of-five UI:
//! its exhaustive driver must define input, resolution and result guidance
//! before this module compiles and its census passes.

use aplan_app::{PlanContext, TestCatalogueRequest};
use aplan_core::{Evidence, EvidenceKind};

use crate::form::{DefinitionError, FieldSpec, FormSubmission, MutationForm};

/// Evidence-bearing workflows delivered by this cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceWorkflow {
    Close,
    AmendAdd,
    AmendReplace,
    AmendRemove,
    Restamp,
    VerifyFix,
}

impl EvidenceWorkflow {
    pub const ALL: &'static [Self] = &[
        Self::Close,
        Self::AmendAdd,
        Self::AmendReplace,
        Self::AmendRemove,
        Self::Restamp,
        Self::VerifyFix,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Close => "Close task with evidence",
            Self::AmendAdd => "Add evidence to closed task",
            Self::AmendReplace => "Replace evidence on closed task",
            Self::AmendRemove => "Remove evidence from closed task",
            Self::Restamp => "Review and restamp baselines",
            Self::VerifyFix => "Apply verification repairs",
        }
    }

    pub const fn destructive(self) -> bool {
        matches!(
            self,
            Self::Close | Self::AmendReplace | Self::AmendRemove | Self::Restamp | Self::VerifyFix
        )
    }
}

/// One evidence kind's complete interactive disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceInputDriver {
    pub kind: EvidenceKind,
    pub field: &'static str,
    pub label: &'static str,
    pub input_help: &'static str,
    pub resolution_help: &'static str,
    pub result_help: &'static str,
    pub requires_authored_date: bool,
}

/// The exhaustive kind driver. Adding a variant to `EvidenceKind` makes this
/// match fail to compile until its complete interactive answer is supplied.
#[must_use]
pub const fn evidence_driver(kind: EvidenceKind) -> EvidenceInputDriver {
    match kind {
        EvidenceKind::Commit => EvidenceInputDriver {
            kind,
            field: "commit",
            label: "Commit",
            input_help: "Full or unambiguous commit id; blank close input derives current HEAD",
            resolution_help: "Resolved in the selected repository and checked against task paths",
            result_help: "Preview shows resolver strength, locality, dilution and exact commit id",
            requires_authored_date: false,
        },
        EvidenceKind::Url => EvidenceInputDriver {
            kind,
            field: "url",
            label: "URL",
            input_help: "External evidence URL; repeat for multiple independent records",
            resolution_help: "Close deliberately does not contact the network, so URL stays unresolved",
            result_help: "Preview labels the citation unresolved instead of implying a live check passed",
            requires_authored_date: false,
        },
        EvidenceKind::File => EvidenceInputDriver {
            kind,
            field: "file",
            label: "Committed file",
            input_help: "Repository-relative path, optionally #anchor and/or @revision",
            resolution_help: "Resolved from committed repository content and bound to task paths",
            result_help: "Preview shows the exact file citation and mutable-reference fingerprint work",
            requires_authored_date: false,
        },
        EvidenceKind::Test => EvidenceInputDriver {
            kind,
            field: "test",
            label: "Configured test",
            input_help: "Exact normalized name returned by the configured test catalogue",
            resolution_help: "Runner listings are searched at L2; explicit Run cited tests also requires an exact L3 pass",
            result_help: "Preview retains each typed L2/L3 outcome and refuses ambiguity, failure or unavailable execution",
            requires_authored_date: false,
        },
        EvidenceKind::Note => EvidenceInputDriver {
            kind,
            field: "note",
            label: "Dated note",
            input_help: "Human assertion text; each entry needs the corresponding authored YYYY-MM-DD date",
            resolution_help: "A note is non-resolving by design and is never presented as machine proof",
            result_help: "Preview keeps the note in the unresolved ledger with its author-supplied date",
            requires_authored_date: true,
        },
    }
}

#[must_use]
pub fn evidence_drivers() -> Vec<EvidenceInputDriver> {
    EvidenceKind::ALL
        .iter()
        .copied()
        .map(evidence_driver)
        .collect()
}

/// Construct one evidence workflow's reusable form.
pub fn form(
    context: &PlanContext,
    workflow: EvidenceWorkflow,
    selected_task: Option<&str>,
) -> Result<MutationForm, DefinitionError> {
    let form = MutationForm::new(
        workflow.title(),
        &context.identity().sha256,
        workflow.destructive(),
        fields(workflow, selected_task),
    )?;
    let mut guidance = selected_task
        .and_then(|id| aplan_app::evidence_requirement(context, id).ok())
        .map_or_else(Vec::new, |requirement| requirement_guidance(&requirement));
    if workflow == EvidenceWorkflow::Close {
        guidance.push(
            "Focused L3 is off by default. Type yes in Run cited tests to start the configured test.run/test.batch boundary during Preview."
                .to_string(),
        );
    }
    Ok(form.with_guidance(guidance))
}

fn requirement_guidance(requirement: &aplan_app::EvidenceRequirement) -> Vec<String> {
    let required = match &requirement.required {
        None => "required kinds: not configured".to_string(),
        Some(kinds) if kinds.is_empty() => "required kinds: explicitly none".to_string(),
        Some(kinds) => format!(
            "required kinds: {}",
            kinds
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let mut rows = vec![required];
    if !requirement.exempt_tags.is_empty() {
        rows.push(format!(
            "exempt tags: {}",
            requirement.exempt_tags.join(", ")
        ));
    }
    if !requirement.matched_exemptions.is_empty() {
        rows.push(format!(
            "this task matches: {}",
            requirement.matched_exemptions.join(", ")
        ));
    }
    rows
}

fn fields(workflow: EvidenceWorkflow, selected_task: Option<&str>) -> Vec<FieldSpec> {
    let task = || {
        selected_task.map_or_else(
            || FieldSpec::new("id", "Task", "Task id").required(),
            |id| {
                FieldSpec::new("id", "Task", "Task id")
                    .required()
                    .initial_value(id)
            },
        )
    };
    match workflow {
        EvidenceWorkflow::Close => {
            let mut fields = vec![task()];
            for driver in evidence_drivers() {
                fields
                    .push(FieldSpec::new(driver.field, driver.label, driver.input_help).repeated());
                if driver.requires_authored_date {
                    fields.push(
                        FieldSpec::new(
                            "note_captured",
                            "Note authored dates",
                            "One YYYY-MM-DD value for each non-blank note, in the same order",
                        )
                        .repeated(),
                    );
                }
            }
            fields.push(FieldSpec::new(
                "no_resolve",
                "Record unchecked",
                "Type yes only to skip repository and configured-runner resolution explicitly",
            ));
            fields.push(FieldSpec::new(
                "execute",
                "Run cited tests",
                "Type yes to start configured test.run/test.batch commands during Preview; no keeps close at L2",
            ));
            fields
        }
        EvidenceWorkflow::AmendAdd => vec![
            task(),
            kind_field("kind", "Kind to add"),
            FieldSpec::new("reference", "Reference", "New citation reference").required(),
            FieldSpec::new(
                "captured",
                "Authored date",
                "Required only for note evidence; YYYY-MM-DD",
            ),
            no_resolve_field(),
        ],
        EvidenceWorkflow::AmendReplace => vec![
            task(),
            kind_field("from_kind", "Existing kind"),
            FieldSpec::new(
                "from_reference",
                "Existing reference",
                "Citation to replace",
            )
            .required(),
            kind_field("to_kind", "Replacement kind"),
            FieldSpec::new("to_reference", "Replacement reference", "New citation").required(),
            FieldSpec::new(
                "to_captured",
                "Replacement authored date",
                "Required only when the replacement is a note; YYYY-MM-DD",
            ),
            no_resolve_field(),
        ],
        EvidenceWorkflow::AmendRemove => vec![
            task(),
            kind_field("kind", "Kind to remove"),
            FieldSpec::new("reference", "Reference", "Existing citation to remove").required(),
            no_resolve_field(),
        ],
        EvidenceWorkflow::Restamp => vec![task()],
        EvidenceWorkflow::VerifyFix => vec![FieldSpec::new(
            "review",
            "Batch review",
            "Type review after inspecting every exact repair; this does not bypass confirmation",
        )
        .required()],
    }
}

fn kind_field(key: &str, label: &str) -> FieldSpec {
    let accepted = EvidenceKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    FieldSpec::new(
        key,
        label,
        format!("{accepted}; kind-specific guidance follows selection"),
    )
    .required()
}

fn no_resolve_field() -> FieldSpec {
    FieldSpec::new(
        "no_resolve",
        "Record unchecked",
        "Type yes only to make the unresolved trust boundary explicit",
    )
}

/// Typed request decoded from a complete evidence form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceRequest {
    Close(aplan_app::CloseRequest),
    Amend(aplan_app::AmendRequest),
    Restamp(aplan_app::RestampRequest),
    VerifyFix,
}

impl EvidenceRequest {
    /// Run a task-local preview. Verification repair batches need the retained
    /// workbench and therefore use [`preview_verification_fix`] instead.
    pub fn preview(
        &self,
        context: &PlanContext,
    ) -> Result<aplan_app::EvidenceMutationPreview, InputError> {
        match self {
            Self::Close(request) => {
                aplan_app::preview_close(context, request).map_err(InputError::Domain)
            }
            Self::Amend(request) => {
                aplan_app::preview_amend(context, request).map_err(InputError::Domain)
            }
            Self::Restamp(request) => {
                aplan_app::preview_restamp(context, request).map_err(InputError::Domain)
            }
            Self::VerifyFix => Err(InputError::Invalid {
                field: "review",
                message: "verification fixes require the retained verification result".to_string(),
            }),
        }
    }
}

/// Decode one complete form submission.
pub fn request(
    workflow: EvidenceWorkflow,
    submission: &FormSubmission,
) -> Result<EvidenceRequest, InputError> {
    let id = || required(submission, "id");
    match workflow {
        EvidenceWorkflow::Close => {
            let no_resolve = yes_no(submission, "no_resolve")?;
            let execute = yes_no(submission, "execute")?;
            if no_resolve && execute {
                return Err(InputError::Invalid {
                    field: "execute",
                    message:
                        "focused L3 cannot run while evidence resolution is explicitly skipped"
                            .to_string(),
                });
            }
            Ok(EvidenceRequest::Close(aplan_app::CloseRequest {
                id: id()?,
                evidence: close_evidence(submission)?,
                no_resolve,
                execute,
            }))
        }
        EvidenceWorkflow::AmendAdd => Ok(EvidenceRequest::Amend(aplan_app::AmendRequest {
            id: id()?,
            operation: aplan_app::AmendOperation::Add(citation(
                submission,
                "kind",
                "reference",
                Some("captured"),
                false,
            )?),
            no_resolve: yes_no(submission, "no_resolve")?,
        })),
        EvidenceWorkflow::AmendReplace => Ok(EvidenceRequest::Amend(aplan_app::AmendRequest {
            id: id()?,
            operation: aplan_app::AmendOperation::Replace {
                from: citation(submission, "from_kind", "from_reference", None, true)?,
                to: citation(
                    submission,
                    "to_kind",
                    "to_reference",
                    Some("to_captured"),
                    false,
                )?,
            },
            no_resolve: yes_no(submission, "no_resolve")?,
        })),
        EvidenceWorkflow::AmendRemove => Ok(EvidenceRequest::Amend(aplan_app::AmendRequest {
            id: id()?,
            operation: aplan_app::AmendOperation::Remove(citation(
                submission,
                "kind",
                "reference",
                None,
                true,
            )?),
            no_resolve: yes_no(submission, "no_resolve")?,
        })),
        EvidenceWorkflow::Restamp => Ok(EvidenceRequest::Restamp(aplan_app::RestampRequest {
            id: id()?,
        })),
        EvidenceWorkflow::VerifyFix => {
            if required(submission, "review")? != "review" {
                return Err(InputError::Invalid {
                    field: "review",
                    message: "type `review` after inspecting the repair batch".to_string(),
                });
            }
            Ok(EvidenceRequest::VerifyFix)
        }
    }
}

/// Prepare the exact retained verification repair batch.
pub fn preview_verification_fix(
    request: &EvidenceRequest,
    context: &PlanContext,
    verification: &aplan_app::VerificationWorkbench,
    options: &aplan_evidence::VerifyOptions,
) -> Result<aplan_app::EvidenceMutationPreview, InputError> {
    if !matches!(request, EvidenceRequest::VerifyFix) {
        return Err(InputError::Invalid {
            field: "review",
            message: "this request is not a verification repair batch".to_string(),
        });
    }
    aplan_app::preview_verification_repairs(context, verification, options)
        .map_err(InputError::Domain)
}

fn close_evidence(submission: &FormSubmission) -> Result<Vec<Evidence>, InputError> {
    let note_dates = values(submission, "note_captured");
    let notes = values(submission, "note");
    if notes.len() != note_dates.len() {
        return Err(InputError::Invalid {
            field: "note_captured",
            message: format!(
                "{} note(s) need exactly {} authored date value(s)",
                notes.len(),
                notes.len()
            ),
        });
    }
    let mut evidence = Vec::new();
    for driver in evidence_drivers() {
        for (index, reference) in values(submission, driver.field).into_iter().enumerate() {
            evidence.push(Evidence {
                kind: driver.kind,
                reference,
                captured: driver
                    .requires_authored_date
                    .then(|| note_dates[index].clone()),
            });
        }
    }
    Ok(evidence)
}

fn citation(
    submission: &FormSubmission,
    kind_field: &'static str,
    reference_field: &'static str,
    captured_field: Option<&'static str>,
    naming_existing: bool,
) -> Result<Evidence, InputError> {
    let kind = parse_kind(&required(submission, kind_field)?, kind_field)?;
    let captured = captured_field.and_then(|field| optional(submission, field));
    if naming_existing && captured.is_some() {
        return Err(InputError::Invalid {
            field: captured_field.unwrap_or(reference_field),
            message: "an existing citation is addressed by kind and reference, not date"
                .to_string(),
        });
    }
    Ok(Evidence {
        kind,
        reference: required(submission, reference_field)?,
        captured,
    })
}

fn parse_kind(value: &str, field: &'static str) -> Result<EvidenceKind, InputError> {
    EvidenceKind::ALL
        .iter()
        .copied()
        .find(|kind| kind.as_str() == value)
        .ok_or_else(|| InputError::Invalid {
            field,
            message: format!("unknown evidence kind `{value}`"),
        })
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

fn values(submission: &FormSubmission, field: &str) -> Vec<String> {
    submission
        .values(field)
        .into_iter()
        .flatten()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn yes_no(submission: &FormSubmission, field: &'static str) -> Result<bool, InputError> {
    match optional(submission, field).as_deref() {
        None | Some("no") => Ok(false),
        Some("yes") => Ok(true),
        Some(_) => Err(InputError::Invalid {
            field,
            message: "expected `yes` or `no`".to_string(),
        }),
    }
}

/// Search result retaining exact resolver usability rather than only a label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestEvidenceChoice {
    pub name: String,
    /// Reference inserted into a field whose kind is already `test`.
    pub reference: String,
    /// Complete citation shown for review or copied outside the typed form.
    pub citation: String,
    pub selectable: bool,
    pub advice: String,
}

/// One complete configured-test answer retained by the interactive selector.
///
/// The worker returns the catalogue and its insertion-safe choices together so
/// the shell cannot accidentally render a name from one observation with
/// ambiguity or failure advice from another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCatalogueView {
    pub catalogue: aplan_app::TestCatalogue,
    pub choices: Vec<TestEvidenceChoice>,
}

/// Search the shared configured-runner catalogue used by close and verify.
pub fn search_tests(
    context: &PlanContext,
    query: Option<String>,
) -> Result<TestCatalogueView, InputError> {
    let config = context.report_config();
    let catalogue = aplan_app::test_catalogue(
        &context.identity().repository_root,
        config.and_then(|config| config.test.as_ref()),
        context.identity().configuration.path.as_deref(),
        context.verdict(),
        TestCatalogueRequest { query },
    )
    .map_err(InputError::Catalogue)?;
    let choices = catalogue
        .runners
        .iter()
        .flat_map(|runner| runner.tests.iter().map(move |test| (runner, test)))
        .map(|(runner, test)| TestEvidenceChoice {
            name: test.name.clone(),
            reference: test.name.clone(),
            citation: test.evidence.clone(),
            selectable: !test.ambiguous,
            advice: if test.ambiguous {
                format!(
                    "refused: `{}` is listed by {} configured runners; make test names unique before citing it",
                    test.name,
                    test.listed_by.len()
                )
            } else {
                format!(
                    "accepted reference: use `{}` unchanged (citation `{}`); listed by runner {} ({})",
                    test.name, test.evidence, runner.runner.index, runner.runner.label
                )
            },
        })
        .collect();
    Ok(TestCatalogueView { catalogue, choices })
}

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("field `{0}` is required")]
    Required(&'static str),
    #[error("field `{field}` is invalid: {message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
    #[error(transparent)]
    Domain(#[from] aplan_app::EvidenceMutationError),
    #[error(transparent)]
    Catalogue(#[from] aplan_app::TestCatalogueError),
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::form::{FormAction, FormFocus, FormTransition};

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, text).expect("fixture");
    }

    fn context(root: &Path, runners: Option<aplan_config::TestRunners>) -> aplan_app::PlanContext {
        context_with_trust(root, runners, aplan_app::Trust::RepositoryRead)
    }

    fn context_with_trust(
        root: &Path,
        runners: Option<aplan_config::TestRunners>,
        trust: aplan_app::Trust,
    ) -> aplan_app::PlanContext {
        write(
            &root.join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/**]\n---\n",
        );
        aplan_app::PlanContext::new(
            aplan_core::load_plan_identified(&root.join("aplan")),
            aplan_app::ContextInputs {
                repository_root: root.to_path_buf(),
                configuration_file: None,
                trust,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(aplan_config::Config {
            test: runners,
            ..aplan_config::Config::default()
        })
    }

    fn runner(name: &str, listed: &str) -> aplan_config::TestRunner {
        aplan_config::TestRunner {
            weight: None,
            exclusive: None,
            name: Some(name.to_string()),
            dir: None,
            covers: None,
            covers_by_test: None,
            inputs_by_test: None,
            skipped: None,
            output_lines: None,
            max_output: None,
            expect_tests: None,
            max_memory: None,
            max_processes: None,
            wrap: None,
            run_selects: None,
            list: list_command(listed),
            run: successful_run_command(),
            batch: None,
            timeout_seconds: None,
        }
    }

    #[cfg(unix)]
    fn list_command(listed: &str) -> Vec<String> {
        vec!["printf".to_string(), format!("{listed}\n")]
    }

    #[cfg(windows)]
    fn list_command(listed: &str) -> Vec<String> {
        let writes = listed
            .lines()
            .map(|line| format!("Write-Output '{}'", line.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join("; ");
        vec![
            "powershell.exe".to_string(),
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            writes,
        ]
    }

    #[cfg(unix)]
    fn successful_run_command() -> Vec<String> {
        vec!["true".to_string(), "{test}".to_string()]
    }

    #[cfg(windows)]
    fn successful_run_command() -> Vec<String> {
        vec![
            "powershell.exe".to_string(),
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            "if ('{test}' -eq '') { exit 1 }; exit 0".to_string(),
        ]
    }

    #[cfg(unix)]
    fn failing_list_command() -> Vec<String> {
        vec!["false".to_string()]
    }

    #[cfg(windows)]
    fn failing_list_command() -> Vec<String> {
        vec![
            "powershell.exe".to_string(),
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            "exit 1".to_string(),
        ]
    }

    #[cfg(unix)]
    fn marked_list_command(marker: &Path, listed: &str) -> Vec<String> {
        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("touch '{}'; printf '{listed}\\n'", marker.display()),
        ]
    }

    #[cfg(windows)]
    fn marked_list_command(marker: &Path, listed: &str) -> Vec<String> {
        vec![
            "powershell.exe".to_string(),
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            format!(
                "[IO.File]::WriteAllText('{}', ''); Write-Output '{}'",
                marker.display().to_string().replace('\'', "''"),
                listed.replace('\'', "''")
            ),
        ]
    }

    #[cfg(unix)]
    fn marked_run_command(marker: &Path) -> Vec<String> {
        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("printf executed > '{}'", marker.display()),
            "focused-close".to_string(),
            "{test}".to_string(),
        ]
    }

    #[cfg(windows)]
    fn marked_run_command(marker: &Path) -> Vec<String> {
        vec![
            "powershell.exe".to_string(),
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            format!(
                "if ('{{test}}' -ne 'proof') {{ exit 1 }}; [IO.File]::WriteAllText('{}', 'executed')",
                marker.display().to_string().replace('\'', "''")
            ),
        ]
    }

    fn submission(mut form: MutationForm, edits: &[(&str, &str)]) -> FormSubmission {
        for (key, value) in edits {
            let index = form
                .fields()
                .iter()
                .position(|field| field.spec().key() == *key)
                .expect("field exists");
            while form.focus() != FormFocus::Field(index) {
                let _ = form.reduce(FormAction::NextFocus);
            }
            let _ = form.reduce(FormAction::Insert((*value).to_string()));
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("complete form did not yield a submission")
        };
        submission
    }

    /// Proof for `tui.evidence_mutation_previews_bind_identified_inputs`: every kind
    /// drives a concrete field and reaches the exact guarded preview rather
    /// than merely appearing in a hand-maintained inventory.
    #[test]
    fn every_evidence_kind_drives_input_help_resolution_and_preview_results() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path(), None);
        let drivers = evidence_drivers();
        assert_eq!(
            drivers.iter().map(|driver| driver.kind).collect::<Vec<_>>(),
            EvidenceKind::ALL,
            "the driver census must be executable and exhaustive"
        );
        let close = form(&context, EvidenceWorkflow::Close, Some("A-1")).expect("close form");
        for driver in &drivers {
            assert!(!driver.input_help.is_empty());
            assert!(!driver.resolution_help.is_empty());
            assert!(!driver.result_help.is_empty());
            let field = close
                .fields()
                .iter()
                .find(|field| field.spec().key() == driver.field)
                .expect("every driver creates a real field");
            assert_eq!(field.spec().cardinality(), crate::form::Cardinality::Many);
        }

        let close = submission(
            close,
            &[
                ("commit", "abc123"),
                ("url", "https://example.invalid/proof"),
                ("file", "proof.md#contract"),
                ("test", "suite::proof"),
                ("note", "manual review"),
                ("note_captured", "2026-10-24"),
                ("no_resolve", "yes"),
            ],
        );
        let decoded = request(EvidenceWorkflow::Close, &close).expect("decode all kinds");
        let EvidenceRequest::Close(close) = &decoded else {
            panic!("wrong request")
        };
        assert_eq!(
            close
                .evidence
                .iter()
                .map(|citation| citation.kind)
                .collect::<Vec<_>>(),
            EvidenceKind::ALL
        );
        let preview = decoded.preview(&context).expect("unresolved preview");
        assert!(matches!(
            preview.change_set().evidence(),
            aplan_app::ChangeEvidence::Skipped { citations, .. }
                if citations.len() == EvidenceKind::ALL.len()
        ));
    }

    #[test]
    fn every_evidence_workflow_has_a_distinct_complete_form() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path(), None);
        let cases = [
            (EvidenceWorkflow::Close, 9),
            (EvidenceWorkflow::AmendAdd, 5),
            (EvidenceWorkflow::AmendReplace, 7),
            (EvidenceWorkflow::AmendRemove, 4),
            (EvidenceWorkflow::Restamp, 1),
            (EvidenceWorkflow::VerifyFix, 1),
        ];
        assert_eq!(
            cases.iter().map(|case| case.0).collect::<Vec<_>>(),
            EvidenceWorkflow::ALL
        );
        for (workflow, fields) in cases {
            let form = form(&context, workflow, Some("A-1")).expect("form");
            assert_eq!(form.title(), workflow.title());
            assert_eq!(form.fields().len(), fields);
            assert_eq!(form.is_destructive(), workflow.destructive());
            if workflow == EvidenceWorkflow::Close {
                assert!(
                    form.guidance().iter().any(|line| {
                        line.contains("off by default")
                            && line.contains("test.run/test.batch boundary")
                    }),
                    "the interactive execution boundary is not named before preview"
                );
            }
        }
    }

    #[test]
    fn configured_test_search_preserves_ambiguous_and_unlistable_advice() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let catalogue_context = context(
            dir.path(),
            Some(aplan_config::TestRunners::Many(vec![
                runner("left", "shared\nleft_only"),
                runner("right", "shared\nright_only"),
            ])),
        );
        let TestCatalogueView { catalogue, choices } =
            search_tests(&catalogue_context, Some("shared".to_string())).expect("catalogue");
        assert_eq!(catalogue.summary.ambiguous_names, 1);
        assert_eq!(
            choices.len(),
            2,
            "one ambiguous spelling appears per runner"
        );
        assert!(choices.iter().all(|choice| !choice.selectable));
        assert!(
            choices
                .iter()
                .all(|choice| choice.advice.contains("listed by 2 configured runners"))
        );

        let TestCatalogueView {
            choices: unique, ..
        } = search_tests(&catalogue_context, Some("left_only".to_string())).expect("unique search");
        assert_eq!(unique.len(), 1);
        assert!(unique[0].selectable);
        assert_eq!(unique[0].reference, "left_only");
        assert_eq!(unique[0].citation, "test:left_only");
        assert!(unique[0].advice.contains("accepted reference"));

        let TestCatalogueView {
            catalogue: missing,
            choices,
        } = search_tests(&catalogue_context, Some("not_listed".to_string()))
            .expect("missing search");
        assert_eq!(missing.state, aplan_app::TestCatalogueState::NoMatch);
        assert!(choices.is_empty());
        assert_eq!(missing.summary.matched_pairs, 0);

        let mut failed = runner("broken", "ignored");
        failed.list = failing_list_command();
        let partial_context = context(
            dir.path(),
            Some(aplan_config::TestRunners::Many(vec![
                runner("healthy", "known"),
                failed,
            ])),
        );
        let TestCatalogueView {
            catalogue: partial,
            choices,
        } = search_tests(&partial_context, Some("absent".to_string()))
            .expect("failed runner remains a typed partial answer");
        assert_eq!(partial.state, aplan_app::TestCatalogueState::Partial);
        assert_eq!(partial.summary.failed_runners, 1);
        assert!(!partial.findings.is_empty());
        assert!(choices.is_empty());
    }

    #[test]
    fn note_cardinality_and_verify_fix_review_are_fail_closed() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let context = context(dir.path(), None);
        let close = submission(
            form(&context, EvidenceWorkflow::Close, Some("A-1")).expect("close"),
            &[("note", "assertion"), ("no_resolve", "yes")],
        );
        assert!(
            request(EvidenceWorkflow::Close, &close)
                .expect_err("note without date refuses")
                .to_string()
                .contains("authored date")
        );

        let fix = submission(
            form(&context, EvidenceWorkflow::VerifyFix, None).expect("fix"),
            &[("review", "yes")],
        );
        assert!(
            request(EvidenceWorkflow::VerifyFix, &fix)
                .expect_err("explicit review word required")
                .to_string()
                .contains("type `review`")
        );

        let remove = submission(
            form(&context, EvidenceWorkflow::AmendRemove, Some("A-1")).expect("remove"),
            &[
                ("kind", "test"),
                ("reference", "suite::old"),
                ("no_resolve", "yes"),
            ],
        );
        let EvidenceRequest::Amend(remove) =
            request(EvidenceWorkflow::AmendRemove, &remove).expect("unchecked remove")
        else {
            panic!("remove must decode as an amendment")
        };
        assert!(remove.no_resolve);
    }

    #[test]
    fn rendered_preview_keeps_policy_exemptions_and_resolver_advice() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        write(
            &dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/**]\ntags: [untestable]\n---\n",
        );
        write(&dir.path().join("src/one.rs"), "fn one() {}\n");
        for args in [
            &["init", "-q"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "Test"][..],
            &["add", "."][..],
            &["commit", "-qm", "fixture"][..],
        ] {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .expect("git runs");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let options = aplan_core::Options {
            require_min_strength: Some(vec![EvidenceKind::Test]),
            exempt_tags: vec!["untestable".to_string()],
            ..aplan_core::Options::default()
        };
        let context = aplan_app::PlanContext::new(
            aplan_core::load_plan_identified(&dir.path().join("aplan")),
            aplan_app::ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: None,
                trust: aplan_app::Trust::RepositoryRead,
                options,
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(aplan_config::Config {
            test: Some(aplan_config::TestRunners::Many(vec![
                runner("left", "proof"),
                runner("right", "proof"),
            ])),
            ..aplan_config::Config::default()
        });
        let mut form = form(&context, EvidenceWorkflow::Close, Some("A-1")).expect("form");
        let editing = form.render(240, 2_000).rows.join("\n");
        assert!(editing.contains("required kinds: test"));
        assert!(editing.contains("exempt tags: untestable"));
        assert!(editing.contains("this task matches: untestable"));
        let field = form
            .fields()
            .iter()
            .position(|field| field.spec().key() == "test")
            .expect("test field");
        while form.focus() != FormFocus::Field(field) {
            let _ = form.reduce(FormAction::NextFocus);
        }
        let _ = form.reduce(FormAction::Insert("proof".to_string()));
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("form should request a preview")
        };
        let request_id = submission.request();
        let request = request(EvidenceWorkflow::Close, &submission).expect("request");
        let preview = request
            .preview(&context)
            .expect("inspectable denied preview");
        assert!(form.accept_preview(request_id, Ok(preview.into_change_set())));
        let rows = form.render(240, 2_000).rows.join("\n");
        assert!(rows.contains("Evidence policy: required kinds: test"));
        assert!(rows.contains("exempt tags: untestable"));
        assert!(rows.contains("matched exemptions: untestable"));
        assert!(rows.contains("Evidence resolver findings (1)"));
        assert!(rows.contains("APLAN013"));
        assert!(rows.contains("more than one runner"));
    }

    #[test]
    fn configured_resolution_starts_only_after_the_form_requests_a_preview() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let marker = dir.path().join("runner-was-started");
        let mut configured = runner("observable", "ignored");
        configured.list = marked_list_command(&marker, "proof");
        let _ = context(dir.path(), None);
        for args in [
            &["init", "-q"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "Test"][..],
            &["add", "."][..],
            &["commit", "-qm", "fixture"][..],
        ] {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .expect("git runs");
            assert!(output.status.success());
        }
        let context = context(dir.path(), Some(aplan_config::TestRunners::One(configured)));
        let mut form = form(&context, EvidenceWorkflow::Close, Some("A-1")).expect("form");
        let field = form
            .fields()
            .iter()
            .position(|field| field.spec().key() == "test")
            .expect("test field");
        while form.focus() != FormFocus::Field(field) {
            let _ = form.reduce(FormAction::NextFocus);
        }
        let _ = form.reduce(FormAction::Insert("proof".to_string()));
        assert!(!marker.exists(), "editing must not start configured work");
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("review should produce a typed request")
        };
        assert!(
            !marker.exists(),
            "the input reducer must hand work to the surrounding worker"
        );
        let request = request(EvidenceWorkflow::Close, &submission).expect("decode request");
        assert!(
            !marker.exists(),
            "decoding must remain local and non-blocking"
        );
        let _ = request.preview(&context).expect("preview result");
        assert!(
            marker.exists(),
            "only the preview service runs configured work"
        );
    }

    /// Proof for `close.focused_l3_terminal_outcomes_name_external_effects`:
    /// once Preview crosses the configured-command boundary, the retained
    /// review says that those effects are outside guarded publication.
    #[test]
    fn focused_close_execution_is_explicit_typed_and_preview_bounded() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let marker = dir.path().join("focused-runner-was-started");
        let _ = context(dir.path(), None);
        for args in [
            &["init", "-q"][..],
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "Test"][..],
            &["add", "."][..],
            &["commit", "-qm", "fixture"][..],
        ] {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .expect("git runs");
            assert!(output.status.success());
        }
        let mut configured = runner("focused", "proof");
        configured.run_selects = Some(true);
        configured.run = marked_run_command(&marker);
        let context = context_with_trust(
            dir.path(),
            Some(aplan_config::TestRunners::One(configured)),
            aplan_app::Trust::ConfiguredProcesses,
        );
        let focused = submission(
            form(&context, EvidenceWorkflow::Close, Some("A-1")).expect("close"),
            &[("test", "proof"), ("execute", "yes")],
        );
        assert!(
            !marker.exists(),
            "editing or Review started the test runner"
        );
        let focused_request = request(EvidenceWorkflow::Close, &focused).expect("focused request");
        let EvidenceRequest::Close(close) = &focused_request else {
            panic!("wrong request")
        };
        assert!(close.execute);
        assert!(!marker.exists(), "decoding started the test runner");
        let preview = focused_request.preview(&context).expect("focused preview");
        assert_eq!(
            preview.change_set().disposition(),
            aplan_app::ChangeDisposition::Ready
        );
        assert!(marker.exists(), "Preview did not start the selected runner");
        assert!(
            preview
                .notices()
                .iter()
                .any(|notice| notice.contains("effects are not rolled back")),
            "the interactive preview hid configured-command effects"
        );

        let conflict = submission(
            form(&context, EvidenceWorkflow::Close, Some("A-1")).expect("close"),
            &[("test", "proof"), ("no_resolve", "yes"), ("execute", "yes")],
        );
        assert!(
            request(EvidenceWorkflow::Close, &conflict)
                .expect_err("unresolved focused execution must conflict")
                .to_string()
                .contains("cannot run while evidence resolution is explicitly skipped")
        );
    }
}
