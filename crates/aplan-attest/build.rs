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

use std::path::Path;

mod build_support;

fn main() {
    println!("cargo:rerun-if-env-changed=APLAN_SOURCE_REV");
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for path in build_support::script_watch_paths(manifest_dir) {
        if let Some(path) = build_support::directive_path(&path) {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    let override_rev = std::env::var("APLAN_SOURCE_REV")
        .ok()
        .filter(|revision| !revision.trim().is_empty());
    if override_rev.is_none() {
        // Rebuild when this checkout's HEAD can have moved. Every selected path
        // exists, and none is the shared refs directory (`A1-629`).
        for path in build_support::git_watch_paths(manifest_dir) {
            if let Some(path) = build_support::directive_path(&path) {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }

    let rev = override_rev
        .or_else(|| build_support::git_head(manifest_dir))
        .unwrap_or_default();
    println!("cargo:rustc-env=APLAN_SOURCE_REV={rev}");
}
