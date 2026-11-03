//! Fixtures are built on disk, not in RAM (`A1-257`).
//!
//! `tempfile::tempdir()` writes under `TMPDIR`, which is `/tmp`, which on any
//! modern Linux is a `tmpfs`. A `TempDir` removes itself on drop, so every
//! leftover is from a run that panicked or was killed -- and a session doing
//! mutation testing kills runs constantly. Measured on one machine: 3,966 stale
//! directories, the oldest a month old, and `/tmp` mounted `usrquota`, so the
//! next fixture write failed as `EDQUOT` inside a test about globs.
//!
//! The rule is enforced rather than remembered, in the shape `A1-207` used for
//! spawning: a new test reaching for the RAM disk fails here, by name, with the
//! replacement in the message.

use std::path::{Path, PathBuf};

use crate::common;

/// Every Rust source this workspace owns, test and production alike.
fn workspace_sources() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root is two above this crate")
        .to_path_buf();
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("xtask")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .expect("reading a source directory")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                // `target` may sit inside a crate when `CARGO_TARGET_DIR` is
                // left alone, and it holds copies of nothing this gate owns.
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                out.push((
                    rel,
                    std::fs::read_to_string(&path).expect("reading a source"),
                ));
            }
        }
    }
    out.sort();
    assert!(
        out.len() > 60,
        "the workspace did not enumerate, so this gate reads nothing: {} file(s)",
        out.len()
    );
    out
}

/// A file with a reason to name the RAM disk anyway, and the reason.
///
/// Held entries rather than a silent exception: each names what it does and why
/// the fixture rule does not reach it.
const MAY_NAME_THE_RAM_DISK: &[(&str, &str)] = &[
    (
        "crates/aplan-core/src/fixture.rs",
        "it is the replacement, and says in its own documentation what it is a \
         replacement for",
    ),
    (
        "crates/aplan/tests/fixture_root.rs",
        "this gate, which must name what it refuses",
    ),
    (
        "crates/aplan-tui/src/lib.rs",
        "the live external-editor handoff creates one bounded private user draft in the operating \
         system's temporary directory; it is production data, not a test fixture",
    ),
];

#[test]
fn every_fixture_is_built_on_disk_rather_than_in_ram() {
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    for (name, text) in workspace_sources() {
        if MAY_NAME_THE_RAM_DISK.iter().any(|(f, _)| *f == name) {
            continue;
        }
        scanned += 1;
        for (n, line) in text.lines().enumerate() {
            // Prose may name what the rule forbids, and this repository's prose
            // does: a comment explaining the rule is not a breach of it.
            if line.trim_start().starts_with("//") {
                continue;
            }
            // `tempdir()` with no `_in` takes `TMPDIR`; so does `NamedTempFile`
            // and `Builder::tempfile` without a directory.
            // Every shape that takes `TMPDIR`. None but the first was in use
            // when this was written; they are named so the next one is caught
            // by the gate rather than by a full `/tmp`.
            let reaches_for_tmpdir = line.contains("tempfile::tempdir()")
                || line.contains("TempDir::new()")
                || line.contains("NamedTempFile::new()")
                || line.contains("tempfile::tempfile()")
                || line.contains("env::temp_dir()")
                || line.contains("CARGO_TARGET_TMPDIR");
            if reaches_for_tmpdir {
                offenders.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(scanned > 60, "only {scanned} file(s) were read");
    assert!(
        offenders.is_empty(),
        "a fixture is built in RAM, or against a root only integration tests have.\n\
         Use `tempfile::Builder::new().tempdir_in(aplan_core::fixture::fixture_root())`, \
         which is `<target>/tmp` -- the disk `cargo clean` already empties, and the one \
         root that works in a unit test too (`A1-257`):\n  {}",
        offenders.join("\n  ")
    );
}

/// A fixture whose repository never came up must not reach the checkout it sits
/// inside (`A1-257`).
///
/// This is the failure that made the task urgent, and it is not hypothetical:
/// under a full `/tmp` a fixture's `git init` failed, the `git add` and
/// `git commit` after it ran anyway, and a linked worktree's reflog recorded two
/// fixture commits while its index was reduced to the fixture's own files.
///
/// It got *more* likely when fixtures moved onto disk, because `<target>/tmp` is
/// inside this checkout unless `CARGO_TARGET_DIR` says otherwise, where `/tmp`
/// had no repository above it at all. So the guard is proven here rather than
/// reasoned about: a stand-in parent repository, a fixture below it with no
/// repository of its own, and the parent read back afterwards.
#[test]
fn a_fixture_whose_init_failed_cannot_commit_into_the_checkout_above_it() {
    let tmp = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("a fixture root");
    let parent = tmp.path().join("parent");
    std::fs::create_dir_all(&parent).expect("the parent");
    common::git(&parent, &["init", "-q", "."]);
    common::git(&parent, &["config", "user.email", "t@example.com"]);
    common::git(&parent, &["config", "user.name", "t"]);
    std::fs::write(parent.join("real.txt"), "the checkout's own file\n").expect("a file");
    common::git(&parent, &["add", "-A"]);
    common::git(
        &parent,
        &["commit", "-q", "-m", "the checkout's own commit"],
    );

    let head = String::from_utf8_lossy(&common::git(&parent, &["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string();
    let reflog = common::git(&parent, &["reflog"]).stdout;
    let index = common::git(&parent, &["ls-files"]).stdout;

    // A fixture below it whose `init` never ran, which is what a failed `init`
    // leaves behind: a writable directory, files in it, and no `.git`.
    let fixture = parent.join("target/tmp/fixture");
    std::fs::create_dir_all(&fixture).expect("the fixture");
    std::fs::write(fixture.join("f.txt"), "fixture data\n").expect("a fixture file");
    assert!(
        !fixture.join(".git").exists(),
        "the fixture has no repository"
    );

    // The setup carries on, as it did. Each call must refuse rather than find
    // the repository above, so each is expected to fail.
    for args in [
        &["add", "-A"][..],
        &["commit", "-q", "-m", "must never exist"][..],
    ] {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&fixture)
            // The stand-in for `<target>/tmp`, mirroring how the real suite is
            // arranged: the ceiling names the tree the fixtures share, and the
            // checkout sits above it.
            .env("GIT_CEILING_DIRECTORIES", parent.join("target/tmp"))
            .output()
            .expect("git runs");
        assert!(
            !out.status.success(),
            "`git {args:?}` succeeded from a fixture with no repository, which means \
             it found one above: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // And the checkout above is exactly as it was.
    assert_eq!(
        String::from_utf8_lossy(&common::git(&parent, &["rev-parse", "HEAD"]).stdout).trim(),
        head,
        "the parent's HEAD moved"
    );
    assert_eq!(
        common::git(&parent, &["reflog"]).stdout,
        reflog,
        "the parent's reflog gained an entry"
    );
    assert_eq!(
        common::git(&parent, &["ls-files"]).stdout,
        index,
        "the parent's index changed"
    );
    assert_eq!(
        std::fs::read_to_string(parent.join("real.txt")).expect("its file"),
        "the checkout's own file\n",
        "the parent's files changed"
    );
}

/// The fixture root is on disk, and nothing above it declares a policy.
///
/// Both halves are the task. Disk, because `/tmp` is RAM. And no policy above,
/// because `.aplan.yml` discovery climbs to the filesystem root: a fixture under
/// the checkout inherits the checkout's cuts and rules. That failed 26 tests
/// across 14 binaries when the root was `<target>/tmp` with `CARGO_TARGET_DIR`
/// left alone -- including one whose whole subject is what happens when there is
/// *no* configuration, which cannot be staged below a directory that has one.
///
/// It failed only where the target directory sits inside the checkout, so CI and
/// anyone who moves it saw green while everybody else saw red. That asymmetry is
/// what this pins.
#[test]
fn the_fixture_root_is_on_disk_and_owned_by_no_checkout() {
    let root = aplan_core::fixture::fixture_root();
    assert!(root.is_dir(), "{} is not a directory", root.display());

    assert!(
        !root.starts_with(std::env::temp_dir()),
        "fixtures are being built under `{}`, the system temporary directory, which is \
         a `tmpfs` on this machine -- that is RAM, and a killed run never gives it back",
        root.display()
    );

    // Both walks climb to the filesystem root, so both decide what a fixture
    // sees: `.aplan.yml` is the policy `aplan` discovers, `.git` the repository
    // it opens. A home directory that is a dotfiles repository is the ordinary
    // way to meet the second, and it breaks the tests whose subject is having
    // no repository at all.
    if let Some(owner) = root.ancestors().find(|d| {
        d.join(".aplan.yml").symlink_metadata().is_ok() || d.join(".git").symlink_metadata().is_ok()
    }) {
        panic!(
            "the fixture root `{}` sits under `{}`, whose policy or repository every \
             fixture below it would inherit",
            root.display(),
            owner.display()
        );
    }
}
