//! One spawn, and a record of what the suite ran (`A1-207`).
//!
//! Forty-eight files started the binary and thirty-six defined a helper to do
//! it, twenty-five of them byte-identical. The duplication was the visible half;
//! the reason it came first in its group is that `A1-208`, `A1-209` and `A1-211`
//! all need to know what the suite actually invoked, and **for a Rust test the
//! only honest source is the spawn itself**. Reading it out of the test sources
//! was tried twice and failed twice: counting command names counts the word
//! `check` in prose, and attributing flags by position misses `--config`, which
//! `trusted_config.rs` passes *before* the subcommand.
//!
//! The `trycmd` corpus is the other half and needs no observing: `trycmd` spawns
//! the binary itself, so those 126 invocations never reach this module, and they
//! are declared verbatim in `tests/cmd/*.trycmd` anyway.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::common;

/// The files a test author writes, which is what the gate below reads.
fn test_sources() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("reading tests/").flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path
                .file_name()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            out.push((
                name,
                std::fs::read_to_string(&path).expect("reading a test"),
            ));
        }
    }
    out.sort();
    assert!(
        out.len() > 40,
        "the test directory did not enumerate, so this gate reads nothing: {} file(s)",
        out.len()
    );
    out
}

/// A file may name the binary without spawning it, and one does.
const NAMES_THE_BINARY_WITHOUT_RUNNING_IT: &[(&str, &str)] = &[];

/// The environment variable cargo sets to the binary's path, spelled in two
/// halves so that **this file does not contain it**.
///
/// A gate that names what it forbids has to exempt itself, and an exemption on
/// the gate's own file is a hole in the one place nobody thinks to look. Joining
/// it here costs a line and keeps the scan total (`A1-207`).
fn the_token() -> String {
    format!("{}_{}", "CARGO_BIN_EXE", "aplan")
}

/// **No test spawns the binary except through `common`.**
///
/// The scan looks for cargo's path variable rather than for `Command::new`,
/// because a file that bound the path to a local first would pass the narrower
/// reading while spawning exactly as before -- and this exists to catch the next
/// copy, which will not look like the last twenty-five.
#[test]
fn no_test_file_reaches_for_the_binary_itself() {
    let exempt: BTreeSet<&str> = NAMES_THE_BINARY_WITHOUT_RUNNING_IT
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let mut offenders = Vec::new();
    let mut scanned = 0_usize;
    for (name, body) in test_sources() {
        scanned += 1;
        if !body.contains(&the_token()) || exempt.contains(name.as_str()) {
            continue;
        }
        offenders.push(name);
    }
    assert!(scanned > 40, "only {scanned} file(s) were scanned");

    // An exemption is for naming the binary, never for running it. Without this
    // the exempt file is the one place a spawn could be added back, which is a
    // hole in the file nobody rereads (`A1-207`, audit).
    let sources = test_sources();
    for (name, reason) in NAMES_THE_BINARY_WITHOUT_RUNNING_IT {
        assert!(
            reason.len() > 30,
            "`{name}` is exempt with no reason worth reading: {reason:?}"
        );
        let (_, body) = sources
            .iter()
            .find(|(f, _)| f == name)
            .unwrap_or_else(|| panic!("`{name}` is exempt and does not exist"));
        // `Command::new` alone is too broad: this file also runs `git`. What
        // the exemption must not cover is a process started from *the binary's
        // own path*, so the check reads what precedes each mention of it.
        let started: Vec<usize> = body
            .match_indices(&the_token())
            .map(|(at, _)| at)
            .filter(|at| body[..*at].ends_with("Command::new(env!(\""))
            .collect();
        assert!(
            started.is_empty(),
            "`{name}` is exempt from naming the binary, and now starts {} process(es) with it",
            started.len()
        );
    }
    assert!(
        offenders.is_empty(),
        "{} file(s) start the binary themselves instead of through `common`:\n  {}\n\
         Use `common::run`, `common::output`, `common::command` or \
         `common::tui_terminal`. Every spawn goes \
         through one place so the suite can record what it invoked ({}).",
        offenders.len(),
        offenders.join("\n  "),
        common::RECORD_TO
    );
}

/// Cargo may replace its executable path while an integration-test process is
/// still running. The helper holds the inode it first saw, not merely that
/// mutable path, and every process resolves that build to the same hard link
/// rather than copying its hundreds of megabytes (`A1-242`, `A1-399`).
#[test]
fn the_binary_under_test_is_a_snapshot_not_a_mutable_cargo_path() {
    use std::os::unix::fs::MetadataExt as _;

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let source = dir.path().join("aplan-source");
    std::fs::write(&source, b"first build").expect("write first build");

    let first = common::BinarySnapshot::capture(&source);
    let second = common::BinarySnapshot::capture(&source);
    let source_metadata = source.metadata().expect("source metadata");
    let snapshot_metadata = first.path().metadata().expect("snapshot metadata");

    assert_eq!(first.path(), second.path(), "one inode gets one cache path");
    assert_eq!(
        (source_metadata.dev(), source_metadata.ino()),
        (snapshot_metadata.dev(), snapshot_metadata.ino()),
        "the snapshot must be a hard link, not a second allocation"
    );

    let replacement = dir.path().join("aplan-replacement");
    std::fs::write(&replacement, b"second build").expect("write second build");
    std::fs::rename(&replacement, &source).expect("replace the source path atomically");

    assert_eq!(
        std::fs::read(first.path()).expect("read snapshot"),
        b"first build",
        "a rebuild replaced the source path, not the artifact this test process resolved"
    );
}

/// The recording is one line per invocation, and the line is the argv as given.
///
/// JSON rather than a joined string because an argument may contain a space:
/// `aplan add "Ship it"` is in the corpus, and a reader splitting on spaces
/// would see two arguments where the suite passed one.
#[test]
fn a_recorded_invocation_is_the_argument_vector_as_given() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let log = dir.path().join("invocations.jsonl");

    common::record_to(&log, &["check", "--format", "json"]);
    common::record_to(&log, &["add", "Ship it", "--effort", "1"]);

    let text = std::fs::read_to_string(&log).expect("the recording exists");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "one line per invocation: {text:?}");

    let first: Vec<String> = serde_json::from_str(lines[0]).expect("line 0 is JSON");
    assert_eq!(first, ["check", "--format", "json"]);
    let second: Vec<String> = serde_json::from_str(lines[1]).expect("line 1 is JSON");
    assert_eq!(
        second[1], "Ship it",
        "an argument holding a space survived as one argument"
    );
}

/// The merged integration harness runs this beside every other CLI test. A
/// recording override must affect only its calling thread and must disappear at
/// scope exit, or an unrelated test can corrupt the invocation corpus.
#[test]
fn a_recording_override_is_thread_local_and_scoped() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    let log = dir.path().join("invocations.jsonl");

    {
        let _recording = common::record_into(log.clone());
        common::run(dir.path(), &["summary"]);
        let other = dir.path().to_owned();
        std::thread::spawn(move || common::run(&other, &["ready"]))
            .join()
            .expect("other test thread completed");
    }
    common::run(dir.path(), &["check"]);

    let lines: Vec<Vec<String>> = std::fs::read_to_string(&log)
        .expect("scoped invocation log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("recording line is JSON"))
        .collect();
    assert_eq!(lines, [vec!["summary"]]);
}

/// **Silent unless asked.** With no variable set, a run writes nothing at all,
/// so an ordinary `cargo test` behaves exactly as it did before this existed.
#[test]
fn a_run_records_nothing_when_no_file_is_named() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");

    // Both branches are asserted, because CI now runs this suite *with* the
    // variable set: `A1-208`'s surface audit resolves against the recording, so
    // `ci/scripts/test.sh` names a file. The first version skipped when it was
    // set and asserted that CI never set it, which those two tasks between them
    // made false -- and CI said so on the commit that made it false.
    let named = std::env::var_os(common::RECORD_TO).map(std::path::PathBuf::from);
    let before = entries(dir.path());
    let out = common::run(dir.path(), &["check"]);
    assert!(out.code == 0 || out.code == 1, "the binary ran: {out:?}");
    assert_eq!(
        entries(dir.path()),
        before,
        "a run wrote into the plan it was pointed at"
    );

    match named {
        // The property this test is named for: nothing asked, nothing written.
        // What it can observe is that the plan gained no file; that the *path* a
        // recording would take stays absent is asserted end to end by
        // `spawn_recording.rs`, which sets the destination itself.
        None => {}
        // And when a file *is* named, the recording goes there rather than
        // beside the plan.
        Some(log) => assert!(
            log.exists(),
            "{} named {} and the run recorded nowhere",
            common::RECORD_TO,
            log.display()
        ),
    }
}

fn entries(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("reading the fixture")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}
