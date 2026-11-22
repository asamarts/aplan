//! Cache-writer input discovery shared by `crates/aplan/build.rs` and its tests.
//!
//! `APLAN_CACHE_WRITER_IDENTITY` binds persisted plan-cache entries to the exact
//! sources that compiled this `aplan` binary. In a workspace checkout, that is
//! the root workspace manifest/lockfile plus every product crate under
//! `crates/`. In a packaged standalone build (such as `cargo install aplan` from
//! crates.io), that is strictly `CARGO_MANIFEST_DIR` itself -- walking
//! `manifest.parent().parent()` there escapes into `$CARGO_HOME/registry/src`
//! (or `/` when unpacked shallowly) and hashes unrelated crates (`A2-192`).

use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Directories skipped at any depth while collecting compiled product sources.
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    "target",
    "llvm-cov-target",
    "dist",
    "tests",
    "benches",
    "examples",
];

/// Return the workspace root when `manifest` is the workspace member
/// `<workspace>/crates/aplan`; return `None` for packaged or standalone builds.
pub(crate) fn workspace_root_for(manifest: &Path) -> Option<PathBuf> {
    if manifest.file_name() != Some(OsStr::new("aplan")) {
        return None;
    }
    let crates_dir = manifest.parent()?;
    if crates_dir.file_name() != Some(OsStr::new("crates")) {
        return None;
    }
    let workspace = crates_dir.parent()?;
    let root_manifest = fs::read_to_string(workspace.join("Cargo.toml")).ok()?;
    let lists_aplan =
        root_manifest.contains("\"crates/aplan\"") || root_manifest.contains("'crates/aplan'");
    (root_manifest.contains("[workspace]") && lists_aplan).then(|| workspace.to_path_buf())
}

/// Collect the deterministic, sorted set of source/manifest paths relative to
/// the returned root directory whose bytes contribute to `APLAN_CACHE_WRITER_IDENTITY`.
pub(crate) fn collect_cache_identity_inputs(manifest: &Path) -> (PathBuf, Vec<PathBuf>) {
    let mut inputs = Vec::new();
    if let Some(workspace) = workspace_root_for(manifest) {
        for root_file in ["Cargo.toml", "Cargo.lock"] {
            let candidate = workspace.join(root_file);
            if candidate.is_file() {
                inputs.push(PathBuf::from(root_file));
            }
        }
        let crates_dir = workspace.join("crates");
        if crates_dir.is_dir() {
            let mut entries = sorted_dir_entries(&crates_dir);
            entries.retain(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(OsStr::to_str)
                        .is_some_and(|name| name != "aplan-e2e" && !SKIPPED_DIRS.contains(&name))
            });
            for crate_dir in entries {
                collect_crate_sources(&workspace, &crate_dir, &mut inputs);
            }
        }
        inputs.sort();
        inputs.dedup();
        (workspace, inputs)
    } else {
        collect_crate_sources(manifest, manifest, &mut inputs);
        inputs.sort();
        inputs.dedup();
        (manifest.to_path_buf(), inputs)
    }
}

/// Render a collected relative path with portable forward slashes so the hashed
/// path identity does not vary across host path separators.
pub(crate) fn portable_relative_path(relative: &Path) -> String {
    relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn collect_crate_sources(root: &Path, directory: &Path, output: &mut Vec<PathBuf>) {
    for path in sorted_dir_entries(directory) {
        if path.is_dir() {
            if path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| SKIPPED_DIRS.contains(&name))
            {
                continue;
            }
            collect_crate_sources(root, &path, output);
            continue;
        }
        let file_name = path.file_name().and_then(OsStr::to_str);
        let is_manifest = matches!(file_name, Some("Cargo.toml" | "Cargo.lock" | "build.rs"));
        let is_rust = path.extension().is_some_and(|extension| extension == "rs");
        if is_manifest || is_rust {
            let relative = path
                .strip_prefix(root)
                .expect("collected source under root");
            output.push(relative.to_path_buf());
        }
    }
}

fn sorted_dir_entries(directory: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read source entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}
