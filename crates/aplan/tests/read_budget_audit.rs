//! Cross-channel read-surface latency, work-counter, cache-state, and
//! CLI/MCP/TUI semantic-parity audit (`A2-134`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

use aplan::{COMMAND_SURFACES, cli_command, read_surface_inventory};
use aplan_mcp::resources::{RESOURCE_TEMPLATES, RESOURCES};
use aplan_mcp::tools::TOOLS;
use aplan_output::stream::{OutputDisposition, ReadChannel, ReadLatencyClass};

use crate::common;

fn tmp(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("aplan-budget-{name}-"))
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

fn parse_timings(stderr: &str) -> BTreeMap<String, String> {
    let line = stderr
        .lines()
        .find(|line| line.starts_with("aplan: timings "))
        .unwrap_or_else(|| panic!("missing timing diagnostic in stderr:\n{stderr}"));
    line.trim_start_matches("aplan: timings ")
        .split_whitespace()
        .map(|field| {
            let (k, v) = field
                .split_once('=')
                .unwrap_or_else(|| panic!("malformed timing field `{field}`"));
            (k.to_string(), v.to_string())
        })
        .collect()
}

fn timing_u64(timings: &BTreeMap<String, String>, key: &str) -> u64 {
    timings
        .get(key)
        .unwrap_or_else(|| panic!("missing `{key}` in timings: {timings:?}"))
        .parse::<u64>()
        .unwrap_or_else(|e| panic!("non-integer `{key}` in timings: {e}"))
}

fn timing_f64(timings: &BTreeMap<String, String>, key: &str) -> f64 {
    timings
        .get(key)
        .unwrap_or_else(|| panic!("missing `{key}` in timings: {timings:?}"))
        .parse::<f64>()
        .unwrap_or_else(|e| panic!("non-float `{key}` in timings: {e}"))
}

fn run_with_cache(root: &Path, cache_dir: &Path, args: &[&str]) -> common::Run {
    let out = common::command(root, args)
        .env("APLAN_CACHE_DIR", cache_dir)
        .output()
        .expect("run aplan");
    common::Run {
        stdout: String::from_utf8(out.stdout).expect("utf8 stdout"),
        stderr: String::from_utf8(out.stderr).expect("utf8 stderr"),
        code: out.status.code().expect("exit code"),
    }
}

fn normalize_claim_epoch(raw: &str) -> serde_json::Value {
    let mut val: serde_json::Value = serde_json::from_str(raw).expect("valid JSON");
    if let Some(obj) = val.as_object_mut() {
        obj.remove("claim_observed_at_epoch_seconds");
    }
    val
}

fn mcp_session(
    repo_root: &Path,
    cache_dir: &Path,
    cache_mode: &str,
    requests: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let plan_root = repo_root.join("aplan");
    let root_str = plan_root.display().to_string();
    let mut child = common::command(
        repo_root,
        &["--cache", cache_mode, "mcp", "--root", &root_str],
    )
    .env("APLAN_CACHE_DIR", cache_dir)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("spawn aplan mcp");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for req in requests {
            writeln!(stdin, "{req}").expect("write json-rpc");
        }
        stdin.flush().ok();
    }
    let out = child.wait_with_output().expect("wait aplan mcp");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "aplan mcp exited {}: stderr:\n{stderr}",
        out.status
    );
    String::from_utf8(out.stdout)
        .expect("utf8 mcp stdout")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("json-rpc response"))
        .collect()
}

fn make_traced_repo() -> tempfile::TempDir {
    let dir = tmp("traced-repo");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("mkdir tasks");
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::write(root.join("src/lib.rs"), "pub fn audited() -> u32 { 1 }\n").expect("write src");
    let design = "# Spec\n\nAuthoritative traced section.\n";
    std::fs::write(root.join("DESIGN.md"), design).expect("write design");
    let stamp = aplan_core::stamp_in("DESIGN.md", design, "spec").expect("trace stamp");
    std::fs::write(
        root.join(".aplan.yml"),
        r"root: aplan
detect_drift: true
require_effort: true
evidence:
  observation_inputs:
    - .aplan.yml
    - 'src/**'
test:
  name: unit
  list: ['printf', 'suite::ok\n']
  run: ['true', '{test}']
  run_selects: true
",
    )
    .expect("write config");
    std::fs::write(
        root.join("aplan/tasks/M-01.md"),
        "---\nid: M-01\ntitle: Audit cut\nphase: 1\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [T-001, T-002, T-003]\n---\n\n## Why\n\nCut.\n",
    )
    .expect("write M-01");
    std::fs::write(
        root.join("aplan/tasks/T-001.md"),
        format!(
            "---\nid: T-001\ntitle: Closed traced task\nphase: 1\nstatus: done\n\
             effort_days: 1\nclosed_at: \"2026-01-01T00:00:00Z\"\npaths:\n- src/lib.rs\n\
             traces:\n- DESIGN.md#spec\ntrace_fingerprints:\n  DESIGN.md#spec: \"{stamp}\"\n\
             evidence:\n- kind: test\n  ref: suite::ok\n---\n\n## Why\n\nDone.\n"
        ),
    )
    .expect("write T-001");
    std::fs::write(
        root.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: Dependent task\nphase: 1\nstatus: todo\neffort_days: 1\n\
         blocked_by: [T-001]\npaths:\n- src/lib.rs\n---\n\n## Why\n\nWork.\n",
    )
    .expect("write T-002");
    std::fs::write(
        root.join("aplan/tasks/T-003.md"),
        "---\nid: T-003\ntitle: Ready task alpha\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n- src/lib.rs\n---\n\n## Why\n\nReady.\n",
    )
    .expect("write T-003");

    common::git(root, &["init", "-q", "."]);
    common::git(root, &["config", "user.email", "test@example.com"]);
    common::git(root, &["config", "user.name", "test"]);
    common::git(root, &["add", "-A"]);
    common::git(root, &["commit", "-q", "-m", "initial audit repo"]);
    dir
}

fn tui_check_diagnostics(
    repo_root: &Path,
    cache_dir: &Path,
    mode: aplan_app::CacheMode,
) -> (usize, Vec<String>) {
    let config_path = repo_root.join(".aplan.yml");
    let config_bytes = std::fs::read(&config_path).expect("read config");
    let config =
        aplan_config::parse_config_bytes(&config_bytes, &config_path).expect("parse config");
    let plan_root = repo_root.join("aplan");
    let store = aplan_app::PlanStore::open(
        &plan_root,
        &aplan_app::PlanStoreOptions {
            mode,
            configuration_file: Some(config_path.clone()),
            cache_root: Some(cache_dir.to_path_buf()),
            writer_identity: Some("audit-tui-test".to_string()),
            retain_inputs: true,
            include_input_digests: true,
            defer_publication: false,
        },
    )
    .expect("open plan store");
    let options = aplan_core::Options {
        require_effort: config.require_effort.unwrap_or(false),
        detect_drift: config.detect_drift.unwrap_or(false),
        traces_root: Some(repo_root.to_path_buf()),
        ..aplan_core::Options::default()
    };
    let context = aplan_app::PlanContext::from_shared(
        store.snapshot_arc(),
        aplan_app::ContextInputs {
            repository_root: repo_root.to_path_buf(),
            configuration_file: Some(config_path),
            trust: aplan_app::Trust::Hermetic,
            options,
            verdict: aplan_core::Verdict::default(),
        },
    )
    .with_captured_report_config(config)
    .expect("captured config");
    #[cfg(feature = "tui")]
    {
        let build = aplan_app::BuildIdentity {
            version: env!("CARGO_PKG_VERSION").to_string(),
            source: "audit-tui-test".to_string(),
            facts_sha256: "audit-facts".to_string(),
            binary_sha256: "audit-binary".to_string(),
        };
        let sheet =
            aplan_app::verification_run_sheet(&context, &aplan_evidence::VerifyOptions::default());
        let _prepared = aplan_tui::prepare_snapshot(&context, &build, sheet);
    }
    let check = aplan_app::check(&context, aplan_app::CheckRequest);
    let mut codes: Vec<String> = check
        .report
        .diagnostics
        .iter()
        .map(|d| d.code.to_string())
        .collect();
    codes.sort();
    (context.plan().tasks.len(), codes)
}

fn mcp_check_and_show_codes(
    repo_root: &Path,
    cache_dir: &Path,
    cache_mode: &str,
) -> (Vec<String>, serde_json::Value) {
    let replies = mcp_session(
        repo_root,
        cache_dir,
        cache_mode,
        &[
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "audit", "version": "1"}
                }
            }),
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "aplan_check", "arguments": {}}
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {"name": "aplan_show", "arguments": {"id": "T-001"}}
            }),
        ],
    );
    let check_reply = replies.iter().find(|r| r["id"] == 2).expect("check reply");
    let mut codes: Vec<String> = check_reply["result"]["structuredContent"]["data"]["diagnostics"]
        .as_array()
        .expect("mcp diagnostics array")
        .iter()
        .map(|d| d["code"].as_str().expect("code").to_string())
        .collect();
    codes.sort();
    let show_reply = replies.iter().find(|r| r["id"] == 3).expect("show reply");
    let show_data = show_reply["result"]["structuredContent"]["data"].clone();
    (codes, show_data)
}

fn cli_check_codes(stdout: &str) -> Vec<String> {
    let val: serde_json::Value = serde_json::from_str(stdout).expect("check JSON");
    let mut codes: Vec<String> = val["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .map(|d| d["code"].as_str().expect("code").to_string())
        .collect();
    codes.sort();
    codes
}

fn corrupt_plan_cache_files(cache_dir: &Path) -> usize {
    let plan_dir = cache_dir.join("aplan/plan-v1");
    let mut count = 0;
    for entry in std::fs::read_dir(&plan_dir).expect("read plan-v1") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|ext| ext == "bin") {
            std::fs::write(&path, b"corrupt-generation-bytes").expect("corrupt bin");
            count += 1;
        }
    }
    count
}

/// Proves `read.reports_latency_classes_and_work_counters`: every CLI, MCP and
/// TUI read surface is derived from live registries with one latency class.
#[test]
fn derived_inventory_assigns_every_cli_mcp_and_tui_surface_one_latency_class() {
    let inventory = read_surface_inventory();
    let mut ids = BTreeSet::new();
    let mut seen_classes = BTreeSet::new();
    let mut by_channel: BTreeMap<ReadChannel, usize> = BTreeMap::new();

    for entry in &inventory {
        assert!(
            ids.insert(entry.surface.clone()),
            "duplicate read-surface id: {}",
            entry.surface
        );
        assert!(!entry.atomic_error_boundary.trim().is_empty());
        assert!(!entry.rationale.trim().is_empty());
        seen_classes.insert(entry.latency_class);
        *by_channel.entry(entry.channel).or_default() += 1;
    }

    // Every latency class in the public contract must be represented.
    for class in ReadLatencyClass::ALL {
        assert!(
            seen_classes.contains(class),
            "latency class `{}` has no surface in inventory",
            class.as_str()
        );
    }

    // CLI + MCP + TUI channels are all derived and populated.
    let cli_count = *by_channel.get(&ReadChannel::Cli).unwrap_or(&0);
    let mcp_tool_count = *by_channel.get(&ReadChannel::McpTool).unwrap_or(&0);
    let mcp_res_count = *by_channel.get(&ReadChannel::McpResource).unwrap_or(&0);
    assert!(
        cli_count >= 50,
        "expected >=50 CLI surfaces, got {cli_count}"
    );
    assert_eq!(mcp_tool_count, 6, "6 read-only MCP tools");
    assert_eq!(
        mcp_res_count,
        RESOURCES.len() + RESOURCE_TEMPLATES.len(),
        "all MCP resources and templates"
    );
    #[cfg(feature = "tui")]
    {
        let tui_count = *by_channel.get(&ReadChannel::TuiOperation).unwrap_or(&0);
        assert_eq!(tui_count, 13, "6 read EffectKinds + 7 interactive Routes");
    }

    // Spot-check critical latency class boundaries and live registries.
    assert_inventory_boundaries_and_live_registries(&inventory);
}

fn assert_inventory_boundaries_and_live_registries(
    inventory: &[aplan_output::stream::ReadSurfaceEntry],
) {
    let find = |surface: &str| {
        inventory
            .iter()
            .find(|e| e.surface == surface)
            .unwrap_or_else(|| panic!("missing expected surface `{surface}`"))
    };
    assert_eq!(
        find("cli:why:json").latency_class,
        ReadLatencyClass::LocalBoundedRead
    );
    assert_eq!(
        find("cli:check:json").latency_class,
        ReadLatencyClass::FullStructuralRead
    );
    assert_eq!(
        find("cli:vacuity:json").latency_class,
        ReadLatencyClass::FullStructuralRead
    );
    assert_eq!(
        find("cli:show:json").latency_class,
        ReadLatencyClass::EvidenceBackedInspection
    );
    assert_eq!(
        find("cli:paths:json").latency_class,
        ReadLatencyClass::RepositoryRead
    );
    assert_eq!(
        find("cli:stale:json").latency_class,
        ReadLatencyClass::RepositoryRead
    );
    assert_eq!(
        find("cli:verify:json").latency_class,
        ReadLatencyClass::ExternalRefresh
    );
    assert_eq!(
        find("cli:tasks:json").latency_class,
        ReadLatencyClass::OutputDominatedRead
    );
    assert_eq!(
        find("cli:tasks:json").disposition,
        OutputDisposition::Streamed
    );
    assert_eq!(
        find("mcp:tool:aplan_verify").latency_class,
        ReadLatencyClass::ExternalRefresh
    );
    assert_eq!(
        find("mcp:tool:aplan_show").latency_class,
        ReadLatencyClass::EvidenceBackedInspection
    );
    #[cfg(feature = "tui")]
    {
        assert_eq!(
            find("tui:effect:startup:frame").latency_class,
            ReadLatencyClass::FullStructuralRead
        );
        assert_eq!(
            find("tui:effect:verify:frame").latency_class,
            ReadLatencyClass::ExternalRefresh
        );
        assert_eq!(
            find("tui:route:graph:frame").latency_class,
            ReadLatencyClass::OutputDominatedRead
        );
        assert_eq!(
            find("tui:route:completed:frame").latency_class,
            ReadLatencyClass::RepositoryRead
        );
    }

    // Confirm zero stale CLI/MCP entries outside the live command/tool registries.
    let cli = cli_command();
    let live_cli: BTreeSet<&str> = cli.get_subcommands().map(clap::Command::get_name).collect();
    let live_tools: BTreeSet<&str> = TOOLS.iter().map(|t| t.name).collect();
    let formatted_cmds: BTreeSet<&str> = COMMAND_SURFACES.iter().map(|(c, _, _)| *c).collect();
    for entry in inventory {
        match entry.channel {
            ReadChannel::Cli => {
                let top = entry.target.split_whitespace().next().unwrap_or("");
                assert!(
                    live_cli.contains(top),
                    "stale CLI read surface `{}` not in clap tree",
                    entry.target
                );
                if formatted_cmds.contains(entry.target.as_str()) {
                    assert!(!entry.format.is_empty());
                }
            }
            ReadChannel::McpTool => {
                assert!(
                    live_tools.contains(entry.target.as_str()),
                    "stale MCP tool `{}`",
                    entry.target
                );
            }
            ReadChannel::McpResource | ReadChannel::TuiOperation => {}
        }
    }
}

fn assert_clean_hit_and_one_edit_states(root: &Path, cache_dir: &Path) {
    // 1. Clean state (no cache)
    let clean_check = run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    assert_eq!(clean_check.code, 0, "clean check: {}", clean_check.stderr);
    let t_clean = parse_timings(&clean_check.stderr);
    assert_eq!(timing_u64(&t_clean, "cache_hits"), 0);
    assert_eq!(timing_u64(&t_clean, "tasks_parsed"), 4);
    assert!(timing_u64(&t_clean, "files_hashed") >= 4);
    assert_eq!(timing_u64(&t_clean, "report_builds"), 1);
    assert_eq!(timing_u64(&t_clean, "provider_spawns"), 0);

    // 2. Exact hit state
    let hit_why = run_with_cache(
        root,
        cache_dir,
        &["--timings", "why", "T-002", "--format", "json"],
    );
    assert_eq!(hit_why.code, 0, "hit why: {}", hit_why.stderr);
    let t_hit_why = parse_timings(&hit_why.stderr);
    assert_eq!(timing_u64(&t_hit_why, "cache_hits"), 1);
    assert_eq!(timing_u64(&t_hit_why, "tasks_parsed"), 0);
    assert_eq!(timing_u64(&t_hit_why, "files_hashed"), 0);
    assert_eq!(timing_u64(&t_hit_why, "report_builds"), 0);
    assert_eq!(timing_u64(&t_hit_why, "provider_spawns"), 0);

    let hit_check = run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    assert_eq!(hit_check.code, 0, "hit check: {}", hit_check.stderr);
    assert_eq!(hit_check.stdout, clean_check.stdout);
    let t_hit_check = parse_timings(&hit_check.stderr);
    assert_eq!(timing_u64(&t_hit_check, "cache_hits"), 1);
    assert_eq!(timing_u64(&t_hit_check, "tasks_parsed"), 0);
    assert_eq!(timing_u64(&t_hit_check, "trace_reads"), 0);
    assert_eq!(timing_u64(&t_hit_check, "markdown_parses"), 0);
    assert_eq!(timing_u64(&t_hit_check, "report_builds"), 0);
    assert_eq!(timing_u64(&t_hit_check, "provider_spawns"), 0);

    // 3. One-edit state: title-only keeps report_builds == 0; semantic edit rebuilds report.
    let task_path = root.join("aplan/tasks/T-003.md");
    let orig_task = std::fs::read_to_string(&task_path).expect("read T-003");
    std::fs::write(&task_path, orig_task.replace("alpha", "bravo")).expect("title edit");
    let one_edit_title =
        run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    assert_eq!(one_edit_title.code, 0);
    let t_title = parse_timings(&one_edit_title.stderr);
    assert_eq!(timing_u64(&t_title, "cache_hits"), 1);
    assert_eq!(timing_u64(&t_title, "tasks_parsed"), 1);
    assert_eq!(timing_u64(&t_title, "report_builds"), 0);
    assert_eq!(timing_u64(&t_title, "provider_spawns"), 0);

    std::fs::write(&task_path, orig_task.replace("effort_days: 1\n", "")).expect("semantic edit");
    let one_edit_sem = run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    assert_eq!(one_edit_sem.code, 0);
    let t_sem = parse_timings(&one_edit_sem.stderr);
    assert_eq!(timing_u64(&t_sem, "cache_hits"), 1);
    assert_eq!(timing_u64(&t_sem, "tasks_parsed"), 1);
    assert_eq!(timing_u64(&t_sem, "report_builds"), 1);
    assert_eq!(timing_u64(&t_sem, "provider_spawns"), 0);
    assert!(cli_check_codes(&one_edit_sem.stdout).contains(&"APLAN009".to_string()));
    std::fs::write(&task_path, orig_task).expect("restore T-003");
}

fn assert_trace_config_corrupt_off_and_stale_states(root: &Path, cache_dir: &Path) {
    // Re-warm clean baseline after restore.
    let warm = run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    assert_eq!(warm.code, 0);

    // 4. Trace-edit state: tasks unchanged, shared trace doc changed -> APLAN018 across CLI/MCP/TUI.
    let design_path = root.join("DESIGN.md");
    let orig_design = std::fs::read_to_string(&design_path).expect("read design");
    std::fs::write(&design_path, "# Spec\n\nDrifted traced content.\n").expect("edit trace");
    let trace_edit = run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    let t_trace = parse_timings(&trace_edit.stderr);
    assert_eq!(timing_u64(&t_trace, "tasks_parsed"), 0);
    assert_eq!(timing_u64(&t_trace, "report_builds"), 1);
    assert_eq!(timing_u64(&t_trace, "trace_reads"), 1);
    assert_eq!(timing_u64(&t_trace, "markdown_parses"), 1);
    assert_eq!(timing_u64(&t_trace, "provider_spawns"), 0);
    let cli_codes = cli_check_codes(&trace_edit.stdout);
    let (mcp_codes, _) = mcp_check_and_show_codes(root, cache_dir, "auto");
    let (tui_tasks, tui_codes) = tui_check_diagnostics(root, cache_dir, aplan_app::CacheMode::Auto);
    assert!(cli_codes.contains(&"APLAN018".to_string()), "{cli_codes:?}");
    assert_eq!(cli_codes, mcp_codes, "CLI vs MCP trace-edit parity");
    assert_eq!(cli_codes, tui_codes, "CLI vs TUI trace-edit parity");
    assert_eq!(tui_tasks, 4);
    std::fs::write(&design_path, orig_design).expect("restore design");

    // 5. Config-edit state: toggle require_effort / deny_warnings while tasks stay unchanged.
    let cfg_path = root.join(".aplan.yml");
    let orig_cfg = std::fs::read_to_string(&cfg_path).expect("read config");
    std::fs::write(
        &cfg_path,
        orig_cfg.replace("require_effort: true", "require_effort: false"),
    )
    .expect("edit config");
    let cfg_edit = run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    assert_eq!(cfg_edit.code, 0);
    let t_cfg = parse_timings(&cfg_edit.stderr);
    assert_eq!(timing_u64(&t_cfg, "provider_spawns"), 0);
    let (mcp_cfg_codes, _) = mcp_check_and_show_codes(root, cache_dir, "auto");
    let (_, tui_cfg_codes) = tui_check_diagnostics(root, cache_dir, aplan_app::CacheMode::Auto);
    assert_eq!(cli_check_codes(&cfg_edit.stdout), mcp_cfg_codes);
    assert_eq!(cli_check_codes(&cfg_edit.stdout), tui_cfg_codes);
    std::fs::write(&cfg_path, orig_cfg).expect("restore config");
    assert_corrupt_off_and_stale_states(root, cache_dir);
}

fn assert_corrupt_off_and_stale_states(root: &Path, cache_dir: &Path) {
    // 6. Corrupt-index & --cache=off semantic parity across CLI, MCP, and TUI.
    let off_cache = tmp("off-cache");
    let off_check = run_with_cache(
        root,
        off_cache.path(),
        &["--timings", "--cache", "off", "check", "--format", "json"],
    );
    assert_eq!(off_check.code, 0);
    assert!(
        std::fs::read_dir(off_cache.path())
            .expect("read off cache")
            .next()
            .is_none(),
        "--cache=off must write zero files"
    );
    let (mcp_off_codes, _) = mcp_check_and_show_codes(root, off_cache.path(), "off");
    let (_, tui_off_codes) =
        tui_check_diagnostics(root, off_cache.path(), aplan_app::CacheMode::Off);
    assert_eq!(cli_check_codes(&off_check.stdout), mcp_off_codes);
    assert_eq!(cli_check_codes(&off_check.stdout), tui_off_codes);

    let rewarm = run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    assert_eq!(rewarm.code, 0);
    assert!(
        corrupt_plan_cache_files(cache_dir) >= 1,
        "expected at least one .bin generation to corrupt"
    );
    let corrupt_check =
        run_with_cache(root, cache_dir, &["--timings", "check", "--format", "json"]);
    assert_eq!(corrupt_check.code, 0);
    assert_eq!(corrupt_check.stdout, off_check.stdout);
    let t_corrupt = parse_timings(&corrupt_check.stderr);
    assert_eq!(timing_u64(&t_corrupt, "cache_hits"), 0);
    assert_eq!(timing_u64(&t_corrupt, "tasks_parsed"), 4);
    assert_eq!(timing_u64(&t_corrupt, "provider_spawns"), 0);

    // 7. Stale-observation state on default `show` across CLI and MCP (zero provider spawns).
    let refresh = run_with_cache(
        root,
        cache_dir,
        &[
            "--timings",
            "show",
            "T-001",
            "--refresh-evidence",
            "--format",
            "json",
        ],
    );
    assert_eq!(refresh.code, 0, "refresh failed: {}", refresh.stderr);
    assert!(timing_u64(&parse_timings(&refresh.stderr), "provider_spawns") >= 1);

    let current_show = run_with_cache(
        root,
        cache_dir,
        &["--timings", "show", "T-001", "--format", "json"],
    );
    assert_eq!(current_show.code, 0);
    assert_eq!(
        timing_u64(&parse_timings(&current_show.stderr), "provider_spawns"),
        0
    );
    let current_json = normalize_claim_epoch(&current_show.stdout);
    assert_eq!(current_json["resolved"], true);
    assert_eq!(current_json["evidence_observation"]["state"], "current");

    let src_path = root.join("src/lib.rs");
    std::fs::write(&src_path, "pub fn audited() -> u32 { 2 }\n").expect("mutate observed src");
    let stale_show = run_with_cache(
        root,
        cache_dir,
        &["--timings", "show", "T-001", "--format", "json"],
    );
    assert_eq!(stale_show.code, 0);
    assert_eq!(
        timing_u64(&parse_timings(&stale_show.stderr), "provider_spawns"),
        0,
        "default show must never spawn providers when observation is stale"
    );
    let stale_json = normalize_claim_epoch(&stale_show.stdout);
    assert_eq!(stale_json["resolved"], false);
    assert_eq!(stale_json["evidence_observation"]["state"], "stale");

    let (_, mcp_stale_show) = mcp_check_and_show_codes(root, cache_dir, "auto");
    assert_eq!(mcp_stale_show["resolved"], false);
    assert_eq!(mcp_stale_show["evidence_observation"]["state"], "stale");
}

/// Proves `read.reports_latency_classes_and_work_counters`: all seven cache and
/// observation states preserve CLI/MCP/TUI parity and ratchet work counters.
#[test]
fn seven_cache_and_observation_states_preserve_cli_mcp_tui_parity_and_ratchet_work_counters() {
    let repo = make_traced_repo();
    let cache = tmp("state-cache");
    assert_clean_hit_and_one_edit_states(repo.path(), cache.path());
    assert_trace_config_corrupt_off_and_stale_states(repo.path(), cache.path());
}

/// Proves `read.reports_latency_classes_and_work_counters`: repository reads
/// separate `repo_ms` from plan overhead and external refresh counts spawns.
#[test]
fn repository_reads_separate_plan_overhead_and_external_refresh_reports_provider_spawns() {
    let repo = make_traced_repo();
    let cache = tmp("repo-cache");

    // Warm the plan index first so repository reads hit the snapshot.
    let warm = run_with_cache(
        repo.path(),
        cache.path(),
        &["--timings", "why", "T-002", "--format", "json"],
    );
    assert_eq!(warm.code, 0);

    for (label, argv, expect_zero_parse) in [
        (
            "paths",
            vec!["--timings", "paths", "--sample", "5", "--format", "json"],
            true,
        ),
        (
            "paths match",
            vec!["--timings", "paths", "match", "src/**", "--format", "json"],
            true,
        ),
        (
            "stale",
            vec!["--timings", "stale", "--format", "json"],
            true,
        ),
        (
            "summary --at HEAD",
            vec!["--timings", "summary", "--at", "HEAD", "--format", "json"],
            false,
        ),
        (
            "completed",
            vec!["--timings", "completed", "--format", "json"],
            true,
        ),
    ] {
        let run = run_with_cache(repo.path(), cache.path(), &argv);
        assert_eq!(
            run.code, 0,
            "{label} failed (code {}): {}",
            run.code, run.stderr
        );
        let timings = parse_timings(&run.stderr);
        assert!(timing_f64(&timings, "load_ms") >= 0.0);
        assert!(timing_f64(&timings, "repo_ms") >= 0.0);
        assert_eq!(
            timing_u64(&timings, "provider_spawns"),
            0,
            "{label} must never spawn configured evidence providers"
        );
        if expect_zero_parse {
            assert_eq!(
                timing_u64(&timings, "tasks_parsed"),
                0,
                "{label} should reuse warm plan snapshot"
            );
        }
    }

    // Explicit external refresh MUST record provider spawns > 0.
    let verify = run_with_cache(
        repo.path(),
        cache.path(),
        &["--timings", "verify", "--format", "json"],
    );
    assert_eq!(verify.code, 0, "verify failed: {}", verify.stderr);
    let t_verify = parse_timings(&verify.stderr);
    assert!(
        timing_u64(&t_verify, "provider_spawns") >= 1,
        "explicit verify must report spawned configured providers: {t_verify:?}"
    );
}
