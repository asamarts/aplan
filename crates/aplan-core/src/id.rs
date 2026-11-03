//! How a repository mints the id of a new task (`A1-047`).
//!
//! `aplan add` derives the next id from the convention the plan already uses,
//! which is what makes it fit a repository rather than impose a format on one.
//! That derivation has dstask's documented failure: two people working offline
//! both derive `A1-047`, and the collision surfaces at the merge rather than at
//! the moment either of them typed it. `APLAN002` catches it there, so a plan
//! cannot end up holding two tasks under one id, but the repair is a manual
//! renumber and every reference to the loser moves with it.
//!
//! The vocabulary lives here rather than in `aplan-config` because two crates
//! need it -- the config's schema is derived from the type, and the minting is
//! `aplan-interop`'s -- and `aplan-core` is the sink both may depend on (HLD
//! Section 7).
//!
//! **This is not a default.** A ULID is 26 characters of Crockford base32, and
//! an id is a thing people type: `aplan why A1-047` against
//! `aplan why A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD`. A repository with one author does
//! not have the collision and should not pay for it, which is why `sequential`
//! stays the default and the trade is stated where an adopter chooses.

use serde::{Deserialize, Serialize};

/// How `aplan add` mints an id when the caller does not pass `--id` (`A1-047`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum IdStrategy {
    /// Continue the sequence the plan already uses: `A1-046` -> `A1-047`. The
    /// default, and what every existing plan gets.
    #[default]
    Sequential,
    /// Mint a ULID, so two authors offline cannot derive the same id. Readable
    /// ids are the cost.
    Ulid,
}

impl IdStrategy {
    /// The word this strategy is written as in `.aplan.yml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Ulid => "ulid",
        }
    }
}

impl std::fmt::Display for IdStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_one_every_existing_plan_already_has() {
        assert_eq!(IdStrategy::default(), IdStrategy::Sequential);
    }

    /// The words a config file writes, which are a consumer contract: renaming
    /// one silently invalidates every `.aplan.yml` that set it.
    #[test]
    fn each_strategy_is_written_as_the_word_a_config_uses() {
        assert_eq!(IdStrategy::Sequential.as_str(), "sequential");
        assert_eq!(IdStrategy::Ulid.as_str(), "ulid");
        for s in [IdStrategy::Sequential, IdStrategy::Ulid] {
            let yaml = format!("{s}");
            let parsed: IdStrategy = yaml_compat::from_str(&yaml).expect("round trip");
            assert_eq!(parsed, s, "`{yaml}` did not parse back to what wrote it");
        }
    }

    /// An unknown value names the ones that exist, rather than failing with the
    /// shape of the error and none of the vocabulary (`A1-133` learned this on
    /// `tag_families`).
    #[test]
    fn an_unknown_strategy_names_the_ones_that_exist() {
        let err = yaml_compat::from_str::<IdStrategy>("uild").expect_err("not a strategy");
        let text = err.to_string();
        assert!(
            text.contains("sequential") && text.contains("ulid"),
            "{text}"
        );
    }
}
