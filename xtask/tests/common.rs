//! Helpers shared by xtask's consolidated integration-test harness.

use std::path::{Path, PathBuf};

fn select_binary_source(
    nextest: Option<&std::ffi::OsStr>,
    cargo: Option<&std::ffi::OsStr>,
    compiled: &Path,
) -> PathBuf {
    nextest
        .filter(|path| !path.is_empty())
        .or(cargo)
        .filter(|path| !path.is_empty())
        .map_or_else(|| compiled.to_owned(), PathBuf::from)
}

/// Resolve xtask through the runner's relocatable contract when available.
///
/// Nextest rewrites these runtime variables when an archive is extracted;
/// Cargo's compile-time path is only a compatibility fallback (`A1-616`).
pub fn xtask() -> PathBuf {
    let nextest = std::env::var_os("NEXTEST_BIN_EXE_xtask");
    let cargo = std::env::var_os("CARGO_BIN_EXE_xtask");
    select_binary_source(
        nextest.as_deref(),
        cargo.as_deref(),
        Path::new(env!("CARGO_BIN_EXE_xtask")),
    )
}

#[test]
fn xtask_prefers_the_runners_relocatable_path() {
    use std::ffi::OsStr;

    let compiled = Path::new("/build-host/target/debug/xtask");
    assert_eq!(
        select_binary_source(
            Some(OsStr::new("/archive/target/debug/xtask")),
            Some(OsStr::new("/cargo/target/debug/xtask")),
            compiled,
        ),
        Path::new("/archive/target/debug/xtask")
    );
    assert_eq!(
        select_binary_source(
            None,
            Some(OsStr::new("/cargo/target/debug/xtask")),
            compiled,
        ),
        Path::new("/cargo/target/debug/xtask")
    );
    assert_eq!(select_binary_source(None, None, compiled), compiled);
    assert_eq!(
        select_binary_source(Some(OsStr::new("")), Some(OsStr::new("")), compiled),
        compiled
    );
}
