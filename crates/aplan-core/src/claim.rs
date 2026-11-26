//! Ephemeral work claims, kept distinct from durable task ownership (`A1-531`).
//!
//! A task's `owner` is plan data: the role or person responsible for the work.
//! A claim answers a different and shorter-lived question: who is acting on it
//! now. These types are the delivery-neutral projection shared by CLI, MCP and
//! application consumers. Persistence belongs to `aplan-app`; the core model
//! deliberately knows neither Git nor a filesystem.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Status;

/// Status-versus-coordination lifecycle state of one task's local claim
/// (`ADR-0073`, `A2-152`).
///
/// Computed once from the task's durable status, its local claim record, and
/// whether local coordination is available, so CLI, MCP, and TUI agree on
/// active claims, stale terminal claims, and unclaimed `in_progress` work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimCoordinationState {
    /// Live task held by an active local claim.
    Active,
    /// Terminal task (`done` or `dropped`) still carrying a leftover local
    /// claim from an older build, another actor, or failed post-close cleanup.
    StaleTerminal,
    /// Live `in_progress` task with no active local claim while coordination is
    /// available: durable status and local coordination disagree.
    InProgressUnclaimed,
    /// Ordinary task with no active claim.
    Unclaimed,
    /// Local coordination scope is unavailable (for example outside Git).
    Unavailable,
}

impl ClaimCoordinationState {
    /// Derive the lifecycle state from durable task status and local claim
    /// observation.
    #[must_use]
    pub const fn derive(
        status: Status,
        claim: Option<&ClaimRecord>,
        scope_available: bool,
    ) -> Self {
        if !scope_available && claim.is_none() {
            return Self::Unavailable;
        }
        let is_terminal = matches!(status, Status::Done | Status::Dropped);
        match (is_terminal, claim) {
            (true, Some(_)) => Self::StaleTerminal,
            (false, Some(_)) => Self::Active,
            (false, None) if matches!(status, Status::InProgress) => Self::InProgressUnclaimed,
            (_, None) => Self::Unclaimed,
        }
    }

    /// Stable machine label for this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::StaleTerminal => "stale_terminal",
            Self::InProgressUnclaimed => "in_progress_unclaimed",
            Self::Unclaimed => "unclaimed",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Format a signed Unix epoch timestamp as an ISO-8601 UTC instant
/// (`YYYY-MM-DDTHH:MM:SSZ`) without external date crates (`ADR-0073`, `A2-152`).
#[must_use]
pub fn format_utc_timestamp(epoch_seconds: i64) -> String {
    let days = epoch_seconds.div_euclid(86_400);
    let secs_of_day = epoch_seconds.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    // Howard Hinnant's civil-from-days algorithm (proleptic Gregorian calendar).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = y + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Format a non-negative duration in seconds as a compact, deterministic human
/// span (for example `0s`, `17s`, `5m`, `3h 12m`, `2d 4h`).
#[must_use]
pub fn format_duration_short(seconds: i64) -> String {
    let secs = seconds.max(0);
    if secs < 60 {
        return format!("{secs}s");
    }
    if secs < 3600 {
        let mins = secs / 60;
        let rem = secs % 60;
        return if rem == 0 {
            format!("{mins}m")
        } else {
            format!("{mins}m {rem}s")
        };
    }
    if secs < 86_400 {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        return if mins == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {mins}m")
        };
    }
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    if hours == 0 {
        format!("{days}d")
    } else {
        format!("{days}d {hours}h")
    }
}

/// Format a claim's age relative to `observed_at_epoch_seconds`, explicitly
/// naming clock skew when `claimed_at_epoch_seconds > observed_at_epoch_seconds`.
#[must_use]
pub fn format_claim_age(claimed_at_epoch_seconds: i64, observed_at_epoch_seconds: i64) -> String {
    if claimed_at_epoch_seconds > observed_at_epoch_seconds {
        let ahead = claimed_at_epoch_seconds.saturating_sub(observed_at_epoch_seconds);
        return format!(
            "in the future by {} (clock skew)",
            format_duration_short(ahead)
        );
    }
    let elapsed = observed_at_epoch_seconds.saturating_sub(claimed_at_epoch_seconds);
    if elapsed == 0 {
        "just now".to_string()
    } else {
        format!("{} ago", format_duration_short(elapsed))
    }
}

/// Format a claim timestamp for human output: absolute UTC instant plus stable
/// relative age (`YYYY-MM-DDTHH:MM:SSZ (<age>)`).
#[must_use]
pub fn format_claim_time_human(
    claimed_at_epoch_seconds: i64,
    observed_at_epoch_seconds: i64,
) -> String {
    format!(
        "{} ({})",
        format_utc_timestamp(claimed_at_epoch_seconds),
        format_claim_age(claimed_at_epoch_seconds, observed_at_epoch_seconds)
    )
}

/// Where an unauthenticated holder label was observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimOrigin {
    /// Host label, when the environment offered one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Canonical worktree path. `None` for a bare repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

/// One ephemeral claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimRecord {
    /// An attribution label, not an authenticated principal.
    pub holder: String,
    /// Which deterministic identity source supplied `holder`.
    pub identity_source: String,
    /// Signed Unix seconds when this exact claim or takeover was recorded.
    pub claimed_at_epoch_seconds: i64,
    /// Where the command that recorded it ran.
    pub origin: ClaimOrigin,
}

/// The coordination boundary behind a claim snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimScope {
    /// Whether a shared local scope was available.
    pub available: bool,
    /// `git_common_directory` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Stable digest of the canonical common-directory path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The durability contract, always explicit in machine output.
    pub durability: String,
    /// Why coordination was unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ClaimScope {
    /// A scope shared by every linked worktree of one local Git repository.
    #[must_use]
    pub fn git_common_directory(id: String) -> Self {
        Self {
            available: true,
            kind: Some("git_common_directory".to_string()),
            id: Some(id),
            durability: "local_ephemeral_until_explicit_release".to_string(),
            reason: None,
        }
    }

    /// No safe shared local scope could be established.
    #[must_use]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            kind: None,
            id: None,
            durability: "unavailable".to_string(),
            reason: Some(reason.into()),
        }
    }
}

/// One coherent observation of the local claim store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimSnapshot {
    /// Clock at which this snapshot was observed, so claim age is derivable.
    pub observed_at_epoch_seconds: i64,
    pub scope: ClaimScope,
    /// Claims keyed by exact task id, in deterministic order.
    pub claims: BTreeMap<String, ClaimRecord>,
}

impl ClaimSnapshot {
    /// An explicit degraded snapshot for a repository with no shared scope.
    #[must_use]
    pub fn unavailable(observed_at_epoch_seconds: i64, reason: impl Into<String>) -> Self {
        Self {
            observed_at_epoch_seconds,
            scope: ClaimScope::unavailable(reason),
            claims: BTreeMap::new(),
        }
    }

    /// The exact record for `id`, when this snapshot has one.
    #[must_use]
    pub fn claim(&self, id: &str) -> Option<&ClaimRecord> {
        self.claims.get(id)
    }

    /// Whole seconds this snapshot had observed the claim to be held.
    #[must_use]
    pub fn age_seconds(&self, id: &str) -> Option<i64> {
        self.claim(id).map(|claim| {
            self.observed_at_epoch_seconds
                .saturating_sub(claim.claimed_at_epoch_seconds)
                .max(0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordination_state_labels_and_duration_formatting_cover_all_branches() {
        assert_eq!(ClaimCoordinationState::Active.as_str(), "active");
        assert_eq!(
            ClaimCoordinationState::StaleTerminal.as_str(),
            "stale_terminal"
        );
        assert_eq!(
            ClaimCoordinationState::InProgressUnclaimed.as_str(),
            "in_progress_unclaimed"
        );
        assert_eq!(ClaimCoordinationState::Unclaimed.as_str(), "unclaimed");
        assert_eq!(ClaimCoordinationState::Unavailable.as_str(), "unavailable");

        assert_eq!(format_duration_short(-5), "0s");
        assert_eq!(format_duration_short(17), "17s");
        assert_eq!(format_duration_short(120), "2m");
        assert_eq!(format_duration_short(125), "2m 5s");
        assert_eq!(format_duration_short(7200), "2h");
        assert_eq!(format_duration_short(7320), "2h 2m");
        assert_eq!(format_duration_short(172_800), "2d");
        assert_eq!(format_duration_short(180_000), "2d 2h");
    }
}
