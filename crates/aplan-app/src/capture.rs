//! Typed quick-capture reads and guarded lifecycle mutations.

use std::path::{Path, PathBuf};

use aplan_core::capture::{
    Capture, CaptureCreate, CaptureList, CapturePolicy, CaptureProfile, CaptureQuery,
    CaptureSchema, CaptureState, Clock, EffectiveCapture, Inbox, LoadedCapture, PromotionDefaults,
    RevisitFilter, SourceResolution,
};
use sha2::{Digest, Sha256};

use crate::authoring::{AddRequest, AuthoringError};
use crate::{ChangeEvidence, ChangeSet, FileChange, InputIdentity, PlanContext, SemanticEffect};

/// A full stable id supplied by a machine caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactCaptureId(String);

impl ExactCaptureId {
    /// Parse only the complete canonical id shape.
    pub fn new(value: impl Into<String>) -> Result<Self, CaptureError> {
        let value = value.into();
        if aplan_core::capture::is_capture_id(&value) {
            Ok(Self(value))
        } else {
            Err(CaptureError::Input(
                "a machine capture id must be the complete canonical `I-<ULID>`".to_string(),
            ))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A canonical leading id substring supplied by a human workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanCapturePrefix(String);

impl HumanCapturePrefix {
    /// Keep prefix parsing distinct from exact machine-id parsing.
    pub fn new(value: impl Into<String>) -> Result<Self, CaptureError> {
        let value = value.into();
        if !value.starts_with("I-")
            || value.len() <= 2
            || value.len() > 28
            || !value[2..]
                .bytes()
                .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte))
        {
            return Err(CaptureError::Input(
                "a human capture prefix must begin `I-` and continue with canonical upper-case Crockford characters"
                    .to_string(),
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Selection is typed so a machine request can never silently gain prefix
/// matching when the human CLI does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSelector {
    Exact(ExactCaptureId),
    HumanPrefix(HumanCapturePrefix),
}

/// A complete focused capture read.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureRead {
    pub input: InputIdentity,
    pub inbox_sha256: String,
    pub complete: bool,
    pub capture: Capture,
}

/// A bounded collection read and the snapshot it was evaluated against.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureCollection {
    pub input: InputIdentity,
    pub inbox_sha256: String,
    pub complete: bool,
    #[serde(flatten)]
    pub list: CaptureList,
}

/// Bounded immutable inbox state for an interactive client.
///
/// Each lifecycle contributes at most the core list bound. The total counts
/// make truncation explicit, while retaining the complete records for the
/// displayed rows lets a focused inspector show provenance without another
/// filesystem read. Search and filters therefore stay reducer-local and
/// responsive; every mutation still reloads and guards the current inbox.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureWorkspace {
    pub input: InputIdentity,
    pub inbox_sha256: String,
    pub evaluated_at: String,
    pub deferred_open: usize,
    pub captures: Vec<Capture>,
    pub total_open: usize,
    pub total_promoted: usize,
    pub total_discarded: usize,
    pub kinds: Vec<String>,
    pub profiles: Vec<(String, CaptureProfile)>,
}

impl CaptureWorkspace {
    #[must_use]
    pub fn total(&self) -> usize {
        self.total_open
            .saturating_add(self.total_promoted)
            .saturating_add(self.total_discarded)
    }

    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.captures.len() < self.total()
    }
}

/// Mutable fields on one open capture. Nested options distinguish an omitted
/// edit from explicitly clearing an optional value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaptureAmendRequest {
    pub selector: Option<CaptureSelector>,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub body: Option<Option<String>>,
    pub labels: Option<Vec<String>>,
    pub revisit_at: Option<Option<String>>,
    pub promotion_defaults: Option<Option<PromotionDefaults>>,
    /// Optional editor/read baseline. A changed record refuses rather than
    /// applying text prepared from stale focused content.
    pub expected_sha256: Option<String>,
}

/// A discard request. The operation clock is injected separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDiscardRequest {
    pub selector: CaptureSelector,
    pub reason: Option<String>,
}

/// Explicit task fields supplied while promoting one retained capture.
/// `None` on a collection means inherit its snapshotted capture default;
/// `Some([])` deliberately clears that default.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturePromotionRequest {
    pub selector: CaptureSelector,
    pub task_id: Option<String>,
    pub phase: Option<u32>,
    pub cut: Option<String>,
    pub effort_days: Option<f64>,
    pub owner: Option<String>,
    pub paths: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub reason: Option<String>,
}

/// One promotion preview carrying both stable identities.
#[derive(Debug)]
pub struct CapturePromotionPreview {
    capture: Capture,
    task_id: String,
    change: ChangeSet,
    idempotent_retry: bool,
    inbox_sha256: String,
}

impl CapturePromotionPreview {
    #[must_use]
    pub const fn capture(&self) -> &Capture {
        &self.capture
    }

    #[must_use]
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    #[must_use]
    pub const fn idempotent_retry(&self) -> bool {
        self.idempotent_retry
    }

    #[must_use]
    pub fn inbox_sha256(&self) -> &str {
        &self.inbox_sha256
    }

    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }
}

/// One immutable capture mutation preview.
#[derive(Debug)]
pub struct CaptureMutationPreview {
    capture: Capture,
    change: ChangeSet,
    idempotent_retry: bool,
    inbox_sha256: String,
}

impl CaptureMutationPreview {
    #[must_use]
    pub const fn capture(&self) -> &Capture {
        &self.capture
    }

    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    #[must_use]
    pub const fn idempotent_retry(&self) -> bool {
        self.idempotent_retry
    }

    #[must_use]
    pub fn inbox_sha256(&self) -> &str {
        &self.inbox_sha256
    }

    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Refused(String),
    #[error("reading capture inbox snapshot: {0}")]
    Snapshot(String),
    #[error("serializing capture `{id}`: {detail}")]
    Serialize { id: String, detail: String },
    #[error("minting capture identity: {0}")]
    Identity(String),
    #[error("capture id `{id}` already exists; mint another id")]
    IdentityCollision { id: String },
    #[error("{0}")]
    Authoring(#[from] AuthoringError),
}

#[derive(Debug)]
struct CaptureSnapshot {
    inbox: Inbox,
    policy: CapturePolicy,
    sha256: String,
}

/// Snapshot-wide publication guard carried by a generic [`ChangeSet`].
#[derive(Debug, Clone)]
pub(crate) struct InboxGuard {
    pub(crate) root: PathBuf,
    pub(crate) sha256: String,
}

impl InboxGuard {
    pub(crate) fn is_current(&self) -> bool {
        capture_snapshot_digest(&self.root).is_ok_and(|actual| actual == self.sha256)
    }

    pub(crate) fn target(&self) -> PathBuf {
        self.root.join("capture-inbox-snapshot")
    }

    pub(crate) fn is_current_while_locked(&self) -> bool {
        capture_snapshot_digest(&self.root).is_ok_and(|actual| actual == self.sha256)
    }
}

/// Read a bounded inbox projection from one identified repository snapshot.
pub fn list_captures(
    context: &PlanContext,
    query: &CaptureQuery,
    clock: &impl Clock,
) -> Result<CaptureCollection, CaptureError> {
    let snapshot = capture_snapshot(context, clock)?;
    if let Some(kind) = &query.kind
        && !snapshot.policy.kinds().contains(kind)
    {
        return Err(CaptureError::Input(format!(
            "capture kind `{kind}` is not built in or declared by `capture.kinds`"
        )));
    }
    let list = snapshot
        .inbox
        .list(query, clock)
        .map_err(CaptureError::Input)?;
    Ok(CaptureCollection {
        input: context.identity().clone(),
        inbox_sha256: snapshot.sha256,
        complete: true,
        list,
    })
}

/// Load the bounded three-state workspace used by the native inbox.
///
/// The snapshot and operation clock are shared across all state projections,
/// so rows cannot disagree about deferred eligibility or repository identity.
pub fn capture_workspace(
    context: &PlanContext,
    clock: &impl Clock,
) -> Result<CaptureWorkspace, CaptureError> {
    let operation_clock = aplan_core::capture::FixedClock(clock.now_epoch());
    let snapshot = capture_snapshot(context, &operation_clock)?;
    let evaluated_at = aplan_core::capture::format_capture_instant(operation_clock.now_epoch());
    let mut captures = Vec::new();
    let mut deferred_open = 0;
    for state in [
        CaptureState::Open,
        CaptureState::Promoted,
        CaptureState::Discarded,
    ] {
        let list = snapshot
            .inbox
            .list(
                &CaptureQuery {
                    state: Some(state),
                    revisit: RevisitFilter::All,
                    limit: aplan_core::capture::MAX_LIST_LIMIT,
                    ..CaptureQuery::default()
                },
                &operation_clock,
            )
            .map_err(CaptureError::Input)?;
        if state == CaptureState::Open {
            deferred_open = list.deferred;
        }
        for summary in list.captures {
            let capture = snapshot
                .inbox
                .show(&summary.id)
                .map_err(CaptureError::Refused)?
                .expect("a summary came from this inbox snapshot")
                .clone();
            captures.push(capture);
        }
    }
    let count = |state| {
        snapshot
            .inbox
            .captures
            .iter()
            .filter(|loaded| loaded.capture.state == state)
            .count()
    };
    Ok(CaptureWorkspace {
        input: context.identity().clone(),
        inbox_sha256: snapshot.sha256,
        evaluated_at,
        deferred_open,
        captures,
        total_open: count(CaptureState::Open),
        total_promoted: count(CaptureState::Promoted),
        total_discarded: count(CaptureState::Discarded),
        kinds: snapshot.policy.kinds().iter().cloned().collect(),
        profiles: snapshot
            .policy
            .profiles()
            .iter()
            .map(|(name, profile)| (name.clone(), profile.clone()))
            .collect(),
    })
}

/// Whether one freshly loaded capture snapshot contains every reviewed
/// after-state in a capture mutation.
///
/// Capture records are deliberately excluded from the task loader's input
/// set. An interactive client therefore cannot reconcile a capture write with
/// [`crate::snapshot_observes_publication`] alone. Binding this check to the
/// exact inbox digest keeps task/configuration files on their ordinary
/// captured-input path while proving capture bytes came from the same full
/// inbox snapshot as the client's projection.
#[must_use]
pub fn capture_snapshot_observes_publication(
    context: &PlanContext,
    expected_inbox_sha256: &str,
    change: &ChangeSet,
    clock: &impl Clock,
) -> bool {
    let Ok(snapshot) = capture_snapshot(context, clock) else {
        return false;
    };
    if snapshot.sha256 != expected_inbox_sha256 {
        return false;
    }
    let inbox_root = crate::absolute(
        &context
            .identity()
            .plan_root
            .join(aplan_core::capture::INBOX_DIR),
    );
    change.files().iter().all(|file| {
        let path = crate::absolute(file.path());
        if path.parent() != Some(inbox_root.as_path()) {
            return crate::mutation::snapshot_observes_file(context, file);
        }
        if file
            .source_path()
            .is_some_and(|source| crate::absolute(source) != path)
        {
            return false;
        }
        snapshot
            .inbox
            .captures
            .iter()
            .find(|loaded| crate::absolute(&loaded.path) == path)
            .and_then(|loaded| aplan_core::capture::render_capture(&loaded.capture).ok())
            .is_some_and(|rendered| rendered == file.after())
    })
}

/// Read one focused record, including its opaque source key.
pub fn show_capture(
    context: &PlanContext,
    selector: &CaptureSelector,
    clock: &impl Clock,
) -> Result<CaptureRead, CaptureError> {
    let snapshot = capture_snapshot(context, clock)?;
    let loaded = resolve(&snapshot.inbox, selector)?;
    Ok(CaptureRead {
        input: context.identity().clone(),
        inbox_sha256: snapshot.sha256,
        complete: true,
        capture: loaded.capture.clone(),
    })
}

/// Prepare a retry-safe creation with an application-injected clock.
pub fn preview_capture_create(
    context: &PlanContext,
    request: &CaptureCreate,
    clock: &impl Clock,
) -> Result<CaptureMutationPreview, CaptureError> {
    let id = format!(
        "I-{}",
        aplan_interop::ids::mint_ulid()
            .map_err(|error| CaptureError::Identity(error.to_string()))?
    );
    preview_capture_create_with_id(context, request, clock, &id)
}

/// Deterministic creation seam for tests and non-random embedding hosts.
pub fn preview_capture_create_with_id(
    context: &PlanContext,
    request: &CaptureCreate,
    clock: &impl Clock,
    id: &str,
) -> Result<CaptureMutationPreview, CaptureError> {
    if !aplan_core::capture::is_capture_id(id) {
        return Err(CaptureError::Input(
            "a generated capture id is not a canonical `I-<ULID>`".to_string(),
        ));
    }
    let snapshot = capture_snapshot(context, clock)?;
    let effective = request
        .effective(&snapshot.policy)
        .map_err(CaptureError::Input)?;
    match snapshot
        .inbox
        .resolve_source(&effective)
        .map_err(CaptureError::Refused)?
    {
        SourceResolution::Equivalent { id, .. } => {
            let capture = snapshot
                .inbox
                .show(&id)
                .map_err(CaptureError::Refused)?
                .expect("source resolution returned an existing capture")
                .clone();
            return Ok(CaptureMutationPreview {
                capture,
                change: empty_change(context, &snapshot),
                idempotent_retry: true,
                inbox_sha256: snapshot.sha256,
            });
        }
        SourceResolution::Conflict {
            id,
            state,
            namespace,
        } => {
            return Err(CaptureError::Refused(format!(
                "source namespace `{namespace}` already identifies capture `{id}` in state `{}` with different effective content; the opaque key is intentionally not printed",
                state_name(state)
            )));
        }
        SourceResolution::NoSource | SourceResolution::Vacant => {}
    }
    if snapshot
        .inbox
        .show(id)
        .map_err(CaptureError::Refused)?
        .is_some()
    {
        return Err(CaptureError::IdentityCollision { id: id.to_string() });
    }
    let capture = capture_from_effective(id, effective, clock);
    changed_preview(
        context,
        snapshot,
        capture,
        None,
        SemanticEffect::CaptureCreated { id: id.to_string() },
        clock.now_epoch(),
    )
}

/// Prepare an amendment of the fields ADR-0063 leaves mutable while open.
pub fn preview_capture_amend(
    context: &PlanContext,
    request: &CaptureAmendRequest,
    clock: &impl Clock,
) -> Result<CaptureMutationPreview, CaptureError> {
    let selector = request
        .selector
        .as_ref()
        .ok_or_else(|| CaptureError::Input("capture amendment needs an id".to_string()))?;
    let snapshot = capture_snapshot(context, clock)?;
    let loaded = resolve(&snapshot.inbox, selector)?.clone();
    if request
        .expected_sha256
        .as_ref()
        .is_some_and(|expected| expected != &capture_fingerprint(&loaded.capture))
    {
        return Err(CaptureError::Refused(format!(
            "capture `{}` changed while its amendment was being prepared; no capture was written",
            loaded.capture.id
        )));
    }
    if loaded.capture.state != CaptureState::Open {
        return Err(CaptureError::Refused(format!(
            "cannot amend capture `{}` in state `{}`; only open captures are mutable",
            loaded.capture.id,
            state_name(loaded.capture.state)
        )));
    }
    let mut capture = loaded.capture.clone();
    if let Some(title) = &request.title {
        capture.title.clone_from(title);
    }
    if let Some(kind) = &request.kind {
        capture.kind.clone_from(kind);
    }
    if let Some(body) = &request.body {
        capture.body.clone_from(body);
    }
    if let Some(labels) = &request.labels {
        capture.labels.clone_from(labels);
        capture.labels.sort();
        capture.labels.dedup();
    }
    if let Some(revisit_at) = &request.revisit_at {
        capture.revisit_at.clone_from(revisit_at);
    }
    if let Some(defaults) = &request.promotion_defaults {
        capture.promotion_defaults.clone_from(defaults);
    }
    let effective = EffectiveCapture {
        title: capture.title.clone(),
        kind: capture.kind.clone(),
        body: capture.body.clone(),
        labels: capture.labels.clone(),
        revisit_at: capture.revisit_at.clone(),
        profile: capture.profile.clone(),
        promotion_defaults: capture.promotion_defaults.clone(),
        source: capture.source.clone(),
        origin: capture.origin,
    };
    effective.validate().map_err(CaptureError::Input)?;
    let id = capture.id.clone();
    changed_preview(
        context,
        snapshot,
        capture,
        Some(loaded),
        SemanticEffect::CaptureAmended { id },
        clock.now_epoch(),
    )
}

/// Prepare the open-to-discarded transition.
pub fn preview_capture_discard(
    context: &PlanContext,
    request: &CaptureDiscardRequest,
    clock: &impl Clock,
) -> Result<CaptureMutationPreview, CaptureError> {
    let snapshot = capture_snapshot(context, clock)?;
    let loaded = resolve(&snapshot.inbox, &request.selector)?.clone();
    if loaded.capture.state != CaptureState::Open {
        return Err(CaptureError::Refused(format!(
            "cannot discard capture `{}` in state `{}`; only open captures can be discarded",
            loaded.capture.id,
            state_name(loaded.capture.state)
        )));
    }
    let mut capture = loaded.capture.clone();
    capture.state = CaptureState::Discarded;
    capture.disposed_at = Some(aplan_core::capture::format_capture_instant(
        clock.now_epoch(),
    ));
    capture.reason.clone_from(&request.reason);
    let id = capture.id.clone();
    changed_preview(
        context,
        snapshot,
        capture,
        Some(loaded),
        SemanticEffect::CaptureDiscarded { id },
        clock.now_epoch(),
    )
}

/// Prepare the discarded-to-open correction, clearing disposition fields.
pub fn preview_capture_restore(
    context: &PlanContext,
    selector: &CaptureSelector,
    clock: &impl Clock,
) -> Result<CaptureMutationPreview, CaptureError> {
    let snapshot = capture_snapshot(context, clock)?;
    let loaded = resolve(&snapshot.inbox, selector)?.clone();
    if loaded.capture.state != CaptureState::Discarded {
        return Err(CaptureError::Refused(format!(
            "cannot restore capture `{}` in state `{}`; only discarded captures can be restored and promoted captures are irreversible",
            loaded.capture.id,
            state_name(loaded.capture.state)
        )));
    }
    let mut capture = loaded.capture.clone();
    capture.state = CaptureState::Open;
    capture.disposed_at = None;
    capture.reason = None;
    let id = capture.id.clone();
    changed_preview(
        context,
        snapshot,
        capture,
        Some(loaded),
        SemanticEffect::CaptureRestored { id },
        clock.now_epoch(),
    )
}

/// Prepare task creation and capture disposition as one guarded transaction.
pub fn preview_capture_promotion(
    context: &PlanContext,
    request: &CapturePromotionRequest,
    clock: &impl Clock,
) -> Result<CapturePromotionPreview, CaptureError> {
    let snapshot = capture_snapshot(context, clock)?;
    let loaded = resolve(&snapshot.inbox, &request.selector)?.clone();
    let (task_request, fingerprint) = effective_promotion(context, &loaded.capture, request)?;

    if loaded.capture.state == CaptureState::Promoted {
        let existing = loaded
            .capture
            .promoted_to
            .clone()
            .expect("validated promoted capture has a task id");
        if loaded.capture.promotion_fingerprint.as_deref() != Some(&fingerprint) {
            return Err(CaptureError::Refused(format!(
                "capture `{}` was already promoted to `{existing}` with a different request; no task or capture was written",
                loaded.capture.id
            )));
        }
        return Ok(CapturePromotionPreview {
            capture: loaded.capture,
            task_id: existing,
            change: empty_change(context, &snapshot),
            idempotent_retry: true,
            inbox_sha256: snapshot.sha256,
        });
    }
    if loaded.capture.state != CaptureState::Open {
        return Err(CaptureError::Refused(format!(
            "cannot promote capture `{}` in state `{}`; only open captures can be promoted",
            loaded.capture.id,
            state_name(loaded.capture.state)
        )));
    }

    let authoring = crate::authoring::preview_add(context, &task_request)?;
    let task_id = authoring
        .created()
        .first()
        .cloned()
        .expect("single-task add preview creates exactly one task");
    let mut capture = loaded.capture.clone();
    capture.state = CaptureState::Promoted;
    capture.disposed_at = Some(aplan_core::capture::format_capture_instant(
        clock.now_epoch(),
    ));
    capture.reason.clone_from(&request.reason);
    capture.promoted_to = Some(task_id.clone());
    capture.promotion_fingerprint = Some(fingerprint);
    aplan_core::capture::validate_capture_record(
        &capture,
        &snapshot.policy,
        &FixedClockRef(clock.now_epoch()),
    )
    .map_err(|errors| CaptureError::Input(errors.join("; ")))?;
    let original = std::fs::read_to_string(&loaded.path)
        .map_err(|error| CaptureError::Snapshot(error.to_string()))?;
    let file = FileChange::replacement(loaded.path, original, serialized_capture(&capture)?);
    let capture_id = capture.id.clone();
    let inbox_sha256 = snapshot.sha256.clone();
    let change = authoring.into_change_set().add_capture_promotion(
        file,
        capture_id,
        task_id.clone(),
        InboxGuard {
            root: context.identity().plan_root.clone(),
            sha256: snapshot.sha256,
        },
    );
    Ok(CapturePromotionPreview {
        capture,
        task_id,
        change,
        idempotent_retry: false,
        inbox_sha256,
    })
}

#[derive(serde::Serialize)]
struct PromotionFingerprint<'a> {
    schema: &'static str,
    title: &'a str,
    body: &'a Option<String>,
    explicit_task_id: bool,
    task_id: &'a Option<String>,
    phase: Option<u32>,
    cut: &'a Option<String>,
    effort_days: Option<f64>,
    owner: &'a Option<String>,
    paths: &'a [String],
    tags: &'a [String],
    reason: &'a Option<String>,
}

fn effective_promotion(
    context: &PlanContext,
    capture: &Capture,
    request: &CapturePromotionRequest,
) -> Result<(AddRequest, String), CaptureError> {
    let defaults = capture.promotion_defaults.clone().unwrap_or_default();
    let cut = request.cut.clone().or(defaults.cut);
    let cut = cut
        .map(|value| {
            aplan_interop::cut::resolve_id(context.plan(), &value)
                .map_err(|error| CaptureError::Refused(error.to_string()))
        })
        .transpose()?;
    let phase = request.phase.or(defaults.phase);
    let effort_days = request.effort_days.or(defaults.effort_days);
    let owner = request.owner.clone().or(defaults.owner);
    let mut paths = request.paths.clone().unwrap_or(defaults.paths);
    let mut tags = request.tags.clone().unwrap_or(defaults.tags);
    paths.sort();
    paths.dedup();
    tags.sort();
    tags.dedup();
    if tags
        .iter()
        .any(|tag| tag == aplan_core::task::MILESTONE_TAG || tag == aplan_core::task::RELEASE_TAG)
    {
        return Err(CaptureError::Input(
            "capture promotion creates work, not a milestone or release; remove the structural task tag"
                .to_string(),
        ));
    }
    let canonical = PromotionFingerprint {
        schema: "aplan/capture-promotion-request/v1",
        title: &capture.title,
        body: &capture.body,
        explicit_task_id: request.task_id.is_some(),
        task_id: &request.task_id,
        phase,
        cut: &cut,
        effort_days,
        owner: &owner,
        paths: &paths,
        tags: &tags,
        reason: &request.reason,
    };
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| CaptureError::Input(format!("encoding promotion request: {error}")))?;
    let fingerprint = hex(&Sha256::digest(bytes));
    Ok((
        AddRequest {
            title: capture.title.clone(),
            id: request.task_id.clone(),
            phase,
            cut,
            effort_days,
            owner,
            status: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            members: Vec::new(),
            tags,
            paths,
            traces: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            evidence: Vec::new(),
            body: capture.body.clone(),
        },
        fingerprint,
    ))
}

fn changed_preview(
    context: &PlanContext,
    snapshot: CaptureSnapshot,
    capture: Capture,
    before: Option<LoadedCapture>,
    effect: SemanticEffect,
    now: i64,
) -> Result<CaptureMutationPreview, CaptureError> {
    aplan_core::capture::validate_capture_record(&capture, &snapshot.policy, &FixedClockRef(now))
        .map_err(|errors| CaptureError::Input(errors.join("; ")))?;
    let (files, effects) = if let Some(before) = before {
        if before.capture == capture {
            (Vec::new(), Vec::new())
        } else {
            let after = serialized_capture(&capture)?;
            let original = std::fs::read_to_string(&before.path)
                .map_err(|error| CaptureError::Snapshot(error.to_string()))?;
            (
                vec![FileChange::replacement(before.path, original, after)],
                vec![effect],
            )
        }
    } else {
        let after = serialized_capture(&capture)?;
        (
            vec![FileChange::creation(
                context
                    .identity()
                    .plan_root
                    .join(aplan_core::capture::INBOX_DIR)
                    .join(format!("{}.yml", capture.id)),
                after,
            )],
            vec![effect],
        )
    };
    let inbox_sha256 = snapshot.sha256.clone();
    let change = ChangeSet::from_parts(
        context.identity().clone(),
        files,
        effects,
        aplan_core::mutation::Delta::default(),
        false,
        ChangeEvidence::NotRequired,
    )
    .with_inbox_guard(InboxGuard {
        root: context.identity().plan_root.clone(),
        sha256: snapshot.sha256,
    });
    Ok(CaptureMutationPreview {
        capture,
        change,
        idempotent_retry: false,
        inbox_sha256,
    })
}

fn serialized_capture(capture: &Capture) -> Result<String, CaptureError> {
    aplan_core::capture::render_capture(capture).map_err(|detail| CaptureError::Serialize {
        id: capture.id.clone(),
        detail,
    })
}

fn empty_change(context: &PlanContext, snapshot: &CaptureSnapshot) -> ChangeSet {
    ChangeSet::from_parts(
        context.identity().clone(),
        Vec::new(),
        Vec::new(),
        aplan_core::mutation::Delta::default(),
        false,
        ChangeEvidence::NotRequired,
    )
    .with_inbox_guard(InboxGuard {
        root: context.identity().plan_root.clone(),
        sha256: snapshot.sha256.clone(),
    })
}

fn capture_from_effective(id: &str, effective: EffectiveCapture, clock: &impl Clock) -> Capture {
    Capture {
        schema: CaptureSchema::V1,
        id: id.to_string(),
        title: effective.title,
        kind: effective.kind,
        state: CaptureState::Open,
        captured_at: aplan_core::capture::format_capture_instant(clock.now_epoch()),
        origin: effective.origin,
        body: effective.body,
        labels: effective.labels,
        revisit_at: effective.revisit_at,
        profile: effective.profile,
        promotion_defaults: effective.promotion_defaults,
        source: effective.source,
        disposed_at: None,
        reason: None,
        promoted_to: None,
        promotion_fingerprint: None,
    }
}

fn capture_snapshot(
    context: &PlanContext,
    clock: &impl Clock,
) -> Result<CaptureSnapshot, CaptureError> {
    let config = context.report_config().ok_or_else(|| {
        CaptureError::Snapshot(
            "capture operations require resolved repository configuration on PlanContext"
                .to_string(),
        )
    })?;
    let root = &context.identity().plan_root;
    let before = capture_snapshot_digest(root).map_err(CaptureError::Snapshot)?;
    let policy = config.capture_policy();
    let inbox = aplan_core::capture::load_inbox(root, &policy, clock);
    let after = capture_snapshot_digest(root).map_err(CaptureError::Snapshot)?;
    if before != after {
        return Err(CaptureError::Snapshot(
            "the inbox changed while its guarded snapshot was being loaded; retry the operation"
                .to_string(),
        ));
    }
    if !inbox.is_clean() {
        return Err(CaptureError::Refused(format!(
            "the capture inbox has {} `APLAN057` finding(s); run `aplan check` and repair every record before using a partial snapshot",
            inbox.diagnostics.len()
        )));
    }
    Ok(CaptureSnapshot {
        inbox,
        policy,
        sha256: after,
    })
}

fn resolve<'a>(
    inbox: &'a Inbox,
    selector: &CaptureSelector,
) -> Result<&'a LoadedCapture, CaptureError> {
    match selector {
        CaptureSelector::Exact(id) => inbox
            .captures
            .iter()
            .find(|loaded| loaded.capture.id == id.as_str())
            .ok_or_else(|| CaptureError::Input(format!("no capture `{}`", id.as_str()))),
        CaptureSelector::HumanPrefix(prefix) => inbox
            .resolve_prefix(prefix.as_str())
            .map_err(CaptureError::Input),
    }
}

fn state_name(state: CaptureState) -> &'static str {
    match state {
        CaptureState::Open => "open",
        CaptureState::Promoted => "promoted",
        CaptureState::Discarded => "discarded",
    }
}

/// Stable digest of one focused record, used to bind editor input to the bytes
/// it was seeded from without exposing those bytes in a request.
#[must_use]
pub fn capture_fingerprint(capture: &Capture) -> String {
    let bytes = aplan_core::capture::render_capture(capture)
        .expect("a loaded typed capture always serializes through its own contract");
    hex(&Sha256::digest(bytes.as_bytes()))
}

fn capture_snapshot_digest(root: &Path) -> Result<String, String> {
    let directory = root.join(aplan_core::capture::INBOX_DIR);
    let mut digest = Sha256::new();
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(hex(&digest.finalize()));
        }
        Err(error) => return Err(format!("reading {}: {error}", directory.display())),
    };
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("reading {}: {error}", directory.display()))?;
    paths.sort();
    for path in paths {
        let name_os = path
            .file_name()
            .ok_or_else(|| format!("{} has no filename", path.display()))?;
        if aplan_core::capture::is_capture_staging_name(name_os) {
            continue;
        }
        let name = name_os.as_encoded_bytes();
        hash_part(&mut digest, b"name", name);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("reading {} metadata: {error}", path.display()))?;
        let kind = if metadata.file_type().is_file() {
            b"file".as_slice()
        } else if metadata.file_type().is_dir() {
            b"directory".as_slice()
        } else if metadata.file_type().is_symlink() {
            b"symlink".as_slice()
        } else {
            b"other".as_slice()
        };
        hash_part(&mut digest, b"kind", kind);
        if metadata.file_type().is_file() {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            hash_part(&mut digest, b"bytes", &bytes);
        } else if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&path)
                .map_err(|error| format!("reading {}: {error}", path.display()))?;
            hash_part(
                &mut digest,
                b"target",
                target.as_os_str().as_encoded_bytes(),
            );
        }
    }
    Ok(hex(&digest.finalize()))
}

fn hash_part(digest: &mut Sha256, label: &[u8], bytes: &[u8]) {
    digest.update(label.len().to_le_bytes());
    digest.update(label);
    digest.update(bytes.len().to_le_bytes());
    digest.update(bytes);
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

struct FixedClockRef(i64);

impl Clock for FixedClockRef {
    fn now_epoch(&self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    const FIRST: &str = "I-01K8Z3A4H00000000000000000";
    const SECOND: &str = "I-01K8Z3A4H00000000000000001";
    const NOW: i64 = 1_767_225_600;

    struct MovingClock {
        inbox: PathBuf,
        moved: Cell<bool>,
    }

    struct CountingClock {
        calls: Cell<usize>,
    }

    impl Clock for CountingClock {
        fn now_epoch(&self) -> i64 {
            self.calls.set(self.calls.get() + 1);
            NOW
        }
    }

    impl Clock for MovingClock {
        fn now_epoch(&self) -> i64 {
            if !self.moved.replace(true) {
                std::fs::write(self.inbox.join("concurrent-member"), "arrived")
                    .expect("concurrent member");
            }
            NOW
        }
    }

    fn context(root: &Path) -> PlanContext {
        context_with_config(root, aplan_config::Config::default())
    }

    fn context_with_config(root: &Path, config: aplan_config::Config) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(root),
            crate::ContextInputs {
                repository_root: root.to_path_buf(),
                configuration_file: None,
                trust: crate::Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(config)
    }

    fn request(source: Option<&str>) -> CaptureCreate {
        CaptureCreate {
            title: "captured".to_string(),
            kind: None,
            body: None,
            labels: None,
            revisit_at: None,
            profile: None,
            promotion_defaults: None,
            source: source.map(|key| aplan_core::capture::CaptureSource {
                namespace: "test".to_string(),
                key: key.to_string(),
            }),
            origin: aplan_core::capture::CaptureOrigin::Cli,
        }
    }

    fn promotion_request(id: &str) -> CapturePromotionRequest {
        CapturePromotionRequest {
            selector: CaptureSelector::Exact(ExactCaptureId::new(id).expect("exact id")),
            task_id: Some("A-1".to_string()),
            phase: Some(1),
            cut: None,
            effort_days: Some(1.0),
            owner: None,
            paths: Some(vec!["src/**".to_string()]),
            tags: Some(vec!["cross-cutting".to_string()]),
            reason: None,
        }
    }

    fn create_capture(root: &Path, source: Option<&str>) -> Capture {
        let current = context(root);
        let create = preview_capture_create_with_id(
            &current,
            &request(source),
            &aplan_core::capture::FixedClock(NOW),
            FIRST,
        )
        .expect("creation");
        assert!(matches!(
            crate::commit_change_set(&current, create.into_change_set()),
            crate::CommitOutcome::Applied(_)
        ));
        show_capture(
            &context(root),
            &CaptureSelector::Exact(ExactCaptureId::new(FIRST).expect("exact id")),
            &aplan_core::capture::FixedClock(NOW),
        )
        .expect("created capture")
        .capture
    }

    fn retained_capture(index: usize, state: CaptureState) -> Capture {
        let id = format!("I-01K8Z3A4H00000000000000{index:03}");
        assert!(aplan_core::capture::is_capture_id(&id), "{id}");
        let terminal = state != CaptureState::Open;
        Capture {
            schema: CaptureSchema::V1,
            id,
            title: format!("capture {index:03}"),
            kind: if index.is_multiple_of(2) {
                "question".to_string()
            } else {
                "task".to_string()
            },
            state,
            captured_at: "2025-12-31T23:59:00Z".to_string(),
            origin: aplan_core::capture::CaptureOrigin::Tui,
            body: Some(format!("body {index:03}")),
            labels: vec![format!("label-{}", index % 3)],
            revisit_at: (state == CaptureState::Open && index < 2)
                .then(|| "2030-01-01T00:00:00Z".to_string()),
            profile: None,
            promotion_defaults: None,
            source: None,
            disposed_at: terminal.then(|| "2026-01-01T00:00:00Z".to_string()),
            reason: (state == CaptureState::Discarded).then(|| "reviewed".to_string()),
            promoted_to: (state == CaptureState::Promoted).then(|| format!("T-{index:03}")),
            promotion_fingerprint: (state == CaptureState::Promoted).then(|| "a".repeat(64)),
        }
    }

    fn write_retained(root: &Path, capture: &Capture) {
        let inbox = root.join(aplan_core::capture::INBOX_DIR);
        std::fs::create_dir_all(&inbox).expect("inbox");
        std::fs::write(
            inbox.join(format!("{}.yml", capture.id)),
            aplan_core::capture::render_capture(capture).expect("capture YAML"),
        )
        .expect("capture file");
    }

    #[test]
    fn exact_machine_ids_and_human_prefixes_are_different_types() {
        let full = "I-01K8Z3A4H00000000000000000";
        let exact = ExactCaptureId::new(full).expect("full id");
        assert_eq!(exact.as_str(), full);
        assert!(ExactCaptureId::new("I-01K8").is_err());
        assert!(HumanCapturePrefix::new("I-01K8").is_ok());
        assert!(HumanCapturePrefix::new("01K8").is_err());
    }

    #[test]
    fn capture_workspace_is_single_clock_bounded_complete_and_configuration_aware() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let per_state = aplan_core::capture::MAX_LIST_LIMIT + 1;
        for (group, state) in [
            CaptureState::Open,
            CaptureState::Promoted,
            CaptureState::Discarded,
        ]
        .into_iter()
        .enumerate()
        {
            for offset in 0..per_state {
                write_retained(
                    dir.path(),
                    &retained_capture(group * per_state + offset, state),
                );
            }
        }
        let profile = CaptureProfile {
            kind: Some("question".to_string()),
            labels: vec!["profiled".to_string()],
            ..CaptureProfile::default()
        };
        let config = aplan_config::Config {
            capture: Some(aplan_config::CaptureConfig {
                kinds: vec!["question".to_string()],
                profiles: BTreeMap::from([("triage".to_string(), profile.clone())]),
            }),
            ..aplan_config::Config::default()
        };
        config.validate_capture().expect("capture policy");
        let clock = CountingClock {
            calls: Cell::new(0),
        };
        let workspace =
            capture_workspace(&context_with_config(dir.path(), config), &clock).expect("workspace");

        assert_eq!(
            clock.calls.get(),
            1,
            "the operation clock moved mid-snapshot"
        );
        assert_eq!(workspace.evaluated_at, "2026-01-01T00:00:00Z");
        assert_eq!(workspace.deferred_open, 2);
        assert_eq!(workspace.total_open, per_state);
        assert_eq!(workspace.total_promoted, per_state);
        assert_eq!(workspace.total_discarded, per_state);
        assert_eq!(workspace.total(), per_state * 3);
        assert_eq!(
            workspace.captures.len(),
            aplan_core::capture::MAX_LIST_LIMIT * 3
        );
        assert!(workspace.is_truncated());
        assert!(workspace.kinds.iter().any(|kind| kind == "question"));
        assert_eq!(workspace.profiles, vec![("triage".to_string(), profile)]);
        assert!(
            workspace.captures[..aplan_core::capture::MAX_LIST_LIMIT]
                .iter()
                .all(|capture| capture.state == CaptureState::Open)
        );
    }

    #[test]
    fn capture_publication_reconciliation_binds_full_inbox_and_plan_snapshots() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).expect("source directory");
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn fixture() {}\n")
            .expect("source file");

        let initial = context(dir.path());
        let create = preview_capture_create_with_id(
            &initial,
            &request(None),
            &aplan_core::capture::FixedClock(NOW),
            FIRST,
        )
        .expect("creation preview");
        let crate::CommitOutcome::Applied(create_change) =
            crate::commit_change_set(&initial, create.into_change_set())
        else {
            panic!("capture creation did not publish")
        };
        let created = context(dir.path());
        let created_workspace = capture_workspace(&created, &aplan_core::capture::FixedClock(NOW))
            .expect("created workspace");
        assert!(capture_snapshot_observes_publication(
            &created,
            &created_workspace.inbox_sha256,
            &create_change,
            &aplan_core::capture::FixedClock(NOW),
        ));
        assert!(!capture_snapshot_observes_publication(
            &created,
            "stale-inbox-digest",
            &create_change,
            &aplan_core::capture::FixedClock(NOW),
        ));

        let promote = preview_capture_promotion(
            &created,
            &promotion_request(FIRST),
            &aplan_core::capture::FixedClock(NOW + 1),
        )
        .expect("promotion preview");
        let crate::CommitOutcome::Applied(promotion_change) =
            crate::commit_change_set(&created, promote.into_change_set())
        else {
            panic!("promotion did not publish")
        };
        let promoted = context(dir.path());
        let promoted_workspace =
            capture_workspace(&promoted, &aplan_core::capture::FixedClock(NOW + 1))
                .expect("promoted workspace");
        assert!(capture_snapshot_observes_publication(
            &promoted,
            &promoted_workspace.inbox_sha256,
            &promotion_change,
            &aplan_core::capture::FixedClock(NOW + 1),
        ));
        assert!(
            promotion_change
                .files()
                .iter()
                .any(|file| file.path().ends_with("A-1.md")),
            "the mixed task/capture snapshot path was not exercised"
        );
    }

    #[test]
    fn capture_input_refusals_name_the_policy_identity_and_configuration() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let current = context(dir.path());

        let unknown_kind = list_captures(
            &current,
            &CaptureQuery {
                kind: Some("undeclared".into()),
                ..CaptureQuery::default()
            },
            &aplan_core::capture::FixedClock(NOW),
        )
        .expect_err("unknown kind");
        assert!(unknown_kind.to_string().contains("capture.kinds"));

        let invalid_id = preview_capture_create_with_id(
            &current,
            &request(None),
            &aplan_core::capture::FixedClock(NOW),
            "I-short",
        )
        .expect_err("invalid generated id");
        assert!(invalid_id.to_string().contains("canonical `I-<ULID>`"));

        let without_config = PlanContext::new(
            aplan_core::load_plan_identified(dir.path()),
            crate::ContextInputs {
                repository_root: dir.path().to_path_buf(),
                configuration_file: None,
                trust: crate::Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        );
        let missing_config = list_captures(
            &without_config,
            &CaptureQuery::default(),
            &aplan_core::capture::FixedClock(NOW),
        )
        .expect_err("missing resolved configuration");
        assert!(
            missing_config
                .to_string()
                .contains("resolved repository configuration")
        );
    }

    #[test]
    fn exact_selectors_and_focused_edit_preconditions_refuse_their_subject() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let created = create_capture(dir.path(), Some("stable"));
        let refreshed = context(dir.path());
        let exact = CaptureSelector::Exact(ExactCaptureId::new(FIRST).expect("exact id"));
        let focused = show_capture(&refreshed, &exact, &aplan_core::capture::FixedClock(NOW))
            .expect("exact focused read");
        assert_eq!(focused.capture, created);

        let missing = CaptureSelector::Exact(ExactCaptureId::new(SECOND).expect("exact id"));
        let missing_error =
            show_capture(&refreshed, &missing, &aplan_core::capture::FixedClock(NOW))
                .expect_err("missing exact id");
        assert!(missing_error.to_string().contains(SECOND));

        let stale = preview_capture_amend(
            &refreshed,
            &CaptureAmendRequest {
                selector: Some(exact.clone()),
                title: Some("new title".into()),
                expected_sha256: Some("not-the-focused-fingerprint".into()),
                ..CaptureAmendRequest::default()
            },
            &aplan_core::capture::FixedClock(NOW),
        )
        .expect_err("stale focused edit");
        assert!(stale.to_string().contains("changed while"));
    }

    #[test]
    fn terminal_source_conflicts_and_hostile_members_fail_the_full_snapshot() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let mut promoted = create_capture(dir.path(), Some("stable"));
        promoted.state = CaptureState::Promoted;
        promoted.disposed_at = Some("2026-01-01T00:00:00Z".into());
        promoted.promoted_to = Some("A-1".into());
        promoted.promotion_fingerprint = Some("a".repeat(64));
        let capture_path = dir
            .path()
            .join(aplan_core::capture::INBOX_DIR)
            .join(format!("{FIRST}.yml"));
        std::fs::write(
            &capture_path,
            aplan_core::capture::render_capture(&promoted).expect("promoted capture"),
        )
        .expect("write promoted capture");
        let conflict = preview_capture_create_with_id(
            &context(dir.path()),
            &CaptureCreate {
                title: "different".into(),
                ..request(Some("stable"))
            },
            &aplan_core::capture::FixedClock(NOW),
            SECOND,
        )
        .expect_err("promoted source conflict");
        let conflict = conflict.to_string();
        assert!(
            conflict.contains(FIRST) && conflict.contains("promoted"),
            "{conflict}"
        );

        let inbox = dir.path().join(aplan_core::capture::INBOX_DIR);
        std::fs::create_dir(inbox.join("foreign-directory")).expect("foreign directory");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&capture_path, inbox.join("foreign-symlink"))
            .expect("foreign symlink");
        let invalid_snapshot = list_captures(
            &context(dir.path()),
            &CaptureQuery::default(),
            &aplan_core::capture::FixedClock(NOW),
        )
        .expect_err("invalid directory members");
        assert!(invalid_snapshot.to_string().contains("APLAN057"));
    }

    #[test]
    fn snapshot_refuses_non_directory_inbox_and_concurrent_membership_change() {
        let malformed = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(malformed.path().join("aplan")).expect("plan directory");
        std::fs::write(
            malformed.path().join(aplan_core::capture::INBOX_DIR),
            "not a directory",
        )
        .expect("inbox-shaped file");
        let unreadable = list_captures(
            &context(malformed.path()),
            &CaptureQuery::default(),
            &aplan_core::capture::FixedClock(NOW),
        )
        .expect_err("inbox must be a directory");
        assert!(unreadable.to_string().contains("reading"), "{unreadable}");

        let moving = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        create_capture(moving.path(), None);
        let inbox = moving.path().join(aplan_core::capture::INBOX_DIR);
        let changed = list_captures(
            &context(moving.path()),
            &CaptureQuery::default(),
            &MovingClock {
                inbox,
                moved: Cell::new(false),
            },
        )
        .expect_err("moving snapshot");
        assert!(changed.to_string().contains("changed while"), "{changed}");
    }

    #[test]
    fn capture_id_collisions_refuse_overwrite() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let initial = context(dir.path());
        let create = preview_capture_create_with_id(
            &initial,
            &request(None),
            &aplan_core::capture::FixedClock(NOW),
            FIRST,
        )
        .expect("first creation");
        assert!(matches!(
            crate::commit_change_set(&initial, create.into_change_set()),
            crate::CommitOutcome::Applied(_)
        ));
        let bytes = std::fs::read_to_string(
            dir.path()
                .join(aplan_core::capture::INBOX_DIR)
                .join(format!("{FIRST}.yml")),
        )
        .expect("first bytes");

        let refreshed = context(dir.path());
        let collision = preview_capture_create_with_id(
            &refreshed,
            &CaptureCreate {
                title: "different".to_string(),
                ..request(None)
            },
            &aplan_core::capture::FixedClock(NOW),
            FIRST,
        )
        .expect_err("collision");
        assert!(matches!(
            collision,
            CaptureError::IdentityCollision { ref id } if id == FIRST
        ));
        assert_eq!(
            std::fs::read_to_string(
                dir.path()
                    .join(aplan_core::capture::INBOX_DIR)
                    .join(format!("{FIRST}.yml"))
            )
            .expect("unchanged bytes"),
            bytes
        );
    }

    /// Two different ULID files still share source uniqueness, so the later
    /// writer must be made stale by the directory guard rather than relying on
    /// a destination-file collision that can never happen here.
    #[test]
    fn source_uniqueness_is_rechecked_under_one_inbox_publication_lock() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let first_context = context(dir.path());
        let second_context = context(dir.path());
        let first = preview_capture_create_with_id(
            &first_context,
            &request(Some("same")),
            &aplan_core::capture::FixedClock(NOW),
            FIRST,
        )
        .expect("first preview");
        let second = preview_capture_create_with_id(
            &second_context,
            &request(Some("same")),
            &aplan_core::capture::FixedClock(NOW),
            SECOND,
        )
        .expect("second preview");
        assert!(matches!(
            crate::commit_change_set(&first_context, first.into_change_set()),
            crate::CommitOutcome::Applied(_)
        ));
        let stale = crate::commit_change_set(&second_context, second.into_change_set());
        assert!(matches!(stale, crate::CommitOutcome::Stale { .. }));

        let refreshed = context(dir.path());
        let retry = preview_capture_create_with_id(
            &refreshed,
            &request(Some("same")),
            &aplan_core::capture::FixedClock(NOW),
            SECOND,
        )
        .expect("retry preview");
        assert_eq!(retry.capture().id, FIRST);
        assert!(retry.idempotent_retry());
        assert_eq!(
            retry.change_set().disposition(),
            crate::ChangeDisposition::NoOp
        );
    }

    /// A prefix is resolved once against the guarded snapshot. Adding another
    /// matching member cannot make the already-reviewed mutation choose either
    /// one; it makes that mutation stale.
    #[test]
    fn prefix_membership_changes_make_mutations_stale() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let initial = context(dir.path());
        let create = preview_capture_create_with_id(
            &initial,
            &request(None),
            &aplan_core::capture::FixedClock(NOW),
            FIRST,
        )
        .expect("creation");
        assert!(matches!(
            crate::commit_change_set(&initial, create.into_change_set()),
            crate::CommitOutcome::Applied(_)
        ));

        let editing = context(dir.path());
        let preview = preview_capture_amend(
            &editing,
            &CaptureAmendRequest {
                selector: Some(CaptureSelector::HumanPrefix(
                    HumanCapturePrefix::new(&FIRST[..27]).expect("prefix"),
                )),
                title: Some("amended".to_string()),
                ..CaptureAmendRequest::default()
            },
            &aplan_core::capture::FixedClock(NOW),
        )
        .expect("amendment preview");

        let concurrent = context(dir.path());
        let second = preview_capture_create_with_id(
            &concurrent,
            &request(None),
            &aplan_core::capture::FixedClock(NOW),
            SECOND,
        )
        .expect("second creation");
        assert!(matches!(
            crate::commit_change_set(&concurrent, second.into_change_set()),
            crate::CommitOutcome::Applied(_)
        ));
        assert!(matches!(
            crate::commit_change_set(&editing, preview.into_change_set()),
            crate::CommitOutcome::Stale { .. }
        ));

        let current = context(dir.path());
        let selector =
            CaptureSelector::HumanPrefix(HumanCapturePrefix::new(&FIRST[..27]).expect("prefix"));
        let error = show_capture(&current, &selector, &aplan_core::capture::FixedClock(NOW))
            .expect_err("prefix is now ambiguous");
        assert!(error.to_string().contains("ambiguous"), "{error}");
    }

    #[test]
    fn concurrent_inbox_writer_makes_whole_promotion_stale() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        create_capture(dir.path(), None);
        let promoting = context(dir.path());
        let preview = preview_capture_promotion(
            &promoting,
            &promotion_request(FIRST),
            &aplan_core::capture::FixedClock(NOW + 1),
        )
        .expect("promotion preview");

        let concurrent = context(dir.path());
        let second = preview_capture_create_with_id(
            &concurrent,
            &request(None),
            &aplan_core::capture::FixedClock(NOW + 1),
            SECOND,
        )
        .expect("concurrent capture preview");
        assert!(matches!(
            crate::commit_change_set(&concurrent, second.into_change_set()),
            crate::CommitOutcome::Applied(_)
        ));
        assert!(matches!(
            crate::commit_change_set(&promoting, preview.into_change_set()),
            crate::CommitOutcome::Stale { .. }
        ));
        assert!(!dir.path().join("A-1.md").exists());
        let first = show_capture(
            &context(dir.path()),
            &CaptureSelector::Exact(ExactCaptureId::new(FIRST).expect("exact id")),
            &aplan_core::capture::FixedClock(NOW + 1),
        )
        .expect("first capture");
        assert_eq!(first.capture.state, CaptureState::Open);
        assert!(first.capture.promoted_to.is_none());
    }

    #[test]
    fn promotion_preview_is_stale_when_selected_configuration_changes() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let config_path = dir.path().join("selected.yml");
        std::fs::write(&config_path, "{}\n").expect("config");
        let configured_context = || {
            let (config, _) = aplan_config::load_from(
                &aplan_config::ConfigSource::File(config_path.clone()),
                dir.path(),
            )
            .expect("load config");
            PlanContext::new(
                aplan_core::load_plan_identified(dir.path()),
                crate::ContextInputs {
                    repository_root: dir.path().to_path_buf(),
                    configuration_file: Some(config_path.clone()),
                    trust: crate::Trust::Hermetic,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            )
            .with_captured_report_config(config)
            .expect("capture config revision")
        };
        let initial = configured_context();
        let create = preview_capture_create_with_id(
            &initial,
            &request(None),
            &aplan_core::capture::FixedClock(NOW),
            FIRST,
        )
        .expect("capture preview");
        assert!(matches!(
            crate::commit_change_set(&initial, create.into_change_set()),
            crate::CommitOutcome::Applied(_)
        ));

        let current = configured_context();
        let capture_path = dir
            .path()
            .join(aplan_core::capture::INBOX_DIR)
            .join(format!("{FIRST}.yml"));
        let before = std::fs::read_to_string(&capture_path).expect("open capture");
        let preview = preview_capture_promotion(
            &current,
            &promotion_request(FIRST),
            &aplan_core::capture::FixedClock(NOW + 1),
        )
        .expect("promotion preview");
        std::fs::write(&config_path, "stale_after_days: 17\n").expect("changed config");

        let crate::CommitOutcome::Stale { change, .. } =
            crate::commit_change_set(&current, preview.into_change_set())
        else {
            panic!("selected configuration change should stale promotion");
        };
        assert!(change.effects().iter().any(|effect| matches!(
            effect,
            SemanticEffect::CapturePromoted { capture, task }
                if capture == FIRST && task == "A-1"
        )));
        assert_eq!(std::fs::read_to_string(capture_path).unwrap(), before);
        assert!(!dir.path().join("A-1.md").exists());
    }
}
