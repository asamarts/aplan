//! Namespaced, atomic blueprint graph application (`A1-387`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Barrier};

use aplan_app::{
    BlueprintRequest, ChangeDisposition, CommitOutcome, ContextInputs, PlanContext, Trust,
    commit_change_set, preview_blueprint,
};
use aplan_interop::blueprint::{
    SCHEMA, application_keys, bind_parameters, parse, resolve_application,
};

use crate::common;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent directory");
    }
    std::fs::write(path, text).expect("fixture file");
}

fn repository() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("blueprint-application-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("temporary repository");
    let tasks = dir.path().join("aplan/tasks");
    write(&dir.path().join(".aplan.yml"), "root: aplan\n");
    write(
        &tasks.join("A-1.md"),
        "---\nid: A-1\ntitle: existing collision\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/existing.rs]\n---\n",
    );
    write(
        &tasks.join("M-v1.md"),
        "---\nid: M-v1\ntitle: first cut\nphase: 1\nstatus: todo\nblocked_by: [A-1]\ntags: [milestone]\n---\n",
    );
    dir
}

fn context(root: &Path) -> PlanContext {
    PlanContext::new(
        aplan_core::load_plan_identified(&root.join("aplan")),
        ContextInputs {
            repository_root: root.to_path_buf(),
            configuration_file: None,
            trust: Trust::RepositoryRead,
            options: aplan_core::Options::default(),
            verdict: aplan_core::Verdict::default(),
        },
    )
    .with_report_config(aplan_config::Config::default())
}

fn request(contents: &str, cut: &str) -> BlueprintRequest {
    BlueprintRequest {
        source_name: "feature.blueprint.yml".to_string(),
        contents: contents.to_string(),
        bindings: BTreeMap::new(),
        cut: cut.to_string(),
    }
}

fn document(tasks: &str) -> String {
    format!(
        "schema: {SCHEMA}\nname: feature\ntitle: Feature graph\ndescription: Reusable graph.\ntasks:\n{tasks}"
    )
}

/// Proves `blueprint.references_resolve_atomically`: every graph field uses
/// the same explicit namespace grammar, and a local key which is also an
/// existing task id cannot change what either spelling means.
#[test]
fn blueprint_references_are_namespaced_and_total() {
    let source = document(
        "  - key: A-1\n    title: Local collision\n    effort: 1\n\
         \n  - key: graph\n    title: Graph owner\n    blocked_by: [local:A-1]\n\
         \n    blocks: [task:A-1, local:A-1]\n\
         \n    prefer_after: [local:A-1, task:A-1]\n    ratified_by: [local:A-1]\n\
         \n    contains: [local:A-1]\n    tags: [milestone]\n",
    );
    let parsed = parse(Path::new("feature.blueprint.yml"), &source).expect("blueprint");
    let bound = bind_parameters(&parsed, &BTreeMap::new()).expect("no parameters");
    assert_eq!(
        application_keys(&bound).expect("valid graph"),
        ["A-1", "graph"]
    );

    let resolved = resolve_application(
        &bound,
        &BTreeMap::from([
            ("A-1".to_string(), "A-2".to_string()),
            ("graph".to_string(), "A-3".to_string()),
        ]),
        "M-v1",
    )
    .expect("resolve graph");
    let graph = &resolved[1];
    assert_eq!(graph.blocked_by, ["A-2"]);
    assert_eq!(graph.blocks, ["A-1", "A-2"]);
    assert_eq!(graph.prefer_after, ["A-2", "A-1"]);
    assert_eq!(graph.ratified_by, ["A-2"]);
    assert_eq!(graph.contains, ["A-2"]);
    assert_eq!(graph.id.as_deref(), Some("A-3"));
    assert_eq!(graph.cut.as_deref(), Some("M-v1"));
}

fn assert_graph_grammar_refusals() {
    for (tasks, expected) in [
        ("  - key: ''\n    title: Blank key\n", "invalid local key"),
        (
            "  - key: two words\n    title: Prose key\n",
            "invalid local key",
        ),
        (
            "  - key: same\n    title: First\n  - key: same\n    title: Second\n",
            "claimed by tasks 1 and 2",
        ),
        (
            "  - key: first\n    title: First\n    blocked_by: [local:missing]\n",
            "names no blueprint task key",
        ),
        (
            "  - key: first\n    title: First\n    blocked_by: [A-1]\n",
            "must begin with `local:` or `task:`",
        ),
        (
            "  - key: first\n    title: First\n    blocks: [A-1]\n",
            "must begin with `local:` or `task:`",
        ),
        (
            "  - key: first\n    title: First\n    blocked_by: ['local:']\n",
            "`local:` with no visible target",
        ),
        (
            "  - key: first\n    title: First\n    blocked_by: ['task:   ']\n",
            "`task:` with no visible target",
        ),
        (
            "  - key: first\n    title: First\n    id: FIXED\n",
            "must mint its id",
        ),
        (
            "  - key: first\n    title: First\n    cut: M-v1\n",
            "one application cut",
        ),
    ] {
        let parsed = parse(Path::new("invalid.yml"), &document(tasks)).expect("typed blueprint");
        let bound = bind_parameters(&parsed, &BTreeMap::new()).expect("no parameters");
        let error = application_keys(&bound)
            .expect_err("invalid graph")
            .to_string();
        assert!(error.contains(expected), "missing {expected:?}: {error}");
    }
}

/// The graph language is total before minting: invalid keys and references,
/// fixed ids, per-entry cuts and an absent application cut are refusals, and
/// merely asking for any of them cannot create a task file. This proves the
/// refusal half of `blueprint.references_resolve_atomically`.
#[test]
fn blueprint_graph_refusals_are_specific_and_write_nothing() {
    assert_graph_grammar_refusals();

    let dir = repository();
    let before = std::fs::read_dir(dir.path().join("aplan/tasks"))
        .expect("tasks")
        .count();
    for (contents, expected) in [
        (
            "schema: [not closed",
            "is not an aplan/blueprint/v1 blueprint",
        ),
        (
            "schema: aplan/blueprint/v1\nname: feature\ntitle: Feature\n\
             description: Bound feature.\nparameters:\n  - name: feature\n    description: Name.\n\
             tasks:\n  - key: first\n    title: 'Build {{feature}}'\n",
            "required blueprint parameter `feature` has no binding",
        ),
        (
            "schema: aplan/blueprint/v1\nname: feature\ntitle: Feature\n\
             description: Duplicate key.\ntasks:\n  - key: same\n    title: First\n\
             \n  - key: same\n    title: Second\n",
            "claimed by tasks 1 and 2",
        ),
    ] {
        let error = preview_blueprint(&context(dir.path()), &request(contents, "M-v1"))
            .expect_err("adapter refusal")
            .to_string();
        assert!(error.contains(expected), "missing {expected:?}: {error}");
    }
    let error = preview_blueprint(
        &context(dir.path()),
        &request(
            &document("  - key: first\n    title: First\n    effort: 1\n"),
            "M-missing",
        ),
    )
    .expect_err("missing target cut")
    .to_string();
    assert!(error.contains("no cut"), "{error}");
    assert_eq!(
        std::fs::read_dir(dir.path().join("aplan/tasks"))
            .expect("tasks")
            .count(),
        before
    );

    let error = preview_blueprint(
        &context(dir.path()),
        &request(
            &document(
                "  - key: first\n    title: First\n    effort: 1\n    blocked_by: [task:A-missing]\n",
            ),
            "M-v1",
        ),
    )
    .expect_err("unknown existing task")
    .to_string();
    assert!(error.contains("not declared: A-missing"), "{error}");
    assert_eq!(
        std::fs::read_dir(dir.path().join("aplan/tasks"))
            .expect("tasks")
            .count(),
        before
    );
}

/// Proves `blueprint.references_resolve_atomically` and
/// `dep.atomic_batch_rewrites`: a rejected complete
/// post-state publishes no files, while two contending applications re-preview
/// after the guarded writer's stale result and publish two whole, disjoint
/// minted graphs.
#[test]
fn blueprint_application_is_atomic_and_remints_under_contention() {
    let dir = repository();
    let cyclic = document(
        "  - key: first\n    title: First\n    effort: 1\n    blocked_by: [local:second]\n\
         \n  - key: second\n    title: Second\n    effort: 1\n    blocked_by: [local:first]\n",
    );
    let cut_before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-v1.md"))
        .expect("cut before refusal");
    let refused = preview_blueprint(&context(dir.path()), &request(&cyclic, "M-v1"))
        .expect("the complete invalid post-state is still reviewable");
    assert_eq!(
        refused.change_set().disposition(),
        ChangeDisposition::Denied
    );
    assert!(matches!(
        commit_change_set(&context(dir.path()), refused.into_change_set()),
        CommitOutcome::Denied(_)
    ));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-v1.md")).expect("cut after refusal"),
        cut_before
    );
    assert!(!dir.path().join("aplan/tasks/A-2.md").exists());

    let valid = Arc::new(document(
        "  - key: design\n    title: Design\n    effort: 1\n    blocks: [task:A-1]\n\
         \n  - key: implement\n    title: Implement\n    effort: 1\n    blocked_by: [local:design]\n",
    ));
    let root = Arc::new(dir.path().to_path_buf());
    let first_attempt = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let root = Arc::clone(&root);
            let valid = Arc::clone(&valid);
            let first_attempt = Arc::clone(&first_attempt);
            std::thread::spawn(move || {
                for attempt in 0..20 {
                    let snapshot = context(&root);
                    let preview = preview_blueprint(&snapshot, &request(&valid, "M-v1"))
                        .expect("blueprint preview");
                    let ids = preview.created().to_vec();
                    if attempt == 0 {
                        first_attempt.wait();
                    }
                    match commit_change_set(&snapshot, preview.into_change_set()) {
                        CommitOutcome::Applied(_) => return ids,
                        CommitOutcome::Stale { .. } => {}
                        other => panic!("unexpected blueprint publication: {other:?}"),
                    }
                }
                panic!("blueprint contention never settled")
            })
        })
        .collect::<Vec<_>>();
    let batches = handles
        .into_iter()
        .map(|handle| handle.join().expect("writer"))
        .collect::<Vec<_>>();
    assert_eq!(batches[0].len(), 2);
    assert_eq!(batches[1].len(), 2);
    let all = batches.iter().flatten().cloned().collect::<BTreeSet<_>>();
    assert_eq!(all.len(), 4, "concurrent applications reused an id");
    for batch in &batches {
        let second =
            std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{}.md", batch[1])))
                .expect("second task");
        assert!(
            second.contains(&format!("blocked_by:\n- {}", batch[0])),
            "{second}"
        );
    }
    let existing = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md"))
        .expect("existing reverse target");
    for batch in &batches {
        assert!(
            existing.contains(&batch[0]),
            "existing task omitted reverse blocker {}:\n{existing}",
            batch[0]
        );
    }
    let cut = std::fs::read_to_string(dir.path().join("aplan/tasks/M-v1.md"))
        .expect("cut after applications");
    for id in &all {
        assert!(cut.contains(id), "cut omitted {id}:\n{cut}");
    }
}

fn parameterized_blueprint() -> String {
    format!(
        concat!(
            "schema: {schema}\n",
            "name: feature\n",
            "title: Feature graph\n",
            "description: Create one bound graph.\n",
            "parameters:\n",
            "  - name: feature\n",
            "    description: Feature name.\n",
            "tasks:\n",
            "  - key: design\n",
            "    title: 'Design {{{{feature}}}}'\n",
            "    effort: 1\n",
            "    paths: [src/design.rs]\n",
            "  - key: implement\n",
            "    title: 'Implement {{{{feature}}}}'\n",
            "    effort: 2\n",
            "    paths: [src/implement.rs]\n",
            "    blocked_by: [local:design]\n",
        ),
        schema = SCHEMA
    )
}

fn application_args(format: &str, dry_run: bool) -> Vec<&str> {
    let mut args = vec![
        "blueprint",
        "apply",
        "--file",
        "feature.blueprint.yml",
        "--cut",
        "M-v1",
        "--set",
        "feature=bulk close",
        "--format",
        format,
    ];
    if dry_run {
        args.push("--dry-run");
    }
    args
}

fn json_run(dir: &Path, format: &str, dry_run: bool) -> (common::Run, serde_json::Value) {
    let run = common::run(dir, &application_args(format, dry_run));
    let document = serde_json::from_str(&run.stdout)
        .unwrap_or_else(|error| panic!("{format} output was not JSON: {error}: {run:?}"));
    (run, document)
}

/// End-to-end proof for the public adapter: dry-run and apply identify the same
/// once-captured bytes, while only apply publishes the exact reported ids.
#[test]
fn blueprint_cli_dry_run_and_apply_report_one_captured_input() {
    let dir = repository();
    let source = parameterized_blueprint();
    write(&dir.path().join("feature.blueprint.yml"), &source);
    let digest = aplan_attest::sha256_hex(source.as_bytes());

    let (dry, predicted) = json_run(dir.path(), "json", true);
    assert_eq!(dry.code, 0, "{dry:?}");
    assert_eq!(predicted["schema"], "aplan/blueprint-application/v1");
    assert_eq!(predicted["source_path"], "feature.blueprint.yml");
    assert_eq!(predicted["source_sha256"], digest);
    assert_eq!(predicted["outcome"], "predicted");
    assert_eq!(predicted["publishable"], true);
    assert_eq!(predicted["cut"], "M-v1");
    assert_eq!(predicted["tasks"][0]["id"], "A-2");
    assert_eq!(predicted["tasks"][1]["id"], "A-3");
    assert_eq!(predicted["tasks"][0]["title"], "Design bulk close");
    assert_eq!(predicted["attempts"], 1);
    assert_eq!(predicted["plan_sha256"].as_str().map(str::len), Some(64));
    assert!(!dir.path().join("aplan/tasks/A-2.md").exists());
    assert!(predicted["files"].as_array().is_some_and(|files| {
        files
            .iter()
            .any(|file| file["path"] == "tasks/M-v1.md" && file["kind"] == "update")
    }));

    let (applied_run, applied) = json_run(dir.path(), "json", false);
    assert_eq!(applied_run.code, 0, "{applied_run:?}");
    assert_eq!(applied["outcome"], "applied");
    assert_eq!(applied["source_sha256"], digest);
    assert_eq!(applied["plan_sha256"], predicted["plan_sha256"]);
    assert_eq!(applied["tasks"], predicted["tasks"]);
    for id in ["A-2", "A-3"] {
        assert!(dir.path().join(format!("aplan/tasks/{id}.md")).exists());
    }
    let implementation =
        std::fs::read_to_string(dir.path().join("aplan/tasks/A-3.md")).expect("created task");
    assert!(
        implementation.contains("blocked_by:\n- A-2"),
        "{implementation}"
    );
}

/// The action surface is one answer in three encodings. JSON and agent payloads
/// agree field-for-field, while the human rendering names every exact identity.
#[test]
fn blueprint_cli_formats_carry_the_same_application_answer() {
    let dir = repository();
    let source = parameterized_blueprint();
    write(&dir.path().join("feature.blueprint.yml"), &source);
    let digest = aplan_attest::sha256_hex(source.as_bytes());

    let (json, mut machine) = json_run(dir.path(), "json", true);
    let (agent, mut agent_document) = json_run(dir.path(), "agent", true);
    assert_eq!(json.code, 0, "{json:?}");
    assert_eq!(agent.code, 0, "{agent:?}");
    machine
        .as_object_mut()
        .expect("JSON object")
        .remove("schema");
    let agent_object = agent_document.as_object_mut().expect("agent object");
    agent_object.remove("schema");
    let instruction = agent_object
        .remove("agent_instruction")
        .and_then(|value| value.as_str().map(str::to_string))
        .expect("actionable agent instruction");
    assert!(instruction.contains("without --dry-run"), "{instruction}");
    assert_eq!(agent_document, machine);

    let human = common::run(dir.path(), &application_args("human", true));
    assert_eq!(human.code, 0, "{human:?}");
    for exact in [
        "blueprint application: predicted",
        "source: feature.blueprint.yml",
        &format!("sha256: {digest}"),
        "cut: M-v1",
        "predicted A-2",
        "predicted A-3",
        "create tasks/A-2.md",
        "create tasks/A-3.md",
        "update tasks/M-v1.md",
    ] {
        assert!(human.stdout.contains(exact), "missing {exact:?}: {human:?}");
    }
    assert!(!dir.path().join("aplan/tasks/A-2.md").exists());
}

/// Input and gate refusals stay pre-write, including visually plausible CLI
/// mistakes, non-UTF-8 bytes and a complete graph whose post-state is cyclic.
#[test]
fn blueprint_cli_refusals_are_specific_and_write_nothing() {
    let dir = repository();
    write(
        &dir.path().join("feature.blueprint.yml"),
        &parameterized_blueprint(),
    );
    let before = std::fs::read_to_string(dir.path().join("aplan/tasks/M-v1.md"))
        .expect("cut before refusals");

    let duplicate = common::run(
        dir.path(),
        &[
            "blueprint",
            "apply",
            "--file",
            "feature.blueprint.yml",
            "--cut",
            "M-v1",
            "--set",
            "feature=one",
            "--set",
            "feature=two",
        ],
    );
    assert_eq!(duplicate.code, 2, "{duplicate:?}");
    assert!(
        duplicate.stderr.contains("bound more than once"),
        "{duplicate:?}"
    );

    std::fs::write(dir.path().join("not-utf8.yml"), [0xff, 0xfe]).expect("invalid input");
    let invalid = common::run(
        dir.path(),
        &[
            "blueprint",
            "apply",
            "--file",
            "not-utf8.yml",
            "--cut",
            "M-v1",
        ],
    );
    assert_eq!(invalid.code, 2, "{invalid:?}");
    assert!(invalid.stderr.contains("is not UTF-8"), "{invalid:?}");

    let cyclic = document(
        "  - key: first\n    title: First\n    effort: 1\n    blocked_by: [local:second]\n\
         \n  - key: second\n    title: Second\n    effort: 1\n    blocked_by: [local:first]\n",
    );
    write(&dir.path().join("cycle.blueprint.yml"), &cyclic);
    let refused = common::run(
        dir.path(),
        &[
            "blueprint",
            "apply",
            "--file",
            "cycle.blueprint.yml",
            "--cut",
            "M-v1",
            "--format",
            "json",
        ],
    );
    assert_eq!(refused.code, 1, "{refused:?}");
    let document: serde_json::Value = serde_json::from_str(&refused.stdout).expect("refusal JSON");
    assert_eq!(document["outcome"], "refused");
    assert_eq!(document["publishable"], false);
    assert!(document["findings"].as_array().is_some_and(|findings| {
        findings.iter().any(|finding| {
            finding
                .as_str()
                .is_some_and(|line| line.contains("APLAN004"))
        })
    }));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/M-v1.md")).expect("cut after"),
        before
    );
    assert!(!dir.path().join("aplan/tasks/A-2.md").exists());
}

/// Multiple real CLI processes may race on one sequential-id plan. Every
/// success reports and publishes one whole disjoint graph; stale previews are
/// retried from current plan state with the original captured blueprint bytes.
#[test]
fn blueprint_cli_concurrent_applications_publish_disjoint_whole_graphs() {
    let dir = repository();
    let source = document(
        "  - key: design\n    title: Design\n    effort: 1\n\
         \n  - key: implement\n    title: Implement\n    effort: 1\n    blocked_by: [local:design]\n",
    );
    write(&dir.path().join("concurrent.blueprint.yml"), &source);
    let args = [
        "blueprint",
        "apply",
        "--file",
        "concurrent.blueprint.yml",
        "--cut",
        "M-v1",
        "--format",
        "json",
    ];
    let children = (0..4)
        .map(|_| {
            common::command(dir.path(), &args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("start concurrent application")
        })
        .collect::<Vec<_>>();
    let mut batches = Vec::new();
    for child in children {
        let output = child.wait_with_output().expect("concurrent application");
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let document: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("application JSON");
        let ids = document["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .map(|task| task["id"].as_str().expect("id").to_string())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 2);
        batches.push(ids);
    }
    let all = batches.iter().flatten().collect::<BTreeSet<_>>();
    assert_eq!(
        all.len(),
        8,
        "concurrent applications reused ids: {batches:?}"
    );
    for batch in &batches {
        let second =
            std::fs::read_to_string(dir.path().join(format!("aplan/tasks/{}.md", batch[1])))
                .expect("published sibling");
        assert!(
            second.contains(&format!("blocked_by:\n- {}", batch[0])),
            "{second}"
        );
    }
}
