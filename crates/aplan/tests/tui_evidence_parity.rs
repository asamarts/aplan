//! Complete CLI/TUI composition parity for evidence mutations (`A1-443`).

#![cfg(feature = "tui")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aplan_app::{CommitOutcome, ContextInputs, PlanContext, Trust};
use aplan_tui::form::{FormAction, FormFocus, FormSubmission, FormTransition};
use aplan_tui::mutations::{MutationAction, MutationCandidate, PreparedMutation};

use crate::common;

type EvidenceCase = (
    MutationAction,
    Start,
    &'static [(&'static str, &'static [&'static str])],
    &'static [&'static str],
);

#[derive(Clone, Copy)]
enum Start {
    Todo,
    DoneWithFile,
    DoneWithTwoFiles,
    RepairableTest,
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    plan: PathBuf,
}

impl Fixture {
    fn new(prefix: &str, start: Start) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = dir.path().to_path_buf();
        let plan = root.join("aplan");
        std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
        std::fs::write(root.join("proof.md"), "proof one\n").expect("proof");
        std::fs::write(root.join("proof2.md"), "proof two\n").expect("proof");
        std::fs::create_dir_all(root.join("src")).expect("src");
        std::fs::write(root.join("src/lib.rs"), "pub fn evidence() {}\n").expect("source");
        let (config, status, paths, evidence) = match start {
            Start::Todo => (
                "root: aplan\ndetect_drift: true\n",
                "todo",
                "paths: [proof*.md]\n",
                "",
            ),
            Start::DoneWithFile => (
                "root: aplan\ndetect_drift: true\n",
                "done",
                "paths: [proof*.md]\n",
                "evidence:\n  - kind: file\n    ref: proof.md\n",
            ),
            Start::DoneWithTwoFiles => (
                "root: aplan\ndetect_drift: true\n",
                "done",
                "paths: [proof*.md]\n",
                "evidence:\n  - kind: file\n    ref: proof.md\n  - kind: file\n    ref: proof2.md\n",
            ),
            Start::RepairableTest => (
                "root: aplan\ntest:\n  list: [printf, 'suite::known_test\\n']\n  run: ['true', '{test}']\n",
                "done",
                "paths: [src/**]\n",
                "evidence:\n  - kind: test\n    ref: known_test\n",
            ),
        };
        std::fs::write(root.join(".aplan.yml"), config).expect("config");
        std::fs::write(
            plan.join("tasks/A-1.md"),
            format!(
                "---\nid: A-1\ntitle: evidence work\nphase: 1\nstatus: {status}\neffort_days: 1\n{paths}tags: [cross-cutting]\n{evidence}---\n"
            ),
        )
        .expect("task");
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
        let config = aplan_config::load_config(&self.root).expect("config");
        PlanContext::new(
            aplan_core::load_plan_identified(&self.plan),
            ContextInputs {
                repository_root: self.root.clone(),
                configuration_file: Some(self.root.join(".aplan.yml")),
                trust: Trust::RepositoryRead,
                options: aplan_core::Options {
                    detect_drift: config.detect_drift.unwrap_or(false),
                    traces_root: Some(self.root.clone()),
                    ..aplan_core::Options::default()
                },
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_captured_report_config(config)
        .expect("captured config")
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
        let context = self.context();
        let response = aplan_app::check(&context, aplan_app::CheckRequest);
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

fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

fn set_current(form: &mut aplan_tui::form::MutationForm, values: &[&str]) {
    let FormFocus::Field(index) = form.focus() else {
        panic!("field value changed while a control was focused")
    };
    let current = form.fields()[index]
        .values()
        .next()
        .unwrap_or("")
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
        aplan_tui::mutations::prepare(context, action, selected).expect("form")
    else {
        panic!("evidence action unexpectedly had no form")
    };
    for index in 0..form.fields().len() {
        assert_eq!(form.focus(), FormFocus::Field(index));
        let key = form.fields()[index].spec().key().to_string();
        if let Some((_, field_values)) = values.iter().find(|(candidate, _)| *candidate == key) {
            set_current(&mut form, field_values);
        }
        form.reduce(FormAction::NextFocus);
    }
    let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
        panic!(
            "valid evidence form did not request preview: {}",
            form.render(120, 200).snapshot()
        )
    };
    submission
}

fn apply_tui(
    fixture: &Fixture,
    action: MutationAction,
    values: &[(&str, &[&str])],
    verification: Option<&aplan_app::VerificationWorkbench>,
) {
    let context = fixture.context();
    let submission = submission(&context, action, Some("A-1"), values);
    let candidate =
        aplan_tui::mutations::preview(&context, action, Some("A-1"), &submission, verification)
            .expect("TUI preview");
    let MutationCandidate::Plan {
        action: returned,
        change,
        ..
    } = candidate
    else {
        panic!("evidence returned a coordination mutation")
    };
    assert_eq!(returned, action);
    let outcome = aplan_app::commit_change_set(&context, *change);
    assert!(
        matches!(outcome, CommitOutcome::Applied(_) | CommitOutcome::NoOp(_)),
        "{action:?} did not publish: {outcome:?}"
    );
}

fn run_cli(fixture: &Fixture, arguments: &[&str]) {
    let output = common::run(&fixture.root, arguments);
    assert_eq!(output.code, 0, "{}", output.text());
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: close, every
/// amend shape and restamp cross the production form composition and leave the
/// exact task bytes written by the CLI.
fn every_task_local_evidence_action_matches_the_complete_cli_post_state() {
    let cases: &[EvidenceCase] = &[
        (
            MutationAction::Close,
            Start::Todo,
            &[("file", &["proof.md"])],
            &["close", "A-1", "--evidence", "file:proof.md"],
        ),
        (
            MutationAction::AmendAdd,
            Start::DoneWithFile,
            &[("kind", &["file"]), ("reference", &["proof2.md"])],
            &["amend", "A-1", "--add-evidence", "file:proof2.md"],
        ),
        (
            MutationAction::AmendReplace,
            Start::DoneWithFile,
            &[
                ("from_kind", &["file"]),
                ("from_reference", &["proof.md"]),
                ("to_kind", &["file"]),
                ("to_reference", &["proof2.md"]),
            ],
            &[
                "amend",
                "A-1",
                "--replace-evidence",
                "file:proof.md",
                "file:proof2.md",
            ],
        ),
        (
            MutationAction::AmendRemove,
            Start::DoneWithTwoFiles,
            &[("kind", &["file"]), ("reference", &["proof.md"])],
            &["amend", "A-1", "--remove-evidence", "file:proof.md"],
        ),
        (
            MutationAction::Restamp,
            Start::DoneWithFile,
            &[],
            &["close", "A-1", "--restamp"],
        ),
    ];

    for (action, start, values, arguments) in cases {
        let cli = Fixture::new("evidence-parity-cli", *start);
        let tui = Fixture::new("evidence-parity-tui", *start);
        run_cli(&cli, arguments);
        apply_tui(&tui, *action, values, None);
        assert_eq!(
            cli.observed_state(),
            tui.observed_state(),
            "{action:?} complete post-state diverged"
        );
    }
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: the TUI repair
/// form consumes the same retained verifier answer and writes the exact bytes
/// produced by `verify --fix`.
fn verification_fix_matches_the_complete_cli_post_state() {
    let cli = Fixture::new("verify-fix-parity-cli", Start::RepairableTest);
    let tui = Fixture::new("verify-fix-parity-tui", Start::RepairableTest);
    run_cli(&cli, &["verify", "--fix"]);

    let context = tui.context();
    let config = context.report_config().cloned().expect("report config");
    let options = aplan_app::configured_verification(
        &config,
        context.identity().configuration.path.clone(),
        context.options().traces_root.clone(),
    );
    let verification = aplan_app::verify_context(&context, &options).expect("verification");
    assert_eq!(verification.repairs.len(), 1);
    apply_tui(
        &tui,
        MutationAction::VerifyFix,
        &[("review", &["review"])],
        Some(&verification),
    );
    assert_eq!(cli.observed_state(), tui.observed_state());
}
