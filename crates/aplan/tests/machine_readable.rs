//! `ready` and `why` answer a machine, and answer it the same way MCP does
//! (`A1-182`).
//!
//! The surface was inverted. `aplan_ready` and `aplan_why` returned structured
//! payloads, so the answers already existed as fields -- the *CLI* was the
//! surface that could not hand them over, and a script driving it had to regular
//! -expression `A1-117  Fork-PR routing gate [...] (1d)`. The human surface was
//! more capable than the machine one, which is a divergence nobody would choose.
//!
//! These tests compare the two surfaces on one plan. They are not a check that
//! two builders agree: the CLI calls `ready_of`/`why_of` and serialises the
//! result, so the answers are one value rendered twice. What they prove is that
//! it stays that way -- that nobody reintroduces a second shape beside it, which
//! is exactly how the two drifted apart in the first place.

use std::path::Path;

use crate::common;

/// A plan with a blocked task, a preference among ready work, and a cut whose
/// members are all done, so every field in both payloads has a value.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");

    let tasks = [
        (
            "T-001",
            "---\nid: T-001\ntitle: first, and done\nphase: 0\nstatus: done\nowner: t\n\
                   effort_days: 1\nevidence:\n  - kind: note\n    ref: seeded\n    \
                   captured: 2026-01-01\n---\n\n## Why\n\nx\n",
        ),
        (
            "T-002",
            "---\nid: T-002\ntitle: ready, and defers to its neighbour\n\
                   phase: 0\nstatus: todo\nowner: t\neffort_days: 2\nprefer_after: [T-003]\n\
                   ---\n\n## Why\n\nx\n",
        ),
        (
            "T-003",
            "---\nid: T-003\ntitle: also ready\nphase: 0\nstatus: todo\nowner: t\n\
                   effort_days: 3\n---\n\n## Why\n\nx\n",
        ),
        (
            "T-004",
            "---\nid: T-004\ntitle: waiting on work and on a ghost\nphase: 0\n\
                   status: todo\nowner: t\neffort_days: 1\nblocked_by: [T-002, GHOST]\n\
                   ---\n\n## Why\n\nx\n",
        ),
        (
            "M-c0",
            "---\nid: M-c0\ntitle: a cut whose members are all done\nphase: 0\n\
                  status: todo\ntags: [milestone]\nblocked_by: [T-001]\n---\n\n## Why\n\nx\n",
        ),
    ];
    for (id, body) in tasks {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }
}

fn cli_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = common::output(dir, args);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    serde_json::from_str(text.trim()).unwrap_or_else(|e| {
        panic!(
            "`aplan {}` is not JSON: {e}\n{text}{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

/// The CLI document without its envelope: what is left must be the MCP answer.
fn payload(mut doc: serde_json::Value) -> serde_json::Value {
    let map = doc.as_object_mut().expect("a JSON object");
    assert!(
        map.remove("schema").is_some(),
        "every machine-readable output carries a `schema` key"
    );
    doc
}

/// Remove the deliberately per-observation clock after proving it is present.
///
/// Separate CLI/MCP invocations can cross a one-second boundary. Comparing the
/// sampled clock for equality would make a parity audit depend on scheduling,
/// while omitting it without checking would let the field disappear unnoticed
/// (`A1-531`).
fn stable_ready(mut doc: serde_json::Value) -> serde_json::Value {
    let observed = doc
        .as_object_mut()
        .expect("a ready object")
        .remove("claim_observed_at_epoch_seconds")
        .and_then(|value| value.as_i64())
        .expect("ready carries an integer claim observation clock");
    assert!(observed >= 0, "claim observation precedes the Unix epoch");
    doc
}

#[test]
fn ready_answers_the_cli_and_mcp_with_one_value() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let cli = payload(cli_json(dir.path(), &["ready", "--format", "json"]));
    let mcp = serde_json::to_value(aplan_mcp::tools::ready(dir.path()).expect("mcp ready"))
        .expect("serialisable");

    assert_eq!(
        stable_ready(cli.clone()),
        stable_ready(mcp),
        "the CLI and MCP answers for `ready` differ"
    );

    // And the answer is worth having: the fixture is built so each field has a
    // value, or this would be comparing two empty lists.
    let ready = cli["ready"].as_array().expect("a ready list");
    assert!(
        ready.len() >= 2,
        "the fixture offers nothing to rank: {cli}"
    );
    assert_eq!(
        cli["closeable_cuts"]
            .as_array()
            .expect("closeable cuts")
            .len(),
        1,
        "the closeable cut is missing, so the field beside the list proves nothing: {cli}"
    );
    assert!(
        ready.iter().any(|t| t["prefers_to_follow"]
            .as_array()
            .is_some_and(|a| !a.is_empty())),
        "no task carries a preference, so the reason-as-a-field is untested: {cli}"
    );
}

#[test]
fn why_answers_the_cli_and_mcp_with_one_value() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // Both traversals: `--all` is a control on both surfaces, and one value means
    // one value under either (`A1-263`).
    for id in ["T-004", "T-003", "M-c0"] {
        for all in [false, true] {
            let mut argv = vec!["why", id, "--format", "json"];
            if all {
                argv.push("--all");
            }
            let cli = payload(cli_json(dir.path(), &argv));
            let mcp =
                serde_json::to_value(aplan_mcp::tools::why(dir.path(), id, all).expect("mcp why"))
                    .expect("serialisable");
            assert_eq!(
                cli, mcp,
                "the CLI and MCP answers for `why {id}` differ (all={all})"
            );
        }
    }

    // The three facts that used to exist only in the human rendering.
    let blocked = payload(cli_json(dir.path(), &["why", "T-004", "--format", "json"]));
    assert_eq!(
        blocked["unknown"].as_array().map(Vec::len),
        Some(1),
        "a blocker naming no task is not reported: {blocked}"
    );
    let cut = payload(cli_json(dir.path(), &["why", "M-c0", "--format", "json"]));
    assert_eq!(
        cut["closeable"],
        serde_json::json!(true),
        "a cut waiting to be closed is not distinguished from a blocked task: {cut}"
    );
}

/// The reasons `--explain` renders as a sentence are fields in the JSON, so a
/// consumer can sort by what it is being told rather than parsing prose.
/// Proves the named behaviour `ready.order_is_derived` (`A1-211`).
#[test]
fn the_explain_reasons_are_fields_not_a_sentence() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let doc = cli_json(dir.path(), &["ready", "--format", "json"]);
    for task in doc["ready"].as_array().expect("a ready list") {
        for field in ["releases", "critical"] {
            assert!(
                !task[field].is_null(),
                "`{field}` is the reason for the order and is missing: {task}"
            );
        }
    }

    // And the sentence is built from those same values: a task that prefers to
    // follow another says so in both surfaces or in neither.
    let out = common::output(dir.path(), &["ready", "--explain"]);
    let human = String::from_utf8_lossy(&out.stdout);
    let follows: Vec<&str> = doc["ready"]
        .as_array()
        .expect("a ready list")
        .iter()
        .filter(|t| {
            t["prefers_to_follow"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        })
        .map(|t| t["id"].as_str().expect("an id"))
        .collect();
    assert!(
        !follows.is_empty(),
        "nothing prefers to follow, so this proves nothing"
    );
    // On the indented `--explain` line, not anywhere in the output. The first
    // draft looked for the phrase across the whole of stdout, and the fixture's
    // own task *title* contained it, so the assertion held with the sentence
    // removed entirely.
    let sentences: Vec<&str> = human.lines().filter(|l| l.starts_with("    ")).collect();
    assert!(
        !sentences.is_empty(),
        "`--explain` printed no reasons at all:\n{human}"
    );
    assert!(
        sentences.iter().any(|l| l.contains("prefers to follow")),
        "the JSON reports a preference that no `--explain` reason mentions:\n{human}"
    );
}

/// `--format json` is not conditional on `--explain`: a machine shape that
/// changes with a flag is one a consumer has to know how it was invoked to read.
#[test]
fn the_json_shape_does_not_depend_on_explain() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let plain = stable_ready(cli_json(dir.path(), &["ready", "--format", "json"]));
    let explained = stable_ready(cli_json(
        dir.path(),
        &["ready", "--explain", "--format", "json"],
    ));
    assert_eq!(plain, explained, "`--explain` changed the machine shape");
}

/// `aplan facts` now answers for these two, which is what `A1-184` made
/// possible and what a consumer asks before it invokes.
#[test]
fn facts_advertises_the_new_formats() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let doc = cli_json(dir.path(), &["facts"]);

    for name in ["ready", "why"] {
        let entry = doc["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no `{name}` command in facts"));
        assert_eq!(
            entry["formats"],
            serde_json::json!(["human", "json"]),
            "`{name}` does not advertise the format it now takes"
        );
    }
}
