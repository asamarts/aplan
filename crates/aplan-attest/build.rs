//! The source identity of this build, captured where it is knowable (`A1-154`).
//!
//! A binary can report its package version, and every development build from
//! `main` reports the same one -- which is what the fifth pilot measured:
//! materially different capability sets under `aplan 0.3.0-dev`. The commit the
//! binary was built from is knowable only here, at build time, and only when
//! there is a checkout to ask.
//!
//! `APLAN_SOURCE_REV` overrides, so a release pipeline building from a tarball
//! can supply what its own provenance already knows -- and a pipeline that cares
//! should set it, because what a build script can learn is **best effort**: it
//! runs only when one of its declared inputs changes, so the commit it captured
//! is the commit as of the last time cargo thought it needed to ask.
//!
//! That is why the tree's *cleanliness* is deliberately not recorded here. The
//! first version appended `git describe --dirty`, and this task's audit measured
//! what that claim was worth: a clean tree still reported `-dirty` from an
//! earlier run, and the ordinary edit-compile loop -- change a file, rebuild,
//! `HEAD` unmoved -- reported clean while the binary contained the change. A
//! provenance document may not carry a flag that says "this came from exactly
//! that commit" when nothing checked. `binary_sha256` is the field that
//! identifies the artifact, and it cannot go stale, because it is computed from
//! the artifact.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=APLAN_SOURCE_REV");
    // Rebuild when the checkout moves, or the recorded rev goes stale the first
    // time somebody commits without touching this crate.
    // `packed-refs` too: after `git gc` a branch may have no file under
    // `refs/heads`, and the commit would then go stale without it.
    for path in [".git/HEAD", ".git/refs/heads", ".git/packed-refs"] {
        println!("cargo:rerun-if-changed=../../{path}");
    }

    let rev = std::env::var("APLAN_SOURCE_REV")
        .ok()
        .filter(|r| !r.trim().is_empty())
        .or_else(git_describe)
        .unwrap_or_default();
    println!("cargo:rustc-env=APLAN_SOURCE_REV={rev}");
}

/// The commit this source directory was at, as a full sha.
///
/// No `--dirty`: see the module comment. A marker that cannot be kept true is
/// worse in a provenance record than no marker.
fn git_describe() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let rev = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!rev.is_empty()).then_some(rev)
}
