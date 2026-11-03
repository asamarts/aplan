//! Phase is one required reporting coordinate across the versioned task surfaces.
//!
//! ADR-0046 keeps the field and narrows its meaning: it is not cut membership,
//! tag-family membership, or executable order. This regression binds the
//! stability half of that decision to the real CLI and committed schemas.

use std::path::{Path, PathBuf};

use crate::common;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: Phase seven\nphase: 7\nstatus: todo\neffort_days: 1\n\
         paths: [src/**]\n---\n\n## Why\n\nFixture.\n",
    )
    .expect("task");
    dir
}

fn json(root: &Path, args: &[&str]) -> serde_json::Value {
    let run = common::run(root, args);
    assert_eq!(run.code, 0, "{}", run.text());
    serde_json::from_str(&run.stdout).expect("command emits JSON")
}

fn workspace_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .join(path)
}

fn assert_required_integer(schema: &serde_json::Value, definition: Option<&str>) {
    let object = definition.map_or(schema, |name| &schema["$defs"][name]);
    let required = object["required"].as_array().expect("required fields");
    assert!(required.iter().any(|field| field == "phase"), "{object}");
    assert_eq!(object["properties"]["phase"]["type"], "integer");
    assert_eq!(object["properties"]["phase"]["minimum"], 0);
}

#[test]
fn phase_is_required_and_reaches_every_versioned_task_view() {
    let dir = repo();

    let task_schema = json(dir.path(), &["schema"]);
    assert_required_integer(&task_schema, None);

    let summary = json(dir.path(), &["summary", "--format", "json"]);
    assert_eq!(summary["schema"], "aplan/summary/v2");
    assert_eq!(summary["phases"][0]["phase"], 7);

    let tasks = json(dir.path(), &["tasks", "--format", "json"]);
    assert_eq!(tasks["schema"], "aplan/tasks/v2");
    assert_eq!(tasks["tasks"][0]["phase"], 7);

    let show = json(
        dir.path(),
        &["show", "T-1", "--format", "json", "--no-resolve"],
    );
    assert_eq!(show["schema"], "aplan/show/v3");
    assert_eq!(show["phase"], 7);

    let committed_task: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace_file("schemas/v1/task.json")).expect("task schema"),
    )
    .expect("committed task schema is JSON");
    assert_required_integer(&committed_task, None);

    let committed_tasks: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace_file("schemas/v1/tasks.json")).expect("tasks schema"),
    )
    .expect("committed tasks schema is JSON");
    assert_required_integer(&committed_tasks, Some("TaskRow"));
}
