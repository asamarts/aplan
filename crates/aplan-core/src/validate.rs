//! The invariants. This is the whole point of the tool.
//!
//! A tracker that lets you write `status: done` and walk away is a tracker that
//! lies. Every rule here exists because a plan can assert something no one
//! checked.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::diagnostic::{Diagnostic, Report, code};
use crate::graph::Graph;
use crate::load::{LoadedTask, Plan};
use crate::task::{EvidenceKind, Status};

/// Options that soften rules for early-stage plans.
#[derive(Debug, Clone)]
pub struct Options {
    /// Require an `effort_days` estimate on non-terminal, non-milestone tasks.
    pub require_effort: bool,
    /// Root against which `traces` paths are resolved. `None` disables
    /// [`code::DANGLING_TRACE`] entirely.
    pub traces_root: Option<std::path::PathBuf>,
    /// Evidence kinds strong enough to close a task. `Some(kinds)` turns on
    /// [`code::WEAK_EVIDENCE`] (`APLAN017`): a `done` task must carry at least one
    /// evidence of one of these kinds. `None` disables the rule.
    pub require_min_strength: Option<Vec<EvidenceKind>>,
    /// Tags that exempt a task from [`code::WEAK_EVIDENCE`], for work where
    /// nothing stronger than a note exists (a phone call, a regulator letter).
    pub exempt_tags: Vec<String>,
    /// Turn on drift detection ([`code::TRACE_DRIFT`], `APLAN018`): compare a
    /// `done` task's stored `trace_fingerprints` against the live traced
    /// sections. Off by default; needs `traces_root` to read the documents.
    pub detect_drift: bool,
    /// Turn on membership totality ([`code::MEMBERSHIP_TOTALITY`], `APLAN022`):
    /// every non-milestone task must be a member of exactly one cut, unless
    /// tagged `cross-cutting`. Off by default.
    pub require_cut_membership: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            require_effort: true,
            traces_root: None,
            require_min_strength: None,
            exempt_tags: Vec::new(),
            detect_drift: false,
            require_cut_membership: false,
        }
    }
}

/// Run every invariant over a loaded plan.
pub fn validate(plan: &Plan, opts: &Options) -> Report {
    let mut report = Report {
        diagnostics: plan.load_diagnostics.clone(),
        tasks_checked: plan.tasks.len(),
    };

    // Duplicate ids. Detected before building the graph, which dedupes.
    let mut seen: BTreeMap<&str, &Path> = BTreeMap::new();
    for lt in &plan.tasks {
        if let Some(first) = seen.insert(lt.task.id.as_str(), &lt.path) {
            report.diagnostics.push(
                Diagnostic::error(
                    code::DUPLICATE_ID,
                    format!("duplicate task id, first defined in {}", first.display()),
                )
                .with_task(&lt.task.id)
                .with_file(&lt.path)
                .with_agent_instruction("Rename one of the two tasks to a unique id."),
            );
        }
    }

    let tasks: Vec<_> = plan.tasks.iter().map(|lt| &lt.task).collect();
    let graph = Graph::new(tasks.iter().copied());
    let file_of: BTreeMap<&str, &Path> = plan
        .tasks
        .iter()
        .map(|lt| (lt.task.id.as_str(), lt.path.as_path()))
        .collect();

    for lt in &plan.tasks {
        check_task(lt, &graph, opts, &mut report.diagnostics);
    }

    // Cycles. Reported once for the whole graph, not once per member.
    if let Some(cycle) = graph.find_cycle() {
        let path = cycle.join(" → ");
        let anchor = cycle.first().cloned().unwrap_or_default();
        let mut d = Diagnostic::error(code::CYCLE, format!("dependency cycle: {path}"))
            .with_task(&anchor)
            .with_agent_instruction(
                "Break the cycle by removing one `blocked_by` edge; a plan with a cycle has no \
                 valid execution order.",
            );
        if let Some(p) = file_of.get(anchor.as_str()) {
            d = d.with_file(*p);
        }
        report.diagnostics.push(d);
    }

    // APLAN022: membership totality. Every non-milestone task must be a member of
    // exactly one cut, in exactly one milestone's `blocked_by` (ADR-0008), unless
    // it is tagged `cross-cutting`. Opt-in, because a plan need not use cuts.
    if opts.require_cut_membership {
        let mut cut_count: BTreeMap<&str, usize> = BTreeMap::new();
        for t in &tasks {
            if t.is_milestone() {
                for b in &t.blocked_by {
                    *cut_count.entry(b.as_str()).or_default() += 1;
                }
            }
        }
        for lt in &plan.tasks {
            let t = &lt.task;
            if t.is_milestone() || t.tags.iter().any(|g| g == "cross-cutting") {
                continue;
            }
            let n = cut_count.get(t.id.as_str()).copied().unwrap_or(0);
            if n != 1 {
                let msg = if n == 0 {
                    "belongs to no cut".to_string()
                } else {
                    format!("belongs to {n} cuts (a task belongs to exactly one)")
                };
                report.diagnostics.push(
                    Diagnostic::error(code::MEMBERSHIP_TOTALITY, msg)
                        .with_task(&t.id)
                        .with_file(&lt.path)
                        .with_agent_instruction(
                            "Add the task to exactly one milestone's `blocked_by` (its cut), or \
                             tag it `cross-cutting` if it deliberately belongs to none.",
                        ),
                );
            }
        }
    }

    // Stable output: a *total* order, so two diagnostics that tie on
    // (code, task) can't reorder with the filesystem walk across CI runners.
    report.diagnostics.sort_by(|a, b| {
        a.code
            .cmp(b.code)
            .then(a.task.cmp(&b.task))
            .then(a.file.cmp(&b.file))
            .then(a.message.cmp(&b.message))
    });
    report
}

/// Per-task invariants.
///
/// Split out of [`validate`] so each function stays readable: the graph-wide
/// checks (duplicate ids, cycles) belong to the caller, which owns the whole
/// plan; everything here needs only one task plus the graph to look blockers up.
fn check_task(lt: &LoadedTask, graph: &Graph<'_>, opts: &Options, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;

    check_edges(lt, graph, out);
    check_evidence(lt, opts, out);

    // Effort, for phase rollups. Milestones are checkpoints, not work: they
    // carry no effort by design (and are excluded from every rollup), so asking
    // one for an estimate would be noise.
    if opts.require_effort
        && !t.status.is_terminal()
        && !t.is_milestone()
        && t.effort_days.is_none()
    {
        out.push(
            Diagnostic::warning(code::MISSING_EFFORT, "no `effort_days` estimate")
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction("Add an `effort_days` estimate, or drop the task."),
        );
    }

    // A task tagged `milestone` must be shaped like one (ADR-0008): it scopes
    // work via `blocked_by`, and carries no effort or paths, because a checkpoint
    // is not work and owns no files. The tag silently excludes it from every
    // work-view, so this warning is what makes a mis-tag, or an empty cut,
    // visible instead of a quiet reclassification.
    if t.is_milestone() {
        let mut problems = Vec::new();
        if t.blocked_by.is_empty() {
            problems.push("has no members (`blocked_by` is empty)");
        }
        if t.effort_days.is_some() {
            problems.push("carries `effort_days`");
        }
        if !t.paths.is_empty() {
            problems.push("declares `paths`");
        }
        if !problems.is_empty() {
            out.push(
                Diagnostic::warning(
                    code::MILESTONE_HYGIENE,
                    format!("is tagged `milestone` but {}", problems.join(" and ")),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(
                    "Remove the `milestone` tag if this is work, or make it a checkpoint: \
                     give it its cut's tasks as `blocked_by`, and drop `effort_days` and `paths`.",
                ),
            );
        }
    }

    // Traces must resolve. This is the link that stops a research document
    // and a plan from silently diverging.
    if let Some(root) = &opts.traces_root {
        // Deduplicate, exactly as `check_edges` does for `blocked_by`: a trace
        // listed twice is one relationship, and two identical diagnostics is just
        // noise (two identical annotations under `--format github`).
        let traces: BTreeSet<&str> = t.traces.iter().map(String::as_str).collect();
        for tr in traces {
            if let Some(diag) = check_trace(root, tr) {
                out.push(diag.with_task(&t.id).with_file(&lt.path));
            }
            // APLAN018: a closed task whose traced section has drifted from the
            // fingerprint it stamped at close time. Advisory, and only where a
            // stamp exists (no stamp is unverifiable, not a violation).
            if opts.detect_drift
                && t.status == Status::Done
                && let Some(stamp) = t.trace_fingerprints.get(tr)
                && let Some(diag) = check_drift(root, tr, stamp)
            {
                out.push(diag.with_task(&t.id).with_file(&lt.path));
            }
        }
    }
}

/// Compare a stored trace fingerprint against the live traced section. `None`
/// when they match, the document or anchor cannot be read (that is `APLAN008`'s
/// job, not a drift), or the trace names no anchor (only sections are stamped).
fn check_drift(root: &Path, trace: &str, stamp: &str) -> Option<Diagnostic> {
    let (rel, anchor) = trace.split_once('#')?;
    let text = std::fs::read_to_string(root.join(rel)).ok()?;
    let current = crate::anchor::anchor_fingerprint(&text, anchor)?;
    if current == *stamp {
        return None;
    }
    Some(
        Diagnostic::warning(
            code::TRACE_DRIFT,
            format!("traced section `{trace}` has changed since this task was closed"),
        )
        .with_agent_instruction(format!(
            "Re-review the task against `{rel}`; if it still holds, re-stamp with \
             `aplan close --restamp`, otherwise reopen the task."
        )),
    )
}

/// The invariants that read the `blocked_by` edges: `APLAN003`, `APLAN006`,
/// `APLAN007`, `APLAN020`.
fn check_edges(lt: &LoadedTask, graph: &Graph<'_>, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;

    // Self-dependency. A cycle of one, reported specifically because the
    // generic cycle message reads badly for it.
    if t.blocked_by.iter().any(|b| b == &t.id) {
        out.push(
            Diagnostic::error(code::SELF_DEP, "task lists itself in `blocked_by`")
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!("Remove `{}` from its own `blocked_by`.", t.id)),
        );
    }

    // Edges are deduplicated: `blocked_by: [GHOST, GHOST]` is one broken edge,
    // not two, and emitting the identical diagnostic twice is just noise (two
    // identical annotations, in `--format github`). Sorted, so the order does
    // not depend on how the author happened to type the list.
    let blockers: BTreeSet<&str> = t.blocked_by.iter().map(String::as_str).collect();

    for b in blockers {
        if b == t.id {
            continue; // already `SELF_DEP`
        }
        let Some(dep) = graph.get(b) else {
            out.push(
                Diagnostic::error(
                    code::UNKNOWN_DEP,
                    format!("`blocked_by` references unknown task `{b}`"),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either create task `{b}` or remove it from `{}`'s `blocked_by`.",
                    t.id
                )),
            );
            continue;
        };

        // A done task whose blocker is not done means one of the two is lying.
        if t.status == Status::Done && !dep.status.satisfies_dependency() {
            out.push(
                Diagnostic::error(
                    code::DONE_BEFORE_BLOCKER,
                    // Format the status via Display, not by lowercasing the whole
                    // message, that mangled the task id (`C-001` -> `c-001`),
                    // which is exactly the sort of thing this tool should notice.
                    format!("is `done` but blocker `{b}` is `{}`", dep.status),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either close `{b}` with evidence, or `{}` was not actually done.",
                    t.id
                )),
            );
        }

        // A live task blocked by a dropped one can never legitimately start: the
        // plan is deadlocked. `Dropped` failing to satisfy a dependency already
        // shaped `ready`/`why`, but `check`, the gate, stayed silent, so a
        // provably unworkable plan passed CI clean.
        //
        // Live only: a `done` dependent is `APLAN007` above (the two must not
        // double-report one edge), and a dropped task waiting on a dropped task
        // is just two abandoned things.
        if !t.status.is_terminal() && dep.status == Status::Dropped {
            out.push(
                Diagnostic::error(
                    code::DROPPED_BLOCKER,
                    format!(
                        "is `{}` but blocker `{b}` is `dropped`: it can never legitimately start",
                        t.status
                    ),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(format!(
                    "Either drop `{}` too, revive `{b}`, or remove `{b}` from its `blocked_by` \
                     and name a real blocker.",
                    t.id
                )),
            );
        }
    }
}

/// The invariants that read `evidence`: `APLAN005`, `APLAN010`.
fn check_evidence(lt: &LoadedTask, opts: &Options, out: &mut Vec<Diagnostic>) {
    let t = &lt.task;

    // Evidence with a blank `ref` is vacuous, a citation pointing at nothing, so
    // it must not count toward "done has proof". Otherwise `status: done` with
    // `ref: ""` sails through the hermetic gate: `verify` catches an empty
    // commit/file/test ref (it fails to resolve), but `url` and `note` have no
    // resolver, so `check` is the only rung that can object for those kinds.
    let has_substantive_evidence = t.evidence.iter().any(|e| !is_vacuous_ref(&e.reference));

    // The keystone: done requires substantive evidence.
    if t.status == Status::Done && !has_substantive_evidence {
        // Keep the no-evidence wording byte-for-byte (it is a pinned contract);
        // name the blank-`ref` case distinctly.
        let message = if t.evidence.is_empty() {
            "`status: done` with no `evidence`: a status is a claim, not a proof".to_string()
        } else {
            "`status: done` but every `evidence` entry has a blank `ref`: a status is a claim, \
             not a proof"
                .to_string()
        };
        out.push(
            Diagnostic::error(code::DONE_WITHOUT_EVIDENCE, message)
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(
                    "Add at least one evidence entry (commit, url, file, test, or a dated note), \
                     or move the task back to `in_progress`.",
                ),
        );
    }

    // An undated, or falsely dated, note is a rumour.
    for e in &t.evidence {
        if e.kind != EvidenceKind::Note {
            continue;
        }
        match &e.captured {
            None => out.push(
                Diagnostic::error(
                    code::UNDATED_NOTE,
                    "evidence of kind `note` requires a `captured` date",
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction("Add `captured: YYYY-MM-DD` to the note evidence entry."),
            ),
            // A present-but-nonsense date defeats the rule's purpose ("a note
            // carries a date") just as an absent one does.
            Some(d) if !is_iso_date(d) => out.push(
                Diagnostic::error(
                    code::UNDATED_NOTE,
                    format!("note evidence `captured: {d}` is not an ISO-8601 date (YYYY-MM-DD)"),
                )
                .with_task(&t.id)
                .with_file(&lt.path)
                .with_agent_instruction(
                    "Write the note's `captured` date as YYYY-MM-DD (for example 2026-07-18).",
                ),
            ),
            Some(_) => {}
        }
    }

    // Configured minimum evidence strength. A `done` task must carry at least one
    // piece of an accepted kind *with a real reference*, unless a tag exempts it.
    // Vacuous evidence is `APLAN005`'s business, so this only speaks when there is
    // substantive evidence but none of it is strong enough, which is the
    // shallow-`done` this rule exists to catch.
    if let Some(required) = &opts.require_min_strength
        && t.status == Status::Done
        && has_substantive_evidence
        && !t.tags.iter().any(|tag| opts.exempt_tags.contains(tag))
        && !t
            .evidence
            .iter()
            .any(|e| required.contains(&e.kind) && !is_vacuous_ref(&e.reference))
    {
        let kinds = required
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        out.push(
            Diagnostic::error(
                code::WEAK_EVIDENCE,
                format!("`done` but no evidence of a required kind (need one of: {kinds})"),
            )
            .with_task(&t.id)
            .with_file(&lt.path)
            .with_agent_instruction(format!(
                "Add evidence of a stronger kind ({kinds}), or exempt the task with a configured \
                 tag if nothing stronger exists."
            )),
        );
    }
}

/// Whether an evidence `ref` cites nothing: empty, or made up entirely of
/// whitespace, control, and zero-width / bidi format characters. `str::trim` only
/// strips `White_Space`, so a `ref` of a single U+200B (zero-width space) would
/// otherwise count as real proof, the same invisible class the house-style gate
/// forbids, letting `status: done` pass with no citation.
fn is_vacuous_ref(reference: &str) -> bool {
    reference
        .chars()
        .all(|c| c.is_whitespace() || c.is_control() || is_zero_width(c))
}

/// Invisible characters `char::is_whitespace` does not cover: the zero-width
/// spaces/joiners, the word joiner, the BOM, the soft hyphen, and the bidi marks.
fn is_zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{2060}'
            | '\u{FEFF}'
            | '\u{00AD}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{061C}'
    )
}

/// Whether `s` is an ISO-8601 calendar date (`YYYY-MM-DD`) with a real month and
/// day. Deliberately strict and dependency-free: this is a validation gate, not a
/// date library, so it accepts only the one unambiguous format the schema comment
/// and the docs ask for, and rejects impossible dates like `2026-13-40`.
///
/// Public because anything that *produces* a `captured` date must agree with the
/// rule that judges it. A command validating its own input against a private
/// re-implementation is how you get a date accepted at the boundary and then
/// rejected by `APLAN010` after the file is written.
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !(b[0..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    let year: i32 = s[0..4].parse().unwrap_or(0);
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day)
}

/// Verify that `trace` (`path/to/doc.md#anchor`) resolves to a real file, and,
/// when an anchor is given, that a heading slugifying to it exists.
fn check_trace(root: &Path, trace: &str) -> Option<Diagnostic> {
    let (rel, anchor) = match trace.split_once('#') {
        Some((r, a)) => (r, Some(a)),
        None => (trace, None),
    };
    let path = root.join(rel);
    if !path.is_file() {
        return Some(
            Diagnostic::error(
                code::DANGLING_TRACE,
                format!("`traces` target `{rel}` does not exist"),
            )
            .with_agent_instruction(format!("Create `{rel}` or correct the trace path.")),
        );
    }
    let anchor = anchor?;
    let Ok(text) = std::fs::read_to_string(&path) else {
        return None;
    };
    // GitHub assigns each heading an anchor by slugifying it and disambiguating
    // repeats with a `-N` suffix. Reproduce that set exactly, then test
    // membership against the anchor a human copied from the rendered document's
    // URL. GitHub ids are always lowercase, so fold the anchor's case too.
    if crate::anchor::heading_anchors(&text).contains(anchor.to_lowercase().as_str()) {
        None
    } else {
        Some(
            Diagnostic::error(
                code::DANGLING_TRACE,
                format!("`traces` anchor `#{anchor}` not found as a heading in `{rel}`"),
            )
            .with_agent_instruction(format!(
                "Add a heading matching `{anchor}` to `{rel}`, or correct the anchor."
            )),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::LoadedTask;
    use crate::task::{Evidence, Task};
    use std::path::PathBuf;

    fn t(id: &str, status: Status) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
        }
    }

    fn plan_of(tasks: Vec<Task>) -> Plan {
        Plan {
            tasks: tasks
                .into_iter()
                .map(|task| LoadedTask {
                    task,
                    path: PathBuf::from("plan/tasks/x.md"),
                })
                .collect(),
            load_diagnostics: Vec::new(),
        }
    }

    fn opts() -> Options {
        Options {
            require_effort: true,
            traces_root: None,
            ..Default::default()
        }
    }

    #[test]
    fn done_without_evidence_is_an_error() {
        let p = plan_of(vec![t("A", Status::Done)]);
        let r = validate(&p, &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE)
        );
        assert!(!r.is_clean());
    }

    #[test]
    fn done_with_evidence_is_clean() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "d213c9a".into(),
            captured: None,
        }];
        let r = validate(&plan_of(vec![task]), &opts());
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    #[test]
    fn undated_note_is_an_error() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![Evidence {
            kind: EvidenceKind::Note,
            reference: "spoke to the Division".into(),
            captured: None,
        }];
        let r = validate(&plan_of(vec![task]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::UNDATED_NOTE));
    }

    /// APLAN017: the configured minimum evidence strength.
    fn note_ev() -> Evidence {
        Evidence {
            kind: EvidenceKind::Note,
            reference: "phone call".into(),
            captured: Some("2026-07-01".into()),
        }
    }

    fn commit_ev() -> Evidence {
        Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }
    }

    fn strength(kinds: &[EvidenceKind], exempt: &[&str]) -> Options {
        Options {
            require_min_strength: Some(kinds.to_vec()),
            exempt_tags: exempt.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn weak_evidence_fires_when_only_a_note_backs_a_done_task() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![note_ev()];
        let o = strength(
            &[EvidenceKind::Commit, EvidenceKind::Test, EvidenceKind::File],
            &[],
        );
        let r = validate(&plan_of(vec![task]), &o);
        assert!(r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE));
    }

    #[test]
    fn weak_evidence_is_silent_with_one_strong_piece() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![note_ev(), commit_ev()]; // one weak, one strong
        let o = strength(
            &[EvidenceKind::Commit, EvidenceKind::Test, EvidenceKind::File],
            &[],
        );
        let r = validate(&plan_of(vec![task]), &o);
        assert!(!r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE));
    }

    #[test]
    fn weak_evidence_exempts_a_tagged_task() {
        let mut task = t("A", Status::Done);
        task.tags = vec!["phone".into()];
        task.evidence = vec![note_ev()];
        let o = strength(&[EvidenceKind::Commit], &["phone"]);
        let r = validate(&plan_of(vec![task]), &o);
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE),
            "an exempt tag must silence APLAN017: {:?}",
            r.diagnostics
        );
    }

    #[test]
    fn weak_evidence_is_off_by_default_and_never_doubles_with_005() {
        // Off by default: a note-only `done` task is fine.
        let mut note_only = t("A", Status::Done);
        note_only.evidence = vec![note_ev()];
        let r = validate(&plan_of(vec![note_only]), &opts());
        assert!(!r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE));

        // With the rule on, empty evidence is APLAN005's business, not 017's: a
        // `done` task with no evidence must not fire both.
        let empty = t("B", Status::Done);
        let o = strength(&[EvidenceKind::Commit], &[]);
        let r = validate(&plan_of(vec![empty]), &o);
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE)
        );
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE),
            "empty evidence must not also fire APLAN017"
        );
    }

    /// A `done` task whose only evidence has a blank `ref` is not proven: the
    /// keystone must fire, or `status: done` plus `ref: ""` sails through `check`.
    #[test]
    fn done_with_only_blank_ref_evidence_is_an_error() {
        for blank in ["", "   ", "\t"] {
            let mut task = t("A", Status::Done);
            task.evidence = vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: blank.to_string(),
                captured: None,
            }];
            let r = validate(&plan_of(vec![task]), &opts());
            assert!(
                r.diagnostics
                    .iter()
                    .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE),
                "a blank ref ({blank:?}) must not satisfy `done` needs evidence"
            );
        }
    }

    /// A `ref` made only of invisible characters (a zero-width space) cites
    /// nothing, so it must not satisfy the keystone. `str::trim` alone misses it.
    #[test]
    fn done_with_only_an_invisible_ref_is_an_error() {
        for invisible in [
            "\u{200B}",
            "\u{FEFF}",
            "\u{200B}\u{200D}\u{2060}",
            " \u{200B} ",
        ] {
            let mut task = t("A", Status::Done);
            task.evidence = vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: invisible.to_string(),
                captured: None,
            }];
            let r = validate(&plan_of(vec![task]), &opts());
            assert!(
                r.diagnostics
                    .iter()
                    .any(|d| d.code == code::DONE_WITHOUT_EVIDENCE),
                "an invisible ref ({invisible:?}) must not satisfy `done` needs evidence"
            );
        }
        // A real ref with an incidental invisible char is still substantive.
        let mut ok = t("B", Status::Done);
        ok.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }];
        assert!(validate(&plan_of(vec![ok]), &opts()).is_clean());
    }

    /// A blank-`ref` note of an accepted kind must not satisfy APLAN017 either:
    /// strength is about a real citation, not just the right `kind`.
    #[test]
    fn weak_evidence_ignores_a_blank_ref_of_an_accepted_kind() {
        let mut task = t("A", Status::Done);
        task.evidence = vec![
            note_ev(), // substantive, so APLAN005 stays silent
            Evidence {
                kind: EvidenceKind::Commit,
                reference: "  ".into(),
                captured: None,
            },
        ];
        let o = strength(&[EvidenceKind::Commit], &[]);
        let r = validate(&plan_of(vec![task]), &o);
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::WEAK_EVIDENCE),
            "a blank-ref commit must not count as strong evidence: {:?}",
            r.diagnostics
        );
    }

    #[test]
    fn a_note_with_a_nonsense_captured_date_is_an_error() {
        for bad in [
            "someday",
            "2026-13-40",
            "2026-02-30",
            "07/18/2026",
            "2026-7-1",
        ] {
            let mut task = t("A", Status::Done);
            task.evidence = vec![Evidence {
                kind: EvidenceKind::Note,
                reference: "spoke to the Division".into(),
                captured: Some(bad.to_string()),
            }];
            let r = validate(&plan_of(vec![task]), &opts());
            assert!(
                r.diagnostics.iter().any(|d| d.code == code::UNDATED_NOTE),
                "captured {bad:?} is not an ISO date and must fire APLAN010"
            );
        }
        // A real date is clean.
        assert!(is_iso_date("2026-07-18"));
        assert!(is_iso_date("2024-02-29")); // leap day
        assert!(!is_iso_date("2026-02-29")); // 2026 is not a leap year
    }

    /// Regression: a trace listed twice must produce one diagnostic, not two
    /// identical PR annotations (matching how `blocked_by` is deduplicated).
    #[test]
    fn a_repeated_trace_is_reported_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["NOPE.md#x".into(), "NOPE.md#x".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        let n = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::DANGLING_TRACE)
            .count();
        assert_eq!(n, 1, "one trace, one diagnostic: {:?}", r.diagnostics);
    }

    #[test]
    fn unknown_dependency_is_an_error() {
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["GHOST".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::UNKNOWN_DEP));
    }

    #[test]
    fn self_dependency_is_an_error() {
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::SELF_DEP));
    }

    /// Regression: a self-loop fired both `SELF_DEP` and `CYCLE`, though the
    /// former exists precisely so the generic cycle message isn't used for it.
    #[test]
    fn self_loop_is_self_dep_not_also_cycle() {
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::SELF_DEP));
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::CYCLE),
            "self-loop double-reported as a cycle: {:?}",
            r.diagnostics
        );
    }

    /// Regression: two diagnostics that tie on (code, task) must order
    /// deterministically, the sort key is now total (..., file, message).
    #[test]
    fn tied_diagnostics_sort_deterministically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["ZZZ.md#x".into(), "AAA.md#y".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        let msgs: Vec<_> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::DANGLING_TRACE)
            .map(|d| d.message.clone())
            .collect();
        assert_eq!(msgs.len(), 2);
        let mut sorted = msgs.clone();
        sorted.sort();
        assert_eq!(msgs, sorted, "tied diagnostics must be message-ordered");
    }

    #[test]
    fn cycle_is_an_error() {
        let mut a = t("A", Status::Todo);
        let mut b = t("B", Status::Todo);
        a.blocked_by = vec!["B".into()];
        b.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![a, b]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::CYCLE));
    }

    #[test]
    fn done_before_blocker_is_an_error() {
        let mut a = t("A", Status::Done);
        a.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }];
        a.blocked_by = vec!["B".into()];
        let b = t("B", Status::Todo);
        let r = validate(&plan_of(vec![a, b]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_BEFORE_BLOCKER)
        );
    }

    /// The plan is deadlocked: `B` is live but waits on a task that was
    /// abandoned, so it can never legitimately start. `ready`/`why` already knew
    /// this (`Dropped` does not satisfy a dependency); `check` used to say `ok`.
    #[test]
    fn live_task_blocked_by_a_dropped_one_is_an_error() {
        let dropped = t("A", Status::Dropped);
        let mut live = t("B", Status::Todo);
        live.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![dropped, live]), &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::DROPPED_BLOCKER)
            .expect("deadlock reported");
        assert_eq!(d.task.as_deref(), Some("B"));
        assert!(!r.is_clean(), "a deadlocked plan must not pass `check`");
    }

    /// The rule is scoped to *live* tasks: a `done` dependent is already
    /// `APLAN007`, and two abandoned tasks are not a deadlock.
    #[test]
    fn dropped_blocker_rule_does_not_fire_on_terminal_dependents() {
        let dropped = t("A", Status::Dropped);
        let mut also_dropped = t("B", Status::Dropped);
        also_dropped.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![dropped, also_dropped]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::DROPPED_BLOCKER),
            "a dropped task waiting on a dropped task is not a deadlock: {:?}",
            r.diagnostics
        );

        // A `done` dependent is `APLAN007`'s business, not this rule's, the two
        // must not double-report the same edge.
        let dropped = t("A", Status::Dropped);
        let mut done = t("C", Status::Done);
        done.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }];
        done.blocked_by = vec!["A".into()];
        let r = validate(&plan_of(vec![dropped, done]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::DONE_BEFORE_BLOCKER)
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::DROPPED_BLOCKER),
            "APLAN007 and APLAN020 must not both fire on one edge: {:?}",
            r.diagnostics
        );
    }

    /// Regression: `blocked_by: [GHOST, GHOST]` is one broken edge, but emitted
    /// the identical diagnostic twice (two identical PR annotations).
    #[test]
    fn a_repeated_blocker_is_reported_once() {
        let mut a = t("A", Status::Todo);
        a.blocked_by = vec!["GHOST".into(), "GHOST".into()];
        let r = validate(&plan_of(vec![a]), &opts());
        let n = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::UNKNOWN_DEP)
            .count();
        assert_eq!(n, 1, "one broken edge, one diagnostic: {:?}", r.diagnostics);
    }

    #[test]
    fn duplicate_id_is_an_error() {
        let r = validate(
            &plan_of(vec![t("A", Status::Todo), t("A", Status::Todo)]),
            &opts(),
        );
        assert!(r.diagnostics.iter().any(|d| d.code == code::DUPLICATE_ID));
    }

    /// Regression: the `DONE_BEFORE_BLOCKER` message once lowercased the whole
    /// string to normalise the status, mangling the task id (`C-001` ->
    /// `c-001`). Pin the casing.
    #[test]
    fn done_before_blocker_preserves_task_id_case() {
        let mut a = t("D-001", Status::Done);
        a.evidence = vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: "abc1234".into(),
            captured: None,
        }];
        a.blocked_by = vec!["C-001".into()];
        let b = t("C-001", Status::Todo);
        let r = validate(&plan_of(vec![a, b]), &opts());
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == code::DONE_BEFORE_BLOCKER)
            .expect("diagnostic raised");
        assert!(d.message.contains("C-001"), "message: {}", d.message);
        assert!(
            !d.message.contains("c-001"),
            "id was lowercased: {}",
            d.message
        );
        assert!(
            d.message.contains("todo"),
            "status not rendered: {}",
            d.message
        );
    }

    /// A cycle longer than one hop must be reported with its full path.
    #[test]
    fn multi_node_cycle_is_reported_with_path() {
        let mut first = t("A", Status::Todo);
        let mut second = t("B", Status::Todo);
        let mut third = t("C", Status::Todo);
        first.blocked_by = vec!["B".into()];
        second.blocked_by = vec!["C".into()];
        third.blocked_by = vec!["A".into()];
        let report = validate(&plan_of(vec![first, second, third]), &opts());
        let diag = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::CYCLE)
            .expect("cycle reported");
        for id in ["A", "B", "C"] {
            assert!(
                diag.message.contains(id),
                "cycle path incomplete: {}",
                diag.message
            );
        }
    }

    #[test]
    fn missing_effort_is_only_a_warning() {
        let mut a = t("A", Status::Todo);
        a.effort_days = None;
        let r = validate(&plan_of(vec![a]), &opts());
        assert!(r.diagnostics.iter().any(|d| d.code == code::MISSING_EFFORT));
        assert!(r.is_clean(), "missing effort must not fail the build");
    }

    #[test]
    fn a_milestone_without_effort_is_not_warned() {
        // A milestone is a checkpoint, not work, so `APLAN009` must not ask it for
        // an estimate, even though it is non-terminal and has no `effort_days`.
        let mut m = t("M", Status::Todo);
        m.effort_days = None;
        m.tags = vec!["milestone".into()];
        m.blocked_by = vec!["X".into()];
        let x = t("X", Status::Todo);
        let r = validate(&plan_of(vec![m, x]), &opts());
        assert!(
            !r.diagnostics.iter().any(|d| d.code == code::MISSING_EFFORT),
            "a milestone must be exempt from the effort-estimate warning"
        );
    }

    #[test]
    fn milestone_hygiene_flags_mis_shaped_milestones() {
        // APLAN021: a well-formed milestone (has a member, no effort, no paths) is
        // silent; an empty cut or a work task mis-tagged `milestone` fires.
        let x = t("X", Status::Todo);

        let mut good = t("M", Status::Todo);
        good.tags = vec!["milestone".into()];
        good.blocked_by = vec!["X".into()];
        good.effort_days = None;
        let r = validate(&plan_of(vec![good, x.clone()]), &opts());
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "a well-formed milestone must be silent"
        );

        // No members: the cut scopes no work.
        let mut empty = t("E", Status::Todo);
        empty.tags = vec!["milestone".into()];
        empty.effort_days = None;
        let r = validate(&plan_of(vec![empty]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "an empty cut must fire APLAN021"
        );

        // Work carrying effort but tagged `milestone`: the mis-tag would otherwise
        // silently delete it from every work-view.
        let mut mistag = t("W", Status::Todo); // `t` sets effort_days = Some(1.0)
        mistag.tags = vec!["milestone".into()];
        mistag.blocked_by = vec!["X".into()];
        let r = validate(&plan_of(vec![mistag, x]), &opts());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::MILESTONE_HYGIENE),
            "work carrying effort but tagged `milestone` must fire APLAN021"
        );
    }

    #[test]
    fn membership_totality_flags_orphans_and_double_membership() {
        // APLAN022 (opt-in): a task in exactly one cut is silent; a task in no cut
        // fires (unless `cross-cutting`); a task in two cuts fires.
        let w1 = t("W1", Status::Todo); // in one cut -> silent
        let orphan = t("O", Status::Todo); // in no cut -> fires
        let mut xcut = t("X", Status::Todo); // in no cut but cross-cutting -> silent
        xcut.tags = vec!["cross-cutting".into()];
        let dbl = t("D", Status::Todo); // in two cuts -> fires

        let mut m1 = t("M1", Status::Todo);
        m1.tags = vec!["milestone".into(), "v0.1".into()];
        m1.blocked_by = vec!["W1".into(), "D".into()];
        let mut m2 = t("M2", Status::Todo);
        m2.tags = vec!["milestone".into(), "v0.2".into()];
        m2.blocked_by = vec!["D".into()];

        let plan = plan_of(vec![w1, orphan, xcut, dbl, m1, m2]);
        let on = Options {
            require_cut_membership: true,
            require_effort: false,
            ..Default::default()
        };
        let fires: Vec<(String, String)> = validate(&plan, &on)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == code::MEMBERSHIP_TOTALITY)
            .map(|d| (d.task.unwrap_or_default(), d.message))
            .collect();
        assert!(
            fires.iter().any(|(id, _)| id == "O"),
            "a task in no cut must fire: {fires:?}"
        );
        assert!(
            fires
                .iter()
                .any(|(id, m)| id == "D" && m.contains("2 cuts")),
            "a task in two cuts must fire: {fires:?}"
        );
        assert!(
            !fires.iter().any(|(id, _)| id == "W1" || id == "X"),
            "one-cut and cross-cutting tasks are silent: {fires:?}"
        );

        // Off by default: no APLAN022 without the opt-in.
        let off = Options {
            require_effort: false,
            ..Default::default()
        };
        assert!(
            !validate(&plan, &off)
                .diagnostics
                .iter()
                .any(|d| d.code == code::MEMBERSHIP_TOTALITY),
            "APLAN022 must be off unless require_cut_membership is set"
        );
    }

    #[test]
    fn dangling_trace_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["NOPE.md#x".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        assert!(r.diagnostics.iter().any(|d| d.code == code::DANGLING_TRACE));
    }

    #[test]
    fn resolving_trace_anchor_is_clean() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("R.md"), "# Intro\n\n## Delaware ICIS\n").expect("write");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["R.md#delaware-icis".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    /// APLAN018: a `done` task whose stamped traced section later changes fires a
    /// (advisory) warning; a matching stamp stays silent.
    #[test]
    fn trace_drift_fires_only_when_a_stamped_section_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let doc = "# Intro\n\noriginal content\n";
        std::fs::write(dir.path().join("D.md"), doc).expect("write");
        let stamp = crate::anchor::anchor_fingerprint(doc, "intro").expect("fingerprint");

        let mut task = t("A", Status::Done);
        task.evidence = vec![commit_ev()];
        task.traces = vec!["D.md#intro".into()];
        task.trace_fingerprints = BTreeMap::from([("D.md#intro".to_string(), stamp)]);
        let on = Options {
            require_effort: false,
            detect_drift: true,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        // Stamp matches the live section: silent.
        assert!(
            !validate(&plan_of(vec![task.clone()]), &on)
                .diagnostics
                .iter()
                .any(|d| d.code == code::TRACE_DRIFT)
        );

        // The section changes; the stamp is now stale: fires, but advisory.
        std::fs::write(dir.path().join("D.md"), "# Intro\n\nCHANGED content\n").expect("write");
        let r = validate(&plan_of(vec![task]), &on);
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::TRACE_DRIFT),
            "drift must fire: {:?}",
            r.diagnostics
        );
        assert!(
            r.is_clean(),
            "APLAN018 is advisory, it must not fail `check`"
        );
    }

    /// APLAN018 stays silent without a stamp (unverifiable, not a violation) and
    /// when `detect_drift` is off.
    #[test]
    fn trace_drift_is_silent_without_a_stamp_or_when_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("D.md"), "# Intro\n\ncontent\n").expect("write");
        let mut task = t("A", Status::Done);
        task.evidence = vec![commit_ev()];
        task.traces = vec!["D.md#intro".into()];

        let fires = |task: &Task, detect_drift: bool| {
            let o = Options {
                require_effort: false,
                detect_drift,
                traces_root: Some(dir.path().to_path_buf()),
                ..Default::default()
            };
            validate(&plan_of(vec![task.clone()]), &o)
                .diagnostics
                .iter()
                .any(|d| d.code == code::TRACE_DRIFT)
        };

        // No stamp, drift on: nothing to compare against, so silent.
        assert!(!fires(&task, true));
        // A stale stamp, but drift off: silent.
        task.trace_fingerprints = BTreeMap::from([("D.md#intro".to_string(), "stale".to_string())]);
        assert!(!fires(&task, false));
    }

    /// Regression: the README's documented `traces` example must resolve against
    /// the heading the README shows. The old `slugify` rejected it.
    #[test]
    fn readme_trace_example_resolves() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("INITIAL_RESEARCH.md"),
            "# Research\n\n## 2.1 Delaware: the filing channel\n",
        )
        .expect("write");
        let mut a = t("A", Status::Todo);
        a.traces = vec!["INITIAL_RESEARCH.md#21-delaware-the-filing-channel".into()];
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let r = validate(&plan_of(vec![a]), &o);
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    #[test]
    fn duplicate_heading_resolves_by_suffix() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("R.md"), "## Setup\n## Setup\n").expect("write");
        let o = Options {
            require_effort: false,
            traces_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        // The second `Setup` is reachable only as `setup-1`.
        let mut ok = t("A", Status::Todo);
        ok.traces = vec!["R.md#setup-1".into()];
        assert!(validate(&plan_of(vec![ok]), &o).is_clean());
        // An anchor GitHub would never mint is still an error.
        let mut bad = t("B", Status::Todo);
        bad.traces = vec!["R.md#setup-2".into()];
        assert!(!validate(&plan_of(vec![bad]), &o).is_clean());
    }
}
