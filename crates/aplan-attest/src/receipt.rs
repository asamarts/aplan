//! Portable, provider-scoped test-evidence receipts.
//!
//! A receipt records one complete configured provider observation. It does not
//! record or bless a plan verdict: a consumer recomputes that verdict from the
//! exact current plan. Authentication is likewise transport metadata supplied
//! by the consumer, never a boolean trusted from the receipt itself.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{ToolIdentity, sha256_hex};
use aplan_config::TestRunner;
use aplan_evidence::{
    Repo, TestCatalogueRunnerState, TestExecutionObservation, TestMaterialIdentity,
    validate_result_entry,
};

/// The in-toto Statement major emitted and accepted by this build.
pub const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
/// The aplan predicate major emitted and accepted by this build.
pub const PREDICATE_TYPE: &str = "https://aplan.dev/attestation/test-evidence/v1";
/// The typed predicate schema discriminator.
pub const PREDICATE_SCHEMA: &str = "aplan/test-evidence/v1";
/// The receipt-set manifest major accepted by current-only verification.
pub const IMPORT_SET_SCHEMA: &str = "aplan/test-evidence-import-set/v1";
/// DSSE payload type for an in-toto Statement.
pub const DSSE_PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";
/// Maximum raw statement or envelope size accepted at the trust boundary.
pub const MAX_RECEIPT_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

/// Why a receipt could not be built, persisted or trusted.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReceiptError {
    #[error("invalid test-evidence receipt: {0}")]
    Invalid(String),
    #[error("test-evidence receipt identity mismatch: {0}")]
    Identity(String),
    #[error("test-evidence receipt trust policy was not satisfied: {0}")]
    Trust(String),
    #[error("test-evidence receipt I/O failed: {0}")]
    Io(String),
}

/// Parse, current-identity and authentication costs at the receipt boundary.
/// These durations are telemetry only; no timeout or trust decision reads them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReceiptVerificationCost {
    pub parsing_micros: u128,
    pub identity_micros: u128,
    pub authentication_micros: u128,
}

/// One in-toto subject descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceDescriptor {
    pub name: String,
    pub digest: BTreeMap<String, String>,
}

/// Repository state retained for audit, but not used as a substitute for the
/// exact material identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryContext {
    pub commit: String,
    pub tree_sha256: String,
    pub modified: usize,
    pub untracked: usize,
    pub status_sha256: String,
}

/// Whether the workflow was allowed to publish privileged evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProducerPrivilege {
    Trusted,
    Untrusted,
}

/// Authenticated producer coordinates. In a statement these are self-reported;
/// a trust policy gives them meaning only after transport authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProducerIdentity {
    pub repository: String,
    pub workflow: String,
    pub run_id: String,
    pub event: String,
    pub privilege: ProducerPrivilege,
}

impl ProducerIdentity {
    /// Validate producer coordinates before starting expensive provider work.
    /// Authentication still belongs to the consumer's transport policy.
    pub fn validate(&self) -> Result<(), ReceiptError> {
        validate_producer(self)
    }
}

/// Exact configured-provider definition and its derived digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceiptProvider {
    pub index: usize,
    pub label: String,
    pub definition: serde_json::Value,
    pub sha256: String,
}

/// Complete namespace observed immediately before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceiptCatalogue {
    pub complete: bool,
    pub names: Vec<String>,
    pub sha256: String,
}

/// The provider-scoped statement body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestEvidencePredicate {
    pub schema: String,
    pub repository: RepositoryContext,
    pub provider: ReceiptProvider,
    pub catalogue: ReceiptCatalogue,
    pub requested: Vec<String>,
    pub outcomes: Vec<aplan_evidence::TestBatchResultEntry>,
    pub complete: bool,
    pub material: TestMaterialIdentity,
    pub tool: ToolIdentity,
    pub producer: ProducerIdentity,
    pub started_at_epoch_seconds: u64,
    pub completed_at_epoch_seconds: u64,
}

/// An in-toto v1 Statement carrying aplan test evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestEvidenceStatement {
    #[serde(rename = "_type")]
    pub statement_type: String,
    pub subject: Vec<ResourceDescriptor>,
    #[serde(rename = "predicateType")]
    pub predicate_type: String,
    pub predicate: TestEvidencePredicate,
}

impl TestEvidenceStatement {
    /// Stable, key-ordered JSON bytes terminated by one newline.
    pub fn canonical_json(&self) -> Result<Vec<u8>, ReceiptError> {
        canonical_document(self)
    }
}

/// Inputs to receipt construction. Provider execution and plan resolution are
/// intentionally not combined here.
#[derive(Debug, Clone)]
pub struct ReceiptInput {
    pub repository: RepositoryContext,
    pub runner: TestRunner,
    pub observation: TestExecutionObservation,
    pub material: TestMaterialIdentity,
    pub tool: ToolIdentity,
    pub producer: ProducerIdentity,
    pub started_at_epoch_seconds: u64,
    pub completed_at_epoch_seconds: u64,
}

/// Strong identities a consumer expects now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptExpectation {
    pub provider_sha256: String,
    pub inputs_sha256: String,
    pub environment_sha256: String,
    pub toolchain_sha256: String,
    pub material_sha256: String,
    pub tool: ToolIdentity,
    pub repository_commit: Option<String>,
    /// Revision movement waives only the audit commit comparison. It never
    /// waives a material, provider, environment or toolchain mismatch.
    pub allow_source_revision_movement: bool,
}

/// Required authentication mode. The receipt never chooses this itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustPolicy {
    LocalFile,
    SameWorkflow { producer: ProducerIdentity },
    Signed { producer: ProducerIdentity },
}

/// Facts established outside the untrusted receipt bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptTransport {
    LocalFile,
    SameWorkflow {
        artifact_sha256: String,
        producer: ProducerIdentity,
    },
    Signed {
        payload_sha256: String,
        producer: ProducerIdentity,
    },
}

/// Caller-owned trust facts for one imported receipt.
///
/// These values are not authenticated by their presence in this document. The
/// caller must create the import set in its trusted orchestration boundary,
/// using artifact or signature facts established outside the receipt bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceiptImportTrust {
    LocalFile,
    SameWorkflow {
        artifact_sha256: String,
        producer: ProducerIdentity,
    },
    Signed {
        payload_sha256: String,
        producer: ProducerIdentity,
    },
}

impl ReceiptImportTrust {
    /// Project trusted-orchestrator input onto the verifier's separate policy
    /// and transport values.
    #[must_use]
    pub fn verifier_inputs(&self) -> (TrustPolicy, ReceiptTransport) {
        match self {
            Self::LocalFile => (TrustPolicy::LocalFile, ReceiptTransport::LocalFile),
            Self::SameWorkflow {
                artifact_sha256,
                producer,
            } => (
                TrustPolicy::SameWorkflow {
                    producer: producer.clone(),
                },
                ReceiptTransport::SameWorkflow {
                    artifact_sha256: artifact_sha256.clone(),
                    producer: producer.clone(),
                },
            ),
            Self::Signed {
                payload_sha256,
                producer,
            } => (
                TrustPolicy::Signed {
                    producer: producer.clone(),
                },
                ReceiptTransport::Signed {
                    payload_sha256: payload_sha256.clone(),
                    producer: producer.clone(),
                },
            ),
        }
    }
}

/// One configured provider's receipt and current-source policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceiptImportEntry {
    /// One-based provider index or exact configured label.
    pub provider: String,
    /// Receipt path, relative to the import-set document unless absolute.
    pub path: String,
    #[serde(default)]
    pub allow_source_revision_movement: bool,
    pub trust: ReceiptImportTrust,
}

/// Complete current-only input for receipt-backed whole-plan verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceiptImportSet {
    pub schema: String,
    pub receipts: Vec<ReceiptImportEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DsseEnvelope {
    #[serde(rename = "payloadType")]
    payload_type: String,
    payload: String,
    signatures: Vec<DsseSignature>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DsseSignature {
    #[serde(rename = "keyid", default)]
    _key_id: String,
    sig: String,
}

/// Construct and fully validate a canonical statement.
pub fn build(mut input: ReceiptInput) -> Result<TestEvidenceStatement, ReceiptError> {
    validate_material(&input.material)?;
    validate_producer(&input.producer)?;
    validate_tool(&input.tool)?;
    validate_repository(&input.repository)?;
    if input.completed_at_epoch_seconds < input.started_at_epoch_seconds {
        return Err(invalid("completion timestamp precedes start timestamp"));
    }
    for outcome in &mut input.observation.outcomes {
        validate_safe_text("outcome identity", &outcome.test)?;
        validate_result_entry(outcome).map_err(ReceiptError::Invalid)?;
        if let Some(diagnostic) = outcome.diagnostic.take() {
            if diagnostic.len() > MAX_DIAGNOSTIC_BYTES {
                return Err(invalid(format!(
                    "diagnostic for `{}` exceeds {MAX_DIAGNOSTIC_BYTES} bytes",
                    outcome.test
                )));
            }
            outcome.diagnostic = sanitize_diagnostic(&diagnostic);
        }
    }
    // Diagnostics are untrusted provider output. Normalize that boundary
    // before validating the typed observation, whose importer correctly
    // rejects unsafe controls in already-published bytes.
    validate_observation(&input.observation)?;

    let provider = provider_identity_validated(
        input.observation.runner.index,
        &input.observation.runner.label,
        &input.runner,
    );
    let catalogue_sha256 = digest_strings(
        b"aplan-test-catalogue-v1\0",
        &input.observation.runner.names,
    );
    let catalogue = ReceiptCatalogue {
        complete: true,
        names: input.observation.runner.names,
        sha256: catalogue_sha256,
    };
    let predicate = TestEvidencePredicate {
        schema: PREDICATE_SCHEMA.into(),
        repository: input.repository,
        provider,
        catalogue,
        requested: input.observation.requested,
        outcomes: input.observation.outcomes,
        complete: true,
        material: input.material,
        tool: input.tool,
        producer: input.producer,
        started_at_epoch_seconds: input.started_at_epoch_seconds,
        completed_at_epoch_seconds: input.completed_at_epoch_seconds,
    };
    let subject = expected_subject(&predicate)?;
    Ok(TestEvidenceStatement {
        statement_type: STATEMENT_TYPE.into(),
        subject,
        predicate_type: PREDICATE_TYPE.into(),
        predicate,
    })
}

/// Derive the exact receipt identity for a currently configured provider.
///
/// Consumers use this function when constructing a [`ReceiptExpectation`], so
/// producer and consumer cannot drift into separate canonicalization or digest
/// algorithms.
pub fn provider_identity(
    index: usize,
    label: &str,
    runner: &TestRunner,
) -> Result<ReceiptProvider, ReceiptError> {
    if index == 0 {
        return Err(invalid("provider index and label must be present"));
    }
    validate_safe_text("provider label", label)?;
    Ok(provider_identity_validated(index, label, runner))
}

fn provider_identity_validated(index: usize, label: &str, runner: &TestRunner) -> ReceiptProvider {
    // `TestRunner` contains only JSON-representable fields. The public entry
    // point validates the caller-owned identity before reaching this helper.
    let definition = serde_json::to_value(runner).expect("TestRunner always serializes as JSON");
    let sha256 = provider_digest(index, label, &definition);
    ReceiptProvider {
        index,
        label: label.to_string(),
        definition,
        sha256,
    }
}

/// Parse hostile bytes, validate every derived identity, then apply caller-owned
/// freshness and transport authentication.
pub fn parse_and_verify(
    bytes: &[u8],
    expected: &ReceiptExpectation,
    policy: &TrustPolicy,
    transport: &ReceiptTransport,
) -> Result<TestEvidenceStatement, ReceiptError> {
    parse_and_verify_measured(bytes, expected, policy, transport).map(|(statement, _)| statement)
}

/// Parse and verify a receipt while retaining phase durations for performance
/// analysis. The ordinary verifier delegates here so measurement cannot drift
/// into a second trust implementation.
pub fn parse_and_verify_measured(
    bytes: &[u8],
    expected: &ReceiptExpectation,
    policy: &TrustPolicy,
    transport: &ReceiptTransport,
) -> Result<(TestEvidenceStatement, ReceiptVerificationCost), ReceiptError> {
    let (statement, payload, envelope, parsing_micros) = parse_validated_document(bytes)?;
    let identity_started = Instant::now();
    validate_expectation(&statement, expected)?;
    let identity_micros = identity_started.elapsed().as_micros();
    let authentication_started = Instant::now();
    validate_trust(
        bytes,
        &payload,
        envelope.as_ref(),
        &statement,
        policy,
        transport,
    )?;
    let authentication_micros = authentication_started.elapsed().as_micros();
    Ok((
        statement,
        ReceiptVerificationCost {
            parsing_micros,
            identity_micros,
            authentication_micros,
        },
    ))
}

/// Parse and authenticate a receipt before deciding whether its observation is
/// reusable for current inputs.
///
/// Incremental refresh needs the prior material and catalogue in order to
/// decide what must be rerun. This entry point deliberately omits only the
/// *current* expectation comparison; it still validates the complete typed
/// statement, canonical encoding, internally derived identities and the
/// caller-owned transport policy. Callers must compare provider, tool,
/// revision and material identities before reusing any outcome.
pub fn parse_and_authenticate(
    bytes: &[u8],
    policy: &TrustPolicy,
    transport: &ReceiptTransport,
) -> Result<TestEvidenceStatement, ReceiptError> {
    let (statement, payload, envelope, _) = parse_validated_document(bytes)?;
    validate_trust(
        bytes,
        &payload,
        envelope.as_ref(),
        &statement,
        policy,
        transport,
    )?;
    Ok(statement)
}

fn parse_validated_document(
    bytes: &[u8],
) -> Result<(TestEvidenceStatement, Vec<u8>, Option<DsseEnvelope>, u128), ReceiptError> {
    let parsing_started = Instant::now();
    if bytes.is_empty() || bytes.len() > MAX_RECEIPT_BYTES {
        return Err(invalid(format!(
            "document size must be between 1 and {MAX_RECEIPT_BYTES} bytes"
        )));
    }
    let (payload, envelope) = decode_payload(bytes)?;
    let statement: TestEvidenceStatement = serde_json::from_slice(&payload)
        .map_err(|error| invalid(format!("parsing statement JSON: {error}")))?;
    validate_statement(&statement)?;
    if statement.canonical_json()? != payload {
        return Err(invalid(
            "statement payload is not the canonical JSON encoding for its typed content",
        ));
    }
    Ok((
        statement,
        payload,
        envelope,
        parsing_started.elapsed().as_micros(),
    ))
}

/// Derive the committed JSON Schema from the Rust statement type.
#[must_use]
pub fn derived_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(TestEvidenceStatement)).unwrap_or_default()
}

/// Derive the committed receipt import-set schema from its Rust type.
#[must_use]
pub fn derived_import_set_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ReceiptImportSet)).unwrap_or_default()
}

/// Snapshot the Git audit context without claiming it is the freshness key.
pub fn repository_context(repo_root: &Path) -> Result<RepositoryContext, ReceiptError> {
    let repo = Repo::discover(repo_root).map_err(|error| ReceiptError::Io(error.to_string()))?;
    let tree = repo
        .head_tree()
        .map_err(|error| ReceiptError::Io(error.to_string()))?;
    let status = repo
        .worktree_status()
        .map_err(|error| ReceiptError::Io(error.to_string()))?;
    let mut tree_hasher = Sha256::new();
    tree_hasher.update(b"aplan-repository-tree-v1\0");
    hash_bytes(&mut tree_hasher, tree.tree.as_bytes());
    for (path, blob) in tree.blobs {
        hash_bytes(&mut tree_hasher, path.as_bytes());
        hash_bytes(&mut tree_hasher, blob.as_bytes());
    }
    let mut status_hasher = Sha256::new();
    status_hasher.update(b"aplan-worktree-status-v1\0");
    for entry in status.entries {
        hash_bytes(&mut status_hasher, &entry.path);
        status_hasher.update(entry.status_bits.to_le_bytes());
    }
    Ok(RepositoryContext {
        commit: tree.commit,
        tree_sha256: hex(&tree_hasher.finalize()),
        modified: status.changes.modified,
        untracked: status.changes.untracked,
        status_sha256: hex(&status_hasher.finalize()),
    })
}

/// Publish bytes beside the destination and rename only after a durable write.
/// A producer crash before rename leaves the previous destination untouched.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ReceiptError> {
    write_atomic_before_publish(path, bytes, || Ok(()))
}

fn write_atomic_before_publish(
    path: &Path,
    bytes: &[u8],
    before_publish: impl FnOnce() -> Result<(), ReceiptError>,
) -> Result<(), ReceiptError> {
    if bytes.is_empty() || bytes.len() > MAX_RECEIPT_BYTES {
        return Err(invalid(format!(
            "document size must be between 1 and {MAX_RECEIPT_BYTES} bytes"
        )));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| ReceiptError::Io(error.to_string()))?;
    let mut staged = tempfile::Builder::new()
        .prefix(".aplan-receipt-")
        .tempfile_in(parent)
        .map_err(|error| ReceiptError::Io(error.to_string()))?;
    staged
        .write_all(bytes)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|error| ReceiptError::Io(error.to_string()))?;
    before_publish()?;
    staged
        .persist(path)
        .map_err(|error| ReceiptError::Io(error.error.to_string()))?;
    sync_directory(parent)?;
    Ok(())
}

fn validate_statement(statement: &TestEvidenceStatement) -> Result<(), ReceiptError> {
    if statement.statement_type != STATEMENT_TYPE {
        return Err(invalid("unknown statement type"));
    }
    if statement.predicate_type != PREDICATE_TYPE {
        return Err(invalid("wrong or unknown predicate type"));
    }
    if statement.predicate.schema != PREDICATE_SCHEMA {
        return Err(invalid("unknown predicate schema"));
    }
    validate_predicate(&statement.predicate)?;
    let expected = expected_subject(&statement.predicate)?;
    if statement.subject != expected {
        return Err(invalid("subject digest does not match the predicate"));
    }
    Ok(())
}

fn validate_predicate(predicate: &TestEvidencePredicate) -> Result<(), ReceiptError> {
    if !predicate.complete || !predicate.catalogue.complete {
        return Err(invalid(
            "partial provider observations cannot satisfy evidence",
        ));
    }
    validate_repository(&predicate.repository)?;
    validate_producer(&predicate.producer)?;
    validate_tool(&predicate.tool)?;
    validate_material(&predicate.material)?;
    if predicate.provider.index == 0 {
        return Err(invalid("provider index and label must be present"));
    }
    validate_safe_text("provider label", &predicate.provider.label)?;
    validate_sha256("provider definition", &predicate.provider.sha256)?;
    let provider_digest = provider_digest(
        predicate.provider.index,
        &predicate.provider.label,
        &predicate.provider.definition,
    );
    require_equal(
        "provider definition digest",
        &predicate.provider.sha256,
        &provider_digest,
    )?;
    validate_sorted_unique("catalogue identity", &predicate.catalogue.names, false)?;
    validate_sha256("catalogue", &predicate.catalogue.sha256)?;
    let catalogue_digest = digest_strings(b"aplan-test-catalogue-v1\0", &predicate.catalogue.names);
    require_equal(
        "catalogue digest",
        &predicate.catalogue.sha256,
        &catalogue_digest,
    )?;
    validate_execution_sets(
        &predicate.catalogue.names,
        &predicate.requested,
        &predicate.outcomes,
    )?;
    if predicate.completed_at_epoch_seconds < predicate.started_at_epoch_seconds {
        return Err(invalid("completion timestamp precedes start timestamp"));
    }
    Ok(())
}

fn validate_observation(observation: &TestExecutionObservation) -> Result<(), ReceiptError> {
    if !observation.complete {
        return Err(invalid("the provider observation is explicitly incomplete"));
    }
    if observation.runner.index == 0 || observation.runner.state != TestCatalogueRunnerState::Listed
    {
        return Err(invalid(
            "the provider must have a non-empty complete listed catalogue and stable identity",
        ));
    }
    validate_safe_text("provider label", &observation.runner.label)?;
    validate_sorted_unique("catalogue identity", &observation.runner.names, false)?;
    validate_execution_sets(
        &observation.runner.names,
        &observation.requested,
        &observation.outcomes,
    )
}

fn validate_execution_sets(
    catalogue: &[String],
    requested: &[String],
    outcomes: &[aplan_evidence::TestBatchResultEntry],
) -> Result<(), ReceiptError> {
    validate_sorted_unique("requested identity", requested, false)?;
    let catalogue = catalogue
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for name in requested {
        if !catalogue.contains(name.as_str()) {
            return Err(invalid(format!(
                "requested identity `{name}` is absent from the complete catalogue"
            )));
        }
    }
    let expected = requested
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();
    let mut previous: Option<&str> = None;
    for outcome in outcomes {
        validate_safe_text("outcome identity", &outcome.test)?;
        if let Some(previous) = previous {
            if previous == outcome.test {
                return Err(invalid(format!(
                    "outcome identity `{}` repeats",
                    outcome.test
                )));
            }
            if previous > outcome.test.as_str() {
                return Err(invalid("outcome identity list is not strictly sorted"));
            }
        }
        previous = Some(&outcome.test);
        validate_result_entry(outcome).map_err(ReceiptError::Invalid)?;
        if let Some(diagnostic) = &outcome.diagnostic {
            if diagnostic.len() > MAX_DIAGNOSTIC_BYTES {
                return Err(invalid(format!(
                    "diagnostic for `{}` exceeds {MAX_DIAGNOSTIC_BYTES} bytes",
                    outcome.test
                )));
            }
            if aplan_output::sanitize::visible(diagnostic).as_ref() != diagnostic {
                return Err(invalid(format!(
                    "diagnostic for `{}` contains unsafe terminal or bidi control characters",
                    outcome.test
                )));
            }
        }
        // The strict-order check above has already proved uniqueness.
        observed.insert(outcome.test.as_str());
        if !expected.contains(outcome.test.as_str()) {
            return Err(invalid(format!(
                "outcome identity `{}` was not requested",
                outcome.test
            )));
        }
    }
    if observed != expected {
        let missing = expected
            .difference(&observed)
            .next()
            .copied()
            .unwrap_or_default();
        return Err(invalid(format!(
            "requested identity `{missing}` is absent from outcomes"
        )));
    }
    Ok(())
}

fn validate_expectation(
    statement: &TestEvidenceStatement,
    expected: &ReceiptExpectation,
) -> Result<(), ReceiptError> {
    let predicate = &statement.predicate;
    require_expected(
        "provider",
        &expected.provider_sha256,
        &predicate.provider.sha256,
    )?;
    require_expected(
        "input material",
        &expected.inputs_sha256,
        &predicate.material.inputs_sha256,
    )?;
    require_expected(
        "environment",
        &expected.environment_sha256,
        &predicate.material.environment_sha256,
    )?;
    require_expected(
        "toolchain",
        &expected.toolchain_sha256,
        &predicate.material.toolchain_sha256,
    )?;
    require_expected(
        "composed material",
        &expected.material_sha256,
        &predicate.material.sha256,
    )?;
    if expected.tool != predicate.tool {
        return Err(ReceiptError::Identity(
            "aplan tool identity changed (version, source, facts or binary differs)".into(),
        ));
    }
    if !expected.allow_source_revision_movement {
        let commit = expected.repository_commit.as_ref().ok_or_else(|| {
            invalid(
                "expected repository commit is absent while source-revision movement is forbidden",
            )
        })?;
        require_expected("repository commit", commit, &predicate.repository.commit)?;
    }
    Ok(())
}

fn validate_trust(
    document: &[u8],
    payload: &[u8],
    envelope: Option<&DsseEnvelope>,
    statement: &TestEvidenceStatement,
    policy: &TrustPolicy,
    transport: &ReceiptTransport,
) -> Result<(), ReceiptError> {
    match (policy, transport) {
        (TrustPolicy::LocalFile, ReceiptTransport::LocalFile) => {
            if envelope.is_some() {
                return Err(trust("local-file policy expects a raw statement"));
            }
        }
        (
            TrustPolicy::SameWorkflow { producer: wanted },
            ReceiptTransport::SameWorkflow {
                artifact_sha256,
                producer,
            },
        ) => {
            if envelope.is_some() {
                return Err(trust("same-workflow transport expects a raw artifact"));
            }
            if artifact_sha256 != &sha256_hex(document) {
                return Err(trust("authenticated artifact digest differs"));
            }
            authenticate_producer(wanted, producer, &statement.predicate.producer)?;
        }
        (
            TrustPolicy::Signed { producer: wanted },
            ReceiptTransport::Signed {
                payload_sha256,
                producer,
            },
        ) => {
            if envelope.is_none() {
                return Err(trust("signed policy requires a DSSE envelope"));
            }
            if payload_sha256 != &sha256_hex(payload) {
                return Err(trust("authenticated signed payload digest differs"));
            }
            authenticate_producer(wanted, producer, &statement.predicate.producer)?;
        }
        _ => {
            return Err(trust(
                "transport kind does not match the configured trust policy",
            ));
        }
    }
    Ok(())
}

fn authenticate_producer(
    wanted: &ProducerIdentity,
    authenticated: &ProducerIdentity,
    stated: &ProducerIdentity,
) -> Result<(), ReceiptError> {
    if authenticated != wanted || stated != wanted {
        return Err(trust(
            "authenticated producer, required producer and statement producer differ",
        ));
    }
    if authenticated.privilege != ProducerPrivilege::Trusted {
        return Err(trust(
            "producer is untrusted and cannot publish privileged evidence",
        ));
    }
    Ok(())
}

fn decode_payload(bytes: &[u8]) -> Result<(Vec<u8>, Option<DsseEnvelope>), ReceiptError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("parsing receipt JSON: {error}")))?;
    if value.get("payloadType").is_none() {
        return Ok((bytes.to_vec(), None));
    }
    let envelope: DsseEnvelope = serde_json::from_value(value)
        .map_err(|error| invalid(format!("parsing DSSE envelope: {error}")))?;
    if envelope.payload_type != DSSE_PAYLOAD_TYPE {
        return Err(invalid(format!(
            "DSSE payload type `{}` is not `{DSSE_PAYLOAD_TYPE}`",
            envelope.payload_type
        )));
    }
    if envelope.signatures.is_empty()
        || envelope
            .signatures
            .iter()
            .any(|signature| signature.sig.trim().is_empty())
    {
        return Err(invalid("DSSE envelope has no non-empty signature"));
    }
    for signature in &envelope.signatures {
        // `keyid` may be empty in DSSE. Its bytes are still covered by the
        // document bound and by the transport's external authentication.
        base64::engine::general_purpose::STANDARD
            .decode(&signature.sig)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&signature.sig))
            .map_err(|error| invalid(format!("decoding DSSE signature: {error}")))?;
    }
    let payload = base64::engine::general_purpose::STANDARD
        .decode(&envelope.payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&envelope.payload))
        .map_err(|error| invalid(format!("decoding DSSE payload: {error}")))?;
    if payload.is_empty() || payload.len() > MAX_RECEIPT_BYTES {
        return Err(invalid(
            "DSSE payload is empty or exceeds the receipt bound",
        ));
    }
    Ok((payload, Some(envelope)))
}

fn expected_subject(
    predicate: &TestEvidencePredicate,
) -> Result<Vec<ResourceDescriptor>, ReceiptError> {
    let bytes = canonical_document(predicate)?;
    let digest = BTreeMap::from([("sha256".into(), sha256_hex(&bytes))]);
    Ok(vec![ResourceDescriptor {
        name: format!(
            "aplan-test-provider/{}/{}",
            predicate.provider.index, predicate.provider.label
        ),
        digest,
    }])
}

fn validate_material(material: &TestMaterialIdentity) -> Result<(), ReceiptError> {
    for (name, value) in [
        ("input material", &material.inputs_sha256),
        ("environment", &material.environment_sha256),
        ("toolchain", &material.toolchain_sha256),
        ("composed material", &material.sha256),
    ] {
        validate_sha256(name, value)?;
    }
    let mut combined = Sha256::new();
    combined.update(b"aplan-test-material-v1\0");
    for value in [
        &material.inputs_sha256,
        &material.environment_sha256,
        &material.toolchain_sha256,
    ] {
        let decoded = decode_hex(value)?;
        combined.update(decoded);
    }
    require_equal(
        "composed material digest",
        &material.sha256,
        &hex(&combined.finalize()),
    )
}

fn validate_repository(repository: &RepositoryContext) -> Result<(), ReceiptError> {
    if repository.commit.len() != 40
        || !repository
            .commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid(
            "repository commit is not a full hexadecimal Git object id",
        ));
    }
    validate_sha256("repository tree", &repository.tree_sha256)?;
    validate_sha256("repository status", &repository.status_sha256)
}

fn validate_tool(tool: &ToolIdentity) -> Result<(), ReceiptError> {
    validate_safe_text("tool version", &tool.version)?;
    if !tool.source.is_empty()
        && (tool.source.len() != 40 || !tool.source.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(invalid(
            "tool source is not empty or a full hexadecimal Git object id",
        ));
    }
    validate_sha256("tool facts", &tool.facts_sha256)?;
    validate_sha256("tool binary", &tool.binary_sha256)
}

fn validate_producer(producer: &ProducerIdentity) -> Result<(), ReceiptError> {
    for (name, value) in [
        ("repository", &producer.repository),
        ("workflow", &producer.workflow),
        ("run id", &producer.run_id),
        ("event", &producer.event),
    ] {
        validate_safe_text(&format!("producer {name}"), value)?;
    }
    Ok(())
}

fn validate_sorted_unique(
    what: &str,
    values: &[String],
    allow_empty: bool,
) -> Result<(), ReceiptError> {
    if !allow_empty && values.is_empty() {
        return Err(invalid(format!("{what} list is empty")));
    }
    let mut previous: Option<&str> = None;
    for value in values {
        validate_safe_text(what, value)?;
        if previous.is_some_and(|previous| previous >= value.as_str()) {
            return Err(invalid(format!(
                "{what} list is not strictly sorted and unique"
            )));
        }
        previous = Some(value);
    }
    Ok(())
}

fn validate_sha256(name: &str, value: &str) -> Result<(), ReceiptError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(format!("{name} digest is not lowercase SHA-256")));
    }
    Ok(())
}

fn decode_hex(value: &str) -> Result<[u8; 32], ReceiptError> {
    validate_sha256("material component", value)?;
    let mut out = [0u8; 32];
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    for (index, chunk) in pairs.iter().enumerate() {
        let text = std::str::from_utf8(chunk).map_err(|_| invalid("digest is not UTF-8"))?;
        out[index] = u8::from_str_radix(text, 16).map_err(|_| invalid("digest is not hex"))?;
    }
    Ok(out)
}

fn require_expected(name: &str, expected: &str, actual: &str) -> Result<(), ReceiptError> {
    validate_sha256_if_digest(name, expected)?;
    if expected != actual {
        return Err(ReceiptError::Identity(format!("{name} changed")));
    }
    Ok(())
}

fn validate_sha256_if_digest(name: &str, value: &str) -> Result<(), ReceiptError> {
    if name == "repository commit" {
        if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid(
                "expected repository commit is not a full object id",
            ));
        }
        Ok(())
    } else {
        validate_sha256(name, value)
    }
}

fn require_equal(name: &str, left: &str, right: &str) -> Result<(), ReceiptError> {
    if left != right {
        return Err(ReceiptError::Identity(format!("{name} differs")));
    }
    Ok(())
}

fn digest_strings(domain: &[u8], values: &[String]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    for value in values {
        hash_bytes(&mut digest, value.as_bytes());
    }
    hex(&digest.finalize())
}

fn provider_digest(index: usize, label: &str, definition: &serde_json::Value) -> String {
    let identity = serde_json::json!({
        "definition": definition,
        "index": index,
        "label": label,
    });
    // A `serde_json::Value` has no fallible serialization cases: object keys
    // are already strings and the value cannot contain a non-finite number.
    let bytes = canonical_value_bytes(&identity).expect("JSON values always serialize to JSON");
    sha256_hex(&bytes)
}

fn hash_bytes(digest: &mut Sha256, bytes: &[u8]) {
    digest.update(bytes.len().to_le_bytes());
    digest.update(bytes);
}

fn sanitize_diagnostic(value: &str) -> Option<String> {
    aplan_evidence::excerpt::of(
        value.as_bytes(),
        &[],
        aplan_evidence::excerpt::Bound {
            lines: 96,
            line_chars: 512,
        },
    )
}

fn canonical_document<T: Serialize>(value: &T) -> Result<Vec<u8>, ReceiptError> {
    let value = serde_json::to_value(value)
        .map_err(|error| invalid(format!("serializing canonical JSON: {error}")))?;
    let mut bytes = canonical_value_bytes(&value)?;
    if bytes.len() >= MAX_RECEIPT_BYTES {
        return Err(invalid(format!(
            "canonical document exceeds {MAX_RECEIPT_BYTES} bytes"
        )));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_value_bytes(value: &serde_json::Value) -> Result<Vec<u8>, ReceiptError> {
    let normalized = sort_json(value);
    serde_json::to_vec(&normalized)
        .map_err(|error| invalid(format!("serializing canonical JSON: {error}")))
}

fn sort_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), sort_json(&values[key]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(sort_json).collect())
        }
        other => other.clone(),
    }
}

fn sync_directory(_path: &Path) -> Result<(), ReceiptError> {
    #[cfg(unix)]
    {
        fs::File::open(_path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| ReceiptError::Io(error.to_string()))?;
    }
    Ok(())
}

fn vacuous(value: &str) -> bool {
    aplan_core::task::is_vacuous_ref(value)
}

fn validate_safe_text(name: &str, value: &str) -> Result<(), ReceiptError> {
    if vacuous(value) {
        return Err(invalid(format!("{name} is absent or blank")));
    }
    if aplan_output::sanitize::visible(value).as_ref() != value {
        return Err(invalid(format!(
            "{name} contains unsafe terminal or bidi control characters"
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> ReceiptError {
    ReceiptError::Invalid(message.into())
}

fn trust(message: impl Into<String>) -> ReceiptError {
    ReceiptError::Trust(message.into())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
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

    fn producer() -> ProducerIdentity {
        ProducerIdentity {
            repository: "owner/repository".into(),
            workflow: "checks.yml:unit".into(),
            run_id: "17".into(),
            event: "push".into(),
            privilege: ProducerPrivilege::Trusted,
        }
    }

    #[test]
    fn import_trust_projects_every_mode_without_losing_or_crossing_inputs() {
        assert_eq!(
            ReceiptImportTrust::LocalFile.verifier_inputs(),
            (TrustPolicy::LocalFile, ReceiptTransport::LocalFile)
        );

        let producer = producer();
        assert_eq!(
            ReceiptImportTrust::SameWorkflow {
                artifact_sha256: "artifact".into(),
                producer: producer.clone(),
            }
            .verifier_inputs(),
            (
                TrustPolicy::SameWorkflow {
                    producer: producer.clone(),
                },
                ReceiptTransport::SameWorkflow {
                    artifact_sha256: "artifact".into(),
                    producer: producer.clone(),
                },
            )
        );
        assert_eq!(
            ReceiptImportTrust::Signed {
                payload_sha256: "payload".into(),
                producer: producer.clone(),
            }
            .verifier_inputs(),
            (
                TrustPolicy::Signed {
                    producer: producer.clone(),
                },
                ReceiptTransport::Signed {
                    payload_sha256: "payload".into(),
                    producer,
                },
            )
        );
    }

    #[test]
    fn import_set_schema_is_derived_from_the_public_type() {
        let schema = derived_import_set_schema();
        let text = serde_json::to_string(&schema).expect("schema JSON");
        assert!(text.contains("ReceiptImportSet"), "{text}");
        assert!(text.contains("ReceiptImportTrust"), "{text}");
    }

    #[test]
    fn canonical_json_orders_object_keys_recursively() {
        let value = serde_json::json!({"z": {"b": 2, "a": 1}, "a": 0});
        assert_eq!(
            canonical_document(&value).expect("canonical"),
            b"{\"a\":0,\"z\":{\"a\":1,\"b\":2}}\n"
        );
    }

    #[test]
    fn canonical_documents_have_a_hard_size_bound() {
        let oversized = "x".repeat(MAX_RECEIPT_BYTES);
        let error = canonical_document(&oversized).expect_err("oversized document");
        assert!(error.to_string().contains("exceeds"), "{error}");
    }

    #[test]
    fn atomic_publication_reports_a_destination_that_cannot_be_replaced() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let destination = directory.path().join("receipt.json");
        fs::create_dir(&destination).expect("directory at destination");

        let error = write_atomic(&destination, b"receipt").expect_err("cannot replace directory");
        assert!(matches!(error, ReceiptError::Io(_)), "{error}");
        assert!(
            destination.is_dir(),
            "the failed publish preserves its target"
        );
    }

    #[test]
    fn producer_crash_before_atomic_rename_leaves_the_existing_destination_untouched() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let path = directory.path().join("receipt.json");
        fs::write(&path, "original").expect("original");
        let error = write_atomic_before_publish(&path, b"replacement", || {
            Err(ReceiptError::Io("simulated producer crash".into()))
        })
        .expect_err("crash before publish");
        assert!(error.to_string().contains("simulated producer crash"));
        assert_eq!(fs::read_to_string(path).expect("still there"), "original");
    }
}
