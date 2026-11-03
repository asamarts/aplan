//! Focused CLI/TUI parity for the first interactive mutation surface (`A1-438`).
//!
//! The exhaustive mutation census and PTY journeys belong to `A1-443`. This
//! test proves the concrete quick-action slice now: a TUI form request and the
//! real commands leave the same durable status/claim meaning, and both reject
//! terminal transitions and unnamed takeovers.

#![cfg(feature = "tui")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aplan_app::{ContextInputs, PlanContext, Trust, WorkActionCommitOutcome, WorkActionKind};
use aplan_tui::form::{FormAction, FormTransition};
use aplan_tui::quick_actions::{QuickActionWorkflow, form, request};

use crate::common;

const STATUS_CASES: [(&str, &str); 5] = [
    ("T-live", "todo"),
    ("T-progress", "in_progress"),
    ("T-blocked", "blocked"),
    ("T-done", "done"),
    ("T-dropped", "dropped"),
];

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    plan: PathBuf,
}

impl Fixture {
    fn new(prefix: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = dir.path().to_path_buf();
        let plan = root.join("aplan");
        std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
        std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("config");
        for (id, status, evidence) in [
            ("T-live", "todo", ""),
            ("T-progress", "in_progress", ""),
            ("T-blocked", "blocked", ""),
            (
                "T-done",
                "done",
                "evidence:\n  - kind: note\n    ref: 2026-10-25 reviewed\n",
            ),
            ("T-dropped", "dropped", ""),
        ] {
            std::fs::write(
                plan.join(format!("tasks/{id}.md")),
                format!(
                    "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: {status}\neffort_days: 1\npaths: [src/**]\ntags: [cross-cutting]\n{evidence}---\n"
                ),
            )
            .expect("task");
        }
        std::fs::create_dir_all(root.join("src")).expect("source");
        std::fs::write(root.join("src/lib.rs"), "pub fn work() {}\n").expect("source");
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "Fixture"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        Self {
            _dir: dir,
            root,
            plan,
        }
    }

    fn context(&self) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(&self.plan),
            ContextInputs {
                repository_root: self.root.clone(),
                configuration_file: Some(self.root.join(".aplan.yml")),
                trust: Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
    }

    fn task(&self, id: &str) -> String {
        std::fs::read_to_string(self.plan.join(format!("tasks/{id}.md"))).expect("task")
    }

    fn observed_state(&self) -> (BTreeMap<String, String>, bool, usize, Vec<String>, String) {
        let context = self.context();
        let response = aplan_app::check(&context, aplan_app::CheckRequest);
        let tasks = std::fs::read_dir(self.plan.join("tasks"))
            .expect("tasks")
            .map(|entry| entry.expect("entry").path())
            .map(|path| {
                (
                    path.file_name()
                        .expect("name")
                        .to_string_lossy()
                        .into_owned(),
                    std::fs::read_to_string(path).expect("task"),
                )
            })
            .collect();
        let diagnostics = response
            .report
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let file = diagnostic.file.as_ref().map(|path| {
                    path.strip_prefix(&self.root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .into_owned()
                });
                format!(
                    "{}|{:?}|{:?}|{:?}|{}|{:?}|{:?}|{:?}|{:?}",
                    diagnostic.code,
                    diagnostic.severity,
                    diagnostic.task,
                    file,
                    diagnostic.message,
                    diagnostic.agent_instruction,
                    diagnostic.repair,
                    diagnostic.location,
                    diagnostic.detail,
                )
            })
            .collect();
        (
            tasks,
            response.denied,
            response.report.tasks_checked,
            diagnostics,
            format!("{:?}", response.evidence_unresolved),
        )
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

fn tui_claim(fixture: &Fixture, as_name: &str) {
    let context = fixture.context();
    let catalogue =
        aplan_app::work_action_catalogue(&context, "T-live", Some(as_name)).expect("catalogue");
    let mut form = form(&catalogue, QuickActionWorkflow::Claim)
        .expect("form definition")
        .expect("claim form");
    let _ = form.reduce(FormAction::Insert(as_name.to_string()));
    let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
        panic!("claim form must request a preview")
    };
    let request =
        request(&catalogue, QuickActionWorkflow::Claim, Some(&submission)).expect("claim request");
    let preview = aplan_app::preview_work_action(&context, &request).expect("claim preview");
    assert!(matches!(
        aplan_app::commit_work_action(&context, preview),
        WorkActionCommitOutcome::Coordination(aplan_app::CoordinationOutcome::Applied(_))
    ));
}

fn tui_zero_input(fixture: &Fixture, workflow: QuickActionWorkflow) {
    let context = fixture.context();
    let catalogue =
        aplan_app::work_action_catalogue(&context, "T-live", Some("alex")).expect("catalogue");
    let request = request(&catalogue, workflow, None).expect("available quick action");
    let preview = aplan_app::preview_work_action(&context, &request).expect("preview");
    match aplan_app::commit_work_action(&context, preview) {
        WorkActionCommitOutcome::Plan { outcome, .. }
            if matches!(*outcome, aplan_app::CommitOutcome::Applied(_)) => {}
        WorkActionCommitOutcome::Coordination(aplan_app::CoordinationOutcome::Applied(_)) => {}
        other => panic!("quick action was not applied: {other:?}"),
    }
}

fn claim_record(root: &Path, id: &str) -> Option<aplan_core::ClaimRecord> {
    let path = root.join(".git/aplan/claims-v1.json");
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("claim store: {error}"),
    };
    let value: serde_json::Value = serde_json::from_str(&text).expect("claim JSON");
    let claim = &value["claims"][id];
    if claim.is_null() {
        return None;
    }
    Some(serde_json::from_value(claim.clone()).expect("complete claim record"))
}

fn assert_equivalent_claims(cli: &Fixture, tui: &Fixture, id: &str) {
    match (claim_record(&cli.root, id), claim_record(&tui.root, id)) {
        (None, None) => {}
        (Some(cli_claim), Some(tui_claim)) => {
            assert_eq!(cli_claim.holder, tui_claim.holder);
            assert_eq!(cli_claim.identity_source, tui_claim.identity_source);
            assert_eq!(cli_claim.origin.host, tui_claim.origin.host);
            assert!(
                cli_claim
                    .claimed_at_epoch_seconds
                    .abs_diff(tui_claim.claimed_at_epoch_seconds)
                    <= 2,
                "adapter clocks diverged: {cli_claim:?} != {tui_claim:?}"
            );
            assert_eq!(
                cli_claim.origin.worktree.as_deref(),
                Some(
                    cli.root
                        .canonicalize()
                        .expect("CLI root")
                        .to_string_lossy()
                        .as_ref()
                )
            );
            assert_eq!(
                tui_claim.origin.worktree.as_deref(),
                Some(
                    tui.root
                        .canonicalize()
                        .expect("TUI root")
                        .to_string_lossy()
                        .as_ref()
                )
            );
        }
        claims => panic!("claim presence diverged: {claims:?}"),
    }
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: the CLI and TUI
/// typed work actions leave exact status and claim state equal.
fn cli_and_tui_claim_start_stop_release_leave_the_same_state() {
    let cli = Fixture::new("work-action-cli");
    let tui = Fixture::new("work-action-tui");

    let claimed = common::run(&cli.root, &["claim", "T-live", "--as", "alex"]);
    assert_eq!(claimed.code, 0, "{}", claimed.text());
    tui_claim(&tui, "alex");
    assert_equivalent_claims(&cli, &tui, "T-live");
    assert_eq!(cli.observed_state(), tui.observed_state());

    let started = common::run(&cli.root, &["start", "T-live"]);
    assert_eq!(started.code, 0, "{}", started.text());
    tui_zero_input(&tui, QuickActionWorkflow::Start);
    assert_eq!(cli.observed_state(), tui.observed_state());

    let stopped = common::run(&cli.root, &["stop", "T-live"]);
    assert_eq!(stopped.code, 0, "{}", stopped.text());
    tui_zero_input(&tui, QuickActionWorkflow::Stop);
    assert_eq!(cli.observed_state(), tui.observed_state());

    let released = common::run(&cli.root, &["release", "T-live"]);
    assert_eq!(released.code, 0, "{}", released.text());
    tui_zero_input(&tui, QuickActionWorkflow::Release);
    assert_eq!(claim_record(&cli.root, "T-live"), None);
    assert_eq!(claim_record(&tui.root, "T-live"), None);
    assert_eq!(cli.observed_state(), tui.observed_state());
}

#[test]
fn cli_and_typed_tui_boundary_refuse_terminal_transitions_and_unnamed_takeovers() {
    let fixture = Fixture::new("work-action-refusal");
    let context = fixture.context();
    for id in ["T-done", "T-dropped"] {
        let cli = common::run(&fixture.root, &["start", id]);
        assert_ne!(cli.code, 0, "terminal CLI start succeeded: {}", cli.text());
        let tui = aplan_app::preview_work_action(
            &context,
            &aplan_app::WorkActionRequest::simple(id, WorkActionKind::Start),
        );
        assert!(
            matches!(tui, Err(aplan_app::WorkActionError::Arguments(ref message)) if message.contains("terminal")),
            "terminal TUI preview was not refused: {tui:?}"
        );
    }

    let cli_claim = common::run(&fixture.root, &["claim", "T-live", "--as", "alex"]);
    assert_eq!(cli_claim.code, 0, "{}", cli_claim.text());
    let context = fixture.context();
    let direct = aplan_app::preview_work_action(
        &context,
        &aplan_app::WorkActionRequest {
            task: "T-live".to_string(),
            action: WorkActionKind::Claim,
            as_name: Some("blair".to_string()),
            from: None,
            claimed_at_epoch_seconds: Some(10),
        },
    );
    let cli_takeover = common::run(&fixture.root, &["claim", "T-live", "--as", "blair"]);
    assert_ne!(cli_takeover.code, 0, "unnamed CLI takeover succeeded");
    assert!(
        matches!(
            direct,
            Err(aplan_app::WorkActionError::Claim(
                aplan_app::claims::ClaimError::Invalid { .. }
            ))
        ),
        "unnamed TUI takeover was not refused: {direct:?}"
    );
    assert_eq!(
        claim_record(&fixture.root, "T-live").map(|claim| claim.holder),
        Some("alex".to_string())
    );
}

#[test]
fn cli_and_typed_tui_start_and_stop_cover_every_status() {
    for (action, expected_success) in [
        ("start", [true, true, true, false, false]),
        ("stop", [true, true, false, false, false]),
    ] {
        let cli = Fixture::new(&format!("work-action-{action}-cli"));
        let tui = Fixture::new(&format!("work-action-{action}-tui"));
        for ((id, status), success) in STATUS_CASES.into_iter().zip(expected_success) {
            let before = tui.task(id);
            let cli_result = common::run(&cli.root, &[action, id]);
            assert_eq!(
                cli_result.code == 0,
                success,
                "CLI {action} on {status}: {}",
                cli_result.text()
            );
            let kind = if action == "start" {
                WorkActionKind::Start
            } else {
                WorkActionKind::Stop
            };
            let context = tui.context();
            let typed = aplan_app::preview_work_action(
                &context,
                &aplan_app::WorkActionRequest::simple(id, kind),
            );
            assert_eq!(
                typed.is_ok(),
                success,
                "typed {action} on {status}: {typed:?}"
            );
            if let Ok(preview) = typed {
                assert!(matches!(
                    aplan_app::commit_work_action(&context, preview),
                    WorkActionCommitOutcome::Plan { outcome, .. }
                        if matches!(
                            *outcome,
                            aplan_app::CommitOutcome::Applied(_)
                                | aplan_app::CommitOutcome::NoOp(_)
                        )
                ));
            } else {
                assert_eq!(tui.task(id), before, "refused typed action wrote {id}");
            }
            assert_eq!(cli.task(id), tui.task(id), "{action} diverged for {status}");
        }
    }
}

#[test]
fn cli_and_typed_tui_claim_and_release_cover_every_status() {
    let cli = Fixture::new("work-action-claim-status-cli");
    let tui = Fixture::new("work-action-claim-status-tui");
    for (id, status) in STATUS_CASES {
        let cli_result = common::run(&cli.root, &["claim", id, "--as", "alex"]);
        let context = tui.context();
        let typed = aplan_app::preview_work_action(
            &context,
            &aplan_app::WorkActionRequest {
                task: id.to_string(),
                action: WorkActionKind::Claim,
                as_name: Some("alex".to_string()),
                from: None,
                claimed_at_epoch_seconds: None,
            },
        );
        let live = matches!(status, "todo" | "in_progress" | "blocked");
        assert_eq!(cli_result.code == 0, live, "CLI claim on {status}");
        assert_eq!(typed.is_ok(), live, "typed claim on {status}: {typed:?}");
        if let Ok(preview) = typed {
            assert!(matches!(
                aplan_app::commit_work_action(&context, preview),
                WorkActionCommitOutcome::Coordination(aplan_app::CoordinationOutcome::Applied(_))
            ));
        }
        assert_equivalent_claims(&cli, &tui, id);
    }

    for (id, status) in STATUS_CASES {
        let cli_result = common::run(&cli.root, &["release", id]);
        assert_eq!(
            cli_result.code,
            0,
            "CLI release on {status}: {}",
            cli_result.text()
        );
        let context = tui.context();
        let preview = aplan_app::preview_work_action(
            &context,
            &aplan_app::WorkActionRequest::simple(id, WorkActionKind::Release),
        )
        .expect("release preview is status-independent");
        assert!(matches!(
            aplan_app::commit_work_action(&context, preview),
            WorkActionCommitOutcome::Coordination(
                aplan_app::CoordinationOutcome::Applied(_)
                    | aplan_app::CoordinationOutcome::NoOp(_)
            )
        ));
        assert_eq!(claim_record(&cli.root, id), None);
        assert_eq!(claim_record(&tui.root, id), None);
    }
}

#[test]
fn cli_and_typed_tui_release_recover_orphans_and_refuse_unknown_ids() {
    let cli = Fixture::new("work-action-orphan-cli");
    let tui = Fixture::new("work-action-orphan-tui");
    let claimed = common::run(&cli.root, &["claim", "T-live", "--as", "alex"]);
    assert_eq!(claimed.code, 0, "{}", claimed.text());
    tui_claim(&tui, "alex");

    std::fs::remove_file(cli.plan.join("tasks/T-live.md")).expect("remove CLI task");
    std::fs::remove_file(tui.plan.join("tasks/T-live.md")).expect("remove TUI task");
    let released = common::run(&cli.root, &["release", "T-live"]);
    assert_eq!(released.code, 0, "{}", released.text());
    let context = tui.context();
    let preview = aplan_app::preview_work_action(
        &context,
        &aplan_app::WorkActionRequest::simple("T-live", WorkActionKind::Release),
    )
    .expect("typed release preserves orphan recovery");
    assert!(matches!(
        aplan_app::commit_work_action(&context, preview),
        WorkActionCommitOutcome::Coordination(aplan_app::CoordinationOutcome::Applied(_))
    ));
    assert_eq!(claim_record(&cli.root, "T-live"), None);
    assert_eq!(claim_record(&tui.root, "T-live"), None);

    let cli_unknown = common::run(&cli.root, &["release", "T-live"]);
    assert_ne!(cli_unknown.code, 0, "missing CLI task and claim succeeded");
    let typed_unknown = aplan_app::preview_work_action(
        &tui.context(),
        &aplan_app::WorkActionRequest::simple("T-live", WorkActionKind::Release),
    );
    assert!(
        matches!(typed_unknown, Err(aplan_app::WorkActionError::Arguments(ref message)) if message.contains("no task or local claim")),
        "missing typed task and claim was not refused: {typed_unknown:?}"
    );
}
