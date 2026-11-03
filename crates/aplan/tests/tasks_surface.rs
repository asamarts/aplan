//! `aplan tasks` carries what a task *is*, not what a rollup needed (`A1-220`).
//!
//! `A1-075` made this the data surface and said any rollup the tool does not
//! have is a `jq` line downstream. That was true of thirteen fields, and the
//! claim was quoted for more: `A1-186` and `A1-214` rest on it, and so does
//! `ADR-0026`, which refuses a query language on the grounds that every
//! predicate over a row is answerable here.
//!
//! It was not. A consumer could ask which tasks are ready in a cut and could not
//! ask **which `done` tasks cite only `note` evidence** -- the question closest
//! to this tool's own thesis, and the first one an auditor asks.

use std::path::{Path, PathBuf};

use crate::common;

/// One task done on a note alone, one done with a commit beside a note, and one
/// carrying the other three declared fields.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    let files = [
        (
            "W-1",
            "---\nid: W-1\ntitle: done on a note alone\nphase: 0\nstatus: done\nowner: t\n\
             effort_days: 1\nevidence:\n  - kind: note\n    ref: I finished it\n    \
             captured: 2026-01-01\n---\n\nbody\n",
        ),
        (
            "W-2",
            "---\nid: W-2\ntitle: done with a commit\nphase: 0\nstatus: done\nowner: t\n\
             effort_days: 1\npaths: [src/**]\nevidence:\n  - kind: commit\n    ref: abc1234\n  \
             - kind: note\n    ref: context\n    captured: 2026-01-01\n---\n\nbody\n",
        ),
        (
            "W-3",
            "---\nid: W-3\ntitle: traces and prefers\nphase: 0\nstatus: todo\nowner: t\n\
             effort_days: 1\ntraces: [DOC.md#intro]\nprefer_after: [W-1]\npaths: [docs/**]\n\
             ---\n\nbody\n",
        ),
    ];
    for (id, body) in files {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }
}

fn rows(dir: &Path) -> Vec<serde_json::Value> {
    let out = common::output(dir, &["tasks", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("tasks --format json");
    doc["tasks"].as_array().expect("tasks").clone()
}

fn workspace_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .join(path)
}

/// The question the task is named for, answered from this surface alone.
#[test]
fn a_consumer_can_ask_which_done_tasks_cite_only_a_note() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let rows = rows(dir.path());
    let note_only: Vec<&str> = rows
        .iter()
        .filter(|t| t["status"] == "done")
        .filter(|t| {
            let ev = t["evidence"].as_array().expect("evidence is a list");
            !ev.is_empty() && ev.iter().all(|e| e["kind"] == "note")
        })
        .filter_map(|t| t["id"].as_str())
        .collect();

    assert_eq!(
        note_only,
        ["W-1"],
        "the surface cannot distinguish a note-only close from a commit-backed one"
    );
}

/// The declared half of a task is reachable, not just the derived half.
#[test]
fn paths_traces_and_prefer_after_are_reachable() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let rows = rows(dir.path());
    let row = |id: &str| {
        rows.iter()
            .find(|t| t["id"] == id)
            .unwrap_or_else(|| panic!("no row for {id}"))
    };

    assert_eq!(row("W-3")["paths"], serde_json::json!(["docs/**"]));
    assert_eq!(row("W-3")["traces"], serde_json::json!(["DOC.md#intro"]));
    assert_eq!(row("W-3")["prefer_after"], serde_json::json!(["W-1"]));
    // And the fields are present-but-empty rather than absent, so a consumer
    // reads a list either way.
    for field in ["evidence", "paths", "traces", "prefer_after"] {
        assert!(
            row("W-1")[field].is_array(),
            "`{field}` is missing from a row that has none of it"
        );
    }
    assert!(row("W-1")["trace_fingerprints"].is_object());
    assert!(row("W-1")["evidence_fingerprints"].is_object());
}

/// Everything a task declares reaches the surface, checked against the schema
/// the loader validates against rather than a list written here.
///
/// The old thirteen were the fields a *rollup* needed, and nobody decided the
/// rest were out. This is what stops that happening again: a field added to
/// `Task` and not to the row fails here.
#[test]
fn every_declared_field_reaches_the_surface() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let schema = common::output(dir.path(), &["schema"]);
    let schema: serde_json::Value = serde_json::from_slice(&schema.stdout).expect("schema is JSON");
    let declared: Vec<&str> = schema["properties"]
        .as_object()
        .expect("the task schema has properties")
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        declared.len() >= 13,
        "only {} declared fields",
        declared.len()
    );

    let row = &rows(dir.path())[0];
    let missing: Vec<&&str> = declared
        .iter()
        .filter(|field| row.get(**field).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "a task declares {missing:?} and `aplan tasks` does not carry them, so the surface \
         `ADR-0026` refuses a query language in favour of cannot answer about them"
    );
}

/// The premise `ADR-0026` rests on, stated as a test.
///
/// The ADR refuses a query language because "every predicate over a row is a
/// `jq` line once `aplan tasks` carries the field". That was written before the
/// field was carried, which made the refusal true by intention rather than by
/// fact.
#[test]
fn the_adr_that_refuses_a_query_language_has_its_premise() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the workspace root")
        .join("docs/adr/0026-reachability-not-a-query-language.md");
    let adr = std::fs::read_to_string(&root).expect("ADR-0026 exists");
    assert!(
        adr.contains("`jq` line once"),
        "the ADR no longer rests on this premise; this test is checking the wrong thing"
    );

    // The fields the ADR names as answerable downstream.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    let row = &rows(dir.path())[0];
    for named in ["status", "owner", "tags", "cut", "effort_days"] {
        assert!(
            row.get(named).is_some(),
            "the ADR promises `{named}` downstream"
        );
    }
    assert!(
        row.get("evidence").is_some(),
        "the ADR promises evidence kind downstream, which is what it means by \
         `evidence kind` in its list"
    );
}

/// Every citation has the same shape, whatever kind it is.
///
/// The surface reused the *file format's* citation type at first, and that type
/// omits `captured` when there is none -- correctly, because a task file should
/// not carry `captured: null` on a commit. A contract a consumer reads is the
/// opposite case: a key that comes and goes hands it `undefined` where it
/// expected a value. Worse, the shape would have moved whenever somebody
/// adjusted the YAML for reasons that have nothing to do with this surface.
#[test]
fn a_citation_has_the_same_keys_whatever_kind_it_is() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let mut seen = 0_usize;
    for row in rows(dir.path()) {
        for citation in row["evidence"].as_array().expect("evidence") {
            seen += 1;
            let mut keys: Vec<&str> = citation
                .as_object()
                .expect("a citation is an object")
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                ["captured", "kind", "ref"],
                "a {} citation has a different shape from the others",
                citation["kind"]
            );
        }
    }
    // The fixture has a dated note and an undated commit, which is the pair
    // that distinguishes carrying the key from omitting it.
    assert!(
        seen >= 3,
        "only {seen} citations, so the shapes are not compared"
    );
}

/// The task-file and task-data DTOs intentionally have different shapes, but
/// they must not teach different meanings for a field they share (`A1-493`).
#[test]
fn both_task_schemas_give_captured_the_same_note_only_meaning() {
    let read = |path: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(workspace_file(path)).expect("committed schema"))
            .expect("committed schema is JSON")
    };
    let task = read("schemas/v1/task.json");
    let tasks = read("schemas/v1/tasks.json");
    let authored_evidence = &task["$defs"]["Evidence"];
    let authored = &authored_evidence["properties"]["captured"];
    let data = &tasks["$defs"]["EvidenceRow"]["properties"]["captured"];

    assert_eq!(
        authored["description"], data["description"],
        "the file and data contracts disagree about capture provenance"
    );
    let description = data["description"].as_str().expect("description");
    for claim in ["inline assertion", "Required for `note`", "APLAN055"] {
        assert!(
            description.contains(claim),
            "the shared description no longer states `{claim}`: {description}"
        );
    }

    // The semantics agree without collapsing the two deliberate wire shapes:
    // authored YAML omits an absent date, while every data row carries null.
    assert!(
        !authored_evidence["required"]
            .as_array()
            .expect("authored evidence required fields")
            .iter()
            .any(|field| field == "captured")
    );
    assert!(
        tasks["$defs"]["EvidenceRow"]["required"]
            .as_array()
            .expect("row required fields")
            .iter()
            .any(|field| field == "captured")
    );
    assert_eq!(data["type"], serde_json::json!(["string", "null"]));
}

/// `TaskRow` never skips a field while serializing, including an absent
/// `Option`; the schema must describe that stable-key contract (`A1-493`).
#[test]
fn the_schema_requires_every_runtime_row_key() {
    let tasks: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace_file("schemas/v1/tasks.json")).expect("committed schema"),
    )
    .expect("committed schema is JSON");
    let required = tasks["$defs"]["TaskRow"]["required"]
        .as_array()
        .expect("row required fields");

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    for key in rows(dir.path())[0].as_object().expect("runtime row").keys() {
        assert!(
            required.iter().any(|field| field == key),
            "runtime always emits `{key}`, but the schema permits it to be absent"
        );
    }
}

/// The surface's rule, checkable in both directions.
///
/// "What a task *is*, plus what the plan derives about it." The other test
/// checks that every declared field arrives; this checks that nothing else does,
/// so a field cannot be added to the contract without being one or the other.
#[test]
fn every_row_field_is_declared_or_a_named_derivation() {
    /// Derived, and why each is here rather than on the task.
    const DERIVED: &[&str] = &[
        // Who is acting on it now, from the coherent local ephemeral claim
        // snapshot carried by the envelope; durable responsibility remains the
        // task's declared `owner` (`A1-531`).
        "claim",
        // The label of the cut that lists it, from the roadmap.
        "cut",
        // The id of that cut, since two may share a label.
        "milestone",
        // Whether this task *is* a cut, so a rollup can exclude checkpoints.
        "milestone_task",
        // Whether every blocker is done: the same answer `ready` gives.
        "ready",
        // Where `ready` puts it, `null` when it is not offered (`A1-125`).
        "rank",
    ];

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let schema = common::output(dir.path(), &["schema"]);
    let schema: serde_json::Value = serde_json::from_slice(&schema.stdout).expect("schema is JSON");
    let declared: Vec<&str> = schema["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .map(String::as_str)
        .collect();

    let row = rows(dir.path());
    let row = row[0].as_object().expect("a row is an object");
    let unexplained: Vec<&String> = row
        .keys()
        .filter(|k| !declared.contains(&k.as_str()) && !DERIVED.contains(&k.as_str()))
        .collect();

    assert!(
        unexplained.is_empty(),
        "{unexplained:?} on a row is neither a field a task declares nor a derivation named \
         here. The surface's rule is `what a task is, plus what the plan derives about it`: \
         add it to `DERIVED` with the reason, or it does not belong on the contract"
    );
}
