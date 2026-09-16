//! `done` over a definition of done nobody answered (`A1-224`, `APLAN038`).
//!
//! `aplan add` scaffolds `- [ ] state what done means for this task, in this
//! repository's terms`, and `close` writes the status, the evidence and the
//! fingerprints and touches nothing in the body. The tool authored the
//! obligation and never read it.
//!
//! Measured over this plan at the commit the rule was built on: of 220 closed
//! tasks, 211 fully ticked, 7 with none ticked, 2 stating none at all, and
//! **none partially ticked**. That zero is why this is a forgotten-step gate
//! rather than one that overrides a judgement: nobody is making a
//! per-criterion call and leaving one open, so a finding here is a step
//! somebody forgot rather than a decision this rule would be overriding.

use std::path::Path;

mod common;

fn plan(criteria: &str, status: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("criteria-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan")).expect("plan");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: a task\nphase: 1\nstatus: {status}\neffort_days: 1\n\
             evidence:\n  - kind: note\n    ref: done by hand\n    captured: 2026-09-03\n\
             ---\n\n## Why\n\nx\n\n## Acceptance criteria\n\n{criteria}"
        ),
    )
    .expect("task");
    dir
}

fn check(dir: &Path) -> common::Run {
    common::run(dir, &["check"])
}

/// **A `done` task with an unanswered criterion is reported, at the line.**
#[test]
fn a_done_task_with_an_unticked_criterion_is_reported() {
    let dir = plan("- [x] the first thing\n- [ ] the second thing\n", "done");
    let out = check(dir.path());

    assert!(
        out.text().contains("APLAN038"),
        "the claim contradicts the task's own terms: {}",
        out.text()
    );
    assert!(
        out.text().contains("1 of 2 criteria in its body ticked"),
        "and says how far it got: {}",
        out.text()
    );
    // Derived from the fixture rather than written here, so the assertion cannot
    // drift when the frontmatter above it changes.
    let text = std::fs::read_to_string(dir.path().join("aplan/T-1.md")).expect("read");
    let line = text
        .lines()
        .position(|l| l.trim() == "- [ ] the second thing")
        .expect("the fixture states it")
        + 1;
    assert!(
        out.text().contains(&format!("T-1.md:{line}:")),
        "at the first unanswered line ({line}), not the top of the file: {}",
        out.text()
    );
}

/// **A warning, not an error**, because an unticked box may be bookkeeping
/// rather than a false status -- and `deny_warnings` is how a repository whose
/// policy is that every gate blocks says so.
#[test]
fn it_is_a_warning_that_deny_warnings_escalates() {
    let dir = plan("- [ ] the only thing\n", "done");
    let out = check(dir.path());
    assert_eq!(
        out.code,
        0,
        "a warning alone does not fail the build: {}",
        out.text()
    );

    let denied = common::run(dir.path(), &["check", "--deny-warnings"]);
    assert_eq!(
        denied.code,
        1,
        "and a repository that wants it to block says so: {}",
        denied.text()
    );
}

/// **Silent where the body states no criteria.**
///
/// Two of this plan's closed tasks state none (`A0-002`, from before the
/// convention, and `A1-240`). A rule about a markdown convention must not
/// demand the convention: absence is `APLAN005`'s business.
#[test]
fn a_task_that_states_no_criteria_is_not_reported() {
    let dir = plan("Prose, and no checkbox anywhere.\n", "done");
    let out = check(dir.path());
    assert!(
        !out.text().contains("APLAN038"),
        "nothing was claimed, so nothing is unanswered: {}",
        out.text()
    );
}

/// **Silent on a task that is not done.** An unticked criterion on live work is
/// the normal state of a plan, and reporting it would fire on almost every task.
#[test]
fn a_live_task_with_unticked_criteria_is_the_normal_state() {
    let dir = plan("- [ ] not yet\n", "todo");
    let out = check(dir.path());
    assert!(!out.text().contains("APLAN038"), "{}", out.text());
}

/// **Silent when every criterion is ticked**, which 211 of this plan's 220
/// closed tasks already are, and the reason this gate is not noise.
#[test]
fn a_fully_answered_task_is_silent() {
    let dir = plan("- [x] one\n- [X] two\n", "done");
    let out = check(dir.path());
    assert!(!out.text().contains("APLAN038"), "{}", out.text());
}

/// **A checkbox inside a fence is quoted, not stated.**
///
/// The case `A1-224`'s own body is the plan's only instance of: it shows the
/// scaffold inside a fence while stating criteria of its own. Closing that task
/// is itself the proof, since a counter that read the fence would report it.
#[test]
fn a_fenced_checkbox_does_not_make_a_task_unanswered() {
    let dir = plan(
        "- [x] the real one\n\nThe scaffold writes:\n\n```markdown\n- [ ] state what done means\n```\n",
        "done",
    );
    let out = check(dir.path());
    assert!(
        !out.text().contains("APLAN038"),
        "the fenced line is quoted: {}",
        out.text()
    );
}

/// **Nothing ticks a criterion on the author's behalf.**
///
/// A flag that marked every criterion met because the task was closed would
/// manufacture exactly the claim this tool exists to disbelieve. Asserted
/// against the surface rather than trusted: no command offers it.
#[test]
fn no_command_offers_to_tick_a_criterion() {
    let offered: Vec<String> = aplan::cli_command()
        .get_subcommands()
        .flat_map(|c| {
            c.get_arguments()
                .filter_map(|a| a.get_long())
                .map(|l| format!("{} --{l}", c.get_name()))
                .collect::<Vec<_>>()
        })
        .filter(|s| s.contains("tick") || s.contains("criteri"))
        .collect();
    assert!(
        offered.is_empty(),
        "a flag that ticks a criterion is refused, not deferred: {offered:?}"
    );
}

/// A repository holding one live task, whose body states `criteria`, and one
/// commit touching the file that task owns. Enough for `close` to derive its own
/// evidence and for the citation to resolve.
fn repo_with(criteria: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("criteria-close-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let p = dir.path();
    std::fs::create_dir_all(p.join("aplan")).expect("plan");
    std::fs::create_dir_all(p.join("src")).expect("src");
    std::fs::write(p.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(p.join("src/a.rs"), "fn main() {}\n").expect("src file");
    std::fs::write(
        p.join("aplan/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: a task\nphase: 1\nstatus: in_progress\neffort_days: 1\n\
             paths:\n- src/**\n---\n\n## Acceptance criteria\n\n{criteria}"
        ),
    )
    .expect("task");
    for args in [
        &["init", "-q", "."][..],
        &["config", "user.email", "t@example.com"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-q", "-m", "initial"],
    ] {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(p)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
    dir
}

/// **Caught at the moment of the decision, not only at the next `check`.**
///
/// The half of `A1-224` that reads "at close": `close` compares the findings
/// before and after the edit it is about to make and reports the ones it adds,
/// so this rule reaches the author while they are standing at the closure. It
/// needed no code in `close` -- because the rule reads a counted summary of the
/// body, and `close` does not touch the body, the count is identical on both
/// sides of the mutation and only the status changes.
#[test]
fn closing_a_task_reports_its_own_unanswered_criteria() {
    let dir = repo_with("- [x] the first thing\n- [ ] the second thing\n");
    let out = common::run(dir.path(), &["close", "T-1"]);
    assert!(out.ok(), "the closure goes ahead: {}", out.text());
    assert!(
        out.text().contains("APLAN038") && out.text().contains("1 of 2"),
        "and says what it cost, at the moment of the decision: {}",
        out.text()
    );
}

/// **And refused outright where the repository's policy is that gates block.**
///
/// `deny_warnings` escalates it: the file is left as it was, so a plan under
/// that policy cannot record `done` over its own unanswered definition of done.
#[test]
fn deny_warnings_refuses_the_closure_and_writes_nothing() {
    let dir = repo_with("- [ ] the only thing\n");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\n",
    )
    .expect("config");
    let before = std::fs::read_to_string(dir.path().join("aplan/T-1.md")).expect("read");

    let out = common::run(dir.path(), &["close", "T-1"]);
    assert_eq!(out.code, 2, "refused: {}", out.text());
    assert!(
        out.text().contains("APLAN038"),
        "for this reason: {}",
        out.text()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("aplan/T-1.md")).expect("read"),
        before,
        "and the task is untouched, not closed-then-warned"
    );
}

/// **`close` leaves the body byte-identical.** Criterion four of `A1-224` is a
/// refusal, and a refusal that only means "we did not build it" is satisfied by
/// nothing. This is what holds it: whatever `close` writes, the criteria it just
/// reported on come out exactly as the author left them. A future `--tick`, or
/// a body rewrite that ticked a box in passing, fails here.
#[test]
fn close_does_not_touch_the_criteria_it_reports_on() {
    let dir = repo_with("- [x] one\n- [ ] two\n- [ ] three\n");
    let path = dir.path().join("aplan/T-1.md");
    let body_of = |s: &str| -> String {
        s.split("\n---\n")
            .nth(1)
            .expect("a body after the frontmatter")
            .to_string()
    };
    let before = body_of(&std::fs::read_to_string(&path).expect("read"));

    let out = common::run(dir.path(), &["close", "T-1"]);
    assert!(out.ok(), "{}", out.text());
    let after = std::fs::read_to_string(&path).expect("read");
    assert!(after.contains("status: done"), "it did close: {after}");
    assert_eq!(
        body_of(&after),
        before,
        "the tool reports the unanswered criteria and answers none of them"
    );
}

/// **A cut is a task too, and its exit criteria are criteria.**
///
/// Nine of this plan's own milestones head that section `## Exit criteria`
/// rather than `## Acceptance criteria`, and an adopter is free to head it
/// anything at all. The rule counts boxes and does not read the heading, so it
/// reaches all of them and its message names none of them.
#[test]
fn a_closed_cut_over_an_unticked_exit_criterion_is_reported() {
    // `T-1` closed and clean, so nothing else in this plan has anything to say
    // about `M-1` and the assertion below cannot pass on another rule's line.
    let dir = plan("- [x] the only thing\n", "done");
    std::fs::write(
        dir.path().join("aplan/M-1.md"),
        "---\nid: M-1\ntitle: a cut\nphase: 1\nstatus: done\ntags: [milestone]\n\
         blocked_by: [T-1]\nevidence:\n  - kind: note\n    ref: shipped\n    \
         captured: 2026-09-03\n---\n\n## Exit criteria\n\n- [x] one\n- [ ] two\n",
    )
    .expect("cut");
    let out = check(dir.path());
    assert!(
        out.text()
            .contains("[APLAN038] M-1: is `done` with 1 of 2 criteria in its body ticked"),
        "the cut closed over its own unmet condition: {}",
        out.text()
    );
    assert!(
        !out.text().contains("acceptance criteria"),
        "and is not told it has a section it does not have: {}",
        out.text()
    );
}

/// **The scaffold, ticked** (`A1-271`, `APLAN039`).
///
/// `aplan add` writes `- [ ] state what done means for this task, in this
/// repository's terms`. Unticked it is `APLAN038`'s business. Ticked, the task
/// asserts it met the instruction to write a criterion.
#[test]
fn ticking_the_scaffold_is_reported() {
    let dir = plan(
        &format!("- [x] {}\n", aplan_core::scaffold::criterion_prompt()),
        "done",
    );
    let out = check(dir.path());
    assert!(
        out.text().contains("APLAN039"),
        "the tool's own prompt came back as an answer: {}",
        out.text()
    );
    assert!(
        out.text().contains("asks for a criterion, and is not one"),
        "and says why that is not one: {}",
        out.text()
    );
    // `APLAN038` is silent: every box is ticked, which is the honest count.
    assert!(
        !out.text().contains("APLAN038"),
        "the two rules say different things and only one applies: {}",
        out.text()
    );
}

/// **Unticked, it belongs to `APLAN038` and this rule says nothing.**
#[test]
fn the_scaffold_left_unticked_is_the_other_rules_business() {
    let dir = plan(
        &format!("- [ ] {}\n", aplan_core::scaffold::criterion_prompt()),
        "done",
    );
    let out = check(dir.path());
    assert!(out.text().contains("APLAN038"), "{}", out.text());
    assert!(
        !out.text().contains("APLAN039"),
        "nobody claimed anything, so there is no false claim: {}",
        out.text()
    );
}

/// **A criterion that merely begins the way the prompt does is silent.**
///
/// Matching is exact after trailing whitespace and never a prefix. Guessing
/// about somebody's prose is what this rule refuses to do, and this is the
/// sentence that would catch it doing so.
#[test]
fn a_criterion_that_resembles_the_scaffold_is_not_the_scaffold() {
    let dir = plan(
        "- [x] state what done means for the *renderer*, in pixels\n\
         - [x] state what done means for this task, in this repository's terms and no others\n",
        "done",
    );
    let out = check(dir.path());
    assert!(
        !out.text().contains("APLAN039"),
        "both of those are somebody's real criteria: {}",
        out.text()
    );
}

/// **Ticked beside real criteria, still reported.**
///
/// The count is honest, so `APLAN038` is right to be quiet. The false claim is
/// recorded anyway, which is what this rule is about.
#[test]
fn the_scaffold_ticked_beside_real_criteria_is_still_a_false_claim() {
    let dir = plan(
        &format!(
            "- [x] the renderer emits SVG\n- [x] {}\n",
            aplan_core::scaffold::criterion_prompt()
        ),
        "done",
    );
    let out = check(dir.path());
    assert!(out.text().contains("APLAN039"), "{}", out.text());
}

/// **A live task carrying the scaffold is the normal state of a new plan.**
///
/// Every task `aplan add` creates carries it, ticked or not, so a rule that
/// spoke about live work would fire on the whole plan the moment it was
/// authored.
#[test]
fn a_live_task_carrying_the_scaffold_is_silent() {
    let dir = plan(
        &format!("- [x] {}\n", aplan_core::scaffold::criterion_prompt()),
        "in_progress",
    );
    let out = check(dir.path());
    assert!(!out.text().contains("APLAN039"), "{}", out.text());
}

/// **The import prompt is a prompt too** (`A1-271`'s audit).
///
/// `add` asks the author to *state* what done means for this *task*; the import
/// paths ask them to *restate* it for this *item*, because an imported item
/// arrived with words of its own. They are different sentences, a comment in
/// this repository claimed for a long time that they were the same one, and the
/// rule built on that comment knew only the first. So a task created by
/// `import` -- the path that makes tasks in bulk -- could be closed with its
/// prompt ticked and nothing said so.
#[test]
fn the_import_prompt_ticked_is_reported_too() {
    let dir = plan(
        &format!("- [x] {}\n", aplan_core::scaffold::IMPORT_CRITERION),
        "done",
    );
    let out = check(dir.path());
    assert!(
        out.text().contains("APLAN039"),
        "the other prompt this tool writes: {}",
        out.text()
    );
    assert!(
        out.text().contains(aplan_core::scaffold::IMPORT_CRITERION),
        "and the finding quotes the one it found, not the one it looked for first: {}",
        out.text()
    );
}

/// **A task quoting a prompt inside a fence is quoting it.**
///
/// `A1-224` built the fence handling for the task that showed the scaffold while
/// stating criteria of its own. `A1-271` is now the second such task in this
/// plan, and a rule that read fenced lines would fire on the task that
/// introduced it.
#[test]
fn a_prompt_inside_a_fence_is_not_a_ticked_prompt() {
    let dir = plan(
        &format!(
            "- [x] the rule refuses the prompt\n\nThe scaffold writes:\n\n```markdown\n- [x] {}\n```\n",
            aplan_core::scaffold::criterion_prompt()
        ),
        "done",
    );
    let out = check(dir.path());
    assert!(
        !out.text().contains("APLAN039"),
        "a task cannot be reported for showing the line it is about: {}",
        out.text()
    );
}
