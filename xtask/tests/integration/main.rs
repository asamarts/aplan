#![cfg(unix)]

//! The single integration-test harness for this crate (`A1-630`).
//!
//! Keep the source modules in `tests/`: task paths and trace references name
//! those files directly. The integration-target census checks that this list
//! and the files on disk remain equal. These tests exercise POSIX shells,
//! signals, permissions and non-UTF-8 paths; the hosted portable contract runs
//! the cross-platform library and binary tests instead.

#[path = "../common.rs"]
mod common;

#[path = "../action_selftest.rs"]
mod action_selftest;
#[path = "../fixture_executable.rs"]
mod fixture_executable;
#[path = "../pre_push_hook.rs"]
mod pre_push_hook;
#[path = "../pushed.rs"]
mod pushed;
#[path = "../release_changelog.rs"]
mod release_changelog;
#[path = "../untracked.rs"]
mod untracked;
#[path = "../validation_profiles.rs"]
mod validation_profiles;
#[path = "../validation_rollout.rs"]
mod validation_rollout;
#[path = "../validation_selection.rs"]
mod validation_selection;
