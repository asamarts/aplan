//! End-to-end streaming, bounded-memory, byte-parity, early-termination, and
//! derived read-surface disposition audits (`A2-136`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use aplan::{COMMAND_SURFACES, cli_command, read_surface_inventory};
use aplan_mcp::contract::{MAX_RESOURCE_BYTES, MAX_TOOL_RESULT_BYTES};
use aplan_mcp::resources::{RESOURCE_TEMPLATES, RESOURCES};
use aplan_mcp::tools::TOOLS;
use aplan_output::stream::{
    BOUNDED_RESPONSE_THRESHOLD_BYTES, OutputDisposition, ReadChannel, STREAM_BUFFER_CAPACITY,
};

use crate::common;

fn tmp(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("aplan-stream-{name}-"))
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

fn normalize_claim_epoch(raw: &str) -> String {
    let needle = "\"claim_observed_at_epoch_seconds\": ";
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(pos) = rest.find(needle) {
        out.push_str(&rest[..pos + needle.len()]);
        let after = &rest[pos + needle.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        out.push('0');
        rest = &after[digits..];
    }
    out.push_str(rest);
    out
}

fn make_audit_repo() -> (tempfile::TempDir, String) {
    let dir = tmp("audit-repo");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("mkdir tasks");
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::write(root.join("src/lib.rs"), "pub fn audited() {}\n").expect("write src");
    std::fs::write(root.join("DOC.md"), "# Doc\n\nTracks T-001.\n").expect("write doc");
    std::fs::write(
        root.join(".aplan.yml"),
        "root: aplan\nevidence:\n  observation_inputs: [.aplan.yml, 'src/**']\n\
         test:\n  name: unit\n  list: ['printf', 'suite::t1\\n']\n  run: ['true', '{test}']\n  run_selects: true\n",
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
        "---\nid: T-001\ntitle: Closed audited task\nphase: 1\nstatus: done\n\
         effort_days: 1\nclosed_at: \"2026-01-01T00:00:00Z\"\npaths:\n- src/lib.rs\n\
         evidence:\n- kind: test\n  ref: suite::t1\n---\n\n## Why\n\nDone.\n",
    )
    .expect("write T-001");
    std::fs::write(
        root.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: Dependent task with revisit\nphase: 1\nstatus: todo\n\
         effort_days: 1\nblocked_by: [T-001]\nrevisit_at: \"2026-01-02\"\n\
         paths:\n- src/lib.rs\n---\n\n## Why\n\nWork.\n",
    )
    .expect("write T-002");
    std::fs::write(
        root.join("aplan/tasks/T-003.md"),
        "---\nid: T-003\ntitle: Ready task\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n- src/lib.rs\n---\n\n## Why\n\nReady.\n",
    )
    .expect("write T-003");

    let cap = common::run(root, &["capture", "Audit inbox record"]);
    assert_eq!(cap.code, 0, "capture failed: {}", cap.stderr);
    let inbox = common::run(root, &["inbox", "--format", "json"]);
    assert_eq!(inbox.code, 0, "inbox failed: {}", inbox.stderr);
    let inbox_json: serde_json::Value = serde_json::from_str(&inbox.stdout).expect("inbox JSON");
    let capture_id = inbox_json["captures"][0]["id"]
        .as_str()
        .expect("capture id")
        .to_string();

    common::git(root, &["init", "-q", "."]);
    common::git(root, &["config", "user.email", "test@example.com"]);
    common::git(root, &["config", "user.name", "test"]);
    common::git(root, &["add", "-A"]);
    common::git(root, &["commit", "-q", "-m", "audit fixture"]);
    (dir, capture_id)
}

fn make_large_plan(count: usize) -> tempfile::TempDir {
    let dir = tmp("large-plan");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("mkdir tasks");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("write config");
    let cuts = 10usize;
    let mut per_cut: Vec<Vec<String>> = vec![Vec::new(); cuts];
    for n in 1..=count {
        let id = format!("L-{n:05}");
        let cut = (n - 1) % cuts;
        per_cut[cut].push(id.clone());
        let blocker = if n % 5 != 1 && n > 1 {
            format!("blocked_by:\n- L-{:05}\n", n - 1)
        } else {
            String::new()
        };
        std::fs::write(
            tasks.join(format!("{id}.md")),
            format!(
                "---\nid: {id}\ntitle: Streamed read benchmark task {n} with descriptive payload\n\
                 phase: 1\nstatus: todo\neffort_days: 1\n{blocker}paths:\n- src/mod_{cut}.rs\n\
                 ---\n\n## Why\n\nGenerated large corpus task {n}.\n"
            ),
        )
        .expect("write task");
    }
    for (idx, members) in per_cut.iter().enumerate() {
        let mid = format!("M-{:02}", idx + 1);
        let mut blocked = String::new();
        for m in members {
            let _ = writeln!(blocked, "- {m}");
        }
        std::fs::write(
            tasks.join(format!("{mid}.md")),
            format!(
                "---\nid: {mid}\ntitle: Cut {}\nphase: 1\nstatus: todo\ntags: [milestone]\n\
                 blocked_by:\n{blocked}---\n\n## Why\n\nCut.\n",
                idx + 1
            ),
        )
        .expect("write milestone");
    }
    dir
}

fn mcp_talk(root: &Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let root_str = root.display().to_string();
    let mut child = common::command(Path::new("."), &["mcp", "--root", &root_str])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aplan mcp");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for req in requests {
            writeln!(stdin, "{req}").expect("write request");
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
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("invalid JSON-RPC line `{line}`: {e}\nstderr:\n{stderr}")
            })
        })
        .collect()
}

fn cli_args_for_inventory_entry(
    surface: &str,
    format: &str,
    capture_id: &str,
    scratch: &Path,
) -> Vec<String> {
    let mut args = vec![
        "--timings".to_string(),
        "--color".to_string(),
        "never".to_string(),
    ];
    match surface {
        "tree" => args.extend(["tree", "T-001"].map(str::to_string)),
        "facts" => args.push("facts".to_string()),
        "schema" => args.push("schema".to_string()),
        "explain" => args.extend(["explain", "rule", "APLAN001"].map(str::to_string)),
        "attest" => args.push("attest".to_string()),
        "export-agents-md" => args.push("export-agents-md".to_string()),
        "export-roadmap-json" => {
            let dest = scratch.join("roadmap-export.json");
            std::fs::write(
                &dest,
                r#"{"format_version":1,"project":"audit","workstreams":[{"id":"W1","title":"Audit","milestones":[{"id":"M-01","title":"Audit cut","status":"not_started","depends_on":[]}]}]}"#,
            )
            .expect("write roadmap template");
            args.extend([
                "export-roadmap-json".to_string(),
                dest.display().to_string(),
            ]);
        }
        "export" => {
            args.extend(["export", "--format", "checklist"].map(str::to_string));
        }
        "show" => {
            args.extend(["show", "T-001", "--no-resolve", "--format", format].map(str::to_string));
        }
        "why" => args.extend(["why", "T-002", "--format", format].map(str::to_string)),
        "version" => args.extend(["version", "T-001", "--format", format].map(str::to_string)),
        "check-docs" => {
            args.extend(["check-docs", "DOC.md", "--format", format].map(str::to_string));
        }
        "graph" if format == "text" => {
            args.extend(["graph", "--format", "text", "--width", "120"].map(str::to_string));
        }
        "paths match" => {
            args.extend(["paths", "match", "src/**", "--format", format].map(str::to_string));
        }
        "inbox show" => {
            args.extend(["inbox", "show", capture_id, "--format", format].map(str::to_string));
        }
        "test" => args.extend(["test", "list", "--format", format].map(str::to_string)),
        "test receipt" => args.extend(
            [
                "test",
                "receipt",
                "unit",
                "--output",
                "-",
                "--producer-repository",
                "owner/repo",
                "--producer-workflow",
                "ci/test",
                "--producer-run-id",
                "1",
                "--producer-event",
                "push",
                "--producer-privilege",
                "trusted",
            ]
            .map(str::to_string),
        ),
        other => args.extend([other, "--format", format].map(str::to_string)),
    }
    args
}

fn assert_inventory_completeness(inventory: &[aplan_output::stream::ReadSurfaceEntry]) {
    let mut keys = BTreeSet::new();
    let mut surface_ids = BTreeSet::new();
    for entry in inventory {
        assert!(
            surface_ids.insert(entry.surface.clone()),
            "duplicate canonical surface id: {entry:?}"
        );
        assert!(
            keys.insert((entry.channel, entry.target.clone(), entry.format.clone())),
            "duplicate read-surface entry: {entry:?}"
        );
        assert!(
            !entry.atomic_error_boundary.trim().is_empty(),
            "missing atomic_error_boundary on {entry:?}"
        );
        assert!(
            !entry.rationale.trim().is_empty(),
            "missing rationale on {entry:?}"
        );
    }

    let non_read_cli: BTreeSet<&str> = [
        "add",
        "amend",
        "blueprint",
        "capture",
        "claim",
        "close",
        "dep",
        "drop",
        "import",
        "init",
        "mcp",
        "mv",
        "park",
        "rekey",
        "release",
        "retarget-path",
        "review",
        "start",
        "stop",
        "supersede",
        "tag",
        "tui",
        "unpark",
    ]
    .into_iter()
    .collect();
    let cli_surfaces_in_inventory: BTreeSet<String> = inventory
        .iter()
        .filter(|e| e.channel == ReadChannel::Cli)
        .map(|e| e.target.clone())
        .collect();
    for sub in cli_command().get_subcommands() {
        let name = sub.get_name();
        if non_read_cli.contains(name) {
            assert!(
                !cli_surfaces_in_inventory.contains(name),
                "non-read command `{name}` should not be in read-surface inventory"
            );
        } else {
            assert!(
                cli_surfaces_in_inventory.contains(name),
                "CLI read command `{name}` is missing from read-surface inventory"
            );
        }
    }
    for &(cmd, surface, _) in COMMAND_SURFACES {
        if non_read_cli.contains(cmd) {
            continue;
        }
        for fmt in surface.formats() {
            assert!(
                keys.contains(&(ReadChannel::Cli, cmd.to_string(), fmt.as_str().to_string())),
                "formatted CLI read surface `{cmd} --format {}` missing from inventory",
                fmt.as_str()
            );
        }
    }

    assert_mcp_inventory_completeness(&keys);
}

fn assert_mcp_inventory_completeness(keys: &BTreeSet<(ReadChannel, String, String)>) {
    let mutating_mcp_tools: BTreeSet<&str> = [
        "aplan_add",
        "aplan_claim",
        "aplan_close",
        "aplan_dep",
        "aplan_release",
        "aplan_review",
        "aplan_start",
        "aplan_stop",
    ]
    .into_iter()
    .collect();
    for tool in TOOLS {
        let listed = keys.contains(&(
            ReadChannel::McpTool,
            tool.name.to_string(),
            "json-rpc/structured".to_string(),
        ));
        if mutating_mcp_tools.contains(tool.name) {
            assert!(
                !listed,
                "mutating MCP tool `{}` listed as read surface",
                tool.name
            );
        } else {
            assert!(
                listed,
                "MCP read tool `{}` missing from inventory",
                tool.name
            );
        }
    }
    for res in RESOURCES {
        assert!(
            keys.contains(&(
                ReadChannel::McpResource,
                res.uri.to_string(),
                res.mime_type.to_string(),
            )),
            "MCP resource `{}` missing from inventory",
            res.uri
        );
    }
    for tmpl in RESOURCE_TEMPLATES {
        assert!(
            keys.contains(&(
                ReadChannel::McpResource,
                tmpl.uri_template.to_string(),
                tmpl.mime_type.to_string(),
            )),
            "MCP resource template `{}` missing from inventory",
            tmpl.uri_template
        );
    }
}

fn drive_cli_inventory(
    inventory: &[aplan_output::stream::ReadSurfaceEntry],
    repo_root: &Path,
    capture_id: &str,
) {
    let scratch = tmp("audit-scratch");
    for entry in inventory.iter().filter(|e| e.channel == ReadChannel::Cli) {
        let argv =
            cli_args_for_inventory_entry(&entry.target, &entry.format, capture_id, scratch.path());
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let run = common::run(repo_root, &refs);
        assert_eq!(
            run.code, 0,
            "inventory CLI entry `{}` (`{}`) failed (code {}):\nstdout:\n{}\nstderr:\n{}",
            entry.target, entry.format, run.code, run.stdout, run.stderr
        );
        let timings = parse_timings(&run.stderr);
        let output_bytes = timing_u64(&timings, "output_bytes");
        let peak_buf = timing_u64(&timings, "peak_output_buffer_bytes");
        assert!(
            output_bytes > 0,
            "inventory CLI entry `{}` (`{}`) emitted 0 output bytes",
            entry.target,
            entry.format
        );
        if entry.disposition == OutputDisposition::Streamed {
            assert!(
                peak_buf <= STREAM_BUFFER_CAPACITY as u64,
                "streamed CLI entry `{}` (`{}`) exceeded 16 KiB buffer cap: peak={peak_buf}",
                entry.target,
                entry.format
            );
        }
    }
}

fn drive_mcp_inventory(inventory: &[aplan_output::stream::ReadSurfaceEntry], plan_root: &Path) {
    let mut requests = vec![
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
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    ];
    let mut expected_mcp = Vec::new();
    for (id, entry) in (2u64..).zip(inventory.iter().filter(|e| e.channel != ReadChannel::Cli)) {
        let (req, cap) = match entry.channel {
            ReadChannel::McpTool => {
                let args = match entry.target.as_str() {
                    "aplan_why" => serde_json::json!({"id": "T-002"}),
                    "aplan_show" => serde_json::json!({"id": "T-001"}),
                    _ => serde_json::json!({}),
                };
                (
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "method": "tools/call",
                        "params": {"name": entry.target, "arguments": args}
                    }),
                    MAX_TOOL_RESULT_BYTES,
                )
            }
            ReadChannel::McpResource => {
                let uri = match entry.target.as_str() {
                    "aplan://plan/summary" => "aplan://plan/summary",
                    "aplan://tasks/{id}" => "aplan://tasks/T-001",
                    "aplan://collections/ready{?limit,cursor}" => {
                        "aplan://collections/ready?limit=2"
                    }
                    "aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}" => {
                        "aplan://collections/completed?limit=2"
                    }
                    other => panic!("unhandled MCP resource in audit: {other}"),
                };
                (
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "method": "resources/read",
                        "params": {"uri": uri}
                    }),
                    MAX_RESOURCE_BYTES,
                )
            }
            ReadChannel::Cli => unreachable!(),
        };
        requests.push(req);
        expected_mcp.push((id, entry.target.clone(), cap));
    }

    let replies = mcp_talk(plan_root, &requests);
    let by_id: BTreeMap<u64, &serde_json::Value> = replies
        .iter()
        .filter_map(|r| {
            r.get("id")
                .and_then(serde_json::Value::as_u64)
                .map(|id| (id, r))
        })
        .collect();
    for (id, surface, cap) in expected_mcp {
        let reply = by_id
            .get(&id)
            .unwrap_or_else(|| panic!("missing MCP reply for `{surface}` (id {id})"));
        assert!(
            reply.get("error").is_none(),
            "MCP read surface `{surface}` returned protocol error: {reply}"
        );
        let serialized_len = serde_json::to_vec(&reply["result"])
            .expect("serialize")
            .len();
        assert!(
            serialized_len <= cap + 1024,
            "MCP read surface `{surface}` exceeded declared cap {cap}: {serialized_len} bytes"
        );
    }
}

/// Proof for `read.streams_large_outputs_with_bounded_memory`: the derived
/// cross-channel read-surface inventory covers every CLI read command/format
/// and every MCP read tool/resource/template with zero omissions or duplicates,
/// and driving every single entry proves its declared output disposition.
#[test]
fn read_surface_inventory_covers_and_drives_every_cli_and_mcp_surface() {
    let inventory = read_surface_inventory();
    assert!(!inventory.is_empty(), "inventory must not be empty");
    assert_inventory_completeness(&inventory);
    let (repo, capture_id) = make_audit_repo();
    drive_cli_inventory(&inventory, repo.path(), &capture_id);
    drive_mcp_inventory(&inventory, &repo.path().join("aplan"));
}

/// Proof for `read.streams_large_outputs_with_bounded_memory`: outputs above
/// the measured 256 KiB bounded-response threshold stream with O(1) <= 16 KiB
/// peak output buffering, produce deterministic byte-identical output across
/// repeated runs matching buffered renderers, and preflight invalid floats
/// before writing byte 0.
#[test]
fn large_read_outputs_stream_with_bounded_buffer_and_byte_parity() {
    let dir = make_large_plan(1_200);
    let root = dir.path();

    // Cases spanning human, JSON, Markdown, Mermaid, and DOT, including multiple
    // surfaces that exceed the 256 KiB bounded-response threshold.
    let cases: &[(&str, &[&str], bool, bool)] = &[
        ("tasks json", &["tasks", "--format", "json"], true, true),
        ("tasks human", &["tasks"], false, false),
        ("roadmap json", &["roadmap", "--format", "json"], true, true),
        (
            "roadmap markdown",
            &["roadmap", "--format", "markdown"],
            false,
            false,
        ),
        ("roadmap human", &["roadmap"], false, false),
        ("graph json", &["graph", "--format", "json"], true, true),
        ("graph dot", &["graph", "--format", "dot"], false, false),
        (
            "graph mermaid",
            &["graph", "--format", "mermaid"],
            false,
            false,
        ),
        ("ready json", &["ready", "--format", "json"], false, true),
        ("ready human", &["ready"], false, false),
    ];

    for &(label, subargs, expect_above_threshold, is_json) in cases {
        let mut argv = vec!["--timings", "--color", "never"];
        argv.extend_from_slice(subargs);

        let first = common::run(root, &argv);
        assert_eq!(first.code, 0, "`{label}` failed: {}", first.stderr);
        let second = common::run(root, &argv);
        assert_eq!(
            second.code, 0,
            "`{label}` second run failed: {}",
            second.stderr
        );

        assert_eq!(
            normalize_claim_epoch(&first.stdout),
            normalize_claim_epoch(&second.stdout),
            "`{label}` output was not deterministic across repeated runs"
        );

        let timings = parse_timings(&first.stderr);
        let output_bytes = timing_u64(&timings, "output_bytes");
        let peak_buf = timing_u64(&timings, "peak_output_buffer_bytes");
        assert_eq!(
            output_bytes,
            first.stdout.len() as u64,
            "`{label}` output_bytes counter must match stdout length"
        );
        assert!(
            peak_buf <= STREAM_BUFFER_CAPACITY as u64,
            "`{label}` buffered {peak_buf} bytes (> {STREAM_BUFFER_CAPACITY} byte streaming cap) \
             for {output_bytes}-byte response"
        );
        if expect_above_threshold {
            assert!(
                first.stdout.len() > BOUNDED_RESPONSE_THRESHOLD_BYTES,
                "`{label}` was expected to exceed {BOUNDED_RESPONSE_THRESHOLD_BYTES} bytes, \
                 got {} bytes",
                first.stdout.len()
            );
        }
        if is_json {
            let parsed: serde_json::Value = serde_json::from_str(&first.stdout)
                .unwrap_or_else(|e| panic!("`{label}` emitted invalid JSON: {e}"));
            assert!(parsed.is_object(), "`{label}` JSON root must be an object");
        }
    }

    assert_library_stream_parity_and_preflight(root);
}

fn assert_library_stream_parity_and_preflight(root: &Path) {
    // Verify exact byte-for-byte parity between buffered and streaming library
    // helpers on the same real plan snapshot, plus preflight zero-byte refusal.
    let loaded = aplan_core::load_plan(&root.join("aplan"));
    let graph = aplan_core::Graph::new(loaded.tasks.iter().map(|lt| &lt.task));
    let roadmap = graph.roadmap();
    let buffered_md = aplan_output::render_roadmap_markdown(&roadmap, None, None);
    let mut streamed_md = Vec::new();
    aplan_output::write_roadmap_markdown(&mut streamed_md, &roadmap, None, None)
        .expect("stream roadmap markdown");
    assert_eq!(
        String::from_utf8(streamed_md).expect("utf8"),
        buffered_md,
        "streamed roadmap markdown diverged from buffered renderer"
    );
    let buffered_roadmap_human =
        aplan_output::render_roadmap(&roadmap, aplan_output::Format::Human);
    let mut streamed_roadmap_human = Vec::new();
    aplan_output::write_roadmap_human(&mut streamed_roadmap_human, &roadmap)
        .expect("stream roadmap human");
    assert_eq!(
        String::from_utf8(streamed_roadmap_human).expect("utf8"),
        buffered_roadmap_human,
        "streamed roadmap human diverged from buffered renderer"
    );
    let buffered_roadmap_json = aplan_output::render_roadmap(&roadmap, aplan_output::Format::Json);
    let mut streamed_roadmap_json = Vec::new();
    aplan_output::write_roadmap_json(&mut streamed_roadmap_json, &roadmap)
        .expect("stream roadmap json");
    assert_eq!(
        String::from_utf8(streamed_roadmap_json).expect("utf8"),
        buffered_roadmap_json,
        "streamed roadmap json diverged from buffered renderer"
    );
    let all_ids: BTreeSet<&str> = loaded.tasks.iter().map(|lt| lt.task.id.as_str()).collect();
    let graph_view = aplan_output::GraphView::of(&graph, &all_ids);
    let mut streamed_mermaid = Vec::new();
    aplan_output::write_mermaid_of_view(&mut streamed_mermaid, &graph_view)
        .expect("stream mermaid");
    assert_eq!(
        String::from_utf8(streamed_mermaid).expect("utf8"),
        aplan_output::mermaid_of_view(&graph_view),
        "streamed mermaid diverged from buffered renderer"
    );
    let mut streamed_dot = Vec::new();
    aplan_output::write_dot_of_view(&mut streamed_dot, &graph_view).expect("stream dot");
    assert_eq!(
        String::from_utf8(streamed_dot).expect("utf8"),
        aplan_output::dot_of_view(&graph_view),
        "streamed dot diverged from buffered renderer"
    );

    let mut bad_roadmap = roadmap;
    bad_roadmap.cross_cutting_effort_days = f64::NAN;
    let mut preflight_buf = Vec::new();
    let err = aplan_output::write_roadmap_json(&mut preflight_buf, &bad_roadmap)
        .expect_err("non-finite roadmap effort must fail preflight");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        preflight_buf.is_empty(),
        "preflight failure must not publish partial bytes; wrote {} bytes",
        preflight_buf.len()
    );
}

#[cfg(target_os = "linux")]
fn process_alive(pid: u32) -> bool {
    let path = format!("/proc/{pid}/stat");
    let Ok(stat) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Some((_, fields)) = stat.rsplit_once(") ") else {
        return false;
    };
    let state = fields.split_whitespace().next().unwrap_or("");
    state != "Z" && state != "X"
}

fn assert_cli_broken_pipe_stops_early(root: &Path) {
    let full = common::run(root, &["--color", "never", "tasks", "--format", "json"]);
    assert_eq!(full.code, 0, "full tasks json failed: {}", full.stderr);
    let full_bytes = full.stdout.len() as u64;
    assert!(
        full_bytes > BOUNDED_RESPONSE_THRESHOLD_BYTES as u64,
        "expected > 256 KiB payload, got {full_bytes}"
    );

    for args in [
        &["--timings", "--color", "never", "tasks", "--format", "json"][..],
        &["--timings", "--color", "never", "tasks"][..],
        &[
            "--timings",
            "--color",
            "never",
            "roadmap",
            "--format",
            "json",
        ][..],
        &[
            "--timings",
            "--color",
            "never",
            "roadmap",
            "--format",
            "markdown",
        ][..],
        &["--timings", "--color", "never", "graph", "--format", "dot"][..],
        &["--timings", "--color", "never", "ready", "--format", "json"][..],
    ] {
        let started = Instant::now();
        let mut child = common::command(root, args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn aplan for broken-pipe test");
        let mut stdout = child.stdout.take().expect("stdout");
        let mut stderr = child.stderr.take().expect("stderr");

        let mut prefix = [0u8; 64];
        let n = stdout.read(&mut prefix).expect("read prefix");
        assert!(n > 0, "expected initial stdout bytes for {args:?}");
        drop(stdout);

        let mut stderr_text = String::new();
        stderr
            .read_to_string(&mut stderr_text)
            .expect("read stderr");
        let status = child.wait().expect("wait child");
        let elapsed = started.elapsed();

        assert!(
            status.success(),
            "early pipe closure must exit 0 for {args:?}, got {status}; stderr:\n{stderr_text}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "early pipe closure took too long ({elapsed:?}) for {args:?}"
        );
        assert!(
            !stderr_text.contains("panicked") && !stderr_text.contains("Broken pipe"),
            "unexpected panic or raw broken-pipe error for {args:?}:\n{stderr_text}"
        );
        if args.contains(&"tasks") && args.contains(&"json") {
            let timings = parse_timings(&stderr_text);
            let emitted = timing_u64(&timings, "output_bytes");
            assert!(
                emitted < full_bytes,
                "expected early stop below full {full_bytes} bytes on BrokenPipe, emitted {emitted}"
            );
        }
    }
}

fn assert_mcp_oversized_refusal_and_pagination() {
    let oversized_dir = tmp("mcp-oversized");
    let oversized_tasks = oversized_dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&oversized_tasks).expect("mkdir");
    let filler = "a".repeat(2_000);
    for i in 1..=320 {
        std::fs::write(
            oversized_tasks.join(format!("B-{i:03}.md")),
            format!(
                "---\nid: B-{i:03}\ntitle: Oversized task {i} {filler}\nphase: 1\n\
                 status: todo\neffort_days: 1\n---\n\nbody\n"
            ),
        )
        .expect("write oversized task");
    }
    let replies = mcp_talk(
        &oversized_dir.path().join("aplan"),
        &[
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "oversized-test", "version": "1"}
                }
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "aplan_ready", "arguments": {}}
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "resources/read",
                "params": {"uri": "aplan://collections/ready?limit=10"}
            }),
        ],
    );
    let tool_reply = replies.iter().find(|r| r["id"] == 2).expect("tool reply");
    assert_eq!(
        tool_reply["result"]["isError"], true,
        "oversized tool must refuse: {tool_reply}"
    );
    let refused_text = tool_reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert!(
        refused_text.contains("exceeds") && refused_text.contains("aplan://collections/ready"),
        "refusal should direct caller to bounded resource: {refused_text}"
    );
    let page_reply = replies.iter().find(|r| r["id"] == 3).expect("page reply");
    let page_text = page_reply["result"]["contents"][0]["text"]
        .as_str()
        .expect("page text");
    assert!(
        page_text.len() <= MAX_RESOURCE_BYTES,
        "paginated MCP resource exceeded cap: {} bytes",
        page_text.len()
    );
    let page_json: serde_json::Value = serde_json::from_str(page_text).expect("page json");
    assert_eq!(page_json["items"].as_array().expect("items").len(), 10);
    assert!(
        page_json["next_cursor"].as_str().is_some(),
        "expected continuation cursor on paginated collection: {page_json}"
    );
}

#[cfg(target_os = "linux")]
fn make_blocking_mcp_fixture() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt as _;

    let block_dir = tmp("mcp-orphan-guard");
    let broot = block_dir.path();
    std::fs::create_dir_all(broot.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(broot.join("src")).expect("mkdir");
    std::fs::write(broot.join("src/lib.rs"), "pub fn p() {}\n").expect("src");
    let fifo_status = std::process::Command::new("mkfifo")
        .arg(broot.join("provider.block"))
        .status()
        .expect("mkfifo");
    assert!(fifo_status.success(), "mkfifo failed");
    std::fs::write(
        broot.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: Blocking verify task\nphase: 1\nstatus: done\n\
         effort_days: 1\npaths:\n- src/lib.rs\nevidence:\n- kind: test\n  ref: suite::p\n\
         ---\n\nbody\n",
    )
    .expect("task");
    let runner = broot.join("list.sh");
    std::fs::write(
        &runner,
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$$\" > provider.pid\n\
         /bin/cat provider.block &\nprintf '%s\\n' \"$!\" > provider.descendant.pid\n\
         : > provider.started\nwait\n",
    )
    .expect("runner");
    let mut perms = runner.metadata().expect("meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&runner, perms).expect("chmod");
    std::fs::write(
        broot.join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [./list.sh]\n  run: ['true', '{test}']\n",
    )
    .expect("cfg");
    common::git(broot, &["init", "-q", "."]);
    common::git(broot, &["config", "user.email", "test@example.com"]);
    common::git(broot, &["config", "user.name", "test"]);
    common::git(broot, &["add", "-A"]);
    common::git(broot, &["commit", "-q", "-m", "blocking mcp fixture"]);
    block_dir
}

#[cfg(target_os = "linux")]
fn wait_for_blocking_provider_pids(broot: &Path) -> (u32, u32) {
    let started_marker = broot.join("provider.started");
    let wait_deadline = Instant::now() + Duration::from_secs(10);
    while !started_marker.exists() && Instant::now() < wait_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        started_marker.exists(),
        "blocking MCP provider never started"
    );
    let provider_pid: u32 = std::fs::read_to_string(broot.join("provider.pid"))
        .expect("provider.pid")
        .trim()
        .parse()
        .expect("provider pid");
    let descendant_pid: u32 = std::fs::read_to_string(broot.join("provider.descendant.pid"))
        .expect("provider.descendant.pid")
        .trim()
        .parse()
        .expect("descendant pid");
    (provider_pid, descendant_pid)
}

#[cfg(target_os = "linux")]
fn assert_pids_reaped(provider_pid: u32, descendant_pid: u32, context: &str) {
    let reap_deadline = Instant::now() + Duration::from_secs(5);
    while (process_alive(provider_pid) || process_alive(descendant_pid))
        && Instant::now() < reap_deadline
    {
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(
        !process_alive(provider_pid),
        "MCP provider {provider_pid} remained alive after {context}"
    );
    assert!(
        !process_alive(descendant_pid),
        "MCP provider descendant {descendant_pid} remained alive after {context}"
    );
}

#[cfg(target_os = "linux")]
fn assert_interrupted_mcp_kills_blocking_provider() {
    let block_dir = make_blocking_mcp_fixture();
    let broot = block_dir.path();

    let mut child = common::command(broot, &["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn blocking mcp");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");

    for msg in [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "disconnect-test", "version": "1"}
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "aplan_verify", "arguments": {}}
        }),
    ] {
        writeln!(stdin, "{msg}").expect("write mcp msg");
    }
    stdin.flush().expect("flush");

    let (provider_pid, descendant_pid) = wait_for_blocking_provider_pids(broot);
    drop(stdin);
    drop(stdout);

    let disconnect_started = Instant::now();
    let exit_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(
            Instant::now() < exit_deadline,
            "interrupted `aplan mcp` hung instead of stopping its blocking provider"
        );
        std::thread::sleep(Duration::from_millis(15));
    };
    let mut stderr_buf = String::new();
    stderr.read_to_string(&mut stderr_buf).ok();
    assert!(
        status.success(),
        "interrupted `aplan mcp` should exit cleanly, got {status} after {:?}; stderr:\n{stderr_buf}",
        disconnect_started.elapsed()
    );
    assert!(
        !stderr_buf.contains("panicked"),
        "interrupted `aplan mcp` panicked:\n{stderr_buf}"
    );
    assert_pids_reaped(provider_pid, descendant_pid, "client disconnect");
}

#[cfg(target_os = "linux")]
fn assert_cancelled_mcp_request_kills_blocking_provider_and_keeps_session_alive() {
    use std::io::BufRead as _;

    let block_dir = make_blocking_mcp_fixture();
    let broot = block_dir.path();

    let mut child = common::command(broot, &["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn blocking mcp for cancellation test");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut reader = std::io::BufReader::new(child.stdout.take().expect("stdout"));

    for msg in [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "cancel-test", "version": "1"}
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "aplan_verify", "arguments": {}}
        }),
    ] {
        writeln!(stdin, "{msg}").expect("write mcp msg");
    }
    stdin.flush().expect("flush");

    let mut init_line = String::new();
    reader.read_line(&mut init_line).expect("read init reply");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&init_line).expect("init json")["id"],
        1
    );

    let (provider_pid, descendant_pid) = wait_for_blocking_provider_pids(broot);
    for msg in [
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": 2, "reason": "client stopped waiting"}
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "aplan_identity", "arguments": {}}
        }),
    ] {
        writeln!(stdin, "{msg}").expect("write cancel/follow-up");
    }
    stdin.flush().expect("flush after cancel");

    assert_pids_reaped(provider_pid, descendant_pid, "notifications/cancelled");

    let mut saw_identity = false;
    for _ in 0..4 {
        let mut line = String::new();
        if reader.read_line(&mut line).expect("read reply") == 0 {
            break;
        }
        let reply: serde_json::Value = serde_json::from_str(&line).expect("reply json");
        if reply["id"] == 3 {
            assert!(
                reply.get("error").is_none(),
                "session broke after request cancellation: {reply}"
            );
            assert_eq!(reply["result"]["isError"], false);
            saw_identity = true;
            break;
        }
    }
    assert!(
        saw_identity,
        "follow-up aplan_identity did not succeed on the live session after request cancellation"
    );

    drop(stdin);
    let status = child.wait().expect("wait cancelled mcp child");
    assert!(status.success(), "cancelled MCP child exited {status}");
}

/// Proof for `read.streams_large_outputs_with_bounded_memory`: closing a CLI
/// consumer pipe early (`BrokenPipe` / pager quit) stops production promptly
/// with exit 0 and zero panics; MCP refuses oversized tool results cleanly
/// while paginating bounded resources; and an interrupted MCP client terminates
/// active blocking provider descendants without leaving orphaned workers.
#[test]
fn broken_pipe_and_interrupted_mcp_consumers_stop_cleanly_without_orphans() {
    let dir = make_large_plan(1_200);
    assert_cli_broken_pipe_stops_early(dir.path());

    let eof_child = common::command(
        dir.path(),
        &[
            "mcp",
            "--root",
            &dir.path().join("aplan").display().to_string(),
        ],
    )
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .output()
    .expect("run eof mcp");
    assert!(
        eof_child.status.success(),
        "EOF before MCP handshake should exit 0, stderr:\n{}",
        String::from_utf8_lossy(&eof_child.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&eof_child.stderr).contains("panicked"),
        "EOF before MCP handshake panicked"
    );

    assert_mcp_oversized_refusal_and_pagination();
    #[cfg(target_os = "linux")]
    {
        assert_interrupted_mcp_kills_blocking_provider();
        assert_cancelled_mcp_request_kills_blocking_provider_and_keeps_session_alive();
    }
}
