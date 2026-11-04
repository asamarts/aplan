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

/// Replace one private state file with the same guarded atomic protocol.
///
/// When the target is first created, its staged file is restricted to the
/// current user before any bytes are written or published. Existing targets
/// retain their mode exactly as [`replace`] does. Callers still have to reject
/// an existing insecure target before invoking this function; silently fixing
/// one would briefly trust bytes other users could have changed.
///
/// # Errors
///
/// The same errors as [`replace`].
pub fn replace_private(path: &Path, expected: &str, updated: &str) -> Result<(), SaveError> {
    replace_all_inner(
        &[Replacement {
            path: path.to_path_buf(),
            expected: expected.to_string(),
            updated: updated.to_string(),
        }],
        None,
        true,
        None,
    )
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

/// One file whose identity moves while its content is replaced.
///
/// Both names are locked and checked before anything is renamed. `expected` is
/// the content read at `from`; when the names differ, `to` must not exist. An
/// equal pair is an in-place identity rewrite that still gets the recovering
/// transaction used for a filename move. This is intentionally distinct from a
/// creation plus a deletion: splitting those would expose a plan containing
/// both ids, or neither, between the two writes.
#[derive(Debug, Clone)]
pub struct Relocation {
    /// The file that currently exists.
    pub from: PathBuf,
    /// The free path it moves to.
    pub to: PathBuf,
    /// The bytes the caller read at `from`.
    pub expected: String,
    /// The content to install at `to`.
    pub updated: String,
}

/// Replace several files as one guarded change.
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
/// filesystem can fail -- a full disk between two renames -- and that late case
/// can leave a partial publication. Callers that need to present an exact
/// outcome must inspect the affected paths after a [`SaveError::Io`].
///
/// **Locks are taken in resolved-path order**, which is what makes a deadlock
/// impossible rather than unlikely: two moves in opposite directions ask for the
/// same two files in the same sequence, so one waits and the other proceeds.
/// A property of the ordering, not of the timing.
///
/// # Errors
/// [`SaveError::Changed`] when any file moved under the caller,
/// [`SaveError::Busy`] when any is locked, and [`SaveError::Io`] when the
/// filesystem refused. Conflict, lock, comparison and staging failures happen
/// before any rename; an I/O failure in the final rename loop may happen after
/// an earlier member was published.
pub fn replace_all(batch: &[Replacement]) -> Result<(), SaveError> {
    replace_all_inner(batch, None, false, None)
}

/// Replace several files while holding one additional shared-snapshot lock.
///
/// The shared lock replaces per-destination locks for this batch, then every
/// destination is compare-and-swap checked before the callback runs. Commands
/// whose invariant spans directory membership use the same `guard_target`, so
/// two otherwise independent file creations serialize and the later writer
/// rechecks the complete snapshot. A false callback result is reported as
/// [`SaveError::Changed`] and writes nothing. Unlike an ordinary broad batch,
/// this shared-namespace form backs up each existing member during publication
/// and restores every earlier rename if a later rename fails. That stronger
/// boundary keeps a created task and its retained capture disposition from
/// becoming observable separately.
///
/// # Errors
///
/// The same errors as [`replace_all`].
pub fn replace_all_guarded(
    batch: &[Replacement],
    guard_target: &Path,
    mut snapshot_is_current: impl FnMut() -> bool,
) -> Result<(), SaveError> {
    replace_all_inner(
        batch,
        None,
        false,
        Some(PublicationGuard {
            target: guard_target,
            snapshot_is_current: &mut snapshot_is_current,
        }),
    )
}

/// Replace a batch and relocate one member as one guarded change.
///
/// Every replacement, the relocation source, and its destination are locked
/// and checked before a rename occurs. A stale source, a concurrently-created
/// destination, or a stale sibling therefore leaves every file untouched. The
/// source's permissions are carried to the destination.
///
/// # Errors
///
/// The same errors as [`replace_all`], plus [`SaveError::Exists`] when the
/// destination is occupied. Every pre-write refusal leaves the batch unchanged.
pub fn replace_all_relocating(
    batch: &[Replacement],
    relocation: &Relocation,
) -> Result<(), SaveError> {
    replace_all_inner(batch, Some(relocation), false, None)
}

struct PublicationGuard<'a> {
    target: &'a Path,
    snapshot_is_current: &'a mut dyn FnMut() -> bool,
}

fn replace_all_inner(
    batch: &[Replacement],
    relocation: Option<&Relocation>,
    private_new: bool,
    mut publication_guard: Option<PublicationGuard<'_>>,
) -> Result<(), SaveError> {
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

    let relocation_paths = relocation.map(|moving| (moving.from.clone(), moving.to.clone()));
    let relocation_keys = relocation_paths.as_ref().map(|(from, to)| {
        let from_key = lock_path(from);
        let to_key = lock_path(to);
        (from_key, to_key)
    });
    if let (Some((_, to)), Some((from_key, to_key))) =
        (relocation_paths.as_ref(), relocation_keys.as_ref())
        && resolved
            .iter()
            .any(|(path, _)| path == from_key || path == to_key)
    {
        return Err(SaveError::Conflict { path: to.clone() });
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
    if let Some((_, to)) = &relocation_paths
        && !to.exists()
        && let Some(dir) = to.parent()
    {
        std::fs::create_dir_all(dir).map_err(|source| SaveError::Io {
            path: to.clone(),
            source,
        })?;
    }

    let _guards = acquire_batch_locks(
        &resolved,
        relocation_keys.as_ref(),
        publication_guard.as_ref().map(|guard| guard.target),
    )?;
    for (path, item) in &resolved {
        check_replaceable(path, &item.expected)?;
    }
    if let (Some(moving), Some((from, to))) = (relocation, relocation_paths.as_ref()) {
        check_replaceable(from, &moving.expected)?;
        if from != to {
            check_replaceable(to, "")?;
        }
    }
    if let Some(guard) = &mut publication_guard
        && !(guard.snapshot_is_current)()
    {
        return Err(SaveError::Changed {
            path: guard.target.to_path_buf(),
        });
    }
    // Staged under a guard: a failure part-way through leaves temporary files
    // beside their targets. Repository loaders keep the writer's exact hidden
    // staging names outside their data set, but litter that survives every run
    // is what the single-file path was careful about, and a batch has more of
    // it to lose (`A1-234`, audit).
    let mut pending = Staged::default();
    for (path, item) in &resolved {
        pending.files.push((
            path.clone(),
            stage_for(path, &item.updated, None, private_new)?,
        ));
    }
    let staged_move =
        if let (Some(moving), Some((from, to))) = (relocation, relocation_paths.as_ref()) {
            Some((
                from.clone(),
                to.clone(),
                stage_with_mode(to, &moving.updated, Some(from))?,
            ))
        } else {
            None
        };

    publish(
        pending,
        staged_move,
        relocation_paths.as_ref(),
        publication_guard.is_some(),
    )
}

fn acquire_batch_locks(
    resolved: &[(PathBuf, &Replacement)],
    relocation_keys: Option<&(PathBuf, PathBuf)>,
    publication_guard: Option<&Path>,
) -> Result<Vec<Lock>, SaveError> {
    // A guarded directory snapshot is the common lock for every destination
    // in that namespace. Taking per-file locks as well would publish those
    // implementation files into the very membership snapshot being checked,
    // and distinct capture ids do not need a second lock once every capture
    // writer holds the same outer one.
    let mut targets: Vec<PathBuf> = if publication_guard.is_some() {
        Vec::new()
    } else {
        resolved.iter().map(|(path, _)| path.clone()).collect()
    };
    if let Some((from, to)) = relocation_keys {
        targets.push(from.clone());
        targets.push(to.clone());
    }
    if let Some(target) = publication_guard {
        targets.push(lock_path(target));
    }
    targets.sort();
    targets.dedup();
    targets
        .into_iter()
        .map(|path| Lock::acquire(&path))
        .collect()
}

/// Publish staged replacements, recovering the relocation if a later rename
/// fails.
fn publish(
    pending: Staged,
    staged_move: Option<(PathBuf, PathBuf, PathBuf)>,
    relocation_paths: Option<&(PathBuf, PathBuf)>,
    recover_batch: bool,
) -> Result<(), SaveError> {
    publish_with_backup(
        pending,
        staged_move,
        relocation_paths,
        recover_batch,
        move_to_backup,
    )
}

/// The publish transaction with its one fallible backup operation exposed so a
/// test can force the late failure that ordinary pre-write checks prevent.
fn publish_with_backup(
    mut pending: Staged,
    staged_move: Option<(PathBuf, PathBuf, PathBuf)>,
    relocation_paths: Option<&(PathBuf, PathBuf)>,
    recover_batch: bool,
    mut backup_file: impl FnMut(&Path) -> Result<PathBuf, SaveError>,
) -> Result<(), SaveError> {
    // Make the old task path inert before publishing the new one. If publishing
    // fails, restore the old path; no sibling replacement has been committed at
    // this point. The hidden backup has no `.md` suffix, so even a process that
    // reads between the two renames cannot load two copies of the task.
    let mut backup = None;
    if let Some((from, to, temp)) = staged_move {
        let backup_path = match backup_file(&from) {
            Ok(path) => path,
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                return Err(error);
            }
        };
        if let Err(source) = rename_replacing(&temp, &to) {
            let _ = rename_replacing(&backup_path, &from);
            let _ = std::fs::remove_file(&temp);
            return Err(SaveError::Io { path: to, source });
        }
        backup = Some(backup_path);
    }
    if let (Some((from, to)), Some(source_backup)) = (relocation_paths.as_ref(), backup.as_ref()) {
        let mut committed: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        while let Some((path, temp)) = pending.files.last().cloned() {
            let replacement_backup = if path.exists() {
                let backup_path = match backup_file(&path) {
                    Ok(path) => path,
                    Err(error) => {
                        // Keep the current staging file in `pending` until its
                        // original path is recoverable. Its Drop then removes
                        // both this file and every not-yet-attempted sibling on
                        // the late failure (`A1-413`).
                        rollback_relocation(&committed, from, to, source_backup);
                        return Err(error);
                    }
                };
                Some(backup_path)
            } else {
                None
            };
            let _ = pending.files.pop();
            if let Err(source) = rename_replacing(&temp, &path) {
                let _ = std::fs::remove_file(&temp);
                committed.push((path.clone(), replacement_backup));
                rollback_relocation(&committed, from, to, source_backup);
                return Err(SaveError::Io { path, source });
            }
            committed.push((path, replacement_backup));
        }
        for (_, backup) in committed {
            if let Some(path) = backup {
                let _ = std::fs::remove_file(path);
            }
        }
        let _ = std::fs::remove_file(source_backup);
    } else if recover_batch {
        let mut committed: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        while let Some((path, temp)) = pending.files.last().cloned() {
            let replacement_backup = if path.exists() {
                match backup_file(&path) {
                    Ok(path) => Some(path),
                    Err(error) => {
                        rollback_batch(&committed);
                        return Err(error);
                    }
                }
            } else {
                None
            };
            let _ = pending.files.pop();
            if let Err(source) = rename_replacing(&temp, &path) {
                let _ = std::fs::remove_file(&temp);
                committed.push((path.clone(), replacement_backup));
                rollback_batch(&committed);
                return Err(SaveError::Io { path, source });
            }
            committed.push((path, replacement_backup));
        }
        for (_, backup) in committed {
            if let Some(path) = backup {
                let _ = std::fs::remove_file(path);
            }
        }
    } else {
        while let Some((path, temp)) = pending.files.pop() {
            rename_replacing(&temp, &path).map_err(|source| {
                let _ = std::fs::remove_file(&temp);
                SaveError::Io { path, source }
            })?;
        }
    }
    Ok(())
}

/// Restore every already-published member of a recoverable batch.
fn rollback_batch(committed: &[(PathBuf, Option<PathBuf>)]) {
    for (path, backup) in committed.iter().rev() {
        let _ = std::fs::remove_file(path);
        if let Some(backup) = backup {
            let _ = rename_replacing(backup, path);
        }
    }
}

/// Best-effort rollback after a relocation has published its destination.
///
/// The only way here is a filesystem failure during the rename loop. Restoring
/// in reverse order gives each target its original directory entry before the
/// source identity is put back. A filesystem that refuses the rollback as well
/// is beyond an all-or-none promise; every ordinary contention and permission
/// refusal happened before this point.
fn rollback_relocation(
    committed: &[(PathBuf, Option<PathBuf>)],
    from: &Path,
    to: &Path,
    source_backup: &Path,
) {
    for (path, backup) in committed.iter().rev() {
        let _ = std::fs::remove_file(path);
        if let Some(backup) = backup {
            let _ = rename_replacing(backup, path);
        }
    }
    let _ = std::fs::remove_file(to);
    let _ = rename_replacing(source_backup, from);
}

/// The path whose lock every spelling of `path` agrees on, including a target
/// that does not exist yet below a symlinked directory.
fn lock_path(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    path.parent()
        .and_then(|parent| std::fs::canonicalize(parent).ok())
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| path.to_path_buf())
}

/// Move an existing target to an unused hidden path beside it.
///
/// `create_temp` reserves a collision-free name by creating it. Remove that
/// placeholder before the rename: Windows does not let `rename` replace an
/// existing destination, while Unix does. The target's lock is held by the
/// caller, and no repository writer targets the process-specific hidden name.
fn move_to_backup(path: &Path) -> Result<PathBuf, SaveError> {
    let (backup, handle) = create_temp(path)?;
    drop(handle);
    std::fs::remove_file(&backup).map_err(|source| SaveError::Io {
        path: backup.clone(),
        source,
    })?;
    rename_replacing(path, &backup).map_err(|source| SaveError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(backup)
}

/// Atomically move one staged file over its destination.
///
/// Windows can temporarily refuse an otherwise valid replacement while a
/// concurrent reader still has the destination open. The caller already holds
/// the writer lock, so a short bounded retry preserves the same atomic rename
/// boundary without weakening compare-and-swap or treating a durable
/// permission failure as contention.
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        const ATTEMPTS: usize = 128;

        let parent = to
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut replacement = tempfile::Builder::new()
            .prefix(".aplan-replace-")
            .tempfile_in(parent)?;
        let mut staged = std::fs::File::open(from)?;
        std::io::copy(&mut staged, replacement.as_file_mut())?;
        replacement.as_file_mut().sync_all()?;
        drop(staged);
        std::fs::remove_file(from)?;

        for attempt in 0..ATTEMPTS {
            match replacement.persist(to) {
                Ok(file) => {
                    drop(file);
                    return Ok(());
                }
                Err(error)
                    if (error.error.kind() == std::io::ErrorKind::PermissionDenied
                        || matches!(error.error.raw_os_error(), Some(32 | 33)))
                        && attempt + 1 < ATTEMPTS =>
                {
                    replacement = error.file;
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(error) => return Err(error.error),
            }
        }
        unreachable!("the bounded Windows rename loop returns")
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
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
#[cfg(test)]
fn stage(path: &Path, updated: &str) -> Result<PathBuf, SaveError> {
    stage_with_mode(path, updated, None)
}

fn stage_with_mode(
    path: &Path,
    updated: &str,
    mode_from: Option<&Path>,
) -> Result<PathBuf, SaveError> {
    stage_for(path, updated, mode_from, false)
}

fn stage_for(
    path: &Path,
    updated: &str,
    mode_from: Option<&Path>,
    private_new: bool,
) -> Result<PathBuf, SaveError> {
    // The replacement is a new file, so it is created with the default mode
    // rather than the target's. Carry the target's across, or a milestone an
    // author restricted to themselves becomes world-readable the first time
    // anything is added to its cut.
    let mode = std::fs::metadata(mode_from.unwrap_or(path))
        .map(|m| m.permissions())
        .ok();
    #[cfg(not(unix))]
    let _ = private_new;

    let (temp, mut handle) = create_temp(path)?;
    #[cfg(unix)]
    if private_new && mode.is_none() {
        use std::os::unix::fs::PermissionsExt as _;
        if let Err(source) = handle.set_permissions(std::fs::Permissions::from_mode(0o600)) {
            drop(handle);
            let _ = std::fs::remove_file(&temp);
            return Err(SaveError::Io { path: temp, source });
        }
    }
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
}

impl Lock {
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
                    return Ok(Self { path });
                }
                Err(error) if existing_lock_contention(&path, &error) => {}
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

fn existing_lock_contention(path: &Path, error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return true;
    }
    #[cfg(windows)]
    {
        // Windows can report ERROR_ACCESS_DENIED rather than
        // ERROR_FILE_EXISTS when CREATE_NEW races an existing lock file. Only
        // classify it as contention after independently observing that exact
        // lock path; a genuine directory permission failure remains I/O.
        error.kind() == std::io::ErrorKind::PermissionDenied
            && std::fs::symlink_metadata(path).is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
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

    /// A relocation is the filename move and every referencing edit together.
    #[test]
    fn a_relocation_moves_the_source_and_replaces_its_siblings_together() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let old = dir.path().join("OLD.md");
        let new = dir.path().join("NEW.md");
        let sibling = dir.path().join("cut.md");
        std::fs::write(&old, "id: OLD\n").expect("old");
        std::fs::write(&sibling, "blocked_by: [OLD]\n").expect("sibling");

        replace_all_relocating(
            &[Replacement {
                path: sibling.clone(),
                expected: "blocked_by: [OLD]\n".into(),
                updated: "blocked_by: [NEW]\n".into(),
            }],
            &Relocation {
                from: old.clone(),
                to: new.clone(),
                expected: "id: OLD\n".into(),
                updated: "id: NEW\n".into(),
            },
        )
        .expect("relocate");

        assert!(!old.exists(), "the old identity remained loadable");
        assert_eq!(std::fs::read_to_string(new).expect("new"), "id: NEW\n");
        assert_eq!(
            std::fs::read_to_string(sibling).expect("sibling"),
            "blocked_by: [NEW]\n"
        );
        assert_eq!(
            std::fs::read_dir(dir.path())
                .expect("directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
                .count(),
            0,
            "a successful relocation left a hidden staging or backup file"
        );
    }

    /// A custom task filename stays put but still uses the same recovering
    /// transaction as a conventional filename move.
    #[test]
    fn an_in_place_relocation_replaces_the_source_and_its_siblings_together() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let custom = dir.path().join("custom-name.md");
        let sibling = dir.path().join("cut.md");
        std::fs::write(&custom, "id: OLD\n").expect("custom");
        std::fs::write(&sibling, "blocked_by: [OLD]\n").expect("sibling");

        replace_all_relocating(
            &[Replacement {
                path: sibling.clone(),
                expected: "blocked_by: [OLD]\n".into(),
                updated: "blocked_by: [NEW]\n".into(),
            }],
            &Relocation {
                from: custom.clone(),
                to: custom.clone(),
                expected: "id: OLD\n".into(),
                updated: "id: NEW\n".into(),
            },
        )
        .expect("in-place relocation");

        assert_eq!(
            std::fs::read_to_string(custom).expect("custom"),
            "id: NEW\n"
        );
        assert_eq!(
            std::fs::read_to_string(sibling).expect("sibling"),
            "blocked_by: [NEW]\n"
        );
    }

    /// Every ordinary contention case is found before the source is moved.
    #[test]
    fn a_refused_relocation_leaves_the_source_destination_and_siblings_unchanged() {
        for stale in ["source", "sibling", "destination"] {
            let dir = tempfile::Builder::new()
                .tempdir_in(crate::fixture::fixture_root())
                .expect("tempdir");
            let old = dir.path().join("OLD.md");
            let new = dir.path().join("NEW.md");
            let sibling = dir.path().join("cut.md");
            std::fs::write(&old, "id: OLD\n").expect("old");
            std::fs::write(&sibling, "blocked_by: [OLD]\n").expect("sibling");
            if stale == "destination" {
                std::fs::write(&new, "somebody else's file\n").expect("destination");
            }

            let result = replace_all_relocating(
                &[Replacement {
                    path: sibling.clone(),
                    expected: if stale == "sibling" {
                        "stale\n".into()
                    } else {
                        "blocked_by: [OLD]\n".into()
                    },
                    updated: "blocked_by: [NEW]\n".into(),
                }],
                &Relocation {
                    from: old.clone(),
                    to: new.clone(),
                    expected: if stale == "source" {
                        "stale\n".into()
                    } else {
                        "id: OLD\n".into()
                    },
                    updated: "id: NEW\n".into(),
                },
            );
            assert!(result.is_err(), "{stale} did not refuse");
            assert_eq!(std::fs::read_to_string(&old).expect("old"), "id: OLD\n");
            assert_eq!(
                std::fs::read_to_string(&sibling).expect("sibling"),
                "blocked_by: [OLD]\n"
            );
            if stale == "destination" {
                assert_eq!(
                    std::fs::read_to_string(&new).expect("destination"),
                    "somebody else's file\n"
                );
            } else {
                assert!(!new.exists(), "a refused move created its destination");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_relocation_preserves_the_source_mode_at_the_destination() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let old = dir.path().join("OLD.md");
        let new = dir.path().join("NEW.md");
        std::fs::write(&old, "old\n").expect("old");
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o600)).expect("mode");

        replace_all_relocating(
            &[],
            &Relocation {
                from: old,
                to: new.clone(),
                expected: "old\n".into(),
                updated: "new\n".into(),
            },
        )
        .expect("relocate");
        assert_eq!(
            std::fs::metadata(new)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn relocation_rollback_restores_replaced_created_and_source_files() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let from = dir.path().join("OLD.md");
        let to = dir.path().join("NEW.md");
        let source_backup = dir.path().join(".OLD.md.backup");
        let replaced = dir.path().join("cut.md");
        let replaced_backup = dir.path().join(".cut.md.backup");
        let created = dir.path().join("created.md");
        std::fs::write(&to, "id: NEW\n").expect("published destination");
        std::fs::write(&source_backup, "id: OLD\n").expect("source backup");
        std::fs::write(&replaced, "new cut\n").expect("published replacement");
        std::fs::write(&replaced_backup, "old cut\n").expect("replacement backup");
        std::fs::write(&created, "new file\n").expect("published creation");

        rollback_relocation(
            &[
                (replaced.clone(), Some(replaced_backup)),
                (created.clone(), None),
            ],
            &from,
            &to,
            &source_backup,
        );

        assert_eq!(std::fs::read_to_string(from).expect("source"), "id: OLD\n");
        assert_eq!(
            std::fs::read_to_string(replaced).expect("replacement"),
            "old cut\n"
        );
        assert!(!to.exists(), "the published destination survived rollback");
        assert!(
            !created.exists(),
            "a newly-created sibling survived rollback"
        );
    }

    /// Proof for behaviour `rekey.relocation_late_failure_cleans_staging`: the
    /// current sibling stays under the staging guard until its backup exists.
    #[test]
    fn a_late_relocation_failure_restores_every_file_and_removes_staging() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let from = dir.path().join("OLD.md");
        let to = dir.path().join("NEW.md");
        let sibling = dir.path().join("cut.md");
        let created = dir.path().join("created.md");
        std::fs::write(&from, "id: OLD\n").expect("source");
        std::fs::write(&sibling, "blocked_by: [OLD]\n").expect("sibling");
        let staged_sibling = stage(&sibling, "blocked_by: [NEW]\n").expect("staged sibling");
        let staged_created = stage(&created, "new sibling\n").expect("staged creation");
        let staged_move = stage_with_mode(&to, "id: NEW\n", Some(&from)).expect("staged move");
        let pending = Staged {
            // Publication pops from the end: commit the creation first, then
            // fail while making the existing sibling recoverable.
            files: vec![
                (sibling.clone(), staged_sibling.clone()),
                (created.clone(), staged_created.clone()),
            ],
        };
        let mut backup_calls = 0;

        let result = publish_with_backup(
            pending,
            Some((from.clone(), to.clone(), staged_move)),
            Some(&(from.clone(), to.clone())),
            false,
            |path| {
                backup_calls += 1;
                if backup_calls == 2 {
                    Err(SaveError::Io {
                        path: path.to_path_buf(),
                        source: std::io::Error::other("forced sibling backup failure"),
                    })
                } else {
                    move_to_backup(path)
                }
            },
        );

        assert!(matches!(result, Err(SaveError::Io { path, .. }) if path == sibling));
        assert_eq!(std::fs::read_to_string(&from).expect("source"), "id: OLD\n");
        assert_eq!(
            std::fs::read_to_string(&sibling).expect("sibling"),
            "blocked_by: [OLD]\n"
        );
        assert!(!to.exists(), "the published destination survived rollback");
        assert!(!created.exists(), "a published creation survived rollback");
        assert!(
            !staged_created.exists(),
            "the creation's staging path survived publication"
        );
        assert!(
            !staged_sibling.exists(),
            "the failing sibling's staging file survived the error"
        );
        assert_eq!(
            std::fs::read_dir(dir.path())
                .expect("directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
                .count(),
            0,
            "a hidden staging or backup file survived rollback"
        );
    }

    #[test]
    fn guarded_batch_late_failure_rolls_back_replacements_and_creations() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let capture = dir.path().join("capture.yml");
        let task = dir.path().join("task.md");
        let cut = dir.path().join("cut.md");
        std::fs::write(&capture, "state: open\n").expect("capture");
        std::fs::write(&cut, "blocked_by: [A1-000]\n").expect("cut");
        let staged_capture = stage(&capture, "state: promoted\n").expect("staged capture");
        let staged_task = stage(&task, "id: A1-001\n").expect("staged task");
        let staged_cut = stage(&cut, "blocked_by: [A1-000, A1-001]\n").expect("staged cut");
        let pending = Staged {
            // Publication pops from the end: replace the cut, create the task,
            // then fail before replacing the capture that would link to it.
            files: vec![
                (capture.clone(), staged_capture.clone()),
                (task.clone(), staged_task.clone()),
                (cut.clone(), staged_cut.clone()),
            ],
        };

        let mut backup_calls = 0;
        let result = publish_with_backup(pending, None, None, true, |path| {
            backup_calls += 1;
            if backup_calls == 2 {
                Err(SaveError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("injected between guarded replacements"),
                })
            } else {
                move_to_backup(path)
            }
        });

        assert!(
            matches!(&result, Err(SaveError::Io { path, .. }) if path == &capture),
            "{result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&capture).expect("restored capture"),
            "state: open\n"
        );
        assert_eq!(
            std::fs::read_to_string(&cut).expect("restored cut"),
            "blocked_by: [A1-000]\n"
        );
        assert!(!task.exists(), "the task creation survived rollback");
        assert!(
            !staged_capture.exists(),
            "capture staging survived rollback"
        );
        assert!(!staged_task.exists(), "task staging survived rollback");
        assert!(!staged_cut.exists(), "cut staging survived rollback");
    }

    #[test]
    fn guarded_batch_rename_failure_restores_the_current_replacement() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let capture = dir.path().join("capture.yml");
        std::fs::write(&capture, "state: open\n").expect("capture");
        let missing_stage = dir.path().join("missing-stage");
        let pending = Staged {
            files: vec![(capture.clone(), missing_stage)],
        };

        let result = publish_with_backup(pending, None, None, true, move_to_backup);

        assert!(
            matches!(&result, Err(SaveError::Io { path, .. }) if path == &capture),
            "{result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&capture).expect("restored capture"),
            "state: open\n"
        );
        assert_eq!(
            std::fs::read_dir(dir.path())
                .expect("directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
                .count(),
            0,
            "a backup survived rollback"
        );
    }

    #[test]
    fn a_relocation_refuses_a_batch_that_also_names_either_identity() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let old = dir.path().join("OLD.md");
        let new = dir.path().join("NEW.md");
        std::fs::write(&old, "id: OLD\n").expect("old");

        let result = replace_all_relocating(
            &[Replacement {
                path: new.clone(),
                expected: String::new(),
                updated: "competing answer\n".into(),
            }],
            &Relocation {
                from: old.clone(),
                to: new.clone(),
                expected: "id: OLD\n".into(),
                updated: "id: NEW\n".into(),
            },
        );

        assert!(matches!(result, Err(SaveError::Conflict { path }) if path == new));
        assert_eq!(std::fs::read_to_string(old).expect("old"), "id: OLD\n");
        assert!(
            !new.exists(),
            "a conflicted transaction wrote its destination"
        );
    }

    /// **A stale member refuses the complete batch before publication.**
    ///
    /// One file having moved under the caller must leave every other file
    /// untouched, or `mv` is back to the half-applied move this exists to end
    /// (`A1-234`).
    #[test]
    fn a_stale_batch_member_is_refused_before_any_file_is_written() {
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
        let message = err.to_string();
        assert!(message.contains(&a.display().to_string()), "{message}");
        assert!(message.contains("two different replacements"), "{message}");
        assert!(message.contains("Nothing was written"), "{message}");
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
    #[cfg(unix)]
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
    #[cfg(unix)]
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

        let message = err.to_string();
        assert!(message.contains(&taken.display().to_string()), "{message}");
        assert!(message.contains("already exists"), "{message}");
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
