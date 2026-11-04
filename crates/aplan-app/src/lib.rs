//! Typed application operations shared by delivery adapters.
//!
//! This crate is deliberately smaller than a command framework. It owns
//! concrete requests and responses for use cases that otherwise have to be
//! orchestrated independently by the CLI, MCP and an interactive client. The
//! persisted model, graph and diagnostics remain [`aplan_core`] types; there is
//! no second `Plan` and no transport-shaped request map.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub use aplan_core::Severity;
use aplan_core::reach::{Edges, Reach};
pub use aplan_core::select::Selector;
pub use aplan_core::show::{
    Cited, EvidenceObservation, EvidenceObservationCompleteness, EvidenceObservationScope,
    EvidenceObservationState, EvidenceProviderSource, Finding, Related, RelatedAvailability, Show,
};
pub use aplan_core::task::Status;
use aplan_core::task::Task;
use aplan_core::{ClaimSnapshot, EdgeKind, Graph, Options, Plan, Report, Verdict};
pub use aplan_evidence::{CitationOutcome, CitationStrength, Cost as VerificationCost};
use aplan_output::GraphView;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

mod reports;
pub use reports::*;
mod progress;
pub use progress::*;
mod graph_window;
pub use graph_window::*;
mod graph_explorer;
pub use graph_explorer::*;
mod cell_scene;
pub use cell_scene::*;
mod configuration;
pub use configuration::*;
mod work;
pub use work::*;
mod capabilities;
pub use capabilities::*;
pub mod claims;
mod onboarding;
pub use onboarding::*;
mod store;
pub use store::*;
mod mutation;
pub use mutation::*;
mod capture;
pub use capture::*;
mod work_action;
pub use work_action::*;
mod evidence_mutation;
pub use evidence_mutation::*;
pub mod authoring;
pub use authoring::*;
pub mod planning;
pub use planning::*;
mod portfolio;
pub use portfolio::*;
mod test_catalogue;
pub use test_catalogue::*;
mod evidence_observation;
pub use evidence_observation::*;
mod receipt_observation;
pub use receipt_observation::*;
mod incremental_receipt;
pub use incremental_receipt::*;

/// The trust boundary under which an application operation runs.
///
/// The two operations proved in this crate are hermetic. Naming the mode in
/// the context keeps a later repository-reading or process-running operation
/// from silently inheriting that claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// Read identified plan and explicitly configured document bytes and write
    /// through the guarded plan writer; open no repository, network connection
    /// or configured process.
    Hermetic,
    /// Repository reads are permitted, but configured processes and the network
    /// are not.
    RepositoryRead,
    /// The caller explicitly permitted configured processes.
    ConfiguredProcesses,
}

/// The selected configuration location, including absence as an identified
/// input.
///
/// The operation's resolved [`Options`], [`Verdict`] and (when attached) full
/// report configuration are hashed as the policy meaning at this boundary.
/// Preview construction never pairs a newly read file with older resolved
/// policy. Publication re-resolves a selected file into a separate current
/// identity and refuses when that meaning differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigurationIdentity {
    /// The selected file, or `None` when configuration was explicitly refused
    /// or discovery found none.
    pub path: Option<PathBuf>,
    /// Digest of the resolved configuration semantics used by this snapshot.
    /// This is deliberately not a digest of bytes re-read after resolution.
    pub sha256: String,
    /// Exact selected-file bytes captured with this context. Application hosts
    /// which can prove those bytes produced the resolved configuration record
    /// them here; direct embedders may explicitly leave them unobserved.
    pub content: ConfigurationContentIdentity,
}

/// Observation of the selected configuration file's exact bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConfigurationContentIdentity {
    /// A selected file exists, but this embedding did not capture its bytes.
    NotObserved,
    /// Configuration selection deliberately read no file.
    Absent,
    /// Exact bytes were captured and matched the resolved configuration.
    Observed { sha256: String },
    /// A previously captured selected file can no longer be read.
    Unavailable { reason: String },
}

impl ConfigurationIdentity {
    fn selected(path: Option<&Path>, options: &Options, verdict: &Verdict) -> Self {
        Self {
            path: path.map(absolute),
            sha256: validation_policy_digest(options, verdict),
            content: if path.is_some() {
                ConfigurationContentIdentity::NotObserved
            } else {
                ConfigurationContentIdentity::Absent
            },
        }
    }
}

fn report_configuration_identity(
    path: Option<&Path>,
    options: &Options,
    verdict: &Verdict,
    config: &aplan_config::Config,
    content: ConfigurationContentIdentity,
) -> ConfigurationIdentity {
    let selected = ConfigurationIdentity::selected(path, options, verdict);
    let mut digest = Sha256::new();
    hash_part(
        &mut digest,
        b"validation_policy",
        selected.sha256.as_bytes(),
    );
    let config_bytes = serde_json::to_vec(config)
        .expect("Config contains only canonically serialisable typed values");
    hash_part(&mut digest, b"resolved_config", &config_bytes);
    ConfigurationIdentity {
        path: selected.path,
        sha256: to_hex(&digest.finalize()),
        content,
    }
}

fn unavailable_configuration_identity(
    path: &Path,
    reason: &str,
    content: ConfigurationContentIdentity,
) -> ConfigurationIdentity {
    let mut digest = Sha256::new();
    hash_part(&mut digest, b"configuration_unavailable", reason.as_bytes());
    ConfigurationIdentity {
        path: Some(absolute(path)),
        sha256: to_hex(&digest.finalize()),
        content,
    }
}

/// Why exact selected-configuration capture could not join an application
/// snapshot.
#[derive(Debug, thiserror::Error)]
pub enum ConfigurationCaptureError {
    #[error("reading selected configuration {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the selected configuration {} changed while the application snapshot was being prepared: {detail}", path.display())]
    Changed { path: PathBuf, detail: String },
}

/// The repository observation captured with an identified application input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RepositoryIdentity {
    /// The operation's declared trust boundary did not permit a repository read.
    NotObserved { reason: String },
    /// Repository reads were permitted, but discovery or status failed.
    Unavailable { reason: String },
    /// HEAD and exact changed work-tree material were captured together for
    /// stale-input checks.
    Observed {
        head: Option<String>,
        modified: usize,
        untracked: usize,
        status_sha256: String,
    },
}

/// The complete input identity an operation was computed against.
///
/// The digest covers the exact candidate bytes parsed by the identified plan
/// load, the selected configuration location, roots, resolved
/// validation/verdict policy and trust mode. It is an internal stale-result
/// identity, not a persisted cache key, a filesystem transaction or an
/// authentication claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputIdentity {
    pub source: &'static str,
    pub plan_root: PathBuf,
    pub plan_sha256: String,
    pub repository_root: PathBuf,
    pub repository: RepositoryIdentity,
    pub configuration: ConfigurationIdentity,
    pub trust: Trust,
    pub sha256: String,
}

/// Resolved inputs from which one immutable application snapshot is built.
#[derive(Debug)]
pub struct ContextInputs {
    pub repository_root: PathBuf,
    pub configuration_file: Option<PathBuf>,
    pub trust: Trust,
    pub options: Options,
    pub verdict: Verdict,
}

/// How a configured document declares a cut's members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentMembership {
    Line(String),
    Table,
}

/// Bytes and interpretation for one configured `check-docs` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentInput {
    /// The stable label shown to the user, normally relative to the selected
    /// configuration directory.
    pub label: String,
    pub path: PathBuf,
    /// Exact text read for this snapshot, or the read failure that prevented it.
    pub content: Result<String, String>,
    pub membership: Option<DocumentMembership>,
}

/// Why configured document validation can or cannot run in this snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentInputState {
    Configured,
    NotConfigured,
    Unavailable(String),
}

/// The complete configured document input selected by the delivery host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentInputs {
    pub state: DocumentInputState,
    pub documents: Vec<DocumentInput>,
}

impl Default for DocumentInputs {
    fn default() -> Self {
        Self {
            state: DocumentInputState::NotConfigured,
            documents: Vec::new(),
        }
    }
}

/// One loaded plan and the resolved policy, repository and trust inputs used to
/// answer questions about it.
#[derive(Debug)]
pub struct PlanContext {
    plan: Plan,
    task_sources: BTreeMap<String, PathBuf>,
    source_inputs: BTreeMap<PathBuf, aplan_core::load::PlanFileContent>,
    documents: DocumentInputs,
    documents_identified: bool,
    report_config: Option<aplan_config::Config>,
    options: Options,
    verdict: Verdict,
    base_sha256: String,
    identity: InputIdentity,
    structural_report: std::sync::OnceLock<Report>,
    report_publisher: Option<StructuralReportPublisher>,
}

#[derive(Clone)]
struct StructuralReportPublisher(std::sync::Arc<dyn Fn(&Report) + Send + Sync + 'static>);

impl std::fmt::Debug for StructuralReportPublisher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StructuralReportPublisher(..)")
    }
}

/// A report already resolved by the shared store plus the guarded publication
/// hook for a miss.
pub struct StructuralReportCache {
    cached: Option<Report>,
    publish: StructuralReportPublisher,
}

impl std::fmt::Debug for StructuralReportCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StructuralReportCache")
            .field("cached", &self.cached.is_some())
            .finish_non_exhaustive()
    }
}

impl StructuralReportCache {
    #[must_use]
    pub fn new(cached: Option<Report>, publish: impl Fn(&Report) + Send + Sync + 'static) -> Self {
        Self {
            cached,
            publish: StructuralReportPublisher(std::sync::Arc::new(publish)),
        }
    }
}

impl PlanContext {
    /// Identify an already-loaded plan without constructing another model.
    ///
    /// Loading stays with the host for this first extraction because the CLI
    /// has a distinct usage error for a missing root while MCP is long-lived.
    /// Read operations answer from this snapshot and resolved policy. A write
    /// operation may refresh plan bytes immediately before deriving its guarded
    /// post-state. Publication does not adopt newly discovered policy; when the
    /// caller selected a configuration file, it re-resolves only that path and
    /// treats any semantic difference as stale.
    #[must_use]
    pub fn new(loaded: aplan_core::IdentifiedPlan, inputs: ContextInputs) -> Self {
        let configuration = ConfigurationIdentity::selected(
            inputs.configuration_file.as_deref(),
            &inputs.options,
            &inputs.verdict,
        );
        let plan_root = loaded.root.clone();
        let repository_root = absolute(&inputs.repository_root);
        let repository = repository_identity(&repository_root, inputs.trust);
        let plan_sha256 = plan_input_digest(&loaded.inputs, &plan_root);
        let sha256 = input_digest(
            &plan_sha256,
            &repository_root,
            &repository,
            &configuration,
            inputs.trust,
            &inputs.options,
            &inputs.verdict,
        );
        let source_inputs: BTreeMap<_, _> = loaded
            .inputs
            .into_iter()
            .map(|input| (input.path, input.content))
            .collect();
        let mut task_sources = BTreeMap::new();
        for task in &loaded.plan.tasks {
            let Some(aplan_core::load::PlanFileContent::Bytes(bytes)) =
                source_inputs.get(&task.path)
            else {
                continue;
            };
            let Some((_, _)) = std::str::from_utf8(bytes)
                .ok()
                .and_then(aplan_core::load::split_frontmatter)
            else {
                continue;
            };
            // Match `Graph::new`: the first definition is the one every
            // projection uses while APLAN001 reports the duplicate. Letting a
            // later duplicate overwrite only its body would pair two files.
            task_sources
                .entry(task.task.id.clone())
                .or_insert_with(|| task.path.clone());
        }
        Self {
            plan: loaded.plan,
            task_sources,
            source_inputs,
            documents: DocumentInputs::default(),
            documents_identified: false,
            report_config: None,
            options: inputs.options,
            verdict: inputs.verdict,
            base_sha256: sha256.clone(),
            identity: InputIdentity {
                source: "working_tree",
                plan_root,
                plan_sha256,
                repository_root,
                repository,
                configuration,
                trust: inputs.trust,
                sha256,
            },
            structural_report: std::sync::OnceLock::new(),
            report_publisher: None,
        }
    }

    /// Attach the structural-report row from the same shared [`PlanStore`]
    /// that supplied this snapshot.
    #[must_use]
    pub fn with_structural_report_cache(mut self, cache: StructuralReportCache) -> Self {
        if let Some(report) = cache.cached {
            let _ = self.structural_report.set(report);
        }
        self.report_publisher = Some(cache.publish);
        self
    }

    fn structural_report_with(&self, compute: impl FnOnce() -> Report) -> &Report {
        self.structural_report.get_or_init(|| {
            let report = compute();
            if let Some(publish) = &self.report_publisher {
                (publish.0)(&report);
            }
            report
        })
    }

    /// Attach configured document bytes to this immutable application input.
    ///
    /// The host resolves configuration paths and reads each document once. The
    /// digest is extended with exactly those results, so a later refresh cannot
    /// pair a plan result with document bytes from another instant.
    #[must_use]
    pub fn with_documents(mut self, documents: DocumentInputs) -> Self {
        self.documents = documents;
        self.documents_identified = true;
        self.refresh_auxiliary_identity();
        self
    }

    /// Attach the exact resolved configuration used by aggregate reports.
    ///
    /// Report policy is identified separately from validation policy because
    /// throughput, grouping, stale windows and version floors do not otherwise
    /// affect [`Options`] or [`Verdict`]. Keeping it on the context prevents an
    /// interactive refresh from combining a plan with a newer configuration.
    /// When a configuration path is selected, this semantic-only attachment is
    /// explicitly `not_observed` and a later publication fails closed; hosts
    /// which publish mutations must use [`Self::with_captured_report_config`].
    #[must_use]
    pub fn with_report_config(mut self, config: aplan_config::Config) -> Self {
        let content = self.identity.configuration.content.clone();
        self.identity.configuration = report_configuration_identity(
            self.identity.configuration.path.as_deref(),
            &self.options,
            &self.verdict,
            &config,
            content,
        );
        self.base_sha256 = input_digest(
            &self.identity.plan_sha256,
            &self.identity.repository_root,
            &self.identity.repository,
            &self.identity.configuration,
            self.identity.trust,
            &self.options,
            &self.verdict,
        );
        self.report_config = Some(config);
        self.refresh_auxiliary_identity();
        self
    }

    /// Attach resolved report configuration and bind it to the exact selected
    /// file bytes which produced it.
    ///
    /// This is the application-host boundary: reading and parsing happen from
    /// one byte buffer, and the method refuses a resolved value from another
    /// revision. Direct embedders which already own an atomic configuration
    /// source may use [`Self::with_report_config`], whose identity records the
    /// selected file as `not_observed` rather than pretending to have bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigurationCaptureError::Read`] if a selected file cannot
    /// be read, or [`ConfigurationCaptureError::Changed`] if its bytes are
    /// invalid or do not resolve to `config`.
    pub fn with_captured_report_config(
        mut self,
        config: aplan_config::Config,
    ) -> Result<Self, ConfigurationCaptureError> {
        let content = if let Some(path) = self.identity.configuration.path.as_deref() {
            let bytes = std::fs::read(path).map_err(|source| ConfigurationCaptureError::Read {
                path: path.to_path_buf(),
                source,
            })?;
            let parsed = aplan_config::parse_config_bytes(&bytes, path).map_err(|error| {
                ConfigurationCaptureError::Changed {
                    path: path.to_path_buf(),
                    detail: error.to_string(),
                }
            })?;
            let parsed = serde_json::to_vec(&parsed)
                .expect("Config contains only canonically serialisable typed values");
            let resolved = serde_json::to_vec(&config)
                .expect("Config contains only canonically serialisable typed values");
            if parsed != resolved {
                return Err(ConfigurationCaptureError::Changed {
                    path: path.to_path_buf(),
                    detail: "the selected bytes resolve to different configuration semantics"
                        .to_string(),
                });
            }
            ConfigurationContentIdentity::Observed {
                sha256: to_hex(&Sha256::digest(&bytes)),
            }
        } else {
            ConfigurationContentIdentity::Absent
        };
        self.identity.configuration = report_configuration_identity(
            self.identity.configuration.path.as_deref(),
            &self.options,
            &self.verdict,
            &config,
            content,
        );
        self.base_sha256 = input_digest(
            &self.identity.plan_sha256,
            &self.identity.repository_root,
            &self.identity.repository,
            &self.identity.configuration,
            self.identity.trust,
            &self.options,
            &self.verdict,
        );
        self.report_config = Some(config);
        self.refresh_auxiliary_identity();
        Ok(self)
    }

    fn refresh_auxiliary_identity(&mut self) {
        self.identity.sha256 = self.auxiliary_identity(&self.base_sha256);
    }

    fn auxiliary_identity(&self, plan_sha256: &str) -> String {
        let mut sha256 = plan_sha256.to_string();
        if self.documents_identified {
            sha256 = document_input_digest(&sha256, &self.documents);
        }
        sha256
    }

    #[must_use]
    pub const fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The body parsed from the same bytes as this context, never from a later
    /// filesystem read.
    #[must_use]
    pub fn task_body(&self, id: &str) -> Option<&str> {
        let path = self.task_sources.get(id)?;
        let aplan_core::load::PlanFileContent::Bytes(bytes) = self.source_inputs.get(path)? else {
            return None;
        };
        std::str::from_utf8(bytes)
            .ok()
            .and_then(aplan_core::load::split_frontmatter)
            .map(|(_, body)| body)
    }

    /// Exact UTF-8 plan-file text captured by this context.
    ///
    /// Mutation services use this instead of re-reading a path while preparing
    /// a preview. Publication still performs its own compare-and-swap check.
    pub(crate) fn source_text(&self, path: &Path) -> Option<&str> {
        self.source_inputs
            .iter()
            .find(|(candidate, _)| absolute(candidate) == absolute(path))
            .and_then(|(_, content)| match content {
                aplan_core::load::PlanFileContent::Bytes(bytes) => std::str::from_utf8(bytes).ok(),
                aplan_core::load::PlanFileContent::Sha256(_)
                | aplan_core::load::PlanFileContent::ReadError { .. } => None,
            })
    }

    #[must_use]
    pub const fn options(&self) -> &Options {
        &self.options
    }

    #[must_use]
    pub const fn verdict(&self) -> &Verdict {
        &self.verdict
    }

    #[must_use]
    pub const fn identity(&self) -> &InputIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn documents(&self) -> &DocumentInputs {
        &self.documents
    }

    /// Configuration resolved from the same host read as this snapshot.
    #[must_use]
    pub fn report_config(&self) -> Option<&aplan_config::Config> {
        self.report_config.as_ref()
    }

    fn source_line(&self, path: &Path, line: usize) -> Option<String> {
        let index = line.checked_sub(1)?;
        let content = if let Some(content) = self.source_inputs.get(path) {
            content
        } else {
            let mut matches = self
                .source_inputs
                .iter()
                .filter(|(candidate, _)| candidate.ends_with(path))
                .map(|(_, content)| content);
            let only = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            only
        };
        let aplan_core::load::PlanFileContent::Bytes(bytes) = content else {
            return None;
        };
        let text = std::str::from_utf8(bytes).ok()?;
        text.lines().nth(index).map(str::to_string)
    }

    fn identity_for(&self, loaded: &aplan_core::IdentifiedPlan) -> InputIdentity {
        let mut identity = self.identity.clone();
        identity.repository = repository_identity(&identity.repository_root, identity.trust);
        if self.report_config.is_some()
            && let Some(path) = identity.configuration.path.clone()
        {
            identity.configuration = match std::fs::read(&path) {
                Ok(bytes) => match aplan_config::parse_config_bytes(&bytes, &path) {
                    Ok(current) => report_configuration_identity(
                        Some(&path),
                        &self.options,
                        &self.verdict,
                        &current,
                        ConfigurationContentIdentity::Observed {
                            sha256: to_hex(&Sha256::digest(&bytes)),
                        },
                    ),
                    Err(error) => {
                        let reason = error.to_string();
                        unavailable_configuration_identity(
                            &path,
                            &reason,
                            ConfigurationContentIdentity::Unavailable {
                                reason: reason.clone(),
                            },
                        )
                    }
                },
                Err(error) => {
                    let reason = error.to_string();
                    unavailable_configuration_identity(
                        &path,
                        &reason,
                        ConfigurationContentIdentity::Unavailable {
                            reason: reason.clone(),
                        },
                    )
                }
            };
        }
        let plan_sha256 = plan_input_digest(&loaded.inputs, &identity.plan_root);
        let base_sha256 = input_digest(
            &plan_sha256,
            &identity.repository_root,
            &identity.repository,
            &identity.configuration,
            identity.trust,
            &self.options,
            &self.verdict,
        );
        identity.plan_sha256 = plan_sha256;
        identity.sha256 = self.auxiliary_identity(&base_sha256);
        identity
    }
}

/// The concrete request for structural validation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckRequest;

/// Structural validation as a typed application result.
#[derive(Debug, Clone)]
pub struct CheckResponse {
    pub input: InputIdentity,
    /// Ordered by the core validator's total diagnostic order.
    pub report: Report,
    /// The full repository verdict, not an error-count shortcut.
    pub denied: bool,
    /// Citations this hermetic operation deliberately did not open.
    pub evidence_unresolved: aplan_core::Unresolved,
}

/// Run the hermetic validation use case over one identified snapshot.
#[must_use]
pub fn check(context: &PlanContext, _request: CheckRequest) -> CheckResponse {
    let report = context
        .structural_report_with(|| aplan_core::validate(context.plan(), context.options()))
        .clone();
    CheckResponse {
        input: context.identity().clone(),
        denied: report.denied_by(context.verdict()),
        evidence_unresolved: aplan_core::unresolved_by_check(context.plan()),
        report,
    }
}

/// One-run warning-policy choice for an interactive structural check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WarningPolicyOverride {
    #[default]
    Inherit,
    Deny,
    Allow,
}

/// Explicit one-run changes to the structural check inputs.
///
/// This value travels with an effect request. It is never stored back into the
/// repository or silently reused by a later ordinary refresh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckOverrides {
    pub no_require_effort: bool,
    pub no_traces: bool,
    pub traces_root: Option<PathBuf>,
    pub warning_policy: WarningPolicyOverride,
}

/// The scope that produced a diagnostic result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticScope {
    Check,
    CheckDocs,
    Verify,
}

impl DiagnosticScope {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::CheckDocs => "check-docs",
            Self::Verify => "verify",
        }
    }
}

/// How one finding contributes to its complete run's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictContribution {
    Blocking,
    Advisory,
    NonBlocking,
}

impl VerdictContribution {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Blocking => "blocks this run",
            Self::Advisory => "advisory; does not block",
            Self::NonBlocking => "does not block under this policy",
        }
    }
}

/// Whether a diagnostic run had subjects to check and loaded them completely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticCoverage {
    Complete { subjects: usize },
    NoSubjects(String),
    Partial { subjects: usize, unavailable: usize },
    Unavailable(String),
}

/// One structural-check policy projected for interactive inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckPolicySummary {
    pub require_effort: bool,
    pub traces_root: Option<PathBuf>,
    pub traces_repository: Option<PathBuf>,
    pub traces_external: Vec<PathBuf>,
    pub detect_drift: bool,
    pub deny_warnings: bool,
    pub advisory: Vec<String>,
}

/// The inherited and effective policy plus one-run deviations behind a
/// structural result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRunSheet {
    /// Repository policy to which the next ordinary refresh returns.
    pub repository: CheckPolicySummary,
    /// Effective policy used for this retained result.
    pub effective: CheckPolicySummary,
    pub overrides: CheckOverrides,
    pub relaxed: bool,
}

/// One complete diagnostic scope. Filtering never changes these counts or the
/// verdict; it only changes which entries a view presents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRun {
    pub scope: DiagnosticScope,
    pub coverage: DiagnosticCoverage,
    pub findings: Vec<OverviewFinding>,
    pub errors: usize,
    pub warnings: usize,
    pub blocking: usize,
    pub advisory: usize,
    pub denied: bool,
    pub input_label: String,
}

/// The complete Diagnostics destination projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticWorkbench {
    pub check: DiagnosticRun,
    pub documents: DiagnosticRun,
    pub run_sheet: CheckRunSheet,
}

/// Whether an optional verification capability can be selected for this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable,
}

impl Availability {
    #[must_use]
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Configured providers shown before an interactive verification run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationProviders {
    pub test_runners: usize,
    pub url_check: Availability,
    pub version_current: Availability,
    pub version_check: Availability,
}

/// Inputs and trust boundary shown before an interactive verification run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationRunSheet {
    /// Identity of plan, repository and resolver configuration. The execute
    /// choice travels separately with the effect request.
    pub input_sha256: String,
    pub repository: PathBuf,
    pub configuration: Option<PathBuf>,
    pub policy: CheckPolicySummary,
    /// The broadest capability the configured verification providers may use.
    pub trust: Trust,
    pub providers: VerificationProviders,
    pub execute: Availability,
    /// Typed repairs can be staged through the guarded mutation service. The
    /// read-only workbench does not advertise an apply action until mutation
    /// composition wires that service into the shell.
    pub fix: Availability,
}

/// One evidence citation as inspected by the resolver that actually ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationCitation {
    pub task: String,
    pub kind: String,
    pub reference: String,
    pub resolver: String,
    pub strength: CitationStrength,
    pub outcome: CitationOutcome,
    pub detail: Option<String>,
    pub cost_subject: Option<String>,
    /// Whether the resolver answer describes the inputs identified by this run.
    pub observation_state: EvidenceObservationState,
}

/// Provider-level resource cost. Commands may be memoised across citations,
/// so assigning this total to each citation would overstate the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationProviderCost {
    pub subject: String,
    pub cost: VerificationCost,
}

/// One exact repair proposed by a verification diagnostic.
///
/// The interactive surface exposes these operands to the shared guarded repair
/// preview. Shell composition keeps inspection and explicit apply confirmation
/// as separate actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationRepair {
    pub task: Option<String>,
    pub from_kind: String,
    pub from_reference: String,
    pub to_kind: String,
    pub to_reference: String,
}

/// Whether the verifier ran to completion or refused an incomplete input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationState {
    Completed,
    Refused,
}

/// A failure before a verification answer can be produced.
///
/// Evidence findings remain inside [`VerificationWorkbench`]. This type keeps
/// repository/configuration availability distinct from a configured provider
/// failing to start, so an interactive adapter need not infer meaning from an
/// error sentence.
#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    #[error("repository verification failed: {0}")]
    Repository(String),
    #[error("configured verification provider failed: {0}")]
    Provider(String),
    #[error("verification is unavailable: {0}")]
    Unavailable(String),
    #[error(
        "verification input changed after review (expected {expected}, loaded {actual}); review the refreshed run sheet before running it"
    )]
    Stale { expected: String, actual: String },
}

impl From<aplan_evidence::Error> for VerificationError {
    fn from(error: aplan_evidence::Error) -> Self {
        if matches!(&error, aplan_evidence::Error::Exec(_)) {
            Self::Provider(error.to_string())
        } else {
            Self::Repository(error.to_string())
        }
    }
}

/// One complete interactive verification answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationWorkbench {
    pub input: InputIdentity,
    pub sheet: VerificationRunSheet,
    pub execute: bool,
    pub state: VerificationState,
    /// Aggregate freshness and provenance for the evidence answer.
    pub observation: EvidenceObservation,
    pub run: DiagnosticRun,
    pub citations: Vec<VerificationCitation>,
    pub costs: Vec<VerificationProviderCost>,
    pub repairs: Vec<VerificationRepair>,
}

/// Project the verification inputs a user must review before permitting work.
#[must_use]
pub fn verification_run_sheet(
    context: &PlanContext,
    options: &aplan_evidence::VerifyOptions,
) -> VerificationRunSheet {
    let mut identified_options = options.clone();
    identified_options.execute = false;
    let mut digest = Sha256::new();
    digest.update(context.identity().sha256.as_bytes());
    digest.update([0]);
    digest.update(format!("{identified_options:?}").as_bytes());
    let version_current = options
        .version
        .as_ref()
        .is_some_and(|policy| policy.current.is_some());
    let version_check = options
        .version
        .as_ref()
        .is_some_and(|policy| policy.check.is_some());
    let configured_processes = options.test_runner.is_some()
        || options.url_check.is_some()
        || version_current
        || version_check;
    let mut policy = check_policy_summary(context.options(), context.verdict());
    policy.detect_drift = options.detect_drift;
    VerificationRunSheet {
        input_sha256: to_hex(&digest.finalize()),
        repository: context.identity().repository_root.clone(),
        configuration: context.identity().configuration.path.clone(),
        policy,
        trust: if configured_processes {
            Trust::ConfiguredProcesses
        } else {
            Trust::RepositoryRead
        },
        providers: VerificationProviders {
            test_runners: options
                .test_runner
                .as_ref()
                .map_or(0, |runners| runners.all().len()),
            url_check: availability(options.url_check.is_some()),
            version_current: availability(version_current),
            version_check: availability(version_check),
        },
        execute: availability(options.test_runner.is_some()),
        fix: Availability::Unavailable,
    }
}

const fn availability(available: bool) -> Availability {
    if available {
        Availability::Available
    } else {
        Availability::Unavailable
    }
}

/// Run structural validation and evidence resolution over one identified
/// application snapshot.
///
/// This is the typed use case shared by the interactive adapter. It calls the
/// evidence engine directly and never parses CLI output. A partially loaded
/// plan is refused before repository or configured-process work, matching the
/// command gate's completeness boundary.
pub fn verify_context(
    context: &PlanContext,
    options: &aplan_evidence::VerifyOptions,
) -> Result<VerificationWorkbench, VerificationError> {
    verify_context_inner(context, options, None)
}

/// Run interactive verification while reporting typed operation boundaries.
pub fn verify_context_with_progress(
    context: &PlanContext,
    options: &aplan_evidence::VerifyOptions,
    operation: OperationId,
    progress: &mut dyn FnMut(ProgressEvent),
) -> Result<VerificationWorkbench, VerificationError> {
    let mut emitter = ProgressEmitter::begin(operation, OperationKind::Verification, progress);
    let result = verify_context_inner(context, options, Some(&mut emitter));
    if result.is_ok() {
        emitter.complete();
    } else {
        emitter.fail();
    }
    result
}

struct VerificationEvidence {
    state: VerificationState,
    observation: EvidenceObservation,
    citations: Vec<VerificationCitation>,
    costs: Vec<VerificationProviderCost>,
}

fn observe_verification_evidence(
    context: &PlanContext,
    options: &aplan_evidence::VerifyOptions,
    report: &mut Report,
    unloadable: usize,
    mut progress: Option<&mut ProgressEmitter<'_>>,
) -> Result<VerificationEvidence, VerificationError> {
    if unloadable != 0 {
        return Ok(VerificationEvidence {
            state: VerificationState::Refused,
            observation: EvidenceObservation::not_observed(
                "verification refused an incompletely loaded plan",
            ),
            citations: Vec::new(),
            costs: Vec::new(),
        });
    }
    if let Some(emitter) = progress.as_deref_mut() {
        emitter.phase_started(
            ProgressPhase::RepositoryObservation,
            None,
            ProgressMeasure::Indeterminate,
        );
    }
    let mut provider_progress = CatalogueProgressAdapter::default();
    let observed = inspect_evidence(
        context.plan(),
        &context.identity().repository_root,
        options,
        EvidenceInspectionMode::Refresh,
        &EvidenceObservationOptions::default(),
        |event| {
            if let Some(emitter) = progress.as_deref_mut() {
                provider_progress.accept(emitter, &event);
            }
        },
    );
    let observed = match observed {
        Ok(observed) => {
            if let Some(emitter) = progress {
                emitter.phase_completed(
                    ProgressPhase::RepositoryObservation,
                    None,
                    ProgressMeasure::Indeterminate,
                );
            }
            observed
        }
        Err(error) => {
            if let Some(emitter) = progress {
                emitter.phase_failed(
                    ProgressPhase::RepositoryObservation,
                    None,
                    ProgressMeasure::Indeterminate,
                );
            }
            return Err(VerificationError::from(error));
        }
    };
    let mut observation = observed.observation.clone();
    observation.scope = EvidenceObservationScope::WholePlan;
    report.diagnostics.extend(observed.report.diagnostics);
    Ok(VerificationEvidence {
        state: VerificationState::Completed,
        observation,
        citations: verification_citations(observed.citations, observed.citation_states),
        costs: observed
            .spend
            .into_iter()
            .map(|(subject, cost)| VerificationProviderCost { subject, cost })
            .collect(),
    })
}

fn verify_context_inner(
    context: &PlanContext,
    options: &aplan_evidence::VerifyOptions,
    mut progress: Option<&mut ProgressEmitter<'_>>,
) -> Result<VerificationWorkbench, VerificationError> {
    let sheet = verification_run_sheet(context, options);
    let mut input = context.identity().clone();
    input.trust = sheet.trust;
    input.sha256.clone_from(&sheet.input_sha256);
    if let Some(emitter) = progress.as_deref_mut() {
        emitter.phase_started(
            ProgressPhase::StructuralValidation,
            None,
            ProgressMeasure::Indeterminate,
        );
    }
    let mut report = context
        .structural_report_with(|| aplan_core::validate(context.plan(), context.options()))
        .clone();
    if let Some(emitter) = progress.as_deref_mut() {
        emitter.phase_completed(
            ProgressPhase::StructuralValidation,
            None,
            ProgressMeasure::Indeterminate,
        );
    }
    let unloadable = context.plan().load_diagnostics.len();
    let evidence =
        observe_verification_evidence(context, options, &mut report, unloadable, progress)?;
    let VerificationEvidence {
        state,
        observation,
        citations,
        costs,
    } = evidence;
    report.diagnostics.sort_by(|left, right| {
        left.code
            .cmp(right.code)
            .then(left.task.cmp(&right.task))
            .then(left.file.cmp(&right.file))
            .then(left.message.cmp(&right.message))
    });
    let repairs = report
        .diagnostics
        .iter()
        .filter_map(|finding| {
            let aplan_core::Repair::ReplaceEvidence { from, to } = finding.repair.as_ref()?;
            Some(VerificationRepair {
                task: finding.task.clone(),
                from_kind: from.kind.as_str().to_string(),
                from_reference: from.reference.clone(),
                to_kind: to.kind.as_str().to_string(),
                to_reference: to.reference.clone(),
            })
        })
        .collect();
    let findings = report
        .diagnostics
        .iter()
        .map(|finding| core_finding(context, finding, context.verdict(), DiagnosticScope::Verify))
        .collect::<Vec<_>>();
    let coverage = if state == VerificationState::Refused {
        DiagnosticCoverage::Unavailable(format!(
            "{unloadable} task file(s) failed to load; evidence was not resolved"
        ))
    } else if report.tasks_checked == 0 {
        DiagnosticCoverage::NoSubjects("the plan contains no task subjects".to_string())
    } else {
        DiagnosticCoverage::Complete {
            subjects: report.tasks_checked,
        }
    };
    let denied = state == VerificationState::Refused || report.denied_by(context.verdict());
    Ok(VerificationWorkbench {
        input,
        sheet,
        execute: options.execute,
        state,
        observation,
        run: diagnostic_run(
            DiagnosticScope::Verify,
            coverage,
            findings,
            denied,
            format!(
                "repository {}",
                context.identity().repository_root.display()
            ),
        ),
        citations,
        costs,
        repairs,
    })
}

fn verification_citations(
    citations: Vec<aplan_evidence::CitationObservation>,
    states: Vec<EvidenceObservationState>,
) -> Vec<VerificationCitation> {
    citations
        .into_iter()
        .zip(states)
        .map(|(citation, observation_state)| VerificationCitation {
            task: citation.task,
            kind: citation.kind.as_str().to_string(),
            reference: citation.reference,
            resolver: citation.resolution.resolver,
            strength: citation.resolution.strength,
            outcome: citation.resolution.outcome,
            detail: citation.resolution.detail,
            cost_subject: citation.resolution.cost_subject,
            observation_state,
        })
        .collect()
}

/// Apply one identity-checked interactive verification answer to a task view.
pub fn apply_verification_to_show(show: &mut Show, verification: &VerificationWorkbench) {
    let mut seen = BTreeMap::<(String, String), usize>::new();
    for cited in &mut show.evidence {
        let key = (cited.kind.clone(), cited.reference.clone());
        let occurrence = seen.entry(key.clone()).or_default();
        let observed = verification
            .citations
            .iter()
            .filter(|citation| {
                citation.task == show.id && citation.kind == key.0 && citation.reference == key.1
            })
            .nth(*occurrence);
        *occurrence += 1;
        if let Some(observed) = observed {
            cited.observation = EvidenceObservation {
                state: observed.observation_state,
                scope: verification.observation.scope,
                provider_source: verification.observation.provider_source,
                completeness: verification.observation.completeness,
                identity: (cited.kind == aplan_core::EvidenceKind::Test.as_str())
                    .then(|| verification.observation.identity.clone())
                    .flatten(),
                resolution_identity: verification.observation.resolution_identity.clone(),
                observed_at_epoch_seconds: verification.observation.observed_at_epoch_seconds,
                resolver: Some(observed.resolver.clone()),
                strength: Some(observed.strength.label().to_string()),
                outcome: Some(observed.outcome.label().to_string()),
                detail: observed.detail.clone(),
                refresh: verification.observation.refresh.clone(),
            };
        }
    }
    let states = show
        .evidence
        .iter()
        .map(|citation| citation.observation.state)
        .collect::<Vec<_>>();
    show.evidence_observation = verification.observation.clone();
    let task_state = evidence_observation::aggregate_state(&states);
    if task_state != show.evidence_observation.state {
        show.evidence_observation.detail =
            Some("derived from this task's citations in the complete verification run".to_string());
    }
    show.evidence_observation.state = task_state;
    show.resolved = show.evidence_observation.state == EvidenceObservationState::Current;
}

/// Cancel every repository-configured command this process currently owns.
///
/// This is the shared process-termination boundary. The non-interactive CLI
/// calls it before re-raising the original signal; the TUI calls it only after
/// restoring terminal state.
pub fn terminate_verification_commands() {
    aplan_evidence::terminate_active_configured_commands();
}

/// A typed rule explanation projected from the same registry as the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleExplanation {
    pub code: String,
    pub severity: Severity,
    pub title: String,
}

/// Exact refusal for a name outside the rule catalogue.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown rule `{name}`; known rule codes: {known}")]
pub struct UnknownRule {
    pub name: String,
    pub known: String,
}

/// Explain an exact rule code through the core registry used by `aplan
/// explain rule`; unfamiliar and normalized spellings are refused.
pub fn explain_rule(code: &str) -> Result<RuleExplanation, UnknownRule> {
    aplan_core::explain_rule(code)
        .map(|explanation| RuleExplanation {
            code: explanation.code.to_string(),
            severity: explanation.severity,
            title: explanation.title.to_string(),
        })
        .ok_or_else(|| UnknownRule {
            name: code.to_string(),
            known: aplan_core::Rule::ALL
                .iter()
                .map(|rule| rule.code())
                .collect::<Vec<_>>()
                .join(", "),
        })
}

/// A stable ordering offered by the task browser.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskSort {
    /// Ready work in the exact order of `aplan ready`, followed by all other
    /// tasks in id order.
    #[default]
    ReadyRank,
    Id,
    Title,
    Status,
    Cut,
    Phase,
    Owner,
}

impl TaskSort {
    pub const ALL: [Self; 7] = [
        Self::ReadyRank,
        Self::Id,
        Self::Title,
        Self::Status,
        Self::Cut,
        Self::Phase,
        Self::Owner,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadyRank => "ready rank",
            Self::Id => "id",
            Self::Title => "title",
            Self::Status => "status",
            Self::Cut => "cut",
            Self::Phase => "phase",
            Self::Owner => "owner",
        }
    }

    #[must_use]
    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .expect("every sort is in TaskSort::ALL");
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

/// A byte range in a task field. Both ends are UTF-8 boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
}

/// One task projected for discovery without creating a second plan model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBrowserRow {
    pub id: String,
    pub title: String,
    pub phase: u32,
    pub status: Status,
    pub owner: Option<String>,
    pub claim: Option<aplan_core::ClaimRecord>,
    pub coordination: aplan_core::ClaimScope,
    pub claim_observed_at_epoch_seconds: i64,
    pub tags: Vec<String>,
    pub cut: Option<String>,
    pub milestone: Option<String>,
    pub ready_rank: Option<usize>,
    pub rank_reason: Option<String>,
    /// Exact body text from the identified plan input. It is searched but not
    /// presented in the compact row.
    pub body: String,
}

/// A row that survived both structured selection and free-text search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBrowserMatch {
    pub task: TaskBrowserRow,
    pub id_matches: Vec<MatchSpan>,
    pub title_matches: Vec<MatchSpan>,
    pub body_matches: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskBrowserRequest {
    pub query: String,
    pub selector: Selector,
    pub sort: TaskSort,
}

/// Why a task browser has no visible rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoTaskResults {
    EmptyPlan,
    EmptyScope,
    CoordinationUnavailable,
    NoFilterMatches,
    NoSearchMatches,
}

impl NoTaskResults {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::EmptyPlan => "The plan contains no tasks.",
            Self::EmptyScope => "This aggregate contains no tasks.",
            Self::CoordinationUnavailable => {
                "Unclaimed work cannot be selected because local claim coordination is unavailable."
            }
            Self::NoFilterMatches => "No tasks match the active filters.",
            Self::NoSearchMatches => "No filtered tasks match the search text.",
        }
    }
}

/// A complete deterministic browser answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBrowserResponse {
    pub rows: Vec<TaskBrowserMatch>,
    pub total: usize,
    pub selector_matches: usize,
    pub active_filters: Vec<String>,
    pub sort: TaskSort,
    pub no_results: Option<NoTaskResults>,
}

/// A graph scope offered by interactive clients.
///
/// These are named operations rather than a query language. Reachability is
/// delegated to [`Reach`], while neighbourhoods use the same walk as
/// `aplan tree`; delivery adapters never recover either traversal from edges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum GraphScope {
    #[default]
    Whole,
    Upstream(String),
    Downstream(String),
    Between(String, String),
    Unreachable,
    Neighbourhood {
        task: String,
        depth: Option<usize>,
        live: bool,
    },
    CriticalPath,
}

impl GraphScope {
    /// The relation class this structural scope actually traverses.
    ///
    /// `unreachable` is a containment question even in a dependency-only
    /// presentation, and neighbourhoods use the complete `blocked_by` graph.
    /// Keeping that exception here prevents delivery adapters from claiming
    /// they walked the relation they merely paint.
    #[must_use]
    pub const fn walked_edges(&self, membership: bool) -> Option<Edges> {
        let asked = if membership {
            Edges::Any
        } else {
            Edges::Dependency
        };
        match self {
            Self::Whole | Self::CriticalPath => None,
            Self::Unreachable => Some(Edges::Membership),
            Self::Neighbourhood { .. } => Some(Edges::Any),
            Self::Upstream(_) | Self::Downstream(_) | Self::Between(_, _) => Some(asked),
        }
    }

    /// Human wording carried into every graph renderer's scope notes.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Whole => "whole plan".to_string(),
            Self::Upstream(id) => format!("upstream of `{id}`"),
            Self::Downstream(id) => format!("downstream of `{id}`"),
            Self::Between(from, to) => format!("between `{from}` and `{to}`"),
            Self::Unreachable => "unreachable from every milestone".to_string(),
            Self::Neighbourhood { task, depth, live } => format!(
                "neighbourhood of `{task}`{}{}",
                depth.map_or_else(
                    || " at every depth".to_string(),
                    |value| format!(" at depth {value}")
                ),
                if *live { ", live work only" } else { "" }
            ),
            Self::CriticalPath => "critical path".to_string(),
        }
    }
}

/// A deterministic interactive graph request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphRequest {
    pub selector: Selector,
    pub scope: GraphScope,
    /// Include cut membership as a visually distinct relationship.
    pub membership: bool,
}

/// Whether a graph projection is drawable and complete for its request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "task", rename_all = "snake_case")]
pub enum GraphProjectionState {
    Complete,
    EmptyScope,
    NoEdges,
    UnknownTask(String),
    CyclicCriticalPath,
}

/// One direct dependency of the selected graph node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRelation {
    pub id: String,
    pub kind: EdgeKind,
}

/// A typed, renderer-independent graph answer.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphProjection {
    pub input: InputIdentity,
    pub view: GraphView,
    pub state: GraphProjectionState,
    /// Identities admitted before dependency-only grouping omissions.
    pub admitted: BTreeSet<String>,
    pub critical_path: Vec<String>,
    /// Why a known task is absent. Missing ids are deliberately not invented.
    pub exclusions: BTreeMap<String, String>,
}

impl GraphProjection {
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.view.nodes.iter().any(|node| node.id == id)
    }

    #[must_use]
    pub fn exclusion(&self, id: &str) -> Option<&str> {
        self.exclusions.get(id).map(String::as_str)
    }

    /// Project an explicitly bounded window from this already-resolved graph.
    ///
    /// Interactive clients commonly retain the complete semantic projection
    /// for relationship queries while changing only its presentation window.
    /// This avoids resolving selectors and structural scope a second time for
    /// every continuation, page or history transition.
    #[must_use]
    pub fn window(&self, request: &GraphWindowRequest) -> GraphWindowProjection {
        graph_window::project_window(self, request)
    }

    /// Direct prerequisites present in this projection.
    #[must_use]
    pub fn blockers(&self, id: &str) -> Vec<GraphRelation> {
        self.view
            .edges
            .iter()
            .filter(|edge| edge.to == id && edge.kind == EdgeKind::Dependency)
            .map(|edge| GraphRelation {
                id: edge.from.clone(),
                kind: edge.kind,
            })
            .collect()
    }

    /// Direct dependents present in this projection.
    #[must_use]
    pub fn dependents(&self, id: &str) -> Vec<GraphRelation> {
        self.view
            .edges
            .iter()
            .filter(|edge| edge.from == id && edge.kind == EdgeKind::Dependency)
            .map(|edge| GraphRelation {
                id: edge.to.clone(),
                kind: edge.kind,
            })
            .collect()
    }
}

/// The immutable graph facts retained by an interactive snapshot.
///
/// `tasks` are clones of the persisted domain values, not a second graph
/// model. A [`Graph`] is reconstructed only to invoke core reachability and
/// critical-path semantics; visible nodes and edges always come from `whole`.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphWorkspace {
    input: InputIdentity,
    tasks: Vec<Task>,
    catalogue: Vec<TaskBrowserRow>,
    whole: GraphView,
    critical_path: Option<Vec<String>>,
}

impl GraphWorkspace {
    /// Build a workspace from already identified domain inputs.
    #[must_use]
    pub fn new(input: InputIdentity, tasks: Vec<Task>, catalogue: Vec<TaskBrowserRow>) -> Self {
        let graph = Graph::new(tasks.iter());
        let all: BTreeSet<&str> = graph.by_id.keys().copied().collect();
        let whole = GraphView::of(&graph, &all);
        let critical_path = graph
            .critical_path()
            .map(|path| path.tasks.into_iter().map(|task| task.id).collect());
        Self {
            input,
            tasks,
            catalogue,
            whole,
            critical_path,
        }
    }

    /// Project selectors and one structural scope over the same canonical view.
    #[must_use]
    pub fn project(&self, request: &GraphRequest) -> GraphProjection {
        let graph = Graph::new(self.tasks.iter());
        let all: BTreeSet<String> = graph.by_id.keys().map(|id| (*id).to_string()).collect();
        let selected: BTreeSet<String> = self
            .catalogue
            .iter()
            .filter(|row| {
                request.selector.matches_fields_claimed(
                    row.phase,
                    row.status,
                    row.owner.as_deref(),
                    &row.tags,
                    row.cut.as_deref(),
                    row.milestone.as_deref(),
                    row.coordination.available.then(|| row.claim.is_some()),
                )
            })
            .map(|row| row.id.clone())
            .collect();
        let edges = request
            .scope
            .walked_edges(request.membership)
            .unwrap_or_else(|| requested_graph_edges(request.membership));
        let (scope_ids, exceptional) = match &request.scope {
            GraphScope::Whole => (Some(all.clone()), None),
            GraphScope::Upstream(id) => reach_ids(&graph, &Reach::Upstream(id.clone()), edges),
            GraphScope::Downstream(id) => reach_ids(&graph, &Reach::Downstream(id.clone()), edges),
            GraphScope::Between(from, to) => {
                reach_ids(&graph, &Reach::Between(from.clone(), to.clone()), edges)
            }
            GraphScope::Unreachable => reach_ids(&graph, &Reach::Unreachable, edges),
            GraphScope::Neighbourhood { task, depth, live } => {
                match aplan_output::tree_ids(&graph, task, *depth, *live) {
                    Some(ids) => (Some(ids), None),
                    None => (None, Some(GraphProjectionState::UnknownTask(task.clone()))),
                }
            }
            GraphScope::CriticalPath => match &self.critical_path {
                Some(ids) => (Some(ids.iter().cloned().collect()), None),
                None => (None, Some(GraphProjectionState::CyclicCriticalPath)),
            },
        };
        let admitted: BTreeSet<String> = scope_ids
            .unwrap_or_default()
            .intersection(&selected)
            .cloned()
            .collect();
        let refs: BTreeSet<&str> = admitted.iter().map(String::as_str).collect();
        let mut view = self.whole.scoped(&refs, request.membership);
        view.scope = request.selector.describe();
        if request.scope != GraphScope::Whole {
            view.scope.push(format!(
                "{} over {}",
                request.scope.label(),
                graph_scope_edge_description(&request.scope, request.membership)
            ));
        }
        let shown: BTreeSet<&str> = view.nodes.iter().map(|node| node.id.as_str()).collect();
        let mut exclusions = BTreeMap::new();
        for id in &all {
            if !selected.contains(id) {
                exclusions.insert(
                    id.clone(),
                    "excluded by the active task filters".to_string(),
                );
            } else if !admitted.contains(id) {
                exclusions.insert(
                    id.clone(),
                    format!("outside the {} scope", request.scope.label()),
                );
            } else if !shown.contains(id.as_str()) {
                exclusions.insert(
                    id.clone(),
                    "has only cut-membership relationships in this view; enable membership"
                        .to_string(),
                );
            }
        }
        let state = exceptional.unwrap_or({
            if view.nodes.is_empty() {
                GraphProjectionState::EmptyScope
            } else if view.edges.is_empty() {
                GraphProjectionState::NoEdges
            } else {
                GraphProjectionState::Complete
            }
        });
        GraphProjection {
            input: self.input.clone(),
            view,
            state,
            admitted,
            critical_path: self.critical_path.clone().unwrap_or_default(),
            exclusions,
        }
    }

    /// Project one explicitly bounded, fully accounted semantic graph window.
    ///
    /// Selector, scope and membership semantics are resolved by [`Self::project`]
    /// first. The window layer can therefore reduce presentation cost without
    /// changing what the caller asked the graph to mean.
    #[must_use]
    pub fn project_window(
        &self,
        request: &GraphRequest,
        window: &GraphWindowRequest,
    ) -> GraphWindowProjection {
        self.project(request).window(window)
    }
}

const fn requested_graph_edges(membership: bool) -> Edges {
    if membership {
        Edges::Any
    } else {
        Edges::Dependency
    }
}

fn graph_scope_edge_description(scope: &GraphScope, membership: bool) -> &'static str {
    if matches!(scope, GraphScope::CriticalPath) {
        "critical-path dependencies and containment"
    } else {
        scope
            .walked_edges(membership)
            .unwrap_or_else(|| requested_graph_edges(membership))
            .describe()
    }
}

fn reach_ids(
    graph: &Graph<'_>,
    reach: &Reach,
    edges: Edges,
) -> (Option<BTreeSet<String>>, Option<GraphProjectionState>) {
    if let Some(ids) = reach.select(graph, edges) {
        (Some(ids.into_iter().map(str::to_string).collect()), None)
    } else {
        let unknown = reach
            .ids()
            .into_iter()
            .find(|id| graph.get(id).is_none())
            .map(|id| GraphProjectionState::UnknownTask(id.to_string()));
        (None, unknown)
    }
}

/// Build the interactive graph facts from the same identified plan snapshot as
/// task, diagnostic and overview projections.
#[must_use]
pub fn graph_workspace(context: &PlanContext, catalogue: Vec<TaskBrowserRow>) -> GraphWorkspace {
    GraphWorkspace::new(
        context.identity().clone(),
        context
            .plan()
            .tasks
            .iter()
            .map(|loaded| loaded.task.clone())
            .collect(),
        catalogue,
    )
}

/// One immutable read model for the Tasks destination.
///
/// Validation, compact rows and complete inspectors are derived together so a
/// caller cannot accidentally pair diagnostics from one input identity with
/// task bodies or metadata from another.
#[derive(Debug)]
pub struct TaskView {
    pub check: CheckResponse,
    pub catalogue: Vec<TaskBrowserRow>,
    pub inspectors: Vec<Show>,
    pub overview: PlanOverview,
    pub diagnostics: DiagnosticWorkbench,
}

/// The running executable's identity, supplied by the delivery host.
///
/// The application crate cannot inspect the binary that embeds it. Carrying
/// the host's already-computed identity keeps the Overview from substituting
/// the package version for a build identity; development builds routinely
/// share a version while differing in capability and bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildIdentity {
    pub version: String,
    pub source: String,
    pub facts_sha256: String,
    pub binary_sha256: String,
}

/// Whether all plan candidates loaded, and whether any usable work survived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewLoadState {
    Complete,
    Empty,
    Malformed,
    PartiallyUnloadable,
}

/// The state of non-milestone work, separate from load health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewWorkState {
    Active,
    AllDone,
    OnlyDropped,
    NoLoadedWork,
}

/// Whether the live-work effort aggregate is complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewEffortState {
    Complete,
    Partial,
    None,
    NoLiveWork,
}

/// One task named by an Overview aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewTask {
    pub id: String,
    pub title: String,
}

/// One due parked task at the Overview's explicit clock instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewDueTask {
    pub id: String,
    pub title: String,
    pub revisit_at: String,
    pub overdue_seconds: i64,
}

/// One parked task whose revisit instant has not arrived yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewWaitingTask {
    pub id: String,
    pub title: String,
    pub revisit_at: String,
    pub seconds_until: i64,
}

/// One cut rollup. `task_ids` is the exact set behind this row and is retained
/// so an interactive surface can drill into the aggregate without recreating
/// membership rules.
#[derive(Debug, Clone, PartialEq)]
pub struct OverviewCut {
    pub label: String,
    pub milestone_id: String,
    pub status: Status,
    pub borrows: bool,
    pub total: usize,
    pub done: usize,
    pub ready: usize,
    pub blocked: usize,
    pub effort_days: f64,
    pub done_effort_days: f64,
    pub task_ids: Vec<String>,
}

/// A diagnostic retained whole enough for an exact Overview drill-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewFinding {
    /// Stable across filtering, grouping and refresh while the finding's
    /// semantic identity is unchanged.
    pub identity: String,
    pub scope: DiagnosticScope,
    pub code: String,
    pub severity: Severity,
    /// The exact entity judged by the diagnostic. This is deliberately
    /// separate from `task`: document diagnostics can judge a reference,
    /// milestone or input without offering task navigation.
    pub subject: String,
    pub task: Option<String>,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
    pub agent_instruction: Option<String>,
    pub detail: Vec<String>,
    pub contribution: VerdictContribution,
    /// Exact inert source line from the identified input, where one exists.
    pub source_line: Option<String>,
}

/// All plan-health and programme-progress values rendered by the Overview.
///
/// Counts, task identities and source state travel together. A renderer never
/// re-walks `Plan` or `Graph`, which makes both textual values and drill-down
/// membership projections of the same immutable answer.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanOverview {
    pub input: InputIdentity,
    pub load_state: OverviewLoadState,
    pub work_state: OverviewWorkState,
    pub effort_state: OverviewEffortState,
    pub loaded_tasks: usize,
    pub loaded_work: usize,
    pub live_work: usize,
    pub done_work: usize,
    pub dropped_work: usize,
    pub live_task_ids: Vec<String>,
    pub live_effort_days: f64,
    pub done_effort_days: f64,
    pub missing_effort_task_ids: Vec<String>,
    pub cuts: Vec<OverviewCut>,
    pub cut_task_ids: Vec<String>,
    pub cut_work: usize,
    pub done_cut_work: usize,
    pub ready: Vec<OverviewTask>,
    pub as_of_epoch_seconds: i64,
    pub due: Vec<OverviewDueTask>,
    pub waiting_revisits: Vec<OverviewWaitingTask>,
    /// `None` means the path is undefined because the graph is cyclic. An empty
    /// vector is a defined path over a plan with no contributing work.
    pub critical_path: Option<(f64, Vec<OverviewTask>)>,
    pub findings: Vec<OverviewFinding>,
    pub errors: usize,
    pub warnings: usize,
    pub denied: bool,
    pub unloadable: Vec<(String, PathBuf)>,
}

/// One selectable structured-filter value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskFilter {
    Status(Status),
    Cut(String),
    Phase(u32),
    Owner(String),
    Unclaimed,
    Tag(String),
}

impl TaskFilter {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Status(value) => format!("status: {value}"),
            Self::Cut(value) => format!("cut: {value}"),
            Self::Phase(value) => format!("phase: {value}"),
            Self::Owner(value) => format!("owner: {value}"),
            Self::Unclaimed => "unclaimed (no active claim)".to_string(),
            Self::Tag(value) => format!("tag: {value}"),
        }
    }

    #[must_use]
    pub fn selected(&self, selector: &Selector) -> bool {
        match self {
            Self::Status(value) => selector.statuses.contains(value),
            Self::Cut(value) => selector.cuts.contains(value),
            Self::Phase(value) => selector.phases.contains(value),
            Self::Owner(value) => selector.owners.contains(value),
            Self::Unclaimed => selector.unclaimed,
            Self::Tag(value) => selector.tags.contains(value),
        }
    }

    pub fn toggle(&self, selector: &mut Selector) {
        match self {
            Self::Status(value) => {
                toggle_value(&mut selector.statuses, *value);
                selector
                    .statuses
                    .sort_by_key(|status| status_order(*status));
            }
            Self::Cut(value) => {
                toggle_value(&mut selector.cuts, value.clone());
                selector.cuts.sort();
            }
            Self::Phase(value) => {
                toggle_value(&mut selector.phases, *value);
                selector.phases.sort_unstable();
            }
            Self::Owner(value) => {
                toggle_value(&mut selector.owners, value.clone());
                selector.owners.sort();
            }
            Self::Unclaimed => selector.unclaimed = !selector.unclaimed,
            Self::Tag(value) => {
                toggle_value(&mut selector.tags, value.clone());
                selector.tags.sort();
            }
        }
    }
}

fn toggle_value<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if let Some(index) = values.iter().position(|candidate| candidate == &value) {
        values.remove(index);
    } else {
        values.push(value);
    }
}

/// Project a loaded context into rows the interactive browser can keep.
#[must_use]
pub fn task_catalogue(context: &PlanContext) -> Vec<TaskBrowserRow> {
    let graph = Graph::new(context.plan().tasks.iter().map(|loaded| &loaded.task));
    let claims = claims::snapshot(&context.identity().repository_root, context.plan())
        .unwrap_or_else(|error| ClaimSnapshot::unavailable(epoch_now(), error.to_string()));
    task_catalogue_with_graph(context, &graph, &claims)
}

fn task_catalogue_with_graph(
    context: &PlanContext,
    graph: &Graph<'_>,
    claims: &ClaimSnapshot,
) -> Vec<TaskBrowserRow> {
    let reasons: std::collections::BTreeMap<&str, String> = graph
        .ready_ranked()
        .into_iter()
        .map(|(task, rank)| (task.id.as_str(), rank.because()))
        .collect();
    graph
        .task_rows()
        .tasks
        .into_iter()
        .map(|row| {
            let claim = claims.claim(&row.id).cloned();
            TaskBrowserRow {
                body: context.task_body(&row.id).unwrap_or_default().to_string(),
                rank_reason: reasons.get(row.id.as_str()).cloned(),
                id: row.id,
                title: row.title,
                phase: row.phase,
                status: row.status,
                owner: row.owner,
                claim,
                coordination: claims.scope.clone(),
                claim_observed_at_epoch_seconds: claims.observed_at_epoch_seconds,
                tags: row.tags,
                cut: row.cut,
                milestone: row.milestone,
                ready_rank: row.rank,
            }
        })
        .collect()
}

/// Values that can be applied to a task catalogue, in a stable group order.
#[must_use]
pub fn task_filter_choices(rows: &[TaskBrowserRow]) -> Vec<TaskFilter> {
    let mut choices = Vec::new();
    for row in rows {
        choices.push(TaskFilter::Status(row.status));
        if let Some(cut) = &row.cut {
            choices.push(TaskFilter::Cut(cut.clone()));
        }
        choices.push(TaskFilter::Phase(row.phase));
        if let Some(owner) = &row.owner {
            choices.push(TaskFilter::Owner(owner.clone()));
        }
        if row.coordination.available && row.claim.is_none() {
            choices.push(TaskFilter::Unclaimed);
        }
        for tag in &row.tags {
            choices.push(TaskFilter::Tag(tag.clone()));
        }
    }
    choices.sort_by_cached_key(filter_key);
    choices.dedup();
    choices
}

fn status_order(status: Status) -> usize {
    Status::ALL
        .iter()
        .position(|candidate| *candidate == status)
        .expect("every status is in Status::ALL")
}

fn filter_key(filter: &TaskFilter) -> (u8, u8, String) {
    match filter {
        TaskFilter::Status(value) => (0, 0, format!("{:02}", status_order(*value))),
        TaskFilter::Cut(value) => (1, 0, value.clone()),
        TaskFilter::Phase(value) => (2, 0, format!("{value:010}")),
        TaskFilter::Unclaimed => (3, 0, String::new()),
        TaskFilter::Owner(value) => (3, 1, value.clone()),
        TaskFilter::Tag(value) => (4, 0, value.clone()),
    }
}

/// Apply the shared structured selector, free-text matching and stable sort.
#[must_use]
pub fn browse_tasks(
    catalogue: &[TaskBrowserRow],
    request: &TaskBrowserRequest,
) -> TaskBrowserResponse {
    let selected: Vec<_> = catalogue
        .iter()
        .filter(|row| {
            request.selector.matches_fields_claimed(
                row.phase,
                row.status,
                row.owner.as_deref(),
                &row.tags,
                row.cut.as_deref(),
                row.milestone.as_deref(),
                row.coordination.available.then(|| row.claim.is_some()),
            )
        })
        .collect();
    let selector_matches = selected.len();
    let folded_query = request.query.to_lowercase();
    let mut rows: Vec<_> = selected
        .into_iter()
        .filter_map(|row| {
            let id_matches = match_spans(&row.id, &request.query);
            let title_matches = match_spans(&row.title, &request.query);
            let body_matches = matches_folded(&row.body, &folded_query);
            (request.query.is_empty()
                || !id_matches.is_empty()
                || !title_matches.is_empty()
                || body_matches)
                .then(|| TaskBrowserMatch {
                    task: row.clone(),
                    id_matches,
                    title_matches,
                    body_matches,
                })
        })
        .collect();
    sort_task_matches(&mut rows, request.sort);
    let no_results = if !rows.is_empty() {
        None
    } else if catalogue.is_empty() {
        Some(NoTaskResults::EmptyPlan)
    } else if request.selector.unclaimed && catalogue.iter().any(|row| !row.coordination.available)
    {
        Some(NoTaskResults::CoordinationUnavailable)
    } else if selector_matches == 0 {
        Some(NoTaskResults::NoFilterMatches)
    } else {
        Some(NoTaskResults::NoSearchMatches)
    };
    TaskBrowserResponse {
        rows,
        total: catalogue.len(),
        selector_matches,
        active_filters: request.selector.describe(),
        sort: request.sort,
        no_results,
    }
}

/// Build one identified task read model for an interactive snapshot.
///
/// This is deliberately the same [`aplan_core::Show`] the CLI and MCP
/// serialize. The interactive snapshot is hermetic, so `resolved` is false;
/// the evidence-running operation adds its results separately when requested.
#[must_use]
pub fn task_view(context: &PlanContext) -> TaskView {
    task_view_with_overrides(context, &CheckOverrides::default())
}

/// Build an immutable task and diagnostic snapshot under explicit one-run
/// structural-check overrides.
#[must_use]
pub fn task_view_with_overrides(context: &PlanContext, overrides: &CheckOverrides) -> TaskView {
    task_view_at_with_overrides(context, epoch_now(), overrides)
}

/// Build the same immutable task/overview read model at an explicit clock
/// instant. The ordinary interactive path uses the system clock; tests and
/// reproducible evaluations supply one so due work is not time-dependent.
#[must_use]
pub fn task_view_at(context: &PlanContext, as_of_epoch_seconds: i64) -> TaskView {
    task_view_at_with_overrides(context, as_of_epoch_seconds, &CheckOverrides::default())
}

/// Explicit-clock and explicit-policy form used by deterministic interactive
/// tests and one-run advanced checks.
#[must_use]
pub fn task_view_at_with_overrides(
    context: &PlanContext,
    as_of_epoch_seconds: i64,
    overrides: &CheckOverrides,
) -> TaskView {
    let (options, verdict) = effective_check_policy(context, overrides);
    let graph = Graph::new(context.plan().tasks.iter().map(|loaded| &loaded.task));
    let report = if overrides == &CheckOverrides::default() {
        context
            .structural_report_with(|| {
                aplan_core::validate_with_graph(context.plan(), &options, &graph)
            })
            .clone()
    } else {
        aplan_core::validate_with_graph(context.plan(), &options, &graph)
    };
    let check = CheckResponse {
        input: context.identity().clone(),
        denied: report.denied_by(&verdict),
        evidence_unresolved: aplan_core::unresolved_by_check(context.plan()),
        report,
    };
    let diagnostics = diagnostic_workbench(context, &check, &options, &verdict, overrides);
    let claims = claims::snapshot(&context.identity().repository_root, context.plan())
        .unwrap_or_else(|error| ClaimSnapshot::unavailable(epoch_now(), error.to_string()));
    let catalogue = task_catalogue_with_graph(context, &graph, &claims);
    let inspectors = task_inspectors(context, &graph, &check.report.diagnostics, &claims);
    let overview = plan_overview(
        context,
        &graph,
        &check,
        &diagnostics.check,
        as_of_epoch_seconds,
    );
    TaskView {
        check,
        catalogue,
        inspectors,
        overview,
        diagnostics,
    }
}

fn effective_check_policy(context: &PlanContext, overrides: &CheckOverrides) -> (Options, Verdict) {
    let mut options = context.options().clone();
    let mut verdict = context.verdict().clone();
    if overrides.no_require_effort {
        options.require_effort = false;
    }
    if overrides.no_traces {
        options.traces_root = None;
    } else if let Some(root) = &overrides.traces_root {
        options.traces_root = Some(root.clone());
    }
    match overrides.warning_policy {
        WarningPolicyOverride::Inherit => {}
        WarningPolicyOverride::Deny => verdict.deny_warnings = true,
        WarningPolicyOverride::Allow => verdict.deny_warnings = false,
    }
    (options, verdict)
}

fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

struct OverviewWorkProjection<'a> {
    live: Vec<&'a aplan_core::task::Task>,
    loaded_work: usize,
    live_work: usize,
    done_work: usize,
    dropped_work: usize,
    live_task_ids: Vec<String>,
    live_effort_days: f64,
    done_effort_days: f64,
    missing_effort_task_ids: Vec<String>,
    work_state: OverviewWorkState,
    effort_state: OverviewEffortState,
}

fn overview_work<'a>(graph: &Graph<'a>) -> OverviewWorkProjection<'a> {
    let work: Vec<_> = graph
        .by_id
        .values()
        .copied()
        .filter(|task| !task.is_milestone())
        .collect();
    let loaded_work = work.len();
    let live: Vec<_> = work
        .iter()
        .copied()
        .filter(|task| task.status != Status::Dropped)
        .collect();
    let live_work = live.len();
    let done_work = live
        .iter()
        .filter(|task| task.status == Status::Done)
        .count();
    let dropped_work = work
        .iter()
        .filter(|task| task.status == Status::Dropped)
        .count();
    let live_task_ids = live.iter().map(|task| task.id.clone()).collect::<Vec<_>>();
    let live_effort_days = live.iter().filter_map(|task| task.effort_days).sum::<f64>() + 0.0;
    let done_effort_days = live
        .iter()
        .filter(|task| task.status == Status::Done)
        .filter_map(|task| task.effort_days)
        .sum::<f64>()
        + 0.0;
    let missing_effort_task_ids = live
        .iter()
        .filter(|task| task.effort_days.is_none())
        .map(|task| task.id.clone())
        .collect::<Vec<_>>();
    let estimated = live_work.saturating_sub(missing_effort_task_ids.len());
    let effort_state = if live_work == 0 {
        OverviewEffortState::NoLiveWork
    } else if estimated == 0 {
        OverviewEffortState::None
    } else if missing_effort_task_ids.is_empty() {
        OverviewEffortState::Complete
    } else {
        OverviewEffortState::Partial
    };
    let work_state = if loaded_work == 0 {
        OverviewWorkState::NoLoadedWork
    } else if live_work == 0 {
        OverviewWorkState::OnlyDropped
    } else if done_work == live_work {
        OverviewWorkState::AllDone
    } else {
        OverviewWorkState::Active
    };

    OverviewWorkProjection {
        live,
        loaded_work,
        live_work,
        done_work,
        dropped_work,
        live_task_ids,
        live_effort_days,
        done_effort_days,
        missing_effort_task_ids,
        work_state,
        effort_state,
    }
}

fn overview_load(
    context: &PlanContext,
    loaded_tasks: usize,
) -> (OverviewLoadState, Vec<(String, PathBuf)>) {
    let unloadable = context
        .plan()
        .unloadable
        .iter()
        .map(|(id, path)| (id.clone(), path.clone()))
        .collect::<Vec<_>>();
    let load_state = if !unloadable.is_empty() && loaded_tasks > 0 {
        OverviewLoadState::PartiallyUnloadable
    } else if !context.plan().load_diagnostics.is_empty() {
        OverviewLoadState::Malformed
    } else if loaded_tasks == 0 {
        OverviewLoadState::Empty
    } else {
        OverviewLoadState::Complete
    };
    (load_state, unloadable)
}

struct OverviewCutProjection {
    cuts: Vec<OverviewCut>,
    task_ids: Vec<String>,
    work: usize,
    done_work: usize,
}

fn overview_cuts(graph: &Graph<'_>) -> OverviewCutProjection {
    let roadmap = graph.roadmap();
    let cuts = roadmap
        .cuts
        .iter()
        .map(|cut| OverviewCut {
            label: cut.version.clone(),
            milestone_id: cut.milestone_id.clone(),
            status: cut.status,
            borrows: cut.borrows,
            total: cut.total,
            done: cut.done,
            ready: cut.ready,
            blocked: cut.blocked,
            effort_days: cut.effort_days,
            done_effort_days: cut.done_effort_days,
            task_ids: cut
                .members
                .iter()
                .filter(|task| task.status != Status::Dropped)
                .map(|task| task.id.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    let mut cut_task_ids = BTreeSet::new();
    for cut in &cuts {
        if !cut.borrows {
            cut_task_ids.extend(cut.task_ids.iter().cloned());
        }
    }
    let cut_work = cut_task_ids.len();
    let done_cut_work = cut_task_ids
        .iter()
        .filter(|id| {
            graph
                .get(id)
                .is_some_and(|task| task.status == Status::Done)
        })
        .count();
    OverviewCutProjection {
        cuts,
        task_ids: cut_task_ids.into_iter().collect(),
        work: cut_work,
        done_work: done_cut_work,
    }
}

fn overview_due(
    live: &[&aplan_core::task::Task],
    as_of_epoch_seconds: i64,
) -> (Vec<OverviewDueTask>, Vec<OverviewWaitingTask>) {
    let mut due = Vec::new();
    let mut waiting = Vec::new();
    for task in live {
        if task.status != Status::Blocked {
            continue;
        }
        let Some(revisit_at) = &task.revisit_at else {
            continue;
        };
        let Some(revisit_epoch) = aplan_core::validate::rfc3339_to_epoch(revisit_at) else {
            continue;
        };
        if revisit_epoch > as_of_epoch_seconds {
            waiting.push(OverviewWaitingTask {
                id: task.id.clone(),
                title: task.title.clone(),
                revisit_at: revisit_at.clone(),
                seconds_until: revisit_epoch - as_of_epoch_seconds,
            });
        } else {
            due.push(OverviewDueTask {
                id: task.id.clone(),
                title: task.title.clone(),
                revisit_at: revisit_at.clone(),
                overdue_seconds: as_of_epoch_seconds - revisit_epoch,
            });
        }
    }
    due.sort_by(|left, right| {
        right
            .overdue_seconds
            .cmp(&left.overdue_seconds)
            .then_with(|| left.id.cmp(&right.id))
    });
    waiting.sort_by(|left, right| {
        left.seconds_until
            .cmp(&right.seconds_until)
            .then_with(|| left.id.cmp(&right.id))
    });
    (due, waiting)
}

fn diagnostic_workbench(
    context: &PlanContext,
    check: &CheckResponse,
    options: &Options,
    verdict: &Verdict,
    overrides: &CheckOverrides,
) -> DiagnosticWorkbench {
    let check_findings = check
        .report
        .diagnostics
        .iter()
        .map(|finding| core_finding(context, finding, verdict, DiagnosticScope::Check))
        .collect::<Vec<_>>();
    let load_failures = context.plan().load_diagnostics.len();
    let check_coverage = if load_failures > 0 && check.report.tasks_checked == 0 {
        DiagnosticCoverage::Unavailable(format!(
            "the plan has {load_failures} load diagnostic(s), so no task subjects were checked"
        ))
    } else if load_failures > 0 {
        DiagnosticCoverage::Partial {
            subjects: check.report.tasks_checked,
            unavailable: load_failures,
        }
    } else if check.report.tasks_checked == 0 {
        DiagnosticCoverage::NoSubjects("the plan loaded, but contains no task subjects".to_string())
    } else {
        DiagnosticCoverage::Complete {
            subjects: check.report.tasks_checked,
        }
    };
    let check_run = diagnostic_run(
        DiagnosticScope::Check,
        check_coverage,
        check_findings,
        check.denied || load_failures > 0,
        format!("identified plan {}", context.identity().sha256),
    );
    let documents = document_diagnostics(context);
    let relaxed = (context.options().require_effort && !options.require_effort)
        || (context.options().traces_root.is_some() && options.traces_root.is_none())
        || (context.verdict().deny_warnings && !verdict.deny_warnings);
    DiagnosticWorkbench {
        check: check_run,
        documents,
        run_sheet: CheckRunSheet {
            repository: check_policy_summary(context.options(), context.verdict()),
            effective: check_policy_summary(options, verdict),
            overrides: overrides.clone(),
            relaxed,
        },
    }
}

fn check_policy_summary(options: &Options, verdict: &Verdict) -> CheckPolicySummary {
    CheckPolicySummary {
        require_effort: options.require_effort,
        traces_root: options.traces_root.clone(),
        traces_repository: options.traces_repository.clone(),
        traces_external: options.traces_external.clone(),
        detect_drift: options.detect_drift,
        deny_warnings: verdict.deny_warnings,
        advisory: verdict.advisory.clone(),
    }
}

fn diagnostic_run(
    scope: DiagnosticScope,
    coverage: DiagnosticCoverage,
    findings: Vec<OverviewFinding>,
    denied: bool,
    input_label: String,
) -> DiagnosticRun {
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .count();
    let warnings = findings.len().saturating_sub(errors);
    let blocking = findings
        .iter()
        .filter(|finding| finding.contribution == VerdictContribution::Blocking)
        .count();
    let advisory = findings
        .iter()
        .filter(|finding| finding.contribution == VerdictContribution::Advisory)
        .count();
    DiagnosticRun {
        scope,
        coverage,
        findings,
        errors,
        warnings,
        blocking,
        advisory,
        denied,
        input_label,
    }
}

fn core_finding(
    context: &PlanContext,
    finding: &aplan_core::Diagnostic,
    verdict: &Verdict,
    scope: DiagnosticScope,
) -> OverviewFinding {
    let contribution = if verdict.is_advisory(finding.code) {
        VerdictContribution::Advisory
    } else if finding.severity == Severity::Error || verdict.deny_warnings {
        VerdictContribution::Blocking
    } else {
        VerdictContribution::NonBlocking
    };
    let subject = finding
        .task
        .as_deref()
        .map_or_else(|| "whole plan".to_string(), |task| format!("task {task}"));
    make_finding(
        scope,
        finding.code,
        finding.severity,
        subject,
        finding.task.clone(),
        finding.file.clone(),
        finding.location.map(|location| location.line),
        finding.location.map(|location| location.column),
        finding.message.clone(),
        finding.agent_instruction.clone(),
        finding.detail.clone(),
        contribution,
        finding
            .file
            .as_deref()
            .zip(finding.location)
            .and_then(|(path, location)| context.source_line(path, location.line)),
    )
}

fn document_diagnostics(context: &PlanContext) -> DiagnosticRun {
    let documents = context.documents();
    let input_label = match documents.state {
        DocumentInputState::Configured => {
            format!("configured documents ({})", documents.documents.len())
        }
        DocumentInputState::NotConfigured => "no configured documents".to_string(),
        DocumentInputState::Unavailable(_) => "configured document selection".to_string(),
    };
    match &documents.state {
        DocumentInputState::NotConfigured => {
            return diagnostic_run(
                DiagnosticScope::CheckDocs,
                DiagnosticCoverage::Unavailable(
                    "`.aplan.yml` declares no `docs.paths` or `docs.membership`; no document verdict was computed"
                        .to_string(),
                ),
                Vec::new(),
                false,
                input_label,
            );
        }
        DocumentInputState::Unavailable(reason) => {
            return diagnostic_run(
                DiagnosticScope::CheckDocs,
                DiagnosticCoverage::Unavailable(reason.clone()),
                Vec::new(),
                true,
                input_label,
            );
        }
        DocumentInputState::Configured => {}
    }

    let load_failures = context.plan().load_diagnostics.len();
    if load_failures > 0 {
        return diagnostic_run(
            DiagnosticScope::CheckDocs,
            DiagnosticCoverage::Unavailable(format!(
                "the plan has {load_failures} load diagnostic(s), so document references cannot be resolved safely"
            )),
            Vec::new(),
            true,
            input_label,
        );
    }

    let ids = context
        .plan()
        .tasks
        .iter()
        .map(|loaded| loaded.task.id.clone())
        .collect::<BTreeSet<_>>();
    if aplan_interop::docs::Vocabulary::of(&ids).is_empty() {
        return diagnostic_run(
            DiagnosticScope::CheckDocs,
            DiagnosticCoverage::NoSubjects(
                "the plan's ids teach no recognizable document-reference prefix; check-docs checked nothing"
                    .to_string(),
            ),
            Vec::new(),
            true,
            input_label,
        );
    }

    configured_document_diagnostics(context, documents, &ids, input_label)
}

fn configured_document_diagnostics(
    context: &PlanContext,
    documents: &DocumentInputs,
    ids: &BTreeSet<String>,
    input_label: String,
) -> DiagnosticRun {
    let milestones = context
        .plan()
        .tasks
        .iter()
        .filter(|loaded| loaded.task.is_milestone() && !loaded.task.is_release())
        .map(|loaded| {
            (
                loaded.task.id.clone(),
                loaded
                    .task
                    .blocked_by
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut findings = Vec::new();
    let (loaded, unavailable) = append_document_input_findings(&mut findings, documents);
    append_document_reference_findings(&mut findings, documents, ids);
    append_document_membership_findings(&mut findings, documents, &milestones, ids);

    let coverage = if unavailable > 0 && loaded == 0 {
        DiagnosticCoverage::Unavailable(
            "none of the configured documents could be read, so check-docs checked no document subjects"
                .to_string(),
        )
    } else if unavailable > 0 {
        DiagnosticCoverage::Partial {
            subjects: loaded,
            unavailable,
        }
    } else if loaded == 0 {
        DiagnosticCoverage::NoSubjects(
            "the configured document set contains no readable documents".to_string(),
        )
    } else {
        DiagnosticCoverage::Complete { subjects: loaded }
    };
    let denied = unavailable > 0 || !findings.is_empty();
    diagnostic_run(
        DiagnosticScope::CheckDocs,
        coverage,
        findings,
        denied,
        input_label,
    )
}

fn append_document_input_findings(
    findings: &mut Vec<OverviewFinding>,
    documents: &DocumentInputs,
) -> (usize, usize) {
    let mut loaded = 0;
    let mut unavailable = 0;
    for document in &documents.documents {
        let Err(error) = &document.content else {
            loaded += 1;
            continue;
        };
        unavailable += 1;
        findings.push(make_finding(
            DiagnosticScope::CheckDocs,
            "DOC-INPUT",
            Severity::Error,
            format!("document {}", document.label),
            None,
            Some(document.path.clone()),
            None,
            None,
            format!(
                "could not read configured document `{}`: {error}",
                document.label
            ),
            Some("Restore or remove the configured document path, then rerun check-docs.".into()),
            Vec::new(),
            VerdictContribution::Blocking,
            None,
        ));
    }
    (loaded, unavailable)
}

fn append_document_reference_findings(
    findings: &mut Vec<OverviewFinding>,
    documents: &DocumentInputs,
    ids: &BTreeSet<String>,
) {
    for document in &documents.documents {
        let Ok(text) = &document.content else {
            continue;
        };
        for reference in aplan_interop::docs::dangling(text, ids) {
            let near = aplan_interop::docs::nearest(&reference.id, ids);
            let hint = suggestion_hint(&near);
            let prefix = if reference.mistyped_prefix.is_empty() {
                String::new()
            } else {
                format!(
                    ", whose ids use {}",
                    reference
                        .mistyped_prefix
                        .iter()
                        .map(|prefix| format!("`{prefix}`"))
                        .collect::<Vec<_>>()
                        .join(" or ")
                )
            };
            findings.push(make_finding(
                DiagnosticScope::CheckDocs,
                "DOC-REFERENCE",
                Severity::Error,
                format!("reference {}", reference.id),
                None,
                Some(document.path.clone()),
                Some(reference.line),
                Some(reference.column),
                format!(
                    "`{}` is not a task in this plan{prefix}{hint}",
                    reference.id
                ),
                Some(format!(
                    "Correct or remove the reference `{}` in `{}`.",
                    reference.id, document.label
                )),
                near.iter()
                    .map(|candidate| (*candidate).to_string())
                    .collect(),
                VerdictContribution::Blocking,
                reference
                    .line
                    .checked_sub(1)
                    .and_then(|index| text.lines().nth(index))
                    .map(str::to_string),
            ));
        }
    }
}

fn append_document_membership_findings(
    findings: &mut Vec<OverviewFinding>,
    documents: &DocumentInputs,
    milestones: &BTreeMap<String, BTreeSet<String>>,
    ids: &BTreeSet<String>,
) {
    for document in &documents.documents {
        let (Ok(text), Some(membership)) = (&document.content, &document.membership) else {
            continue;
        };
        let declared_by = match membership {
            DocumentMembership::Line(marker) => {
                aplan_interop::docs::DeclaredBy::Line(marker.clone())
            }
            DocumentMembership::Table => aplan_interop::docs::DeclaredBy::Table,
        };
        for problem in aplan_interop::docs::membership(text, milestones, &declared_by, ids) {
            append_membership_findings(findings, document, text, &problem);
        }
    }
}

fn append_membership_findings(
    findings: &mut Vec<OverviewFinding>,
    document: &DocumentInput,
    text: &str,
    problem: &aplan_interop::docs::MembershipProblem,
) {
    let headings = membership_heading_lines(text, &problem.milestone);
    let line = (headings.len() == 1).then_some(headings[0]);
    let source_line = line.and_then(|line| {
        line.checked_sub(1)
            .and_then(|index| text.lines().nth(index))
            .map(str::to_string)
    });
    let ambiguous_detail = (headings.len() > 1).then(|| {
        format!(
            "matching section headings are at lines {}",
            headings
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    let mut push = |message: String, instruction: String| {
        findings.push(make_finding(
            DiagnosticScope::CheckDocs,
            "DOC-MEMBERSHIP",
            Severity::Error,
            format!("milestone {}", problem.milestone),
            Some(problem.milestone.clone()),
            Some(document.path.clone()),
            line,
            line.map(|_| 1),
            message,
            Some(instruction),
            ambiguous_detail.clone().into_iter().collect(),
            VerdictContribution::Blocking,
            source_line.clone(),
        ));
    };
    if problem.repeated_section {
        push(
            format!(
                "more than one heading names `{}`, so its membership declaration is ambiguous",
                problem.milestone
            ),
            format!(
                "Keep one `{}` section in `{}` or rename the unrelated heading.",
                problem.milestone, document.label
            ),
        );
    }
    if problem.no_declaration {
        push(
            format!(
                "`{}` has members and its document section declares none",
                problem.milestone
            ),
            format!(
                "Add the configured membership declaration to `{}` for `{}`.",
                document.label, problem.milestone
            ),
        );
    }
    if !problem.undeclared.is_empty() {
        push(
            format!(
                "`{}` includes {}, which the document does not declare",
                problem.milestone,
                quoted_ids(&problem.undeclared)
            ),
            format!(
                "Add {} to `{}`'s `{}` declaration, or remove them from the cut.",
                quoted_ids(&problem.undeclared),
                document.label,
                problem.milestone
            ),
        );
    }
    if !problem.unplanned.is_empty() {
        push(
            format!(
                "the document declares {}, which `{}` does not include",
                quoted_ids(&problem.unplanned),
                problem.milestone
            ),
            format!(
                "Remove {} from `{}`'s declaration, or add them to `{}`.",
                quoted_ids(&problem.unplanned),
                document.label,
                problem.milestone
            ),
        );
    }
}

fn membership_heading_lines(text: &str, milestone: &str) -> Vec<usize> {
    aplan_core::anchor::sections(text)
        .into_iter()
        .filter(|section| {
            let label = aplan_interop::docs::heading_label(&section.title);
            label == milestone || milestone.strip_prefix("M-") == Some(label.as_str())
        })
        // `Section::line` is zero-based because it indexes the line slice used
        // by the membership parser. Interactive locations are editor-style.
        .map(|section| section.line + 1)
        .collect()
}

fn suggestion_hint(near: &[&str]) -> String {
    match near.len() {
        0 => String::new(),
        1..=3 => format!(" (did you mean {}?)", near.join(", ")),
        count => format!(
            " (did you mean {}, or {} others?)",
            near[..3].join(", "),
            count - 3
        ),
    }
}

fn quoted_ids(ids: &[String]) -> String {
    ids.iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[allow(clippy::too_many_arguments)]
fn make_finding(
    scope: DiagnosticScope,
    code: &str,
    severity: Severity,
    subject: String,
    task: Option<String>,
    file: Option<PathBuf>,
    line: Option<usize>,
    column: Option<usize>,
    message: String,
    agent_instruction: Option<String>,
    detail: Vec<String>,
    contribution: VerdictContribution,
    source_line: Option<String>,
) -> OverviewFinding {
    let identity = finding_identity(
        scope,
        code,
        &subject,
        task.as_deref(),
        file.as_deref(),
        line,
        column,
        &message,
        agent_instruction.as_deref(),
        &detail,
    );
    OverviewFinding {
        identity,
        scope,
        code: code.to_string(),
        severity,
        subject,
        task,
        file,
        line,
        column,
        message,
        agent_instruction,
        detail,
        contribution,
        source_line,
    }
}

#[allow(clippy::too_many_arguments)]
fn finding_identity(
    scope: DiagnosticScope,
    code: &str,
    subject: &str,
    task: Option<&str>,
    file: Option<&Path>,
    line: Option<usize>,
    column: Option<usize>,
    message: &str,
    instruction: Option<&str>,
    detail: &[String],
) -> String {
    let mut digest = Sha256::new();
    for (label, value) in [
        ("scope", format!("{scope:?}")),
        ("code", code.to_string()),
        ("subject", subject.to_string()),
        ("task", task.unwrap_or_default().to_string()),
        (
            "file",
            file.map_or_else(String::new, |path| path.display().to_string()),
        ),
        (
            "line",
            line.map_or_else(String::new, |value| value.to_string()),
        ),
        (
            "column",
            column.map_or_else(String::new, |value| value.to_string()),
        ),
        ("message", message.to_string()),
        ("instruction", instruction.unwrap_or_default().to_string()),
        ("detail", detail.join("\n")),
    ] {
        hash_part(&mut digest, label.as_bytes(), value.as_bytes());
    }
    to_hex(&digest.finalize())
}

fn plan_overview(
    context: &PlanContext,
    graph: &Graph<'_>,
    check: &CheckResponse,
    diagnostics: &DiagnosticRun,
    as_of_epoch_seconds: i64,
) -> PlanOverview {
    let loaded_tasks = graph.by_id.len();
    let work = overview_work(graph);
    let (load_state, unloadable) = overview_load(context, loaded_tasks);
    let cuts = overview_cuts(graph);
    let ready = graph
        .ready_ranked()
        .into_iter()
        .map(|(task, _)| OverviewTask {
            id: task.id.clone(),
            title: task.title.clone(),
        })
        .collect();
    let (due, waiting_revisits) = overview_due(&work.live, as_of_epoch_seconds);
    let critical_path = graph.critical_path().map(|path| {
        (
            path.total_effort_days,
            path.tasks
                .into_iter()
                .map(|task| OverviewTask {
                    id: task.id,
                    title: task.title,
                })
                .collect(),
        )
    });
    PlanOverview {
        input: check.input.clone(),
        load_state,
        work_state: work.work_state,
        effort_state: work.effort_state,
        loaded_tasks,
        loaded_work: work.loaded_work,
        live_work: work.live_work,
        done_work: work.done_work,
        dropped_work: work.dropped_work,
        live_task_ids: work.live_task_ids,
        live_effort_days: work.live_effort_days,
        done_effort_days: work.done_effort_days,
        missing_effort_task_ids: work.missing_effort_task_ids,
        cuts: cuts.cuts,
        cut_task_ids: cuts.task_ids,
        cut_work: cuts.work,
        done_cut_work: cuts.done_work,
        ready,
        as_of_epoch_seconds,
        due,
        waiting_revisits,
        critical_path,
        findings: diagnostics.findings.clone(),
        errors: diagnostics.errors,
        warnings: diagnostics.warnings,
        denied: diagnostics.denied,
        unloadable,
    }
}

fn task_inspectors(
    context: &PlanContext,
    graph: &Graph<'_>,
    diagnostics: &[aplan_core::Diagnostic],
    claims: &ClaimSnapshot,
) -> Vec<Show> {
    let rows = graph.task_rows();
    rows.tasks
        .iter()
        .filter_map(|row| {
            aplan_core::Show::assemble(
                aplan_core::ShowSources {
                    graph,
                    rows: &rows,
                    diagnostics,
                    unloadable: &context.plan().unloadable,
                    claims,
                },
                &row.id,
                false,
                context.task_body(&row.id).map(str::to_string),
            )
        })
        .collect()
}

fn sort_task_matches(rows: &mut [TaskBrowserMatch], sort: TaskSort) {
    rows.sort_by(|a, b| {
        let left = &a.task;
        let right = &b.task;
        let primary = match sort {
            TaskSort::ReadyRank => left
                .ready_rank
                .unwrap_or(usize::MAX)
                .cmp(&right.ready_rank.unwrap_or(usize::MAX)),
            TaskSort::Id => left.id.cmp(&right.id),
            TaskSort::Title => left.title.cmp(&right.title),
            TaskSort::Status => status_order(left.status).cmp(&status_order(right.status)),
            TaskSort::Cut => left.cut.cmp(&right.cut),
            TaskSort::Phase => left.phase.cmp(&right.phase),
            TaskSort::Owner => left.owner.cmp(&right.owner),
        };
        primary.then_with(|| left.id.cmp(&right.id))
    });
}

fn match_spans(haystack: &str, needle: &str) -> Vec<MatchSpan> {
    if needle.is_empty() {
        return Vec::new();
    }
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut folded = String::new();
    let mut segments = Vec::new();
    for (original_start, character) in haystack.char_indices() {
        let original_end = original_start + character.len_utf8();
        let folded_start = folded.len();
        folded.extend(character.to_lowercase());
        segments.push((folded_start, folded.len(), original_start, original_end));
    }

    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(relative) = folded[from..].find(&needle) {
        let start = from + relative;
        let end = start + needle.len();
        let original_start = segments
            .iter()
            .find(|(_, folded_end, _, _)| *folded_end > start)
            .map(|(_, _, original_start, _)| *original_start);
        let original_end = segments
            .iter()
            .rev()
            .find(|(folded_start, _, _, _)| *folded_start < end)
            .map(|(_, _, _, original_end)| *original_end);
        if let (Some(start), Some(end)) = (original_start, original_end) {
            spans.push(MatchSpan { start, end });
        }
        from = end;
    }
    spans
}

fn matches_folded(haystack: &str, folded_needle: &str) -> bool {
    !folded_needle.is_empty() && haystack.to_lowercase().contains(folded_needle)
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Atomically replace a staged destination, tolerating only the short-lived
/// sharing denial Windows reports while another application reader closes the
/// old file.
pub(crate) fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        const ATTEMPTS: usize = 64;
        for attempt in 0..ATTEMPTS {
            match std::fs::rename(from, to) {
                Ok(()) => return Ok(()),
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        && attempt + 1 < ATTEMPTS =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("the bounded Windows rename loop returns")
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

fn input_digest(
    plan_sha256: &str,
    repository_root: &Path,
    repository: &RepositoryIdentity,
    configuration: &ConfigurationIdentity,
    trust: Trust,
    options: &Options,
    verdict: &Verdict,
) -> String {
    let mut digest = Sha256::new();
    hash_part(&mut digest, b"source", b"working_tree");
    hash_part(&mut digest, b"plan", plan_sha256.as_bytes());
    hash_part(
        &mut digest,
        b"repository_root",
        repository_root.as_os_str().as_encoded_bytes(),
    );
    hash_part(
        &mut digest,
        b"configuration_path",
        configuration
            .path
            .as_deref()
            .map_or(&[][..], |path| path.as_os_str().as_encoded_bytes()),
    );
    hash_part(
        &mut digest,
        b"configuration_semantics",
        configuration.sha256.as_bytes(),
    );
    hash_json_part(
        &mut digest,
        b"configuration_content",
        &configuration.content,
    );
    hash_repository_identity(&mut digest, repository);
    hash_part(
        &mut digest,
        b"trust",
        match trust {
            Trust::Hermetic => &b"hermetic"[..],
            Trust::RepositoryRead => &b"repository_read"[..],
            Trust::ConfiguredProcesses => &b"configured_processes"[..],
        },
    );
    hash_part(
        &mut digest,
        b"validation_policy",
        validation_policy_digest(options, verdict).as_bytes(),
    );

    to_hex(&digest.finalize())
}

fn validation_policy_digest(options: &Options, verdict: &Verdict) -> String {
    let mut digest = Sha256::new();
    hash_part(
        &mut digest,
        b"require_effort",
        &[u8::from(options.require_effort)],
    );
    hash_json_part(&mut digest, b"require_impact", &options.require_impact);
    hash_optional_path(&mut digest, b"traces_root", options.traces_root.as_deref());
    hash_optional_path(
        &mut digest,
        b"traces_repository",
        options.traces_repository.as_deref(),
    );
    hash_part(
        &mut digest,
        b"traces_external_count",
        &options.traces_external.len().to_le_bytes(),
    );
    for path in &options.traces_external {
        hash_part(
            &mut digest,
            b"traces_external_path",
            path.as_os_str().as_encoded_bytes(),
        );
    }
    hash_json_part(
        &mut digest,
        b"require_min_strength",
        &options.require_min_strength,
    );
    hash_json_part(&mut digest, b"evidence_scopes", &options.evidence_scopes);
    hash_json_part(&mut digest, b"exempt_tags", &options.exempt_tags);
    hash_part(
        &mut digest,
        b"detect_drift",
        &[u8::from(options.detect_drift)],
    );
    hash_part(
        &mut digest,
        b"require_cut_membership",
        &[u8::from(options.require_cut_membership)],
    );
    hash_json_part(&mut digest, b"tag_families", &options.tag_families);
    hash_json_part(&mut digest, b"owners", &options.owners);
    match &options.agents_block {
        Some(block) => {
            hash_part(&mut digest, b"agents_block_state", b"present");
            hash_part(
                &mut digest,
                b"agents_block_path",
                block.path.as_os_str().as_encoded_bytes(),
            );
            hash_part(
                &mut digest,
                b"agents_block_current",
                block.current.as_bytes(),
            );
        }
        None => hash_part(&mut digest, b"agents_block_state", b"absent"),
    }
    hash_optional_path(&mut digest, b"config_file", options.config_file.as_deref());
    hash_part(
        &mut digest,
        b"deny_warnings",
        &[u8::from(verdict.deny_warnings)],
    );
    hash_json_part(&mut digest, b"advisory", &verdict.advisory);
    to_hex(&digest.finalize())
}

fn hash_json_part<T: Serialize>(digest: &mut Sha256, label: &[u8], value: &T) {
    let bytes = serde_json::to_vec(value)
        .expect("validation policy values have a canonical JSON representation");
    hash_part(digest, label, &bytes);
}

fn hash_optional_path(digest: &mut Sha256, label: &[u8], path: Option<&Path>) {
    match path {
        Some(path) => {
            hash_part(digest, label, b"present");
            hash_part(digest, b"path", path.as_os_str().as_encoded_bytes());
        }
        None => hash_part(digest, label, b"absent"),
    }
}

pub(crate) fn io_error_kind_name(kind: std::io::ErrorKind) -> &'static str {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::NotFound => "not_found",
        ErrorKind::PermissionDenied => "permission_denied",
        ErrorKind::ConnectionRefused => "connection_refused",
        ErrorKind::ConnectionReset => "connection_reset",
        ErrorKind::HostUnreachable => "host_unreachable",
        ErrorKind::NetworkUnreachable => "network_unreachable",
        ErrorKind::ConnectionAborted => "connection_aborted",
        ErrorKind::NotConnected => "not_connected",
        ErrorKind::AddrInUse => "address_in_use",
        ErrorKind::AddrNotAvailable => "address_not_available",
        ErrorKind::NetworkDown => "network_down",
        ErrorKind::BrokenPipe => "broken_pipe",
        ErrorKind::AlreadyExists => "already_exists",
        ErrorKind::WouldBlock => "would_block",
        ErrorKind::NotADirectory => "not_a_directory",
        ErrorKind::IsADirectory => "is_a_directory",
        ErrorKind::DirectoryNotEmpty => "directory_not_empty",
        ErrorKind::ReadOnlyFilesystem => "read_only_filesystem",
        ErrorKind::StaleNetworkFileHandle => "stale_network_file_handle",
        ErrorKind::InvalidInput => "invalid_input",
        ErrorKind::InvalidData => "invalid_data",
        ErrorKind::TimedOut => "timed_out",
        ErrorKind::WriteZero => "write_zero",
        ErrorKind::StorageFull => "storage_full",
        ErrorKind::NotSeekable => "not_seekable",
        ErrorKind::QuotaExceeded => "quota_exceeded",
        ErrorKind::FileTooLarge => "file_too_large",
        ErrorKind::ResourceBusy => "resource_busy",
        ErrorKind::ExecutableFileBusy => "executable_file_busy",
        ErrorKind::Deadlock => "deadlock",
        ErrorKind::CrossesDevices => "crosses_devices",
        ErrorKind::TooManyLinks => "too_many_links",
        ErrorKind::InvalidFilename => "invalid_filename",
        ErrorKind::ArgumentListTooLong => "argument_list_too_long",
        ErrorKind::Interrupted => "interrupted",
        ErrorKind::Unsupported => "unsupported",
        ErrorKind::UnexpectedEof => "unexpected_eof",
        ErrorKind::OutOfMemory => "out_of_memory",
        _ => "future_error_kind",
    }
}

fn hash_repository_identity(digest: &mut Sha256, repository: &RepositoryIdentity) {
    match repository {
        RepositoryIdentity::NotObserved { reason } => {
            hash_part(digest, b"repository_state", b"not_observed");
            hash_part(digest, b"repository_reason", reason.as_bytes());
        }
        RepositoryIdentity::Unavailable { reason } => {
            hash_part(digest, b"repository_state", b"unavailable");
            hash_part(digest, b"repository_reason", reason.as_bytes());
        }
        RepositoryIdentity::Observed {
            head,
            modified,
            untracked,
            status_sha256,
        } => {
            hash_part(digest, b"repository_state", b"observed");
            hash_part(
                digest,
                b"repository_head",
                head.as_deref().unwrap_or_default().as_bytes(),
            );
            hash_part(
                digest,
                b"repository_modified",
                &u64::try_from(*modified).unwrap_or(u64::MAX).to_le_bytes(),
            );
            hash_part(
                digest,
                b"repository_untracked",
                &u64::try_from(*untracked).unwrap_or(u64::MAX).to_le_bytes(),
            );
            hash_part(digest, b"repository_status", status_sha256.as_bytes());
        }
    }
}

fn plan_input_digest(plan_inputs: &[aplan_core::PlanFileInput], plan_root: &Path) -> String {
    let mut digest = Sha256::new();
    hash_part(
        &mut digest,
        b"plan_root",
        plan_root.as_os_str().as_encoded_bytes(),
    );
    let mut files: Vec<_> = plan_inputs.iter().collect();
    files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    for file in files {
        let path = absolute(&file.path);
        hash_part(
            &mut digest,
            b"plan_candidate_path",
            path.as_os_str().as_encoded_bytes(),
        );
        match &file.content {
            aplan_core::PlanFileContent::Bytes(bytes) => {
                hash_part(
                    &mut digest,
                    b"plan_candidate_sha256",
                    to_hex(&Sha256::digest(bytes)).as_bytes(),
                );
            }
            aplan_core::PlanFileContent::Sha256(sha256) => {
                hash_part(&mut digest, b"plan_candidate_sha256", sha256.as_bytes());
            }
            aplan_core::PlanFileContent::ReadError { kind, message } => {
                hash_part(
                    &mut digest,
                    b"plan_candidate_read_error_kind",
                    io_error_kind_name(*kind).as_bytes(),
                );
                hash_part(
                    &mut digest,
                    b"plan_candidate_read_error_message",
                    message.as_bytes(),
                );
            }
        }
    }
    to_hex(&digest.finalize())
}

fn repository_identity(repository_root: &Path, trust: Trust) -> RepositoryIdentity {
    if trust == Trust::Hermetic {
        return RepositoryIdentity::NotObserved {
            reason: "repository reads are outside this operation's trust boundary".to_string(),
        };
    }
    let repo = match aplan_evidence::Repo::discover(repository_root) {
        Ok(repo) => repo,
        Err(error) => return repository_unavailable(&error),
    };
    let status = match repo.worktree_status() {
        Ok(status) => status,
        Err(error) => return repository_unavailable(&error),
    };
    let mut digest = Sha256::new();
    let worktree_root =
        git_worktree_root(repository_root).unwrap_or_else(|| absolute(repository_root));
    for entry in &status.entries {
        hash_part(&mut digest, b"path", &entry.path);
        hash_part(&mut digest, b"status", &entry.status_bits.to_le_bytes());
        if let Err(error) = hash_worktree_material(&mut digest, &worktree_root, &entry.path) {
            return RepositoryIdentity::Unavailable { reason: error };
        }
    }
    RepositoryIdentity::Observed {
        head: repo.head_sha(),
        modified: status.changes.modified,
        untracked: status.changes.untracked,
        status_sha256: to_hex(&digest.finalize()),
    }
}

fn hash_worktree_material(
    digest: &mut Sha256,
    worktree_root: &Path,
    path: &[u8],
) -> Result<(), String> {
    #[cfg(unix)]
    let relative = {
        use std::os::unix::ffi::OsStrExt as _;
        PathBuf::from(std::ffi::OsStr::from_bytes(path))
    };
    #[cfg(not(unix))]
    let relative = PathBuf::from(
        std::str::from_utf8(path)
            .map_err(|_| "a changed repository path is not representable on this platform")?,
    );
    let selected = worktree_root.join(&relative);
    let before = match std::fs::symlink_metadata(&selected) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            hash_part(digest, b"worktree_material", b"absent");
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "reading changed repository path `{}`: {error}",
                relative.display()
            ));
        }
    };
    if before.file_type().is_symlink() {
        let target = std::fs::read_link(&selected).map_err(|error| {
            format!(
                "reading changed repository symbolic link `{}`: {error}",
                relative.display()
            )
        })?;
        hash_part(digest, b"worktree_material_kind", b"symlink");
        hash_part(
            digest,
            b"worktree_material_target",
            target.as_os_str().as_encoded_bytes(),
        );
        return Ok(());
    }
    if before.is_dir() {
        hash_part(digest, b"worktree_material_kind", b"directory");
        return Ok(());
    }
    if !before.is_file() {
        hash_part(digest, b"worktree_material_kind", b"special");
        return Ok(());
    }
    let bytes = std::fs::read(&selected).map_err(|error| {
        format!(
            "reading changed repository file `{}`: {error}",
            relative.display()
        )
    })?;
    let after = std::fs::symlink_metadata(&selected).map_err(|error| {
        format!(
            "rechecking changed repository file `{}`: {error}",
            relative.display()
        )
    })?;
    if !after.is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(format!(
            "changed repository file `{}` moved while it was being identified",
            relative.display()
        ));
    }
    hash_part(digest, b"worktree_material_kind", b"file");
    hash_part(digest, b"worktree_material_bytes", &bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        hash_part(
            digest,
            b"worktree_material_mode",
            &after.permissions().mode().to_le_bytes(),
        );
    }
    Ok(())
}

fn repository_unavailable(error: &impl std::fmt::Display) -> RepositoryIdentity {
    RepositoryIdentity::Unavailable {
        reason: error.to_string(),
    }
}

fn hash_documents(digest: &mut Sha256, documents: &DocumentInputs) {
    let state = match &documents.state {
        DocumentInputState::Configured => "configured",
        DocumentInputState::NotConfigured => "not_configured",
        DocumentInputState::Unavailable(reason) => {
            hash_part(digest, b"document_unavailable_reason", reason.as_bytes());
            "unavailable"
        }
    };
    hash_part(digest, b"document_input_state", state.as_bytes());
    for document in &documents.documents {
        hash_part(digest, b"document_label", document.label.as_bytes());
        hash_part(
            digest,
            b"document_path",
            absolute(&document.path).as_os_str().as_encoded_bytes(),
        );
        match &document.membership {
            None => hash_part(digest, b"document_membership", b"absent"),
            Some(DocumentMembership::Table) => {
                hash_part(digest, b"document_membership", b"table");
            }
            Some(DocumentMembership::Line(marker)) => {
                hash_part(digest, b"document_membership", b"line");
                hash_part(digest, b"document_membership_marker", marker.as_bytes());
            }
        }
        match &document.content {
            Ok(text) => hash_part(digest, b"document_text", text.as_bytes()),
            Err(error) => hash_part(digest, b"document_read_error", error.as_bytes()),
        }
    }
}

fn document_input_digest(plan_sha256: &str, documents: &DocumentInputs) -> String {
    let mut digest = Sha256::new();
    hash_part(&mut digest, b"plan_input", plan_sha256.as_bytes());
    hash_documents(&mut digest, documents);
    to_hex(&digest.finalize())
}

fn hash_part(digest: &mut Sha256, label: &[u8], value: &[u8]) {
    digest.update(label.len().to_le_bytes());
    digest.update(label);
    digest.update(value.len().to_le_bytes());
    digest.update(value);
}

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_names_are_stable_and_total_for_cache_identity() {
        use std::io::ErrorKind;

        let cases = [
            (ErrorKind::NotFound, "not_found"),
            (ErrorKind::PermissionDenied, "permission_denied"),
            (ErrorKind::ConnectionRefused, "connection_refused"),
            (ErrorKind::ConnectionReset, "connection_reset"),
            (ErrorKind::HostUnreachable, "host_unreachable"),
            (ErrorKind::NetworkUnreachable, "network_unreachable"),
            (ErrorKind::ConnectionAborted, "connection_aborted"),
            (ErrorKind::NotConnected, "not_connected"),
            (ErrorKind::AddrInUse, "address_in_use"),
            (ErrorKind::AddrNotAvailable, "address_not_available"),
            (ErrorKind::NetworkDown, "network_down"),
            (ErrorKind::BrokenPipe, "broken_pipe"),
            (ErrorKind::AlreadyExists, "already_exists"),
            (ErrorKind::WouldBlock, "would_block"),
            (ErrorKind::NotADirectory, "not_a_directory"),
            (ErrorKind::IsADirectory, "is_a_directory"),
            (ErrorKind::DirectoryNotEmpty, "directory_not_empty"),
            (ErrorKind::ReadOnlyFilesystem, "read_only_filesystem"),
            (
                ErrorKind::StaleNetworkFileHandle,
                "stale_network_file_handle",
            ),
            (ErrorKind::InvalidInput, "invalid_input"),
            (ErrorKind::InvalidData, "invalid_data"),
            (ErrorKind::TimedOut, "timed_out"),
            (ErrorKind::WriteZero, "write_zero"),
            (ErrorKind::StorageFull, "storage_full"),
            (ErrorKind::NotSeekable, "not_seekable"),
            (ErrorKind::QuotaExceeded, "quota_exceeded"),
            (ErrorKind::FileTooLarge, "file_too_large"),
            (ErrorKind::ResourceBusy, "resource_busy"),
            (ErrorKind::ExecutableFileBusy, "executable_file_busy"),
            (ErrorKind::Deadlock, "deadlock"),
            (ErrorKind::CrossesDevices, "crosses_devices"),
            (ErrorKind::TooManyLinks, "too_many_links"),
            (ErrorKind::InvalidFilename, "invalid_filename"),
            (ErrorKind::ArgumentListTooLong, "argument_list_too_long"),
            (ErrorKind::Interrupted, "interrupted"),
            (ErrorKind::Unsupported, "unsupported"),
            (ErrorKind::UnexpectedEof, "unexpected_eof"),
            (ErrorKind::OutOfMemory, "out_of_memory"),
        ];

        for (kind, expected) in cases {
            assert_eq!(io_error_kind_name(kind), expected, "{kind:?}");
        }
        assert_eq!(
            io_error_kind_name(ErrorKind::Other),
            "future_error_kind",
            "new or deliberately unclassified kinds must retain a deterministic fallback"
        );
    }

    #[test]
    fn structural_report_cache_debug_output_exposes_state_not_callbacks() {
        let empty = StructuralReportCache::new(None, |_| {});
        assert_eq!(
            format!("{empty:?}"),
            "StructuralReportCache { cached: false, .. }"
        );
        assert_eq!(
            format!("{:?}", empty.publish),
            "StructuralReportPublisher(..)"
        );

        let populated = StructuralReportCache::new(
            Some(Report {
                diagnostics: Vec::new(),
                tasks_checked: 0,
            }),
            |_| {},
        );
        assert!(format!("{populated:?}").contains("cached: true"));
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
        std::fs::write(
            dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("first");
        std::fs::write(
            dir.path().join("aplan/tasks/A-2.md"),
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("second");
        std::fs::write(dir.path().join(".aplan.yml"), "deny_warnings: false\n").expect("config");
        dir
    }

    #[test]
    fn input_identity_uses_typed_canonical_policy_bytes() {
        // Proof for `input_identity.uses_typed_canonical_bytes`.
        let mut first = Options::default();
        first.tag_families.insert(
            "z".to_string(),
            aplan_core::validate::TagFamily::members(vec!["z-one".to_string()]),
        );
        first.tag_families.insert(
            "a".to_string(),
            aplan_core::validate::TagFamily::members(vec!["a-one".to_string()]),
        );
        let mut second = Options::default();
        second.tag_families.insert(
            "a".to_string(),
            aplan_core::validate::TagFamily::members(vec!["a-one".to_string()]),
        );
        second.tag_families.insert(
            "z".to_string(),
            aplan_core::validate::TagFamily::members(vec!["z-one".to_string()]),
        );
        let verdict = Verdict {
            deny_warnings: true,
            advisory: vec!["APLAN009".to_string()],
        };
        assert_eq!(
            validation_policy_digest(&first, &verdict),
            validation_policy_digest(&second, &verdict)
        );

        second.require_effort = false;
        assert_ne!(
            validation_policy_digest(&first, &verdict),
            validation_policy_digest(&second, &verdict)
        );

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt as _;
            first.traces_root = Some(PathBuf::from(std::ffi::OsString::from_vec(vec![0x80])));
            second.traces_root = Some(PathBuf::from(std::ffi::OsString::from_vec(vec![0x81])));
            assert_ne!(
                validation_policy_digest(&first, &verdict),
                validation_policy_digest(&second, &verdict)
            );
        }
    }

    #[test]
    fn digest_only_plan_inputs_preserve_identity_without_retaining_bodies() {
        let directory = fixture();
        let root = directory.path().join("aplan");
        let loaded = aplan_core::load_plan_identified(&root);
        let mut digests = loaded.clone();
        for input in &mut digests.inputs {
            if let aplan_core::PlanFileContent::Bytes(bytes) = &input.content {
                input.content = aplan_core::PlanFileContent::Sha256(to_hex(&Sha256::digest(bytes)));
            }
        }
        let inputs = || ContextInputs {
            repository_root: directory.path().to_path_buf(),
            configuration_file: Some(directory.path().join(".aplan.yml")),
            trust: Trust::Hermetic,
            options: Options::default(),
            verdict: Verdict::default(),
        };
        let retained = PlanContext::new(loaded, inputs());
        let identified = PlanContext::new(digests, inputs());
        assert_eq!(retained.identity(), identified.identity());
        assert_eq!(retained.task_body("A-1"), Some(""));
        assert_eq!(identified.task_body("A-1"), None);
    }

    fn test_context(
        dir: &tempfile::TempDir,
        trust: Trust,
        options: Options,
        verdict: Verdict,
    ) -> PlanContext {
        let root = dir.path().join("aplan");
        PlanContext::new(
            aplan_core::load_plan_identified(&root),
            ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(dir.path().join(".aplan.yml")),
                trust,
                options,
                verdict,
            },
        )
    }

    fn git(dir: &tempfile::TempDir, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Proof for `tui.verification_is_typed_and_non_blocking`,
    /// `tui.verification_execution_is_preview_bound` and
    /// `progress.events_are_generation_safe`: application
    /// verification combines the structural and evidence engines over one
    /// identified snapshot, while resolver policy receives its own stale-input
    /// identity and execute remains a separate trust choice.
    #[test]
    fn verification_context_is_identified_and_uses_the_typed_engine() {
        let dir = fixture();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "fixture"]);
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let options = aplan_evidence::VerifyOptions::default();
        let baseline = verification_run_sheet(&context, &options);
        assert_eq!(baseline.trust, Trust::RepositoryRead);
        assert_eq!(baseline.execute, Availability::Unavailable);
        assert_eq!(baseline.providers.test_runners, 0);
        assert_eq!(baseline.providers.url_check, Availability::Unavailable);
        let mut execute = options.clone();
        execute.execute = true;
        assert_eq!(
            baseline.input_sha256,
            verification_run_sheet(&context, &execute).input_sha256,
            "execute is a request, not a hidden change to resolver inputs"
        );
        let mut changed_policy = options.clone();
        changed_policy.max_unrelated_files = Some(7);
        assert_ne!(
            baseline.input_sha256,
            verification_run_sheet(&context, &changed_policy).input_sha256,
            "resolver policy participates in stale-result identity"
        );
        let mut changed_drift = options.clone();
        changed_drift.detect_drift = true;
        assert!(
            verification_run_sheet(&context, &changed_drift)
                .policy
                .detect_drift
        );

        let result = verify_context(&context, &options).expect("verification runs");
        assert_eq!(result.state, VerificationState::Completed);
        assert_eq!(result.run.scope, DiagnosticScope::Verify);
        assert_eq!(context.identity().trust, Trust::Hermetic);
        assert_eq!(result.input.trust, Trust::RepositoryRead);
        assert_eq!(result.input.sha256, baseline.input_sha256);
        assert_eq!(result.sheet, baseline);
        assert!(!result.execute);

        std::fs::write(
            dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: done\neffort_days: 1\npaths: [src/**, proof.md]\nevidence:\n  - kind: test\n    ref: known_name\n  - kind: file\n    ref: proof.md\n---\n",
        )
        .expect("evidence task");
        let evidence_context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let runner = aplan_evidence::TestRunner {
            weight: None,
            exclusive: None,
            name: Some("unit".to_string()),
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
            list: vec![
                "git".to_string(),
                "-c".to_string(),
                "aplan.tests=suite::known_name".to_string(),
                "config".to_string(),
                "--get".to_string(),
                "aplan.tests".to_string(),
            ],
            run: vec![
                "git".to_string(),
                "rev-parse".to_string(),
                "--sq-quote".to_string(),
                "{test}".to_string(),
            ],
            batch: None,
            timeout_seconds: None,
        };
        let repair_options = aplan_evidence::VerifyOptions {
            test_runner: Some(aplan_evidence::TestRunners::One(runner)),
            ..aplan_evidence::VerifyOptions::default()
        };
        let mut progress = Vec::new();
        let operation = OperationId::new(41);
        let mut repaired = verify_context_with_progress(
            &evidence_context,
            &repair_options,
            operation,
            &mut |event| {
                progress.push(event);
            },
        )
        .expect("near match is a finding, not a transport failure");
        assert_eq!(repaired.sheet.trust, Trust::ConfiguredProcesses);
        assert_eq!(repaired.input.trust, Trust::ConfiguredProcesses);
        assert_eq!(repaired.input.sha256, repaired.sheet.input_sha256);
        assert_eq!(repaired.sheet.execute, Availability::Available);
        assert_eq!(repaired.sheet.providers.test_runners, 1);
        assert_eq!(repaired.costs.len(), 1);
        let mut state = ProgressState::new(operation, OperationKind::Verification);
        for event in progress.iter().copied() {
            state
                .accept(event)
                .expect("application emitted valid progress");
        }
        assert!(progress.iter().all(|event| event.operation == operation));
        assert!(progress.iter().any(|event| {
            event.subject
                == ProgressSubject::Phase {
                    phase: ProgressPhase::EvidenceProvider,
                    child: Some(ProgressChildId::new(1)),
                }
                && event.lifecycle == ProgressLifecycle::Started
        }));
        assert!(progress.iter().any(|event| {
            event.subject
                == ProgressSubject::Phase {
                    phase: ProgressPhase::EvidenceProvider,
                    child: None,
                }
                && event.lifecycle == ProgressLifecycle::Completed
                && event.measure == ProgressMeasure::bounded(1, 1, ProgressUnit::Providers)
        }));
        assert_eq!(
            repaired.observation.state,
            EvidenceObservationState::Unsupported,
            "a run without an exhaustive material-input claim is immediate but non-reusable"
        );
        assert_eq!(repaired.costs[0].subject, "the `unit` test runner");
        assert_eq!(
            repaired.repairs,
            vec![VerificationRepair {
                task: Some("A-1".to_string()),
                from_kind: "test".to_string(),
                from_reference: "known_name".to_string(),
                to_kind: "test".to_string(),
                to_reference: "suite::known_name".to_string(),
            }]
        );
        assert!(repaired.citations.iter().any(|citation| {
            citation.task == "A-1"
                && citation.reference == "known_name"
                && citation.resolver == "configured test runner"
                && citation.strength == CitationStrength::Recorded
                && citation.outcome == CitationOutcome::Failed
                && citation.observation_state == EvidenceObservationState::Unsupported
        }));

        repaired.observation.identity = Some("provider-observation".to_string());
        let mut inspector = task_view(&evidence_context)
            .inspectors
            .into_iter()
            .find(|show| show.id == "A-1")
            .expect("A-1 inspector");
        apply_verification_to_show(&mut inspector, &repaired);
        let test = inspector
            .evidence
            .iter()
            .find(|citation| citation.kind == "test")
            .expect("test citation");
        let file = inspector
            .evidence
            .iter()
            .find(|citation| citation.kind == "file")
            .expect("file citation");
        assert_eq!(
            test.observation.identity.as_deref(),
            Some("provider-observation")
        );
        assert!(
            file.observation.identity.is_none(),
            "a test-catalogue identity leaked onto repository-backed evidence"
        );
    }

    #[test]
    fn context_identity_moves_with_each_input_and_not_with_time() {
        let dir = fixture();
        let baseline = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let same = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        assert_eq!(baseline.identity(), same.identity());
        assert_eq!(baseline.identity().sha256.len(), 64);
        assert!(
            baseline
                .identity()
                .configuration
                .path
                .as_ref()
                .is_some_and(|path| path.is_absolute())
        );

        let other_trust = test_context(
            &dir,
            Trust::RepositoryRead,
            Options::default(),
            Verdict::default(),
        );
        assert_ne!(baseline.identity().sha256, other_trust.identity().sha256);

        let strict = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict {
                deny_warnings: true,
                advisory: Vec::new(),
            },
        );
        assert_ne!(baseline.identity().sha256, strict.identity().sha256);

        // The application boundary hashes resolved policy, not a later read of
        // its source file. A source-only change cannot relabel an old policy.
        std::fs::write(dir.path().join(".aplan.yml"), "deny_warnings: true\n")
            .expect("change config");
        let same_resolved_policy = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        assert_eq!(
            baseline.identity().sha256,
            same_resolved_policy.identity().sha256
        );

        let task = dir.path().join("aplan/tasks/A-1.md");
        let bytes = std::fs::read_to_string(&task).expect("read task");
        std::fs::write(&task, bytes.replace("first", "changed")).expect("change task");
        let changed_task = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        assert_ne!(
            same_resolved_policy.identity().sha256,
            changed_task.identity().sha256
        );

        std::fs::remove_file(dir.path().join(".aplan.yml")).expect("remove config");
        let missing_config = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        assert_eq!(
            missing_config.identity().configuration.path,
            baseline.identity().configuration.path
        );
        assert_eq!(
            changed_task.identity().sha256,
            missing_config.identity().sha256
        );
    }

    #[cfg(unix)]
    fn assert_repository_identity_tracks_symbolic_link_targets(dir: &tempfile::TempDir) {
        use std::os::unix::fs::symlink;

        let observe = || {
            test_context(
                dir,
                Trust::RepositoryRead,
                Options::default(),
                Verdict::default(),
            )
        };
        let link = dir.path().join("untracked-link");
        symlink("missing-target-one", &link).expect("first link target");
        let first = observe();
        std::fs::remove_file(&link).expect("replace link target");
        symlink("missing-target-two", &link).expect("second link target");
        let second = observe();
        let RepositoryIdentity::Observed {
            status_sha256: first_status,
            untracked: 1,
            ..
        } = &first.identity().repository
        else {
            panic!("the first symbolic link must be identified")
        };
        let RepositoryIdentity::Observed {
            status_sha256: second_status,
            untracked: 1,
            ..
        } = &second.identity().repository
        else {
            panic!("the second symbolic link must be identified")
        };
        assert_ne!(
            first_status, second_status,
            "unchanged status bits must not hide a changed symbolic-link target"
        );
    }

    /// Proof for `tui.external_refresh_is_identity_safe`: repository-reading
    /// contexts identify HEAD and the exact changed work-tree material, while
    /// the separate plan digest moves only with plan inputs.
    #[test]
    fn repository_identity_distinguishes_head_and_dirty_path_transitions() {
        let dir = fixture();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "fixture"]);

        let observe = || {
            test_context(
                &dir,
                Trust::RepositoryRead,
                Options::default(),
                Verdict::default(),
            )
        };
        let clean = observe();
        let aplan_app_identity = clean.identity();
        let RepositoryIdentity::Observed {
            head: Some(clean_head),
            modified: 0,
            untracked: 0,
            status_sha256: clean_status,
        } = &aplan_app_identity.repository
        else {
            panic!("a committed fixture must have an observed clean repository")
        };
        assert_eq!(clean_head.len(), 40);
        assert_eq!(clean_status.len(), 64);

        std::fs::write(dir.path().join("first-untracked"), "one\n").expect("untracked file");
        let first_untracked = observe();
        assert_eq!(
            clean.identity().plan_sha256,
            first_untracked.identity().plan_sha256
        );
        assert_ne!(clean.identity().sha256, first_untracked.identity().sha256);
        let RepositoryIdentity::Observed {
            modified: 0,
            untracked: 1,
            status_sha256: first_status,
            ..
        } = &first_untracked.identity().repository
        else {
            panic!("the untracked path must be represented")
        };

        std::fs::write(dir.path().join("first-untracked"), "changed bytes\n")
            .expect("change the same dirty path");
        let same_path_changed = observe();
        let RepositoryIdentity::Observed {
            status_sha256: same_path_changed_status,
            untracked: 1,
            ..
        } = &same_path_changed.identity().repository
        else {
            panic!("the changed untracked path must remain represented")
        };
        assert_ne!(
            first_status, same_path_changed_status,
            "unchanged status bits must not hide changed source bytes"
        );

        std::fs::remove_file(dir.path().join("first-untracked")).expect("remove first path");
        std::fs::write(dir.path().join("second-untracked"), "two\n").expect("second path");
        let second_untracked = observe();
        let RepositoryIdentity::Observed {
            status_sha256: second_status,
            untracked: 1,
            ..
        } = &second_untracked.identity().repository
        else {
            panic!("the replacement untracked path must be represented")
        };
        assert_ne!(
            first_status, second_status,
            "equal counts must not erase which path is dirty"
        );

        git(&dir, &["add", "second-untracked"]);
        let staged = observe();
        let RepositoryIdentity::Observed {
            modified: 1,
            untracked: 0,
            ..
        } = &staged.identity().repository
        else {
            panic!("a staged new file is a tracked change, not untracked")
        };
        assert_ne!(second_untracked.identity().sha256, staged.identity().sha256);

        git(&dir, &["commit", "-qm", "advance head"]);
        let advanced = observe();
        let RepositoryIdentity::Observed {
            head: Some(advanced_head),
            modified: 0,
            untracked: 0,
            ..
        } = &advanced.identity().repository
        else {
            panic!("the committed transition must return to clean")
        };
        assert_ne!(clean_head, advanced_head);
        assert_ne!(staged.identity().sha256, advanced.identity().sha256);

        #[cfg(unix)]
        assert_repository_identity_tracks_symbolic_link_targets(&dir);
    }

    /// Proof for `tui.external_refresh_is_identity_safe`: discovering a Git
    /// repository is not enough to claim an observed identity when that
    /// repository cannot supply work-tree status.
    #[test]
    fn repository_identity_fails_closed_when_status_is_unavailable() {
        let dir = fixture();
        git(&dir, &["init", "--bare", "bare.git"]);
        let plan_root = dir.path().join("aplan");
        let context = PlanContext::new(
            aplan_core::load_plan_identified(&plan_root),
            ContextInputs {
                repository_root: dir.path().join("bare.git"),
                configuration_file: None,
                trust: Trust::RepositoryRead,
                options: Options::default(),
                verdict: Verdict::default(),
            },
        );

        let RepositoryIdentity::Unavailable { reason } = &context.identity().repository else {
            panic!("a bare repository must not impersonate an observed work tree")
        };
        assert!(
            reason.contains("workdir") || reason.contains("bare"),
            "{reason}"
        );
        assert_eq!(context.identity().sha256.len(), 64);
    }

    #[test]
    fn report_configuration_participates_in_identity() {
        let dir = fixture();
        let context = || {
            PlanContext::new(
                aplan_core::load_plan_identified(&dir.path().join("aplan")),
                ContextInputs {
                    repository_root: dir.path().to_path_buf(),
                    configuration_file: None,
                    trust: Trust::Hermetic,
                    options: Options::default(),
                    verdict: Verdict::default(),
                },
            )
        };
        let baseline = context();
        let configured = context().with_report_config(aplan_config::Config {
            stale_after_days: Some(17),
            ..aplan_config::Config::default()
        });
        assert_ne!(
            baseline.identity().sha256,
            configured.identity().sha256,
            "report-only policy must participate in snapshot identity"
        );
        assert_eq!(
            configured
                .report_config()
                .and_then(|config| config.stale_after_days),
            Some(17)
        );
        let refreshed = aplan_core::load_plan_identified(&dir.path().join("aplan"));
        assert_eq!(
            configured.identity_for(&refreshed).sha256,
            configured.identity().sha256,
            "refresh identity must retain the resolved report configuration"
        );
        let different_report_config = context().with_report_config(aplan_config::Config {
            stale_after_days: Some(18),
            ..aplan_config::Config::default()
        });
        assert_ne!(
            configured.identity().sha256,
            different_report_config.identity().sha256
        );
    }

    #[test]
    fn captured_report_configuration_refuses_another_file_revision() {
        let dir = fixture();
        let path = dir.path().join("selected.yml");
        std::fs::write(&path, "stale_after_days: 17\n").expect("selected config");
        let context = PlanContext::new(
            aplan_core::load_plan_identified(&dir.path().join("aplan")),
            ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(path.clone()),
                trust: Trust::RepositoryRead,
                options: Options::default(),
                verdict: Verdict::default(),
            },
        );
        let error = context
            .with_captured_report_config(aplan_config::Config::default())
            .expect_err("resolved configuration from another revision");
        assert!(matches!(error, ConfigurationCaptureError::Changed { .. }));

        let (config, _) =
            aplan_config::load_from(&aplan_config::ConfigSource::File(path.clone()), dir.path())
                .expect("load exact config");
        let captured = PlanContext::new(
            aplan_core::load_plan_identified(&dir.path().join("aplan")),
            ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(path.clone()),
                trust: Trust::RepositoryRead,
                options: Options::default(),
                verdict: Verdict::default(),
            },
        )
        .with_captured_report_config(config)
        .expect("capture exact revision");
        assert!(matches!(
            captured.identity().configuration.content,
            ConfigurationContentIdentity::Observed { .. }
        ));

        let semantic_only = PlanContext::new(
            aplan_core::load_plan_identified(&dir.path().join("aplan")),
            ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(path),
                trust: Trust::RepositoryRead,
                options: Options::default(),
                verdict: Verdict::default(),
            },
        )
        .with_report_config(
            captured
                .report_config()
                .expect("captured report config")
                .clone(),
        );
        let refreshed = aplan_core::load_plan_identified(&dir.path().join("aplan"));
        let actual = semantic_only.identity_for(&refreshed);
        assert!(matches!(
            semantic_only.identity().configuration.content,
            ConfigurationContentIdentity::NotObserved
        ));
        assert!(matches!(
            actual.configuration.content,
            ConfigurationContentIdentity::Observed { .. }
        ));
        assert_ne!(
            semantic_only.identity().sha256,
            actual.sha256,
            "semantic-only config attachment must fail a publication comparison"
        );
    }

    #[test]
    fn captured_report_configuration_fails_closed_across_io_and_parse_boundaries() {
        let dir = fixture();
        let path = dir.path().join("selected.yml");
        let context = || {
            PlanContext::new(
                aplan_core::load_plan_identified(&dir.path().join("aplan")),
                ContextInputs {
                    repository_root: dir.path().to_path_buf(),
                    configuration_file: Some(path.clone()),
                    trust: Trust::Hermetic,
                    options: Options::default(),
                    verdict: Verdict::default(),
                },
            )
        };

        let missing = context()
            .with_captured_report_config(aplan_config::Config::default())
            .expect_err("a missing selected file cannot be captured");
        assert!(matches!(missing, ConfigurationCaptureError::Read { .. }));

        std::fs::write(&path, "unknown_key: true\n").expect("invalid config");
        let invalid = context()
            .with_captured_report_config(aplan_config::Config::default())
            .expect_err("invalid selected bytes cannot be captured");
        assert!(matches!(invalid, ConfigurationCaptureError::Changed { .. }));

        std::fs::write(&path, "deny_warnings: false\n").expect("valid config");
        let captured = context()
            .with_captured_report_config(aplan_config::Config {
                deny_warnings: Some(false),
                ..aplan_config::Config::default()
            })
            .expect("capture selected bytes");
        let loaded = aplan_core::load_plan_identified(&dir.path().join("aplan"));

        std::fs::write(&path, "unknown_key: true\n").expect("invalid edit");
        let invalid_actual = captured.identity_for(&loaded);
        assert!(matches!(
            invalid_actual.configuration.content,
            ConfigurationContentIdentity::Unavailable { ref reason }
                if reason.contains("unknown key")
        ));
        assert_ne!(captured.identity().sha256, invalid_actual.sha256);

        std::fs::remove_file(&path).expect("remove selected config");
        let missing_reason = std::fs::read_to_string(&path)
            .expect_err("selected config remains absent")
            .to_string();
        let missing_actual = captured.identity_for(&loaded);
        assert!(matches!(
            missing_actual.configuration.content,
            ConfigurationContentIdentity::Unavailable { ref reason }
                if reason == &missing_reason
        ));
        assert_ne!(captured.identity().sha256, missing_actual.sha256);
        assert_ne!(invalid_actual.sha256, missing_actual.sha256);
    }

    fn graph_fixture_workspace() -> GraphWorkspace {
        let dir = fixture();
        let status = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(dir.path())
            .status()
            .expect("git init");
        assert!(status.success());
        std::fs::write(
            dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\nowner: kim\neffort_days: 1\ntags: [alpha]\n---\n",
        )
        .expect("first");
        std::fs::write(
            dir.path().join("aplan/tasks/A-2.md"),
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 2\nblocked_by: [A-1]\ntags: [beta]\n---\n",
        )
        .expect("second");
        std::fs::write(
            dir.path().join("aplan/tasks/A-3.md"),
            "---\nid: A-3\ntitle: third\nphase: 2\nstatus: todo\neffort_days: 3\nblocked_by: [A-2]\n---\n",
        )
        .expect("third");
        std::fs::write(
            dir.path().join("aplan/tasks/M-v1.md"),
            "---\nid: M-v1\ntitle: release\nphase: 1\nstatus: todo\nblocked_by: [A-1, A-2, A-3]\ntags: [milestone, v1.0]\n---\n",
        )
        .expect("milestone");
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        graph_workspace(&context, task_catalogue(&context))
    }

    fn graph_explorer_fixture_workspace() -> GraphWorkspace {
        let dir = fixture();
        let status = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(dir.path())
            .status()
            .expect("git init");
        assert!(status.success());
        let tasks = [
            (
                "A-1.md",
                "---\nid: A-1\ntitle: active first-cut work\nphase: 1\nstatus: in_progress\neffort_days: 1\n---\n",
            ),
            (
                "A-2.md",
                "---\nid: A-2\ntitle: first-cut join\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [B-1]\n---\n",
            ),
            (
                "B-1.md",
                "---\nid: B-1\ntitle: second-cut source\nphase: 2\nstatus: todo\neffort_days: 1\n---\n",
            ),
            (
                "B-2.md",
                "---\nid: B-2\ntitle: second-cut join\nphase: 2\nstatus: todo\neffort_days: 1\nblocked_by: [A-1]\n---\n",
            ),
            (
                "X-1.md",
                "---\nid: X-1\ntitle: ungrouped cross-phase work\nphase: 3\nstatus: todo\neffort_days: 1\nblocked_by: [A-1]\ntags: [cross-cutting]\n---\n",
            ),
            (
                "M-1.md",
                "---\nid: M-1\ntitle: first cut\nphase: 1\nstatus: todo\nblocked_by: [A-1, A-2]\ntags: [milestone, v1]\n---\n",
            ),
            (
                "M-2.md",
                "---\nid: M-2\ntitle: second cut\nphase: 2\nstatus: todo\nblocked_by: [B-1, B-2]\ntags: [milestone, v2]\n---\n",
            ),
            (
                "M-empty.md",
                "---\nid: M-empty\ntitle: empty checkpoint\nphase: 4\nstatus: todo\ntags: [milestone, empty]\n---\n",
            ),
        ];
        for (name, contents) in tasks {
            std::fs::write(dir.path().join("aplan/tasks").join(name), contents).expect(name);
        }
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        graph_workspace(&context, task_catalogue(&context))
    }

    fn oversized_aggregate_explorer_workspace() -> GraphWorkspace {
        let dir = fixture();
        for index in 0usize..20 {
            let task = format!("T-{index:02}");
            let milestone = format!("M-{index:02}");
            let blocked_by = index
                .checked_sub(1)
                .map_or_else(String::new, |prior| format!("blocked_by: [T-{prior:02}]\n"));
            std::fs::write(
                dir.path().join("aplan/tasks").join(format!("{task}.md")),
                format!(
                    "---\nid: {task}\ntitle: work {index}\nphase: 1\nstatus: todo\neffort_days: 1\n{blocked_by}---\n"
                ),
            )
            .expect("aggregate member");
            std::fs::write(
                dir.path()
                    .join("aplan/tasks")
                    .join(format!("{milestone}.md")),
                format!(
                    "---\nid: {milestone}\ntitle: cut {index}\nphase: 1\nstatus: todo\nblocked_by: [{task}]\ntags: [milestone]\n---\n"
                ),
            )
            .expect("aggregate milestone");
        }
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        graph_workspace(&context, task_catalogue(&context))
    }

    /// Proof for `tui.graph_explorer_uses_semantic_scene`: every structural
    /// scope is an application projection of core reach/tree semantics and the
    /// canonical `GraphView`, with one membership policy.
    #[test]
    fn graph_workspace_composes_every_scope_selector_and_membership_policy() {
        let workspace = graph_fixture_workspace();

        for reach in [
            GraphScope::Upstream("A-3".to_string()),
            GraphScope::Downstream("A-1".to_string()),
            GraphScope::Between("A-1".to_string(), "A-3".to_string()),
        ] {
            assert_eq!(reach.walked_edges(false), Some(Edges::Dependency));
            assert_eq!(reach.walked_edges(true), Some(Edges::Any));
        }
        assert_eq!(
            GraphScope::Unreachable.walked_edges(false),
            Some(Edges::Membership)
        );
        assert_eq!(
            GraphScope::Neighbourhood {
                task: "A-2".to_string(),
                depth: Some(1),
                live: false,
            }
            .walked_edges(false),
            Some(Edges::Any)
        );
        assert_eq!(GraphScope::Whole.walked_edges(false), None);
        assert_eq!(GraphScope::CriticalPath.walked_edges(true), None);

        let dependencies = workspace.project(&GraphRequest::default());
        assert_eq!(
            dependencies
                .view
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["A-1", "A-2", "A-3"]
        );
        assert_eq!(dependencies.view.edges.len(), 2);
        assert_eq!(dependencies.view.grouping.edges, 3);
        assert_eq!(dependencies.view.grouping.tasks, 1);
        assert_eq!(
            dependencies.exclusion("M-v1"),
            Some("has only cut-membership relationships in this view; enable membership")
        );

        let membership = workspace.project(&GraphRequest {
            membership: true,
            ..GraphRequest::default()
        });
        assert_eq!(membership.view.nodes.len(), 4);
        assert_eq!(membership.view.edges.len(), 5);
        assert_eq!(membership.view.grouping, aplan_output::Grouping::default());

        for scope in [
            GraphScope::Upstream("A-3".to_string()),
            GraphScope::Downstream("A-1".to_string()),
            GraphScope::Between("A-1".to_string(), "A-3".to_string()),
            GraphScope::CriticalPath,
        ] {
            let projection = workspace.project(&GraphRequest {
                scope,
                ..GraphRequest::default()
            });
            assert_eq!(
                projection.admitted,
                BTreeSet::from(["A-1".to_string(), "A-2".to_string(), "A-3".to_string(),])
            );
            assert_eq!(projection.state, GraphProjectionState::Complete);
        }

        let neighbourhood = workspace.project(&GraphRequest {
            scope: GraphScope::Neighbourhood {
                task: "A-2".to_string(),
                depth: Some(1),
                live: false,
            },
            membership: true,
            ..GraphRequest::default()
        });
        assert_eq!(
            neighbourhood.admitted,
            BTreeSet::from([
                "A-1".to_string(),
                "A-2".to_string(),
                "A-3".to_string(),
                "M-v1".to_string(),
            ])
        );
        assert!(
            neighbourhood
                .view
                .scope
                .iter()
                .any(|scope| scope.contains("every `blocked_by` edge"))
        );

        let critical = workspace.project(&GraphRequest {
            scope: GraphScope::CriticalPath,
            ..GraphRequest::default()
        });
        assert!(
            critical
                .view
                .scope
                .iter()
                .any(|scope| scope.contains("critical-path dependencies and containment"))
        );
    }

    /// Proof for `tui.graph_explorer_uses_semantic_scene`: every shared
    /// selector composes with the graph, while exclusions, exceptional scopes
    /// and direct relationship roles retain typed answers.
    #[test]
    fn graph_workspace_keeps_selectors_exclusions_and_failures_typed() {
        let workspace = graph_fixture_workspace();
        let dependencies = workspace.project(&GraphRequest::default());

        let filtered = workspace.project(&GraphRequest {
            selector: Selector {
                owners: vec!["kim".to_string()],
                ..Selector::default()
            },
            ..GraphRequest::default()
        });
        assert_eq!(filtered.state, GraphProjectionState::NoEdges);
        assert!(filtered.contains("A-1"));
        assert_eq!(
            filtered.exclusion("A-2"),
            Some("excluded by the active task filters")
        );

        for selector in [
            Selector {
                cuts: vec!["M-v1".to_string()],
                ..Selector::default()
            },
            Selector {
                tags: vec!["beta".to_string()],
                ..Selector::default()
            },
            Selector {
                phases: vec![1],
                ..Selector::default()
            },
            Selector {
                statuses: vec![Status::Todo],
                ..Selector::default()
            },
            Selector {
                unclaimed: true,
                ..Selector::default()
            },
        ] {
            let projection = workspace.project(&GraphRequest {
                selector,
                membership: true,
                ..GraphRequest::default()
            });
            assert!(
                !projection.admitted.is_empty(),
                "each shared selector dimension must compose with graph scope"
            );
        }

        let unreachable = workspace.project(&GraphRequest {
            scope: GraphScope::Unreachable,
            membership: true,
            ..GraphRequest::default()
        });
        assert_eq!(unreachable.state, GraphProjectionState::EmptyScope);
        assert!(
            unreachable
                .view
                .scope
                .iter()
                .any(|scope| scope.contains("cut-membership edges"))
        );

        let unknown = workspace.project(&GraphRequest {
            scope: GraphScope::Between("A-1".to_string(), "missing".to_string()),
            ..GraphRequest::default()
        });
        assert_eq!(
            unknown.state,
            GraphProjectionState::UnknownTask("missing".to_string())
        );
        assert!(unknown.view.nodes.is_empty());

        assert_eq!(
            dependencies.blockers("A-2"),
            [GraphRelation {
                id: "A-1".to_string(),
                kind: EdgeKind::Dependency,
            }]
        );
        assert_eq!(
            dependencies.dependents("A-2"),
            [GraphRelation {
                id: "A-3".to_string(),
                kind: EdgeKind::Dependency,
            }]
        );
    }

    #[test]
    fn graph_workspace_applies_semantic_scope_before_bounded_windowing() {
        let workspace = graph_fixture_workspace();
        let request = GraphRequest {
            scope: GraphScope::Upstream("A-3".to_string()),
            ..GraphRequest::default()
        };
        let complete = workspace.project(&request);
        let bounded = workspace.project_window(
            &request,
            &GraphWindowRequest {
                preferred_anchor: Some("A-2".to_string()),
                budget: GraphWindowBudget {
                    nodes: 2,
                    edges: 1,
                    lanes: 1,
                },
                continuation: None,
            },
        );
        assert_eq!(bounded.input, complete.input);
        assert_eq!(bounded.source_state, complete.state);
        assert_eq!(bounded.source_nodes, complete.view.nodes.len());
        assert_eq!(bounded.source_edges, complete.view.edges.len());
        assert_eq!(bounded.view.scope, complete.view.scope);
        assert_eq!(bounded.anchor.expect("anchor").selected, "A-2");
        assert_eq!(bounded.state, GraphWindowState::Windowed);
    }

    /// Proof for `tui.graph_explorer_projects_multiscale_semantics`: a plan,
    /// phase, milestone and task projection all retain canonical identities and
    /// every aggregate edge can be expanded back to its exact task edges.
    #[test]
    fn graph_explorer_projects_multiscale_semantics() {
        let workspace = graph_explorer_fixture_workspace();
        let request = GraphExplorerRequest::default();
        let projection = workspace.project_explorer(&request);

        assert_eq!(projection.state, GraphExplorerState::Complete);
        assert_eq!(projection.graph.phase_bands.len(), 4);
        assert_eq!(projection.graph.nodes.len(), 4);
        assert_eq!(
            projection
                .graph
                .nodes
                .iter()
                .map(|node| node.identity.clone())
                .collect::<Vec<_>>(),
            [
                GraphExplorerIdentity::Milestone("M-1".to_string()),
                GraphExplorerIdentity::Milestone("M-2".to_string()),
                GraphExplorerIdentity::Milestone("M-empty".to_string()),
                GraphExplorerIdentity::Task("X-1".to_string()),
            ]
        );
        let represented: BTreeSet<String> = projection
            .graph
            .nodes
            .iter()
            .flat_map(|node| node.represented_ids.iter().cloned())
            .collect();
        assert_eq!(
            represented,
            projection
                .accounting
                .admitted_task_ids
                .into_iter()
                .collect()
        );
        assert_eq!(projection.accounting.admitted_dependency_edges, 3);
        assert_eq!(projection.accounting.represented_dependency_edges, 3);

        let feedback: Vec<&GraphExplorerEdge> = projection
            .graph
            .edges
            .iter()
            .filter(|edge| edge.disposition == GraphExplorerEdgeDisposition::AggregateFeedback)
            .collect();
        assert_eq!(feedback.len(), 2);
        assert_eq!(feedback[0].dependency_count, 1);
        assert_eq!(feedback[0].provenance.len(), 1);
        assert!(projection.conditions.iter().any(|condition| matches!(
            condition,
            GraphExplorerCondition::AggregateFeedback { components }
                if components == &vec![vec![
                    GraphExplorerIdentity::Milestone("M-1".to_string()),
                    GraphExplorerIdentity::Milestone("M-2".to_string()),
                ]]
        )));
        assert!(
            !projection.conditions.iter().any(|condition| matches!(
                condition,
                GraphExplorerCondition::InvalidTaskCycle { .. }
            ))
        );
        assert!(projection.conditions.iter().any(|condition| matches!(
            condition,
            GraphExplorerCondition::TasksOutsideMilestones { ids }
                if ids == &["X-1".to_string()]
        )));
        assert_eq!(
            projection.selection,
            Some(GraphExplorerSelection {
                subject: GraphExplorerIdentity::Task("A-1".to_string()),
                represented_by: GraphExplorerIdentity::Milestone("M-1".to_string()),
                reason: GraphExplorerAnchorReason::InProgress,
            })
        );

        let all_provenance: BTreeSet<GraphExplorerDependency> = projection
            .graph
            .edges
            .iter()
            .flat_map(|edge| edge.provenance.iter().cloned())
            .chain(
                projection
                    .graph
                    .nodes
                    .iter()
                    .flat_map(|node| node.internal_dependencies.iter().cloned()),
            )
            .collect();
        assert_eq!(all_provenance.len(), 3);

        let mut tasks = workspace.tasks.clone();
        tasks.reverse();
        let mut catalogue = workspace.catalogue.clone();
        catalogue.reverse();
        let reversed = GraphWorkspace::new(workspace.input.clone(), tasks, catalogue)
            .project_explorer(&request);
        assert_eq!(
            serde_json::to_value(&projection.graph).expect("graph JSON"),
            serde_json::to_value(&reversed.graph).expect("reversed graph JSON"),
            "input enumeration cannot change the typed semantic graph"
        );
    }

    #[test]
    fn graph_explorer_accounting_retains_grouping_and_reach_scope() {
        let workspace = graph_explorer_fixture_workspace();
        let complete = workspace.project_explorer(&GraphExplorerRequest::default());
        assert_eq!(complete.accounting.grouping.edges, 4);
        assert_eq!(complete.accounting.grouping.tasks, 2);
        assert!(complete.accounting.source_scope.is_empty());

        let reached = workspace.project_explorer(&GraphExplorerRequest {
            graph: GraphRequest {
                scope: GraphScope::Downstream("A-1".to_string()),
                ..GraphRequest::default()
            },
            ..GraphExplorerRequest::default()
        });
        assert!(
            reached
                .accounting
                .source_scope
                .iter()
                .any(|scope| scope.contains("dependency edges")),
            "the multiscale projection lost which graph reachability walked"
        );
    }

    /// Proof for `graph.aggregate_levels_are_bounded_and_continuable`: plan
    /// and phase aggregate levels apply the semantic budget before layout,
    /// account omitted raw tasks and retain source-bound continuations.
    #[test]
    fn aggregate_graph_levels_are_bounded_before_cell_layout() {
        let workspace = oversized_aggregate_explorer_workspace();
        let default_plan = workspace.project_explorer(&GraphExplorerRequest::default());
        assert_eq!(default_plan.graph.nodes.len(), 10);
        assert!(default_plan.graph.edges.len() <= 20);
        assert_eq!(
            default_plan
                .semantic_window
                .as_ref()
                .map(|window| (window.budget.nodes, window.budget.edges)),
            Some((10, 20))
        );

        let request = GraphExplorerRequest {
            selected: Some(GraphExplorerIdentity::Task("T-10".to_string())),
            selection_origin: GraphExplorerSelectionOrigin::Contextual,
            budget: GraphWindowBudget {
                nodes: 4,
                edges: 4,
                lanes: 1,
            },
            ..GraphExplorerRequest::default()
        };
        let plan = workspace.project_explorer(&request);
        assert_eq!(plan.graph.nodes.len(), 4);
        assert!(plan.graph.edges.len() <= 4);
        assert_eq!(
            plan.graph.edges.len(),
            plan.semantic_window
                .as_ref()
                .expect("bounded plan window")
                .view
                .edges
                .len()
        );
        assert_eq!(plan.accounting.admitted_task_ids.len(), 42);
        assert!(plan.accounting.visible_task_ids.len() < 42);
        assert!(!plan.accounting.window_omitted_task_ids.is_empty());
        assert!(!plan.boundaries.is_empty());
        assert_eq!(
            plan.semantic_window
                .as_ref()
                .map(|window| window.source_nodes),
            Some(22)
        );
        assert!(
            plan.graph.nodes.iter().any(|node| {
                node.identity == GraphExplorerIdentity::Milestone("M-10".to_string())
            })
        );
        assert_eq!(
            plan.selection.as_ref().map(|selection| &selection.subject),
            Some(&GraphExplorerIdentity::Task("T-10".to_string()))
        );
        assert_eq!(
            plan.accounting.represented_dependency_edges,
            plan.graph
                .edges
                .iter()
                .map(|edge| edge.dependency_count)
                .sum::<usize>()
                + plan
                    .graph
                    .nodes
                    .iter()
                    .map(|node| node.internal_dependencies.len())
                    .sum::<usize>()
        );

        let cursor = plan.boundaries[0].cursor.clone();
        let continued = workspace.project_explorer(&GraphExplorerRequest {
            continuation: Some(cursor.clone()),
            budget: request.budget,
            ..GraphExplorerRequest::default()
        });
        let continued_window = continued
            .semantic_window
            .as_ref()
            .expect("continued window");
        assert_eq!(continued_window.input.sha256, cursor.input_sha256);
        assert!(
            continued_window
                .view
                .nodes
                .iter()
                .any(|node| node.id == cursor.key.boundary)
        );
        assert!(continued.selection.as_ref().is_some_and(|selection| {
            continued
                .graph
                .nodes
                .iter()
                .any(|node| node.identity == selection.represented_by)
        }));

        let phase = workspace.project_explorer(&GraphExplorerRequest {
            location: GraphExplorerLocation::Phase(1),
            budget: request.budget,
            ..GraphExplorerRequest::default()
        });
        assert_eq!(phase.graph.nodes.len(), 4);
        assert!(!phase.boundaries.is_empty());
        assert_eq!(phase.accounting.level_task_ids.len(), 42);
        assert!(phase.accounting.visible_task_ids.len() < 42);
    }

    fn assert_small_dag_projection(
        mask: u8,
        projection: &GraphExplorerProjection,
        expected: &[GraphExplorerDependency],
    ) {
        assert_eq!(
            projection.state,
            GraphExplorerState::Complete,
            "mask {mask}"
        );
        assert_eq!(
            projection.accounting.admitted_dependency_edges,
            expected.len(),
            "mask {mask}"
        );
        assert_eq!(
            projection.accounting.represented_dependency_edges,
            expected.len(),
            "mask {mask}"
        );

        let mut represented = BTreeMap::<String, usize>::new();
        for id in projection
            .graph
            .nodes
            .iter()
            .flat_map(|node| node.represented_ids.iter())
        {
            *represented.entry(id.clone()).or_default() += 1;
        }
        assert_eq!(
            represented,
            projection
                .accounting
                .admitted_task_ids
                .iter()
                .cloned()
                .map(|id| (id, 1usize))
                .collect(),
            "every admitted identity must be represented exactly once for mask {mask}"
        );

        let visible: BTreeSet<_> = projection
            .graph
            .nodes
            .iter()
            .map(|node| node.identity.clone())
            .collect();
        let mut actual: Vec<_> = projection
            .graph
            .edges
            .iter()
            .flat_map(|edge| {
                assert!(visible.contains(&edge.from), "mask {mask}");
                assert!(visible.contains(&edge.to), "mask {mask}");
                assert_eq!(edge.dependency_count, edge.provenance.len(), "mask {mask}");
                edge.provenance.iter().cloned()
            })
            .chain(
                projection
                    .graph
                    .nodes
                    .iter()
                    .flat_map(|node| node.internal_dependencies.iter().cloned()),
            )
            .collect();
        actual.sort();
        assert_eq!(actual, expected, "mask {mask}");
        assert!(projection.conditions.iter().all(|condition| !matches!(
            condition,
            GraphExplorerCondition::InvalidTaskCycle { .. }
        )));
    }

    #[test]
    fn graph_explorer_exhaustively_partitions_small_dags_without_losing_provenance() {
        let seed = graph_explorer_fixture_workspace();
        let work_ids = ["A-1", "A-2", "B-1", "B-2"];
        let milestone_ids = ["M-1", "M-2"];
        let pairs = [(0usize, 1usize), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];

        for mask in 0u8..(1 << pairs.len()) {
            let mut tasks: Vec<Task> = work_ids
                .iter()
                .chain(milestone_ids.iter())
                .map(|id| {
                    seed.tasks
                        .iter()
                        .find(|task| task.id == *id)
                        .expect("fixture task")
                        .clone()
                })
                .collect();
            for task in &mut tasks {
                task.blocked_by.clear();
            }
            let mut expected = Vec::new();
            for (bit, &(from, to)) in pairs.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    tasks[to].blocked_by.push(work_ids[from].to_string());
                    expected.push(GraphExplorerDependency {
                        from: work_ids[from].to_string(),
                        to: work_ids[to].to_string(),
                    });
                }
            }
            expected.sort();

            let catalogue: Vec<TaskBrowserRow> = work_ids
                .iter()
                .chain(milestone_ids.iter())
                .map(|id| {
                    seed.catalogue
                        .iter()
                        .find(|row| row.id == *id)
                        .expect("fixture catalogue row")
                        .clone()
                })
                .collect();
            let projection =
                GraphWorkspace::new(seed.input.clone(), tasks.clone(), catalogue.clone())
                    .project_explorer(&GraphExplorerRequest::default());
            assert_small_dag_projection(mask, &projection, &expected);

            tasks.reverse();
            let mut reversed_catalogue = catalogue;
            reversed_catalogue.reverse();
            let reversed = GraphWorkspace::new(seed.input.clone(), tasks, reversed_catalogue)
                .project_explorer(&GraphExplorerRequest::default());
            assert_eq!(
                serde_json::to_value(&projection.graph).expect("graph JSON"),
                serde_json::to_value(&reversed.graph).expect("reversed graph JSON"),
                "enumeration order changed graph for mask {mask}"
            );
        }
    }

    /// Proof for `tui.layered_cell_scene_is_bounded_and_deterministic`: the
    /// semantic graph becomes complete integer-cell geometry without losing
    /// phase, node or dependency identity, and identical work is byte stable.
    #[test]
    fn layered_cell_scene_is_bounded_deterministic_and_kernel_independent() {
        let projection =
            graph_explorer_fixture_workspace().project_explorer(&GraphExplorerRequest::default());
        let mut request = CellSceneRequest::new(projection.clone());
        request.orientation = CellSceneOrientation::LeftToRight;
        let first = CellSceneJob::new(17, request.clone()).run();
        let second = CellSceneJob::new(17, request).run();
        assert_eq!(
            serde_json::to_vec(&first).expect("first scene JSON"),
            serde_json::to_vec(&second).expect("second scene JSON"),
            "identical semantic input must produce byte-identical cell geometry"
        );

        let CellSceneResult::Ready(scene) = first.result else {
            panic!("the bounded fixture must produce a scene")
        };
        assert_eq!(scene.direction, CellSceneDirection::LeftToRight);
        assert_eq!(scene.nodes.len(), projection.graph.nodes.len());
        assert_eq!(scene.edges.len(), projection.graph.edges.len());
        assert_eq!(scene.bands.len(), projection.graph.phase_bands.len());
        assert!(scene.bounds.width > 0 && scene.bounds.height > 0);
        assert!(scene.nodes.iter().all(|node| {
            node.bounds.x >= 0
                && node.bounds.y >= 0
                && node.bounds.x + i32::from(node.bounds.width) <= i32::from(scene.bounds.width)
                && node.bounds.y + i32::from(node.bounds.height) <= i32::from(scene.bounds.height)
        }));
        assert!(scene.edges.iter().all(|edge| {
            !edge.identity.provenance.is_empty()
                && edge.arrowhead == edge.points.last().copied()
                && edge.source_port.is_some()
                && edge.target_port.is_some()
                && edge.degradations.is_empty()
        }));
        assert!(scene.bands.iter().any(|band| {
            band.phase == 4
                && band.nodes == [GraphExplorerIdentity::Milestone("M-empty".to_string())]
                && band.bounds.is_some()
                && band.label == "Phase 4"
                && band.label_anchor.is_some()
                && band.degradation.is_none()
        }));
        for band in scene.bands.iter().filter(|band| !band.nodes.is_empty()) {
            let bounds = band.bounds.expect("non-empty phase bounds");
            let label = band.label_anchor.expect("non-empty phase label");
            assert!(
                label.x >= bounds.x
                    && label.x <= bounds.x + i32::from(bounds.width)
                    && label.y >= bounds.y
                    && label.y <= bounds.y + i32::from(bounds.height)
            );
            for identity in &band.nodes {
                let node = scene
                    .nodes
                    .iter()
                    .find(|node| &node.identity == identity)
                    .expect("band node");
                assert!(node.bounds.x >= bounds.x && node.bounds.y >= bounds.y);
                assert!(
                    node.bounds.x + i32::from(node.bounds.width)
                        <= bounds.x + i32::from(bounds.width)
                );
                assert!(
                    node.bounds.y + i32::from(node.bounds.height)
                        <= bounds.y + i32::from(bounds.height)
                );
            }
        }
        assert!(
            scene
                .edges
                .iter()
                .any(|edge| edge.feedback == CellFeedbackKind::AggregateQuotient)
        );
        assert_eq!(scene.metrics.node_overlaps, 0);
        assert_eq!(scene.metrics.node_edge_hits, 0);
        assert_eq!(scene.metrics.endpoint_errors, 0);
    }

    #[test]
    fn auto_cell_scene_scores_both_directions_deterministically() {
        let projection =
            graph_explorer_fixture_workspace().project_explorer(&GraphExplorerRequest::default());
        let request = CellSceneRequest::new(projection);
        let first = CellSceneJob::new(8, request.clone()).run();
        let second = CellSceneJob::new(8, request).run();
        assert_eq!(first, second);
        let CellSceneResult::Ready(scene) = first.result else {
            panic!("auto scene")
        };
        assert_eq!(scene.ordering_policy, CellPeerOrder::Readability);
        assert!(matches!(
            scene.direction,
            CellSceneDirection::LeftToRight | CellSceneDirection::TopToBottom
        ));
        assert!(scene.metrics.quality_score >= scene.metrics.area_cells);
    }

    /// Proof for `graph.layered_scene_is_complete_or_fallback`: status reuse,
    /// orientation changes and retained failure geometry keep distinct scene
    /// contracts, including valid top-to-bottom phase containment.
    #[test]
    fn cell_scene_reuses_status_geometry_recentres_orientation_and_retains_fallback() {
        let projection =
            graph_explorer_fixture_workspace().project_explorer(&GraphExplorerRequest::default());
        let mut initial_request = CellSceneRequest::new(projection.clone());
        initial_request.orientation = CellSceneOrientation::LeftToRight;
        let CellSceneResult::Ready(initial) = CellSceneJob::new(1, initial_request).run().result
        else {
            panic!("initial scene")
        };

        let mut status_projection = projection.clone();
        status_projection.graph.nodes[0].status = Status::Done;
        status_projection.input.sha256 = "status-only-refresh".to_string();
        let mut status_request = CellSceneRequest::new(status_projection);
        status_request.orientation = CellSceneOrientation::LeftToRight;
        status_request.prior = Some(initial.clone());
        let CellSceneResult::Ready(reused) = CellSceneJob::new(2, status_request).run().result
        else {
            panic!("status-only scene")
        };
        assert_eq!(reused.movement, CellSceneMovement::ReusedStatusOnly);
        assert_eq!(reused.nodes, initial.nodes);
        assert_eq!(reused.edges, initial.edges);
        assert_eq!(reused.input.sha256, "status-only-refresh");

        let mut status_order_request = CellSceneRequest::new(projection.clone());
        status_order_request.orientation = CellSceneOrientation::LeftToRight;
        status_order_request.peer_order = CellPeerOrder::Status;
        let CellSceneResult::Ready(status_ordered) =
            CellSceneJob::new(3, status_order_request).run().result
        else {
            panic!("status-ordered scene")
        };
        let mut reordered_projection = projection.clone();
        reordered_projection.graph.nodes[0].status = Status::Done;
        let mut reordered_request = CellSceneRequest::new(reordered_projection);
        reordered_request.orientation = CellSceneOrientation::LeftToRight;
        reordered_request.peer_order = CellPeerOrder::Status;
        reordered_request.prior = Some(status_ordered);
        let CellSceneResult::Ready(reordered) =
            CellSceneJob::new(4, reordered_request).run().result
        else {
            panic!("changed status sort must relayout")
        };
        assert_ne!(reordered.movement, CellSceneMovement::ReusedStatusOnly);

        let mut vertical_request = CellSceneRequest::new(projection.clone());
        vertical_request.orientation = CellSceneOrientation::TopToBottom;
        vertical_request.prior = Some(reused.clone());
        let vertical_result = CellSceneJob::new(5, vertical_request).run().result;
        let CellSceneResult::Ready(vertical) = vertical_result else {
            panic!("vertical scene: {vertical_result:?}")
        };
        assert_eq!(vertical.direction, CellSceneDirection::TopToBottom);
        assert!(vertical.focus.as_ref().is_some_and(|focus| focus.recenter));
        assert!(matches!(
            &vertical.movement,
            CellSceneMovement::Relayout {
                orientation_changed: true,
                reasons,
                ..
            } if reasons.contains(&CellRelayoutReason::OrientationChange)
        ));

        let mut resized_request = CellSceneRequest::new(projection.clone());
        resized_request.orientation = CellSceneOrientation::LeftToRight;
        resized_request.prior = Some(initial.clone());
        let resized_identity = projection.graph.nodes[0].identity.clone();
        resized_request
            .card_sizes
            .get_mut(&resized_identity)
            .expect("fixture card")
            .columns += 1;
        let CellSceneResult::Ready(resized) = CellSceneJob::new(6, resized_request).run().result
        else {
            panic!("resized scene")
        };
        assert!(matches!(
            &resized.movement,
            CellSceneMovement::Relayout { reasons, .. }
                if reasons.contains(&CellRelayoutReason::CardSizeChange)
        ));

        let mut over_budget = CellSceneRequest::new(projection.clone());
        over_budget.budget.max_nodes = 1;
        over_budget.prior = Some(vertical.clone());
        let CellSceneResult::Fallback(fallback) = CellSceneJob::new(7, over_budget).run().result
        else {
            panic!("over-budget work must fall back")
        };
        assert!(matches!(
            fallback.reasons.as_slice(),
            [CellSceneDegradation::LimitExceeded {
                resource,
                got: 4,
                limit: 1,
            }] if resource == "nodes"
        ));
        assert_eq!(fallback.nodes.len(), projection.graph.nodes.len());
        assert_eq!(fallback.edges.len(), projection.graph.edges.len());
        assert_eq!(fallback.retained.as_deref(), Some(vertical.as_ref()));
    }

    fn synthetic_projection(
        ids: &[&str],
        dependencies: &[(usize, usize)],
    ) -> GraphExplorerProjection {
        let seed = graph_fixture_workspace();
        synthetic_projection_from_seed(&seed, ids, dependencies)
    }

    fn synthetic_projection_from_seed(
        seed: &GraphWorkspace,
        ids: &[&str],
        dependencies: &[(usize, usize)],
    ) -> GraphExplorerProjection {
        let mut tasks = Vec::with_capacity(ids.len());
        let mut catalogue = Vec::with_capacity(ids.len());
        for (index, id) in ids.iter().enumerate() {
            let mut task = seed.tasks[0].clone();
            task.id = (*id).to_string();
            task.title = format!("node {id}");
            task.phase = 1;
            task.status = Status::Todo;
            task.tags.clear();
            task.blocked_by = dependencies
                .iter()
                .filter(|(_, to)| *to == index)
                .map(|(from, _)| ids[*from].to_string())
                .collect();
            tasks.push(task);

            let mut row = seed.catalogue[0].clone();
            row.id = (*id).to_string();
            row.title = format!("node {id}");
            row.phase = 1;
            row.status = Status::Todo;
            row.owner = None;
            row.tags.clear();
            row.cut = None;
            row.milestone = None;
            row.ready_rank = (index == 0).then_some(1);
            row.rank_reason = None;
            row.body.clear();
            catalogue.push(row);
        }
        let workspace = GraphWorkspace::new(seed.input.clone(), tasks, catalogue);
        let request = if ids.len() > AGGREGATE_WINDOW_NODES {
            GraphExplorerRequest {
                location: GraphExplorerLocation::Task(ids[0].to_string()),
                budget: GraphWindowBudget {
                    nodes: ids.len(),
                    edges: dependencies.len(),
                    lanes: usize::MAX,
                },
                ..GraphExplorerRequest::default()
            }
        } else {
            GraphExplorerRequest::default()
        };
        workspace.project_explorer(&request)
    }

    fn assert_scene_edge_contract(
        label: &str,
        projection: &GraphExplorerProjection,
        scene: &CellScene,
    ) {
        let mut ordinals = BTreeMap::<(GraphExplorerIdentity, GraphExplorerIdentity), u32>::new();
        let expected_edges = projection
            .graph
            .edges
            .iter()
            .map(|edge| {
                let ordinal = ordinals
                    .entry((edge.from.clone(), edge.to.clone()))
                    .or_default();
                let identity = CellEdgeIdentity {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    ordinal: *ordinal,
                    provenance: edge.provenance.clone(),
                };
                *ordinal = ordinal.saturating_add(1);
                identity
            })
            .collect::<Vec<_>>();
        assert_eq!(
            scene
                .edges
                .iter()
                .map(|edge| &edge.identity)
                .collect::<Vec<_>>(),
            expected_edges.iter().collect::<Vec<_>>(),
            "{label}: edge identities or provenance moved"
        );
        for edge in &scene.edges {
            assert!(edge.degradations.is_empty(), "{label}: {edge:?}");
            assert!(edge.points.len() >= 2, "{label}: {edge:?}");
            assert_eq!(edge.arrowhead, edge.points.last().copied(), "{label}");
            assert_eq!(
                edge.source_port.map(|port| port.point),
                edge.points.first().copied(),
                "{label}"
            );
            assert_eq!(
                edge.target_port.map(|port| port.point),
                edge.points.last().copied(),
                "{label}"
            );
            assert!(
                edge.points
                    .windows(2)
                    .all(|pair| { pair[0].x == pair[1].x || pair[0].y == pair[1].y }),
                "{label}: non-orthogonal {edge:?}"
            );
            if edge.feedback == CellFeedbackKind::None {
                assert!(
                    edge.points.windows(2).all(|pair| match scene.direction {
                        CellSceneDirection::LeftToRight => pair[0].x <= pair[1].x,
                        CellSceneDirection::TopToBottom => pair[0].y <= pair[1].y,
                    }),
                    "{label}: forward route reversed {edge:?}"
                );
            }
        }
    }

    fn assert_scene_node_contract(label: &str, scene: &CellScene) {
        for (index, node) in scene.nodes.iter().enumerate() {
            assert!(node.bounds.width > 0 && node.bounds.height > 0, "{label}");
            assert!(
                node.bounds.x >= scene.bounds.x
                    && node.bounds.y >= scene.bounds.y
                    && node.bounds.x + i32::from(node.bounds.width)
                        <= scene.bounds.x + i32::from(scene.bounds.width)
                    && node.bounds.y + i32::from(node.bounds.height)
                        <= scene.bounds.y + i32::from(scene.bounds.height),
                "{label}: {:?}",
                node.identity
            );
            assert!(
                scene.nodes[index + 1..].iter().all(|other| {
                    let left = node.bounds;
                    let right = other.bounds;
                    !(left.x < right.x + i32::from(right.width)
                        && right.x < left.x + i32::from(left.width)
                        && left.y < right.y + i32::from(right.height)
                        && right.y < left.y + i32::from(left.height))
                }),
                "{label}: {:?} overlaps another node",
                node.identity
            );
        }
    }

    fn assert_scene_band_contract(
        label: &str,
        projection: &GraphExplorerProjection,
        scene: &CellScene,
    ) {
        for (band, semantic) in scene.bands.iter().zip(&projection.graph.phase_bands) {
            assert_eq!(band.phase, semantic.phase, "{label}");
            assert_eq!(band.nodes, semantic.nodes, "{label}");
            if band.nodes.is_empty() {
                assert_eq!(
                    band.degradation,
                    Some(CellSceneDegradation::EmptyPhaseBand),
                    "{label}: empty phase was not explicit"
                );
                continue;
            }
            assert_eq!(band.degradation, None, "{label}");
            let bounds = band.bounds.expect("non-empty phase has bounds");
            for identity in &band.nodes {
                let node = scene
                    .nodes
                    .iter()
                    .find(|node| &node.identity == identity)
                    .expect("phase member has a scene node");
                assert!(
                    node.bounds.x >= bounds.x
                        && node.bounds.y >= bounds.y
                        && node.bounds.x + i32::from(node.bounds.width)
                            <= bounds.x + i32::from(bounds.width)
                        && node.bounds.y + i32::from(node.bounds.height)
                            <= bounds.y + i32::from(bounds.height),
                    "{label}: phase {} does not contain {identity:?}",
                    band.phase
                );
            }
        }
        let bounded_bands = scene
            .bands
            .iter()
            .filter_map(|band| band.bounds.map(|bounds| (band.phase, bounds)))
            .collect::<Vec<_>>();
        for (index, (phase, bounds)) in bounded_bands.iter().enumerate() {
            assert!(
                bounded_bands[index + 1..].iter().all(|(_, other)| {
                    !(bounds.x < other.x + i32::from(other.width)
                        && other.x < bounds.x + i32::from(bounds.width)
                        && bounds.y < other.y + i32::from(other.height)
                        && other.y < bounds.y + i32::from(bounds.height))
                }),
                "{label}: phase {phase} overlaps another phase band"
            );
        }
    }

    fn assert_complete_scene_contract(
        label: &str,
        projection: &GraphExplorerProjection,
        scene: &CellScene,
    ) {
        assert_eq!(scene.nodes.len(), projection.graph.nodes.len(), "{label}");
        assert_eq!(scene.edges.len(), projection.graph.edges.len(), "{label}");
        assert_eq!(
            scene.bands.len(),
            projection.graph.phase_bands.len(),
            "{label}"
        );
        assert_eq!(scene.metrics.node_overlaps, 0, "{label}");
        assert_eq!(scene.metrics.node_edge_hits, 0, "{label}");
        assert_eq!(scene.metrics.endpoint_errors, 0, "{label}");
        assert_eq!(scene.metrics.degraded_edges, 0, "{label}");
        assert_scene_node_contract(label, scene);
        assert_scene_edge_contract(label, projection, scene);
        assert_scene_band_contract(label, projection, scene);
        let represented = scene
            .edges
            .iter()
            .map(|edge| edge.identity.provenance.len())
            .sum::<usize>()
            + projection
                .graph
                .nodes
                .iter()
                .map(|node| node.internal_dependencies.len())
                .sum::<usize>();
        assert_eq!(
            represented, projection.accounting.represented_dependency_edges,
            "{label}: dependency accounting moved"
        );
    }

    /// One named hostile corpus drives projection and scene hardening. The
    /// inventory is deliberately visible in one place so a later graph shape
    /// cannot be credited by an unrelated fixture with a similar title.
    type HostileGraphCase<'a> = (&'a str, Vec<&'a str>, Vec<(usize, usize)>);

    fn lane_scene_for(
        projection: &GraphExplorerProjection,
    ) -> aplan_output::graph_scene::GraphScene {
        use aplan_output::{GraphEdge, GraphNode, Grouping};

        let view = GraphView {
            nodes: projection
                .graph
                .nodes
                .iter()
                .map(|node| GraphNode {
                    id: node.identity.id().to_string(),
                    title: node.title.clone(),
                    status: node.status,
                    done: node.status.satisfies_dependency(),
                })
                .collect(),
            edges: projection
                .graph
                .edges
                .iter()
                .map(|edge| GraphEdge {
                    from: edge.from.id().to_string(),
                    to: edge.to.id().to_string(),
                    kind: EdgeKind::Dependency,
                })
                .collect(),
            hidden_edges: 0,
            grouping: Grouping::default(),
            scope: Vec::new(),
        };
        aplan_output::graph_scene::GraphScene::layout(&view, aplan_output::MAX_WIDTH)
            .expect("the matched hostile fixture fits the lane renderer")
    }

    fn lane_crossings_and_bends(scene: &aplan_output::graph_scene::GraphScene) -> (usize, usize) {
        use aplan_output::graph_scene::{ConnectorKind, SceneEdgeId, ScenePoint};

        let paths: BTreeMap<&SceneEdgeId, BTreeSet<ScenePoint>> = scene
            .edges
            .iter()
            .map(|edge| (&edge.id, edge.cells.iter().map(|cell| cell.0).collect()))
            .collect();
        let direction_at = |edge: &SceneEdgeId, point: ScenePoint| {
            let points = paths.get(edge).expect("connector names a known edge");
            let horizontal = points
                .iter()
                .any(|other| other.row == point.row && other.column.abs_diff(point.column) == 1);
            let vertical = points
                .iter()
                .any(|other| other.column == point.column && other.row.abs_diff(point.row) == 1);
            (horizontal, vertical)
        };

        let crossings = scene
            .rows()
            .iter()
            .flat_map(|row| &row.connectors)
            .filter(|cell| cell.kind == ConnectorKind::Cross)
            .map(|cell| {
                cell.edges
                    .iter()
                    .enumerate()
                    .map(|(index, left)| {
                        let left = direction_at(left, cell.id.0);
                        cell.edges[index + 1..]
                            .iter()
                            .filter(|right| {
                                let right = direction_at(right, cell.id.0);
                                (left.0 && right.1) || (left.1 && right.0)
                            })
                            .count()
                    })
                    .sum::<usize>()
            })
            .sum();
        let bends = paths
            .values()
            .map(|points| {
                points
                    .iter()
                    .filter(|point| {
                        let horizontal = points.iter().filter(|other| {
                            other.row == point.row && other.column.abs_diff(point.column) == 1
                        });
                        let vertical = points.iter().filter(|other| {
                            other.column == point.column && other.row.abs_diff(point.row) == 1
                        });
                        horizontal.count() == 1 && vertical.count() == 1
                    })
                    .count()
            })
            .sum();
        (crossings, bends)
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    /// Proof for `graph.layered_hardening_corpus_is_executable` and
    /// `graph.layered_routes_choose_low_bend_ports_within_crossing_budget`.
    fn hostile_graph_corpus_preserves_every_scene_invariant() {
        let seed = graph_fixture_workspace();
        let cases: Vec<HostileGraphCase<'_>> = vec![
            (
                "chain",
                vec!["C-0", "C-1", "C-2", "C-3", "C-4", "C-5"],
                vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)],
            ),
            (
                "diamond",
                vec!["D-0", "D-1", "D-2", "D-3"],
                vec![(0, 1), (0, 2), (1, 3), (2, 3)],
            ),
            (
                "repeated-fan-in-out",
                vec!["F-0", "F-1", "F-2", "F-3", "F-4", "F-5", "F-6"],
                vec![
                    (0, 1),
                    (0, 2),
                    (0, 3),
                    (1, 4),
                    (2, 4),
                    (3, 4),
                    (4, 5),
                    (4, 6),
                ],
            ),
            (
                "dense-crossings-and-long-edge",
                vec!["X-0", "X-1", "X-2", "X-3", "X-4", "X-5", "X-6", "X-7"],
                vec![
                    (0, 3),
                    (0, 4),
                    (1, 3),
                    (1, 5),
                    (2, 4),
                    (2, 5),
                    (3, 6),
                    (4, 7),
                    (5, 6),
                    (0, 7),
                ],
            ),
            (
                "disconnected",
                vec!["I-0", "I-1", "I-2", "I-3", "I-4"],
                vec![(0, 1), (2, 3)],
            ),
            (
                "cycle-self-and-parallel",
                vec!["P-0", "P-1", "P-2"],
                vec![(0, 0), (0, 1), (0, 1), (1, 2), (2, 0)],
            ),
        ];
        assert_eq!(
            cases.iter().map(|case| case.0).collect::<Vec<_>>(),
            [
                "chain",
                "diamond",
                "repeated-fan-in-out",
                "dense-crossings-and-long-edge",
                "disconnected",
                "cycle-self-and-parallel",
            ]
        );

        for (case_index, (name, ids, dependencies)) in cases.iter().enumerate() {
            let mut projection =
                synthetic_projection_from_seed(&seed, ids, dependencies.as_slice());
            if *name == "dense-crossings-and-long-edge" {
                projection.graph.nodes[0].title =
                    "hostile \u{754c} e\u{301} \u{1f642} label".repeat(4);
            }
            let lane_metrics = (*name == "dense-crossings-and-long-edge").then(|| {
                let lanes = lane_scene_for(&projection);
                lane_crossings_and_bends(&lanes)
            });
            for orientation in [
                CellSceneOrientation::LeftToRight,
                CellSceneOrientation::TopToBottom,
            ] {
                let mut request = CellSceneRequest::new(projection.clone());
                request.orientation = orientation;
                let generation = u64::try_from(case_index).unwrap_or(u64::MAX);
                let first = CellSceneJob::new(generation, request.clone()).run();
                let second = CellSceneJob::new(generation, request).run();
                assert_eq!(first, second, "{name}: geometry is not deterministic");
                let CellSceneResult::Ready(scene) = first.result else {
                    panic!("{name}: bounded corpus case must produce a complete scene")
                };
                assert_complete_scene_contract(name, &projection, &scene);
                if let Some((lane_crossings, lane_bends)) = lane_metrics {
                    assert!(
                        scene.metrics.crossings <= lane_crossings,
                        "{name} {:?}: Layered has {} crossings; Lanes has {lane_crossings}",
                        scene.direction,
                        scene.metrics.crossings,
                    );
                    assert!(
                        scene.metrics.bends.saturating_mul(4) <= lane_bends.saturating_mul(5),
                        "{name} {:?}: Layered has {} bends; Lanes has {lane_bends}",
                        scene.direction,
                        scene.metrics.bends,
                    );
                }
                eprintln!(
                    "GRAPH_HARDENING case={name} direction={:?} nodes={} edges={} crossings={} bends={} area_cells={} shared_segments={} junctions={} lane_crossings={:?} lane_bends={:?}",
                    scene.direction,
                    scene.nodes.len(),
                    scene.edges.len(),
                    scene.metrics.crossings,
                    scene.metrics.bends,
                    scene.metrics.area_cells,
                    scene.metrics.shared_segments,
                    scene.metrics.junctions,
                    lane_metrics.map(|metrics| metrics.0),
                    lane_metrics.map(|metrics| metrics.1),
                );
            }
        }

        let multiscale =
            graph_explorer_fixture_workspace().project_explorer(&GraphExplorerRequest::default());
        assert!(multiscale.graph.nodes.iter().any(|node| {
            node.identity == GraphExplorerIdentity::Milestone("M-empty".to_string())
                && node.member_ids.is_empty()
        }));
        assert!(multiscale.graph.edges.iter().any(|edge| {
            edge.disposition == GraphExplorerEdgeDisposition::AggregateFeedback
                && edge.dependency_count == edge.provenance.len()
                && !edge.provenance.is_empty()
        }));
        assert!(
            !multiscale.conditions.iter().any(|condition| matches!(
                condition,
                GraphExplorerCondition::InvalidTaskCycle { .. }
            ))
        );
        let CellSceneResult::Ready(scene) =
            CellSceneJob::new(99, CellSceneRequest::new(multiscale.clone()))
                .run()
                .result
        else {
            panic!("cross-phase/cross-cut aggregate corpus must produce a scene")
        };
        assert_complete_scene_contract("multiscale-aggregate-cycle", &multiscale, &scene);
        assert!(scene.edges.iter().any(|edge| {
            edge.feedback == CellFeedbackKind::AggregateQuotient
                && !edge.identity.provenance.is_empty()
        }));
    }

    const RELEASE_CANDIDATE_IDS: &[&str] = &[
        "A1-445", "A1-446", "A1-447", "A1-448", "A1-449", "A1-450", "A1-451", "A1-452", "A1-662",
        "A1-663", "A1-664", "A1-665", "A1-666", "A1-667", "A1-668", "A1-669", "A1-670", "A1-671",
        "A1-672", "A1-673", "A1-674", "A1-676", "A1-677", "A1-678", "A1-679", "A1-680", "A1-681",
        "A1-682", "A1-683", "A1-684", "A1-685", "A1-686", "A1-687", "A1-688", "A1-689", "A1-696",
    ];
    const RELEASE_CANDIDATE_PAIRS: &[(&str, &str)] = &[
        ("A1-662", "A1-445"),
        ("A1-445", "A1-446"),
        ("A1-663", "A1-446"),
        ("A1-445", "A1-447"),
        ("A1-666", "A1-447"),
        ("A1-667", "A1-447"),
        ("A1-669", "A1-447"),
        ("A1-670", "A1-447"),
        ("A1-671", "A1-447"),
        ("A1-672", "A1-447"),
        ("A1-446", "A1-448"),
        ("A1-666", "A1-448"),
        ("A1-667", "A1-448"),
        ("A1-671", "A1-448"),
        ("A1-672", "A1-448"),
        ("A1-445", "A1-449"),
        ("A1-446", "A1-449"),
        ("A1-447", "A1-449"),
        ("A1-448", "A1-449"),
        ("A1-683", "A1-449"),
        ("A1-447", "A1-450"),
        ("A1-448", "A1-450"),
        ("A1-683", "A1-450"),
        ("A1-445", "A1-451"),
        ("A1-446", "A1-451"),
        ("A1-447", "A1-451"),
        ("A1-448", "A1-451"),
        ("A1-683", "A1-451"),
        ("A1-449", "A1-452"),
        ("A1-450", "A1-452"),
        ("A1-451", "A1-452"),
        ("A1-674", "A1-452"),
        ("A1-696", "A1-452"),
        ("A1-445", "A1-663"),
        ("A1-446", "A1-664"),
        ("A1-664", "A1-665"),
        ("A1-665", "A1-666"),
        ("A1-665", "A1-667"),
        ("A1-664", "A1-668"),
        ("A1-668", "A1-669"),
        ("A1-664", "A1-670"),
        ("A1-666", "A1-671"),
        ("A1-669", "A1-671"),
        ("A1-670", "A1-671"),
        ("A1-672", "A1-671"),
        ("A1-446", "A1-672"),
        ("A1-664", "A1-672"),
        ("A1-672", "A1-673"),
        ("A1-669", "A1-676"),
        ("A1-676", "A1-677"),
        ("A1-676", "A1-678"),
        ("A1-677", "A1-678"),
        ("A1-678", "A1-679"),
        ("A1-677", "A1-680"),
        ("A1-679", "A1-680"),
        ("A1-677", "A1-681"),
        ("A1-678", "A1-681"),
        ("A1-680", "A1-682"),
        ("A1-681", "A1-682"),
        ("A1-682", "A1-683"),
        ("A1-686", "A1-683"),
        ("A1-687", "A1-683"),
        ("A1-688", "A1-683"),
        ("A1-676", "A1-684"),
        ("A1-682", "A1-685"),
        ("A1-682", "A1-686"),
        ("A1-686", "A1-687"),
        ("A1-682", "A1-688"),
        ("A1-683", "A1-689"),
        ("A1-689", "A1-696"),
    ];

    /// Proof for `graph.cell_routes_repair_terminal_border_collisions`: the
    /// release-candidate milestone shape which exposed floating-point routes
    /// sharing painted card borders remains drawable in both orientations.
    #[test]
    fn release_candidate_milestone_repairs_cell_border_routes_in_both_orientations() {
        let by_id: BTreeMap<_, _> = RELEASE_CANDIDATE_IDS
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i))
            .collect();
        let dependencies: Vec<_> = RELEASE_CANDIDATE_PAIRS
            .iter()
            .map(|(from, to)| (by_id[from], by_id[to]))
            .collect();
        let projection = synthetic_projection(RELEASE_CANDIDATE_IDS, &dependencies);

        for orientation in [
            CellSceneOrientation::LeftToRight,
            CellSceneOrientation::TopToBottom,
        ] {
            let mut request = CellSceneRequest::new(projection.clone());
            request.orientation = orientation;
            request.card_sizes.values_mut().for_each(|size| {
                *size = CellCardSize {
                    columns: 16,
                    rows: 4,
                };
            });
            let result = CellSceneJob::new(696, request).run().result;
            let CellSceneResult::Ready(scene) = result else {
                panic!("release-candidate milestone fell back in {orientation:?}: {result:?}")
            };
            assert_complete_scene_contract("release-candidate milestone", &projection, &scene);
            let bends: usize = scene
                .edges
                .iter()
                .map(|edge| {
                    edge.points
                        .windows(3)
                        .filter(|window| {
                            (window[0].x == window[1].x) != (window[1].x == window[2].x)
                        })
                        .count()
                })
                .sum();
            assert_eq!(scene.metrics.bends, bends);
        }
    }

    #[test]
    /// Proof for `graph.layered_hardening_corpus_is_executable`.
    fn every_four_node_dag_has_complete_deterministic_cell_geometry() {
        let seed = graph_fixture_workspace();
        let ids = ["E-0", "E-1", "E-2", "E-3"];
        let possible = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];
        for mask in 0usize..(1 << possible.len()) {
            let dependencies = possible
                .iter()
                .enumerate()
                .filter_map(|(bit, edge)| ((mask & (1 << bit)) != 0).then_some(*edge))
                .collect::<Vec<_>>();
            let projection = synthetic_projection_from_seed(&seed, &ids, &dependencies);
            for orientation in [
                CellSceneOrientation::LeftToRight,
                CellSceneOrientation::TopToBottom,
            ] {
                let mut request = CellSceneRequest::new(projection.clone());
                request.orientation = orientation;
                let CellSceneResult::Ready(scene) =
                    CellSceneJob::new(u64::try_from(mask).unwrap_or(u64::MAX), request)
                        .run()
                        .result
                else {
                    panic!("four-node DAG mask {mask} must produce complete geometry")
                };
                assert_complete_scene_contract(&format!("DAG mask {mask}"), &projection, &scene);
            }
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn graph_entry_policy_names_every_reason_and_never_mutates_source_state() {
        let seed = graph_explorer_fixture_workspace();
        let source_tasks = seed.tasks.clone();
        let source_catalogue = seed.catalogue.clone();
        let project = |tasks: Vec<Task>, catalogue: Vec<TaskBrowserRow>, request| {
            GraphWorkspace::new(seed.input.clone(), tasks, catalogue).project_explorer(&request)
        };

        let contextual = project(
            seed.tasks.clone(),
            seed.catalogue.clone(),
            GraphExplorerRequest {
                selected: Some(GraphExplorerIdentity::Task("A-2".to_string())),
                selection_origin: GraphExplorerSelectionOrigin::Contextual,
                ..GraphExplorerRequest::default()
            },
        );
        assert_eq!(
            contextual.selection.expect("contextual anchor").reason,
            GraphExplorerAnchorReason::Contextual
        );
        let remembered = project(
            seed.tasks.clone(),
            seed.catalogue.clone(),
            GraphExplorerRequest {
                selected: Some(GraphExplorerIdentity::Task("A-2".to_string())),
                selection_origin: GraphExplorerSelectionOrigin::Remembered,
                ..GraphExplorerRequest::default()
            },
        );
        assert_eq!(
            remembered.selection.expect("remembered anchor").reason,
            GraphExplorerAnchorReason::Remembered
        );
        let automatic = project(
            seed.tasks.clone(),
            seed.catalogue.clone(),
            GraphExplorerRequest::default(),
        );
        assert_eq!(
            automatic.selection.expect("in-progress anchor").reason,
            GraphExplorerAnchorReason::InProgress
        );

        let mut ready_tasks = seed.tasks.clone();
        let mut ready_catalogue = seed.catalogue.clone();
        for task in &mut ready_tasks {
            if task.id == "A-1" {
                task.status = Status::Todo;
            }
        }
        for row in &mut ready_catalogue {
            row.status = Status::Todo;
            row.ready_rank = (row.id == "A-2").then_some(0);
        }
        let ready = project(
            ready_tasks.clone(),
            ready_catalogue.clone(),
            GraphExplorerRequest::default(),
        );
        assert_eq!(
            ready.selection.expect("ready anchor").reason,
            GraphExplorerAnchorReason::Ready
        );

        for row in &mut ready_catalogue {
            row.ready_rank = matches!(row.id.as_str(), "A-2" | "B-1").then_some(0);
        }
        let tied = project(
            ready_tasks.clone(),
            ready_catalogue.clone(),
            GraphExplorerRequest::default(),
        );
        let mut reversed_tasks = ready_tasks.clone();
        reversed_tasks.reverse();
        let mut reversed_catalogue = ready_catalogue.clone();
        reversed_catalogue.reverse();
        let tied_reversed = project(
            reversed_tasks,
            reversed_catalogue,
            GraphExplorerRequest::default(),
        );
        assert_eq!(
            tied.selection, tied_reversed.selection,
            "ready ties depend on input enumeration"
        );
        assert_eq!(
            tied.selection.expect("ready tie anchor").subject,
            GraphExplorerIdentity::Task("A-2".to_string())
        );

        for row in &mut ready_catalogue {
            row.ready_rank = None;
        }
        let unfinished = project(
            ready_tasks.clone(),
            ready_catalogue.clone(),
            GraphExplorerRequest::default(),
        );
        assert_eq!(
            unfinished.selection.expect("unfinished anchor").reason,
            GraphExplorerAnchorReason::FirstUnfinishedMilestone
        );

        for task in &mut ready_tasks {
            if task
                .tags
                .iter()
                .any(|tag| tag == aplan_core::task::MILESTONE_TAG)
            {
                task.status = Status::Done;
            }
        }
        for row in &mut ready_catalogue {
            if row
                .tags
                .iter()
                .any(|tag| tag == aplan_core::task::MILESTONE_TAG)
            {
                row.status = Status::Done;
            }
        }
        let all_done = project(
            ready_tasks,
            ready_catalogue,
            GraphExplorerRequest::default(),
        );
        assert_eq!(
            all_done.selection.expect("all-done anchor").reason,
            GraphExplorerAnchorReason::FirstMilestone
        );

        let no_milestone = synthetic_projection_from_seed(&seed, &["U-1", "U-2"], &[(0, 1)]);
        assert!(
            no_milestone
                .conditions
                .iter()
                .any(|condition| matches!(condition, GraphExplorerCondition::NoMilestones))
        );
        assert_eq!(
            no_milestone
                .selection
                .expect("ungrouped ready anchor")
                .reason,
            GraphExplorerAnchorReason::Ready
        );
        assert_eq!(
            seed.tasks, source_tasks,
            "projection mutated persisted tasks"
        );
        assert_eq!(
            seed.catalogue, source_catalogue,
            "projection mutated derived catalogue"
        );
    }

    #[test]
    fn cell_scene_ranks_match_an_independent_diamond_reference_in_both_directions() {
        let projection = synthetic_projection(
            &["D-0", "D-1", "D-2", "D-3"],
            &[(0, 1), (0, 2), (1, 3), (2, 3)],
        );
        let reference = BTreeMap::from([
            (GraphExplorerIdentity::Task("D-0".to_string()), 0),
            (GraphExplorerIdentity::Task("D-1".to_string()), 1),
            (GraphExplorerIdentity::Task("D-2".to_string()), 1),
            (GraphExplorerIdentity::Task("D-3".to_string()), 2),
        ]);
        for orientation in [
            CellSceneOrientation::LeftToRight,
            CellSceneOrientation::TopToBottom,
        ] {
            let mut request = CellSceneRequest::new(projection.clone());
            request.orientation = orientation;
            request
                .ordering_hints
                .insert(GraphExplorerIdentity::Task("D-1".to_string()), 10);
            request
                .ordering_hints
                .insert(GraphExplorerIdentity::Task("D-2".to_string()), 0);
            let CellSceneResult::Ready(scene) = CellSceneJob::new(1, request).run().result else {
                panic!("diamond scene")
            };
            assert_eq!(
                scene
                    .nodes
                    .iter()
                    .map(|node| (node.identity.clone(), node.rank.expect("connected rank")))
                    .collect::<BTreeMap<_, _>>(),
                reference
            );
            for edge in &scene.edges {
                let from = reference[&edge.identity.from];
                let to = reference[&edge.identity.to];
                assert!(from < to);
                assert_eq!(edge.feedback, CellFeedbackKind::None);
                assert!(edge.degradations.is_empty());
            }
            let orders: BTreeMap<_, _> = scene
                .nodes
                .iter()
                .map(|node| (node.identity.id(), node.order))
                .collect();
            assert!(orders["D-2"] < orders["D-1"]);
        }
    }

    #[test]
    fn phase_bands_group_without_inventing_causality_or_rank() {
        let mut projection = synthetic_projection(&["P-1", "P-2"], &[]);
        projection.graph.nodes[0].phase = 1;
        projection.graph.nodes[1].phase = 2;
        projection.graph.phase_bands = vec![
            GraphExplorerPhaseBand {
                phase: 1,
                nodes: vec![projection.graph.nodes[0].identity.clone()],
                facts: projection.graph.nodes[0].facts.clone(),
            },
            GraphExplorerPhaseBand {
                phase: 2,
                nodes: vec![projection.graph.nodes[1].identity.clone()],
                facts: projection.graph.nodes[1].facts.clone(),
            },
        ];
        let mut request = CellSceneRequest::new(projection);
        request.orientation = CellSceneOrientation::LeftToRight;
        let CellSceneResult::Ready(scene) = CellSceneJob::new(1, request).run().result else {
            panic!("phase lane scene")
        };
        assert!(scene.edges.is_empty());
        assert_eq!(scene.bands.len(), 2);
        assert!(scene.bands.iter().all(|band| band.bounds.is_some()));
        assert_eq!(scene.nodes[0].rank, scene.nodes[1].rank);
    }

    #[test]
    fn phase_boundaries_accept_dependency_crossings_in_both_group_directions() {
        let mut projection = synthetic_projection(&["P-0", "P-1", "P-2"], &[(0, 1), (1, 2)]);
        projection.graph.nodes[0].phase = 1;
        projection.graph.nodes[1].phase = 2;
        projection.graph.nodes[2].phase = 1;
        projection.graph.phase_bands = vec![
            GraphExplorerPhaseBand {
                phase: 1,
                nodes: vec![
                    projection.graph.nodes[0].identity.clone(),
                    projection.graph.nodes[2].identity.clone(),
                ],
                facts: GraphExplorerFacts::default(),
            },
            GraphExplorerPhaseBand {
                phase: 2,
                nodes: vec![projection.graph.nodes[1].identity.clone()],
                facts: projection.graph.nodes[1].facts.clone(),
            },
        ];
        let mut request = CellSceneRequest::new(projection);
        request.orientation = CellSceneOrientation::LeftToRight;
        let CellSceneResult::Ready(scene) = CellSceneJob::new(1, request).run().result else {
            panic!("cross-phase scene")
        };
        assert_eq!(scene.edges.len(), 2);
        assert!(scene.edges.iter().all(|edge| {
            edge.feedback == CellFeedbackKind::None
                && edge.degradations.is_empty()
                && edge
                    .points
                    .windows(2)
                    .all(|segment| segment[0].x <= segment[1].x)
        }));
        assert_eq!(scene.bands.len(), 2);
        assert!(scene.bands.iter().all(|band| band.bounds.is_some()));
        assert!(
            scene.bands[0].bounds.expect("phase 1").y < scene.bands[1].bounds.expect("phase 2").y
        );
    }

    #[test]
    fn filtered_empty_phase_bands_remain_typed_omissions() {
        let workspace = graph_explorer_fixture_workspace();
        let projection = workspace.project_explorer(&GraphExplorerRequest {
            graph: GraphRequest {
                selector: Selector {
                    phases: vec![1],
                    ..Selector::default()
                },
                ..GraphRequest::default()
            },
            ..GraphExplorerRequest::default()
        });
        let CellSceneResult::Ready(scene) = CellSceneJob::new(1, CellSceneRequest::new(projection))
            .run()
            .result
        else {
            panic!("filtered scene")
        };
        assert!(scene.bands.iter().any(|band| {
            band.phase == 2
                && band.nodes.is_empty()
                && band.bounds.is_none()
                && band.label_anchor.is_none()
                && band.degradation == Some(CellSceneDegradation::EmptyPhaseBand)
        }));
    }

    #[test]
    fn cell_scene_preserves_self_loop_cycle_and_parallel_edge_identities() {
        let projection = synthetic_projection(&["C-0", "C-1"], &[(0, 0), (0, 1), (0, 1), (1, 0)]);
        assert_eq!(projection.state, GraphExplorerState::InvalidTaskCycle);
        let mut request = CellSceneRequest::new(projection);
        request.orientation = CellSceneOrientation::LeftToRight;
        let CellSceneResult::Ready(scene) = CellSceneJob::new(1, request).run().result else {
            panic!("cyclic scene")
        };
        assert!(scene.edges.iter().any(|edge| {
            edge.identity.from == edge.identity.to
                && edge.kind == Some(CellRouteKind::SelfLoop)
                && edge.feedback == CellFeedbackKind::CanonicalTaskCycle
        }));
        let parallel = scene
            .parallel_edges
            .iter()
            .find(|group| group.from.id() == "C-0" && group.to.id() == "C-1")
            .expect("parallel group");
        assert_eq!(parallel.edges.len(), 2);
        assert_eq!(parallel.edges[0].ordinal, 0);
        assert_eq!(parallel.edges[1].ordinal, 1);
        assert!(
            scene
                .edges
                .iter()
                .all(|edge| edge.feedback != CellFeedbackKind::None)
        );
    }

    #[test]
    fn cell_scene_handles_budget_ceiling_fanout_wide_labels_and_singletons() {
        let ids: Vec<String> = (0..64).map(|index| format!("F-{index:02}")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let dependencies: Vec<_> = (1..64).map(|target| (0, target)).collect();
        let mut fanout = synthetic_projection(&id_refs, &dependencies);
        fanout.graph.nodes[0].title =
            "A wide \u{1f642} title with e\u{301} combining clusters and a long suffix".repeat(3);
        let request = CellSceneRequest::new(fanout);
        assert!(
            request
                .card_sizes
                .values()
                .all(|size| size.columns <= 28 && size.rows >= 4)
        );
        let CellSceneResult::Ready(scene) = CellSceneJob::new(1, request).run().result else {
            panic!("the exact budget ceiling must be layoutable")
        };
        assert_eq!(scene.nodes.len(), 64);
        assert_eq!(scene.edges.len(), 63);
        assert!(scene.metrics.routing_waypoints <= 1_536);

        let singleton = synthetic_projection(&["ONE"], &[]);
        let CellSceneResult::Ready(singleton) =
            CellSceneJob::new(2, CellSceneRequest::new(singleton))
                .run()
                .result
        else {
            panic!("singleton scene")
        };
        assert_eq!(singleton.nodes.len(), 1);
        assert!(singleton.edges.is_empty());
        assert_eq!(singleton.nodes[0].rank, Some(0));
    }

    #[test]
    fn cell_scene_refuses_ambiguous_semantic_projection_before_layout() {
        let projection =
            graph_explorer_fixture_workspace().project_explorer(&GraphExplorerRequest::default());

        let mut duplicate = projection.clone();
        duplicate.graph.nodes[1].identity = duplicate.graph.nodes[0].identity.clone();
        let CellSceneResult::Fallback(duplicate) =
            CellSceneJob::new(1, CellSceneRequest::new(duplicate))
                .run()
                .result
        else {
            panic!("duplicate identities must not become geometry")
        };
        assert!(matches!(
            duplicate.reasons.as_slice(),
            [CellSceneDegradation::InvalidProjection { issue }]
                if issue == "duplicate node identity"
        ));

        let mut mismatched = projection.clone();
        let wrong = mismatched
            .graph
            .nodes
            .iter()
            .find(|node| node.phase != mismatched.graph.phase_bands[0].phase)
            .expect("fixture has another phase")
            .identity
            .clone();
        mismatched.graph.phase_bands[0].nodes.push(wrong);
        let CellSceneResult::Fallback(mismatched) =
            CellSceneJob::new(2, CellSceneRequest::new(mismatched))
                .run()
                .result
        else {
            panic!("false phase membership must not become geometry")
        };
        assert!(matches!(
            mismatched.reasons.as_slice(),
            [CellSceneDegradation::InvalidProjection { issue }]
                if issue == "phase membership disagrees with its node"
        ));

        let mut unknown_hint = CellSceneRequest::new(projection);
        unknown_hint
            .ordering_hints
            .insert(GraphExplorerIdentity::Task("not-admitted".to_string()), 0);
        let CellSceneResult::Fallback(unknown_hint) =
            CellSceneJob::new(3, unknown_hint).run().result
        else {
            panic!("an unknown ordering subject must not be ignored")
        };
        assert!(matches!(
            unknown_hint.reasons.as_slice(),
            [CellSceneDegradation::InvalidProjection { issue }]
                if issue == "ordering hint names an unadmitted node"
        ));
    }

    fn assert_exact_boundary_accounting(projection: &GraphExplorerProjection) {
        let boundary = &projection.accounting.boundary_dependencies;
        assert!(!boundary.is_empty());
        assert_eq!(
            projection.accounting.boundary_dependency_edges,
            boundary.len()
        );
        assert!(
            boundary.windows(2).all(|pair| pair[0] <= pair[1]),
            "boundary identities must be deterministic"
        );
    }

    #[test]
    fn graph_explorer_phase_focus_accounts_for_every_hidden_relationship() {
        let workspace = graph_explorer_fixture_workspace();
        let projection = workspace.project_explorer(&GraphExplorerRequest {
            location: GraphExplorerLocation::Phase(1),
            selected: Some(GraphExplorerIdentity::Task("A-1".to_string())),
            selection_origin: GraphExplorerSelectionOrigin::Contextual,
            ..GraphExplorerRequest::default()
        });
        let context = projection.phase_context.expect("phase context");
        assert_eq!(context.previous, None);
        assert_eq!(context.next, Some(2));
        assert_eq!(context.omitted.len(), 3);
        assert_eq!(context.boundary_edges.len(), 3);
        assert_eq!(
            context
                .boundary_edges
                .iter()
                .map(|edge| edge.dependency_count)
                .sum::<usize>(),
            3
        );
        assert_eq!(projection.accounting.level_task_ids, ["A-1", "A-2", "M-1"]);
        assert_eq!(projection.accounting.represented_dependency_edges, 3);
        assert_eq!(
            projection.selection.expect("selection").represented_by,
            GraphExplorerIdentity::Milestone("M-1".to_string())
        );

        let filtered = workspace.project_explorer(&GraphExplorerRequest {
            graph: GraphRequest {
                selector: Selector {
                    phases: vec![1],
                    ..Selector::default()
                },
                ..GraphRequest::default()
            },
            ..GraphExplorerRequest::default()
        });
        assert!(filtered.conditions.iter().any(|condition| matches!(
            condition,
            GraphExplorerCondition::EmptyPhases { phases } if phases == &[2, 3, 4]
        )));
        assert_exact_boundary_accounting(&filtered);
        assert!(
            filtered
                .transitions
                .iter()
                .any(|transition| { transition.to == GraphExplorerLocation::Phase(1) })
        );

        let checkpoint_excluded = workspace.project_explorer(&GraphExplorerRequest {
            graph: GraphRequest {
                selector: Selector {
                    statuses: vec![Status::InProgress],
                    ..Selector::default()
                },
                ..GraphRequest::default()
            },
            ..GraphExplorerRequest::default()
        });
        let container = checkpoint_excluded
            .graph
            .nodes
            .iter()
            .find(|node| node.identity == GraphExplorerIdentity::Milestone("M-1".to_string()))
            .expect("selected member retains its container");
        assert!(!container.checkpoint_in_scope);
        let drill = checkpoint_excluded
            .transitions
            .iter()
            .find(|transition| transition.to == GraphExplorerLocation::Milestone("M-1".to_string()))
            .expect("excluded checkpoint container remains drillable");
        assert!(
            workspace
                .follow_explorer_transition(
                    &GraphExplorerRequest {
                        graph: GraphRequest {
                            selector: Selector {
                                statuses: vec![Status::InProgress],
                                ..Selector::default()
                            },
                            ..GraphRequest::default()
                        },
                        ..GraphExplorerRequest::default()
                    },
                    drill
                )
                .is_ok()
        );

        let ready = workspace.project_explorer(&GraphExplorerRequest {
            graph: GraphRequest {
                selector: Selector {
                    statuses: vec![Status::Todo],
                    ..Selector::default()
                },
                ..GraphRequest::default()
            },
            ..GraphExplorerRequest::default()
        });
        assert_eq!(
            ready.selection.expect("ready fallback").reason,
            GraphExplorerAnchorReason::Ready
        );
    }

    #[test]
    fn graph_explorer_detail_windows_search_and_round_trips_are_exact() {
        let workspace = graph_explorer_fixture_workspace();
        let plan = workspace.project_explorer(&GraphExplorerRequest::default());
        let drill = plan
            .transitions
            .iter()
            .find(|transition| transition.to == GraphExplorerLocation::Milestone("M-1".to_string()))
            .expect("milestone transition")
            .clone();
        assert_eq!(
            drill.to_selection,
            Some(GraphExplorerIdentity::Task("A-1".to_string()))
        );
        let detail = workspace
            .follow_explorer_transition(&GraphExplorerRequest::default(), &drill)
            .expect("follow current transition");
        assert_eq!(
            detail.location,
            GraphExplorerLocation::Milestone("M-1".to_string())
        );
        assert_eq!(
            detail.selection.as_ref().map(|selected| &selected.subject),
            Some(&GraphExplorerIdentity::Task("A-1".to_string()))
        );
        assert!(
            detail
                .graph
                .nodes
                .iter()
                .all(|node| matches!(node.identity, GraphExplorerIdentity::Task(_)))
        );
        assert!(detail.graph.nodes.iter().any(|node| {
            node.identity == GraphExplorerIdentity::Task("A-2".to_string())
                && node.milestone.as_deref() == Some("M-1")
        }));

        let inverse = drill.inverse();
        let back_request = GraphExplorerRequest {
            location: detail.location.clone(),
            selected: detail
                .selection
                .as_ref()
                .map(|selected| selected.subject.clone()),
            selection_origin: GraphExplorerSelectionOrigin::Contextual,
            ..GraphExplorerRequest::default()
        };
        let back = workspace
            .follow_explorer_transition(&back_request, &inverse)
            .expect("follow exact inverse");
        assert_eq!(back.location, GraphExplorerLocation::Plan);
        assert_eq!(
            back.selection.expect("restored selection").represented_by,
            GraphExplorerIdentity::Milestone("M-1".to_string())
        );

        let bounded = workspace.project_explorer(&GraphExplorerRequest {
            location: GraphExplorerLocation::Milestone("M-1".to_string()),
            search: "ungrouped".to_string(),
            budget: GraphWindowBudget {
                nodes: 2,
                edges: 1,
                lanes: 1,
            },
            ..GraphExplorerRequest::default()
        });
        assert!(!bounded.accounting.window_omitted_task_ids.is_empty());
        assert!(!bounded.boundaries.is_empty());
        assert_eq!(bounded.search_matches.len(), 1);
        assert_eq!(bounded.search_matches[0].task_id, "X-1");
        assert_eq!(
            bounded.search_matches[0].open,
            GraphExplorerLocation::Task("X-1".to_string())
        );
        let continuation = bounded.boundaries[0].cursor.clone();
        let continued = workspace.project_explorer(&GraphExplorerRequest {
            location: GraphExplorerLocation::Milestone("M-1".to_string()),
            budget: GraphWindowBudget {
                nodes: 2,
                edges: 1,
                lanes: 1,
            },
            continuation: Some(continuation.clone()),
            ..GraphExplorerRequest::default()
        });
        assert_eq!(
            continued
                .semantic_window
                .as_ref()
                .and_then(|window| window.page.as_ref())
                .map(|page| &page.current),
            Some(&continuation)
        );
    }

    #[test]
    fn graph_explorer_transitions_are_source_bound_and_refresh_keeps_raw_identity() {
        let workspace = graph_explorer_fixture_workspace();
        let remembered = GraphExplorerRequest {
            selected: Some(GraphExplorerIdentity::Task("A-2".to_string())),
            selection_origin: GraphExplorerSelectionOrigin::Remembered,
            ..GraphExplorerRequest::default()
        };
        let projection = workspace.project_explorer(&remembered);
        assert_eq!(
            projection.selection.expect("remembered selection"),
            GraphExplorerSelection {
                subject: GraphExplorerIdentity::Task("A-2".to_string()),
                represented_by: GraphExplorerIdentity::Milestone("M-1".to_string()),
                reason: GraphExplorerAnchorReason::Remembered,
            }
        );
        let transition = workspace
            .project_explorer(&GraphExplorerRequest::default())
            .transitions
            .into_iter()
            .find(|candidate| matches!(candidate.to, GraphExplorerLocation::Milestone(_)))
            .expect("transition");
        let mut refreshed_input = workspace.input.clone();
        refreshed_input.sha256 = "refreshed-input".to_string();
        let refreshed = GraphWorkspace::new(
            refreshed_input,
            workspace.tasks.clone(),
            workspace.catalogue.clone(),
        );
        assert!(matches!(
            refreshed.follow_explorer_transition(&GraphExplorerRequest::default(), &transition),
            Err(GraphExplorerTransitionFailure::StaleInput { .. })
        ));
        assert!(matches!(
            workspace.follow_explorer_transition(
                &GraphExplorerRequest {
                    location: GraphExplorerLocation::Phase(1),
                    ..GraphExplorerRequest::default()
                },
                &transition
            ),
            Err(GraphExplorerTransitionFailure::WrongSource { .. })
        ));
    }

    #[test]
    fn graph_explorer_distinguishes_invalid_cycles_milestone_only_and_no_milestone() {
        let workspace = graph_explorer_fixture_workspace();
        let milestone_only = workspace.project_explorer(&GraphExplorerRequest {
            graph: GraphRequest {
                selector: Selector {
                    tags: vec!["milestone".to_string()],
                    ..Selector::default()
                },
                ..GraphRequest::default()
            },
            ..GraphExplorerRequest::default()
        });
        assert!(
            milestone_only
                .conditions
                .iter()
                .any(|condition| matches!(condition, GraphExplorerCondition::MilestoneOnly))
        );

        let no_milestone_dir = fixture();
        let no_milestone_context = test_context(
            &no_milestone_dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let no_milestone =
            graph_workspace(&no_milestone_context, task_catalogue(&no_milestone_context))
                .project_explorer(&GraphExplorerRequest::default());
        assert!(
            no_milestone
                .conditions
                .iter()
                .any(|condition| matches!(condition, GraphExplorerCondition::NoMilestones))
        );
        assert_eq!(
            no_milestone
                .selection
                .as_ref()
                .map(|selection| (&selection.subject, &selection.reason)),
            Some((
                &GraphExplorerIdentity::Task("A-1".to_string()),
                &GraphExplorerAnchorReason::Ready,
            ))
        );
        assert_eq!(no_milestone.no_selection_reason, None);

        let cycle_dir = fixture();
        std::fs::write(
            cycle_dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-2]\n---\n",
        )
        .expect("cycle first");
        std::fs::write(
            cycle_dir.path().join("aplan/tasks/A-2.md"),
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1]\n---\n",
        )
        .expect("cycle second");
        let cycle_context = test_context(
            &cycle_dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let cycle = graph_workspace(&cycle_context, task_catalogue(&cycle_context))
            .project_explorer(&GraphExplorerRequest::default());
        assert_eq!(cycle.state, GraphExplorerState::InvalidTaskCycle);
        assert!(cycle.conditions.iter().any(|condition| matches!(
            condition,
            GraphExplorerCondition::InvalidTaskCycle { components }
                if components == &vec![vec!["A-1".to_string(), "A-2".to_string()]]
        )));
        assert!(!cycle.conditions.iter().any(|condition| matches!(
            condition,
            GraphExplorerCondition::AggregateFeedback { .. }
        )));
    }

    #[test]
    fn graph_explorer_does_not_normalize_duplicate_task_edges() {
        let dir = fixture();
        std::fs::write(
            dir.path().join("aplan/tasks/A-2.md"),
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1, A-1]\n---\n",
        )
        .expect("duplicate edge");
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let projection = graph_workspace(&context, task_catalogue(&context))
            .project_explorer(&GraphExplorerRequest::default());
        assert_eq!(projection.graph.edges.len(), 2);
        assert!(
            projection
                .graph
                .edges
                .iter()
                .all(|edge| edge.dependency_count == 1 && edge.provenance.len() == 1)
        );
        assert_eq!(projection.accounting.admitted_dependency_edges, 2);
        assert_eq!(projection.accounting.represented_dependency_edges, 2);
    }

    fn linear_graph_explorer_projection(count: usize) -> GraphExplorerProjection {
        let seed = graph_fixture_workspace();
        let template = seed.tasks[0].clone();
        let row_template = seed.catalogue[0].clone();
        let mut tasks = Vec::with_capacity(count);
        let mut catalogue = Vec::with_capacity(count);
        for index in 0usize..count {
            let id = format!("L-{index:05}");
            let previous = index
                .checked_sub(1)
                .map(|previous| format!("L-{previous:05}"));
            let mut task = template.clone();
            task.id.clone_from(&id);
            task.title = format!("linear task {index}");
            task.phase = 1;
            task.status = Status::Todo;
            task.tags.clear();
            task.blocked_by = previous.clone().into_iter().collect();
            tasks.push(task);

            let mut row = row_template.clone();
            row.id = id;
            row.title = format!("linear task {index}");
            row.phase = 1;
            row.status = Status::Todo;
            row.owner = None;
            row.tags.clear();
            row.cut = None;
            row.milestone = None;
            row.ready_rank = (index == 0).then_some(1);
            row.rank_reason = None;
            row.body.clear();
            catalogue.push(row);
        }
        GraphWorkspace::new(seed.input, tasks, catalogue)
            .project_explorer(&GraphExplorerRequest::default())
    }

    fn complete_linear_scene_projection(count: usize) -> GraphExplorerProjection {
        let mut projection = linear_graph_explorer_projection(count);
        let template = projection.graph.nodes[0].clone();
        projection.graph.nodes = (0..count)
            .map(|index| {
                let id = format!("L-{index:05}");
                let mut node = template.clone();
                node.identity = GraphExplorerIdentity::Task(id.clone());
                node.title = format!("linear task {index}");
                node.represented_ids = vec![id.clone()];
                node.member_ids = vec![id];
                node
            })
            .collect();
        projection.graph.edges = (1..count)
            .map(|index| {
                let from = format!("L-{:05}", index - 1);
                let to = format!("L-{index:05}");
                GraphExplorerEdge {
                    from: GraphExplorerIdentity::Task(from.clone()),
                    to: GraphExplorerIdentity::Task(to.clone()),
                    disposition: GraphExplorerEdgeDisposition::Forward,
                    dependency_count: 1,
                    provenance: vec![GraphExplorerDependency { from, to }],
                }
            })
            .collect();
        projection
    }

    #[test]
    fn ten_thousand_node_multiscale_projection_is_linear_and_stack_safe() {
        let projection = linear_graph_explorer_projection(10_000);
        assert_eq!(projection.graph.nodes.len(), AGGREGATE_WINDOW_NODES);
        assert!(projection.graph.edges.len() <= AGGREGATE_WINDOW_EDGES);
        assert_eq!(projection.accounting.admitted_dependency_edges, 9_999);
        assert_eq!(projection.accounting.admitted_task_ids.len(), 10_000);
        assert_eq!(projection.accounting.window_omitted_task_ids.len(), 9_990);
        assert_eq!(
            projection
                .semantic_window
                .as_ref()
                .map(|window| (window.source_nodes, window.omitted_nodes)),
            Some((10_000, 9_990))
        );
        assert!(projection.conditions.iter().all(|condition| !matches!(
            condition,
            GraphExplorerCondition::InvalidTaskCycle { .. }
        )));
    }

    /// Proof for `tui.layered_cell_scene_is_bounded_and_deterministic`: a
    /// complete source above the scene budget is accounted without invoking
    /// the coordinate kernel.
    #[test]
    fn cell_scene_refuses_ten_thousand_nodes_before_coordinate_work() {
        let projection = complete_linear_scene_projection(10_000);
        let request = CellSceneRequest::new(projection);
        let CellSceneResult::Fallback(fallback) = CellSceneJob::new(1, request).run().result else {
            panic!("a complete-plan layout must be refused before the kernel")
        };
        assert!(matches!(
            fallback.reasons.as_slice(),
            [CellSceneDegradation::LimitExceeded {
                resource,
                got: 10_000,
                limit: 64,
            }] if resource == "nodes"
        ));
        assert_eq!(fallback.nodes.len(), 10_000);
        assert_eq!(fallback.edges.len(), 9_999);
        assert!(fallback.retained.is_none());
    }

    /// Proof for `show.carries_complete_task_projection`: duplicate ids are a
    /// finding, but the read model must not combine the first task's metadata
    /// with a later duplicate's body.
    #[test]
    fn duplicate_ids_use_one_consistent_task_and_body_projection() {
        let dir = fixture();
        std::fs::write(
            dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nFirst body.\n",
        )
        .expect("first task");
        std::fs::write(
            dir.path().join("aplan/tasks/Z-duplicate.md"),
            "---\nid: A-1\ntitle: duplicate\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nSecond body.\n",
        )
        .expect("duplicate task");
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let source_path = context.task_sources.get("A-1").expect("indexed source");
        let aplan_core::load::PlanFileContent::Bytes(source) = context
            .source_inputs
            .get(source_path)
            .expect("owned source bytes")
        else {
            panic!("task source was not readable")
        };
        let body = context.task_body("A-1").expect("body");
        let source_range = source.as_ptr_range();
        assert!(
            source_range.contains(&body.as_ptr()),
            "the body must borrow the context's one source buffer rather than a cloned String"
        );
        let graph_before = aplan_core::graph_constructions();
        let view = task_view(&context);
        assert_eq!(
            aplan_core::graph_constructions() - graph_before,
            1,
            "one interactive read snapshot must build one borrowed graph"
        );
        let matches: Vec<_> = view
            .inspectors
            .iter()
            .filter(|show| show.id == "A-1")
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].title, "first");
        assert_eq!(matches[0].body.as_deref(), Some("\nFirst body.\n"));
    }

    #[test]
    fn task_bodies_fail_closed_when_identified_inputs_are_inconsistent() {
        let dir = fixture();
        let root = dir.path().join("aplan");
        let first = root.join("tasks/A-1.md");
        let second = root.join("tasks/A-2.md");
        let inputs = || ContextInputs {
            repository_root: dir.path().to_path_buf(),
            configuration_file: Some(dir.path().join(".aplan.yml")),
            trust: Trust::Hermetic,
            options: Options::default(),
            verdict: Verdict::default(),
        };

        // `IdentifiedPlan` is a public application-boundary value. Even if a
        // caller constructs one that cannot come from the loader, the read
        // model must not panic or pair a task with unrelated source text.
        let mut missing = aplan_core::load_plan_identified(&root);
        missing.inputs.retain(|input| input.path != first);
        let missing = PlanContext::new(missing, inputs());
        assert_eq!(missing.task_body("A-1"), None);
        assert_eq!(missing.task_body("A-2"), Some(""));

        let mut unreadable = aplan_core::load_plan_identified(&root);
        let input = unreadable
            .inputs
            .iter_mut()
            .find(|input| input.path == first)
            .expect("first input");
        input.content = aplan_core::load::PlanFileContent::ReadError {
            kind: std::io::ErrorKind::PermissionDenied,
            message: "permission denied".to_string(),
        };
        let unreadable = PlanContext::new(unreadable, inputs());
        assert_eq!(unreadable.task_body("A-1"), None);

        let mut malformed = aplan_core::load_plan_identified(&root);
        let first_input = malformed
            .inputs
            .iter_mut()
            .find(|input| input.path == first)
            .expect("first input");
        first_input.content = aplan_core::load::PlanFileContent::Bytes(vec![0xff]);
        let second_input = malformed
            .inputs
            .iter_mut()
            .find(|input| input.path == second)
            .expect("second input");
        second_input.content =
            aplan_core::load::PlanFileContent::Bytes(b"not frontmatter".to_vec());
        let malformed = PlanContext::new(malformed, inputs());
        assert_eq!(malformed.task_body("A-1"), None);
        assert_eq!(malformed.task_body("A-2"), None);
    }

    /// Proof for `application.input_identity_matches_loaded_inputs`.
    #[test]
    fn a_result_identity_names_the_bytes_that_produced_it() {
        let dir = fixture();
        let root = dir.path().join("aplan");
        let loaded = aplan_core::load_plan_identified(&root);
        let task = root.join("tasks/A-2.md");
        let source = std::fs::read_to_string(&task).expect("task");
        std::fs::write(&task, source.replace("effort_days: 1\n", "")).expect("external edit");

        let stale_context = PlanContext::new(
            loaded,
            ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(dir.path().join(".aplan.yml")),
                trust: Trust::Hermetic,
                options: Options::default(),
                verdict: Verdict::default(),
            },
        );
        let old_result = check(&stale_context, CheckRequest);
        let current_context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let current_result = check(&current_context, CheckRequest);

        assert!(old_result.report.diagnostics.is_empty());
        assert_eq!(current_result.report.diagnostics[0].code, "APLAN009");
        assert_ne!(old_result.input.sha256, current_result.input.sha256);
    }

    /// Proof for `application.input_identity_matches_loaded_inputs`.
    #[test]
    fn a_non_task_markdown_candidate_is_part_of_the_input_identity() {
        let dir = fixture();
        let notes = dir.path().join("aplan/notes.md");
        std::fs::write(&notes, "ordinary prose before\n").expect("notes");
        let before = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        std::fs::write(&notes, "ordinary prose after\n").expect("edit notes");
        let after = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );

        assert_eq!(before.plan().tasks.len(), after.plan().tasks.len());
        assert_ne!(before.identity().sha256, after.identity().sha256);
    }

    #[test]
    fn check_carries_the_ordered_diagnostics_and_full_verdict() {
        let dir = fixture();
        let task = dir.path().join("aplan/tasks/A-2.md");
        let bytes = std::fs::read_to_string(&task).expect("read task");
        std::fs::write(&task, bytes.replace("effort_days: 1\n", "")).expect("change task");
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict {
                deny_warnings: true,
                advisory: Vec::new(),
            },
        );

        let result = check(&context, CheckRequest);
        assert!(result.denied);
        assert_eq!(result.report.tasks_checked, 2);
        assert_eq!(result.report.diagnostics.len(), 1);
        assert_eq!(result.report.diagnostics[0].code, "APLAN009");
        assert_eq!(result.report.diagnostics[0].task.as_deref(), Some("A-2"));
        assert_eq!(result.input, *context.identity());
    }

    /// Proof for `tui.overview_is_one_typed_projection` and
    /// `tui.overview_revisit_membership_is_exact`: every displayed aggregate
    /// carries the exact task identities behind it, including both sides of
    /// the clock-dependent revisit answer.
    #[test]
    fn overview_projects_counts_membership_due_and_findings_once() {
        let dir = fixture();
        std::fs::write(
            dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: finished\nphase: 1\nstatus: done\neffort_days: 2\n\
             evidence:\n- kind: note\n  ref: reviewed\n  captured: 2026-01-01\n---\n",
        )
        .expect("done task");
        std::fs::write(
            dir.path().join("aplan/tasks/A-2.md"),
            "---\nid: A-2\ntitle: parked\nphase: 1\nstatus: blocked\nblocked_by: [A-1]\n\
             revisit_at: 2026-01-02T00:00:00Z\n---\n",
        )
        .expect("due task");
        std::fs::write(
            dir.path().join("aplan/tasks/A-3.md"),
            "---\nid: A-3\ntitle: cancelled\nphase: 1\nstatus: dropped\neffort_days: 5\n---\n",
        )
        .expect("dropped task");
        std::fs::write(
            dir.path().join("aplan/tasks/A-4.md"),
            "---\nid: A-4\ntitle: revisit later\nphase: 1\nstatus: blocked\nblocked_by: [A-1]\n\
             revisit_at: 2026-01-04T00:00:00Z\neffort_days: 3\n---\n",
        )
        .expect("future revisit task");
        std::fs::write(
            dir.path().join("aplan/tasks/M-1.md"),
            "---\nid: M-1\ntitle: first cut\nphase: 1\nstatus: todo\n\
             blocked_by: [A-1, A-2, A-3, A-4]\ntags: [milestone, v0.1]\n---\n",
        )
        .expect("cut");
        let options = Options {
            require_effort: true,
            ..Options::default()
        };
        let context = test_context(
            &dir,
            Trust::Hermetic,
            options,
            Verdict {
                deny_warnings: true,
                advisory: Vec::new(),
            },
        );
        let as_of =
            aplan_core::validate::rfc3339_to_epoch("2026-01-03T00:00:00Z").expect("fixed time");
        let view = task_view_at(&context, as_of);
        let overview = &view.overview;

        assert_eq!(overview.input, view.check.input);
        assert_eq!(overview.load_state, OverviewLoadState::Complete);
        assert_eq!(overview.work_state, OverviewWorkState::Active);
        assert_eq!(overview.effort_state, OverviewEffortState::Partial);
        assert_eq!(overview.loaded_tasks, 5);
        assert_eq!(overview.loaded_work, 4);
        assert_eq!(overview.live_work, 3);
        assert_eq!(overview.done_work, 1);
        assert_eq!(overview.dropped_work, 1);
        assert_eq!(overview.live_task_ids, ["A-1", "A-2", "A-4"]);
        assert!(
            (overview.live_effort_days - 5.0).abs() < f64::EPSILON,
            "unexpected live effort: {}",
            overview.live_effort_days
        );
        assert!(
            (overview.done_effort_days - 2.0).abs() < f64::EPSILON,
            "unexpected done effort: {}",
            overview.done_effort_days
        );
        assert_eq!(overview.missing_effort_task_ids, ["A-2"]);
        assert_eq!(overview.cuts.len(), 1);
        assert_eq!(overview.cuts[0].task_ids, ["A-1", "A-2", "A-4"]);
        assert_eq!(overview.cut_task_ids, ["A-1", "A-2", "A-4"]);
        assert_eq!(overview.cut_work, 3);
        assert_eq!(overview.done_cut_work, 1);
        assert!(overview.ready.is_empty());
        assert_eq!(overview.due.len(), 1);
        assert_eq!(overview.due[0].id, "A-2");
        assert_eq!(overview.due[0].overdue_seconds, 86_400);
        assert_eq!(overview.waiting_revisits.len(), 1);
        assert_eq!(overview.waiting_revisits[0].id, "A-4");
        assert_eq!(overview.waiting_revisits[0].seconds_until, 86_400);
        let (days, path) = overview.critical_path.as_ref().expect("acyclic path");
        assert!(
            (*days - 5.0).abs() < f64::EPSILON,
            "unexpected critical-path effort: {days}"
        );
        assert_eq!(
            path.iter().map(|task| task.id.as_str()).collect::<Vec<_>>(),
            ["A-1", "A-4"]
        );
        assert_eq!(overview.findings.len(), view.check.report.diagnostics.len());
        assert_eq!(overview.errors, 0);
        assert_eq!(overview.warnings, overview.findings.len());
        assert!(overview.denied);
    }

    #[test]
    fn overview_revisits_ignore_inapplicable_values_and_order_every_tie() {
        let dir = fixture();
        let plan = aplan_core::load::load_plan(&dir.path().join("aplan"));
        let base = plan.tasks[0].task.clone();
        let task = |id: &str, status: Status, revisit_at: Option<&str>| {
            let mut task = base.clone();
            task.id = id.to_string();
            task.title = format!("task {id}");
            task.status = status;
            task.revisit_at = revisit_at.map(str::to_string);
            task
        };
        let tasks = vec![
            task("ignored-live", Status::Todo, Some("2026-01-01T00:00:00Z")),
            task("ignored-absent", Status::Blocked, None),
            task("ignored-invalid", Status::Blocked, Some("not-an-instant")),
            task("due-later", Status::Blocked, Some("2026-01-02T00:00:00Z")),
            task("due-b", Status::Blocked, Some("2026-01-01T00:00:00Z")),
            task("due-a", Status::Blocked, Some("2026-01-01T00:00:00Z")),
            task(
                "waiting-later",
                Status::Blocked,
                Some("2026-01-05T00:00:00Z"),
            ),
            task("waiting-b", Status::Blocked, Some("2026-01-04T00:00:00Z")),
            task("waiting-a", Status::Blocked, Some("2026-01-04T00:00:00Z")),
        ];
        let live = tasks.iter().collect::<Vec<_>>();
        let as_of =
            aplan_core::validate::rfc3339_to_epoch("2026-01-03T00:00:00Z").expect("fixed time");

        let (due, waiting) = overview_due(&live, as_of);
        assert_eq!(
            due.iter().map(|task| task.id.as_str()).collect::<Vec<_>>(),
            ["due-a", "due-b", "due-later"]
        );
        assert_eq!(
            waiting
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["waiting-a", "waiting-b", "waiting-later"]
        );
    }

    /// Proof for `tui.overview_is_one_typed_projection`: absence, malformed
    /// input and terminal work states remain distinguishable.
    #[test]
    fn overview_names_empty_all_done_no_effort_and_partial_load_states() {
        let empty = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(empty.path().join("aplan/tasks")).expect("tasks");
        std::fs::write(empty.path().join(".aplan.yml"), "").expect("config");
        let empty_context = test_context(
            &empty,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let empty_view = task_view_at(&empty_context, 0);
        assert_eq!(empty_view.overview.load_state, OverviewLoadState::Empty);
        assert_eq!(
            empty_view.overview.work_state,
            OverviewWorkState::NoLoadedWork
        );
        assert_eq!(
            empty_view.overview.effort_state,
            OverviewEffortState::NoLiveWork
        );

        let no_effort = fixture();
        for id in ["A-1", "A-2"] {
            let path = no_effort.path().join(format!("aplan/tasks/{id}.md"));
            let text = std::fs::read_to_string(&path).expect("task");
            std::fs::write(path, text.replace("effort_days: 1\n", "")).expect("remove effort");
        }
        let no_effort_context = test_context(
            &no_effort,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        assert_eq!(
            task_view_at(&no_effort_context, 0).overview.effort_state,
            OverviewEffortState::None
        );

        let all_done = fixture();
        for id in ["A-1", "A-2"] {
            let path = all_done.path().join(format!("aplan/tasks/{id}.md"));
            let text = std::fs::read_to_string(&path).expect("task");
            let done = text.replace("status: todo", "status: done").replace(
                "effort_days: 1\n---\n",
                "effort_days: 1\nevidence:\n- kind: note\n  ref: reviewed\n  captured: 2026-01-01\n---\n",
            );
            std::fs::write(path, done).expect("finish task");
        }
        let all_done_context = test_context(
            &all_done,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        assert_eq!(
            task_view_at(&all_done_context, 0).overview.work_state,
            OverviewWorkState::AllDone
        );

        let partial = fixture();
        std::fs::write(
            partial.path().join("aplan/tasks/A-broken.md"),
            "---\nid: A-broken\ntitle: broken\nphase: 1\nstatus: not-a-status\n---\n",
        )
        .expect("broken task");
        let partial_context = test_context(
            &partial,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let partial_view = task_view_at(&partial_context, 0);
        assert_eq!(
            partial_view.overview.load_state,
            OverviewLoadState::PartiallyUnloadable
        );
        assert_eq!(partial_view.overview.unloadable.len(), 1);
        assert_eq!(partial_view.overview.unloadable[0].0, "A-broken");
        assert!(!partial_view.overview.findings.is_empty());
    }

    #[test]
    fn a_wholly_malformed_plan_has_unavailable_structural_coverage() {
        let malformed = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(malformed.path().join("aplan/tasks")).expect("tasks");
        std::fs::write(
            malformed.path().join("aplan/tasks/broken.md"),
            "---\nid: [\n---\n",
        )
        .expect("malformed task");
        let context = test_context(
            &malformed,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let view = task_view_at(&context, 0);
        assert_eq!(view.overview.load_state, OverviewLoadState::Malformed);
        assert!(matches!(
            view.diagnostics.check.coverage,
            DiagnosticCoverage::Unavailable(_)
        ));
    }

    #[test]
    fn document_scope_refuses_to_judge_references_from_a_partial_plan() {
        let partial = fixture();
        std::fs::write(
            partial.path().join("aplan/tasks/A-broken.md"),
            "---\nid: A-broken\ntitle: broken\nphase: 1\nstatus: not-a-status\n---\n",
        )
        .expect("broken task");
        let context = test_context(
            &partial,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        )
        .with_documents(DocumentInputs {
            state: DocumentInputState::Configured,
            documents: vec![DocumentInput {
                label: "PLAN.md".to_string(),
                path: partial.path().join("PLAN.md"),
                content: Ok("A-1 and A-missing".to_string()),
                membership: None,
            }],
        });
        let run = task_view_at(&context, 0).diagnostics.documents;
        assert!(matches!(
            run.coverage,
            DiagnosticCoverage::Unavailable(ref reason)
                if reason.contains("plan has 1 load diagnostic")
        ));
        assert!(run.denied);
        assert!(
            run.findings.is_empty(),
            "a partial id set must not judge document references"
        );
    }

    fn diagnostic_fixture() -> (tempfile::TempDir, PathBuf, PlanContext) {
        let dir = fixture();
        let task = dir.path().join("aplan/tasks/A-2.md");
        let text = std::fs::read_to_string(&task).expect("task");
        std::fs::write(&task, text.replace("effort_days: 1\n", "")).expect("remove effort");
        std::fs::write(
            dir.path().join("aplan/tasks/M-1.md"),
            "---\nid: M-1\ntitle: first cut\nphase: 1\nstatus: todo\n\
             blocked_by: [A-1, A-2]\ntags: [milestone]\n---\n",
        )
        .expect("milestone");
        let document = dir.path().join("PLAN.md");
        let documents = DocumentInputs {
            state: DocumentInputState::Configured,
            documents: vec![DocumentInput {
                label: "PLAN.md".to_string(),
                path: document.clone(),
                content: Ok("# M-1\n\n**Tasks:** A-1\n\nA-9 is allegedly complete.\n".to_string()),
                membership: Some(DocumentMembership::Line("**Tasks:**".to_string())),
            }],
        };
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options {
                require_effort: true,
                ..Options::default()
            },
            Verdict::default(),
        )
        .with_documents(documents);
        (dir, document, context)
    }

    /// Proof for `tui.diagnostic_workbench_retains_complete_runs` and
    /// `tui.check_overrides_are_one_run`: a policy-only rerun changes the
    /// contribution, not the finding's identity, while a relaxed effort run is
    /// labelled and keeps the repository policy it will restore.
    #[test]
    fn diagnostic_workbench_preserves_scope_verdict_location_instruction_and_vacuity() {
        let (_dir, _document, context) = diagnostic_fixture();
        let baseline = task_view_at_with_overrides(&context, 0, &CheckOverrides::default());
        let effort = baseline
            .diagnostics
            .check
            .findings
            .iter()
            .find(|finding| finding.code == "APLAN009")
            .expect("effort finding");
        assert_eq!(effort.scope, DiagnosticScope::Check);
        assert_eq!(effort.subject, "task A-2");
        assert_eq!(effort.task.as_deref(), Some("A-2"));
        assert!(effort.file.is_some());
        assert!(effort.agent_instruction.is_some());
        assert_eq!(effort.line, None, "a missing field must not invent a line");
        assert_eq!(
            effort.source_line, None,
            "source context requires an exact line"
        );
        assert_eq!(effort.contribution, VerdictContribution::NonBlocking);
        assert!(!baseline.diagnostics.check.denied);

        let denied = task_view_at_with_overrides(
            &context,
            0,
            &CheckOverrides {
                warning_policy: WarningPolicyOverride::Deny,
                ..CheckOverrides::default()
            },
        );
        let denied_effort = denied
            .diagnostics
            .check
            .findings
            .iter()
            .find(|finding| finding.code == "APLAN009")
            .expect("effort finding after policy override");
        assert_eq!(denied_effort.identity, effort.identity);
        assert_eq!(denied_effort.contribution, VerdictContribution::Blocking);
        assert!(denied.diagnostics.check.denied);
        assert!(!denied.diagnostics.run_sheet.relaxed);

        let relaxed = task_view_at_with_overrides(
            &context,
            0,
            &CheckOverrides {
                no_require_effort: true,
                warning_policy: WarningPolicyOverride::Allow,
                ..CheckOverrides::default()
            },
        );
        assert!(relaxed.diagnostics.run_sheet.relaxed);
        assert!(relaxed.diagnostics.run_sheet.repository.require_effort);
        assert!(!relaxed.diagnostics.run_sheet.effective.require_effort);
        assert!(
            !relaxed
                .diagnostics
                .check
                .findings
                .iter()
                .any(|finding| finding.code == "APLAN009")
        );
    }

    #[test]
    fn document_findings_retain_exact_subject_location_source_and_instruction() {
        let (_dir, document, context) = diagnostic_fixture();
        let run = task_view_at(&context, 0).diagnostics.documents;
        let reference = run
            .findings
            .iter()
            .find(|finding| finding.code == "DOC-REFERENCE")
            .expect("document reference finding");
        assert_eq!(reference.scope, DiagnosticScope::CheckDocs);
        assert_eq!(reference.subject, "reference A-9");
        assert_eq!(reference.file.as_deref(), Some(document.as_path()));
        assert_eq!((reference.line, reference.column), (Some(5), Some(1)));
        assert_eq!(
            reference.source_line.as_deref(),
            Some("A-9 is allegedly complete.")
        );
        assert!(
            reference
                .agent_instruction
                .as_deref()
                .is_some_and(
                    |instruction| instruction.contains("A-9") && instruction.contains("PLAN.md")
                )
        );
        assert!(run.denied);

        let membership = run
            .findings
            .iter()
            .find(|finding| {
                finding.code == "DOC-MEMBERSHIP"
                    && finding.message.contains("document does not declare")
            })
            .expect("membership finding");
        assert_eq!(membership.task.as_deref(), Some("M-1"));
        assert_eq!(membership.subject, "milestone M-1");
        assert_eq!((membership.line, membership.column), (Some(1), Some(1)));
        assert_eq!(membership.source_line.as_deref(), Some("# M-1"));
        assert!(membership.message.contains("A-2"));
    }

    #[test]
    fn document_coverage_distinguishes_unconfigured_unreadable_and_partial_inputs() {
        let dir = fixture();
        let unconfigured = task_view_at(
            &test_context(
                &dir,
                Trust::Hermetic,
                Options::default(),
                Verdict::default(),
            ),
            0,
        );
        assert!(matches!(
            unconfigured.diagnostics.documents.coverage,
            DiagnosticCoverage::Unavailable(_)
        ));
        assert!(!unconfigured.diagnostics.documents.denied);

        let unreadable = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        )
        .with_documents(DocumentInputs {
            state: DocumentInputState::Configured,
            documents: vec![DocumentInput {
                label: "missing.md".to_string(),
                path: dir.path().join("missing.md"),
                content: Err("permission denied".to_string()),
                membership: None,
            }],
        });
        let unreadable_run = task_view_at(&unreadable, 0).diagnostics.documents;
        assert!(matches!(
            unreadable_run.coverage,
            DiagnosticCoverage::Unavailable(ref reason)
                if reason.contains("none of the configured documents")
        ));
        assert!(unreadable_run.denied);
        assert_eq!(unreadable_run.findings.len(), 1);
        assert_eq!(unreadable_run.findings[0].code, "DOC-INPUT");
        assert_eq!(unreadable_run.findings[0].subject, "document missing.md");

        let partial_documents = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        )
        .with_documents(DocumentInputs {
            state: DocumentInputState::Configured,
            documents: vec![
                DocumentInput {
                    label: "missing.md".to_string(),
                    path: dir.path().join("missing.md"),
                    content: Err("permission denied".to_string()),
                    membership: None,
                },
                DocumentInput {
                    label: "PLAN.md".to_string(),
                    path: dir.path().join("PLAN.md"),
                    content: Ok("A-1 is planned".to_string()),
                    membership: None,
                },
            ],
        });
        assert!(matches!(
            task_view_at(&partial_documents, 0)
                .diagnostics
                .documents
                .coverage,
            DiagnosticCoverage::Partial {
                subjects: 1,
                unavailable: 1
            }
        ));
    }

    #[test]
    fn document_coverage_refuses_a_plan_with_no_recognizable_subjects() {
        let empty = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(empty.path().join("aplan/tasks")).expect("tasks");
        let empty_context = test_context(
            &empty,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        )
        .with_documents(DocumentInputs {
            state: DocumentInputState::Configured,
            documents: vec![DocumentInput {
                label: "PLAN.md".to_string(),
                path: empty.path().join("PLAN.md"),
                content: Ok("no task vocabulary".to_string()),
                membership: None,
            }],
        });
        let vacuous = task_view_at(&empty_context, 0);
        assert!(matches!(
            vacuous.diagnostics.documents.coverage,
            DiagnosticCoverage::NoSubjects(_)
        ));
        assert!(vacuous.diagnostics.documents.denied);
    }

    #[test]
    fn rule_explanation_is_exact_and_document_bytes_extend_input_identity() {
        let dir = fixture();
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let first = context.identity().sha256.clone();
        let documents = DocumentInputs {
            state: DocumentInputState::Configured,
            documents: vec![DocumentInput {
                label: "PLAN.md".to_string(),
                path: dir.path().join("PLAN.md"),
                content: Ok("A-1".to_string()),
                membership: None,
            }],
        };
        let with_document = context.with_documents(documents.clone());
        assert_ne!(with_document.identity().sha256, first);
        let refreshed = aplan_core::load_plan_identified(&dir.path().join("aplan"));
        assert_eq!(
            with_document.identity_for(&refreshed).sha256,
            with_document.identity().sha256,
            "refreshing plan bytes must retain the identified document inputs"
        );
        let replacement = DocumentInputs {
            state: DocumentInputState::Configured,
            documents: vec![DocumentInput {
                label: "OTHER.md".to_string(),
                path: dir.path().join("OTHER.md"),
                content: Ok("A-2".to_string()),
                membership: None,
            }],
        };
        let replaced = with_document.with_documents(replacement.clone());
        let attached_once = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        )
        .with_documents(replacement);
        assert_eq!(
            replaced.identity().sha256,
            attached_once.identity().sha256,
            "reattaching documents replaces rather than chains the old document digest"
        );
        assert_eq!(
            explain_rule("APLAN009").expect("known rule").code,
            "APLAN009"
        );
        for unknown in ["aplan009", "APLAN009 ", "DOC-REFERENCE"] {
            let error = explain_rule(unknown).expect_err("lookup is exact");
            assert_eq!(error.name, unknown);
            assert!(error.known.contains("APLAN009"));
        }
    }

    #[test]
    fn repeated_membership_sections_name_every_heading_without_guessing_a_location() {
        let input = DocumentInput {
            label: "PLAN.md".to_string(),
            path: PathBuf::from("PLAN.md"),
            content: Ok("# M-1\n\n# M-1 again\n".to_string()),
            membership: Some(DocumentMembership::Table),
        };
        let mut findings = Vec::new();
        append_membership_findings(
            &mut findings,
            &input,
            input.content.as_ref().expect("text"),
            &aplan_interop::docs::MembershipProblem {
                milestone: "M-1".to_string(),
                undeclared: Vec::new(),
                unplanned: Vec::new(),
                no_declaration: false,
                repeated_section: true,
            },
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, None);
        assert_eq!(findings[0].column, None);
        assert_eq!(findings[0].source_line, None);
        assert_eq!(
            findings[0].detail,
            ["matching section headings are at lines 1, 3"]
        );
    }

    #[test]
    fn document_selection_and_every_membership_discrepancy_remain_explicit() {
        let dir = fixture();
        let unavailable = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        )
        .with_documents(DocumentInputs {
            state: DocumentInputState::Unavailable("glob matched no files".to_string()),
            documents: Vec::new(),
        });
        let unavailable_run = task_view_at(&unavailable, 0).diagnostics.documents;
        assert_eq!(unavailable_run.input_label, "configured document selection");
        assert!(matches!(
            unavailable_run.coverage,
            DiagnosticCoverage::Unavailable(ref reason) if reason == "glob matched no files"
        ));
        assert!(unavailable_run.denied);

        let configured_empty = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        )
        .with_documents(DocumentInputs {
            state: DocumentInputState::Configured,
            documents: Vec::new(),
        });
        let empty_run = task_view_at(&configured_empty, 0).diagnostics.documents;
        assert!(matches!(
            empty_run.coverage,
            DiagnosticCoverage::NoSubjects(ref reason)
                if reason.contains("no readable documents")
        ));
        assert!(!empty_run.denied);

        let input = DocumentInput {
            label: "ROADMAP.md".to_string(),
            path: PathBuf::from("ROADMAP.md"),
            content: Ok("# M-1\n".to_string()),
            membership: Some(DocumentMembership::Table),
        };
        let mut findings = Vec::new();
        append_membership_findings(
            &mut findings,
            &input,
            input.content.as_ref().expect("text"),
            &aplan_interop::docs::MembershipProblem {
                milestone: "M-1".to_string(),
                undeclared: vec!["A-1".to_string(), "A-2".to_string()],
                unplanned: vec!["A-9".to_string()],
                no_declaration: true,
                repeated_section: false,
            },
        );
        assert_eq!(findings.len(), 3);
        assert!(findings.iter().all(|finding| {
            finding.subject == "milestone M-1"
                && finding.task.as_deref() == Some("M-1")
                && finding.line == Some(1)
                && finding.column == Some(1)
                && finding.source_line.as_deref() == Some("# M-1")
                && finding.agent_instruction.is_some()
        }));
        assert!(findings.iter().any(|finding| {
            finding.message.contains("section declares none")
                && finding
                    .agent_instruction
                    .as_deref()
                    .is_some_and(|instruction| instruction.contains("Add the configured"))
        }));
        assert!(findings.iter().any(|finding| {
            finding.message.contains("`A-1`, `A-2`")
                && finding
                    .agent_instruction
                    .as_deref()
                    .is_some_and(|instruction| instruction.contains("remove them from the cut"))
        }));
        assert!(findings.iter().any(|finding| {
            finding.message.contains("`A-9`")
                && finding
                    .agent_instruction
                    .as_deref()
                    .is_some_and(|instruction| instruction.contains("add them to `M-1`"))
        }));
    }

    #[test]
    fn dependency_changeset_preview_is_exact_and_commit_publishes_the_same_bytes() {
        let dir = fixture();
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let path = dir.path().join("aplan/tasks/A-2.md");
        let before = std::fs::read_to_string(&path).expect("before");
        let request = DependencyRequest {
            id: "A-2".to_string(),
            after: "A-1".to_string(),
            operation: DependencyOperation::Add,
        };

        let preview = preview_dependency(&context, &request).expect("preview");
        assert_eq!(preview.request(), &request);
        assert_eq!(preview.target(), path);
        let change = preview.change_set();
        assert_eq!(change.input(), context.identity());
        assert_eq!(change.disposition(), ChangeDisposition::Ready);
        assert!(change.diagnostics().added.is_empty());
        assert_eq!(change.evidence(), &ChangeEvidence::NotRequired);
        assert_eq!(change.files().len(), 1);
        assert_eq!(change.files()[0].path(), path);
        assert_eq!(change.files()[0].before(), Some(before.as_str()));
        let after = change.files()[0].after().to_string();
        assert_eq!(
            aplan_core::mutation::task_of(&after)
                .expect("parse previewed task")
                .blocked_by,
            ["A-1"]
        );
        assert_eq!(
            change.effects(),
            [SemanticEffect::DependencyAdded {
                task: "A-2".to_string(),
                after: "A-1".to_string(),
            }]
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("dry run did not write"),
            before
        );

        let outcome = commit_change_set(&context, preview.into_change_set());
        assert_eq!(outcome.change_set().files()[0].after(), after);
        let CommitOutcome::Applied(committed) = outcome else {
            panic!("ready preview was not applied: {outcome:?}");
        };
        assert_eq!(committed.files()[0].after(), after);
        assert_eq!(
            std::fs::read_to_string(&path).expect("committed task"),
            after,
            "the guarded writer did not publish the exact previewed bytes"
        );

        let reloaded = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let remove = DependencyRequest {
            operation: DependencyOperation::Remove,
            ..request
        };
        let preview = preview_dependency(&reloaded, &remove).expect("remove preview");
        assert_eq!(
            preview.change_set().effects(),
            [SemanticEffect::DependencyRemoved {
                task: "A-2".to_string(),
                after: "A-1".to_string(),
            }]
        );
        assert_eq!(
            preview.change_set().files()[0].before(),
            Some(after.as_str())
        );
        assert_eq!(preview.change_set().files()[0].after(), before);
        assert!(matches!(
            commit_dependency(&reloaded, preview),
            CommitOutcome::Applied(_)
        ));
        assert_eq!(
            std::fs::read_to_string(path).expect("task after removal"),
            before,
            "removal did not publish the exact previewed bytes"
        );
    }

    #[test]
    fn dependency_changeset_distinguishes_noop_denied_and_stale_without_writing() {
        let noop_dir = fixture();
        let noop_context = test_context(
            &noop_dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let request = DependencyRequest {
            id: "A-2".to_string(),
            after: "A-1".to_string(),
            operation: DependencyOperation::Add,
        };
        edit_dependency(&noop_context, &request).expect("seed edge");
        let noop_context = test_context(
            &noop_dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let noop_path = noop_dir.path().join("aplan/tasks/A-2.md");
        let noop_before = std::fs::read_to_string(&noop_path).expect("before no-op");
        let noop = preview_dependency(&noop_context, &request).expect("no-op preview");
        assert_eq!(noop.change_set().disposition(), ChangeDisposition::NoOp);
        let outcome = commit_dependency(&noop_context, noop);
        assert_eq!(outcome.change_set().disposition(), ChangeDisposition::NoOp);
        assert!(matches!(outcome, CommitOutcome::NoOp(_)));
        assert_eq!(
            std::fs::read_to_string(noop_path).expect("after no-op"),
            noop_before
        );

        let denied_dir = fixture();
        let denied_context = test_context(
            &denied_dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        edit_dependency(&denied_context, &request).expect("seed cycle direction");
        let denied_context = test_context(
            &denied_dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let denied_path = denied_dir.path().join("aplan/tasks/A-1.md");
        let denied_before = std::fs::read_to_string(&denied_path).expect("before denial");
        let denied = preview_dependency(
            &denied_context,
            &DependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
            },
        )
        .expect("denied preview");
        assert_eq!(denied.change_set().disposition(), ChangeDisposition::Denied);
        assert_eq!(
            denied
                .change_set()
                .diagnostics()
                .added
                .iter()
                .map(|finding| finding.code)
                .collect::<Vec<_>>(),
            ["APLAN004"]
        );
        let outcome = commit_dependency(&denied_context, denied);
        assert_eq!(
            outcome.change_set().disposition(),
            ChangeDisposition::Denied
        );
        assert!(matches!(outcome, CommitOutcome::Denied(_)));
        assert_eq!(
            std::fs::read_to_string(denied_path).expect("after denial"),
            denied_before
        );

        let stale_dir = fixture();
        let stale_context = test_context(
            &stale_dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let stale = preview_dependency(&stale_context, &request).expect("stale preview");
        let stale_path = stale_dir.path().join("aplan/tasks/A-2.md");
        let intervening = format!(
            "{}\nintervening body edit\n",
            std::fs::read_to_string(&stale_path).expect("stale target")
        );
        std::fs::write(&stale_path, &intervening).expect("intervening writer");
        let outcome = commit_dependency(&stale_context, stale);
        assert_eq!(outcome.change_set().files().len(), 1);
        let CommitOutcome::Stale { change, actual } = outcome else {
            panic!("stale preview was not refused as stale: {outcome:?}");
        };
        assert_ne!(actual, *change.input());
        assert_eq!(
            std::fs::read_to_string(stale_path).expect("after stale refusal"),
            intervening
        );
    }

    #[test]
    fn dependency_edit_adds_removes_and_reports_idempotent_requests() {
        let dir = fixture();
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let add = DependencyRequest {
            id: "A-2".to_string(),
            after: "A-1".to_string(),
            operation: DependencyOperation::Add,
        };
        let added = edit_dependency(&context, &add).expect("add edge");
        assert!(added.changed);
        assert!(!added.denied);
        assert!(added.diagnostics.added.is_empty());
        let task = std::fs::read_to_string(dir.path().join("aplan/tasks/A-2.md")).expect("task");
        assert_eq!(
            aplan_core::mutation::task_of(&task)
                .expect("parse written task")
                .blocked_by,
            ["A-1"]
        );

        let reloaded = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let repeated = edit_dependency(&reloaded, &add).expect("repeat edge");
        assert!(!repeated.changed);
        assert!(!repeated.denied);

        let remove = DependencyRequest {
            operation: DependencyOperation::Remove,
            ..add
        };
        let removed = edit_dependency(&reloaded, &remove).expect("remove edge");
        assert!(removed.changed);
        let task = std::fs::read_to_string(dir.path().join("aplan/tasks/A-2.md")).expect("task");
        assert!(
            aplan_core::mutation::task_of(&task)
                .expect("parse written task")
                .blocked_by
                .is_empty()
        );
    }

    /// Proof for `dep.rebases_on_current_target`.
    #[test]
    fn a_dependency_edit_preserves_an_intervening_valid_edge() {
        let dir = fixture();
        std::fs::write(
            dir.path().join("aplan/tasks/A-3.md"),
            "---\nid: A-3\ntitle: third\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("third");
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let path = dir.path().join("aplan/tasks/A-2.md");
        let original = std::fs::read_to_string(&path).expect("target");
        let intervening =
            aplan_core::close::set_blocked_by(&original, &["A-3".to_string()]).expect("edit");
        std::fs::write(&path, intervening).expect("intervening writer");

        let response = edit_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
            },
        )
        .expect("merge the requested edge");

        let written = std::fs::read_to_string(path).expect("written task");
        assert_eq!(
            aplan_core::mutation::task_of(&written)
                .expect("parse written task")
                .blocked_by,
            ["A-3", "A-1"]
        );
        assert!(response.changed);
        assert_ne!(response.input.sha256, context.identity().sha256);
    }

    #[test]
    fn a_denied_dependency_edit_returns_its_diagnostics_and_writes_nothing() {
        let dir = fixture();
        let initial = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        edit_dependency(
            &initial,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
            },
        )
        .expect("seed edge");

        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let path = dir.path().join("aplan/tasks/A-1.md");
        let before = std::fs::read_to_string(&path).expect("before");
        let denied = edit_dependency(
            &context,
            &DependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
            },
        )
        .expect("rule verdict");
        assert!(denied.denied);
        assert!(!denied.changed);
        assert!(!denied.diagnostics.added.is_empty());
        assert_eq!(
            denied
                .diagnostics
                .added
                .iter()
                .map(|finding| finding.code)
                .collect::<Vec<_>>(),
            vec!["APLAN004"]
        );
        assert_eq!(std::fs::read_to_string(path).expect("after"), before);
    }

    #[test]
    fn dependency_arguments_are_typed_failures_and_never_write() {
        let dir = fixture();
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let self_edge = edit_dependency(
            &context,
            &DependencyRequest {
                id: "A-1".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
            },
        );
        assert!(matches!(self_edge, Err(Error::Arguments(_))));
        let unknown = edit_dependency(
            &context,
            &DependencyRequest {
                id: "missing".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
            },
        );
        assert!(matches!(unknown, Err(Error::Arguments(_))));

        std::fs::write(
            dir.path().join("aplan/tasks/M-1.md"),
            "---\nid: M-1\ntitle: cut\nphase: 1\nstatus: todo\nblocked_by: [A-1]\ntags: [milestone]\n---\n",
        )
        .expect("milestone");
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let membership = preview_dependency(
            &context,
            &DependencyRequest {
                id: "M-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
            },
        );
        assert!(matches!(membership, Err(Error::Refused(_))));
        let task = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("task");
        assert!(!task.contains("blocked_by"), "{task}");
    }

    #[test]
    fn an_impossible_dependency_rewrite_retains_the_core_error_type() {
        let dir = fixture();
        std::fs::write(
            dir.path().join("aplan/tasks/A-3.md"),
            "---\nid: A-3\ntitle: third\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("third");
        let path = dir.path().join("aplan/tasks/A-2.md");
        let before = "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\neffort_days: 1\n\
                      blocked_by: &edges\n  - A-1\ntags: *edges\n---\n";
        std::fs::write(&path, before).expect("anchored task");
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );

        let result = edit_dependency(
            &context,
            &DependencyRequest {
                id: "A-2".to_string(),
                after: "A-3".to_string(),
                operation: DependencyOperation::Add,
            },
        );

        assert!(matches!(
            result,
            Err(Error::Rewrite {
                source: aplan_core::CloseError::NoSequenceDelta("blocked_by"),
                ..
            })
        ));
        assert_eq!(std::fs::read_to_string(path).expect("task"), before);
    }

    fn browser_row(
        id: &str,
        title: &str,
        status: Status,
        owner: Option<&str>,
        tags: &[&str],
        rank: Option<usize>,
    ) -> TaskBrowserRow {
        TaskBrowserRow {
            id: id.to_string(),
            title: title.to_string(),
            body: String::new(),
            phase: if id == "A-2" { 2 } else { 1 },
            status,
            owner: owner.map(str::to_string),
            claim: None,
            coordination: aplan_core::ClaimScope::git_common_directory("test".to_string()),
            claim_observed_at_epoch_seconds: 0,
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            cut: Some(if id == "A-2" { "later" } else { "now" }.to_string()),
            milestone: Some(if id == "A-2" { "M-later" } else { "M-now" }.to_string()),
            ready_rank: rank,
            rank_reason: rank.map(|_| "releases 1".to_string()),
        }
    }

    /// Proof for `tui.task_browser_preserves_selector_semantics`.
    #[test]
    fn task_browser_preserves_selector_semantics_and_ready_rank() {
        let rows = vec![
            browser_row(
                "A-2",
                "later task",
                Status::InProgress,
                Some("lee"),
                &["ui", "urgent"],
                Some(1),
            ),
            browser_row("A-1", "first task", Status::Todo, None, &["ui"], Some(2)),
            browser_row(
                "A-3",
                "finished",
                Status::Done,
                Some("lee"),
                &["docs"],
                None,
            ),
        ];
        let response = browse_tasks(
            &rows,
            &TaskBrowserRequest {
                selector: Selector {
                    cuts: vec!["M-later".to_string()],
                    phases: vec![2],
                    tags: vec!["ui".to_string(), "another".to_string()],
                    owners: vec!["lee".to_string()],
                    statuses: vec![Status::Todo, Status::InProgress],
                    unclaimed: false,
                },
                ..TaskBrowserRequest::default()
            },
        );
        assert_eq!(
            response
                .rows
                .iter()
                .map(|row| row.task.id.as_str())
                .collect::<Vec<_>>(),
            ["A-2"]
        );
        assert_eq!(
            response.rows[0].task.rank_reason.as_deref(),
            Some("releases 1")
        );

        let all = browse_tasks(&rows, &TaskBrowserRequest::default());
        assert_eq!(
            all.rows
                .iter()
                .map(|row| row.task.id.as_str())
                .collect::<Vec<_>>(),
            ["A-2", "A-1", "A-3"]
        );
    }

    /// Proof for `tui.task_inspector_uses_show_projection`: body
    /// search joins id and title search without inventing unsafe highlight spans.
    #[test]
    fn task_browser_search_returns_unicode_safe_case_insensitive_spans() {
        let rows = vec![browser_row(
            "A-1",
            "CAF\u{c9} \u{130}dea",
            Status::Todo,
            None,
            &[],
            Some(1),
        )];
        let response = browse_tasks(
            &rows,
            &TaskBrowserRequest {
                query: "f\u{e9}".to_string(),
                ..TaskBrowserRequest::default()
            },
        );
        let span = response.rows[0].title_matches[0];
        assert_eq!(
            &response.rows[0].task.title[span.start..span.end],
            "F\u{c9}"
        );

        let expanded = browse_tasks(
            &rows,
            &TaskBrowserRequest {
                query: "i\u{307}".to_string(),
                ..TaskBrowserRequest::default()
            },
        );
        let span = expanded.rows[0].title_matches[0];
        assert_eq!(
            &expanded.rows[0].task.title[span.start..span.end],
            "\u{130}"
        );
        assert!(expanded.rows[0].task.title.is_char_boundary(span.start));
        assert!(expanded.rows[0].task.title.is_char_boundary(span.end));

        let mut body_row = rows[0].clone();
        body_row.body = "A buried implementation detail".to_string();
        let body = browse_tasks(
            &[body_row],
            &TaskBrowserRequest {
                query: "implementation".to_string(),
                ..TaskBrowserRequest::default()
            },
        );
        assert!(body.rows[0].body_matches);
        assert!(body.rows[0].id_matches.is_empty());
        assert!(body.rows[0].title_matches.is_empty());
    }

    /// Proof for `show.carries_complete_task_projection`: the
    /// application returns the canonical typed `Show` from one identified set
    /// of bytes, including relationships that did not load.
    #[test]
    fn task_inspectors_are_complete_show_projections_of_the_identified_input() {
        let dir = fixture();
        let task = dir.path().join("aplan/tasks/A-2.md");
        std::fs::write(
            &task,
            concat!(
                "---\n",
                "id: A-2\n",
                "title: second\n",
                "phase: 1\n",
                "status: todo\n",
                "effort_days: 2\n",
                "blocked_by: [A-1, A-3, GHOST]\n",
                "prefer_after: [A-1]\n",
                "ratified_by: [A-1]\n",
                "contains: [A-1]\n",
                "paths: ['src/**']\n",
                "traces: ['DOC.md#intro']\n",
                "trace_fingerprints:\n",
                "  DOC.md#intro: trace-stamp\n",
                "evidence:\n",
                "- kind: note\n",
                "  ref: reviewed\n",
                "  captured: 2026-10-15\n",
                "evidence_fingerprints:\n",
                "  'file:proof.md': evidence-stamp\n",
                "version_impact: breaking\n",
                "version_impact_reason: explicit contract break\n",
                "---\n\n## Why\n\nOriginal body.\n",
            ),
        )
        .expect("rich task");
        std::fs::write(
            dir.path().join("aplan/tasks/A-3.md"),
            "---\nid: A-3\ntitle: [\nphase: 1\nstatus: todo\n---\n",
        )
        .expect("unloadable task");
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        std::fs::write(&task, "changed after the identified load\n").expect("external edit");

        let view = task_view(&context);
        let show = view
            .inspectors
            .iter()
            .find(|show| show.id == "A-2")
            .unwrap_or_else(|| {
                panic!(
                    "A-2 inspector; diagnostics: {:?}",
                    context.plan().load_diagnostics
                )
            });
        assert_eq!(show.body.as_deref(), Some("\n## Why\n\nOriginal body.\n"));
        assert_eq!(show.paths, ["src/**"]);
        assert_eq!(show.traces, ["DOC.md#intro"]);
        assert_eq!(
            show.trace_fingerprints
                .get("DOC.md#intro")
                .map(String::as_str),
            Some("trace-stamp")
        );
        assert_eq!(show.evidence[0].kind, "note");
        assert_eq!(show.evidence[0].reference, "reviewed");
        assert_eq!(
            show.evidence_fingerprints
                .get("file:proof.md")
                .map(String::as_str),
            Some("evidence-stamp")
        );
        assert_eq!(show.prefer_after[0].id, "A-1");
        assert_eq!(show.ratified_by[0].id, "A-1");
        assert_eq!(show.contains[0].id, "A-1");
        assert_eq!(
            show.version_impact,
            Some(aplan_core::task::VersionImpact::Breaking)
        );
        assert_eq!(
            show.version_impact_reason.as_deref(),
            Some("explicit contract break")
        );
        assert_eq!(
            show.waits_on
                .iter()
                .map(|related| (related.id.as_str(), related.availability))
                .collect::<Vec<_>>(),
            [
                ("A-1", RelatedAvailability::Loaded),
                ("A-3", RelatedAvailability::Unloadable),
                ("GHOST", RelatedAvailability::Missing),
            ]
        );
        assert!(
            show.waits_on[1]
                .source
                .as_deref()
                .is_some_and(|source| source.ends_with("A-3.md"))
        );
        assert!(!show.resolved);
        assert!(
            show.findings
                .iter()
                .any(|finding| finding.message.contains("GHOST") || finding.message.contains("A-3"))
        );
    }

    #[test]
    fn task_browser_distinguishes_empty_filter_and_search_results() {
        let rows = vec![browser_row(
            "A-1",
            "first",
            Status::Todo,
            Some("kim"),
            &[],
            Some(1),
        )];
        let responsible_but_inactive = browse_tasks(
            &rows,
            &TaskBrowserRequest {
                selector: Selector {
                    owners: vec!["kim".to_string()],
                    unclaimed: true,
                    ..Selector::default()
                },
                ..TaskBrowserRequest::default()
            },
        );
        assert_eq!(responsible_but_inactive.rows.len(), 1);
        assert_eq!(responsible_but_inactive.rows[0].task.id, "A-1");
        let mut unavailable = rows.clone();
        unavailable[0].coordination =
            aplan_core::ClaimScope::unavailable("not inside a Git repository");
        let refused_unclaimed = browse_tasks(
            &unavailable,
            &TaskBrowserRequest {
                selector: Selector {
                    unclaimed: true,
                    ..Selector::default()
                },
                ..TaskBrowserRequest::default()
            },
        );
        assert_eq!(
            refused_unclaimed.no_results,
            Some(NoTaskResults::CoordinationUnavailable)
        );
        let no_filter = browse_tasks(
            &rows,
            &TaskBrowserRequest {
                selector: Selector {
                    statuses: vec![Status::Done],
                    ..Selector::default()
                },
                ..TaskBrowserRequest::default()
            },
        );
        assert_eq!(no_filter.no_results, Some(NoTaskResults::NoFilterMatches));
        let no_search = browse_tasks(
            &rows,
            &TaskBrowserRequest {
                query: "absent".to_string(),
                ..TaskBrowserRequest::default()
            },
        );
        assert_eq!(no_search.no_results, Some(NoTaskResults::NoSearchMatches));
    }

    #[test]
    fn task_filter_choices_and_sorts_are_complete_and_deterministic() {
        let rows = vec![
            browser_row("A-2", "beta", Status::Todo, Some("lee"), &["ui"], None),
            browser_row("A-1", "alpha", Status::Done, None, &["docs"], Some(1)),
            browser_row(
                "A-3",
                "literal owner",
                Status::Todo,
                Some("nobody"),
                &[],
                None,
            ),
        ];
        let choices = task_filter_choices(&rows);
        assert_eq!(
            choices.iter().map(TaskFilter::label).collect::<Vec<_>>(),
            [
                "status: todo",
                "status: done",
                "cut: later",
                "cut: now",
                "phase: 1",
                "phase: 2",
                "unclaimed (no active claim)",
                "owner: lee",
                "owner: nobody",
                "tag: docs",
                "tag: ui",
            ]
        );
        let mut empty_owner = rows[0].clone();
        empty_owner.owner = Some(String::new());
        let mut duplicate = empty_owner.clone();
        duplicate.id = "A-4".to_string();
        let mut odd_rows = rows.clone();
        odd_rows.extend([empty_owner, duplicate]);
        assert_eq!(
            task_filter_choices(&odd_rows)
                .iter()
                .filter(|choice| matches!(choice, TaskFilter::Owner(owner) if owner.is_empty()))
                .count(),
            1,
            "equal sort keys must not separate duplicate filter choices"
        );
        let mut request = TaskBrowserRequest::default();
        for sort in TaskSort::ALL {
            request.sort = sort;
            let first = browse_tasks(&rows, &request);
            let second = browse_tasks(&rows, &request);
            assert_eq!(
                first.rows,
                second.rows,
                "{} sort must be stable",
                sort.label()
            );
        }
    }

    #[test]
    fn task_browser_vocabulary_and_controls_are_executable() {
        assert_eq!(
            TaskSort::ALL.map(TaskSort::label),
            [
                "ready rank",
                "id",
                "title",
                "status",
                "cut",
                "phase",
                "owner"
            ]
        );
        let mut sort = TaskSort::ReadyRank;
        for expected in [
            TaskSort::Id,
            TaskSort::Title,
            TaskSort::Status,
            TaskSort::Cut,
            TaskSort::Phase,
            TaskSort::Owner,
            TaskSort::ReadyRank,
        ] {
            sort = sort.next();
            assert_eq!(sort, expected);
        }
        assert_eq!(
            [
                NoTaskResults::EmptyPlan,
                NoTaskResults::EmptyScope,
                NoTaskResults::CoordinationUnavailable,
                NoTaskResults::NoFilterMatches,
                NoTaskResults::NoSearchMatches,
            ]
            .map(NoTaskResults::message),
            [
                "The plan contains no tasks.",
                "This aggregate contains no tasks.",
                "Unclaimed work cannot be selected because local claim coordination is unavailable.",
                "No tasks match the active filters.",
                "No filtered tasks match the search text.",
            ]
        );

        let controls = [
            TaskFilter::Status(Status::Todo),
            TaskFilter::Cut("now".to_string()),
            TaskFilter::Phase(1),
            TaskFilter::Owner("nobody".to_string()),
            TaskFilter::Unclaimed,
            TaskFilter::Tag("ui".to_string()),
        ];
        let mut selector = Selector {
            statuses: vec![Status::Done],
            ..Selector::default()
        };
        for control in &controls {
            assert!(
                !control.selected(&selector),
                "{} starts clear",
                control.label()
            );
            control.toggle(&mut selector);
            assert!(
                control.selected(&selector),
                "{} toggles on",
                control.label()
            );
            control.toggle(&mut selector);
            assert!(
                !control.selected(&selector),
                "{} toggles off",
                control.label()
            );
        }
    }

    #[test]
    fn task_browser_catalogue_empty_plan_and_repeated_matches_are_executable() {
        let dir = fixture();
        let context = test_context(
            &dir,
            Trust::Hermetic,
            Options::default(),
            Verdict::default(),
        );
        let catalogue = task_catalogue(&context);
        assert_eq!(
            catalogue
                .iter()
                .map(|row| (row.id.as_str(), row.ready_rank))
                .collect::<Vec<_>>(),
            [("A-1", Some(1)), ("A-2", Some(2))]
        );
        assert!(catalogue.iter().all(|row| row.rank_reason.is_some()));

        let empty = browse_tasks(&[], &TaskBrowserRequest::default());
        assert_eq!(empty.no_results, Some(NoTaskResults::EmptyPlan));
        let repeated = browse_tasks(
            &[browser_row(
                "A-1",
                "Alpha alpha",
                Status::Todo,
                None,
                &[],
                Some(1),
            )],
            &TaskBrowserRequest {
                query: "ALPHA".to_string(),
                ..TaskBrowserRequest::default()
            },
        );
        assert_eq!(repeated.rows[0].title_matches.len(), 2);
    }

    #[test]
    fn task_browser_large_plan_latency_baseline() {
        let rows: Vec<_> = (0..10_000)
            .map(|index| {
                let mut row = browser_row(
                    &format!("A-{index:05}"),
                    &format!("generated task number {index}"),
                    if index % 3 == 0 {
                        Status::Todo
                    } else {
                        Status::InProgress
                    },
                    (index % 2 == 0).then_some("owner"),
                    &[if index % 5 == 0 { "ui" } else { "core" }],
                    (index < 100).then_some(index + 1),
                );
                row.body = format!("Detailed implementation record {index}");
                row
            })
            .collect();
        let request = TaskBrowserRequest {
            query: "implementation record 99".to_string(),
            selector: Selector {
                statuses: vec![Status::Todo, Status::InProgress],
                tags: vec!["ui".to_string()],
                ..Selector::default()
            },
            sort: TaskSort::Title,
        };
        let started = std::time::Instant::now();
        let response = browse_tasks(&rows, &request);
        let elapsed = started.elapsed();
        assert!(!response.rows.is_empty());
        eprintln!("10,000 task browser rows: {elapsed:?}");
    }
}

#[cfg(test)]
mod verification_error_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn verification_errors_keep_provider_execution_distinct_from_repository_failures() {
        let provider = VerificationError::from(aplan_evidence::Error::Exec(
            "configured `test.run` failed".into(),
        ));
        assert!(matches!(provider, VerificationError::Provider(_)));
        assert!(
            provider
                .to_string()
                .contains("configured verification provider")
        );

        let repository = VerificationError::from(aplan_evidence::Error::NotARepository(
            PathBuf::from("not-a-repository"),
        ));
        assert!(matches!(repository, VerificationError::Repository(_)));
        assert!(
            repository
                .to_string()
                .contains("repository verification failed")
        );
    }
}
