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
    /// The task was written and could not be placed in its cut.
    #[error("{0}")]
    Place(String),
}

/// Whether `id` is safe to use as a file stem.
///
/// Rejects path separators and the traversal names outright rather than
/// sanitising them: a silently rewritten id would break the link between the
/// file and the plan, and imported ids come from someone else's repository.
///
/// Public because two paths write task files and both have to make this
/// refusal: `write_task` for one task, and `aplan add --from` for a batch, whose
/// ids come out of a manifest a script generated from a document (`A1-241`,
/// audit -- the batch path bypassed this and wrote outside the plan directory).
///
/// This is a containment check, not a portability one. Characters that are
/// legal here but reserved on Windows (`:`, `?`, `*`, `<`, `>`, `|`, `"`) are
/// allowed through, because rejecting them would refuse ids a Unix repository
/// uses legitimately. A flavor that mints such ids will fail at the write on
/// Windows with an ordinary [`EmitError::Io`], which is the honest outcome.
#[must_use]
pub fn is_usable_id(id: &str) -> bool {
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

/// A repeatable authoring input, with repeats removed and the order kept.
///
/// `--tag x --tag x` wrote the tag twice, and so did `--paths` and, when it
/// arrived, `--traces` (`A1-116`). None of it is caught by a rule: a duplicate
/// tag is what `APLAN026` had to be taught not to double-count, a duplicate glob
/// is judged twice by `APLAN012` and `APLAN019`, and a duplicate trace stamps one
/// fingerprint under two entries, so `APLAN028` compares a set against a list.
/// `drop` and `mv` already treat an id named twice as one; this is the same rule
/// for the fields `add` collects.
///
/// Order is the author's, because the file is read by people: the first mention
/// wins its place.
#[must_use]
pub fn deduped(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    values
        .iter()
        .filter(|v| seen.insert(v.as_str()))
        .cloned()
        .collect()
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
/// The body a scaffolded task starts with.
///
/// Beside `write_task`, which consumes it, because both surfaces that create a
/// task must create the same one. It lived in the `aplan` binary crate as a
/// private const, which is the third time a piece of shared behaviour has been on
/// the wrong side of that boundary: `trace_fingerprints` (`A1-100`) and the
/// verdict (`A1-101`) were the first two, and each drifted the moment a second
/// surface needed it.
///
/// Prompts, not content. Every line is a question only the author can answer, and
/// an `aplan` that answered them would be generating a plan rather than recording
/// one. The import paths ask a *different* question -- see
/// `aplan_core::scaffold`, which holds both.
///
/// Re-exported from `aplan_core::scaffold`, where it moved so that the rule
/// which refuses to accept it back as an answer reads the same constant this
/// writes (`A1-271`).
pub use aplan_core::scaffold::ADD_BODY;

/// Write a task and place it in a cut, or leave the plan exactly as it was.
///
/// `write_task` then `cut::place` is two writes, and the second can fail after
/// the first succeeded: an unwritable milestone is enough, and the result is a
/// task in no cut, which a plan with `require_cut_membership` rejects. Both
/// surfaces did that identically, so the parity audit was satisfied and the plan
/// was still broken (`A1-103`).
///
/// The task file is removed if placement fails, so the only states are "created
/// and placed" and "nothing happened". Not a transaction: if the *removal* fails
/// too, the error says so rather than pretending, because a caller told the plan
/// is untouched when it is not is worse off than one told the truth.
///
/// Placing is optional: with no cut there is one write and nothing to undo.
pub fn write_task_placed(
    out_dir: &std::path::Path,
    root: &std::path::Path,
    task: &aplan_core::Task,
    body: &str,
    cut_id: Option<&str>,
) -> Result<Option<std::path::PathBuf>, EmitError> {
    write_task(out_dir, task, body)?;
    let Some(cut_id) = cut_id else {
        return Ok(None);
    };
    // Reload, so the task just written is the one being placed.
    let plan = aplan_core::load_plan(root);
    match crate::cut::place(&plan, cut_id, std::slice::from_ref(&task.id)) {
        Ok(placed) => Ok(Some(placed.path)),
        Err(e) => {
            let written = task_path(out_dir, &task.id);
            match std::fs::remove_file(&written) {
                Ok(()) => Err(EmitError::Place(format!(
                    "placing `{}` in `{cut_id}`: {e}. The task file was removed, so the \
                     plan is as it was.",
                    task.id
                ))),
                // Names the command rather than a hand-edit: `mv --cut` puts a
                // task in a cut, including one that is in none, and the tool
                // sending an author to a text editor for something it does is
                // the defect `A1-265` was filed about one message over.
                Err(cleanup) => Err(EmitError::Place(format!(
                    "placing `{}` in `{cut_id}`: {e}. Removing {} afterwards also failed \
                     ({cleanup}), so the plan now has a task in no cut: put it in one with \
                     `aplan mv {} --cut {cut_id}`, or delete the file.",
                    task.id,
                    written.display(),
                    task.id
                ))),
            }
        }
    }
}

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
///
/// Carries no `ADR-NNNN` citation, deliberately (`A1-044`). This text is written
/// into a *consumer's* task file and committed there, and it is never
/// regenerated; a reference to this repository's ADR-0012 resolves here and is a
/// dangling pointer into a `docs/adr/` that does not contain it. The guarantee is
/// what the reader needs, and it survives without the number.
pub fn provenance_body(source_name: &str, flavor: &str) -> String {
    format!(
        "## Why\n\n\
         Imported from `{source_name}` by `aplan import --format {flavor}`.\n\n\
         The source's own vocabulary is preserved in this task's tags. Import is lossy by\n\
         design: whatever the task model could not hold was reported at import time rather\n\
         than dropped in silence.\n\n\
         ## Acceptance criteria\n\n\
         - [ ] {criterion}\n",
        criterion = aplan_core::scaffold::IMPORT_CRITERION
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
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: vec!["kep:implementable".to_string()],
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
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
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
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
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
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

#[cfg(test)]
mod placing {
    /// Repeats out, order kept: the file is read by people, so the first mention
    /// keeps its place (`A1-116`).
    #[test]
    fn deduped_keeps_the_first_mention_and_drops_the_rest() {
        let v = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert_eq!(super::deduped(&v(&["b", "a", "b"])), v(&["b", "a"]));
        assert_eq!(super::deduped(&v(&[])), v(&[]));
        assert_eq!(super::deduped(&v(&["a"])), v(&["a"]));
        // Not a set: a value that differs by a character is a different value,
        // and this must never "helpfully" fold near-misses together.
        assert_eq!(
            super::deduped(&v(&["src/**", "src/*", "src/**"])),
            v(&["src/**", "src/*"])
        );
    }

    use super::*;

    fn plan_dir(dir: &std::path::Path) {
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

    fn task(id: &str) -> aplan_core::Task {
        aplan_core::Task {
            id: id.to_string(),
            title: "Wire it".into(),
            phase: 0,
            status: aplan_core::Status::Todo,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        }
    }

    #[test]
    fn a_task_that_cannot_be_placed_is_not_left_behind() {
        // `A1-103`. Two writes, and the second can fail after the first
        // succeeded, leaving a task in no cut: a plan `require_cut_membership`
        // rejects. Both surfaces did this identically, so parity was satisfied
        // and the plan was still broken.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        plan_dir(&root);
        let milestone = root.join("tasks/M-s0.md");
        let mut perms = std::fs::metadata(&milestone).expect("meta").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&milestone, perms).expect("chmod");

        let err = write_task_placed(
            &root.join("tasks"),
            &root,
            &task("A-2"),
            ADD_BODY,
            Some("M-s0"),
        )
        .expect_err("the milestone cannot be written");

        assert!(
            err.to_string().contains("the plan is as it was"),
            "the caller must be told the state it is in: {err}"
        );
        assert!(
            !root.join("tasks/A-2.md").exists(),
            "the task file must be gone, or the plan has a task in no cut"
        );
    }

    #[test]
    fn a_task_that_places_stays_and_the_milestone_gains_it() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        plan_dir(&root);

        let placed = write_task_placed(
            &root.join("tasks"),
            &root,
            &task("A-2"),
            ADD_BODY,
            Some("M-s0"),
        )
        .expect("places")
        .expect("a milestone was edited");

        assert!(root.join("tasks/A-2.md").exists());
        let milestone = std::fs::read_to_string(&placed).expect("read");
        assert!(milestone.contains("A-2"), "{milestone}");
    }

    #[test]
    fn with_no_cut_there_is_one_write_and_nothing_to_undo() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("aplan");
        plan_dir(&root);

        let placed = write_task_placed(&root.join("tasks"), &root, &task("A-2"), ADD_BODY, None)
            .expect("writes");
        assert!(placed.is_none(), "no milestone was touched");
        assert!(root.join("tasks/A-2.md").exists());
    }
}
