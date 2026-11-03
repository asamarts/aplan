//! Provider observations for bounded, non-blocking evidence inspection.
//!
//! The cache contains raw configured-test catalogues, never an authoritative
//! plan verdict. Git-backed rules and every plan-dependent test rule are
//! recomputed against the task being displayed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aplan_core::show::{
    EvidenceObservation, EvidenceObservationCompleteness, EvidenceObservationScope,
    EvidenceObservationState, EvidenceProviderSource,
};
use aplan_core::{EvidenceKind, Plan, Report, Show};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const MAGIC: &str = "aplan-evidence-observation";
const PROTOCOL: u32 = 1;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const STALE_LOCK_AFTER: Duration = Duration::from_secs(30);
const LOCK_ATTEMPTS: usize = 4_096;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Whether inspection may execute configured providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceInspectionMode {
    /// Read a reusable observation and never start a configured provider.
    Stored,
    /// Do not even read retained observations.
    Skip,
    /// Explicitly run providers and publish only a complete stable result.
    Refresh,
}

/// Host controls for the disposable observation store.
#[derive(Debug, Clone, Default)]
pub struct EvidenceObservationOptions {
    pub cache_root: Option<PathBuf>,
}

/// One complete inspection answer.
#[derive(Debug)]
pub struct EvidenceInspection {
    pub report: Report,
    pub citations: Vec<aplan_evidence::CitationObservation>,
    pub observation: EvidenceObservation,
    pub spend: Vec<(String, aplan_evidence::Cost)>,
    pub(crate) citation_states: Vec<EvidenceObservationState>,
}

#[derive(Debug, Clone, Copy)]
struct ObservationProvenance {
    scope: EvidenceObservationScope,
    provider_source: EvidenceProviderSource,
    completeness: EvidenceObservationCompleteness,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCatalogue {
    runners: Vec<aplan_evidence::TestCatalogueRunner>,
}

impl StoredCatalogue {
    fn from_raw(raw: &aplan_evidence::TestCatalogue) -> Self {
        Self {
            runners: raw.runners.clone(),
        }
    }

    fn raw(&self) -> aplan_evidence::TestCatalogue {
        aplan_evidence::TestCatalogue {
            runners: self.runners.clone(),
            diagnostics: Vec::new(),
            spend: Vec::new(),
            failure_outcomes: Vec::new(),
        }
    }

    fn complete(&self) -> bool {
        self.runners
            .iter()
            .all(|runner| runner.state != aplan_evidence::TestCatalogueRunnerState::Failed)
    }

    fn matches(&self, runners: &aplan_evidence::TestRunners) -> bool {
        self.runners.len() == runners.all().len()
            && self
                .runners
                .iter()
                .enumerate()
                .all(|(index, runner)| runner.index == index + 1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DerivedIdentity {
    identity: String,
    plan: String,
    configuration: String,
    repository: String,
    provider_observation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredObservation {
    protocol: u32,
    root: String,
    provider: String,
    material: String,
    identity: String,
    observed_at_epoch_seconds: i64,
    started_at_epoch_nanos: u128,
    complete: bool,
    failure: Option<String>,
    catalogue: Option<StoredCatalogue>,
    derived: Option<DerivedIdentity>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    magic: String,
    checksum: String,
    observation: StoredObservation,
}

#[derive(Debug)]
enum MaterialIdentity {
    Identified(String),
    Unsupported(String),
}

impl MaterialIdentity {
    fn as_identified(&self) -> Option<&str> {
        match self {
            Self::Identified(value) => Some(value),
            Self::Unsupported(_) => None,
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Identified(_) => None,
            Self::Unsupported(reason) => Some(reason),
        }
    }
}

/// Inspect evidence without letting an ordinary read execute opaque providers.
pub fn inspect_evidence(
    plan: &Plan,
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    mode: EvidenceInspectionMode,
    store_options: &EvidenceObservationOptions,
    progress: impl FnMut(aplan_evidence::CatalogueProgressEvent),
) -> Result<EvidenceInspection, aplan_evidence::Error> {
    match mode {
        EvidenceInspectionMode::Skip => Ok(skipped(plan)),
        EvidenceInspectionMode::Stored => inspect_stored(plan, repo_root, options, store_options),
        EvidenceInspectionMode::Refresh => {
            refresh(plan, repo_root, options, store_options, progress)
        }
    }
}

/// Inspect evidence while reporting the same typed operation contract used by
/// interactive verification. Runner names and output remain in the final
/// evidence result; progress carries only stable phase and ordinal identities.
pub fn inspect_evidence_with_progress(
    plan: &Plan,
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    mode: EvidenceInspectionMode,
    store_options: &EvidenceObservationOptions,
    operation: crate::OperationId,
    progress: &mut dyn FnMut(crate::ProgressEvent),
) -> Result<EvidenceInspection, aplan_evidence::Error> {
    let mut emitter =
        crate::ProgressEmitter::begin(operation, crate::OperationKind::Verification, progress);
    emitter.phase_started(
        crate::ProgressPhase::RepositoryObservation,
        None,
        crate::ProgressMeasure::Indeterminate,
    );
    let mut provider_progress = crate::progress::CatalogueProgressAdapter::default();
    let result = inspect_evidence(plan, repo_root, options, mode, store_options, |event| {
        provider_progress.accept(&mut emitter, &event);
    });
    if result.is_ok() {
        emitter.phase_completed(
            crate::ProgressPhase::RepositoryObservation,
            None,
            crate::ProgressMeasure::Indeterminate,
        );
        emitter.complete();
    } else {
        emitter.phase_failed(
            crate::ProgressPhase::RepositoryObservation,
            None,
            crate::ProgressMeasure::Indeterminate,
        );
        emitter.fail();
    }
    result
}

fn skipped(plan: &Plan) -> EvidenceInspection {
    let citations = plan
        .tasks
        .iter()
        .flat_map(|loaded| {
            loaded.task.evidence.iter().map(|evidence| {
                unavailable_citation(
                    &loaded.task.id,
                    evidence,
                    "the evidence was not resolved (--no-resolve skipped the ladder)",
                )
            })
        })
        .collect();
    EvidenceInspection {
        report: Report::default(),
        citations,
        observation: observation(
            EvidenceObservationState::NotObserved,
            ObservationProvenance {
                scope: EvidenceObservationScope::SelectedTasks,
                provider_source: EvidenceProviderSource::Deferred,
                completeness: EvidenceObservationCompleteness::Partial,
            },
            None,
            None,
            None,
            "the evidence was not resolved (--no-resolve skipped the ladder)",
        ),
        spend: Vec::new(),
        citation_states: plan
            .tasks
            .iter()
            .flat_map(|loaded| loaded.task.evidence.iter())
            .map(|_| EvidenceObservationState::NotObserved)
            .collect(),
    }
}

fn inspect_stored(
    plan: &Plan,
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    store_options: &EvidenceObservationOptions,
) -> Result<EvidenceInspection, aplan_evidence::Error> {
    let uses_tests = has_test_citations(plan);
    let (test_state, catalogue, provider_observation, observed_at, detail) = if uses_tests {
        let provider = provider_identity(options);
        let stored = read_observation(repo_root, store_options);
        let material = stored.as_ref().map_or_else(
            || MaterialIdentity::Unsupported("no retained observation needs an input check".into()),
            |_| material_identity(repo_root, options),
        );
        stored_test_state(stored.as_ref(), &provider, &material, options)
    } else {
        (
            EvidenceObservationState::Unsupported,
            None,
            None,
            None,
            "the displayed task cites no configured test provider".to_string(),
        )
    };

    let mut local = options.clone();
    local.execute = false;
    local.version = None;
    local.url_check = None;
    if test_state != EvidenceObservationState::Current {
        local.test_runner = None;
    }
    let observed = match catalogue.as_ref() {
        Some(catalogue) => {
            aplan_evidence::verify_observed_with_test_catalogue(plan, repo_root, &local, catalogue)?
        }
        None => aplan_evidence::verify_observed(plan, repo_root, &local)?,
    };
    let repository = repository_identity(repo_root)?;
    let resolution = resolution_identity(
        plan,
        options,
        &repository,
        provider_observation.as_deref(),
        &observed,
    );
    Ok(project_inspection(
        plan,
        observed,
        EvidenceObservationScope::SelectedTasks,
        EvidenceProviderSource::Deferred,
        EvidenceObservationCompleteness::Partial,
        test_state,
        EvidenceObservationState::Unsupported,
        provider_observation,
        observed_at,
        Some(resolution),
        detail,
    ))
}

fn refresh(
    plan: &Plan,
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    store_options: &EvidenceObservationOptions,
    progress: impl FnMut(aplan_evidence::CatalogueProgressEvent),
) -> Result<EvidenceInspection, aplan_evidence::Error> {
    let started_at_epoch_nanos = epoch_nanos();
    let provider = provider_identity(options);
    let uses_tests = has_test_citations(plan);
    let configured_tests = uses_tests.then_some(options.test_runner.as_ref()).flatten();
    let before = configured_tests.map_or_else(
        || MaterialIdentity::Unsupported("no configured test catalogue is needed".into()),
        |_| material_identity(repo_root, options),
    );
    let catalogue = configured_tests.map(|runners| {
        aplan_evidence::configured_test_catalogue_bounded(
            runners,
            repo_root,
            options.config_file.as_deref(),
            progress,
        )
    });
    let observed = match catalogue.as_ref() {
        Some(catalogue) => aplan_evidence::verify_observed_with_test_catalogue(
            plan, repo_root, options, catalogue,
        )?,
        _ => aplan_evidence::verify_observed(plan, repo_root, options)?,
    };
    let after = configured_tests.map_or_else(
        || MaterialIdentity::Unsupported("no configured test catalogue is needed".into()),
        |_| material_identity(repo_root, options),
    );
    let stable = before
        .as_identified()
        .zip(after.as_identified())
        .and_then(|(before, after)| (before == after).then_some(before.to_string()));
    let catalogue_complete = catalogue
        .as_ref()
        .is_none_or(|catalogue| StoredCatalogue::from_raw(catalogue).complete());
    let observed_at = epoch_seconds();
    let provider_observation = stable.as_ref().map(|material| {
        observation_identity(
            &provider,
            material,
            catalogue.as_ref().map(StoredCatalogue::from_raw).as_ref(),
        )
    });
    let repository = repository_identity(repo_root)?;
    let resolution = resolution_identity(
        plan,
        options,
        &repository,
        provider_observation.as_deref(),
        &observed,
    );
    let publishable = stable.is_some() && catalogue_complete && catalogue.is_some();
    if configured_tests.is_some() {
        let failure = refresh_failure(catalogue_complete, &before, stable.as_deref());
        let stored = StoredObservation {
            protocol: PROTOCOL,
            root: root_identity(repo_root),
            provider: provider.clone(),
            material: stable.clone().unwrap_or_default(),
            identity: provider_observation.clone().unwrap_or_default(),
            observed_at_epoch_seconds: observed_at,
            started_at_epoch_nanos,
            complete: publishable,
            failure,
            catalogue: publishable
                .then(|| catalogue.as_ref().map(StoredCatalogue::from_raw))
                .flatten(),
            derived: publishable.then(|| DerivedIdentity {
                identity: resolution.clone(),
                plan: plan_identity(plan),
                configuration: configuration_identity(options),
                repository: repository.clone(),
                provider_observation: provider_observation.clone().unwrap_or_default(),
            }),
        };
        publish_observation(repo_root, store_options, &stored);
    }
    let (state, detail) = refreshed_state_and_detail(
        uses_tests,
        options.test_runner.is_some(),
        catalogue_complete,
        &before,
        &after,
        stable.as_deref(),
    );
    let (provider_source, completeness) = provider_provenance(uses_tests, options.execute);
    Ok(project_inspection(
        plan,
        observed,
        EvidenceObservationScope::SelectedTasks,
        provider_source,
        completeness,
        state,
        configured_url_state(options),
        provider_observation,
        Some(observed_at),
        Some(resolution),
        detail,
    ))
}

fn configured_url_state(options: &aplan_evidence::VerifyOptions) -> EvidenceObservationState {
    if options.url_check.is_some() {
        EvidenceObservationState::Current
    } else {
        EvidenceObservationState::Unsupported
    }
}

fn provider_provenance(
    uses_tests: bool,
    execute: bool,
) -> (EvidenceProviderSource, EvidenceObservationCompleteness) {
    match (uses_tests, execute) {
        (false, _) => (
            EvidenceProviderSource::None,
            EvidenceObservationCompleteness::Complete,
        ),
        (true, true) => (
            EvidenceProviderSource::Direct,
            EvidenceObservationCompleteness::Complete,
        ),
        (true, false) => (
            EvidenceProviderSource::Deferred,
            EvidenceObservationCompleteness::Partial,
        ),
    }
}

fn refresh_failure(
    catalogue_complete: bool,
    before: &MaterialIdentity,
    stable: Option<&str>,
) -> Option<String> {
    if !catalogue_complete {
        Some("one or more configured test runners did not return a complete catalogue".into())
    } else if before.as_identified().is_none() {
        before.reason().map(str::to_string)
    } else if stable.is_none() {
        Some("declared provider inputs changed while refresh was running".into())
    } else {
        None
    }
}

fn refreshed_state_and_detail(
    uses_tests: bool,
    has_runner: bool,
    catalogue_complete: bool,
    before: &MaterialIdentity,
    after: &MaterialIdentity,
    stable: Option<&str>,
) -> (EvidenceObservationState, String) {
    let state = if !uses_tests || !has_runner {
        EvidenceObservationState::Unsupported
    } else if !catalogue_complete {
        EvidenceObservationState::RefreshFailed
    } else if before.as_identified().is_none() || after.as_identified().is_none() {
        EvidenceObservationState::Unsupported
    } else if stable.is_none() {
        EvidenceObservationState::Stale
    } else {
        EvidenceObservationState::Current
    };
    let detail = match state {
        EvidenceObservationState::RefreshFailed => {
            "one or more configured test runners did not return a complete catalogue".to_string()
        }
        EvidenceObservationState::Stale => before
            .reason()
            .or_else(|| after.reason())
            .unwrap_or("declared provider inputs changed while refresh was running")
            .to_string(),
        EvidenceObservationState::Unsupported if !uses_tests => {
            "the displayed task cites no configured test provider".to_string()
        }
        EvidenceObservationState::Unsupported if !has_runner => {
            "the repository configures no test evidence provider".to_string()
        }
        EvidenceObservationState::Unsupported => before
            .reason()
            .or_else(|| after.reason())
            .unwrap_or("provider inputs are not identifiable")
            .to_string(),
        _ => "explicit evidence refresh completed".to_string(),
    };
    (state, detail)
}

fn stored_test_state(
    stored: Option<&StoredObservation>,
    provider: &str,
    material: &MaterialIdentity,
    options: &aplan_evidence::VerifyOptions,
) -> (
    EvidenceObservationState,
    Option<aplan_evidence::TestCatalogue>,
    Option<String>,
    Option<i64>,
    String,
) {
    if options.test_runner.is_none() {
        return (
            EvidenceObservationState::Unsupported,
            None,
            None,
            None,
            "the repository configures no test evidence provider".into(),
        );
    }
    let Some(stored) = stored else {
        return (
            EvidenceObservationState::NotObserved,
            None,
            None,
            None,
            "no provider observation has been published; use --refresh-evidence".into(),
        );
    };
    let Some(material) = material.as_identified() else {
        return (
            EvidenceObservationState::Unsupported,
            None,
            None,
            None,
            material
                .reason()
                .unwrap_or("provider inputs are not identifiable")
                .into(),
        );
    };
    if stored.protocol != PROTOCOL || stored.provider != provider || stored.material != material {
        return (
            EvidenceObservationState::Stale,
            None,
            Some(stored.identity.clone()).filter(|identity| !identity.is_empty()),
            Some(stored.observed_at_epoch_seconds),
            "a provider, environment, executable, or declared material input changed".into(),
        );
    }
    if !stored.complete {
        return (
            EvidenceObservationState::RefreshFailed,
            None,
            None,
            Some(stored.observed_at_epoch_seconds),
            stored
                .failure
                .clone()
                .unwrap_or_else(|| "the last refresh was incomplete".into()),
        );
    }
    let Some(catalogue) = stored.catalogue.as_ref().map(StoredCatalogue::raw) else {
        return (
            EvidenceObservationState::RefreshFailed,
            None,
            None,
            Some(stored.observed_at_epoch_seconds),
            "the retained observation is incomplete".into(),
        );
    };
    let structurally_current = stored
        .catalogue
        .as_ref()
        .zip(options.test_runner.as_ref())
        .is_some_and(|(catalogue, runners)| catalogue.matches(runners));
    if !structurally_current {
        return (
            EvidenceObservationState::RefreshFailed,
            None,
            None,
            Some(stored.observed_at_epoch_seconds),
            "the retained catalogue does not contain exactly one ordered row per configured runner"
                .into(),
        );
    }
    (
        EvidenceObservationState::Current,
        Some(catalogue),
        Some(stored.identity.clone()),
        Some(stored.observed_at_epoch_seconds),
        "reused a complete provider observation whose declared inputs are unchanged".into(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn project_inspection(
    plan: &Plan,
    observed: aplan_evidence::ObservedVerification,
    scope: EvidenceObservationScope,
    provider_source: EvidenceProviderSource,
    completeness: EvidenceObservationCompleteness,
    test_state: EvidenceObservationState,
    url_state: EvidenceObservationState,
    provider_identity: Option<String>,
    observed_at: Option<i64>,
    resolution_identity: Option<String>,
    detail: String,
) -> EvidenceInspection {
    let mut citations = observed.citations;
    let mut states = Vec::new();
    for citation in &citations {
        states.push(match citation.kind {
            EvidenceKind::Test => test_state,
            EvidenceKind::Url => url_state,
            EvidenceKind::Commit | EvidenceKind::File | EvidenceKind::Note => {
                EvidenceObservationState::Current
            }
        });
    }
    // A resolver omitted by configuration still gets a typed citation row from
    // the engine. Preserve its ordering while replacing only freshness below.
    if citations.is_empty() {
        for loaded in &plan.tasks {
            for evidence in &loaded.task.evidence {
                citations.push(unavailable_citation(
                    &loaded.task.id,
                    evidence,
                    "no resolver observation was produced",
                ));
                states.push(EvidenceObservationState::Unsupported);
            }
        }
    }
    let aggregate = aggregate_state(&states);
    let has_test = citations
        .iter()
        .any(|citation| citation.kind == EvidenceKind::Test);
    let has_url = citations
        .iter()
        .any(|citation| citation.kind == EvidenceKind::Url);
    let detail = match aggregate {
        EvidenceObservationState::Current => {
            "all cited evidence observations are current".to_string()
        }
        EvidenceObservationState::Unsupported if !has_test && has_url => {
            "URL observations are not retained without an immutable-subject or expiry contract"
                .to_string()
        }
        _ => detail,
    };
    EvidenceInspection {
        report: observed.report,
        citations,
        observation: observation(
            aggregate,
            ObservationProvenance {
                scope,
                provider_source,
                completeness,
            },
            has_test.then_some(provider_identity).flatten(),
            resolution_identity,
            observed_at,
            detail,
        ),
        spend: observed.spend,
        citation_states: states,
    }
}

fn has_test_citations(plan: &Plan) -> bool {
    plan.tasks.iter().any(|loaded| {
        loaded
            .task
            .evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::Test)
    })
}

pub(crate) fn aggregate_state(states: &[EvidenceObservationState]) -> EvidenceObservationState {
    if states.is_empty()
        || states
            .iter()
            .all(|state| *state == EvidenceObservationState::Current)
    {
        return EvidenceObservationState::Current;
    }
    for state in [
        EvidenceObservationState::Untrusted,
        EvidenceObservationState::Incompatible,
        EvidenceObservationState::Fault,
        EvidenceObservationState::RefreshFailed,
        EvidenceObservationState::Stale,
        EvidenceObservationState::NotObserved,
        EvidenceObservationState::Unsupported,
    ] {
        if states.contains(&state) {
            return state;
        }
    }
    EvidenceObservationState::Current
}

fn unavailable_citation(
    task: &str,
    evidence: &aplan_core::Evidence,
    detail: &str,
) -> aplan_evidence::CitationObservation {
    aplan_evidence::CitationObservation {
        task: task.to_string(),
        kind: evidence.kind,
        reference: evidence.reference.clone(),
        resolution: aplan_evidence::CitationResolution {
            resolver: "not observed".into(),
            strength: aplan_evidence::CitationStrength::Recorded,
            outcome: aplan_evidence::CitationOutcome::Unavailable,
            detail: Some(detail.into()),
            cost_subject: None,
        },
    }
}

fn observation(
    state: EvidenceObservationState,
    provenance: ObservationProvenance,
    identity: Option<String>,
    resolution_identity: Option<String>,
    observed_at_epoch_seconds: Option<i64>,
    detail: impl Into<String>,
) -> EvidenceObservation {
    EvidenceObservation {
        state,
        scope: provenance.scope,
        provider_source: provenance.provider_source,
        completeness: provenance.completeness,
        identity,
        resolution_identity,
        observed_at_epoch_seconds,
        resolver: None,
        strength: None,
        outcome: None,
        detail: Some(detail.into()),
        refresh: None,
    }
}

/// Attach typed aggregate and per-citation observations to the shared view.
pub fn apply_evidence_inspection(show: &mut Show, inspection: &EvidenceInspection) {
    show.evidence_observation = inspection.observation.clone();
    show.resolved = inspection.observation.state == EvidenceObservationState::Current;
    let mut observations: BTreeMap<
        (String, String),
        Vec<(
            &aplan_evidence::CitationObservation,
            EvidenceObservationState,
        )>,
    > = BTreeMap::new();
    for (citation, state) in inspection
        .citations
        .iter()
        .zip(&inspection.citation_states)
        .filter(|(citation, _)| citation.task == show.id)
    {
        observations
            .entry((
                citation.kind.as_str().to_string(),
                citation.reference.clone(),
            ))
            .or_default()
            .push((citation, *state));
    }
    let mut used: BTreeMap<(String, String), usize> = BTreeMap::new();
    for cited in &mut show.evidence {
        let key = (cited.kind.clone(), cited.reference.clone());
        let index = used.entry(key.clone()).or_default();
        let observed = observations
            .get(&key)
            .and_then(|entries| entries.get(*index))
            .copied();
        *index += 1;
        let (citation, state) = observed.map_or(
            (None, EvidenceObservationState::NotObserved),
            |(citation, state)| (Some(citation), state),
        );
        cited.observation = EvidenceObservation {
            state,
            scope: inspection.observation.scope,
            provider_source: inspection.observation.provider_source,
            completeness: inspection.observation.completeness,
            identity: (cited.kind == EvidenceKind::Test.as_str())
                .then(|| inspection.observation.identity.clone())
                .flatten(),
            resolution_identity: inspection.observation.resolution_identity.clone(),
            observed_at_epoch_seconds: inspection.observation.observed_at_epoch_seconds,
            resolver: citation.map(|value| value.resolution.resolver.clone()),
            strength: citation.map(|value| value.resolution.strength.label().to_string()),
            outcome: citation.map(|value| value.resolution.outcome.label().to_string()),
            detail: citation
                .and_then(|value| value.resolution.detail.clone())
                .or_else(|| inspection.observation.detail.clone()),
            refresh: inspection.observation.refresh.clone(),
        };
    }
}

fn provider_identity(options: &aplan_evidence::VerifyOptions) -> String {
    let mut digest = Sha256::new();
    digest.update(b"aplan-evidence-provider-v1\0");
    digest.update(PROTOCOL.to_le_bytes());
    if let Some(runners) = &options.test_runner {
        digest.update(serde_json::to_vec(runners).unwrap_or_default());
    }
    digest.update(
        serde_json::to_vec(&options.observation_inputs).unwrap_or_else(|_| b"null".to_vec()),
    );
    hex(&digest.finalize())
}

fn material_identity(
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
) -> MaterialIdentity {
    let Some(patterns) = options.observation_inputs.as_deref() else {
        return MaterialIdentity::Unsupported(
            "the repository has not declared exhaustive evidence.observation_inputs".into(),
        );
    };
    let glob_matcher = match glob_set(patterns) {
        Ok(matcher) => matcher,
        Err(reason) => return MaterialIdentity::Unsupported(reason),
    };
    let roots = input_roots(patterns);
    let mut input_files = BTreeMap::new();
    for root in roots {
        let path = repo_root.join(&root);
        if let Err(reason) = collect_inputs(repo_root, &path, &glob_matcher, &mut input_files) {
            return MaterialIdentity::Unsupported(reason);
        }
    }
    if input_files.is_empty() {
        return MaterialIdentity::Unsupported(
            "evidence.observation_inputs matched no regular files; an empty material key cannot prove provider currency"
                .into(),
        );
    }
    let input_files = input_files.into_iter().collect::<Vec<_>>();
    // The executable behind a runner can be larger than every declared source
    // input combined (the rustup proxy is one example). Hash it alongside the
    // source set. This changes no trust boundary: both byte-level identities
    // still have to complete before an observation can be current.
    let (inputs, executables) = std::thread::scope(|scope| {
        let executables = scope.spawn(|| provider_executables_identity(options));
        let inputs = declared_inputs_identity(repo_root, &input_files);
        (inputs, executables.join())
    });
    let inputs = match inputs {
        Ok(identity) => identity,
        Err(reason) => return MaterialIdentity::Unsupported(reason),
    };
    let executables = match executables {
        Ok(Ok(identity)) => identity,
        Ok(Err(reason)) => return MaterialIdentity::Unsupported(reason),
        Err(_) => {
            return MaterialIdentity::Unsupported(
                "a provider executable identity worker stopped unexpectedly".into(),
            );
        }
    };
    let mut digest = Sha256::new();
    digest.update(b"aplan-evidence-material-v2\0");
    digest.update(root_identity(repo_root).as_bytes());
    digest.update(inputs);
    hash_environment(&mut digest);
    digest.update(executables);
    MaterialIdentity::Identified(hex(&digest.finalize()))
}

fn declared_inputs_identity(
    repo_root: &Path,
    inputs: &[(PathBuf, fs::Metadata)],
) -> Result<[u8; 32], String> {
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(4)
        .min(inputs.len());
    let chunk_size = inputs.len().div_ceil(workers);
    let identities = std::thread::scope(|scope| {
        let handles = inputs
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(relative, before)| {
                            declared_input_identity(repo_root, relative, before)
                                .map(|identity| (relative, identity))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut identities = Vec::with_capacity(inputs.len());
        for handle in handles {
            identities.extend(
                handle
                    .join()
                    .map_err(|_| "a declared-input identity worker stopped unexpectedly")?,
            );
        }
        Ok::<_, &str>(identities)
    })
    .map_err(str::to_string)?;

    let mut digest = blake3::Hasher::new();
    digest.update(b"aplan-evidence-declared-inputs-v1\0");
    for identity in identities {
        let (relative, identity) = identity?;
        hash_path_blake3(&mut digest, relative);
        digest.update(&identity);
    }
    Ok(*digest.finalize().as_bytes())
}

fn declared_input_identity(
    repo_root: &Path,
    relative: &Path,
    before: &fs::Metadata,
) -> Result<[u8; 32], String> {
    let path = repo_root.join(relative);
    if !before.file_type().is_file() {
        return Err(format!(
            "declared provider input `{}` is not a regular file",
            relative.display()
        ));
    }
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "declared provider input `{}` could not be read: {error}",
            relative.display()
        )
    })?;
    let after = fs::symlink_metadata(&path).map_err(|error| {
        format!(
            "declared provider input `{}` moved while it was read: {error}",
            relative.display()
        )
    })?;
    if !same_file_observation(before, &after) {
        return Err(format!(
            "declared provider input `{}` changed while it was read",
            relative.display()
        ));
    }
    let mut digest = blake3::Hasher::new();
    digest.update(b"aplan-evidence-declared-input-v1\0");
    hash_file_metadata_blake3(&mut digest, before);
    digest.update(&bytes);
    Ok(*digest.finalize().as_bytes())
}

fn glob_set(patterns: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|error| error.to_string())?);
    }
    builder.build().map_err(|error| error.to_string())
}

fn input_roots(patterns: &[String]) -> BTreeSet<PathBuf> {
    patterns
        .iter()
        .filter_map(|pattern| Path::new(pattern).components().next())
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(PathBuf::from(value)),
            _ => None,
        })
        .collect()
}

fn collect_inputs(
    repo_root: &Path,
    path: &Path,
    glob_matcher: &GlobSet,
    input_files: &mut BTreeMap<PathBuf, fs::Metadata>,
) -> Result<(), String> {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("observing `{}`: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "declared provider input tree contains symbolic link `{}`; its target is not an exact repository input",
            relative.display()
        ));
    }
    if metadata.is_file() {
        if glob_matcher.is_match(relative) {
            input_files.insert(relative.to_path_buf(), metadata);
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("enumerating `{}`: {error}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("enumerating `{}`: {error}", path.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        collect_inputs(repo_root, &entry.path(), glob_matcher, input_files)?;
    }
    Ok(())
}

fn same_file_observation(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && file_permissions(before) == file_permissions(after)
}

#[cfg(unix)]
fn file_permissions(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt as _;
    metadata.mode()
}

#[cfg(not(unix))]
fn file_permissions(metadata: &fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[cfg(unix)]
fn hash_file_metadata_blake3(digest: &mut blake3::Hasher, metadata: &fs::Metadata) {
    use std::os::unix::fs::MetadataExt as _;
    digest.update(&metadata.mode().to_le_bytes());
}

#[cfg(not(unix))]
fn hash_file_metadata_blake3(digest: &mut blake3::Hasher, metadata: &fs::Metadata) {
    digest.update(&[u8::from(metadata.permissions().readonly())]);
}

fn hash_environment(digest: &mut Sha256) {
    let mut environment = std::env::vars_os().collect::<Vec<_>>();
    environment.sort_by(|left, right| left.0.cmp(&right.0));
    digest.update(b"environment\0");
    for (key, value) in environment {
        hash_os(digest, &key);
        hash_os(digest, &value);
    }
}

fn provider_executables_identity(
    options: &aplan_evidence::VerifyOptions,
) -> Result<[u8; 32], String> {
    let mut digest = blake3::Hasher::new();
    digest.update(b"aplan-evidence-provider-executables-v1\0");
    let mut programs = BTreeSet::new();
    if let Some(runners) = &options.test_runner {
        for runner in runners.all() {
            if let Some(program) = runner.wrap.as_ref().and_then(|wrap| wrap.first()) {
                programs.insert(program.clone());
            }
            if let Some(program) = runner.list.first() {
                programs.insert(program.clone());
            }
        }
    }
    let mut uses_rustup_proxy = false;
    for program in programs {
        digest.update(b"program\0");
        digest.update(program.as_bytes());
        let path = resolve_program(&program)
            .ok_or_else(|| format!("configured provider executable `{program}` was not found"))?;
        uses_rustup_proxy |= path
            .canonicalize()
            .ok()
            .and_then(|target| target.file_name().map(std::ffi::OsStr::to_owned))
            .is_some_and(|name| name == "rustup");
        hash_path_blake3(&mut digest, &path);
        hash_external_identity_file(&mut digest, &path)?;
    }
    if uses_rustup_proxy {
        digest.update(&rustup_state_identity(rustup_home().as_deref())?);
    }
    Ok(*digest.finalize().as_bytes())
}

fn rustup_home() -> Option<PathBuf> {
    std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")))
}

/// Hash rustup's small installation manifests without spawning the toolchain.
///
/// Direct configured executables are byte-hashed above. A rustup `cargo` is a
/// proxy, though, so its bytes alone do not identify the selected compiler.
/// Rustup's settings and per-toolchain update hashes identify that installation
/// cheaply enough for the ordinary read path. Hashing every installed entry is
/// deliberately conservative: adding or updating an unrelated toolchain may
/// stale an observation, while silently accepting a changed active toolchain
/// would be the unsafe direction.
fn rustup_state_identity(root: Option<&Path>) -> Result<[u8; 32], String> {
    let mut digest = blake3::Hasher::new();
    digest.update(b"aplan-evidence-rustup-state-v1\0");
    let root = root.ok_or_else(|| {
        "the configured Rustup proxy has no identifiable RUSTUP_HOME or HOME".to_string()
    })?;
    hash_path_blake3(&mut digest, root);
    hash_external_identity_file(&mut digest, &root.join("settings.toml"))?;
    let update_hashes = root.join("update-hashes");
    let mut entries = fs::read_dir(&update_hashes)
        .map_err(|error| format!("reading `{}`: {error}", update_hashes.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("reading `{}`: {error}", update_hashes.display()))?;
    if entries.is_empty() {
        return Err(format!(
            "Rustup toolchain identity directory `{}` is empty",
            update_hashes.display()
        ));
    }
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        hash_external_identity_file(&mut digest, &entry.path())?;
    }
    Ok(*digest.finalize().as_bytes())
}

fn hash_external_identity_file(digest: &mut blake3::Hasher, path: &Path) -> Result<(), String> {
    hash_path_blake3(digest, path);
    let before_link = fs::symlink_metadata(path)
        .map_err(|error| format!("identifying `{}`: {error}", path.display()))?;
    if !before_link.is_file() && !before_link.file_type().is_symlink() {
        return Err(format!(
            "provider identity input `{}` is not a regular file",
            path.display()
        ));
    }
    let before =
        fs::metadata(path).map_err(|error| format!("identifying `{}`: {error}", path.display()))?;
    if !before.is_file() {
        return Err(format!(
            "provider identity input `{}` does not resolve to a regular file",
            path.display()
        ));
    }
    hash_file_metadata_blake3(digest, &before);
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "reading provider identity input `{}`: {error}",
            path.display()
        )
    })?;
    digest.update(&bytes);
    let after =
        fs::metadata(path).map_err(|error| format!("rechecking `{}`: {error}", path.display()))?;
    if !same_file_observation(&before, &after) {
        return Err(format!(
            "provider identity input `{}` changed while it was read",
            path.display()
        ));
    }
    Ok(())
}

fn resolve_program(program: &str) -> Option<PathBuf> {
    let direct = Path::new(program);
    if direct.components().count() > 1 {
        return direct.exists().then(|| direct.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

pub(crate) fn repository_identity(repo_root: &Path) -> Result<String, aplan_evidence::Error> {
    let repository = aplan_evidence::Repo::discover(repo_root)?;
    let mut digest = Sha256::new();
    digest.update(b"aplan-evidence-repository-v1\0");
    digest.update(root_identity(repo_root).as_bytes());
    digest.update(repository.head_sha().unwrap_or_default().as_bytes());
    Ok(hex(&digest.finalize()))
}

fn plan_identity(plan: &Plan) -> String {
    let mut digest = Sha256::new();
    digest.update(b"aplan-evidence-plan-v1\0");
    for loaded in &plan.tasks {
        hash_path(&mut digest, &loaded.path);
        digest.update(serde_json::to_vec(&loaded.task).unwrap_or_default());
    }
    hex(&digest.finalize())
}

fn configuration_identity(options: &aplan_evidence::VerifyOptions) -> String {
    let mut digest = Sha256::new();
    digest.update(b"aplan-evidence-configuration-v1\0");
    digest.update([u8::from(options.detect_drift)]);
    digest.update([u8::from(options.execute)]);
    digest.update(serde_json::to_vec(&options.traces_root).unwrap_or_default());
    digest.update(serde_json::to_vec(&options.max_unrelated_files).unwrap_or_default());
    digest.update(serde_json::to_vec(&options.test_runner).unwrap_or_default());
    digest.update(serde_json::to_vec(&options.observation_inputs).unwrap_or_default());
    digest.update(serde_json::to_vec(&options.url_check).unwrap_or_default());
    digest.update(serde_json::to_vec(&options.config_file).unwrap_or_default());
    digest.update(serde_json::to_vec(&options.version).unwrap_or_default());
    hex(&digest.finalize())
}

pub(crate) fn resolution_identity(
    plan: &Plan,
    options: &aplan_evidence::VerifyOptions,
    repository: &str,
    provider: Option<&str>,
    observed: &aplan_evidence::ObservedVerification,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"aplan-evidence-resolution-v1\0");
    digest.update(plan_identity(plan).as_bytes());
    digest.update(configuration_identity(options).as_bytes());
    digest.update(repository.as_bytes());
    digest.update(provider.unwrap_or("no-provider-observation").as_bytes());
    // The repository token binds the source revision and selected root. The
    // exact resolver answer additionally binds index/worktree-dependent facts
    // (for example the tracked-path set used by APLAN019) without a second,
    // redundant full-worktree status traversal after verification already
    // computed those facts.
    digest.update(observed.report.tasks_checked.to_le_bytes());
    for diagnostic in &observed.report.diagnostics {
        digest.update(diagnostic.code.as_bytes());
        digest.update([0]);
        digest.update(diagnostic.severity.to_string().as_bytes());
        digest.update([0]);
        if let Some(task) = &diagnostic.task {
            digest.update(task.as_bytes());
        }
        digest.update([0]);
        if let Some(file) = &diagnostic.file {
            hash_path(&mut digest, file);
        }
        digest.update(diagnostic.message.as_bytes());
        digest.update([0]);
        if let Some(instruction) = &diagnostic.agent_instruction {
            digest.update(instruction.as_bytes());
        }
        digest.update([0]);
        digest.update(serde_json::to_vec(&diagnostic.repair).unwrap_or_default());
        if let Some(location) = diagnostic.location {
            digest.update(location.line.to_le_bytes());
            digest.update(location.column.to_le_bytes());
        }
        for detail in &diagnostic.detail {
            digest.update(detail.as_bytes());
            digest.update([0]);
        }
        digest.update([0xff]);
    }
    for citation in &observed.citations {
        digest.update(citation.task.as_bytes());
        digest.update([0]);
        digest.update(citation.kind.as_str().as_bytes());
        digest.update([0]);
        digest.update(citation.reference.as_bytes());
        digest.update([0]);
        digest.update(citation.resolution.resolver.as_bytes());
        digest.update([0]);
        digest.update(citation.resolution.strength.label().as_bytes());
        digest.update([0]);
        digest.update(citation.resolution.outcome.label().as_bytes());
        digest.update([0]);
        if let Some(detail) = &citation.resolution.detail {
            digest.update(detail.as_bytes());
        }
        digest.update([0]);
        if let Some(subject) = &citation.resolution.cost_subject {
            digest.update(subject.as_bytes());
        }
        digest.update([0]);
    }
    hex(&digest.finalize())
}

fn observation_identity(
    provider: &str,
    material: &str,
    catalogue: Option<&StoredCatalogue>,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"aplan-evidence-observation-v1\0");
    digest.update(provider.as_bytes());
    digest.update(material.as_bytes());
    digest.update(serde_json::to_vec(&catalogue).unwrap_or_default());
    hex(&digest.finalize())
}

fn root_identity(root: &Path) -> String {
    let absolute = root.canonicalize().unwrap_or_else(|_| absolute_path(root));
    let mut digest = Sha256::new();
    hash_path(&mut digest, &absolute);
    hex(&digest.finalize())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn cache_file(root: &Path, options: &EvidenceObservationOptions) -> Option<PathBuf> {
    let base = options
        .cache_root
        .clone()
        .or_else(|| std::env::var_os("APLAN_CACHE_DIR").map(PathBuf::from))
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(
        base.join("aplan")
            .join("evidence-v1")
            .join(format!("{}.json", root_identity(root))),
    )
}

fn read_observation(
    root: &Path,
    options: &EvidenceObservationOptions,
) -> Option<StoredObservation> {
    let path = cache_file(root, options)?;
    safe_store_namespace(path.parent()?)?;
    let metadata = safe_private_regular_file(&path)?;
    if metadata.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).ok()?);
    fs::File::open(&path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let envelope: Envelope = serde_json::from_slice(&bytes).ok()?;
    let body = serde_json::to_vec(&envelope.observation).ok()?;
    if envelope.magic != MAGIC
        || envelope.checksum != hex(&Sha256::digest(&body))
        || envelope.observation.root != root_identity(root)
    {
        return None;
    }
    Some(envelope.observation)
}

fn publish_observation(
    root: &Path,
    options: &EvidenceObservationOptions,
    observation: &StoredObservation,
) {
    let Some(path) = cache_file(root, options) else {
        return;
    };
    let Some(directory) = path.parent() else {
        return;
    };
    if prepare_store_directory(directory).is_err() {
        return;
    }
    let lock = path.with_extension("lock");
    let Some(_lock) = CacheLock::acquire(&lock) else {
        return;
    };
    if read_observation(root, options)
        .is_some_and(|current| current.started_at_epoch_nanos >= observation.started_at_epoch_nanos)
    {
        return;
    }
    let Ok(body) = serde_json::to_vec(observation) else {
        return;
    };
    let envelope = Envelope {
        magic: MAGIC.into(),
        checksum: hex(&Sha256::digest(&body)),
        observation: observation.clone(),
    };
    let bytes = match serde_json::to_vec(&envelope) {
        Ok(bytes) if bytes.len() as u64 <= MAX_BYTES => bytes,
        _ => return,
    };
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(
        ".evidence-{}-{}-{}.tmp",
        std::process::id(),
        epoch_nanos(),
        sequence
    ));
    let wrote = private_create_new(&temporary)
        .and_then(|mut file| file.write_all(&bytes))
        .and_then(|()| fs::rename(&temporary, &path));
    if wrote.is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

#[derive(Debug)]
struct CacheLock {
    path: PathBuf,
}

impl CacheLock {
    fn acquire(path: &Path) -> Option<Self> {
        for _ in 0..LOCK_ATTEMPTS {
            match private_create_new(path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Some(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if stale_regular_file(path) {
                        let _ = fs::remove_file(path);
                    } else {
                        std::thread::yield_now();
                    }
                }
                Err(_) => return None,
            }
        }
        None
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn stale_regular_file(path: &Path) -> bool {
    safe_regular_file(path)
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > STALE_LOCK_AFTER)
}

fn prepare_store_directory(directory: &Path) -> std::io::Result<()> {
    let namespace = directory
        .parent()
        .ok_or_else(|| std::io::Error::other("evidence cache has no namespace parent"))?;
    let base = namespace
        .parent()
        .ok_or_else(|| std::io::Error::other("evidence cache has no base directory"))?;
    fs::create_dir_all(base)?;
    ensure_private_child(namespace)?;
    ensure_private_child(directory)
}

fn ensure_private_child(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(std::io::Error::other("unsafe evidence cache namespace"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(path)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(std::io::Error::other("unsafe evidence cache namespace"));
                }
            }
            Err(error) => return Err(error),
        },
        Err(error) => return Err(error),
    }
    set_private_directory_permissions(path)
}

fn safe_store_namespace(directory: &Path) -> Option<()> {
    let namespace = directory.parent()?;
    private_permissions(&safe_directory(namespace)?).then_some(())?;
    private_permissions(&safe_directory(directory)?).then_some(())
}

fn safe_directory(path: &Path) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    (!metadata.file_type().is_symlink() && metadata.is_dir()).then_some(metadata)
}

fn safe_regular_file(path: &Path) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    (!metadata.file_type().is_symlink() && metadata.is_file()).then_some(metadata)
}

fn safe_private_regular_file(path: &Path) -> Option<fs::Metadata> {
    let metadata = safe_regular_file(path)?;
    private_permissions(&metadata).then_some(metadata)
}

#[cfg(unix)]
fn private_permissions(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode().trailing_zeros() >= 6
}

#[cfg(not(unix))]
fn private_permissions(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn private_create_new(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_create_new(path: &Path) -> std::io::Result<fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn epoch_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn hash_path(digest: &mut Sha256, path: &Path) {
    hash_os(digest, path.as_os_str());
}

fn hash_path_blake3(digest: &mut blake3::Hasher, path: &Path) {
    hash_os_blake3(digest, path.as_os_str());
}

#[cfg(unix)]
fn hash_os(digest: &mut Sha256, value: &std::ffi::OsStr) {
    use std::os::unix::ffi::OsStrExt as _;
    digest.update(value.as_bytes());
    digest.update([0]);
}

#[cfg(unix)]
fn hash_os_blake3(digest: &mut blake3::Hasher, value: &std::ffi::OsStr) {
    use std::os::unix::ffi::OsStrExt as _;
    digest.update(value.as_bytes());
    digest.update(&[0]);
}

#[cfg(not(unix))]
fn hash_os(digest: &mut Sha256, value: &std::ffi::OsStr) {
    digest.update(value.to_string_lossy().as_bytes());
    digest.update([0]);
}

#[cfg(not(unix))]
fn hash_os_blake3(digest: &mut blake3::Hasher, value: &std::ffi::OsStr) {
    digest.update(value.to_string_lossy().as_bytes());
    digest.update(&[0]);
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        EvidenceObservationOptions,
    ) {
        let root = tempfile::Builder::new()
            .prefix("observation-root-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("root fixture");
        let cache = tempfile::Builder::new()
            .prefix("observation-cache-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("cache fixture");
        let options = EvidenceObservationOptions {
            cache_root: Some(cache.path().to_path_buf()),
        };
        (root, cache, options)
    }

    fn stored(started_at_epoch_nanos: u128, identity: &str) -> StoredObservation {
        StoredObservation {
            protocol: PROTOCOL,
            root: String::new(),
            provider: "provider".to_string(),
            material: "material".to_string(),
            identity: identity.to_string(),
            observed_at_epoch_seconds: 1,
            started_at_epoch_nanos,
            complete: true,
            failure: None,
            catalogue: Some(StoredCatalogue {
                runners: Vec::new(),
            }),
            derived: None,
        }
    }

    fn publish(root: &Path, options: &EvidenceObservationOptions, mut value: StoredObservation) {
        value.root = root_identity(root);
        publish_observation(root, options, &value);
    }

    fn one_runner() -> aplan_evidence::TestRunners {
        aplan_config::parse_config(
            "test:\n  list: [printf, 'suite::one: test\\n']\n  run: ['true', '{test}']\n",
            Path::new(".aplan.yml"),
        )
        .expect("runner config")
        .test
        .expect("runner")
    }

    fn plan_with_evidence(kind: &str, reference: &str) -> Plan {
        aplan_core::load_plan_files(
            Path::new("."),
            [(
                PathBuf::from("T-001.md"),
                format!(
                    "---\nid: T-001\ntitle: Observe evidence\nphase: 1\nstatus: todo\nevidence:\n  - kind: {kind}\n    ref: {reference}\n---\n"
                )
                .into_bytes(),
            )],
        )
    }

    #[test]
    fn retained_catalogue_shape_cannot_fall_back_to_provider_execution() {
        let mut value = stored(1, "observation");
        let runners = one_runner();
        let options = aplan_evidence::VerifyOptions {
            test_runner: Some(runners.clone()),
            ..aplan_evidence::VerifyOptions::default()
        };
        let state = stored_test_state(
            Some(&value),
            "provider",
            &MaterialIdentity::Identified("material".to_string()),
            &options,
        );
        assert_eq!(state.0, EvidenceObservationState::RefreshFailed);
        assert!(state.1.is_none(), "a malformed catalogue must not be used");

        value.catalogue = Some(StoredCatalogue {
            runners: vec![aplan_evidence::TestCatalogueRunner {
                index: 1,
                label: "runner 1".to_string(),
                state: aplan_evidence::TestCatalogueRunnerState::Empty,
                names: Vec::new(),
            }],
        });
        let state = stored_test_state(
            Some(&value),
            "provider",
            &MaterialIdentity::Identified("material".to_string()),
            &options,
        );
        assert_eq!(state.0, EvidenceObservationState::Current);
        assert!(state.1.is_some(), "the exact configured shape is reusable");
    }

    #[test]
    fn retained_observation_failures_remain_typed_and_actionable() {
        let no_runner = stored_test_state(
            None,
            "provider",
            &MaterialIdentity::Identified("material".to_string()),
            &aplan_evidence::VerifyOptions::default(),
        );
        assert_eq!(no_runner.0, EvidenceObservationState::Unsupported);
        assert!(no_runner.4.contains("no test evidence provider"));

        let options = aplan_evidence::VerifyOptions {
            test_runner: Some(one_runner()),
            ..aplan_evidence::VerifyOptions::default()
        };
        let unsupported = stored_test_state(
            Some(&stored(1, "observation")),
            "provider",
            &MaterialIdentity::Unsupported("material cannot be identified".to_string()),
            &options,
        );
        assert_eq!(unsupported.0, EvidenceObservationState::Unsupported);
        assert_eq!(unsupported.4, "material cannot be identified");

        let mut failed = stored(2, "observation");
        failed.complete = false;
        failed.failure = Some("runner did not list tests".to_string());
        let failed_state = stored_test_state(
            Some(&failed),
            "provider",
            &MaterialIdentity::Identified("material".to_string()),
            &options,
        );
        assert_eq!(failed_state.0, EvidenceObservationState::RefreshFailed);
        assert_eq!(failed_state.4, "runner did not list tests");

        failed.failure = None;
        assert!(
            stored_test_state(
                Some(&failed),
                "provider",
                &MaterialIdentity::Identified("material".to_string()),
                &options,
            )
            .4
            .contains("last refresh was incomplete")
        );

        failed.complete = true;
        failed.catalogue = None;
        let missing_catalogue = stored_test_state(
            Some(&failed),
            "provider",
            &MaterialIdentity::Identified("material".to_string()),
            &options,
        );
        assert_eq!(missing_catalogue.0, EvidenceObservationState::RefreshFailed);
        assert!(
            missing_catalogue
                .4
                .contains("retained observation is incomplete")
        );
    }

    #[test]
    fn refresh_movement_and_unobserved_url_projection_say_exactly_what_happened() {
        let identified = MaterialIdentity::Identified("before".to_string());
        assert_eq!(
            refresh_failure(true, &identified, None).as_deref(),
            Some("declared provider inputs changed while refresh was running")
        );
        let moved = refreshed_state_and_detail(
            true,
            true,
            true,
            &identified,
            &MaterialIdentity::Identified("after".to_string()),
            None,
        );
        assert_eq!(moved.0, EvidenceObservationState::Stale);
        assert!(moved.1.contains("changed while refresh was running"));

        let plan = plan_with_evidence("url", "https://example.invalid/proof");
        assert!(plan.load_diagnostics.is_empty());
        let projected = project_inspection(
            &plan,
            aplan_evidence::ObservedVerification {
                report: Report::default(),
                spend: Vec::new(),
                citations: Vec::new(),
            },
            EvidenceObservationScope::SelectedTasks,
            EvidenceProviderSource::Deferred,
            EvidenceObservationCompleteness::Partial,
            EvidenceObservationState::Unsupported,
            EvidenceObservationState::Unsupported,
            None,
            None,
            None,
            "resolver was unavailable".to_string(),
        );
        assert_eq!(projected.citations.len(), 1);
        assert_eq!(
            projected.citation_states,
            vec![EvidenceObservationState::Unsupported]
        );
        assert!(
            projected
                .observation
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("URL observations are not retained"))
        );
    }

    #[test]
    fn empty_material_sets_and_evidence_policy_changes_are_not_current_by_accident() {
        let (root, _cache, _store) = fixture();
        let no_match = aplan_evidence::VerifyOptions {
            observation_inputs: Some(vec!["missing/**".to_string()]),
            ..aplan_evidence::VerifyOptions::default()
        };
        assert!(matches!(
            material_identity(root.path(), &no_match),
            MaterialIdentity::Unsupported(reason) if reason.contains("matched no regular files")
        ));

        let baseline = aplan_evidence::VerifyOptions::default();
        let changed = aplan_evidence::VerifyOptions {
            max_unrelated_files: Some(7),
            ..baseline.clone()
        };
        assert_ne!(
            configuration_identity(&baseline),
            configuration_identity(&changed),
            "a policy change can change a derived evidence verdict"
        );
    }

    #[test]
    fn malformed_moving_and_non_file_material_inputs_fail_closed() {
        let (root, _cache, _store) = fixture();
        let invalid_glob = aplan_evidence::VerifyOptions {
            observation_inputs: Some(vec!["[".to_string()]),
            ..aplan_evidence::VerifyOptions::default()
        };
        assert!(matches!(
            material_identity(root.path(), &invalid_glob),
            MaterialIdentity::Unsupported(reason) if !reason.is_empty()
        ));

        let directory = root.path().join("inputs");
        fs::create_dir(&directory).expect("input directory");
        let directory_metadata = fs::symlink_metadata(&directory).expect("directory metadata");
        let not_file =
            declared_input_identity(root.path(), Path::new("inputs"), &directory_metadata)
                .expect_err("a directory is not an exact file input");
        assert!(not_file.contains("not a regular file"));

        let moving = directory.join("moving.txt");
        fs::write(&moving, "before\n").expect("initial input");
        let before = fs::symlink_metadata(&moving).expect("initial metadata");
        fs::write(&moving, "after, with another length\n").expect("move input");
        let changed = declared_input_identity(root.path(), Path::new("inputs/moving.txt"), &before)
            .expect_err("changed input");
        assert!(changed.contains("changed while it was read"));

        let missing = directory.join("missing.txt");
        fs::write(&missing, "present\n").expect("temporary input");
        let before = fs::symlink_metadata(&missing).expect("temporary metadata");
        fs::remove_file(&missing).expect("remove temporary input");
        let unreadable =
            declared_input_identity(root.path(), Path::new("inputs/missing.txt"), &before)
                .expect_err("missing input");
        assert!(unreadable.contains("could not be read"));

        let input = directory.join("source.txt");
        fs::write(&input, "source\n").expect("provider input");
        let nonexistent_program = aplan_config::parse_config(
            "test:\n  list: [aplan-provider-that-does-not-exist]\n  run: ['true', '{test}']\n",
            Path::new(".aplan.yml"),
        )
        .expect("runner config")
        .test
        .expect("runner");
        let missing_executable = material_identity(
            root.path(),
            &aplan_evidence::VerifyOptions {
                test_runner: Some(nonexistent_program),
                observation_inputs: Some(vec!["inputs/**".to_string()]),
                ..aplan_evidence::VerifyOptions::default()
            },
        );
        assert!(matches!(
            missing_executable,
            MaterialIdentity::Unsupported(reason) if reason.contains("was not found")
        ));
    }

    #[test]
    fn observation_publication_is_atomic_private_and_rejects_late_results() {
        let (root, _cache, options) = fixture();
        publish(root.path(), &options, stored(20, "newer"));
        publish(root.path(), &options, stored(10, "older"));
        let retained = read_observation(root.path(), &options).expect("complete envelope");
        assert_eq!(
            retained.identity, "newer",
            "late old work replaced new work"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let path = cache_file(root.path(), &options).expect("cache path");
            assert_eq!(
                path.metadata().expect("metadata").permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                path.parent()
                    .expect("directory")
                    .metadata()
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn corrupt_oversized_or_stale_locked_observations_fail_closed() {
        let (root, _cache, options) = fixture();
        publish(root.path(), &options, stored(1, "valid"));
        let path = cache_file(root.path(), &options).expect("cache path");
        fs::write(&path, b"{\"not\":\"an envelope\"}").expect("corrupt cache");
        assert!(read_observation(root.path(), &options).is_none());

        let oversized = fs::File::options()
            .write(true)
            .open(&path)
            .expect("open cache");
        oversized.set_len(MAX_BYTES + 1).expect("oversize cache");
        assert!(read_observation(root.path(), &options).is_none());

        let lock = path.with_extension("lock");
        private_create_new(&lock).expect("stale lock");
        fs::File::options()
            .write(true)
            .open(&lock)
            .and_then(|file| file.set_times(std::fs::FileTimes::new().set_modified(UNIX_EPOCH)))
            .expect("age lock");
        publish(root.path(), &options, stored(2, "recovered"));
        assert_eq!(
            read_observation(root.path(), &options)
                .expect("recovered envelope")
                .identity,
            "recovered"
        );
        assert!(!lock.exists());
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_cache_namespace_and_symlinked_material_are_safe_misses() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let (root, cache, options) = fixture();
        let redirected = cache.path().join("redirected");
        fs::create_dir(&redirected).expect("redirect target");
        symlink(&redirected, cache.path().join("aplan")).expect("namespace symlink");
        publish(root.path(), &options, stored(1, "must-not-write"));
        assert!(!redirected.join("evidence-v1").exists());
        assert!(read_observation(root.path(), &options).is_none());

        fs::remove_file(cache.path().join("aplan")).expect("remove symlink");
        publish(root.path(), &options, stored(2, "private"));
        let path = cache_file(root.path(), &options).expect("cache path");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("open permissions");
        assert!(read_observation(root.path(), &options).is_none());

        let external = root.path().join("external");
        fs::create_dir(&external).expect("external directory");
        fs::write(external.join("source.rs"), "fn external() {}\n").expect("external source");
        symlink(&external, root.path().join("inputs")).expect("input symlink");
        let identity = material_identity(
            root.path(),
            &aplan_evidence::VerifyOptions {
                observation_inputs: Some(vec!["inputs/**".to_string()]),
                ..aplan_evidence::VerifyOptions::default()
            },
        );
        assert!(
            matches!(identity, MaterialIdentity::Unsupported(reason) if reason.contains("symbolic link"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn material_identity_is_deterministic_and_includes_bytes_and_executable_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        let (root, _cache, _options) = fixture();
        let inputs = root.path().join("inputs");
        fs::create_dir(&inputs).expect("input directory");
        let input = inputs.join("runner.sh");
        fs::write(&input, "#!/bin/sh\nexit 0\n").expect("input");
        for index in 0..40 {
            fs::write(
                inputs.join(format!("input-{index:02}.txt")),
                format!("input {index}\n"),
            )
            .expect("parallel input");
        }
        fs::set_permissions(&input, fs::Permissions::from_mode(0o600)).expect("permissions");
        let options = aplan_evidence::VerifyOptions {
            observation_inputs: Some(vec!["inputs/**".to_string()]),
            ..aplan_evidence::VerifyOptions::default()
        };
        let before = material_identity(root.path(), &options)
            .as_identified()
            .expect("identified")
            .to_string();
        assert_eq!(
            material_identity(root.path(), &options).as_identified(),
            Some(before.as_str()),
            "parallel input collection must retain canonical order"
        );

        fs::write(&input, "#!/bin/sh\nexit 1\n").expect("same-length replacement");
        let changed_bytes = material_identity(root.path(), &options)
            .as_identified()
            .expect("identified")
            .to_string();
        assert_ne!(before, changed_bytes, "same-length byte edit was missed");

        fs::set_permissions(&input, fs::Permissions::from_mode(0o700)).expect("permissions");
        let after = material_identity(root.path(), &options)
            .as_identified()
            .expect("identified")
            .to_string();
        assert_ne!(
            changed_bytes, after,
            "chmod changed provider behavior but not identity"
        );
    }

    #[test]
    fn rustup_manifest_changes_move_the_toolchain_identity_without_spawning() {
        let directory = tempfile::Builder::new()
            .prefix("observation-rustup-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("rustup fixture");
        let updates = directory.path().join("update-hashes");
        fs::create_dir(&updates).expect("update hashes");
        fs::write(
            directory.path().join("settings.toml"),
            "default_toolchain = 'stable'\n",
        )
        .expect("settings");
        let manifest = updates.join("stable-host");
        fs::write(&manifest, "aaaaaaaaaaaaaaaaaaaa").expect("manifest");
        let before = rustup_state_identity(Some(directory.path())).expect("initial identity");

        fs::write(&manifest, "bbbbbbbbbbbbbbbbbbbb").expect("same-size manifest edit");
        let after = rustup_state_identity(Some(directory.path())).expect("changed identity");
        assert_ne!(before, after, "toolchain manifest change was missed");
        assert_eq!(
            after,
            rustup_state_identity(Some(directory.path())).expect("repeated identity"),
            "toolchain identity is not deterministic"
        );
    }

    #[test]
    fn toolchain_identity_refuses_missing_empty_and_non_file_manifests() {
        assert!(
            rustup_state_identity(None)
                .expect_err("missing home")
                .contains("no identifiable RUSTUP_HOME or HOME")
        );

        let directory = tempfile::Builder::new()
            .prefix("observation-rustup-invalid-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("rustup fixture");
        fs::write(
            directory.path().join("settings.toml"),
            "profile = 'minimal'\n",
        )
        .expect("settings");
        fs::create_dir(directory.path().join("update-hashes")).expect("update hashes");
        assert!(
            rustup_state_identity(Some(directory.path()))
                .expect_err("empty manifests")
                .contains("is empty")
        );

        let mut digest = blake3::Hasher::new();
        assert!(
            hash_external_identity_file(&mut digest, directory.path())
                .expect_err("directory identity")
                .contains("not a regular file")
        );
        assert!(
            hash_external_identity_file(&mut digest, &directory.path().join("absent"))
                .expect_err("missing identity")
                .contains("identifying")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = directory.path().join("directory-link");
            symlink(directory.path(), &link).expect("directory symlink");
            assert!(
                hash_external_identity_file(&mut digest, &link)
                    .expect_err("directory symlink identity")
                    .contains("does not resolve to a regular file")
            );
        }

        let direct = directory.path().join("provider");
        fs::write(&direct, "provider bytes\n").expect("provider");
        assert_eq!(
            resolve_program(direct.to_str().expect("utf8 path")),
            Some(direct)
        );
        assert!(absolute_path(Path::new("relative")).is_absolute());
        assert_eq!(absolute_path(directory.path()), directory.path());
    }

    #[test]
    fn test_catalogue_identity_does_not_depend_on_uncached_url_policy() {
        let baseline = aplan_evidence::VerifyOptions::default();
        let with_url = aplan_evidence::VerifyOptions {
            url_check: Some(aplan_config::UrlCheck {
                check: vec!["true".to_string(), "{url}".to_string()],
                timeout_seconds: None,
                max_memory: None,
                max_processes: None,
                max_output: None,
                wrap: None,
            }),
            ..baseline.clone()
        };
        assert_eq!(provider_identity(&baseline), provider_identity(&with_url));
    }
}
