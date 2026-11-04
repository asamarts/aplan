//! Explicit fail-closed refresh of portable test-evidence receipts.
//!
//! Selection changes provider work, never the verdict: after publishing one
//! complete current observation per configured provider, the ordinary
//! receipt-backed whole-plan resolver judges every citation again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use aplan_attest::ToolIdentity;
use aplan_attest::receipt::{
    IMPORT_SET_SCHEMA, MAX_RECEIPT_BYTES, ProducerIdentity, ProducerPrivilege, ReceiptImportEntry,
    ReceiptImportSet, ReceiptImportTrust, ReceiptInput, TestEvidenceStatement, build,
    parse_and_authenticate, provider_identity, repository_context, write_atomic,
};
use aplan_config::{TestInputDependency, TestRunner};
use aplan_core::Plan;
use aplan_core::show::{EvidenceProviderSource, EvidenceRefreshSummary};
use aplan_evidence::{
    ChangedPath, Repo, TestBatchResultEntry, TestExecutionObservation, TestMaterialIdentity,
    test_material_identity, test_material_includes_path, test_runner_by_selector,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use sha2::{Digest as _, Sha256};

use crate::receipt_observation::{read_bounded, resolve_path};

/// The repository snapshot whose changes may invalidate prior outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptRefreshBoundary {
    /// Compare two committed revisions. `head` must be the clean checkout's HEAD.
    Revisions { base: String, head: String },
    /// Compare one committed base with HEAD, index, work tree and untracked files.
    WorkingTree { base: String },
}

/// A complete incremental refresh answer and its machine-readable explanation.
#[derive(Debug)]
pub struct IncrementalReceiptInspection {
    pub evidence: crate::EvidenceInspection,
    pub summary: EvidenceRefreshSummary,
}

/// A refusal before a complete current receipt set can be published.
#[derive(Debug, thiserror::Error)]
pub enum IncrementalReceiptError {
    #[error("incremental receipt refresh requires configured test providers")]
    NoProviders,
    #[error("incremental receipt refresh requires exhaustive `evidence.observation_inputs`")]
    NoInputs,
    #[error("incremental receipt import set could not be read: {0}")]
    Read(String),
    #[error("incremental receipt import set is invalid: {0}")]
    Invalid(String),
    #[error("incremental change boundary is unsafe: {0}")]
    Boundary(String),
    #[error("provider `{0}` is unavailable: {1}")]
    Provider(String, String),
    #[error("receipt publication failed: {0}")]
    Publish(String),
    #[error("incremental and direct-full evidence disagree: {0}")]
    Shadow(String),
    #[error(transparent)]
    Receipt(#[from] crate::receipt_observation::ReceiptImportError),
    #[error(transparent)]
    Evidence(#[from] aplan_evidence::Error),
}

#[derive(Debug)]
struct BoundarySnapshot {
    mode: &'static str,
    base: String,
    head: String,
    changed: Vec<ChangedPath>,
}

#[derive(Debug)]
struct PendingReceipt {
    index: usize,
    label: String,
    runner: TestRunner,
    observation: TestExecutionObservation,
    material: TestMaterialIdentity,
    started_at: u64,
    completed_at: u64,
}

#[derive(Debug)]
struct ProviderRefresh {
    entry: Option<ReceiptImportEntry>,
    pending: Option<PendingReceipt>,
    reused: bool,
    widened: bool,
    selected_tests: usize,
}

#[derive(Debug)]
struct RefreshBatch {
    providers: Vec<ProviderRefresh>,
    materials: Vec<TestMaterialIdentity>,
    reused: Vec<String>,
    refreshed: Vec<String>,
    widened: Vec<String>,
    selected_tests: usize,
}

/// Refresh absent or invalidated provider observations, publish the complete
/// set atomically through the import manifest, and derive a fresh whole-plan
/// verdict. No selection can remove a citation from that final verdict.
pub fn refresh_test_receipts(
    plan: &Plan,
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    tool: &ToolIdentity,
    import_set_path: &Path,
    boundary: &ReceiptRefreshBoundary,
    shadow: bool,
) -> Result<IncrementalReceiptInspection, IncrementalReceiptError> {
    let runners = options
        .test_runner
        .as_ref()
        .ok_or(IncrementalReceiptError::NoProviders)?;
    let declared_inputs = options
        .observation_inputs
        .as_deref()
        .ok_or(IncrementalReceiptError::NoInputs)?;
    let (manifest_bytes, selected) = read_import_set(runners, import_set_path)?;
    let (snapshot, repository_before) = prepare_boundary(repo_root, boundary)?;
    let base = import_set_path.parent().unwrap_or_else(|| Path::new("."));
    let force_full = selected_configuration_changed(
        repo_root,
        options.config_file.as_deref(),
        &snapshot.changed,
    ) || snapshot.changed.iter().any(|path| {
        path.introduced
            && !test_material_includes_path(declared_inputs, &path.path).unwrap_or(false)
    });
    let batch = refresh_configured_providers(
        runners,
        repo_root,
        options.config_file.as_deref(),
        base,
        &selected,
        tool,
        &snapshot,
        declared_inputs,
        force_full,
    )?;
    let repository_after = ensure_refresh_stable(
        repo_root,
        declared_inputs,
        &repository_before,
        &batch.providers,
    )?;

    let (new_manifest, published_manifest) = publish_complete_set(
        repo_root,
        import_set_path,
        declared_inputs,
        tool,
        &repository_after,
        batch.providers,
    )?;
    debug_assert_eq!(new_manifest.receipts.len(), runners.all().len());

    let mut inspection = inspect_published_set(
        plan,
        repo_root,
        options,
        tool,
        import_set_path,
        published_manifest.as_deref(),
        &manifest_bytes,
    )?;
    let summary = EvidenceRefreshSummary {
        mode: snapshot.mode.into(),
        base: snapshot.base,
        head: snapshot.head,
        changed_paths: snapshot.changed.len(),
        reused_providers: batch.reused.len(),
        reused: batch.reused,
        refreshed_providers: batch.refreshed.len(),
        refreshed: batch.refreshed,
        selected_tests: batch.selected_tests,
        widened_providers: batch.widened.len(),
        widened: batch.widened,
        unavailable_providers: 0,
        unavailable: Vec::new(),
        shadow_compared: shadow,
    };
    inspection.evidence.observation.provider_source = EvidenceProviderSource::IncrementalRefresh;
    inspection.evidence.observation.detail = Some(format!(
        "incremental refresh reused {}, refreshed {}, widened {}, selected {} exact test(s), and re-derived every current citation",
        summary.reused_providers,
        summary.refreshed_providers,
        summary.widened_providers,
        summary.selected_tests
    ));
    inspection.evidence.observation.refresh = Some(summary.clone());

    if shadow {
        shadow_compare(
            plan,
            repo_root,
            options,
            runners,
            declared_inputs,
            &batch.materials,
            &repository_after,
            &inspection.evidence,
            import_set_path,
            published_manifest.as_deref(),
            &manifest_bytes,
        )?;
    }

    Ok(IncrementalReceiptInspection {
        evidence: inspection.evidence,
        summary,
    })
}

#[allow(clippy::too_many_arguments)]
fn inspect_published_set(
    plan: &Plan,
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    tool: &ToolIdentity,
    import_set_path: &Path,
    published_manifest: Option<&[u8]>,
    previous_manifest: &[u8],
) -> Result<crate::receipt_observation::ReceiptEvidenceInspection, IncrementalReceiptError> {
    let inspection =
        match crate::inspect_receipt_evidence(plan, repo_root, options, tool, import_set_path) {
            Ok(inspection) => inspection,
            Err(error) => {
                rollback_manifest(import_set_path, published_manifest, previous_manifest)?;
                return Err(error.into());
            }
        };
    if inspection.import.rejected != 0 {
        rollback_manifest(import_set_path, published_manifest, previous_manifest)?;
        return Err(IncrementalReceiptError::Publish(format!(
            "the published set immediately rejected {} provider(s); the previous manifest was restored",
            inspection.import.rejected
        )));
    }
    Ok(inspection)
}

fn read_import_set(
    runners: &aplan_config::TestRunners,
    path: &Path,
) -> Result<(Vec<u8>, BTreeMap<usize, ReceiptImportEntry>), IncrementalReceiptError> {
    let bytes = read_bounded(path, MAX_RECEIPT_BYTES).map_err(IncrementalReceiptError::Read)?;
    let manifest: ReceiptImportSet = serde_json::from_slice(&bytes)
        .map_err(|error| IncrementalReceiptError::Invalid(error.to_string()))?;
    if manifest.schema != IMPORT_SET_SCHEMA {
        return Err(IncrementalReceiptError::Invalid(format!(
            "schema must be `{IMPORT_SET_SCHEMA}`"
        )));
    }
    let mut selected = BTreeMap::new();
    for entry in manifest.receipts {
        let (index, _, _) = test_runner_by_selector(runners, &entry.provider)
            .map_err(IncrementalReceiptError::Invalid)?;
        if selected.insert(index, entry).is_some() {
            return Err(IncrementalReceiptError::Invalid(format!(
                "configured provider {index} appears more than once"
            )));
        }
    }
    Ok((bytes, selected))
}

fn prepare_boundary(
    repo_root: &Path,
    boundary: &ReceiptRefreshBoundary,
) -> Result<(BoundarySnapshot, aplan_attest::receipt::RepositoryContext), IncrementalReceiptError> {
    let repo = Repo::discover(repo_root).map_err(|error| {
        IncrementalReceiptError::Boundary(format!("opening repository: {error}"))
    })?;
    let snapshot = boundary_snapshot(&repo, boundary)?;
    let repository = repository_context(repo_root)
        .map_err(|error| IncrementalReceiptError::Boundary(error.to_string()))?;
    if matches!(boundary, ReceiptRefreshBoundary::Revisions { .. }) {
        if repository.commit != snapshot.head {
            return Err(IncrementalReceiptError::Boundary(format!(
                "selected head `{}` is not the checked-out HEAD `{}`",
                snapshot.head, repository.commit
            )));
        }
        if repository.modified != 0 || repository.untracked != 0 {
            return Err(IncrementalReceiptError::Boundary(
                "base/head mode requires a clean work tree; use the explicit working-tree mode"
                    .into(),
            ));
        }
    }
    Ok((snapshot, repository))
}

#[allow(clippy::too_many_arguments)]
fn refresh_configured_providers(
    runners: &aplan_config::TestRunners,
    repo_root: &Path,
    config_file: Option<&Path>,
    import_base: &Path,
    selected: &BTreeMap<usize, ReceiptImportEntry>,
    tool: &ToolIdentity,
    snapshot: &BoundarySnapshot,
    declared_inputs: &[String],
    force_full: bool,
) -> Result<RefreshBatch, IncrementalReceiptError> {
    let mut batch = RefreshBatch {
        providers: Vec::with_capacity(runners.all().len()),
        materials: Vec::with_capacity(runners.all().len()),
        reused: Vec::new(),
        refreshed: Vec::new(),
        widened: Vec::new(),
        selected_tests: 0,
    };
    for (offset, runner) in runners.all().iter().enumerate() {
        let index = offset + 1;
        let (_, label, _) = test_runner_by_selector(runners, &index.to_string())
            .expect("configured provider resolves by one-based index");
        let material = test_material_identity(repo_root, declared_inputs, runner)
            .map_err(|error| IncrementalReceiptError::Provider(label.clone(), error))?;
        batch.materials.push(material.clone());
        let prior = selected
            .get(&index)
            .map(|entry| authenticate_prior(import_base, entry))
            .transpose()
            .map_err(|error| IncrementalReceiptError::Provider(label.clone(), error))?
            .flatten();
        let provider = provider_identity(index, &label, runner)
            .map_err(|error| IncrementalReceiptError::Provider(label.clone(), error.to_string()))?;
        let refresh = refresh_provider(
            runners,
            repo_root,
            config_file,
            index,
            label.clone(),
            runner,
            selected.get(&index),
            prior.as_ref(),
            &provider.sha256,
            &material,
            tool,
            snapshot,
            declared_inputs,
            force_full,
        )?;
        if refresh.reused {
            batch.reused.push(label.clone());
        } else {
            batch.refreshed.push(label.clone());
        }
        if refresh.widened {
            batch.widened.push(label);
        }
        batch.selected_tests += refresh.selected_tests;
        batch.providers.push(refresh);
    }
    Ok(batch)
}

fn ensure_refresh_stable(
    repo_root: &Path,
    declared_inputs: &[String],
    repository_before: &aplan_attest::receipt::RepositoryContext,
    providers: &[ProviderRefresh],
) -> Result<aplan_attest::receipt::RepositoryContext, IncrementalReceiptError> {
    let repository_after = repository_context(repo_root)
        .map_err(|error| IncrementalReceiptError::Boundary(error.to_string()))?;
    if repository_before != &repository_after {
        return Err(IncrementalReceiptError::Boundary(
            "the repository changed while providers were refreshed; no import set was published"
                .into(),
        ));
    }
    for refresh in providers {
        let Some(pending) = &refresh.pending else {
            continue;
        };
        let after = test_material_identity(repo_root, declared_inputs, &pending.runner)
            .map_err(|error| IncrementalReceiptError::Provider(pending.label.clone(), error))?;
        if after != pending.material {
            return Err(IncrementalReceiptError::Provider(
                pending.label.clone(),
                "declared material changed while refresh was running; nothing was published".into(),
            ));
        }
    }
    Ok(repository_after)
}

#[allow(clippy::too_many_arguments)]
fn shadow_compare(
    plan: &Plan,
    repo_root: &Path,
    options: &aplan_evidence::VerifyOptions,
    runners: &aplan_config::TestRunners,
    declared_inputs: &[String],
    materials_before: &[TestMaterialIdentity],
    repository_before: &aplan_attest::receipt::RepositoryContext,
    incremental: &crate::EvidenceInspection,
    import_set_path: &Path,
    published_manifest: Option<&[u8]>,
    previous_manifest: &[u8],
) -> Result<(), IncrementalReceiptError> {
    let mut direct_options = options.clone();
    direct_options.execute = true;
    let direct = match crate::inspect_evidence(
        plan,
        repo_root,
        &direct_options,
        crate::EvidenceInspectionMode::Refresh,
        &crate::EvidenceObservationOptions::default(),
        |_| {},
    ) {
        Ok(direct) => direct,
        Err(error) => {
            rollback_manifest(import_set_path, published_manifest, previous_manifest)?;
            return Err(error.into());
        }
    };
    if let Err(error) = compare_evidence(incremental, &direct) {
        rollback_manifest(import_set_path, published_manifest, previous_manifest)?;
        return Err(error);
    }
    let repository_after = match repository_context(repo_root) {
        Ok(repository) => repository,
        Err(error) => {
            rollback_manifest(import_set_path, published_manifest, previous_manifest)?;
            return Err(IncrementalReceiptError::Boundary(error.to_string()));
        }
    };
    let material_moved = runners
        .all()
        .iter()
        .zip(materials_before)
        .any(
            |(runner, before)| match test_material_identity(repo_root, declared_inputs, runner) {
                Ok(after) => after != *before,
                Err(_) => true,
            },
        );
    if repository_after != *repository_before || material_moved {
        rollback_manifest(import_set_path, published_manifest, previous_manifest)?;
        return Err(IncrementalReceiptError::Shadow(
            "the direct oracle changed the repository or declared provider material; the previous manifest was restored"
                .into(),
        ));
    }
    Ok(())
}

fn boundary_snapshot(
    repo: &Repo,
    boundary: &ReceiptRefreshBoundary,
) -> Result<BoundarySnapshot, IncrementalReceiptError> {
    match boundary {
        ReceiptRefreshBoundary::Revisions { base, head } => {
            let (base, head, changed) = repo
                .changed_paths_between(base, head)
                .map_err(IncrementalReceiptError::Boundary)?;
            Ok(BoundarySnapshot {
                mode: "base_head",
                base,
                head,
                changed,
            })
        }
        ReceiptRefreshBoundary::WorkingTree { base } => {
            let (base, head, changed) = repo
                .changed_paths_to_worktree(base)
                .map_err(IncrementalReceiptError::Boundary)?;
            Ok(BoundarySnapshot {
                mode: "working_tree",
                base,
                head,
                changed,
            })
        }
    }
}

fn selected_configuration_changed(
    repo_root: &Path,
    config_file: Option<&Path>,
    changed: &[ChangedPath],
) -> bool {
    let Some(config) = config_file.and_then(|path| repository_relative(repo_root, path)) else {
        return false;
    };
    changed.iter().any(|path| path.path == config)
}

fn repository_relative(repo_root: &Path, path: &Path) -> Option<String> {
    if let Ok(relative) = path.strip_prefix(repo_root)
        && relative != path
    {
        return relative.to_str().map(ToString::to_string);
    }
    let root = std::fs::canonicalize(repo_root).ok()?;
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let absolute = if absolute_path.exists() {
        std::fs::canonicalize(absolute_path).ok()?
    } else {
        let parent = std::fs::canonicalize(absolute_path.parent()?).ok()?;
        parent.join(absolute_path.file_name()?)
    };
    absolute
        .strip_prefix(root)
        .ok()?
        .to_str()
        .map(ToString::to_string)
}

fn authenticate_prior(
    base: &Path,
    entry: &ReceiptImportEntry,
) -> Result<Option<TestEvidenceStatement>, String> {
    let path = resolve_path(base, &entry.path);
    let Ok(bytes) = read_bounded(&path, MAX_RECEIPT_BYTES) else {
        return Ok(None);
    };
    let (policy, transport) = entry.trust.verifier_inputs();
    parse_and_authenticate(&bytes, &policy, &transport)
        .map(Some)
        .map_err(|error| format!("the supplied prior receipt cannot be trusted: {error}"))
}

#[allow(clippy::too_many_arguments)]
fn reuse_prior(
    index: usize,
    label: &str,
    runner: &TestRunner,
    existing_entry: Option<&ReceiptImportEntry>,
    prior: Option<&TestEvidenceStatement>,
    provider_sha256: &str,
    current_material: &TestMaterialIdentity,
    tool: &ToolIdentity,
    snapshot: &BoundarySnapshot,
    force_full: bool,
) -> Result<Option<ProviderRefresh>, IncrementalReceiptError> {
    let Some(prior) = prior else {
        refuse_trusted_replacement(label, existing_entry)?;
        return Ok(None);
    };
    let predicate = &prior.predicate;
    let revision_is_current = predicate.repository.commit == snapshot.head
        || existing_entry.is_some_and(|entry| entry.allow_source_revision_movement);
    let identities_match = predicate.provider.sha256 == provider_sha256
        && predicate.material == *current_material
        && predicate.tool == *tool;
    if identities_match && revision_is_current && !force_full {
        return Ok(Some(ProviderRefresh {
            entry: existing_entry.cloned(),
            pending: None,
            reused: true,
            widened: false,
            selected_tests: 0,
        }));
    }

    // A receipt admitted through an authenticated transport may be reused
    // unchanged, but this local process cannot mint its successor. Check this
    // before revision-only repackaging, or an unrelated commit would silently
    // replace same-workflow or signed provenance with a local-file receipt.
    refuse_trusted_replacement(label, existing_entry)?;
    if identities_match && !force_full {
        return Ok(Some(ProviderRefresh {
            entry: None,
            pending: Some(PendingReceipt {
                index,
                label: label.into(),
                runner: runner.clone(),
                observation: observation_from(prior),
                material: current_material.clone(),
                started_at: predicate.started_at_epoch_seconds,
                completed_at: predicate.completed_at_epoch_seconds,
            }),
            reused: true,
            widened: false,
            selected_tests: 0,
        }));
    }
    Ok(None)
}

fn refuse_trusted_replacement(
    label: &str,
    existing_entry: Option<&ReceiptImportEntry>,
) -> Result<(), IncrementalReceiptError> {
    if existing_entry.is_some_and(|entry| entry.trust != ReceiptImportTrust::LocalFile) {
        return Err(IncrementalReceiptError::Provider(
            label.into(),
            "an authenticated artifact or signed receipt cannot be replaced by a local refresh; refresh it inside its authenticated producer and rebuild the trusted import set"
                .into(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn refresh_provider(
    runners: &aplan_config::TestRunners,
    repo_root: &Path,
    config_file: Option<&Path>,
    index: usize,
    label: String,
    runner: &TestRunner,
    existing_entry: Option<&ReceiptImportEntry>,
    prior: Option<&TestEvidenceStatement>,
    provider_sha256: &str,
    current_material: &TestMaterialIdentity,
    tool: &ToolIdentity,
    snapshot: &BoundarySnapshot,
    declared_inputs: &[String],
    force_full: bool,
) -> Result<ProviderRefresh, IncrementalReceiptError> {
    if let Some(reused) = reuse_prior(
        index,
        &label,
        runner,
        existing_entry,
        prior,
        provider_sha256,
        current_material,
        tool,
        snapshot,
        force_full,
    )? {
        return Ok(reused);
    }

    let started_at = epoch_seconds()?;
    let mut narrowed = false;
    let mut selected_count = 0usize;
    let can_narrow = prior.is_some_and(|statement| {
        statement.predicate.provider.sha256 == provider_sha256
            && statement.predicate.tool == *tool
            && statement.predicate.repository.commit == snapshot.base
            && statement.predicate.material.environment_sha256
                == current_material.environment_sha256
            && statement.predicate.material.toolchain_sha256 == current_material.toolchain_sha256
    }) && !force_full;

    let fresh = if can_narrow && runner.inputs_by_test.is_some() {
        let old = prior.expect("can_narrow requires prior");
        let mut widened = false;
        let observed = aplan_evidence::observe_test_runner_selected(
            runners,
            repo_root,
            config_file.map(Path::to_path_buf),
            &index.to_string(),
            |names| {
                let selection = select_tests(
                    runner.inputs_by_test.as_deref().unwrap_or_default(),
                    declared_inputs,
                    &snapshot.changed,
                    names,
                    &old.predicate.catalogue.names,
                    &old.predicate.outcomes,
                )?;
                widened = selection.widened;
                selected_count = selection.names.len();
                Ok(selection.names)
            },
        )
        .map_err(|error| IncrementalReceiptError::Provider(label.clone(), error))?;
        if widened {
            narrowed = false;
            observed
        } else {
            narrowed = true;
            merge_observations(old, observed)
                .map_err(|error| IncrementalReceiptError::Provider(label.clone(), error))?
        }
    } else {
        let observed = aplan_evidence::observe_test_runner(
            runners,
            repo_root,
            config_file.map(Path::to_path_buf),
            &index.to_string(),
        )
        .map_err(|error| IncrementalReceiptError::Provider(label.clone(), error))?;
        selected_count = observed.requested.len();
        observed
    };
    if !fresh.complete {
        return Err(IncrementalReceiptError::Provider(
            label,
            "the refreshed provider did not return a complete attributable outcome set".into(),
        ));
    }
    let completed_at = epoch_seconds()?;
    Ok(ProviderRefresh {
        entry: None,
        pending: Some(PendingReceipt {
            index,
            label,
            runner: runner.clone(),
            observation: fresh,
            material: current_material.clone(),
            started_at,
            completed_at,
        }),
        reused: false,
        widened: !narrowed,
        selected_tests: selected_count,
    })
}

#[derive(Debug)]
struct Selection {
    names: Vec<String>,
    widened: bool,
}

fn select_tests(
    declarations: &[TestInputDependency],
    material_inputs: &[String],
    changed: &[ChangedPath],
    current_names: &[String],
    old_names: &[String],
    old_outcomes: &[TestBatchResultEntry],
) -> Result<Selection, String> {
    let compiled = declarations
        .iter()
        .map(CompiledDependency::new)
        .collect::<Result<Vec<_>, _>>()?;
    let mut owner = BTreeMap::<&str, usize>::new();
    for name in current_names {
        let matches = compiled
            .iter()
            .enumerate()
            .filter_map(|(index, dependency)| dependency.tests.is_match(name).then_some(index))
            .collect::<Vec<_>>();
        let [index] = matches.as_slice() else {
            return Ok(Selection {
                names: current_names.to_vec(),
                widened: true,
            });
        };
        owner.insert(name, *index);
    }

    let mut affected_entries = BTreeSet::new();
    for path in changed {
        let matches = compiled
            .iter()
            .enumerate()
            .filter_map(|(index, dependency)| {
                dependency.paths.is_match(&path.path).then_some(index)
            })
            .collect::<Vec<_>>();
        let in_material = test_material_includes_path(material_inputs, &path.path)?;
        let relevant = path.introduced || in_material;
        if (!matches.is_empty() && !in_material) || (relevant && matches.len() != 1) {
            return Ok(Selection {
                names: current_names.to_vec(),
                widened: true,
            });
        }
        affected_entries.extend(matches);
    }

    let old_names = old_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let old_outcomes = old_outcomes
        .iter()
        .map(|outcome| outcome.test.as_str())
        .collect::<BTreeSet<_>>();
    let names = current_names
        .iter()
        .filter(|name| {
            !old_names.contains(name.as_str())
                || !old_outcomes.contains(name.as_str())
                || owner
                    .get(name.as_str())
                    .is_some_and(|entry| affected_entries.contains(entry))
        })
        .cloned()
        .collect();
    Ok(Selection {
        names,
        widened: false,
    })
}

struct CompiledDependency {
    tests: GlobSet,
    paths: GlobSet,
}

impl CompiledDependency {
    fn new(dependency: &TestInputDependency) -> Result<Self, String> {
        Ok(Self {
            tests: compile_globs(&dependency.tests)?,
            paths: compile_globs(&dependency.paths)?,
        })
    }
}

fn compile_globs(patterns: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|error| error.to_string())?);
    }
    builder.build().map_err(|error| error.to_string())
}

fn observation_from(statement: &TestEvidenceStatement) -> TestExecutionObservation {
    TestExecutionObservation {
        runner: aplan_evidence::TestCatalogueRunner {
            index: statement.predicate.provider.index,
            label: statement.predicate.provider.label.clone(),
            state: aplan_evidence::TestCatalogueRunnerState::Listed,
            names: statement.predicate.catalogue.names.clone(),
        },
        requested: statement.predicate.requested.clone(),
        outcomes: statement.predicate.outcomes.clone(),
        complete: statement.predicate.complete,
    }
}

fn merge_observations(
    old: &TestEvidenceStatement,
    fresh: TestExecutionObservation,
) -> Result<TestExecutionObservation, String> {
    let mut outcomes = old
        .predicate
        .outcomes
        .iter()
        .cloned()
        .map(|outcome| (outcome.test.clone(), outcome))
        .collect::<BTreeMap<_, _>>();
    for outcome in fresh.outcomes {
        outcomes.insert(outcome.test.clone(), outcome);
    }
    let mut merged = Vec::with_capacity(fresh.runner.names.len());
    for name in &fresh.runner.names {
        let outcome = outcomes
            .remove(name)
            .ok_or_else(|| format!("no current or refreshed outcome exists for `{name}`"))?;
        merged.push(outcome);
    }
    Ok(TestExecutionObservation {
        runner: fresh.runner,
        requested: merged.iter().map(|outcome| outcome.test.clone()).collect(),
        outcomes: merged,
        complete: fresh.complete,
    })
}

fn publish_complete_set(
    repo_root: &Path,
    import_set_path: &Path,
    material_inputs: &[String],
    tool: &ToolIdentity,
    repository: &aplan_attest::receipt::RepositoryContext,
    refreshes: Vec<ProviderRefresh>,
) -> Result<(ReceiptImportSet, Option<Vec<u8>>), IncrementalReceiptError> {
    let base = import_set_path.parent().unwrap_or_else(|| Path::new("."));
    let mut entries = Vec::with_capacity(refreshes.len());
    let producer = ProducerIdentity {
        repository: "local-refresh".into(),
        workflow: "aplan-incremental-refresh".into(),
        run_id: epoch_nanos()?.to_string(),
        event: "local".into(),
        privilege: ProducerPrivilege::Untrusted,
    };
    let mut documents = Vec::new();
    for refresh in refreshes {
        if let Some(entry) = refresh.entry {
            entries.push(entry);
            continue;
        }
        let pending = refresh.pending.ok_or_else(|| {
            IncrementalReceiptError::Publish(
                "a provider had neither a reusable entry nor a refreshed observation".into(),
            )
        })?;
        let statement = build(ReceiptInput {
            repository: repository.clone(),
            runner: pending.runner,
            observation: pending.observation,
            material: pending.material,
            tool: tool.clone(),
            producer: producer.clone(),
            started_at_epoch_seconds: pending.started_at,
            completed_at_epoch_seconds: pending.completed_at,
        })
        .map_err(|error| {
            IncrementalReceiptError::Provider(pending.label.clone(), error.to_string())
        })?;
        let bytes = statement.canonical_json().map_err(|error| {
            IncrementalReceiptError::Provider(pending.label.clone(), error.to_string())
        })?;
        let digest = sha256(&bytes);
        let file_name = format!(".aplan-test-receipt-{}-{digest}.json", pending.index);
        let destination = base.join(&file_name);
        refuse_material_output(repo_root, &destination, material_inputs)?;
        documents.push((destination, bytes));
        entries.push(ReceiptImportEntry {
            provider: pending.index.to_string(),
            path: file_name,
            allow_source_revision_movement: false,
            trust: ReceiptImportTrust::LocalFile,
        });
    }
    if documents.is_empty() {
        Ok((
            ReceiptImportSet {
                schema: IMPORT_SET_SCHEMA.into(),
                receipts: entries,
            },
            None,
        ))
    } else {
        refuse_material_output(repo_root, import_set_path, material_inputs)?;
        for (path, bytes) in &documents {
            write_atomic(path, bytes)
                .map_err(|error| IncrementalReceiptError::Publish(error.to_string()))?;
        }
        let manifest = ReceiptImportSet {
            schema: IMPORT_SET_SCHEMA.into(),
            receipts: entries,
        };
        let mut bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| IncrementalReceiptError::Publish(error.to_string()))?;
        bytes.push(b'\n');
        write_atomic(import_set_path, &bytes)
            .map_err(|error| IncrementalReceiptError::Publish(error.to_string()))?;
        Ok((manifest, Some(bytes)))
    }
}

fn rollback_manifest(
    path: &Path,
    published: Option<&[u8]>,
    previous: &[u8],
) -> Result<(), IncrementalReceiptError> {
    let Some(published) = published else {
        return Ok(());
    };
    let current = read_bounded(path, MAX_RECEIPT_BYTES).map_err(|error| {
        IncrementalReceiptError::Publish(format!(
            "the new manifest could not be checked for rollback after verification failed: {error}"
        ))
    })?;
    if current != published {
        return Err(IncrementalReceiptError::Publish(
            "the import manifest changed after publication; refusing to overwrite the concurrent writer while rolling back a failed verification"
                .into(),
        ));
    }
    write_atomic(path, previous).map_err(|error| {
        IncrementalReceiptError::Publish(format!(
            "restoring the previous manifest after verification failed: {error}"
        ))
    })
}

fn refuse_material_output(
    repo_root: &Path,
    path: &Path,
    material_inputs: &[String],
) -> Result<(), IncrementalReceiptError> {
    let Some(relative) = repository_relative(repo_root, path) else {
        return Ok(());
    };
    if test_material_includes_path(material_inputs, &relative)
        .map_err(IncrementalReceiptError::Publish)?
    {
        return Err(IncrementalReceiptError::Publish(format!(
            "receipt output `{relative}` is inside `evidence.observation_inputs`; publishing it would invalidate its own result"
        )));
    }
    let repo = Repo::discover(repo_root)
        .map_err(|error| IncrementalReceiptError::Publish(error.to_string()))?;
    if !repo
        .path_is_ignored(Path::new(&relative))
        .map_err(IncrementalReceiptError::Publish)?
    {
        return Err(IncrementalReceiptError::Publish(format!(
            "receipt output `{relative}` is inside the repository and is not ignored by Git; place refresh artifacts outside the checkout or ignore their directory so they cannot invalidate the next working-tree boundary"
        )));
    }
    Ok(())
}

fn compare_evidence(
    incremental: &crate::EvidenceInspection,
    direct: &crate::EvidenceInspection,
) -> Result<(), IncrementalReceiptError> {
    let diagnostics = |inspection: &crate::EvidenceInspection| {
        inspection
            .report
            .diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.code.to_string(),
                    diagnostic.task.clone(),
                    diagnostic.file.clone(),
                    diagnostic.message.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let citations = |inspection: &crate::EvidenceInspection| {
        inspection
            .citations
            .iter()
            .map(|citation| {
                (
                    citation.task.clone(),
                    citation.kind.as_str().to_string(),
                    citation.reference.clone(),
                    citation.resolution.outcome.label().to_string(),
                )
            })
            .collect::<Vec<_>>()
    };
    if diagnostics(incremental) != diagnostics(direct) {
        return Err(IncrementalReceiptError::Shadow(
            "diagnostic sets differ".into(),
        ));
    }
    if citations(incremental) != citations(direct) {
        return Err(IncrementalReceiptError::Shadow(
            "per-citation outcomes differ".into(),
        ));
    }
    Ok(())
}

fn epoch_seconds() -> Result<u64, IncrementalReceiptError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| IncrementalReceiptError::Boundary("system clock is before Unix epoch".into()))
}

fn epoch_nanos() -> Result<u128, IncrementalReceiptError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|_| IncrementalReceiptError::Boundary("system clock is before Unix epoch".into()))
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_evidence::TestBatchOutcome;

    fn runners() -> aplan_config::TestRunners {
        aplan_config::parse_config(
            "test:\n  name: unit\n  list: [git, -c, 'aplan.tests=alpha', config, --get, aplan.tests]\n  run: [git, rev-parse, --sq-quote, '{test}']\n  run_selects: true\n",
            Path::new(".aplan.yml"),
        )
        .expect("test config")
        .test
        .expect("test runner")
    }

    fn git(repo: &Path, arguments: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(repo)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git output UTF-8")
            .trim()
            .to_string()
    }

    fn committed_repository() -> (tempfile::TempDir, String) {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        git(directory.path(), &["init", "-q", "."]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(directory.path(), &["config", "user.name", "Test"]);
        std::fs::write(directory.path().join("input.txt"), "one\n").expect("input");
        git(directory.path(), &["add", "input.txt"]);
        git(directory.path(), &["commit", "-q", "-m", "base"]);
        let head = git(directory.path(), &["rev-parse", "HEAD"]);
        (directory, head)
    }

    fn empty_plan() -> Plan {
        Plan {
            tasks: Vec::new(),
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        }
    }

    fn empty_inspection() -> crate::EvidenceInspection {
        crate::EvidenceInspection {
            report: aplan_core::Report::default(),
            citations: Vec::new(),
            observation: aplan_core::show::EvidenceObservation::current(),
            spend: Vec::new(),
            citation_states: Vec::new(),
        }
    }

    fn dependency(tests: &[&str], paths: &[&str]) -> TestInputDependency {
        TestInputDependency {
            tests: tests.iter().map(|value| (*value).to_string()).collect(),
            paths: paths.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    fn outcome(name: &str) -> TestBatchResultEntry {
        TestBatchResultEntry {
            test: name.into(),
            outcome: TestBatchOutcome::Passed,
            attempts: None,
            diagnostic: None,
        }
    }

    #[test]
    fn import_set_rejects_unreadable_invalid_wrong_schema_and_duplicate_providers() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let path = directory.path().join("receipts.json");
        let configured = runners();

        let error = read_import_set(&configured, &path).expect_err("missing manifest");
        assert!(matches!(error, IncrementalReceiptError::Read(_)), "{error}");

        std::fs::write(&path, "not JSON\n").expect("invalid manifest");
        let error = read_import_set(&configured, &path).expect_err("invalid manifest");
        assert!(
            matches!(error, IncrementalReceiptError::Invalid(_)),
            "{error}"
        );

        std::fs::write(&path, r#"{"schema":"wrong","receipts":[]}"#).expect("wrong schema");
        let error = read_import_set(&configured, &path).expect_err("wrong schema");
        assert!(error.to_string().contains(IMPORT_SET_SCHEMA), "{error}");

        let entry = serde_json::json!({
            "provider": "unit",
            "path": "receipt.json",
            "trust": {"mode": "local_file"}
        });
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema": IMPORT_SET_SCHEMA,
                "receipts": [entry.clone(), entry]
            }))
            .expect("manifest JSON"),
        )
        .expect("duplicate manifest");
        let error = read_import_set(&configured, &path).expect_err("duplicate provider");
        assert!(
            error.to_string().contains("appears more than once"),
            "{error}"
        );

        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema": IMPORT_SET_SCHEMA,
                "receipts": [{
                    "provider": "not-configured",
                    "path": "receipt.json",
                    "trust": {"mode": "local_file"}
                }]
            }))
            .expect("manifest JSON"),
        )
        .expect("unknown-provider manifest");
        let error = read_import_set(&configured, &path).expect_err("unknown provider");
        assert!(error.to_string().contains("not-configured"), "{error}");
    }

    #[test]
    fn committed_boundary_refuses_non_repository_wrong_head_and_dirty_tree() {
        let outside = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let error = prepare_boundary(
            outside.path(),
            &ReceiptRefreshBoundary::Revisions {
                base: "HEAD".into(),
                head: "HEAD".into(),
            },
        )
        .expect_err("non-repository");
        assert!(error.to_string().contains("opening repository"), "{error}");

        let (directory, base) = committed_repository();
        std::fs::write(directory.path().join("input.txt"), "two\n").expect("change");
        git(directory.path(), &["add", "input.txt"]);
        git(directory.path(), &["commit", "-q", "-m", "head"]);
        let current = git(directory.path(), &["rev-parse", "HEAD"]);
        let error = prepare_boundary(
            directory.path(),
            &ReceiptRefreshBoundary::Revisions {
                base: base.clone(),
                head: base,
            },
        )
        .expect_err("selected head is not checkout");
        assert!(
            error.to_string().contains("not the checked-out HEAD"),
            "{error}"
        );

        std::fs::write(directory.path().join("untracked"), "dirty\n").expect("dirty");
        let error = prepare_boundary(
            directory.path(),
            &ReceiptRefreshBoundary::Revisions {
                base: current.clone(),
                head: current,
            },
        )
        .expect_err("dirty committed boundary");
        assert!(
            error.to_string().contains("requires a clean work tree"),
            "{error}"
        );
    }

    #[test]
    fn stability_gate_refuses_repository_or_declared_material_movement() {
        let (directory, _) = committed_repository();
        let before = repository_context(directory.path()).expect("repository context");
        std::fs::write(directory.path().join("untracked"), "dirty\n").expect("dirty");
        let error = ensure_refresh_stable(directory.path(), &["input.txt".into()], &before, &[])
            .expect_err("repository moved");
        assert!(error.to_string().contains("repository changed"), "{error}");
        std::fs::remove_file(directory.path().join("untracked")).expect("clean");

        let runner = runners().all()[0].clone();
        let actual = test_material_identity(directory.path(), &["input.txt".into()], &runner)
            .expect("material");
        let mut wrong = actual.clone();
        wrong.sha256 = "0".repeat(64);
        let pending = ProviderRefresh {
            entry: None,
            pending: Some(PendingReceipt {
                index: 1,
                label: "unit".into(),
                runner,
                observation: TestExecutionObservation {
                    runner: aplan_evidence::TestCatalogueRunner {
                        index: 1,
                        label: "unit".into(),
                        state: aplan_evidence::TestCatalogueRunnerState::Listed,
                        names: vec!["alpha".into()],
                    },
                    requested: vec!["alpha".into()],
                    outcomes: vec![outcome("alpha")],
                    complete: true,
                },
                material: wrong,
                started_at: 1,
                completed_at: 2,
            }),
            reused: false,
            widened: false,
            selected_tests: 1,
        };
        let error =
            ensure_refresh_stable(directory.path(), &["input.txt".into()], &before, &[pending])
                .expect_err("material moved");
        assert!(error.to_string().contains("material changed"), "{error}");
    }

    #[test]
    fn helper_refusals_are_fail_closed_and_leave_caller_owned_files_alone() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let manifest = directory.path().join("receipts.json");
        assert!(
            authenticate_prior(
                directory.path(),
                &ReceiptImportEntry {
                    provider: "unit".into(),
                    path: "absent.json".into(),
                    allow_source_revision_movement: false,
                    trust: ReceiptImportTrust::LocalFile,
                },
            )
            .expect("missing prior is refreshable")
            .is_none()
        );

        rollback_manifest(&manifest, None, b"old\n").expect("nothing published");
        let error = rollback_manifest(&manifest, Some(b"new\n"), b"old\n")
            .expect_err("missing publication cannot be rolled back blindly");
        assert!(
            error.to_string().contains("could not be checked"),
            "{error}"
        );

        let repository = aplan_attest::receipt::RepositoryContext {
            commit: "0".repeat(40),
            tree_sha256: "0".repeat(64),
            modified: 0,
            untracked: 0,
            status_sha256: "0".repeat(64),
        };
        let error = publish_complete_set(
            directory.path(),
            &manifest,
            &["src/**".into()],
            &ToolIdentity::default(),
            &repository,
            vec![ProviderRefresh {
                entry: None,
                pending: None,
                reused: false,
                widened: false,
                selected_tests: 0,
            }],
        )
        .expect_err("incomplete refresh row");
        assert!(
            error.to_string().contains("neither a reusable entry"),
            "{error}"
        );

        assert!(compile_globs(&["[".into()]).is_err());
        assert!(
            refuse_material_output(
                directory.path(),
                Path::new("/outside/the/repository/receipts.json"),
                &["src/**".into()],
            )
            .is_ok()
        );

        let invalid_pending = ProviderRefresh {
            entry: None,
            pending: Some(PendingReceipt {
                index: 1,
                label: "unit".into(),
                runner: runners().all()[0].clone(),
                observation: TestExecutionObservation {
                    runner: aplan_evidence::TestCatalogueRunner {
                        index: 1,
                        label: "unit".into(),
                        state: aplan_evidence::TestCatalogueRunnerState::Listed,
                        names: vec!["alpha".into()],
                    },
                    requested: vec!["alpha".into()],
                    outcomes: vec![outcome("alpha")],
                    complete: true,
                },
                material: TestMaterialIdentity {
                    inputs_sha256: "not a digest".into(),
                    environment_sha256: "0".repeat(64),
                    toolchain_sha256: "0".repeat(64),
                    sha256: "0".repeat(64),
                },
                started_at: 1,
                completed_at: 2,
            }),
            reused: false,
            widened: true,
            selected_tests: 1,
        };
        let error = publish_complete_set(
            directory.path(),
            &manifest,
            &["src/**".into()],
            &ToolIdentity {
                version: "test".into(),
                source: String::new(),
                facts_sha256: "0".repeat(64),
                binary_sha256: "0".repeat(64),
            },
            &repository,
            vec![invalid_pending],
        )
        .expect_err("invalid pending receipt");
        assert!(
            matches!(error, IncrementalReceiptError::Provider(_, _)),
            "{error}"
        );
    }

    #[test]
    fn incomplete_provider_refresh_is_never_publishable() {
        let (directory, head) = committed_repository();
        let mut runner = runners().all()[0].clone();
        runner.batch = Some(aplan_config::TestBatch {
            protocol: aplan_config::TEST_BATCH_PROTOCOL,
            run: vec![
                "git".into(),
                "config".into(),
                "--get".into(),
                "aplan.missing".into(),
                aplan_config::BATCH_MANIFEST_PLACEHOLDER.into(),
                aplan_config::BATCH_RESULT_PLACEHOLDER.into(),
            ],
        });
        let configured = aplan_config::TestRunners::One(runner.clone());
        let material = test_material_identity(directory.path(), &["input.txt".into()], &runner)
            .expect("material");
        let provider = provider_identity(1, "unit", &runner).expect("provider identity");
        let error = refresh_provider(
            &configured,
            directory.path(),
            None,
            1,
            "unit".into(),
            &runner,
            None,
            None,
            &provider.sha256,
            &material,
            &ToolIdentity::default(),
            &BoundarySnapshot {
                mode: "base_head",
                base: head.clone(),
                head,
                changed: Vec::new(),
            },
            &["input.txt".into()],
            true,
        )
        .expect_err("incomplete observation");
        assert!(
            error
                .to_string()
                .contains("did not return a complete attributable outcome set"),
            "{error}"
        );
    }

    #[test]
    fn published_set_failure_and_rejection_both_restore_the_previous_manifest() {
        let (directory, _) = committed_repository();
        let manifest = directory.path().join("receipts.json");
        let published = serde_json::to_vec(&serde_json::json!({
            "schema": IMPORT_SET_SCHEMA,
            "receipts": []
        }))
        .expect("published manifest");
        let previous = b"previous manifest\n";

        std::fs::write(&manifest, &published).expect("published manifest");
        let error = inspect_published_set(
            &empty_plan(),
            directory.path(),
            &aplan_evidence::VerifyOptions::default(),
            &ToolIdentity::default(),
            &manifest,
            Some(&published),
            previous,
        )
        .expect_err("missing providers");
        assert!(
            matches!(error, IncrementalReceiptError::Receipt(_)),
            "{error}"
        );
        assert_eq!(std::fs::read(&manifest).expect("restored"), previous);

        std::fs::write(&manifest, &published).expect("republished manifest");
        let options = aplan_evidence::VerifyOptions {
            test_runner: Some(runners()),
            observation_inputs: Some(vec!["input.txt".into()]),
            ..aplan_evidence::VerifyOptions::default()
        };
        let error = inspect_published_set(
            &empty_plan(),
            directory.path(),
            &options,
            &ToolIdentity::default(),
            &manifest,
            Some(&published),
            previous,
        )
        .expect_err("an absent provider receipt rejects the published set");
        assert!(
            error.to_string().contains("immediately rejected 1"),
            "{error}"
        );
        assert_eq!(std::fs::read(&manifest).expect("restored"), previous);
    }

    #[test]
    fn shadow_comparison_checks_diagnostic_subjects_and_each_citation_outcome() {
        let left = empty_inspection();
        let right = empty_inspection();
        compare_evidence(&left, &right).expect("identical evidence");

        let mut diagnostic_difference = empty_inspection();
        diagnostic_difference.report.diagnostics.push(
            aplan_core::Diagnostic::error(
                aplan_core::code::TEST_UNRESOLVED,
                "different diagnostic",
            )
            .with_task("A-1")
            .with_file(Path::new("aplan/tasks/A-1.md")),
        );
        let error =
            compare_evidence(&left, &diagnostic_difference).expect_err("diagnostic mismatch");
        assert!(
            error.to_string().contains("diagnostic sets differ"),
            "{error}"
        );

        let citation = |outcome| aplan_evidence::CitationObservation {
            task: "A-1".into(),
            kind: aplan_core::EvidenceKind::Test,
            reference: "suite::case".into(),
            resolution: aplan_evidence::CitationResolution {
                resolver: "test".into(),
                strength: aplan_evidence::CitationStrength::Executed,
                outcome,
                detail: None,
                cost_subject: None,
            },
        };
        let mut resolved = empty_inspection();
        resolved
            .citations
            .push(citation(aplan_evidence::CitationOutcome::Resolved));
        let mut refused = empty_inspection();
        refused
            .citations
            .push(citation(aplan_evidence::CitationOutcome::Refused));
        let error = compare_evidence(&resolved, &refused).expect_err("citation mismatch");
        assert!(
            error.to_string().contains("per-citation outcomes differ"),
            "{error}"
        );
    }

    #[test]
    fn shadow_oracle_failure_or_material_movement_restores_the_previous_manifest() {
        let outside = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let manifest = outside.path().join("receipts.json");
        let published = b"published\n";
        let previous = b"previous\n";
        std::fs::write(&manifest, published).expect("published manifest");
        let configured = runners();
        let options = aplan_evidence::VerifyOptions {
            test_runner: Some(configured.clone()),
            ..aplan_evidence::VerifyOptions::default()
        };
        let error = shadow_compare(
            &empty_plan(),
            outside.path(),
            &options,
            &configured,
            &["input.txt".into()],
            &[],
            &aplan_attest::receipt::RepositoryContext {
                commit: String::new(),
                tree_sha256: String::new(),
                modified: 0,
                untracked: 0,
                status_sha256: String::new(),
            },
            &empty_inspection(),
            &manifest,
            Some(published),
            previous,
        )
        .expect_err("direct oracle cannot verify outside a repository");
        assert!(
            matches!(error, IncrementalReceiptError::Evidence(_)),
            "{error}"
        );
        assert_eq!(std::fs::read(&manifest).expect("restored"), previous);

        let (directory, _) = committed_repository();
        let manifest = directory.path().join("receipts.json");
        std::fs::write(&manifest, published).expect("published manifest");
        let repository = repository_context(directory.path()).expect("repository context");
        let wrong_material = TestMaterialIdentity {
            inputs_sha256: "0".repeat(64),
            environment_sha256: "0".repeat(64),
            toolchain_sha256: "0".repeat(64),
            sha256: "0".repeat(64),
        };
        let error = shadow_compare(
            &empty_plan(),
            directory.path(),
            &options,
            &configured,
            &["input.txt".into()],
            &[wrong_material],
            &repository,
            &empty_inspection(),
            &manifest,
            Some(published),
            previous,
        )
        .expect_err("shadow material moved");
        assert!(
            error
                .to_string()
                .contains("changed the repository or declared provider material"),
            "{error}"
        );
        assert_eq!(std::fs::read(&manifest).expect("restored"), previous);
    }

    #[test]
    fn refresh_entry_point_refuses_missing_provider_and_material_configuration_first() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let boundary = ReceiptRefreshBoundary::WorkingTree {
            base: "HEAD".into(),
        };
        let error = refresh_test_receipts(
            &empty_plan(),
            directory.path(),
            &aplan_evidence::VerifyOptions::default(),
            &ToolIdentity::default(),
            &directory.path().join("receipts.json"),
            &boundary,
            false,
        )
        .expect_err("providers are required");
        assert!(matches!(error, IncrementalReceiptError::NoProviders));

        let options = aplan_evidence::VerifyOptions {
            test_runner: Some(runners()),
            ..aplan_evidence::VerifyOptions::default()
        };
        let error = refresh_test_receipts(
            &empty_plan(),
            directory.path(),
            &options,
            &ToolIdentity::default(),
            &directory.path().join("receipts.json"),
            &boundary,
            false,
        )
        .expect_err("material inputs are required");
        assert!(matches!(error, IncrementalReceiptError::NoInputs));
    }

    #[test]
    fn configuration_and_merge_helpers_cover_absence_and_incomplete_history() {
        let (directory, _) = committed_repository();
        let config = directory.path().join(".aplan.yml");
        assert!(!selected_configuration_changed(
            directory.path(),
            None,
            &[ChangedPath {
                path: ".aplan.yml".into(),
                introduced: false,
            }],
        ));
        assert!(selected_configuration_changed(
            directory.path(),
            Some(&config),
            &[ChangedPath {
                path: ".aplan.yml".into(),
                introduced: false,
            }],
        ));

        let old = TestEvidenceStatement {
            statement_type: String::new(),
            subject: Vec::new(),
            predicate_type: String::new(),
            predicate: aplan_attest::receipt::TestEvidencePredicate {
                schema: String::new(),
                repository: aplan_attest::receipt::RepositoryContext {
                    commit: String::new(),
                    tree_sha256: String::new(),
                    modified: 0,
                    untracked: 0,
                    status_sha256: String::new(),
                },
                provider: aplan_attest::receipt::ReceiptProvider {
                    index: 1,
                    label: "unit".into(),
                    definition: serde_json::Value::Null,
                    sha256: String::new(),
                },
                catalogue: aplan_attest::receipt::ReceiptCatalogue {
                    complete: true,
                    names: vec!["alpha".into()],
                    sha256: String::new(),
                },
                requested: vec!["alpha".into()],
                outcomes: vec![outcome("alpha")],
                complete: true,
                material: TestMaterialIdentity {
                    inputs_sha256: String::new(),
                    environment_sha256: String::new(),
                    toolchain_sha256: String::new(),
                    sha256: String::new(),
                },
                tool: ToolIdentity::default(),
                producer: ProducerIdentity {
                    repository: String::new(),
                    workflow: String::new(),
                    run_id: String::new(),
                    event: String::new(),
                    privilege: ProducerPrivilege::Untrusted,
                },
                started_at_epoch_seconds: 0,
                completed_at_epoch_seconds: 0,
            },
        };
        let fresh = TestExecutionObservation {
            runner: aplan_evidence::TestCatalogueRunner {
                index: 1,
                label: "unit".into(),
                state: aplan_evidence::TestCatalogueRunnerState::Listed,
                names: vec!["alpha".into(), "beta".into()],
            },
            requested: vec!["beta".into()],
            outcomes: Vec::new(),
            complete: true,
        };
        let error = merge_observations(&old, fresh).expect_err("missing outcome");
        assert!(error.contains("beta"), "{error}");
    }

    #[test]
    fn exhaustive_dependencies_select_only_affected_and_new_tests() {
        let declarations = vec![
            dependency(&["unit::*"], &["src/**", "fixtures/unit/**"]),
            dependency(&["web::*"], &["web/**"]),
        ];
        let changed = vec![ChangedPath {
            path: "src/lib.rs".into(),
            introduced: false,
        }];
        let selected = select_tests(
            &declarations,
            &["src/**".into(), "web/**".into(), "fixtures/**".into()],
            &changed,
            &["unit::a".into(), "web::a".into(), "web::new".into()],
            &["unit::a".into(), "web::a".into()],
            &[outcome("unit::a"), outcome("web::a")],
        )
        .expect("selection");
        assert!(!selected.widened);
        assert_eq!(selected.names, vec!["unit::a", "web::new"]);
    }

    #[test]
    fn missing_overlap_material_and_new_path_cases_widen() {
        let current = vec!["unit::a".into(), "web::a".into()];
        let old = current.clone();
        let outcomes = vec![outcome("unit::a"), outcome("web::a")];
        let cases = [
            (
                vec![dependency(&["unit::*"], &["src/**"])],
                vec![ChangedPath {
                    path: "src/lib.rs".into(),
                    introduced: false,
                }],
            ),
            (
                vec![
                    dependency(&["*::a"], &["src/**"]),
                    dependency(&["unit::*"], &["src/**"]),
                ],
                vec![ChangedPath {
                    path: "src/lib.rs".into(),
                    introduced: false,
                }],
            ),
            (
                vec![
                    dependency(&["unit::*"], &["src/unit/**"]),
                    dependency(&["web::*"], &["web/**"]),
                ],
                vec![ChangedPath {
                    path: "src/shared.rs".into(),
                    introduced: false,
                }],
            ),
            (
                vec![
                    dependency(&["unit::*"], &["src/**"]),
                    dependency(&["web::*"], &["web/**"]),
                ],
                vec![ChangedPath {
                    path: "new-component/file.rs".into(),
                    introduced: true,
                }],
            ),
            (
                vec![
                    dependency(&["unit::*"], &["shared/**"]),
                    dependency(&["web::*"], &["shared/**"]),
                ],
                vec![ChangedPath {
                    path: "shared/fixture.json".into(),
                    introduced: false,
                }],
            ),
            (
                vec![
                    dependency(&["unit::*"], &["external/**"]),
                    dependency(&["web::*"], &["web/**"]),
                ],
                vec![ChangedPath {
                    path: "external/fixture.json".into(),
                    introduced: false,
                }],
            ),
        ];
        for (declarations, changed) in cases {
            let selection = select_tests(
                &declarations,
                &["src/**".into(), "web/**".into()],
                &changed,
                &current,
                &old,
                &outcomes,
            )
            .expect("selection");
            assert!(selection.widened, "{declarations:?} {changed:?}");
            assert_eq!(selection.names, current);
        }
    }

    #[test]
    fn unrelated_existing_plan_prose_does_not_invalidate_provider_outcomes() {
        let selection = select_tests(
            &[
                dependency(&["unit::*"], &["src/**"]),
                dependency(&["web::*"], &["web/**"]),
            ],
            &["src/**".into(), "web/**".into()],
            &[ChangedPath {
                path: "aplan/tasks/T-1.md".into(),
                introduced: false,
            }],
            &["unit::a".into(), "web::a".into()],
            &["unit::a".into(), "web::a".into()],
            &[outcome("unit::a"), outcome("web::a")],
        )
        .expect("selection");
        assert!(!selection.widened);
        assert!(selection.names.is_empty());
    }

    #[test]
    fn plan_readers_must_declare_plan_inputs_while_other_tests_remain_reusable() {
        let selection = select_tests(
            &[
                dependency(&["unit::*"], &["src/**"]),
                dependency(&["plan_reader::*"], &["aplan/**"]),
            ],
            &["src/**".into(), "aplan/**".into()],
            &[ChangedPath {
                path: "aplan/tasks/T-1.md".into(),
                introduced: false,
            }],
            &["plan_reader::policy".into(), "unit::a".into()],
            &["plan_reader::policy".into(), "unit::a".into()],
            &[outcome("plan_reader::policy"), outcome("unit::a")],
        )
        .expect("selection");
        assert!(!selection.widened);
        assert_eq!(selection.names, ["plan_reader::policy"]);
    }

    #[test]
    fn failed_post_publication_verification_restores_only_its_own_manifest() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let manifest = directory.path().join("receipts.json");
        std::fs::write(&manifest, b"new\n").expect("published manifest");
        rollback_manifest(&manifest, Some(b"new\n"), b"old\n").expect("rollback");
        assert_eq!(std::fs::read(&manifest).expect("restored"), b"old\n");

        std::fs::write(&manifest, b"concurrent\n").expect("concurrent manifest");
        let error = rollback_manifest(&manifest, Some(b"new\n"), b"old\n")
            .expect_err("concurrent writer is not overwritten");
        assert!(error.to_string().contains("concurrent writer"), "{error}");
        assert_eq!(
            std::fs::read(&manifest).expect("concurrent survives"),
            b"concurrent\n"
        );
    }

    #[test]
    fn in_repository_refresh_outputs_must_be_ignored_and_outside_material() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(directory.path())
                .status()
                .expect("git runs")
                .success()
        );
        std::fs::write(directory.path().join(".gitignore"), "artifacts/\n")
            .expect("ignore artifacts");
        std::fs::create_dir(directory.path().join("artifacts")).expect("artifacts");
        let ignored = directory.path().join("artifacts/receipts.json");
        std::fs::write(&ignored, "old\n").expect("ignored manifest");
        refuse_material_output(directory.path(), &ignored, &["src/**".into()])
            .expect("ignored non-material output");

        let visible = directory.path().join("receipts.json");
        std::fs::write(&visible, "old\n").expect("visible manifest");
        let error = refuse_material_output(directory.path(), &visible, &["src/**".into()])
            .expect_err("visible output joins the next dirty boundary");
        assert!(error.to_string().contains("not ignored by Git"), "{error}");

        let error = refuse_material_output(directory.path(), &ignored, &["artifacts/**".into()])
            .expect_err("an ignored output still cannot be provider material");
        assert!(error.to_string().contains("observation_inputs"), "{error}");
    }
}
