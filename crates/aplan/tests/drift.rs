//! The drift loop, as an adopter walks it (`A1-107`).
//!
//! Four of the last six pilot findings were about trace drift, and every one was
//! proved by a unit test on `anchor_fingerprint` or by a run in a throwaway
//! directory that nobody kept. Those prove the function and the afternoon; they do
//! not prove the command. If `close` stopped stamping tomorrow, every test in
//! `anchor.rs` would still pass.
//!
//! So this drives the built binary through the sequence a repository actually
//! performs: close a task, edit something else, edit the traced thing, re-stamp,
//! retarget. Both anchor shapes, because the table row (ADR-0015) is the one with
//! no end-to-end proof at all.
//!
//! **Not a `trycmd` case**, and the reason is measured rather than assumed: the
//! corpus runs a case inside its own `.in` directory with no sandbox, so a command
//! that writes edits the fixture and the next run starts somewhere else. A
//! throwaway case that closed a task changed its fixture's hash, which is why
//! `close.trycmd` pins only the guards that refuse.

use std::path::Path;

use crate::common;

/// A repository with one document in two shapes: a prose section, and a register
/// kept as a table. Both are things real plans trace, and only the second was
/// unproved.
fn fixture(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "detect_drift: true\n").expect("config");
    std::fs::write(
        dir.join("DOC.md"),
        "# Overview\n\nthe prose a task traces\n\n\
         # Invariants\n\n| # | Invariant | Status |\n|---|---|---|\n\
         | 7 | The seventh | Planned |\n| 11 | The eleventh | Planned |\n",
    )
    .expect("doc");
    // Two tasks: one traces a heading, one traces a row. Deliberately the
    // *second* row: with the first, "resolved the anchor" and "took whichever row
    // came first" produce the same answer, and a fixture that cannot tell them
    // apart is the one-candidate blindness `AGENTS.md` warns about. Caught by
    // mutating the resolver and watching this pass.
    for (id, trace) in [("A-1", "DOC.md#overview"), ("A-2", "DOC.md#11")] {
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {id}\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 paths:\n- DOC.md\ntraces:\n- {trace}\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }
    "d213c9abcdef".to_string()
}

fn run(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = common::output(dir, args);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

fn task(dir: &Path, id: &str) -> String {
    std::fs::read_to_string(dir.join(format!("aplan/tasks/{id}.md"))).expect("read")
}

#[test]
fn closing_stamps_both_anchor_shapes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());

    for id in ["A-1", "A-2"] {
        let (stdout, stderr, ok) = run(
            dir.path(),
            &["close", id, "--evidence", &format!("commit:{sha}")],
        );
        assert!(ok, "close {id} failed: {stderr}");
        assert!(
            stdout.contains("1 trace fingerprint(s) stamped"),
            "close must say it stamped: {stdout}"
        );
    }

    assert!(
        task(dir.path(), "A-1").contains("DOC.md#overview:"),
        "a heading section is stamped"
    );
    // The regression the unit tests cannot see: `anchor_fingerprint` knowing about
    // rows is not the same as `close` stamping one.
    assert!(
        task(dir.path(), "A-2").contains("DOC.md#11:"),
        "a table row is stamped, which is what ADR-0015 is for: {}",
        task(dir.path(), "A-2")
    );
}

#[test]
fn an_edit_to_a_sibling_row_is_not_this_tasks_drift() {
    // p00020's case exactly: twelve invariants under one heading, and closing a
    // task that edits invariant 11 used to re-flag every task tracing the table.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    run(
        dir.path(),
        &["close", "A-2", "--evidence", &format!("commit:{sha}")],
    );

    let doc = std::fs::read_to_string(dir.path().join("DOC.md")).expect("read");
    std::fs::write(
        dir.path().join("DOC.md"),
        doc.replace(
            "| 7 | The seventh | Planned |",
            "| 7 | The seventh | Live |",
        ),
    )
    .expect("edit");

    let (stdout, _, _) = run(dir.path(), &["check"]);
    assert!(
        !stdout.contains("APLAN018"),
        "invariant 11 did not change, so nothing may say it did: {stdout}"
    );
}

#[test]
fn an_edit_to_the_traced_row_fires_and_names_the_commit_to_diff_from() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    run(
        dir.path(),
        &["close", "A-2", "--evidence", &format!("commit:{sha}")],
    );

    let doc = std::fs::read_to_string(dir.path().join("DOC.md")).expect("read");
    std::fs::write(
        dir.path().join("DOC.md"),
        doc.replace(
            "| 11 | The eleventh | Planned |",
            "| 11 | The eleventh | Live |",
        ),
    )
    .expect("edit");

    let (stdout, _, _) = run(dir.path(), &["check"]);
    assert!(
        stdout.contains("APLAN018") && stdout.contains("DOC.md#11"),
        "the traced row changed and must be reported by name: {stdout}"
    );
    // `A1-096` is pinned as *wording* in the catalogue, which normalises the SHA
    // to a placeholder on purpose. Nothing asserted that the commit interpolated
    // is the one the task cites, and that is the half a reader pastes.
    assert!(
        stdout.contains(&format!("git diff {sha} -- DOC.md")),
        "the fix must name the commit this task cites: {stdout}"
    );
}

#[test]
fn restamp_clears_it_which_is_the_remedy_the_instruction_names() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    run(
        dir.path(),
        &["close", "A-2", "--evidence", &format!("commit:{sha}")],
    );
    let doc = std::fs::read_to_string(dir.path().join("DOC.md")).expect("read");
    std::fs::write(
        dir.path().join("DOC.md"),
        doc.replace(
            "| 11 | The eleventh | Planned |",
            "| 11 | The eleventh | Live |",
        ),
    )
    .expect("edit");

    let (stdout, stderr, ok) = run(dir.path(), &["close", "A-2", "--restamp"]);
    assert!(ok, "restamp failed: {stderr}");
    assert!(stdout.contains("restamped"), "{stdout}");

    let (stdout, _, ok) = run(dir.path(), &["check"]);
    assert!(
        ok && !stdout.contains("APLAN018"),
        "still drifting: {stdout}"
    );
}

#[test]
fn retargeting_a_trace_raises_aplan028_from_the_command_line() {
    // `A1-098` had a scenario and a unit test and no run of the sequence that
    // produces it, which is the migration ADR-0015 recommends: retarget a closed
    // task's trace from the section to the row.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let sha = fixture(dir.path());
    run(
        dir.path(),
        &["close", "A-1", "--evidence", &format!("commit:{sha}")],
    );

    let before = task(dir.path(), "A-1");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        before.replace("- DOC.md#overview\n", "- DOC.md#11\n"),
    )
    .expect("retarget");

    let (stdout, _, ok) = run(dir.path(), &["check"]);
    assert!(!ok, "a partially stamped task is an error: {stdout}");
    assert!(
        stdout.contains("APLAN028") && stdout.contains("DOC.md#11"),
        "the unstamped trace must be named: {stdout}"
    );
    assert!(
        stdout.contains("DOC.md#overview"),
        "and the orphan it still stamps, which is the only record of what it was \
         closed against: {stdout}"
    );

    // The remedy the instruction names repairs both halves.
    run(dir.path(), &["close", "A-1", "--restamp"]);
    let (stdout, _, ok) = run(dir.path(), &["check"]);
    assert!(ok, "restamp must clear it: {stdout}");
    assert!(
        !task(dir.path(), "A-1").contains("DOC.md#overview:"),
        "and drop the key no trace names"
    );
}

fn git_run(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_git(dir: &Path) {
    git_run(dir, &["init"]);
    git_run(dir, &["config", "user.email", "test@example.com"]);
    git_run(dir, &["config", "user.name", "Test"]);
    git_run(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit_all(dir: &Path, message: &str) -> String {
    git_run(dir, &["add", "-A"]);
    git_run(dir, &["commit", "-m", message]);
    git_run(dir, &["rev-parse", "--short=12", "HEAD"])
}

fn call_mcp_review(dir: &Path, id: &str) -> serde_json::Value {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = common::command(dir, &["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aplan mcp");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for req in [
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "aplan-test", "version": "1" }
                }
            }),
            serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "aplan_review", "arguments": { "id": id } }
            }),
        ] {
            writeln!(stdin, "{req}").expect("write request");
        }
        stdin.flush().ok();
    }
    let out = child.wait_with_output().expect("wait for aplan mcp");
    assert!(
        out.status.success(),
        "aplan mcp failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let replies: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("json-rpc reply"))
        .collect();
    replies
        .into_iter()
        .find(|reply| reply["id"] == 2)
        .expect("reply 2")
}

fn write_task_file(dir: &Path, id: &str, path_glob: &str, trace: Option<&str>, body: &str) {
    let traces = trace.map_or_else(String::new, |t| format!("traces:\n- {t}\n"));
    std::fs::write(
        dir.join(format!("aplan/tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\neffort_days: 1\n\
             paths:\n- {path_glob}\n{traces}---\n\n{body}\n"
        ),
    )
    .expect("task");
}

fn assert_verify_stale(dir: &Path, expect_stale: bool, context: &str) -> String {
    let (stdout, _, ok) = run(dir, &["verify"]);
    if expect_stale {
        assert!(!ok && stdout.contains("APLAN045"), "{context}: {stdout}");
    } else {
        assert!(ok && !stdout.contains("APLAN045"), "{context}: {stdout}");
    }
    stdout
}

/// Proves `traces.stale_premise_requires_substantive_edit_or_review` (`A2-053`).
#[test]
fn review_clears_stale_premise_and_rearms_when_section_moves_again() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    init_git(dir.path());
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "deny_warnings: true\n").expect("config");
    std::fs::write(
        dir.path().join("SPEC.md"),
        "# Scope\n\nOriginal requirement.\n",
    )
    .expect("spec");
    write_task_file(
        dir.path(),
        "A-1",
        "SPEC.md",
        Some("SPEC.md#scope"),
        "Initial body.",
    );
    write_task_file(dir.path(), "A-2", "SPEC.md", None, "No traces.");
    let initial_sha = commit_all(dir.path(), "initial");

    // Traced section moves after A-1 was authored -> APLAN045 fires and names both remedies.
    std::fs::write(
        dir.path().join("SPEC.md"),
        "# Scope\n\nRevised requirement.\n",
    )
    .expect("edit 1");
    commit_all(dir.path(), "revise scope");
    let stdout = assert_verify_stale(dir.path(), true, "initial drift");
    assert!(
        stdout.contains("SPEC.md#scope") && stdout.contains("aplan review A-1"),
        "APLAN045 must name the section and `aplan review A-1`: {stdout}"
    );

    // Non-substantive workflow mutations (`tag add`, `start`, uncommitted and committed) never clear APLAN045.
    assert!(run(dir.path(), &["tag", "add", "A-1", "backend"]).2);
    assert!(run(dir.path(), &["start", "A-1"]).2);
    assert_verify_stale(dir.path(), true, "uncommitted metadata edits");
    commit_all(dir.path(), "metadata workflow churn");
    assert_verify_stale(dir.path(), true, "committed metadata edits");

    // A bogus hand-written `trace_fingerprints` stamp must NOT clear APLAN045.
    let before = task(dir.path(), "A-1");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        before.replace(
            "traces:\n- SPEC.md#scope\n",
            "traces:\n- SPEC.md#scope\ntrace_fingerprints:\n  SPEC.md#scope: heading:0000000000000000\n",
        ),
    )
    .expect("bogus stamp");
    assert_verify_stale(dir.path(), true, "bogus hand-edited stamp");

    // `aplan review A-1` records the current kind-tagged stamp without committing and clears APLAN045.
    let (stdout, stderr, ok) = run(dir.path(), &["review", "A-1"]);
    assert!(ok, "aplan review A-1 failed: {stderr}");
    assert!(stdout.contains("reviewed"), "{stdout}");
    assert!(
        task(dir.path(), "A-1").contains("SPEC.md#scope: heading:"),
        "review must write a kind-tagged stamp: {}",
        task(dir.path(), "A-1")
    );
    assert_verify_stale(dir.path(), false, "after aplan review");

    // Committing another edit to the traced section re-arms APLAN045; an uncommitted substantive body edit clears it.
    commit_all(dir.path(), "record review stamp");
    std::fs::write(dir.path().join("SPEC.md"), "# Scope\n\nThird revision.\n").expect("edit 2");
    commit_all(dir.path(), "revise scope again");
    assert_verify_stale(dir.path(), true, "re-armed after spec edit");

    let current = task(dir.path(), "A-1");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        current.replace("Initial body.", "Updated body for third revision."),
    )
    .expect("substantive body edit");
    assert_verify_stale(dir.path(), false, "uncommitted substantive body edit");

    // Commit the substantive edit, move the spec once more, and clear through MCP `aplan_review`.
    commit_all(dir.path(), "substantive task update");
    std::fs::write(dir.path().join("SPEC.md"), "# Scope\n\nFourth revision.\n").expect("edit 3");
    commit_all(dir.path(), "fourth revision");
    assert_verify_stale(dir.path(), true, "fourth spec revision");

    let reply = call_mcp_review(dir.path(), "A-1");
    assert_ne!(
        reply["result"]["isError"], true,
        "aplan_review failed: {reply:#?}"
    );
    assert_verify_stale(dir.path(), false, "MCP aplan_review");

    // Refusal guards: untraced live task and terminal task.
    let (_, stderr, ok) = run(dir.path(), &["review", "A-2"]);
    assert!(
        !ok && (stderr.contains("anchored") || stderr.contains("traces")),
        "{stderr}"
    );

    let (_, stderr, ok) = run(
        dir.path(),
        &[
            "close",
            "A-1",
            "--evidence",
            &format!("commit:{initial_sha}"),
        ],
    );
    assert!(ok, "close A-1 failed: {stderr}");
    let (_, stderr, ok) = run(dir.path(), &["review", "A-1"]);
    assert!(!ok && stderr.contains("--restamp"), "{stderr}");
}

fn json_diagnostics(dir: &Path, subcommand: &str) -> (bool, Vec<serde_json::Value>) {
    let (stdout, _, ok) = run(dir, &[subcommand, "--format", "json"]);
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("json report");
    let diags = report["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .clone();
    (ok, diags)
}

/// Proves `A2-051` (#30) end-to-end across both open (`verify`) and closed (`check`) tasks,
/// symmetric `heading -> row` and `row -> heading` retargeting, advisory immunity, and
/// legacy bare-hex compatibility.
#[test]
fn trace_retargeting_between_heading_and_row_blocks_check_and_verify_even_when_drift_is_advisory() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    init_git(dir.path());
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "detect_drift: true\ndeny_warnings: true\nadvisory:\n- APLAN018\n- APLAN045\n",
    )
    .expect("config");

    let doc1 =
        "## Beta\n\nHeading content.\n\n| Name | Note |\n|---|---|\n| Beta | Row content |\n";
    let doc2 = "# Register\n\n| Name | Note |\n|---|---|\n| Gamma | Row content |\n";
    std::fs::write(dir.path().join("DOC1.md"), doc1).expect("doc1");
    std::fs::write(dir.path().join("DOC2.md"), doc2).expect("doc2");

    let legacy_bare = aplan_core::anchor::anchor_fingerprint(doc1, "beta").expect("bare");
    for (id, file, trace) in [
        ("A-1", "DOC1.md", "DOC1.md#beta"),
        ("A-2", "DOC1.md", "DOC1.md#beta"),
        ("A-4", "DOC2.md", "DOC2.md#gamma"),
        ("A-5", "DOC2.md", "DOC2.md#gamma"),
    ] {
        write_task_file(dir.path(), id, file, Some(trace), "body");
    }
    let base_sha = commit_all(dir.path(), "initial docs and tasks");

    for id in ["A-2", "A-5"] {
        let (_, stderr, ok) = run(
            dir.path(),
            &["close", id, "--evidence", &format!("commit:{base_sha}")],
        );
        assert!(ok, "close {id} failed: {stderr}");
    }
    assert!(task(dir.path(), "A-2").contains("DOC1.md#beta: heading:"));
    assert!(task(dir.path(), "A-5").contains("DOC2.md#gamma: row:"));
    std::fs::write(
        dir.path().join("aplan/tasks/A-3.md"),
        format!(
            "---\nid: A-3\ntitle: Legacy bare stamp\nphase: 1\nstatus: done\neffort_days: 1\n\
             paths:\n- DOC1.md\ntraces:\n- DOC1.md#beta\ntrace_fingerprints:\n  DOC1.md#beta: {legacy_bare}\n\
             evidence:\n- kind: commit\n  ref: {base_sha}\n---\n\nbody\n"
        ),
    )
    .expect("task A-3");
    commit_all(dir.path(), "close stamped tasks");

    assert!(run(dir.path(), &["check"]).2, "baseline check must pass");
    assert!(run(dir.path(), &["verify"]).2, "baseline verify must pass");

    // Retarget DOC1.md#beta (`heading -> row`) and DOC2.md#gamma (`row -> heading`).
    std::fs::write(
        dir.path().join("DOC1.md"),
        "## Beta Renamed\n\nHeading content.\n\n| Name | Note |\n|---|---|\n| Beta | Row content |\n",
    )
    .expect("retarget doc1");
    std::fs::write(
        dir.path().join("DOC2.md"),
        "# Register\n\n## Gamma\n\nNew heading.\n\n| Name | Note |\n|---|---|\n| Gamma | Row content |\n",
    )
    .expect("retarget doc2");
    commit_all(dir.path(), "retarget anchors");

    // Closed tasks block `check` with APLAN060 even when APLAN018 is advisory; legacy A-3 stays APLAN018.
    let (ok, diags) = json_diagnostics(dir.path(), "check");
    assert!(
        !ok,
        "APLAN060 must block check even with advisory APLAN018: {diags:#?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d["task"] == "A-2" && d["code"] == "APLAN060"),
        "{diags:#?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d["task"] == "A-5" && d["code"] == "APLAN060"),
        "{diags:#?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d["task"] == "A-3" && d["code"] == "APLAN018")
            && !diags
                .iter()
                .any(|d| d["task"] == "A-3" && d["code"] == "APLAN060"),
        "legacy bare-hex A-3 must stay APLAN018 without migration churn: {diags:#?}"
    );

    // Open tasks block `verify` with APLAN060 even when APLAN045 is advisory.
    let (ok, diags) = json_diagnostics(dir.path(), "verify");
    assert!(
        !ok,
        "APLAN060 must block verify even with advisory APLAN045: {diags:#?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d["task"] == "A-1" && d["code"] == "APLAN060"),
        "{diags:#?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d["task"] == "A-4" && d["code"] == "APLAN060"),
        "{diags:#?}"
    );
}

/// Proves `A2-085` end-to-end: YAML frontmatter and nested `---`-fenced metadata blocks
/// mint no heading anchors (`APLAN008`), while real headings work and date updates cause no drift.
#[test]
fn frontmatter_and_nested_metadata_delimiters_reject_traced_keys_end_to_end() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "detect_drift: true\n").expect("config");

    let doc_text = "\
---
id: ABC
public_readable: false
last_updated: 2026-11-22
---

# Real Plan

Body prose.

---
owner: platform
last_verified: 2026-11-22
---

Real Setext
---

Setext body.
";
    std::fs::write(dir.path().join("PLAN.md"), doc_text).expect("doc");
    for (id, trace) in [
        ("A-1", "PLAN.md#last_updated-2026-11-22"),
        ("A-2", "PLAN.md#last_verified-2026-11-22"),
        ("A-3", "PLAN.md#real-plan"),
        ("A-4", "PLAN.md#real-setext"),
    ] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: todo\neffort_days: 1\n\
                 paths:\n- PLAN.md\ntraces:\n- {trace}\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }

    let (stdout, _, ok) = run(dir.path(), &["check", "--format", "json"]);
    assert!(!ok, "frontmatter phantom traces must fail check: {stdout}");
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    let diags = report["diagnostics"].as_array().expect("diagnostics");
    for id in ["A-1", "A-2"] {
        assert!(
            diags
                .iter()
                .any(|d| d["task"] == id && d["code"] == "APLAN008"),
            "{id} must fail with APLAN008: {diags:#?}"
        );
    }
    for id in ["A-3", "A-4"] {
        assert!(
            !diags.iter().any(|d| d["task"] == id),
            "{id} traces a real heading and must be clean: {diags:#?}"
        );
    }

    // Remove the invalid tasks, close A-3 and A-4, then update leading frontmatter date -> zero drift.
    std::fs::remove_file(dir.path().join("aplan/tasks/A-1.md")).expect("rm A-1");
    std::fs::remove_file(dir.path().join("aplan/tasks/A-2.md")).expect("rm A-2");
    for id in ["A-3", "A-4"] {
        let (_, stderr, ok) = run(
            dir.path(),
            &["close", id, "--evidence", "commit:d213c9abcdef"],
        );
        assert!(ok, "close {id} failed: {stderr}");
    }
    std::fs::write(
        dir.path().join("PLAN.md"),
        doc_text.replace("last_updated: 2026-11-22", "last_updated: 2026-12-01"),
    )
    .expect("bump leading frontmatter date");
    let (stdout, stderr, ok) = run(dir.path(), &["check"]);
    assert!(
        ok && !stdout.contains("APLAN018"),
        "frontmatter date updates must not drift real headings: {stdout}\n{stderr}"
    );
}
