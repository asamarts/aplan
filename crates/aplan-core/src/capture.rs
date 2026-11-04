//! Versioned capture records: durable intake that is deliberately not a task.
//!
//! Captures live below `<plan-root>/.inbox/`.  This module owns their closed
//! schema, strict loader, cross-record validation and read projections.  It
//! does not expose a conversion to [`crate::Task`]: promotion is an application
//! transaction, while loading a capture must never put it in the dependency
//! graph by accident (ADR-0063, `A1-514`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, code};
use crate::task::is_vacuous_ref;
use crate::validate::rfc3339_to_epoch;

/// The reserved directory below the resolved plan root.
pub const INBOX_DIR: &str = ".inbox";
/// The only capture document version this build accepts.
pub const CAPTURE_SCHEMA: &str = "aplan/capture/v1";
/// Kinds every repository accepts, whether or not it has configuration.
pub const BUILTIN_KINDS: &[&str] = &["idea", "note", "task"];
/// Maximum bytes accepted for an opaque external source key.
pub const MAX_SOURCE_KEY_BYTES: usize = 1_024;
/// Maximum bytes in one capture file or body.
pub const MAX_CAPTURE_BYTES: usize = 1_048_576;
/// Maximum bytes in a capture title.
pub const MAX_TITLE_BYTES: usize = 512;
/// Maximum labels on one capture.
pub const MAX_LABELS: usize = 64;
/// Maximum bytes accepted for a list search expression.
pub const MAX_SEARCH_BYTES: usize = 256;
/// Maximum number of records returned by one list projection.
pub const MAX_LIST_LIMIT: usize = 100;

/// A clock supplied by the application at the operation boundary.
pub trait Clock {
    /// Current Unix time in whole seconds.
    fn now_epoch(&self) -> i64;
}

/// The ordinary process clock. Tests use [`FixedClock`] instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_epoch(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
            })
    }
}

/// A deterministic operation clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_epoch(&self) -> i64 {
        self.0
    }
}

/// The closed lifecycle vocabulary for intake records.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Open,
    Promoted,
    Discarded,
}

/// The immutable adapter that first wrote a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureOrigin {
    Cli,
    Mcp,
    Tui,
}

/// The closed on-disk capture schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CaptureSchema {
    #[serde(rename = "aplan/capture/v1")]
    V1,
}

/// Optional caller identity for retry-safe creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureSource {
    /// A portable lower-case dotted or kebab-case namespace.
    pub namespace: String,
    /// Opaque repository data. It is shown only by focused reads.
    #[schemars(length(min = 1, max = 1024))]
    pub key: String,
}

/// Task-creation defaults snapshotted onto a capture.
///
/// These are inert data until promotion. They contain no status, dependency,
/// evidence, trace or command field, so a profile cannot weaken task policy by
/// construction.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromotionDefaults {
    /// Default task phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<u32>,
    /// Default destination cut id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<String>,
    /// Default positive finite effort estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_days: Option<f64>,
    /// Default task owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Default task path globs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Default task tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl PromotionDefaults {
    /// Overlay explicit values on profile defaults.
    #[must_use]
    pub fn overlay(&self, explicit: &Self) -> Self {
        Self {
            phase: explicit.phase.or(self.phase),
            cut: explicit.cut.clone().or_else(|| self.cut.clone()),
            effort_days: explicit.effort_days.or(self.effort_days),
            owner: explicit.owner.clone().or_else(|| self.owner.clone()),
            paths: if explicit.paths.is_empty() {
                self.paths.clone()
            } else {
                explicit.paths.clone()
            },
            tags: if explicit.tags.is_empty() {
                self.tags.clone()
            } else {
                explicit.tags.clone()
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.phase.is_none()
            && self.cut.is_none()
            && self.effort_days.is_none()
            && self.owner.is_none()
            && self.paths.is_empty()
            && self.tags.is_empty()
    }
}

/// Portable repository defaults for capture creation and later promotion.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureProfile {
    /// Default capture kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Default capture labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Default body scaffold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Defaults snapshotted for later task promotion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion: Option<PromotionDefaults>,
}

/// One versioned capture file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    /// Always `aplan/capture/v1`.
    pub schema: CaptureSchema,
    /// `I-` followed by one canonical Crockford ULID.
    pub id: String,
    /// Non-vacuous intake title.
    #[schemars(length(min = 1, max = 512))]
    pub title: String,
    /// Built-in or repository-declared organizational kind.
    pub kind: String,
    /// Closed capture lifecycle state.
    pub state: CaptureState,
    /// UTC RFC 3339, whole-second precision.
    pub captured_at: String,
    /// Immutable adapter of origin.
    pub origin: CaptureOrigin,
    /// Optional intake detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 1_048_576))]
    pub body: Option<String>,
    /// Sorted unique non-vacuous intake labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 64))]
    pub labels: Vec<String>,
    /// Optional UTC instant before which an open capture is deferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revisit_at: Option<String>,
    /// Explicit applied repository profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Effective task-promotion defaults frozen at creation or amendment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion_defaults: Option<PromotionDefaults>,
    /// Optional namespaced retry identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CaptureSource>,
    /// UTC whole-second instant at which this record left `open`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposed_at: Option<String>,
    /// Optional disposition explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Resulting task id, required only for `promoted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_to: Option<String>,
    /// Canonical promotion-request digest, required only for `promoted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion_fingerprint: Option<String>,
}

/// A valid capture paired with the file that supplied it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedCapture {
    pub capture: Capture,
    pub path: PathBuf,
}

/// Repository policy needed to interpret capture kinds and profiles.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturePolicy {
    kinds: BTreeSet<String>,
    profiles: BTreeMap<String, CaptureProfile>,
}

impl Default for CapturePolicy {
    fn default() -> Self {
        Self {
            kinds: BUILTIN_KINDS
                .iter()
                .map(|kind| (*kind).to_string())
                .collect(),
            profiles: BTreeMap::new(),
        }
    }
}

impl CapturePolicy {
    /// Validate and construct repository capture policy.
    ///
    /// # Errors
    /// When a kind/profile/default is vacuous, duplicated or outside the closed
    /// portable syntax.
    pub fn try_new(
        additional_kinds: &[String],
        profiles: &BTreeMap<String, CaptureProfile>,
    ) -> Result<Self, String> {
        let mut policy = Self::default();
        for kind in additional_kinds {
            validate_key(kind, KeySyntax::Kebab)
                .map_err(|reason| format!("capture kind `{kind}` {reason}"))?;
            if !policy.kinds.insert(kind.clone()) {
                return Err(format!("capture kind `{kind}` is declared more than once"));
            }
        }
        for (name, profile) in profiles {
            validate_key(name, KeySyntax::Kebab)
                .map_err(|reason| format!("capture profile `{name}` {reason}"))?;
            validate_profile(name, profile, &policy.kinds)?;
        }
        policy.profiles.clone_from(profiles);
        Ok(policy)
    }

    #[must_use]
    pub fn kinds(&self) -> &BTreeSet<String> {
        &self.kinds
    }

    #[must_use]
    pub fn profiles(&self) -> &BTreeMap<String, CaptureProfile> {
        &self.profiles
    }
}

fn validate_profile(
    name: &str,
    profile: &CaptureProfile,
    kinds: &BTreeSet<String>,
) -> Result<(), String> {
    if let Some(kind) = &profile.kind
        && !kinds.contains(kind)
    {
        return Err(format!(
            "capture profile `{name}` names unknown kind `{kind}`; declare it in `capture.kinds`"
        ));
    }
    validate_values("labels", &profile.labels, false)
        .map_err(|reason| format!("capture profile `{name}` {reason}"))?;
    if profile.labels.len() > MAX_LABELS {
        return Err(format!(
            "capture profile `{name}` has more than {MAX_LABELS} labels"
        ));
    }
    if let Some(body) = &profile.body {
        if body.len() > MAX_CAPTURE_BYTES {
            return Err(format!(
                "capture profile `{name}` body exceeds the {MAX_CAPTURE_BYTES}-byte limit"
            ));
        }
        if is_vacuous_ref(body) {
            return Err(format!(
                "capture profile `{name}` body is blank or invisible"
            ));
        }
        validate_text(body, TextKind::Body)
            .map_err(|reason| format!("capture profile `{name}` body {reason}"))?;
    }
    if let Some(promotion) = &profile.promotion {
        validate_promotion_defaults(promotion)
            .map_err(|reason| format!("capture profile `{name}` promotion {reason}"))?;
    }
    Ok(())
}

fn validate_promotion_defaults(defaults: &PromotionDefaults) -> Result<(), String> {
    if let Some(effort) = defaults.effort_days
        && (!effort.is_finite() || effort <= 0.0)
    {
        return Err("`effort_days` must be a positive finite number".into());
    }
    for (field, value) in [
        ("cut", defaults.cut.as_deref()),
        ("owner", defaults.owner.as_deref()),
    ] {
        if value.is_some_and(is_vacuous_ref) {
            return Err(format!("`{field}` is blank or invisible"));
        }
        if let Some(value) = value {
            validate_text(value, TextKind::Scalar)
                .map_err(|reason| format!("`{field}` {reason}"))?;
        }
    }
    validate_values("paths", &defaults.paths, false)?;
    validate_values("tags", &defaults.tags, false)?;
    Ok(())
}

/// A creation request before profile defaults have been resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureCreate {
    pub title: String,
    pub kind: Option<String>,
    pub body: Option<String>,
    /// `None` means inherit profile labels; `Some([])` explicitly clears them.
    pub labels: Option<Vec<String>>,
    pub revisit_at: Option<String>,
    pub profile: Option<String>,
    pub promotion_defaults: Option<PromotionDefaults>,
    pub source: Option<CaptureSource>,
    pub origin: CaptureOrigin,
}

/// The exact persisted creation content, excluding generated identity and time.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveCapture {
    pub title: String,
    pub kind: String,
    pub body: Option<String>,
    pub labels: Vec<String>,
    pub revisit_at: Option<String>,
    pub profile: Option<String>,
    pub promotion_defaults: Option<PromotionDefaults>,
    pub source: Option<CaptureSource>,
    pub origin: CaptureOrigin,
}

impl CaptureCreate {
    /// Resolve a profile and explicit overrides to the bytes creation will store.
    ///
    /// # Errors
    /// When the named profile/kind is unknown or effective content is invalid.
    pub fn effective(&self, policy: &CapturePolicy) -> Result<EffectiveCapture, String> {
        let profile = self
            .profile
            .as_ref()
            .map(|name| {
                policy
                    .profiles
                    .get(name)
                    .ok_or_else(|| format!("capture profile `{name}` is not declared"))
            })
            .transpose()?;
        let kind = self
            .kind
            .clone()
            .or_else(|| profile.and_then(|value| value.kind.clone()))
            .unwrap_or_else(|| "task".to_string());
        if !policy.kinds.contains(&kind) {
            return Err(format!("capture kind `{kind}` is not declared"));
        }
        let body = self
            .body
            .clone()
            .or_else(|| profile.and_then(|value| value.body.clone()));
        let mut labels = self
            .labels
            .clone()
            .unwrap_or_else(|| profile.map_or_else(Vec::new, |value| value.labels.clone()));
        labels.sort();
        labels.dedup();
        let promotion = match (
            profile.and_then(|value| value.promotion.as_ref()),
            self.promotion_defaults.as_ref(),
        ) {
            (Some(base), Some(explicit)) => Some(base.overlay(explicit)),
            (Some(base), None) => Some(base.clone()),
            (None, Some(explicit)) => Some(explicit.clone()),
            (None, None) => None,
        }
        .filter(|defaults| !defaults.is_empty());
        let effective = EffectiveCapture {
            title: self.title.clone(),
            kind,
            body,
            labels,
            revisit_at: self.revisit_at.clone(),
            profile: self.profile.clone(),
            promotion_defaults: promotion,
            source: self.source.clone(),
            origin: self.origin,
        };
        effective.validate()?;
        Ok(effective)
    }
}

impl EffectiveCapture {
    /// Validate effective persisted creation or amendment content.
    pub fn validate(&self) -> Result<(), String> {
        if is_vacuous_ref(&self.title) {
            return Err("capture title is blank or invisible".into());
        }
        if self.title.len() > MAX_TITLE_BYTES {
            return Err(format!(
                "capture title exceeds the {MAX_TITLE_BYTES}-byte limit"
            ));
        }
        validate_text(&self.title, TextKind::Scalar)
            .map_err(|reason| format!("capture title {reason}"))?;
        if let Some(body) = &self.body {
            if body.len() > MAX_CAPTURE_BYTES {
                return Err(format!(
                    "capture body exceeds the {MAX_CAPTURE_BYTES}-byte limit"
                ));
            }
            if is_vacuous_ref(body) {
                return Err("capture body is blank or invisible".into());
            }
            validate_text(body, TextKind::Body)
                .map_err(|reason| format!("capture body {reason}"))?;
        }
        if self.labels.len() > MAX_LABELS {
            return Err(format!("capture has more than {MAX_LABELS} labels"));
        }
        validate_values("labels", &self.labels, true)?;
        if let Some(at) = &self.revisit_at {
            parse_capture_instant(at).ok_or_else(|| {
                "`revisit_at` must be UTC RFC 3339 at whole-second precision".to_string()
            })?;
        }
        if let Some(profile) = &self.profile {
            validate_key(profile, KeySyntax::Kebab)
                .map_err(|reason| format!("capture profile `{profile}` {reason}"))?;
        }
        if let Some(defaults) = &self.promotion_defaults {
            validate_promotion_defaults(defaults)?;
        }
        if let Some(source) = &self.source {
            validate_source(source)?;
        }
        Ok(())
    }

    fn agrees_with(&self, capture: &Capture) -> bool {
        self.title == capture.title
            && self.kind == capture.kind
            && self.body == capture.body
            && self.labels == capture.labels
            && self.revisit_at == capture.revisit_at
            && self.profile == capture.profile
            && self.promotion_defaults == capture.promotion_defaults
            && self.source == capture.source
            && self.origin == capture.origin
    }
}

/// Serialize one capture through the repository's pinned YAML contract.
///
/// # Errors
/// When the typed record cannot be represented by the pinned serializer.
pub fn render_capture(capture: &Capture) -> Result<String, String> {
    yaml_compat::to_string(capture).map_err(|error| error.to_string())
}

/// Validate one application-constructed record against the same semantic
/// checks the V1 loader applies.
///
/// # Errors
/// Every semantic finding, in deterministic validator order.
pub fn validate_capture_record(
    capture: &Capture,
    policy: &CapturePolicy,
    clock: &impl Clock,
) -> Result<(), Vec<String>> {
    let errors = validate_capture(capture, &capture.id, policy, clock.now_epoch());
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Read one bounded regular UTF-8 body source without following a symlink or
/// blocking on a substituted FIFO on Unix.
///
/// # Errors
/// When the path is not a stable regular file, exceeds the capture bound, is
/// not UTF-8 or cannot be read.
pub fn read_capture_body_file(path: &Path) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspecting {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("opening {} safely: {error}", path.display()))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspecting opened {}: {error}", path.display()))?;
    if !opened.file_type().is_file() {
        return Err(format!(
            "{} stopped naming a regular file while it was opened",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    file.take(u64::try_from(MAX_CAPTURE_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err(format!(
            "{} exceeds the {MAX_CAPTURE_BYTES}-byte capture body limit",
            path.display()
        ));
    }
    String::from_utf8(bytes).map_err(|error| {
        format!(
            "{} is not UTF-8 (invalid byte at offset {})",
            path.display(),
            error.utf8_error().valid_up_to()
        )
    })
}

/// Result of looking up optional external retry identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceResolution {
    NoSource,
    Vacant,
    Equivalent {
        id: String,
        state: CaptureState,
    },
    Conflict {
        id: String,
        state: CaptureState,
        namespace: String,
    },
}

/// A complete deterministic inbox snapshot.
#[derive(Debug, Clone, Default)]
pub struct Inbox {
    pub captures: Vec<LoadedCapture>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Inbox {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Resolve a source retry against persisted effective values.
    ///
    /// # Errors
    /// When any inbox entry failed validation, so choosing one retained record
    /// would treat a partial snapshot as complete.
    pub fn resolve_source(&self, proposed: &EffectiveCapture) -> Result<SourceResolution, String> {
        self.ensure_clean()?;
        let Some(source) = &proposed.source else {
            return Ok(SourceResolution::NoSource);
        };
        let Some(existing) = self
            .captures
            .iter()
            .find(|loaded| loaded.capture.source.as_ref() == Some(source))
        else {
            return Ok(SourceResolution::Vacant);
        };
        if proposed.agrees_with(&existing.capture) {
            Ok(SourceResolution::Equivalent {
                id: existing.capture.id.clone(),
                state: existing.capture.state,
            })
        } else {
            Ok(SourceResolution::Conflict {
                id: existing.capture.id.clone(),
                state: existing.capture.state,
                namespace: source.namespace.clone(),
            })
        }
    }

    /// Focused lookup. This is the read that may expose the full source pair.
    ///
    /// # Errors
    /// When the inbox has any validation finding.
    pub fn show(&self, id: &str) -> Result<Option<&Capture>, String> {
        self.ensure_clean()?;
        Ok(self
            .captures
            .iter()
            .find(|loaded| loaded.capture.id == id)
            .map(|loaded| &loaded.capture))
    }

    /// Resolve one human-entered canonical leading id substring.
    ///
    /// Full ids use the same path, while zero and multiple matches remain
    /// distinct refusals. Machine adapters should call [`Self::show`] with a
    /// separately validated full id instead.
    pub fn resolve_prefix(&self, prefix: &str) -> Result<&LoadedCapture, String> {
        self.ensure_clean()?;
        if !prefix.starts_with("I-")
            || prefix.len() <= 2
            || prefix.len() > 28
            || !prefix[2..]
                .bytes()
                .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte))
        {
            return Err(
                "a capture id prefix must begin `I-` and continue with canonical upper-case Crockford characters"
                    .to_string(),
            );
        }
        let mut matches = self
            .captures
            .iter()
            .filter(|loaded| loaded.capture.id.starts_with(prefix));
        let Some(found) = matches.next() else {
            return Err(format!("no capture matches prefix `{prefix}`"));
        };
        if matches.next().is_some() {
            return Err(format!(
                "capture prefix `{prefix}` is ambiguous; provide more of the id"
            ));
        }
        Ok(found)
    }

    /// Build a bounded deterministic list projection at one injected instant.
    ///
    /// # Errors
    /// When a filter would make the operation unbounded or ambiguous.
    pub fn list(&self, query: &CaptureQuery, clock: &impl Clock) -> Result<CaptureList, String> {
        self.ensure_clean()?;
        query.validate()?;
        let now = clock.now_epoch();
        let needle = query.search.as_ref().map(|text| text.to_lowercase());
        let mut matching = Vec::new();
        let mut deferred = 0usize;
        for loaded in &self.captures {
            let capture = &loaded.capture;
            if query.state.is_some_and(|state| state != capture.state)
                || query
                    .kind
                    .as_ref()
                    .is_some_and(|kind| kind != &capture.kind)
                || query
                    .label
                    .as_ref()
                    .is_some_and(|label| !capture.labels.contains(label))
                || needle.as_ref().is_some_and(|needle| {
                    !capture.title.to_lowercase().contains(needle)
                        && !capture
                            .body
                            .as_ref()
                            .is_some_and(|body| body.to_lowercase().contains(needle))
                })
            {
                continue;
            }
            let is_deferred = capture.state == CaptureState::Open
                && capture
                    .revisit_at
                    .as_deref()
                    .and_then(parse_capture_instant)
                    .is_some_and(|at| at > now);
            if is_deferred {
                deferred += 1;
            }
            let revisit_matches = match query.revisit {
                RevisitFilter::Actionable => !is_deferred,
                RevisitFilter::Deferred => is_deferred,
                RevisitFilter::All => true,
            };
            if revisit_matches {
                matching.push(capture);
            }
        }
        matching.sort_by(|left, right| {
            parse_capture_instant(&right.captured_at)
                .cmp(&parse_capture_instant(&left.captured_at))
                .then(left.id.cmp(&right.id))
        });
        matching.truncate(query.limit);
        Ok(CaptureList {
            evaluated_at: format_capture_instant(now),
            deferred,
            captures: matching.into_iter().map(CaptureSummary::from).collect(),
        })
    }

    fn ensure_clean(&self) -> Result<(), String> {
        if self.diagnostics.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "the capture inbox has {} validation finding(s); repair it before reading or mutating a partial snapshot",
                self.diagnostics.len()
            ))
        }
    }
}

/// Revisit eligibility for a list projection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RevisitFilter {
    /// Exclude open records whose revisit instant is still in the future.
    #[default]
    Actionable,
    Deferred,
    All,
}

/// Bounded list filters. The default is the actionable open inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureQuery {
    pub kind: Option<String>,
    pub state: Option<CaptureState>,
    pub label: Option<String>,
    pub revisit: RevisitFilter,
    pub search: Option<String>,
    pub limit: usize,
}

impl Default for CaptureQuery {
    fn default() -> Self {
        Self {
            kind: None,
            state: Some(CaptureState::Open),
            label: None,
            revisit: RevisitFilter::Actionable,
            search: None,
            limit: 50,
        }
    }
}

impl CaptureQuery {
    pub fn validate(&self) -> Result<(), String> {
        if self.limit == 0 || self.limit > MAX_LIST_LIMIT {
            return Err(format!(
                "capture list limit must be between 1 and {MAX_LIST_LIMIT}"
            ));
        }
        if let Some(search) = &self.search {
            if is_vacuous_ref(search) {
                return Err("capture search is blank or invisible".into());
            }
            if search.len() > MAX_SEARCH_BYTES {
                return Err(format!(
                    "capture search exceeds the {MAX_SEARCH_BYTES}-byte limit"
                ));
            }
            validate_text(search, TextKind::Scalar)
                .map_err(|reason| format!("capture search {reason}"))?;
        }
        if let Some(label) = &self.label {
            validate_values("label filter", std::slice::from_ref(label), false)?;
        }
        Ok(())
    }
}

/// Non-sensitive fields returned by an inbox list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureSummary {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub state: CaptureState,
    pub captured_at: String,
    pub labels: Vec<String>,
    pub revisit_at: Option<String>,
    pub profile: Option<String>,
    /// External retry namespace, without its opaque key.
    pub source_namespace: Option<String>,
    pub promoted_to: Option<String>,
}

impl From<&Capture> for CaptureSummary {
    fn from(capture: &Capture) -> Self {
        Self {
            id: capture.id.clone(),
            title: capture.title.clone(),
            kind: capture.kind.clone(),
            state: capture.state,
            captured_at: capture.captured_at.clone(),
            labels: capture.labels.clone(),
            revisit_at: capture.revisit_at.clone(),
            profile: capture.profile.clone(),
            source_namespace: capture
                .source
                .as_ref()
                .map(|source| source.namespace.clone()),
            promoted_to: capture.promoted_to.clone(),
        }
    }
}

/// Machine metadata and rows for one evaluated list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureList {
    pub evaluated_at: String,
    pub deferred: usize,
    pub captures: Vec<CaptureSummary>,
}

/// Load and validate the inbox below `root` at one injected instant.
#[must_use]
pub fn load_inbox(root: &Path, policy: &CapturePolicy, clock: &impl Clock) -> Inbox {
    let directory = root.join(INBOX_DIR);
    let entries = match inbox_entries(&directory) {
        Ok(Some(entries)) => entries,
        Ok(None) => return Inbox::default(),
        Err(diagnostic) => return invalid_inbox(*diagnostic),
    };

    let mut inbox = Inbox::default();
    let now = clock.now_epoch();
    for entry in entries {
        load_inbox_entry(&entry, policy, now, &mut inbox);
    }
    validate_cross_record(&mut inbox);
    inbox
        .captures
        .sort_by(|left, right| left.capture.id.cmp(&right.capture.id));
    inbox.diagnostics.sort_by(|left, right| {
        left.code
            .cmp(right.code)
            .then(left.file.cmp(&right.file))
            .then(left.message.cmp(&right.message))
    });
    inbox
}

fn invalid_inbox(diagnostic: Diagnostic) -> Inbox {
    Inbox {
        captures: Vec::new(),
        diagnostics: vec![diagnostic],
    }
}

fn inbox_entries(directory: &Path) -> Result<Option<Vec<std::fs::DirEntry>>, Box<Diagnostic>> {
    let metadata = match std::fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(capture_load_error(
                directory,
                format!("cannot inspect the capture inbox: {error}"),
                "Make `.inbox` readable, or remove it so an empty inbox can be used.",
            ));
        }
    };
    if !metadata.file_type().is_dir() {
        return Err(capture_load_error(
            directory,
            "the reserved `.inbox` path is not a directory",
            "Replace `.inbox` with a directory, or remove it for an empty inbox.",
        ));
    }
    let mut entries: Vec<_> = std::fs::read_dir(directory)
        .and_then(Iterator::collect)
        .map_err(|error| {
            capture_load_error(
                directory,
                format!("cannot enumerate the capture inbox: {error}"),
                "Make `.inbox` readable, or remove it so an empty inbox can be used.",
            )
        })?;
    // The guarded writer stages beside its destination so the final rename is
    // atomic. Those names are neither records nor user files, and a concurrent
    // read must not turn an in-flight or crash-left staging artifact into an
    // `APLAN057` finding. The pattern is deliberately exact: another hidden
    // file remains invalid and visible.
    entries.retain(|entry| !is_capture_staging_name(&entry.file_name()));
    entries.sort_by_key(std::fs::DirEntry::file_name);
    Ok(Some(entries))
}

/// Whether a directory member is the guarded writer's exact staging pattern,
/// and therefore not repository capture data.
#[must_use]
pub fn is_capture_staging_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some((id, number)) = rest.split_once(".yml.tmp") else {
        return false;
    };
    is_capture_id(id) && !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
}

fn load_inbox_entry(
    entry: &std::fs::DirEntry,
    policy: &CapturePolicy,
    now: i64,
    inbox: &mut Inbox,
) {
    let (path, id_from_name) = match capture_entry_identity(entry) {
        Ok(identity) => identity,
        Err(diagnostic) => {
            inbox.diagnostics.push(*diagnostic);
            return;
        }
    };
    let capture = match read_capture(&path) {
        Ok(capture) => capture,
        Err(diagnostic) => {
            inbox.diagnostics.push(*diagnostic);
            return;
        }
    };
    let errors = validate_capture(&capture, &id_from_name, policy, now);
    if errors.is_empty() {
        inbox.captures.push(LoadedCapture { capture, path });
    } else {
        inbox.diagnostics.extend(errors.into_iter().map(|error| {
            capture_diagnostic(
                &path,
                error,
                "Repair the capture so its id, timestamps, configured kind and lifecycle fields agree.",
            )
        }));
    }
}

fn capture_entry_identity(entry: &std::fs::DirEntry) -> Result<(PathBuf, String), Box<Diagnostic>> {
    let path = entry.path();
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| {
        capture_load_error(
            &path,
            "capture entry disappeared while the inbox was being read",
            "Retry after the concurrent edit finishes; every inbox entry must be stable and readable.",
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(capture_load_error(
            &path,
            "the capture inbox contains an entry that is not a regular file",
            "Keep only canonical `I-<ULID>.yml` regular files directly below `.inbox`.",
        ));
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            capture_load_error(
                &path,
                "the capture filename is not valid UTF-8",
                "Rename it to the record's canonical `I-<ULID>.yml` filename.",
            )
        })?;
    let id = file_name.strip_suffix(".yml").ok_or_else(|| {
        capture_load_error(
            &path,
            "the capture filename does not end in the canonical lower-case `.yml` suffix",
            "Rename it to the record's canonical `I-<ULID>.yml` filename.",
        )
    })?;
    if !is_capture_id(id) {
        return Err(capture_load_error(
            &path,
            "the capture filename does not contain a canonical `I-<ULID>` id",
            "Rename the file and its `id` to one canonical `I-`-prefixed Crockford ULID.",
        ));
    }
    let id = id.to_string();
    if metadata.len() > u64::try_from(MAX_CAPTURE_BYTES).unwrap_or(u64::MAX) {
        return Err(Box::new(oversized_capture(&path)));
    }
    Ok((path, id))
}

fn read_capture(path: &Path) -> Result<Capture, Box<Diagnostic>> {
    // Windows refuses opening a directory as a file before an opened handle can
    // be inspected. Classify the path first so every platform reports the same
    // capture boundary. The handle metadata check below still closes the race
    // between this observation and the open.
    let path_metadata = std::fs::symlink_metadata(path).map_err(|error| {
        capture_load_error(
            path,
            format!("cannot open the capture file safely: {error}"),
            "Make the file readable, or remove it from `.inbox`.",
        )
    })?;
    if !path_metadata.file_type().is_file() {
        return Err(capture_load_error(
            path,
            "the capture path stopped naming a regular file while the inbox was being read",
            "Retry after the concurrent edit finishes; keep only canonical regular capture files in `.inbox`.",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|error| {
        capture_load_error(
            path,
            format!("cannot open the capture file safely: {error}"),
            "Make the file readable, or remove it from `.inbox`.",
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        capture_load_error(
            path,
            format!("cannot inspect the opened capture file: {error}"),
            "Make the file readable, or remove it from `.inbox`.",
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(capture_load_error(
            path,
            "the capture path stopped naming a regular file while the inbox was being read",
            "Retry after the concurrent edit finishes; keep only canonical regular capture files in `.inbox`.",
        ));
    }
    if metadata.len() > u64::try_from(MAX_CAPTURE_BYTES).unwrap_or(u64::MAX) {
        return Err(Box::new(oversized_capture(path)));
    }
    let mut bytes = Vec::new();
    file.take(u64::try_from(MAX_CAPTURE_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            capture_load_error(
                path,
                format!("cannot read the capture file: {error}"),
                "Make the file readable, or remove it from `.inbox`.",
            )
        })?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err(Box::new(oversized_capture(path)));
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| {
        capture_load_error(
            path,
            "the capture file is not valid UTF-8",
            "Rewrite the file as UTF-8 YAML, or remove it from `.inbox`.",
        )
    })?;
    let value: serde_json::Value = yaml_compat::from_str(source).map_err(|error| {
        capture_load_error(
            path,
            format!("invalid capture YAML: {error}"),
            "Repair the YAML so it is one `aplan/capture/v1` mapping.",
        )
    })?;
    let validator = capture_validator()?;
    if let Some(error) = validator.iter_errors(&value).next() {
        return Err(Box::new(schema_diagnostic(path, &error)));
    }
    yaml_compat::from_str(source).map_err(|error| {
        capture_load_error(
            path,
            format!("capture types could not be read after schema validation: {error}"),
            "Repair the capture to match `schemas/v1/capture.json`.",
        )
    })
}

fn oversized_capture(path: &Path) -> Diagnostic {
    capture_diagnostic(
        path,
        format!("capture file exceeds the {MAX_CAPTURE_BYTES}-byte limit"),
        "Reduce the capture body so the complete YAML record stays within the documented bound.",
    )
}

fn schema_diagnostic(path: &Path, error: &jsonschema::ValidationError<'_>) -> Diagnostic {
    let at = error.instance_path().to_string();
    let sensitive = ["/title", "/body", "/labels", "/source/key"]
        .iter()
        .any(|prefix| {
            at.strip_prefix(*prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        });
    let detail = if sensitive {
        String::new()
    } else {
        format!(": {error}")
    };
    capture_diagnostic(
        path,
        format!(
            "capture does not satisfy the V1 schema at `{}`{detail}",
            if at.is_empty() { "/" } else { &at },
        ),
        "Repair the named field to match `schemas/v1/capture.json`; unknown fields and schema versions are refused.",
    )
}

fn validate_capture(
    capture: &Capture,
    id_from_name: &str,
    policy: &CapturePolicy,
    now: i64,
) -> Vec<String> {
    let mut errors = Vec::new();
    validate_capture_content(capture, id_from_name, policy, &mut errors);
    validate_capture_times(capture, now, &mut errors);
    validate_capture_lifecycle(capture, &mut errors);
    errors
}

fn validate_capture_content(
    capture: &Capture,
    id_from_name: &str,
    policy: &CapturePolicy,
    errors: &mut Vec<String>,
) {
    if !is_capture_id(&capture.id) {
        errors.push("`id` is not a canonical `I-<ULID>` capture id".into());
    } else if capture.id != id_from_name {
        errors.push(format!(
            "capture id `{}` does not agree with filename `{id_from_name}.yml`",
            capture.id
        ));
    }
    if is_vacuous_ref(&capture.title) {
        errors.push("capture title is blank or invisible".into());
    } else if capture.title.len() > MAX_TITLE_BYTES {
        errors.push(format!(
            "capture title exceeds the {MAX_TITLE_BYTES}-byte limit"
        ));
    } else if let Err(reason) = validate_text(&capture.title, TextKind::Scalar) {
        errors.push(format!("capture title {reason}"));
    }
    if !policy.kinds.contains(&capture.kind) {
        errors.push(format!(
            "capture kind `{}` is not built in or declared by `capture.kinds`",
            capture.kind
        ));
    }
    if let Some(body) = &capture.body {
        if body.len() > MAX_CAPTURE_BYTES {
            errors.push(format!(
                "capture body exceeds the {MAX_CAPTURE_BYTES}-byte limit"
            ));
        } else if is_vacuous_ref(body) {
            errors.push("capture body is blank or invisible".into());
        } else if let Err(reason) = validate_text(body, TextKind::Body) {
            errors.push(format!("capture body {reason}"));
        }
    }
    if capture.labels.len() > MAX_LABELS {
        errors.push(format!("capture has more than {MAX_LABELS} labels"));
    }
    if let Err(reason) = validate_values("labels", &capture.labels, true) {
        errors.push(reason);
    }
    if let Some(at) = &capture.revisit_at
        && parse_capture_instant(at).is_none()
    {
        errors.push("`revisit_at` is not UTC RFC 3339 at whole-second precision".into());
    }
    if let Some(profile) = &capture.profile
        && let Err(reason) = validate_key(profile, KeySyntax::Kebab)
    {
        errors.push(format!("capture profile `{profile}` {reason}"));
    }
    if let Some(defaults) = &capture.promotion_defaults
        && let Err(reason) = validate_promotion_defaults(defaults)
    {
        errors.push(format!("promotion defaults {reason}"));
    }
    if let Some(source) = &capture.source
        && let Err(reason) = validate_source(source)
    {
        errors.push(reason);
    }
}

fn validate_capture_times(capture: &Capture, now: i64, errors: &mut Vec<String>) {
    let captured = parse_capture_instant(&capture.captured_at);
    if captured.is_none() {
        errors.push("`captured_at` is not UTC RFC 3339 at whole-second precision".into());
    } else if captured.is_some_and(|at| at > now) {
        errors.push("`captured_at` is later than this operation's injected clock".into());
    }
    let disposed = capture
        .disposed_at
        .as_deref()
        .and_then(parse_capture_instant);
    if capture.disposed_at.is_some() && disposed.is_none() {
        errors.push("`disposed_at` is not UTC RFC 3339 at whole-second precision".into());
    }
    if disposed.is_some_and(|at| at > now) {
        errors.push("`disposed_at` is later than this operation's injected clock".into());
    }
    if let (Some(captured), Some(disposed)) = (captured, disposed)
        && disposed < captured
    {
        errors.push("`disposed_at` is earlier than immutable `captured_at`".into());
    }
}

fn validate_capture_lifecycle(capture: &Capture, errors: &mut Vec<String>) {
    if let Some(reason) = &capture.reason {
        if is_vacuous_ref(reason) {
            errors.push("disposition `reason` is blank or invisible".into());
        } else if let Err(error) = validate_text(reason, TextKind::Scalar) {
            errors.push(format!("disposition `reason` {error}"));
        }
    }
    match capture.state {
        CaptureState::Open => {
            if capture.disposed_at.is_some()
                || capture.reason.is_some()
                || capture.promoted_to.is_some()
                || capture.promotion_fingerprint.is_some()
            {
                errors
                    .push("an `open` capture cannot carry disposition or promotion fields".into());
            }
        }
        CaptureState::Discarded => {
            if capture.disposed_at.is_none() {
                errors.push("a `discarded` capture requires `disposed_at`".into());
            }
            if capture.promoted_to.is_some() || capture.promotion_fingerprint.is_some() {
                errors.push("a `discarded` capture cannot carry a promotion link".into());
            }
        }
        CaptureState::Promoted => {
            if capture.disposed_at.is_none() {
                errors.push("a `promoted` capture requires `disposed_at`".into());
            }
            if capture
                .promoted_to
                .as_ref()
                .is_none_or(|id| is_vacuous_ref(id))
            {
                errors.push("a `promoted` capture requires non-vacuous `promoted_to`".into());
            }
            if !capture
                .promotion_fingerprint
                .as_ref()
                .is_some_and(|value| is_lower_hex_digest(value))
            {
                errors.push(
                    "a `promoted` capture requires a 64-character lower-case promotion fingerprint"
                        .into(),
                );
            }
        }
    }
}

fn validate_cross_record(inbox: &mut Inbox) {
    let mut ids: BTreeMap<&str, &Path> = BTreeMap::new();
    let mut sources: BTreeMap<(&str, &str), (&str, &Path)> = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for loaded in &inbox.captures {
        let capture = &loaded.capture;
        if let Some(first) = ids.get(capture.id.as_str()) {
            diagnostics.push(capture_diagnostic(
                &loaded.path,
                format!(
                    "capture id `{}` is duplicated; it first appeared in `{}`",
                    capture.id,
                    first.display()
                ),
                "Give each retained capture one unique full id and matching filename.",
            ));
        } else {
            ids.insert(&capture.id, &loaded.path);
        }
        if let Some(source) = &capture.source
            && let Some((first_id, first_path)) =
                sources.get(&(source.namespace.as_str(), source.key.as_str()))
        {
            diagnostics.push(capture_diagnostic(
                &loaded.path,
                format!(
                    "source namespace `{}` is duplicated by captures `{first_id}` and `{}`; the opaque key is intentionally not printed (first record `{}`)",
                    source.namespace,
                    capture.id,
                    first_path.display()
                ),
                "Keep exactly one retained capture for each source namespace/key pair; merge the content or replace one source identity without printing the opaque key.",
            ));
        } else if let Some(source) = &capture.source {
            sources.insert(
                (&source.namespace, &source.key),
                (&capture.id, &loaded.path),
            );
        }
    }
    inbox.diagnostics.extend(diagnostics);
}

fn validate_source(source: &CaptureSource) -> Result<(), String> {
    validate_key(&source.namespace, KeySyntax::Namespace)
        .map_err(|reason| format!("source namespace `{}` {reason}", source.namespace))?;
    if is_vacuous_ref(&source.key) {
        return Err("source key is blank or invisible".into());
    }
    if source.key.len() > MAX_SOURCE_KEY_BYTES {
        return Err(format!(
            "source key exceeds the {MAX_SOURCE_KEY_BYTES}-byte limit"
        ));
    }
    if source.key.chars().any(char::is_control) {
        return Err("source key contains a control character".into());
    }
    validate_text(&source.key, TextKind::Scalar)
        .map_err(|_| "source key contains an invisible or unsupported character".to_string())?;
    Ok(())
}

#[derive(Clone, Copy)]
enum KeySyntax {
    Kebab,
    Namespace,
}

fn validate_key(value: &str, syntax: KeySyntax) -> Result<(), &'static str> {
    if is_vacuous_ref(value) {
        return Err("is blank or invisible");
    }
    if !value.is_ascii() {
        return Err("must use lower-case ASCII");
    }
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment.bytes().enumerate().all(|(index, byte)| match byte {
                b'a'..=b'z' => true,
                b'0'..=b'9' => index > 0,
                b'-' => index > 0 && index + 1 < segment.len(),
                _ => false,
            })
            && !segment.contains("--")
    };
    let valid = match syntax {
        KeySyntax::Kebab => valid_segment(value),
        KeySyntax::Namespace => value.split('.').all(valid_segment),
    };
    if valid {
        Ok(())
    } else {
        Err("must be a lower-case ASCII key with letters, digits and internal hyphens")
    }
}

#[derive(Clone, Copy)]
enum TextKind {
    Scalar,
    Body,
}

fn validate_text(value: &str, kind: TextKind) -> Result<(), &'static str> {
    let bad = value.chars().any(|character| {
        (character.is_control()
            && (matches!(kind, TextKind::Scalar) || !matches!(character, '\n' | '\r' | '\t')))
            || matches!(
                character,
                '\u{00ad}'
                    | '\u{061c}'
                    | '\u{200b}'
                    | '\u{200c}'
                    | '\u{200d}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{2060}'
                    | '\u{feff}'
            )
    });
    if bad {
        Err("contains an invisible or unsupported control character")
    } else {
        Ok(())
    }
}

fn validate_values(field: &str, values: &[String], require_sorted: bool) -> Result<(), String> {
    if values.iter().any(|value| is_vacuous_ref(value)) {
        return Err(format!("`{field}` contains a blank or invisible entry"));
    }
    for value in values {
        validate_text(value, TextKind::Scalar)
            .map_err(|reason| format!("`{field}` entry {reason}"))?;
    }
    let mut seen = BTreeSet::new();
    if values.iter().any(|value| !seen.insert(value)) {
        return Err(format!("`{field}` contains a duplicate entry"));
    }
    if require_sorted && values.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(format!("`{field}` must be sorted intake keys"));
    }
    Ok(())
}

/// Whether `value` is one canonical capture id.
#[must_use]
pub fn is_capture_id(value: &str) -> bool {
    let Some(ulid) = value.strip_prefix("I-") else {
        return false;
    };
    ulid.len() == 26
        && ulid
            .bytes()
            .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte))
        && ulid.as_bytes().first().is_some_and(|byte| *byte <= b'7')
}

/// Parse the capture timestamp subset: UTC and whole seconds only.
#[must_use]
pub fn parse_capture_instant(value: &str) -> Option<i64> {
    (value.len() == 20 && value.ends_with('Z'))
        .then(|| rfc3339_to_epoch(value))
        .flatten()
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Format one injected operation instant as canonical whole-second UTC.
#[must_use]
pub fn format_capture_instant(epoch: i64) -> String {
    // Read projections need an exact evaluation instant, but formatting the
    // complete Gregorian range is not their domain. Every persisted instant is
    // already canonical; this handles the practical clock range without adding
    // a second date dependency. The fallback remains explicit and deterministic.
    let days = epoch.div_euclid(86_400);
    let seconds = epoch.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    if !(0..=9999).contains(&year) {
        return format!("unix:{epoch}");
    }
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

// Howard Hinnant's civil-from-days transform, with 1970-01-01 as day zero.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
    let month = month_piece + if month_piece < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn capture_diagnostic(path: &Path, message: impl Into<String>, instruction: &str) -> Diagnostic {
    Diagnostic::error(code::CAPTURE_INVALID, message)
        .with_file(path)
        .with_agent_instruction(instruction)
}

fn capture_load_error(
    path: &Path,
    message: impl Into<String>,
    instruction: &str,
) -> Box<Diagnostic> {
    Box::new(capture_diagnostic(path, message, instruction))
}

fn capture_validator() -> Result<&'static jsonschema::Validator, Box<Diagnostic>> {
    static VALIDATOR: std::sync::OnceLock<Result<jsonschema::Validator, String>> =
        std::sync::OnceLock::new();
    match VALIDATOR.get_or_init(|| {
        jsonschema::validator_for(&derived_schema()).map_err(|error| error.to_string())
    }) {
        Ok(validator) => Ok(validator),
        Err(error) => Err(Box::new(
            Diagnostic::error(
                code::CAPTURE_INVALID,
                format!("internal: derived capture schema is not a valid JSON Schema: {error}"),
            )
            .with_agent_instruction("This is an internal error in aplan; please file a bug."),
        )),
    }
}

/// The generated capture JSON Schema.
#[must_use]
pub fn derived_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Capture)).expect("capture schema is serialisable")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "I-01K8Z3A4H00000000000000000";
    const SECOND: &str = "I-01K8Z3A4H00000000000000001";
    const THIRD: &str = "I-01K8Z3A4H00000000000000002";
    const FOURTH: &str = "I-01K8Z3A4H00000000000000003";
    const NOW: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z

    fn capture(id: &str, at: &str) -> Capture {
        Capture {
            schema: CaptureSchema::V1,
            id: id.into(),
            title: format!("title {id}"),
            kind: "task".into(),
            state: CaptureState::Open,
            captured_at: at.into(),
            origin: CaptureOrigin::Cli,
            body: None,
            labels: Vec::new(),
            revisit_at: None,
            profile: None,
            promotion_defaults: None,
            source: None,
            disposed_at: None,
            reason: None,
            promoted_to: None,
            promotion_fingerprint: None,
        }
    }

    fn write_capture(root: &Path, value: &Capture) {
        std::fs::create_dir_all(root.join(INBOX_DIR)).expect("inbox");
        std::fs::write(
            root.join(INBOX_DIR).join(format!("{}.yml", value.id)),
            yaml_compat::to_string(value).expect("yaml"),
        )
        .expect("capture");
    }

    fn projection_inbox() -> Inbox {
        let mut older = capture(FIRST, "2025-12-31T23:59:58Z");
        older.labels = vec!["alpha".into()];
        older.body = Some("find this text".into());
        let mut deferred = capture(SECOND, "2025-12-31T23:59:59Z");
        deferred.labels = vec!["alpha".into()];
        deferred.revisit_at = Some("2026-01-01T00:00:01Z".into());
        let mut tied = capture(THIRD, "2025-12-31T23:59:59Z");
        tied.title = "needle in the title".into();
        tied.kind = "idea".into();
        tied.labels = vec!["beta".into()];
        Inbox {
            captures: [(deferred, "two"), (tied, "three"), (older, "one")]
                .into_iter()
                .map(|(capture, path)| LoadedCapture {
                    capture,
                    path: PathBuf::from(path),
                })
                .collect(),
            diagnostics: Vec::new(),
        }
    }

    fn profiled_request() -> (CapturePolicy, CaptureCreate) {
        let profiles = BTreeMap::from([(
            "triage".into(),
            CaptureProfile {
                kind: Some("idea".into()),
                labels: vec!["inbox".into()],
                body: Some("scaffold".into()),
                promotion: Some(PromotionDefaults {
                    phase: Some(2),
                    tags: vec!["feature".into()],
                    ..PromotionDefaults::default()
                }),
            },
        )]);
        let policy = CapturePolicy::try_new(&[], &profiles).expect("policy");
        let request = CaptureCreate {
            title: "captured".into(),
            kind: None,
            body: None,
            labels: None,
            revisit_at: None,
            profile: Some("triage".into()),
            promotion_defaults: None,
            source: Some(CaptureSource {
                namespace: "tracker".into(),
                key: "opaque".into(),
            }),
            origin: CaptureOrigin::Mcp,
        };
        (policy, request)
    }

    #[test]
    fn capture_loader_reports_what_it_says_without_exposing_source_keys() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let mut one = capture(FIRST, "2025-12-31T23:59:59Z");
        one.source = Some(CaptureSource {
            namespace: "tracker".into(),
            key: "private-upstream-42".into(),
        });
        let mut two = capture(SECOND, "2025-12-31T23:59:58Z");
        two.source = one.source.clone();
        write_capture(dir.path(), &one);
        write_capture(dir.path(), &two);

        let inbox = load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW));
        let diagnostic = inbox
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("source namespace"))
            .expect("duplicate source finding");
        assert_eq!(diagnostic.code, code::CAPTURE_INVALID);
        assert!(diagnostic.message.contains(FIRST));
        assert!(diagnostic.message.contains(SECOND));
        assert!(!diagnostic.message.contains("private-upstream-42"));
        assert!(
            diagnostic
                .agent_instruction
                .as_deref()
                .is_some_and(|instruction| {
                    instruction.contains("namespace/key pair")
                        && !instruction.contains("private-upstream-42")
                })
        );
        assert!(inbox.show(FIRST).is_err(), "focused reads fail closed too");
        assert!(
            inbox
                .list(&CaptureQuery::default(), &FixedClock(NOW))
                .is_err(),
            "lists must not project a valid-looking subset of an invalid inbox"
        );
    }

    /// Proof for `capture.body_is_non_vacuous`.
    #[test]
    fn capture_body_is_non_vacuous() {
        let mut value = capture(FIRST, "2025-12-31T23:59:59Z");
        value.body = Some(" \n\t".to_string());
        let effective = EffectiveCapture {
            title: value.title.clone(),
            kind: value.kind.clone(),
            body: value.body.clone(),
            labels: Vec::new(),
            revisit_at: None,
            profile: None,
            promotion_defaults: None,
            source: None,
            origin: CaptureOrigin::Cli,
        };
        assert!(
            effective
                .validate()
                .is_err_and(|error| error.contains("blank or invisible"))
        );

        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        write_capture(dir.path(), &value);
        let inbox = load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW));
        assert!(inbox.diagnostics.iter().any(|finding| {
            finding
                .message
                .contains("capture body is blank or invisible")
                && finding
                    .agent_instruction
                    .as_deref()
                    .is_some_and(|instruction| instruction.contains("Repair the capture"))
        }));
    }

    /// Proof for `capture.writer_staging_is_not_a_record`.
    #[test]
    fn capture_writer_staging_is_not_a_record() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let inbox_path = dir.path().join(INBOX_DIR);
        std::fs::create_dir_all(&inbox_path).expect("inbox");
        std::fs::write(inbox_path.join(format!(".{FIRST}.yml.tmp0")), "partial")
            .expect("staging file");
        let clean = load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW));
        assert!(clean.is_clean());
        assert!(clean.captures.is_empty());

        std::fs::write(inbox_path.join(".not-a-writer-file.tmp0"), "partial")
            .expect("foreign hidden file");
        let invalid = load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW));
        assert_eq!(invalid.diagnostics.len(), 1);
        assert!(invalid.diagnostics[0].message.contains("canonical"));
    }

    #[test]
    fn invisible_and_control_characters_fail_visibly_without_leaking_values() {
        for invisible in [
            '\u{00ad}', '\u{061c}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{200e}', '\u{200f}',
            '\u{2060}', '\u{feff}',
        ] {
            assert!(
                validate_text(&format!("visible{invisible}"), TextKind::Scalar).is_err(),
                "U+{:04X} must not survive in a visible scalar",
                u32::from(invisible)
            );
        }
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let mut invisible = capture(SECOND, "2025-12-31T23:59:58Z");
        invisible.title = "looks valid\u{200b}".into();
        let mut controlled = capture(THIRD, "2025-12-31T23:59:57Z");
        controlled.source = Some(CaptureSource {
            namespace: "tracker".into(),
            key: "opaque\u{7}key".into(),
        });
        write_capture(dir.path(), &invisible);
        write_capture(dir.path(), &controlled);
        std::fs::write(
            dir.path().join(INBOX_DIR).join(format!("{FOURTH}.yml")),
            format!(
                "schema: {CAPTURE_SCHEMA}\nid: {FOURTH}\ntitle: visible\nkind: task\nstate: open\ncaptured_at: '2025-12-31T23:59:56Z'\norigin: cli\nbody:\n  private-secret-value: true\n"
            ),
        )
        .expect("schema-sensitive fixture");

        let inbox = load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW));
        assert_eq!(inbox.diagnostics.len(), 3);
        let rendered = inbox
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("invisible"), "{rendered}");
        assert!(rendered.contains("control character"), "{rendered}");
        assert!(!rendered.contains("looks valid"), "{rendered}");
        assert!(!rendered.contains("opaque"), "{rendered}");
        assert!(!rendered.contains("private-secret-value"), "{rendered}");
    }

    #[test]
    fn capture_loader_rejects_time_and_lifecycle_neighbours_at_an_injected_clock() {
        let policy = CapturePolicy::default();
        let mut value = capture(FIRST, "2026-01-01T00:00:00Z");
        assert!(
            validate_capture(&value, FIRST, &policy, NOW).is_empty(),
            "the exact operation instant is valid"
        );

        value.captured_at = "2026-01-01T00:00:01Z".into();
        let errors = validate_capture(&value, FIRST, &policy, NOW);
        assert!(errors.iter().any(|error| error.contains("injected clock")));

        value.captured_at = "2025-12-31T23:59:59+00:00".into();
        value.revisit_at = Some("2026-01-01T00:00:00.5Z".into());
        let errors = validate_capture(&value, FIRST, &policy, NOW);
        assert!(errors.iter().any(|error| error.contains("captured_at")));
        assert!(errors.iter().any(|error| error.contains("revisit_at")));

        value.captured_at = "2025-12-31T23:59:59Z".into();
        value.revisit_at = None;
        value.state = CaptureState::Discarded;
        value.disposed_at = Some("2025-12-31T23:59:58Z".into());
        let errors = validate_capture(&value, FIRST, &policy, NOW);
        assert!(errors.iter().any(|error| error.contains("earlier")));

        value.disposed_at = Some("2026-01-01T00:00:01Z".into());
        let errors = validate_capture(&value, FIRST, &policy, NOW);
        assert!(errors.iter().any(|error| error.contains("disposed_at")));

        value.state = CaptureState::Open;
        let errors = validate_capture(&value, FIRST, &policy, NOW);
        assert!(errors.iter().any(|error| error.contains("cannot carry")));

        value.state = CaptureState::Discarded;
        value.disposed_at = None;
        value.promoted_to = Some("A-1".into());
        value.promotion_fingerprint = Some("a".repeat(64));
        let errors = validate_capture(&value, FIRST, &policy, NOW);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("requires `disposed_at`"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("cannot carry a promotion"))
        );

        value.state = CaptureState::Promoted;
        value.promoted_to = None;
        value.promotion_fingerprint = Some("A".repeat(64));
        let errors = validate_capture(&value, FIRST, &policy, NOW);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("requires `disposed_at`"))
        );
        assert!(errors.iter().any(|error| error.contains("promoted_to")));
        assert!(errors.iter().any(|error| error.contains("lower-case")));

        value.disposed_at = Some("2025-12-31T23:59:59Z".into());
        value.promoted_to = Some("A-1".into());
        value.promotion_fingerprint = Some("a".repeat(64));
        assert!(
            validate_capture(&value, FIRST, &policy, NOW).is_empty(),
            "a complete promoted state at the capture instant is valid"
        );
    }

    #[test]
    fn projections_are_bounded_filtered_and_newest_first_with_stable_ties() {
        let inbox = projection_inbox();
        let list = inbox
            .list(&CaptureQuery::default(), &FixedClock(NOW))
            .expect("list");
        assert_eq!(list.evaluated_at, "2026-01-01T00:00:00Z");
        assert_eq!(list.deferred, 1);
        assert_eq!(
            list.captures
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec![THIRD, FIRST]
        );

        let all = inbox
            .list(
                &CaptureQuery {
                    revisit: RevisitFilter::All,
                    ..CaptureQuery::default()
                },
                &FixedClock(NOW),
            )
            .expect("all");
        assert_eq!(
            all.captures
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec![SECOND, THIRD, FIRST],
            "equal instants use full id ascending as the total-order tie breaker"
        );

        let body_search = inbox
            .list(
                &CaptureQuery {
                    search: Some("THIS".into()),
                    ..CaptureQuery::default()
                },
                &FixedClock(NOW),
            )
            .expect("search");
        assert_eq!(body_search.captures[0].id, FIRST);

        for (query, expected) in [
            (
                CaptureQuery {
                    kind: Some("idea".into()),
                    ..CaptureQuery::default()
                },
                THIRD,
            ),
            (
                CaptureQuery {
                    label: Some("beta".into()),
                    ..CaptureQuery::default()
                },
                THIRD,
            ),
            (
                CaptureQuery {
                    revisit: RevisitFilter::Deferred,
                    ..CaptureQuery::default()
                },
                SECOND,
            ),
            (
                CaptureQuery {
                    search: Some("NEEDLE".into()),
                    ..CaptureQuery::default()
                },
                THIRD,
            ),
        ] {
            let filtered = inbox.list(&query, &FixedClock(NOW)).expect("filter");
            assert_eq!(filtered.captures.len(), 1);
            assert_eq!(filtered.captures[0].id, expected);
        }
    }

    #[test]
    fn projections_filter_terminal_state_and_refuse_unbounded_queries() {
        let inbox = projection_inbox();
        let terminal = Capture {
            state: CaptureState::Discarded,
            disposed_at: Some("2026-01-01T00:00:00Z".into()),
            ..capture(FOURTH, "2025-12-31T23:59:57Z")
        };
        let with_terminal = Inbox {
            captures: inbox
                .captures
                .iter()
                .cloned()
                .chain([LoadedCapture {
                    capture: terminal,
                    path: PathBuf::from("four"),
                }])
                .collect(),
            diagnostics: Vec::new(),
        };
        let discarded = with_terminal
            .list(
                &CaptureQuery {
                    state: Some(CaptureState::Discarded),
                    revisit: RevisitFilter::All,
                    ..CaptureQuery::default()
                },
                &FixedClock(NOW),
            )
            .expect("state filter");
        assert_eq!(discarded.captures[0].id, FOURTH);

        for query in [
            CaptureQuery {
                limit: 0,
                ..CaptureQuery::default()
            },
            CaptureQuery {
                limit: MAX_LIST_LIMIT + 1,
                ..CaptureQuery::default()
            },
            CaptureQuery {
                search: Some(" ".into()),
                ..CaptureQuery::default()
            },
            CaptureQuery {
                search: Some("x".repeat(MAX_SEARCH_BYTES + 1)),
                ..CaptureQuery::default()
            },
        ] {
            assert!(inbox.list(&query, &FixedClock(NOW)).is_err());
        }
    }

    #[test]
    fn body_files_and_human_prefixes_refuse_each_boundary_by_name() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let directory_error = read_capture_body_file(dir.path()).expect_err("directory body");
        assert!(
            directory_error.contains("not a regular file"),
            "{directory_error}"
        );

        let oversized = dir.path().join("oversized-body");
        let file = std::fs::File::create(&oversized).expect("oversized body");
        file.set_len(u64::try_from(MAX_CAPTURE_BYTES).expect("bound") + 1)
            .expect("extend body");
        let oversized_error = read_capture_body_file(&oversized).expect_err("oversized body");
        assert!(
            oversized_error.contains("capture body limit"),
            "{oversized_error}"
        );

        let invalid_utf8 = dir.path().join("invalid-utf8-body");
        std::fs::write(&invalid_utf8, [0xff, 0xfe]).expect("invalid UTF-8 body");
        let encoding_error = read_capture_body_file(&invalid_utf8).expect_err("invalid UTF-8");
        assert!(
            encoding_error.contains("invalid byte at offset 0"),
            "{encoding_error}"
        );

        let inbox = projection_inbox();
        for invalid in ["I-", "I-lower", "I-01K8Z3A4H000000000000000000"] {
            let error = inbox.resolve_prefix(invalid).expect_err("invalid prefix");
            assert!(error.contains("canonical upper-case"), "{invalid}: {error}");
        }
        let missing = inbox
            .resolve_prefix("I-01K8Z3A4H00000000000000009")
            .expect_err("missing prefix");
        assert!(missing.contains("no capture matches"), "{missing}");
    }

    #[test]
    fn profiles_prefill_only_declared_fields_and_retry_compares_persisted_effective_values() {
        let (policy, request) = profiled_request();
        let source = request.source.clone().expect("source");
        let effective = request.effective(&policy).expect("effective");
        assert_eq!(effective.kind, "idea");
        assert_eq!(effective.labels, ["inbox"]);
        assert_eq!(
            effective.promotion_defaults.as_ref().and_then(|p| p.phase),
            Some(2)
        );

        let mut stored = capture(FIRST, "2025-12-31T23:59:59Z");
        stored.title.clone_from(&effective.title);
        stored.kind.clone_from(&effective.kind);
        stored.body.clone_from(&effective.body);
        stored.labels.clone_from(&effective.labels);
        stored.profile.clone_from(&effective.profile);
        stored
            .promotion_defaults
            .clone_from(&effective.promotion_defaults);
        stored.source = Some(source);
        stored.origin = CaptureOrigin::Mcp;
        let inbox = Inbox {
            captures: vec![LoadedCapture {
                capture: stored,
                path: "one".into(),
            }],
            diagnostics: Vec::new(),
        };
        assert!(matches!(
            inbox.resolve_source(&effective).expect("valid inbox"),
            SourceResolution::Equivalent { .. }
        ));

        let mut disposed = inbox.clone();
        disposed.captures[0].capture.state = CaptureState::Discarded;
        disposed.captures[0].capture.disposed_at = Some("2026-01-01T00:00:00Z".into());
        assert!(matches!(
            disposed
                .resolve_source(&effective)
                .expect("valid disposed retry"),
            SourceResolution::Equivalent {
                state: CaptureState::Discarded,
                ..
            }
        ));

        let mut changed = effective;
        changed.labels.push("new-default".into());
        let conflict = inbox.resolve_source(&changed).expect("valid inbox");
        assert!(matches!(
            &conflict,
            SourceResolution::Conflict { id, state: CaptureState::Open, namespace }
                if id == FIRST && namespace == "tracker"
        ));
        assert!(
            !format!("{conflict:?}").contains("opaque"),
            "ordinary conflict data omits the source key"
        );
        assert_eq!(
            inbox
                .show(FIRST)
                .expect("valid inbox")
                .and_then(|capture| capture.source.as_ref())
                .map(|source| source.key.as_str()),
            Some("opaque"),
            "focused show is the one read allowed to expose the complete source"
        );
    }

    #[test]
    fn explicit_capture_and_promotion_values_override_profile_defaults() {
        let (policy, request) = profiled_request();
        let overridden = CaptureCreate {
            kind: Some("note".into()),
            body: Some("explicit body".into()),
            labels: Some(Vec::new()),
            promotion_defaults: Some(PromotionDefaults {
                phase: Some(3),
                tags: vec!["explicit".into()],
                ..PromotionDefaults::default()
            }),
            ..request
        }
        .effective(&policy)
        .expect("explicit override");
        assert_eq!(overridden.kind, "note");
        assert_eq!(overridden.body.as_deref(), Some("explicit body"));
        assert!(overridden.labels.is_empty());
        let promotion = overridden.promotion_defaults.expect("promotion snapshot");
        assert_eq!(promotion.phase, Some(3));
        assert_eq!(promotion.tags, ["explicit"]);
    }

    #[test]
    fn loader_refuses_noncanonical_members_duplicate_yaml_and_id_disagreement() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let inbox_dir = dir.path().join(INBOX_DIR);
        std::fs::create_dir(&inbox_dir).expect("inbox");
        std::fs::write(
            inbox_dir.join(format!("{FIRST}.yml")),
            format!(
                "schema: {CAPTURE_SCHEMA}\nid: {FIRST}\ntitle: one\ntitle: two\nkind: task\nstate: open\ncaptured_at: '2025-12-31T23:59:59Z'\norigin: cli\n"
            ),
        )
        .expect("duplicate field");
        std::fs::write(inbox_dir.join(format!("{SECOND}.yaml")), "not: canonical\n")
            .expect("wrong suffix");
        let mismatch = capture(FOURTH, "2025-12-31T23:59:59Z");
        std::fs::write(
            inbox_dir.join(format!("{THIRD}.yml")),
            yaml_compat::to_string(&mismatch).expect("yaml"),
        )
        .expect("mismatch");
        std::fs::create_dir(inbox_dir.join(format!("{FOURTH}.yml"))).expect("non-regular");

        let inbox = load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW));
        let messages = inbox
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(inbox.diagnostics.len(), 4, "{messages}");
        assert!(messages.contains("duplicate field"), "{messages}");
        assert!(messages.contains("lower-case `.yml` suffix"), "{messages}");
        assert!(
            messages.contains("does not agree with filename"),
            "{messages}"
        );
        assert!(messages.contains("not a regular file"), "{messages}");
        assert!(inbox.captures.is_empty());
    }

    #[test]
    fn inbox_load_uses_one_injected_operation_instant() {
        struct CountingClock(std::cell::Cell<usize>);

        impl Clock for CountingClock {
            fn now_epoch(&self) -> i64 {
                self.0.set(self.0.get() + 1);
                NOW
            }
        }

        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        write_capture(dir.path(), &capture(FIRST, "2025-12-31T23:59:59Z"));
        write_capture(dir.path(), &capture(SECOND, "2025-12-31T23:59:59Z"));
        let clock = CountingClock(std::cell::Cell::new(0));
        let inbox = load_inbox(dir.path(), &CapturePolicy::default(), &clock);
        assert!(inbox.is_clean());
        assert_eq!(clock.0.get(), 1, "one snapshot has one evaluation instant");
    }

    #[test]
    fn derived_schema_closes_versions_origins_states_and_unknown_fields() {
        let schema = derived_schema();
        let validator = jsonschema::validator_for(&schema).expect("derived schema");
        let valid = serde_json::json!({
            "schema": CAPTURE_SCHEMA,
            "id": FIRST,
            "title": "intake",
            "kind": "task",
            "state": "open",
            "captured_at": "2025-12-31T23:59:59Z",
            "origin": "cli"
        });
        assert!(validator.is_valid(&valid));
        for (field, value) in [
            ("schema", serde_json::json!("aplan/capture/v2")),
            ("state", serde_json::json!("parked")),
            ("origin", serde_json::json!("import")),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid
                .as_object_mut()
                .expect("capture object")
                .insert(field.into(), value);
            assert!(!validator.is_valid(&invalid), "{invalid}");
        }
    }

    #[test]
    fn empty_and_missing_inboxes_are_valid_and_capture_ids_are_canonical() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        assert!(load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW)).is_clean());
        std::fs::create_dir(dir.path().join(INBOX_DIR)).expect("inbox");
        assert!(load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW)).is_clean());

        let retired_profile = Capture {
            profile: Some("retired-profile".into()),
            ..capture(FIRST, "2025-12-31T23:59:59Z")
        };
        write_capture(dir.path(), &retired_profile);
        assert!(
            load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW)).is_clean(),
            "a stored profile snapshot stays readable after config removes that profile"
        );

        let wrong_shape = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(wrong_shape.path().join(INBOX_DIR), "not a directory")
            .expect("reserved file");
        let invalid = load_inbox(
            wrong_shape.path(),
            &CapturePolicy::default(),
            &FixedClock(NOW),
        );
        assert_eq!(invalid.diagnostics.len(), 1);
        assert!(invalid.diagnostics[0].message.contains("not a directory"));

        assert!(is_capture_id(FIRST));
        assert!(!is_capture_id("I-81K8Z3A4H00000000000000000"));
        assert!(!is_capture_id("i-01K8Z3A4H00000000000000000"));
        assert!(parse_capture_instant("2026-01-01T00:00:00Z").is_some());
        assert!(parse_capture_instant("2026-01-01T00:00:00+00:00").is_none());
        assert!(parse_capture_instant("2026-01-01T00:00:00.0Z").is_none());
    }

    #[test]
    fn oversized_capture_is_refused_at_the_bounded_read_boundary() {
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let inbox = dir.path().join(INBOX_DIR);
        std::fs::create_dir(&inbox).expect("inbox");
        let file = std::fs::File::create(inbox.join(format!("{FIRST}.yml"))).expect("capture");
        file.set_len(u64::try_from(MAX_CAPTURE_BYTES).expect("bound") + 1)
            .expect("sparse oversize fixture");
        let loaded = load_inbox(dir.path(), &CapturePolicy::default(), &FixedClock(NOW));
        assert_eq!(loaded.diagnostics.len(), 1);
        assert!(
            loaded.diagnostics[0]
                .message
                .contains("exceeds the 1048576-byte limit")
        );
    }

    #[test]
    fn policy_validation_covers_vocabulary_profiles_and_promotion_neighbours() {
        assert!(SystemClock.now_epoch() > 0, "the process clock is usable");
        assert!(
            CapturePolicy::try_new(&["task".into()], &BTreeMap::new())
                .expect_err("built-in duplicate")
                .contains("more than once")
        );
        assert!(
            CapturePolicy::try_new(&["Bad_Kind".into()], &BTreeMap::new())
                .expect_err("invalid kind")
                .contains("lower-case ASCII")
        );
        assert!(
            CapturePolicy::try_new(
                &[],
                &BTreeMap::from([("Bad_Profile".into(), CaptureProfile::default())]),
            )
            .expect_err("invalid profile name")
            .contains("capture profile")
        );

        let full_profile = CaptureProfile {
            kind: Some("feature-request".into()),
            labels: vec!["inbox".into()],
            body: Some("line one\nline two\tstill text".into()),
            promotion: Some(PromotionDefaults {
                phase: Some(2),
                cut: Some("M-capture".into()),
                effort_days: Some(1.5),
                owner: Some("maintainer".into()),
                paths: vec!["crates/**".into()],
                tags: vec!["feature".into()],
            }),
        };
        let profiles = BTreeMap::from([("triage".into(), full_profile.clone())]);
        let policy = CapturePolicy::try_new(&["feature-request".into()], &profiles)
            .expect("complete policy");
        assert!(policy.kinds().contains("feature-request"));
        assert_eq!(policy.profiles().get("triage"), Some(&full_profile));

        let invalid_profile = |profile: CaptureProfile| {
            CapturePolicy::try_new(
                &["feature-request".into()],
                &BTreeMap::from([("triage".into(), profile)]),
            )
            .expect_err("invalid profile")
        };
        assert!(
            invalid_profile(CaptureProfile {
                kind: Some("unknown".into()),
                ..CaptureProfile::default()
            })
            .contains("unknown kind")
        );
        assert!(
            invalid_profile(CaptureProfile {
                labels: (0..=MAX_LABELS).map(|index| format!("l{index}")).collect(),
                ..CaptureProfile::default()
            })
            .contains("more than 64 labels")
        );
        assert!(
            invalid_profile(CaptureProfile {
                body: Some("x".repeat(MAX_CAPTURE_BYTES + 1)),
                ..CaptureProfile::default()
            })
            .contains("body exceeds")
        );
        assert!(
            invalid_profile(CaptureProfile {
                body: Some("body\u{200b}".into()),
                ..CaptureProfile::default()
            })
            .contains("invisible")
        );

        for (defaults, expected) in [
            (
                PromotionDefaults {
                    effort_days: Some(0.0),
                    ..PromotionDefaults::default()
                },
                "positive finite",
            ),
            (
                PromotionDefaults {
                    effort_days: Some(f64::INFINITY),
                    ..PromotionDefaults::default()
                },
                "positive finite",
            ),
            (
                PromotionDefaults {
                    cut: Some(" \u{200b}".into()),
                    ..PromotionDefaults::default()
                },
                "blank or invisible",
            ),
            (
                PromotionDefaults {
                    owner: Some("owner\nname".into()),
                    ..PromotionDefaults::default()
                },
                "unsupported control",
            ),
            (
                PromotionDefaults {
                    paths: vec!["src/**".into(), "src/**".into()],
                    ..PromotionDefaults::default()
                },
                "duplicate",
            ),
            (
                PromotionDefaults {
                    tags: vec![String::new()],
                    ..PromotionDefaults::default()
                },
                "blank or invisible",
            ),
        ] {
            let error = invalid_profile(CaptureProfile {
                promotion: Some(defaults),
                ..CaptureProfile::default()
            });
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn effective_capture_validation_covers_defaults_and_every_bounded_field() {
        let request = CaptureCreate {
            title: "captured".into(),
            kind: None,
            body: None,
            labels: None,
            revisit_at: None,
            profile: None,
            promotion_defaults: None,
            source: None,
            origin: CaptureOrigin::Tui,
        };
        let policy = CapturePolicy::default();
        let effective = request.effective(&policy).expect("default request");
        assert_eq!(effective.kind, "task");
        assert!(effective.promotion_defaults.is_none());
        assert_eq!(
            Inbox::default()
                .resolve_source(&effective)
                .expect("clean inbox"),
            SourceResolution::NoSource
        );
        assert_eq!(Inbox::default().show(FIRST).expect("clean inbox"), None);

        let empty_promotion = CaptureCreate {
            promotion_defaults: Some(PromotionDefaults::default()),
            ..request.clone()
        }
        .effective(&policy)
        .expect("empty defaults");
        assert!(empty_promotion.promotion_defaults.is_none());

        let no_profile_base = PromotionDefaults {
            paths: vec!["profile/**".into()],
            tags: vec!["profile".into()],
            ..PromotionDefaults::default()
        };
        let explicit = PromotionDefaults {
            paths: vec!["explicit/**".into()],
            tags: vec!["explicit".into()],
            ..PromotionDefaults::default()
        };
        let overlaid = no_profile_base.overlay(&explicit);
        assert_eq!(overlaid.paths, ["explicit/**"]);
        assert_eq!(overlaid.tags, ["explicit"]);
        let inherited = no_profile_base.overlay(&PromotionDefaults::default());
        assert_eq!(inherited.paths, ["profile/**"]);
        assert_eq!(inherited.tags, ["profile"]);

        let unknown_profile = CaptureCreate {
            profile: Some("missing".into()),
            ..request.clone()
        };
        assert!(
            unknown_profile
                .effective(&policy)
                .expect_err("unknown profile")
                .contains("not declared")
        );
        let unknown_kind = CaptureCreate {
            kind: Some("feature-request".into()),
            ..request.clone()
        };
        assert!(
            unknown_kind
                .effective(&policy)
                .expect_err("unknown kind")
                .contains("not declared")
        );

        let invalid_effective = |mutate: fn(&mut EffectiveCapture), expected: &str| {
            let mut value = effective.clone();
            mutate(&mut value);
            let error = value.validate().expect_err("invalid effective capture");
            assert!(error.contains(expected), "{error}");
        };
        invalid_effective(|value| value.title = " ".into(), "blank or invisible");
        invalid_effective(
            |value| value.title = "x".repeat(MAX_TITLE_BYTES + 1),
            "title exceeds",
        );
        invalid_effective(
            |value| value.title = "title\nline".into(),
            "unsupported control",
        );
        invalid_effective(
            |value| value.body = Some("x".repeat(MAX_CAPTURE_BYTES + 1)),
            "body exceeds",
        );
        invalid_effective(
            |value| value.body = Some("body\u{2060}".into()),
            "invisible",
        );
        invalid_effective(
            |value| {
                value.labels = (0..=MAX_LABELS).map(|index| format!("l{index}")).collect();
            },
            "more than 64 labels",
        );
        invalid_effective(
            |value| value.labels = vec!["z".into(), "a".into()],
            "must be sorted",
        );
        invalid_effective(
            |value| value.revisit_at = Some("2026-01-01T00:00:00.1Z".into()),
            "whole-second",
        );
        invalid_effective(
            |value| value.profile = Some("Bad".into()),
            "lower-case ASCII",
        );
        invalid_effective(
            |value| {
                value.promotion_defaults = Some(PromotionDefaults {
                    effort_days: Some(-1.0),
                    ..PromotionDefaults::default()
                });
            },
            "positive finite",
        );
        invalid_effective(
            |value| {
                value.source = Some(CaptureSource {
                    namespace: "tracker".into(),
                    key: "\u{200b}".into(),
                });
            },
            "blank or invisible",
        );

        let sourced = CaptureCreate {
            source: Some(CaptureSource {
                namespace: "tracker".into(),
                key: "unseen".into(),
            }),
            ..request
        }
        .effective(&policy)
        .expect("sourced request");
        assert_eq!(
            Inbox::default()
                .resolve_source(&sourced)
                .expect("clean inbox"),
            SourceResolution::Vacant
        );
        let control_search = CaptureQuery {
            search: Some("bad\nsearch".into()),
            ..CaptureQuery::default()
        };
        assert!(
            projection_inbox()
                .list(&control_search, &FixedClock(NOW))
                .expect_err("control search")
                .contains("unsupported control")
        );
    }

    #[test]
    fn loader_reports_encoding_schema_and_hostile_entry_neighbours() {
        // Proof for `capture.regular_file_boundary_is_portable`.
        let dir = tempfile::Builder::new()
            .tempdir_in(crate::fixture::fixture_root())
            .expect("tempdir");
        let inbox = dir.path().join(INBOX_DIR);
        std::fs::create_dir(&inbox).expect("inbox");

        let non_utf8 = inbox.join(format!("{FIRST}.yml"));
        std::fs::write(&non_utf8, [0xff, 0xfe]).expect("non-utf8 fixture");
        let error = read_capture(&non_utf8).expect_err("non-utf8 capture");
        assert!(error.message.contains("not valid UTF-8"));

        let invalid_yaml = inbox.join(format!("{SECOND}.yml"));
        std::fs::write(&invalid_yaml, "[unterminated").expect("invalid YAML fixture");
        let error = read_capture(&invalid_yaml).expect_err("invalid YAML");
        assert!(error.message.contains("invalid capture YAML"));

        let schema_error = inbox.join(format!("{THIRD}.yml"));
        std::fs::write(
            &schema_error,
            format!(
                "schema: {CAPTURE_SCHEMA}\nid: {THIRD}\ntitle: valid\nkind: 7\nstate: open\ncaptured_at: '2025-12-31T23:59:59Z'\norigin: cli\n"
            ),
        )
        .expect("schema fixture");
        let error = read_capture(&schema_error).expect_err("schema mismatch");
        assert!(error.message.contains("at `/kind`"));
        assert!(
            error.message.contains("must be string") || error.message.contains("type"),
            "non-sensitive schema errors retain safe detail: {}",
            error.message
        );

        let bad_name = inbox.join("I-not-a-canonical-ulid.yml");
        std::fs::write(&bad_name, "anything").expect("bad filename fixture");
        let entry = std::fs::read_dir(&inbox)
            .expect("entries")
            .find_map(|entry| {
                let entry = entry.expect("entry");
                (entry.path() == bad_name).then_some(entry)
            })
            .expect("bad entry");
        let error = capture_entry_identity(&entry).expect_err("noncanonical id");
        assert!(error.message.contains("canonical `I-<ULID>`"));

        let missing = inbox.join(format!("{FOURTH}.yml"));
        let error = read_capture(&missing).expect_err("missing capture");
        assert!(error.message.contains("cannot open"));
        let error = read_capture(&inbox).expect_err("directory is not a capture");
        assert!(error.message.contains("stopped naming a regular file"));

        let parent_file = dir.path().join("not-a-directory");
        std::fs::write(&parent_file, "ordinary file").expect("file parent fixture");
        let error = inbox_entries(&parent_file.join(INBOX_DIR))
            .expect_err("a non-directory parent cannot contain an inbox");
        assert!(error.message.contains("cannot inspect"));

        let vanished = inbox.join(format!("{FOURTH}.yml"));
        std::fs::write(&vanished, "anything").expect("vanishing fixture");
        let entry = std::fs::read_dir(&inbox)
            .expect("entries")
            .find_map(|entry| {
                let entry = entry.expect("entry");
                (entry.path() == vanished).then_some(entry)
            })
            .expect("vanishing entry");
        std::fs::remove_file(&vanished).expect("remove vanishing fixture");
        let error = capture_entry_identity(&entry).expect_err("entry disappeared");
        assert!(error.message.contains("disappeared"));

        let oversized = inbox.join(format!("{FOURTH}.yml"));
        let file = std::fs::File::create(&oversized).expect("oversized fixture");
        file.set_len(u64::try_from(MAX_CAPTURE_BYTES).expect("bound") + 1)
            .expect("extend fixture");
        let error = read_capture(&oversized).expect_err("direct oversized read");
        assert!(error.message.contains("exceeds"));
        std::fs::remove_file(&oversized).expect("remove oversized fixture");

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt as _, symlink};

            let unreadable = dir.path().join("unreadable").join(INBOX_DIR);
            std::fs::create_dir_all(&unreadable).expect("unreadable inbox");
            let original = std::fs::metadata(&unreadable)
                .expect("unreadable metadata")
                .permissions();
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o0))
                .expect("remove inbox permissions");
            let unreadable_error = inbox_entries(&unreadable).expect_err("unreadable inbox");
            std::fs::set_permissions(&unreadable, original).expect("restore inbox permissions");
            assert!(unreadable_error.message.contains("cannot enumerate"));

            let link = inbox.join(format!("{FOURTH}.yml"));
            symlink(&schema_error, &link).expect("symlink fixture");
            let error = read_capture(&link).expect_err("safe open refuses symlink");
            assert!(error.message.contains("stopped naming a regular file"));

            #[cfg(target_os = "linux")]
            assert_non_utf8_capture_name_is_refused(&inbox);
        }
    }

    // Darwin refuses this byte sequence at the filesystem boundary, before
    // the capture loader can inspect the entry.
    #[cfg(target_os = "linux")]
    fn assert_non_utf8_capture_name_is_refused(inbox: &Path) {
        use std::os::unix::ffi::OsStringExt as _;

        let name = std::ffi::OsString::from_vec(vec![b'I', b'-', 0xff, b'.', b'y', b'm', b'l']);
        let path = inbox.join(name);
        std::fs::write(&path, "anything").expect("non-utf8 name");
        let entry = std::fs::read_dir(inbox)
            .expect("entries")
            .find_map(|entry| {
                let entry = entry.expect("entry");
                (entry.path() == path).then_some(entry)
            })
            .expect("non-utf8 entry");
        let error = capture_entry_identity(&entry).expect_err("non-utf8 filename");
        assert!(error.message.contains("filename is not valid UTF-8"));
    }

    #[test]
    fn content_validation_enumerates_each_independent_field_boundary() {
        let policy = CapturePolicy::default();
        let invalid_content = |mutate: fn(&mut Capture), expected: &str| {
            let mut value = capture(FIRST, "2025-12-31T23:59:59Z");
            mutate(&mut value);
            let mut errors = Vec::new();
            validate_capture_content(&value, FIRST, &policy, &mut errors);
            assert!(
                errors.iter().any(|error| error.contains(expected)),
                "expected `{expected}` in {errors:?}"
            );
        };
        invalid_content(|value| value.id = "I-invalid".into(), "canonical");
        invalid_content(|value| value.title = String::new(), "blank or invisible");
        invalid_content(
            |value| value.title = "x".repeat(MAX_TITLE_BYTES + 1),
            "title exceeds",
        );
        invalid_content(|value| value.title = "title\u{200f}".into(), "invisible");
        invalid_content(|value| value.kind = "unknown".into(), "not built in");
        invalid_content(
            |value| value.body = Some("x".repeat(MAX_CAPTURE_BYTES + 1)),
            "body exceeds",
        );
        invalid_content(
            |value| value.body = Some("body\u{00ad}".into()),
            "invisible",
        );
        invalid_content(
            |value| {
                value.labels = (0..=MAX_LABELS).map(|index| format!("l{index}")).collect();
            },
            "more than 64 labels",
        );
        invalid_content(
            |value| value.labels = vec![String::new()],
            "blank or invisible",
        );
        invalid_content(
            |value| value.labels = vec!["same".into(), "same".into()],
            "duplicate",
        );
        invalid_content(
            |value| value.labels = vec!["z".into(), "a".into()],
            "must be sorted",
        );
        invalid_content(
            |value| value.revisit_at = Some("not-an-instant".into()),
            "revisit_at",
        );
        invalid_content(
            |value| value.profile = Some("bad_profile".into()),
            "lower-case ASCII",
        );
        invalid_content(
            |value| {
                value.promotion_defaults = Some(PromotionDefaults {
                    effort_days: Some(0.0),
                    ..PromotionDefaults::default()
                });
            },
            "promotion defaults",
        );
    }

    #[test]
    fn source_disposition_and_time_validation_cover_each_independent_boundary() {
        let policy = CapturePolicy::default();
        let mut invalid_namespace = capture(FIRST, "2025-12-31T23:59:59Z");
        invalid_namespace.source = Some(CaptureSource {
            namespace: "Bad.Namespace".into(),
            key: "opaque".into(),
        });
        let mut errors = Vec::new();
        validate_capture_content(&invalid_namespace, FIRST, &policy, &mut errors);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("source namespace")),
            "{errors:?}"
        );

        for (source, expected) in [
            (
                CaptureSource {
                    namespace: "tracker".into(),
                    key: String::new(),
                },
                "blank or invisible",
            ),
            (
                CaptureSource {
                    namespace: "tracker".into(),
                    key: "x".repeat(MAX_SOURCE_KEY_BYTES + 1),
                },
                "exceeds",
            ),
            (
                CaptureSource {
                    namespace: "tracker".into(),
                    key: "line\nbreak".into(),
                },
                "control character",
            ),
            (
                CaptureSource {
                    namespace: "tracker".into(),
                    key: "opaque\u{061c}".into(),
                },
                "invisible or unsupported",
            ),
        ] {
            let error = validate_source(&source).expect_err("invalid source");
            assert!(error.contains(expected), "{error}");
        }

        let mut invalid_time = capture(FIRST, "2025-12-31T23:59:59Z");
        invalid_time.state = CaptureState::Discarded;
        invalid_time.disposed_at = Some("2025-12-31 23:59:59".into());
        let errors = validate_capture(&invalid_time, FIRST, &policy, NOW);
        assert!(errors.iter().any(|error| error.contains("disposed_at")));

        let mut bad_reason = capture(FIRST, "2025-12-31T23:59:59Z");
        bad_reason.state = CaptureState::Discarded;
        bad_reason.disposed_at = Some("2025-12-31T23:59:59Z".into());
        bad_reason.reason = Some("\u{200b}".into());
        let errors = validate_capture(&bad_reason, FIRST, &policy, NOW);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("blank or invisible"))
        );
        bad_reason.reason = Some("line\nbreak".into());
        let errors = validate_capture(&bad_reason, FIRST, &policy, NOW);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("unsupported control"))
        );
    }

    #[test]
    fn cross_record_ids_and_time_formatting_keep_deterministic_boundaries() {
        let mut inbox = Inbox {
            captures: vec![
                LoadedCapture {
                    capture: capture(FIRST, "2025-12-31T23:59:59Z"),
                    path: "first".into(),
                },
                LoadedCapture {
                    capture: capture(FIRST, "2025-12-31T23:59:58Z"),
                    path: "second".into(),
                },
            ],
            diagnostics: Vec::new(),
        };
        validate_cross_record(&mut inbox);
        assert_eq!(inbox.diagnostics.len(), 1);
        assert!(inbox.diagnostics[0].message.contains("duplicated"));
        assert!(inbox.diagnostics[0].message.contains("first"));

        assert_eq!(format_capture_instant(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_capture_instant(-1), "1969-12-31T23:59:59Z");
        assert_eq!(
            format_capture_instant(i64::MAX),
            format!("unix:{}", i64::MAX)
        );
        assert!(!is_capture_id("A-01K8Z3A4H00000000000000000"));
        assert!(!is_capture_id("I-01K8Z3A4H0000000000000000I"));

        for (value, syntax) in [
            ("", KeySyntax::Kebab),
            ("é", KeySyntax::Kebab),
            ("1starts-with-digit", KeySyntax::Kebab),
            ("trailing-", KeySyntax::Kebab),
            ("double--hyphen", KeySyntax::Kebab),
            ("empty..segment", KeySyntax::Namespace),
        ] {
            assert!(validate_key(value, syntax).is_err(), "`{value}` must fail");
        }
        assert!(validate_key("tracker.github", KeySyntax::Namespace).is_ok());
        assert!(validate_text("body\nwith\ttabs", TextKind::Body).is_ok());
    }
}
