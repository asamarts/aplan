use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use aplan_app::{
    BuildIdentity, CapabilityView, CellScene, CellSceneJob, CellSceneOrientation, CellSceneRequest,
    CellSceneResult, CheckOverrides, DiagnosticRun, DiagnosticScope, DiagnosticWorkbench,
    GraphContinuation, GraphContinuationDirection, GraphExplorerIdentity, GraphExplorerLocation,
    GraphExplorerProjection, GraphExplorerRequest, GraphExplorerSelectionOrigin, GraphProjection,
    GraphRequest, GraphScope, GraphWindowProjection, GraphWindowRequest, GraphWindowState,
    GraphWorkspace, Onboarding, PlanContext, PlanOverview, ReportKind, ReportProjection,
    ReportRequest, ReportRowProvenance, ReportWorkspace, Selector, TaskBrowserMatch,
    TaskBrowserRequest, TaskBrowserResponse, TaskFilter, TaskSort, WarningPolicyOverride,
    WhyTraversal, WorkDiscoveryRequest, WorkDiscoveryWorkspace,
};
use aplan_output::graph_scene::{GraphScene, SceneHit, ScenePoint};
use aplan_output::lanes::TooWide;

use crate::form::{DomainIssue, FormAction, FormTransition, MutationForm, RenderedForm};
use crate::graph_navigation::{self, GraphOrdering, SpatialDirection};
use crate::graph_widget::{
    FALLBACK_MESSAGE, GraphDensity, GraphEdgePresentation, ResolvedGraphDensity, card_size,
    fallback_message, fallback_node_width, resolve_density,
};
use crate::mutations::{
    MutationAction, MutationCandidate, MutationCommitResult, MutationEffect, MutationPayload,
    MutationReload, MutationTransfer, PreparedMutation,
};
use crate::text;

fn layered_scene_request(
    projection: GraphExplorerProjection,
    density: ResolvedGraphDensity,
    orientation: CellSceneOrientation,
    ordering: GraphOrdering,
    ordering_hints: BTreeMap<GraphExplorerIdentity, i64>,
    prior: Option<Box<CellScene>>,
) -> CellSceneRequest {
    let mut request = CellSceneRequest::new(projection);
    request.card_sizes = request
        .projection
        .graph
        .nodes
        .iter()
        .map(|node| (node.identity.clone(), card_size(node, density)))
        .collect();
    request.prior = prior;
    request.orientation = orientation;
    request.peer_order = ordering.cell_order();
    request.ordering_hints = ordering_hints;
    request
}

fn cycle_after<T: Clone + PartialEq>(candidates: &[T], selected: Option<&T>) -> Option<T> {
    let next = selected
        .and_then(|selected| {
            candidates
                .iter()
                .position(|candidate| candidate == selected)
        })
        .map_or(0, |index| (index + 1) % candidates.len().max(1));
    candidates.get(next).cloned()
}

pub(crate) const MAX_PENDING_EFFECTS: usize = 2;
const MAX_EFFECT_RECORDS: usize = 16;
const NOTICE_TICKS: u8 = 4;
const MAX_PALETTE_QUERY_CELLS: usize = 64;
const MAX_PALETTE_QUERY_GRAPHEMES: usize = 128;
const MAX_PALETTE_QUERY_BYTES: usize = 512;
const MAX_TASK_QUERY_CELLS: usize = 160;
const MAX_TASK_QUERY_GRAPHEMES: usize = 256;
const MAX_TASK_QUERY_BYTES: usize = 1_024;
const MAX_GRAPH_EXPLORER_HISTORY: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Route {
    Overview,
    Tasks,
    Diagnostics,
    Graph,
    Reports,
    Inbox,
}

impl Route {
    pub(crate) const ALL: [Self; 6] = [
        Self::Overview,
        Self::Tasks,
        Self::Diagnostics,
        Self::Graph,
        Self::Reports,
        Self::Inbox,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Tasks => "Tasks",
            Self::Inbox => "Inbox",
            Self::Diagnostics => "Diagnostics",
            Self::Graph => "Graph",
            Self::Reports => "Reports",
        }
    }

    pub(crate) const fn number(self) -> u8 {
        match self {
            Self::Overview => 1,
            Self::Tasks => 2,
            Self::Diagnostics => 3,
            Self::Graph => 4,
            Self::Reports => 5,
            Self::Inbox => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Navigation,
    Primary,
    Detail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InboxFilterChoice {
    State(Option<aplan_core::capture::CaptureState>),
    Revisit(aplan_core::capture::RevisitFilter),
    Kind(Option<String>),
    Label(Option<String>),
}

impl InboxFilterChoice {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::State(state) => format!("State: {}", state.map_or("all", capture_state_name)),
            Self::Revisit(filter) => format!(
                "Review: {}",
                match filter {
                    aplan_core::capture::RevisitFilter::Actionable => "actionable",
                    aplan_core::capture::RevisitFilter::Deferred => "deferred",
                    aplan_core::capture::RevisitFilter::All => "all",
                }
            ),
            Self::Kind(kind) => format!("Kind: {}", kind.as_deref().unwrap_or("all")),
            Self::Label(label) => format!("Label: {}", label.as_deref().unwrap_or("all")),
        }
    }
}

impl Focus {
    pub(crate) const ALL: [Self; 3] = [Self::Navigation, Self::Primary, Self::Detail];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Modal {
    About,
    Help,
    Palette,
    Search,
    ReportDate,
    Filters,
    RunSheet,
    VerifyRunSheet,
    Verification,
    RuleSearch,
    RuleExplanation,
    WorkDate,
    TestCatalogue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TestCatalogueLayer {
    Loading,
    Ready(Box<crate::evidence::TestCatalogueView>),
    Failed(String),
}

#[derive(Debug)]
pub(crate) enum MutationState {
    Loading,
    Form(Box<MutationForm>),
    QuickReview {
        candidate: Option<Box<MutationCandidate>>,
        form: Option<Box<MutationForm>>,
        rows: Vec<String>,
    },
    Result {
        form: Option<Box<MutationForm>>,
        rows: Vec<String>,
    },
}

#[derive(Debug)]
pub(crate) struct MutationLayer {
    pub(crate) action: MutationAction,
    pub(crate) selected_task: Option<String>,
    pub(crate) state: MutationState,
    edited_body: Option<String>,
    pub(crate) committing: bool,
    plain_scroll: usize,
    selection_after_commit: Option<String>,
}

impl MutationLayer {
    pub(crate) fn rows(&self, width: usize, height: usize) -> Vec<String> {
        let source = if self.committing {
            vec![
                format!("Publishing reviewed {}...", self.action.title()),
                "The guarded write is running; this layer cannot be dismissed yet.".to_string(),
                "Quit remains graceful and waits for the repository operation.".to_string(),
            ]
        } else {
            match &self.state {
                MutationState::Loading => vec![
                    format!("Preparing {}...", self.action.title()),
                    "No write occurs while this preview is prepared.".to_string(),
                    "Esc cancels this layer; queued work is discarded when possible.".to_string(),
                ],
                MutationState::Form(form) => return form.render(width, height).rows,
                MutationState::QuickReview { rows, .. } | MutationState::Result { rows, .. } => {
                    let mut rows = rows.clone();
                    if matches!(&self.state, MutationState::QuickReview { .. }) {
                        rows.push(
                            "Controls: PgUp/PgDn Scroll  Esc Cancel  Enter Confirm".to_string(),
                        );
                    } else {
                        rows.push("Controls: PgUp/PgDn Scroll  Esc Close".to_string());
                    }
                    rows
                }
            }
        };
        RenderedForm::window(&source, width, height, self.plain_scroll).rows
    }

    fn scroll_plain(&mut self, delta: i8) {
        if matches!(
            self.state,
            MutationState::QuickReview { .. } | MutationState::Result { .. }
        ) {
            self.plain_scroll = if delta.is_negative() {
                self.plain_scroll
                    .saturating_sub(usize::from(delta.unsigned_abs()))
            } else {
                self.plain_scroll
                    .saturating_add(usize::from(delta.unsigned_abs()))
            };
        }
    }
}

fn work_commit_rows(outcome: &aplan_app::WorkActionCommitOutcome, reloaded: bool) -> Vec<String> {
    use aplan_app::{CoordinationOutcome, WorkActionCommitOutcome};

    let (preview, verdict, detail) = match outcome {
        WorkActionCommitOutcome::Plan { outcome, .. } => {
            let (verdict, detail) = match outcome.as_ref() {
                aplan_app::CommitOutcome::Applied(_) if reloaded => (
                    "COMMITTED",
                    "A fresh snapshot contains the reviewed status bytes.".to_string(),
                ),
                aplan_app::CommitOutcome::Applied(_) => (
                    "RELOAD REQUIRED",
                    "Publication returned, but no fresh snapshot has reconciled it.".to_string(),
                ),
                aplan_app::CommitOutcome::NoOp(_) => (
                    "NOT PUBLISHED",
                    "The reviewed status already existed.".to_string(),
                ),
                aplan_app::CommitOutcome::Denied(_) => (
                    "DENIED",
                    "The effective post-state policy denied publication.".to_string(),
                ),
                aplan_app::CommitOutcome::Stale { actual, .. } => (
                    "STALE",
                    format!(
                        "Identified input changed before publication: {}",
                        actual.sha256
                    ),
                ),
                aplan_app::CommitOutcome::PublicationFailed { error, report, .. } => (
                    "RECOVERY REQUIRED",
                    format!(
                        "Publication failed ({error}); observed state: {:?}",
                        report.state()
                    ),
                ),
            };
            (None, verdict, detail)
        }
        WorkActionCommitOutcome::Coordination(result) => match result {
            CoordinationOutcome::Applied(preview) => (
                Some(preview),
                if reloaded {
                    "COMMITTED"
                } else {
                    "RELOAD REQUIRED"
                },
                "The complete reviewed claim record was published.".to_string(),
            ),
            CoordinationOutcome::NoOp(preview) => (
                Some(preview),
                "NOT PUBLISHED",
                "The reviewed claim state already existed.".to_string(),
            ),
            CoordinationOutcome::Stale { preview, actual } => (
                Some(preview),
                "STALE",
                format!("Claim state changed before confirmation: {actual:?}"),
            ),
            CoordinationOutcome::Failed { preview, error } => (
                Some(preview),
                "FAILED",
                format!("Claim publication failed: {error}"),
            ),
        },
        WorkActionCommitOutcome::StaleInput { preview, actual } => (
            Some(preview),
            "STALE",
            format!(
                "Identified input changed before confirmation: {}",
                actual.sha256
            ),
        ),
    };
    let mut rows = vec![format!("Verdict: {verdict}")];
    if let Some(preview) = preview {
        rows.extend(crate::quick_actions::review_rows(preview));
    }
    rows.push(detail);
    rows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Theme {
    Dark,
    Light,
    LowColor,
    Monochrome,
}

impl Theme {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 4] = [Self::Dark, Self::Light, Self::LowColor, Self::Monochrome];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::LowColor => "low-color",
            Self::Monochrome => "monochrome",
        }
    }

    const fn next(self, capability: ColorCapability) -> Self {
        match capability {
            ColorCapability::None => Self::Monochrome,
            ColorCapability::Basic => match self {
                Self::LowColor => Self::Monochrome,
                _ => Self::LowColor,
            },
            ColorCapability::Extended => match self {
                Self::Dark => Self::Light,
                Self::Light => Self::LowColor,
                Self::LowColor => Self::Monochrome,
                Self::Monochrome => Self::Dark,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorCapability {
    None,
    Basic,
    Extended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Appearance {
    pub(crate) theme: Theme,
    pub(crate) glyphs: Glyphs,
    pub(crate) color: ColorCapability,
    pub(crate) reduced_motion: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Motion {
    Animated,
    Reduced,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            glyphs: Glyphs::Unicode,
            color: ColorCapability::Extended,
            reduced_motion: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Glyphs {
    Unicode,
    Ascii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationMode {
    ResolveOnly,
    Execute,
}

impl VerificationMode {
    pub(crate) const fn execute(self) -> bool {
        matches!(self, Self::Execute)
    }

    const fn toggled(self) -> Self {
        match self {
            Self::ResolveOnly => Self::Execute,
            Self::Execute => Self::ResolveOnly,
        }
    }
}

impl Glyphs {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 2] = [Self::Unicode, Self::Ascii];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Unicode => "unicode",
            Self::Ascii => "ASCII",
        }
    }
}

/// One complete, identified projection that is safe to keep on screen while a
/// later operation runs or fails.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) build: BuildIdentity,
    pub(crate) overview: PlanOverview,
    pub(crate) task_rows: Vec<aplan_app::TaskBrowserRow>,
    pub(crate) inspectors: Vec<aplan_app::Show>,
    pub(crate) diagnostics: DiagnosticWorkbench,
    pub(crate) verification_sheet: aplan_app::VerificationRunSheet,
    pub(crate) graph: GraphWorkspace,
    pub(crate) reports: ReportWorkspace,
    pub(crate) work: WorkDiscoveryWorkspace,
    pub(crate) inbox: Result<aplan_app::CaptureWorkspace, String>,
}

fn onboarding_input(state: &Onboarding) -> aplan_app::InputIdentity {
    let configuration_path = match &state.configuration {
        aplan_app::ConfigurationSelection::Discover { found, .. } => found.clone(),
        aplan_app::ConfigurationSelection::Explicit { path } => Some(path.clone()),
        aplan_app::ConfigurationSelection::None => None,
    };
    let configuration_content = if configuration_path.is_some() {
        aplan_app::ConfigurationContentIdentity::NotObserved
    } else {
        aplan_app::ConfigurationContentIdentity::Absent
    };
    aplan_app::InputIdentity {
        source: "onboarding",
        plan_root: state.plan_root.clone(),
        plan_sha256: String::new(),
        repository_root: state
            .plan_root
            .parent()
            .unwrap_or(&state.plan_root)
            .to_path_buf(),
        repository: aplan_app::RepositoryIdentity::NotObserved {
            reason: "onboarding performs no repository observation".to_string(),
        },
        configuration: aplan_app::ConfigurationIdentity {
            path: configuration_path,
            sha256: String::new(),
            content: configuration_content,
        },
        trust: aplan_app::Trust::Hermetic,
        sha256: "onboarding".to_string(),
    }
}

impl Snapshot {
    pub(crate) fn from_context(
        context: &PlanContext,
        build: &BuildIdentity,
        verification_sheet: aplan_app::VerificationRunSheet,
    ) -> Self {
        Self::from_context_with_overrides(
            context,
            build,
            &CheckOverrides::default(),
            verification_sheet,
        )
    }

    pub(crate) fn from_context_with_overrides(
        context: &PlanContext,
        build: &BuildIdentity,
        overrides: &CheckOverrides,
        verification_sheet: aplan_app::VerificationRunSheet,
    ) -> Self {
        let response = aplan_app::task_view_with_overrides(context, overrides);
        Self::from_task_view(context, build, verification_sheet, response)
    }

    pub(crate) fn from_context_observed(
        context: &PlanContext,
        build: &BuildIdentity,
        verification_sheet: aplan_app::VerificationRunSheet,
        progress: &mut aplan_app::ProgressEmitter<'_>,
        cancellation: &aplan_app::CancellationToken,
    ) -> Option<Self> {
        if cancellation.is_cancelled() {
            return None;
        }
        progress.phase_started(
            aplan_app::ProgressPhase::StructuralValidation,
            None,
            aplan_app::ProgressMeasure::Indeterminate,
        );
        let response = aplan_app::task_view(context);
        if cancellation.is_cancelled() {
            return None;
        }
        progress.phase_completed(
            aplan_app::ProgressPhase::StructuralValidation,
            None,
            aplan_app::ProgressMeasure::Indeterminate,
        );
        progress.phase_started(
            aplan_app::ProgressPhase::Projection,
            None,
            aplan_app::ProgressMeasure::Indeterminate,
        );
        let snapshot = Self::from_task_view(context, build, verification_sheet, response);
        if cancellation.is_cancelled() {
            return None;
        }
        progress.phase_completed(
            aplan_app::ProgressPhase::Projection,
            None,
            aplan_app::ProgressMeasure::Indeterminate,
        );
        Some(snapshot)
    }

    fn from_task_view(
        context: &PlanContext,
        build: &BuildIdentity,
        verification_sheet: aplan_app::VerificationRunSheet,
        response: aplan_app::TaskView,
    ) -> Self {
        let graph = aplan_app::graph_workspace(context, response.catalogue.clone());
        let reports = aplan_app::report_workspace(context, response.catalogue.clone());
        let work = WorkDiscoveryWorkspace::from_context(context);
        let inbox = aplan_app::capture_workspace(context, &aplan_core::capture::SystemClock)
            .map_err(|error| error.to_string());
        Self {
            build: build.clone(),
            overview: response.overview,
            task_rows: response.catalogue,
            inspectors: response.inspectors,
            diagnostics: response.diagnostics,
            verification_sheet,
            graph,
            reports,
            work,
            inbox,
        }
    }

    /// Build inert storage for the reducer while onboarding owns the screen.
    ///
    /// No plan claim is made from this value: rendering and commands branch on
    /// `Model::onboarding` before consulting it. Keeping the ordinary reducer's
    /// complete snapshot invariant avoids teaching every loaded-plan operation
    /// about a partial plan while the onboarding gate remains explicit.
    pub(crate) fn onboarding(build: &BuildIdentity, state: &Onboarding) -> Self {
        let input = onboarding_input(state);
        let policy = aplan_app::CheckPolicySummary {
            require_effort: true,
            traces_root: None,
            traces_repository: None,
            traces_external: Vec::new(),
            detect_drift: false,
            deny_warnings: false,
            advisory: Vec::new(),
        };
        let unavailable = |scope, label: &str| DiagnosticRun {
            scope,
            coverage: aplan_app::DiagnosticCoverage::Unavailable(
                "no plan is loaded during onboarding".to_string(),
            ),
            findings: Vec::new(),
            errors: 0,
            warnings: 0,
            blocking: 0,
            advisory: 0,
            denied: false,
            input_label: label.to_string(),
        };
        let diagnostics = DiagnosticWorkbench {
            check: unavailable(DiagnosticScope::Check, "no loaded plan"),
            documents: unavailable(DiagnosticScope::CheckDocs, "no loaded documents"),
            run_sheet: aplan_app::CheckRunSheet {
                repository: policy.clone(),
                effective: policy.clone(),
                overrides: CheckOverrides::default(),
                relaxed: false,
            },
        };
        let overview = onboarding_overview(input.clone());
        let graph = GraphWorkspace::new(input.clone(), Vec::new(), Vec::new());
        let reports = ReportWorkspace::without_repository(
            input.clone(),
            Vec::new(),
            Vec::new(),
            "no plan is loaded during onboarding",
        );
        let work = WorkDiscoveryWorkspace::new(
            input.clone(),
            Vec::new(),
            std::collections::BTreeMap::default(),
        );
        Self {
            build: build.clone(),
            overview,
            task_rows: Vec::new(),
            inspectors: Vec::new(),
            diagnostics,
            verification_sheet: aplan_app::VerificationRunSheet {
                input_sha256: input.sha256.clone(),
                repository: input.repository_root.clone(),
                configuration: input.configuration.path.clone(),
                policy,
                trust: aplan_app::Trust::Hermetic,
                providers: aplan_app::VerificationProviders {
                    test_runners: 0,
                    url_check: aplan_app::Availability::Unavailable,
                    version_current: aplan_app::Availability::Unavailable,
                    version_check: aplan_app::Availability::Unavailable,
                },
                execute: aplan_app::Availability::Unavailable,
                fix: aplan_app::Availability::Unavailable,
            },
            graph,
            reports,
            work,
            inbox: Err("no plan is loaded during onboarding".to_string()),
        }
    }

    /// Inert storage used only until the identified launch replaces it.
    ///
    /// Rendering branches on `Model::startup` before consulting this value.
    /// Reusing the onboarding invariant keeps every ordinary loaded operation
    /// defined without allowing the placeholder to masquerade as plan data.
    pub(crate) fn loading(build: &BuildIdentity) -> Self {
        Self::onboarding(
            build,
            &Onboarding {
                cause: aplan_app::OnboardingCause::MissingPlan,
                plan_root: PathBuf::new(),
                root_selection: aplan_app::RootSelection::Default,
                configuration: aplan_app::ConfigurationSelection::None,
                detail: "startup has not selected plan inputs".to_string(),
                guidance: Vec::new(),
                commands: Vec::new(),
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn test(
        input: &str,
        root: &str,
        task_rows: Vec<aplan_app::TaskBrowserRow>,
        inspectors: Vec<aplan_app::Show>,
        errors: usize,
        warnings: usize,
    ) -> Self {
        let task_count = task_rows.len();
        let (findings, diagnostics) = synthetic_diagnostics(task_count, errors, warnings);
        let input_identity = synthetic_input_identity(input, root);
        let live_task_ids = task_rows.iter().map(|row| row.id.clone()).collect();
        let ready = task_rows
            .iter()
            .filter(|row| row.ready_rank.is_some())
            .map(|row| aplan_app::OverviewTask {
                id: row.id.clone(),
                title: row.title.clone(),
            })
            .collect();
        let (graph, reports, work) = synthetic_workspaces(&input_identity, &task_rows);
        let inbox = Ok(synthetic_capture_workspace(&input_identity));
        Self {
            build: BuildIdentity {
                version: "0.test".to_string(),
                source: "test-source".to_string(),
                facts_sha256: "facts".to_string(),
                binary_sha256: "binary".to_string(),
            },
            overview: PlanOverview {
                input: input_identity,
                load_state: if task_count == 0 {
                    aplan_app::OverviewLoadState::Empty
                } else {
                    aplan_app::OverviewLoadState::Complete
                },
                work_state: if task_count == 0 {
                    aplan_app::OverviewWorkState::NoLoadedWork
                } else {
                    aplan_app::OverviewWorkState::Active
                },
                effort_state: if task_count == 0 {
                    aplan_app::OverviewEffortState::NoLiveWork
                } else {
                    aplan_app::OverviewEffortState::Complete
                },
                loaded_tasks: task_count,
                loaded_work: task_count,
                live_work: task_count,
                done_work: 0,
                dropped_work: 0,
                live_task_ids,
                live_effort_days: f64::from(
                    u32::try_from(task_count).expect("test task count fits u32"),
                ),
                done_effort_days: 0.0,
                missing_effort_task_ids: Vec::new(),
                cuts: Vec::new(),
                cut_task_ids: Vec::new(),
                cut_work: 0,
                done_cut_work: 0,
                ready,
                as_of_epoch_seconds: 0,
                due: Vec::new(),
                waiting_revisits: Vec::new(),
                critical_path: Some((0.0, Vec::new())),
                findings,
                errors,
                warnings,
                denied: errors > 0,
                unloadable: Vec::new(),
            },
            task_rows,
            inspectors,
            diagnostics,
            verification_sheet: aplan_app::VerificationRunSheet {
                input_sha256: input.to_string(),
                repository: root.into(),
                configuration: None,
                policy: aplan_app::CheckPolicySummary {
                    require_effort: true,
                    traces_root: None,
                    traces_repository: None,
                    traces_external: Vec::new(),
                    detect_drift: false,
                    deny_warnings: false,
                    advisory: Vec::new(),
                },
                trust: aplan_app::Trust::ConfiguredProcesses,
                providers: aplan_app::VerificationProviders {
                    test_runners: 1,
                    url_check: aplan_app::Availability::Unavailable,
                    version_current: aplan_app::Availability::Unavailable,
                    version_check: aplan_app::Availability::Unavailable,
                },
                execute: aplan_app::Availability::Available,
                fix: aplan_app::Availability::Unavailable,
            },
            graph,
            reports,
            work,
            inbox,
        }
    }
}

fn onboarding_overview(input: aplan_app::InputIdentity) -> PlanOverview {
    PlanOverview {
        input,
        load_state: aplan_app::OverviewLoadState::Empty,
        work_state: aplan_app::OverviewWorkState::NoLoadedWork,
        effort_state: aplan_app::OverviewEffortState::NoLiveWork,
        loaded_tasks: 0,
        loaded_work: 0,
        live_work: 0,
        done_work: 0,
        dropped_work: 0,
        live_task_ids: Vec::new(),
        live_effort_days: 0.0,
        done_effort_days: 0.0,
        missing_effort_task_ids: Vec::new(),
        cuts: Vec::new(),
        cut_task_ids: Vec::new(),
        cut_work: 0,
        done_cut_work: 0,
        ready: Vec::new(),
        as_of_epoch_seconds: 0,
        due: Vec::new(),
        waiting_revisits: Vec::new(),
        critical_path: Some((0.0, Vec::new())),
        findings: Vec::new(),
        errors: 0,
        warnings: 0,
        denied: false,
        unloadable: Vec::new(),
    }
}

#[cfg(test)]
fn synthetic_input_identity(input: &str, root: &str) -> aplan_app::InputIdentity {
    aplan_app::InputIdentity {
        source: "working_tree",
        plan_root: root.into(),
        plan_sha256: format!("plan-{input}"),
        repository_root: root.into(),
        repository: aplan_app::RepositoryIdentity::NotObserved {
            reason: "synthetic snapshot".to_string(),
        },
        configuration: aplan_app::ConfigurationIdentity {
            path: None,
            sha256: format!("config-{input}"),
            content: aplan_app::ConfigurationContentIdentity::Absent,
        },
        trust: aplan_app::Trust::Hermetic,
        sha256: input.to_string(),
    }
}

#[cfg(test)]
fn synthetic_tasks(rows: &[aplan_app::TaskBrowserRow]) -> Vec<aplan_core::task::Task> {
    rows.iter()
        .map(|row| aplan_core::task::Task {
            id: row.id.clone(),
            title: row.title.clone(),
            phase: row.phase,
            status: row.status,
            owner: row.owner.clone(),
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: row.tags.clone(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        })
        .collect()
}

#[cfg(test)]
fn synthetic_capture_workspace(input: &aplan_app::InputIdentity) -> aplan_app::CaptureWorkspace {
    aplan_app::CaptureWorkspace {
        input: input.clone(),
        inbox_sha256: "test-inbox".to_string(),
        evaluated_at: "1970-01-01T00:00:00Z".to_string(),
        deferred_open: 0,
        captures: Vec::new(),
        total_open: 0,
        total_promoted: 0,
        total_discarded: 0,
        kinds: vec!["idea".to_string(), "note".to_string(), "task".to_string()],
        profiles: Vec::new(),
    }
}

#[cfg(test)]
fn synthetic_workspaces(
    input: &aplan_app::InputIdentity,
    rows: &[aplan_app::TaskBrowserRow],
) -> (GraphWorkspace, ReportWorkspace, WorkDiscoveryWorkspace) {
    let tasks = synthetic_tasks(rows);
    (
        GraphWorkspace::new(input.clone(), tasks.clone(), rows.to_vec()),
        ReportWorkspace::without_repository(
            input.clone(),
            tasks.clone(),
            rows.to_vec(),
            "synthetic snapshot has no repository observations",
        ),
        WorkDiscoveryWorkspace::new(input.clone(), tasks, std::collections::BTreeMap::new()),
    )
}

#[cfg(test)]
fn synthetic_diagnostics(
    tasks: usize,
    errors: usize,
    warnings: usize,
) -> (Vec<aplan_app::OverviewFinding>, DiagnosticWorkbench) {
    let finding =
        |code: String, severity: aplan_app::Severity, message: &str| aplan_app::OverviewFinding {
            identity: code.clone(),
            scope: DiagnosticScope::Check,
            code,
            severity,
            subject: "whole plan".to_string(),
            task: None,
            file: None,
            line: None,
            column: None,
            message: message.to_string(),
            agent_instruction: Some("repair the synthetic finding".to_string()),
            detail: Vec::new(),
            contribution: aplan_app::VerdictContribution::Blocking,
            source_line: None,
        };
    let findings = (0..errors)
        .map(|index| {
            finding(
                format!("ERROR-{index}"),
                aplan_app::Severity::Error,
                "synthetic error",
            )
        })
        .chain((0..warnings).map(|index| {
            finding(
                format!("WARN-{index}"),
                aplan_app::Severity::Warning,
                "synthetic warning",
            )
        }))
        .collect::<Vec<_>>();
    let check = DiagnosticRun {
        scope: DiagnosticScope::Check,
        coverage: if tasks == 0 {
            aplan_app::DiagnosticCoverage::NoSubjects("no synthetic tasks".to_string())
        } else {
            aplan_app::DiagnosticCoverage::Complete { subjects: tasks }
        },
        findings: findings.clone(),
        errors,
        warnings,
        blocking: findings.len(),
        advisory: 0,
        denied: errors > 0,
        input_label: "synthetic plan".to_string(),
    };
    let policy = aplan_app::CheckPolicySummary {
        require_effort: true,
        traces_root: None,
        traces_repository: None,
        traces_external: Vec::new(),
        detect_drift: false,
        deny_warnings: false,
        advisory: Vec::new(),
    };
    (
        findings,
        DiagnosticWorkbench {
            check,
            documents: DiagnosticRun {
                scope: DiagnosticScope::CheckDocs,
                coverage: aplan_app::DiagnosticCoverage::Unavailable(
                    "no synthetic document configuration".to_string(),
                ),
                findings: Vec::new(),
                errors: 0,
                warnings: 0,
                blocking: 0,
                advisory: 0,
                denied: false,
                input_label: "no configured documents".to_string(),
            },
            run_sheet: aplan_app::CheckRunSheet {
                repository: policy.clone(),
                effective: policy,
                overrides: CheckOverrides::default(),
                relaxed: false,
            },
        },
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum DiagnosticGroup {
    #[default]
    Canonical,
    Severity,
    Rule,
    Task,
    File,
}

impl DiagnosticGroup {
    pub(crate) const ALL: [Self; 5] = [
        Self::Canonical,
        Self::Severity,
        Self::Rule,
        Self::Task,
        Self::File,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Canonical => "canonical order",
            Self::Severity => "severity",
            Self::Rule => "rule",
            Self::Task => "task",
            Self::File => "file",
        }
    }

    fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .expect("every diagnostic group is enumerated");
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiagnosticFilter {
    Severity(aplan_app::Severity),
    Rule(String),
    Task(String),
    File(PathBuf),
}

impl DiagnosticFilter {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Severity(value) => format!("severity: {value}"),
            Self::Rule(value) => format!("rule: {value}"),
            Self::Task(value) => format!("task: {value}"),
            Self::File(value) => format!("file: {}", value.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiagnosticReturn {
    scope: DiagnosticScope,
    finding_identity: String,
}

fn all_finding_ids(workbench: &DiagnosticWorkbench) -> BTreeSet<String> {
    workbench
        .check
        .findings
        .iter()
        .chain(&workbench.documents.findings)
        .map(|finding| finding.identity.clone())
        .collect()
}

fn refresh_identity_changes(
    before: &aplan_app::InputIdentity,
    after: &aplan_app::InputIdentity,
    before_load: aplan_app::OverviewLoadState,
    after_load: aplan_app::OverviewLoadState,
) -> Vec<String> {
    let mut changes = Vec::new();
    if before.plan_root != after.plan_root || before.plan_sha256 != after.plan_sha256 {
        changes.push("plan input changed".to_string());
    }
    if before.configuration != after.configuration {
        changes.push("configuration changed".to_string());
    }
    if before.repository_root != after.repository_root || before.repository != after.repository {
        changes.push(format!(
            "repository is now {}",
            repository_identity_summary(&after.repository)
        ));
    }
    if before_load != after_load {
        changes.push(format!(
            "load state changed from {} to {}",
            load_state_label(before_load),
            load_state_label(after_load)
        ));
    }
    changes
}

fn repository_identity_summary(identity: &aplan_app::RepositoryIdentity) -> String {
    match identity {
        aplan_app::RepositoryIdentity::NotObserved { .. } => "not observed".to_string(),
        aplan_app::RepositoryIdentity::Unavailable { .. } => "unavailable".to_string(),
        aplan_app::RepositoryIdentity::Observed {
            head,
            modified,
            untracked,
            ..
        } => {
            let head = head
                .as_deref()
                .map_or("unborn", |sha| sha.get(..sha.len().min(12)).unwrap_or(sha));
            if *modified == 0 && *untracked == 0 {
                format!("HEAD {head}, clean")
            } else {
                format!("HEAD {head}, dirty ({modified} modified, {untracked} untracked)")
            }
        }
    }
}

const fn load_state_label(state: aplan_app::OverviewLoadState) -> &'static str {
    match state {
        aplan_app::OverviewLoadState::Complete => "complete",
        aplan_app::OverviewLoadState::Empty => "empty",
        aplan_app::OverviewLoadState::Malformed => "malformed",
        aplan_app::OverviewLoadState::PartiallyUnloadable => "partially unloadable",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverviewMetric {
    Cuts,
    Effort,
    Ready,
    Due,
    Findings,
    CriticalPath,
}

impl OverviewMetric {
    pub(crate) const ALL: [Self; 6] = [
        Self::Cuts,
        Self::Effort,
        Self::Ready,
        Self::Due,
        Self::Findings,
        Self::CriticalPath,
    ];

    const fn scope_label(self) -> &'static str {
        match self {
            Self::Cuts => "cut progress",
            Self::Effort => "live effort",
            Self::Ready => "ready work",
            Self::Due => "due work",
            Self::CriticalPath => "critical path",
            Self::Findings => "findings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotFreshness {
    Current,
    Refreshing,
    Unconfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TaskScope {
    Overview(OverviewMetric),
    Explicit {
        label: String,
        ids: BTreeSet<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectorSection {
    Summary,
    Work,
    Body,
    Relationships,
    Paths,
    Traces,
    Evidence,
    Findings,
}

impl InspectorSection {
    pub(crate) const ALL: [Self; 8] = [
        Self::Summary,
        Self::Work,
        Self::Body,
        Self::Relationships,
        Self::Paths,
        Self::Traces,
        Self::Evidence,
        Self::Findings,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Work => "Work",
            Self::Body => "Body",
            Self::Relationships => "Relations",
            Self::Paths => "Paths",
            Self::Traces => "Traces",
            Self::Evidence => "Evidence",
            Self::Findings => "Findings",
        }
    }

    fn adjacent(self, forward: bool) -> Self {
        let index = Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .expect("every inspector section is enumerated");
        let next = if forward {
            (index + 1) % Self::ALL.len()
        } else {
            index.checked_sub(1).unwrap_or(Self::ALL.len() - 1)
        };
        Self::ALL[next]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InspectorLink {
    Task(String),
    Section(InspectorSection),
}

/// Non-colour meaning carried by one inspector value.
///
/// The renderer maps this to the active palette, but the presentation retains
/// a textual cue even in monochrome. Keeping the tone beside the value also
/// prevents a renderer from inferring status by matching user-visible prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectorTone {
    Normal,
    Accent,
    Good,
    Warning,
    Error,
    Muted,
}

/// The small semantic vocabulary shared by every task-inspector section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InspectorPresentation {
    Identity {
        id: String,
        title: String,
    },
    Labelled {
        label: String,
        value: String,
    },
    Badge {
        label: String,
        value: String,
        tone: InspectorTone,
    },
    Metric {
        label: String,
        value: String,
        detail: Option<String>,
    },
    Callout {
        label: String,
        value: String,
        tone: InspectorTone,
    },
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectorItem {
    /// A stable semantic group. The renderer emits this as a heading without
    /// inserting a selectable pseudo-row into keyboard navigation.
    pub(crate) group: Option<&'static str>,
    pub(crate) presentation: InspectorPresentation,
    pub(crate) link: Option<InspectorLink>,
}

impl InspectorItem {
    fn text(text: impl Into<String>) -> Self {
        Self {
            group: None,
            presentation: InspectorPresentation::Text(text.into()),
            link: None,
        }
    }

    fn labelled(group: &'static str, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            group: Some(group),
            presentation: InspectorPresentation::Labelled {
                label: label.into(),
                value: value.into(),
            },
            link: None,
        }
    }

    fn badge(
        group: &'static str,
        label: impl Into<String>,
        value: impl Into<String>,
        tone: InspectorTone,
    ) -> Self {
        Self {
            group: Some(group),
            presentation: InspectorPresentation::Badge {
                label: label.into(),
                value: value.into(),
                tone,
            },
            link: None,
        }
    }

    fn metric(
        group: &'static str,
        label: impl Into<String>,
        value: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            group: Some(group),
            presentation: InspectorPresentation::Metric {
                label: label.into(),
                value: value.into(),
                detail,
            },
            link: None,
        }
    }

    fn callout(
        group: &'static str,
        label: impl Into<String>,
        value: impl Into<String>,
        tone: InspectorTone,
    ) -> Self {
        Self {
            group: Some(group),
            presentation: InspectorPresentation::Callout {
                label: label.into(),
                value: value.into(),
                tone,
            },
            link: None,
        }
    }

    fn identity(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            group: None,
            presentation: InspectorPresentation::Identity {
                id: id.into(),
                title: title.into(),
            },
            link: None,
        }
    }

    fn with_link(mut self, link: InspectorLink) -> Self {
        self.link = Some(link);
        self
    }

    fn section(
        group: &'static str,
        label: impl Into<String>,
        value: impl Into<String>,
        detail: Option<String>,
        section: InspectorSection,
    ) -> Self {
        Self::metric(group, label, value, detail).with_link(InspectorLink::Section(section))
    }

    fn task(
        group: &'static str,
        label: impl Into<String>,
        value: impl Into<String>,
        id: &str,
    ) -> Self {
        Self::labelled(group, label, value).with_link(InspectorLink::Task(id.to_string()))
    }

    #[cfg(test)]
    pub(crate) fn semantic_text(&self) -> String {
        match &self.presentation {
            InspectorPresentation::Identity { id, title } => format!("{id} {title}"),
            InspectorPresentation::Labelled { label, value }
            | InspectorPresentation::Badge { label, value, .. }
            | InspectorPresentation::Callout { label, value, .. } => format!("{label}: {value}"),
            InspectorPresentation::Metric {
                label,
                value,
                detail,
            } => detail.as_ref().map_or_else(
                || format!("{label}: {value}"),
                |detail| format!("{label}: {value}; {detail}"),
            ),
            InspectorPresentation::Text(text) => text.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectorLocation {
    task_id: String,
    section: InspectorSection,
    selected: usize,
    scroll: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectKind {
    Startup,
    Probe,
    Refresh,
    Retry,
    Verify,
    Mutation,
    TestCatalogue,
}

impl EffectKind {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 7] = [
        Self::Startup,
        Self::Probe,
        Self::Refresh,
        Self::Retry,
        Self::Verify,
        Self::Mutation,
        Self::TestCatalogue,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Probe => "external-change probe",
            Self::Refresh => "refresh",
            Self::Retry => "onboarding retry",
            Self::Verify => "verify",
            Self::Mutation => "mutation",
            Self::TestCatalogue => "configured test catalogue",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct EffectId(pub(crate) u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EffectRequest {
    pub(crate) id: EffectId,
    pub(crate) generation: u64,
    pub(crate) focus_epoch: u64,
    pub(crate) input: String,
    pub(crate) kind: EffectKind,
    pub(crate) check_overrides: CheckOverrides,
    pub(crate) execute: bool,
    pub(crate) mutation: Option<MutationEffect>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EffectOutput {
    Startup(Box<StartupReady>),
    Identity(Box<aplan_app::InputIdentity>),
    LaunchSnapshot {
        snapshot: Box<Snapshot>,
        resolution: aplan_app::LaunchResolution,
    },
    Onboarding(Box<Onboarding>),
    Snapshot(Box<Snapshot>),
    Verification(Box<aplan_app::VerificationWorkbench>),
    Mutation(MutationTransfer),
    TestCatalogue(Box<crate::evidence::TestCatalogueView>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StartupReady {
    pub(crate) snapshot: Snapshot,
    pub(crate) onboarding: Option<Onboarding>,
    pub(crate) capabilities: CapabilityView,
    pub(crate) resolution: Option<aplan_app::LaunchResolution>,
}

#[derive(Debug)]
pub(crate) struct StartupState {
    pub(crate) started: Instant,
    pub(crate) latest: Option<aplan_app::ProgressEvent>,
    progress: aplan_app::ProgressState,
}

impl StartupState {
    fn new(operation: aplan_app::OperationId) -> Self {
        Self {
            started: Instant::now(),
            latest: None,
            progress: aplan_app::ProgressState::new(operation, aplan_app::OperationKind::PlanLoad),
        }
    }

    fn accept(&mut self, event: aplan_app::ProgressEvent) -> bool {
        if self.progress.accept(event).is_err() {
            return false;
        }
        self.latest = Some(event);
        true
    }
}

impl EffectOutput {
    pub(crate) fn snapshot(snapshot: Snapshot) -> Self {
        Self::Snapshot(Box::new(snapshot))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EffectFailure {
    #[error("repository operation failed: {0}")]
    Repository(String),
    #[error("configured process failed: {0}")]
    Process(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("verification input changed after review (expected {expected}, loaded {actual})")]
    Stale { expected: String, actual: String },
    #[error("the background worker panicked")]
    WorkerPanicked,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EffectCompletion {
    pub(crate) request: EffectRequest,
    pub(crate) outcome: Result<EffectOutput, EffectFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectState {
    Queued,
    Running,
    Complete,
    Failed,
    Stale,
    Refused,
    Cancelled,
    Unavailable,
}

impl EffectState {
    const fn is_pending(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectRecord {
    request: EffectRequest,
    state: EffectState,
    detail: Option<String>,
    progress: Option<aplan_app::ProgressState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Notice {
    text: String,
    ticks_left: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchOperation {
    Insert,
    Backspace,
    Delete,
}

fn edit_search_text(
    query: &mut String,
    cursor: &mut usize,
    operation: SearchOperation,
    input: Option<&str>,
    bounds: (usize, usize, usize),
) -> bool {
    let (max_cells, max_graphemes, max_bytes) = bounds;
    match (operation, input) {
        (SearchOperation::Insert, Some(input)) => {
            text::insert_bounded(query, cursor, input, max_cells, max_graphemes, max_bytes)
        }
        (SearchOperation::Backspace, _) => text::delete_previous_grapheme(query, cursor),
        (SearchOperation::Delete, _) => text::delete_next_grapheme(query, cursor),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionState {
    Present,
    Removed,
}

/// A graph node's non-colour identity relative to the current selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphRole {
    Selected,
    SelectedCritical,
    Blocker,
    BlockerCritical,
    BlockerDependent,
    Dependent,
    DependentCritical,
    Member,
    MemberCritical,
    Critical,
    Unrelated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphViewportMode {
    FollowSelection,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraphSessionState {
    NeverVisited,
    Started,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphJump {
    PreviousLayer,
    NextLayer,
    Source,
    Sink,
    Critical,
    Milestone,
    PreviousPhase,
    NextPhase,
    Finding,
}

impl GraphJump {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::PreviousLayer => "previous dependency layer",
            Self::NextLayer => "next dependency layer",
            Self::Source => "source",
            Self::Sink => "sink",
            Self::Critical => "critical-route node",
            Self::Milestone => "milestone",
            Self::PreviousPhase => "previous phase",
            Self::NextPhase => "next phase",
            Self::Finding => "diagnostic-bearing node",
        }
    }
}

impl GraphRole {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn badge(self) -> &'static str {
        match self {
            Self::Selected => "[S] ",
            Self::SelectedCritical => "[SC]",
            Self::Blocker => "[B] ",
            Self::BlockerCritical => "[BC]",
            Self::BlockerDependent => "[BR]",
            Self::Dependent => "[R] ",
            Self::DependentCritical => "[RC]",
            Self::Member => "[M] ",
            Self::MemberCritical => "[MC]",
            Self::Critical => "[C] ",
            Self::Unrelated => "[ ] ",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphExplorerLocationState {
    location: GraphExplorerLocation,
    selected: Option<GraphExplorerIdentity>,
    continuation: Option<aplan_app::GraphContinuationCursor>,
    scroll_row: usize,
    scroll_column: usize,
    viewport_mode: GraphViewportMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphRelationCursor {
    subject: GraphExplorerIdentity,
    incoming: bool,
    index: usize,
    count: usize,
    last_target: GraphExplorerIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphReturnContext {
    route: Route,
    selected_id: Option<String>,
    graph_request: GraphRequest,
    explorer: GraphExplorerLocationState,
    ordering: GraphOrdering,
    orientation: CellSceneOrientation,
    density: GraphDensity,
    resolved_density: ResolvedGraphDensity,
    edge_presentation: GraphEdgePresentation,
}

// One declaration drives the production action enum and the test-only kind
// inventory. Adding an action therefore expands the surface that the reducer
// table must cover; there is no second hand-maintained list that can omit it.
macro_rules! define_actions {
    (
        unit { $( $(#[$unit_attr:meta])* $unit:ident ),+ $(,)? }
        tuple { $( $(#[$tuple_attr:meta])* $tuple:ident($tuple_ty:ty) ),+ $(,)? }
        record { $( $(#[$record_attr:meta])* $record:ident {
            $( $field:ident: $field_ty:ty ),+ $(,)?
        } ),+ $(,)? }
    ) => {
        #[derive(Debug, Clone, PartialEq)]
        pub(crate) enum Action {
            $( $(#[$unit_attr])* $unit, )+
            $( $(#[$tuple_attr])* $tuple($tuple_ty), )+
            $( $(#[$record_attr])* $record { $( $field: $field_ty, )+ }, )+
        }

        #[cfg(test)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub(crate) enum ActionKind {
            $( $unit, )+
            $( $tuple, )+
            $( $record, )+
        }

        #[cfg(test)]
        impl ActionKind {
            pub(crate) const ALL: &'static [Self] = &[
                $( Self::$unit, )+
                $( Self::$tuple, )+
                $( Self::$record, )+
            ];
        }

        impl Action {
            #[cfg(test)]
            pub(crate) const fn kind(&self) -> ActionKind {
                match self {
                    $( Self::$unit => ActionKind::$unit, )+
                    $( Self::$tuple(..) => ActionKind::$tuple, )+
                    $( Self::$record { .. } => ActionKind::$record, )+
                }
            }
        }
    };
}

define_actions! {
    unit {
        Quit,
        OpenDetail,
        CloseLayer,
        OpenAbout,
        OpenHelp,
        OpenPalette,
        OpenSearch,
        OpenReportDate,
        OpenWorkDate,
        OpenTestCatalogue,
        UseTestEvidence,
        SubmitWorkDate,
        OpenFilters,
        OpenRunSheet,
        OpenVerifyRunSheet,
        OpenContextualRule,
        OpenRuleSearch,
        SubmitRuleSearch,
        OpenFindingTask,
        CycleDiagnosticScope,
        PaletteBackspace,
        SearchBackspace,
        SearchDelete,
        SearchLeft,
        SearchRight,
        SearchHome,
        SearchEnd,
        SearchClear,
        ClearFilters,
        ClearNarrowing,
        CycleTaskSort,
        ToggleWhyTraversal,
        PreviousInspectorSection,
        NextInspectorSection,
        CycleTheme,
        ToggleGlyphs,
        CycleGraphScope,
        SetGraphBetween,
        ToggleGraphMembership,
        CycleGraphDensity,
        CycleGraphOrdering,
        CycleGraphOrientation,
        ToggleGraphRelationsOnly,
        GraphOverview,
        FitGraph,
        FollowGraphBlocker,
        FollowGraphDependent,
        PreviousGraphContinuation,
        NextGraphContinuation,
        OpenGraphContinuation,
        ContractGraphDepth,
        ExpandGraphDepth,
        PreviousGraphWindow,
        RecenterGraph,
        SelectNext,
        SelectPrevious,
        ExternalProbeOverdue,
        PollExternal,
        Refresh,
        Retry,
        #[cfg(test)]
        Verify,
        CancelQueued,
        ConfirmMutation,
        OpenExternalEditor,
        FocusGained,
        FocusLost,
        Tick,
    }
    tuple {
        Route(Route),
        Focus(Focus),
        PaletteInput(String),
        SearchInput(String),
        ToggleFilter(usize),
        ToggleRunSheet(usize),
        ToggleVerifyRunSheet(usize),
        SelectModal(usize),
        AdjustModalScroll(i8),
        SelectIndex(usize),
        SelectReportRow(usize),
        Notify(String),
        Invoke(Box<Action>),
        OpenMutation(MutationAction),
        MutationForm(FormAction),
        MutationScroll(i8),
        Paste(String),
        EffectStarted(EffectId),
        SetInspectorScroll(usize),
        #[cfg_attr(not(test), allow(dead_code))]
        SelectTask(String),
        SelectGraphIdentity(GraphExplorerIdentity),
        OpenGraphAt(String),
        OpenGraphLocation(GraphExplorerLocation),
        MoveGraph(SpatialDirection),
        JumpGraph(GraphJump),
        EffectCompleted(Box<EffectCompletion>),
    }
    record {
        Resize { columns: u16, rows: u16 },
        EffectRejected { id: EffectId, reason: String },
        EffectProgress { id: EffectId, progress: aplan_app::ProgressEvent },
        SetGraphViewport { row: usize, column: usize },
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Transition {
    pub(crate) effect: Option<EffectRequest>,
    pub(crate) cancel: Option<EffectId>,
}

/// State changed only by semantic actions. Neither terminal events nor worker
/// threads can mutate it directly.
#[derive(Debug)]
pub(crate) struct Model {
    pub(crate) snapshot: Snapshot,
    pub(crate) onboarding: Option<Onboarding>,
    pub(crate) capabilities: CapabilityView,
    pub(crate) launch_resolution: Option<aplan_app::LaunchResolution>,
    pub(crate) startup: Option<StartupState>,
    startup_failure: Option<String>,
    pub(crate) onboarding_scroll: usize,
    pub(crate) selected: usize,
    pub(crate) overview_selected: usize,
    pub(crate) route: Route,
    pub(crate) focus: Focus,
    pub(crate) modal: Option<Modal>,
    pub(crate) mutation: Option<MutationLayer>,
    pub(crate) modal_selected: usize,
    pub(crate) modal_scroll: usize,
    pub(crate) palette_query: String,
    pub(crate) test_query: String,
    pub(crate) test_query_cursor: usize,
    pub(crate) test_catalogue: Option<TestCatalogueLayer>,
    pending_test_reference: Option<String>,
    pub(crate) task_query: String,
    pub(crate) task_query_cursor: usize,
    pub(crate) inbox_query: String,
    pub(crate) inbox_query_cursor: usize,
    pub(crate) inbox_state: Option<aplan_core::capture::CaptureState>,
    pub(crate) inbox_revisit: aplan_core::capture::RevisitFilter,
    pub(crate) inbox_kind: Option<String>,
    pub(crate) inbox_label: Option<String>,
    pub(crate) selected_capture_id: Option<String>,
    pub(crate) task_selector: Selector,
    pub(crate) task_sort: TaskSort,
    pub(crate) task_browser: TaskBrowserResponse,
    pub(crate) graph_request: GraphRequest,
    pub(crate) graph_projection: GraphProjection,
    pub(crate) graph_window_request: GraphWindowRequest,
    pub(crate) graph_window: GraphWindowProjection,
    pub(crate) graph_scene: Result<GraphScene, TooWide>,
    pub(crate) graph_explorer_request: GraphExplorerRequest,
    pub(crate) graph_explorer: GraphExplorerProjection,
    graph_fallback_node_columns: usize,
    pub(crate) graph_cell_scene: CellSceneResult,
    graph_scene_generation: u64,
    pub(crate) graph_density: GraphDensity,
    pub(crate) graph_resolved_density: ResolvedGraphDensity,
    pub(crate) graph_ordering: GraphOrdering,
    pub(crate) graph_orientation: CellSceneOrientation,
    pub(crate) graph_edge_presentation: GraphEdgePresentation,
    pub(crate) graph_continuation_selected: usize,
    graph_explorer_history: Vec<GraphExplorerLocationState>,
    graph_relation_cursor: Option<GraphRelationCursor>,
    graph_return_context: Option<GraphReturnContext>,
    graph_session: GraphSessionState,
    pub(crate) graph_scroll_row: usize,
    pub(crate) graph_scroll_column: usize,
    pub(crate) graph_viewport_mode: GraphViewportMode,
    pub(crate) graph_between_anchor: Option<String>,
    pub(crate) report_kind: ReportKind,
    pub(crate) report_projection: ReportProjection,
    pub(crate) report_selected_row: usize,
    pub(crate) report_kind_query: String,
    pub(crate) report_kind_query_cursor: usize,
    pub(crate) report_row_query: String,
    pub(crate) report_row_query_cursor: usize,
    pub(crate) report_as_of: String,
    pub(crate) report_as_of_cursor: usize,
    pub(crate) work_as_of: String,
    pub(crate) work_as_of_draft: String,
    pub(crate) work_as_of_cursor: usize,
    pub(crate) work_as_of_epoch_seconds: i64,
    pub(crate) why_traversal: WhyTraversal,
    pub(crate) work_projection: Option<aplan_app::WorkDiscoveryProjection>,
    task_scope: Option<TaskScope>,
    pub(crate) diagnostic_scope: DiagnosticScope,
    pub(crate) diagnostic_group: DiagnosticGroup,
    pub(crate) diagnostic_query: String,
    pub(crate) diagnostic_query_cursor: usize,
    pub(crate) diagnostic_filters: Vec<DiagnosticFilter>,
    pub(crate) selected_finding_id: Option<String>,
    diagnostic_return: Option<DiagnosticReturn>,
    pub(crate) pending_check_overrides: CheckOverrides,
    pub(crate) pending_verification_mode: VerificationMode,
    pub(crate) rule_query: String,
    pub(crate) rule_query_cursor: usize,
    pub(crate) rule_explanation: Option<aplan_app::RuleExplanation>,
    pub(crate) selected_id: Option<String>,
    pub(crate) selection_state: SelectionState,
    pub(crate) inspector_section: InspectorSection,
    pub(crate) inspector_selected: usize,
    /// `None` follows the selected item's wrapped range. `Some(0)` is an
    /// explicit request for the top, so `PageUp` can reach it from a selected
    /// item that started below the viewport.
    pub(crate) inspector_scroll: Option<usize>,
    inspector_history: Vec<InspectorLocation>,
    pub(crate) theme: Theme,
    pub(crate) glyphs: Glyphs,
    pub(crate) color_capability: ColorCapability,
    pub(crate) motion: Motion,
    pub(crate) columns: u16,
    pub(crate) rows: u16,
    pub(crate) active: bool,
    pub(crate) freshness: SnapshotFreshness,
    generation: u64,
    focus_epoch: u64,
    next_effect: u64,
    effects: VecDeque<EffectRecord>,
    pub(crate) verification: Option<Box<aplan_app::VerificationWorkbench>>,
    notice: Option<Notice>,
    pub(crate) animation_frame: u8,
    dirty: bool,
    external_editor_request: Option<String>,
    quit: bool,
}

fn initial_work_projection(
    snapshot: &Snapshot,
    selected_id: Option<&str>,
    as_of_epoch_seconds: i64,
) -> Option<aplan_app::WorkDiscoveryProjection> {
    selected_id.and_then(|id| {
        snapshot.work.project(
            id,
            WorkDiscoveryRequest {
                traversal: WhyTraversal::CutBounded,
                as_of_epoch_seconds,
            },
        )
    })
}

pub(crate) const fn capture_state_name(state: aplan_core::capture::CaptureState) -> &'static str {
    match state {
        aplan_core::capture::CaptureState::Open => "OPEN",
        aplan_core::capture::CaptureState::Promoted => "PROMOTED",
        aplan_core::capture::CaptureState::Discarded => "DISCARDED",
    }
}

fn capture_is_deferred(capture: &aplan_core::capture::Capture, evaluated_at: &str) -> bool {
    capture.state == aplan_core::capture::CaptureState::Open
        && capture
            .revisit_at
            .as_deref()
            .is_some_and(|instant| instant > evaluated_at)
}

fn format_progress_detail(event: aplan_app::ProgressEvent) -> String {
    let lifecycle = match event.lifecycle {
        aplan_app::ProgressLifecycle::Queued => "queued",
        aplan_app::ProgressLifecycle::Started => "started",
        aplan_app::ProgressLifecycle::Advanced => "in progress",
        aplan_app::ProgressLifecycle::Completed => "complete",
        aplan_app::ProgressLifecycle::Failed => "failed",
        aplan_app::ProgressLifecycle::Cancelled => "cancelled",
    };
    match event.subject {
        aplan_app::ProgressSubject::Operation => {
            format!("{} {lifecycle}", event.kind.label())
        }
        aplan_app::ProgressSubject::Phase { phase, child } => {
            let child = child.map_or_else(String::new, |child| format!(" {}", child.get()));
            match event.measure {
                aplan_app::ProgressMeasure::Indeterminate => {
                    format!("{}{child}: {lifecycle}", phase.label())
                }
                aplan_app::ProgressMeasure::Bounded {
                    completed, total, ..
                } => format!("{}{child}: {completed}/{total}, {lifecycle}", phase.label()),
            }
        }
    }
}

impl Model {
    #[cfg(test)]
    pub(crate) fn new(snapshot: Snapshot) -> Self {
        let capabilities = aplan_app::BuildFacts::default().capability_view(&snapshot.build);
        Self::with_launch(snapshot, None, capabilities, None, Appearance::default())
    }

    // One complete initializer is easier to audit than defaults patched after
    // construction: every reducer field starts from one visible state.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn with_launch(
        snapshot: Snapshot,
        onboarding: Option<Onboarding>,
        capabilities: CapabilityView,
        launch_resolution: Option<aplan_app::LaunchResolution>,
        appearance: Appearance,
    ) -> Self {
        let work_as_of_epoch_seconds = snapshot.overview.as_of_epoch_seconds;
        let task_browser =
            aplan_app::browse_tasks(&snapshot.task_rows, &TaskBrowserRequest::default());
        let selected_id = task_browser.rows.first().map(|row| row.task.id.clone());
        let selected_capture_id = snapshot.inbox.as_ref().ok().and_then(|workspace| {
            workspace
                .captures
                .iter()
                .find(|capture| {
                    capture.state == aplan_core::capture::CaptureState::Open
                        && !capture_is_deferred(capture, &workspace.evaluated_at)
                })
                .map(|capture| capture.id.clone())
        });
        let selected_finding_id = snapshot
            .diagnostics
            .check
            .findings
            .first()
            .map(|finding| finding.identity.clone());
        let work_projection =
            initial_work_projection(&snapshot, selected_id.as_deref(), work_as_of_epoch_seconds);
        let graph_request = GraphRequest::default();
        let graph_projection = snapshot.graph.project(&graph_request);
        let graph_window_request = GraphWindowRequest {
            preferred_anchor: selected_id.clone(),
            ..GraphWindowRequest::default()
        };
        let graph_window = graph_projection.window(&graph_window_request);
        let graph_scene = GraphScene::layout(&graph_window.view, aplan_output::MAX_WIDTH);
        let graph_explorer_request = GraphExplorerRequest {
            graph: graph_request.clone(),
            ..GraphExplorerRequest::default()
        };
        let graph_explorer = snapshot.graph.project_explorer(&graph_explorer_request);
        let graph_fallback_node_columns = graph_explorer
            .graph
            .nodes
            .iter()
            .map(fallback_node_width)
            .max()
            .unwrap_or_default();
        let graph_cell_scene = CellSceneJob::new(
            1,
            layered_scene_request(
                graph_explorer.clone(),
                ResolvedGraphDensity::Comfortable,
                CellSceneOrientation::Auto,
                GraphOrdering::Readability,
                BTreeMap::new(),
                None,
            ),
        )
        .run()
        .result;
        let report_kind = ReportKind::Roadmap;
        let report_projection = snapshot
            .reports
            .project(report_kind, &ReportRequest::default());
        Self {
            snapshot,
            onboarding,
            capabilities,
            launch_resolution,
            startup: None,
            startup_failure: None,
            onboarding_scroll: 0,
            selected: 0,
            overview_selected: 0,
            route: Route::Overview,
            focus: Focus::Primary,
            modal: None,
            mutation: None,
            modal_selected: 0,
            modal_scroll: 0,
            palette_query: String::new(),
            test_query: String::new(),
            test_query_cursor: 0,
            test_catalogue: None,
            pending_test_reference: None,
            task_query: String::new(),
            task_query_cursor: 0,
            inbox_query: String::new(),
            inbox_query_cursor: 0,
            inbox_state: Some(aplan_core::capture::CaptureState::Open),
            inbox_revisit: aplan_core::capture::RevisitFilter::Actionable,
            inbox_kind: None,
            inbox_label: None,
            selected_capture_id,
            task_selector: Selector::default(),
            task_sort: TaskSort::default(),
            task_browser,
            graph_request,
            graph_projection,
            graph_window_request,
            graph_window,
            graph_scene,
            graph_explorer_request,
            graph_explorer,
            graph_fallback_node_columns,
            graph_cell_scene,
            graph_scene_generation: 1,
            graph_density: GraphDensity::Auto,
            graph_resolved_density: ResolvedGraphDensity::Comfortable,
            graph_ordering: GraphOrdering::Readability,
            graph_orientation: CellSceneOrientation::Auto,
            graph_edge_presentation: GraphEdgePresentation::All,
            graph_continuation_selected: 0,
            graph_explorer_history: Vec::new(),
            graph_relation_cursor: None,
            graph_return_context: None,
            graph_session: GraphSessionState::NeverVisited,
            graph_scroll_row: 0,
            graph_scroll_column: 0,
            graph_viewport_mode: GraphViewportMode::FollowSelection,
            graph_between_anchor: None,
            report_kind,
            report_projection,
            report_selected_row: 0,
            report_kind_query: String::new(),
            report_kind_query_cursor: 0,
            report_row_query: String::new(),
            report_row_query_cursor: 0,
            report_as_of: String::new(),
            report_as_of_cursor: 0,
            work_as_of: String::new(),
            work_as_of_draft: String::new(),
            work_as_of_cursor: 0,
            work_as_of_epoch_seconds,
            why_traversal: WhyTraversal::CutBounded,
            work_projection,
            task_scope: None,
            diagnostic_scope: DiagnosticScope::Check,
            diagnostic_group: DiagnosticGroup::Canonical,
            diagnostic_query: String::new(),
            diagnostic_query_cursor: 0,
            diagnostic_filters: Vec::new(),
            selected_finding_id,
            diagnostic_return: None,
            pending_check_overrides: CheckOverrides::default(),
            pending_verification_mode: VerificationMode::ResolveOnly,
            rule_query: String::new(),
            rule_query_cursor: 0,
            rule_explanation: None,
            selected_id,
            selection_state: SelectionState::Present,
            inspector_section: InspectorSection::Summary,
            inspector_selected: 0,
            inspector_scroll: None,
            inspector_history: Vec::new(),
            theme: appearance.theme,
            glyphs: appearance.glyphs,
            color_capability: appearance.color,
            motion: if appearance.reduced_motion {
                Motion::Reduced
            } else {
                Motion::Animated
            },
            columns: 0,
            rows: 0,
            active: true,
            freshness: SnapshotFreshness::Current,
            generation: 0,
            focus_epoch: 0,
            next_effect: 1,
            effects: VecDeque::new(),
            verification: None,
            notice: None,
            animation_frame: 0,
            dirty: true,
            external_editor_request: None,
            quit: false,
        }
    }

    pub(crate) fn loading(
        build: &BuildIdentity,
        capabilities: CapabilityView,
        appearance: Appearance,
    ) -> (Self, EffectRequest) {
        let snapshot = Snapshot::loading(build);
        let mut model = Self::with_launch(snapshot, None, capabilities, None, appearance);
        let operation = aplan_app::OperationId::new(model.next_effect);
        model.startup = Some(StartupState::new(operation));
        let transition = model.request_effect(EffectKind::Startup, CheckOverrides::default());
        let request = transition
            .effect
            .expect("a fresh loading model always admits its startup effect");
        (model, request)
    }

    pub(crate) fn update(&mut self, action: Action) -> Transition {
        match action {
            Action::OpenMutation(action) => {
                self.pending_test_reference = None;
                self.open_mutation(action)
            }
            Action::MutationForm(action) => self.update_mutation_form(action),
            Action::MutationScroll(delta) => {
                self.scroll_mutation(delta);
                Transition::default()
            }
            Action::ConfirmMutation => self.confirm_quick_mutation(),
            Action::OpenExternalEditor => {
                self.request_external_editor();
                Transition::default()
            }
            action => self.update_non_mutation(action),
        }
    }

    #[allow(clippy::too_many_lines)] // One exhaustive reducer keeps the closed action surface auditable.
    fn update_non_mutation(&mut self, action: Action) -> Transition {
        match action {
            Action::Quit => self.quit = true,
            Action::Route(route) => self.select_route(route),
            Action::Focus(focus) => {
                self.focus = focus;
                self.dirty = true;
            }
            Action::OpenDetail => self.open_detail(),
            Action::CloseLayer => return self.close_layer(),
            Action::OpenAbout => self.open_modal(Modal::About),
            Action::OpenHelp => self.open_modal(Modal::Help),
            Action::OpenPalette => self.open_modal(Modal::Palette),
            Action::OpenSearch => self.open_modal(Modal::Search),
            Action::OpenReportDate => self.open_modal(Modal::ReportDate),
            Action::OpenWorkDate => self.open_work_date(),
            Action::OpenTestCatalogue => return self.open_test_catalogue(),
            Action::UseTestEvidence => return self.use_test_evidence(),
            Action::SubmitWorkDate => self.submit_work_date(),
            Action::OpenFilters => self.open_modal(Modal::Filters),
            Action::OpenRunSheet => self.open_modal(Modal::RunSheet),
            Action::OpenVerifyRunSheet => self.open_modal(Modal::VerifyRunSheet),
            Action::OpenContextualRule => self.open_contextual_rule(),
            Action::OpenRuleSearch => self.open_rule_search(),
            Action::SubmitRuleSearch => self.submit_rule_search(),
            Action::OpenFindingTask => self.open_finding_task(),
            Action::CycleDiagnosticScope => self.cycle_diagnostic_scope(),
            Action::PaletteInput(input) => self.edit_palette(Some(&input)),
            Action::PaletteBackspace => self.edit_palette(None),
            Action::SearchInput(input) => self.edit_search(Some(&input), SearchOperation::Insert),
            Action::SearchBackspace => self.edit_search(None, SearchOperation::Backspace),
            Action::SearchDelete => self.edit_search(None, SearchOperation::Delete),
            Action::SearchLeft => self.move_search_cursor(false),
            Action::SearchRight => self.move_search_cursor(true),
            Action::SearchHome => self.move_search_boundary(false),
            Action::SearchEnd => self.move_search_boundary(true),
            Action::SearchClear => self.clear_active_query(),
            Action::ClearFilters => self.clear_filters(),
            Action::ClearNarrowing => self.clear_narrowing(),
            Action::CycleTaskSort => self.cycle_sort_or_group(),
            Action::ToggleWhyTraversal => self.toggle_why_traversal(),
            Action::PreviousInspectorSection => self.move_inspector_section(false),
            Action::NextInspectorSection => self.move_inspector_section(true),
            Action::SetInspectorScroll(scroll) => self.set_inspector_scroll(scroll),
            Action::ToggleFilter(index) => self.toggle_filter(index),
            Action::ToggleRunSheet(index) => return self.toggle_run_sheet(index),
            Action::ToggleVerifyRunSheet(index) => return self.toggle_verify_run_sheet(index),
            Action::SelectModal(index) => self.select_modal(index),
            Action::AdjustModalScroll(delta) => self.adjust_modal_scroll(delta),
            Action::CycleTheme => self.cycle_theme(),
            Action::ToggleGlyphs => self.toggle_glyphs(),
            Action::CycleGraphScope => self.cycle_graph_scope(),
            Action::SetGraphBetween => self.set_graph_between(),
            Action::ToggleGraphMembership => self.toggle_graph_membership(),
            Action::CycleGraphDensity => self.cycle_graph_density(),
            Action::CycleGraphOrdering => self.cycle_graph_ordering(),
            Action::CycleGraphOrientation => self.cycle_graph_orientation(),
            Action::ToggleGraphRelationsOnly => self.toggle_graph_relations_only(),
            Action::GraphOverview => self.open_graph_overview(),
            Action::FitGraph => self.fit_graph(),
            Action::FollowGraphBlocker => self.follow_graph_relation(true),
            Action::FollowGraphDependent => self.follow_graph_relation(false),
            Action::PreviousGraphContinuation => self.move_graph_continuation(false),
            Action::NextGraphContinuation => self.move_graph_continuation(true),
            Action::OpenGraphContinuation => self.open_graph_continuation(),
            Action::ContractGraphDepth => self.adjust_graph_depth(false),
            Action::ExpandGraphDepth => self.adjust_graph_depth(true),
            Action::PreviousGraphWindow => self.restore_previous_graph_context(),
            Action::RecenterGraph => {
                self.graph_viewport_mode = GraphViewportMode::FollowSelection;
                self.dirty = true;
            }
            Action::SelectIndex(index) => self.select_index(index),
            Action::SelectReportRow(index) => self.select_report_row(index),
            Action::SelectTask(id) => self.select_graph_task(id),
            Action::SelectGraphIdentity(identity) => self.select_graph_identity(identity),
            Action::OpenGraphAt(id) => self.open_graph_at(id),
            Action::OpenGraphLocation(location) => self.open_graph_location(location, true),
            Action::MoveGraph(direction) => self.move_graph_spatial(direction),
            Action::JumpGraph(jump) => self.jump_graph(jump),
            Action::SetGraphViewport { row, column } => self.set_graph_viewport(row, column),
            Action::Notify(message) => self.show_notice(message),
            Action::Invoke(action) => return self.invoke(*action),
            Action::SelectNext => self.move_selection(true),
            Action::SelectPrevious => self.move_selection(false),
            Action::ExternalProbeOverdue => self.mark_external_probe_overdue(),
            Action::PollExternal => return self.request_external_probe(),
            Action::Refresh => {
                return self.request_effect(EffectKind::Refresh, CheckOverrides::default());
            }
            Action::Retry => {
                return self.request_effect(EffectKind::Retry, CheckOverrides::default());
            }
            #[cfg(test)]
            Action::Verify => return self.request_verification(false),
            Action::CancelQueued => return self.cancel_queued(),
            Action::OpenMutation(_)
            | Action::MutationForm(_)
            | Action::MutationScroll(_)
            | Action::ConfirmMutation
            | Action::OpenExternalEditor => {
                unreachable!("mutation actions are routed by update")
            }
            Action::Resize { columns, rows } => {
                self.columns = columns;
                self.rows = rows;
                let resolved = resolve_density(
                    self.graph_density,
                    self.graph_resolved_density,
                    columns,
                    rows,
                );
                if resolved != self.graph_resolved_density {
                    self.graph_resolved_density = resolved;
                    if self.route == Route::Graph {
                        self.rebuild_layered_scene_with_prior(false);
                    }
                }
                self.dirty = true;
            }
            Action::Paste(text) => self.paste(&text),
            Action::FocusGained => return self.update_active(true),
            Action::FocusLost => return self.update_active(false),
            Action::Tick => self.tick(),
            Action::EffectStarted(id) => self.start_effect(id),
            Action::EffectCompleted(completion) => return self.complete_effect(*completion),
            Action::EffectProgress { id, progress } => self.accept_effect_progress(id, &progress),
            Action::EffectRejected { id, reason } => self.reject_effect(id, &reason),
        }
        Transition::default()
    }

    fn start_effect(&mut self, id: EffectId) {
        if let Some(record) = self.effect_mut(id) {
            record.state = EffectState::Running;
            if record.request.kind != EffectKind::Probe {
                self.dirty = true;
            }
        }
    }

    fn select_modal(&mut self, index: usize) {
        self.modal_selected = index;
        self.modal_scroll = 0;
        self.dirty = true;
    }

    fn toggle_glyphs(&mut self) {
        self.glyphs = match self.glyphs {
            Glyphs::Unicode => Glyphs::Ascii,
            Glyphs::Ascii => Glyphs::Unicode,
        };
        self.dirty = true;
    }

    fn adjust_modal_scroll(&mut self, delta: i8) {
        if self.onboarding.is_some() && self.modal.is_none() {
            self.onboarding_scroll = if delta.is_negative() {
                self.onboarding_scroll
                    .saturating_sub(usize::from(delta.unsigned_abs()))
            } else {
                self.onboarding_scroll
                    .saturating_add(usize::from(delta.unsigned_abs()))
            };
        } else if matches!(
            self.modal,
            Some(
                Modal::About | Modal::VerifyRunSheet | Modal::Verification | Modal::RuleExplanation
            )
        ) {
            self.modal_scroll = if delta.is_negative() {
                self.modal_scroll
                    .saturating_sub(usize::from(delta.unsigned_abs()))
            } else {
                self.modal_scroll
                    .saturating_add(usize::from(delta.unsigned_abs()))
            };
        }
        self.dirty = true;
    }

    fn update_active(&mut self, active: bool) -> Transition {
        if self.active == active {
            return Transition::default();
        }
        if !active {
            self.focus_epoch = self.focus_epoch.saturating_add(1);
        }
        self.active = active;
        self.dirty = true;
        if self.active {
            self.request_external_probe()
        } else {
            self.freshness = SnapshotFreshness::Unconfirmed;
            Transition::default()
        }
    }

    fn mark_external_probe_overdue(&mut self) {
        if self.active && self.freshness == SnapshotFreshness::Current {
            self.freshness = SnapshotFreshness::Unconfirmed;
            self.dirty = true;
        }
    }

    fn move_search_boundary(&mut self, end: bool) {
        if self.modal == Some(Modal::TestCatalogue) {
            self.test_query_cursor = if end { self.test_query.len() } else { 0 };
        } else if self.modal == Some(Modal::RuleSearch) {
            self.rule_query_cursor = if end { self.rule_query.len() } else { 0 };
        } else if self.modal == Some(Modal::Search) && self.route == Route::Diagnostics {
            self.diagnostic_query_cursor = if end { self.diagnostic_query.len() } else { 0 };
        } else if self.modal == Some(Modal::ReportDate) {
            self.report_as_of_cursor = if end { self.report_as_of.len() } else { 0 };
        } else if self.modal == Some(Modal::WorkDate) {
            self.work_as_of_cursor = if end { self.work_as_of_draft.len() } else { 0 };
        } else if self.modal == Some(Modal::Search)
            && self.route == Route::Reports
            && self.focus == Focus::Detail
        {
            self.report_row_query_cursor = if end { self.report_row_query.len() } else { 0 };
        } else if self.modal == Some(Modal::Search) && self.route == Route::Reports {
            self.report_kind_query_cursor = if end { self.report_kind_query.len() } else { 0 };
        } else if self.modal == Some(Modal::Search) && self.route == Route::Inbox {
            self.inbox_query_cursor = if end { self.inbox_query.len() } else { 0 };
        } else if self.modal == Some(Modal::Search) {
            self.task_query_cursor = if end { self.task_query.len() } else { 0 };
        } else {
            return;
        }
        self.dirty = true;
    }

    fn clear_filters(&mut self) {
        if self.route == Route::Diagnostics {
            self.diagnostic_filters.clear();
            self.rebuild_diagnostics();
        } else if self.route == Route::Inbox {
            self.inbox_state = Some(aplan_core::capture::CaptureState::Open);
            self.inbox_revisit = aplan_core::capture::RevisitFilter::Actionable;
            self.inbox_kind = None;
            self.inbox_label = None;
            self.reconcile_inbox_selection();
        } else {
            self.task_selector = Selector::default();
            if self.route == Route::Graph {
                self.rebuild_graph();
            } else if self.route == Route::Reports {
                self.rebuild_report();
            } else {
                self.rebuild_task_browser();
            }
        }
    }

    fn clear_narrowing(&mut self) {
        if self.route == Route::Diagnostics {
            self.diagnostic_query.clear();
            self.diagnostic_query_cursor = 0;
            self.diagnostic_filters.clear();
            self.rebuild_diagnostics();
        } else if self.route == Route::Inbox {
            self.inbox_query.clear();
            self.inbox_query_cursor = 0;
            self.inbox_state = Some(aplan_core::capture::CaptureState::Open);
            self.inbox_revisit = aplan_core::capture::RevisitFilter::Actionable;
            self.inbox_kind = None;
            self.inbox_label = None;
            self.reconcile_inbox_selection();
        } else {
            self.task_selector = Selector::default();
            self.task_scope = None;
            if self.route == Route::Graph {
                self.task_query.clear();
                self.task_query_cursor = 0;
                self.graph_request = GraphRequest::default();
                self.graph_between_anchor = None;
                self.graph_scroll_row = 0;
                self.graph_scroll_column = 0;
                self.graph_viewport_mode = GraphViewportMode::FollowSelection;
                self.rebuild_graph();
            } else if self.route == Route::Reports {
                self.report_kind_query.clear();
                self.report_kind_query_cursor = 0;
                self.report_row_query.clear();
                self.report_row_query_cursor = 0;
                self.report_as_of.clear();
                self.report_as_of_cursor = 0;
                self.rebuild_report();
            } else {
                self.task_query.clear();
                self.task_query_cursor = 0;
                self.rebuild_task_browser();
            }
        }
    }

    fn cycle_sort_or_group(&mut self) {
        if self.route == Route::Diagnostics {
            self.diagnostic_group = self.diagnostic_group.next();
            self.dirty = true;
        } else if self.route != Route::Inbox {
            self.task_sort = self.task_sort.next();
            self.rebuild_task_browser();
        }
    }

    fn select_route(&mut self, route: Route) {
        let previous_route = self.route;
        if route == Route::Graph && previous_route == Route::Graph {
            self.focus = Focus::Primary;
            self.modal = None;
            self.dirty = true;
            return;
        }
        if previous_route == Route::Graph
            && route != Route::Graph
            && let Some(origin) = self.graph_return_context.take()
        {
            let selected = self.selected_id.clone();
            self.restore_saved_graph_session(&origin);
            self.selected_id = selected;
        }
        if route == Route::Tasks {
            self.task_scope = None;
            self.rebuild_task_browser();
        }
        if route == Route::Diagnostics {
            self.rebuild_diagnostics();
        } else {
            self.diagnostic_return = None;
        }
        if route == Route::Graph {
            if self.graph_session == GraphSessionState::Started {
                self.refresh_graph(false);
                if let Some(subject) = self.graph_explorer_request.selected.as_ref() {
                    self.selected_id = Some(subject.id().to_string());
                    self.selection_state = SelectionState::Present;
                }
            } else {
                self.graph_session = GraphSessionState::Started;
                self.graph_explorer_request.location = GraphExplorerLocation::Plan;
                self.graph_explorer_request.selected = None;
                self.graph_explorer_request.selection_origin =
                    GraphExplorerSelectionOrigin::Automatic;
                self.graph_explorer_history.clear();
                self.rebuild_graph();
                if let Some(selection) = &self.graph_explorer.selection {
                    self.selected_id = Some(selection.subject.id().to_string());
                }
            }
            self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        }
        self.route = route;
        if route == Route::Reports {
            self.rebuild_report();
        }
        if route == Route::Inbox {
            self.reconcile_inbox_selection();
        }
        if previous_route == Route::Graph && route != Route::Graph {
            self.graph_relation_cursor = None;
        }
        if self.focus != Focus::Navigation {
            self.focus = Focus::Primary;
        }
        self.modal = None;
        self.dirty = true;
    }

    fn open_detail(&mut self) {
        if self.route == Route::Inbox && self.focus == Focus::Primary {
            if self.selected_capture().is_some() {
                self.focus = Focus::Detail;
                self.inspector_scroll = None;
                self.dirty = true;
            }
        } else if self.route == Route::Inbox && self.focus == Focus::Detail {
            let linked = self
                .selected_capture()
                .and_then(|capture| capture.promoted_to.clone());
            if let Some(task) = linked {
                if self.snapshot.inspectors.iter().any(|show| show.id == task) {
                    self.selected_id = Some(task);
                    self.selection_state = SelectionState::Present;
                    self.route = Route::Tasks;
                    self.focus = Focus::Detail;
                    self.inspector_section = InspectorSection::Summary;
                    self.inspector_selected = 0;
                    self.inspector_scroll = None;
                    self.inspector_history.clear();
                    self.rebuild_task_browser();
                    self.rebuild_work_projection();
                    self.dirty = true;
                } else {
                    self.show_notice(
                        "The promoted task is not present in the current loaded snapshot."
                            .to_string(),
                    );
                }
            }
        } else if self.route == Route::Graph && self.focus == Focus::Primary {
            self.drill_graph();
        } else if self.route == Route::Overview && self.focus == Focus::Primary {
            self.open_overview_metric();
        } else if self.route == Route::Diagnostics && self.focus == Focus::Primary {
            if self.selected_finding().is_some() {
                self.focus = Focus::Detail;
                self.inspector_scroll = None;
                self.dirty = true;
            }
        } else if self.route == Route::Reports && self.focus == Focus::Primary {
            self.focus = Focus::Detail;
            self.report_selected_row = self
                .report_selected_row
                .min(self.report_projection.rows.len().saturating_sub(1));
            self.dirty = true;
        } else if self.route == Route::Reports && self.focus == Focus::Detail {
            self.open_report_row();
        } else if self.focus == Focus::Detail {
            self.activate_inspector_item();
        } else if self.selected_task().is_some()
            || (self.route == Route::Graph && self.selected_inspector().is_some())
        {
            self.focus = Focus::Detail;
            self.inspector_section = InspectorSection::Summary;
            self.inspector_selected = 0;
            self.inspector_scroll = None;
            self.inspector_history.clear();
            self.dirty = true;
        }
    }

    fn open_overview_metric(&mut self) {
        let metric = OverviewMetric::ALL[self
            .overview_selected
            .min(OverviewMetric::ALL.len().saturating_sub(1))];
        if metric == OverviewMetric::Findings {
            self.route = Route::Diagnostics;
            self.focus = Focus::Primary;
            self.selected = 0;
            self.diagnostic_scope = DiagnosticScope::Check;
            self.diagnostic_query.clear();
            self.diagnostic_query_cursor = 0;
            self.diagnostic_filters.clear();
            self.selected_finding_id = self
                .snapshot
                .diagnostics
                .check
                .findings
                .first()
                .map(|finding| finding.identity.clone());
            self.dirty = true;
            return;
        }
        let ids = match metric {
            OverviewMetric::Cuts
            | OverviewMetric::Effort
            | OverviewMetric::Ready
            | OverviewMetric::Due
            | OverviewMetric::CriticalPath => self.overview_task_ids(metric),
            OverviewMetric::Findings => unreachable!("findings route handled above"),
        };
        self.task_query.clear();
        self.task_query_cursor = 0;
        self.task_selector = Selector::default();
        self.task_scope = Some(TaskScope::Overview(metric));
        self.selected_id = ids.first().cloned();
        self.route = Route::Tasks;
        self.focus = Focus::Primary;
        self.rebuild_task_browser();
    }

    fn close_layer(&mut self) -> Transition {
        if self
            .mutation
            .as_ref()
            .is_some_and(|mutation| mutation.committing)
        {
            self.show_notice(
                "The reviewed write is still running; quit waits for it to finish.".to_string(),
            );
            return Transition::default();
        }
        if self.mutation.take().is_some() {
            self.dirty = true;
            let cancel = self
                .effects
                .iter()
                .rev()
                .find(|record| {
                    record.state == EffectState::Queued
                        && record.request.kind == EffectKind::Mutation
                })
                .map(|record| record.request.id);
            return Transition {
                effect: None,
                cancel,
            };
        }
        if let Some(closed) = self.modal.take() {
            if closed == Modal::Palette {
                self.palette_query.clear();
            } else if closed == Modal::RunSheet {
                self.pending_check_overrides = CheckOverrides::default();
            } else if closed == Modal::VerifyRunSheet {
                self.pending_verification_mode = VerificationMode::ResolveOnly;
            } else if closed == Modal::TestCatalogue {
                self.test_catalogue = None;
                self.test_query.clear();
                self.test_query_cursor = 0;
                let cancel = self
                    .effects
                    .iter()
                    .rev()
                    .find(|record| {
                        record.state == EffectState::Queued
                            && record.request.kind == EffectKind::TestCatalogue
                    })
                    .map(|record| record.request.id);
                self.modal_selected = 0;
                self.modal_scroll = 0;
                self.dirty = true;
                return Transition {
                    effect: None,
                    cancel,
                };
            }
            self.modal_selected = 0;
            self.modal_scroll = 0;
        } else if self.focus == Focus::Detail {
            if self.route == Route::Inbox {
                self.focus = Focus::Primary;
                self.inspector_scroll = None;
                self.dirty = true;
                return Transition::default();
            }
            if self.route == Route::Tasks
                && let Some(back) = self.diagnostic_return.take()
            {
                self.route = Route::Diagnostics;
                self.diagnostic_scope = back.scope;
                self.selected_finding_id = Some(back.finding_identity);
                self.focus = Focus::Detail;
                self.rebuild_diagnostics();
                self.dirty = true;
                return Transition::default();
            }
            if let Some(previous) = self.inspector_history.pop() {
                self.selected_id = Some(previous.task_id);
                self.inspector_section = previous.section;
                self.inspector_selected = previous.selected;
                self.inspector_scroll = previous.scroll;
                self.selection_state = if self.selected_inspector().is_some() {
                    SelectionState::Present
                } else {
                    SelectionState::Removed
                };
                self.rebuild_task_browser();
                self.focus = Focus::Detail;
            } else {
                self.focus = Focus::Primary;
            }
        } else if self.route != Route::Overview {
            self.route = Route::Overview;
            self.focus = Focus::Primary;
        } else {
            self.quit = true;
        }
        self.dirty = true;
        Transition::default()
    }

    fn open_modal(&mut self, modal: Modal) {
        self.modal = Some(modal);
        if modal == Modal::Palette {
            self.palette_query.clear();
        }
        self.modal_selected = 0;
        self.modal_scroll = 0;
        self.dirty = true;
    }

    fn open_work_date(&mut self) {
        self.work_as_of_draft.clone_from(&self.work_as_of);
        self.work_as_of_cursor = self.work_as_of_draft.len();
        self.open_modal(Modal::WorkDate);
    }

    fn open_test_catalogue(&mut self) -> Transition {
        self.open_modal(Modal::TestCatalogue);
        self.test_query.clear();
        self.test_query_cursor = 0;
        self.test_catalogue = Some(TestCatalogueLayer::Loading);
        self.request_effect(EffectKind::TestCatalogue, CheckOverrides::default())
    }

    fn use_test_evidence(&mut self) -> Transition {
        let Some(choice) = self.selected_test_choice().cloned() else {
            self.show_notice("No configured test is selected.".to_string());
            return Transition::default();
        };
        if !choice.selectable {
            self.show_notice(choice.advice);
            return Transition::default();
        }
        let Some(task) = self.selected_inspector() else {
            self.show_notice(
                "Select a loaded task before inserting configured test evidence.".to_string(),
            );
            return Transition::default();
        };
        let action = if matches!(task.status.as_str(), "done" | "dropped") {
            MutationAction::AmendAdd
        } else {
            MutationAction::Close
        };
        self.modal = None;
        self.test_catalogue = None;
        self.test_query.clear();
        self.test_query_cursor = 0;
        let transition = self.open_mutation(action);
        if transition.effect.is_some() {
            self.pending_test_reference = Some(choice.reference);
        }
        transition
    }

    fn submit_work_date(&mut self) {
        // Blank is a semantic state, not merely an input spelling. Keep only
        // the canonical spelling so a whitespace-only draft continues to
        // follow later snapshot clocks.
        let normalized = self.work_as_of_draft.trim().to_string();
        let Some(epoch) =
            aplan_app::parse_work_as_of(&normalized, self.snapshot.overview.as_of_epoch_seconds)
        else {
            self.show_notice(
                "Invalid as-of value: enter RFC 3339 or signed epoch seconds; blank uses the snapshot clock."
                    .to_string(),
            );
            return;
        };
        self.work_as_of = normalized.clone();
        self.work_as_of_draft = normalized;
        self.work_as_of_cursor = self.work_as_of_draft.len();
        self.work_as_of_epoch_seconds = epoch;
        self.modal = None;
        self.modal_selected = 0;
        self.modal_scroll = 0;
        if self.task_scope == Some(TaskScope::Overview(OverviewMetric::Due)) {
            self.rebuild_task_browser();
        } else {
            self.rebuild_work_projection();
        }
        self.inspector_selected = 0;
        if self.route == Route::Inbox {
            self.inspector_scroll = None;
        }
        self.dirty = true;
    }

    fn toggle_why_traversal(&mut self) {
        self.why_traversal = self.why_traversal.toggled();
        self.inspector_selected = 0;
        self.inspector_scroll = None;
        self.rebuild_work_projection();
        self.dirty = true;
    }

    fn cycle_theme(&mut self) {
        self.theme = self.theme.next(self.color_capability);
        self.dirty = true;
    }

    fn search_editor(&mut self) -> (&mut String, &mut usize, (usize, usize, usize)) {
        let standard = (
            MAX_TASK_QUERY_CELLS,
            MAX_TASK_QUERY_GRAPHEMES,
            MAX_TASK_QUERY_BYTES,
        );
        match (self.modal, self.route, self.focus) {
            (Some(Modal::TestCatalogue), _, _) => {
                (&mut self.test_query, &mut self.test_query_cursor, standard)
            }
            (Some(Modal::RuleSearch), _, _) => (
                &mut self.rule_query,
                &mut self.rule_query_cursor,
                (24, 24, 64),
            ),
            (Some(Modal::Search), Route::Diagnostics, _) => (
                &mut self.diagnostic_query,
                &mut self.diagnostic_query_cursor,
                standard,
            ),
            (Some(Modal::ReportDate), _, _) => (
                &mut self.report_as_of,
                &mut self.report_as_of_cursor,
                (10, 10, 10),
            ),
            (Some(Modal::WorkDate), _, _) => (
                &mut self.work_as_of_draft,
                &mut self.work_as_of_cursor,
                (40, 40, 64),
            ),
            (Some(Modal::Search), Route::Reports, Focus::Detail) => (
                &mut self.report_row_query,
                &mut self.report_row_query_cursor,
                standard,
            ),
            (Some(Modal::Search), Route::Reports, _) => (
                &mut self.report_kind_query,
                &mut self.report_kind_query_cursor,
                standard,
            ),
            (Some(Modal::Search), Route::Inbox, _) => (
                &mut self.inbox_query,
                &mut self.inbox_query_cursor,
                standard,
            ),
            _ => (&mut self.task_query, &mut self.task_query_cursor, standard),
        }
    }

    fn edit_search(&mut self, input: Option<&str>, operation: SearchOperation) {
        if !matches!(
            self.modal,
            Some(
                Modal::Search
                    | Modal::RuleSearch
                    | Modal::ReportDate
                    | Modal::WorkDate
                    | Modal::TestCatalogue
            )
        ) {
            return;
        }
        // Preserve the semantic selection before editing changes the visible
        // row set. Reading the selected row after filtering would preserve an
        // index instead: if an earlier row disappears, that can silently move
        // selection to the next row even though the selected key still exists.
        let selected_report_row = (self.modal == Some(Modal::Search)
            && self.route == Route::Reports
            && self.focus == Focus::Detail)
            .then(|| {
                self.report_rows()
                    .get(self.report_selected_row)
                    .map(|row| row.key.clone())
            })
            .flatten();
        let (query, cursor, bounds) = self.search_editor();
        let changed = edit_search_text(query, cursor, operation, input, bounds);
        if changed {
            if self.modal == Some(Modal::TestCatalogue) {
                self.modal_selected = self
                    .modal_selected
                    .min(self.filtered_test_choices().len().saturating_sub(1));
                self.dirty = true;
            } else if self.modal == Some(Modal::Search) && self.route == Route::Diagnostics {
                self.rebuild_diagnostics();
            } else if self.modal == Some(Modal::Search) && self.route == Route::Graph {
                self.select_graph_search_match();
            } else if self.modal == Some(Modal::ReportDate) {
                self.rebuild_report();
            } else if self.modal == Some(Modal::Search) && self.route == Route::Reports {
                if self.focus == Focus::Detail {
                    self.reconcile_report_row(selected_report_row.as_deref());
                    self.dirty = true;
                } else {
                    self.reconcile_report_search();
                }
            } else if self.modal == Some(Modal::Search) && self.route == Route::Inbox {
                self.reconcile_inbox_selection();
            } else if self.modal == Some(Modal::Search) {
                self.rebuild_task_browser();
            } else {
                self.dirty = true;
            }
        }
    }

    fn move_search_cursor(&mut self, forward: bool) {
        let changed = if self.modal == Some(Modal::TestCatalogue) {
            text::move_cursor(&self.test_query, &mut self.test_query_cursor, forward)
        } else if self.modal == Some(Modal::RuleSearch) {
            text::move_cursor(&self.rule_query, &mut self.rule_query_cursor, forward)
        } else if self.modal == Some(Modal::Search) && self.route == Route::Diagnostics {
            text::move_cursor(
                &self.diagnostic_query,
                &mut self.diagnostic_query_cursor,
                forward,
            )
        } else if self.modal == Some(Modal::ReportDate) {
            text::move_cursor(&self.report_as_of, &mut self.report_as_of_cursor, forward)
        } else if self.modal == Some(Modal::WorkDate) {
            text::move_cursor(&self.work_as_of_draft, &mut self.work_as_of_cursor, forward)
        } else if self.modal == Some(Modal::Search)
            && self.route == Route::Reports
            && self.focus == Focus::Detail
        {
            text::move_cursor(
                &self.report_row_query,
                &mut self.report_row_query_cursor,
                forward,
            )
        } else if self.modal == Some(Modal::Search) && self.route == Route::Reports {
            text::move_cursor(
                &self.report_kind_query,
                &mut self.report_kind_query_cursor,
                forward,
            )
        } else if self.modal == Some(Modal::Search) && self.route == Route::Inbox {
            text::move_cursor(&self.inbox_query, &mut self.inbox_query_cursor, forward)
        } else if self.modal == Some(Modal::Search) {
            text::move_cursor(&self.task_query, &mut self.task_query_cursor, forward)
        } else {
            false
        };
        if changed {
            self.dirty = true;
        }
    }

    fn toggle_filter(&mut self, index: usize) {
        if self.modal != Some(Modal::Filters) {
            return;
        }
        if self.route == Route::Diagnostics {
            if let Some(filter) = self.diagnostic_filter_choices().get(index).cloned() {
                if let Some(existing) = self
                    .diagnostic_filters
                    .iter()
                    .position(|candidate| candidate == &filter)
                {
                    self.diagnostic_filters.remove(existing);
                } else {
                    self.diagnostic_filters.push(filter);
                }
                self.rebuild_diagnostics();
            }
        } else if self.route == Route::Inbox {
            if let Some(choice) = self.inbox_filter_choices().get(index).cloned() {
                match choice {
                    InboxFilterChoice::State(state) => self.inbox_state = state,
                    InboxFilterChoice::Revisit(revisit) => self.inbox_revisit = revisit,
                    InboxFilterChoice::Kind(kind) => self.inbox_kind = kind,
                    InboxFilterChoice::Label(label) => self.inbox_label = label,
                }
                self.reconcile_inbox_selection();
                self.modal_selected = self
                    .modal_selected
                    .min(self.inbox_filter_choices().len().saturating_sub(1));
            }
        } else if let Some(filter) = self.filter_choices().get(index).cloned() {
            filter.toggle(&mut self.task_selector);
            if self.route == Route::Graph {
                self.rebuild_graph();
            } else if self.route == Route::Reports {
                self.rebuild_report();
            } else {
                self.rebuild_task_browser();
            }
        }
    }

    pub(crate) fn filter_choices(&self) -> Vec<TaskFilter> {
        aplan_app::task_filter_choices(&self.snapshot.task_rows)
    }

    pub(crate) fn diagnostic_run(&self) -> &DiagnosticRun {
        match self.diagnostic_scope {
            DiagnosticScope::Check => &self.snapshot.diagnostics.check,
            DiagnosticScope::CheckDocs => &self.snapshot.diagnostics.documents,
            DiagnosticScope::Verify => {
                unreachable!("verification has its own identified result view")
            }
        }
    }

    pub(crate) fn diagnostic_findings(&self) -> Vec<&aplan_app::OverviewFinding> {
        let query = self.diagnostic_query.to_lowercase();
        self.diagnostic_run()
            .findings
            .iter()
            .filter(|finding| self.diagnostic_filter_matches(finding))
            .filter(|finding| {
                query.is_empty()
                    || finding.code.to_lowercase().contains(&query)
                    || finding.message.to_lowercase().contains(&query)
                    || finding.subject.to_lowercase().contains(&query)
                    || finding
                        .task
                        .as_deref()
                        .is_some_and(|value| value.to_lowercase().contains(&query))
                    || finding.file.as_ref().is_some_and(|value| {
                        value.to_string_lossy().to_lowercase().contains(&query)
                    })
                    || finding
                        .agent_instruction
                        .as_deref()
                        .is_some_and(|value| value.to_lowercase().contains(&query))
            })
            .collect()
    }

    pub(crate) fn selected_finding(&self) -> Option<&aplan_app::OverviewFinding> {
        let identity = self.selected_finding_id.as_deref()?;
        self.diagnostic_findings()
            .into_iter()
            .find(|finding| finding.identity == identity)
    }

    pub(crate) fn diagnostic_filter_choices(&self) -> Vec<DiagnosticFilter> {
        let mut choices = Vec::new();
        for finding in &self.diagnostic_run().findings {
            choices.push(DiagnosticFilter::Severity(finding.severity));
            choices.push(DiagnosticFilter::Rule(finding.code.clone()));
            if let Some(task) = &finding.task {
                choices.push(DiagnosticFilter::Task(task.clone()));
            }
            if let Some(file) = &finding.file {
                choices.push(DiagnosticFilter::File(file.clone()));
            }
        }
        choices.sort_by_cached_key(|filter| match filter {
            DiagnosticFilter::Severity(_) => (0, filter.label()),
            DiagnosticFilter::Rule(_) => (1, filter.label()),
            DiagnosticFilter::Task(_) => (2, filter.label()),
            DiagnosticFilter::File(_) => (3, filter.label()),
        });
        choices.dedup();
        choices
    }

    fn diagnostic_filter_matches(&self, finding: &aplan_app::OverviewFinding) -> bool {
        let mut has_severity = false;
        let mut has_rule = false;
        let mut has_task = false;
        let mut has_file = false;
        let mut severity_matches = false;
        let mut rule_matches = false;
        let mut task_matches = false;
        let mut file_matches = false;
        for filter in &self.diagnostic_filters {
            match filter {
                DiagnosticFilter::Severity(value) => {
                    has_severity = true;
                    severity_matches |= finding.severity == *value;
                }
                DiagnosticFilter::Rule(value) => {
                    has_rule = true;
                    rule_matches |= finding.code == *value;
                }
                DiagnosticFilter::Task(value) => {
                    has_task = true;
                    task_matches |= finding.task.as_ref() == Some(value);
                }
                DiagnosticFilter::File(value) => {
                    has_file = true;
                    file_matches |= finding.file.as_ref() == Some(value);
                }
            }
        }
        (!has_severity || severity_matches)
            && (!has_rule || rule_matches)
            && (!has_task || task_matches)
            && (!has_file || file_matches)
    }

    fn rebuild_diagnostics(&mut self) {
        let visible = self
            .diagnostic_findings()
            .into_iter()
            .map(|finding| finding.identity.clone())
            .collect::<Vec<_>>();
        if let Some(identity) = self.selected_finding_id.as_deref() {
            if let Some(index) = visible.iter().position(|candidate| candidate == identity) {
                self.selected = index;
            } else if let Some(first) = visible.first() {
                self.selected = 0;
                self.selected_finding_id = Some(first.clone());
            } else {
                self.selected = 0;
            }
        } else if let Some(first) = visible.first() {
            self.selected = 0;
            self.selected_finding_id = Some(first.clone());
        }
        self.selected = self.selected.min(visible.len().saturating_sub(1));
        if self.modal == Some(Modal::Filters) {
            self.modal_selected = self
                .modal_selected
                .min(self.diagnostic_filter_choices().len().saturating_sub(1));
        }
        self.dirty = true;
    }

    fn clear_active_query(&mut self) {
        if self.modal == Some(Modal::TestCatalogue) {
            self.test_query.clear();
            self.test_query_cursor = 0;
            self.modal_selected = 0;
            self.dirty = true;
        } else if self.modal == Some(Modal::RuleSearch) {
            self.rule_query.clear();
            self.rule_query_cursor = 0;
            self.dirty = true;
        } else if self.route == Route::Diagnostics {
            self.diagnostic_query.clear();
            self.diagnostic_query_cursor = 0;
            self.rebuild_diagnostics();
        } else if self.route == Route::Graph {
            self.task_query.clear();
            self.task_query_cursor = 0;
            self.dirty = true;
        } else if self.modal == Some(Modal::ReportDate) {
            self.report_as_of.clear();
            self.report_as_of_cursor = 0;
            self.rebuild_report();
        } else if self.modal == Some(Modal::WorkDate) {
            self.work_as_of_draft.clear();
            self.work_as_of_cursor = 0;
            self.dirty = true;
        } else if self.route == Route::Reports && self.focus == Focus::Detail {
            let selected_key = self
                .report_rows()
                .get(self.report_selected_row)
                .map(|row| row.key.clone());
            self.report_row_query.clear();
            self.report_row_query_cursor = 0;
            self.reconcile_report_row(selected_key.as_deref());
            self.dirty = true;
        } else if self.route == Route::Reports {
            self.report_kind_query.clear();
            self.report_kind_query_cursor = 0;
            self.reconcile_report_search();
        } else if self.route == Route::Inbox {
            self.inbox_query.clear();
            self.inbox_query_cursor = 0;
            self.reconcile_inbox_selection();
        } else {
            self.task_query.clear();
            self.task_query_cursor = 0;
            self.rebuild_task_browser();
        }
    }

    fn cycle_diagnostic_scope(&mut self) {
        if self.route != Route::Diagnostics {
            return;
        }
        self.diagnostic_scope = match self.diagnostic_scope {
            DiagnosticScope::Check => DiagnosticScope::CheckDocs,
            DiagnosticScope::CheckDocs | DiagnosticScope::Verify => DiagnosticScope::Check,
        };
        self.diagnostic_query.clear();
        self.diagnostic_query_cursor = 0;
        self.diagnostic_filters.clear();
        self.selected_finding_id = self
            .diagnostic_run()
            .findings
            .first()
            .map(|finding| finding.identity.clone());
        self.selected = 0;
        self.focus = Focus::Primary;
        self.dirty = true;
    }

    fn open_contextual_rule(&mut self) {
        let Some(code) = self.selected_finding().map(|finding| finding.code.clone()) else {
            self.show_notice("No finding is selected.".to_string());
            return;
        };
        match aplan_app::explain_rule(&code) {
            Ok(explanation) => {
                self.rule_explanation = Some(explanation);
                self.open_modal(Modal::RuleExplanation);
            }
            Err(error) => self.show_notice(error.to_string()),
        }
    }

    fn open_rule_search(&mut self) {
        self.rule_query.clear();
        self.rule_query_cursor = 0;
        self.rule_explanation = None;
        self.open_modal(Modal::RuleSearch);
    }

    fn submit_rule_search(&mut self) {
        match aplan_app::explain_rule(&self.rule_query) {
            Ok(explanation) => {
                self.rule_explanation = Some(explanation);
                self.modal = Some(Modal::RuleExplanation);
                self.dirty = true;
            }
            Err(error) => self.show_notice(error.to_string()),
        }
    }

    fn open_finding_task(&mut self) {
        let Some(finding) = self.selected_finding() else {
            self.show_notice("No finding is selected.".to_string());
            return;
        };
        let (Some(task), finding_identity) = (finding.task.clone(), finding.identity.clone())
        else {
            self.show_notice("This finding has no task subject.".to_string());
            return;
        };
        if !self.snapshot.inspectors.iter().any(|show| show.id == task) {
            self.show_notice(format!("Task {task} is missing or unloadable."));
            return;
        }
        self.diagnostic_return = Some(DiagnosticReturn {
            scope: self.diagnostic_scope,
            finding_identity,
        });
        self.selected_id = Some(task);
        self.selection_state = SelectionState::Present;
        self.task_scope = None;
        self.task_query.clear();
        self.task_query_cursor = 0;
        self.task_selector = Selector::default();
        self.rebuild_task_browser();
        self.route = Route::Tasks;
        self.focus = Focus::Detail;
        self.inspector_section = InspectorSection::Findings;
        self.inspector_selected = 0;
        self.inspector_scroll = None;
        self.dirty = true;
    }

    fn toggle_run_sheet(&mut self, index: usize) -> Transition {
        if self.modal != Some(Modal::RunSheet) {
            return Transition::default();
        }
        match index {
            0 => {
                self.pending_check_overrides.no_require_effort =
                    !self.pending_check_overrides.no_require_effort;
            }
            1 => {
                self.pending_check_overrides.no_traces = !self.pending_check_overrides.no_traces;
                if self.pending_check_overrides.no_traces {
                    self.pending_check_overrides.traces_root = None;
                }
            }
            2 => {
                self.pending_check_overrides.warning_policy =
                    match self.pending_check_overrides.warning_policy {
                        WarningPolicyOverride::Inherit => WarningPolicyOverride::Deny,
                        WarningPolicyOverride::Deny => WarningPolicyOverride::Allow,
                        WarningPolicyOverride::Allow => WarningPolicyOverride::Inherit,
                    };
            }
            3 => {
                let overrides = self.pending_check_overrides.clone();
                let transition = self.request_effect(EffectKind::Refresh, overrides);
                if transition.effect.is_some() {
                    self.pending_check_overrides = CheckOverrides::default();
                    self.modal = None;
                    self.modal_selected = 0;
                }
                return transition;
            }
            _ => return Transition::default(),
        }
        self.dirty = true;
        Transition::default()
    }

    fn toggle_verify_run_sheet(&mut self, index: usize) -> Transition {
        if self.modal != Some(Modal::VerifyRunSheet) {
            return Transition::default();
        }
        match index {
            0 => {
                if self.snapshot.verification_sheet.execute.is_available() {
                    self.pending_verification_mode = self.pending_verification_mode.toggled();
                    self.dirty = true;
                } else {
                    self.show_notice(
                        "Execute is unavailable: `.aplan.yml` configures no test runner."
                            .to_string(),
                    );
                }
                Transition::default()
            }
            1 => {
                let transition =
                    self.request_verification(self.pending_verification_mode.execute());
                if transition.effect.is_some() {
                    self.pending_verification_mode = VerificationMode::ResolveOnly;
                    self.modal = None;
                    self.modal_selected = 0;
                }
                transition
            }
            _ => Transition::default(),
        }
    }

    fn rebuild_graph(&mut self) {
        if self.graph_return_context.is_none() {
            self.graph_request.selector = self.task_selector.clone();
        }
        self.graph_projection = self.snapshot.graph.project(&self.graph_request);
        self.graph_continuation_selected = 0;
        self.graph_window_request = GraphWindowRequest {
            preferred_anchor: self.selected_id.clone(),
            ..GraphWindowRequest::default()
        };
        self.graph_window = self.graph_projection.window(&self.graph_window_request);
        self.graph_scene = GraphScene::layout(&self.graph_window.view, aplan_output::MAX_WIDTH);
        self.rebuild_layered_scene();
        self.recover_invalid_graph_location();
        self.graph_scroll_row = 0;
        self.graph_scroll_column = 0;
        self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        self.dirty = true;
    }

    fn refresh_graph(&mut self, identity_changed: bool) {
        if self.graph_return_context.is_none() {
            self.graph_request.selector = self.task_selector.clone();
        }
        self.graph_projection = self.snapshot.graph.project(&self.graph_request);
        let graph_identity_changed =
            self.graph_window.input.sha256 != self.graph_projection.input.sha256;
        if identity_changed || graph_identity_changed {
            self.graph_explorer_history.clear();
            self.graph_explorer_request.continuation = None;
            self.graph_continuation_selected = 0;
            self.graph_window_request = GraphWindowRequest {
                preferred_anchor: self.selected_id.clone(),
                ..GraphWindowRequest::default()
            };
            self.graph_scroll_row = 0;
            self.graph_scroll_column = 0;
            self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        }
        let mut window = self.graph_projection.window(&self.graph_window_request);
        if matches!(window.state, GraphWindowState::Failed(_)) {
            // An unchanged identity should make every retained cursor valid,
            // but a malformed or obsolete in-memory request must not strand
            // the destination. Fall back to an ordinary anchored window.
            self.graph_continuation_selected = 0;
            self.graph_window_request = GraphWindowRequest {
                preferred_anchor: self.selected_id.clone(),
                ..GraphWindowRequest::default()
            };
            window = self.graph_projection.window(&self.graph_window_request);
        }
        self.graph_window = window;
        self.graph_scene = GraphScene::layout(&self.graph_window.view, aplan_output::MAX_WIDTH);
        self.rebuild_layered_scene();
        self.recover_invalid_graph_location();
        self.graph_continuation_selected = self
            .graph_continuation_selected
            .min(self.graph_explorer.boundaries.len().saturating_sub(1));
        self.dirty = true;
    }

    fn recover_invalid_graph_location(&mut self) {
        if matches!(
            self.graph_explorer.state,
            aplan_app::GraphExplorerState::UnknownLocation(_)
        ) {
            self.graph_explorer_request.location = GraphExplorerLocation::Plan;
            self.graph_explorer_history.clear();
            self.rebuild_layered_scene();
        }
    }

    fn restore_previous_graph_context(&mut self) {
        if self.restore_previous_graph_level() {
            return;
        }
        self.return_graph_origin();
    }

    fn move_graph_continuation(&mut self, forward: bool) {
        if self.route != Route::Graph || self.graph_explorer.boundaries.is_empty() {
            return;
        }
        let count = self.graph_explorer.boundaries.len();
        self.graph_continuation_selected = if forward {
            (self.graph_continuation_selected + 1) % count
        } else {
            self.graph_continuation_selected
                .checked_sub(1)
                .unwrap_or(count - 1)
        };
        self.dirty = true;
    }

    fn open_graph_continuation(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        let Some(continuation) = self.selected_graph_continuation().cloned() else {
            self.show_notice("This graph window has no omitted continuation.".to_string());
            return;
        };
        self.push_graph_explorer_history();
        let boundary = continuation.cursor.key.boundary.clone();
        self.graph_explorer_request.continuation = Some(continuation.cursor);
        self.graph_explorer_request.selected = Some(GraphExplorerIdentity::Task(boundary.clone()));
        self.graph_explorer_request.selection_origin = GraphExplorerSelectionOrigin::Contextual;
        self.rebuild_layered_scene_with_prior(true);
        self.selected_id = Some(boundary);
        self.graph_continuation_selected = 0;
        self.graph_scroll_row = 0;
        self.graph_scroll_column = 0;
        self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        self.dirty = true;
    }

    fn adjust_graph_depth(&mut self, expand: bool) {
        if self.route != Route::Graph {
            return;
        }
        let Some(selected) = self.selected_id.clone() else {
            self.show_notice(
                "Select a graph task before changing neighbourhood depth.".to_string(),
            );
            return;
        };
        let (depth, live) = match &self.graph_request.scope {
            GraphScope::Neighbourhood { depth, live, .. } => (*depth, *live),
            _ if expand => (Some(0), false),
            _ => {
                self.show_notice(
                    "Contract depth is available in a neighbourhood scope; press g to reach one."
                        .to_string(),
                );
                return;
            }
        };
        let next = if expand {
            if let Some(value) = depth {
                Some(value.saturating_add(1).max(1))
            } else {
                self.show_notice("This neighbourhood already includes every depth.".to_string());
                return;
            }
        } else {
            match depth {
                Some(value) if value > 1 => Some(value - 1),
                Some(_) => {
                    self.show_notice(
                        "Neighbourhood depth is already at its minimum of 1.".to_string(),
                    );
                    return;
                }
                None => {
                    self.show_notice(
                        "Choose a finite neighbourhood with g before contracting its depth."
                            .to_string(),
                    );
                    return;
                }
            }
        };
        self.graph_request.scope = GraphScope::Neighbourhood {
            task: selected,
            depth: next,
            live,
        };
        self.graph_between_anchor = None;
        self.rebuild_graph();
    }

    fn select_graph_search_match(&mut self) {
        let response = aplan_app::browse_tasks(
            &self.snapshot.task_rows,
            &TaskBrowserRequest {
                query: self.task_query.clone(),
                selector: Selector::default(),
                sort: TaskSort::Id,
            },
        );
        if let Some(id) = response
            .rows
            .iter()
            .map(|row| &row.task.id)
            .find(|id| self.graph_projection.contains(id))
            .cloned()
        {
            self.select_graph_task(id);
        } else {
            self.dirty = true;
        }
    }

    fn cycle_graph_scope(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        let Some(selected) = self.selected_id.clone() else {
            self.show_notice("Select a graph task before using a task-centred scope.".to_string());
            return;
        };
        self.graph_request.scope = match &self.graph_request.scope {
            GraphScope::Whole => GraphScope::Upstream(selected),
            GraphScope::Upstream(_) => GraphScope::Downstream(selected),
            GraphScope::Downstream(_) => GraphScope::Unreachable,
            GraphScope::Unreachable => GraphScope::Neighbourhood {
                task: selected,
                depth: Some(1),
                live: false,
            },
            GraphScope::Neighbourhood { depth: Some(1), .. } => GraphScope::Neighbourhood {
                task: selected,
                depth: Some(2),
                live: false,
            },
            GraphScope::Neighbourhood { depth: Some(2), .. } => GraphScope::Neighbourhood {
                task: selected,
                depth: None,
                live: false,
            },
            GraphScope::Neighbourhood {
                depth: None,
                live: false,
                ..
            } => GraphScope::Neighbourhood {
                task: selected,
                depth: None,
                live: true,
            },
            GraphScope::Neighbourhood { .. } => GraphScope::CriticalPath,
            GraphScope::CriticalPath | GraphScope::Between(_, _) => GraphScope::Whole,
        };
        self.graph_between_anchor = None;
        self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        self.rebuild_graph();
    }

    fn set_graph_between(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        let Some(selected) = self.selected_id.clone() else {
            self.show_notice("Select a graph task before setting a between scope.".to_string());
            return;
        };
        if let Some(from) = self.graph_between_anchor.take() {
            self.graph_request.scope = GraphScope::Between(from, selected);
            self.graph_viewport_mode = GraphViewportMode::FollowSelection;
            self.rebuild_graph();
        } else {
            self.graph_between_anchor = Some(selected.clone());
            self.show_notice(format!(
                "Between start is {selected}; select the destination and press b again."
            ));
        }
    }

    fn toggle_graph_membership(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        self.graph_request.membership = !self.graph_request.membership;
        self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        self.rebuild_graph();
    }

    fn cycle_graph_density(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        self.graph_density = self.graph_density.next();
        let resolved = resolve_density(
            self.graph_density,
            self.graph_resolved_density,
            self.columns,
            self.rows,
        );
        if resolved != self.graph_resolved_density {
            self.graph_resolved_density = resolved;
            self.rebuild_layered_scene_with_prior(false);
        }
        self.dirty = true;
    }

    fn cycle_graph_ordering(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        self.graph_ordering = self.graph_ordering.next();
        self.rebuild_layered_scene_with_prior(true);
        self.show_notice(format!(
            "Graph peer order: {} ({}).",
            self.graph_ordering.label(),
            self.graph_ordering.layout_cost()
        ));
    }

    fn cycle_graph_orientation(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        self.graph_orientation = match self.graph_orientation {
            CellSceneOrientation::Auto => CellSceneOrientation::LeftToRight,
            CellSceneOrientation::LeftToRight => CellSceneOrientation::TopToBottom,
            CellSceneOrientation::TopToBottom => CellSceneOrientation::Auto,
        };
        self.rebuild_layered_scene_with_prior(true);
        self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        self.show_notice(format!(
            "Graph direction: {}.",
            self.graph_orientation_label()
        ));
    }

    pub(crate) const fn graph_orientation_label(&self) -> &'static str {
        match self.graph_orientation {
            CellSceneOrientation::Auto => "auto",
            CellSceneOrientation::LeftToRight => "left-to-right",
            CellSceneOrientation::TopToBottom => "top-to-bottom",
        }
    }

    fn toggle_graph_relations_only(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        self.graph_edge_presentation = self.graph_edge_presentation.toggled();
        self.dirty = true;
    }

    fn follow_graph_relation(&mut self, blocker: bool) {
        if self.route != Route::Graph {
            return;
        }
        let Some(current) = self.graph_relation_subject().cloned() else {
            self.show_notice("No graph task is selected.".to_string());
            return;
        };
        let selected = self
            .graph_relation_cursor
            .as_ref()
            .filter(|cursor| {
                cursor.incoming == blocker && cursor.count > 1 && cursor.last_target == current
            })
            .map_or_else(|| current.clone(), |cursor| cursor.subject.clone());
        let relations = self.graph_relations(&selected, blocker);
        if relations.is_empty() {
            self.show_notice(format!(
                "The selected task has no direct {} in this view.",
                if blocker { "blocker" } else { "dependent" }
            ));
            return;
        }
        let previous = self
            .graph_relation_cursor
            .as_ref()
            .filter(|cursor| cursor.subject == selected && cursor.incoming == blocker)
            .map_or(usize::MAX, |cursor| cursor.index);
        let index = if previous == usize::MAX {
            0
        } else {
            (previous + 1) % relations.len()
        };
        let target = relations[index].clone();
        self.select_graph_identity_or_boundary(target.clone());
        self.graph_relation_cursor = Some(GraphRelationCursor {
            subject: selected,
            incoming: blocker,
            index,
            count: relations.len(),
            last_target: target,
        });
        self.show_notice(format!(
            "{} {}/{}: {}.",
            if blocker { "Blocker" } else { "Dependent" },
            index + 1,
            relations.len(),
            relations[index].id()
        ));
    }

    fn graph_relations(
        &self,
        selected: &GraphExplorerIdentity,
        incoming: bool,
    ) -> Vec<GraphExplorerIdentity> {
        let mut relations: Vec<GraphExplorerIdentity> = if matches!(
            self.graph_explorer.location,
            GraphExplorerLocation::Milestone(_) | GraphExplorerLocation::Task(_)
        ) && matches!(
            selected,
            GraphExplorerIdentity::Task(_)
        ) {
            let raw = if incoming {
                self.graph_projection.blockers(selected.id())
            } else {
                self.graph_projection.dependents(selected.id())
            };
            raw.into_iter()
                .map(|relation| GraphExplorerIdentity::Task(relation.id))
                .collect()
        } else {
            self.graph_explorer
                .graph
                .edges
                .iter()
                .filter_map(|edge| {
                    if incoming && edge.to == *selected {
                        Some(edge.from.clone())
                    } else if !incoming && edge.from == *selected {
                        Some(edge.to.clone())
                    } else {
                        None
                    }
                })
                .chain(
                    self.graph_explorer
                        .phase_context
                        .iter()
                        .flat_map(|context| &context.boundary_edges)
                        .filter_map(|edge| {
                            let direction_matches = matches!(
                                (incoming, edge.direction),
                                (true, aplan_app::GraphExplorerBoundaryDirection::Incoming)
                                    | (false, aplan_app::GraphExplorerBoundaryDirection::Outgoing)
                            );
                            (edge.visible == *selected && direction_matches)
                                .then(|| edge.remote.clone())
                        }),
                )
                .collect()
        };
        relations.sort();
        relations.dedup();
        relations
    }

    pub(crate) fn graph_relation_count(&self, incoming: bool) -> usize {
        self.graph_relation_subject()
            .map_or(0, |selected| self.graph_relations(selected, incoming).len())
    }

    fn graph_relation_subject(&self) -> Option<&GraphExplorerIdentity> {
        self.selected_id.as_ref()?;
        let selection = self.graph_explorer.selection.as_ref()?;
        if matches!(
            self.graph_explorer.location,
            GraphExplorerLocation::Milestone(_) | GraphExplorerLocation::Task(_)
        ) && matches!(selection.subject, GraphExplorerIdentity::Task(_))
        {
            Some(&selection.subject)
        } else {
            Some(&selection.represented_by)
        }
    }

    fn graph_selected_identity(&self) -> Option<&GraphExplorerIdentity> {
        self.selected_id.as_ref()?;
        self.graph_explorer
            .selection
            .as_ref()
            .map(|selection| &selection.represented_by)
    }

    fn move_graph_spatial(&mut self, direction: SpatialDirection) {
        if self.route != Route::Graph {
            return;
        }
        let Some(selected) = self.graph_selected_identity().cloned() else {
            return;
        };
        let target = self
            .graph_cell_scene()
            .and_then(|scene| graph_navigation::spatial_target(scene, &selected, direction))
            .or_else(|| self.fallback_spatial_target(&selected, direction));
        let Some(target) = target else {
            self.show_notice("No graph node lies in that semantic direction.".to_string());
            return;
        };
        self.select_graph_identity(target);
    }

    fn fallback_spatial_target(
        &self,
        selected: &GraphExplorerIdentity,
        direction: SpatialDirection,
    ) -> Option<GraphExplorerIdentity> {
        self.graph_scene_fallback()?;
        let along_flow = matches!(direction, SpatialDirection::Left | SpatialDirection::Right);
        if along_flow {
            let successor = direction == SpatialDirection::Right;
            let mut related: Vec<_> = self
                .graph_explorer
                .graph
                .edges
                .iter()
                .filter_map(|edge| {
                    if successor && edge.from == *selected {
                        Some(edge.to.clone())
                    } else if !successor && edge.to == *selected {
                        Some(edge.from.clone())
                    } else {
                        None
                    }
                })
                .collect();
            related.sort();
            related.dedup();
            if let Some(target) = related.first() {
                return Some(target.clone());
            }
        }
        let identities = self.fallback_graph_identities();
        let index = identities
            .iter()
            .position(|identity| identity == selected)?;
        match direction {
            SpatialDirection::Up => index
                .checked_sub(1)
                .and_then(|index| identities.get(index).cloned()),
            SpatialDirection::Down => identities.get(index + 1).cloned(),
            SpatialDirection::Left | SpatialDirection::Right => None,
        }
    }

    fn graph_layer_jump_candidates(
        &self,
        scene: Option<&CellScene>,
        forward: bool,
    ) -> Vec<GraphExplorerIdentity> {
        let Some(scene) = scene else {
            return Vec::new();
        };
        let Some(current_rank) = self.graph_selected_identity().and_then(|identity| {
            scene
                .nodes
                .iter()
                .find(|node| &node.identity == identity)
                .and_then(|node| node.rank)
        }) else {
            return Vec::new();
        };
        let target_rank = scene
            .nodes
            .iter()
            .filter_map(|node| node.rank)
            .filter(|candidate| {
                if forward {
                    *candidate > current_rank
                } else {
                    *candidate < current_rank
                }
            })
            .min_by_key(|candidate| (candidate - current_rank).unsigned_abs());
        scene
            .nodes
            .iter()
            .filter(|node| node.rank == target_rank)
            .map(|node| node.identity.clone())
            .collect()
    }

    fn graph_jump_candidates(
        &self,
        jump: GraphJump,
        scene: Option<&CellScene>,
    ) -> Vec<GraphExplorerIdentity> {
        match jump {
            GraphJump::PreviousLayer | GraphJump::NextLayer => {
                self.graph_layer_jump_candidates(scene, jump == GraphJump::NextLayer)
            }
            GraphJump::Source | GraphJump::Sink => self
                .graph_explorer
                .graph
                .nodes
                .iter()
                .filter(|node| {
                    !self.graph_explorer.graph.edges.iter().any(|edge| {
                        if jump == GraphJump::Source {
                            edge.to == node.identity
                        } else {
                            edge.from == node.identity
                        }
                    })
                })
                .map(|node| node.identity.clone())
                .collect(),
            GraphJump::Critical => self
                .graph_explorer
                .graph
                .nodes
                .iter()
                .filter(|node| {
                    node.represented_ids
                        .iter()
                        .any(|id| self.graph_explorer.critical_path.contains(id))
                })
                .map(|node| node.identity.clone())
                .collect(),
            GraphJump::Milestone => self
                .graph_explorer
                .graph
                .nodes
                .iter()
                .filter(|node| matches!(node.identity, GraphExplorerIdentity::Milestone(_)))
                .map(|node| node.identity.clone())
                .collect(),
            GraphJump::Finding => {
                let finding_tasks: BTreeSet<&str> = self
                    .snapshot
                    .diagnostics
                    .check
                    .findings
                    .iter()
                    .chain(&self.snapshot.diagnostics.documents.findings)
                    .filter_map(|finding| finding.task.as_deref())
                    .collect();
                self.graph_explorer
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| {
                        node.represented_ids
                            .iter()
                            .any(|id| finding_tasks.contains(id.as_str()))
                    })
                    .map(|node| node.identity.clone())
                    .collect()
            }
            GraphJump::PreviousPhase | GraphJump::NextPhase => unreachable!(),
        }
    }

    fn jump_graph(&mut self, jump: GraphJump) {
        if self.route != Route::Graph {
            return;
        }
        if matches!(jump, GraphJump::PreviousPhase | GraphJump::NextPhase) {
            self.jump_graph_phase(jump == GraphJump::NextPhase);
            return;
        }
        let scene = self.graph_cell_scene();
        let selected = self.graph_selected_identity();
        let mut candidates = self.graph_jump_candidates(jump, scene);
        candidates.sort_by_key(|identity| {
            scene
                .and_then(|scene| scene.nodes.iter().find(|node| node.identity == *identity))
                .map_or((i32::MAX, u32::MAX, identity.clone()), |node| {
                    (node.rank.unwrap_or(i32::MAX), node.order, identity.clone())
                })
        });
        candidates.dedup();
        let Some(target) = cycle_after(&candidates, selected) else {
            self.show_notice(format!("No {} is visible.", jump.label()));
            return;
        };
        self.select_graph_identity(target);
    }

    fn jump_graph_phase(&mut self, forward: bool) {
        if let Some(context) = &self.graph_explorer.phase_context {
            let target = if forward {
                context.next
            } else {
                context.previous
            };
            if let Some(phase) = target {
                self.open_graph_location(GraphExplorerLocation::Phase(phase), true);
            } else {
                self.show_notice("There is no adjacent phase in that direction.".to_string());
            }
            return;
        }
        let phases: Vec<u32> = self
            .graph_explorer
            .graph
            .phase_bands
            .iter()
            .map(|band| band.phase)
            .collect();
        let current = match self.graph_explorer.location {
            GraphExplorerLocation::Phase(phase) => Some(phase),
            _ => self.graph_selected_identity().and_then(|selected| {
                self.graph_explorer
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.identity == *selected)
                    .map(|node| node.phase)
            }),
        };
        let target = current
            .and_then(|phase| phases.iter().position(|candidate| *candidate == phase))
            .and_then(|index| {
                if forward {
                    phases.get(index + 1).copied()
                } else {
                    index
                        .checked_sub(1)
                        .and_then(|index| phases.get(index).copied())
                }
            })
            .or_else(|| phases.first().copied());
        if let Some(phase) = target {
            self.open_graph_location(GraphExplorerLocation::Phase(phase), true);
        } else {
            self.show_notice("There is no adjacent phase in that direction.".to_string());
        }
    }

    fn select_graph_task(&mut self, id: String) {
        if !self.graph_projection.contains(&id) {
            return;
        }
        let represented = self.graph_explorer.graph.nodes.iter().any(|node| {
            node.identity.id() == id
                || node
                    .represented_ids
                    .iter()
                    .any(|represented| represented == &id)
        });
        if !represented {
            let identity = GraphExplorerIdentity::Task(id.clone());
            self.open_graph_location(GraphExplorerLocation::Task(id), true);
            self.select_graph_identity(identity);
            return;
        }
        self.select_graph_identity(GraphExplorerIdentity::Task(id));
    }

    fn select_graph_identity_or_boundary(&mut self, identity: GraphExplorerIdentity) {
        if self
            .graph_explorer
            .graph
            .nodes
            .iter()
            .any(|node| node.identity == identity)
        {
            self.select_graph_identity(identity);
            return;
        }
        if let GraphExplorerIdentity::Task(id) = &identity
            && matches!(
                self.graph_explorer.location,
                GraphExplorerLocation::Milestone(_) | GraphExplorerLocation::Task(_)
            )
        {
            self.open_graph_location(GraphExplorerLocation::Task(id.clone()), true);
            self.select_graph_identity(identity);
            return;
        }
        let phase = self
            .snapshot
            .task_rows
            .iter()
            .find(|row| row.id == identity.id())
            .map(|row| row.phase);
        if let Some(phase) = phase {
            self.open_graph_location(GraphExplorerLocation::Phase(phase), true);
            self.select_graph_identity(identity);
        }
    }

    fn select_graph_identity(&mut self, identity: GraphExplorerIdentity) {
        let id = identity.id().to_string();
        self.graph_explorer_request.selection_origin = GraphExplorerSelectionOrigin::Contextual;
        self.selected_id = Some(id);
        self.selection_state = SelectionState::Present;
        self.selected = self
            .selected_id
            .as_deref()
            .and_then(|selected| {
                self.snapshot
                    .task_rows
                    .iter()
                    .position(|row| row.id == selected)
            })
            .unwrap_or(0);
        self.focus = Focus::Primary;
        self.inspector_history.clear();
        self.inspector_section = InspectorSection::Summary;
        self.inspector_selected = 0;
        self.inspector_scroll = None;
        self.rebuild_work_projection();
        let selection_independent_level = matches!(
            self.graph_explorer.location,
            GraphExplorerLocation::Plan | GraphExplorerLocation::Phase(_)
        );
        let reused_projection =
            selection_independent_level && self.graph_explorer.select_contextual(&identity);
        self.graph_explorer_request.selected = Some(identity);
        if reused_projection {
            self.rebuild_current_cell_scene(true);
        } else {
            self.rebuild_layered_scene();
        }
        self.graph_relation_cursor = None;
        self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        self.dirty = true;
    }

    fn graph_explorer_location_state(&self) -> GraphExplorerLocationState {
        GraphExplorerLocationState {
            location: self.graph_explorer_request.location.clone(),
            selected: self.graph_explorer_request.selected.clone(),
            continuation: self.graph_explorer_request.continuation.clone(),
            scroll_row: self.graph_scroll_row,
            scroll_column: self.graph_scroll_column,
            viewport_mode: self.graph_viewport_mode,
        }
    }

    fn push_graph_explorer_history(&mut self) {
        if self.graph_explorer_history.len() == MAX_GRAPH_EXPLORER_HISTORY {
            self.graph_explorer_history.remove(0);
        }
        self.graph_explorer_history
            .push(self.graph_explorer_location_state());
    }

    fn open_graph_location(&mut self, location: GraphExplorerLocation, remember: bool) {
        if self.route != Route::Graph {
            return;
        }
        if self.graph_explorer_request.location == location {
            return;
        }
        if remember {
            self.push_graph_explorer_history();
        }
        self.graph_explorer_request.location = location;
        self.graph_explorer_request.continuation = None;
        self.graph_explorer_request.selection_origin = GraphExplorerSelectionOrigin::Contextual;
        self.rebuild_layered_scene_with_prior(true);
        if matches!(
            self.graph_explorer.state,
            aplan_app::GraphExplorerState::UnknownLocation(_)
        ) {
            if remember {
                self.graph_explorer_history.pop();
            }
            self.graph_explorer_request.location = GraphExplorerLocation::Plan;
            self.rebuild_layered_scene_with_prior(true);
            self.show_notice("That graph location no longer exists; opened Overview.".to_string());
        }
        if let Some(selection) = &self.graph_explorer.selection {
            self.selected_id = Some(selection.subject.id().to_string());
        }
        self.graph_scroll_row = 0;
        self.graph_scroll_column = 0;
        self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        self.graph_relation_cursor = None;
        self.dirty = true;
    }

    fn restore_previous_graph_level(&mut self) -> bool {
        let Some(previous) = self.graph_explorer_history.pop() else {
            return false;
        };
        self.graph_explorer_request.location = previous.location;
        self.graph_explorer_request.selected = previous.selected;
        self.graph_explorer_request.continuation = previous.continuation;
        self.graph_explorer_request.selection_origin = GraphExplorerSelectionOrigin::Remembered;
        self.rebuild_layered_scene_with_prior(true);
        self.graph_scroll_row = previous.scroll_row;
        self.graph_scroll_column = previous.scroll_column;
        self.graph_viewport_mode = previous.viewport_mode;
        if let Some(selection) = &self.graph_explorer.selection {
            self.selected_id = Some(selection.subject.id().to_string());
        }
        self.graph_relation_cursor = None;
        self.dirty = true;
        true
    }

    fn drill_graph(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        let selected = self.graph_explorer.selection.clone();
        let target = match (&self.graph_explorer.location, selected.as_ref()) {
            (GraphExplorerLocation::Plan, Some(selection)) => self
                .graph_explorer
                .graph
                .nodes
                .iter()
                .find(|node| node.identity == selection.represented_by)
                .map(|node| GraphExplorerLocation::Phase(node.phase)),
            (GraphExplorerLocation::Phase(_), Some(selection)) => {
                Some(match &selection.represented_by {
                    GraphExplorerIdentity::Milestone(id) => {
                        GraphExplorerLocation::Milestone(id.clone())
                    }
                    GraphExplorerIdentity::Task(id) => GraphExplorerLocation::Task(id.clone()),
                })
            }
            (GraphExplorerLocation::Milestone(id), Some(selection)) => {
                let member = match &selection.subject {
                    GraphExplorerIdentity::Task(task) if task != id => Some(task.clone()),
                    GraphExplorerIdentity::Milestone(_) | GraphExplorerIdentity::Task(_) => {
                        self.graph_explorer.graph.nodes.iter().find_map(|node| {
                            matches!(node.identity, GraphExplorerIdentity::Task(_))
                                .then(|| node.identity.id().to_string())
                        })
                    }
                };
                member.map(GraphExplorerLocation::Task)
            }
            (GraphExplorerLocation::Task(_), _) => {
                self.focus = Focus::Detail;
                self.inspector_section = InspectorSection::Summary;
                self.inspector_selected = 0;
                self.inspector_scroll = None;
                self.inspector_history.clear();
                self.dirty = true;
                return;
            }
            (_, None) => None,
        };
        if let Some(target) = target {
            self.open_graph_location(target, true);
        } else {
            self.show_notice("The selected graph subject has no deeper level.".to_string());
        }
    }

    fn open_graph_overview(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        if self.graph_explorer_request.location == GraphExplorerLocation::Plan {
            self.graph_viewport_mode = GraphViewportMode::FollowSelection;
            self.dirty = true;
            return;
        }
        self.open_graph_location(GraphExplorerLocation::Plan, true);
    }

    fn fit_graph(&mut self) {
        if self.route != Route::Graph {
            return;
        }
        let relayout = self.graph_resolved_density != ResolvedGraphDensity::Compact;
        self.graph_density = GraphDensity::Compact;
        self.graph_resolved_density = ResolvedGraphDensity::Compact;
        if relayout {
            self.rebuild_layered_scene_with_prior(false);
        }
        self.graph_scroll_row = 0;
        self.graph_scroll_column = 0;
        self.graph_viewport_mode = GraphViewportMode::Manual;
        self.show_notice(
            "Fit uses compact cards and the scene origin; Home focuses the selection and v changes the density pin."
                .to_string(),
        );
        self.dirty = true;
    }

    fn open_graph_at(&mut self, id: String) {
        if !self.snapshot.task_rows.iter().any(|row| row.id == id) {
            self.show_notice(format!("Task {id} is not in the current plan snapshot."));
            return;
        }
        let origin = GraphReturnContext {
            route: self.route,
            selected_id: self.selected_id.clone(),
            graph_request: self.graph_request.clone(),
            explorer: self.graph_explorer_location_state(),
            ordering: self.graph_ordering,
            orientation: self.graph_orientation,
            density: self.graph_density,
            resolved_density: self.graph_resolved_density,
            edge_presentation: self.graph_edge_presentation,
        };
        let narrowed = !self.graph_projection.contains(&id);
        if narrowed {
            self.graph_request.scope = GraphScope::Whole;
            self.graph_request.selector = Selector::default();
            self.graph_projection = self.snapshot.graph.project(&self.graph_request);
        }
        let milestone = self
            .snapshot
            .task_rows
            .iter()
            .find(|row| row.id == id)
            .is_some_and(|row| {
                row.tags
                    .iter()
                    .any(|tag| tag == aplan_core::task::MILESTONE_TAG)
            });
        let identity = if milestone {
            GraphExplorerIdentity::Milestone(id.clone())
        } else {
            GraphExplorerIdentity::Task(id.clone())
        };
        let location = if milestone {
            GraphExplorerLocation::Milestone(id.clone())
        } else {
            GraphExplorerLocation::Task(id.clone())
        };
        self.route = Route::Graph;
        self.focus = Focus::Primary;
        self.modal = None;
        self.graph_session = GraphSessionState::Started;
        self.graph_return_context = Some(origin);
        self.graph_explorer_history.clear();
        self.graph_explorer_request.location = location;
        self.graph_explorer_request.continuation = None;
        self.graph_explorer_request.selected = Some(identity);
        self.graph_explorer_request.selection_origin = GraphExplorerSelectionOrigin::Contextual;
        self.selected_id = Some(id);
        self.rebuild_layered_scene_with_prior(true);
        self.graph_viewport_mode = GraphViewportMode::FollowSelection;
        if narrowed {
            self.show_notice(
                "Opened the exact task in a temporary whole-plan graph; Esc restores the saved Graph session."
                    .to_string(),
            );
        }
        self.dirty = true;
    }

    fn return_graph_origin(&mut self) {
        let Some(origin) = self.graph_return_context.take() else {
            return;
        };
        self.restore_saved_graph_session(&origin);
        self.route = origin.route;
        self.selected_id = origin.selected_id;
        self.selection_state = SelectionState::Present;
        self.focus = Focus::Primary;
        if self.route == Route::Tasks {
            self.rebuild_task_browser();
        }
        self.dirty = true;
    }

    fn restore_saved_graph_session(&mut self, origin: &GraphReturnContext) {
        self.graph_request = origin.graph_request.clone();
        self.graph_projection = self.snapshot.graph.project(&self.graph_request);
        self.graph_explorer_request.location = origin.explorer.location.clone();
        self.graph_explorer_request
            .selected
            .clone_from(&origin.explorer.selected);
        self.graph_explorer_request
            .continuation
            .clone_from(&origin.explorer.continuation);
        self.graph_explorer_request.selection_origin = GraphExplorerSelectionOrigin::Remembered;
        self.graph_ordering = origin.ordering;
        self.graph_orientation = origin.orientation;
        self.graph_density = origin.density;
        self.graph_resolved_density = origin.resolved_density;
        self.graph_edge_presentation = origin.edge_presentation;
        self.rebuild_layered_scene_with_prior(true);
        self.graph_scroll_row = origin.explorer.scroll_row;
        self.graph_scroll_column = origin.explorer.scroll_column;
        self.graph_viewport_mode = origin.explorer.viewport_mode;
        self.graph_explorer_history.clear();
        self.graph_relation_cursor = None;
    }

    fn set_graph_viewport(&mut self, row: usize, column: usize) {
        if self.route != Route::Graph {
            return;
        }
        let (rows, columns) = self.graph_content_extent();
        self.graph_scroll_row = row.min(rows.saturating_sub(1));
        self.graph_scroll_column = column.min(columns.saturating_sub(1));
        self.graph_viewport_mode = GraphViewportMode::Manual;
        self.dirty = true;
    }

    fn rebuild_layered_scene(&mut self) {
        self.rebuild_layered_scene_with_prior(true);
    }

    fn rebuild_layered_scene_with_prior(&mut self, retain_prior: bool) {
        self.graph_explorer_request.graph = self.graph_request.clone();
        self.graph_explorer_request
            .search
            .clone_from(&self.task_query);
        let selected_is_known =
            self.graph_explorer_request
                .selected
                .as_ref()
                .is_none_or(|identity| match identity {
                    GraphExplorerIdentity::Task(id) => self.graph_projection.contains(id),
                    GraphExplorerIdentity::Milestone(id) => {
                        self.snapshot.task_rows.iter().any(|row| {
                            row.id == *id
                                && row
                                    .tags
                                    .iter()
                                    .any(|tag| tag == aplan_core::task::MILESTONE_TAG)
                        })
                    }
                });
        if !selected_is_known {
            self.graph_explorer_request.selected = None;
            self.graph_explorer_request.selection_origin = GraphExplorerSelectionOrigin::Automatic;
        }
        self.graph_explorer = self
            .snapshot
            .graph
            .project_explorer(&self.graph_explorer_request);
        self.graph_fallback_node_columns = self
            .graph_explorer
            .graph
            .nodes
            .iter()
            .map(fallback_node_width)
            .max()
            .unwrap_or_default();
        self.rebuild_current_cell_scene(retain_prior);
    }

    fn rebuild_current_cell_scene(&mut self, retain_prior: bool) {
        self.graph_scene_generation = self.graph_scene_generation.saturating_add(1);
        let prior = if retain_prior {
            self.graph_retained_cell_scene().cloned().map(Box::new)
        } else {
            None
        };
        let ordering_hints = graph_navigation::ordering_hints(
            self.graph_ordering,
            &self.graph_explorer,
            &self.snapshot.task_rows,
        );
        let request = layered_scene_request(
            self.graph_explorer.clone(),
            self.graph_resolved_density,
            self.graph_orientation,
            self.graph_ordering,
            ordering_hints,
            prior,
        );
        self.graph_cell_scene = CellSceneJob::new(self.graph_scene_generation, request)
            .run()
            .result;
    }

    pub(crate) fn graph_cell_scene(&self) -> Option<&CellScene> {
        match &self.graph_cell_scene {
            CellSceneResult::Ready(scene) => Some(scene),
            CellSceneResult::Fallback(_) => None,
        }
    }

    /// Geometry retained across a failed relayout is recovery input, not the
    /// current projection. It may seed a later successful layout but must not
    /// be painted, hit-tested or navigated as if it represented current data.
    pub(crate) fn graph_retained_cell_scene(&self) -> Option<&CellScene> {
        match &self.graph_cell_scene {
            CellSceneResult::Ready(scene) => Some(scene),
            CellSceneResult::Fallback(fallback) => fallback.retained.as_deref(),
        }
    }

    pub(crate) fn graph_scene_fallback(&self) -> Option<&aplan_app::CellSceneFallback> {
        match &self.graph_cell_scene {
            CellSceneResult::Ready(_) => None,
            CellSceneResult::Fallback(fallback) => Some(fallback),
        }
    }

    pub(crate) fn graph_content_extent(&self) -> (usize, usize) {
        self.graph_cell_scene().map_or_else(
            || {
                let columns =
                    self.graph_fallback_node_columns
                        .max(self.graph_scene_fallback().map_or_else(
                            || crate::text::cells(FALLBACK_MESSAGE),
                            |fallback| crate::text::cells(&fallback_message(fallback)),
                        ));
                (
                    self.graph_explorer.graph.nodes.len().saturating_add(1),
                    columns,
                )
            },
            |scene| {
                (
                    usize::from(scene.bounds.height),
                    usize::from(scene.bounds.width),
                )
            },
        )
    }

    pub(crate) fn graph_node_ids(&self) -> Vec<String> {
        if let Some(scene) = self.graph_cell_scene() {
            let mut nodes = scene.nodes.iter().collect::<Vec<_>>();
            nodes.sort_by_key(|node| (node.rank, node.order, node.identity.clone()));
            return nodes
                .into_iter()
                .map(|node| node.identity.id().to_string())
                .collect();
        }
        self.fallback_graph_identities()
            .into_iter()
            .map(|identity| identity.id().to_string())
            .collect()
    }

    fn fallback_graph_identities(&self) -> Vec<GraphExplorerIdentity> {
        self.graph_explorer
            .graph
            .nodes
            .iter()
            .map(|node| node.identity.clone())
            .collect()
    }

    pub(crate) fn graph_role(&self, id: &str) -> GraphRole {
        let selected = self.selected_id.as_deref();
        let selected_identity = self.graph_selected_identity();
        let blocker = selected_identity.is_some_and(|selected| {
            self.graph_explorer
                .graph
                .edges
                .iter()
                .any(|edge| edge.to == *selected && edge.from.id() == id)
        });
        let dependent = selected_identity.is_some_and(|selected| {
            self.graph_explorer
                .graph
                .edges
                .iter()
                .any(|edge| edge.from == *selected && edge.to.id() == id)
        });
        let critical = self
            .graph_explorer
            .graph
            .nodes
            .iter()
            .find(|node| node.identity.id() == id)
            .is_some_and(|node| {
                node.represented_ids
                    .iter()
                    .any(|represented| self.graph_explorer.critical_path.contains(represented))
            });
        let member = selected.is_some_and(|selected| {
            self.graph_projection.view.edges.iter().any(|edge| {
                edge.kind == aplan_core::EdgeKind::Membership
                    && ((edge.from == selected && edge.to == id)
                        || (edge.to == selected && edge.from == id))
            })
        });
        match (selected == Some(id), blocker, dependent, member, critical) {
            (true, _, _, _, true) => GraphRole::SelectedCritical,
            (true, _, _, _, false) => GraphRole::Selected,
            (false, true, true, _, _) => GraphRole::BlockerDependent,
            (false, true, false, _, true) => GraphRole::BlockerCritical,
            (false, true, false, _, false) => GraphRole::Blocker,
            (false, false, true, _, true) => GraphRole::DependentCritical,
            (false, false, true, _, false) => GraphRole::Dependent,
            (false, false, false, true, true) => GraphRole::MemberCritical,
            (false, false, false, true, false) => GraphRole::Member,
            (false, false, false, false, true) => GraphRole::Critical,
            (false, false, false, false, false) => GraphRole::Unrelated,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn graph_hit(&self, point: ScenePoint) -> Option<String> {
        let scene = self.graph_scene.as_ref().ok()?;
        let SceneHit { nodes, edges, .. } = scene.hit_test(point);
        nodes
            .into_iter()
            .next()
            .or_else(|| edges.into_iter().next().map(|edge| edge.to))
    }

    pub(crate) fn graph_selection_exclusion(&self) -> Option<&str> {
        let id = self.selected_id.as_deref()?;
        (!self.graph_projection.contains(id))
            .then(|| self.graph_projection.exclusion(id))
            .flatten()
    }

    pub(crate) fn selected_graph_continuation(&self) -> Option<&GraphContinuation> {
        self.graph_explorer
            .boundaries
            .get(self.graph_continuation_selected)
    }

    pub(crate) fn graph_continuation_count(&self) -> usize {
        self.graph_explorer.boundaries.len()
    }

    pub(crate) fn has_graph_window_history(&self) -> bool {
        !self.graph_explorer_history.is_empty() || self.graph_return_context.is_some()
    }

    pub(crate) fn graph_breadcrumb(&self) -> String {
        let mut parts = Vec::new();
        if let Some(origin) = &self.graph_return_context {
            parts.push(origin.route.label().to_string());
        }
        parts.push("Overview".to_string());
        match &self.graph_explorer.location {
            GraphExplorerLocation::Plan => {}
            GraphExplorerLocation::Phase(phase) => parts.push(format!("Phase {phase}")),
            GraphExplorerLocation::Milestone(id) => {
                if let Some(phase) = self
                    .snapshot
                    .task_rows
                    .iter()
                    .find(|row| row.id == *id)
                    .map(|row| row.phase)
                {
                    parts.push(format!("Phase {phase}"));
                }
                parts.push(id.clone());
            }
            GraphExplorerLocation::Task(id) => {
                if let Some(row) = self.snapshot.task_rows.iter().find(|row| row.id == *id) {
                    parts.push(format!("Phase {}", row.phase));
                    if let Some(milestone) = &row.milestone {
                        parts.push(milestone.clone());
                    }
                }
                parts.push(id.clone());
            }
        }
        parts.join(" > ")
    }

    pub(crate) fn graph_relation_progress(&self) -> Option<(bool, usize, usize)> {
        self.graph_relation_cursor
            .as_ref()
            .map(|cursor| (cursor.incoming, cursor.index + 1, cursor.count))
    }

    pub(crate) fn graph_movement_label(&self) -> String {
        let Some(scene) = self.graph_cell_scene() else {
            return "layout unavailable".to_string();
        };
        match &scene.movement {
            aplan_app::CellSceneMovement::Fresh => "layout fresh".to_string(),
            aplan_app::CellSceneMovement::ReusedStatusOnly => "layout stable".to_string(),
            aplan_app::CellSceneMovement::Relayout {
                moved_nodes,
                total_distance,
                reasons,
                ..
            } => {
                let reasons = reasons
                    .iter()
                    .map(|reason| match reason {
                        aplan_app::CellRelayoutReason::StructuralGraphChange => "structure",
                        aplan_app::CellRelayoutReason::CardSizeChange => "card size",
                        aplan_app::CellRelayoutReason::OrientationChange => "direction",
                        aplan_app::CellRelayoutReason::OrderingPolicyChange => "order",
                        aplan_app::CellRelayoutReason::LayoutInputChange => "layout input",
                    })
                    .collect::<Vec<_>>()
                    .join("+");
                format!("layout moved {moved_nodes}, distance {total_distance} ({reasons})")
            }
        }
    }

    pub(crate) fn graph_continuation_direction_label(
        direction: GraphContinuationDirection,
    ) -> &'static str {
        match direction {
            GraphContinuationDirection::Incoming => "incoming",
            GraphContinuationDirection::Outgoing => "outgoing",
            GraphContinuationDirection::Component => "component",
        }
    }

    pub(crate) fn has_graph_narrowing(&self) -> bool {
        !self.task_query.is_empty()
            || !self.task_selector.is_empty()
            || self.graph_request.scope != GraphScope::Whole
            || self.graph_request.membership
    }

    fn rebuild_report(&mut self) {
        let selected_key = self
            .report_rows()
            .get(self.report_selected_row)
            .map(|row| row.key.clone());
        let request = ReportRequest {
            selector: self.task_selector.clone(),
            stale_as_of: (!self.report_as_of.is_empty()).then(|| self.report_as_of.clone()),
            stale_window_days: None,
        };
        self.report_projection = self.snapshot.reports.project(self.report_kind, &request);
        self.reconcile_report_row(selected_key.as_deref());
        self.dirty = true;
    }

    fn reconcile_report_search(&mut self) {
        if self.focus == Focus::Detail {
            let selected_key = self
                .report_rows()
                .get(self.report_selected_row)
                .map(|row| row.key.clone());
            self.reconcile_report_row(selected_key.as_deref());
        } else {
            let visible = self.report_kinds();
            if !visible.contains(&self.report_kind)
                && let Some(first) = visible.first()
            {
                self.report_kind = *first;
                self.report_selected_row = 0;
                self.rebuild_report();
                return;
            }
        }
        self.dirty = true;
    }

    fn reconcile_report_row(&mut self, selected_key: Option<&str>) {
        let (position, len) = {
            let rows = self.report_rows();
            (
                selected_key.and_then(|key| rows.iter().position(|row| row.key == key)),
                rows.len(),
            )
        };
        self.report_selected_row =
            position.unwrap_or_else(|| self.report_selected_row.min(len.saturating_sub(1)));
    }

    fn open_report_row(&mut self) {
        let Some(row) = self.report_rows().get(self.report_selected_row).copied() else {
            self.show_notice("This report has no drill-down row.".to_string());
            return;
        };
        match row.provenance {
            ReportRowProvenance::Configuration => {
                self.show_notice(
                    "This row is a configuration fact and has no contributing task.".to_string(),
                );
                return;
            }
            ReportRowProvenance::NoSubjects => {
                self.show_notice(
                    "This row has no subjects, so there is no contributing task to open."
                        .to_string(),
                );
                return;
            }
            ReportRowProvenance::Tasks => {}
        }
        if row.task_ids.is_empty() {
            self.show_notice("This row has no contributing task to open.".to_string());
            return;
        }
        let ids = row.task_ids.iter().cloned().collect::<BTreeSet<_>>();
        self.task_scope = Some(TaskScope::Explicit {
            label: format!("report: {} / {}", self.report_kind.label(), row.label),
            ids: ids.clone(),
        });
        self.task_query.clear();
        self.task_query_cursor = 0;
        // The row already carries the exact contributing set. Retaining the
        // report editor's selector here would intersect that set a second time
        // and can hide contributors of a whole-plan report, even though its
        // scope line correctly said the selector did not affect computation.
        self.task_selector = Selector::default();
        self.selected_id = ids.iter().next().cloned();
        self.route = Route::Tasks;
        self.focus = Focus::Primary;
        self.rebuild_task_browser();
    }

    pub(crate) fn report_kinds(&self) -> Vec<ReportKind> {
        let query = self.report_kind_query.to_lowercase();
        ReportKind::ALL
            .into_iter()
            .filter(|kind| query.is_empty() || kind.label().to_lowercase().contains(&query))
            .collect()
    }

    pub(crate) fn report_rows(&self) -> Vec<&aplan_app::ReportRow> {
        let query = self.report_row_query.to_lowercase();
        self.report_projection
            .rows
            .iter()
            .filter(|row| {
                query.is_empty()
                    || row.key.to_lowercase().contains(&query)
                    || row.label.to_lowercase().contains(&query)
                    || row.summary.to_lowercase().contains(&query)
            })
            .collect()
    }

    pub(crate) fn has_report_narrowing(&self) -> bool {
        !self.task_selector.is_empty()
            || !self.report_kind_query.is_empty()
            || !self.report_row_query.is_empty()
            || !self.report_as_of.is_empty()
    }

    fn rebuild_task_browser(&mut self) {
        self.task_browser = aplan_app::browse_tasks(
            &self.snapshot.task_rows,
            &TaskBrowserRequest {
                query: self.task_query.clone(),
                selector: self.task_selector.clone(),
                sort: self.task_sort,
            },
        );
        if let Some(scope) = self.task_scope.clone() {
            let (label, ids) = match scope {
                TaskScope::Overview(metric) => (
                    format!("overview: {}", metric.scope_label()),
                    self.overview_task_ids(metric).into_iter().collect(),
                ),
                TaskScope::Explicit { label, ids } => (label, ids),
            };
            let scope_exists = self
                .snapshot
                .task_rows
                .iter()
                .any(|row| ids.contains(&row.id));
            self.task_browser
                .rows
                .retain(|row| ids.contains(&row.task.id));
            self.task_browser.selector_matches = self.task_browser.rows.len();
            self.task_browser.active_filters.insert(0, label);
            if self.task_browser.rows.is_empty() && !scope_exists {
                self.task_browser.no_results = Some(aplan_app::NoTaskResults::EmptyScope);
            }
        }
        if let Some(selected_id) = &self.selected_id {
            if let Some(index) = self
                .task_browser
                .rows
                .iter()
                .position(|row| &row.task.id == selected_id)
            {
                self.selected = index;
            } else {
                // Keep the identity so clearing a transient filter restores the
                // selection, but anchor a hidden selection at the visible start.
                self.selected = 0;
            }
        } else if let Some(first) = self.task_browser.rows.first() {
            self.selected = 0;
            self.selected_id = Some(first.task.id.clone());
        }
        self.selected = self
            .selected
            .min(self.task_browser.rows.len().saturating_sub(1));
        if self.modal == Some(Modal::Filters) {
            self.modal_selected = self
                .modal_selected
                .min(self.filter_choices().len().saturating_sub(1));
        }
        self.rebuild_work_projection();
        self.dirty = true;
    }

    fn overview_task_ids(&self, metric: OverviewMetric) -> Vec<String> {
        match metric {
            OverviewMetric::Cuts => self.snapshot.overview.cut_task_ids.clone(),
            OverviewMetric::Effort => self.snapshot.overview.live_task_ids.clone(),
            OverviewMetric::Ready => self
                .snapshot
                .overview
                .ready
                .iter()
                .map(|task| task.id.clone())
                .collect(),
            OverviewMetric::Due => {
                let schedule = self.work_revisits();
                schedule
                    .due
                    .into_iter()
                    .chain(schedule.waiting)
                    .map(|task| task.id)
                    .collect()
            }
            OverviewMetric::CriticalPath => self
                .snapshot
                .overview
                .critical_path
                .as_ref()
                .map(|(_, tasks)| tasks.iter().map(|task| task.id.clone()).collect())
                .unwrap_or_default(),
            OverviewMetric::Findings => Vec::new(),
        }
    }

    pub(crate) fn has_task_narrowing(&self) -> bool {
        self.task_scope.is_some() || !self.task_query.is_empty() || !self.task_selector.is_empty()
    }

    pub(crate) fn has_diagnostic_narrowing(&self) -> bool {
        !self.diagnostic_query.is_empty() || !self.diagnostic_filters.is_empty()
    }

    pub(crate) fn has_inbox_narrowing(&self) -> bool {
        !self.inbox_query.is_empty()
            || self.inbox_state != Some(aplan_core::capture::CaptureState::Open)
            || self.inbox_revisit != aplan_core::capture::RevisitFilter::Actionable
            || self.inbox_kind.is_some()
            || self.inbox_label.is_some()
    }

    pub(crate) fn inbox_rows(&self) -> Vec<&aplan_core::capture::Capture> {
        let Ok(workspace) = &self.snapshot.inbox else {
            return Vec::new();
        };
        let needle = self.inbox_query.to_lowercase();
        let mut rows = workspace
            .captures
            .iter()
            .filter(|capture| {
                self.inbox_state.is_none_or(|state| capture.state == state)
                    && self
                        .inbox_kind
                        .as_ref()
                        .is_none_or(|kind| &capture.kind == kind)
                    && self
                        .inbox_label
                        .as_ref()
                        .is_none_or(|label| capture.labels.contains(label))
            })
            .filter(|capture| {
                let deferred = capture_is_deferred(capture, &workspace.evaluated_at);
                match self.inbox_revisit {
                    aplan_core::capture::RevisitFilter::Actionable => !deferred,
                    aplan_core::capture::RevisitFilter::Deferred => deferred,
                    aplan_core::capture::RevisitFilter::All => true,
                }
            })
            .filter(|capture| {
                needle.is_empty()
                    || capture.title.to_lowercase().contains(&needle)
                    || capture
                        .body
                        .as_ref()
                        .is_some_and(|body| body.to_lowercase().contains(&needle))
                    || capture.id.to_lowercase().contains(&needle)
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            right
                .captured_at
                .cmp(&left.captured_at)
                .then(left.id.cmp(&right.id))
        });
        rows
    }

    pub(crate) fn selected_capture(&self) -> Option<&aplan_core::capture::Capture> {
        let id = self.selected_capture_id.as_deref()?;
        self.snapshot
            .inbox
            .as_ref()
            .ok()?
            .captures
            .iter()
            .find(|capture| capture.id == id)
    }

    pub(crate) fn selected_capture_visible_index(&self) -> Option<usize> {
        let id = self.selected_capture_id.as_deref()?;
        self.inbox_rows()
            .iter()
            .position(|capture| capture.id == id)
    }

    pub(crate) fn inbox_filter_choices(&self) -> Vec<InboxFilterChoice> {
        let mut choices = vec![
            InboxFilterChoice::State(Some(aplan_core::capture::CaptureState::Open)),
            InboxFilterChoice::State(Some(aplan_core::capture::CaptureState::Promoted)),
            InboxFilterChoice::State(Some(aplan_core::capture::CaptureState::Discarded)),
            InboxFilterChoice::State(None),
            InboxFilterChoice::Revisit(aplan_core::capture::RevisitFilter::Actionable),
            InboxFilterChoice::Revisit(aplan_core::capture::RevisitFilter::Deferred),
            InboxFilterChoice::Revisit(aplan_core::capture::RevisitFilter::All),
            InboxFilterChoice::Kind(None),
        ];
        if let Ok(workspace) = &self.snapshot.inbox {
            choices.extend(
                workspace
                    .kinds
                    .iter()
                    .cloned()
                    .map(|kind| InboxFilterChoice::Kind(Some(kind))),
            );
            choices.push(InboxFilterChoice::Label(None));
            let labels = workspace
                .captures
                .iter()
                .flat_map(|capture| capture.labels.iter().cloned())
                .collect::<BTreeSet<_>>();
            choices.extend(
                labels
                    .into_iter()
                    .map(|label| InboxFilterChoice::Label(Some(label))),
            );
        }
        choices
    }

    pub(crate) fn inbox_filter_selected(&self, choice: &InboxFilterChoice) -> bool {
        match choice {
            InboxFilterChoice::State(state) => self.inbox_state == *state,
            InboxFilterChoice::Revisit(revisit) => self.inbox_revisit == *revisit,
            InboxFilterChoice::Kind(kind) => self.inbox_kind == *kind,
            InboxFilterChoice::Label(label) => self.inbox_label == *label,
        }
    }

    fn reconcile_inbox_selection(&mut self) {
        let visible = self
            .inbox_rows()
            .into_iter()
            .map(|capture| capture.id.clone())
            .collect::<Vec<_>>();
        if self
            .selected_capture_id
            .as_ref()
            .is_none_or(|selected| !visible.contains(selected))
        {
            self.selected_capture_id = visible.first().cloned();
        }
        if self.route == Route::Inbox {
            self.selected = self
                .selected_capture_id
                .as_ref()
                .and_then(|selected| visible.iter().position(|id| id == selected))
                .unwrap_or_default();
            self.inspector_scroll = None;
        }
        self.dirty = true;
    }

    pub(crate) fn selected_task(&self) -> Option<&TaskBrowserMatch> {
        let id = self.selected_id.as_deref()?;
        self.task_browser.rows.iter().find(|row| row.task.id == id)
    }

    pub(crate) fn selected_visible_index(&self) -> Option<usize> {
        let id = self.selected_id.as_deref()?;
        self.task_browser
            .rows
            .iter()
            .position(|row| row.task.id == id)
    }

    pub(crate) fn selected_inspector(&self) -> Option<&aplan_app::Show> {
        let id = self.selected_id.as_deref()?;
        self.snapshot
            .inspectors
            .iter()
            .find(|inspector| inspector.id == id)
    }

    pub(crate) fn filtered_test_choices(&self) -> Vec<&crate::evidence::TestEvidenceChoice> {
        let Some(TestCatalogueLayer::Ready(view)) = self.test_catalogue.as_ref() else {
            return Vec::new();
        };
        let needle = self.test_query.to_lowercase();
        view.choices
            .iter()
            .filter(|choice| needle.is_empty() || choice.name.to_lowercase().contains(&needle))
            .collect()
    }

    pub(crate) fn selected_test_choice(&self) -> Option<&crate::evidence::TestEvidenceChoice> {
        self.filtered_test_choices()
            .get(self.modal_selected)
            .copied()
    }

    pub(crate) fn inspector_items(&self) -> Vec<InspectorItem> {
        let Some(show) = self.selected_inspector() else {
            return Vec::new();
        };
        match self.inspector_section {
            InspectorSection::Summary => inspector_summary(show),
            InspectorSection::Work => self.work_projection().map_or_else(Vec::new, work_items),
            InspectorSection::Body => vec![InspectorItem::text(
                show.body.as_deref().unwrap_or("No body was present."),
            )],
            InspectorSection::Relationships => inspector_relationships(show),
            InspectorSection::Paths => {
                strings_or_empty(&show.paths, "DECLARED PATHS", "Path", "No paths declared.")
            }
            InspectorSection::Traces => inspector_traces(show),
            InspectorSection::Evidence => inspector_evidence(show),
            InspectorSection::Findings => inspector_findings(show),
        }
    }

    pub(crate) fn work_projection(&self) -> Option<&aplan_app::WorkDiscoveryProjection> {
        self.work_projection.as_ref()
    }

    fn rebuild_work_projection(&mut self) {
        self.work_projection = self.selected_id.as_deref().and_then(|id| {
            self.snapshot.work.project(
                id,
                WorkDiscoveryRequest {
                    traversal: self.why_traversal,
                    as_of_epoch_seconds: self.effective_work_as_of_epoch_seconds(),
                },
            )
        });
    }

    pub(crate) fn work_revisits(&self) -> aplan_app::RevisitSchedule {
        if self.work_as_of.is_empty() {
            return aplan_app::RevisitSchedule {
                due: self
                    .snapshot
                    .overview
                    .due
                    .iter()
                    .map(|task| aplan_app::ScheduledRevisit {
                        id: task.id.clone(),
                        title: task.title.clone(),
                        at: task.revisit_at.clone(),
                        overdue_seconds: task.overdue_seconds,
                    })
                    .collect(),
                waiting: self
                    .snapshot
                    .overview
                    .waiting_revisits
                    .iter()
                    .map(|task| aplan_app::ScheduledRevisit {
                        id: task.id.clone(),
                        title: task.title.clone(),
                        at: task.revisit_at.clone(),
                        overdue_seconds: -task.seconds_until,
                    })
                    .collect(),
            };
        }
        self.snapshot
            .work
            .revisit_schedule(self.effective_work_as_of_epoch_seconds())
    }

    pub(crate) fn effective_work_as_of_epoch_seconds(&self) -> i64 {
        if self.work_as_of.is_empty() {
            self.snapshot.overview.as_of_epoch_seconds
        } else {
            self.work_as_of_epoch_seconds
        }
    }

    pub(crate) fn work_as_of_draft_epoch(&self) -> Option<i64> {
        aplan_app::parse_work_as_of(
            &self.work_as_of_draft,
            self.snapshot.overview.as_of_epoch_seconds,
        )
    }

    fn move_inspector_section(&mut self, forward: bool) {
        if self.focus != Focus::Detail {
            return;
        }
        self.inspector_section = self.inspector_section.adjacent(forward);
        self.inspector_selected = 0;
        self.inspector_scroll = None;
        self.dirty = true;
    }

    fn move_inspector_item(&mut self, forward: bool) {
        let count = self.inspector_items().len();
        if count == 0 {
            return;
        }
        self.inspector_selected = if forward {
            (self.inspector_selected + 1) % count
        } else {
            self.inspector_selected.checked_sub(1).unwrap_or(count - 1)
        };
        self.inspector_scroll = None;
        self.dirty = true;
    }

    fn set_inspector_scroll(&mut self, scroll: usize) {
        if self.focus == Focus::Detail {
            self.inspector_scroll = Some(scroll);
            self.dirty = true;
        }
    }

    fn reset_inspector_for_snapshot(&mut self) {
        if self.selection_state == SelectionState::Removed {
            self.inspector_section = InspectorSection::Summary;
            self.inspector_selected = 0;
            self.inspector_scroll = None;
            self.inspector_history.clear();
            return;
        }
        let count = self.inspector_items().len();
        self.inspector_selected = self.inspector_selected.min(count.saturating_sub(1));
        // A link history records coordinates in the previous immutable
        // snapshot. A later refresh may have removed either the task or the
        // item, so retaining it would make Back restore a fabricated location.
        self.inspector_history.clear();
    }

    pub(crate) fn inspector_item_has_link(&self) -> bool {
        self.inspector_items()
            .get(self.inspector_selected)
            .is_some_and(|item| item.link.is_some())
    }

    pub(crate) fn inspector_has_history(&self) -> bool {
        !self.inspector_history.is_empty()
    }

    fn inspector_location(&self) -> Option<InspectorLocation> {
        Some(InspectorLocation {
            task_id: self.selected_id.clone()?,
            section: self.inspector_section,
            selected: self.inspector_selected,
            scroll: self.inspector_scroll,
        })
    }

    fn activate_inspector_item(&mut self) {
        let Some(link) = self
            .inspector_items()
            .get(self.inspector_selected)
            .and_then(|item| item.link.clone())
        else {
            self.show_notice("The selected inspector item has no linked destination.".to_string());
            return;
        };
        if let Some(location) = self.inspector_location() {
            self.inspector_history.push(location);
        }
        match link {
            InspectorLink::Section(section) => {
                self.inspector_section = section;
                self.inspector_selected = 0;
                self.inspector_scroll = None;
            }
            InspectorLink::Task(id) => {
                if !self.snapshot.inspectors.iter().any(|show| show.id == id) {
                    self.inspector_history.pop();
                    self.show_notice(format!(
                        "Task {id} is missing or unloadable; its relationship remains visible."
                    ));
                    return;
                }
                self.selected_id = Some(id);
                self.selection_state = SelectionState::Present;
                self.inspector_section = InspectorSection::Summary;
                self.inspector_selected = 0;
                self.inspector_scroll = None;
                self.rebuild_work_projection();
                if self.route == Route::Graph {
                    self.graph_viewport_mode = GraphViewportMode::FollowSelection;
                    self.dirty = true;
                } else {
                    self.rebuild_task_browser();
                }
                self.focus = Focus::Detail;
            }
        }
        self.dirty = true;
    }

    fn edit_palette(&mut self, input: Option<&str>) {
        if self.modal != Some(Modal::Palette) {
            return;
        }
        let changed = match input {
            Some(input) => text::append_bounded(
                &mut self.palette_query,
                input,
                MAX_PALETTE_QUERY_CELLS,
                MAX_PALETTE_QUERY_GRAPHEMES,
                MAX_PALETTE_QUERY_BYTES,
            ),
            None => text::pop_grapheme(&mut self.palette_query),
        };
        if changed {
            self.modal_selected = 0;
            self.dirty = true;
        }
    }

    fn select_index(&mut self, index: usize) {
        if self.route == Route::Overview {
            if index < OverviewMetric::ALL.len() {
                self.overview_selected = index;
                self.focus = Focus::Primary;
                self.dirty = true;
            }
            return;
        }
        if self.route == Route::Diagnostics {
            if let Some(identity) = self
                .diagnostic_findings()
                .get(index)
                .map(|finding| finding.identity.clone())
            {
                self.selected = index;
                self.selected_finding_id = Some(identity);
                self.focus = Focus::Primary;
                self.inspector_scroll = None;
                self.dirty = true;
            }
            return;
        }
        if self.route == Route::Graph {
            if let Some(id) = self.graph_node_ids().get(index).cloned() {
                self.select_graph_task(id);
            }
            return;
        }
        if self.route == Route::Reports {
            if let Some(kind) = self.report_kinds().get(index).copied() {
                self.report_kind = kind;
                self.report_selected_row = 0;
                self.focus = Focus::Primary;
                self.rebuild_report();
            }
            return;
        }
        if self.route == Route::Inbox {
            if let Some(id) = self
                .inbox_rows()
                .get(index)
                .map(|capture| capture.id.clone())
            {
                self.selected = index;
                self.selected_capture_id = Some(id);
                self.focus = Focus::Primary;
                self.inspector_scroll = None;
                self.dirty = true;
            }
            return;
        }
        if let Some(row) = self.task_browser.rows.get(index) {
            self.selected = index;
            self.selected_id = Some(row.task.id.clone());
            self.selection_state = SelectionState::Present;
            self.focus = Focus::Primary;
            self.inspector_history.clear();
            self.inspector_section = InspectorSection::Summary;
            self.inspector_selected = 0;
            self.inspector_scroll = None;
            self.rebuild_work_projection();
            self.dirty = true;
        }
    }

    fn select_report_row(&mut self, index: usize) {
        if self.route != Route::Reports || index >= self.report_rows().len() {
            return;
        }
        self.report_selected_row = index;
        self.focus = Focus::Detail;
        self.dirty = true;
    }

    fn invoke(&mut self, action: Action) -> Transition {
        self.modal = None;
        self.palette_query.clear();
        self.modal_selected = 0;
        self.dirty = true;
        self.update(action)
    }

    fn move_selection(&mut self, forward: bool) {
        if self.route == Route::Reports {
            self.move_report_selection(forward);
            return;
        }
        if self.focus == Focus::Detail {
            if !matches!(self.route, Route::Overview | Route::Diagnostics) {
                self.move_inspector_item(forward);
            }
            return;
        }
        if self.route == Route::Overview {
            let count = OverviewMetric::ALL.len();
            self.overview_selected = if forward {
                (self.overview_selected + 1) % count
            } else {
                self.overview_selected.checked_sub(1).unwrap_or(count - 1)
            };
            self.dirty = true;
            return;
        }
        if self.route == Route::Diagnostics {
            let identities = self
                .diagnostic_findings()
                .into_iter()
                .map(|finding| finding.identity.clone())
                .collect::<Vec<_>>();
            let count = identities.len();
            if count == 0 {
                return;
            }
            self.selected = if forward {
                (self.selected + 1) % count
            } else {
                self.selected.checked_sub(1).unwrap_or(count - 1)
            };
            self.selected_finding_id = Some(identities[self.selected].clone());
            self.inspector_scroll = None;
            self.dirty = true;
            return;
        }
        if self.route == Route::Graph {
            self.move_graph_spatial(if forward {
                SpatialDirection::Down
            } else {
                SpatialDirection::Up
            });
            return;
        }
        if self.route == Route::Inbox {
            let ids = self
                .inbox_rows()
                .into_iter()
                .map(|capture| capture.id.clone())
                .collect::<Vec<_>>();
            if ids.is_empty() {
                return;
            }
            let current = self
                .selected_capture_id
                .as_ref()
                .and_then(|selected| ids.iter().position(|id| id == selected));
            self.selected = if let Some(current) = current {
                if forward {
                    (current + 1) % ids.len()
                } else {
                    current.checked_sub(1).unwrap_or(ids.len() - 1)
                }
            } else if forward {
                0
            } else {
                ids.len() - 1
            };
            self.selected_capture_id = Some(ids[self.selected].clone());
            self.focus = Focus::Primary;
            self.inspector_scroll = None;
            self.dirty = true;
            return;
        }
        let count = self.task_browser.rows.len();
        if count == 0 {
            return;
        }
        let current = self.selected_visible_index();
        self.selected = if let Some(current) = current {
            if forward {
                (current + 1) % count
            } else {
                current.checked_sub(1).unwrap_or(count - 1)
            }
        } else if forward {
            0
        } else {
            count - 1
        };
        self.selected_id = Some(self.task_browser.rows[self.selected].task.id.clone());
        self.selection_state = SelectionState::Present;
        self.focus = Focus::Primary;
        self.rebuild_work_projection();
        self.dirty = true;
    }

    fn move_report_selection(&mut self, forward: bool) {
        if self.focus == Focus::Detail {
            let count = self.report_rows().len();
            if count > 0 {
                self.report_selected_row = if forward {
                    (self.report_selected_row + 1) % count
                } else {
                    self.report_selected_row.checked_sub(1).unwrap_or(count - 1)
                };
                self.dirty = true;
            }
            return;
        }
        let visible = self.report_kinds();
        let current = visible
            .iter()
            .position(|kind| *kind == self.report_kind)
            .unwrap_or_default();
        let count = visible.len();
        if count == 0 {
            return;
        }
        let next = if forward {
            (current + 1) % count
        } else {
            current.checked_sub(1).unwrap_or(count - 1)
        };
        self.report_kind = visible[next];
        self.report_selected_row = 0;
        self.rebuild_report();
    }

    fn paste(&mut self, input: &str) {
        if self
            .mutation
            .as_ref()
            .is_some_and(|layer| matches!(layer.state, MutationState::Form(_)) && !layer.committing)
        {
            let _ = self.update_mutation_form(FormAction::Insert(input.to_string()));
        } else if self.modal == Some(Modal::Palette) {
            self.edit_palette(Some(input));
        } else if matches!(
            self.modal,
            Some(
                Modal::Search
                    | Modal::RuleSearch
                    | Modal::ReportDate
                    | Modal::WorkDate
                    | Modal::TestCatalogue
            )
        ) {
            self.edit_search(Some(input), SearchOperation::Insert);
        } else {
            self.show_notice(format!(
                "Paste ignored: no text field is focused ({} byte(s)).",
                input.len()
            ));
        }
    }

    fn tick(&mut self) {
        if !self.needs_tick() {
            return;
        }
        if self.motion == Motion::Animated {
            self.animation_frame = self.animation_frame.wrapping_add(1) % 4;
        }
        let had_notice = self.notice.is_some();
        if let Some(notice) = &mut self.notice {
            notice.ticks_left = notice.ticks_left.saturating_sub(1);
            if notice.ticks_left == 0 {
                self.notice = None;
            }
        }
        self.dirty = had_notice || self.motion == Motion::Animated || self.startup.is_some();
    }

    fn reject_effect(&mut self, id: EffectId, reason: &str) {
        if self
            .effects
            .iter()
            .any(|record| record.request.id == id && record.request.kind == EffectKind::Refresh)
        {
            self.freshness = SnapshotFreshness::Unconfirmed;
        }
        if let Some(record) = self.effect_mut(id) {
            record.state = EffectState::Refused;
            record.detail = Some(reason.to_string());
        }
        self.show_notice(format!("Effect refused: {reason}"));
    }

    fn open_mutation(&mut self, action: MutationAction) -> Transition {
        if self.onboarding.is_some() {
            self.show_notice("Mutations are unavailable until a plan loads.".to_string());
            return Transition::default();
        }
        if self.has_pending_kind(EffectKind::Mutation) {
            self.show_notice(
                "A prior mutation operation is still running; wait for it before opening another."
                    .to_string(),
            );
            return Transition::default();
        }
        if self.pending_effects() >= MAX_PENDING_EFFECTS {
            self.show_notice(format!(
                "Effect queue is full ({MAX_PENDING_EFFECTS} running or queued)."
            ));
            return Transition::default();
        }
        let selected_task = if action.capture().is_some() {
            self.selected_capture_id.clone()
        } else {
            self.selected_id.clone()
        };
        self.modal = None;
        self.palette_query.clear();
        self.mutation = Some(MutationLayer {
            action,
            selected_task: selected_task.clone(),
            state: MutationState::Loading,
            edited_body: None,
            committing: false,
            plain_scroll: 0,
            selection_after_commit: None,
        });
        self.request_mutation_effect(MutationEffect::Prepare {
            action,
            selected_task,
        })
    }

    fn update_mutation_form(&mut self, action: FormAction) -> Transition {
        if matches!(
            action,
            FormAction::Confirm(_) | FormAction::PointerConfirm(_)
        ) && (self.pending_effects() >= MAX_PENDING_EFFECTS
            || self.effects.iter().any(|record| {
                record.state.is_pending() && record.request.kind == EffectKind::Mutation
            }))
        {
            self.show_notice("Confirmation is waiting for pending work to finish.".to_string());
            return Transition::default();
        }
        let Some(layer) = self.mutation.as_mut() else {
            return Transition::default();
        };
        let MutationState::Form(form) = &mut layer.state else {
            return Transition::default();
        };
        let transition = form.reduce(action);
        let action = layer.action;
        let selected_task = layer.selected_task.clone();
        let selection_after_commit = layer.selection_after_commit.clone();
        match transition {
            FormTransition::None => {
                self.dirty = true;
                Transition::default()
            }
            FormTransition::Cancelled => {
                self.mutation = None;
                self.dirty = true;
                Transition::default()
            }
            FormTransition::PreviewRequested(submission) => {
                let verification = self.verification.clone();
                let edited_body = layer
                    .edited_body
                    .as_deref()
                    .map(std::sync::Arc::<str>::from);
                self.request_mutation_effect(MutationEffect::Preview {
                    action,
                    selected_task,
                    submission,
                    verification,
                    edited_body,
                })
            }
            FormTransition::CommitRequested(change) => {
                let candidate = MutationCandidate::Plan {
                    action,
                    change,
                    review_rows: Vec::new(),
                    selection_after_commit,
                };
                let transition = self.request_mutation_effect(MutationEffect::Commit {
                    candidate: MutationTransfer::new(MutationPayload::CommitCandidate(candidate)),
                });
                if transition.effect.is_some()
                    && let Some(layer) = self.mutation.as_mut()
                {
                    layer.committing = true;
                }
                transition
            }
        }
    }

    fn scroll_mutation(&mut self, delta: i8) {
        if let Some(layer) = self.mutation.as_mut()
            && !layer.committing
        {
            layer.scroll_plain(delta);
            self.dirty = true;
        }
    }

    fn request_external_editor(&mut self) {
        let Some(layer) = self.mutation.as_ref() else {
            return;
        };
        if layer.action != MutationAction::Add {
            self.show_notice(
                "External editing is available only for an Add task body.".to_string(),
            );
            return;
        }
        let MutationState::Form(form) = &layer.state else {
            self.show_notice(
                "Return to the editable Add form before opening an external editor.".to_string(),
            );
            return;
        };
        if !form.is_editing() {
            self.show_notice(
                "Recompute from the editable form before changing its external body.".to_string(),
            );
            return;
        }
        self.external_editor_request = Some(
            layer
                .edited_body
                .clone()
                .unwrap_or_else(|| form.value("body").unwrap_or_default().to_string()),
        );
    }

    pub(crate) fn take_external_editor_request(&mut self) -> Option<String> {
        self.external_editor_request.take()
    }

    pub(crate) fn accept_external_editor_body(&mut self, body: String) {
        let Some(layer) = self.mutation.as_mut() else {
            self.show_notice("Edited body discarded because the Add form was closed.".to_string());
            return;
        };
        if layer.action != MutationAction::Add {
            self.show_notice("Edited body discarded because the active form changed.".to_string());
            return;
        }
        let MutationState::Form(form) = &mut layer.state else {
            self.show_notice(
                "Edited body discarded because the Add form is no longer editable.".to_string(),
            );
            return;
        };
        let bytes = body.len();
        layer.edited_body = Some(body);
        form.set_external_note(Some(format!(
            "External body: {bytes} byte(s); Ctrl-E reopens it and it overrides the short Body field."
        )));
        self.show_notice(format!(
            "External editor returned a {bytes}-byte task body."
        ));
        self.dirty = true;
    }

    pub(crate) fn reject_external_editor(&mut self, message: impl Into<String>) {
        self.show_notice(format!("External editor failed: {}", message.into()));
        self.dirty = true;
    }

    fn confirm_quick_mutation(&mut self) -> Transition {
        if self.pending_effects() >= MAX_PENDING_EFFECTS
            || self.effects.iter().any(|record| {
                record.state.is_pending() && record.request.kind == EffectKind::Mutation
            })
        {
            self.show_notice("Confirmation is waiting for pending work to finish.".to_string());
            return Transition::default();
        }
        let Some(layer) = self.mutation.as_mut() else {
            return Transition::default();
        };
        let MutationState::QuickReview { candidate, .. } = &mut layer.state else {
            return Transition::default();
        };
        let Some(candidate) = candidate.take() else {
            return Transition::default();
        };
        let transition = self.request_mutation_effect(MutationEffect::Commit {
            candidate: MutationTransfer::new(MutationPayload::CommitCandidate(*candidate)),
        });
        if transition.effect.is_some()
            && let Some(layer) = self.mutation.as_mut()
        {
            layer.committing = true;
        }
        transition
    }

    fn request_mutation_effect(&mut self, mutation: MutationEffect) -> Transition {
        if self.pending_effects() >= MAX_PENDING_EFFECTS {
            self.show_notice(format!(
                "Effect queue is full ({MAX_PENDING_EFFECTS} running or queued)."
            ));
            return Transition::default();
        }
        if self
            .effects
            .iter()
            .any(|record| record.state.is_pending() && record.request.kind == EffectKind::Mutation)
        {
            self.show_notice("A mutation operation is already pending.".to_string());
            return Transition::default();
        }
        let request = EffectRequest {
            id: EffectId(self.next_effect),
            generation: self.generation,
            focus_epoch: self.focus_epoch,
            input: self.snapshot.overview.input.sha256.clone(),
            kind: EffectKind::Mutation,
            check_overrides: CheckOverrides::default(),
            execute: false,
            mutation: Some(mutation),
        };
        self.next_effect = self.next_effect.saturating_add(1);
        self.make_record_room();
        self.effects.push_back(EffectRecord {
            request: request.clone(),
            state: EffectState::Queued,
            detail: None,
            progress: None,
        });
        self.dirty = true;
        Transition {
            effect: Some(request),
            cancel: None,
        }
    }

    fn accept_mutation(&mut self, id: EffectId, transfer: &MutationTransfer) -> Transition {
        if let Some(layer) = self.mutation.as_mut() {
            layer.committing = false;
        }
        let Some(payload) = transfer.take() else {
            return self.fail_mutation_payload(id, "mutation worker returned an empty handoff");
        };
        if self.mutation.is_none()
            && matches!(
                &payload,
                MutationPayload::Prepared(_)
                    | MutationPayload::Preview { .. }
                    | MutationPayload::PreviewFailed { .. }
            )
        {
            self.finish_mutation_effect(id, "result discarded after mutation layer closed");
            self.dirty = true;
            return Transition::default();
        }
        match payload {
            MutationPayload::Prepared(prepared) => {
                return self.accept_prepared_mutation(id, prepared);
            }
            MutationPayload::Preview { request, candidate } => {
                return self.accept_mutation_preview(id, request, candidate);
            }
            MutationPayload::PreviewFailed {
                action,
                request,
                message,
            } => {
                let Some(layer) = self.mutation.as_mut() else {
                    return self.fail_mutation_payload(id, "mutation layer was closed");
                };
                if layer.action != action {
                    return self.fail_mutation_payload(id, "preview belongs to another action");
                }
                let MutationState::Form(form) = &mut layer.state else {
                    return self.fail_mutation_payload(id, "preview refusal has no form");
                };
                if !form.accept_preview(request, Err(DomainIssue::new(message, Vec::new()))) {
                    return self.fail_mutation_payload(id, "late mutation refusal was discarded");
                }
                self.finish_mutation_effect(id, "preview refused");
            }
            MutationPayload::CommitCandidate(_) => {
                return self.fail_mutation_payload(id, "mutation worker returned its input");
            }
            MutationPayload::Committed(result) => return self.accept_mutation_commit(id, result),
        }
        self.dirty = true;
        Transition::default()
    }

    fn accept_mutation_preview(
        &mut self,
        id: EffectId,
        request: u64,
        candidate: MutationCandidate,
    ) -> Transition {
        let Some(layer) = self.mutation.as_mut() else {
            return self.fail_mutation_payload(id, "mutation layer was closed");
        };
        if candidate.action() != layer.action {
            return self.fail_mutation_payload(id, "preview belongs to another mutation action");
        }
        match candidate {
            MutationCandidate::Plan {
                change,
                review_rows,
                selection_after_commit,
                ..
            } => {
                let MutationState::Form(form) = &mut layer.state else {
                    return self.fail_mutation_payload(id, "plan preview has no form");
                };
                if !form.accept_preview_with_rows(request, Ok(*change), review_rows) {
                    return self.fail_mutation_payload(id, "late mutation preview was discarded");
                }
                layer.selection_after_commit = selection_after_commit;
            }
            MutationCandidate::Work { preview } => {
                if layer.selected_task.as_deref() != Some(preview.request().task.as_str()) {
                    return self.fail_mutation_payload(
                        id,
                        "work preview belongs to another selected task",
                    );
                }
                let MutationState::Form(form) = &mut layer.state else {
                    return self.fail_mutation_payload(id, "work preview has no form");
                };
                if form.input_sha256() != preview.input().sha256 {
                    return self
                        .fail_mutation_payload(id, "work preview belongs to another plan input");
                }
                if !form.accept_preview_request(request) {
                    return self.fail_mutation_payload(id, "late mutation preview was discarded");
                }
                let candidate = MutationCandidate::Work { preview };
                let rows = candidate.review_rows();
                let old = std::mem::replace(&mut layer.state, MutationState::Loading);
                let MutationState::Form(form) = old else {
                    unreachable!("work form checked before replacement")
                };
                layer.state = MutationState::QuickReview {
                    candidate: Some(Box::new(candidate)),
                    form: Some(form),
                    rows,
                };
                layer.plain_scroll = 0;
            }
        }
        self.finish_mutation_effect(id, "preview ready");
        self.dirty = true;
        Transition::default()
    }

    fn accept_prepared_mutation(&mut self, id: EffectId, prepared: PreparedMutation) -> Transition {
        let seed = self.pending_test_reference.take();
        let Some(layer) = self.mutation.as_mut() else {
            return self.fail_mutation_payload(id, "mutation layer was closed");
        };
        match prepared {
            PreparedMutation::Form(mut form) => {
                if let Some(reference) = seed {
                    let seeded = match layer.action {
                        MutationAction::Close => form.prefill("test", [reference]),
                        MutationAction::AmendAdd => form
                            .prefill("kind", ["test"])
                            .and_then(|()| form.prefill("reference", [reference])),
                        _ => Err(crate::form::DefinitionError::UnknownKey(
                            "configured test evidence target".to_string(),
                        )),
                    };
                    if let Err(error) = seeded {
                        return self.fail_mutation_payload(
                            id,
                            &format!("could not insert configured test evidence: {error}"),
                        );
                    }
                }
                layer.state = MutationState::Form(form);
                self.finish_mutation_effect(id, "form ready");
            }
            PreparedMutation::Preview(candidate) => {
                if seed.is_some() {
                    return self.fail_mutation_payload(
                        id,
                        "configured test evidence target did not return a form",
                    );
                }
                if candidate.action() != layer.action {
                    return self.fail_mutation_payload(
                        id,
                        "prepared preview belongs to another mutation action",
                    );
                }
                let rows = candidate.review_rows();
                layer.state = MutationState::QuickReview {
                    candidate: Some(Box::new(candidate)),
                    form: None,
                    rows,
                };
                self.finish_mutation_effect(id, "preview ready");
            }
        }
        if let Some(layer) = self.mutation.as_mut() {
            layer.plain_scroll = 0;
        }
        self.dirty = true;
        Transition::default()
    }

    fn accept_mutation_commit(&mut self, id: EffectId, result: MutationCommitResult) -> Transition {
        match result {
            MutationCommitResult::Plan {
                outcome,
                reload,
                selection_after_commit,
            } => {
                return self.accept_plan_mutation_commit(
                    id,
                    outcome,
                    reload,
                    selection_after_commit,
                );
            }
            MutationCommitResult::Work { outcome, reload } => {
                let stale = matches!(
                    outcome.as_ref(),
                    aplan_app::WorkActionCommitOutcome::StaleInput { .. }
                        | aplan_app::WorkActionCommitOutcome::Coordination(
                            aplan_app::CoordinationOutcome::Stale { .. }
                        )
                );
                let rows = work_commit_rows(&outcome, matches!(reload, Ok(Some(_))));
                let form = self.mutation.as_mut().and_then(|layer| {
                    let old = std::mem::replace(&mut layer.state, MutationState::Loading);
                    match old {
                        MutationState::QuickReview { form, .. }
                        | MutationState::Result { form, .. } => form,
                        _ => None,
                    }
                });
                if let Some(layer) = self.mutation.as_mut() {
                    layer.state = MutationState::Result { form, rows };
                    layer.plain_scroll = 0;
                }
                if let Ok(Some(reload)) = reload {
                    return self.accept_refresh(id, *reload.snapshot, false);
                }
                if let Err(error) = reload {
                    self.show_notice(format!(
                        "Mutation publication returned, but reloading failed: {error}"
                    ));
                }
                self.finish_mutation_effect(id, "mutation did not publish");
                if stale {
                    self.freshness = SnapshotFreshness::Unconfirmed;
                    return self.request_effect(EffectKind::Refresh, CheckOverrides::default());
                }
            }
        }
        self.dirty = true;
        Transition::default()
    }

    fn accept_plan_mutation_commit(
        &mut self,
        id: EffectId,
        outcome: Box<aplan_app::CommitOutcome>,
        reload: Result<Option<MutationReload>, String>,
        selection_after_commit: Option<String>,
    ) -> Transition {
        let stale = matches!(outcome.as_ref(), aplan_app::CommitOutcome::Stale { .. });
        let Some(layer) = self.mutation.as_mut() else {
            return self.fail_mutation_payload(id, "mutation layer was closed");
        };
        let MutationState::Form(form) = &mut layer.state else {
            return self.fail_mutation_payload(id, "plan commit has no form");
        };
        form.accept_commit_outcome(*outcome);
        let action = layer.action;
        if let Ok(Some(reload)) = &reload
            && let Some(report) = reload.publication.clone()
        {
            let _ = form.accept_reconciliation(&reload.input_sha256, report);
        }
        if let Ok(Some(reload)) = reload {
            let transition = self.accept_refresh(id, *reload.snapshot, false);
            if let Some(selection) = selection_after_commit {
                self.select_after_mutation(action, selection);
            }
            self.dirty = true;
            return transition;
        }
        if let Err(error) = reload {
            self.show_notice(format!(
                "Mutation publication returned, but reloading failed: {error}"
            ));
        }
        self.finish_mutation_effect(id, "mutation did not publish");
        if stale {
            self.freshness = SnapshotFreshness::Unconfirmed;
            return self.request_effect(EffectKind::Refresh, CheckOverrides::default());
        }
        self.dirty = true;
        Transition::default()
    }

    fn select_after_mutation(&mut self, action: MutationAction, selection: String) {
        if action.capture().is_none() {
            self.selected_id = Some(selection);
            self.selection_state = SelectionState::Present;
            self.rebuild_task_browser();
            self.rebuild_work_projection();
            return;
        }
        self.selected_capture_id = Some(selection);
        if self.route != Route::Inbox {
            return;
        }
        self.inbox_query.clear();
        self.inbox_query_cursor = 0;
        self.inbox_kind = None;
        self.inbox_label = None;
        self.inbox_revisit = aplan_core::capture::RevisitFilter::All;
        self.inbox_state = match action {
            MutationAction::CaptureDiscard => Some(aplan_core::capture::CaptureState::Discarded),
            MutationAction::CapturePromote => Some(aplan_core::capture::CaptureState::Promoted),
            MutationAction::CaptureCreate
            | MutationAction::CaptureAmend
            | MutationAction::CaptureRestore => Some(aplan_core::capture::CaptureState::Open),
            _ => unreachable!("capture action checked above"),
        };
        self.reconcile_inbox_selection();
    }

    fn finish_mutation_effect(&mut self, id: EffectId, detail: &str) {
        if let Some(record) = self.effect_mut(id) {
            record.state = EffectState::Complete;
            record.detail = Some(detail.to_string());
        }
    }

    fn fail_mutation_payload(&mut self, id: EffectId, message: &str) -> Transition {
        if let Some(record) = self.effect_mut(id) {
            record.state = EffectState::Failed;
            record.detail = Some(message.to_string());
        }
        self.show_notice(message.to_string());
        Transition::default()
    }

    fn request_effect(&mut self, kind: EffectKind, check_overrides: CheckOverrides) -> Transition {
        self.request_effect_with(kind, check_overrides, false)
    }

    fn request_verification(&mut self, execute: bool) -> Transition {
        self.request_effect_with(EffectKind::Verify, CheckOverrides::default(), execute)
    }

    fn request_external_probe(&mut self) -> Transition {
        if self.onboarding.is_some() || !self.active || self.pending_effects() > 0 {
            return Transition::default();
        }
        self.request_effect(EffectKind::Probe, CheckOverrides::default())
    }

    fn request_effect_with(
        &mut self,
        kind: EffectKind,
        check_overrides: CheckOverrides,
        execute: bool,
    ) -> Transition {
        let valid_for_launch = match kind {
            EffectKind::Startup => self.startup.is_some(),
            EffectKind::Retry => self.onboarding.is_some(),
            EffectKind::Probe
            | EffectKind::Refresh
            | EffectKind::Verify
            | EffectKind::Mutation
            | EffectKind::TestCatalogue => self.onboarding.is_none(),
        };
        if !valid_for_launch {
            self.show_notice(format!(
                "{} is unavailable in the current launch state.",
                kind.label()
            ));
            return Transition::default();
        }
        if let Some(pending) = self
            .effects
            .iter()
            .find(|record| record.state.is_pending() && record.request.kind == kind)
        {
            let equivalent = pending.request.check_overrides == check_overrides
                && pending.request.execute == execute;
            self.show_notice(if equivalent {
                format!("{} already pending; request coalesced.", kind.label())
            } else {
                format!(
                    "{} already pending with different options; this request was not scheduled.",
                    kind.label()
                )
            });
            return Transition::default();
        }
        if self.pending_effects() >= MAX_PENDING_EFFECTS {
            self.show_notice(format!(
                "Effect queue is full ({MAX_PENDING_EFFECTS} running or queued)."
            ));
            return Transition::default();
        }

        let request = EffectRequest {
            id: EffectId(self.next_effect),
            generation: self.generation,
            focus_epoch: self.focus_epoch,
            input: if kind == EffectKind::Verify {
                self.snapshot.verification_sheet.input_sha256.clone()
            } else {
                self.snapshot.overview.input.sha256.clone()
            },
            kind,
            check_overrides,
            execute,
            mutation: None,
        };
        self.next_effect = self.next_effect.saturating_add(1);
        self.make_record_room();
        self.effects.push_back(EffectRecord {
            request: request.clone(),
            state: EffectState::Queued,
            detail: None,
            progress: None,
        });
        if matches!(kind, EffectKind::Refresh | EffectKind::Retry) {
            self.freshness = SnapshotFreshness::Refreshing;
        }
        if kind != EffectKind::Probe {
            self.dirty = true;
        }
        Transition {
            effect: Some(request),
            cancel: None,
        }
    }

    fn cancel_queued(&mut self) -> Transition {
        let Some(id) = self
            .effects
            .iter()
            .rev()
            .find(|record| record.state == EffectState::Queued)
            .map(|record| record.request.id)
        else {
            self.show_notice(
                "No queued effect can be cancelled; running work is not interruptible and quit waits for it."
                    .to_string(),
            );
            return Transition::default();
        };
        Transition {
            effect: None,
            cancel: Some(id),
        }
    }

    pub(crate) fn accept_cancel(&mut self, id: EffectId, cancelled: bool) {
        if cancelled {
            if let Some(record) = self.effect_mut(id) {
                record.state = EffectState::Cancelled;
                record.detail = Some("cancelled before it started".to_string());
            }
            self.show_notice("Queued effect cancelled before it started.".to_string());
        } else {
            self.show_notice(
                "Cancellation arrived after the effect started; it remains running and quit will wait for it."
                    .to_string(),
            );
        }
    }

    fn discard_stale_completion(&mut self, request: &EffectRequest) -> bool {
        let current_input = if request.kind == EffectKind::Verify {
            &self.snapshot.verification_sheet.input_sha256
        } else {
            &self.snapshot.overview.input.sha256
        };
        if request.generation == self.generation && request.input == *current_input {
            return false;
        }
        if let Some(record) = self.effect_mut(request.id) {
            record.state = EffectState::Stale;
            record.detail = Some("result belongs to an older snapshot".to_string());
        }
        if request.kind == EffectKind::TestCatalogue && self.modal == Some(Modal::TestCatalogue) {
            self.test_catalogue = Some(TestCatalogueLayer::Failed(
                "Identified plan or repository input changed while configured tests were being listed; close and retry."
                    .to_string(),
            ));
        }
        self.show_notice(format!("Stale {} result discarded.", request.kind.label()));
        true
    }

    fn accept_test_catalogue(
        &mut self,
        id: EffectId,
        view: Box<crate::evidence::TestCatalogueView>,
    ) {
        let detail = if self.modal == Some(Modal::TestCatalogue) {
            self.modal_selected = 0;
            self.test_catalogue = Some(TestCatalogueLayer::Ready(view));
            "configured test catalogue ready"
        } else {
            "configured test catalogue discarded after selector closed"
        };
        if let Some(record) = self.effect_mut(id) {
            record.state = EffectState::Complete;
            record.detail = Some(detail.to_string());
        }
    }

    fn accept_startup(&mut self, ready: StartupReady, crossed_focus_epoch: bool) -> Transition {
        let appearance = Appearance {
            theme: self.theme,
            glyphs: self.glyphs,
            color: self.color_capability,
            reduced_motion: self.motion == Motion::Reduced,
        };
        let columns = self.columns;
        let rows = self.rows;
        let active = self.active;
        let focus_epoch = self.focus_epoch;
        let mut loaded = Self::with_launch(
            ready.snapshot,
            ready.onboarding,
            ready.capabilities,
            ready.resolution,
            appearance,
        );
        loaded.columns = columns;
        loaded.rows = rows;
        loaded.active = active;
        loaded.focus_epoch = focus_epoch;
        if !active || crossed_focus_epoch {
            loaded.freshness = SnapshotFreshness::Unconfirmed;
        }
        *self = loaded;
        if active && crossed_focus_epoch {
            self.request_external_probe()
        } else {
            Transition::default()
        }
    }

    fn complete_effect(&mut self, completion: EffectCompletion) -> Transition {
        let id = completion.request.id;
        let Some(record) = self.effects.iter().find(|record| record.request.id == id) else {
            self.show_notice(format!(
                "Unrecognized {} completion discarded.",
                completion.request.kind.label()
            ));
            return Transition::default();
        };
        if !record.state.is_pending() || record.request != completion.request {
            self.show_notice(format!(
                "Unexpected {} completion discarded.",
                completion.request.kind.label()
            ));
            return Transition::default();
        }
        if self.discard_stale_completion(&completion.request) {
            return Transition::default();
        }

        let crossed_focus_epoch = completion.request.focus_epoch != self.focus_epoch;
        match (completion.request.kind, completion.outcome) {
            (EffectKind::Startup, Ok(EffectOutput::Startup(ready))) => {
                return self.accept_startup(*ready, crossed_focus_epoch);
            }
            (EffectKind::Startup, Err(error)) => {
                self.startup_failure = Some(error.to_string());
                self.quit = true;
                self.dirty = true;
                return Transition::default();
            }
            (EffectKind::Probe, Ok(EffectOutput::Identity(identity))) => {
                return self.complete_probe(id, &identity, crossed_focus_epoch);
            }
            (EffectKind::Refresh, Ok(EffectOutput::Snapshot(snapshot))) => {
                return self.accept_refresh(id, *snapshot, crossed_focus_epoch);
            }
            (
                EffectKind::Retry,
                Ok(EffectOutput::LaunchSnapshot {
                    snapshot,
                    resolution,
                }),
            ) => {
                self.onboarding = None;
                self.onboarding_scroll = 0;
                self.launch_resolution = Some(resolution);
                self.route = Route::Overview;
                self.focus = Focus::Primary;
                self.modal = None;
                return self.accept_refresh(id, *snapshot, crossed_focus_epoch);
            }
            (EffectKind::Retry, Ok(EffectOutput::Onboarding(onboarding))) => {
                self.launch_resolution = Some(onboarding.resolution());
                self.onboarding = Some(*onboarding);
                self.onboarding_scroll = 0;
                self.freshness = SnapshotFreshness::Unconfirmed;
                if let Some(record) = self.effect_mut(id) {
                    record.state = EffectState::Complete;
                    record.detail = Some("launch inputs are still unavailable".to_string());
                }
                self.show_notice("Retry complete; the launch issue remains.".to_string());
            }
            (EffectKind::Verify, Ok(EffectOutput::Verification(workbench))) => {
                self.accept_verification(id, &completion.request, workbench);
            }
            (EffectKind::Mutation, Ok(EffectOutput::Mutation(transfer))) => {
                return self.accept_mutation(id, &transfer);
            }
            (EffectKind::TestCatalogue, Ok(EffectOutput::TestCatalogue(view))) => {
                self.accept_test_catalogue(id, view);
            }
            (kind, Err(error)) => {
                return self.fail_effect(id, kind, &error);
            }
            (kind, Ok(_)) => {
                if matches!(
                    kind,
                    EffectKind::Probe | EffectKind::Refresh | EffectKind::Retry
                ) {
                    self.freshness = SnapshotFreshness::Unconfirmed;
                }
                let detail = format!("worker returned the wrong result type for {}", kind.label());
                if let Some(record) = self.effect_mut(id) {
                    record.state = EffectState::Failed;
                    record.detail = Some(detail.clone());
                }
                self.show_notice(format!("{} failed: {detail}", kind.label()));
            }
        }
        self.dirty = true;
        Transition::default()
    }

    fn complete_probe(
        &mut self,
        id: EffectId,
        identity: &aplan_app::InputIdentity,
        crossed_focus_epoch: bool,
    ) -> Transition {
        self.effects.retain(|record| record.request.id != id);
        if !self.active || crossed_focus_epoch {
            // Focus may have been lost while the probe was running. Its answer
            // describes one instant before an interval we deliberately do not
            // monitor, so it cannot restore `Current` or start hidden work.
            self.freshness = SnapshotFreshness::Unconfirmed;
            return if self.active {
                self.request_external_probe()
            } else {
                Transition::default()
            };
        }
        if identity.sha256 != self.snapshot.overview.input.sha256 {
            self.freshness = SnapshotFreshness::Unconfirmed;
            self.show_notice(
                "External plan, configuration or repository change detected; refreshing."
                    .to_string(),
            );
            return self.request_effect(EffectKind::Refresh, CheckOverrides::default());
        }
        if self.freshness != SnapshotFreshness::Current {
            self.freshness = SnapshotFreshness::Current;
            self.show_notice(
                "Identified inputs confirmed unchanged; the displayed snapshot is current."
                    .to_string(),
            );
        }
        Transition::default()
    }

    fn fail_effect(&mut self, id: EffectId, kind: EffectKind, error: &EffectFailure) -> Transition {
        let was_current = self.freshness == SnapshotFreshness::Current;
        if matches!(
            kind,
            EffectKind::Probe | EffectKind::Refresh | EffectKind::Retry
        ) {
            self.freshness = SnapshotFreshness::Unconfirmed;
        }
        if kind == EffectKind::Mutation
            && let Some(layer) = self.mutation.as_mut()
        {
            layer.committing = false;
            if matches!(layer.state, MutationState::Loading) {
                layer.state = MutationState::Result {
                    form: None,
                    rows: vec![
                        "Verdict: UNAVAILABLE".to_string(),
                        text::safe(&error.to_string()),
                    ],
                };
                layer.plain_scroll = 0;
            }
        }
        if kind == EffectKind::TestCatalogue && self.modal == Some(Modal::TestCatalogue) {
            self.test_catalogue = Some(TestCatalogueLayer::Failed(error.to_string()));
        }
        let stale = matches!(error, EffectFailure::Stale { .. });
        if let Some(record) = self.effect_mut(id) {
            record.state = if stale {
                EffectState::Stale
            } else if matches!(error, EffectFailure::Unavailable(_)) {
                EffectState::Unavailable
            } else {
                EffectState::Failed
            };
            record.detail = Some(error.to_string());
        }
        if kind == EffectKind::Probe && !was_current {
            // Keep retrying quietly while an already-unconfirmed source is
            // still unloadable; the first transition carried the failure.
            return Transition::default();
        }
        let notice = if stale && kind == EffectKind::Verify {
            "Stale verification result discarded after its identified input changed.".to_string()
        } else if stale && kind == EffectKind::TestCatalogue {
            "Configured test catalogue discarded because the identified input changed; close and retry."
                .to_string()
        } else if stale {
            "Refresh input changed while its replacement was being built; the previous complete snapshot remains visible."
                .to_string()
        } else {
            format!("{} failed: {error}", kind.label())
        };
        self.show_notice(notice);
        Transition::default()
    }

    fn accept_verification(
        &mut self,
        id: EffectId,
        request: &EffectRequest,
        workbench: Box<aplan_app::VerificationWorkbench>,
    ) {
        if workbench.sheet.input_sha256 != request.input
            || workbench.input.sha256 != workbench.sheet.input_sha256
            || workbench.input.trust != workbench.sheet.trust
        {
            if let Some(record) = self.effect_mut(id) {
                record.state = EffectState::Stale;
                record.detail = Some(
                    "verifier returned an identity different from the requested snapshot"
                        .to_string(),
                );
            }
            self.show_notice("Stale verify result discarded.".to_string());
            self.dirty = true;
            return;
        }
        let errors = workbench.run.errors;
        let warnings = workbench.run.warnings;
        let refused = workbench.state == aplan_app::VerificationState::Refused;
        if !refused {
            for inspector in &mut self.snapshot.inspectors {
                aplan_app::apply_verification_to_show(inspector, &workbench);
            }
        }
        self.snapshot.verification_sheet = workbench.sheet.clone();
        self.verification = Some(workbench);
        if let Some(record) = self.effect_mut(id) {
            record.state = if refused {
                EffectState::Refused
            } else {
                EffectState::Complete
            };
            record.detail = refused.then(|| "incomplete input was refused".to_string());
        }
        self.modal = Some(Modal::Verification);
        self.modal_selected = 0;
        self.modal_scroll = 0;
        self.show_notice(if refused {
            "Verification refused: the plan did not load completely.".to_string()
        } else {
            format!("Verification complete: {errors} error(s), {warnings} warning(s).")
        });
    }

    fn accept_refresh(
        &mut self,
        id: EffectId,
        snapshot: Snapshot,
        crossed_focus_epoch: bool,
    ) -> Transition {
        let previous_finding_ids = all_finding_ids(&self.snapshot.diagnostics);
        let next_finding_ids = all_finding_ids(&snapshot.diagnostics);
        let added = next_finding_ids.difference(&previous_finding_ids).count();
        let removed = previous_finding_ids.difference(&next_finding_ids).count();
        let identity_changed = snapshot.overview.input.sha256
            != self.snapshot.overview.input.sha256
            || snapshot.verification_sheet.input_sha256
                != self.snapshot.verification_sheet.input_sha256;
        let selected_id = self.selected_id.clone();
        let selected_capture_id = self.selected_capture_id.clone();
        let previous_input = self.snapshot.overview.input.clone();
        let previous_load = self.snapshot.overview.load_state;
        let work_clock_follows_snapshot = self.work_as_of.is_empty();
        self.snapshot = snapshot;
        if work_clock_follows_snapshot {
            self.work_as_of_epoch_seconds = self.snapshot.overview.as_of_epoch_seconds;
        }
        if self
            .selected_finding_id
            .as_ref()
            .is_some_and(|identity| !next_finding_ids.contains(identity))
        {
            self.selected_finding_id = self
                .diagnostic_run()
                .findings
                .first()
                .map(|finding| finding.identity.clone());
        }
        self.selection_state = if selected_id
            .as_ref()
            .is_some_and(|id| !self.snapshot.task_rows.iter().any(|row| &row.id == id))
        {
            SelectionState::Removed
        } else {
            SelectionState::Present
        };
        self.selected_id = selected_id;
        self.selected_capture_id = selected_capture_id;
        self.refresh_visible_projection(identity_changed);
        self.reconcile_inbox_selection();
        self.reset_inspector_for_snapshot();
        if identity_changed {
            self.generation = self.generation.saturating_add(1);
        }
        if let Some(record) = self.effect_mut(id) {
            record.state = EffectState::Complete;
            record.detail = None;
        }
        // A coherent replacement may complete after focus was lost. It is
        // safe to display, but an inactive session is not continuously
        // observed and therefore cannot claim that replacement is current.
        self.freshness = if self.active && !crossed_focus_epoch {
            SnapshotFreshness::Current
        } else {
            SnapshotFreshness::Unconfirmed
        };
        let mut changes = refresh_identity_changes(
            &previous_input,
            &self.snapshot.overview.input,
            previous_load,
            self.snapshot.overview.load_state,
        );
        if self.selection_state == SelectionState::Removed
            && let Some(selected) = &self.selected_id
        {
            changes.push(format!("selected task `{selected}` was removed"));
        }
        if identity_changed
            && let Some(anchor) = self
                .graph_window
                .anchor
                .as_ref()
                .filter(|anchor| anchor.replacement_reason.is_some())
        {
            changes.push(format!(
                "graph anchor is now `{}` because {}",
                anchor.selected,
                anchor
                    .replacement_reason
                    .as_deref()
                    .expect("filtered above")
            ));
        }
        let change_summary = if changes.is_empty() {
            "identified inputs unchanged".to_string()
        } else {
            changes.join("; ")
        };
        self.show_notice(format!(
            "Refresh complete: {added} new, {removed} removed finding(s); {change_summary}."
        ));
        if self.active && crossed_focus_epoch {
            self.request_external_probe()
        } else {
            Transition::default()
        }
    }

    fn refresh_visible_projection(&mut self, identity_changed: bool) {
        match self.route {
            Route::Tasks => self.rebuild_task_browser(),
            Route::Graph => self.refresh_graph(identity_changed),
            Route::Reports => self.rebuild_report(),
            Route::Diagnostics => self.rebuild_diagnostics(),
            Route::Inbox => self.reconcile_inbox_selection(),
            Route::Overview => {}
        }
        if self.route != Route::Tasks {
            self.rebuild_work_projection();
        }
        if self.modal == Some(Modal::Filters) {
            let count = if self.route == Route::Inbox {
                self.inbox_filter_choices().len()
            } else if self.route == Route::Diagnostics {
                self.diagnostic_filter_choices().len()
            } else {
                self.filter_choices().len()
            };
            self.modal_selected = self.modal_selected.min(count.saturating_sub(1));
        }
    }

    fn effect_mut(&mut self, id: EffectId) -> Option<&mut EffectRecord> {
        self.effects
            .iter_mut()
            .find(|record| record.request.id == id)
    }

    fn accept_effect_progress(&mut self, id: EffectId, progress: &aplan_app::ProgressEvent) {
        if self.effects.iter().any(|record| {
            record.request.id == id
                && record.state == EffectState::Running
                && record.request.kind == EffectKind::Startup
        }) {
            if self
                .startup
                .as_mut()
                .is_some_and(|startup| startup.accept(*progress))
            {
                self.dirty = true;
            }
            return;
        }
        if let Some(record) = self.effect_mut(id)
            && record.state == EffectState::Running
            && record.request.kind == EffectKind::Verify
        {
            let state = record.progress.get_or_insert_with(|| {
                aplan_app::ProgressState::new(
                    aplan_app::OperationId::new(id.0),
                    aplan_app::OperationKind::Verification,
                )
            });
            if state.accept(*progress).is_err() {
                return;
            }
            record.detail = Some(format_progress_detail(*progress));
            self.dirty = true;
        }
    }

    fn make_record_room(&mut self) {
        if self.effects.len() < MAX_EFFECT_RECORDS {
            return;
        }
        if let Some(index) = self
            .effects
            .iter()
            .position(|record| !record.state.is_pending())
        {
            self.effects.remove(index);
        }
    }

    fn show_notice(&mut self, text: String) {
        self.notice = Some(Notice {
            text,
            ticks_left: NOTICE_TICKS,
        });
        self.dirty = true;
    }

    pub(crate) fn pending_effects(&self) -> usize {
        self.effects
            .iter()
            .filter(|record| record.state.is_pending())
            .count()
    }

    pub(crate) fn has_pending_kind(&self, kind: EffectKind) -> bool {
        self.effects
            .iter()
            .any(|record| record.state.is_pending() && record.request.kind == kind)
    }

    pub(crate) fn should_probe_external(&self) -> bool {
        self.startup.is_none()
            && self.onboarding.is_none()
            && self.active
            && self.pending_effects() == 0
    }

    pub(crate) fn has_running_current_epoch_observation(&self) -> bool {
        self.effects.iter().any(|record| {
            record.state == EffectState::Running
                && matches!(record.request.kind, EffectKind::Probe | EffectKind::Refresh)
                && record.request.focus_epoch == self.focus_epoch
        })
    }

    pub(crate) fn has_queued_effect(&self) -> bool {
        self.effects
            .iter()
            .any(|record| record.state == EffectState::Queued)
    }

    pub(crate) const fn can_cycle_theme(&self) -> bool {
        !matches!(self.color_capability, ColorCapability::None)
    }

    pub(crate) fn needs_tick(&self) -> bool {
        (self.startup.is_some() && self.motion == Motion::Animated)
            || self.notice.is_some()
            || (self.motion == Motion::Animated
                && self.effects.iter().any(|record| {
                    record.state.is_pending() && record.request.kind != EffectKind::Probe
                }))
    }

    pub(crate) fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub(crate) const fn should_quit(&self) -> bool {
        self.quit
    }

    pub(crate) fn take_startup_failure(&mut self) -> Option<String> {
        self.startup_failure.take()
    }

    pub(crate) fn status_line(&self) -> String {
        if let Some(notice) = &self.notice {
            return notice.text.clone();
        }
        if let Some(record) = self.effects.iter().find(|record| {
            record.state == EffectState::Running && record.request.kind != EffectKind::Probe
        }) {
            const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
            let detail = record
                .detail
                .as_deref()
                .map_or_else(String::new, |detail| format!(": {detail}"));
            return format!(
                "{} {} in progress{detail}",
                if self.motion == Motion::Reduced {
                    "*"
                } else {
                    FRAMES[usize::from(self.animation_frame)]
                },
                record.request.kind.label()
            );
        }
        if let Some(record) = self.effects.iter().find(|record| {
            record.state == EffectState::Queued && record.request.kind != EffectKind::Probe
        }) {
            return format!("{} queued", record.request.kind.label());
        }
        if let Some(onboarding) = &self.onboarding {
            return format!(
                "{}; repair the input, then press r",
                onboarding.cause.label()
            );
        }
        match self.verification.as_deref() {
            Some(verification) => {
                format!(
                    "Last verify: {} error(s), {} warning(s)",
                    verification.run.errors, verification.run.warnings
                )
            }
            None => "Idle".to_string(),
        }
    }
}

fn inspector_summary(show: &aplan_app::Show) -> Vec<InspectorItem> {
    let mut items = vec![InspectorItem::identity(&show.id, &show.title)];
    items.extend(summary_state(show));
    items.push(InspectorItem::labelled(
        "RESPONSIBILITY",
        "Owner",
        show.owner.as_deref().unwrap_or("none"),
    ));
    push_claim_summary(&mut items, show);
    items.extend(summary_placement(show));
    items.extend(summary_planning(show));
    items.extend(summary_detail_links(show));
    items
}

fn summary_state(show: &aplan_app::Show) -> [InspectorItem; 2] {
    [
        InspectorItem::badge(
            "STATE",
            "Status",
            &show.status,
            status_tone(&show.status, show.ready),
        ),
        InspectorItem::badge(
            "STATE",
            "Readiness",
            if show.workable {
                if show.ready {
                    "ready to start"
                } else {
                    "waiting on prerequisites"
                }
            } else {
                "not applicable to terminal work or milestones"
            },
            if show.workable && show.ready {
                InspectorTone::Good
            } else if show.workable {
                InspectorTone::Warning
            } else {
                InspectorTone::Muted
            },
        ),
    ]
}

fn summary_placement(show: &aplan_app::Show) -> [InspectorItem; 4] {
    [
        InspectorItem::metric("PLACEMENT", "Phase", show.phase.to_string(), None),
        InspectorItem::labelled("PLACEMENT", "Cut", show.cut.as_deref().unwrap_or("none")),
        InspectorItem::labelled(
            "PLACEMENT",
            "Milestone",
            show.milestone.as_deref().unwrap_or("none"),
        ),
        InspectorItem::labelled(
            "PLACEMENT",
            "Tags",
            if show.tags.is_empty() {
                "none".to_string()
            } else {
                show.tags.join(", ")
            },
        ),
    ]
}

fn summary_planning(show: &aplan_app::Show) -> Vec<InspectorItem> {
    let mut items = vec![InspectorItem::metric(
        "PLANNING",
        "Effort",
        show.effort_days.map_or_else(
            || "not estimated".to_string(),
            |effort| format!("{effort}d"),
        ),
        None,
    )];
    for (label, value) in [
        ("Revisit at", show.revisit_at.as_deref()),
        (
            "Version impact reason",
            show.version_impact_reason.as_deref(),
        ),
        ("Version", show.version.as_deref()),
    ] {
        if let Some(value) = value {
            items.push(InspectorItem::labelled("PLANNING", label, value));
        }
    }
    if let Some(impact) = show.version_impact {
        items.push(InspectorItem::badge(
            "PLANNING",
            "Version impact",
            format!("{impact:?}").to_lowercase(),
            InspectorTone::Accent,
        ));
    }
    items
}

fn summary_detail_links(show: &aplan_app::Show) -> [InspectorItem; 7] {
    [
        InspectorItem::section(
            "DETAILS",
            "Work",
            "open",
            Some("ready rank, why, due and releases".to_string()),
            InspectorSection::Work,
        ),
        InspectorItem::section(
            "DETAILS",
            "Body",
            if show.body.is_some() {
                "present"
            } else {
                "absent"
            },
            None,
            InspectorSection::Body,
        ),
        InspectorItem::section(
            "DETAILS",
            "Relationships",
            (show.waits_on.len()
                + show.releases.len()
                + show.prefer_after.len()
                + show.ratified_by.len()
                + show.contains.len())
            .to_string(),
            Some(format!(
                "{} blocker(s), {} release(s), {} preference(s), {} ratifier(s), {} member(s)",
                show.waits_on.len(),
                show.releases.len(),
                show.prefer_after.len(),
                show.ratified_by.len(),
                show.contains.len()
            )),
            InspectorSection::Relationships,
        ),
        InspectorItem::section(
            "DETAILS",
            "Paths",
            show.paths.len().to_string(),
            None,
            InspectorSection::Paths,
        ),
        InspectorItem::section(
            "DETAILS",
            "Traces",
            show.traces.len().to_string(),
            None,
            InspectorSection::Traces,
        ),
        InspectorItem::section(
            "DETAILS",
            "Evidence",
            show.evidence.len().to_string(),
            None,
            InspectorSection::Evidence,
        ),
        InspectorItem::section(
            "DETAILS",
            "Findings",
            show.findings.len().to_string(),
            Some(if show.resolved {
                "evidence resolved".to_string()
            } else {
                "hermetic rules only".to_string()
            }),
            InspectorSection::Findings,
        ),
    ]
}

fn status_tone(status: &str, ready: bool) -> InspectorTone {
    match status {
        "done" | "dropped" => InspectorTone::Muted,
        "blocked" => InspectorTone::Warning,
        "in_progress" => InspectorTone::Accent,
        "todo" if ready => InspectorTone::Good,
        _ => InspectorTone::Normal,
    }
}

fn push_claim_summary(items: &mut Vec<InspectorItem>, show: &aplan_app::Show) {
    if let Some(claim) = &show.claim {
        let age = show.claim_age_seconds.map_or_else(
            || "unknown age".to_string(),
            |seconds| format!("{seconds}s"),
        );
        items.push(InspectorItem::badge(
            "RESPONSIBILITY",
            "Claim",
            format!("held by {} for {age}", claim.holder),
            InspectorTone::Accent,
        ));
        let mut origin = Vec::new();
        if let Some(host) = &claim.origin.host {
            origin.push(format!("host {host}"));
        }
        if let Some(worktree) = &claim.origin.worktree {
            origin.push(format!("worktree {worktree}"));
        }
        items.push(InspectorItem::labelled(
            "RESPONSIBILITY",
            "Claim source",
            if origin.is_empty() {
                format!("{}; origin unknown", claim.identity_source)
            } else {
                format!("{}; {}", claim.identity_source, origin.join(", "))
            },
        ));
    } else if show.coordination.available {
        items.push(InspectorItem::badge(
            "RESPONSIBILITY",
            "Claim",
            "unclaimed",
            InspectorTone::Muted,
        ));
    } else {
        items.push(InspectorItem::callout(
            "RESPONSIBILITY",
            "Claim unavailable",
            show.coordination
                .reason
                .as_deref()
                .unwrap_or("no local coordination scope"),
            InspectorTone::Warning,
        ));
    }
}

fn work_items(projection: &aplan_app::WorkDiscoveryProjection) -> Vec<InspectorItem> {
    let mut items = vec![work_verdict_item(projection)];
    items.extend(work_query_items(projection));
    items.push(work_priority_item(projection));
    items.extend(work_preference_items(projection));
    items.push(work_revisit_item(projection));
    if projection.blockers.is_empty() {
        items.push(InspectorItem::callout(
            "BLOCKERS",
            format!("Why ({})", projection.traversal.label()),
            "no unmet graph blockers",
            InspectorTone::Good,
        ));
    } else {
        items.extend(
            projection
                .blockers
                .iter()
                .map(|blocker| work_relation_item("BLOCKERS", "Blocked by", blocker)),
        );
    }
    if projection.releases.is_empty() {
        items.push(InspectorItem::callout(
            "DEPENDENTS",
            "Releases",
            "no direct dependent tasks",
            InspectorTone::Muted,
        ));
    } else {
        items.extend(
            projection
                .releases
                .iter()
                .map(|released| work_relation_item("DEPENDENTS", "Releases", released)),
        );
    }
    items
}

fn work_verdict_item(projection: &aplan_app::WorkDiscoveryProjection) -> InspectorItem {
    use aplan_app::WorkVerdict;

    let (verdict, tone) = match projection.verdict {
        WorkVerdict::Ready => (
            "ordinary work can start now".to_string(),
            InspectorTone::Good,
        ),
        WorkVerdict::Blocked => (
            "not ready; unmet graph blockers follow".to_string(),
            InspectorTone::Warning,
        ),
        WorkVerdict::Parked => (
            "explicitly parked; graph completion cannot release it".to_string(),
            InspectorTone::Warning,
        ),
        WorkVerdict::CloseableCut => (
            "not work; every member is complete, so close this cut".to_string(),
            InspectorTone::Accent,
        ),
        WorkVerdict::UnblockedCut => (
            "not work; this is an unblocked cut checkpoint".to_string(),
            InspectorTone::Accent,
        ),
        WorkVerdict::Terminal(status) => (
            format!("not applicable; terminal status `{status}`"),
            InspectorTone::Muted,
        ),
    };
    InspectorItem::badge("VERDICT", "Ready", verdict, tone)
}

fn work_query_items(projection: &aplan_app::WorkDiscoveryProjection) -> [InspectorItem; 2] {
    [
        InspectorItem::labelled(
            "QUERY",
            "Why traversal",
            format!(
                "{} (press w for {})",
                projection.traversal.label(),
                projection.traversal.toggled().label()
            ),
        ),
        InspectorItem::labelled(
            "QUERY",
            "Due as-of",
            format!(
                "epoch {} (press d to change)",
                projection.as_of_epoch_seconds
            ),
        ),
    ]
}

fn work_priority_item(projection: &aplan_app::WorkDiscoveryProjection) -> InspectorItem {
    projection.rank.as_ref().map_or_else(
        || {
            InspectorItem::metric(
                "PRIORITY",
                "Ready rank",
                "not ranked",
                Some("this task is not ready work".to_string()),
            )
        },
        |rank| {
            InspectorItem::metric(
                "PRIORITY",
                "Ready rank",
                format!("#{}", rank.position),
                Some(format!(
                    "releases {}; critical {}; downstream depth {}; effective effort {}d",
                    rank.releases,
                    if rank.critical { "yes" } else { "no" },
                    rank.downstream_depth,
                    rank.effort_days
                )),
            )
        },
    )
}

fn work_revisit_item(projection: &aplan_app::WorkDiscoveryProjection) -> InspectorItem {
    use aplan_app::RevisitState;

    let (revisit, revisit_tone) = match &projection.revisit {
        RevisitState::None => ("none scheduled".to_string(), InspectorTone::Muted),
        RevisitState::Waiting { at, seconds_until } => (
            format!(
                "waiting until {at} ({seconds_until}s away); condition satisfied: not asserted"
            ),
            InspectorTone::Normal,
        ),
        RevisitState::Due {
            at,
            overdue_seconds,
        } => (
            format!(
                "DUE since {at} ({overdue_seconds}s overdue); condition satisfied: not asserted"
            ),
            InspectorTone::Warning,
        ),
        RevisitState::Invalid { at } => (
            format!("invalid timestamp `{at}`; validation finding applies"),
            InspectorTone::Error,
        ),
        RevisitState::NotParked { at } => (
            format!(
                "`{at}` is on non-parked work and is not a due signal; validation finding applies"
            ),
            InspectorTone::Warning,
        ),
    };
    InspectorItem::badge("SCHEDULE", "Revisit", revisit, revisit_tone)
}

fn work_preference_items(projection: &aplan_app::WorkDiscoveryProjection) -> Vec<InspectorItem> {
    if projection.preferences.is_empty() {
        return vec![InspectorItem::callout(
            "PREFERENCES",
            "Ready preferences",
            "none declared",
            InspectorTone::Muted,
        )];
    }
    projection
        .preferences
        .iter()
        .map(|preference| {
            let active = projection
                .rank
                .as_ref()
                .is_some_and(|rank| rank.active_preferences.contains(&preference.id));
            let state = if active {
                "active in this ready answer"
            } else if projection.rank.is_none() {
                "declared; this task is not currently offered"
            } else {
                "declared; preferred task is not currently offered"
            };
            work_relation_item(
                "PREFERENCES",
                &format!("Prefers after ({state})"),
                preference,
            )
        })
        .collect()
}

fn work_relation_item(
    group: &'static str,
    kind: &str,
    related: &aplan_app::WorkRelation,
) -> InspectorItem {
    let availability = match &related.availability {
        aplan_app::WorkRelationAvailability::Loaded => related
            .status
            .map_or_else(|| "loaded".to_string(), |status| status.to_string()),
        aplan_app::WorkRelationAvailability::Missing => "undefined blocker".to_string(),
        aplan_app::WorkRelationAvailability::Unloadable(path) => {
            format!("unloadable at {}", path.display())
        }
    };
    let cut = if related.is_cut { "; cut" } else { "" };
    let folded = related.folded_unmet.map_or_else(String::new, |count| {
        format!("; folds {count} unmet task(s); press w for complete")
    });
    let title = related
        .title
        .as_deref()
        .map_or_else(String::new, |title| format!(" - {title}"));
    let value = format!("{} ({availability}{cut}{folded}){title}", related.id);
    if related.availability == aplan_app::WorkRelationAvailability::Loaded {
        InspectorItem::task(group, kind, value, &related.id)
    } else {
        InspectorItem::callout(group, kind, value, InspectorTone::Warning)
    }
}

fn relationship_item(kind: &str, related: &aplan_app::Related) -> InspectorItem {
    let availability = match related.availability {
        aplan_app::RelatedAvailability::Loaded => related.status.clone(),
        aplan_app::RelatedAvailability::Missing => "missing reference".to_string(),
        aplan_app::RelatedAvailability::Unloadable => related.source.as_ref().map_or_else(
            || "unloadable task".to_string(),
            |source| format!("unloadable task at {source}"),
        ),
    };
    let group = match kind {
        "Blocked by" => "BLOCKERS",
        "Releases" => "DEPENDENTS",
        "Prefers after" => "PREFERENCES",
        "Ratified by" => "RATIFICATION",
        "Contains" => "MEMBERS",
        _ => "RELATIONSHIPS",
    };
    let value = format!("{} ({availability})", related.id);
    if related.availability == aplan_app::RelatedAvailability::Loaded {
        InspectorItem::task(group, kind, value, &related.id)
    } else {
        InspectorItem::callout(group, kind, value, InspectorTone::Warning)
            .with_link(InspectorLink::Task(related.id.clone()))
    }
}

fn inspector_relationships(show: &aplan_app::Show) -> Vec<InspectorItem> {
    let mut items = Vec::new();
    for (kind, relations) in [
        ("Blocked by", &show.waits_on),
        ("Releases", &show.releases),
        ("Prefers after", &show.prefer_after),
        ("Ratified by", &show.ratified_by),
        ("Contains", &show.contains),
    ] {
        items.extend(
            relations
                .iter()
                .map(|related| relationship_item(kind, related)),
        );
    }
    if items.is_empty() {
        items.push(InspectorItem::callout(
            "RELATIONSHIPS",
            "None",
            "No relationships declared or resolved.",
            InspectorTone::Muted,
        ));
    }
    items
}

fn strings_or_empty(
    values: &[String],
    group: &'static str,
    label: &str,
    empty: &str,
) -> Vec<InspectorItem> {
    if values.is_empty() {
        vec![InspectorItem::callout(
            group,
            "None",
            empty,
            InspectorTone::Muted,
        )]
    } else {
        values
            .iter()
            .cloned()
            .map(|value| InspectorItem::labelled(group, label, value))
            .collect()
    }
}

fn inspector_traces(show: &aplan_app::Show) -> Vec<InspectorItem> {
    let mut items: Vec<_> = show
        .traces
        .iter()
        .map(|trace| {
            InspectorItem::labelled(
                "DECLARED TRACES",
                "Trace",
                show.trace_fingerprints.get(trace).map_or_else(
                    || format!("{trace} (unstamped)"),
                    |fingerprint| format!("{trace} (fingerprint {fingerprint})"),
                ),
            )
        })
        .collect();
    items.extend(
        show.trace_fingerprints
            .iter()
            .filter(|(trace, _)| !show.traces.contains(trace))
            .map(|(trace, fingerprint)| {
                InspectorItem::callout(
                    "ORPHANED STAMPS",
                    "Trace",
                    format!(
                        "{trace} (orphaned fingerprint {fingerprint}; trace is no longer declared)"
                    ),
                    InspectorTone::Warning,
                )
            }),
    );
    if items.is_empty() {
        items.push(InspectorItem::callout(
            "DECLARED TRACES",
            "None",
            "No traces declared.",
            InspectorTone::Muted,
        ));
    }
    items
}

fn inspector_evidence(show: &aplan_app::Show) -> Vec<InspectorItem> {
    let mut live_keys = Vec::new();
    let mut items: Vec<_> = show
        .evidence
        .iter()
        .map(|evidence| {
            let key = format!("{}:{}", evidence.kind, evidence.reference);
            live_keys.push(key.clone());
            let captured = evidence
                .captured
                .as_ref()
                .map_or_else(String::new, |value| format!("; captured {value}"));
            let fingerprint = show.evidence_fingerprints.get(&key).map_or_else(
                || "no mutable-content fingerprint".to_string(),
                |fingerprint| format!("fingerprint {fingerprint}"),
            );
            InspectorItem::task(
                "CITATIONS",
                format!("{} citation", evidence.kind),
                format!("{}{captured}; {fingerprint}", evidence.reference),
                &show.id,
            )
        })
        .collect();
    items.extend(
        show.evidence_fingerprints
            .iter()
            .filter(|(citation, _)| !live_keys.contains(citation))
            .map(|(citation, fingerprint)| {
                InspectorItem::task(
                    "ORPHANED STAMPS",
                    "Citation",
                    format!(
                        "{citation}; orphaned fingerprint {fingerprint}; citation is no longer declared"
                    ),
                    &show.id,
                )
            }),
    );
    if items.is_empty() {
        items.push(InspectorItem::callout(
            "CITATIONS",
            "None",
            "No evidence cited.",
            InspectorTone::Muted,
        ));
    }
    items
}

fn inspector_findings(show: &aplan_app::Show) -> Vec<InspectorItem> {
    if show.findings.is_empty() {
        return vec![InspectorItem::callout(
            "PLAN HEALTH",
            "No findings",
            if show.resolved {
                "Evidence was resolved for this snapshot."
            } else {
                "Hermetic rules are clean; evidence has not been resolved."
            },
            if show.resolved {
                InspectorTone::Good
            } else {
                InspectorTone::Warning
            },
        )];
    }
    show.findings
        .iter()
        .map(|finding| {
            let fix = finding
                .fix
                .as_ref()
                .map_or_else(String::new, |fix| format!("; instruction: {fix}"));
            let repair = finding
                .repair
                .as_ref()
                .map_or_else(String::new, |repair| format!("; repair: {repair:?}"));
            let tone = match finding.severity.as_str() {
                "error" => InspectorTone::Error,
                "warning" => InspectorTone::Warning,
                _ => InspectorTone::Normal,
            };
            InspectorItem::callout(
                "PLAN HEALTH",
                format!("{} {}", finding.severity, finding.code),
                format!("{}{fix}{repair}", finding.message),
                tone,
            )
            .with_link(InspectorLink::Task(show.id.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ModelBuilder, capture_record, graph_task, task_inspector, task_row};
    use std::collections::BTreeSet;

    fn snapshot(input: &str, rows: usize) -> Snapshot {
        let task_rows: Vec<_> = (0..rows)
            .map(|index| task_row(format!("T-{index}"), format!("task {index}")))
            .collect();
        let inspectors = task_rows.iter().map(task_inspector).collect();
        Snapshot::test(input, "repo", task_rows, inspectors, 0, 0)
    }

    fn mutation_context_and_snapshot(
        plan: &std::path::Path,
        repository: &std::path::Path,
    ) -> (PlanContext, Snapshot) {
        let context = PlanContext::new(
            aplan_core::load_plan_identified(plan),
            aplan_app::ContextInputs {
                repository_root: repository.to_path_buf(),
                configuration_file: None,
                trust: aplan_app::Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        );
        let sheet =
            aplan_app::verification_run_sheet(&context, &aplan_evidence::VerifyOptions::default());
        let snapshot = Snapshot::from_context(&context, &BuildIdentity::default(), sheet);
        (context, snapshot)
    }

    fn completion(request: EffectRequest, outcome: Result<EffectOutput, EffectFailure>) -> Action {
        Action::EffectCompleted(Box::new(EffectCompletion { request, outcome }))
    }

    fn git(root: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {arguments:?}: {output:?}");
    }

    fn verification(input: &str, errors: usize, warnings: usize) -> EffectOutput {
        let snapshot = Snapshot::test(input, "repo", Vec::new(), Vec::new(), errors, warnings);
        let mut identity = snapshot.overview.input.clone();
        identity
            .sha256
            .clone_from(&snapshot.verification_sheet.input_sha256);
        identity.trust = snapshot.verification_sheet.trust;
        EffectOutput::Verification(Box::new(aplan_app::VerificationWorkbench {
            input: identity,
            sheet: snapshot.verification_sheet,
            execute: false,
            state: aplan_app::VerificationState::Completed,
            observation: aplan_app::EvidenceObservation::current(),
            run: snapshot.diagnostics.check,
            citations: Vec::new(),
            costs: Vec::new(),
            repairs: Vec::new(),
        }))
    }

    fn test_catalogue_view(
        choices: Vec<crate::evidence::TestEvidenceChoice>,
    ) -> crate::evidence::TestCatalogueView {
        crate::evidence::TestCatalogueView {
            catalogue: aplan_app::TestCatalogue {
                state: aplan_app::TestCatalogueState::Complete,
                query: None,
                summary: aplan_app::TestCatalogueSummary {
                    runners: 1,
                    listed_runners: 1,
                    empty_runners: 0,
                    failed_runners: 0,
                    unique_names: choices.len(),
                    runner_name_pairs: choices.len(),
                    matched_names: choices.len(),
                    matched_pairs: choices.len(),
                    ambiguous_names: choices.iter().filter(|choice| !choice.selectable).count(),
                },
                runners: Vec::new(),
                findings: Vec::new(),
                denied: false,
            },
            choices,
        }
    }

    /// Proof for `tui.configured_test_selector_is_reachable`. A catalogue helper called only
    /// by tests is not a product surface. This drives the palette action's
    /// worker result through local search and into the ordinary close form.
    #[test]
    fn configured_test_selector_is_reachable_and_inserts_the_exact_reference() {
        let mut model = Model::new(snapshot("one", 1));
        let request = model
            .update(Action::OpenTestCatalogue)
            .effect
            .expect("catalogue worker request");
        assert_eq!(request.kind, EffectKind::TestCatalogue);
        assert_eq!(model.modal, Some(Modal::TestCatalogue));
        assert_eq!(model.test_catalogue, Some(TestCatalogueLayer::Loading));
        model.update(Action::EffectStarted(request.id));
        model.update(completion(
            request,
            Ok(EffectOutput::TestCatalogue(Box::new(test_catalogue_view(
                vec![
                    crate::evidence::TestEvidenceChoice {
                        name: "other::case".to_string(),
                        reference: "other::case".to_string(),
                        citation: "test:other::case".to_string(),
                        selectable: true,
                        advice: "accepted reference".to_string(),
                    },
                    crate::evidence::TestEvidenceChoice {
                        name: "suite::proof".to_string(),
                        reference: "suite::proof".to_string(),
                        citation: "test:suite::proof".to_string(),
                        selectable: true,
                        advice: "accepted reference".to_string(),
                    },
                ],
            )))),
        ));
        model.update(Action::SearchInput("PROOF".to_string()));
        assert_eq!(
            model
                .selected_test_choice()
                .map(|choice| choice.reference.as_str()),
            Some("suite::proof")
        );

        let prepare = model
            .update(Action::UseTestEvidence)
            .effect
            .expect("seeded close preparation");
        assert_eq!(prepare.kind, EffectKind::Mutation);
        assert_eq!(model.modal, None);
        model.update(Action::EffectStarted(prepare.id));
        let form = MutationForm::new(
            "close",
            "one",
            false,
            vec![
                crate::form::FieldSpec::new("id", "Task", "task").required(),
                crate::form::FieldSpec::new("test", "Tests", "test evidence").repeated(),
            ],
        )
        .expect("close form");
        model.update(completion(
            prepare,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Form(Box::new(form))),
            ))),
        ));
        let Some(MutationLayer {
            state: MutationState::Form(form),
            ..
        }) = model.mutation.as_ref()
        else {
            panic!("configured test selector did not open a close form")
        };
        let test = form
            .fields()
            .iter()
            .find(|field| field.spec().key() == "test")
            .expect("test field");
        assert_eq!(test.values().collect::<Vec<_>>(), vec!["suite::proof"]);
    }

    /// Proof for `tui.external_editor_handoff_is_safe_and_identity_bound`: the
    /// body returned after terminal ownership is released remains attached to
    /// the exact Add form and travels with its ordinary preview request rather
    /// than bypassing review or replacing the visible short field.
    #[test]
    fn external_editor_body_is_bound_to_the_add_preview_request() {
        let mut model = Model::new(snapshot("one", 1));
        let form = MutationForm::new(
            "Add task",
            "one",
            false,
            vec![crate::form::FieldSpec::new("body", "Body", "short body")],
        )
        .expect("add form");
        model.mutation = Some(MutationLayer {
            action: MutationAction::Add,
            selected_task: None,
            state: MutationState::Form(Box::new(form)),
            edited_body: None,
            committing: false,
            plain_scroll: 0,
            selection_after_commit: None,
        });

        model.update(Action::OpenExternalEditor);
        assert_eq!(model.take_external_editor_request().as_deref(), Some(""));
        model.accept_external_editor_body("## Why\n\nEdited outside.\n".to_string());
        let layer = model.mutation.as_ref().expect("Add layer retained");
        assert!(
            layer
                .rows(100, 30)
                .iter()
                .any(|row| row.contains("External body: 24 byte(s); Ctrl-E reopens it")),
            "the preferred body is visible before review"
        );

        let effect = model
            .update(Action::MutationForm(FormAction::Review))
            .effect
            .expect("Add preview request");
        let Some(MutationEffect::Preview {
            action: MutationAction::Add,
            edited_body: Some(body),
            ..
        }) = effect.mutation
        else {
            panic!("external body did not reach the Add preview")
        };
        assert_eq!(&*body, "## Why\n\nEdited outside.\n");
    }

    #[test]
    fn configured_test_selector_refuses_ambiguous_names_without_leaving_the_catalogue() {
        let mut model = Model::new(snapshot("one", 1));
        model.modal = Some(Modal::TestCatalogue);
        model.test_catalogue = Some(TestCatalogueLayer::Ready(Box::new(test_catalogue_view(
            vec![crate::evidence::TestEvidenceChoice {
                name: "shared".to_string(),
                reference: "shared".to_string(),
                citation: "test:shared".to_string(),
                selectable: false,
                advice: "refused: listed by two configured runners".to_string(),
            }],
        ))));
        let transition = model.update(Action::UseTestEvidence);
        assert!(transition.effect.is_none());
        assert_eq!(model.modal, Some(Modal::TestCatalogue));
        assert!(
            model
                .notice
                .as_ref()
                .is_some_and(|notice| notice.text.contains("refused"))
        );
    }

    #[test]
    fn configured_test_selector_uses_amend_add_for_terminal_tasks() {
        let mut state = snapshot("one", 1);
        state.inspectors[0].status = "done".to_string();
        let mut model = Model::new(state);
        model.modal = Some(Modal::TestCatalogue);
        model.test_catalogue = Some(TestCatalogueLayer::Ready(Box::new(test_catalogue_view(
            vec![crate::evidence::TestEvidenceChoice {
                name: "suite::proof".to_string(),
                reference: "suite::proof".to_string(),
                citation: "test:suite::proof".to_string(),
                selectable: true,
                advice: "accepted reference".to_string(),
            }],
        ))));

        let request = model
            .update(Action::UseTestEvidence)
            .effect
            .expect("seeded amend preparation");
        let Some(crate::mutations::MutationEffect::Prepare { action, .. }) = request.mutation
        else {
            panic!("configured test selector did not request mutation preparation")
        };
        assert_eq!(action, MutationAction::AmendAdd);
    }

    #[test]
    fn configured_test_selector_rejects_stale_results_and_cancels_queued_listing() {
        let mut model = Model::new(snapshot("one", 1));
        let request = model
            .update(Action::OpenTestCatalogue)
            .effect
            .expect("catalogue worker request");
        let transition = model.update(Action::CloseLayer);
        assert_eq!(transition.cancel, Some(request.id));
        assert_eq!(model.modal, None);
        assert_eq!(model.test_catalogue, None);
        model.update(Action::EffectStarted(request.id));
        model.update(completion(
            request,
            Ok(EffectOutput::TestCatalogue(Box::new(test_catalogue_view(
                Vec::new(),
            )))),
        ));
        assert!(model.effects.back().is_some_and(|record| {
            record.state == EffectState::Complete
                && record
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("selector closed"))
        }));

        let mut model = Model::new(snapshot("one", 1));
        let request = model
            .update(Action::OpenTestCatalogue)
            .effect
            .expect("second catalogue worker request");
        model.update(Action::EffectStarted(request.id));
        model.snapshot.overview.input.sha256 = "new-input".to_string();
        model.update(completion(
            request,
            Ok(EffectOutput::TestCatalogue(Box::new(test_catalogue_view(
                Vec::new(),
            )))),
        ));
        assert!(matches!(
            model.test_catalogue,
            Some(TestCatalogueLayer::Failed(ref message))
                if message.contains("input changed")
        ));
        assert!(model.selected_test_choice().is_none());
    }

    #[test]
    fn configured_test_selector_requires_a_loaded_selected_task() {
        let mut model = Model::new(snapshot("one", 1));
        model.selected_id = None;
        model.modal = Some(Modal::TestCatalogue);
        model.test_catalogue = Some(TestCatalogueLayer::Ready(Box::new(test_catalogue_view(
            vec![crate::evidence::TestEvidenceChoice {
                name: "suite::proof".to_string(),
                reference: "suite::proof".to_string(),
                citation: "test:suite::proof".to_string(),
                selectable: true,
                advice: "accepted reference".to_string(),
            }],
        ))));
        let transition = model.update(Action::UseTestEvidence);
        assert!(transition.effect.is_none());
        assert_eq!(model.modal, Some(Modal::TestCatalogue));
        assert!(
            model
                .notice
                .as_ref()
                .is_some_and(|notice| { notice.text.contains("Select a loaded task") })
        );
    }

    /// Proof for `show.evidence_observations_are_nonblocking`.
    #[test]
    fn verification_progress_uses_a_privacy_safe_provider_ordinal_without_a_fake_percentage() {
        let mut model = Model::new(snapshot("one", 1));
        let request = model
            .update(Action::Verify)
            .effect
            .expect("verification request");
        model.update(Action::EffectStarted(request.id));
        let operation = aplan_app::OperationId::new(request.id.0);
        for (sequence, subject, lifecycle, measure) in [
            (
                0,
                aplan_app::ProgressSubject::Operation,
                aplan_app::ProgressLifecycle::Queued,
                aplan_app::ProgressMeasure::Indeterminate,
            ),
            (
                1,
                aplan_app::ProgressSubject::Operation,
                aplan_app::ProgressLifecycle::Started,
                aplan_app::ProgressMeasure::Indeterminate,
            ),
            (
                2,
                aplan_app::ProgressSubject::Phase {
                    phase: aplan_app::ProgressPhase::EvidenceProvider,
                    child: None,
                },
                aplan_app::ProgressLifecycle::Started,
                aplan_app::ProgressMeasure::bounded(1, 12, aplan_app::ProgressUnit::Providers),
            ),
            (
                3,
                aplan_app::ProgressSubject::Phase {
                    phase: aplan_app::ProgressPhase::EvidenceProvider,
                    child: Some(aplan_app::ProgressChildId::new(2)),
                },
                aplan_app::ProgressLifecycle::Started,
                aplan_app::ProgressMeasure::Indeterminate,
            ),
        ] {
            model.update(Action::EffectProgress {
                id: request.id,
                progress: aplan_app::ProgressEvent {
                    operation,
                    sequence,
                    kind: aplan_app::OperationKind::Verification,
                    subject,
                    lifecycle,
                    measure,
                },
            });
        }
        let status = model.status_line();
        assert!(
            status.contains("querying evidence providers 2: started"),
            "{status}"
        );
        assert!(
            !status.contains('%'),
            "progress must not invent a percentage"
        );
        model.update(Action::EffectProgress {
            id: request.id,
            progress: aplan_app::ProgressEvent {
                operation: aplan_app::OperationId::new(request.id.0.saturating_add(1)),
                sequence: 4,
                kind: aplan_app::OperationKind::Verification,
                subject: aplan_app::ProgressSubject::Operation,
                lifecycle: aplan_app::ProgressLifecycle::Completed,
                measure: aplan_app::ProgressMeasure::Indeterminate,
            },
        });
        assert_eq!(
            model.status_line(),
            status,
            "a late event from another operation must not overwrite visible state"
        );
    }

    fn finding(
        identity: &str,
        code: &str,
        severity: aplan_app::Severity,
        task: Option<&str>,
        file: Option<&str>,
    ) -> aplan_app::OverviewFinding {
        aplan_app::OverviewFinding {
            identity: identity.to_string(),
            scope: DiagnosticScope::Check,
            code: code.to_string(),
            severity,
            subject: task.map_or_else(|| "whole plan".to_string(), |task| format!("task {task}")),
            task: task.map(str::to_string),
            file: file.map(PathBuf::from),
            line: Some(3),
            column: Some(2),
            message: format!("message for {identity}"),
            agent_instruction: Some(format!("repair {identity}")),
            detail: vec![format!("detail for {identity}")],
            contribution: aplan_app::VerdictContribution::Blocking,
            source_line: Some(format!("source for {identity}")),
        }
    }

    fn with_check_findings(
        mut snapshot: Snapshot,
        findings: Vec<aplan_app::OverviewFinding>,
        denied: bool,
    ) -> Snapshot {
        snapshot.overview.findings = findings.clone();
        snapshot.overview.errors = findings
            .iter()
            .filter(|finding| finding.severity == aplan_app::Severity::Error)
            .count();
        snapshot.overview.warnings = findings.len().saturating_sub(snapshot.overview.errors);
        snapshot.overview.denied = denied;
        snapshot.diagnostics.check.findings = findings;
        snapshot.diagnostics.check.errors = snapshot.overview.errors;
        snapshot.diagnostics.check.warnings = snapshot.overview.warnings;
        snapshot.diagnostics.check.blocking = snapshot.diagnostics.check.findings.len();
        snapshot.diagnostics.check.denied = denied;
        snapshot
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_semantic_action_has_a_reducer_case() {
        let mut model = Model::new(snapshot("one", 2));
        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let verify = model.update(Action::Verify).effect.expect("verify");
        let actions = vec![
            Action::Quit,
            Action::Route(Route::Tasks),
            Action::Focus(Focus::Navigation),
            Action::OpenDetail,
            Action::CloseLayer,
            Action::OpenAbout,
            Action::OpenHelp,
            Action::OpenPalette,
            Action::OpenSearch,
            Action::OpenReportDate,
            Action::OpenWorkDate,
            Action::OpenTestCatalogue,
            Action::UseTestEvidence,
            Action::SubmitWorkDate,
            Action::OpenFilters,
            Action::OpenRunSheet,
            Action::OpenVerifyRunSheet,
            Action::OpenContextualRule,
            Action::OpenRuleSearch,
            Action::SubmitRuleSearch,
            Action::OpenFindingTask,
            Action::CycleDiagnosticScope,
            Action::PaletteInput("x".to_string()),
            Action::PaletteBackspace,
            Action::SearchInput("x".to_string()),
            Action::SearchBackspace,
            Action::SearchDelete,
            Action::SearchLeft,
            Action::SearchRight,
            Action::SearchHome,
            Action::SearchEnd,
            Action::SearchClear,
            Action::ClearFilters,
            Action::ClearNarrowing,
            Action::CycleTaskSort,
            Action::ToggleWhyTraversal,
            Action::PreviousInspectorSection,
            Action::NextInspectorSection,
            Action::SetInspectorScroll(5),
            Action::ToggleFilter(0),
            Action::ToggleRunSheet(0),
            Action::ToggleVerifyRunSheet(0),
            Action::SelectModal(1),
            Action::AdjustModalScroll(1),
            Action::CycleTheme,
            Action::ToggleGlyphs,
            Action::CycleGraphScope,
            Action::SetGraphBetween,
            Action::ToggleGraphMembership,
            Action::CycleGraphDensity,
            Action::CycleGraphOrdering,
            Action::CycleGraphOrientation,
            Action::ToggleGraphRelationsOnly,
            Action::GraphOverview,
            Action::FitGraph,
            Action::FollowGraphBlocker,
            Action::FollowGraphDependent,
            Action::PreviousGraphContinuation,
            Action::NextGraphContinuation,
            Action::OpenGraphContinuation,
            Action::ContractGraphDepth,
            Action::ExpandGraphDepth,
            Action::PreviousGraphWindow,
            Action::RecenterGraph,
            Action::SelectIndex(1),
            Action::SelectReportRow(0),
            Action::SelectTask("T-1".to_string()),
            Action::SelectGraphIdentity(GraphExplorerIdentity::Task("T-1".to_string())),
            Action::OpenGraphAt("T-1".to_string()),
            Action::OpenGraphLocation(GraphExplorerLocation::Plan),
            Action::MoveGraph(SpatialDirection::Right),
            Action::JumpGraph(GraphJump::NextLayer),
            Action::SetGraphViewport { row: 1, column: 1 },
            Action::Notify("notice".to_string()),
            Action::Invoke(Box::new(Action::Route(Route::Graph))),
            Action::SelectNext,
            Action::SelectPrevious,
            Action::ExternalProbeOverdue,
            Action::PollExternal,
            Action::Refresh,
            Action::Retry,
            Action::Verify,
            Action::CancelQueued,
            Action::OpenMutation(MutationAction::Add),
            Action::MutationForm(FormAction::Cancel),
            Action::MutationScroll(1),
            Action::ConfirmMutation,
            Action::OpenExternalEditor,
            Action::Resize {
                columns: 80,
                rows: 24,
            },
            Action::Paste("text".to_string()),
            Action::FocusGained,
            Action::FocusLost,
            Action::Tick,
            Action::EffectStarted(refresh.id),
            Action::EffectProgress {
                id: refresh.id,
                progress: aplan_app::ProgressEvent {
                    operation: aplan_app::OperationId::new(refresh.id.0),
                    sequence: 0,
                    kind: aplan_app::OperationKind::Verification,
                    subject: aplan_app::ProgressSubject::Operation,
                    lifecycle: aplan_app::ProgressLifecycle::Queued,
                    measure: aplan_app::ProgressMeasure::Indeterminate,
                },
            },
            completion(
                refresh.clone(),
                Ok(EffectOutput::snapshot(snapshot("one", 2))),
            ),
            Action::EffectRejected {
                id: verify.id,
                reason: "full".to_string(),
            },
        ];
        let seen: BTreeSet<_> = actions.iter().map(Action::kind).collect();
        let expected: BTreeSet<_> = ActionKind::ALL.iter().copied().collect();
        assert_eq!(seen, expected, "the table must drive every action kind");

        for action in actions {
            model.update(action);
        }
    }

    fn graph_tasks() -> Vec<aplan_core::task::Task> {
        let mut disconnected = graph_task("E", "disconnected", &[]);
        disconnected.owner = Some("kim".to_string());
        vec![
            graph_task("A", "root", &[]),
            graph_task("B", "left", &["A"]),
            graph_task("C", "right", &["A"]),
            graph_task("D", "merge", &["B", "C"]),
            disconnected,
        ]
    }

    fn graph_model() -> Model {
        let mut model = ModelBuilder::default().graph(graph_tasks()).build();
        model.update(Action::Route(Route::Graph));
        model
    }

    /// Proof for `tui.hidden_graph_resize_defers_reprojection`: responsive
    /// density still tracks terminal size outside Graph, but complete-source
    /// projection and cell layout wait until the destination is visible again.
    #[test]
    fn resize_defers_hidden_graph_reprojection_until_graph_reentry() {
        let mut model = graph_model();
        model.update(Action::Route(Route::Overview));
        let generation = model.graph_scene_generation;

        model.update(Action::Resize {
            columns: 60,
            rows: 16,
        });
        assert_eq!(model.graph_resolved_density, ResolvedGraphDensity::Compact);
        assert_eq!(
            model.graph_scene_generation, generation,
            "a hidden Graph destination was reprojected during Overview resize"
        );

        model.update(Action::Route(Route::Graph));
        assert!(model.graph_scene_generation > generation);
        assert_eq!(model.graph_resolved_density, ResolvedGraphDensity::Compact);
    }

    /// Proof for `tui.graph_selection_reuses_current_projection`: selecting a
    /// represented aggregate subject updates exactly the selection-dependent
    /// projection fields and reuses geometry instead of reprojecting source.
    #[test]
    fn represented_graph_selection_reuses_projection_and_geometry() {
        let mut model = hierarchical_graph_model();
        let before = model.graph_cell_scene().expect("initial scene").clone();
        let target = model
            .graph_explorer
            .graph
            .nodes
            .iter()
            .flat_map(|node| node.represented_ids.iter())
            .find(|id| model.selected_id.as_deref() != Some(id.as_str()))
            .expect("another represented subject")
            .clone();

        model.update(Action::SelectTask(target));

        let expected = model
            .snapshot
            .graph
            .project_explorer(&model.graph_explorer_request);
        assert_eq!(model.graph_explorer, expected);
        let after = model.graph_cell_scene().expect("refocused scene");
        assert_eq!(after.geometry_key, before.geometry_key);
        assert_eq!(after.nodes, before.nodes);
        assert_eq!(after.edges, before.edges);
        assert_eq!(
            after.movement,
            aplan_app::CellSceneMovement::ReusedStatusOnly
        );
    }

    fn hierarchical_graph_fixture() -> (Vec<aplan_core::task::Task>, Vec<aplan_app::TaskBrowserRow>)
    {
        let mut a = graph_task("A", "ready root", &[]);
        a.phase = 1;
        let mut b = graph_task("B", "phase one dependent", &["A"]);
        b.phase = 1;
        let mut c = graph_task("C", "phase two dependent", &["B"]);
        c.phase = 2;
        let mut d = graph_task("D", "second root dependent", &["A"]);
        d.phase = 2;
        let mut first = graph_task("M-1", "first milestone", &["A", "B"]);
        first.phase = 1;
        first.tags.push(aplan_core::task::MILESTONE_TAG.to_string());
        let mut second = graph_task("M-2", "second milestone", &["C", "D"]);
        second.phase = 2;
        second
            .tags
            .push(aplan_core::task::MILESTONE_TAG.to_string());
        let tasks = vec![a, b, c, d, first, second];
        let rows = tasks
            .iter()
            .map(|task| {
                let mut row = task_row(&task.id, &task.title);
                row.phase = task.phase;
                row.status = task.status;
                row.tags.clone_from(&task.tags);
                row.milestone = match task.id.as_str() {
                    "A" | "B" => Some("M-1".to_string()),
                    "C" | "D" => Some("M-2".to_string()),
                    _ => None,
                };
                if task.id == "A" {
                    row.ready_rank = Some(0);
                }
                row
            })
            .collect();
        (tasks, rows)
    }

    fn hierarchical_graph_model() -> Model {
        let (tasks, rows) = hierarchical_graph_fixture();
        let mut model = ModelBuilder::default()
            .graph_with_catalogue(tasks, rows)
            .build();
        model.update(Action::Route(Route::Graph));
        model
    }

    fn oversized_graph_tasks() -> Vec<aplan_core::task::Task> {
        let mut tasks = vec![graph_task("T-000", "root", &[])];
        tasks.extend((1..=60).map(|index| {
            graph_task(
                &format!("T-{index:03}"),
                &format!("dependent {index}"),
                &["T-000"],
            )
        }));
        tasks
    }

    fn open_later_graph_pages_until(model: &mut Model, target: &str) -> usize {
        let mut opened_pages = 1;
        while !model.graph_node_ids().iter().any(|id| id == target) && opened_pages < 16 {
            model.graph_continuation_selected = model
                .graph_explorer
                .boundaries
                .iter()
                .position(|candidate| candidate.travel == aplan_app::GraphContinuationTravel::Later)
                .expect("every non-terminal high-degree page has a later continuation");
            model.update(Action::OpenGraphContinuation);
            opened_pages += 1;
        }
        opened_pages
    }

    type NodeGeometry = (String, usize, usize);
    type EdgeGeometry = (String, String, Vec<(usize, usize)>);

    fn graph_geometry(model: &Model) -> (Vec<NodeGeometry>, Vec<EdgeGeometry>) {
        let scene = model.graph_scene.as_ref().expect("graph scene");
        let mut nodes = scene
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node.marker.row, node.marker.column))
            .collect::<Vec<_>>();
        nodes.sort();
        let mut edges = scene
            .edges
            .iter()
            .map(|edge| {
                (
                    edge.id.from.clone(),
                    edge.id.to.clone(),
                    edge.cells
                        .iter()
                        .map(|cell| (cell.0.row, cell.0.column))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        edges.sort();
        (nodes, edges)
    }

    /// A refresh whose only graph-visible change is status, and a refresh that
    /// changes diagnostics alone, retain the selected identity, manual
    /// viewport and structural placement. This is intentionally stronger than
    /// merely proving that the selected id still exists.
    #[test]
    fn status_and_diagnostic_refreshes_retain_graph_placement_and_selection() {
        let mut model = graph_model();
        model.update(Action::SelectTask("D".to_string()));
        model.update(Action::SetGraphViewport { row: 2, column: 3 });
        let initial_geometry = graph_geometry(&model);
        let initial_viewport = (model.graph_scroll_row, model.graph_scroll_column);

        let refresh = model
            .update(Action::Refresh)
            .effect
            .expect("status refresh");
        let mut tasks = graph_tasks();
        tasks
            .iter_mut()
            .find(|task| task.id == "B")
            .expect("B task")
            .status = aplan_core::Status::Done;
        let replacement = ModelBuilder::default().graph(tasks).build().snapshot;
        model.update(completion(refresh, Ok(EffectOutput::snapshot(replacement))));
        assert_eq!(model.selected_id.as_deref(), Some("D"));
        assert_eq!(model.selection_state, SelectionState::Present);
        assert_eq!(model.graph_viewport_mode, GraphViewportMode::Manual);
        assert_eq!(
            (model.graph_scroll_row, model.graph_scroll_column),
            initial_viewport
        );
        assert_eq!(graph_geometry(&model), initial_geometry);

        let status_geometry = graph_geometry(&model);
        let refresh = model
            .update(Action::Refresh)
            .effect
            .expect("diagnostic refresh");
        let mut diagnostics_only = model.snapshot.clone();
        diagnostics_only.diagnostics.check.findings.push(finding(
            "diagnostic-only",
            "APLAN999",
            aplan_app::Severity::Warning,
            Some("B"),
            Some("aplan/tasks/B.md"),
        ));
        model.update(completion(
            refresh,
            Ok(EffectOutput::snapshot(diagnostics_only)),
        ));
        assert_eq!(model.selected_id.as_deref(), Some("D"));
        assert_eq!(model.graph_viewport_mode, GraphViewportMode::Manual);
        assert_eq!(
            (model.graph_scroll_row, model.graph_scroll_column),
            initial_viewport
        );
        assert_eq!(graph_geometry(&model), status_geometry);
    }

    /// Proof for `tui.refresh_rebuilds_only_the_visible_projection`: a Graph
    /// refresh adopts shared source state immediately but leaves hidden Tasks
    /// materialization alone until entering that destination, where it must
    /// expose the new status rather than stale rows.
    #[test]
    fn refresh_defers_hidden_destination_projection_until_route_entry() {
        let mut model = graph_model();
        let hidden_before = model
            .task_browser
            .rows
            .iter()
            .find(|row| row.task.id == "A")
            .expect("hidden task row")
            .task
            .status;
        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let mut tasks = graph_tasks();
        tasks
            .iter_mut()
            .find(|task| task.id == "A")
            .expect("replacement task")
            .status = aplan_core::Status::Done;
        let replacement = ModelBuilder::default().graph(tasks).build().snapshot;

        model.update(completion(refresh, Ok(EffectOutput::snapshot(replacement))));

        assert_eq!(
            model
                .task_browser
                .rows
                .iter()
                .find(|row| row.task.id == "A")
                .expect("deferred hidden row")
                .task
                .status,
            hidden_before
        );
        model.update(Action::Route(Route::Tasks));
        assert_eq!(
            model
                .task_browser
                .rows
                .iter()
                .find(|row| row.task.id == "A")
                .expect("reconciled visible row")
                .task
                .status,
            aplan_core::Status::Done
        );
    }

    /// Proof for `tui.graph_explorer_uses_semantic_scene`: graph search and
    /// shared filters preserve scope, while every simultaneous semantic role
    /// remains visible without color.
    #[test]
    fn graph_search_filters_and_combined_roles_are_semantic() {
        let mut model = graph_model();

        assert_eq!(model.graph_node_ids(), ["A", "E", "B", "C", "D"]);
        assert_eq!(model.graph_role("A"), GraphRole::SelectedCritical);
        assert!(matches!(
            model.graph_role("B"),
            GraphRole::Dependent | GraphRole::DependentCritical
        ));
        assert!(matches!(
            model.graph_role("C"),
            GraphRole::Dependent | GraphRole::DependentCritical
        ));
        assert_ne!(model.graph_role("B"), model.graph_role("C"));
        assert_eq!(model.graph_role("E"), GraphRole::Unrelated);

        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("disconnected".to_string()));
        assert_eq!(model.selected_id.as_deref(), Some("E"));
        assert_eq!(model.graph_request.scope, GraphScope::Whole);
        model.update(Action::SearchClear);
        model.update(Action::CloseLayer);
        model.update(Action::OpenFilters);
        let owner = model
            .filter_choices()
            .iter()
            .position(|choice| matches!(choice, TaskFilter::Owner(value) if value == "kim"))
            .expect("owner filter");
        model.update(Action::ToggleFilter(owner));
        assert_eq!(model.graph_node_ids(), ["E"]);
        model.update(Action::CloseLayer);
        model.update(Action::ClearNarrowing);
        assert_eq!(model.graph_node_ids(), ["A", "E", "B", "C", "D"]);
    }

    /// Proof for `tui.layered_graph_navigation_is_semantic_and_reversible`:
    /// every peer-order policy reaches the coordinate kernel, reports its
    /// layout cost and returns exactly to crossing-minimized readability.
    #[test]
    fn graph_peer_order_modes_are_complete_visible_and_reversible() {
        let mut model = graph_model();
        for expected in [
            GraphOrdering::PlanPriority,
            GraphOrdering::Identity,
            GraphOrdering::Title,
            GraphOrdering::StatusReadiness,
            GraphOrdering::Criticality,
            GraphOrdering::Readability,
        ] {
            model.update(Action::CycleGraphOrdering);
            assert_eq!(model.graph_ordering, expected);
            assert_eq!(
                model
                    .graph_cell_scene()
                    .expect("ordered scene")
                    .ordering_policy,
                expected.cell_order()
            );
            assert!(model.status_line().contains(expected.label()));
            assert!(model.graph_movement_label().contains("order"));
        }
    }

    /// Proof for `tui.layered_graph_navigation_is_semantic_and_reversible`:
    /// arrows consume actual cell geometry, repeated relation commands retain
    /// their semantic subject, and hierarchy history restores level,
    /// selection and viewport rather than replaying formatted scene rows.
    #[test]
    fn layered_graph_navigation_is_semantic_and_reversible() {
        let mut spatial = graph_model();
        spatial.update(Action::SelectGraphIdentity(GraphExplorerIdentity::Task(
            "A".to_string(),
        )));
        let source_order = spatial.graph_node_ids();
        let source_next = source_order
            .iter()
            .position(|id| id == "A")
            .and_then(|index| source_order.get(index + 1))
            .cloned()
            .expect("scene-order successor");
        let expected = graph_navigation::spatial_target(
            spatial.graph_cell_scene().expect("cell scene"),
            &GraphExplorerIdentity::Task("A".to_string()),
            SpatialDirection::Right,
        )
        .expect("visible dependency successor");
        assert_ne!(expected.id(), source_next);
        spatial.update(Action::MoveGraph(SpatialDirection::Right));
        assert_eq!(spatial.selected_id.as_deref(), Some(expected.id()));

        spatial.update(Action::SelectGraphIdentity(GraphExplorerIdentity::Task(
            "A".to_string(),
        )));
        spatial.update(Action::FollowGraphDependent);
        assert_eq!(spatial.selected_id.as_deref(), Some("B"));
        assert_eq!(spatial.graph_relation_progress(), Some((false, 1, 2)));
        spatial.update(Action::FollowGraphDependent);
        assert_eq!(spatial.selected_id.as_deref(), Some("C"));
        assert_eq!(spatial.graph_relation_progress(), Some((false, 2, 2)));
        spatial.update(Action::FollowGraphDependent);
        assert_eq!(spatial.selected_id.as_deref(), Some("B"));
        assert_eq!(spatial.graph_relation_progress(), Some((false, 1, 2)));

        let mut hierarchy = hierarchical_graph_model();
        assert_eq!(
            hierarchy.graph_explorer.location,
            GraphExplorerLocation::Plan
        );
        let anchor = hierarchy.graph_explorer.selection.as_ref().expect("anchor");
        assert_eq!(anchor.subject.id(), "A");
        assert_eq!(anchor.represented_by.id(), "M-1");
        assert_eq!(anchor.reason, aplan_app::GraphExplorerAnchorReason::Ready);

        hierarchy.update(Action::OpenDetail);
        assert_eq!(
            hierarchy.graph_explorer.location,
            GraphExplorerLocation::Phase(1)
        );
        hierarchy.update(Action::OpenDetail);
        assert_eq!(
            hierarchy.graph_explorer.location,
            GraphExplorerLocation::Milestone("M-1".to_string())
        );
        hierarchy.update(Action::SetGraphViewport { row: 2, column: 3 });
        hierarchy.update(Action::OpenDetail);
        assert_eq!(
            hierarchy.graph_explorer.location,
            GraphExplorerLocation::Task("A".to_string())
        );
        hierarchy.update(Action::CycleGraphOrdering);
        hierarchy.update(Action::CycleGraphOrientation);
        hierarchy.update(Action::PreviousGraphWindow);
        assert_eq!(
            hierarchy.graph_explorer.location,
            GraphExplorerLocation::Milestone("M-1".to_string())
        );
        assert_eq!(
            (hierarchy.graph_scroll_row, hierarchy.graph_scroll_column),
            (2, 3)
        );
        assert_eq!(hierarchy.graph_viewport_mode, GraphViewportMode::Manual);
        assert_eq!(hierarchy.graph_ordering, GraphOrdering::PlanPriority);
        assert_eq!(
            hierarchy.graph_orientation,
            CellSceneOrientation::LeftToRight
        );
        hierarchy.update(Action::JumpGraph(GraphJump::NextPhase));
        assert_eq!(
            hierarchy.graph_explorer.location,
            GraphExplorerLocation::Phase(2)
        );
        hierarchy.update(Action::Resize {
            columns: 80,
            rows: 18,
        });
        assert_eq!(hierarchy.graph_ordering, GraphOrdering::PlanPriority);
        assert_eq!(
            hierarchy.graph_orientation,
            CellSceneOrientation::LeftToRight
        );
    }

    /// Proof for `tui.layered_graph_navigation_is_semantic_and_reversible`:
    /// an unchanged refresh preserves every explicit semantic and presentation
    /// policy, the exact selected level, bounded history and manual viewport.
    #[test]
    fn graph_refresh_preserves_semantic_session_and_explicit_pins() {
        let mut model = hierarchical_graph_model();
        model.update(Action::OpenDetail);
        model.update(Action::JumpGraph(GraphJump::NextPhase));
        model.update(Action::CycleGraphOrdering);
        model.update(Action::CycleGraphOrientation);
        model.update(Action::CycleGraphDensity);
        model.update(Action::ToggleGraphRelationsOnly);
        model.update(Action::SetGraphViewport { row: 2, column: 3 });
        let preserved = (
            model.graph_explorer_request.location.clone(),
            model.selected_id.clone(),
            model.graph_ordering,
            model.graph_orientation,
            model.graph_density,
            model.graph_resolved_density,
            model.graph_edge_presentation,
            model.graph_request.clone(),
            model.graph_explorer_history.len(),
            model.graph_viewport_mode,
            model.graph_scroll_row,
            model.graph_scroll_column,
        );
        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let unchanged = model.snapshot.clone();
        model.update(completion(refresh, Ok(EffectOutput::snapshot(unchanged))));
        assert_eq!(
            (
                model.graph_explorer_request.location.clone(),
                model.selected_id.clone(),
                model.graph_ordering,
                model.graph_orientation,
                model.graph_density,
                model.graph_resolved_density,
                model.graph_edge_presentation,
                model.graph_request.clone(),
                model.graph_explorer_history.len(),
                model.graph_viewport_mode,
                model.graph_scroll_row,
                model.graph_scroll_column,
            ),
            preserved
        );
    }

    /// Proof for `tui.layered_graph_navigation_is_semantic_and_reversible`:
    /// contextual entry can temporarily widen a saved scope, returns to the
    /// exact originating selection, and leaves a later direct Graph visit at
    /// the prior session level.
    #[test]
    fn contextual_graph_entry_restores_the_saved_session_and_scope() {
        let mut model = hierarchical_graph_model();
        model.update(Action::OpenDetail);
        assert_eq!(
            model.graph_explorer.location,
            GraphExplorerLocation::Phase(1)
        );
        model.graph_request.scope = GraphScope::Upstream("A".to_string());
        model.rebuild_graph();
        let saved_request = model.graph_request.clone();
        let saved_location = model.graph_explorer_request.location.clone();

        model.update(Action::Route(Route::Tasks));
        let task_index = model
            .snapshot
            .task_rows
            .iter()
            .position(|row| row.id == "D")
            .expect("D row");
        model.update(Action::SelectIndex(task_index));
        model.update(Action::OpenGraphAt("D".to_string()));
        assert_eq!(model.route, Route::Graph);
        assert_eq!(model.selected_id.as_deref(), Some("D"));
        assert_eq!(model.graph_request.scope, GraphScope::Whole);
        assert!(model.graph_breadcrumb().starts_with("Tasks > Overview"));

        model.update(Action::PreviousGraphWindow);
        assert_eq!(model.route, Route::Tasks);
        assert_eq!(model.selected_id.as_deref(), Some("D"));
        assert_eq!(model.graph_request, saved_request);
        assert_eq!(model.graph_explorer_request.location, saved_location);
        model.update(Action::Route(Route::Graph));
        assert_eq!(model.graph_request, saved_request);
        assert_eq!(model.graph_explorer_request.location, saved_location);
    }

    /// Proof for `tui.layered_graph_navigation_is_semantic_and_reversible`:
    /// every semantic jump uses typed graph, phase or finding provenance and
    /// none of them widens or replaces the user's structural scope.
    #[test]
    fn graph_jump_commands_preserve_scope_and_use_semantic_candidates() {
        let mut model = hierarchical_graph_model();
        let scope = model.graph_request.clone();

        model.update(Action::JumpGraph(GraphJump::Sink));
        assert_eq!(model.selected_id.as_deref(), Some("M-2"));
        model.update(Action::JumpGraph(GraphJump::Source));
        assert_eq!(model.selected_id.as_deref(), Some("M-1"));
        model.update(Action::JumpGraph(GraphJump::NextLayer));
        assert_eq!(model.selected_id.as_deref(), Some("M-2"));
        let scene = model.graph_cell_scene().expect("layered scene");
        assert_eq!(
            model
                .graph_selected_identity()
                .map(GraphExplorerIdentity::id),
            Some("M-2"),
            "selection={:?}, nodes={:?}",
            model.graph_explorer.selection,
            scene
                .nodes
                .iter()
                .map(|node| (&node.identity, node.rank, node.order))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            model.graph_jump_candidates(GraphJump::PreviousLayer, model.graph_cell_scene()),
            [GraphExplorerIdentity::Milestone("M-1".to_string())]
        );
        model.update(Action::JumpGraph(GraphJump::PreviousLayer));
        assert_eq!(model.selected_id.as_deref(), Some("M-1"));
        model.update(Action::JumpGraph(GraphJump::Milestone));
        assert_eq!(model.selected_id.as_deref(), Some("M-2"));
        model.update(Action::JumpGraph(GraphJump::Critical));
        assert!(matches!(model.selected_id.as_deref(), Some("M-1" | "M-2")));

        model.snapshot.diagnostics.check.findings.push(finding(
            "finding-on-c",
            "APLAN999",
            aplan_app::Severity::Warning,
            Some("C"),
            Some("aplan/tasks/C.md"),
        ));
        model.update(Action::JumpGraph(GraphJump::Finding));
        assert_eq!(model.selected_id.as_deref(), Some("M-2"));
        assert_eq!(model.graph_request, scope);

        model.update(Action::OpenGraphLocation(GraphExplorerLocation::Phase(1)));
        model.update(Action::JumpGraph(GraphJump::NextPhase));
        assert_eq!(
            model.graph_explorer.location,
            GraphExplorerLocation::Phase(2)
        );
        model.update(Action::JumpGraph(GraphJump::PreviousPhase));
        assert_eq!(
            model.graph_explorer.location,
            GraphExplorerLocation::Phase(1)
        );
        assert_eq!(model.graph_request, scope);
    }

    /// Proof for `tui.graph_explorer_uses_semantic_scene`: scope changes,
    /// relationship following, selection, hit identities and viewport state
    /// operate on the application projection and shared semantic scene.
    #[test]
    fn graph_explorer_uses_semantic_scene_end_to_end() {
        let mut model = graph_model();
        model.update(Action::SelectTask("A".to_string()));
        let marker = model
            .graph_scene
            .as_ref()
            .expect("scene")
            .nodes
            .iter()
            .find(|node| node.id == "C")
            .expect("C node")
            .marker;
        assert_eq!(model.graph_hit(marker), Some("C".to_string()));

        model.update(Action::FollowGraphDependent);
        assert_eq!(model.selected_id.as_deref(), Some("B"));
        assert_eq!(
            model.selected_inspector().map(|show| show.id.as_str()),
            Some("B")
        );
        model.update(Action::FollowGraphBlocker);
        assert_eq!(model.selected_id.as_deref(), Some("A"));

        model.update(Action::CycleGraphScope);
        assert_eq!(
            model.graph_request.scope,
            GraphScope::Upstream("A".to_string())
        );
        assert_eq!(model.graph_node_ids(), ["A"]);
        assert_eq!(
            model.graph_projection.state,
            aplan_app::GraphProjectionState::NoEdges
        );
        model.update(Action::CycleGraphScope);
        assert_eq!(
            model.graph_request.scope,
            GraphScope::Downstream("A".to_string())
        );
        assert_eq!(model.graph_node_ids(), ["A", "B", "C", "D"]);

        model.update(Action::SetGraphBetween);
        assert_eq!(model.graph_between_anchor.as_deref(), Some("A"));
        model.update(Action::SelectTask("D".to_string()));
        model.update(Action::SetGraphBetween);
        assert_eq!(
            model.graph_request.scope,
            GraphScope::Between("A".to_string(), "D".to_string())
        );
        assert_eq!(model.graph_node_ids(), ["A", "B", "C", "D"]);

        model.update(Action::SetGraphViewport { row: 7, column: 5 });
        let bounds = model.graph_cell_scene().expect("cell scene").bounds;
        assert_eq!(
            (model.graph_scroll_row, model.graph_scroll_column),
            (
                7.min(usize::from(bounds.height).saturating_sub(1)),
                5.min(usize::from(bounds.width).saturating_sub(1))
            )
        );
        assert_eq!(model.graph_viewport_mode, GraphViewportMode::Manual);
        model.update(Action::RecenterGraph);
        assert_eq!(
            model.graph_viewport_mode,
            GraphViewportMode::FollowSelection
        );

        model.graph_request.scope = GraphScope::CriticalPath;
        model.rebuild_graph();
        model.selected_id = Some("E".to_string());
        assert!(
            model
                .graph_selection_exclusion()
                .is_some_and(|reason| reason.contains("critical path"))
        );
        model.update(Action::ClearNarrowing);
        assert_eq!(model.graph_request, GraphRequest::default());
        assert!(model.graph_projection.contains("E"));
    }

    #[test]
    fn graph_scope_cycle_roles_and_empty_states_have_explicit_answers() {
        let mut model = graph_model();
        model.update(Action::SelectTask("A".to_string()));

        let expected = [
            GraphScope::Upstream("A".to_string()),
            GraphScope::Downstream("A".to_string()),
            GraphScope::Unreachable,
            GraphScope::Neighbourhood {
                task: "A".to_string(),
                depth: Some(1),
                live: false,
            },
            GraphScope::Neighbourhood {
                task: "A".to_string(),
                depth: Some(2),
                live: false,
            },
            GraphScope::Neighbourhood {
                task: "A".to_string(),
                depth: None,
                live: false,
            },
            GraphScope::Neighbourhood {
                task: "A".to_string(),
                depth: None,
                live: true,
            },
            GraphScope::CriticalPath,
            GraphScope::Whole,
        ];
        for scope in expected {
            model.update(Action::CycleGraphScope);
            assert_eq!(model.graph_request.scope, scope);
        }

        model.update(Action::ToggleGraphMembership);
        assert!(model.graph_request.membership);
        assert_eq!(
            model.graph_viewport_mode,
            GraphViewportMode::FollowSelection
        );

        model.update(Action::SelectTask("D".to_string()));
        let blocker_roles = [model.graph_role("B"), model.graph_role("C")];
        assert!(blocker_roles.contains(&GraphRole::BlockerCritical));
        assert!(blocker_roles.contains(&GraphRole::Blocker));
        model.update(Action::SelectTask("E".to_string()));
        assert_eq!(model.graph_role("E"), GraphRole::Selected);
        model.update(Action::FollowGraphBlocker);
        assert!(model.status_line().contains("no direct blocker"));
        model.update(Action::FollowGraphDependent);
        assert!(model.status_line().contains("no direct dependent"));

        let cycle = vec![
            graph_task("X", "cycle one", &["Y"]),
            graph_task("Y", "cycle two", &["X"]),
        ];
        let mut cyclic = ModelBuilder::default().graph(cycle).build();
        cyclic.update(Action::Route(Route::Graph));
        cyclic.update(Action::SelectTask("X".to_string()));
        assert_eq!(cyclic.graph_role("Y"), GraphRole::BlockerDependent);

        assert_eq!(GraphRole::Selected.badge(), "[S] ");
        assert_eq!(GraphRole::Blocker.badge(), "[B] ");
        assert_eq!(GraphRole::BlockerCritical.badge(), "[BC]");
        assert_eq!(GraphRole::BlockerDependent.badge(), "[BR]");

        model.selected_id = None;
        model.update(Action::CycleGraphScope);
        assert!(model.status_line().contains("Select a graph task"));
        model.update(Action::SetGraphBetween);
        assert!(model.status_line().contains("Select a graph task"));
        model.update(Action::FollowGraphBlocker);
        assert!(model.status_line().contains("No graph task"));
        model.update(Action::SelectTask("missing".to_string()));
        assert!(model.selected_id.is_none());

        model.graph_scene = Err(TooWide {
            lanes: 3,
            needed: 6,
            width: 5,
        });
        assert_eq!(model.graph_node_ids(), ["A", "E", "B", "C", "D"]);
    }

    /// Proof for `tui.graph_explorer_windows_oversized_inputs`: a graph wider
    /// than the complete scene limit opens as an accounted semantic window,
    /// reaches omitted identities and reverses the transition without ever
    /// replaying a cursor after the source identity changes.
    #[test]
    fn oversized_graph_windows_are_traversable_reversible_and_source_bound() {
        let tasks = oversized_graph_tasks();
        let mut model = ModelBuilder::default().graph(tasks.clone()).build();
        model.update(Action::Route(Route::Graph));
        model.graph_explorer_request.budget = aplan_app::GraphWindowBudget {
            nodes: 8,
            edges: 8,
            lanes: 4,
        };
        model.update(Action::OpenGraphLocation(GraphExplorerLocation::Task(
            "T-000".to_string(),
        )));

        assert_eq!(model.selected_id.as_deref(), Some("T-000"));
        assert!(
            !model
                .graph_explorer
                .accounting
                .window_omitted_task_ids
                .is_empty()
        );
        assert!(!model.graph_explorer.boundaries.is_empty());
        let initial_ids = model.graph_node_ids();
        let omitted = tasks
            .iter()
            .map(|task| task.id.clone())
            .find(|id| !initial_ids.contains(id))
            .expect("wide fixture has an omitted identity");

        model.update(Action::SetGraphViewport { row: 2, column: 3 });
        let initial_viewport = (model.graph_scroll_row, model.graph_scroll_column);
        model.update(Action::SelectTask(omitted.clone()));
        assert_eq!(model.selected_id.as_deref(), Some(omitted.as_str()));
        assert!(model.graph_node_ids().contains(&omitted));
        assert!(model.has_graph_window_history());
        model.update(Action::PreviousGraphWindow);
        assert_eq!(model.graph_node_ids(), initial_ids);
        assert_eq!(model.selected_id.as_deref(), Some("T-000"));
        assert_eq!(model.graph_relation_count(false), 60);
        assert_eq!(model.graph_viewport_mode, GraphViewportMode::Manual);
        assert_eq!(
            (model.graph_scroll_row, model.graph_scroll_column),
            initial_viewport
        );

        let continuation = model
            .selected_graph_continuation()
            .expect("initial continuation")
            .cursor
            .clone();
        model.update(Action::OpenGraphContinuation);
        assert!(model.graph_explorer_request.continuation.is_some());
        let first_page = model.graph_node_ids();
        assert_ne!(first_page, initial_ids);
        let opened_pages = open_later_graph_pages_until(&mut model, "T-060");
        assert!(opened_pages > 1);
        assert!(model.graph_node_ids().contains(&"T-060".to_string()));
        for _ in 1..opened_pages {
            model.update(Action::PreviousGraphWindow);
        }
        assert_eq!(model.graph_node_ids(), first_page);
        model.update(Action::PreviousGraphWindow);
        assert_eq!(model.graph_node_ids(), initial_ids);
        assert_eq!(model.selected_id.as_deref(), Some("T-000"));
        assert_eq!(model.graph_relation_count(false), 60);

        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let replacement = ModelBuilder::default()
            .input("replacement-input")
            .graph(tasks)
            .build()
            .snapshot;
        model.update(completion(refresh, Ok(EffectOutput::snapshot(replacement))));
        assert!(!model.has_graph_window_history());
        assert!(model.graph_explorer_request.continuation.is_none());
        assert_eq!(model.selected_id.as_deref(), Some("T-000"));
        assert_ne!(model.graph_explorer.input.sha256, continuation.input_sha256);

        let stale = GraphWindowRequest {
            preferred_anchor: Some("T-000".to_string()),
            budget: model.graph_window_request.budget,
            continuation: Some(continuation),
        };
        assert!(matches!(
            model.graph_projection.window(&stale).state,
            GraphWindowState::Failed(aplan_app::GraphWindowFailure::InvalidContinuation(_))
        ));

        model.update(Action::SelectTask("T-060".to_string()));
        let refresh = model
            .update(Action::Refresh)
            .effect
            .expect("removal refresh");
        let mut without_selected = oversized_graph_tasks();
        without_selected.retain(|task| task.id != "T-060");
        let replacement = ModelBuilder::default()
            .input("removed-selected-input")
            .graph(without_selected)
            .build()
            .snapshot;
        model.update(completion(refresh, Ok(EffectOutput::snapshot(replacement))));
        assert_eq!(model.selected_id.as_deref(), Some("T-060"));
        assert_eq!(model.selection_state, SelectionState::Removed);
        assert!(model.graph_window.anchor.as_ref().is_some_and(|anchor| {
            anchor.selected != "T-060" && anchor.replacement_reason.is_some()
        }));
        assert!(model.status_line().contains("graph anchor is now"));
    }

    /// Proof for `tui.layered_graph_navigation_is_semantic_and_reversible`:
    /// direct-relation cycling reads the complete scoped projection and opens
    /// omitted identities through bounded task detail instead of treating the
    /// painted window as the graph.
    #[test]
    fn bounded_relation_cycle_reaches_every_omitted_direct_neighbor() {
        let mut journey = ModelBuilder::default()
            .graph(oversized_graph_tasks())
            .build();
        journey.update(Action::Route(Route::Graph));
        journey.update(Action::OpenDetail);
        journey.update(Action::OpenDetail);
        assert!(matches!(
            journey.graph_explorer_request.location,
            GraphExplorerLocation::Task(_)
        ));
        assert_eq!(journey.graph_relation_count(false), 60);
        journey.update(Action::OpenGraphContinuation);
        journey.update(Action::PreviousGraphWindow);
        assert_eq!(journey.graph_relation_count(false), 60);

        let mut model = ModelBuilder::default()
            .graph(oversized_graph_tasks())
            .build();
        model.update(Action::Route(Route::Graph));
        model.graph_explorer_request.budget = aplan_app::GraphWindowBudget {
            nodes: 8,
            edges: 8,
            lanes: 4,
        };
        model.update(Action::OpenGraphLocation(GraphExplorerLocation::Task(
            "T-000".to_string(),
        )));
        assert_eq!(model.graph_relation_count(false), 60);
        let initial = model.graph_node_ids().into_iter().collect::<BTreeSet<_>>();
        let mut visited = BTreeSet::new();
        for index in 1..=60 {
            model.update(Action::FollowGraphDependent);
            let selected = model.selected_id.clone().expect("selected dependent");
            visited.insert(selected);
            assert_eq!(model.graph_relation_progress(), Some((false, index, 60)));
        }
        assert_eq!(visited.len(), 60);
        assert!(visited.iter().any(|id| !initial.contains(id)));
        assert_eq!(model.selected_id.as_deref(), Some("T-060"));
        assert!(model.graph_node_ids().contains(&"T-060".to_string()));
    }

    /// Proof for `tui.graph_explorer_windows_oversized_inputs`: graph window
    /// controls compose with structural scope, while membership remains a
    /// distinct non-colour role.
    #[test]
    fn graph_window_controls_preserve_scope_and_expose_membership_without_colour() {
        let mut model = graph_model();
        model.update(Action::SelectTask("A".to_string()));
        model.update(Action::ExpandGraphDepth);
        assert_eq!(
            model.graph_request.scope,
            GraphScope::Neighbourhood {
                task: "A".to_string(),
                depth: Some(1),
                live: false,
            }
        );
        model.update(Action::ExpandGraphDepth);
        assert!(matches!(
            model.graph_request.scope,
            GraphScope::Neighbourhood { depth: Some(2), .. }
        ));
        model.update(Action::ContractGraphDepth);
        assert!(matches!(
            model.graph_request.scope,
            GraphScope::Neighbourhood { depth: Some(1), .. }
        ));

        let member = graph_task("A", "member", &[]);
        let mut cut = graph_task("M", "cut", &["A"]);
        cut.tags.push(aplan_core::task::MILESTONE_TAG.to_string());
        let mut membership = ModelBuilder::default().graph(vec![member, cut]).build();
        membership.update(Action::Route(Route::Graph));
        membership.update(Action::ToggleGraphMembership);
        membership.update(Action::SelectTask("A".to_string()));
        assert_eq!(membership.graph_role("M"), GraphRole::Member);
        assert_eq!(GraphRole::Member.badge(), "[M] ");
    }

    /// Proof for `tui.diagnostic_workbench_retains_complete_runs`: every
    /// narrowing dimension is intersected across dimensions and unioned within one,
    /// while survivors keep the validator's canonical order and grouping does
    /// not mutate it.
    #[test]
    fn diagnostic_workbench_filters_without_reordering_or_recomputing_verdict() {
        let findings = vec![
            finding(
                "one",
                "APLAN009",
                aplan_app::Severity::Warning,
                Some("T-0"),
                Some("a.md"),
            ),
            finding(
                "two",
                "APLAN004",
                aplan_app::Severity::Error,
                Some("T-1"),
                Some("b.md"),
            ),
            finding(
                "three",
                "APLAN009",
                aplan_app::Severity::Warning,
                Some("T-1"),
                Some("a.md"),
            ),
        ];
        let mut model = Model::new(with_check_findings(snapshot("one", 2), findings, true));
        model.update(Action::Route(Route::Diagnostics));
        let canonical = |model: &Model| {
            model
                .diagnostic_findings()
                .into_iter()
                .map(|finding| finding.identity.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(canonical(&model), ["one", "two", "three"]);
        model.update(Action::CycleTaskSort);
        assert_eq!(model.diagnostic_group, DiagnosticGroup::Severity);
        assert_eq!(canonical(&model), ["one", "two", "three"]);

        model.diagnostic_filters = vec![
            DiagnosticFilter::Severity(aplan_app::Severity::Warning),
            DiagnosticFilter::Task("T-1".to_string()),
        ];
        model.rebuild_diagnostics();
        assert_eq!(canonical(&model), ["three"]);
        model.diagnostic_query = "repair three".to_string();
        model.diagnostic_query_cursor = model.diagnostic_query.len();
        model.rebuild_diagnostics();
        assert_eq!(canonical(&model), ["three"]);
        assert!(
            model.diagnostic_run().denied,
            "filtering cannot green the run"
        );
        assert_eq!(model.diagnostic_run().findings.len(), 3);

        model.diagnostic_query = "does not occur".to_string();
        model.diagnostic_query_cursor = model.diagnostic_query.len();
        model.rebuild_diagnostics();
        assert!(canonical(&model).is_empty());
        assert!(
            model.diagnostic_run().denied,
            "an empty view is not a clean run"
        );
    }

    fn diagnostic_command_model() -> Model {
        let findings = vec![
            finding(
                "warning",
                "APLAN009",
                aplan_app::Severity::Warning,
                Some("T-1"),
                Some("a.md"),
            ),
            finding(
                "plan",
                "APLAN004",
                aplan_app::Severity::Error,
                None,
                Some("b.md"),
            ),
            finding(
                "missing-task",
                "APLAN009",
                aplan_app::Severity::Warning,
                Some("T-9"),
                Some("c.md"),
            ),
        ];
        let mut snapshot = with_check_findings(snapshot("one", 2), findings, true);
        let mut document = finding(
            "document",
            "DOC-REFERENCE",
            aplan_app::Severity::Error,
            None,
            Some("PLAN.md"),
        );
        document.scope = DiagnosticScope::CheckDocs;
        document.subject = "reference T-99".to_string();
        snapshot.diagnostics.documents.findings = vec![document];
        snapshot.diagnostics.documents.errors = 1;
        snapshot.diagnostics.documents.blocking = 1;
        snapshot.diagnostics.documents.denied = true;
        snapshot.diagnostics.documents.coverage =
            aplan_app::DiagnosticCoverage::Complete { subjects: 1 };

        let mut model = Model::new(snapshot);
        model.update(Action::Route(Route::Diagnostics));
        model
    }

    #[test]
    fn diagnostic_commands_drive_every_narrowing_lookup_and_navigation_state() {
        let mut model = diagnostic_command_model();

        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("T-1".to_string()));
        assert_eq!(model.diagnostic_findings().len(), 1);
        model.update(Action::SearchHome);
        model.update(Action::SearchRight);
        model.update(Action::SearchDelete);
        model.update(Action::SearchBackspace);
        model.update(Action::SearchEnd);
        model.update(Action::SearchClear);
        assert_eq!(model.diagnostic_findings().len(), 3);
        model.update(Action::CloseLayer);

        model.update(Action::OpenFilters);
        let choices = model.diagnostic_filter_choices();
        assert!(choices.iter().any(|choice| matches!(
            choice,
            DiagnosticFilter::Severity(aplan_app::Severity::Error)
        )));
        assert!(
            choices
                .iter()
                .any(|choice| matches!(choice, DiagnosticFilter::Rule(code) if code == "APLAN009"))
        );
        assert!(
            choices
                .iter()
                .any(|choice| matches!(choice, DiagnosticFilter::Task(task) if task == "T-1"))
        );
        assert!(choices.iter().any(
            |choice| matches!(choice, DiagnosticFilter::File(path) if path == &PathBuf::from("a.md"))
        ));
        for index in 0..choices.len() {
            model.update(Action::ToggleFilter(index));
        }
        assert!(model.has_diagnostic_narrowing());
        for index in (0..choices.len()).rev() {
            model.update(Action::ToggleFilter(index));
        }
        assert!(model.diagnostic_filters.is_empty());
        model.update(Action::CloseLayer);

        for expected in [
            DiagnosticGroup::Severity,
            DiagnosticGroup::Rule,
            DiagnosticGroup::Task,
            DiagnosticGroup::File,
            DiagnosticGroup::Canonical,
        ] {
            model.update(Action::CycleTaskSort);
            assert_eq!(model.diagnostic_group, expected);
        }

        model.update(Action::OpenRuleSearch);
        model.update(Action::SearchInput("APLAN009".to_string()));
        model.update(Action::SearchHome);
        model.update(Action::SearchRight);
        model.update(Action::SearchDelete);
        model.update(Action::SearchInput("P".to_string()));
        model.update(Action::SearchEnd);
        model.update(Action::SubmitRuleSearch);
        assert_eq!(model.modal, Some(Modal::RuleExplanation));
        assert_eq!(
            model
                .rule_explanation
                .as_ref()
                .map(|rule| rule.code.as_str()),
            Some("APLAN009")
        );
        model.update(Action::CloseLayer);

        model.update(Action::SelectIndex(1));
        model.update(Action::OpenFindingTask);
        assert!(model.status_line().contains("no task subject"));
        model.update(Action::SelectIndex(2));
        model.update(Action::OpenFindingTask);
        assert!(model.status_line().contains("T-9 is missing or unloadable"));

        model.update(Action::CycleDiagnosticScope);
        assert_eq!(model.diagnostic_scope, DiagnosticScope::CheckDocs);
        assert!(model.diagnostic_query.is_empty() && model.diagnostic_filters.is_empty());
        model.update(Action::OpenContextualRule);
        assert!(model.status_line().contains("unknown rule `DOC-REFERENCE`"));

        model.diagnostic_query = "nothing matches".to_string();
        model.diagnostic_query_cursor = model.diagnostic_query.len();
        model.rebuild_diagnostics();
        model.update(Action::OpenContextualRule);
        assert!(model.status_line().contains("No finding is selected"));
        model.update(Action::OpenFindingTask);
        assert!(model.status_line().contains("No finding is selected"));
        model.update(Action::ClearNarrowing);
        model.update(Action::CycleDiagnosticScope);
        assert_eq!(model.diagnostic_scope, DiagnosticScope::Check);
    }

    /// Proof for `tui.check_overrides_are_one_run`: an advanced request owns
    /// its overrides, clears the draft immediately, and an ordinary refresh
    /// returns to an empty override set.
    #[test]
    fn advanced_check_overrides_are_explicit_and_one_run() {
        let mut model = Model::new(snapshot("one", 1));
        model.update(Action::Route(Route::Diagnostics));
        model.update(Action::OpenRunSheet);
        model.update(Action::ToggleRunSheet(0));
        model.update(Action::ToggleRunSheet(1));
        model.update(Action::ToggleRunSheet(2));
        let request = model
            .update(Action::ToggleRunSheet(3))
            .effect
            .expect("one-run request");
        assert!(request.check_overrides.no_require_effort);
        assert!(request.check_overrides.no_traces);
        assert_eq!(
            request.check_overrides.warning_policy,
            WarningPolicyOverride::Deny
        );
        assert_eq!(model.pending_check_overrides, CheckOverrides::default());
        assert_eq!(model.modal, None);

        model.update(Action::OpenRunSheet);
        model.update(Action::ToggleRunSheet(0));
        assert!(model.update(Action::ToggleRunSheet(3)).effect.is_none());
        assert!(model.pending_check_overrides.no_require_effort);
        assert_eq!(model.modal, Some(Modal::RunSheet));
        model.update(Action::CloseLayer);
        assert_eq!(model.pending_check_overrides, CheckOverrides::default());

        model.update(Action::EffectRejected {
            id: request.id,
            reason: "test completion".to_string(),
        });
        let ordinary = model
            .update(Action::Refresh)
            .effect
            .expect("ordinary refresh");
        assert_eq!(ordinary.check_overrides, CheckOverrides::default());
    }

    /// Proof for `tui.diagnostic_workbench_retains_complete_runs`: refresh
    /// tracks semantic identities rather than row numbers and reports its set
    /// delta even when the selected finding survives at a different index.
    #[test]
    fn diagnostic_refresh_preserves_identity_and_announces_delta() {
        let before = with_check_findings(
            snapshot("one", 2),
            vec![
                finding(
                    "gone",
                    "APLAN004",
                    aplan_app::Severity::Error,
                    Some("T-0"),
                    None,
                ),
                finding(
                    "kept",
                    "APLAN009",
                    aplan_app::Severity::Warning,
                    Some("T-1"),
                    None,
                ),
            ],
            true,
        );
        let mut model = Model::new(before);
        model.update(Action::Route(Route::Diagnostics));
        model.update(Action::SelectIndex(1));
        let request = model.update(Action::Refresh).effect.expect("refresh");
        let after = with_check_findings(
            snapshot("two", 2),
            vec![
                finding(
                    "new",
                    "APLAN004",
                    aplan_app::Severity::Error,
                    Some("T-0"),
                    None,
                ),
                finding(
                    "kept",
                    "APLAN009",
                    aplan_app::Severity::Warning,
                    Some("T-1"),
                    None,
                ),
            ],
            true,
        );
        model.update(completion(request, Ok(EffectOutput::snapshot(after))));
        assert_eq!(model.selected_finding_id.as_deref(), Some("kept"));
        assert_eq!(model.selected, 1);
        assert!(model.status_line().contains("1 new, 1 removed"));
    }

    /// Proof for `tui.diagnostic_workbench_retains_complete_runs`: rule lookup
    /// remains exact and task navigation returns to the originating finding.
    #[test]
    fn exact_rule_lookup_and_task_navigation_return_to_the_finding() {
        let before = with_check_findings(
            snapshot("one", 2),
            vec![finding(
                "effort",
                "APLAN009",
                aplan_app::Severity::Warning,
                Some("T-1"),
                Some("task.md"),
            )],
            false,
        );
        let mut model = Model::new(before);
        model.update(Action::Route(Route::Diagnostics));
        model.update(Action::OpenContextualRule);
        assert_eq!(model.modal, Some(Modal::RuleExplanation));
        assert_eq!(
            model
                .rule_explanation
                .as_ref()
                .map(|rule| rule.code.as_str()),
            Some("APLAN009")
        );
        model.update(Action::CloseLayer);
        model.update(Action::OpenRuleSearch);
        model.update(Action::SearchInput("aplan009".to_string()));
        model.update(Action::SubmitRuleSearch);
        assert_eq!(model.modal, Some(Modal::RuleSearch));
        assert!(model.status_line().contains("unknown rule `aplan009`"));
        model.update(Action::CloseLayer);

        model.update(Action::OpenFindingTask);
        assert_eq!((model.route, model.focus), (Route::Tasks, Focus::Detail));
        assert_eq!(model.selected_id.as_deref(), Some("T-1"));
        model.update(Action::CloseLayer);
        assert_eq!(
            (model.route, model.focus),
            (Route::Diagnostics, Focus::Detail)
        );
        assert_eq!(model.selected_finding_id.as_deref(), Some("effort"));
    }

    #[test]
    fn duplicate_effects_coalesce_and_the_queue_is_bounded() {
        let mut model = Model::new(snapshot("one", 1));
        assert!(model.update(Action::Refresh).effect.is_some());
        assert!(model.update(Action::Refresh).effect.is_none());
        assert!(model.update(Action::Verify).effect.is_some());
        assert!(model.update(Action::Refresh).effect.is_none());
        assert_eq!(model.pending_effects(), MAX_PENDING_EFFECTS);
        assert!(model.status_line().contains("refresh already pending"));
        assert!(
            model
                .request_effect(EffectKind::Probe, CheckOverrides::default())
                .effect
                .is_none()
        );
        assert!(model.status_line().contains("queue is full"));
    }

    #[test]
    fn a_pending_verification_cannot_hide_a_different_trust_choice() {
        let mut model = Model::new(snapshot("one", 1));
        let resolve_only = model
            .request_verification(false)
            .effect
            .expect("resolve only");
        let execute = model.request_verification(true).effect;
        assert!(!resolve_only.execute);
        assert!(execute.is_none());
        assert_eq!(model.pending_effects(), 1);
        assert!(model.status_line().contains("different options"));
    }

    #[test]
    fn cancellation_is_honest_about_queued_and_running_work() {
        let mut model = Model::new(snapshot("one", 1));
        let running = model.update(Action::Refresh).effect.expect("running");
        model.update(Action::EffectStarted(running.id));
        let queued = model.request_verification(false).effect.expect("queued");

        let transition = model.update(Action::CancelQueued);
        assert_eq!(transition.cancel, Some(queued.id));
        model.accept_cancel(queued.id, true);
        assert_eq!(
            model.effect_mut(queued.id).map(|record| record.state),
            Some(EffectState::Cancelled)
        );

        let transition = model.update(Action::CancelQueued);
        assert_eq!(transition.cancel, None);
        assert!(model.status_line().contains("not interruptible"));
        assert_eq!(
            model.effect_mut(running.id).map(|record| record.state),
            Some(EffectState::Running)
        );
    }

    /// Proof for `tui.verification_is_typed_and_non_blocking` and
    /// `tui.external_refresh_is_identity_safe`: a late answer
    /// stays visibly stale and cannot replace the current identified input.
    #[test]
    fn stale_completion_is_visible_and_cannot_replace_the_snapshot() {
        let mut model = Model::new(snapshot("one", 1));
        let replacing = model.update(Action::Refresh).effect.expect("refresh");
        let old = model.update(Action::Verify).effect.expect("verify");
        model.update(completion(
            replacing,
            Ok(EffectOutput::snapshot(snapshot("two", 3))),
        ));
        assert_eq!(model.snapshot.overview.input.sha256, "two");

        model.update(completion(old, Ok(verification("one", 9, 9))));
        assert_eq!(model.snapshot.overview.input.sha256, "two");
        assert!(
            model
                .status_line()
                .contains("Stale verify result discarded")
        );

        let current = model.update(Action::Verify).effect.expect("current verify");
        let EffectOutput::Verification(mut inconsistent) = verification("two", 0, 0) else {
            unreachable!("the helper always returns verification")
        };
        inconsistent.input.trust = aplan_app::Trust::Hermetic;
        model.update(completion(
            current,
            Ok(EffectOutput::Verification(inconsistent)),
        ));
        assert!(model.verification.is_none());
        assert!(
            model
                .status_line()
                .contains("Stale verify result discarded")
        );
    }

    /// Proof for `tui.external_refresh_is_identity_safe`: a low-cost probe is
    /// silent when identity is unchanged and schedules an atomic refresh when
    /// any identified input moves. The prior complete snapshot remains visible
    /// until that refresh completes.
    #[test]
    fn external_probe_refreshes_only_after_identity_moves() {
        let mut model = Model::new(snapshot("one", 2));
        assert!(model.take_dirty());
        let same = model
            .update(Action::PollExternal)
            .effect
            .expect("identity probe");
        assert!(
            !model.take_dirty(),
            "a hidden probe does not request a frame"
        );
        model.update(Action::EffectStarted(same.id));
        assert!(!model.needs_tick(), "a hidden probe has no animation clock");
        assert_eq!(model.status_line(), "Idle");
        assert!(!model.take_dirty(), "probe start remains silent");
        let identity = model.snapshot.overview.input.clone();
        let transition = model.update(completion(
            same,
            Ok(EffectOutput::Identity(Box::new(identity))),
        ));
        assert!(transition.effect.is_none());
        assert!(!model.take_dirty(), "an unchanged probe does not redraw");
        assert_eq!(model.pending_effects(), 0);
        assert_eq!(model.snapshot.overview.input.sha256, "one");
        assert_eq!(model.freshness, SnapshotFreshness::Current);

        let inactive = model
            .update(Action::PollExternal)
            .effect
            .expect("probe before focus loss");
        model.update(Action::FocusLost);
        assert!(model.update(Action::PollExternal).effect.is_none());
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        let mut changed_while_inactive = model.snapshot.overview.input.clone();
        changed_while_inactive.sha256 = "changed-while-inactive".to_string();
        let transition = model.update(completion(
            inactive,
            Ok(EffectOutput::Identity(Box::new(changed_while_inactive))),
        ));
        assert!(
            transition.effect.is_none(),
            "a pre-focus-loss probe cannot start hidden refresh work"
        );
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        assert_eq!(model.snapshot.overview.input.sha256, "one");

        let regained = model
            .update(Action::FocusGained)
            .effect
            .expect("focus gain probes");
        let identity = model.snapshot.overview.input.clone();
        model.update(completion(
            regained,
            Ok(EffectOutput::Identity(Box::new(identity))),
        ));
        assert_eq!(model.freshness, SnapshotFreshness::Current);
        assert!(model.status_line().contains("confirmed unchanged"));

        let changed = model
            .update(Action::PollExternal)
            .effect
            .expect("second identity probe");
        let mut identity = model.snapshot.overview.input.clone();
        identity.sha256 = "two".to_string();
        identity.plan_sha256 = "plan-two".to_string();
        let transition = model.update(completion(
            changed,
            Ok(EffectOutput::Identity(Box::new(identity))),
        ));
        let refresh = transition.effect.expect("changed identity refreshes");
        assert_eq!(refresh.kind, EffectKind::Refresh);
        assert_eq!(model.snapshot.overview.input.sha256, "one");
        assert_eq!(model.freshness, SnapshotFreshness::Refreshing);

        let mut replacement = snapshot("two", 2);
        replacement.overview.input.plan_sha256 = "plan-two".to_string();
        replacement.overview.load_state = aplan_app::OverviewLoadState::Malformed;
        model.update(completion(refresh, Ok(EffectOutput::snapshot(replacement))));
        assert_eq!(model.snapshot.overview.input.sha256, "two");
        assert_eq!(
            model.snapshot.overview.load_state,
            aplan_app::OverviewLoadState::Malformed
        );
        assert!(model.status_line().contains("plan input changed"));
        assert!(model.status_line().contains("complete to malformed"));
    }

    /// Proof for `tui.external_refresh_is_identity_safe`: the transition text
    /// identifies independently moving configuration, repository and load
    /// state instead of collapsing every refresh into an ambiguous message.
    #[test]
    fn identity_transition_messages_name_configuration_repository_and_load_state() {
        let before = snapshot("before", 1).overview.input;
        let mut after = before.clone();
        after.configuration.sha256 = "changed-configuration".to_string();
        after.repository = aplan_app::RepositoryIdentity::Observed {
            head: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            modified: 2,
            untracked: 1,
            status_sha256: "changed-status".to_string(),
        };
        after.sha256 = "after".to_string();

        let changes = refresh_identity_changes(
            &before,
            &after,
            aplan_app::OverviewLoadState::Complete,
            aplan_app::OverviewLoadState::PartiallyUnloadable,
        );
        assert_eq!(
            changes,
            [
                "configuration changed",
                "repository is now HEAD 0123456789ab, dirty (2 modified, 1 untracked)",
                "load state changed from complete to partially unloadable",
            ]
        );
        assert_eq!(
            repository_identity_summary(&aplan_app::RepositoryIdentity::NotObserved {
                reason: "hermetic".to_string(),
            }),
            "not observed"
        );
        assert_eq!(
            repository_identity_summary(&aplan_app::RepositoryIdentity::Unavailable {
                reason: "not a repository".to_string(),
            }),
            "unavailable"
        );
        assert_eq!(
            repository_identity_summary(&aplan_app::RepositoryIdentity::Observed {
                head: None,
                modified: 0,
                untracked: 0,
                status_sha256: "empty".to_string(),
            }),
            "HEAD unborn, clean"
        );
    }

    /// Proof for `tui.external_refresh_is_identity_safe`: a first probe
    /// failure is visible, repeated unavailability while already unconfirmed
    /// stays quiet, and an executor/model type mismatch fails closed.
    #[test]
    fn probe_failures_and_wrong_outputs_remain_explicit_without_retry_noise() {
        let mut model = Model::new(snapshot("current", 1));
        let failed = model.update(Action::PollExternal).effect.expect("probe");
        model.update(completion(
            failed,
            Err(EffectFailure::Repository("status failed".to_string())),
        ));
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        assert!(model.status_line().contains("status failed"));

        model.notice = None;
        let retry = model.update(Action::PollExternal).effect.expect("retry");
        model.update(completion(
            retry,
            Err(EffectFailure::Unavailable("still unavailable".to_string())),
        ));
        assert!(
            model.notice.is_none(),
            "an unchanged unavailable state must not produce five-second notice spam"
        );

        let mismatched = model
            .update(Action::PollExternal)
            .effect
            .expect("mismatched probe");
        model.update(completion(
            mismatched,
            Ok(EffectOutput::snapshot(snapshot("wrong-kind", 1))),
        ));
        assert!(model.status_line().contains("wrong result type"));
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
    }

    /// Proof for `tui.external_refresh_is_identity_safe`: a source that fails
    /// during a probe or changes again while replacement state is built never
    /// replaces the last complete snapshot.
    #[test]
    fn unloadable_and_racing_external_edits_keep_the_complete_snapshot() {
        let mut model = Model::new(snapshot("complete", 2));
        let probe = model.update(Action::PollExternal).effect.expect("probe");
        model.update(completion(
            probe,
            Err(EffectFailure::Repository(
                "configuration is temporarily invalid".to_string(),
            )),
        ));
        assert_eq!(model.snapshot.overview.input.sha256, "complete");
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        assert!(model.status_line().contains("temporarily invalid"));

        let refresh = model
            .update(Action::Refresh)
            .effect
            .expect("manual refresh");
        model.update(completion(
            refresh,
            Err(EffectFailure::Stale {
                expected: "revision-a".to_string(),
                actual: "revision-b".to_string(),
            }),
        ));
        assert_eq!(model.snapshot.overview.input.sha256, "complete");
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        assert!(model.status_line().contains("previous complete snapshot"));

        let refresh = model
            .update(Action::Refresh)
            .effect
            .expect("coherent refresh before focus loss");
        model.update(Action::FocusLost);
        model.update(completion(
            refresh,
            Ok(EffectOutput::snapshot(snapshot("coherent", 2))),
        ));
        assert_eq!(model.snapshot.overview.input.sha256, "coherent");
        assert_eq!(
            model.freshness,
            SnapshotFreshness::Unconfirmed,
            "completion while inactive must not make a currency claim"
        );
    }

    /// Proof for `tui.focus_epoch_invalidates_external_results`: a complete
    /// focus cycle invalidates both kinds of external-state observation even
    /// when focus returns before their completions are delivered.
    #[test]
    fn focus_epoch_invalidates_pre_loss_probe_and_refresh_results() {
        let mut model = Model::new(snapshot("one", 2));
        let pre_loss = model
            .update(Action::PollExternal)
            .effect
            .expect("probe before a complete focus cycle");
        assert_eq!(pre_loss.focus_epoch, 0);
        model.update(Action::FocusLost);
        assert!(model.update(Action::FocusGained).effect.is_none());

        let identity = model.snapshot.overview.input.clone();
        let transition = model.update(completion(
            pre_loss,
            Ok(EffectOutput::Identity(Box::new(identity))),
        ));
        let probe = transition
            .effect
            .expect("the pre-loss result is replaced immediately");
        assert_eq!(probe.kind, EffectKind::Probe);
        assert_eq!(probe.focus_epoch, 1);
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        let identity = model.snapshot.overview.input.clone();
        model.update(completion(
            probe,
            Ok(EffectOutput::Identity(Box::new(identity))),
        ));
        assert_eq!(model.freshness, SnapshotFreshness::Current);

        let refresh = model
            .update(Action::Refresh)
            .effect
            .expect("refresh before another complete focus cycle");
        model.update(Action::FocusLost);
        assert!(model.update(Action::FocusGained).effect.is_none());
        let transition = model.update(completion(
            refresh,
            Ok(EffectOutput::snapshot(snapshot("crossed-focus", 2))),
        ));
        assert_eq!(model.snapshot.overview.input.sha256, "crossed-focus");
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        let probe = transition
            .effect
            .expect("a crossed-focus refresh gets a current-epoch probe");
        assert_eq!(probe.kind, EffectKind::Probe);
        assert_eq!(probe.focus_epoch, 2);
        let identity = model.snapshot.overview.input.clone();
        model.update(completion(
            probe,
            Ok(EffectOutput::Identity(Box::new(identity))),
        ));
        assert_eq!(model.freshness, SnapshotFreshness::Current);
    }

    /// Proof for `tui.continue_establishes_freshness_boundary`: the handled
    /// stop path and the continue path may both report the same observation
    /// gap, but an already-inactive model advances its epoch only once.
    #[test]
    fn repeated_focus_loss_is_idempotent_until_focus_returns() {
        let mut model = Model::new(snapshot("one", 1));
        model.update(Action::FocusLost);
        model.update(Action::FocusLost);
        let probe = model
            .update(Action::FocusGained)
            .effect
            .expect("resume requests a probe");

        assert_eq!(probe.focus_epoch, 1);
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
    }

    #[test]
    fn an_unknown_or_mistyped_completion_cannot_change_the_snapshot() {
        let mut model = Model::new(snapshot("one", 1));
        let unknown = EffectRequest {
            id: EffectId(99),
            generation: 0,
            focus_epoch: 0,
            input: "one".to_string(),
            kind: EffectKind::Refresh,
            check_overrides: CheckOverrides::default(),
            execute: false,
            mutation: None,
        };
        model.update(completion(
            unknown,
            Ok(EffectOutput::snapshot(snapshot("unknown", 9))),
        ));
        assert_eq!(model.snapshot.overview.input.sha256, "one");
        assert!(model.status_line().contains("Unrecognized refresh"));

        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        model.update(completion(refresh, Ok(verification("one", 0, 0))));
        assert_eq!(model.snapshot.overview.input.sha256, "one");
        assert!(
            model
                .status_line()
                .contains("wrong result type for refresh")
        );
    }

    /// Proof for `tui.verification_execution_is_preview_bound`: a preflight
    /// mismatch is stale work, not a provider failure, and retains the last
    /// complete model.
    #[test]
    fn a_failed_effect_keeps_the_last_complete_model() {
        for (failure, message) in [
            (
                EffectFailure::Repository("unreadable".to_string()),
                "repository operation failed",
            ),
            (EffectFailure::WorkerPanicked, "background worker panicked"),
        ] {
            let mut model = Model::new(snapshot("complete", 2));
            let request = model.update(Action::Refresh).effect.expect("request");
            model.update(completion(request, Err(failure)));
            assert_eq!(model.snapshot, snapshot("complete", 2));
            assert!(model.status_line().contains(message));
        }

        let mut model = Model::new(snapshot("complete", 2));
        let request = model.update(Action::Verify).effect.expect("verify request");
        let id = request.id;
        model.update(completion(
            request,
            Err(EffectFailure::Unavailable(
                "configuration could not be loaded".to_string(),
            )),
        ));
        assert_eq!(
            model.effect_mut(id).map(|record| record.state),
            Some(EffectState::Unavailable),
            "unavailable is not a generic provider failure"
        );

        let request = model.update(Action::Verify).effect.expect("verify request");
        let id = request.id;
        model.update(completion(
            request,
            Err(EffectFailure::Stale {
                expected: "reviewed".to_string(),
                actual: "changed".to_string(),
            }),
        ));
        assert_eq!(
            model.effect_mut(id).map(|record| record.state),
            Some(EffectState::Stale),
            "a preflight mismatch is not a provider failure"
        );
        assert!(
            model
                .status_line()
                .contains("result discarded after its identified input changed")
        );
    }

    #[test]
    fn idle_state_neither_ticks_nor_redraws() {
        let mut model = Model::new(snapshot("one", 1));
        assert!(model.take_dirty(), "the first frame is dirty");
        assert!(!model.take_dirty(), "idle does not redraw");
        assert!(!model.needs_tick(), "idle has no clock");
        model.update(Action::Tick);
        assert!(!model.take_dirty(), "an unneeded tick changes nothing");
    }

    /// Proof for `tui.visual_language_is_semantic_and_deterministic`: reduced
    /// motion preserves the exact operation state while removing its clock.
    #[test]
    fn reduced_motion_keeps_progress_state_without_animation() {
        let mut model = Model::new(snapshot("one", 1));
        model.motion = Motion::Reduced;
        let request = model.update(Action::Refresh).effect.expect("refresh");
        model.update(Action::EffectStarted(request.id));
        assert_eq!(model.status_line(), "* refresh in progress");
        assert!(
            !model.needs_tick(),
            "a static marker needs no animation clock"
        );
        model.update(Action::Tick);
        assert_eq!(model.status_line(), "* refresh in progress");

        let build = BuildIdentity::default();
        let capabilities = aplan_app::BuildFacts::default().capability_view(&build);
        let (mut loading, request) = Model::loading(
            &build,
            capabilities,
            Appearance {
                reduced_motion: true,
                ..Appearance::default()
            },
        );
        loading.update(Action::EffectStarted(request.id));
        assert!(loading.startup.is_some());
        assert!(
            !loading.needs_tick(),
            "reduced-motion startup repaints only on progress events"
        );
        assert!(loading.take_dirty(), "startup has one pending frame");
        loading.update(Action::Tick);
        assert!(!loading.take_dirty(), "a disabled startup clock is inert");
    }

    #[test]
    fn visible_actions_and_effect_completion_each_request_a_frame() {
        let mut model = Model::new(snapshot("one", 2));
        assert!(model.take_dirty());

        model.update(Action::SelectNext);
        assert!(model.take_dirty());
        assert!(!model.take_dirty());

        model.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        assert!(model.take_dirty());
        assert!(!model.take_dirty());

        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        assert!(model.take_dirty());
        assert!(!model.take_dirty());

        model.update(completion(
            refresh,
            Ok(EffectOutput::snapshot(snapshot("two", 3))),
        ));
        assert!(model.take_dirty());
        assert!(!model.take_dirty());
    }

    #[test]
    fn ticks_exist_only_while_visible_transient_state_needs_them() {
        let mut model = Model::new(snapshot("one", 1));
        model.take_dirty();
        model.update(Action::Paste("x".to_string()));
        assert!(model.needs_tick());
        for _ in 0..NOTICE_TICKS {
            model.update(Action::Tick);
            assert!(model.take_dirty());
        }
        assert!(!model.needs_tick());
        model.update(Action::Tick);
        assert!(!model.take_dirty());
    }

    #[test]
    fn palette_storage_is_bounded_even_when_input_occupies_no_cells() {
        use unicode_segmentation::UnicodeSegmentation as _;

        let mut model = Model::new(snapshot("one", 1));
        model.update(Action::OpenPalette);
        model.update(Action::PaletteInput("\u{200b}".repeat(4_096)));
        assert!(model.palette_query.len() <= MAX_PALETTE_QUERY_BYTES);
        assert!(model.palette_query.graphemes(true).count() <= MAX_PALETTE_QUERY_GRAPHEMES);
        assert!(text::cells(&model.palette_query) <= MAX_PALETTE_QUERY_CELLS);
    }

    #[test]
    fn inbox_projection_filters_searches_orders_and_opens_the_promoted_task_link() {
        use aplan_core::capture::{CaptureState, RevisitFilter};

        let mut older = capture_record(
            "I-older",
            "older actionable",
            CaptureState::Open,
            "2026-01-01T00:00:01Z",
        );
        older.labels = vec!["alpha".to_string()];
        let mut newer = capture_record(
            "I-newer",
            "newer \u{754c} actionable",
            CaptureState::Open,
            "2026-01-01T00:00:02Z",
        );
        newer.kind = "question".to_string();
        newer.labels = vec!["beta".to_string()];
        newer.body = Some("searchable unicode \u{754c}".to_string());
        let mut deferred = capture_record(
            "I-deferred",
            "review later",
            CaptureState::Open,
            "2026-01-01T00:00:03Z",
        );
        deferred.revisit_at = Some("2030-01-01T00:00:00Z".to_string());
        let mut promoted = capture_record(
            "I-promoted",
            "promoted record",
            CaptureState::Promoted,
            "2026-01-01T00:00:04Z",
        );
        promoted.promoted_to = Some("A-1".to_string());
        let discarded = capture_record(
            "I-discarded",
            "discarded record",
            CaptureState::Discarded,
            "2026-01-01T00:00:05Z",
        );
        let mut model = ModelBuilder::default()
            .inbox(
                vec![older, newer, deferred, promoted, discarded],
                "2026-01-01T00:00:10Z",
            )
            .build();
        model.update(Action::Route(Route::Inbox));

        assert_eq!(
            model
                .inbox_rows()
                .iter()
                .map(|capture| capture.id.as_str())
                .collect::<Vec<_>>(),
            ["I-newer", "I-older"]
        );
        model.inbox_state = None;
        model.inbox_revisit = RevisitFilter::All;
        model.inbox_query = "\u{754c}".to_string();
        assert_eq!(model.inbox_rows()[0].id, "I-newer");
        model.inbox_query.clear();
        model.inbox_kind = Some("question".to_string());
        assert_eq!(model.inbox_rows()[0].id, "I-newer");
        model.inbox_kind = None;
        model.inbox_label = Some("alpha".to_string());
        assert_eq!(model.inbox_rows()[0].id, "I-older");

        model.inbox_label = None;
        model.inbox_state = Some(CaptureState::Promoted);
        model.selected_capture_id = Some("I-promoted".to_string());
        model.reconcile_inbox_selection();
        model.update(Action::OpenDetail);
        assert_eq!(model.focus, Focus::Detail);
        model.update(Action::OpenDetail);
        assert_eq!(model.route, Route::Tasks);
        assert_eq!(model.focus, Focus::Detail);
        assert_eq!(model.selected_id.as_deref(), Some("A-1"));
        assert_eq!(model.inspector_section, InspectorSection::Summary);
    }

    fn inbox_control_model() -> Model {
        use aplan_core::capture::CaptureState;

        let mut actionable = capture_record(
            "I-actionable",
            "Actionable intake",
            CaptureState::Open,
            "2026-01-01T00:00:01Z",
        );
        actionable.labels = vec!["alpha".to_string()];
        let mut deferred = capture_record(
            "I-deferred",
            "Deferred intake",
            CaptureState::Open,
            "2026-01-01T00:00:02Z",
        );
        deferred.kind = "question".to_string();
        deferred.labels = vec!["beta".to_string()];
        deferred.body = Some("searchable deferred body".to_string());
        deferred.revisit_at = Some("2030-01-01T00:00:00Z".to_string());
        let mut promoted = capture_record(
            "I-promoted",
            "Promoted intake",
            CaptureState::Promoted,
            "2026-01-01T00:00:03Z",
        );
        promoted.promoted_to = Some("A-missing".to_string());
        let discarded = capture_record(
            "I-discarded",
            "Discarded intake",
            CaptureState::Discarded,
            "2026-01-01T00:00:04Z",
        );
        let mut model = ModelBuilder::default()
            .inbox(
                vec![actionable, deferred, promoted, discarded],
                "2026-01-01T00:00:10Z",
            )
            .build();
        model.update(Action::Route(Route::Inbox));
        model
    }

    #[test]
    fn inbox_filter_controls_project_each_typed_dimension_and_reset_to_defaults() {
        use aplan_core::capture::{CaptureState, RevisitFilter};

        let mut model = inbox_control_model();

        let labels = model
            .inbox_filter_choices()
            .iter()
            .map(InboxFilterChoice::label)
            .collect::<Vec<_>>();
        for expected in [
            "State: OPEN",
            "State: PROMOTED",
            "State: DISCARDED",
            "State: all",
            "Review: actionable",
            "Review: deferred",
            "Review: all",
            "Kind: all",
            "Kind: question",
            "Label: all",
            "Label: alpha",
            "Label: beta",
        ] {
            assert!(labels.iter().any(|label| label == expected), "{labels:?}");
        }

        model.update(Action::OpenFilters);
        let all_states = model
            .inbox_filter_choices()
            .iter()
            .position(|choice| matches!(choice, InboxFilterChoice::State(None)))
            .expect("all-states filter");
        model.update(Action::ToggleFilter(all_states));
        let deferred_only = model
            .inbox_filter_choices()
            .iter()
            .position(|choice| {
                matches!(choice, InboxFilterChoice::Revisit(RevisitFilter::Deferred))
            })
            .expect("deferred filter");
        model.update(Action::ToggleFilter(deferred_only));
        let questions = model
            .inbox_filter_choices()
            .iter()
            .position(|choice| {
                matches!(choice, InboxFilterChoice::Kind(Some(kind)) if kind == "question")
            })
            .expect("question filter");
        model.update(Action::ToggleFilter(questions));
        let beta = model
            .inbox_filter_choices()
            .iter()
            .position(
                |choice| matches!(choice, InboxFilterChoice::Label(Some(label)) if label == "beta"),
            )
            .expect("beta filter");
        model.update(Action::ToggleFilter(beta));
        assert_eq!(model.inbox_rows()[0].id, "I-deferred");
        for choice in model.inbox_filter_choices() {
            let selected = model.inbox_filter_selected(&choice);
            if matches!(
                choice,
                InboxFilterChoice::State(None)
                    | InboxFilterChoice::Revisit(RevisitFilter::Deferred)
            ) {
                assert!(selected, "{choice:?}");
            }
        }

        model.update(Action::ClearFilters);
        assert_eq!(model.inbox_state, Some(CaptureState::Open));
        assert_eq!(model.inbox_revisit, RevisitFilter::Actionable);
        assert!(!model.has_inbox_narrowing());
    }

    #[test]
    fn inbox_search_selection_and_missing_promotion_link_remain_semantic() {
        use aplan_core::capture::{CaptureState, RevisitFilter};

        let mut model = inbox_control_model();

        model.inbox_state = None;
        model.inbox_revisit = RevisitFilter::All;
        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("deferred body".to_string()));
        assert_eq!(model.selected_capture_id.as_deref(), Some("I-deferred"));
        model.update(Action::SearchHome);
        assert_eq!(model.inbox_query_cursor, 0);
        model.update(Action::SearchRight);
        model.update(Action::SearchEnd);
        assert_eq!(model.inbox_query_cursor, model.inbox_query.len());
        model.update(Action::SearchLeft);
        model.update(Action::SearchDelete);
        model.update(Action::SearchBackspace);
        model.update(Action::SearchClear);
        assert!(model.inbox_query.is_empty());
        model.update(Action::CloseLayer);

        model.selected_capture_id = None;
        model.update(Action::SelectNext);
        assert!(model.selected_capture_id.is_some());
        model.selected_capture_id = None;
        model.update(Action::SelectPrevious);
        assert!(model.selected_capture_id.is_some());
        model.update(Action::SelectIndex(1));
        assert_eq!(model.selected_capture_visible_index(), Some(1));

        model.inbox_query = "none".to_string();
        model.inbox_kind = Some("question".to_string());
        model.inbox_label = Some("beta".to_string());
        model.update(Action::ClearNarrowing);
        assert!(!model.has_inbox_narrowing());

        model.inbox_state = Some(CaptureState::Promoted);
        model.inbox_revisit = RevisitFilter::All;
        model.reconcile_inbox_selection();
        model.update(Action::OpenDetail);
        model.update(Action::OpenDetail);
        assert!(
            model
                .notice
                .as_ref()
                .is_some_and(|notice| notice.text.contains("not present"))
        );
        model.update(Action::CloseLayer);
        assert_eq!(model.focus, Focus::Primary);

        model.modal = Some(Modal::Filters);
        model.modal_selected = usize::MAX;
        model.refresh_visible_projection(false);
        assert!(model.modal_selected < model.inbox_filter_choices().len());
    }

    #[test]
    fn inbox_post_mutation_selection_reveals_each_lifecycle_destination() {
        use aplan_core::capture::CaptureState;

        let mut model = inbox_control_model();

        model.update(Action::Route(Route::Reports));
        model.select_after_mutation(MutationAction::CaptureCreate, "I-actionable".to_string());
        assert_eq!(model.selected_capture_id.as_deref(), Some("I-actionable"));
        model.update(Action::Route(Route::Inbox));
        for (action, id, state) in [
            (
                MutationAction::CaptureDiscard,
                "I-discarded",
                CaptureState::Discarded,
            ),
            (
                MutationAction::CapturePromote,
                "I-promoted",
                CaptureState::Promoted,
            ),
            (
                MutationAction::CaptureRestore,
                "I-actionable",
                CaptureState::Open,
            ),
            (
                MutationAction::CaptureAmend,
                "I-deferred",
                CaptureState::Open,
            ),
        ] {
            model.select_after_mutation(action, id.to_string());
            assert_eq!(model.inbox_state, Some(state));
            assert_eq!(model.selected_capture_id.as_deref(), Some(id));
        }
    }

    #[test]
    fn global_quick_capture_preserves_the_current_navigation_context() {
        let mut model = ModelBuilder::default().build();
        model.update(Action::Route(Route::Reports));
        model.update(Action::Focus(Focus::Detail));

        let transition = model.update(Action::OpenMutation(MutationAction::CaptureCreate));

        assert!(transition.effect.is_some());
        assert_eq!(model.route, Route::Reports);
        assert_eq!(model.focus, Focus::Detail);
        let layer = model.mutation.as_ref().expect("capture layer");
        assert_eq!(layer.action, MutationAction::CaptureCreate);
        assert!(layer.selected_task.is_none());
    }

    #[test]
    fn navigation_wraps_without_waiting_for_an_effect() {
        let mut model = Model::new(snapshot("one", 2));
        model.update(Action::Route(Route::Tasks));
        model.update(Action::SelectNext);
        assert_eq!(model.selected, 1);
        model.update(Action::SelectNext);
        assert_eq!(model.selected, 0);
        model.update(Action::SelectPrevious);
        assert_eq!(model.selected, 1);
    }

    /// Proof for `tui.overview_is_one_typed_projection`: every Overview row
    /// opens the exact task or finding identities carried by the projection,
    /// including an honestly empty aggregate.
    #[test]
    fn overview_aggregates_drill_into_their_exact_projection_members() {
        let mut state = snapshot("one", 3);
        state.overview.cut_task_ids = vec!["T-2".to_string()];
        state.overview.live_task_ids = vec!["T-0".to_string(), "T-2".to_string()];
        state.overview.ready = vec![aplan_app::OverviewTask {
            id: "T-1".to_string(),
            title: "task 1".to_string(),
        }];
        state.overview.due = vec![aplan_app::OverviewDueTask {
            id: "T-0".to_string(),
            title: "task 0".to_string(),
            revisit_at: "1970-01-01T00:00:00Z".to_string(),
            overdue_seconds: 1,
        }];
        state.overview.waiting_revisits = vec![aplan_app::OverviewWaitingTask {
            id: "T-2".to_string(),
            title: "task 2".to_string(),
            revisit_at: "1970-01-01T00:00:02Z".to_string(),
            seconds_until: 1,
        }];
        state.overview.critical_path = Some((
            3.0,
            vec![aplan_app::OverviewTask {
                id: "T-0".to_string(),
                title: "task 0".to_string(),
            }],
        ));

        for (metric, expected) in [
            (OverviewMetric::Cuts, vec!["T-2"]),
            (OverviewMetric::Effort, vec!["T-0", "T-2"]),
            (OverviewMetric::Ready, vec!["T-1"]),
            (OverviewMetric::Due, vec!["T-0", "T-2"]),
            (OverviewMetric::CriticalPath, vec!["T-0"]),
        ] {
            let mut model = Model::new(state.clone());
            let index = OverviewMetric::ALL
                .iter()
                .position(|candidate| *candidate == metric)
                .expect("metric");
            model.update(Action::SelectIndex(index));
            model.update(Action::OpenDetail);
            assert_eq!(model.route, Route::Tasks);
            assert_eq!(
                model
                    .task_browser
                    .rows
                    .iter()
                    .map(|row| row.task.id.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{metric:?}"
            );
            model.update(Action::ClearNarrowing);
            assert_eq!(model.task_browser.rows.len(), 3);
        }

        let mut model = Model::new(state.clone());
        let findings = OverviewMetric::ALL
            .iter()
            .position(|metric| *metric == OverviewMetric::Findings)
            .expect("findings metric");
        model.update(Action::SelectIndex(findings));
        model.update(Action::OpenDetail);
        assert_eq!(model.route, Route::Diagnostics);
        assert_eq!(model.snapshot.overview.findings.len(), 0);

        let mut model = Model::new(state);
        let ready = OverviewMetric::ALL
            .iter()
            .position(|metric| *metric == OverviewMetric::Ready)
            .expect("ready metric");
        model.update(Action::SelectIndex(ready));
        model.update(Action::OpenDetail);
        assert_eq!(model.task_browser.rows[0].task.id, "T-1");
        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let mut refreshed = snapshot("two", 3);
        refreshed.overview.ready = vec![aplan_app::OverviewTask {
            id: "T-2".to_string(),
            title: "task 2".to_string(),
        }];
        model.update(completion(refresh, Ok(EffectOutput::snapshot(refreshed))));
        assert_eq!(
            model
                .task_browser
                .rows
                .iter()
                .map(|row| row.task.id.as_str())
                .collect::<Vec<_>>(),
            ["T-2"],
            "a retained aggregate scope must re-project from the new snapshot"
        );
    }

    /// Proof for `tui.overview_revisit_membership_is_exact`: both sides of the
    /// composite revisit row keep their identities and re-project on refresh.
    #[test]
    fn overview_revisit_row_preserves_due_and_future_membership_after_refresh() {
        let mut first = snapshot("one", 3);
        first.overview.due = vec![aplan_app::OverviewDueTask {
            id: "T-0".to_string(),
            title: "task 0".to_string(),
            revisit_at: "1970-01-01T00:00:00Z".to_string(),
            overdue_seconds: 1,
        }];
        first.overview.waiting_revisits = vec![aplan_app::OverviewWaitingTask {
            id: "T-2".to_string(),
            title: "task 2".to_string(),
            revisit_at: "1970-01-01T00:00:02Z".to_string(),
            seconds_until: 1,
        }];
        let mut model = Model::new(first);
        model.overview_selected = OverviewMetric::ALL
            .iter()
            .position(|metric| *metric == OverviewMetric::Due)
            .expect("due row");
        model.update(Action::OpenDetail);
        assert_eq!(
            model
                .task_browser
                .rows
                .iter()
                .map(|row| row.task.id.as_str())
                .collect::<Vec<_>>(),
            ["T-0", "T-2"]
        );

        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let mut refreshed = snapshot("two", 3);
        refreshed.overview.due = vec![aplan_app::OverviewDueTask {
            id: "T-1".to_string(),
            title: "task 1".to_string(),
            revisit_at: "1970-01-01T00:00:00Z".to_string(),
            overdue_seconds: 2,
        }];
        refreshed.overview.waiting_revisits.clear();
        model.update(completion(refresh, Ok(EffectOutput::snapshot(refreshed))));
        assert_eq!(
            model
                .task_browser
                .rows
                .iter()
                .map(|row| row.task.id.as_str())
                .collect::<Vec<_>>(),
            ["T-1"]
        );
    }

    /// Proof for `tui.overview_is_one_typed_projection`: a failed refresh
    /// cannot relabel retained bytes as current.
    #[test]
    fn overview_freshness_never_calls_a_failed_refresh_current() {
        let mut model = Model::new(snapshot("one", 1));
        assert_eq!(model.freshness, SnapshotFreshness::Current);
        let request = model.update(Action::Refresh).effect.expect("refresh");
        assert_eq!(model.freshness, SnapshotFreshness::Refreshing);
        model.update(completion(
            request,
            Err(EffectFailure::Repository("unreadable".to_string())),
        ));
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);

        let request = model.update(Action::Refresh).effect.expect("retry");
        model.update(completion(
            request,
            Ok(EffectOutput::snapshot(snapshot("two", 1))),
        ));
        assert_eq!(model.freshness, SnapshotFreshness::Current);
    }

    #[test]
    fn task_search_edits_unicode_at_the_cursor_and_preserves_hidden_selection() {
        let mut model = Model::new(snapshot("one", 2));
        model.update(Action::Route(Route::Tasks));
        model.update(Action::SelectIndex(1));
        assert_eq!(model.selected_id.as_deref(), Some("T-1"));
        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("task \u{754c}".to_string()));
        model.update(Action::SearchLeft);
        model.update(Action::SearchInput("0".to_string()));
        assert_eq!(model.task_query, "task 0\u{754c}");
        model.update(Action::SearchDelete);
        assert_eq!(model.task_query, "task 0");
        assert_eq!(model.selected_id.as_deref(), Some("T-1"));
        assert_eq!(model.selected_visible_index(), None);
        model.update(Action::OpenDetail);
        assert_eq!(model.focus, Focus::Primary);

        model.update(Action::SearchBackspace);
        assert_eq!(model.task_query, "task ");
        assert_eq!(model.selected_visible_index(), Some(1));
        model.update(Action::SearchHome);
        model.update(Action::Paste("\u{1b}[2J".to_string()));
        assert!(!model.task_query.contains('\u{1b}'));
        assert!(model.task_query.starts_with("\u{fffd}[2J"));
        model.update(Action::ClearNarrowing);
        assert_eq!(model.selected_id.as_deref(), Some("T-1"));
        assert_eq!(model.selected_visible_index(), Some(1));
    }

    #[test]
    fn structured_owner_and_unclaimed_filters_compose_as_distinct_dimensions() {
        let mut state = snapshot("one", 2);
        state.task_rows[0].owner = Some("lee".to_string());
        state.task_rows[0].tags = vec!["ui".to_string()];
        let mut model = Model::new(state);
        model.update(Action::Route(Route::Tasks));
        model.update(Action::OpenFilters);
        let choices = model.filter_choices();
        let owner = choices
            .iter()
            .position(|filter| filter.label() == "owner: lee")
            .expect("owner choice");
        let unclaimed = choices
            .iter()
            .position(|filter| filter.label() == "unclaimed (no active claim)")
            .expect("unclaimed choice");
        model.update(Action::ToggleFilter(owner));
        assert_eq!(model.task_browser.rows.len(), 1);
        model.update(Action::ToggleFilter(unclaimed));
        assert_eq!(model.task_browser.rows.len(), 1);
        assert_eq!(model.task_browser.rows[0].task.id, "T-0");
        model.update(Action::ClearFilters);
        assert_eq!(model.task_browser.rows.len(), 2);
        assert!(model.task_browser.active_filters.is_empty());
    }

    /// Proof for `tui.task_browser_state_is_refresh_and_repeat_safe`: a
    /// refreshed filter catalogue cannot strand its modal cursor.
    #[test]
    fn refresh_clamps_the_filter_cursor_to_the_refreshed_vocabulary() {
        let mut state = snapshot("one", 4);
        for (index, row) in state.task_rows.iter_mut().enumerate() {
            row.owner = Some(format!("owner-{index}"));
            row.tags = vec![format!("tag-{index}")];
        }
        let mut model = Model::new(state);
        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        model.update(Action::OpenFilters);
        let old_last = model.filter_choices().len() - 1;
        model.update(Action::SelectModal(old_last));

        model.update(completion(
            refresh,
            Ok(EffectOutput::snapshot(snapshot("two", 1))),
        ));
        let refreshed_count = model.filter_choices().len();
        assert!(refreshed_count > 0);
        assert_eq!(model.modal_selected, refreshed_count - 1);

        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        model.update(completion(
            refresh,
            Ok(EffectOutput::snapshot(snapshot("three", 0))),
        ));
        assert!(model.filter_choices().is_empty());
        assert_eq!(model.modal_selected, 0);
    }

    /// Proof for `tui.task_browser_preserves_selector_semantics`: selection is
    /// an entity identity, not a row number in one projection.
    #[test]
    fn refresh_preserves_selection_by_id_and_reports_removal() {
        let mut model = Model::new(snapshot("one", 3));
        model.update(Action::Route(Route::Tasks));
        model.update(Action::SelectIndex(1));
        let selected = model.selected_id.clone().expect("selected id");
        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let mut reordered = snapshot("two", 3);
        reordered.task_rows.reverse();
        reordered
            .task_rows
            .iter_mut()
            .find(|row| row.id == selected)
            .expect("selected row")
            .title = "renamed without changing identity".to_string();
        model.update(completion(refresh, Ok(EffectOutput::snapshot(reordered))));
        assert_eq!(model.selected_id.as_deref(), Some(selected.as_str()));
        assert_eq!(
            model.selected_task().map(|row| row.task.title.as_str()),
            Some("renamed without changing identity")
        );

        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let mut removed = snapshot("three", 3);
        removed.task_rows.retain(|row| row.id != selected);
        model.update(completion(refresh, Ok(EffectOutput::snapshot(removed))));
        assert_eq!(model.selection_state, SelectionState::Removed);
        assert_eq!(model.selected_id.as_deref(), Some(selected.as_str()));
        assert!(model.selected_task().is_none());
    }

    #[test]
    fn refresh_clamps_diagnostic_selection_to_the_new_projection() {
        let mut model = Model::new(Snapshot::test("one", "repo", Vec::new(), Vec::new(), 3, 0));
        model.update(Action::Route(Route::Diagnostics));
        model.update(Action::SelectIndex(2));
        assert_eq!(model.selected, 2);

        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        let refreshed = Snapshot::test("two", "repo", Vec::new(), Vec::new(), 1, 0);
        model.update(completion(refresh, Ok(EffectOutput::snapshot(refreshed))));

        assert_eq!(model.selected, 0);
        assert!(
            model
                .snapshot
                .overview
                .findings
                .get(model.selected)
                .is_some()
        );
    }

    /// Proof for `tui.task_inspector_uses_show_projection` and
    /// `tui.external_refresh_is_identity_safe`: refresh clamps a
    /// link selection and clears snapshot-relative back history while keeping
    /// the reader's scroll position for a task whose stable identity survives.
    #[test]
    fn refresh_reconciles_inspector_state_with_the_new_snapshot() {
        let mut state = snapshot("one", 2);
        state.inspectors[0].evidence.push(aplan_app::Cited {
            kind: "test".to_string(),
            reference: "suite::proof".to_string(),
            captured: None,
            observation: aplan_app::EvidenceObservation::not_observed("test fixture"),
        });
        let mut model = Model::new(state);
        model.update(Action::Route(Route::Tasks));
        model.update(Action::OpenDetail);
        model.inspector_selected = model
            .inspector_items()
            .iter()
            .position(|item| item.link == Some(InspectorLink::Section(InspectorSection::Evidence)))
            .expect("evidence section link");
        model.update(Action::OpenDetail);
        model.inspector_selected = usize::MAX;
        model.inspector_scroll = Some(usize::MAX);
        assert!(model.inspector_has_history());

        let refresh = model.update(Action::Refresh).effect.expect("refresh");
        model.update(completion(
            refresh,
            Ok(EffectOutput::snapshot(snapshot("two", 2))),
        ));

        assert_eq!(model.inspector_section, InspectorSection::Evidence);
        assert_eq!(model.inspector_selected, 0);
        assert_eq!(model.inspector_scroll, Some(usize::MAX));
        assert!(!model.inspector_has_history());
    }

    /// Proof for `tui.task_inspector_uses_show_projection`: section
    /// and task links retain the exact location that Back restores, while a
    /// missing relationship stays visible and cannot become a false task.
    #[test]
    fn inspector_sections_and_relationships_preserve_back_navigation() {
        let mut state = snapshot("one", 3);
        let first = &mut state.inspectors[0];
        first.evidence.push(aplan_app::Cited {
            kind: "test".to_string(),
            reference: "suite::proof".to_string(),
            captured: None,
            observation: aplan_app::EvidenceObservation::not_observed("test fixture"),
        });
        first.findings.push(aplan_app::Finding {
            code: "APLAN999".to_string(),
            severity: "warning".to_string(),
            message: "synthetic finding".to_string(),
            fix: Some("inspect the task".to_string()),
            repair: None,
        });
        first.waits_on = vec![
            aplan_app::Related {
                id: "T-1".to_string(),
                status: "todo".to_string(),
                availability: aplan_app::RelatedAvailability::Loaded,
                source: None,
            },
            aplan_app::Related {
                id: "GHOST".to_string(),
                status: "unknown".to_string(),
                availability: aplan_app::RelatedAvailability::Missing,
                source: None,
            },
        ];
        let mut model = Model::new(state);
        model.update(Action::Route(Route::Tasks));
        model.update(Action::OpenDetail);
        assert_eq!(model.focus, Focus::Detail);

        model.inspector_selected = model
            .inspector_items()
            .iter()
            .position(|item| item.link == Some(InspectorLink::Section(InspectorSection::Evidence)))
            .expect("evidence section link");
        model.update(Action::OpenDetail);
        assert_eq!(model.inspector_section, InspectorSection::Evidence);
        assert!(model.inspector_has_history());
        model.update(Action::SetInspectorScroll(5));
        assert_eq!(model.inspector_scroll, Some(5));
        model.update(Action::OpenDetail);
        assert_eq!(model.inspector_section, InspectorSection::Summary);
        assert_eq!(model.selected_id.as_deref(), Some("T-0"));
        model.update(Action::CloseLayer);
        assert_eq!(model.inspector_section, InspectorSection::Evidence);
        assert_eq!(model.inspector_scroll, Some(5));
        model.update(Action::CloseLayer);
        assert_eq!(model.inspector_section, InspectorSection::Summary);
        assert_eq!(model.selected_id.as_deref(), Some("T-0"));

        model.inspector_section = InspectorSection::Findings;
        model.inspector_selected = 0;
        assert_eq!(
            model.inspector_items()[0].link,
            Some(InspectorLink::Task("T-0".to_string()))
        );
        model.update(Action::OpenDetail);
        assert_eq!(model.inspector_section, InspectorSection::Summary);
        model.update(Action::CloseLayer);
        assert_eq!(model.inspector_section, InspectorSection::Findings);

        model.inspector_section = InspectorSection::Relationships;
        model.inspector_selected = 0;
        model.update(Action::OpenDetail);
        assert_eq!(model.selected_id.as_deref(), Some("T-1"));
        assert_eq!(model.inspector_section, InspectorSection::Summary);
        model.update(Action::CloseLayer);
        assert_eq!(model.selected_id.as_deref(), Some("T-0"));
        assert_eq!(model.inspector_section, InspectorSection::Relationships);
        assert_eq!(model.inspector_selected, 0);

        model.inspector_selected = 1;
        model.update(Action::OpenDetail);
        assert_eq!(model.selected_id.as_deref(), Some("T-0"));
        assert!(model.status_line().contains("missing or unloadable"));
        assert_eq!(model.inspector_history.len(), 0);
    }

    /// Proof for `tui.inspectors_use_semantic_information_hierarchy`: every
    /// summary fact retains a typed role, group and exact value while section
    /// links remain identities rather than text parsed by the renderer.
    #[test]
    fn inspector_summary_is_typed_grouped_exact_and_link_complete() {
        let mut row = task_row(
            "T-WIDE\u{754c}",
            "A title with \u{1b}[2J controls and e\u{301} clusters",
        );
        row.status = aplan_app::Status::Blocked;
        row.owner = Some("owner:with:colons".to_string());
        row.tags = vec!["ux".to_string(), "wide\u{754c}".to_string()];
        let mut show = task_inspector(&row);
        show.ready = false;
        show.workable = true;
        show.cut = Some("release:one".to_string());
        show.milestone = Some("M-public".to_string());
        show.paths = vec!["src/a:path/**".to_string()];
        show.traces = vec!["HLD.md#one".to_string()];
        show.findings = vec![aplan_app::Finding {
            code: "APLAN999".to_string(),
            severity: "warning".to_string(),
            message: "exact finding".to_string(),
            fix: None,
            repair: None,
        }];

        let items = inspector_summary(&show);
        assert!(matches!(
            &items[0].presentation,
            InspectorPresentation::Identity { id, title }
                if id == "T-WIDE\u{754c}"
                    && title == "A title with \u{1b}[2J controls and e\u{301} clusters"
        ));
        assert!(items.iter().any(|item| matches!(
            &item.presentation,
            InspectorPresentation::Badge { label, value, tone }
                if label == "Status" && value == "blocked" && *tone == InspectorTone::Warning
        )));
        assert!(items.iter().any(|item| matches!(
            &item.presentation,
            InspectorPresentation::Labelled { label, value }
                if label == "Owner" && value == "owner:with:colons"
        )));
        let groups =
            items
                .iter()
                .filter_map(|item| item.group)
                .fold(Vec::new(), |mut groups, group| {
                    if groups.last().copied() != Some(group) {
                        groups.push(group);
                    }
                    groups
                });
        assert_eq!(
            groups,
            [
                "STATE",
                "RESPONSIBILITY",
                "PLACEMENT",
                "PLANNING",
                "DETAILS"
            ]
        );
        assert_eq!(
            items
                .iter()
                .filter_map(|item| match item.link {
                    Some(InspectorLink::Section(section)) => Some(section),
                    Some(InspectorLink::Task(_)) | None => None,
                })
                .collect::<Vec<_>>(),
            InspectorSection::ALL[1..]
        );
        assert!(items.iter().all(|item| {
            !matches!(item.presentation, InspectorPresentation::Text(_)) || item.group.is_none()
        }));

        let sparse_row = task_row("T-DONE", "finished work");
        let mut sparse = task_inspector(&sparse_row);
        sparse.status = "done".to_string();
        sparse.workable = false;
        sparse.ready = false;
        sparse.owner = None;
        sparse.cut = None;
        sparse.milestone = None;
        sparse.tags.clear();
        sparse.effort_days = None;
        sparse.body = None;
        let sparse = inspector_summary(&sparse)
            .into_iter()
            .map(|item| item.semantic_text())
            .collect::<Vec<_>>();
        for exact in [
            "Status: done",
            "Readiness: not applicable to terminal work or milestones",
            "Owner: none",
            "Cut: none",
            "Milestone: none",
            "Tags: none",
            "Effort: not estimated",
            "Body: absent",
        ] {
            assert!(
                sparse.iter().any(|value| value == exact),
                "missing `{exact}`"
            );
        }
    }

    /// Proof for `tui.work_discovery_is_one_typed_flow` and
    /// `tui.work_discovery_state_is_semantically_exact`: bounded and complete
    /// why answers, preference attribution, due clocks and relation navigation
    /// share one inspector and every loaded hop preserves the exact back-stack
    /// location.
    #[test]
    fn work_discovery_controls_and_links_share_one_inspector_flow() {
        let leaf = graph_task("LEAF", "leaf", &[]);
        let preferred = graph_task("PREFERRED", "preferred", &[]);
        let mut cut = graph_task("CUT", "cut", &["LEAF"]);
        cut.tags.push("milestone".to_string());
        let mut subject = graph_task("SUBJECT", "subject", &["CUT", "GHOST"]);
        subject.prefer_after.push("PREFERRED".to_string());
        let released = graph_task("RELEASED", "released", &["SUBJECT"]);
        let mut parked = graph_task("PARKED", "parked", &[]);
        parked.status = aplan_app::Status::Blocked;
        parked.revisit_at = Some("1970-01-01T00:00:10Z".to_string());
        let mut model = ModelBuilder::default()
            .graph(vec![leaf, preferred, cut, subject, released, parked])
            .build();
        model.update(Action::Route(Route::Tasks));
        let subject_index = model
            .task_browser
            .rows
            .iter()
            .position(|row| row.task.id == "SUBJECT")
            .expect("subject row");
        model.update(Action::SelectIndex(subject_index));
        model.update(Action::OpenDetail);
        model.inspector_section = InspectorSection::Work;

        let bounded = model.inspector_items();
        assert!(
            bounded
                .iter()
                .any(|item| item.semantic_text().contains("Why traversal: cut-bounded"))
        );
        assert!(bounded.iter().any(|item| {
            let text = item.semantic_text();
            text.contains("CUT") && text.contains("folds 1")
        }));
        assert!(bounded.iter().any(|item| {
            item.semantic_text().contains("GHOST")
                && item.semantic_text().contains("undefined blocker")
                && item.link.is_none()
        }));
        assert!(bounded.iter().any(|item| {
            item.semantic_text()
                .contains("Prefers after (declared; this task is not currently offered): PREFERRED")
                && item.link == Some(InspectorLink::Task("PREFERRED".to_string()))
        }));
        let cut_link = bounded
            .iter()
            .position(|item| item.link == Some(InspectorLink::Task("CUT".to_string())))
            .expect("cut is navigable");
        let released_link = bounded
            .iter()
            .position(|item| item.link == Some(InspectorLink::Task("RELEASED".to_string())))
            .expect("released task is navigable");
        assert_ne!(cut_link, released_link);
        model.inspector_selected = cut_link;
        model.update(Action::OpenDetail);
        assert_eq!(model.selected_id.as_deref(), Some("CUT"));
        assert!(model.inspector_has_history());
        model.update(Action::CloseLayer);
        assert_eq!(model.selected_id.as_deref(), Some("SUBJECT"));
        assert_eq!(model.inspector_section, InspectorSection::Work);
        assert_eq!(model.inspector_selected, cut_link);

        model.update(Action::ToggleWhyTraversal);
        assert_eq!(model.why_traversal, WhyTraversal::Complete);
        assert!(model.inspector_items().iter().any(|item| {
            item.link == Some(InspectorLink::Task("LEAF".to_string()))
                && !item.semantic_text().contains("folds")
        }));

        model.selected_id = Some("PARKED".to_string());
        model.inspector_section = InspectorSection::Work;
        model.inspector_selected = 0;
        model.update(Action::OpenWorkDate);
        model.update(Action::SearchInput("not-a-date".to_string()));
        model.update(Action::SubmitWorkDate);
        assert_eq!(model.modal, Some(Modal::WorkDate));
        assert_eq!(model.effective_work_as_of_epoch_seconds(), 0);
        model.update(Action::SearchClear);
        model.update(Action::SearchInput("1970-01-01T00:00:10Z".to_string()));
        model.update(Action::SubmitWorkDate);
        assert_eq!(model.modal, None);
        assert_eq!(model.effective_work_as_of_epoch_seconds(), 10);
        assert_eq!(
            model
                .work_revisits()
                .due
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["PARKED"]
        );
        assert!(model.inspector_items().iter().any(|item| {
            item.semantic_text().contains("Revisit: DUE")
                && item
                    .semantic_text()
                    .contains("condition satisfied: not asserted")
        }));
    }

    /// Proof for `tui.work_discovery_state_is_semantically_exact`: a blank
    /// clock follows the identified snapshot across refresh, while an explicit
    /// clock remains pinned to the user's chosen instant.
    #[test]
    fn blank_work_clock_tracks_refresh_while_explicit_clock_stays_pinned() {
        let mut initial = snapshot("initial", 1);
        initial.overview.as_of_epoch_seconds = 10;
        let mut model = Model::new(initial);

        model.update(Action::OpenWorkDate);
        model.update(Action::SearchInput(" \t ".to_string()));
        model.update(Action::SubmitWorkDate);
        assert_eq!(model.work_as_of, "");
        assert_eq!(model.work_as_of_draft, "");
        assert_eq!(model.effective_work_as_of_epoch_seconds(), 10);

        let request = model
            .update(Action::Refresh)
            .effect
            .expect("refresh request");
        let mut refreshed = snapshot("refreshed", 1);
        refreshed.overview.as_of_epoch_seconds = 20;
        model.update(completion(request, Ok(EffectOutput::snapshot(refreshed))));
        assert_eq!(model.effective_work_as_of_epoch_seconds(), 20);

        model.update(Action::OpenWorkDate);
        model.update(Action::SearchInput("7".to_string()));
        model.update(Action::SubmitWorkDate);
        assert_eq!(model.effective_work_as_of_epoch_seconds(), 7);

        let request = model
            .update(Action::Refresh)
            .effect
            .expect("refresh request");
        let mut refreshed = snapshot("refreshed-again", 1);
        refreshed.overview.as_of_epoch_seconds = 30;
        model.update(completion(request, Ok(EffectOutput::snapshot(refreshed))));
        assert_eq!(model.effective_work_as_of_epoch_seconds(), 7);
    }

    #[test]
    fn every_effect_kind_has_a_stable_label() {
        assert_eq!(
            EffectKind::ALL.map(EffectKind::label),
            [
                "startup",
                "external-change probe",
                "refresh",
                "onboarding retry",
                "verify",
                "mutation",
                "configured test catalogue"
            ]
        );
    }

    /// Proof for `tui.startup_feedback_precedes_plan_io`: startup progress is
    /// accepted only for its exact generation, and one typed completion
    /// atomically replaces the inert model while retaining terminal state.
    #[test]
    fn startup_progress_is_generation_safe_and_adopts_one_snapshot() {
        let build = BuildIdentity::default();
        let capabilities = aplan_app::BuildFacts::default().capability_view(&build);
        let (mut model, request) = Model::loading(&build, capabilities, Appearance::default());
        model.update(Action::Resize {
            columns: 67,
            rows: 19,
        });
        model.update(Action::FocusLost);
        model.update(Action::EffectStarted(request.id));

        let event = |operation, sequence, lifecycle, subject| aplan_app::ProgressEvent {
            operation: aplan_app::OperationId::new(operation),
            sequence,
            kind: aplan_app::OperationKind::PlanLoad,
            subject,
            lifecycle,
            measure: aplan_app::ProgressMeasure::Indeterminate,
        };
        model.update(Action::EffectProgress {
            id: request.id,
            progress: event(
                request.id.0 + 1,
                0,
                aplan_app::ProgressLifecycle::Queued,
                aplan_app::ProgressSubject::Operation,
            ),
        });
        assert!(
            model
                .startup
                .as_ref()
                .is_some_and(|startup| startup.latest.is_none()),
            "a cross-generation event must be inert"
        );
        for progress in [
            event(
                request.id.0,
                0,
                aplan_app::ProgressLifecycle::Queued,
                aplan_app::ProgressSubject::Operation,
            ),
            event(
                request.id.0,
                1,
                aplan_app::ProgressLifecycle::Started,
                aplan_app::ProgressSubject::Operation,
            ),
            event(
                request.id.0,
                2,
                aplan_app::ProgressLifecycle::Started,
                aplan_app::ProgressSubject::Phase {
                    phase: aplan_app::ProgressPhase::ConfigurationDiscovery,
                    child: None,
                },
            ),
        ] {
            model.update(Action::EffectProgress {
                id: request.id,
                progress,
            });
        }
        assert!(model.startup.as_ref().is_some_and(|startup| {
            matches!(
                startup.latest,
                Some(aplan_app::ProgressEvent {
                    subject: aplan_app::ProgressSubject::Phase {
                        phase: aplan_app::ProgressPhase::ConfigurationDiscovery,
                        ..
                    },
                    ..
                })
            )
        }));

        let replacement = snapshot("loaded", 2);
        let replacement_capabilities =
            aplan_app::BuildFacts::default().capability_view(&replacement.build);
        model.update(completion(
            request,
            Ok(EffectOutput::Startup(Box::new(StartupReady {
                snapshot: replacement,
                onboarding: None,
                capabilities: replacement_capabilities,
                resolution: None,
            }))),
        ));
        assert!(model.startup.is_none());
        assert_eq!(model.snapshot.overview.input.sha256, "loaded");
        assert_eq!((model.columns, model.rows), (67, 19));
        assert!(!model.active);
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        assert_eq!(model.pending_effects(), 0);
    }

    /// Proof for `tui.startup_feedback_precedes_plan_io`: a fatal worker
    /// result requests exit and retains one diagnostic for the caller only.
    #[test]
    fn startup_failure_exits_through_the_post_restoration_error_path() {
        let build = BuildIdentity::default();
        let capabilities = aplan_app::BuildFacts::default().capability_view(&build);
        let (mut model, request) = Model::loading(&build, capabilities, Appearance::default());
        model.update(Action::EffectStarted(request.id));
        model.update(completion(
            request,
            Err(EffectFailure::Repository(
                "selected input failed".to_string(),
            )),
        ));
        assert!(model.should_quit());
        assert_eq!(
            model.take_startup_failure().as_deref(),
            Some("repository operation failed: selected input failed")
        );
        assert!(model.take_startup_failure().is_none());
    }

    /// Proof for `tui.startup_feedback_precedes_plan_io`: startup work which
    /// crossed an observation gap is adopted whole but cannot claim freshness.
    #[test]
    fn startup_crossing_a_focus_epoch_schedules_confirmation_after_adoption() {
        let build = BuildIdentity::default();
        let capabilities = aplan_app::BuildFacts::default().capability_view(&build);
        let (mut model, request) = Model::loading(&build, capabilities, Appearance::default());
        model.update(Action::EffectStarted(request.id));
        model.update(Action::FocusLost);
        model.update(Action::FocusGained);
        let replacement = snapshot("loaded-after-gap", 1);
        let replacement_capabilities =
            aplan_app::BuildFacts::default().capability_view(&replacement.build);
        let transition = model.update(completion(
            request,
            Ok(EffectOutput::Startup(Box::new(StartupReady {
                snapshot: replacement,
                onboarding: None,
                capabilities: replacement_capabilities,
                resolution: None,
            }))),
        ));
        assert_eq!(model.freshness, SnapshotFreshness::Unconfirmed);
        assert!(model.active);
        assert_eq!(
            transition.effect.as_ref().map(|effect| effect.kind),
            Some(EffectKind::Probe)
        );
    }

    #[test]
    fn mutation_reviews_are_bounded_scrollable_and_not_dismissible_while_publishing() {
        let mut model = Model::new(snapshot("mutation", 1));
        let request = model
            .update(Action::OpenMutation(MutationAction::Add))
            .effect
            .expect("prepare request");
        let rows = (0..30)
            .map(|index| format!("review row {index:02} with a deliberately long value"))
            .collect::<Vec<_>>();
        let layer = model.mutation.as_mut().expect("mutation layer");
        layer.state = MutationState::Result { form: None, rows };
        let top = layer.rows(18, 6);
        assert_eq!(top.len(), 6);
        assert!(top.last().is_some_and(|row| row.starts_with("Rows ")));

        model.update(Action::MutationScroll(5));
        let scrolled = model.mutation.as_ref().expect("layer").rows(18, 6);
        assert_ne!(scrolled, top);
        assert!(scrolled.last().is_some_and(|row| row.starts_with("Rows ")));

        model.mutation.as_mut().expect("layer").committing = true;
        model.update(Action::CloseLayer);
        assert!(model.mutation.is_some(), "a guarded write was dismissible");
        assert!(model.status_line().contains("write is still running"));
        let scroll = model.mutation.as_ref().expect("layer").plain_scroll;
        model.update(Action::MutationScroll(5));
        assert_eq!(
            model.mutation.as_ref().expect("layer").plain_scroll,
            scroll,
            "publishing state accepted navigation"
        );

        model.mutation.as_mut().expect("layer").committing = false;
        let transition = model.update(Action::CloseLayer);
        assert!(model.mutation.is_none());
        assert_eq!(transition.cancel, Some(request.id));
        model.accept_cancel(request.id, true);
    }

    #[test]
    fn a_refusal_for_another_form_request_is_rejected_as_a_corrupt_worker_handoff() {
        use crate::form::FieldSpec;

        let mut model = Model::new(snapshot("mutation-refusal", 1));
        let prepare = model
            .update(Action::OpenMutation(MutationAction::Add))
            .effect
            .expect("prepare request");
        let form = MutationForm::new(
            "Add task",
            "mutation-refusal",
            false,
            vec![FieldSpec::new("title", "Title", "required").required()],
        )
        .expect("form");
        model.update(completion(
            prepare,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Form(Box::new(form))),
            ))),
        ));
        model.update(Action::MutationForm(FormAction::Insert(
            "complete title".to_string(),
        )));
        model.update(Action::MutationForm(FormAction::NextFocus));
        model.update(Action::MutationForm(FormAction::NextFocus));
        let preview = model
            .update(Action::MutationForm(FormAction::Review))
            .effect
            .expect("preview request");
        let expected = match preview.mutation.as_ref().expect("mutation payload") {
            MutationEffect::Preview { submission, .. } => submission.request(),
            other => panic!("wrong effect: {other:?}"),
        };
        let id = preview.id;
        model.update(completion(
            preview,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::PreviewFailed {
                    action: MutationAction::Add,
                    request: expected + 1,
                    message: "refusal for a different edit".to_string(),
                },
            ))),
        ));

        assert!(model.status_line().contains("late mutation refusal"));
        let record = model
            .effects
            .iter()
            .find(|record| record.request.id == id)
            .expect("effect record");
        assert_eq!(record.state, EffectState::Failed);
        let MutationState::Form(form) = &model.mutation.as_ref().expect("layer").state else {
            panic!("a corrupt refusal replaced the current form")
        };
        assert!(form.domain_issue().is_none());
    }

    /// Proof for `tui.mutation_layers_do_not_adopt_prior_effects`: dismissing
    /// running preview work cannot create a new layer for its late answer.
    #[test]
    fn a_running_mutation_must_finish_before_another_layer_can_open() {
        let mut model = Model::new(snapshot("one", 1));
        let old = model
            .update(Action::OpenMutation(MutationAction::Add))
            .effect
            .expect("prepare mutation");
        model.update(Action::EffectStarted(old.id));
        model.update(Action::CloseLayer);
        assert!(model.mutation.is_none());

        let refused = model.update(Action::OpenMutation(MutationAction::Add));
        assert!(refused.effect.is_none());
        assert!(model.mutation.is_none());
        assert!(model.status_line().contains("prior mutation operation"));

        let form = MutationForm::new(
            "old add form",
            "one",
            false,
            vec![crate::form::FieldSpec::new("title", "Title", "task title")],
        )
        .expect("form");
        model.update(completion(
            old,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Form(Box::new(form))),
            ))),
        ));
        assert!(model.mutation.is_none(), "old response created a new layer");

        let fresh = model.update(Action::OpenMutation(MutationAction::Add));
        assert!(fresh.effect.is_some());
        assert!(matches!(
            model.mutation.as_ref().map(|layer| &layer.state),
            Some(MutationState::Loading)
        ));
    }

    /// Proof for `tui.work_preview_responses_are_request_bound`: the typed
    /// work preview must belong to the unchanged form, selected task and input
    /// which requested it before it can replace editable state.
    #[test]
    #[allow(clippy::too_many_lines)] // Four handoffs share one live form/request sequence.
    fn work_previews_are_bound_to_form_request_task_and_input() {
        let directory = tempfile::Builder::new()
            .prefix("work-preview-binding")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let repository = directory.path().to_path_buf();
        let plan = repository.join("aplan");
        std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
        for id in ["T-1", "T-2"] {
            std::fs::write(
                plan.join(format!("tasks/{id}.md")),
                format!(
                    "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\npaths: [src/**]\ntags: [cross-cutting]\n---\n"
                ),
            )
            .expect("task");
        }
        std::fs::create_dir_all(repository.join("src")).expect("source");
        std::fs::write(repository.join("src/lib.rs"), "pub fn fixture() {}\n").expect("source");
        git(&repository, &["init", "-q"]);
        git(&repository, &["config", "user.email", "test@example.com"]);
        git(&repository, &["config", "user.name", "Fixture"]);
        git(&repository, &["add", "."]);
        git(&repository, &["commit", "-qm", "fixture"]);

        let (context, initial) = mutation_context_and_snapshot(&plan, &repository);
        let catalogue = aplan_app::work_action_catalogue(&context, "T-1", Some("alex"))
            .expect("claim catalogue");
        let form = crate::quick_actions::form(
            &catalogue,
            crate::quick_actions::QuickActionWorkflow::Claim,
        )
        .expect("claim form definition")
        .expect("claim has a form");
        let mut model = Model::new(initial);
        assert_eq!(model.selected_id.as_deref(), Some("T-1"));

        let prepare = model
            .update(Action::OpenMutation(MutationAction::Claim))
            .effect
            .expect("prepare claim form");
        model.update(completion(
            prepare,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Form(Box::new(form))),
            ))),
        ));
        model.update(Action::MutationForm(FormAction::Insert("alex".to_string())));
        let first = model
            .update(Action::MutationForm(FormAction::Review))
            .effect
            .expect("first claim preview");
        let first_request = match first.mutation.as_ref().expect("mutation") {
            MutationEffect::Preview { submission, .. } => submission.request(),
            other => panic!("wrong first request: {other:?}"),
        };
        let wrong_task = aplan_app::preview_work_action(
            &context,
            &aplan_app::WorkActionRequest {
                task: "T-2".to_string(),
                action: aplan_app::WorkActionKind::Claim,
                as_name: Some("alex".to_string()),
                from: None,
                claimed_at_epoch_seconds: Some(10),
            },
        )
        .expect("other-task preview");
        model.update(completion(
            first,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Preview {
                    request: first_request,
                    candidate: MutationCandidate::Work {
                        preview: Box::new(wrong_task),
                    },
                },
            ))),
        ));
        assert!(model.status_line().contains("another selected task"));
        assert!(matches!(
            model.mutation.as_ref().map(|layer| &layer.state),
            Some(MutationState::Form(_))
        ));

        // Editing invalidates the refused outstanding token and permits a new
        // request. The next handoff names the right task but a different plan.
        model.update(Action::MutationForm(FormAction::NextFocus));
        model.update(Action::MutationForm(FormAction::Insert(
            "-edited".to_string(),
        )));
        let second = model
            .update(Action::MutationForm(FormAction::Review))
            .effect
            .expect("second claim preview");
        let second_request = match second.mutation.as_ref().expect("mutation") {
            MutationEffect::Preview { submission, .. } => submission.request(),
            other => panic!("wrong second request: {other:?}"),
        };
        let task_path = plan.join("tasks/T-1.md");
        let moved_bytes = std::fs::read_to_string(&task_path)
            .expect("task")
            .replace("title: T-1", "title: moved input");
        std::fs::write(&task_path, moved_bytes).expect("move plan input");
        let (moved_context, _) = mutation_context_and_snapshot(&plan, &repository);
        let wrong_input = aplan_app::preview_work_action(
            &moved_context,
            &aplan_app::WorkActionRequest {
                task: "T-1".to_string(),
                action: aplan_app::WorkActionKind::Claim,
                as_name: Some("alex-edited".to_string()),
                from: None,
                claimed_at_epoch_seconds: Some(11),
            },
        )
        .expect("other-input preview");
        model.update(completion(
            second,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Preview {
                    request: second_request,
                    candidate: MutationCandidate::Work {
                        preview: Box::new(wrong_input),
                    },
                },
            ))),
        ));
        assert!(model.status_line().contains("another plan input"));

        // Finally deliver the exact original candidate after another edit.
        // The request token, not just task/input equality, must discard it.
        model.update(Action::MutationForm(FormAction::NextFocus));
        model.update(Action::MutationForm(FormAction::Insert(
            "-again".to_string(),
        )));
        let third = model
            .update(Action::MutationForm(FormAction::Review))
            .effect
            .expect("third claim preview");
        let (third_request, request) = match third.mutation.as_ref().expect("mutation") {
            MutationEffect::Preview { submission, .. } => (
                submission.request(),
                crate::quick_actions::request(
                    &catalogue,
                    crate::quick_actions::QuickActionWorkflow::Claim,
                    Some(submission),
                )
                .expect("exact claim request"),
            ),
            other => panic!("wrong third request: {other:?}"),
        };
        let exact = aplan_app::preview_work_action(&context, &request).expect("exact preview");
        model.update(Action::MutationForm(FormAction::NextFocus));
        model.update(Action::MutationForm(FormAction::Insert(
            "-late".to_string(),
        )));
        model.update(completion(
            third,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Preview {
                    request: third_request,
                    candidate: MutationCandidate::Work {
                        preview: Box::new(exact),
                    },
                },
            ))),
        ));
        assert!(model.status_line().contains("late mutation preview"));
        let MutationState::Form(form) = &model.mutation.as_ref().expect("claim layer").state else {
            panic!("late preview replaced the edited claim form")
        };
        assert_eq!(
            form.fields()[0].values().collect::<Vec<_>>(),
            vec!["alex-edited-again-late"]
        );

        let fourth = model
            .update(Action::MutationForm(FormAction::Review))
            .effect
            .expect("fourth claim preview");
        let (fourth_request, request) = match fourth.mutation.as_ref().expect("mutation") {
            MutationEffect::Preview { submission, .. } => (
                submission.request(),
                crate::quick_actions::request(
                    &catalogue,
                    crate::quick_actions::QuickActionWorkflow::Claim,
                    Some(submission),
                )
                .expect("accepted claim request"),
            ),
            other => panic!("wrong fourth request: {other:?}"),
        };
        let accepted =
            aplan_app::preview_work_action(&context, &request).expect("accepted preview");
        model.update(completion(
            fourth,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Preview {
                    request: fourth_request,
                    candidate: MutationCandidate::Work {
                        preview: Box::new(accepted),
                    },
                },
            ))),
        ));
        let MutationState::QuickReview {
            candidate, rows, ..
        } = &model.mutation.as_ref().expect("claim layer").state
        else {
            panic!("exact claim preview was not accepted")
        };
        assert_eq!(
            candidate.as_deref().map(MutationCandidate::action),
            Some(MutationAction::Claim)
        );
        let rendered = rows.join("\n");
        assert!(rendered.contains("PREVIEW: claim `T-1`"), "{rendered}");
        assert!(
            rendered.contains("After.holder: alex-edited-again-late"),
            "{rendered}"
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // The reducer must retain one ordered review/commit/reload chain.
    fn plan_and_work_mutations_reconcile_only_their_exact_reviewed_candidates() {
        let directory = tempfile::Builder::new()
            .prefix("mutation-reducer")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let repository = directory.path().to_path_buf();
        let plan = repository.join("aplan");
        std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
        std::fs::write(
            plan.join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");
        let (context, initial) = mutation_context_and_snapshot(&plan, &repository);
        assert!(context.plan().load_diagnostics.is_empty());
        let mut model = Model::new(initial);

        let prepare = model
            .update(Action::OpenMutation(MutationAction::TagAdd))
            .effect
            .expect("prepare tag form");
        let form = crate::planning::form(
            &context,
            crate::planning::PlanningWorkflow::TagAdd,
            Some("T-1"),
        )
        .expect("tag form");
        model.update(completion(
            prepare,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Form(Box::new(form))),
            ))),
        ));
        model.update(Action::MutationForm(FormAction::NextFocus));
        model.update(Action::MutationForm(FormAction::Insert(
            "reviewed".to_string(),
        )));
        model.update(Action::MutationForm(FormAction::NextFocus));
        model.update(Action::MutationForm(FormAction::NextFocus));
        let preview_request = model
            .update(Action::MutationForm(FormAction::Review))
            .effect
            .expect("tag preview request");
        let request_id = match preview_request.mutation.as_ref().expect("mutation payload") {
            MutationEffect::Preview { submission, .. } => submission.request(),
            other => panic!("wrong preview request: {other:?}"),
        };
        let preview_effect_id = preview_request.id;
        let preview = aplan_app::preview_planning(
            &context,
            &aplan_app::PlanningRequest::Tag(aplan_app::TagRequest::Add {
                id: "T-1".to_string(),
                tag: "reviewed".to_string(),
            }),
        )
        .expect("tag preview");
        let change = preview.into_change_set();
        model.update(completion(
            preview_request,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Preview {
                    request: request_id,
                    candidate: MutationCandidate::Plan {
                        action: MutationAction::TagAdd,
                        change: Box::new(change),
                        review_rows: vec!["Tag `reviewed` will be added.".to_string()],
                        selection_after_commit: Some("T-1".to_string()),
                    },
                },
            ))),
        ));
        let MutationState::Form(form) = &model.mutation.as_ref().expect("layer").state else {
            panic!("plan preview did not return to its form")
        };
        assert!(form.change_set().is_some());
        model
            .effect_mut(preview_effect_id)
            .expect("preview effect record")
            .state = EffectState::Queued;
        let blocked = model.update(Action::MutationForm(FormAction::PointerConfirm(
            crate::form::Activation::from_key_kind(ratatui::crossterm::event::KeyEventKind::Press),
        )));
        assert!(blocked.effect.is_none());
        assert!(model.status_line().contains("pending work"));
        let MutationState::Form(form) = &model.mutation.as_ref().expect("layer").state else {
            panic!("blocked pointer confirmation replaced the form")
        };
        assert!(
            form.change_set().is_some(),
            "a refused pointer confirmation consumed the reviewed candidate"
        );
        model
            .effect_mut(preview_effect_id)
            .expect("preview effect record")
            .state = EffectState::Complete;
        let commit = model
            .update(Action::MutationForm(FormAction::PointerConfirm(
                crate::form::Activation::from_key_kind(
                    ratatui::crossterm::event::KeyEventKind::Press,
                ),
            )))
            .effect
            .expect("plan commit request");
        let transfer = match commit.mutation.as_ref().expect("mutation payload") {
            MutationEffect::Commit { candidate } => candidate,
            other => panic!("wrong commit request: {other:?}"),
        };
        let Some(MutationPayload::CommitCandidate(MutationCandidate::Plan { change, .. })) =
            transfer.take()
        else {
            panic!("commit did not retain the exact plan candidate")
        };
        let outcome = aplan_app::commit_change_set(&context, *change);
        assert!(matches!(outcome, aplan_app::CommitOutcome::Applied(_)));
        let report = aplan_app::observe_publication(outcome.change_set());
        let (tagged_context, tagged_snapshot) = mutation_context_and_snapshot(&plan, &repository);
        let reload = crate::mutations::MutationReload {
            input_sha256: tagged_context.identity().sha256.clone(),
            snapshot: Box::new(tagged_snapshot),
            publication: Some(report),
        };
        model.update(completion(
            commit,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Committed(MutationCommitResult::Plan {
                    outcome: Box::new(outcome),
                    reload: Ok(Some(reload)),
                    selection_after_commit: Some("T-1".to_string()),
                }),
            ))),
        ));
        assert_eq!(model.selected_id.as_deref(), Some("T-1"));
        assert!(
            std::fs::read_to_string(plan.join("tasks/T-1.md"))
                .expect("tagged task")
                .contains("reviewed")
        );
        model.update(Action::CloseLayer);

        let prepare = model
            .update(Action::OpenMutation(MutationAction::Start))
            .effect
            .expect("prepare start preview");
        let work_preview = aplan_app::preview_work_action(
            &tagged_context,
            &aplan_app::WorkActionRequest::simple("T-1", aplan_app::WorkActionKind::Start),
        )
        .expect("start preview");
        model.update(completion(
            prepare,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Preview(MutationCandidate::Work {
                    preview: Box::new(work_preview),
                })),
            ))),
        ));
        let MutationState::QuickReview { candidate, .. } =
            &model.mutation.as_ref().expect("start layer").state
        else {
            panic!("zero-input start did not become a quick review")
        };
        assert!(candidate.is_some());
        let commit = model
            .update(Action::ConfirmMutation)
            .effect
            .expect("start commit request");
        let transfer = match commit.mutation.as_ref().expect("mutation payload") {
            MutationEffect::Commit { candidate } => candidate,
            other => panic!("wrong work commit request: {other:?}"),
        };
        let Some(MutationPayload::CommitCandidate(MutationCandidate::Work { preview, .. })) =
            transfer.take()
        else {
            panic!("commit did not retain the exact work candidate")
        };
        let outcome = aplan_app::commit_work_action(&tagged_context, *preview);
        let (started_context, started_snapshot) = mutation_context_and_snapshot(&plan, &repository);
        let reload = crate::mutations::MutationReload {
            input_sha256: started_context.identity().sha256.clone(),
            snapshot: Box::new(started_snapshot),
            publication: None,
        };
        model.update(completion(
            commit,
            Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Committed(MutationCommitResult::Work {
                    outcome: Box::new(outcome),
                    reload: Ok(Some(reload)),
                }),
            ))),
        ));
        let MutationState::Result { rows, .. } =
            &model.mutation.as_ref().expect("work result").state
        else {
            panic!("work commit did not retain a visible result")
        };
        assert!(rows.iter().any(|row| row.contains("COMMITTED")), "{rows:?}");
        assert!(
            std::fs::read_to_string(plan.join("tasks/T-1.md"))
                .expect("started task")
                .contains("status: in_progress")
        );
    }

    /// Proof for `tui.onboarding_and_capabilities_are_typed`: onboarding owns
    /// an inert reducer state, retry can remain classified or install one
    /// complete snapshot, and ordinary plan effects cannot escape the gate.
    #[test]
    fn onboarding_retry_is_typed_inert_and_transitions_in_place() {
        let build = BuildIdentity::default();
        let onboarding = Onboarding {
            cause: aplan_app::OnboardingCause::MissingPlan,
            plan_root: PathBuf::from("aplan"),
            root_selection: aplan_app::RootSelection::Default,
            configuration: aplan_app::ConfigurationSelection::None,
            detail: "no plan".to_string(),
            guidance: vec!["repair it".to_string()],
            commands: Vec::new(),
        };
        let capabilities = aplan_app::BuildFacts::default().capability_view(&build);
        let inert = Snapshot::onboarding(&build, &onboarding);
        assert!(matches!(
            inert.overview.input.configuration.content,
            aplan_app::ConfigurationContentIdentity::Absent
        ));
        let mut selected = onboarding.clone();
        selected.configuration = aplan_app::ConfigurationSelection::Explicit {
            path: PathBuf::from("/trusted/policy.yml"),
        };
        assert!(matches!(
            Snapshot::onboarding(&build, &selected)
                .overview
                .input
                .configuration
                .content,
            aplan_app::ConfigurationContentIdentity::NotObserved
        ));
        let mut model = Model::with_launch(
            inert,
            Some(onboarding.clone()),
            capabilities,
            Some(onboarding.resolution()),
            Appearance::default(),
        );

        assert!(!model.should_probe_external());
        assert!(model.update(Action::Refresh).effect.is_none());
        assert!(model.update(Action::Verify).effect.is_none());
        model.update(Action::OpenAbout);
        assert_eq!(model.modal, Some(Modal::About));
        model.update(Action::CloseLayer);

        let first = model
            .update(Action::Retry)
            .effect
            .expect("retry is available during onboarding");
        assert_eq!(first.kind, EffectKind::Retry);
        let mut invalid = onboarding;
        invalid.cause = aplan_app::OnboardingCause::InvalidConfiguration;
        model.update(completion(
            first,
            Ok(EffectOutput::Onboarding(Box::new(invalid.clone()))),
        ));
        assert_eq!(model.onboarding.as_ref(), Some(&invalid));
        assert!(model.status_line().contains("launch issue remains"));

        let second = model
            .update(Action::Retry)
            .effect
            .expect("a classified retry may be retried again");
        let resolution = aplan_app::LaunchResolution {
            plan_root: PathBuf::from("/repo/aplan"),
            root_selection: aplan_app::RootSelection::Explicit,
            configuration: aplan_app::ConfigurationSelection::None,
        };
        model.update(completion(
            second,
            Ok(EffectOutput::LaunchSnapshot {
                snapshot: Box::new(snapshot("loaded", 1)),
                resolution: resolution.clone(),
            }),
        ));
        assert!(model.onboarding.is_none());
        assert_eq!(model.snapshot.overview.input.sha256, "loaded");
        assert_eq!(model.launch_resolution.as_ref(), Some(&resolution));
        assert_eq!(model.route, Route::Overview);
        assert_eq!(model.focus, Focus::Primary);
        assert!(model.should_probe_external());
    }

    /// Proof for `tui.report_hub_preserves_cli_report_semantics` and
    /// `tui.report_discovery_is_literal_and_parameter_safe`: navigation,
    /// parameter validation and drill-down operate on typed report rows while
    /// whole-plan reports refuse to acquire selector semantics.
    #[test]
    fn report_hub_navigation_dates_and_drill_down_are_operable() {
        let mut model = Model::new(snapshot("reports", 2));
        model.update(Action::Route(Route::Reports));
        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("stale".to_string()));
        assert_eq!(model.report_kinds(), vec![ReportKind::Stale]);
        assert_eq!(model.report_kind, ReportKind::Stale);
        model.update(Action::CloseLayer);
        model.update(Action::ClearNarrowing);
        assert_eq!(model.report_kind, ReportKind::Stale);
        model.update(Action::SelectIndex(0));
        model.update(Action::SelectPrevious);
        assert_eq!(model.report_kind, ReportKind::Deprecations);
        model.update(Action::SelectNext);
        assert_eq!(model.report_kind, ReportKind::Roadmap);
        model.update(Action::OpenDetail);
        model.update(Action::SelectNext);
        model.update(Action::SelectPrevious);
        assert_eq!(model.report_selected_row, 0);
        model.update(Action::Focus(Focus::Primary));
        model.task_selector.statuses = vec![aplan_app::Status::Done];
        model.rebuild_report();
        assert_eq!(model.report_kind, ReportKind::Roadmap);
        assert!(model.report_projection.scope.starts_with("whole plan"));
        assert_eq!(model.report_projection.rows.len(), 1);
        model.update(Action::OpenDetail);
        model.update(Action::OpenDetail);
        assert_eq!(model.route, Route::Tasks);
        assert_eq!(model.task_browser.rows.len(), 2);
        assert!(model.task_selector.is_empty());

        model.update(Action::Route(Route::Reports));
        model.task_selector.statuses = vec![aplan_app::Status::Done];
        model.update(Action::SelectIndex(1));
        assert_eq!(model.report_kind, ReportKind::Summary);
        assert!(model.report_projection.rows.is_empty());
        model.update(Action::ClearFilters);
        assert_eq!(model.report_projection.rows.len(), 1);
        model.update(Action::OpenDetail);
        assert_eq!(model.focus, Focus::Detail);
        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("phase 1".to_string()));
        assert_eq!(model.report_rows().len(), 1);
        model.update(Action::SearchInput(" absent".to_string()));
        assert!(model.report_rows().is_empty());
        model.update(Action::SearchClear);
        model.update(Action::CloseLayer);
        model.update(Action::OpenDetail);
        assert_eq!(model.route, Route::Tasks);
        assert_eq!(model.task_browser.rows.len(), 2);
        assert!(model.task_browser.active_filters[0].starts_with("report: Summary"));

        model.update(Action::Route(Route::Reports));
        model.update(Action::SelectIndex(4));
        model.update(Action::OpenDetail);
        model.update(Action::OpenDetail);
        assert!(
            model
                .notice
                .as_ref()
                .is_some_and(|notice| notice.text.contains("no subjects"))
        );
        model.update(Action::Route(Route::Reports));
        model.update(Action::SelectIndex(2));
        model.update(Action::OpenDetail);
        model.update(Action::OpenDetail);
        assert!(
            model
                .notice
                .as_ref()
                .is_some_and(|notice| notice.text.contains("configuration fact"))
        );
        model.update(Action::Route(Route::Reports));
        model.update(Action::SelectIndex(5));
        assert_eq!(model.report_kind, ReportKind::Stale);
        model.update(Action::OpenReportDate);
        model.update(Action::SearchInput("not-a-date".to_string()));
        assert!(matches!(
            model.report_projection.state,
            aplan_app::ReportState::Invalid(_)
        ));
        model.update(Action::SearchClear);
        assert!(matches!(
            model.report_projection.state,
            aplan_app::ReportState::Unavailable(_)
        ));
    }

    #[test]
    fn report_row_search_preserves_the_selected_key_across_index_changes() {
        let mut model = Model::new(snapshot("reports", 3));
        model.update(Action::Route(Route::Reports));
        model.update(Action::Focus(Focus::Detail));
        model.report_projection.rows = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|key| aplan_app::ReportRow {
                key: key.to_string(),
                label: key.to_string(),
                summary: format!("{key} summary"),
                provenance: ReportRowProvenance::Tasks,
                task_ids: Vec::new(),
            })
            .collect();
        model.report_selected_row = 1;

        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("beta".to_string()));
        assert_eq!(model.report_selected_row, 0);
        assert_eq!(model.report_rows()[0].key, "beta");

        model.update(Action::SearchClear);
        assert_eq!(model.report_selected_row, 1);
        assert_eq!(model.report_rows()[model.report_selected_row].key, "beta");
    }
}
