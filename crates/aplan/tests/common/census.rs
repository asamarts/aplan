//! The command census: how to drive every command this binary has, and the
//! fixture to drive it against.
//!
//! It lives here because two gates need the same list, and a second copy of it
//! would drift. `mutation_policy.rs` (`A1-266`, `A1-356`) measures which
//! commands write the plan; `no_effect_outside_the_plan.rs` (`A1-354`,
//! ADR-0043) measures what each one spawns. Both are answers to "what does
//! every command actually do", and either is worth only as much as the list
//! being the whole surface -- which is asserted against clap's own tree in
//! `mutation_policy.rs` rather than trusted here.
//!
//! Every item is `allow(dead_code)` for the reason the parent module gives: it
//! is compiled into each test binary that declares it, and no one binary uses
//! all of it.
#![allow(dead_code)]

use std::path::Path;

/// Where a command starts, for the measurements that drive it.
#[derive(Clone, Copy, PartialEq)]
pub enum Start {
    /// A plan with a cut, a live task, a closed one and a document to import.
    Populated,
    /// An empty directory. `init` needs one: against a plan that already exists
    /// it refuses, and against a *subdirectory* it would write outside the tree
    /// being measured, which would read as "writes nothing" and be wrong.
    Empty,
}

/// How to run each command so that it reaches its work.
///
/// A command that cannot be driven at all says why instead, in [`NOT_RUN`].
pub const RUNS: &[(&str, &[&str], Start)] = &[
    (
        "add",
        &["add", "New work", "--effort", "1"],
        Start::Populated,
    ),
    (
        "amend",
        &["amend", "W-2", "--add-evidence", "file:NOTES.md"],
        Start::Populated,
    ),
    ("attest", &["attest"], Start::Populated),
    (
        "claim",
        &["claim", "W-1", "--as", "measurement"],
        Start::Populated,
    ),
    (
        "blueprint",
        &[
            "blueprint",
            "apply",
            "--file",
            "feature.blueprint.yml",
            "--cut",
            "M-a",
            "--set",
            "feature=census",
        ],
        Start::Populated,
    ),
    ("capture", &["capture", "Census capture"], Start::Populated),
    ("check", &["check"], Start::Populated),
    ("check-docs", &["check-docs", "NOTES.md"], Start::Populated),
    (
        "close",
        &["close", "W-1", "--evidence", "file:NOTES.md"],
        Start::Populated,
    ),
    ("critical-path", &["critical-path"], Start::Populated),
    (
        "dep",
        &["dep", "add", "W-1", "--after", "W-3"],
        Start::Populated,
    ),
    ("deprecations", &["deprecations"], Start::Populated),
    ("drop", &["drop", "W-3"], Start::Populated),
    ("due", &["due"], Start::Populated),
    (
        "explain",
        &["explain", "rule", "APLAN012"],
        Start::Populated,
    ),
    ("export", &["export"], Start::Populated),
    (
        "export-agents-md",
        &["export-agents-md", "--write", "AGENTS.md"],
        Start::Populated,
    ),
    (
        "export-roadmap-json",
        &["export-roadmap-json", "ROADMAP.json"],
        Start::Populated,
    ),
    ("facts", &["facts"], Start::Populated),
    ("graph", &["graph"], Start::Populated),
    (
        "import",
        &["import", "NOTES.md", "--format", "checklist"],
        Start::Populated,
    ),
    ("inbox", &["inbox"], Start::Populated),
    ("init", &["init"], Start::Empty),
    ("mv", &["mv", "W-1", "--cut", "M-b"], Start::Populated),
    (
        "park",
        &["park", "W-3", "--reason", "portfolio deferred"],
        Start::Populated,
    ),
    ("paths", &["paths"], Start::Populated),
    ("ready", &["ready"], Start::Populated),
    ("rekey", &["rekey", "W-1", "W-9"], Start::Populated),
    ("release", &["release", "W-4"], Start::Populated),
    ("roadmap", &["roadmap"], Start::Populated),
    ("schema", &["schema"], Start::Populated),
    ("show", &["show", "W-1"], Start::Populated),
    ("stale", &["stale"], Start::Populated),
    ("start", &["start", "W-1"], Start::Populated),
    ("stop", &["stop", "W-4"], Start::Populated),
    ("summary", &["summary"], Start::Populated),
    (
        "supersede",
        &["supersede", "W-2", "A successor", "--effort", "1"],
        Start::Populated,
    ),
    ("tag", &["tag", "add", "W-1", "kept"], Start::Populated),
    ("test", &["test", "list"], Start::Populated),
    ("tasks", &["tasks"], Start::Populated),
    ("tree", &["tree", "W-1"], Start::Populated),
    ("unpark", &["unpark", "W-5"], Start::Populated),
    ("vacuity", &["vacuity"], Start::Populated),
    ("verify", &["verify"], Start::Populated),
    // Keep the repository-reading form here. Besides exercising the command,
    // the configured-command effect audit relies on this census entry to
    // prove that `version.current` is the only external command it reaches.
    ("version", &["version"], Start::Populated),
    ("why", &["why", "W-1"], Start::Populated),
];

/// The commands this measurement cannot drive, and why.
pub const NOT_RUN: &[(&str, &str)] = &[
    (
        "mcp",
        "serves the tools over stdio and does not exit, so running it here would hang \
         rather than answer. Each tool it serves is one of the commands above",
    ),
    (
        "tui",
        "owns an interactive terminal until the user quits; a pipe is deliberately refused, \
         and PTY lifecycle measurement belongs to the dedicated terminal harness",
    ),
];

/// A plan with a cut, a live task, a closed one, and a document to import.
pub fn a_plan_to_act_on(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    // The bare `version` form is the repository-reading half of that command.
    // Give it a deterministic artifact answer so every consumer of this
    // census drives real work rather than treating a missing-config refusal as
    // command coverage.
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\nversion:\n  current:\n    command: [printf, '0.1.0\\n']\n",
    )
    .expect("config");
    std::fs::write(
        dir.join("NOTES.md"),
        "# Notes\n\n- [ ] something to import\n",
    )
    .expect("doc");
    std::fs::write(
        dir.join("feature.blueprint.yml"),
        concat!(
            "schema: aplan/blueprint/v1\n",
            "name: census\n",
            "title: Census blueprint\n",
            "description: Drive the command census.\n",
            "parameters:\n",
            "  - name: feature\n",
            "    description: Feature name.\n",
            "tasks:\n",
            "  - key: work\n",
            "    title: 'Build {{feature}}'\n",
            "    effort: 1\n",
            "    paths: [src/**]\n",
        ),
    )
    .expect("blueprint");
    std::fs::write(
        dir.join("ROADMAP.json"),
        r#"{"format_version":1,"phases":[],"workstreams":[{"id":"fixture","title":"Fixture","milestones":[{"id":"M-a","title":"a cut","status":"not_started"},{"id":"M-b","title":"another cut","status":"not_started"}]}]}
"#,
    )
    .expect("foreign roadmap contract");
    std::fs::write(
        dir.join("aplan/tasks/M-a.md"),
        "---\nid: M-a\ntitle: a cut\nphase: 1\nstatus: todo\ntags:\n  - milestone\n\
         blocked_by:\n  - W-1\n---\n\n## Why\n\nA cut.\n",
    )
    .expect("cut");
    std::fs::write(
        dir.join("aplan/tasks/M-b.md"),
        "---\nid: M-b\ntitle: another cut\nphase: 1\nstatus: todo\ntags:\n  - milestone\n\
         blocked_by:\n  - W-3\n---\n\n## Why\n\nA cut.\n",
    )
    .expect("cut");
    for (id, status, extra) in [
        ("W-1", "todo", ""),
        (
            "W-2",
            "done",
            "evidence:\n  - kind: note\n    ref: reviewed\n    captured: 2026-08-14\n",
        ),
        ("W-3", "todo", ""),
        // Held and under way, so `release` and `stop` have something to act on
        // (`A1-359`). Without it the measurement would run them against a task
        // in neither state, they would correctly write nothing, and the census
        // would conclude they are not writing commands.
        ("W-4", "in_progress", "owner: ada\n"),
        // Parked and unclaimed, so `unpark` reaches and publishes its guarded
        // post-state rather than being measured at an argument refusal.
        ("W-5", "blocked", "revisit_at: 2030-01-02T03:04:05Z\n"),
    ] {
        std::fs::write(
            dir.join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: task {id}\nphase: 1\nstatus: {status}\n\
                 effort_days: 1\n{extra}---\n\n## Why\n\nBecause.\n"
            ),
        )
        .expect("task");
    }
}

/// Every file under the plan directory, as bytes.
pub fn plan_bytes(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let root = dir.join("aplan");
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                out.insert(path.display().to_string(), bytes);
            }
        }
    }
    out
}
