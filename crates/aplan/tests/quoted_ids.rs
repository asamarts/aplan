//! A plan `check` calls clean can be edited by every mutating command (`A1-250`).
//!
//! The loader accepts an id like `"  spaced"`, and the writer used to emit list
//! entries as plain YAML scalars, so writing that id into another task's
//! `blocked_by` produced frontmatter that read back as `spaced`. The round-trip
//! guard caught it and reported exit `3` -- *this is a bug in aplan, please
//! report it* -- for an id the caller had every right to type. The loader and
//! the writer disagreed about what an id may be, and nothing between them said
//! so.
//!
//! Settled from the writer's side: the scalar is serialised rather than pasted,
//! so anything the loader accepts can be written back. These tests are the two
//! halves of that promise -- the awkward value is *written* when it names a real
//! task, and *refused as the plan's problem* when it does not.

use std::path::Path;

use crate::common;
use common::aplan;

/// An id that only survives being written if the writer quotes it.
const SPACED: &str = "  spaced";

fn write_task(root: &Path, file: &str, id: &str, extra: &str) {
    std::fs::write(
        root.join(format!("aplan/tasks/{file}.md")),
        format!(
            "---\nid: \"{id}\"\ntitle: a task\nphase: 1\nstatus: todo\nowner: kaminsod\n\
             effort_days: 1\n{extra}---\n\n## Why\n\nSomething.\n"
        ),
    )
    .expect("task");
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::write(root.join(".aplan.yml"), "").expect("config");
    write_task(root, "spaced", SPACED, "");
    write_task(root, "A-1", "A-1", "");
    write_task(root, "A-2", "A-2", "");
    dir
}

/// **Nothing a mutating command does to it blames `aplan`.**
///
/// Exit `3` says the run proved nothing and asks the reader to file a bug. A
/// task the loader accepted is not that, whatever its id is spelled with.
#[test]
fn every_mutating_command_can_edit_a_task_whose_id_needs_quoting() {
    let dir = repo();
    let root = dir.path();
    for args in [
        vec!["check"],
        vec!["dep", "add", "A-1", "--after", SPACED],
        vec!["tag", "add", SPACED, "cli"],
        vec!["tag", "rm", SPACED, "cli"],
        vec!["tag", "rename", "cli", "backend"],
        vec!["mv", SPACED, "--phase", "2"],
        vec!["dep", "rm", "A-1", "--after", SPACED],
        vec!["drop", SPACED],
        vec!["check"],
    ] {
        let (text, code) = aplan(root, &args);
        assert_ne!(code, 3, "`{args:?}` reported aplan's own defect: {text}");
        assert!(
            !text.contains("bug in aplan"),
            "`{args:?}` told the reader to file a bug: {text}"
        );
    }
}

/// **And the edge really round-trips**, which is the thing the exit code was
/// standing in for: the id goes into another task's `blocked_by` and comes back
/// the same id, spaces and all.
#[test]
fn an_id_that_needs_quoting_survives_being_written_into_another_task() {
    let dir = repo();
    let root = dir.path();
    let (text, code) = aplan(root, &["dep", "add", "A-1", "--after", SPACED]);
    assert_eq!(code, 0, "{text}");

    // Read back through the tool rather than off the disk: what matters is that
    // the plan `aplan` loads holds the id that was written, not that some
    // particular bytes are in the file.
    let (shown, code) = aplan(root, &["show", "A-1", "--format", "json"]);
    assert_eq!(code, 0, "{shown}");
    // `aplan` prints a note about the missing git repository first, and `show`
    // prints its body after the object, so this reads the first JSON value out
    // of the stream rather than assuming the whole capture is one.
    let from = shown.find('{').expect("an object in {shown}");
    let json: serde_json::Value = serde_json::Deserializer::from_str(&shown[from..])
        .into_iter::<serde_json::Value>()
        .next()
        .expect("show prints an object")
        .expect("show is json");
    let waits: Vec<&str> = json["waits_on"]
        .as_array()
        .expect("waits_on")
        .iter()
        .filter_map(|w| w["id"].as_str())
        .collect();
    assert_eq!(waits, vec![SPACED], "the id came back changed: {shown}");

    // And the plan still loads, which is what the round-trip guard protects.
    let (text, code) = aplan(root, &["check"]);
    assert_ne!(code, 3, "{text}");
}

/// **An id the plan does not know is the plan's problem, not `aplan`'s.**
///
/// This is `A1-250`'s table. Every row used to reach exit `3` except the first;
/// the first row is the answer they should all get.
#[test]
fn an_unknown_id_is_a_finding_whatever_it_is_spelled_with() {
    let dir = repo();
    let root = dir.path();
    for awkward in [
        "nosuch",
        "  unknown",
        "trail  ",
        "has: colon",
        "hash #x",
        "*star",
        "&anchor",
        "!bang",
        "quote\"x",
        "dash-x",
        "new\nline",
    ] {
        let (text, code) = aplan(root, &["dep", "add", "A-1", "--after", awkward]);
        assert_eq!(code, 2, "`{awkward}` did not refuse cleanly: {text}");
        assert!(
            text.contains("APLAN003"),
            "`{awkward}` refused without naming the unknown id: {text}"
        );
        assert!(
            !text.contains("bug in aplan"),
            "`{awkward}` was reported as aplan's defect: {text}"
        );
    }
}
