//! When the tool emits colour, and that its own prose is subject to the same
//! answer as its findings (`A1-286`).
//!
//! **The env half already worked and this file pins it anyway.** `anstream`
//! resolves `NO_COLOR`, `CLICOLOR_FORCE` and `CLICOLOR=0` before it decides, so
//! the shipped binary honoured all three with no code. That is precisely why it
//! is worth a test: nothing in this repository was asserting a contract it was
//! keeping by accident, and the first change to how the streams are built would
//! have broken it silently.
//!
//! **It has to use a pty for half of it.** The default is colour at a terminal
//! and none in a pipe, so a test that only captured a pipe would be testing the
//! side that emits nothing, and would pass just as well with the flag ignored
//! entirely. `common::on_a_terminal_raw` is the pty, with the escapes left in.

use std::path::Path;

use crate::common;
use common::{on_a_terminal_raw, run};

const FOCUSED_GRAPH: &[&str] = &[
    "graph",
    "--format",
    "text",
    "--layout",
    "lanes",
    "--focus",
    "T-003",
    "--critical",
];
const LAYERED_GRAPH: &[&str] = &[
    "graph",
    "--format",
    "text",
    "--width",
    "160",
    "--ascii",
    "--focus",
    "T-003",
    "--critical",
];

/// The shared spawn boundary, rather than a contributor's shell profile,
/// decides the fixture's starting point (`A1-346`).
///
/// `Command::get_envs` exposes both explicit values and explicit removals, so
/// this proof does not have to mutate the test process's global environment.
/// The terminal tests below exercise the same private baseline through the pty
/// launcher; their per-case values are applied afterwards and retain authority.
#[test]
fn the_shared_launcher_has_a_known_colour_baseline() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let command = common::command(dir.path(), &["--help"]);
    let env: std::collections::BTreeMap<String, Option<String>> = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();

    for key in ["NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE"] {
        assert_eq!(env.get(key), Some(&None), "{key} was inherited");
    }
    assert_eq!(env.get("TERM"), Some(&Some("xterm-256color".to_owned())));
    assert_eq!(env.get("COLORTERM"), Some(&Some("truecolor".to_owned())));
}

/// A plan small enough to be quick, and **not clean**.
///
/// `T-002` claims `done` with nothing to show for it, which is `APLAN005`. That
/// is not decoration: the diagnostics formats render *findings*, so on a clean
/// plan `check --format github` prints nothing at all, and a census that
/// compared nothing against nothing passed while checking neither. The
/// non-emptiness guard in the census is what found that, one commit after this
/// file was written with a clean fixture.
fn capture_fixtures(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/.inbox")).expect("inbox");
    std::fs::write(
        dir.join("aplan/.inbox/I-01K8Z3A4H00000000000000000.yml"),
        "schema: aplan/capture/v1\nid: I-01K8Z3A4H00000000000000000\n\
         title: An open capture\nkind: task\nstate: open\n\
         captured_at: '2026-01-01T00:00:00Z'\norigin: cli\n\
         source:\n  namespace: colour\n  key: census\n",
    )
    .expect("open capture");
    std::fs::write(
        dir.join("aplan/.inbox/I-01K8Z3A4H00000000000000001.yml"),
        "schema: aplan/capture/v1\nid: I-01K8Z3A4H00000000000000001\n\
         title: A discarded capture\nkind: task\nstate: discarded\n\
         captured_at: '2026-01-01T00:00:00Z'\norigin: cli\n\
         disposed_at: '2026-01-02T00:00:00Z'\nreason: reviewed\n",
    )
    .expect("discarded capture");
}

fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    capture_fixtures(dir);
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("DOC.md"), "# Doc\n\n## intro\n\nbody\n").expect("doc");
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: A task\nphase: 0\nstatus: todo\nowner: someone\n\
         effort_days: 1\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: A task claiming done with nothing to show for it\nphase: 0\n\
         status: done\nowner: someone\neffort_days: 1\n---\n\n## Why\n\nx\n",
    )
    .expect("a finding to render");
    // A real task-to-task dependency, so `graph` has an edge to draw: with only
    // membership edges it refuses, prints nothing, and every format of it was
    // being compared against nothing.
    std::fs::write(
        dir.join("aplan/tasks/T-003.md"),
        "---\nid: T-003\ntitle: A task waiting on the first\nphase: 0\nstatus: todo\n\
         owner: someone\neffort_days: 1\nblocked_by: [T-001]\npaths:\n  - DOC.md\n---\n\n\
         ## Why\n\nx\n",
    )
    .expect("a dependency");
    // A parked task, which is the only way a *coloured* status reaches `why`: it
    // lists blockers that are unmet, and an unmet blocker is never `done`.
    std::fs::write(
        dir.join("aplan/tasks/T-004.md"),
        "---\nid: T-004\ntitle: A task parked on a decision\nphase: 0\nstatus: blocked\n\
         owner: someone\neffort_days: 1\nrevisit_at: 2026-01-01T00:00:00Z\n---\n\n\
         ## Why\n\nx\n",
    )
    .expect("a parked task");
    // A cut, so `roadmap` renders a cut header rather than only the
    // `cross-cutting` fallback -- which is where its `HEADING` role lives.
    std::fs::write(
        dir.join("aplan/tasks/M-c1.md"),
        "---\nid: M-c1\ntitle: A cut\nphase: 0\nstatus: todo\nowner: someone\n\
         tags: [milestone]\nblocked_by: [T-001, T-002, T-003, T-004]\n---\n\n## Why\n\nx\n",
    )
    .expect("a cut");
    std::fs::write(
        dir.join("feature.blueprint.yml"),
        concat!(
            "schema: aplan/blueprint/v1\n",
            "name: colour\n",
            "title: Colour blueprint\n",
            "description: Exercise blueprint rendering.\n",
            "tasks:\n",
            "  - key: work\n",
            "    title: Render a blueprint\n",
            "    effort: 1\n",
        ),
    )
    .expect("blueprint");
    std::fs::write(dir.join("edges.yml"), "- id: T-003\n  add: [T-001]\n")
        .expect("dependency batch");
    // A repository, because `verify` opens one and prints nothing without it --
    // the second pair the non-emptiness guard caught being checked against
    // nothing. Nothing here has evidence to resolve; what matters is that the
    // command gets far enough to render its verdict in the format asked for.
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
    };
    git(&["init", "-q", "."]);
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@example.invalid",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "fixture",
    ]);
}

/// A plan and a repository rich enough that **every** command in `STYLED` prints
/// the row that carries its role.
///
/// Each addition is here because the census refused without it, which is the
/// point of its non-emptiness assertion: `stale` needs two commits and an old
/// one, because ages are measured from `HEAD`'s commit time and a single commit
/// makes every task zero days idle however old it is; `deprecations` needs a
/// `deprecation` and a `removal`; `due` needs a `revisit_at`; `why` needs a task
/// with blockers; and `paths` needs a task that declares some.
fn census_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    capture_fixtures(dir);
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("DOC.md"), "# Doc\n\n## intro\n\nbody\n").expect("doc");
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: A first task\nphase: 0\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n  - DOC.md\ntags: [deprecation]\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: A task that removes what the first announced\nphase: 0\n\
         status: todo\nowner: t\neffort_days: 1\ntags: [removal]\nblocked_by: [T-001]\n\
         ---\n\n## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/T-003.md"),
        "---\nid: T-003\ntitle: A task parked on a decision\nphase: 0\nstatus: blocked\n\
         owner: t\neffort_days: 1\nrevisit_at: 2026-01-01T00:00:00Z\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/M-c1.md"),
        "---\nid: M-c1\ntitle: A cut\nphase: 0\nstatus: todo\nowner: t\n\
         tags: [milestone]\nblocked_by: [T-001, T-002, T-003]\n---\n\n## Why\n\nx\n",
    )
    .expect("cut");
    std::fs::write(
        dir.join("feature.blueprint.yml"),
        concat!(
            "schema: aplan/blueprint/v1\n",
            "name: colour\n",
            "title: Colour blueprint\n",
            "description: Exercise blueprint rendering.\n",
            "tasks:\n",
            "  - key: work\n",
            "    title: Render a blueprint\n",
            "    effort: 1\n",
        ),
    )
    .expect("blueprint");
    std::fs::write(dir.join("edges.yml"), "- id: T-003\n  add: [T-001]\n")
        .expect("dependency batch");
    let git = |args: &[&str], date: Option<&str>| {
        let mut c = std::process::Command::new("git");
        c.args(args).current_dir(dir);
        if let Some(date) = date {
            c.env("GIT_AUTHOR_DATE", date)
                .env("GIT_COMMITTER_DATE", date);
        }
        c.output().expect("git");
    };
    let who = ["-c", "user.email=t@example.invalid", "-c", "user.name=t"];
    git(&["init", "-q", "."], None);
    git(&["add", "-A"], None);
    let mut plan = who.to_vec();
    plan.extend(["commit", "-qm", "plan"]);
    git(&plan, Some("2025-01-01T00:00:00Z"));
    // A second, recent commit: `stale` measures from `HEAD`, so one commit makes
    // every task zero days idle however old that commit is.
    std::fs::write(dir.join("DOC.md"), "# Doc\n\n## intro\n\nbody\n\nmore\n").expect("doc");
    git(&["add", "-A"], None);
    let mut later = who.to_vec();
    later.extend(["commit", "-qm", "later"]);
    git(&later, None);
}

fn blueprint_render_args() -> Vec<&'static str> {
    vec![
        "apply",
        "--file",
        "feature.blueprint.yml",
        "--cut",
        "M-c1",
        "--dry-run",
    ]
}

fn dependency_render_args() -> Vec<&'static str> {
    vec!["apply", "--from", "edges.yml", "--dry-run"]
}

/// Every CSI escape sequence in `text`.
///
/// The final byte is searched for *after* the `[`, not from the escape: `[` is
/// itself in the `@`..`~` range a sequence ends on, so scanning from the start
/// returns `ESC [` and leaves `1m` in the text. That bug passed every
/// presence assertion in this file, because `ESC [` is still an escape, and was
/// caught only by the one test that compares the stripped bytes.
fn csi(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('\u{1b}') {
        rest = &rest[i..];
        let Some(body) = rest.strip_prefix("\u{1b}[") else {
            // Not a CSI sequence. Drop the escape and keep looking, rather than
            // stopping, so one stray byte cannot hide every sequence after it.
            rest = &rest['\u{1b}'.len_utf8()..];
            continue;
        };
        let Some((end, _)) = body.char_indices().find(|(_, c)| ('@'..='~').contains(c)) else {
            break;
        };
        let len = "\u{1b}[".len() + end + 1;
        out.push(rest[..len].to_string());
        rest = &rest[len..];
    }
    out
}

fn sgr(text: &str) -> Vec<String> {
    csi(text)
        .into_iter()
        .filter(|sequence| sequence.ends_with('m'))
        .collect()
}

fn has_colour(text: &str) -> bool {
    !sgr(text).is_empty()
}

/// Remove every SGR sequence, so two renderings can be compared on their words.
fn plain(text: &str) -> String {
    let mut out = text.to_string();
    for seq in sgr(text) {
        out = out.replace(&seq, "");
    }
    out
}

/// Remove deliberately per-invocation values before comparing whether colour
/// changed a JSON document. Their presence and type remain part of the
/// assertion: only equality of two independently sampled clocks is not.
fn stable_machine_json(command: &str, output: &str) -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(output).expect("machine JSON");
    if matches!(command, "ready" | "stale" | "tasks") {
        let observed = value
            .as_object_mut()
            .expect("machine JSON object")
            .remove("claim_observed_at_epoch_seconds")
            .and_then(|clock| clock.as_i64())
            .expect("claim-aware output carries an integer observation clock");
        assert!(observed >= 0, "claim observation precedes the Unix epoch");
    }
    if command == "verify" {
        let observed = value
            .get_mut("evidence_observation")
            .and_then(serde_json::Value::as_object_mut)
            .expect("verify machine output carries an evidence observation")
            .remove("observed_at_epoch_seconds")
            .and_then(|clock| clock.as_i64())
            .expect("evidence observation carries an integer observation clock");
        assert!(
            observed >= 0,
            "evidence observation precedes the Unix epoch"
        );
    }
    if command == "inbox" {
        let observed = value
            .as_object_mut()
            .expect("inbox machine JSON object")
            .remove("evaluated_at")
            .and_then(|clock| clock.as_str().map(str::to_string))
            .expect("inbox output carries an evaluation clock");
        assert!(
            observed.ends_with('Z'),
            "inbox evaluation clock is not UTC: {observed}"
        );
    }
    value
}

fn stable_human_text(command: &str, output: &str) -> String {
    if command != "inbox" {
        return output.to_string();
    }
    let mut normalized = output
        .lines()
        .map(|line| {
            if line.starts_with("evaluated at: ") {
                "evaluated at: <clock>"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if output.ends_with('\n') {
        normalized.push('\n');
    }
    normalized
}

#[test]
fn volatile_observation_clocks_do_not_impersonate_colour_differences() {
    let first = r#"{
        "evidence_observation": {"state": "current", "observed_at_epoch_seconds": 1},
        "warnings": []
    }"#;
    let later = r#"{
        "evidence_observation": {"state": "current", "observed_at_epoch_seconds": 2},
        "warnings": []
    }"#;

    assert_eq!(
        stable_machine_json("verify", first),
        stable_machine_json("verify", later)
    );
}

const COLS: usize = 100;

/// The case auto-detection cannot serve: a pager, or a screen capture.
/// Half of `color.explicitly_selectable`.
#[test]
fn colour_can_be_forced_into_a_pipe() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let forced = run(dir.path(), &["--color=always", "check"]);
    assert!(
        has_colour(&forced.stdout),
        "`--color=always` emitted no escape into a pipe, which is the one thing \
         it exists for: {:?}",
        forced.stdout
    );
    let default = run(dir.path(), &["check"]);
    assert!(
        !has_colour(&default.stdout),
        "a pipe was coloured with no flag asking for it, so the default moved: {:?}",
        default.stdout
    );
}

/// The other case: none at a terminal, without exporting a variable for one run.
/// The other half of `color.explicitly_selectable`.
#[test]
fn colour_can_be_refused_at_a_terminal() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let Some(default) = on_a_terminal_raw(dir.path(), COLS, &[], &["check"]) else {
        return;
    };
    assert!(
        has_colour(&default),
        "a terminal got no colour by default, so this test is measuring nothing: {default:?}"
    );
    let refused =
        on_a_terminal_raw(dir.path(), COLS, &[], &["--color=never", "check"]).expect("pty");
    assert!(
        !has_colour(&refused),
        "`--color=never` still coloured a terminal: {refused:?}"
    );
}

/// The contract `anstream` keeps, asserted so that a change to how the streams
/// are built cannot drop it quietly.
#[test]
fn the_environment_contract_still_holds() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    for (env, want, why) in [
        (vec![("NO_COLOR", "1")], false, "NO_COLOR disables"),
        (vec![("CLICOLOR", "0")], false, "CLICOLOR=0 disables"),
    ] {
        let Some(text) = on_a_terminal_raw(dir.path(), COLS, &env, &["check"]) else {
            return;
        };
        assert_eq!(has_colour(&text), want, "{why}: {text:?}");
    }

    // The forcing half is only visible in a pipe, where the default is none.
    let forced = common::command(dir.path(), &["check"])
        .env("CLICOLOR_FORCE", "1")
        .output()
        .expect("run");
    assert!(
        has_colour(&String::from_utf8_lossy(&forced.stdout)),
        "CLICOLOR_FORCE did not force colour into a pipe"
    );
}

/// Proof for `graph.text_focus_and_critical_overlay`: graph `text` is a human
/// terminal shape even though it is not named `human`, and it obeys the same
/// automatic, environment and explicit colour contract as every report.
#[test]
fn graph_text_obeys_every_colour_mode_without_changing_its_plain_bytes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let graph = FOCUSED_GRAPH;

    let piped = run(dir.path(), graph);
    assert!(!has_colour(&piped.stdout), "auto coloured a pipe");
    let mut always = vec!["--color=always"];
    always.extend(graph.iter().copied());
    let forced = run(dir.path(), &always);
    assert!(
        has_colour(&forced.stdout),
        "always did not colour graph text"
    );
    assert_eq!(plain(&forced.stdout), piped.stdout);

    let Some(terminal) = on_a_terminal_raw(dir.path(), COLS, &[], graph) else {
        return;
    };
    assert!(
        has_colour(&terminal),
        "auto did not colour graph text at a terminal"
    );
    for (environment, args, reason) in [
        (
            vec![("NO_COLOR", "1")],
            graph.to_vec(),
            "NO_COLOR did not disable graph colour",
        ),
        (
            vec![("CLICOLOR", "0")],
            graph.to_vec(),
            "CLICOLOR=0 did not disable graph colour",
        ),
        (
            vec![("CLICOLOR_FORCE", "1")],
            {
                let mut args = vec!["--color=never"];
                args.extend(graph);
                args
            },
            "--color=never did not override CLICOLOR_FORCE",
        ),
    ] {
        let text = on_a_terminal_raw(dir.path(), COLS, &environment, &args).expect("pty");
        assert!(!has_colour(&text), "{reason}: {text:?}");
    }

    let forced_by_environment = common::command(dir.path(), graph)
        .env("CLICOLOR_FORCE", "1")
        .output()
        .expect("run graph");
    assert!(
        has_colour(&String::from_utf8_lossy(&forced_by_environment.stdout)),
        "CLICOLOR_FORCE did not colour graph text in a pipe"
    );
}

/// Proof for `graph.layered_text_uses_the_shared_semantic_scene`: the static
/// cell painter uses the process-wide human-output palette while redirected
/// Auto and Never remain plain and all three choices have identical content.
#[test]
fn layered_graph_text_obeys_colour_without_changing_plain_bytes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let automatic = run(dir.path(), LAYERED_GRAPH);
    assert!(!has_colour(&automatic.stdout), "auto coloured a pipe");

    let mut always = vec!["--color=always"];
    always.extend(LAYERED_GRAPH.iter().copied());
    let forced = run(dir.path(), &always);
    assert!(
        has_colour(&forced.stdout),
        "always did not colour layered text"
    );
    assert_eq!(plain(&forced.stdout), automatic.stdout);

    let mut never = vec!["--color=never"];
    never.extend(LAYERED_GRAPH.iter().copied());
    let disabled = run(dir.path(), &never);
    assert!(!has_colour(&disabled.stdout));
    assert_eq!(disabled.stdout, automatic.stdout);
}

/// **The flag wins over the environment**, in both directions.
///
/// The precedence nobody writes down until it surprises them: a reader who
/// types `--color=always` on a machine whose profile exports `NO_COLOR` is
/// asking for this run to differ, and one who types `--color=never` under
/// `CLICOLOR_FORCE` is asking for the same thing the other way.
/// `color.explicitly_selectable` says *overrides*, and this is that word.
#[test]
fn an_explicit_choice_beats_the_environment() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let forced = common::command(dir.path(), &["--color=always", "check"])
        .env("NO_COLOR", "1")
        .output()
        .expect("run");
    assert!(
        has_colour(&String::from_utf8_lossy(&forced.stdout)),
        "`NO_COLOR` overrode an explicit `--color=always`"
    );

    let Some(refused) = on_a_terminal_raw(
        dir.path(),
        COLS,
        &[("CLICOLOR_FORCE", "1")],
        &["--color=never", "check"],
    ) else {
        return;
    };
    assert!(
        !has_colour(&refused),
        "`CLICOLOR_FORCE` overrode an explicit `--color=never`: {refused:?}"
    );
}

/// The defect this task is named for: the CLI crate's own prose was on a raw
/// stream, so no choice could reach it.
///
/// Before `A1-286` this printed the same bytes at every setting, on a terminal
/// and in a pipe alike, because `eprintln!` is not a stream anything can style
/// or strip.
/// `color.explicitly_selectable` promises the choice reaches the refusals, and
/// this is the half of it that was not true before this task.
#[test]
fn the_refusal_is_on_the_same_stream_as_a_finding() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let piped = run(dir.path(), &["show", "NOPE"]);
    assert!(
        piped.stderr.contains("no task `NOPE`"),
        "the refusal changed: {:?}",
        piped.stderr
    );
    assert!(
        !has_colour(&piped.stderr),
        "a piped refusal carried colour: {:?}",
        piped.stderr
    );

    let forced = run(dir.path(), &["--color=always", "show", "NOPE"]);
    assert!(
        has_colour(&forced.stderr),
        "the refusal ignored `--color=always`, so it is still not on the stream: {:?}",
        forced.stderr
    );

    let Some(terminal) = on_a_terminal_raw(dir.path(), COLS, &[], &["show", "NOPE"]) else {
        return;
    };
    assert!(
        has_colour(&terminal),
        "the refusal was plain at a terminal while a finding was coloured: {terminal:?}"
    );
    let refused =
        on_a_terminal_raw(dir.path(), COLS, &[], &["--color=never", "show", "NOPE"]).expect("pty");
    assert!(
        !has_colour(&refused),
        "`--color=never` did not reach the refusal: {refused:?}"
    );
}

/// What the pre-scan buys, and the only reason it exists.
///
/// `clap` renders its own usage error before any command runs, through an
/// `anstream` stream built with `Auto`. Reading `--color` only after
/// `get_matches()` would leave the first output a reader meets unable to obey
/// the flag they just typed.
/// `color.explicitly_selectable` names `clap`'s usage errors, and this is why
/// the choice is written before `get_matches()` rather than after.
#[test]
fn claps_own_error_obeys_the_choice() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let forced = run(dir.path(), &["--color=always", "check", "--nonsense"]);
    assert!(
        forced.stderr.contains("unexpected argument"),
        "not the error this test is about: {:?}",
        forced.stderr
    );
    assert!(
        has_colour(&forced.stderr),
        "`clap`'s error ignored `--color=always`, so the choice is set too late: {:?}",
        forced.stderr
    );

    let Some(refused) = on_a_terminal_raw(
        dir.path(),
        COLS,
        &[],
        &["--color=never", "check", "--nonsense"],
    ) else {
        return;
    };
    assert!(
        !has_colour(&refused),
        "`clap` coloured a usage error under `--color=never`: {refused:?}"
    );
}

/// A command, and the roles its rows must still carry.
type Surface<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)]);

/// **Every surface `A1-287` coloured is still coloured**, by the role it chose.
///
/// Found by mutation: removing the status colour from the `tasks` table broke no
/// test at all. That is `ready.order_is_derived`'s failure -- a promise pinned by
/// nothing, revertible with the suite green -- on the whole product of that task.
///
/// This is a list, and a list is not a census: `A1-288` walks `clap`'s tree and
/// requires every command to be styled or held with a reason, which is what
/// covers a command added later. What this holds is that the surfaces named in
/// `A1-287`'s own acceptance criterion carry the roles it named.
#[test]
fn every_surface_that_was_coloured_is_still_coloured() {
    // Rendered from the palette rather than spelled out. A literal `\e[36m` here
    // would be a second copy of `style::ID`, and the two would disagree the first
    // time the palette moved -- with this test failing as though a renderer had
    // stopped colouring, which is the opposite of what would have happened.
    let role = |s: anstyle::Style| s.render().to_string();
    let (id, done) = (
        role(aplan_output::style::ID),
        role(aplan_output::style::DONE),
    );
    let (heading, dim) = (
        role(aplan_output::style::HEADING),
        role(aplan_output::style::DIM),
    );
    let (attention, warning, error) = (
        role(aplan_output::style::ATTENTION),
        role(aplan_output::style::WARNING),
        role(aplan_output::style::ERROR),
    );
    let (id, done, heading) = (id.as_str(), done.as_str(), heading.as_str());
    let (dim, attention, warning, error) = (
        dim.as_str(),
        attention.as_str(),
        warning.as_str(),
        error.as_str(),
    );

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // Every command `A1-287`'s criterion names, with the roles its rows carry.
    // A surface printing no status is written that way on purpose: `ready`
    // offers work that is `todo` by construction, and `critical-path` is a
    // chain, so a status column on either would be one value repeated.
    let expected: &[Surface<'_>] = &[
        (&["ready"], &[(id, "the task id"), (dim, "the effort")]),
        (
            &["tasks"],
            &[
                (id, "the id column"),
                (done, "the status column"),
                (heading, "the header row"),
            ],
        ),
        (
            &["roadmap"],
            &[
                (id, "the member id"),
                (done, "the member status"),
                (heading, "the cut label"),
            ],
        ),
        (
            &["tree", "T-002"],
            &[(id, "the id"), (done, "the blocker's status")],
        ),
        (&["show", "T-002"], &[(id, "the id"), (done, "the status")]),
        (
            &["why", "M-c1"],
            &[(id, "the blocker id"), (attention, "a parked blocker")],
        ),
        (
            &["critical-path"],
            &[(id, "the id"), (heading, "the headline")],
        ),
        (
            FOCUSED_GRAPH,
            &[
                (heading, "the focused role"),
                (warning, "the direct-blocker role"),
                (id, "the critical route and ids"),
            ],
        ),
        (
            &["vacuity"],
            &[(id, "the rule code"), (heading, "the header row")],
        ),
        (
            &["paths"],
            &[(id, "the task column"), (heading, "the header row")],
        ),
        (&["summary"], &[(heading, "the header row")]),
        (&["check"], &[(error, "the severity")]),
        (&["show", "NOPE"], &[(error, "the refusal")]),
        // `due` rather than `stale`: both are id-and-status tables, and `stale`
        // needs a repository whose commits are old, which is a backdated fixture
        // for one role that `due` gives for the price of a `revisit_at`.
        (
            &["due", "--as-of", "2027-01-01T00:00:00Z"],
            &[
                (id, "the id column"),
                (heading, "the header row"),
                (dim, "the revisit"),
            ],
        ),
    ];

    let mut checked = 0_usize;
    for (args, roles) in expected {
        let mut forced = vec!["--color=always"];
        forced.extend_from_slice(args);
        let out = common::run(dir.path(), &forced).text();
        for (role, what) in *roles {
            assert!(
                out.contains(role),
                "`aplan {}` no longer styles {what}: {out:?}",
                args.join(" ")
            );
            checked += 1;
        }
    }
    assert!(checked >= 25, "only {checked} roles were checked");
}

/// A styled refusal folds where an unstyled one does, under its own indent.
///
/// The one style `A1-286` adds sits in front of text `fold_block` re-flows, and
/// `A1-287` records that folding a styled *run* leaves a style open across the
/// break. Seven columns cannot be split by any width, so this prefix is the case
/// that is safe -- and this is what says so, rather than the reasoning saying
/// it. It is also the regression pin for when `A1-287` changes `fold`.
#[test]
fn a_styled_refusal_folds_like_an_unstyled_one() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // Narrow enough that the sentence must fold more than once.
    let Some(coloured) = on_a_terminal_raw(
        dir.path(),
        46,
        &[],
        &["--color=always", "check", "--root", "nope"],
    ) else {
        return;
    };
    let bare = on_a_terminal_raw(
        dir.path(),
        46,
        &[],
        &["--color=never", "check", "--root", "nope"],
    )
    .expect("pty");

    assert!(
        bare.lines().count() >= 3,
        "the refusal did not fold, so this proves nothing: {bare:?}"
    );
    assert_eq!(
        plain(&coloured),
        bare,
        "styling changed where the refusal folded, or what it folded under"
    );
    // The style opens and closes on the first line: no continuation inherits it,
    // which is what would paint an indent.
    let first = coloured.lines().next().expect("a line");
    assert_eq!(
        sgr(first).last().map(String::as_str),
        Some("\u{1b}[0m"),
        "the prefix's style did not close on the line it opened: {first:?}"
    );
    for line in coloured.lines().skip(1) {
        assert!(
            sgr(line).is_empty(),
            "a continuation carried styling the prefix opened: {line:?}"
        );
    }
}

/// Colour adds emphasis and never information.
///
/// The proof that carries this is *not* the byte-identical `trycmd` corpus,
/// which captures a pipe and so has no escapes to remove. It is this: the forced
/// rendering with every escape stripped is the refused rendering, byte for byte.
/// `A1-287` is what makes it load-bearing, by adding colour worth carrying; this
/// pins the property while there is almost none.
///
/// A census over `COMMAND_SURFACES`, for the same reason the machine half is
/// one: a hand-written list stops covering the tool the moment a command is
/// added, and says nothing about it while going on passing.
#[test]
fn stripping_a_forced_colour_gives_the_refused_one() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let extra = |command: &str| -> Vec<&'static str> {
        match command {
            // `version` joins them for the same reason: without an id the bare
            // form refuses, and a refusal prints nothing, which is the vacuity
            // this census exists to catch (`A1-313`).
            "show" | "why" | "tree" | "version" => vec!["T-001"],
            "test" => vec!["list"],
            "check-docs" => vec!["DOC.md"],
            "rekey" => vec!["T-001", "T-900"],
            "blueprint" => blueprint_render_args(),
            "dep" => dependency_render_args(),
            "capture" => vec!["An open capture", "--source", "colour:census"],
            _ => Vec::new(),
        }
    };

    let mut checked = 0_usize;
    // Every command with a human rendering, plus the two shapes that reach a
    // reader through stderr rather than stdout: a refusal the tool raises, and
    // one `clap` raises.
    let mut shapes: Vec<Vec<&str>> = aplan::COMMAND_SURFACES
        .iter()
        .filter(|(_, surface, _)| surface.formats().contains(&aplan_output::Format::Human))
        .map(|(command, ..)| {
            let mut args = vec![*command];
            args.extend(extra(command));
            args
        })
        .collect();
    // Graph's terminal rendering is named `text` rather than `human`, because
    // the same surface also has Mermaid and DOT. It carries the identical
    // stream-resolved styling contract and must not fall out of this census.
    shapes.push(vec!["graph", "--format", "text", "--width", "240"]);
    shapes.push(vec!["show", "NOPE"]);
    shapes.push(vec!["check", "--root", "nope"]);
    shapes.push(vec!["check", "--nonsense"]);

    for args in &shapes {
        let mut forced = vec!["--color=always"];
        forced.extend_from_slice(args);
        let mut refused = vec!["--color=never"];
        refused.extend_from_slice(args);

        let a = common::run(dir.path(), &forced);
        let n = if args.first() == Some(&"rekey") {
            let other = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("tempdir");
            fixture(other.path());
            common::run(other.path(), &refused)
        } else {
            common::run(dir.path(), &refused)
        };
        assert!(
            !a.text().is_empty(),
            "`aplan {}` said nothing at all, so nothing was compared",
            args.join(" ")
        );
        assert_eq!(
            stable_human_text(args[0], &plain(&a.text())),
            stable_human_text(args[0], &n.text()),
            "`aplan {}` says something in colour that it does not say without it",
            args.join(" ")
        );
        checked += 1;
    }
    assert!(
        checked >= 15,
        "only {checked} renderings were compared, so the table is not reaching \
         the commands"
    );
}

/// A parsed format is bytes, and the strongest colour setting must not reach it.
///
/// **A census over the tool's own table, not a list.** The first version of this
/// test named ten pairs by hand, which is the shape `A1-206`'s audit called "a
/// sample calling itself a census": a new command with a `--format` would emit
/// whatever it liked under `--color=always` and this file would stay green.
/// `COMMAND_SURFACES` says which surface each command prints and
/// `Surface::formats` says what renders it, so the pairs come from the same
/// table the binary refuses on.
///
/// The last clause of `color.explicitly_selectable`. `A1-288` owns the standing
/// gate; this is the claim itself, asserted where the flag that could break it
/// ships.
#[test]
fn the_machine_formats_carry_no_colour_when_it_is_forced() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // What a command needs before `--format` is reached at all. Without it clap
    // refuses first and every format looks clean, which is how the sibling
    // census in `format_surfaces.rs` records the same trap.
    let extra = |command: &str| -> Vec<&'static str> {
        match command {
            "show" | "why" | "tree" | "version" => vec!["T-001"],
            "test" => vec!["list"],
            "check-docs" => vec!["DOC.md"],
            "rekey" => vec!["T-001", "T-900"],
            "blueprint" => blueprint_render_args(),
            "dep" => dependency_render_args(),
            "capture" => vec!["An open capture", "--source", "colour:census"],
            // **The clock is pinned, and it has to be.** This test runs each
            // command twice and compares the bytes, and `due`'s JSON carries
            // `overdue_seconds` measured from *now* -- so two runs that straddle
            // a second differ by one, and the test fails saying colour changed
            // the output when nothing about colour was involved. It passed
            // locally, where the two invocations landed in the same second, and
            // failed on the CI runner, which is the flake reading exactly
            // backwards. `--as-of` is the flag that exists for this.
            "due" => vec!["--as-of", "2027-01-01T00:00:00Z"],
            _ => Vec::new(),
        }
    };

    let mut checked = 0_usize;
    for (command, surface, _) in aplan::COMMAND_SURFACES {
        for format in surface.formats() {
            if format.supports_terminal_styling() {
                continue; // the two human terminal renderings colour is for
            }
            let mut args: Vec<&str> = vec![command];
            args.extend(extra(command));
            args.extend(["--format", format.as_str()]);

            let mut forced = vec!["--color=always"];
            forced.extend_from_slice(&args);
            let a = common::run(dir.path(), &forced);
            checked += 1;
            // Without this the whole census is one missing argument away from
            // vacuity: a command `clap` refuses prints nothing to stdout, and
            // "nothing" carries no escape and matches "nothing".
            assert!(
                !a.stdout.is_empty(),
                "`aplan {}` printed nothing to stdout, so this pair was checked \
                 vacuously: {:?}",
                args.join(" "),
                a.stderr
            );
            assert!(
                !has_colour(&a.stdout),
                "`aplan {}` emitted an escape under `--color=always`, into output a \
                 parser reads: {:?}",
                args.join(" "),
                csi(&a.stdout)
            );
            let n = if *command == "rekey" {
                let other = tempfile::Builder::new()
                    .tempdir_in(aplan_core::fixture::fixture_root())
                    .expect("tempdir");
                fixture(other.path());
                common::run(other.path(), &args)
            } else {
                common::run(dir.path(), &args)
            };
            if matches!(format.as_str(), "json" | "agent") {
                assert_eq!(
                    stable_machine_json(command, &a.stdout),
                    stable_machine_json(command, &n.stdout),
                    "`aplan {}` rendered differently under `--color=always`",
                    args.join(" ")
                );
            } else {
                assert_eq!(
                    a.stdout,
                    n.stdout,
                    "`aplan {}` rendered differently under `--color=always`",
                    args.join(" ")
                );
            }
        }
    }

    // The commands that emit a machine shape with no `--format` at all, so the
    // table above cannot reach them. `mcp` has its own test. `attest` is not
    // here: it refuses to attest a plan carrying findings, and this fixture
    // carries one on purpose, so it would print nothing to stdout and be checked
    // vacuously -- which is exactly what the first version of this test did,
    // with a `--dry-run` flag that does not even exist.
    for args in [&["facts"][..], &["schema"][..], &["export-agents-md"][..]] {
        let mut forced = vec!["--color=always"];
        forced.extend_from_slice(args);
        let a = common::run(dir.path(), &forced);
        checked += 1;
        assert!(
            !a.stdout.is_empty(),
            "`aplan {}` printed nothing, so nothing was checked",
            args.join(" ")
        );
        assert!(
            !has_colour(&a.stdout),
            "`aplan {}` emitted an escape under `--color=always`: {:?}",
            args.join(" "),
            csi(&a.stdout)
        );
        assert_eq!(
            a.stdout,
            common::run(dir.path(), args).stdout,
            "`aplan {}` rendered differently under `--color=always`",
            args.join(" ")
        );
    }

    assert!(
        checked >= 30,
        "only {checked} machine renderings were exercised, so the table is not \
         reaching the commands and this passes vacuously"
    );
}

/// `aplan mcp` speaks JSON-RPC on a pipe and must not gain a byte at any
/// setting.
///
/// It reaches stdout through `rmcp`'s own transport rather than through
/// `write_stdout`, so this is a property the wiring gives for free -- which is
/// exactly the kind that stops being true when somebody unifies the two.
#[test]
fn mcp_is_unchanged_under_every_setting() {
    use std::io::{BufRead as _, Read as _, Write as _};

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let speak = |args: &[&str]| -> String {
        let mut child = common::command(dir.path(), args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the server starts");
        let mut stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let mut stdout = std::io::BufReader::new(stdout);
        let mut output = String::new();
        let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
        let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let tools = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;

        writeln!(stdin, "{initialize}").expect("write initialize");
        stdin.flush().expect("flush initialize");
        assert_ne!(
            stdout.read_line(&mut output).expect("read initialize"),
            0,
            "server closed before initialize response"
        );

        writeln!(stdin, "{initialized}").expect("write initialized notification");
        writeln!(stdin, "{tools}").expect("write tool list");
        stdin.flush().expect("flush tool list");
        assert_ne!(
            stdout.read_line(&mut output).expect("read tool list"),
            0,
            "server closed before tool-list response"
        );
        drop(stdin);
        stdout.read_to_string(&mut output).expect("drain stdout");
        let status = child.wait().expect("the server exits");
        assert!(status.success(), "server exited with {status}");
        output
    };

    let plainly = speak(&["mcp"]);
    assert!(
        plainly.contains("aplan_ready"),
        "the server did not answer, so this proves nothing: {plainly}"
    );
    for choice in ["always", "never", "auto"] {
        let flagged = speak(&[&format!("--color={choice}"), "mcp"]);
        assert_eq!(
            flagged, plainly,
            "`--color={choice}` changed what the MCP server wrote"
        );
    }
}

/// Every command a person reads at a terminal: an invocation that makes it print
/// the thing carrying a role, and a marker its *successful* output contains.
///
/// **The marker is not decoration.** A refusal is styled too -- `aplan:` is the
/// error role -- so "this emitted a role" is satisfied by a command that refused
/// in red, and nineteen commands could each be quietly failing with the census
/// green. The marker is what makes the assertion about the row rather than about
/// any coloured byte.
///
/// The invocation matters as much as the name. A command given no argument is
/// refused by `clap` and prints nothing, and nothing carries no escape -- which
/// is how three pairs in the machine census above were passing against an empty
/// string until a non-emptiness guard was added (`A1-286`, audit).
const STYLED: &[(&str, &[&str], &str)] = &[
    ("check", &["check"], "DAG acyclic"),
    ("verify", &["verify"], "evidence resolved"),
    ("test list", &["test", "list"], "test catalogue:"),
    (
        "blueprint apply",
        &[
            "blueprint",
            "apply",
            "--file",
            "feature.blueprint.yml",
            "--cut",
            "M-c1",
            "--dry-run",
        ],
        "blueprint application: predicted",
    ),
    (
        "dep apply",
        &["dep", "apply", "--from", "edges.yml", "--dry-run"],
        "dependency batch: predicted",
    ),
    ("check-docs", &["check-docs", "DOC.md"], "document(s)"),
    ("ready", &["ready"], "T-001"),
    // `why` on a task with no blockers prints one unstyled sentence, so the
    // census has to point it at one that has some.
    ("why", &["why", "M-c1"], "is blocked by"),
    ("show", &["show", "T-001"], "phase 0"),
    ("explain", &["explain", "rule", "APLAN012"], "APLAN012"),
    ("tree", &["tree", "T-001"], "releases:"),
    ("roadmap", &["roadmap"], "roadmap:"),
    ("summary", &["summary"], "effort"),
    ("tasks", &["tasks"], "status"),
    (
        "graph",
        &[
            "graph",
            "--format",
            "text",
            "--focus",
            "T-002",
            "--critical",
        ],
        "legend:",
    ),
    ("critical-path", &["critical-path"], "critical path:"),
    ("vacuity", &["vacuity"], "rule/task pair"),
    ("paths", &["paths"], "commit(s) sampled"),
    ("stale", &["stale"], "idle"),
    ("deprecations", &["deprecations"], "announced in"),
    (
        "due",
        &["due", "--as-of", "2027-01-01T00:00:00Z"],
        "revisit",
    ),
    (
        "capture",
        &["capture", "A capture created by the colour census"],
        "capture",
    ),
    ("inbox", &["inbox"], "inbox:"),
    (
        "inbox show",
        &["inbox", "show", "I-01K8Z3A4H00000000000000000"],
        "I-01K8Z3A4H00000000000000000",
    ),
    (
        "inbox amend",
        &[
            "inbox",
            "amend",
            "I-01K8Z3A4H00000000000000000",
            "--title",
            "A capture amended by the colour census",
        ],
        "I-01K8Z3A4H00000000000000000",
    ),
    (
        "inbox discard",
        &["inbox", "discard", "I-01K8Z3A4H00000000000000000"],
        "I-01K8Z3A4H00000000000000000",
    ),
    (
        "inbox restore",
        &["inbox", "restore", "I-01K8Z3A4H00000000000000001"],
        "I-01K8Z3A4H00000000000000001",
    ),
    (
        "inbox promote",
        &[
            "inbox",
            "promote",
            "I-01K8Z3A4H00000000000000000",
            "--task-id",
            "T-900",
            "--phase",
            "0",
            "--effort",
            "1",
            "--path",
            "DOC.md",
            "--tag",
            "cross-cutting",
        ],
        "T-900",
    ),
    // The mutations that print a row of the plan rather than a sentence about
    // it. Each gets a fixture of its own below, so one writing cannot change
    // what the next one prints.
    (
        "add",
        &[
            "add",
            "a task the census adds",
            "--id",
            "T-900",
            "--effort",
            "1",
        ],
        "wrote ",
    ),
    (
        "close",
        &[
            "close",
            "T-001",
            "--evidence",
            "file:DOC.md",
            "--no-resolve",
        ],
        "closed ",
    ),
    ("drop", &["drop", "T-003"], "dropped "),
    (
        "park",
        &["park", "T-001", "--reason", "portfolio decision"],
        "parked ",
    ),
    ("unpark", &["unpark", "T-003"], "unparked "),
];

/// Commands whose output carries no role, and why each one does not.
///
/// This exists so the assertion below is a census rather than a sample. `STYLED`
/// is a list somebody typed, and a list somebody typed stops covering the tool
/// the moment a command is added -- silently, with the suite still green. Naming
/// the exclusions makes adding a command a choice between two lists instead of
/// an omission from one, which is the shape `A1-206` settled on for width.
const UNSTYLED: &[(&str, &str, Option<&[&str]>)] = &[
    (
        "blueprint",
        "a parent with no output of its own: `clap` prints its help and exits",
        None,
    ),
    (
        "test",
        "a parent with no output of its own: `clap` prints its help and exits",
        None,
    ),
    (
        "test receipt",
        "a canonical machine statement whose bytes are correctness-bearing",
        None,
    ),
    (
        "facts",
        "a machine format, asserted by the machine census",
        None,
    ),
    (
        "version",
        "one word from a closed vocabulary, or a version number: there is no role to \
         carry, and colouring the answer a script is meant to substitute would be a \
         hazard rather than a help",
        Some(&["version", "T-001"]),
    ),
    (
        "schema",
        "a machine format, asserted by the machine census",
        None,
    ),
    (
        "export-roadmap-json",
        "writes a contract file; what it prints is the path it wrote",
        None,
    ),
    (
        "export-agents-md",
        "writes a document; what it prints is the document",
        Some(&["export-agents-md"]),
    ),
    (
        "attest",
        "prints a signed digest, which must not gain a byte from any setting",
        None,
    ),
    (
        "mcp",
        "speaks JSON-RPC on a pipe and never to a terminal",
        None,
    ),
    (
        "tui",
        "owns a Ratatui cell grid and semantic palette rather than emitting the CLI's styled prose; its visual roles are gated with its fixed-size buffers",
        None,
    ),
    (
        "help",
        "`clap` renders it, and that it obeys `--color` is `claps_own_error_obeys_the_choice`",
        None,
    ),
    (
        "export",
        "writes a document in somebody else's shape; what it prints is that document, and a \
         generated file with escape codes in it is not one its consumers accept",
        // **With a run, and that is the point.** A held entry with `None` is
        // skipped by `no_human_surface_lets_a_plan_write_to_the_terminal`, so
        // registering the command here without one quietly took it out of the
        // census that would have caught it writing a plan's own escape into a
        // committed file (`A1-189`, audit).
        Some(&["export", "--format", "checklist"]),
    ),
    ("init", "writes a config; what it prints is the path", None),
    (
        "import",
        "writes tasks; what it prints is a count and the fields it could not carry",
        Some(&[
            "import",
            "SRC.md",
            "--format",
            "checklist",
            "--id-prefix",
            "IM",
        ]),
    ),
    // `A1-359`'s four are the `dep add` shape exactly: a sentence about what was
    // just done, naming the id the reader typed. The holder is a name rather
    // than a role, so there is nothing for a colour to mean -- and it is read
    // through `sanitize::visible`, which is the property the sanitize census
    // asserts for it.
    (
        "claim",
        "a sentence about the claim just made, naming the id the reader typed and the name \
         recorded",
        Some(&["claim", "T-001", "--as", "ada"]),
    ),
    (
        "release",
        "a sentence about the claim just dropped, naming the id the reader typed",
        Some(&["release", "T-001"]),
    ),
    (
        "start",
        "a sentence about the status just set, naming the id the reader typed",
        Some(&["start", "T-001"]),
    ),
    (
        "stop",
        "a sentence about the status just cleared, naming the id the reader typed",
        Some(&["stop", "T-001"]),
    ),
    (
        "rekey",
        "a migration confirmation and review locations, whose ids and paths carry no semantic role",
        Some(&["rekey", "T-001", "T-900"]),
    ),
    (
        "dep",
        "a parent with no output of its own: `clap` prints its help and exits",
        None,
    ),
    (
        "dep add",
        "a sentence about the edge just asked for, naming the two ids the reader typed",
        Some(&["dep", "add", "T-002", "--after", "T-001"]),
    ),
    (
        "dep rm",
        "a sentence about the edge just removed, naming the two ids the reader typed",
        Some(&["dep", "rm", "T-002", "--after", "T-001"]),
    ),
    (
        "tag",
        "a parent with no output of its own: `clap` prints its help and exits",
        None,
    ),
    (
        "tag add",
        "a sentence about the tag just put on, naming the id and the word the reader typed",
        Some(&["tag", "add", "T-001", "backend"]),
    ),
    (
        "tag rm",
        "a sentence about the tag just taken off, naming the id and the word the reader typed",
        Some(&["tag", "rm", "T-001", "backend"]),
    ),
    (
        "tag rename",
        "a count and the two words the reader typed: there is no role to carry, and the \
         ids are not named because a rename is about the word rather than about any task",
        Some(&["tag", "rename", "backend", "frontend"]),
    ),
    (
        "mv",
        "a sentence about the move just asked for, naming the ids the reader typed",
        Some(&["mv", "T-001", "--cut", "M-c1"]),
    ),
    (
        "amend",
        "repairs a closed task's evidence, so it needs a citation that has rotted before it prints anything; `amend.rs` is what exercises it",
        Some(&["amend", "T-001", "--add-evidence", "note:x"]),
    ),
    (
        "supersede",
        "needs a closed task whose traced outcome no longer holds before it has a successor to write; `supersede.rs` is what exercises it",
        Some(&["supersede", "T-001", "a successor"]),
    ),
];

/// **The census: every command is styled or held, and held with a reason.**
///
/// Enumerated from `clap`'s own tree rather than from `--help`, so a command
/// that exists but has stopped being listed is still counted.
#[test]
fn every_command_is_either_styled_or_held() {
    let styled: std::collections::HashSet<&str> = STYLED.iter().map(|(name, ..)| *name).collect();
    let held: std::collections::HashSet<&str> = UNSTYLED.iter().map(|(name, ..)| *name).collect();

    let mut unclassified = Vec::new();
    let mut both = Vec::new();
    let mut total = 0_usize;
    // The *whole* tree, not its first level. `dep` has `add` and `rm` beneath it,
    // and a walk that stopped at the top classified `dep` and left its two
    // children unmeasured while calling itself a census (`A1-288`, audit).
    // `inert_flags.rs` walks the same tree the same way for flags.
    let mut stack: Vec<(String, clap::Command)> = aplan::cli_command()
        .get_subcommands()
        .map(|c| (c.get_name().to_string(), c.clone()))
        .collect();
    while let Some((path, command)) = stack.pop() {
        total += 1;
        match (styled.contains(path.as_str()), held.contains(path.as_str())) {
            (false, false) => unclassified.push(path.clone()),
            (true, true) => both.push(path.clone()),
            _ => {}
        }
        for sub in command.get_subcommands() {
            if sub.get_name() == "help" {
                continue; // `clap` generates one under every parent
            }
            stack.push((format!("{path} {}", sub.get_name()), sub.clone()));
        }
    }

    assert!(
        total > 25,
        "only {total} commands were walked, so the census is not reaching the tree"
    );
    assert!(
        unclassified.is_empty(),
        "{} command(s) are neither styled nor held: {}\n\
         Add each to `STYLED` with an invocation that makes it print, or to \
         `UNSTYLED` with the reason its output carries no role.",
        unclassified.len(),
        unclassified.join(", ")
    );
    assert!(
        both.is_empty(),
        "{} command(s) are both styled and held, so one of the two lists is lying: {}",
        both.len(),
        both.join(", ")
    );
    for (name, why, _) in UNSTYLED {
        assert!(
            why.len() > 30,
            "`{name}` is held with no reason worth reading: {why:?}"
        );
    }
}

/// **Everything in `STYLED` is styled, measured at a terminal.**
///
/// A pty and not a pipe: the default is colour at a terminal and none in a pipe,
/// so a census that captured a pipe would be measuring the half that emits
/// nothing and would pass just as well against a tool that had stopped styling
/// entirely.
#[test]
fn every_styled_command_emits_a_role_at_a_terminal() {
    let mut checked = 0_usize;
    for (_, args, marker) in STYLED {
        // A fixture each. Three of these write -- `add`, `close`, `drop` -- and a
        // census whose fifteenth command reads what its third one wrote is one
        // where the order is load-bearing and nobody wrote the order down.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        census_fixture(dir.path());
        let Some(text) = on_a_terminal_raw(dir.path(), 100, &[], args) else {
            return; // no `script(1)`; `on_a_terminal_raw` hard-fails under CI
        };
        // The marker first: a refusal is styled, so asserting only that
        // something was coloured is satisfied by a command that failed in red.
        assert!(
            text.contains(marker),
            "`aplan {}` did not print `{marker}`, so it did not do what this \
             census is about to say it styled: {text:?}",
            args.join(" ")
        );
        assert!(
            has_colour(&text),
            "`aplan {}` emitted no styling at a terminal: {text:?}",
            args.join(" ")
        );
        checked += 1;
    }
    assert!(
        checked >= 15,
        "only {checked} commands were exercised at a terminal"
    );
}

/// **No style is built outside the palette.**
///
/// `A1-287` put every role in one module so that swapping the palette is a
/// one-file edit. The state it started from is the argument for the rule: eleven
/// call sites built their own styles inline and seven of them were the same
/// `Style::new().bold()`, so "what does bold mean here" had eleven answers and
/// changing one changed nothing else. A convention alone lets the twelfth
/// re-declare it, which is how the first eleven happened.
///
/// Source-level, because there is no runtime signal for it: a style built inline
/// renders identically to one from the palette, so nothing watching the output
/// can tell them apart.
#[test]
fn no_renderer_builds_a_style_of_its_own() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let palette = root.join("crates/aplan-output/src/style.rs");

    let mut offenders = Vec::new();
    let mut scanned = 0_usize;
    for path in rust_sources(&root.join("crates")) {
        // The palette is where styles are built, and a test may name a role in
        // order to assert about it.
        if path == palette || path.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read");
        scanned += 1;
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if code.contains("anstyle::Style::new()") || code.contains("AnsiColor::") {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    n + 1,
                    code
                ));
            }
        }
    }

    assert!(
        scanned > 20,
        "only {scanned} source file(s) were scanned, so this is not reaching the workspace"
    );
    assert!(
        offenders.is_empty(),
        "{} style(s) built outside `aplan-output::style`, where the palette lives:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// **The CLI crate writes nothing to a raw stream.**
///
/// `A1-286` moved thirty-seven `write_stdout` calls, twenty `println!`, sixteen
/// `eprintln!` and seven bare `std::io::stdout()` onto `anstream`, which is what
/// made the tool's own refusals subject to `--color` at all. A move like that is
/// undone one call site at a time: the next `eprintln!` compiles, reads
/// naturally, and is invisible until somebody notices a refusal that ignores the
/// flag.
#[test]
fn the_cli_crate_writes_only_through_the_stream() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Every source in the crate, not the one file that had the writes:
    // `main.rs` is a shim today and a shim that grew a `println!` is exactly the
    // way this comes back.
    let sources = rust_sources(&root.join("crates/aplan/src"));
    assert!(
        sources.len() >= 2,
        "only {} source(s) found in the CLI crate",
        sources.len()
    );
    let text: String = sources
        .iter()
        .map(|p| std::fs::read_to_string(p).expect("read"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut offenders = Vec::new();
    let mut streams = 0_usize;
    for (n, line) in text.lines().enumerate() {
        if line.starts_with("#[cfg(test)]") {
            break; // the unit tests below may spell anything
        }
        let code = line.trim_start();
        // A comment naming one is prose about the seam rather than a use of it,
        // and this file's comments name `eprintln!` repeatedly, explaining why
        // it is not used.
        if code.starts_with("//") {
            continue;
        }
        if code.contains("anstream::stdout()") || code.contains("anstream::stderr()") {
            streams += 1;
        }
        for raw in [
            "println!",
            "eprintln!",
            "print!",
            "eprint!",
            "std::io::stdout()",
            // The spelling this rule did not have, and the one that was in the
            // crate: `assert_capabilities` locked a raw `std::io::stderr()` for
            // its refusal, so the strongest colour setting could not reach it
            // and `--color=never` could not strip it (`A1-291`).
            "std::io::stderr()",
        ] {
            // `IsTerminal::is_terminal(&std::io::stdout())` asks the stream a
            // question rather than writing to it, and a question has nothing to
            // strip.
            if code.contains(raw) && !code.contains("IsTerminal") {
                offenders.push(format!("{}: {}", n + 1, code));
            }
        }
    }

    // An exact count of streams was tried first and is the wrong assertion:
    // besides `write_stdout` and the two macros, four call sites need a `Write`
    // handle rather than a `&str` -- `serde_json::to_writer_pretty`, and the
    // loops in `drop` and `mv` -- and those are uses of the stream, not second
    // seams. What matters is that every stream opened here is an `anstream` one,
    // which the offender list is what enforces.
    assert!(
        streams > 0,
        "the CLI crate opens no `anstream` stream at all, so nothing it prints \
         can be styled or stripped"
    );
    assert!(
        offenders.is_empty(),
        "{} raw write(s) in the CLI crate, which `--color` cannot reach and \
         `anstream` cannot strip:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// Every `.rs` under `dir`, recursively.
fn rust_sources(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// **No human surface lets a plan write to the reader's terminal** (`A1-289`).
///
/// A plan is data. YAML refuses a raw control character and then hands one over
/// for a double-quoted escape, so a title can carry one, `check` is clean, and
/// the sequence reaches whoever reads the plan. Escapes are not only paint:
/// erase-display, set-window-title and switch-to-alternate-screen are all in the
/// same alphabet.
///
/// Half of `output.plan_text_cannot_steer_a_terminal`.
///
/// **This is a census and not a list, deliberately.** Sanitising at each
/// interpolation is the shape that leaves the eighth one out, so the assertion
/// is over the same `STYLED` table `every_styled_command_emits_a_role_at_a_terminal`
/// uses: every command a person reads, run against a plan whose every string
/// field is hostile, with every escape the palette itself emits accounted for
/// and nothing else allowed.
#[test]
fn no_human_surface_lets_a_plan_write_to_the_terminal() {
    // **Every command that can be run, not only the styled ones.** Leaking is a
    // property of printing a plan's text, and a held command prints it too:
    // `import` was passing a source document's escape straight through, and it
    // is held. An id can carry one as well -- YAML accepts the escaped form in
    // any scalar -- so "it prints only ids" is not a defence either (`A1-289`,
    // audit).
    let runs: Vec<&[&str]> = STYLED
        .iter()
        .map(|(_, args, _)| *args)
        .chain(UNSTYLED.iter().filter_map(|(.., run)| *run))
        .collect();

    let mut checked = 0_usize;
    for args in runs {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        hostile_fixture(dir.path());

        for choice in ["always", "never"] {
            let mut argv = vec![format!("--color={choice}")];
            argv.extend(args.iter().map(|a| (*a).to_string()));
            let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
            let text = common::run(dir.path(), &argv).text();

            // Every escape in the output must be one the palette emits. The
            // palette is SGR and nothing else, so a sequence ending in anything
            // but `m` came from the plan.
            let smuggled: Vec<String> = csi(&text)
                .into_iter()
                .filter(|seq| !seq.ends_with('m'))
                .collect();
            assert!(
                smuggled.is_empty(),
                "`aplan --color={choice} {}` passed a plan's own escape through to \
                 the terminal: {smuggled:?}",
                args.join(" ")
            );
            // And an SGR the plan supplied is just as much a write: the palette
            // never emits a background colour, so `[41m` is not ours.
            assert!(
                !text.contains("\u{1b}[41m"),
                "`aplan --color={choice} {}` passed a plan's own colour through: {text:?}",
                args.join(" ")
            );
            // The bidi half, which does not steer the terminal but steers the
            // reading of it, and which the palette never emits at all.
            let reordering: Vec<char> = text
                .chars()
                .filter(|c| {
                    matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}')
                        || ('\u{202a}'..='\u{202e}').contains(c)
                        || ('\u{2066}'..='\u{2069}').contains(c)
                })
                .collect();
            assert!(
                reordering.is_empty(),
                "`aplan --color={choice} {}` passed a plan's own bidi control \
                 through, so what a reader sees is not what the plan says: \
                 {reordering:?}",
                args.join(" ")
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 30,
        "only {checked} renderings were checked, so the table is not reaching the commands"
    );
}

/// The census fixture, with a control character in every string field a plan can
/// set: the title, the owner, a tag, a `paths` entry and a trace anchor.
fn hostile_fixture(dir: &Path) {
    census_fixture(dir);
    // The YAML *escape*, not a raw control character: a raw one is refused by
    // the parser, which is why this is not trivially reachable and why the first
    // draft of this fixture proved nothing -- every task failed to load and the
    // census passed on a plan with no titles in it.
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: \"a title carrying \\e[41m and \\e[2J and a tail long enough \
         to fold somewhere\"\nphase: 0\nstatus: todo\nowner: \"o\\e[41mwner\"\n\
         effort_days: 1\npaths:\n  - \"DOC\\e[41m.md\"\ntags: [deprecation]\n---\n\n\
         ## Why\n\nx\n",
    )
    .expect("hostile task");
    // A second hostile shape: text that *reads* as something else. The task id
    // carries one too, because YAML accepts the escaped form in any scalar and
    // an id is printed by almost every command (`A1-289`, audit).
    std::fs::write(
        dir.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: \"safe\\u202E evitcirtser\\u202C tail\"\nphase: 0\n\
         status: todo\nowner: t\neffort_days: 1\ntags: [removal]\nblocked_by: [T-001]\n\
         ---\n\n## Why\n\nx\n",
    )
    .expect("a reordering task");
    // What `import` reads, since a source document is untrusted in exactly the
    // way a plan is.
    std::fs::write(
        dir.join("SRC.md"),
        "# Plan\n\n- [ ] an imported title carrying \u{1b}[41m an escape\n",
    )
    .expect("hostile source");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
    };
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@example.invalid",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "hostile",
    ]);
}

/// **A machine format keeps the bytes the plan holds.**
///
/// The other half of `output.plan_text_cannot_steer_a_terminal`, and the reason
/// the repair is not at the loader: `json` already escapes a control character
/// to its `\uXXXX` form, so a parser receives what the plan actually says. A
/// consumer diffing two exports, or round-tripping through `import`, must not
/// have its input rewritten on the way past.
#[test]
fn a_machine_format_keeps_the_bytes_the_plan_holds() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    hostile_fixture(dir.path());

    let out = common::run(dir.path(), &["tasks", "--format", "json"]).stdout;
    let doc: serde_json::Value = serde_json::from_str(&out).expect("tasks emits JSON");
    let title = doc["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["id"] == "T-001")
        .map(|t| t["title"].as_str().expect("a title").to_string())
        .expect("T-001");

    assert!(
        title.contains('\u{1b}'),
        "the machine format lost the escape the plan holds, so a consumer \
         parsing it no longer sees what the plan says: {title:?}"
    );
    assert!(
        !title.contains(aplan_output::sanitize::MARKER),
        "the machine format was rewritten with the human marker: {title:?}"
    );
}

/// **A heading names the thing it is a heading for** (`A1-296`).
///
/// The role assertions above prove a surface carries `HEADING` somewhere. That
/// was true of `roadmap` before this task and told a reader nothing: the bold
/// reached the cut's *version* and stopped, so `v0.5` was emphasised and
/// `Roadmap self-hosting` -- the thing anyone scrolling is looking for -- was
/// not. What has to be asserted is which words are inside the span.
#[test]
fn a_heading_covers_the_name_and_not_only_its_label() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    census_fixture(dir.path());

    let heading = aplan_output::style::HEADING.render().to_string();
    let reset = "\u{1b}[0m";

    // The cut's own name is inside the bold span, with its version.
    let roadmap = common::run(dir.path(), &["--color=always", "roadmap"]).stdout;
    let span = roadmap
        .split(&heading)
        .nth(1)
        .and_then(|rest| rest.split(reset).next())
        .expect("a bold span in `roadmap`")
        .to_string();
    assert!(
        span.contains("A cut"),
        "the cut's heading is bold up to its version and stops, so the name a \
         reader scrolls for is the unemphasised part of the line it names: {span:?}"
    );

    // And the rollup behind it is not: emphasis in both directions or neither.
    assert!(
        roadmap.contains(&aplan_output::style::DIM.render().to_string()),
        "the rollup after the heading is not dimmed, so the heading has to carry \
         the contrast alone: {roadmap:?}"
    );

    // A section label is a heading too, in the two commands that print one.
    for (args, label) in [
        (&["--color=always", "show", "T-001"][..], "paths:"),
        (&["--color=always", "tree", "T-001"][..], "releases:"),
    ] {
        let text = common::run(dir.path(), args).stdout;
        assert!(
            text.contains(&format!("{heading}{label}")),
            "`aplan {}` prints `{label}` with no heading role, so a section label \
             is less findable than a column header: {text:?}",
            args.join(" ")
        );
    }
}

/// **Every section label carries the heading role** (`A1-296`, audit).
///
/// `a_heading_covers_the_name_and_not_only_its_label` names three labels by
/// hand, and a list of three stops covering the tool the moment a fourth is
/// printed. It did: `show`'s `findings: none` was plain while its `findings:`
/// with content below was bold, `tree`'s `rank:` was plain while `show`'s
/// `tags:` was bold, and `why`'s `is blocked by:` carried neither role. Three
/// spellings of one decision, disagreeing.
///
/// A label is recognised by shape rather than by a list: a line whose first
/// token is a lowercase word or two ending in a colon. That is what a section
/// label *is* in this tool's output, so a new one is covered by being printed.
#[test]
fn every_section_label_is_a_heading() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    census_fixture(dir.path());

    let heading = aplan_output::style::HEADING.render().to_string();
    // Every command that prints a labelled section, with the invocation that
    // makes it do so. `why` needs a task with blockers *and* one without, since
    // the two branches print different sentences and only one was styled.
    let runs: &[&[&str]] = &[
        &["show", "T-001"],
        &["show", "T-003"],
        &["tree", "T-001"],
        &["tree", "T-003"],
        &["why", "M-c1"],
        &["why", "T-001"],
        &["roadmap"],
        &["stale"],
        &["paths"],
        &["deprecations"],
    ];

    let mut offenders = Vec::new();
    let mut checked = 0_usize;
    for args in runs {
        let mut argv = vec!["--color=always"];
        argv.extend_from_slice(args);
        let text = common::run(dir.path(), &argv).text();
        for line in text.lines() {
            let bare = plain(line);
            let trimmed = bare.trim_start();
            // `<one or two lowercase words>:` at the start of the line, and not
            // an evidence reference (`test:name`) or a URL, which are content.
            let Some((label, rest)) = trimmed.split_once(':') else {
                continue;
            };
            // Two shapes, and the second was found by mutation: the first
            // version of this rule required the label to be the whole of what
            // precedes the colon, in lowercase, so `M-c1 is blocked by:` --
            // which begins with an id -- was skipped, and removing its role
            // did not fail this test.
            let inline = label.split(' ').count() <= 3
                && !label.is_empty()
                && label.chars().all(|c| c.is_ascii_lowercase() || c == ' ')
                && rest.starts_with(' ');
            // A line that *ends* in a colon introduces what follows it, whatever
            // precedes the label.
            let introduces = rest.is_empty() && trimmed.ends_with(':');
            if !inline && !introduces {
                continue;
            }
            checked += 1;
            // The masthead is deliberately unstyled: it sits where nothing
            // competes with it, and bolding it would compete with the cut
            // headings a reader is scrolling for. Named, so leaving it is a
            // decision this test records rather than a gap it hides.
            if matches!(label, "roadmap" | "throughput") {
                continue;
            }
            if !line.contains(&heading) {
                offenders.push(format!("`aplan {}`: {bare:?}", args.join(" ")));
            }
        }
    }

    assert!(
        checked >= 12,
        "only {checked} label(s) were found, so this is not reaching the sections"
    );
    assert!(
        offenders.is_empty(),
        "{} section label(s) carry no heading role, so one spelling of a label is \
         findable and another is not:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// **The one path by which a control character reaches a plan by accident says
/// so** (`A1-290`).
///
/// `A1-289` defends the reader of a plan. It tells the *author* nothing, and
/// `check` is silent -- correctly, since nothing about such a plan is invalid.
/// The question `A1-290` asked was whether it should also be a finding, and the
/// answer was no, measured: not one of 1,847 task files across seventeen plans
/// carries a control character, so a rule code would be reported by this tool's
/// own `vacuity` on every plan that exists.
///
/// What is true is that `import` is how one arrives without being typed: a
/// source document that picked up a terminal capture imports cleanly, and the
/// character is written back as YAML's escaped form. The person who ran the
/// import is the one who can fix it, and this is what tells them.
#[test]
fn an_import_says_when_a_source_carried_a_control_character() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("HOSTILE.md"),
        "# Plan\n\n- [ ] a title from a terminal capture \u{1b}[41m with an escape\n\
         - [ ] an ordinary title\n",
    )
    .expect("source");
    std::fs::write(
        dir.path().join("CLEAN.md"),
        "# Plan\n\n- [ ] an ordinary title\n",
    )
    .expect("source");

    let out = common::run(
        dir.path(),
        &[
            "import",
            "HOSTILE.md",
            "--format",
            "checklist",
            "--id-prefix",
            "IM",
        ],
    )
    .text();
    assert!(
        out.contains("1 imported field(s) carry a control character"),
        "an import that carried a terminal capture into the plan said nothing \
         about it: {out}"
    );

    // The escape really is in the written task, as the escaped form -- which is
    // what makes the notice worth printing rather than decorative.
    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/IM-001.md")).expect("read");
    assert!(
        written.contains("\\e[41m"),
        "the import did not carry the character into the plan, so this test is \
         measuring nothing: {written}"
    );

    // And a clean source says nothing, or the notice is noise on every import.
    let clean = common::run(
        dir.path(),
        &[
            "import",
            "CLEAN.md",
            "--format",
            "checklist",
            "--id-prefix",
            "CL",
            "--out",
            "other",
        ],
    )
    .text();
    assert!(
        !clean.contains("control character"),
        "a clean source was told about control characters: {clean}"
    );

    // **The second path: a manifest carries every field.** `add --from` reads a
    // generated manifest, which supplies `owner`, `tags` and `paths` as well as
    // the title, and a generator reading a terminal capture puts one wherever it
    // captured. Found by mutation: narrowing the count to titles broke no test
    // until this case existed.
    std::fs::write(
        dir.path().join("m.yml"),
        "- id: MF-001\n  title: an ordinary task\n  effort: 1\n  owner: \"o\\e[41mwner\"\n",
    )
    .expect("manifest");
    let bulk = common::run(dir.path(), &["add", "--from", "m.yml"]).text();
    assert!(
        bulk.contains("1 field(s) carry a control character"),
        "a manifest carried one into `owner` and `add --from` said nothing: {bulk}"
    );
    let written = std::fs::read_to_string(dir.path().join("aplan/tasks/MF-001.md")).expect("read");
    assert!(
        written.contains("\\e[41m"),
        "the manifest's character did not reach the plan: {written}"
    );

    // **And `import` reaches a field other than the title after all.** The first
    // version of this task said it could not, because every importer writes
    // `owner: None`. `kep` does not: it takes `owner` from the document's
    // `authors`, so a `kep.yaml` whose author line was pasted from a terminal
    // puts one straight into `owner` (`A1-290`, audit). That makes the walk over
    // `owner`, `tags` and `paths` required rather than defensive.
    std::fs::write(
        dir.path().join("k.yaml"),
        "title: A KEP\nkep-number: 1\nauthors:\n  - \"@auth\\e[41mor\"\n\
         status: implementable\nstage: alpha\n",
    )
    .expect("kep");
    let kep = common::run(
        dir.path(),
        &["import", "k.yaml", "--format", "kep", "--out", "kep-out"],
    )
    .text();
    assert!(
        kep.contains("1 imported field(s) carry a control character"),
        "`kep` carried one into `owner` and the import said nothing: {kep}"
    );
    let kep_task = std::fs::read_dir(dir.path().join("kep-out"))
        .expect("kep-out")
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).expect("read"))
        .find(|t| t.contains("owner:"))
        .expect("a task with an owner");
    assert!(
        kep_task.contains("owner: \"auth\\e[41mor\""),
        "the author's character did not reach `owner`, so this proves nothing: {kep_task}"
    );
}
