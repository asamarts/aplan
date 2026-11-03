//! The single integration-test harness for this crate (`A1-630`).
//!
//! Keep the source modules in `tests/`: task paths and trace references name
//! those files directly. The integration-target census checks that this list
//! and the files on disk remain equal.

#[path = "../prompts.rs"]
mod prompts;
#[path = "../switches.rs"]
mod switches;
#[path = "../yaml_contract.rs"]
mod yaml_contract;
