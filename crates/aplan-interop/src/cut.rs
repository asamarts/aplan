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
    /// No task in the plan carries that id.
    #[error("no task with id `{id}` in the plan")]
    NotFound {
        /// The id asked for.
        id: String,
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
        "{}: could not extend `blocked_by` (it is neither an inline `[a, b]` list nor a \
         `- item` block); add the ids by hand", .path.display()
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
    /// The file could not be read or written.
    #[error("{}: {source}", .path.display())]
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
pub fn check_target(plan: &Plan, milestone_id: &str) -> Result<(), CutError> {
    resolve(plan, milestone_id).map(|_| ())
}

/// Check that `id` names a cut this import is allowed to extend.
fn resolve<'p>(plan: &'p Plan, id: &str) -> Result<&'p aplan_core::load::LoadedTask, CutError> {
    let found = plan
        .tasks
        .iter()
        .find(|lt| lt.task.id == id)
        .ok_or_else(|| CutError::NotFound { id: id.to_string() })?;

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

    done.then(|| lines.concat())
}

/// Whether `line` is the top-level `blocked_by:` key.
fn is_blocked_by_key(line: &str) -> bool {
    let content = line.trim_end_matches(['\r', '\n']);
    content
        .strip_prefix("blocked_by")
        .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
}

/// Append `ids` to the cut named by `milestone_id`, writing the file in place.
///
/// Every guard runs before anything is written, so a refusal leaves the plan
/// exactly as it was.
pub fn place(plan: &Plan, milestone_id: &str, ids: &[String]) -> Result<Placed, CutError> {
    let target = resolve(plan, milestone_id)?;
    let path = target.path.clone();

    let added = missing_from(&target.task, ids);
    if added.is_empty() {
        return Ok(Placed { path, added });
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

    std::fs::write(&path, updated).map_err(|source| CutError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(Placed { path, added })
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

fn read(path: &Path) -> Result<String, CutError> {
    std::fs::read_to_string(path).map_err(|source| CutError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn a_milestone_without_a_blocked_by_line_is_refused() {
        let src = "---\nid: M\ntags: [milestone]\n---\n";
        assert!(extend_blocked_by(src, &ids(&["KEP-1"])).is_none());
    }
}
