//! The exact-request/complete-result boundary for batched test execution.
//!
//! Framework output is not evidence. A configured adapter receives a private,
//! bounded manifest and writes one nonce-bound result with exactly one outcome
//! for every requested identity. This module owns that interchange and the
//! filesystem boundary; the resolver owns what each typed outcome means for a
//! citation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use aplan_config::{TestBatch, TestRunner};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Manifest schema emitted by this build.
pub const MANIFEST_SCHEMA: &str = "aplan/test-batch-manifest/v1";
/// Result schema accepted by this build.
pub const RESULT_SCHEMA: &str = "aplan/test-batch-result/v1";

/// The largest exact request one adapter invocation may receive.
pub const MAX_BATCH_IDENTITIES: usize = 100_000;
/// A single exact identity may not make the protocol document unbounded.
pub const MAX_BATCH_NAME_BYTES: usize = 16 * 1024;
/// One adapter diagnostic is useful context, not an unbounded report channel.
pub const MAX_BATCH_DIAGNOSTIC_BYTES: usize = 64 * 1024;
/// The complete manifest or result, before parsing.
pub const MAX_BATCH_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;

/// One exact, deduplicated request to a configured adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestBatchManifest {
    pub schema: String,
    pub nonce: String,
    pub provider: String,
    pub tests: Vec<String>,
}

/// The adapter's typed answer for one requested identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestBatchOutcome {
    /// One execution passed and no retry was hidden.
    Passed,
    /// The execution ran and failed.
    Failed,
    /// The framework selected the identity but did not run it.
    Skipped,
    /// A failure the suite explicitly expected. It is not unconditional proof.
    ExpectedFailure,
    /// A suite expected failure but the test passed. It is not unconditional proof.
    UnexpectedPass,
    /// A later attempt passed after at least one earlier failure.
    RetriedPassed,
    /// Attempts disagreed and the framework classified the identity as flaky.
    Flaky,
    /// Execution was cancelled before a terminal test result.
    Cancelled,
    /// Execution exceeded its framework-owned time limit.
    TimedOut,
    /// Execution crossed a declared memory or process boundary.
    ResourceBreach,
    /// The framework or adapter crashed before an attributable result.
    FrameworkCrash,
    /// The producer explicitly reports that output for this identity is partial.
    Incomplete,
    /// The runner exited successfully, but its output was indistinguishable
    /// from the configured runner's exact-name miss sentinel.
    UnprovenSelection,
    /// Backward-compatible v1 spelling for a selected identity that did not finish.
    Stopped,
    /// Backward-compatible v1 spelling for an identity with no attributable result.
    Unavailable,
}

impl TestBatchOutcome {
    /// Stable protocol spelling used in findings as well as JSON.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::ExpectedFailure => "an expected failure",
            Self::UnexpectedPass => "an unexpected pass",
            Self::RetriedPassed => "passed after retry",
            Self::Flaky => "flaky",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed out",
            Self::ResourceBreach => "stopped at a resource boundary",
            Self::FrameworkCrash => "lost to a framework crash",
            Self::Incomplete => "reported incompletely",
            Self::UnprovenSelection => "did not prove exact selection",
            Self::Stopped => "stopped",
            Self::Unavailable => "unavailable",
        }
    }

    /// Why this outcome cannot satisfy executed evidence.
    #[must_use]
    pub const fn meaning(self) -> &'static str {
        match self {
            Self::Passed => "the adapter reported an unconditional pass",
            Self::Failed => "the requested test failed",
            Self::Skipped => "the adapter reported that the requested test did not run",
            Self::ExpectedFailure => {
                "an expected failure records a known failing test rather than successful proof"
            }
            Self::UnexpectedPass => {
                "an unexpected pass contradicts the suite's declared expectation and needs review"
            }
            Self::RetriedPassed => {
                "a later attempt passed only after an earlier failure, so the result is not unconditional"
            }
            Self::Flaky => "the attempts disagreed, so the result is not deterministic proof",
            Self::Cancelled => "execution was cancelled before a terminal result",
            Self::TimedOut => "execution exceeded its framework-owned time limit",
            Self::ResourceBreach => "execution crossed a declared resource boundary",
            Self::FrameworkCrash => {
                "the framework or adapter crashed before producing an attributable result"
            }
            Self::Incomplete => "the producer declared the per-test output partial",
            Self::UnprovenSelection => {
                "the runner's successful result was indistinguishable from selecting no test"
            }
            Self::Stopped => "the adapter reported that the requested test did not finish",
            Self::Unavailable => {
                "the adapter produced no attributable result for the requested test"
            }
        }
    }

    pub(crate) const fn citation_outcome(self) -> crate::CitationOutcome {
        match self {
            Self::Passed => crate::CitationOutcome::Resolved,
            Self::Cancelled | Self::TimedOut | Self::ResourceBreach | Self::Stopped => {
                crate::CitationOutcome::Refused
            }
            Self::FrameworkCrash
            | Self::Incomplete
            | Self::UnprovenSelection
            | Self::Unavailable => crate::CitationOutcome::Unavailable,
            Self::Failed
            | Self::Skipped
            | Self::ExpectedFailure
            | Self::UnexpectedPass
            | Self::RetriedPassed
            | Self::Flaky => crate::CitationOutcome::Failed,
        }
    }
}

/// One result row. Diagnostics are bounded and sanitized before display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestBatchResultEntry {
    pub test: String,
    pub outcome: TestBatchOutcome,
    /// Number of attempts when the framework retried this identity.
    ///
    /// A plain `passed` outcome may not carry a value greater than one: doing
    /// so would erase the difference between an unconditional pass and a green
    /// produced by retry policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

/// One complete answer to one [`TestBatchManifest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestBatchResult {
    pub schema: String,
    pub nonce: String,
    pub provider: String,
    pub complete: bool,
    pub tests: Vec<TestBatchResultEntry>,
}

/// A validated row handed to the evidence resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportedOutcome {
    pub outcome: TestBatchOutcome,
    pub attempts: Option<u32>,
    pub diagnostic: Option<String>,
}

/// Private invocation state retained until both documents have been checked.
#[derive(Debug)]
pub(crate) struct BatchWorkspace {
    _directory: tempfile::TempDir,
    manifest_path: PathBuf,
    result_path: PathBuf,
    manifest: TestBatchManifest,
    manifest_file: FileIdentity,
    manifest_digest: [u8; 32],
}

impl BatchWorkspace {
    pub(crate) fn create(
        runner_index: usize,
        runner: &TestRunner,
        names: &BTreeSet<String>,
    ) -> Result<Self, String> {
        validate_requested(names)?;
        let directory = tempfile::Builder::new()
            .prefix("aplan-test-batch-")
            .tempdir()
            .map_err(|error| format!("creating private batch directory: {error}"))?;
        set_private_directory(directory.path())?;
        let manifest_path = directory.path().join("manifest.json");
        let result_path = directory.path().join("result.json");
        let manifest = TestBatchManifest {
            schema: MANIFEST_SCHEMA.into(),
            nonce: nonce()?,
            provider: provider_identity(runner_index, runner)?,
            tests: names.iter().cloned().collect(),
        };
        let bytes = encode_manifest(&manifest, MAX_BATCH_DOCUMENT_BYTES)?;
        let mut file = private_create_new(&manifest_path)
            .map_err(|error| format!("creating batch manifest: {error}"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("writing batch manifest: {error}"))?;
        let manifest_file = FileIdentity::from_file(&file)
            .map_err(|error| format!("identifying batch manifest: {error}"))?;
        let manifest_digest = Sha256::digest(&bytes).into();
        Ok(Self {
            _directory: directory,
            manifest_path,
            result_path,
            manifest,
            manifest_file,
            manifest_digest,
        })
    }

    pub(crate) fn argv(&self, batch: &TestBatch) -> Result<Vec<String>, String> {
        let manifest = self
            .manifest_path
            .to_str()
            .ok_or_else(|| "batch manifest path is not valid UTF-8".to_string())?;
        let result = self
            .result_path
            .to_str()
            .ok_or_else(|| "batch result path is not valid UTF-8".to_string())?;
        Ok(batch.run_argv(manifest, result))
    }

    /// Remove invocation-private paths from an error or adapter diagnostic.
    pub(crate) fn redact_paths(&self, text: &str) -> String {
        let text = text.replace(
            &self.manifest_path.to_string_lossy().into_owned(),
            "[batch manifest]",
        );
        text.replace(
            &self.result_path.to_string_lossy().into_owned(),
            "[batch result]",
        )
    }

    pub(crate) fn read_result(&self) -> Result<BTreeMap<String, ReportedOutcome>, String> {
        self.check_manifest()?;
        let bytes = read_stable_private_result(
            &self.result_path,
            self.manifest_file.owner(),
            MAX_BATCH_DOCUMENT_BYTES,
        )?;
        let result: TestBatchResult = serde_json::from_slice(&bytes)
            .map_err(|error| format!("reading batch result JSON: {error}"))?;
        if result.schema != RESULT_SCHEMA {
            return Err(format!(
                "batch result schema must be `{RESULT_SCHEMA}` (found `{}`)",
                result.schema
            ));
        }
        if result.nonce != self.manifest.nonce {
            return Err("batch result nonce does not match this invocation".into());
        }
        if result.provider != self.manifest.provider {
            return Err("batch result provider does not match this runner".into());
        }
        if !result.complete {
            return Err("batch result declares itself incomplete".into());
        }
        if result.tests.len() > MAX_BATCH_IDENTITIES {
            return Err(format!(
                "batch result has more than {MAX_BATCH_IDENTITIES} rows"
            ));
        }
        let expected = self.manifest.tests.iter().cloned().collect::<BTreeSet<_>>();
        let mut outcomes = BTreeMap::new();
        for entry in result.tests {
            validate_name(&entry.test)?;
            validate_result_entry(&entry)?;
            if entry
                .diagnostic
                .as_ref()
                .is_some_and(|text| text.len() > MAX_BATCH_DIAGNOSTIC_BYTES)
            {
                return Err(format!(
                    "one batch diagnostic exceeds {MAX_BATCH_DIAGNOSTIC_BYTES} bytes"
                ));
            }
            let name = entry.test;
            if outcomes
                .insert(
                    name.clone(),
                    ReportedOutcome {
                        outcome: entry.outcome,
                        attempts: entry.attempts,
                        diagnostic: entry.diagnostic,
                    },
                )
                .is_some()
            {
                return Err("batch result repeats a requested test identity".into());
            }
        }
        let actual = outcomes.keys().cloned().collect::<BTreeSet<_>>();
        if actual != expected {
            let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
            let extra = actual.difference(&expected).cloned().collect::<Vec<_>>();
            return Err(format!(
                "batch result does not exactly cover the request ({} missing, {} extra)",
                missing.len(),
                extra.len()
            ));
        }
        Ok(outcomes)
    }

    fn check_manifest(&self) -> Result<(), String> {
        let (bytes, identity) = read_stable_regular(
            &self.manifest_path,
            "batch manifest",
            MAX_BATCH_DOCUMENT_BYTES,
        )?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if identity != self.manifest_file || digest != self.manifest_digest {
            return Err("batch manifest was replaced or changed during the invocation".into());
        }
        Ok(())
    }
}

/// Validate the semantics shared by batch results and portable receipts.
///
/// # Errors
///
/// Returns a bounded protocol error when retry metadata contradicts the typed
/// outcome. This is public so receipt generation cannot grow a second, weaker
/// interpretation of the same result vocabulary.
pub fn validate_result_entry(entry: &TestBatchResultEntry) -> Result<(), String> {
    if entry.attempts == Some(0) {
        return Err("batch result attempts must be positive when present".into());
    }
    match entry.outcome {
        TestBatchOutcome::Passed if entry.attempts.is_some_and(|attempts| attempts > 1) => Err(
            "a retry-created green must use `retried_passed` or `flaky`, not unconditional `passed`"
                .into(),
        ),
        TestBatchOutcome::RetriedPassed | TestBatchOutcome::Flaky
            if entry.attempts.is_none_or(|attempts| attempts < 2) =>
        {
            Err(format!(
                "batch result `{}` requires `attempts` of at least 2",
                match entry.outcome {
                    TestBatchOutcome::RetriedPassed => "retried_passed",
                    TestBatchOutcome::Flaky => "flaky",
                    _ => unreachable!(),
                }
            ))
        }
        _ => Ok(()),
    }
}

fn encode_manifest(manifest: &TestBatchManifest, limit: u64) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| format!("serializing batch manifest: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "batch manifest exceeds {limit} {}",
            byte_unit(limit)
        ));
    }
    Ok(bytes)
}

fn validate_requested(names: &BTreeSet<String>) -> Result<(), String> {
    if names.is_empty() {
        return Err("a batch manifest cannot request zero tests".into());
    }
    if names.len() > MAX_BATCH_IDENTITIES {
        return Err(format!(
            "batch request has more than {MAX_BATCH_IDENTITIES} identities"
        ));
    }
    let mut maximum_encoded_bytes = 256_u64;
    for name in names {
        validate_name(name)?;
        // Measure one bounded identity at a time before allocating the complete
        // document. JSON may spell one input byte as a six-byte `\u00xx`
        // escape, so source byte length alone is not the protocol bound.
        let encoded = serde_json::to_vec(name)
            .map_err(|error| format!("measuring a batch test identity: {error}"))?;
        maximum_encoded_bytes = maximum_encoded_bytes
            .saturating_add(encoded.len() as u64)
            .saturating_add(1);
        if maximum_encoded_bytes > MAX_BATCH_DOCUMENT_BYTES {
            return Err(format!(
                "batch manifest may exceed {MAX_BATCH_DOCUMENT_BYTES} encoded bytes"
            ));
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("batch test identities may not be blank".into());
    }
    if name.len() > MAX_BATCH_NAME_BYTES {
        return Err(format!(
            "batch test identity exceeds {MAX_BATCH_NAME_BYTES} bytes"
        ));
    }
    Ok(())
}

fn provider_identity(runner_index: usize, runner: &TestRunner) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"aplan-test-batch-provider-v1\0");
    digest.update(runner_index.to_le_bytes());
    digest.update(
        serde_json::to_vec(runner)
            .map_err(|error| format!("serializing batch provider identity: {error}"))?,
    );
    Ok(hex(&digest.finalize()))
}

fn nonce() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| format!("creating batch nonce: {error}"))?;
    Ok(hex(&bytes))
}

fn read_stable_private_result(
    path: &Path,
    expected_owner: ExpectedOwner,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("opening batch result: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("batch result is not a regular file".into());
    }
    let (bytes, identity) = read_stable_regular(path, "batch result", limit)?;
    reject_unsafe_result_identity(&identity, expected_owner)?;
    Ok(bytes)
}

fn read_stable_regular(
    path: &Path,
    subject: &str,
    limit: u64,
) -> Result<(Vec<u8>, FileIdentity), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("opening {subject}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{subject} is not a regular file"));
    }
    if metadata.len() > limit {
        return Err(format!("{subject} exceeds {limit} {}", byte_unit(limit)));
    }
    let before = FileIdentity::from_metadata(&metadata);
    let mut file = no_follow_open(path).map_err(|error| format!("opening {subject}: {error}"))?;
    let identity = FileIdentity::from_file(&file)
        .map_err(|error| format!("identifying {subject}: {error}"))?;
    if before != identity {
        return Err(format!("{subject} was replaced before it was read"));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    std::io::Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading {subject}: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{subject} exceeds {limit} {}", byte_unit(limit)));
    }
    let opened_after = FileIdentity::from_file(&file)
        .map_err(|error| format!("rechecking open {subject}: {error}"))?;
    let after =
        fs::symlink_metadata(path).map_err(|error| format!("rechecking {subject}: {error}"))?;
    let after = FileIdentity::from_metadata(&after);
    if identity != opened_after || identity != after {
        return Err(format!(
            "{subject} was replaced or changed while it was read"
        ));
    }
    Ok((bytes, identity))
}

fn byte_unit(count: u64) -> &'static str {
    if count == 1 { "byte" } else { "bytes" }
}

#[cfg(unix)]
fn no_follow_open(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn no_follow_open(path: &Path) -> std::io::Result<fs::File> {
    OpenOptions::new().read(true).open(path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity {
    len: u64,
    modified: Option<SystemTime>,
    readonly: bool,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    links: u64,
    #[cfg(unix)]
    owner: u32,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    changed_seconds: i64,
    #[cfg(unix)]
    changed_nanoseconds: i64,
}

impl FileIdentity {
    fn from_file(file: &fs::File) -> std::io::Result<Self> {
        file.metadata()
            .map(|metadata| Self::from_metadata(&metadata))
    }

    fn from_metadata(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            readonly: metadata.permissions().readonly(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            links: metadata.nlink(),
            #[cfg(unix)]
            owner: metadata.uid(),
            #[cfg(unix)]
            mode: metadata.mode(),
            #[cfg(unix)]
            changed_seconds: metadata.ctime(),
            #[cfg(unix)]
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }

    #[cfg(unix)]
    fn owner(&self) -> u32 {
        self.owner
    }

    #[cfg(not(unix))]
    fn owner(&self) {
        // Ownership is not part of the portable metadata API.
    }
}

#[cfg(unix)]
type ExpectedOwner = u32;
#[cfg(not(unix))]
type ExpectedOwner = ();

#[cfg(unix)]
fn reject_unsafe_result_identity(
    identity: &FileIdentity,
    expected_owner: ExpectedOwner,
) -> Result<(), String> {
    if identity.links != 1 {
        return Err("batch result must not be a hard link".into());
    }
    if identity.owner != expected_owner {
        return Err("batch result owner differs from the aplan-owned manifest".into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn reject_unsafe_result_identity(
    _identity: &FileIdentity,
    _expected_owner: ExpectedOwner,
) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("securing private batch directory: {error}"))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn private_create_new(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_create_new(path: &Path) -> std::io::Result<fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_config::{
        BATCH_MANIFEST_PLACEHOLDER, BATCH_RESULT_PLACEHOLDER, PLACEHOLDER, TEST_BATCH_PROTOCOL,
    };

    fn runner(name: &str) -> TestRunner {
        TestRunner {
            weight: None,
            exclusive: None,
            name: Some(name.into()),
            dir: None,
            covers: None,
            covers_by_test: None,
            inputs_by_test: None,
            skipped: None,
            output_lines: None,
            max_output: None,
            expect_tests: None,
            max_memory: None,
            max_processes: None,
            wrap: None,
            run_selects: None,
            list: vec!["true".into()],
            run: vec!["true".into(), PLACEHOLDER.into()],
            batch: Some(TestBatch {
                protocol: TEST_BATCH_PROTOCOL,
                run: vec![
                    "adapter".into(),
                    BATCH_MANIFEST_PLACEHOLDER.into(),
                    BATCH_RESULT_PLACEHOLDER.into(),
                ],
            }),
            timeout_seconds: None,
        }
    }

    fn names(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn workspace(values: &[&str]) -> BatchWorkspace {
        BatchWorkspace::create(1, &runner("unit"), &names(values)).expect("workspace")
    }

    fn entry(test: &str, outcome: TestBatchOutcome) -> TestBatchResultEntry {
        TestBatchResultEntry {
            test: test.into(),
            outcome,
            attempts: None,
            diagnostic: None,
        }
    }

    fn result(workspace: &BatchWorkspace, tests: Vec<TestBatchResultEntry>) -> TestBatchResult {
        TestBatchResult {
            schema: RESULT_SCHEMA.into(),
            nonce: workspace.manifest.nonce.clone(),
            provider: workspace.manifest.provider.clone(),
            complete: true,
            tests,
        }
    }

    fn write_result(workspace: &BatchWorkspace, result: &TestBatchResult) {
        fs::write(
            &workspace.result_path,
            serde_json::to_vec(result).expect("serialize result"),
        )
        .expect("write result");
    }

    #[test]
    fn a_private_manifest_is_exact_deduplicated_and_starts_without_a_result() {
        let requested = names(&["beta", "alpha", "alpha"]);
        let workspace = BatchWorkspace::create(3, &runner("unit"), &requested).unwrap();
        assert_eq!(workspace.manifest.schema, MANIFEST_SCHEMA);
        assert_eq!(workspace.manifest.tests, ["alpha", "beta"]);
        assert_eq!(workspace.manifest.nonce.len(), 64);
        assert_eq!(workspace.manifest.provider.len(), 64);
        assert!(!workspace.result_path.exists());
        let diagnostic = workspace.redact_paths(&format!(
            "read {} then write {}",
            workspace.manifest_path.display(),
            workspace.result_path.display()
        ));
        assert_eq!(
            diagnostic,
            "read [batch manifest] then write [batch result]"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            assert_eq!(
                fs::metadata(workspace.manifest_path.parent().unwrap())
                    .unwrap()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&workspace.manifest_path).unwrap().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn manifest_encoding_enforces_its_final_document_bound() {
        let workspace = workspace(&["alpha"]);
        let encoded = encode_manifest(&workspace.manifest, MAX_BATCH_DOCUMENT_BYTES)
            .expect("the production bound accepts a valid manifest");
        assert!(!encoded.is_empty());
        assert_eq!(
            encode_manifest(&workspace.manifest, 1).unwrap_err(),
            "batch manifest exceeds 1 byte"
        );
    }

    #[cfg(unix)]
    #[test]
    fn adapter_arguments_refuse_non_utf8_protocol_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        let mut workspace = workspace(&["alpha"]);
        let runner = runner("unit");
        let batch = runner.batch.as_ref().unwrap();
        let valid_manifest = workspace.manifest_path.clone();
        workspace.manifest_path = PathBuf::from(OsString::from_vec(vec![0xff]));
        assert_eq!(
            workspace.argv(batch).unwrap_err(),
            "batch manifest path is not valid UTF-8"
        );

        workspace.manifest_path = valid_manifest;
        workspace.result_path = PathBuf::from(OsString::from_vec(vec![0xff]));
        assert_eq!(
            workspace.argv(batch).unwrap_err(),
            "batch result path is not valid UTF-8"
        );
    }

    /// Proof for `evidence.batch_outcomes_preserve_terminal_semantics`.
    #[test]
    fn batch_retry_and_terminal_outcomes_remain_distinct() {
        let cases = [
            ("cancelled", TestBatchOutcome::Cancelled, None),
            ("expected", TestBatchOutcome::ExpectedFailure, None),
            ("failed", TestBatchOutcome::Failed, None),
            ("flaky", TestBatchOutcome::Flaky, Some(3)),
            ("framework", TestBatchOutcome::FrameworkCrash, None),
            ("incomplete", TestBatchOutcome::Incomplete, None),
            ("passed", TestBatchOutcome::Passed, Some(1)),
            ("resource", TestBatchOutcome::ResourceBreach, None),
            ("retried", TestBatchOutcome::RetriedPassed, Some(2)),
            ("skipped", TestBatchOutcome::Skipped, None),
            ("stopped", TestBatchOutcome::Stopped, None),
            ("timed-out", TestBatchOutcome::TimedOut, None),
            ("unavailable", TestBatchOutcome::Unavailable, None),
            ("unexpected", TestBatchOutcome::UnexpectedPass, None),
            ("unproven", TestBatchOutcome::UnprovenSelection, None),
        ];
        let names = cases.iter().map(|(name, ..)| *name).collect::<Vec<_>>();
        let workspace = workspace(&names);
        let mut failed = entry("failed", TestBatchOutcome::Failed);
        failed.diagnostic = Some("assertion context".into());
        let rows = cases
            .iter()
            .map(|(name, outcome, attempts)| {
                if *name == "failed" {
                    return failed.clone();
                }
                let mut row = entry(name, *outcome);
                row.attempts = *attempts;
                row
            })
            .collect();
        write_result(&workspace, &result(&workspace, rows));
        let outcomes = workspace.read_result().expect("valid complete result");
        assert_eq!(outcomes.len(), cases.len());
        for (name, outcome, attempts) in cases {
            assert_eq!(outcomes[name].outcome, outcome, "{name}");
            assert_eq!(outcomes[name].attempts, attempts, "{name}");
            assert!(!outcome.label().is_empty(), "{name}");
            assert!(!outcome.meaning().is_empty(), "{name}");
        }
        assert_eq!(
            TestBatchOutcome::Passed.citation_outcome(),
            crate::CitationOutcome::Resolved
        );
        assert_eq!(
            TestBatchOutcome::Failed.citation_outcome(),
            crate::CitationOutcome::Failed
        );
        assert_eq!(
            outcomes["failed"].diagnostic.as_deref(),
            Some("assertion context")
        );

        let mut zero = entry("zero", TestBatchOutcome::Failed);
        zero.attempts = Some(0);
        assert!(
            validate_result_entry(&zero)
                .unwrap_err()
                .contains("positive")
        );
        let mut hidden_retry = entry("passed", TestBatchOutcome::Passed);
        hidden_retry.attempts = Some(2);
        assert!(
            validate_result_entry(&hidden_retry)
                .unwrap_err()
                .contains("retried_passed")
        );
        for outcome in [TestBatchOutcome::RetriedPassed, TestBatchOutcome::Flaky] {
            let missing_attempts = entry("retried", outcome);
            assert!(
                validate_result_entry(&missing_attempts)
                    .unwrap_err()
                    .contains("at least 2")
            );
        }
    }

    #[test]
    fn incomplete_missing_extra_duplicate_and_malformed_results_are_refused() {
        let workspace = workspace(&["alpha", "beta"]);
        let cases = [
            result(&workspace, vec![entry("alpha", TestBatchOutcome::Passed)]),
            result(
                &workspace,
                vec![
                    entry("alpha", TestBatchOutcome::Passed),
                    entry("beta", TestBatchOutcome::Passed),
                    entry("extra", TestBatchOutcome::Passed),
                ],
            ),
            result(
                &workspace,
                vec![
                    entry("alpha", TestBatchOutcome::Passed),
                    entry("alpha", TestBatchOutcome::Passed),
                ],
            ),
        ];
        for value in cases {
            write_result(&workspace, &value);
            assert!(workspace.read_result().is_err(), "accepted {value:?}");
        }

        let mut incomplete = result(
            &workspace,
            vec![
                entry("alpha", TestBatchOutcome::Passed),
                entry("beta", TestBatchOutcome::Passed),
            ],
        );
        incomplete.complete = false;
        write_result(&workspace, &incomplete);
        assert!(workspace.read_result().is_err());

        fs::write(&workspace.result_path, b"not JSON").unwrap();
        assert!(workspace.read_result().is_err());
        fs::write(
            &workspace.result_path,
            format!(
                "{{\"schema\":\"{RESULT_SCHEMA}\",\"nonce\":\"{}\",\"provider\":\"{}\",\"complete\":true,\"tests\":[],\"unknown\":true}}",
                workspace.manifest.nonce, workspace.manifest.provider
            ),
        )
        .unwrap();
        assert!(workspace.read_result().is_err());
    }

    #[test]
    fn stale_foreign_and_oversized_results_are_refused_without_leaking_private_paths() {
        let first = workspace(&["alpha"]);
        let second = workspace(&["alpha"]);
        let stale = result(&first, vec![entry("alpha", TestBatchOutcome::Passed)]);
        write_result(&second, &stale);
        assert!(second.read_result().is_err(), "accepted another nonce");

        let mut foreign = result(&second, vec![entry("alpha", TestBatchOutcome::Passed)]);
        foreign.provider = provider_identity(2, &runner("other")).unwrap();
        write_result(&second, &foreign);
        assert!(second.read_result().is_err(), "accepted another provider");

        let mut unknown = result(&second, vec![entry("alpha", TestBatchOutcome::Passed)]);
        unknown.schema = "aplan/test-batch-result/v2".into();
        write_result(&second, &unknown);
        assert!(second.read_result().is_err(), "accepted an unknown schema");

        let mut oversized = result(&second, vec![entry("alpha", TestBatchOutcome::Failed)]);
        oversized.tests[0].diagnostic = Some("x".repeat(MAX_BATCH_DIAGNOSTIC_BYTES + 1));
        write_result(&second, &oversized);
        assert!(second.read_result().is_err(), "accepted oversized detail");

        fs::remove_file(&second.result_path).unwrap();
        let error = second.read_result().expect_err("missing result");
        assert!(!error.contains("aplan-test-batch-"), "leaked path: {error}");
    }

    #[test]
    fn empty_and_oversized_requests_are_refused_before_an_adapter_can_run() {
        assert!(BatchWorkspace::create(1, &runner("unit"), &BTreeSet::new()).is_err());
        assert!(
            BatchWorkspace::create(
                1,
                &runner("unit"),
                &names(&[&"x".repeat(MAX_BATCH_NAME_BYTES + 1)]),
            )
            .is_err()
        );
        let large = (0..400)
            .map(|index| format!("{index:03}{}", "\u{1}".repeat(14 * 1024)))
            .collect::<BTreeSet<_>>();
        assert!(
            BatchWorkspace::create(1, &runner("unit"), &large).is_err(),
            "the encoded document bound must be checked before serialization"
        );

        let workspace = workspace(&["alpha"]);
        fs::File::create(&workspace.result_path)
            .unwrap()
            .set_len(MAX_BATCH_DOCUMENT_BYTES + 1)
            .unwrap();
        assert!(workspace.read_result().is_err());
    }

    #[test]
    fn request_and_result_cardinality_bounds_are_enforced() {
        let too_many = (0..=MAX_BATCH_IDENTITIES)
            .map(|index| format!("test-{index}"))
            .collect::<BTreeSet<_>>();
        let error = BatchWorkspace::create(1, &runner("unit"), &too_many)
            .expect_err("an oversized request must be rejected");
        assert!(error.contains("more than 100000 identities"), "{error}");

        let workspace = workspace(&["alpha"]);
        let too_many = (0..=MAX_BATCH_IDENTITIES)
            .map(|index| entry(&format!("test-{index}"), TestBatchOutcome::Passed))
            .collect();
        write_result(&workspace, &result(&workspace, too_many));
        let error = workspace
            .read_result()
            .expect_err("an oversized result must be rejected");
        assert!(error.contains("more than 100000 rows"), "{error}");
    }

    #[test]
    fn blank_and_oversized_result_identities_are_refused() {
        let workspace = workspace(&["alpha"]);
        for invalid in [" \t".to_string(), "x".repeat(MAX_BATCH_NAME_BYTES + 1)] {
            write_result(
                &workspace,
                &result(&workspace, vec![entry(&invalid, TestBatchOutcome::Passed)]),
            );
            assert!(
                workspace.read_result().is_err(),
                "accepted invalid result identity of {} bytes",
                invalid.len()
            );
        }
    }

    #[test]
    fn stable_file_reader_refuses_non_files_and_metadata_oversize() {
        let directory = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let error = read_stable_regular(directory.path(), "fixture", 1)
            .expect_err("a directory is not a protocol document");
        assert_eq!(error, "fixture is not a regular file");

        let oversized = directory.path().join("oversized");
        fs::File::create(&oversized)
            .expect("create oversized fixture")
            .set_len(2)
            .expect("size oversized fixture");
        let error = read_stable_regular(&oversized, "fixture", 1)
            .expect_err("metadata beyond the bound must be rejected");
        assert_eq!(error, "fixture exceeds 1 byte");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stable_file_reader_bounds_a_pseudo_file_whose_metadata_reports_zero() {
        let error = read_stable_regular(Path::new("/proc/self/cmdline"), "fixture", 0)
            .expect_err("the read bound, not only metadata, must be enforced");
        assert_eq!(error, "fixture exceeds 0 bytes");
    }

    #[test]
    fn changing_or_replacing_the_manifest_is_refused() {
        let workspace = workspace(&["alpha"]);
        fs::write(&workspace.manifest_path, b"{}").unwrap();
        write_result(
            &workspace,
            &result(&workspace, vec![entry("alpha", TestBatchOutcome::Passed)]),
        );
        assert!(workspace.read_result().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_and_hard_link_results_are_refused() {
        use std::os::unix::fs::symlink;

        let workspace = workspace(&["alpha"]);
        let target = workspace.result_path.with_file_name("target.json");
        fs::write(
            &target,
            serde_json::to_vec(&result(
                &workspace,
                vec![entry("alpha", TestBatchOutcome::Passed)],
            ))
            .unwrap(),
        )
        .unwrap();
        symlink(&target, &workspace.result_path).unwrap();
        assert!(workspace.read_result().is_err(), "accepted a symbolic link");

        fs::remove_file(&workspace.result_path).unwrap();
        fs::hard_link(&target, &workspace.result_path).unwrap();
        assert!(workspace.read_result().is_err(), "accepted a hard link");
    }

    #[cfg(unix)]
    #[test]
    fn a_result_owned_by_another_identity_is_refused() {
        let workspace = workspace(&["alpha"]);
        let mut identity = workspace.manifest_file.clone();
        identity.owner = identity.owner.wrapping_add(1);
        let error = reject_unsafe_result_identity(&identity, workspace.manifest_file.owner())
            .expect_err("a foreign owner must be rejected");
        assert_eq!(
            error,
            "batch result owner differs from the aplan-owned manifest"
        );
    }
}
