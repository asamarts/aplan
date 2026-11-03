//! `aplan export-roadmap-json`, the contract generated rather than kept by hand
//! (`A1-033`).
//!
//! The first pilot's repository cannot drop its `roadmap.json`: a web build and
//! a spec linter read it. So the end state is generating the half that drifts --
//! the statuses and the dependency graph -- while every other byte of meaning is
//! carried through from the file itself. What these pin is that the carrying
//! through is complete, that the statuses are the plan's verdict rather than the
//! source's claim, and that anything the file has no slot for is reported rather
//! than invented.

use std::path::Path;
use std::process::Command;

use crate::common;
use common::aplan;

const SOURCE: &str = r#"{
  "format_version": 1,
  "project": "example",
  "note": "Status projection only.",
  "phases": [
    {"id": "0", "title": "Foundations", "status": "done", "blurb": "Cited research."}
  ],
  "workstreams": [{
    "id": "RFC-0001",
    "title": "A workstream",
    "authority": "normative",
    "status_validity": {"state": "current", "valid_through": "2026-08-13T00:00:00Z"},
    "milestones": [
      {"id": "M-A", "title": "Containment", "status": "done", "depends_on": [],
       "evidence": ["docs/proof.md#result"]},
      {"id": "M-B", "title": "Restoration", "status": "not_started", "depends_on": ["M-A"]},
      {"id": "M-C", "title": "Withdrawn", "status": "not_applicable", "depends_on": []}
    ]
  }],
  "adrs": [{"id": "ADR-0001", "title": "a", "status": "accepted"}],
  "specs": [{"id": "SPEC-0001", "title": "s", "status": "draft"}]
}
"#;

/// The whole reason this is a merge: those fields are what another project's
/// build reads, and a file generated from the plan alone would validate here and
/// break there.
#[test]
fn every_field_this_model_does_not_hold_is_carried_through() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "{out}");

    let before: serde_json::Value = serde_json::from_str(SOURCE).expect("json");
    let after = read(dir.path());
    for key in ["format_version", "project", "note", "adrs", "specs"] {
        assert_eq!(after[key], before[key], "{key} was not carried through");
    }
    let ws = &after["workstreams"][0];
    assert_eq!(ws["authority"], "normative");
    assert_eq!(
        ws["status_validity"],
        before["workstreams"][0]["status_validity"]
    );
    assert_eq!(after["phases"][0]["blurb"], "Cited research.");
    assert_eq!(
        ws["milestones"][0]["evidence"], before["workstreams"][0]["milestones"][0]["evidence"],
        "the source's own citations stay where the source put them"
    );
}

/// The statuses are the plan's verdict in the source's vocabulary, which is what
/// stops the file being hand-maintained. A `done` the plan has not proved reads
/// `not_started`, deliberately: writing it back would launder the claim through
/// a file that is supposed to have stopped drifting.
#[test]
fn a_claim_the_plan_has_not_proved_is_not_written_back_as_done() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "{out}");

    let after = read(dir.path());
    assert_eq!(
        after["phases"][0]["status"], "not_started",
        "imported as a claim, not as proof"
    );
    assert_eq!(
        after["workstreams"][0]["milestones"][0]["status"],
        "not_started"
    );

    // Close it with the source's own proof -- which `A1-159` made citable -- and
    // the file says `done` again, with something behind it.
    let (out, code) = aplan(
        dir.path(),
        &["close", "M-A", "--evidence", "file:docs/proof.md#result"],
    );
    assert_eq!(code, 0, "{out}");
    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        read(dir.path())["workstreams"][0]["milestones"][0]["status"],
        "done"
    );
}

/// `not_applicable` is a requirement discharged rather than work abandoned
/// (`A1-160`), and the import kept that word as a tag precisely so the round
/// trip does not flatten it to `dropped`.
#[test]
fn a_discharged_requirement_goes_back_in_the_sources_own_word() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        read(dir.path())["workstreams"][0]["milestones"][2]["status"],
        "not_applicable"
    );
}

/// A task the plan has and the file has nowhere to put is reported. Where it
/// belongs in a foreign schema is not something this can know, and a generated
/// contract silently missing a milestone is the drift this command retires,
/// arriving from the other direction.
#[test]
fn a_task_the_document_has_no_slot_for_is_reported_not_invented() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/NEW-1.md"),
        "---\nid: NEW-1\ntitle: Added here\nphase: 1\nstatus: todo\neffort_days: 1\n\
         ---\n\nbody\n",
    )
    .expect("task");

    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "reported, not refused: {out}");
    assert!(out.contains("NEW-1"), "{out}");
    assert!(out.contains("not in this document"), "{out}");

    let after = read(dir.path());
    let ids: Vec<&str> = after["workstreams"][0]["milestones"]
        .as_array()
        .expect("milestones")
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(!ids.contains(&"NEW-1"), "nothing was invented: {ids:?}");
}

/// The gate shape `roadmap --format markdown` already has: `--check` compares
/// and never writes.
#[test]
fn check_compares_and_writes_nothing() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    // The committed file disagrees with the plan to begin with.
    let (out, code) = aplan(
        dir.path(),
        &["export-roadmap-json", "roadmap.json", "--check"],
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("disagree"), "{out}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("roadmap.json")).expect("read"),
        SOURCE,
        "`--check` must not write"
    );

    // Generated, it agrees, and stays byte-identical on a second run.
    let (_, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0);
    let first = std::fs::read_to_string(dir.path().join("roadmap.json")).expect("read");
    let (out, code) = aplan(
        dir.path(),
        &["export-roadmap-json", "roadmap.json", "--check"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("up to date"), "{out}");

    let (_, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("roadmap.json")).expect("read"),
        first,
        "byte-stable across runs, which is what `--check` needs"
    );
}

/// A dependency edge goes back in the source's own spelling, not in the ids this
/// importer gave its phases.
#[test]
fn a_dependency_goes_back_in_the_sources_own_ids() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        read(dir.path())["workstreams"][0]["milestones"][1]["depends_on"][0],
        "M-A"
    );
}

/// An edge goes back in the source's spelling wherever it appears, not only on
/// the half that happened to be passed the map. A milestone that comes to depend
/// on a phase wrote `PHASE-00` into a document that spells it `0` -- an id that
/// repository's own linter reports as a missing dependency (`A1-033`'s audit).
#[test]
fn a_renamed_id_goes_back_in_the_sources_spelling_on_both_halves() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(dir.path(), &["dep", "add", "M-B", "--after", "PHASE-00"]);
    assert_eq!(code, 0, "{out}");
    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "{out}");

    let after = read(dir.path());
    let deps = after["workstreams"][0]["milestones"][1]["depends_on"]
        .as_array()
        .expect("depends_on")
        .iter()
        .filter_map(|d| d.as_str())
        .collect::<Vec<_>>();
    assert!(
        deps.contains(&"0"),
        "the source spells that phase `0`: {deps:?}"
    );
    assert!(
        !deps.contains(&"PHASE-00"),
        "and not by the id this importer gave it: {deps:?}"
    );
}

/// The other half of the same map: a phase waiting on the workstream goes back
/// as the workstream, not as the cut task this importer names it.
///
/// Its own fixture, because a cut lists its members as blockers: adding this
/// edge to a plan where a milestone already waits on the phase is a cycle, and
/// `dep add` is right to refuse it.
#[test]
fn an_edge_to_the_workstream_goes_back_as_the_workstream() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let (out, code) = aplan(
        dir.path(),
        &["dep", "add", "PHASE-00", "--after", "RFC-0001-CUT"],
    );
    assert_eq!(code, 0, "{out}");
    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(read(dir.path())["phases"][0]["depends_on"][0], "RFC-0001");
}

/// An entry the document declares and the plan does not is a status nothing
/// backs, left standing in a generated file -- the drift this command retires,
/// surviving in the one place nobody looks.
#[test]
fn an_entry_with_no_task_in_the_plan_is_reported() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::remove_file(dir.path().join("aplan/tasks/M-C.md")).expect("rm");

    let (out, code) = aplan(dir.path(), &["export-roadmap-json", "roadmap.json"]);
    assert_eq!(code, 0, "reported, not refused: {out}");
    assert!(out.contains("M-C"), "{out}");
    assert!(out.contains("the plan does not"), "{out}");

    // Left exactly as it was, because inventing a status for it would be worse.
    assert_eq!(
        read(dir.path())["workstreams"][0]["milestones"][2]["status"],
        "not_applicable"
    );
}

fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("docs")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(dir.join("roadmap.json"), SOURCE).expect("source");
    std::fs::write(
        dir.join("docs/proof.md"),
        "# Proof\n\n## Result\n\nIt held.\n",
    )
    .expect("doc");
    // `file` evidence resolves against git, so the proof has to be committed.
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "Test"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "the proof"],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(dir)
            .output()
            .expect("git");
    }
    let (out, code) = aplan(dir, &["import", "roadmap.json", "--format", "roadmap-json"]);
    assert_eq!(code, 0, "{out}");
}

fn read(dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("roadmap.json")).expect("read");
    serde_json::from_str(&text).expect("json")
}
