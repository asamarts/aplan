//! Where a test fixture is built, so a killed run leaks onto disk and not into
//! RAM -- and so the fixture cannot inherit anything from the tree it is running
//! in (`A1-257`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How long a leftover fixture is kept before a later run clears it.
///
/// Nothing live is a week old. Fixtures under `<target>/tmp` need no such rule
/// because `cargo clean` empties that; the cache root below has nothing else
/// that would ever tidy it.
const KEEP_LEFTOVERS_FOR: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// The directory this workspace's test fixtures are made under.
///
/// **Not [`std::env::temp_dir`]**, which is `/tmp`, which on any modern Linux is
/// a `tmpfs`, which is RAM. A fixture directory removes itself when its run
/// finishes, so every leftover is from a run that panicked or was killed -- and
/// a session doing mutation testing kills runs constantly. Measured on one
/// machine: 3,966 stale `/tmp/.tmp*` directories, the oldest a month old. It
/// does not fail as itself, either: with `/tmp` mounted `usrquota` the next
/// fixture write returned `EDQUOT` inside a test whose subject was glob
/// measurement, while `df` showed gigabytes free.
///
/// `<target>/tmp` -- what cargo means by `CARGO_TARGET_TMPDIR` -- when that is
/// usable, because `cargo clean` already empties it. Found from the test
/// binary's own path rather than read from that variable, because cargo sets it
/// only for integration tests and benches, and most of this suite's fixtures are
/// unit tests inside `src/` where it is absent.
///
/// **And a fallback, which is not a nicety.** `target/` is *inside* the checkout
/// unless `CARGO_TARGET_DIR` says otherwise, and a fixture below a checkout
/// inherits it: `.aplan.yml` discovery climbs to the filesystem root, and so
/// does git's. Measured, 26 tests across 14 binaries failed that way -- among
/// them tests whose whole subject is what happens with *no* configuration and
/// *no* repository, which cannot be staged under a directory that has one. It
/// failed only where `CARGO_TARGET_DIR` is left alone, so CI and anyone who
/// moves it saw green while everybody else saw red.
///
/// So the root is the first candidate no checkout owns, and if neither qualifies
/// this refuses rather than producing that failure again somewhere unrelated.
#[doc(hidden)]
#[must_use]
pub fn fixture_root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let cache = cache_fallback();
        let root = match target_tmp() {
            Some(target) if owner_of(&target).is_none() => target,
            _ => cache.clone(),
        };
        if let Some(owner) = owner_of(&root) {
            panic!(
                "fixtures would be built in `{}`, which is inside `{}` -- so every one of \
                 them would inherit that tree's `.aplan.yml` or its git repository, and the \
                 tests whose subject is having neither could not run at all.\n\
                 Point `CARGO_TARGET_DIR` at a directory outside any checkout, or \
                 `XDG_CACHE_HOME` at one.",
                root.display(),
                owner.display()
            );
        }
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|e| panic!("creating the fixture root `{}`: {e}", root.display()));
        if root == cache {
            clear_old_leftovers(&root);
        }
        root
    })
    .clone()
}

/// `<target>/tmp`, from the running test binary.
fn target_tmp() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // `<target>/<profile>/deps/<binary>`: the third ancestor is the target dir.
    // `ancestors()` starts at the path itself, so `nth(3)` is three up.
    Some(exe.ancestors().nth(3)?.join("tmp"))
}

/// The nearest directory at or above `dir` that a fixture would inherit from.
///
/// Both walks, because both reach the filesystem root and both decide what a
/// fixture sees: `.aplan.yml` is the policy `aplan` discovers, and `.git` is the
/// repository it opens. A home directory that is a dotfiles repository is the
/// ordinary way to meet the second.
fn owner_of(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|d| {
        d.join(".aplan.yml").symlink_metadata().is_ok() || d.join(".git").symlink_metadata().is_ok()
    })
}

/// Somewhere on disk that no checkout owns.
fn cache_fallback() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .expect("a fixture needs somewhere on disk: set `XDG_CACHE_HOME` or `HOME`");
    base.join("aplan").join("test-fixtures")
}

/// Remove fixtures old enough that no run could still be using them.
///
/// Best effort and never fatal: this is tidying, and a test that fails because
/// tidying failed would be reporting the wrong thing.
///
/// By age alone, not by name. Matching `tempfile`'s default prefix was the first
/// version and left every fixture that sets its own -- `inert-flags-*` and the
/// rest -- to accumulate forever, which is the problem this is here to stop.
/// This directory holds nothing but fixtures: it is created here, named for this
/// suite, and lives under a cache root the XDG specification already says may be
/// deleted at any time.
fn clear_old_leftovers(root: &Path) {
    let cutoff = std::time::SystemTime::now() - KEEP_LEFTOVERS_FOR;
    clear_leftovers_older_than(root, cutoff);
}

/// Remove directories whose modification time predates `cutoff`.
///
/// The cutoff is an input so the age boundary can be proven without sleeping or
/// changing filesystem timestamps. Production supplies the one-week boundary;
/// the test supplies times on either side of freshly-created fixtures.
fn clear_leftovers_older_than(root: &Path, cutoff: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < cutoff);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The runner keeps Cargo's target outside the checkout, while a default
    /// local build keeps it inside. That changed whether production happened to
    /// call cleanup and made the crate's coverage floor environment-dependent.
    /// Drive the policy directly so both layouts measure the same behavior.
    #[test]
    fn leftover_cleanup_is_exercised_independently_of_target_location() {
        let fixture = tempfile::Builder::new()
            .prefix("leftover-cleanup-")
            .tempdir_in(fixture_root())
            .expect("cleanup fixture");
        let root = fixture.path().join("root");
        let leftover = root.join("leftover");
        std::fs::create_dir_all(&leftover).expect("leftover directory");

        // A cutoff before the entry's creation keeps it.
        clear_leftovers_older_than(&root, std::time::SystemTime::UNIX_EPOCH);
        assert!(leftover.is_dir(), "a recent fixture was removed");

        // A cutoff after the entry's creation removes it. This avoids changing
        // mtimes, whose precision and directory support vary by filesystem.
        clear_leftovers_older_than(&root, std::time::SystemTime::now() + KEEP_LEFTOVERS_FOR);
        assert!(!leftover.exists(), "an expired fixture was retained");

        // The best-effort production wrapper is silent when there is nothing
        // to inspect, including a cache root removed by another cleanup.
        clear_old_leftovers(&root.join("absent"));
    }
}
