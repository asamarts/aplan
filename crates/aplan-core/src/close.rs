//! `aplan close`: set a task `done` and attach evidence, in place.
//!
//! Evidence should be easy to record correctly: hand-typing a SHA
//! into YAML is the tax that stops people closing tasks honestly. `close` makes it
//! a one-liner (and can derive the commit for free). It is the first thing that
//! *writes* a task file, so the rewrite is deliberately surgical: it changes only
//! the `status:` line, appends an `evidence:` block, and adds or replaces a
//! `trace_fingerprints:` block under drift detection. The caller then uses the
//! same surgical block editor for `evidence_fingerprints:` after the evidence has
//! resolved, leaving the rest of the frontmatter and the whole markdown body
//! byte-for-byte untouched. A round-trip
//! through the typed model would reformat everything (`1` -> `1.0`, flow lists
//! to block lists) and bury the real change in noise.
//!
//! [`restamp`] is the APLAN018/APLAN054 "clear": its caller replaces a `done`
//! task's trace and evidence fingerprint blocks after both were re-reviewed.

use std::collections::BTreeMap;

use crate::diagnostic::Repair;
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
    /// The citation named appears more than once, so naming it does not pick one.
    ///
    /// Worded for the operation-neutral case (`A1-361`, audit): this is raised by
    /// a removal as well as a replacement, and said "which one to replace" to
    /// somebody who had asked to remove. One message serving two operations is
    /// the defect this increment kept finding in the code; it is the same defect
    /// in the words.
    #[error("this task cites `{0}` {1} times, so naming it does not pick one of them")]
    AmbiguousEvidence(String, usize),
    /// The replacement is already on the task, so the repair would collapse two
    /// citations into one and silently drop proof.
    #[error("this task already cites `{0}`")]
    DuplicateEvidence(String),
    #[error("the task is `{0}`, a terminal state: revive it (change its status) before closing")]
    Terminal(Status),
    #[error(
        "this live task already has an `evidence:` field; `close` only records evidence when the \
         field is absent. Review why the field is present and whether any citations still prove \
         the task; reconcile or remove that field, then retry"
    )]
    HasEvidence,
    /// The source mapping already owns the release record's key, but its null
    /// value deserialises to `None`. Appending the derived record would create
    /// a duplicate key rather than fill that authored field (`A1-724`).
    #[error(
        "this release already has a `version` field with no value, while the plan derives \
         `{derived}`. Record the intended version or remove the empty field, then retry"
    )]
    EmptyVersion {
        /// What close would record if the field were genuinely absent.
        derived: String,
    },
    /// The node already records a version, and it is not the one the plan now
    /// derives (`A1-339`). Naming both is what makes the disagreement
    /// actionable: one of the two is wrong and this tool cannot tell which.
    #[error(
        "this release already records `version: {recorded}`, and the plan now derives \
         `{derived}`. One of the two is wrong, and which is not this tool's to decide"
    )]
    VersionDisagrees {
        /// What the node already says.
        recorded: String,
        /// What the plan derives today.
        derived: String,
    },
    #[error(
        "no top-level `{0}:` line this edit can rewrite: it is absent, or its value is a shape \
         (written below the key, or carrying a YAML tag, anchor or alias) that would have to be \
         guessed at -- change it by hand"
    )]
    NoScalar(&'static str),
    #[error(
        "the top-level `{0}:` value is not one unambiguous scalar sequence matching the loaded \
         task: duplicate members, reordered retained members, multiline flow values and \
         structural YAML cannot be edited as a source-preserving delta -- simplify that field by \
         hand, then retry"
    )]
    NoSequenceDelta(&'static str),
    #[error(
        "the top-level `{0}:` value is not one source-local ordinary string mapping: YAML \
         anchors, aliases and tags can affect source outside it, while multiline values and \
         structural keys cannot be replaced as one independent block -- simplify that field by \
         hand, then retry"
    )]
    NoMappingReplacement(&'static str),
    #[error(
        "the top-level `evidence:` value is not one source-local ordinary citation sequence \
         matching the loaded task: YAML anchors, aliases and tags can affect source outside it, \
         while flow collections and multiline or structural citation fields cannot be replaced \
         as one independent block -- simplify that field by hand, then retry"
    )]
    NoEvidenceReplacement,
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
/// Only [`CloseError::AlreadyDone`] has one today. It is the refusal for which
/// a separate mutation can perform exactly what the caller intended: the
/// message used to end *edit the rest by hand*, and
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
        _ => None,
    }
}

pub fn close(
    file: &str,
    evidence: &[Evidence],
    fingerprints: &BTreeMap<String, String>,
    version: Option<&str>,
) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;

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
    if top_level_mapping_has_key(frontmatter, "evidence") {
        return Err(CloseError::HasEvidence);
    }

    // Presence is a source fact, not the same thing as a typed optional value.
    // `version: null` and its alternate-key spellings load as `None`; appending
    // the derived record beside that authored key creates a duplicate mapping
    // entry and reaches the internal round-trip guard. Refuse the input before
    // constructing a candidate, naming the value close would otherwise record.
    let version_was_present = top_level_mapping_has_key(frontmatter, "version");

    // **A record already there and disagreeing is refused, naming both**
    // (`A1-339`). A reopened release, or one edited by hand: overwriting would
    // erase a number somebody wrote down, and keeping the old one silently would
    // leave the node claiming a version the plan no longer derives. Neither is a
    // record, so the closure stops and says which two numbers it is between.
    if let (Some(recorded), Some(derived)) = (task.version.as_deref(), version)
        && recorded != derived
    {
        return Err(CloseError::VersionDisagrees {
            recorded: recorded.to_string(),
            derived: derived.to_string(),
        });
    }
    if let Some(derived) = version
        && task.version.is_none()
        && version_was_present
    {
        return Err(CloseError::EmptyVersion {
            derived: derived.to_string(),
        });
    }

    let nl = newline(file, bom);
    // Already present and agreeing: leave the file's own line alone rather than
    // writing a second key, which would be a duplicate-key document.
    let version = version.filter(|_| !version_was_present);
    let fingerprints_were_present = top_level_mapping_has_key(frontmatter, "trace_fingerprints");
    let fingerprints_block = fingerprint_block("trace_fingerprints", fingerprints, nl)?;
    let with_status = set_status_in(frontmatter, Status::Done)?;
    // A live task may validly carry stamps from an earlier closure. Closure is
    // authoritative for what `done` claims now, so replace that mapping where
    // the author put it (or remove it when no trace resolves) rather than
    // appending a second key. The absent-key path deliberately remains an
    // append, preserving `close`'s established frontmatter order.
    let with_fingerprints = replace_fingerprint_mapping(
        &with_status,
        "trace_fingerprints",
        &fingerprints_block,
        &task.trace_fingerprints,
        nl,
        false,
    )?;
    let new_frontmatter = format!(
        "{}{}{}{}",
        with_fingerprints,
        version.map_or_else(String::new, |v| format!("version: {v}{nl}")),
        evidence_block(evidence, nl)?,
        if fingerprints_were_present {
            String::new()
        } else {
            fingerprints_block
        },
    );

    // Never write frontmatter that would not load: parse it back, and confirm the
    // status, the *exact* evidence, and the fingerprints round-tripped (value
    // equality, not a matching count, so a mis-serialised reference cannot slip).
    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != Status::Done
        || parsed.evidence.as_slice() != evidence
        || &parsed.trace_fingerprints != fingerprints
        || (version.is_some() && parsed.version.as_deref() != version)
    {
        return Err(CloseError::Produced(
            "the status, evidence, version or fingerprints did not round-trip".to_string(),
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
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
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

/// Set or clear durable `owner`, leaving everything else untouched (`A1-359`).
///
/// The same surgical shape as [`set_status`], and for the same reason: the task
/// file is a document somebody wrote, and rewriting it through serde would
/// reflow comments, reorder keys and requote scalars that were fine. So the
/// value token is swapped in place where the key exists, a line is inserted
/// where it does not, and the line goes away entirely when responsibility is
/// cleared -- because `owner` is `Option<String>` with
/// `skip_serializing_if`, so absent is how the model spells unassigned, and
/// `owner:` with nothing after it is a `null` the schema refuses (the shape
/// `A1-363` found one layer down in `blocked_by`).
///
/// **No terminal check here**, unlike `set_status`. Refusing to reopen a closed
/// task is a model invariant (ADR-0020); `owner` on a closed task is ordinary
/// and this plan has 228 of them. Whether assigning responsibility for finished
/// work makes sense is a question for the caller rather than for a text
/// rewriter. Active claims are separate local state (`A1-531`).
pub fn set_owner(file: &str, to: Option<&str>) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    // Parsed before anything is rewritten, so a file that does not load is
    // reported as that rather than as a failed rewrite.
    let task: Task =
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;

    let nl = newline(file, bom);
    let new_frontmatter = set_owner_in(frontmatter, to, task.owner.is_some(), nl)?;
    // The same read-back `close` and `set_status` do. A rewrite that cannot be
    // parsed again is a corrupted file, and the one that matters here is a name
    // that needs quoting: this plan already carries an `owner` holding an ANSI
    // escape, from `A1-290`'s fixture.
    let parsed = reparse(&new_frontmatter)?;
    if parsed.owner.as_deref() != to {
        return Err(CloseError::Produced(format!(
            "`owner` did not round-trip: asked for {to:?} and read back {:?}",
            parsed.owner
        )));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// Set or clear a parked task's RFC 3339 revisit instant without reformatting
/// the rest of its frontmatter or body.
///
/// Status semantics belong to the guarded caller: this primitive only makes an
/// optional scalar round-trip exactly, just as [`set_owner`] does.
///
/// # Errors
///
/// [`CloseError::NotATask`] or [`CloseError::Frontmatter`] for unreadable input,
/// [`CloseError::NoScalar`] for a top-level shape that cannot be edited safely,
/// and [`CloseError::Produced`] when the requested value does not round-trip.
pub fn set_revisit_at(file: &str, to: Option<&str>) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    let nl = newline(file, bom);
    let new_frontmatter = set_optional_scalar_in(
        frontmatter,
        "revisit_at",
        to,
        "status",
        task.revisit_at.is_some(),
        nl,
    )?;
    let parsed = reparse(&new_frontmatter)?;
    if parsed.revisit_at.as_deref() != to {
        return Err(CloseError::Produced(format!(
            "`revisit_at` did not round-trip: asked for {to:?} and read back {:?}",
            parsed.revisit_at
        )));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

fn set_optional_scalar_in(
    frontmatter: &str,
    key: &'static str,
    to: Option<&str>,
    insert_after: &str,
    loaded_present: bool,
    nl: &str,
) -> Result<String, CloseError> {
    let line_to_change = frontmatter
        .split_inclusive('\n')
        .find(|line| is_ordinary_scalar_key_line(line, key));
    // A quoted key, explicit-key form or value supplied through structural
    // YAML is present in the loaded mapping but has no independent ordinary
    // source line this helper owns. Treating it as absent makes a clear a
    // silent no-op and makes an assignment append a duplicate key; both reach
    // the round-trip guard only after claiming the edit was possible.
    if line_to_change.is_none() && (loaded_present || top_level_mapping_has_key(frontmatter, key)) {
        return Err(CloseError::NoScalar(key));
    }
    if line_to_change.is_some() && !has_single_source_local_scalar_line(frontmatter, key) {
        return Err(CloseError::NoScalar(key));
    }
    let Some(value) = to else {
        return Ok(frontmatter
            .split_inclusive('\n')
            .filter(|line| !is_ordinary_scalar_key_line(line, key))
            .collect());
    };
    let value = yaml_scalar(value);
    let mut out = String::with_capacity(frontmatter.len() + value.len() + key.len() + 4);
    let mut done = false;
    for line in frontmatter.split_inclusive('\n') {
        if !done && is_ordinary_scalar_key_line(line, key) {
            if let Some(rewritten) = rewrite_scalar_line(line, key, &value) {
                out.push_str(&rewritten);
            } else {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{key}: {value}{nl}"));
            }
            done = true;
            continue;
        }
        out.push_str(line);
        if !done && line_to_change.is_none() && is_scalar_key(line, insert_after) {
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{key}: {value}{nl}"));
            done = true;
        }
    }
    if !done {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{key}: {value}{nl}"));
    }
    Ok(out)
}

/// Change a task's stable id, leaving every other field and the body alone.
///
/// This is the scalar half of `aplan rekey`. References to the id live in other
/// files and are deliberately handled by the caller, so all of them can be
/// judged and saved as one change. As with the other surgical editors in this
/// module, the result is parsed back and the requested value must survive
/// exactly before any caller may write it.
///
/// # Errors
///
/// [`CloseError::NotATask`] without frontmatter, [`CloseError::Frontmatter`]
/// when it does not parse, [`CloseError::NoScalar`] when `id` is not a
/// replaceable top-level scalar, and [`CloseError::Produced`] when the rewrite
/// does not round-trip.
pub fn set_id(file: &str, to: &str) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let _: Task =
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if !has_single_source_local_scalar_line(frontmatter, "id") {
        return Err(CloseError::NoScalar("id"));
    }

    let nl = newline(file, bom);
    let value = yaml_scalar(to);
    let mut out = String::with_capacity(frontmatter.len() + value.len());
    let mut changed = false;
    for line in frontmatter.split_inclusive('\n') {
        if !changed && let Some(rewritten) = rewrite_scalar_line(line, "id", &value) {
            out.push_str(&rewritten);
            changed = true;
            continue;
        }
        out.push_str(line);
    }
    if !changed {
        return Err(CloseError::NoScalar("id"));
    }
    let parsed = reparse(&out)?;
    if parsed.id != to {
        return Err(CloseError::Produced(format!(
            "`id` did not round-trip: asked for {to:?} and read back {:?}",
            parsed.id
        )));
    }
    Ok(format!("{bom}---{nl}{out}---{nl}{body}"))
}

/// `owner` set, replaced or removed in `frontmatter`.
///
/// Inserted after `status:` when the key is absent, because that is where every
/// file this tool has written puts it and a diff that moves a key reads as a
/// bigger change than it is. Appended when there is no `status:` either, which
/// is a file `check` already rejects.
fn set_owner_in(
    frontmatter: &str,
    to: Option<&str>,
    loaded_present: bool,
    nl: &str,
) -> Result<String, CloseError> {
    set_optional_scalar_in(frontmatter, "owner", to, "status", loaded_present, nl)
}

/// Whether `line` declares `key` at the top level with no value on it.
///
/// `rewrite_scalar_line` returns `None` for this shape deliberately -- there is
/// no token to swap -- so removing a key has to recognise it separately, or
/// clearing responsibility would leave `owner:` behind and the schema would read it as
/// `null`.
fn is_bare_key(line: &str, key: &str) -> bool {
    let content = line.trim_end_matches(['\r', '\n']);
    let Some(after) = content.strip_prefix(key) else {
        return false;
    };
    let after = after.trim_start_matches([' ', '\t']);
    after
        .strip_prefix(':')
        .is_some_and(|rest| rest.trim().is_empty())
}

/// Whether `line` begins one ordinary literal top-level key, independent of
/// the shape or extent of its value.
fn is_ordinary_scalar_key_line(line: &str, key: &str) -> bool {
    let content = line.trim_end_matches(['\r', '\n']);
    content
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
}

/// Whether exactly one ordinary key line carries the complete parsed value.
///
/// Parsing erases whether a plain scalar continued on following physical
/// lines. Comparing the token on this line with the mapping value recovers that
/// boundary: `id: T-\n  old` reads as `T- old`, which is not the `T-` token a
/// line-local editor would otherwise replace. A bare key is local only when it
/// really maps to null; `owner:\n  ada` is a multiline string, not an empty
/// field. Requiring one source line also prevents a duplicate literal key from
/// being mistaken for one independently owned scalar.
fn has_single_source_local_scalar_line(frontmatter: &str, key: &str) -> bool {
    let mut lines = frontmatter
        .split_inclusive('\n')
        .filter(|line| is_ordinary_scalar_key_line(line, key));
    let Some(line) = lines.next() else {
        return false;
    };
    if lines.next().is_some() {
        return false;
    }

    let Ok(document) = yaml_compat::from_str::<yaml_compat::Value>(frontmatter) else {
        return false;
    };
    let Some(mapped) = document
        .as_mapping()
        .and_then(|mapping| mapping.get(yaml_compat::Value::String(key.to_string())))
    else {
        return false;
    };
    if is_bare_key(line, key) {
        return matches!(mapped, yaml_compat::Value::Null);
    }
    let Some(parts) = scalar_line_parts(line, key) else {
        return false;
    };
    yaml_compat::from_str::<yaml_compat::Value>(parts.value).is_ok_and(|inline| &inline == mapped)
}

/// Whether `line` is the top-level `key:` line with a value this can sit after.
fn is_scalar_key(line: &str, key: &str) -> bool {
    rewrite_scalar_line(line, key, "x").is_some()
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
    set_id_list(file, "blocked_by", blocked_by, SequenceEdit::Delta, |t| {
        &t.blocked_by
    })
}

/// Rekey `blocked_by` members one for one, retaining each inline member's own
/// spacing and quote style where the document uses a flow sequence.
///
/// # Errors
///
/// Whatever [`set_blocked_by`] returns, for the same reasons.
pub fn rekey_blocked_by(file: &str, blocked_by: &[String]) -> Result<String, CloseError> {
    set_id_list(file, "blocked_by", blocked_by, SequenceEdit::Rekey, |t| {
        &t.blocked_by
    })
}

/// Move a task to another phase, rewriting one line (`A1-171`).
///
/// `phase` is the one field every task carries that no command wrote after
/// creation: `add --phase` set it once and nothing changed it, so changing the
/// reporting coordinate ended in a hand-edit of the files this tool claims to
/// own. Rollups read it (`summary` groups by it), so a plan could express a
/// change of cut and not a change of phase.
///
/// Surgical, like [`set_status`] and for the same reason: the value token is
/// replaced and the key's spacing, any trailing comment and the line ending are
/// the author's. Nothing about a phase can be illegal -- no rule reads its
/// value, and the schema says it creates no membership, dependency, readiness
/// gate, or execution order -- so there is no guard here beyond proving the
/// file still parses and really says what was asked.
///
/// # Errors
/// [`CloseError::NotATask`] without frontmatter, [`CloseError::Frontmatter`]
/// when it does not parse, [`CloseError::NoScalar`] with no top-level `phase:`,
/// and [`CloseError::Produced`] when the rewrite does not round-trip.
pub fn set_phase(file: &str, to: u32) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let _: Task =
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if !has_single_source_local_scalar_line(frontmatter, "phase") {
        return Err(CloseError::NoScalar("phase"));
    }

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
    set_id_list(
        file,
        "prefer_after",
        prefer_after,
        SequenceEdit::Delta,
        |t| &t.prefer_after,
    )
}

/// Rekey `prefer_after` members without rebuilding an inline flow sequence.
///
/// # Errors
///
/// Whatever [`set_prefer_after`] returns, for the same reasons.
pub fn rekey_prefer_after(file: &str, prefer_after: &[String]) -> Result<String, CloseError> {
    set_id_list(
        file,
        "prefer_after",
        prefer_after,
        SequenceEdit::Rekey,
        |t| &t.prefer_after,
    )
}

/// The same surgical list edit for `ratified_by`.
///
/// # Errors
///
/// Whatever [`set_blocked_by`] returns, for the same reasons.
pub fn set_ratified_by(file: &str, ratified_by: &[String]) -> Result<String, CloseError> {
    set_id_list(file, "ratified_by", ratified_by, SequenceEdit::Rekey, |t| {
        &t.ratified_by
    })
}

/// The same surgical list edit for milestone containment.
///
/// # Errors
///
/// Whatever [`set_blocked_by`] returns, for the same reasons.
pub fn set_contains(file: &str, contains: &[String]) -> Result<String, CloseError> {
    set_id_list(file, "contains", contains, SequenceEdit::Rekey, |t| {
        &t.contains
    })
}

/// The same for `tags`, the vocabulary (`A1-213`).
///
/// Through `set_id_list` despite the name: what that function does is replace a
/// top-level list of scalars while keeping the form the file already used, and
/// nothing in it is about ids. A second copy for tags would be a second place
/// for the flow-versus-block rule to be got wrong, which is the reason it was
/// one function for two edges in the first place (`A1-150`).
///
/// # Errors
///
/// Whatever [`set_blocked_by`] returns, for the same reasons.
pub fn set_tags(file: &str, tags: &[String]) -> Result<String, CloseError> {
    set_id_list(file, "tags", tags, SequenceEdit::Preserve, |t| &t.tags)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SequenceEdit {
    /// Add or remove members while retaining the source of every member that
    /// survives. Used only by dependency and preference edges.
    Delta,
    /// Replace members one for one, retaining each member's source trivia.
    Rekey,
    /// Preserve a tag edit as a delta when cardinality changes and as a rekey
    /// when it does not. Tags have both command shapes on one setter.
    Preserve,
}

/// One list entry, quoted exactly as far as YAML needs and no further.
///
/// The writer used to emit the value raw, which is right for every id this plan
/// actually carries and wrong for anything that opens a YAML construct: `*` is
/// an alias, `&` an anchor, `!` a tag, and none of them survives being written
/// literally in either the flow form or the block one. `add` never had the
/// problem because it serialises the whole task, so the asymmetry was between
/// two commands rather than between two values (`A1-213`, audit).
///
/// Serialising the scalar is what keeps them agreeing. It leaves ordinary ids
/// alone, so the author's `[cli, backend]` is still written back unquoted.
fn yaml_scalar(value: &str) -> String {
    let rendered = yaml_compat::to_string(value).map_or_else(
        // Unreachable for a string, and a raw value is what the caller asked
        // for: the round-trip guard below is what refuses to write it.
        |_| value.to_string(),
        |rendered| rendered.trim_end_matches('\n').to_string(),
    );
    if !rendered.contains('\n') {
        return rendered;
    }
    // A value carrying a newline is rendered as a `|-` block, which is a
    // perfectly good scalar and cannot be embedded in one list line. YAML's
    // double-quoted form escapes the same characters JSON does, so the JSON
    // rendering is one -- and it keeps a value nobody should type from becoming
    // an error that blames the tool for it.
    serde_json::to_string(value).unwrap_or(rendered)
}

fn scalar_like(original: &str, value: &str) -> String {
    if original.starts_with('"') {
        return serde_json::to_string(value).unwrap_or_else(|_| yaml_scalar(value));
    }
    if original.starts_with('\'') {
        return format!("'{}'", value.replace('\'', "''"));
    }
    yaml_scalar(value)
}

/// Render one scalar for a flow sequence, retaining an existing quote style
/// where possible and adding quotes when the plain YAML rendering would become
/// flow syntax. A comma is ordinary text in a block scalar but separates two
/// members inside `[...]`; asking the parser keeps that distinction out of a
/// hand-maintained punctuation table.
fn flow_scalar_like(original: &str, value: &str) -> String {
    let rendered = scalar_like(original, value);
    let probe = format!("[{rendered}]");
    if yaml_compat::from_str::<Vec<String>>(&probe).is_ok_and(|values| values == [value]) {
        rendered
    } else {
        serde_json::to_string(value).unwrap_or(rendered)
    }
}

/// Rewrite the members of a one-line flow sequence without moving any byte
/// around them.
///
/// `rekey` changes values one for one, so preserving the spaces and quote style
/// around those scalars is part of its surgical-write contract. Dependency
/// cardinality changes use the separate delta editor below.
fn rewrite_inline_members(
    frontmatter: &str,
    key: &str,
    before: &[String],
    after: &[String],
    nl: &str,
) -> Option<String> {
    if before.len() != after.len() {
        return None;
    }
    let line = frontmatter.lines().find(|line| {
        line.strip_prefix(key)
            .and_then(|rest| rest.trim_start_matches([' ', '\t']).strip_prefix(':'))
            .is_some_and(|rest| rest.trim_start().starts_with('['))
    })?;
    let open = line.find('[')?;
    let value = &line[open..];
    let close = closing_bracket(value)?;
    let inside = &value[1..close];
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut chars = inside.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == ',' => {
                pieces.push((&inside[start..index], Some(',')));
                start = index + 1;
            }
            _ => {}
        }
    }
    pieces.push((&inside[start..], None));
    if pieces.len() != before.len() {
        return None;
    }

    let mut rewritten = String::new();
    for ((piece, comma), (was, now)) in pieces.into_iter().zip(before.iter().zip(after)) {
        let lead = piece.len() - piece.trim_start_matches([' ', '\t']).len();
        let end = piece.trim_end_matches([' ', '\t']).len();
        let token = &piece[lead..end];
        let parsed: String = yaml_compat::from_str(token).ok()?;
        if parsed != *was {
            return None;
        }
        rewritten.push_str(&piece[..lead]);
        if was == now {
            rewritten.push_str(token);
        } else {
            rewritten.push_str(&flow_scalar_like(token, now));
        }
        rewritten.push_str(&piece[end..]);
        if comma.is_some() {
            rewritten.push(',');
        }
    }
    Some(format!(
        "{}[{}]{}{nl}",
        &line[..open],
        rewritten,
        &value[close + 1..]
    ))
}

/// Rewrite a block sequence one scalar at a time, retaining every byte around
/// its members.
///
/// This is the block-form counterpart to [`rewrite_inline_members`]. Rebuilding
/// the list was once the add/remove path, but is not correct for `rekey`'s
/// one-for-one migration: it deleted the comment that explained an edge even
/// though the parsed relationship still round-tripped (`A1-410`). Dependency
/// cardinality changes now use the source-delta counterpart below.
fn rewrite_block_members(
    frontmatter: &str,
    key: &str,
    before: &[String],
    after: &[String],
) -> Option<String> {
    if before.len() != after.len() {
        return None;
    }
    let mut out = String::with_capacity(frontmatter.len());
    let mut in_block = false;
    let mut found = false;
    let mut member = 0usize;
    for line in frontmatter.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let eol = &line[content.len()..];
        if !in_block {
            let is_key = content.strip_prefix(key).and_then(|rest| {
                let rest = rest.trim_start_matches([' ', '\t']).strip_prefix(':')?;
                let value = rest.trim_start_matches([' ', '\t']);
                (value.is_empty() || value.starts_with('#')).then_some(())
            });
            if is_key.is_some() {
                in_block = true;
                found = true;
            }
            out.push_str(line);
            continue;
        }

        let trimmed = content.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push_str(line);
            continue;
        }
        let Some(after_dash) = trimmed.strip_prefix('-') else {
            // A new top-level key ends the sequence. An indented non-member is
            // a continuation or nested structure this scalar editor declines.
            if trimmed.len() == content.len() {
                in_block = false;
                out.push_str(line);
                continue;
            }
            return None;
        };
        if !after_dash.is_empty() && !after_dash.starts_with([' ', '\t']) {
            return None;
        }
        let spacing_len = after_dash.len() - after_dash.trim_start_matches([' ', '\t']).len();
        let (spacing, value) = after_dash.split_at(spacing_len);
        let end = inline_scalar_end(value)?;
        let token = &value[..end];
        let parsed: String = yaml_compat::from_str(token).ok()?;
        let was = before.get(member)?;
        let now = after.get(member)?;
        if parsed != *was {
            return None;
        }
        let prefix_len = content.len() - after_dash.len();
        out.push_str(&content[..prefix_len]);
        out.push_str(spacing);
        if was == now {
            out.push_str(token);
        } else {
            out.push_str(&scalar_like(token, now));
        }
        out.push_str(&value[end..]);
        out.push_str(eol);
        member += 1;
    }
    (found && member == before.len()).then_some(out)
}

/// One scalar as it appeared between two commas in a flow sequence.
struct FlowMember<'a> {
    whole: &'a str,
    leading: &'a str,
    token: &'a str,
    trailing: &'a str,
    value: String,
}

/// Split a one-line flow sequence without treating commas inside quotes as
/// separators. Structural values are refused by `inline_scalar_end`.
fn flow_members(inside: &str) -> Option<Vec<FlowMember<'_>>> {
    if inside.trim().is_empty() {
        return Some(Vec::new());
    }

    let mut raw = Vec::new();
    let mut start = 0;
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut chars = inside.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == ',' => {
                raw.push(&inside[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if !matches!(quote, FlowQuote::None) {
        return None;
    }
    raw.push(&inside[start..]);

    raw.into_iter()
        .map(|whole| {
            let lead = whole.len() - whole.trim_start_matches([' ', '\t']).len();
            let tail = whole.trim_end_matches([' ', '\t']).len();
            let token = &whole[lead..tail];
            let end = inline_scalar_end(token)?;
            if end != token.len() {
                return None;
            }
            Some(FlowMember {
                whole,
                leading: &whole[..lead],
                token,
                trailing: &whole[tail..],
                value: yaml_compat::from_str(token).ok()?,
            })
        })
        .collect()
}

fn unique_members(values: &[String]) -> bool {
    values
        .iter()
        .enumerate()
        .all(|(index, value)| !values[index + 1..].contains(value))
}

/// A delta can add and remove members, but it cannot express a reorder as a
/// surgical edit. Duplicates make the source member to retain ambiguous.
fn is_scalar_sequence_delta(before: &[String], after: &[String]) -> bool {
    if !unique_members(before) || !unique_members(after) {
        return false;
    }
    let mut previous = None;
    for value in after {
        let Some(index) = before.iter().position(|candidate| candidate == value) else {
            continue;
        };
        if previous.is_some_and(|previous| index <= previous) {
            return false;
        }
        previous = Some(index);
    }
    true
}

/// Apply an add/remove delta to a one-line flow sequence while retaining the
/// complete source slice of every surviving member.
fn rewrite_inline_delta(
    frontmatter: &str,
    key: &str,
    before: &[String],
    after: &[String],
    nl: &str,
) -> Option<String> {
    if !is_scalar_sequence_delta(before, after) {
        return None;
    }

    let mut target = None;
    for (index, line) in frontmatter.split_inclusive('\n').enumerate() {
        let content = line.trim_end_matches(['\r', '\n']);
        let Some(after_key) = content.strip_prefix(key) else {
            continue;
        };
        let after_ws = after_key.trim_start_matches([' ', '\t']);
        let Some(after_colon) = after_ws.strip_prefix(':') else {
            continue;
        };
        let spacing = after_colon.len() - after_colon.trim_start_matches([' ', '\t']).len();
        let value = &after_colon[spacing..];
        if !value.starts_with('[') {
            continue;
        }
        let close = closing_bracket(value)?;
        let suffix = &value[close + 1..];
        if !suffix.trim().is_empty() && !suffix.trim_start().starts_with('#') {
            return None;
        }
        target = Some((index, content, value, close));
        break;
    }
    let (target_index, content, value, close) = target?;
    let members = flow_members(&value[1..close])?;
    if members.iter().map(|member| &member.value).ne(before.iter()) {
        return None;
    }

    let replacement = if after.is_empty() {
        let suffix = &value[close + 1..];
        suffix
            .find('#')
            .map_or_else(String::new, |comment| format!("{}{nl}", &suffix[comment..]))
    } else {
        let mut inside = String::new();
        let mut last_source = None;
        for (output_index, value) in after.iter().enumerate() {
            if output_index > 0 {
                inside.push(',');
            }
            if let Some(source_index) = before.iter().position(|candidate| candidate == value) {
                inside.push_str(members[source_index].whole);
                last_source = Some(source_index);
                continue;
            }

            let next_source = after[output_index + 1..]
                .iter()
                .find_map(|candidate| before.iter().position(|value| value == candidate));
            let reference = last_source
                .or(next_source)
                .or_else(|| (!members.is_empty()).then_some(0));
            if let Some(reference) = reference {
                let member = &members[reference];
                let leading = if output_index == 0 {
                    members
                        .first()
                        .map_or(member.leading, |first| first.leading)
                } else if members.len() == 1 {
                    " "
                } else if reference == 0 && members.len() > 1 {
                    members[1].leading
                } else {
                    member.leading
                };
                inside.push_str(leading);
                inside.push_str(&flow_scalar_like(member.token, value));
                inside.push_str(member.trailing);
            } else {
                if output_index > 0 {
                    inside.push(' ');
                }
                inside.push_str(&yaml_scalar(value));
            }
        }
        format!(
            "{}[{}]{}{nl}",
            &content[..content.len() - value.len()],
            inside,
            &value[close + 1..]
        )
    };

    let mut out = String::with_capacity(frontmatter.len() + replacement.len());
    for (index, line) in frontmatter.split_inclusive('\n').enumerate() {
        if index == target_index {
            out.push_str(&replacement);
        } else {
            out.push_str(line);
        }
    }
    Some(out)
}

struct BlockMember<'a> {
    line_index: usize,
    whole: &'a str,
    prefix: &'a str,
    token: &'a str,
    eol: &'a str,
    value: String,
}

fn block_member(line_index: usize, line: &str) -> Option<BlockMember<'_>> {
    let content = line.trim_end_matches(['\r', '\n']);
    let eol = &line[content.len()..];
    let trimmed = content.trim_start_matches([' ', '\t']);
    let after_dash = trimmed.strip_prefix('-')?;
    if !after_dash.is_empty() && !after_dash.starts_with([' ', '\t']) {
        return None;
    }
    let spacing = after_dash.len() - after_dash.trim_start_matches([' ', '\t']).len();
    let value = &after_dash[spacing..];
    let end = inline_scalar_end(value)?;
    let token = &value[..end];
    let prefix_len = content.len() - after_dash.len() + spacing;
    Some(BlockMember {
        line_index,
        whole: line,
        prefix: &content[..prefix_len],
        token,
        eol,
        value: yaml_compat::from_str(token).ok()?,
    })
}

fn write_block_member(out: &mut String, source: &BlockMember<'_>, value: &str, nl: &str) {
    out.push_str(source.prefix);
    out.push_str(&scalar_like(source.token, value));
    out.push_str(if source.eol.is_empty() {
        nl
    } else {
        source.eol
    });
}

struct BlockSequence<'a> {
    key_index: usize,
    end: usize,
    key_comment: Option<String>,
    members: Vec<BlockMember<'a>>,
}

fn block_sequence<'a>(
    lines: &[&'a str],
    key: &str,
    before: &[String],
) -> Option<BlockSequence<'a>> {
    let mut key_index = None;
    let mut key_comment = None;
    for (index, line) in lines.iter().enumerate() {
        let content = line.trim_end_matches(['\r', '\n']);
        let Some(rest) = content.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim_start_matches([' ', '\t']);
        let Some(after_colon) = rest.strip_prefix(':') else {
            continue;
        };
        let value = after_colon.trim_start_matches([' ', '\t']);
        if !value.is_empty() && !value.starts_with('#') {
            return None;
        }
        key_index = Some(index);
        key_comment = value.strip_prefix('#').map(|comment| format!("#{comment}"));
        break;
    }
    let key_index = key_index?;

    let mut end = lines.len();
    let mut members = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(key_index + 1) {
        let content = line.trim_end_matches(['\r', '\n']);
        let trimmed = content.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "-" || trimmed.starts_with("- ") || trimmed.starts_with("-\t") {
            members.push(block_member(index, line)?);
            continue;
        }
        if trimmed.len() == content.len() {
            end = index;
            break;
        }
        return None;
    }
    if members.iter().map(|member| &member.value).ne(before.iter()) {
        return None;
    }
    Some(BlockSequence {
        key_index,
        end,
        key_comment,
        members,
    })
}

/// Apply the same source delta to a block sequence. Comments and blank lines
/// remain at their exact source offsets; only removed member lines disappear.
fn rewrite_block_delta(
    frontmatter: &str,
    key: &str,
    before: &[String],
    after: &[String],
    nl: &str,
) -> Option<String> {
    if !is_scalar_sequence_delta(before, after) {
        return None;
    }
    let lines: Vec<&str> = frontmatter.split_inclusive('\n').collect();
    let BlockSequence {
        key_index,
        end,
        key_comment,
        members,
    } = block_sequence(&lines, key, before)?;

    let mut insert_before: Vec<Vec<&String>> = (0..before.len()).map(|_| Vec::new()).collect();
    let mut tail = Vec::new();
    for (index, value) in after.iter().enumerate() {
        if before.contains(value) {
            continue;
        }
        let next = after[index + 1..]
            .iter()
            .find_map(|candidate| before.iter().position(|value| value == candidate));
        if let Some(next) = next {
            insert_before[next].push(value);
        } else {
            tail.push(value);
        }
    }
    let mut insert_after: Vec<Vec<&String>> = (0..before.len()).map(|_| Vec::new()).collect();
    if let Some(last_retained) = after
        .iter()
        .rev()
        .find_map(|candidate| before.iter().position(|value| value == candidate))
    {
        insert_after[last_retained].append(&mut tail);
    } else if !members.is_empty() {
        insert_before[0].append(&mut tail);
    }

    let mut out = String::with_capacity(frontmatter.len() + after.len() * 16);
    for (index, line) in lines.iter().enumerate() {
        if index < key_index || index >= end {
            out.push_str(line);
            continue;
        }
        if index == key_index {
            if after.is_empty() {
                if let Some(comment) = &key_comment {
                    out.push_str(comment);
                    out.push_str(nl);
                }
            } else {
                out.push_str(line);
            }
            continue;
        }
        let Some((source_index, member)) = members
            .iter()
            .enumerate()
            .find(|(_, member)| member.line_index == index)
        else {
            out.push_str(line);
            continue;
        };
        if !after.is_empty() {
            for value in &insert_before[source_index] {
                write_block_member(&mut out, member, value, nl);
            }
            if after.contains(&member.value) {
                out.push_str(member.whole);
            }
            for value in &insert_after[source_index] {
                write_block_member(&mut out, member, value, nl);
            }
        }
    }
    Some(out)
}

/// Whether writing `candidate` as `key`'s value yields frontmatter that loads
/// and reads back as exactly `ids`.
///
/// Asked rather than predicted (`A1-213`, audit). Which values a YAML flow list
/// can hold unquoted is a question about the grammar -- flow indicators, but
/// also the indicators only a *leading* character makes special, `*` and `&` and
/// `!` among them -- and a predicate enumerating them is a list of the cases its
/// author thought of. The parser already knows the answer, so it is asked, and
/// the fallback is the block form that `add` writes.
///
/// **This narrows the round-trip guard, deliberately.** A flow candidate that
/// does not read back used to reach that guard and be reported; now it quietly
/// becomes a block list. What the guard loses is a class of *cosmetic* defect
/// -- a bug in the spacing or trailing-comment capture would reformat the line
/// rather than refuse -- and what it keeps is the whole of correctness, because
/// the block form is checked by the same guard on the way out. Trading a loud
/// failure about formatting for a quiet correct write is the right way round in
/// a tool whose refusal tells the reader to report a bug.
fn holds(
    frontmatter: &str,
    key: &str,
    candidate: &str,
    ids: &[String],
    read_back: fn(&Task) -> &Vec<String>,
) -> bool {
    reparse(&replace_top_level_block(frontmatter, key, candidate))
        .is_ok_and(|task| read_back(&task) == ids)
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
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut chars = value.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == ']' => return Some(index),
            _ => {}
        }
    }
    None
}

#[derive(Clone, Copy)]
enum FlowQuote {
    None,
    Single,
    Double,
}

/// The `key: [a, b]` line, when the file writes this list inline (`A1-179`).
///
/// Only the flow form, and only at the top level: a `key:` with nothing after it
/// is the block form, and an indented one belongs to some nested mapping rather
/// than to the task. Anything else -- an anchor, a value this cannot recognise
/// -- returns `None`. Replacement-mode callers may use a block fallback; a
/// dependency delta refuses an existing field it cannot preserve. A key being
/// created still uses the block form.
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
        // A flow list broken across lines has no closing bracket here. Generic
        // replacement callers retain their old one-line fallback; dependency
        // deltas take the stricter source-preserving refusal path above.
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
    key: &'static str,
    ids: &[String],
    edit: SequenceEdit,
    read_back: fn(&Task) -> &Vec<String>,
) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let original: Task =
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    let before = read_back(&original).clone();

    let nl = newline(file, bom);
    if before == ids {
        return Ok(file.to_string());
    }

    let source_delta = edit == SequenceEdit::Delta
        || (edit == SequenceEdit::Preserve && before.len() != ids.len());
    if source_delta {
        let surgical_inline = rewrite_inline_delta(frontmatter, key, &before, ids, nl)
            .filter(|candidate| reparse(candidate).is_ok_and(|task| read_back(&task) == ids));
        let surgical_block = rewrite_block_delta(frontmatter, key, &before, ids, nl)
            .filter(|candidate| reparse(candidate).is_ok_and(|task| read_back(&task) == ids));
        if let Some(new_frontmatter) = surgical_inline.or(surgical_block) {
            return Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"));
        }
        // An absent key has no source form to preserve and is written in the
        // canonical block form below. Once the key exists, falling through to
        // that reconstruction would turn a refusal into the cosmetic rewrite
        // this delta editor exists to prevent.
        if top_level_mapping_has_key(frontmatter, key) {
            return Err(CloseError::NoSequenceDelta(key));
        }
    }

    // Indented, like `evidence:` and `trace_fingerprints:` beside it, because
    // `replace_top_level_block` ends a block at the first line that does not
    // start with whitespace: an unindented `- B` is not skipped, so replacing a
    // list left the old entries behind it. The round-trip check caught that
    // before anything was written, which is what it is for.
    let block_form = || {
        let mut out = format!("{key}:{nl}");
        for id in ids {
            out.push_str("  - ");
            out.push_str(&yaml_scalar(id));
            out.push_str(nl);
        }
        out
    };

    // One-for-one rekeys preserve the member tokens and their trivia. Tags use
    // this path for an ordinary rename and the delta path above for add, remove
    // and a rename onto a tag the task already carries.
    let source_rekey = edit == SequenceEdit::Rekey
        || (edit == SequenceEdit::Preserve && before.len() == ids.len());
    let surgical_inline = source_rekey
        .then(|| rewrite_inline_members(frontmatter, key, &before, ids, nl))
        .flatten()
        .filter(|candidate| holds(frontmatter, key, candidate, ids, read_back));
    let surgical_block = source_rekey
        .then(|| rewrite_block_members(frontmatter, key, &before, ids))
        .flatten()
        .filter(|candidate| reparse(candidate).is_ok_and(|task| read_back(&task) == ids));
    if source_rekey
        && before.len() == ids.len()
        && before != ids
        && surgical_inline.is_none()
        && surgical_block.is_none()
    {
        return Err(if edit == SequenceEdit::Preserve {
            CloseError::NoSequenceDelta(key)
        } else {
            CloseError::NoScalar(key)
        });
    }
    if let Some(new_frontmatter) = surgical_block {
        return Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"));
    }
    let block = if ids.is_empty() {
        String::new()
    } else {
        surgical_inline
            .or_else(|| {
                inline_list_line(frontmatter, key).map(|found| {
                    format!(
                        "{}{}[{}]{}{nl}",
                        found.key,
                        found.spacing,
                        ids.iter()
                            .map(|id| yaml_scalar(id))
                            .collect::<Vec<_>>()
                            .join(", "),
                        found.trailing
                    )
                })
            })
            .filter(|candidate| holds(frontmatter, key, candidate, ids, read_back))
            .unwrap_or_else(block_form)
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
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if task.status != Status::Done {
        return Err(CloseError::NotDone(task.status));
    }

    let nl = newline(file, bom);
    // In place, not remove-and-append: a re-stamp of a task whose
    // `trace_fingerprints:` sat before other keys moved the block to the end, so
    // acknowledging drift arrived as a diff that reordered the frontmatter. Found
    // auditing `A1-110`, and shipped this way since `A1-086`.
    let new_frontmatter = replace_fingerprint_mapping(
        frontmatter,
        "trace_fingerprints",
        &fingerprint_block("trace_fingerprints", fingerprints, nl)?,
        &task.trace_fingerprints,
        nl,
        true,
    )?;

    // Restamp touches only the fingerprints: confirm the status and evidence came
    // through unchanged (a future bug in the block removal would be caught here,
    // as `close` catches its own).
    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != task.evidence
        || parsed.evidence_fingerprints != task.evidence_fingerprints
        || &parsed.trace_fingerprints != fingerprints
    {
        return Err(CloseError::Produced(
            "restamp altered the status, evidence, or fingerprints".to_string(),
        ));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// Replace a `done` task's `evidence_fingerprints:` mapping, leaving its
/// citations, trace stamps, status and body untouched.
///
/// The evidence resolver computes the mapping because it owns git. This module
/// owns the surgical write because it already guarantees that every task edit
/// round-trips without reformatting unrelated frontmatter.
pub fn set_evidence_fingerprints(
    file: &str,
    fingerprints: &BTreeMap<String, String>,
) -> Result<String, CloseError> {
    let bom = leading_bom(file);
    let (frontmatter, body) = split_frontmatter(file).ok_or(CloseError::NotATask)?;
    let task: Task =
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if task.status != Status::Done {
        return Err(CloseError::NotDone(task.status));
    }

    let nl = newline(file, bom);
    let new_frontmatter = replace_fingerprint_mapping(
        frontmatter,
        "evidence_fingerprints",
        &fingerprint_block("evidence_fingerprints", fingerprints, nl)?,
        &task.evidence_fingerprints,
        nl,
        true,
    )?;
    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != task.evidence
        || parsed.trace_fingerprints != task.trace_fingerprints
        || &parsed.evidence_fingerprints != fingerprints
    {
        return Err(CloseError::Produced(
            "evidence restamp altered the status, citations, trace fingerprints, or fingerprints"
                .to_string(),
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
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
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
    let new_frontmatter = replace_evidence_sequence(frontmatter, &extended, &task.evidence, nl)?;

    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != extended
        || parsed.trace_fingerprints != task.trace_fingerprints
        || parsed.evidence_fingerprints != task.evidence_fingerprints
    {
        return Err(CloseError::Produced(
            "the addition altered the status, the fingerprints, or the existing citations"
                .to_string(),
        ));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// Whether two citations name the same thing: kind and reference, not the whole
/// record (`A1-361`).
///
/// A citation is addressed as `kind:ref` on every surface -- it is what the
/// flags take and what `NoSuchEvidence` and `AmbiguousEvidence` print -- so it
/// is what "the same citation" has to mean. Comparing the whole `Evidence`
/// compared `captured` too, which a caller naming a citation has no way to
/// supply: `kind:ref` has no room for a date. Every citation in this repository
/// happens to carry none, so the two comparisons agreed here and nowhere else.
/// An imported plan is where they part -- `import --implemented-as done` records
/// note-level provenance *with* a date -- and there the citation could be named
/// in the message and never matched by the code that printed it.
fn names_same(a: &Evidence, b: &Evidence) -> bool {
    a.kind == b.kind && a.reference == b.reference
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
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    if task.status != Status::Done {
        return Err(CloseError::NotDone(task.status));
    }

    let label = |e: &Evidence| format!("{}:{}", e.kind.as_str(), e.reference);
    match task.evidence.iter().filter(|e| names_same(e, drop)).count() {
        0 => return Err(CloseError::NoSuchEvidence(label(drop))),
        1 => {}
        n => return Err(CloseError::AmbiguousEvidence(label(drop), n)),
    }

    let remaining: Vec<Evidence> = task
        .evidence
        .iter()
        .filter(|e| !names_same(e, drop))
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
    let new_frontmatter = replace_evidence_sequence(frontmatter, &remaining, &task.evidence, nl)?;

    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != remaining
        || parsed.trace_fingerprints != task.trace_fingerprints
        || parsed.evidence_fingerprints != task.evidence_fingerprints
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
        yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Frontmatter(e.to_string()))?;
    // Scoped to a closed task, because that is the case with no other route: a
    // live task's evidence can still be given by `close`.
    if task.status != Status::Done {
        return Err(CloseError::NotDone(task.status));
    }

    let label = |e: &Evidence| format!("{}:{}", e.kind.as_str(), e.reference);
    let matches = task.evidence.iter().filter(|e| names_same(e, from)).count();
    match matches {
        0 => return Err(CloseError::NoSuchEvidence(label(from))),
        1 => {}
        n => return Err(CloseError::AmbiguousEvidence(label(from), n)),
    }
    if task.evidence.iter().any(|e| names_same(e, to)) {
        return Err(CloseError::DuplicateEvidence(label(to)));
    }

    let repaired: Vec<Evidence> = task
        .evidence
        .iter()
        .map(|e| {
            if names_same(e, from) {
                to.clone()
            } else {
                e.clone()
            }
        })
        .collect();

    let nl = newline(file, bom);
    let new_frontmatter = replace_evidence_sequence(frontmatter, &repaired, &task.evidence, nl)?;

    // The same self-check `close` and `restamp` make: nothing but the one
    // citation may have moved.
    let parsed = reparse(&new_frontmatter)?;
    if parsed.status != task.status
        || parsed.evidence != repaired
        || parsed.trace_fingerprints != task.trace_fingerprints
        || parsed.evidence_fingerprints != task.evidence_fingerprints
    {
        return Err(CloseError::Produced(
            "the repair altered the status, the fingerprints, or more than the one citation"
                .to_string(),
        ));
    }
    Ok(format!("{bom}---{nl}{new_frontmatter}---{nl}{body}"))
}

/// Apply a diagnostic's exact repair through the same surgical primitive the
/// explicit command uses.
///
/// This function only rewrites. Its caller must still resolve the proposed
/// evidence, run the mutation gate, and save through the guarded writer; keeping
/// those policy checks at the command boundary prevents a typed repair from
/// becoming a way around `aplan amend` (`A1-199`).
pub fn apply_repair(file: &str, repair: &Repair) -> Result<String, CloseError> {
    match repair {
        Repair::ReplaceEvidence { from, to } => replace_evidence(file, from, to),
    }
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
    yaml_compat::from_str(frontmatter).map_err(|e| CloseError::Produced(e.to_string()))
}

/// Whether the frontmatter declares `key` as a top-level key (with or without a
/// value, and with or without a space before the colon).
fn has_top_level_key(frontmatter: &str, key: &str) -> bool {
    frontmatter.lines().any(|line| {
        line.strip_prefix(key)
            .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
    })
}

/// Whether the parsed top-level mapping carries `key`, including a quoted key.
///
/// The source editor below intentionally recognises only the ordinary literal
/// form. Looking at the parsed mapping as well prevents an alternate spelling
/// such as `"trace_fingerprints":` from being mistaken for an absent field and
/// receiving a duplicate unquoted key.
fn top_level_mapping_has_key(frontmatter: &str, key: &str) -> bool {
    yaml_compat::from_str::<yaml_compat::Value>(frontmatter).is_ok_and(|value| {
        value.as_mapping().is_some_and(|mapping| {
            mapping.contains_key(yaml_compat::Value::String(key.to_string()))
        })
    })
}

/// Replace an authored evidence sequence only when its source is local to that
/// field and agrees exactly with the typed task.
///
/// Deserialisation erases YAML anchors, aliases and explicit tags. Rebuilding a
/// sequence after that erasure can remove an anchor still used by another
/// field, so the typed value alone is not authority to delete its source. The
/// accepted form is deliberately the ordinary block sequence this writer
/// emits: each citation is a mapping of inline scalar fields. An absent empty
/// sequence remains appendable for old closed tasks which need their first
/// citation added.
fn replace_evidence_sequence(
    frontmatter: &str,
    desired: &[Evidence],
    current: &[Evidence],
    nl: &str,
) -> Result<String, CloseError> {
    if top_level_mapping_has_key(frontmatter, "evidence") {
        let Some(source) = ordinary_evidence_sequence(frontmatter).filter(|source| {
            has_top_level_key(frontmatter, "evidence") && source.values == current
        }) else {
            return Err(CloseError::NoEvidenceReplacement);
        };
        return source.rewrite_delta(frontmatter, desired, nl);
    }
    if current.is_empty() {
        Ok(replace_top_level_block(
            frontmatter,
            "evidence",
            &evidence_block(desired, nl)?,
        ))
    } else {
        // A citation loaded without a literal top-level source is necessarily
        // structural (for example supplied through a merge) and is not local
        // material this field editor may replace.
        Err(CloseError::NoEvidenceReplacement)
    }
}

/// The semantic citations and exact boundary of an ordinary evidence block.
struct OrdinaryEvidenceSequence {
    values: Vec<Evidence>,
    boundary_comment: Option<String>,
    key_line_index: usize,
    fields: Vec<OrdinaryEvidenceField>,
    insert_after_line: usize,
    dash_indent: String,
    continuation_indent: String,
}

impl OrdinaryEvidenceSequence {
    fn rewrite_delta(
        &self,
        frontmatter: &str,
        desired: &[Evidence],
        nl: &str,
    ) -> Result<String, CloseError> {
        let delta = EvidenceDelta::between(&self.values, desired)?;
        let mut out = String::with_capacity(frontmatter.len() + 128);
        for (line_index, line) in frontmatter.split_inclusive('\n').enumerate() {
            if line_index == self.key_line_index && delta.removes_final(self.values.len()) {
                if let Some(comment) = &self.boundary_comment {
                    out.push_str(comment);
                    out.push_str(nl);
                }
                continue;
            }

            let field = self
                .fields
                .iter()
                .find(|field| field.line_index == line_index);
            if field.is_some_and(|field| delta.removes_entry(field.entry_index)) {
                continue;
            }

            if let (Some(field), EvidenceDelta::Replace { index, value }) = (field, &delta)
                && field.entry_index == *index
            {
                let replacement = field.name.value(value);
                if replacement.is_none() {
                    continue;
                }
                let replacement = replacement.expect("a retained evidence field has a value");
                if field.name.value(&self.values[*index]) == Some(replacement) {
                    out.push_str(line);
                } else {
                    let old = &line[field.value_range.clone()];
                    out.push_str(&line[..field.value_range.start]);
                    out.push_str(&scalar_like(old, replacement));
                    out.push_str(&line[field.value_range.end..]);
                }
                if field.name == EvidenceFieldName::Reference
                    && self.values[*index].captured.is_none()
                    && value.captured.is_some()
                {
                    self.write_captured(&mut out, value, nl);
                }
                continue;
            }

            out.push_str(line);
            if line_index == self.insert_after_line
                && let EvidenceDelta::Add(value) = &delta
            {
                self.write_added(&mut out, value, nl);
            }
        }
        Ok(out)
    }

    fn write_added(&self, out: &mut String, value: &Evidence, nl: &str) {
        use std::fmt::Write as _;

        let _ = write!(
            out,
            "{}- kind: {}{nl}{}ref: {}{nl}",
            self.dash_indent,
            yaml_scalar(value.kind.as_str()),
            self.continuation_indent,
            yaml_scalar(&value.reference),
        );
        self.write_captured(out, value, nl);
    }

    fn write_captured(&self, out: &mut String, value: &Evidence, nl: &str) {
        use std::fmt::Write as _;

        if let Some(captured) = &value.captured {
            let _ = write!(
                out,
                "{}captured: {}{nl}",
                self.continuation_indent,
                yaml_scalar(captured)
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EvidenceFieldName {
    Kind,
    Reference,
    Captured,
}

impl EvidenceFieldName {
    fn value(self, evidence: &Evidence) -> Option<&str> {
        match self {
            Self::Kind => Some(evidence.kind.as_str()),
            Self::Reference => Some(&evidence.reference),
            Self::Captured => evidence.captured.as_deref(),
        }
    }
}

struct OrdinaryEvidenceField {
    line_index: usize,
    entry_index: usize,
    name: EvidenceFieldName,
    value_range: std::ops::Range<usize>,
}

enum EvidenceDelta<'a> {
    Add(&'a Evidence),
    Remove { index: usize },
    Replace { index: usize, value: &'a Evidence },
}

impl<'a> EvidenceDelta<'a> {
    fn between(before: &[Evidence], after: &'a [Evidence]) -> Result<Self, CloseError> {
        if after.len() == before.len() + 1 && after[..before.len()] == *before {
            return Ok(Self::Add(
                after.last().expect("a one-member growth has a last member"),
            ));
        }
        if before.len() == after.len() + 1
            && let Some(index) = (0..before.len()).find(|index| {
                before[..*index] == after[..*index] && before[*index + 1..] == after[*index..]
            })
        {
            return Ok(Self::Remove { index });
        }
        if before.len() == after.len() {
            let changed: Vec<usize> = before
                .iter()
                .zip(after)
                .enumerate()
                .filter_map(|(index, (old, new))| (old != new).then_some(index))
                .collect();
            if let [index] = changed.as_slice() {
                return Ok(Self::Replace {
                    index: *index,
                    value: &after[*index],
                });
            }
        }
        Err(CloseError::Produced(
            "an evidence edit was not exactly one add, remove or replacement".to_string(),
        ))
    }

    fn removes_entry(&self, entry: usize) -> bool {
        matches!(self, Self::Remove { index } if *index == entry)
    }

    fn removes_final(&self, entries: usize) -> bool {
        entries == 1 && matches!(self, Self::Remove { index: 0 })
    }
}

/// Read the ordinary block citation sequence emitted by [`evidence_block`].
///
/// Every accepted value token is independently parseable as a string, which
/// excludes YAML properties and nested structure. Reconstructing `Evidence`
/// values and comparing them to the task makes this a recogniser for the field
/// the loader saw, not merely for similarly shaped source text.
fn ordinary_evidence_sequence(frontmatter: &str) -> Option<OrdinaryEvidenceSequence> {
    let lines: Vec<&str> = frontmatter.split_inclusive('\n').collect();
    let (key_index, value) = lines.iter().enumerate().find_map(|(index, line)| {
        let content = line.trim_end_matches(['\r', '\n']);
        let after_key = content.strip_prefix("evidence")?;
        let after_ws = after_key.trim_start_matches([' ', '\t']);
        let after_colon = after_ws.strip_prefix(':')?;
        Some((index, after_colon.trim_start_matches([' ', '\t'])))
    })?;
    if !value.is_empty() && !value.starts_with('#') {
        return None;
    }

    let mut raw = Vec::<BTreeMap<String, String>>::new();
    let mut fields = Vec::new();
    let mut insert_after_line = key_index;
    let mut dash_indent = None;
    let mut continuation_indent = None;
    for (line_index, line) in lines.into_iter().enumerate().skip(key_index + 1) {
        let content = line.trim_end_matches(['\r', '\n']);
        let trimmed = content.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let member = if let Some(after_dash) = trimmed.strip_prefix('-') {
            if !after_dash.is_empty() && !after_dash.starts_with([' ', '\t']) {
                return None;
            }
            let member = after_dash.trim_start_matches([' ', '\t']);
            if member.is_empty() {
                return None;
            }
            raw.push(BTreeMap::new());
            dash_indent.get_or_insert_with(|| content[..content.len() - trimmed.len()].to_string());
            member
        } else if trimmed.len() != content.len() {
            if raw.is_empty() {
                return None;
            }
            continuation_indent
                .get_or_insert_with(|| content[..content.len() - trimmed.len()].to_string());
            trimmed
        } else {
            break;
        };

        let parsed = ordinary_string_mapping_member_source(member, true)?;
        // Citation field names are a fixed schema, not authored values. A
        // quoted or otherwise structural key spelling is deliberately outside
        // this source-local editor even when YAML resolves it to the same text.
        if parsed.key_source != parsed.key {
            return None;
        }
        let name = match parsed.key.as_str() {
            "kind" => EvidenceFieldName::Kind,
            "ref" => EvidenceFieldName::Reference,
            "captured" => EvidenceFieldName::Captured,
            _ => return None,
        };
        let member_offset = content.len() - member.len();
        fields.push(OrdinaryEvidenceField {
            line_index,
            entry_index: raw.len() - 1,
            name,
            value_range: member_offset + parsed.value_range.start
                ..member_offset + parsed.value_range.end,
        });
        if raw.last_mut()?.insert(parsed.key, parsed.value).is_some() {
            return None;
        }
        insert_after_line = line_index;
    }

    let mut values = Vec::with_capacity(raw.len());
    for mut fields in raw {
        let kind = fields.remove("kind")?;
        let kind = crate::task::EvidenceKind::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == kind)?;
        let reference = fields.remove("ref")?;
        let captured = fields.remove("captured");
        values.push(Evidence {
            kind,
            reference,
            captured,
        });
    }

    Some(OrdinaryEvidenceSequence {
        values,
        boundary_comment: value.starts_with('#').then(|| value.to_string()),
        key_line_index: key_index,
        fields,
        insert_after_line,
        dash_indent: dash_indent.unwrap_or_else(|| "  ".to_string()),
        continuation_indent: continuation_indent.unwrap_or_else(|| "    ".to_string()),
    })
}

/// Replace one derived fingerprint mapping only when its source is local to
/// that mapping.
///
/// `yaml_compat` resolves aliases while deserialising, so the typed `Task`
/// cannot say whether deleting this source block would orphan another node.
/// This deliberately small source recogniser accepts the two ordinary forms
/// a fingerprint map uses -- a block of inline string pairs and a one-line flow
/// map of the same -- and refuses every structural form before the generic
/// block splice can affect source outside the field it owns.
fn replace_fingerprint_mapping(
    frontmatter: &str,
    key: &'static str,
    block: &str,
    current: &BTreeMap<String, String>,
    nl: &str,
    append_if_absent: bool,
) -> Result<String, CloseError> {
    if top_level_mapping_has_key(frontmatter, key) {
        let Some(source) = ordinary_fingerprint_mapping(frontmatter, key)
            .filter(|source| has_top_level_key(frontmatter, key) && &source.values == current)
        else {
            return Err(CloseError::NoMappingReplacement(key));
        };
        let replacement = source.replacement_block(key, block, nl)?;
        return Ok(replace_top_level_block(frontmatter, key, &replacement));
    }
    if append_if_absent {
        Ok(replace_top_level_block(frontmatter, key, block))
    } else {
        Ok(frontmatter.to_string())
    }
}

/// The parsed values and the exact source boundary of an ordinary fingerprint
/// mapping. The values prove that the recogniser and typed task saw the same
/// mapping; the other fields let its derived contents move without rewriting
/// authored key spelling or a note attached to that key.
struct OrdinaryFingerprintMapping {
    values: BTreeMap<String, String>,
    boundary_line: String,
    boundary_comment: Option<String>,
}

impl OrdinaryFingerprintMapping {
    fn replacement_block(&self, key: &str, block: &str, nl: &str) -> Result<String, CloseError> {
        if block.is_empty() {
            return Ok(self
                .boundary_comment
                .as_ref()
                .map_or_else(String::new, |comment| format!("{comment}{nl}")));
        }

        let header = format!("{key}:{nl}");
        let members = block.strip_prefix(&header).ok_or_else(|| {
            CloseError::Produced(format!("`{key}` fingerprint block has no canonical header"))
        })?;
        Ok(format!("{}{nl}{members}", self.boundary_line))
    }
}

/// Read an ordinary source-local fingerprint mapping without accepting YAML
/// properties or nested structure. Returning its values as well as recognising
/// its syntax makes the source pass answer for the exact mapping the typed task
/// loaded, rather than for some lookalike line elsewhere in the document.
fn ordinary_fingerprint_mapping(
    frontmatter: &str,
    key: &str,
) -> Option<OrdinaryFingerprintMapping> {
    let lines: Vec<&str> = frontmatter.split_inclusive('\n').collect();
    let (key_index, content, value_start) =
        lines.iter().enumerate().find_map(|(index, line)| {
            let content = line.trim_end_matches(['\r', '\n']);
            let after_key = content.strip_prefix(key)?;
            let after_ws = after_key.trim_start_matches([' ', '\t']);
            let after_colon = after_ws.strip_prefix(':')?;
            let value = after_colon.trim_start_matches([' ', '\t']);
            Some((index, content, content.len() - value.len()))
        })?;
    let value = &content[value_start..];

    if !value.is_empty() && !value.starts_with('#') {
        let (values, close) = ordinary_flow_string_mapping(value)?;
        let suffix = &value[close + 1..];
        return Some(OrdinaryFingerprintMapping {
            values,
            boundary_line: format!("{}{}", &content[..value_start], suffix),
            boundary_comment: suffix
                .trim_start_matches([' ', '\t'])
                .starts_with('#')
                .then(|| suffix.trim_start_matches([' ', '\t']).to_string()),
        });
    }

    let mut out = BTreeMap::new();
    for line in lines.into_iter().skip(key_index + 1) {
        let content = line.trim_end_matches(['\r', '\n']);
        let trimmed = content.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.len() == content.len() {
            break;
        }
        let (member_key, member_value) = ordinary_string_mapping_member(trimmed, true)?;
        if out.insert(member_key, member_value).is_some() {
            return None;
        }
    }
    Some(OrdinaryFingerprintMapping {
        values: out,
        boundary_line: content.to_string(),
        boundary_comment: value.starts_with('#').then(|| value.to_string()),
    })
}

/// A one-line `{key: value}` map whose members are all inline strings.
fn ordinary_flow_string_mapping(source: &str) -> Option<(BTreeMap<String, String>, usize)> {
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut close = None;
    let mut chars = source.char_indices().peekable();
    let (open, first) = chars.next()?;
    if open != 0 || first != '{' {
        return None;
    }
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == '}' => {
                close = Some(index);
                break;
            }
            FlowQuote::None if matches!(ch, '{' | '[' | ']') => return None,
            _ => {}
        }
    }
    let close = close?;
    let suffix = &source[close + 1..];
    if !suffix.trim().is_empty() && !suffix.trim_start().starts_with('#') {
        return None;
    }
    let inside = &source[1..close];
    if inside.trim().is_empty() {
        return Some((BTreeMap::new(), close));
    }

    let mut members = Vec::new();
    let mut start = 0;
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut chars = inside.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == ',' => {
                members.push(&inside[start..index]);
                start = index + 1;
            }
            FlowQuote::None if matches!(ch, '{' | '}' | '[' | ']') => return None,
            _ => {}
        }
    }
    // The first scan only records a closing brace outside a quote, so the
    // identical member scan necessarily finishes outside one too.
    debug_assert!(matches!(quote, FlowQuote::None));
    members.push(&inside[start..]);

    let mut out = BTreeMap::new();
    for member in members {
        let (member_key, member_value) = ordinary_string_mapping_member(member.trim(), false)?;
        if out.insert(member_key, member_value).is_some() {
            return None;
        }
    }
    Some((out, close))
}

/// Parse one `string: string` member without admitting a YAML property, block
/// scalar, nested collection or complex key. The typed task has already proved
/// the complete YAML document is valid; this pass proves the source is safe to
/// replace independently.
fn ordinary_string_mapping_member(source: &str, allow_comment: bool) -> Option<(String, String)> {
    ordinary_string_mapping_member_source(source, allow_comment)
        .map(|member| (member.key, member.value))
}

struct OrdinaryStringMappingMember {
    key: String,
    key_source: String,
    value: String,
    value_range: std::ops::Range<usize>,
}

fn ordinary_string_mapping_member_source(
    source: &str,
    allow_comment: bool,
) -> Option<OrdinaryStringMappingMember> {
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut previous_whitespace = false;
    let mut candidates = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == '#' && previous_whitespace => break,
            FlowQuote::None if ch == ':' => candidates.push(index),
            _ => {}
        }
        previous_whitespace = ch.is_whitespace();
    }
    if !matches!(quote, FlowQuote::None) {
        return None;
    }

    let mut parsed = Vec::new();
    for colon in candidates {
        let key_source = source[..colon].trim();
        let after_colon = &source[colon + 1..];
        let value_source = after_colon.trim_start_matches([' ', '\t']);
        let value_start = colon + 1 + after_colon.len() - value_source.len();
        // A colon inside a plain scalar is another candidate, not proof that
        // the whole member is structural. For `ref: https://example.com`, the
        // URL's colon makes `ref: https` an invalid *key* candidate after the
        // real separator has already parsed successfully. Rejecting the whole
        // line there made ordinary URL citations impossible to maintain.
        let Some(key_end) = inline_scalar_end(key_source) else {
            continue;
        };
        debug_assert_eq!(key_end, key_source.len());
        let Some(value_end) = inline_scalar_end(value_source) else {
            continue;
        };
        let suffix = &value_source[value_end..];
        if !suffix.trim().is_empty() && (!allow_comment || !suffix.trim_start().starts_with('#')) {
            continue;
        }
        // `inline_scalar_end` has already parsed these exact slices as
        // strings; keep the values rather than maintaining impossible retry
        // branches after successful validation.
        let Ok(member_key) = yaml_compat::from_str::<String>(key_source) else {
            continue;
        };
        let Ok(member_value) = yaml_compat::from_str::<String>(&value_source[..value_end]) else {
            continue;
        };
        parsed.push(OrdinaryStringMappingMember {
            key: member_key,
            key_source: key_source.to_string(),
            value: member_value,
            value_range: value_start..value_start + value_end,
        });
    }
    (parsed.len() == 1).then(|| parsed.remove(0))
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
            // Blank lines and comments do not end a YAML block. In particular,
            // a column-zero comment may sit between two indented mapping
            // entries: stopping there leaves the later old entry behind and
            // makes the replacement fail its round-trip. Keep trivia exactly
            // where the author put it, and keep looking for the next value
            // line that can actually establish a new top-level node.
            if content.trim().is_empty() || content.trim_start().starts_with('#') {
                out.push_str(line);
                continue;
            }
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

/// A fingerprint mapping block (empty string when there are none), serialised
/// via serde and re-indented two spaces, using `nl`.
fn fingerprint_block(
    key: &str,
    fingerprints: &BTreeMap<String, String>,
    nl: &str,
) -> Result<String, CloseError> {
    if fingerprints.is_empty() {
        return Ok(String::new());
    }
    let map =
        yaml_compat::to_string(fingerprints).map_err(|e| CloseError::Produced(e.to_string()))?;
    let mut out = format!("{key}:{nl}");
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
    if !has_single_source_local_scalar_line(frontmatter, "status") {
        return Err(CloseError::NoScalar("status"));
    }
    let mut out = String::with_capacity(frontmatter.len() + 8);
    let mut changed = false;
    for line in frontmatter.split_inclusive('\n') {
        if !changed && let Some(rewritten) = rewrite_scalar_line(line, "status", &to.to_string()) {
            out.push_str(&rewritten);
            changed = true;
            continue;
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
struct ScalarLineParts<'a> {
    key_segment: &'a str,
    lead_ws: &'a str,
    value: &'a str,
    trailing: &'a str,
    eol: &'a str,
}

fn scalar_line_parts<'a>(line: &'a str, key: &str) -> Option<ScalarLineParts<'a>> {
    let eol_len = line.len() - line.trim_end_matches(['\r', '\n']).len();
    let content = &line[..line.len() - eol_len];
    let eol = &line[content.len()..];

    // Top-level key only (no leading whitespace: an indented `status:` belongs to
    // some nested mapping, not the task).
    let after_key = content.strip_prefix(key)?;
    let after_ws = after_key.trim_start_matches([' ', '\t']);
    let after_colon = after_ws.strip_prefix(':')?;
    let key_segment = &content[..content.len() - after_colon.len()];

    // Keep the whitespace after the colon; replace the complete scalar; keep
    // whatever follows (spacing and a trailing comment). Whitespace and `#`
    // inside a quoted scalar belong to the value, not to the suffix. Looking
    // for the first one made `rekey` read `id: "T old"` as the token `"T` and
    // splice the rest of the old id after the new one (`A1-410`).
    let ws = after_colon.len() - after_colon.trim_start_matches([' ', '\t']).len();
    let (lead_ws, tail) = after_colon.split_at(ws);
    let val_end = inline_scalar_end(tail)?;
    let value = &tail[..val_end];
    let trailing = &tail[val_end..];
    Some(ScalarLineParts {
        key_segment,
        lead_ws,
        value,
        trailing,
        eol,
    })
}

fn rewrite_scalar_line(line: &str, key: &str, to: &str) -> Option<String> {
    let parts = scalar_line_parts(line, key)?;
    Some(format!(
        "{}{lead_ws}{to}{}{eol}",
        parts.key_segment,
        parts.trailing,
        lead_ws = parts.lead_ws,
        eol = parts.eol
    ))
}

/// End of one inline YAML string scalar, before its trailing trivia.
///
/// Whitespace is part of a plain or quoted string until only whitespace and an
/// optional comment remain. Structural forms are deliberately refused: an
/// anchor or alias can affect another node, and a block scalar spans lines this
/// helper cannot replace without guessing at its extent.
fn inline_scalar_end(value: &str) -> Option<usize> {
    if value.is_empty() || value.starts_with(['!', '&', '*', '>', '|']) {
        return None;
    }
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut previous_whitespace = false;
    let mut comment = None;
    let mut chars = value.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == '#' && previous_whitespace => {
                comment = Some(index);
                break;
            }
            _ => {}
        }
        previous_whitespace = ch.is_whitespace();
    }
    if !matches!(quote, FlowQuote::None) {
        return None;
    }
    let before_comment = &value[..comment.unwrap_or(value.len())];
    let end = before_comment.trim_end_matches([' ', '\t']).len();
    if end == 0 {
        return None;
    }
    yaml_compat::from_str::<String>(&value[..end])
        .ok()
        .map(|_| end)
}

/// The `evidence:` block for `evidence`, serialised via serde (so quoting is
/// correct) and re-indented two spaces to match the hand-written style, using
/// `nl` as the line ending so it matches the file it is appended to.
fn evidence_block(evidence: &[Evidence], nl: &str) -> Result<String, CloseError> {
    let seq = yaml_compat::to_string(&evidence).map_err(|e| CloseError::Produced(e.to_string()))?;
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
            && let Some(fp) = crate::anchor::fingerprint_in(rel, &text, anchor)
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

    /// The author's flow list is kept by quoting a newly added scalar whenever
    /// its punctuation would otherwise be structural (`A1-213`, `A1-715`).
    ///
    /// `aplan add --tag "a, b"` writes that tag in block form, where it is an
    /// ordinary scalar. Writing it back into `[...]` split one tag into two, and
    /// the round-trip guard reported it as an internal fault -- the tool blaming
    /// itself for a value the creation path accepts.
    #[test]
    fn a_flow_list_keeps_source_form_for_every_scalar_value() {
        let file = |tags: &str| {
            format!("---\nid: W-1\ntitle: W-1\nphase: 0\nstatus: todo\n{tags}---\n\nbody\n")
        };
        let tags_of = |written: &str| -> Vec<String> {
            let task: Task =
                yaml_compat::from_str(split_frontmatter(written).expect("frontmatter").0)
                    .expect("parses");
            task.tags
        };

        // Ordinary values keep the form, its spacing and its trailing comment.
        let kept = set_tags(
            &file("tags: [cli, backend]   # why\n"),
            &["cli".into(), "backend".into(), "docs".into()],
        )
        .expect("writes");
        assert!(
            kept.contains("tags: [cli, backend, docs]   # why"),
            "the author's form was rewritten: {kept}"
        );

        // **Every value reachable through `add --tag` survives, whichever form
        // that takes.** The characters worth listing are the ones a predicate
        // would have had to know about: the flow indicators, and the ones only a
        // *leading* position makes special -- `*` opens an alias, `&` an anchor,
        // `!` a tag -- which is why the form is proved rather than predicted.
        for value in [
            "a, b",
            "a[b",
            "a]b",
            "a{b",
            "a}b",
            "a#b",
            "a\"b",
            "a'b",
            "*",
            "*a",
            "&a",
            "!a",
            "|a",
            ">a",
            "%a",
            "@a",
            "`a",
            "?a",
            ":a",
            "-a",
            "a b",
            "a:b",
            "artifact:accepted",
            "priority-p0",
            "v1.2.3",
            // Nothing should type these, and the point is that typing one is a
            // refusal about the value or nothing at all, never the round-trip
            // guard telling the reader to report a bug in `aplan`.
            "",
            " ",
            "a\tb",
            "a\nb",
            "\u{a0}x",
        ] {
            let written = set_tags(&file("tags: [cli]\n"), &["cli".into(), value.into()])
                .unwrap_or_else(|e| panic!("`{value}` could not be written at all: {e}"));
            assert_eq!(
                tags_of(&written),
                vec!["cli".to_string(), value.to_string()],
                "`{value}` did not survive the form chosen for it: {written}"
            );
        }

        // The punctuation is quoted in place rather than turning the whole
        // source sequence into a block.
        let widened = set_tags(&file("tags: [cli]\n"), &["cli".into(), "a, b".into()])
            .expect("writes rather than refusing a value `add --tag` accepts");
        assert!(
            widened.contains("tags: [cli, 'a, b']") || widened.contains("tags: [cli, \"a, b\"]"),
            "expected the original flow form: {widened}"
        );
    }

    #[test]
    fn tag_edits_refuse_structural_sequences_before_rewriting_them() {
        let file = |tags: &str, relationship: &str| {
            format!(
                "---\nid: W-1\ntitle: W-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 {tags}{relationship}---\n\nbody\n"
            )
        };
        let added = &[
            "cli".to_string(),
            "docs".to_string(),
            "reviewed".to_string(),
        ];

        for source in [
            file("tags: &shared\n  - cli\n", "blocked_by: *shared\n"),
            file("blocked_by: &shared\n  - cli\n", "tags: *shared\n"),
            file("tags: !!seq [cli]\n", ""),
            file("tags: [cli,\n  docs]\n", ""),
            file("tags:\n  - |-\n    cli\n", ""),
            file("'tags': [cli]\n", ""),
        ] {
            assert!(matches!(
                set_tags(&source, added),
                Err(CloseError::NoSequenceDelta("tags"))
            ));
        }

        let anchored = file("tags: &shared\n  - cli\n", "blocked_by: *shared\n");
        assert!(matches!(
            set_tags(&anchored, &["documentation".to_string()]),
            Err(CloseError::NoSequenceDelta("tags"))
        ));
    }

    /// **Every printable ASCII character, in every position that makes one
    /// special, survives being written into a list** (`A1-250`, audit).
    ///
    /// The fixed list beside this one is the set of values somebody thought of,
    /// which is exactly the shape of the defect that list was written for: the
    /// first fix here was a predicate enumerating the characters a flow list
    /// cannot hold, and it let `*` through. A sweep cannot think of nothing.
    ///
    /// Position matters because YAML's indicators are positional: `*` opens an
    /// alias only as the first character, `#` starts a comment only after a
    /// space, and `:` ends a key only before one.
    #[test]
    fn every_printable_character_survives_in_every_position() {
        let file = |tags: &str| {
            format!("---\nid: W-1\ntitle: W-1\nphase: 0\nstatus: todo\n{tags}---\n\nbody\n")
        };
        let mut checked = 0usize;
        for byte in 0x20u8..=0x7e {
            let c = byte as char;
            for value in [
                c.to_string(),
                format!("{c}x"),
                format!("x{c}"),
                format!("a{c}b"),
                format!("a {c} b"),
            ] {
                // Both forms: the author's flow list and a block one, because
                // the writer chooses between them and each has its own hazards.
                for existing in ["tags: [cli]\n", "tags:\n  - cli\n"] {
                    let written = set_tags(&file(existing), &["cli".into(), value.clone()])
                        .unwrap_or_else(|e| {
                            panic!("{value:?} (byte {byte:#04x}) could not be written: {e}")
                        });
                    let back: Task =
                        yaml_compat::from_str(split_frontmatter(&written).expect("frontmatter").0)
                            .unwrap_or_else(|e| {
                                panic!("{value:?} produced frontmatter that fails: {e}")
                            });
                    assert_eq!(
                        back.tags,
                        vec!["cli".to_string(), value.clone()],
                        "{value:?} (byte {byte:#04x}) came back changed: {written}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked >= 900, "the sweep did not run: {checked}");
    }

    /// A value long enough that a serialiser might fold it, and a few shapes
    /// that are not ASCII at all (`A1-250`, audit).
    ///
    /// `yaml_compat` does not wrap, but nothing in this file's contract says
    /// it may not, and a folded scalar cannot sit in one list line. The
    /// fallback to the double-quoted form is what makes that a non-event, so it
    /// is worth a test that would notice.
    #[test]
    fn a_long_or_non_ascii_value_survives_too() {
        let file = "---\nid: W-1\ntitle: W-1\nphase: 0\nstatus: todo\ntags: [cli]\n---\n\nbody\n";
        for value in [
            "x".repeat(200),
            "long value with spaces ".repeat(40),
            "a\nb\nc".to_string(),
            "\u{1f600}".to_string(),
            "\u{a0}leading-nbsp".to_string(),
            "tab\there".to_string(),
            "\u{0}".to_string(),
            "\u{7f}".to_string(),
            "\u{feff}".to_string(),
            "ending-newline\n".to_string(),
            " ".to_string(),
            String::new(),
        ] {
            let written = set_tags(file, &["cli".into(), value.clone()])
                .unwrap_or_else(|e| panic!("{value:?} could not be written: {e}"));
            let back: Task =
                yaml_compat::from_str(split_frontmatter(&written).expect("frontmatter").0)
                    .unwrap_or_else(|e| panic!("{value:?} produced frontmatter that fails: {e}"));
            assert_eq!(
                back.tags,
                vec!["cli".to_string(), value.clone()],
                "{value:?} came back changed: {written}"
            );
        }
    }

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
    fn evidence_fingerprints_are_replaced_surgically_and_can_be_cleared() {
        let original =
            closed("evidence_fingerprints:\n  'file:tests/t.rs': old\nowner: alice # keep me\n");
        let fingerprints = [("file:tests/t.rs".to_string(), "new".to_string())]
            .into_iter()
            .collect();
        let written = set_evidence_fingerprints(&original, &fingerprints).expect("restamps");
        assert!(written.contains("file:tests/t.rs"), "{written}");
        assert!(written.contains("new"), "{written}");
        assert!(!written.contains("file:tests/t.rs': old"), "{written}");
        assert!(written.contains("owner: alice # keep me"), "{written}");
        assert!(written.ends_with("\nBody stays.\n"), "{written}");

        let cleared = set_evidence_fingerprints(&written, &BTreeMap::new()).expect("clears");
        assert!(!cleared.contains("evidence_fingerprints:"), "{cleared}");
        assert!(cleared.contains("owner: alice # keep me"), "{cleared}");
        assert!(cleared.ends_with("\nBody stays.\n"), "{cleared}");
    }

    #[test]
    fn fingerprint_replacements_accept_only_source_local_string_mappings() {
        let flow = closed(
            "evidence_fingerprints: {'file:tests/t.rs': old} # keep the form here\n\
             owner: alice # keep me\n",
        );
        let fresh = fps(&[("file:tests/t.rs", "new")]);
        let written = set_evidence_fingerprints(&flow, &fresh)
            .expect("an ordinary one-line string map remains replaceable");
        assert!(written.contains("  file:tests/t.rs: new\n"), "{written}");
        assert!(written.contains("owner: alice # keep me"), "{written}");
        assert!(written.ends_with("\nBody stays.\n"), "{written}");

        let structural = [
            closed(
                "evidence_fingerprints: &stamps\n  'file:tests/t.rs': old\n\
                 trace_fingerprints: *stamps\n",
            ),
            closed("evidence_fingerprints: !!map\n  'file:tests/t.rs': old\n"),
            closed("evidence_fingerprints:\n  'file:tests/t.rs': &stamp old\nowner: *stamp\n"),
            closed("evidence_fingerprints:\n  'file:tests/t.rs': |-\n    old\n"),
            closed("evidence_fingerprints: {\n  'file:tests/t.rs': old\n  }\n"),
            closed("'evidence_fingerprints': {'file:tests/t.rs': old}\n"),
        ];
        for source in structural {
            let (frontmatter, _) = split_frontmatter(&source).expect("frontmatter");
            let before: Task = yaml_compat::from_str(frontmatter)
                .unwrap_or_else(|error| panic!("structural fixture must load: {error}\n{source}"));
            let error = set_evidence_fingerprints(&source, &fresh)
                .expect_err("structural source must be refused before replacement");
            assert!(
                matches!(
                    &error,
                    CloseError::NoMappingReplacement("evidence_fingerprints")
                ),
                "wrong refusal for {source}: {error}"
            );
            assert!(error.to_string().contains("simplify that field by hand"));
            let after: Task = yaml_compat::from_str(frontmatter).expect("unchanged source loads");
            assert_eq!(after, before, "a refusal changed the parsed task");
        }
    }

    #[test]
    fn fingerprint_rewrites_preserve_mapping_boundary_trivia() {
        let fresh_evidence = fps(&[("file:tests/t.rs", "new")]);
        let flow = closed(concat!(
            "evidence_fingerprints :   {'file:tests/t.rs': old}  # keep flow note\n",
            "owner: alice\n",
        ));
        let expected = closed(concat!(
            "evidence_fingerprints :     # keep flow note\n",
            "  file:tests/t.rs: new\n",
            "owner: alice\n",
        ));
        let written = set_evidence_fingerprints(&flow, &fresh_evidence)
            .expect("replaces an ordinary flow mapping");
        assert_eq!(written, expected);
        assert_eq!(
            set_evidence_fingerprints(&written, &BTreeMap::new())
                .expect("clears an ordinary mapping"),
            closed("# keep flow note\nowner: alice\n")
        );

        let block = closed(concat!(
            "trace_fingerprints :  # keep trace baseline\n",
            "  HLD.md#x: old\n",
            "owner: alice\n",
        ))
        .replace('\n', "\r\n");
        let expected = closed(concat!(
            "trace_fingerprints :  # keep trace baseline\n",
            "  HLD.md#x: new\n",
            "owner: alice\n",
        ))
        .replace('\n', "\r\n");
        let written = restamp(&block, &fps(&[("HLD.md#x", "new")]))
            .expect("replaces an ordinary block mapping");
        assert_eq!(written, expected);
        assert_eq!(
            restamp(&written, &BTreeMap::new()).expect("clears an ordinary mapping"),
            closed("# keep trace baseline\nowner: alice\n").replace('\n', "\r\n")
        );
    }

    #[test]
    fn fingerprint_source_recogniser_covers_flow_and_scalar_neighbours() {
        fn values(frontmatter: &str) -> Option<BTreeMap<String, String>> {
            ordinary_fingerprint_mapping(frontmatter, "evidence_fingerprints")
                .map(|source| source.values)
        }

        assert_eq!(
            ordinary_string_mapping_member("'file:a': one # reason: retained", true),
            Some(("file:a".to_string(), "one".to_string()))
        );
        assert_eq!(
            ordinary_string_mapping_member("file:b: 'two''s value'", true),
            Some(("file:b".to_string(), "two's value".to_string()))
        );
        assert_eq!(
            ordinary_string_mapping_member("ref: https://example.com", true),
            Some(("ref".to_string(), "https://example.com".to_string()))
        );
        assert_eq!(values("evidence_fingerprints: {}\n"), Some(BTreeMap::new()));
        assert_eq!(
            values(
                "evidence_fingerprints: {'file:a,b': 'one, two', \"file:b\": \"two: 2\"} # map\n",
            ),
            Some(fps(&[("file:a,b", "one, two"), ("file:b", "two: 2")]))
        );
        assert_eq!(
            values("evidence_fingerprints: {'file:a': 'two''s value'}\n"),
            Some(fps(&[("file:a", "two's value")]))
        );
        assert_eq!(
            values(
                r#"evidence_fingerprints: {"file:\"a": "value\"quoted"}
"#
            ),
            Some(fps(&[("file:\"a", "value\"quoted")]))
        );
        assert_eq!(
            values(
                "evidence_fingerprints:\n  'file:a': one # reason: retained\n\
                 # between members\n  file:b: 'two''s value'\nowner: t\n",
            ),
            Some(fps(&[("file:a", "one"), ("file:b", "two's value")]))
        );

        for structural in [
            "evidence_fingerprints: scalar\n",
            "evidence_fingerprints: {file:a: one, nested: {key: value}}\n",
            "evidence_fingerprints: {file:a: one, nested: [value]}\n",
            "evidence_fingerprints: {file:a: one] }\n",
            "evidence_fingerprints: {same: one, same: two}\n",
            "evidence_fingerprints: {file:a: one} trailing\n",
            "evidence_fingerprints: {'unterminated: one}\n",
            "evidence_fingerprints:\n  file:a:\n    nested: value\n",
            "evidence_fingerprints:\n  same: one\n  same: two\n",
        ] {
            assert_eq!(
                values(structural),
                None,
                "structural source was accepted: {structural}"
            );
        }
        assert_eq!(ordinary_string_mapping_member("no colon", true), None);
        assert_eq!(
            ordinary_string_mapping_member("'unterminated: value", true),
            None
        );
        assert_eq!(
            ordinary_string_mapping_member("key: value trailing", false),
            Some(("key".to_string(), "value trailing".to_string()))
        );
        assert_eq!(
            ordinary_string_mapping_member("key: value # comment", false),
            None,
            "flow members cannot carry a line comment inside the braces"
        );
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

    /// A diagnostic and the explicit command reach one surgical edit, rather
    /// than two implementations that merely intend the same change (`A1-199`).
    #[test]
    fn a_typed_repair_uses_the_same_surgical_rewrite_as_amend() {
        let from = ev(EvidenceKind::Test, "old_name");
        let to = ev(EvidenceKind::Test, "new_name");
        let file = closed("");
        let repair = Repair::ReplaceEvidence {
            from: from.clone(),
            to: to.clone(),
        };

        assert_eq!(
            apply_repair(&file, &repair).expect("typed repair"),
            replace_evidence(&file, &from, &to).expect("explicit amend primitive")
        );
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
    fn evidence_edits_refuse_structural_sequences_before_rewriting_them() {
        let structural = [
            "evidence: !!seq\n  - kind: test\n    ref: old_name\n  - kind: file\n    ref: tests/t.rs\n",
            "evidence:\n  - !!map\n    kind: test\n    ref: old_name\n  - kind: file\n    ref: tests/t.rs\n",
            "evidence:\n  - kind: !!str test\n    ref: old_name\n  - kind: file\n    ref: tests/t.rs\n",
            "evidence:\n  - kind: test\n    ref: |-\n      old_name\n  - kind: file\n    ref: tests/t.rs\n",
            "evidence:\n  - 'kind': test\n    ref: old_name\n  - kind: file\n    ref: tests/t.rs\n",
            "evidence: [{kind: test, ref: old_name}, {kind: file, ref: tests/t.rs}]\n",
            "'evidence':\n  - kind: test\n    ref: old_name\n  - kind: file\n    ref: tests/t.rs\n",
        ];
        for evidence in structural {
            let source = closed("").replace(
                "evidence:\n  - kind: test\n    ref: old_name\n  - kind: file\n    ref: tests/t.rs\n",
                evidence,
            );
            assert!(
                matches!(
                    add_evidence(&source, &ev(EvidenceKind::Test, "new_proof")),
                    Err(CloseError::NoEvidenceReplacement)
                ),
                "structural source was accepted: {source}"
            );
        }

        for source in [
            "evidence:\n  -kind: test\n",
            "evidence:\n  -\n",
            "evidence:\n  kind: test\n",
            "evidence:\n  - kind: test\n    extra: no\n",
            "evidence:\n  - kind: test\n    kind: file\n    ref: old_name\n",
        ] {
            assert!(
                ordinary_evidence_sequence(source).is_none(),
                "non-ordinary mapping source was accepted: {source}"
            );
        }

        let anchored = closed("owner: *shared\n").replace("ref: old_name", "ref: &shared old_name");
        let from = ev(EvidenceKind::Test, "old_name");
        let to = ev(EvidenceKind::Test, "new_name");
        for result in [
            add_evidence(&anchored, &ev(EvidenceKind::Test, "new_proof")),
            remove_evidence(&anchored, &from),
            replace_evidence(&anchored, &from, &to),
            apply_repair(
                &anchored,
                &Repair::ReplaceEvidence {
                    from: from.clone(),
                    to: to.clone(),
                },
            ),
        ] {
            assert!(
                matches!(result, Err(CloseError::NoEvidenceReplacement)),
                "unexpected result: {result:?}"
            );
        }

        assert!(matches!(
            replace_evidence_sequence("id: A\n", &[], std::slice::from_ref(&from), "\n"),
            Err(CloseError::NoEvidenceReplacement)
        ));
        let frontmatter = "evidence:\n  - kind: test\n    ref: old_name\nowner: t\n";
        let ordinary = ordinary_evidence_sequence(frontmatter).expect("ordinary evidence source");
        assert!(matches!(
            ordinary.rewrite_delta(frontmatter, std::slice::from_ref(&from), "\n"),
            Err(CloseError::Produced(message))
                if message.contains("exactly one")
        ));
    }

    #[test]
    fn ordinary_evidence_edits_preserve_the_field_boundary_and_absent_case() {
        let source = concat!(
            "---\n",
            "id: A\n",
            "title: t\n",
            "phase: 0\n",
            "status: done\n",
            "evidence : # why this note is retained\n",
            "- kind: note\n",
            "  ref: reviewed\n",
            "  captured: 2026-10-31\n",
            "owner: alice\n",
            "---\n\n",
            "Body stays.\n",
        );
        let added = add_evidence(source, &ev(EvidenceKind::Test, "new_proof"))
            .expect("ordinary unindented sequence is replaceable");
        assert!(
            added.contains("evidence : # why this note is retained\n"),
            "the authored field boundary moved: {added}"
        );
        assert!(added.contains("captured: 2026-10-31"), "{added}");
        assert!(added.contains("ref: new_proof"), "{added}");
        assert!(added.contains("owner: alice"), "{added}");
        assert!(added.ends_with("\nBody stays.\n"), "{added}");

        let cleared = remove_evidence(
            source,
            &Evidence {
                kind: EvidenceKind::Note,
                reference: "reviewed".to_string(),
                captured: None,
            },
        )
        .expect("the last ordinary citation can be removed by the core primitive");
        assert!(!cleared.contains("evidence :"), "{cleared}");
        assert!(
            cleared.contains("# why this note is retained\nowner: alice"),
            "the boundary note was deleted: {cleared}"
        );

        let canonical =
            "evidence:\n  - kind: test\n    ref: old_name\n  - kind: file\n    ref: tests/t.rs\n";
        let absent = closed("").replace(canonical, "");
        let first = add_evidence(&absent, &ev(EvidenceKind::Test, "first_proof"))
            .expect("an old done task can receive its first evidence block");
        assert!(first.contains("evidence:\n  - kind: test\n    ref: first_proof\n"));
    }

    /// Proves `amend.preserves_evidence_sequence_source_deltas`: an accepted
    /// citation sequence is an authored source structure, not just a vector of
    /// values. Each operation changes only its addressed mapping while quote
    /// style, comments, unusual field order and both newline conventions stay
    /// byte-for-byte stable (`A1-722`).
    const ORDINARY_EVIDENCE_SOURCE: &str = concat!(
        "---\n",
        "id: A\n",
        "title: t\n",
        "phase: 0\n",
        "status: done\n",
        "evidence : # evidence boundary\n",
        "- kind : 'test' # first kind\n",
        "  ref : \"alpha\" # first ref\n",
        "# target follows\n",
        "- ref : 'docs/old.md' # target ref\n",
        "  kind : file # target kind\n",
        "  captured: 2026-10-30 # target date\n",
        "# retained citation follows\n",
        "- kind: note # retained kind\n",
        "  ref: reviewed # retained ref\n",
        "  captured : '2026-10-31' # retained date\n",
        "# after evidence\n",
        "owner: alice\n",
        "---\n\n",
        "Body stays exact.\n",
    );

    #[test]
    fn ordinary_evidence_edits_are_exact_source_deltas() {
        let target = ev(EvidenceKind::File, "docs/old.md");

        for crlf in [false, true] {
            let source = if crlf {
                ORDINARY_EVIDENCE_SOURCE.replace('\n', "\r\n")
            } else {
                ORDINARY_EVIDENCE_SOURCE.to_string()
            };
            let nl = if crlf { "\r\n" } else { "\n" };

            let added = add_evidence(
                &source,
                &Evidence {
                    kind: EvidenceKind::Note,
                    reference: "fresh-review".to_string(),
                    captured: Some("2026-11-01".to_string()),
                },
            )
            .expect("add one mapping with captured metadata");
            let expected = source.replace(
                &format!("  captured : '2026-10-31' # retained date{nl}"),
                &format!(
                    "  captured : '2026-10-31' # retained date{nl}- kind: note{nl}  ref: fresh-review{nl}  captured: 2026-11-01{nl}"
                ),
            );
            assert_eq!(added, expected, "addition rewrote existing source");

            let replaced = replace_evidence(&source, &target, &ev(EvidenceKind::Test, "beta"))
                .expect("replace the middle mapping");
            let expected = source.replace(
                &format!(
                    "- ref : 'docs/old.md' # target ref{nl}  kind : file # target kind{nl}  captured: 2026-10-30 # target date{nl}"
                ),
                &format!(
                    "- ref : 'beta' # target ref{nl}  kind : test # target kind{nl}"
                ),
            );
            assert_eq!(replaced, expected, "replacement rewrote a neighbour");

            let with_captured = replace_evidence(
                &source,
                &ev(EvidenceKind::Test, "alpha"),
                &Evidence {
                    kind: EvidenceKind::File,
                    reference: "docs/new.md".to_string(),
                    captured: Some("2026-11-01".to_string()),
                },
            )
            .expect("replace and add captured metadata");
            let expected = source.replace(
                &format!(
                    "- kind : 'test' # first kind{nl}  ref : \"alpha\" # first ref{nl}"
                ),
                &format!(
                    "- kind : 'file' # first kind{nl}  ref : \"docs/new.md\" # first ref{nl}  captured: 2026-11-01{nl}"
                ),
            );
            assert_eq!(
                with_captured, expected,
                "captured insertion rewrote existing source"
            );

            let changed_captured = replace_evidence(
                &source,
                &target,
                &Evidence {
                    kind: EvidenceKind::File,
                    reference: "docs/new.md".to_string(),
                    captured: Some("2026-11-02".to_string()),
                },
            )
            .expect("replace captured metadata in place");
            let expected = source.replace(
                &format!(
                    "- ref : 'docs/old.md' # target ref{nl}  kind : file # target kind{nl}  captured: 2026-10-30 # target date{nl}"
                ),
                &format!(
                    "- ref : 'docs/new.md' # target ref{nl}  kind : file # target kind{nl}  captured: 2026-11-02 # target date{nl}"
                ),
            );
            assert_eq!(
                changed_captured, expected,
                "captured replacement rewrote existing source"
            );

            let removed = remove_evidence(&source, &target).expect("remove the middle mapping");
            let expected = source.replace(
                &format!(
                    "- ref : 'docs/old.md' # target ref{nl}  kind : file # target kind{nl}  captured: 2026-10-30 # target date{nl}"
                ),
                "",
            );
            assert_eq!(removed, expected, "removal rewrote a neighbour");
        }
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
        close(file, evidence, &BTreeMap::new(), None)
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
        let task: Task = yaml_compat::from_str(fm).unwrap();
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
        let task: Task = yaml_compat::from_str(fm).unwrap();
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

    /// Proves `close.refuses_alternate_evidence_keys`: every authored
    /// top-level key which the loader resolves to `evidence` is present even
    /// when the source does not use the ordinary literal spelling. Initial
    /// close refuses before appending a duplicate field and preserves the
    /// complete input (`A1-723`).
    #[test]
    fn initial_close_refuses_alternate_evidence_keys_before_candidate_construction() {
        let forms = [
            "'evidence': []\n",
            "\"evidence\": []\n",
            "\"\\u0065vidence\": []\n",
            "? evidence\n: []\n",
            "!!str evidence: []\n",
            "&evidence_key evidence: []\n",
            "owner: &evidence_key evidence\n*evidence_key: []\n",
            "\"evidence\":\n  - kind: file\n    ref: proof.txt\n",
        ];
        for field in forms {
            let source = TASK
                .replace("owner: kaminsod\n", "")
                .replace("status: todo\n", &format!("status: todo\n{field}"));
            let error = close_ev(&source, &[commit("x")])
                .expect_err("an alternate evidence key is an existing field");
            assert!(
                matches!(error, CloseError::HasEvidence),
                "{field:?}: {error:?}"
            );
            let said = error.to_string();
            assert!(
                said.contains("live task") && said.contains("Review"),
                "{said}"
            );
            assert!(
                !said.contains("amend"),
                "the named remedy cannot accept this task: {said}"
            );
        }

        let absent = close_ev(TASK, &[commit("x")]).expect("an absent field remains appendable");
        assert!(absent.contains("evidence:\n  - kind: commit\n    ref: x\n"));
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
        let task: Task = yaml_compat::from_str(fm).unwrap();
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
            yaml_compat::from_str::<Task>(fm).unwrap().status,
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
            None,
        )
        .unwrap();
        assert!(out.contains("trace_fingerprints:"), "{out}");
        let (fm, _) = split_frontmatter(&out).unwrap();
        let task: Task = yaml_compat::from_str(fm).unwrap();
        assert_eq!(
            task.trace_fingerprints.get("HLD.md#x").map(String::as_str),
            Some("deadbeef")
        );
        // With no fingerprints there is no block (byte-identical to before).
        let plain = close_ev(TASK, &[commit("abc1234")]).unwrap();
        assert!(!plain.contains("trace_fingerprints"));
    }

    /// Proves the named behaviour `close.replaces_live_trace_fingerprints`.
    #[test]
    fn close_replaces_a_live_tasks_existing_fingerprints_with_fresh_stamps() {
        let live = "\
---
id: A1-003
title: Ship it
phase: 1
status: in_progress
trace_fingerprints:
  HLD.md#old: stale
owner: kaminsod
effort_days: 1
paths: [HLD.md]
traces: [HLD.md#new]
---

## Why

Keep this body byte-for-byte.
";
        let evidence = [commit("first"), commit("second")];
        let fresh = fps(&[("HLD.md#new", "fresh")]);

        let out = close(live, &evidence, &fresh, None).expect("close accepted live task");

        assert_eq!(
            out.matches("trace_fingerprints:").count(),
            1,
            "close emitted a duplicate top-level key: {out}"
        );
        assert!(!out.contains("stale"), "the old stamp survived: {out}");
        assert!(
            out.find("trace_fingerprints:").unwrap() < out.find("owner:").unwrap(),
            "the replacement moved instead of staying in place: {out}"
        );
        assert!(
            out.ends_with("## Why\n\nKeep this body byte-for-byte.\n"),
            "the body changed: {out}"
        );
        let (frontmatter, _) = split_frontmatter(&out).unwrap();
        let task: Task = yaml_compat::from_str(frontmatter).unwrap();
        assert_eq!(task.status, Status::Done);
        assert_eq!(task.evidence, evidence);
        assert_eq!(task.trace_fingerprints, fresh);

        // No freshly resolved stamps is authoritative too (for example when
        // drift detection is now off): the stale live block is removed rather
        // than retained or replaced with an empty key.
        let cleared = close(live, &evidence, &BTreeMap::new(), None).expect("close without stamps");
        assert!(!cleared.contains("trace_fingerprints:"), "{cleared}");
        let (frontmatter, _) = split_frontmatter(&cleared).unwrap();
        let task: Task = yaml_compat::from_str(frontmatter).unwrap();
        assert!(task.trace_fingerprints.is_empty());
        assert_eq!(task.evidence, evidence);
    }

    #[test]
    fn close_and_restamp_refuse_shared_fingerprint_anchors_before_rewriting() {
        let live = "\
---
id: A1-003
title: Ship it
phase: 1
status: in_progress
trace_fingerprints: &stamps
  HLD.md#old: stale
evidence_fingerprints: *stamps
effort_days: 1
paths: [HLD.md]
---

body sentinel
";
        let before: Task = yaml_compat::from_str(split_frontmatter(live).unwrap().0)
            .expect("the shared mapping is accepted YAML");
        let error = close(
            live,
            &[commit("abc1234")],
            &fps(&[("HLD.md#new", "fresh")]),
            None,
        )
        .expect_err("close must recognise the source structure before replacing it");
        assert!(
            matches!(
                &error,
                CloseError::NoMappingReplacement("trace_fingerprints")
            ),
            "{error}"
        );
        assert!(!matches!(&error, CloseError::Produced(_)), "{error}");
        let unchanged: Task = yaml_compat::from_str(split_frontmatter(live).unwrap().0).unwrap();
        assert_eq!(unchanged, before);

        let done = live.replace("status: in_progress", "status: done");
        let error = restamp(&done, &fps(&[("HLD.md#new", "fresh")]))
            .expect_err("restamp must recognise the same source structure");
        assert!(
            matches!(
                &error,
                CloseError::NoMappingReplacement("trace_fingerprints")
            ),
            "{error}"
        );
        assert!(!matches!(&error, CloseError::Produced(_)), "{error}");
    }

    /// Proves the named behaviour
    /// `mutation.block_replacements_cross_yaml_trivia` for a mapping value.
    #[test]
    fn close_replaces_every_old_fingerprint_across_yaml_trivia() {
        let live = "\
---
id: A1-003
title: Ship it
phase: 1
status: in_progress
trace_fingerprints:
  HLD.md#first: stale-first

# Keep this author note.
  HLD.md#second: stale-second
owner: kaminsod
effort_days: 1
paths: [HLD.md]
traces: [HLD.md#new]
---

body sentinel
";
        let evidence = [commit("first"), commit("second")];
        let fresh = fps(&[("HLD.md#new", "fresh-one"), ("HLD.md#other", "fresh-two")]);

        let out = close(live, &evidence, &fresh, None).expect("close accepted valid YAML trivia");
        assert_eq!(out.matches("trace_fingerprints:").count(), 1, "{out}");
        assert!(!out.contains("stale-first"), "{out}");
        assert!(!out.contains("stale-second"), "{out}");
        assert!(
            out.contains(
                "  HLD.md#new: fresh-one\n  HLD.md#other: fresh-two\n\n# Keep this author note.\nowner: kaminsod"
            ),
            "the trivia or following field moved: {out}"
        );
        assert!(out.ends_with("\nbody sentinel\n"), "{out}");
        let (frontmatter, _) = split_frontmatter(&out).unwrap();
        let task: Task = yaml_compat::from_str(frontmatter).unwrap();
        assert_eq!(task.status, Status::Done);
        assert_eq!(task.evidence, evidence);
        assert_eq!(task.trace_fingerprints, fresh);

        let cleared = close(live, &evidence, &BTreeMap::new(), None)
            .expect("an empty replacement accepts the same trivia");
        assert!(!cleared.contains("trace_fingerprints:"), "{cleared}");
        assert!(!cleared.contains("stale-first"), "{cleared}");
        assert!(!cleared.contains("stale-second"), "{cleared}");
        assert!(
            cleared.contains("status: done\n\n# Keep this author note.\nowner: kaminsod"),
            "the trivia or following field moved: {cleared}"
        );
        let (frontmatter, _) = split_frontmatter(&cleared).unwrap();
        let task: Task = yaml_compat::from_str(frontmatter).unwrap();
        assert!(task.trace_fingerprints.is_empty());
        assert_eq!(task.evidence, evidence);

        // This replacement branch has the same byte-preservation contract as
        // ordinary close: a BOM survives and no lone LF enters a CRLF file.
        let decorated = format!("\u{feff}{}", live.replace('\n', "\r\n"));
        let decorated = close(&decorated, &evidence, &fresh, None)
            .expect("BOM and CRLF do not change the accepted mapping");
        assert!(decorated.starts_with('\u{feff}'), "BOM was dropped");
        assert!(
            !decorated.replace("\r\n", "").contains('\n'),
            "a lone LF entered the CRLF file: {decorated:?}"
        );
        assert!(!decorated.contains("stale-first"), "{decorated}");
        assert!(!decorated.contains("stale-second"), "{decorated}");
        let (frontmatter, _) = split_frontmatter(&decorated).unwrap();
        let task: Task = yaml_compat::from_str(frontmatter).unwrap();
        assert_eq!(task.trace_fingerprints, fresh);
        assert_eq!(task.evidence, evidence);
    }

    /// Proves the named behaviour
    /// `mutation.block_replacements_cross_yaml_trivia` for sequence values and
    /// for two independent callers of the shared surgical block editor.
    #[test]
    fn block_replacements_cross_yaml_trivia() {
        let dependencies = "\
---
id: A
title: t
phase: 1
status: todo
blocked_by:
  - B

# Keep the dependency note.
  - C
tags:
  - cli
---

body
";
        let replaced = set_blocked_by(dependencies, &["D".to_string()])
            .expect("dependency replacement crosses YAML trivia");
        let (frontmatter, _) = split_frontmatter(&replaced).unwrap();
        let task: Task = yaml_compat::from_str(frontmatter).unwrap();
        assert_eq!(task.blocked_by, ["D"]);
        assert_eq!(task.tags, ["cli"]);
        assert!(!replaced.contains("  - B"), "{replaced}");
        assert!(!replaced.contains("  - C"), "{replaced}");
        assert!(
            replaced.contains("  - D\n\n# Keep the dependency note.\ntags:\n"),
            "the trivia or following field moved: {replaced}"
        );
        assert!(replaced.ends_with("\nbody\n"), "{replaced}");

        let proof = "\
---
id: A
title: t
phase: 1
status: done
evidence:
  - kind: test
    ref: old_name

# Keep the evidence note.
  - kind: file
    ref: tests/t.rs
paths:
  - tests/**
---

body
";
        let added = commit("abc123");
        let replaced =
            add_evidence(proof, &added).expect("evidence replacement crosses YAML trivia");
        let (frontmatter, _) = split_frontmatter(&replaced).unwrap();
        let task: Task = yaml_compat::from_str(frontmatter).unwrap();
        assert_eq!(
            task.evidence,
            [
                ev(EvidenceKind::Test, "old_name"),
                ev(EvidenceKind::File, "tests/t.rs"),
                added,
            ]
        );
        assert_eq!(replaced.matches("ref: tests/t.rs").count(), 1, "{replaced}");
        assert!(
            replaced.contains("    ref: old_name\n\n# Keep the evidence note.\n  - kind: file\n"),
            "standalone evidence trivia moved: {replaced}"
        );
        assert!(
            replaced.contains("    ref: tests/t.rs\n  - kind: commit\n    ref: abc123\npaths:\n"),
            "the appended citation or following field moved: {replaced}"
        );
        assert!(replaced.ends_with("\nbody\n"), "{replaced}");
    }

    /// **A release records the version it produced when it closes** (`A1-339`,
    /// ADR-0038), and a record already there and disagreeing stops the closure.
    #[test]
    fn closing_a_release_records_the_version_it_produced() {
        let release = "---\nid: M-r\ntitle: r\nphase: 0\nstatus: todo\n\
                       tags:\n  - milestone\n  - release\n---\n\nbody\n";
        let out = close(
            release,
            &[commit("abc1234")],
            &BTreeMap::new(),
            Some("2.0.0"),
        )
        .unwrap();
        let (fm, _) = split_frontmatter(&out).unwrap();
        let task: Task = yaml_compat::from_str(fm).unwrap();
        assert_eq!(task.version.as_deref(), Some("2.0.0"));
        assert_eq!(task.status, Status::Done);

        // Already recorded and agreeing: the file's own line is left alone
        // rather than written twice, which would be a duplicate-key document.
        let agreeing = "---\nid: M-r\ntitle: r\nphase: 0\nstatus: todo\n\
                        tags:\n  - milestone\n  - release\nversion: 2.0.0\n---\n\nbody\n";
        let out = close(
            agreeing,
            &[commit("abc1234")],
            &BTreeMap::new(),
            Some("2.0.0"),
        )
        .unwrap();
        assert_eq!(
            out.matches("version:").count(),
            1,
            "a second `version:` key would not load: {out}"
        );

        // Already recorded and disagreeing: refused, with both numbers named.
        // One of the two is wrong and this tool cannot tell which.
        let disagreeing = "---\nid: M-r\ntitle: r\nphase: 0\nstatus: todo\n\
                           tags:\n  - milestone\n  - release\nversion: 1.5.0\n---\n\nbody\n";
        let err = close(
            disagreeing,
            &[commit("abc1234")],
            &BTreeMap::new(),
            Some("2.0.0"),
        )
        .expect_err("a disagreement is not this tool's to resolve");
        let said = err.to_string();
        assert!(said.contains("1.5.0") && said.contains("2.0.0"), "{said}");

        // And nothing is written for a closure that derives no version, which is
        // every ordinary task.
        let out = close(release, &[commit("abc1234")], &BTreeMap::new(), None).unwrap();
        assert!(!out.contains("version:"), "{out}");
    }

    /// An authored empty value is a present source field even though the typed
    /// optional value is `None`. Proves the core half of
    /// `close.refuses_empty_version_fields`: every YAML spelling which resolves
    /// to an empty top-level `version` is refused before a derived record could
    /// be appended beside it (`A1-724`).
    #[test]
    fn initial_close_refuses_empty_version_fields_before_candidate_construction() {
        let forms = [
            "version:\n",
            "version: null\n",
            "version: ~\n",
            "'version': null\n",
            "\"version\": null\n",
            "\"vers\\u0069on\": null\n",
            "? version\n: null\n",
            "!!str version: null\n",
            "&version_key version: null\n",
            "owner: &version_key version\n*version_key: null\n",
        ];
        for field in forms {
            let release = format!(
                "---\nid: M-r\ntitle: r\nphase: 0\nstatus: todo\n\
                 tags:\n  - milestone\n  - release\n{field}---\n\nbody\n"
            );
            let error = close(
                &release,
                &[commit("abc1234")],
                &BTreeMap::new(),
                Some("2.0.0"),
            )
            .expect_err("a present empty version field cannot receive a duplicate record");
            assert!(
                matches!(
                    &error,
                    CloseError::EmptyVersion { derived } if derived == "2.0.0"
                ),
                "{field:?}: {error:?}"
            );
            let said = error.to_string();
            assert!(
                said.contains("no value")
                    && said.contains("2.0.0")
                    && said.contains("remove the empty field"),
                "{field:?}: {said}"
            );
        }
    }

    #[test]
    fn restamp_replaces_the_block_on_a_done_task() {
        let done = close(
            TASK,
            &[commit("abc1234")],
            &fps(&[("HLD.md#x", "0000")]),
            None,
        )
        .unwrap();
        let fresh = restamp(&done, &fps(&[("HLD.md#x", "ffff")])).unwrap();
        let (fm, _) = split_frontmatter(&fresh).unwrap();
        let task: Task = yaml_compat::from_str(fm).unwrap();
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
        let _: Task = yaml_compat::from_str(fm).expect("loads");
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
        assert!(
            out.contains("blocked_by: [\"X\"] # the first wave\n"),
            "{out}"
        );
        let (fm, _) = split_frontmatter(&out).expect("frontmatter");
        let task: Task = yaml_compat::from_str(fm).expect("loads");
        assert_eq!(task.blocked_by, vec!["X".to_string()]);
    }

    #[test]
    fn a_rekey_keeps_member_trivia_around_escaped_quotes_and_brackets() {
        let file = "---\nid: A\ntitle: T\nphase: 1\nstatus: todo\n\
                    blocked_by: [  \"a\\\"]b\" , 'C'  ] # the first wave\n---\n\nbody\n";
        let out =
            rekey_blocked_by(file, &["X".to_string(), "C".to_string()]).expect("rekeys the member");
        assert!(
            out.contains("blocked_by: [  \"X\" , 'C'  ] # the first wave\n"),
            "{out}"
        );
        let (fm, _) = split_frontmatter(&out).expect("frontmatter");
        let task: Task = yaml_compat::from_str(fm).expect("loads");
        assert_eq!(task.blocked_by, vec!["X".to_string(), "C".to_string()]);
    }

    #[test]
    fn a_quoted_id_with_whitespace_is_rewritten_as_one_scalar() {
        let file = "\u{feff}---\r\nid : \"T old\"  # identity\r\ntitle: T\r\nphase: 1\r\nstatus: todo\r\neffort_days: 1\r\n---\r\n\r\nBody.\r\n";
        let out = set_id(file, "T new").expect("rewrites the whole quoted scalar");
        assert!(
            out.starts_with("\u{feff}---\r\nid : T new  # identity\r\n"),
            "{out:?}"
        );
        assert!(out.ends_with("---\r\n\r\nBody.\r\n"), "{out:?}");
        let (frontmatter, _) = split_frontmatter(&out).expect("frontmatter");
        assert_eq!(
            yaml_compat::from_str::<Task>(frontmatter).expect("task").id,
            "T new"
        );

        let single = file.replace("\"T old\"", "'T old'");
        let single = set_id(&single, "T 'new").expect("single-quoted source");
        let (frontmatter, _) = split_frontmatter(&single).expect("frontmatter");
        assert_eq!(
            yaml_compat::from_str::<Task>(frontmatter).expect("task").id,
            "T 'new"
        );

        let escaped = file.replace("\"T old\"", "\"T \\\"old\\\"\"");
        let escaped = set_id(&escaped, "T new").expect("escaped double-quoted source");
        let (frontmatter, _) = split_frontmatter(&escaped).expect("frontmatter");
        assert_eq!(
            yaml_compat::from_str::<Task>(frontmatter).expect("task").id,
            "T new"
        );

        let structural = file.replace("id : \"T old\"  # identity", "id: &identity T-old");
        assert!(matches!(
            set_id(&structural, "T-new"),
            Err(CloseError::NoScalar("id"))
        ));
    }

    #[test]
    fn multiline_plain_scalars_refuse_every_line_local_scalar_edit() {
        let id =
            "---\nid: T-\n  old\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nBody.\n";
        let (frontmatter, _) = split_frontmatter(id).expect("frontmatter");
        assert_eq!(
            yaml_compat::from_str::<Task>(frontmatter)
                .expect("valid multiline id")
                .id,
            "T- old"
        );
        let error = set_id(id, "T-new").expect_err("a multiline id is not one line to replace");
        assert!(matches!(error, CloseError::NoScalar("id")), "{error:?}");
        assert!(
            error.to_string().contains("written below the key"),
            "{error}"
        );

        for (owner, loaded) in [
            (
                "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nowner: ada\n  lovelace\n---\n\nBody.\n",
                "ada lovelace",
            ),
            (
                "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nowner:\n  ada\n---\n\nBody.\n",
                "ada",
            ),
        ] {
            let (frontmatter, _) = split_frontmatter(owner).expect("frontmatter");
            assert_eq!(
                yaml_compat::from_str::<Task>(frontmatter)
                    .expect("valid multiline owner")
                    .owner
                    .as_deref(),
                Some(loaded)
            );
            for to in [None, Some("grace")] {
                let error = set_owner(owner, to)
                    .expect_err("setting or clearing a multiline owner must share one refusal");
                assert!(matches!(error, CloseError::NoScalar("owner")), "{error:?}");
            }
        }

        for (revisit, loaded) in [
            (
                "---\nid: T\ntitle: T\nphase: 1\nstatus: blocked\neffort_days: 1\nrevisit_at: 2030-01-02T03:04:05Z\n  after-review\n---\n\nBody.\n",
                "2030-01-02T03:04:05Z after-review",
            ),
            (
                "---\nid: T\ntitle: T\nphase: 1\nstatus: blocked\neffort_days: 1\nrevisit_at:\n  2030-01-02T03:04:05Z\n---\n\nBody.\n",
                "2030-01-02T03:04:05Z",
            ),
        ] {
            let (frontmatter, _) = split_frontmatter(revisit).expect("frontmatter");
            assert_eq!(
                yaml_compat::from_str::<Task>(frontmatter)
                    .expect("valid multiline revisit instant source")
                    .revisit_at
                    .as_deref(),
                Some(loaded)
            );
            for to in [None, Some("2031-02-03T04:05:06Z")] {
                let error = set_revisit_at(revisit, to)
                    .expect_err("setting or clearing multiline revisit source must refuse");
                assert!(
                    matches!(error, CloseError::NoScalar("revisit_at")),
                    "{error:?}"
                );
            }
        }
    }

    #[test]
    fn source_local_scalar_detection_requires_one_valid_mapping_line() {
        assert!(has_single_source_local_scalar_line("id: A\n", "id"));
        assert!(
            !has_single_source_local_scalar_line("title: A\n", "id"),
            "an absent source line was treated as editable"
        );
        assert!(
            !has_single_source_local_scalar_line("id: A\nid: B\n", "id"),
            "two physical source owners were treated as one scalar"
        );
        assert!(
            !has_single_source_local_scalar_line("id: [\n", "id"),
            "an unreadable mapping was treated as an editable scalar"
        );
    }

    #[test]
    fn a_rekey_keeps_every_block_member_comment_and_quote() {
        let file = "---\r\nid: T\r\ntitle: T\r\nphase: 1\r\nstatus: todo\r\neffort_days: 1\r\nblocked_by: # mandatory\r\n  - 'T-old' # changed edge\r\n  - T-keep # unchanged edge\r\nprefer_after:\r\n- \"T-old\" # preference\r\nratified_by:\r\n  # approval context\r\n  - T-old # approval\r\ncontains:\r\n  - T-old # child\r\n---\r\n\r\nBody.\r\n";
        let out = rekey_blocked_by(file, &["T-new".into(), "T-keep".into()]).expect("blocked_by");
        let out = rekey_prefer_after(&out, &["T-new".into()]).expect("prefer_after");
        let out = set_ratified_by(&out, &["T-new".into()]).expect("ratified_by");
        let out = set_contains(&out, &["T-new".into()]).expect("contains");

        assert!(out.contains("blocked_by: # mandatory\r\n"), "{out:?}");
        assert!(out.contains("  - 'T-new' # changed edge\r\n"), "{out:?}");
        assert!(out.contains("  - T-keep # unchanged edge\r\n"), "{out:?}");
        assert!(out.contains("- \"T-new\" # preference\r\n"), "{out:?}");
        assert!(out.contains("  # approval context\r\n"), "{out:?}");
        assert!(out.contains("  - T-new # approval\r\n"), "{out:?}");
        assert!(out.contains("  - T-new # child\r\n"), "{out:?}");
        assert!(out.ends_with("---\r\n\r\nBody.\r\n"), "{out:?}");

        let unsupported = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by:\n  - |-\n    T-old\n---\n";
        assert!(matches!(
            rekey_blocked_by(unsupported, &["T-new".into()]),
            Err(CloseError::NoScalar("blocked_by"))
        ));
    }

    /// Proves `dep.preserves_scalar_sequence_source_deltas` at the shared
    /// primitive: retained members and every piece of surrounding trivia stay
    /// byte-identical while removals and insertions change only their own lines.
    #[test]
    fn scalar_sequence_deltas_preserve_retained_members_and_trivia() {
        let file = "---\r\nid: T\r\ntitle: T\r\nphase: 1\r\nstatus: todo\r\neffort_days: 1\r\nblocked_by: # required order\r\n- 'A-1' # first\r\n# why the middle existed\r\n- \"A-2\" # remove this\r\n- A-3 # retained\r\npaths: [src/**]\r\n---\r\n\r\nBody.\r\n";
        let out = set_blocked_by(file, &["A-1".into(), "A-3".into(), "A-4".into()])
            .expect("applies one source-preserving delta");

        assert_eq!(
            out,
            "---\r\nid: T\r\ntitle: T\r\nphase: 1\r\nstatus: todo\r\neffort_days: 1\r\nblocked_by: # required order\r\n- 'A-1' # first\r\n# why the middle existed\r\n- A-3 # retained\r\n- A-4\r\npaths: [src/**]\r\n---\r\n\r\nBody.\r\n"
        );

        let preferred = file.replace("blocked_by", "prefer_after");
        let preferred = set_prefer_after(&preferred, &["A-1".into(), "A-3".into(), "A-4".into()])
            .expect("preferences use the same primitive");
        assert!(preferred.contains("prefer_after: # required order\r\n"));
        assert!(preferred.contains("- 'A-1' # first\r\n"));
        assert!(preferred.contains("- A-3 # retained\r\n- A-4\r\n"));
        assert!(!preferred.contains("remove this"));
    }

    #[test]
    fn scalar_sequence_deltas_preserve_flow_member_tokens_and_comments() {
        let file = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [  'A-1' , \"A-2\" , A-3  ] # order\n---\n";
        let out =
            set_blocked_by(file, &["A-1".into(), "A-3".into(), "A-4".into()]).expect("flow delta");
        assert!(
            out.contains("blocked_by: [  'A-1' , A-3  , A-4  ] # order\n"),
            "{out}"
        );
    }

    #[test]
    fn scalar_sequence_deltas_infer_empty_and_quoted_flow_styles() {
        let empty = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [] # initially empty\n---\n";
        assert_eq!(
            set_blocked_by(empty, &[]).expect("an unchanged list is unchanged"),
            empty
        );
        assert_eq!(
            set_blocked_by(empty, &["A-1".into(), "A-2".into()]).expect("adds to empty flow"),
            "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1, A-2] # initially empty\n---\n"
        );

        let spaced = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1,  A-2]\n---\n";
        assert_eq!(
            set_blocked_by(spaced, &["A-1".into(), "A-new".into(), "A-2".into()])
                .expect("infers spacing from the next member"),
            "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1,  A-new,  A-2]\n---\n"
        );

        let quoted = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [\"A\\\"1\", 'A''2']\n---\n";
        assert_eq!(
            set_blocked_by(quoted, &["A\"1".into(), "A-new".into(), "A'2".into()])
                .expect("quoted separators remain source tokens"),
            "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [\"A\\\"1\", \"A-new\", 'A''2']\n---\n"
        );
    }

    #[test]
    fn scalar_sequence_delta_parsers_refuse_neighbouring_structural_shapes() {
        assert!(flow_members("'unterminated").is_none());
        assert!(flow_members("[nested]").is_none());

        let before = vec!["A-1".to_string()];
        let after = vec!["A-1".to_string(), "A-2".to_string()];
        assert!(
            rewrite_inline_delta("blocked_by [A-1]\n", "blocked_by", &before, &after, "\n")
                .is_none()
        );
        assert!(
            rewrite_inline_delta(
                "blocked_by: [A-1] trailing\n",
                "blocked_by",
                &before,
                &after,
                "\n",
            )
            .is_none()
        );
        assert!(
            rewrite_inline_delta(
                "blocked_by: [source-value]\n",
                "blocked_by",
                &["model-value".into()],
                &["model-value".into(), "new".into()],
                "\n",
            )
            .is_none()
        );

        let source = block_member(0, "- A-1").expect("scalar member without a final newline");
        let mut written = String::new();
        write_block_member(&mut written, &source, "A-2", "\n");
        assert_eq!(written, "- A-2\n");

        let structural = "blocked_by:\n  nested: value\n";
        let lines: Vec<_> = structural.split_inclusive('\n').collect();
        assert!(block_sequence(&lines, "blocked_by", &[]).is_none());
    }

    #[test]
    fn indented_sequence_deltas_infer_each_insertion_position_without_reformatting() {
        let file = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: # order\n  - 'A-1' # one\n  - \"A-3\" # three\npaths: [src/**]\n---\n";
        let out = set_blocked_by(
            file,
            &[
                "A-0".into(),
                "A-1".into(),
                "A-2".into(),
                "A-3".into(),
                "A-4".into(),
            ],
        )
        .expect("inserts before, between and after retained members");
        assert_eq!(
            out,
            "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: # order\n  - 'A-0'\n  - 'A-1' # one\n  - \"A-2\"\n  - \"A-3\" # three\n  - \"A-4\"\npaths: [src/**]\n---\n"
        );
    }

    #[test]
    fn scalar_sequence_deltas_refuse_ambiguous_or_structural_source() {
        let duplicate = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by:\n- A-1\n- A-1\n---\n";
        assert!(matches!(
            set_blocked_by(duplicate, &[]),
            Err(CloseError::NoSequenceDelta("blocked_by"))
        ));

        let block_scalar = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by:\n- |-\n  A-1\n---\n";
        assert!(matches!(
            set_blocked_by(block_scalar, &["A-1".into(), "A-2".into()]),
            Err(CloseError::NoSequenceDelta("blocked_by"))
        ));

        let multiline_flow = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1,\n  A-2]\n---\n";
        assert!(matches!(
            set_blocked_by(multiline_flow, &["A-1".into()]),
            Err(CloseError::NoSequenceDelta("blocked_by"))
        ));

        let reordered = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [A-1, A-2]\n---\n";
        assert!(matches!(
            set_blocked_by(reordered, &["A-2".into(), "A-1".into()]),
            Err(CloseError::NoSequenceDelta("blocked_by"))
        ));

        assert!(
            rewrite_block_delta(
                "blocked_by:\n  - source-value\n",
                "blocked_by",
                &["model-value".into()],
                &["model-value".into(), "new".into()],
                "\n",
            )
            .is_none(),
            "the source parser must agree with the model before it edits"
        );
    }

    #[test]
    fn removing_the_final_sequence_member_keeps_neighbouring_comments() {
        let block = "---\nid: T\ntitle: T\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: # why this existed\n  # retained context\n  - A-1 # last edge\n# following context\npaths: [src/**]\n---\n";
        let out = set_blocked_by(block, &[]).expect("removes final block member");
        assert!(!out.contains("blocked_by:"), "{out}");
        assert!(!out.contains("last edge"), "{out}");
        assert!(out.contains("# why this existed\n"), "{out}");
        assert!(out.contains("  # retained context\n"), "{out}");
        assert!(out.contains("# following context\npaths:"), "{out}");

        let flow = block.replace(
            "blocked_by: # why this existed\n  # retained context\n  - A-1 # last edge\n",
            "blocked_by: [A-1] # why this existed\n",
        );
        let out = set_blocked_by(&flow, &[]).expect("removes final flow member");
        assert!(!out.contains("blocked_by:"), "{out}");
        assert!(out.contains("# why this existed\n"), "{out}");
    }

    #[test]
    fn surgical_scalar_helpers_refuse_every_shape_they_cannot_preserve() {
        assert_eq!(inline_scalar_end("'T''old' # note"), Some("'T''old'".len()));
        assert_eq!(inline_scalar_end("\"unterminated"), None);
        assert_eq!(inline_scalar_end("   "), None);
        assert_eq!(closing_bracket("['T''old']"), Some("['T''old'".len()));
        assert_eq!(closing_bracket("['unterminated"), None);

        let old = vec!["T-old".to_string()];
        let new = vec!["T-new".to_string()];
        assert_eq!(
            rewrite_block_members("blocked_by:\n  - T-old\n", "blocked_by", &old, &[]),
            None,
            "a one-for-one editor must not remove a member"
        );
        assert_eq!(
            rewrite_block_members("blocked_by:\n  nested: T-old\n", "blocked_by", &old, &new),
            None,
            "an indented mapping is not a sequence member"
        );
        assert_eq!(
            rewrite_block_members("blocked_by:\n  -T-old\n", "blocked_by", &old, &new),
            None,
            "a dash without separation is not a sequence indicator"
        );
        assert_eq!(
            rewrite_block_members("blocked_by:\n  - T-other\n", "blocked_by", &old, &new),
            None,
            "the text pass must agree with the parsed member it is changing"
        );
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
            let _: Task = yaml_compat::from_str(fm).expect("loads");
        }
    }

    /// An id is written as asked, whatever it is made of.
    ///
    /// These seven are the values that defeated the writer while `set_blocked_by`
    /// was being built: three that do not parse as bare scalars at all, and four
    /// that parse into something other than what was asked for. They were refused
    /// then, because the writer emitted them raw and reading the result back was
    /// the only way to notice (`A1-124`).
    ///
    /// They are *written* now, because the scalar is serialised rather than
    /// pasted (`A1-213`, audit). Whether `a: b` names a task is a question about
    /// the plan's vocabulary, which `check` answers with `APLAN002`; refusing it
    /// here answered it with "this is a bug in `aplan`, please report it", about
    /// a value the caller had supplied. The round-trip guard stays exactly where
    /// it was -- it is what caught that original defect, and what would catch the
    /// next one -- but it now guards the writer rather than the vocabulary.
    #[test]
    fn an_id_is_written_as_asked_whatever_it_is_made_of() {
        let file = "---\nid: A\ntitle: T\nphase: 1\nstatus: todo\n---\n\nbody\n";
        for awkward in ["a\nb", "a: b", "[a", "#c", "a #c", " a", "a "] {
            let out = set_blocked_by(file, &[awkward.to_string()])
                .unwrap_or_else(|e| panic!("`{awkward}` was refused: {e}"));
            let (fm, _) = split_frontmatter(&out).expect("frontmatter");
            let task: Task = yaml_compat::from_str(fm).expect("loads");
            assert_eq!(
                task.blocked_by,
                vec![awkward.to_string()],
                "`{awkward}` came back changed: {out}"
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
        let task: Task = yaml_compat::from_str(fm).expect("loads");
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
        let task: Task = yaml_compat::from_str(fm).expect("the result loads");
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
        let task: Task = yaml_compat::from_str(fm).expect("the result loads");
        assert!(task.blocked_by.is_empty(), "{out}");
        assert_eq!(task.tags, vec!["x".to_string()], "{out}");
    }

    /// **An owner is recorded, replaced and cleared without touching anything
    /// else** (`A1-359`).
    ///
    /// The three shapes a claim writer has to handle, and the third is the one
    /// that bites: `owner` is `Option<String>` with `skip_serializing_if`, so
    /// unassigned is the key being *absent*. Leaving `owner:` behind with no
    /// value is a `null` the schema refuses, which is the defect `A1-363` found in
    /// `blocked_by` one layer down.
    #[test]
    fn an_owner_is_recorded_replaced_and_dropped() {
        let unowned = "---\nid: A1-001\ntitle: t\nphase: 1\nstatus: todo\n\
                       effort_days: 1\n---\n\nbody\n";
        let assigned = set_owner(unowned, Some("ada")).expect("record");
        assert!(assigned.contains("owner: ada"), "{assigned}");
        // Placed where every file this tool writes puts it, so the diff is one
        // line rather than a reordering.
        let keys: Vec<&str> = assigned
            .lines()
            .filter_map(|l| l.split(':').next())
            .filter(|k| ["status", "owner", "effort_days"].contains(k))
            .collect();
        assert_eq!(keys, vec!["status", "owner", "effort_days"], "{assigned}");

        let reassigned = set_owner(&assigned, Some("grace")).expect("replace");
        assert!(
            reassigned.contains("owner: grace") && !reassigned.contains("ada"),
            "{reassigned}"
        );

        let cleared = set_owner(&reassigned, None).expect("clear");
        assert!(
            !cleared.contains("owner"),
            "the key is left behind, which the schema reads as null:\n{cleared}"
        );
        // And it still loads, which is the whole point of removing the line
        // rather than emptying it.
        let (fm, _) = split_frontmatter(&cleared).expect("frontmatter");
        let task: Task = yaml_compat::from_str(fm).expect("a cleared task still loads");
        assert_eq!(task.owner, None);
        assert_eq!(task.status, Status::Todo, "something else moved");
    }

    /// **A name that needs quoting survives, and one that cannot is refused**
    /// (`A1-359`).
    ///
    /// This plan already carries an `owner` holding an ANSI escape, from
    /// `A1-290`'s fixture, so a writer that interpolates the name raw would
    /// produce a file that does not load. The round-trip check is what catches
    /// it, and `yaml_scalar` is what makes it unnecessary.
    #[test]
    fn an_owner_that_needs_quoting_round_trips() {
        for name in [
            "ada",
            "ada lovelace",
            "a: b",
            "#ada",
            "- ada",
            "\u{1b}[41mada",
            "null",
            "true",
            "123",
            "*anchor",
            "ада",
            "ada\"s",
            "'ada'",
            "@ada",
            "  padded  ",
        ] {
            let file = "---\nid: A1-001\ntitle: t\nphase: 1\nstatus: todo\n---\n\nbody\n";
            let out =
                set_owner(file, Some(name)).unwrap_or_else(|e| panic!("`{name}` was refused: {e}"));
            let (fm, _) = split_frontmatter(&out).expect("frontmatter");
            let task: Task = yaml_compat::from_str(fm)
                .unwrap_or_else(|e| panic!("`{name}` produced unloadable frontmatter: {e}\n{out}"));
            assert_eq!(
                task.owner.as_deref(),
                Some(name),
                "`{name}` did not survive:\n{out}"
            );
        }
    }

    #[test]
    fn revisit_at_can_be_added_replaced_and_cleared_without_touching_the_body() {
        let file = "---\nid: A-1\ntitle: task\nphase: 1\nstatus: blocked\ntags: [one]\n---\n\nBody stays exact.\n";
        let added = set_revisit_at(file, Some("2030-01-02T03:04:05Z")).expect("add");
        assert!(
            added.contains("status: blocked\nrevisit_at: 2030-01-02T03:04:05Z\n"),
            "{added}"
        );
        assert!(added.ends_with("\nBody stays exact.\n"), "{added}");
        let replaced = set_revisit_at(&added, Some("2031-02-03T04:05:06Z")).expect("replace");
        assert!(replaced.contains("revisit_at: 2031-02-03T04:05:06Z"));
        assert!(!replaced.contains("2030-01-02"));
        let cleared = set_revisit_at(&replaced, None).expect("clear");
        assert!(!cleared.contains("revisit_at:"), "{cleared}");
        assert!(cleared.ends_with("\nBody stays exact.\n"), "{cleared}");

        let bare = file.replace("status: blocked\n", "status: blocked\nrevisit_at:\n");
        let filled = set_revisit_at(&bare, Some("2032-03-04T05:06:07Z"))
            .expect("a bare optional scalar is an explicit empty value");
        assert!(filled.contains("revisit_at: 2032-03-04T05:06:07Z"));

        let block = file.replace(
            "status: blocked\n",
            "status: blocked\nrevisit_at: >\n  2030-01-02T03:04:05Z\n",
        );
        assert!(matches!(
            set_revisit_at(&block, Some("2032-03-04T05:06:07Z")),
            Err(CloseError::NoScalar("revisit_at"))
        ));
    }

    /// Proves `portfolio.refuses_alternate_optional_scalar_keys`: a loaded
    /// optional value without an ordinary literal key line is present, not
    /// absent. Both assignment and clear refuse before claiming a source edit,
    /// for the shared revisit and durable-owner paths (`A1-721`).
    #[test]
    fn optional_scalar_edits_refuse_alternate_key_spellings_before_rewrite() {
        for quoted in ["'revisit_at'", "\"revisit_at\""] {
            let file = format!(
                "---\nid: A-1\ntitle: task\nphase: 1\nstatus: blocked\n{quoted}: \
                 2030-01-02T03:04:05Z\n---\n\nbody\n"
            );
            for to in [None, Some("2031-02-03T04:05:06Z")] {
                let error = set_revisit_at(&file, to)
                    .expect_err("an alternate revisit key is not an ordinary scalar line");
                assert!(matches!(error, CloseError::NoScalar("revisit_at")));
                assert!(error.to_string().contains("change it by hand"));
            }
        }

        for quoted in ["'owner'", "\"owner\""] {
            let file = format!(
                "---\nid: A-1\ntitle: task\nphase: 1\nstatus: todo\n{quoted}: ada\n---\n\nbody\n"
            );
            for to in [None, Some("grace")] {
                assert!(matches!(
                    set_owner(&file, to),
                    Err(CloseError::NoScalar("owner"))
                ));
            }
        }
    }

    #[test]
    fn optional_scalar_helpers_append_when_the_preferred_anchor_is_absent() {
        let frontmatter = "id: A-1\ntitle: task\nphase: 1\n";
        let revisit = set_optional_scalar_in(
            frontmatter,
            "revisit_at",
            Some("2030-01-02T03:04:05Z"),
            "status",
            false,
            "\n",
        )
        .expect("append optional scalar");
        assert!(revisit.ends_with("revisit_at: 2030-01-02T03:04:05Z\n"));

        let owner = set_owner_in(frontmatter, Some("ada"), false, "\n").expect("append owner");
        assert!(owner.ends_with("owner: ada\n"));
    }

    /// **An `owner` this cannot swap is refused by name, not appended beside**
    /// (`A1-359`, audit).
    ///
    /// The defect this pins: the writer fell back to appending whenever it had
    /// not rewritten a line, which is correct when the key is absent and wrong
    /// when it is present in block form -- it wrote a second `owner:` and the
    /// read-back reported a `duplicate field` error, which reads as a bug in
    /// this module rather than as a file it declines to guess at. `NoScalar`
    /// already says the right thing.
    #[test]
    fn an_owner_written_as_a_block_is_refused_by_name() {
        let block = "---\nid: A1-001\ntitle: t\nphase: 1\nstatus: todo\nowner: >\n  ada\n\
                     ---\n\nbody\n";
        for to in [Some("grace"), None] {
            let e = set_owner(block, to).expect_err("a block-form owner must be refused");
            assert!(
                matches!(e, CloseError::NoScalar("owner")),
                "refused as {e:?} rather than as a value it will not guess at"
            );
            let said = e.to_string();
            assert!(
                said.contains("change it by hand") && said.contains("written below the key"),
                "the refusal does not say what to do: {said}"
            );
        }
    }

    /// **A bare `owner:` can still be assigned** (`A1-359`).
    ///
    /// `owner:` with nothing after it loads as `null`, which is what the model
    /// means by unassigned, so assigning such a task must work rather than
    /// refuse: there is no value to guess at, only a line to replace.
    #[test]
    fn a_bare_owner_key_can_be_assigned_and_cleared() {
        let bare = "---\nid: A1-001\ntitle: t\nphase: 1\nstatus: todo\nowner:\n---\n\nbody\n";
        let assigned = set_owner(bare, Some("ada")).expect("assign over a null owner");
        let (fm, _) = split_frontmatter(&assigned).expect("frontmatter");
        let task: Task = yaml_compat::from_str(fm).expect("loads");
        assert_eq!(task.owner.as_deref(), Some("ada"), "{assigned}");
        assert_eq!(
            assigned.matches("owner").count(),
            1,
            "a second key:\n{assigned}"
        );

        let cleared = set_owner(bare, None).expect("clear a null owner");
        assert!(!cleared.contains("owner"), "{cleared}");
    }
}
