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
    /// The citation to be replaced is not on the task. Naming both sides is what
    /// makes a repair auditable, so a `from` that matches nothing is a mistake
    /// rather than something to guess at.
    #[error("this task cites no `{0}`")]
    NoSuchEvidence(String),
    /// The citation to be replaced appears more than once, so replacing "it" is
    /// not a defined operation.
    #[error("this task cites `{0}` {1} times, so which one to replace is ambiguous")]
    AmbiguousEvidence(String, usize),
    /// The replacement is already on the task, so the repair would collapse two
    /// citations into one and silently drop proof.
    #[error("this task already cites `{0}`")]
    DuplicateEvidence(String),
    #[error("the task is `{0}`, a terminal state: revive it (change its status) before closing")]
    Terminal(Status),
    #[error("the task already has an `evidence:` block; `close` only adds a task's first evidence")]
    HasEvidence,
    #[error(
        "no top-level `{0}:` line this edit can rewrite: it is absent, or its value is a shape \
         (written below the key, or carrying a YAML tag, anchor or alias) that would have to be \
         guessed at -- change it by hand"
    )]
    NoScalar(&'static str),
    #[error("only a `done` task can be repaired (this one is `{0}`)")]
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
/// The remedy for a refusal, where naming one is better than describing it.
///
/// Only [`CloseError::HasEvidence`] has one today, and it is the reason this
/// exists: the message used to end *edit the rest by hand*, and
/// `aplan amend <id> --add-evidence <kind>:<ref>` already does that operation --
/// resolving the citation the way `close` resolves one and running the
/// post-state rules before writing. So the tool was sending an author around
/// both of its own gates, to edit a YAML block it had just written, which is
/// the file-format coupling this surface exists to remove.
///
/// Two pilots hit it the same day and neither found the command from the
/// message: `p00020` hand-edited five of the eight tasks it closed in one pass,
/// and `p00002` found `--add-evidence` "only by reading `amend --help` on a
/// guess" (`A1-265`).
///
/// It names the whole invocation with the id filled in, which is what
/// `APLAN013` does when it names the test form that would work -- and `p00020`
/// singles that out as the difference between a thirty-second fix and a hunt.
/// It lives here because both surfaces say it: an agent cannot close through
/// MCP what the command refuses, and that includes being told the same way
/// (`A1-131`, `A1-270`).
#[must_use]
pub fn advice(error: &CloseError, id: &str) -> Option<String> {
    match error {
        // The case an author actually reaches. `close` derived a commit, the
        // task is `done`, and now there is a test or a file that also proves
        // it -- which is `p00020`'s five hand-edits and `p00002`'s guess at
        // `--help`. The bare refusal said only that the task was closed, and
        // `Terminal`'s own text offers *revive it*, which is the wrong remedy
        // for wanting to add a citation and the expensive one to follow.
        // `AlreadyDone` only: a `Done` task never reaches `Terminal`, which the
        // branch above routes it away from, and an arm for it would be a
        // mechanism driving nothing. If that ever changes, the test that closes
        // a `done` task and asserts this sentence is what fails.
        CloseError::AlreadyDone => Some(format!(
            "To add a citation to it: `aplan amend {id} --add-evidence <kind>:<ref>`, which \
             resolves it the way `close` does and does not reopen the task. To replace one \
             that was wrong, `--replace-evidence <from> <to>`."
        )),
        CloseError::HasEvidence => Some(format!(
            "To add one to a task that is already closed: \
             `aplan amend {id} --add-evidence <kind>:<ref>`, which resolves it the way `close` \
             does. To replace one that was wrong, `--replace-evidence <from> <to>`."
        )),
        _ => None,
    }
}

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
        set_status_in(frontmatter, Status::Done)?,
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

/// Set a task's `status`, leaving everything else byte-for-byte alone.
///
/// The mutation `drop` and `mv` are built on: a surgical edit to one line, then
/// a parse of the result before anything is written, on `close`'s standard that
/// a rewrite it cannot read again is a corrupted file and refusing beats saving
/// it (`A1-077`).
///
/// Refuses to move *out* of a terminal state. `dropped` records a decision and
/// `done` records evidence; quietly reviving either would launder something the
/// plan said, which is the same reasoning that makes `close` refuse them.
///
/// # Errors
/// [`CloseError::NotATask`] without frontmatter, [`CloseError::NoScalar`] with
/// no top-level `status:`, [`CloseError::Terminal`] when the task is already
/// terminal, and [`CloseError::Produced`] when the rewrite does not round-trip.
pub fn set_status(file: &str, to: Status) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if task.status.is_terminal() {
        return Err(if task.status == Status::Done {
            CloseError::AlreadyDone
        } else {
            CloseError::Terminal(task.status)
        });
    }

    let nl = newline(file, bom);
    let new_frontmatter = set_status_in(frontmatter, to)?;
    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != to {
        return Err(CloseError::Produced(
            "the status did not round-trip".to_string(),
        ));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// Add or remove one `blocked_by` edge, leaving everything else untouched.
///
/// `blocked_by` is the only per-task ordering the model has, and it was written
/// by `add` and never again: the one mechanism this tool uses to say "this comes
/// first" was maintainable only in a text editor, in a repository whose premise
/// is that the tool owns those files. That is the unfinished half of the fourth
/// pilot's item 6, whose other half `A1-077` closed with `drop` and `mv`
/// (`A1-124`).
///
/// Whether the *result* is legal is not decided here: this edits one file, and a
/// cycle or an unknown id is a question about the plan. `A1-113`'s post-state
/// check answers those, which is why this does not try to.
///
/// # Errors
/// [`CloseError::NotATask`] when the file has no frontmatter,
/// [`CloseError::Frontmatter`] when it does not parse, and
/// [`CloseError::Produced`] when the edit would not round-trip.
pub fn set_blocked_by(file: &str, blocked_by: &[String]) -> Result<String, CloseError> {
    set_id_list(file, "blocked_by", blocked_by, |t| &t.blocked_by)
}

/// Move a task to another phase, rewriting one line (`A1-171`).
///
/// `phase` is the one field every task carries that no command wrote after
/// creation: `add --phase` set it once and nothing changed it, so re-grouping
/// work -- which is what re-prioritising *is* -- ended in a hand-edit of the
/// files this tool claims to own. Rollups read it (`summary` groups by it), so
/// a plan could express a change of cut and not a change of phase.
///
/// Surgical, like [`set_status`] and for the same reason: the value token is
/// replaced and the key's spacing, any trailing comment and the line ending are
/// the author's. Nothing about a phase can be illegal -- no rule reads it, and
/// the schema says it is "used for rollups and ordering, not for gating" -- so
/// there is no guard here beyond proving the file still parses and really says
/// what was asked.
///
/// # Errors
/// [`CloseError::NotATask`] without frontmatter, [`CloseError::Frontmatter`]
/// when it does not parse, [`CloseError::NoScalar`] with no top-level `phase:`,
/// and [`CloseError::Produced`] when the rewrite does not round-trip.
pub fn set_phase(file: &str, to: u32) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let _: Task =
        serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;

    let nl = newline(file, bom);
    let mut out = String::with_capacity(frontmatter.len() + 8);
    let mut changed = false;
    for line in frontmatter.split_inclusive('\n') {
        if !changed && let Some(rewritten) = rewrite_scalar_line(line, "phase", &to.to_string()) {
            out.push_str(&rewritten);
            changed = true;
            continue;
        }
        out.push_str(line);
    }
    if !changed {
        return Err(CloseError::NoScalar("phase"));
    }
    let parsed = reparse(&out)?;
    if parsed.phase != to {
        return Err(CloseError::Produced(
            "the phase did not round-trip".to_string(),
        ));
    }
    Ok(format!("{bom}---{nl}{out}---{nl}{body}"))
}

/// The same for `prefer_after`, the discretionary order (`A1-150`).
///
/// # Errors
///
/// Whatever [`set_blocked_by`] returns, for the same reasons.
pub fn set_prefer_after(file: &str, prefer_after: &[String]) -> Result<String, CloseError> {
    set_id_list(file, "prefer_after", prefer_after, |t| &t.prefer_after)
}

/// Replace one top-level list of task ids, and prove it round-tripped.
///
/// One function for both edges because the two differ only in their key and in
/// which field the check reads back: a second copy would be a second place for
/// the indentation rule below to be got wrong (`A1-150`).
/// Where the flow list ends, skipping any `]` inside a quoted id (`A1-179`).
///
/// The first `]` is the list's only when no id contains one. `["a]b"]` is a
/// legal task file that loads, and taking the first bracket made everything
/// after it "the trailing comment", so the rewrite spliced it back and produced
/// frontmatter that would not parse. The round-trip guard refused that rather
/// than writing it -- but as `internal: close produced invalid frontmatter`,
/// which blames this tool for a shape the file chose, and on a file the block
/// form had rewritten happily before. Two defects for one bracket.
fn closing_bracket(value: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in value.char_indices() {
        // A quoted scalar ends at its own quote kind. YAML escaping inside one
        // is richer than this and does not matter: stepping over a `]` that is
        // not the list's is all this has to do.
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            ']' => return Some(i),
            _ => {}
        }
    }
    None
}

/// The `key: [a, b]` line, when the file writes this list inline (`A1-179`).
///
/// Only the flow form, and only at the top level: a `key:` with nothing after it
/// is the block form, and an indented one belongs to some nested mapping rather
/// than to the task. Anything else -- an anchor, a value this cannot recognise
/// -- returns `None` and is written back as a block, which is what this tool
/// wrote before and what it still writes for a key it is creating.
fn inline_list_line<'a>(frontmatter: &'a str, key: &str) -> Option<InlineList<'a>> {
    for line in frontmatter.lines() {
        let Some(after_key) = line.strip_prefix(key) else {
            continue;
        };
        let after_ws = after_key.trim_start_matches([' ', '\t']);
        let Some(after_colon) = after_ws.strip_prefix(':') else {
            continue;
        };
        let key_len = line.len() - after_colon.len();
        let spacing_len = after_colon.len() - after_colon.trim_start().len();
        let value = &after_colon[spacing_len..];
        if !value.starts_with('[') {
            return None;
        }
        // A flow list broken across lines has no closing bracket here, and is
        // rewritten as one line without a trailing anything: `mv` refuses that
        // shape outright, so collapsing it is the smaller surprise of the two.
        let trailing = closing_bracket(value).map_or("", |close| &value[close + 1..]);
        return Some(InlineList {
            key: &line[..key_len],
            spacing: &after_colon[..spacing_len],
            trailing,
        });
    }
    None
}

/// The parts of an inline `key: [a, b] # comment` line that are the author's.
struct InlineList<'a> {
    /// Everything up to and including the colon, so `key :` survives as written.
    key: &'a str,
    /// The whitespace the author put after it.
    spacing: &'a str,
    /// Whatever followed the closing bracket, which is usually a comment.
    trailing: &'a str,
}

fn set_id_list(
    file: &str,
    key: &str,
    ids: &[String],
    read_back: fn(&Task) -> &Vec<String>,
) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let _: Task =
        serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;

    let nl = newline(file, bom);
    let block = if ids.is_empty() {
        String::new()
    } else if let Some(found) = inline_list_line(frontmatter, key) {
        // The form the file already used, with the spacing and any trailing
        // comment it had. A rewrite that changes what it found is a diff nobody
        // asked for, in a file this tool claims to own carefully -- and `dep`'s
        // whole argument was that hand-editing should not be necessary, not that
        // the tool would reformat instead (`A1-179`). `mv` has kept the author's
        // form since it was written; this is the same promise, one command over.
        format!(
            "{}{}[{}]{}{nl}",
            found.key,
            found.spacing,
            ids.join(", "),
            found.trailing
        )
    } else {
        // Indented, like `evidence:` and `trace_fingerprints:` beside it, because
        // `replace_top_level_block` ends a block at the first line that does not
        // start with whitespace: an unindented `- B` is not skipped, so replacing
        // a list left the old entries behind it. The round-trip check caught that
        // before anything was written, which is what it is for.
        let mut out = format!("{key}:{nl}");
        for id in ids {
            out.push_str("  - ");
            out.push_str(id);
            out.push_str(nl);
        }
        out
    };
    let new_frontmatter = replace_top_level_block(frontmatter, key, &block);

    // Never write frontmatter that would not load, and confirm the list itself
    // round-tripped rather than counting entries: an id serialised wrong is the
    // failure this check exists for (`A1-110`).
    let parsed = reparse(&new_frontmatter)?;
    if read_back(&parsed) != ids {
        return Err(CloseError::Produced(format!("`{key}` did not round-trip")));
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
    // In place, not remove-and-append: a re-stamp of a task whose
    // `trace_fingerprints:` sat before other keys moved the block to the end, so
    // acknowledging drift arrived as a diff that reordered the frontmatter. Found
    // auditing `A1-110`, and shipped this way since `A1-086`.
    let new_frontmatter = replace_top_level_block(
        frontmatter,
        "trace_fingerprints",
        &fingerprint_block(fingerprints, nl)?,
    );

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

/// Add a citation to a `done` task's existing evidence block, in place.
///
/// The other half of the repair surface. `replace_evidence` fixes proof that
/// *rotted*; this one completes proof that was never whole: a fix whose tests
/// were written and never cited, which measured at 52 of this plan's 90 closed
/// tasks (`A1-123`). Additive, so nothing already recorded is disturbed, and the
/// caller resolves it exactly as a replacement is resolved.
pub fn add_evidence(file: &str, add: &Evidence) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if task.status != Status::Done {
        return Err(CloseError::NotDone(task.status));
    }
    if task.evidence.iter().any(|e| e == add) {
        return Err(CloseError::DuplicateEvidence(format!(
            "{}:{}",
            add.kind.as_str(),
            add.reference
        )));
    }

    let mut extended = task.evidence.clone();
    extended.push(add.clone());
    let nl = newline(file, bom);
    let new_frontmatter =
        replace_top_level_block(frontmatter, "evidence", &evidence_block(&extended, nl)?);

    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != extended
        || parsed.trace_fingerprints != task.trace_fingerprints
    {
        return Err(CloseError::Produced(
            "the addition altered the status, the fingerprints, or the existing citations"
                .to_string(),
        ));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// Replace one evidence citation on a `done` task with another, in place.
///
/// `test` evidence is cited by name, so a rename leaves every citing task
/// pointing at nothing, including tasks closed weeks earlier that nobody is
/// editing. One refactor in the first pilot did it to two closed tasks at once,
/// and the tool's answer was `close` refusing twice and then telling the author
/// to "edit the rest by hand": the repair for a tool-detected problem was to
/// hand-edit the files the tool owns (pre-public adoption evidence, item 9).
///
/// **Both sides are named.** That is what makes this a repair rather than an
/// edit: the diff shows what was replaced and by what, a `from` that matches
/// nothing is refused instead of guessed at, and there is no shape of this
/// operation that replaces a whole block with something nobody checked.
///
/// Deliberately does *not* touch `trace_fingerprints`. Repairing a citation is
/// not re-reviewing a traced section, and doing both here would let an amendment
/// silently clear a drift warning it never looked at. `close --restamp` is the
/// Remove one citation from a closed task's `evidence`, in place (`A1-137`).
///
/// The third shape of repair, after proof that *rotted* (`replace_evidence`,
/// `A1-110`) and proof that was never *whole* (`add_evidence`, `A1-120`): proof
/// that was never proof. `A1-123` wrote a citation for `A1-120` that would not
/// have failed if that task's work were reverted, noticed, and had to delete it
/// in a text editor -- and `A1-045` found five more, each a `file:` citation
/// naming the test that proves the task, written before a `test:` citation could
/// resolve at all.
///
/// The objection was that removal is how a closed task quietly loses its proof,
/// and unlike the other two shapes there is nothing for the gate to resolve. The
/// answer is the caller's: `A1-113`'s post-state check refuses a removal that
/// leaves the task below `require_min_strength` or with no evidence at all, so
/// an author cannot remove their way under the bar. What they can do is drop a
/// citation that was never proof, after stating what replaces it.
///
/// # Errors
/// [`CloseError::NotDone`] for a live task, [`CloseError::NoSuchEvidence`] when
/// the citation is absent, [`CloseError::AmbiguousEvidence`] when it appears
/// twice, and [`CloseError::Produced`] when the rewrite does not read back as
/// exactly that removal.
pub fn remove_evidence(file: &str, drop: &Evidence) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if task.status != Status::Done {
        return Err(CloseError::NotDone(task.status));
    }

    let label = |e: &Evidence| format!("{}:{}", e.kind.as_str(), e.reference);
    match task.evidence.iter().filter(|e| *e == drop).count() {
        0 => return Err(CloseError::NoSuchEvidence(label(drop))),
        1 => {}
        n => return Err(CloseError::AmbiguousEvidence(label(drop), n)),
    }

    let remaining: Vec<Evidence> = task
        .evidence
        .iter()
        .filter(|e| *e != drop)
        .cloned()
        .collect();
    // `APLAN005` is the rule with an opinion about a `done` task citing nothing,
    // and the caller runs it: refusing here as well would give one fault two
    // messages, and the caller's carries the remedy.
    let nl = newline(file, bom);
    // An empty block removes the key: `replace_top_level_block` writes the block
    // in place of the key line and skips its indented body, so "" deletes both.
    // A `done` task with no evidence is `APLAN005`, which the caller's post-state
    // check reports with its remedy; refusing here too would give one fault two
    // messages.
    let block = if remaining.is_empty() {
        String::new()
    } else {
        evidence_block(&remaining, nl)?
    };
    let new_frontmatter = replace_top_level_block(frontmatter, "evidence", &block);

    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != remaining
        || parsed.trace_fingerprints != task.trace_fingerprints
    {
        return Err(CloseError::Produced(
            "the repair altered the status, the fingerprints, or more than the one citation"
                .to_string(),
        ));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// other repair and stays its own (`A1-110`).
pub fn replace_evidence(file: &str, from: &Evidence, to: &Evidence) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        serde_yaml_ng::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    // Scoped to a closed task, because that is the case with no other route: a
    // live task's evidence can still be given by `close`.
    if task.status != Status::Done {
        return Err(CloseError::NotDone(task.status));
    }

    let label = |e: &Evidence| format!("{}:{}", e.kind.as_str(), e.reference);
    let matches = task.evidence.iter().filter(|e| *e == from).count();
    match matches {
        0 => return Err(CloseError::NoSuchEvidence(label(from))),
        1 => {}
        n => return Err(CloseError::AmbiguousEvidence(label(from), n)),
    }
    if task.evidence.iter().any(|e| e == to) {
        return Err(CloseError::DuplicateEvidence(label(to)));
    }

    let repaired: Vec<Evidence> = task
        .evidence
        .iter()
        .map(|e| if e == from { to.clone() } else { e.clone() })
        .collect();

    let nl = newline(file, bom);
    let new_frontmatter =
        replace_top_level_block(frontmatter, "evidence", &evidence_block(&repaired, nl)?);

    // The same self-check `close` and `restamp` make: nothing but the one
    // citation may have moved.
    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != repaired
        || parsed.trace_fingerprints != task.trace_fingerprints
    {
        return Err(CloseError::Produced(
            "the repair altered the status, the fingerprints, or more than the one citation"
                .to_string(),
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

/// The frontmatter with a top-level `key:` block replaced *in place* by
/// `block`, keeping the key where the author put it.
///
/// [`remove_top_level_block`] plus an append is the wrong shape for a repair:
/// it moves the block to the end, so a one-line change arrives as a diff that
/// deletes a block here and adds it there. This module is surgical on purpose
/// (see the module docs), and `A1-110`'s whole design is that the change be
/// legible in the diff. Measured: repairing a task whose `evidence:` sat before
/// `paths:` and `trace_fingerprints:` reordered all three.
fn replace_top_level_block(frontmatter: &str, key: &str, block: &str) -> String {
    let mut out = String::with_capacity(frontmatter.len() + block.len());
    let mut skipping = false;
    let mut replaced = false;
    for line in frontmatter.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if skipping {
            // A block's continuation lines, either indented or written as an
            // unindented sequence. The second shape is what this tool's own
            // importer produces, and skipping only the first left the old
            // entries standing after the new block: the frontmatter then failed
            // to parse and every `dep add` on an imported task was refused as an
            // internal error (`A1-033`'s audit). A top-level mapping key cannot
            // begin with `- `, so this is unambiguous.
            if content.starts_with([' ', '\t']) || content == "-" || content.starts_with("- ") {
                continue;
            }
            skipping = false;
        }
        if !replaced
            && content
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
        {
            out.push_str(block);
            skipping = true;
            replaced = true;
            continue;
        }
        out.push_str(line);
    }
    // A key that was not there is appended, so this is total: the caller has
    // already established the task carries the citation, but a helper that
    // silently dropped the block on a shape nobody predicted would be worse.
    if !replaced {
        out.push_str(block);
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
fn set_status_in(frontmatter: &str, to: Status) -> Result<String, CloseError> {
    let mut out = String::with_capacity(frontmatter.len() + 8);
    let mut changed = false;
    for line in frontmatter.split_inclusive('\n') {
        if !changed {
            if let Some(rewritten) = rewrite_scalar_line(line, "status", &to.to_string()) {
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
        Err(CloseError::NoScalar("status"))
    }
}

/// If `line` is a top-level `key:` line, return it with only the value token
/// replaced by `to`; otherwise `None`. Preserves indentation-free key spacing,
/// the whitespace after the colon, a trailing comment, and the line ending.
///
/// One function for `status` and `phase` because the two differ only in their
/// key and their value: a second copy would be a second place for the top-level
/// rule below to be got wrong, which is the argument `set_id_list` already makes
/// for the two list keys (`A1-171`).
fn rewrite_scalar_line(line: &str, key: &str, to: &str) -> Option<String> {
    let eol_len = line.len() - line.trim_end_matches(['\r', '\n']).len();
    let content = &line[..line.len() - eol_len];
    let eol = &line[content.len()..];

    // Top-level key only (no leading whitespace: an indented `status:` belongs to
    // some nested mapping, not the task).
    let after_key = content.strip_prefix(key)?;
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
    // A value this cannot replace by swapping one token: an empty one means the
    // value is on the lines below, and a tag, anchor, alias or block indicator
    // means the token is not the value. Replacing the first word of `!!int 0`
    // yields `2 0`, which is not YAML -- caught downstream by the round-trip
    // check, but reported as though this had a bug rather than as a shape it
    // declines to guess at. Refused here, where the shape is visible (`A1-171`,
    // audit). `set_status` was open to the same thing and is closed by the same
    // line, which is what sharing this function is for.
    let value = &tail[..val_end];
    if value.is_empty() || value.starts_with(['!', '&', '*', '>', '|']) {
        return None;
    }
    let trailing = &tail[val_end..];
    Some(format!("{key_segment}{lead_ws}{to}{trailing}{eol}"))
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

/// Every surface that closes a task must stamp the same way, or drift detection
/// is on for one and off for the other. This lived in the `aplan` binary crate as
/// a private helper, so `aplan-mcp` could not call it and closed without stamping
/// at all: every task an agent closed had `APLAN018` silently disabled while its
/// file still declared its `traces` (`A1-100`, pre-public adoption evidence item 17).
/// Content fingerprints for a task's *anchored*, resolvable `traces`, for
/// `APLAN018`. Only sections that read cleanly are stamped; anything else is left
/// unverifiable rather than guessed at.
pub fn trace_fingerprints(
    task: &crate::task::Task,
    traces_root: Option<&std::path::Path>,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Some(root) = traces_root else {
        return out;
    };
    for trace in &task.traces {
        if let Some((rel, anchor)) = trace.split_once('#')
            && let Ok(text) = std::fs::read_to_string(root.join(rel))
            && let Some(fp) = crate::anchor_fingerprint(&text, anchor)
        {
            out.insert(trace.clone(), fp);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::EvidenceKind;

    fn ev(kind: EvidenceKind, reference: &str) -> Evidence {
        Evidence {
            kind,
            reference: reference.to_string(),
            captured: None,
        }
    }

    /// A closed task citing a test and a file, the pilot's shape.
    ///
    /// Built line by line: a wrapped string literal here silently turned the
    /// continuation indentation into YAML indentation, and the fixture stopped
    /// being a task file at all.
    fn closed(extra: &str) -> String {
        let mut out = [
            "---",
            "id: A",
            "title: t",
            "phase: 0",
            "status: done",
            "evidence:",
            "  - kind: test",
            "    ref: old_name",
            "  - kind: file",
            "    ref: tests/t.rs",
        ]
        .join("\n");
        out.push('\n');
        out.push_str(extra);
        out.push_str("---\n\nBody stays.\n");
        out
    }

    #[test]
    fn replace_evidence_swaps_one_citation_and_leaves_the_rest() {
        let out = replace_evidence(
            &closed(""),
            &ev(EvidenceKind::Test, "old_name"),
            &ev(EvidenceKind::Test, "new_name"),
        )
        .expect("repair");
        assert!(out.contains("ref: new_name"), "{out}");
        assert!(!out.contains("ref: old_name"), "{out}");
        // The other citation survives: this replaces one, not the block, which is
        // how a repair differs from an edit (`A1-110`).
        assert!(out.contains("ref: tests/t.rs"), "{out}");
        assert!(out.contains("status: done"), "the task stays closed");
        assert!(out.ends_with("\nBody stays.\n"), "the body is untouched");
    }

    #[test]
    fn replace_evidence_leaves_trace_fingerprints_alone() {
        // Repairing a citation is not re-reviewing a traced section.
        let out = replace_evidence(
            &closed("trace_fingerprints:\n  DOC.md#x: abc123\n"),
            &ev(EvidenceKind::Test, "old_name"),
            &ev(EvidenceKind::Test, "new_name"),
        )
        .expect("repair");
        assert!(out.contains("DOC.md#x: abc123"), "{out}");
    }

    #[test]
    fn add_evidence_extends_the_block_without_disturbing_it() {
        let out = add_evidence(&closed(""), &ev(EvidenceKind::Test, "new_proof")).expect("add");
        // Everything already recorded survives: this completes proof, it does not
        // restate it (`A1-123`).
        assert!(out.contains("ref: old_name"), "{out}");
        assert!(out.contains("ref: tests/t.rs"), "{out}");
        assert!(out.contains("ref: new_proof"), "{out}");
        assert!(out.contains("status: done"), "the task stays closed");
        assert!(out.ends_with("\nBody stays.\n"), "the body is untouched");
    }

    #[test]
    fn add_evidence_refuses_a_duplicate_and_a_live_task() {
        // Adding what is already cited would grow the block without adding proof.
        assert!(matches!(
            add_evidence(&closed(""), &ev(EvidenceKind::Test, "old_name")),
            Err(CloseError::DuplicateEvidence(_))
        ));
        // A live task's evidence is `close`'s business, as for every repair.
        let live = closed("").replace("status: done", "status: todo");
        assert!(matches!(
            add_evidence(&live, &ev(EvidenceKind::Test, "new_proof")),
            Err(CloseError::NotDone(_))
        ));
    }

    #[test]
    fn add_evidence_leaves_every_other_key_where_the_author_put_it() {
        // The same surgical property the other repairs hold.
        let file = [
            "---",
            "id: A",
            "title: t",
            "phase: 0",
            "status: done",
            "evidence:",
            "  - kind: test",
            "    ref: old_name",
            "paths:",
            "- src/**",
            "---",
            "",
            "Body stays.",
            "",
        ]
        .join("\n");
        let out = add_evidence(&file, &ev(EvidenceKind::Commit, "d213c9a")).expect("add");
        assert_eq!(
            out,
            file.replace(
                "    ref: old_name\n",
                "    ref: old_name\n  - kind: commit\n    ref: d213c9a\n"
            ),
            "the addition lands inside the block, and nothing else moves"
        );
    }

    #[test]
    fn a_repair_leaves_every_other_key_where_the_author_put_it() {
        // The property this module claims in its own docs: surgical, so the one
        // line that changed is the whole diff. The first version removed the
        // block and appended a new one, which moved `evidence:` past `paths:` and
        // `trace_fingerprints:` and buried a one-line repair in a reordering
        // (`A1-110`, audit). `restamp` had shipped with the same shape since
        // `A1-086`.
        let file = [
            "---",
            "id: A",
            "title: t",
            "phase: 0",
            "status: done",
            "evidence:",
            "  - kind: test",
            "    ref: old_name",
            "paths:",
            "- src/**",
            "trace_fingerprints:",
            "  DOC.md#x: abc123",
            "---",
            "",
            "Body stays.",
            "",
        ]
        .join("\n");

        let repaired = replace_evidence(
            &file,
            &ev(EvidenceKind::Test, "old_name"),
            &ev(EvidenceKind::Test, "new_name"),
        )
        .expect("repair");
        assert_eq!(
            repaired,
            file.replace("ref: old_name", "ref: new_name"),
            "a repair must change exactly the cited line and nothing else"
        );

        // The same for the other repair, on its own key.
        let stamped = restamp(
            &file,
            &BTreeMap::from([("DOC.md#x".into(), "def456".into())]),
        )
        .expect("restamp");
        assert_eq!(
            stamped,
            file.replace("DOC.md#x: abc123", "DOC.md#x: def456"),
            "a re-stamp must not move the block it replaces"
        );
    }

    #[test]
    fn restamp_adds_a_first_block_to_a_task_that_never_had_one() {
        // A task closed before `detect_drift` was turned on carries `traces:` and
        // no fingerprints, and re-stamping is how it gets its first. The in-place
        // replacement has to handle a key that is not there, or the migration
        // `APLAN028` walks an adopter through would write nothing (`A1-098`).
        let file = [
            "---",
            "id: A",
            "title: t",
            "phase: 0",
            "status: done",
            "traces:",
            "- DOC.md#x",
            "---",
            "",
            "Body stays.",
            "",
        ]
        .join("\n");
        let out = restamp(
            &file,
            &BTreeMap::from([("DOC.md#x".to_string(), "abc123".to_string())]),
        )
        .expect("restamp");
        assert!(out.contains("trace_fingerprints:"), "{out}");
        assert!(out.contains("DOC.md#x: abc123"), "{out}");
        assert!(out.ends_with("\nBody stays.\n"), "the body is untouched");
    }

    #[test]
    fn replace_evidence_preserves_a_bom_and_crlf_endings() {
        // The whole module is surgical on purpose: a rewrite that quietly
        // converted CRLF to LF, or dropped a BOM, would put the entire file in
        // the diff and bury the one line that changed. A repair has to hold that
        // property as much as `close` does.
        let from = ev(EvidenceKind::Test, "old_name");
        let to = ev(EvidenceKind::Test, "new_name");

        let crlf = closed("").replace('\n', "\r\n");
        let out = replace_evidence(&crlf, &from, &to).expect("repair");
        assert!(out.contains("ref: new_name"), "{out}");
        assert!(
            !out.contains('\n') || !out.replace("\r\n", "").contains('\n'),
            "every newline must still be CRLF: {out:?}"
        );

        let bom = String::from('\u{feff}') + &closed("");
        let out = replace_evidence(&bom, &from, &to).expect("repair");
        assert!(out.starts_with('\u{feff}'), "the BOM must survive");
        assert!(out.contains("ref: new_name"), "{out}");
    }

    #[test]
    fn replace_evidence_refuses_what_it_cannot_do_unambiguously() {
        let from = ev(EvidenceKind::Test, "old_name");
        let to = ev(EvidenceKind::Test, "new_name");

        // Naming both sides is what makes this auditable, so a `from` that
        // matches nothing is a mistake rather than something to guess at.
        assert!(matches!(
            replace_evidence(&closed(""), &ev(EvidenceKind::Test, "absent"), &to),
            Err(CloseError::NoSuchEvidence(_))
        ));
        // Replacing "it" is undefined when there are two of it.
        let twice = [
            "---",
            "id: A",
            "title: t",
            "phase: 0",
            "status: done",
            "evidence:",
            "  - kind: test",
            "    ref: old_name",
            "  - kind: test",
            "    ref: old_name",
            "---",
            "",
        ]
        .join("\n");
        assert!(matches!(
            replace_evidence(&twice, &from, &to),
            Err(CloseError::AmbiguousEvidence(_, 2))
        ));
        // Collapsing two citations into one would silently drop proof.
        assert!(matches!(
            replace_evidence(&closed(""), &from, &ev(EvidenceKind::File, "tests/t.rs")),
            Err(CloseError::DuplicateEvidence(_))
        ));
        // A live task's evidence is `close`'s business, not a repair's.
        let live = closed("").replace("status: done", "status: todo");
        assert!(matches!(
            replace_evidence(&live, &from, &to),
            Err(CloseError::NotDone(_))
        ));
        // Not a task file at all.
        assert!(matches!(
            replace_evidence("no frontmatter here\n", &from, &to),
            Err(CloseError::NotATask)
        ));
    }

    /// `phase` is the field nothing wrote after creation (`A1-171`).
    #[test]
    fn set_phase_changes_one_line_and_round_trips() {
        let file = "---\nid: A\ntitle: t\nphase: 0\nstatus: todo\n---\n\nBody stays.\n";
        let out = set_phase(file, 2).expect("set");
        assert_eq!(
            out,
            file.replace("phase: 0", "phase: 2"),
            "one line, no more"
        );
        assert!(out.ends_with("\nBody stays.\n"), "{out}");
    }

    /// The author's spacing, their trailing comment, and their line endings.
    #[test]
    fn set_phase_leaves_the_line_the_author_wrote_around_the_value() {
        let file = "---\r\nid: A\r\ntitle: t\r\nphase:   1   # the second wave\r\n\
                    status: todo\r\n---\r\n";
        let out = set_phase(file, 12).expect("set");
        assert!(
            out.contains("phase:   12   # the second wave\r\n"),
            "{out:?}"
        );
        assert!(!out.contains("\n\n"), "no stray bare newline: {out:?}");
    }

    /// An indented `phase:` belongs to some nested mapping, not to the task, so
    /// rewriting it would move the rollup key somewhere nothing reads. The task
    /// model denies unknown fields, so the only way to nest one is under a key
    /// that takes a mapping.
    #[test]
    fn set_phase_ignores_an_indented_phase_and_rewrites_the_real_one() {
        // The nested one *first*, so a rewriter that trimmed each line before
        // looking would find it and stop before reaching the task's own.
        let file = "---\nid: A\ntitle: t\ntrace_fingerprints:\n  phase: abc123\n\
                    phase: 0\nstatus: todo\n---\n";
        let out = set_phase(file, 2).expect("set");
        assert!(out.contains("\nphase: 2\n"), "the task's own: {out}");
        assert!(
            out.contains("  phase: abc123\n"),
            "the nested one is untouched: {out}"
        );
    }

    /// A value the token swap would mangle is refused, not mangled (`A1-171`,
    /// audit).
    ///
    /// Each of these reached the round-trip check and was reported as
    /// `Produced` -- "internal: `close` produced invalid frontmatter" -- which
    /// blames this tool for a shape the file chose. `!!int 0` became `2 0`, and
    /// a value written below its key became frontmatter with no `phase` in it at
    /// all. Both are refused where the shape is visible, and `set_status` is
    /// closed to the same thing by the same line.
    #[test]
    fn a_phase_value_this_edit_cannot_swap_is_refused_rather_than_corrupted() {
        for value in ["!!int 0", "&anchor 0", "*alias", "> \n  0", "| \n  0"] {
            let file = format!("---\nid: A\ntitle: t\nphase: {value}\nstatus: todo\n---\n");
            // Some of these are not a `u32` at all, so the parse refuses first;
            // what must never happen is a *write*.
            match set_phase(&file, 2) {
                Err(CloseError::NoScalar("phase") | CloseError::Frontmatter(_)) => {}
                other => panic!("`phase: {value}` gave {other:?}"),
            }
        }
        // The value on the line below its key, which is legal YAML and the shape
        // `NoScalar` names first.
        let below = "---\nid: A\ntitle: t\nphase:\n  0\nstatus: todo\n---\n";
        assert!(
            matches!(set_phase(below, 2), Err(CloseError::NoScalar("phase"))),
            "{:?}",
            set_phase(below, 2)
        );
    }

    /// The same shapes, on the key `close` and `drop` rewrite (`A1-171`, audit).
    #[test]
    fn a_status_value_this_edit_cannot_swap_is_refused_rather_than_corrupted() {
        let tagged = "---\nid: A\ntitle: t\nphase: 0\nstatus: !!str todo\n---\n";
        assert!(
            matches!(
                set_status(tagged, Status::Done),
                Err(CloseError::NoScalar("status"))
            ),
            "{:?}",
            set_status(tagged, Status::Done)
        );
        // And a quoted value is *not* one of them: swapping the token is exactly
        // right there, and refusing it would be a regression.
        let quoted = "---\nid: A\ntitle: t\nphase: 0\nstatus: \"todo\"\n---\n";
        let out = set_status(quoted, Status::Done).expect("a quoted scalar is swappable");
        assert!(out.contains("status: done\n"), "{out}");
    }

    /// A file with no `phase:` at all is not a task this model can load, so the
    /// loader answers before the rewriter does. Pinned because the two failures
    /// read very differently to whoever hits one.
    #[test]
    fn set_phase_on_a_file_with_no_phase_fails_at_the_parse() {
        let bare = "---\nid: A\ntitle: t\nstatus: todo\n---\n";
        assert!(
            matches!(set_phase(bare, 2), Err(CloseError::Frontmatter(_))),
            "{:?}",
            set_phase(bare, 2)
        );
    }

    #[test]
    fn set_status_changes_one_line_and_round_trips() {
        let file = "---\nid: A\ntitle: t\nphase: 0\nstatus: todo\n---\n\nBody stays.\n";
        let out = set_status(file, Status::Dropped).expect("set");
        assert!(out.contains("status: dropped"));
        assert!(out.ends_with("\nBody stays.\n"), "{out}");
        assert_eq!(out, file.replace("status: todo", "status: dropped"));
    }

    #[test]
    fn set_status_refuses_to_revive_a_terminal_task() {
        // `dropped` records a decision and `done` records evidence; rewriting
        // either would launder something the plan said.
        let dropped = "---\nid: A\ntitle: t\nphase: 0\nstatus: dropped\n---\n";
        assert!(matches!(
            set_status(dropped, Status::Todo),
            Err(CloseError::Terminal(_))
        ));
        let done = "---\nid: A\ntitle: t\nphase: 0\nstatus: done\n---\n";
        assert!(matches!(
            set_status(done, Status::Dropped),
            Err(CloseError::AlreadyDone)
        ));
    }

    #[test]
    fn set_status_keeps_spacing_and_a_trailing_comment() {
        let file = "---\nid: A\ntitle: t\nphase: 0\nstatus:   todo  # why\n---\n";
        let out = set_status(file, Status::Dropped).expect("set");
        assert!(out.contains("status:   dropped  # why"), "{out}");
    }

    #[test]
    fn set_status_refuses_a_file_with_no_status() {
        let file = "---\nid: A\ntitle: t\nphase: 0\n---\n";
        assert!(set_status(file, Status::Dropped).is_err());
    }

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

    /// A rewrite writes the form it found (`A1-179`).
    ///
    /// `mv` has kept the author's form since it was written; `dep` replaced the
    /// whole key with a block list, so a milestone written the way this tool
    /// writes them was silently reformatted by one command and not the other.
    /// Two commands, one key, two answers.
    #[test]
    fn an_inline_list_stays_inline_with_its_spacing_and_its_comment() {
        let head = "---\nid: A\ntitle: T\nphase: 1\nstatus: todo\n";
        let ids = ["B".to_string(), "Z".to_string()];

        let inline = format!("{head}blocked_by: [B]\n---\n\nbody\n");
        let out = set_blocked_by(&inline, &ids).expect("sets");
        assert!(out.contains("blocked_by: [B, Z]\n"), "{out}");
        assert!(
            !out.contains("  - B"),
            "not reformatted into a block: {out}"
        );

        // The author's spacing and their trailing comment, which `mv` keeps and
        // this dropped: a comment explaining a cut's membership is exactly the
        // kind of line a rewrite must not eat.
        let decorated = format!("{head}blocked_by:   [B]   # the first wave\n---\n\nbody\n");
        let out = set_blocked_by(&decorated, &ids).expect("sets");
        assert!(
            out.contains("blocked_by:   [B, Z]   # the first wave\n"),
            "{out}"
        );

        // A block list is still written as a block, which is what this tool
        // writes for a key it is creating.
        let block = format!("{head}blocked_by:\n  - B\n---\n\nbody\n");
        let out = set_blocked_by(&block, &ids).expect("sets");
        assert!(out.contains("blocked_by:\n  - B\n  - Z\n"), "{out}");

        // `prefer_after` is the same function and the same promise.
        let prefer = format!("{head}prefer_after: [B]\n---\n\nbody\n");
        let out = set_prefer_after(&prefer, &ids).expect("sets");
        assert!(out.contains("prefer_after: [B, Z]\n"), "{out}");

        // And the result loads, which is the guard that caught the original
        // defect this form has to keep clearing.
        let (fm, _) = split_frontmatter(&out).expect("frontmatter");
        let _: Task = serde_yaml_ng::from_str(fm).expect("loads");
    }

    /// The inline form is recognised at the top level and nowhere else
    /// (`A1-179`).
    ///
    /// Tested on the function rather than through a task file, because the task
    /// model denies unknown fields and takes no mapping whose value could be a
    /// list: there is no *valid* task whose frontmatter carries an indented
    /// `blocked_by:`. The rule is still the rule -- an indented key belongs to
    /// some nested mapping rather than to the task -- and this is the only place
    /// it can be held to.
    #[test]
    fn only_a_top_level_key_is_read_as_the_inline_list() {
        let found = inline_list_line("id: A\nblocked_by: [B]\n", "blocked_by").expect("found");
        assert_eq!(found.key, "blocked_by:");
        assert_eq!(found.spacing, " ");
        assert_eq!(found.trailing, "");

        assert!(
            inline_list_line("id: A\n  blocked_by: [B]\n", "blocked_by").is_none(),
            "an indented key belongs to something else"
        );
        assert!(
            inline_list_line("id: A\nblocked_by:\n  - B\n", "blocked_by").is_none(),
            "a block value is not the inline form"
        );
        assert!(
            inline_list_line("id: A\nblocked_by_extra: [B]\n", "blocked_by").is_none(),
            "a longer key is a different key"
        );

        // The author's spacing and comment come back as their own pieces.
        let found = inline_list_line("blocked_by:   [B]   # why\n", "blocked_by").expect("found");
        assert_eq!(found.spacing, "   ");
        assert_eq!(found.trailing, "   # why");

        // A `]` inside a quoted id is not the end of the list. Taking the first
        // one made the rest of the id "the trailing comment", so the rewrite
        // spliced it back and produced frontmatter that would not parse -- and
        // the block form had rewritten this file happily before (`A1-179`,
        // audit).
        let found = inline_list_line("blocked_by: [\"a]b\"] # why\n", "blocked_by").expect("found");
        assert_eq!(found.trailing, " # why");
        // Either quote kind, and a bracket in the comment is still the comment's.
        assert_eq!(
            inline_list_line("blocked_by: ['x]y'] # see [1]\n", "blocked_by")
                .expect("found")
                .trailing,
            " # see [1]"
        );
    }

    /// And the whole rewrite survives it, which is what the reader gets
    /// (`A1-179`, audit).
    #[test]
    fn an_id_holding_a_bracket_is_rewritten_rather_than_refused() {
        let file = "---\nid: A\ntitle: T\nphase: 1\nstatus: todo\n\
                    blocked_by: [\"a]b\"] # the first wave\n---\n\nbody\n";
        let out = set_blocked_by(file, &["X".to_string()]).expect("rewrites");
        assert!(out.contains("blocked_by: [X] # the first wave\n"), "{out}");
        let (fm, _) = split_frontmatter(&out).expect("frontmatter");
        let task: Task = serde_yaml_ng::from_str(fm).expect("loads");
        assert_eq!(task.blocked_by, vec!["X".to_string()]);
    }

    /// `blocked_by` in every shape a task file has it: absent, present with one
    /// entry, present with several, and emptied.
    #[test]
    fn blocked_by_is_added_and_removed_in_every_shape() {
        let without = "---\nid: A\ntitle: T\nphase: 1\nstatus: todo\n---\n\nbody\n";
        let one = set_blocked_by(without, &["B".to_string()]).expect("adds a first edge");
        assert!(one.contains("blocked_by:\n  - B\n"), "{one}");
        assert!(one.contains("body"), "the body survives: {one}");

        let two = set_blocked_by(&one, &["B".to_string(), "C".to_string()]).expect("adds another");
        assert!(two.contains("blocked_by:\n  - B\n  - C\n"), "{two}");

        let back = set_blocked_by(&two, &["C".to_string()]).expect("removes one");
        assert!(back.contains("blocked_by:\n  - C\n"), "{back}");
        assert!(!back.contains("- B"), "{back}");

        let none = set_blocked_by(&back, &[]).expect("removes the last");
        assert!(!none.contains("blocked_by"), "{none}");
        assert!(none.contains("id: A"), "the rest survives: {none}");

        // And each result is a task file that loads, which the function checks
        // for itself and this pins from outside.
        for text in [&one, &two, &back, &none] {
            let (fm, _) = split_frontmatter(text).expect("frontmatter");
            let _: Task = serde_yaml_ng::from_str(fm).expect("loads");
        }
    }

    /// An id that cannot survive being written as a bare YAML scalar is refused
    /// rather than written.
    ///
    /// This guard is what caught the real defect while `set_blocked_by` was being
    /// built: the list was written unindented, `replace_top_level_block` ends a
    /// block at the first line not starting with whitespace, and the old entries
    /// survived underneath the new ones. What the writer produced was not what it
    /// was asked to write, and the only way to know that is to read it back
    /// (`A1-124`).
    #[test]
    fn an_id_that_would_not_round_trip_is_refused() {
        let file = "---\nid: A\ntitle: T\nphase: 1\nstatus: todo\n---\n\nbody\n";
        // The first three do not parse at all; the last four parse into something
        // that is not what was asked for, which only reading it back can catch.
        for bad in ["a\nb", "a: b", "[a", "#c", "a #c", " a", "a "] {
            let out = set_blocked_by(file, &[bad.to_string()]);
            assert!(
                matches!(out, Err(CloseError::Produced(_))),
                "`{bad}` was written: {out:?}"
            );
        }
    }

    /// An inline list is the other shape a hand-written plan uses, and the one a
    /// milestone is written with here.
    #[test]
    fn an_inline_list_is_rewritten_as_a_block() {
        let inline = "---\nid: A\ntitle: T\nphase: 1\nstatus: todo\n\
                      blocked_by: [B, C]\n---\n\nbody\n";
        let out = set_blocked_by(inline, &["B".to_string()]).expect("edits it");
        let (fm, _) = split_frontmatter(&out).expect("frontmatter");
        let task: Task = serde_yaml_ng::from_str(fm).expect("loads");
        assert_eq!(task.blocked_by, vec!["B".to_string()], "{out}");
    }

    #[test]
    fn restamp_refuses_a_task_that_is_not_done() {
        assert!(matches!(
            restamp(TASK, &fps(&[("x", "y")])),
            Err(CloseError::NotDone(Status::Todo))
        ));
    }
    /// An unindented list is the shape this tool's own importer writes, and
    /// replacing a block skipped only indented continuations -- so the old
    /// entries stayed behind the new block and the frontmatter no longer
    /// parsed. Every `dep add` on an imported task was refused as an internal
    /// error, which is where `A1-033`'s audit found it.
    #[test]
    fn an_unindented_list_is_replaced_rather_than_left_behind() {
        let imported = "---\nid: M-B\ntitle: b\nphase: 1\nstatus: todo\n\
                        blocked_by:\n- M-A\ntags:\n- roadmap-json:not_started\n---\n\nbody\n";
        let out = set_blocked_by(imported, &["M-A".to_string(), "PHASE-00".to_string()])
            .expect("rewrites");
        let (fm, _) = split_frontmatter(&out).expect("frontmatter");
        let task: Task = serde_yaml_ng::from_str(fm).expect("the result loads");
        assert_eq!(
            task.blocked_by,
            vec!["M-A".to_string(), "PHASE-00".to_string()]
        );
        // The neighbouring key, itself an unindented list, is untouched.
        assert_eq!(task.tags, vec!["roadmap-json:not_started".to_string()]);
        assert_eq!(
            out.matches("M-A").count(),
            1,
            "the old entry was replaced, not duplicated: {out}"
        );
    }

    /// The same shape, emptied: an unindented list removed entirely leaves no
    /// orphan behind it.
    #[test]
    fn an_unindented_list_can_be_emptied() {
        let imported = "---\nid: M-B\ntitle: b\nphase: 1\nstatus: todo\n\
                        blocked_by:\n- M-A\n- M-C\ntags:\n- x\n---\n\nbody\n";
        let out = set_blocked_by(imported, &[]).expect("rewrites");
        let (fm, _) = split_frontmatter(&out).expect("frontmatter");
        let task: Task = serde_yaml_ng::from_str(fm).expect("the result loads");
        assert!(task.blocked_by.is_empty(), "{out}");
        assert_eq!(task.tags, vec!["x".to_string()], "{out}");
    }
}
