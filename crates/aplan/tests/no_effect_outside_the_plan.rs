//! **A mutation causes nothing outside the plan** (`A1-354`, ADR-0043).
//!
//! ADR-0043 decided that closing a task causes nothing outside the plan: a
//! mutation writes plan files, and spawns only the observing commands trusted
//! configuration names. That was true when it was decided, and **nothing would
//! have noticed it stopping**. A key such as `on_close.command`, or a spawn
//! written into `close`'s resolution path by hand, would have left every test in
//! this workspace green, which is the same shape of hole `policy.rs` describes
//! about the `untestable` exemption: a policy that can be relaxed silently is
//! not a policy.
//!
//! **Two halves, because each is blind where the other sees.**
//!
//! [`every_process_production_starts_is_named_with_its_reason`] reads every
//! crate's production sources and names every place one of them starts a
//! process. It catches a spawn written in by hand, which no configuration would
//! mention and no fixture would trigger. It cannot say what is *reachable from*
//! a mutation, and a scan claiming to would be guessing at a call graph.
//!
//! [`which_configured_commands_a_command_reaches_is_measured_not_declared`]
//! runs every command against configuration that names a recording script in
//! every place configuration can name a command, and compares what each command
//! reached against what it is allowed to reach. It catches what the scan
//! structurally cannot see: `on_close.command` adds no spawn site, because the
//! spawner already exists. It gives `close` a new reason to call it.
//!
//! **What separates an observation from an effect**, in terms a gate can apply:
//! not "it only reads", which is prose, but *what ran*. Every script the fixture
//! installs writes a line when it is executed, so the measurement is of
//! execution rather than of intent. `close` is allowed to execute one of them,
//! `test.list`, and the reason is that it asks which tests exist in order to
//! **refuse** a citation `verify` would reject. An observation made in order to
//! say no is still an observation; the gate does not have to guess that, because
//! the allowance is written next to the command with that reason.
//!
//! **`verify --execute` is in scope and is the control.** It runs the
//! repository's tests, which do anything at all, and it is off by default for
//! exactly that reason. Listing it as an exemption and leaving it there would
//! make this file's central claim unfalsifiable, so it is the one command
//! required to reach `test.run`: it proves the recording channel is live, which
//! is what stops every "reached nothing" above from being a measurement of a
//! broken fixture.

mod common;

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- the scan

/// Every place production code starts a process, and why each is an observation
/// rather than an effect.
///
/// Three, and the list is the argument. Two are the configured-command path
/// itself and one is not, which is the entry worth reading: a mutation does
/// reach a `git` subprocess, and ADR-0043's words ("spawns only the observing
/// commands trusted config names") do not cover it, because nothing configured
/// names it.
const SPAWNS: &[(&str, &str, &str)] = &[
    (
        "crates/aplan-evidence/src/resolve.rs",
        "spawn_in_cage",
        "the one place a command named by trusted configuration is run (`A1-310`). Every \
         configured spawn in the tool funnels through here, which is why the cgroup caps could be \
         installed in one place at all -- and why the measurement below can see all of them by \
         recording what the configured commands do rather than by instrumenting callers",
    ),
    (
        "crates/aplan-evidence/src/resolve.rs",
        "kill_group",
        "stopping what `spawn_in_cage` started, when a command outlives its timeout (`A1-310`). \
         It spawns `sh` to signal a process group, which is an effect on a process this tool \
         created and on nothing else: the alternative is leaving an adopter's test runner behind \
         after the run that started it has gone",
    ),
    (
        "crates/aplan-attest/build.rs",
        "git_describe",
        "asking `git` for the commit this binary was built from, at build time (`A1-154`). It is \
         not reached by any command, because it has already run before there is a binary to run \
         one with: an adopter compiles it, and what it learns is embedded. It is in this list \
         because a walk of `*/src` is not a walk of what ships, and leaving it out would have \
         made that omission invisible rather than argued",
    ),
    (
        "crates/aplan-interop/src/identity.rs",
        "git_user_name",
        "reading `git config user.name` to answer who is acting, when `--as`, `APLAN_OWNER` and \
         the per-user file have not said (`A1-359`, `A1-364`). This is the one on a mutation's \
         path -- `claim` resolves who is acting, on this surface and on the MCP one -- and it is an \
         observation in the strict sense: it reads a value, and the only thing it can change is the text this tool \
         writes into the plan. It is also the one spawn nothing configured names, so the \
         measurement below cannot see it and this list is the only place it is accounted for",
    ),
];

/// Markers for a line that starts a process.
///
/// `Command::new` is the chokepoint: this workspace forbids `unsafe`, so
/// `fork`/`exec` through `libc` is not available to it, and a `Command` has to
/// be constructed before it can be spawned. The rest are here so that a future
/// route in is a failure rather than an omission -- `CommandExt` is the unix
/// extension trait, and `duct` and `subprocess` are the crates a contributor
/// would reach for instead of the standard library.
const STARTS_A_PROCESS: &[&str] = &["Command::new", "CommandExt", "duct::", "Exec::"];

/// **No test code survives into the production view** (`A1-354`, audit).
///
/// A `#[test]` attribute only ever appears inside a `#[cfg(test)]` item, so one
/// reaching the production view means the skip ended early and the scan is
/// reading fixtures as if they were the product. Both bugs the first two
/// attempts at `common::sources::production_lines` had -- a `}` inside a string
/// literal, and a `";` at column 0 -- would have been caught here. Each was
/// instead found by a gate failing on a fixture, which is the slow way round.
#[test]
fn no_test_code_survives() {
    let root = common::sources::workspace_root();
    let mut leaked = Vec::new();
    let mut scanned = 0usize;
    for path in common::sources::all_sources(&root) {
        let text = std::fs::read_to_string(&path).expect("read a source file");
        scanned += 1;
        for (n, line) in common::sources::production_lines(&text) {
            if line.trim_start().starts_with("#[test]") {
                leaked.push(format!("{}:{n}", path.display()));
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "{} `#[test]` attribute(s) read as production code, so a `#[cfg(test)]` item was not \
         skipped and both source scans are reading fixtures:\n  - {}",
        leaked.len(),
        leaked.join("\n  - ")
    );
    assert!(scanned > 30, "the walk read only {scanned} file(s)");
}

/// **The shapes that fooled the first two attempts** (`A1-354`, audit).
#[test]
fn the_production_view_is_not_fooled_by_string_literals() {
    let source = concat!(
        "fn before() {}\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    #[test]\n",
        "    fn a() {\n",
        "        let yaml = \"root: aplan\n",
        "}\n",
        "\";\n",
        "    }\n",
        "}\n",
        "fn after() {}\n",
    );
    let kept: Vec<&str> = common::sources::production_lines(source)
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    assert!(
        kept.contains(&"fn before() {}") && kept.contains(&"fn after() {}"),
        "a production section after a test module must be read: {kept:?}"
    );
    assert!(
        !kept.iter().any(|l| l.contains("#[test]")),
        "the test module's own lines must not be read: {kept:?}"
    );
}

#[test]
fn every_process_production_starts_is_named_with_its_reason() {
    let root = common::sources::workspace_root();
    let mut unnamed = Vec::new();
    let mut found: BTreeSet<(String, String)> = BTreeSet::new();
    let mut scanned = 0usize;

    for path in common::sources::shipped_sources(&root) {
        let text = std::fs::read_to_string(&path).expect("read a source file");
        scanned += 1;
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let mut current = String::new();
        // Production only, and **all** of it. A fixture spawning `sh` to build
        // its own temp state is not the hazard; a second production section
        // after a test module is, and splitting the file at the first
        // `#[cfg(test)]` hid half of `aplan-output` from this gate until its
        // audit (`A1-354`, audit).
        for (n, line) in common::sources::production_lines(&text) {
            if let Some(name) = common::sources::enclosing_fn(line) {
                current = name.to_string();
            }
            if common::sources::is_comment(line) {
                continue;
            }
            if STARTS_A_PROCESS.iter().any(|m| line.contains(m)) {
                found.insert((relative.clone(), current.clone()));
                if !SPAWNS
                    .iter()
                    .any(|(f, fun, _)| *f == relative && *fun == current)
                {
                    unnamed.push(format!("{relative}:{n} in `{current}`: {}", line.trim()));
                }
            }
        }
    }

    assert!(
        unnamed.is_empty(),
        "{} place(s) where production code starts a process without saying why it is an \
         observation rather than an effect (ADR-0043, `A1-354`). Add it to `SPAWNS` with the \
         argument, or do not spawn:\n  - {}",
        unnamed.len(),
        unnamed.join("\n  - ")
    );

    // A scan that finds nothing passes, which is the failure mode this gate is
    // most likely to acquire: a rename, a moved directory, a walk that stopped
    // recursing. Both halves are asserted -- that the walk saw the workspace,
    // and that every named site is still there.
    assert!(
        scanned > 30,
        "the scan read only {scanned} production source file(s), so it cannot have seen the \
         workspace"
    );
    for (file, function, why) in SPAWNS {
        assert!(
            found.contains(&((*file).to_string(), (*function).to_string())),
            "`{function}` in {file} is named as a spawn and the scan cannot find it: a renamed \
             function leaves this list naming nothing, which reads as coverage"
        );
        assert!(
            why.len() > 80,
            "`{function}` is excused with no argument worth reading: {why:?}"
        );
    }
}

// --------------------------------------------------------- the measurement

/// The commands allowed to reach something configuration named, and why.
///
/// Everything else must reach nothing. Stated this way round because the
/// property is about the exceptions: a list of what each of forty commands may
/// do would be a list somebody typed, while "nothing, except these two, for
/// these reasons" is a sentence that can be argued with.
const ALLOWED_TO_REACH: &[(&str, &[&str], &str)] = &[
    (
        "verify",
        &[
            "version.check",
            "version.check.wrap",
            "version.current",
            "version.current.wrap",
        ],
        "judging what the plan says about the released version means asking the artifact what \
         version it ships and whether it is still compatible (`A1-312`, `A1-313`, ADR-0034). Both \
         are questions; neither is told anything. Resolving a `test` citation reaches `test.list` \
         as well, which this fixture's plan does not exercise because its closed task cites a \
         note -- the test-citation path has a test of its own below",
    ),
    (
        "version",
        &["version.current", "version.current.wrap"],
        "its entire output is the version the artifact ships beside the one the plan implies, so \
         asking the artifact is the command, not a side effect of it",
    ),
    (
        "attest",
        &[
            "version.check",
            "version.check.wrap",
            "version.current",
            "version.current.wrap",
        ],
        "an attestation records what was true of the artifact at a commit, and the version it \
         ships is part of that (`A1-197`). It reads the same two answers `verify` reads, writes \
         nothing itself, and is classified in `mutation_policy.rs` among the commands that report",
    ),
];

/// Commands that both changed the plan and ran something configuration named.
///
/// **Empty, and that is the claim.** Of the twelve commands that write the plan
/// -- `add`, `amend`, `claim`, `close`, `dep`, `drop`, `mv`, `release`, `start`,
/// `stop`, `supersede`, `tag` -- not one reaches a configured command as the
/// census drives it. The three entries above are all reporting commands, which
/// is ADR-0043's sentence turned into a measurement rather than a reading of
/// the code.
///
/// The one case that would belong here is `close` citing a *test*: it asks
/// `test.list` in order to refuse a citation `verify` would reject, and it
/// writes. The census closes on a `file:` citation instead, so it does not
/// arise here, and that path has a test of its own
/// ([`the_one_observation_close_makes_is_the_one_it_is_for`]) where the refusal
/// is asserted beside the spawn. A future entry needs that same pairing: the
/// spawn, and the "no" it exists to say.
const WRITES_AND_OBSERVES: &[(&str, &str)] = &[];

/// A command the measurement cannot drive, and why.
const CANNOT_BE_MEASURED: &[(&str, &str)] = &[(
    "init",
    "its work is to create the configuration, so it refuses to run against a directory that \
     already has one. Measuring it would mean handing it the file whose absence it requires, and \
     it would refuse for the wrong reason and read as a command that reached nothing",
)];

/// What one measured run saw.
struct Wiring {
    log: PathBuf,
    config: String,
}

/// Name a recording script in every place configuration can name a command.
///
/// Each script writes the name of the key that named it and then does the least
/// that key's caller will accept -- `list` prints a test name, `current` prints
/// a version, a `wrap` execs what it was given -- so a command that reaches one
/// is recorded without the run failing for a reason unrelated to the question.
fn wire(support: &Path) -> Wiring {
    let log = support.join("reached.log");
    let shown = log.display().to_string();
    let script = |name: &str, body: &str| {
        let path = support.join(format!("{name}.sh"));
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{name}' >> '{shown}'\n{body}\n"),
        )
        .expect("write a recording script");
        path.display().to_string()
    };
    // A `wrap` is a prefix rather than a command of its own, so its script has
    // to run what it was handed or the spawn it wraps never happens.
    let list = script("test.list", "echo the_only_test");
    let run = script("test.run", "exit 0");
    let skipped = script("test.skipped", "exit 0");
    let wrap = script("test.wrap", "exec \"$@\"");
    let url = script("url.check", "exit 0");
    let url_wrap = script("url.wrap", "exec \"$@\"");
    let current = script("version.current", "echo 0.1.0");
    let current_wrap = script("version.current.wrap", "exec \"$@\"");
    let check = script("version.check", "exit 0");
    let check_wrap = script("version.check.wrap", "exec \"$@\"");
    let config = format!(
        "root: aplan\n\
         test:\n  \
           list: [sh, {list}]\n  \
           run: [sh, {run}, '{{test}}']\n  \
           skipped: [sh, {skipped}]\n  \
           wrap: [sh, {wrap}]\n\
         url:\n  \
           check: [sh, {url}, '{{url}}']\n  \
           wrap: [sh, {url_wrap}]\n\
         version:\n  \
           current:\n    \
             command: [sh, {current}]\n    \
             wrap: [sh, {current_wrap}]\n  \
           check:\n    \
             command: [sh, {check}]\n    \
             wrap: [sh, {check_wrap}]\n"
    );
    Wiring { log, config }
}

/// Commit the fixture, so evidence has something to resolve against.
fn as_a_repository(dir: &Path) {
    common::git(dir, &["init", "-q", "."]);
    common::git(dir, &["config", "user.email", "t@example.invalid"]);
    common::git(dir, &["config", "user.name", "t"]);
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-q", "-m", "the plan"]);
}

/// The keys whose scripts ran, in the names [`wire`] gave them.
fn reached(log: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A plan, and configuration that records every command it could reach.
fn a_wired_plan() -> (tempfile::TempDir, tempfile::TempDir, Wiring) {
    let dir = tempfile::Builder::new()
        .prefix("reaches-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    // The scripts and their log live outside the repository under measurement,
    // so that "wrote nothing but the plan" stays a question about the plan.
    let support = tempfile::Builder::new()
        .prefix("runners-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    common::census::a_plan_to_act_on(dir.path());
    let wiring = wire(support.path());
    std::fs::write(dir.path().join(".aplan.yml"), &wiring.config).expect("config");
    // **A repository, because `verify` and `close` resolve evidence against
    // one.** Without it they refuse with "`.` is not a git repository" before
    // reaching any runner, and the measurement would record that bail-out as a
    // command that spawned nothing. Measured once it was there: `close` reaches
    // the test list, and that is the allowance this gate exists to pin.
    as_a_repository(dir.path());
    (dir, support, wiring)
}

#[test]
fn which_configured_commands_a_command_reaches_is_measured_not_declared() {
    let mut wrong = Vec::new();
    let mut both = Vec::new();
    let mut ran = 0usize;

    for (name, argv, start) in common::census::RUNS {
        if CANNOT_BE_MEASURED.iter().any(|(c, _)| c == name) {
            continue;
        }
        // **A command that needs an empty directory cannot be measured here**
        // (`A1-354`, audit). This loop ignored `Start` and always populated,
        // which for `init` would have meant handing it the configuration whose
        // absence it requires: it would refuse, spawn nothing, and read as
        // clean. `init` is excused above by name; a future one has to be.
        assert!(
            *start == common::census::Start::Populated,
            "`{name}` starts from an empty directory, which this measurement cannot provide, and \
             it is not excused in `CANNOT_BE_MEASURED`"
        );
        let (dir, _support, wiring) = a_wired_plan();
        let before = common::census::plan_bytes(dir.path());
        let out = common::run(dir.path(), argv);
        let wrote = before != common::census::plan_bytes(dir.path());
        ran += 1;
        let saw = reached(&wiring.log);
        if wrote && !saw.is_empty() && !WRITES_AND_OBSERVES.iter().any(|(c, _)| c == name) {
            both.push(format!(
                "`aplan {}` changed the plan *and* ran {saw:?}",
                argv.join(" ")
            ));
        }
        let allowed: BTreeSet<String> = ALLOWED_TO_REACH
            .iter()
            .find(|(c, _, _)| c == name)
            .map(|(_, keys, _)| keys.iter().map(|k| (*k).to_string()).collect())
            .unwrap_or_default();
        if saw != allowed {
            wrong.push(format!(
                "`aplan {}` reached {:?} and is allowed {:?} (exit {})\n      {}",
                argv.join(" "),
                saw,
                allowed,
                out.code,
                out.text().lines().take(2).collect::<Vec<_>>().join(" / ")
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "{} command(s) that reached something other than what they are allowed to reach \
         (ADR-0043, `A1-354`). A mutation causing an effect outside the plan is the decision \
         being reverted; an observation newly needed is an entry in `ALLOWED_TO_REACH` with the \
         argument for it:\n  - {}",
        wrong.len(),
        wrong.join("\n  - ")
    );
    assert!(
        both.is_empty(),
        "{} command(s) that changed the plan and ran something configuration named, which is \
         ADR-0043's decision being reverted unless the run is an observation made in order to \
         refuse. Add it to `WRITES_AND_OBSERVES` with the refusal it performs, or do not \
         spawn:\n  - {}",
        both.len(),
        both.join("\n  - ")
    );
    for (command, why) in WRITES_AND_OBSERVES {
        assert!(
            why.len() > 60,
            "`{command}` writes and observes with no argument worth reading: {why:?}"
        );
    }
    assert!(
        ran > 30,
        "the measurement drove only {ran} command(s), so it cannot have seen the surface"
    );
}

#[test]
fn a_command_this_gate_cannot_drive_says_why_and_is_still_a_command() {
    // That `RUNS` covers clap's whole tree is asserted in `mutation_policy.rs`,
    // which owns the census. What is this file's own is narrower: a command
    // *excused from this measurement* has to say why, and has to still exist --
    // an exemption naming a command that was renamed away reads as coverage.
    let declared: BTreeSet<String> = aplan::cli_command()
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();
    for (command, why) in CANNOT_BE_MEASURED {
        assert!(
            declared.contains(*command),
            "`{command}` is excused from the measurement and is not a command any more"
        );
        assert!(
            why.len() > 60,
            "`{command}` is excused with no reason worth reading: {why:?}"
        );
    }
}

#[test]
fn the_one_observation_close_makes_is_the_one_it_is_for() {
    let (dir, _support, wiring) = a_wired_plan();
    // `the_only_test` is the name the fixture's `list` prints, so this is a
    // citation the runner knows.
    let good = common::run(
        dir.path(),
        &["close", "W-1", "--evidence", "test:the_only_test"],
    );
    assert_eq!(
        good.code,
        0,
        "closing with a test the runner lists was refused: {}",
        good.text()
    );
    assert_eq!(
        reached(&wiring.log),
        ["test.list".to_string(), "test.wrap".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "`close` reached something other than the test list: {}",
        good.text()
    );

    // And the refusal is what the observation is *for*. A citation the runner
    // has never heard of is rejected by `close` rather than left for `verify` to
    // find later, which is the reason `close` is allowed to ask at all.
    let (dir, _support, wiring) = a_wired_plan();
    let bad = common::run(
        dir.path(),
        &["close", "W-1", "--evidence", "test:no_such_test"],
    );
    assert_ne!(
        bad.code,
        0,
        "closing on a test the runner does not list was accepted, which is the refusal the \
         allowance in `ALLOWED_TO_REACH` is justified by: {}",
        bad.text()
    );
    assert!(
        reached(&wiring.log).contains("test.list"),
        "the refusal did not come from asking the runner, so it is not the observation this \
         gate permits: {}",
        bad.text()
    );
}

/// **Every wired runner is reached by something** (`A1-354`, audit).
///
/// The exhaustiveness check above proves the fixture *names* a script in every
/// place configuration can name a command. It does not prove any of them would
/// run. A key wired under a misspelling, or one whose shape the loader quietly
/// ignores, is a key no command can ever be caught reaching -- and the
/// measurement would report "reached nothing" for the whole surface and pass.
///
/// `verify --execute` alone proves one of the ten. This drives the citations
/// that reach the rest: a `url` citation for the url checker, a closed `test`
/// citation for the lister, and the two reporting commands that ask about the
/// version.
#[test]
fn every_wired_runner_is_reached_by_something() {
    let (dir, _support, wiring) = a_wired_plan();
    let amended = common::run(
        dir.path(),
        &[
            "amend",
            "W-2",
            "--add-evidence",
            "url:https://example.invalid/ok",
        ],
    );
    assert_eq!(amended.code, 0, "setup: {}", amended.text());
    let closed = common::run(
        dir.path(),
        &["close", "W-1", "--evidence", "test:the_only_test"],
    );
    assert_eq!(closed.code, 0, "setup: {}", closed.text());
    for argv in [
        ["verify", "--execute"].as_slice(),
        ["version"].as_slice(),
        ["attest"].as_slice(),
    ] {
        common::run(dir.path(), argv);
    }

    let saw = reached(&wiring.log);
    let wired: BTreeSet<String> = WIRED.iter().map(|(_, s)| (*s).to_string()).collect();
    let unreached: Vec<&String> = wired.difference(&saw).collect();
    assert!(
        unreached.is_empty(),
        "{} wired runner(s) that nothing in this suite reaches, so no command could be caught \
         reaching them either: {unreached:?}. It saw {saw:?}",
        unreached.len()
    );
}

#[test]
fn verify_execute_is_the_control_that_proves_the_recording_works() {
    let (dir, _support, wiring) = a_wired_plan();
    let closed = common::run(
        dir.path(),
        &["close", "W-1", "--evidence", "test:the_only_test"],
    );
    assert_eq!(closed.code, 0, "setup: {}", closed.text());
    let fresh = reached(&wiring.log);
    let out = common::run(dir.path(), &["verify", "--execute"]);
    let saw = reached(&wiring.log);
    assert!(
        saw.contains("test.run"),
        "`verify --execute` did not run the configured runner, so every \"reached nothing\" in \
         this file is a measurement of a fixture that does not work. It saw {saw:?} (before: \
         {fresh:?}), exit {}:\n{}",
        out.code,
        out.text()
    );
}

// ------------------------------------------- the fixture covers the surface

/// Every configuration path that can name a command, and the script [`wire`]
/// installs there.
///
/// The measurement is only as wide as this list, and a key the fixture does not
/// name is a key no command can be caught reaching. So the list is not trusted:
/// it is compared against the published schema below, which is generated from
/// the Rust types and gated for drift, so a new key reaches this gate on the
/// commit that adds it rather than when somebody remembers.
const WIRED: &[(&str, &str)] = &[
    (".test.list", "test.list"),
    (".test.run", "test.run"),
    (".test.skipped", "test.skipped"),
    (".test.wrap", "test.wrap"),
    (".url.check", "url.check"),
    (".url.wrap", "url.wrap"),
    (".version.check.command", "version.check"),
    (".version.check.wrap", "version.check.wrap"),
    (".version.current.command", "version.current"),
    (".version.current.wrap", "version.current.wrap"),
];

#[test]
fn every_place_configuration_can_name_a_command_is_wired_into_the_fixture() {
    let text =
        std::fs::read_to_string(common::sources::workspace_root().join("schemas/v1/config.json"))
            .expect("the published config schema");
    let schema: serde_json::Value = serde_json::from_str(&text).expect("the schema parses");
    let found = command_valued_paths(&schema);
    let wired: BTreeSet<String> = WIRED.iter().map(|(p, _)| (*p).to_string()).collect();

    let unwired: Vec<&String> = found.difference(&wired).collect();
    assert!(
        unwired.is_empty(),
        "configuration can name a command at {} path(s) the fixture leaves unwired, so no \
         command can be caught reaching them (`A1-354`). Wire each in `wire` and add it to \
         `WIRED`: {unwired:?}",
        unwired.len()
    );
    let stale: Vec<&String> = wired.difference(&found).collect();
    assert!(
        stale.is_empty(),
        "wired, but configuration no longer names a command there: {stale:?}"
    );
    // The discriminator is the authors' own word: a key whose documentation says
    // `argv` or "spawned". It is the honest limit of this half -- a new key
    // documented only as "the command line" would not be found -- and the scan
    // above is what covers the gap, because a key nothing spawns is harmless and
    // a spawn is in the sources whatever its key is called.
    assert!(
        found.len() >= 10,
        "the schema walk found only {} command-valued path(s), so it cannot have seen the \
         configuration surface",
        found.len()
    );
}

/// Every path in the schema where configuration can name a command, with the
/// array subscript of a repeated runner collapsed: `test` takes one runner or a
/// list of them, and `.test.list` is the same key either way.
fn command_valued_paths(schema: &serde_json::Value) -> BTreeSet<String> {
    let defs = schema
        .get("$defs")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let mut out = BTreeSet::new();
    descend(schema, &defs, "", &mut out, &mut Vec::new());
    out
}

fn descend(
    node: &serde_json::Value,
    defs: &serde_json::Value,
    path: &str,
    out: &mut BTreeSet<String>,
    seen: &mut Vec<String>,
) {
    if let Some(reference) = node.get("$ref").and_then(serde_json::Value::as_str) {
        let name = reference
            .rsplit('/')
            .next()
            .unwrap_or(reference)
            .to_string();
        // A definition that refers to itself would otherwise walk for ever.
        if seen.contains(&name) {
            return;
        }
        seen.push(name.clone());
        if let Some(def) = defs.get(&name) {
            descend(def, defs, path, out, seen);
        }
        seen.pop();
        return;
    }
    for combinator in ["anyOf", "allOf", "oneOf"] {
        if let Some(branches) = node.get(combinator).and_then(serde_json::Value::as_array) {
            for branch in branches {
                descend(branch, defs, path, out, seen);
            }
        }
    }
    if let Some(properties) = node
        .get("properties")
        .and_then(serde_json::Value::as_object)
    {
        for (key, property) in properties {
            let child = format!("{path}.{key}");
            if names_a_command(property) {
                out.insert(child.clone());
            }
            descend(property, defs, &child, out, seen);
        }
    }
    if let Some(items) = node.get("items") {
        descend(items, defs, path, out, seen);
    }
    if let Some(extra) = node.get("additionalProperties").filter(|v| v.is_object()) {
        descend(extra, defs, path, out, seen);
    }
}

/// Whether this property is an argv vector configuration hands to a process.
fn names_a_command(property: &serde_json::Value) -> bool {
    let described = property
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    if !(described.contains("argv") || described.contains("spawned")) {
        return false;
    }
    let mut shapes = vec![property];
    if let Some(branches) = property.get("anyOf").and_then(serde_json::Value::as_array) {
        shapes.extend(branches.iter());
    }
    shapes.iter().any(|shape| {
        let typed = |node: Option<&serde_json::Value>, wanted: &str| match node {
            Some(serde_json::Value::String(one)) => one == wanted,
            Some(serde_json::Value::Array(many)) => many.iter().any(|t| t.as_str() == Some(wanted)),
            _ => false,
        };
        typed(shape.get("type"), "array")
            && typed(shape.get("items").and_then(|i| i.get("type")), "string")
    })
}

// ------------------------------------------ the spawn configuration never names

/// The mutations driven under the `git` shim, and whether each may reach it.
///
/// `claim` is the only command in the workspace that resolves an identity, so it
/// is the only one that can reach `git config user.name`. The others are here to
/// make that a measured claim rather than an observation about today's call
/// graph: `release` and `working` read the owner the plan already holds, and a
/// change that had them ask the machine instead would be a mutation consulting
/// the world, which is the thing ADR-0043 is about.
const UNDER_THE_SHIM: &[(&[&str], bool)] = &[
    (&["claim", "W-1"], true),
    (&["release", "W-4"], false),
    (&["start", "W-1"], false),
    (&["stop", "W-4"], false),
    (&["add", "Something"], false),
    (&["close", "W-1", "--evidence", "file:NOTES.md"], false),
    (&["drop", "W-3"], false),
];

#[test]
fn the_one_subprocess_no_configuration_names_is_the_one_that_asks_who_you_are() {
    for (argv, may_reach_git) in UNDER_THE_SHIM {
        let dir = tempfile::Builder::new()
            .prefix("shim-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let support = tempfile::Builder::new()
            .prefix("shim-path-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        common::census::a_plan_to_act_on(dir.path());
        as_a_repository(dir.path());

        // A `git` earlier on `PATH` than the real one, which records what it was
        // asked and answers. `git2` is linked in and resolves nothing through
        // `PATH`, so this shim sees exactly the subprocesses the tool starts by
        // name -- which is the half the configured-command measurement cannot
        // reach, because nothing configures this one.
        let log = support.path().join("git.log");
        let shim = support.path().join("bin");
        std::fs::create_dir_all(&shim).expect("mkdir");
        let script = shim.join("git");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\necho measured-by-git\n",
                log.display()
            ),
        )
        .expect("write the shim");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        // An empty `HOME`, so the per-user file cannot answer first and the
        // chain reaches the shim deterministically rather than depending on
        // whose machine the suite is running on (`A1-364`, whose first
        // injection test measured the machine instead of the code).
        let home = support.path().join("home");
        std::fs::create_dir_all(&home).expect("mkdir");
        let mut command = common::command(dir.path(), argv);
        let inherited = std::env::var("PATH").unwrap_or_default();
        command
            .env("PATH", format!("{}:{inherited}", shim.display()))
            .env("HOME", &home)
            .env_remove("APLAN_OWNER")
            .env_remove("XDG_CONFIG_HOME");
        let out = command.output().expect("run the binary");
        // **The command has to have reached its work** (`A1-354`, audit). This
        // table carried `working`, which is not a command: `aplan` answered
        // `unrecognized subcommand`, started no subprocess, and the row passed
        // having measured nothing. Asserting both that clap declares it and
        // that it exited cleanly is what makes "started no `git`" a claim about
        // the command rather than about a typo.
        assert!(
            aplan::cli_command()
                .get_subcommands()
                .any(|c| c.get_name() == argv[0]),
            "`{}` is not a command, so this row measures nothing",
            argv[0]
        );
        assert!(
            out.status.success(),
            "`aplan {}` did not reach its work, so it cannot show what a mutation spawns: {}{}",
            argv.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let asked: Vec<String> = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();

        if *may_reach_git {
            assert_eq!(
                asked,
                vec!["config user.name".to_string()],
                "`aplan {}` asked git for something other than the one value identity needs: \
                 {asked:?}",
                argv.join(" ")
            );
            let owner = std::fs::read_to_string(dir.path().join("aplan/tasks/W-1.md"))
                .expect("the claimed task");
            assert!(
                owner.contains("measured-by-git"),
                "the shim recorded the question but its answer never reached the plan, so this \
                 test is measuring something other than the chain it names:\n{}",
                String::from_utf8_lossy(&out.stdout)
            );
        } else {
            assert!(
                asked.is_empty(),
                "`aplan {}` started a `git` subprocess, which no configuration names and \
                 ADR-0043 does not permit: {asked:?}",
                argv.join(" ")
            );
        }
    }
}
