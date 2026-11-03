//! Deterministic builders for reducer and renderer tests.

use crate::model::{Action, Focus, Glyphs, Model, Snapshot};
use aplan_app::{Status, TaskBrowserRow};
use aplan_core::task::Task;

pub(crate) fn capture_record(
    id: impl Into<String>,
    title: impl Into<String>,
    state: aplan_core::capture::CaptureState,
    captured_at: impl Into<String>,
) -> aplan_core::capture::Capture {
    let terminal = state != aplan_core::capture::CaptureState::Open;
    aplan_core::capture::Capture {
        schema: aplan_core::capture::CaptureSchema::V1,
        id: id.into(),
        title: title.into(),
        kind: "task".to_string(),
        state,
        captured_at: captured_at.into(),
        origin: aplan_core::capture::CaptureOrigin::Tui,
        body: None,
        labels: Vec::new(),
        revisit_at: None,
        profile: None,
        promotion_defaults: None,
        source: None,
        disposed_at: terminal.then(|| "2026-01-01T00:00:00Z".to_string()),
        reason: None,
        promoted_to: None,
        promotion_fingerprint: (state == aplan_core::capture::CaptureState::Promoted)
            .then(|| "a".repeat(64)),
    }
}

pub(crate) fn task_inspector(row: &TaskBrowserRow) -> aplan_app::Show {
    aplan_app::Show {
        schema: "aplan/show/v3".to_string(),
        id: row.id.clone(),
        title: row.title.clone(),
        status: row.status.to_string(),
        phase: row.phase,
        owner: row.owner.clone(),
        claim: row.claim.clone(),
        claim_age_seconds: row.claim.as_ref().map(|claim| {
            row.claim_observed_at_epoch_seconds
                .saturating_sub(claim.claimed_at_epoch_seconds)
                .max(0)
        }),
        coordination: row.coordination.clone(),
        effort_days: Some(1.0),
        tags: row.tags.clone(),
        cut: row.cut.clone(),
        milestone: row.milestone.clone(),
        ready: row.ready_rank.is_some(),
        workable: row.status.is_workable(),
        paths: Vec::new(),
        traces: Vec::new(),
        trace_fingerprints: std::collections::BTreeMap::new(),
        evidence: Vec::new(),
        evidence_fingerprints: std::collections::BTreeMap::new(),
        waits_on: Vec::new(),
        releases: Vec::new(),
        prefer_after: Vec::new(),
        ratified_by: Vec::new(),
        contains: Vec::new(),
        findings: Vec::new(),
        resolved: false,
        evidence_observation: aplan_core::show::EvidenceObservation::not_observed(
            "synthetic inspector",
        ),
        body: Some(row.body.clone()),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
        version: None,
    }
}

pub(crate) fn task_row(id: impl Into<String>, title: impl Into<String>) -> TaskBrowserRow {
    TaskBrowserRow {
        id: id.into(),
        title: title.into(),
        body: String::new(),
        phase: 1,
        status: Status::Todo,
        owner: None,
        claim: None,
        coordination: aplan_core::ClaimScope::git_common_directory("test".to_string()),
        claim_observed_at_epoch_seconds: 0,
        tags: Vec::new(),
        cut: None,
        milestone: None,
        ready_rank: None,
        rank_reason: None,
    }
}

pub(crate) fn graph_task(id: &str, title: &str, blocked_by: &[&str]) -> Task {
    Task {
        id: id.to_string(),
        title: title.to_string(),
        phase: 1,
        status: Status::Todo,
        owner: None,
        effort_days: Some(1.0),
        blocked_by: blocked_by.iter().map(|id| (*id).to_string()).collect(),
        prefer_after: Vec::new(),
        ratified_by: Vec::new(),
        contains: Vec::new(),
        evidence: Vec::new(),
        paths: Vec::new(),
        traces: Vec::new(),
        trace_fingerprints: std::collections::BTreeMap::new(),
        evidence_fingerprints: std::collections::BTreeMap::new(),
        tags: Vec::new(),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
        version: None,
    }
}

#[derive(Debug)]
pub(crate) struct ModelBuilder {
    snapshot: Snapshot,
    focus: Focus,
    glyphs: Glyphs,
}

impl Default for ModelBuilder {
    fn default() -> Self {
        let task_rows = vec![
            task_row("A-1", "first task"),
            task_row("A-2", "second task"),
        ];
        let inspectors = task_rows.iter().map(task_inspector).collect();
        Self {
            snapshot: Snapshot::test(
                "identity",
                "repo/very-long-root",
                task_rows,
                inspectors,
                1,
                2,
            ),
            focus: Focus::Primary,
            glyphs: Glyphs::Unicode,
        }
    }
}

impl ModelBuilder {
    pub(crate) fn input(mut self, input: &str) -> Self {
        let rows = self.snapshot.task_rows.clone();
        let inspectors = rows.iter().map(task_inspector).collect();
        self.snapshot = Snapshot::test(
            input,
            "repo/very-long-root",
            rows,
            inspectors,
            self.snapshot.overview.errors,
            self.snapshot.overview.warnings,
        );
        self
    }

    pub(crate) const fn focus(mut self, focus: Focus) -> Self {
        self.focus = focus;
        self
    }

    pub(crate) const fn glyphs(mut self, glyphs: Glyphs) -> Self {
        self.glyphs = glyphs;
        self
    }

    pub(crate) fn graph(mut self, tasks: Vec<Task>) -> Self {
        let rows = tasks
            .iter()
            .map(|task| TaskBrowserRow {
                id: task.id.clone(),
                title: task.title.clone(),
                body: String::new(),
                phase: task.phase,
                status: task.status,
                owner: task.owner.clone(),
                claim: None,
                coordination: aplan_core::ClaimScope::git_common_directory("test".to_string()),
                claim_observed_at_epoch_seconds: 0,
                tags: task.tags.clone(),
                cut: None,
                milestone: None,
                ready_rank: None,
                rank_reason: None,
            })
            .collect::<Vec<_>>();
        let inspectors = rows.iter().map(task_inspector).collect();
        self.snapshot.task_rows = rows.clone();
        self.snapshot.inspectors = inspectors;
        self.snapshot.graph = aplan_app::GraphWorkspace::new(
            self.snapshot.overview.input.clone(),
            tasks.clone(),
            rows,
        );
        self.snapshot.work = aplan_app::WorkDiscoveryWorkspace::new(
            self.snapshot.overview.input.clone(),
            tasks,
            std::collections::BTreeMap::new(),
        );
        self
    }

    pub(crate) fn graph_with_catalogue(
        mut self,
        tasks: Vec<Task>,
        catalogue: Vec<TaskBrowserRow>,
    ) -> Self {
        let inspectors = catalogue.iter().map(task_inspector).collect();
        self.snapshot.task_rows = catalogue.clone();
        self.snapshot.inspectors = inspectors;
        self.snapshot.graph = aplan_app::GraphWorkspace::new(
            self.snapshot.overview.input.clone(),
            tasks.clone(),
            catalogue,
        );
        self.snapshot.work = aplan_app::WorkDiscoveryWorkspace::new(
            self.snapshot.overview.input.clone(),
            tasks,
            std::collections::BTreeMap::new(),
        );
        self
    }

    pub(crate) fn inbox(
        mut self,
        captures: Vec<aplan_core::capture::Capture>,
        evaluated_at: &str,
    ) -> Self {
        let kinds = ["idea", "note", "task"]
            .into_iter()
            .map(str::to_string)
            .chain(captures.iter().map(|capture| capture.kind.clone()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let total_open = captures
            .iter()
            .filter(|capture| capture.state == aplan_core::capture::CaptureState::Open)
            .count();
        let total_promoted = captures
            .iter()
            .filter(|capture| capture.state == aplan_core::capture::CaptureState::Promoted)
            .count();
        let total_discarded = captures
            .iter()
            .filter(|capture| capture.state == aplan_core::capture::CaptureState::Discarded)
            .count();
        let deferred_open = captures
            .iter()
            .filter(|capture| {
                capture.state == aplan_core::capture::CaptureState::Open
                    && capture
                        .revisit_at
                        .as_deref()
                        .is_some_and(|instant| instant > evaluated_at)
            })
            .count();
        self.snapshot.inbox = Ok(aplan_app::CaptureWorkspace {
            input: self.snapshot.overview.input.clone(),
            inbox_sha256: "test-inbox".to_string(),
            evaluated_at: evaluated_at.to_string(),
            deferred_open,
            captures,
            total_open,
            total_promoted,
            total_discarded,
            kinds,
            profiles: Vec::new(),
        });
        self
    }

    pub(crate) fn build(self) -> Model {
        let mut model = Model::new(self.snapshot);
        model.update(Action::Focus(self.focus));
        model.glyphs = self.glyphs;
        model
    }
}
