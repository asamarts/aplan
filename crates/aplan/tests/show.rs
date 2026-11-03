//! `aplan show`: one task, whole, with what the file cannot say (`A1-118`).
//!
//! `why` gives blockers and nothing else, `tasks --format json` omits `paths`,
//! `traces` and `evidence`, and `cat` gives a file that cannot know whether its
//! own citations still resolve. The fields carrying this tool's whole argument
//! were reachable only by opening the file by hand.

use std::path::Path;
use std::process::Command;

use crate::common;

fn aplan(dir: &Path, args: &[&str]) -> (String, String, i32) {
    let out = common::output(dir, args);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

/// A repository with a healthy closed task, a live one that waits on it, and a
/// document to trace.
fn repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(root.join("src/a.rs"), "fn main() {}\n").expect("src file");
    std::fs::write(root.join("DOC.md"), "# Doc\n\n## The cause\n\nWhy.\n").expect("doc");
    std::fs::write(
        root.join("aplan/tasks/A-2.md"),
        "---\nid: A-2\ntitle: Waiting on A-1\nphase: 1\nstatus: todo\neffort_days: 2\n\
         blocked_by: [A-1]\npaths:\n- src/**\n---\n\n## Why\n\nThe body nobody prints.\n",
    )
    .expect("task");

    git(root, &["init", "-q", "."]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "initial"]);
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .expect("rev-parse");
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // A closed task, cited by a commit that really touched its paths.
    std::fs::write(
        root.join("aplan/tasks/A-1.md"),
        format!(
            "---\nid: A-1\ntitle: The groundwork\nphase: 1\nstatus: done\nowner: t\n\
             effort_days: 1\npaths:\n- src/**\ntraces:\n- DOC.md#the-cause\nevidence:\n\
             - kind: commit\n  ref: \"{sha}\"\n---\n\n## Why\n\nBody.\n"
        ),
    )
    .expect("task");
    (dir, sha)
}

/// The three fields no other command exposes, and the four things only the graph
/// and the ladder know.
#[test]
fn one_task_shows_its_fields_and_what_the_file_cannot_know() {
    let (dir, sha) = repo();
    let (out, _, code) = aplan(dir.path(), &["show", "A-1"]);
    assert_eq!(code, 0, "{out}");

    // What no other command returns.
    assert!(out.contains("paths:\n    src/**"), "{out}");
    assert!(out.contains("traces:\n    DOC.md#the-cause"), "{out}");
    assert!(out.contains(&format!("commit:{sha}")), "{out}");

    // What the file cannot say: the other direction of the graph, and the state
    // of the task as the rules see it.
    assert!(out.contains("releases:\n    A-2  todo"), "{out}");
    assert!(out.contains("findings: none"), "{out}");
    // `ready` is a word about work somebody can pick up: a finished task is not
    // "ready no", which reads as blocked (`A1-105`).
    assert!(!out.contains("ready"), "{out}");

    // The body is prose, so it is opt-in.
    assert!(!out.contains("Body."), "{out}");
    let (with_body, _, code) = aplan(dir.path(), &["show", "A-1", "--body"]);
    assert_eq!(code, 0, "{with_body}");
    assert!(with_body.contains("Body."), "{with_body}");

    // And a live task carries the word, with the blocker it waits on and the
    // state that blocker is in -- which is what decides the word.
    let (live, _, _) = aplan(dir.path(), &["show", "A-2"]);
    assert!(live.contains("ready yes"), "{live}");
    assert!(live.contains("waits on:\n    A-1  done"), "{live}");

    // Unfinish the blocker: the same task, the same file, a different answer.
    let text = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        text.replace("status: done", "status: todo"),
    )
    .expect("write");
    let (blocked, _, _) = aplan(dir.path(), &["show", "A-2", "--no-resolve"]);
    assert!(blocked.contains("ready no"), "{blocked}");
    assert!(blocked.contains("waits on:\n    A-1  todo"), "{blocked}");
}

/// The case the command exists for: a closed task whose proof rotted reads as
/// healthy in its own file (`A1-118`).
#[test]
fn a_task_whose_evidence_stopped_resolving_looks_different() {
    let (dir, _) = repo();
    // The citation now names a commit this repository does not have. Nothing in
    // the file changes; the file cannot know.
    let text = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
    let rotted = text
        .lines()
        .map(|l| {
            if l.starts_with("  ref: ") {
                "  ref: 0000000000000000000000000000000000000000".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.path().join("aplan/tasks/A-1.md"), rotted).expect("write");

    // And a fault on a *different* task, so "the findings" cannot quietly mean
    // the plan's: `A-2` claims `done` with nothing behind it.
    let other = std::fs::read_to_string(dir.path().join("aplan/tasks/A-2.md")).expect("read");
    std::fs::write(
        dir.path().join("aplan/tasks/A-2.md"),
        other.replace("status: todo", "status: done"),
    )
    .expect("write");

    let (out, _, code) = aplan(dir.path(), &["show", "A-1"]);
    assert_eq!(code, 0, "showing a task is not a verdict: {out}");
    assert!(out.contains("findings:"), "{out}");
    assert!(out.contains("evidence state: current"), "{out}");
    assert!(out.contains("[evidence: current, failed]"), "{out}");
    assert!(out.contains("APLAN011"), "{out}");
    assert!(out.contains("fix: "), "the remedy travels with it: {out}");
    assert!(
        !out.contains("APLAN005"),
        "`A-2`'s finding is not `A-1`'s: {out}"
    );
    // The other way round, so this proves a filter rather than an ordering.
    let (other_view, _, _) = aplan(dir.path(), &["show", "A-2"]);
    assert!(other_view.contains("APLAN005"), "{other_view}");
    assert!(!other_view.contains("APLAN011"), "{other_view}");

    // And `--no-resolve` says it did not look, rather than reporting the same
    // task as clean.
    let (skipped, _, code) = aplan(dir.path(), &["show", "A-1", "--no-resolve"]);
    assert_eq!(code, 0, "{skipped}");
    assert!(!skipped.contains("APLAN011"), "{skipped}");
    assert!(
        skipped.contains("the evidence was not resolved"),
        "silence must not read as a pass: {skipped}"
    );
    assert!(
        skipped.contains("evidence state: not observed"),
        "{skipped}"
    );
    assert!(
        skipped.contains("[evidence: not observed, unavailable]"),
        "{skipped}"
    );
}

/// `--format json` carries the same content, so a consumer reading one task is
/// not pushed back to parsing the file.
#[test]
fn the_json_carries_what_the_human_form_carries() {
    let (dir, sha) = repo();
    let (out, _, code) = aplan(dir.path(), &["show", "A-1", "--format", "json", "--body"]);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");

    assert_eq!(v["schema"], "aplan/show/v3");
    assert_eq!(v["id"], "A-1");
    assert_eq!(v["status"], "done");
    assert_eq!(v["paths"][0], "src/**");
    assert_eq!(v["traces"][0], "DOC.md#the-cause");
    assert_eq!(v["evidence"][0]["kind"], "commit");
    assert_eq!(v["evidence"][0]["reference"], sha);
    assert_eq!(v["evidence"][0]["observation"]["state"], "current");
    assert_eq!(v["evidence"][0]["observation"]["outcome"], "resolved");
    assert_eq!(v["evidence_observation"]["state"], "current");
    assert_eq!(v["releases"][0]["id"], "A-2");
    assert_eq!(v["releases"][0]["status"], "todo");
    assert_eq!(v["resolved"], true);
    assert!(v["body"].as_str().unwrap_or_default().contains("Body."));
    assert_eq!(v["findings"].as_array().map(Vec::len), Some(0));
}

/// Proof for `evidence.stale_premise_uses_configured_roots`: APLAN045 must make
/// an absolute configured plan path repository-relative and read historical
/// document bytes from `traces_root`, not silently look beside the
/// configuration for a file that is not there.
#[test]
fn verify_and_show_resolve_stale_premises_through_configured_roots() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("repository");
    let root = dir.path();
    std::fs::create_dir(root.join("aplan")).expect("plan");
    std::fs::create_dir(root.join("docs")).expect("documents");
    std::fs::write(root.join(".aplan.yml"), "root: aplan\ntraces_root: docs\n").expect("config");
    std::fs::write(
        root.join("aplan/A-1.md"),
        "---\nid: A-1\ntitle: Waiting on a moving premise\nphase: 1\nstatus: todo\n\
         effort_days: 1\npaths: [src/**]\ntraces: [DESIGN.md#fact]\n---\n",
    )
    .expect("task");
    std::fs::write(root.join("docs/DESIGN.md"), "# Fact\n\noriginal\n").expect("document");
    git(root, &["init", "-q", "."]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "initial"]);
    std::fs::write(root.join("docs/DESIGN.md"), "# Fact\n\nchanged\n").expect("changed");
    git(root, &["add", "docs/DESIGN.md"]);
    git(root, &["commit", "-q", "-m", "move premise"]);

    let (verified, err, code) = aplan(root, &["verify", "--format", "json"]);
    assert_eq!(code, 0, "{verified}{err}");
    let verified: serde_json::Value = serde_json::from_str(&verified).expect("verify JSON");
    assert!(
        verified["diagnostics"]
            .as_array()
            .is_some_and(|findings| findings.iter().any(|finding| finding["code"] == "APLAN045")),
        "verify silently missed the moved configured trace: {verified:#}"
    );

    let (out, err, code) = aplan(root, &["show", "A-1", "--format", "json"]);
    assert_eq!(code, 0, "{out}{err}");
    let shown: serde_json::Value = serde_json::from_str(&out).expect("show JSON");
    assert!(
        shown["findings"]
            .as_array()
            .is_some_and(|findings| findings.iter().any(|finding| finding["code"] == "APLAN045")),
        "show silently missed the moved configured trace: {shown:#}"
    );
}

/// Proof for `show.carries_complete_task_projection`: the typed
/// projection really is complete before the TUI consumes it; JSON exposes the
/// persisted relationships, fingerprints and version claims as fields.
#[test]
fn the_typed_show_projection_carries_every_persisted_task_field() {
    let (dir, _) = repo();
    let path = dir.path().join("aplan/tasks/A-1.md");
    let source = std::fs::read_to_string(&path).expect("task");
    let source = source
        .replace(
            "effort_days: 1\n",
            "effort_days: 1\nprefer_after: [MISSING]\nratified_by: [A-2]\ncontains: [A-2]\n\
             version_impact: breaking\nversion_impact_reason: public contract changed\n\
             version: 1.2.3\n",
        )
        .replace(
            "- DOC.md#the-cause\n",
            "- DOC.md#the-cause\ntrace_fingerprints:\n  DOC.md#the-cause: trace-stamp\n",
        )
        .replace(
            "---\n\n## Why",
            "evidence_fingerprints:\n  'file:old-proof.md': evidence-stamp\n---\n\n## Why",
        );
    std::fs::write(path, source).expect("rich task");

    let (out, _, code) = aplan(
        dir.path(),
        &["show", "A-1", "--format", "json", "--no-resolve", "--body"],
    );
    assert_eq!(code, 0, "{out}");
    let show: serde_json::Value = serde_json::from_str(&out).expect("show json");
    assert_eq!(
        show["trace_fingerprints"]["DOC.md#the-cause"],
        "trace-stamp"
    );
    assert_eq!(
        show["evidence_fingerprints"]["file:old-proof.md"],
        "evidence-stamp"
    );
    assert_eq!(show["prefer_after"][0]["id"], "MISSING");
    assert_eq!(show["prefer_after"][0]["availability"], "missing");
    assert_eq!(show["ratified_by"][0]["id"], "A-2");
    assert_eq!(show["ratified_by"][0]["availability"], "loaded");
    assert_eq!(show["contains"][0]["id"], "A-2");
    assert_eq!(show["version_impact"], "breaking");
    assert_eq!(show["version_impact_reason"], "public contract changed");
    assert_eq!(show["version"], "1.2.3");
    assert!(
        show["body"]
            .as_str()
            .is_some_and(|body| body.contains("Body."))
    );

    let (human, _, code) = aplan(dir.path(), &["show", "A-1", "--no-resolve"]);
    assert_eq!(code, 0, "{human}");
    for fact in [
        "version impact breaking",
        "version impact reason: public contract changed",
        "fingerprint trace-stamp",
        "orphaned fingerprint evidence-stamp",
        "prefers after:",
        "MISSING  missing",
        "ratified by:",
        "contains:",
    ] {
        assert!(human.contains(fact), "missing `{fact}`:\n{human}");
    }
}

/// A flag that takes a value and ignores it is worse than one that refuses it
/// (`A1-118`).
#[test]
fn the_report_formats_are_refused_rather_than_ignored() {
    let (dir, _) = repo();
    for format in ["agent", "github", "sarif"] {
        let (out, err, code) = aplan(dir.path(), &["show", "A-1", "--format", format]);
        let said = format!("{out}{err}");
        assert_eq!(code, 2, "`{format}` was accepted: {said}");
        assert!(said.contains("renders one task"), "{said}");
        assert!(said.contains(format), "it names what was asked for: {said}");
        assert!(!said.contains("A-1  done"), "it rendered anyway: {said}");
    }
    // And the two it has still work.
    for format in ["human", "json"] {
        let (out, _, code) = aplan(
            dir.path(),
            &["show", "A-1", "--format", format, "--no-resolve"],
        );
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("A-1"), "{out}");
    }
}

/// The ladder is scoped to the task being shown, which is the whole reason it is
/// affordable to run by default (`A1-118`).
///
/// Resolving the whole plan to report on one task cost a full `verify` -- 1.77s
/// on this repository's own plan, scaling with every *other* task's citations.
/// Sound because every rung of the ladder asks about this task's citations, and
/// the rules that need the other tasks are the hermetic ones, which still run
/// over the whole plan: the test below is that both answers are the same.
#[test]
fn the_findings_are_what_verify_would_say_about_this_task() {
    let (dir, _) = repo();
    // A fault the ladder finds, on the task being shown, and a hermetic fault on
    // another task, so both halves are exercised at once.
    let text = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        text.lines()
            .map(|l| {
                if l.starts_with("  ref: ") {
                    "  ref: 0000000000000000000000000000000000000000".to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("write");

    let (whole, _, _) = aplan(dir.path(), &["verify", "--format", "json"]);
    let whole: serde_json::Value = serde_json::from_str(&whole).expect("json");
    let mut expected: Vec<String> = whole["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter(|d| d["task"] == "A-1")
        .map(|d| format!("{}: {}", d["code"], d["message"]))
        .collect();
    expected.sort();
    assert!(!expected.is_empty(), "the fixture must produce a finding");

    let (one, _, _) = aplan(dir.path(), &["show", "A-1", "--format", "json"]);
    let one: serde_json::Value = serde_json::from_str(&one).expect("json");
    let mut got: Vec<String> = one["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|d| format!("{}: {}", d["code"], d["message"]))
        .collect();
    got.sort();
    assert_eq!(
        got, expected,
        "a scoped ladder must not be a different ladder"
    );
}

/// The scoping is observable, not just faster: a runner that is not needed is
/// not run (`A1-118`).
///
/// A timing test would be a benchmark. This is the same property with a witness:
/// the configured `test.list` command touches a file when it runs, and showing a
/// task that cites no test must not touch it, while `verify` -- which resolves
/// the task next door that *does* cite one -- must.
#[test]
fn showing_one_task_does_not_resolve_another_tasks_citations() {
    let (dir, _) = repo();
    let root = dir.path();
    std::fs::write(
        root.join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [\"sh\", \"-c\", \"touch ran.marker; echo a_test\"]\n  \
         run: [\"sh\", \"-c\", \"true\", \"{test}\"]\n",
    )
    .expect("config");
    // `A-2` cites a test; `A-1` does not.
    let a2 = std::fs::read_to_string(root.join("aplan/tasks/A-2.md")).expect("read");
    std::fs::write(
        root.join("aplan/tasks/A-2.md"),
        a2.replace(
            "status: todo",
            "status: done\nevidence:\n- kind: test\n  ref: a_test",
        ),
    )
    .expect("write");

    let marker = root.join("ran.marker");
    let (out, _, code) = aplan(root, &["show", "A-1"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !marker.exists(),
        "showing `A-1` listed the runner for `A-2`'s citation"
    );

    // And the runner is still run when the task being shown is the one citing a
    // test, or the scoping would have turned into a silence.
    let (out, _, code) = aplan(root, &["show", "A-2", "--refresh-evidence"]);
    assert_eq!(code, 0, "{out}");
    assert!(marker.exists(), "`A-2`'s own citation was not resolved");
}

/// An unknown id is a usage error, and a plan that did not load is not answered
/// with a task read out of it (ADR-0007).
#[test]
fn an_unknown_id_and_a_broken_plan_are_told_apart() {
    let (dir, _) = repo();
    let (out, err, code) = aplan(dir.path(), &["show", "NOPE"]);
    assert_eq!(code, 2, "{out}{err}");
    assert!(format!("{out}{err}").contains("no task `NOPE`"));

    std::fs::write(dir.path().join("aplan/tasks/BAD.md"), "---\nid: [\n---\n").expect("bad");
    let (out, err, code) = aplan(dir.path(), &["show", "A-1"]);
    assert_eq!(
        code, 1,
        "a partial plan is a findings exit, not a usage one"
    );
    let said = format!("{out}{err}");
    assert!(said.contains("failed to load"), "{said}");
}
