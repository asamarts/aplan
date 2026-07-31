//! End-to-end scenario corpus and the coverage-drift audits.
//!
//! This crate has no library code of its own, it exists to hold black-box
//! tests that drive the built `aplan` behaviour through the public API of
//! [`aplan_core`] and [`aplan_output`], and the meta-tests that fail CI when a
//! unit of surface area (a rule, a format) ships without a test.
//!
//! See `tests/coverage.rs`. The load-bearing one is
//! `every_rule_has_a_fires_and_a_silent_scenario`: it walks
//! [`aplan_core::Rule::ALL`], so adding a rule without both a *fires* and a
//! *silent* scenario turns the build red. That is how "test every rule" becomes
//! a property of the build rather than a discipline (ROADMAP §5).
