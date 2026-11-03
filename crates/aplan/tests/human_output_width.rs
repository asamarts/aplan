//! No human output is wider than the terminal it is printed to (`A1-206`).
//!
//! Folding was applied renderer by renderer and the long tail was found the slow
//! way: run a command, look for a line over the width, fold that one, repeat.
//! Eight rounds. This is what makes the ninth unnecessary.
//!
//! **It has to use a pty, and that is the point.** Width is the decision colour
//! already makes here: `anstream` styles for a terminal and emits plain bytes
//! into a pipe, and folding follows it, because the defect is a *terminal*
//! folding a long line at its screen edge. Piped output is deliberately
//! untouched -- the committed `trycmd` corpus is byte-identical across this
//! change -- so a test that captured a pipe would be testing the half that does
//! nothing.
//!
//! The machine formats are excluded, not exempt: `machine_formats_unfolded.rs`
//! asserts the opposite property for them, that they are never folded. Between
//! the two files every command is covered by one rule or the other.

use std::path::Path;

use crate::common;

/// Whether the renderer added more width than the terminal has.
///
/// `A1-206` emits a token longer than the room whole, because a path or a URL is
/// the thing a reader most wants to copy and a break through the middle of one
/// is worse than a long line. A location row goes further and keeps `-->` with
/// its path, so that row is as long as the path plus a few columns whatever the
/// terminal is.
///
/// **Counting words was the first version of this, and it measured the wrong
/// property.** A one-word line was tolerated, but `  --> /a/long/path.md:6:1` is
/// two words. That passed only because fixtures were built in `/tmp`; putting
/// them on disk (`A1-257`) lengthened every fixture path and these gates failed
/// against a renderer that had not changed -- twice, once here and once on a
/// machine whose target directory is spelled differently again. A gate whose
/// result is chosen by the storage root is not measuring the renderer.
///
/// So it measures what the renderer *contributes*: the line minus its longest
/// unbreakable token. The limit of that, stated: a line carrying one very long
/// token and a little foldable prose passes, where in principle the prose could
/// have moved to the next row. Catching that would mean modelling where the
/// renderer is allowed to break, which is the renderer's own job -- and the
/// failure this gate exists for, a renderer that does not fold *at all*, leaves
/// hundreds of columns of prose behind and is caught either way.
fn renderer_added_too_much(line: &str, width: usize) -> bool {
    // Everything but the single longest token: the indent aligning a
    // continuation, the `-->` before a path, the prose around it. That is the
    // part the renderer chooses, and the only part it can fold away.
    let longest = line
        .split_whitespace()
        .map(aplan_output::fold::display_width)
        .max()
        .unwrap_or(0);
    aplan_output::fold::display_width(line).saturating_sub(longest) > width
}

#[test]
fn width_oracle_applies_the_progress_erase_before_measuring_output() {
    let captured = concat!(
        "\u{1b}[36m\u{280b} progress: building the requested view\u{1b}[0m",
        "\r\u{1b}[2K",
        "T-001  done  a fitted title\r\n"
    );
    assert_eq!(
        common::terminal_text(captured),
        "T-001  done  a fitted title\n"
    );
}

use common::on_a_terminal;

/// A plan rich enough that every command has something long to say: a title far
/// past any width, a `traces` anchor that does not resolve, and a second task so
/// rollups have more than one row.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("DOC.md"), "# Doc\n\n## elsewhere\n\nbody\n").expect("doc");
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: A deliberately long title that runs well past any sensible \
         terminal width so that every renderer has something it must decide what to do about\n\
         phase: 0\nstatus: done\nowner: someone\neffort_days: 3\ntags: [alpha, beta]\n\
         paths:\n  - DOC.md\ntraces:\n  - DOC.md#renamed\nevidence:\n  - kind: note\n    ref: seeded\n---\n\n\
         ## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/T-002.md"),
        "---\nid: T-002\ntitle: A second task, also with a title long enough to need folding \
         when it is printed beside a status and an effort in a table\nphase: 0\nstatus: todo\n\
         owner: someone\neffort_days: 2\nblocked_by: [T-001]\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    // The same defect reached through the content instead of the width: YAML
    // hands out a newline for the price of a `|-` block, and a title carrying
    // one puts its continuation at column zero however wide the terminal is.
    // A tab is that plus a measurement error, since `display_width` can only
    // count it as nothing. Both were live in five renderers when this fixture
    // had only well-behaved titles in it.
    std::fs::write(
        dir.join("aplan/tasks/T-003.md"),
        "---\nid: T-003\ntitle: |-\n  a title whose second line\n  would land at column zero and\tcarries a tab\n\
         phase: 0\nstatus: todo\nowner: someone\neffort_days: 1\n---\n\n## Why\n\nx\n",
    )
    .expect("hostile title");
    // **Each of these exists because a command was being measured against no
    // row at all** (`A1-291`). `stale` needs a repository whose history is
    // older than the window; `due` needs a `revisit_at`; `deprecations` needs a
    // `deprecation` and a `removal`; and `why` needs an *unmet* blocker, which
    // `T-001` is not, because it is `done`. Four of the commands in `HUMAN`
    // printed one line of prose apiece and this file called that a pass, while
    // five commands folded no rows at all.
    std::fs::write(
        dir.join("aplan/tasks/T-004.md"),
        "---\nid: T-004\ntitle: A parked task whose title is long enough that it must fold \
         under its own column rather than at the screen edge\nphase: 0\nstatus: blocked\n\
         owner: someone\neffort_days: 1\nrevisit_at: 2026-01-01T00:00:00Z\n\
         tags: [deprecation]\n---\n\n## Why\n\nx\n",
    )
    .expect("parked and announcing");
    std::fs::write(
        dir.join("aplan/tasks/T-005.md"),
        "---\nid: T-005\ntitle: A removal whose title also runs past any sensible width so that \
         the deprecations table has something to fold\nphase: 0\nstatus: todo\nowner: someone\n\
         effort_days: 1\ntags: [removal]\nblocked_by: [T-004]\n---\n\n## Why\n\nx\n",
    )
    .expect("removal");

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
    let mut first = who.to_vec();
    first.extend(["commit", "-qm", "plan"]);
    git(&first, Some("2025-01-01T00:00:00Z"));
    // A second, recent commit: `stale` measures from `HEAD`, so one commit makes
    // every task zero days idle however old that commit is.
    std::fs::write(
        dir.join("DOC.md"),
        "# Doc\n\n## elsewhere\n\nbody\n\nmore\n",
    )
    .expect("doc");
    git(&["add", "-A"], None);
    let mut later = who.to_vec();
    later.extend(["commit", "-qm", "later"]);
    git(&later, None);
    std::fs::write(
        dir.join("aplan/tasks/M-c1.md"),
        "---\nid: M-c1\ntitle: A cut whose own summary line carries counts and a derived \
         duration\nphase: 0\nstatus: todo\ntags: [milestone]\nblocked_by: [T-001, T-002, T-003]\n---\n\n\
         ## Why\n\nx\n",
    )
    .expect("milestone");
}

/// Every command a person reads at a terminal, with arguments that make each one
/// print something.
const HUMAN: &[(&[&str], &str)] = &[
    // The fixture carries findings on purpose, so what `check` renders is the
    // diagnostic block rather than a clean verdict.
    (&["check"], "error["),
    (&["test", "list"], "test catalogue:"),
    (&["ready"], "T-002"),
    (&["roadmap"], "roadmap:"),
    (&["summary"], "effort"),
    (&["stale"], "idle"),
    (&["due", "--as-of", "2027-01-01T00:00:00Z"], "revisit"),
    (&["vacuity"], "rule/task pair"),
    (&["deprecations"], "announced in"),
    (&["paths"], "commit(s) sampled"),
    (
        &[
            "capture",
            "A captured idea whose title is deliberately long enough to require wrapping on a narrow terminal",
            "--source",
            "width:census",
            "--body",
            "A body with enough explanatory prose to require wrapping under its field indentation as well",
        ],
        "capture",
    ),
    (&["inbox"], "source=width"),
    (&["critical-path"], "critical path:"),
    // The target-bounded form puts an id a reader chose into the header, so the
    // line's length stopped being the renderer's alone (`A1-345`).
    (&["critical-path", "T-002"], "critical path to"),
    (&["tasks"], "status"),
    (&["tree", "T-002"], "waits on:"),
    (&["show", "T-001"], "phase 0"),
    (&["explain", "rule", "APLAN012"], "APLAN012"),
    // `why` on a task whose blocker is `done` prints one sentence and no rows,
    // which is what this file was measuring before `A1-291`.
    (&["why", "T-005"], "is blocked by"),
    // T-003 is the one carrying hostile whitespace, so the renderers that print
    // a single task have to be pointed at it or they never see the case.
    (&["show", "T-003"], "phase 0"),
    (&["tree", "T-003"], "T-003"),
    (&["graph", "--ascii"], "level: plan"),
];

/// Commands whose output no person reads as prose, and why each is out of scope
/// here. Every one is covered by something else: the machine formats by
/// `machine_formats_unfolded.rs`, which asserts the opposite property, and the
/// mutating commands by their own suites.
///
/// This exists so that `every_command_is_either_gated_or_held` can be a census
/// rather than a sample. `HUMAN` above is a list somebody typed; a list somebody
/// typed drifts the moment a command is added, and the drift is silent -- the
/// suite goes on passing while the new renderer is never measured. Naming the
/// exclusions makes adding a command a choice between two lists instead of an
/// omission from one.
const NOT_PROSE: &[(&str, &str)] = &[
    ("add", "writes a task and prints its path"),
    (
        "blueprint",
        "prints correctness-bearing file paths, task ids and SHA-256 identities whose exact tokens must not be re-flowed",
    ),
    (
        "version",
        "one word from a closed vocabulary, or a version number: there is nothing to \
         re-flow, and a substituted answer must not gain a line break",
    ),
    ("amend", "writes a task and prints its path"),
    // `A1-359`'s four: each edits one task and prints the id, the new state and
    // the path. The one variable-length part is a name somebody chose, and
    // re-flowing a path is what the width gate exists to prevent elsewhere.
    ("claim", "writes a task and prints its path"),
    ("release", "writes a task and prints its path"),
    ("start", "writes a task and prints its path"),
    ("stop", "writes a task and prints its path"),
    (
        "attest",
        "prints a signed digest, which must not be re-flowed",
    ),
    (
        "check-docs",
        "a drift gate: its output is a diff, not prose",
    ),
    ("close", "writes a task and prints its path"),
    (
        "dep",
        "writes an edge and prints a confirmation; its refusal is prose and is \
         gated by a_refusal_folds_and_keeps_the_shape_it_was_written_with",
    ),
    ("drop", "writes a task and prints its path"),
    (
        "tag",
        "writes a task and prints one sentence naming what the reader typed; its \
         refusal is the shared mutation refusal, gated by \
         a_refusal_folds_and_keeps_the_shape_it_was_written_with",
    ),
    ("export", "writes a document in somebody else's shape"),
    ("export-agents-md", "writes a document"),
    ("export-roadmap-json", "a machine format"),
    ("facts", "a machine format"),
    ("help", "clap renders it, gated by help_wraps.rs (A1-205)"),
    ("import", "writes tasks and prints a count"),
    ("init", "writes a config and prints its path"),
    ("mcp", "speaks JSON-RPC on a pipe, never to a terminal"),
    (
        "park",
        "writes tasks and prints a fixed confirmation count and ids",
    ),
    (
        "tui",
        "owns the whole terminal grid rather than printing prose lines; fixed-size buffer and PTY tests gate its layout",
    ),
    ("mv", "writes tasks and prints a confirmation"),
    (
        "rekey",
        "writes tasks and prints a migration confirmation plus unbreakable path-and-line review locations",
    ),
    ("schema", "a machine format"),
    ("supersede", "writes a task and prints its path"),
    (
        "unpark",
        "writes tasks and prints a fixed confirmation count and ids",
    ),
    (
        "verify",
        "its findings are the same renderer `check` uses and are gated there; \
         its own lines are a fixed summary",
    ),
];

/// The census. Every command the tool has is either measured by `HUMAN` or
/// named in `NOT_PROSE` with a reason.
///
/// Enumerated from `clap`'s own tree rather than from `--help` text, so it
/// cannot be satisfied by a command that exists but has stopped being listed.
#[test]
fn every_command_is_either_gated_or_held() {
    let gated: std::collections::HashSet<&str> = HUMAN.iter().map(|(args, _)| args[0]).collect();
    let held: std::collections::HashSet<&str> = NOT_PROSE.iter().map(|(name, _)| *name).collect();

    let mut unclassified = Vec::new();
    let mut both = Vec::new();
    let mut total = 0_usize;
    for sub in aplan::cli_command().get_subcommands() {
        let name = sub.get_name();
        total += 1;
        match (gated.contains(name), held.contains(name)) {
            (false, false) => unclassified.push(name.to_string()),
            (true, true) => both.push(name.to_string()),
            _ => {}
        }
    }

    assert!(
        total > 25,
        "only {total} commands were walked, so the census is not reaching the tree"
    );
    assert!(
        unclassified.is_empty(),
        "{} command(s) are neither width-gated nor held: {}\n\
         Add each to `HUMAN` if a person reads its output as prose, or to \
         `NOT_PROSE` with the reason it is out of scope.",
        unclassified.len(),
        unclassified.join(", ")
    );
    assert!(
        both.is_empty(),
        "{} command(s) are both gated and held, so one of the two lists is lying: {}",
        both.len(),
        both.join(", ")
    );

    // A held entry with no reason is an omission wearing a classification.
    for (name, reason) in NOT_PROSE {
        assert!(
            reason.len() > 15,
            "`{name}` is held with no real reason given: {reason:?}"
        );
    }
}

#[test]
fn nothing_a_person_reads_is_wider_than_their_terminal() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let mut offenders = Vec::new();
    let mut examined = 0_usize;

    // **More than one width.** A single width is a constant a renderer can
    // happen to match: this file measured 95 and nothing else, and five
    // commands that folded no rows at all passed it (`A1-291`). Sixty is
    // narrower than the widest table the tool prints, which is the case a
    // reader on a split screen actually has.
    for width in [60, 80, 95] {
        for (args, marker) in HUMAN {
            let Some(text) = on_a_terminal(dir.path(), width, args) else {
                eprintln!("skipping: script(1) is not available on this machine");
                return;
            };
            // **A command printing no row is a failure of this gate, not a
            // pass.** Four of the commands below were measured against one line
            // of prose apiece, because the fixture gave `stale` no history,
            // `due` no `revisit_at`, `deprecations` no tagged task and `why` no
            // unmet blocker. Nothing is wider than the terminal when nothing is
            // printed.
            assert!(
                text.contains(marker),
                "`aplan {}` did not print `{marker}` at {width} columns, so the \
                 rows this gate is about were never rendered:\n{text}",
                args.join(" ")
            );
            for line in text.lines() {
                examined += 1;
                let w = aplan_output::fold::display_width(line);
                // A line the renderer could not have folded is not a fold
                // failure. At sixty columns this covers ordinary words as well
                // as paths: `aplan tasks` leaves nine columns for the title and
                // `deliberately` is twelve.
                if w > width && renderer_added_too_much(line, width) {
                    offenders.push(format!(
                        "`aplan {}` at {width}: {w} columns: {line}",
                        args.join(" ")
                    ));
                }
            }
        }
    }

    assert!(
        examined > 200,
        "only {examined} lines were produced, so the fixture is not exercising the renderers"
    );
    assert!(
        offenders.is_empty(),
        "{} line(s) run past the terminal, which is what makes the terminal break \
         them at column zero:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// No human line carries whitespace a terminal reacts to.
///
/// A newline or a tab inside a value is the module's own defect arriving
/// through the content: the newline breaks the row at column zero on any width,
/// and the tab is measured as nothing by [`display_width`], so a line carrying
/// one passes the width test above while rendering wider than it claims. Both
/// were live in `show`, `tree`, `ready`, `critical-path` and `roadmap` while
/// this file's fixture had only well-behaved titles in it.
#[test]
fn no_human_line_carries_whitespace_the_terminal_reacts_to() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let mut offenders = Vec::new();
    let mut seen = 0_usize;
    for (args, _) in HUMAN {
        let Some(text) = on_a_terminal(dir.path(), 95, args) else {
            eprintln!("skipping: script(1) is not available on this machine");
            return;
        };
        for line in text.lines() {
            seen += 1;
            if let Some(c) = line.chars().find(|c| c.is_whitespace() && *c != ' ') {
                offenders.push(format!("`aplan {}`: {c:?} in: {line:?}", args.join(" ")));
            }
        }
    }

    assert!(
        seen > 60,
        "only {seen} lines, so the fixture is not exercising the renderers"
    );
    assert!(
        offenders.is_empty(),
        "whitespace that moves the cursor reached a rendered line ({} case(s)):\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// A newline inside a value leaves no trace on the line it created, so the test
/// above cannot see it: both halves come out clean and narrow. What gives it
/// away is *where* the second half starts. Folding always indents a
/// continuation -- that is the whole of `A1-206` -- so a mid-title fragment
/// sitting at column zero is a line the renderer never chose to break.
#[test]
fn a_continuation_never_starts_at_column_zero() {
    /// A fragment from the middle of `T-003`'s title, which can only begin a
    /// line if something other than the fold put it there. It deliberately sits
    /// ahead of the tab in that title: with the tab in front of it, the raw
    /// line begins with the tab and this test sees nothing, which is exactly
    /// how the first draft of it passed against the unfixed renderer.
    const MID_TITLE: &str = "would land at column zero";

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let mut offenders = Vec::new();
    let mut proved = 0_usize;
    for (args, _) in HUMAN {
        let Some(text) = on_a_terminal(dir.path(), 95, args) else {
            eprintln!("skipping: script(1) is not available on this machine");
            return;
        };
        if text.contains(MID_TITLE) {
            proved += 1;
        }
        for line in text.lines() {
            if line.starts_with(MID_TITLE) {
                offenders.push(format!("`aplan {}`: {line:?}", args.join(" ")));
            }
        }
    }

    assert!(
        proved >= 3,
        "only {proved} commands printed the hostile title, so this asserts almost nothing"
    );
    assert!(
        offenders.is_empty(),
        "a title's second line began at column zero, which is the defect `A1-206` \
         exists to remove ({} case(s)):\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// The same at a width nothing was tuned for, so the fold is computed rather
/// than a constant that happens to match.
#[test]
fn a_narrower_terminal_is_honoured_too() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let Some(text) = on_a_terminal(dir.path(), 60, &["roadmap"]) else {
        eprintln!("skipping: script(1) is not available on this machine");
        return;
    };
    for line in text.lines() {
        assert!(
            aplan_output::fold::display_width(line) <= 60,
            "{} columns on a 60-column terminal: {line}",
            aplan_output::fold::display_width(line)
        );
    }
}

/// A refusal is prose, whoever printed it.
///
/// The mutating commands are held out of `HUMAN` because their normal output is
/// a one-line confirmation. What they also print is the most-read prose the tool
/// has: the refusal that stops a bad edit, a headline with the findings listed
/// under it. `A1-206` folded that whole message as one string, so on a terminal
/// the remedy ran into the finding above it -- `... -> A-01 fix: Break the
/// cycle ...` -- while the piped corpus went on showing the right shape, because
/// a pipe is unbounded and nothing folded at all.
#[test]
fn a_refusal_folds_and_keeps_the_shape_it_was_written_with() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // T-002 is already blocked by T-001, so this closes a cycle and is refused
    // with `APLAN004`, whose remedy is long enough to fold at any usual width.
    let Some(text) = on_a_terminal(dir.path(), 95, &["dep", "add", "T-001", "--after", "T-002"])
    else {
        eprintln!("skipping: script(1) is not available on this machine");
        return;
    };

    assert!(
        text.contains("APLAN004"),
        "the edit was not refused, so this asserts nothing:\n{text}"
    );
    assert!(
        text.lines().any(|l| l.starts_with("  APLAN004")),
        "the finding lost the indent that separates it from the headline:\n{text}"
    );
    assert!(
        text.lines().any(|l| l.trim_start().starts_with("fix:")),
        "the remedy was run into the finding above it:\n{text}"
    );
    for line in text.lines() {
        assert!(
            aplan_output::fold::display_width(line) <= 95,
            "{} columns: {line}",
            aplan_output::fold::display_width(line)
        );
    }
}

/// The control: a pipe must be left alone, or the corpus and every script
/// reading this tool would move under them.
#[test]
fn a_pipe_is_not_folded() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let out = common::output(dir.path(), &["check"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        text.lines().any(|l| l.chars().count() > 95),
        "piped output was folded: it must stay one line per message so `grep` still \
         works and the committed goldens do not move:\n{text}"
    );
}

/// **A command folds against the stream it is writing to** (`A1-291`).
///
/// `A1-206` gave the tool two ways to ask for a width: `out_width()` reads
/// stdout, `err_width()` reads stderr. Four commands built text for stdout and
/// folded it against stderr -- `ready`, `why`, `import` and `check-docs` -- and
/// every test in this file passed, because `script(1)` attaches a pty to *both*
/// streams and the two answers are then the same number. The harness measured
/// the one configuration in which the mistake cannot appear.
///
/// Here stderr has no terminal, so `err_width()` returns "do not fold". A
/// renderer that asked the wrong one emits its longest line whole:
///
/// ```text
/// A1-183  Every policy dial is global, and one subset-aware key shows what scopi
/// ```
///
/// The fix is `fold::row`, which takes no width at all and asks stdout itself,
/// so a call site cannot pass the other one.
#[test]
fn stdout_is_folded_even_when_stderr_is_not_a_terminal() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let width = 70;
    let mut offenders = Vec::new();
    let mut examined = 0_usize;
    for (args, marker) in HUMAN {
        let Some(text) = common::on_a_terminal_stderr_piped(dir.path(), width, args) else {
            eprintln!("skipping: script(1) is not available on this machine");
            return;
        };
        assert!(
            text.contains(marker),
            "`aplan {}` printed no rows with stderr redirected, so this proves \
             nothing about it:\n{text}",
            args.join(" ")
        );
        for line in text.lines() {
            examined += 1;
            let w = aplan_output::fold::display_width(line);
            if w > width && renderer_added_too_much(line, width) {
                offenders.push(format!("`aplan {}`: {w} columns: {line}", args.join(" ")));
            }
        }
    }

    assert!(
        examined > 60,
        "only {examined} lines were produced with stderr redirected"
    );
    assert!(
        offenders.is_empty(),
        "{} line(s) folded against the wrong stream's width: stdout is a terminal \
         and stderr is not, so a renderer asking `err_width()` was told not to \
         fold at all:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}
