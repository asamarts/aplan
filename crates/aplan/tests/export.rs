//! `aplan export`, the other half of `import` (`A1-189`).
//!
//! Five shapes could be read and one could be written, and the one that could
//! was a bespoke command for a single foreign file. What that costs an adopter
//! is specific: the day the plan becomes the source of truth, the document they
//! imported from starts going stale, and it is usually the document their
//! colleagues still read.
//!
//! What these pin is the contract that makes a generated document trustworthy:
//! the round trip closes, what the shape could not carry is *said* rather than
//! quietly left out (ADR-0012 read in the outgoing direction), the report never
//! contaminates the document itself, and a shape that cannot be written honestly
//! is refused by name with the field that stopped it.

use std::path::Path;

mod common;
use common::{aplan, run};

/// A plan with a cut, a task outside it, and one closed task, which is the
/// smallest shape that exercises every part of the renderer.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    write_task(
        dir,
        "M-core",
        "---\nid: M-core\ntitle: Core validator\nphase: 1\nstatus: todo\ntags:\n- milestone\n\
         blocked_by:\n- T-001\n- T-002\n---\n\n## Why\n\nThe first cut.\n",
    );
    write_task(
        dir,
        "T-001",
        "---\nid: T-001\ntitle: Build the core\nphase: 1\nstatus: done\neffort_days: 2.0\n\
         owner: alice\nevidence:\n- kind: note\n  ref: it was built\n  captured: '2026-01-01'\n\
         ---\n\n## Why\n\nBecause.\n",
    );
    write_task(
        dir,
        "T-002",
        "---\nid: T-002\ntitle: Build the renderer\nphase: 1\nstatus: todo\neffort_days: 3.0\n\
         ---\n\n## Why\n\nBecause.\n",
    );
    write_task(
        dir,
        "T-003",
        "---\nid: T-003\ntitle: Write the docs\nphase: 1\nstatus: todo\neffort_days: 1.0\n\
         ---\n\n## Why\n\nBecause.\n",
    );
}

fn write_task(dir: &Path, id: &str, text: &str) {
    std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), text).expect("task");
}

/// **The round trip, asserted the way `A1-033`'s is: the regenerated file is one
/// its consumers accept.**
///
/// A markdown checklist has two consumers here, and both are asked. The reader
/// beside this writer parses it back into tasks, and `check` passes on the plan
/// those tasks make -- so the document is not merely re-readable, it round-trips
/// into a plan this tool will validate.
#[test]
fn the_regenerated_file_is_one_its_consumers_accept() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(
        dir.path(),
        &["export", "--format", "checklist", "--out", "ROADMAP.md"],
    );
    assert_eq!(code, 0, "{out}");
    let document = std::fs::read_to_string(dir.path().join("ROADMAP.md")).expect("the document");
    assert_eq!(
        document,
        "- [ ] Write the docs\n\n## Core validator\n\n- [x] Build the core\n\
         - [ ] Build the renderer\n",
        "the cut is a heading, the closed task is ticked, and the unfiled task is above both"
    );

    // The first consumer: this tool's own reader for the shape.
    std::fs::create_dir_all(dir.path().join("back/tasks")).expect("mkdir");
    let (out, code) = aplan(
        dir.path(),
        &[
            "import",
            "ROADMAP.md",
            "--format",
            "checklist",
            "--root",
            "back",
            "--id-prefix",
            "R",
            "--implemented-as",
            "done",
            "--as-of",
            "2026-01-01",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("imported 3 task(s)"), "{out}");
    assert!(
        out.contains("R-002  done"),
        "the ticked box came back: {out}"
    );
    assert!(
        out.contains("core-validator"),
        "the heading came back as the section tag: {out}"
    );

    // The second: `check`, on the plan the re-import produced. Exit `0` is "no
    // errors", and that is the claim -- the round trip produces a structurally
    // valid plan. Those tasks do carry `APLAN009` warnings, because a checklist
    // holds no estimate and the export said so on the way out.
    let (out, code) = aplan(dir.path(), &["check", "--root", "back"]);
    assert_eq!(
        code, 0,
        "the regenerated file makes a plan `check` accepts: {out}"
    );
    assert!(out.contains("3 tasks"), "{out}");
}

/// The document on standard output, the report on standard error.
///
/// Not a style choice: with no `--out` the reader is redirecting the document
/// into a file, and a report mixed into it would corrupt exactly the artefact
/// this command exists to produce.
#[test]
fn the_report_never_lands_in_the_document() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let out = run(dir.path(), &["export", "--format", "checklist"]);
    assert_eq!(out.code, 0, "{}", out.text());

    let rows = out.stdout.lines().filter(|l| l.starts_with("- [")).count();
    assert_eq!(
        rows, 3,
        "standard output is not the document: {:?}",
        out.stdout
    );
    for line in out.stdout.lines().filter(|l| !l.is_empty()) {
        assert!(
            line.starts_with("- [") || line.starts_with("## "),
            "standard output carries something that is not the document: {line}"
        );
    }
    assert!(out.stderr.contains("report:"), "{}", out.stderr);
    assert!(
        out.stderr.contains("`evidence` has no place in this shape"),
        "the closed task's proof went out unmentioned: {}",
        out.stderr
    );
}

/// ADR-0012 clause 3, read outwards: what the shape could not carry is named,
/// with a count, rather than left for a reader to notice from a diff.
#[test]
fn what_the_shape_could_not_carry_is_named_with_a_count() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let out = run(dir.path(), &["export", "--format", "checklist"]);
    for field in ["id", "evidence", "owner", "effort_days"] {
        assert!(
            out.stderr
                .contains(&format!("`{field}` has no place in this shape")),
            "`{field}` left the plan without a word about it:\n{}",
            out.stderr
        );
    }
    assert!(
        out.stderr.contains("1 task(s) carry it"),
        "a loss with no count is a loss nobody can judge: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("in no cut"),
        "where the unfiled task landed is not said: {}",
        out.stderr
    );
}

/// A shape that cannot be written honestly is refused, and the refusal names
/// the one field a generated document would have had to invent.
///
/// Walked from the flavour table rather than from a list typed here, so a shape
/// added as read-only cannot ship with "unsupported" as its whole answer.
#[test]
fn a_shape_that_cannot_be_written_is_refused_with_the_field_that_stopped_it() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let mut refused = 0;
    for flavor in aplan_interop::FLAVORS {
        let aplan_interop::Direction::ReadOnly { field, instead, .. } = flavor.direction else {
            continue;
        };
        refused += 1;
        // With `--out`, so the assertion below that nothing was written is about
        // a command that was asked to write rather than one that never could be.
        let out = run(
            dir.path(),
            &["export", "--format", flavor.name, "--out", "ROADMAP.md"],
        );
        assert_ne!(
            out.code,
            0,
            "`{}` was written anyway: {}",
            flavor.name,
            out.text()
        );
        let text = out.text();
        assert!(
            text.contains(&format!("`{field}`")),
            "`{}` refuses without naming the field: {text}",
            flavor.name
        );
        // The refusal has to leave the reader somewhere to go.
        let first_words: String = instead
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains(&first_words),
            "`{}` refuses without saying what to do instead: {text}",
            flavor.name
        );
        assert!(
            std::fs::read_dir(dir.path())
                .expect("dir")
                .flatten()
                .all(|e| e.file_name() != "ROADMAP.md"),
            "a refusal wrote a file"
        );
    }
    assert!(refused >= 4, "only {refused} shape(s) were exercised");
}

/// The two halves cannot disagree about which shapes exist, because they read
/// one table.
#[test]
fn the_reader_and_the_writer_know_the_same_shapes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let reading = run(dir.path(), &["import", "ROADMAP.md", "--format", "nope"]).text();
    for flavor in aplan_interop::FLAVORS {
        assert!(
            reading.contains(flavor.name),
            "`{}` is a flavour the reader does not list: {reading}",
            flavor.name
        );
    }
    let writing = run(dir.path(), &["export", "--format", "nope"]).text();
    assert!(writing.contains("checklist"), "{writing}");
    assert!(
        !writing.contains("kep"),
        "a shape this does not write must not be offered as one that would have worked: {writing}"
    );
}

/// `--check` is the gate shape the other generated artefacts already have: it
/// never writes, exits 0 while the file agrees with the plan, and 1 when it
/// stops agreeing, naming the command that would fix it.
#[test]
fn check_is_a_gate_that_never_writes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (out, code) = aplan(
        dir.path(),
        &["export", "--format", "checklist", "--out", "ROADMAP.md"],
    );
    assert_eq!(code, 0, "{out}");

    let out = run(
        dir.path(),
        &["export", "--format", "checklist", "--check", "ROADMAP.md"],
    );
    assert_eq!(out.code, 0, "{}", out.text());
    assert!(out.stderr.contains("is up to date"), "{}", out.text());

    // Close a task, and the document is now behind the plan.
    let path = dir.path().join("aplan/tasks/T-002.md");
    let text = std::fs::read_to_string(&path).expect("task");
    std::fs::write(
        &path,
        text.replace(
            "status: todo",
            "status: done\nevidence:\n- kind: note\n  ref: shipped\n  captured: '2026-01-02'",
        ),
    )
    .expect("write");

    let before = std::fs::read_to_string(dir.path().join("ROADMAP.md")).expect("document");
    let out = run(
        dir.path(),
        &["export", "--format", "checklist", "--check", "ROADMAP.md"],
    );
    assert_eq!(out.code, 1, "{}", out.text());
    assert!(
        out.stderr
            .contains("aplan export --format checklist --out ROADMAP.md"),
        "the gate does not say how to fix it: {}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("ROADMAP.md")).expect("document"),
        before,
        "`--check` wrote"
    );
}

/// A plan with no work is refused rather than written as an empty file, and the
/// refusal says what the plan does hold.
#[test]
fn a_plan_with_nothing_to_write_is_refused_and_says_what_it_has() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    write_task(
        dir.path(),
        "T-001",
        "---\nid: T-001\ntitle: Abandoned\nphase: 1\nstatus: dropped\n---\n\n## Why\n\nNo.\n",
    );
    let out = run(
        dir.path(),
        &["export", "--format", "checklist", "--out", "ROADMAP.md"],
    );
    assert_ne!(out.code, 0, "{}", out.text());
    assert!(out.text().contains("1 dropped"), "{}", out.text());
    assert!(
        !dir.path().join("ROADMAP.md").exists(),
        "a refusal left a file behind"
    );
}

/// **The two ways of writing the document produce the same bytes.**
///
/// `aplan export > F` goes through the stream writer and `--out F` goes to the
/// file directly, and the stream writer strips a control character *and its
/// parameters* while a file keeps them. Written unfiltered, one plan gave two
/// different documents, and the one a reader would have committed carried a live
/// `CSI 2 J` (`A1-189`, audit).
#[test]
fn the_two_ways_of_writing_the_document_produce_the_same_bytes() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    hostile_task(dir.path());

    let piped = run(dir.path(), &["export", "--format", "checklist"]).stdout;
    let (out, code) = aplan(
        dir.path(),
        &["export", "--format", "checklist", "--out", "ROADMAP.md"],
    );
    assert_eq!(code, 0, "{out}");
    let written = std::fs::read_to_string(dir.path().join("ROADMAP.md")).expect("the document");
    assert_eq!(piped, written, "the two write paths disagree");

    // And neither carries what the plan tried to smuggle.
    for line in written.lines() {
        assert!(
            !line.chars().any(char::is_control),
            "a control character reached a committed row: {line:?}"
        );
    }
    assert!(
        written.contains("[2J"),
        "the text was deleted rather than made inert: {written:?}"
    );
}

/// A task whose own text tries to steer whoever opens the generated file.
///
/// The YAML *escape*, not a raw control character: a raw one is refused by the
/// parser, which is why this is reachable at all.
fn hostile_task(dir: &Path) {
    write_task(
        dir,
        "T-004",
        "---\nid: T-004\ntitle: \"erase \\e[2J the screen\"\nphase: 1\nstatus: todo\n\
         effort_days: 1.0\n---\n\n## Why\n\nBecause.\n",
    );
}

/// **Standard output is the document and nothing else, in every mode.**
///
/// The property that lets a reader write `aplan export > ROADMAP.md` without
/// reading the flags first. `--out` and `--check` produce no document, so they
/// produce no standard output at all; the report, the confirmation and the
/// gate's verdict are all on standard error.
#[test]
fn standard_output_is_the_document_and_nothing_else() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let bare = run(dir.path(), &["export", "--format", "checklist"]);
    assert!(
        bare.stdout.starts_with("- [ ] Write the docs"),
        "{:?}",
        bare.stdout
    );

    let wrote = run(
        dir.path(),
        &["export", "--format", "checklist", "--out", "ROADMAP.md"],
    );
    assert_eq!(
        wrote.stdout, "",
        "`--out` printed to standard output: {:?}",
        wrote.stdout
    );
    assert!(
        wrote.stderr.contains("wrote ROADMAP.md"),
        "{}",
        wrote.stderr
    );

    let gate = run(
        dir.path(),
        &["export", "--format", "checklist", "--check", "ROADMAP.md"],
    );
    assert_eq!(gate.code, 0, "{}", gate.text());
    assert_eq!(
        gate.stdout, "",
        "`--check` printed to standard output: {:?}",
        gate.stdout
    );

    // And the refused shapes say nothing there either.
    let refused = run(dir.path(), &["export", "--format", "kep"]);
    assert_eq!(
        refused.stdout, "",
        "a refusal printed a document: {:?}",
        refused.stdout
    );
}

/// **The exhaustive half of the loss census, walked from the generated schema.**
///
/// The unit test beside the renderer walks a *populated* task's serialised keys,
/// and a field with `skip_serializing_if` that the fixture forgets to set is
/// absent from that walk: the census would go on passing while the new field
/// left the plan in silence. `schemas/v1/task.json` is derived from the Rust
/// types and drift-gated, so every field the model declares is in it whether an
/// instance carries one or not.
#[test]
fn every_field_the_schema_declares_is_carried_or_named() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("schemas/v1/task.json")).expect("schema"),
    )
    .expect("json");
    let fields: Vec<String> = schema["properties"]
        .as_object()
        .expect("the task schema declares properties")
        .keys()
        .cloned()
        .collect();
    assert!(
        fields.len() >= 14,
        "only {} field(s) were walked, so this is not reading the model: {fields:?}",
        fields.len()
    );

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // A checkpoint that contains another, so `contains` is exercised too. It has
    // to be a milestone: `APLAN047` refuses containment declared by work, and a
    // fixture that would not pass `check` is a poor witness for what `export`
    // says about a real plan.
    write_task(
        dir.path(),
        "M-OUT",
        "---\nid: M-OUT\ntitle: outer\nphase: 1\nstatus: todo\n\
         tags:\n- milestone\ncontains:\n- M-IN\nblocked_by:\n- M-IN\n\
         ---\n\n## Why\n\nBecause.\n",
    );
    write_task(
        dir.path(),
        "M-IN",
        "---\nid: M-IN\ntitle: inner\nphase: 1\nstatus: todo\n\
         tags:\n- milestone\nblocked_by:\n- T-003\n\
         ---\n\n## Why\n\nBecause.\n",
    );
    // A second phase and a second live status, so the two fields that are only
    // reported when a plan exercises them are exercised.
    write_task(
        dir.path(),
        "T-005",
        "---\nid: T-005\ntitle: A second phase\nphase: 2\nstatus: in_progress\n\
         effort_days: 1.0\nowner: bo\npaths:\n- src/**\ntraces:\n- docs/x.md#y\n\
         tags:\n- cross-cutting\nprefer_after:\n- T-003\nblocked_by:\n- T-003\n\
         revisit_at: '2026-12-01T00:00:00Z'\ntrace_fingerprints:\n  docs/x.md#y: abc\n\
         version_impact: fix\nversion_impact_reason: a note, not a release\n\
         ratified_by:\n- T-003\n\
         ---\n\n## Why\n\nBecause.\n",
    );
    // A closed release carrying the number it produced, so the field that only a
    // release may hold is exercised too (`A1-339`).
    write_task(
        dir.path(),
        "M-shipped",
        "---\nid: M-shipped\ntitle: A release that shipped\nphase: 2\nstatus: done\n\
         tags:\n- milestone\n- release\nversion: 0.9.0\n\
         evidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-09-24\n\
         ---\n\n## Why\n\nBecause.\n",
    );

    let out = run(dir.path(), &["export", "--format", "checklist"]);
    assert_eq!(out.code, 0, "{}", out.text());
    for field in &fields {
        // The one field the shape carries: it is the row's text.
        if field == "title" {
            assert!(out.stdout.contains("A second phase"), "{:?}", out.stdout);
            continue;
        }
        assert!(
            out.stderr.contains(&format!("`{field}`")),
            "`{field}` left the plan without a word about it:\n{}",
            out.stderr
        );
    }
}
