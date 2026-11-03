//! What this repository's own traces are pointed at, and why it matters
//! (`A1-122`).
//!
//! ADR-0015 made a table row an anchor because a register kept as a table has one
//! heading and N independent facts: a task tracing the heading is re-flagged by
//! an edit to any row, including the ones it never touched. `A1-008` and `A1-009`
//! were re-stamped three times in one session for exactly that, and twice more in
//! the session that migrated them.
//!
//! The property is not "the field contains a `#`". It is that **an edit to an
//! unrelated row re-flags nothing**, which is only true while the traces point at
//! rows. This runs the tool over a copy of this repository's own plan and
//! documents, edits one row nobody traces, and asks `verify` whether anything
//! drifted. Revert the migration and it fails, which is what makes it evidence.

use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use crate::common;
use common::aplan;

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy");
        }
    }
}

fn copy_traced_documents(root: &Path, to: &Path) {
    let mut documents = BTreeSet::new();
    for entry in std::fs::read_dir(root.join("aplan/tasks")).expect("read tasks") {
        let path = entry.expect("task entry").path();
        if path.extension().is_none_or(|extension| extension != "md") {
            continue;
        }
        let task = std::fs::read_to_string(path).expect("read task");
        for trace in traces_of(&task) {
            let Some((document, _)) = trace.split_once('#') else {
                continue;
            };
            let relative = Path::new(document);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                continue;
            }
            let source = root.join(relative);
            if source.is_file() {
                documents.insert(relative.to_path_buf());
            }
        }
    }

    for relative in documents {
        let target = to.join(&relative);
        std::fs::create_dir_all(target.parent().expect("traced document parent"))
            .expect("create traced document parent");
        std::fs::copy(root.join(&relative), target).expect("copy traced document");
    }
}

/// This repository's plan and the documents it traces, copied so a test may edit
/// them. Not a git repository: drift is hermetic, and `check` is the command that
/// reports it.
fn plan_copy() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = repo_root();
    copy_dir(&root.join("aplan"), &dir.path().join("aplan"));
    // Root policy and documents are cheap and include `.aplan.yml`. Below
    // `docs/`, copy the exact in-repository documents that traces name. Walking
    // that whole tree makes this fixture depend on ignored prototype build
    // output rather than on the trace contract it is meant to exercise.
    for entry in std::fs::read_dir(&root).expect("read root") {
        let entry = entry.expect("entry");
        let name = entry.file_name();
        let is_doc = Path::new(&name)
            .extension()
            .is_some_and(|e| e == "md" || e == "yml");
        if entry.file_type().expect("file type").is_file() && is_doc {
            std::fs::copy(entry.path(), dir.path().join(&name)).expect("copy doc");
        }
    }
    copy_traced_documents(&root, dir.path());
    dir
}

#[test]
fn trace_fixture_copies_only_in_repository_traced_documents() {
    let sandbox = tempfile::tempdir_in(aplan_core::fixture::fixture_root()).expect("sandbox");
    let source = sandbox.path().join("source/repository");
    let copy = sandbox.path().join("copy/repository");
    let tasks = source.join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("tasks");
    std::fs::write(
        tasks.join("A1-001.md"),
        "---\ntraces:\n- docs/kept.md#kept\n- ../outside.md#outside\n---\n",
    )
    .expect("task");

    let docs = source.join("docs");
    std::fs::create_dir_all(docs.join("prototype/target/dependency")).expect("artifact tree");
    std::fs::write(docs.join("kept.md"), "# Kept\n").expect("traced document");
    std::fs::write(docs.join("untraced.md"), "# Untraced\n").expect("untraced document");
    std::fs::write(
        docs.join("prototype/target/dependency/README.md"),
        "# Build output\n",
    )
    .expect("build output");
    std::fs::write(
        source.parent().expect("source parent").join("outside.md"),
        "# Outside\n",
    )
    .expect("outside document");

    copy_traced_documents(&source, &copy);

    assert!(copy.join("docs/kept.md").is_file());
    assert!(!copy.join("docs/untraced.md").exists());
    assert!(!copy.join("docs/prototype/target").exists());
    assert!(
        !copy
            .parent()
            .expect("copy parent")
            .join("outside.md")
            .exists()
    );
}

fn drifted(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains("APLAN018"))
        .map(ToString::to_string)
        .collect()
}

/// The migration's whole point, as a property rather than as a field value.
#[test]
fn an_edit_to_a_row_nobody_traces_re_flags_nobody() {
    let dir = plan_copy();
    let hld = dir.path().join("HLD.md");
    let before = std::fs::read_to_string(&hld).expect("read");

    // Whatever the plan already reports is not this test's business: the claim is
    // about what *this edit* re-flags, so the answer is a delta, the way every
    // mutation in this tool is judged (`A1-113`).
    let (before_out, _) = aplan(dir.path(), &["check"]);
    let already = drifted(&before_out);

    // A row of the CLI table that no task traces, edited the way any ordinary
    // change to that table edits one row.
    let after = before.replacen("| `aplan tui` | ", "| `aplan tui` | fixture-edited ", 1);
    assert_ne!(before, after, "the fixture row moved; pick another");
    std::fs::write(&hld, after).expect("write");

    let (out, _) = aplan(dir.path(), &["check"]);
    let new_drift: Vec<String> = drifted(&out)
        .into_iter()
        .filter(|d| !already.contains(d))
        .collect();
    assert!(
        new_drift.is_empty(),
        "editing a row nobody traces re-flagged somebody, which is the defect \
         ADR-0015 exists to remove:\n{}",
        new_drift.join("\n")
    );
}

/// And the other direction, so the test above cannot pass by the drift detector
/// being off: editing the row a task *does* trace re-flags that task, and only
/// that task.
/// Proves the named behaviour `traces.table_rows` (`A1-211`).
#[test]
fn an_edit_to_a_traced_row_re_flags_exactly_its_owner() {
    if !repo_root().join("aplan/tasks/A1-009.md").is_file() {
        let tasks = std::fs::read_dir(repo_root().join("aplan/tasks"))
            .expect("public plan task directory")
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|value| value == "md"))
            .collect::<Vec<_>>();
        assert!(
            !tasks.is_empty()
                && tasks.iter().all(|entry| {
                    entry.file_name().to_string_lossy().starts_with("A2-")
                        && !std::fs::read_to_string(entry.path())
                            .expect("public task")
                            .contains("\ntraces:")
                }),
            "the historical traced-row owner may be absent only in the trace-free A2 epoch"
        );
        return;
    }
    let dir = plan_copy();
    let hld = dir.path().join("HLD.md");
    let before = std::fs::read_to_string(&hld).expect("read");
    let (before_out, _) = aplan(dir.path(), &["check"]);
    let already = drifted(&before_out);

    // The row `A1-009` traces, and nothing else does.
    let row = before
        .lines()
        .find(|l| l.starts_with("| `aplan init` |"))
        .expect("the row `A1-009` traces")
        .to_string();
    let reworded = row.replace("refuses to overwrite", "refusing to overwrite one");
    assert_ne!(row, reworded, "the fixture row moved; pick another");
    std::fs::write(&hld, before.replace(&row, &reworded)).expect("write");

    let (out, _) = aplan(dir.path(), &["check"]);
    let new_drift: Vec<String> = drifted(&out)
        .into_iter()
        .filter(|d| !already.contains(d))
        .collect();
    assert_eq!(
        new_drift.len(),
        1,
        "exactly the owner of the edited row, and nobody else:\n{out}"
    );
    assert!(new_drift[0].contains("A1-009"), "{}", new_drift[0]);
}

/// Every anchor this plan traces is minted by exactly one thing in its document
/// (`A1-122`).
///
/// A row anchor is the slug of the first cell, and a repeated first cell mints
/// `name-1`, `name-2` in document order the way a repeated heading does. That is
/// predictable and it is also a trap for a *trace*: inserting a row above a
/// duplicate shifts every later suffix, so a trace that named `name-1` silently
/// starts covering a different fact. The answer is not to disambiguate more
/// cleverly but to trace something whose name is unique, which is what this
/// asserts about the plan as it stands. `HLD.md` has five suffixed row anchors
/// today (`aplan-mcp-1` and friends), and nothing traces any of them.
#[test]
fn no_trace_names_an_anchor_two_things_could_mint() {
    let root = repo_root();
    let mut offenders = Vec::new();

    for entry in std::fs::read_dir(root.join("aplan/tasks")).expect("tasks") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("task");
        let id = path
            .file_stem()
            .expect("stem")
            .to_string_lossy()
            .into_owned();
        for trace in traces_of(&text) {
            let Some((doc, anchor)) = trace.split_once('#') else {
                continue;
            };
            let doc_path = root.join(doc);
            let Ok(body) = std::fs::read_to_string(&doc_path) else {
                continue;
            };
            let minted = minters(&body, anchor);
            if minted > 1 {
                offenders.push(format!("{id} traces `{trace}`, which {minted} lines mint"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a trace whose anchor is not unique moves when a row is inserted above it:\n  {}",
        offenders.join("\n  ")
    );
}

/// The `traces:` entries of a task file.
fn traces_of(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("---") && !out.is_empty() {
            break;
        }
        if line.starts_with("traces:") {
            inside = true;
            continue;
        }
        if inside {
            match line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("  - "))
            {
                Some(t) => out.push(t.trim().to_string()),
                None => inside = false,
            }
        }
    }
    out
}

/// How many headings and table rows in `body` slugify to `anchor`.
///
/// Deliberately counts the *base* name rather than asking the tool which line it
/// resolved to: the question is whether anything else could have minted it, and
/// the tool's answer is the one that silently changes.
fn minters(body: &str, anchor: &str) -> usize {
    let slug = |s: &str| {
        let s = s.replace('`', "");
        let s: String = s
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_')
            .collect();
        s.split_whitespace().collect::<Vec<_>>().join("-")
    };
    let mut fenced = false;
    let mut n = 0;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let name = if let Some(rest) = line.strip_prefix('#') {
            rest.trim_start_matches('#').to_string()
        } else if line.trim_start().starts_with('|') {
            let row = line.trim().trim_matches('|');
            if row.chars().all(|c| "-: |".contains(c)) {
                continue;
            }
            row.split('|').next().unwrap_or_default().to_string()
        } else {
            continue;
        };
        if slug(&name) == anchor.to_lowercase() {
            n += 1;
        }
    }
    n
}

// --- a trace that leaves the repository (`A1-143`, ADR-0021) -----------------

/// A sibling checkout beside a repository, the fleet shape the fifth pilot met.
fn repo_beside_a_sibling() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("aplan/tasks")).expect("plan dir");
    std::fs::create_dir_all(dir.path().join("sibling/docs")).expect("sibling");
    std::fs::write(
        dir.path().join("sibling/docs/STATUS.md"),
        "# Status\n\n## Phase 1\n\nIn progress.\n",
    )
    .expect("sibling doc");
    std::fs::write(
        repo.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: Traces a sibling\nphase: 1\nstatus: todo\neffort_days: 1\n\
         traces:\n- ../sibling/docs/STATUS.md#phase-1\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    dir
}

/// The reported defect: the trace resolves wherever the sibling is checked out,
/// and says nothing about the dependency that creates.
#[test]
fn a_trace_into_a_sibling_is_refused_until_it_is_declared() {
    let dir = repo_beside_a_sibling();
    let repo = dir.path().join("repo");

    let (out, code) = aplan(&repo, &["check"]);
    assert_eq!(code, 1, "an undeclared cross-repo trace passed:\n{out}");
    assert!(out.contains("APLAN034"), "{out}");
    assert!(
        out.contains("../sibling/docs/STATUS.md") && out.contains("traces_external"),
        "the finding must name the target and the remedy:\n{out}"
    );

    // Declared: it resolves, exactly as it did before the rule existed.
    std::fs::write(repo.join(".aplan.yml"), "traces_external: ['../sibling']\n").expect("config");
    let (out, code) = aplan(&repo, &["check"]);
    assert_eq!(code, 0, "a declared root was still refused:\n{out}");
}

/// And the half that makes the declaration worth having: the dependency is
/// stated, so its absence is a configuration error naming the key rather than a
/// finding about a task that is fine.
#[test]
fn a_declared_root_that_is_not_there_stops_the_run_as_configuration() {
    let dir = repo_beside_a_sibling();
    let repo = dir.path().join("repo");
    std::fs::write(repo.join(".aplan.yml"), "traces_external: ['../gone']\n").expect("config");

    for command in ["check", "verify"] {
        let (out, code) = aplan(&repo, &[command]);
        assert_eq!(
            code, 2,
            "{command} did not treat it as configuration:\n{out}"
        );
        assert!(out.contains("traces_external"), "{command}: {out}");
        assert!(
            !out.contains("T-001"),
            "{command} blamed a task for the configuration:\n{out}"
        );
    }
}

/// The declaration means the same thing however the run is invoked (`A1-143`,
/// found by its audit).
///
/// `traces_external` was first resolved against `traces_root`, so
/// `--traces-root docs` silently redefined `../sibling` as `<repo>/sibling` and
/// the declared root stopped being declared. A config key whose meaning a flag
/// can change is not a declaration.
#[test]
fn a_flag_does_not_redefine_a_declared_external_root() {
    let dir = repo_beside_a_sibling();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("docs")).expect("docs");
    std::fs::write(repo.join(".aplan.yml"), "traces_external: ['../sibling']\n").expect("config");

    let (out, code) = aplan(&repo, &["check"]);
    assert_eq!(code, 0, "{out}");

    // The discriminating case: a trace that leaves the repository *from the
    // moved root*, so only a correctly-resolved declaration can cover it. With
    // the declaration resolved against `traces_root`, `../sibling` would mean
    // `<repo>/sibling` here and the trace would be refused.
    std::fs::write(
        repo.join("aplan/tasks/T-001.md"),
        "---\nid: T-001\ntitle: Traces a sibling\nphase: 1\nstatus: todo\neffort_days: 1\n\
         traces:\n- ../../sibling/docs/STATUS.md#phase-1\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    let (out, code) = aplan(&repo, &["check", "--traces-root", "docs"]);
    assert!(
        !out.contains("APLAN034"),
        "a flag redefined what the config key declares:\n{out}"
    );
    assert_eq!(code, 0, "the declared sibling did not resolve:\n{out}");
}

/// The agent surface refuses what the commands refuse (`A1-143`, audit).
///
/// `aplan mcp` reads the config once, at startup, and a server that cannot
/// resolve a declared external root would answer every `aplan_check` with
/// `APLAN008` about tasks that are fine -- which is the noise declaring the root
/// exists to prevent. It refuses to start instead, and this pins that the two
/// surfaces agree (`A1-104`).
#[test]
fn the_agent_surface_refuses_an_absent_declared_root_too() {
    let dir = repo_beside_a_sibling();
    let repo = dir.path().join("repo");
    std::fs::write(repo.join(".aplan.yml"), "traces_external: ['../gone']\n").expect("config");

    let out = common::command(&repo, &["mcp"])
        // No stdin: if the refusal were gone the server would start, read EOF
        // and exit cleanly, which is what tells the two apart.
        .stdin(std::process::Stdio::null())
        .output()
        .expect("running aplan mcp");
    let text = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "the server started with a root it cannot resolve: {text}"
    );
    assert!(text.contains("traces_external"), "{text}");
}
