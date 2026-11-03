//! CLI/TUI parity for capture and inbox lifecycle composition (`A1-518`).

#![cfg(feature = "tui")]

use std::path::PathBuf;

use aplan_app::{
    CaptureSelector, CommitOutcome, ContextInputs, ExactCaptureId, PlanContext, Trust,
};
use aplan_core::capture::{Capture, CaptureCreate, CaptureOrigin, SystemClock};
use aplan_tui::form::{FormAction, FormFocus, FormSubmission, FormTransition};
use aplan_tui::mutations::{MutationAction, MutationCandidate, PreparedMutation};

use crate::common;

const CAPTURE_ID: &str = "I-01K8Z3A4H00000000000000000";

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    plan: PathBuf,
}

impl Fixture {
    fn new(prefix: &str) -> Self {
        let directory = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let root = directory.path().to_path_buf();
        let plan = root.join("aplan");
        std::fs::create_dir_all(plan.join("tasks")).expect("tasks");
        std::fs::create_dir_all(root.join("src")).expect("source");
        std::fs::write(root.join("src/lib.rs"), "pub fn proof() {}\n").expect("source");
        std::fs::write(
            root.join(".aplan.yml"),
            concat!(
                "root: aplan\n",
                "capture:\n",
                "  kinds: [question]\n",
                "  profiles:\n",
                "    triage:\n",
                "      kind: question\n",
                "      labels: [profiled]\n",
            ),
        )
        .expect("configuration");
        Self {
            _directory: directory,
            root,
            plan,
        }
    }

    fn context(&self) -> PlanContext {
        let config = aplan_config::load_config(&self.root).expect("configuration");
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
        .with_captured_report_config(config)
        .expect("captured configuration")
    }

    fn seed(&self) {
        let context = self.context();
        let preview = aplan_app::preview_capture_create_with_id(
            &context,
            &CaptureCreate {
                title: "seed capture".to_string(),
                kind: Some("task".to_string()),
                body: Some("seed body".to_string()),
                labels: Some(vec!["seed".to_string()]),
                revisit_at: None,
                profile: None,
                promotion_defaults: None,
                source: None,
                origin: CaptureOrigin::Tui,
            },
            &SystemClock,
            CAPTURE_ID,
        )
        .expect("seed preview");
        assert!(matches!(
            aplan_app::commit_change_set(&context, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));
    }

    fn capture(&self) -> Capture {
        aplan_app::show_capture(
            &self.context(),
            &CaptureSelector::Exact(ExactCaptureId::new(CAPTURE_ID).expect("capture id")),
            &SystemClock,
        )
        .expect("focused capture")
        .capture
    }

    fn task_bytes(&self, id: &str) -> Option<String> {
        std::fs::read_to_string(self.plan.join(format!("tasks/{id}.md"))).ok()
    }
}

fn set_current(form: &mut aplan_tui::form::MutationForm, values: &[&str]) {
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
        aplan_tui::mutations::prepare(context, action, selected).expect("native capture form")
    else {
        panic!("capture workflow unexpectedly skipped its form")
    };
    let field_count = form.fields().len();
    for index in 0..field_count {
        assert_eq!(form.focus(), FormFocus::Field(index));
        let key = form.fields()[index].spec().key().to_string();
        if let Some((_, values)) = values.iter().find(|(candidate, _)| *candidate == key) {
            set_current(&mut form, values);
        }
        form.reduce(FormAction::NextFocus);
    }
    let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
        panic!("complete capture form did not request a preview")
    };
    submission
}

fn apply_tui(
    fixture: &Fixture,
    action: MutationAction,
    selected: Option<&str>,
    values: &[(&str, &[&str])],
) {
    let context = fixture.context();
    let submission = submission(&context, action, selected, values);
    let candidate = aplan_tui::mutations::preview(&context, action, selected, &submission, None)
        .expect("native capture preview");
    let MutationCandidate::Plan {
        action: returned,
        change,
        ..
    } = candidate
    else {
        panic!("capture action returned a coordination mutation")
    };
    assert_eq!(returned, action);
    assert!(matches!(
        aplan_app::commit_change_set(&context, *change),
        CommitOutcome::Applied(_)
    ));
}

fn run_cli(fixture: &Fixture, arguments: &[&str]) {
    let output = common::run(&fixture.root, arguments);
    assert_eq!(output.code, 0, "{}", output.text());
}

fn normalized(mut capture: Capture) -> Capture {
    assert!(
        aplan_core::capture::parse_capture_instant(&capture.captured_at).is_some(),
        "invalid captured_at: {}",
        capture.captured_at
    );
    capture.captured_at = "CAPTURED".to_string();
    capture.disposed_at = capture.disposed_at.map(|disposed_at| {
        assert!(
            aplan_core::capture::parse_capture_instant(&disposed_at).is_some(),
            "invalid disposed_at: {disposed_at}"
        );
        "DISPOSED".to_string()
    });
    capture
}

fn assert_lifecycle_parity(
    action: MutationAction,
    cli_arguments: &[&str],
    values: &[(&str, &[&str])],
    prepare: impl Fn(&Fixture),
) -> (Fixture, Fixture) {
    let cli = Fixture::new("capture-parity-cli");
    let tui = Fixture::new("capture-parity-tui");
    prepare(&cli);
    prepare(&tui);
    run_cli(&cli, cli_arguments);
    apply_tui(&tui, action, Some(CAPTURE_ID), values);
    assert_eq!(
        normalized(cli.capture()),
        normalized(tui.capture()),
        "{action:?} retained capture diverged"
    );
    (cli, tui)
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: the minimal
/// capture form and all four retained-record lifecycle operations cross the
/// production form/preview boundary and agree with the CLI post-state.
fn native_capture_and_every_inbox_lifecycle_match_cli_post_state() {
    let cli = Fixture::new("capture-create-parity-cli");
    let tui = Fixture::new("capture-create-parity-tui");
    run_cli(
        &cli,
        &[
            "capture",
            "new intake",
            "--profile",
            "triage",
            "--body",
            "pasted detail",
            "--label",
            "explicit",
            "--revisit-at",
            "2030-01-01T00:00:00Z",
        ],
    );
    apply_tui(
        &tui,
        MutationAction::CaptureCreate,
        None,
        &[
            ("title", &["new intake"]),
            ("profile", &["triage"]),
            ("body", &["pasted detail"]),
            ("labels", &["explicit"]),
            ("revisit_at", &["2030-01-01T00:00:00Z"]),
        ],
    );
    let mut cli_capture = aplan_app::capture_workspace(&cli.context(), &SystemClock)
        .expect("CLI workspace")
        .captures
        .pop()
        .expect("CLI capture");
    let mut tui_capture = aplan_app::capture_workspace(&tui.context(), &SystemClock)
        .expect("TUI workspace")
        .captures
        .pop()
        .expect("TUI capture");
    assert_eq!(cli_capture.origin, CaptureOrigin::Cli);
    assert_eq!(tui_capture.origin, CaptureOrigin::Tui);
    cli_capture.id.clear();
    tui_capture.id.clear();
    cli_capture.captured_at.clear();
    tui_capture.captured_at.clear();
    cli_capture.origin = CaptureOrigin::Tui;
    assert_eq!(cli_capture, tui_capture);

    assert_lifecycle_parity(
        MutationAction::CaptureAmend,
        &[
            "inbox",
            "amend",
            CAPTURE_ID,
            "--title",
            "amended",
            "--kind",
            "question",
            "--body",
            "replacement",
            "--label",
            "a",
            "--label",
            "b",
            "--revisit-at",
            "2030-01-01T00:00:00Z",
            "--promotion-phase",
            "1",
            "--promotion-effort",
            "2",
            "--promotion-path",
            "src/**",
            "--promotion-tag",
            "cross-cutting",
        ],
        &[
            ("title", &["amended"]),
            ("kind", &["question"]),
            ("body", &["replacement"]),
            ("labels", &["a", "b"]),
            ("revisit_at", &["2030-01-01T00:00:00Z"]),
            ("phase", &["1"]),
            ("effort", &["2"]),
            ("paths", &["src/**"]),
            ("tags", &["cross-cutting"]),
        ],
        Fixture::seed,
    );

    assert_lifecycle_parity(
        MutationAction::CaptureDiscard,
        &["inbox", "discard", CAPTURE_ID, "--reason", "later"],
        &[("reason", &["later"])],
        Fixture::seed,
    );

    assert_lifecycle_parity(
        MutationAction::CaptureRestore,
        &["inbox", "restore", CAPTURE_ID],
        &[],
        |fixture| {
            fixture.seed();
            let context = fixture.context();
            let preview = aplan_app::preview_capture_discard(
                &context,
                &aplan_app::CaptureDiscardRequest {
                    selector: CaptureSelector::Exact(
                        ExactCaptureId::new(CAPTURE_ID).expect("capture id"),
                    ),
                    reason: Some("later".to_string()),
                },
                &SystemClock,
            )
            .expect("discard preview");
            assert!(matches!(
                aplan_app::commit_change_set(&context, preview.into_change_set()),
                CommitOutcome::Applied(_)
            ));
        },
    );

    let (cli, tui) = assert_lifecycle_parity(
        MutationAction::CapturePromote,
        &[
            "inbox",
            "promote",
            CAPTURE_ID,
            "--task-id",
            "T-9",
            "--phase",
            "1",
            "--effort",
            "1",
            "--path",
            "src/**",
            "--tag",
            "cross-cutting",
            "--reason",
            "triaged",
        ],
        &[
            ("id", &["T-9"]),
            ("phase", &["1"]),
            ("effort", &["1"]),
            ("paths", &["src/**"]),
            ("tags", &["cross-cutting"]),
            ("reason", &["triaged"]),
        ],
        Fixture::seed,
    );
    assert_eq!(cli.task_bytes("T-9"), tui.task_bytes("T-9"));
}
