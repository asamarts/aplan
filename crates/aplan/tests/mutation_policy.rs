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

mod common;

use std::path::Path;

/// A live task whose body leaves one criterion unticked, so closing it adds
/// exactly one warning and nothing else.
fn one_unticked(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("src/a.rs"), "fn main() {}\n").expect("src");
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

/// **Every command that writes takes the pair.**
///
/// Read off clap's own tree rather than listed here, so a mutating command added
/// later without them is a failure rather than an omission nobody notices. The
/// set is the commands that run the post-state gate.
#[test]
fn every_mutating_command_can_be_told_the_policy() {
    let writes = ["close", "amend", "supersede", "add", "drop", "mv", "dep"];
    let mut missing = Vec::new();
    let cli = aplan::cli_command();
    for name in writes {
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

    let runs: &[&[&str]] = &[
        &["close", "T-1"],
        &["amend", "T-1", "--add-evidence", "file:src/a.rs"],
        &["supersede", "T-1", "the next one"],
        &["add", "a new task", "--effort", "1"],
        &["drop", "T-1"],
        &["mv", "T-1", "--cut", "M-a"],
        &["dep", "add", "T-1", "--after", "M-a"],
        &["dep", "rm", "T-1", "--after", "M-a"],
    ];
    for base in runs {
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
