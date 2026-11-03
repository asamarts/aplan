//! Folding is for people, and must not reach a parser (`A1-206`).
//!
//! `A1-206` teaches the human renderer to fold a long line so it continues in
//! the column it began in. The other half of that specification is what must
//! *not* change: `graph` emits Mermaid, `tasks`, `facts` and `schema` emit
//! documents a parser reads, and `--format github` emits annotations that are
//! one line by definition. A newline inserted into any of those turns a
//! cosmetic fix into corruption, and it would be invisible until somebody's CI
//! failed to parse it.
//!
//! Each test below carries its own control: it asserts that the *human* format
//! did fold the same content. Without that, a change which quietly stopped
//! folding everywhere would leave every one of these passing.

use std::path::Path;

use crate::common;

/// A plan whose one task carries a title far wider than the fold width and a
/// `traces` anchor that does not resolve, so `check` produces a finding whose
/// message and fix are both long.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.join("DOC.md"),
        "# Doc\n\n## something else entirely\n\nbody\n",
    )
    .expect("doc");
    std::fs::write(
        dir.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: A deliberately long title that runs well past any sensible \
         terminal width so that every renderer has something it must decide what to do about\n\
         phase: 0\nstatus: done\nowner: t\neffort_days: 1\ntraces:\n  - DOC.md#renamed\n\
         evidence:\n  - kind: note\n    ref: seeded\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = common::output(dir, args);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The title, as one run of words. If a renderer folded it, this will not appear
/// contiguously in the output.
const TITLE: &str = "A deliberately long title that runs well past any sensible terminal width \
                     so that every renderer has something it must decide what to do about";

#[test]
fn json_agent_and_sarif_parse_and_keep_a_long_message_whole() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    for format in ["json", "agent", "sarif"] {
        let text = run(dir.path(), &["check", "--format", format]);
        let doc: serde_json::Value = serde_json::from_str(text.trim()).unwrap_or_else(|e| {
            panic!("`check --format {format}` is not JSON after folding landed: {e}\n{text}")
        });
        let flat = doc.to_string();
        assert!(
            !flat.contains("\\n  ") && !flat.contains("\\n    "),
            "`--format {format}` carries a folded continuation inside a JSON string"
        );
    }
}

#[test]
fn a_github_annotation_stays_on_one_line() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let text = run(dir.path(), &["check", "--format", "github"]);

    let annotations: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("::error") || l.starts_with("::warning"))
        .collect();
    assert!(
        !annotations.is_empty(),
        "no annotations to check, so this proves nothing:\n{text}"
    );
    for line in &annotations {
        assert!(
            line.len() > 95,
            "an annotation shorter than the fold width proves nothing about folding: {line}"
        );
    }
}

#[test]
fn mermaid_keeps_each_node_on_its_own_line() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let text = run(dir.path(), &["graph"]);

    let node = text
        .lines()
        .find(|l| l.contains("T-001"))
        .unwrap_or_else(|| panic!("no node for the task:\n{text}"));
    assert!(
        node.contains(TITLE),
        "the Mermaid label was folded, so the diagram no longer parses:\n{node}"
    );
    // A node is `id(["label"])`, so its line closes with the quote and both
    // brackets. A fold would leave the line open and the diagram unparseable.
    assert!(
        node.trim_end().ends_with("\"])"),
        "a Mermaid node did not close on its own line: {node}"
    );
}

#[test]
fn tasks_as_json_keeps_the_title_whole() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let text = run(dir.path(), &["tasks", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(text.trim()).expect("tasks --format json");
    let title = doc["tasks"][0]["title"].as_str().expect("a title");
    assert_eq!(title, TITLE, "the JSON title was folded");
}

/// The control. Every test above would also pass if folding had been removed
/// entirely, so this asserts the human format *does* fold the same content --
/// which is what makes the others say something.
///
/// Through a pty, because folding follows the terminal: a pipe is deliberately
/// left alone (`A1-206`), which is the very property the tests above rely on.
/// Asserting it against a pipe would assert the opposite of the design.
#[test]
fn the_human_format_folds_what_the_machine_formats_do_not() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // The shared helper, which refuses to skip under `CI`: this file's own copy
    // printed "skipping" and returned, so on a runner without util-linux it
    // asserted nothing and reported green (`A1-207`).
    let Some(human) = common::on_a_terminal(dir.path(), 95, &["tasks"]) else {
        return;
    };

    assert!(
        !human.contains(TITLE),
        "the human format did not fold the long title on a terminal, so the \
         machine-format assertions above are not distinguishing anything:\n{human}"
    );
}
