use std::cell::Cell;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::Once;
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::thread::{self, JoinHandle};

use crate::model::{EffectCompletion, EffectFailure, EffectId, EffectOutput, EffectRequest};

thread_local! {
    /// The process panic hook is global, but whether a panic belongs to the
    /// effect boundary is a property of this thread. A caught worker panic must
    /// become model state without writing through the active alternate screen.
    static EFFECT_WORKER: Cell<bool> = const { Cell::new(false) };
}

static INSTALL_EFFECT_PANIC_HOOK: Once = Once::new();

/// Install one transparent process hook: worker panics caught below are quiet,
/// while every other panic still reaches the hook the host installed first.
///
/// Swapping hooks around each execution would race other threads. Leaving one
/// thread-local dispatcher in place makes repeated runtimes deterministic and
/// does not suppress unrelated panic diagnostics.
fn install_effect_panic_hook() {
    INSTALL_EFFECT_PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let caught_by_effect_boundary = EFFECT_WORKER.try_with(Cell::get).unwrap_or(false);
            if !caught_by_effect_boundary {
                previous(info);
            }
        }));
    });
}

/// Execute typed work without access to terminal state or the reducer.
pub(crate) trait EffectExecutor: Send + Sync + 'static {
    fn execute(
        &self,
        request: &EffectRequest,
        progress: &mut dyn FnMut(aplan_app::ProgressEvent),
    ) -> Result<EffectOutput, EffectFailure>;

    fn execute_cancellable(
        &self,
        request: &EffectRequest,
        _cancellation: &aplan_app::CancellationToken,
        progress: &mut dyn FnMut(aplan_app::ProgressEvent),
    ) -> Result<EffectOutput, EffectFailure> {
        self.execute(request, progress)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Schedule {
    Started,
    Queued,
    Full,
}

#[derive(Debug, PartialEq)]
pub(crate) enum RuntimeEvent {
    Started(EffectId),
    Progress(EffectId, aplan_app::ProgressEvent),
    Completed(Box<EffectCompletion>),
}

/// One running worker plus one queued request. The completion channel is also
/// bounded to one because only the running worker can produce into it.
pub(crate) struct WorkerRuntime {
    executor: Arc<dyn EffectExecutor>,
    completed_tx: SyncSender<EffectCompletion>,
    completed_rx: Receiver<EffectCompletion>,
    progress_tx: Sender<(EffectId, aplan_app::ProgressEvent)>,
    progress_rx: Receiver<(EffectId, aplan_app::ProgressEvent)>,
    active: Option<(EffectId, JoinHandle<()>, aplan_app::CancellationToken)>,
    queued: VecDeque<EffectRequest>,
}

impl WorkerRuntime {
    pub(crate) fn new(executor: Arc<dyn EffectExecutor>) -> Self {
        install_effect_panic_hook();
        let (completed_tx, completed_rx) = sync_channel(1);
        let (progress_tx, progress_rx) = channel();
        Self {
            executor,
            completed_tx,
            completed_rx,
            progress_tx,
            progress_rx,
            active: None,
            queued: VecDeque::with_capacity(1),
        }
    }

    pub(crate) fn submit(&mut self, request: EffectRequest) -> Schedule {
        if self.active.is_none() {
            self.start(request);
            Schedule::Started
        } else if self.queued.is_empty() {
            self.queued.push_back(request);
            Schedule::Queued
        } else {
            Schedule::Full
        }
    }

    pub(crate) fn drain(&mut self) -> Vec<RuntimeEvent> {
        let mut events = Vec::new();
        while let Ok((id, progress)) = self.progress_rx.try_recv() {
            events.push(RuntimeEvent::Progress(id, progress));
        }
        while let Ok(completion) = self.completed_rx.try_recv() {
            if let Some((id, worker, _)) = self.active.take() {
                debug_assert_eq!(id, completion.request.id);
                let _ = worker.join();
            }
            events.push(RuntimeEvent::Completed(Box::new(completion)));
            if let Some(next) = self.queued.pop_front() {
                let id = next.id;
                self.start(next);
                events.push(RuntimeEvent::Started(id));
            }
        }
        events
    }

    pub(crate) fn has_work(&self) -> bool {
        self.active.is_some() || !self.queued.is_empty()
    }

    /// Request cooperative cancellation of the running operation. The worker
    /// remains owned and is still joined after terminal restoration.
    pub(crate) fn cancel_running(&self) {
        if let Some((_, _, cancellation)) = &self.active {
            cancellation.cancel();
        }
    }

    /// Whether the running test worker has finished and placed its completion.
    #[cfg(test)]
    pub(crate) fn active_finished_for_test(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(_, worker, _)| worker.is_finished())
    }

    /// Begin terminal-independent shutdown and join only after the running
    /// worker has actually finished.
    ///
    /// The caller polls this from the same loop that consumes process
    /// lifecycle events. Calling `JoinHandle::join` while the worker is still
    /// active would make the signal handler a write-only queue: a termination
    /// signal would be recorded, but the terminal thread could never observe
    /// and re-raise it (`A1-560`).
    pub(crate) fn finish_shutdown_if_ready(&mut self) -> bool {
        self.queued.clear();
        if self
            .active
            .as_ref()
            .is_some_and(|(_, worker, _)| !worker.is_finished())
        {
            return false;
        }
        if let Some((_, worker, _)) = self.active.take() {
            let _ = worker.join();
        }
        true
    }

    /// Cancel work that has not started. Running repository/process work is not
    /// detached: the caller must keep showing it as active and shutdown joins
    /// it, because the current verifier exposes no cooperative cancellation.
    pub(crate) fn cancel_queued(&mut self, id: EffectId) -> bool {
        let Some(index) = self.queued.iter().position(|request| request.id == id) else {
            return false;
        };
        self.queued.remove(index);
        true
    }

    /// Wait for the sole running worker and discard work that never started.
    /// Workers never receive a terminal handle, so cleanup cannot race them.
    #[cfg(test)]
    pub(crate) fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.queued.clear();
        if let Some((_, worker, cancellation)) = self.active.take() {
            cancellation.cancel();
            let _ = worker.join();
        }
    }

    fn start(&mut self, request: EffectRequest) {
        let id = request.id;
        let executor = Arc::clone(&self.executor);
        let completed = self.completed_tx.clone();
        let progress = self.progress_tx.clone();
        let cancellation = aplan_app::CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        let worker = thread::spawn(move || {
            let outcome = EFFECT_WORKER.with(|effect_worker| {
                effect_worker.set(true);
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    executor.execute_cancellable(&request, &worker_cancellation, &mut |event| {
                        let _ = progress.send((id, event));
                    })
                }))
                .unwrap_or(Err(EffectFailure::WorkerPanicked));
                effect_worker.set(false);
                outcome
            });
            let _ = completed.send(EffectCompletion { request, outcome });
        });
        self.active = Some((id, worker, cancellation));
    }
}

impl Drop for WorkerRuntime {
    fn drop(&mut self) {
        // `run` normally drains `active` through its lifecycle-aware shutdown
        // loop after terminal modes have been restored. An unwind bypasses that
        // loop, so joining from `Drop` still keeps repository or process work
        // from escaping the runtime. This is a no-op on the ordinary path.
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EffectKind, Snapshot};
    use std::process::Command;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Sender, channel};

    fn request(id: u64, kind: EffectKind) -> EffectRequest {
        EffectRequest {
            id: EffectId(id),
            generation: 0,
            focus_epoch: 0,
            input: "input".to_string(),
            kind,
            check_overrides: aplan_app::CheckOverrides::default(),
            execute: false,
            mutation: None,
        }
    }

    fn output() -> EffectOutput {
        EffectOutput::snapshot(Snapshot::test(
            "input",
            "repo",
            Vec::new(),
            Vec::new(),
            0,
            0,
        ))
    }

    struct BlockingExecutor {
        starts: Sender<EffectId>,
        releases: Mutex<Receiver<()>>,
        concurrent: AtomicUsize,
        peak: AtomicUsize,
    }

    impl EffectExecutor for BlockingExecutor {
        fn execute(
            &self,
            request: &EffectRequest,
            _progress: &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<EffectOutput, EffectFailure> {
            let concurrent = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(concurrent, Ordering::SeqCst);
            self.starts.send(request.id).expect("start observer");
            self.releases
                .lock()
                .expect("release lock")
                .recv()
                .expect("release worker");
            self.concurrent.fetch_sub(1, Ordering::SeqCst);
            Ok(output())
        }
    }

    /// Proof for `tui.event_loop_is_generation_safe`: concurrency and queued
    /// work are both bounded by construction.
    #[test]
    fn one_worker_runs_one_waits_and_a_third_is_refused() {
        let (starts_tx, starts_rx) = channel();
        let (release_tx, release_rx) = channel();
        let executor = Arc::new(BlockingExecutor {
            starts: starts_tx,
            releases: Mutex::new(release_rx),
            concurrent: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let mut runtime = WorkerRuntime::new(executor.clone());

        assert_eq!(
            runtime.submit(request(1, EffectKind::Refresh)),
            Schedule::Started
        );
        assert_eq!(starts_rx.recv().expect("first start"), EffectId(1));
        assert_eq!(
            runtime.submit(request(2, EffectKind::Verify)),
            Schedule::Queued
        );
        assert_eq!(
            runtime.submit(request(3, EffectKind::Refresh)),
            Schedule::Full
        );

        release_tx.send(()).expect("release first");
        let mut events = Vec::new();
        while events.is_empty() {
            thread::yield_now();
            events = runtime.drain();
        }
        assert!(matches!(events[0], RuntimeEvent::Completed(_)));
        assert_eq!(events[1], RuntimeEvent::Started(EffectId(2)));
        assert_eq!(starts_rx.recv().expect("second start"), EffectId(2));
        release_tx.send(()).expect("release second");
        runtime.shutdown();
        assert_eq!(executor.peak.load(Ordering::SeqCst), 1);
    }

    struct PanicExecutor;

    impl EffectExecutor for PanicExecutor {
        fn execute(
            &self,
            _request: &EffectRequest,
            _progress: &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<EffectOutput, EffectFailure> {
            panic!("worker fault")
        }
    }

    struct ProgressExecutor;

    impl EffectExecutor for ProgressExecutor {
        fn execute(
            &self,
            _request: &EffectRequest,
            progress: &mut dyn FnMut(aplan_app::ProgressEvent),
        ) -> Result<EffectOutput, EffectFailure> {
            progress(aplan_app::ProgressEvent {
                operation: aplan_app::OperationId::new(1),
                sequence: 0,
                kind: aplan_app::OperationKind::Verification,
                subject: aplan_app::ProgressSubject::Operation,
                lifecycle: aplan_app::ProgressLifecycle::Queued,
                measure: aplan_app::ProgressMeasure::Indeterminate,
            });
            progress(aplan_app::ProgressEvent {
                operation: aplan_app::OperationId::new(1),
                sequence: 1,
                kind: aplan_app::OperationKind::Verification,
                subject: aplan_app::ProgressSubject::Operation,
                lifecycle: aplan_app::ProgressLifecycle::Started,
                measure: aplan_app::ProgressMeasure::Indeterminate,
            });
            Ok(output())
        }
    }

    /// Proof for `show.evidence_observations_are_nonblocking`.
    #[test]
    fn provider_progress_precedes_the_typed_completion() {
        let mut runtime = WorkerRuntime::new(Arc::new(ProgressExecutor));
        assert_eq!(
            runtime.submit(request(1, EffectKind::Verify)),
            Schedule::Started
        );
        let mut events = Vec::new();
        loop {
            events.extend(runtime.drain());
            if events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::Completed(_)))
            {
                break;
            }
            std::thread::yield_now();
        }
        assert!(matches!(
            events.as_slice(),
            [
                RuntimeEvent::Progress(
                    EffectId(1),
                    aplan_app::ProgressEvent {
                        sequence: 0,
                        lifecycle: aplan_app::ProgressLifecycle::Queued,
                        ..
                    }
                ),
                RuntimeEvent::Progress(
                    EffectId(1),
                    aplan_app::ProgressEvent {
                        sequence: 1,
                        lifecycle: aplan_app::ProgressLifecycle::Started,
                        ..
                    }
                ),
                RuntimeEvent::Completed(_)
            ]
        ));
        runtime.shutdown();
    }

    const PANIC_PROBE: &str = "APLAN_TUI_CAUGHT_PANIC_PROBE";

    fn exercise_caught_panics() {
        let hook_calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&hook_calls);
        std::panic::set_hook(Box::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        }));

        // Two runtimes prove that the one-time dispatcher protects later
        // workers too rather than depending on a temporary hook swap.
        for id in 1..=2 {
            let mut runtime = WorkerRuntime::new(Arc::new(PanicExecutor));
            runtime.submit(request(id, EffectKind::Refresh));
            let completion = loop {
                if let Some(RuntimeEvent::Completed(completion)) =
                    runtime.drain().into_iter().next()
                {
                    break completion;
                }
                thread::yield_now();
            };
            assert_eq!(completion.outcome, Err(EffectFailure::WorkerPanicked));
            runtime.shutdown();
        }
        assert_eq!(hook_calls.load(Ordering::SeqCst), 0);

        let _ = catch_unwind(AssertUnwindSafe(|| panic!("ordinary panic")));
        assert_eq!(
            hook_calls.load(Ordering::SeqCst),
            1,
            "a panic outside the effect worker still reaches the previous hook"
        );
    }

    /// Proof for `tui.caught_worker_panics_do_not_write_terminal`: the child
    /// distinguishes typed recovery from a false pass caused by libtest
    /// capturing the panic hook's stderr.
    #[test]
    fn a_worker_panic_is_a_typed_completion() {
        if std::env::var_os(PANIC_PROBE).is_some() {
            exercise_caught_panics();
            return;
        }

        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "worker::tests::a_worker_panic_is_a_typed_completion",
                "--nocapture",
            ])
            .env(PANIC_PROBE, "1")
            .output()
            .expect("isolated panic probe");
        assert!(
            output.status.success(),
            "probe failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "a caught worker panic leaked through the process hook: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Proof for `tui.event_loop_is_generation_safe`: shutdown joins the work
    /// that started and cancels the bounded successor.
    #[test]
    fn shutdown_joins_running_work_and_drops_work_that_never_started() {
        let (starts_tx, starts_rx) = channel();
        let (release_tx, release_rx) = channel();
        let executor = Arc::new(BlockingExecutor {
            starts: starts_tx,
            releases: Mutex::new(release_rx),
            concurrent: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let mut runtime = WorkerRuntime::new(executor);
        runtime.submit(request(1, EffectKind::Refresh));
        starts_rx.recv().expect("worker started");
        runtime.submit(request(2, EffectKind::Verify));

        let (done_tx, done_rx) = channel();
        let joiner = thread::spawn(move || {
            runtime.shutdown();
            done_tx.send(()).expect("shutdown observer");
        });
        assert!(done_rx.try_recv().is_err(), "shutdown must not detach work");
        release_tx.send(()).expect("release worker");
        done_rx.recv().expect("shutdown finished");
        joiner.join().expect("join shutdown observer");
        assert!(starts_rx.try_recv().is_err(), "queued work was cancelled");
    }

    /// Proof for `tui.termination_preempts_worker_shutdown`: an unfinished
    /// worker is observed without entering `join`, leaving the terminal thread
    /// free to consume a lifecycle event.
    #[test]
    fn shutdown_polling_never_joins_a_running_worker() {
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        let mut runtime = WorkerRuntime::new(Arc::new(BlockingExecutor {
            starts: started_tx,
            releases: Mutex::new(release_rx),
            concurrent: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }));
        assert_eq!(
            runtime.submit(request(1, EffectKind::Refresh)),
            Schedule::Started
        );
        started_rx.recv().expect("worker started");
        assert_eq!(
            runtime.submit(request(2, EffectKind::Verify)),
            Schedule::Queued
        );

        assert!(
            !runtime.finish_shutdown_if_ready(),
            "shutdown polling must return to lifecycle handling while work is active"
        );
        release_tx.send(()).expect("release worker");
        while !runtime.finish_shutdown_if_ready() {
            std::thread::yield_now();
        }
        assert!(!runtime.has_work());
        assert!(
            started_rx.try_recv().is_err(),
            "shutdown polling must discard the queued successor"
        );
    }

    /// Proof for `tui.verification_is_typed_and_non_blocking`: cancelling the
    /// bounded successor removes it before provider code can start.
    #[test]
    fn cancelling_a_queued_effect_prevents_it_from_starting() {
        let (starts_tx, starts_rx) = channel();
        let (release_tx, release_rx) = channel();
        let executor = Arc::new(BlockingExecutor {
            starts: starts_tx,
            releases: Mutex::new(release_rx),
            concurrent: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let mut runtime = WorkerRuntime::new(executor);
        assert_eq!(
            runtime.submit(request(1, EffectKind::Refresh)),
            Schedule::Started
        );
        starts_rx.recv().expect("first worker started");
        assert_eq!(
            runtime.submit(request(2, EffectKind::Verify)),
            Schedule::Queued
        );
        assert!(runtime.cancel_queued(EffectId(2)));
        assert!(!runtime.cancel_queued(EffectId(2)));

        release_tx.send(()).expect("release active worker");
        while runtime.drain().is_empty() {
            thread::yield_now();
        }
        assert!(starts_rx.try_recv().is_err(), "cancelled work started");
        runtime.shutdown();
    }

    /// Proof for `tui.worker_cleanup_is_unwind_safe`: dropping the runtime is
    /// the path taken when an unwind bypasses explicit shutdown.
    #[test]
    fn dropping_the_runtime_joins_running_work_and_drops_work_that_never_started() {
        let (starts_tx, starts_rx) = channel();
        let (release_tx, release_rx) = channel();
        let executor = Arc::new(BlockingExecutor {
            starts: starts_tx,
            releases: Mutex::new(release_rx),
            concurrent: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let mut runtime = WorkerRuntime::new(executor);
        runtime.submit(request(1, EffectKind::Refresh));
        starts_rx.recv().expect("worker started");
        runtime.submit(request(2, EffectKind::Verify));

        let (drop_tx, drop_rx) = channel();
        let dropper = thread::spawn(move || {
            drop(runtime);
            drop_tx.send(()).expect("drop observer");
        });
        assert_eq!(
            drop_rx.recv_timeout(std::time::Duration::from_millis(250)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout),
            "dropping the runtime must not detach running work"
        );
        release_tx.send(()).expect("release worker");
        drop_rx.recv().expect("drop finished");
        dropper.join().expect("join drop observer");
        assert!(starts_rx.try_recv().is_err(), "queued work was cancelled");
    }
}
