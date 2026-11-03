//! Focused CLI/TUI-service parity for guided planning operations (`A1-439`).
//!
//! The exhaustive surface census and PTY journeys remain in `A1-443`. These
//! cases pin the delivered slice to the existing commands: accepted edits land
//! the same bytes, while the high-risk structural refusals reject before any
//! file changes on both paths.

#![cfg(feature = "tui")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aplan_app::{
    CheckRequest, CommitOutcome, ContextInputs, DependencyOperation, DropRequest, EdgeMode,
    MoveRequest, PlanContext, PlanningDependencyRequest, PlanningRequest, TagRequest, Trust,
};

use crate::common;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    plan: PathBuf,
}

impl Fixture {
    fn new(config: &str, first_cut_members: &str, second_cut_status: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("planning-parity")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = dir.path().to_path_buf();
        let plan = root.join("aplan");
        std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
        std::fs::write(root.join(".aplan.yml"), format!("root: aplan\n{config}")).expect("config");
        write_task(
            &plan,
            "A-1",
            "first",
            "todo",
            "phase: 1\neffort_days: 1\nblocked_by: []\ntags: [legacy, priority-p0]\n",
        );
        write_task(
            &plan,
            "A-2",
            "second",
            "todo",
            "phase: 1\neffort_days: 1\nblocked_by: []\ntags: [legacy, priority-p0]\n",
        );
        write_task(
            &plan,
            "A-3",
            "third",
            "todo",
            "phase: 1\neffort_days: 1\nblocked_by: []\ntags: [priority-p0]\n",
        );
        write_task(
            &plan,
            "A-4",
            "cross-cutting",
            "todo",
            "phase: 1\neffort_days: 1\nblocked_by: []\ntags: [cross-cutting]\n",
        );
        write_task(
            &plan,
            "D-1",
            "cancelled",
            "dropped",
            "phase: 1\nblocked_by: []\ntags: []\n",
        );
        write_task(
            &plan,
            "M-1",
            "first cut",
            "todo",
            &format!("phase: 1\nblocked_by: [{first_cut_members}]\ntags: [milestone, v1]\n"),
        );
        write_task(
            &plan,
            "M-2",
            "second cut",
            second_cut_status,
            "phase: 2\nblocked_by: []\ntags: [milestone, v2]\n",
        );
        Self {
            _dir: dir,
            root,
            plan,
        }
    }

    fn context(&self, options: aplan_core::Options, verdict: aplan_core::Verdict) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(&self.plan),
            ContextInputs {
                repository_root: self.root.clone(),
                configuration_file: Some(self.root.join(".aplan.yml")),
                trust: Trust::Hermetic,
                options,
                verdict,
            },
        )
    }

    fn bytes(&self) -> BTreeMap<String, String> {
        std::fs::read_dir(self.plan.join("tasks"))
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
            .collect()
    }

    fn observed_state(&self) -> (BTreeMap<String, String>, bool, usize, Vec<String>, String) {
        let context = self.context(
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        );
        let response = aplan_app::check(&context, CheckRequest);
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
            self.bytes(),
            response.denied,
            response.report.tasks_checked,
            diagnostics,
            format!("{:?}", response.evidence_unresolved),
        )
    }
}

fn write_task(plan: &Path, id: &str, title: &str, status: &str, fields: &str) {
    std::fs::write(
        plan.join(format!("tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {title}\nstatus: {status}\n{fields}---\n\n## Why\n\nFixture.\n"
        ),
    )
    .expect("task");
}

fn apply(fixture: &Fixture, request: &PlanningRequest) {
    let context = fixture.context(
        aplan_core::Options::default(),
        aplan_core::Verdict::default(),
    );
    let preview = aplan_tui::planning::preview(&context, request).expect("TUI preview");
    assert!(matches!(
        aplan_app::commit_change_set(&context, preview.into_change_set()),
        CommitOutcome::Applied(_)
    ));
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: accepted CLI and
/// TUI planning requests leave the complete task-file set byte-identical.
fn cli_and_typed_tui_planning_operations_publish_the_same_plan_bytes() {
    let cli = Fixture::new("", "A-1, A-2, A-3", "todo");
    let tui = Fixture::new("", "A-1, A-2, A-3", "todo");
    let operations: Vec<(Vec<&str>, PlanningRequest)> = vec![
        (
            vec!["dep", "add", "A-2", "--after", "A-1"],
            PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
        ),
        (
            vec!["dep", "rm", "A-2", "--after", "A-1"],
            PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-2".to_string(),
                after: "A-1".to_string(),
                operation: DependencyOperation::Remove,
                mode: EdgeMode::Blocking,
            }),
        ),
        (
            vec!["dep", "add", "A-3", "--after", "A-2", "--prefer"],
            PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-3".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Preference,
            }),
        ),
        (
            vec!["dep", "rm", "A-3", "--after", "A-2", "--prefer"],
            PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-3".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Remove,
                mode: EdgeMode::Preference,
            }),
        ),
        (
            vec!["tag", "add", "A-1", "custom"],
            PlanningRequest::Tag(TagRequest::Add {
                id: "A-1".to_string(),
                tag: "custom".to_string(),
            }),
        ),
        (
            vec!["tag", "rm", "A-1", "custom"],
            PlanningRequest::Tag(TagRequest::Remove {
                id: "A-1".to_string(),
                tag: "custom".to_string(),
            }),
        ),
        (
            vec!["tag", "rename", "legacy", "archived"],
            PlanningRequest::Tag(TagRequest::Rename {
                from: "legacy".to_string(),
                to: "archived".to_string(),
            }),
        ),
        (
            vec!["mv", "A-1", "--cut", "v2", "--phase", "2"],
            PlanningRequest::Move(MoveRequest {
                ids: vec!["A-1".to_string()],
                cut: Some("v2".to_string()),
                phase: Some(2),
            }),
        ),
        (
            vec!["drop", "A-4"],
            PlanningRequest::Drop(DropRequest {
                ids: vec!["A-4".to_string()],
                by_cut: false,
            }),
        ),
        (
            vec!["drop", "M-1", "--cut"],
            PlanningRequest::Drop(DropRequest {
                ids: vec!["M-1".to_string()],
                by_cut: true,
            }),
        ),
    ];
    for (arguments, request) in operations {
        let output = common::run(&cli.root, &arguments);
        assert_eq!(output.code, 0, "{}", output.text());
        apply(&tui, &request);
        assert_eq!(
            cli.observed_state(),
            tui.observed_state(),
            "complete post-state diverged after {arguments:?}"
        );
    }
}

fn denied_code(
    fixture: &Fixture,
    request: &PlanningRequest,
    options: aplan_core::Options,
    verdict: aplan_core::Verdict,
) -> String {
    let context = fixture.context(options, verdict);
    let preview = aplan_tui::planning::preview(&context, request).expect("denied preview");
    assert_eq!(
        preview.change_set().disposition(),
        aplan_app::ChangeDisposition::Denied
    );
    preview
        .change_set()
        .diagnostics()
        .added
        .first()
        .expect("finding")
        .code
        .to_string()
}

#[test]
fn cycle_missing_dropped_and_tag_family_refusals_match_cli_without_writes() {
    for (arguments, request, expected) in [
        (
            vec!["dep", "add", "A-1", "--after", "A-2"],
            PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-1".to_string(),
                after: "A-2".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
            "APLAN004",
        ),
        (
            vec!["dep", "add", "A-2", "--after", "missing"],
            PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-2".to_string(),
                after: "missing".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
            "APLAN003",
        ),
        (
            vec!["dep", "add", "A-2", "--after", "D-1"],
            PlanningRequest::Dependency(PlanningDependencyRequest {
                id: "A-2".to_string(),
                after: "D-1".to_string(),
                operation: DependencyOperation::Add,
                mode: EdgeMode::Blocking,
            }),
            "APLAN020",
        ),
    ] {
        let cli = Fixture::new("", "A-1, A-2, A-3", "todo");
        let tui = Fixture::new("", "A-1, A-2, A-3", "todo");
        if expected == "APLAN004" {
            assert_eq!(
                common::run(&cli.root, &["dep", "add", "A-2", "--after", "A-1"]).code,
                0
            );
            apply(
                &tui,
                &PlanningRequest::Dependency(PlanningDependencyRequest {
                    id: "A-2".to_string(),
                    after: "A-1".to_string(),
                    operation: DependencyOperation::Add,
                    mode: EdgeMode::Blocking,
                }),
            );
        }
        let cli_before = cli.bytes();
        let tui_before = tui.bytes();
        let output = common::run(&cli.root, &arguments);
        assert_ne!(output.code, 0, "{}", output.text());
        assert!(output.text().contains(expected), "{}", output.text());
        assert_eq!(
            denied_code(
                &tui,
                &request,
                aplan_core::Options::default(),
                aplan_core::Verdict::default()
            ),
            expected
        );
        assert_eq!(cli.bytes(), cli_before);
        assert_eq!(tui.bytes(), tui_before);
        assert_eq!(
            cli.observed_state(),
            tui.observed_state(),
            "denied post-state or findings diverged for {arguments:?}"
        );
    }

    let config = "tag_families:\n  priority:\n    members: [priority-p0, priority-p1]\n    require: exactly-one\n";
    let cli = Fixture::new(config, "A-1, A-2, A-3", "todo");
    let tui = Fixture::new(config, "A-1, A-2, A-3", "todo");
    let output = common::run(&cli.root, &["tag", "add", "A-1", "priority-p1"]);
    assert_ne!(output.code, 0, "{}", output.text());
    assert!(output.text().contains("APLAN030"), "{}", output.text());
    let mut options = aplan_core::Options::default();
    options.tag_families.insert(
        "priority".to_string(),
        aplan_core::TagFamily {
            members: vec!["priority-p0".to_string(), "priority-p1".to_string()],
            require: aplan_core::Cardinality::ExactlyOne,
            status: aplan_core::StatusScope::Any,
            milestones: aplan_core::MilestoneScope::Include,
        },
    );
    assert_eq!(
        denied_code(
            &tui,
            &PlanningRequest::Tag(TagRequest::Add {
                id: "A-1".to_string(),
                tag: "priority-p1".to_string(),
            }),
            options,
            aplan_core::Verdict::default()
        ),
        "APLAN030"
    );
    assert_eq!(cli.observed_state(), tui.observed_state());
}

#[test]
fn settled_cut_and_broad_change_policy_refusals_match_cli_without_writes() {
    let cli = Fixture::new("", "A-1", "done");
    let tui = Fixture::new("", "A-1", "done");
    let request = PlanningRequest::Move(MoveRequest {
        ids: vec!["A-1".to_string()],
        cut: Some("M-2".to_string()),
        phase: None,
    });
    let cli_before = cli.bytes();
    let tui_before = tui.bytes();
    let output = common::run(&cli.root, &["mv", "A-1", "--cut", "M-2"]);
    assert_ne!(output.code, 0, "{}", output.text());
    let error = aplan_tui::planning::preview(
        &tui.context(
            aplan_core::Options::default(),
            aplan_core::Verdict::default(),
        ),
        &request,
    )
    .expect_err("settled cut refused");
    assert!(output.text().contains("settled cut"), "{}", output.text());
    assert!(error.to_string().contains("settled cut"), "{error}");
    assert_eq!(cli.bytes(), cli_before);
    assert_eq!(tui.bytes(), tui_before);
    assert_eq!(cli.observed_state(), tui.observed_state());

    let cli = Fixture::new("deny_warnings: true\n", "A-1", "todo");
    let tui = Fixture::new("deny_warnings: true\n", "A-1", "todo");
    let output = common::run(&cli.root, &["mv", "A-1", "--cut", "M-2"]);
    assert_ne!(output.code, 0, "{}", output.text());
    assert!(output.text().contains("APLAN021"), "{}", output.text());
    assert_eq!(
        denied_code(
            &tui,
            &PlanningRequest::Move(MoveRequest {
                ids: vec!["A-1".to_string()],
                cut: Some("M-2".to_string()),
                phase: None,
            }),
            aplan_core::Options::default(),
            aplan_core::Verdict {
                deny_warnings: true,
                advisory: Vec::new(),
            }
        ),
        "APLAN021"
    );
    assert_eq!(cli.observed_state(), tui.observed_state());
}
