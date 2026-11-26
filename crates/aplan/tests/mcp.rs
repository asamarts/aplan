//! `aplan mcp`, driven over real stdio as a client would.
//!
//! The MCP surface is unit-tested inside `aplan-mcp`, but nothing exercised the
//! *command*: the runtime it builds, the config it resolves, the transport it
//! serves on. That gap meant `cmd_mcp` could break with CI still green, which is
//! the class of hole this repository does not tolerate elsewhere.
//!
//! This speaks JSON-RPC to the built binary and asserts on the replies, so it
//! covers the wiring end to end rather than the library alone.

use std::io::Write;
use std::process::Stdio;

use crate::common;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

/// A plan with one workable task.
fn plan() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .unwrap();
    let tasks = dir.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join("A-1.md"),
        "---\nid: A-1\ntitle: A real task\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\n## Why\n\nx\n",
    )
    .unwrap();
    dir
}

/// Send `requests` to `aplan mcp` and collect the JSON replies, keyed by id.
fn talk(root: &std::path::Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let root = root.display().to_string();
    spawn_and_talk(
        std::path::Path::new("."),
        &["mcp", "--root", &root],
        requests,
    )
}

/// The same, started *in* `cwd` with whatever arguments a client would pass.
///
/// A real client starts the server in the repository it is working on, so the
/// configuration is discovered from there rather than named -- and discovery is
/// a different code path from the one `--root` takes, resolved earlier and
/// deliberately not bypassable (`A1-068`). Both have to survive a config this
/// build cannot read (`A1-259`, audit).
fn talk_in(cwd: &std::path::Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    spawn_and_talk(cwd, &["mcp"], requests)
}

fn spawn_and_talk(
    cwd: &std::path::Path,
    argv: &[&str],
    requests: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut child = common::command(cwd, argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aplan mcp");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        for r in requests {
            writeln!(stdin, "{r}").expect("write request");
        }
        stdin.flush().ok();
        // Dropping stdin closes the stream, which is how the server learns the
        // client is done and exits rather than blocking this test forever.
    }

    let output = child.wait_with_output().expect("wait for aplan mcp");
    decode_replies(output.status.success(), &output.stdout, &output.stderr)
        .unwrap_or_else(|error| panic!("aplan mcp transport failed: {error}"))
}

/// Decode the complete transport transcript without hiding protocol damage.
///
/// This is deliberately stricter than selecting the replies a test happens to
/// inspect. A server that prints one expected result and then emits malformed
/// JSON or exits unsuccessfully has broken the stdio contract, even though a
/// lossy collector could still find the expected result and report a pass.
fn decode_replies(
    success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Vec<serde_json::Value>, String> {
    let stderr = String::from_utf8_lossy(stderr);
    if !success {
        return Err(format!("server exited unsuccessfully; stderr:\n{stderr}"));
    }
    let stdout = std::str::from_utf8(stdout)
        .map_err(|error| format!("stdout is not UTF-8: {error}; stderr:\n{stderr}"))?;
    stdout
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|error| {
                format!(
                    "stdout line {} is not a JSON-RPC document: {error}: {line:?}; stderr:\n{stderr}",
                    index + 1
                )
            })
        })
        .collect()
}

#[test]
fn mcp_harness_rejects_a_damaged_transport_transcript() {
    let good = br#"{"jsonrpc":"2.0","id":1,"result":{}}
"#;
    assert_eq!(decode_replies(true, good, b"").unwrap().len(), 1);

    let malformed = br#"{"jsonrpc":"2.0","id":1,"result":{}}
not-json
"#;
    let error = decode_replies(true, malformed, b"server detail")
        .expect_err("malformed stdout must not be silently discarded");
    assert!(error.contains("stdout line 2"), "{error}");
    assert!(error.contains("server detail"), "{error}");

    let error = decode_replies(false, good, b"fatal detail")
        .expect_err("an unsuccessful server must not look healthy");
    assert!(error.contains("exited unsuccessfully"), "{error}");
    assert!(error.contains("fatal detail"), "{error}");

    let error = decode_replies(true, b"\xff", b"")
        .expect_err("non-UTF-8 stdout must not be silently discarded");
    assert!(error.contains("not UTF-8"), "{error}");
}

fn initialize() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "aplan-test", "version": "1" }
            }
        }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    ]
}

fn reply(replies: &[serde_json::Value], id: i64) -> &serde_json::Value {
    replies
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no reply with id {id} in {replies:#?}"))
}

fn tool_json(replies: &[serde_json::Value], id: i64) -> serde_json::Value {
    let answer = reply(replies, id);
    assert_ne!(answer["result"]["isError"], true, "{answer:#?}");
    let text = answer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool {id} returned no text: {answer:#?}"));
    serde_json::from_str(text).unwrap_or_else(|error| panic!("tool {id}: {error}: {text}"))
}

#[test]
fn the_server_initializes_and_lists_exactly_its_tools() {
    let dir = plan();
    let mut reqs = initialize();
    reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let replies = talk(dir.path(), &reqs);

    assert_eq!(reply(&replies, 1)["result"]["serverInfo"]["name"], "aplan");

    let tools = reply(&replies, 2)["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        vec![
            "aplan_identity",
            "aplan_ready",
            "aplan_why",
            "aplan_check",
            "aplan_verify",
            "aplan_add",
            "aplan_show",
            "aplan_dep",
            "aplan_close",
            "aplan_claim",
            "aplan_release",
            "aplan_start",
            "aplan_stop",
            "aplan_review",
        ],
        "the surface an agent sees changed. This list is deliberately short: a tool \
         earns a place by being on the loop an agent must complete, which is why \
         `verify` is here (an agent that cannot resolve the evidence it just cited \
         hands CI a plan that fails on it, `A1-102`) and `graph`, `summary` and \
         `critical-path` are not. `dep` earned its place because an agent that can \
         create tasks could say which comes first only at creation, and afterwards \
         not at all (`A1-124`)"
    );
}

/// **The whole agent-facing surface, pinned byte for byte** (`A1-331`).
///
/// The test above asserts the nine tool *names*. That is the half a reader
/// checks; the half an agent breaks on is the `inputSchema` beside each one,
/// and nothing compared it. `A1-331` moved this server across an SDK major
/// version on the strength of "the existing MCP tests pass", and the existing
/// MCP tests would have passed with every schema silently reshaped.
///
/// Measured rather than assumed, which is the reason this exists: the `rmcp`
/// 2.x and 3.x servers were run side by side and every reply, including the
/// refusal for an unknown tool, came back byte-identical. That comparison was
/// done by hand once. This is it committed, so the next SDK bump answers the
/// same question without anybody remembering to ask.
///
/// Same bargain as `docs/rule-catalogue.md`: changing what an agent is told
/// becomes a reviewable diff rather than something a consumer finds in their
/// own logs. Regenerate with
/// `APLAN_MCP_SURFACE=overwrite cargo test -p aplan --test integration
/// mcp::the_tool_surface_is_committed_and_current`.
#[test]
fn the_tool_surface_is_committed_and_current() {
    let dir = plan();
    let mut reqs = initialize();
    reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let replies = talk(dir.path(), &reqs);
    let tools = &reply(&replies, 2)["result"]["tools"];

    let generated = format!(
        "{}\n",
        serde_json::to_string_pretty(tools).expect("the tool list is JSON")
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mcp-surface.json");

    if std::env::var("APLAN_MCP_SURFACE").as_deref() == Ok("overwrite") {
        std::fs::write(&path, &generated).expect("writing the surface fixture");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}. Regenerate with `APLAN_MCP_SURFACE=overwrite cargo test \
             -p aplan --test integration mcp::the_tool_surface_is_committed_and_current`",
            path.display()
        )
    });
    assert_eq!(
        committed, generated,
        "the surface an agent sees has changed. If that is deliberate, regenerate with \
        `APLAN_MCP_SURFACE=overwrite cargo test -p aplan --test integration \
         mcp::the_tool_surface_is_committed_and_current` and let the diff be \
         reviewed; if it is not, an SDK bump has reshaped the contract underneath you"
    );
}

#[test]
fn a_tool_call_returns_real_plan_data() {
    let dir = plan();
    let mut reqs = initialize();
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_ready", "arguments": {} }
    }));
    let replies = talk(dir.path(), &reqs);

    let text = reply(&replies, 2)["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(text).expect("tool output is JSON");
    assert_eq!(value["ready"][0]["id"], "A-1");
}

/// Tool/function parity cannot prove that the JSON-RPC dispatcher routes a
/// published tool to that function with the arguments its schema promises.
/// Exercise the two mutating dispatch arms that previously had no wire-level
/// caller, and observe their durable post-state from outside the server.
#[test]
fn release_and_dependency_mutations_execute_over_the_wire() {
    let dir = plan();
    std::fs::write(
        dir.path().join("tasks/A-2.md"),
        "---\nid: A-2\ntitle: A dependent task\nphase: 1\nstatus: todo\n\
         effort_days: 1\nblocked_by: [A-1]\n---\n\n## Why\n\nWaits for A-1.\n",
    )
    .expect("write dependent task");
    git(dir.path(), &["init", "--quiet"]);

    let mut claim_requests = initialize();
    claim_requests.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {
            "name": "aplan_claim",
            "arguments": { "id": "A-1", "as": "ada" }
        }
    }));
    let claimed = tool_json(&talk(dir.path(), &claim_requests), 2);
    assert_eq!(claimed["id"], "A-1");
    assert_eq!(claimed["held_by"], "ada");
    assert_eq!(claimed["claim"]["holder"], "ada");

    // Requests in one MCP session may run concurrently. A second session is an
    // execution boundary and proves release observes the completed claim rather
    // than accidentally passing because two mutations happened to be ordered.
    let mut release_requests = initialize();
    release_requests.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {
            "name": "aplan_release",
            "arguments": { "id": "A-1" }
        }
    }));
    let released = tool_json(&talk(dir.path(), &release_requests), 3);
    assert_eq!(released["changed"], true);
    assert_eq!(released["was_held_by"], "ada");
    let plan = aplan_core::load::load_plan(&dir.path().join("tasks"));
    let snapshot = aplan_app::claims::snapshot(&dir.path().join("tasks"), &plan)
        .expect("read released claim store");
    assert!(snapshot.claims.is_empty(), "release left a claim behind");

    let mut dependency_requests = initialize();
    dependency_requests.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {
            "name": "aplan_dep",
            "arguments": { "id": "A-2", "after": "A-1", "add": false }
        }
    }));
    let dependency = tool_json(&talk(dir.path(), &dependency_requests), 4);
    assert_eq!(dependency["changed"], true);
    assert_eq!(dependency["id"], "A-2");
    assert_eq!(dependency["after"], "A-1");
    let task = std::fs::read_to_string(dir.path().join("tasks/A-2.md"))
        .expect("read dependency post-state");
    assert!(
        !task.contains("blocked_by"),
        "dependency removal did not reach the task file:\n{task}"
    );
}

/// Proof for `mcp.launch_context_honours_trace_selection`: the fallback used
/// by a long-lived server must not take the precedence of an explicit CLI flag
/// and thereby make the repository's two trace settings unreachable.
#[test]
fn mcp_launch_context_honours_configured_trace_selection() {
    for (config, create_document) in [
        ("root: tasks\ntraces_root: docs\n", true),
        ("root: tasks\nno_traces: true\n", false),
    ] {
        let dir = tempfile::Builder::new()
            .prefix("aplan-mcp-launch-traces-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("repository");
        std::fs::create_dir(dir.path().join("tasks")).expect("plan");
        std::fs::write(dir.path().join(".aplan.yml"), config).expect("config");
        std::fs::write(
            dir.path().join("tasks/A-1.md"),
            "---\nid: A-1\ntitle: traced\nphase: 1\nstatus: todo\neffort_days: 1\n\
             traces: [DESIGN.md#fact]\n---\n",
        )
        .expect("task");
        if create_document {
            std::fs::create_dir(dir.path().join("docs")).expect("documents");
            std::fs::write(dir.path().join("docs/DESIGN.md"), "# Fact\n\nbody\n")
                .expect("document");
        }

        let mut requests = initialize();
        requests.push(serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "aplan_check", "arguments": {} }
        }));
        let checked = tool_json(&talk_in(dir.path(), &requests), 2);
        assert!(
            !checked.to_string().contains("APLAN008"),
            "launch configuration was overridden by the MCP fallback: {checked:#}"
        );
    }
}

/// One running server can select a second repository without retaining any of
/// the launch repository's context. This is the transport proof for
/// `mcp.explicit_root_resolves_one_repository_context` and, through its
/// historical trace assertion, `evidence.stale_premise_uses_configured_roots`
/// (`A1-604`).
#[test]
fn an_explicit_root_selects_an_independent_repository_over_stdio() {
    let launch = tempfile::Builder::new()
        .prefix("aplan-mcp-wire-launch-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("launch repository");
    let selected = tempfile::Builder::new()
        .prefix("aplan-mcp-wire-selected-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("selected repository");
    for repository in [&launch, &selected] {
        git(repository.path(), &["init", "--quiet"]);
    }
    std::fs::create_dir(launch.path().join("aplan")).expect("launch plan");
    std::fs::write(launch.path().join(".aplan.yml"), "root: aplan\n").expect("launch config");
    std::fs::write(
        launch.path().join("aplan/A-1.md"),
        "---\nid: A-1\ntitle: launch\nphase: 1\nstatus: todo\neffort_days: 1\n---\n",
    )
    .expect("launch task");

    let selected_plan = selected.path().join("plans/tasks");
    std::fs::create_dir_all(&selected_plan).expect("selected plan");
    std::fs::create_dir(selected.path().join("docs")).expect("selected docs");
    std::fs::write(
        selected.path().join(".aplan.yml"),
        "root: plans/tasks\ntraces_root: docs\ndeny_warnings: true\n",
    )
    .expect("selected config");
    std::fs::write(selected.path().join("docs/DESIGN.md"), "# Fact\n\nbody\n")
        .expect("selected trace");
    std::fs::write(
        selected_plan.join("B-1.md"),
        "---\nid: B-1\ntitle: selected\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/**]\ntraces: [DESIGN.md#fact]\n---\n",
    )
    .expect("selected task");
    std::fs::write(
        selected_plan.join("B-2.md"),
        "---\nid: B-2\ntitle: warning\nphase: 1\nstatus: todo\n---\n",
    )
    .expect("warning task");
    git(selected.path(), &["config", "user.email", "t@example.com"]);
    git(selected.path(), &["config", "user.name", "t"]);
    git(selected.path(), &["add", "-A"]);
    git(selected.path(), &["commit", "-q", "-m", "initial"]);
    std::fs::write(
        selected.path().join("docs/DESIGN.md"),
        "# Fact\n\nchanged after task\n",
    )
    .expect("changed selected trace");
    git(selected.path(), &["add", "docs/DESIGN.md"]);
    git(selected.path(), &["commit", "-q", "-m", "move premise"]);

    let mut requests = initialize();
    requests.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_check", "arguments": {
            "root": selected.path()
        }}
    }));
    requests.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "aplan_show", "arguments": {
            "root": &selected_plan, "id": "B-1"
        }}
    }));
    let replies = talk_in(launch.path(), &requests);
    let checked = tool_json(&replies, 2);
    assert_eq!(checked["context"]["selection"], "repository");
    assert_eq!(checked["context"]["plan_root"], "plans/tasks");
    assert_eq!(checked["result"]["tasks_checked"], 2);
    assert_eq!(checked["result"]["warnings"], 1);
    assert_eq!(checked["result"]["denied"], true);
    assert!(
        !checked.to_string().contains("APLAN008"),
        "the selected trace was resolved in the wrong repository: {checked}"
    );
    let cli = common::output(selected.path(), &["check", "--format", "json"]);
    let cli_report: serde_json::Value = serde_json::from_slice(&cli.stdout)
        .unwrap_or_else(|error| panic!("CLI check did not return JSON: {error}: {cli:#?}"));
    for field in ["tasks_checked", "errors", "warnings"] {
        assert_eq!(
            checked["result"][field], cli_report[field],
            "CLI and MCP disagree about `{field}`"
        );
    }
    assert_eq!(
        checked["result"]["denied"],
        !cli.status.success(),
        "CLI exit and MCP repository verdict disagree"
    );
    let shown = tool_json(&replies, 3);
    assert_eq!(shown["context"]["selection"], "plan");
    assert_eq!(shown["result"]["id"], "B-1");
    assert!(
        shown["result"]["findings"]
            .as_array()
            .is_some_and(|findings| findings.iter().any(|finding| finding["code"] == "APLAN045")),
        "aplan_show did not use the selected repository's configured trace frame: {shown:#}"
    );

    // A second session avoids assuming JSON-RPC request completion order. The
    // guarded write must touch B and leave A byte-identical.
    let launch_before =
        std::fs::read_to_string(launch.path().join("aplan/A-1.md")).expect("launch before");
    let mut mutation = initialize();
    mutation.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "aplan_start", "arguments": {
            "root": selected.path(), "id": "B-1"
        }}
    }));
    let replies = talk_in(launch.path(), &mutation);
    let started = tool_json(&replies, 4);
    assert_eq!(started["result"]["status"], "in_progress");
    assert!(
        std::fs::read_to_string(selected_plan.join("B-1.md"))
            .expect("selected after")
            .contains("status: in_progress")
    );
    assert_eq!(
        std::fs::read_to_string(launch.path().join("aplan/A-1.md")).expect("launch after"),
        launch_before
    );
}

#[test]
fn close_over_the_wire_refuses_a_self_assessed_score() {
    // The guarantee this whole surface exists for, checked where an agent
    // actually meets it. `mcp-shrimp-task-manager`'s `verify_task` would accept
    // this call and mark the task complete.
    let dir = plan();
    let mut reqs = initialize();
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_close", "arguments": { "id": "A-1", "score": 99 } }
    }));
    let replies = talk(dir.path(), &reqs);

    let result = &reply(&replies, 2)["result"];
    assert_eq!(result["isError"], true, "a scored close must be refused");
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("evidence"),
        "the refusal must say why: {text}"
    );

    // And the task on disk is untouched: a refusal writes nothing.
    let task = std::fs::read_to_string(dir.path().join("tasks/A-1.md")).unwrap();
    assert!(task.contains("status: todo"), "{task}");
}

#[test]
fn close_over_the_wire_succeeds_with_real_evidence() {
    let dir = plan();
    let mut reqs = initialize();
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {
            "name": "aplan_close",
            "arguments": { "id": "A-1", "evidence": ["commit:abc1234"] }
        }
    }));
    let replies = talk(dir.path(), &reqs);

    let result = &reply(&replies, 2)["result"];
    assert_ne!(result["isError"], true, "{result:#?}");

    let task = std::fs::read_to_string(dir.path().join("tasks/A-1.md")).unwrap();
    assert!(task.contains("status: done"), "{task}");
    assert!(task.contains("ref: abc1234"), "{task}");
}

#[test]
fn add_creates_a_task_in_its_cut_and_refuses_arguments_it_cannot_use() {
    // `A1-103`. The host repository tells its agents "before you write code, find
    // or write its task", and step one is `add --cut`, so this is the half of the
    // loop that was missing. The refusal matters as much as the creation: an
    // agent that sends the wrong shape must be told, not half-obeyed.
    let dir = plan();
    std::fs::write(
        dir.path().join("tasks/M-s0.md"),
        "---\nid: M-s0\ntitle: Cut 0\nphase: 0\nstatus: todo\n\
         tags: [milestone, s0]\nblocked_by: [A-1]\n---\n\nbody\n",
    )
    .expect("milestone");

    let mut reqs = initialize();
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_add", "arguments": {
            "title": "Wire the gate", "cut": "M-s0", "effort_days": 2
        }},
    }));
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "aplan_add", "arguments": {
            "title": "Wrong shape", "effort_days": "two"
        }},
    }));
    let replies = talk(dir.path(), &reqs);

    let created = &reply(&replies, 2)["result"];
    assert_eq!(created["isError"], false, "{created}");
    let text = created["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("\"cut\""),
        "the payload names the cut: {text}"
    );

    let milestone = std::fs::read_to_string(dir.path().join("tasks/M-s0.md")).expect("read");
    assert!(
        milestone.contains("A-2"),
        "the membership edit is the reason `add` exists: {milestone}"
    );

    // An effort that is not a number is a schema violation, and the server says
    // so rather than dropping the field and writing a task without an estimate.
    let refused = &reply(&replies, 3)["result"];
    assert_eq!(refused["isError"], true, "{refused}");
    let text = refused["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("bad arguments"), "{text}");
}

/// Proves `mcp.add_accepts_explicit_identity`: the fields introduced for new-cut
/// creation are also a deliberate ordinary-authoring contract, not inputs that
/// happen to be ignored when `members` is absent.
#[test]
fn mcp_add_accepts_explicit_identity_for_an_ordinary_task() {
    let dir = plan();
    let mut reqs = initialize();
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_add", "arguments": {
            "title": "Explicitly identified work",
            "id": "A-42",
            "phase": 7,
            "effort_days": 1
        }},
    }));

    let replies = talk(dir.path(), &reqs);
    let created = &reply(&replies, 2)["result"];
    assert_eq!(created["isError"], false, "{created}");
    let payload: serde_json::Value = serde_json::from_str(
        created["content"][0]["text"]
            .as_str()
            .expect("tool result text"),
    )
    .expect("versioned add payload");
    assert_eq!(payload["created"], "A-42", "{payload}");

    let task = std::fs::read_to_string(dir.path().join("tasks/A-42.md"))
        .expect("the explicitly named task was written");
    assert!(task.contains("id: A-42"), "{task}");
    assert!(task.contains("phase: 7"), "{task}");
    assert!(
        !dir.path().join("tasks/A-2.md").exists(),
        "the tool minted a replacement instead of using the requested identity"
    );
}

#[test]
fn mcp_add_schema_and_runtime_enforce_the_member_identity_contract() {
    let dir = plan();
    let mut reqs = initialize();
    reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    // A client may bypass schema-side validation, so the implementation must
    // still return the precise domain refusal and leave the plan untouched.
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "aplan_add", "arguments": {
            "title": "Missing identity",
            "members": ["A-1"],
            "tags": ["milestone"]
        }},
    }));

    let replies = talk(dir.path(), &reqs);
    let tools = reply(&replies, 2)["result"]["tools"]
        .as_array()
        .expect("tool list");
    let add = tools
        .iter()
        .find(|tool| tool["name"] == "aplan_add")
        .expect("aplan_add schema");
    let conditional = &add["inputSchema"]["allOf"][0];
    assert_eq!(conditional["if"]["properties"]["members"]["minItems"], 1);
    assert_eq!(
        conditional["then"]["required"],
        serde_json::json!(["id", "phase", "tags"])
    );
    assert_eq!(
        conditional["then"]["properties"]["tags"]["contains"]["const"],
        "milestone"
    );
    assert_eq!(
        conditional["then"]["not"]["required"],
        serde_json::json!(["cut"])
    );

    let refused = &reply(&replies, 3)["result"];
    assert_eq!(refused["isError"], true, "{refused}");
    let text = refused["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("supply both `id` and `phase`"), "{text}");
    assert!(!dir.path().join("tasks/A-2.md").exists());
}

/// The agent surface applies A1-369's unconditional refusal too, rather than
/// writing the invalid milestone and returning APLAN021 as a note (`A1-407`).
/// Proves `mcp.add_refuses_milestone_work_fields` over the real protocol.
#[test]
fn mcp_add_refuses_milestone_work_fields_before_writing() {
    for (field, value, advisory) in [
        ("effort_days", serde_json::json!(0), false),
        ("paths", serde_json::json!(["src/**"]), true),
    ] {
        let dir = plan();
        if advisory {
            std::fs::write(dir.path().join(".aplan.yml"), "advisory: [APLAN021]\n")
                .expect("config");
        }
        let mut arguments = serde_json::json!({
            "title": "Bad milestone",
            "tags": ["milestone"],
            "blocked_by": ["A-1"]
        });
        arguments[field] = value;
        let mut reqs = initialize();
        reqs.push(serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "aplan_add", "arguments": arguments },
        }));

        let replies = talk(dir.path(), &reqs);
        let refused = &reply(&replies, 2)["result"];
        assert_eq!(refused["isError"], true, "{field}: {refused}");
        let text = refused["content"][0]["text"].as_str().unwrap_or_default();
        assert!(
            text.contains("APLAN021") && text.contains("cannot add `A-2`"),
            "{field}: {text}"
        );
        assert!(
            !dir.path().join("tasks/A-2.md").exists(),
            "{field}: the refusal wrote the invalid milestone"
        );
    }
}

/// The shared add refusal carries its subject: improving an existing malformed
/// cut remains a lenient warning, including over MCP (`A1-406`, `A1-407`).
/// Proves `add.milestone_work_refusal_is_new_input_only` on the agent surface.
#[test]
fn mcp_add_can_improve_an_existing_malformed_milestone() {
    let dir = plan();
    let cut_path = dir.path().join("tasks/M-bad.md");
    std::fs::write(
        &cut_path,
        "---\nid: M-bad\ntitle: Existing bad cut\nphase: 1\nstatus: todo\neffort_days: 1\n\
         tags: [milestone]\n---\n",
    )
    .expect("milestone");
    let mut reqs = initialize();
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_add", "arguments": {
            "title": "A new member", "cut": "M-bad", "effort_days": 1
        }},
    }));

    let replies = talk(dir.path(), &reqs);
    let created = &reply(&replies, 2)["result"];
    assert_eq!(created["isError"], false, "{created}");
    let text = created["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("APLAN021") && text.contains("carries `effort_days`"),
        "the remaining warning was lost: {text}"
    );
    assert!(
        !text.contains("has no members"),
        "the fixed clause survived: {text}"
    );
    assert!(
        dir.path().join("tasks/A-2.md").exists(),
        "the successful tool call did not write its task"
    );
    let cut = std::fs::read_to_string(cut_path).expect("read cut");
    assert!(
        cut.contains("A-2"),
        "the cut did not gain its member: {cut}"
    );
}

/// **The server names its own build, over the wire, and it is the serving
/// binary's** (`A1-259`).
///
/// `p0000014` pointed an agent at a configured MCP server whose binary predated
/// its checkout, and spent a pass diagnosing a path-resolution defect that the
/// checkout had already fixed. The CLI could say what it was; the server could
/// not, and asking an installed CLI does not answer for a process that may be a
/// different executable entirely.
///
/// Compared through the transport rather than through the shared function,
/// because a shared function is exactly what would still agree if the wiring
/// handed the tool a document computed somewhere else. What is asserted is that
/// the bytes an agent receives are the bytes `aplan facts --identity` prints.
#[test]
fn the_server_reports_the_same_identity_the_command_prints() {
    let dir = plan();
    let mut reqs = initialize();
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_identity", "arguments": {} }
    }));
    let replies = talk(dir.path(), &reqs);

    let text = reply(&replies, 2)["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no identity in {:#?}", reply(&replies, 2)));
    let over_the_wire: serde_json::Value =
        serde_json::from_str(text).expect("the tool returns a JSON document");

    let out = common::output(std::path::Path::new("."), &["facts", "--identity"]);
    assert!(out.status.success(), "`facts --identity` failed");
    let from_the_command: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("the command prints JSON");

    assert_eq!(
        over_the_wire, from_the_command,
        "the server and the command disagree about which build this is, which is the one \
         question a mismatch makes urgent"
    );
    assert_eq!(over_the_wire["schema"], "aplan/tool-identity/v1");
    // The facts that this build can always report, rather than only the shape.
    for field in ["version", "facts_sha256", "binary_sha256"] {
        assert!(
            over_the_wire[field].as_str().is_some_and(|s| !s.is_empty()),
            "`{field}` is missing from the identity: {over_the_wire}"
        );
    }
    // The build script may have run without a discoverable checkout even when
    // this test now runs inside one; unknown provenance must stay absent.
    if let Some(source) = over_the_wire.get("source") {
        assert!(
            source.as_str().is_some_and(|s| !s.is_empty()),
            "a present source must name a commit: {over_the_wire}"
        );
    }
}

/// It answers with no plan, no repository and nothing configured.
///
/// The reason to ask is usually that something else did not work, so an identity
/// that needed a working plan would be unavailable exactly when it is wanted
/// (`A1-259`).
#[test]
fn the_identity_answers_without_a_plan_or_a_repository() {
    let empty = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let missing = empty.path().join("no-such-plan");

    let mut reqs = initialize();
    reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_identity", "arguments": {} }
    }));
    let replies = talk(&missing, &reqs);

    let answer = reply(&replies, 2);
    assert!(
        answer["result"]["isError"] != serde_json::json!(true),
        "refused for want of a plan: {answer:#?}"
    );
    let text = answer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no identity in {answer:#?}"));
    assert!(
        text.contains("aplan/tool-identity/v1"),
        "not an identity document: {text}"
    );
}

/// **A config this build cannot read leaves the identity answerable** (`A1-259`,
/// audit).
///
/// The case the tool exists for, and the one it could not serve. An old binary
/// rejects a config key a newer aplan added, and that is how a stale binary
/// usually announces itself -- `p0000014`'s server was a stale binary. Until
/// this, `aplan mcp` exited before serving, so the agent met a dead process in
/// exactly the situation where *which build are you talking to* is the answer.
///
/// The error is not softened. Every other tool returns it, so nothing runs under
/// a policy nobody chose.
#[test]
fn a_config_this_build_cannot_read_still_answers_which_build_it_is() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\na_key_no_build_of_this_tool_knows: true\n",
    )
    .expect("config");

    let mut reqs = initialize();
    for (id, tool) in [(2, "aplan_identity"), (3, "aplan_check")] {
        reqs.push(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": tool, "arguments": {} }
        }));
    }
    // Both ways a client reaches the server, because the configuration is read
    // *twice* on different paths: resolving the root reads it, deliberately, so
    // `--root` cannot bypass a config check (`A1-068`), and the command reads it
    // again for its policy. A client started in the repository takes the first;
    // one passing `--root` takes the second. Mutating either alone left the
    // other green, which is how this pair was found.
    for (how, replies) in [
        ("started in the repository", talk_in(dir.path(), &reqs)),
        (
            "pointed at it with --root",
            talk(&dir.path().join("aplan"), &reqs),
        ),
    ] {
        let identity = reply(&replies, 2);
        let text = identity["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{how}: the build would not identify itself: {identity:#?}"));
        assert!(
            text.contains("aplan/tool-identity/v1"),
            "{how}: not an identity document: {text}"
        );

        // And the refusal is intact, naming the key and pointing at the tool
        // that explains it.
        let refused = reply(&replies, 3);
        let said = serde_json::to_string(&refused["result"]).unwrap_or_default();
        assert!(
            said.contains("a_key_no_build_of_this_tool_knows"),
            "{how}: the config error was swallowed: {said}"
        );
        assert!(
            said.contains("aplan_identity"),
            "{how}: refused without saying what can still be asked: {said}"
        );
    }
}

/// **A per-user file this build cannot read does not corrupt the protocol**
/// (`A1-364`, audit).
///
/// The server talks JSON-RPC on stdout, and `A1-364` made it read a file
/// outside the repository that may be unreadable, unparseable, or hold a key
/// `.aplan.yml` takes. Each of those produces a `note:` -- deliberately, because
/// somebody wrote the file and believes it is in effect -- and a note on the
/// wrong stream would be a line a client cannot parse, in the middle of a
/// session, for a reason unrelated to anything it asked for.
///
/// **This test spawns its own server rather than using `talk`**, because that
/// helper drops lines it cannot parse (`filter_map(..ok())`) and so cannot see
/// the defect this is about: corruption would look identical to silence.
///
/// The agent reaching aplan through this server is the actor `A1-359` and
/// `A1-364` exist for, so "the note goes to stderr" is a property of the
/// feature rather than a detail of the implementation.
#[test]
fn a_bad_per_user_file_leaves_the_protocol_stream_intact() {
    let dir = plan();
    let git = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(
        git.success(),
        "claim coordination needs a Git common directory"
    );
    let task_path = dir.path().join("tasks/A-1.md");
    let task_before = std::fs::read_to_string(&task_path).expect("task before claim");
    let home = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(home.path().join(".config/aplan")).expect("mkdir");
    // A key `.aplan.yml` takes: refused by name, which is the loudest of the
    // three failures and therefore the one most likely to reach stdout.
    std::fs::write(
        home.path().join(".config/aplan/config.yml"),
        "deny_warnings: false\n",
    )
    .expect("write");

    let root = dir.path().display().to_string();
    let mut child = common::command(std::path::Path::new("."), &["mcp", "--root", &root])
        .env("HOME", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APLAN_OWNER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aplan mcp");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        for r in initialize() {
            writeln!(stdin, "{r}").expect("write");
        }
        writeln!(
            stdin,
            "{}",
            serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "aplan_claim", "arguments": { "id": "A-1", "as": "ada" } }
            })
        )
        .expect("write");
    }
    let out = child.wait_with_output().expect("wait");

    // Every line, not the ones that happen to parse.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "the server said nothing at all");
    let parsed_lines: Vec<serde_json::Value> = lines
        .iter()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("a line on the protocol stream is not JSON: {e}\n{line}")
            })
        })
        .collect();
    for parsed in &parsed_lines {
        assert_eq!(
            parsed["jsonrpc"], "2.0",
            "a line on the protocol stream is not JSON-RPC:\n{parsed}"
        );
    }

    // And the note did happen, on the stream that is for notes.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("never a repository's policy"),
        "the unreadable per-user file was not reported at all: {stderr}"
    );

    // The tool still did its job: a per-user file nobody can read is not a
    // reason to refuse a claim that named its own holder. The claim is local
    // coordination visible through the public read surface, while durable task
    // Markdown remains byte-identical (`A1-531`).
    let claim_reply = reply(&parsed_lines, 2);
    assert_ne!(claim_reply["result"]["isError"], true, "{claim_reply:#?}");
    // JSON-RPC requests may execute concurrently, so observe the completed
    // mutation from a new session after the first server has exited rather
    // than assuming request order is an execution barrier.
    let mut show_requests = initialize();
    show_requests.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "aplan_show", "arguments": { "id": "A-1" } }
    }));
    let show_replies = talk(dir.path(), &show_requests);
    let show_reply = reply(&show_replies, 3);
    let show_text = show_reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("show returned no JSON payload: {show_reply:#?}"));
    let shown: serde_json::Value = serde_json::from_str(show_text).expect("show payload");
    assert_eq!(shown["claim"]["holder"], "ada", "{shown:#?}");
    assert!(shown["owner"].is_null(), "{shown:#?}");
    let task_after = std::fs::read_to_string(&task_path).expect("task after claim");
    assert!(
        !task_after.contains("owner:"),
        "the ephemeral claim leaked into durable ownership:\n{task_after}"
    );
    assert_eq!(task_after, task_before, "claim rewrote task Markdown");
}

fn initialize_version(version: &str) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": { "name": "aplan-test", "version": "1" }
            }
        }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    ]
}

fn check_structured_text_parity(result: &serde_json::Value) -> Result<(), String> {
    let structured = result
        .get("structuredContent")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format!("missing object structuredContent: {result}"))?;
    let text = result["content"][0]["text"]
        .as_str()
        .ok_or_else(|| format!("missing compatibility text: {result}"))?;
    let tool = structured
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .ok_or("missing structured tool")?;
    let subject = structured
        .get("subject")
        .and_then(serde_json::Value::as_str)
        .ok_or("missing structured subject")?;
    let denied = structured
        .get("denied")
        .and_then(serde_json::Value::as_bool)
        .ok_or("missing structured denied")?;
    let effect = structured
        .get("effect")
        .and_then(serde_json::Value::as_str)
        .ok_or("missing structured effect")?;
    let expected_verdict = if denied { "denied" } else { "allowed" };
    let expected_prefix =
        format!("{tool}: subject={subject} verdict={expected_verdict} effect={effect}");
    if !text.starts_with(&expected_prefix) {
        return Err(format!(
            "structured/text parity mismatch: expected text starting with `{expected_prefix}`, \
             got `{text}`"
        ));
    }
    if text.contains('\x1b') {
        return Err(format!("raw ESC leaked into compatibility text: {text:?}"));
    }
    Ok(())
}

/// Proof for `mcp.structured_outputs_and_bounded_resources` (`A2-118`):
/// exercises generated input/output schemas, object-enveloped `structuredContent`
/// and concise text parity across all 14 tools, bounded URI-addressed resources
/// and snapshot-bound continuations, malformed/hostile/stale refusals, and the
/// protocol compatibility matrix across `2024-11-05`, `2025-11-25`, and
/// `2026-07-28`.
#[test]
#[allow(clippy::too_many_lines)]
fn structured_outputs_bounded_resources_and_protocol_matrix() {
    // 0. Negative proof: parity check MUST fail if subject, verdict, or effect diverges.
    let valid_sample = serde_json::json!({
        "content": [{ "type": "text", "text": "aplan_check: subject=plan verdict=allowed effect=read_only -- 0 error(s), 0 warning(s)" }],
        "structuredContent": {
            "contract_version": aplan_mcp::MCP_CONTRACT_VERSION,
            "tool": "aplan_check",
            "subject": "plan",
            "denied": false,
            "effect": "read_only",
            "data": {}
        }
    });
    check_structured_text_parity(&valid_sample).expect("valid sample passes");
    let mut wrong_subject = valid_sample.clone();
    wrong_subject["structuredContent"]["subject"] = serde_json::json!("A-99");
    assert!(check_structured_text_parity(&wrong_subject).is_err());
    let mut wrong_verdict = valid_sample.clone();
    wrong_verdict["structuredContent"]["denied"] = serde_json::json!(true);
    assert!(check_structured_text_parity(&wrong_verdict).is_err());
    let mut wrong_effect = valid_sample;
    wrong_effect["structuredContent"]["effect"] = serde_json::json!("mutated");
    assert!(check_structured_text_parity(&wrong_effect).is_err());

    // Build a real Git-backed fixture with 5 ready tasks so collection paging,
    // claims, dep edits, trace reviews, and close all run end-to-end.
    let dir = tempfile::Builder::new()
        .prefix("mcp-v2-matrix-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let tasks_dir = dir.path().join("tasks");
    std::fs::create_dir_all(&tasks_dir).expect("tasks dir");
    std::fs::write(dir.path().join("SPEC.md"), "# Spec\n\n## Rule\n\nFact.\n").expect("spec");
    std::fs::write(dir.path().join("proof.txt"), "ok\n").expect("proof");
    for i in 1..=5 {
        std::fs::write(
            tasks_dir.join(format!("A-{i}.md")),
            format!(
                "---\nid: A-{i}\ntitle: Task {i}\nphase: 1\nstatus: todo\neffort_days: 1\n\
                 paths:\n- proof.txt\ntraces:\n- SPEC.md#rule\n---\n\n## Why\n\nx\n"
            ),
        )
        .expect("write task");
    }
    git(dir.path(), &["init", "--quiet"]);
    git(
        dir.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(dir.path(), &["config", "user.name", "Test"]);
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "--quiet", "-m", "init"]);

    // 1. Handshake-era `2024-11-05` older text-only client: legacy JSON text, no structuredContent, no resultType.
    let mut legacy_reqs = initialize_version("2024-11-05");
    legacy_reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "aplan_ready", "arguments": {} }
    }));
    let legacy_replies = talk(dir.path(), &legacy_reqs);
    let legacy_result = &reply(&legacy_replies, 2)["result"];
    assert!(
        legacy_result.get("structuredContent").is_none(),
        "2024-11-05 client must not receive structuredContent: {legacy_result:#?}"
    );
    assert!(
        legacy_result.get("resultType").is_none(),
        "2024-11-05 client must not receive resultType: {legacy_result:#?}"
    );
    let legacy_parsed: serde_json::Value =
        serde_json::from_str(legacy_result["content"][0]["text"].as_str().expect("text"))
            .expect("legacy JSON text");
    assert_eq!(legacy_parsed["ready"].as_array().unwrap().len(), 5);

    // 2. Handshake-era `2025-11-25` structured client: drive ALL 14 tools, resources, paging,
    //    malformed arguments, unknown resource (-32002), and hostile URI (-32602).
    let mut s_reqs = initialize_version("2025-11-25");
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" }));
    s_reqs.push(
        serde_json::json!({ "jsonrpc": "2.0", "id": 4, "method": "resources/templates/list" }),
    );
    // Read-only tools + malformed refusal + resource reads can share one session safely.
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": { "name": "aplan_identity", "arguments": {} } }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": { "name": "aplan_ready", "arguments": {} } }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 12, "method": "tools/call", "params": { "name": "aplan_why", "arguments": { "id": "A-1" } } }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 13, "method": "tools/call", "params": { "name": "aplan_check", "arguments": {} } }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 14, "method": "tools/call", "params": { "name": "aplan_verify", "arguments": {} } }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 15, "method": "tools/call", "params": { "name": "aplan_show", "arguments": { "id": "A-1" } } }));
    // Malformed arguments (self-assessment `score` field forbidden by schema).
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 24, "method": "tools/call", "params": { "name": "aplan_close", "arguments": { "id": "A-2", "evidence": [], "score": 100 } } }));
    // Resources on 2025-11-25.
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 30, "method": "resources/read", "params": { "uri": "aplan://plan/summary" } }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 31, "method": "resources/read", "params": { "uri": "aplan://tasks/A-2" } }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 32, "method": "resources/read", "params": { "uri": "aplan://tasks/NO-SUCH" } }));
    s_reqs.push(serde_json::json!({ "jsonrpc": "2.0", "id": 33, "method": "resources/read", "params": { "uri": "file:///etc/passwd" } }));

    let s_replies = talk(dir.path(), &s_reqs);
    // Capabilities advertise both tools and resources (and no subscriptions).
    let init_caps = &reply(&s_replies, 1)["result"]["capabilities"];
    assert!(init_caps.get("tools").is_some(), "{init_caps}");
    assert!(init_caps.get("resources").is_some(), "{init_caps}");

    // Every tool exposes generated object inputSchema AND object outputSchema, and no protocol nextCursor.
    let tools_res = &reply(&s_replies, 2)["result"];
    assert!(tools_res.get("nextCursor").is_none());
    let tools_list = tools_res["tools"].as_array().expect("tools array");
    assert_eq!(tools_list.len(), aplan_mcp::TOOLS.len());
    for tool in tools_list {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert_eq!(tool["outputSchema"]["type"], "object", "{tool}");
    }

    // Catalogue resources & templates have no protocol nextCursor.
    let res_list = &reply(&s_replies, 3)["result"];
    assert!(res_list.get("nextCursor").is_none());
    assert_eq!(res_list["resources"].as_array().unwrap().len(), 1);
    let tmpl_list = &reply(&s_replies, 4)["result"];
    assert!(tmpl_list.get("nextCursor").is_none());
    assert_eq!(tmpl_list["resourceTemplates"].as_array().unwrap().len(), 3);

    // Check all 6 read tools for structured/text parity and contract_version.
    for id in 10..=15 {
        let res = &reply(&s_replies, id)["result"];
        assert_ne!(res["isError"], true, "tool call id={id} failed: {res:#?}");
        assert!(
            res.get("resultType").is_none(),
            "2025-11-25 omits resultType"
        );
        check_structured_text_parity(res)
            .unwrap_or_else(|e| panic!("parity failed for call id={id}: {e}"));
        assert_eq!(
            res["structuredContent"]["contract_version"],
            aplan_mcp::MCP_CONTRACT_VERSION
        );
    }
    // `aplan_identity` advertises `mcp_contract_version`.
    assert_eq!(
        reply(&s_replies, 10)["result"]["structuredContent"]["data"]["mcp_contract_version"],
        aplan_mcp::MCP_CONTRACT_VERSION
    );

    // Run all 8 mutating tools sequentially (JSON-RPC requests within one session may execute concurrently).
    let sequential_mutations = [
        (
            "aplan_add",
            serde_json::json!({ "id": "A-6", "title": "Added task", "phase": 1, "effort_days": 1, "paths": ["proof.txt"] }),
        ),
        (
            "aplan_dep",
            serde_json::json!({ "id": "A-6", "after": "A-1", "add": true }),
        ),
        (
            "aplan_claim",
            serde_json::json!({ "id": "A-1", "as": "ada" }),
        ),
        ("aplan_start", serde_json::json!({ "id": "A-1" })),
        ("aplan_stop", serde_json::json!({ "id": "A-1" })),
        ("aplan_release", serde_json::json!({ "id": "A-1" })),
        ("aplan_review", serde_json::json!({ "id": "A-1" })),
        (
            "aplan_close",
            serde_json::json!({ "id": "A-1", "evidence": ["file:proof.txt"] }),
        ),
    ];
    for (tool_name, args) in sequential_mutations {
        let mut reqs = initialize_version("2025-11-25");
        reqs.push(serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": tool_name, "arguments": args }
        }));
        let replies = talk(dir.path(), &reqs);
        let tool_result = &reply(&replies, 2)["result"];
        assert_ne!(
            tool_result["isError"], true,
            "{tool_name} failed: {tool_result:#?}"
        );
        assert!(
            tool_result.get("resultType").is_none(),
            "2025-11-25 omits resultType"
        );
        check_structured_text_parity(tool_result)
            .unwrap_or_else(|e| panic!("parity failed for {tool_name}: {e}"));
        assert_eq!(
            tool_result["structuredContent"]["contract_version"],
            aplan_mcp::MCP_CONTRACT_VERSION
        );
    }
    // Malformed `score` argument is refused cleanly with structured denial & effect=refused.
    let malformed = &reply(&s_replies, 24)["result"];
    assert_eq!(malformed["isError"], true, "{malformed:#?}");
    check_structured_text_parity(malformed).expect("refusal parity");
    assert_eq!(malformed["structuredContent"]["denied"], true);
    assert_eq!(malformed["structuredContent"]["effect"], "refused");

    // Static summary & focused task resource carry inputIdentity and completeness.
    let summary_content = &reply(&s_replies, 30)["result"]["contents"][0];
    assert_eq!(summary_content["mimeType"], "application/json");
    assert_eq!(summary_content["_meta"]["complete"], true);
    assert_eq!(summary_content["_meta"]["cacheScope"], "private");
    assert_eq!(summary_content["_meta"]["ttlMs"], 0);
    let summary_json: serde_json::Value =
        serde_json::from_str(summary_content["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        summary_json["contract_version"],
        aplan_mcp::MCP_CONTRACT_VERSION
    );
    assert!(
        !summary_json["input_identity"]["plan_sha256"]
            .as_str()
            .unwrap()
            .is_empty()
    );

    let detail_content = &reply(&s_replies, 31)["result"]["contents"][0];
    let detail_json: serde_json::Value =
        serde_json::from_str(detail_content["text"].as_str().unwrap()).unwrap();
    assert_eq!(detail_json["task"]["id"], "A-2");
    assert_eq!(detail_json["complete"], true);

    // Unknown task resource on 2025-11-25 returns RESOURCE_NOT_FOUND (-32002); hostile URI returns INVALID_PARAMS (-32602).
    assert_eq!(reply(&s_replies, 32)["error"]["code"], -32002);
    assert_eq!(reply(&s_replies, 33)["error"]["code"], -32602);

    // 3. Multi-page collection walk + stale continuation refusal when plan changes.
    let mut page1_reqs = initialize_version("2025-11-25");
    page1_reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": { "uri": "aplan://collections/ready?limit=2" }
    }));
    let page1_replies = talk(dir.path(), &page1_reqs);
    let page1: serde_json::Value = serde_json::from_str(
        reply(&page1_replies, 2)["result"]["contents"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page1["returned"], 2);
    assert_eq!(page1["complete"], false);
    let next_uri = page1["next_uri"].as_str().expect("next_uri").to_string();

    // Page 2 succeeds before mutation.
    let mut page2_reqs = initialize_version("2025-11-25");
    page2_reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": { "uri": next_uri }
    }));
    let page2_replies = talk(dir.path(), &page2_reqs);
    let page2: serde_json::Value = serde_json::from_str(
        reply(&page2_replies, 2)["result"]["contents"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page2["returned"], 2);

    // Mutate one task file -> reusing `next_uri` MUST fail as stale!
    std::fs::write(
        tasks_dir.join("A-5.md"),
        "---\nid: A-5\ntitle: Task 5 edited\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n- proof.txt\ntraces:\n- SPEC.md#rule\n---\n\n## Why\n\nedited\n",
    )
    .expect("edit A-5");
    let stale_replies = talk(dir.path(), &page2_reqs);
    assert_eq!(reply(&stale_replies, 2)["error"]["code"], -32602);
    assert!(
        reply(&stale_replies, 2)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stale"),
        "{:#?}",
        reply(&stale_replies, 2)
    );

    // 4. Current protocol revision `2026-07-28` (NO `initialize`, inline `_meta`, `discover`,
    //    `resultType: "complete"`, `ttlMs: 0`, `cacheScope: "private"`, unknown resource -> `-32602`).
    let meta_2026 = serde_json::json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let reqs_2026 = vec![
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "server/discover",
            "params": { "_meta": meta_2026 }
        }),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list",
            "params": { "_meta": meta_2026 }
        }),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "aplan_ready", "arguments": {}, "_meta": meta_2026 }
        }),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "resources/read",
            "params": { "uri": "aplan://plan/summary", "_meta": meta_2026 }
        }),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 5, "method": "resources/read",
            "params": { "uri": "aplan://tasks/NO-SUCH", "_meta": meta_2026 }
        }),
        // Missing required _meta on 2026-07-28 session MUST be refused with -32602.
        serde_json::json!({
            "jsonrpc": "2.0", "id": 6, "method": "tools/list",
            "params": {}
        }),
    ];
    let replies_2026 = talk(dir.path(), &reqs_2026);
    let discover_2026 = &reply(&replies_2026, 1)["result"];
    assert_eq!(
        discover_2026["resultType"], "complete",
        "{discover_2026:#?}"
    );
    assert_eq!(
        discover_2026["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "aplan",
        "{discover_2026:#?}"
    );
    assert!(
        discover_2026["supportedVersions"]
            .as_array()
            .is_some_and(|vs| vs.iter().any(|v| v == "2026-07-28")),
        "{discover_2026:#?}"
    );
    assert!(discover_2026["capabilities"].get("tools").is_some());
    assert!(discover_2026["capabilities"].get("resources").is_some());
    assert_eq!(reply(&replies_2026, 2)["result"]["resultType"], "complete");
    assert_eq!(reply(&replies_2026, 2)["result"]["ttlMs"], 0);
    assert_eq!(reply(&replies_2026, 2)["result"]["cacheScope"], "private");

    let call_2026 = &reply(&replies_2026, 3)["result"];
    assert_eq!(call_2026["resultType"], "complete");
    check_structured_text_parity(call_2026).expect("2026-07-28 tool parity");

    let read_2026 = &reply(&replies_2026, 4)["result"];
    assert_eq!(read_2026["resultType"], "complete");
    assert_eq!(read_2026["ttlMs"], 0);
    assert_eq!(read_2026["cacheScope"], "private");

    // 2026-07-28 maps RESOURCE_NOT_FOUND to INVALID_PARAMS (-32602).
    assert_eq!(reply(&replies_2026, 5)["error"]["code"], -32602);
    // Missing per-request _meta refused with INVALID_PARAMS (-32602).
    assert_eq!(reply(&replies_2026, 6)["error"]["code"], -32602);
}

fn git_dated(dir: &std::path::Path, args: &[&str], date: &str) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
#[allow(clippy::too_many_lines)]
fn completed_history_resource_covers_provenance_paging_shallow_caps_staleness_and_hostile_uris() {
    // 1. Empty case: plan with zero completed tasks.
    let empty_dir = plan();
    let mut empty_reqs = initialize_version("2025-11-25");
    empty_reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": { "uri": "aplan://collections/completed" }
    }));
    let empty_replies = talk(empty_dir.path(), &empty_reqs);
    let empty_content = &reply(&empty_replies, 2)["result"]["contents"][0];
    assert_eq!(empty_content["mimeType"], "application/json");
    let empty_doc: serde_json::Value =
        serde_json::from_str(empty_content["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        empty_doc["contract_version"],
        aplan_mcp::MCP_CONTRACT_VERSION
    );
    assert_eq!(empty_doc["complete"], true);
    assert_eq!(empty_doc["truncated_by_size_cap"], false);
    assert_eq!(empty_doc["order"], "closed_at_desc");
    assert_eq!(empty_doc["completed"]["total_matched"], 0);
    assert_eq!(empty_doc["completed"]["items"].as_array().unwrap().len(), 0);
    assert_eq!(empty_doc["completed"]["completeness"], "complete");
    assert!(empty_doc.get("next_uri").is_none());

    // 2. Real Git repository covering recorded, inferred (`history_first_seen`),
    //    `unknown` (`history=never`), filters, ordering, paging, lost-response
    //    repeat reads, stale/tampered/broadening cursors, hostile URIs, and
    //    protocol compatibility (`2024-11-05`, `2025-11-25`, `2026-07-28`).
    let dir = tempfile::Builder::new()
        .prefix("mcp-completed-e2e-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let repo = dir.path();
    let tasks = repo.join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("tasks dir");
    git(repo, &["init", "--quiet"]);
    git(repo, &["config", "user.email", "test@example.invalid"]);
    git(repo, &["config", "user.name", "Test"]);

    // Commit 1: seed legacy task as `todo`, plus a milestone cut `v1.0`.
    std::fs::write(
        tasks.join("M-v1.md"),
        "---\nid: M-v1\ntitle: Cut v1.0\nphase: 1\nstatus: in_progress\ntags:\n- milestone\n- v1.0\n\
         blocked_by:\n- T-INF\n- T-REC-1\n- T-REC-2\n- T-REC-3\n---\n\nbody\n",
    )
    .expect("milestone");
    std::fs::write(
        tasks.join("T-INF.md"),
        "---\nid: T-INF\ntitle: Inferred legacy task\nphase: 1\nstatus: todo\nowner: bob\n\
         effort_days: 1\ntags:\n- history\n---\n\nbody\n",
    )
    .expect("T-INF todo");
    git(repo, &["add", "."]);
    git_dated(
        repo,
        &["commit", "--quiet", "-m", "seed"],
        "2026-10-01T10:00:00Z",
    );

    // Commit 2: legacy task transitions to `done` (inferred from commit 2).
    std::fs::write(
        tasks.join("T-INF.md"),
        "---\nid: T-INF\ntitle: Inferred legacy task\nphase: 1\nstatus: done\nowner: bob\n\
         effort_days: 1\ntags:\n- history\n---\n\nbody\n",
    )
    .expect("T-INF done");
    git(repo, &["add", "."]);
    git_dated(
        repo,
        &["commit", "--quiet", "-m", "finish T-INF"],
        "2026-10-08T12:00:00Z",
    );

    // Commit 3: add 3 recorded done tasks with distinct instants, owners, tags.
    for (id, owner, tag, ts) in [
        ("T-REC-1", "alice", "mcp", "2026-10-15T12:00:00Z"),
        ("T-REC-2", "alice", "history", "2026-10-12T12:00:00Z"),
        ("T-REC-3", "bob", "mcp", "2026-10-05T12:00:00Z"),
    ] {
        std::fs::write(
            tasks.join(format!("{id}.md")),
            format!(
                "---\nid: {id}\ntitle: Recorded {id}\nphase: 1\nstatus: done\nowner: {owner}\n\
                 effort_days: 1\ntags:\n- {tag}\nclosed_at: \"{ts}\"\n\
                 closed_at_source: local\n---\n\nbody\n"
            ),
        )
        .expect("write recorded task");
    }
    git(repo, &["add", "."]);
    git_dated(
        repo,
        &["commit", "--quiet", "-m", "add recorded tasks"],
        "2026-10-16T00:00:00Z",
    );

    let read_in = |version: &str, uri: &str| -> serde_json::Value {
        if version == "2026-07-28" {
            let meta = serde_json::json!({
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
            });
            let replies = talk_in(
                repo,
                &[serde_json::json!({
                    "jsonrpc": "2.0", "id": 2, "method": "resources/read",
                    "params": { "uri": uri, "_meta": meta }
                })],
            );
            reply(&replies, 2).clone()
        } else {
            let mut reqs = initialize_version(version);
            reqs.push(serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "method": "resources/read",
                "params": { "uri": uri }
            }));
            let replies = talk_in(repo, &reqs);
            reply(&replies, 2).clone()
        }
    };

    // Verify full provenance mix (recorded + history_first_seen) and newest-first order.
    let all_reply = read_in("2025-11-25", "aplan://collections/completed?limit=10");
    let all_doc: serde_json::Value =
        serde_json::from_str(all_reply["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(all_doc["complete"], true);
    assert_eq!(all_doc["completed"]["completeness"], "complete");
    assert!(
        all_doc["completed"]["basis_revision"]
            .as_str()
            .unwrap()
            .starts_with("working_tree@")
    );
    let ids: Vec<&str> = all_doc["completed"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["T-REC-1", "T-REC-2", "T-INF", "T-REC-3"]);
    assert_eq!(all_doc["completed"]["items"][0]["source"], "recorded");
    assert_eq!(all_doc["completed"]["items"][0]["recorded_origin"], "local");
    assert_eq!(
        all_doc["completed"]["items"][2]["source"],
        "history_first_seen"
    );
    assert!(
        all_doc["completed"]["items"][2]["commit"]
            .as_str()
            .is_some_and(|c| c.len() == 40)
    );

    // `history=never` yields `unknown` for legacy task and `history_skipped` completeness.
    let never_reply = read_in(
        "2025-11-25",
        "aplan://collections/completed?history=never&limit=10",
    );
    let never_doc: serde_json::Value = serde_json::from_str(
        never_reply["result"]["contents"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(never_doc["completed"]["completeness"], "history_skipped");
    assert_eq!(never_doc["input_identity"]["trust"], "hermetic");
    let never_ids: Vec<&str> = never_doc["completed"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(never_ids, vec!["T-REC-1", "T-REC-2", "T-REC-3", "T-INF"]);
    assert_eq!(never_doc["completed"]["items"][3]["source"], "unknown");
    assert!(never_doc["completed"]["items"][3]["instant"].is_null());

    // Filters (`cut`, `phase`, `owner`, multi-valued `tag`, `since`, `until`)
    // and `oldest=true` ordering work canonically.
    let filtered_reply = read_in(
        "2025-11-25",
        "aplan://collections/completed?cut=v1.0&phase=1&owner=alice&tag=mcp&tag=history&since=2026-10-10&until=2026-10-16&oldest=true&limit=10",
    );
    let filtered_doc: serde_json::Value = serde_json::from_str(
        filtered_reply["result"]["contents"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(filtered_doc["order"], "closed_at_asc");
    let filtered_ids: Vec<&str> = filtered_doc["completed"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(filtered_ids, vec!["T-REC-2", "T-REC-1"]);

    // Verify `resources/templates/list` on `2026-07-28`: catalogue pagination
    // (`nextCursor`) is absent (distinct from domain collection paging), cache
    // hints enforce private zero-freshness, and `completed_tasks_page` is listed.
    let meta_2026 = serde_json::json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let tmpl_2026_replies = talk_in(
        repo,
        &[serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "resources/templates/list",
            "params": { "_meta": meta_2026 }
        })],
    );
    let tmpl_2026 = &reply(&tmpl_2026_replies, 1)["result"];
    assert_eq!(tmpl_2026["resultType"], "complete");
    assert_eq!(tmpl_2026["ttlMs"], 0);
    assert_eq!(tmpl_2026["cacheScope"], "private");
    assert!(tmpl_2026.get("nextCursor").is_none());
    let completed_tmpl = tmpl_2026["resourceTemplates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "completed_tasks_page")
        .expect("completed_tasks_page template");
    assert_eq!(
        completed_tmpl["uriTemplate"],
        "aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}"
    );
    assert_eq!(completed_tmpl["mimeType"], "application/json");

    // Protocol compatibility across ALL FIVE supported protocol revisions
    // (`2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25`, `2026-07-28`):
    // every revision receives bit-for-bit identical machine-readable subject JSON,
    // while `2026-07-28` carries `resultType: "complete"`, `ttlMs: 0`, `cacheScope: "private"`.
    let mut baseline_p1: Option<serde_json::Value> = None;
    for &rev in aplan_mcp::contract::SUPPORTED_PROTOCOL_REVISIONS {
        let resp = read_in(rev, "aplan://collections/completed?limit=2");
        if rev == "2026-07-28" {
            assert_eq!(resp["result"]["resultType"], "complete");
            assert_eq!(resp["result"]["ttlMs"], 0);
            assert_eq!(resp["result"]["cacheScope"], "private");
        } else {
            assert!(
                resp["result"].get("resultType").is_none(),
                "legacy revision {rev} must omit resultType"
            );
        }
        assert_eq!(
            resp["result"]["contents"][0]["mimeType"],
            "application/json"
        );
        assert_eq!(resp["result"]["contents"][0]["_meta"]["ttlMs"], 0);
        assert_eq!(
            resp["result"]["contents"][0]["_meta"]["cacheScope"],
            "private"
        );
        let doc: serde_json::Value =
            serde_json::from_str(resp["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
        if let Some(expected) = &baseline_p1 {
            assert_eq!(
                &doc, expected,
                "protocol revision {rev} diverged on completed resource payload"
            );
        } else {
            baseline_p1 = Some(doc);
        }
    }
    let p1_2025 = baseline_p1.expect("at least one protocol revision");
    assert_eq!(p1_2025["complete"], false);
    let page2_uri = p1_2025["next_uri"].as_str().expect("page2_uri").to_string();

    // Lost-response safe repeat read: repeating the exact `next_uri` returns
    // bit-for-bit identical rows and metadata, and completes the 4-row collection.
    let p2_first = read_in("2025-11-25", &page2_uri);
    let p2_repeat = read_in("2025-11-25", &page2_uri);
    let p2_doc_a: serde_json::Value =
        serde_json::from_str(p2_first["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    let p2_doc_b: serde_json::Value =
        serde_json::from_str(p2_repeat["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(p2_doc_a, p2_doc_b);
    assert_eq!(p2_doc_a["complete"], true);
    let p2_ids: Vec<&str> = p2_doc_a["completed"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(p2_ids, vec!["T-INF", "T-REC-3"]);

    // Tampered and selector-broadening cursors are refused (`-32602`).
    let alice_p1_reply = read_in(
        "2025-11-25",
        "aplan://collections/completed?owner=alice&limit=1",
    );
    let alice_p1: serde_json::Value = serde_json::from_str(
        alice_p1_reply["result"]["contents"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let alice_next = alice_p1["next_uri"].as_str().expect("alice next_uri");
    // Drop `owner=alice` filter while keeping its cursor (selection broadening!).
    let broadened_uri = alice_next.replace("owner=alice&", "");
    assert_eq!(
        read_in("2025-11-25", &broadened_uri)["error"]["code"],
        -32602
    );
    // Widen `limit=1` to `limit=10` while keeping its cursor.
    let widened_uri = alice_next.replace("limit=1&", "limit=10&");
    assert_eq!(read_in("2025-11-25", &widened_uri)["error"]["code"], -32602);
    // Tamper with the cursor's embedded task id.
    let tampered_uri = alice_next.replace("T-REC-1", "T-NO-SUCH");
    assert_eq!(
        read_in("2025-11-25", &tampered_uri)["error"]["code"],
        -32602
    );

    // Hostile and invalid URIs are refused (`-32602`).
    for hostile in [
        "aplan://collections/completed?at=HEAD",
        "aplan://collections/completed?root=/etc",
        "aplan://collections/completed?path=../../etc/passwd",
        "aplan://collections/completed?limit=0",
        "aplan://collections/completed?limit=51",
        "aplan://collections/completed?cut=../secret",
        "aplan://collections/completed?since=2026-10-15&until=2026-10-10",
    ] {
        let err_reply = read_in("2025-11-25", hostile);
        assert_eq!(
            err_reply["error"]["code"], -32602,
            "hostile URI {hostile:?} was not refused with -32602: {err_reply:#?}"
        );
    }

    // 3. Shallow clone case: clone with `--depth 1` so commit 2 is grafted out
    //    (legacy `T-INF` already appears as `done` at the shallow root commit).
    let shallow_parent = tempfile::Builder::new()
        .prefix("mcp-completed-shallow-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("shallow parent");
    let shallow_repo = shallow_parent.path().join("clone");
    let file_url = format!("file://{}", repo.display());
    git(
        shallow_parent.path(),
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            &file_url,
            shallow_repo.to_str().unwrap(),
        ],
    );
    let mut shallow_reqs = initialize_version("2025-11-25");
    shallow_reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": { "uri": "aplan://collections/completed?history=auto&limit=10" }
    }));
    shallow_reqs.push(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "resources/read",
        "params": { "uri": "aplan://collections/completed?history=required&limit=10" }
    }));
    let shallow_replies = talk_in(&shallow_repo, &shallow_reqs);
    let shallow_auto: serde_json::Value = serde_json::from_str(
        reply(&shallow_replies, 2)["result"]["contents"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        shallow_auto["completed"]["completeness"],
        "incomplete_shallow"
    );
    assert!(
        !shallow_auto["completed"]["limitations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let shallow_inf = shallow_auto["completed"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "T-INF")
        .expect("T-INF row");
    assert_eq!(shallow_inf["source"], "unknown");
    assert!(shallow_inf["instant"].is_null());
    // `history=required` refuses shallow incomplete history cleanly with `-32602`.
    assert_eq!(reply(&shallow_replies, 3)["error"]["code"], -32602);

    // 4a. Stale-page refusal via `basis_revision` movement: committing an
    //     unrelated file advances HEAD while task files remain identical ->
    //     reusing page 1's `next_uri` is refused with `-32602` mentioning `stale`.
    std::fs::write(repo.join("NOTE.txt"), "advance head\n").expect("note");
    git(repo, &["add", "NOTE.txt"]);
    git_dated(
        repo,
        &["commit", "--quiet", "-m", "unrelated head move"],
        "2026-10-17T00:00:00Z",
    );
    let head_moved_reply = read_in("2025-11-25", &page2_uri);
    assert_eq!(head_moved_reply["error"]["code"], -32602);
    assert!(
        head_moved_reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stale"),
        "{head_moved_reply:#?}"
    );

    // Re-read page 1 after HEAD moved to get a fresh cursor, then mutate a task
    // file in the working tree -> reusing that fresh cursor fails as stale too.
    let fresh_p1: serde_json::Value =
        serde_json::from_str(
            read_in("2025-11-25", "aplan://collections/completed?limit=2")["result"]["contents"][0]
                ["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    let fresh_page2_uri = fresh_p1["next_uri"].as_str().expect("fresh next_uri");

    // 4b. Stale-page refusal via working-tree plan edit (`input_identity` moved).
    std::fs::write(
        tasks.join("T-REC-3.md"),
        "---\nid: T-REC-3\ntitle: Recorded T-REC-3 edited\nphase: 1\nstatus: done\nowner: bob\n\
         effort_days: 1\ntags:\n- mcp\nclosed_at: \"2026-10-05T12:00:00Z\"\n\
         closed_at_source: local\n---\n\nbody edited\n",
    )
    .expect("mutate T-REC-3");
    let stale_reply = read_in("2025-11-25", fresh_page2_uri);
    assert_eq!(stale_reply["error"]["code"], -32602);
    assert!(
        stale_reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stale"),
        "{stale_reply:#?}"
    );

    // 4c. Unloadable task file refuses fail-closed (parity with CLI `aplan completed`)
    //     rather than silently returning a partial completed-task list.
    std::fs::write(
        tasks.join("T-BROKEN.md"),
        "---\nid: [unclosed yaml\n---\n\nbroken\n",
    )
    .expect("write broken task");
    let broken_reply = read_in("2025-11-25", "aplan://collections/completed?limit=10");
    assert_eq!(broken_reply["error"]["code"], -32603, "{broken_reply:#?}");
    assert!(
        broken_reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not load cleanly"),
        "{broken_reply:#?}"
    );
    std::fs::remove_file(tasks.join("T-BROKEN.md")).expect("remove broken task");

    // 5. 64 KiB size-capped collection walk over live stdio MCP transport,
    //    verifying the ACTUAL wire `contents[0].text` bytes fit under 64 KiB.
    let big_dir = tempfile::Builder::new()
        .prefix("mcp-completed-cap-e2e-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("big tempdir");
    let big_tasks = big_dir.path().join("tasks");
    std::fs::create_dir_all(&big_tasks).expect("big tasks dir");
    for i in 1..=25 {
        std::fs::write(
            big_tasks.join(format!("B-{i:02}.md")),
            format!(
                "---\nid: B-{i:02}\ntitle: \"Large task {i:02} {}\"\nphase: 1\nstatus: done\n\
                 effort_days: 1\nclosed_at: \"2026-10-10T00:{i:02}:00Z\"\n\
                 closed_at_source: local\n---\n\nbody\n",
                "x".repeat(3_500)
            ),
        )
        .expect("write big task");
    }
    let mut uri = "aplan://collections/completed?history=never&limit=25".to_string();
    let mut seen = Vec::new();
    let mut saw_size_cap = false;
    loop {
        let mut reqs = initialize_version("2025-11-25");
        reqs.push(serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "resources/read",
            "params": { "uri": uri }
        }));
        let replies = talk(big_dir.path(), &reqs);
        let text = reply(&replies, 2)["result"]["contents"][0]["text"]
            .as_str()
            .expect("capped text");
        assert!(
            text.len() <= aplan_mcp::contract::MAX_RESOURCE_BYTES,
            "MCP completed wire payload exceeded 64 KiB cap: {} bytes",
            text.len()
        );
        let doc: serde_json::Value = serde_json::from_str(text).expect("capped json");
        if doc["truncated_by_size_cap"] == true {
            saw_size_cap = true;
        }
        for row in doc["completed"]["items"].as_array().unwrap() {
            seen.push(row["id"].as_str().unwrap().to_string());
        }
        if doc["complete"] == true {
            break;
        }
        uri = doc["next_uri"].as_str().expect("next_uri").to_string();
    }
    assert!(saw_size_cap, "expected 64 KiB size cap to trigger");
    assert_eq!(seen.len(), 25);
    assert_eq!(seen.first().map(String::as_str), Some("B-25"));
    assert_eq!(seen.last().map(String::as_str), Some("B-01"));
}
