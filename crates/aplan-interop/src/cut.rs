//! Place imported tasks into a release cut.
//!
//! ADR-0012 clause 5: writing new task files is import's whole job, and
//! `--cut <milestone>` is the *one* sanctioned mutation of existing state. It
//! appends the imported ids to that milestone's `blocked_by`, which is how a
//! cut records membership (ADR-0008: a cut is an ordinary task tagged
//! `milestone`, blocked by its members).
//!
//! Three guards run before anything is written, and the third is the one an
//! audit added:
//!
//! 1. the target exists,
//! 2. it is tagged `milestone`, so import cannot quietly turn a unit of work
//!    into a checkpoint, and
//! 3. it is **not already `done` or `dropped`**. Appending an unfinished task
//!    to a settled cut would make that milestone depend on an unfinished one
//!    (`APLAN007`, an error), so import refuses rather than invalidate a claim
//!    somebody already closed.
//!
//! The edit is surgical, in the style of `aplan close`: one line is rewritten
//! and the rest of the file, including its line endings, comments, and body,
//! is left byte-for-byte alone. Re-serialising the whole task would reformat a
//! hand-authored file, which is a diff nobody asked for.

use std::path::{Path, PathBuf};

use aplan_core::load::split_frontmatter;
use aplan_core::{Plan, Status, Task};

/// Why imported tasks could not be placed in a cut.
#[derive(Debug, thiserror::Error)]
pub enum CutError {
    /// Nothing in the plan answers to that name, by id or by label.
    ///
    /// The message names the cuts the plan *has*, the way `graph --cut` already
    /// answers, because a reader who typed the wrong name needs the right one
    /// rather than a restatement of the wrong one (`A1-139`).
    #[error("no cut `{name}` in this plan. {known}")]
    NoSuchCut {
        /// The name asked for, id or label.
        name: String,
        /// The cuts this plan declares, or a sentence saying it declares none.
        known: String,
    },
    /// A label two cuts answer to, so the name does not identify one.
    #[error(
        "`{label}` is the label of {} cuts ({}), so it does not say which: name one by its id",
        .ids.len(), .ids.join(", ")
    )]
    AmbiguousLabel {
        /// The label asked for.
        label: String,
        /// The milestone ids carrying it.
        ids: Vec<String>,
    },
    /// A name that is one cut's id and another cut's label.
    ///
    /// Taking the id silently is what the first version did, and it sends a
    /// reader who typed what `roadmap` printed into a different cut -- the
    /// mistake this feature exists to prevent, one layer down (`A1-139`, second
    /// audit).
    #[error(
        "`{name}` is the id of one cut and the label of {} other(s) ({}); name the one you mean \
         by its id",
        .labelled.len(), .labelled.join(", ")
    )]
    ShadowedName {
        /// The name asked for.
        name: String,
        /// The milestone ids that *report* themselves under it.
        labelled: Vec<String>,
    },
    /// The target is a unit of work, not a cut.
    #[error("`{id}` is not tagged `milestone`, so it is not a cut")]
    NotAMilestone {
        /// The id asked for.
        id: String,
    },
    /// The target is already closed or abandoned.
    #[error(
        "`{id}` is `{status}`; adding an unfinished task to a settled cut would break it \
         (APLAN007). Reopen it, or choose another cut."
    )]
    Settled {
        /// The id asked for.
        id: String,
        /// What it is now.
        status: String,
    },
    /// The file on disk is not a task file any more.
    #[error("{}: no `---` frontmatter", .path.display())]
    NotATask {
        /// The file read.
        path: PathBuf,
    },
    /// The `blocked_by` line could not be edited safely.
    #[error(
        "{}: could not edit `blocked_by` (it is neither an inline `[a, b]` list nor a \
         `- item` block); change it by hand", .path.display()
    )]
    Unsupported {
        /// The file read.
        path: PathBuf,
    },
    /// The rewrite did not survive being read back, so nothing was written.
    #[error("{}: the edit did not read back cleanly ({message}); nothing was written", .path.display())]
    SelfCheck {
        /// The file that would have been edited.
        path: PathBuf,
        /// What the read-back found.
        message: String,
    },
    /// Somebody else wrote the file between reading it and saving the edit, so
    /// saving would have discarded their change (`A1-226`).
    ///
    /// Retryable rather than fatal: the plan is intact and the edit was not
    /// applied, so running the command again recomputes it against what is now
    /// there. Said this way because the caller is often a script.
    #[error(
        "`{}` changed while this command was preparing its edit, so writing would have \
         discarded the other change. Nothing was written; run the command again.",
        .path.display()
    )]
    Changed {
        /// The file that moved under the command.
        path: PathBuf,
    },
    /// Another `aplan` held the file for the whole wait, so this edit was not
    /// applied (`A1-226`).
    ///
    /// Distinct from [`CutError::Changed`] on purpose: nothing changed under
    /// this command, it never got its turn. Both are retryable and a reader
    /// deciding what to do next is helped by knowing which happened.
    #[error(
        "`{}` was locked by another `aplan` writing at the same time, and did not become \
         free. Nothing was written; run the command again.",
        .path.display()
    )]
    Busy {
        /// The file that stayed locked.
        path: PathBuf,
    },
    /// The file could not be read or written.
    // Display names the file only: the field is called `source`, so `thiserror`
    // makes it the error's source and `anyhow` prints it after this. Naming it
    // here as well printed "Permission denied (os error 13)" twice (`A1-127`).
    #[error("{}", .path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
}

/// What placing the ids did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// The milestone file that was edited.
    pub path: PathBuf,
    /// Ids appended, in order. Ids already present are not repeated.
    pub added: Vec<String>,
}

/// Check that `id` names a cut this import may extend, without touching it.
///
/// Exposed so a caller can validate the destination *before* it writes anything
/// else. Import writes task files and then places them, so without this the
/// guards would run too late: a refusal would leave newly written tasks stranded
/// outside any cut, and an error message implying nothing had happened.
pub fn check_target(plan: &Plan, cut: &str) -> Result<(), CutError> {
    resolve(plan, cut).map(|_| ())
}

/// The milestone id a `--cut` argument names (`A1-139`, ADR-0013's vocabulary).
///
/// **Id first, then label**, which is the order `graph --cut` already used and
/// the reason this is one function rather than a rule per command: `roadmap`
/// prints the label, `tasks --format json` carries both, and a reader who copied
/// either should not have to know which command wants which. An id is
/// unambiguous by `APLAN002`, so resolving it first means a label can never
/// shadow a real cut.
///
/// The label is the cut's own vocabulary: its `vN.N` tag if it has one, else the
/// first tag that is not the `milestone` marker -- the same derivation `roadmap`
/// renders (`A1-037`).
///
/// # Errors
///
/// [`CutError::NoSuchCut`] when nothing answers to the name, naming the cuts the
/// plan has, and [`CutError::AmbiguousLabel`] when a label belongs to more than
/// one cut.
pub fn resolve_id(plan: &Plan, cut: &str) -> Result<String, CutError> {
    let is_id = plan.tasks.iter().any(|lt| lt.task.id == cut);
    // A cut carrying only the `milestone` marker has *no* label, and an empty
    // name must not match it: `--cut ""` placed a task in the first unlabelled
    // cut it found, silently (`A1-139`, found by its audit). An empty name names
    // nothing.
    let by_label: Vec<String> = if cut.is_empty() {
        Vec::new()
    } else {
        plan.tasks
            .iter()
            .filter(|lt| lt.task.is_milestone() && label_of(&lt.task) == cut)
            .map(|lt| lt.task.id.clone())
            .collect()
    };
    if is_id {
        // One name, two cuts: a milestone whose *id* is `s2` beside one that
        // *reports itself* as `s2`. Taking the id silently sends a reader who
        // typed what `roadmap` printed into the other cut, which is the mistake
        // this whole feature exists to prevent, one layer down (`A1-139`, second
        // audit). Refused with both, exactly as a shared label is.
        let shadowed: Vec<String> = by_label.iter().filter(|id| *id != cut).cloned().collect();
        if !shadowed.is_empty() {
            return Err(CutError::ShadowedName {
                name: cut.to_string(),
                labelled: shadowed,
            });
        }
        return Ok(cut.to_string());
    }
    match by_label.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(CutError::NoSuchCut {
            name: cut.to_string(),
            known: known_cuts(plan),
        }),
        many => Err(CutError::AmbiguousLabel {
            label: cut.to_string(),
            ids: many.to_vec(),
        }),
    }
}

/// The label a cut reports itself under: `roadmap`'s own derivation, called
/// rather than repeated (`A1-139`, found by its audit -- the first version
/// copied the rule, hardcoded the `milestone` tag, and would have drifted the
/// day either changed).
fn label_of(t: &Task) -> &str {
    aplan_core::cut_label(t)
}

/// The cuts a plan declares, for a refusal that answers with them.
fn known_cuts(plan: &Plan) -> String {
    let cuts: Vec<String> = plan
        .tasks
        .iter()
        .filter(|lt| lt.task.is_milestone())
        .map(|lt| {
            let label = label_of(&lt.task);
            if label.is_empty() {
                lt.task.id.clone()
            } else {
                format!("{} (`{label}`)", lt.task.id)
            }
        })
        .collect();
    if cuts.is_empty() {
        "This plan declares no cuts.".to_string()
    } else {
        format!("It has: {}", cuts.join(", "))
    }
}

/// Check that `cut` names a cut this import is allowed to extend.
fn resolve<'p>(plan: &'p Plan, cut: &str) -> Result<&'p aplan_core::load::LoadedTask, CutError> {
    let id = resolve_id(plan, cut)?;
    let id = id.as_str();
    let found = plan
        .tasks
        .iter()
        .find(|lt| lt.task.id == id)
        .ok_or_else(|| CutError::NoSuchCut {
            name: id.to_string(),
            known: known_cuts(plan),
        })?;

    if !found.task.is_milestone() {
        return Err(CutError::NotAMilestone { id: id.to_string() });
    }
    // Guard 3: never extend a settled cut (see the module docs).
    if matches!(found.task.status, Status::Done | Status::Dropped) {
        return Err(CutError::Settled {
            id: id.to_string(),
            status: status_word(found.task.status).to_string(),
        });
    }
    Ok(found)
}

/// The `status:` value as it is written in a task file.
fn status_word(s: Status) -> &'static str {
    match s {
        Status::Todo => "todo",
        Status::InProgress => "in_progress",
        Status::Blocked => "blocked",
        Status::Done => "done",
        Status::Dropped => "dropped",
    }
}

/// Ids from `ids` that `task` does not already list, in the given order.
fn missing_from(task: &Task, ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter(|id| !task.blocked_by.iter().any(|b| b == *id))
        .cloned()
        .collect()
}

/// Rewrite the frontmatter of `file` so `blocked_by` also lists `ids`.
///
/// Returns `None` when the `blocked_by` line is in a shape this cannot extend
/// without risking a mangled file; the caller turns that into
/// [`CutError::Unsupported`] rather than guessing.
fn extend_blocked_by(file: &str, ids: &[String]) -> Option<String> {
    let (frontmatter, _) = split_frontmatter(file)?;
    // Operate on the frontmatter region only: a `blocked_by:` in the prose body
    // is documentation, not data.
    //
    // The offset is taken from the borrowed slice itself rather than by
    // searching for its text, which would find the wrong place for an empty or
    // repeated frontmatter.
    let fm_start = frontmatter.as_ptr() as usize - file.as_ptr() as usize;
    let fm_end = fm_start + frontmatter.len();

    let mut out = String::with_capacity(file.len() + ids.len() * 16);
    out.push_str(&file[..fm_start]);
    out.push_str(&rewrite_region(frontmatter, ids)?);
    out.push_str(&file[fm_end..]);
    Some(out)
}

/// Append `ids` to the `blocked_by` entry inside one frontmatter region.
fn rewrite_region(frontmatter: &str, ids: &[String]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut done = false;

    let mut iter = frontmatter.split_inclusive('\n').peekable();
    while let Some(line) = iter.next() {
        if done || !is_blocked_by_key(line) {
            lines.push(line.to_string());
            continue;
        }

        let eol_len = line.len() - line.trim_end_matches(['\r', '\n']).len();
        let content = &line[..line.len() - eol_len];
        let eol = &line[content.len()..];

        // Preserve the spacing the author chose after the colon: this is a
        // surgical edit, and re-indenting someone's frontmatter is a diff they
        // did not ask for.
        let (key, after_colon) = content.split_once(':')?;
        let ws_len = after_colon.len() - after_colon.trim_start_matches([' ', '\t']).len();
        let (lead_ws, value) = after_colon.split_at(ws_len);

        if value.starts_with('[') {
            // Inline flow: `blocked_by: [A, B]`, the shape aplan writes. Split
            // at the closing bracket rather than requiring it to end the line,
            // so a trailing comment (`[A] # the cut`) is kept rather than
            // refused.
            let close = value.find(']')?;
            let inner = value[1..close].trim();
            let trailing = &value[close + 1..];
            let mut items: Vec<String> = if inner.is_empty() {
                Vec::new()
            } else {
                inner.split(',').map(|s| s.trim().to_string()).collect()
            };
            items.extend(ids.iter().cloned());
            lines.push(format!(
                "{key}:{lead_ws}[{}]{trailing}{eol}",
                items.join(", ")
            ));
            done = true;
        } else if value.trim().is_empty() {
            // Block form: the `- item` lines follow. Keep the key, then copy the
            // block through and append after its last item.
            lines.push(line.to_string());
            let mut indent = String::from("  ");
            while let Some(next) = iter.peek() {
                let trimmed = next.trim_start();
                if !trimmed.starts_with("- ") {
                    break;
                }
                indent = next[..next.len() - trimmed.len()].to_string();
                lines.push(iter.next()?.to_string());
            }
            for id in ids {
                lines.push(format!("{indent}- {id}{eol}"));
            }
            done = true;
        } else {
            // Something else entirely (an anchor, a folded scalar). Refuse.
            return None;
        }
    }

    if !done {
        // No `blocked_by` at all, which is what a milestone written by
        // `aplan add` looks like -- and a cut this tool created could not then
        // be filled by this tool, so the only way to put ordinary work into a
        // new cut was to edit YAML by hand, which is what `mv` exists to stop
        // (`A1-170`).
        //
        // Created here rather than by `add` writing an empty list: an absent
        // key and `blocked_by: []` are indistinguishable to everything that
        // reads a plan -- same `APLAN021`, same `show` -- so a convention in
        // what `add` writes would fix nothing for the milestone somebody else
        // wrote, or an importer did.
        let eol = if frontmatter.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        if lines.last().is_some_and(|l: &String| !l.ends_with('\n')) {
            lines.push(eol.to_string());
        }
        lines.push(format!("blocked_by: [{}]{eol}", ids.join(", ")));
        done = true;
    }
    done.then(|| lines.concat())
}

/// Remove `ids` from one frontmatter region's `blocked_by`, mirroring
/// [`rewrite_region`] so both forms and the author's spacing survive.
fn rewrite_region_removing(frontmatter: &str, ids: &[String]) -> Option<String> {
    let drop_it = |candidate: &str| ids.iter().any(|id| id == candidate.trim());
    let mut lines: Vec<String> = Vec::new();
    let mut done = false;

    let mut iter = frontmatter.split_inclusive('\n').peekable();
    while let Some(line) = iter.next() {
        if done || !is_blocked_by_key(line) {
            lines.push(line.to_string());
            continue;
        }

        let eol_len = line.len() - line.trim_end_matches(['\r', '\n']).len();
        let content = &line[..line.len() - eol_len];
        let eol = &line[content.len()..];
        let (key, after_colon) = content.split_once(':')?;
        let ws_len = after_colon.len() - after_colon.trim_start_matches([' ', '\t']).len();
        let (lead_ws, value) = after_colon.split_at(ws_len);

        if value.starts_with('[') {
            let close = value.find(']')?;
            let inner = value[1..close].trim();
            let trailing = &value[close + 1..];
            let kept: Vec<String> = if inner.is_empty() {
                Vec::new()
            } else {
                inner
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !drop_it(item))
                    .map(ToString::to_string)
                    .collect()
            };
            lines.push(format!(
                "{key}:{lead_ws}[{}]{trailing}{eol}",
                kept.join(", ")
            ));
            done = true;
        } else if value.trim().is_empty() {
            lines.push(line.to_string());
            while let Some(next) = iter.peek() {
                let trimmed = next.trim_start();
                if !trimmed.starts_with("- ") {
                    break;
                }
                let item = trimmed
                    .trim_start_matches("- ")
                    .trim_end_matches(['\r', '\n']);
                let keep = !drop_it(item);
                let taken = iter.next()?;
                if keep {
                    lines.push(taken.to_string());
                }
            }
            done = true;
        } else {
            return None;
        }
    }

    done.then(|| lines.concat())
}

/// Whether `line` is the top-level `blocked_by:` key.
fn is_blocked_by_key(line: &str) -> bool {
    let content = line.trim_end_matches(['\r', '\n']);
    content
        .strip_prefix("blocked_by")
        .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
}

/// A cut membership removal, for [`remove`].
#[derive(Debug, Clone)]
pub struct Removed {
    /// The milestone file that was edited.
    pub path: PathBuf,
    /// Ids removed, in order.
    pub removed: Vec<String>,
}

/// Remove `ids` from the cut named by `milestone_id`, writing the file in place.
///
/// The inverse of [`place`], and held to the same standard: every guard runs
/// before anything is written, and the rewritten file is parsed back before it
/// is saved (`A1-077`).
///
/// # Errors
/// As [`place`], plus [`CutError::Unsupported`] when the `blocked_by` is in a
/// shape this cannot edit surgically.
pub fn remove(plan: &Plan, milestone_id: &str, ids: &[String]) -> Result<Removed, CutError> {
    plan_removal(plan, milestone_id, ids)?.write()
}

/// A removal worked out in full but not yet written.
///
/// `mv` needs this: it edits two milestones, and writing the destination before
/// discovering the source cannot be edited leaves the task in both cuts and the
/// plan failing `APLAN022`. A partial write is worse than a refusal (`A1-030`),
/// so every rewrite is computed before any is saved (`A1-077`).
#[derive(Debug, Clone)]
pub struct PendingRemoval {
    path: PathBuf,
    removed: Vec<String>,
    /// `None` when there is nothing to do, so writing is a no-op.
    text: Option<String>,
    /// The bytes `text` was computed from, so the save can refuse if the file
    /// has moved since (`A1-226`). `None` alongside a `None` `text`.
    expected: Option<String>,
}

impl PendingRemoval {
    /// The file this would write, so a caller can prove it is writable before
    /// writing anything at all (`A1-127`).
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The ids it would take out of the cut, so a caller whose write fails can
    /// say what state the plan is in (`A1-127`).
    #[must_use]
    pub fn removed(&self) -> &[String] {
        &self.removed
    }

    /// Whether it would write at all. A removal of nothing is a no-op, and
    /// probing a file this will not touch would refuse a move for a permission
    /// it never needed.
    #[must_use]
    pub fn writes(&self) -> bool {
        self.text.is_some()
    }

    /// What to hand [`aplan_core::save::replace_all`], or `None` for a no-op
    /// (`A1-234`).
    #[must_use]
    pub fn replacement(&self) -> Option<aplan_core::save::Replacement> {
        Some(aplan_core::save::Replacement {
            path: self.path.clone(),
            expected: self.expected.clone()?,
            updated: self.text.clone()?,
        })
    }

    /// What it took out, once the batch carrying it has been written.
    #[must_use]
    pub fn into_removed(self) -> Removed {
        Removed {
            path: self.path,
            removed: self.removed,
        }
    }

    /// Save it.
    ///
    /// # Errors
    /// [`CutError::Changed`] when somebody else wrote the file first, and
    /// [`CutError::Io`] when it cannot be written.
    pub fn write(self) -> Result<Removed, CutError> {
        if let Some(text) = self.text {
            let expected = self.expected.as_deref().unwrap_or_default();
            save(&self.path, expected, &text)?;
        }
        Ok(Removed {
            path: self.path,
            removed: self.removed,
        })
    }
}

/// Work out a removal without writing it.
///
/// # Errors
/// As [`remove`].
pub fn plan_removal(
    plan: &Plan,
    milestone_id: &str,
    ids: &[String],
) -> Result<PendingRemoval, CutError> {
    let target = resolve(plan, milestone_id)?;
    let path = target.path.clone();

    let removed: Vec<String> = target
        .task
        .blocked_by
        .iter()
        .filter(|b| ids.iter().any(|id| id == *b))
        .cloned()
        .collect();
    if removed.is_empty() {
        return Ok(PendingRemoval {
            path,
            removed,
            text: None,
            expected: None,
        });
    }

    let file = read(&path)?;
    if split_frontmatter(&file).is_none() {
        return Err(CutError::NotATask { path });
    }
    let (frontmatter, _) =
        split_frontmatter(&file).ok_or(CutError::NotATask { path: path.clone() })?;
    let fm_start = frontmatter.as_ptr() as usize - file.as_ptr() as usize;
    let fm_end = fm_start + frontmatter.len();
    let rewritten = rewrite_region_removing(frontmatter, &removed)
        .ok_or_else(|| CutError::Unsupported { path: path.clone() })?;
    let updated = format!("{}{}{}", &file[..fm_start], rewritten, &file[fm_end..]);

    // Read back before writing, as `place` does: a rewrite that cannot be parsed
    // again is a corrupted file, and refusing beats saving it.
    let parsed: crate::cut::TaskProbe = serde_yaml_ng::from_str(
        split_frontmatter(&updated)
            .ok_or_else(|| CutError::Unsupported { path: path.clone() })?
            .0,
    )
    .map_err(|_| CutError::Unsupported { path: path.clone() })?;
    if removed.iter().any(|id| parsed.blocked_by.contains(id)) {
        return Err(CutError::Unsupported { path });
    }

    Ok(PendingRemoval {
        path,
        removed,
        text: Some(updated),
        expected: Some(file),
    })
}

/// The slice of a task the membership edits read back.
#[derive(serde::Deserialize)]
pub(crate) struct TaskProbe {
    #[serde(default)]
    pub(crate) blocked_by: Vec<String>,
}

/// Append `ids` to the cut named by `milestone_id`, writing the file in place.
///
/// Every guard runs before anything is written, so a refusal leaves the plan
/// exactly as it was.
pub fn place(plan: &Plan, milestone_id: &str, ids: &[String]) -> Result<Placed, CutError> {
    let pending = plan_placement(plan, milestone_id, ids)?;
    if let Some(replacement) = pending.replacement() {
        save(
            &replacement.path,
            &replacement.expected,
            &replacement.updated,
        )?;
    }
    Ok(Placed {
        path: pending.path,
        added: pending.added,
    })
}

/// A placement worked out but not written, so a caller can write it beside
/// something else as one change (`A1-234`).
#[derive(Debug)]
pub struct PendingPlacement {
    /// The milestone file it would edit.
    pub path: PathBuf,
    /// Ids it would append, in order. Ids already present are not repeated.
    pub added: Vec<String>,
    /// `None` when every id is already listed, so writing is a no-op.
    text: Option<String>,
    /// The bytes `text` was computed from (`A1-226`).
    expected: Option<String>,
}

impl PendingPlacement {
    /// What to hand [`aplan_core::save::replace_all`], or `None` for a no-op.
    #[must_use]
    pub fn replacement(&self) -> Option<aplan_core::save::Replacement> {
        Some(aplan_core::save::Replacement {
            path: self.path.clone(),
            expected: self.expected.clone()?,
            updated: self.text.clone()?,
        })
    }
}

/// Work out a placement without writing it.
///
/// # Errors
/// As [`place`].
pub fn plan_placement(
    plan: &Plan,
    milestone_id: &str,
    ids: &[String],
) -> Result<PendingPlacement, CutError> {
    let target = resolve(plan, milestone_id)?;
    let path = target.path.clone();

    let added = missing_from(&target.task, ids);
    if added.is_empty() {
        return Ok(PendingPlacement {
            path,
            added,
            text: None,
            expected: None,
        });
    }

    let file = read(&path)?;
    if split_frontmatter(&file).is_none() {
        return Err(CutError::NotATask { path });
    }
    let updated = extend_blocked_by(&file, &added)
        .ok_or_else(|| CutError::Unsupported { path: path.clone() })?;

    // Read back what we are about to write, before writing it. `aplan close`
    // holds itself to the same standard: a surgical text edit that cannot be
    // parsed again is a corrupted file, and refusing beats saving it. This is
    // the difference between "the rewrite looked right" and "the rewrite is
    // still a task aplan can read, which really does list the new members".
    self_check(&updated, &added, &path)?;

    Ok(PendingPlacement {
        path,
        added,
        text: Some(updated),
        expected: Some(file),
    })
}

/// Parse the rewritten file and confirm it is still a task that lists `added`.
///
/// Nothing is written unless this passes, so a bug in the rewrite surfaces as a
/// refusal rather than as a mangled milestone.
fn self_check(updated: &str, added: &[String], path: &Path) -> Result<(), CutError> {
    let fail = |message: String| CutError::SelfCheck {
        path: path.to_path_buf(),
        message,
    };

    let (frontmatter, _) =
        split_frontmatter(updated).ok_or_else(|| fail("the result has no frontmatter".into()))?;
    let parsed: Task = serde_yaml_ng::from_str(frontmatter).map_err(|e| fail(e.to_string()))?;

    if let Some(missing) = added.iter().find(|id| !parsed.blocked_by.contains(id)) {
        return Err(fail(format!(
            "`{missing}` is missing from the rewritten `blocked_by`"
        )));
    }
    Ok(())
}

/// Save a rewritten milestone through the one writer that refuses to clobber.
///
/// Every command that edits cut membership -- `add --cut`, `mv`, `drop` --
/// reaches the filesystem here, so none of them can grow its own unsafe write
/// (`A1-226`).
fn save(path: &Path, expected: &str, updated: &str) -> Result<(), CutError> {
    aplan_core::save::replace(path, expected, updated).map_err(|e| match e {
        aplan_core::save::SaveError::Changed { path } => CutError::Changed { path },
        aplan_core::save::SaveError::Busy { path } => CutError::Busy { path },
        aplan_core::save::SaveError::Io { path, source } => CutError::Io { path, source },
        // One file, one replacement of a milestone that already exists: a batch
        // that contradicts itself, and a creation that collided, cannot arise
        // here. Reported as I/O rather than given variants no caller can reach.
        e @ (aplan_core::save::SaveError::Conflict { .. }
        | aplan_core::save::SaveError::Exists { .. }) => CutError::Io {
            path: std::path::PathBuf::new(),
            source: std::io::Error::other(e.to_string()),
        },
    })
}

fn read(path: &Path) -> Result<String, CutError> {
    std::fs::read_to_string(path).map_err(|source| CutError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {

    /// The io error names the file, and the reason exactly once (`A1-127`).
    ///
    /// The field is called `source`, so `thiserror` makes it the error's source
    /// and every caller that prints the chain -- which is what `aplan` does --
    /// prints it after the message. Naming it in the message too produced
    /// "aplan/tasks/M-001.md: Permission denied (os error 13): Permission denied
    /// (os error 13)", which reads as two failures.
    #[test]
    fn an_io_failure_names_the_file_and_the_reason_once() {
        use std::error::Error as _;
        let err = super::CutError::Io {
            path: std::path::PathBuf::from("aplan/tasks/M-s0.md"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        let shown = err.to_string();
        assert_eq!(shown, "aplan/tasks/M-s0.md");
        // The reason is still reachable, it is just not said twice.
        let reason = err
            .source()
            .expect("the io error is the source")
            .to_string();
        assert!(
            reason.to_lowercase().contains("permission denied"),
            "{reason}"
        );
        assert!(!shown.contains("ermission"), "{shown}");
    }

    /// A plan on disk, so `remove` is exercised through the file it rewrites
    /// rather than only through its text surgery.
    fn plan_dir(milestone: &str) -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let tasks = dir.path().join("tasks");
        std::fs::create_dir_all(&tasks).expect("dirs");
        std::fs::write(
            tasks.join("A-01.md"),
            "---\nid: A-01\ntitle: a\nphase: 0\nstatus: todo\n---\n",
        )
        .expect("write");
        std::fs::write(tasks.join("M-s0.md"), milestone).expect("write");
        dir
    }

    #[test]
    fn remove_rewrites_the_milestone_on_disk() {
        let dir = plan_dir(
            "---\nid: M-s0\ntitle: Cut\nphase: 0\nstatus: todo\n\
             tags: [milestone, s0]\nblocked_by: [A-01]\n---\n",
        );
        let plan = aplan_core::load_plan(dir.path());
        let out = remove(&plan, "M-s0", &["A-01".to_string()]).expect("removed");
        assert_eq!(out.removed, vec!["A-01".to_string()]);
        let text = std::fs::read_to_string(out.path).expect("read");
        assert!(text.contains("blocked_by: []"), "{text}");
    }

    #[test]
    fn remove_is_a_no_op_when_the_task_is_not_a_member() {
        // No write at all, so a `mv` from a cut the task was never in cannot
        // rewrite a file for no reason.
        let dir = plan_dir(
            "---\nid: M-s0\ntitle: Cut\nphase: 0\nstatus: todo\n\
             tags: [milestone, s0]\nblocked_by: [A-01]\n---\n",
        );
        let plan = aplan_core::load_plan(dir.path());
        let before = std::fs::read_to_string(dir.path().join("tasks/M-s0.md")).unwrap();
        let out = remove(&plan, "M-s0", &["B-99".to_string()]).expect("no-op");
        assert!(out.removed.is_empty());
        let after = std::fs::read_to_string(dir.path().join("tasks/M-s0.md")).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn remove_refuses_a_shape_it_cannot_edit_and_writes_nothing() {
        // An anchor is refused rather than guessed at, and the file is left as
        // it was: a refusal must change nothing.
        let dir = plan_dir(
            "---\nid: M-s0\ntitle: Cut\nphase: 0\nstatus: todo\n\
             tags: [milestone, s0]\nblocked_by: &anchor [A-01]\n---\n",
        );
        let plan = aplan_core::load_plan(dir.path());
        let before = std::fs::read_to_string(dir.path().join("tasks/M-s0.md")).unwrap();
        let result = remove(&plan, "M-s0", &["A-01".to_string()]);
        assert!(result.is_err(), "an anchor must be refused");
        let after = std::fs::read_to_string(dir.path().join("tasks/M-s0.md")).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn remove_reports_a_milestone_that_does_not_exist() {
        let dir = plan_dir(
            "---\nid: M-s0\ntitle: Cut\nphase: 0\nstatus: todo\n\
             tags: [milestone, s0]\nblocked_by: [A-01]\n---\n",
        );
        let plan = aplan_core::load_plan(dir.path());
        assert!(matches!(
            remove(&plan, "M-nope", &["A-01".to_string()]),
            Err(CutError::NoSuchCut { .. })
        ));
    }

    #[test]
    fn removing_handles_the_block_sequence_form() {
        // `place` supports both shapes, so its inverse has to: a plan whose
        // milestones use `- item` lines is a plan this must not corrupt.
        let fm = "id: M\nblocked_by:\n  - A\n  - B\n  - C\n";
        let out = rewrite_region_removing(fm, &["B".to_string()]).expect("rewritten");
        assert_eq!(out, "id: M\nblocked_by:\n  - A\n  - C\n");
    }

    #[test]
    fn removing_keeps_the_authors_spacing_and_comment() {
        let fm = "id: M\nblocked_by:   [A, B]  # the cut\n";
        let out = rewrite_region_removing(fm, &["A".to_string()]).expect("rewritten");
        assert_eq!(out, "id: M\nblocked_by:   [B]  # the cut\n");
    }

    #[test]
    fn removing_the_only_member_leaves_an_empty_list() {
        // Not an error here: `APLAN021` reports an empty cut, and this edit
        // should not decide that question on the author's behalf.
        let fm = "id: M\nblocked_by: [A]\n";
        let out = rewrite_region_removing(fm, &["A".to_string()]).expect("rewritten");
        assert_eq!(out, "id: M\nblocked_by: []\n");
    }

    #[test]
    fn removing_refuses_a_shape_it_cannot_edit() {
        // An anchor or folded scalar is refused rather than guessed at, exactly
        // as appending refuses it.
        let fm = "id: M\nblocked_by: *anchor\n";
        assert!(rewrite_region_removing(fm, &["A".to_string()]).is_none());
    }
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    const INLINE: &str = "---\nid: M-v0.6\ntitle: Surfaces\nphase: 1\nstatus: todo\ntags: [milestone, v0.6]\nblocked_by: [A1-003, A1-019]\n---\n\n## Exit criteria\n\n- [ ] something\n";

    #[test]
    fn an_inline_list_gains_the_ids_and_keeps_its_style() {
        let out = extend_blocked_by(INLINE, &ids(&["KEP-1", "KEP-2"])).unwrap();
        assert!(
            out.contains("blocked_by: [A1-003, A1-019, KEP-1, KEP-2]\n"),
            "{out}"
        );
        // Everything else is untouched, including the body.
        assert!(out.contains("## Exit criteria"));
        assert!(out.contains("tags: [milestone, v0.6]"));
        assert_eq!(out.lines().count(), INLINE.lines().count());
    }

    /// A cut this tool created can be filled by this tool (`A1-170`).
    ///
    /// `aplan add --tag milestone` writes no `blocked_by` at all, and an absent
    /// key is not a list to append to, so the only way to put ordinary work into
    /// a new cut was to edit YAML by hand -- which is the thing `mv` exists to
    /// stop. The key is created rather than the edit refused.
    #[test]
    fn a_milestone_with_no_blocked_by_gains_one() {
        let src = "---\nid: M\ntitle: A new cut\ntags:\n- milestone\n---\n\n## Why\n";
        let out = extend_blocked_by(src, &ids(&["W-001"])).unwrap();
        assert!(out.contains("blocked_by: [W-001]\n"), "{out}");
        // Inside the frontmatter, and after what was already there rather than
        // reordering it: this is a surgical edit like every other one here.
        let (frontmatter, body) = aplan_core::load::split_frontmatter(&out).expect("frontmatter");
        assert!(frontmatter.contains("blocked_by: [W-001]"), "{frontmatter}");
        assert!(
            frontmatter.starts_with("id: M\ntitle: A new cut\n"),
            "{frontmatter}"
        );
        assert_eq!(body, "\n## Why\n", "the body is untouched");
    }

    /// The shapes a frontmatter can end with, since the created key is appended
    /// after whatever was there (`A1-170`, audit).
    ///
    /// Each one is parsed back, because "it looks like YAML" is not the claim:
    /// a key written at column zero after a nested block, a block scalar or a
    /// comment either ends that construct correctly or corrupts the file.
    #[test]
    fn a_created_blocked_by_survives_whatever_the_frontmatter_ended_with() {
        for (what, frontmatter) in [
            (
                "a nested mapping",
                "id: M\ntrace_fingerprints:\n  README.md#x: abc123\n",
            ),
            ("a block sequence", "id: M\ntags:\n  - milestone\n"),
            ("a folded scalar", "id: M\ntitle: >\n  a long\n  title\n"),
            ("a literal scalar", "id: M\ntitle: |\n  a long\n  title\n"),
            (
                "a comment",
                "id: M\ntags: [milestone]\n# why this cut exists\n",
            ),
            ("an inline mapping", "id: M\nmeta: {a: 1}\n"),
        ] {
            let src = format!("---\n{frontmatter}---\n\n## Why\n");
            let out = extend_blocked_by(&src, &ids(&["W-001"])).unwrap_or_else(|| {
                panic!("{what}: refused");
            });
            let (fm, _) = aplan_core::load::split_frontmatter(&out).expect("frontmatter");
            let parsed: super::TaskProbe =
                serde_yaml_ng::from_str(fm).unwrap_or_else(|e| panic!("{what}: {e}\n{out}"));
            assert_eq!(
                parsed.blocked_by,
                vec!["W-001".to_string()],
                "{what}: {out}"
            );
        }
    }

    /// A `blocked_by` nested under another key is not this key, and editing it
    /// would put the membership somewhere nothing reads (`A1-170`, audit).
    #[test]
    fn an_indented_blocked_by_is_not_the_one_that_is_extended() {
        let src = "---\nid: M\nmeta:\n  blocked_by: [X]\ntags: [milestone]\n---\n";
        let out = extend_blocked_by(src, &ids(&["W-001"])).expect("created");
        assert!(
            out.contains("  blocked_by: [X]\n"),
            "the nested one is untouched: {out}"
        );
        assert!(
            out.contains("\nblocked_by: [W-001]\n"),
            "a top-level one is created: {out}"
        );
        let (fm, _) = aplan_core::load::split_frontmatter(&out).expect("frontmatter");
        let parsed: super::TaskProbe = serde_yaml_ng::from_str(fm).expect("parses");
        assert_eq!(parsed.blocked_by, vec!["W-001".to_string()], "{out}");
    }

    /// The same on a file written by an editor that uses CRLF.
    #[test]
    fn a_created_blocked_by_keeps_the_files_line_endings() {
        let src = "---\r\nid: M\r\ntags: [milestone]\r\n---\r\n";
        let out = extend_blocked_by(src, &ids(&["W-001"])).unwrap();
        assert!(out.contains("blocked_by: [W-001]\r\n"), "{out:?}");
        assert!(!out.contains("\n\n"), "no stray bare newline: {out:?}");
    }

    /// And a frontmatter whose last line has no newline is not run together
    /// with the key being added.
    #[test]
    fn a_created_blocked_by_starts_its_own_line() {
        let src = "---\nid: M\ntags: [milestone]---\n";
        // Not a task at all -- the `---` has to be its own line -- so this is
        // the shape that would splice, proved through the region editor.
        let out = super::rewrite_region("id: M\ntags: [milestone]", &ids(&["W-001"])).unwrap();
        assert!(
            out.contains("tags: [milestone]\nblocked_by: [W-001]\n"),
            "{out:?}"
        );
        let _ = src;
    }

    #[test]
    fn an_empty_inline_list_is_filled() {
        let src = "---\nid: M\nblocked_by: []\n---\n";
        let out = extend_blocked_by(src, &ids(&["KEP-1"])).unwrap();
        assert!(out.contains("blocked_by: [KEP-1]\n"), "{out}");
    }

    #[test]
    fn a_block_list_gains_items_at_its_own_indentation() {
        let src = "---\nid: M\nblocked_by:\n  - A1-003\n  - A1-019\ntags: [milestone]\n---\n";
        let out = extend_blocked_by(src, &ids(&["KEP-1"])).unwrap();
        assert!(out.contains("  - A1-019\n  - KEP-1\ntags:"), "{out}");
    }

    #[test]
    fn crlf_line_endings_survive() {
        let src = "---\r\nid: M\r\nblocked_by: [A]\r\n---\r\n";
        let out = extend_blocked_by(src, &ids(&["KEP-1"])).unwrap();
        assert!(out.contains("blocked_by: [A, KEP-1]\r\n"), "{out:?}");
        assert!(!out.contains("\n\n"), "no stray bare newline: {out:?}");
    }

    #[test]
    fn a_blocked_by_in_the_body_is_never_touched() {
        let src =
            "---\nid: M\nblocked_by: [A]\n---\n\nProse mentioning blocked_by: [X] in passing.\n";
        let out = extend_blocked_by(src, &ids(&["KEP-1"])).unwrap();
        assert!(out.contains("blocked_by: [A, KEP-1]\n"), "{out}");
        assert!(
            out.contains("blocked_by: [X] in passing."),
            "the body must be untouched: {out}"
        );
    }

    #[test]
    fn a_trailing_comment_is_kept_not_a_reason_to_refuse() {
        let src = "---\nid: M\nblocked_by: [A] # the cut\n---\n";
        let out = extend_blocked_by(src, &ids(&["KEP-1"])).unwrap();
        assert!(out.contains("blocked_by: [A, KEP-1] # the cut\n"), "{out}");
    }

    #[test]
    fn the_authors_spacing_after_the_colon_is_preserved() {
        // A surgical edit that silently re-indents someone's frontmatter is a
        // diff they did not ask for.
        let src = "---\nid: M\nblocked_by:   [A]\n---\n";
        let out = extend_blocked_by(src, &ids(&["KEP-1"])).unwrap();
        assert!(out.contains("blocked_by:   [A, KEP-1]\n"), "{out:?}");
    }

    #[test]
    fn a_multi_line_inline_list_is_refused_rather_than_half_edited() {
        let src = "---\nid: M\nblocked_by: [A,\n  B]\n---\n";
        assert!(extend_blocked_by(src, &ids(&["KEP-1"])).is_none());
    }

    #[test]
    fn a_shape_we_cannot_edit_is_refused_rather_than_mangled() {
        let src = "---\nid: M\nblocked_by: &anchor\n---\n";
        assert!(extend_blocked_by(src, &ids(&["KEP-1"])).is_none());
    }

    /// Superseded by `A1-170`: an absent key is now created, because refusing
    /// it meant a cut `aplan add` had written could not be filled by `aplan mv`.
    /// The two neighbours above are the shapes still refused, and they are the
    /// distinction that matters -- an absent key is *nothing to misread*, while
    /// an anchor or a list broken across lines is something this editor would
    /// have to guess at.
    #[test]
    fn a_milestone_without_a_blocked_by_line_gains_one_rather_than_being_refused() {
        let src = "---\nid: M\ntags: [milestone]\n---\n";
        let out = extend_blocked_by(src, &ids(&["KEP-1"])).expect("created");
        assert!(out.contains("blocked_by: [KEP-1]\n"), "{out}");
    }
}
