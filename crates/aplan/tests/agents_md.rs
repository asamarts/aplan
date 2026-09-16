//! `aplan export-agents-md`, written and re-checked in a sandbox (`A1-038`).
//!
//! The corpus covers what this command *says*; these cover what it does to a
//! file, which the corpus cannot: a writing command run against a `.trycmd`
//! fixture edits the fixture, so the next run tests whatever the last one left.
//!
//! The three cases are the ones an adopter meets in order: a file that has never
//! seen this command, a file that has a block already, and a block somebody
//! edited by hand.

use std::path::Path;

mod common;

#[test]
fn a_file_with_no_markers_keeps_its_prose_and_gains_a_block() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "# AGENTS.md\n\nWhat this repository is, which `aplan` knows nothing about.\n",
    )
    .expect("write");

    let (_, stderr, ok) = run(dir.path(), &["export-agents-md", "--write", "AGENTS.md"]);
    assert!(ok, "{stderr}");
    let text = std::fs::read_to_string(dir.path().join("AGENTS.md")).expect("read");

    // Appended, never prepended: a file that opens with a generated block reads
    // as a generated file, and this one is the project's.
    assert!(
        text.starts_with("# AGENTS.md\n\nWhat this repository is"),
        "the project's own prose stays first: {text}"
    );
    assert!(
        text.contains("## The plan is the source of truth"),
        "{text}"
    );
    // Machine-independent, or the committed block disagrees with itself on the
    // next developer's clone and the gate fails for a reason nothing caused.
    assert!(
        text.contains("live under `aplan/`"),
        "the plan's location is relative to the repository: {text}"
    );
    assert!(
        !text.contains(&dir.path().display().to_string()),
        "no path from this machine reaches the file: {text}"
    );
    assert!(
        text.contains("`commit` or `test` evidence"),
        "the policy is this repository's, read from its config: {text}"
    );

    // And what it wrote is what it checks, or the gate is decorative.
    let (_, stderr, ok) = run(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert!(ok, "a file it just wrote must pass its own check: {stderr}");
}

#[test]
fn a_second_write_replaces_only_the_block() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(dir.path().join("AGENTS.md"), "before\n").expect("write");
    run(dir.path(), &["export-agents-md", "--write", "AGENTS.md"]);
    std::fs::write(
        dir.path().join("AGENTS.md"),
        format!(
            "{}\n\n## Before you push\n\nOur own checklist, added after the block.\n",
            std::fs::read_to_string(dir.path().join("AGENTS.md"))
                .expect("read")
                .trim_end()
        ),
    )
    .expect("write");

    let (_, stderr, ok) = run(dir.path(), &["export-agents-md", "--write", "AGENTS.md"]);
    assert!(ok, "{stderr}");
    let text = std::fs::read_to_string(dir.path().join("AGENTS.md")).expect("read");
    assert!(
        text.starts_with("before\n"),
        "prose before survives: {text}"
    );
    assert!(
        text.contains("## Before you push\n\nOur own checklist, added after the block."),
        "prose after survives: {text}"
    );
    assert_eq!(
        text.matches("aplan:begin").count(),
        1,
        "one block, not two: {text}"
    );
}

#[test]
fn a_hand_edited_block_is_reported_and_repaired() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(dir.path().join("AGENTS.md"), "x\n").expect("write");
    run(dir.path(), &["export-agents-md", "--write", "AGENTS.md"]);

    let text = std::fs::read_to_string(dir.path().join("AGENTS.md")).expect("read");
    std::fs::write(
        dir.path().join("AGENTS.md"),
        text.replace("A status is a claim", "A status is whatever you say"),
    )
    .expect("write");

    let (_, stderr, ok) = run(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert!(!ok, "an edit inside the markers must fail the gate");
    assert!(
        stderr.contains("--write AGENTS.md"),
        "the message names the repair: {stderr}"
    );

    run(dir.path(), &["export-agents-md", "--write", "AGENTS.md"]);
    let (_, _, ok) = run(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert!(ok, "the named repair repairs it");
}

/// A file with no markers at all is a disagreement, not a pass.
///
/// The failure this prevents: a repository adds `--check` to CI, nobody ever
/// runs `--write`, and the gate reports success forever on a file that says
/// nothing.
#[test]
fn a_file_without_the_block_fails_the_check() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(dir.path().join("AGENTS.md"), "no block here\n").expect("write");

    let (_, stderr, ok) = run(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert!(!ok, "a file with no block cannot be up to date: {stderr}");
}

/// A file this command cannot own safely is a refusal, not a repair (found by
/// this task's audit).
///
/// Two blocks were the dangerous one: `--write` updated the first and left the
/// second, and `--check` then reported the file current while the stale copy
/// went on telling agents the wrong policy. That is the failure a generated
/// artifact exists to prevent, arriving from the generator.
#[test]
fn a_file_with_two_blocks_or_half_a_block_is_refused() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let begin = "<!-- aplan:begin -- generated by `aplan export-agents-md`; \
                 edits between these markers are lost -->";
    let begin = begin.replace("                 ", "");
    let end = "<!-- aplan:end -->";

    for (name, body) in [
        (
            "two blocks",
            format!("a\n\n{begin}\nstale one\n{end}\n\nb\n\n{begin}\nstale two\n{end}\n"),
        ),
        ("half a block", format!("a\n\n{begin}\nunterminated\n")),
    ] {
        std::fs::write(dir.path().join("AGENTS.md"), &body).expect("write");

        for mode in ["--check", "--write"] {
            let (_, stderr, ok) = run(dir.path(), &["export-agents-md", mode, "AGENTS.md"]);
            assert!(!ok, "{name} must be refused by {mode}: {stderr}");
            assert!(
                stderr.contains("aplan:begin"),
                "{name}: the message must name what it found: {stderr}"
            );
        }
        // And refusing means refusing: the file is untouched, including the
        // stale block, which the author has to resolve.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("AGENTS.md")).expect("read"),
            body,
            "{name}: a refusal writes nothing"
        );
    }
}

/// A plan running two conventions keeps its contract still while work happens
/// (`A1-180`).
///
/// The block is a *policy* block -- `AgentContract`'s own doc says none of it
/// moves when a task is closed, which is what makes it gate-able. One field was
/// a tally rather than a policy: the id example took the family with the most
/// tasks, so in `p003`, where `K4-*` and `K6-*` are equally real cohorts, adding
/// one K6 task rewrote the contract in three places and failed the drift gate
/// with no id policy changed.
#[test]
fn two_live_id_families_keep_the_contract_byte_stable() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    let write = |id: &str, status: &str| {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: {status}\neffort_days: 1\n---\n\nbody\n"
            ),
        )
        .expect("task");
    };
    for i in 1..=3 {
        write(&format!("K4-00{i}"), "todo");
    }
    for i in 1..=2 {
        write(&format!("K6-00{i}"), "todo");
    }

    let (before, _, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{before}");
    assert!(
        before.contains("this plan's own conventions (`K4-nnn` or `K6-nnn`)"),
        "two cohorts are two conventions, and saying so beats picking the bigger: {before}"
    );
    // The command line needs a single word, and takes the first: a stable choice
    // because the list is keyed by stem, and the sentence above has named both.
    assert!(
        before.contains("aplan why K4-nnn"),
        "the first, not whichever happens to be last: {before}"
    );

    // Take K6 past K4, which is what flipped the old contract.
    write("K6-003", "todo");
    write("K6-004", "todo");
    let (after, _, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{after}");
    assert_eq!(before, after, "ordinary work must not move a policy block");

    // And the other way, including closing work rather than adding it.
    write("K4-004", "todo");
    write("K6-001", "done");
    let (later, _, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{later}");
    assert_eq!(before, later, "nor must closing it");

    // The gate itself, which is what actually broke in the pilot: a committed
    // contract, then ordinary work, then `--check`. Asserting the bytes match is
    // not the same as asserting the gate passes, and the gate is the thing.
    std::fs::write(dir.path().join("AGENTS.md"), &before).expect("write");
    let (out, err, ok) = run(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert!(ok, "progress must not fail the drift gate: {out}{err}");
    write("K6-005", "todo");
    let (out, err, ok) = run(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert!(ok, "nor must one more task: {out}{err}");
}

/// A live *cut* is not live work (`A1-180`, audit).
///
/// `families` excludes milestones by tag on purpose: `M-v0.7` would otherwise
/// parse as the family `M-v0.` and the next task would be offered `M-v0.8`,
/// which is a cut rather than a task. The first version of the liveness rule
/// asked the plan a second time and did not repeat that exclusion, so a live
/// milestone sharing a family's stem made the family read as current even
/// though every task in it was closed. Counted in one pass now, with one set of
/// exclusions.
#[test]
fn a_live_milestone_does_not_make_a_closed_family_current() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.path().join("aplan/tasks/K4-001.md"),
        "---\nid: K4-001\ntitle: the only K4 task, closed\nphase: 1\nstatus: done\n\
         effort_days: 1\nevidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-01-01\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.path().join("aplan/tasks/K4-900.md"),
        "---\nid: K4-900\ntitle: a live cut sharing the K4 stem\nphase: 1\nstatus: todo\n\
         tags: [milestone]\nblocked_by: [K6-001]\n---\n\nbody\n",
    )
    .expect("cut");
    std::fs::write(
        dir.path().join("aplan/tasks/K6-001.md"),
        "---\nid: K6-001\ntitle: the live work\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");

    let (out, _, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("this plan's own convention (`K6-nnn`)"),
        "the family with live work, alone: {out}"
    );
    assert!(
        !out.contains("K4-nnn"),
        "a cut is not a task, and does not keep its stem current: {out}"
    );
}

/// A stray convention is still described by the plan's main one (`A1-180`).
///
/// The majority rule was reaching for this and got it by counting: a family
/// nobody is adding to should not be named. Live work is the signal it wanted --
/// this repository's own `A0-` has two tasks and both are closed, against
/// `A1-`'s 181 -- so a closed family is silent and the single example survives,
/// which is what a predominantly single-family plan needs.
#[test]
fn a_family_with_no_live_work_is_not_named_as_a_convention() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    let write = |id: &str, status: &str, extra: &str| {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: {status}\neffort_days: 1\n{extra}---\n\nbody\n"
            ),
        )
        .expect("task");
    };
    write(
        "A0-001",
        "done",
        "evidence:\n  - kind: note\n    ref: historic\n    captured: 2026-01-01\n",
    );
    write("A1-001", "todo", "");
    write("A1-002", "todo", "");

    let (out, _, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("this plan's own convention (`A1-nnn`)"),
        "the live family, alone: {out}"
    );
    assert!(
        !out.contains("A0-"),
        "the closed family is not a convention: {out}"
    );
    assert!(
        out.contains("aplan why A1-nnn"),
        "and the command uses it: {out}"
    );

    // Where nothing is live the whole plan is history, so every family is named
    // rather than none: there is no current convention to prefer.
    write(
        "A1-001",
        "done",
        "evidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-01-01\n",
    );
    write("A1-002", "dropped", "");
    let (closed, _, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{closed}");
    assert!(
        closed.contains("conventions (`A0-nnn` or `A1-nnn`)"),
        "a finished plan names what it used: {closed}"
    );
}

fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\nevidence:\n  require_min_strength: [commit, test]\n",
    )
    .expect("config");
    std::fs::write(
        dir.join("aplan/tasks/WH2-01.md"),
        "---\nid: WH2-01\ntitle: a task\nphase: 2\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");
}

fn run(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = common::output(dir, args);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

/// The prose and the validator must draw the same boundary (`A1-157`).
///
/// The generated contract said "every task" about three rules that exempt a
/// milestone, so a valid cut container read as noncompliant -- and a literal
/// agent would "repair" it with a meaningless estimate and evidence boundary,
/// which is what the fifth pilot's audit predicted. Proved on a plan carrying
/// both shapes: the milestone declares neither field and `check` is silent about
/// it, while the ordinary task is judged.
#[test]
fn a_milestone_missing_both_fields_is_silent_and_the_prose_says_why() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\nrequire_cut_membership: true\n",
    )
    .expect("config");
    // A cut with no `paths` and no `effort_days`, which is the shape the pilot
    // found reported as noncompliant by the generated prose.
    std::fs::write(
        dir.path().join("aplan/tasks/M-s1.md"),
        "---\nid: M-s1\ntitle: A cut\nphase: 1\nstatus: todo\ntags: [milestone, s1]\n\
         blocked_by: [T-1]\n---\n\nbody\n",
    )
    .expect("milestone");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: Work\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n  - src/**\n---\n\nbody\n",
    )
    .expect("task");

    // The validator: silent about the milestone.
    let (check, err, ok) = run(dir.path(), &["check"]);
    assert!(ok, "{check}{err}");
    assert!(
        !format!("{check}{err}").contains("M-s1"),
        "the milestone is exempt: {check}{err}"
    );

    // The prose: says the same, on each of the three bullets whose rule exempts
    // one, rather than stating a stronger rule than the validator has.
    let (block, err, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{block}{err}");
    for rule in ["APLAN023", "APLAN009", "APLAN022"] {
        let bullet = bullet_citing(&block, rule);
        assert!(
            bullet.contains("not a milestone"),
            "the bullet citing {rule} states a rule the validator does not: {bullet}"
        );
    }
    assert!(
        !block.contains("- Every task declares `paths`."),
        "the universal form is gone: {block}"
    );
}

/// The whole bullet a code appears in, since a bullet wraps across lines.
fn bullet_citing(block: &str, code: &str) -> String {
    let mut current = String::new();
    for line in block.lines() {
        if line.starts_with("- ") {
            if current.contains(code) {
                return current;
            }
            current = line.to_string();
        } else {
            current.push(' ');
            current.push_str(line.trim());
        }
    }
    current
}

/// The other direction, which the scoping gate cannot see (`A1-157`'s audit).
///
/// Three bullets were scoped because their rule exempts a milestone. Two were
/// left alone -- drift (`APLAN018`) and the test citation (`APLAN013`) -- on the
/// measurement that both judge a milestone like anything else. That measurement
/// is what makes leaving them unscoped correct, so it is pinned here: if either
/// rule ever gains the exemption, its bullet is wrong and this fails, where the
/// scoping gate would stay silent because the rule never declared it.
#[test]
fn the_rules_left_unscoped_really_do_judge_a_milestone() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntest:\n  - name: fake\n    list: [printf, 'known_test\\n']\n    \
         run: [/bin/echo, '{test}']\n",
    )
    .expect("config");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "Test"],
    ] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git");
    }
    std::fs::write(dir.path().join("f.txt"), "x\n").expect("file");
    commit(dir.path());

    // A cut citing a test no runner lists. `APLAN013` has no milestone
    // exemption, so it is judged exactly like work.
    std::fs::write(
        dir.path().join("aplan/tasks/M-s1.md"),
        "---\nid: M-s1\ntitle: A cut\nphase: 1\nstatus: done\ntags: [milestone, s1]\n\
         blocked_by: [T-1]\nevidence:\n  - kind: test\n    ref: no_such_test\n---\n\nbody\n",
    )
    .expect("milestone");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: Work\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths:\n  - f.txt\n---\n\nbody\n",
    )
    .expect("task");
    commit(dir.path());

    let (out, err, _) = run(dir.path(), &["verify"]);
    let text = format!("{out}{err}");
    assert!(
        text.contains("APLAN013") && text.contains("M-s1"),
        "the test-citation rule judges a milestone, which is why its bullet is not scoped: {text}"
    );
}

fn commit(dir: &Path) {
    for args in [vec!["add", "-A"], vec!["commit", "-q", "-m", "x"]] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(dir)
            .output()
            .expect("git");
    }
}

/// Every line the generated contract puts in a shell fence parses as shell
/// (`A1-156`).
///
/// The block teaches an agent its first work loop, and its close example spelled
/// evidence as `commit:<sha>`: in an unquoted shell word `<` is redirection, so
/// the line failed at parse time and did something other than what it said. A
/// contributor copying it was stopped before this tool could report a single
/// plan finding.
///
/// Checked with `bash -n`, which parses without executing, rather than by
/// looking for characters: the question is whether a shell would accept the
/// line, and only a shell answers that.
#[test]
fn every_fenced_line_parses_as_shell() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: t\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");

    let (block, err, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{block}{err}");

    let mut inside = false;
    let mut checked = 0usize;
    for line in block.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if !inside || line.trim().is_empty() {
            continue;
        }
        checked += 1;
        let out = std::process::Command::new("bash")
            .args(["-n", "-c", line])
            .output()
            .expect("bash runs");
        assert!(
            out.status.success(),
            "a fenced line is not shell:\n  {line}\n  {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    assert!(
        checked >= 4,
        "the sweep parsed {checked} fenced line(s), so it has stopped matching"
    );
}

/// The two keys reach the contract together, from a real `.aplan.yml`
/// (`A1-272`'s audit).
///
/// The renderer is unit-tested one crate over, which proves the sentence and not
/// the wiring: `deny_warnings` was already threaded through and `advisory` was
/// not, so the contract could go on claiming there is no advisory tier for a
/// repository whose config named two rules in one. That is the same shape as the
/// defect `A1-266`'s audit found -- a policy reaching one of the two places that
/// read it -- so it is proved here, through the binary, from the file an adopter
/// actually writes.
#[test]
fn a_repository_running_both_keys_gets_a_contract_that_says_so() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\nadvisory:\n  - APLAN009\n",
    )
    .expect("config");

    let (out, err, ok) = run(dir.path(), &["export-agents-md"]);
    assert!(ok, "{err}");
    let flat = out.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(flat.contains("**Every warning blocks**"), "{out}");
    assert!(
        flat.contains("`APLAN009` is downgraded with `advisory`"),
        "the downgrade the config names: {out}"
    );
    assert!(
        !flat.contains("there is no advisory tier"),
        "this repository wrote one: {out}"
    );
}

/// **A plausible begin marker is one, and gets rewritten to the real one**
/// (`A1-294`, from pre-public adoption evidence).
///
/// Half of `contract.begin_marker_matched_by_prefix`: the accepting half.
///
/// Placing the block inside prose a repository already owns means writing the
/// markers by hand, and the opening one is a hundred characters. An adopter
/// writes `<!-- aplan:begin -->`, which reads as a begin marker to any person,
/// and was told the file had *none* -- with no way to see the difference short of
/// reading `contract.rs`.
#[test]
fn a_short_begin_marker_is_recognised_and_written_out_in_full() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let path = dir.path().join("AGENTS.md");
    std::fs::write(
        &path,
        "Our own prose above.\n\n<!-- aplan:begin -->\n<!-- aplan:end -->\n\nOur own prose \
         below.\n",
    )
    .expect("AGENTS.md");

    let (out, code) = common::aplan(dir.path(), &["export-agents-md", "--write", "AGENTS.md"]);
    assert_eq!(code, 0, "a plausible begin marker was refused: {out}");

    let after = std::fs::read_to_string(&path).expect("AGENTS.md");
    assert!(
        after.starts_with("Our own prose above.")
            && after.trim_end().ends_with("Our own prose below."),
        "the block did not land between the prose that was already there:\n{after}"
    );
    // Rewritten to the canonical marker, so the next reader of the file learns
    // that edits between the markers are lost.
    assert!(
        after.contains(aplan_output::AGENTS_BEGIN),
        "the short marker was kept, so the file no longer says what it is:\n{after}"
    );
    assert_eq!(
        after.matches("<!-- aplan:begin").count(),
        1,
        "the rewrite left two opening markers:\n{after}"
    );

    let (out, code) = common::aplan(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert_eq!(
        code, 0,
        "the file it just wrote does not pass its own gate: {out}"
    );
}

/// A file with no opening marker at all is told what one looks like, rather than
/// only how many it has.
#[test]
fn a_file_with_no_begin_marker_is_told_what_one_is() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let path = dir.path().join("AGENTS.md");
    std::fs::write(&path, "Prose.\n\n<!-- aplan:end -->\n\nMore.\n").expect("AGENTS.md");

    let (out, code) = common::aplan(dir.path(), &["export-agents-md", "--write", "AGENTS.md"]);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains(aplan_core::contract::BEGIN_PREFIX),
        "the refusal counts markers without saying what one is: {out}"
    );
    assert!(
        out.contains(aplan_output::AGENTS_BEGIN),
        "the refusal does not print the marker this command writes: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("AGENTS.md"),
        "Prose.\n\n<!-- aplan:end -->\n\nMore.\n",
        "a refusal wrote to the file"
    );
}

/// **A longer word that opens with the marker's name is the adopter's comment.**
///
/// The widening's own hazard, and it was live: `<!-- aplan:beginning of the
/// notes -->` was read as a begin marker, so `--write` replaced that line with
/// the generated block. Eating the prose is the failure every refusal in this
/// area exists to avoid, and the change meant to help introduced it
/// (`A1-294`, audit).
#[test]
fn a_word_that_starts_like_the_marker_is_not_replaced() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let path = dir.path().join("AGENTS.md");
    let before = "Prose.\n\n<!-- aplan:beginning of the notes -->\n\n<!-- aplan:end -->\n\nMore.\n";
    std::fs::write(&path, before).expect("AGENTS.md");

    let (out, code) = common::aplan(dir.path(), &["export-agents-md", "--write", "AGENTS.md"]);
    assert_ne!(code, 0, "a comment was taken for a marker: {out}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("AGENTS.md"),
        before,
        "the adopter's own comment was replaced by the generated block"
    );
}
