//! Complete CLI/TUI composition parity for structural authoring (`A1-443`).

#![cfg(feature = "tui")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aplan_app::{CommitOutcome, ContextInputs, PlanContext, Trust};
use aplan_tui::form::{FormAction, FormFocus, FormSubmission, FormTransition};
use aplan_tui::mutations::{MutationAction, MutationCandidate, PreparedMutation};

use crate::common;

type AuthoringCase<'a> = (
    MutationAction,
    Option<&'a str>,
    Vec<(&'a str, Vec<&'a str>)>,
    Vec<String>,
);

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
        write_task(
            &plan,
            "A-1",
            "completed outcome",
            "done",
            "evidence:\n  - kind: note\n    ref: 2026-10-25 reviewed\ntags: [cross-cutting]\n",
        );
        write_task(
            &plan,
            "A-2",
            "live work",
            "todo",
            "effort_days: 1\nblocked_by: [A-1]\ntags: [cross-cutting]\n",
        );
        write_task(
            &plan,
            "M-1",
            "first cut",
            "todo",
            "blocked_by: [A-1, A-2]\ntags: [milestone, v1]\n",
        );
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

    fn bytes(&self) -> BTreeMap<String, String> {
        let mut files = BTreeMap::new();
        collect_files(&self.plan, &self.plan, &mut files);
        files
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

fn write_task(plan: &Path, id: &str, title: &str, status: &str, fields: &str) {
    std::fs::write(
        plan.join(format!("tasks/{id}.md")),
        format!("---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: {status}\n{fields}---\n"),
    )
    .expect("task");
}

fn collect_files(root: &Path, directory: &Path, files: &mut BTreeMap<String, String>) {
    for entry in std::fs::read_dir(directory).expect("directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.insert(
                path.strip_prefix(root)
                    .expect("relative")
                    .to_string_lossy()
                    .into_owned(),
                std::fs::read_to_string(path).expect("UTF-8 plan file"),
            );
        }
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
        panic!("authoring action unexpectedly had no form")
    };
    let field_count = form.fields().len();
    for index in 0..field_count {
        assert_eq!(form.focus(), FormFocus::Field(index));
        let key = form.fields()[index].spec().key().to_string();
        if let Some((_, field_values)) = values.iter().find(|(candidate, _)| *candidate == key) {
            set_current(&mut form, field_values);
        }
        form.reduce(FormAction::NextFocus);
    }
    let FormTransition::PreviewRequested(submission) = form.reduce(FormAction::Review) else {
        panic!(
            "valid authoring form did not request preview: {}",
            form.render(120, 200).snapshot()
        )
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
        .expect("TUI preview");
    let MutationCandidate::Plan {
        action: returned,
        change,
        ..
    } = candidate
    else {
        panic!("authoring returned a coordination mutation")
    };
    assert_eq!(returned, action);
    assert!(matches!(
        aplan_app::commit_change_set(&context, *change),
        CommitOutcome::Applied(_)
    ));
}

fn run_cli(fixture: &Fixture, arguments: &[String]) {
    let borrowed = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let output = common::run(&fixture.root, &borrowed);
    assert_eq!(output.code, 0, "{}", output.text());
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: every authoring
/// action crosses the production form composition and leaves exactly the task
/// tree written by its CLI command.
fn every_authoring_action_matches_the_complete_cli_post_state() {
    let sources = tempfile::Builder::new()
        .prefix("authoring-parity-sources")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("sources");
    let manifest = sources.path().join("manifest.yml");
    std::fs::write(
        &manifest,
        "- id: A-8\n  title: manifest work\n  phase: 1\n  effort: 1\n  tags: [cross-cutting]\n",
    )
    .expect("manifest");
    let checklist = sources.path().join("checklist.md");
    std::fs::write(&checklist, "- [ ] Imported work\n").expect("checklist");
    let manifest_name = manifest.to_string_lossy().into_owned();
    let checklist_name = checklist.to_string_lossy().into_owned();

    let cases: Vec<AuthoringCase<'_>> = vec![
        (
            MutationAction::Add,
            None,
            vec![
                ("title", vec!["new work"]),
                ("id", vec!["A-8"]),
                ("phase", vec!["1"]),
            ],
            vec!["add", "new work", "--id", "A-8", "--phase", "1"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        ),
        (
            MutationAction::Manifest,
            None,
            vec![("source", vec![manifest_name.as_str()])],
            vec![
                "add".to_string(),
                "--from".to_string(),
                manifest_name.clone(),
            ],
        ),
        (
            MutationAction::Import,
            None,
            vec![
                ("source", vec![checklist_name.as_str()]),
                ("format", vec!["checklist"]),
                ("id_prefix", vec!["I"]),
            ],
            vec![
                "import".to_string(),
                checklist_name.clone(),
                "--format".to_string(),
                "checklist".to_string(),
                "--phase".to_string(),
                "1".to_string(),
                "--id-prefix".to_string(),
                "I".to_string(),
            ],
        ),
        (
            MutationAction::Supersede,
            Some("A-1"),
            vec![("title", vec!["replacement work"])],
            vec!["supersede", "A-1", "replacement work"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        ),
        (
            MutationAction::Rekey,
            Some("A-2"),
            vec![("new", vec!["A-9"])],
            vec!["rekey", "A-2", "A-9"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        ),
    ];

    for (action, selected, owned_values, arguments) in cases {
        let cli = Fixture::new("authoring-parity-cli");
        let tui = Fixture::new("authoring-parity-tui");
        let values = owned_values
            .iter()
            .map(|(key, values)| (*key, values.as_slice()))
            .collect::<Vec<_>>();
        run_cli(&cli, &arguments);
        apply_tui(&tui, action, selected, &values);
        assert_eq!(
            cli.observed_state(),
            tui.observed_state(),
            "{action:?} complete post-state diverged"
        );
    }
}
