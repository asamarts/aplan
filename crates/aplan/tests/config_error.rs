//! An unknown config key says which of its two causes it is (`A1-068`).
//!
//! The first adopter added `test.covers` to `.aplan.yml` before staging the
//! binary that knows it, and their pre-push hook runs `aplan`, so the whole gate
//! refused to load and they could not push:
//!
//! ```text
//! aplan: loading .aplan.yml: could not parse `.aplan.yml`: unknown field `covers`,
//! expected one of `name`, `dir`, `list`, `run` at line 10 column 1
//! ```
//!
//! Rejecting unknown keys is deliberate and stays. What was missing is that the
//! sentence is *identical* for a misspelling and for a key from a newer `aplan`,
//! and the one fact that separates them, the running version, was the one thing
//! the tool did not print (pre-public adoption evidence).

use std::process::Command;

use crate::common;
use common::aplan;

/// An empty plan and the given `.aplan.yml`.
fn repo(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("config");
    dir
}

/// The version, unconditionally, because it is what tells the two cases apart.
#[test]
fn an_unknown_key_names_the_running_version_and_what_it_accepts() {
    let dir = repo("zzzzzzzz: 1\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "a config problem is a usage error:\n{out}");
    assert!(out.contains("unknown key `zzzzzzzz`"), "{out}");
    assert!(
        out.contains(&format!("this is aplan {}", env!("CARGO_PKG_VERSION"))),
        "the running version is the fact that settles it:\n{out}"
    );
    assert!(
        out.contains("config key(s); `aplan facts` lists them"),
        "{out}"
    );
    assert!(
        out.contains("reads exactly like a typo from here"),
        "the note has to say *why* the version matters:\n{out}"
    );
}

/// And a plain typo still reads as a typo. The note must not turn every
/// misspelling into a meditation on binary versions, so the correction leads.
#[test]
fn a_typo_still_reads_as_a_typo_and_gets_its_correction() {
    let dir = repo("requre_effort: false\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("unknown key `requre_effort` (did you mean `require_effort`?)"),
        "{out}"
    );
    // The suggestion comes before the version note, in that order, on the line an
    // adopter reads first.
    let suggestion = out.find("did you mean").expect("a suggestion");
    let note = out.find("this is aplan").expect("the version note");
    assert!(
        suggestion < note,
        "the version note buried the correction:\n{out}"
    );
}

/// The pilot's own case, one release on: a nested key the running build does not
/// know. Nothing about `test.covrs` is special except that it is where the
/// original failure was, and serde reported it as a bare `covers` with no path.
#[test]
fn a_nested_unknown_key_is_named_with_its_path() {
    let dir = repo("test:\n  name: x\n  list: [a]\n  run: [b]\n  covrs: [src/**]\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("unknown key `test.covrs` (did you mean `test.covers`?)"),
        "the key must be named with its path, not bare:\n{out}"
    );
}

/// Every offending key at once. An adopter who fixes one, re-runs, and meets the
/// next has been told half the answer twice.
#[test]
fn several_unknown_keys_are_all_named() {
    let dir = repo("polciy: strict\nzzzzzzzz: 1\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("unknown keys"), "the plural:\n{out}");
    assert!(out.contains("`polciy`"), "{out}");
    assert!(out.contains("`zzzzzzzz`"), "{out}");
}

/// A free-form mapping is the adopter's own vocabulary. Reporting it would make
/// a correct config fail with twenty invented mistakes, which is worse than the
/// defect this task fixes.
#[test]
fn a_declared_tag_vocabulary_is_not_reported_as_unknown_keys() {
    let dir = repo("tag_families:\n  epic: [epic-a, epic-b]\n  kind: [kind-r]\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "an adopter's own tag names were rejected:\n{out}");
}

/// A syntax error has one cause, so it must not carry a note about two. The
/// note would be noise on a file that never parsed and has no keys to judge.
#[test]
fn a_syntax_error_is_still_a_syntax_error_and_carries_no_version_note() {
    let dir = repo("require_effort: [not a bool\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("could not parse"), "{out}");
    assert!(!out.contains("unknown key"), "{out}");
    assert!(!out.contains("this is aplan"), "{out}");
}

/// Every command that reads a plan gets the same answer, and `--root` does not
/// change it.
///
/// Writing this found that it did. `ready`, `summary`, `tasks`, `critical-path`
/// and `graph` do not read config keys of their own, so they only met
/// `.aplan.yml` while resolving the root, and passing `--root` returned before
/// that. The same five commands exit 2 without the flag, so `--root` was
/// deciding the verdict: a confident answer, exit 0, from a repository whose
/// configuration could not load. That is the failure this repository already
/// names for a plan that did not load, one input over.
#[test]
fn every_command_reports_it_the_same_way_with_root_given() {
    let dir = repo("requre_effort: false\n");
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .output()
        .expect("git init");
    for args in [
        vec!["check", "--root", "aplan"],
        vec!["verify", "--root", "aplan"],
        vec!["ready", "--root", "aplan"],
        vec!["roadmap", "--root", "aplan"],
        vec!["summary", "--root", "aplan"],
        vec!["tasks", "--root", "aplan"],
        vec!["critical-path", "--root", "aplan"],
        vec!["graph", "--root", "aplan"],
    ] {
        let (out, code) = aplan(dir.path(), &args);
        assert_eq!(code, 2, "`{}` disagreed:\n{out}", args[0]);
        assert!(
            out.contains("did you mean `require_effort`?") && out.contains("this is aplan"),
            "`{}` reported it differently:\n{out}",
            args[0]
        );
    }
}

/// The control for the test above: without `--root` the same five commands
/// already failed, which is how the flag was shown to be the thing deciding it
/// rather than the command.
#[test]
fn the_same_commands_fail_without_the_flag_too() {
    let dir = repo("requre_effort: false\n");
    for cmd in ["ready", "summary", "tasks", "critical-path", "graph"] {
        let (out, code) = aplan(dir.path(), &[cmd]);
        assert_eq!(code, 2, "`{cmd}` disagreed:\n{out}");
    }
}

// ---------------------------------------------------------------------------
// What the audit of this task found. Reporting the unknown keys *replaced* the
// error rather than completing it, so whatever else was wrong with the file
// waited for the next run: the same half-an-answer-twice this task set out to
// stop, one error along.
// ---------------------------------------------------------------------------

/// `serde` stops at its first error, so a file with an unknown key above a type
/// error reports the key, and replacing that message with a better list of keys
/// still left the type error undiscovered. Sniffing `serde`'s sentence would not
/// have found it either, for exactly that reason.
#[test]
fn a_type_error_behind_an_unknown_key_is_reported_at_the_same_time() {
    let dir = repo("zzzzzzzz: 1\nrequire_effort: 5\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("unknown key `zzzzzzzz`"), "{out}");
    assert!(
        out.contains("once that is fixed the file still fails"),
        "{out}"
    );
    assert!(out.contains("expected a boolean"), "{out}");
}

/// Parsing is only half of loading. A runner with no `{test}` placeholder is
/// caught by a semantic check after the parse, and it is as much "what you meet
/// next" as a type error is, so the probe runs the same checks the load does.
#[test]
fn a_semantic_error_behind_an_unknown_key_is_reported_too() {
    let dir = repo("zzzzzzzz: 1\ntest:\n  name: x\n  list: [a]\n  run: [b]\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("unknown key `zzzzzzzz`"), "{out}");
    assert!(out.contains("must contain `{test}`"), "{out}");
}

/// The probe reports what the adopter will meet *after taking this advice*, so a
/// key with a correction is renamed rather than dropped. Dropping a misspelled
/// `test.lst` leaves a runner with no `list` and invents a missing-field error
/// the real fix never produces.
#[test]
fn a_correctable_typo_does_not_invent_a_missing_field() {
    // The runner is otherwise valid, including the `{test}` placeholder: a
    // fixture with a second real fault would prove nothing about the invented
    // one.
    let dir = repo("test:\n  name: x\n  lst: [a]\n  run: [b, '{test}']\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("unknown key `test.lst` (did you mean `test.list`?)"),
        "{out}"
    );
    assert!(
        !out.contains("still fails"),
        "correcting the spelling is the whole fix, and it claimed otherwise:\n{out}"
    );
    assert!(!out.contains("missing field"), "{out}");
}

/// The position `serde` reports belongs to the document with the unknown keys
/// already fixed, which is not the file the reader has. Quoting `line 1` for
/// something on line 2 of their file would be a confident wrong answer.
#[test]
fn the_second_error_does_not_claim_a_line_number_from_a_file_nobody_has() {
    let dir = repo("zzzzzzzz: 1\nrequire_effort: 5\n");
    let (out, _) = aplan(dir.path(), &["check", "--root", "aplan"]);
    let tail = out
        .split("still fails:")
        .nth(1)
        .expect("the second error")
        .to_string();
    assert!(
        !tail.contains(" at line "),
        "it quoted a position from the cleaned document:\n{tail}"
    );
}

/// The remedy the note names has to work in the state that produced the note.
/// `facts` and `--version` read no plan and must stay reachable when the config
/// cannot load, or the advice sends the reader to a second failure.
#[test]
fn the_commands_the_note_names_still_work_with_the_config_broken() {
    let dir = repo("requre_effort: false\n");
    for args in [vec!["facts"], vec!["--version"]] {
        let (out, code) = aplan(dir.path(), &args);
        assert_eq!(
            code, 0,
            "`{}` is unreachable when it is needed:\n{out}",
            args[0]
        );
    }
    // And `facts` really does list the keys the note promises.
    let (out, _) = aplan(dir.path(), &["facts"]);
    let doc: serde_json::Value = serde_json::from_str(&out).expect("facts json");
    assert!(
        doc["config_keys"]
            .as_array()
            .expect("config_keys")
            .iter()
            .any(|k| k == "require_effort"),
        "{out}"
    );
}
