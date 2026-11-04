//! Previewable, guarded plan mutations.
//!
//! A [`ChangeSet`] is the application boundary between deciding what a
//! mutation means and publishing it. It retains the exact identified input,
//! file bytes, semantic effects, findings and evidence work used to make that
//! decision. Delivery adapters can therefore render or inspect a dry run
//! without parsing prose, then commit that same value rather than recomputing a
//! subtly different operation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use aplan_core::mutation::Delta;
use sha2::Digest;

use super::{InputIdentity, PlanContext, absolute};

/// Runtime material captured before an explicit focused test execution.
///
/// The configured definitions are retained only so publication can remeasure
/// executable, interpreter, environment and toolchain bytes. The expected
/// identities are digests and disclose none of the inherited environment.
#[derive(Debug)]
pub(crate) struct TestExecutionGuard {
    repository_root: PathBuf,
    runners: Vec<aplan_config::TestRunner>,
    expected: Vec<aplan_evidence::TestRuntimeIdentity>,
}

#[derive(Debug)]
pub(crate) struct TestExecutionCandidates {
    repository_root: PathBuf,
    runners: Vec<(
        String,
        aplan_config::TestRunner,
        Result<aplan_evidence::TestRuntimeIdentity, String>,
    )>,
}

impl TestExecutionCandidates {
    pub(crate) fn capture(repository_root: &Path, runners: &aplan_config::TestRunners) -> Self {
        let runners = runners
            .all()
            .iter()
            .enumerate()
            .map(|(index, runner)| {
                let label = runner
                    .name
                    .clone()
                    .or_else(|| runner.dir.clone())
                    .unwrap_or_else(|| format!("runner {}", index + 1));
                let resolver = format!("test runner `{label}`");
                let identity = aplan_evidence::test_runtime_identity(repository_root, runner);
                (resolver, runner.clone(), identity)
            })
            .collect::<Vec<_>>();
        Self {
            repository_root: repository_root.to_path_buf(),
            runners,
        }
    }

    pub(crate) fn select(
        self,
        selected_resolvers: &std::collections::BTreeSet<String>,
    ) -> Result<TestExecutionGuard, String> {
        let mut runners = Vec::new();
        let mut expected = Vec::new();
        for (resolver, runner, identity) in self.runners {
            if selected_resolvers.contains(&resolver) {
                runners.push(runner);
                expected.push(identity?);
            }
        }
        if runners.is_empty() {
            return Err(
                "the executed citations named no configured provider that can be rebound"
                    .to_string(),
            );
        }
        Ok(TestExecutionGuard {
            repository_root: self.repository_root,
            runners,
            expected,
        })
    }
}

impl TestExecutionGuard {
    pub(crate) fn movement(&self) -> Option<String> {
        let actual = self
            .runners
            .iter()
            .map(|runner| aplan_evidence::test_runtime_identity(&self.repository_root, runner))
            .collect::<Result<Vec<_>, _>>();
        match actual {
            Ok(actual) if actual == self.expected => None,
            Ok(_) => Some(
                "configured test-provider runtime material changed after focused L3 execution"
                    .to_string(),
            ),
            Err(error) => Some(format!(
                "configured test-provider runtime material is no longer identifiable: {error}"
            )),
        }
    }
}

/// The effective evidence policy shown beside a completion or repair preview.
///
/// `required: None` and `required: Some([])` are deliberately distinct. The
/// latter is an explicit scoped policy saying that no kind is required, while
/// the former means no minimum-strength policy was configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRequirement {
    pub required: Option<Vec<aplan_core::EvidenceKind>>,
    pub exempt_tags: Vec<String>,
    pub matched_exemptions: Vec<String>,
    pub cut: Option<String>,
    pub milestone: Option<String>,
}

/// Evidence-specific material that a generic mutation form must retain.
///
/// This lives on the `ChangeSet`, rather than only on an operation wrapper,
/// because the reusable form owns the change between preview and confirmation.
/// Dropping these fields while converting to a `ChangeSet` would make an
/// evidence preview look complete while hiding its effective policy or the
/// resolver's reason for refusing it.
#[derive(Debug, Clone, Default)]
pub struct EvidenceReview {
    pub requirement: Option<EvidenceRequirement>,
    pub findings: Vec<aplan_core::Diagnostic>,
    pub notices: Vec<String>,
}

/// Whether a preview is publishable, empty, or refused by policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeDisposition {
    /// The preview contains at least one file change and policy permits it.
    Ready,
    /// The requested state already exists, so publication has no work to do.
    NoOp,
    /// The post-state introduces a finding that the effective policy refuses.
    Denied,
}

/// Exact text at one path before a proposed write and the content to publish.
///
/// A missing `before` state represents creation without inventing sentinel
/// content. `source` represents relocation explicitly rather than overloading
/// either path or a magic `after` value. Removal, when introduced, remains a
/// distinct operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    source: Option<PathBuf>,
    path: PathBuf,
    before: Option<String>,
    after: String,
}

impl FileChange {
    pub(crate) fn replacement(path: PathBuf, before: String, after: String) -> Self {
        Self {
            source: None,
            path,
            before: Some(before),
            after,
        }
    }

    pub(crate) fn creation(path: PathBuf, after: String) -> Self {
        Self {
            source: None,
            path,
            before: None,
            after,
        }
    }

    pub(crate) fn relocation(
        source: PathBuf,
        path: PathBuf,
        before: String,
        after: String,
    ) -> Self {
        Self {
            source: Some(source),
            path,
            before: Some(before),
            after,
        }
    }

    /// The old path when this change relocates a file.
    #[must_use]
    pub fn source_path(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// The affected path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Exact UTF-8 content in the identified input, or `None` if absent.
    #[must_use]
    pub fn before(&self) -> Option<&str> {
        self.before.as_deref()
    }

    /// Exact UTF-8 content the mutation proposes to publish.
    #[must_use]
    pub fn after(&self) -> &str {
        &self.after
    }
}

/// A domain-level effect represented by a [`ChangeSet`].
///
/// The list is deliberately independent of the textual file changes: a TUI or
/// another engine client can explain the operation without reverse-engineering
/// YAML, while the exact bytes remain available for detailed review.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SemanticEffect {
    /// One intake record is created outside the task graph.
    CaptureCreated { id: String },
    /// Mutable content on one open intake record changes.
    CaptureAmended { id: String },
    /// One open intake record is retained as discarded.
    CaptureDiscarded { id: String },
    /// One discarded intake record returns to open.
    CaptureRestored { id: String },
    /// One open intake record freezes its link to a newly created task.
    CapturePromoted { capture: String, task: String },
    /// One task moves between live work states.
    TaskStatusChanged {
        task: String,
        from: aplan_core::Status,
        to: aplan_core::Status,
    },
    /// One task gains a prerequisite edge.
    DependencyAdded { task: String, after: String },
    /// One task loses a prerequisite edge.
    DependencyRemoved { task: String, after: String },
    /// One task gains an advisory ordering preference.
    PreferenceAdded { task: String, after: String },
    /// One task loses an advisory ordering preference.
    PreferenceRemoved { task: String, after: String },
    /// One task gains a classification tag.
    TagAdded { task: String, tag: String },
    /// One task loses a classification tag.
    TagRemoved { task: String, tag: String },
    /// One vocabulary tag is replaced on every task which carried it.
    TagRenamed {
        from: String,
        to: String,
        tasks: Vec<String>,
    },
    /// One task file is created.
    TaskCreated { task: String, title: String },
    /// Existing or newly created cut membership gains tasks.
    CutMembershipAdded { cut: String, tasks: Vec<String> },
    /// Existing cut membership loses tasks.
    CutMembershipRemoved { cut: String, tasks: Vec<String> },
    /// One task moves between reporting phases.
    TaskPhaseChanged { task: String, from: u32, to: u32 },
    /// Review of a superseded outcome refreshed its trace stamps.
    TraceFingerprintsRestamped { task: String, count: usize },
    /// One stable task id and all structured references move together.
    TaskRekeyed { old: String, new: String },
    /// A different task's structured reference follows a rekeyed id.
    TaskReferenceRekeyed {
        task: String,
        old: String,
        new: String,
    },
    /// A repository-local policy reference follows a rekeyed cut id.
    PolicyReferenceRekeyed {
        path: PathBuf,
        old: String,
        new: String,
    },
    /// A lossy importer reported one explicit conversion consequence.
    ImportNotice {
        source: String,
        kind: String,
        message: String,
    },
    /// A live task becomes `done` with the listed citations.
    TaskClosed {
        task: String,
        evidence: Vec<aplan_core::Evidence>,
    },
    /// A closed task gains one citation.
    EvidenceAdded {
        task: String,
        evidence: aplan_core::Evidence,
    },
    /// A closed task loses one citation.
    EvidenceRemoved {
        task: String,
        evidence: aplan_core::Evidence,
    },
    /// One citation on a closed task is replaced exactly.
    EvidenceReplaced {
        task: String,
        from: aplan_core::Evidence,
        to: aplan_core::Evidence,
    },
    /// Deliberate review refreshed trace and mutable-evidence baselines.
    EvidenceAndTraceFingerprintsRestamped {
        task: String,
        trace_changes: usize,
        evidence_changes: usize,
    },
    /// One verifier-produced replacement joins a single guarded repair batch.
    VerificationRepairApplied {
        task: String,
        from: aplan_core::Evidence,
        to: aplan_core::Evidence,
    },
}

/// Evidence resolution associated with a mutation preview.
///
/// Dependency editing needs none, but carrying that fact explicitly prevents a
/// later evidence-bearing mutation from silently omitting unfinished resolver
/// work from an otherwise complete-looking preview.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChangeEvidence {
    /// This mutation has no evidence-resolution step.
    NotRequired,
    /// Evidence-bearing mutation work, including both completed observations
    /// and citations that still need a resolver answer.
    Resolution {
        /// Citations for which no resolver produced a definitive observation.
        unresolved: Vec<aplan_core::Evidence>,
        /// Resolver observations already collected for this mutation.
        observed: Vec<aplan_evidence::CitationObservation>,
    },
    /// The author explicitly elected to record without resolver work.
    ///
    /// This is intentionally not represented as an empty successful
    /// resolution: the trust-bearing escape hatch must remain visible in the
    /// final preview.
    Skipped {
        citations: Vec<aplan_core::Evidence>,
        reason: String,
    },
}

/// One immutable, reviewable plan mutation.
///
/// Fields are exposed through read-only accessors so a delivery adapter cannot
/// render one set of bytes and then alter what the guarded writer receives.
#[derive(Debug)]
pub struct ChangeSet {
    input: InputIdentity,
    files: Vec<FileChange>,
    effects: Vec<SemanticEffect>,
    diagnostics: Delta,
    denied: bool,
    evidence: ChangeEvidence,
    evidence_review: EvidenceReview,
    focused_test_execution: bool,
    test_execution_guard: Option<TestExecutionGuard>,
    inbox_guard: Option<super::capture::InboxGuard>,
    stale_reason: Option<String>,
}

impl ChangeSet {
    pub(crate) fn from_parts(
        input: InputIdentity,
        files: Vec<FileChange>,
        effects: Vec<SemanticEffect>,
        diagnostics: Delta,
        denied: bool,
        evidence: ChangeEvidence,
    ) -> Self {
        Self {
            input,
            files,
            effects,
            diagnostics,
            denied,
            evidence,
            evidence_review: EvidenceReview::default(),
            focused_test_execution: false,
            test_execution_guard: None,
            inbox_guard: None,
            stale_reason: None,
        }
    }

    pub(crate) fn with_evidence_review(mut self, review: EvidenceReview) -> Self {
        self.evidence_review = review;
        self
    }

    pub(crate) const fn with_focused_test_execution(mut self) -> Self {
        self.focused_test_execution = true;
        self
    }

    pub(crate) fn with_test_execution_guard(mut self, guard: TestExecutionGuard) -> Self {
        self.test_execution_guard = Some(guard);
        self
    }

    pub(crate) fn with_inbox_guard(mut self, guard: super::capture::InboxGuard) -> Self {
        self.inbox_guard = Some(guard);
        self
    }

    pub(crate) fn add_capture_promotion(
        mut self,
        file: FileChange,
        capture: String,
        task: String,
        guard: super::capture::InboxGuard,
    ) -> Self {
        self.files.push(file);
        self.effects
            .push(SemanticEffect::CapturePromoted { capture, task });
        self.inbox_guard = Some(guard);
        self
    }

    /// Whether preparing this preview explicitly started configured test code.
    ///
    /// Delivery adapters use this only to keep command effects distinct from
    /// aplan's guarded publication on every unsuccessful terminal outcome.
    #[must_use]
    pub const fn focused_test_execution(&self) -> bool {
        self.focused_test_execution
    }
    /// The complete input identity this preview was computed against.
    #[must_use]
    pub const fn input(&self) -> &InputIdentity {
        &self.input
    }

    /// Every affected file, in deterministic publication order.
    #[must_use]
    pub fn files(&self) -> &[FileChange] {
        &self.files
    }

    /// Domain effects corresponding to the proposed bytes.
    #[must_use]
    pub fn effects(&self) -> &[SemanticEffect] {
        &self.effects
    }

    /// Findings introduced by the proposed post-state, in validator order.
    #[must_use]
    pub const fn diagnostics(&self) -> &Delta {
        &self.diagnostics
    }

    /// Evidence work required or already observed for this preview.
    #[must_use]
    pub const fn evidence(&self) -> &ChangeEvidence {
        &self.evidence
    }

    /// Policy, findings and notices that qualify this evidence mutation.
    #[must_use]
    pub const fn evidence_review(&self) -> &EvidenceReview {
        &self.evidence_review
    }

    /// Why publication classified the reviewed result as stale when provider
    /// material, rather than the ordinary plan/repository/config identity,
    /// moved after an explicit focused execution.
    #[must_use]
    pub fn stale_reason(&self) -> Option<&str> {
        self.stale_reason.as_deref()
    }

    /// Whether this preview can be published under its effective policy.
    #[must_use]
    pub fn disposition(&self) -> ChangeDisposition {
        if self.denied {
            ChangeDisposition::Denied
        } else if self.files.is_empty() {
            ChangeDisposition::NoOp
        } else {
            ChangeDisposition::Ready
        }
    }
}

/// Filesystem state observed after a publication failure.
///
/// The writer prevents every ordinary stale/locked refusal before publication,
/// but a late filesystem failure can still happen between renames in a broad
/// batch. Naming the observed state keeps a partial result from being flattened
/// into the same answer as a clean refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationState {
    /// Every affected path still has its previewed `before` content.
    Unchanged,
    /// Every affected path has its previewed `after` content.
    Complete,
    /// Every path is either `before` or `after`, and both states are present.
    Partial,
    /// At least one path is missing, unreadable, or contains third-party bytes.
    Diverged,
}

/// State of one reviewed file observed after a publication attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationFileState {
    /// The path still has its reviewed `before` state.
    Unchanged,
    /// The path has its reviewed `after` state.
    Complete,
    /// The path is missing, unreadable, or contains bytes from neither state.
    Diverged,
}

/// One affected path and the state observed after publication returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationFile {
    source: Option<PathBuf>,
    path: PathBuf,
    state: PublicationFileState,
}

impl PublicationFile {
    /// The old path when this observation concerns a relocation.
    #[must_use]
    pub fn source_path(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// The replacement or relocation destination.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn state(&self) -> PublicationFileState {
        self.state
    }
}

/// Filesystem-observed result of a reviewed publication.
///
/// A batch is guarded together before publication, but its individual atomic
/// renames are not a filesystem transaction. Retaining every path beside the
/// aggregate classification lets a client give exact recovery guidance after
/// a late failure instead of repeating an all-or-none claim the writer cannot
/// make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationReport {
    state: PublicationState,
    files: Vec<PublicationFile>,
}

impl PublicationReport {
    #[must_use]
    pub const fn state(&self) -> PublicationState {
        self.state
    }

    #[must_use]
    pub fn files(&self) -> &[PublicationFile] {
        &self.files
    }
}

/// Re-observe every path in a reviewed change against its exact before/after
/// bytes. This is also the post-reload reconciliation primitive for an
/// interactive client which was interrupted after requesting publication.
#[must_use]
pub fn observe_publication(change: &ChangeSet) -> PublicationReport {
    publication_report(&change.files)
}

/// Whether an application snapshot itself contains every reviewed after-state.
///
/// This is deliberately distinct from [`observe_publication`], which reads the
/// current filesystem. A shell must supply a newly loaded snapshot after a
/// publisher returns; otherwise an old in-memory plan could be labeled as the
/// reloaded input merely because the working tree now contains the new bytes.
#[must_use]
pub fn snapshot_observes_publication(context: &PlanContext, change: &ChangeSet) -> bool {
    change
        .files
        .iter()
        .all(|file| snapshot_observes_file(context, file))
}

/// The result of attempting to publish one exact [`ChangeSet`].
// Keep public variant payloads identical across target-specific size thresholds.
#[cfg_attr(windows, allow(clippy::large_enum_variant))]
#[derive(Debug)]
pub enum CommitOutcome {
    /// Every proposed file change was published.
    Applied(ChangeSet),
    /// The preview was current but contained no change.
    NoOp(ChangeSet),
    /// The preview was current but its policy denied publication.
    Denied(ChangeSet),
    /// Input changed after preview and before publication; nothing was written.
    Stale {
        change: ChangeSet,
        actual: InputIdentity,
    },
    /// The guarded writer failed, with the resulting filesystem state measured.
    PublicationFailed {
        change: ChangeSet,
        report: PublicationReport,
        error: aplan_core::save::SaveError,
    },
}

impl CommitOutcome {
    /// The exact preview this outcome concerns.
    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        match self {
            Self::Applied(change)
            | Self::NoOp(change)
            | Self::Denied(change)
            | Self::Stale { change, .. }
            | Self::PublicationFailed { change, .. } => change,
        }
    }
}

/// Commit a previously reviewed change if its complete application input is
/// still current.
///
/// The preview is consumed so a caller cannot accidentally apply the same
/// approval twice. Publication uses the core batch writer, including its locks,
/// compare-and-swap checks, staging and atomic renames.
#[must_use]
pub fn commit_change_set(context: &PlanContext, change: ChangeSet) -> CommitOutcome {
    let inbox_guard = change.inbox_guard.clone();
    commit_change_set_with(context, change, move |replacements, relocation| {
        if let Some(relocation) = relocation {
            aplan_core::save::replace_all_relocating(replacements, relocation)
        } else if let Some(guard) = inbox_guard {
            let target = guard.target();
            aplan_core::save::replace_all_guarded(replacements, &target, || {
                guard.is_current_while_locked()
            })
        } else {
            aplan_core::save::replace_all(replacements)
        }
    })
}

fn commit_change_set_with(
    context: &PlanContext,
    mut change: ChangeSet,
    publish: impl FnOnce(
        &[aplan_core::save::Replacement],
        Option<&aplan_core::save::Relocation>,
    ) -> Result<(), aplan_core::save::SaveError>,
) -> CommitOutcome {
    let current = aplan_core::load_plan_identified(&change.input.plan_root);
    let actual = context.identity_for(&current);
    if actual != change.input {
        return CommitOutcome::Stale { change, actual };
    }
    if let Some(reason) = change
        .test_execution_guard
        .as_ref()
        .and_then(TestExecutionGuard::movement)
    {
        change.stale_reason = Some(reason);
        return CommitOutcome::Stale { change, actual };
    }
    if change
        .inbox_guard
        .as_ref()
        .is_some_and(|guard| !guard.is_current())
    {
        change.stale_reason =
            Some("capture inbox membership or content changed after preview".to_string());
        return CommitOutcome::Stale { change, actual };
    }
    match change.disposition() {
        ChangeDisposition::NoOp => return CommitOutcome::NoOp(change),
        ChangeDisposition::Denied => return CommitOutcome::Denied(change),
        ChangeDisposition::Ready => {}
    }

    let replacements = change
        .files
        .iter()
        .filter(|file| file.source.is_none())
        .map(|file| aplan_core::save::Replacement {
            path: file.path.clone(),
            expected: file.before.clone().unwrap_or_default(),
            updated: file.after.clone(),
        })
        .collect::<Vec<_>>();
    let mut relocations = change.files.iter().filter_map(|file| {
        file.source
            .as_ref()
            .map(|source| aplan_core::save::Relocation {
                from: source.clone(),
                to: file.path.clone(),
                expected: file.before.clone().unwrap_or_default(),
                updated: file.after.clone(),
            })
    });
    let relocation = relocations.next();
    if relocations.next().is_some() {
        let error = aplan_core::save::SaveError::Conflict {
            path: change.input.plan_root.clone(),
        };
        return CommitOutcome::PublicationFailed {
            report: publication_report(&change.files),
            change,
            error,
        };
    }
    match publish(&replacements, relocation.as_ref()) {
        Ok(()) => CommitOutcome::Applied(change),
        Err(
            aplan_core::save::SaveError::Changed { .. }
            | aplan_core::save::SaveError::Exists { .. },
        ) => {
            let current = aplan_core::load_plan_identified(&change.input.plan_root);
            let actual = context.identity_for(&current);
            CommitOutcome::Stale { change, actual }
        }
        Err(error) => {
            let report = publication_report(&change.files);
            CommitOutcome::PublicationFailed {
                change,
                report,
                error,
            }
        }
    }
}

/// Which dependency edit to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyOperation {
    Add,
    Remove,
}

impl DependencyOperation {
    const fn adding(self) -> bool {
        matches!(self, Self::Add)
    }
}

/// Add or remove one dependency edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyRequest {
    pub id: String,
    pub after: String,
    pub operation: DependencyOperation,
}

/// Dependency additions and removals judged against one complete post-state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyBatchRequest {
    pub operations: Vec<DependencyRequest>,
}

/// One immutable multi-file dependency rewrite preview.
#[derive(Debug)]
pub struct DependencyBatchPreview {
    request: DependencyBatchRequest,
    change: ChangeSet,
}

impl DependencyBatchPreview {
    #[must_use]
    pub const fn request(&self) -> &DependencyBatchRequest {
        &self.request
    }

    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }
}

/// Compute every requested edge edit before judging the resulting graph.
///
/// This deliberately does not apply operations one at a time. A replacement
/// may therefore remove one side of a path while adding another side that
/// would have formed a cycle against the old graph, provided the final graph
/// is valid. Publication remains snapshot-bound through [`commit_dependency_batch`].
pub fn preview_dependency_batch(
    context: &PlanContext,
    request: &DependencyBatchRequest,
) -> Result<DependencyBatchPreview, Error> {
    validate_dependency_batch(context.plan(), request)?;

    let mut by_target: BTreeMap<&str, (BTreeSet<&str>, BTreeSet<&str>)> = BTreeMap::new();
    for operation in &request.operations {
        let (adds, removes) = by_target.entry(&operation.id).or_default();
        if operation.operation.adding() {
            adds.insert(&operation.after);
        } else {
            removes.insert(&operation.after);
        }
    }

    let mut after_plan = aplan_core::mutation::clone_of(context.plan());
    let mut files = Vec::new();
    let mut effects = Vec::new();
    for (id, (adds, removes)) in by_target {
        let target = context
            .plan()
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == id)
            .expect("the batch target was validated");
        let mut edges = target
            .task
            .blocked_by
            .iter()
            .filter(|edge| !removes.contains(edge.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for after in adds {
            if !edges.iter().any(|edge| edge == after) {
                edges.push(after.to_string());
            }
        }
        if edges == target.task.blocked_by {
            continue;
        }
        let original = source_text(context, &target.path)
            .ok_or_else(|| changed_while_preparing(&target.task.id))?;
        let updated = aplan_core::close::set_blocked_by(&original, &edges).map_err(|source| {
            Error::Rewrite {
                id: target.task.id.clone(),
                source,
            }
        })?;
        let produced = aplan_core::mutation::task_of(&updated)
            .map_err(|error| Error::Plan(format!("`{id}`: {error}")))?;
        after_plan = aplan_core::mutation::with_replaced(&after_plan, id, produced);
        for after in target
            .task
            .blocked_by
            .iter()
            .filter(|edge| !edges.contains(edge))
        {
            effects.push(SemanticEffect::DependencyRemoved {
                task: id.to_string(),
                after: after.clone(),
            });
        }
        for after in edges
            .iter()
            .filter(|edge| !target.task.blocked_by.contains(edge))
        {
            effects.push(SemanticEffect::DependencyAdded {
                task: id.to_string(),
                after: after.clone(),
            });
        }
        files.push(FileChange::replacement(
            target.path.clone(),
            original,
            updated,
        ));
    }
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after_plan, context.options());
    let denied = diagnostics.refuses(context.verdict());
    Ok(DependencyBatchPreview {
        request: request.clone(),
        change: ChangeSet {
            input: context.identity().clone(),
            files,
            effects,
            diagnostics,
            denied,
            evidence: ChangeEvidence::NotRequired,
            evidence_review: EvidenceReview::default(),
            focused_test_execution: false,
            test_execution_guard: None,
            inbox_guard: None,
            stale_reason: None,
        },
    })
}

#[must_use]
pub fn commit_dependency_batch(
    context: &PlanContext,
    preview: DependencyBatchPreview,
) -> CommitOutcome {
    commit_change_set(context, preview.change)
}

fn validate_dependency_batch(
    plan: &aplan_core::Plan,
    request: &DependencyBatchRequest,
) -> Result<(), Error> {
    if request.operations.is_empty() {
        return Err(Error::Arguments(
            "a dependency batch needs at least one operation".to_string(),
        ));
    }
    if !plan.load_diagnostics.is_empty() {
        return Err(Error::Plan(
            "the plan does not load cleanly; fix it before changing dependencies".to_string(),
        ));
    }
    let mut seen = BTreeMap::new();
    for operation in &request.operations {
        if operation.id == operation.after {
            return Err(Error::Arguments(format!(
                "`{}` cannot come after itself",
                operation.id
            )));
        }
        for id in [&operation.id, &operation.after] {
            if let Some(why) = aplan_core::mutation::ambiguous_id(plan, id) {
                return Err(Error::Refused(why));
            }
            if !plan.tasks.iter().any(|loaded| loaded.task.id == *id) {
                return Err(Error::Arguments(format!("no task `{id}` in this plan")));
            }
        }
        let key = (operation.id.as_str(), operation.after.as_str());
        if let Some(previous) = seen.insert(key, operation.operation) {
            let kind = if previous == operation.operation {
                "duplicate"
            } else {
                "conflicting"
            };
            return Err(Error::Arguments(format!(
                "{kind} dependency operations for `{}` after `{}`",
                operation.id, operation.after
            )));
        }
        let target = plan
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == operation.id)
            .expect("the batch target was just validated");
        if target.task.status.is_terminal() {
            return Err(Error::Refused(format!(
                "cannot change dependencies of settled task `{}`; revive it first",
                operation.id
            )));
        }
        let blocker = plan
            .tasks
            .iter()
            .find(|loaded| loaded.task.id == operation.after)
            .expect("the batch blocker was just validated");
        if operation.operation.adding() && blocker.task.status == aplan_core::Status::Dropped {
            return Err(Error::Refused(format!(
                "cannot make `{}` wait on cancelled task `{}`",
                operation.id, operation.after
            )));
        }
        if let Some(why) = aplan_core::mutation::membership_edit(
            plan,
            &operation.id,
            &operation.after,
            operation.operation.adding(),
        ) {
            return Err(Error::Refused(why));
        }
    }
    Ok(())
}

/// A dependency request and its immutable generic change preview.
#[derive(Debug)]
pub struct DependencyPreview {
    request: DependencyRequest,
    target: PathBuf,
    change: ChangeSet,
}

impl DependencyPreview {
    /// The typed request represented by this preview.
    #[must_use]
    pub const fn request(&self) -> &DependencyRequest {
        &self.request
    }

    /// The task file the request addresses, including an idempotent no-op.
    #[must_use]
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// The generic preview consumed by every mutation client.
    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    /// Consume the operation-specific wrapper and return the generic preview.
    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }
}

/// The complete application result of a dependency edit.
#[derive(Debug)]
pub struct DependencyResponse {
    pub input: InputIdentity,
    pub id: String,
    pub after: String,
    pub operation: DependencyOperation,
    /// `false` for an idempotent no-op and for a denied proposed edit.
    pub changed: bool,
    /// The task file when this request names one.
    pub file: Option<PathBuf>,
    /// Findings introduced by the proposed post-state, in validator order.
    pub diagnostics: Delta,
    /// Whether policy refused the proposed post-state. A denied response never
    /// writes, and remains a result rather than transport-specific prose.
    pub denied: bool,
}

/// Failures that are not a rule-engine verdict.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Arguments(String),
    #[error("{0}")]
    Plan(String),
    #[error("{0}")]
    Refused(String),
    #[error("`{id}`: {source}")]
    Rewrite {
        id: String,
        #[source]
        source: aplan_core::CloseError,
    },
    #[error(transparent)]
    Write(#[from] aplan_core::save::SaveError),
}

/// Compute a dependency edit from exactly the bytes and policy in `context`.
///
/// This is the dry-run operation: it performs no write. The returned preview is
/// the value [`commit_dependency`] consumes after a client has reviewed it.
pub fn preview_dependency(
    context: &PlanContext,
    request: &DependencyRequest,
) -> Result<DependencyPreview, Error> {
    validate_request(context.plan(), request, false)?;
    let target = context
        .plan()
        .tasks
        .iter()
        .find(|loaded| loaded.task.id == request.id)
        .expect("the target was validated before previewing");
    let path = target.path.clone();
    let original =
        source_text(context, &path).ok_or_else(|| changed_while_preparing(&request.id))?;
    prepare_dependency(
        context.plan(),
        path,
        original,
        context.identity().clone(),
        context,
        request,
    )
}

/// Publish one reviewed dependency preview through the generic guarded writer.
#[must_use]
pub fn commit_dependency(context: &PlanContext, preview: DependencyPreview) -> CommitOutcome {
    commit_change_set(context, preview.change)
}

/// Apply one dependency edit through preview, policy and guarded publication.
///
/// This compatibility operation refreshes mutable plan bytes before previewing,
/// preserving the CLI and MCP behavior that rebases an invocation on a valid
/// intervening edge. Interactive clients instead retain and explicitly commit
/// [`preview_dependency`]'s snapshot-bound value.
pub fn edit_dependency(
    context: &PlanContext,
    request: &DependencyRequest,
) -> Result<DependencyResponse, Error> {
    validate_request(context.plan(), request, false)?;
    let (current, target_path, original) = current_dependency_target(context, request)?;
    let preview = prepare_dependency(
        &current.plan,
        target_path,
        original,
        context.identity_for(&current),
        context,
        request,
    )?;
    let target = preview.target.clone();
    let request = preview.request.clone();
    match commit_dependency(context, preview) {
        CommitOutcome::Applied(change) => {
            Ok(dependency_response(change, request, target, true, false))
        }
        CommitOutcome::NoOp(change) => {
            Ok(dependency_response(change, request, target, false, false))
        }
        CommitOutcome::Denied(change) => {
            Ok(dependency_response(change, request, target, false, true))
        }
        CommitOutcome::Stale { change, .. } => {
            let path = change
                .files
                .first()
                .map_or(target, |file| file.path.clone());
            Err(Error::Write(aplan_core::save::SaveError::Changed { path }))
        }
        CommitOutcome::PublicationFailed { error, .. } => Err(Error::Write(error)),
    }
}

fn dependency_response(
    change: ChangeSet,
    request: DependencyRequest,
    target: PathBuf,
    changed: bool,
    denied: bool,
) -> DependencyResponse {
    DependencyResponse {
        input: change.input,
        id: request.id,
        after: request.after,
        operation: request.operation,
        changed,
        file: Some(target),
        diagnostics: change.diagnostics,
        denied,
    }
}

fn prepare_dependency(
    plan: &aplan_core::Plan,
    target_path: PathBuf,
    original: String,
    input: InputIdentity,
    context: &PlanContext,
    request: &DependencyRequest,
) -> Result<DependencyPreview, Error> {
    validate_request(plan, request, true)?;
    let target = plan
        .tasks
        .iter()
        .find(|loaded| loaded.task.id == request.id)
        .expect("the target was validated before preparing its change");
    let adding = request.operation.adding();
    if let Some(why) =
        aplan_core::mutation::membership_edit(plan, &request.id, &request.after, adding)
    {
        return Err(Error::Refused(why));
    }

    let mut edges = target.task.blocked_by.clone();
    let present = edges.iter().any(|edge| edge == &request.after);
    if adding == present {
        return Ok(DependencyPreview {
            request: request.clone(),
            target: target_path,
            change: ChangeSet {
                input,
                files: Vec::new(),
                effects: Vec::new(),
                diagnostics: Delta::default(),
                denied: false,
                evidence: ChangeEvidence::NotRequired,
                evidence_review: EvidenceReview::default(),
                focused_test_execution: false,
                test_execution_guard: None,
                inbox_guard: None,
                stale_reason: None,
            },
        });
    }
    if adding {
        edges.push(request.after.clone());
    } else {
        edges.retain(|edge| edge != &request.after);
    }

    let updated =
        aplan_core::close::set_blocked_by(&original, &edges).map_err(|source| Error::Rewrite {
            id: request.id.clone(),
            source,
        })?;
    let produced = aplan_core::mutation::task_of(&updated)
        .map_err(|error| Error::Plan(format!("`{}`: {error}", request.id)))?;
    let after_plan = aplan_core::mutation::with_replaced(plan, &request.id, produced);
    let diagnostics = aplan_core::mutation::delta(plan, &after_plan, context.options());
    let denied = diagnostics.refuses(context.verdict());
    let effect = if adding {
        SemanticEffect::DependencyAdded {
            task: request.id.clone(),
            after: request.after.clone(),
        }
    } else {
        SemanticEffect::DependencyRemoved {
            task: request.id.clone(),
            after: request.after.clone(),
        }
    };
    Ok(DependencyPreview {
        request: request.clone(),
        target: target_path.clone(),
        change: ChangeSet {
            input,
            files: vec![FileChange::replacement(target_path, original, updated)],
            effects: vec![effect],
            diagnostics,
            denied,
            evidence: ChangeEvidence::NotRequired,
            evidence_review: EvidenceReview::default(),
            focused_test_execution: false,
            test_execution_guard: None,
            inbox_guard: None,
            stale_reason: None,
        },
    })
}

fn validate_request(
    plan: &aplan_core::Plan,
    request: &DependencyRequest,
    refreshed: bool,
) -> Result<(), Error> {
    if request.id == request.after {
        return Err(Error::Arguments(format!(
            "`{}` cannot come after itself",
            request.id
        )));
    }
    if !plan.load_diagnostics.is_empty() {
        return Err(Error::Plan(if refreshed {
            "the plan changed and no longer loads cleanly; fix it before changing what comes after what"
                .to_string()
        } else {
            "the plan does not load cleanly; fix it before changing what comes after what"
                .to_string()
        }));
    }
    if let Some(why) = aplan_core::mutation::ambiguous_id(plan, &request.id) {
        return Err(Error::Refused(why));
    }
    if !plan.tasks.iter().any(|loaded| loaded.task.id == request.id) {
        return Err(Error::Arguments(format!(
            "no task `{}` in this plan",
            request.id
        )));
    }
    Ok(())
}

/// Refresh the mutable input and prove its target still matches the path the
/// invocation originally selected.
fn current_dependency_target(
    context: &PlanContext,
    request: &DependencyRequest,
) -> Result<(aplan_core::IdentifiedPlan, PathBuf, String), Error> {
    let target_path = context
        .plan()
        .tasks
        .iter()
        .find(|loaded| loaded.task.id == request.id)
        .map(|loaded| loaded.path.clone())
        .expect("the original target was validated before refreshing");
    let current = aplan_core::load_plan_identified(&context.identity.plan_root);
    validate_request(&current.plan, request, true)?;
    let target = current
        .plan
        .tasks
        .iter()
        .find(|loaded| loaded.task.id == request.id)
        .expect("the refreshed target was validated before reading it");
    if absolute(&target.path) != absolute(&target_path) {
        return Err(changed_while_preparing(&request.id));
    }
    let original = current
        .inputs
        .iter()
        .find(|input| absolute(&input.path) == absolute(&target.path))
        .and_then(|input| match &input.content {
            aplan_core::PlanFileContent::Bytes(bytes) => std::str::from_utf8(bytes).ok(),
            aplan_core::PlanFileContent::Sha256(_)
            | aplan_core::PlanFileContent::ReadError { .. } => None,
        })
        .ok_or_else(|| changed_while_preparing(&request.id))?
        .to_string();
    Ok((current, target_path, original))
}

fn source_text(context: &PlanContext, path: &Path) -> Option<String> {
    context.source_text(path).map(str::to_string)
}

fn changed_while_preparing(id: &str) -> Error {
    Error::Refused(format!(
        "`{id}` changed while the dependency edit was being prepared; reload and try again"
    ))
}

fn publication_report(files: &[FileChange]) -> PublicationReport {
    let files = files
        .iter()
        .map(|file| PublicationFile {
            source: file.source.clone(),
            path: file.path.clone(),
            state: file_state(file),
        })
        .collect::<Vec<_>>();
    let state = if files
        .iter()
        .any(|file| file.state == PublicationFileState::Diverged)
    {
        PublicationState::Diverged
    } else if files
        .iter()
        .all(|file| file.state == PublicationFileState::Unchanged)
    {
        PublicationState::Unchanged
    } else if files
        .iter()
        .all(|file| file.state == PublicationFileState::Complete)
    {
        PublicationState::Complete
    } else {
        PublicationState::Partial
    };
    PublicationReport { state, files }
}

#[cfg(test)]
fn publication_state(files: &[FileChange]) -> PublicationState {
    publication_report(files).state
}

fn file_state(file: &FileChange) -> PublicationFileState {
    let destination = observed_text(&file.path);
    let Some(source) = &file.source else {
        return if destination.matches(file.before.as_deref()) {
            PublicationFileState::Unchanged
        } else if destination.matches(Some(&file.after)) {
            PublicationFileState::Complete
        } else {
            PublicationFileState::Diverged
        };
    };
    if absolute(source) == absolute(&file.path) {
        return if destination.matches(file.before.as_deref()) {
            PublicationFileState::Unchanged
        } else if destination.matches(Some(&file.after)) {
            PublicationFileState::Complete
        } else {
            PublicationFileState::Diverged
        };
    }
    let source = observed_text(source);
    if source.matches(file.before.as_deref()) && destination.matches(None) {
        PublicationFileState::Unchanged
    } else if source.matches(None) && destination.matches(Some(&file.after)) {
        PublicationFileState::Complete
    } else {
        PublicationFileState::Diverged
    }
}

enum ObservedText {
    Missing,
    Present(String),
    Unreadable,
}

impl ObservedText {
    fn matches(&self, expected: Option<&str>) -> bool {
        match (self, expected) {
            (Self::Missing, None) => true,
            (Self::Present(actual), Some(expected)) => actual == expected,
            _ => false,
        }
    }
}

fn observed_text(path: &Path) -> ObservedText {
    match std::fs::read_to_string(path) {
        Ok(content) => ObservedText::Present(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ObservedText::Missing,
        Err(_) => ObservedText::Unreadable,
    }
}

pub(crate) fn snapshot_observes_file(context: &PlanContext, file: &FileChange) -> bool {
    if !snapshot_observes_text(context, &file.path, Some(&file.after)) {
        return false;
    }
    file.source.as_ref().is_none_or(|source| {
        absolute(source) == absolute(&file.path) || snapshot_observes_text(context, source, None)
    })
}

fn snapshot_observes_text(context: &PlanContext, path: &Path, expected: Option<&str>) -> bool {
    let path = absolute(path);
    if context
        .identity
        .configuration
        .path
        .as_deref()
        .map(absolute)
        .as_ref()
        == Some(&path)
    {
        return match (&context.identity.configuration.content, expected) {
            (super::ConfigurationContentIdentity::Observed { sha256 }, Some(expected)) => {
                *sha256 == super::to_hex(&sha2::Sha256::digest(expected.as_bytes()))
            }
            _ => false,
        };
    }
    let plan_root = absolute(&context.identity.plan_root);
    if path.starts_with(&plan_root) {
        let captured = context
            .source_inputs
            .iter()
            .find(|(candidate, _)| absolute(candidate) == path)
            .map(|(_, content)| content);
        return match (captured, expected) {
            (Some(aplan_core::load::PlanFileContent::Bytes(actual)), Some(expected)) => {
                actual == expected.as_bytes()
            }
            (None, None) => true,
            _ => false,
        };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn context(root: &Path) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(root),
            super::super::ContextInputs {
                repository_root: root.to_path_buf(),
                configuration_file: None,
                trust: super::super::Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
    }

    fn change_set(context: &PlanContext, files: Vec<FileChange>) -> ChangeSet {
        ChangeSet {
            input: context.identity().clone(),
            files,
            effects: Vec::new(),
            diagnostics: Delta::default(),
            denied: false,
            evidence: ChangeEvidence::NotRequired,
            evidence_review: EvidenceReview::default(),
            focused_test_execution: false,
            test_execution_guard: None,
            inbox_guard: None,
            stale_reason: None,
        }
    }

    #[test]
    fn publication_state_distinguishes_unchanged_partial_complete_and_diverged() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let first = dir.path().join("first.md");
        let second = dir.path().join("second.md");
        std::fs::write(&first, "before first").expect("first");
        std::fs::write(&second, "before second").expect("second");
        let files = vec![
            FileChange::replacement(first.clone(), "before first".into(), "after first".into()),
            FileChange::replacement(
                second.clone(),
                "before second".into(),
                "after second".into(),
            ),
        ];

        assert_eq!(publication_state(&files), PublicationState::Unchanged);
        std::fs::write(&first, "after first").expect("publish first");
        assert_eq!(publication_state(&files), PublicationState::Partial);
        std::fs::write(&second, "after second").expect("publish second");
        assert_eq!(publication_state(&files), PublicationState::Complete);
        std::fs::write(&first, "somebody else's bytes").expect("diverge");
        assert_eq!(publication_state(&files), PublicationState::Diverged);

        let created = dir.path().join("created.md");
        let creation = vec![FileChange::creation(created.clone(), "created".to_string())];
        assert_eq!(publication_state(&creation), PublicationState::Unchanged);
        std::fs::write(&created, "created").expect("create");
        assert_eq!(publication_state(&creation), PublicationState::Complete);

        let unreadable = dir.path().join("directory");
        std::fs::create_dir(&unreadable).expect("directory");
        let unreadable = vec![FileChange::replacement(
            unreadable,
            "before".to_string(),
            "after".to_string(),
        )];
        assert_eq!(publication_state(&unreadable), PublicationState::Diverged);

        let moved_from = dir.path().join("old.md");
        let moved_to = dir.path().join("new.md");
        std::fs::write(&moved_from, "old bytes").expect("relocation source");
        let moved = vec![FileChange::relocation(
            moved_from.clone(),
            moved_to.clone(),
            "old bytes".to_string(),
            "new bytes".to_string(),
        )];
        assert_eq!(publication_state(&moved), PublicationState::Unchanged);
        std::fs::write(&moved_to, "new bytes").expect("relocation destination");
        assert_eq!(publication_state(&moved), PublicationState::Diverged);
        std::fs::remove_file(&moved_from).expect("remove relocation source");
        assert_eq!(publication_state(&moved), PublicationState::Complete);
    }

    #[test]
    fn multiple_relocations_are_refused_and_in_place_relocations_are_measured() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        let first = root.join("tasks/A-1.md");
        let second = root.join("tasks/A-2.md");
        let first_before = "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        let second_before = "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\n---\n";
        std::fs::write(&first, first_before).expect("first");
        std::fs::write(&second, second_before).expect("second");
        let context = context(&root);
        let change = ChangeSet {
            input: context.identity().clone(),
            files: vec![
                FileChange::relocation(
                    first.clone(),
                    root.join("tasks/A-9.md"),
                    first_before.to_string(),
                    first_before.replace("A-1", "A-9"),
                ),
                FileChange::relocation(
                    second.clone(),
                    root.join("tasks/A-8.md"),
                    second_before.to_string(),
                    second_before.replace("A-2", "A-8"),
                ),
            ],
            effects: Vec::new(),
            diagnostics: Delta::default(),
            denied: false,
            evidence: ChangeEvidence::NotRequired,
            evidence_review: EvidenceReview::default(),
            focused_test_execution: false,
            test_execution_guard: None,
            inbox_guard: None,
            stale_reason: None,
        };
        let outcome = commit_change_set_with(&context, change, |_, _| {
            panic!("an unsupported relocation batch reached the publisher")
        });
        assert!(matches!(
            outcome,
            CommitOutcome::PublicationFailed {
                report: PublicationReport {
                    state: PublicationState::Unchanged,
                    ..
                },
                error: aplan_core::save::SaveError::Conflict { .. },
                ..
            }
        ));

        let in_place = vec![FileChange::relocation(
            first.clone(),
            first.clone(),
            first_before.to_string(),
            format!("{first_before}\nchanged\n"),
        )];
        assert_eq!(publication_state(&in_place), PublicationState::Unchanged);
        std::fs::write(&first, in_place[0].after()).expect("complete in-place relocation");
        assert_eq!(publication_state(&in_place), PublicationState::Complete);
        std::fs::write(&first, "different").expect("diverged in-place relocation");
        assert_eq!(publication_state(&in_place), PublicationState::Diverged);
    }

    #[test]
    fn a_late_batch_failure_returns_a_typed_partial_publication() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        let first = root.join("tasks/A-1.md");
        let second = root.join("tasks/A-2.md");
        let first_before = "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        let second_before = "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\n---\n";
        let first_after = format!("{first_before}\nfirst changed\n");
        let second_after = format!("{second_before}\nsecond changed\n");
        std::fs::write(&first, first_before).expect("first");
        std::fs::write(&second, second_before).expect("second");
        let context = context(&root);
        let change = ChangeSet {
            input: context.identity().clone(),
            files: vec![
                FileChange::replacement(
                    first.clone(),
                    first_before.to_string(),
                    first_after.clone(),
                ),
                FileChange::replacement(second.clone(), second_before.to_string(), second_after),
            ],
            effects: Vec::new(),
            diagnostics: Delta::default(),
            denied: false,
            evidence: ChangeEvidence::NotRequired,
            evidence_review: EvidenceReview::default(),
            focused_test_execution: false,
            test_execution_guard: None,
            inbox_guard: None,
            stale_reason: None,
        };

        let outcome = commit_change_set_with(&context, change, |replacements, relocation| {
            assert!(relocation.is_none());
            std::fs::write(&replacements[0].path, &replacements[0].updated)
                .expect("publish first member");
            Err(aplan_core::save::SaveError::Io {
                path: replacements[1].path.clone(),
                source: std::io::Error::other("injected late failure"),
            })
        });
        assert_eq!(outcome.change_set().files().len(), 2);
        let CommitOutcome::PublicationFailed { report, error, .. } = outcome else {
            panic!("late failure was flattened into another outcome: {outcome:?}");
        };
        assert_eq!(report.state(), PublicationState::Partial);
        assert_eq!(report.files().len(), 2);
        assert_eq!(report.files()[0].path(), first);
        assert_eq!(report.files()[0].state(), PublicationFileState::Complete);
        assert_eq!(report.files()[1].path(), second);
        assert_eq!(report.files()[1].state(), PublicationFileState::Unchanged);
        assert!(matches!(error, aplan_core::save::SaveError::Io { .. }));
        assert_eq!(
            std::fs::read_to_string(first).expect("first after failure"),
            first_after
        );
        assert_eq!(
            std::fs::read_to_string(second).expect("second after failure"),
            second_before
        );
    }

    #[test]
    fn a_late_relocation_failure_measures_the_source_destination_and_siblings() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan/tasks");
        std::fs::create_dir_all(&root).expect("tasks");
        let source = root.join("A-1.md");
        let destination = root.join("A-9.md");
        let sibling = root.join("A-2.md");
        let before = "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        let after = "---\nid: A-9\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        let sibling_before = "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\n---\n";
        std::fs::write(&source, before).expect("source");
        std::fs::write(&sibling, sibling_before).expect("sibling");
        let context = context(&dir.path().join("aplan"));
        let change = ChangeSet {
            input: context.identity().clone(),
            files: vec![
                FileChange::relocation(
                    source.clone(),
                    destination.clone(),
                    before.to_string(),
                    after.to_string(),
                ),
                FileChange::replacement(
                    sibling.clone(),
                    sibling_before.to_string(),
                    format!("{sibling_before}\nplanned sibling\n"),
                ),
            ],
            effects: Vec::new(),
            diagnostics: Delta::default(),
            denied: false,
            evidence: ChangeEvidence::NotRequired,
            evidence_review: EvidenceReview::default(),
            focused_test_execution: false,
            test_execution_guard: None,
            inbox_guard: None,
            stale_reason: None,
        };
        let outcome = commit_change_set_with(&context, change, |replacements, relocation| {
            assert_eq!(replacements.len(), 1);
            let relocation = relocation.expect("relocation");
            std::fs::write(&relocation.to, &relocation.updated).expect("publish destination");
            std::fs::remove_file(&relocation.from).expect("remove source");
            Err(aplan_core::save::SaveError::Io {
                path: replacements[0].path.clone(),
                source: std::io::Error::other("injected after relocation"),
            })
        });
        let CommitOutcome::PublicationFailed { report, .. } = outcome else {
            panic!("late relocation failure was flattened: {outcome:?}");
        };
        assert_eq!(report.state(), PublicationState::Partial);
        assert_eq!(report.files().len(), 2);
        assert_eq!(report.files()[0].source_path(), Some(source.as_path()));
        assert_eq!(report.files()[0].path(), destination);
        assert_eq!(report.files()[0].state(), PublicationFileState::Complete);
        assert_eq!(report.files()[1].state(), PublicationFileState::Unchanged);
        assert!(!source.exists());
        assert_eq!(
            std::fs::read_to_string(destination).expect("destination"),
            after
        );
        assert_eq!(
            std::fs::read_to_string(sibling).expect("sibling unchanged"),
            sibling_before
        );
    }

    #[test]
    fn lock_and_staging_refusals_are_measured_before_publication() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan/tasks");
        std::fs::create_dir_all(&root).expect("tasks");
        let path = root.join("A-1.md");
        let before = "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        std::fs::write(&path, before).expect("task");
        let context = context(&dir.path().join("aplan"));

        for staging in [false, true] {
            let change = ChangeSet {
                input: context.identity().clone(),
                files: vec![FileChange::replacement(
                    path.clone(),
                    before.to_string(),
                    format!("{before}\nplanned\n"),
                )],
                effects: Vec::new(),
                diagnostics: Delta::default(),
                denied: false,
                evidence: ChangeEvidence::NotRequired,
                evidence_review: EvidenceReview::default(),
                focused_test_execution: false,
                test_execution_guard: None,
                inbox_guard: None,
                stale_reason: None,
            };
            let outcome = commit_change_set_with(&context, change, |replacements, relocation| {
                assert!(relocation.is_none());
                assert_eq!(
                    std::fs::read_to_string(&path).expect("before refusal"),
                    before
                );
                if staging {
                    Err(aplan_core::save::SaveError::Io {
                        path: replacements[0].path.clone(),
                        source: std::io::Error::other("injected staging refusal"),
                    })
                } else {
                    Err(aplan_core::save::SaveError::Busy {
                        path: replacements[0].path.clone(),
                    })
                }
            });
            let CommitOutcome::PublicationFailed { report, .. } = outcome else {
                panic!("prepublication refusal was flattened: {outcome:?}");
            };
            assert_eq!(report.state(), PublicationState::Unchanged);
            assert_eq!(
                std::fs::read_to_string(&path).expect("after refusal"),
                before
            );
        }
    }

    #[test]
    fn a_compare_and_swap_race_is_stale_not_a_publication_failure() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        let path = root.join("tasks/A-1.md");
        let before = "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        std::fs::write(&path, before).expect("task");
        let context = context(&root);
        let change = ChangeSet {
            input: context.identity().clone(),
            files: vec![FileChange::replacement(
                path.clone(),
                before.to_string(),
                format!("{before}\npreviewed edit\n"),
            )],
            effects: Vec::new(),
            diagnostics: Delta::default(),
            denied: false,
            evidence: ChangeEvidence::NotRequired,
            evidence_review: EvidenceReview::default(),
            focused_test_execution: false,
            test_execution_guard: None,
            inbox_guard: None,
            stale_reason: None,
        };

        let outcome = commit_change_set_with(&context, change, |replacements, relocation| {
            assert!(relocation.is_none());
            std::fs::write(&path, format!("{before}\nconcurrent edit\n"))
                .expect("concurrent writer");
            Err(aplan_core::save::SaveError::Changed {
                path: replacements[0].path.clone(),
            })
        });
        assert_eq!(outcome.change_set().files().len(), 1);
        let CommitOutcome::Stale { change, actual } = outcome else {
            panic!("compare-and-swap race was not classified as stale: {outcome:?}");
        };
        assert_ne!(change.input(), &actual);
        assert_eq!(
            std::fs::read_to_string(path).expect("raced task"),
            format!("{before}\nconcurrent edit\n")
        );
    }

    #[test]
    fn dependency_batch_contention_and_lock_failure_publish_no_partial_rewrite() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        let tasks = root.join("tasks");
        std::fs::create_dir_all(&tasks).expect("tasks");
        let first = tasks.join("A-1.md");
        let second = tasks.join("A-2.md");
        let first_before =
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\nblocked_by: []\n---\n";
        let second_before =
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\nblocked_by: [A-1]\n---\n";
        std::fs::write(&first, first_before).expect("first");
        std::fs::write(&second, second_before).expect("second");
        let request = DependencyBatchRequest {
            operations: vec![
                DependencyRequest {
                    id: "A-1".to_string(),
                    after: "A-2".to_string(),
                    operation: DependencyOperation::Add,
                },
                DependencyRequest {
                    id: "A-2".to_string(),
                    after: "A-1".to_string(),
                    operation: DependencyOperation::Remove,
                },
            ],
        };

        let stale_context = context(&root);
        let stale_preview = preview_dependency_batch(&stale_context, &request).expect("preview");
        assert_eq!(stale_preview.change_set().files().len(), 2);
        let concurrent = second_before.replace("title: second", "title: concurrent");
        std::fs::write(&second, &concurrent).expect("concurrent writer");
        assert!(matches!(
            commit_dependency_batch(&stale_context, stale_preview),
            CommitOutcome::Stale { .. }
        ));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), first_before);
        assert_eq!(std::fs::read_to_string(&second).unwrap(), concurrent);

        std::fs::write(&second, second_before).expect("restore");
        let locked_context = context(&root);
        let locked_preview =
            preview_dependency_batch(&locked_context, &request).expect("locked preview");
        std::fs::write(tasks.join(".A-2.md.lock"), "").expect("hold lock");
        let CommitOutcome::PublicationFailed { report, .. } =
            commit_dependency_batch(&locked_context, locked_preview)
        else {
            panic!("a held sibling lock did not refuse the whole batch")
        };
        assert_eq!(report.state(), PublicationState::Unchanged);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), first_before);
        assert_eq!(std::fs::read_to_string(&second).unwrap(), second_before);
    }

    fn dependency_preview_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        String,
        DependencyBatchRequest,
    ) {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        let tasks = root.join("tasks");
        std::fs::create_dir_all(&tasks).expect("tasks");
        let first = tasks.join("A-1.md");
        let second = tasks.join("A-2.md");
        let first_source =
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\nblocked_by: []\n---\n";
        let second_source =
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\nblocked_by: []\n---\n";
        std::fs::write(&first, first_source).expect("first");
        std::fs::write(&second, second_source).expect("second");
        let request = DependencyBatchRequest {
            operations: vec![DependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
            }],
        };
        (dir, root, tasks, first, first_source.to_string(), request)
    }

    fn dependency_edge(id: &str, after: &str, operation: DependencyOperation) -> DependencyRequest {
        DependencyRequest {
            id: id.to_string(),
            after: after.to_string(),
            operation,
        }
    }

    #[test]
    fn dependency_batch_preview_covers_typed_views_and_argument_refusals() {
        let (_dir, root, _tasks, _first, _first_source, request) = dependency_preview_fixture();

        let clean = context(&root);
        let preview = preview_dependency_batch(&clean, &request).expect("preview");
        assert_eq!(preview.request(), &request);
        let change = preview.into_change_set();
        assert_eq!(change.files().len(), 1);
        assert_eq!(
            observe_publication(&change).state(),
            PublicationState::Unchanged
        );

        let empty = DependencyBatchRequest {
            operations: Vec::new(),
        };
        let error = preview_dependency_batch(&clean, &empty).expect_err("empty batch");
        assert!(matches!(error, Error::Arguments(_)), "{error}");

        let self_edge = DependencyBatchRequest {
            operations: vec![DependencyRequest {
                id: "A-1".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
            }],
        };
        let error = preview_dependency_batch(&clean, &self_edge).expect_err("self edge");
        assert!(matches!(error, Error::Arguments(_)), "{error}");

        let unknown = DependencyBatchRequest {
            operations: vec![dependency_edge("A-1", "NOPE", DependencyOperation::Add)],
        };
        let error = preview_dependency_batch(&clean, &unknown).expect_err("unknown blocker");
        assert!(matches!(error, Error::Arguments(_)), "{error}");

        for operations in [
            vec![
                dependency_edge("A-1", "A-2", DependencyOperation::Add),
                dependency_edge("A-1", "A-2", DependencyOperation::Add),
            ],
            vec![
                dependency_edge("A-1", "A-2", DependencyOperation::Add),
                dependency_edge("A-1", "A-2", DependencyOperation::Remove),
            ],
        ] {
            let error = preview_dependency_batch(&clean, &DependencyBatchRequest { operations })
                .expect_err("repeated pair");
            assert!(matches!(error, Error::Arguments(_)), "{error}");
        }
    }

    #[test]
    fn dependency_batch_preview_covers_plan_and_rewrite_refusals() {
        let (_dir, root, tasks, first, first_source, request) = dependency_preview_fixture();

        let terminal = tasks.join("A-3.md");
        let dropped = tasks.join("A-4.md");
        let milestone = tasks.join("M-cut.md");
        std::fs::write(
            &terminal,
            "---\nid: A-3\ntitle: terminal\nphase: 1\nstatus: done\n---\n",
        )
        .expect("terminal");
        std::fs::write(
            &dropped,
            "---\nid: A-4\ntitle: dropped\nphase: 1\nstatus: dropped\n---\n",
        )
        .expect("dropped");
        std::fs::write(
            &milestone,
            "---\nid: M-cut\ntitle: cut\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [A-1]\n---\n",
        )
        .expect("milestone");
        let constrained = context(&root);
        for (request, expected) in [
            (
                DependencyBatchRequest {
                    operations: vec![dependency_edge("A-3", "A-2", DependencyOperation::Add)],
                },
                "settled task",
            ),
            (
                DependencyBatchRequest {
                    operations: vec![dependency_edge("A-1", "A-4", DependencyOperation::Add)],
                },
                "cancelled task",
            ),
            (
                DependencyBatchRequest {
                    operations: vec![dependency_edge("M-cut", "A-2", DependencyOperation::Add)],
                },
                "aplan mv",
            ),
        ] {
            let error = preview_dependency_batch(&constrained, &request).expect_err(expected);
            assert!(error.to_string().contains(expected), "{error}");
        }
        for path in [&terminal, &dropped, &milestone] {
            std::fs::remove_file(path).expect("remove constrained task");
        }

        let mut missing_source = context(&root);
        missing_source.source_inputs.clear();
        let error = preview_dependency_batch(&missing_source, &request).expect_err("source bytes");
        assert!(matches!(error, Error::Refused(_)), "{error}");

        let duplicate = tasks.join("duplicate.md");
        std::fs::write(&duplicate, &first_source).expect("duplicate id");
        let ambiguous = context(&root);
        let error = preview_dependency_batch(&ambiguous, &request).expect_err("ambiguous id");
        assert!(matches!(error, Error::Refused(_)), "{error}");
        std::fs::remove_file(&duplicate).expect("remove duplicate");

        let broken = tasks.join("broken.md");
        std::fs::write(&broken, "---\nid: BAD\nstatus: not-a-status\n---\n").expect("broken");
        let unloadable = context(&root);
        let error = preview_dependency_batch(&unloadable, &request).expect_err("unloadable plan");
        assert!(matches!(error, Error::Plan(_)), "{error}");
        std::fs::remove_file(&broken).expect("remove broken");

        std::fs::write(
            &first,
            first_source.replace("blocked_by: []", "blocked_by: [A-2]"),
        )
        .expect("existing edge");
        let no_op = context(&root);
        let preview = preview_dependency_batch(&no_op, &request).expect("no-op preview");
        assert_eq!(preview.change_set().disposition(), ChangeDisposition::NoOp);

        let anchored =
            "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\nblocked_by: &edges []\n---\n";
        std::fs::write(&first, anchored).expect("anchored source");
        let structural = context(&root);
        let error = preview_dependency_batch(&structural, &request).expect_err("structural alias");
        assert!(matches!(error, Error::Rewrite { .. }), "{error}");
    }

    /// Proof for `tui.broad_mutations_are_recoverable`: a mutation snapshot is
    /// not a HEAD alias. Dirty task bytes and a byte-only selected-config edit
    /// both invalidate a preview while HEAD remains unchanged, and neither can
    /// be overwritten by the reviewed mutation.
    #[test]
    fn mutation_snapshot_identity_is_not_only_repository_head() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let repository = dir.path().join("repository");
        let plan_root = repository.join("aplan");
        let tasks = plan_root.join("tasks");
        std::fs::create_dir_all(&tasks).expect("tasks");
        let first = tasks.join("A-1.md");
        let second = tasks.join("A-2.md");
        let first_text = "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        let second_text =
            "---\nid: A-2\ntitle: second\nphase: 1\nstatus: todo\nblocked_by: [A-1]\n---\n";
        std::fs::write(&first, first_text).expect("first");
        std::fs::write(&second, second_text).expect("second");
        let config_path = dir.path().join("selected.yml");
        std::fs::write(&config_path, "deny_warnings: false\n").expect("config");
        git(&repository, &["init", "-q"]);
        git(&repository, &["config", "user.email", "test@example.com"]);
        git(&repository, &["config", "user.name", "Test"]);
        git(&repository, &["add", "."]);
        git(&repository, &["commit", "-qm", "fixture"]);

        let make_context = || {
            let (config, _) = aplan_config::load_from(
                &aplan_config::ConfigSource::File(config_path.clone()),
                &repository,
            )
            .expect("load config");
            PlanContext::new(
                aplan_core::load_plan_identified(&plan_root),
                super::super::ContextInputs {
                    repository_root: repository.clone(),
                    configuration_file: Some(config_path.clone()),
                    trust: super::super::Trust::RepositoryRead,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            )
            .with_captured_report_config(config)
            .expect("capture selected config")
        };
        let request = DependencyRequest {
            id: "A-2".to_string(),
            after: "A-1".to_string(),
            operation: DependencyOperation::Remove,
        };

        let task_context = make_context();
        let task_preview = preview_dependency(&task_context, &request)
            .expect("task preview")
            .into_change_set();
        let expected = task_context.identity().clone();
        std::fs::write(
            &second,
            second_text.replace("title: second", "title: concurrent"),
        )
        .expect("concurrent task edit");
        let CommitOutcome::Stale { actual, .. } = commit_change_set(&task_context, task_preview)
        else {
            panic!("dirty task bytes were not stale")
        };
        assert_eq!(repository_head(&expected), repository_head(&actual));
        assert_ne!(expected.plan_sha256, actual.plan_sha256);
        assert!(
            std::fs::read_to_string(&second)
                .expect("concurrent task")
                .contains("blocked_by: [A-1]"),
            "the reviewed edge removal overwrote concurrent bytes"
        );
        git(&repository, &["checkout", "--", "aplan/tasks/A-2.md"]);
        let checked_out_second = std::fs::read_to_string(&second).expect("checked-out task bytes");

        let config_context = make_context();
        let config_preview = preview_dependency(&config_context, &request)
            .expect("config preview")
            .into_change_set();
        let expected = config_context.identity().clone();
        std::fs::write(&config_path, "# byte-only edit\ndeny_warnings: false\n")
            .expect("config byte edit");
        let CommitOutcome::Stale { actual, .. } =
            commit_change_set(&config_context, config_preview)
        else {
            panic!("dirty selected-config bytes were not stale")
        };
        assert_eq!(repository_head(&expected), repository_head(&actual));
        assert_eq!(
            expected.configuration.sha256, actual.configuration.sha256,
            "the fixture is meant to isolate a byte-only config edit"
        );
        assert_ne!(
            expected.configuration.content, actual.configuration.content,
            "selected configuration bytes outside Git must be identified"
        );
        assert_ne!(expected.sha256, actual.sha256);
        assert_eq!(
            std::fs::read_to_string(&second).expect("unchanged task"),
            checked_out_second,
            "a stale config preview published task bytes"
        );
    }

    /// Proof for `tui.broad_mutations_are_recoverable`: reconciliation reads
    /// the selected config before the plan-root fallback, requires relocation
    /// sources to be absent, and refuses paths no application snapshot owns.
    #[test]
    fn snapshot_reconciliation_observes_exact_plan_relocation_and_config_bytes() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let plan_root = dir.path().join("aplan");
        let tasks = plan_root.join("tasks");
        std::fs::create_dir_all(&tasks).expect("tasks");
        let source = tasks.join("A-1.md");
        let destination = tasks.join("A-9.md");
        let before = "---\nid: A-1\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        let after = "---\nid: A-9\ntitle: first\nphase: 1\nstatus: todo\n---\n";
        std::fs::write(&destination, after).expect("relocated task");
        let before_reload = context(&plan_root);
        let relocated = change_set(
            &before_reload,
            vec![FileChange::relocation(
                source.clone(),
                destination.clone(),
                before.to_string(),
                after.to_string(),
            )],
        );
        let reloaded = context(&plan_root);
        assert!(snapshot_observes_publication(&reloaded, &relocated));

        let in_place = change_set(
            &reloaded,
            vec![FileChange::relocation(
                destination.clone(),
                destination.clone(),
                after.to_string(),
                after.to_string(),
            )],
        );
        assert!(snapshot_observes_publication(&reloaded, &in_place));

        let outside = change_set(
            &reloaded,
            vec![FileChange::creation(
                dir.path().join("outside.md"),
                "outside\n".to_string(),
            )],
        );
        assert!(!snapshot_observes_publication(&reloaded, &outside));

        let config_path = plan_root.join(".aplan.yml");
        let config_after = "deny_warnings: false\n";
        std::fs::write(&config_path, config_after).expect("selected config");
        let config = aplan_config::parse_config(config_after, &config_path).expect("parse config");
        let configured = PlanContext::new(
            aplan_core::load_plan_identified(&plan_root),
            super::super::ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(config_path.clone()),
                trust: super::super::Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_captured_report_config(config)
        .expect("capture config");
        let config_change = change_set(
            &configured,
            vec![FileChange::replacement(
                config_path.clone(),
                "deny_warnings: true\n".to_string(),
                config_after.to_string(),
            )],
        );
        assert!(snapshot_observes_publication(&configured, &config_change));

        let semantic_only = PlanContext::new(
            aplan_core::load_plan_identified(&plan_root),
            super::super::ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: Some(config_path),
                trust: super::super::Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(aplan_config::Config {
            deny_warnings: Some(false),
            ..aplan_config::Config::default()
        });
        assert!(!snapshot_observes_publication(
            &semantic_only,
            &config_change
        ));
    }

    fn repository_head(identity: &super::super::InputIdentity) -> Option<&str> {
        match &identity.repository {
            super::super::RepositoryIdentity::Observed { head, .. } => head.as_deref(),
            super::super::RepositoryIdentity::NotObserved { .. }
            | super::super::RepositoryIdentity::Unavailable { .. } => None,
        }
    }
}
