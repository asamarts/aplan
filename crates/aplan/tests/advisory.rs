//! `advisory:` reaches a verdict from a real `.aplan.yml` (`A1-112`).
//!
//! The library tests construct a [`Verdict`](aplan_core::Verdict) in Rust, which
//! skips the half of this feature an adopter writes: the YAML, the exit code, and
//! the summary line the exit code has to agree with. The case the pilot reported
//! is a *migration*, so what is proved here is that a plan that cannot yet satisfy
//! `APLAN022` can still see every one of its findings while its build stays green
//! (pre-public adoption evidence, item 3).

use std::process::Command;

mod common;
use common::aplan;

/// The unresolved-evidence note is on stderr, so a JSON consumer's stdout stays
/// one document (`A1-173`).
///
/// Pinned here rather than in the `trycmd` corpus, which merges the two streams
/// and so cannot see the difference at all: the note is written for the reader
/// of a gate's log, and a line of English in the middle of a report a consumer
/// parses would be a worse defect than the silence it replaces. It is the same
/// reason `denied:` is on stderr, one command over.
#[test]
fn the_unresolved_evidence_note_never_lands_in_a_json_report() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let tasks = dir.path().join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("dirs");
    std::fs::write(
        tasks.join("W-001.md"),
        "---\nid: W-001\ntitle: work\nphase: 0\nstatus: done\neffort_days: 1\n\
         paths:\n  - src/**\nevidence:\n  - kind: commit\n    ref: d213c9a\n---\n\nWhy.\n",
    )
    .expect("write");

    let out = common::output(dir.path(), &["check", "--format", "json"]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    let doc: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is one document: {e}\n{stdout}"));
    assert_eq!(doc["schema"], "aplan/check-report/v1", "{doc}");
    assert!(
        !stdout.contains("did not open the repository"),
        "the note must not be in the report: {stdout}"
    );
    assert!(
        stderr.contains("did not open the repository") && stderr.contains("aplan verify"),
        "and it must still be said: {stderr}"
    );
}

/// A plan mid-migration: membership totality is on, and no task is in a cut yet.
///
/// `extra` is appended to `.aplan.yml`, so each test writes only the policy it is
/// about.
fn repo(extra: &str) -> tempfile::TempDir {
    repo_with(&format!("require_cut_membership: true\n{extra}"))
}

/// The same three tasks under exactly the config given, for a test about a rule
/// other than `APLAN022`: leaving membership totality on would fail the run for
/// a reason the test is not about.
fn repo_with(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::write(root.join(".aplan.yml"), config).expect("config");
    for n in 1..=3 {
        std::fs::write(
            root.join(format!("aplan/tasks/A-{n}.md")),
            format!(
                "---\nid: A-{n}\ntitle: A task that is number {n} in this fixture\n\
                 phase: 1\nstatus: todo\nowner: kaminsod\neffort_days: 1\n\
                 paths:\n- src/**\n---\n\n## Why\n\nSomething to check.\n"
            ),
        )
        .expect("task");
    }
    dir
}

/// The pilot's own case: every finding is heard, and the build is green.
#[test]
fn an_advisory_rule_is_reported_in_full_and_does_not_fail_the_build() {
    let dir = repo("advisory:\n  - APLAN022\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "advisory must not block:\n{out}");
    // Heard: one line per task, not a count and not a summary.
    for n in 1..=3 {
        assert!(out.contains(&format!("A-{n}: belongs to no cut")), "{out}");
    }
    // At the rule's own severity, because the severity is the rule's and only the
    // tolerance is the repository's.
    assert!(out.contains("error[APLAN022]"), "{out}");
}

/// The defect this file exists to prevent: a summary that contradicts its own
/// exit code. `failed:` on a command that exited 0 is the same wrong-noun defect
/// this project files tasks about in other people's tools.
/// Proves the named behaviour `verdict.summary_agrees_with_exit_code` (`A1-211`).
#[test]
fn the_summary_agrees_with_the_exit_code_and_names_what_was_downgraded() {
    let dir = repo("advisory:\n  - APLAN022\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("failed:"),
        "passing build said failed:\n{out}"
    );
    assert!(out.contains("ok: 3 tasks"), "{out}");
    // Never silent: the count and the rule that produced it are both on the line,
    // so a green build that was green because of a downgrade says so.
    assert!(out.contains("3 advisory (APLAN022)"), "{out}");
}

/// The control. Without the key the same plan fails, so the test above is
/// measuring the policy rather than a plan that was clean anyway.
#[test]
fn without_the_key_the_same_plan_fails() {
    let dir = repo("");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("failed: 3 error(s)"), "{out}");
    assert!(!out.contains("advisory"), "nothing was downgraded:\n{out}");
}

/// A consumer scripting on the JSON counts must get counts that agree with the
/// exit code, or the two disagree at the only place a machine reads.
#[test]
fn the_json_counts_are_the_blocking_ones_and_advisory_is_reported_separately() {
    let dir = repo("advisory:\n  - APLAN022\n");
    let (out, code) = aplan(
        dir.path(),
        &["check", "--root", "aplan", "--format", "json"],
    );
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json document");
    assert_eq!(doc["errors"], 0, "{out}");
    assert_eq!(doc["warnings"], 0, "{out}");
    assert_eq!(doc["advisory"], 3, "{out}");
    // The diagnostics themselves are untouched: a consumer asserting what a rule
    // *says* must not be asserting something a config can change.
    let diags = doc["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(diags.len(), 3, "{out}");
    assert!(diags.iter().all(|d| d["severity"] == "error"), "{out}");
}

/// Downgrading one rule must not downgrade the next one. A policy that swallowed
/// unrelated findings would be worse than no policy: it would make the build green
/// for a reason nobody chose.
#[test]
fn an_advisory_rule_does_not_swallow_an_unrelated_error() {
    let dir = repo("advisory:\n  - APLAN022\n");
    // A dependency on a task that does not exist: `APLAN003`, nobody downgraded it.
    std::fs::write(
        dir.path().join("aplan/tasks/A-4.md"),
        "---\nid: A-4\ntitle: A task that depends on one nobody wrote\n\
         phase: 1\nstatus: todo\nowner: kaminsod\neffort_days: 1\n\
         blocked_by:\n- A-99\npaths:\n- src/**\n---\n\n## Why\n\nSomething to check.\n",
    )
    .expect("task");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 1, "an undowngraded error must still block:\n{out}");
    // Exactly one error is counted: the four `APLAN022` findings are heard and
    // excluded, and the summary still discloses them.
    assert!(out.contains("failed: 1 error(s)"), "{out}");
    assert!(out.contains("4 advisory (APLAN022)"), "{out}");
}

/// A typo'd code would downgrade nothing while reading as a repository that made
/// a choice, which is the vacuity this whole task is about. It is refused as a
/// usage error rather than ignored.
#[test]
fn an_unknown_rule_code_is_refused_and_named() {
    let dir = repo("advisory:\n  - APLAN999\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "a config mistake is a usage error:\n{out}");
    assert!(
        out.contains("APLAN999"),
        "the message names the code:\n{out}"
    );
}

/// `verify` is the other half of the surface, and an adopter who set the policy
/// for `check` did not set a different one for `verify`.
#[test]
fn verify_honours_the_same_policy() {
    let dir = repo("advisory:\n  - APLAN022\n");
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .output()
        .expect("git init");
    let (out, code) = aplan(dir.path(), &["verify", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("3 advisory (APLAN022)"), "{out}");
    // And it still says what *this* command checked. Nothing pinned this noun,
    // so `verify` could have claimed `DAG acyclic` and no test would have known,
    // which is the wrong-noun claim the partial-plan cases are about.
    assert!(out.contains("evidence resolved"), "{out}");
}

/// The two knobs are independent: `deny_warnings` turns warnings into blockers,
/// and `advisory` takes a named rule back out. A repository can want both.
#[test]
fn deny_warnings_does_not_block_on_a_rule_the_repository_made_advisory() {
    // `APLAN009`: an active task carrying no effort estimate is a warning here.
    let dir = repo("deny_warnings: true\nadvisory:\n  - APLAN022\n");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: A task that is number 1 in this fixture\n\
         phase: 1\nstatus: todo\nowner: kaminsod\npaths:\n- src/**\n---\n\n## Why\n\nSomething.\n",
    )
    .expect("task");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    // The warning still blocks, because only `APLAN022` was downgraded.
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("failed: 0 error(s), 1 warning(s)"), "{out}");
    assert!(out.contains("3 advisory (APLAN022)"), "{out}");

    // And with that warning's rule downgraded too, the same plan passes.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "require_cut_membership: true\ndeny_warnings: true\n\
         advisory:\n  - APLAN022\n  - APLAN009\n",
    )
    .expect("config");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 advisory (APLAN009, APLAN022)"), "{out}");
}

/// The same defect, one knob over, and it predates this task: under
/// `deny_warnings` a run with one warning and no errors exits 1, and the summary
/// said `ok`. The word is the verdict's, so both knobs get it right or neither
/// does.
/// Proves the named behaviour `verdict.summary_agrees_with_exit_code` (`A1-211`).
#[test]
fn a_denied_warning_does_not_summarise_as_ok() {
    let dir = repo("deny_warnings: true\nadvisory:\n  - APLAN022\n");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: A task that is number 1 in this fixture\n\
         phase: 1\nstatus: todo\nowner: kaminsod\npaths:\n- src/**\n---\n\n## Why\n\nSomething.\n",
    )
    .expect("task");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 1, "{out}");
    assert!(!out.contains("ok: "), "a denied run said ok:\n{out}");
    assert!(out.contains("failed: 0 error(s), 1 warning(s)"), "{out}");
    // And the reason is still stated, since `failed:` alone would not say which
    // of the two knobs produced it.
    assert!(out.contains("`deny_warnings` is set"), "{out}");
}

// ---------------------------------------------------------------------------
// What the audit of this task found: the surfaces the change did not touch.
// A policy that holds on one output and not the next is worse than no policy,
// because the surface that disagrees is the one somebody is scripting against.
// ---------------------------------------------------------------------------

/// SARIF is the format a repository uploads to a code-scanning dashboard, and it
/// was the one surface that could not tell an advisory finding from a blocking
/// one: error-level alerts, no marker, on a build that passed.
#[test]
fn sarif_suppresses_an_advisory_finding_and_keeps_its_level() {
    let dir = repo("advisory:\n  - APLAN022\n");
    let (out, code) = aplan(
        dir.path(),
        &["check", "--root", "aplan", "--format", "sarif"],
    );
    assert_eq!(code, 0, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("sarif document");
    let results = doc["runs"][0]["results"].as_array().expect("results");
    assert_eq!(results.len(), 3, "{out}");
    for r in results {
        // The level stays the rule's own: a dashboard must still learn what the
        // rule says, whatever this repository tolerates.
        assert_eq!(r["level"], "error", "{out}");
        let s = r["suppressions"].as_array().expect("suppressions");
        assert_eq!(s.len(), 1, "{out}");
        assert_eq!(s[0]["kind"], "external", "{out}");
        assert!(
            s[0]["justification"]
                .as_str()
                .expect("justification")
                .contains("advisory"),
            "{out}"
        );
    }
}

/// The control: with nothing downgraded, no result carries a suppression. A
/// document that suppressed everything would pass the test above and hide every
/// finding a repository has.
#[test]
fn sarif_suppresses_nothing_when_the_repository_downgraded_nothing() {
    let dir = repo("");
    let (out, code) = aplan(
        dir.path(),
        &["check", "--root", "aplan", "--format", "sarif"],
    );
    assert_eq!(code, 1, "{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("sarif document");
    for r in doc["runs"][0]["results"].as_array().expect("results") {
        assert!(r.get("suppressions").is_none(), "{out}");
    }
}

/// An annotation has no suppression concept, so the disclosure goes where a
/// reviewer reads it. The level stays the rule's, for the same reason as SARIF.
#[test]
fn a_github_annotation_says_an_advisory_finding_is_advisory() {
    let dir = repo("advisory:\n  - APLAN022\n");
    let (out, code) = aplan(
        dir.path(),
        &["check", "--root", "aplan", "--format", "github"],
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.lines().count(), 3, "{out}");
    for line in out.lines() {
        assert!(line.starts_with("::error "), "{out}");
        assert!(line.contains("title=APLAN022 A-"), "{out}");
        assert!(line.contains(" (advisory)::"), "{out}");
    }
}

/// The control, and the one that matters most: an ordinary finding must not
/// acquire the word, or the marker means nothing.
#[test]
fn a_github_annotation_says_nothing_extra_when_nothing_was_downgraded() {
    let dir = repo("");
    let (out, _) = aplan(
        dir.path(),
        &["check", "--root", "aplan", "--format", "github"],
    );
    assert!(!out.contains("advisory"), "{out}");
    assert!(out.contains("title=APLAN022 A-1::"), "{out}");
}

/// `verify` cannot speak honestly about a plan it could not read, so it exits 1
/// whatever the policy says. It printed `ok: 3 tasks ... evidence resolved`
/// beside that exit code the moment `APLAN001` was made advisory: a passing word
/// and a positive claim, both false, on a run that resolved nothing.
#[test]
fn verify_on_a_plan_that_did_not_load_never_reads_as_ok() {
    // `repo_with`, deliberately: with membership totality on, the three tasks
    // would each be a blocking `APLAN022` and the run would fail for a reason
    // this test is not about, which is a fixture that passes for the wrong
    // reason. Here the *only* finding is the downgraded one, so the verdict
    // turns entirely on the plan having failed to load.
    let dir = repo_with("advisory:\n  - APLAN001\n");
    std::fs::write(
        dir.path().join("aplan/tasks/BAD.md"),
        "---\nid: BAD\ntitle: a file whose frontmatter fence never closes\n",
    )
    .expect("task");
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .output()
        .expect("git init");
    let (out, code) = aplan(dir.path(), &["verify", "--root", "aplan"]);
    assert_eq!(code, 1, "a plan that did not load is not tolerable:\n{out}");
    assert!(!out.contains("ok: "), "{out}");
    // Nothing was resolved, so the line must not say it was.
    assert!(!out.contains("evidence resolved"), "{out}");
    // And the reason travels with the verdict, since a consumer reading stdout
    // alone would otherwise see `failed: 0 error(s), 0 warning(s)`.
    assert!(out.contains("failed: 1 task file(s) did not load"), "{out}");
    assert!(out.contains("1 advisory (APLAN001)"), "{out}");
}

/// The four commands that read a plan must give one answer about it. `ready`,
/// `why` and `verify` all refuse a plan whose files did not load; `check` called
/// it `ok` beside them, the moment `APLAN001` could be downgraded. A file the
/// loader could not read is not a finding a repository can be tolerant of,
/// because every other rule's silence about that file means nothing.
#[test]
fn every_command_gives_the_same_answer_about_a_plan_that_did_not_load() {
    let dir = repo_with("advisory:\n  - APLAN001\n");
    std::fs::write(
        dir.path().join("aplan/tasks/BAD.md"),
        "---\nid: BAD\ntitle: a file whose frontmatter fence never closes\n",
    )
    .expect("task");
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .output()
        .expect("git init");

    for args in [
        vec!["check", "--root", "aplan"],
        vec!["verify", "--root", "aplan"],
        vec!["ready", "--root", "aplan"],
        vec!["why", "A-1", "--root", "aplan"],
    ] {
        let (out, code) = aplan(dir.path(), &args);
        assert_eq!(code, 1, "`{}` tolerated it:\n{out}", args[0]);
    }

    // And `check` says why, rather than reporting a clean graph it built from
    // half a plan.
    let (out, _) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert!(out.contains("failed: 1 task file(s) did not load"), "{out}");
    assert!(!out.contains("DAG acyclic"), "{out}");
}

/// `close` refuses exactly what `verify` rejects (`A1-109`). It read only the
/// `deny_warnings` half of the policy, so a repository that made a rule advisory
/// got a green `verify` and a `close` that refused the same citation: the tool
/// contradicting itself about its central claim, one config key later.
#[test]
fn close_accepts_the_citation_verify_accepts_under_an_advisory_rule() {
    let dir = repo_with("advisory:\n  - APLAN012\n");
    let root = dir.path();
    for cmd in [
        vec!["init", "-q"],
        vec!["config", "user.email", "a@b.c"],
        vec!["config", "user.name", "t"],
    ] {
        Command::new("git")
            .args(&cmd)
            .current_dir(root)
            .output()
            .expect("git");
    }
    // A commit that touches none of the task's `paths`: `APLAN012`, downgraded.
    std::fs::create_dir_all(root.join("docs")).expect("docs");
    std::fs::write(root.join("docs/note.md"), "hi\n").expect("note");
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-qm", "docs only"])
        .current_dir(root)
        .output()
        .expect("git commit");
    let sha = String::from_utf8_lossy(
        &Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(root)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .trim()
    .to_string();

    let (out, code) = aplan(
        root,
        &[
            "close",
            "A-1",
            "--root",
            "aplan",
            "--evidence",
            &format!("commit:{sha}"),
        ],
    );
    assert_eq!(code, 0, "close refused what verify accepts:\n{out}");
    assert!(out.contains("closed A-1"), "{out}");
    // The preview says what it checked, and not "DAG acyclic", which it had been
    // claiming on a report that never looked at the graph.
    assert!(out.contains("this evidence resolves"), "{out}");
    assert!(!out.contains("DAG acyclic"), "{out}");

    // And the two agree: `verify` is green on what `close` just wrote.
    let (out, code) = aplan(root, &["verify", "--root", "aplan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("advisory (APLAN012)"), "{out}");
}

/// The control for the test above: with the key gone, the same citation is
/// refused, so the acceptance is the policy's doing rather than a gate that
/// stopped working.
#[test]
fn close_still_refuses_the_same_citation_without_the_key() {
    let dir = repo_with("");
    let root = dir.path();
    for cmd in [
        vec!["init", "-q"],
        vec!["config", "user.email", "a@b.c"],
        vec!["config", "user.name", "t"],
    ] {
        Command::new("git")
            .args(&cmd)
            .current_dir(root)
            .output()
            .expect("git");
    }
    std::fs::create_dir_all(root.join("docs")).expect("docs");
    std::fs::write(root.join("docs/note.md"), "hi\n").expect("note");
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-qm", "docs only"])
        .current_dir(root)
        .output()
        .expect("git commit");
    let sha = String::from_utf8_lossy(
        &Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(root)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .trim()
    .to_string();
    let (out, code) = aplan(
        root,
        &[
            "close",
            "A-1",
            "--root",
            "aplan",
            "--evidence",
            &format!("commit:{sha}"),
        ],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("refusing to close `A-1`"), "{out}");
}
