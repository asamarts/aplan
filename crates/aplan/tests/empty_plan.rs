//! The first command of an adoption, on a plan that has nothing in it
//! (`A1-252`).
//!
//! Two pilots stopped here, for two different reasons, and each log judged its
//! own instance too small to file. `p0000000000005` hit the id convention: the
//! first task cannot imply one out of nothing, and its own `AGENTS.md` example
//! omitted `--id` (item 1). `p00002` hit the one after it: under
//! `require_cut_membership`, `APLAN022` says to add the task to a milestone's
//! `blocked_by`, and there is no milestone (item 1).
//!
//! Both refusals are right. What neither said is the level above: this plan is
//! empty, and here is the first thing to do about it. An empty plan is a state
//! the tool can detect exactly, which is what makes this a message rather than a
//! model change.

use std::path::Path;

mod common;

fn empty_plan(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("empty-plan-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("config");
    dir
}

const STRICT: &str = "root: aplan\nrequire_cut_membership: true\n";

/// **A refusal that says "add it to a milestone" says so when there is none.**
///
/// The fixture is an empty plan on purpose: the note exists only for that state,
/// so a test against a plan with cuts would pass whatever the message said.
#[test]
fn the_first_add_is_told_the_plan_has_no_cut_yet() {
    let dir = empty_plan(STRICT);

    let out = common::run(
        dir.path(),
        &["add", "a first task", "--id", "A-001", "--effort", "1"],
    );
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("APLAN022"),
        "the rule still reports itself: {}",
        out.text()
    );
    assert!(
        out.text().contains("no cut yet"),
        "and the plan-level fact the rule cannot see is said too: {}",
        out.text()
    );
    assert!(
        out.text().contains("--tag milestone"),
        "naming how a cut is made, since that is what does not exist: {}",
        out.text()
    );
}

/// **The path the message gives works when typed as written.**
///
/// A remedy nobody ran is advice, and this is the one an adopter meets on their
/// first command. Both steps come out of the sentence above.
#[test]
fn the_remedy_the_message_names_produces_a_clean_plan() {
    let dir = empty_plan(STRICT);

    let cut = common::run(
        dir.path(),
        &["add", "the first cut", "--id", "M-s0", "--tag", "milestone"],
    );
    assert_eq!(cut.code, 0, "{}", cut.text());

    let work = common::run(
        dir.path(),
        &[
            "add",
            "a first task",
            "--id",
            "A-001",
            "--effort",
            "1",
            "--cut",
            "M-s0",
        ],
    );
    assert_eq!(work.code, 0, "{}", work.text());

    let check = common::run(dir.path(), &["check"]);
    assert_eq!(
        check.code,
        0,
        "two commands out of one message, and the plan is clean: {}",
        check.text()
    );
}

/// **A plan that has a cut is not told how to make one.**
///
/// The other half, and the one that keeps the first from becoming noise: on a
/// plan with cuts, "add it to one" is complete advice and a second paragraph
/// would be repeated on every refusal forever.
#[test]
fn a_plan_that_has_cuts_gets_no_extra_paragraph() {
    let dir = empty_plan(STRICT);
    let made = common::run(
        dir.path(),
        &["add", "the first cut", "--id", "M-s0", "--tag", "milestone"],
    );
    assert_eq!(made.code, 0, "{}", made.text());

    let out = common::run(
        dir.path(),
        &["add", "another", "--id", "A-002", "--effort", "1"],
    );
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("APLAN022"),
        "still refused: {}",
        out.text()
    );
    assert!(
        !out.text().contains("no cut yet"),
        "and not told how to make what it already has: {}",
        out.text()
    );
}

/// **The id refusal already says what to do, and this pins that it keeps doing
/// it.**
///
/// `p0000000000005`'s half. The message was judged good at the time and the
/// documentation was the gap; the sentence is worth a test anyway, because it is
/// the first thing an adopter reads from this tool.
#[test]
fn the_first_add_without_an_id_says_to_pass_one() {
    let dir = empty_plan("root: aplan\n");

    let out = common::run(dir.path(), &["add", "a first task"]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("--id"),
        "the first task cannot imply a convention, and the message says the flag: {}",
        out.text()
    );
}

/// The README is where a reader meets `add` first, and its first example is now
/// the first-task form.
#[test]
fn the_documented_first_add_carries_an_id() {
    let readme =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"))
            .expect("README");
    let first = readme
        .lines()
        .find(|l| l.starts_with("aplan add "))
        .expect("the README shows `aplan add`");
    assert!(
        first.contains("--id"),
        "an example that omits `--id` is the one that fails on an empty plan, and it is the \
         first one a reader copies: {first}"
    );
}

/// **`supersede` gets the same sentence**, which is what makes the shared
/// refusal the right home for it.
///
/// The note went into `refuse_bad_mutation` rather than into `add` on the
/// reasoning that another command reaches the same state. That was asserted and
/// not shown, and the case first cited for it was the wrong one: `p003`'s
/// `supersede` refusal is `APLAN022` on a plan that *has* cuts, where this note
/// is deliberately silent (`A1-251`). This is the case that does reach it -- a
/// plan with no milestone at all -- and it is the same first-adoption shape
/// (`A1-252`, audit).
#[test]
fn supersede_on_a_plan_with_no_cut_is_told_the_same_thing() {
    let dir = empty_plan(STRICT);
    let repo = dir.path();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(repo)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }
    std::fs::write(repo.join("f.txt"), "x\n").expect("file");
    for args in [vec!["add", "-A"], vec!["commit", "-qm", "init"]] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(repo)
            .output()
            .expect("git");
    }
    let sha = String::from_utf8_lossy(
        &std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .trim()
    .to_string();
    // `cross-cutting` so the predecessor itself satisfies APLAN022: the finding
    // under test has to be one this mutation *adds*.
    std::fs::write(
        repo.join("aplan/A-001.md"),
        format!(
            "---\nid: A-001\ntitle: done, in no cut\nphase: 1\nstatus: done\neffort_days: 1\n\
             paths: [\"f.txt\"]\ntags: [cross-cutting]\nevidence:\n  - kind: commit\n    \
             ref: {sha}\n---\n\n## Why\n\nx\n"
        ),
    )
    .expect("predecessor");

    let out = common::run(repo, &["supersede", "A-001", "the successor"]);
    assert_eq!(out.code, 2, "{}", out.text());
    assert!(
        out.text().contains("no cut yet"),
        "a command that is not `add` reaches this state, which is why the note is not in \
         `add`: {}",
        out.text()
    );
}

/// **`import` says it too, in its own words.**
///
/// Its notice named `--cut <milestone>` to a plan that has no milestone. Not a
/// refusal, so it does not pass through the shared path, and the checklist
/// importer is exactly what an adopter with an existing roadmap runs first
/// (`A1-252`, audit).
#[test]
fn import_into_a_plan_with_no_cut_says_there_is_none() {
    let dir = empty_plan(STRICT);
    std::fs::write(
        dir.path().join("list.md"),
        "# Roadmap\n\n## Phase\n\n- [ ] first item\n",
    )
    .expect("checklist");

    let out = common::run(
        dir.path(),
        &[
            "import",
            "list.md",
            "--format",
            "checklist",
            "--id-prefix",
            "Z",
            "--dry-run",
        ],
    );
    assert!(
        out.text().contains("this plan has none yet"),
        "{}",
        out.text()
    );

    // And with a cut present, the advice that names one is right again.
    std::fs::write(
        dir.path().join("aplan/M-1.md"),
        "---\nid: M-1\ntitle: a cut\nphase: 1\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [Z-001]\n---\n\n## Why\n\nx\n",
    )
    .expect("cut");
    let out = common::run(
        dir.path(),
        &[
            "import",
            "list.md",
            "--format",
            "checklist",
            "--id-prefix",
            "Y",
            "--dry-run",
        ],
    );
    assert!(
        out.text().contains("`--cut <milestone>`") && !out.text().contains("none yet"),
        "{}",
        out.text()
    );
}

/// **The invocation this message names is one the CLI accepts.**
///
/// `A1-089` sweeps consumer strings for exactly this and does not see this one:
/// its scan reads the arguments of known call sites (`bail!`, `Diagnostic::error`,
/// `eprintln!`), and a message built in a helper and interpolated into one is
/// outside that. Renaming `--id` to a flag that does not exist left every gate
/// green, which is how this was found (`A1-252`, audit; the general gap is
/// `A1-255`).
///
/// Checked against the *emitted* message rather than the source, which is the
/// stronger reading: it is what a reader actually copies.
#[test]
fn the_invocation_the_message_offers_is_one_the_cli_accepts() {
    let dir = empty_plan(STRICT);
    let out = common::run(
        dir.path(),
        &["add", "a first task", "--id", "A-001", "--effort", "1"],
    );

    let mut checked = 0;
    let mut rest = out.text();
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        let span = after[..close].to_string();
        rest = after[close + 1..].to_string();
        if !span.starts_with("aplan ") {
            continue;
        }
        checked += 1;
        let argv = common::argv_of(&span);
        if let Err(e) = aplan::cli_command().try_get_matches_from(&argv) {
            use clap::error::ErrorKind::{InvalidSubcommand, UnknownArgument, ValueValidation};
            assert!(
                !matches!(
                    e.kind(),
                    UnknownArgument | InvalidSubcommand | ValueValidation
                ),
                "the refusal offers `{span}`, which the CLI rejects: {}",
                e.to_string().lines().next().unwrap_or_default()
            );
        }
    }
    assert!(
        checked > 0,
        "the message named no invocation, so this passes vacuously: {}",
        out.text()
    );
}
