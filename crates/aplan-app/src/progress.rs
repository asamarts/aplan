//! Typed progress at the application boundary.
//!
//! The application reports facts about work. Delivery adapters decide whether
//! and how to draw them. In particular, no value in this module contains a
//! path, task body, provider output or terminal control sequence.

use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Identity of one operation generation.
///
/// A host chooses the value. Reusing a visible control's identity (for example
/// a TUI effect id) lets that host reject an event from an operation it has
/// already replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(u64);

impl OperationId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable identity of independently progressing work inside an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgressChildId(u64);

impl ProgressChildId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The application operation being observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    CliCommand,
    PlanLoad,
    Verification,
    TestCatalogue,
}

impl OperationKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::CliCommand => "command",
            Self::PlanLoad => "plan load",
            Self::Verification => "verification",
            Self::TestCatalogue => "test catalogue",
        }
    }
}

/// A bounded, privacy-safe vocabulary for application work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProgressPhase {
    BuildIdentification,
    ConfigurationDiscovery,
    CacheLookup,
    SourceEnumeration,
    SourceParsing,
    SnapshotConfirmation,
    CachePublication,
    StructuralValidation,
    RepositoryObservation,
    VerificationPreparation,
    EvidenceProvider,
    Projection,
}

impl ProgressPhase {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::BuildIdentification => "identifying this build",
            Self::ConfigurationDiscovery => "selecting configuration",
            Self::CacheLookup => "checking the plan cache",
            Self::SourceEnumeration => "finding plan sources",
            Self::SourceParsing => "reading plan sources",
            Self::SnapshotConfirmation => "confirming the plan snapshot",
            Self::CachePublication => "publishing the plan cache",
            Self::StructuralValidation => "validating the plan",
            Self::RepositoryObservation => "observing repository evidence",
            Self::VerificationPreparation => "preparing verification inputs",
            Self::EvidenceProvider => "querying evidence providers",
            Self::Projection => "building the requested view",
        }
    }
}

/// Cooperative cancellation shared by one application operation.
///
/// Cancellation is deliberately a fact that work polls, not an assumption
/// that dropping a thread stopped filesystem or repository access. Clones
/// identify the same operation and carry no terminal or presentation state.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Ask the operation to stop at its next safe boundary.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether cancellation has been requested for this operation.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Convert a cancellation request into the conventional interrupted I/O
    /// result used by source-loading workers.
    pub fn check(&self) -> std::io::Result<()> {
        if self.is_cancelled() {
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "operation cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

/// What a defensible bounded total counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressUnit {
    Sources,
    Providers,
    Items,
}

/// The amount of work known at one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMeasure {
    /// Activity is known, but no honest total is available.
    Indeterminate,
    /// A monotonic count against a stable total.
    Bounded {
        completed: u64,
        total: u64,
        unit: ProgressUnit,
    },
}

impl ProgressMeasure {
    #[must_use]
    pub fn bounded(completed: usize, total: usize, unit: ProgressUnit) -> Self {
        Self::Bounded {
            completed: u64::try_from(completed).unwrap_or(u64::MAX),
            total: u64::try_from(total).unwrap_or(u64::MAX),
            unit,
        }
    }
}

/// The subject whose lifecycle or measure changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProgressSubject {
    Operation,
    Phase {
        phase: ProgressPhase,
        child: Option<ProgressChildId>,
    },
}

/// A lifecycle transition. Presentation must never infer one from text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressLifecycle {
    Queued,
    Started,
    Advanced,
    Completed,
    Failed,
    Cancelled,
}

/// One ordered operation event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressEvent {
    pub operation: OperationId,
    pub sequence: u64,
    pub kind: OperationKind,
    pub subject: ProgressSubject,
    pub lifecycle: ProgressLifecycle,
    pub measure: ProgressMeasure,
}

/// Why a consumer refused an event instead of corrupting visible state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressRejection {
    WrongOperation,
    WrongKind,
    OutOfSequence,
    InvalidLifecycle,
    InvalidMeasure,
    Regression,
    AlreadyTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubjectState {
    started: bool,
    terminal: bool,
    measure: ProgressMeasure,
}

/// Validated consumer state for exactly one operation generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressState {
    operation: OperationId,
    kind: OperationKind,
    next_sequence: u64,
    queued: bool,
    started: bool,
    terminal: bool,
    subjects: BTreeMap<ProgressSubject, SubjectState>,
}

impl ProgressState {
    #[must_use]
    pub fn new(operation: OperationId, kind: OperationKind) -> Self {
        Self {
            operation,
            kind,
            next_sequence: 0,
            queued: false,
            started: false,
            terminal: false,
            subjects: BTreeMap::new(),
        }
    }

    /// Apply one event only when its identity, order, lifecycle and measure all
    /// agree with the state already accepted.
    pub fn accept(&mut self, event: ProgressEvent) -> Result<(), ProgressRejection> {
        if event.operation != self.operation {
            return Err(ProgressRejection::WrongOperation);
        }
        if event.kind != self.kind {
            return Err(ProgressRejection::WrongKind);
        }
        if event.sequence != self.next_sequence {
            return Err(ProgressRejection::OutOfSequence);
        }
        if self.terminal {
            return Err(ProgressRejection::AlreadyTerminal);
        }
        validate_measure(event.measure)?;
        match event.subject {
            ProgressSubject::Operation => self.accept_operation(event)?,
            subject @ ProgressSubject::Phase { .. } => self.accept_subject(subject, event)?,
        }
        self.next_sequence += 1;
        Ok(())
    }

    fn accept_operation(&mut self, event: ProgressEvent) -> Result<(), ProgressRejection> {
        if event.measure != ProgressMeasure::Indeterminate {
            return Err(ProgressRejection::InvalidMeasure);
        }
        match event.lifecycle {
            ProgressLifecycle::Queued if !self.queued && !self.started => self.queued = true,
            ProgressLifecycle::Started if self.queued && !self.started => self.started = true,
            ProgressLifecycle::Completed
            | ProgressLifecycle::Failed
            | ProgressLifecycle::Cancelled
                if self.started =>
            {
                self.terminal = true;
            }
            _ => return Err(ProgressRejection::InvalidLifecycle),
        }
        Ok(())
    }

    fn accept_subject(
        &mut self,
        subject: ProgressSubject,
        event: ProgressEvent,
    ) -> Result<(), ProgressRejection> {
        if !self.started {
            return Err(ProgressRejection::InvalidLifecycle);
        }
        match event.lifecycle {
            ProgressLifecycle::Started => {
                if self.subjects.contains_key(&subject) {
                    return Err(ProgressRejection::InvalidLifecycle);
                }
                self.subjects.insert(
                    subject,
                    SubjectState {
                        started: true,
                        terminal: false,
                        measure: event.measure,
                    },
                );
            }
            ProgressLifecycle::Advanced => {
                let state = self
                    .subjects
                    .get_mut(&subject)
                    .ok_or(ProgressRejection::InvalidLifecycle)?;
                if !state.started || state.terminal {
                    return Err(ProgressRejection::InvalidLifecycle);
                }
                validate_monotonic(state.measure, event.measure)?;
                state.measure = event.measure;
            }
            ProgressLifecycle::Completed
            | ProgressLifecycle::Failed
            | ProgressLifecycle::Cancelled => {
                let state = self
                    .subjects
                    .get_mut(&subject)
                    .ok_or(ProgressRejection::InvalidLifecycle)?;
                if !state.started || state.terminal {
                    return Err(ProgressRejection::InvalidLifecycle);
                }
                validate_monotonic(state.measure, event.measure)?;
                state.measure = event.measure;
                state.terminal = true;
            }
            ProgressLifecycle::Queued => return Err(ProgressRejection::InvalidLifecycle),
        }
        Ok(())
    }
}

fn validate_measure(measure: ProgressMeasure) -> Result<(), ProgressRejection> {
    match measure {
        ProgressMeasure::Indeterminate => Ok(()),
        ProgressMeasure::Bounded {
            completed, total, ..
        } if completed <= total => Ok(()),
        ProgressMeasure::Bounded { .. } => Err(ProgressRejection::InvalidMeasure),
    }
}

fn validate_monotonic(
    previous: ProgressMeasure,
    next: ProgressMeasure,
) -> Result<(), ProgressRejection> {
    match (previous, next) {
        (ProgressMeasure::Indeterminate, ProgressMeasure::Indeterminate) => Ok(()),
        (
            ProgressMeasure::Bounded {
                completed: old,
                total: old_total,
                unit: old_unit,
            },
            ProgressMeasure::Bounded {
                completed: new,
                total: new_total,
                unit: new_unit,
            },
        ) if old_total == new_total && old_unit == new_unit && old <= new => Ok(()),
        _ => Err(ProgressRejection::Regression),
    }
}

/// Ordered event producer for one operation generation.
pub struct ProgressEmitter<'a> {
    operation: OperationId,
    kind: OperationKind,
    sequence: u64,
    terminal: bool,
    sink: &'a mut dyn FnMut(ProgressEvent),
}

impl std::fmt::Debug for ProgressEmitter<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProgressEmitter")
            .field("operation", &self.operation)
            .field("kind", &self.kind)
            .field("sequence", &self.sequence)
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

impl<'a> ProgressEmitter<'a> {
    pub fn begin(
        operation: OperationId,
        kind: OperationKind,
        sink: &'a mut dyn FnMut(ProgressEvent),
    ) -> Self {
        let mut emitter = Self {
            operation,
            kind,
            sequence: 0,
            terminal: false,
            sink,
        };
        emitter.emit(
            ProgressSubject::Operation,
            ProgressLifecycle::Queued,
            ProgressMeasure::Indeterminate,
        );
        emitter.emit(
            ProgressSubject::Operation,
            ProgressLifecycle::Started,
            ProgressMeasure::Indeterminate,
        );
        emitter
    }

    pub fn phase_started(
        &mut self,
        phase: ProgressPhase,
        child: Option<ProgressChildId>,
        measure: ProgressMeasure,
    ) {
        self.emit(
            ProgressSubject::Phase { phase, child },
            ProgressLifecycle::Started,
            measure,
        );
    }

    pub fn phase_advanced(
        &mut self,
        phase: ProgressPhase,
        child: Option<ProgressChildId>,
        measure: ProgressMeasure,
    ) {
        self.emit(
            ProgressSubject::Phase { phase, child },
            ProgressLifecycle::Advanced,
            measure,
        );
    }

    pub fn phase_completed(
        &mut self,
        phase: ProgressPhase,
        child: Option<ProgressChildId>,
        measure: ProgressMeasure,
    ) {
        self.emit(
            ProgressSubject::Phase { phase, child },
            ProgressLifecycle::Completed,
            measure,
        );
    }

    pub fn phase_failed(
        &mut self,
        phase: ProgressPhase,
        child: Option<ProgressChildId>,
        measure: ProgressMeasure,
    ) {
        self.emit(
            ProgressSubject::Phase { phase, child },
            ProgressLifecycle::Failed,
            measure,
        );
    }

    pub fn phase_cancelled(
        &mut self,
        phase: ProgressPhase,
        child: Option<ProgressChildId>,
        measure: ProgressMeasure,
    ) {
        self.emit(
            ProgressSubject::Phase { phase, child },
            ProgressLifecycle::Cancelled,
            measure,
        );
    }

    pub fn complete(&mut self) {
        self.terminal(ProgressLifecycle::Completed);
    }

    pub fn fail(&mut self) {
        self.terminal(ProgressLifecycle::Failed);
    }

    pub fn cancel(&mut self) {
        self.terminal(ProgressLifecycle::Cancelled);
    }

    fn terminal(&mut self, lifecycle: ProgressLifecycle) {
        if !self.terminal {
            self.emit(
                ProgressSubject::Operation,
                lifecycle,
                ProgressMeasure::Indeterminate,
            );
            self.terminal = true;
        }
    }

    fn emit(
        &mut self,
        subject: ProgressSubject,
        lifecycle: ProgressLifecycle,
        measure: ProgressMeasure,
    ) {
        (self.sink)(ProgressEvent {
            operation: self.operation,
            sequence: self.sequence,
            kind: self.kind,
            subject,
            lifecycle,
            measure,
        });
        self.sequence = self.sequence.saturating_add(1);
    }
}

impl Drop for ProgressEmitter<'_> {
    fn drop(&mut self) {
        // During an unwind, invoking user code can double-panic and abort the
        // process. The host's panic boundary owns that terminal outcome.
        if !self.terminal && !std::thread::panicking() {
            self.cancel();
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct CatalogueProgressAdapter {
    aggregate_started: bool,
}

impl CatalogueProgressAdapter {
    pub(crate) fn accept(
        &mut self,
        emitter: &mut ProgressEmitter<'_>,
        event: &aplan_evidence::CatalogueProgressEvent,
    ) {
        match event {
            aplan_evidence::CatalogueProgressEvent::Started {
                index,
                completed,
                remaining,
                ..
            } => {
                let total = completed.saturating_add(*remaining);
                if !self.aggregate_started {
                    emitter.phase_started(
                        ProgressPhase::EvidenceProvider,
                        None,
                        ProgressMeasure::bounded(*completed, total, ProgressUnit::Providers),
                    );
                    self.aggregate_started = true;
                }
                emitter.phase_started(
                    ProgressPhase::EvidenceProvider,
                    Some(ProgressChildId::new(
                        u64::try_from(*index).unwrap_or(u64::MAX),
                    )),
                    ProgressMeasure::Indeterminate,
                );
            }
            aplan_evidence::CatalogueProgressEvent::Finished {
                index,
                completed,
                remaining,
                ..
            } => {
                emitter.phase_completed(
                    ProgressPhase::EvidenceProvider,
                    Some(ProgressChildId::new(
                        u64::try_from(*index).unwrap_or(u64::MAX),
                    )),
                    ProgressMeasure::Indeterminate,
                );
                let total = completed.saturating_add(*remaining);
                let measure = ProgressMeasure::bounded(*completed, total, ProgressUnit::Providers);
                if *remaining == 0 {
                    emitter.phase_completed(ProgressPhase::EvidenceProvider, None, measure);
                } else {
                    emitter.phase_advanced(ProgressPhase::EvidenceProvider, None, measure);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_clones_share_one_monotonic_request() {
        let token = CancellationToken::default();
        let observer = token.clone();
        assert!(!token.is_cancelled());
        assert!(observer.check().is_ok());
        token.cancel();
        assert!(observer.is_cancelled());
        assert_eq!(
            observer.check().expect_err("cancelled").kind(),
            std::io::ErrorKind::Interrupted
        );
        observer.cancel();
        assert!(
            token.is_cancelled(),
            "a repeated request cannot revive work"
        );
    }

    #[test]
    fn public_progress_vocabulary_and_cancelled_emission_are_complete() {
        assert_eq!(OperationId::new(17).get(), 17);
        assert_eq!(ProgressChildId::new(23).get(), 23);
        assert_eq!(
            [
                OperationKind::CliCommand,
                OperationKind::PlanLoad,
                OperationKind::Verification,
                OperationKind::TestCatalogue,
            ]
            .map(OperationKind::label),
            ["command", "plan load", "verification", "test catalogue"]
        );
        assert_eq!(
            [
                ProgressPhase::BuildIdentification,
                ProgressPhase::ConfigurationDiscovery,
                ProgressPhase::CacheLookup,
                ProgressPhase::SourceEnumeration,
                ProgressPhase::SourceParsing,
                ProgressPhase::SnapshotConfirmation,
                ProgressPhase::CachePublication,
                ProgressPhase::StructuralValidation,
                ProgressPhase::RepositoryObservation,
                ProgressPhase::VerificationPreparation,
                ProgressPhase::EvidenceProvider,
                ProgressPhase::Projection,
            ]
            .map(ProgressPhase::label),
            [
                "identifying this build",
                "selecting configuration",
                "checking the plan cache",
                "finding plan sources",
                "reading plan sources",
                "confirming the plan snapshot",
                "publishing the plan cache",
                "validating the plan",
                "observing repository evidence",
                "preparing verification inputs",
                "querying evidence providers",
                "building the requested view",
            ]
        );

        let mut events = Vec::new();
        {
            let mut sink = |event| events.push(event);
            let mut emitter =
                ProgressEmitter::begin(OperationId::new(31), OperationKind::PlanLoad, &mut sink);
            assert!(format!("{emitter:?}").contains("ProgressEmitter"));
            emitter.phase_started(
                ProgressPhase::SourceParsing,
                None,
                ProgressMeasure::Indeterminate,
            );
            emitter.phase_cancelled(
                ProgressPhase::SourceParsing,
                None,
                ProgressMeasure::Indeterminate,
            );
            emitter.cancel();
            emitter.complete();
        }
        assert_eq!(
            events
                .iter()
                .map(|event| event.lifecycle)
                .collect::<Vec<_>>(),
            [
                ProgressLifecycle::Queued,
                ProgressLifecycle::Started,
                ProgressLifecycle::Started,
                ProgressLifecycle::Cancelled,
                ProgressLifecycle::Cancelled,
            ]
        );
    }

    fn event(
        operation: u64,
        sequence: u64,
        subject: ProgressSubject,
        lifecycle: ProgressLifecycle,
        measure: ProgressMeasure,
    ) -> ProgressEvent {
        ProgressEvent {
            operation: OperationId::new(operation),
            sequence,
            kind: OperationKind::PlanLoad,
            subject,
            lifecycle,
            measure,
        }
    }

    #[test]
    fn reducer_accepts_a_complete_monotonic_lifecycle() {
        let phase = ProgressSubject::Phase {
            phase: ProgressPhase::SourceParsing,
            child: Some(ProgressChildId::new(0)),
        };
        let mut state = ProgressState::new(OperationId::new(7), OperationKind::PlanLoad);
        let events = [
            event(
                7,
                0,
                ProgressSubject::Operation,
                ProgressLifecycle::Queued,
                ProgressMeasure::Indeterminate,
            ),
            event(
                7,
                1,
                ProgressSubject::Operation,
                ProgressLifecycle::Started,
                ProgressMeasure::Indeterminate,
            ),
            event(
                7,
                2,
                phase,
                ProgressLifecycle::Started,
                ProgressMeasure::bounded(0, 2, ProgressUnit::Sources),
            ),
            event(
                7,
                3,
                phase,
                ProgressLifecycle::Advanced,
                ProgressMeasure::bounded(1, 2, ProgressUnit::Sources),
            ),
            event(
                7,
                4,
                phase,
                ProgressLifecycle::Completed,
                ProgressMeasure::bounded(2, 2, ProgressUnit::Sources),
            ),
            event(
                7,
                5,
                ProgressSubject::Operation,
                ProgressLifecycle::Completed,
                ProgressMeasure::Indeterminate,
            ),
        ];
        for event in events {
            state.accept(event).expect("valid lifecycle");
        }
    }

    /// Proof for `progress.events_are_generation_safe`.
    #[test]
    fn progress_events_are_generation_safe() {
        let phase = ProgressSubject::Phase {
            phase: ProgressPhase::SourceParsing,
            child: None,
        };
        let mut wrong = ProgressState::new(OperationId::new(1), OperationKind::PlanLoad);
        assert_eq!(
            wrong.accept(event(
                2,
                0,
                ProgressSubject::Operation,
                ProgressLifecycle::Queued,
                ProgressMeasure::Indeterminate
            )),
            Err(ProgressRejection::WrongOperation)
        );

        let mut state = ProgressState::new(OperationId::new(1), OperationKind::PlanLoad);
        state
            .accept(event(
                1,
                0,
                ProgressSubject::Operation,
                ProgressLifecycle::Queued,
                ProgressMeasure::Indeterminate,
            ))
            .unwrap();
        assert_eq!(
            state.accept(event(
                1,
                2,
                ProgressSubject::Operation,
                ProgressLifecycle::Started,
                ProgressMeasure::Indeterminate
            )),
            Err(ProgressRejection::OutOfSequence)
        );
        state
            .accept(event(
                1,
                1,
                ProgressSubject::Operation,
                ProgressLifecycle::Started,
                ProgressMeasure::Indeterminate,
            ))
            .unwrap();
        state
            .accept(event(
                1,
                2,
                phase,
                ProgressLifecycle::Started,
                ProgressMeasure::bounded(1, 2, ProgressUnit::Sources),
            ))
            .unwrap();
        assert_eq!(
            state.accept(event(
                1,
                3,
                phase,
                ProgressLifecycle::Advanced,
                ProgressMeasure::bounded(0, 2, ProgressUnit::Sources)
            )),
            Err(ProgressRejection::Regression)
        );
        state
            .accept(event(
                1,
                3,
                phase,
                ProgressLifecycle::Completed,
                ProgressMeasure::bounded(2, 2, ProgressUnit::Sources),
            ))
            .unwrap();
        state
            .accept(event(
                1,
                4,
                ProgressSubject::Operation,
                ProgressLifecycle::Completed,
                ProgressMeasure::Indeterminate,
            ))
            .unwrap();
        assert_eq!(
            state.accept(event(
                1,
                5,
                ProgressSubject::Operation,
                ProgressLifecycle::Completed,
                ProgressMeasure::Indeterminate
            )),
            Err(ProgressRejection::AlreadyTerminal)
        );
    }

    #[test]
    fn reducer_rejects_invalid_identity_and_lifecycle_neighbours() {
        let operation = OperationId::new(5);
        let mut state = ProgressState::new(operation, OperationKind::PlanLoad);
        let wrong_kind = ProgressEvent {
            kind: OperationKind::Verification,
            ..event(
                5,
                0,
                ProgressSubject::Operation,
                ProgressLifecycle::Queued,
                ProgressMeasure::Indeterminate,
            )
        };
        assert_eq!(state.accept(wrong_kind), Err(ProgressRejection::WrongKind));
        assert_eq!(
            state.accept(event(
                5,
                0,
                ProgressSubject::Phase {
                    phase: ProgressPhase::SourceParsing,
                    child: None,
                },
                ProgressLifecycle::Started,
                ProgressMeasure::Indeterminate,
            )),
            Err(ProgressRejection::InvalidLifecycle)
        );
        state
            .accept(event(
                5,
                0,
                ProgressSubject::Operation,
                ProgressLifecycle::Queued,
                ProgressMeasure::Indeterminate,
            ))
            .unwrap();
        assert_eq!(
            state.accept(event(
                5,
                1,
                ProgressSubject::Operation,
                ProgressLifecycle::Started,
                ProgressMeasure::bounded(0, 1, ProgressUnit::Items),
            )),
            Err(ProgressRejection::InvalidMeasure)
        );
        state
            .accept(event(
                5,
                1,
                ProgressSubject::Operation,
                ProgressLifecycle::Started,
                ProgressMeasure::Indeterminate,
            ))
            .unwrap();
        let phase = ProgressSubject::Phase {
            phase: ProgressPhase::SourceParsing,
            child: None,
        };
        state
            .accept(event(
                5,
                2,
                phase,
                ProgressLifecycle::Started,
                ProgressMeasure::bounded(0, 2, ProgressUnit::Sources),
            ))
            .unwrap();
        assert_eq!(
            state.accept(event(
                5,
                3,
                phase,
                ProgressLifecycle::Started,
                ProgressMeasure::bounded(0, 2, ProgressUnit::Sources),
            )),
            Err(ProgressRejection::InvalidLifecycle)
        );
    }

    #[test]
    fn reducer_rejects_invalid_and_regressing_measure_neighbours() {
        let phase = ProgressSubject::Phase {
            phase: ProgressPhase::SourceParsing,
            child: None,
        };
        let mut state = ProgressState::new(OperationId::new(6), OperationKind::PlanLoad);
        for (sequence, lifecycle) in [
            (0, ProgressLifecycle::Queued),
            (1, ProgressLifecycle::Started),
        ] {
            state
                .accept(event(
                    6,
                    sequence,
                    ProgressSubject::Operation,
                    lifecycle,
                    ProgressMeasure::Indeterminate,
                ))
                .unwrap();
        }
        assert_eq!(
            state.accept(event(
                6,
                2,
                phase,
                ProgressLifecycle::Started,
                ProgressMeasure::bounded(2, 1, ProgressUnit::Sources),
            )),
            Err(ProgressRejection::InvalidMeasure)
        );
        state
            .accept(event(
                6,
                2,
                phase,
                ProgressLifecycle::Started,
                ProgressMeasure::bounded(0, 2, ProgressUnit::Sources),
            ))
            .unwrap();
        for changed_measure in [
            ProgressMeasure::bounded(1, 3, ProgressUnit::Sources),
            ProgressMeasure::bounded(1, 2, ProgressUnit::Items),
            ProgressMeasure::Indeterminate,
        ] {
            assert_eq!(
                state.accept(event(
                    6,
                    3,
                    phase,
                    ProgressLifecycle::Advanced,
                    changed_measure,
                )),
                Err(ProgressRejection::Regression)
            );
        }
    }

    #[test]
    fn emitter_numbers_events_and_cancels_an_abandoned_operation() {
        let mut events = Vec::new();
        {
            let mut sink = |event| events.push(event);
            let mut emitter =
                ProgressEmitter::begin(OperationId::new(9), OperationKind::Verification, &mut sink);
            emitter.phase_started(
                ProgressPhase::RepositoryObservation,
                None,
                ProgressMeasure::Indeterminate,
            );
        }
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            events.last().map(|event| event.lifecycle),
            Some(ProgressLifecycle::Cancelled)
        );
    }

    #[test]
    fn reducer_accepts_explicit_failed_and_cancelled_terminal_paths() {
        for terminal in [ProgressLifecycle::Failed, ProgressLifecycle::Cancelled] {
            let operation = OperationId::new(if terminal == ProgressLifecycle::Failed {
                21
            } else {
                22
            });
            let phase = ProgressSubject::Phase {
                phase: ProgressPhase::RepositoryObservation,
                child: None,
            };
            let mut state = ProgressState::new(operation, OperationKind::PlanLoad);
            for (sequence, subject, lifecycle) in [
                (0, ProgressSubject::Operation, ProgressLifecycle::Queued),
                (1, ProgressSubject::Operation, ProgressLifecycle::Started),
                (2, phase, ProgressLifecycle::Started),
                (3, phase, terminal),
                (4, ProgressSubject::Operation, terminal),
            ] {
                state
                    .accept(ProgressEvent {
                        operation,
                        sequence,
                        kind: OperationKind::PlanLoad,
                        subject,
                        lifecycle,
                        measure: ProgressMeasure::Indeterminate,
                    })
                    .expect("valid terminal lifecycle");
            }
        }
    }

    #[test]
    fn emitter_does_not_invoke_a_user_sink_while_unwinding() {
        let mut events = Vec::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut sink = |event| events.push(event);
            let _emitter = ProgressEmitter::begin(
                OperationId::new(30),
                OperationKind::Verification,
                &mut sink,
            );
            panic!("exercise unwind cleanup");
        }));
        assert!(result.is_err());
        assert_eq!(
            events
                .iter()
                .map(|event| event.lifecycle)
                .collect::<Vec<_>>(),
            vec![ProgressLifecycle::Queued, ProgressLifecycle::Started],
            "Drop must not call a user sink while another panic is active"
        );
    }

    #[test]
    fn catalogue_adapter_preserves_concurrent_child_identity_and_aggregate_counts() {
        let operation = OperationId::new(40);
        let mut events = Vec::new();
        {
            let mut sink = |event| events.push(event);
            let mut emitter =
                ProgressEmitter::begin(operation, OperationKind::TestCatalogue, &mut sink);
            let mut adapter = CatalogueProgressAdapter::default();
            for event in [
                aplan_evidence::CatalogueProgressEvent::Started {
                    index: 1,
                    label: "private-one".to_string(),
                    completed: 0,
                    remaining: 2,
                },
                aplan_evidence::CatalogueProgressEvent::Started {
                    index: 2,
                    label: "private-two".to_string(),
                    completed: 0,
                    remaining: 2,
                },
                aplan_evidence::CatalogueProgressEvent::Finished {
                    index: 2,
                    label: "private-two".to_string(),
                    completed: 1,
                    remaining: 1,
                    elapsed_millis: 3,
                },
                aplan_evidence::CatalogueProgressEvent::Finished {
                    index: 1,
                    label: "private-one".to_string(),
                    completed: 2,
                    remaining: 0,
                    elapsed_millis: 4,
                },
            ] {
                adapter.accept(&mut emitter, &event);
            }
            emitter.complete();
        }

        let mut state = ProgressState::new(operation, OperationKind::TestCatalogue);
        for event in events.iter().copied() {
            state.accept(event).expect("interleaved progress is valid");
        }
        let children = events
            .iter()
            .filter_map(|event| match event.subject {
                ProgressSubject::Phase {
                    phase: ProgressPhase::EvidenceProvider,
                    child: Some(child),
                } if event.lifecycle == ProgressLifecycle::Started => Some(child.get()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(children, vec![1, 2]);
        assert!(events.iter().any(|event| {
            event.subject
                == ProgressSubject::Phase {
                    phase: ProgressPhase::EvidenceProvider,
                    child: None,
                }
                && event.lifecycle == ProgressLifecycle::Completed
                && event.measure == ProgressMeasure::bounded(2, 2, ProgressUnit::Providers)
        }));
    }
}
