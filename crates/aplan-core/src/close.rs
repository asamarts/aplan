//! `aplan close`: set a task `done` and attach evidence, in place.
//!
//! Evidence is friction, and friction is the top adoption risk: hand-typing a SHA
//! into YAML is the tax that stops people closing tasks honestly. `close` makes it
//! a one-liner (and can derive the commit for free). It is the first thing that
//! *writes* a task file, so the rewrite is deliberately surgical: it changes only
//! the `status:` line and appends an `evidence:` block (and, under drift
//! detection, a `trace_fingerprints:` block), leaving the rest of the frontmatter
//! and the whole markdown body byte-for-byte untouched. A round-trip through the
//! typed model would reformat everything (`1` -> `1.0`, flow lists to block lists)
//! and bury the real change in noise.
//!
//! [`restamp`] is the APLAN018 "clear": it replaces a `done` task's
//! `trace_fingerprints:` block after the traced section was re-reviewed.

use std::collections::BTreeMap;

use crate::load::split_frontmatter;
use crate::task::{Evidence, Status, Task};

/// Why a `close` could not be applied. None of these write anything.
#[derive(Debug, thiserror::Error)]
pub enum CloseError {
    #[error("not a task file (no `---` frontmatter)")]
    NotATask,
    #[error("invalid frontmatter: {0}")]
    Frontmatter(String),
    #[error("the task is already `done`")]
    AlreadyDone,
    #[error("the task is `{0}`, a terminal state: revive it (change its status) before closing")]
    Terminal(Status),
    #[error(
        "the task already has an `evidence:` block; `close` only adds a task's first evidence, \
         edit the rest by hand"
    )]
    HasEvidence,
    #[error("no top-level `status:` line to rewrite (is the frontmatter block-style?)")]
    NoStatus,
    #[error("`--restamp` applies only to a `done` task (this one is `{0}`)")]
    NotDone(Status),
    #[error("internal: `close` produced invalid frontmatter ({0}) and refused to write it")]
    Produced(String),
}

/// Return `file` with the task set `done` and `evidence` appended, or a
/// [`CloseError`] (having written nothing). The caller writes the result.
///
/// Scoped to a task that is not yet `done` and has no evidence: that is the whole
/// point (record a task's first, honest proof), and it keeps the rewrite a safe
/// text edit rather than a fragile splice into an existing block.
pub fn close(
    file: &str,
    evidence: &[Evidence],
    fingerprints: &BTreeMap<String, String>,
) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;

    // Both terminal states are refused: `Done` is already closed, and `Dropped`
    // was deliberately abandoned, silently reviving it to `done` would launder a
    // decision the plan recorded.
    if task.status.is_terminal() {
        return Err(if task.status == Status::Done {
            CloseError::AlreadyDone
        } else {
            CloseError::Terminal(task.status)
        });
    }

    // Refuse if an `evidence:` key already exists, even `evidence: []`: `close`
    // only *appends* a task's first block, and a second `evidence:` key would be
    // a duplicate-key document. (This is stricter than the parsed
    // `task.evidence`, which reads `evidence: []` as "no evidence".)
    if has_top_level_key(frontmatter, "evidence") {
        return Err(CloseError::HasEvidence);
    }

    let nl = newline(file, bom);
    let new_frontmatter = format!(
        "{}{}{}",
        set_status_done(frontmatter)?,
        evidence_block(evidence, nl)?,
        fingerprint_block(fingerprints, nl)?,
    );

    // Never write frontmatter that would not load: parse it back, and confirm the
    // status, the *exact* evidence, and the fingerprints round-tripped (value
    // equality, not a matching count, so a mis-serialised reference cannot slip).
    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != Status::Done
        || parsed.evidence.as_slice() != evidence
        || &parsed.trace_fingerprints != fingerprints
    {
        return Err(CloseError::Produced(
            "the status, evidence, or fingerprints did not round-trip".to_string(),
        ));
    }

    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// Replace a `done` task's `trace_fingerprints:` block with a freshly computed
/// one, leaving `status`, `evidence`, and the body untouched. This is the
/// APLAN018 "clear": after re-reviewing a task whose traced section drifted, its
/// stamp is re-taken so the warning is silenced deliberately, not by accident.
pub fn restamp(file: &str, fingerprints: &BTreeMap<String, String>) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if task.status != Status::Done {
        return Err(CloseError::NotDone(task.status));
    }

    let nl = newline(file, bom);
    let without = remove_top_level_block(frontmatter, "trace_fingerprints");
    let new_frontmatter = format!("{}{}", without, fingerprint_block(fingerprints, nl)?);

    // Restamp touches only the fingerprints: confirm the status and evidence came
    // through unchanged (a future bug in the block removal would be caught here,
    // as `close` catches its own).
    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != task.evidence
        || &parsed.trace_fingerprints != fingerprints
    {
        return Err(CloseError::Produced(
            "restamp altered the status, evidence, or fingerprints".to_string(),
        ));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// A leading BOM, stripped by `split_frontmatter`, preserved so a byte-for-byte
/// rewrite restores exactly what was there.
fn leading_bom(file: &str) -> &'static str {
    if file.starts_with('\u{feff}') {
        "\u{feff}"
    } else {
        ""
    }
}

/// The file's own newline (the one the opening fence used), so a CRLF file stays
/// all-CRLF: the rebuilt fences and appended blocks match it.
fn newline(file: &str, bom: &str) -> &'static str {
    if file[bom.len()..].starts_with("---\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Parse frontmatter back to a `Task`, refusing to return anything that would not
/// load.
fn reparse(frontmatter: &str) -> Result<Task, CloseError> {
    serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Produced(e.to_string()))
}

/// Whether the frontmatter declares `key` as a top-level key (with or without a
/// value, and with or without a space before the colon).
fn has_top_level_key(frontmatter: &str, key: &str) -> bool {
    frontmatter.lines().any(|line| {
        line.strip_prefix(key)
            .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
    })
}

/// The frontmatter with a top-level `key:` line and its indented continuation
/// removed. Used by [`restamp`] to replace an existing `trace_fingerprints:`
/// block.
fn remove_top_level_block(frontmatter: &str, key: &str) -> String {
    let mut out = String::with_capacity(frontmatter.len());
    let mut skipping = false;
    for line in frontmatter.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if skipping {
            // Only an indented continuation line still belongs to the block; a
            // blank line or a new top-level key ends it (so a blank line *after*
            // the block is preserved, not swallowed).
            if content.starts_with([' ', '\t']) {
                continue;
            }
            skipping = false;
        }
        if content
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
        {
            skipping = true;
            continue;
        }
        out.push_str(line);
    }
    out
}

/// The `trace_fingerprints:` block for `fingerprints` (empty string when there
/// are none), serialised via serde and re-indented two spaces, using `nl`.
fn fingerprint_block(
    fingerprints: &BTreeMap<String, String>,
    nl: &str,
) -> Result<String, CloseError> {
    if fingerprints.is_empty() {
        return Ok(String::new());
    }
    let map =
        serde_yaml_ng::to_string(fingerprints).map_err(|e| CloseError::Produced(e.to_string()))?;
    let mut out = format!("trace_fingerprints:{nl}");
    for line in map.lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push_str(nl);
    }
    Ok(out)
}

/// Replace the value of the first top-level `status:` line with `done`,
/// preserving the exact key segment (`status:` or `status :`), the spacing after
/// the colon, any trailing `# comment`, and the line's own ending. Every other
/// line is untouched.
fn set_status_done(frontmatter: &str) -> Result<String, CloseError> {
    let mut out = String::with_capacity(frontmatter.len() + 8);
    let mut changed = false;
    for line in frontmatter.split_inclusive('\n') {
        if !changed {
            if let Some(rewritten) = rewrite_status_line(line) {
                out.push_str(&rewritten);
                changed = true;
                continue;
            }
        }
        out.push_str(line);
    }
    if changed {
        Ok(out)
    } else {
        Err(CloseError::NoStatus)
    }
}

/// If `line` is a top-level `status:` line, return it with only the value token
/// replaced by `done`; otherwise `None`. Preserves indentation-free key spacing,
/// the whitespace after the colon, a trailing comment, and the line ending.
fn rewrite_status_line(line: &str) -> Option<String> {
    let eol_len = line.len() - line.trim_end_matches(['\r', '\n']).len();
    let content = &line[..line.len() - eol_len];
    let eol = &line[content.len()..];

    // Top-level key only (no leading whitespace: an indented `status:` belongs to
    // some nested mapping, not the task).
    let after_key = content.strip_prefix("status")?;
    let after_ws = after_key.trim_start_matches([' ', '\t']);
    let after_colon = after_ws.strip_prefix(':')?;
    let key_segment = &content[..content.len() - after_colon.len()];

    // Keep the whitespace after the colon; replace the value token (up to the
    // next whitespace or `#`); keep whatever follows (a trailing comment).
    let ws = after_colon.len() - after_colon.trim_start_matches([' ', '\t']).len();
    let (lead_ws, tail) = after_colon.split_at(ws);
    let val_end = tail
        .find(|c: char| c.is_whitespace() || c == '#')
        .unwrap_or(tail.len());
    let trailing = &tail[val_end..];
    Some(format!("{key_segment}{lead_ws}done{trailing}{eol}"))
}

/// The `evidence:` block for `evidence`, serialised via serde (so quoting is
/// correct) and re-indented two spaces to match the hand-written style, using
/// `nl` as the line ending so it matches the file it is appended to.
fn evidence_block(evidence: &[Evidence], nl: &str) -> Result<String, CloseError> {
    let seq =
        serde_yaml_ng::to_string(&evidence).map_err(|e| CloseError::Produced(e.to_string()))?;
    let mut out = format!("evidence:{nl}");
    for line in seq.lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push_str(nl);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::EvidenceKind;

    fn commit(reference: &str) -> Evidence {
        Evidence {
            kind: EvidenceKind::Commit,
            reference: reference.to_string(),
            captured: None,
        }
    }

    /// `close` with no trace fingerprints, the common case in these tests.
    fn close_ev(file: &str, evidence: &[Evidence]) -> Result<String, CloseError> {
        close(file, evidence, &BTreeMap::new())
    }

    fn fps(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    const TASK: &str = "\
---
id: A1-003
title: Ship it
phase: 1
status: todo
owner: kaminsod
effort_days: 1
blocked_by: [A1-001]
tags: [ci]
---

## Why

Because.
";

    #[test]
    fn closes_a_task_and_leaves_everything_else_untouched() {
        let out = close_ev(TASK, &[commit("abc1234")]).unwrap();
        // The change is present...
        assert!(out.contains("status: done"));
        assert!(out.contains("evidence:\n  - kind: commit\n    ref: abc1234\n"));
        // ...and everything else is byte-identical: no `1` -> `1.0`, no reflowed
        // lists, and the body is intact.
        assert!(out.contains("effort_days: 1\n"));
        assert!(out.contains("blocked_by: [A1-001]\n"));
        assert!(out.contains("tags: [ci]\n"));
        assert!(out.contains("\n## Why\n\nBecause.\n"));
        // And it still loads as a `done` task with that evidence.
        let (fm, _) = split_frontmatter(&out).unwrap();
        let task: Task = serde_yaml_ng::from_str(fm).unwrap();
        assert_eq!(task.status, Status::Done);
        assert_eq!(task.evidence, vec![commit("abc1234")]);
    }

    #[test]
    fn refuses_an_already_done_task() {
        let done = TASK.replace("status: todo", "status: done");
        assert!(matches!(
            close_ev(&done, &[commit("x")]),
            Err(CloseError::AlreadyDone)
        ));
    }

    #[test]
    fn refuses_a_task_that_already_has_evidence() {
        let with_ev = TASK.replace(
            "status: todo\n",
            "status: todo\nevidence:\n  - kind: note\n    ref: x\n    captured: 2026-07-01\n",
        );
        assert!(matches!(
            close_ev(&with_ev, &[commit("x")]),
            Err(CloseError::HasEvidence)
        ));
    }

    #[test]
    fn refuses_a_non_task_file() {
        assert!(matches!(
            close_ev("# just prose\n", &[commit("x")]),
            Err(CloseError::NotATask)
        ));
    }

    #[test]
    fn a_reference_needing_quotes_is_quoted() {
        // A ref with a leading `@` or a colon must not corrupt the YAML.
        let out = close_ev(TASK, &[commit("path: with colon")]).unwrap();
        let (fm, _) = split_frontmatter(&out).unwrap();
        let task: Task = serde_yaml_ng::from_str(fm).unwrap();
        assert_eq!(task.evidence[0].reference, "path: with colon");
    }

    #[test]
    fn refuses_a_dropped_task() {
        // `Dropped` is terminal: `close` must not launder an abandoned task into
        // `done`.
        let dropped = TASK.replace("status: todo", "status: dropped");
        assert!(matches!(
            close_ev(&dropped, &[commit("x")]),
            Err(CloseError::Terminal(Status::Dropped))
        ));
    }

    #[test]
    fn refuses_a_task_with_an_empty_evidence_list() {
        // `evidence: []` parses as "no evidence", but the key is present;
        // appending a second `evidence:` would be a duplicate-key document.
        let with_empty = TASK.replace("status: todo\n", "status: todo\nevidence: []\n");
        assert!(matches!(
            close_ev(&with_empty, &[commit("x")]),
            Err(CloseError::HasEvidence)
        ));
    }

    #[test]
    fn a_crlf_file_stays_all_crlf() {
        let crlf = TASK.replace('\n', "\r\n");
        let out = close_ev(&crlf, &[commit("abc1234")]).unwrap();
        // No lone LF survives: strip every CRLF, and nothing with a `\n` remains.
        assert!(
            !out.replace("\r\n", "").contains('\n'),
            "a lone LF crept into a CRLF file: {out:?}"
        );
        assert!(out.contains("status: done\r\n"));
        assert!(out.contains("evidence:\r\n  - kind: commit\r\n    ref: abc1234\r\n"));
        let (fm, _) = split_frontmatter(&out).unwrap();
        let task: Task = serde_yaml_ng::from_str(fm).unwrap();
        assert_eq!(task.status, Status::Done);
        assert_eq!(task.evidence, vec![commit("abc1234")]);
    }

    #[test]
    fn a_leading_bom_is_preserved() {
        let with_bom = format!("\u{feff}{TASK}");
        let out = close_ev(&with_bom, &[commit("x")]).unwrap();
        assert!(out.starts_with('\u{feff}'), "BOM was dropped: {out:?}");
        assert!(!out["\u{feff}".len()..].contains('\u{feff}'), "BOM doubled");
        let (fm, _) = split_frontmatter(&out).unwrap();
        assert_eq!(
            serde_yaml_ng::from_str::<Task>(fm).unwrap().status,
            Status::Done
        );
    }

    #[test]
    fn a_trailing_comment_on_the_status_line_is_preserved() {
        let commented = TASK.replace("status: todo", "status: todo  # keep me");
        let out = close_ev(&commented, &[commit("x")]).unwrap();
        assert!(
            out.contains("status: done  # keep me"),
            "trailing comment lost: {out:?}"
        );
    }

    #[test]
    fn a_space_before_the_colon_is_still_found_and_preserved() {
        // `status : todo` is valid YAML (key `status`); close must find it and
        // keep the author's spacing.
        let spaced = TASK.replace("status: todo", "status : todo");
        let out = close_ev(&spaced, &[commit("x")]).unwrap();
        assert!(
            out.contains("status : done"),
            "spaced key mishandled: {out:?}"
        );
    }

    #[test]
    fn close_stamps_a_trace_fingerprints_block() {
        let out = close(
            TASK,
            &[commit("abc1234")],
            &fps(&[("HLD.md#x", "deadbeef")]),
        )
        .unwrap();
        assert!(out.contains("trace_fingerprints:"), "{out}");
        let (fm, _) = split_frontmatter(&out).unwrap();
        let task: Task = serde_yaml_ng::from_str(fm).unwrap();
        assert_eq!(
            task.trace_fingerprints.get("HLD.md#x").map(String::as_str),
            Some("deadbeef")
        );
        // With no fingerprints there is no block (byte-identical to before).
        let plain = close_ev(TASK, &[commit("abc1234")]).unwrap();
        assert!(!plain.contains("trace_fingerprints"));
    }

    #[test]
    fn restamp_replaces_the_block_on_a_done_task() {
        let done = close(TASK, &[commit("abc1234")], &fps(&[("HLD.md#x", "0000")])).unwrap();
        let fresh = restamp(&done, &fps(&[("HLD.md#x", "ffff")])).unwrap();
        let (fm, _) = split_frontmatter(&fresh).unwrap();
        let task: Task = serde_yaml_ng::from_str(fm).unwrap();
        assert_eq!(
            task.trace_fingerprints.get("HLD.md#x").map(String::as_str),
            Some("ffff")
        );
        // Status and evidence are untouched, and the old block was replaced (not
        // duplicated).
        assert_eq!(task.status, Status::Done);
        assert_eq!(task.evidence, vec![commit("abc1234")]);
        assert_eq!(fresh.matches("trace_fingerprints:").count(), 1);
    }

    #[test]
    fn restamp_refuses_a_task_that_is_not_done() {
        assert!(matches!(
            restamp(TASK, &fps(&[("x", "y")])),
            Err(CloseError::NotDone(Status::Todo))
        ));
    }
}
