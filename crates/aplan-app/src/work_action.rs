//! Contextual claim and work-state mutations for interactive clients.
//!
//! Claim state is local coordination rather than plan data, so it cannot be
//! smuggled through a task-file [`crate::ChangeSet`]. This module gives both
//! stores one review/commit contract while preserving their distinct guarded
//! writers: complete-record compare-and-swap for claims and the shared plan
//! publisher for durable status.

use aplan_core::{ClaimRecord, ClaimScope, ClaimSnapshot, Graph, Status};

use crate::{
    ChangeEvidence, ChangeSet, CommitOutcome, EvidenceReview, FileChange, InputIdentity,
    PlanContext, SemanticEffect,
};

/// The four high-frequency work actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkActionKind {
    Claim,
    Release,
    Start,
    Stop,
}

impl WorkActionKind {
    pub const ALL: [Self; 4] = [Self::Claim, Self::Release, Self::Start, Self::Stop];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Release => "release",
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Claim => 0,
            Self::Release => 1,
            Self::Start => 2,
            Self::Stop => 3,
        }
    }
}

/// One exact request, before repository-aware preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkActionRequest {
    pub task: String,
    pub action: WorkActionKind,
    /// Explicit claim identity. `None` uses the same host precedence as CLI.
    pub as_name: Option<String>,
    /// The holder an intentional takeover expects to displace.
    pub from: Option<String>,
    /// Explicit clock for deterministic clients and tests. `None` reads now.
    pub claimed_at_epoch_seconds: Option<i64>,
}

impl WorkActionRequest {
    #[must_use]
    pub fn simple(task: impl Into<String>, action: WorkActionKind) -> Self {
        Self {
            task: task.into(),
            action,
            as_name: None,
            from: None,
            claimed_at_epoch_seconds: None,
        }
    }
}

/// Whether a contextual action should be offered for the selected task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkActionAvailability {
    Available { request: WorkActionRequest },
    Unavailable { reason: String },
}

impl WorkActionAvailability {
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }
}

/// One stable action row. Every kind is present exactly once so disabled
/// reasons remain discoverable without a static always-enabled menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkActionEntry {
    pub action: WorkActionKind,
    pub availability: WorkActionAvailability,
}

/// Context used to choose and explain quick actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkActionCatalogue {
    pub input_sha256: String,
    pub task: String,
    pub status: Status,
    pub ready: bool,
    pub actor: Option<String>,
    /// Why no effective actor can be used for identity-bearing actions.
    /// Release and stop remain independent of this failure.
    pub actor_error: Option<String>,
    pub claim: Option<ClaimRecord>,
    pub coordination: ClaimScope,
    actions: [WorkActionEntry; 4],
}

impl WorkActionCatalogue {
    #[must_use]
    pub const fn actions(&self) -> &[WorkActionEntry; 4] {
        &self.actions
    }

    #[must_use]
    pub const fn action(&self, kind: WorkActionKind) -> &WorkActionEntry {
        &self.actions[kind.index()]
    }
}

/// Exact claim-store delta retained through confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimPreview {
    pub before: Option<ClaimRecord>,
    pub after: Option<ClaimRecord>,
    pub scope: ClaimScope,
    pub warnings: Vec<String>,
}

/// The reviewed store-specific change.
#[derive(Debug)]
pub enum WorkActionChange {
    Coordination(Box<ClaimPreview>),
    Plan(Box<ChangeSet>),
}

/// One immutable work-action preview.
#[derive(Debug)]
pub struct WorkActionPreview {
    request: WorkActionRequest,
    input: InputIdentity,
    change: WorkActionChange,
}

impl WorkActionPreview {
    #[must_use]
    pub const fn request(&self) -> &WorkActionRequest {
        &self.request
    }

    #[must_use]
    pub const fn input(&self) -> &InputIdentity {
        &self.input
    }

    #[must_use]
    pub const fn change(&self) -> &WorkActionChange {
        &self.change
    }

    #[must_use]
    pub fn change_set(&self) -> Option<&ChangeSet> {
        match &self.change {
            WorkActionChange::Coordination(_) => None,
            WorkActionChange::Plan(change) => Some(change.as_ref()),
        }
    }
}

/// Result of committing one coordination preview.
#[derive(Debug)]
pub enum CoordinationOutcome {
    Applied(WorkActionPreview),
    NoOp(WorkActionPreview),
    Stale {
        preview: WorkActionPreview,
        actual: ClaimSnapshot,
    },
    Failed {
        preview: WorkActionPreview,
        error: crate::claims::ClaimError,
    },
}

/// Result of confirming a work action. Plan mutations retain the shared
/// [`CommitOutcome`] without flattening partial publication or denial states.
#[derive(Debug)]
pub enum WorkActionCommitOutcome {
    Plan {
        request: WorkActionRequest,
        outcome: Box<CommitOutcome>,
    },
    Coordination(CoordinationOutcome),
    /// Plan/config/repository input moved before a claim-store write began.
    StaleInput {
        preview: WorkActionPreview,
        actual: InputIdentity,
    },
}

/// A request that cannot be previewed faithfully.
#[derive(Debug, thiserror::Error)]
pub enum WorkActionError {
    #[error("{0}")]
    Arguments(String),
    #[error("{0}")]
    Plan(String),
    #[error(transparent)]
    Claim(#[from] crate::claims::ClaimError),
    #[error("claim identity could not be resolved: {0}")]
    Identity(#[from] aplan_interop::identity::IdentityError),
    #[error("`{id}`: {source}")]
    Rewrite {
        id: String,
        #[source]
        source: aplan_core::CloseError,
    },
}

/// Resolve a claim identity through the exact precedence used by CLI.
pub fn resolve_claim_identity(
    context: &PlanContext,
    explicit: Option<&str>,
) -> Result<aplan_interop::identity::ResolvedIdentity, WorkActionError> {
    let sources =
        aplan_interop::identity::from_environment(explicit, &context.identity().repository_root);
    Ok(aplan_interop::identity::resolve_detailed(&sources)?)
}

/// Derive the complete contextual action catalogue for one task.
pub fn work_action_catalogue(
    context: &PlanContext,
    id: &str,
    explicit_actor: Option<&str>,
) -> Result<WorkActionCatalogue, WorkActionError> {
    let task = unique_task(context, id)?;
    let graph = Graph::new(context.plan().tasks.iter().map(|loaded| &loaded.task));
    // This is the readiness predicate, not a ranked ready-list request. Avoid
    // computing the whole critical path and downstream ranking merely to
    // decide whether one selected task can be started.
    let ready = graph.is_ready(id);
    let claims = crate::claims::snapshot(&context.identity().repository_root, context.plan())
        .unwrap_or_else(|error| {
            ClaimSnapshot::unavailable(crate::claims::now_epoch_seconds(), error.to_string())
        });
    let claim = claims.claim(id).cloned();
    let (actor, actor_error) = match resolve_claim_identity(context, explicit_actor) {
        Ok(identity) => (Some(identity.name), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let actions = catalogue_actions(
        id,
        task.task.status,
        task.task.is_milestone(),
        ready,
        ActorContext {
            explicit: explicit_actor,
            resolved: actor.as_deref(),
            error: actor_error.as_deref(),
        },
        claim.as_ref(),
        &claims.scope,
    );
    Ok(WorkActionCatalogue {
        input_sha256: context.identity().sha256.clone(),
        task: id.to_string(),
        status: task.task.status,
        ready,
        actor,
        actor_error,
        claim,
        coordination: claims.scope,
        actions,
    })
}

#[derive(Clone, Copy)]
struct ActorContext<'a> {
    explicit: Option<&'a str>,
    resolved: Option<&'a str>,
    error: Option<&'a str>,
}

fn catalogue_actions(
    id: &str,
    status: Status,
    milestone: bool,
    ready: bool,
    actor: ActorContext<'_>,
    claim: Option<&ClaimRecord>,
    scope: &ClaimScope,
) -> [WorkActionEntry; 4] {
    let live = !status.is_terminal();
    let coordination_reason = || {
        scope
            .reason
            .clone()
            .unwrap_or_else(|| "local claim coordination is unavailable".to_string())
    };
    let blocked_reason = || {
        if status == Status::Blocked {
            "the task is explicitly parked (`blocked`)".to_string()
        } else {
            "the task is not ready; complete or resolve its blockers first".to_string()
        }
    };
    let identity_reason = || {
        actor.error.map_or_else(
            || "claim identity is unavailable".to_string(),
            str::to_string,
        )
    };

    let claim_action = if !live {
        unavailable(format!("the task is `{status}`; claims name live work"))
    } else if milestone {
        unavailable("a milestone is a checkpoint, not work to claim")
    } else if !ready && status != Status::InProgress {
        unavailable(blocked_reason())
    } else if !scope.available {
        unavailable(coordination_reason())
    } else if actor.resolved.is_none() {
        unavailable(identity_reason())
    } else if let Some(current) = claim {
        if actor.resolved == Some(current.holder.as_str()) {
            unavailable(format!("already held by `{}`", current.holder))
        } else {
            available_claim(id, actor.explicit, Some(current.holder.clone()))
        }
    } else {
        available_claim(id, actor.explicit, None)
    };
    let release_action = if !scope.available {
        unavailable(coordination_reason())
    } else if claim.is_some() {
        available(id, WorkActionKind::Release)
    } else {
        unavailable("held by nobody")
    };
    let start_action = if !live {
        unavailable(format!(
            "the task is `{status}`; terminal work cannot be reopened"
        ))
    } else if milestone {
        unavailable("a milestone is a checkpoint, not work to start")
    } else if status == Status::InProgress {
        unavailable("already `in_progress`")
    } else if !ready {
        unavailable(blocked_reason())
    } else if !scope.available {
        unavailable(coordination_reason())
    } else if actor.resolved.is_none() {
        unavailable(identity_reason())
    } else if let Some(current) = claim {
        if actor.resolved == Some(current.holder.as_str()) {
            available(id, WorkActionKind::Start)
        } else {
            unavailable(format!(
                "held by `{}`; take over the claim before starting it",
                current.holder
            ))
        }
    } else {
        unavailable("claim the task before starting it")
    };
    let stop_action = if status == Status::InProgress {
        available(id, WorkActionKind::Stop)
    } else {
        unavailable(format!("the task is `{status}`, not `in_progress`"))
    };
    [
        WorkActionEntry {
            action: WorkActionKind::Claim,
            availability: claim_action,
        },
        WorkActionEntry {
            action: WorkActionKind::Release,
            availability: release_action,
        },
        WorkActionEntry {
            action: WorkActionKind::Start,
            availability: start_action,
        },
        WorkActionEntry {
            action: WorkActionKind::Stop,
            availability: stop_action,
        },
    ]
}

fn available(id: &str, action: WorkActionKind) -> WorkActionAvailability {
    WorkActionAvailability::Available {
        request: WorkActionRequest::simple(id, action),
    }
}

fn available_claim(
    id: &str,
    explicit_actor: Option<&str>,
    from: Option<String>,
) -> WorkActionAvailability {
    WorkActionAvailability::Available {
        request: WorkActionRequest {
            task: id.to_string(),
            action: WorkActionKind::Claim,
            as_name: explicit_actor.map(str::to_string),
            from,
            claimed_at_epoch_seconds: None,
        },
    }
}

fn unavailable(reason: impl Into<String>) -> WorkActionAvailability {
    WorkActionAvailability::Unavailable {
        reason: reason.into(),
    }
}

/// Compute an exact preview without writing either store.
pub fn preview_work_action(
    context: &PlanContext,
    request: &WorkActionRequest,
) -> Result<WorkActionPreview, WorkActionError> {
    // Release is the recovery path for a stale local claim and intentionally
    // remains usable when unrelated task files do not load. The other three
    // operations match their CLI commands and require one coherent plan.
    if request.action != WorkActionKind::Release && !context.plan().load_diagnostics.is_empty() {
        return Err(WorkActionError::Plan(
            "the plan does not load cleanly; run `aplan check` before mutating it".to_string(),
        ));
    }
    match request.action {
        WorkActionKind::Claim => {
            unique_task(context, &request.task)?;
            let identity = resolve_claim_identity(context, request.as_name.as_deref())?;
            let change = crate::claims::preview_claim(
                &context.identity().repository_root,
                context.plan(),
                &request.task,
                &identity,
                request.from.as_deref(),
                request
                    .claimed_at_epoch_seconds
                    .unwrap_or_else(crate::claims::now_epoch_seconds),
            )?;
            Ok(coordination_preview(
                context,
                request,
                change.displaced,
                Some(change.current),
                change.scope,
                change.warnings,
            ))
        }
        WorkActionKind::Release => {
            refuse_claim_fields(request)?;
            let change =
                crate::claims::preview_release(&context.identity().repository_root, &request.task)?;
            if change.released.is_none()
                && !context
                    .plan()
                    .tasks
                    .iter()
                    .any(|loaded| loaded.task.id == request.task)
            {
                return Err(WorkActionError::Arguments(format!(
                    "no task or local claim has id `{}`",
                    request.task
                )));
            }
            Ok(coordination_preview(
                context,
                request,
                change.released,
                None,
                change.scope,
                Vec::new(),
            ))
        }
        WorkActionKind::Start | WorkActionKind::Stop => {
            refuse_claim_fields(request)?;
            let task = unique_task(context, &request.task)?;
            preview_status(context, task, request)
        }
    }
}

fn coordination_preview(
    context: &PlanContext,
    request: &WorkActionRequest,
    before: Option<ClaimRecord>,
    after: Option<ClaimRecord>,
    scope: ClaimScope,
    warnings: Vec<String>,
) -> WorkActionPreview {
    WorkActionPreview {
        request: request.clone(),
        input: context.identity().clone(),
        change: WorkActionChange::Coordination(Box::new(ClaimPreview {
            before,
            after,
            scope,
            warnings,
        })),
    }
}

fn refuse_claim_fields(request: &WorkActionRequest) -> Result<(), WorkActionError> {
    if request.as_name.is_some()
        || request.from.is_some()
        || request.claimed_at_epoch_seconds.is_some()
    {
        return Err(WorkActionError::Arguments(format!(
            "`{}` does not accept claim identity, takeover, or claim-time fields",
            request.action.label()
        )));
    }
    Ok(())
}

fn preview_status(
    context: &PlanContext,
    task: &aplan_core::load::LoadedTask,
    request: &WorkActionRequest,
) -> Result<WorkActionPreview, WorkActionError> {
    let starting = request.action == WorkActionKind::Start;
    let want = if starting {
        Status::InProgress
    } else {
        Status::Todo
    };
    let current = task.task.status;
    if current == want {
        return Ok(WorkActionPreview {
            request: request.clone(),
            input: context.identity().clone(),
            change: WorkActionChange::Plan(Box::new(ChangeSet::from_parts(
                context.identity().clone(),
                Vec::new(),
                Vec::new(),
                aplan_core::mutation::Delta::default(),
                false,
                ChangeEvidence::NotRequired,
            ))),
        });
    }
    if current.is_terminal() {
        return Err(WorkActionError::Arguments(format!(
            "cannot {} `{}` because it is `{current}`, a terminal state",
            request.action.label(),
            request.task
        )));
    }
    if !starting && current != Status::InProgress {
        return Err(WorkActionError::Arguments(format!(
            "`{}` is `{current}`, not `in_progress`, so there is nothing to stop",
            request.task
        )));
    }
    let original = context.source_text(&task.path).ok_or_else(|| {
        WorkActionError::Plan(format!(
            "`{}` changed while its {} preview was being prepared; reload and try again",
            request.task,
            request.action.label()
        ))
    })?;
    let updated = aplan_core::close::set_status(original, want).map_err(|source| {
        WorkActionError::Rewrite {
            id: request.task.clone(),
            source,
        }
    })?;
    let produced = aplan_core::mutation::task_of(&updated)
        .map_err(|error| WorkActionError::Plan(format!("`{}`: {error}", request.task)))?;
    let after = aplan_core::mutation::with_replaced(context.plan(), &request.task, produced);
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    let change = ChangeSet::from_parts(
        context.identity().clone(),
        vec![FileChange::replacement(
            task.path.clone(),
            original.to_string(),
            updated,
        )],
        vec![SemanticEffect::TaskStatusChanged {
            task: request.task.clone(),
            from: current,
            to: want,
        }],
        diagnostics,
        denied,
        ChangeEvidence::NotRequired,
    )
    .with_evidence_review(EvidenceReview::default());
    Ok(WorkActionPreview {
        request: request.clone(),
        input: context.identity().clone(),
        change: WorkActionChange::Plan(Box::new(change)),
    })
}

/// Confirm an exact preview against both its plan input and store-specific
/// compare-and-swap boundary.
#[must_use]
pub fn commit_work_action(
    context: &PlanContext,
    preview: WorkActionPreview,
) -> WorkActionCommitOutcome {
    match preview.change {
        WorkActionChange::Plan(change) => WorkActionCommitOutcome::Plan {
            request: preview.request,
            outcome: Box::new(crate::commit_change_set(context, *change)),
        },
        WorkActionChange::Coordination(ref claim) => {
            let loaded = aplan_core::load_plan_identified(&preview.input.plan_root);
            let actual = context.identity_for(&loaded);
            if actual != preview.input {
                return WorkActionCommitOutcome::StaleInput { preview, actual };
            }
            let result = match preview.request.action {
                WorkActionKind::Claim => {
                    let reviewed = crate::claims::ClaimChange {
                        current: claim
                            .after
                            .clone()
                            .expect("a claim preview always has an after record"),
                        displaced: claim.before.clone(),
                        warnings: claim.warnings.clone(),
                        scope: claim.scope.clone(),
                    };
                    crate::claims::commit_claim(
                        &preview.input.repository_root,
                        &loaded.plan,
                        &preview.request.task,
                        &reviewed,
                    )
                    .map(|_| claim.before != claim.after)
                }
                WorkActionKind::Release => {
                    let reviewed = crate::claims::ReleaseChange {
                        released: claim.before.clone(),
                        scope: claim.scope.clone(),
                    };
                    crate::claims::commit_release(
                        &preview.input.repository_root,
                        &preview.request.task,
                        &reviewed,
                    )
                    .map(|_| claim.before.is_some())
                }
                WorkActionKind::Start | WorkActionKind::Stop => {
                    unreachable!("plan work actions were dispatched above")
                }
            };
            match result {
                Ok(true) => {
                    WorkActionCommitOutcome::Coordination(CoordinationOutcome::Applied(preview))
                }
                Ok(false) => {
                    WorkActionCommitOutcome::Coordination(CoordinationOutcome::NoOp(preview))
                }
                Err(crate::claims::ClaimError::Stale { .. }) => {
                    let actual =
                        crate::claims::snapshot(&preview.input.repository_root, &loaded.plan)
                            .unwrap_or_else(|error| {
                                ClaimSnapshot::unavailable(
                                    crate::claims::now_epoch_seconds(),
                                    error.to_string(),
                                )
                            });
                    WorkActionCommitOutcome::Coordination(CoordinationOutcome::Stale {
                        preview,
                        actual,
                    })
                }
                Err(error) => WorkActionCommitOutcome::Coordination(CoordinationOutcome::Failed {
                    preview,
                    error,
                }),
            }
        }
    }
}

fn unique_task<'a>(
    context: &'a PlanContext,
    id: &str,
) -> Result<&'a aplan_core::load::LoadedTask, WorkActionError> {
    let mut matches = context
        .plan()
        .tasks
        .iter()
        .filter(|loaded| loaded.task.id == id);
    let Some(task) = matches.next() else {
        return Err(WorkActionError::Arguments(format!(
            "no task `{id}` exists in this plan"
        )));
    };
    if matches.next().is_some() {
        return Err(WorkActionError::Plan(format!(
            "more than one task declares `{id}`; refusing to guess which it is"
        )));
    }
    Ok(task)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::{ContextInputs, Trust};

    struct Fixture {
        _dir: tempfile::TempDir,
        repository: PathBuf,
        plan: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("fixture");
            let repository = dir.path().to_path_buf();
            let plan = repository.join("plan");
            std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
            let task = |id: &str, status: &str, blocked_by: &[&str], paths: &[&str]| {
                let blocked = if blocked_by.is_empty() {
                    String::new()
                } else {
                    format!("blocked_by: [{}]\n", blocked_by.join(", "))
                };
                let evidence = if status == "done" {
                    "evidence:\n  - kind: note\n    ref: 2026-10-25 reviewed\n"
                } else {
                    ""
                };
                format!(
                    "---\nid: {id}\ntitle: {id} title\nphase: 1\nstatus: {status}\neffort_days: 1\n{blocked}paths: [{}]\ntags: [cross-cutting]\n{evidence}---\n",
                    paths.join(", ")
                )
            };
            for (id, status, blocked, paths) in [
                ("A-1", "todo", Vec::new(), vec!["src/**"]),
                ("A-2", "todo", vec!["A-1"], vec!["src/**"]),
                ("A-3", "in_progress", Vec::new(), vec!["src/**"]),
                ("A-4", "blocked", Vec::new(), vec!["src/**"]),
                ("A-5", "done", Vec::new(), vec!["src/**"]),
                ("A-6", "dropped", Vec::new(), vec!["src/**"]),
                ("A-7", "todo", Vec::new(), vec!["missing/**"]),
            ] {
                std::fs::write(
                    plan.join(format!("tasks/{id}.md")),
                    task(id, status, &blocked, &paths),
                )
                .expect("task");
            }
            std::fs::write(
                plan.join("tasks/M-1.md"),
                "---\nid: M-1\ntitle: checkpoint\nphase: 1\nstatus: todo\neffort_days: 0\nblocked_by: [A-1]\npaths: [src/**]\ntags: [milestone, cross-cutting]\n---\n",
            )
            .expect("milestone");
            std::fs::create_dir_all(repository.join("src")).expect("source directory");
            std::fs::write(repository.join("src/lib.rs"), "pub fn work() {}\n").expect("source");
            git(&repository, &["init", "-q"]);
            git(&repository, &["config", "user.email", "test@example.com"]);
            git(&repository, &["config", "user.name", "Fixture User"]);
            git(&repository, &["add", "."]);
            git(&repository, &["commit", "-qm", "fixture"]);
            Self {
                _dir: dir,
                repository,
                plan,
            }
        }

        fn context(&self, deny_warnings: bool) -> PlanContext {
            PlanContext::new(
                aplan_core::load_plan_identified(&self.plan),
                ContextInputs {
                    repository_root: self.repository.clone(),
                    configuration_file: None,
                    trust: Trust::RepositoryRead,
                    options: aplan_core::Options::default(),
                    verdict: aplan_core::Verdict {
                        deny_warnings,
                        advisory: Vec::new(),
                    },
                },
            )
        }

        fn task_path(&self, id: &str) -> PathBuf {
            self.plan.join(format!("tasks/{id}.md"))
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn request(id: &str, action: WorkActionKind) -> WorkActionRequest {
        WorkActionRequest::simple(id, action)
    }

    fn claim_request(id: &str, actor: &str, from: Option<&str>, at: i64) -> WorkActionRequest {
        WorkActionRequest {
            task: id.to_string(),
            action: WorkActionKind::Claim,
            as_name: Some(actor.to_string()),
            from: from.map(str::to_string),
            claimed_at_epoch_seconds: Some(at),
        }
    }

    #[test]
    fn contextual_actions_follow_readiness_status_claim_and_actor() {
        let fixture = Fixture::new();
        let context = fixture.context(false);

        let ready = work_action_catalogue(&context, "A-1", Some("alex")).expect("ready");
        assert!(ready.ready);
        let WorkActionAvailability::Available { request } =
            &ready.action(WorkActionKind::Claim).availability
        else {
            panic!("claim should be available")
        };
        assert_eq!(request.as_name.as_deref(), Some("alex"));
        assert!(matches!(
            &ready.action(WorkActionKind::Start).availability,
            WorkActionAvailability::Unavailable { reason } if reason.contains("claim")
        ));
        assert!(matches!(
            &ready.action(WorkActionKind::Release).availability,
            WorkActionAvailability::Unavailable { reason } if reason == "held by nobody"
        ));

        let waiting = work_action_catalogue(&context, "A-2", Some("alex")).expect("waiting");
        assert!(!waiting.ready);
        for action in [WorkActionKind::Claim, WorkActionKind::Start] {
            assert!(matches!(
                &waiting.action(action).availability,
                WorkActionAvailability::Unavailable { reason } if reason.contains("not ready")
            ));
        }
        let parked = work_action_catalogue(&context, "A-4", Some("alex")).expect("parked");
        assert!(matches!(
            &parked.action(WorkActionKind::Start).availability,
            WorkActionAvailability::Unavailable { reason } if reason.contains("parked")
        ));
        for id in ["A-5", "A-6"] {
            let terminal = work_action_catalogue(&context, id, Some("alex")).expect("terminal");
            assert!(
                WorkActionKind::ALL
                    .into_iter()
                    .all(|action| !terminal.action(action).availability.is_available())
            );
        }
        let milestone = work_action_catalogue(&context, "M-1", Some("alex")).expect("milestone");
        for action in [WorkActionKind::Claim, WorkActionKind::Start] {
            assert!(matches!(
                &milestone.action(action).availability,
                WorkActionAvailability::Unavailable { reason } if reason.contains("checkpoint")
            ));
        }
    }

    #[test]
    fn contextual_actions_bind_identity_and_preserve_stale_claim_cleanup() {
        let fixture = Fixture::new();
        let context = fixture.context(false);
        let invalid_actor =
            work_action_catalogue(&context, "A-1", Some("\n")).expect("catalogue remains useful");
        assert!(invalid_actor.actor.is_none());
        assert!(invalid_actor.actor_error.is_some());
        for action in [WorkActionKind::Claim, WorkActionKind::Start] {
            assert!(matches!(
                &invalid_actor.action(action).availability,
                WorkActionAvailability::Unavailable { reason } if reason.contains("cannot be used as a name")
            ));
        }

        let preview = preview_work_action(&context, &claim_request("A-1", "alex", None, 10))
            .expect("claim preview");
        assert!(matches!(
            commit_work_action(&context, preview),
            WorkActionCommitOutcome::Coordination(CoordinationOutcome::Applied(_))
        ));
        let claimed_context = fixture.context(false);
        let own = work_action_catalogue(&claimed_context, "A-1", Some("alex")).expect("own");
        assert!(
            own.action(WorkActionKind::Release)
                .availability
                .is_available()
        );
        assert!(
            own.action(WorkActionKind::Start)
                .availability
                .is_available()
        );
        assert!(
            !own.action(WorkActionKind::Claim)
                .availability
                .is_available()
        );

        let other = work_action_catalogue(&claimed_context, "A-1", Some("blair")).expect("other");
        let WorkActionAvailability::Available { request } =
            &other.action(WorkActionKind::Claim).availability
        else {
            panic!("takeover must be available")
        };
        assert_eq!(request.from.as_deref(), Some("alex"));
        assert!(matches!(
            &other.action(WorkActionKind::Start).availability,
            WorkActionAvailability::Unavailable { reason } if reason.contains("alex")
        ));

        std::fs::write(
            fixture.task_path("A-1"),
            std::fs::read_to_string(fixture.task_path("A-1"))
                .expect("task")
                .replace(
                    "status: todo",
                    "status: done\nevidence:\n  - kind: note\n    ref: 2026-10-25 reviewed",
                ),
        )
        .expect("terminal task with stale claim");
        let terminal_context = fixture.context(false);
        let stale_claim =
            work_action_catalogue(&terminal_context, "A-1", Some("alex")).expect("terminal");
        assert!(
            stale_claim
                .action(WorkActionKind::Release)
                .availability
                .is_available()
        );
        assert!(
            !stale_claim
                .action(WorkActionKind::Claim)
                .availability
                .is_available()
        );
    }

    #[test]
    fn claim_and_release_preview_exact_records_and_detect_every_store_race() {
        let fixture = Fixture::new();
        let context = fixture.context(false);
        let preview = preview_work_action(&context, &claim_request("A-1", "alex", None, 10))
            .expect("claim preview");
        let WorkActionChange::Coordination(change) = preview.change() else {
            panic!("claim change")
        };
        assert!(change.before.is_none());
        assert_eq!(
            change.after.as_ref().map(|record| record.holder.as_str()),
            Some("alex")
        );
        assert_eq!(
            change
                .after
                .as_ref()
                .map(|record| record.claimed_at_epoch_seconds),
            Some(10)
        );

        let identity = resolve_claim_identity(&context, Some("alex")).expect("identity");
        crate::claims::claim(
            &fixture.repository,
            context.plan(),
            "A-1",
            &identity,
            None,
            11,
        )
        .expect("same-holder concurrent claim");
        let stale = commit_work_action(&context, preview);
        let WorkActionCommitOutcome::Coordination(CoordinationOutcome::Stale { actual, .. }) =
            stale
        else {
            panic!("same-holder replacement must be stale: {stale:?}")
        };
        assert_eq!(
            actual
                .claim("A-1")
                .map(|record| record.claimed_at_epoch_seconds),
            Some(11)
        );
        let snapshot = crate::claims::snapshot(&fixture.repository, context.plan()).expect("state");
        assert_eq!(
            snapshot
                .claim("A-1")
                .map(|record| record.claimed_at_epoch_seconds),
            Some(11)
        );

        let noop_claim =
            preview_work_action(&context, &claim_request("A-1", "alex", Some("alex"), 11))
                .expect("identical reclaim preview");
        assert!(matches!(
            commit_work_action(&context, noop_claim),
            WorkActionCommitOutcome::Coordination(CoordinationOutcome::NoOp(_))
        ));

        let release = preview_work_action(&context, &request("A-1", WorkActionKind::Release))
            .expect("release preview");
        let identity = resolve_claim_identity(&context, Some("blair")).expect("identity");
        crate::claims::claim(
            &fixture.repository,
            context.plan(),
            "A-1",
            &identity,
            Some("alex"),
            12,
        )
        .expect("takeover");
        let stale = commit_work_action(&context, release);
        let WorkActionCommitOutcome::Coordination(CoordinationOutcome::Stale { actual, .. }) =
            stale
        else {
            panic!("takeover must make release stale: {stale:?}")
        };
        assert_eq!(
            actual.claim("A-1").map(|record| record.holder.as_str()),
            Some("blair")
        );
        let snapshot = crate::claims::snapshot(&fixture.repository, context.plan()).expect("state");
        assert_eq!(
            snapshot.claim("A-1").map(|record| record.holder.as_str()),
            Some("blair")
        );

        let release = preview_work_action(&context, &request("A-1", WorkActionKind::Release))
            .expect("current release");
        assert!(matches!(
            commit_work_action(&context, release),
            WorkActionCommitOutcome::Coordination(CoordinationOutcome::Applied(_))
        ));
        let noop = preview_work_action(&context, &request("A-1", WorkActionKind::Release))
            .expect("empty release");
        assert!(matches!(
            commit_work_action(&context, noop),
            WorkActionCommitOutcome::Coordination(CoordinationOutcome::NoOp(_))
        ));
    }

    #[test]
    fn release_recovers_an_orphan_but_refuses_an_unknown_id() {
        let fixture = Fixture::new();
        let context = fixture.context(false);
        let claim = preview_work_action(&context, &claim_request("A-1", "alex", None, 10))
            .expect("claim preview");
        assert!(matches!(
            commit_work_action(&context, claim),
            WorkActionCommitOutcome::Coordination(CoordinationOutcome::Applied(_))
        ));

        std::fs::remove_file(fixture.task_path("A-1")).expect("remove claimed task");
        let orphaned = fixture.context(false);
        let release = preview_work_action(&orphaned, &request("A-1", WorkActionKind::Release))
            .expect("an orphaned claim remains releasable");
        assert!(matches!(
            commit_work_action(&orphaned, release),
            WorkActionCommitOutcome::Coordination(CoordinationOutcome::Applied(_))
        ));

        let unknown = preview_work_action(
            &fixture.context(false),
            &request("A-1", WorkActionKind::Release),
        );
        assert!(matches!(
            unknown,
            Err(WorkActionError::Arguments(message))
                if message == "no task or local claim has id `A-1`"
        ));
    }

    #[test]
    fn claim_confirmation_rejects_plan_movement_before_touching_claim_state() {
        let fixture = Fixture::new();
        let context = fixture.context(false);
        let preview = preview_work_action(&context, &claim_request("A-1", "alex", None, 10))
            .expect("preview");
        std::fs::write(
            fixture.task_path("A-2"),
            std::fs::read_to_string(fixture.task_path("A-2"))
                .expect("task")
                .replace("A-2 title", "changed concurrently"),
        )
        .expect("concurrent edit");
        let outcome = commit_work_action(&context, preview);
        let WorkActionCommitOutcome::StaleInput { preview, actual } = outcome else {
            panic!("plan movement must make claim stale: {outcome:?}")
        };
        assert_ne!(preview.input(), &actual);
        let current = fixture.context(false);
        let snapshot =
            crate::claims::snapshot(&fixture.repository, current.plan()).expect("claims");
        assert!(snapshot.claim("A-1").is_none());
    }

    #[test]
    fn status_previews_match_cli_rules_and_publish_only_the_reviewed_bytes() {
        let fixture = Fixture::new();
        let context = fixture.context(false);
        let preview = preview_work_action(&context, &request("A-1", WorkActionKind::Start))
            .expect("start preview");
        let change = preview.change_set().expect("plan change");
        assert_eq!(change.files().len(), 1);
        assert!(
            change.files()[0]
                .before()
                .expect("before")
                .contains("status: todo")
        );
        assert!(change.files()[0].after().contains("status: in_progress"));
        assert_eq!(
            change.effects(),
            [SemanticEffect::TaskStatusChanged {
                task: "A-1".to_string(),
                from: Status::Todo,
                to: Status::InProgress,
            }]
        );
        let reviewed = change.files()[0].after().to_string();
        assert!(matches!(
            commit_work_action(&context, preview),
            WorkActionCommitOutcome::Plan { outcome, .. }
                if matches!(*outcome, CommitOutcome::Applied(_))
        ));
        assert_eq!(
            std::fs::read_to_string(fixture.task_path("A-1")).expect("written"),
            reviewed
        );

        let started = fixture.context(false);
        let noop = preview_work_action(&started, &request("A-1", WorkActionKind::Start))
            .expect("idempotent start");
        assert!(matches!(
            commit_work_action(&started, noop),
            WorkActionCommitOutcome::Plan { outcome, .. }
                if matches!(*outcome, CommitOutcome::NoOp(_))
        ));
        let stop = preview_work_action(&started, &request("A-1", WorkActionKind::Stop))
            .expect("stop preview");
        assert!(
            stop.change_set().expect("plan").files()[0]
                .after()
                .contains("status: todo")
        );

        for id in ["A-5", "A-6"] {
            assert!(matches!(
                preview_work_action(&started, &request(id, WorkActionKind::Start)),
                Err(WorkActionError::Arguments(message)) if message.contains("terminal")
            ));
        }
        let todo_stop = preview_work_action(&started, &request("A-2", WorkActionKind::Stop))
            .expect("CLI treats stop on todo as an idempotent no-op");
        assert_eq!(
            todo_stop.change_set().expect("plan").disposition(),
            crate::ChangeDisposition::NoOp
        );
        assert!(matches!(
            preview_work_action(&started, &request("A-4", WorkActionKind::Stop)),
            Err(WorkActionError::Arguments(message)) if message.contains("nothing to stop")
        ));
    }

    #[test]
    fn status_preview_retains_gate_findings_and_guarded_staleness() {
        let fixture = Fixture::new();
        let strict = fixture.context(true);
        let reviewed = preview_work_action(&strict, &request("A-7", WorkActionKind::Start))
            .expect("reviewed preview");
        let change = reviewed.change_set().expect("plan change");
        // APLAN053 is repository evidence, not a hermetic mutation finding.
        // Match the CLI gate exactly rather than inventing a stronger TUI-only
        // refusal: start records the claim and `verify` can contradict it.
        assert_eq!(change.disposition(), crate::ChangeDisposition::Ready);
        assert!(change.diagnostics().added.is_empty());
        assert!(matches!(
            commit_work_action(&strict, reviewed),
            WorkActionCommitOutcome::Plan { outcome, .. }
                if matches!(*outcome, CommitOutcome::Applied(_))
        ));
        assert!(
            std::fs::read_to_string(fixture.task_path("A-7"))
                .expect("written")
                .contains("status: in_progress")
        );

        let current = fixture.context(false);
        let stale = preview_work_action(&current, &request("A-2", WorkActionKind::Start))
            .expect("preview even when contextual menu disables it");
        std::fs::write(
            fixture.task_path("A-2"),
            std::fs::read_to_string(fixture.task_path("A-2"))
                .expect("task")
                .replace("A-2 title", "concurrent"),
        )
        .expect("edit");
        let outcome = commit_work_action(&current, stale);
        let WorkActionCommitOutcome::Plan { outcome, .. } = outcome else {
            panic!("concurrent task edit must be stale: {outcome:?}")
        };
        let CommitOutcome::Stale { change, actual } = *outcome else {
            panic!("concurrent task edit must be stale")
        };
        assert_ne!(change.input(), &actual);
        assert!(
            std::fs::read_to_string(fixture.task_path("A-2"))
                .expect("preserved")
                .contains("concurrent")
        );
    }

    #[test]
    fn malformed_requests_and_unavailable_coordination_fail_closed() {
        let fixture = Fixture::new();
        let context = fixture.context(false);
        let mut malformed = request("A-1", WorkActionKind::Start);
        malformed.as_name = Some("alex".to_string());
        assert!(matches!(
            preview_work_action(&context, &malformed),
            Err(WorkActionError::Arguments(message)) if message.contains("does not accept")
        ));
        for action in [
            WorkActionKind::Release,
            WorkActionKind::Start,
            WorkActionKind::Stop,
        ] {
            let mut malformed = request("A-1", action);
            malformed.from = Some("alex".to_string());
            assert!(matches!(
                preview_work_action(&context, &malformed),
                Err(WorkActionError::Arguments(message)) if message.contains(action.label())
            ));
        }
        assert!(matches!(
            preview_work_action(&context, &request("missing", WorkActionKind::Start)),
            Err(WorkActionError::Arguments(message)) if message.contains("no task")
        ));
        assert!(matches!(
            preview_work_action(&context, &claim_request("A-1", "\n", None, 10)),
            Err(WorkActionError::Identity(_))
        ));
        assert!(matches!(
            preview_work_action(&context, &claim_request("A-1", "alex", None, -1)),
            Err(WorkActionError::Claim(
                crate::claims::ClaimError::Invalid { .. }
            ))
        ));

        let outside = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("outside");
        std::fs::create_dir_all(outside.path().join("tasks")).expect("tasks");
        std::fs::write(
            outside.path().join("tasks/A-1.md"),
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\n---\n",
        )
        .expect("task");
        let outside_context = PlanContext::new(
            aplan_core::load_plan_identified(outside.path()),
            ContextInputs {
                repository_root: outside.path().to_path_buf(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        );
        let catalogue =
            work_action_catalogue(&outside_context, "A-1", Some("alex")).expect("catalogue");
        assert!(!catalogue.coordination.available);
        assert!(matches!(
            &catalogue.action(WorkActionKind::Claim).availability,
            WorkActionAvailability::Unavailable { reason } if reason.contains("Git repository")
        ));
        assert!(matches!(
            preview_work_action(&outside_context, &claim_request("A-1", "alex", None, 10)),
            Err(WorkActionError::Claim(
                crate::claims::ClaimError::Unavailable { .. }
            ))
        ));

        std::fs::write(
            fixture.plan.join("tasks/broken.md"),
            "---\nid: [not valid YAML\n---\n",
        )
        .expect("broken task");
        let broken = fixture.context(false);
        assert!(!broken.plan().load_diagnostics.is_empty());
        assert!(preview_work_action(&broken, &request("A-1", WorkActionKind::Release)).is_ok());
        assert!(matches!(
            preview_work_action(&broken, &request("A-1", WorkActionKind::Start)),
            Err(WorkActionError::Plan(message)) if message.contains("does not load cleanly")
        ));

        std::fs::write(
            fixture.plan.join("tasks/duplicate.md"),
            std::fs::read_to_string(fixture.task_path("A-1")).expect("task"),
        )
        .expect("duplicate");
        let duplicate = fixture.context(false);
        let release = preview_work_action(&duplicate, &request("A-1", WorkActionKind::Release))
            .expect("release does not need to choose between task files");
        assert!(matches!(
            commit_work_action(&duplicate, release),
            WorkActionCommitOutcome::Coordination(CoordinationOutcome::NoOp(_))
        ));
    }

    #[test]
    fn preview_and_commit_failures_retain_their_typed_boundary() {
        let fixture = Fixture::new();
        let context = fixture.context(false);
        assert_eq!(
            WorkActionKind::ALL.map(WorkActionKind::label),
            ["claim", "release", "start", "stop"]
        );

        let preview = preview_work_action(&context, &claim_request("A-1", "alex", None, 10))
            .expect("claim preview");
        let identity = resolve_claim_identity(&context, Some("blair")).expect("identity");
        crate::claims::claim(
            &fixture.repository,
            context.plan(),
            "A-1",
            &identity,
            None,
            11,
        )
        .expect("concurrent claim");
        let state = fixture.repository.join(".git/aplan/claims-v1.json");
        std::fs::write(&state, "{not JSON\n").expect("corrupt coordination state");
        let failed = commit_work_action(&context, preview);
        let WorkActionCommitOutcome::Coordination(CoordinationOutcome::Failed {
            preview,
            error: crate::claims::ClaimError::Invalid { .. },
        }) = failed
        else {
            panic!("coordination failure was flattened: {failed:?}")
        };
        assert_eq!(preview.request().task, "A-1");
        assert!(preview.change_set().is_none());

        let mut digest_only = aplan_core::load_plan_identified(&fixture.plan);
        for input in &mut digest_only.inputs {
            if input.path == fixture.task_path("A-2") {
                input.content = aplan_core::load::PlanFileContent::Sha256("digest".to_string());
            }
        }
        let digest_context = PlanContext::new(
            digest_only,
            ContextInputs {
                repository_root: fixture.repository.clone(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        );
        assert!(matches!(
            preview_work_action(
                &digest_context,
                &request("A-2", WorkActionKind::Start)
            ),
            Err(WorkActionError::Plan(message)) if message.contains("changed while")
        ));

        let mut inconsistent = aplan_core::load_plan_identified(&fixture.plan);
        for input in &mut inconsistent.inputs {
            if input.path == fixture.task_path("A-2")
                && let aplan_core::load::PlanFileContent::Bytes(bytes) = &mut input.content
            {
                *bytes = String::from_utf8_lossy(bytes)
                    .replace("status: todo", "status: [todo]")
                    .into_bytes();
            }
        }
        let inconsistent_context = PlanContext::new(
            inconsistent,
            ContextInputs {
                repository_root: fixture.repository.clone(),
                configuration_file: None,
                trust: Trust::Hermetic,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        );
        assert!(matches!(
            preview_work_action(
                &inconsistent_context,
                &request("A-2", WorkActionKind::Start)
            ),
            Err(WorkActionError::Rewrite { id, .. }) if id == "A-2"
        ));
    }
}
