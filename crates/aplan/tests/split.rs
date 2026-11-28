//! End-to-end integration tests for atomic work-task splitting (`aplan split`,
//! `aplan_split`, and TUI `MutationAction::Split`; `A2-150`).

use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use crate::common;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(path, text).expect("write fixture");
}

fn tempdir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

fn setup_oversized_plan(dir: &Path) {
    write(
        &dir.join(".aplan.yml"),
        "root: aplan\nrequire_effort: true\nmax_effort_days: 3.0\n\
         require_cut_membership: true\ndeny_warnings: true\n\
         tag_families:\n  area:\n    members: [core, cli]\n    require: exactly-one\n",
    );
    write(
        &dir.join("DESIGN.md"),
        "# Design\n\n## Atomic Split\n\nKeep increments reviewable.\n",
    );
    write(
        &dir.join("aplan/tasks/M-1.md"),
        "---\n# cut comment\nid: M-1\ntitle: Core cut\nphase: 1\nstatus: todo\n\
         tags: [milestone, v1, core]\nblocked_by: [A1-01, A1-02]\n---\n\n## Why\n\nCut.\n",
    );
    write(
        &dir.join("aplan/tasks/A1-01.md"),
        "---\n# keep parent header comment\nid: A1-01\ntitle: Oversized platform work\n\
         phase: 1\nstatus: in_progress\nowner: ada\neffort_days: 8.0 # legacy estimate\n\
         paths: [src/platform/**]\ntraces: [DESIGN.md#atomic-split]\n\
         tags: [core, adoption]\n---\n\n## Why\n\nLarge in-progress task.\n\n\
         ## Acceptance criteria\n\n- [x] spike notes kept verbatim on parent\n- [ ] integrated\n",
    );
    write(
        &dir.join("aplan/tasks/A1-02.md"),
        "---\nid: A1-02\ntitle: Downstream consumer\nphase: 1\nstatus: todo\n\
         effort_days: 1.5\nblocked_by: [A1-01]\npaths: [src/downstream.rs]\n\
         tags: [cli]\n---\n\n## Why\n\nWaits on A1-01 integration.\n",
    );
}

#[test]
fn real_oversized_in_progress_task_dry_run_apply_yaml_preservation_and_graph_parity() {
    let dir = tempdir("split-e2e-");
    setup_oversized_plan(dir.path());

    let spec_path = dir.path().join("split.yml");
    write(
        &spec_path,
        concat!(
            "target: A1-01\n",
            "parent_effort: 0.5\n",
            "children:\n",
            "  - key: core\n",
            "    title: Core engine slice\n",
            "    effort: 2.5\n",
            "    tags: [perf]\n",
            "    paths: [src/platform/core.rs]\n",
            "  - key: wire\n",
            "    title: Surface wiring slice\n",
            "    effort: 2.5\n",
            "    tags: [cli]\n",
            "    blocked_by: [local:core]\n",
            "    paths: [src/platform/cli.rs]\n",
            "  - key: docs\n",
            "    title: Migration docs and tests\n",
            "    effort: 2.5\n",
            "    blocked_by: [local:wire]\n",
            "    traces: []\n",
        ),
    );

    // 1. Dry-run previews every child, parent edit, effort reconciliation, and identity without
    //    creating any files or modifying the parent.
    let before_parent = std::fs::read_to_string(dir.path().join("aplan/tasks/A1-01.md")).unwrap();
    let dry = common::run(
        dir.path(),
        &[
            "split",
            "--from",
            spec_path.to_str().unwrap(),
            "--dry-run",
            "--format",
            "json",
        ],
    );
    assert_eq!(dry.code, 0, "dry-run failed: {}", dry.stderr);
    let preview_doc: serde_json::Value = serde_json::from_str(&dry.stdout).expect("preview json");
    assert_eq!(preview_doc["schema"], "aplan/split/v1");
    assert_eq!(preview_doc["applied"], false);
    assert_eq!(preview_doc["parent"]["before_status"], "in_progress");
    assert_eq!(preview_doc["parent"]["after_status"], "todo");
    assert_eq!(preview_doc["parent"]["checked_criteria_retained"], 1);
    assert_eq!(preview_doc["effort"]["before_parent_days"], 8.0);
    assert_eq!(preview_doc["effort"]["after_parent_days"], 0.5);
    assert_eq!(preview_doc["effort"]["children_total_days"], 7.5);
    assert_eq!(preview_doc["effort"]["after_total_days"], 8.0);
    assert!(!dir.path().join("aplan/tasks/A1-03.md").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aplan/tasks/A1-01.md")).unwrap(),
        before_parent
    );

    // Agent format emits deterministic instruction naming `--if-match`.
    assert_split_agent_dry_run_and_apply(dir.path(), &spec_path, &preview_doc);
    assert_post_split_yaml_and_graph_views(dir.path());
}

fn assert_split_agent_dry_run_and_apply(
    dir: &Path,
    spec_path: &Path,
    preview_doc: &serde_json::Value,
) {
    let agent = common::run(
        dir,
        &[
            "split",
            "--from",
            spec_path.to_str().unwrap(),
            "--dry-run",
            "--format",
            "agent",
        ],
    );
    assert_eq!(agent.code, 0, "agent dry-run: {}", agent.stderr);
    let agent_doc: serde_json::Value = serde_json::from_str(&agent.stdout).expect("agent json");
    let identity = preview_doc["preview_identity"]
        .as_str()
        .expect("preview identity");
    assert!(
        agent_doc["agent_instruction"]
            .as_str()
            .unwrap_or("")
            .contains(identity),
        "{agent_doc}"
    );

    let applied = common::run(
        dir,
        &[
            "split",
            "--from",
            spec_path.to_str().unwrap(),
            "--if-match",
            identity,
            "--format",
            "json",
        ],
    );
    assert_eq!(applied.code, 0, "apply failed: {}", applied.stderr);
    let applied_doc: serde_json::Value = serde_json::from_str(&applied.stdout).expect("apply json");
    assert_eq!(applied_doc["applied"], true);
    assert_eq!(applied_doc["children"][0]["id"], "A1-03");
    assert_eq!(
        applied_doc["children"][0]["tags"],
        serde_json::json!(["perf", "core"])
    );
    assert_eq!(applied_doc["children"][1]["id"], "A1-04");
    assert_eq!(
        applied_doc["children"][1]["tags"],
        serde_json::json!(["cli"])
    );
    assert_eq!(
        applied_doc["children"][1]["blocked_by"],
        serde_json::json!(["A1-03"])
    );
    assert_eq!(applied_doc["children"][2]["traces"], serde_json::json!([]));
}

fn assert_post_split_yaml_and_graph_views(dir: &Path) {
    let parent_text = std::fs::read_to_string(dir.join("aplan/tasks/A1-01.md")).unwrap();
    assert!(
        parent_text.contains("# keep parent header comment"),
        "{parent_text}"
    );
    assert!(parent_text.contains("# legacy estimate"), "{parent_text}");
    assert!(parent_text.contains("effort_days: 0.5"), "{parent_text}");
    assert!(parent_text.contains("status: todo"), "{parent_text}");
    assert!(
        parent_text.contains("- [x] spike notes kept verbatim on parent"),
        "{parent_text}"
    );

    let c1_text = std::fs::read_to_string(dir.join("aplan/tasks/A1-03.md")).unwrap();
    assert!(
        !c1_text.contains("- [x]"),
        "never copy checked criteria to child: {c1_text}"
    );

    // Immediately clean under `check` (APLAN067 resolved, APLAN066 clean, APLAN030 clean).
    let check = common::run(dir, &["check"]);
    assert_eq!(
        check.code, 0,
        "post-split check failed:\n{}{}",
        check.stdout, check.stderr
    );

    // Parity with ordinary graph views: `ready`, `show`, `why`, `tree`, `summary`, `roadmap`.
    let ready = common::run(dir, &["ready", "--format", "json"]);
    assert_eq!(ready.code, 0, "{}", ready.stderr);
    let ready_doc: serde_json::Value = serde_json::from_str(&ready.stdout).expect("ready json");
    let ready_ids: Vec<&str> = ready_doc["ready"]
        .as_array()
        .expect("ready arr")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert_eq!(
        ready_ids,
        vec!["A1-03"],
        "only first increment should be ready: {ready_doc}"
    );

    let show = common::run(dir, &["show", "A1-01", "--format", "json"]);
    assert_eq!(show.code, 0, "{}", show.stderr);
    let show_doc: serde_json::Value = serde_json::from_str(&show.stdout).expect("show json");
    assert_eq!(show_doc["decomposition"]["children_total"], 3);
    assert_eq!(show_doc["decomposition"]["current_increment"], "A1-03");
    assert_eq!(show_doc["decomposition"]["total_effort_days"], 8.0);

    let summary = common::run(dir, &["summary", "--format", "json"]);
    assert_eq!(summary.code, 0, "{}", summary.stderr);
    let sum_doc: serde_json::Value = serde_json::from_str(&summary.stdout).expect("summary json");
    // Single-counted total: 8.0 (A1-01 + 3 children) + 1.5 (A1-02) = 9.5d.
    assert_eq!(sum_doc["total_effort_days"], 9.5, "{sum_doc}");

    let why = common::run(dir, &["why", "A1-02", "--format", "json"]);
    assert_eq!(why.code, 0, "{}", why.stderr);
    assert!(why.stdout.contains("A1-01"), "{}", why.stdout);
    let tree = common::run(dir, &["tree", "A1-01"]);
    assert_eq!(tree.code, 0, "{}", tree.stderr);
    assert!(
        tree.stdout.contains("A1-03") && tree.stdout.contains("A1-04"),
        "{}",
        tree.stdout
    );
}

#[test]
fn adversarial_refusals_stale_previews_and_atomic_rollback() {
    let dir = tempdir("split-adversarial-");
    setup_adversarial_plan(dir.path());
    let snapshot = snapshot_plan_files(&dir.path().join("aplan/tasks"));
    assert_adversarial_cli_refusals(dir.path(), &snapshot);
    assert_stale_preview_and_publication_rollback(dir.path(), &snapshot);
}

fn setup_adversarial_plan(dir: &Path) {
    setup_oversized_plan(dir);
    write(
        &dir.join("aplan/tasks/M-2.md"),
        "---\nid: M-2\ntitle: Other cut\nphase: 2\nstatus: todo\n\
         tags: [milestone, v2]\nblocked_by: [A1-10, A1-11, A1-12]\n---\n",
    );
    write(
        &dir.join("aplan/tasks/A1-10.md"),
        "---\nid: A1-10\ntitle: Done task\nphase: 2\nstatus: done\neffort_days: 2\n\
         paths: [src/d.rs]\ntags: [core]\n\
         evidence:\n  - kind: note\n    ref: closed\n    captured: 2026-10-01\n---\n",
    );
    write(
        &dir.join("aplan/tasks/A1-11.md"),
        "---\nid: A1-11\ntitle: Already decomposed\nphase: 2\nstatus: todo\n\
         blocked_by: [A1-12]\nintegrates: [A1-12]\npaths: [src/p.rs]\ntags: [core]\n---\n",
    );
    write(
        &dir.join("aplan/tasks/A1-12.md"),
        "---\nid: A1-12\ntitle: Child task\nphase: 2\nstatus: todo\neffort_days: 1\n\
         paths: [src/c.rs]\ntags: [core]\n---\n",
    );
}

const ADVERSARIAL_CASES: &[(&str, &[&str])] = &[
    ("empty", &["split", "A1-01"]),
    ("single child", &["split", "A1-01", "--child", "One=8"]),
    (
        "duplicate title",
        &[
            "split",
            "A1-01",
            "--child",
            "Dup=4",
            "--child",
            "Dup=4",
            "--reestimate",
        ],
    ),
    (
        "effort mismatch",
        &["split", "A1-01", "--child", "A=2", "--child", "B=2"],
    ),
    (
        "cross-cut",
        &[
            "split",
            "A1-01",
            "--cut",
            "M-2",
            "--child",
            "A=2",
            "--child",
            "B=2",
            "--reestimate",
        ],
    ),
    (
        "terminal done",
        &["split", "A1-10", "--child", "A=1", "--child", "B=1"],
    ),
    (
        "milestone target",
        &[
            "split",
            "M-1",
            "--child",
            "A=1",
            "--child",
            "B=1",
            "--reestimate",
        ],
    ),
    (
        "already decomposed",
        &[
            "split",
            "A1-11",
            "--child",
            "A=1",
            "--child",
            "B=1",
            "--reestimate",
        ],
    ),
    (
        "over-limit child APLAN067",
        &["split", "A1-01", "--child", "Big=5", "--child", "Small=3"],
    ),
    (
        "over-limit kept parent APLAN067",
        &[
            "split",
            "A1-01",
            "--keep-effort",
            "8.0",
            "--reestimate",
            "--child",
            "A=2",
            "--child",
            "B=2",
        ],
    ),
];

fn assert_adversarial_cli_refusals(
    dir: &Path,
    snapshot: &std::collections::BTreeMap<String, String>,
) {
    for (label, args) in ADVERSARIAL_CASES {
        assert_ne!(
            common::run(dir, args).code,
            0,
            "case `{label}` must fail closed"
        );
    }
    let cyc_spec = dir.join("cyc.yml");
    write(
        &cyc_spec,
        "- title: Cyclic child\n  effort: 2.5\n  blocked_by: [A1-02]\n- title: Second child\n  effort: 2.5\n",
    );
    assert_ne!(
        common::run(
            dir,
            &[
                "split",
                "A1-01",
                "--from",
                cyc_spec.to_str().unwrap(),
                "--reestimate"
            ]
        )
        .code,
        0
    );
    assert_eq!(snapshot_plan_files(&dir.join("aplan/tasks")), *snapshot);
}

fn assert_stale_preview_and_publication_rollback(
    dir: &Path,
    expected_initial: &std::collections::BTreeMap<String, String>,
) {
    // Stale preview refused when plan changes between `--dry-run` and apply.
    let dry = common::run(
        dir,
        &[
            "split",
            "A1-01",
            "--child",
            "Part A=2.5",
            "--child",
            "Part B=2.5",
            "--reestimate",
            "--dry-run",
            "--format",
            "json",
        ],
    );
    assert_eq!(dry.code, 0, "{}", dry.stderr);
    let id = serde_json::from_str::<serde_json::Value>(&dry.stdout).unwrap()["preview_identity"]
        .as_str()
        .unwrap()
        .to_string();

    // Mutate an unrelated file so snapshot digest moves.
    write(
        &dir.join("aplan/tasks/A1-12.md"),
        "---\nid: A1-12\ntitle: Child task edited\nphase: 2\nstatus: todo\neffort_days: 1\n\
         paths: [src/c.rs]\ntags: [core]\n---\n",
    );
    let stale = common::run(
        dir,
        &[
            "split",
            "A1-01",
            "--child",
            "Part A=2.5",
            "--child",
            "Part B=2.5",
            "--reestimate",
            "--if-match",
            &id,
        ],
    );
    assert_ne!(stale.code, 0, "stale preview must refuse");
    assert!(stale.stderr.contains("stale preview"), "{}", stale.stderr);
    assert!(!dir.join("aplan/tasks/A1-13.md").exists());

    // Restore A1-12, then inject a publication failure (blocking directory at second child path)
    // to prove atomic rollback leaves parent, cut, and first child completely untouched.
    write(
        &dir.join("aplan/tasks/A1-12.md"),
        expected_initial.get("A1-12.md").unwrap(),
    );
    let blocker_dir = dir.join("aplan/tasks/A1-99.md");
    std::fs::create_dir_all(&blocker_dir).expect("blocker dir");
    let spec = dir.path_buf_join("rollback.yml");
    write(
        &spec,
        "- id: A1-98\n  title: First child\n  effort: 2.5\n\
         - id: A1-99\n  title: Colliding child path\n  effort: 2.5\n",
    );
    let fail = common::run(
        dir,
        &[
            "split",
            "A1-01",
            "--from",
            spec.to_str().unwrap(),
            "--reestimate",
        ],
    );
    assert_ne!(fail.code, 0, "publication failure must fail");
    std::fs::remove_dir_all(&blocker_dir).expect("clean blocker dir");
    assert!(
        !dir.join("aplan/tasks/A1-98.md").exists(),
        "first child must not survive rolled-back split"
    );
    assert_eq!(
        snapshot_plan_files(&dir.join("aplan/tasks")),
        *expected_initial
    );
}

trait PathJoinExt {
    fn path_buf_join(&self, rel: &str) -> std::path::PathBuf;
}
impl PathJoinExt for Path {
    fn path_buf_join(&self, rel: &str) -> std::path::PathBuf {
        self.join(rel)
    }
}

fn snapshot_plan_files(dir: &Path) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("entry");
        if entry.path().is_file() {
            let name = entry.file_name().to_string_lossy().to_string();
            let text = std::fs::read_to_string(entry.path()).expect("read");
            map.insert(name, text);
        }
    }
    map
}

fn run_mcp_stdio(dir: &Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = common::command(dir, &["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aplan mcp");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for req in requests {
            writeln!(stdin, "{req}").expect("write");
        }
        stdin.flush().ok();
    }
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "mcp failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect()
}

fn mcp_reply(replies: &[serde_json::Value], id: i64) -> &serde_json::Value {
    replies
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("missing reply id {id} in {replies:#?}"))
}

fn mcp_split_call(dir: &Path, args: &serde_json::Value) -> serde_json::Value {
    let replies = run_mcp_stdio(
        dir,
        &[
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "split-test", "version": "1" }
                }
            }),
            serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "aplan_split", "arguments": args }
            }),
        ],
    );
    mcp_reply(&replies, 2).clone()
}

#[test]
fn mcp_stdio_and_tui_share_the_split_contract_and_cancel_safely() {
    let dir = tempdir("split-mcp-tui-");
    setup_oversized_plan(dir.path());

    let children_json = serde_json::json!([
        { "key": "a", "title": "First bounded increment", "effort_days": 2.5 },
        { "key": "b", "title": "Second bounded increment", "effort_days": 2.5, "blocked_by": ["local:a"] },
        { "key": "c", "title": "Third bounded increment", "effort_days": 3.0, "blocked_by": ["local:b"] }
    ]);
    let dry_res = mcp_split_call(
        dir.path(),
        &serde_json::json!({
            "target": "A1-01",
            "children": children_json,
            "dry_run": true
        }),
    );
    assert_eq!(
        dry_res["result"]["isError"],
        serde_json::json!(false),
        "{dry_res:#?}"
    );
    let dry_payload: serde_json::Value =
        serde_json::from_str(dry_res["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(dry_payload["applied"], false);
    assert!(!dir.path().join("aplan/tasks/A1-03.md").exists());
    let identity = dry_payload["preview_identity"]
        .as_str()
        .unwrap()
        .to_string();

    #[cfg(feature = "tui")]
    assert_tui_split_preview_cancel_has_no_side_effects(dir.path());

    let bad_res = mcp_split_call(
        dir.path(),
        &serde_json::json!({
            "target": "A1-01",
            "children": children_json,
            "expected_identity": "0000000000000000"
        }),
    );
    assert_eq!(bad_res["result"]["isError"], serde_json::json!(true));
    assert!(!dir.path().join("aplan/tasks/A1-03.md").exists());

    let ok_res = mcp_split_call(
        dir.path(),
        &serde_json::json!({
            "target": "A1-01",
            "children": children_json,
            "expected_identity": identity
        }),
    );
    assert_eq!(
        ok_res["result"]["isError"],
        serde_json::json!(false),
        "{ok_res:#?}"
    );
    assert!(dir.path().join("aplan/tasks/A1-03.md").exists());
    let check = common::run(dir.path(), &["check"]);
    assert_eq!(
        check.code, 0,
        "post-MCP-split check: {}{}",
        check.stdout, check.stderr
    );
}

#[cfg(feature = "tui")]
fn assert_tui_split_preview_cancel_has_no_side_effects(dir: &Path) {
    let before = snapshot_plan_files(&dir.join("aplan/tasks"));
    let cfg_path = dir.join(".aplan.yml");
    let cfg = aplan_config::parse_config_bytes(&std::fs::read(&cfg_path).unwrap(), &cfg_path)
        .expect("cfg");
    let opts = aplan_app::configured_options(
        &cfg,
        aplan_app::ConfigurationInputs {
            directory: Some(dir),
            file: Some(cfg_path.clone()),
            no_require_effort: false,
            traces_root: None,
            no_traces: false,
            default_traces: Some(dir.to_path_buf()),
            traces_repository: Some(dir.to_path_buf()),
        },
    );
    let ctx = aplan_app::PlanContext::new(
        aplan_core::load_plan_identified(&dir.join("aplan")),
        aplan_app::ContextInputs {
            repository_root: dir.to_path_buf(),
            configuration_file: Some(cfg_path),
            trust: aplan_app::Trust::Hermetic,
            options: opts,
            verdict: aplan_app::configured_verdict(&cfg),
        },
    )
    .with_captured_report_config(cfg)
    .expect("ctx");

    let mut form = aplan_tui::authoring::form(
        &ctx,
        aplan_tui::authoring::AuthoringWorkflow::Split,
        Some("A1-01"),
    )
    .expect("split form");
    let idx = form
        .fields()
        .iter()
        .position(|f| f.spec().key() == "children")
        .expect("children field");
    while form.focus() != aplan_tui::form::FormFocus::Field(idx) {
        let _ = form.reduce(aplan_tui::form::FormAction::NextFocus);
    }
    let _ = form.reduce(aplan_tui::form::FormAction::Insert(
        "TUI part one=3".to_string(),
    ));
    let _ = form.reduce(aplan_tui::form::FormAction::AddItem);
    let _ = form.reduce(aplan_tui::form::FormAction::Insert(
        "TUI part two=2.5".to_string(),
    ));
    let _ = form.reduce(aplan_tui::form::FormAction::AddItem);
    let _ = form.reduce(aplan_tui::form::FormAction::Insert(
        "TUI part three=2.5".to_string(),
    ));
    let aplan_tui::form::FormTransition::PreviewRequested(sub) =
        form.reduce(aplan_tui::form::FormAction::Review)
    else {
        panic!("expected preview transition");
    };
    let candidate = aplan_tui::mutations::preview(
        &ctx,
        aplan_tui::mutations::MutationAction::Split,
        Some("A1-01"),
        &sub,
        None,
    )
    .expect("tui split preview");
    let aplan_tui::mutations::MutationCandidate::Plan {
        change,
        review_rows,
        ..
    } = candidate
    else {
        panic!("expected plan candidate");
    };
    assert_eq!(change.disposition(), aplan_app::ChangeDisposition::Ready);
    assert!(
        review_rows.iter().any(|r| r.contains("split `A1-01`")),
        "{review_rows:?}"
    );
    // Cancel before confirmation by dropping `change` without committing.
    drop(change);
    assert_eq!(
        snapshot_plan_files(&dir.join("aplan/tasks")),
        before,
        "cancelling TUI split preview must have zero side effects"
    );
}

#[test]
fn cli_split_covers_from_file_stdin_and_argument_errors() {
    let dir = tempdir("split-cli-from-");
    write(
        &dir.path().join(".aplan.yml"),
        "root: aplan\nrequire_effort: true\n",
    );
    write(
        &dir.path().join("aplan/tasks/A1-01.md"),
        "---\nid: A1-01\ntitle: Flat task\nphase: 1\nstatus: todo\neffort_days: 4\n---\n\n## Why\n\nX.\n",
    );
    let spec_path = dir.path().join("split.yml");
    write(
        &spec_path,
        "target: A1-01\nchildren:\n  - title: Slice A\n    effort: 2\n  - title: Slice B\n    effort: 2\n",
    );
    let list_path = dir.path().join("list.yml");
    write(
        &list_path,
        "- title: Slice A\n  effort: 2\n- title: Slice B\n  effort: 2\n",
    );

    let missing_spec = common::run(
        dir.path(),
        &["split", "A1-01", "--from", "does-not-exist.yml"],
    );
    assert_eq!(missing_spec.code, 2, "{}", missing_spec.stderr);

    // Passing explicit CLI target with a list-only `--from` file succeeds.
    let cli_target_list = common::run(
        dir.path(),
        &[
            "split",
            "A1-01",
            "--from",
            list_path.to_str().unwrap(),
            "--dry-run",
            "--format",
            "json",
        ],
    );
    assert_eq!(cli_target_list.code, 0, "{}", cli_target_list.stderr);

    let conflict = common::run(
        dir.path(),
        &["split", "A1-99", "--from", spec_path.to_str().unwrap()],
    );
    assert_eq!(conflict.code, 2, "{}", conflict.stderr);
    assert!(conflict.stderr.contains("conflicting split target"));

    let missing_file_target = common::run(
        dir.path(),
        &["split", "--from", list_path.to_str().unwrap()],
    );
    assert_eq!(
        missing_file_target.code, 2,
        "{}",
        missing_file_target.stderr
    );
    assert!(
        missing_file_target
            .stderr
            .contains("target task id is required")
    );

    let missing_inline_target = common::run(
        dir.path(),
        &["split", "--child", "One=2", "--child", "Two=2"],
    );
    assert_eq!(
        missing_inline_target.code, 2,
        "{}",
        missing_inline_target.stderr
    );

    let mut stdin_child = common::command(
        dir.path(),
        &["split", "--from", "-", "--dry-run", "--format", "agent"],
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("spawn split stdin");
    stdin_child
        .stdin
        .take()
        .unwrap()
        .write_all(
            b"target: A1-01\nchildren:\n  - title: Stdin A\n    effort: 2\n  - title: Stdin B\n    effort: 2\n",
        )
        .unwrap();
    let stdin_out = stdin_child.wait_with_output().expect("wait split stdin");
    assert_eq!(
        stdin_out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&stdin_out.stderr)
    );
    let dry_agent: serde_json::Value =
        serde_json::from_slice(&stdin_out.stdout).expect("agent json");
    assert_eq!(dry_agent["applied"], false);
    assert!(
        dry_agent["agent_instruction"]
            .as_str()
            .unwrap()
            .contains("--if-match")
    );
}

#[test]
fn cli_split_renders_agent_and_human_applied_outputs_with_notes() {
    let dir = tempdir("split-cli-render-");
    write(
        &dir.path().join(".aplan.yml"),
        "root: aplan\nrequire_effort: true\nmax_effort_days: 1.5\nadvisory: [APLAN067]\n",
    );
    write(
        &dir.path().join("aplan/tasks/A1-01.md"),
        "---\nid: A1-01\ntitle: Flat task\nphase: 1\nstatus: todo\neffort_days: 4\n---\n\n## Why\n\nX.\n",
    );
    let spec_path = dir.path().join("split.yml");
    write(
        &spec_path,
        "target: A1-01\nchildren:\n  - title: Slice A\n    effort: 2\n  - title: Slice B\n    effort: 2\n",
    );

    let human_dry = common::run(
        dir.path(),
        &[
            "split",
            "--from",
            spec_path.to_str().unwrap(),
            "--dry-run",
            "--format",
            "human",
        ],
    );
    assert_eq!(human_dry.code, 0, "{}", human_dry.stderr);
    assert!(
        human_dry.stdout.contains("dry-run:") && human_dry.stdout.contains("note:"),
        "{}",
        human_dry.stdout
    );

    let apply_agent = common::run(
        dir.path(),
        &[
            "split",
            "--from",
            spec_path.to_str().unwrap(),
            "--format",
            "agent",
        ],
    );
    assert_eq!(apply_agent.code, 0, "{}", apply_agent.stderr);
    let applied_doc: serde_json::Value =
        serde_json::from_str(&apply_agent.stdout).expect("applied agent json");
    assert_eq!(applied_doc["applied"], true);
    assert!(
        applied_doc["agent_instruction"]
            .as_str()
            .unwrap()
            .contains("Split applied atomically")
    );

    write(
        &dir.path().join("aplan/tasks/A1-10.md"),
        "---\nid: A1-10\ntitle: Second flat task\nphase: 1\nstatus: todo\neffort_days: 2\n---\n\n## Why\n\nY.\n",
    );
    let apply_human = common::run(
        dir.path(),
        &[
            "split",
            "A1-10",
            "--child",
            "Human A=1.5",
            "--child",
            "Human B=1.5",
            "--sequential",
            "--reestimate",
            "--format",
            "human",
        ],
    );
    assert_eq!(apply_human.code, 0, "{}", apply_human.stderr);
    assert!(
        apply_human
            .stdout
            .contains("split `A1-10` into 2 child increment(s)")
            && apply_human.stdout.contains("(reestimated)")
            && apply_human.stdout.contains("[blocked_by:"),
        "{}",
        apply_human.stdout
    );
}
