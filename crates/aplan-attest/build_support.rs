//! Git input discovery shared by the build script and its executable tests.
//!
//! Cargo treats a missing `rerun-if-changed` input as dirty forever and walks a
//! watched directory recursively. Every path returned here is therefore an
//! existing file, and a symbolic `HEAD` is followed only to its exact ref (or
//! exact reflog while that ref is packed).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(manifest_dir: &Path, args: &[&str]) -> Option<Output> {
    let mut command = Command::new("git");
    command.args(args).current_dir(manifest_dir);
    // A build can be launched by a hook whose relative `GIT_DIR` is meaningful
    // in the hook's working directory but not in this crate. Clearing only
    // `git rev-parse --local-env-vars` is insufficient: discovery controls and
    // configuration injection (`GIT_CEILING_DIRECTORIES`, `GIT_CONFIG_*`, ...)
    // live outside that list, and Git may add another variable later. The
    // source directory is the complete authority for this read, so no `GIT_`
    // value inherited from its caller belongs here (`A1-638`).
    for (name, _) in std::env::vars_os().filter(|(name, _)| {
        name.to_string_lossy()
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("GIT_"))
    }) {
        command.env_remove(name);
    }
    command
        .output()
        .ok()
        .filter(|output| output.status.success())
}

fn one_line(output: Output) -> Option<String> {
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim_end_matches(['\r', '\n']);
    (!value.is_empty() && !value.contains(['\r', '\n'])).then(|| value.to_owned())
}

fn git_stdout(manifest_dir: &Path, args: &[&str]) -> Option<String> {
    one_line(git(manifest_dir, args)?)
}

fn git_path(manifest_dir: &Path, name: &str) -> Option<PathBuf> {
    // `--path-format=absolute` avoids making Cargo interpret a relative path in
    // its own frame. The fallback retains compatibility with older Git builds;
    // `--git-path` then writes a path relative to this command's current dir.
    let value = git_stdout(
        manifest_dir,
        &["rev-parse", "--path-format=absolute", "--git-path", name],
    )
    .or_else(|| git_stdout(manifest_dir, &["rev-parse", "--git-path", name]))?;
    let path = PathBuf::from(value);
    Some(if path.is_absolute() {
        path
    } else {
        manifest_dir.join(path)
    })
}

fn existing_file(path: &Path) -> Option<PathBuf> {
    path.is_file()
        .then(|| std::fs::canonicalize(path).ok())
        .flatten()
}

fn existing_directory(path: &Path) -> Option<PathBuf> {
    path.is_dir()
        .then(|| std::fs::canonicalize(path).ok())
        .flatten()
}

fn nearest_existing_ref_directory(manifest_dir: &Path, reference: &str) -> Option<PathBuf> {
    let refs = git_path(manifest_dir, "refs").and_then(|path| existing_directory(&path))?;
    let reference = git_path(manifest_dir, reference)?;
    let mut candidate = reference.parent();
    while let Some(path) = candidate.filter(|path| path.starts_with(&refs)) {
        if let Some(path) = existing_directory(path) {
            return Some(path);
        }
        candidate = path.parent();
    }
    None
}

fn symbolic_ref(head: &Path) -> Option<String> {
    let bytes = std::fs::read(head).ok()?;
    let reference = bytes.strip_prefix(b"ref: ")?;
    let reference = std::str::from_utf8(reference).ok()?;
    let reference = reference.trim_end_matches(['\r', '\n']);
    (reference.starts_with("refs/") && !reference.is_empty() && !reference.contains(['\r', '\n']))
        .then(|| reference.to_owned())
}

/// Existing Git files whose content can change the commit named by `HEAD`.
pub(crate) fn git_watch_paths(manifest_dir: &Path) -> Vec<PathBuf> {
    let Some(head) = git_path(manifest_dir, "HEAD").and_then(|path| existing_file(&path)) else {
        return Vec::new();
    };
    let reference = symbolic_ref(&head);
    let mut paths = vec![head];
    let Some(reference) = reference else {
        return paths;
    };

    if let Some(loose) = git_path(manifest_dir, &reference).and_then(|path| existing_file(&path)) {
        paths.push(loose);
        return paths;
    }

    // A commit from a packed ref creates a loose ref. Its exact reflog changes
    // in the same operation and is the existing, branch-local signal for that
    // transition. Watching the parent ref directory would make other branches
    // rebuild this checkout; watching the absent loose path would make every
    // build dirty.
    let branch_reflog = format!("logs/{reference}");
    let reflog = git_path(manifest_dir, &branch_reflog)
        .and_then(|path| existing_file(&path))
        .or_else(|| git_path(manifest_dir, "logs/HEAD").and_then(|path| existing_file(&path)));
    if let Some(reflog) = reflog {
        paths.push(reflog);
    } else if let Some(directory) = nearest_existing_ref_directory(manifest_dir, &reference) {
        // Reflogs can be disabled. A packed branch then has no exact existing
        // path that changes when its next commit creates a loose ref. Watching
        // the nearest existing ref directory is the correctness fallback; the
        // rerun immediately switches back to the exact loose file. This may
        // observe a sibling ref once, but only in the exceptional no-reflog
        // state (`A1-638`). It also covers an unborn symbolic HEAD.
        paths.push(directory);
    }
    if let Some(packed) =
        git_path(manifest_dir, "packed-refs").and_then(|path| existing_file(&path))
    {
        paths.push(packed);
    }
    paths
}

/// Existing source files that can change the build script's behavior.
pub(crate) fn script_watch_paths(manifest_dir: &Path) -> Vec<PathBuf> {
    ["build.rs", "build_support.rs"]
        .into_iter()
        .filter_map(|name| existing_file(&manifest_dir.join(name)))
        .collect()
}

/// The checked-out commit, queried without inheriting a caller's Git context.
pub(crate) fn git_head(manifest_dir: &Path) -> Option<String> {
    git_stdout(manifest_dir, &["rev-parse", "--verify", "HEAD"])
}

/// A path that can be represented by one Cargo build-script directive.
pub(crate) fn directive_path(path: &Path) -> Option<&str> {
    path.to_str().filter(|value| !value.contains(['\r', '\n']))
}
