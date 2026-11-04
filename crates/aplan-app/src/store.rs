//! Disposable persistent reuse for authoritative plan inputs.
//!
//! Markdown remains the source of truth. Every load enumerates the current
//! candidate set and validates each cached observation, reading and hashing a
//! candidate whenever its metadata cannot prove it predates the snapshot.
//! Cache failures are deliberately invisible to plan semantics.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aplan_core::diagnostic::{Diagnostic, Location, code};
use aplan_core::load::{
    Criteria, IdentifiedPlan, LoadStats, LoadedTask, ParsedCandidate, PlanFileContent,
    PlanFileInput, parse_plan_candidate, plan_candidate_paths,
};
use aplan_core::{Plan, Task};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    CancellationToken, OperationId, OperationKind, ProgressChildId, ProgressEmitter, ProgressEvent,
    ProgressMeasure, ProgressPhase, ProgressUnit,
};

const MAGIC: &str = "aplan-plan-cache";
const FORMAT_EPOCH: u32 = 4;
const SEMANTIC_EPOCH: u32 = 1;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CANDIDATES: usize = 100_000;
const MAX_PATH_BYTES: usize = 64 * 1024;
const MAX_REPORT_DIAGNOSTICS: usize = 1_000_000;
const MAX_REPORT_TEXT_BYTES: usize = 32 * 1024 * 1024;
const MAX_DIAGNOSTIC_TEXT_BYTES: usize = 1024 * 1024;
const MAX_CACHE_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MAX_CACHE_TOTAL: u64 = 256 * 1024 * 1024;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A source-authoritative snapshot could not stabilize while other writers
/// changed the plan's candidate-file set.
///
/// The store reports this as an [`io::Error`] because loading is an I/O
/// operation. Keeping a typed source inside that error lets a mutation retry
/// this safe pre-write condition without treating every `ErrorKind::Other` or
/// every error containing the word "retry" as contention.
#[derive(Debug, thiserror::Error)]
#[error("the plan's candidate set kept changing while it was read; retry the command")]
pub struct PlanSnapshotChanged;

/// Whether an I/O failure is the typed transient snapshot condition above.
#[must_use]
pub fn is_transient_plan_snapshot_change(error: &io::Error) -> bool {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<PlanSnapshotChanged>())
        .is_some()
}

/// Whether the disposable plan cache participates in a load.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheMode {
    /// Reuse an exact generation and publish a replacement after a miss.
    #[default]
    Auto,
    /// Read and write no persistent cache state.
    Off,
    /// Ignore the old generation, then publish the newly parsed state.
    Refresh,
}

impl CacheMode {
    pub const NAMES: &'static [&'static str] = &["auto", "off", "refresh"];

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "auto" => Some(Self::Auto),
            "off" => Some(Self::Off),
            "refresh" => Some(Self::Refresh),
            _ => None,
        }
    }
}

/// Host choices for one store.
#[derive(Debug, Clone)]
pub struct PlanStoreOptions {
    pub mode: CacheMode,
    /// Selected configuration path. Absence is part of the key.
    pub configuration_file: Option<PathBuf>,
    /// An explicit root for tests. Production callers leave this unset and use
    /// the operating system convention (or `APLAN_CACHE_DIR`).
    pub cache_root: Option<PathBuf>,
    /// Exact implementation identity for persisted parsed artifacts and
    /// semantic results. A host without one gets no persistent reuse.
    pub writer_identity: Option<String>,
    /// Retain authoritative candidate bytes for source/body projections.
    pub retain_inputs: bool,
    /// Coalesce an incremental artifact generation with the structural report
    /// that the caller is about to compute. If no report is computed, skipping
    /// publication only loses a future optimization.
    pub defer_publication: bool,
}

impl Default for PlanStoreOptions {
    fn default() -> Self {
        Self {
            mode: CacheMode::Auto,
            configuration_file: None,
            cache_root: None,
            writer_identity: None,
            retain_inputs: true,
            defer_publication: false,
        }
    }
}

/// Deterministic work counters for cache tests and timing diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanStoreStats {
    pub load: LoadStats,
    pub cache_generation_hit: bool,
    pub artifacts_reused: usize,
    pub artifacts_parsed: usize,
    pub cache_bytes_read: u64,
    pub cache_bytes_written: u64,
}

/// A loaded immutable plan snapshot and the store work that produced it.
#[derive(Debug)]
pub struct PlanStore {
    /// The root spelling supplied by the host. Domain paths preserve it so the
    /// cache cannot turn relative diagnostics into host-absolute output.
    root: PathBuf,
    /// Absolute, non-canonical identity used only for cache isolation.
    identity_root: PathBuf,
    options: PlanStoreOptions,
    snapshot: IdentifiedPlan,
    candidates: Vec<CachedCandidate>,
    reports: Vec<CachedReport>,
    reuse_horizon: TimeValue,
    stats: PlanStoreStats,
}

impl PlanStore {
    /// Load one source-authoritative snapshot, quietly treating every cache
    /// problem as a miss.
    pub fn open(root: &Path, options: &PlanStoreOptions) -> io::Result<Self> {
        Self::open_inner(root, options, None, None)
    }

    /// Load while reporting typed application progress.
    pub fn open_with_progress(
        root: &Path,
        options: &PlanStoreOptions,
        operation: OperationId,
        progress: &mut dyn FnMut(ProgressEvent),
    ) -> io::Result<Self> {
        let mut emitter = ProgressEmitter::begin(operation, OperationKind::PlanLoad, progress);
        let result = Self::open_inner(root, options, Some(&mut emitter), None);
        if result.is_ok() {
            emitter.complete();
        } else {
            emitter.fail();
        }
        result
    }

    /// Load as one phase of an operation whose lifecycle belongs to the
    /// caller. This is the seam used by interactive startup: configuration,
    /// source loading, repository observation and projection remain one
    /// generation instead of several superficially adjacent progress runs.
    pub fn open_observed(
        root: &Path,
        options: &PlanStoreOptions,
        emitter: &mut ProgressEmitter<'_>,
        cancellation: &CancellationToken,
    ) -> io::Result<Self> {
        Self::open_inner(root, options, Some(emitter), Some(cancellation))
    }

    fn open_inner(
        root: &Path,
        options: &PlanStoreOptions,
        mut progress: Option<&mut ProgressEmitter<'_>>,
        cancellation: Option<&CancellationToken>,
    ) -> io::Result<Self> {
        check_cancelled(cancellation)?;
        let root = root.to_path_buf();
        let identity_root = absolute(&root);
        let cache = CacheLocation::resolve(&identity_root, options);
        let mut stats = PlanStoreStats::default();
        if let Some(emitter) = progress.as_deref_mut() {
            emitter.phase_started(
                ProgressPhase::CacheLookup,
                None,
                ProgressMeasure::Indeterminate,
            );
        }
        let generation = if options.mode == CacheMode::Auto {
            cache
                .as_ref()
                .and_then(|location| read_generation(location, &identity_root, options, &mut stats))
        } else {
            None
        };
        if let Some(emitter) = progress.as_deref_mut() {
            emitter.phase_completed(
                ProgressPhase::CacheLookup,
                None,
                ProgressMeasure::Indeterminate,
            );
        }
        stats.cache_generation_hit = generation.is_some();
        let reports = if options.writer_identity.is_some() {
            generation
                .as_ref()
                .map_or_else(Vec::new, |generation| generation.reports.clone())
        } else {
            Vec::new()
        };
        let previous_horizon = generation.as_ref().map(|generation| generation.created_at);
        let previous = generation
            .map(|generation| {
                generation
                    .candidates
                    .into_iter()
                    .map(|candidate| (candidate.path.clone(), candidate))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        let (snapshot, candidates) = capture(
            &root,
            &previous,
            previous_horizon,
            options.retain_inputs,
            &mut stats,
            progress.as_deref_mut(),
            cancellation,
        )?;
        let reuse_horizon = TimeValue::now();
        if options.mode != CacheMode::Off
            && !options.defer_publication
            && (options.mode == CacheMode::Refresh
                || !stats.cache_generation_hit
                || stats.artifacts_parsed > 0)
            && let Some(location) = &cache
        {
            check_cancelled(cancellation)?;
            if let Some(emitter) = progress.as_deref_mut() {
                emitter.phase_started(
                    ProgressPhase::CachePublication,
                    None,
                    ProgressMeasure::Indeterminate,
                );
            }
            let generation = Generation::new(
                &identity_root,
                options,
                reuse_horizon,
                candidates.clone(),
                reports.clone(),
            );
            publish_generation(location, &generation, &mut stats);
            evict(&location.directory);
            if let Some(emitter) = progress {
                emitter.phase_completed(
                    ProgressPhase::CachePublication,
                    None,
                    ProgressMeasure::Indeterminate,
                );
            }
        }

        Ok(Self {
            root,
            identity_root,
            options: options.clone(),
            snapshot,
            candidates,
            reports,
            reuse_horizon,
            stats,
        })
    }

    /// Re-read authoritative source inputs while retaining this process's
    /// parsed artifacts. Long-lived MCP and TUI hosts use this instead of
    /// constructing an adapter-local cache.
    pub fn refresh(&mut self) -> io::Result<()> {
        self.refresh_inner(None, None)
    }

    /// Refresh while reporting typed application progress.
    pub fn refresh_with_progress(
        &mut self,
        operation: OperationId,
        progress: &mut dyn FnMut(ProgressEvent),
    ) -> io::Result<()> {
        let mut emitter = ProgressEmitter::begin(operation, OperationKind::PlanLoad, progress);
        let result = self.refresh_inner(Some(&mut emitter), None);
        if result.is_ok() {
            emitter.complete();
        } else {
            emitter.fail();
        }
        result
    }

    /// Refresh inside a caller-owned progress generation with cooperative
    /// cancellation between source observations.
    pub fn refresh_observed(
        &mut self,
        emitter: &mut ProgressEmitter<'_>,
        cancellation: &CancellationToken,
    ) -> io::Result<()> {
        self.refresh_inner(Some(emitter), Some(cancellation))
    }

    fn refresh_inner(
        &mut self,
        mut progress: Option<&mut ProgressEmitter<'_>>,
        cancellation: Option<&CancellationToken>,
    ) -> io::Result<()> {
        check_cancelled(cancellation)?;
        let mut stats = PlanStoreStats::default();
        let previous = if self.options.mode == CacheMode::Auto {
            stats.cache_generation_hit = true;
            self.candidates
                .iter()
                .cloned()
                .map(|candidate| (candidate.path.clone(), candidate))
                .collect()
        } else {
            BTreeMap::new()
        };
        let (snapshot, candidates) = capture(
            &self.root,
            &previous,
            Some(self.reuse_horizon),
            self.options.retain_inputs,
            &mut stats,
            progress.as_deref_mut(),
            cancellation,
        )?;
        let reuse_horizon = TimeValue::now();
        if self.options.mode != CacheMode::Off
            && !self.options.defer_publication
            && (self.options.mode == CacheMode::Refresh || stats.artifacts_parsed > 0)
            && let Some(location) = CacheLocation::resolve(&self.identity_root, &self.options)
        {
            check_cancelled(cancellation)?;
            if let Some(emitter) = progress.as_deref_mut() {
                emitter.phase_started(
                    ProgressPhase::CachePublication,
                    None,
                    ProgressMeasure::Indeterminate,
                );
            }
            let generation = Generation::new(
                &self.identity_root,
                &self.options,
                reuse_horizon,
                candidates.clone(),
                self.reports.clone(),
            );
            publish_generation(&location, &generation, &mut stats);
            evict(&location.directory);
            if let Some(emitter) = progress {
                emitter.phase_completed(
                    ProgressPhase::CachePublication,
                    None,
                    ProgressMeasure::Indeterminate,
                );
            }
        }
        self.snapshot = snapshot;
        self.candidates = candidates;
        self.reuse_horizon = reuse_horizon;
        self.stats = stats;
        Ok(())
    }

    #[must_use]
    pub const fn snapshot(&self) -> &IdentifiedPlan {
        &self.snapshot
    }

    #[must_use]
    pub const fn stats(&self) -> PlanStoreStats {
        self.stats
    }

    #[must_use]
    pub fn into_snapshot(self) -> IdentifiedPlan {
        self.snapshot
    }

    /// Bind a validation request to this exact plan and trace-input state.
    #[must_use]
    pub fn structural_report_key(&self, options: &aplan_core::Options) -> Option<String> {
        (self.options.writer_identity.is_some() && self.options.mode != CacheMode::Off)
            .then(|| report_key(&self.root, &self.candidates, &self.snapshot.inputs, options))
            .flatten()
    }

    /// Rehydrate a report for an already-bound key.
    #[must_use]
    pub fn cached_structural_report(&self, key: &str) -> Option<aplan_core::Report> {
        if self.options.writer_identity.is_some() && self.options.mode == CacheMode::Auto {
            return self
                .reports
                .iter()
                .find(|report| report.key == key)
                .and_then(CachedReport::report);
        }
        None
    }

    /// Publish a report only if the store still observes the key it was
    /// computed for. A concurrent refresh therefore cannot install an old
    /// answer over a newer snapshot.
    pub fn remember_structural_report(
        &mut self,
        key: &str,
        options: &aplan_core::Options,
        report: &aplan_core::Report,
    ) {
        if self.options.writer_identity.is_some()
            && self.options.mode != CacheMode::Off
            && report_key(&self.root, &self.candidates, &self.snapshot.inputs, options).as_deref()
                == Some(key)
        {
            self.reports.retain(|cached| cached.key != key);
            self.reports
                .push(CachedReport::from_report(key.to_string(), report));
            if self.reports.len() > 8 {
                self.reports.remove(0);
            }
            if let Some(location) = CacheLocation::resolve(&self.identity_root, &self.options) {
                let generation = Generation::new(
                    &self.identity_root,
                    &self.options,
                    self.reuse_horizon,
                    self.candidates.clone(),
                    self.reports.clone(),
                );
                publish_generation(&location, &generation, &mut self.stats);
                evict(&location.directory);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Envelope {
    checksum: String,
    generation: Generation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Generation {
    magic: String,
    format_epoch: u32,
    semantic_epoch: u32,
    platform: String,
    schema_sha256: String,
    writer_identity: Option<String>,
    root: EncodedPath,
    configuration_file: Option<EncodedPath>,
    created_at: TimeValue,
    candidates: Vec<CachedCandidate>,
    reports: Vec<CachedReport>,
}

impl Generation {
    fn new(
        root: &Path,
        options: &PlanStoreOptions,
        created_at: TimeValue,
        candidates: Vec<CachedCandidate>,
        reports: Vec<CachedReport>,
    ) -> Self {
        Self {
            magic: MAGIC.to_string(),
            format_epoch: FORMAT_EPOCH,
            semantic_epoch: SEMANTIC_EPOCH,
            platform: platform_tag().to_string(),
            schema_sha256: schema_identity(),
            writer_identity: options.writer_identity.clone(),
            root: EncodedPath::from_path(root),
            configuration_file: options
                .configuration_file
                .as_deref()
                .map(absolute)
                .as_deref()
                .map(EncodedPath::from_path),
            created_at,
            candidates,
            reports,
        }
    }

    fn matches(&self, root: &Path, options: &PlanStoreOptions) -> bool {
        self.magic == MAGIC
            && self.format_epoch == FORMAT_EPOCH
            && self.semantic_epoch == SEMANTIC_EPOCH
            && self.platform == platform_tag()
            && self.schema_sha256 == schema_identity()
            && self.writer_identity == options.writer_identity
            && self.root == EncodedPath::from_path(root)
            && self.configuration_file
                == options
                    .configuration_file
                    .as_deref()
                    .map(absolute)
                    .as_deref()
                    .map(EncodedPath::from_path)
            && self.created_at.bounded()
            && self.candidates.len() <= MAX_CANDIDATES
            && self.reports.len() <= 8
            && self.candidates.iter().all(CachedCandidate::bounded)
            && self.reports.iter().all(CachedReport::bounded)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedCandidate {
    path: EncodedPath,
    sha256: String,
    observation: FileObservation,
    artifact: CachedArtifact,
}

impl CachedCandidate {
    fn bounded(&self) -> bool {
        self.path.encoded.len() <= MAX_PATH_BYTES.saturating_mul(2)
            && self.path.is_bounded_relative()
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            && self.observation.bounded()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileObservation {
    length: u64,
    modified: Option<TimeValue>,
    file_type: String,
    symlink_target: Option<EncodedPath>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    change_seconds: i64,
    #[cfg(unix)]
    change_nanoseconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct TimeValue {
    before_epoch: bool,
    seconds: u64,
    nanoseconds: u32,
}

impl FileObservation {
    fn capture(path: &Path) -> io::Result<Self> {
        let followed = fs::metadata(path)?;
        let link = fs::symlink_metadata(path)?;
        let file_type = if followed.is_file() {
            "file"
        } else if followed.is_dir() {
            "directory"
        } else {
            "other"
        };
        let symlink_target = link
            .file_type()
            .is_symlink()
            .then(|| fs::read_link(path).map(|target| EncodedPath::from_path(&target)))
            .transpose()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Ok(Self {
                length: followed.len(),
                modified: followed.modified().ok().map(TimeValue::from_system),
                file_type: file_type.to_string(),
                symlink_target,
                device: followed.dev(),
                inode: followed.ino(),
                mode: followed.mode(),
                change_seconds: followed.ctime(),
                change_nanoseconds: followed.ctime_nsec(),
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                length: followed.len(),
                modified: followed.modified().ok().map(TimeValue::from_system),
                file_type: file_type.to_string(),
                symlink_target,
            })
        }
    }

    fn permits_metadata_reuse(&self, current: &Self, horizon: TimeValue) -> bool {
        self == current && self.has_strong_change_identity() && self.changed_before(horizon)
    }

    fn has_strong_change_identity(&self) -> bool {
        if self.file_type != "file" {
            return false;
        }
        #[cfg(unix)]
        {
            // A zero sub-second change time may be a filesystem that exposes
            // only coarse timestamps. Re-hash there rather than pretending a
            // same-size write inside that second is distinguishable.
            self.change_nanoseconds != 0
        }
        #[cfg(not(unix))]
        {
            // std does not expose an equally strong portable file/change id.
            // Correctness wins: these platforms take the content path.
            false
        }
    }

    fn bounded(&self) -> bool {
        self.file_type.len() <= 16
            && self.modified.is_none_or(TimeValue::bounded)
            && self
                .symlink_target
                .as_ref()
                .is_none_or(|target| target.encoded.len() <= MAX_PATH_BYTES.saturating_mul(2))
            && {
                #[cfg(unix)]
                {
                    (0..1_000_000_000).contains(&self.change_nanoseconds)
                }
                #[cfg(not(unix))]
                {
                    true
                }
            }
    }

    fn changed_before(&self, horizon: TimeValue) -> bool {
        let Some(modified) = self.modified else {
            return false;
        };
        if modified >= horizon {
            return false;
        }
        #[cfg(unix)]
        {
            self.change_seconds < 0
                || TimeValue {
                    before_epoch: false,
                    seconds: u64::try_from(self.change_seconds).unwrap_or_default(),
                    nanoseconds: u32::try_from(self.change_nanoseconds).unwrap_or_default(),
                } < horizon
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

impl TimeValue {
    fn now() -> Self {
        Self::from_system(SystemTime::now())
    }

    const fn bounded(self) -> bool {
        !self.before_epoch && self.nanoseconds < 1_000_000_000
    }

    fn from_system(value: SystemTime) -> Self {
        match value.duration_since(UNIX_EPOCH) {
            Ok(duration) => Self {
                before_epoch: false,
                seconds: duration.as_secs(),
                nanoseconds: duration.subsec_nanos(),
            },
            Err(error) => {
                let duration = error.duration();
                Self {
                    before_epoch: true,
                    seconds: duration.as_secs(),
                    nanoseconds: duration.subsec_nanos(),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CachedArtifact {
    NotTask,
    Task(Box<CachedTask>),
    /// A load failure stays in the content manifest but is reparsed so an old
    /// diagnostic implementation is never treated as a current parser result.
    Uncacheable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedTask {
    task: Task,
    fields: BTreeMap<String, CachedLocation>,
    criteria: CachedCriteria,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct CachedLocation {
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedCriteria {
    total: usize,
    ticked: usize,
    first_unticked: Option<usize>,
    scaffold_prompt: Option<String>,
    scaffold_ticked: Option<usize>,
    scaffold_unticked: Option<usize>,
    prompts: usize,
    why_untouched: bool,
}

impl CachedArtifact {
    fn from_parsed(parsed: &ParsedCandidate) -> Self {
        match parsed {
            ParsedCandidate::NotTask => Self::NotTask,
            ParsedCandidate::Task(loaded) => Self::Task(Box::new(CachedTask {
                task: loaded.task.clone(),
                fields: loaded
                    .fields
                    .iter()
                    .map(|(name, location)| {
                        (
                            name.clone(),
                            CachedLocation {
                                line: location.line,
                                column: location.column,
                            },
                        )
                    })
                    .collect(),
                criteria: CachedCriteria {
                    total: loaded.criteria.total,
                    ticked: loaded.criteria.ticked,
                    first_unticked: loaded.criteria.first_unticked,
                    scaffold_prompt: loaded.criteria.scaffold_prompt.map(str::to_string),
                    scaffold_ticked: loaded.criteria.scaffold_ticked,
                    scaffold_unticked: loaded.criteria.scaffold_unticked,
                    prompts: loaded.criteria.prompts,
                    why_untouched: loaded.criteria.why_untouched,
                },
            })),
            ParsedCandidate::Failed { .. } => Self::Uncacheable,
        }
    }

    fn parsed(&self, path: &Path) -> Option<ParsedCandidate> {
        match self {
            Self::NotTask => Some(ParsedCandidate::NotTask),
            Self::Task(cached) => {
                let prompt = match cached.criteria.scaffold_prompt.as_deref() {
                    Some(value) => Some(
                        aplan_core::scaffold::criterion_prompts()
                            .into_iter()
                            .find(|known| *known == value)?,
                    ),
                    None => None,
                };
                Some(ParsedCandidate::Task(Box::new(LoadedTask {
                    task: cached.task.clone(),
                    path: path.to_path_buf(),
                    fields: cached
                        .fields
                        .iter()
                        .map(|(name, location)| {
                            (
                                name.clone(),
                                Location {
                                    line: location.line,
                                    column: location.column,
                                },
                            )
                        })
                        .collect(),
                    criteria: Criteria {
                        total: cached.criteria.total,
                        ticked: cached.criteria.ticked,
                        first_unticked: cached.criteria.first_unticked,
                        scaffold_prompt: prompt,
                        scaffold_ticked: cached.criteria.scaffold_ticked,
                        scaffold_unticked: cached.criteria.scaffold_unticked,
                        prompts: cached.criteria.prompts,
                        why_untouched: cached.criteria.why_untouched,
                    },
                })))
            }
            Self::Uncacheable => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedReport {
    key: String,
    tasks_checked: usize,
    diagnostics: Vec<CachedDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedDiagnostic {
    code: String,
    severity: String,
    task: Option<String>,
    file: Option<EncodedPath>,
    message: String,
    agent_instruction: Option<String>,
    repair: Option<aplan_core::Repair>,
    location: Option<CachedLocation>,
    detail: Vec<String>,
}

impl CachedReport {
    fn bounded(&self) -> bool {
        if self.key.len() != 64
            || !self.key.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.tasks_checked > MAX_CANDIDATES
            || self.diagnostics.len() > MAX_REPORT_DIAGNOSTICS
        {
            return false;
        }
        let mut aggregate = 0_usize;
        for diagnostic in &self.diagnostics {
            let repair_bytes = diagnostic
                .repair
                .as_ref()
                .and_then(|repair| serde_json::to_vec(repair).ok())
                .map_or(0, |bytes| bytes.len());
            let local = diagnostic
                .code
                .len()
                .saturating_add(diagnostic.severity.len())
                .saturating_add(diagnostic.task.as_ref().map_or(0, String::len))
                .saturating_add(diagnostic.message.len())
                .saturating_add(diagnostic.agent_instruction.as_ref().map_or(0, String::len))
                .saturating_add(diagnostic.detail.iter().map(String::len).sum::<usize>())
                .saturating_add(repair_bytes);
            if local > MAX_DIAGNOSTIC_TEXT_BYTES
                || diagnostic
                    .file
                    .as_ref()
                    .is_some_and(|path| path.encoded.len() > MAX_PATH_BYTES.saturating_mul(2))
            {
                return false;
            }
            aggregate = aggregate.saturating_add(local);
        }
        aggregate <= MAX_REPORT_TEXT_BYTES
    }

    fn from_report(key: String, report: &aplan_core::Report) -> Self {
        Self {
            key,
            tasks_checked: report.tasks_checked,
            diagnostics: report
                .diagnostics
                .iter()
                .map(|diagnostic| CachedDiagnostic {
                    code: diagnostic.code.to_string(),
                    severity: diagnostic.severity.to_string(),
                    task: diagnostic.task.clone(),
                    file: diagnostic.file.as_deref().map(EncodedPath::from_path),
                    message: diagnostic.message.clone(),
                    agent_instruction: diagnostic.agent_instruction.clone(),
                    repair: diagnostic.repair.clone(),
                    location: diagnostic.location.map(|location| CachedLocation {
                        line: location.line,
                        column: location.column,
                    }),
                    detail: diagnostic.detail.clone(),
                })
                .collect(),
        }
    }

    fn report(&self) -> Option<aplan_core::Report> {
        let diagnostics = self
            .diagnostics
            .iter()
            .map(CachedDiagnostic::diagnostic)
            .collect::<Option<Vec<_>>>()?;
        Some(aplan_core::Report {
            diagnostics,
            tasks_checked: self.tasks_checked,
        })
    }
}

impl CachedDiagnostic {
    fn diagnostic(&self) -> Option<Diagnostic> {
        let code = aplan_core::Rule::from_code(&self.code)?.code();
        let severity = match self.severity.as_str() {
            "warning" => aplan_core::Severity::Warning,
            "error" => aplan_core::Severity::Error,
            _ => return None,
        };
        let mut diagnostic = match severity {
            aplan_core::Severity::Warning => Diagnostic::warning(code, self.message.clone()),
            aplan_core::Severity::Error => Diagnostic::error(code, self.message.clone()),
        };
        diagnostic.task.clone_from(&self.task);
        diagnostic.file = self.file.as_ref().and_then(EncodedPath::to_path);
        diagnostic
            .agent_instruction
            .clone_from(&self.agent_instruction);
        diagnostic.repair.clone_from(&self.repair);
        diagnostic.location = self.location.map(|location| Location {
            line: location.line,
            column: location.column,
        });
        diagnostic.detail.clone_from(&self.detail);
        Some(diagnostic)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct EncodedPath {
    encoding: String,
    encoded: String,
}

impl EncodedPath {
    #[cfg(unix)]
    fn from_path(path: &Path) -> Self {
        use std::os::unix::ffi::OsStrExt as _;
        Self {
            encoding: "unix-bytes-hex".to_string(),
            encoded: hex(path.as_os_str().as_bytes()),
        }
    }

    #[cfg(windows)]
    fn from_path(path: &Path) -> Self {
        use std::os::windows::ffi::OsStrExt as _;
        let mut bytes = Vec::new();
        for unit in path.as_os_str().encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        Self {
            encoding: "windows-wide-hex".to_string(),
            encoded: hex(&bytes),
        }
    }

    #[cfg(unix)]
    fn to_path(&self) -> Option<PathBuf> {
        use std::os::unix::ffi::OsStringExt as _;
        (self.encoding == "unix-bytes-hex")
            .then(|| unhex(&self.encoded).map(std::ffi::OsString::from_vec))?
            .map(PathBuf::from)
    }

    #[cfg(windows)]
    fn to_path(&self) -> Option<PathBuf> {
        use std::os::windows::ffi::OsStringExt as _;
        if self.encoding != "windows-wide-hex" {
            return None;
        }
        let bytes = unhex(&self.encoded)?;
        if bytes.len() % 2 != 0 {
            return None;
        }
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        Some(PathBuf::from(std::ffi::OsString::from_wide(&units)))
    }

    fn is_bounded_relative(&self) -> bool {
        let Some(path) = self.to_path() else {
            return false;
        };
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
    }
}

#[derive(Debug)]
struct CacheLocation {
    directory: PathBuf,
    generation: PathBuf,
    lock: PathBuf,
}

fn report_key(
    root: &Path,
    candidates: &[CachedCandidate],
    inputs: &[PlanFileInput],
    options: &aplan_core::Options,
) -> Option<String> {
    let mut digest = Sha256::new();
    digest.update(b"aplan-structural-report-v1\0");
    // Parsed artifacts are reconstructed with the current root spelling, but
    // a persisted Report already contains the diagnostic paths produced by the
    // invocation that computed it. Keep relative and absolute (or otherwise
    // lexically distinct) invocations from borrowing each other's prose.
    digest.update(b"root-spelling\0");
    digest.update(EncodedPath::from_path(root).encoding.as_bytes());
    digest.update([0]);
    digest.update(EncodedPath::from_path(root).encoded.as_bytes());
    digest.update([0]);
    digest.update(
        super::validation_policy_digest(options, &aplan_core::Verdict::default()).as_bytes(),
    );
    for candidate in candidates {
        digest.update(candidate.path.encoding.as_bytes());
        digest.update([0]);
        digest.update(candidate.path.encoded.as_bytes());
        digest.update([0]);
        match &candidate.artifact {
            CachedArtifact::NotTask => digest.update(b"not-task"),
            CachedArtifact::Uncacheable => {
                // Load diagnostics carry parser wording and source locations,
                // so malformed input remains byte-identified.
                digest.update(b"unloadable\0");
                digest.update(candidate.sha256.as_bytes());
            }
            CachedArtifact::Task(task) => {
                // No structural rule reads title text. Its presence and type
                // were established by parsing; changing one non-empty title to
                // another changes projections but cannot change this report.
                // Every field a rule does read, plus field/criterion locations,
                // remains in the canonical typed artifact below.
                let mut semantic = task.clone();
                semantic.task.title.clear();
                digest.update(b"task\0");
                digest.update(serde_json::to_vec(&semantic).ok()?);
            }
        }
        digest.update([0]);
    }

    // Read failures cannot become cached parsed candidates, but they are part
    // of Plan::load_diagnostics and therefore part of a structural Report. A
    // newly added unreadable file must not reuse the report from the candidate
    // set that preceded it. Successful inputs are already represented above.
    for input in inputs {
        let PlanFileContent::ReadError { kind, message } = &input.content else {
            continue;
        };
        digest.update(b"read-error\0");
        digest.update(EncodedPath::from_path(&input.path).encoding.as_bytes());
        digest.update([0]);
        digest.update(EncodedPath::from_path(&input.path).encoded.as_bytes());
        digest.update([0]);
        digest.update(super::io_error_kind_name(*kind).as_bytes());
        digest.update([0]);
        digest.update(message.as_bytes());
        digest.update([0]);
    }

    // APLAN041 is the other structural rule that reads outside the task
    // candidate set. `agents_block.current` is the expected generated text;
    // the file at `path` is the independently changing observed input the rule
    // compares it with, so both halves belong in the key.
    if let Some(agents_block) = &options.agents_block {
        digest.update(b"agents-block-observed\0");
        hash_observed_file(&mut digest, &agents_block.path);
    } else {
        digest.update(b"agents-block-absent\0");
    }

    let Some(traces_root) = options.traces_root.as_deref() else {
        digest.update(b"traces-absent\0");
        return Some(hex(&digest.finalize()));
    };
    digest.update(b"traces-present\0");
    let mut paths = std::collections::BTreeSet::new();
    for candidate in candidates {
        let CachedArtifact::Task(task) = &candidate.artifact else {
            continue;
        };
        for trace in &task.task.traces {
            let Some((relative, _)) = trace.split_once('#') else {
                continue;
            };
            let path = traces_root.join(relative);
            let identity = path.canonicalize().unwrap_or_else(|_| absolute(&path));
            paths.insert(identity);
        }
    }
    for path in paths {
        digest.update(EncodedPath::from_path(&path).encoded.as_bytes());
        digest.update([0]);
        // Unlike task candidates, reports do not persist a prior trace
        // observation plus its capture horizon. Hashing the bounded referenced
        // documents is the conservative racy-clean path and still avoids every
        // Markdown/YAML trace parse on a hit.
        hash_observed_file(&mut digest, &path);
    }
    Some(hex(&digest.finalize()))
}

fn hash_observed_file(digest: &mut Sha256, path: &Path) {
    match stable_hash(path) {
        Ok(content_sha256) => {
            digest.update(b"bytes\0");
            digest.update(content_sha256);
        }
        Err(error) => {
            digest.update(b"error\0");
            match error.raw_os_error() {
                Some(code) => {
                    digest.update(b"os\0");
                    digest.update(code.to_le_bytes());
                }
                None => digest.update(b"no-os-code\0"),
            }
            digest.update([0]);
            digest.update(error.to_string().as_bytes());
        }
    }
    digest.update([0]);
}

impl CacheLocation {
    fn resolve(root: &Path, options: &PlanStoreOptions) -> Option<Self> {
        if options.mode == CacheMode::Off {
            return None;
        }
        let writer_identity = options.writer_identity.as_deref()?;
        let base = options
            .cache_root
            .clone()
            .or_else(|| std::env::var_os("APLAN_CACHE_DIR").map(PathBuf::from))
            .or_else(system_cache_root)?;
        let directory = base.join("aplan").join("plan-v1");
        let mut digest = Sha256::new();
        digest.update(b"aplan-cache-key-v1\0");
        digest.update(EncodedPath::from_path(root).encoded.as_bytes());
        digest.update(b"\0config\0");
        match options.configuration_file.as_deref() {
            Some(path) => digest.update(EncodedPath::from_path(&absolute(path)).encoded.as_bytes()),
            None => digest.update(b"absent"),
        }
        digest.update(b"\0writer\0");
        digest.update(writer_identity.as_bytes());
        let name = hex(&digest.finalize());
        Some(Self {
            generation: directory.join(format!("{name}.json")),
            lock: directory.join(format!("{name}.lock")),
            directory,
        })
    }
}

// One loop intentionally owns candidate-set comparison and every per-file
// result: splitting its state machine made it easier to return a mixed retry.
#[allow(clippy::too_many_lines)]
fn capture(
    root: &Path,
    previous: &BTreeMap<EncodedPath, CachedCandidate>,
    previous_horizon: Option<TimeValue>,
    retain_inputs: bool,
    stats: &mut PlanStoreStats,
    mut progress: Option<&mut ProgressEmitter<'_>>,
    cancellation: Option<&CancellationToken>,
) -> io::Result<(IdentifiedPlan, Vec<CachedCandidate>)> {
    for attempt in 0..3 {
        check_cancelled(cancellation)?;
        let child = Some(ProgressChildId::new(attempt));
        if let Some(emitter) = progress.as_deref_mut() {
            emitter.phase_started(
                ProgressPhase::SourceEnumeration,
                child,
                ProgressMeasure::Indeterminate,
            );
        }
        let before = plan_candidate_paths(root);
        if let Some(emitter) = progress.as_deref_mut() {
            emitter.phase_completed(
                ProgressPhase::SourceEnumeration,
                child,
                ProgressMeasure::Indeterminate,
            );
            emitter.phase_started(
                ProgressPhase::SourceParsing,
                child,
                ProgressMeasure::bounded(0, before.len(), ProgressUnit::Sources),
            );
        }
        let mut plan = Plan::default();
        let mut inputs = Vec::with_capacity(before.len());
        let mut candidates = Vec::with_capacity(before.len());
        let mut local = LoadStats::default();

        for (candidate_index, path) in before.iter().enumerate() {
            check_cancelled(cancellation)?;
            local.candidate_files += 1;
            let relative = path.strip_prefix(root).unwrap_or(path);
            let encoded = EncodedPath::from_path(relative);
            let observed = FileObservation::capture(path);
            let metadata_reuse = observed.as_ref().ok().and_then(|observation| {
                previous.get(&encoded).and_then(|candidate| {
                    previous_horizon
                        .is_some_and(|horizon| {
                            candidate
                                .observation
                                .permits_metadata_reuse(observation, horizon)
                        })
                        .then(|| candidate.artifact.parsed(path))
                        .flatten()
                        .map(|parsed| (candidate, parsed))
                })
            });
            if let Some((candidate, parsed)) = metadata_reuse
                && !retain_inputs
            {
                local.readable_files += 1;
                if !matches!(candidate.artifact, CachedArtifact::NotTask) {
                    local.frontmatter_candidates += 1;
                }
                stats.artifacts_reused += 1;
                apply_parsed(&mut plan, parsed, path);
                candidates.push(candidate.clone());
                inputs.push(PlanFileInput {
                    path: path.clone(),
                    content: PlanFileContent::Sha256(candidate.sha256.clone()),
                });
                if let Some(emitter) = progress.as_deref_mut() {
                    emitter.phase_advanced(
                        ProgressPhase::SourceParsing,
                        child,
                        ProgressMeasure::bounded(
                            candidate_index.saturating_add(1),
                            before.len(),
                            ProgressUnit::Sources,
                        ),
                    );
                }
                continue;
            }
            match stable_read(path) {
                Ok(captured) => {
                    let bytes = captured.bytes;
                    local.readable_files += 1;
                    local.bytes_read = local
                        .bytes_read
                        .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                    if retain_inputs {
                        local.retained_files += 1;
                        local.retained_bytes = local
                            .retained_bytes
                            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                    }
                    let digest = hex_sha256(&bytes);
                    let reused = previous.get(&encoded).and_then(|candidate| {
                        (candidate.sha256 == digest)
                            .then(|| candidate.artifact.parsed(path))
                            .flatten()
                    });
                    let was_reused = reused.is_some();
                    let parsed = if let Some(parsed) = reused {
                        stats.artifacts_reused += 1;
                        parsed
                    } else {
                        stats.artifacts_parsed += 1;
                        parse_plan_candidate(path, &bytes)
                    };
                    if !matches!(parsed, ParsedCandidate::NotTask) {
                        local.frontmatter_candidates += 1;
                        if !was_reused {
                            local.task_files_parsed += 1;
                        }
                    }
                    apply_parsed(&mut plan, parsed.clone(), path);
                    candidates.push(CachedCandidate {
                        path: encoded,
                        sha256: digest.clone(),
                        observation: captured.observation,
                        artifact: CachedArtifact::from_parsed(&parsed),
                    });
                    if retain_inputs {
                        inputs.push(PlanFileInput {
                            path: path.clone(),
                            content: PlanFileContent::Bytes(bytes),
                        });
                    } else {
                        inputs.push(PlanFileInput {
                            path: path.clone(),
                            content: PlanFileContent::Sha256(digest),
                        });
                    }
                }
                Err(error) => {
                    if retain_inputs {
                        local.retained_files += 1;
                    }
                    plan.load_diagnostics.push(
                        Diagnostic::error(code::SCHEMA, format!("cannot read: {error}"))
                            .with_file(path)
                            .with_agent_instruction(
                                "Make the file readable, or remove it from the plan directory."
                                    .to_string(),
                            ),
                    );
                    inputs.push(PlanFileInput {
                        path: path.clone(),
                        content: PlanFileContent::ReadError {
                            kind: error.kind(),
                            message: error.to_string(),
                        },
                    });
                }
            }
            if let Some(emitter) = progress.as_deref_mut() {
                emitter.phase_advanced(
                    ProgressPhase::SourceParsing,
                    child,
                    ProgressMeasure::bounded(
                        candidate_index.saturating_add(1),
                        before.len(),
                        ProgressUnit::Sources,
                    ),
                );
            }
        }

        if let Some(emitter) = progress.as_deref_mut() {
            emitter.phase_completed(
                ProgressPhase::SourceParsing,
                child,
                ProgressMeasure::bounded(before.len(), before.len(), ProgressUnit::Sources),
            );
            emitter.phase_started(
                ProgressPhase::SnapshotConfirmation,
                child,
                ProgressMeasure::Indeterminate,
            );
        }

        let after = plan_candidate_paths(root);
        check_cancelled(cancellation)?;
        if let Some(emitter) = progress.as_deref_mut() {
            emitter.phase_completed(
                ProgressPhase::SnapshotConfirmation,
                child,
                ProgressMeasure::Indeterminate,
            );
        }
        if before == after {
            plan.tasks.sort_by(|a, b| a.task.id.cmp(&b.task.id));
            stats.load = local;
            return Ok((
                IdentifiedPlan {
                    root: root.to_path_buf(),
                    plan,
                    inputs,
                },
                candidates,
            ));
        }
        if attempt == 2 {
            return Err(io::Error::other(PlanSnapshotChanged));
        }
    }
    unreachable!("the bounded capture loop returns")
}

fn check_cancelled(cancellation: Option<&CancellationToken>) -> io::Result<()> {
    cancellation.map_or(Ok(()), CancellationToken::check)
}

fn apply_parsed(plan: &mut Plan, parsed: ParsedCandidate, path: &Path) {
    match parsed {
        ParsedCandidate::NotTask => {}
        ParsedCandidate::Task(task) => plan.tasks.push(*task),
        ParsedCandidate::Failed {
            id,
            mut diagnostics,
        } => {
            if let Some(id) = id {
                plan.unloadable.insert(id, path.to_path_buf());
            }
            plan.load_diagnostics.append(&mut diagnostics);
        }
    }
}

struct CapturedFile {
    bytes: Vec<u8>,
    observation: FileObservation,
}

fn stable_read(path: &Path) -> io::Result<CapturedFile> {
    for attempt in 0..3 {
        let before = FileObservation::capture(path)?;
        let bytes = fs::read(path)?;
        let after = FileObservation::capture(path)?;
        if before == after && after.length == u64::try_from(bytes.len()).unwrap_or(u64::MAX) {
            return Ok(CapturedFile {
                bytes,
                observation: after,
            });
        }
        if attempt == 2 {
            return Err(io::Error::other(format!(
                "`{}` kept changing while it was read; retry the command",
                path.display()
            )));
        }
    }
    unreachable!("the bounded read loop returns")
}

fn stable_hash(path: &Path) -> io::Result<[u8; 32]> {
    for attempt in 0..3 {
        let before = FileObservation::capture(path)?;
        let mut file = fs::File::open(path)?;
        let mut digest = Sha256::new();
        let mut bytes_read = 0_u64;
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
            bytes_read = bytes_read.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        }
        let after = FileObservation::capture(path)?;
        if before == after && after.length == bytes_read {
            return Ok(digest.finalize().into());
        }
        if attempt == 2 {
            return Err(io::Error::other(format!(
                "`{}` kept changing while it was hashed; retry the command",
                path.display()
            )));
        }
    }
    unreachable!("the bounded hash loop returns")
}

fn read_generation(
    location: &CacheLocation,
    root: &Path,
    options: &PlanStoreOptions,
    stats: &mut PlanStoreStats,
) -> Option<Generation> {
    safe_private_cache_namespace(&location.directory)?;
    let metadata = safe_private_regular_file(&location.generation)?;
    if metadata.len() > MAX_CACHE_BYTES {
        return None;
    }
    // Do not let a file that grows after the metadata check allocate without
    // bound. The generation is private derived state, so any extra byte is a
    // quiet miss rather than a partial decode.
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).ok()?);
    fs::File::open(&location.generation)
        .ok()?
        .take(MAX_CACHE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    if u64::try_from(bytes.len()).ok()? > MAX_CACHE_BYTES {
        return None;
    }
    stats.cache_bytes_read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let envelope: Envelope = serde_json::from_slice(&bytes).ok()?;
    let generation_bytes = serde_json::to_vec(&envelope.generation).ok()?;
    if envelope.checksum != hex_sha256(&generation_bytes)
        || !envelope.generation.matches(root, options)
    {
        return None;
    }
    Some(envelope.generation)
}

fn publish_generation(
    location: &CacheLocation,
    generation: &Generation,
    stats: &mut PlanStoreStats,
) {
    if ensure_private_directory(&location.directory).is_err() {
        return;
    }
    let Some(_lock) = CacheLock::acquire(&location.lock) else {
        return;
    };
    let Ok(generation_bytes) = serde_json::to_vec(generation) else {
        return;
    };
    let envelope = Envelope {
        checksum: hex_sha256(&generation_bytes),
        generation: generation.clone(),
    };
    let Ok(bytes) = serde_json::to_vec(&envelope) else {
        return;
    };
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CACHE_BYTES {
        return;
    }
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = location.directory.join(format!(
        ".tmp-{}-{}-{sequence}",
        std::process::id(),
        now_nanos()
    ));
    let opened = private_create_new(&temporary);
    let Ok(mut file) = opened else {
        return;
    };
    let written = file
        .write_all(&bytes)
        .and_then(|()| crate::rename_replacing(&temporary, &location.generation));
    if written.is_ok() {
        stats.cache_bytes_written = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    } else {
        let _ = fs::remove_file(&temporary);
    }
}

#[derive(Debug)]
struct CacheLock {
    path: PathBuf,
}

impl CacheLock {
    fn acquire(path: &Path) -> Option<Self> {
        if let Some(metadata) = safe_regular_file(path)
            && metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(30))
        {
            let _ = fs::remove_file(path);
        }
        let mut file = private_create_new(path).ok()?;
        writeln!(file, "{}", std::process::id()).ok()?;
        Some(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn evict(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = Vec::new();
    let mut total = 0_u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(metadata) = safe_regular_file(&path) else {
            continue;
        };
        if path.extension().is_some_and(|ext| ext == "lock") {
            continue;
        }
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        if modified.elapsed().is_ok_and(|age| age > MAX_CACHE_AGE) {
            let _ = fs::remove_file(path);
            continue;
        }
        total = total.saturating_add(metadata.len());
        files.push((modified, metadata.len(), path));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));
    for (_, size, path) in files {
        if total <= MAX_CACHE_TOTAL {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

fn ensure_private_directory(directory: &Path) -> io::Result<()> {
    let parent = directory
        .parent()
        .ok_or_else(|| io::Error::other("cache directory has no parent"))?;
    fs::create_dir_all(parent)?;
    if safe_directory(parent).is_none() {
        return Err(io::Error::other("unsafe cache parent"));
    }
    set_private_directory_permissions(parent)?;
    match fs::symlink_metadata(directory) {
        Ok(_) if safe_directory(directory).is_none() => {
            return Err(io::Error::other("unsafe cache directory"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(directory)?,
        Err(error) => return Err(error),
    }
    set_private_directory_permissions(directory)
}

fn safe_private_cache_namespace(directory: &Path) -> Option<()> {
    let parent = directory.parent()?;
    let parent = safe_directory(parent)?;
    let directory = safe_directory(directory)?;
    private_permissions(&parent).then_some(())?;
    private_permissions(&directory).then_some(())
}

fn safe_directory(path: &Path) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    (!metadata.file_type().is_symlink() && metadata.is_dir()).then_some(metadata)
}

fn safe_regular_file(path: &Path) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    (!metadata.file_type().is_symlink() && metadata.is_file()).then_some(metadata)
}

fn safe_private_regular_file(path: &Path) -> Option<fs::Metadata> {
    let metadata = safe_regular_file(path)?;
    private_permissions(&metadata).then_some(metadata)
}

#[cfg(unix)]
fn private_permissions(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode().trailing_zeros() >= 6
}

#[cfg(not(unix))]
fn private_permissions(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)] // Preserve one fallible permission interface on every OS.
fn set_private_directory_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn private_create_new(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_create_new(path: &Path) -> io::Result<fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn system_cache_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    }
}

fn platform_tag() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(unix) {
        "unix"
    } else {
        "other"
    }
}

fn schema_identity() -> String {
    let bytes = serde_json::to_vec(&aplan_core::load::derived_schema()).unwrap_or_default();
    hex_sha256(&bytes)
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

fn unhex(value: &str) -> Option<Vec<u8>> {
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            u8::try_from((high << 4) | low).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    #[cfg(unix)]
    use super::FileObservation;
    #[cfg(unix)]
    use super::TimeValue;
    use super::{
        CacheMode, CachedDiagnostic, CachedReport, EncodedPath, Envelope, MAX_CACHE_BYTES,
        MAX_CACHE_TOTAL, MAX_DIAGNOSTIC_TEXT_BYTES, PlanStore, PlanStoreOptions, evict, hex_sha256,
        unhex,
    };

    #[test]
    fn cache_mode_and_hostile_private_values_fail_closed() {
        assert_eq!(CacheMode::from_name("auto"), Some(CacheMode::Auto));
        assert_eq!(CacheMode::from_name("off"), Some(CacheMode::Off));
        assert_eq!(CacheMode::from_name("refresh"), Some(CacheMode::Refresh));
        assert_eq!(CacheMode::from_name("unknown"), None);
        assert!(unhex("0").is_none());
        assert!(unhex("zz").is_none());

        let invalid_key = CachedReport {
            key: "not-a-digest".to_string(),
            tasks_checked: 0,
            diagnostics: Vec::new(),
        };
        assert!(!invalid_key.bounded());

        let diagnostic = CachedDiagnostic {
            code: aplan_core::diagnostic::code::SCHEMA.to_string(),
            severity: "future-severity".to_string(),
            task: None,
            file: None,
            message: "invalid source".to_string(),
            agent_instruction: Some("fix it".to_string()),
            repair: None,
            location: None,
            detail: Vec::new(),
        };
        let invalid_severity = CachedReport {
            key: "0".repeat(64),
            tasks_checked: 1,
            diagnostics: vec![diagnostic.clone()],
        };
        assert!(invalid_severity.bounded());
        assert!(invalid_severity.report().is_none());

        let mut oversized = diagnostic;
        oversized.severity = "error".to_string();
        oversized.message = "x".repeat(MAX_DIAGNOSTIC_TEXT_BYTES + 1);
        assert!(
            !CachedReport {
                key: "0".repeat(64),
                tasks_checked: 1,
                diagnostics: vec![oversized],
            }
            .bounded()
        );
    }

    #[cfg(unix)]
    #[test]
    fn filesystem_observations_cover_non_files_and_pre_epoch_times() {
        let temporary = tempfile::Builder::new()
            .prefix("aplan-cache-observation-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary directory");
        let directory = FileObservation::capture(temporary.path()).expect("directory observation");
        assert_eq!(directory.file_type, "directory");
        assert!(!directory.has_strong_change_identity());

        assert_eq!(
            FileObservation::capture(Path::new("/dev/null"))
                .expect("device observation")
                .file_type,
            "other"
        );

        let before_epoch = TimeValue::from_system(
            std::time::UNIX_EPOCH
                .checked_sub(std::time::Duration::from_secs(2))
                .expect("pre-epoch instant"),
        );
        assert!(before_epoch.before_epoch);
        assert_eq!(before_epoch.seconds, 2);

        let mut no_modified = directory;
        no_modified.modified = None;
        assert!(!no_modified.changed_before(TimeValue::now()));

        let foreign = EncodedPath {
            encoding: "future-encoding".to_string(),
            encoded: "00".to_string(),
        };
        assert!(foreign.to_path().is_none());
        let traversal = EncodedPath::from_path(Path::new("../outside"));
        assert!(!traversal.is_bounded_relative());
    }

    #[test]
    fn eviction_bounds_age_and_aggregate_size_without_plan_state() {
        let temporary = tempfile::Builder::new()
            .prefix("aplan-cache-eviction-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary cache directory");
        let stale = temporary.path().join("stale.json");
        fs::write(&stale, b"old").expect("stale generation");
        fs::File::options()
            .write(true)
            .open(&stale)
            .and_then(|file| {
                file.set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            })
            .expect("age stale generation");
        evict(temporary.path());
        assert!(!stale.exists());

        for name in ["a.json", "b.json", "c.json"] {
            let file = fs::File::create(temporary.path().join(name)).expect("size generation");
            file.set_len(MAX_CACHE_TOTAL / 2)
                .expect("sparse cache file");
        }
        evict(temporary.path());
        let total = fs::read_dir(temporary.path())
            .expect("cache listing")
            .map(|entry| {
                entry
                    .expect("cache entry")
                    .metadata()
                    .expect("metadata")
                    .len()
            })
            .sum::<u64>();
        assert!(total <= MAX_CACHE_TOTAL, "cache retained {total} bytes");
    }

    #[cfg(unix)]
    #[test]
    fn metadata_reuse_refuses_a_racy_observation_at_the_snapshot_horizon() {
        let observed = FileObservation {
            length: 10,
            modified: Some(TimeValue {
                before_epoch: false,
                seconds: 20,
                nanoseconds: 500,
            }),
            file_type: "file".to_string(),
            symlink_target: None,
            device: 1,
            inode: 2,
            mode: 0o100_600,
            change_seconds: 20,
            change_nanoseconds: 500,
        };
        let overlapping = TimeValue {
            before_epoch: false,
            seconds: 20,
            nanoseconds: 500,
        };
        let safely_after = TimeValue {
            before_epoch: false,
            seconds: 20,
            nanoseconds: 501,
        };

        assert!(!observed.permits_metadata_reuse(&observed, overlapping));
        assert!(observed.permits_metadata_reuse(&observed, safely_after));

        let mut coarse = observed.clone();
        coarse.change_nanoseconds = 0;
        assert!(!coarse.permits_metadata_reuse(&coarse, safely_after));
    }

    fn task(id: &str, title: &str) -> String {
        format!(
            "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: todo\neffort_days: 1\nblocked_by: []\npaths:\n- src/**\n---\n\n## Acceptance criteria\n\n- [ ] works\n"
        )
    }

    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let temporary = tempfile::Builder::new()
            .prefix("aplan-store-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary directory");
        let root = temporary.path().join("plan");
        let cache = temporary.path().join("cache");
        fs::create_dir(&root).expect("plan directory");
        fs::write(root.join("A-1.md"), task("A-1", "one")).expect("task");
        fs::write(root.join("README.md"), "# prose\n").expect("prose");
        (temporary, root, cache)
    }

    fn options(cache: &Path) -> PlanStoreOptions {
        PlanStoreOptions {
            cache_root: Some(cache.to_path_buf()),
            writer_identity: Some("test-build-a".to_string()),
            ..PlanStoreOptions::default()
        }
    }

    /// Proof for `progress.events_are_generation_safe`: the plan loader emits
    /// its real bounded source work through the validated shared lifecycle.
    #[test]
    fn plan_store_emits_a_valid_bounded_source_lifecycle() {
        let (_temporary, root, cache) = fixture();
        let operation = crate::OperationId::new(17);
        let mut events = Vec::new();
        PlanStore::open_with_progress(&root, &options(&cache), operation, &mut |event| {
            events.push(event);
        })
        .expect("progressive load");

        let mut state = crate::ProgressState::new(operation, crate::OperationKind::PlanLoad);
        for event in events.iter().copied() {
            state.accept(event).expect("store emitted valid progress");
        }
        assert!(events.iter().any(|event| {
            event.subject
                == crate::ProgressSubject::Phase {
                    phase: crate::ProgressPhase::SourceParsing,
                    child: Some(crate::ProgressChildId::new(0)),
                }
                && event.lifecycle == crate::ProgressLifecycle::Completed
                && matches!(
                    event.measure,
                    crate::ProgressMeasure::Bounded {
                        completed,
                        total,
                        unit: crate::ProgressUnit::Sources,
                    } if completed == total && total > 0
                )
        }));
        assert_eq!(
            events.last().map(|event| event.lifecycle),
            Some(crate::ProgressLifecycle::Completed)
        );
    }

    #[test]
    fn observed_plan_store_refuses_cancelled_work_before_source_io() {
        let (_temporary, root, cache) = fixture();
        let operation = crate::OperationId::new(18);
        let cancellation = crate::CancellationToken::default();
        cancellation.cancel();
        let mut events = Vec::new();
        let mut collect = |event| events.push(event);
        let mut emitter =
            crate::ProgressEmitter::begin(operation, crate::OperationKind::PlanLoad, &mut collect);
        let error = PlanStore::open_observed(&root, &options(&cache), &mut emitter, &cancellation)
            .expect_err("cancelled load");
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        emitter.cancel();
        drop(emitter);
        assert_eq!(
            events.last().map(|event| event.lifecycle),
            Some(crate::ProgressLifecycle::Cancelled)
        );
        assert!(
            !cache.exists(),
            "cancellation before source I/O must not publish a cache generation"
        );
    }

    fn titles(store: &PlanStore) -> Vec<(&str, &str)> {
        store
            .snapshot()
            .plan
            .tasks
            .iter()
            .map(|loaded| (loaded.task.id.as_str(), loaded.task.title.as_str()))
            .collect()
    }

    fn generation(cache: &Path) -> PathBuf {
        fs::read_dir(cache.join("aplan/plan-v1"))
            .expect("cache directory")
            .flatten()
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .expect("generation")
    }

    fn rewrite_generation(cache: &Path, mutate: impl FnOnce(&mut super::Generation)) {
        let path = generation(cache);
        let mut envelope: Envelope =
            serde_json::from_slice(&fs::read(&path).expect("read generation"))
                .expect("decode generation");
        mutate(&mut envelope.generation);
        envelope.checksum =
            hex_sha256(&serde_json::to_vec(&envelope.generation).expect("encode body"));
        fs::write(
            path,
            serde_json::to_vec(&envelope).expect("encode envelope"),
        )
        .expect("rewrite generation");
    }

    #[test]
    fn exact_hit_reuses_every_artifact_without_parsing_yaml() {
        let (_temporary, root, cache) = fixture();
        let first = PlanStore::open(&root, &options(&cache)).expect("cold load");
        assert_eq!(first.stats().artifacts_parsed, 2);
        assert_eq!(titles(&first), [("A-1", "one")]);

        let second = PlanStore::open(&root, &options(&cache)).expect("warm load");
        assert!(second.stats().cache_generation_hit);
        assert_eq!(second.stats().artifacts_reused, 2);
        assert_eq!(second.stats().artifacts_parsed, 0);
        assert_eq!(second.stats().load.task_files_parsed, 0);
        assert_eq!(titles(&second), titles(&first));
    }

    /// Proof for `storage.atomic_replacement_is_portable`: a changed cache
    /// generation is durable after the updating process goes away.
    #[test]
    fn one_same_size_edit_updates_the_persisted_generation() {
        let (_temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        fs::write(root.join("A-1.md"), task("A-1", "two")).expect("same-size edit");

        let changed = PlanStore::open(&root, &options(&cache)).expect("incremental load");
        assert!(changed.stats().cache_generation_hit);
        assert_eq!(changed.stats().artifacts_reused, 1);
        assert_eq!(changed.stats().artifacts_parsed, 1);
        assert_eq!(changed.stats().load.task_files_parsed, 1);
        assert_eq!(titles(&changed), [("A-1", "two")]);

        let retained = PlanStore::open(&root, &options(&cache)).expect("updated generation");
        assert!(retained.stats().cache_generation_hit);
        assert_eq!(retained.stats().artifacts_reused, 2);
        assert_eq!(retained.stats().artifacts_parsed, 0);
        assert_eq!(titles(&retained), [("A-1", "two")]);
    }

    #[test]
    fn atomic_editor_replacement_invalidates_the_old_artifact() {
        let (temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        let replacement = temporary.path().join("replacement");
        fs::write(&replacement, task("A-1", "two")).expect("replacement task");
        fs::rename(&replacement, root.join("A-1.md")).expect("atomic replacement");

        let changed = PlanStore::open(&root, &options(&cache)).expect("replaced load");
        assert_eq!(changed.stats().artifacts_parsed, 1);
        assert_eq!(titles(&changed), [("A-1", "two")]);
    }

    #[cfg(unix)]
    #[test]
    fn restoring_the_modified_time_does_not_hide_a_same_size_edit() {
        let (_temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        let path = root.join("A-1.md");
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .expect("original modified time");
        fs::write(&path, task("A-1", "two")).expect("same-size edit");
        fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_times(std::fs::FileTimes::new().set_modified(modified)))
            .expect("restore modified time");

        let changed = PlanStore::open(&root, &options(&cache)).expect("incremental load");
        assert_eq!(changed.stats().artifacts_parsed, 1);
        assert_eq!(titles(&changed), [("A-1", "two")]);
    }

    // Darwin rejects this deliberately invalid UTF-8 byte sequence at the
    // filesystem boundary, before the store can observe it.
    #[cfg(target_os = "linux")]
    #[test]
    fn native_non_utf8_candidate_paths_round_trip_through_the_cache() {
        use std::os::unix::ffi::OsStringExt as _;

        let (_temporary, root, cache) = fixture();
        let name = std::ffi::OsString::from_vec(vec![b'X', 0xff, b'.', b'm', b'd']);
        fs::write(root.join(name), "# prose\n").expect("non-UTF-8 candidate");
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        let warm = PlanStore::open(&root, &options(&cache)).expect("warm load");
        assert_eq!(warm.stats().artifacts_parsed, 0);
        assert_eq!(warm.stats().artifacts_reused, 3);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_retargeting_cannot_reuse_the_old_artifact() {
        use std::os::unix::fs::symlink;

        let (temporary, root, cache) = fixture();
        let first = temporary.path().join("first.md");
        let second = temporary.path().join("second.md");
        fs::write(&first, task("A-2", "first")).expect("first target");
        fs::write(&second, task("A-2", "other")).expect("second target");
        let link = root.join("linked.md");
        symlink(&first, &link).expect("first link");
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        fs::remove_file(&link).expect("remove old link");
        symlink(&second, &link).expect("second link");

        let changed = PlanStore::open(&root, &options(&cache)).expect("retargeted load");
        assert_eq!(changed.stats().artifacts_parsed, 1);
        assert!(titles(&changed).contains(&("A-2", "other")));
    }

    #[test]
    fn additions_removals_and_renames_replace_the_complete_candidate_set() {
        let (_temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        fs::rename(root.join("A-1.md"), root.join("renamed.MD")).expect("rename");
        fs::write(root.join("A-2.md"), task("A-2", "second")).expect("addition");
        fs::remove_file(root.join("README.md")).expect("removal");

        let changed = PlanStore::open(&root, &options(&cache)).expect("changed load");
        assert_eq!(changed.stats().artifacts_reused, 0);
        assert_eq!(changed.stats().artifacts_parsed, 2);
        assert_eq!(titles(&changed), [("A-1", "one"), ("A-2", "second")]);

        let exact = PlanStore::open(&root, &options(&cache)).expect("new exact load");
        assert_eq!(exact.stats().artifacts_reused, 2);
        assert_eq!(exact.stats().artifacts_parsed, 0);
    }

    #[test]
    fn corruption_and_wrong_writer_are_quiet_complete_misses() {
        let (_temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        fs::write(generation(&cache), b"{truncated").expect("corrupt generation");

        let corrupt = PlanStore::open(&root, &options(&cache)).expect("corrupt miss");
        assert!(!corrupt.stats().cache_generation_hit);
        assert_eq!(corrupt.stats().artifacts_parsed, 2);
        assert_eq!(titles(&corrupt), [("A-1", "one")]);

        let mut other = options(&cache);
        other.writer_identity = Some("test-build-b".to_string());
        let wrong_writer = PlanStore::open(&root, &other).expect("writer miss");
        assert!(!wrong_writer.stats().cache_generation_hit);
        assert_eq!(wrong_writer.stats().artifacts_parsed, 2);
    }

    #[test]
    fn an_oversized_generation_is_a_bounded_safe_miss() {
        let (_temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        fs::OpenOptions::new()
            .write(true)
            .open(generation(&cache))
            .and_then(|file| file.set_len(MAX_CACHE_BYTES + 1))
            .expect("oversized sparse generation");

        let loaded = PlanStore::open(&root, &options(&cache)).expect("bounded miss");
        assert!(!loaded.stats().cache_generation_hit);
        assert_eq!(loaded.stats().artifacts_parsed, 2);
        assert_eq!(titles(&loaded), [("A-1", "one")]);
    }

    #[test]
    fn validly_checksummed_wrong_or_hostile_generations_are_safe_misses() {
        let (_temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        let original = fs::read(generation(&cache)).expect("original generation");

        let cases: [fn(&mut super::Generation); 6] = [
            |generation| generation.format_epoch += 1,
            |generation| generation.semantic_epoch += 1,
            |generation| generation.platform.push_str("-other"),
            |generation| generation.schema_sha256 = "0".repeat(64),
            |generation| generation.root.encoded.push_str("00"),
            |generation| {
                generation.candidates[0].path = EncodedPath::from_path(Path::new(".."));
            },
        ];
        for mutate in cases {
            fs::write(generation(&cache), &original).expect("restore generation");
            rewrite_generation(&cache, mutate);
            let loaded = PlanStore::open(&root, &options(&cache)).expect("safe miss");
            assert!(!loaded.stats().cache_generation_hit);
            assert_eq!(loaded.stats().artifacts_parsed, 2);
            assert_eq!(titles(&loaded), [("A-1", "one")]);
        }
    }

    #[test]
    fn configuration_selection_is_part_of_the_cache_key() {
        let (_temporary, root, cache) = fixture();
        let mut first = options(&cache);
        first.configuration_file = Some(root.join("first.yml"));
        PlanStore::open(&root, &first).expect("first selection");
        let mut second = options(&cache);
        second.configuration_file = Some(root.join("second.yml"));
        let selected = PlanStore::open(&root, &second).expect("second selection");
        assert!(!selected.stats().cache_generation_hit);
        assert_eq!(selected.stats().artifacts_parsed, 2);
    }

    #[test]
    fn off_reads_and_writes_no_cache_and_refresh_ignores_a_generation() {
        let (_temporary, root, cache) = fixture();
        let mut off = options(&cache);
        off.mode = CacheMode::Off;
        let mut uncached = PlanStore::open(&root, &off).expect("uncached load");
        assert_eq!(uncached.stats().artifacts_parsed, 2);
        assert!(!cache.exists());
        uncached.refresh().expect("uncached refresh");
        assert_eq!(uncached.stats().artifacts_parsed, 2);
        assert_eq!(titles(&uncached), [("A-1", "one")]);

        PlanStore::open(&root, &options(&cache)).expect("cold cached load");
        let mut refresh = options(&cache);
        refresh.mode = CacheMode::Refresh;
        let refreshed = PlanStore::open(&root, &refresh).expect("refresh");
        assert!(!refreshed.stats().cache_generation_hit);
        assert_eq!(refreshed.stats().artifacts_parsed, 2);
        assert!(refreshed.stats().cache_bytes_written > 0);
    }

    #[test]
    fn a_host_without_exact_writer_identity_never_persists_or_reuses() {
        let (_temporary, root, cache) = fixture();
        let mut unidentified = options(&cache);
        unidentified.writer_identity = None;

        let first = PlanStore::open(&root, &unidentified).expect("first unidentified load");
        assert_eq!(first.stats().artifacts_parsed, 2);
        assert!(!cache.exists());
        assert!(
            first
                .structural_report_key(&aplan_core::Options::default())
                .is_none()
        );

        let second = PlanStore::open(&root, &unidentified).expect("second unidentified load");
        assert!(!second.stats().cache_generation_hit);
        assert_eq!(second.stats().artifacts_reused, 0);
        assert_eq!(second.stats().artifacts_parsed, 2);
        assert_eq!(
            second
                .into_snapshot()
                .plan
                .tasks
                .iter()
                .map(|loaded| loaded.task.id.as_str())
                .collect::<Vec<_>>(),
            ["A-1"]
        );
    }

    #[test]
    fn structural_report_history_is_bounded_to_eight_inputs() {
        let (_temporary, root, cache) = fixture();
        let mut store = PlanStore::open(&root, &options(&cache)).expect("store");
        let report = aplan_core::validate(&store.snapshot().plan, &aplan_core::Options::default());
        for index in 0..9 {
            let mut validation = aplan_core::Options::default();
            validation.owners.insert(format!("owner-{index}"));
            let key = store
                .structural_report_key(&validation)
                .expect("identified report key");
            store.remember_structural_report(&key, &validation, &report);
        }
        assert_eq!(store.reports.len(), 8);
    }

    #[test]
    fn unavailable_cache_storage_is_a_quiet_source_backed_miss() {
        let (temporary, root, _) = fixture();
        let unavailable = temporary.path().join("cache-is-a-file");
        fs::write(&unavailable, b"not a directory").expect("unavailable cache root");

        let loaded = PlanStore::open(&root, &options(&unavailable)).expect("source-backed load");
        assert_eq!(titles(&loaded), [("A-1", "one")]);
        assert!(!loaded.stats().cache_generation_hit);
        assert_eq!(loaded.stats().artifacts_parsed, 2);
        assert_eq!(loaded.stats().cache_bytes_written, 0);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_cache_storage_is_a_quiet_source_backed_miss() {
        use std::os::unix::fs::PermissionsExt as _;

        let (temporary, root, _) = fixture();
        let read_only = temporary.path().join("read-only-cache");
        fs::create_dir(&read_only).expect("cache root");
        fs::set_permissions(&read_only, fs::Permissions::from_mode(0o500))
            .expect("read-only permissions");

        let loaded = PlanStore::open(&root, &options(&read_only)).expect("source-backed load");
        assert_eq!(titles(&loaded), [("A-1", "one")]);
        assert!(!loaded.stats().cache_generation_hit);
        assert_eq!(loaded.stats().cache_bytes_written, 0);

        fs::set_permissions(&read_only, fs::Permissions::from_mode(0o700))
            .expect("restore cleanup permissions");
    }

    #[test]
    fn invalid_utf8_is_never_hidden_by_a_generation() {
        let (_temporary, root, cache) = fixture();
        fs::write(root.join("bad.md"), [0xff, 0xfe]).expect("invalid UTF-8");
        let first = PlanStore::open(&root, &options(&cache)).expect("first load");
        let second = PlanStore::open(&root, &options(&cache)).expect("second load");
        for store in [&first, &second] {
            assert_eq!(store.snapshot().plan.load_diagnostics.len(), 1);
            let diagnostic = &store.snapshot().plan.load_diagnostics[0];
            assert_eq!(diagnostic.code, aplan_core::diagnostic::code::SCHEMA);
            assert_eq!(
                diagnostic.message,
                "cannot read: stream did not contain valid UTF-8"
            );
            assert_eq!(
                diagnostic.agent_instruction.as_deref(),
                Some("Make the file readable, or remove it from the plan directory.")
            );
        }
        assert_eq!(second.stats().artifacts_reused, 2);
        assert_eq!(second.stats().artifacts_parsed, 1);
    }

    #[test]
    fn held_lock_skips_publication_without_affecting_the_answer() {
        let (_temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        let generation = generation(&cache);
        let lock = generation.with_extension("lock");
        fs::write(&lock, "held\n").expect("held lock");
        fs::write(root.join("A-1.md"), task("A-1", "two")).expect("edit");

        let skipped = PlanStore::open(&root, &options(&cache)).expect("load under contention");
        assert_eq!(titles(&skipped), [("A-1", "two")]);
        assert_eq!(skipped.stats().cache_bytes_written, 0);
        let repeated = PlanStore::open(&root, &options(&cache)).expect("repeat old generation");
        assert_eq!(repeated.stats().artifacts_parsed, 1);
        assert_eq!(titles(&repeated), [("A-1", "two")]);

        fs::File::options()
            .write(true)
            .open(&lock)
            .and_then(|file| {
                file.set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            })
            .expect("age held lock");
        let recovered = PlanStore::open(&root, &options(&cache)).expect("stale lock recovery");
        assert!(recovered.stats().cache_bytes_written > 0);
        assert!(!lock.exists());
    }

    #[test]
    fn concurrent_writers_publish_only_complete_generations() {
        let (_temporary, root, cache) = fixture();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let barrier = std::sync::Arc::clone(&barrier);
                let root = root.clone();
                let options = options(&cache);
                std::thread::spawn(move || {
                    barrier.wait();
                    PlanStore::open(&root, &options).expect("concurrent load")
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            let store = thread.join().expect("writer thread");
            assert_eq!(titles(&store), [("A-1", "one")]);
        }
        let published = PlanStore::open(&root, &options(&cache)).expect("published generation");
        assert!(published.stats().cache_generation_hit);
        assert_eq!(published.stats().artifacts_parsed, 0);
    }

    #[cfg(unix)]
    #[test]
    fn cache_files_are_private_and_a_symlinked_namespace_is_a_safe_miss() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let (_temporary, root, cache) = fixture();
        PlanStore::open(&root, &options(&cache)).expect("cold load");
        let metadata = fs::metadata(generation(&cache)).expect("generation metadata");
        assert_eq!(metadata.permissions().mode() & 0o077, 0);

        let aliased_cache = cache.with_file_name("aliased-cache");
        fs::create_dir(&aliased_cache).expect("aliased cache base");
        symlink(cache.join("aplan"), aliased_cache.join("aplan"))
            .expect("populated namespace symlink");
        let aliased = PlanStore::open(&root, &options(&aliased_cache)).expect("aliased safe miss");
        assert!(!aliased.stats().cache_generation_hit);
        assert_eq!(aliased.stats().artifacts_parsed, 2);
        assert_eq!(aliased.stats().cache_bytes_written, 0);

        let unsafe_cache = cache.with_file_name("unsafe-cache");
        let target = cache.with_file_name("cache-target");
        fs::create_dir(&unsafe_cache).expect("cache base");
        fs::create_dir(&target).expect("symlink target");
        symlink(&target, unsafe_cache.join("aplan")).expect("namespace symlink");
        let loaded = PlanStore::open(&root, &options(&unsafe_cache)).expect("safe miss");
        assert_eq!(titles(&loaded), [("A-1", "one")]);
        assert_eq!(loaded.stats().cache_bytes_written, 0);
        assert!(
            fs::read_dir(&target)
                .expect("target listing")
                .next()
                .is_none()
        );

        let unsafe_directory = cache.with_file_name("unsafe-directory");
        fs::create_dir_all(unsafe_directory.join("aplan")).expect("cache namespace");
        let directory_target = cache.with_file_name("directory-target");
        fs::create_dir(&directory_target).expect("directory target");
        symlink(&directory_target, unsafe_directory.join("aplan/plan-v1"))
            .expect("generation directory symlink");
        let loaded =
            PlanStore::open(&root, &options(&unsafe_directory)).expect("directory symlink miss");
        assert_eq!(titles(&loaded), [("A-1", "one")]);
        assert_eq!(loaded.stats().cache_bytes_written, 0);
    }

    #[test]
    fn exact_report_hit_parses_zero_trace_documents_and_trace_changes_miss() {
        let (temporary, root, cache) = fixture();
        fs::write(
            root.join("A-1.md"),
            "---\nid: A-1\ntitle: traced\nphase: 1\nstatus: todo\neffort_days: 1\n\
             blocked_by: []\npaths:\n- src/**\ntraces:\n- DESIGN.md#fact\n---\n",
        )
        .expect("traced task");
        fs::write(temporary.path().join("DESIGN.md"), "# Fact\n\nfirst\n").expect("trace");
        let validation = aplan_core::Options {
            traces_root: Some(temporary.path().to_path_buf()),
            ..aplan_core::Options::default()
        };

        let mut cold = PlanStore::open(&root, &options(&cache)).expect("cold store");
        let key = cold.structural_report_key(&validation).expect("report key");
        assert!(cold.cached_structural_report(&key).is_none());
        let before = aplan_core::trace_document_work();
        let report = aplan_core::validate(&cold.snapshot().plan, &validation);
        let after = aplan_core::trace_document_work();
        assert_eq!(after.reads.saturating_sub(before.reads), 1);
        assert_eq!(
            after.markdown_parses.saturating_sub(before.markdown_parses),
            1
        );
        cold.remember_structural_report(&key, &validation, &report);

        let warm = PlanStore::open(&root, &options(&cache)).expect("warm store");
        let warm_key = warm
            .structural_report_key(&validation)
            .expect("warm report key");
        let before = aplan_core::trace_document_work();
        let cached = warm
            .cached_structural_report(&warm_key)
            .expect("cached report");
        let after = aplan_core::trace_document_work();
        assert_eq!(after.reads.saturating_sub(before.reads), 0);
        assert_eq!(
            cached
                .diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code, diagnostic.message.as_str()))
                .collect::<Vec<_>>(),
            report
                .diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code, diagnostic.message.as_str()))
                .collect::<Vec<_>>()
        );

        let source = fs::read_to_string(root.join("A-1.md")).expect("task source");
        fs::write(
            root.join("A-1.md"),
            source.replace("title: traced", "title: renamed"),
        )
        .expect("title edit");
        let renamed = PlanStore::open(&root, &options(&cache)).expect("renamed store");
        assert_eq!(renamed.stats().artifacts_parsed, 1);
        let renamed_key = renamed
            .structural_report_key(&validation)
            .expect("renamed report key");
        assert_eq!(renamed_key, warm_key);
        assert!(renamed.cached_structural_report(&renamed_key).is_some());

        let trace = temporary.path().join("DESIGN.md");
        let modified = fs::metadata(&trace)
            .and_then(|metadata| metadata.modified())
            .expect("trace modified time");
        fs::write(&trace, "# Fact\n\nother\n").expect("same-size trace edit");
        fs::File::options()
            .write(true)
            .open(&trace)
            .and_then(|file| file.set_times(std::fs::FileTimes::new().set_modified(modified)))
            .expect("restore trace modified time");
        let changed_key = renamed
            .structural_report_key(&validation)
            .expect("changed report key");
        assert_ne!(changed_key, warm_key);
        assert!(renamed.cached_structural_report(&changed_key).is_none());
    }

    #[test]
    fn structural_reports_do_not_cross_root_spellings() {
        let (_temporary, root, cache) = fixture();
        let alternate = root
            .parent()
            .expect("plan parent")
            .join(".")
            .join(root.file_name().expect("plan name"));
        assert_ne!(
            EncodedPath::from_path(&root),
            EncodedPath::from_path(&alternate),
            "the fixture must exercise two lexical spellings of one root"
        );

        let validation = aplan_core::Options::default();
        let first = PlanStore::open(&root, &options(&cache)).expect("first spelling");
        let second = PlanStore::open(&alternate, &options(&cache)).expect("alternate spelling");
        assert_eq!(first.identity_root, second.identity_root);
        assert_ne!(
            first.structural_report_key(&validation),
            second.structural_report_key(&validation),
            "cached diagnostic paths must retain the current invocation's spelling"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_candidate_invalidates_a_preexisting_structural_report() {
        use std::os::unix::fs::PermissionsExt as _;

        let (_temporary, root, cache) = fixture();
        let validation = aplan_core::Options::default();
        let mut cold = PlanStore::open(&root, &options(&cache)).expect("cold store");
        let old_key = cold.structural_report_key(&validation).expect("old key");
        let report = aplan_core::validate(&cold.snapshot().plan, &validation);
        cold.remember_structural_report(&old_key, &validation, &report);

        let unreadable = root.join("unreadable.md");
        fs::write(&unreadable, task("A-2", "hidden")).expect("unreadable candidate");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000))
            .expect("remove read permission");
        let changed = PlanStore::open(&root, &options(&cache)).expect("changed store");
        let changed_key = changed
            .structural_report_key(&validation)
            .expect("changed key");
        assert_ne!(changed_key, old_key);
        assert!(changed.cached_structural_report(&changed_key).is_none());
        assert_eq!(changed.snapshot().plan.load_diagnostics.len(), 1);

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600))
            .expect("restore cleanup permissions");
    }
}
