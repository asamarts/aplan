//! Configured test-name discovery shared by CLI and interactive clients.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use aplan_core::Verdict;
use serde::Serialize;

/// What part of the configured catalogue to return.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestCatalogueRequest {
    /// Case-insensitive substring over the exact normalized test name.
    pub query: Option<String>,
}

/// A search term that cannot select anything visible.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TestCatalogueError {
    #[error("a test-name search must contain a visible character; omit it to list everything")]
    VacuousQuery,
}

/// Whether the catalogue is authoritative for the requested search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCatalogueState {
    /// No runner was configured, so there is no evidence namespace to inspect.
    Unconfigured,
    /// Every configured runner answered.
    Complete,
    /// At least one runner failed; absence from the returned names proves
    /// nothing about the runners that did not answer.
    Partial,
    /// Every runner answered and none of its names matched the search.
    NoMatch,
}

/// A stable runner identity. Configuration labels are not required to be
/// unique, so the one-based declaration position is part of the identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct TestCatalogueRunnerIdentity {
    pub index: usize,
    pub label: String,
}

/// One exact evidence name in one runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestCatalogueName {
    pub name: String,
    pub evidence: String,
    /// Every runner that listed this exact name. More than one means a bare
    /// `test:` citation is ambiguous and the resolver will refuse it.
    pub listed_by: Vec<TestCatalogueRunnerIdentity>,
    pub ambiguous: bool,
}

/// What one configured runner returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCatalogueRunnerState {
    Listed,
    Empty,
    Failed,
}

/// One runner and the names selected from its complete answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestCatalogueRunner {
    pub runner: TestCatalogueRunnerIdentity,
    pub state: TestCatalogueRunnerState,
    pub total: usize,
    pub matched: usize,
    pub tests: Vec<TestCatalogueName>,
}

/// A resolver finding retained without parsing its prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestCatalogueFinding {
    pub code: String,
    pub severity: String,
    pub file: Option<PathBuf>,
    pub message: String,
    /// Adapters decide whether to expose this as `agent_instruction`; keeping
    /// it typed here avoids parsing or recomputing the resolver's advice.
    pub instruction: Option<String>,
    pub advisory: bool,
    pub blocking: bool,
}

/// Counts that let a machine distinguish names from runner entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestCatalogueSummary {
    pub runners: usize,
    pub listed_runners: usize,
    pub empty_runners: usize,
    pub failed_runners: usize,
    pub unique_names: usize,
    pub runner_name_pairs: usize,
    pub matched_names: usize,
    pub matched_pairs: usize,
    /// Ambiguous names in the returned selection, not hidden matches outside
    /// the query. With no query this is the complete catalogue count.
    pub ambiguous_names: usize,
}

/// The complete typed configured-test discovery answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestCatalogue {
    pub state: TestCatalogueState,
    pub query: Option<String>,
    pub summary: TestCatalogueSummary,
    pub runners: Vec<TestCatalogueRunner>,
    pub findings: Vec<TestCatalogueFinding>,
    pub denied: bool,
}

fn unconfigured(query: Option<String>) -> TestCatalogue {
    TestCatalogue {
        state: TestCatalogueState::Unconfigured,
        query,
        summary: TestCatalogueSummary {
            runners: 0,
            listed_runners: 0,
            empty_runners: 0,
            failed_runners: 0,
            unique_names: 0,
            runner_name_pairs: 0,
            matched_names: 0,
            matched_pairs: 0,
            ambiguous_names: 0,
        },
        runners: Vec::new(),
        findings: Vec::new(),
        denied: false,
    }
}

fn project_runners(
    raw: &[aplan_evidence::TestCatalogueRunner],
    needle: Option<&str>,
) -> (
    Vec<TestCatalogueRunner>,
    BTreeMap<String, Vec<TestCatalogueRunnerIdentity>>,
) {
    let identities: Vec<_> = raw
        .iter()
        .map(|runner| TestCatalogueRunnerIdentity {
            index: runner.index,
            label: runner.label.clone(),
        })
        .collect();
    let mut listed_by: BTreeMap<String, Vec<TestCatalogueRunnerIdentity>> = BTreeMap::new();
    for (runner, identity) in raw.iter().zip(&identities) {
        for name in &runner.names {
            listed_by
                .entry(name.clone())
                .or_default()
                .push(identity.clone());
        }
    }
    let projected = raw
        .iter()
        .zip(identities)
        .map(|(runner, identity)| {
            let state = match runner.state {
                aplan_evidence::TestCatalogueRunnerState::Listed => {
                    TestCatalogueRunnerState::Listed
                }
                aplan_evidence::TestCatalogueRunnerState::Empty => TestCatalogueRunnerState::Empty,
                aplan_evidence::TestCatalogueRunnerState::Failed => {
                    TestCatalogueRunnerState::Failed
                }
            };
            let tests = runner
                .names
                .iter()
                .filter(|name| needle.is_none_or(|needle| name.to_lowercase().contains(needle)))
                .map(|name| {
                    let owners = listed_by
                        .get(name)
                        .cloned()
                        .expect("every listed name has an owner");
                    TestCatalogueName {
                        name: name.clone(),
                        evidence: format!("test:{name}"),
                        ambiguous: owners.len() > 1,
                        listed_by: owners,
                    }
                })
                .collect::<Vec<_>>();
            TestCatalogueRunner {
                runner: identity,
                state,
                total: runner.names.len(),
                matched: tests.len(),
                tests,
            }
        })
        .collect();
    (projected, listed_by)
}

fn project_findings(
    diagnostics: &[aplan_core::Diagnostic],
    verdict: &Verdict,
) -> Vec<TestCatalogueFinding> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let advisory = verdict.is_advisory(diagnostic.code);
            let blocking = !advisory
                && (diagnostic.severity == aplan_core::Severity::Error || verdict.deny_warnings);
            TestCatalogueFinding {
                code: diagnostic.code.to_string(),
                severity: diagnostic.severity.to_string(),
                file: diagnostic.file.clone(),
                message: diagnostic.message.clone(),
                instruction: diagnostic.agent_instruction.clone(),
                advisory,
                blocking,
            }
        })
        .collect()
}

fn summarize(
    raw: &[aplan_evidence::TestCatalogueRunner],
    projected: &[TestCatalogueRunner],
    listed_by: &BTreeMap<String, Vec<TestCatalogueRunnerIdentity>>,
) -> TestCatalogueSummary {
    let matched_names: BTreeSet<&str> = projected
        .iter()
        .flat_map(|runner| runner.tests.iter().map(|test| test.name.as_str()))
        .collect();
    TestCatalogueSummary {
        runners: projected.len(),
        listed_runners: projected
            .iter()
            .filter(|runner| runner.state == TestCatalogueRunnerState::Listed)
            .count(),
        empty_runners: projected
            .iter()
            .filter(|runner| runner.state == TestCatalogueRunnerState::Empty)
            .count(),
        failed_runners: projected
            .iter()
            .filter(|runner| runner.state == TestCatalogueRunnerState::Failed)
            .count(),
        unique_names: listed_by.len(),
        runner_name_pairs: raw.iter().map(|runner| runner.names.len()).sum(),
        matched_names: matched_names.len(),
        matched_pairs: projected.iter().map(|runner| runner.matched).sum(),
        ambiguous_names: matched_names
            .iter()
            .filter(|name| listed_by.get(**name).is_some_and(|owners| owners.len() > 1))
            .count(),
    }
}

/// List or search the names accepted by configured test evidence.
///
/// This application operation performs no listing itself. The evidence crate's
/// resolver owns the commands and normalization; this layer supplies stable
/// search, ambiguity and verdict projections to every delivery adapter.
pub fn test_catalogue(
    repo_root: &Path,
    runners: Option<&aplan_config::TestRunners>,
    config_file: Option<&Path>,
    verdict: &Verdict,
    request: TestCatalogueRequest,
) -> Result<TestCatalogue, TestCatalogueError> {
    test_catalogue_inner(
        repo_root,
        runners,
        config_file,
        verdict,
        request,
        &mut |_| {},
    )
}

/// List or search configured names while reporting typed provider progress.
pub fn test_catalogue_with_progress(
    repo_root: &Path,
    runners: Option<&aplan_config::TestRunners>,
    config_file: Option<&Path>,
    verdict: &Verdict,
    request: TestCatalogueRequest,
    operation: crate::OperationId,
    progress: &mut dyn FnMut(crate::ProgressEvent),
) -> Result<TestCatalogue, TestCatalogueError> {
    let mut emitter =
        crate::ProgressEmitter::begin(operation, crate::OperationKind::TestCatalogue, progress);
    let mut adapter = crate::progress::CatalogueProgressAdapter::default();
    let result = test_catalogue_inner(
        repo_root,
        runners,
        config_file,
        verdict,
        request,
        &mut |event| adapter.accept(&mut emitter, &event),
    );
    if result.is_ok() {
        emitter.complete();
    } else {
        emitter.fail();
    }
    result
}

fn test_catalogue_inner(
    repo_root: &Path,
    runners: Option<&aplan_config::TestRunners>,
    config_file: Option<&Path>,
    verdict: &Verdict,
    request: TestCatalogueRequest,
    progress: &mut dyn FnMut(aplan_evidence::CatalogueProgressEvent),
) -> Result<TestCatalogue, TestCatalogueError> {
    if request
        .query
        .as_deref()
        .is_some_and(aplan_core::task::is_vacuous_ref)
    {
        return Err(TestCatalogueError::VacuousQuery);
    }

    let Some(configured) = runners else {
        return Ok(unconfigured(request.query));
    };

    let raw = aplan_evidence::configured_test_catalogue_bounded(
        configured,
        repo_root,
        config_file,
        progress,
    );
    let needle = request.query.as_ref().map(|query| query.to_lowercase());
    let (projected, listed_by) = project_runners(&raw.runners, needle.as_deref());

    let findings = project_findings(&raw.diagnostics, verdict);
    let denied = findings.iter().any(|finding| finding.blocking);
    let summary = summarize(&raw.runners, &projected, &listed_by);
    let state = if summary.failed_runners > 0 {
        TestCatalogueState::Partial
    } else if request.query.is_some() && summary.matched_pairs == 0 {
        TestCatalogueState::NoMatch
    } else {
        TestCatalogueState::Complete
    };
    Ok(TestCatalogue {
        state,
        query: request.query,
        summary,
        runners: projected,
        findings,
        denied,
    })
}
