//! The tool surface, as plain functions.
//!
//! Deliberately independent of the transport: every rule that matters here is a
//! property of what a tool *does*, not of how a request arrived, so the logic is
//! testable without standing up a server. The `rmcp` handler is a thin adapter
//! over these.
//!
//! The surface is **small on purpose** (`ready`, `why`, `check`, `close`).
//! task-master ships ~36 tools costing ~21k tokens of context by default and had
//! to add an env var to trim them (RESEARCH section 6); an ETH Zurich study over
//! 138 repositories found context files raise inference cost by more than 20% and
//! can *lower* success. A small tool surface is a feature, not an unfinished one.

use std::path::{Path, PathBuf};

use aplan_core::{Graph, Status, load_plan, validate};
use serde::{Deserialize, Serialize};

/// The tools this server exposes, and the single place the surface is defined.
///
/// A test pins this list, so the surface cannot grow silently: an added tool is
/// a deliberate, reviewed change to what an agent can do, not a side effect.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "aplan_ready",
        description: "List tasks whose blockers are all done, so an agent can pick real work.",
    },
    ToolSpec {
        name: "aplan_why",
        description: "Explain what is blocking a task, transitively.",
    },
    ToolSpec {
        name: "aplan_check",
        description: "Validate the plan and return its diagnostics. Hermetic: no git, no network.",
    },
    ToolSpec {
        name: "aplan_close",
        description: "Close a task with evidence. Refuses exactly what the CLI refuses; \
                      evidence is a citation to be resolved, never a score the caller asserts.",
    },
];

/// One tool's identity, kept minimal for the same reason the surface is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    /// Tool name as an agent sees it.
    pub name: &'static str,
    /// One line, because a description is context an agent pays for.
    pub description: &'static str,
}

/// Why a tool call could not be served.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// The plan could not be loaded, or does not validate.
    #[error("{0}")]
    Plan(String),
    /// The arguments were wrong.
    #[error("{0}")]
    Args(String),
    /// `close` refused, for one of the reasons the CLI refuses.
    #[error("{0}")]
    Refused(String),
}

/// A task as `ready` reports it.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ReadyTask {
    /// Task id.
    pub id: String,
    /// One-line title.
    pub title: String,
    /// Estimated effort, when the task carries one.
    pub effort_days: Option<f64>,
}

/// What `why` reports about one blocker.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Blocker {
    /// Blocking task id.
    pub id: String,
    /// Its status, so an agent can tell "not started" from "abandoned".
    pub status: String,
    /// Its title.
    pub title: String,
}

/// Tasks that can be started now.
pub fn ready(root: &Path) -> Result<Vec<ReadyTask>, ToolError> {
    let plan = load_plan(root);
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    Ok(graph
        .ready()
        .into_iter()
        .map(|t| ReadyTask {
            id: t.id.clone(),
            title: t.title.clone(),
            effort_days: t.effort_days,
        })
        .collect())
}

/// What a task is waiting on, transitively.
pub fn why(root: &Path, id: &str) -> Result<Vec<Blocker>, ToolError> {
    let plan = load_plan(root);
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    if graph.get(id).is_none() {
        return Err(ToolError::Args(format!("unknown task `{id}`")));
    }
    Ok(graph
        .why_blocked(id)
        .into_iter()
        .map(|t| Blocker {
            id: t.id.clone(),
            status: status_word(t.status).to_string(),
            title: t.title.clone(),
        })
        .collect())
}

/// The plan's diagnostics, in the same shape `--format json` returns.
pub fn check(root: &Path, options: &validate::Options) -> Result<serde_json::Value, ToolError> {
    let plan = load_plan(root);
    let report = validate(&plan, options);
    Ok(serde_json::json!({
        "tasks_checked": report.tasks_checked,
        "errors": report.errors(),
        "warnings": report.warnings(),
        "diagnostics": report.diagnostics.iter().map(|d| serde_json::json!({
            "code": d.code,
            "severity": match d.severity {
                aplan_core::Severity::Error => "error",
                aplan_core::Severity::Warning => "warning",
            },
            "task": d.task,
            "file": d.file.as_ref().map(|f| f.display().to_string()),
            "message": d.message,
            "agent_instruction": d.agent_instruction,
        })).collect::<Vec<_>>(),
    }))
}

/// Close a task, through the same gate the CLI uses.
///
/// This is the tool the whole task exists to get right. The anti-pattern it
/// refuses by example is `mcp-shrimp-task-manager`'s `verify_task`, which takes a
/// completion *score* from the calling agent and auto-completes at 80 or above:
/// an agent grading its own homework. There is no score parameter here and no
/// path that could accept one. The only thing a caller may supply is *evidence*,
/// a citation that something else resolves, and `aplan_core::close` applies the
/// identical refusals it applies to the CLI (unknown task, still blocked, already
/// done, already carrying evidence). A `note` is refused earlier still, at parse
/// time, for the same reason the CLI refuses it: it needs a `captured` date that
/// neither surface will invent.
pub fn close(root: &Path, id: &str, evidence: &[String]) -> Result<serde_json::Value, ToolError> {
    if evidence.is_empty() {
        return Err(ToolError::Args(
            "`evidence` is required: a status is a claim, and MCP does not get to make it \
             without one. Cite a commit, test, file, or url."
                .to_string(),
        ));
    }

    let parsed = parse_evidence(evidence)?;
    let plan = load_plan(root);

    // Never edit a task inside a plan that does not load: its neighbours may be
    // the reason this one should not close. The CLI takes the same position.
    if !plan.load_diagnostics.is_empty() {
        return Err(ToolError::Plan(
            "the plan does not load cleanly; fix it before closing anything".to_string(),
        ));
    }

    let target = plan
        .tasks
        .iter()
        .find(|lt| lt.task.id == id)
        .ok_or_else(|| ToolError::Args(format!("unknown task `{id}`")))?;

    // The blocked-task refusal lives in the graph, not in `aplan_core::close`,
    // which only ever sees one file. Reusing the shared predicate is what keeps
    // this surface exactly as strict as the CLI: a private reimplementation here
    // is how an agent ends up able to close through MCP what the CLI refuses.
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let unmet = graph.unmet_blockers(id);
    if !unmet.is_empty() {
        return Err(ToolError::Refused(format!(
            "cannot close `{id}`: blocker(s) not done: {}",
            unmet.join(", ")
        )));
    }

    let source = std::fs::read_to_string(&target.path)
        .map_err(|e| ToolError::Plan(format!("reading {}: {e}", target.path.display())))?;

    // No `trace_fingerprints`: stamping drift is `close --restamp`'s job and
    // needs the traced documents, which this server does not read. An empty map
    // means "stamp nothing", not "the stamps are current".
    let updated = aplan_core::close(&source, &parsed, &std::collections::BTreeMap::new())
        .map_err(|e| ToolError::Refused(e.to_string()))?;

    std::fs::write(&target.path, updated)
        .map_err(|e| ToolError::Plan(format!("writing {}: {e}", target.path.display())))?;

    Ok(serde_json::json!({
        "closed": id,
        "file": target.path.display().to_string(),
        "evidence": evidence,
    }))
}

/// Parse `kind:ref` evidence the way the CLI does.
fn parse_evidence(raw: &[String]) -> Result<Vec<aplan_core::Evidence>, ToolError> {
    raw.iter()
        .map(|item| {
            let (kind, reference) = item
                .split_once(':')
                .filter(|(k, r)| !k.is_empty() && !r.is_empty())
                .ok_or_else(|| {
                    ToolError::Args(format!(
                        "evidence `{item}` is not `kind:ref` (for example `commit:d213c9a`)"
                    ))
                })?;
            // The same table the CLI uses, including its refusal of `note`:
            // a note needs a `captured` date, and neither the CLI nor this
            // server invents one. An agent that wants a note writes it by hand,
            // where a human sees the date it claims.
            let kind = match kind {
                "commit" => aplan_core::EvidenceKind::Commit,
                "test" => aplan_core::EvidenceKind::Test,
                "file" => aplan_core::EvidenceKind::File,
                "url" => aplan_core::EvidenceKind::Url,
                "note" => {
                    return Err(ToolError::Args(
                        "`note` evidence needs a `captured` date; add it by hand rather than \
                         through `close`"
                            .to_string(),
                    ));
                }
                other => {
                    return Err(ToolError::Args(format!(
                        "unknown evidence kind `{other}` (commit, test, file, url)"
                    )));
                }
            };
            Ok(aplan_core::Evidence {
                kind,
                reference: reference.to_string(),
                captured: None,
            })
        })
        .collect()
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::Todo => "todo",
        Status::InProgress => "in_progress",
        Status::Blocked => "blocked",
        Status::Done => "done",
        Status::Dropped => "dropped",
    }
}

/// The plan root a request applies to.
pub fn resolve_root(explicit: Option<&str>, default_root: &Path) -> PathBuf {
    explicit.map_or_else(|| default_root.to_path_buf(), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plan directory with one open task blocked by an unfinished one.
    fn plan_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let tasks = dir.path().join("tasks");
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::write(
            tasks.join("A-1.md"),
            "---\nid: A-1\ntitle: The blocker\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\n## Why\n\nx\n",
        )
        .unwrap();
        std::fs::write(
            tasks.join("A-2.md"),
            "---\nid: A-2\ntitle: Waiting on A-1\nphase: 1\nstatus: todo\neffort_days: 2\nblocked_by: [A-1]\n---\n\n## Why\n\nx\n",
        )
        .unwrap();
        dir
    }

    /// The surface is pinned so it cannot grow silently: every added tool is
    /// context an agent pays for on every call, so growth must be a reviewed
    /// decision rather than a side effect of a convenient refactor.
    #[test]
    fn the_tool_surface_is_exactly_these_four() {
        let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec!["aplan_ready", "aplan_why", "aplan_check", "aplan_close"],
            "the MCP tool surface changed; that is a deliberate decision, not a refactor"
        );
        for t in TOOLS {
            assert!(!t.description.is_empty(), "{} has no description", t.name);
        }
    }

    /// The headline guarantee of this crate. `mcp-shrimp-task-manager`'s
    /// `verify_task` accepts a completion score from the calling agent and
    /// auto-completes at 80 or above. Nothing here may resemble that.
    #[test]
    fn no_tool_accepts_a_self_assessed_score() {
        let source = include_str!("tools.rs");
        // The close path takes a task id and evidence citations. It has no
        // score, confidence, or completeness parameter, and this fails if one
        // is ever added.
        for forbidden in ["score", "confidence", "completeness", "percent"] {
            let in_signature = source
                .lines()
                .filter(|l| l.contains("pub fn close") || l.contains("fn parse_evidence"))
                .any(|l| l.to_ascii_lowercase().contains(forbidden));
            assert!(
                !in_signature,
                "`{forbidden}` appears in the close path: an agent must not grade its own work"
            );
        }
    }

    #[test]
    fn ready_reports_only_workable_tasks() {
        let dir = plan_dir();
        let out = ready(dir.path()).unwrap();
        let ids: Vec<&str> = out.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["A-1"], "A-2 is blocked by A-1");
    }

    #[test]
    fn why_explains_the_blockage_and_rejects_an_unknown_task() {
        let dir = plan_dir();
        let out = why(dir.path(), "A-2").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "A-1");
        assert_eq!(out[0].status, "todo");

        let err = why(dir.path(), "NOPE").unwrap_err();
        assert!(matches!(err, ToolError::Args(_)), "{err:?}");
    }

    #[test]
    fn check_returns_the_same_verdict_the_cli_would() {
        let dir = plan_dir();
        let out = check(dir.path(), &validate::Options::default()).unwrap();
        assert_eq!(out["tasks_checked"], 2);
        assert_eq!(out["errors"], 0);
    }

    #[test]
    fn close_refuses_without_evidence() {
        // The whole thesis in one assertion: MCP does not get to assert `done`.
        let dir = plan_dir();
        let err = close(dir.path(), "A-1", &[]).unwrap_err();
        assert!(matches!(err, ToolError::Args(_)), "{err:?}");
        assert!(err.to_string().contains("evidence"), "{err}");
    }

    #[test]
    fn close_refuses_a_note_exactly_as_the_cli_does() {
        let dir = plan_dir();
        let err = close(dir.path(), "A-1", &["note:trust me".to_string()]).unwrap_err();
        assert!(
            err.to_string().contains("captured"),
            "a note needs a date neither surface invents: {err}"
        );
    }

    #[test]
    fn close_refuses_an_unknown_task_and_a_malformed_citation() {
        let dir = plan_dir();
        let err = close(dir.path(), "GHOST", &["commit:abc123".to_string()]).unwrap_err();
        assert!(matches!(err, ToolError::Args(_)), "{err:?}");

        let err = close(dir.path(), "A-1", &["not-a-citation".to_string()]).unwrap_err();
        assert!(err.to_string().contains("kind:ref"), "{err}");

        let err = close(dir.path(), "A-1", &["telepathy:vibes".to_string()]).unwrap_err();
        assert!(err.to_string().contains("unknown evidence kind"), "{err}");
    }

    #[test]
    fn close_refuses_a_task_whose_blocker_is_unfinished() {
        // `aplan_core::close` owns this refusal; the point is that routing
        // through it means MCP inherits the refusal rather than reimplementing
        // (and eventually weakening) it.
        let dir = plan_dir();
        let err = close(dir.path(), "A-2", &["commit:abc123".to_string()]).unwrap_err();
        assert!(matches!(err, ToolError::Refused(_)), "{err:?}");
    }

    #[test]
    fn a_legitimate_close_writes_evidence_into_the_task_file() {
        let dir = plan_dir();
        let out = close(dir.path(), "A-1", &["commit:abc123".to_string()]).unwrap();
        assert_eq!(out["closed"], "A-1");

        let text = std::fs::read_to_string(dir.path().join("tasks/A-1.md")).unwrap();
        assert!(text.contains("status: done"), "{text}");
        assert!(text.contains("ref: abc123"), "{text}");
        assert!(text.contains("kind: commit"), "{text}");

        // And closing again is refused, because the task is already done.
        let err = close(dir.path(), "A-1", &["commit:def456".to_string()]).unwrap_err();
        assert!(matches!(err, ToolError::Refused(_)), "{err:?}");
    }
}
