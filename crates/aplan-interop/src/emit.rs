//! Turn imported [`Task`]s into task files on disk.
//!
//! ADR-0012 clause 5: import writes new task files and nothing else. Writing
//! refuses to clobber, because an importer that silently overwrote a
//! hand-authored task would destroy exactly the record this tool exists to
//! protect.
//!
//! Two halves of that refusal, deliberately split:
//!
//! - **Here, by path.** [`write_task`] fails if the target file already exists.
//!   This is the last line of defence and needs no knowledge of the plan.
//! - **By id, in the caller.** A task's identity is its `id` field, not its
//!   path (ADR-0010), so an id can already be taken by a *differently named*
//!   file. Only something that has loaded the plan can see that, so the command
//!   layer checks ids before calling here.

use std::path::{Path, PathBuf};

use aplan_core::Task;

/// Why a task could not be written.
#[derive(Debug, thiserror::Error)]
pub enum EmitError {
    /// The target file is already there. Never overwritten (clause 5).
    #[error("{}: already exists, refusing to overwrite", .path.display())]
    Exists {
        /// The file that would have been clobbered.
        path: PathBuf,
    },
    /// The id cannot be a file name.
    #[error("task id {id:?} is not usable as a file name")]
    UnusableId {
        /// The offending id.
        id: String,
    },
    /// The frontmatter could not be serialised.
    #[error("serialising {id}: {message}")]
    Serialise {
        /// The task being written.
        id: String,
        /// What went wrong.
        message: String,
    },
    /// The file could not be written.
    #[error("writing {}: {source}", .path.display())]
    Io {
        /// The target file.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
}

/// Whether `id` is safe to use as a file stem.
///
/// Rejects path separators and the traversal names outright rather than
/// sanitising them: a silently rewritten id would break the link between the
/// file and the plan, and imported ids come from someone else's repository.
///
/// This is a containment check, not a portability one. Characters that are
/// legal here but reserved on Windows (`:`, `?`, `*`, `<`, `>`, `|`, `"`) are
/// allowed through, because rejecting them would refuse ids a Unix repository
/// uses legitimately. A flavor that mints such ids will fail at the write on
/// Windows with an ordinary [`EmitError::Io`], which is the honest outcome.
fn is_usable_id(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', '\0']) && id.trim() == id
}

/// Render a task as the markdown-plus-frontmatter file `aplan` loads.
///
/// The frontmatter is serialised from [`Task`] itself, so it can never drift
/// from the model: fields the task does not carry are skipped by the type's own
/// `skip_serializing_if`, and field order follows the struct.
pub fn task_file(task: &Task, body: &str) -> Result<String, EmitError> {
    let frontmatter = serde_yaml_ng::to_string(task).map_err(|e| EmitError::Serialise {
        id: task.id.clone(),
        message: e.to_string(),
    })?;

    let mut out = String::with_capacity(frontmatter.len() + body.len() + 16);
    out.push_str("---\n");
    out.push_str(frontmatter.trim_end());
    out.push_str("\n---\n");
    if !body.trim().is_empty() {
        out.push('\n');
        out.push_str(body.trim_end());
        out.push('\n');
    }
    Ok(out)
}

/// The file a task is written to inside `dir`.
pub fn task_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.md"))
}

/// Write one task into `dir`, refusing to overwrite an existing file.
///
/// Returns the path written.
///
/// The refusal is the file system's, not a prior `exists()` check: the file is
/// opened with `create_new`, which fails atomically if something is already
/// there. A check-then-write would leave a window in which a concurrent import
/// (or the user's editor) created the file between the two, and losing a
/// hand-authored task to that race is precisely what clause 5 forbids.
pub fn write_task(dir: &Path, task: &Task, body: &str) -> Result<PathBuf, EmitError> {
    use std::io::Write as _;

    if !is_usable_id(&task.id) {
        return Err(EmitError::UnusableId {
            id: task.id.clone(),
        });
    }
    let path = task_path(dir, &task.id);
    let contents = task_file(task, body)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| EmitError::Io {
            path: path.clone(),
            source,
        })?;
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                EmitError::Exists { path: path.clone() }
            } else {
                EmitError::Io {
                    path: path.clone(),
                    source,
                }
            }
        })?;
    file.write_all(contents.as_bytes())
        .map_err(|source| EmitError::Io {
            path: path.clone(),
            source,
        })?;
    Ok(path)
}

/// A standard body recording where a task came from.
///
/// The body is free prose that `aplan` never parses, but an imported task with
/// no explanation is a task nobody can audit later, so every import leaves one.
pub fn provenance_body(source_name: &str, flavor: &str) -> String {
    format!(
        "## Why\n\n\
         Imported from `{source_name}` by `aplan import --format {flavor}`.\n\n\
         The source's own vocabulary is preserved in this task's tags. Import is lossy by\n\
         design (ADR-0012): whatever the task model could not hold was reported at import\n\
         time rather than dropped in silence.\n\n\
         ## Acceptance criteria\n\n\
         - [ ] restate what done means for this item, in this repository's terms\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_core::Status;

    fn task(id: &str) -> Task {
        Task {
            id: id.to_string(),
            title: "A title".to_string(),
            phase: 1,
            status: Status::Todo,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: vec!["kep:implementable".to_string()],
        }
    }

    #[test]
    fn the_file_is_fenced_frontmatter_then_body() {
        let out = task_file(&task("KEP-1"), "## Why\n\nBecause.").unwrap();
        assert!(out.starts_with("---\n"), "{out}");
        assert!(out.contains("\nid: KEP-1\n"), "{out}");
        assert!(out.contains("\n---\n\n## Why"), "{out}");
        assert!(out.ends_with('\n'), "files end with a newline");
    }

    #[test]
    fn absent_fields_are_omitted_not_written_as_null() {
        let out = task_file(&task("KEP-1"), "").unwrap();
        for absent in ["owner:", "effort_days:", "evidence:", "paths:", "traces:"] {
            assert!(!out.contains(absent), "{absent} should be skipped:\n{out}");
        }
        assert!(out.contains("tags:"), "a present field is written:\n{out}");
    }

    #[test]
    fn an_empty_body_leaves_a_valid_file() {
        let out = task_file(&task("KEP-1"), "   \n").unwrap();
        assert!(out.ends_with("---\n"), "{out:?}");
    }

    #[test]
    fn rendering_is_stable_across_runs() {
        let t = task("KEP-1");
        assert_eq!(task_file(&t, "b").unwrap(), task_file(&t, "b").unwrap());
    }

    #[test]
    fn writing_refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let t = task("KEP-1");
        let path = write_task(dir.path(), &t, "body").unwrap();
        assert!(path.exists());

        let err = write_task(dir.path(), &t, "different body").unwrap_err();
        assert!(matches!(err, EmitError::Exists { .. }), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            task_file(&t, "body").unwrap(),
            "the original must be untouched"
        );
    }

    #[test]
    fn an_id_that_is_not_a_file_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../escape", "a/b", "", ".", "..", " leading"] {
            let err = write_task(dir.path(), &task(bad), "body").unwrap_err();
            assert!(
                matches!(err, EmitError::UnusableId { .. }),
                "{bad:?} must be refused, got {err:?}"
            );
        }
    }

    #[test]
    fn the_provenance_body_names_the_source_and_the_flavor() {
        let b = provenance_body("kep.yaml", "kep");
        assert!(b.contains("kep.yaml"));
        assert!(b.contains("--format kep"));
    }
}
