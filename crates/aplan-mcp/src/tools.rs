//! The tool surface, as plain functions.
//!
//! Deliberately independent of the transport: every rule that matters here is a
//! property of what a tool *does*, not of how a request arrived, so the logic is
//! testable without standing up a server. The `rmcp` handler is a thin adapter
//! over these.
//!
//! The surface is **small on purpose** (`ready`, `why`, `check`, `verify`, `show`, `add`,
//! `dep`, `close`).
//! task-master ships ~36 tools costing ~21k tokens of context by default and had
//! to add an env var to trim them (RESEARCH section 6); an ETH Zurich study over
//! 138 repositories found context files raise inference cost by more than 20% and
//! can *lower* success. A small tool surface is a feature, not an unfinished one.
//!
//! **Changing what an existing tool *does* earns a `Behaviour`** (`A1-142`).
//!
//! Adding a tool is assertable the moment it ships: `facts` carries `mcp_tools`.
//! Widening one is not, and `aplan_close` stamping traces (`A1-100`) shipped
//! with `--require aplan_close` passing on the build that did not stamp. Name it
//! in `aplan_core::behaviour::ALL`, in the same commit.
//!
//! **`amend` is deliberately absent** (`A1-110`). Repairing a closed task's
//! evidence is the one operation where an agent could most cheaply turn a red
//! plan green: the gate stops it recording a citation that does not resolve, and
//! it does not stop it re-pointing a citation at a different test that happens to
//! pass. Deciding that a repair is a repair rather than a substitution is a
//! judgement, and this project's objection is to an agent grading its own work.
//! A person runs `aplan amend`. Recorded here rather than left to be noticed, so
//! `A1-104`'s audit is not quietly narrower than the CLI.

use std::path::{Path, PathBuf};

use aplan_core::{Graph, Status, load_plan, validate};
use serde::{Deserialize, Serialize};

/// The tools this server exposes, and the single place the surface is defined.
///
/// A test pins this list, so the surface cannot grow silently: an added tool is
/// a deliberate, reviewed change to what an agent can do, not a side effect.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "aplan_identity",
        description: "Name the build of this server: its version, the commit it was built from, \
                      and the digests of its capability document and its own image. Asking an \
                      installed CLI does not answer this -- a configured MCP process may be a \
                      different executable, which is how a pilot spent a pass on a defect the \
                      checkout had already fixed. Needs no plan and no repository.",
    },
    ToolSpec {
        name: "aplan_ready",
        description: "List tasks whose blockers are all done, so an agent can pick real work.",
    },
    ToolSpec {
        name: "aplan_why",
        description: "Explain what is blocking a task: its own blockers, and any unfinished \
                      cut, without expanding that cut's membership. Pass `all` for the \
                      complete transitive closure; every answer names its `traversal`.",
    },
    ToolSpec {
        name: "aplan_check",
        description: "Validate the plan and return its diagnostics, and `denied`: whether \
                      this plan fails the build under the repository's warning policy. \
                      Hermetic: no git, no network.",
    },
    ToolSpec {
        name: "aplan_verify",
        description: "Resolve the plan's evidence against the repository (L1/L2): a cited \
                      commit must exist and touch the task's `paths`, a cited file must \
                      exist. Opens git. Never runs tests.",
    },
    ToolSpec {
        name: "aplan_add",
        description: "Create a task: the next id in this plan's own convention (or its prefix \
                      and a ULID, where the repository set `id_strategy: ulid`), placed in a \
                      cut, whose `phase` it takes. Refuses before writing what `check` would reject after, by running \
                      the rules on the plan this would produce; what it would only warn about \
                      is named in `notes` instead.",
    },
    ToolSpec {
        name: "aplan_show",
        description: "One task whole: its `paths`, `traces` and `evidence`, which no other tool \
                      returns, plus what the graph resolves (its cut, whether it is ready, what \
                      it waits on and what it releases) and every finding attached to it. Reads \
                      the evidence ladder, so a closed task whose citation rotted looks different \
                      from a healthy one.",
    },
    ToolSpec {
        name: "aplan_dep",
        description: "Add or remove one dependency edge: `after` is the task that must finish \
                      first. `blocked_by` is the only ordering the model has and it was \
                      writable only at creation, so re-sequencing meant editing files by \
                      hand. Refuses before writing what `check` would reject after: a cycle, \
                      an id that does not exist, a cancelled blocker, or a done task \
                      acquiring an unfinished one. A milestone's `blocked_by` is which tasks \
                      are in that cut rather than what it comes after, so editing that is \
                      refused here and belongs to `aplan mv`.",
    },
    ToolSpec {
        name: "aplan_close",
        // Every clause is checked against behaviour by
        // `the_close_description_matches_what_close_does`, which fails for
        // under-promising as much as for over-promising. The second sentence read
        // "Recorded here, not resolved: run `aplan_verify`", which was true until
        // `A1-109` made close resolve before writing and false afterwards, so the
        // words told an agent to distrust a gate that had already run (`A1-131`).
        description: "Close a task with evidence, resolved before it is written: a cited \
                      commit must exist and touch the task's `paths`, a cited file must \
                      exist, and a cited test must be one a configured runner lists. \
                      Refuses exactly what the CLI refuses; evidence is a citation to be \
                      resolved, never a score the caller asserts. It never runs a test and \
                      never fetches a url, and whatever nothing here could check is named \
                      in `unchecked` rather than left looking like a pass. `aplan_verify` \
                      resolves the whole plan.",
    },
    ToolSpec {
        name: "aplan_claim",
        description: "Record that you are working on a task, so other agents sharing this \
                      repository can see it and `aplan_ready` can be filtered to what nobody \
                      holds. Two agents cannot both be told they hold one task: the write is \
                      a compare-and-swap, and the second is refused rather than overwriting \
                      the first. Taking a task somebody holds needs `from` naming them, which \
                      is checked. The name comes from `as`, else `APLAN_OWNER`, else the \
                      repository's git user, else the operating system's; it is never read \
                      from `.aplan.yml`, because several actors share one repository.",
    },
    ToolSpec {
        name: "aplan_release",
        description: "Drop a claim, reporting whose it was. Any caller may release, because a \
                      claim lasts until it is released or taken over and an abandoned one has \
                      to be clearable by somebody else; the record of who held it is in the \
                      diff. Releasing a task nobody holds changes nothing and says so.",
    },
    ToolSpec {
        name: "aplan_start",
        description: "Record that work on a task has begun, setting `in_progress`. A claim \
                      rather than a proof, and checked like one: `aplan_verify` reports \
                      `APLAN053` where the repository holds nothing matching the task's \
                      `paths`, so a status nobody acted on does not pass as progress. Refuses \
                      a task that is already terminal, because nothing reopens finished work.",
    },
    ToolSpec {
        name: "aplan_stop",
        description: "Record that work on a task has stopped, returning it to `todo`. Not a \
                      closure: it claims nothing about what was done, which is what \
                      `aplan_close` is for. Refuses a task that is not `in_progress`, since \
                      there is then nothing to stop.",
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

/// Write one task file, but only while it still holds what this tool read.
///
/// The agent surface had the defect `A1-226` measured, on the surface it
/// measured it on: `close` and `dep` called `std::fs::write`, so a concurrent
/// editor's change was discarded in silence, there was no lock, and an
/// interrupted write left a partial task file. Twenty-four agents was the
/// number in that measurement, and an agent reaches this tool through here
/// (`A1-270`).
///
/// The sentence comes from `aplan_core::save::advice`, which the commands also
/// use, so the same condition is described in the same words on both surfaces
/// (`A1-131`). What differs is the retry: an agent re-runs a *tool*, so that is
/// what it is told to run.
fn save_task(path: &Path, expected: &str, updated: &str, tool: &str) -> Result<(), ToolError> {
    aplan_core::save::replace(path, expected, updated)
        .map_err(|e| ToolError::Plan(aplan_core::save::advice(&e, tool)))
}

/// What `ready` answers: the work, and the cuts that could be closed.
#[derive(Debug, serde::Serialize)]
pub struct Ready {
    /// Work an agent can pick up now.
    pub ready: Vec<ReadyTask>,
    /// Milestones whose members are all done. Empty on an ordinary run; the
    /// reason `ready` is empty at a cut boundary.
    pub closeable_cuts: Vec<ReadyTask>,
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
    /// Unfinished tasks that become ready when this one is done, and whether it
    /// lies on the critical path: the two inputs to the order this list arrives
    /// in (`A1-125`, ADR-0017). Carried rather than left implicit because an
    /// agent that cannot interrogate the order can only take it on trust, which
    /// is the one thing this project asks nobody to do.
    #[serde(default)]
    pub releases: usize,
    /// The longest chain of live work waiting behind this one, over declared
    /// task-to-task edges (`A1-298`). The second term of the order, and the one
    /// that speaks when `releases` is silent -- which measured across twelve
    /// plans is 88% of ready tasks.
    #[serde(default)]
    pub downstream_depth: usize,
    #[serde(default)]
    pub critical: bool,
    /// Which of the *other ready* tasks this one prefers to follow.
    ///
    /// The third input to the order, and the only one that is a judgement
    /// somebody recorded rather than a fact derived from the plan (ADR-0025).
    /// Carried as a field rather than folded into a sentence, because a
    /// consumer that can read it can sort by it; `ready --explain` renders the
    /// same value as prose (`A1-182`).
    ///
    /// Restricted to tasks that are themselves ready: a preference to follow
    /// something that is not on offer explains nothing about this list.
    ///
    /// Always serialised, empty or not. A field that disappears when it has
    /// nothing in it hands a consumer `null` where it expected a list, and this
    /// is precisely the field the task wanted a consumer to be able to sort by.
    #[serde(default)]
    pub prefers_to_follow: Vec<String>,
    /// Who holds this task, when somebody does (`A1-359`).
    ///
    /// The coordination signal a fleet needs *before* any commit exists, and
    /// the reason it is a field here rather than a line the human renderer
    /// composes: an agent picking work has to be able to skip what another
    /// agent already holds, and it can only do that if the answer is in the
    /// payload it reads. `ready --unclaimed` filters on the same value, so the
    /// list and the filter cannot disagree (`A1-182`).
    ///
    /// Absent rather than empty when nobody holds it, because that is how the
    /// model spells unassigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_by: Option<String>,
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
    /// When this blocker is a cut a bounded walk stopped at: how many unmet
    /// tasks lie behind it, transitively.
    ///
    /// The cut's own remaining work, **not** a delta: summing this with
    /// `blocked_by`'s length does not give the size of the `all` answer, because
    /// a task can be a direct blocker *and* sit behind the folded cut. Every one
    /// of them does appear under `all`.
    ///
    /// Absent rather than zero, so its presence is the signal that something was
    /// folded and an ordinary task blocker never carries a number that means
    /// nothing (`A1-263`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind: Option<usize>,
}

/// Tasks that can be started now.
pub fn ready(root: &Path) -> Result<Ready, ToolError> {
    Ok(ready_of(&load_plan(root)))
}

/// The same answer, over a plan the caller already has.
///
/// Split out so the CLI's `--format json` is *this value* rather than a second
/// shape built beside it. `A1-182` found the surfaces inverted -- the MCP tools
/// returned fields while the CLI could only be regular-expressed -- and two
/// builders that must agree are two builders that eventually do not.
#[must_use]
pub fn ready_of(plan: &aplan_core::load::Plan) -> Ready {
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    // `closeable_cuts` reuses this, and a cut's own chain is not a number a
    // reader of that list is asking for: it is offered because its members are
    // done, not because of what waits behind it.
    let to_task = |t: &aplan_core::Task| ReadyTask {
        downstream_depth: 0,
        id: t.id.clone(),
        title: t.title.clone(),
        effort_days: t.effort_days,
        // Overridden for the ranked list below. A cut is not ranked work and is
        // never on the reported spine, but what closing it *releases* is the most
        // useful number at a cut boundary: it is the moment `ready` is empty and
        // the next cut's members are all waiting on this checkpoint (`A1-105`).
        // Zero here would have read as "closing this releases nothing".
        releases: 0,
        critical: false,
        prefers_to_follow: Vec::new(),
        // Set for every row, including a closeable cut: a cut a repository has
        // given an owner is a cut somebody is answerable for closing.
        held_by: t.owner.clone(),
    };
    let releases = graph.releases_all();
    let ranked = graph.ready_ranked();
    // A preference only explains this list if what it names is also on it.
    let offered: std::collections::BTreeSet<&str> =
        ranked.iter().map(|(t, _)| t.id.as_str()).collect();
    Ready {
        ready: ranked
            .iter()
            .map(|(t, rank)| ReadyTask {
                releases: rank.releases,
                downstream_depth: rank.downstream_depth,
                critical: rank.critical,
                prefers_to_follow: t
                    .prefer_after
                    .iter()
                    .filter(|p| offered.contains(p.as_str()))
                    .cloned()
                    .collect(),
                ..to_task(t)
            })
            .collect(),
        // Beside the list, never inside it: an agent filtering `ready` for work
        // must not find a checkpoint there. An empty `ready` is the answer to two
        // different questions, and at a cut boundary it is the one where every
        // task in the next cut waits on a milestone somebody could close
        // (`A1-105`).
        closeable_cuts: graph
            .closeable_milestones()
            .into_iter()
            .map(|m| ReadyTask {
                releases: releases.get(m.id.as_str()).copied().unwrap_or(0),
                ..to_task(m)
            })
            .collect(),
    }
}

/// Everything `why` knows about one task.
///
/// `blocked_by` alone was the whole answer, and the CLI's human output had three
/// facts beside it that no machine reader could get: a task can be *parked* by
/// its own status rather than by an edge, it can name a blocker that does not
/// exist, and a cut whose members are all done is waiting to be closed rather
/// than blocked. A machine surface less capable than the human one is the
/// inversion `A1-182` set out to remove, so they are carried here and both
/// surfaces read them (`A1-182`).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Blocked {
    /// Which traversal produced `blocked_by`: `cut-bounded` or `complete`.
    ///
    /// Stated on every answer, so a consumer can never read a folded list as
    /// the whole truth. A compaction that did not say it was one would be worse
    /// than the 86 lines it replaced (`A1-263`).
    pub traversal: String,
    /// The blockers, as far as `traversal` went. Named `blocked_by` because that
    /// is the field on a task, and the MCP response has always called it that.
    pub blocked_by: Vec<Blocker>,
    /// Ids this task names as blockers that no task in the plan defines.
    ///
    /// Always serialised, for the same reason `blocked_by` beside it is: one
    /// struct whose fields come and go is harder to read than one that does not.
    #[serde(default)]
    pub unknown: Vec<String>,
    /// The status says `blocked`: parked by a decision, not waiting on an edge.
    #[serde(default)]
    pub parked: bool,
    /// When a parked task names one, the instant it is worth looking again
    /// (`A1-264`).
    ///
    /// An agent reading `parked: true` learns the graph is not the answer and
    /// nothing else; the CLI's human rendering says when to come back, and this
    /// surface has to say the same thing or the two disagree about one task.
    /// It is *when to re-read*, never that the wait is over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revisit_at: Option<String>,
    /// A cut whose members are all done. Not blocked; waiting to be closed.
    #[serde(default)]
    pub closeable: bool,
}

/// What a task is waiting on: the cuts and tasks it is behind.
///
/// `all` asks for the complete transitive closure, which is what an audit
/// wants. Without it a cut is reported as itself and its membership is folded,
/// because an agent reading 86 lines of a roadmap it can query separately is
/// spending context on the wrong answer (`A1-263`).
pub fn why(root: &Path, id: &str, all: bool) -> Result<Blocked, ToolError> {
    why_of(&load_plan(root), id, walk_for(all))
}

/// Read the `all` argument off a tool call.
///
/// Refused rather than defaulted when it is not a boolean. The schema says
/// `boolean` and a well-behaved client sends one, but a silent fallback answers
/// a *different question* from the one asked: a caller that sent `"all": "true"`
/// would read a folded answer as the complete one, which is the single thing
/// this compaction must never allow.
///
/// Here rather than in the server's dispatch because that function is private,
/// so nothing can hold this case directly: `crates/aplan/tests/mcp.rs` drives
/// the dispatch over real stdio, which would catch a *wrong answer* and not a
/// silently-defaulted argument, and `mcp_parity.rs` calls the layer underneath
/// it. Unit-testable here (`A1-263`, audit).
///
/// # Errors
///
/// When `all` is present and is neither `true`, `false` nor null.
pub fn all_flag(value: Option<&serde_json::Value>) -> Result<bool, ToolError> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(false),
        Some(serde_json::Value::Bool(b)) => Ok(*b),
        Some(other) => Err(ToolError::Args(format!(
            "`all` must be true or false, not {other}: it decides whether the answer is the \
             complete closure, so guessing at it would answer a different question"
        ))),
    }
}

/// The traversal a caller's `all` asks for, in one place so the CLI and the
/// agent surface cannot disagree about what the flag means.
#[must_use]
pub fn walk_for(all: bool) -> aplan_core::graph::Walk {
    if all {
        aplan_core::graph::Walk::Complete
    } else {
        aplan_core::graph::Walk::CutBounded
    }
}

/// The same answer, over a plan the caller already has (`A1-182`).
pub fn why_of(
    plan: &aplan_core::load::Plan,
    id: &str,
    walk: aplan_core::graph::Walk,
) -> Result<Blocked, ToolError> {
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let Some(task) = graph.get(id) else {
        return Err(ToolError::Args(format!("no task `{id}` in this plan")));
    };
    Ok(Blocked {
        // Named, always: a consumer that cannot tell a bounded answer from a
        // complete one reads a fold as exhaustive truth, which is the one thing
        // a compaction must never let happen (`A1-263`).
        traversal: match walk {
            aplan_core::graph::Walk::CutBounded => "cut-bounded",
            aplan_core::graph::Walk::Complete => "complete",
        }
        .to_string(),
        blocked_by: graph
            .why_blocked(id, walk)
            .into_iter()
            .map(|t| Blocker {
                id: t.id.clone(),
                status: status_word(t.status).to_string(),
                title: t.title.clone(),
                // Only where something was folded, so the field's presence is
                // the signal and a task blocker never carries a misleading 0.
                behind: (walk == aplan_core::graph::Walk::CutBounded && t.is_milestone())
                    .then(|| graph.folded_behind(&t.id))
                    .filter(|n| *n > 0),
            })
            .collect(),
        unknown: graph
            .direct_unknown_blockers(id)
            .into_iter()
            .map(ToString::to_string)
            .collect(),
        parked: !task.status.is_workable() && !task.status.is_terminal(),
        revisit_at: task.revisit_at.clone(),
        closeable: graph.closeable_milestones().iter().any(|m| m.id == task.id),
    })
}

/// The plan's diagnostics, in the same shape `--format json` returns.
///
/// Plus `evidence_unresolved`, which `verify` carries in its own form: this one
/// counts what a hermetic run left, and `verify`'s counts what its configuration
/// gave it no resolver for. Saying `verify` simply "is the half that opens them"
/// was true of the command and not of every citation, which is `A1-253`.
///
/// The CLI says this in a note on stderr, and an agent has no stderr to read: it
/// reads a field, sees `errors: 0` and `denied: false`, and concludes the plan
/// passed -- which is `A1-101`'s defect exactly, one rule later. The fifth pilot
/// called gating on `check` alone "the single most likely misconfiguration,
/// because `check` alone looks like it is working", and an agent is the reader
/// least able to notice which command it ran.
///
/// Absent rather than zero when there is nothing to open, so a plan with no
/// resolvable citation reads as it always did, and the field's presence is
/// itself the signal.
pub fn check(root: &Path, policy: &crate::Policy) -> Result<serde_json::Value, ToolError> {
    let plan = load_plan(root);
    let report = validate(&plan, &policy.options);
    let mut payload = report_payload(&report, &policy.verdict);
    let left = aplan_core::unresolved_by_check(&plan);
    if left.any()
        && let Some(map) = payload.as_object_mut()
    {
        map.insert(
            "evidence_unresolved".to_string(),
            serde_json::json!({
                "citations": left.citations,
                "tasks": left.tasks,
                "resolved_by": "verify",
            }),
        );
    }
    Ok(payload)
}

/// One report, as both `check` and `verify` return it.
///
/// Shared so the two cannot describe the same plan differently, which is the
/// whole lesson of `A1-100` and `A1-101`: a second rendering of one fact drifts.
fn report_payload(report: &aplan_core::Report, policy: &aplan_core::Verdict) -> serde_json::Value {
    serde_json::json!({
        "tasks_checked": report.tasks_checked,
        // The blocking counts, so they agree with `denied` beside them and with
        // what the CLI prints for the same plan. The raw total is still derivable
        // from `diagnostics`, and `advisory` says what was heard (`A1-112`).
        "errors": report.blocking(policy, aplan_core::Severity::Error),
        "warnings": report.blocking(policy, aplan_core::Severity::Warning),
        // The verdict, not just the counts. `isError` stays "the tool ran", which
        // is the same distinction ADR-0007 draws between exit codes 1 and 2: a
        // violation is not a malfunction, and an agent that cannot tell them
        // apart retries the wrong one (`A1-101`).
        "denied": report.denied_by(policy),
        // Named, so an agent cannot read a passing verdict without seeing what
        // the repository chose to hear rather than stop for (`A1-112`).
        "advisory": report.advisory_count(policy),
        "diagnostics": report.diagnostics.iter().map(|d| serde_json::json!({
            "code": d.code,
            "severity": match d.severity {
                aplan_core::Severity::Error => "error",
                aplan_core::Severity::Warning => "warning",
            },
            "task": d.task,
            "file": d.file.as_ref().map(|f| f.display().to_string()),
            // The line the finding is about, so an agent editing the file does
            // not have to search for the field the message names (`A1-202`).
            // Null, never 1, when there is no single line: the two must look
            // different or a consumer cannot tell them apart.
            "line": d.location.map(|l| l.line),
            "column": d.location.map(|l| l.column),
            "message": d.message,
            "agent_instruction": d.agent_instruction,
            // The evidence a finding carries, as the array the CLI's JSON
            // gives (`A1-365`). An agent that cannot see why a cited test
            // failed is the actor this was built for: it has no terminal to
            // rerun the command in.
            "detail": d.detail,
        })).collect::<Vec<_>>(),
    })
}

/// Close a task, through the same gate the CLI uses.
///
/// This is the tool the whole task exists to get right. The anti-pattern it
/// refuses by example is `mcp-shrimp-task-manager`'s `verify_task`, which takes a
/// completion *score* from the calling agent and auto-completes at 80 or above:
/// an agent grading its own homework. There is no score parameter here and no
/// path that could accept one. The only thing a caller may supply is *evidence*,
/// a citation that something else resolves, and `aplan_core::close` applies the
/// identical refusals it applies to the CLI (no such task, still blocked, already
/// done, already carrying evidence). A `note` is refused earlier still, at parse
/// time, for the same reason the CLI refuses it: it needs a `captured` date that
/// neither surface will invent.
/// Resolve the plan's evidence against the repository (L1/L2).
///
/// `check` is hermetic by design, so the whole loop an agent could perform was
/// blind to unresolvable evidence: it could close a task citing a file that does
/// not exist, see `0 errors`, and hand CI a plan that fails on the citation it
/// just wrote. `close`'s own description calls evidence "a citation to be
/// resolved", so the surface promised the product's central claim and could not
/// perform it (`A1-102`, pre-public adoption evidence item 19).
///
/// A superset of `check`, exactly as the CLI's `verify` is (`A1-058`): the
/// hermetic rules run too, so an agent that runs this need not also run that, and
/// the two commands cannot disagree about whether a plan passes.
///
/// **Never executes.** `VerifyOptions::execute` stays off here whatever the
/// config says, because running a repository's tests because an agent asked is a
/// One task, whole (`A1-118`).
///
/// The decision `A1-104` forces to be explicit: this surface gets it. An agent
/// has `ready`, `why`, `check`, `verify`, `add`, `dep` and `close`, and none of
/// them returns a task's `paths`, `traces` or `evidence` -- so an agent about to
/// close a task cannot see the globs its commit citation will be judged against,
/// which is the one thing `close` will refuse it for. The alternative is reading
/// the file, and a file cannot say whether its own citations still resolve.
///
/// Resolves by default, like `close` (`A1-109`), and never executes a test, like
/// `verify` (`A1-102`): running a repository's tests because an agent asked is a
/// different trust question from reading its history.
pub fn show(root: &Path, id: &str, policy: &crate::Policy) -> Result<serde_json::Value, ToolError> {
    let plan = load_plan(root);
    // A task read out of a plan that did not load whole is a task whose blockers
    // and findings may be missing, and this surface exists to be trusted about
    // exactly those (`A1-102`).
    if !plan.load_diagnostics.is_empty() {
        return Err(ToolError::Plan(
            "the plan does not load cleanly; `aplan_check` says what failed".to_string(),
        ));
    }
    let mut report = validate(&plan, &policy.options);
    let opts = aplan_evidence::VerifyOptions {
        version: None,
        traces_root: None,
        config_file: None,
        execute: false,
        ..policy.verify.clone()
    };
    // A plan outside a repository is a real situation, and the answer is the one
    // `close` gives: say the ladder did not run rather than refuse to answer.
    let mut resolved = true;
    let one = aplan_core::show::only(&plan, id);
    match aplan_evidence::verify(&one, &policy.repo_root, &opts) {
        Ok(evidence) => report.diagnostics.extend(evidence.diagnostics),
        Err(aplan_evidence::Error::NotARepository(_) | aplan_evidence::Error::NoCommits(_)) => {
            resolved = false;
        }
        Err(e) => return Err(ToolError::Plan(format!("resolving evidence: {e}"))),
    }

    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let rows = graph.task_rows();
    let view = aplan_core::Show::assemble(&graph, &rows, &report.diagnostics, id, resolved, None)
        .ok_or_else(|| ToolError::Args(format!("no task `{id}` in this plan")))?;
    serde_json::to_value(&view).map_err(|e| ToolError::Plan(format!("rendering `{id}`: {e}")))
}

/// different trust question from reading its history.
pub fn verify(root: &Path, policy: &crate::Policy) -> Result<serde_json::Value, ToolError> {
    let plan = load_plan(root);
    let mut report = validate(&plan, &policy.options);
    // A plan that did not load whole gets the check report and nothing more, the
    // stance the CLI takes: "verify cannot speak honestly about a plan that did
    // not load". A task in the unreadable file may be the blocker, the owner of
    // the cited path, or the reason a citation looks stray, so resolving the rest
    // produces findings that read as complete and are not. Copying the happy path
    // without this guard is how the two surfaces began to disagree (`A1-102`).
    if !plan.unloadable.is_empty() {
        return Ok(report_payload(&report, &policy.verdict));
    }
    let opts = aplan_evidence::VerifyOptions {
        version: None,
        traces_root: None,
        config_file: None,
        execute: false,
        ..policy.verify.clone()
    };
    let evidence = aplan_evidence::verify(&plan, &policy.repo_root, &opts)
        .map_err(|e| ToolError::Plan(format!("resolving evidence: {e}")))?;
    report.diagnostics.extend(evidence.diagnostics);
    let mut payload = report_payload(&report, &policy.verdict);
    // The same field, for the same reader, one rung up (`A1-253`). A resolver
    // joins only when `.aplan.yml` declares it, so a `test` citation under no
    // runner is recorded and never opened. The CLI says so in a note on stderr
    // and an agent has no stderr: it reads `errors: 0`, sees no field, and
    // concludes the evidence resolved -- which is `A1-101`'s defect again, and
    // the reason `check` already carries the twin of this.
    //
    // Absent rather than zero when everything was opened, so a plan whose
    // configuration covers it reads exactly as it did.
    let left = aplan_evidence::unresolved_by_verify(&plan, &policy.repo_root, &opts);
    if left.any()
        && let Some(map) = payload.as_object_mut()
    {
        map.insert(
            "evidence_unresolved".to_string(),
            serde_json::json!({
                "citations": left.citations,
                "tasks": left.tasks,
                "unconfigured_kinds": left
                    .kinds
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>(),
            }),
        );
    }
    Ok(payload)
}

/// The one task an id names, refusing a plan where more than one declares it
/// (`A1-135`).
///
/// The same refusal the CLI makes, through the same predicate: a mutation that
/// cannot know which file it would edit must not guess, and the two surfaces
/// must not differ about it (`A1-104`).
fn locate<'p>(
    plan: &'p aplan_core::load::Plan,
    id: &str,
) -> Result<&'p aplan_core::load::LoadedTask, ToolError> {
    if let Some(why) = aplan_core::mutation::ambiguous_id(plan, id) {
        return Err(ToolError::Refused(why));
    }
    plan.tasks
        .iter()
        .find(|lt| lt.task.id == id)
        // The wording the CLI uses, because one question deserves one answer on
        // both surfaces (`A1-104`, `A1-135`).
        .ok_or_else(|| ToolError::Args(format!("no task `{id}` in this plan")))
}

/// The version to write into a release as it closes, and why it could not be
/// (`A1-339`, ADR-0038).
///
/// The command's decision, from the same place: one question deserves one answer
/// on both surfaces (`A1-104`), and two copies of this had the window's own
/// refusals missing from both -- a plan with two open releases recorded a number
/// the command refuses to give.
///
/// `Err` carries the reason rather than being swallowed, because an agent has no
/// stderr to read and a `null` with no explanation cannot be told from "this is
/// not a release" (`A1-173`).
fn recorded_version(
    plan: &aplan_core::load::Plan,
    task: &aplan_core::task::Task,
    policy: &crate::Policy,
) -> Option<Result<String, String>> {
    if !task.is_release() {
        return None;
    }
    let version = policy.verify.version.clone().unwrap_or_default();
    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = aplan_core::graph::Graph::new(tasks.iter().copied());
    Some(aplan_evidence::version::version_for_release(
        &policy.repo_root,
        plan,
        &graph,
        &version,
    ))
}

pub fn close(
    root: &Path,
    id: &str,
    evidence: &[String],
    policy: &crate::Policy,
) -> Result<serde_json::Value, ToolError> {
    let options = &policy.options;
    if evidence.is_empty() {
        return Err(ToolError::Args(
            "`evidence` is required: a status is a claim, and MCP does not get to make it \
             without one. Cite a commit, test, file, or url."
                .to_string(),
        ));
    }

    let plan = load_plan(root);

    // Never edit a task inside a plan that does not load: its neighbours may be
    // the reason this one should not close. The CLI takes the same position.
    if !plan.load_diagnostics.is_empty() {
        return Err(ToolError::Plan(
            "the plan does not load cleanly; fix it before closing anything".to_string(),
        ));
    }

    // Which task this is, before what it would be closed with: an id two files
    // declare is not a citation problem, and telling an agent to fix its
    // evidence would send it to the wrong half of the question (`A1-135`).
    let target = locate(&plan, id)?;
    let parsed = parse_evidence(evidence)?;

    let source = std::fs::read_to_string(&target.path)
        .map_err(|e| ToolError::Plan(format!("reading {}: {e}", target.path.display())))?;

    // Stamp exactly as the CLI does. The comment that used to stand here said
    // stamping was `close --restamp`'s job, which is false: an ordinary `close`
    // stamps, and `--restamp` re-stamps an already-closed task. What was true was
    // the second half, that this server did not read the traced documents, and
    // that was an implementation constraint rather than a principle: the stamping
    // helper was private to the `aplan` binary crate, so this surface could not
    // call it and closed without stamping at all. Every task an agent closed had
    // `APLAN018` silently off (`A1-100`).
    let fingerprints = if options.detect_drift {
        aplan_core::trace_fingerprints(&target.task, options.traces_root.as_deref())
    } else {
        std::collections::BTreeMap::new()
    };
    // Structural refusals first, exactly as the CLI orders them: an already-closed
    // task must be told that, not told its citation would fail `verify`.
    // **A release records the version it produced, on this surface too**
    // (`A1-339`). An agent has no stderr to read, so what was recorded -- and
    // `null` when nothing could be -- comes back in the payload instead of a
    // note (`A1-173`).
    let recorded = recorded_version(&plan, &target.task, policy);
    let number = recorded.as_ref().and_then(|r| r.as_ref().ok());
    let updated = aplan_core::close(&source, &parsed, &fingerprints, number.map(String::as_str))
        .map_err(|e| {
            // The same remedy the command gives, from the same place: an agent
            // cannot close through MCP what the command refuses, and that includes
            // being told the same way (`A1-131`, `A1-265`). The invocation named is
            // the command's, because `amend` has no tool -- which is the honest
            // answer rather than inventing one.
            ToolError::Refused(match aplan_core::close::advice(&e, id) {
                Some(advice) => format!("{e}. {advice}"),
                None => e.to_string(),
            })
        })?;

    // The same gate the CLI applies, through the same function, for the reason
    // this whole surface exists to respect: an agent must not be able to close
    // through MCP what the CLI refuses. `close` once certified `test` citations
    // `verify` rejected seconds later, and an agent grading its own work is the
    // anti-pattern this project is named against (`A1-109`).
    // The CLI's `close` deliberately reaches no network to record a citation: a
    // link down for an hour would block a closure, and `APLAN016` is a warning
    // precisely because it is not a statement about the plan (`A1-041`). This
    // surface passed the whole policy, whose `url_check` is `aplan_verify`'s, so
    // under `deny_warnings` an agent was refused a closure the command records.
    // "An agent cannot close through MCP what it could not close here" has to
    // hold in both directions (`A1-131`).
    let closing = aplan_evidence::VerifyOptions {
        version: None,
        traces_root: None,
        config_file: None,
        url_check: None,
        ..policy.verify.clone()
    };
    let resolution = aplan_evidence::resolve_proposed(
        &target.task,
        &target.path,
        &parsed,
        &policy.repo_root,
        &closing,
    )
    .map_err(|e| ToolError::Plan(format!("resolving the evidence: {e}")))?;
    if resolution.refuses(&policy.verdict) {
        return Err(ToolError::Refused(format!(
            "cannot close `{id}`: this evidence would fail `aplan verify`: {}",
            resolution
                .report
                .diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }

    // The same post-state check the command runs, or this surface keeps the old
    // behaviour and `A1-104`'s audit fails (`A1-113`).
    let produced = aplan_core::mutation::task_of(&updated)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    let after = aplan_core::mutation::with_replaced(&plan, id, produced);
    let delta = aplan_core::mutation::delta(&plan, &after, &policy.options);
    if delta.refuses(&policy.verdict) {
        return Err(ToolError::Refused(format!(
            "cannot close `{id}`: it would add {} finding(s) `check` reports: {}",
            delta.added.len(),
            delta.lines().join("; ")
        )));
    }

    save_task(&target.path, &source, &updated, "aplan_close")?;

    Ok(serde_json::json!({
        "closed": id,
        "file": target.path.display().to_string(),
        "evidence": evidence,
        // What a release recorded, `null` for everything else, and why when a
        // release could not record one: an agent has no stderr to read, and a
        // `null` with no explanation cannot be told from "this is not a
        // release" (`A1-339`, `A1-173`).
        "version": number,
        "version_not_recorded": recorded.as_ref().and_then(|r| r.as_ref().err()),
        // Named rather than implied: a caller cannot tell a citation that was
        // checked from one nothing could check, and silence reads as passed.
        "unchecked": resolution.unchecked_names(),
        // What the closure cost, when it did not cost enough to refuse it. The
        // same `delta.lines()` `add` and `dep` have always returned under this
        // name, and the same the command prints as `note: this adds N
        // finding(s)`. This tool computed them, refused on them, and then threw
        // the non-refusing ones away, so an agent closing a task over a warning
        // -- its own unticked acceptance criteria, since `A1-224` -- read a
        // success payload identical to a clean closure. An agent has no stderr
        // to read (`A1-173`), and this surface's rule is that it must not be
        // able to close through MCP what the CLI refuses; being *told* less
        // than the CLI tells is the same asymmetry one report short
        // (`A1-224`'s audit).
        "notes": delta.lines(),
    }))
}

/// Add or remove one `blocked_by` edge (`A1-124`).
///
/// The same shape as the command, through the same post-state check: an agent
/// that can create tasks can say which comes first, and cannot say anything
/// `check` would reject.
///
/// # Errors
/// [`ToolError::Args`] for an unknown id or a self-edge, [`ToolError::Refused`]
/// when the result would not pass, and [`ToolError::Plan`] for an unreadable or
/// unwritable file.
pub fn dep(
    root: &Path,
    id: &str,
    after: &str,
    add: bool,
    policy: &crate::Policy,
) -> Result<serde_json::Value, ToolError> {
    if id == after {
        return Err(ToolError::Args(format!("`{id}` cannot come after itself")));
    }
    let plan = load_plan(root);
    // Never edit ordering inside a plan that does not load: the task in the file
    // that failed is exactly the one that could make this edge a cycle, an
    // unknown id or a second cut, and `mutation::clone_of` documents that every
    // mutating command has refused before reaching it. `close` guards here and
    // the command guards there; this tool shipped without it (`A1-124`).
    if !plan.load_diagnostics.is_empty() {
        return Err(ToolError::Plan(
            "the plan does not load cleanly; fix it before changing what comes after what"
                .to_string(),
        ));
    }
    let target = locate(&plan, id)?;
    if let Some(why) = aplan_core::mutation::membership_edit(&plan, id, after, add) {
        return Err(ToolError::Refused(why));
    }

    let mut edges = target.task.blocked_by.clone();
    let present = edges.iter().any(|b| b == after);
    match (add, present) {
        (true, true) | (false, false) => {
            return Ok(serde_json::json!({ "id": id, "after": after, "changed": false }));
        }
        (true, false) => edges.push(after.to_string()),
        (false, true) => edges.retain(|b| b != after),
    }

    let original = std::fs::read_to_string(&target.path)
        .map_err(|e| ToolError::Plan(format!("reading {}: {e}", target.path.display())))?;
    let updated = aplan_core::close::set_blocked_by(&original, &edges)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    let produced = aplan_core::mutation::task_of(&updated)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;

    let after_plan = aplan_core::mutation::with_replaced(&plan, id, produced);
    let delta = aplan_core::mutation::delta(&plan, &after_plan, &policy.options);
    if delta.refuses(&policy.verdict) {
        return Err(ToolError::Refused(format!(
            "cannot change what `{id}` comes after: it would add {} finding(s) `check` reports: {}",
            delta.added.len(),
            delta.lines().join("; ")
        )));
    }

    save_task(&target.path, &original, &updated, "aplan_dep")?;
    Ok(serde_json::json!({
        "id": id,
        "after": after,
        "changed": true,
        "file": target.path.display().to_string(),
        "notes": delta.lines(),
    }))
}

/// `aplan_claim`: record who holds a task (`A1-359`).
///
/// **This tool is why `start` and `stop` exist at all.** `A1-077` refused a
/// status setter as "write surface bought with no invariant to maintain", and
/// that reasoning assumed an actor with a text editor. An agent reaching aplan
/// through this server has tools and no way to edit a file, so for it
/// "hand-editing is fine as it is" is simply false. The commands and the tools
/// therefore land together rather than CLI-first.
///
/// The mutual exclusion is the compare-and-swap in `save_task`, exactly as in
/// the CLI: two agents that both read "unassigned" cannot both be told they
/// hold it. The holder is read from the bytes about to be swapped rather than from
/// the plan snapshot, which is the race the CLI's own concurrency test caught.
pub fn claim(
    root: &Path,
    id: &str,
    as_name: Option<&str>,
    from: Option<&str>,
    policy: &crate::Policy,
) -> Result<serde_json::Value, ToolError> {
    let plan = load_plan(root);
    if !plan.load_diagnostics.is_empty() {
        return Err(ToolError::Plan(
            "the plan does not load cleanly; fix it before recording who holds what".to_string(),
        ));
    }
    let target = locate(&plan, id)?;
    let who =
        aplan_interop::identity::resolve(&aplan_interop::identity::from_environment(as_name, root))
            .map_err(|e| ToolError::Args(e.to_string()))?;

    let original = std::fs::read_to_string(&target.path)
        .map_err(|e| ToolError::Plan(format!("reading {}: {e}", target.path.display())))?;
    let current = aplan_core::mutation::task_of(&original)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    match (current.owner.as_deref(), from) {
        (Some(holder), None) => {
            return Err(ToolError::Refused(format!(
                "`{id}` is held by `{holder}`; pass `from` naming them to take it, or \
                 release it first"
            )));
        }
        (Some(holder), Some(named)) if holder != named => {
            return Err(ToolError::Refused(format!(
                "`{id}` is held by `{holder}`, not `{named}`; `from` is checked rather \
                 than taken on trust"
            )));
        }
        (None, Some(named)) => {
            return Err(ToolError::Args(format!(
                "`{id}` is held by nobody, so there is nothing to take it from `{named}`"
            )));
        }
        _ => {}
    }

    let updated = aplan_core::close::set_owner(&original, Some(&who))
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    let notes = refuse_if_worse(&plan, id, &updated, policy, &format!("claim `{id}`"))?;
    save_task(&target.path, &original, &updated, "aplan_claim")?;
    Ok(serde_json::json!({
        "id": id,
        "held_by": who,
        "taken_from": current.owner,
        "file": target.path.display().to_string(),
        "notes": notes,
    }))
}

/// `aplan_release`: drop a claim, naming whose it was (`A1-359`).
///
/// Any caller may release, for the reason the CLI does: a claim with no expiry
/// needs a way out that does not depend on the holder being reachable, and the
/// record of who held it is in the diff and in this answer.
pub fn release(
    root: &Path,
    id: &str,
    policy: &crate::Policy,
) -> Result<serde_json::Value, ToolError> {
    let plan = load_plan(root);
    if !plan.load_diagnostics.is_empty() {
        return Err(ToolError::Plan(
            "the plan does not load cleanly; fix it before recording who holds what".to_string(),
        ));
    }
    let target = locate(&plan, id)?;
    let original = std::fs::read_to_string(&target.path)
        .map_err(|e| ToolError::Plan(format!("reading {}: {e}", target.path.display())))?;
    let current = aplan_core::mutation::task_of(&original)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    let Some(holder) = current.owner.clone() else {
        // The outcome the caller asked for, reported rather than refused.
        return Ok(serde_json::json!({ "id": id, "changed": false, "held_by": null }));
    };

    let updated = aplan_core::close::set_owner(&original, None)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    let notes = refuse_if_worse(&plan, id, &updated, policy, &format!("release `{id}`"))?;
    save_task(&target.path, &original, &updated, "aplan_release")?;
    Ok(serde_json::json!({
        "id": id,
        "changed": true,
        "was_held_by": holder,
        "file": target.path.display().to_string(),
        "notes": notes,
    }))
}

/// `aplan_start` and `aplan_stop`: set and clear `in_progress` (`A1-359`).
///
/// A stored claim, and `APLAN053` is what keeps it honest: `verify` reports an
/// `in_progress` the repository holds nothing for. The premise is not "do not
/// store claims", it is "do not take claims on trust".
pub fn working(
    root: &Path,
    id: &str,
    starting: bool,
    policy: &crate::Policy,
) -> Result<serde_json::Value, ToolError> {
    let plan = load_plan(root);
    if !plan.load_diagnostics.is_empty() {
        return Err(ToolError::Plan(
            "the plan does not load cleanly; fix it before changing what is under way".to_string(),
        ));
    }
    let target = locate(&plan, id)?;
    let want = if starting {
        Status::InProgress
    } else {
        Status::Todo
    };
    let original = std::fs::read_to_string(&target.path)
        .map_err(|e| ToolError::Plan(format!("reading {}: {e}", target.path.display())))?;
    let current = aplan_core::mutation::task_of(&original)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    if current.status == want {
        return Ok(serde_json::json!({ "id": id, "status": want.to_string(), "changed": false }));
    }
    if current.status.is_terminal() {
        return Err(ToolError::Refused(format!(
            "`{id}` is `{}`, a terminal state, and nothing returns it to being worked",
            current.status
        )));
    }
    if !starting && current.status != Status::InProgress {
        return Err(ToolError::Refused(format!(
            "`{id}` is `{}`, not `in_progress`, so there is nothing to stop",
            current.status
        )));
    }

    let updated = aplan_core::close::set_status(&original, want)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    let verb = if starting { "start" } else { "stop" };
    let notes = refuse_if_worse(&plan, id, &updated, policy, &format!("{verb} `{id}`"))?;
    save_task(&target.path, &original, &updated, "aplan_working")?;
    Ok(serde_json::json!({
        "id": id,
        "status": want.to_string(),
        "changed": true,
        "held_by": current.owner,
        "file": target.path.display().to_string(),
        "notes": notes,
    }))
}

/// The post-state gate, as the four coordination tools need it (`A1-359`).
///
/// The same judgement every writing tool makes, in one place because four of
/// them make it identically: run the rules against the plan this edit would
/// produce and refuse one that adds a finding. It is not a formality here --
/// a `policy` scope may select on `owner`, so a claim can move a task into a
/// stricter evidence standard.
fn refuse_if_worse(
    plan: &aplan_core::load::Plan,
    id: &str,
    updated: &str,
    policy: &crate::Policy,
    what: &str,
) -> Result<Vec<String>, ToolError> {
    let produced = aplan_core::mutation::task_of(updated)
        .map_err(|e| ToolError::Plan(format!("`{id}`: {e}")))?;
    let after = aplan_core::mutation::with_replaced(plan, id, produced);
    let delta = aplan_core::mutation::delta(plan, &after, &policy.options);
    if delta.refuses(&policy.verdict) {
        return Err(ToolError::Refused(format!(
            "cannot {what}: it would add {} finding(s) `check` reports: {}",
            delta.added.len(),
            delta.lines().join("; ")
        )));
    }
    Ok(delta.lines())
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

/// The status an agent reads, spelled as the task file spells it.
///
/// Pinned by a test, because an agent branches on this string: `in_progress`
/// arriving as `inprogress` is a payload nobody can match on, and a silent
/// rename here is invisible to every test that only checks a diagnostic count.
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
/// The policy `aplan mcp` would build, for a test that only cares about a field
/// or two of it. At file scope so every test module reaches it through
/// `use super::*`.
fn test_policy(
    root: &std::path::Path,
    deny_warnings: bool,
    options: validate::Options,
) -> crate::Policy {
    crate::Policy {
        id_strategy: aplan_core::IdStrategy::Sequential,
        options,
        verdict: aplan_core::Verdict {
            deny_warnings,
            advisory: Vec::new(),
        },
        verify: aplan_evidence::VerifyOptions {
            version: None,
            traces_root: None,
            config_file: None,
            execute: false,
            test_runner: None,
            max_unrelated_files: None,
            url_check: None,
        },
        repo_root: root.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {

    /// `all` decides whether the answer is complete, so a value that is not a
    /// boolean is refused rather than read as `false` (`A1-263`, audit).
    ///
    /// The server's dispatch has no tests and its function is private, which is
    /// the wiring blind spot `mcp_parity.rs` names in its own header: the parse
    /// lives here so that something can hold it.
    #[test]
    fn a_non_boolean_all_is_refused_rather_than_read_as_false() {
        use serde_json::json;
        assert!(!all_flag(None).expect("absent is false"));
        assert!(!all_flag(Some(&json!(null))).expect("null is false"));
        assert!(all_flag(Some(&json!(true))).expect("true"));
        assert!(!all_flag(Some(&json!(false))).expect("false"));

        for bad in [json!("true"), json!(1), json!([]), json!({})] {
            let err = all_flag(Some(&bad)).expect_err("refused");
            let said = err.to_string();
            assert!(said.contains("must be true or false"), "{bad}: {said}");
            assert!(
                said.contains("different question"),
                "says why it matters rather than only what is wrong: {said}"
            );
        }
    }

    use super::*;

    /// A plan directory with one open task blocked by an unfinished one.
    fn plan_dir() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
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

    /// **The agent surface says what nothing was configured to check**
    /// (`A1-253`).
    ///
    /// `check` has carried `evidence_unresolved` since `A1-173`, for a reader
    /// with no stderr to read. `verify` did not, on the reasoning that it is the
    /// half that opens citations -- true of the command, and not of a citation
    /// whose resolver was never built. An agent gating on this payload sees
    /// `errors: 0` and concludes the evidence resolved.
    /// Proves the named behaviour `verify.reports_what_nothing_checked` (`A1-253`).
    #[test]
    fn verify_says_which_kinds_nothing_is_configured_to_check() {
        let dir = plan_dir();
        let root = dir.path();
        // `verify` opens the repository, so the fixture has to be one.
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            let out = std::process::Command::new("git")
                .args(&args)
                .current_dir(root)
                .output()
                .expect("git");
            assert!(out.status.success(), "git {args:?}");
        }
        std::fs::write(
            root.join("tasks/A-1.md"),
            "---\nid: A-1\ntitle: The blocker\nphase: 1\nstatus: done\neffort_days: 1\n\
             evidence:\n  - kind: test\n    ref: some_name\n---\n\n## Why\n\nx\n",
        )
        .unwrap();
        let policy = |runner: Option<aplan_evidence::TestRunners>| crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: runner,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: root.to_path_buf(),
        };

        let out = verify(root, &policy(None)).expect("verifies");
        assert_eq!(
            out["evidence_unresolved"]["citations"],
            serde_json::json!(1),
            "a `test` citation with no runner is not opened: {out}"
        );
        assert_eq!(
            out["evidence_unresolved"]["unconfigured_kinds"],
            serde_json::json!(["test"]),
            "and the payload names what to declare: {out}"
        );

        // Absent rather than zero once the configuration covers it, so a plan
        // that was always fine reads exactly as it did.
        let runner = aplan_evidence::TestRunners::One(aplan_evidence::TestRunner {
            output_lines: None,
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers: None,
            covers_by_test: None,
            name: None,
            dir: None,
            list: vec!["echo".into(), "some_name".into()],
            run: vec!["true".into(), "{test}".into()],
        });
        let covered = verify(root, &policy(Some(runner))).expect("verifies");
        assert!(
            covered.get("evidence_unresolved").is_none(),
            "nothing to say when everything was opened: {covered}"
        );
    }

    /// `aplan_dep` in each of its outcomes (`A1-124`).
    #[test]
    fn one_edge_is_added_removed_and_refused() {
        let dir = plan_dir();
        let root = dir.path();
        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: root.to_path_buf(),
        };

        // A-2 already comes after A-1 in this fixture, so adding it changes
        // nothing and says so rather than rewriting the file.
        let same = dep(root, "A-2", "A-1", true, &policy).expect("no-op");
        assert_eq!(same["changed"], serde_json::json!(false), "{same}");

        // Removing it does change something, and the file loads without it.
        let removed = dep(root, "A-2", "A-1", false, &policy).expect("removes");
        assert_eq!(removed["changed"], serde_json::json!(true), "{removed}");
        // The field, not the word: this task's *title* is "Waiting on A-1".
        let text = std::fs::read_to_string(root.join("tasks/A-2.md")).expect("read");
        assert!(!text.contains("blocked_by"), "{text}");

        // And adding it back restores it.
        let added = dep(root, "A-2", "A-1", true, &policy).expect("adds");
        assert_eq!(added["changed"], serde_json::json!(true), "{added}");
        let text = std::fs::read_to_string(root.join("tasks/A-2.md")).expect("read");
        assert!(text.contains("blocked_by:\n  - A-1"), "{text}");

        // A cycle is refused, by the shared post-state check rather than by a
        // private copy of `APLAN004` (`A1-113`).
        let err = dep(root, "A-1", "A-2", true, &policy).expect_err("must refuse");
        assert!(err.to_string().contains("APLAN004"), "{err}");

        // An id nobody has, and an edge to itself.
        assert!(dep(root, "NOPE", "A-1", true, &policy).is_err());
        assert!(dep(root, "A-1", "A-1", true, &policy).is_err());
    }

    /// The two things the tool shipped without, both of which the command had and
    /// both of which are what makes the surfaces one behaviour rather than two
    /// implementations of the same idea (`A1-124`).
    #[test]
    fn a_cut_and_a_broken_plan_are_both_refused_here_too() {
        let dir = plan_dir();
        let root = dir.path();
        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: root.to_path_buf(),
        };

        // A cut whose one member is `A-1`, sequenced after nothing.
        std::fs::write(
            root.join("tasks/M-1.md"),
            "---\nid: M-1\ntitle: A cut\nphase: 1\nstatus: todo\ntags: [milestone]\n\
             blocked_by: [A-1]\n---\n\n## Why\n\nx\n",
        )
        .expect("cut");
        let before = std::fs::read_to_string(root.join("tasks/M-1.md")).expect("read");

        for (id, after, add) in [("M-1", "A-2", true), ("M-1", "A-1", false)] {
            let err = dep(root, id, after, add, &policy).expect_err("membership is `mv`'s");
            assert!(err.to_string().contains("aplan mv"), "{err}");
        }
        assert_eq!(
            before,
            std::fs::read_to_string(root.join("tasks/M-1.md")).expect("read")
        );

        // A file that does not load: the tool must not edit ordering inside a
        // plan whose missing task could be the reason the edge is illegal.
        std::fs::write(root.join("tasks/A-3.md"), "---\nid: A-3\nnot: closed\n").expect("broken");
        let err = dep(root, "A-2", "A-1", false, &policy).expect_err("must refuse");
        assert!(err.to_string().contains("does not load cleanly"), "{err}");
        assert!(
            std::fs::read_to_string(root.join("tasks/A-2.md"))
                .expect("read")
                .contains("blocked_by"),
            "the edge was removed from a plan that did not load"
        );
    }

    /// The surface is pinned so it cannot grow silently: every added tool is
    /// context an agent pays for on every call, so growth must be a reviewed
    /// decision rather than a side effect of a convenient refactor.
    ///
    /// `aplan_identity` is the only one that answers about the *server* rather
    /// than about a plan: `p0000014` spent a pass diagnosing a defect its
    /// checkout had already fixed, because a configured MCP process may be a
    /// different executable from the CLI beside it and nothing could say so
    /// (`A1-259`).
    ///
    /// The last four are `A1-359`'s, and they are the growth this test is
    /// hardest on: four tools is a real cost on every call. They earn it
    /// because the agent they serve is the one that cannot do this another way.
    /// `A1-077` refused a status setter on the ground that hand-editing a
    /// status is fine, which is true of an actor with an editor and false of
    /// one reaching aplan through this server.
    #[test]
    fn the_tool_surface_is_exactly_these_thirteen() {
        let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "aplan_identity",
                "aplan_ready",
                "aplan_why",
                "aplan_check",
                "aplan_verify",
                "aplan_add",
                "aplan_show",
                "aplan_dep",
                "aplan_close",
                "aplan_claim",
                "aplan_release",
                "aplan_start",
                "aplan_stop",
            ],
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
        let ids: Vec<&str> = out.ready.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["A-1"], "A-2 is blocked by A-1");
    }

    /// The agent surface gets the same derived order the command does, with the
    /// two inputs that produced it, so an agent picking the first item can say
    /// why rather than taking the list on trust (`A1-125`).
    /// Proves the named behaviour `ready.order_is_derived` (`A1-211`).
    #[test]
    fn ready_is_ranked_here_too_and_carries_its_reason() {
        let dir = plan_dir();
        let root = dir.path();
        // `A-1` releases `A-2`; `C-1` releases nothing and is bigger. Id order
        // would put `A-1` first for the wrong reason, so `C-1` is written to
        // outrank it on effort and lose on what it releases.
        std::fs::write(
            root.join("tasks/C-1.md"),
            "---\nid: C-1\ntitle: Bigger, and releases nothing\nphase: 1\nstatus: todo\n\
             effort_days: 9\n---\n\n## Why\n\nx\n",
        )
        .expect("task");

        let out = ready(root).expect("ready");
        let ids: Vec<&str> = out.ready.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["A-1", "C-1"],
            "what it releases outranks the rest"
        );
        assert_eq!(out.ready[0].releases, 1);
        assert_eq!(out.ready[1].releases, 0);
        // At 9 days `C-1` is the whole critical path, and still second: what a
        // task releases is the input about the *plan*, so it outranks both of
        // the inputs that describe the task alone.
        assert!(out.ready[1].critical, "C-1 is the spine");
        assert!(!out.ready[0].critical);
    }

    /// The tool an agent needs before it acts on a task (`A1-118`).
    #[test]
    fn show_returns_the_fields_no_other_tool_does() {
        let dir = plan_dir();
        let root = dir.path();
        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: root.to_path_buf(),
        };

        let out = show(root, "A-2", &policy).expect("show");
        assert_eq!(out["id"], "A-2");
        // The edge the file states, and the one it does not.
        assert_eq!(out["waits_on"][0]["id"], "A-1");
        assert_eq!(out["waits_on"][0]["status"], "todo");
        assert_eq!(out["ready"], false);
        // Outside a repository the ladder cannot run, and the answer says so
        // rather than presenting an unresolved task as a clean one.
        assert_eq!(out["resolved"], false);
        assert!(out["paths"].is_array() && out["traces"].is_array());

        let err = show(root, "NOPE", &policy).expect_err("unknown id");
        assert!(matches!(err, ToolError::Args(_)), "{err:?}");
    }

    /// Inside a repository the tool resolves *this* task's citations, which is
    /// the half of the answer a file cannot give (`A1-118`).
    #[test]
    fn show_resolves_the_citations_of_the_task_it_was_asked_about() {
        let dir = plan_dir();
        let root = dir.path();
        for args in [
            vec!["init", "-q", "."],
            vec!["add", "-A"],
            vec![
                "-c",
                "user.email=a@b.c",
                "-c",
                "user.name=t",
                "commit",
                "-qm",
                "init",
            ],
        ] {
            std::process::Command::new("git")
                .args(&args)
                .current_dir(root)
                .output()
                .expect("git");
        }
        // A citation naming a commit this repository does not have, on `A-1`,
        // and nothing wrong with `A-2`.
        std::fs::write(
            root.join("tasks/A-1.md"),
            "---\nid: A-1\ntitle: The blocker\nphase: 1\nstatus: done\neffort_days: 1\n\
             paths:\n- tasks/**\nevidence:\n- kind: commit\n  \
             ref: 0000000000000000000000000000000000000000\n---\n\n## Why\n\nx\n",
        )
        .expect("task");

        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: root.to_path_buf(),
        };

        let out = show(root, "A-1", &policy).expect("show");
        assert_eq!(out["resolved"], true, "{out}");
        let codes: Vec<&str> = out["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .filter_map(|f| f["code"].as_str())
            .collect();
        assert!(codes.contains(&"APLAN011"), "{out}");

        // And the task beside it is clean, so the findings belong to the task
        // asked about rather than to the plan.
        let other = show(root, "A-2", &policy).expect("show");
        assert_eq!(
            other["findings"].as_array().map(Vec::len),
            Some(0),
            "{other}"
        );
    }

    /// The agent surface can record what caused the task it files (`A1-116`).
    ///
    /// `A1-103` gave an agent `add` so it could create the work it is told to
    /// create; without `traces` an agent asked to file a task for a finding could
    /// create the task and not record the finding, and its only route was to
    /// hand-edit YAML -- the anti-pattern this project names, reached from the
    /// surface that exists to keep the two honest.
    #[test]
    fn add_records_the_trace_that_caused_the_task_and_refuses_a_dead_one() {
        let dir = plan_dir();
        let root = dir.path();
        std::fs::write(
            root.join("DOC.md"),
            "# A document\n\n## The cause\n\nWhy.\n",
        )
        .expect("doc");
        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options {
                // The rules resolve a trace against this root, and the fixture's
                // document lives beside the plan rather than above it.
                traces_root: Some(root.to_path_buf()),
                ..aplan_core::validate::Options::default()
            },
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: root.to_path_buf(),
        };

        let out = add(
            root,
            &AddArgs {
                title: "traced".into(),
                traces: vec!["DOC.md#the-cause".into()],
                effort_days: Some(1.0),
                ..Default::default()
            },
            &policy,
        )
        .expect("add");
        let written = std::fs::read_to_string(root.join(format!(
            "tasks/{}.md",
            out["created"].as_str().expect("the new id")
        )))
        .expect("read");
        assert!(written.contains("traces:\n- DOC.md#the-cause"), "{written}");

        let err = add(
            root,
            &AddArgs {
                title: "dead".into(),
                traces: vec!["DOC.md#nope".into()],
                effort_days: Some(1.0),
                ..Default::default()
            },
            &policy,
        )
        .expect_err("an anchor that does not resolve is refused");
        assert!(err.to_string().contains("APLAN008"), "{err}");

        // An agent repeating itself gets one of each, the way the command does:
        // both surfaces call the same helper, and this is what says so
        // (`A1-116`).
        let out = add(
            root,
            &AddArgs {
                title: "repeated".into(),
                traces: vec!["DOC.md#the-cause".into(), "DOC.md#the-cause".into()],
                tags: vec!["cli".into(), "cli".into()],
                paths: vec!["src/**".into(), "src/**".into()],
                effort_days: Some(1.0),
                ..Default::default()
            },
            &policy,
        )
        .expect("add");
        let written = std::fs::read_to_string(root.join(format!(
            "tasks/{}.md",
            out["created"].as_str().expect("the new id")
        )))
        .expect("read");
        for value in ["DOC.md#the-cause", "cli", "src/**"] {
            assert_eq!(
                written.matches(&format!("- {value}")).count(),
                1,
                "`{value}` was written twice:\n{written}"
            );
        }
    }

    /// The phase comes from the cut here too, or the surfaces would disagree
    /// about the id an agent gets for the same call (`A1-108`, `A1-104`).
    #[test]
    fn add_takes_its_phase_from_the_cut_it_was_given() {
        let dir = plan_dir();
        let root = dir.path();
        // A later phase in use, and an earlier cut to add to.
        std::fs::write(
            root.join("tasks/A6-001.md"),
            "---\nid: A6-001\ntitle: Later work\nphase: 6\nstatus: todo\n\
             effort_days: 1\n---\n\n## Why\n\nx\n",
        )
        .expect("task");
        std::fs::write(
            root.join("tasks/M-s1.md"),
            "---\nid: M-s1\ntitle: Cut one\nphase: 1\nstatus: todo\ntags: [milestone]\n\
             blocked_by: [A-1]\n---\n\n## Why\n\nx\n",
        )
        .expect("cut");

        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: root.to_path_buf(),
        };
        let out = add(
            root,
            &AddArgs {
                title: "probe".into(),
                cut: Some("M-s1".into()),
                effort_days: Some(0.5),
                ..Default::default()
            },
            &policy,
        )
        .expect("add");
        let written = std::fs::read_to_string(root.join(format!(
            "tasks/{}.md",
            out["created"].as_str().expect("the new id")
        )))
        .expect("read");
        assert!(
            written.contains("phase: 1"),
            "the cut's phase, not the highest in use: {written}"
        );
    }

    /// A cut boundary, which no test on this surface covered: `ready` is empty,
    /// the closeable cut is beside it, and it says how much closing it frees.
    ///
    /// `A1-105` added `closeable_cuts` because an empty `ready` reads as a
    /// finished plan or a broken config. A cut carrying `releases: 0` would have
    /// re-created that in miniature: the number is exactly the reason to close it
    /// (`A1-125`).
    /// Proves the named behaviour `ready.reports_a_closeable_cut` (`A1-211`).
    #[test]
    fn a_closeable_cut_says_how_much_closing_it_would_free() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let tasks = dir.path().join("tasks");
        std::fs::create_dir_all(&tasks).expect("tasks");
        let write = |name: &str, body: &str| {
            std::fs::write(tasks.join(format!("{name}.md")), body).expect("write");
        };
        write(
            "A-1",
            "---\nid: A-1\ntitle: The cut's only work\nphase: 0\nstatus: done\n\
             effort_days: 1\nevidence:\n- kind: note\n  ref: done\n  captured: 2026-08-15\n\
             ---\n\n## Why\n\nx\n",
        );
        write(
            "M-s0",
            "---\nid: M-s0\ntitle: The first cut\nphase: 0\nstatus: todo\n\
             tags: [milestone]\nblocked_by: [A-1]\n---\n\n## Why\n\nx\n",
        );
        write(
            "M-s1",
            "---\nid: M-s1\ntitle: The second cut\nphase: 1\nstatus: todo\n\
             tags: [milestone]\nblocked_by: [B-1, B-2, M-s0]\n---\n\n## Why\n\nx\n",
        );
        for id in ["B-1", "B-2"] {
            write(
                id,
                &format!(
                    "---\nid: {id}\ntitle: Waiting on the cut before it\nphase: 1\n\
                     status: todo\neffort_days: 1\n---\n\n## Why\n\nx\n"
                ),
            );
        }

        let out = ready(dir.path()).expect("ready");
        assert!(
            out.ready.is_empty(),
            "the next cut's work waits on the checkpoint: {:?}",
            out.ready
        );
        let ids: Vec<&str> = out.closeable_cuts.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["M-s0"]);
        assert_eq!(
            out.closeable_cuts[0].releases, 2,
            "closing it frees both members of the next cut"
        );
        assert!(!out.closeable_cuts[0].critical, "a cut is not the spine");
    }

    #[test]
    fn why_explains_the_blockage_and_rejects_an_unknown_task() {
        let dir = plan_dir();
        let out = why(dir.path(), "A-2", false).unwrap();
        assert_eq!(out.blocked_by.len(), 1);
        assert_eq!(out.blocked_by[0].id, "A-1");
        assert_eq!(out.blocked_by[0].status, "todo");
        // The three facts that used to live only in the CLI's human output
        // (`A1-182`): this task is blocked by an edge, not parked, and is not a
        // cut waiting to be closed.
        assert!(out.unknown.is_empty());
        assert!(!out.parked);
        assert!(!out.closeable);

        let err = why(dir.path(), "NOPE", false).unwrap_err();
        assert!(matches!(err, ToolError::Args(_)), "{err:?}");
    }

    #[test]
    fn check_returns_the_same_verdict_the_cli_would() {
        let dir = plan_dir();
        let out = check(
            dir.path(),
            &test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap();
        assert_eq!(out["tasks_checked"], 2);
        assert_eq!(out["errors"], 0);
    }

    /// **A closure that costs a warning says so, as `add` and `dep` do.**
    ///
    /// `close` computed the post-state delta, refused on it, and threw the
    /// non-refusing findings away, so an agent closing a task over its own
    /// unticked acceptance criteria (`APLAN038`, `A1-224`) read a payload
    /// identical to a clean closure. The command has always printed them and
    /// the two sibling tools have always returned them under this name; this
    /// one is the surface where a status becomes a claim, and it was the one
    /// that said nothing. Found by `A1-224`'s audit.
    #[test]
    fn close_names_the_finding_it_did_not_refuse_on() {
        let dir = plan_dir();
        // A criterion the author never answered, on the task about to close.
        std::fs::write(
            dir.path().join("tasks/A-1.md"),
            "---\nid: A-1\ntitle: The blocker\nphase: 1\nstatus: todo\neffort_days: 1\n\
             ---\n\n## Acceptance criteria\n\n- [x] one\n- [ ] two\n",
        )
        .unwrap();
        let policy = super::test_policy(dir.path(), false, validate::Options::default());
        let out = close(
            dir.path(),
            "A-1",
            &["url:https://x.test/receipt".to_string()],
            &policy,
        )
        .unwrap();

        assert_eq!(out["closed"], "A-1", "it did close: {out}");
        let notes = out["notes"].as_array().expect("notes is a list").clone();
        assert!(
            notes.iter().any(|n| n
                .as_str()
                .is_some_and(|n| n.contains("APLAN038") && n.contains("1 of 2"))),
            "and named what the closure cost: {out}"
        );
    }

    /// The sibling that already did this, asserted, because nothing was.
    ///
    /// `notes` was returned by `add` and by `dep` and read by no test in this
    /// repository, so the field the fix above matches was itself a mechanism
    /// driving nothing (`A1-224`'s audit). `dep` builds it from the same
    /// `delta.lines()` expression and is not asserted here: every warning an
    /// edge can add is one `dep` refuses outright, since `membership_edit`
    /// rejects the milestone edits and an edge into unfinished work is an
    /// `APLAN007` error.
    #[test]
    fn add_names_its_findings_too() {
        let dir = plan_dir();
        let options = validate::Options {
            require_effort: true,
            ..validate::Options::default()
        };
        let policy = super::test_policy(dir.path(), false, options);
        // No `effort`, which `APLAN009` warns about and does not refuse.
        let out = add(
            dir.path(),
            &AddArgs {
                title: "no estimate".to_string(),
                ..Default::default()
            },
            &policy,
        )
        .unwrap();
        assert!(
            out["notes"]
                .as_array()
                .expect("notes is a list")
                .iter()
                .any(|n| n.as_str().is_some_and(|n| n.contains("APLAN009"))),
            "add says what it cost: {out}"
        );
    }

    /// **A locked task file is refused, on the agent surface too** (`A1-270`).
    ///
    /// `A1-226` measured the lost update *at twenty-four agents*, and an agent
    /// reaches this tool through here -- so until `A1-270` the fix covered the
    /// surface a person uses one command at a time and not the one twenty-four
    /// of them use at once. One case per writing tool, each asserting the
    /// refusal, that the file is byte-identical afterwards, and that the same
    /// call succeeds once the lock is gone.
    #[test]
    fn a_locked_task_file_is_refused_by_every_tool_that_writes_one() {
        for (name, run) in [
            (
                "aplan_close",
                &(|dir: &std::path::Path| {
                    close(
                        dir,
                        "A-1",
                        &["url:https://x.test/receipt".to_string()],
                        &super::test_policy(dir, false, validate::Options::default()),
                    )
                })
                    as &dyn Fn(&std::path::Path) -> Result<serde_json::Value, ToolError>,
            ),
            ("aplan_dep", &|dir: &std::path::Path| {
                dep(
                    dir,
                    "A-1",
                    "A-3",
                    true,
                    &super::test_policy(dir, false, validate::Options::default()),
                )
            }),
        ] {
            let d = plan_dir();
            // A third task, so `dep` has an edge to add that is neither a
            // duplicate nor a cycle: `A-2` already waits on `A-1`.
            std::fs::write(
                d.path().join("tasks/A-3.md"),
                "---\nid: A-3\ntitle: Independent\nphase: 1\nstatus: todo\n\
                 effort_days: 1\n---\n\n## Why\n\nx\n",
            )
            .unwrap();
            let target = d.path().join("tasks/A-1.md");
            let before = std::fs::read_to_string(&target).expect("read");

            // The lock `aplan_core::save` takes, held by somebody else.
            let lock = d.path().join("tasks/.A-1.md.lock");
            std::fs::write(&lock, "").expect("lock");

            let err = run(d.path()).expect_err(&format!("`{name}` was not refused"));
            assert!(
                err.to_string().contains("nothing was written"),
                "`{name}` must say the plan is untouched: {err}"
            );
            assert!(
                err.to_string().contains(name),
                "`{name}` must name the tool to run again, as the command names itself: {err}"
            );
            assert_eq!(
                std::fs::read_to_string(&target).expect("read"),
                before,
                "`{name}` changed the file it said it had not written"
            );

            std::fs::remove_file(&lock).expect("unlock");
            run(d.path()).unwrap_or_else(|e| panic!("`{name}` failed without the lock: {e}"));
            assert_ne!(
                std::fs::read_to_string(&target).expect("read"),
                before,
                "`{name}` reported success and wrote nothing"
            );
        }
    }

    /// **The lost update itself, on the agent surface** (`A1-270`'s audit).
    ///
    /// The test above holds the lock, which proves `aplan_close` reaches
    /// `aplan_core::save`. It does not prove the tool hands `save` the *right*
    /// `expected`: a call site passing the text it was about to write, or
    /// re-reading the file, would lock correctly and clobber anyway. That is the
    /// discarded-in-silence half of `A1-226`, and the first pass at this task
    /// reproduced on MCP exactly the gap `A1-246`'s audit had just closed on the
    /// commands.
    ///
    /// Deterministic rather than raced, the same way: `close` resolves its
    /// citation before writing, and resolving a `test` citation runs the
    /// configured runner's `list` command, so a runner that edits the task file
    /// puts a real third-party change between the read and the write every time.
    #[test]
    fn a_concurrent_edit_is_refused_rather_than_discarded() {
        let dir = plan_dir();
        let target = dir.path().join("tasks/A-1.md");
        // A repository, because `resolve_proposed` short-circuits every kind to
        // *unchecked* outside one -- deliberately, since `close` was hermetic
        // before `A1-109` and must not start demanding git. Without this the
        // runner below is never reached and the test passes for the wrong
        // reason.
        for args in [
            &["init", "-q", "."][..],
            &["config", "user.email", "t@example.com"],
            &["config", "user.name", "t"],
            &["add", "-A"],
            &["commit", "-q", "-m", "the plan"],
        ] {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("git");
            assert!(out.status.success(), "git {args:?}");
        }

        let tamper = dir.path().join("tamper.sh");
        std::fs::write(
            &tamper,
            format!(
                "#!/bin/sh\nprintf '\\n<!-- somebody else was here -->\\n' >> {}\n\
                 echo the-name\n",
                target.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&tamper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut policy = super::test_policy(dir.path(), false, validate::Options::default());
        policy.verify.test_runner = Some(aplan_evidence::TestRunners::One(
            aplan_evidence::TestRunner {
                output_lines: None,
                max_memory: None,
                max_processes: None,
                wrap: None,
                expect_tests: None,
                skipped: None,
                run_selects: None,
                timeout_seconds: None,
                covers: None,
                covers_by_test: None,
                name: None,
                dir: None,
                list: vec![tamper.display().to_string()],
                run: vec!["true".into(), "{test}".into()],
            },
        ));

        let err = close(dir.path(), "A-1", &["test:the-name".to_string()], &policy)
            .expect_err("the closure was not refused");
        assert!(
            err.to_string()
                .contains("changed while this was being prepared"),
            "and must say why, so an agent re-reads rather than retrying blind: {err}"
        );

        let after = std::fs::read_to_string(&target).expect("read");
        assert!(
            after.contains("somebody else was here"),
            "the other writer's change was discarded, which is the whole defect"
        );
        assert!(
            !after.contains("status: done"),
            "and the closure landed anyway: {after}"
        );
    }

    /// **An agent is told the same remedy the command gives** (`A1-265`).
    ///
    /// `A1-131`'s rule is that an agent cannot close through MCP what the
    /// command refuses, and being told *less* than the command tells is the
    /// same asymmetry one sentence short. The invocation named is the
    /// command's, because `amend` has no tool -- the honest answer rather than
    /// an invented one.
    #[test]
    fn a_refusal_with_a_remedy_names_it_to_an_agent_too() {
        let dir = plan_dir();
        let policy = super::test_policy(dir.path(), false, validate::Options::default());
        close(
            dir.path(),
            "A-1",
            &["url:https://x.test/receipt".to_string()],
            &policy,
        )
        .expect("closes");

        // The case an author actually reaches: another citation for a task
        // that is already `done`.
        let err = close(
            dir.path(),
            "A-1",
            &["url:https://x.test/second".to_string()],
            &policy,
        )
        .expect_err("closing twice was not refused");
        assert!(
            err.to_string().contains("aplan amend A-1 --add-evidence"),
            "an agent must be told what adds a citation, with the id filled in: {err}"
        );
        assert!(
            !err.to_string().contains("by hand"),
            "and never told to edit the file the tool owns: {err}"
        );
    }

    #[test]
    fn close_refuses_without_evidence() {
        // The whole thesis in one assertion: MCP does not get to assert `done`.
        let dir = plan_dir();
        let err = close(
            dir.path(),
            "A-1",
            &[],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::Args(_)), "{err:?}");
        assert!(err.to_string().contains("evidence"), "{err}");
    }

    #[test]
    fn close_refuses_a_note_exactly_as_the_cli_does() {
        let dir = plan_dir();
        let err = close(
            dir.path(),
            "A-1",
            &["note:trust me".to_string()],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("captured"),
            "a note needs a date neither surface invents: {err}"
        );
    }

    #[test]
    fn close_refuses_an_unknown_task_and_a_malformed_citation() {
        let dir = plan_dir();
        let err = close(
            dir.path(),
            "GHOST",
            &["commit:abc123".to_string()],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::Args(_)), "{err:?}");

        let err = close(
            dir.path(),
            "A-1",
            &["not-a-citation".to_string()],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("kind:ref"), "{err}");

        let err = close(
            dir.path(),
            "A-1",
            &["telepathy:vibes".to_string()],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown evidence kind"), "{err}");
    }

    #[test]
    fn close_refuses_a_task_whose_blocker_is_unfinished() {
        // `aplan_core::close` owns this refusal; the point is that routing
        // through it means MCP inherits the refusal rather than reimplementing
        // (and eventually weakening) it.
        let dir = plan_dir();
        let err = close(
            dir.path(),
            "A-2",
            &["commit:abc123".to_string()],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::Refused(_)), "{err:?}");
    }

    #[test]
    fn a_legitimate_close_writes_evidence_into_the_task_file() {
        let dir = plan_dir();
        let out = close(
            dir.path(),
            "A-1",
            &["commit:abc123".to_string()],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap();
        assert_eq!(out["closed"], "A-1");

        let text = std::fs::read_to_string(dir.path().join("tasks/A-1.md")).unwrap();
        assert!(text.contains("status: done"), "{text}");
        assert!(text.contains("ref: abc123"), "{text}");
        assert!(text.contains("kind: commit"), "{text}");

        // And closing again is refused, because the task is already done.
        let err = close(
            dir.path(),
            "A-1",
            &["commit:def456".to_string()],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::Refused(_)), "{err:?}");
    }
}

#[cfg(test)]
mod stamping {
    use super::*;

    /// A plan with one task tracing a document, in a fresh temp dir.
    fn plan(dir: &std::path::Path, trace: &str) {
        std::fs::create_dir_all(dir.join("tasks")).expect("mkdir");
        std::fs::write(
            dir.join("tasks/A-1.md"),
            format!(
                "---\nid: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 traces:\n- {trace}\n---\n\nbody\n"
            ),
        )
        .expect("task");
    }

    fn opts(root: &std::path::Path) -> validate::Options {
        validate::Options {
            detect_drift: true,
            traces_root: Some(root.to_path_buf()),
            ..validate::Options::default()
        }
    }

    #[test]
    fn closing_stamps_the_traces_it_can_read() {
        // `A1-100`. Before this, an agent's close wrote no stamps at all and
        // `APLAN018` was silently off for the task ever after.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("DOC.md"), "# Intro\n\nprose\n").expect("doc");
        plan(&dir.path().join("aplan"), "DOC.md#intro");

        close(
            &dir.path().join("aplan"),
            "A-1",
            &["file:DOC.md".into()],
            &super::test_policy(dir.path(), false, opts(dir.path())),
        )
        .expect("closes");

        let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
        assert!(
            written.contains("trace_fingerprints:") && written.contains("DOC.md#intro:"),
            "the trace must be stamped, or drift is off for this task: {written}"
        );
    }

    #[test]
    fn a_trace_it_cannot_read_is_left_unstamped_rather_than_guessed_at() {
        // The traced document is missing, which is `APLAN008`'s finding and not
        // something to invent a fingerprint for. Stamping it would record that a
        // section was reviewed at close when there was no section.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        plan(&dir.path().join("aplan"), "GONE.md#intro");

        close(
            &dir.path().join("aplan"),
            "A-1",
            &["file:GONE.md".into()],
            &super::test_policy(dir.path(), false, opts(dir.path())),
        )
        .expect("closes");

        let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
        assert!(
            !written.contains("GONE.md#intro:"),
            "an unreadable trace must not be stamped: {written}"
        );
    }

    #[test]
    fn drift_off_stamps_nothing() {
        // The setting is the CLI's, and this surface must honour it identically.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("DOC.md"), "# Intro\n\nprose\n").expect("doc");
        plan(&dir.path().join("aplan"), "DOC.md#intro");

        close(
            &dir.path().join("aplan"),
            "A-1",
            &["file:DOC.md".into()],
            &super::test_policy(dir.path(), false, validate::Options::default()),
        )
        .expect("closes");

        let written = std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("read");
        assert!(
            !written.contains("trace_fingerprints:"),
            "detect_drift is off: {written}"
        );
    }
}

#[cfg(test)]
mod verdict {
    use super::*;

    /// A plan with one warning and no errors: the case the policy decides.
    fn warning_only(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("tasks")).expect("mkdir");
        std::fs::write(
            dir.join("tasks/A-1.md"),
            "---\nid: A-1\ntitle: A-1\nphase: 0\nstatus: todo\n---\n\nbody\n",
        )
        .expect("task");
    }

    #[test]
    fn the_payload_carries_the_verdict_and_it_follows_the_policy() {
        // `A1-101`. Counts alone let an agent read a denied plan as clean, and
        // `isError` is the field an MCP client treats as "did this succeed".
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        warning_only(dir.path());
        let opts = validate::Options {
            require_effort: true,
            ..validate::Options::default()
        };

        let lenient =
            check(dir.path(), &test_policy(dir.path(), false, opts.clone())).expect("checks");
        assert_eq!(lenient["warnings"], 1, "the fixture must warn: {lenient}");
        assert_eq!(
            lenient["denied"], false,
            "a warning does not block unless the repository says so: {lenient}"
        );

        let strict =
            check(dir.path(), &test_policy(dir.path(), true, opts.clone())).expect("checks");
        assert_eq!(
            strict["denied"], true,
            "with `deny_warnings`, a warning is a finding: {strict}"
        );
        assert_eq!(
            strict["errors"], 0,
            "and it is still not an error, which is what `denied` exists to say"
        );
    }
}

/// What `aplan_add` was asked to create.
///
/// No `id`: it follows the plan's own convention, which is the reason `add` exists
/// rather than a template. No `out` or `dry_run` either: a directory override and
/// a preview are things a person wants at a terminal, and neither keeps a plan
/// valid.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddArgs {
    pub title: String,
    #[serde(default)]
    pub cut: Option<String>,
    #[serde(default)]
    pub effort_days: Option<f64>,
    #[serde(default)]
    pub paths: Vec<String>,
    /// Document sections this task answers to, `path/to/doc.md#anchor`. Refused
    /// when an anchor does not resolve, by the same post-state check the command
    /// runs (`A1-116`).
    #[serde(default)]
    pub traces: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

/// The refusals `add` can make from the arguments and the plan alone, in the
/// order the command makes them, and all of them before anything is written.
fn refuse_unusable_args(
    plan: &aplan_core::Plan,
    id: &str,
    args: &AddArgs,
) -> Result<(), ToolError> {
    if args.title.trim().is_empty() {
        return Err(ToolError::Args(
            "a task needs a title: it is how anyone finds this task again".into(),
        ));
    }
    if let Some(days) = args.effort_days
        && (!days.is_finite() || days < 0.0)
    {
        return Err(ToolError::Args(format!(
            "`effort_days: {days}` is not a number of days; give a finite, non-negative \
             estimate or omit it"
        )));
    }
    if let Some(existing) = plan.tasks.iter().find(|lt| lt.task.id == id) {
        return Err(ToolError::Plan(format!(
            "task `{id}` already exists in {}; refusing to overwrite it",
            existing.path.display()
        )));
    }
    let unknown: Vec<&str> = args
        .blocked_by
        .iter()
        .map(String::as_str)
        .filter(|b| !plan.tasks.iter().any(|lt| lt.task.id == *b))
        .collect();
    if !unknown.is_empty() {
        return Err(ToolError::Args(format!(
            "blocked by task(s) that do not exist: {}. `APLAN003` would reject the file, \
             so the id is checked before the file is written.",
            unknown.join(", ")
        )));
    }
    Ok(())
}

/// Create a task, the way the CLI's `add` creates one.
///
/// The host repository's instruction to its agents is "before you write code,
/// find or write its task", and step one is `aplan add --cut`, precisely because
/// `add` maintains the milestone's membership and a hand-written file will not
/// satisfy `require_cut_membership`. An agent could list ready work and close it
/// and could not create it, so the instruction was not performable through the
/// interface it was given (`A1-103`, pre-public adoption evidence item 20).
///
/// Every refusal the CLI makes is made here, in the same order, and all of them
/// *before* anything is written: a task the loader would reject, one nobody could
/// identify, an id that already exists, a blocker that does not, a cut that is not
/// a milestone. Writing first and validating after is how an author ends up with a
/// file `check` fails on and no obvious cause.
pub fn add(
    root: &Path,
    args: &AddArgs,
    policy: &crate::Policy,
) -> Result<serde_json::Value, ToolError> {
    use aplan_interop::{cut, emit, ids};

    let options = &policy.options;

    let plan = load_plan(root);
    // The same derivation the command uses (`A1-108`): a placement beats a guess.
    let phase = ids::phase_for(&plan, args.cut.as_deref());
    let id =
        ids::mint(&plan, phase, policy.id_strategy).map_err(|e| ToolError::Plan(e.to_string()))?;

    refuse_unusable_args(&plan, &id, args)?;
    // Id or label, the same vocabulary the CLI takes, resolved through the same
    // function so the two surfaces cannot answer differently (`A1-139`,
    // `A1-104`).
    let cut = match &args.cut {
        Some(name) => {
            Some(cut::resolve_id(&plan, name).map_err(|e| ToolError::Args(e.to_string()))?)
        }
        None => None,
    };
    if let Some(cut_id) = cut.as_deref() {
        cut::check_target(&plan, cut_id).map_err(|e| ToolError::Args(e.to_string()))?;
    }

    let task = aplan_core::Task {
        id: id.clone(),
        title: args.title.clone(),
        phase,
        status: aplan_core::Status::Todo,
        owner: args.owner.clone(),
        effort_days: args.effort_days,
        blocked_by: args.blocked_by.clone(),
        prefer_after: Vec::new(),
        ratified_by: Vec::new(),
        contains: Vec::new(),
        evidence: Vec::new(),
        // Repeats removed on both surfaces, in one place (`A1-116`).
        paths: emit::deduped(&args.paths),
        traces: emit::deduped(&args.traces),
        trace_fingerprints: std::collections::BTreeMap::new(),
        tags: emit::deduped(&args.tags),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
        version: None,
    };

    // The whole post-state, through the same shared path the command uses, or
    // this surface keeps the old behaviour and `A1-104`'s audit fails (`A1-113`).
    let mut after = aplan_core::mutation::with_added(
        &plan,
        std::path::PathBuf::from(format!("{id}.md")),
        task.clone(),
    );
    if let Some(cut_id) = cut.as_deref()
        && let Some(slot) = after.tasks.iter_mut().find(|lt| lt.task.id == cut_id)
    {
        slot.task.blocked_by.push(id.clone());
    }
    let delta = aplan_core::mutation::delta(&plan, &after, options);
    if delta.refuses(&policy.verdict) {
        return Err(ToolError::Refused(format!(
            "cannot add `{id}`: it would add {} finding(s) `check` reports: {}",
            delta.added.len(),
            delta.lines().join("; ")
        )));
    }

    let out_dir = {
        let conventional = root.join("tasks");
        if conventional.is_dir() {
            conventional
        } else {
            root.to_path_buf()
        }
    };
    // Created *and* placed, or nothing: a task written and left in no cut is a
    // plan `require_cut_membership` rejects, and both surfaces produced exactly
    // that when the milestone could not be written (`A1-103`).
    let placed_in = emit::write_task_placed(&out_dir, root, &task, emit::ADD_BODY, cut.as_deref())
        .map_err(|e| ToolError::Plan(e.to_string()))?
        .map(|p| p.display().to_string());

    // What was written that `check` will still reject. The command says this to a
    // human and the payload said nothing to an agent, so the surface that exists
    // for agents was the one that did not mention the plan it had just broken
    // (`A1-131`, and the shape `A1-101` names).
    let notes: Vec<String> = delta.lines();

    Ok(serde_json::json!({
        "created": id,
        "file": emit::task_path(&out_dir, &id).display().to_string(),
        "cut": cut,
        "milestone_file": placed_in,
        "notes": notes,
    }))
}

#[cfg(test)]
mod adding {
    use super::*;

    /// A plan with one cut and one member, in a fresh temp dir.
    fn plan(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("tasks")).expect("mkdir");
        std::fs::write(
            dir.join("tasks/M-s0.md"),
            "---\nid: M-s0\ntitle: Cut 0\nphase: 0\nstatus: todo\n\
             tags: [milestone, s0]\nblocked_by: [A-1]\n---\n\nbody\n",
        )
        .expect("milestone");
        std::fs::write(
            dir.join("tasks/A-1.md"),
            "---\nid: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
        )
        .expect("task");
    }

    /// The agent surface refuses a duplicate id exactly as the CLI does
    /// (`A1-135`, `A1-104`).
    ///
    /// A tool that guessed which of two files to edit would be the worse half of
    /// this defect: an agent cannot see which file it wrote to.
    #[test]
    fn a_duplicate_id_is_refused_on_this_surface_too() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path();
        plan(root);
        std::fs::write(
            root.join("tasks/dup.md"),
            "---\nid: A-1\ntitle: Declared twice\nphase: 0\nstatus: todo\n\
             effort_days: 1\n---\n\nbody\n",
        )
        .expect("dup");
        let before = std::fs::read_to_string(root.join("tasks/A-1.md")).expect("read");

        let err = super::close(root, "A-1", &["note:reviewed".to_string()], &plain_policy())
            .expect_err("a duplicate id must be refused");
        let text = err.to_string();
        assert!(
            text.contains("declared by 2 files") && text.contains("dup.md"),
            "{text}"
        );
        assert_eq!(
            before,
            std::fs::read_to_string(root.join("tasks/A-1.md")).expect("read"),
            "the refusal wrote to one of the two files"
        );
    }

    fn args(title: &str) -> AddArgs {
        AddArgs {
            title: title.to_string(),
            ..AddArgs::default()
        }
    }

    /// Every status an agent can be handed, spelled as the task file spells it.
    /// An agent branches on this string, and four of the five arms had never been
    /// rendered by a test (`A1-131`).
    #[test]
    fn every_status_reaches_an_agent_under_its_own_name() {
        assert_eq!(status_word(Status::Todo), "todo");
        assert_eq!(status_word(Status::InProgress), "in_progress");
        assert_eq!(status_word(Status::Blocked), "blocked");
        assert_eq!(status_word(Status::Done), "done");
        assert_eq!(status_word(Status::Dropped), "dropped");
        // And each is what a task file spells, so a round trip through an agent
        // cannot rename a status: the payload word is the frontmatter word.
        for (word, status) in [
            ("todo", Status::Todo),
            ("in_progress", Status::InProgress),
            ("blocked", Status::Blocked),
            ("done", Status::Done),
            ("dropped", Status::Dropped),
        ] {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("tempdir");
            std::fs::create_dir_all(dir.path().join("tasks")).expect("tasks");
            std::fs::write(
                dir.path().join("tasks/S-1.md"),
                format!(
                    "---\nid: S-1\ntitle: A task in each status\nphase: 1\nstatus: {word}\n\
                     owner: k\nevidence:\n  - kind: note\n    ref: r\n    captured: 2026-08-14\n\
                     ---\n\n## Why\n\nSomething.\n"
                ),
            )
            .expect("task");
            let plan = aplan_core::load_plan(dir.path());
            let loaded = plan.tasks.first().expect("loaded");
            assert_eq!(
                loaded.task.status, status,
                "`{word}` did not load as itself"
            );
            assert_eq!(status_word(loaded.task.status), word);
        }
    }

    /// The cardinality gate on this surface, where there was none. An agent
    /// could write through the tool exactly what `aplan add` refused, under a
    /// description promising it "refuses before writing what `check` would
    /// reject after" (`A1-133`).
    #[test]
    fn a_declared_cardinality_is_refused_here_as_it_is_on_the_command() {
        let opts_with =
            |families: &std::collections::BTreeMap<String, aplan_core::TagFamily>| crate::Policy {
                id_strategy: aplan_core::IdStrategy::Sequential,
                options: aplan_core::validate::Options {
                    tag_families: families.clone(),
                    ..aplan_core::validate::Options::default()
                },
                verdict: aplan_core::Verdict::default(),
                verify: aplan_evidence::VerifyOptions {
                    version: None,
                    traces_root: None,
                    config_file: None,
                    execute: false,
                    test_runner: None,
                    max_unrelated_files: None,
                    url_check: None,
                },
                repo_root: std::path::PathBuf::from("."),
            };
        let families: std::collections::BTreeMap<String, aplan_core::TagFamily> = [(
            "priority".to_string(),
            aplan_core::TagFamily {
                members: vec!["priority-p0".to_string(), "priority-p1".to_string()],
                require: aplan_core::Cardinality::ExactlyOne,
                status: aplan_core::StatusScope::Live,
                // A cut is not work, and this vocabulary classifies work.
                milestones: aplan_core::MilestoneScope::Exclude,
            },
        )]
        .into_iter()
        .collect();

        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        plan(dir.path());
        // The rest of the plan already satisfies the requirement, so the delta is
        // about the task being added rather than about the first priority tag
        // binding the vocabulary for everyone: adding one to an unbound family is
        // a real refusal, and a different case from this one (`A1-133`).
        std::fs::write(
            dir.path().join("tasks/A-1.md"),
            "---\nid: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
             tags: [priority-p0]\n---\n\nbody\n",
        )
        .expect("task");
        let before = std::fs::read_dir(dir.path()).expect("dir").count();

        // None, and two, are both refused, and the message names the family.
        for tags in [
            vec![],
            vec!["priority-p0".to_string(), "priority-p1".to_string()],
        ] {
            let err = add(
                dir.path(),
                &AddArgs {
                    tags,
                    effort_days: Some(1.0),
                    ..args("Wire the gate")
                },
                &opts_with(&families),
            )
            .expect_err("must refuse");
            // From the shared post-state check (`A1-113`), so it names the rule
            // and carries its remedy rather than being this command's own
            // sentence about one rule it happened to know.
            let text = err.to_string();
            assert!(text.contains("APLAN030 A-2:"), "{text}");
            assert!(text.contains("is declared exactly-one"), "{text}");
            assert!(
                text.contains("fix: Carry exactly one of `tag_families.priority`"),
                "{text}"
            );
        }
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("dir").count(),
            before,
            "a refused add left a file behind"
        );

        // And one goes through, or the refusals above would be a gate that
        // refuses everything.
        add(
            dir.path(),
            &AddArgs {
                tags: vec!["priority-p0".to_string()],
                effort_days: Some(1.0),
                ..args("Wire the gate")
            },
            &opts_with(&families),
        )
        .expect("creates");
    }

    /// A policy with nothing configured, for the cases that are about `add`
    /// rather than about a rule.
    fn plain_policy() -> crate::Policy {
        crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: std::path::PathBuf::from("."),
        }
    }

    #[test]
    fn a_task_lands_in_its_cut_with_the_next_id() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        plan(dir.path());
        let out = add(
            dir.path(),
            &AddArgs {
                cut: Some("M-s0".into()),
                effort_days: Some(2.0),
                ..args("Wire the gate")
            },
            &plain_policy(),
        )
        .expect("creates");

        assert_eq!(
            out["created"], "A-2",
            "the plan's own convention, not a uuid"
        );
        let milestone = std::fs::read_to_string(dir.path().join("tasks/M-s0.md")).expect("read");
        assert!(
            milestone.contains("A-2"),
            "the membership edit is why `add` is a command and not a template: {milestone}"
        );
    }

    /// The agent surface mints under the repository's strategy, not its own
    /// (`A1-047`).
    ///
    /// This is the case the key exists for. A repository sets `ulid` because two
    /// authors derive the same counter offline, and one of those authors is an
    /// agent: a tool that kept counting while the person's CLI stopped would be
    /// the collision, arriving through the surface nobody watches.
    #[test]
    fn the_agent_surface_mints_under_the_configured_strategy() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        plan(dir.path());
        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Ulid,
            ..plain_policy()
        };
        let out = add(dir.path(), &args("Wire the gate"), &policy).expect("creates");
        let id = out["created"].as_str().expect("an id").to_string();
        assert_ne!(id, "A-2", "the counter was continued under `ulid`");
        assert!(
            id.starts_with("A-") && id.len() == "A-".len() + 26,
            "the plan's stem, then a ULID: {id}"
        );

        // And a second one, because the failure that matters is two ids that
        // agree, not one id of the wrong shape.
        let again = add(dir.path(), &args("And another"), &policy).expect("creates");
        assert_ne!(again["created"].as_str().expect("an id"), id);
    }

    #[test]
    fn every_refusal_happens_before_anything_is_written() {
        // The order is the point. A task written and then rejected leaves the
        // author with a file `check` fails on and no obvious cause.
        let cases: Vec<(&str, AddArgs)> = vec![
            ("a task needs a title", args("   ")),
            (
                "not a number of days",
                AddArgs {
                    effort_days: Some(f64::INFINITY),
                    ..args("Infinite")
                },
            ),
            (
                "do not exist",
                AddArgs {
                    blocked_by: vec!["GHOST".into()],
                    ..args("Blocked by a ghost")
                },
            ),
            (
                "not tagged `milestone`",
                AddArgs {
                    cut: Some("A-1".into()),
                    ..args("Wrong cut")
                },
            ),
        ];
        for (expected, a) in cases {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("tempdir");
            plan(dir.path());
            let before: Vec<_> = std::fs::read_dir(dir.path().join("tasks"))
                .expect("dir")
                .filter_map(Result::ok)
                .map(|e| e.file_name())
                .collect();

            let err = add(dir.path(), &a, &plain_policy()).expect_err("must refuse");
            assert!(
                err.to_string().contains(expected),
                "expected a refusal mentioning {expected:?}, got: {err}"
            );

            let after: Vec<_> = std::fs::read_dir(dir.path().join("tasks"))
                .expect("dir")
                .filter_map(Result::ok)
                .map(|e| e.file_name())
                .collect();
            assert_eq!(
                before.len(),
                after.len(),
                "refused, and still wrote a file: {expected}"
            );
        }
    }
}

#[cfg(test)]
mod coordinating {
    use super::*;

    /// A plan with one free task, one held task and one closed task.
    fn plan(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("tasks")).expect("mkdir");
        for (id, status, extra) in [
            ("T-free", "todo", ""),
            ("T-spare", "todo", ""),
            ("T-held", "todo", "owner: ada\n"),
            // Parked. `stop` on a `todo` task reports "already there" rather
            // than refusing, so this is the only status that reaches the
            // "not `in_progress`" refusal.
            ("T-parked", "blocked", ""),
            (
                "T-done",
                "done",
                "evidence:\n  - kind: note\n    ref: shipped\n    captured: 2026-10-01\n",
            ),
        ] {
            std::fs::write(
                dir.join(format!("tasks/{id}.md")),
                format!(
                    "---\nid: {id}\ntitle: task {id}\nphase: 0\nstatus: {status}\n\
                     effort_days: 1\n{extra}---\n\nbody\n"
                ),
            )
            .expect("task");
        }
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        plan(dir.path());
        dir
    }

    fn policy() -> crate::Policy {
        crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: std::path::PathBuf::from("."),
        }
    }

    /// The owner line of a task, or `None`.
    fn owner_of(root: &std::path::Path, id: &str) -> Option<String> {
        let text = std::fs::read_to_string(root.join(format!("tasks/{id}.md"))).expect("read");
        text.lines()
            .find(|l| l.starts_with("owner:"))
            .map(|l| l.trim_start_matches("owner:").trim().to_string())
    }

    /// **Every refusal `claim` makes, on this surface** (`A1-359`).
    ///
    /// The CLI and this server must refuse the same things -- `mcp_parity`
    /// asserts they leave the same file -- and the refusals are where the two
    /// are most easily allowed to drift, because each one is a separate branch
    /// nothing forces a caller to exercise.
    #[test]
    fn claim_refuses_what_the_command_refuses() {
        let dir = fixture();
        let root = dir.path();

        let out = claim(root, "T-free", Some("ada"), None, &policy()).expect("a free task");
        assert_eq!(out["held_by"], "ada");
        assert_eq!(owner_of(root, "T-free").as_deref(), Some("ada"));

        // Held, and no `from`.
        let e = claim(root, "T-held", Some("grace"), None, &policy())
            .expect_err("a held task needs `from`");
        assert!(e.to_string().contains("held by `ada`"), "{e}");

        // Held, and the wrong `from`.
        let e = claim(root, "T-held", Some("grace"), Some("bob"), &policy())
            .expect_err("a wrong `from` must be refused");
        assert!(e.to_string().contains("not `bob`"), "{e}");
        assert_eq!(owner_of(root, "T-held").as_deref(), Some("ada"), "it moved");

        // `from` against a task nobody holds. `T-spare`, because `T-free` was
        // claimed above and is no longer free -- which is the whole point of
        // reading the holder from the file rather than from a snapshot.
        let e = claim(root, "T-spare", Some("x"), Some("nobody"), &policy())
            .expect_err("there is nothing to take it from");
        assert!(e.to_string().contains("held by nobody"), "{e}");

        // The named takeover works, and says what it displaced.
        let out = claim(root, "T-held", Some("grace"), Some("ada"), &policy()).expect("takeover");
        assert_eq!(out["taken_from"], "ada");
        assert_eq!(owner_of(root, "T-held").as_deref(), Some("grace"));

        // An id that is not there.
        assert!(claim(root, "GHOST", Some("x"), None, &policy()).is_err());
    }

    /// **`release` names whose claim it dropped, and says so when there was
    /// none** (`A1-359`).
    #[test]
    fn release_reports_the_claim_it_dropped() {
        let dir = fixture();
        let root = dir.path();

        let out = release(root, "T-held", &policy()).expect("a held task");
        assert_eq!(out["was_held_by"], "ada");
        assert_eq!(out["changed"], true);
        assert_eq!(owner_of(root, "T-held"), None, "the key was left behind");

        // Again: the outcome the caller asked for, reported rather than refused.
        let out = release(root, "T-held", &policy()).expect("already released");
        assert_eq!(out["changed"], false);
        assert!(out["held_by"].is_null());

        assert!(release(root, "GHOST", &policy()).is_err());
    }

    /// **`start` and `stop` move the status, and refuse what cannot move**
    /// (`A1-359`).
    #[test]
    fn starting_and_stopping_refuse_what_the_command_refuses() {
        let dir = fixture();
        let root = dir.path();

        let out = working(root, "T-free", true, &policy()).expect("start");
        assert_eq!(out["status"], "in_progress");
        assert_eq!(out["changed"], true);

        // Already there: reported, not refused, and nothing written.
        let out = working(root, "T-free", true, &policy()).expect("already started");
        assert_eq!(out["changed"], false);

        let out = working(root, "T-free", false, &policy()).expect("stop");
        assert_eq!(out["status"], "todo");

        // A `todo` task is already where `stop` would put it, so that is
        // reported rather than refused.
        let out = working(root, "T-held", false, &policy()).expect("already todo");
        assert_eq!(out["changed"], false);

        // Parked is the status that is neither, and the one the refusal is for.
        let e = working(root, "T-parked", false, &policy()).expect_err("nothing to stop");
        assert!(e.to_string().contains("not `in_progress`"), "{e}");

        // Terminal: nothing reopens finished work.
        let e = working(root, "T-done", true, &policy()).expect_err("a closed task");
        assert!(e.to_string().contains("terminal"), "{e}");

        assert!(working(root, "GHOST", true, &policy()).is_err());
    }

    /// **None of the four writes into a plan that does not load** (`A1-359`,
    /// and `A1-124`'s rule for this surface).
    ///
    /// The task in the file that failed is the one that could make the edit
    /// wrong, so every mutating tool refuses here. `aplan_dep` shipped without
    /// this guard once, which is why it is asserted for each of the four rather
    /// than for one of them.
    #[test]
    fn none_of_them_writes_into_a_plan_that_does_not_load() {
        let dir = fixture();
        let root = dir.path();
        std::fs::write(root.join("tasks/broken.md"), "---\nid: B\nnot: a task\n").expect("broken");
        let before = std::fs::read_to_string(root.join("tasks/T-free.md")).expect("read");

        for e in [
            claim(root, "T-free", Some("x"), None, &policy()).err(),
            release(root, "T-held", &policy()).err(),
            working(root, "T-free", true, &policy()).err(),
            working(root, "T-held", false, &policy()).err(),
        ] {
            let e = e.expect("a plan that does not load must refuse every write");
            assert!(e.to_string().contains("does not load cleanly"), "{e}");
        }
        assert_eq!(
            before,
            std::fs::read_to_string(root.join("tasks/T-free.md")).expect("read"),
            "a refusal wrote anyway"
        );
    }
}

#[cfg(test)]
mod coordination_gate {
    use super::*;

    /// **A claim can move a task into a stricter evidence standard, and the
    /// gate catches it** (`A1-359`).
    ///
    /// This is the reason `claim` runs the post-state rules at all, and the
    /// reason is easy to disbelieve: `owner` looks inert. It is not. A `policy`
    /// scope may select on `owners`, so naming a different holder can change
    /// which `require_min_strength` a closed task is held to -- and a task that
    /// closed cleanly on a note becomes `APLAN017` the moment it is claimed by
    /// somebody the stricter scope covers.
    ///
    /// Written as a test rather than left as an assertion in a comment, because
    /// a gate nobody has seen refuse is a gate nobody knows is wired up.
    #[test]
    fn claiming_into_a_stricter_policy_scope_is_refused() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("tasks")).expect("mkdir");
        // Closed on a note, which is fine under the whole-plan default.
        std::fs::write(
            root.join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: closed on a note\nphase: 0\nstatus: done\n\
             effort_days: 1\nevidence:\n  - kind: note\n    ref: agreed\n    \
             captured: 2026-10-01\n---\n\nbody\n",
        )
        .expect("task");

        // `ada`'s work must close on something a machine can check.
        let mut options = aplan_core::validate::Options::default();
        options
            .evidence_scopes
            .push(aplan_core::validate::EvidenceScope {
                selector: aplan_core::select::Selector {
                    owners: vec!["ada".to_string()],
                    ..aplan_core::select::Selector::default()
                },
                named_dimensions: 1,
                require_min_strength: Some(vec![aplan_core::EvidenceKind::Commit]),
                exempt_tags: None,
            });
        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options,
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: std::path::PathBuf::from("."),
        };

        let before = std::fs::read_to_string(root.join("tasks/T-1.md")).expect("read");
        let e = claim(root, "T-1", Some("ada"), None, &policy)
            .expect_err("the claim introduces a finding, so it must be refused");
        assert!(
            e.to_string().contains("APLAN017") || e.to_string().contains("finding"),
            "the refusal does not say what it would have added: {e}"
        );
        assert_eq!(
            before,
            std::fs::read_to_string(root.join("tasks/T-1.md")).expect("read"),
            "a refused claim wrote anyway"
        );

        // And somebody the scope does not cover may still claim it, which is
        // what keeps the gate a gate rather than a ban.
        claim(root, "T-1", Some("grace"), None, &policy).expect("an uncovered holder is fine");
    }

    /// **An `owner` the writer will not guess at is refused on this surface
    /// too** (`A1-359`).
    ///
    /// The one reachable failure of `set_owner`: a block-form value, which
    /// `rewrite_scalar_line` declines rather than mangle. The CLI reports it
    /// and so must this, or an agent would see a different outcome from the
    /// same plan.
    #[test]
    fn a_block_form_owner_is_refused_rather_than_mangled() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("tasks")).expect("mkdir");
        std::fs::write(
            root.join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: odd owner\nphase: 0\nstatus: todo\n\
             effort_days: 1\nowner: >\n  ada\n---\n\nbody\n",
        )
        .expect("task");
        let before = std::fs::read_to_string(root.join("tasks/T-1.md")).expect("read");

        let policy = crate::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                version: None,
                traces_root: None,
                config_file: None,
                execute: false,
                test_runner: None,
                max_unrelated_files: None,
                url_check: None,
            },
            repo_root: std::path::PathBuf::from("."),
        };
        // `release` is the path that reaches the writer: it reads the holder
        // only to report it, so nothing intervenes.
        let e = release(root, "T-1", &policy).expect_err("a value this cannot swap is refused");
        assert!(
            e.to_string().contains("change it by hand"),
            "the refusal does not say what to do: {e}"
        );

        // `claim` is refused earlier and for a different reason, which is worth
        // pinning rather than glossing: a block scalar keeps its newline, so the
        // holder reads as `ada\n` and `from: ada` genuinely does not name it.
        // The holder comparison is doing its job; it simply gets there first.
        let e = claim(root, "T-1", Some("grace"), Some("ada"), &policy)
            .expect_err("the named holder does not match");
        assert!(e.to_string().contains("not `ada`"), "{e}");
        assert_eq!(
            before,
            std::fs::read_to_string(root.join("tasks/T-1.md")).expect("read"),
            "a refusal rewrote the file"
        );
    }
}
