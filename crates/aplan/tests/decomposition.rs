//! End-to-end integration tests for work-task decomposition via `integrates`
//! (`A2-148`, ADR-0076, `APLAN066`, `effort.exempts_decomposed_parents`).

#[cfg(feature = "tui")]
use std::collections::BTreeMap;
use std::path::Path;

use aplan_core::validate;
use aplan_mcp::tools;

use crate::common;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(path, text).expect("write fixture file");
}

fn tempdir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create tempdir")
}

fn task_md(frontmatter: &str) -> String {
    format!(
        "---\n{}\n---\n\n## Why\n\nFixture task.\n",
        frontmatter.trim()
    )
}

fn init_decomposed_effort_plan(dir: &Path) {
    write(
        &dir.join(".aplan.yml"),
        "root: aplan\nrequire_effort: true\nrequire_cut_membership: true\ndeny_warnings: true\n",
    );
    write(
        &dir.join("aplan/tasks/M-s1.md"),
        &task_md(
            "id: M-s1\ntitle: First slice\nphase: 1\nstatus: todo\n\
             tags: [milestone]\n\
             blocked_by: [A1-01, A1-02, A1-03, A1-04, A1-05]",
        ),
    );
    // Parent without own effort_days (exempt from APLAN009 because it integrates valid children).
    write(
        &dir.join("aplan/tasks/A1-01.md"),
        &task_md(
            "id: A1-01\ntitle: Parent without own effort\nphase: 1\nstatus: todo\n\
             paths: [src/parent.rs]\n\
             blocked_by: [A1-02, A1-03]\nintegrates: [A1-02, A1-03]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-02.md"),
        &task_md(
            "id: A1-02\ntitle: First child increment\nphase: 1\nstatus: todo\n\
             effort_days: 2.0\npaths: [src/c1.rs]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-03.md"),
        &task_md(
            "id: A1-03\ntitle: Second child increment\nphase: 1\nstatus: todo\n\
             effort_days: 3.0\npaths: [src/c2.rs]\nblocked_by: [A1-02]",
        ),
    );
    // Parent carrying explicit own integration effort.
    write(
        &dir.join("aplan/tasks/A1-04.md"),
        &task_md(
            "id: A1-04\ntitle: Parent with integration effort\nphase: 1\nstatus: todo\n\
             effort_days: 1.5\npaths: [src/int.rs]\n\
             blocked_by: [A1-05]\nintegrates: [A1-05]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-05.md"),
        &task_md(
            "id: A1-05\ntitle: Child beneath integration parent\nphase: 1\nstatus: todo\n\
             effort_days: 2.5\npaths: [src/int_child.rs]",
        ),
    );
}

fn check_codes(dir: &Path) -> Vec<(String, String)> {
    let out = common::run(dir, &["check", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect("check json");
    doc["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap_or_default().to_string(),
                d["message"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn assert_single_counted_effort_rollups(dir: &Path) {
    let roadmap_out = common::run(dir, &["roadmap", "--format", "json"]);
    assert_eq!(roadmap_out.code, 0, "roadmap: {}", roadmap_out.stderr);
    let roadmap: serde_json::Value = serde_json::from_str(&roadmap_out.stdout).expect("roadmap");
    assert_eq!(roadmap["cuts"][0]["effort_days"], 9.0);
    assert_eq!(roadmap["cuts"][0]["done_effort_days"], 0.0);

    let summary_out = common::run(dir, &["summary", "--format", "json"]);
    assert_eq!(summary_out.code, 0, "summary: {}", summary_out.stderr);
    let summary: serde_json::Value = serde_json::from_str(&summary_out.stdout).expect("summary");
    assert_eq!(summary["total_effort_days"], 9.0);
    let decomps = summary["decompositions"]
        .as_array()
        .expect("summary decompositions");
    assert_eq!(decomps.len(), 2);
    assert_eq!(decomps[0]["id"], "A1-01");
    assert!(decomps[0]["own_effort_days"].is_null());
    assert_eq!(decomps[0]["progress"]["children_effort_days"], 5.0);
    assert_eq!(decomps[0]["progress"]["total_effort_days"], 5.0);
    assert_eq!(decomps[1]["id"], "A1-04");
    assert_eq!(decomps[1]["own_effort_days"], 1.5);
    assert_eq!(decomps[1]["progress"]["children_effort_days"], 2.5);
    assert_eq!(decomps[1]["progress"]["total_effort_days"], 4.0);

    let tasks_out = common::run(dir, &["tasks", "--format", "json"]);
    assert_eq!(tasks_out.code, 0, "tasks: {}", tasks_out.stderr);
    let tasks: serde_json::Value = serde_json::from_str(&tasks_out.stdout).expect("tasks");
    let rows = tasks["tasks"].as_array().expect("rows");
    let parent = rows.iter().find(|r| r["id"] == "A1-01").expect("A1-01");
    assert!(parent["effort_days"].is_null());
    assert_eq!(parent["integrates"], serde_json::json!(["A1-02", "A1-03"]));
    assert_eq!(parent["decomposition"]["children_effort_days"], 5.0);
    let child = rows.iter().find(|r| r["id"] == "A1-02").expect("A1-02");
    assert_eq!(child["parent"], "A1-01");

    let cp_out = common::run(dir, &["critical-path", "--format", "json"]);
    assert_eq!(cp_out.code, 0, "critical-path: {}", cp_out.stderr);
    let cp: serde_json::Value = serde_json::from_str(&cp_out.stdout).expect("cp");
    assert_eq!(cp["total_effort_days"], 5.0);
}

/// Proves `effort.exempts_decomposed_parents`: under `require_effort: true`, a
/// live non-milestone parent with valid non-dropped integrated children may omit
/// `effort_days` (both while children are open and once all children finish at
/// its integration boundary), while leaf tasks and parents with only dropped or
/// missing children still fire `APLAN009`, and cut/roadmap/summary/critical-path
/// totals count every task's own effort exactly once.
#[test]
fn parent_effort_exemption_and_single_counted_cut_rollups() {
    let dir = tempdir("decomp-effort-");
    init_decomposed_effort_plan(dir.path());

    let clean = common::run(dir.path(), &["check"]);
    assert_eq!(clean.code, 0, "expected clean check: {}", clean.stderr);

    assert_single_counted_effort_rollups(dir.path());

    // Close both children beneath A1-01: A1-01 reaches its integration boundary
    // and remains exempt from APLAN009 even though every child is now `done`.
    for (child, proof) in [
        ("A1-02", "url:https://example.invalid/a1-02"),
        ("A1-03", "url:https://example.invalid/a1-03"),
    ] {
        let closed = common::run(dir.path(), &["close", child, "--evidence", proof]);
        assert_eq!(closed.code, 0, "close {child}: {}", closed.stderr);
    }
    let after_children = common::run(dir.path(), &["check"]);
    assert_eq!(
        after_children.code, 0,
        "parent at integration boundary must remain APLAN009-exempt: {}",
        after_children.stderr
    );
    let ready_out = common::run(dir.path(), &["ready", "--format", "json"]);
    assert_eq!(ready_out.code, 0);
    let ready: serde_json::Value = serde_json::from_str(&ready_out.stdout).expect("ready");
    let a1_01 = ready["ready"]
        .as_array()
        .expect("ready array")
        .iter()
        .find(|t| t["id"] == "A1-01")
        .expect("A1-01 is ready at integration boundary");
    assert_eq!(a1_01["decomposition"]["integration_ready"], true);

    // Narrow exemption guard: a leaf task without effort_days still fires APLAN009.
    write(
        &dir.path().join("aplan/tasks/A1-06.md"),
        &task_md("id: A1-06\ntitle: Unestimated leaf\nphase: 1\nstatus: todo\npaths: [src/l.rs]"),
    );
    write(
        &dir.path().join("aplan/tasks/M-s1.md"),
        &task_md(
            "id: M-s1\ntitle: First slice\nphase: 1\nstatus: todo\n\
             tags: [milestone]\n\
             blocked_by: [A1-01, A1-02, A1-03, A1-04, A1-05, A1-06, A1-07, A1-08]",
        ),
    );
    // Narrow exemption guard: a parent whose only integrated child is `dropped`
    // does NOT qualify for the APLAN009 exemption.
    write(
        &dir.path().join("aplan/tasks/A1-07.md"),
        &task_md(
            "id: A1-07\ntitle: Parent of dropped child\nphase: 1\nstatus: todo\n\
             paths: [src/p7.rs]\nblocked_by: [A1-08]\nintegrates: [A1-08]",
        ),
    );
    write(
        &dir.path().join("aplan/tasks/A1-08.md"),
        &task_md(
            "id: A1-08\ntitle: Dropped child\nphase: 1\nstatus: dropped\n\
             effort_days: 1.0\npaths: [src/c8.rs]",
        ),
    );
    let check_out = common::run(dir.path(), &["check", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&check_out.stdout).expect("check json");
    let a009_tasks: Vec<&str> = doc["diagnostics"]
        .as_array()
        .expect("diags")
        .iter()
        .filter(|d| d["code"] == "APLAN009")
        .filter_map(|d| d["task"].as_str())
        .collect();
    assert!(a009_tasks.contains(&"A1-06"), "{a009_tasks:?}");
    assert!(a009_tasks.contains(&"A1-07"), "{a009_tasks:?}");
    assert!(!a009_tasks.contains(&"A1-01"), "{a009_tasks:?}");
}

fn init_nested_plan(dir: &Path) {
    write(
        &dir.join(".aplan.yml"),
        "root: aplan\nrequire_effort: true\nrequire_cut_membership: true\ndeny_warnings: true\n",
    );
    write(
        &dir.join("aplan/tasks/M-v1.md"),
        &task_md(
            "id: M-v1\ntitle: Core cut\nphase: 1\nstatus: todo\ntags: [milestone]\n\
             blocked_by: [A1-01, A1-10, A1-11, A1-12, A1-13, A1-14]",
        ),
    );
    // Flat standalone task alongside a nested decomposition tree.
    write(
        &dir.join("aplan/tasks/A1-01.md"),
        &task_md(
            "id: A1-01\ntitle: Standalone flat task\nphase: 1\nstatus: todo\n\
             effort_days: 1.0\npaths: [src/flat.rs]",
        ),
    );
    // Nested tree: A1-10 -> [A1-11, A1-14], A1-11 -> [A1-12 (done), A1-13 (todo)].
    write(
        &dir.join("aplan/tasks/A1-10.md"),
        &task_md(
            "id: A1-10\ntitle: Deliver search subsystem\nphase: 1\nstatus: todo\n\
             effort_days: 1.0\npaths: [src/search/mod.rs]\n\
             blocked_by: [A1-11, A1-14]\nintegrates: [A1-11, A1-14]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-11.md"),
        &task_md(
            "id: A1-11\ntitle: Indexing pipeline\nphase: 1\nstatus: todo\n\
             paths: [src/search/index.rs]\n\
             blocked_by: [A1-12, A1-13]\nintegrates: [A1-12, A1-13]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-12.md"),
        &task_md(
            "id: A1-12\ntitle: Tokenizer\nphase: 1\nstatus: done\n\
             effort_days: 1.5\npaths: [src/search/tok.rs]\n\
             evidence:\n  - kind: note\n    ref: tokenizer reviewed\n    captured: 2026-11-01",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-13.md"),
        &task_md(
            "id: A1-13\ntitle: Inverted index\nphase: 1\nstatus: todo\n\
             effort_days: 2.0\npaths: [src/search/inv.rs]\nblocked_by: [A1-12]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-14.md"),
        &task_md(
            "id: A1-14\ntitle: Query planner\nphase: 1\nstatus: todo\n\
             effort_days: 2.5\npaths: [src/search/query.rs]\nblocked_by: [A1-11]",
        ),
    );
}

fn assert_show_why_tree_projections(dir: &Path) {
    let show_json = common::run(dir, &["show", "A1-10", "--format", "json", "--no-resolve"]);
    assert_eq!(show_json.code, 0, "show json: {}", show_json.stderr);
    let show: serde_json::Value = serde_json::from_str(&show_json.stdout).expect("show json");
    assert!(show["parent"].is_null());
    assert_eq!(show["integrates"][0]["id"], "A1-11");
    assert_eq!(show["integrates"][1]["id"], "A1-14");
    assert_eq!(show["decomposition"]["children_done"], 0);
    assert_eq!(show["decomposition"]["children_total"], 2);
    assert_eq!(
        show["decomposition"]["remaining_children"],
        serde_json::json!(["A1-11", "A1-14"])
    );
    assert_eq!(show["decomposition"]["descendants_done"], 1);
    assert_eq!(show["decomposition"]["descendants_total"], 4);
    assert_eq!(show["decomposition"]["descendants_remaining"], 3);
    assert_eq!(show["decomposition"]["current_increment"], "A1-13");
    assert_eq!(show["decomposition"]["integration_ready"], false);
    assert_eq!(show["decomposition"]["children_effort_days"], 6.0);
    assert_eq!(show["decomposition"]["children_done_effort_days"], 1.5);
    assert_eq!(show["decomposition"]["total_effort_days"], 7.0);

    let show_human = common::run(dir, &["show", "A1-10", "--no-resolve"]);
    assert_eq!(show_human.code, 0);
    assert!(
        show_human.stdout.contains("decomposition:"),
        "{}",
        show_human.stdout
    );
    assert!(
        show_human.stdout.contains("current increment: A1-13"),
        "{}",
        show_human.stdout
    );
    assert!(
        show_human.stdout.contains("integrates:"),
        "{}",
        show_human.stdout
    );

    let show_child = common::run(dir, &["show", "A1-13", "--format", "json", "--no-resolve"]);
    let child: serde_json::Value = serde_json::from_str(&show_child.stdout).expect("child json");
    assert_eq!(child["parent"]["id"], "A1-11");
    assert!(child["decomposition"].is_null());

    let why_json = common::run(dir, &["why", "A1-10", "--format", "json"]);
    assert_eq!(why_json.code, 0);
    let why: serde_json::Value = serde_json::from_str(&why_json.stdout).expect("why json");
    assert_eq!(why["decomposition"]["current_increment"], "A1-13");
    assert!(
        why["blocked_by"]
            .as_array()
            .expect("blockers")
            .iter()
            .any(|b| b["id"] == "A1-11" && b["integrated_child"] == true)
    );

    let why_human = common::run(dir, &["why", "A1-10"]);
    assert!(
        why_human.stdout.contains("decomposition:"),
        "{}",
        why_human.stdout
    );
    assert!(
        why_human.stdout.contains("(integrated child)"),
        "{}",
        why_human.stdout
    );

    let why_leaf = common::run(dir, &["why", "A1-13"]);
    assert!(
        why_leaf.stdout.contains("part of: A1-11"),
        "{}",
        why_leaf.stdout
    );

    let tree_parent = common::run(dir, &["tree", "A1-10"]);
    assert_eq!(tree_parent.code, 0);
    assert!(
        tree_parent.stdout.contains("decomposition:"),
        "{}",
        tree_parent.stdout
    );
    assert!(
        tree_parent.stdout.contains("(integrated child)"),
        "{}",
        tree_parent.stdout
    );

    let tree_child = common::run(dir, &["tree", "A1-13"]);
    assert_eq!(tree_child.code, 0);
    assert!(
        tree_child.stdout.contains("part of: A1-11"),
        "{}",
        tree_child.stdout
    );
}

fn assert_graph_roadmap_summary_tasks_and_tui(dir: &Path) {
    let mermaid = common::run(dir, &["graph", "--format", "mermaid"]);
    assert_eq!(mermaid.code, 0);
    assert!(
        mermaid.stdout.contains("==>|integrates|"),
        "{}",
        mermaid.stdout
    );

    let dot = common::run(dir, &["graph", "--format", "dot"]);
    assert_eq!(dot.code, 0);
    assert!(
        dot.stdout.contains("label=\"integrates\""),
        "{}",
        dot.stdout
    );

    let graph_json = common::run(dir, &["graph", "--format", "json"]);
    assert_eq!(graph_json.code, 0);
    let graph: serde_json::Value = serde_json::from_str(&graph_json.stdout).expect("graph json");
    assert!(
        graph["edges"]
            .as_array()
            .expect("edges")
            .iter()
            .any(|e| e["from"] == "A1-11" && e["to"] == "A1-10" && e["integrates"] == true)
    );
    let parent_node = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == "A1-10")
        .expect("A1-10 node");
    assert_eq!(parent_node["decomposition"]["current_increment"], "A1-13");

    let roadmap_human = common::run(dir, &["roadmap"]);
    assert_eq!(roadmap_human.code, 0);
    assert!(
        roadmap_human
            .stdout
            .contains("[integrates 0/2; current A1-13]"),
        "{}",
        roadmap_human.stdout
    );
    assert!(
        roadmap_human.stdout.contains("part of A1-11"),
        "{}",
        roadmap_human.stdout
    );

    let roadmap_md = common::run(dir, &["roadmap", "--format", "markdown"]);
    assert_eq!(roadmap_md.code, 0);
    assert!(
        roadmap_md
            .stdout
            .contains("[integrates 0/2; current A1-13]"),
        "{}",
        roadmap_md.stdout
    );

    let summary_human = common::run(dir, &["summary"]);
    assert_eq!(summary_human.code, 0);
    assert!(
        summary_human.stdout.contains("decompositions:"),
        "{}",
        summary_human.stdout
    );
    assert!(
        summary_human.stdout.contains("current A1-13"),
        "{}",
        summary_human.stdout
    );

    let tasks_human = common::run(dir, &["tasks"]);
    assert_eq!(tasks_human.code, 0);
    assert!(
        tasks_human.stdout.contains("A1-10"),
        "{}",
        tasks_human.stdout
    );

    #[cfg(feature = "tui")]
    assert_tui_authoring_and_inspector_parity(dir);
}

#[cfg(feature = "tui")]
fn assert_tui_authoring_and_inspector_parity(dir: &Path) {
    let context = aplan_app::PlanContext::new(
        aplan_core::load_plan_identified(&dir.join("aplan")),
        aplan_app::ContextInputs {
            repository_root: dir.to_path_buf(),
            configuration_file: Some(dir.join(".aplan.yml")),
            trust: aplan_app::Trust::Hermetic,
            options: validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
        },
    );
    let form =
        aplan_tui::authoring::form(&context, aplan_tui::authoring::AuthoringWorkflow::Add, None)
            .expect("tui add form");
    assert!(
        form.fields()
            .iter()
            .any(|field| field.spec().key() == "integrates"),
        "TUI add form must expose `integrates`"
    );
    let plan = aplan_core::load_plan(&dir.join("aplan"));
    let g = aplan_core::Graph::new(plan.tasks.iter().map(|lt| &lt.task));
    let claims = aplan_core::ClaimSnapshot::unavailable(0, "test");
    let show = aplan_core::Show::assemble_for_task(
        &g,
        &[],
        &BTreeMap::new(),
        &claims,
        "A1-10",
        false,
        None,
    )
    .expect("assemble show");
    let progress = show.decomposition.as_ref().expect("decomposition progress");
    assert_eq!(progress.current_increment.as_deref(), Some("A1-13"));
    assert_eq!(progress.remaining_children, vec!["A1-11", "A1-14"]);
    assert!(!progress.integration_ready);
}

#[test]
fn flat_one_level_nested_and_partial_completion_across_all_surfaces() {
    let dir = tempdir("decomp-surfaces-");
    init_nested_plan(dir.path());

    let check = common::run(dir.path(), &["check"]);
    assert_eq!(check.code, 0, "nested plan must be valid: {}", check.stderr);

    assert_show_why_tree_projections(dir.path());
    assert_graph_roadmap_summary_tasks_and_tui(dir.path());

    // Progress through partial completion to the integration boundary.
    let close_13 = common::run(
        dir.path(),
        &[
            "close",
            "A1-13",
            "--evidence",
            "url:https://example.invalid/a1-13",
        ],
    );
    assert_eq!(close_13.code, 0, "close A1-13: {}", close_13.stderr);

    let ready_mid = common::run(dir.path(), &["ready"]);
    assert_eq!(ready_mid.code, 0);
    assert!(
        ready_mid.stdout.contains("A1-11")
            && ready_mid
                .stdout
                .contains("integration boundary: 2/2 children done"),
        "{}",
        ready_mid.stdout
    );

    let close_11 = common::run(
        dir.path(),
        &[
            "close",
            "A1-11",
            "--evidence",
            "url:https://example.invalid/a1-11",
        ],
    );
    assert_eq!(close_11.code, 0, "close A1-11: {}", close_11.stderr);
    let close_14 = common::run(
        dir.path(),
        &[
            "close",
            "A1-14",
            "--evidence",
            "url:https://example.invalid/a1-14",
        ],
    );
    assert_eq!(close_14.code, 0, "close A1-14: {}", close_14.stderr);

    let ready_top = common::run(dir.path(), &["ready", "--format", "json"]);
    assert_eq!(ready_top.code, 0);
    let top: serde_json::Value = serde_json::from_str(&ready_top.stdout).expect("ready top");
    let a1_10 = top["ready"]
        .as_array()
        .expect("ready")
        .iter()
        .find(|t| t["id"] == "A1-10")
        .expect("A1-10 ready at boundary");
    assert_eq!(a1_10["decomposition"]["integration_ready"], true);
    assert_eq!(a1_10["decomposition"]["children_done"], 2);
    assert_eq!(a1_10["decomposition"]["children_total"], 2);
}

fn assert_a066_case(name: &str, files: &[(&str, &str)], must_mention: &[&str]) {
    let dir = tempdir(&format!("decomp-refuse-{name}-"));
    write(&dir.path().join(".aplan.yml"), "root: aplan\n");
    for (rel, body) in files {
        write(&dir.path().join("aplan/tasks").join(rel), &task_md(body));
    }
    let findings = check_codes(dir.path());
    let a066: Vec<&str> = findings
        .iter()
        .filter(|(c, _)| c == "APLAN066")
        .map(|(_, m)| m.as_str())
        .collect();
    assert!(
        !a066.is_empty(),
        "{name}: expected APLAN066, got {findings:?}"
    );
    let joined = a066.join("\n");
    for needle in must_mention {
        assert!(
            joined.contains(needle),
            "{name}: APLAN066 must name `{needle}`, got:\n{joined}"
        );
    }
}

fn assert_a066_endpoint_and_topology_refusals() {
    // 1. Milestone parent
    assert_a066_case(
        "milestone-parent",
        &[
            (
                "M-1.md",
                "id: M-1\ntitle: M-1\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [C-1]\nintegrates: [C-1]",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: todo\neffort_days: 1",
            ),
        ],
        &["milestone"],
    );
    // 2. Self-edge
    assert_a066_case(
        "self-edge",
        &[(
            "P-1.md",
            "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nblocked_by: [P-1]\nintegrates: [P-1]",
        )],
        &["P-1", "itself"],
    );
    // 3. Duplicate child
    assert_a066_case(
        "duplicate",
        &[
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nblocked_by: [C-1]\nintegrates: [C-1, C-1]",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: todo\neffort_days: 1",
            ),
        ],
        &["C-1", "repeats"],
    );
    // 4. Missing child
    assert_a066_case(
        "missing",
        &[(
            "P-1.md",
            "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nblocked_by: [GHOST]\nintegrates: [GHOST]",
        )],
        &["GHOST"],
    );
    // 5. Milestone child
    assert_a066_case(
        "milestone-child",
        &[
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nblocked_by: [M-1]\nintegrates: [M-1]",
            ),
            (
                "M-1.md",
                "id: M-1\ntitle: M-1\nphase: 1\nstatus: todo\ntags: [milestone]",
            ),
        ],
        &["M-1", "milestone"],
    );
}

fn assert_a066_topology_refusals() {
    // 6. Missing blocked_by pairing
    assert_a066_case(
        "missing-blocked-by",
        &[
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nintegrates: [C-1]",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: todo\neffort_days: 1",
            ),
        ],
        &["C-1", "blocked_by"],
    );
    // 7. Multi-parent conflict
    assert_a066_case(
        "multi-parent",
        &[
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nblocked_by: [C-1]\nintegrates: [C-1]",
            ),
            (
                "P-2.md",
                "id: P-2\ntitle: P-2\nphase: 1\nstatus: todo\nblocked_by: [C-1]\nintegrates: [C-1]",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: todo\neffort_days: 1",
            ),
        ],
        &["P-1", "P-2"],
    );
    // 8. Cycle (also verified alongside APLAN004)
    assert_a066_case(
        "cycle",
        &[
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nblocked_by: [P-2]\nintegrates: [P-2]",
            ),
            (
                "P-2.md",
                "id: P-2\ntitle: P-2\nphase: 1\nstatus: todo\nblocked_by: [P-1]\nintegrates: [P-1]",
            ),
        ],
        &["P-1", "P-2", "cycle"],
    );
}

fn assert_a066_cut_and_terminal_refusals() {
    // 9a. Cross-cut mismatch (different cuts)
    assert_a066_case(
        "cross-cut",
        &[
            (
                "M-1.md",
                "id: M-1\ntitle: M-1\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [P-1]",
            ),
            (
                "M-2.md",
                "id: M-2\ntitle: M-2\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [C-1]",
            ),
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nblocked_by: [C-1]\nintegrates: [C-1]",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: todo\neffort_days: 1",
            ),
        ],
        &["P-1", "C-1", "M-1", "M-2"],
    );
    // 9b. Cross-cut mismatch (cut vs cross-cutting / no cut)
    assert_a066_case(
        "cut-vs-no-cut",
        &[
            (
                "M-1.md",
                "id: M-1\ntitle: M-1\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [P-1]",
            ),
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: todo\nblocked_by: [C-1]\nintegrates: [C-1]",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: todo\neffort_days: 1\ntags: [cross-cutting]",
            ),
        ],
        &["P-1", "C-1", "M-1", "no cut"],
    );
    // 10. Terminal done parent with open child and with dropped child
    assert_a066_case(
        "done-with-open",
        &[
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: done\nblocked_by: [C-1]\nintegrates: [C-1]\nevidence:\n  - kind: note\n    ref: ok\n    captured: 2026-11-01",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: todo\neffort_days: 1",
            ),
        ],
        &["P-1", "C-1", "todo"],
    );
    assert_a066_case(
        "done-with-dropped",
        &[
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: done\nblocked_by: [C-1]\nintegrates: [C-1]\nevidence:\n  - kind: note\n    ref: ok\n    captured: 2026-11-01",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: dropped\neffort_days: 1",
            ),
        ],
        &["P-1", "C-1", "dropped"],
    );
    // 11. Terminal dropped parent with live child
    assert_a066_case(
        "dropped-with-live",
        &[
            (
                "P-1.md",
                "id: P-1\ntitle: P-1\nphase: 1\nstatus: dropped\nblocked_by: [C-1]\nintegrates: [C-1]",
            ),
            (
                "C-1.md",
                "id: C-1\ntitle: C-1\nphase: 1\nstatus: in_progress\neffort_days: 1",
            ),
        ],
        &["P-1", "C-1", "in_progress"],
    );
}

#[test]
fn all_aplan066_refusal_boundaries_fail_closed_and_name_offending_subjects() {
    assert_a066_endpoint_and_topology_refusals();
    assert_a066_topology_refusals();
    assert_a066_cut_and_terminal_refusals();

    // Valid terminal records: done parent with done child, and dropped parent with terminal children.
    let valid_term = tempdir("decomp-valid-term-");
    write(&valid_term.path().join(".aplan.yml"), "root: aplan\n");
    write(
        &valid_term.path().join("aplan/tasks/P-1.md"),
        &task_md(
            "id: P-1\ntitle: Done parent\nphase: 1\nstatus: done\n\
             blocked_by: [C-1]\nintegrates: [C-1]\n\
             evidence:\n  - kind: note\n    ref: ok\n    captured: 2026-11-01",
        ),
    );
    write(
        &valid_term.path().join("aplan/tasks/C-1.md"),
        &task_md(
            "id: C-1\ntitle: Done child\nphase: 1\nstatus: done\n\
             evidence:\n  - kind: note\n    ref: ok\n    captured: 2026-11-01",
        ),
    );
    write(
        &valid_term.path().join("aplan/tasks/P-2.md"),
        &task_md(
            "id: P-2\ntitle: Dropped parent\nphase: 1\nstatus: dropped\n\
             blocked_by: [C-2]\nintegrates: [C-2]",
        ),
    );
    write(
        &valid_term.path().join("aplan/tasks/C-2.md"),
        &task_md("id: C-2\ntitle: Dropped child\nphase: 1\nstatus: dropped"),
    );
    assert!(check_codes(valid_term.path()).is_empty());
}

fn init_lifecycle_plan(dir: &Path) {
    write(
        &dir.join(".aplan.yml"),
        "root: aplan\nrequire_cut_membership: true\ndeny_warnings: true\nadvisory: [APLAN043]\n",
    );
    write(
        &dir.join("aplan/tasks/M-1.md"),
        &task_md(
            "id: M-1\ntitle: Cut 1\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [A1-00, A1-01]",
        ),
    );
    write(
        &dir.join("aplan/tasks/M-2.md"),
        &task_md(
            "id: M-2\ntitle: Cut 2\nphase: 1\nstatus: todo\ntags: [milestone]\nblocked_by: [A1-99]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-00.md"),
        &task_md(
            "id: A1-00\ntitle: Cut 1 anchor\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/anchor.rs]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-01.md"),
        &task_md(
            "id: A1-01\ntitle: First child\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/a.rs]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-99.md"),
        &task_md(
            "id: A1-99\ntitle: Cut 2 anchor\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/z.rs]",
        ),
    );
}

fn mcp_policy(dir: &Path) -> aplan_mcp::Policy {
    aplan_mcp::Policy {
        id_strategy: aplan_core::IdStrategy::Sequential,
        options: validate::Options {
            require_cut_membership: true,
            ..validate::Options::default()
        },
        verdict: aplan_core::Verdict {
            deny_warnings: true,
            advisory: Vec::new(),
        },
        verify: aplan_evidence::VerifyOptions {
            detect_drift: false,
            version: None,
            traces_root: None,
            config_file: None,
            execute: false,
            test_runner: None,
            max_unrelated_files: None,
            observation_inputs: None,
            url_check: None,
        },
        repo_root: dir.to_path_buf(),
    }
}

fn assert_cli_mcp_parity_on_decomposed_plan(dir: &Path) {
    let policy = mcp_policy(dir);

    // show parity on parent and child
    for id in ["A1-02", "A1-01"] {
        let cli_show = common::run(dir, &["show", id, "--format", "json", "--no-resolve"]);
        assert_eq!(cli_show.code, 0, "cli show {id}: {}", cli_show.stderr);
        let mut cli_v: serde_json::Value =
            serde_json::from_str(&cli_show.stdout).expect("cli show");
        let mut mcp_v = tools::show(&dir.join("aplan"), id, &policy).expect("mcp show");
        for v in [&mut cli_v, &mut mcp_v] {
            if let Some(o) = v.as_object_mut() {
                o.remove("resolved");
                o.remove("claim_observed_at_epoch_seconds");
            }
        }
        assert_eq!(cli_v, mcp_v, "show parity diverged for {id}");
    }

    // why parity on parent
    let cli_why = common::run(dir, &["why", "A1-02", "--format", "json"]);
    assert_eq!(cli_why.code, 0);
    let cli_w: serde_json::Value = serde_json::from_str(&cli_why.stdout).expect("cli why");
    let mcp_w =
        serde_json::to_value(tools::why(&dir.join("aplan"), "A1-02", false).expect("mcp why"))
            .expect("mcp why json");
    assert_eq!(cli_w["decomposition"], mcp_w["decomposition"]);
    assert_eq!(cli_w["blocked_by"], mcp_w["blocked_by"]);

    // ready parity
    let cli_ready = common::run(dir, &["ready", "--format", "json"]);
    assert_eq!(cli_ready.code, 0);
    let cli_r: serde_json::Value = serde_json::from_str(&cli_ready.stdout).expect("cli ready");
    let mcp_r = serde_json::to_value(tools::ready(&dir.join("aplan")).expect("mcp ready"))
        .expect("mcp ready json");
    assert_eq!(cli_r["ready"], mcp_r["ready"]);
}

fn assert_add_cli_mcp_parity() {
    let cli_dir = tempdir("decomp-add-cli-");
    let mcp_dir = tempdir("decomp-add-mcp-");
    init_lifecycle_plan(cli_dir.path());
    init_lifecycle_plan(mcp_dir.path());

    let added = common::run(
        cli_dir.path(),
        &[
            "add",
            "Integrated parent",
            "--id",
            "A1-02",
            "--cut",
            "M-1",
            "--blocked-by",
            "A1-01",
            "--integrates",
            "A1-01",
            "--paths",
            "src/p.rs",
        ],
    );
    assert_eq!(added.code, 0, "cli add --integrates: {}", added.stderr);

    let policy = mcp_policy(mcp_dir.path());
    tools::add(
        &mcp_dir.path().join("aplan"),
        &tools::AddArgs {
            title: "Integrated parent".to_string(),
            id: Some("A1-02".to_string()),
            cut: Some("M-1".to_string()),
            blocked_by: vec!["A1-01".to_string()],
            integrates: vec!["A1-01".to_string()],
            paths: vec!["src/p.rs".to_string()],
            ..tools::AddArgs::default()
        },
        &policy,
    )
    .expect("mcp add with integrates");

    let cli_file =
        std::fs::read_to_string(cli_dir.path().join("aplan/tasks/A1-02.md")).expect("cli");
    let mcp_file =
        std::fs::read_to_string(mcp_dir.path().join("aplan/tasks/A1-02.md")).expect("mcp");
    assert_eq!(
        cli_file, mcp_file,
        "CLI and MCP `add` must write byte-identical task files"
    );

    assert_cli_mcp_parity_on_decomposed_plan(cli_dir.path());
}

fn assert_blueprint_and_manifest_support(dir: &Path) {
    // 1. `add --from` YAML manifest carrying `integrates`
    let manifest_path = dir.join("batch.yml");
    write(
        &manifest_path,
        "- id: A1-20\n  title: Manifest child\n  cut: M-1\n  effort: 1\n  paths: [src/mc.rs]\n  body: Real manifest child body.\n\
         - id: A1-21\n  title: Manifest parent\n  cut: M-1\n  blocked_by: [A1-20]\n  \
           integrates: [A1-20]\n  paths: [src/mp.rs]\n  body: Real manifest parent body.\n",
    );
    let from_out = common::run(
        dir,
        &["add", "--from", manifest_path.to_str().expect("utf8")],
    );
    assert_eq!(from_out.code, 0, "add --from: {}", from_out.stderr);

    // 2. `blueprint apply` resolving `local:` keys inside `integrates` and `blocked_by`
    let bp_path = dir.join("decomp.blueprint.yml");
    write(
        &bp_path,
        concat!(
            "schema: aplan/blueprint/v1\n",
            "name: decomp\n",
            "title: Decomposed feature\n",
            "description: Two increments integrated by one parent.\n",
            "tasks:\n",
            "  - key: step1\n",
            "    title: Step one\n",
            "    effort: 1\n",
            "    paths: [src/bp1.rs]\n",
            "    body: Step one implementation.\n",
            "  - key: parent\n",
            "    title: Integrated feature\n",
            "    blocked_by: [local:step1]\n",
            "    integrates: [local:step1]\n",
            "    paths: [src/bp_parent.rs]\n",
            "    body: Integrated feature boundary.\n",
        ),
    );
    let bp_out = common::run(
        dir,
        &[
            "blueprint",
            "apply",
            "--file",
            bp_path.to_str().expect("utf8"),
            "--cut",
            "M-1",
        ],
    );
    assert_eq!(bp_out.code, 0, "blueprint apply: {}", bp_out.stderr);
    let check = common::run(dir, &["check"]);
    assert_eq!(
        check.code, 0,
        "check after manifest+blueprint: stdout={} stderr={}",
        check.stdout, check.stderr
    );
}

#[test]
fn mutations_interop_and_cli_mcp_parity_for_decomposition() {
    assert_add_cli_mcp_parity();

    let dir = tempdir("decomp-lifecycle-");
    init_lifecycle_plan(dir.path());

    // Refuse invalid add --integrates (without matching --after).
    let bad_add = common::run(
        dir.path(),
        &[
            "add",
            "Bad parent",
            "--id",
            "A1-02",
            "--cut",
            "M-1",
            "--integrates",
            "A1-01",
            "--paths",
            "src/p.rs",
        ],
    );
    assert_ne!(
        bad_add.code, 0,
        "add --integrates without --after must fail"
    );

    // Valid add --integrates paired with --after.
    let ok_add = common::run(
        dir.path(),
        &[
            "add",
            "Good parent",
            "--id",
            "A1-02",
            "--cut",
            "M-1",
            "--blocked-by",
            "A1-01",
            "--integrates",
            "A1-01",
            "--paths",
            "src/p.rs",
        ],
    );
    assert_eq!(ok_add.code, 0, "add --integrates: {}", ok_add.stderr);

    // dep rm that would orphan `integrates` fails closed with APLAN066.
    let bad_dep_rm = common::run(dir.path(), &["dep", "rm", "A1-02", "--after", "A1-01"]);
    assert_ne!(bad_dep_rm.code, 0, "dep rm must not orphan integrates");
    assert!(
        bad_dep_rm.stderr.contains("APLAN066") || bad_dep_rm.stdout.contains("APLAN066"),
        "stderr={} stdout={}",
        bad_dep_rm.stderr,
        bad_dep_rm.stdout
    );

    // mv --cut of child alone fails closed with APLAN066; moving subtree together succeeds.
    let bad_mv = common::run(dir.path(), &["mv", "A1-01", "--cut", "M-2"]);
    assert_ne!(
        bad_mv.code, 0,
        "splitting parent/child across cuts must fail"
    );
    let ok_mv = common::run(dir.path(), &["mv", "A1-01", "A1-02", "--cut", "M-2"]);
    assert_eq!(
        ok_mv.code, 0,
        "moving whole subtree together: {}",
        ok_mv.stderr
    );

    // rekey rewrites `integrates` atomically on child and parent.
    let rekey_child = common::run(dir.path(), &["rekey", "A1-01", "A1-11"]);
    assert_eq!(rekey_child.code, 0, "rekey child: {}", rekey_child.stderr);
    let parent_text =
        std::fs::read_to_string(dir.path().join("aplan/tasks/A1-02.md")).expect("parent md");
    assert!(
        parent_text.contains("A1-11") && !parent_text.contains("A1-01"),
        "{parent_text}"
    );
    let rekey_parent = common::run(dir.path(), &["rekey", "A1-02", "A1-12"]);
    assert_eq!(
        rekey_parent.code, 0,
        "rekey parent: {}",
        rekey_parent.stderr
    );
    let check_rekey = common::run(dir.path(), &["check"]);
    assert_eq!(
        check_rekey.code, 0,
        "check after rekey: stdout={} stderr={}",
        check_rekey.stdout, check_rekey.stderr
    );

    // export --format checklist reports that `integrates` is not carried.
    assert_export_drop_and_supersede_lifecycle(dir.path());
}

fn assert_export_drop_and_supersede_lifecycle(dir: &Path) {
    let export_out = common::run(dir, &["export", "--format", "checklist"]);
    assert_eq!(
        export_out.code, 0,
        "export checklist: {}",
        export_out.stderr
    );
    assert!(
        export_out.stderr.contains("integrates"),
        "expected checklist export report to note `integrates`: {}",
        export_out.stderr
    );

    // drop parent alone fails closed; drop parent+child together succeeds.
    let bad_drop = common::run(dir, &["drop", "A1-12"]);
    assert_ne!(
        bad_drop.code, 0,
        "dropping parent alone while child is live must fail"
    );
    let ok_drop = common::run(dir, &["drop", "A1-12", "A1-11"]);
    assert_eq!(
        ok_drop.code, 0,
        "dropping subtree together: {}",
        ok_drop.stderr
    );
    let check_drop = common::run(dir, &["check"]);
    assert_eq!(
        check_drop.code, 0,
        "check after drop: stdout={} stderr={}",
        check_drop.stdout, check_drop.stderr
    );

    // Manifest (`add --from`) + blueprint apply support.
    assert_blueprint_and_manifest_support(dir);

    // Supersede works cleanly on a closed parent and child.
    let c20 = common::run(
        dir,
        &[
            "close",
            "A1-20",
            "--evidence",
            "url:https://example.invalid/a1-20",
        ],
    );
    assert_eq!(c20.code, 0, "close A1-20: {}", c20.stderr);
    let c21 = common::run(
        dir,
        &[
            "close",
            "A1-21",
            "--evidence",
            "url:https://example.invalid/a1-21",
        ],
    );
    assert_eq!(c21.code, 0, "close A1-21: {}", c21.stderr);
    let sup = common::run(
        dir,
        &[
            "supersede",
            "A1-21",
            "Follow-up integration",
            "--cut",
            "M-1",
            "--effort",
            "1",
        ],
    );
    assert_eq!(sup.code, 0, "supersede closed parent: {}", sup.stderr);
    let check_sup = common::run(dir, &["check"]);
    assert_eq!(
        check_sup.code, 0,
        "check after supersede: stdout={} stderr={}",
        check_sup.stdout, check_sup.stderr
    );
}
