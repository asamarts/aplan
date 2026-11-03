//! One format vocabulary, and one table saying who can emit what (`A1-184`).
//!
//! There were two vocabularies and ten allow-lists. `Format` held five names;
//! `markdown` was a sixth that `roadmap` and `deprecations` compared for as a
//! raw string, so the vocabulary the tool documents did not contain one of the
//! names it accepts. Per-command support was nine `narrow_format` calls carrying
//! a literal `&["human", "json"]` and a tenth written longhand inside `show`.
//! `aplan facts` reported `formats: 5`, which was true of no single command.
//!
//! The tests below run the real binary. A test that asked `format_for` whether a
//! format is allowed would be asking the table about the table: it would pass
//! just as happily if no command consulted it.

use std::collections::BTreeSet;
use std::path::Path;

use aplan::{COMMAND_SURFACES, NOT_AN_OUTPUT_FORMAT, surface_of};
use aplan_output::{Format, Surface};

use crate::common;

/// A plan small enough to load and rich enough that every command has something
/// to print.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("DOC.md"), "# Doc\n\n## intro\n\nbody\n").expect("doc");
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: a task\nphase: 0\nstatus: todo\nowner: t\neffort_days: 1\n\
         ---\n\n## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: another task\nphase: 0\nstatus: todo\nowner: t\neffort_days: 1\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 0\nstatus: todo\ntags: [milestone]\n\
         ---\n\n## Why\n\nA cut.\n",
    )
    .expect("cut");
    std::fs::write(
        dir.join("feature.blueprint.yml"),
        concat!(
            "schema: aplan/blueprint/v1\n",
            "name: format\n",
            "title: Format blueprint\n",
            "description: Exercise every action rendering.\n",
            "tasks:\n",
            "  - key: work\n",
            "    title: Render blueprint output\n",
            "    effort: 1\n",
        ),
    )
    .expect("blueprint");
    std::fs::write(dir.join("edges.yml"), "- id: T-001\n  add: [T-002]\n")
        .expect("dependency batch");
}

/// The arguments each command needs before `--format` is even reached.
fn extra_args(dir: &Path, command: &str) -> Vec<&'static str> {
    match command {
        // A command needing an argument must get one before `--format` is
        // reached at all: without it clap refuses first, every format looks
        // accepted, and this file silently stops testing the gate for that
        // command. `why` did exactly that when it gained the flag.
        "show" | "why" | "version" => vec!["T-001"],
        "test" => vec!["list"],
        "check-docs" => vec!["DOC.md"],
        "capture" => vec!["A format census capture", "--dry-run"],
        "blueprint" => vec![
            "apply",
            "--file",
            "feature.blueprint.yml",
            "--cut",
            "M-a",
            "--dry-run",
        ],
        "dep" => vec!["apply", "--from", "edges.yml", "--dry-run"],
        // The rendering tests invoke every accepted format against one fixture.
        // Rekey is necessarily mutating, so alternate the direction instead of
        // letting the second rendering become a missing-source refusal.
        "rekey" if dir.join("aplan/tasks/T-001.md").exists() => {
            vec!["T-001", "T-900"]
        }
        "rekey" => vec!["T-900", "T-001"],
        _ => Vec::new(),
    }
}

/// A renderable invocation for one declared command/format pair.
///
/// Redirected layered graph text deliberately requires an exact width. Keep
/// that presentation control on the text pair alone: machine/export formats
/// must continue to reject terminal-only controls.
fn format_args<'a>(dir: &Path, command: &'a str, value: &'a str) -> Vec<&'a str> {
    let mut args = vec![command];
    args.extend(extra_args(dir, command));
    args.extend(["--format", value]);
    if command == "graph" && value == "text" {
        args.extend(["--width", "240"]);
    }
    args
}

/// Whether the binary accepted `--format value` for `command`.
///
/// Acceptance is "it was not refused for the format", not "it exited zero": a
/// command may still report findings, and that is not this question.
fn accepts(dir: &Path, command: &str, value: &str) -> bool {
    let args = format_args(dir, command, value);
    let out = common::output(dir, &args);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    !text.contains("takes --format") && !text.contains("unknown format")
}

/// The census: a command that takes `--format` is in exactly one of the two
/// tables, and neither table names a command that does not exist.
///
/// Without this, a new command with a `--format` would accept every format in
/// the vocabulary -- including `markdown` on a set of findings -- and nothing
/// would say so.
#[test]
fn every_formatted_command_is_declared_or_held() {
    let declared: BTreeSet<&str> = COMMAND_SURFACES.iter().map(|(n, ..)| *n).collect();
    let held: BTreeSet<&str> = NOT_AN_OUTPUT_FORMAT.iter().map(|(n, _)| *n).collect();

    let cli = aplan::cli_command();
    let mut takes_format = BTreeSet::new();
    for sub in cli.get_subcommands() {
        if sub.get_arguments().any(|a| a.get_long() == Some("format")) {
            takes_format.insert(sub.get_name());
        }
    }

    assert!(
        takes_format.len() > 10,
        "only {} command(s) take --format, so the census is not reaching the tree",
        takes_format.len()
    );

    let unclassified: Vec<&&str> = takes_format
        .iter()
        .filter(|n| !declared.contains(*n) && !held.contains(*n))
        .collect();
    assert!(
        unclassified.is_empty(),
        "command(s) take `--format` but declare no surface, so they accept the whole \
         vocabulary silently: {unclassified:?}"
    );

    let both: Vec<&&str> = takes_format
        .iter()
        .filter(|n| declared.contains(*n) && held.contains(*n))
        .collect();
    assert!(both.is_empty(), "declared and held at once: {both:?}");

    for name in declared.iter().chain(held.iter()) {
        assert!(
            takes_format.contains(name),
            "`{name}` is in a format table but takes no `--format`"
        );
    }
    for (name, reason) in NOT_AN_OUTPUT_FORMAT {
        assert!(
            reason.len() > 20,
            "`{name}` is held with no real reason: {reason:?}"
        );
    }

    // A repeated row is not a compile error, and not a behaviour change until
    // the two rows disagree -- at which point the first silently wins and the
    // second is a declaration nothing reads.
    assert_eq!(
        declared.len(),
        COMMAND_SURFACES.len(),
        "a command is declared twice; the first row wins and the second is never read"
    );
    assert_eq!(
        held.len(),
        NOT_AN_OUTPUT_FORMAT.len(),
        "a command is held twice"
    );
}

/// No command bypasses the table.
///
/// What it can and cannot see is worth being exact about. It compares the binary
/// against the table the binary itself reads, so changing a row moves both sides
/// together and proves nothing about whether the row is *right*. What it does
/// prove is the thing that actually went wrong before: a command answering the
/// question on its own. `show` used to, in a longhand `match` nobody counted
/// among the nine, and reintroducing that makes this fail.
///
/// It walks `Format::ALL` and `COMMAND_SURFACES` rather than a list written
/// here, so a format or a command added to the table is tried against every
/// other on the commit that declares it.
#[test]
fn the_binary_accepts_exactly_what_the_surface_declares() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let mut wrong = Vec::new();
    let mut checked = 0_usize;
    for (command, surface, _) in COMMAND_SURFACES {
        for format in Format::ALL {
            let want = format.renders(*surface);
            let got = accepts(dir.path(), command, format.as_str());
            checked += 1;
            if want != got {
                wrong.push(format!(
                    "`aplan {command} --format {}`: the table says {want}, the binary says {got}",
                    format.as_str()
                ));
            }
        }
    }

    assert_eq!(
        checked,
        COMMAND_SURFACES.len() * Format::ALL.len(),
        "every command should have been tried with every format"
    );
    assert!(
        wrong.is_empty(),
        "{} pairing(s) where the binary and the table disagree:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}

/// `aplan facts` answers for the command, and answers the same thing the command
/// does.
///
/// The old answer was `formats: 5`, a vocabulary rather than a capability, so a
/// consumer asking what it could ask for was told about names no command would
/// accept.
#[test]
fn facts_reports_what_each_command_actually_accepts() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let out = common::output(dir.path(), &["facts"]);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("facts is JSON");

    let mut named = 0_usize;
    for entry in doc["commands"].as_array().expect("commands") {
        let name = entry["name"].as_str().expect("a name");
        let reported: Vec<&str> = entry["formats"]
            .as_array()
            .expect("every command entry carries a formats list")
            .iter()
            .map(|v| v.as_str().expect("a format name"))
            .collect();

        let Some(surface) = surface_of(name) else {
            assert!(
                reported.is_empty(),
                "`{name}` prints no formatted surface but facts lists {reported:?}"
            );
            continue;
        };
        named += 1;

        let expected: Vec<&str> = surface.formats().iter().map(|f| f.as_str()).collect();
        assert_eq!(
            reported, expected,
            "facts disagrees with the table for `{name}`"
        );

        // And the binary agrees with what facts said.
        for format in Format::ALL {
            let advertised = reported.contains(&format.as_str());
            assert_eq!(
                advertised,
                accepts(dir.path(), name, format.as_str()),
                "facts says `{name}` {} accept `{}`, the binary disagrees",
                if advertised { "does" } else { "does not" },
                format.as_str()
            );
        }
    }
    assert_eq!(
        named,
        COMMAND_SURFACES.len(),
        "facts missed a formatted command"
    );
}

/// `markdown` is a format, not a string somebody remembered to compare against.
#[test]
fn markdown_is_in_the_vocabulary_and_is_not_compared_as_a_string() {
    assert!(
        Format::ALL.iter().any(|f| f.as_str() == "markdown"),
        "markdown is still outside the vocabulary"
    );

    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("reading the CLI source");
    let offenders: Vec<&str> = source
        .lines()
        .filter(|l| l.contains(r#"== "markdown""#) || l.contains(r#""markdown" =="#))
        .collect();
    assert!(
        offenders.is_empty(),
        "`markdown` is still compared as a raw string, so the vocabulary has a second \
         member outside it:\n  {}",
        offenders.join("\n  ")
    );
}

/// The nine allow-lists are gone, and cannot come back quietly.
///
/// This is what "adding a format touches one row" means in practice: there is no
/// second place naming format strings, so a new row in `define_formats!` reaches
/// the parser, the refusal, every command's allow-list and `aplan facts` without
/// anything else being edited. A literal list of format names anywhere in the
/// CLI is the shape that made the old drift possible, so it is refused here
/// rather than left to review.
#[test]
fn no_hand_written_list_of_format_names_remains() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("reading the CLI source");

    let names: Vec<&str> = Format::ALL.iter().map(|f| f.as_str()).collect();
    let mut offenders = Vec::new();
    for (n, line) in source.lines().enumerate() {
        // Two or more format names quoted on one line is an allow-list, whatever
        // it is spelled as: `&["human", "json"]` was the old shape.
        let quoted = names
            .iter()
            .filter(|name| line.contains(&format!("\"{name}\"")))
            .count();
        if quoted >= 2 {
            offenders.push(format!("src/lib.rs:{}: {}", n + 1, line.trim()));
        }
    }

    assert!(
        offenders.is_empty(),
        "a hand-written list of format names is back, which is the drift `A1-184` removed. \
         Declare the surface in `COMMAND_SURFACES` instead:\n  {}",
        offenders.join("\n  ")
    );
}

/// Commands whose `--format` help deliberately names formats it does *not*
/// take, and why that is right rather than drift.
const HELP_NAMES_MORE_THAN_IT_TAKES: &[(&str, &str)] = &[
    (
        "show",
        "its help explains which report formats it refuses and why, which is the \
         documentation `A1-118` asked for rather than a list of what it accepts",
    ),
    (
        "inbox",
        "the parent is a report, while its nested `promote` operation returns an action and \
         therefore additionally accepts the agent action format; the executable matrix below \
         audits that deliberate split",
    ),
];

/// `inbox` normally renders a report, but promotion is an action result. Keep
/// that one nested widening executable rather than hiding it in conditional
/// adapter code or widening every inbox read to an agent action document.
#[test]
fn nested_inbox_promotion_accepts_exactly_the_action_formats() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let mut checked = 0_usize;
    for format in Format::ALL {
        let out = common::output(
            dir.path(),
            &[
                "inbox",
                "promote",
                "I-01K8Z3A4H00000000000000000",
                "--dry-run",
                "--format",
                format.as_str(),
            ],
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let accepted = !text.contains("takes --format") && !text.contains("unknown format");
        assert_eq!(
            accepted,
            format.renders(Surface::Action),
            "`inbox promote` and the action surface disagree for `{}`: {text}",
            format.as_str()
        );
        checked += 1;
    }
    assert_eq!(checked, Format::ALL.len());
}

/// The help text names exactly the formats the command takes.
///
/// The thirteenth list. `A1-184` removed ten allow-lists that *enforced* format
/// support and left thirteen doc comments that *described* it, which drift the
/// same way and are the half an adopter actually reads. They had: `check` and
/// `verify` advertised "human, json, agent, github" for as long as `sarif` had
/// existed, so the format the entire code-scanning integration depends on could
/// not be discovered from `--help`.
#[test]
fn the_help_names_exactly_the_formats_the_command_takes() {
    let exempt: BTreeSet<&str> = HELP_NAMES_MORE_THAN_IT_TAKES
        .iter()
        .map(|(n, _)| *n)
        .collect();
    let cli = aplan::cli_command();
    let mut wrong = Vec::new();
    let mut checked = 0_usize;

    for (command, surface, _) in COMMAND_SURFACES {
        let sub = cli
            .get_subcommands()
            .find(|c| c.get_name() == *command)
            .unwrap_or_else(|| panic!("`{command}` is not a subcommand"));
        let arg = sub
            .get_arguments()
            .find(|a| a.get_long() == Some("format"))
            .unwrap_or_else(|| panic!("`{command}` has no --format"));
        let help = format!(
            "{}{}",
            arg.get_help().map(ToString::to_string).unwrap_or_default(),
            arg.get_long_help()
                .map(ToString::to_string)
                .unwrap_or_default()
        );

        let named: BTreeSet<&str> = Format::ALL
            .iter()
            .map(|f| f.as_str())
            .filter(|name| help.contains(*name))
            .collect();
        let takes: BTreeSet<&str> = surface.formats().iter().map(|f| f.as_str()).collect();

        checked += 1;
        let missing: Vec<&&str> = takes.difference(&named).collect();
        if !missing.is_empty() {
            wrong.push(format!(
                "`{command}` takes {missing:?} but its help does not say so"
            ));
        }
        if !exempt.contains(command) {
            let extra: Vec<&&str> = named.difference(&takes).collect();
            if !extra.is_empty() {
                wrong.push(format!(
                    "`{command}` does not take {extra:?} but its help names them"
                ));
            }
        }
    }

    assert_eq!(checked, COMMAND_SURFACES.len());
    for (name, reason) in HELP_NAMES_MORE_THAN_IT_TAKES {
        assert!(
            reason.len() > 20,
            "`{name}` is exempt with no reason: {reason:?}"
        );
    }
    assert!(
        wrong.is_empty(),
        "the help disagrees with the surface, which is the half an adopter reads:\n  {}",
        wrong.join("\n  ")
    );
}

// --- what a declared pair actually renders (`A1-210`) ------------------------

/// The same plan, in a real repository and carrying a finding.
///
/// Both halves are load-bearing. `verify` opens git and exits before it renders
/// anything without one, so the acceptance walk above -- which runs in a bare
/// tempdir -- proves `verify`'s *refusals* honestly and never sees it render.
/// And a plan with no findings makes `agent`, `github` and `sarif` emit an empty
/// report, so a shape check against it would pass on a renderer that printed
/// nothing at all.
fn rendering_fixture(dir: &Path) {
    fixture(dir);
    // `T-404` does not exist, so `check` reports `APLAN003` and every
    // diagnostic format has a finding to render.
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: a task\nphase: 0\nstatus: todo\nowner: t\neffort_days: 1\n\
         blocked_by:\n- T-404\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    let repo = git2::Repository::init(dir).expect("git init");
    let mut index = repo.index().expect("index");
    index
        .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add");
    index.write().expect("write index");
    let tree = repo
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree");
    let sig = git2::Signature::now("t", "t@e.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "in", &tree, &[])
        .expect("commit");
}

/// What `command --format value` put on **stdout**.
///
/// Stdout only, because that is the half a consumer reads. A renderer that
/// wrote its JSON to stderr would pass a check that concatenated the two, and
/// the note `check` prints about unchecked citations goes to stderr precisely so
/// it does not corrupt the document on stdout.
fn rendered(dir: &Path, command: &str, value: &str) -> String {
    ran(dir, command, value).1
}

/// The exit code and stdout of `command --format value`.
fn ran(dir: &Path, command: &str, value: &str) -> (i32, String) {
    let args = format_args(dir, command, value);
    let out = common::output(dir, &args);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Which format a rendering *is*, decided from the bytes alone.
///
/// One classifier rather than nine predicates, so the shapes have to be mutually
/// exclusive: a predicate per format could be satisfied by the same output nine
/// times over, which is how a list checked against itself passes.
///
/// `human` and `text` share the `plain` answer because neither declares a marker
/// and no surface offers both -- `text` renders a graph, which has no `human` at
/// all. Within a surface the answer is still unambiguous.
///
/// **`plain` is a residual, and worth being exact about.** For the fifteen pairs
/// that expect it, this proves the output is non-empty and is not any of the
/// machine shapes -- not that it is the right prose. What carries the rest is
/// the exit code checked beside it, the sibling audit that no two of a command's
/// formats render the same bytes, and the `trycmd` corpus where the command has
/// a case. Four commands have none: `show`, `paths`, `vacuity` and `stale`
/// appear nowhere in `crates/aplan/tests/cmd/`, so for those this is the only
/// thing standing between a working human rendering and a broken one
/// (`A1-210` audit; the gap belongs to `A1-208`).
fn shape_of(text: &str) -> &'static str {
    let body = text.trim();
    if body.is_empty() {
        return "empty";
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        return if value.get("$schema").is_some_and(|s| {
            s.as_str()
                .is_some_and(|s| s.to_ascii_lowercase().contains("sarif"))
        }) {
            "sarif"
        } else if body.contains("agent_instruction") {
            "agent"
        } else {
            "json"
        };
    }
    if body.lines().any(|l| l.starts_with("::")) {
        return "github";
    }
    if body.contains("<!-- generated by") {
        return "markdown";
    }
    if body.starts_with("digraph") {
        return "dot";
    }
    if body.starts_with("graph ") || body.starts_with("flowchart") {
        return "mermaid";
    }
    "plain"
}

/// The shape a format is supposed to produce.
fn shape_wanted(format: Format) -> &'static str {
    match format {
        Format::Human | Format::Text => "plain",
        Format::Json => "json",
        Format::Agent => "agent",
        Format::Github => "github",
        Format::Sarif => "sarif",
        Format::Markdown => "markdown",
        Format::Mermaid => "mermaid",
        Format::Dot => "dot",
    }
}

/// **Every declared pair renders its own format**, run through the binary.
///
/// This replaces `every_format_is_exercised`, which built a list of all nine
/// `Format` variants and asserted `Format::ALL` was contained in it: a copy of
/// the enum compared against the enum, driving nothing. It could not fail, and
/// the doc comment above it had drifted to describing four formats (`A1-210`).
///
/// The unit is a (command, format) pair, not a format, because support is per
/// command: forty pairs, enumerated from `COMMAND_SURFACES` and
/// `Surface::formats()` rather than from a list kept here. And the audit does
/// not *survey* whether some other test happened to cover a pair -- it renders
/// every one itself, which is why it needs no record of what the suite ran.
///
/// What it adds to the acceptance walk above is the difference between accepted
/// and rendered: `accepts` asks only whether the refusal message is absent, so a
/// command that took `--format sarif` and then printed a human table, or
/// nothing, satisfied it.
#[test]
fn every_declared_pair_renders_its_own_format() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    rendering_fixture(dir.path());

    // The fixture earns its keep or the shape checks below are vacuous.
    assert_eq!(
        shape_of(&rendered(dir.path(), "check", "github")),
        "github",
        "the fixture stopped producing findings, so the diagnostic formats have \
         nothing to render and this audit proves nothing"
    );

    let mut wrong = Vec::new();
    let mut checked = 0_usize;
    let mut expected = 0_usize;
    for (command, surface, _) in COMMAND_SURFACES {
        expected += surface.formats().len();
        for format in surface.formats() {
            checked += 1;
            let (code, text) = ran(dir.path(), command, format.as_str());
            let (got, want) = (shape_of(&text), shape_wanted(format));
            if got != want {
                wrong.push(format!(
                    "`aplan {command} --format {}` rendered {got}, not {want}",
                    format.as_str()
                ));
            }
            // `0` clean and `1` errors are the two a rendering run can end on
            // (ADR-0007). `2` says the invocation was refused after all, which
            // is how `verify` passed the acceptance walk for years without ever
            // reaching a renderer; `3` is a bug, and a panicking renderer leaves
            // plausible half-output that the shape check reads as `plain`.
            if !matches!(code, 0 | 1) {
                wrong.push(format!(
                    "`aplan {command} --format {}` exited {code}; a declared pair renders, so \
                     the contract leaves it 0 or 1",
                    format.as_str()
                ));
            }
        }
    }

    assert_eq!(
        checked, expected,
        "a declared pair was skipped rather than rendered"
    );
    assert!(
        wrong.is_empty(),
        "{} declared pair(s) that do not render their own format:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}

/// A format that changes nothing is a format in name only.
///
/// The audit above would pass if two of a command's formats rendered the same
/// bytes and both happened to match their shape -- `human` and `text` share the
/// `plain` answer, so that is not hypothetical. This asks the sharper question:
/// within one command, no two declared formats produce identical output.
#[test]
fn no_two_formats_of_one_command_render_the_same_bytes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    rendering_fixture(dir.path());

    let mut same = Vec::new();
    for (command, surface, _) in COMMAND_SURFACES {
        let formats = surface.formats();
        let outputs: Vec<(Format, String)> = formats
            .iter()
            .map(|f| (*f, rendered(dir.path(), command, f.as_str())))
            .collect();
        for (index, (left, a)) in outputs.iter().enumerate() {
            for (right, b) in outputs.iter().skip(index + 1) {
                if a == b {
                    same.push(format!(
                        "`aplan {command}`: `{}` and `{}` render identical bytes",
                        left.as_str(),
                        right.as_str()
                    ));
                }
            }
        }
    }
    assert!(
        same.is_empty(),
        "{} pair(s) of formats that are the same output under two names:\n  {}",
        same.len(),
        same.join("\n  ")
    );
}

/// **No format is in the vocabulary and emitted by nothing.**
///
/// This is what `every_format_is_exercised` was reaching for, asked so that it
/// can fail. The two audits above walk the *declared* pairs, so narrowing a
/// surface's `renders` list moves the table and the binary together and neither
/// notices: delete `Surface::Diagnostics` from `Sarif` and there are simply
/// thirty-nine pairs, all of them fine.
///
/// `the_help_names_exactly_the_formats_the_command_takes` does fail on that
/// mutation, but for its own reason -- the help text and the surface stop
/// agreeing -- so the property would hold only as a side effect of a test about
/// something else, and only while the help is generated the way it is today.
///
/// A name no command accepts is the defect `A1-184` found in the other
/// direction, where `aplan facts` reported `formats: 5` -- a vocabulary rather
/// than a capability, true of no single command.
#[test]
fn every_format_in_the_vocabulary_is_emitted_by_some_command() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    rendering_fixture(dir.path());

    let mut orphaned = Vec::new();
    for format in Format::ALL {
        // The commands that declare it, and then the proof one of them really
        // renders it: a declaration is what this file exists to distrust.
        let emitted = COMMAND_SURFACES
            .iter()
            .filter(|(_, surface, _)| format.renders(*surface))
            .any(|(command, ..)| {
                shape_of(&rendered(dir.path(), command, format.as_str())) == shape_wanted(*format)
            });
        if !emitted {
            orphaned.push(format.as_str());
        }
    }
    assert!(
        orphaned.is_empty(),
        "format(s) in the vocabulary that no command emits: {}. A name an adopter \
         can read and no command will honour is a vocabulary, not a capability.",
        orphaned.join(", ")
    );
}
