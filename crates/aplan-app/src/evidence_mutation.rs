//! Previewable completion and evidence-repair mutations.
//!
//! Evidence work is deliberately prepared from one identified [`PlanContext`]
//! and published only through [`crate::commit_change_set`]. Resolver findings,
//! skipped work and exact citation observations travel with the preview; a
//! delivery adapter never has to parse command output or silently treat an
//! unresolved citation as a successful one.

use std::collections::{BTreeMap, BTreeSet};

use aplan_core::{Evidence, EvidenceKind, Graph, Repair, Status, Task};

use crate::mutation::{TestExecutionCandidates, TestExecutionGuard};
use crate::{
    ChangeEvidence, ChangeSet, EvidenceRequirement, EvidenceReview, FileChange, PlanContext,
    SemanticEffect, VerificationState, VerificationWorkbench,
};

/// Close a live task with explicit citations, or derive `commit:HEAD` when the
/// list is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseRequest {
    pub id: String,
    pub evidence: Vec<Evidence>,
    pub no_resolve: bool,
    /// Explicitly execute the proposed closure's test citations at L3 before
    /// any plan bytes are published.
    pub execute: bool,
}

/// Deliberately refresh the trace and mutable-evidence baselines of one closed
/// task without changing its evidence or status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestampRequest {
    pub id: String,
}

/// One supported repair of a closed task's evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmendOperation {
    Add(Evidence),
    Remove(Evidence),
    Replace { from: Evidence, to: Evidence },
}

/// Amend one closed task without conflating the edit with restamping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendRequest {
    pub id: String,
    pub operation: AmendOperation,
    pub no_resolve: bool,
}

/// A guarded mutation plus evidence-specific review data.
#[derive(Debug)]
pub struct EvidenceMutationPreview {
    change: ChangeSet,
}

impl EvidenceMutationPreview {
    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    #[must_use]
    pub const fn requirement(&self) -> Option<&EvidenceRequirement> {
        self.change.evidence_review().requirement.as_ref()
    }

    #[must_use]
    pub fn resolver_findings(&self) -> &[aplan_core::Diagnostic] {
        &self.change.evidence_review().findings
    }

    #[must_use]
    pub fn notices(&self) -> &[String] {
        &self.change.evidence_review().notices
    }

    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvidenceMutationError {
    #[error("{0}")]
    Arguments(String),
    #[error("{0}")]
    Plan(String),
    #[error("{0}")]
    Refused(String),
    #[error(
        "{0}; `aplan` published no plan bytes. Effects of the explicitly launched configured test command are not rolled back"
    )]
    AfterFocusedExecution(String),
    #[error("`{id}`: {source}")]
    Rewrite {
        id: String,
        #[source]
        source: aplan_core::CloseError,
    },
    #[error("resolving evidence: {0}")]
    Evidence(#[from] aplan_evidence::Error),
}

/// Project the exact minimum-strength and exemption policy used by validation.
pub fn evidence_requirement(
    context: &PlanContext,
    id: &str,
) -> Result<EvidenceRequirement, EvidenceMutationError> {
    let loaded = unique_task(context, id)?;
    let (milestone, cut) = membership(context, id);
    let (required, exempt_tags) =
        context
            .options()
            .evidence_for(&loaded.task, cut.as_deref(), milestone.as_deref());
    let matched_exemptions = loaded
        .task
        .tags
        .iter()
        .filter(|tag| exempt_tags.contains(tag))
        .cloned()
        .collect();
    Ok(EvidenceRequirement {
        required: required.map(<[EvidenceKind]>::to_vec),
        exempt_tags: exempt_tags.to_vec(),
        matched_exemptions,
        cut,
        milestone,
    })
}

/// Prepare one closure without writing any plan file.
pub fn preview_close(
    context: &PlanContext,
    request: &CloseRequest,
) -> Result<EvidenceMutationPreview, EvidenceMutationError> {
    if request.execute && request.no_resolve {
        return Err(EvidenceMutationError::Arguments(
            "focused execution conflicts with unresolved mutation mode".to_string(),
        ));
    }
    if request.execute && context.identity().trust != crate::Trust::ConfiguredProcesses {
        return Err(EvidenceMutationError::Refused(
            "focused execution requires a context that explicitly permits configured processes"
                .to_string(),
        ));
    }
    require_clean(context)?;
    let loaded = unique_task(context, &request.id)?;
    let source = source(context, loaded)?;
    let evidence = if request.evidence.is_empty() {
        vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: aplan_evidence::head_sha(&context.identity().repository_root)?,
            captured: None,
        }]
    } else {
        request.evidence.clone()
    };
    validate_new_evidence(&evidence)?;
    if request.execute
        && !evidence
            .iter()
            .any(|citation| citation.kind == EvidenceKind::Test)
    {
        return Err(EvidenceMutationError::Arguments(
            "focused execution requires at least one `test` citation in the proposed closure"
                .to_string(),
        ));
    }

    let fingerprints = if context.options().detect_drift {
        aplan_core::trace_fingerprints(&loaded.task, context.options().traces_root.as_deref())
    } else {
        BTreeMap::new()
    };
    let (version, version_notice) = recorded_version(context, &loaded.task);
    let mut updated = aplan_core::close(&source, &evidence, &fingerprints, version.as_deref())
        .map_err(|source| rewrite(&request.id, source))?;
    let produced_for_resolution =
        aplan_core::mutation::task_of(&updated).map_err(EvidenceMutationError::Plan)?;
    let (resolution, execution_guard) =
        resolve_close_evidence(context, loaded, &produced_for_resolution.evidence, request)?;
    let evidence_fingerprints = if context.options().detect_drift && !request.no_resolve {
        aplan_evidence::update_evidence_fingerprints(
            &loaded.task.evidence,
            &loaded.task.evidence_fingerprints,
            &evidence,
            &context.identity().repository_root,
            false,
        )?
    } else {
        BTreeMap::new()
    };
    updated = aplan_core::set_evidence_fingerprints(&updated, &evidence_fingerprints)
        .map_err(|source| rewrite(&request.id, source))?;
    let produced = aplan_core::mutation::task_of(&updated).map_err(EvidenceMutationError::Plan)?;
    let after = aplan_core::mutation::with_replaced(context.plan(), &request.id, produced);
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied =
        !request.no_resolve && (resolution.denied || diagnostics.refuses(context.verdict()));
    let mut notices = resolution.notices;
    if let Some(notice) = version_notice {
        notices.push(notice);
    }
    let requirement = evidence_requirement(context, &request.id)?;
    let mut change = ChangeSet::from_parts(
        context.identity().clone(),
        vec![FileChange::replacement(
            loaded.path.clone(),
            source,
            updated,
        )],
        vec![SemanticEffect::TaskClosed {
            task: request.id.clone(),
            evidence,
        }],
        diagnostics,
        denied,
        resolution.evidence,
    )
    .with_evidence_review(EvidenceReview {
        requirement: Some(requirement),
        findings: resolution.findings,
        notices,
    });
    if request.execute {
        change = change.with_focused_test_execution();
    }
    if let Some(guard) = execution_guard {
        change = change.with_test_execution_guard(guard);
    }
    Ok(EvidenceMutationPreview { change })
}

/// Prepare one evidence amendment without changing trace baselines.
pub fn preview_amend(
    context: &PlanContext,
    request: &AmendRequest,
) -> Result<EvidenceMutationPreview, EvidenceMutationError> {
    require_clean(context)?;
    let loaded = unique_task(context, &request.id)?;
    let source = source(context, loaded)?;
    let (rewritten, amended, effect) = apply_amend(&source, &loaded.task, request)?;
    let resolution = resolve_amend(
        context,
        loaded,
        &loaded.task.evidence,
        &amended,
        request.no_resolve,
    )?;
    let evidence_fingerprints = maintained_fingerprints(
        context,
        request.no_resolve,
        &loaded.task.evidence,
        &loaded.task.evidence_fingerprints,
        &amended,
    )?;
    let rewritten = aplan_core::set_evidence_fingerprints(&rewritten, &evidence_fingerprints)
        .map_err(|source| rewrite(&request.id, source))?;
    let produced =
        aplan_core::mutation::task_of(&rewritten).map_err(EvidenceMutationError::Plan)?;
    let after = aplan_core::mutation::with_replaced(context.plan(), &request.id, produced);
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied =
        !request.no_resolve && (resolution.denied || diagnostics.refuses(context.verdict()));
    let requirement = evidence_requirement(context, &request.id)?;
    Ok(EvidenceMutationPreview {
        change: ChangeSet::from_parts(
            context.identity().clone(),
            vec![FileChange::replacement(
                loaded.path.clone(),
                source,
                rewritten,
            )],
            vec![effect],
            diagnostics,
            denied,
            resolution.evidence,
        )
        .with_evidence_review(EvidenceReview {
            requirement: Some(requirement),
            findings: resolution.findings,
            notices: resolution.notices,
        }),
    })
}

/// Prepare a deliberate trace and mutable-evidence baseline refresh.
pub fn preview_restamp(
    context: &PlanContext,
    request: &RestampRequest,
) -> Result<EvidenceMutationPreview, EvidenceMutationError> {
    require_clean(context)?;
    if !context.options().detect_drift {
        return Err(EvidenceMutationError::Refused(format!(
            "cannot restamp `{}` while detect_drift is disabled",
            request.id
        )));
    }
    let loaded = unique_task(context, &request.id)?;
    if loaded.task.status != Status::Done {
        return Err(EvidenceMutationError::Refused(format!(
            "cannot restamp `{}` because it is `{}` rather than `done`",
            request.id, loaded.task.status
        )));
    }
    let source = source(context, loaded)?;
    let trace_fingerprints =
        aplan_core::trace_fingerprints(&loaded.task, context.options().traces_root.as_deref());
    let evidence_fingerprints = aplan_evidence::update_evidence_fingerprints(
        &loaded.task.evidence,
        &loaded.task.evidence_fingerprints,
        &loaded.task.evidence,
        &context.identity().repository_root,
        true,
    )?;
    let trace_changes = fingerprint_changes(&loaded.task.trace_fingerprints, &trace_fingerprints);
    let evidence_changes =
        fingerprint_changes(&loaded.task.evidence_fingerprints, &evidence_fingerprints);
    let files = if trace_changes == 0 && evidence_changes == 0 {
        Vec::new()
    } else {
        let updated = aplan_core::restamp(&source, &trace_fingerprints)
            .and_then(|updated| {
                aplan_core::set_evidence_fingerprints(&updated, &evidence_fingerprints)
            })
            .map_err(|source| rewrite(&request.id, source))?;
        vec![FileChange::replacement(
            loaded.path.clone(),
            source,
            updated,
        )]
    };
    let diagnostics = if let Some(file) = files.first() {
        let task =
            aplan_core::mutation::task_of(file.after()).map_err(EvidenceMutationError::Plan)?;
        let after = aplan_core::mutation::with_replaced(context.plan(), &request.id, task);
        aplan_core::mutation::delta(context.plan(), &after, context.options())
    } else {
        aplan_core::mutation::Delta::default()
    };
    let denied = diagnostics.refuses(context.verdict());
    Ok(EvidenceMutationPreview {
        change: ChangeSet::from_parts(
            context.identity().clone(),
            files,
            vec![SemanticEffect::EvidenceAndTraceFingerprintsRestamped {
                task: request.id.clone(),
                trace_changes,
                evidence_changes,
            }],
            diagnostics,
            denied,
            ChangeEvidence::NotRequired,
        ),
    })
}

/// Stage every exact repair from one completed verification answer as one
/// guarded batch.
pub fn preview_verification_repairs(
    context: &PlanContext,
    verification: &VerificationWorkbench,
    options: &aplan_evidence::VerifyOptions,
) -> Result<EvidenceMutationPreview, EvidenceMutationError> {
    require_clean(context)?;
    if verification.state != VerificationState::Completed {
        return Err(EvidenceMutationError::Refused(
            "verification did not complete, so it cannot authorize repairs".to_string(),
        ));
    }
    let expected = crate::verification_run_sheet(context, options);
    if verification.sheet.input_sha256 != expected.input_sha256 {
        return Err(EvidenceMutationError::Refused(
            "verification inputs are stale; run verification again before reviewing repairs"
                .to_string(),
        ));
    }
    let grouped = repairs_by_task(verification)?;
    if grouped.is_empty() {
        return Ok(no_repair_preview(context));
    }

    let mut after = aplan_core::mutation::clone_of(context.plan());
    let mut files = Vec::new();
    let mut effects = Vec::new();
    let mut observations = Vec::new();
    let mut unresolved = Vec::new();
    let mut findings = Vec::new();
    let mut resolution_denied = false;
    for (id, repairs) in grouped {
        let loaded = unique_task(context, &id)?;
        let source = source(context, loaded)?;
        let before = loaded.task.evidence.clone();
        let mut updated = source.clone();
        for repair in &repairs {
            updated = aplan_core::apply_repair(&updated, repair)
                .map_err(|source| rewrite(&id, source))?;
            let Repair::ReplaceEvidence { from, to } = repair;
            effects.push(SemanticEffect::VerificationRepairApplied {
                task: id.clone(),
                from: from.clone(),
                to: to.clone(),
            });
        }
        let task = aplan_core::mutation::task_of(&updated).map_err(EvidenceMutationError::Plan)?;
        let resolution = resolve_amend(context, loaded, &before, &task.evidence, false)?;
        resolution_denied |= resolution.denied;
        findings.extend(resolution.findings);
        match resolution.evidence {
            ChangeEvidence::Resolution {
                unresolved: pending,
                observed,
            } => {
                unresolved.extend(pending);
                observations.extend(observed);
            }
            ChangeEvidence::NotRequired | ChangeEvidence::Skipped { .. } => {
                return Err(EvidenceMutationError::Plan(
                    "resolved verification repairs returned no resolution ledger".to_string(),
                ));
            }
        }
        let fingerprints = maintained_fingerprints(
            context,
            false,
            &before,
            &loaded.task.evidence_fingerprints,
            &task.evidence,
        )?;
        updated = aplan_core::set_evidence_fingerprints(&updated, &fingerprints)
            .map_err(|source| rewrite(&id, source))?;
        let task = aplan_core::mutation::task_of(&updated).map_err(EvidenceMutationError::Plan)?;
        after = aplan_core::mutation::with_replaced(&after, &id, task);
        files.push(FileChange::replacement(
            loaded.path.clone(),
            source,
            updated,
        ));
    }
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied = resolution_denied || diagnostics.refuses(context.verdict());
    Ok(EvidenceMutationPreview {
        change: ChangeSet::from_parts(
            context.identity().clone(),
            files,
            effects,
            diagnostics,
            denied,
            ChangeEvidence::Resolution {
                unresolved,
                observed: observations,
            },
        )
        .with_evidence_review(EvidenceReview {
            requirement: None,
            findings,
            notices: Vec::new(),
        }),
    })
}

fn no_repair_preview(context: &PlanContext) -> EvidenceMutationPreview {
    EvidenceMutationPreview {
        change: ChangeSet::from_parts(
            context.identity().clone(),
            Vec::new(),
            Vec::new(),
            aplan_core::mutation::Delta::default(),
            false,
            ChangeEvidence::NotRequired,
        )
        .with_evidence_review(EvidenceReview {
            requirement: None,
            findings: Vec::new(),
            notices: vec!["verification produced no machine-applicable repairs".to_string()],
        }),
    }
}

fn repairs_by_task(
    verification: &VerificationWorkbench,
) -> Result<BTreeMap<String, Vec<Repair>>, EvidenceMutationError> {
    let mut grouped = BTreeMap::<String, Vec<Repair>>::new();
    for repair in &verification.repairs {
        let id = repair.task.clone().ok_or_else(|| {
            EvidenceMutationError::Plan("a machine-applicable repair has no task".to_string())
        })?;
        let item = Repair::ReplaceEvidence {
            from: evidence_from_parts(&repair.from_kind, &repair.from_reference, true)?,
            to: evidence_from_parts(&repair.to_kind, &repair.to_reference, false)?,
        };
        let repairs = grouped.entry(id).or_default();
        if !repairs.contains(&item) {
            repairs.push(item);
        }
    }
    Ok(grouped)
}

struct Resolution {
    evidence: ChangeEvidence,
    findings: Vec<aplan_core::Diagnostic>,
    notices: Vec<String>,
    denied: bool,
}

fn resolve_close_evidence(
    context: &PlanContext,
    loaded: &aplan_core::load::LoadedTask,
    evidence: &[Evidence],
    request: &CloseRequest,
) -> Result<(Resolution, Option<TestExecutionGuard>), EvidenceMutationError> {
    let execution_candidates = if request.execute {
        let runners = context
            .report_config()
            .and_then(|config| config.test.as_ref())
            .ok_or_else(|| {
                EvidenceMutationError::Refused(
                    "focused execution requires a configured test runner".to_string(),
                )
            })?;
        Some(TestExecutionCandidates::capture(
            &context.identity().repository_root,
            runners,
        ))
    } else {
        None
    };
    let resolution = resolve(
        context,
        loaded,
        evidence,
        request.no_resolve,
        request.execute,
    )?;
    let guard = if !request.execute || resolution.denied {
        None
    } else {
        let selected = match &resolution.evidence {
            ChangeEvidence::Resolution { observed, .. } => observed
                .iter()
                .filter(|citation| citation.kind == EvidenceKind::Test)
                .map(|citation| citation.resolution.resolver.clone())
                .collect::<BTreeSet<_>>(),
            ChangeEvidence::NotRequired | ChangeEvidence::Skipped { .. } => BTreeSet::new(),
        };
        Some(
            execution_candidates
                .expect("focused execution captured configured runners above")
                .select(&selected)
                .map_err(|error| {
                    EvidenceMutationError::Refused(format!(
                        "focused execution cannot identify selected provider material: {error}"
                    ))
                })?,
        )
    };
    if let Some(reason) = guard.as_ref().and_then(TestExecutionGuard::movement) {
        return Err(EvidenceMutationError::AfterFocusedExecution(format!(
            "focused execution became stale before review: {reason}"
        )));
    }
    Ok((resolution, guard))
}

fn resolve(
    context: &PlanContext,
    loaded: &aplan_core::load::LoadedTask,
    evidence: &[Evidence],
    skip: bool,
    execute: bool,
) -> Result<Resolution, EvidenceMutationError> {
    if skip {
        return Ok(Resolution {
            evidence: ChangeEvidence::Skipped {
                citations: evidence.to_vec(),
                reason: "the user explicitly selected the unresolved mutation mode".to_string(),
            },
            findings: Vec::new(),
            notices: vec![
                "evidence will be recorded unchecked; verify it before trusting the mutation"
                    .to_string(),
            ],
            denied: false,
        });
    }
    let answer = aplan_evidence::resolve_proposed(
        &loaded.task,
        &loaded.path,
        evidence,
        &context.identity().repository_root,
        &close_options(context, execute),
    )?;
    let unresolved = unresolved(evidence, &answer.unchecked);
    let mut notices = Vec::new();
    if !unresolved.is_empty() {
        notices.push(if answer.repository_unreadable {
            "some evidence is unresolved because the selected path is not a readable repository"
                .to_string()
        } else {
            "some evidence is unresolved because no close-time resolver is configured for it"
                .to_string()
        });
    }
    let expected_tests = evidence
        .iter()
        .filter(|citation| citation.kind == EvidenceKind::Test)
        .count();
    let observed_tests = answer
        .citations
        .iter()
        .filter(|citation| citation.kind == EvidenceKind::Test)
        .collect::<Vec<_>>();
    let l3_incomplete = execute
        && (observed_tests.len() != expected_tests
            || observed_tests.iter().any(|citation| {
                citation.resolution.strength != aplan_evidence::CitationStrength::Executed
                    || citation.resolution.outcome != aplan_evidence::CitationOutcome::Resolved
            }));
    if execute {
        let distinct = answer
            .citations
            .iter()
            .filter(|citation| citation.kind == EvidenceKind::Test)
            .map(|citation| {
                (
                    citation.resolution.resolver.clone(),
                    citation.reference.clone(),
                )
            })
            .collect::<BTreeSet<_>>()
            .len();
        notices.push(if l3_incomplete {
            "focused L3 execution did not prove every proposed test citation; L2 listing resolution is not enough to close"
                .to_string()
        } else {
            format!(
                "focused L3 execution proved {distinct} distinct configured runner/test identity(ies)"
            )
        });
        notices.push(
            "configured test commands are trusted code whose effects are not rolled back; inspect the working tree after focused L3 execution"
                .to_string(),
        );
    }
    Ok(Resolution {
        denied: answer.refuses(context.verdict()) || l3_incomplete,
        findings: answer.report.diagnostics,
        notices,
        evidence: ChangeEvidence::Resolution {
            unresolved,
            observed: answer.citations,
        },
    })
}

fn resolve_amend(
    context: &PlanContext,
    loaded: &aplan_core::load::LoadedTask,
    before: &[Evidence],
    after: &[Evidence],
    skip: bool,
) -> Result<Resolution, EvidenceMutationError> {
    if skip {
        return resolve(context, loaded, after, true, false);
    }
    let options = close_options(context, false);
    let existing = aplan_evidence::resolve_proposed(
        &loaded.task,
        &loaded.path,
        before,
        &context.identity().repository_root,
        &options,
    )?;
    let proposed = aplan_evidence::resolve_proposed(
        &loaded.task,
        &loaded.path,
        after,
        &context.identity().repository_root,
        &options,
    )?;
    let old = existing
        .report
        .diagnostics
        .iter()
        .map(diagnostic_identity)
        .collect::<BTreeSet<_>>();
    let findings = proposed
        .report
        .diagnostics
        .into_iter()
        .filter(|finding| !old.contains(&diagnostic_identity(finding)))
        .collect::<Vec<_>>();
    let introduced = after
        .iter()
        .filter(|citation| !before.contains(citation))
        .cloned()
        .collect::<Vec<_>>();
    let unresolved = unresolved(&introduced, &proposed.unchecked);
    let mut notices = Vec::new();
    if !unresolved.is_empty() {
        notices.push(if proposed.repository_unreadable {
            "introduced evidence is unresolved because the selected path is not a readable repository"
                .to_string()
        } else {
            "introduced evidence is unresolved because no close-time resolver is configured for it"
                .to_string()
        });
    }
    Ok(Resolution {
        denied: !findings.is_empty(),
        findings,
        notices,
        evidence: ChangeEvidence::Resolution {
            unresolved,
            observed: proposed.citations,
        },
    })
}

fn diagnostic_identity(diagnostic: &aplan_core::Diagnostic) -> (String, String) {
    (diagnostic.code.to_string(), diagnostic.message.clone())
}

fn unresolved(evidence: &[Evidence], kinds: &[EvidenceKind]) -> Vec<Evidence> {
    evidence
        .iter()
        .filter(|citation| kinds.contains(&citation.kind))
        .cloned()
        .collect()
}

pub(crate) fn close_options(context: &PlanContext, execute: bool) -> aplan_evidence::VerifyOptions {
    let config = context.report_config();
    aplan_evidence::VerifyOptions {
        traces_root: None,
        detect_drift: false,
        execute,
        test_runner: config.and_then(|config| config.test.clone()),
        max_unrelated_files: config
            .and_then(|config| config.evidence.as_ref())
            .and_then(|evidence| evidence.max_unrelated_files),
        observation_inputs: config
            .and_then(|config| config.evidence.as_ref())
            .and_then(|evidence| evidence.observation_inputs.clone()),
        url_check: None,
        config_file: None,
        version: None,
    }
}

fn apply_amend(
    source: &str,
    task: &Task,
    request: &AmendRequest,
) -> Result<(String, Vec<Evidence>, SemanticEffect), EvidenceMutationError> {
    match &request.operation {
        AmendOperation::Add(add) => {
            validate_new_evidence(std::slice::from_ref(add))?;
            let mut evidence = task.evidence.clone();
            evidence.push(add.clone());
            let rewritten = aplan_core::add_evidence(source, add)
                .map_err(|source| rewrite(&request.id, source))?;
            Ok((
                rewritten,
                evidence,
                SemanticEffect::EvidenceAdded {
                    task: request.id.clone(),
                    evidence: add.clone(),
                },
            ))
        }
        AmendOperation::Remove(remove) => {
            let rewritten = aplan_core::remove_evidence(source, remove)
                .map_err(|source| rewrite(&request.id, source))?;
            let evidence = aplan_core::mutation::task_of(&rewritten)
                .map_err(EvidenceMutationError::Plan)?
                .evidence;
            Ok((
                rewritten,
                evidence,
                SemanticEffect::EvidenceRemoved {
                    task: request.id.clone(),
                    evidence: remove.clone(),
                },
            ))
        }
        AmendOperation::Replace { from, to } => {
            validate_new_evidence(std::slice::from_ref(to))?;
            let rewritten = aplan_core::replace_evidence(source, from, to)
                .map_err(|source| rewrite(&request.id, source))?;
            let evidence = aplan_core::mutation::task_of(&rewritten)
                .map_err(EvidenceMutationError::Plan)?
                .evidence;
            Ok((
                rewritten,
                evidence,
                SemanticEffect::EvidenceReplaced {
                    task: request.id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                },
            ))
        }
    }
}

fn validate_new_evidence(evidence: &[Evidence]) -> Result<(), EvidenceMutationError> {
    for citation in evidence {
        if aplan_core::task::is_vacuous_ref(&citation.reference) {
            return Err(EvidenceMutationError::Arguments(format!(
                "{} evidence needs a non-blank reference",
                citation.kind.as_str()
            )));
        }
        match citation.kind.capabilities().capture {
            aplan_core::task::EvidenceCapture::AuthoredDate { .. } => {
                let valid = citation.captured.as_deref().is_some_and(valid_date);
                if !valid {
                    return Err(EvidenceMutationError::Arguments(
                        "note evidence needs its authored date as YYYY-MM-DD; the interface will not invent it"
                            .to_string(),
                    ));
                }
            }
            aplan_core::task::EvidenceCapture::NotApplicable { .. } => {
                if citation.captured.is_some() {
                    return Err(EvidenceMutationError::Arguments(format!(
                        "{} evidence cannot carry an author-entered date",
                        citation.kind.as_str()
                    )));
                }
            }
        }
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    aplan_core::is_iso_date(value)
}

fn evidence_from_parts(
    kind: &str,
    reference: &str,
    naming_existing: bool,
) -> Result<Evidence, EvidenceMutationError> {
    let kind = EvidenceKind::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == kind)
        .ok_or_else(|| EvidenceMutationError::Plan(format!("unknown evidence kind `{kind}`")))?;
    if !naming_existing && kind == EvidenceKind::Note {
        return Err(EvidenceMutationError::Plan(
            "a computed repair cannot invent the authored date for note evidence".to_string(),
        ));
    }
    let evidence = Evidence {
        kind,
        reference: reference.to_string(),
        captured: None,
    };
    // The verifier may name malformed evidence that is already recorded so it
    // can repair it, but the replacement is a new citation and crosses the
    // same validation boundary as close/amend. VerificationWorkbench is a
    // public application value; callers must not be able to manufacture a
    // stronger repair authority by editing its fields.
    if !naming_existing {
        validate_new_evidence(std::slice::from_ref(&evidence))?;
    }
    Ok(evidence)
}

fn maintained_fingerprints(
    context: &PlanContext,
    no_resolve: bool,
    before: &[Evidence],
    existing: &BTreeMap<String, String>,
    after: &[Evidence],
) -> Result<BTreeMap<String, String>, EvidenceMutationError> {
    if context.options().detect_drift && !no_resolve {
        Ok(aplan_evidence::update_evidence_fingerprints(
            before,
            existing,
            after,
            &context.identity().repository_root,
            false,
        )?)
    } else {
        Ok(aplan_evidence::retained_evidence_fingerprints(
            existing, after,
        ))
    }
}

fn recorded_version(context: &PlanContext, task: &Task) -> (Option<String>, Option<String>) {
    if !task.is_release() {
        return (None, None);
    }
    let policy = context
        .report_config()
        .and_then(|config| config.version.clone())
        .unwrap_or_default();
    let tasks = context.plan().tasks.iter().map(|loaded| &loaded.task);
    let graph = Graph::new(tasks);
    match aplan_evidence::version::version_for_release(
        &context.identity().repository_root,
        context.plan(),
        &graph,
        &policy,
    ) {
        Ok(version) => (Some(version), None),
        Err(reason) => (
            None,
            Some(format!(
                "this release closes without recording the version it produced: {reason}"
            )),
        ),
    }
}

fn membership(context: &PlanContext, id: &str) -> (Option<String>, Option<String>) {
    let mut membership = (None, None);
    for loaded in context
        .plan()
        .tasks
        .iter()
        .filter(|loaded| loaded.task.is_milestone())
    {
        if loaded.task.blocked_by.iter().any(|member| member == id) {
            membership = (
                Some(loaded.task.id.clone()),
                Some(aplan_core::cut_label(&loaded.task).to_string()),
            );
        }
    }
    membership
}

fn fingerprint_changes(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> usize {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .count()
}

fn require_clean(context: &PlanContext) -> Result<(), EvidenceMutationError> {
    if context.plan().load_diagnostics.is_empty() {
        Ok(())
    } else {
        Err(EvidenceMutationError::Plan(
            "the plan does not load cleanly; repair it before changing evidence".to_string(),
        ))
    }
}

fn unique_task<'a>(
    context: &'a PlanContext,
    id: &str,
) -> Result<&'a aplan_core::load::LoadedTask, EvidenceMutationError> {
    let mut matches = context
        .plan()
        .tasks
        .iter()
        .filter(|task| task.task.id == id);
    let Some(task) = matches.next() else {
        return Err(EvidenceMutationError::Arguments(format!(
            "no task `{id}` in this plan"
        )));
    };
    if matches.next().is_some() {
        return Err(EvidenceMutationError::Refused(format!(
            "task id `{id}` is ambiguous; repair the duplicate before mutating it"
        )));
    }
    Ok(task)
}

fn source(
    context: &PlanContext,
    loaded: &aplan_core::load::LoadedTask,
) -> Result<String, EvidenceMutationError> {
    context
        .source_text(&loaded.path)
        .map(str::to_string)
        .ok_or_else(|| {
            EvidenceMutationError::Refused(format!(
                "`{}` changed while its preview was being prepared; reload and try again",
                loaded.task.id
            ))
        })
}

fn rewrite(id: &str, source: aplan_core::CloseError) -> EvidenceMutationError {
    EvidenceMutationError::Rewrite {
        id: id.to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::{
        ChangeDisposition, CommitOutcome, ContextInputs, Trust, commit_change_set, verify_context,
    };

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new(task: &str) -> Self {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("tempdir");
            write(&dir.path().join("aplan/tasks/A-1.md"), task);
            write(&dir.path().join("src/one.rs"), "pub fn one() {}\n");
            write(&dir.path().join("src/other.rs"), "pub fn other() {}\n");
            write(
                &dir.path().join("proof.md"),
                "# Proof\n\n## Contract\n\nHolds.\n",
            );
            git(dir.path(), &["init", "-q"]);
            git(dir.path(), &["config", "user.email", "test@example.com"]);
            git(dir.path(), &["config", "user.name", "Test"]);
            git(dir.path(), &["add", "."]);
            git(dir.path(), &["commit", "-qm", "fixture"]);
            Self { dir }
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn task_path(&self) -> PathBuf {
            self.root().join("aplan/tasks/A-1.md")
        }

        fn commit_other(&self) -> String {
            write(
                &self.root().join("src/other.rs"),
                "pub fn other() { println!(\"x\"); }\n",
            );
            git(self.root(), &["add", "src/other.rs"]);
            git(self.root(), &["commit", "-qm", "other"]);
            git_output(self.root(), &["rev-parse", "HEAD"])
        }
    }

    fn live_task() -> &'static str {
        "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/one.rs]\n---\n\n## Acceptance criteria\n\n- [x] proved\n"
    }

    fn done_task(reference: &str) -> String {
        format!(
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: done\neffort_days: 1\npaths: [src/one.rs]\nevidence:\n  - kind: test\n    ref: {reference}\n---\n\n## Acceptance criteria\n\n- [x] proved\n"
        )
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, text).expect("write fixture");
    }

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git runs");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_string()
    }

    fn runner(name: &str, listed: &str) -> aplan_config::TestRunner {
        aplan_config::TestRunner {
            weight: None,
            exclusive: None,
            name: Some(name.to_string()),
            dir: None,
            covers: Some(vec!["src/**".to_string()]),
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
            list: vec![
                "git".to_string(),
                "-c".to_string(),
                format!("aplan.tests={listed}"),
                "config".to_string(),
                "--get".to_string(),
                "aplan.tests".to_string(),
            ],
            run: vec![
                "git".to_string(),
                "--version".to_string(),
                "{test}".to_string(),
            ],
            batch: None,
            timeout_seconds: None,
        }
    }

    fn fixture_context(
        fixture: &Fixture,
        options: aplan_core::Options,
        runners: Option<aplan_config::TestRunners>,
    ) -> PlanContext {
        let config = aplan_config::Config {
            test: runners,
            ..aplan_config::Config::default()
        };
        PlanContext::new(
            aplan_core::load_plan_identified(&fixture.root().join("aplan")),
            ContextInputs {
                repository_root: fixture.root().to_path_buf(),
                configuration_file: None,
                trust: if config.test.is_some() {
                    Trust::ConfiguredProcesses
                } else {
                    Trust::RepositoryRead
                },
                options,
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(config)
    }

    fn test_options() -> aplan_core::Options {
        aplan_core::Options {
            require_min_strength: Some(vec![EvidenceKind::Test]),
            exempt_tags: vec!["untestable".to_string()],
            ..aplan_core::Options::default()
        }
    }

    #[test]
    fn close_previews_policy_resolution_exact_bytes_and_stale_publication() {
        let applied = Fixture::new(live_task());
        let applied_context = fixture_context(
            &applied,
            test_options(),
            Some(aplan_config::TestRunners::One(runner(
                "unit",
                "suite::proof",
            ))),
        );
        let applied_preview = preview_close(
            &applied_context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![Evidence {
                    kind: EvidenceKind::Test,
                    reference: "suite::proof".to_string(),
                    captured: None,
                }],
                no_resolve: false,
                execute: false,
            },
        )
        .expect("applied close preview");
        assert!(matches!(
            commit_change_set(&applied_context, applied_preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        let applied_text = std::fs::read_to_string(applied.task_path()).expect("applied task");
        assert!(applied_text.contains("status: done"));
        assert!(applied_text.contains("ref: suite::proof"));

        let fixture = Fixture::new(live_task());
        let context = fixture_context(
            &fixture,
            test_options(),
            Some(aplan_config::TestRunners::One(runner(
                "unit",
                "suite::proof",
            ))),
        );
        let preview = preview_close(
            &context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![Evidence {
                    kind: EvidenceKind::Test,
                    reference: "suite::proof".to_string(),
                    captured: None,
                }],
                no_resolve: false,
                execute: false,
            },
        )
        .expect("close preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        assert_eq!(
            preview.requirement().expect("requirement").required,
            Some(vec![EvidenceKind::Test])
        );
        assert!(preview.resolver_findings().is_empty());
        let ChangeEvidence::Resolution {
            unresolved,
            observed,
        } = preview.change_set().evidence()
        else {
            panic!("close must carry a resolution ledger")
        };
        assert!(unresolved.is_empty());
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].reference, "suite::proof");
        let after = preview.change_set().files()[0].after();
        assert!(after.contains("status: done"));
        assert!(after.contains("ref: suite::proof"));

        write(
            &fixture.task_path(),
            &live_task().replace("title: one", "title: concurrent"),
        );
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Stale { .. }
        ));
        assert!(
            std::fs::read_to_string(fixture.task_path())
                .expect("task")
                .contains("title: concurrent")
        );
    }

    #[test]
    fn focused_l3_preview_is_typed_and_provider_or_source_movement_is_stale() {
        let fixture = Fixture::new(live_task());
        let support = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("provider support");
        let provider_input = support.path().join("provider.input");
        write(&provider_input, "provider version one\n");
        let mut configured = runner("unit", "suite::proof");
        configured.run = vec![
            "git".to_string(),
            "rev-parse".to_string(),
            "--sq-quote".to_string(),
            "{test}".to_string(),
            provider_input.display().to_string(),
        ];
        configured.run_selects = Some(true);
        let context = fixture_context(
            &fixture,
            test_options(),
            Some(aplan_config::TestRunners::One(configured)),
        );
        let request = CloseRequest {
            id: "A-1".to_string(),
            evidence: vec![Evidence {
                kind: EvidenceKind::Test,
                reference: "suite::proof".to_string(),
                captured: None,
            }],
            no_resolve: false,
            execute: true,
        };
        let preview = preview_close(&context, &request).expect("focused preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        assert!(preview.change_set().focused_test_execution());
        let ChangeEvidence::Resolution { observed, .. } = preview.change_set().evidence() else {
            panic!("focused preview lost its typed resolution ledger")
        };
        assert!(matches!(
            observed.as_slice(),
            [observation]
                if observation.resolution.strength == aplan_evidence::CitationStrength::Executed
                    && observation.resolution.outcome
                        == aplan_evidence::CitationOutcome::Resolved
        ));
        let before = std::fs::read_to_string(fixture.task_path()).expect("task before stale run");
        write(&provider_input, "provider version two\n");
        let outcome = commit_change_set(&context, preview.into_change_set());
        let CommitOutcome::Stale { change, .. } = outcome else {
            panic!("provider movement was not stale: {outcome:?}")
        };
        assert!(
            change
                .stale_reason()
                .is_some_and(|reason| reason.contains("provider runtime material"))
        );
        assert_eq!(
            std::fs::read_to_string(fixture.task_path()).expect("task after provider movement"),
            before
        );

        let source_fixture = Fixture::new(live_task());
        write(
            &source_fixture.root().join("src/one.rs"),
            "pub fn one() { println!(\"before\"); }\n",
        );
        let mut configured = runner("unit", "suite::proof");
        configured.run_selects = Some(true);
        let source_context = fixture_context(
            &source_fixture,
            test_options(),
            Some(aplan_config::TestRunners::One(configured)),
        );
        let source_preview =
            preview_close(&source_context, &request).expect("dirty-source focused preview");
        write(
            &source_fixture.root().join("src/one.rs"),
            "pub fn one() { println!(\"after!\"); }\n",
        );
        assert!(matches!(
            commit_change_set(&source_context, source_preview.into_change_set()),
            CommitOutcome::Stale { .. }
        ));
        assert!(
            std::fs::read_to_string(source_fixture.task_path())
                .expect("task after source movement")
                .contains("status: todo")
        );
    }

    #[test]
    fn focused_l3_requires_explicit_process_trust_and_exact_selection_proof() {
        let fixture = Fixture::new(live_task());
        let configured = aplan_config::TestRunners::One(runner("unit", "suite::proof"));
        let config = aplan_config::Config {
            test: Some(configured.clone()),
            ..aplan_config::Config::default()
        };
        let untrusted = PlanContext::new(
            aplan_core::load_plan_identified(&fixture.root().join("aplan")),
            ContextInputs {
                repository_root: fixture.root().to_path_buf(),
                configuration_file: None,
                trust: Trust::RepositoryRead,
                options: test_options(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(config);
        let request = CloseRequest {
            id: "A-1".to_string(),
            evidence: vec![Evidence {
                kind: EvidenceKind::Test,
                reference: "suite::proof".to_string(),
                captured: None,
            }],
            no_resolve: false,
            execute: true,
        };
        assert!(
            preview_close(&untrusted, &request)
                .expect_err("repository-only trust started a command")
                .to_string()
                .contains("explicitly permits configured processes")
        );

        let mut conflicting = request.clone();
        conflicting.no_resolve = true;
        assert!(
            preview_close(&untrusted, &conflicting)
                .expect_err("focused unresolved mode must conflict before trust is consulted")
                .to_string()
                .contains("conflicts with unresolved mutation mode")
        );

        let without_test = CloseRequest {
            evidence: vec![Evidence {
                kind: EvidenceKind::File,
                reference: "src/one.rs".to_string(),
                captured: None,
            }],
            ..request.clone()
        };
        let trusted_without_runner = PlanContext::new(
            aplan_core::load_plan_identified(&fixture.root().join("aplan")),
            ContextInputs {
                repository_root: fixture.root().to_path_buf(),
                configuration_file: None,
                trust: Trust::ConfiguredProcesses,
                options: test_options(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(aplan_config::Config::default());
        assert!(
            preview_close(&trusted_without_runner, &without_test)
                .expect_err("focused close without a test citation must refuse")
                .to_string()
                .contains("requires at least one `test` citation")
        );
        assert!(
            preview_close(&trusted_without_runner, &request)
                .expect_err("focused close without a configured runner must refuse")
                .to_string()
                .contains("requires a configured test runner")
        );

        // With no `run_selects` declaration, a zero exit status is still only
        // an L2 answer when it is indistinguishable from the miss probe. This
        // remains a hard closure refusal even when warnings do not block.
        let context = fixture_context(&fixture, test_options(), Some(configured));
        let preview = preview_close(&context, &request).expect("inspectable selection refusal");
        assert_eq!(
            preview.change_set().disposition(),
            ChangeDisposition::Denied
        );
        assert!(preview.change_set().focused_test_execution());
        assert!(preview.resolver_findings().iter().any(|finding| {
            finding.code == "APLAN037"
                && finding
                    .agent_instruction
                    .as_deref()
                    .is_some_and(|instruction| instruction.contains("run_selects"))
        }));
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Denied(_)
        ));
        assert!(
            std::fs::read_to_string(fixture.task_path())
                .expect("task after selection refusal")
                .contains("status: todo")
        );
    }

    /// Proof for `tui.evidence_mutation_previews_bind_identified_inputs`:
    /// repository
    /// state and selected configuration semantics are part of the same stale
    /// boundary as plan bytes, even when the task file itself is untouched.
    #[test]
    fn evidence_mutation_preview_refuses_repository_and_configuration_only_races() {
        let fixture = Fixture::new(live_task());
        let repository_context = fixture_context(&fixture, aplan_core::Options::default(), None);
        let request = CloseRequest {
            id: "A-1".to_string(),
            evidence: vec![Evidence {
                kind: EvidenceKind::Note,
                reference: "reviewed".to_string(),
                captured: Some("2026-10-24".to_string()),
            }],
            no_resolve: true,
            execute: false,
        };
        let repository_preview =
            preview_close(&repository_context, &request).expect("repository preview");
        write(
            &fixture.root().join("src/other.rs"),
            "pub fn other() { println!(\"changed\"); }\n",
        );
        assert!(matches!(
            commit_change_set(&repository_context, repository_preview.into_change_set()),
            CommitOutcome::Stale { .. }
        ));
        git(fixture.root(), &["checkout", "--", "src/other.rs"]);

        let config_dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("config tempdir");
        let config_path = config_dir.path().join("selected.yml");
        write(&config_path, "deny_warnings: false\n");
        let (config, _) = aplan_config::load_from(
            &aplan_config::ConfigSource::File(config_path.clone()),
            config_dir.path(),
        )
        .expect("config");
        let configuration_context = PlanContext::new(
            aplan_core::load_plan_identified(&fixture.root().join("aplan")),
            ContextInputs {
                repository_root: fixture.root().to_path_buf(),
                configuration_file: Some(config_path.clone()),
                trust: Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(config);
        let configuration_preview =
            preview_close(&configuration_context, &request).expect("configuration preview");
        write(&config_path, "deny_warnings: true\n");
        assert!(matches!(
            commit_change_set(
                &configuration_context,
                configuration_preview.into_change_set()
            ),
            CommitOutcome::Stale { .. }
        ));
        assert!(
            std::fs::read_to_string(fixture.task_path())
                .expect("task")
                .contains("status: todo")
        );
    }

    #[test]
    fn close_refuses_ambiguous_tests_irrelevant_commits_and_terminal_tasks() {
        let fixture = Fixture::new(live_task());
        let ambiguous = fixture_context(
            &fixture,
            test_options(),
            Some(aplan_config::TestRunners::Many(vec![
                runner("left", "proof"),
                runner("right", "proof"),
            ])),
        );
        let ambiguous_preview = preview_close(
            &ambiguous,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![Evidence {
                    kind: EvidenceKind::Test,
                    reference: "proof".to_string(),
                    captured: None,
                }],
                no_resolve: false,
                execute: false,
            },
        )
        .expect("denied preview remains inspectable");
        assert_eq!(
            ambiguous_preview.change_set().disposition(),
            ChangeDisposition::Denied
        );
        assert!(ambiguous_preview.resolver_findings().iter().any(|finding| {
            finding.code == "APLAN013" && finding.message.contains("more than one runner")
        }));

        let irrelevant = fixture.commit_other();
        let commit_context = fixture_context(&fixture, aplan_core::Options::default(), None);
        let commit_preview = preview_close(
            &commit_context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![Evidence {
                    kind: EvidenceKind::Commit,
                    reference: irrelevant,
                    captured: None,
                }],
                no_resolve: false,
                execute: false,
            },
        )
        .expect("irrelevant commit is a denied preview");
        assert_eq!(
            commit_preview.change_set().disposition(),
            ChangeDisposition::Denied
        );
        assert!(
            commit_preview
                .resolver_findings()
                .iter()
                .any(|finding| finding.code == "APLAN012")
        );

        write(&fixture.task_path(), &done_task("proof"));
        let terminal = fixture_context(&fixture, aplan_core::Options::default(), None);
        let error = preview_close(
            &terminal,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: Vec::new(),
                no_resolve: false,
                execute: false,
            },
        )
        .expect_err("already done refuses before resolver work");
        assert!(error.to_string().contains("already"));
    }

    #[test]
    fn no_resolve_and_note_provenance_are_explicit_not_false_successes() {
        let fixture = Fixture::new(live_task());
        let context = fixture_context(&fixture, test_options(), None);
        let note = Evidence {
            kind: EvidenceKind::Note,
            reference: "reviewed with release owner".to_string(),
            captured: Some("2026-10-24".to_string()),
        };
        let preview = preview_close(
            &context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![note.clone()],
                no_resolve: true,
                execute: false,
            },
        )
        .expect("explicit unresolved preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        assert!(matches!(
            preview.change_set().evidence(),
            ChangeEvidence::Skipped { citations, .. } if citations == std::slice::from_ref(&note)
        ));
        assert!(preview.notices()[0].contains("unchecked"));

        for captured in [None, Some("2026-02-29".to_string())] {
            let error = preview_close(
                &context,
                &CloseRequest {
                    id: "A-1".to_string(),
                    evidence: vec![Evidence {
                        kind: EvidenceKind::Note,
                        reference: "assertion".to_string(),
                        captured,
                    }],
                    no_resolve: true,
                    execute: false,
                },
            )
            .expect_err("invented or impossible date refuses");
            assert!(error.to_string().contains("authored date"));
        }

        for reference in ["", " ", "\u{200b}"] {
            let error = preview_close(
                &context,
                &CloseRequest {
                    id: "A-1".to_string(),
                    evidence: vec![Evidence {
                        kind: EvidenceKind::Commit,
                        reference: reference.to_string(),
                        captured: None,
                    }],
                    no_resolve: true,
                    execute: false,
                },
            )
            .expect_err("vacuous evidence refuses even in unchecked mode");
            assert!(error.to_string().contains("non-blank reference"));
        }
    }

    #[test]
    fn default_head_and_unresolved_url_are_concrete_review_states() {
        let fixture = Fixture::new(live_task());
        let context = fixture_context(&fixture, aplan_core::Options::default(), None);
        let expected_head = git_output(fixture.root(), &["rev-parse", "HEAD"]);
        let default = preview_close(
            &context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: Vec::new(),
                no_resolve: false,
                execute: false,
            },
        )
        .expect("default HEAD preview");
        assert!(matches!(
            default.change_set().effects(),
            [SemanticEffect::TaskClosed { evidence, .. }]
                if evidence == &[Evidence {
                    kind: EvidenceKind::Commit,
                    reference: expected_head,
                    captured: None,
                }]
        ));

        let url = Evidence {
            kind: EvidenceKind::Url,
            reference: "https://example.invalid/proof".to_string(),
            captured: None,
        };
        let unresolved = preview_close(
            &context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![url.clone()],
                no_resolve: false,
                execute: false,
            },
        )
        .expect("URL is inspectably unresolved, not silently checked");
        assert!(matches!(
            unresolved.change_set().evidence(),
            ChangeEvidence::Resolution { unresolved, .. } if unresolved == &[url]
        ));
        assert!(
            unresolved
                .notices()
                .iter()
                .any(|notice| notice.contains("no close-time resolver"))
        );
    }

    #[test]
    fn policy_projection_and_restamp_preconditions_are_exact() {
        let fixture = Fixture::new(&live_task().replace(
            "paths: [src/one.rs]",
            "paths: [src/one.rs]\ntags: [untestable, manual]",
        ));
        write(
            &fixture.root().join("aplan/tasks/M-1.md"),
            "---\nid: M-1\ntitle: first cut\nphase: 1\nstatus: todo\ntags: [milestone, \"cut:alpha\"]\nblocked_by: [A-1]\n---\n",
        );
        let mut options = test_options();
        options.exempt_tags = vec!["untestable".to_string(), "manual".to_string()];
        let context = fixture_context(&fixture, options, None);
        let requirement = evidence_requirement(&context, "A-1").expect("scoped policy");
        assert_eq!(requirement.milestone.as_deref(), Some("M-1"));
        assert_eq!(requirement.cut.as_deref(), Some("cut:alpha"));
        assert_eq!(
            requirement.matched_exemptions,
            ["untestable".to_string(), "manual".to_string()]
        );
        assert!(
            evidence_requirement(&context, "A-missing")
                .expect_err("unknown task")
                .to_string()
                .contains("no task")
        );

        let disabled = preview_restamp(
            &context,
            &RestampRequest {
                id: "A-1".to_string(),
            },
        )
        .expect_err("restamp needs drift detection");
        assert!(disabled.to_string().contains("detect_drift is disabled"));

        let live_context = fixture_context(
            &fixture,
            aplan_core::Options {
                detect_drift: true,
                ..aplan_core::Options::default()
            },
            None,
        );
        let live = preview_restamp(
            &live_context,
            &RestampRequest {
                id: "A-1".to_string(),
            },
        )
        .expect_err("only done tasks can be restamped");
        assert!(live.to_string().contains("rather than `done`"));
    }

    #[test]
    fn malformed_duplicate_and_digest_only_plans_fail_closed() {
        let fixture = Fixture::new(live_task());
        write(
            &fixture.root().join("aplan/tasks/duplicate.md"),
            &live_task().replace("title: one", "title: duplicate"),
        );
        let duplicate = fixture_context(&fixture, aplan_core::Options::default(), None);
        assert!(
            evidence_requirement(&duplicate, "A-1")
                .expect_err("duplicate id")
                .to_string()
                .contains("ambiguous")
        );

        let malformed = Fixture::new("---\nid: [\n---\n");
        let malformed_context = fixture_context(&malformed, aplan_core::Options::default(), None);
        let error = preview_close(
            &malformed_context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: Vec::new(),
                no_resolve: false,
                execute: false,
            },
        )
        .expect_err("a malformed plan cannot be mutated");
        assert!(error.to_string().contains("does not load cleanly"));

        let digest_fixture = Fixture::new(live_task());
        let retained = aplan_core::load_plan_identified(&digest_fixture.root().join("aplan"));
        let mut digest_only = retained.clone();
        for input in &mut digest_only.inputs {
            if matches!(input.content, aplan_core::PlanFileContent::Bytes(_)) {
                input.content = aplan_core::PlanFileContent::Sha256("0".repeat(64));
            }
        }
        let digest_context = PlanContext::new(
            digest_only,
            ContextInputs {
                repository_root: digest_fixture.root().to_path_buf(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        );
        let error = preview_close(
            &digest_context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![Evidence {
                    kind: EvidenceKind::Note,
                    reference: "reviewed".to_string(),
                    captured: Some("2026-10-24".to_string()),
                }],
                no_resolve: true,
                execute: false,
            },
        )
        .expect_err("a digest-only context cannot author bytes");
        assert!(
            error
                .to_string()
                .contains("changed while its preview was being prepared")
        );
    }

    #[test]
    fn amend_modes_keep_resolution_delta_and_minimum_strength_guards() {
        let fixture = Fixture::new(&done_task("suite::old"));
        let context = fixture_context(
            &fixture,
            test_options(),
            Some(aplan_config::TestRunners::One(runner(
                "unit",
                "suite::old\nsuite::new",
            ))),
        );
        let replace = preview_amend(
            &context,
            &AmendRequest {
                id: "A-1".to_string(),
                operation: AmendOperation::Replace {
                    from: Evidence {
                        kind: EvidenceKind::Test,
                        reference: "suite::old".to_string(),
                        captured: None,
                    },
                    to: Evidence {
                        kind: EvidenceKind::Test,
                        reference: "suite::new".to_string(),
                        captured: None,
                    },
                },
                no_resolve: false,
            },
        )
        .expect("replacement preview");
        assert_eq!(replace.change_set().disposition(), ChangeDisposition::Ready);
        assert!(
            replace.change_set().files()[0]
                .after()
                .contains("ref: suite::new")
        );

        let remove = preview_amend(
            &context,
            &AmendRequest {
                id: "A-1".to_string(),
                operation: AmendOperation::Remove(Evidence {
                    kind: EvidenceKind::Test,
                    reference: "suite::old".to_string(),
                    captured: None,
                }),
                no_resolve: false,
            },
        )
        .expect("removal preview");
        assert_eq!(remove.change_set().disposition(), ChangeDisposition::Denied);
        assert!(
            remove
                .change_set()
                .diagnostics()
                .added
                .iter()
                .any(|finding| finding.code == "APLAN005")
        );

        let add = preview_amend(
            &context,
            &AmendRequest {
                id: "A-1".to_string(),
                operation: AmendOperation::Add(Evidence {
                    kind: EvidenceKind::Note,
                    reference: "manual review".to_string(),
                    captured: Some("2026-10-24".to_string()),
                }),
                no_resolve: false,
            },
        )
        .expect("dated note addition");
        assert_eq!(add.change_set().disposition(), ChangeDisposition::Ready);
        assert!(matches!(
            add.change_set().evidence(),
            ChangeEvidence::Resolution { unresolved, .. }
                if unresolved.iter().any(|citation| citation.kind == EvidenceKind::Note)
        ));
    }

    #[test]
    fn restamp_is_separate_and_a_current_baseline_is_a_no_op() {
        let task = "---\nid: A-1\ntitle: one\nphase: 1\nstatus: done\neffort_days: 1\npaths: [proof.md]\ntraces: [proof.md#contract]\nevidence:\n  - kind: file\n    ref: proof.md\ntrace_fingerprints:\n  proof.md#contract: stale\nevidence_fingerprints:\n  file:proof.md: stale\n---\n";
        let fixture = Fixture::new(task);
        let options = aplan_core::Options {
            detect_drift: true,
            traces_root: Some(fixture.root().to_path_buf()),
            ..aplan_core::Options::default()
        };
        let context = fixture_context(&fixture, options.clone(), None);
        let request = RestampRequest {
            id: "A-1".to_string(),
        };
        let preview = preview_restamp(&context, &request).expect("restamp preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        assert_eq!(preview.change_set().files().len(), 1);
        let outcome = commit_change_set(&context, preview.into_change_set());
        assert!(matches!(outcome, CommitOutcome::Applied(_)));

        let refreshed = fixture_context(&fixture, options, None);
        let no_op = preview_restamp(&refreshed, &request).expect("current restamp");
        assert_eq!(no_op.change_set().disposition(), ChangeDisposition::NoOp);
        assert!(no_op.change_set().files().is_empty());
    }

    fn verification_options() -> aplan_evidence::VerifyOptions {
        aplan_evidence::VerifyOptions {
            test_runner: Some(aplan_config::TestRunners::One(runner(
                "unit",
                "suite::known_name",
            ))),
            ..aplan_evidence::VerifyOptions::default()
        }
    }

    #[test]
    fn close_and_amend_cover_drift_and_strict_citation_metadata() {
        let drift = Fixture::new(&live_task().replace("paths: [src/one.rs]", "paths: [proof.md]"));
        let drift_context = fixture_context(
            &drift,
            aplan_core::Options {
                detect_drift: true,
                traces_root: Some(drift.root().to_path_buf()),
                ..aplan_core::Options::default()
            },
            None,
        );
        let close = preview_close(
            &drift_context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![Evidence {
                    kind: EvidenceKind::File,
                    reference: "proof.md".to_string(),
                    captured: None,
                }],
                no_resolve: false,
                execute: false,
            },
        )
        .expect("drift-aware close");
        assert!(
            close.change_set().files()[0]
                .after()
                .contains("evidence_fingerprints:")
        );

        let amended = Fixture::new(&done_task("suite::proof"));
        let unchecked = preview_amend(
            &fixture_context(&amended, aplan_core::Options::default(), None),
            &AmendRequest {
                id: "A-1".to_string(),
                operation: AmendOperation::Add(Evidence {
                    kind: EvidenceKind::Note,
                    reference: "manual follow-up".to_string(),
                    captured: Some("2026-10-24".to_string()),
                }),
                no_resolve: true,
            },
        )
        .expect("unchecked amend remains an explicit skipped resolution");
        assert!(matches!(
            unchecked.change_set().evidence(),
            ChangeEvidence::Skipped { citations, .. }
                if citations.iter().any(|citation| citation.kind == EvidenceKind::Note)
        ));

        let invalid_capture = preview_close(
            &fixture_context(&drift, aplan_core::Options::default(), None),
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![Evidence {
                    kind: EvidenceKind::File,
                    reference: "proof.md".to_string(),
                    captured: Some("2026-10-24".to_string()),
                }],
                no_resolve: true,
                execute: false,
            },
        )
        .expect_err("resolver-owned evidence cannot carry an authored date");
        assert!(invalid_capture.to_string().contains("cannot carry"));
    }

    #[test]
    fn degraded_repository_and_release_version_are_explicit() {
        let unreadable = Fixture::new(live_task());
        let unreadable_context = fixture_context(&unreadable, aplan_core::Options::default(), None);
        std::fs::rename(
            unreadable.root().join(".git"),
            unreadable.root().join("git-away"),
        )
        .expect("hide repository after identifying context");
        let unresolved = preview_close(
            &unreadable_context,
            &CloseRequest {
                id: "A-1".to_string(),
                evidence: vec![Evidence {
                    kind: EvidenceKind::Url,
                    reference: "https://example.invalid/proof".to_string(),
                    captured: None,
                }],
                no_resolve: false,
                execute: false,
            },
        )
        .expect("repository degradation remains reviewable");
        assert!(
            unresolved
                .notices()
                .iter()
                .any(|notice| notice.contains("not a readable repository"))
        );

        let release = Fixture::new(&done_task("suite::proof"));
        write(
            &release.root().join("aplan/tasks/M-1.md"),
            "---\nid: M-1\ntitle: release\nphase: 1\nstatus: todo\ntags: [milestone, release]\nblocked_by: [A-1]\n---\n",
        );
        let release_context = fixture_context(&release, aplan_core::Options::default(), None);
        let release_task = release_context
            .plan()
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == "M-1")
            .expect("release task");
        let (version, notice) = recorded_version(&release_context, &release_task.task);
        assert!(version.is_none());
        assert!(
            notice
                .expect("missing version policy is explicit")
                .contains("declares no command")
        );
    }

    #[test]
    fn verification_repair_operands_are_untrusted_at_the_application_boundary() {
        let fixture = Fixture::new(&done_task("known_name"));
        let options = verification_options();
        let context = fixture_context(
            &fixture,
            aplan_core::Options::default(),
            options.test_runner.clone(),
        );
        let verification = verify_context(&context, &options).expect("repair-producing verify");
        assert_eq!(verification.repairs.len(), 1);

        let error_for = |repair: crate::VerificationRepair| {
            let mut candidate = verification.clone();
            candidate.repairs = vec![repair];
            preview_verification_repairs(&context, &candidate, &options)
                .expect_err("hostile repair row")
                .to_string()
        };
        let repair = |task: Option<&str>, from_kind: &str, to_kind: &str, to_ref: &str| {
            crate::VerificationRepair {
                task: task.map(str::to_string),
                from_kind: from_kind.to_string(),
                from_reference: "known_name".to_string(),
                to_kind: to_kind.to_string(),
                to_reference: to_ref.to_string(),
            }
        };

        assert!(error_for(repair(None, "test", "test", "suite::known_name")).contains("no task"));
        assert!(
            error_for(repair(Some("A-1"), "invented", "test", "suite::known_name"))
                .contains("unknown evidence kind")
        );
        assert!(
            error_for(repair(Some("A-1"), "test", "invented", "suite::known_name"))
                .contains("unknown evidence kind")
        );
        assert!(
            error_for(repair(Some("A-1"), "test", "note", "reviewed")).contains("cannot invent")
        );
        assert!(
            error_for(repair(Some("A-1"), "test", "test", "\u{200b}"))
                .contains("non-blank reference")
        );
        assert!(
            error_for(repair(
                Some("A-missing"),
                "test",
                "test",
                "suite::known_name"
            ))
            .contains("no task")
        );
        let missing = error_for(crate::VerificationRepair {
            task: Some("A-1".to_string()),
            from_kind: "test".to_string(),
            from_reference: "not-recorded".to_string(),
            to_kind: "test".to_string(),
            to_reference: "suite::known_name".to_string(),
        });
        assert!(missing.contains("cites no"), "unexpected error: {missing}");

        let mut duplicated = verification;
        duplicated.repairs.push(duplicated.repairs[0].clone());
        let preview = preview_verification_repairs(&context, &duplicated, &options)
            .expect("duplicate identical repairs are idempotent");
        assert_eq!(preview.change_set().effects().len(), 1);
    }

    /// Proof for `tui.evidence_mutation_previews_bind_identified_inputs`: a complete
    /// verifier repair answer becomes one guarded batch, while stale, refused
    /// and empty answers remain distinct non-writing outcomes.
    #[test]
    fn verification_fix_is_one_guarded_batch_with_no_repair_and_stale_controls() {
        let applied = Fixture::new(&done_task("known_name"));
        let applied_options = verification_options();
        let applied_context = fixture_context(
            &applied,
            aplan_core::Options::default(),
            applied_options.test_runner.clone(),
        );
        let applied_verification =
            verify_context(&applied_context, &applied_options).expect("verify applied fixture");
        let applied_preview =
            preview_verification_repairs(&applied_context, &applied_verification, &applied_options)
                .expect("applied repair preview");
        assert!(matches!(
            commit_change_set(&applied_context, applied_preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
        assert!(
            std::fs::read_to_string(applied.task_path())
                .expect("repaired task")
                .contains("ref: suite::known_name")
        );

        let fixture = Fixture::new(&done_task("known_name"));
        let runners = verification_options().test_runner.clone();
        let context = fixture_context(&fixture, aplan_core::Options::default(), runners);
        let options = verification_options();
        let verification = verify_context(&context, &options).expect("verify");
        assert_eq!(verification.repairs.len(), 1);
        let preview = preview_verification_repairs(&context, &verification, &options)
            .expect("repair preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::Ready);
        assert_eq!(preview.change_set().files().len(), 1);
        assert_eq!(preview.change_set().effects().len(), 1);

        let mut changed_options = options.clone();
        changed_options.max_unrelated_files = Some(7);
        assert!(
            preview_verification_repairs(&context, &verification, &changed_options)
                .expect_err("resolver policy change is stale")
                .to_string()
                .contains("stale")
        );

        write(
            &fixture.task_path(),
            &done_task("known_name").replace("title: one", "title: concurrent"),
        );
        assert!(matches!(
            commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Stale { .. }
        ));

        write(&fixture.task_path(), &done_task("suite::known_name"));
        let fixed_context = fixture_context(
            &fixture,
            aplan_core::Options::default(),
            options.test_runner.clone(),
        );
        let clean = verify_context(&fixed_context, &options).expect("clean verify");
        assert!(clean.repairs.is_empty());
        let no_op = preview_verification_repairs(&fixed_context, &clean, &options)
            .expect("no repair preview");
        assert_eq!(no_op.change_set().disposition(), ChangeDisposition::NoOp);

        let mut refused = clean;
        refused.state = VerificationState::Refused;
        assert!(
            preview_verification_repairs(&fixed_context, &refused, &options)
                .expect_err("refused verification cannot write")
                .to_string()
                .contains("did not complete")
        );

        let denied_fixture = Fixture::new(&done_task("suite::known_name"));
        let denied_context =
            fixture_context(&denied_fixture, test_options(), options.test_runner.clone());
        let mut denied_verification =
            verify_context(&denied_context, &options).expect("clean strict verification");
        assert!(denied_verification.repairs.is_empty());
        denied_verification.repairs.push(crate::VerificationRepair {
            task: Some("A-1".to_string()),
            from_kind: "test".to_string(),
            from_reference: "suite::known_name".to_string(),
            to_kind: "url".to_string(),
            to_reference: "https://example.invalid/weaker".to_string(),
        });
        let denied = preview_verification_repairs(&denied_context, &denied_verification, &options)
            .expect("a policy refusal remains an inspectable preview");
        assert_eq!(denied.change_set().disposition(), ChangeDisposition::Denied);
        assert!(
            denied
                .change_set()
                .diagnostics()
                .added
                .iter()
                .any(|finding| finding.code == "APLAN017")
        );
    }
}
