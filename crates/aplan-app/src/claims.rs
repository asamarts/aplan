//! Private, worktree-shared persistence for ephemeral claims (`A1-531`).
//!
//! The store lives below Git's common administrative directory, so linked
//! worktrees contend on one file while task Markdown remains durable plan data.
//! It is local coordination, not a distributed lock or an identity boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aplan_core::{ClaimOrigin, ClaimRecord, ClaimScope, ClaimSnapshot, Plan};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const SCHEMA: &str = "aplan/claims/v1";
const STATE_DIR: &str = "aplan";
const STATE_FILE: &str = "claims-v1.json";
const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CAS_RETRIES: usize = 64;
const MAX_READ_RETRIES: usize = 64;

/// A claim-store failure that must not be mistaken for an empty store.
#[derive(Debug, thiserror::Error)]
pub enum ClaimError {
    #[error("cannot use local claim coordination for `{root}`: {reason}")]
    Unavailable { root: PathBuf, reason: String },
    #[error("unsafe local claim state at `{path}`: {reason}")]
    Unsafe { path: PathBuf, reason: String },
    #[error("invalid local claim state at `{path}`: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error(
        "local claim state names task(s) this worktree cannot see: {ids}. Claims were not \
         treated as free. Switch to a worktree containing each task or run `aplan release \
         <id>` for every obsolete claim"
    )]
    Orphaned { ids: String },
    #[error("writing local claim state: {0}")]
    Write(#[from] aplan_core::save::SaveError),
    #[error("reading local claim state at `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "local claim for `{id}` changed after it was reviewed; reload before confirming the action"
    )]
    Stale { path: PathBuf, id: String },
    #[doc(hidden)]
    #[error("local claim state at `{path}` changed while it was being inspected: {reason}")]
    ReadRace { path: PathBuf, reason: String },
}

/// Result of recording or taking over one claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimChange {
    pub current: ClaimRecord,
    pub displaced: Option<ClaimRecord>,
    pub warnings: Vec<String>,
    pub scope: ClaimScope,
}

/// Result of releasing a claim. `released == None` is an idempotent no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseChange {
    pub released: Option<ClaimRecord>,
    pub scope: ClaimScope,
}

/// Compute the exact claim record and displaced value without writing them.
///
/// The returned value is suitable for review and for [`commit_claim`]. The
/// latter compares the complete displaced record, not only its holder label,
/// so a same-holder reclaim between preview and confirmation is stale.
pub(crate) fn preview_claim(
    root: &Path,
    plan: &Plan,
    id: &str,
    identity: &aplan_interop::identity::ResolvedIdentity,
    from: Option<&str>,
    claimed_at_epoch_seconds: i64,
) -> Result<ClaimChange, ClaimError> {
    validate_claim_request(root, plan, id, from, claimed_at_epoch_seconds)?;
    validate_identity(root, identity)?;
    let location = location(root)?.ok_or_else(|| ClaimError::Unavailable {
        root: root.to_path_buf(),
        reason:
            "the plan is not inside a Git repository; there is no scope shared by linked worktrees"
                .to_string(),
    })?;
    let loaded = load_state(&location)?;
    refuse_orphans(&loaded.state, plan)?;
    let displaced = loaded.state.claims.get(id).cloned();
    validate_takeover(&location.file, id, displaced.as_ref(), from)?;
    let current = ClaimRecord {
        holder: identity.name.clone(),
        identity_source: identity.source.as_str().to_string(),
        claimed_at_epoch_seconds,
        origin: location.origin,
    };
    validate_record(&location.file, id, &current)?;
    Ok(ClaimChange {
        warnings: claim_warnings(&loaded.state, identity, &current),
        current,
        displaced,
        scope: location.scope,
    })
}

/// Publish one exact reviewed claim if its complete prior record is unchanged.
pub(crate) fn commit_claim(
    root: &Path,
    plan: &Plan,
    id: &str,
    preview: &ClaimChange,
) -> Result<ClaimChange, ClaimError> {
    validate_claim_request(
        root,
        plan,
        id,
        preview
            .displaced
            .as_ref()
            .map(|record| record.holder.as_str()),
        preview.current.claimed_at_epoch_seconds,
    )?;
    let location = location(root)?.ok_or_else(|| ClaimError::Unavailable {
        root: root.to_path_buf(),
        reason:
            "the plan is not inside a Git repository; there is no scope shared by linked worktrees"
                .to_string(),
    })?;
    if location.scope != preview.scope || location.origin != preview.current.origin {
        return Err(stale(&location.file, id));
    }
    validate_record(&location.file, id, &preview.current)?;
    prepare_directory(&location)?;
    for attempt in 0..MAX_CAS_RETRIES {
        let mut loaded = load_state(&location)?;
        refuse_orphans(&loaded.state, plan)?;
        if loaded.state.claims.get(id) != preview.displaced.as_ref() {
            return Err(stale(&location.file, id));
        }
        loaded
            .state
            .claims
            .insert(id.to_string(), preview.current.clone());
        match write_state(&location, &loaded) {
            Ok(()) => return Ok(preview.clone()),
            Err(ClaimError::Write(
                aplan_core::save::SaveError::Changed { .. }
                | aplan_core::save::SaveError::Exists { .. }
                | aplan_core::save::SaveError::Busy { .. },
            )) if attempt + 1 < MAX_CAS_RETRIES => std::thread::yield_now(),
            Err(error) => return Err(error),
        }
    }
    Err(stale(&location.file, id))
}

/// Observe the exact release without modifying the shared claim store.
pub(crate) fn preview_release(root: &Path, id: &str) -> Result<ReleaseChange, ClaimError> {
    if let Some(reason) = invalid_text(id, 256) {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!("the requested task id {reason}"),
        });
    }
    let location = location(root)?.ok_or_else(|| ClaimError::Unavailable {
        root: root.to_path_buf(),
        reason: "the plan is not inside a Git repository; no local claim store exists".to_string(),
    })?;
    let loaded = load_state(&location)?;
    Ok(ReleaseChange {
        released: loaded.state.claims.get(id).cloned(),
        scope: location.scope,
    })
}

/// Publish one exact reviewed release if its complete record is unchanged.
pub(crate) fn commit_release(
    root: &Path,
    id: &str,
    preview: &ReleaseChange,
) -> Result<ReleaseChange, ClaimError> {
    if let Some(reason) = invalid_text(id, 256) {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!("the requested task id {reason}"),
        });
    }
    let location = location(root)?.ok_or_else(|| ClaimError::Unavailable {
        root: root.to_path_buf(),
        reason: "the plan is not inside a Git repository; no local claim store exists".to_string(),
    })?;
    if location.scope != preview.scope {
        return Err(stale(&location.file, id));
    }
    for attempt in 0..MAX_CAS_RETRIES {
        let mut loaded = load_state(&location)?;
        if loaded.state.claims.get(id) != preview.released.as_ref() {
            return Err(stale(&location.file, id));
        }
        if preview.released.is_none() {
            return Ok(preview.clone());
        }
        loaded.state.claims.remove(id);
        match write_state(&location, &loaded) {
            Ok(()) => return Ok(preview.clone()),
            Err(ClaimError::Write(
                aplan_core::save::SaveError::Changed { .. }
                | aplan_core::save::SaveError::Busy { .. },
            )) if attempt + 1 < MAX_CAS_RETRIES => std::thread::yield_now(),
            Err(error) => return Err(error),
        }
    }
    Err(stale(&location.file, id))
}

#[derive(Debug)]
struct Location {
    directory: PathBuf,
    file: PathBuf,
    scope: ClaimScope,
    origin: ClaimOrigin,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskState {
    schema: String,
    claims: BTreeMap<String, ClaimRecord>,
}

impl Default for DiskState {
    fn default() -> Self {
        Self {
            schema: SCHEMA.to_string(),
            claims: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
struct LoadedState {
    state: DiskState,
    original: String,
}

/// Current signed Unix seconds, saturating to zero before the epoch.
#[must_use]
pub fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Observe one coherent claim snapshot and reject records invisible here.
///
/// No Git repository is a supported degraded state, represented in the value.
/// Corrupt, unsafe or orphaned state is an error: returning an empty map for any
/// of those would make held work look free.
pub fn snapshot(root: &Path, plan: &Plan) -> Result<ClaimSnapshot, ClaimError> {
    let observed_at = now_epoch_seconds();
    let Some(location) = location(root)? else {
        return Ok(ClaimSnapshot::unavailable(
            observed_at,
            "the plan is not inside a Git repository; claims cannot be coordinated across actors",
        ));
    };
    let loaded = load_state(&location)?;
    refuse_orphans(&loaded.state, plan)?;
    Ok(ClaimSnapshot {
        observed_at_epoch_seconds: observed_at,
        scope: location.scope,
        claims: loaded.state.claims,
    })
}

/// Record one claim against the shared common-directory state.
pub fn claim(
    root: &Path,
    plan: &Plan,
    id: &str,
    identity: &aplan_interop::identity::ResolvedIdentity,
    from: Option<&str>,
    claimed_at_epoch_seconds: i64,
) -> Result<ClaimChange, ClaimError> {
    validate_claim_request(root, plan, id, from, claimed_at_epoch_seconds)?;
    validate_identity(root, identity)?;
    let location = location(root)?.ok_or_else(|| ClaimError::Unavailable {
        root: root.to_path_buf(),
        reason:
            "the plan is not inside a Git repository; there is no scope shared by linked worktrees"
                .to_string(),
    })?;
    prepare_directory(&location)?;
    let current = ClaimRecord {
        holder: identity.name.clone(),
        identity_source: identity.source.as_str().to_string(),
        claimed_at_epoch_seconds,
        origin: location.origin.clone(),
    };
    validate_record(&location.file, id, &current)?;
    for attempt in 0..MAX_CAS_RETRIES {
        let mut loaded = load_state(&location)?;
        refuse_orphans(&loaded.state, plan)?;
        let displaced = loaded.state.claims.get(id).cloned();
        validate_takeover(&location.file, id, displaced.as_ref(), from)?;

        let warnings = claim_warnings(&loaded.state, identity, &current);
        loaded.state.claims.insert(id.to_string(), current.clone());
        match write_state(&location, &loaded) {
            Ok(()) => {
                return Ok(ClaimChange {
                    current,
                    displaced,
                    warnings,
                    scope: location.scope,
                });
            }
            Err(ClaimError::Write(
                aplan_core::save::SaveError::Changed { .. }
                | aplan_core::save::SaveError::Exists { .. }
                | aplan_core::save::SaveError::Busy { .. },
            )) if attempt + 1 < MAX_CAS_RETRIES => {
                std::thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
    Err(ClaimError::Invalid {
        path: location.file,
        reason: format!(
            "claim state changed {MAX_CAS_RETRIES} times while recording `{id}`; nothing was overwritten"
        ),
    })
}

fn claim_warnings(
    state: &DiskState,
    identity: &aplan_interop::identity::ResolvedIdentity,
    current: &ClaimRecord,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(warning) = aplan_interop::identity::placeholder_warning(identity) {
        warnings.push(warning);
    }
    let other_origins: BTreeSet<String> = state
        .claims
        .iter()
        .filter(|(_, record)| record.holder == identity.name && record.origin != current.origin)
        .map(|(_, record)| describe_origin(&record.origin))
        .collect();
    if !other_origins.is_empty() {
        warnings.push(format!(
            "identity label `{}` already holds claim(s) from {}. If those are separate actors, \
             give each a distinct `--as`, `APLAN_OWNER`, or per-user `me` label",
            identity.name,
            other_origins.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    warnings
}

fn stale(path: &Path, id: &str) -> ClaimError {
    ClaimError::Stale {
        path: path.to_path_buf(),
        id: id.to_string(),
    }
}

fn validate_identity(
    root: &Path,
    identity: &aplan_interop::identity::ResolvedIdentity,
) -> Result<(), ClaimError> {
    if let Some(reason) = invalid_text(&identity.name, 64) {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!("the claim holder {reason}"),
        });
    }
    Ok(())
}

fn validate_claim_request(
    root: &Path,
    plan: &Plan,
    id: &str,
    from: Option<&str>,
    claimed_at_epoch_seconds: i64,
) -> Result<(), ClaimError> {
    if let Some(reason) = invalid_text(id, 256) {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!("the requested task id {reason}"),
        });
    }
    if let Some(named) = from
        && let Some(reason) = invalid_text(named, 64)
    {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!("the `--from` holder {reason}"),
        });
    }
    let mut matching = plan.tasks.iter().filter(|loaded| loaded.task.id == id);
    let Some(target) = matching.next() else {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!("no task `{id}` exists in this plan"),
        });
    };
    if matching.next().is_some() {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!("more than one task declares `{id}`; refusing to guess which it is"),
        });
    }
    if target.task.status.is_terminal() {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!(
                "cannot claim `{id}` because it is `{}`; claims name work happening now",
                target.task.status
            ),
        });
    }
    if claimed_at_epoch_seconds < 0 {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: "a claim time cannot precede the Unix epoch".to_string(),
        });
    }
    Ok(())
}

fn validate_takeover(
    path: &Path,
    id: &str,
    displaced: Option<&ClaimRecord>,
    from: Option<&str>,
) -> Result<(), ClaimError> {
    let reason = match (displaced, from) {
        (Some(record), None) => Some(format!(
            "`{id}` is held by `{}`; name that holder with `--from` to record a takeover",
            record.holder
        )),
        (Some(record), Some(named)) if record.holder != named => Some(format!(
            "`{id}` is held by `{}`, not `{named}`; the displaced record changed",
            record.holder
        )),
        (None, Some(named)) => Some(format!(
            "`{id}` is held by nobody, so there is nothing to take from `{named}`"
        )),
        _ => None,
    };
    reason.map_or(Ok(()), |reason| {
        Err(ClaimError::Invalid {
            path: path.to_path_buf(),
            reason,
        })
    })
}

/// Release a claim, including one whose task is absent from this worktree.
///
/// This is the manual recovery path for deletion, rekeying and branch switches.
/// It deliberately does not reject other orphaned records: a caller must be
/// able to clear them one at a time without first making the snapshot healthy.
pub fn release(root: &Path, id: &str) -> Result<ReleaseChange, ClaimError> {
    if let Some(reason) = invalid_text(id, 256) {
        return Err(ClaimError::Invalid {
            path: root.to_path_buf(),
            reason: format!("the requested task id {reason}"),
        });
    }
    let location = location(root)?.ok_or_else(|| ClaimError::Unavailable {
        root: root.to_path_buf(),
        reason: "the plan is not inside a Git repository; no local claim store exists".to_string(),
    })?;
    for attempt in 0..MAX_CAS_RETRIES {
        let mut loaded = load_state(&location)?;
        let released = loaded.state.claims.remove(id);
        if released.is_none() {
            return Ok(ReleaseChange {
                released,
                scope: location.scope,
            });
        }
        match write_state(&location, &loaded) {
            Ok(()) => {
                return Ok(ReleaseChange {
                    released,
                    scope: location.scope,
                });
            }
            Err(ClaimError::Write(
                aplan_core::save::SaveError::Changed { .. }
                | aplan_core::save::SaveError::Busy { .. },
            )) if attempt + 1 < MAX_CAS_RETRIES => {
                std::thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
    Err(ClaimError::Invalid {
        path: location.file,
        reason: format!(
            "claim state changed {MAX_CAS_RETRIES} times while releasing `{id}`; nothing was overwritten"
        ),
    })
}

fn location(root: &Path) -> Result<Option<Location>, ClaimError> {
    let repo = match aplan_evidence::Repo::discover(root) {
        Ok(repo) => repo,
        Err(aplan_evidence::Error::NotARepository(_)) => return Ok(None),
        Err(error) => {
            return Err(ClaimError::Unavailable {
                root: root.to_path_buf(),
                reason: error.to_string(),
            });
        }
    };
    let common = std::fs::canonicalize(repo.common_dir()).map_err(|source| ClaimError::Io {
        path: repo.common_dir().to_path_buf(),
        source,
    })?;
    let worktree = repo
        .workdir()
        .map(|path| {
            std::fs::canonicalize(&path).map_err(|source| ClaimError::Io {
                path: path.clone(),
                source,
            })
        })
        .transpose()?;
    let mut digest = Sha256::new();
    digest.update(common.as_os_str().as_encoded_bytes());
    let scope_id = format!("sha256:{}", hex(&digest.finalize()));
    let directory = common.join(STATE_DIR);
    Ok(Some(Location {
        file: directory.join(STATE_FILE),
        directory,
        scope: ClaimScope::git_common_directory(scope_id),
        origin: ClaimOrigin {
            host: host_label(),
            worktree: worktree.map(|path| path.display().to_string()),
        },
    }))
}

fn host_label() -> Option<String> {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find_map(|value| valid_host_label(&value))
}

fn valid_host_label(value: &str) -> Option<String> {
    let value = value.trim().to_string();
    if !aplan_core::task::is_vacuous_ref(&value)
        && value.chars().count() <= 255
        && !value.chars().any(char::is_control)
    {
        Some(value)
    } else {
        None
    }
}

fn prepare_directory(location: &Location) -> Result<(), ClaimError> {
    match location.directory.symlink_metadata() {
        Ok(_) => validate_directory(&location.directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match create_private_directory(&location.directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(ClaimError::Io {
                        path: location.directory.clone(),
                        source,
                    });
                }
            }
            validate_directory(&location.directory)
        }
        Err(source) => Err(ClaimError::Io {
            path: location.directory.clone(),
            source,
        }),
    }
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path)
}

fn validate_directory(path: &Path) -> Result<(), ClaimError> {
    let metadata = path.symlink_metadata().map_err(|source| ClaimError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ClaimError::Unsafe {
            path: path.to_path_buf(),
            reason:
                "the state directory must be a real directory, not a symlink or another file type"
                    .to_string(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ClaimError::Unsafe {
                path: path.to_path_buf(),
                reason: "the directory is accessible by group or other users; require mode 0700"
                    .to_string(),
            });
        }
    }
    Ok(())
}

fn load_state(location: &Location) -> Result<LoadedState, ClaimError> {
    let mut last_race = None;
    for _ in 0..MAX_READ_RETRIES {
        match load_state_once(location) {
            Err(error @ ClaimError::ReadRace { .. }) => {
                last_race = Some(error);
                std::thread::yield_now();
            }
            outcome => return outcome,
        }
    }
    let ClaimError::ReadRace { path, reason } =
        last_race.expect("a retry loop exits only after observing a read race")
    else {
        unreachable!("only read races are retained")
    };
    Err(ClaimError::Unsafe {
        path,
        reason: format!(
            "the state did not become stable after {MAX_READ_RETRIES} coherent-read attempts: \
             {reason}"
        ),
    })
}

fn load_state_once(location: &Location) -> Result<LoadedState, ClaimError> {
    match location.directory.symlink_metadata() {
        Ok(_) => validate_directory(&location.directory)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadedState {
                state: DiskState::default(),
                original: String::new(),
            });
        }
        Err(source) => {
            return Err(ClaimError::Io {
                path: location.directory.clone(),
                source,
            });
        }
    }
    let metadata = match location.file.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadedState {
                state: DiskState::default(),
                original: String::new(),
            });
        }
        Err(source) => {
            return Err(ClaimError::Io {
                path: location.file.clone(),
                source,
            });
        }
    };
    validate_file(&location.file, &metadata)?;
    if metadata.len() > MAX_STATE_BYTES {
        return Err(ClaimError::Invalid {
            path: location.file.clone(),
            reason: format!(
                "the file is {} bytes, above the {MAX_STATE_BYTES}-byte safety bound",
                metadata.len()
            ),
        });
    }
    let file = std::fs::File::open(&location.file).map_err(|source| ClaimError::Io {
        path: location.file.clone(),
        source,
    })?;
    let opened = file.metadata().map_err(|source| ClaimError::Io {
        path: location.file.clone(),
        source,
    })?;
    validate_file(&location.file, &opened)?;
    refuse_changed_identity(&location.file, &metadata, &opened)?;
    let mut original = String::new();
    file.take(MAX_STATE_BYTES + 1)
        .read_to_string(&mut original)
        .map_err(|source| ClaimError::Io {
            path: location.file.clone(),
            source,
        })?;
    if u64::try_from(original.len()).unwrap_or(u64::MAX) > MAX_STATE_BYTES {
        return Err(ClaimError::Invalid {
            path: location.file.clone(),
            reason: format!("the file grew above the {MAX_STATE_BYTES}-byte safety bound"),
        });
    }
    let after = location
        .file
        .symlink_metadata()
        .map_err(|source| ClaimError::Io {
            path: location.file.clone(),
            source,
        })?;
    validate_file(&location.file, &after)?;
    refuse_changed_identity(&location.file, &opened, &after)?;
    let state: DiskState =
        serde_json::from_str(&original).map_err(|error| ClaimError::Invalid {
            path: location.file.clone(),
            reason: error.to_string(),
        })?;
    validate_state(&location.file, &state)?;
    Ok(LoadedState { state, original })
}

fn validate_file(path: &Path, metadata: &std::fs::Metadata) -> Result<(), ClaimError> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ClaimError::Unsafe {
            path: path.to_path_buf(),
            reason:
                "the state path must be a real regular file, not a symlink or another file type"
                    .to_string(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ClaimError::Unsafe {
                path: path.to_path_buf(),
                reason: "the file is accessible by group or other users; require mode 0600"
                    .to_string(),
            });
        }
        if metadata.nlink() != 1 {
            return Err(ClaimError::ReadRace {
                path: path.to_path_buf(),
                reason: "the state file was observed with more than one hard link".to_string(),
            });
        }
    }
    Ok(())
}

#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))] // Keep one fallible identity interface.
fn refuse_changed_identity(
    path: &Path,
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
) -> Result<(), ClaimError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(ClaimError::ReadRace {
                path: path.to_path_buf(),
                reason: "the state file was replaced while it was being inspected".to_string(),
            });
        }
    }
    #[cfg(not(unix))]
    let _ = (path, before, after);
    Ok(())
}

fn validate_state(path: &Path, state: &DiskState) -> Result<(), ClaimError> {
    if state.schema != SCHEMA {
        return Err(ClaimError::Invalid {
            path: path.to_path_buf(),
            reason: format!(
                "schema is `{}`, expected `{SCHEMA}`",
                diagnostic_text(&state.schema)
            ),
        });
    }
    for (id, record) in &state.claims {
        validate_record(path, id, record)?;
    }
    Ok(())
}

fn validate_record(path: &Path, id: &str, record: &ClaimRecord) -> Result<(), ClaimError> {
    let invalid = |reason: String| {
        Err(ClaimError::Invalid {
            path: path.to_path_buf(),
            reason,
        })
    };
    if let Some(reason) = invalid_text(id, 256) {
        return invalid(format!("a claim key {reason}"));
    }
    if let Some(reason) = invalid_text(&record.holder, 64) {
        return invalid(format!("claim `{id}` has a holder that {reason}"));
    }
    if !matches!(
        record.identity_source.as_str(),
        "argument" | "environment" | "user_config" | "git" | "operating_system"
    ) {
        return invalid(format!(
            "claim `{id}` has unknown identity source `{}`",
            diagnostic_text(&record.identity_source)
        ));
    }
    if let Some(host) = &record.origin.host
        && let Some(reason) = invalid_text(host, 255)
    {
        return invalid(format!("claim `{id}` has a host label that {reason}"));
    }
    if let Some(worktree) = &record.origin.worktree
        && let Some(reason) = invalid_text(worktree, 4096)
    {
        return invalid(format!("claim `{id}` has a worktree path that {reason}"));
    }
    if record.claimed_at_epoch_seconds < 0 {
        return invalid(format!("claim `{id}` has a time before the Unix epoch"));
    }
    Ok(())
}

fn invalid_text(value: &str, max_chars: usize) -> Option<String> {
    if aplan_core::task::is_vacuous_ref(value) {
        Some("is blank".to_string())
    } else if value.chars().count() > max_chars {
        Some(format!("is longer than {max_chars} characters"))
    } else if value.chars().any(char::is_control) {
        Some("contains a control character".to_string())
    } else {
        None
    }
}

fn diagnostic_text(value: &str) -> String {
    aplan_output::sanitize::visible(value)
        .chars()
        .map(|character| {
            if character.is_control() {
                aplan_output::sanitize::MARKER
            } else {
                character
            }
        })
        .collect()
}

fn refuse_orphans(state: &DiskState, plan: &Plan) -> Result<(), ClaimError> {
    let known: BTreeSet<&str> = plan
        .tasks
        .iter()
        .map(|loaded| loaded.task.id.as_str())
        .collect();
    let orphans: Vec<&str> = state
        .claims
        .keys()
        .map(String::as_str)
        .filter(|id| !known.contains(id))
        .collect();
    if orphans.is_empty() {
        Ok(())
    } else {
        Err(ClaimError::Orphaned {
            ids: orphans
                .into_iter()
                .map(|id| format!("`{id}`"))
                .collect::<Vec<_>>()
                .join(", "),
        })
    }
}

fn write_state(location: &Location, loaded: &LoadedState) -> Result<(), ClaimError> {
    validate_state(&location.file, &loaded.state)?;
    let mut updated =
        serde_json::to_string_pretty(&loaded.state).map_err(|error| ClaimError::Invalid {
            path: location.file.clone(),
            reason: error.to_string(),
        })?;
    updated.push('\n');
    if u64::try_from(updated.len()).unwrap_or(u64::MAX) > MAX_STATE_BYTES {
        return Err(ClaimError::Invalid {
            path: location.file.clone(),
            reason: format!(
                "the candidate file is {} bytes, above the {MAX_STATE_BYTES}-byte safety bound; \
                 the previous state was left untouched",
                updated.len()
            ),
        });
    }
    aplan_core::save::replace_private(&location.file, &loaded.original, &updated)?;
    validate_published_file(&location.file)
}

fn validate_published_file(path: &Path) -> Result<(), ClaimError> {
    let mut last_race = None;
    for _ in 0..MAX_READ_RETRIES {
        let metadata = path.symlink_metadata().map_err(|source| ClaimError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        match validate_file(path, &metadata) {
            Err(error @ ClaimError::ReadRace { .. }) => {
                last_race = Some(error);
                std::thread::yield_now();
            }
            outcome => return outcome,
        }
    }
    let ClaimError::ReadRace {
        path: raced_path,
        reason,
    } = last_race.expect("a retry loop exits only after observing a read race")
    else {
        unreachable!("only read races are retained")
    };
    Err(ClaimError::Unsafe {
        path: raced_path,
        reason: format!(
            "the published state did not become stable after {MAX_READ_RETRIES} inspections: \
             {reason}"
        ),
    })
}

fn describe_origin(origin: &ClaimOrigin) -> String {
    match (&origin.host, &origin.worktree) {
        (Some(host), Some(worktree)) => format!("host `{host}` worktree `{worktree}`"),
        (Some(host), None) => format!("host `{host}`"),
        (None, Some(worktree)) => format!("worktree `{worktree}`"),
        (None, None) => "an unknown local origin".to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let status = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(dir.path())
            .status()
            .expect("git init");
        assert!(status.success());
        std::fs::create_dir_all(dir.path().join("tasks")).expect("tasks");
        std::fs::write(
            dir.path().join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: one\nphase: 0\nstatus: todo\neffort_days: 1\nowner: team\n---\n\nbody\n",
        )
        .expect("task");
        dir
    }

    fn identity(name: &str) -> aplan_interop::identity::ResolvedIdentity {
        aplan_interop::identity::ResolvedIdentity {
            name: name.to_string(),
            source: aplan_interop::identity::IdentitySource::Argument,
        }
    }

    fn record() -> ClaimRecord {
        ClaimRecord {
            holder: "ada".to_string(),
            identity_source: "argument".to_string(),
            claimed_at_epoch_seconds: 42,
            origin: ClaimOrigin {
                host: Some("host".to_string()),
                worktree: Some("/worktree".to_string()),
            },
        }
    }

    #[test]
    fn claim_state_is_private_ephemeral_and_does_not_touch_the_task() {
        let dir = fixture();
        let plan = aplan_core::load::load_plan(dir.path());
        let task = dir.path().join("tasks/T-1.md");
        let before = std::fs::read(&task).expect("task bytes");
        let change = claim(dir.path(), &plan, "T-1", &identity("ada"), None, 42).expect("claim");
        assert_eq!(change.current.holder, "ada");
        assert_eq!(change.current.claimed_at_epoch_seconds, 42);
        assert_eq!(std::fs::read(&task).expect("task bytes"), before);
        let observed = snapshot(dir.path(), &plan).expect("snapshot");
        assert_eq!(
            observed.claim("T-1").map(|record| record.holder.as_str()),
            Some("ada")
        );
        assert!(observed.scope.available);
        assert_eq!(
            release(dir.path(), "T-1").expect("release").released,
            Some(change.current)
        );
        assert!(
            snapshot(dir.path(), &plan)
                .expect("snapshot")
                .claims
                .is_empty()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let directory = dir.path().join(".git/aplan");
            assert_eq!(
                directory
                    .metadata()
                    .expect("directory metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                directory
                    .join("claims-v1.json")
                    .metadata()
                    .expect("file metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn reviewed_claim_and_release_bound_scope_record_and_arguments() {
        let dir = fixture();
        let plan = aplan_core::load::load_plan(dir.path());

        let proposed =
            preview_claim(dir.path(), &plan, "T-1", &identity("ada"), None, 42).expect("preview");
        assert!(proposed.displaced.is_none());
        assert_eq!(proposed.current.holder, "ada");

        let mut wrong_scope = proposed.clone();
        wrong_scope.scope.id = Some("another repository".to_string());
        assert!(matches!(
            commit_claim(dir.path(), &plan, "T-1", &wrong_scope),
            Err(ClaimError::Stale { .. })
        ));
        assert!(
            snapshot(dir.path(), &plan)
                .expect("state")
                .claims
                .is_empty()
        );

        let mut wrong_origin = proposed.clone();
        wrong_origin.current.origin.host = Some("another host".to_string());
        assert!(matches!(
            commit_claim(dir.path(), &plan, "T-1", &wrong_origin),
            Err(ClaimError::Stale { .. })
        ));

        let mut forged = proposed.clone();
        forged.current.identity_source = "invented".to_string();
        assert!(matches!(
            commit_claim(dir.path(), &plan, "T-1", &forged),
            Err(ClaimError::Invalid { .. })
        ));
        commit_claim(dir.path(), &plan, "T-1", &proposed).expect("publish claim");
        assert!(matches!(
            commit_claim(dir.path(), &plan, "T-1", &proposed),
            Err(ClaimError::Stale { .. })
        ));

        let release = preview_release(dir.path(), "T-1").expect("release preview");
        let mut wrong_scope = release.clone();
        wrong_scope.scope.id = Some("another repository".to_string());
        assert!(matches!(
            commit_release(dir.path(), "T-1", &wrong_scope),
            Err(ClaimError::Stale { .. })
        ));
        commit_release(dir.path(), "T-1", &release).expect("publish release");
        let empty = preview_release(dir.path(), "T-1").expect("empty preview");
        commit_release(dir.path(), "T-1", &empty).expect("idempotent release");

        for id in ["", "\u{200b}", "x\n"] {
            assert!(matches!(
                preview_release(dir.path(), id),
                Err(ClaimError::Invalid { .. })
            ));
            assert!(matches!(
                commit_release(dir.path(), id, &empty),
                Err(ClaimError::Invalid { .. })
            ));
        }

        let outside = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("non-repository");
        assert!(matches!(
            preview_release(outside.path(), "T-1"),
            Err(ClaimError::Unavailable { .. })
        ));
        assert!(matches!(
            commit_release(outside.path(), "T-1", &empty),
            Err(ClaimError::Unavailable { .. })
        ));
    }

    /// Proof for `release.recovers_orphaned_local_claims`.
    #[test]
    fn corrupt_and_orphaned_state_never_looks_free_but_orphans_can_be_released() {
        let dir = fixture();
        let plan = aplan_core::load::load_plan(dir.path());
        claim(dir.path(), &plan, "T-1", &identity("ada"), None, 42).expect("claim");
        let state = dir.path().join(".git/aplan/claims-v1.json");
        let valid = std::fs::read_to_string(&state).expect("state");
        std::fs::write(&state, "{ not JSON\n").expect("corrupt");
        assert!(matches!(
            snapshot(dir.path(), &plan),
            Err(ClaimError::Invalid { .. })
        ));
        std::fs::write(&state, valid).expect("restore");

        std::fs::remove_file(dir.path().join("tasks/T-1.md")).expect("delete task");
        let without = aplan_core::load::load_plan(dir.path());
        assert!(matches!(
            snapshot(dir.path(), &without),
            Err(ClaimError::Orphaned { .. })
        ));
        let cleared = release(dir.path(), "T-1").expect("orphan recovery");
        assert_eq!(
            cleared.released.map(|record| record.holder),
            Some("ada".to_string())
        );
        assert!(
            snapshot(dir.path(), &without)
                .expect("healthy empty state")
                .claims
                .is_empty()
        );
    }

    #[test]
    fn blank_and_overlong_claim_values_are_corruption_not_coordination() {
        let dir = fixture();
        let plan = aplan_core::load::load_plan(dir.path());
        claim(dir.path(), &plan, "T-1", &identity("ada"), None, 42).expect("claim");
        let state = dir.path().join(".git/aplan/claims-v1.json");
        let valid = std::fs::read_to_string(&state).expect("state");

        let mut blank_holder: serde_json::Value =
            serde_json::from_str(&valid).expect("valid state JSON");
        blank_holder["claims"]["T-1"]["holder"] = serde_json::json!("\u{200b}");
        let mut blank_host = blank_holder.clone();
        blank_host["claims"]["T-1"]["holder"] = serde_json::json!("ada");
        blank_host["claims"]["T-1"]["origin"]["host"] = serde_json::json!("");
        let mut overlong_worktree = blank_host.clone();
        overlong_worktree["claims"]["T-1"]["origin"]["host"] = serde_json::json!("host");
        overlong_worktree["claims"]["T-1"]["origin"]["worktree"] =
            serde_json::json!("x".repeat(4097));

        for invalid in [blank_holder, blank_host, overlong_worktree] {
            std::fs::write(
                &state,
                serde_json::to_string_pretty(&invalid).expect("invalid state JSON"),
            )
            .expect("invalid state");
            assert!(
                matches!(snapshot(dir.path(), &plan), Err(ClaimError::Invalid { .. })),
                "a blank or overlong value was accepted"
            );
        }
    }

    #[test]
    fn every_untrusted_record_field_is_validated_at_its_boundary() {
        let path = Path::new("claims-v1.json");

        let invalid_schema = DiskState {
            schema: format!("wrong{}schema", '\u{1b}'),
            claims: BTreeMap::new(),
        };
        let error = validate_state(path, &invalid_schema).expect_err("unknown schema");
        assert!(!error.to_string().contains('\u{1b}'), "{error}");
        assert!(error.to_string().contains("aplan/claims/v1"));

        for (id, expected) in [
            ("\u{200b}".to_string(), "claim key is blank"),
            ("x".repeat(257), "longer than 256"),
            (format!("T{}1", '\u{7}'), "control character"),
        ] {
            let error = validate_record(path, &id, &record()).expect_err("invalid task id");
            assert!(error.to_string().contains(expected), "{error}");
        }

        let mut invalid_records = Vec::new();
        let mut blank_holder = record();
        blank_holder.holder = " ".to_string();
        invalid_records.push((blank_holder, "holder that is blank"));
        let mut long_holder = record();
        long_holder.holder = "x".repeat(65);
        invalid_records.push((long_holder, "holder that is longer than 64"));
        let mut unknown_source = record();
        unknown_source.identity_source = format!("bad{}source", '\u{1b}');
        invalid_records.push((unknown_source, "unknown identity source"));
        let mut blank_host = record();
        blank_host.origin.host = Some("\u{200b}".to_string());
        invalid_records.push((blank_host, "host label that is blank"));
        let mut long_host = record();
        long_host.origin.host = Some("x".repeat(256));
        invalid_records.push((long_host, "host label that is longer than 255"));
        let mut blank_worktree = record();
        blank_worktree.origin.worktree = Some("\t".to_string());
        invalid_records.push((blank_worktree, "worktree path that is blank"));
        let mut long_worktree = record();
        long_worktree.origin.worktree = Some("x".repeat(4097));
        invalid_records.push((long_worktree, "worktree path that is longer than 4096"));
        let mut negative_time = record();
        negative_time.claimed_at_epoch_seconds = -1;
        invalid_records.push((negative_time, "time before the Unix epoch"));

        for (invalid, expected) in invalid_records {
            let error = validate_record(path, "T-1", &invalid).expect_err("invalid record");
            assert!(error.to_string().contains(expected), "{error}");
            assert!(!error.to_string().contains('\u{1b}'), "{error}");
        }

        for source in [
            "argument",
            "environment",
            "user_config",
            "git",
            "operating_system",
        ] {
            let mut valid = record();
            valid.identity_source = source.to_string();
            validate_record(path, "T-1", &valid).expect("known identity source");
        }
    }

    #[test]
    fn origin_descriptions_preserve_every_available_dimension() {
        let origin = |host: Option<&str>, worktree: Option<&str>| ClaimOrigin {
            host: host.map(str::to_string),
            worktree: worktree.map(str::to_string),
        };
        assert_eq!(
            describe_origin(&origin(Some("h"), Some("w"))),
            "host `h` worktree `w`"
        );
        assert_eq!(describe_origin(&origin(Some("h"), None)), "host `h`");
        assert_eq!(describe_origin(&origin(None, Some("w"))), "worktree `w`");
        assert_eq!(
            describe_origin(&origin(None, None)),
            "an unknown local origin"
        );
    }

    /// Proof for `claim.origin_fallback_skips_invalid_host_values`.
    #[test]
    fn host_label_selection_skips_invalid_candidates_before_falling_back() {
        assert_eq!(
            ["\u{200b}".to_string(), "  workstation  ".to_string()]
                .into_iter()
                .find_map(|value| valid_host_label(&value)),
            Some("workstation".to_string())
        );
        assert_eq!(
            ["x".repeat(256), format!("bad{}host", '\u{7}')]
                .into_iter()
                .find_map(|value| valid_host_label(&value)),
            None
        );
    }

    #[test]
    fn takeover_validation_names_the_exact_record_transition() {
        let path = Path::new("claims-v1.json");
        let held = record();
        for (displaced, from, expected) in [
            (Some(&held), None, "name that holder with `--from`"),
            (Some(&held), Some("grace"), "not `grace`"),
            (None, Some("ada"), "held by nobody"),
        ] {
            let error = validate_takeover(path, "T-1", displaced, from)
                .expect_err("invalid takeover transition");
            assert!(error.to_string().contains(expected), "{error}");
        }
        validate_takeover(path, "T-1", Some(&held), Some("ada")).expect("matching takeover");
        validate_takeover(path, "T-1", None, None).expect("new claim");
    }

    #[test]
    fn placeholder_and_cross_origin_identity_collisions_are_actionable() {
        let dir = fixture();
        for id in ["T-2", "T-3"] {
            std::fs::write(
                dir.path().join(format!("tasks/{id}.md")),
                format!(
                    "---\nid: {id}\ntitle: {id}\nphase: 0\nstatus: todo\neffort_days: 1\n---\n"
                ),
            )
            .expect("task");
        }
        let plan = aplan_core::load::load_plan(dir.path());
        claim(dir.path(), &plan, "T-1", &identity("ada"), None, 42).expect("first claim");

        let state = dir.path().join(".git/aplan/claims-v1.json");
        let mut stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&state).expect("state"))
                .expect("state JSON");
        stored["claims"]["T-1"]["origin"] = serde_json::json!({
            "host": "another-host"
        });
        std::fs::write(
            &state,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&stored).expect("state JSON")
            ),
        )
        .expect("move claim origin");

        let collision = claim(dir.path(), &plan, "T-2", &identity("ada"), None, 43)
            .expect("same label from another origin");
        assert_eq!(collision.warnings.len(), 1);
        assert!(collision.warnings[0].contains("another-host"));
        assert!(collision.warnings[0].contains("distinct `--as`"));

        let placeholder = claim(dir.path(), &plan, "T-3", &identity("test"), None, 44)
            .expect("placeholder is warned, not silently accepted");
        assert_eq!(placeholder.warnings.len(), 1);
        assert!(placeholder.warnings[0].contains("placeholder"));
        assert!(placeholder.warnings[0].contains("APLAN_OWNER"));
    }

    #[test]
    fn a_non_git_plan_reports_degraded_reads_and_refuses_claim_mutation() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("tasks")).expect("tasks");
        std::fs::write(
            dir.path().join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: one\nphase: 0\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("task");
        let plan = aplan_core::load::load_plan(dir.path());
        let observed = snapshot(dir.path(), &plan).expect("degraded snapshot");
        assert!(!observed.scope.available);
        assert!(matches!(
            claim(dir.path(), &plan, "T-1", &identity("ada"), None, 42),
            Err(ClaimError::Unavailable { .. })
        ));
        assert!(matches!(
            release(dir.path(), "T-1"),
            Err(ClaimError::Unavailable { .. })
        ));
    }

    #[test]
    fn direct_claim_callers_cannot_create_unknown_terminal_or_invalid_time_records() {
        let dir = fixture();
        let plan = aplan_core::load::load_plan(dir.path());
        for (id, time, expected) in [("T-missing", 42, "no task"), ("T-1", -1, "Unix epoch")] {
            let error = claim(dir.path(), &plan, id, &identity("ada"), None, time)
                .expect_err("invalid claim");
            assert!(error.to_string().contains(expected), "{error}");
        }

        let vacuous = aplan_interop::identity::ResolvedIdentity {
            name: "\u{200b}".to_string(),
            source: aplan_interop::identity::IdentitySource::Argument,
        };
        let error = claim(dir.path(), &plan, "T-1", &vacuous, None, 42)
            .expect_err("vacuous direct identity");
        assert!(error.to_string().contains("holder"), "{error}");
        assert!(
            !dir.path().join(".git/aplan").exists(),
            "an invalid request must not create coordination state"
        );

        std::fs::write(
            dir.path().join("tasks/T-1.md"),
            "---\nid: T-1\ntitle: one\nphase: 0\nstatus: done\neffort_days: 1\n---\n",
        )
        .expect("terminal task");
        let terminal = aplan_core::load::load_plan(dir.path());
        let error = claim(dir.path(), &terminal, "T-1", &identity("ada"), None, 42)
            .expect_err("terminal claim");
        assert!(error.to_string().contains("done"), "{error}");
        assert!(!dir.path().join(".git/aplan/claims-v1.json").exists());
    }

    #[test]
    fn duplicate_task_ids_and_unheld_releases_are_never_guessed() {
        let dir = fixture();
        std::fs::write(
            dir.path().join("tasks/T-duplicate.md"),
            "---\nid: T-1\ntitle: duplicate\nphase: 0\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("duplicate task");
        let duplicate = aplan_core::load::load_plan(dir.path());
        let error = claim(dir.path(), &duplicate, "T-1", &identity("ada"), None, 42)
            .expect_err("ambiguous task");
        assert!(error.to_string().contains("more than one task"), "{error}");

        assert!(
            release(dir.path(), "T-1")
                .expect("idempotent release")
                .released
                .is_none()
        );
    }

    #[test]
    fn malformed_existing_store_shapes_and_bytes_fail_closed() {
        let plan_for = |dir: &tempfile::TempDir| aplan_core::load::load_plan(dir.path());

        let state_directory = fixture();
        let state_location = location(state_directory.path())
            .expect("location")
            .expect("Git scope");
        prepare_directory(&state_location).expect("state directory");
        std::fs::create_dir(&state_location.file).expect("directory at state path");
        assert!(matches!(
            snapshot(state_directory.path(), &plan_for(&state_directory)),
            Err(ClaimError::Unsafe { .. })
        ));

        let invalid_utf8 = fixture();
        let invalid_utf8_location = location(invalid_utf8.path())
            .expect("location")
            .expect("Git scope");
        prepare_directory(&invalid_utf8_location).expect("state directory");
        std::fs::write(&invalid_utf8_location.file, [0xff]).expect("invalid UTF-8 state");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(
                &invalid_utf8_location.file,
                std::fs::Permissions::from_mode(0o600),
            )
            .expect("private file");
        }
        assert!(matches!(
            snapshot(invalid_utf8.path(), &plan_for(&invalid_utf8)),
            Err(ClaimError::Io { .. })
        ));

        let oversized = fixture();
        let oversized_location = location(oversized.path())
            .expect("location")
            .expect("Git scope");
        prepare_directory(&oversized_location).expect("state directory");
        let file = std::fs::File::create(&oversized_location.file).expect("oversized state");
        file.set_len(MAX_STATE_BYTES + 1).expect("grow state");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(
                &oversized_location.file,
                std::fs::Permissions::from_mode(0o600),
            )
            .expect("private file");
        }
        let error = snapshot(oversized.path(), &plan_for(&oversized)).expect_err("oversized state");
        assert!(error.to_string().contains("safety bound"), "{error}");

        let bad_directory = fixture();
        let bad_directory_location = location(bad_directory.path())
            .expect("location")
            .expect("Git scope");
        std::fs::write(&bad_directory_location.directory, "not a directory")
            .expect("state directory impostor");
        assert!(matches!(
            snapshot(bad_directory.path(), &plan_for(&bad_directory)),
            Err(ClaimError::Unsafe { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn inode_replacement_and_published_hard_links_are_detected() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = fixture();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::write(&first, "first").expect("first");
        std::fs::write(&second, "second").expect("second");
        let error = refuse_changed_identity(
            &first,
            &first.metadata().expect("first metadata"),
            &second.metadata().expect("second metadata"),
        )
        .expect_err("different inode");
        assert!(matches!(error, ClaimError::ReadRace { .. }));

        std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o600))
            .expect("private first");
        let alias = dir.path().join("first-alias");
        std::fs::hard_link(&first, alias).expect("hard link");
        let error = validate_published_file(&first).expect_err("published hard link");
        assert!(matches!(error, ClaimError::Unsafe { .. }));
        assert!(
            error.to_string().contains("did not become stable"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_private_but_unreadable_or_unwritable_state_reports_io() {
        use std::os::unix::fs::PermissionsExt as _;

        let unreadable = fixture();
        let plan = aplan_core::load::load_plan(unreadable.path());
        claim(unreadable.path(), &plan, "T-1", &identity("ada"), None, 42).expect("seed state");
        let state = unreadable.path().join(".git/aplan/claims-v1.json");
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o000))
            .expect("unreadable state");
        assert!(matches!(
            snapshot(unreadable.path(), &plan),
            Err(ClaimError::Io { .. })
        ));

        let unwritable = fixture();
        let plan = aplan_core::load::load_plan(unwritable.path());
        claim(unwritable.path(), &plan, "T-1", &identity("ada"), None, 42).expect("seed state");
        let state_dir = unwritable.path().join(".git/aplan");
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o500))
            .expect("unwritable state directory");
        assert!(matches!(
            release(unwritable.path(), "T-1"),
            Err(ClaimError::Write(aplan_core::save::SaveError::Io { .. }))
        ));
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
            .expect("restore state directory");

        let claim_unwritable = fixture();
        std::fs::write(
            claim_unwritable.path().join("tasks/T-2.md"),
            "---\nid: T-2\ntitle: two\nphase: 0\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("second task");
        let plan = aplan_core::load::load_plan(claim_unwritable.path());
        claim(
            claim_unwritable.path(),
            &plan,
            "T-1",
            &identity("ada"),
            None,
            42,
        )
        .expect("seed claim state");
        let state_dir = claim_unwritable.path().join(".git/aplan");
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o500))
            .expect("unwritable state directory");
        assert!(matches!(
            claim(
                claim_unwritable.path(),
                &plan,
                "T-2",
                &identity("grace"),
                None,
                43
            ),
            Err(ClaimError::Write(aplan_core::save::SaveError::Io { .. }))
        ));
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
            .expect("restore state directory");
    }

    #[test]
    fn missing_store_paths_report_the_exact_failed_boundary() {
        let dir = fixture();
        let location = location(dir.path()).expect("location").expect("Git scope");
        let missing_directory = dir.path().join("missing-directory");
        let error = validate_directory(&missing_directory).expect_err("missing directory");
        assert!(matches!(error, ClaimError::Io { ref path, .. } if path == &missing_directory));

        let missing_file = dir.path().join("missing-file");
        let error = validate_published_file(&missing_file).expect_err("missing published file");
        assert!(matches!(error, ClaimError::Io { ref path, .. } if path == &missing_file));

        prepare_directory(&location).expect("private state directory");
        std::fs::remove_dir(&location.directory).expect("remove empty state directory");
        assert!(
            load_state_once(&location)
                .expect("an absent store is empty")
                .state
                .claims
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn inaccessible_git_metadata_reports_lookup_and_creation_boundaries() {
        use std::os::unix::fs::PermissionsExt as _;

        let creation = fixture();
        let creation_location = location(creation.path())
            .expect("location")
            .expect("Git scope");
        let git = creation.path().join(".git");
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o500))
            .expect("make Git directory unwritable");
        let error =
            prepare_directory(&creation_location).expect_err("state directory creation denied");
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o700))
            .expect("restore Git directory");
        assert!(
            matches!(error, ClaimError::Io { ref path, .. } if path == &creation_location.directory)
        );

        let lookup = fixture();
        let lookup_location = location(lookup.path())
            .expect("location")
            .expect("Git scope");
        let git = lookup.path().join(".git");
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o000))
            .expect("make Git directory inaccessible");
        let error = prepare_directory(&lookup_location).expect_err("state directory lookup denied");
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o700))
            .expect("restore Git directory");
        assert!(
            matches!(error, ClaimError::Io { ref path, .. } if path == &lookup_location.directory)
        );
    }

    #[test]
    fn direct_mutation_arguments_cannot_steer_diagnostics_or_bypass_record_bounds() {
        let dir = fixture();
        let plan = aplan_core::load::load_plan(dir.path());
        let escape = format!("T-{}[2J", '\u{1b}');
        let error = claim(dir.path(), &plan, &escape, &identity("ada"), None, 42)
            .expect_err("control-bearing id");
        assert!(!error.to_string().contains('\u{1b}'), "{error}");
        assert!(error.to_string().contains("task id contains a control"));

        let from = format!("ada{}[2J", '\u{1b}');
        let error = claim(
            dir.path(),
            &plan,
            "T-1",
            &identity("grace"),
            Some(&from),
            42,
        )
        .expect_err("control-bearing takeover label");
        assert!(!error.to_string().contains('\u{1b}'), "{error}");
        assert!(
            error
                .to_string()
                .contains("`--from` holder contains a control")
        );

        let error = release(dir.path(), &escape).expect_err("control-bearing release id");
        assert!(!error.to_string().contains('\u{1b}'), "{error}");
        assert!(error.to_string().contains("task id contains a control"));
        assert!(!dir.path().join(".git/aplan/claims-v1.json").exists());
    }

    #[test]
    fn a_write_cannot_create_state_too_large_to_read() {
        let dir = fixture();
        let location = location(dir.path()).expect("location").expect("Git scope");
        prepare_directory(&location).expect("private state directory");
        let mut state = DiskState::default();
        for index in 0..1_100 {
            state.claims.insert(
                format!("T-{index}"),
                ClaimRecord {
                    holder: "ada".to_string(),
                    identity_source: "argument".to_string(),
                    claimed_at_epoch_seconds: 42,
                    origin: ClaimOrigin {
                        host: Some("host".to_string()),
                        worktree: Some("x".repeat(4_096)),
                    },
                },
            );
        }
        let error = write_state(
            &location,
            &LoadedState {
                state,
                original: String::new(),
            },
        )
        .expect_err("oversized candidate");
        assert!(
            error
                .to_string()
                .contains("previous state was left untouched")
        );
        assert!(!location.file.exists());
    }

    /// Proof for `claim.concurrent_disjoint_updates_compose`.
    #[test]
    fn concurrent_claims_and_releases_on_distinct_tasks_all_compose() {
        let dir = fixture();
        for index in 0..24 {
            let id = format!("T-{index}");
            std::fs::write(
                dir.path().join(format!("tasks/{id}.md")),
                format!(
                    "---\nid: {id}\ntitle: task {index}\nphase: 0\nstatus: todo\n\
                     effort_days: 1\n---\n"
                ),
            )
            .expect("task");
        }
        // T-1 replaced the fixture's task above; one exact id still exists.
        let plan = std::sync::Arc::new(aplan_core::load::load_plan(dir.path()));
        let root = dir.path().to_path_buf();
        let claims = (0..24)
            .map(|index| {
                let root = root.clone();
                let plan = plan.clone();
                std::thread::spawn(move || {
                    let id = format!("T-{index}");
                    claim(
                        &root,
                        &plan,
                        &id,
                        &identity(&format!("actor-{index}")),
                        None,
                        42,
                    )
                    .map(|_| id)
                })
            })
            .collect::<Vec<_>>();
        let claimed = claims
            .into_iter()
            .map(|thread| thread.join().expect("claim thread"))
            .collect::<Result<Vec<_>, _>>()
            .expect("every disjoint claim composes");
        assert_eq!(claimed.len(), 24);
        assert_eq!(snapshot(&root, &plan).expect("snapshot").claims.len(), 24);

        let releases = claimed
            .into_iter()
            .map(|id| {
                let root = root.clone();
                std::thread::spawn(move || release(&root, &id))
            })
            .collect::<Vec<_>>();
        for thread in releases {
            assert!(
                thread
                    .join()
                    .expect("release thread")
                    .expect("disjoint release")
                    .released
                    .is_some()
            );
        }
        assert!(snapshot(&root, &plan).expect("snapshot").claims.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_overbroad_permissions_are_refused_not_repaired() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let dir = fixture();
        let plan = aplan_core::load::load_plan(dir.path());
        let state_dir = dir.path().join(".git/aplan");
        std::fs::create_dir(&state_dir).expect("state directory");
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o755))
            .expect("permissions");
        assert!(matches!(
            snapshot(dir.path(), &plan),
            Err(ClaimError::Unsafe { .. })
        ));
        assert_eq!(
            state_dir.metadata().expect("metadata").permissions().mode() & 0o777,
            0o755,
            "an existing unsafe directory was silently chmodded"
        );

        std::fs::remove_dir(&state_dir).expect("remove directory");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).expect("outside");
        symlink(&outside, &state_dir).expect("symlink");
        assert!(matches!(
            snapshot(dir.path(), &plan),
            Err(ClaimError::Unsafe { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_state_files_are_refused_not_followed_or_repaired() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let dir = fixture();
        let plan = aplan_core::load::load_plan(dir.path());
        let state_dir = dir.path().join(".git/aplan");
        std::fs::create_dir(&state_dir).expect("state directory");
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
            .expect("directory permissions");
        let state = state_dir.join("claims-v1.json");
        let outside = dir.path().join("outside-state");
        std::fs::write(&outside, "do not read\n").expect("outside state");
        symlink(&outside, &state).expect("state symlink");
        assert!(matches!(
            snapshot(dir.path(), &plan),
            Err(ClaimError::Unsafe { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(&outside).expect("outside"),
            "do not read\n"
        );

        std::fs::remove_file(&state).expect("remove symlink");
        claim(dir.path(), &plan, "T-1", &identity("ada"), None, 42).expect("claim");
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o644))
            .expect("broad permissions");
        assert!(matches!(
            snapshot(dir.path(), &plan),
            Err(ClaimError::Unsafe { .. })
        ));
        assert_eq!(
            state
                .metadata()
                .expect("state metadata")
                .permissions()
                .mode()
                & 0o777,
            0o644,
            "an unsafe existing file was silently chmodded"
        );

        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o600))
            .expect("restore permissions");
        let alias = dir.path().join("state-alias");
        std::fs::hard_link(&state, &alias).expect("hard link");
        assert!(matches!(
            snapshot(dir.path(), &plan),
            Err(ClaimError::Unsafe { .. })
        ));
    }
}
