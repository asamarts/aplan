//! `aplan mcp`, driven over real stdio as a client would.
//!
//! The MCP surface is unit-tested inside `aplan-mcp`, but nothing exercised the
//! *command*: the runtime it builds, the config it resolves, the transport it
//! serves on. That gap meant `cmd_mcp` could break with CI still green, which is
//! the class of hole this repository does not tolerate elsewhere.
//!
//! This speaks JSON-RPC to the built binary and asserts on the replies, so it
//! covers the wiring end to end rather than the library alone.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;

mod common;

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
        .stderr(Stdio::null())
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

    let stdout = child.stdout.take().expect("stdout");
    let replies: Vec<serde_json::Value> = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect();

    let _ = child.wait();
    replies
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
/// `APLAN_MCP_SURFACE=overwrite cargo test -p aplan --test mcp`.
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
             -p aplan --test mcp`",
            path.display()
        )
    });
    assert_eq!(
        committed, generated,
        "the surface an agent sees has changed. If that is deliberate, regenerate with \
         `APLAN_MCP_SURFACE=overwrite cargo test -p aplan --test mcp` and let the diff be \
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
    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("a line on the protocol stream is not JSON: {e}\n{line}"));
        assert_eq!(
            parsed["jsonrpc"], "2.0",
            "a line on the protocol stream is not JSON-RPC:\n{line}"
        );
    }

    // And the note did happen, on the stream that is for notes.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("never a repository's policy"),
        "the unreadable per-user file was not reported at all: {stderr}"
    );

    // The tool still did its job: a per-user file nobody can read is not a
    // reason to refuse a claim that named its own holder.
    let held = std::fs::read_to_string(dir.path().join("tasks/A-1.md")).expect("read");
    assert!(
        held.contains("owner: ada"),
        "the claim did not go through:\n{held}"
    );
}
