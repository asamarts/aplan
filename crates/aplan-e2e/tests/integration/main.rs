//! The single integration-test harness for this crate (`A1-630`).
//!
//! Keep the source modules in `tests/`: task paths and trace references name
//! those files directly. The integration-target census checks that this list
//! and the files on disk remain equal.

#[path = "../coverage.rs"]
mod coverage;
#[path = "../evidence_kinds.rs"]
mod evidence_kinds;
#[path = "../hermetic.rs"]
mod hermetic;
#[path = "../milestones.rs"]
mod milestones;
#[path = "../rule_tests.rs"]
mod rule_tests;
#[path = "../scale.rs"]
mod scale;
