//! The terminal boundary for aplan's interactive interface.
//!
//! Domain meaning stays in `aplan-app`. This crate owns terminal modes,
//! event decoding and Ratatui painting; later TUI work can add state and
//! effects without making terminal cleanup or plan interpretation a second
//! implementation.

pub mod authoring;
pub mod discovery;
pub mod evidence;
pub mod form;
mod graph_navigation;
mod graph_widget;
pub mod inbox;
mod model;
pub mod mutations;
#[cfg(test)]
#[path = "../tests/support/performance.rs"]
mod performance_tests;
pub mod planning;
pub mod quick_actions;
mod shell;
#[cfg(test)]
mod testing;
mod text;
mod worker;

use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal as _};
use std::process::{Child, Command, ExitStatus};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aplan_app::{BuildFacts, BuildIdentity, Onboarding, PlanContext};
use ratatui::crossterm::{
    event::{
        self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event,
    },
    execute,
    style::available_color_count,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use model::{
    Action, Appearance, ColorCapability, EffectFailure, EffectKind, EffectOutput, EffectRequest,
    Glyphs, Model, Snapshot, Theme,
};
use mutations::{
    MutationCandidate, MutationCommitResult, MutationEffect, MutationPayload, MutationReload,
    MutationTransfer,
};
use shell::{decode_event, render};
use worker::{EffectExecutor, RuntimeEvent, Schedule, WorkerRuntime};

/// Render the committed user-facing action reference from the same closed
/// catalogues that drive key dispatch, contextual help and the command palette.
///
/// This is intentionally a generated documentation seam rather than a second
/// public action model. `cargo xtask tui-reference` owns the committed file and
/// the specification gate rejects drift.
#[doc(hidden)]
#[must_use]
pub fn generated_action_reference_markdown() -> String {
    shell::generated_action_reference_markdown()
}

const IDLE_POLL_TIMEOUT: Duration = Duration::from_secs(60);
const ACTIVE_POLL_TIMEOUT: Duration = Duration::from_millis(250);
const EXTERNAL_POLL_INTERVAL: Duration = Duration::from_secs(5);
#[cfg(unix)]
const SIGNAL_POLL_TIMEOUT: Duration = Duration::from_millis(100);

/// A terminal failure with enough context to know whether setup, interaction,
/// or cleanup failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "`aplan tui` needs interactive stdin and stdout terminals (stdin: {stdin}, stdout: \
         {stdout}). Run it in a terminal; use `aplan check`, `aplan ready`, or a machine \
         format for redirected output."
    )]
    NotTerminal { stdin: bool, stdout: bool },
    #[error("could not enable terminal mode `{mode}`: {source}{cleanup}")]
    Initialize {
        mode: &'static str,
        #[source]
        source: io::Error,
        cleanup: CleanupNote,
    },
    #[error("terminal {operation} failed: {source}{cleanup}")]
    Operation {
        operation: &'static str,
        #[source]
        source: io::Error,
        cleanup: CleanupNote,
    },
    #[error("terminal cleanup failed: {0}")]
    Cleanup(String),
    #[error("interactive startup failed: {0}")]
    Startup(String),
    #[cfg(unix)]
    #[error("could not install terminal signal handlers: {0}")]
    SignalSetup(io::Error),
    #[cfg(unix)]
    #[error("could not suspend and resume the terminal process: {0}")]
    Suspend(io::Error),
}

/// Cleanup context appended to a primary terminal error.
#[derive(Debug, Default)]
pub struct CleanupNote(Option<String>);

impl std::fmt::Display for CleanupNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(message) = &self.0 {
            write!(f, "; cleanup also failed: {message}")?;
        }
        Ok(())
    }
}

/// Refuse before acquiring any terminal mode when either interactive stream is
/// redirected. Both are required: input supplies events and output owns the
/// alternate screen.
pub fn require_terminal() -> Result<(), Error> {
    require_terminal_state(io::stdin().is_terminal(), io::stdout().is_terminal())
}

fn require_terminal_state(stdin: bool, stdout: bool) -> Result<(), Error> {
    if stdin && stdout {
        Ok(())
    } else {
        Err(Error::NotTerminal { stdin, stdout })
    }
}

/// Open the foundation TUI for an already resolved application context.
///
/// The caller resolves the initial plan, configuration and trust boundary and
/// supplies the same typed resolution path for explicit refreshes. No rendered
/// CLI output is parsed here.
pub type Reload = Box<
    dyn Fn() -> Result<(PlanContext, aplan_app::VerificationRunSheet), String>
        + Send
        + Sync
        + 'static,
>;

/// Reload only the identified inputs needed to decide whether a snapshot moved.
///
/// This is deliberately separate from [`Reload`]: a five-second observation must
/// not construct a verification run sheet or any rendered projection merely to
/// compare input identities.
/// Resolve the current identified input for an external-change probe.
///
/// The cancellation token is part of this boundary because a periodic probe
/// may still be running when the user quits. Repository enumeration and
/// parsing must then yield before graceful shutdown's transport bound instead
/// of turning an otherwise idle final frame into an uninterruptible wait.
pub type Observe = Box<
    dyn Fn(&aplan_app::CancellationToken) -> Result<PlanContext, String> + Send + Sync + 'static,
>;

pub type Verify = Box<
    dyn Fn(
            &str,
            bool,
            aplan_app::OperationId,
            &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<aplan_app::VerificationWorkbench, aplan_app::VerificationError>
        + Send
        + Sync
        + 'static,
>;

/// Resolve one initial interactive launch after the terminal has painted its
/// first frame. The callback owns host-specific configuration and repository
/// selection; the TUI owns the operation generation, cancellation and frame.
pub type InitialLoad = Box<
    dyn FnOnce(
            &mut aplan_app::ProgressEmitter<'_>,
            &aplan_app::CancellationToken,
        ) -> Result<InitialLaunch, String>
        + Send
        + 'static,
>;

/// The complete host-owned state resolved behind the first interactive frame.
///
/// Build identity belongs here as well as plan input: hashing the running
/// executable before terminal acquisition can be the cold-start bottleneck the
/// loading frame exists to explain.
#[derive(Debug)]
pub struct InitialLaunch {
    pub launch: Launch,
    pub build: BuildIdentity,
    pub facts: BuildFacts,
}

/// One successful launch assembled by the host from an identified context.
#[derive(Debug)]
pub struct LoadedLaunch {
    pub context: PlanContext,
    pub verification_sheet: aplan_app::VerificationRunSheet,
    pub resolution: aplan_app::LaunchResolution,
}

/// The complete typed result of trying to enter the interactive application.
///
/// Expected first-use and repair conditions remain inside the terminal instead
/// of being flattened into an early CLI error. Unexpected host failures still
/// use the callback's `Err` path.
#[derive(Debug)]
pub enum Launch {
    Loaded(Box<LoadedLaunch>),
    Onboarding(Onboarding),
}

/// A requested built-in terminal theme.
///
/// Dark and light are deliberate user choices; aplan never attempts to infer
/// terminal background color. A reduced-color terminal safely falls back to
/// [`ThemeOverride::LowColor`], and `NO_COLOR` always selects monochrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeOverride {
    Dark,
    Light,
    LowColor,
    Monochrome,
}

/// Presentation choices fixed before the terminal session acquires modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppearanceOptions {
    pub theme: Option<ThemeOverride>,
    pub ascii: bool,
    pub reduced_motion: bool,
}

/// Host services and presentation choices owned by one interactive session.
pub struct Session {
    pub appearance: AppearanceOptions,
    pub observe: Observe,
    pub reload: Reload,
    pub retry: Retry,
    pub verify: Verify,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("appearance", &self.appearance)
            .field("observe", &"<callback>")
            .field("reload", &"<callback>")
            .field("retry", &"<callback>")
            .field("verify", &"<callback>")
            .finish()
    }
}

fn resolve_appearance(
    options: AppearanceOptions,
    no_color: bool,
    available_colors: u16,
) -> Appearance {
    let color = if no_color {
        ColorCapability::None
    } else if available_colors < 256 {
        ColorCapability::Basic
    } else {
        ColorCapability::Extended
    };
    let requested = match options.theme {
        Some(ThemeOverride::Dark) => Theme::Dark,
        Some(ThemeOverride::Light) => Theme::Light,
        Some(ThemeOverride::LowColor) | None => Theme::LowColor,
        Some(ThemeOverride::Monochrome) => Theme::Monochrome,
    };
    let theme = match color {
        ColorCapability::None => Theme::Monochrome,
        ColorCapability::Basic if matches!(requested, Theme::Dark | Theme::Light) => {
            Theme::LowColor
        }
        ColorCapability::Basic | ColorCapability::Extended => requested,
    };
    Appearance {
        theme,
        glyphs: if options.ascii {
            Glyphs::Ascii
        } else {
            Glyphs::Unicode
        },
        color,
        reduced_motion: options.reduced_motion,
    }
}

/// Re-run the host's complete launch resolution after the user changes files.
pub type Retry = Box<dyn Fn() -> Result<Launch, String> + Send + Sync + 'static>;

/// A complete immutable TUI projection prepared without touching terminal state.
///
/// Keeping this stage explicit lets hosts measure and prepare the same snapshot
/// that [`run`] will display while the terminal adapter retains ownership of
/// modes, input and background execution.
#[derive(Debug)]
pub struct PreparedSnapshot(Snapshot);

#[must_use]
pub fn prepare_snapshot(
    context: &PlanContext,
    build: &BuildIdentity,
    verification_sheet: aplan_app::VerificationRunSheet,
) -> PreparedSnapshot {
    PreparedSnapshot(Snapshot::from_context(context, build, verification_sheet))
}

pub fn run(
    launch: Launch,
    build: BuildIdentity,
    facts: &BuildFacts,
    session: Session,
) -> Result<(), Error> {
    require_terminal()?;
    let Session {
        appearance: appearance_options,
        observe,
        reload,
        retry,
        verify,
    } = session;
    // `NO_COLOR` is an existence contract, not a Unicode contract. A non-empty
    // platform value must still disable color when it cannot be decoded as
    // UTF-8.
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    let appearance = resolve_appearance(appearance_options, no_color, available_color_count());
    let capabilities = facts.capability_view(&build);
    let (snapshot, onboarding, resolution) = match launch {
        Launch::Loaded(loaded) => (
            prepare_snapshot(&loaded.context, &build, loaded.verification_sheet).0,
            None,
            Some(loaded.resolution),
        ),
        Launch::Onboarding(onboarding) => {
            let resolution = onboarding.resolution();
            (
                Snapshot::onboarding(&build, &onboarding),
                Some(onboarding),
                Some(resolution),
            )
        }
    };
    let executor = Arc::new(SnapshotExecutor {
        initial_load: std::sync::Mutex::new(None),
        observe,
        reload,
        retry,
        verify,
        runtime_build: std::sync::OnceLock::from(RuntimeBuild {
            build,
            facts: facts.clone(),
        }),
    });
    let mut runtime = WorkerRuntime::new(executor);
    let mut model = Model::with_launch(snapshot, onboarding, capabilities, resolution, appearance);
    run_application(&mut model, &mut runtime, None)
}

/// Open the terminal before the host reads configuration or plan inputs, then
/// resolve and project the launch on the bounded worker while the frame remains
/// responsive. `--progress` is intentionally absent: interactive progress is
/// application state, never a stream-presentation choice.
pub fn run_loading(load: InitialLoad, session: Session) -> Result<(), Error> {
    require_terminal()?;
    let Session {
        appearance: appearance_options,
        observe,
        reload,
        retry,
        verify,
    } = session;
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    let appearance = resolve_appearance(appearance_options, no_color, available_color_count());
    let placeholder_build = BuildIdentity::default();
    let placeholder_capabilities = BuildFacts::default().capability_view(&placeholder_build);
    let (mut model, request) =
        Model::loading(&placeholder_build, placeholder_capabilities, appearance);
    let executor = Arc::new(SnapshotExecutor {
        initial_load: std::sync::Mutex::new(Some(load)),
        observe,
        reload,
        retry,
        verify,
        runtime_build: std::sync::OnceLock::new(),
    });
    let mut runtime = WorkerRuntime::new(executor);
    run_application(&mut model, &mut runtime, Some(request))
}

fn run_application(
    model: &mut Model,
    runtime: &mut WorkerRuntime,
    initial_request: Option<EffectRequest>,
) -> Result<(), Error> {
    let mut lifecycle = Lifecycle::new()?;
    let mut initial_request = initial_request;
    loop {
        let session = with_modes(CrosstermModes, || {
            run_screen(model, runtime, &mut lifecycle, &mut initial_request)
        });
        let pending = lifecycle.take_event();
        let exit = match pending {
            Some(LifecycleEvent::Terminate(signal)) => Some(SessionExit::Terminate(signal)),
            Some(LifecycleEvent::Suspend) => Some(SessionExit::Suspend),
            Some(LifecycleEvent::Continue) => session_exit_after_continue(session?),
            None => Some(session?),
        };
        let Some(exit) = exit else {
            // A continue delivered after the screen elected to suspend but
            // before the default stop handler runs cancels that stale stop.
            // The screen was nevertheless torn down, so record the observation
            // gap and request a current-epoch probe before reacquiring it.
            request_resume_observation(model, runtime);
            continue;
        };
        match exit {
            SessionExit::Quit => {
                runtime.cancel_running();
                shutdown_after_terminal_restore(runtime, &mut lifecycle)?;
                if let Some(error) = model.take_startup_failure() {
                    return Err(Error::Startup(error));
                }
                return Ok(());
            }
            SessionExit::Terminate(signal) => Lifecycle::terminate(signal),
            SessionExit::Suspend => {
                // Once terminal ownership is released the process can no
                // longer observe repository changes. Enter the same
                // fail-closed epoch used by an ordinary focus loss before the
                // default handler stops us; SIGCONT will request a fresh
                // observation after modes are reacquired.
                model.update(Action::FocusLost);
                Lifecycle::suspend()?;
            }
            SessionExit::ExternalEditor(initial) => {
                match run_external_editor(&initial, &mut lifecycle)? {
                    EditorOutcome::Edited(body) => model.accept_external_editor_body(body),
                    EditorOutcome::Failed(message) => model.reject_external_editor(message),
                    EditorOutcome::Terminate(signal) => Lifecycle::terminate(signal),
                }
                // The editor ran with no screen ownership. Treat the complete
                // interval like a focus loss even if it changed only the draft:
                // it could also have changed plan, configuration or Git state.
                request_resume_observation(model, runtime);
            }
        }
    }
}

fn session_exit_after_continue(session: SessionExit) -> Option<SessionExit> {
    match session {
        SessionExit::Suspend => None,
        other => Some(other),
    }
}

#[derive(Debug)]
enum EditorOutcome {
    Edited(String),
    Failed(String),
    Terminate(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorCommand {
    program: OsString,
    arguments: Vec<OsString>,
}

fn editor_command() -> Result<EditorCommand, String> {
    editor_command_from(
        std::env::var_os("VISUAL").as_deref(),
        std::env::var_os("EDITOR").as_deref(),
    )
}

fn editor_command_from(
    visual: Option<&OsStr>,
    editor: Option<&OsStr>,
) -> Result<EditorCommand, String> {
    let configured = [visual, editor]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty());
    let Some(configured) = configured else {
        return Ok(EditorCommand {
            #[cfg(windows)]
            program: OsString::from("notepad.exe"),
            #[cfg(not(windows))]
            program: OsString::from("vi"),
            arguments: Vec::new(),
        });
    };
    let configured = configured
        .to_str()
        .ok_or_else(|| "VISUAL/EDITOR is not valid Unicode".to_string())?;
    let mut words = shell_words::split(configured)
        .map_err(|error| format!("VISUAL/EDITOR has invalid quoting: {error}"))?;
    if words.is_empty() {
        return Err("VISUAL/EDITOR contains no executable".to_string());
    }
    Ok(EditorCommand {
        program: OsString::from(words.remove(0)),
        arguments: words.into_iter().map(OsString::from).collect(),
    })
}

fn run_external_editor(
    initial: &str,
    lifecycle: &mut impl LifecycleSource,
) -> Result<EditorOutcome, Error> {
    let command = match editor_command() {
        Ok(command) => command,
        Err(error) => return Ok(EditorOutcome::Failed(error)),
    };
    let handoff = match authoring::ExternalEditorHandoff::prepare(&std::env::temp_dir(), initial) {
        Ok(handoff) => handoff,
        Err(error) => {
            return Ok(EditorOutcome::Failed(format!(
                "could not prepare the private body draft: {error}"
            )));
        }
    };
    let mut child = match Command::new(&command.program)
        .args(&command.arguments)
        .arg(handoff.path())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return Ok(EditorOutcome::Failed(format!(
                "could not start the configured program: {error}"
            )));
        }
    };
    let status = loop {
        match lifecycle.take_event() {
            Some(LifecycleEvent::Terminate(signal)) => {
                terminate_editor(&mut child);
                return Ok(EditorOutcome::Terminate(signal));
            }
            Some(LifecycleEvent::Suspend) => Lifecycle::suspend()?,
            Some(LifecycleEvent::Continue) | None => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::park_timeout(
                lifecycle.poll_cap().unwrap_or(Duration::from_millis(100)),
            ),
            Err(error) => {
                terminate_editor(&mut child);
                return Ok(EditorOutcome::Failed(format!(
                    "could not observe the configured program: {error}"
                )));
            }
        }
    };
    Ok(complete_external_editor(status, handoff))
}

fn terminate_editor(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn complete_external_editor(
    status: ExitStatus,
    handoff: authoring::ExternalEditorHandoff,
) -> EditorOutcome {
    if !status.success() {
        return EditorOutcome::Failed(status.code().map_or_else(
            || "configured program ended by signal".to_string(),
            |code| format!("configured program exited with status {code}"),
        ));
    }
    match handoff.complete() {
        Ok(body) => EditorOutcome::Edited(body),
        Err(error) => EditorOutcome::Failed(format!("could not accept the edited body: {error}")),
    }
}

/// Finish background work after terminal restoration without turning the
/// lifecycle queue into a dead end.
///
/// `q` is intentionally graceful: repository work is joined rather than
/// detached. The join cannot itself be blocking, however, because the signal
/// handlers remain installed until `run` returns. Poll worker completion and
/// lifecycle events together so a catchable termination still takes the same
/// prompt re-raise path after the last terminal frame (`A1-560`). The final
/// check immediately after the worker finishes also gives a signal delivered
/// with its completion precedence over a successful return.
fn shutdown_after_terminal_restore(
    runtime: &mut WorkerRuntime,
    lifecycle: &mut impl LifecycleSource,
) -> Result<(), Error> {
    loop {
        match wait_for_worker_or_lifecycle(runtime, lifecycle) {
            Some(LifecycleEvent::Terminate(signal)) => Lifecycle::terminate(signal),
            Some(LifecycleEvent::Suspend) => Lifecycle::suspend()?,
            Some(LifecycleEvent::Continue) => {}
            None => return Ok(()),
        }
    }
}

fn wait_for_worker_or_lifecycle(
    runtime: &mut WorkerRuntime,
    lifecycle: &mut impl LifecycleSource,
) -> Option<LifecycleEvent> {
    loop {
        if let Some(event) = lifecycle.take_event() {
            return Some(event);
        }
        if runtime.finish_shutdown_if_ready() {
            // Prefer an event already delivered with worker completion over a
            // successful return. Do not add work between this final lifecycle
            // poll and returning to `run`, where `Lifecycle` is the last local
            // created and therefore unregisters its handlers first.
            return lifecycle.take_event();
        }
        std::thread::park_timeout(lifecycle.poll_cap().unwrap_or(ACTIVE_POLL_TIMEOUT));
    }
}

fn capture_change(change: &aplan_app::ChangeSet) -> bool {
    change.effects().iter().any(|effect| {
        matches!(
            effect,
            aplan_app::SemanticEffect::CaptureCreated { .. }
                | aplan_app::SemanticEffect::CaptureAmended { .. }
                | aplan_app::SemanticEffect::CaptureDiscarded { .. }
                | aplan_app::SemanticEffect::CaptureRestored { .. }
                | aplan_app::SemanticEffect::CapturePromoted { .. }
        )
    })
}

fn snapshot_observes_mutation(
    context: &PlanContext,
    capture_digest: Option<&str>,
    change: &aplan_app::ChangeSet,
) -> bool {
    capture_digest.map_or_else(
        || aplan_app::snapshot_observes_publication(context, change),
        |digest| {
            aplan_app::capture_snapshot_observes_publication(
                context,
                digest,
                change,
                &aplan_core::capture::SystemClock,
            )
        },
    )
}

struct SnapshotExecutor {
    initial_load: std::sync::Mutex<Option<InitialLoad>>,
    observe: Observe,
    reload: Reload,
    retry: Retry,
    verify: Verify,
    runtime_build: std::sync::OnceLock<RuntimeBuild>,
}

#[derive(Debug, Clone)]
struct RuntimeBuild {
    build: BuildIdentity,
    facts: BuildFacts,
}

impl SnapshotExecutor {
    fn execute_startup(
        &self,
        request: &EffectRequest,
        cancellation: &aplan_app::CancellationToken,
        progress: &mut dyn FnMut(aplan_app::ProgressEvent),
    ) -> Result<EffectOutput, EffectFailure> {
        let loader = self
            .initial_load
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| {
                EffectFailure::Unavailable("initial loading can run only once".to_string())
            })?;
        let operation = aplan_app::OperationId::new(request.id.0);
        let mut emitter = aplan_app::ProgressEmitter::begin(
            operation,
            aplan_app::OperationKind::PlanLoad,
            progress,
        );
        if cancellation.is_cancelled() {
            emitter.cancel();
            return Err(EffectFailure::Unavailable("startup cancelled".to_string()));
        }
        let initial = match loader(&mut emitter, cancellation) {
            Ok(initial) if !cancellation.is_cancelled() => initial,
            Ok(_) => {
                emitter.cancel();
                return Err(EffectFailure::Unavailable("startup cancelled".to_string()));
            }
            Err(_) if cancellation.is_cancelled() => {
                emitter.cancel();
                return Err(EffectFailure::Unavailable("startup cancelled".to_string()));
            }
            Err(error) => {
                emitter.fail();
                return Err(EffectFailure::Repository(error));
            }
        };
        let runtime_build = RuntimeBuild {
            build: initial.build,
            facts: initial.facts,
        };
        let capabilities = runtime_build.facts.capability_view(&runtime_build.build);
        let projected = match initial.launch {
            Launch::Loaded(ready) => {
                let LoadedLaunch {
                    context,
                    verification_sheet,
                    resolution,
                } = *ready;
                Snapshot::from_context_observed(
                    &context,
                    &runtime_build.build,
                    verification_sheet,
                    &mut emitter,
                    cancellation,
                )
                .map(|snapshot| (snapshot, None, Some(resolution)))
            }
            Launch::Onboarding(onboarding) => {
                let resolution = onboarding.resolution();
                if cancellation.is_cancelled() {
                    emitter.cancel();
                    return Err(EffectFailure::Unavailable("startup cancelled".to_string()));
                }
                emitter.phase_started(
                    aplan_app::ProgressPhase::Projection,
                    None,
                    aplan_app::ProgressMeasure::Indeterminate,
                );
                let snapshot = Snapshot::onboarding(&runtime_build.build, &onboarding);
                if cancellation.is_cancelled() {
                    emitter.cancel();
                    return Err(EffectFailure::Unavailable("startup cancelled".to_string()));
                }
                emitter.phase_completed(
                    aplan_app::ProgressPhase::Projection,
                    None,
                    aplan_app::ProgressMeasure::Indeterminate,
                );
                Some((snapshot, Some(onboarding), Some(resolution)))
            }
        };
        let Some((snapshot, onboarding, resolution)) = projected else {
            emitter.cancel();
            return Err(EffectFailure::Unavailable("startup cancelled".to_string()));
        };
        self.runtime_build.set(runtime_build).map_err(|_| {
            EffectFailure::Unavailable("initial loading can publish only once".to_string())
        })?;
        emitter.complete();
        Ok(EffectOutput::Startup(Box::new(model::StartupReady {
            snapshot,
            onboarding,
            capabilities,
            resolution,
        })))
    }

    fn build(&self) -> Result<&BuildIdentity, EffectFailure> {
        self.runtime_build
            .get()
            .map(|runtime| &runtime.build)
            .ok_or_else(|| EffectFailure::Unavailable("build identity is not loaded".to_string()))
    }

    fn mutation_context(
        &self,
        expected: &str,
    ) -> Result<(PlanContext, aplan_app::VerificationRunSheet), EffectFailure> {
        let (context, sheet) = (self.reload)().map_err(EffectFailure::Repository)?;
        let actual = context.identity().sha256.clone();
        if actual != expected {
            return Err(EffectFailure::Stale {
                expected: expected.to_string(),
                actual,
            });
        }
        Ok((context, sheet))
    }

    fn reload_mutation(
        &self,
        change: Option<&aplan_app::ChangeSet>,
        coordination: Option<(&str, Option<&aplan_core::ClaimRecord>)>,
    ) -> Result<MutationReload, String> {
        let (context, sheet) = (self.reload)()?;
        let expected = context.identity().sha256.clone();
        let snapshot = Snapshot::from_context(
            &context,
            self.build().map_err(|error| error.to_string())?,
            sheet,
        );
        let capture_digest = change
            .filter(|change| capture_change(change))
            .map(|_| {
                snapshot
                    .inbox
                    .as_ref()
                    .map(|workspace| workspace.inbox_sha256.clone())
                    .map_err(|error| format!("reloading the capture inbox: {error}"))
            })
            .transpose()?;
        if let Some(change) = change
            && !snapshot_observes_mutation(&context, capture_digest.as_deref(), change)
        {
            return Err(
                "the fresh application snapshot does not contain every reviewed after-state"
                    .to_string(),
            );
        }
        let confirmed = (self.observe)(&aplan_app::CancellationToken::default())?;
        if confirmed.identity().sha256 != expected {
            return Err(format!(
                "the identified input moved again while rebuilding the snapshot (expected {expected}, loaded {})",
                confirmed.identity().sha256
            ));
        }
        if let Some(change) = change
            && !snapshot_observes_mutation(&confirmed, capture_digest.as_deref(), change)
        {
            return Err(
                "the reviewed after-state changed again while rebuilding the snapshot".to_string(),
            );
        }
        if let Some((task, expected)) = coordination {
            let observed = aplan_app::work_action_catalogue(&confirmed, task, None)
                .map_err(|error| format!("reloading claim state for `{task}`: {error}"))?;
            if observed.claim.as_ref() != expected {
                return Err(format!(
                    "claim state for `{task}` changed again while rebuilding the snapshot"
                ));
            }
        }
        let publication = change.map(aplan_app::observe_publication);
        if publication
            .as_ref()
            .is_some_and(|report| report.state() != aplan_app::PublicationState::Complete)
        {
            return Err(
                "the reviewed files changed again before reload reconciliation completed"
                    .to_string(),
            );
        }
        Ok(MutationReload {
            snapshot: Box::new(snapshot),
            input_sha256: expected,
            publication,
        })
    }

    fn execute_mutation(
        &self,
        request: &EffectRequest,
        effect: &MutationEffect,
    ) -> Result<EffectOutput, EffectFailure> {
        match effect {
            MutationEffect::Prepare {
                action,
                selected_task,
            } => {
                let (context, _) = self.mutation_context(&request.input)?;
                let prepared = mutations::prepare(&context, *action, selected_task.as_deref())
                    .map_err(EffectFailure::Unavailable)?;
                Ok(EffectOutput::Mutation(MutationTransfer::new(
                    MutationPayload::Prepared(prepared),
                )))
            }
            MutationEffect::Preview {
                action,
                selected_task,
                submission,
                verification,
                edited_body,
            } => Ok(self.preview_mutation(
                request,
                *action,
                selected_task.as_deref(),
                submission,
                verification.as_deref(),
                edited_body.as_deref(),
            )),
            MutationEffect::Commit { candidate } => self.commit_mutation(&request.input, candidate),
        }
    }

    fn preview_mutation(
        &self,
        request: &EffectRequest,
        action: mutations::MutationAction,
        selected_task: Option<&str>,
        submission: &crate::form::FormSubmission,
        verification: Option<&aplan_app::VerificationWorkbench>,
        edited_body: Option<&str>,
    ) -> EffectOutput {
        let preview = self
            .mutation_context(&request.input)
            .and_then(|(context, _)| {
                if submission.input_sha256() != request.input {
                    return Err(EffectFailure::Stale {
                        expected: submission.input_sha256().to_string(),
                        actual: request.input.clone(),
                    });
                }
                mutations::preview_with_external_body(
                    &context,
                    action,
                    selected_task,
                    submission,
                    verification,
                    edited_body,
                )
                .map_err(EffectFailure::Unavailable)
            });
        let payload = match preview {
            Ok(candidate) => MutationPayload::Preview {
                request: submission.request(),
                candidate,
            },
            Err(error) => MutationPayload::PreviewFailed {
                action,
                request: submission.request(),
                message: error.to_string(),
            },
        };
        EffectOutput::Mutation(MutationTransfer::new(payload))
    }

    fn commit_mutation(
        &self,
        expected_input: &str,
        transfer: &MutationTransfer,
    ) -> Result<EffectOutput, EffectFailure> {
        let Some(MutationPayload::CommitCandidate(candidate)) = transfer.take() else {
            return Err(EffectFailure::Unavailable(
                "the reviewed mutation was already consumed or had the wrong type".to_string(),
            ));
        };
        let (context, _) = (self.reload)().map_err(EffectFailure::Repository)?;
        let result = if context.identity().sha256 == expected_input {
            match candidate {
                MutationCandidate::Plan {
                    change,
                    selection_after_commit,
                    ..
                } => {
                    let outcome = aplan_app::commit_change_set(&context, *change);
                    let reload = match &outcome {
                        aplan_app::CommitOutcome::Applied(change)
                        | aplan_app::CommitOutcome::PublicationFailed { change, .. } => {
                            self.reload_mutation(Some(change), None).map(Some)
                        }
                        aplan_app::CommitOutcome::NoOp(_)
                        | aplan_app::CommitOutcome::Denied(_)
                        | aplan_app::CommitOutcome::Stale { .. } => Ok(None),
                    };
                    MutationCommitResult::Plan {
                        outcome: Box::new(outcome),
                        reload,
                        selection_after_commit,
                    }
                }
                MutationCandidate::Work { preview, .. } => {
                    let outcome = aplan_app::commit_work_action(&context, *preview);
                    let reload = self.reload_work_mutation(&outcome);
                    MutationCommitResult::Work {
                        outcome: Box::new(outcome),
                        reload,
                    }
                }
            }
        } else {
            Self::stale_mutation(candidate, context.identity().clone())
        };
        Ok(EffectOutput::Mutation(MutationTransfer::new(
            MutationPayload::Committed(result),
        )))
    }

    fn stale_mutation(
        candidate: MutationCandidate,
        actual: aplan_app::InputIdentity,
    ) -> MutationCommitResult {
        match candidate {
            MutationCandidate::Plan {
                change,
                selection_after_commit,
                ..
            } => MutationCommitResult::Plan {
                outcome: Box::new(aplan_app::CommitOutcome::Stale {
                    change: *change,
                    actual,
                }),
                reload: Ok(None),
                selection_after_commit,
            },
            MutationCandidate::Work { preview, .. } => MutationCommitResult::Work {
                outcome: Box::new(aplan_app::WorkActionCommitOutcome::StaleInput {
                    preview: *preview,
                    actual,
                }),
                reload: Ok(None),
            },
        }
    }

    fn reload_work_mutation(
        &self,
        outcome: &aplan_app::WorkActionCommitOutcome,
    ) -> Result<Option<MutationReload>, String> {
        match outcome {
            aplan_app::WorkActionCommitOutcome::Plan { outcome, .. } => match outcome.as_ref() {
                aplan_app::CommitOutcome::Applied(change)
                | aplan_app::CommitOutcome::PublicationFailed { change, .. } => {
                    self.reload_mutation(Some(change), None).map(Some)
                }
                aplan_app::CommitOutcome::NoOp(_)
                | aplan_app::CommitOutcome::Denied(_)
                | aplan_app::CommitOutcome::Stale { .. } => Ok(None),
            },
            aplan_app::WorkActionCommitOutcome::Coordination(
                aplan_app::CoordinationOutcome::Applied(preview),
            ) => {
                let expected = match preview.change() {
                    aplan_app::WorkActionChange::Coordination(change) => change.after.as_ref(),
                    aplan_app::WorkActionChange::Plan(_) => {
                        unreachable!("coordination outcome retains coordination preview")
                    }
                };
                self.reload_mutation(None, Some((preview.request().task.as_str(), expected)))
                    .map(Some)
            }
            aplan_app::WorkActionCommitOutcome::Coordination(
                aplan_app::CoordinationOutcome::NoOp(_)
                | aplan_app::CoordinationOutcome::Stale { .. }
                | aplan_app::CoordinationOutcome::Failed { .. },
            )
            | aplan_app::WorkActionCommitOutcome::StaleInput { .. } => Ok(None),
        }
    }

    fn execute_test_catalogue(
        &self,
        request: &EffectRequest,
    ) -> Result<EffectOutput, EffectFailure> {
        let (context, _) = self.mutation_context(&request.input)?;
        let view = crate::evidence::search_tests(&context, None)
            .map_err(|error| EffectFailure::Process(error.to_string()))?;
        let confirmed = (self.observe)(&aplan_app::CancellationToken::default())
            .map_err(EffectFailure::Repository)?;
        let actual = confirmed.identity().sha256.clone();
        if actual != request.input {
            return Err(EffectFailure::Stale {
                expected: request.input.clone(),
                actual,
            });
        }
        Ok(EffectOutput::TestCatalogue(Box::new(view)))
    }

    fn execute_probe(
        &self,
        cancellation: &aplan_app::CancellationToken,
    ) -> Result<EffectOutput, EffectFailure> {
        let context = (self.observe)(cancellation).map_err(|error| {
            if cancellation.is_cancelled() {
                EffectFailure::Unavailable("external observation cancelled".to_string())
            } else {
                EffectFailure::Repository(error)
            }
        })?;
        if cancellation.is_cancelled() {
            return Err(EffectFailure::Unavailable(
                "external observation cancelled".to_string(),
            ));
        }
        Ok(EffectOutput::Identity(Box::new(context.identity().clone())))
    }

    fn execute_refresh(
        &self,
        request: &EffectRequest,
        cancellation: &aplan_app::CancellationToken,
    ) -> Result<EffectOutput, EffectFailure> {
        let (context, verification_sheet) = (self.reload)().map_err(EffectFailure::Repository)?;
        let expected = context.identity().sha256.clone();
        let snapshot = Snapshot::from_context_with_overrides(
            &context,
            self.build()?,
            &request.check_overrides,
            verification_sheet,
        );
        let confirmed = (self.observe)(cancellation).map_err(EffectFailure::Repository)?;
        let actual = confirmed.identity().sha256.clone();
        if expected != actual {
            return Err(EffectFailure::Stale { expected, actual });
        }
        Ok(EffectOutput::snapshot(snapshot))
    }
}

impl EffectExecutor for SnapshotExecutor {
    fn execute(
        &self,
        request: &EffectRequest,
        progress: &mut dyn FnMut(aplan_app::ProgressEvent),
    ) -> Result<EffectOutput, EffectFailure> {
        self.execute_cancellable(request, &aplan_app::CancellationToken::default(), progress)
    }

    fn execute_cancellable(
        &self,
        request: &EffectRequest,
        cancellation: &aplan_app::CancellationToken,
        progress: &mut dyn FnMut(aplan_app::ProgressEvent),
    ) -> Result<EffectOutput, EffectFailure> {
        match request.kind {
            EffectKind::Startup => self.execute_startup(request, cancellation, progress),
            EffectKind::Probe => self.execute_probe(cancellation),
            EffectKind::Refresh => self.execute_refresh(request, cancellation),
            EffectKind::Retry => match (self.retry)().map_err(EffectFailure::Repository)? {
                Launch::Onboarding(onboarding) => {
                    Ok(EffectOutput::Onboarding(Box::new(onboarding)))
                }
                Launch::Loaded(loaded) => {
                    let expected = loaded.context.identity().sha256.clone();
                    match (self.retry)().map_err(EffectFailure::Repository)? {
                        Launch::Loaded(confirmed)
                            if confirmed.context.identity().sha256 == expected =>
                        {
                            // The second complete load is the one we confirmed. Keep
                            // its projection, verification inputs and provenance
                            // together; equal context identities do not imply that
                            // launch provenance or a run sheet is interchangeable.
                            let snapshot = Snapshot::from_context_with_overrides(
                                &confirmed.context,
                                self.build()?,
                                &request.check_overrides,
                                confirmed.verification_sheet,
                            );
                            Ok(EffectOutput::LaunchSnapshot {
                                snapshot: Box::new(snapshot),
                                resolution: confirmed.resolution,
                            })
                        }
                        Launch::Loaded(confirmed) => Err(EffectFailure::Stale {
                            expected,
                            actual: confirmed.context.identity().sha256.clone(),
                        }),
                        Launch::Onboarding(onboarding) => Err(EffectFailure::Stale {
                            expected,
                            actual: format!("onboarding:{:?}", onboarding.cause),
                        }),
                    }
                }
            },
            EffectKind::Verify => {
                let workbench = (self.verify)(
                    &request.input,
                    request.execute,
                    aplan_app::OperationId::new(request.id.0),
                    progress,
                )
                .map_err(|error| match error {
                    aplan_app::VerificationError::Repository(message) => {
                        EffectFailure::Repository(message)
                    }
                    aplan_app::VerificationError::Provider(message) => {
                        EffectFailure::Process(message)
                    }
                    aplan_app::VerificationError::Unavailable(message) => {
                        EffectFailure::Unavailable(message)
                    }
                    aplan_app::VerificationError::Stale { expected, actual } => {
                        EffectFailure::Stale { expected, actual }
                    }
                })?;
                let (_, confirmed) = (self.reload)().map_err(EffectFailure::Repository)?;
                if confirmed.input_sha256 != request.input {
                    return Err(EffectFailure::Stale {
                        expected: request.input.clone(),
                        actual: confirmed.input_sha256,
                    });
                }
                Ok(EffectOutput::Verification(Box::new(workbench)))
            }
            EffectKind::Mutation => {
                let effect = request.mutation.as_ref().ok_or_else(|| {
                    EffectFailure::Unavailable("mutation effect payload is missing".to_string())
                })?;
                self.execute_mutation(request, effect)
            }
            EffectKind::TestCatalogue => self.execute_test_catalogue(request),
        }
    }
}

fn run_screen(
    model: &mut Model,
    runtime: &mut WorkerRuntime,
    lifecycle: &mut impl LifecycleSource,
    initial_request: &mut Option<EffectRequest>,
) -> Result<SessionExit, BodyError> {
    let backend = CrosstermBackend::new(io::stdout());
    run_screen_with_initial(
        backend,
        &mut CrosstermInput,
        model,
        runtime,
        lifecycle,
        initial_request,
    )
}

#[cfg(test)]
fn run_screen_with<B: ratatui::backend::Backend<Error = io::Error>>(
    backend: B,
    input: &mut impl InputSource,
    model: &mut Model,
    runtime: &mut WorkerRuntime,
    lifecycle: &mut impl LifecycleSource,
) -> Result<SessionExit, BodyError> {
    run_screen_with_initial(backend, input, model, runtime, lifecycle, &mut None)
}

fn run_screen_with_initial<B: ratatui::backend::Backend<Error = io::Error>>(
    backend: B,
    input: &mut impl InputSource,
    model: &mut Model,
    runtime: &mut WorkerRuntime,
    lifecycle: &mut impl LifecycleSource,
    initial_request: &mut Option<EffectRequest>,
) -> Result<SessionExit, BodyError> {
    let mut terminal =
        Terminal::new(backend).map_err(|source| BodyError::new("initialization", source))?;
    let area = terminal
        .size()
        .map_err(|source| BodyError::new("initialization", source))?;
    model.update(Action::Resize {
        columns: area.width,
        rows: area.height,
    });
    if let Some(request) = initial_request.take() {
        // The first frame is committed before the worker can read repository
        // inputs. A fast load may replace it on the next draw, but it cannot
        // make an eligible invocation look hung before work starts.
        terminal
            .draw(|frame| render(frame, model))
            .map_err(|source| BodyError::new("draw", source))?;
        let _ = model.take_dirty();
        let id = request.id;
        match runtime.submit(request) {
            Schedule::Started => dispatch(model, runtime, Action::EffectStarted(id)),
            Schedule::Queued => {}
            Schedule::Full => dispatch(
                model,
                runtime,
                Action::EffectRejected {
                    id,
                    reason: "startup worker queue is full".to_string(),
                },
            ),
        }
    }
    let body = run_loop(&mut terminal, input, model, runtime, lifecycle);
    restore_renderer_cursor(&mut terminal, body)
}

trait InputSource {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
}

struct CrosstermInput;

impl InputSource for CrosstermInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }
}

fn run_loop<B: ratatui::backend::Backend<Error = io::Error>>(
    terminal: &mut Terminal<B>,
    input: &mut impl InputSource,
    model: &mut Model,
    runtime: &mut WorkerRuntime,
    lifecycle: &mut impl LifecycleSource,
) -> Result<SessionExit, BodyError> {
    let now = Instant::now();
    // A prior guarded session can hand an unconfirmed model back to this one
    // when a continue cancels suspension after teardown. Keep that observation
    // due if the bounded worker could not accept the immediate focus probe.
    let mut next_external_probe = if model.freshness == model::SnapshotFreshness::Current {
        now + EXTERNAL_POLL_INTERVAL
    } else {
        now
    };
    loop {
        if let Some(event) = lifecycle.take_event() {
            match event {
                LifecycleEvent::Terminate(signal) => {
                    return Ok(SessionExit::Terminate(signal));
                }
                LifecycleEvent::Suspend => return Ok(SessionExit::Suspend),
                LifecycleEvent::Continue if !model.should_quit() => {
                    // SIGSTOP cannot be caught before the kernel stops us, so
                    // a standalone continue may be the first observable edge
                    // of a suspension. Fail closed even when no SIGTSTP was
                    // seen; duplicate invalidation is avoided while inactive.
                    model.update(Action::FocusLost);
                    regain_focus(model, runtime, Instant::now(), &mut next_external_probe);
                    rebuild_after_resume(terminal, model)?;
                }
                // Continue is advisory once the reducer has accepted quit. In
                // particular it must not reactivate the model and schedule the
                // resume observation that graceful shutdown would then join.
                LifecycleEvent::Continue => {}
            }
        }

        // Draining is also scheduling: when a running effect has completed,
        // `WorkerRuntime::drain` starts the queued effect before returning its
        // events. Observe quit first so shutdown can join only work that was
        // already running and discard work that never started.
        if model.should_quit() {
            runtime.cancel_running();
            return Ok(SessionExit::Quit);
        }
        for event in runtime.drain() {
            match event {
                RuntimeEvent::Started(id) => dispatch(model, runtime, Action::EffectStarted(id)),
                RuntimeEvent::Progress(id, progress) => {
                    dispatch(model, runtime, Action::EffectProgress { id, progress });
                }
                RuntimeEvent::Completed(completion) => {
                    dispatch(model, runtime, Action::EffectCompleted(completion));
                }
            }
        }

        if let Some(action) = external_probe_action(
            model,
            runtime.has_work(),
            Instant::now(),
            &mut next_external_probe,
        ) {
            dispatch(model, runtime, action);
        }

        if model.take_dirty() {
            terminal
                .draw(|frame| render(frame, model))
                .map_err(|source| BodyError::new("draw", source))?;
        }

        let probe_due_in = (model.should_probe_external() && !runtime.has_work())
            .then(|| next_external_probe.saturating_duration_since(Instant::now()));
        let timeout = poll_timeout(model, runtime, lifecycle, probe_due_in);
        match input.poll(timeout) {
            Ok(true) => {
                let event = input
                    .read()
                    .map_err(|source| BodyError::new("input read", source))?;
                if let Some(action) = decode_event(event, model) {
                    if action == Action::FocusGained {
                        regain_focus(model, runtime, Instant::now(), &mut next_external_probe);
                    } else {
                        dispatch(model, runtime, action);
                    }
                    if let Some(initial) = model.take_external_editor_request() {
                        if runtime.has_work() {
                            model.reject_external_editor(
                                "wait for the active repository operation and try again",
                            );
                        } else {
                            // Invalidate retained repository freshness before
                            // terminal ownership is released. The editor may
                            // change both its private draft and repository
                            // inputs while the TUI cannot observe them.
                            model.update(Action::FocusLost);
                            return Ok(SessionExit::ExternalEditor(initial));
                        }
                    }
                }
            }
            Ok(false) if model.needs_tick() => dispatch(model, runtime, Action::Tick),
            Ok(false) => {}
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {
                // A registered signal may interrupt the terminal poll before
                // its self-pipe is observed. The next loop consumes the typed
                // lifecycle event; an unrelated interruption is a harmless
                // retry rather than a false input failure.
            }
            Err(source) => return Err(BodyError::new("input poll", source)),
        }
    }
}

fn external_probe_action(
    model: &mut Model,
    worker_busy: bool,
    now: Instant,
    next_external_probe: &mut Instant,
) -> Option<Action> {
    if !model.active {
        return None;
    }
    // An overdue deadline can meet the current-epoch probe that an older
    // completion scheduled while the worker queue was being drained. That
    // probe is the observation the deadline asked for; start the next interval
    // from now instead of scheduling a redundant successor when it completes.
    if model.has_running_current_epoch_observation() {
        if now >= *next_external_probe {
            *next_external_probe = now + EXTERNAL_POLL_INTERVAL;
        }
        return None;
    }
    if now < *next_external_probe {
        return None;
    }
    if worker_busy || !model.should_probe_external() {
        model.update(Action::ExternalProbeOverdue);
        return None;
    }
    *next_external_probe = now + EXTERNAL_POLL_INTERVAL;
    Some(Action::PollExternal)
}

fn regain_focus(
    model: &mut Model,
    runtime: &mut WorkerRuntime,
    now: Instant,
    next_external_probe: &mut Instant,
) {
    let was_active = model.active;
    dispatch(model, runtime, Action::FocusGained);
    if was_active {
        return;
    }
    // Focus gain asks the reducer for an immediate observation. If another
    // effect owns the bounded worker, leave the deadline due so the event loop
    // retries as soon as capacity returns. If the observation was accepted,
    // the next ordinary poll belongs one interval after this one.
    *next_external_probe = if model.has_running_current_epoch_observation() {
        now + EXTERNAL_POLL_INTERVAL
    } else {
        now
    };
}

fn request_resume_observation(model: &mut Model, runtime: &mut WorkerRuntime) {
    // FocusLost is idempotent while already inactive. A handled SIGTSTP has
    // therefore advanced the epoch before stopping, while a standalone
    // SIGCONT (for example after SIGSTOP) advances it here exactly once.
    model.update(Action::FocusLost);
    dispatch(model, runtime, Action::FocusGained);
}

fn rebuild_after_resume<B: ratatui::backend::Backend<Error = io::Error>>(
    terminal: &mut Terminal<B>,
    model: &mut Model,
) -> Result<(), BodyError> {
    terminal
        .clear()
        .map_err(|source| BodyError::new("resume clear", source))?;
    let area = terminal
        .size()
        .map_err(|source| BodyError::new("resume size", source))?;
    model.update(Action::Resize {
        columns: area.width,
        rows: area.height,
    });
    Ok(())
}

fn dispatch(model: &mut Model, runtime: &mut WorkerRuntime, action: Action) {
    let transition = model.update(action);
    if let Some(id) = transition.cancel {
        let cancelled = runtime.cancel_queued(id);
        model.accept_cancel(id, cancelled);
    }
    let Some(effect) = transition.effect else {
        return;
    };
    match runtime.submit(effect.clone()) {
        Schedule::Started => {
            model.update(Action::EffectStarted(effect.id));
        }
        Schedule::Queued => {}
        Schedule::Full => {
            model.update(Action::EffectRejected {
                id: effect.id,
                reason: "the bounded worker already has one running and one queued effect"
                    .to_string(),
            });
        }
    }
}

fn poll_timeout(
    model: &Model,
    runtime: &WorkerRuntime,
    lifecycle: &impl LifecycleSource,
    probe_due_in: Option<Duration>,
) -> Duration {
    let ordinary = if model.needs_tick() || runtime.has_work() {
        ACTIVE_POLL_TIMEOUT
    } else {
        IDLE_POLL_TIMEOUT
    };
    let ordinary = probe_due_in.map_or(ordinary, |probe| ordinary.min(probe));
    lifecycle
        .poll_cap()
        .map_or(ordinary, |cap| ordinary.min(cap))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionExit {
    Quit,
    Terminate(i32),
    Suspend,
    ExternalEditor(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    all(not(unix), not(test)),
    expect(
        dead_code,
        reason = "non-Unix lifecycle sources cannot produce Unix signal states; tests inject them into the shared reducer"
    )
)]
enum LifecycleEvent {
    Terminate(i32),
    Suspend,
    Continue,
}

trait LifecycleSource {
    fn take_event(&mut self) -> Option<LifecycleEvent>;

    fn poll_cap(&self) -> Option<Duration> {
        None
    }
}

#[cfg(unix)]
struct Lifecycle {
    signals: signal_hook::iterator::Signals,
}

#[cfg(unix)]
impl Lifecycle {
    fn new() -> Result<Self, Error> {
        use signal_hook::consts::signal::{SIGCONT, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP};

        let signals = signal_hook::iterator::Signals::new([
            SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP, SIGCONT,
        ])
        .map_err(Error::SignalSetup)?;
        Ok(Self { signals })
    }

    fn suspend() -> Result<(), Error> {
        signal_hook::low_level::emulate_default_handler(signal_hook::consts::signal::SIGTSTP)
            .map_err(Error::Suspend)
    }

    fn terminate(signal: i32) -> ! {
        // This resets the selected signal to its default disposition and
        // re-raises it, preserving what the parent observes. Known termination
        // signals do not return. Refuse new configured-command spawns and kill
        // active process groups first; joining an uncooperative effect worker
        // would reintroduce the terminal damage this path exists to prevent.
        aplan_app::terminate_verification_commands();
        let _ = signal_hook::low_level::emulate_default_handler(signal);
        signal_hook::low_level::exit(128_i32.saturating_add(signal));
    }
}

#[cfg(unix)]
impl LifecycleSource for Lifecycle {
    fn take_event(&mut self) -> Option<LifecycleEvent> {
        lifecycle_event(self.signals.pending())
    }

    fn poll_cap(&self) -> Option<Duration> {
        Some(SIGNAL_POLL_TIMEOUT)
    }
}

#[cfg(unix)]
fn lifecycle_event(pending: impl IntoIterator<Item = i32>) -> Option<LifecycleEvent> {
    use signal_hook::consts::signal::{SIGCONT, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP};

    let pending = pending
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    for signal in [SIGHUP, SIGINT, SIGQUIT, SIGTERM] {
        if pending.contains(&signal) {
            return Some(LifecycleEvent::Terminate(signal));
        }
    }
    // `Signals::pending` deliberately coalesces signals and returns them in
    // arbitrary order. If stop and continue arrived within one poll window,
    // the latest observable intent is not to leave the process stopped. Do
    // not consume that continue and then suspend forever waiting for another.
    if pending.contains(&SIGCONT) {
        Some(LifecycleEvent::Continue)
    } else if pending.contains(&SIGTSTP) {
        Some(LifecycleEvent::Suspend)
    } else {
        None
    }
}

#[cfg(not(unix))]
#[derive(Debug, Default)]
struct Lifecycle;

#[cfg(not(unix))]
impl Lifecycle {
    fn new() -> Result<Self, Error> {
        Ok(Self)
    }

    fn suspend() -> Result<(), Error> {
        unreachable!("non-Unix platforms do not produce suspend events")
    }

    fn terminate(_signal: i32) -> ! {
        unreachable!("non-Unix platforms do not produce termination events")
    }
}

#[cfg(not(unix))]
impl LifecycleSource for Lifecycle {
    fn take_event(&mut self) -> Option<LifecycleEvent> {
        None
    }
}

#[derive(Debug)]
struct BodyError {
    operation: &'static str,
    source: io::Error,
    cleanup: CleanupNote,
}

impl BodyError {
    fn new(operation: &'static str, source: io::Error) -> Self {
        Self {
            operation,
            source,
            cleanup: CleanupNote::default(),
        }
    }
}

/// Ratatui hides the cursor as part of drawing a frame and records that state
/// so its `Drop` can make a best-effort repair. Restore explicitly first: a
/// cleanup failure is part of this command's result, not a line Ratatui prints
/// while the command otherwise reports success. On failure Ratatui deliberately
/// retries from `Drop`; the primary result remains an error either way.
fn restore_renderer_cursor<B, T>(
    terminal: &mut Terminal<B>,
    body: Result<T, BodyError>,
) -> Result<T, BodyError>
where
    B: ratatui::backend::Backend<Error = io::Error>,
{
    let cursor = terminal.show_cursor();
    match (body, cursor) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(source)) => Err(BodyError::new("cursor restoration", source)),
        (Err(body), Ok(())) => Err(body),
        (Err(mut body), Err(cursor)) => {
            body.cleanup = CleanupNote(Some(format!("cursor: {cursor}")));
            Err(body)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Raw,
    AlternateScreen,
    FocusChange,
    MouseCapture,
    BracketedPaste,
}

impl Mode {
    const ALL: [Self; 5] = [
        Self::Raw,
        Self::AlternateScreen,
        Self::FocusChange,
        Self::MouseCapture,
        Self::BracketedPaste,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::AlternateScreen => "alternate-screen",
            Self::FocusChange => "focus-change",
            Self::MouseCapture => "mouse-capture",
            Self::BracketedPaste => "bracketed-paste",
        }
    }
}

trait ModeControl {
    fn enable(&mut self, mode: Mode) -> io::Result<()>;
    fn disable(&mut self, mode: Mode) -> io::Result<()>;
}

#[derive(Debug)]
struct ModeGuard<C: ModeControl> {
    control: C,
    active: Vec<Mode>,
}

impl<C: ModeControl> ModeGuard<C> {
    fn acquire(control: C) -> Result<Self, Error> {
        let mut guard = Self {
            control,
            active: Vec::with_capacity(Mode::ALL.len()),
        };
        for mode in Mode::ALL {
            // Record the mode before attempting it. A terminal command may
            // write its escape and then fail while flushing; cleanup is safe
            // when it changed nothing and essential when it changed partially.
            guard.active.push(mode);
            if let Err(source) = guard.control.enable(mode) {
                let cleanup = CleanupNote(guard.restore().err());
                return Err(Error::Initialize {
                    mode: mode.name(),
                    source,
                    cleanup,
                });
            }
        }
        Ok(guard)
    }

    fn restore(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        while let Some(mode) = self.active.pop() {
            if let Err(error) = self.control.disable(mode) {
                failures.push(format!("{}: {error}", mode.name()));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join(", "))
        }
    }
}

impl<C: ModeControl> Drop for ModeGuard<C> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn with_modes<C: ModeControl, T>(
    control: C,
    body: impl FnOnce() -> Result<T, BodyError>,
) -> Result<T, Error> {
    let mut guard = ModeGuard::acquire(control)?;
    let body = body();
    let cleanup = guard.restore();
    match (body, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup)) => Err(Error::Cleanup(cleanup)),
        (Err(body), cleanup) => {
            let cleanup = [body.cleanup.0, cleanup.err()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
            Err(Error::Operation {
                operation: body.operation,
                source: body.source,
                cleanup: CleanupNote((!cleanup.is_empty()).then_some(cleanup)),
            })
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CrosstermModes;

impl ModeControl for CrosstermModes {
    fn enable(&mut self, mode: Mode) -> io::Result<()> {
        match mode {
            Mode::Raw => enable_raw_mode(),
            Mode::AlternateScreen => execute!(io::stdout(), EnterAlternateScreen),
            Mode::FocusChange => execute!(io::stdout(), EnableFocusChange),
            Mode::MouseCapture => execute!(io::stdout(), EnableMouseCapture),
            Mode::BracketedPaste => execute!(io::stdout(), EnableBracketedPaste),
        }
    }

    fn disable(&mut self, mode: Mode) -> io::Result<()> {
        match mode {
            Mode::Raw => disable_raw_mode(),
            Mode::AlternateScreen => execute!(io::stdout(), LeaveAlternateScreen),
            Mode::FocusChange => execute!(io::stdout(), DisableFocusChange),
            Mode::MouseCapture => execute!(io::stdout(), DisableMouseCapture),
            Mode::BracketedPaste => execute!(io::stdout(), DisableBracketedPaste),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::{FormAction, FormFocus, FormTransition};
    use crate::testing::{task_inspector, task_row};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};

    #[test]
    fn editor_command_prefers_visual_parses_arguments_and_never_invokes_a_shell() {
        let visual = OsStr::new("'/opt/Visual Editor/bin/edit' --wait --line 4");
        let editor = OsStr::new("ignored --argument");
        assert_eq!(
            editor_command_from(Some(visual), Some(editor)).expect("configured editor"),
            EditorCommand {
                program: OsString::from("/opt/Visual Editor/bin/edit"),
                arguments: ["--wait", "--line", "4"]
                    .into_iter()
                    .map(OsString::from)
                    .collect(),
            }
        );
        assert!(
            editor_command_from(Some(OsStr::new("'unterminated")), None).is_err(),
            "invalid quoting must fail rather than become shell source"
        );
        assert_eq!(
            editor_command_from(Some(OsStr::new("")), Some(OsStr::new("nano -w")))
                .expect("EDITOR fallback"),
            EditorCommand {
                program: OsString::from("nano"),
                arguments: vec![OsString::from("-w")],
            }
        );
    }

    fn test_runtime_build() -> std::sync::OnceLock<RuntimeBuild> {
        std::sync::OnceLock::from(RuntimeBuild {
            build: BuildIdentity::default(),
            facts: BuildFacts::default(),
        })
    }

    fn mutation_test_context(
        plan: &std::path::Path,
        repository: &std::path::Path,
    ) -> (PlanContext, aplan_app::VerificationRunSheet) {
        let context = PlanContext::new(
            aplan_core::load_plan_identified(plan),
            aplan_app::ContextInputs {
                repository_root: repository.to_path_buf(),
                configuration_file: Some(repository.join(".aplan.yml")),
                trust: aplan_app::Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        );
        let sheet =
            aplan_app::verification_run_sheet(&context, &aplan_evidence::VerifyOptions::default());
        (context, sheet)
    }

    fn configured_test_context(
        plan: &std::path::Path,
        repository: &std::path::Path,
    ) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(plan),
            aplan_app::ContextInputs {
                repository_root: repository.to_path_buf(),
                configuration_file: None,
                trust: aplan_app::Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(aplan_config::Config {
            test: Some(aplan_config::TestRunners::One(aplan_config::TestRunner {
                weight: None,
                exclusive: None,
                name: Some("selector".to_string()),
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
                list: vec!["printf".to_string(), "suite::proof\n".to_string()],
                run: vec!["true".to_string(), "{test}".to_string()],
                batch: None,
                timeout_seconds: None,
            })),
            ..aplan_config::Config::default()
        })
    }

    #[derive(Debug, Default)]
    struct NoLifecycle;

    impl LifecycleSource for NoLifecycle {
        fn take_event(&mut self) -> Option<LifecycleEvent> {
            None
        }
    }

    #[derive(Debug, Default)]
    struct RecordingState {
        events: Vec<(bool, Mode)>,
        fail_enable: Option<Mode>,
        fail_disable: Vec<Mode>,
    }

    #[derive(Debug, Clone)]
    struct RecordingModes(Rc<RefCell<RecordingState>>);

    impl ModeControl for RecordingModes {
        fn enable(&mut self, mode: Mode) -> io::Result<()> {
            if self.0.borrow().fail_enable == Some(mode) {
                return Err(io::Error::other("enable failed"));
            }
            self.0.borrow_mut().events.push((true, mode));
            Ok(())
        }

        fn disable(&mut self, mode: Mode) -> io::Result<()> {
            self.0.borrow_mut().events.push((false, mode));
            if self.0.borrow().fail_disable.contains(&mode) {
                return Err(io::Error::other("disable failed"));
            }
            Ok(())
        }
    }

    fn recorder(state: &Rc<RefCell<RecordingState>>) -> RecordingModes {
        RecordingModes(Rc::clone(state))
    }

    fn expected_cycle() -> Vec<(bool, Mode)> {
        Mode::ALL
            .into_iter()
            .map(|mode| (true, mode))
            .chain(Mode::ALL.into_iter().rev().map(|mode| (false, mode)))
            .collect()
    }

    #[derive(Debug, Default)]
    struct QueuedLifecycle {
        events: std::collections::VecDeque<LifecycleEvent>,
    }

    impl LifecycleSource for QueuedLifecycle {
        fn take_event(&mut self) -> Option<LifecycleEvent> {
            self.events.pop_front()
        }
    }

    #[derive(Debug, Clone)]
    struct SignalOnEnableModes {
        state: Rc<RefCell<RecordingState>>,
        lifecycle: Rc<RefCell<QueuedLifecycle>>,
        at: Mode,
    }

    impl ModeControl for SignalOnEnableModes {
        fn enable(&mut self, mode: Mode) -> io::Result<()> {
            self.state.borrow_mut().events.push((true, mode));
            if mode == self.at {
                self.lifecycle
                    .borrow_mut()
                    .events
                    .push_back(LifecycleEvent::Terminate(15));
            }
            Ok(())
        }

        fn disable(&mut self, mode: Mode) -> io::Result<()> {
            self.state.borrow_mut().events.push((false, mode));
            Ok(())
        }
    }

    #[test]
    fn normal_exit_restores_each_terminal_mode_once_in_reverse_order() {
        let state = Rc::new(RefCell::new(RecordingState::default()));
        with_modes(recorder(&state), || Ok(())).expect("session");
        assert_eq!(state.borrow().events, expected_cycle());
    }

    #[test]
    fn every_partial_initialization_failure_restores_every_mode_that_may_have_changed() {
        for failed in Mode::ALL {
            let state = Rc::new(RefCell::new(RecordingState {
                fail_enable: Some(failed),
                ..RecordingState::default()
            }));
            let error = with_modes(recorder(&state), || Ok(())).expect_err("must fail");
            assert!(error.to_string().contains(failed.name()));

            let attempted: Vec<Mode> = Mode::ALL
                .into_iter()
                .take_while(|mode| *mode != failed)
                .chain(std::iter::once(failed))
                .collect();
            let expected: Vec<(bool, Mode)> = attempted
                .iter()
                .copied()
                .map(|mode| (true, mode))
                // The failing fake records no successful enable event, but its
                // disable is still required because a real write can fail late.
                .filter(|(_, mode)| *mode != failed)
                .chain(attempted.iter().rev().copied().map(|mode| (false, mode)))
                .collect();
            assert_eq!(state.borrow().events, expected, "failed at {failed:?}");
        }
    }

    /// Proof for `tui.terminal_lifecycle_handles_signals`: a signal recorded
    /// while any individual mode is being acquired is consumed only on the
    /// safe main-thread boundary, after which every possibly acquired mode is
    /// restored in reverse order.
    #[test]
    fn a_signal_during_each_partial_acquisition_restores_the_complete_attempt() {
        for at in Mode::ALL {
            let state = Rc::new(RefCell::new(RecordingState::default()));
            let lifecycle = Rc::new(RefCell::new(QueuedLifecycle::default()));
            let control = SignalOnEnableModes {
                state: Rc::clone(&state),
                lifecycle: Rc::clone(&lifecycle),
                at,
            };
            let exit = with_modes(control, || {
                let event = lifecycle.borrow_mut().take_event();
                assert_eq!(event, Some(LifecycleEvent::Terminate(15)));
                Ok(SessionExit::Terminate(15))
            })
            .expect("signal cleanup");
            assert_eq!(exit, SessionExit::Terminate(15));
            assert_eq!(state.borrow().events, expected_cycle(), "signal at {at:?}");
        }
    }

    /// Proof for `tui.terminal_lifecycle_handles_signals`: signal-hook
    /// coalesces a burst, so a continue in the same batch must cancel a stale
    /// suspend while termination remains dominant.
    #[cfg(unix)]
    #[test]
    fn coalesced_continue_cannot_leave_the_process_suspended_forever() {
        use signal_hook::consts::signal::{SIGCONT, SIGTERM, SIGTSTP};

        assert_eq!(
            lifecycle_event([SIGTSTP, SIGCONT]),
            Some(LifecycleEvent::Continue)
        );
        assert_eq!(
            lifecycle_event([SIGCONT, SIGTSTP]),
            Some(LifecycleEvent::Continue)
        );
        assert_eq!(
            lifecycle_event([SIGCONT, SIGTERM, SIGTSTP]),
            Some(LifecycleEvent::Terminate(SIGTERM))
        );
    }

    /// Proof for `tui.continue_cancels_pending_suspend_handoff`: a continue
    /// delivered after the screen has returned cancels only its stale suspend
    /// decision; ordinary completion and termination retain their meaning.
    #[test]
    fn continue_after_screen_exit_cancels_only_a_pending_suspend() {
        assert_eq!(session_exit_after_continue(SessionExit::Suspend), None);
        assert_eq!(
            session_exit_after_continue(SessionExit::Quit),
            Some(SessionExit::Quit)
        );
        assert_eq!(
            session_exit_after_continue(SessionExit::Terminate(15)),
            Some(SessionExit::Terminate(15))
        );
    }

    #[test]
    fn a_draw_failure_is_kept_and_every_mode_is_still_restored() {
        let state = Rc::new(RefCell::new(RecordingState::default()));
        let error = with_modes(recorder(&state), || {
            Err::<(), _>(BodyError {
                operation: "draw",
                source: io::Error::other("draw failed"),
                cleanup: CleanupNote::default(),
            })
        })
        .expect_err("draw must fail");
        assert!(error.to_string().contains("draw failed"));
        assert_eq!(state.borrow().events, expected_cycle());
    }

    #[test]
    fn cleanup_attempts_every_mode_and_reports_both_failures() {
        let state = Rc::new(RefCell::new(RecordingState {
            fail_disable: vec![Mode::MouseCapture],
            ..RecordingState::default()
        }));
        let error = with_modes(recorder(&state), || {
            Err::<(), _>(BodyError {
                operation: "input",
                source: io::Error::other("read failed"),
                cleanup: CleanupNote::default(),
            })
        })
        .expect_err("input and cleanup fail");
        let message = error.to_string();
        assert!(message.contains("read failed") && message.contains("mouse-capture"));
        assert_eq!(state.borrow().events, expected_cycle());
    }

    #[test]
    fn every_cleanup_failure_is_reported_without_short_circuiting_restoration() {
        let state = Rc::new(RefCell::new(RecordingState {
            fail_disable: vec![Mode::MouseCapture, Mode::Raw],
            ..RecordingState::default()
        }));
        let error = with_modes(recorder(&state), || Ok(())).expect_err("cleanup must fail");
        let message = error.to_string();
        assert!(message.contains("mouse-capture") && message.contains("raw"));
        assert_eq!(state.borrow().events, expected_cycle());
    }

    #[derive(Debug, Default)]
    struct CursorState {
        draws: usize,
        hides: usize,
        shows: usize,
        size_calls: usize,
        fail_size_call: Option<usize>,
        fail_draw: bool,
        fail_hide: bool,
        fail_flush: bool,
        show_failures_left: usize,
    }

    #[derive(Debug)]
    struct CursorBackend {
        state: Rc<RefCell<CursorState>>,
        size: ratatui::layout::Size,
        draw_observer: Option<Arc<AtomicUsize>>,
    }

    impl ratatui::backend::Backend for CursorBackend {
        type Error = io::Error;

        fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
        where
            I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
        {
            let mut state = self.state.borrow_mut();
            state.draws += 1;
            if let Some(observer) = &self.draw_observer {
                observer.fetch_add(1, Ordering::SeqCst);
            }
            content.for_each(drop);
            if state.fail_draw {
                Err(io::Error::other("draw failed"))
            } else {
                Ok(())
            }
        }

        fn hide_cursor(&mut self) -> Result<(), Self::Error> {
            let mut state = self.state.borrow_mut();
            state.hides += 1;
            if state.fail_hide {
                Err(io::Error::other("hide failed"))
            } else {
                Ok(())
            }
        }

        fn show_cursor(&mut self) -> Result<(), Self::Error> {
            let mut state = self.state.borrow_mut();
            state.shows += 1;
            if state.show_failures_left > 0 {
                state.show_failures_left -= 1;
                Err(io::Error::other("show failed"))
            } else {
                Ok(())
            }
        }

        fn get_cursor_position(&mut self) -> Result<ratatui::layout::Position, Self::Error> {
            Ok(ratatui::layout::Position::ORIGIN)
        }

        fn set_cursor_position<P: Into<ratatui::layout::Position>>(
            &mut self,
            _position: P,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn clear(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn clear_region(
            &mut self,
            _clear_type: ratatui::backend::ClearType,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn size(&self) -> Result<ratatui::layout::Size, Self::Error> {
            let mut state = self.state.borrow_mut();
            state.size_calls += 1;
            if state.fail_size_call == Some(state.size_calls) {
                Err(io::Error::other("size failed"))
            } else {
                Ok(self.size)
            }
        }

        fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, Self::Error> {
            Ok(ratatui::backend::WindowSize {
                columns_rows: self.size,
                pixels: ratatui::layout::Size::default(),
            })
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            if self.state.borrow().fail_flush {
                Err(io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

    fn cursor_backend(state: &Rc<RefCell<CursorState>>) -> CursorBackend {
        CursorBackend {
            state: Rc::clone(state),
            size: ratatui::layout::Size::new(80, 24),
            draw_observer: None,
        }
    }

    /// `tui.cursor_restoration_is_singular`: Ratatui owns cursor state because
    /// later frames may deliberately show and position it. The outer mode guard
    /// must not fight that lifecycle with a second hide/show pair.
    #[test]
    fn the_renderer_is_the_only_cursor_owner() {
        assert_eq!(
            Mode::ALL,
            [
                Mode::Raw,
                Mode::AlternateScreen,
                Mode::FocusChange,
                Mode::MouseCapture,
                Mode::BracketedPaste,
            ],
            "the surrounding guard must not acquire cursor visibility"
        );
        let state = Rc::new(RefCell::new(CursorState::default()));
        {
            let mut terminal = Terminal::new(cursor_backend(&state)).expect("terminal");
            let body = terminal
                .draw(|_| {})
                .map(|_| ())
                .map_err(|source| BodyError::new("draw", source));
            restore_renderer_cursor(&mut terminal, body).expect("cursor restoration");
        }
        assert_eq!(state.borrow().hides, 1);
        assert_eq!(state.borrow().shows, 1);
    }

    #[test]
    fn a_late_draw_failure_still_restores_the_renderer_owned_cursor() {
        let state = Rc::new(RefCell::new(CursorState {
            fail_flush: true,
            ..CursorState::default()
        }));
        {
            let mut terminal = Terminal::new(cursor_backend(&state)).expect("terminal");
            let body = terminal
                .draw(|_| {})
                .map(|_| ())
                .map_err(|source| BodyError::new("draw", source));
            let error = restore_renderer_cursor(&mut terminal, body).expect_err("flush must fail");
            assert!(error.source.to_string().contains("flush failed"));
        }
        assert_eq!(state.borrow().hides, 1);
        assert_eq!(state.borrow().shows, 1);
    }

    #[test]
    fn a_cursor_cleanup_failure_is_returned_even_when_drop_retries() {
        let state = Rc::new(RefCell::new(CursorState {
            show_failures_left: 1,
            ..CursorState::default()
        }));
        {
            let mut terminal = Terminal::new(cursor_backend(&state)).expect("terminal");
            let body = terminal
                .draw(|_| {})
                .map(|_| ())
                .map_err(|source| BodyError::new("draw", source));
            let error = restore_renderer_cursor(&mut terminal, body).expect_err("show must fail");
            assert_eq!(error.operation, "cursor restoration");
            assert!(error.source.to_string().contains("show failed"));
        }
        assert_eq!(state.borrow().hides, 1);
        assert_eq!(
            state.borrow().shows,
            2,
            "the explicit failure is returned and Ratatui's Drop makes one defensive retry"
        );
    }

    #[test]
    fn primary_cursor_and_mode_failures_are_all_reported_and_restored() {
        let modes = Rc::new(RefCell::new(RecordingState {
            fail_disable: vec![Mode::Raw],
            ..RecordingState::default()
        }));
        let cursor = Rc::new(RefCell::new(CursorState {
            fail_flush: true,
            show_failures_left: 1,
            ..CursorState::default()
        }));
        let error = with_modes(recorder(&modes), || {
            let mut terminal = Terminal::new(cursor_backend(&cursor)).expect("terminal");
            let body = terminal
                .draw(|_| {})
                .map(|_| ())
                .map_err(|source| BodyError::new("draw", source));
            restore_renderer_cursor(&mut terminal, body)
        })
        .expect_err("all three boundaries must fail");

        let message = error.to_string();
        assert!(message.contains("flush failed"), "{message}");
        assert!(message.contains("cursor: show failed"), "{message}");
        assert!(message.contains("raw: disable failed"), "{message}");
        assert_eq!(modes.borrow().events, expected_cycle());
        assert_eq!(cursor.borrow().hides, 1);
        assert_eq!(cursor.borrow().shows, 2, "Drop makes the repair retry");
    }

    #[test]
    fn unwinding_restores_each_terminal_mode_once() {
        let state = Rc::new(RefCell::new(RecordingState::default()));
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let state = Rc::clone(&state);
            move || {
                let _guard = ModeGuard::acquire(recorder(&state)).expect("guard");
                panic!("fault injection");
            }
        }));
        assert!(unwind.is_err());
        assert_eq!(state.borrow().events, expected_cycle());
    }

    #[test]
    fn unwinding_after_a_frame_restores_the_renderer_cursor_and_every_mode() {
        let modes = Rc::new(RefCell::new(RecordingState::default()));
        let cursor = Rc::new(RefCell::new(CursorState::default()));
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let modes = Rc::clone(&modes);
            let cursor = Rc::clone(&cursor);
            move || {
                let _: Result<(), Error> = with_modes(recorder(&modes), || {
                    let mut terminal =
                        Terminal::new(cursor_backend(&cursor)).expect("terminal initialization");
                    terminal.draw(|_| {}).expect("first frame");
                    panic!("fault injection after first frame");
                });
            }
        }));

        assert!(unwind.is_err());
        assert_eq!(modes.borrow().events, expected_cycle());
        assert_eq!(cursor.borrow().hides, 1);
        assert_eq!(
            cursor.borrow().shows,
            1,
            "Ratatui's terminal guard repairs its renderer-owned cursor while unwinding"
        );
    }

    #[test]
    fn a_non_terminal_names_each_stream_and_acquires_nothing() {
        assert!(require_terminal_state(true, true).is_ok());
        for (stdin, stdout) in [(false, true), (true, false), (false, false)] {
            let error = require_terminal_state(stdin, stdout).expect_err("must refuse");
            let message = error.to_string();
            assert!(message.contains(&format!("stdin: {stdin}")));
            assert!(message.contains(&format!("stdout: {stdout}")));
        }
    }

    /// Proof for `tui.visual_language_is_semantic_and_deterministic`: startup
    /// appearance has one precedence order and never infers a background.
    #[test]
    fn appearance_policy_is_deterministic_across_terminal_capabilities() {
        let requested_light = AppearanceOptions {
            theme: Some(ThemeOverride::Light),
            ascii: true,
            reduced_motion: true,
        };
        let no_color = resolve_appearance(requested_light, true, u16::MAX);
        assert_eq!(no_color.theme, Theme::Monochrome);
        assert_eq!(no_color.color, ColorCapability::None);
        assert_eq!(no_color.glyphs, Glyphs::Ascii);
        assert!(no_color.reduced_motion);

        for requested in [ThemeOverride::Dark, ThemeOverride::Light] {
            let reduced = resolve_appearance(
                AppearanceOptions {
                    theme: Some(requested),
                    ..AppearanceOptions::default()
                },
                false,
                8,
            );
            assert_eq!(reduced.theme, Theme::LowColor);
            assert_eq!(reduced.color, ColorCapability::Basic);
        }
        assert_eq!(
            resolve_appearance(AppearanceOptions::default(), false, 256).theme,
            Theme::LowColor
        );
        assert_eq!(
            resolve_appearance(
                AppearanceOptions {
                    theme: Some(ThemeOverride::Light),
                    ..AppearanceOptions::default()
                },
                false,
                256,
            )
            .theme,
            Theme::Light
        );
        assert_eq!(
            resolve_appearance(
                AppearanceOptions {
                    theme: Some(ThemeOverride::Monochrome),
                    ..AppearanceOptions::default()
                },
                false,
                u16::MAX,
            )
            .theme,
            Theme::Monochrome
        );
    }

    #[test]
    fn session_debug_names_configuration_without_exposing_callbacks() {
        let session = Session {
            appearance: AppearanceOptions {
                theme: Some(ThemeOverride::Dark),
                ascii: true,
                reduced_motion: true,
            },
            observe: Box::new(|_| unreachable!("debug must not invoke observe")),
            reload: Box::new(|| unreachable!("debug must not invoke reload")),
            retry: Box::new(|| unreachable!("debug must not invoke retry")),
            verify: Box::new(|_, _, _, _| unreachable!("debug must not invoke verify")),
        };
        let debug = format!("{session:?}");
        assert!(debug.contains("ThemeOverride::Dark") || debug.contains("theme: Some(Dark)"));
        assert!(debug.contains("ascii: true"));
        assert!(debug.contains("reduced_motion: true"));
        assert_eq!(debug.matches("<callback>").count(), 4);
    }

    #[test]
    fn base_shell_quit_and_back_keys_are_press_only() {
        let model = Model::new(test_snapshot("one", 1));
        assert_eq!(
            decode_event(
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                &model,
            ),
            Some(Action::CloseLayer)
        );
        for event in [
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        ] {
            assert_eq!(decode_event(event, &model), Some(Action::Quit));
        }
        for event in [
            Event::Key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT)),
        ] {
            assert_eq!(decode_event(event, &model), None);
        }
        assert!(matches!(
            decode_event(
                Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)),
                &model,
            ),
            Some(Action::Notify(message)) if message.contains("no queued effect")
        ));
        let mut release = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(decode_event(Event::Key(release), &model), None);
        let mut repeat = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        repeat.kind = KeyEventKind::Repeat;
        assert_eq!(decode_event(Event::Key(repeat), &model), None);
    }

    /// Proof for `tui.event_loop_is_generation_safe`: decoding and state
    /// transition are separate boundaries.
    #[test]
    fn terminal_events_decode_without_updating_the_model() {
        let mut model = Model::new(test_snapshot("one", 2));
        model.update(Action::Resize {
            columns: 80,
            rows: 24,
        });
        let mut down_repeat = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        down_repeat.kind = KeyEventKind::Repeat;
        let mut refresh_repeat = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
        refresh_repeat.kind = KeyEventKind::Repeat;

        assert_eq!(
            decode_event(Event::Key(down_repeat), &model),
            Some(Action::SelectNext)
        );
        assert_eq!(decode_event(Event::Key(refresh_repeat), &model), None);
        assert_eq!(
            decode_event(Event::Resize(0, u16::MAX), &model),
            Some(Action::Resize {
                columns: 0,
                rows: u16::MAX,
            })
        );
        assert_eq!(
            decode_event(Event::Paste("q\n\u{1b}[2J".to_string()), &model),
            Some(Action::Paste("q\n\u{1b}[2J".to_string()))
        );
        assert_eq!(
            decode_event(Event::FocusGained, &model),
            Some(Action::FocusGained)
        );
        assert_eq!(
            decode_event(Event::FocusLost, &model),
            Some(Action::FocusLost)
        );
    }

    /// Proof for `tui.event_loop_is_generation_safe`: Crossterm has one
    /// synchronous input owner and no asynchronous reader can race it.
    #[test]
    fn crossterm_poll_and_read_have_one_owner() {
        let source = include_str!("lib.rs");
        let poll_call = ["event::", "poll("].concat();
        let read_call = ["event::", "read("].concat();
        let async_reader = ["Event", "Stream"].concat();
        assert_eq!(source.matches(&poll_call).count(), 1);
        assert_eq!(source.matches(&read_call).count(), 1);
        assert!(!source.contains(&async_reader));
    }

    /// Proof for `tui.external_refresh_is_identity_safe`: the executor checks
    /// identity again after every projection is built, so a deterministic edit
    /// between its two reads produces a stale result instead of a mixed
    /// replacement snapshot.
    #[test]
    fn refresh_executor_rejects_an_edit_that_races_snapshot_construction() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = directory.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        let task = root.join("tasks/T-1.md");
        std::fs::write(
            &task,
            "---\nid: T-1\ntitle: before\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");

        let reload_calls = Arc::new(AtomicUsize::new(0));
        let reload_count = Arc::clone(&reload_calls);
        let reload_root = root.clone();
        let repository_root = directory.path().to_path_buf();
        let reload: Reload = Box::new(move || {
            reload_count.fetch_add(1, Ordering::SeqCst);
            let context = PlanContext::new(
                aplan_core::load_plan_identified(&reload_root),
                aplan_app::ContextInputs {
                    repository_root: repository_root.clone(),
                    configuration_file: None,
                    trust: aplan_app::Trust::Hermetic,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            );
            let sheet = aplan_app::verification_run_sheet(
                &context,
                &aplan_evidence::VerifyOptions::default(),
            );
            Ok((context, sheet))
        });
        let observe_calls = Arc::new(AtomicUsize::new(0));
        let observe_count = Arc::clone(&observe_calls);
        let observe_root = root;
        let observe_repository_root = directory.path().to_path_buf();
        let observe: Observe = Box::new(move |_| {
            observe_count.fetch_add(1, Ordering::SeqCst);
            let before = std::fs::read_to_string(&task).expect("old task");
            std::fs::write(&task, before.replace("title: before", "title: after"))
                .expect("racing edit");
            Ok(PlanContext::new(
                aplan_core::load_plan_identified(&observe_root),
                aplan_app::ContextInputs {
                    repository_root: observe_repository_root.clone(),
                    configuration_file: None,
                    trust: aplan_app::Trust::Hermetic,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            ))
        });
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(None),
            observe,
            reload,
            retry: Box::new(|| panic!("a refresh must not retry launch")),
            verify: Box::new(|_, _, _, _| {
                Err(aplan_app::VerificationError::Unavailable(
                    "not used".to_string(),
                ))
            }),
            runtime_build: std::sync::OnceLock::from(RuntimeBuild {
                build: BuildIdentity {
                    version: "test".to_string(),
                    source: "test".to_string(),
                    facts_sha256: "facts".to_string(),
                    binary_sha256: "binary".to_string(),
                },
                facts: BuildFacts::default(),
            }),
        };
        let request = EffectRequest {
            id: model::EffectId(1),
            generation: 0,
            focus_epoch: 0,
            input: "before".to_string(),
            kind: EffectKind::Refresh,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: None,
        };

        let error = executor
            .execute(&request, &mut |_| {})
            .expect_err("race must be stale");
        assert!(matches!(error, EffectFailure::Stale { .. }));
        assert_eq!(reload_calls.load(Ordering::SeqCst), 1);
        assert_eq!(observe_calls.load(Ordering::SeqCst), 1);
    }

    /// Proof for `tui.configured_test_selector_is_reachable`. The v0.12
    /// closure audit found that the shared catalogue adapter was
    /// exercised only by direct tests. Drive the production worker branch and
    /// prove that its answer is tied to a re-observed input identity.
    #[test]
    fn configured_test_selector_runs_on_the_worker_and_confirms_its_input() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = directory.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        std::fs::write(
            root.join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");
        let initial = configured_test_context(&root, directory.path());
        let expected = initial.identity().sha256.clone();
        let reload_root = root.clone();
        let reload_repository = directory.path().to_path_buf();
        let observe_root = root;
        let observe_repository = directory.path().to_path_buf();
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(None),
            observe: Box::new(move |_| {
                Ok(configured_test_context(&observe_root, &observe_repository))
            }),
            reload: Box::new(move || {
                let context = configured_test_context(&reload_root, &reload_repository);
                let sheet = aplan_app::verification_run_sheet(
                    &context,
                    &aplan_evidence::VerifyOptions::default(),
                );
                Ok((context, sheet))
            }),
            retry: Box::new(|| panic!("catalogue discovery does not retry launch")),
            verify: Box::new(|_, _, _, _| unreachable!("catalogue discovery does not verify")),
            runtime_build: test_runtime_build(),
        };
        let request = EffectRequest {
            id: model::EffectId(1),
            generation: 0,
            focus_epoch: 0,
            input: expected,
            kind: EffectKind::TestCatalogue,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: None,
        };
        let EffectOutput::TestCatalogue(view) = executor
            .execute(&request, &mut |_| {})
            .expect("configured catalogue")
        else {
            panic!("configured catalogue worker returned the wrong output")
        };
        assert_eq!(
            view.catalogue.state,
            aplan_app::TestCatalogueState::Complete
        );
        assert_eq!(view.choices.len(), 1);
        assert_eq!(view.choices[0].citation, "test:suite::proof");
        assert!(view.choices[0].selectable);
    }

    #[test]
    fn mutation_executor_returns_the_exact_review_as_stale_when_input_moves() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = directory.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        let task = root.join("tasks/T-1.md");
        std::fs::write(
            &task,
            "---\nid: T-1\ntitle: before\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");
        let repository_root = directory.path().to_path_buf();
        let load = |plan_root: &std::path::Path, repository_root: &std::path::Path| {
            PlanContext::new(
                aplan_core::load_plan_identified(plan_root),
                aplan_app::ContextInputs {
                    repository_root: repository_root.to_path_buf(),
                    configuration_file: None,
                    trust: aplan_app::Trust::Hermetic,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            )
        };
        let before = load(&root, &repository_root);
        let expected = before.identity().sha256.clone();
        let change = aplan_app::preview_planning(
            &before,
            &aplan_app::PlanningRequest::Tag(aplan_app::TagRequest::Add {
                id: "T-1".to_string(),
                tag: "reviewed".to_string(),
            }),
        )
        .expect("tag preview")
        .into_change_set();
        std::fs::write(
            &task,
            "---\nid: T-1\ntitle: concurrent\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
        )
        .expect("concurrent edit");

        let reload_root = root.clone();
        let reload_repository = repository_root.clone();
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(None),
            observe: Box::new(|_| panic!("a stale commit does not build a replacement snapshot")),
            reload: Box::new(move || {
                let context = load(&reload_root, &reload_repository);
                let sheet = aplan_app::verification_run_sheet(
                    &context,
                    &aplan_evidence::VerifyOptions::default(),
                );
                Ok((context, sheet))
            }),
            retry: Box::new(|| panic!("mutation does not retry launch")),
            verify: Box::new(|_, _, _, _| unreachable!("mutation does not verify")),
            runtime_build: test_runtime_build(),
        };
        let transfer =
            MutationTransfer::new(MutationPayload::CommitCandidate(MutationCandidate::Plan {
                action: mutations::MutationAction::TagAdd,
                change: Box::new(change),
                review_rows: vec!["reviewed exact tag addition".to_string()],
                selection_after_commit: Some("T-1".to_string()),
            }));
        let request = EffectRequest {
            id: model::EffectId(1),
            generation: 0,
            focus_epoch: 0,
            input: expected,
            kind: EffectKind::Mutation,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: Some(MutationEffect::Commit {
                candidate: transfer,
            }),
        };

        let EffectOutput::Mutation(result) = executor
            .execute(&request, &mut |_| {})
            .expect("typed stale result")
        else {
            panic!("mutation returned the wrong output")
        };
        let Some(MutationPayload::Committed(MutationCommitResult::Plan {
            outcome,
            reload: Ok(None),
            selection_after_commit: Some(selected),
        })) = result.take()
        else {
            panic!("stale commit did not retain its reviewed mutation")
        };
        let aplan_app::CommitOutcome::Stale { change, actual } = *outcome else {
            panic!("commit was not retained as stale")
        };
        assert_eq!(selected, "T-1");
        assert_ne!(actual.sha256, request.input);
        assert_eq!(change.effects().len(), 1);
        let bytes = std::fs::read_to_string(task).expect("task remains readable");
        assert!(bytes.contains("title: concurrent"), "{bytes}");
        assert!(!bytes.contains("reviewed"), "{bytes}");
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One ordered journey proves the exact handoffs cannot mix.
    fn mutation_executor_drives_prepare_preview_publish_reload_and_coordination_end_to_end() {
        let directory = tempfile::Builder::new()
            .prefix("mutation-executor")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let repository = directory.path().to_path_buf();
        let plan = repository.join("aplan");
        std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
        std::fs::write(repository.join(".aplan.yml"), "root: aplan\n").expect("configuration");
        std::fs::write(
            plan.join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: first\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");
        for arguments in [
            &["init", "-q"][..],
            &["config", "user.email", "fixture@example.com"][..],
            &["config", "user.name", "Fixture Actor"][..],
            &["add", "."][..],
            &["commit", "-qm", "fixture"][..],
        ] {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args(arguments)
                .output()
                .expect("git");
            assert!(output.status.success(), "git {arguments:?}: {output:?}");
        }

        let (initial, _) = mutation_test_context(&plan, &repository);
        assert!(initial.plan().load_diagnostics.is_empty());
        let initial_input = initial.identity().sha256.clone();
        let reload_plan = plan.clone();
        let reload_repository = repository.clone();
        let observe_plan = plan.clone();
        let observe_repository = repository.clone();
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(None),
            reload: Box::new(move || Ok(mutation_test_context(&reload_plan, &reload_repository))),
            observe: Box::new(move |_| {
                Ok(mutation_test_context(&observe_plan, &observe_repository).0)
            }),
            retry: Box::new(|| panic!("mutation does not retry launch")),
            verify: Box::new(|_, _, _, _| unreachable!("mutation does not verify")),
            runtime_build: test_runtime_build(),
        };
        let request = |id, input: String, mutation| EffectRequest {
            id: model::EffectId(id),
            generation: 0,
            focus_epoch: 0,
            input,
            kind: EffectKind::Mutation,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: Some(mutation),
        };

        let prepared = executor
            .execute(
                &request(
                    1,
                    initial_input.clone(),
                    MutationEffect::Prepare {
                        action: mutations::MutationAction::Add,
                        selected_task: None,
                    },
                ),
                &mut |_| {},
            )
            .expect("prepare");
        let EffectOutput::Mutation(prepared) = prepared else {
            panic!("prepare returned a non-mutation output")
        };
        let Some(MutationPayload::Prepared(mutations::PreparedMutation::Form(mut form))) =
            prepared.take()
        else {
            panic!("prepare did not return the add form")
        };
        for index in 0..form.fields().len() {
            assert_eq!(form.focus(), FormFocus::Field(index));
            match form.fields()[index].spec().key() {
                "title" => {
                    form.reduce(FormAction::Insert("created through worker".to_string()));
                }
                "id" => {
                    form.reduce(FormAction::Insert("T-2".to_string()));
                }
                "phase" => {
                    form.reduce(FormAction::Insert("1".to_string()));
                }
                "tags" => {
                    form.reduce(FormAction::Insert("cross-cutting".to_string()));
                }
                _ => {}
            }
            form.reduce(FormAction::NextFocus);
        }
        let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
            panic!("complete add form did not request preview")
        };
        let previewed = executor
            .execute(
                &request(
                    2,
                    initial_input.clone(),
                    MutationEffect::Preview {
                        action: mutations::MutationAction::Add,
                        selected_task: None,
                        submission,
                        verification: None,
                        edited_body: None,
                    },
                ),
                &mut |_| {},
            )
            .expect("preview");
        let EffectOutput::Mutation(previewed) = previewed else {
            panic!("preview returned a non-mutation output")
        };
        let Some(MutationPayload::Preview {
            request: 1,
            candidate,
        }) = previewed.take()
        else {
            panic!("preview did not retain its request and candidate")
        };
        let committed = executor
            .execute(
                &request(
                    3,
                    initial_input,
                    MutationEffect::Commit {
                        candidate: MutationTransfer::new(MutationPayload::CommitCandidate(
                            candidate,
                        )),
                    },
                ),
                &mut |_| {},
            )
            .expect("commit");
        let EffectOutput::Mutation(committed) = committed else {
            panic!("commit returned a non-mutation output")
        };
        let Some(MutationPayload::Committed(MutationCommitResult::Plan {
            outcome,
            reload: Ok(Some(reload)),
            selection_after_commit: Some(selection),
        })) = committed.take()
        else {
            panic!("plan publication was not reloaded and reconciled")
        };
        assert!(matches!(*outcome, aplan_app::CommitOutcome::Applied(_)));
        assert_eq!(selection, "T-2");
        assert_eq!(
            reload
                .publication
                .as_ref()
                .map(aplan_app::PublicationReport::state),
            Some(aplan_app::PublicationState::Complete)
        );
        assert!(plan.join("tasks/T-2.md").is_file());
        let current_input = reload.input_sha256;

        let claim = executor
            .execute(
                &request(
                    5,
                    current_input.clone(),
                    MutationEffect::Prepare {
                        action: mutations::MutationAction::Claim,
                        selected_task: Some("T-1".to_string()),
                    },
                ),
                &mut |_| {},
            )
            .expect("claim form");
        let EffectOutput::Mutation(claim) = claim else {
            panic!("claim returned a non-mutation output")
        };
        let Some(MutationPayload::Prepared(mutations::PreparedMutation::Form(mut claim))) =
            claim.take()
        else {
            panic!("claim did not return its identity form")
        };
        for _ in 0..claim.fields().len() {
            claim.reduce(FormAction::NextFocus);
        }
        let FormTransition::PreviewRequested(claim_submission) = claim.reduce(FormAction::Review)
        else {
            panic!("claim form did not request preview")
        };
        let commit_submission = claim_submission.clone();
        let claim = executor
            .execute(
                &request(
                    6,
                    current_input.clone(),
                    MutationEffect::Preview {
                        action: mutations::MutationAction::Claim,
                        selected_task: Some("T-1".to_string()),
                        submission: claim_submission,
                        verification: None,
                        edited_body: None,
                    },
                ),
                &mut |_| {},
            )
            .expect("claim preview");
        let EffectOutput::Mutation(claim) = claim else {
            panic!("claim preview returned a non-mutation output")
        };
        let Some(MutationPayload::Preview {
            candidate: claim, ..
        }) = claim.take()
        else {
            panic!("claim preview was not retained")
        };
        let stale = SnapshotExecutor::stale_mutation(claim, initial.identity().clone());
        assert!(matches!(
            stale,
            MutationCommitResult::Work {
                outcome,
                reload: Ok(None)
            } if matches!(*outcome, aplan_app::WorkActionCommitOutcome::StaleInput { .. })
        ));
        let claim = executor
            .execute(
                &request(
                    7,
                    current_input.clone(),
                    MutationEffect::Preview {
                        action: mutations::MutationAction::Claim,
                        selected_task: Some("T-1".to_string()),
                        submission: commit_submission,
                        verification: None,
                        edited_body: None,
                    },
                ),
                &mut |_| {},
            )
            .expect("second claim preview");
        let EffectOutput::Mutation(claim) = claim else {
            panic!("second claim preview returned a non-mutation output")
        };
        let Some(MutationPayload::Preview {
            candidate: claim, ..
        }) = claim.take()
        else {
            panic!("second claim preview was not retained")
        };
        let claim = executor
            .execute(
                &request(
                    8,
                    current_input,
                    MutationEffect::Commit {
                        candidate: MutationTransfer::new(MutationPayload::CommitCandidate(claim)),
                    },
                ),
                &mut |_| {},
            )
            .expect("claim commit");
        let EffectOutput::Mutation(claim) = claim else {
            panic!("claim commit returned a non-mutation output")
        };
        let Some(MutationPayload::Committed(MutationCommitResult::Work {
            outcome,
            reload: Ok(Some(_)),
        })) = claim.take()
        else {
            panic!("coordination publication was not reloaded and reconciled")
        };
        assert!(matches!(
            *outcome,
            aplan_app::WorkActionCommitOutcome::Coordination(
                aplan_app::CoordinationOutcome::Applied(_)
            )
        ));

        let wrong = MutationTransfer::new(MutationPayload::PreviewFailed {
            action: mutations::MutationAction::Add,
            request: 9,
            message: "wrong boundary".to_string(),
        });
        let error = executor
            .commit_mutation("irrelevant", &wrong)
            .expect_err("a non-candidate transfer must fail closed");
        assert!(matches!(error, EffectFailure::Unavailable(_)));
    }

    /// Proof for `show.evidence_observations_are_nonblocking`: verification
    /// checks the identified input after provider work, so a late old answer
    /// cannot replace current inspectors.
    #[test]
    fn verification_executor_rejects_an_edit_that_races_provider_work() {
        let EffectOutput::Verification(workbench) = test_verification("before", 0, 0) else {
            unreachable!("fixture is a verification answer")
        };
        let expected = workbench.sheet.input_sha256.clone();
        let returned = (*workbench).clone();

        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = directory.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        std::fs::write(
            root.join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: changed\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");
        let repository_root = directory.path().to_path_buf();
        let reload: Reload = Box::new(move || {
            let context = PlanContext::new(
                aplan_core::load_plan_identified(&root),
                aplan_app::ContextInputs {
                    repository_root: repository_root.clone(),
                    configuration_file: None,
                    trust: aplan_app::Trust::Hermetic,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            );
            let mut sheet = aplan_app::verification_run_sheet(
                &context,
                &aplan_evidence::VerifyOptions::default(),
            );
            sheet.input_sha256 = "changed-after-provider-work".to_string();
            Ok((context, sheet))
        });
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(None),
            observe: Box::new(|_| panic!("verification does not use the probe observer")),
            reload,
            retry: Box::new(|| panic!("verification does not retry launch")),
            verify: Box::new(move |_, _, _, _| Ok(returned.clone())),
            runtime_build: test_runtime_build(),
        };
        let request = EffectRequest {
            id: model::EffectId(1),
            generation: 0,
            focus_epoch: 0,
            input: expected.clone(),
            kind: EffectKind::Verify,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: None,
        };

        assert!(matches!(
            executor.execute(&request, &mut |_| {}),
            Err(EffectFailure::Stale { expected: stale_expected, actual })
                if stale_expected == expected && actual == "changed-after-provider-work"
        ));
    }

    /// Proof for `tui.onboarding_and_capabilities_are_typed`: a successful
    /// retry is confirmed by a second complete launch and carries that launch's
    /// root/configuration provenance into the replacement snapshot.
    #[test]
    fn retry_executor_confirms_one_typed_launch_snapshot() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = directory.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        std::fs::write(
            root.join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: retry\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");

        let calls = Arc::new(AtomicUsize::new(0));
        let retry_calls = Arc::clone(&calls);
        let retry_root = root.clone();
        let repository_root = directory.path().to_path_buf();
        let retry: Retry = Box::new(move || {
            let call = retry_calls.fetch_add(1, Ordering::SeqCst);
            let context = PlanContext::new(
                aplan_core::load_plan_identified(&retry_root),
                aplan_app::ContextInputs {
                    repository_root: repository_root.clone(),
                    configuration_file: None,
                    trust: aplan_app::Trust::Hermetic,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict::default(),
                },
            );
            let mut verification_sheet = aplan_app::verification_run_sheet(
                &context,
                &aplan_evidence::VerifyOptions::default(),
            );
            if call > 0 {
                verification_sheet.execute = aplan_app::Availability::Available;
            }
            Ok(Launch::Loaded(Box::new(LoadedLaunch {
                context,
                verification_sheet,
                resolution: aplan_app::LaunchResolution {
                    plan_root: retry_root.clone(),
                    root_selection: if call == 0 {
                        aplan_app::RootSelection::Default
                    } else {
                        aplan_app::RootSelection::Explicit
                    },
                    configuration: aplan_app::ConfigurationSelection::None,
                },
            })))
        });
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(None),
            observe: Box::new(|_| panic!("retry must not use the refresh observer")),
            reload: Box::new(|| panic!("retry must not use the refresh loader")),
            retry,
            verify: Box::new(|_, _, _, _| unreachable!("retry does not verify evidence")),
            runtime_build: test_runtime_build(),
        };
        let request = EffectRequest {
            id: model::EffectId(1),
            generation: 0,
            focus_epoch: 0,
            input: "onboarding".to_string(),
            kind: EffectKind::Retry,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: None,
        };

        let EffectOutput::LaunchSnapshot {
            snapshot,
            resolution,
        } = executor
            .execute(&request, &mut |_| {})
            .expect("stable retry")
        else {
            panic!("retry must return one typed launch snapshot")
        };
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(snapshot.overview.input.plan_root, root);
        assert_eq!(
            snapshot.verification_sheet.execute,
            aplan_app::Availability::Available,
            "the returned snapshot must use the confirmed load's run sheet"
        );
        assert!(matches!(
            resolution.root_selection,
            aplan_app::RootSelection::Explicit
        ));
        assert!(matches!(
            resolution.configuration,
            aplan_app::ConfigurationSelection::None
        ));
    }

    /// Proof for `tui.external_refresh_is_identity_safe` and
    /// `tui.production_external_probe_avoids_refresh_work`: a probe performs
    /// only identified context loading and returns no partial projection.
    #[test]
    fn snapshot_executor_probe_returns_only_the_identified_input() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = directory.path().join("aplan");
        std::fs::create_dir_all(root.join("tasks")).expect("tasks");
        std::fs::write(
            root.join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: probe\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");
        let repository_root = directory.path().to_path_buf();
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(None),
            observe: Box::new(move |_| {
                Ok(PlanContext::new(
                    aplan_core::load_plan_identified(&root),
                    aplan_app::ContextInputs {
                        repository_root: repository_root.clone(),
                        configuration_file: None,
                        trust: aplan_app::Trust::Hermetic,
                        options: aplan_core::Options::default(),
                        verdict: aplan_core::Verdict::default(),
                    },
                ))
            }),
            reload: Box::new(|| panic!("a probe must not build refresh inputs")),
            retry: Box::new(|| panic!("a probe must not retry launch")),
            verify: Box::new(|_, _, _, _| unreachable!("a probe does not verify evidence")),
            runtime_build: test_runtime_build(),
        };
        let request = EffectRequest {
            id: model::EffectId(1),
            generation: 0,
            focus_epoch: 0,
            input: "prior".to_string(),
            kind: EffectKind::Probe,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: None,
        };

        let EffectOutput::Identity(identity) =
            executor.execute(&request, &mut |_| {}).expect("probe")
        else {
            panic!("a probe must return identity rather than a complete snapshot")
        };
        assert_eq!(identity.plan_root, directory.path().join("aplan"));
        assert_eq!(identity.trust, aplan_app::Trust::Hermetic);
    }

    #[test]
    /// Proof for `tui.external_observation_is_shutdown_cancellable`.
    fn graceful_quit_cancels_an_in_flight_external_observation() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(None),
            observe: Box::new(move |cancellation| {
                started_tx.send(()).expect("probe start observer");
                let deadline = Instant::now() + Duration::from_secs(1);
                while Instant::now() < deadline {
                    if cancellation.is_cancelled() {
                        cancelled_tx
                            .send(true)
                            .expect("probe cancellation observer");
                        return Err("cancelled at an identified input boundary".to_string());
                    }
                    std::thread::yield_now();
                }
                cancelled_tx.send(false).expect("negative-control observer");
                Err("probe did not receive graceful-shutdown cancellation".to_string())
            }),
            reload: Box::new(|| panic!("a probe must not build refresh inputs")),
            retry: Box::new(|| panic!("a probe must not retry launch")),
            verify: Box::new(|_, _, _, _| unreachable!("a probe does not verify evidence")),
            runtime_build: test_runtime_build(),
        };
        let request = EffectRequest {
            id: model::EffectId(1),
            generation: 0,
            focus_epoch: 0,
            input: "prior".to_string(),
            kind: EffectKind::Probe,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: None,
        };
        let mut runtime = WorkerRuntime::new(Arc::new(executor));
        assert_eq!(runtime.submit(request), Schedule::Started);
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("probe began");

        runtime.cancel_running();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !runtime.finish_shutdown_if_ready() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(
            runtime.finish_shutdown_if_ready(),
            "probe shutdown stayed active"
        );
        assert!(
            cancelled_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("cancellation result"),
            "the executor substituted a non-cancellable observation token"
        );
    }

    #[test]
    fn plan_text_is_visible_and_inert_in_the_rendered_buffer() {
        use ratatui::backend::TestBackend;

        let task_rows = vec![task_row(
            "T-1\u{9b}",
            "erase\u{1b}[2J\tand reverse\u{202e}txt",
        )];
        let inspectors = task_rows.iter().map(task_inspector).collect();
        let mut model = Model::new(Snapshot::test(
            "identity",
            "repo\u{1b}[2J\nnext",
            task_rows,
            inspectors,
            0,
            0,
        ));
        model.update(Action::Route(crate::model::Route::Tasks));
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, &model)).expect("draw");
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();

        assert!(rendered.contains("repo\u{fffd}[2J next"));
        assert!(rendered.contains("T-1\u{fffd}"));
        assert!(rendered.contains("erase\u{fffd}[2J and reverse\u{fffd}txt"));
        assert!(!rendered.contains('\u{1b}') && !rendered.contains('\u{202e}'));
        assert!(!rendered.contains('\n') && !rendered.contains('\t'));
    }

    struct NeverExecutor;

    impl EffectExecutor for NeverExecutor {
        fn execute(
            &self,
            _request: &EffectRequest,
            _progress: &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<EffectOutput, EffectFailure> {
            Err(EffectFailure::Unavailable("not invoked".to_string()))
        }
    }

    struct CountingExecutor(Arc<AtomicUsize>);

    impl EffectExecutor for CountingExecutor {
        fn execute(
            &self,
            _request: &EffectRequest,
            _progress: &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<EffectOutput, EffectFailure> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(EffectFailure::Unavailable(
                "counted test effect".to_string(),
            ))
        }
    }

    struct GatedExecutor {
        started: Sender<()>,
        release: Mutex<Receiver<()>>,
    }

    struct CancellableStartupExecutor {
        first_draws: Arc<AtomicUsize>,
        observed: Sender<(bool, bool)>,
    }

    impl EffectExecutor for CancellableStartupExecutor {
        fn execute(
            &self,
            _request: &EffectRequest,
            _progress: &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<EffectOutput, EffectFailure> {
            panic!("startup runtime must supply its cancellation token")
        }

        fn execute_cancellable(
            &self,
            _request: &EffectRequest,
            cancellation: &aplan_app::CancellationToken,
            _progress: &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<EffectOutput, EffectFailure> {
            let painted_before_start = self.first_draws.load(Ordering::SeqCst) > 0;
            while !cancellation.is_cancelled() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            self.observed
                .send((painted_before_start, cancellation.is_cancelled()))
                .expect("startup observer");
            Err(EffectFailure::Unavailable("startup cancelled".to_string()))
        }
    }

    impl EffectExecutor for GatedExecutor {
        fn execute(
            &self,
            _request: &EffectRequest,
            _progress: &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<EffectOutput, EffectFailure> {
            self.started.send(()).expect("start observer");
            self.release
                .lock()
                .expect("release lock")
                .recv()
                .expect("release effect");
            Ok(EffectOutput::snapshot(test_snapshot("one", 2)))
        }
    }

    struct OneEventInput {
        event: Option<Event>,
        polls: Vec<Duration>,
        reads: usize,
    }

    impl InputSource for OneEventInput {
        fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            self.polls.push(timeout);
            Ok(self.event.is_some())
        }

        fn read(&mut self) -> io::Result<Event> {
            self.reads += 1;
            self.event
                .take()
                .ok_or_else(|| io::Error::other("read without a ready event"))
        }
    }

    /// Proof for `tui.startup_feedback_precedes_plan_io`: host resolution and
    /// snapshot validation/projection share one operation, and the exact build
    /// resolved behind the frame becomes the runtime build for later effects.
    #[test]
    fn startup_executor_projects_after_host_loading_and_publishes_exact_build() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary directory");
        let plan = directory.path().join("plan");
        std::fs::create_dir_all(plan.join("tasks")).expect("task directory");
        std::fs::write(
            plan.join("tasks/T-001.md"),
            "---\nid: T-001\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n---\n",
        )
        .expect("task");
        let (context, verification_sheet) = mutation_test_context(&plan, directory.path());
        let expected_build = BuildIdentity {
            version: "startup-version".to_string(),
            source: "startup-source".to_string(),
            facts_sha256: "startup-facts".to_string(),
            binary_sha256: "startup-binary".to_string(),
        };
        let returned_build = expected_build.clone();
        let facts = BuildFacts {
            version: expected_build.version.clone(),
            ..BuildFacts::default()
        };
        let initial_load: InitialLoad = Box::new(move |_, _| {
            Ok(InitialLaunch {
                launch: Launch::Loaded(Box::new(LoadedLaunch {
                    context,
                    verification_sheet,
                    resolution: aplan_app::LaunchResolution {
                        plan_root: plan,
                        root_selection: aplan_app::RootSelection::Explicit,
                        configuration: aplan_app::ConfigurationSelection::None,
                    },
                })),
                build: returned_build,
                facts,
            })
        });
        let executor = SnapshotExecutor {
            initial_load: std::sync::Mutex::new(Some(initial_load)),
            observe: Box::new(|_| panic!("startup does not observe twice")),
            reload: Box::new(|| panic!("startup does not reload")),
            retry: Box::new(|| panic!("startup does not retry")),
            verify: Box::new(|_, _, _, _| unreachable!("startup does not verify")),
            runtime_build: std::sync::OnceLock::new(),
        };
        let request = EffectRequest {
            id: model::EffectId(7),
            generation: 0,
            focus_epoch: 0,
            input: String::new(),
            kind: EffectKind::Startup,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: None,
        };
        let cancellation = aplan_app::CancellationToken::default();
        let mut events = Vec::new();
        let output = executor
            .execute_cancellable(&request, &cancellation, &mut |event| events.push(event))
            .expect("startup completes");
        let EffectOutput::Startup(ready) = output else {
            panic!("startup returned the wrong output")
        };
        assert_eq!(ready.snapshot.build, expected_build);
        assert_eq!(executor.build().expect("published build"), &expected_build);
        let started = events
            .iter()
            .filter_map(|event| match (event.lifecycle, event.subject) {
                (
                    aplan_app::ProgressLifecycle::Started,
                    aplan_app::ProgressSubject::Phase { phase, .. },
                ) => Some(phase),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            started,
            [
                aplan_app::ProgressPhase::StructuralValidation,
                aplan_app::ProgressPhase::Projection,
            ]
        );
    }

    /// Proof for `tui.startup_feedback_precedes_plan_io`: the initial frame is
    /// flushed before the startup request reaches the worker, and a quit event
    /// cancels that worker without detaching it or bypassing mode restoration.
    #[test]
    fn first_loading_frame_precedes_cancellable_worker_and_quit_restores_modes() {
        let first_draws = Arc::new(AtomicUsize::new(0));
        let (observed_tx, observed_rx) = channel();
        let mut runtime = WorkerRuntime::new(Arc::new(CancellableStartupExecutor {
            first_draws: Arc::clone(&first_draws),
            observed: observed_tx,
        }));
        let build = BuildIdentity::default();
        let capabilities = BuildFacts::default().capability_view(&build);
        let (mut model, request) = Model::loading(&build, capabilities, Appearance::default());
        let mut initial = Some(request);
        let mut input = OneEventInput {
            event: Some(Event::Key(KeyEvent::new(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
            ))),
            polls: Vec::new(),
            reads: 0,
        };
        let cursor = Rc::new(RefCell::new(CursorState::default()));
        let backend = CursorBackend {
            state: Rc::clone(&cursor),
            size: ratatui::layout::Size::new(80, 24),
            draw_observer: Some(Arc::clone(&first_draws)),
        };
        let terminal_modes = Rc::new(RefCell::new(RecordingState::default()));
        let mut lifecycle = NoLifecycle;
        let exit = with_modes(recorder(&terminal_modes), || {
            run_screen_with_initial(
                backend,
                &mut input,
                &mut model,
                &mut runtime,
                &mut lifecycle,
                &mut initial,
            )
        })
        .expect("loading screen exits cleanly");
        assert_eq!(exit, SessionExit::Quit);
        runtime.shutdown();
        assert_eq!(
            observed_rx.recv().expect("startup observation"),
            (true, true)
        );
        assert!(first_draws.load(Ordering::SeqCst) >= 1);
        assert_eq!(terminal_modes.borrow().events, expected_cycle());
        assert_eq!(cursor.borrow().shows, 1);
    }

    /// Proof for `tui.startup_feedback_precedes_plan_io`: an error or unwind
    /// which drops the runtime still cancels startup before joining its owner.
    #[test]
    fn dropping_a_startup_runtime_requests_cancellation_before_joining() {
        let first_draws = Arc::new(AtomicUsize::new(1));
        let (observed_tx, observed_rx) = channel();
        let mut runtime = WorkerRuntime::new(Arc::new(CancellableStartupExecutor {
            first_draws,
            observed: observed_tx,
        }));
        let build = BuildIdentity::default();
        let capabilities = BuildFacts::default().capability_view(&build);
        let (_, request) = Model::loading(&build, capabilities, Appearance::default());
        assert_eq!(runtime.submit(request), Schedule::Started);
        drop(runtime);
        assert_eq!(
            observed_rx.recv().expect("startup observation"),
            (true, true)
        );
    }

    #[derive(Debug, Clone, Copy)]
    enum InputFailure {
        Poll,
        Read,
    }

    struct FailingInput(InputFailure);

    impl InputSource for FailingInput {
        fn poll(&mut self, _timeout: Duration) -> io::Result<bool> {
            match self.0 {
                InputFailure::Poll => Err(io::Error::other("poll failed")),
                InputFailure::Read => Ok(true),
            }
        }

        fn read(&mut self) -> io::Result<Event> {
            match self.0 {
                InputFailure::Poll => panic!("read after a failed poll"),
                InputFailure::Read => Err(io::Error::other("read failed")),
            }
        }
    }

    fn test_snapshot(input: &str, rows: usize) -> Snapshot {
        let task_rows: Vec<_> = (0..rows)
            .map(|index| task_row(format!("T-{index}"), format!("task {index}")))
            .collect();
        let inspectors = task_rows.iter().map(task_inspector).collect();
        Snapshot::test(input, "repo", task_rows, inspectors, 0, 0)
    }

    fn test_verification(input: &str, errors: usize, warnings: usize) -> EffectOutput {
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

    #[test]
    fn every_renderer_initialization_boundary_restores_all_acquired_modes() {
        for failed_size_call in [1, 2] {
            let mode_state = Rc::new(RefCell::new(RecordingState::default()));
            let cursor = Rc::new(RefCell::new(CursorState {
                fail_size_call: Some(failed_size_call),
                ..CursorState::default()
            }));
            let mut input = OneEventInput {
                event: Some(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                ))),
                polls: Vec::new(),
                reads: 0,
            };
            let mut runtime = WorkerRuntime::new(Arc::new(NeverExecutor));
            let mut model = Model::new(test_snapshot("one", 1));
            let mut lifecycle = NoLifecycle;
            let error = with_modes(recorder(&mode_state), || {
                run_screen_with(
                    cursor_backend(&cursor),
                    &mut input,
                    &mut model,
                    &mut runtime,
                    &mut lifecycle,
                )
            })
            .expect_err("initialization must fail");
            runtime.shutdown();

            let message = error.to_string();
            assert!(message.contains("initialization"), "{message}");
            assert!(message.contains("size failed"), "{message}");
            assert_eq!(mode_state.borrow().events, expected_cycle());
            assert_eq!(cursor.borrow().hides, 0);
            assert_eq!(cursor.borrow().shows, 0);
        }
    }

    #[test]
    fn every_first_frame_failure_restores_the_cursor_and_all_acquired_modes() {
        let cases = [
            (
                CursorState {
                    fail_size_call: Some(3),
                    ..CursorState::default()
                },
                "size failed",
            ),
            (
                CursorState {
                    fail_draw: true,
                    ..CursorState::default()
                },
                "draw failed",
            ),
            (
                CursorState {
                    fail_hide: true,
                    ..CursorState::default()
                },
                "hide failed",
            ),
            (
                CursorState {
                    fail_flush: true,
                    ..CursorState::default()
                },
                "flush failed",
            ),
        ];

        for (cursor_state, expected) in cases {
            let mode_state = Rc::new(RefCell::new(RecordingState::default()));
            let cursor = Rc::new(RefCell::new(cursor_state));
            let mut input = OneEventInput {
                event: Some(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                ))),
                polls: Vec::new(),
                reads: 0,
            };
            let mut runtime = WorkerRuntime::new(Arc::new(NeverExecutor));
            let mut model = Model::new(test_snapshot("one", 1));
            let mut lifecycle = NoLifecycle;
            let error = with_modes(recorder(&mode_state), || {
                run_screen_with(
                    cursor_backend(&cursor),
                    &mut input,
                    &mut model,
                    &mut runtime,
                    &mut lifecycle,
                )
            })
            .expect_err("first frame must fail");
            runtime.shutdown();

            let message = error.to_string();
            assert!(message.contains("terminal draw failed"), "{message}");
            assert!(message.contains(expected), "{message}");
            assert_eq!(mode_state.borrow().events, expected_cycle());
            assert_eq!(cursor.borrow().shows, 1, "failed at {expected}");
        }
    }

    #[test]
    fn every_input_failure_restores_the_cursor_and_all_acquired_modes() {
        for (failure, operation, source) in [
            (InputFailure::Poll, "input poll", "poll failed"),
            (InputFailure::Read, "input read", "read failed"),
        ] {
            let mode_state = Rc::new(RefCell::new(RecordingState::default()));
            let cursor = Rc::new(RefCell::new(CursorState::default()));
            let mut input = FailingInput(failure);
            let mut runtime = WorkerRuntime::new(Arc::new(NeverExecutor));
            let mut model = Model::new(test_snapshot("one", 1));
            let mut lifecycle = NoLifecycle;
            let error = with_modes(recorder(&mode_state), || {
                run_screen_with(
                    cursor_backend(&cursor),
                    &mut input,
                    &mut model,
                    &mut runtime,
                    &mut lifecycle,
                )
            })
            .expect_err("input must fail");
            runtime.shutdown();

            let message = error.to_string();
            assert!(message.contains(operation), "{message}");
            assert!(message.contains(source), "{message}");
            assert_eq!(mode_state.borrow().events, expected_cycle());
            assert_eq!(cursor.borrow().hides, 1);
            assert_eq!(cursor.borrow().shows, 1);
        }
    }

    #[test]
    fn an_idle_loop_draws_once_and_blocks_on_its_single_input_owner() {
        let cursor = Rc::new(RefCell::new(CursorState::default()));
        let mut terminal = Terminal::new(cursor_backend(&cursor)).expect("terminal");
        let mut input = OneEventInput {
            event: Some(Event::Key(KeyEvent::new(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
            ))),
            polls: Vec::new(),
            reads: 0,
        };
        let mut model = Model::new(test_snapshot("one", 1));
        let mut runtime = WorkerRuntime::new(Arc::new(NeverExecutor));
        let mut lifecycle = NoLifecycle;

        run_loop(
            &mut terminal,
            &mut input,
            &mut model,
            &mut runtime,
            &mut lifecycle,
        )
        .expect("event loop");
        runtime.shutdown();

        assert_eq!(input.polls.len(), 1);
        assert!(input.polls[0] <= EXTERNAL_POLL_INTERVAL);
        assert_eq!(input.reads, 1, "read follows the one successful poll");
        assert_eq!(cursor.borrow().draws, 1, "idle renders exactly one frame");
        assert!(
            !model.take_dirty(),
            "quit does not schedule a redundant frame"
        );
    }

    /// Proof for `tui.quit_starts_no_new_work`: a requested quit is observed
    /// before an overdue identity probe can start or another frame or input poll can occur.
    #[test]
    fn quit_does_not_start_an_overdue_external_probe() {
        let cursor = Rc::new(RefCell::new(CursorState::default()));
        let mut terminal = Terminal::new(cursor_backend(&cursor)).expect("terminal");
        let mut input = OneEventInput {
            event: None,
            polls: Vec::new(),
            reads: 0,
        };
        let mut model = Model::new(test_snapshot("one", 1));
        assert!(model.take_dirty());
        model.freshness = model::SnapshotFreshness::Unconfirmed;
        model.update(Action::Quit);
        let mut runtime = WorkerRuntime::new(Arc::new(NeverExecutor));
        let mut lifecycle = NoLifecycle;

        let exit = run_loop(
            &mut terminal,
            &mut input,
            &mut model,
            &mut runtime,
            &mut lifecycle,
        )
        .expect("event loop");

        assert!(matches!(exit, SessionExit::Quit));
        assert!(!runtime.has_work(), "quit must not initiate a fresh probe");
        assert!(input.polls.is_empty(), "quit must not wait for more input");
        assert_eq!(cursor.borrow().draws, 0, "quit must not draw another frame");
        runtime.shutdown();
    }

    /// Proof for `tui.quit_starts_no_new_work`: a coalesced continue is consumed
    /// without reactivating a session whose reducer already accepted quit.
    #[test]
    fn quit_ignores_continue_without_starting_a_resume_probe() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cursor = Rc::new(RefCell::new(CursorState::default()));
        let mut terminal = Terminal::new(cursor_backend(&cursor)).expect("terminal");
        let mut input = OneEventInput {
            event: None,
            polls: Vec::new(),
            reads: 0,
        };
        let mut model = Model::new(test_snapshot("one", 1));
        model.update(Action::Quit);
        let mut runtime = WorkerRuntime::new(Arc::new(CountingExecutor(Arc::clone(&calls))));
        let mut lifecycle = QueuedLifecycle {
            events: [LifecycleEvent::Continue].into(),
        };

        let exit = run_loop(
            &mut terminal,
            &mut input,
            &mut model,
            &mut runtime,
            &mut lifecycle,
        )
        .expect("event loop");
        runtime.shutdown();

        assert!(matches!(exit, SessionExit::Quit));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(lifecycle.events.is_empty(), "the continue was consumed");
        assert!(input.polls.is_empty(), "quit must not wait for more input");
        assert_eq!(cursor.borrow().draws, 0, "quit must not draw another frame");
    }

    /// Proof for `tui.quit_starts_no_new_work`: consuming a completed worker
    /// would start its queued successor, so quit must be observed before drain.
    #[test]
    fn quit_drops_queued_work_before_draining_a_completion() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cursor = Rc::new(RefCell::new(CursorState::default()));
        let mut terminal = Terminal::new(cursor_backend(&cursor)).expect("terminal");
        let mut input = OneEventInput {
            event: None,
            polls: Vec::new(),
            reads: 0,
        };
        let mut model = Model::new(test_snapshot("one", 1));
        let mut runtime = WorkerRuntime::new(Arc::new(CountingExecutor(Arc::clone(&calls))));
        dispatch(&mut model, &mut runtime, Action::Refresh);
        dispatch(&mut model, &mut runtime, Action::Verify);
        assert!(
            model.has_queued_effect(),
            "the second effect must be queued"
        );
        while !runtime.active_finished_for_test() {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        model.update(Action::Quit);
        let mut lifecycle = NoLifecycle;

        let exit = run_loop(
            &mut terminal,
            &mut input,
            &mut model,
            &mut runtime,
            &mut lifecycle,
        )
        .expect("event loop");
        runtime.shutdown();

        assert!(matches!(exit, SessionExit::Quit));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "shutdown joins the completed first effect but drops its queued successor"
        );
        assert!(input.polls.is_empty(), "quit must not wait for more input");
        assert_eq!(cursor.borrow().draws, 0, "quit must not draw another frame");
    }

    /// Proof for `tui.quit_starts_no_new_work`: process termination and
    /// suspension remain stronger lifecycle outcomes than an ordinary quit.
    #[test]
    fn quit_preserves_termination_and_suspension_precedence() {
        for (event, expected) in [
            (LifecycleEvent::Terminate(15), SessionExit::Terminate(15)),
            (LifecycleEvent::Suspend, SessionExit::Suspend),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let cursor = Rc::new(RefCell::new(CursorState::default()));
            let mut terminal = Terminal::new(cursor_backend(&cursor)).expect("terminal");
            let mut input = OneEventInput {
                event: None,
                polls: Vec::new(),
                reads: 0,
            };
            let mut model = Model::new(test_snapshot("one", 1));
            model.update(Action::Quit);
            let mut runtime = WorkerRuntime::new(Arc::new(CountingExecutor(Arc::clone(&calls))));
            let mut lifecycle = QueuedLifecycle {
                events: [event].into(),
            };

            let exit = run_loop(
                &mut terminal,
                &mut input,
                &mut model,
                &mut runtime,
                &mut lifecycle,
            )
            .expect("event loop");
            runtime.shutdown();

            assert_eq!(exit, expected);
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert!(input.polls.is_empty());
            assert_eq!(cursor.borrow().draws, 0);
        }
    }

    /// Proof for `tui.overdue_external_probe_is_fail_closed`: a busy worker
    /// cannot move the observation deadline or leave its old snapshot labelled
    /// current, and releasing that worker yields one immediate probe without a
    /// repeated notice or redraw loop.
    #[test]
    fn an_overdue_external_probe_waits_for_capacity_without_claiming_current() {
        let mut model = Model::new(test_snapshot("one", 1));
        assert!(model.take_dirty());
        let now = Instant::now();
        let mut deadline = now;

        assert!(external_probe_action(&mut model, true, now, &mut deadline).is_none());
        assert_eq!(
            model.freshness,
            crate::model::SnapshotFreshness::Unconfirmed
        );
        assert!(model.take_dirty(), "the changed freshness is rendered once");
        assert_eq!(deadline, now, "a missed deadline remains due");

        assert!(external_probe_action(&mut model, true, now, &mut deadline).is_none());
        assert!(
            !model.take_dirty(),
            "a busy loop does not redraw repeatedly"
        );
        assert_eq!(deadline, now);

        let action = external_probe_action(&mut model, false, now, &mut deadline)
            .expect("capacity starts the overdue probe immediately");
        assert_eq!(action.kind(), crate::model::ActionKind::PollExternal);
        assert_eq!(deadline, now + EXTERNAL_POLL_INTERVAL);
        let probe = model.update(action).effect.expect("probe request");
        assert_eq!(probe.kind, EffectKind::Probe);
        assert_eq!(
            model.freshness,
            crate::model::SnapshotFreshness::Unconfirmed
        );
    }

    /// Proof for `tui.suspension_invalidates_snapshot_freshness`: resuming
    /// while deliberate work owns the worker keeps the probe deadline due, so
    /// capacity starts the new-epoch observation without a five-second wait.
    #[test]
    fn focus_regain_retries_its_probe_as_soon_as_a_busy_worker_is_available() {
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        let mut runtime = WorkerRuntime::new(Arc::new(GatedExecutor {
            started: started_tx,
            release: Mutex::new(release_rx),
        }));
        let mut model = Model::new(test_snapshot("one", 1));
        dispatch(&mut model, &mut runtime, Action::Verify);
        started_rx.recv().expect("verification started");

        model.update(Action::FocusLost);
        let now = Instant::now();
        let mut deadline = now + EXTERNAL_POLL_INTERVAL;
        regain_focus(&mut model, &mut runtime, now, &mut deadline);
        assert_eq!(deadline, now, "the refused focus probe remains due");
        assert_eq!(
            model.freshness,
            crate::model::SnapshotFreshness::Unconfirmed
        );

        dispatch(&mut model, &mut runtime, Action::Refresh);
        assert!(model.has_queued_effect(), "the manual refresh is queued");
        assert!(
            external_probe_action(&mut model, true, now, &mut deadline).is_none(),
            "a queued replacement does not create a concurrent probe"
        );
        assert_eq!(
            deadline, now,
            "queued work that can still be cancelled does not satisfy the deadline"
        );
        dispatch(&mut model, &mut runtime, Action::CancelQueued);
        assert!(!model.has_queued_effect());

        release_tx.send(()).expect("release verification");
        while runtime.has_work() {
            for event in runtime.drain() {
                match event {
                    RuntimeEvent::Started(id) => {
                        dispatch(&mut model, &mut runtime, Action::EffectStarted(id));
                    }
                    RuntimeEvent::Progress(id, progress) => dispatch(
                        &mut model,
                        &mut runtime,
                        Action::EffectProgress { id, progress },
                    ),
                    RuntimeEvent::Completed(completion) => dispatch(
                        &mut model,
                        &mut runtime,
                        Action::EffectCompleted(completion),
                    ),
                }
            }
            std::thread::yield_now();
        }

        let action = external_probe_action(&mut model, false, now, &mut deadline)
            .expect("worker capacity starts the due focus probe");
        let probe = model.update(action).effect.expect("probe request");
        assert_eq!(probe.kind, EffectKind::Probe);
        assert_eq!(probe.focus_epoch, 1, "the probe belongs to the new epoch");
        assert_eq!(deadline, now + EXTERNAL_POLL_INTERVAL);
        runtime.shutdown();
    }

    /// Proof for `tui.event_loop_is_generation_safe` and
    /// `tui.verification_is_typed_and_non_blocking`: a running effect never
    /// moves navigation off the input thread.
    #[test]
    fn navigation_remains_available_while_an_effect_is_blocked() {
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        let mut runtime = WorkerRuntime::new(Arc::new(GatedExecutor {
            started: started_tx,
            release: Mutex::new(release_rx),
        }));
        let mut model = Model::new(test_snapshot("one", 2));
        model.update(Action::Route(crate::model::Route::Tasks));

        dispatch(&mut model, &mut runtime, Action::Refresh);
        started_rx.recv().expect("effect started");
        assert!(runtime.has_work());

        let response_started = Instant::now();
        dispatch(&mut model, &mut runtime, Action::SelectNext);
        let response_time = response_started.elapsed();
        assert_eq!(model.selected, 1, "navigation stays on the input thread");
        assert!(
            response_time < Duration::from_millis(100),
            "input-thread navigation took {response_time:?} while a worker was blocked"
        );

        release_tx.send(()).expect("release effect");
        loop {
            let events = runtime.drain();
            if events.is_empty() {
                std::thread::yield_now();
                continue;
            }
            for event in events {
                match event {
                    RuntimeEvent::Started(id) => {
                        dispatch(&mut model, &mut runtime, Action::EffectStarted(id));
                    }
                    RuntimeEvent::Progress(id, progress) => dispatch(
                        &mut model,
                        &mut runtime,
                        Action::EffectProgress { id, progress },
                    ),
                    RuntimeEvent::Completed(completion) => dispatch(
                        &mut model,
                        &mut runtime,
                        Action::EffectCompleted(completion),
                    ),
                }
            }
            break;
        }
        assert_eq!(model.pending_effects(), 0);
        runtime.shutdown();
    }

    /// Proof for `tui.event_loop_is_generation_safe` and
    /// `tui.verification_is_typed_and_non_blocking`: terminal-visible state is
    /// restored before shutdown can wait on a worker.
    #[test]
    fn terminal_modes_restore_before_shutdown_waits_for_a_worker() {
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        let mut runtime = WorkerRuntime::new(Arc::new(GatedExecutor {
            started: started_tx,
            release: Mutex::new(release_rx),
        }));
        let request = Model::new(test_snapshot("one", 1))
            .update(Action::Refresh)
            .effect
            .expect("refresh");
        assert_eq!(runtime.submit(request), Schedule::Started);
        started_rx.recv().expect("effect started");

        let modes = Rc::new(RefCell::new(RecordingState::default()));
        with_modes(recorder(&modes), || Ok(())).expect("terminal body");
        assert_eq!(
            modes.borrow().events,
            expected_cycle(),
            "terminal ownership ends before the worker is joined"
        );

        release_tx.send(()).expect("release effect");
        runtime.shutdown();
    }

    #[derive(Debug, Default)]
    struct EventOnlyDuringShutdown {
        polls: usize,
    }

    impl LifecycleSource for EventOnlyDuringShutdown {
        fn take_event(&mut self) -> Option<LifecycleEvent> {
            self.polls += 1;
            (self.polls == 2).then_some(LifecycleEvent::Terminate(15))
        }

        fn poll_cap(&self) -> Option<Duration> {
            Some(Duration::ZERO)
        }
    }

    /// Proof for `tui.termination_preempts_worker_shutdown`: a lifecycle event
    /// that does not exist at the first post-terminal check is still consumed
    /// while an unfinished worker prevents graceful return.
    #[test]
    fn a_lifecycle_event_arriving_only_during_worker_shutdown_is_consumed() {
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        let mut runtime = WorkerRuntime::new(Arc::new(GatedExecutor {
            started: started_tx,
            release: Mutex::new(release_rx),
        }));
        let request = Model::new(test_snapshot("one", 1))
            .update(Action::Refresh)
            .effect
            .expect("refresh");
        assert_eq!(runtime.submit(request), Schedule::Started);
        started_rx.recv().expect("worker started");

        let mut lifecycle = EventOnlyDuringShutdown::default();
        assert_eq!(
            wait_for_worker_or_lifecycle(&mut runtime, &mut lifecycle),
            Some(LifecycleEvent::Terminate(15))
        );
        assert!(runtime.has_work(), "the event preempted the worker wait");

        release_tx.send(()).expect("release worker");
        runtime.shutdown();
    }

    /// A continued process may leave a coalesced `SIGCONT` in the lifecycle
    /// queue just as the user quits. It is advisory at this boundary: consume
    /// it, observe that no worker remains, and finish the graceful shutdown.
    #[test]
    fn a_continue_at_graceful_shutdown_does_not_prevent_completion() {
        let mut runtime = WorkerRuntime::new(Arc::new(GatedExecutor {
            started: channel().0,
            release: Mutex::new(channel().1),
        }));
        let mut lifecycle = QueuedLifecycle {
            events: [LifecycleEvent::Continue].into(),
        };

        shutdown_after_terminal_restore(&mut runtime, &mut lifecycle)
            .expect("continued graceful shutdown");

        assert!(!runtime.has_work());
        assert!(lifecycle.events.is_empty(), "the continue was consumed");
    }

    /// `tui.event_loop_is_generation_safe`: rendering is dirty-driven and a
    /// completion identified by an older generation cannot replace the current
    /// complete snapshot.
    #[test]
    fn event_loop_is_dirty_driven_and_effects_are_generation_safe() {
        let mut model = Model::new(test_snapshot("one", 1));
        assert!(model.take_dirty());
        assert!(!model.take_dirty());

        let replacement = model.update(Action::Refresh).effect.expect("refresh");
        let stale = model.update(Action::Verify).effect.expect("verify");
        model.update(Action::EffectCompleted(Box::new(model::EffectCompletion {
            request: replacement,
            outcome: Ok(EffectOutput::snapshot(test_snapshot("two", 2))),
        })));
        model.update(Action::EffectCompleted(Box::new(model::EffectCompletion {
            request: stale,
            outcome: Ok(test_verification("one", 9, 9)),
        })));

        assert_eq!(model.snapshot.overview.input.sha256, "two");
        assert_eq!(model.snapshot.overview.loaded_tasks, 2);
        assert!(
            model
                .status_line()
                .contains("Stale verify result discarded")
        );
    }
}
