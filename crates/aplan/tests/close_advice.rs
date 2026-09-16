//! **A refusal names a command that exists, and does the thing** (`A1-265`).
//!
//! `close` refused a second citation with *edit the rest by hand*, and
//! `aplan amend <ID> --add-evidence <CITATION>` already did that operation --
//! resolving the citation the way `close` resolves one and running the
//! post-state rules before writing. So the tool sent an author around both of
//! its own gates, to edit a YAML block it had just written.
//!
//! Two pilots hit it the same day and neither found the command from the
//! message: `p00020` (item 28) hand-edited five of the eight tasks it closed in
//! one pass, and `p00002` found `--add-evidence` "only by reading `amend
//! --help` on a guess".
//!
//! These tests *drive* the message rather than pinning its bytes: every command
//! it names is parsed back by clap, so a rename that leaves the sentence behind
//! fails here rather than in somebody's terminal.

use std::path::Path;

mod common;

/// A repository with one closed task and one live task that already cites
/// something, which are the two refusals that have a remedy.
fn repo(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("src/a.rs"), "fn main() {}\n").expect("src");
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
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git");
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    for (id, status) in [("T-1", "done"), ("T-2", "in_progress")] {
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: {status}\neffort_days: 1\n\
                 paths:\n- src/**\nevidence:\n  - kind: commit\n    ref: {sha}\n---\n\n\
                 ## Acceptance criteria\n\n- [x] done\n"
            ),
        )
        .expect("task");
    }
    sha
}

/// Every `aplan ...` invocation a string names, as argv.
fn invocations(text: &str) -> Vec<Vec<String>> {
    text.split('`')
        .filter(|s| s.starts_with("aplan "))
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .collect()
}

/// **Both refusals name `amend --add-evidence`, and clap accepts what they
/// name.**
///
/// The placeholders are substituted with real values first, so this asserts the
/// *shape* is runnable rather than that the literal string parses.
#[test]
fn a_refusal_that_has_a_remedy_names_a_command_that_runs() {
    let dir = tempfile::Builder::new()
        .prefix("close-advice-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    repo(dir.path());

    for (id, why) in [
        ("T-1", "a task that is already closed"),
        ("T-2", "a live task that already cites something"),
    ] {
        let out = common::run(dir.path(), &["close", id, "--evidence", "file:src/a.rs"]);
        assert_eq!(out.code, 2, "{why} was not refused: {out:?}");
        assert!(
            out.text().contains("--add-evidence"),
            "{why} must be told the command that adds one: {}",
            out.text()
        );
        assert!(
            !out.text().contains("by hand"),
            "and never told to edit the file the tool owns: {}",
            out.text()
        );

        let named = invocations(&out.text());
        assert!(!named.is_empty(), "no invocation named: {}", out.text());
        for argv in named {
            // `<kind>:<ref>` and `<from> <to>` are placeholders; give them
            // values so the shape is what is parsed.
            let filled: Vec<String> = argv
                .iter()
                .map(|a| {
                    a.replace("<kind>:<ref>", "file:src/a.rs")
                        .replace("<from>", "file:src/a.rs")
                        .replace("<to>", "file:src/b.rs")
                })
                .collect();
            aplan::cli_command()
                .try_get_matches_from(&filled)
                .unwrap_or_else(|e| {
                    panic!(
                        "`{}` is named by a refusal and clap rejects it: {e}",
                        filled.join(" ")
                    )
                });
        }
    }
}

/// **And the command it names does the thing.**
///
/// The message would still be wrong if `--add-evidence` refused what `close`
/// refused, so this runs it: the closed task gains the citation, and `verify`
/// resolves the result.
#[test]
fn the_named_command_adds_the_citation_close_would_not() {
    let dir = tempfile::Builder::new()
        .prefix("close-advice-run-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    repo(dir.path());

    let out = common::run(
        dir.path(),
        &["amend", "T-1", "--add-evidence", "file:src/a.rs"],
    );
    assert!(out.ok(), "the remedy failed: {out:?}");
    let task = std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read");
    assert!(task.contains("ref: src/a.rs"), "{task}");
    assert!(
        task.contains("status: done"),
        "and did not reopen it: {task}"
    );

    let verified = common::run(dir.path(), &["verify"]);
    assert!(
        verified.ok(),
        "the repair recorded what verify rejects: {verified:?}"
    );
}

/// **Where the tool says "by hand", no command of its own does the job.**
///
/// `A1-265` was one message: `close` refused a second citation and sent the
/// author to a text editor while `amend --add-evidence` did it through the same
/// gates. Its audit found a second -- a failed placement told the author to add
/// the task to a milestone by hand, which is `mv --cut` -- so this sweeps for
/// the shape rather than fixing instances as pilots trip over them.
///
/// It cannot decide whether a command exists for an arbitrary sentence, so it
/// asserts the reverse and keeps the list short: every remaining "by hand" is
/// named here with the reason no command can help, and a new one has to be
/// argued for in this file.
#[test]
fn a_hand_edit_is_only_suggested_where_no_command_does_it() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // (a distinctive phrase from the message, why no command can do it)
    //
    // Keyed on the message rather than the file: allowing a whole file would
    // pass the next "by hand" somebody adds to it, and `lib.rs` alone is eight
    // thousand lines. The first version of this audit did that, and found the
    // CLI twin of a message it had already allowed on the MCP side only
    // because the two live in different files.
    let allowed: &[(&str, &str)] = &[
        (
            "no top-level `{0}:` line this edit can rewrite",
            "there is nothing to rewrite: no edit this tool makes can guess what the author meant",
        ),
        (
            "could not edit `blocked_by`",
            "the same shape, on `blocked_by`",
        ),
        (
            "Add them to `roadmap.json` once",
            "it is the consumer's document, and inventing entries in it is what `export` refuses",
        ),
        (
            "`note` evidence needs a `captured` date",
            "neither surface invents a date, and both refuse it identically",
        ),
    ];

    let mut found = Vec::new();
    let mut scanned = String::new();
    for dir in [
        root.join("crates/aplan/src"),
        root.join("crates/aplan-core/src"),
        root.join("crates/aplan-interop/src"),
        root.join("crates/aplan-mcp/src"),
        root.join("crates/aplan-evidence/src"),
    ] {
        for entry in std::fs::read_dir(&dir).expect("crate src").flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read");
            let production = text
                .split_once("\n#[cfg(test)]")
                .map_or(text.as_str(), |(before, _)| before);
            scanned.push_str(production);
            // A message is a string literal that may span several source lines,
            // so the phrase identifying it is often not on the line carrying
            // "by hand". Match a window instead.
            for (at, _) in production.match_indices("by hand") {
                let before = production[..at].rfind('\n').map_or(0, |i| i + 1);
                let line_no = production[..at].matches('\n').count() + 1;
                let line = production[before..].lines().next().unwrap_or_default();
                if line.trim_start().starts_with("//") {
                    continue;
                }
                // "run it by hand" is a person running a command, which is the
                // opposite of what this is about.
                if line.contains("Run ") || line.contains("run ") {
                    continue;
                }
                let window = &production[at.saturating_sub(400)..at];
                if !allowed.iter().any(|(phrase, _)| window.contains(phrase)) {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    found.push(format!("{name}:{line_no}: {}", line.trim()));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "a message tells somebody to edit a file by hand from a file this audit does not \
         account for. If a command does that job, name the command; if none does, add the file \
         here with the reason (`A1-265`):\n  {}",
        found.join("\n  ")
    );
    // Every allowance must still match something, or the list silently becomes
    // a set of excuses for messages that no longer exist.
    for (phrase, why) in allowed {
        assert!(
            scanned.contains(phrase),
            "`{phrase}` is allowed here ({why}) and no message says it any more"
        );
    }
}
