//! `verify` says what it had no resolver for (`A1-253`).
//!
//! A resolver joins the run only when `.aplan.yml` declares it: no `test:`
//! runner, no test resolver, so a `test` citation stays at L0 and nothing
//! resolves it. The run then reported `ok: N tasks, 0 warning(s), evidence
//! resolved`, with a JSON document carrying no field that distinguished a
//! citation it opened from one it never looked at. `close` was honest at the
//! moment it wrote the citation; the command whose whole job is resolving them
//! said less.
//!
//! This repository was the first instance (`A1-120`, 28 citations across 27
//! tasks) and fixed it here by configuring a runner, recording that the general
//! version "generalises past this repository: any adopter can be in it, and
//! nothing tells them". The ninth pilot was that adopter
//! (pre-public adoption evidence item 3), arriving with a suite of four
//! all-or-nothing commands and `require_min_strength: [test]`, which is the
//! policy the docs steer toward.

use std::path::Path;
use std::process::Command;

use crate::common;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}");
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("unopened-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan")).expect("plan dir");
    std::fs::write(dir.path().join("f.txt"), "x").expect("file");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "init"]);
    dir
}

/// A plan of one `done` task citing `evidence`, under `config`.
fn plan(dir: &Path, config: &str, evidence: &str) {
    std::fs::write(dir.join(".aplan.yml"), config).expect("config");
    std::fs::write(
        dir.join("aplan/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: a task\nphase: 1\nstatus: done\neffort_days: 1\n\
             evidence:\n{evidence}---\n\n## Why\n\nBecause.\n"
        ),
    )
    .expect("task");
}

const NO_RUNNER: &str = "root: aplan\nevidence:\n  require_min_strength: [test]\n";
const A_RUNNER: &str =
    "root: aplan\ntest:\n  list: [echo, some_name]\n  run: [\"true\", \"{test}\"]\n";
const CITES_A_TEST: &str = "  - kind: test\n    ref: some_name\n";

/// **The verdict line does not claim what the run did not do.**
///
/// On the summary rather than only on the note, because a consumer reading
/// stdout alone is exactly who is misled: stderr is what a pipe separates, which
/// is the argument `Kind::failure_cause` already carries for the failing case.
/// Proves the named behaviour `verify.reports_what_nothing_checked` (`A1-253`).
#[test]
fn the_summary_says_how_many_citations_nothing_opened() {
    let dir = repo();
    plan(dir.path(), NO_RUNNER, CITES_A_TEST);

    let out = common::run(dir.path(), &["verify"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        out.stdout.contains("1 citation(s) unchecked"),
        "the summary must say what was not opened: {}",
        out.stdout
    );
    assert!(
        out.stderr.contains("nothing is configured to check `test`"),
        "and the note must name the configuration that caused it: {}",
        out.stderr
    );
}

/// **A configured runner leaves the verdict exactly as it was.**
///
/// The other half, and the one that keeps the first honest: a count that never
/// reaches zero would be a permanent nag, and the summary of a run that really
/// did resolve everything must not have gained a clause.
#[test]
fn a_run_that_opened_everything_says_what_it_always_said() {
    let dir = repo();
    plan(dir.path(), A_RUNNER, CITES_A_TEST);

    let out = common::run(dir.path(), &["verify"]);
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(
        out.stdout.contains("evidence resolved") && !out.stdout.contains("unchecked"),
        "a complete run's verdict is unchanged: {}",
        out.stdout
    );
    assert!(
        !out.stderr.contains("nothing is configured"),
        "and it says nothing: {}",
        out.stderr
    );
}

/// **The machine report carries it as a field**, so a gate branches on data
/// rather than parsing the summary it was told not to trust.
///
/// The same name and the same shape the MCP payload uses. One fact described two
/// ways is how two surfaces start disagreeing about one plan (`A1-104`), and the
/// field is absent rather than zero where everything was opened, so its presence
/// is itself the signal.
/// Proves the named behaviour `verify.reports_what_nothing_checked` (`A1-253`).
#[test]
fn the_json_report_carries_the_count_in_both_directions() {
    let dir = repo();
    plan(dir.path(), NO_RUNNER, CITES_A_TEST);
    let out = common::run(dir.path(), &["verify", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect("the report is json");
    assert_eq!(
        doc["evidence_unresolved"],
        serde_json::json!({"citations": 1, "tasks": 1, "unconfigured_kinds": ["test"]}),
        "the field is what a gate reads: {}",
        out.stdout
    );

    let dir = repo();
    plan(dir.path(), A_RUNNER, CITES_A_TEST);
    let out = common::run(dir.path(), &["verify", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect("the report is json");
    assert!(
        doc.get("evidence_unresolved").is_none(),
        "absent when there is nothing to say: {}",
        out.stdout
    );
}

/// **A dated note is not counted**, for the same reason `check`'s twin does not
/// count it: a note is recorded rather than resolved by design, so counting it
/// would make the note fire on every plan that holds one and mean nothing.
#[test]
fn a_note_is_recorded_by_design_and_is_not_reported_as_unopened() {
    let dir = repo();
    plan(
        dir.path(),
        "root: aplan\n",
        "  - kind: note\n    ref: a phone call\n    captured: 2026-09-02\n",
    );

    let out = common::run(dir.path(), &["verify"]);
    assert!(
        !out.text().contains("unchecked") && !out.text().contains("nothing is configured"),
        "a note is not an unopened citation: {}",
        out.text()
    );
}

/// **The note names every kind nothing checks, not just the first.**
///
/// `url` has the same shape as `test`: the resolver joins only when a checker is
/// configured. A message naming one of two would send an adopter to fix half a
/// configuration and run again.
#[test]
fn every_unconfigured_kind_is_named_once() {
    let dir = repo();
    plan(
        dir.path(),
        "root: aplan\n",
        "  - kind: test\n    ref: some_name\n  - kind: url\n    ref: https://example.invalid/\n",
    );

    let out = common::run(dir.path(), &["verify"]);
    let err = &out.stderr;
    assert!(
        err.contains("`test`") && err.contains("`url`"),
        "both kinds must be named: {err}"
    );
    assert!(
        err.contains("2 citation(s) on 1 task(s)"),
        "counted per citation and per task, like `check`'s twin: {err}"
    );
}

/// **Both surfaces describe one plan with one field.**
///
/// The guard that keeps the pair from drifting apart again. `check` carries
/// `evidence_unresolved` on the agent surface and a note on the CLI's stderr;
/// `verify` had neither, and the first fix gave the CLI a bare
/// `unchecked_citations` number while the agent got an object under a different
/// name. Two names for one fact is how two surfaces start disagreeing about one
/// plan (`A1-104`), so this compares the documents rather than trusting that
/// they were written together.
#[test]
fn the_two_surfaces_report_it_identically() {
    let dir = repo();
    plan(
        dir.path(),
        "root: aplan\n",
        "  - kind: test\n    ref: some_name\n  - kind: url\n    ref: https://example.invalid/\n",
    );

    let cli: serde_json::Value =
        serde_json::from_str(&common::run(dir.path(), &["verify", "--format", "json"]).stdout)
            .expect("the report is json");

    let agent = aplan_mcp::tools::verify(
        &dir.path().join("aplan"),
        &aplan_mcp::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
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
            repo_root: dir.path().to_path_buf(),
        },
    )
    .expect("the agent surface verifies");

    assert_eq!(
        cli["evidence_unresolved"], agent["evidence_unresolved"],
        "one plan, one answer:\n  cli:   {}\n  agent: {}",
        cli["evidence_unresolved"], agent["evidence_unresolved"]
    );
    assert!(
        !cli["evidence_unresolved"].is_null(),
        "and it is not that both are silent: {cli}"
    );
}
