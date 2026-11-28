//! End-to-end integration tests for repository-declared maximum live task
//! effort (`max_effort_days`, `APLAN067`, `A2-149`).

use std::path::Path;

use aplan_core::validate;

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

fn init_base_plan(dir: &Path, extra_config: &str) {
    write(
        &dir.join(".aplan.yml"),
        &format!("root: aplan\nrequire_effort: true\nrequire_cut_membership: true\n{extra_config}"),
    );
    write(
        &dir.join("aplan/tasks/M-1.md"),
        &task_md(
            "id: M-1\ntitle: Core cut\nphase: 1\nstatus: todo\ntags: [milestone, v1]\n\
             blocked_by: [A1-01, A1-02]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-01.md"),
        &task_md(
            "id: A1-01\ntitle: First bounded increment\nphase: 1\nstatus: todo\n\
             effort_days: 1.5\npaths: [src/one.rs]",
        ),
    );
    write(
        &dir.join("aplan/tasks/A1-02.md"),
        &task_md(
            "id: A1-02\ntitle: Historical completed task\nphase: 1\nstatus: done\n\
             effort_days: 8.0\npaths: [src/legacy.rs]\n\
             evidence:\n  - kind: note\n    ref: legacy record\n    captured: 2026-10-01",
        ),
    );
}

#[test]
fn boundary_fractional_exemptions_and_decomposition_rollups() {
    let dir = tempdir("max-effort-boundary-");
    init_base_plan(dir.path(), "max_effort_days: 2.5\ndeny_warnings: true\n");

    // Add decomposed parent (no own effort) + exactly-at-limit child (2.5d) + fractional child
    // (1.25d) + dropped oversized task (9.0d). Subtree total is 3.75d > 2.5d, yet every live
    // task's own effort is <= 2.5d so check must pass cleanly.
    write(
        &dir.path().join("aplan/tasks/M-1.md"),
        &task_md(
            "id: M-1\ntitle: Core cut\nphase: 1\nstatus: todo\ntags: [milestone, v1]\n\
             blocked_by: [A1-01, A1-02, A1-03, A1-04, A1-05, A1-06]",
        ),
    );
    write(
        &dir.path().join("aplan/tasks/A1-03.md"),
        &task_md(
            "id: A1-03\ntitle: Decomposed integration boundary\nphase: 1\nstatus: todo\n\
             paths: [src/parent.rs]\nblocked_by: [A1-04, A1-05]\nintegrates: [A1-04, A1-05]",
        ),
    );
    write(
        &dir.path().join("aplan/tasks/A1-04.md"),
        &task_md(
            "id: A1-04\ntitle: Exactly at limit\nphase: 1\nstatus: todo\n\
             effort_days: 2.5\npaths: [src/exact.rs]",
        ),
    );
    write(
        &dir.path().join("aplan/tasks/A1-05.md"),
        &task_md(
            "id: A1-05\ntitle: Fractional child\nphase: 1\nstatus: in_progress\n\
             effort_days: 1.25\npaths: [src/frac.rs]",
        ),
    );
    write(
        &dir.path().join("aplan/tasks/A1-06.md"),
        &task_md(
            "id: A1-06\ntitle: Dropped oversized work\nphase: 1\nstatus: dropped\n\
             effort_days: 9.0\npaths: [src/drop.rs]",
        ),
    );
    let clean = common::run(dir.path(), &["check"]);
    assert_eq!(clean.code, 0, "clean boundary check: {}", clean.stderr);

    // Just over limit (2.51d) and zero-effort live task (0d) both fire APLAN067.
    write(
        &dir.path().join("aplan/tasks/A1-07.md"),
        &task_md(
            "id: A1-07\ntitle: Just over limit\nphase: 1\nstatus: todo\n\
             effort_days: 2.51\npaths: [src/over.rs]",
        ),
    );
    write(
        &dir.path().join("aplan/tasks/A1-08.md"),
        &task_md(
            "id: A1-08\ntitle: Zero effort fiction\nphase: 1\nstatus: todo\n\
             effort_days: 0\npaths: [src/zero.rs]",
        ),
    );
    write(
        &dir.path().join("aplan/tasks/M-1.md"),
        &task_md(
            "id: M-1\ntitle: Core cut\nphase: 1\nstatus: todo\ntags: [milestone, v1]\n\
             blocked_by: [A1-01, A1-02, A1-03, A1-04, A1-05, A1-06, A1-07, A1-08]",
        ),
    );
    let json_out = common::run(dir.path(), &["check", "--format", "json"]);
    assert_eq!(json_out.code, 1);
    let doc: serde_json::Value = serde_json::from_str(&json_out.stdout).expect("check json");
    let a067: Vec<&serde_json::Value> = doc["diagnostics"]
        .as_array()
        .expect("diags")
        .iter()
        .filter(|d| d["code"] == "APLAN067")
        .collect();
    assert_eq!(a067.len(), 2, "{a067:?}");
    assert_eq!(a067[0]["task"], "A1-07");
    assert!(a067[0]["message"].as_str().unwrap_or("").contains("2.51d"));
    assert!(a067[0]["message"].as_str().unwrap_or("").contains("2.5d"));
    assert_eq!(a067[1]["task"], "A1-08");
    assert_a067_human_agent_and_agents_md_outputs(dir.path());
}

fn assert_a067_human_agent_and_agents_md_outputs(dir: &Path) {
    let human = common::run(dir, &["check"]);
    assert_eq!(human.code, 1);
    assert!(
        human.stdout.contains("APLAN067") && human.stdout.contains("2.51d"),
        "{}",
        human.stdout
    );
    let agent = common::run(dir, &["check", "--format", "agent"]);
    assert_eq!(agent.code, 1);
    let agent_doc: serde_json::Value = serde_json::from_str(&agent.stdout).expect("agent json");
    let over = agent_doc["diagnostics"]
        .as_array()
        .expect("agent diags")
        .iter()
        .find(|d| d["code"] == "APLAN067" && d["task"] == "A1-07")
        .expect("A1-07 APLAN067 in agent format");
    let instr = over["agent_instruction"].as_str().unwrap_or("");
    assert!(
        instr.contains("integrates") && instr.contains("blocked_by") && instr.contains("2.5d"),
        "{instr}"
    );
    let agents_md = common::run(dir, &["export-agents-md"]);
    assert_eq!(agents_md.code, 0, "{}", agents_md.stderr);
    assert!(
        agents_md.stdout.contains("max_effort_days")
            && agents_md.stdout.contains("APLAN067")
            && agents_md.stdout.contains("local review-size policy"),
        "{}",
        agents_md.stdout
    );
}

#[test]
fn disabled_malformed_and_migrated_legacy_configuration_postures() {
    // 1. Omitted and explicit `null` leave oversized live tasks silent on APLAN067.
    for cfg in ["", "max_effort_days: null\n"] {
        let dir = tempdir("max-effort-disabled-");
        init_base_plan(dir.path(), cfg);
        write(
            &dir.path().join("aplan/tasks/A1-01.md"),
            &task_md(
                "id: A1-01\ntitle: Legacy large live task\nphase: 1\nstatus: todo\n\
                 effort_days: 12.0\npaths: [src/legacy_live.rs]",
            ),
        );
        let out = common::run(dir.path(), &["check"]);
        assert_eq!(out.code, 0, "disabled `{cfg:?}` must pass: {}", out.stderr);
    }

    // 2. Malformed `max_effort_days` values fail config load with exit code 2.
    for bad in ["0", "-2", ".inf", "-.inf", ".nan", "\"three\"", "true"] {
        let dir = tempdir("max-effort-bad-cfg-");
        init_base_plan(dir.path(), &format!("max_effort_days: {bad}\n"));
        let out = common::run(dir.path(), &["check"]);
        assert_eq!(out.code, 2, "bad `{bad}` must fail config: {}", out.stdout);
        assert!(
            out.stderr.contains("max_effort_days"),
            "bad `{bad}` stderr: {}",
            out.stderr
        );
    }

    // 3. Migrated legacy plan: done 8.0d task stays silent; live oversized task is observed
    // without blocking when `advisory: [APLAN067]` is staged during migration.
    let mig = tempdir("max-effort-migrate-");
    init_base_plan(
        mig.path(),
        "max_effort_days: 3\nadvisory: [APLAN067]\ndeny_warnings: true\n",
    );
    write(
        &mig.path().join("aplan/tasks/A1-01.md"),
        &task_md(
            "id: A1-01\ntitle: Legacy oversized live task\nphase: 1\nstatus: todo\n\
             effort_days: 8.0\npaths: [src/legacy_live.rs]",
        ),
    );
    let mig_out = common::run(mig.path(), &["check", "--format", "json"]);
    assert_eq!(
        mig_out.code, 0,
        "advisory migration must exit 0: {}",
        mig_out.stderr
    );
    let doc: serde_json::Value = serde_json::from_str(&mig_out.stdout).expect("mig json");
    let codes: Vec<(&str, &str)> = doc["diagnostics"]
        .as_array()
        .expect("diags")
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap_or(""),
                d["task"].as_str().unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(codes, vec![("APLAN067", "A1-01")]);

    // 4. Negative, infinite, and non-numeric task estimates retain APLAN001 refusals.
    for bad_task in ["-1", ".inf", ".nan", "large"] {
        let dir = tempdir("max-effort-bad-task-");
        init_base_plan(dir.path(), "max_effort_days: 3\n");
        write(
            &dir.path().join("aplan/tasks/A1-01.md"),
            &task_md(&format!(
                "id: A1-01\ntitle: Bad estimate\nphase: 1\nstatus: todo\n\
                 effort_days: {bad_task}\npaths: [src/bad.rs]"
            )),
        );
        let out = common::run(dir.path(), &["check", "--format", "json"]);
        assert_eq!(out.code, 1, "bad task `{bad_task}` must fail");
        assert!(out.stdout.contains("APLAN001"), "{}", out.stdout);
    }
}

fn assert_cli_authoring_paths_enforce_max_effort(dir: &Path) {
    // 1. CLI `add` refuses over-limit before writing and accepts at-limit.
    let bad_add = common::run(
        dir,
        &[
            "add",
            "Too large",
            "--id",
            "A1-10",
            "--cut",
            "M-1",
            "--effort",
            "3.5",
            "--paths",
            "src/over.rs",
        ],
    );
    assert_ne!(bad_add.code, 0, "over-limit add must fail");
    assert!(
        bad_add.stderr.contains("APLAN067") && bad_add.stderr.contains("3.5d"),
        "{}",
        bad_add.stderr
    );
    assert!(!dir.join("aplan/tasks/A1-10.md").exists());

    // 2. CLI `add --from` (batch) refuses atomically before writing any task.
    let manifest = dir.join("batch.yml");
    write(
        &manifest,
        "- id: A1-11\n  title: Small ok\n  cut: M-1\n  effort: 1\n  paths: [src/ok.rs]\n\
         - id: A1-12\n  title: Oversized batch item\n  cut: M-1\n  effort: 5\n  paths: [src/big.rs]\n",
    );
    let bad_batch = common::run(dir, &["add", "--from", manifest.to_str().expect("utf8")]);
    assert_ne!(bad_batch.code, 0, "over-limit batch add must fail");
    assert!(
        bad_batch.stderr.contains("APLAN067"),
        "{}",
        bad_batch.stderr
    );
    assert!(!dir.join("aplan/tasks/A1-11.md").exists());
    assert!(!dir.join("aplan/tasks/A1-12.md").exists());

    // 3. CLI `blueprint apply` refuses oversized blueprint task before writing.
    let bp = dir.join("big.blueprint.yml");
    write(
        &bp,
        "schema: aplan/blueprint/v1\nname: big\ntitle: Big\ndescription: Oversized.\n\
         tasks:\n  - key: huge\n    title: Huge task\n    effort: 6\n    paths: [src/bp.rs]\n",
    );
    let bad_bp = common::run(
        dir,
        &[
            "blueprint",
            "apply",
            "--file",
            bp.to_str().expect("utf8"),
            "--cut",
            "M-1",
        ],
    );
    assert_ne!(bad_bp.code, 0, "over-limit blueprint must fail");
    assert!(
        bad_bp.stderr.contains("APLAN067") || bad_bp.stdout.contains("APLAN067"),
        "stderr={} stdout={}",
        bad_bp.stderr,
        bad_bp.stdout
    );

    // 4. CLI `supersede` refuses oversized successor before writing.
    let bad_sup = common::run(
        dir,
        &[
            "supersede",
            "A1-02",
            "Oversized successor",
            "--cut",
            "M-1",
            "--effort",
            "4.0",
        ],
    );
    assert_ne!(bad_sup.code, 0, "over-limit supersede must fail");
    assert!(bad_sup.stderr.contains("APLAN067"), "{}", bad_sup.stderr);

    // 5. CLI `import` runs cleanly on checklist proposals (which carry no effort).
    let checklist = dir.join("items.md");
    write(&checklist, "# Checklist\n\n- [ ] Imported item\n");
    let imp = common::run(
        dir,
        &[
            "import",
            "--format",
            "checklist",
            "--cut",
            "M-1",
            "--dry-run",
            checklist.to_str().expect("utf8"),
        ],
    );
    assert_eq!(imp.code, 0, "import dry-run: {}", imp.stderr);
}

fn assert_inbox_promote_enforces_max_effort(dir: &Path) {
    let captured = common::run(
        dir,
        &["capture", "Inbox oversized work", "--format", "json"],
    );
    assert_eq!(captured.code, 0, "capture: {}", captured.stderr);
    let doc: serde_json::Value = serde_json::from_str(&captured.stdout).expect("capture json");
    let capture_id = doc["capture"]["id"].as_str().expect("capture id");

    let bad_promote = common::run(
        dir,
        &[
            "inbox",
            "promote",
            capture_id,
            "--task-id",
            "A1-19",
            "--cut",
            "M-1",
            "--effort",
            "4.0",
            "--path",
            "src/inb_big.rs",
        ],
    );
    assert_ne!(bad_promote.code, 0, "over-limit inbox promote must fail");
    assert!(
        bad_promote.stderr.contains("APLAN067") || bad_promote.stdout.contains("APLAN067"),
        "stderr={} stdout={}",
        bad_promote.stderr,
        bad_promote.stdout
    );
    assert!(!dir.join("aplan/tasks/A1-19.md").exists());

    let ok_promote = common::run(
        dir,
        &[
            "inbox",
            "promote",
            capture_id,
            "--task-id",
            "A1-19",
            "--cut",
            "M-1",
            "--effort",
            "2.5",
            "--path",
            "src/inb_ok.rs",
        ],
    );
    assert_eq!(
        ok_promote.code, 0,
        "at-limit inbox promote: {}",
        ok_promote.stderr
    );
    assert!(dir.join("aplan/tasks/A1-19.md").exists());
}

#[test]
fn cli_mcp_and_tui_creation_paths_judge_the_same_max_effort_post_state() {
    let dir = tempdir("max-effort-surfaces-");
    init_base_plan(dir.path(), "max_effort_days: 2.5\n");
    assert_cli_authoring_paths_enforce_max_effort(dir.path());
    assert_inbox_promote_enforces_max_effort(dir.path());

    // Wire-level MCP `aplan_add` refuses over-limit before writing and accepts at-limit.
    let policy = aplan_mcp::Policy {
        options: validate::Options {
            require_effort: true,
            require_cut_membership: true,
            max_effort_days: Some(2.5),
            ..validate::Options::default()
        },
        repo_root: dir.path().to_path_buf(),
        ..aplan_mcp::Policy::default()
    };
    let server = aplan_mcp::AplanServer::new(
        dir.path().join("aplan"),
        policy,
        serde_json::json!({}),
        aplan_app::CacheMode::Off,
        "max-effort-test".to_string(),
        ("0.8.0-dev".to_string(), "0123456789ab".to_string()),
        None,
    );
    let mcp_bad = server.dispatch_structured(
        "aplan_add",
        &serde_json::json!({
            "id": "A1-20",
            "title": "MCP oversized",
            "cut": "M-1",
            "effort_days": 3.0,
            "paths": ["src/mcp_big.rs"]
        }),
    );
    assert!(mcp_bad.is_error, "{mcp_bad:?}");
    assert!(
        mcp_bad.concise_text.contains("APLAN067"),
        "{}",
        mcp_bad.concise_text
    );
    assert!(!dir.path().join("aplan/tasks/A1-20.md").exists());

    let mcp_ok = server.dispatch_structured(
        "aplan_add",
        &serde_json::json!({
            "id": "A1-20",
            "title": "MCP at limit",
            "cut": "M-1",
            "effort_days": 2.5,
            "paths": ["src/mcp_ok.rs"]
        }),
    );
    assert!(!mcp_ok.is_error, "{mcp_ok:?}");

    // TUI authoring form decodes and denies over-limit via the shared application preview.
    #[cfg(feature = "tui")]
    assert_tui_add_form_denies_over_limit(dir.path());
}

#[cfg(feature = "tui")]
fn assert_tui_add_form_denies_over_limit(dir: &Path) {
    let ctx = aplan_app::PlanContext::new(
        aplan_core::load_plan_identified(&dir.join("aplan")),
        aplan_app::ContextInputs {
            repository_root: dir.to_path_buf(),
            configuration_file: Some(dir.join(".aplan.yml")),
            trust: aplan_app::Trust::Hermetic,
            options: validate::Options {
                require_effort: true,
                require_cut_membership: true,
                max_effort_days: Some(2.5),
                ..validate::Options::default()
            },
            verdict: aplan_core::Verdict::default(),
        },
    );
    let mut form =
        aplan_tui::authoring::form(&ctx, aplan_tui::authoring::AuthoringWorkflow::Add, None)
            .expect("tui form");
    for (key, value) in [
        ("title", "TUI oversized"),
        ("id", "A1-21"),
        ("cut", "M-1"),
        ("effort", "4.5"),
        ("paths", "src/tui_big.rs"),
    ] {
        let index = form
            .fields()
            .iter()
            .position(|field| field.spec().key() == key)
            .expect("field exists");
        while form.focus() != aplan_tui::form::FormFocus::Field(index) {
            let _ = form.reduce(aplan_tui::form::FormAction::NextFocus);
        }
        let _ = form.reduce(aplan_tui::form::FormAction::Insert(value.to_string()));
    }
    let aplan_tui::form::FormTransition::PreviewRequested(sub) =
        form.reduce(aplan_tui::form::FormAction::Review)
    else {
        panic!("complete TUI form did not yield submission");
    };
    let req = aplan_tui::authoring::request(
        aplan_tui::authoring::AuthoringWorkflow::Add,
        &sub,
        aplan_tui::authoring::AuthoringAssets::default(),
    )
    .expect("decode tui add");
    let preview = req.preview(&ctx).expect("preview tui add");
    assert_eq!(
        preview.change_set().disposition(),
        aplan_app::ChangeDisposition::Denied,
        "TUI preview must deny APLAN067"
    );
    assert!(
        preview
            .change_set()
            .diagnostics()
            .added
            .iter()
            .any(|d| d.code == aplan_core::code::EFFORT_EXCEEDS_MAX)
    );
}
