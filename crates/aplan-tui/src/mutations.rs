//! Closed-world interactive mutation catalogue and typed shell composition.
//!
//! The four workflow modules own their forms. This module owns the exhaustive
//! union consumed by the shell and by the cross-surface parity audit, so a new
//! native mutation cannot be implemented yet omitted from interactive driving.

use std::io::Read as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use aplan_app::{
    ChangeSet, PlanContext, WorkActionAvailability, WorkActionKind, WorkActionPreview,
};

use crate::authoring::{AuthoringAssets, AuthoringWorkflow, CapturedSource};
use crate::evidence::{EvidenceRequest, EvidenceWorkflow};
use crate::form::{FormSubmission, MutationForm};
use crate::inbox::CaptureWorkflow;
use crate::planning::PlanningWorkflow;
use crate::quick_actions::QuickActionWorkflow;

static NEXT_TRANSFER: AtomicU64 = AtomicU64::new(1);
const MAX_CAPTURE_BYTES: u64 = 4 * 1024 * 1024;

/// Every mutation the accepted TUI product contract treats as native.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MutationAction {
    CaptureCreate,
    CaptureAmend,
    CaptureDiscard,
    CaptureRestore,
    CapturePromote,
    Claim,
    Release,
    Start,
    Stop,
    DependencyAdd,
    DependencyRemove,
    PreferenceAdd,
    PreferenceRemove,
    TagAdd,
    TagRemove,
    TagRename,
    Move,
    DropTasks,
    DropCuts,
    Add,
    Manifest,
    Import,
    Supersede,
    Rekey,
    Close,
    AmendAdd,
    AmendReplace,
    AmendRemove,
    Restamp,
    VerifyFix,
}

impl MutationAction {
    pub const ALL: [Self; 30] = [
        Self::CaptureCreate,
        Self::CaptureAmend,
        Self::CaptureDiscard,
        Self::CaptureRestore,
        Self::CapturePromote,
        Self::Claim,
        Self::Release,
        Self::Start,
        Self::Stop,
        Self::DependencyAdd,
        Self::DependencyRemove,
        Self::PreferenceAdd,
        Self::PreferenceRemove,
        Self::TagAdd,
        Self::TagRemove,
        Self::TagRename,
        Self::Move,
        Self::DropTasks,
        Self::DropCuts,
        Self::Add,
        Self::Manifest,
        Self::Import,
        Self::Supersede,
        Self::Rekey,
        Self::Close,
        Self::AmendAdd,
        Self::AmendReplace,
        Self::AmendRemove,
        Self::Restamp,
        Self::VerifyFix,
    ];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::CaptureCreate => "Quick capture",
            Self::CaptureAmend => "Amend capture",
            Self::CaptureDiscard => "Discard capture",
            Self::CaptureRestore => "Restore capture",
            Self::CapturePromote => "Promote capture to task",
            Self::Claim => "Claim task",
            Self::Release => "Release claim",
            Self::Start => "Start task",
            Self::Stop => "Stop task",
            Self::DependencyAdd => "Add blocking dependency",
            Self::DependencyRemove => "Remove blocking dependency",
            Self::PreferenceAdd => "Add ordering preference",
            Self::PreferenceRemove => "Remove ordering preference",
            Self::TagAdd => "Add task tag",
            Self::TagRemove => "Remove task tag",
            Self::TagRename => "Rename tag across plan",
            Self::Move => "Move tasks",
            Self::DropTasks => "Drop tasks",
            Self::DropCuts => "Drop cuts and live members",
            Self::Add => "Add task",
            Self::Manifest => "Add tasks from manifest",
            Self::Import => "Import proposal tasks",
            Self::Supersede => "Supersede completed outcome",
            Self::Rekey => "Rekey task",
            Self::Close => "Close task with evidence",
            Self::AmendAdd => "Add evidence to closed task",
            Self::AmendReplace => "Replace evidence on closed task",
            Self::AmendRemove => "Remove evidence from closed task",
            Self::Restamp => "Review and restamp baselines",
            Self::VerifyFix => "Apply verification repairs",
        }
    }

    /// Command path whose durable semantics this action mirrors.
    #[must_use]
    pub const fn cli(self) -> &'static str {
        match self {
            Self::CaptureCreate => "capture",
            Self::CaptureAmend => "inbox amend",
            Self::CaptureDiscard => "inbox discard",
            Self::CaptureRestore => "inbox restore",
            Self::CapturePromote => "inbox promote",
            Self::Claim => "claim",
            Self::Release => "release",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::DependencyAdd => "dep add",
            Self::DependencyRemove => "dep rm",
            Self::PreferenceAdd => "dep add --prefer",
            Self::PreferenceRemove => "dep rm --prefer",
            Self::TagAdd => "tag add",
            Self::TagRemove => "tag rm",
            Self::TagRename => "tag rename",
            Self::Move => "mv",
            Self::DropTasks => "drop",
            Self::DropCuts => "drop --cut",
            Self::Add => "add",
            Self::Manifest => "add --from",
            Self::Import => "import",
            Self::Supersede => "supersede",
            Self::Rekey => "rekey",
            Self::Close => "close",
            Self::AmendAdd => "amend --add-evidence",
            Self::AmendReplace => "amend --replace-evidence",
            Self::AmendRemove => "amend --remove-evidence",
            Self::Restamp => "close --restamp",
            Self::VerifyFix => "verify --fix",
        }
    }

    /// Top-level command names and domain words which discover this action.
    ///
    /// The shell searches these aliases as well as the visible title and CLI
    /// path. Configured-test discovery has its own selector action, which
    /// inserts one exact result into these evidence forms.
    #[must_use]
    pub const fn discovery_aliases(self) -> &'static [&'static str] {
        match self {
            Self::CaptureCreate => &["capture", "inbox", "intake", "quick"],
            Self::CaptureAmend => &["inbox", "capture", "amend", "edit"],
            Self::CaptureDiscard => &["inbox", "capture", "discard"],
            Self::CaptureRestore => &["inbox", "capture", "restore"],
            Self::CapturePromote => &["inbox", "capture", "promote", "task"],
            Self::Claim => &["claim", "ownership"],
            Self::Release => &["release", "claim", "ownership"],
            Self::Start => &["start", "status"],
            Self::Stop => &["stop", "status"],
            Self::DependencyAdd | Self::DependencyRemove => &["dep", "dependency", "blocker"],
            Self::PreferenceAdd | Self::PreferenceRemove => &["dep", "preference", "ordering"],
            Self::TagAdd | Self::TagRemove | Self::TagRename => &["tag"],
            Self::Move => &["mv", "move"],
            Self::DropTasks | Self::DropCuts => &["drop"],
            Self::Add | Self::Manifest => &["add"],
            Self::Import => &["import"],
            Self::Supersede => &["supersede"],
            Self::Rekey => &["rekey"],
            Self::Close => &["close", "evidence"],
            Self::AmendAdd | Self::AmendReplace | Self::AmendRemove => &["amend", "evidence"],
            Self::Restamp => &["close", "restamp", "trace"],
            Self::VerifyFix => &["verify", "fix", "repair"],
        }
    }

    #[must_use]
    pub const fn needs_selected_task(self) -> bool {
        matches!(
            self,
            Self::Claim
                | Self::Release
                | Self::Start
                | Self::Stop
                | Self::DependencyAdd
                | Self::DependencyRemove
                | Self::PreferenceAdd
                | Self::PreferenceRemove
                | Self::TagAdd
                | Self::TagRemove
                | Self::DropTasks
                | Self::Supersede
                | Self::Rekey
                | Self::Close
                | Self::AmendAdd
                | Self::AmendReplace
                | Self::AmendRemove
                | Self::Restamp
        )
    }

    #[must_use]
    pub const fn needs_selected_capture(self) -> bool {
        matches!(
            self,
            Self::CaptureAmend | Self::CaptureDiscard | Self::CaptureRestore | Self::CapturePromote
        )
    }

    /// CLI modes which deliberately remain outside the native interaction.
    ///
    /// These sentences are rendered in the form or zero-input review rather
    /// than existing only in a parity test, so an unavailable mode has a
    /// discoverable reason at the point where a user chooses the action.
    #[must_use]
    pub const fn cli_only_help(self) -> &'static [&'static str] {
        const POLICY: &str = "One-run --deny-warnings/--no-deny-warnings overrides stay CLI-only; the TUI uses the loaded plan policy.";
        const OUT: &str = "Writing outside the loaded plan with --out stays CLI-only; TUI previews publish only to the loaded plan.";
        const DEPENDENCY_BATCH: &str = "Batch dependency manifests selected with dep apply --from stay CLI-only; the TUI reviews one dependency edge at a time.";
        match self {
            Self::CaptureCreate => &[
                "Retry source identity from --source stays CLI-only because native captures have the explicit tui origin and no upstream machine identity.",
                "Reading --body-file or launching --edit stays CLI-only; paste short detail into the native Body field.",
                "Explicit --promotion-phase, --promotion-cut, --promotion-effort, --promotion-owner, --promotion-path and --promotion-tag defaults stay CLI-only; select a configured profile or amend the retained capture before promotion.",
            ],
            Self::CaptureAmend => &[
                "Reading --body-file or launching --edit stays CLI-only; paste replacement detail into the native Body field.",
            ],
            Self::CaptureDiscard | Self::CaptureRestore | Self::Claim | Self::Release => &[],
            Self::CapturePromote => &[
                "One-run --deny-warnings/--no-deny-warnings overrides stay CLI-only; the TUI uses the loaded plan policy.",
            ],
            Self::Start
            | Self::Stop
            | Self::PreferenceAdd
            | Self::PreferenceRemove
            | Self::TagAdd
            | Self::TagRemove
            | Self::TagRename
            | Self::Move
            | Self::DropTasks
            | Self::DropCuts
            | Self::Supersede => &[POLICY],
            Self::DependencyAdd | Self::DependencyRemove => &[DEPENDENCY_BATCH, POLICY],
            Self::Add | Self::Manifest => &[OUT, POLICY],
            Self::Import => &[OUT],
            Self::Rekey => &[
                "Machine JSON output stays CLI-only; the TUI shows the same typed review interactively.",
                POLICY,
            ],
            Self::Close
            | Self::AmendAdd
            | Self::AmendReplace
            | Self::AmendRemove
            | Self::Restamp => &[
                "Selecting another evidence repository with --repo stays CLI-only; the TUI uses its loaded repository.",
                POLICY,
            ],
            Self::VerifyFix => &[
                "The whole-plan verification gate refuses --cut, --phase, --tag, --owner, --unclaimed and --status; use Tasks to inspect a selected subset.",
                "One-run --no-require-effort, --traces-root, --no-traces, --deny-warnings and --no-deny-warnings verification policy overrides stay CLI-only; the TUI uses the loaded plan policy.",
                "Trust-bearing --test-receipts, --refresh-test-receipts, --refresh-base, --refresh-head, --refresh-working-tree and --refresh-shadow automation stays CLI-only; TUI repair uses the displayed identified verification run.",
            ],
        }
    }

    #[must_use]
    pub const fn quick(self) -> Option<QuickActionWorkflow> {
        match self {
            Self::Claim => Some(QuickActionWorkflow::Claim),
            Self::Release => Some(QuickActionWorkflow::Release),
            Self::Start => Some(QuickActionWorkflow::Start),
            Self::Stop => Some(QuickActionWorkflow::Stop),
            _ => None,
        }
    }

    #[must_use]
    pub const fn planning(self) -> Option<PlanningWorkflow> {
        match self {
            Self::DependencyAdd => Some(PlanningWorkflow::DependencyAdd),
            Self::DependencyRemove => Some(PlanningWorkflow::DependencyRemove),
            Self::PreferenceAdd => Some(PlanningWorkflow::PreferenceAdd),
            Self::PreferenceRemove => Some(PlanningWorkflow::PreferenceRemove),
            Self::TagAdd => Some(PlanningWorkflow::TagAdd),
            Self::TagRemove => Some(PlanningWorkflow::TagRemove),
            Self::TagRename => Some(PlanningWorkflow::TagRename),
            Self::Move => Some(PlanningWorkflow::Move),
            Self::DropTasks => Some(PlanningWorkflow::DropTasks),
            Self::DropCuts => Some(PlanningWorkflow::DropCuts),
            _ => None,
        }
    }

    #[must_use]
    pub const fn authoring(self) -> Option<AuthoringWorkflow> {
        match self {
            Self::Add => Some(AuthoringWorkflow::Add),
            Self::Manifest => Some(AuthoringWorkflow::Manifest),
            Self::Import => Some(AuthoringWorkflow::Import),
            Self::Supersede => Some(AuthoringWorkflow::Supersede),
            Self::Rekey => Some(AuthoringWorkflow::Rekey),
            _ => None,
        }
    }

    #[must_use]
    pub const fn evidence(self) -> Option<EvidenceWorkflow> {
        match self {
            Self::Close => Some(EvidenceWorkflow::Close),
            Self::AmendAdd => Some(EvidenceWorkflow::AmendAdd),
            Self::AmendReplace => Some(EvidenceWorkflow::AmendReplace),
            Self::AmendRemove => Some(EvidenceWorkflow::AmendRemove),
            Self::Restamp => Some(EvidenceWorkflow::Restamp),
            Self::VerifyFix => Some(EvidenceWorkflow::VerifyFix),
            _ => None,
        }
    }

    #[must_use]
    pub const fn capture(self) -> Option<CaptureWorkflow> {
        match self {
            Self::CaptureCreate => Some(CaptureWorkflow::Create),
            Self::CaptureAmend => Some(CaptureWorkflow::Amend),
            Self::CaptureDiscard => Some(CaptureWorkflow::Discard),
            Self::CaptureRestore => Some(CaptureWorkflow::Restore),
            Self::CapturePromote => Some(CaptureWorkflow::Promote),
            _ => None,
        }
    }
}

/// Result of opening one action against an identified snapshot.
#[derive(Debug)]
pub enum PreparedMutation {
    Form(Box<MutationForm>),
    Preview(MutationCandidate),
}

/// Exact value retained between visible preview and confirmation.
#[derive(Debug)]
pub enum MutationCandidate {
    Plan {
        action: MutationAction,
        change: Box<ChangeSet>,
        review_rows: Vec<String>,
        selection_after_commit: Option<String>,
    },
    Work {
        preview: Box<WorkActionPreview>,
    },
}

/// Non-cloning handoff across the bounded worker boundary.
///
/// Effect requests are cloned into the reducer's audit record. The payload is
/// therefore held once behind a shared token and consumed exactly once by the
/// receiver; cloning the token cannot clone or replay a reviewed mutation.
#[derive(Clone)]
pub(crate) struct MutationTransfer {
    id: u64,
    payload: Arc<Mutex<Option<MutationPayload>>>,
}

impl MutationTransfer {
    pub(crate) fn new(payload: MutationPayload) -> Self {
        Self {
            id: NEXT_TRANSFER.fetch_add(1, Ordering::Relaxed),
            payload: Arc::new(Mutex::new(Some(payload))),
        }
    }

    pub(crate) fn take(&self) -> Option<MutationPayload> {
        self.payload.lock().ok()?.take()
    }
}

impl std::fmt::Debug for MutationTransfer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MutationTransfer")
            .field("id", &self.id)
            .field(
                "available",
                &self.payload.lock().is_ok_and(|payload| payload.is_some()),
            )
            .finish()
    }
}

impl PartialEq for MutationTransfer {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for MutationTransfer {}

#[derive(Debug)]
pub(crate) enum MutationPayload {
    Prepared(PreparedMutation),
    Preview {
        request: u64,
        candidate: MutationCandidate,
    },
    PreviewFailed {
        action: MutationAction,
        request: u64,
        message: String,
    },
    CommitCandidate(MutationCandidate),
    Committed(MutationCommitResult),
}

#[derive(Debug)]
pub(crate) struct MutationReload {
    pub(crate) snapshot: Box<crate::model::Snapshot>,
    pub(crate) input_sha256: String,
    pub(crate) publication: Option<aplan_app::PublicationReport>,
}

#[derive(Debug)]
pub(crate) enum MutationCommitResult {
    Plan {
        outcome: Box<aplan_app::CommitOutcome>,
        reload: Result<Option<MutationReload>, String>,
        selection_after_commit: Option<String>,
    },
    Work {
        outcome: Box<aplan_app::WorkActionCommitOutcome>,
        reload: Result<Option<MutationReload>, String>,
    },
}

/// Work the repository worker performs for one mutation layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MutationEffect {
    Prepare {
        action: MutationAction,
        selected_task: Option<String>,
    },
    Preview {
        action: MutationAction,
        selected_task: Option<String>,
        submission: FormSubmission,
        verification: Option<Box<aplan_app::VerificationWorkbench>>,
        /// A live terminal handoff can return a long Add body without copying
        /// it into the bounded single-line form editor. `Arc` keeps cloned
        /// effect audit records cheap while the worker owns the one conversion
        /// into the application request.
        edited_body: Option<Arc<str>>,
    },
    Commit {
        candidate: MutationTransfer,
    },
}

impl MutationCandidate {
    #[must_use]
    pub const fn action(&self) -> MutationAction {
        match self {
            Self::Plan { action, .. } => *action,
            Self::Work { preview, .. } => mutation_action(preview.request().action),
        }
    }

    #[must_use]
    pub fn review_rows(&self) -> Vec<String> {
        match self {
            Self::Plan { review_rows, .. } => review_rows.clone(),
            Self::Work { preview } => {
                let mut rows = crate::quick_actions::review_rows(preview);
                rows.extend(cli_review_rows(self.action()));
                rows
            }
        }
    }
}

const fn mutation_action(action: WorkActionKind) -> MutationAction {
    match action {
        WorkActionKind::Claim => MutationAction::Claim,
        WorkActionKind::Release => MutationAction::Release,
        WorkActionKind::Start => MutationAction::Start,
        WorkActionKind::Stop => MutationAction::Stop,
    }
}

fn cli_review_rows(action: MutationAction) -> impl Iterator<Item = String> {
    action
        .cli_only_help()
        .iter()
        .map(|row| format!("CLI-only: {row}"))
}

/// Build a form or a zero-input preview without writing.
pub fn prepare(
    context: &PlanContext,
    action: MutationAction,
    selected_task: Option<&str>,
) -> Result<PreparedMutation, String> {
    if action.needs_selected_task() && selected_task.is_none() {
        return Err(format!(
            "{} needs a selected task; select one in Tasks or Graph first",
            action.title()
        ));
    }
    if action.needs_selected_capture() && selected_task.is_none() {
        return Err(format!(
            "{} needs a selected capture; select one in Inbox first",
            action.title()
        ));
    }
    if let Some(workflow) = action.capture() {
        let form = crate::inbox::form(context, workflow, selected_task)?;
        return Ok(PreparedMutation::Form(Box::new(
            form.with_additional_guidance(
                action.cli_only_help().iter().map(|row| (*row).to_string()),
            ),
        )));
    }
    if let Some(workflow) = action.quick() {
        let task = selected_task.expect("selected task checked above");
        let catalogue = aplan_app::work_action_catalogue(context, task, None)
            .map_err(|error| error.to_string())?;
        let entry = catalogue.action(workflow.kind());
        if let WorkActionAvailability::Unavailable { reason } = &entry.availability {
            return Err(reason.clone());
        }
        if let Some(form) =
            crate::quick_actions::form(&catalogue, workflow).map_err(|error| error.to_string())?
        {
            return Ok(PreparedMutation::Form(Box::new(
                form.with_additional_guidance(
                    action.cli_only_help().iter().map(|row| (*row).to_string()),
                ),
            )));
        }
        let request = crate::quick_actions::request(&catalogue, workflow, None)
            .map_err(|error| error.to_string())?;
        let preview =
            aplan_app::preview_work_action(context, &request).map_err(|error| error.to_string())?;
        return Ok(PreparedMutation::Preview(MutationCandidate::Work {
            preview: Box::new(preview),
        }));
    }
    let form = if let Some(workflow) = action.planning() {
        crate::planning::form(context, workflow, selected_task)
            .map_err(|error| error.to_string())?
    } else if let Some(workflow) = action.authoring() {
        crate::authoring::form(context, workflow, selected_task)
            .map_err(|error| error.to_string())?
    } else if let Some(workflow) = action.evidence() {
        crate::evidence::form(context, workflow, selected_task)
            .map_err(|error| error.to_string())?
    } else {
        unreachable!("the exhaustive action has one workflow family")
    };
    Ok(PreparedMutation::Form(Box::new(
        form.with_additional_guidance(action.cli_only_help().iter().map(|row| (*row).to_string())),
    )))
}

/// Turn submitted fields into the immutable typed preview shown by the shell.
pub fn preview(
    context: &PlanContext,
    action: MutationAction,
    selected_task: Option<&str>,
    submission: &FormSubmission,
    verification: Option<&aplan_app::VerificationWorkbench>,
) -> Result<MutationCandidate, String> {
    preview_with_external_body(
        context,
        action,
        selected_task,
        submission,
        verification,
        None,
    )
}

/// Preview with the exact body returned by a live external-editor handoff.
/// Only Add accepts this value; every other authoring workflow remains closed
/// over its declared fields and captured source.
pub fn preview_with_external_body(
    context: &PlanContext,
    action: MutationAction,
    selected_task: Option<&str>,
    submission: &FormSubmission,
    verification: Option<&aplan_app::VerificationWorkbench>,
    edited_body: Option<&str>,
) -> Result<MutationCandidate, String> {
    if let Some(workflow) = action.capture() {
        if edited_body.is_some() {
            return Err("external editor content belongs only to Add task".to_string());
        }
        let preview = crate::inbox::preview(context, workflow, submission)?;
        // Capture workflows remain anchored to the durable intake record.
        // Promotion exposes its task through that record's promoted-to link;
        // treating a capture id as a task selection corrupts the task browser.
        let selection_after_commit = Some(preview.capture().id.clone());
        let mut review_rows = crate::inbox::review_rows(&preview);
        review_rows.extend(cli_review_rows(action));
        return Ok(MutationCandidate::Plan {
            action,
            change: Box::new(preview.into_change_set()),
            review_rows,
            selection_after_commit,
        });
    }
    if let Some(workflow) = action.quick() {
        let task =
            selected_task.ok_or_else(|| "the quick action lost its selected task".to_string())?;
        let catalogue = aplan_app::work_action_catalogue(context, task, None)
            .map_err(|error| error.to_string())?;
        let request = crate::quick_actions::request(&catalogue, workflow, Some(submission))
            .map_err(|error| error.to_string())?;
        let preview =
            aplan_app::preview_work_action(context, &request).map_err(|error| error.to_string())?;
        return Ok(MutationCandidate::Work {
            preview: Box::new(preview),
        });
    }
    if let Some(workflow) = action.planning() {
        let request =
            crate::planning::request(workflow, submission).map_err(|error| error.to_string())?;
        let preview =
            crate::planning::preview(context, &request).map_err(|error| error.to_string())?;
        let mut review_rows = crate::planning::review_rows(&preview);
        review_rows.extend(cli_review_rows(action));
        let selection_after_commit = preview.selection_after_commit().map(str::to_string);
        return Ok(MutationCandidate::Plan {
            action,
            change: Box::new(preview.into_change_set()),
            review_rows,
            selection_after_commit,
        });
    }
    if let Some(workflow) = action.authoring() {
        let assets = capture_authoring_assets(workflow, submission, edited_body)?;
        let request = crate::authoring::request(workflow, submission, assets)
            .map_err(|error| error.to_string())?;
        let preview = request
            .preview(context)
            .map_err(|error| error.to_string())?;
        let selection_after_commit = preview.created().first().cloned();
        return Ok(MutationCandidate::Plan {
            action,
            change: Box::new(preview.into_change_set()),
            review_rows: cli_review_rows(action).collect(),
            selection_after_commit,
        });
    }
    let workflow = action
        .evidence()
        .expect("the exhaustive action has one workflow family");
    let request =
        crate::evidence::request(workflow, submission).map_err(|error| error.to_string())?;
    let preview = if matches!(request, EvidenceRequest::VerifyFix) {
        let verification = verification.ok_or_else(|| {
            "run verification in this session before applying its repairs".to_string()
        })?;
        let config = context.report_config().cloned().unwrap_or_default();
        let options = aplan_app::configured_verification(
            &config,
            context.identity().configuration.path.clone(),
            context.options().traces_root.clone(),
        );
        crate::evidence::preview_verification_fix(&request, context, verification, &options)
            .map_err(|error| error.to_string())?
    } else {
        request
            .preview(context)
            .map_err(|error| error.to_string())?
    };
    Ok(MutationCandidate::Plan {
        action,
        change: Box::new(preview.into_change_set()),
        review_rows: cli_review_rows(action).collect(),
        selection_after_commit: submission.value("id").map(str::to_string),
    })
}

fn capture_authoring_assets(
    workflow: AuthoringWorkflow,
    submission: &FormSubmission,
    edited_body: Option<&str>,
) -> Result<AuthoringAssets, String> {
    if workflow == AuthoringWorkflow::Add {
        return Ok(AuthoringAssets {
            source: None,
            edited_body: edited_body.map(str::to_string),
        });
    }
    if edited_body.is_some() {
        return Err("an external body belongs only to the Add workflow".to_string());
    }
    if !matches!(
        workflow,
        AuthoringWorkflow::Manifest | AuthoringWorkflow::Import
    ) {
        return Ok(AuthoringAssets::default());
    }
    let source = submission
        .value("source")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "field `source` is required".to_string())?;
    let contents = read_captured_source(Path::new(source))
        .map_err(|error| format!("reading selected source `{source}`: {error}"))?;
    Ok(AuthoringAssets {
        source: Some(CapturedSource {
            name: source.to_string(),
            contents,
        }),
        edited_body: None,
    })
}

fn read_captured_source(path: &Path) -> Result<String, String> {
    // Inspect before opening so the ordinary FIFO/device case is refused
    // without letting `open` wait for a producer. The descriptor check below
    // remains authoritative because the path can change between these calls.
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(
            "the TUI captures only regular files; streams and devices stay CLI-only".to_string(),
        );
    }

    let (file, metadata) = open_captured_regular_file(path)?;
    if metadata.len() > MAX_CAPTURE_BYTES {
        return Err(format!(
            "the file is {} bytes, above the {MAX_CAPTURE_BYTES}-byte interactive safety bound",
            metadata.len()
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CAPTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CAPTURE_BYTES {
        return Err(format!(
            "the file grew above the {MAX_CAPTURE_BYTES}-byte interactive safety bound while it was captured"
        ));
    }
    String::from_utf8(bytes).map_err(|error| format!("the source is not UTF-8: {error}"))
}

fn open_captured_regular_file(path: &Path) -> Result<(std::fs::File, std::fs::Metadata), String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // If a regular file is replaced by a FIFO after the preflight, opening
        // the replacement must still be non-blocking. Descriptor metadata then
        // rejects it before any bytes are accepted.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(
            "the TUI captures only regular files; streams and devices stay CLI-only".to_string(),
        );
    }
    Ok((file, metadata))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs::OpenOptions;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::form::{FormAction, FormFocus, FormTransition};

    struct Fixture {
        _directory: tempfile::TempDir,
        repository: PathBuf,
        plan: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::Builder::new()
                .prefix("mutation-composition")
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("fixture");
            let repository = directory.path().to_path_buf();
            let plan = repository.join("aplan");
            std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
            std::fs::write(
                repository.join(".aplan.yml"),
                "root: aplan\ndetect_drift: true\n",
            )
            .expect("configuration");
            std::fs::write(repository.join("proof.md"), "reviewed proof\n").expect("proof");
            write_task(
                &plan,
                "T-1",
                "first",
                "todo",
                "blocked_by: []\ntags: [legacy]\npaths: [proof.md]\n",
            );
            write_task(
                &plan,
                "T-2",
                "second",
                "in_progress",
                "blocked_by: [T-1]\ntags: []\n",
            );
            write_task(
                &plan,
                "T-3",
                "third",
                "todo",
                "blocked_by: []\nprefer_after: [T-1]\ntags: []\n",
            );
            std::fs::write(
                plan.join("tasks/M-1.md"),
                "---\nid: M-1\ntitle: cut\nphase: 1\nstatus: todo\nblocked_by: [T-1, T-2, T-3]\ntags: [milestone, v1]\n---\n",
            )
            .expect("milestone");
            git(&repository, &["init", "-q"]);
            git(
                &repository,
                &["config", "user.email", "fixture@example.com"],
            );
            git(&repository, &["config", "user.name", "Fixture Actor"]);
            git(&repository, &["add", "."]);
            git(&repository, &["commit", "-qm", "fixture"]);
            Self {
                _directory: directory,
                repository,
                plan,
            }
        }

        fn context(&self) -> PlanContext {
            let config = aplan_config::load_config(&self.repository).expect("configuration");
            PlanContext::new(
                aplan_core::load_plan_identified(&self.plan),
                aplan_app::ContextInputs {
                    repository_root: self.repository.clone(),
                    configuration_file: Some(self.repository.join(".aplan.yml")),
                    trust: aplan_app::Trust::RepositoryRead,
                    options: aplan_core::Options {
                        detect_drift: true,
                        traces_root: Some(self.repository.clone()),
                        ..aplan_core::Options::default()
                    },
                    verdict: aplan_core::Verdict::default(),
                },
            )
            .with_captured_report_config(config)
            .expect("captured configuration")
        }
    }

    fn write_task(plan: &Path, id: &str, title: &str, status: &str, fields: &str) {
        std::fs::write(
            plan.join(format!("tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: {status}\neffort_days: 1\n{fields}---\n"
            ),
        )
        .expect("task");
    }

    fn git(root: &Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {arguments:?}: {output:?}");
    }

    fn replace_current(form: &mut MutationForm, values: &[&str]) {
        let FormFocus::Field(index) = form.focus() else {
            panic!("field value changed while a control was focused")
        };
        let current = form.fields()[index]
            .values()
            .next()
            .unwrap_or_default()
            .to_string();
        form.reduce(FormAction::CursorHome);
        for _ in current.chars() {
            form.reduce(FormAction::Delete);
        }
        if let Some(first) = values.first() {
            form.reduce(FormAction::Insert((*first).to_string()));
            for value in &values[1..] {
                form.reduce(FormAction::AddItem);
                form.reduce(FormAction::Insert((*value).to_string()));
            }
        }
    }

    fn submission(
        context: &PlanContext,
        action: MutationAction,
        selected: Option<&str>,
        values: &[(&str, &[&str])],
    ) -> FormSubmission {
        let PreparedMutation::Form(mut form) =
            prepare(context, action, selected).expect("mutation form")
        else {
            panic!("{action:?} unexpectedly prepared a zero-input preview")
        };
        for index in 0..form.fields().len() {
            assert_eq!(form.focus(), FormFocus::Field(index));
            let key = form.fields()[index].spec().key().to_string();
            if let Some((_, field_values)) = values.iter().find(|(candidate, _)| *candidate == key)
            {
                replace_current(&mut form, field_values);
            }
            form.reduce(FormAction::NextFocus);
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("valid {action:?} form did not request a preview")
        };
        submission
    }

    #[test]
    fn catalogue_is_the_exact_union_of_every_workflow_family() {
        let quick = MutationAction::ALL
            .into_iter()
            .filter_map(MutationAction::quick)
            .collect::<BTreeSet<_>>();
        assert_eq!(quick, QuickActionWorkflow::ALL.into_iter().collect());

        let planning = MutationAction::ALL
            .into_iter()
            .filter_map(MutationAction::planning)
            .collect::<BTreeSet<_>>();
        assert_eq!(planning, PlanningWorkflow::ALL.iter().copied().collect());

        let authoring = MutationAction::ALL
            .into_iter()
            .filter_map(MutationAction::authoring)
            .collect::<BTreeSet<_>>();
        assert_eq!(authoring, AuthoringWorkflow::ALL.iter().copied().collect());

        let evidence = MutationAction::ALL
            .into_iter()
            .filter_map(MutationAction::evidence)
            .collect::<BTreeSet<_>>();
        assert_eq!(evidence, EvidenceWorkflow::ALL.iter().copied().collect());

        let capture = MutationAction::ALL
            .into_iter()
            .filter_map(MutationAction::capture)
            .collect::<BTreeSet<_>>();
        assert_eq!(capture, CaptureWorkflow::ALL.into_iter().collect());

        assert_eq!(
            quick.len() + planning.len() + authoring.len() + evidence.len() + capture.len(),
            MutationAction::ALL.len(),
            "an action was assigned to zero or several workflow families"
        );
    }

    #[test]
    fn every_action_has_a_nonblank_unique_cli_semantic_path() {
        let mut titles = BTreeSet::new();
        for action in MutationAction::ALL {
            assert!(!action.title().trim().is_empty());
            assert!(!action.cli().trim().is_empty());
            assert!(titles.insert(action.title()), "duplicate title: {action:?}");
            let families = [
                action.quick().is_some(),
                action.planning().is_some(),
                action.authoring().is_some(),
                action.evidence().is_some(),
                action.capture().is_some(),
            ];
            assert_eq!(families.into_iter().filter(|member| *member).count(), 1);
            for boundary in action.cli_only_help() {
                assert!(!boundary.trim().is_empty(), "blank boundary for {action:?}");
            }
            if action.capture().is_some() {
                assert!(!action.needs_selected_task());
                assert_eq!(
                    action.needs_selected_capture(),
                    action != MutationAction::CaptureCreate
                );
            } else {
                assert!(!action.needs_selected_capture());
                assert_eq!(
                    action.needs_selected_task(),
                    !matches!(
                        action,
                        MutationAction::TagRename
                            | MutationAction::Move
                            | MutationAction::DropCuts
                            | MutationAction::Add
                            | MutationAction::Manifest
                            | MutationAction::Import
                            | MutationAction::VerifyFix
                    )
                );
            }
        }
    }

    #[test]
    fn verification_repair_form_renders_every_cli_only_mode_boundary() {
        let fixture = Fixture::new();
        let context = fixture.context();
        let PreparedMutation::Form(form) =
            prepare(&context, MutationAction::VerifyFix, None).expect("verification repair form")
        else {
            panic!("verification repair unexpectedly skipped its review form")
        };
        let rendered = form.render(240, 100).snapshot();
        for spelling in [
            "--cut",
            "--phase",
            "--tag",
            "--owner",
            "--unclaimed",
            "--status",
            "--no-require-effort",
            "--traces-root",
            "--no-traces",
            "--deny-warnings",
            "--no-deny-warnings",
            "--test-receipts",
            "--refresh-test-receipts",
            "--refresh-base",
            "--refresh-head",
            "--refresh-working-tree",
            "--refresh-shadow",
        ] {
            assert!(
                rendered.contains(spelling),
                "verification repair hid `{spelling}` from its rendered boundary guidance:\n{rendered}"
            );
        }
    }

    #[test]
    fn transfer_tokens_are_debuggable_equal_and_strictly_single_consumption() {
        let transfer = MutationTransfer::new(MutationPayload::PreviewFailed {
            action: MutationAction::Add,
            request: 7,
            message: "refused".to_string(),
        });
        let alias = transfer.clone();
        let unrelated = MutationTransfer::new(MutationPayload::PreviewFailed {
            action: MutationAction::Add,
            request: 8,
            message: "other".to_string(),
        });
        assert_eq!(transfer, alias);
        assert_ne!(transfer, unrelated);
        assert!(format!("{transfer:?}").contains("available: true"));
        let Some(MutationPayload::PreviewFailed {
            action,
            request,
            message,
        }) = alias.take()
        else {
            panic!("typed payload was not retained")
        };
        assert_eq!(
            (action, request, message.as_str()),
            (MutationAction::Add, 7, "refused")
        );
        assert!(
            transfer.take().is_none(),
            "a clone replayed a consumed payload"
        );
        assert!(format!("{transfer:?}").contains("available: false"));
    }

    #[test]
    #[allow(clippy::too_many_lines)] // The closed action union is audited in one table-driven journey.
    fn composition_prepares_every_family_and_previews_representative_typed_requests() {
        let fixture = Fixture::new();
        let context = fixture.context();

        for action in MutationAction::ALL {
            if action.needs_selected_task() {
                let error = prepare(&context, action, None).expect_err("selection is required");
                assert!(error.contains("selected task"), "{action:?}: {error}");
            }
        }

        for action in MutationAction::ALL {
            if action.quick().is_none() && action.capture().is_none() {
                let selected = action.needs_selected_task().then_some("T-1");
                assert!(
                    matches!(
                        prepare(&context, action, selected),
                        Ok(PreparedMutation::Form(_))
                    ),
                    "{action:?} did not prepare its typed form"
                );
            }
        }

        assert!(matches!(
            prepare(&context, MutationAction::Claim, Some("T-1")),
            Ok(PreparedMutation::Form(_))
        ));
        let claim = submission(&context, MutationAction::Claim, Some("T-1"), &[]);
        let MutationCandidate::Work { preview: claim, .. } =
            preview(&context, MutationAction::Claim, Some("T-1"), &claim, None)
                .expect("claim preview")
        else {
            panic!("claim did not retain its coordination preview")
        };
        assert!(matches!(
            aplan_app::commit_work_action(&context, *claim),
            aplan_app::WorkActionCommitOutcome::Coordination(
                aplan_app::CoordinationOutcome::Applied(_)
            )
        ));
        assert!(matches!(
            prepare(&context, MutationAction::Release, Some("T-1")),
            Ok(PreparedMutation::Preview(MutationCandidate::Work { .. }))
        ));
        assert!(matches!(
            prepare(&context, MutationAction::Start, Some("T-1")),
            Ok(PreparedMutation::Preview(MutationCandidate::Work { .. }))
        ));
        assert!(matches!(
            prepare(&context, MutationAction::Stop, Some("T-2")),
            Ok(PreparedMutation::Preview(MutationCandidate::Work { .. }))
        ));

        let planning = [
            (
                MutationAction::DependencyAdd,
                Some("T-3"),
                vec![("after", &["T-2"][..])],
            ),
            (
                MutationAction::DependencyRemove,
                Some("T-2"),
                vec![("after", &["T-1"][..])],
            ),
            (
                MutationAction::PreferenceAdd,
                Some("T-2"),
                vec![("after", &["T-3"][..])],
            ),
            (
                MutationAction::PreferenceRemove,
                Some("T-3"),
                vec![("after", &["T-1"][..])],
            ),
            (
                MutationAction::TagAdd,
                Some("T-1"),
                vec![("tag", &["new-tag"][..])],
            ),
            (
                MutationAction::TagRemove,
                Some("T-1"),
                vec![("tag", &["legacy"][..])],
            ),
            (
                MutationAction::TagRename,
                None,
                vec![("from", &["legacy"][..]), ("to", &["renamed"][..])],
            ),
            (
                MutationAction::Move,
                None,
                vec![("tasks", &["T-1"][..]), ("phase", &["2"][..])],
            ),
            (MutationAction::DropTasks, Some("T-1"), Vec::new()),
            (MutationAction::DropCuts, None, vec![("cuts", &["M-1"][..])]),
        ];
        for (action, selected, values) in planning {
            let submission = submission(&context, action, selected, &values);
            let candidate = preview(&context, action, selected, &submission, None)
                .unwrap_or_else(|error| panic!("{action:?}: {error}"));
            assert_eq!(candidate.action(), action);
            assert!(matches!(candidate, MutationCandidate::Plan { .. }));
        }

        let add = submission(
            &context,
            MutationAction::Add,
            None,
            &[
                ("title", &["new work"]),
                ("id", &["T-9"]),
                ("phase", &["1"]),
            ],
        );
        assert!(matches!(
            preview(&context, MutationAction::Add, None, &add, None),
            Ok(MutationCandidate::Plan { .. })
        ));

        let close = submission(
            &context,
            MutationAction::Close,
            Some("T-1"),
            &[("file", &["proof.md"])],
        );
        assert!(matches!(
            preview(&context, MutationAction::Close, Some("T-1"), &close, None),
            Ok(MutationCandidate::Plan { .. })
        ));

        let verify_fix = submission(
            &context,
            MutationAction::VerifyFix,
            None,
            &[("review", &["review"])],
        );
        let error = preview(&context, MutationAction::VerifyFix, None, &verify_fix, None)
            .expect_err("repair needs an identified verification run");
        assert!(error.contains("run verification"), "{error}");

        let manifest = submission(
            &context,
            MutationAction::Manifest,
            None,
            &[("source", &["missing.yml"])],
        );
        let error = preview(&context, MutationAction::Manifest, None, &manifest, None)
            .expect_err("missing captured source");
        assert!(error.contains("reading selected source"), "{error}");
    }

    #[test]
    fn captured_sources_are_regular_bounded_utf8_snapshots() {
        let dir = tempfile::Builder::new()
            .prefix("mutation-source")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let source = dir.path().join("source.md");
        std::fs::write(&source, "captured\nbytes\n").expect("source");
        assert_eq!(
            read_captured_source(&source).expect("regular UTF-8 source"),
            "captured\nbytes\n"
        );

        let invalid = dir.path().join("invalid.md");
        std::fs::write(&invalid, [0xff, 0xfe]).expect("invalid source");
        assert!(
            read_captured_source(&invalid)
                .expect_err("invalid UTF-8 refused")
                .contains("not UTF-8")
        );
        assert!(
            read_captured_source(dir.path())
                .expect_err("directories refused")
                .contains("regular files")
        );

        let oversized = dir.path().join("oversized.md");
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&oversized)
            .expect("oversized source")
            .set_len(MAX_CAPTURE_BYTES + 1)
            .expect("sparse source length");
        let error = read_captured_source(&oversized).expect_err("oversized source refused");
        assert!(error.contains("interactive safety bound"), "{error}");
        assert!(
            read_captured_source(&dir.path().join("missing.md"))
                .expect_err("missing source refused")
                .contains("No such file")
        );
    }

    /// Proof for `tui.authoring_capture_refuses_non_regular_before_blocking`:
    /// a FIFO with no writer is rejected before its read-only open can wait.
    #[cfg(unix)]
    #[test]
    fn captured_sources_refuse_non_regular_files_before_open_can_block() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::Builder::new()
            .prefix("mutation-fifo")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let fifo = dir.path().join("source.fifo");
        let created = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo");
        assert!(created.success(), "mkfifo failed: {created}");

        for (case, capture) in [
            (
                "pre-open type check",
                read_captured_source as fn(&Path) -> Result<String, String>,
            ),
            (
                "non-blocking descriptor check after a path race",
                |path: &Path| open_captured_regular_file(path).map(|_| String::new()),
            ),
        ] {
            let (send, receive) = mpsc::sync_channel(1);
            let source = fifo.clone();
            let reader = std::thread::spawn(move || {
                let _ = send.send(capture(&source));
            });
            let result = receive
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or_else(|_| panic!("{case} blocked waiting for a FIFO writer"));
            let error = result.expect_err("FIFO must not be captured");
            assert!(error.contains("regular files"), "{case}: {error}");
            reader.join().expect("capture worker");
        }
    }
}
