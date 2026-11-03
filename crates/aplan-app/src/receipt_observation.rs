//! Current-only import of portable test-evidence receipts.
//!
//! The import set supplies trusted transport facts; each receipt supplies raw
//! provider observation. The current runner, material, tool and repository
//! identities are recomputed here, and the evidence engine derives the verdict
//! from the current plan rather than accepting one from the artifact.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use aplan_attest::ToolIdentity;
use aplan_attest::receipt::{
    IMPORT_SET_SCHEMA, MAX_RECEIPT_BYTES, ReceiptError, ReceiptExpectation, ReceiptImportEntry,
    ReceiptImportSet, RepositoryContext, parse_and_verify_measured, provider_identity,
    repository_context,
};
use aplan_core::Plan;
use aplan_core::show::{
    EvidenceObservationCompleteness, EvidenceObservationScope, EvidenceObservationState,
    EvidenceProviderSource,
};
use aplan_evidence::{
    TestCatalogueRunner, TestCatalogueRunnerState, TestExecutionObservation,
    TestExecutionObservations, TestObservationRejectionKind, TestProviderObservation, TestRunner,
    test_material_identity, test_runner_by_selector,
};
use sha2::{Digest as _, Sha256};

/// Import and resolution phase measurements. They are telemetry, not verdicts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReceiptVerificationCost {
    pub manifest_micros: u128,
    pub material_micros: u128,
    pub receipt_parsing_micros: u128,
    pub receipt_identity_micros: u128,
    pub receipt_authentication_micros: u128,
    pub catalogue_projection_micros: u128,
}

/// Complete current-only input plus path-free provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedTestReceipts {
    pub observations: TestExecutionObservations,
    pub identity: String,
    pub observed_at_epoch_seconds: Option<i64>,
    pub accepted: usize,
    pub rejected: usize,
    pub freshness: EvidenceObservationState,
    pub cost: ReceiptVerificationCost,
    /// Present only when every declared provider material could be measured.
    /// A material fault is already a typed provider rejection and must not be
    /// promoted into a top-level import error by the movement check.
    currency_identity: Option<String>,
}

/// A receipt-set error before individual provider findings can be produced.
#[derive(Debug, thiserror::Error)]
pub enum ReceiptImportError {
    #[error("receipt import requires configured test providers")]
    NoProviders,
    #[error("receipt import set could not be read: {0}")]
    Read(String),
    #[error("receipt import set is invalid: {0}")]
    Invalid(String),
    #[error("repository identity could not be read: {0}")]
    Repository(String),
    #[error("receipt-backed evidence resolution failed: {0}")]
    Evidence(#[from] aplan_evidence::Error),
}

/// Receipt-backed whole-plan verification plus import phase telemetry.
#[derive(Debug)]
pub struct ReceiptEvidenceInspection {
    pub evidence: crate::EvidenceInspection,
    pub import: ImportedTestReceipts,
}

struct ReceiptImportContext<'a> {
    repo_root: &'a Path,
    declared_inputs: &'a [String],
    tool: &'a ToolIdentity,
    base: &'a Path,
    repository: &'a RepositoryContext,
}

struct ImportedProvider {
    observation: TestProviderObservation,
    receipt_bytes: Option<Vec<u8>>,
    rejection: Option<TestObservationRejectionKind>,
    observed_at_epoch_seconds: Option<i64>,
    material_sha256: Option<String>,
    cost: ReceiptVerificationCost,
}

/// Derive a whole-plan verdict from current receipts without starting a test
/// provider. Repository, file and configured URL resolvers keep their ordinary
/// semantics; only the configured-test observation source changes.
pub fn inspect_receipt_evidence(
    plan: &Plan,
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    tool: &ToolIdentity,
    import_set_path: &Path,
) -> Result<ReceiptEvidenceInspection, ReceiptImportError> {
    let imported = import_test_receipts(repo_root, options, tool, import_set_path)?;
    let mut local = options.clone();
    local.execute = true;
    let observed = aplan_evidence::verify_observed_with_test_observations(
        plan,
        repo_root,
        &local,
        &imported.observations,
    )?;
    if let Some(imported_currency) = imported.currency_identity.as_deref() {
        let current = current_currency_identity(repo_root, &local)?;
        if current != imported_currency {
            return Err(ReceiptImportError::Repository(
                "the repository or declared provider material changed during receipt-backed resolution"
                    .into(),
            ));
        }
    }
    let repository = crate::evidence_observation::repository_identity(repo_root)?;
    let resolution = crate::evidence_observation::resolution_identity(
        plan,
        &local,
        &repository,
        Some(&imported.identity),
        &observed,
    );
    let state = imported.freshness;
    let completeness = if imported.rejected == 0 {
        EvidenceObservationCompleteness::Complete
    } else {
        EvidenceObservationCompleteness::Partial
    };
    let detail = if imported.rejected == 0 {
        format!(
            "all {} configured test provider receipt(s) are current and complete",
            imported.accepted
        )
    } else {
        format!(
            "{} configured test provider receipt(s) accepted and {} rejected; no provider fallback was started",
            imported.accepted, imported.rejected
        )
    };
    let mut evidence = crate::evidence_observation::project_inspection(
        plan,
        observed,
        EvidenceObservationScope::WholePlan,
        EvidenceProviderSource::Receipt,
        completeness,
        state,
        if options.url_check.is_some() {
            EvidenceObservationState::Current
        } else {
            EvidenceObservationState::Unsupported
        },
        Some(imported.identity.clone()),
        imported.observed_at_epoch_seconds,
        Some(resolution),
        detail,
    );
    // Provider-set validity is a whole-mode property even when the current
    // plan happens to cite no test. Citation aggregation alone cannot erase a
    // rejected configured provider from the aggregate result.
    evidence.observation.state = state;
    Ok(ReceiptEvidenceInspection {
        evidence,
        import: imported,
    })
}

/// Validate a complete receipt set against the current provider/material/tool
/// identity without starting configured commands.
pub fn import_test_receipts(
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    tool: &ToolIdentity,
    import_set_path: &Path,
) -> Result<ImportedTestReceipts, ReceiptImportError> {
    let runners = options
        .test_runner
        .as_ref()
        .ok_or(ReceiptImportError::NoProviders)?;
    let started = Instant::now();
    let manifest_bytes =
        read_bounded(import_set_path, MAX_RECEIPT_BYTES).map_err(ReceiptImportError::Read)?;
    let manifest: ReceiptImportSet = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| ReceiptImportError::Invalid(error.to_string()))?;
    if manifest.schema != IMPORT_SET_SCHEMA {
        return Err(ReceiptImportError::Invalid(format!(
            "schema must be `{IMPORT_SET_SCHEMA}`"
        )));
    }
    let manifest_micros = started.elapsed().as_micros();
    let base = import_set_path.parent().unwrap_or_else(|| Path::new("."));
    let mut selected = BTreeMap::new();
    for entry in &manifest.receipts {
        let (index, _, _) = test_runner_by_selector(runners, &entry.provider)
            .map_err(ReceiptImportError::Invalid)?;
        if selected.insert(index, entry).is_some() {
            return Err(ReceiptImportError::Invalid(format!(
                "configured provider {index} appears more than once"
            )));
        }
    }
    let repository = repository_context(repo_root)
        .map_err(|error| ReceiptImportError::Repository(error.to_string()))?;
    let declared_inputs = options.observation_inputs.as_deref().ok_or_else(|| {
        ReceiptImportError::Invalid("exhaustive `evidence.observation_inputs` is required".into())
    })?;
    let mut providers = Vec::with_capacity(runners.all().len());
    let mut accepted_bytes = Vec::new();
    let mut rejection_kinds = Vec::new();
    let mut observed_at_epoch_seconds = None;
    let mut cost = ReceiptVerificationCost {
        manifest_micros,
        ..ReceiptVerificationCost::default()
    };
    let mut currency = Sha256::new();
    let mut currency_is_complete = true;
    currency.update(b"aplan-test-receipt-currency-v1\0");
    currency.update(serde_json::to_vec(&repository).unwrap_or_default());
    let context = ReceiptImportContext {
        repo_root,
        declared_inputs,
        tool,
        base,
        repository: &repository,
    };
    for (offset, runner) in runners.all().iter().enumerate() {
        let index = offset + 1;
        let (_, label, _) = test_runner_by_selector(runners, &index.to_string())
            .expect("a configured provider resolves by its one-based index");
        let Some(entry) = selected.get(&index) else {
            currency_is_complete = false;
            rejection_kinds.push(TestObservationRejectionKind::Absent);
            providers.push(rejected_provider(
                index,
                label,
                TestObservationRejectionKind::Absent,
                "no receipt was supplied for this configured provider",
            ));
            continue;
        };
        let imported = import_provider(&context, index, label, runner, entry);
        fold_receipt_cost(&mut cost, imported.cost);
        if let Some(material_sha256) = imported.material_sha256 {
            currency.update(index.to_le_bytes());
            currency.update(material_sha256.as_bytes());
        } else {
            currency_is_complete = false;
        }
        if let Some(bytes) = imported.receipt_bytes {
            accepted_bytes.push((index, bytes));
        }
        if let Some(kind) = imported.rejection {
            rejection_kinds.push(kind);
        }
        observed_at_epoch_seconds =
            observed_at_epoch_seconds.max(imported.observed_at_epoch_seconds);
        providers.push(imported.observation);
    }
    let accepted = accepted_bytes.len();
    let rejected = runners.all().len().saturating_sub(accepted);
    Ok(ImportedTestReceipts {
        observations: TestExecutionObservations { providers },
        identity: receipt_set_identity(&accepted_bytes),
        observed_at_epoch_seconds,
        accepted,
        rejected,
        freshness: receipt_freshness(&rejection_kinds),
        cost,
        currency_identity: currency_is_complete.then(|| hex_bytes(&currency.finalize())),
    })
}

fn receipt_set_identity(accepted: &[(usize, Vec<u8>)]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"aplan-test-receipt-set-v1\0");
    for (index, bytes) in accepted {
        digest.update(index.to_le_bytes());
        digest.update(Sha256::digest(bytes));
    }
    format!("sha256:{}", hex_bytes(&digest.finalize()))
}

fn import_provider(
    context: &ReceiptImportContext<'_>,
    index: usize,
    label: String,
    runner: &TestRunner,
    entry: &ReceiptImportEntry,
) -> ImportedProvider {
    let material_started = Instant::now();
    let material = match test_material_identity(context.repo_root, context.declared_inputs, runner)
    {
        Ok(material) => material,
        Err(error) => {
            return ImportedProvider {
                observation: rejected_provider(
                    index,
                    label,
                    TestObservationRejectionKind::Fault,
                    &format!("current provider material could not be identified: {error}"),
                ),
                receipt_bytes: None,
                rejection: Some(TestObservationRejectionKind::Fault),
                observed_at_epoch_seconds: None,
                material_sha256: None,
                cost: ReceiptVerificationCost {
                    material_micros: material_started.elapsed().as_micros(),
                    ..ReceiptVerificationCost::default()
                },
            };
        }
    };
    let mut cost = ReceiptVerificationCost {
        material_micros: material_started.elapsed().as_micros(),
        ..ReceiptVerificationCost::default()
    };
    let provider = provider_identity(index, &label, runner)
        .expect("configured provider labels and indexes are already validated");
    let expected = ReceiptExpectation {
        provider_sha256: provider.sha256,
        inputs_sha256: material.inputs_sha256.clone(),
        environment_sha256: material.environment_sha256.clone(),
        toolchain_sha256: material.toolchain_sha256.clone(),
        material_sha256: material.sha256.clone(),
        tool: context.tool.clone(),
        repository_commit: Some(context.repository.commit.clone()),
        allow_source_revision_movement: entry.allow_source_revision_movement,
    };
    let receipt_path = resolve_path(context.base, &entry.path);
    let parsed = read_bounded(&receipt_path, MAX_RECEIPT_BYTES)
        .map_err(ReceiptError::Io)
        .and_then(|bytes| {
            let (policy, transport) = entry.trust.verifier_inputs();
            parse_and_verify_measured(&bytes, &expected, &policy, &transport)
                .map(|(statement, measured)| (bytes, statement, measured))
        });
    let material_sha256 = Some(material.sha256);
    match parsed {
        Ok((bytes, statement, measured)) => {
            cost.receipt_parsing_micros += measured.parsing_micros;
            cost.receipt_identity_micros += measured.identity_micros;
            cost.receipt_authentication_micros += measured.authentication_micros;
            let projection_started = Instant::now();
            let predicate = statement.predicate;
            let completed_at =
                i64::try_from(predicate.completed_at_epoch_seconds).unwrap_or(i64::MAX);
            let observation = TestExecutionObservation {
                runner: TestCatalogueRunner {
                    index,
                    label,
                    state: TestCatalogueRunnerState::Listed,
                    names: predicate.catalogue.names,
                },
                requested: predicate.requested,
                outcomes: predicate.outcomes,
                complete: predicate.complete,
            };
            cost.catalogue_projection_micros += projection_started.elapsed().as_micros();
            ImportedProvider {
                observation: TestProviderObservation::Current(observation),
                receipt_bytes: Some(bytes),
                rejection: None,
                observed_at_epoch_seconds: Some(completed_at),
                material_sha256,
                cost,
            }
        }
        Err(error) => {
            let kind = rejection_kind(&error);
            ImportedProvider {
                observation: rejected_provider(index, label, kind, &error.to_string()),
                receipt_bytes: None,
                rejection: Some(kind),
                observed_at_epoch_seconds: None,
                material_sha256,
                cost,
            }
        }
    }
}

fn fold_receipt_cost(total: &mut ReceiptVerificationCost, one: ReceiptVerificationCost) {
    total.material_micros += one.material_micros;
    total.receipt_parsing_micros += one.receipt_parsing_micros;
    total.receipt_identity_micros += one.receipt_identity_micros;
    total.receipt_authentication_micros += one.receipt_authentication_micros;
    total.catalogue_projection_micros += one.catalogue_projection_micros;
}

fn receipt_freshness(kinds: &[TestObservationRejectionKind]) -> EvidenceObservationState {
    for (kind, state) in [
        (
            TestObservationRejectionKind::Untrusted,
            EvidenceObservationState::Untrusted,
        ),
        (
            TestObservationRejectionKind::Incompatible,
            EvidenceObservationState::Incompatible,
        ),
        (
            TestObservationRejectionKind::Fault,
            EvidenceObservationState::Fault,
        ),
        (
            TestObservationRejectionKind::Stale,
            EvidenceObservationState::Stale,
        ),
        (
            TestObservationRejectionKind::Partial,
            EvidenceObservationState::NotObserved,
        ),
        (
            TestObservationRejectionKind::Absent,
            EvidenceObservationState::NotObserved,
        ),
    ] {
        if kinds.contains(&kind) {
            return state;
        }
    }
    EvidenceObservationState::Current
}

fn rejected_provider(
    index: usize,
    label: String,
    kind: TestObservationRejectionKind,
    detail: &str,
) -> TestProviderObservation {
    TestProviderObservation::Rejected {
        index,
        label,
        kind,
        detail: path_free(detail),
    }
}

fn rejection_kind(error: &ReceiptError) -> TestObservationRejectionKind {
    match error {
        ReceiptError::Identity(_) => TestObservationRejectionKind::Stale,
        ReceiptError::Trust(_) => TestObservationRejectionKind::Untrusted,
        ReceiptError::Invalid(message)
            if message.contains("partial")
                || message.contains("complete")
                || message.contains("outcome") =>
        {
            TestObservationRejectionKind::Partial
        }
        ReceiptError::Invalid(_) => TestObservationRejectionKind::Incompatible,
        ReceiptError::Io(_) => TestObservationRejectionKind::Absent,
    }
}

pub(crate) fn resolve_path(base: &Path, configured: &str) -> PathBuf {
    let path = Path::new(configured);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

pub(crate) fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, String> {
    let mut file =
        File::open(path).map_err(|_| "the supplied file is absent or unreadable".to_string())?;
    let metadata = file
        .metadata()
        .map_err(|_| "the supplied file identity could not be read".to_string())?;
    if metadata.len() == 0 || metadata.len() > maximum as u64 {
        return Err(format!(
            "document size must be between 1 and {maximum} bytes"
        ));
    }
    let observed_size = usize::try_from(metadata.len())
        .map_err(|_| "the supplied file is too large for this platform".to_string())?;
    let mut bytes = Vec::with_capacity(observed_size);
    file.by_ref()
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "the supplied file could not be read completely".to_string())?;
    if bytes.len() != observed_size || bytes.len() > maximum {
        return Err("the supplied file changed while it was read".to_string());
    }
    let after = file
        .metadata()
        .map_err(|_| "the supplied file identity could not be re-read".to_string())?;
    if after.len() != metadata.len() || after.modified().ok() != metadata.modified().ok() {
        return Err("the supplied file changed while it was read".to_string());
    }
    Ok(bytes)
}

fn path_free(detail: &str) -> String {
    let visible = aplan_output::sanitize::visible(detail);
    visible
        .split_whitespace()
        .take(80)
        .collect::<Vec<_>>()
        .join(" ")
}

fn current_currency_identity(
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
) -> Result<String, ReceiptImportError> {
    let runners = options
        .test_runner
        .as_ref()
        .ok_or(ReceiptImportError::NoProviders)?;
    let declared_inputs = options.observation_inputs.as_deref().ok_or_else(|| {
        ReceiptImportError::Invalid("exhaustive `evidence.observation_inputs` is required".into())
    })?;
    let repository = repository_context(repo_root)
        .map_err(|error| ReceiptImportError::Repository(error.to_string()))?;
    let mut digest = Sha256::new();
    digest.update(b"aplan-test-receipt-currency-v1\0");
    digest.update(serde_json::to_vec(&repository).unwrap_or_default());
    for (offset, runner) in runners.all().iter().enumerate() {
        let material = test_material_identity(repo_root, declared_inputs, runner)
            .map_err(|error| ReceiptImportError::Repository(error.clone()))?;
        digest.update((offset + 1).to_le_bytes());
        digest.update(material.sha256.as_bytes());
    }
    Ok(hex_bytes(&digest.finalize()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_attest::receipt::{
        ProducerIdentity, ProducerPrivilege, ReceiptImportEntry, ReceiptImportTrust, ReceiptInput,
        build,
    };
    use aplan_config::{PLACEHOLDER, TestRunner, TestRunners};
    use aplan_core::load::{Criteria, FieldLines, LoadedTask};
    use aplan_core::{Evidence, EvidenceKind, Plan, Task};
    use aplan_evidence::{TestBatchOutcome, TestBatchResultEntry};
    use git2::{Repository, Signature};

    #[test]
    fn receipt_error_kinds_paths_and_bounded_reads_preserve_their_boundaries() {
        let cases = [
            (
                ReceiptError::Identity("moved".into()),
                TestObservationRejectionKind::Stale,
            ),
            (
                ReceiptError::Trust("untrusted".into()),
                TestObservationRejectionKind::Untrusted,
            ),
            (
                ReceiptError::Invalid("partial outcome set".into()),
                TestObservationRejectionKind::Partial,
            ),
            (
                ReceiptError::Invalid("wrong schema".into()),
                TestObservationRejectionKind::Incompatible,
            ),
            (
                ReceiptError::Io("absent".into()),
                TestObservationRejectionKind::Absent,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(rejection_kind(&error), expected, "{error}");
        }

        let base = Path::new("receipts");
        assert_eq!(resolve_path(base, "one.json"), base.join("one.json"));
        let absolute = std::env::current_dir()
            .expect("current directory")
            .join("one.json");
        assert_eq!(
            resolve_path(base, absolute.to_str().expect("UTF-8")),
            absolute
        );

        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let file = root.path().join("receipt.json");
        std::fs::write(&file, []).expect("empty");
        assert!(
            read_bounded(&file, 4)
                .expect_err("empty")
                .contains("between 1 and 4")
        );
        std::fs::write(&file, b"12345").expect("oversized");
        assert!(
            read_bounded(&file, 4)
                .expect_err("oversized")
                .contains("between 1 and 4")
        );
        std::fs::write(&file, b"1234").expect("bounded");
        assert_eq!(read_bounded(&file, 4).expect("bounded read"), b"1234");

        let options = aplan_evidence::VerifyOptions {
            test_runner: Some(TestRunners::One(runner(&root.path().join("marker")))),
            ..aplan_evidence::VerifyOptions::default()
        };
        let error = current_currency_identity(root.path(), &options)
            .expect_err("material declaration required");
        assert!(error.to_string().contains("observation_inputs"), "{error}");
    }

    #[test]
    fn receipt_import_refuses_schema_selector_and_input_set_ambiguity() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let options = aplan_evidence::VerifyOptions {
            test_runner: Some(TestRunners::One(runner(&root.path().join("marker")))),
            ..aplan_evidence::VerifyOptions::default()
        };
        let invalid_schema = root.path().join("invalid-schema.json");
        std::fs::write(
            &invalid_schema,
            serde_json::to_vec(&ReceiptImportSet {
                schema: "wrong".into(),
                receipts: Vec::new(),
            })
            .expect("invalid schema fixture"),
        )
        .expect("invalid schema fixture");
        let error = import_test_receipts(root.path(), &options, &tool(), &invalid_schema)
            .expect_err("schema mismatch");
        assert!(error.to_string().contains(IMPORT_SET_SCHEMA), "{error}");

        let duplicate = root.path().join("duplicate.json");
        let entry = ReceiptImportEntry {
            provider: "receipt".into(),
            path: "receipt.json".into(),
            allow_source_revision_movement: false,
            trust: ReceiptImportTrust::LocalFile,
        };
        std::fs::write(
            &duplicate,
            serde_json::to_vec(&ReceiptImportSet {
                schema: IMPORT_SET_SCHEMA.into(),
                receipts: vec![entry.clone(), entry],
            })
            .expect("duplicate fixture"),
        )
        .expect("duplicate fixture");
        let error = import_test_receipts(root.path(), &options, &tool(), &duplicate)
            .expect_err("duplicate selector");
        assert!(
            error.to_string().contains("appears more than once"),
            "{error}"
        );

        let (repository, _repo) = repository();
        let import = repository.path().join("empty.json");
        std::fs::write(
            &import,
            serde_json::to_vec(&ReceiptImportSet {
                schema: IMPORT_SET_SCHEMA.into(),
                receipts: Vec::new(),
            })
            .expect("empty import fixture"),
        )
        .expect("empty import fixture");
        let missing_inputs = aplan_evidence::VerifyOptions {
            test_runner: options.test_runner.clone(),
            ..aplan_evidence::VerifyOptions::default()
        };
        let error = import_test_receipts(repository.path(), &missing_inputs, &tool(), &import)
            .expect_err("exhaustive inputs required");
        assert!(error.to_string().contains("observation_inputs"), "{error}");
    }

    fn repository() -> (tempfile::TempDir, Repository) {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture root");
        let repo = Repository::init(root.path()).expect("repository");
        std::fs::write(root.path().join("input.txt"), "current\n").expect("input");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("input.txt")).expect("stage");
        index.write().expect("index write");
        let tree_id = index.write_tree().expect("tree");
        {
            let tree = repo.find_tree(tree_id).expect("tree object");
            let signature =
                Signature::now("Aplan Test", "test@example.invalid").expect("signature");
            repo.commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
                .expect("commit");
        }
        (root, repo)
    }

    fn runner(marker: &Path) -> TestRunner {
        serde_json::from_value(serde_json::json!({
            "name": "receipt",
            "list": ["sh", "-c", format!("touch '{}'; exit 91", marker.display())],
            "run": ["false", PLACEHOLDER],
        }))
        .expect("runner")
    }

    fn tool() -> ToolIdentity {
        ToolIdentity {
            version: "test".into(),
            source: "ab".repeat(20),
            facts_sha256: "11".repeat(32),
            binary_sha256: "22".repeat(32),
        }
    }

    fn plan() -> Plan {
        let mut task: Task = serde_json::from_value(serde_json::json!({
            "id": "A",
            "title": "receipt-backed task",
            "phase": 1,
            "status": "done",
        }))
        .expect("task");
        task.evidence.push(Evidence {
            kind: EvidenceKind::Test,
            reference: "suite::case".into(),
            captured: None,
        });
        Plan {
            tasks: vec![LoadedTask {
                task,
                path: PathBuf::from("aplan/tasks/A.md"),
                fields: FieldLines::new(),
                criteria: Criteria::default(),
            }],
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        }
    }

    fn write_receipt_set(root: &Path, configured: &TestRunner, identity: &ToolIdentity) -> PathBuf {
        let material =
            test_material_identity(root, &["input.txt".into()], configured).expect("material");
        let statement = build(ReceiptInput {
            repository: repository_context(root).expect("repository context"),
            runner: configured.clone(),
            observation: TestExecutionObservation {
                runner: TestCatalogueRunner {
                    index: 1,
                    label: "receipt".into(),
                    state: TestCatalogueRunnerState::Listed,
                    names: vec!["suite::case".into()],
                },
                requested: vec!["suite::case".into()],
                outcomes: vec![TestBatchResultEntry {
                    test: "suite::case".into(),
                    outcome: TestBatchOutcome::Passed,
                    attempts: None,
                    diagnostic: None,
                }],
                complete: true,
            },
            material,
            tool: identity.clone(),
            producer: ProducerIdentity {
                repository: "example/project".into(),
                workflow: "local".into(),
                run_id: "1".into(),
                event: "manual".into(),
                privilege: ProducerPrivilege::Untrusted,
            },
            started_at_epoch_seconds: 10,
            completed_at_epoch_seconds: 11,
        })
        .expect("receipt statement");
        std::fs::write(
            root.join("receipt.json"),
            statement.canonical_json().expect("canonical receipt"),
        )
        .expect("receipt");
        let import = ReceiptImportSet {
            schema: IMPORT_SET_SCHEMA.into(),
            receipts: vec![ReceiptImportEntry {
                provider: "receipt".into(),
                path: "receipt.json".into(),
                allow_source_revision_movement: false,
                trust: ReceiptImportTrust::LocalFile,
            }],
        };
        let path = root.join("receipts.json");
        std::fs::write(&path, serde_json::to_vec(&import).expect("import set"))
            .expect("import set write");
        path
    }

    #[test]
    fn current_receipts_derive_a_new_whole_plan_verdict_without_provider_spawns() {
        let (root, _repo) = repository();
        let marker = root.path().join("provider-started");
        let configured = runner(&marker);
        let identity = tool();
        let import_set = write_receipt_set(root.path(), &configured, &identity);
        let options = aplan_evidence::VerifyOptions {
            execute: false,
            test_runner: Some(TestRunners::One(configured)),
            observation_inputs: Some(vec!["input.txt".into()]),
            url_check: Some(aplan_config::UrlCheck {
                check: vec!["true".into(), "{url}".into()],
                timeout_seconds: None,
                max_memory: None,
                max_processes: None,
                max_output: None,
                wrap: None,
            }),
            ..aplan_evidence::VerifyOptions::default()
        };
        let mut current_plan = plan();
        current_plan.tasks[0].task.evidence.push(Evidence {
            kind: EvidenceKind::Url,
            reference: "https://example.invalid/evidence".into(),
            captured: None,
        });
        let result =
            inspect_receipt_evidence(&current_plan, root.path(), &options, &identity, &import_set)
                .expect("receipt-backed verification");
        assert!(
            result.evidence.report.is_clean(),
            "{:?}",
            result.evidence.report.diagnostics
        );
        assert_eq!(result.import.accepted, 1);
        assert_eq!(result.import.rejected, 0);
        assert_eq!(
            result.evidence.observation.scope,
            EvidenceObservationScope::WholePlan
        );
        assert_eq!(
            result.evidence.observation.provider_source,
            EvidenceProviderSource::Receipt
        );
        assert_eq!(
            result.evidence.observation.state,
            EvidenceObservationState::Current
        );
        assert!(
            result
                .evidence
                .citations
                .iter()
                .any(|citation| citation.kind == EvidenceKind::Url)
        );
        assert!(
            result
                .evidence
                .citation_states
                .iter()
                .all(|state| *state == EvidenceObservationState::Current)
        );
        assert!(!marker.exists(), "receipt import must not start test.list");
    }

    #[test]
    fn stale_material_is_a_typed_provider_finding_and_never_falls_back() {
        let (root, _repo) = repository();
        let marker = root.path().join("provider-started");
        let configured = runner(&marker);
        let identity = tool();
        let import_set = write_receipt_set(root.path(), &configured, &identity);
        std::fs::write(root.path().join("input.txt"), "changed\n").expect("move material");
        let options = aplan_evidence::VerifyOptions {
            execute: false,
            test_runner: Some(TestRunners::One(configured)),
            observation_inputs: Some(vec!["input.txt".into()]),
            ..aplan_evidence::VerifyOptions::default()
        };
        let result =
            inspect_receipt_evidence(&plan(), root.path(), &options, &identity, &import_set)
                .expect("stale receipt is a finding");
        assert_eq!(result.import.accepted, 0);
        assert_eq!(result.import.rejected, 1);
        assert_eq!(result.import.freshness, EvidenceObservationState::Stale);
        assert_eq!(
            result.evidence.observation.state,
            EvidenceObservationState::Stale
        );
        let finding = result
            .evidence
            .report
            .diagnostics
            .iter()
            .find(|finding| finding.code == aplan_core::code::TEST_UNRESOLVED)
            .expect("typed stale provider finding");
        assert!(finding.message.contains("stale"), "{}", finding.message);
        assert!(
            !finding.message.contains("receipt.json"),
            "storage paths stay private"
        );
        assert!(!marker.exists(), "stale receipt must not trigger fallback");
    }

    #[test]
    fn unmeasurable_current_material_is_a_typed_finding_not_an_import_crash() {
        let (root, _repo) = repository();
        let marker = root.path().join("provider-started");
        let configured = runner(&marker);
        let identity = tool();
        let import_set = write_receipt_set(root.path(), &configured, &identity);
        let options = aplan_evidence::VerifyOptions {
            execute: false,
            test_runner: Some(TestRunners::One(configured)),
            observation_inputs: Some(vec!["missing/**".into()]),
            ..aplan_evidence::VerifyOptions::default()
        };
        let result =
            inspect_receipt_evidence(&plan(), root.path(), &options, &identity, &import_set)
                .expect("material failure is a provider finding");
        assert_eq!(result.import.accepted, 0);
        assert_eq!(result.import.rejected, 1);
        assert_eq!(result.import.freshness, EvidenceObservationState::Fault);
        let finding = result
            .evidence
            .report
            .diagnostics
            .iter()
            .find(|finding| finding.code == aplan_core::code::TEST_UNRESOLVED)
            .expect("typed provider finding");
        assert!(finding.message.contains("fault"), "{}", finding.message);
        assert!(
            finding.message.contains("matched no regular files"),
            "{}",
            finding.message
        );
        assert!(
            !marker.exists(),
            "material failure must not trigger fallback"
        );
    }

    #[test]
    fn unauthenticated_transport_is_typed_and_does_not_disclose_storage_paths() {
        let (root, _repo) = repository();
        let marker = root.path().join("provider-started");
        let configured = runner(&marker);
        let identity = tool();
        let import_set_path = write_receipt_set(root.path(), &configured, &identity);
        let mut import_set: ReceiptImportSet =
            serde_json::from_slice(&std::fs::read(&import_set_path).expect("read import set"))
                .expect("parse import set");
        import_set.receipts[0].trust = ReceiptImportTrust::SameWorkflow {
            artifact_sha256: "00".repeat(32),
            producer: ProducerIdentity {
                repository: "example/project".into(),
                workflow: "local".into(),
                run_id: "1".into(),
                event: "manual".into(),
                privilege: ProducerPrivilege::Untrusted,
            },
        };
        std::fs::write(
            &import_set_path,
            serde_json::to_vec(&import_set).expect("serialize import set"),
        )
        .expect("rewrite import set");
        let options = aplan_evidence::VerifyOptions {
            execute: false,
            test_runner: Some(TestRunners::One(configured)),
            observation_inputs: Some(vec!["input.txt".into()]),
            ..aplan_evidence::VerifyOptions::default()
        };
        let result =
            inspect_receipt_evidence(&plan(), root.path(), &options, &identity, &import_set_path)
                .expect("trust failure is a provider finding");
        let finding = result
            .evidence
            .report
            .diagnostics
            .iter()
            .find(|finding| finding.code == aplan_core::code::TEST_UNRESOLVED)
            .expect("typed provider finding");
        assert!(finding.message.contains("untrusted"), "{}", finding.message);
        assert_eq!(
            result.evidence.observation.state,
            EvidenceObservationState::Untrusted
        );
        assert!(
            !finding.message.contains("receipt.json"),
            "{}",
            finding.message
        );
        assert!(!marker.exists(), "trust failure must not trigger fallback");
    }
}
