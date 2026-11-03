//! **A command that writes can be told the policy the gate enforces** (`A1-266`).
//!
//! `close` runs the rules against the plan it would produce and refuses a
//! mutation that adds a finding -- `mutation.refused_when_check_would_reject`,
//! whose whole point is that an author cannot close through a command what
//! `check` would then reject.
//!
//! It read its threshold from `.aplan.yml` alone. `check` and `verify` take
//! `--deny-warnings` as a *flag*, so a repository whose CI runs
//! `aplan check --deny-warnings` had a `close` applying a weaker policy than its
//! own gate: the command reported what it added and proceeded, and CI failed on
//! it. `p0000000000005` hit that twice on genuinely-complete tasks and read it as
//! one rule's ergonomics, proposing `close --criteria-met` -- which is refused
//! (`A1-224`), and is not what it wanted.

use crate::common;

use crate::common::census::{NOT_RUN, RUNS, Start, a_plan_to_act_on, plan_bytes};

use std::path::Path;

/// A live task whose body leaves one criterion unticked, so closing it adds
/// exactly one warning and nothing else.
fn one_unticked(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("src/a.rs"), "fn main() {}\n").expect("src");
    std::fs::write(
        dir.join("feature.blueprint.yml"),
        concat!(
            "schema: aplan/blueprint/v1\n",
            "name: policy\n",
            "title: Policy blueprint\n",
            "description: Exercise mutation policy.\n",
            "tasks:\n",
            "  - key: work\n",
            "    title: New work\n",
            "    effort: 1\n",
            "    paths: [src/**]\n",
        ),
    )
    .expect("blueprint");
    std::fs::write(
        dir.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: T-1\nphase: 1\nstatus: in_progress\neffort_days: 1\n\
         paths:\n- src/**\n---\n\n## Acceptance criteria\n\n- [x] one\n- [ ] two\n",
    )
    .expect("task");
    for args in [
        &["init", "-q", "."][..],
        &["config", "user.email", "t@example.com"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-q", "-m", "work"],
    ] {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }
}

/// **The flag is the whole difference.**
///
/// The same plan and the same command: without it the closure goes ahead with a
/// note, with it the closure is refused and nothing is written.
#[test]
fn the_flag_is_the_difference_between_a_note_and_a_refusal() {
    let dir = tempfile::Builder::new()
        .prefix("deny-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    one_unticked(dir.path());
    let target = dir.path().join("aplan/tasks/T-1.md");
    let before = std::fs::read_to_string(&target).expect("read");

    let refused = common::run(dir.path(), &["close", "T-1", "--deny-warnings"]);
    assert_eq!(refused.code, 2, "not refused: {refused:?}");
    assert!(
        refused.text().contains("APLAN038"),
        "and says which finding stopped it: {}",
        refused.text()
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("read"),
        before,
        "refused and wrote anyway"
    );

    let allowed = common::run(dir.path(), &["close", "T-1"]);
    assert!(
        allowed.ok(),
        "the same closure without the flag: {allowed:?}"
    );
    assert!(
        allowed.text().contains("this adds 1 finding"),
        "which still says what it cost: {}",
        allowed.text()
    );
}

/// **`--no-deny-warnings` wins over the config, here too.**
///
/// The pair is symmetric on `check` and `verify`, and a command that took only
/// half of it would be a third policy.
#[test]
fn the_config_can_be_overridden_in_both_directions() {
    let dir = tempfile::Builder::new()
        .prefix("deny-config-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    one_unticked(dir.path());
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\n",
    )
    .expect("config");

    let refused = common::run(dir.path(), &["close", "T-1"]);
    assert_eq!(
        refused.code, 2,
        "the config alone must still refuse: {refused:?}"
    );

    let allowed = common::run(dir.path(), &["close", "T-1", "--no-deny-warnings"]);
    assert!(
        allowed.ok(),
        "the flag must be able to say otherwise, as it can for `check`: {allowed:?}"
    );
}

/// The commands that run the post-state gate, and so must take the pair.
///
/// `tag` was missing from this list for the whole of its existence (`A1-356`).
/// It *had* the flags, so nothing was broken; the guarantee simply went
/// unchecked, which is worse than a failure because it looks like coverage.
const GATED: &[&str] = &[
    "add",
    "amend",
    "blueprint",
    "close",
    "dep",
    "drop",
    "mv",
    "park",
    "rekey",
    "start",
    "stop",
    "supersede",
    "tag",
    "unpark",
];

/// Commands whose ordinary mode only reports, but whose explicit mode writes
/// through the same post-state gate as [`GATED`].
///
/// Keeping these separate matters to the write census: `verify` is read-only,
/// while `verify --fix` is a mutation. Calling the whole command mutating would
/// make a read-only invocation pass that audit for the wrong reason.
const CONDITIONALLY_GATED: &[(&str, &str)] = &[(
    "verify",
    "reports by default; `--fix` atomically applies typed repairs through the post-state gate",
)];

/// Nested modes whose parent primarily reports or mutates non-task state, but
/// this named operation creates a task through the ordinary post-state gate.
const NESTED_GATED: &[(&str, &str, &str)] = &[(
    "inbox",
    "promote",
    "listing and capture-only lifecycle actions do not change the task graph; `promote` creates an ordinary task and therefore runs the complete task post-state gate",
)];

/// Commands whose ordinary invocation writes versioned intake beside the task
/// graph without changing that graph.
///
/// These writes are intentionally visible to the byte measurement below, but
/// they cannot run the task post-state gate: a capture is not a task and the
/// capture validator is its own model boundary (ADR-0063).
const INTAKE_WRITES: &[&str] = &["capture"];

/// Every other command, with why the pair does not apply to it.
///
/// Some of these do write files -- `export-agents-md --write` writes a
/// document, `import` and `init` create a plan -- and the distinction the pair
/// belongs to is narrower than "writes": it is running the rules against the
/// plan this command would produce and refusing one that adds a finding. A
/// command with no prior plan to compare against has nothing to refuse.
const UNGATED: &[(&str, &str)] = &[
    (
        "capture",
        "writes a validated versioned intake record outside the task graph, so there is no task post-state for the plan rule gate to judge",
    ),
    (
        "claim",
        "writes only private Git-common ephemeral coordination state, not a plan post-state",
    ),
    (
        "release",
        "writes only private Git-common ephemeral coordination state, including orphan recovery",
    ),
    (
        "check",
        "is the gate; it reports rather than running a post-state of its own",
    ),
    (
        "ready",
        "orders the workable tasks and reports; it changes no file",
    ),
    (
        "test",
        "lists or searches configured test evidence names and changes no file",
    ),
    (
        "due",
        "reports what a date-bearing plan owes and when; it writes nothing",
    ),
    ("why", "explains one task's position and reports"),
    ("version", "derives a version and reports"),
    (
        "graph",
        "renders the dependency graph and reports; it changes no file",
    ),
    (
        "tui",
        "is currently a read-only interactive view; later native mutations cross the same typed post-state gate",
    ),
    ("show", "renders one task and reports; it changes no file"),
    (
        "explain",
        "renders one rule's stable meaning and changes no file",
    ),
    (
        "tree",
        "renders the plan as a tree and reports; it changes no file",
    ),
    ("paths", "measures `paths` against history and reports"),
    ("vacuity", "asks whether a rule ran on anything and reports"),
    (
        "stale",
        "names work nothing has touched lately and reports; it changes no file",
    ),
    (
        "deprecations",
        "reads the runways a plan declares and reports",
    ),
    (
        "attest",
        "signs what `verify` proved; it writes an attestation, not a task",
    ),
    (
        "export-agents-md",
        "writes a document rather than a task, so no plan post-state exists",
    ),
    (
        "export-roadmap-json",
        "writes a rollup of the plan, not a change to it",
    ),
    ("export", "writes the plan out in another format"),
    ("check-docs", "compares prose against the plan and reports"),
    (
        "summary",
        "rolls up effort per phase and reports; it changes no file",
    ),
    (
        "tasks",
        "lists the plan's tasks and reports; it changes no file",
    ),
    ("critical-path", "derives the longest chain and reports"),
    (
        "roadmap",
        "rolls up the cuts and reports; the committed copy is written by xtask",
    ),
    (
        "import",
        "creates a plan from a document, one way, so there is no prior plan to \
         compare a post-state against (`A1-039`). Whether it should judge what it \
         creates the way `add` judges one task is open, and is `A1-356`'s to ask",
    ),
    (
        "init",
        "writes `.aplan.yml` and nothing else: it scaffolds the configuration, so there is \
         no task for a post-state to be about",
    ),
    (
        "mcp",
        "serves the tools, each of which is one of the commands above",
    ),
    ("facts", "prints what this build is and reports"),
    (
        "schema",
        "prints the schema this build derives from its types; it changes no file",
    ),
];

/// A command that creates a plan rather than changing one.
///
/// It writes task files and runs no post-state gate, because there is no prior
/// plan to compare against. Kept apart from [`UNGATED`] so the measurement below
/// can tell "writes and should" from "writes and should not".
///
/// `init` was listed here and is not one: the measurement below said so on its
/// first run, because `aplan init` writes `.aplan.yml` and nothing else. It
/// scaffolds the *configuration*, and a directory with no tasks in it is not a
/// plan this command made. The hand-written classification was wrong and the
/// measured one was right, which is the argument for measuring it.
const CREATES: &[&str] = &["import"];

/// **Whether a command writes the plan is measured, not declared** (`A1-356`,
/// audit).
///
/// The classification above is still written by hand, and a hand-written
/// classification is the thing this task exists to distrust. So it is checked
/// against the tool's actual behaviour: each command is run against a plan it
/// can act on, and the plan directory is compared byte for byte before and
/// after. A command that writes must be classified as one that writes, and one
/// that does not must not be -- whichever way round the disagreement is.
///
/// This cannot derive *which gate* a writing command runs, because that is not
/// observable from outside. What it removes is the larger half of the
/// hand-written claim: the set of commands that touch the plan at all.
#[test]
fn whether_a_command_writes_the_plan_is_measured_not_declared() {
    let declared_to_write: std::collections::BTreeSet<&str> = GATED
        .iter()
        .chain(CREATES.iter())
        .chain(INTAKE_WRITES.iter())
        .copied()
        .collect();
    let mut wrong = Vec::new();
    let mut ran = 0usize;

    for (name, argv, start) in RUNS {
        let dir = tempfile::Builder::new()
            .prefix("writes-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        if *start == Start::Populated {
            a_plan_to_act_on(dir.path());
        }
        // Parking deliberately fails closed unless claims share a Git-common
        // coordination scope. Give those two measurements a repository so
        // they reach their writes; the common command fixture stays neutral
        // because other audits explicitly control when Git history exists.
        if matches!(*name, "park" | "unpark") {
            common::git(dir.path(), &["init", "-q", "."]);
            common::git(dir.path(), &["config", "user.email", "t@example.invalid"]);
            common::git(dir.path(), &["config", "user.name", "t"]);
            common::git(dir.path(), &["add", "-A"]);
            common::git(dir.path(), &["commit", "-q", "-m", "fixture"]);
        }
        let before = plan_bytes(dir.path());
        let out = common::run(dir.path(), argv);
        let after = plan_bytes(dir.path());
        ran += 1;
        let wrote = before != after;
        let declared = declared_to_write.contains(name);
        if wrote != declared {
            wrong.push(format!(
                "`{name}` {} the plan, and is classified as a command that {}: `aplan {}` \
                 exited {}\n      {}",
                if wrote { "writes" } else { "does not write" },
                if declared { "does" } else { "does not" },
                argv.join(" "),
                out.code,
                out.text().lines().take(2).collect::<Vec<_>>().join(" / ")
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "{} command(s) whose classification disagrees with what they do:\n  - {}",
        wrong.len(),
        wrong.join("\n  - ")
    );
    assert!(ran > 30, "the measurement drove only {ran} command(s)");
}

/// **A conditionally mutating command is measured in both modes.**
///
/// The broad census deliberately drives `verify` without `--fix`, because that
/// is the command's ordinary, read-only meaning. This second measurement gives
/// its writing mode a real repair to apply, so neither half can pass merely
/// because both invocations happened to leave the plan untouched.
#[test]
fn verify_fix_is_the_conditionally_mutating_mode() {
    let dir = tempfile::Builder::new()
        .prefix("verify-fix-writes-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("tasks");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/a.rs"), "pub fn a() {}\n").expect("source");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [printf, 'suite::known_test\\n']\n  \
         run: ['true', '{test}']\n",
    )
    .expect("config");
    let task = dir.path().join("aplan/tasks/T-1.md");
    std::fs::write(
        &task,
        "---\nid: T-1\ntitle: repairable\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [src/**]\ntags: [cross-cutting]\nevidence:\n  - kind: test\n    ref: known_test\n\
         ---\n\nbody\n",
    )
    .expect("task");
    for args in [
        &["init", "-q", "."][..],
        &["config", "user.email", "t@example.com"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-q", "-m", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }
    let before = std::fs::read_to_string(&task).expect("read");

    let report = common::run(dir.path(), &["verify"]);
    assert_eq!(
        std::fs::read_to_string(&task).expect("read"),
        before,
        "ordinary `verify` wrote the plan: {report:?}"
    );
    assert!(
        report.text().contains("repair: replace evidence"),
        "{report:?}"
    );

    let fixed = common::run(dir.path(), &["verify", "--fix"]);
    assert!(fixed.ok(), "the computed repair was not applied: {fixed:?}");
    let after = std::fs::read_to_string(&task).expect("read");
    assert_ne!(after, before, "`verify --fix` wrote nothing: {fixed:?}");
    assert!(after.contains("ref: suite::known_test"), "{after}");
}

/// **Every command is either driven by the measurement or excused from it.**
#[test]
fn every_command_is_measured_or_says_why_not() {
    let cli = aplan::cli_command();
    let declared: std::collections::BTreeSet<String> = cli
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();
    let covered: std::collections::BTreeSet<String> = RUNS
        .iter()
        .map(|(c, _, _)| (*c).to_string())
        .chain(NOT_RUN.iter().map(|(c, _)| (*c).to_string()))
        .collect();
    let missing: Vec<&String> = declared.difference(&covered).collect();
    assert!(
        missing.is_empty(),
        "a command the measurement never runs, so nothing checks what it writes: {missing:?}"
    );
    let stale: Vec<&String> = covered.difference(&declared).collect();
    assert!(
        stale.is_empty(),
        "driven, but no longer a command: {stale:?}"
    );
    for (command, why) in NOT_RUN {
        assert!(
            why.len() > 30,
            "`{command}` is excused from the measurement with no reason worth reading: {why:?}"
        );
    }
}

/// **Every command is classified, and a new one is refused until it is**
/// (`A1-356`).
///
/// The list above used to be seven names written inline, under a doc comment
/// saying the set was "read off clap's own tree rather than listed here, so a
/// mutating command added later without them is a failure rather than an
/// omission nobody notices". Only the *flags* were read off clap. `tag` was
/// added, ran the post-state gate, and was never checked -- which is the exact
/// outcome that sentence promised to prevent.
#[test]
fn every_command_says_whether_the_pair_applies_to_it() {
    let cli = aplan::cli_command();
    let declared: std::collections::BTreeSet<String> = cli
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();
    let answered: std::collections::BTreeSet<String> = GATED
        .iter()
        .map(|c| (*c).to_string())
        .chain(CONDITIONALLY_GATED.iter().map(|(c, _)| (*c).to_string()))
        .chain(NESTED_GATED.iter().map(|(c, _, _)| (*c).to_string()))
        .chain(UNGATED.iter().map(|(c, _)| (*c).to_string()))
        .collect();

    let unclassified: Vec<&String> = declared.difference(&answered).collect();
    assert!(
        unclassified.is_empty(),
        "a command that says neither that it runs the post-state gate nor why it does not; \
         a guarantee is not carried to a new command by itself (`A1-356`): {unclassified:?}"
    );
    let stale: Vec<&String> = answered.difference(&declared).collect();
    assert!(
        stale.is_empty(),
        "classified, but no longer a command: {stale:?}"
    );
    for (command, why) in UNGATED {
        assert!(
            why.len() > 20,
            "`{command}` is excused with no reason worth reading: {why:?}"
        );
    }
    for (command, why) in CONDITIONALLY_GATED {
        assert!(
            why.len() > 20,
            "`{command}` is conditionally gated with no reason worth reading: {why:?}"
        );
    }
    for (command, mode, why) in NESTED_GATED {
        assert!(
            why.len() > 20,
            "`{command} {mode}` is nested-gated with no reason worth reading: {why:?}"
        );
    }
}

/// **Every command that writes takes the pair.**
#[test]
fn every_mutating_command_can_be_told_the_policy() {
    let mut missing = Vec::new();
    let cli = aplan::cli_command();
    for name in GATED
        .iter()
        .copied()
        .chain(CONDITIONALLY_GATED.iter().map(|(name, _)| *name))
    {
        let Some(cmd) = cli.get_subcommands().find(|c| c.get_name() == name) else {
            missing.push(format!("`{name}` is not a command any more"));
            continue;
        };
        // `dep` carries its flags on its own subcommands, which is where its
        // arguments live.
        let carriers: Vec<&clap::Command> = if cmd.has_subcommands() {
            cmd.get_subcommands().collect()
        } else {
            vec![cmd]
        };
        for c in carriers {
            let longs: Vec<&str> = c.get_arguments().filter_map(clap::Arg::get_long).collect();
            for flag in ["deny-warnings", "no-deny-warnings"] {
                if !longs.contains(&flag) {
                    missing.push(format!("`{name} {}` has no `--{flag}`", c.get_name()));
                }
            }
        }
    }
    for (name, mode, _) in NESTED_GATED {
        let Some(cmd) = cli.get_subcommands().find(|c| c.get_name() == *name) else {
            missing.push(format!("`{name}` is not a command any more"));
            continue;
        };
        let Some(carrier) = cmd.get_subcommands().find(|c| c.get_name() == *mode) else {
            missing.push(format!("`{name} {mode}` is not a command any more"));
            continue;
        };
        let longs: Vec<&str> = carrier
            .get_arguments()
            .filter_map(clap::Arg::get_long)
            .collect();
        for flag in ["deny-warnings", "no-deny-warnings"] {
            if !longs.contains(&flag) {
                missing.push(format!("`{name} {mode}` has no `--{flag}`"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "a command that writes cannot be told the policy its repository's gate enforces, so it \
         refuses less than `check` would (`A1-266`):\n  {}",
        missing.join("\n  ")
    );
}

/// **Each command actually accepts the pair**, not just declares it.
///
/// The audit above reads clap's tree, which proves the flags are *declared*.
/// This runs them, which proves they parse where a user would type them -- and
/// it is what the CLI-surface audit counts, since a flag nothing ever invokes
/// is a flag nobody has tried.
///
/// The outcome is deliberately not asserted: several of these refuse for
/// reasons of their own on this fixture, which is fine. What must never happen
/// is clap rejecting the flag itself.
#[test]
fn each_mutating_command_accepts_the_pair_when_run() {
    let dir = tempfile::Builder::new()
        .prefix("accepts-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    one_unticked(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 1\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [T-1]\n---\n\nbody\n",
    )
    .expect("milestone");

    let runs: &[(&str, &[&str])] = &[
        ("close", &["close", "T-1"]),
        (
            "blueprint",
            &[
                "blueprint",
                "apply",
                "--file",
                "feature.blueprint.yml",
                "--cut",
                "M-a",
            ],
        ),
        (
            "amend",
            &["amend", "T-1", "--add-evidence", "file:src/a.rs"],
        ),
        ("supersede", &["supersede", "T-1", "the next one"]),
        ("add", &["add", "a new task", "--effort", "1"]),
        ("drop", &["drop", "T-1"]),
        ("mv", &["mv", "T-1", "--cut", "M-a"]),
        ("park", &["park", "T-1", "--reason", "later"]),
        ("unpark", &["unpark", "T-1"]),
        ("rekey", &["rekey", "T-1", "T-9"]),
        ("dep", &["dep", "add", "T-1", "--after", "M-a"]),
        ("dep", &["dep", "rm", "T-1", "--after", "M-a"]),
        ("start", &["start", "T-1"]),
        ("stop", &["stop", "T-1"]),
        ("tag", &["tag", "add", "T-1", "kept"]),
        ("verify", &["verify", "--fix"]),
        (
            "inbox",
            &["inbox", "promote", "I-01K8Z3A4H00000000000000000"],
        ),
    ];
    let expected: std::collections::BTreeSet<&str> = GATED
        .iter()
        .copied()
        .chain(CONDITIONALLY_GATED.iter().map(|(name, _)| *name))
        .chain(NESTED_GATED.iter().map(|(name, _, _)| *name))
        .collect();
    let driven: std::collections::BTreeSet<&str> = runs.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        driven, expected,
        "the runtime policy-flag probes drifted from the classified commands"
    );
    for (_, base) in runs {
        for flag in ["--deny-warnings", "--no-deny-warnings"] {
            let mut argv: Vec<&str> = base.to_vec();
            argv.push(flag);
            let out = common::run(dir.path(), &argv);
            assert!(
                !out.text().contains("unexpected argument"),
                "`aplan {}` rejected the flag itself: {}",
                argv.join(" "),
                out.text()
            );
        }
    }
}

/// Proof for `coordination.is_not_a_plan_verdict_mutation`: the flags removed
/// from claim and release cannot suggest that changing local coordination ran
/// or overrode the repository's plan gate.
#[test]
fn local_coordination_commands_refuse_plan_verdict_overrides() {
    let dir = tempfile::Builder::new()
        .prefix("coordination-verdict-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    one_unticked(dir.path());

    for argv in [
        &["claim", "T-1", "--deny-warnings"][..],
        &["release", "T-1", "--no-deny-warnings"][..],
    ] {
        let out = common::run(dir.path(), argv);
        assert_eq!(out.code, 2, "`aplan {}`: {}", argv.join(" "), out.text());
        assert!(
            out.text().contains("unexpected argument"),
            "`aplan {}` did not explicitly refuse the inapplicable plan-verdict flag: {}",
            argv.join(" "),
            out.text()
        );
    }
}

/// **`close` runs two gates, and the flag reaches both** (`A1-266`'s audit).
///
/// The post-state rules judge the plan a write would produce; the evidence
/// ladder judges the citation. `A1-112` already argued that a preview rendered
/// under a different policy from the one deciding the refusal is a second
/// opinion nobody asked for -- and the first pass at this task threaded the flag
/// into one gate and left the other reading the config key alone. So the same
/// command answered two ways about the same word: a diluted commit was refused
/// under `deny_warnings: true` and closed under `--deny-warnings`.
#[test]
fn the_flag_reaches_the_evidence_gate_and_not_only_the_post_state_one() {
    let dir = tempfile::Builder::new()
        .prefix("two-gates-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    // Sixty files in one commit, and a task owning one of them: `APLAN024`
    // reports the citation as diluted, which is a *warning* -- the only
    // severity where the flag can be the difference.
    for i in 0..60 {
        std::fs::write(dir.path().join(format!("src/f{i}.rs")), "fn f() {}\n").expect("src");
    }
    std::fs::write(dir.path().join("src/a.rs"), "fn main() {}\n").expect("src");
    for args in [
        &["init", "-q", "."][..],
        &["config", "user.email", "t@example.com"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-q", "-m", "a large commit"],
    ] {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git");
    let sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: narrow\nphase: 1\nstatus: in_progress\neffort_days: 1\n\
         paths:\n- src/a.rs\n---\n\n## Acceptance criteria\n\n- [x] one\n",
    )
    .expect("task");

    let cite = format!("commit:{sha}");
    let refused = common::run(
        dir.path(),
        &["close", "T-1", "--evidence", &cite, "--deny-warnings"],
    );
    // `1`, not `2`: the exit contract is that `1` means at least one finding,
    // and this gate found one. The post-state gate *bails* instead and lands on
    // `2`, which is the usage code -- so a script telling the two apart sees a
    // different number depending on which gate stopped the same command. Both
    // are non-zero and both mean refused; changing either is a consumer
    // contract change and not this task's. Asserted rather than smoothed over,
    // so the difference is written down where somebody meets it.
    assert_eq!(
        refused.code, 1,
        "the evidence warning did not block: {refused:?}"
    );
    assert!(
        refused.text().contains("APLAN024"),
        "and says which finding stopped it: {}",
        refused.text()
    );
    let task = std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read");
    assert!(
        !task.contains("status: done"),
        "refused and closed anyway: {task}"
    );

    // Without it the same citation records, so the flag is the difference here
    // exactly as it is on the other gate.
    let allowed = common::run(dir.path(), &["close", "T-1", "--evidence", &cite]);
    assert!(allowed.ok(), "{allowed:?}");
}
