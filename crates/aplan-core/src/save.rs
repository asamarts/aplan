//! Replacing a file somebody else may be replacing at the same time (`A1-226`).
//!
//! Every command that edits a plan read a file, computed a new version, and
//! called `std::fs::write`. Between the read and the write another process could
//! do the same, and the second writer silently discarded the first one's change.
//! Measured with twenty-four agents adding to one 2,000-member cut: over four
//! rounds, 23 of 24 memberships landed, then 22 of 24, then two clean rounds --
//! and **every agent was told it had succeeded** every time.
//!
//! Two separate defects, fixed together because they share a call site:
//!
//! 1. **The lost update.** The write takes an exclusive lock, and then refuses
//!    if the file no longer holds the bytes the caller read, so a writer that
//!    would clobber somebody is told so instead of doing it. A refusal is
//!    retryable, which is what `add` already does when an id collides.
//!
//! 2. **The truncating write.** `std::fs::write` opens with `O_TRUNC`, so an
//!    interrupted write leaves a *shorter* file. On a milestone that is every
//!    member of the cut orphaned at once. [`replace`] writes a temporary file
//!    beside the target and renames it, and rename is atomic: a reader sees the
//!    old bytes or the new ones, never half of either.
//!
//! **Compare-and-swap alone was not enough, and the test said so.** `A1-226`
//! weighed a lock against a compare-and-swap and chose the second, because it
//! needs no stale-lock story. Built that way, the reproduction still failed:
//! twelve writers, "10 reported success, 9 landed", on run after run. The window
//! between the comparison and the rename is a handful of syscalls and that is
//! still wide enough, because every writer is doing the same handful at the same
//! moment. So the write takes an exclusive lock as well, and the task's stated
//! preference is overruled by its own acceptance test rather than by argument.
//!
//! The lock is a file created with `create_new`, which is atomic, and released
//! by [`Drop`]. A holder that dies leaves it behind, so a lock older than
//! [`STALE`] is broken rather than waited on -- a write here takes microseconds,
//! so a lock that old is not a live writer under any scheduling.
//!
//! The comparison is still there and still earns its place: the lock only spans
//! this write, while the comparison spans the caller's whole read-compute-write,
//! which is where a *second command* editing the same file lands.

use std::path::{Path, PathBuf};

/// Why a replacement did not happen.
#[derive(Debug)]
pub enum SaveError {
    /// The file no longer holds what the caller read, so writing would discard
    /// somebody else's change. Retryable: read again, recompute, write again.
    Changed {
        /// The file that moved under the caller.
        path: PathBuf,
    },
    /// Another writer holds the lock and did not let go, which after [`STALE`]
    /// means it died holding it or something outside this tool is writing.
    Busy {
        /// The file that could not be locked.
        path: PathBuf,
    },
    /// The batch names one file twice, with two different replacements. A caller
    /// bug: applying whichever came last would be a silent wrong answer.
    Conflict {
        /// The file named twice.
        path: PathBuf,
    },
    /// The batch meant to *create* this file and something else got there first.
    ///
    /// Distinct from [`SaveError::Changed`]: nothing was discarded, and the
    /// remedy is different. A retry would collide again, because the id is
    /// already taken (`A1-241`).
    Exists {
        /// The file that was already there.
        path: PathBuf,
    },
    /// The filesystem refused.
    Io {
        /// The file being replaced.
        path: PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exists { path } => {
                write!(f, "{} already exists", path.display())
            }
            Self::Changed { path } => write!(
                f,
                "{} changed while this command was preparing its edit, so writing \
                 would have discarded the other change. Nothing was written; run \
                 the command again.",
                path.display()
            ),
            Self::Busy { path } => write!(
                f,
                "{} is locked by another `aplan` writing at the same time, and did not \
                 become free. Nothing was written; run the command again.",
                path.display()
            ),
            Self::Conflict { path } => write!(
                f,
                "this change names {} twice, with two different replacements. Nothing was \
                 written: applying one of them would be a silent choice between them.",
                path.display()
            ),
            Self::Io { path, source } => write!(f, "writing {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Changed { .. }
            | Self::Busy { .. }
            | Self::Conflict { .. }
            | Self::Exists { .. } => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}

/// What to tell somebody a failed write cost them, and how to recover.
///
/// The sentence lives here rather than at a call site because two surfaces
/// produce it: the commands, whose `retry` is an invocation (`aplan close`), and
/// the MCP tools, whose `retry` is a tool name (`aplan_close`). `A1-131`'s rule
/// is that an agent cannot close through MCP what the CLI refuses, and "cannot"
/// includes being told the same thing in the same words when the same condition
/// stops it -- which two independent `match` arms would not stay true to
/// (`A1-270`).
///
/// [`SaveError::Exists`] deliberately names the file rather than the retry: a
/// second attempt collides again, because the id is taken.
#[must_use]
pub fn advice(error: &SaveError, retry: &str) -> String {
    match error {
        SaveError::Changed { path } => format!(
            "{} changed while this was being prepared, so nothing was written. \
             Run `{retry}` again",
            path.display()
        ),
        SaveError::Busy { path } => format!(
            "another writer holds {}, so nothing was written. Run `{retry}` again",
            path.display()
        ),
        SaveError::Io { path, source } => format!("writing {}: {source}", path.display()),
        SaveError::Exists { path } => format!(
            "{} already exists, so nothing was written: the id is taken",
            path.display()
        ),
        // A batch naming one file twice with two different replacements is a
        // caller bug, and its own message already says so.
        other @ SaveError::Conflict { .. } => other.to_string(),
    }
}

/// Replace `path` with `updated`, but only while it still holds `expected`.
///
/// `expected` is the text the caller read before computing `updated`. If the
/// file has changed since, nothing is written and [`SaveError::Changed`] says
/// so.
///
/// # Errors
/// [`SaveError::Changed`] when the file moved under the caller,
/// [`SaveError::Busy`] when another writer holds it, and [`SaveError::Io`] when
/// the filesystem refused.
pub fn replace(path: &Path, expected: &str, updated: &str) -> Result<(), SaveError> {
    replace_all(&[Replacement {
        path: path.to_path_buf(),
        expected: expected.to_string(),
        updated: updated.to_string(),
    }])
}

/// One file's worth of a batch: what it must still hold, and what to put there.
#[derive(Debug, Clone)]
pub struct Replacement {
    /// The file to replace.
    pub path: PathBuf,
    /// The text the caller read before computing `updated`.
    pub expected: String,
    /// What to write.
    pub updated: String,
}

/// Replace several files as one change: all of them, or none.
///
/// **Why a batch exists.** `mv` places a task in the destination cut and then
/// takes it out of the source. Written one at a time, a failure between the two
/// leaves the task in both cuts, and `APLAN022` only reports that where the
/// repository asked for `require_cut_membership` -- so the usual outcome is a
/// plan a person has to repair by hand, with nothing saying so. That was rare
/// while "a file that changed under us" meant a permission problem; `A1-226`
/// made it reachable by ordinary contention, since another writer holding the
/// lock for two seconds is an ordinary Tuesday under a fleet (`A1-234`).
///
/// **Every lock first, then every comparison, then every rename.** A caller
/// cannot observe a half-applied batch through this function: nothing is renamed
/// until every file has been locked, checked against what the caller read, and
/// proved writable. What remains is the rename loop itself, where only the
/// filesystem can fail -- a full disk between two renames -- and that is the one
/// case the message still has to describe.
///
/// **Locks are taken in resolved-path order**, which is what makes a deadlock
/// impossible rather than unlikely: two moves in opposite directions ask for the
/// same two files in the same sequence, so one waits and the other proceeds.
/// A property of the ordering, not of the timing.
///
/// # Errors
/// [`SaveError::Changed`] when any file moved under the caller,
/// [`SaveError::Busy`] when any is locked, and [`SaveError::Io`] when the
/// filesystem refused. In every case nothing has been renamed.
pub fn replace_all(batch: &[Replacement]) -> Result<(), SaveError> {
    // Resolved first, and sorted on the resolved path: two names for one file
    // must take one lock, and two batches must agree on the order.
    let mut resolved: Vec<(PathBuf, &Replacement)> = batch
        .iter()
        .map(|r| {
            (
                std::fs::canonicalize(&r.path).unwrap_or_else(|_| r.path.clone()),
                r,
            )
        })
        .collect();
    resolved.sort_by(|a, b| a.0.cmp(&b.0));

    // One path named twice with two different answers is a caller bug, and
    // applying the last of them silently is the shape of wrong answer this
    // module exists to refuse. Two *identical* entries are harmless and stay
    // allowed, because a batch assembled from several sources can produce them.
    for pair in resolved.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.0 == b.0 && (a.1.updated != b.1.updated || a.1.expected != b.1.expected) {
            return Err(SaveError::Conflict { path: a.0.clone() });
        }
    }

    // Before any lock, because a lock file lives *in* the target's directory: a
    // creation into a directory that is not there could not even be locked, and
    // reported "No such file or directory" from the lock rather than from the
    // write. `emit::write_task` creates the parent for the same reason, so the
    // first `aplan add --from` in a repository with no plan directory yet works
    // exactly as the first `aplan add` does (`A1-241`, audit).
    //
    // Not undone if the batch then fails. An empty directory is not a
    // half-applied change: the loader reads `*.md` and finds none, which is what
    // it found before.
    for (path, item) in &resolved {
        if item.expected.is_empty()
            && !path.exists()
            && let Some(dir) = path.parent()
        {
            std::fs::create_dir_all(dir).map_err(|source| SaveError::Io {
                path: path.clone(),
                source,
            })?;
        }
    }

    let mut guards = Vec::with_capacity(resolved.len());
    for (path, item) in &resolved {
        // One lock per file even when a batch names it twice: taking the second
        // would block on the first for the full wait and then report `Busy`
        // against the caller's own hold.
        if !guards.iter().any(|g: &Lock| g.covers(path)) {
            guards.push(Lock::acquire(path)?);
        }
        check_replaceable(path, &item.expected)?;
    }
    // Staged under a guard: a failure part-way through leaves temporary files
    // beside their targets, and the loader ignores them only because they are
    // not named `*.md`. Litter that survives every run is what the single-file
    // path was careful about, and a batch has more of it to lose (`A1-234`,
    // audit).
    let mut pending = Staged::default();
    for (path, item) in &resolved {
        pending
            .files
            .push((path.clone(), stage(path, &item.updated)?));
    }
    while let Some((path, temp)) = pending.files.pop() {
        std::fs::rename(&temp, &path).map_err(|source| {
            let _ = std::fs::remove_file(&temp);
            SaveError::Io { path, source }
        })?;
    }
    Ok(())
}

/// Temporary files written but not yet renamed, removed if the batch does not
/// finish.
#[derive(Default)]
struct Staged {
    files: Vec<(PathBuf, PathBuf)>,
}

impl Drop for Staged {
    fn drop(&mut self) {
        for (_, temp) in &self.files {
            let _ = std::fs::remove_file(temp);
        }
    }
}

/// The checks that must pass before anything in a batch is written.
fn check_replaceable(path: &Path, expected: &str) -> Result<(), SaveError> {
    // An empty `expected` on a file that is not there is a *creation*: the
    // caller read nothing because there was nothing to read. `add --from` writes
    // a batch of new tasks and the cuts that hold them in one change, and
    // splitting that into "create the files, then replace the milestones" is the
    // half-applied plan this function exists to prevent (`A1-241`).
    //
    // Only when `expected` is empty. A file that vanished between the read and
    // the write leaves a non-empty `expected`, so that case is still the `Io`
    // error it always was, rather than a silent re-creation of somebody's
    // deleted work.
    if expected.is_empty() && !path.exists() {
        return Ok(());
    }
    // Meant to create, and something is there. `emit::write_task` refuses this
    // with `create_new`, and a batch must refuse it for the same reason: the id
    // is taken, and overwriting a hand-authored file is the one thing a plan
    // directory must never do.
    if expected.is_empty() {
        return Err(SaveError::Exists {
            path: path.to_path_buf(),
        });
    }
    let current = std::fs::read_to_string(path).map_err(|source| SaveError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if current != expected {
        return Err(SaveError::Changed {
            path: path.to_path_buf(),
        });
    }

    // `rename` replaces a directory entry, so it would happily overwrite a file
    // the caller has no write permission on -- which `std::fs::write` refused and
    // `A1-103` relies on ("a task that cannot be placed is not left behind").
    // Ask for write access first, so the refusal survives the change of
    // mechanism.
    drop(
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|source| SaveError::Io {
                path: path.to_path_buf(),
                source,
            })?,
    );
    Ok(())
}

/// Write the replacement beside its target and hand back the temporary name.
///
/// Everything that can fail on the way to a rename happens here, so the rename
/// loop that follows is the only place a batch can come apart -- and only on a
/// filesystem error.
fn stage(path: &Path, updated: &str) -> Result<PathBuf, SaveError> {
    // The replacement is a new file, so it is created with the default mode
    // rather than the target's. Carry the target's across, or a milestone an
    // author restricted to themselves becomes world-readable the first time
    // anything is added to its cut.
    let mode = std::fs::metadata(path).map(|m| m.permissions()).ok();

    let (temp, mut handle) = create_temp(path)?;
    let written = {
        use std::io::Write;
        handle
            .write_all(updated.as_bytes())
            .and_then(|()| handle.sync_all())
    };
    drop(handle);
    if let Err(source) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(SaveError::Io {
            path: path.to_path_buf(),
            source,
        });
    }

    if let Some(mode) = mode
        && let Err(source) = std::fs::set_permissions(&temp, mode)
    {
        let _ = std::fs::remove_file(&temp);
        return Err(SaveError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(temp)
}

/// How long a lock may be held before it is assumed to belong to a dead writer.
///
/// A write here is a comparison, a small file and a rename: microseconds. Thirty
/// seconds is not a slow writer under any scheduling, it is a writer that is
/// gone, so breaking the lock is safe and waiting for it is not.
pub const STALE: std::time::Duration = std::time::Duration::from_secs(30);

/// An exclusive lock on one file, released when dropped.
struct Lock {
    path: PathBuf,
    /// The file it was taken for, so a batch naming one file twice takes one
    /// lock rather than blocking on its own hold.
    target: PathBuf,
}

impl Lock {
    /// Whether this lock is the one for `target`.
    fn covers(&self, target: &Path) -> bool {
        self.target == target
    }

    /// Take it, waiting briefly for a live writer and breaking a dead one's.
    fn acquire(target: &Path) -> Result<Self, SaveError> {
        let dir = target.parent().unwrap_or_else(|| Path::new("."));
        let stem = target
            .file_name()
            .map_or_else(|| "plan".to_string(), |n| n.to_string_lossy().into_owned());
        let path = dir.join(format!(".{stem}.lock"));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut broke_one = false;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => {
                    return Ok(Self {
                        path,
                        target: target.to_path_buf(),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(SaveError::Io {
                        path: target.to_path_buf(),
                        source,
                    });
                }
            }

            // The stale check runs on every attempt rather than only after the
            // deadline: a lock left by a dead writer should cost the next one no
            // wait at all. Broken once, so a pathological loop cannot spin here.
            // Breaking it retries immediately; only a live holder is waited on.
            if !broke_one && held_longer_than(&path, STALE) {
                let _ = std::fs::remove_file(&path);
                broke_one = true;
            } else if std::time::Instant::now() >= deadline {
                return Err(SaveError::Busy {
                    path: target.to_path_buf(),
                });
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Best effort: a lock left behind is broken by the next writer once it
        // is `STALE`, which is the case this cannot make worse.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Whether `path` was created longer ago than `age`, treating an unreadable
/// timestamp as "not stale" so a lock is never broken on a guess.
fn held_longer_than(path: &Path, age: std::time::Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|held| held > age)
}

/// A temporary file beside `path`, created exclusively so two processes racing
/// here cannot pick the same name.
///
/// Beside, rather than in a temporary directory, because `rename` is only
/// atomic within one filesystem. The name carries no extension the loader reads
/// (`*.md`) and starts with a dot, so a crash between creation and rename
/// leaves something inert rather than something that parses as a task.
///
/// `create_new` is what makes this race-free without a process id, which
/// `aplan-core` may not ask for: the hermetic gate forbids `std::process` here.
fn create_temp(path: &Path) -> Result<(PathBuf, std::fs::File), SaveError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .map_or_else(|| "plan".to_string(), |n| n.to_string_lossy().into_owned());
    let mut last = None;
    for n in 0..1024 {
        let candidate = dir.join(format!(".{stem}.tmp{n}"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            // Taken by another writer between our check and our create, which is
            // the race this loop exists for: try the next name.
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => last = Some(source),
        }
    }
    Err(SaveError::Io {
        path: path.to_path_buf(),
        source: last.unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "no free temporary name beside the target",
            )
        }),
    })
}

#[cfg(test)]
mod tests {

    /// **A batch is all of its files or none of them.**
    ///
    /// One file having moved under the caller must leave every other file
    /// untouched, or `mv` is back to the half-applied move this exists to end
    /// (`A1-234`).
    #[test]
    fn a_batch_writes_every_file_or_none_of_them() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "one\n").expect("a");
        std::fs::write(&b, "two\n").expect("b");

        // `b`'s expectation is stale, as it would be if somebody else had
        // written it since this caller read it.
        let err = replace_all(&[
            Replacement {
                path: a.clone(),
                expected: "one\n".into(),
                updated: "ONE\n".into(),
            },
            Replacement {
                path: b.clone(),
                expected: "stale\n".into(),
                updated: "TWO\n".into(),
            },
        ])
        .expect_err("a stale expectation must refuse");
        assert!(matches!(err, SaveError::Changed { .. }), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&a).expect("a"),
            "one\n",
            "the first file was written before the second was checked"
        );
        assert_eq!(std::fs::read_to_string(&b).expect("b"), "two\n");

        // And with both expectations current, both land.
        replace_all(&[
            Replacement {
                path: a.clone(),
                expected: "one\n".into(),
                updated: "ONE\n".into(),
            },
            Replacement {
                path: b.clone(),
                expected: "two\n".into(),
                updated: "TWO\n".into(),
            },
        ])
        .expect("both current");
        assert_eq!(std::fs::read_to_string(&a).expect("a"), "ONE\n");
        assert_eq!(std::fs::read_to_string(&b).expect("b"), "TWO\n");
    }

    /// A batch naming one file twice takes one lock, rather than waiting the
    /// full two seconds on its own hold and then reporting the caller busy
    /// against itself.
    #[test]
    fn a_batch_that_names_one_file_twice_does_not_wait_on_itself() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let a = dir.path().join("a.md");
        std::fs::write(&a, "one\n").expect("a");

        let began = std::time::Instant::now();
        let err = replace_all(&[
            Replacement {
                path: a.clone(),
                expected: "one\n".into(),
                updated: "ONE\n".into(),
            },
            Replacement {
                path: a.clone(),
                expected: "one\n".into(),
                updated: "ONE\n".into(),
            },
        ]);
        assert!(err.is_ok(), "{err:?}");
        assert!(
            began.elapsed() < std::time::Duration::from_secs(1),
            "it waited on a lock it was already holding"
        );
    }

    /// A batch that contradicts itself is refused, not silently resolved.
    ///
    /// Naming one file twice with two different replacements has no right
    /// answer, and applying whichever came last is the shape of silent wrong
    /// answer this module exists to refuse. Two *identical* entries stay
    /// allowed, because a batch assembled from several sources can produce them
    /// (`A1-234`, audit).
    #[test]
    fn a_batch_that_names_one_file_twice_with_two_answers_is_refused() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let a = dir.path().join("a.md");
        std::fs::write(&a, "one\n").expect("a");
        let entry = |updated: &str| Replacement {
            path: a.clone(),
            expected: "one\n".into(),
            updated: updated.into(),
        };

        let err =
            replace_all(&[entry("ONE\n"), entry("OTHER\n")]).expect_err("two answers for one file");
        assert!(matches!(err, SaveError::Conflict { .. }), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&a).expect("a"),
            "one\n",
            "one of the two was applied anyway"
        );

        replace_all(&[entry("ONE\n"), entry("ONE\n")]).expect("identical entries are fine");
        assert_eq!(std::fs::read_to_string(&a).expect("a"), "ONE\n");
    }

    /// **A batch that does not finish leaves nothing behind.**
    ///
    /// Every replacement is written beside its target before any is renamed, so
    /// a refusal after staging would leave temporary files there for ever. The
    /// loader ignores them only because they are not named `*.md`, which is a
    /// thin reason to rely on.
    #[test]
    fn a_refused_batch_leaves_no_temporary_files() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "one\n").expect("a");
        std::fs::write(&b, "two\n").expect("b");

        let _ = replace_all(&[
            Replacement {
                path: a.clone(),
                expected: "one\n".into(),
                updated: "ONE\n".into(),
            },
            Replacement {
                path: b.clone(),
                expected: "stale\n".into(),
                updated: "TWO\n".into(),
            },
        ])
        .expect_err("stale");

        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files survived: {leftovers:?}"
        );
    }

    /// And the case the refusal above cannot reach: a batch that fails **after**
    /// staging.
    ///
    /// The check phase refuses before anything is written, so a stale
    /// expectation leaves nothing to clean up and says nothing about the guard.
    /// This exhausts the temporary names beside the *second* file, so the first
    /// is staged and the batch then fails -- which is where a leaked temporary
    /// would survive every future run (`A1-234`, audit).
    #[test]
    fn a_batch_that_fails_after_staging_removes_what_it_staged() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "one\n").expect("a");
        std::fs::write(&b, "two\n").expect("b");
        // Every name `create_temp` would try for `b`, taken.
        for n in 0..1024 {
            std::fs::write(dir.path().join(format!(".b.md.tmp{n}")), "").expect("squat");
        }

        let err = replace_all(&[
            Replacement {
                path: a.clone(),
                expected: "one\n".into(),
                updated: "ONE\n".into(),
            },
            Replacement {
                path: b.clone(),
                expected: "two\n".into(),
                updated: "TWO\n".into(),
            },
        ])
        .expect_err("no temporary name is free for `b`");
        assert!(matches!(err, SaveError::Io { .. }), "{err:?}");

        let leaked: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".a.md.tmp"))
            .collect();
        assert!(
            leaked.is_empty(),
            "the first file's temporary survived the failed batch: {leaked:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&a).expect("a"),
            "one\n",
            "and it was renamed into place anyway"
        );
    }

    /// **The order is a property of the paths, not of the caller.**
    ///
    /// Two moves in opposite directions ask for the same two files. Taking them
    /// in the order each caller happened to list them is how they block each
    /// other; taking them in resolved-path order is why one proceeds. The
    /// acquire timeout means no order can hang for ever -- a cycle degrades to
    /// `Busy` after the wait -- so what sorting buys is that opposite batches do
    /// not *both* fail.
    ///
    /// Observed rather than argued: both expectations are stale, so the batch
    /// refuses at whichever file it reaches first, and the file it names is the
    /// order. Given in reverse, a sorted batch still names `a.md`.
    #[test]
    fn a_batch_visits_files_in_path_order_whatever_order_it_was_given() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "one\n").expect("a");
        std::fs::write(&b, "two\n").expect("b");

        for pair in [[&a, &b], [&b, &a]] {
            let err = replace_all(
                &pair
                    .iter()
                    .map(|p| Replacement {
                        path: (*p).clone(),
                        expected: "stale\n".into(),
                        updated: "x\n".into(),
                    })
                    .collect::<Vec<_>>(),
            )
            .expect_err("both expectations are stale");
            match err {
                SaveError::Changed { path } => assert!(
                    path.ends_with("a.md"),
                    "the caller's order decided which file was reached first: {path:?}"
                ),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(
            std::fs::read_to_string(&b).expect("b"),
            "two\n",
            "a file was written despite the refusal"
        );
    }
    use super::*;

    fn at(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).expect("seed");
        p
    }

    #[test]
    fn a_file_that_still_holds_what_was_read_is_replaced() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        replace(&p, "before\n", "after\n").expect("replace");
        assert_eq!(std::fs::read_to_string(&p).expect("read"), "after\n");
    }

    #[test]
    fn a_file_that_moved_is_refused_and_left_alone() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        std::fs::write(&p, "somebody else\n").expect("interleave");

        let err = replace(&p, "before\n", "after\n").expect_err("must refuse");
        assert!(matches!(err, SaveError::Changed { .. }), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&p).expect("read"),
            "somebody else\n",
            "a refused write must not touch the file"
        );
    }

    #[test]
    fn the_refusal_says_what_to_do_about_it() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        std::fs::write(&p, "moved\n").expect("interleave");
        let err = replace(&p, "before\n", "after\n").expect_err("must refuse");
        let said = err.to_string();
        assert!(said.contains("Nothing was written"), "{said}");
        assert!(said.contains("run the command again"), "{said}");
    }

    #[test]
    fn no_temporary_file_survives_a_success() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        replace(&p, "before\n", "after\n").expect("replace");
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "m.md")
            .collect();
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    #[test]
    fn no_temporary_file_survives_a_refusal() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        std::fs::write(&p, "moved\n").expect("interleave");
        let _ = replace(&p, "before\n", "after\n");
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "m.md")
            .collect();
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    /// The temporary name must not be one the loader would read, or a crash
    /// between creation and rename would add a task nobody wrote.
    #[test]
    fn the_temporary_name_is_not_one_the_loader_reads() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "A1-001.md", "x\n");
        let (temp, file) = create_temp(&p).expect("temp");
        drop(file);
        let name = temp
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with('.'), "{name}");
        assert!(
            temp.extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case("md")),
            "{name} would be loaded as a task"
        );
        std::fs::remove_file(&temp).expect("clean");
    }

    /// A rename installs a new file, so the target's mode has to be carried
    /// across or a restricted plan file quietly becomes readable by everyone.
    #[test]
    fn the_targets_permissions_survive_the_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).expect("chmod");

        replace(&p, "before\n", "after\n").expect("replace");

        let mode = std::fs::metadata(&p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode became {mode:o}");
        assert_eq!(std::fs::read_to_string(&p).expect("read"), "after\n");
    }

    /// A plan file reached through a symlink must be written *through*, not
    /// replaced: renaming over the link would strand the real file.
    #[test]
    fn a_symlinked_target_is_written_through_rather_than_replaced() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let real = at(dir.path(), "real.md", "before\n");
        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        replace(&link, "before\n", "after\n").expect("replace");

        assert!(
            std::fs::symlink_metadata(&link)
                .expect("lstat")
                .file_type()
                .is_symlink(),
            "the symlink was replaced by a regular file"
        );
        assert_eq!(
            std::fs::read_to_string(&real).expect("read real"),
            "after\n",
            "the write did not reach the file the link points at"
        );
    }

    /// A file that is not there at all reports the filesystem's reason rather
    /// than pretending it changed.
    #[test]
    fn a_missing_target_reports_the_io_error() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let missing = dir.path().join("gone.md");
        let err = replace(&missing, "before\n", "after\n").expect_err("must fail");
        assert!(matches!(err, SaveError::Io { .. }), "{err:?}");
        assert!(err.to_string().contains("gone.md"), "{err}");
        assert!(
            std::error::Error::source(&err).is_some(),
            "an io failure must keep the filesystem's own error as its source"
        );
    }

    /// A directory nothing may be created in fails at the lock, which is the
    /// first thing that writes.
    #[test]
    fn a_directory_that_refuses_new_files_is_reported() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        let mut perms = std::fs::metadata(dir.path()).expect("meta").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(dir.path(), perms).expect("chmod dir");

        let outcome = replace(&p, "before\n", "after\n");

        // Restore before asserting, or the tempdir cannot clean itself up.
        let mut back = std::fs::metadata(dir.path()).expect("meta").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        back.set_readonly(false);
        std::fs::set_permissions(dir.path(), back).expect("chmod back");

        let err = outcome.expect_err("a lock cannot be created here");
        assert!(matches!(err, SaveError::Io { .. }), "{err:?}");
    }

    /// The `Changed` and `Busy` refusals are not filesystem failures, so they
    /// carry no source to unwrap.
    #[test]
    fn a_refusal_has_no_underlying_error() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        std::fs::write(&p, "moved\n").expect("interleave");
        let err = replace(&p, "before\n", "after\n").expect_err("must refuse");
        assert!(std::error::Error::source(&err).is_none(), "{err:?}");
    }

    /// `rename` does not consult the target's permissions, so this is checked
    /// separately or a read-only plan file silently becomes writable (`A1-103`).
    #[test]
    fn a_read_only_target_is_still_refused() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        let mut perms = std::fs::metadata(&p).expect("meta").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&p, perms).expect("chmod");

        let err = replace(&p, "before\n", "after\n").expect_err("must refuse");
        assert!(matches!(err, SaveError::Io { .. }), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&p).expect("read"),
            "before\n",
            "a read-only file must keep its contents"
        );
        assert!(
            !dir.path().join(".m.md.lock").exists(),
            "the lock outlived the refusal"
        );
    }

    /// A lock a dead writer left behind must not block the next one forever.
    #[test]
    fn a_stale_lock_is_broken_rather_than_waited_on() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        let lock = dir.path().join(".m.md.lock");
        std::fs::write(&lock, "").expect("stale lock");
        let old = std::time::SystemTime::now() - (STALE * 2);
        std::fs::File::open(&lock)
            .expect("open lock")
            .set_modified(old)
            .expect("age the lock");

        let started = std::time::Instant::now();
        replace(&p, "before\n", "after\n").expect("a stale lock must not stop the write");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "breaking a stale lock should be immediate, took {:?}",
            started.elapsed()
        );
        assert_eq!(std::fs::read_to_string(&p).expect("read"), "after\n");
    }

    /// A lock a *live* writer holds must refuse rather than clobber.
    #[test]
    fn a_held_lock_refuses_and_says_to_retry() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        std::fs::write(dir.path().join(".m.md.lock"), "").expect("fresh lock");

        let err = replace(&p, "before\n", "after\n").expect_err("must refuse");
        assert!(matches!(err, SaveError::Busy { .. }), "{err:?}");
        assert!(err.to_string().contains("run the command again"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&p).expect("read"),
            "before\n",
            "a refused write must not touch the file"
        );
    }

    /// The lock is released even when the write refuses, or one `Changed` would
    /// wedge the file until `STALE`.
    #[test]
    fn the_lock_is_released_after_a_refusal() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        std::fs::write(&p, "moved\n").expect("interleave");
        let _ = replace(&p, "before\n", "after\n");
        assert!(
            !dir.path().join(".m.md.lock").exists(),
            "the lock outlived the refusal"
        );
        replace(&p, "moved\n", "next\n").expect("the next writer must not be blocked");
    }

    /// Every candidate name taken is the loop's own failure case, and it must
    /// report rather than spin or overwrite one of them.
    #[test]
    fn a_directory_with_every_temporary_name_taken_is_reported() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "before\n");
        for n in 0..1024 {
            std::fs::write(dir.path().join(format!(".m.md.tmp{n}")), "").expect("occupy");
        }

        let err = create_temp(&p).expect_err("no name is free");
        assert!(matches!(err, SaveError::Io { .. }), "{err:?}");
        assert!(
            std::fs::read_to_string(&p).expect("read") == "before\n",
            "the target must be untouched when no temporary can be made"
        );
    }

    /// Two writers cannot pick the same temporary name, which is what makes the
    /// exclusive create worth its loop.
    #[test]
    fn two_temporaries_beside_one_file_do_not_collide() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "m.md", "x\n");
        let (a, fa) = create_temp(&p).expect("first");
        let (b, fb) = create_temp(&p).expect("second");
        assert_ne!(a, b);
        drop(fa);
        drop(fb);
        std::fs::remove_file(&a).expect("clean");
        std::fs::remove_file(&b).expect("clean");
    }

    /// **A batch creates and replaces in one change** (`A1-241`).
    ///
    /// `add --from` writes a batch of new task files *and* the milestones that
    /// hold them. Splitting that into "create the files, then replace the
    /// milestones" is the half-applied plan this module exists to prevent: a
    /// failure between them leaves either tasks in no cut or cuts naming tasks
    /// that do not exist.
    #[test]
    fn a_batch_creates_a_new_file_beside_a_replacement() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let existing = at(dir.path(), "m.md", "before\n");
        let fresh = dir.path().join("new.md");

        replace_all(&[
            Replacement {
                path: existing.clone(),
                expected: "before\n".into(),
                updated: "after\n".into(),
            },
            Replacement {
                path: fresh.clone(),
                expected: String::new(),
                updated: "made\n".into(),
            },
        ])
        .expect("both");

        assert_eq!(std::fs::read_to_string(&existing).expect("read"), "after\n");
        assert_eq!(std::fs::read_to_string(&fresh).expect("read"), "made\n");
    }

    /// **A creation that collides writes nothing at all.**
    ///
    /// `emit::write_task` refuses an existing file with `create_new`, and a batch
    /// has to refuse it for the same reason: the id is taken, and overwriting a
    /// hand-authored task is the one thing a plan directory must never do. The
    /// replacement beside it must not land either, or the batch was not a batch.
    #[test]
    fn a_creation_over_an_existing_file_refuses_the_whole_batch() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let existing = at(dir.path(), "m.md", "before\n");
        let taken = at(dir.path(), "taken.md", "hand written\n");

        let err = replace_all(&[
            Replacement {
                path: existing.clone(),
                expected: "before\n".into(),
                updated: "after\n".into(),
            },
            Replacement {
                path: taken.clone(),
                expected: String::new(),
                updated: "generated\n".into(),
            },
        ])
        .expect_err("the id is taken");

        assert!(matches!(err, SaveError::Exists { .. }), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&taken).expect("read"),
            "hand written\n",
            "the file that was there is untouched"
        );
        assert_eq!(
            std::fs::read_to_string(&existing).expect("read"),
            "before\n",
            "and so is the replacement that would have succeeded alone"
        );
    }

    /// **A file that vanished is still an error, not a re-creation.**
    ///
    /// The creation path is opened only by an *empty* `expected`. A caller that
    /// read something and then found it gone has lost somebody's work, and
    /// writing its own copy over the grave would hide that.
    #[test]
    fn a_deleted_file_is_not_quietly_written_again() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let gone = dir.path().join("gone.md");

        let err = replace_all(&[Replacement {
            path: gone.clone(),
            expected: "what the caller read\n".into(),
            updated: "its edit\n".into(),
        }])
        .expect_err("the file is not there");

        assert!(matches!(err, SaveError::Io { .. }), "{err:?}");
        assert!(!gone.exists(), "and nothing was created in its place");
    }

    /// **A creation makes the directory it writes into.**
    ///
    /// The lock file lives beside the target, so a creation into a directory
    /// that does not exist could not even be locked: it failed reporting "No
    /// such file or directory" about the path it was about to make. That is the
    /// state every adoption starts in (`A1-241`, audit).
    #[test]
    fn a_creation_makes_the_directory_it_writes_into() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let nested = dir.path().join("plan/tasks/A-1.md");

        replace_all(&[Replacement {
            path: nested.clone(),
            expected: String::new(),
            updated: "made\n".into(),
        }])
        .expect("creates");

        assert_eq!(std::fs::read_to_string(&nested).expect("read"), "made\n");
    }
}
