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
