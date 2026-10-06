//! Reading this workspace's own production sources, for the gates that scan
//! them.
//!
//! Two gates do: `every_plan_write_goes_through_save` (`A1-270`) asks which
//! code writes a plan file, and `every_process_production_starts_is_named_with_its_reason`
//! (`A1-354`) asks which code starts a process. Both had the same two defects,
//! because the second was written from the first.
//!
//! **One directory level.** A walk that reads `crates/*/src` without recursing
//! is complete only while no crate has a subdirectory there. The first
//! `src/thing/mod.rs` would have been invisible to both, and a scan that
//! silently stops covering a crate is the failure `A1-270`'s own audit was
//! about: it looks like coverage.
//!
//! **Everything after the first `#[cfg(test)]`.** Both split the file at the
//! first occurrence and scanned only what came before, which assumes a file is
//! production followed by tests. Five files in this workspace are not shaped
//! like that. `aplan-output/src/lib.rs` has its first test module at line 2,661
//! of 6,002 and forty top-level items after it; `aplan-config/src/user.rs` has
//! its first at line 77 of 398. **So both gates were blind to more than half of
//! `aplan-output` and most of `user.rs`.** Nothing was hiding there when this
//! was found, which is the only reason it was a latent hole rather than a
//! defect, and is also why it would never have been noticed.
//!
//! [`production_lines`] skips the `#[cfg(test)]` *item* instead, by brace depth,
//! so a second production section after a test module is read like any other.

use std::path::{Path, PathBuf};

/// The root of this workspace.
pub fn workspace_root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    here.canonicalize().unwrap_or(here)
}

/// Every `.rs` file under the shipped crates, at every depth.
///
/// `xtask` is not here: see `all_sources` for the walk that includes it, and
/// the gate that wants it.
pub fn shipped_sources(root: &Path) -> Vec<PathBuf> {
    let crates: Vec<PathBuf> = std::fs::read_dir(root.join("crates"))
        .expect("the workspace has a crates directory")
        .flatten()
        .map(|e| e.path())
        .collect();
    let mut dirs: Vec<PathBuf> = crates
        .iter()
        .map(|c| c.join("src"))
        .filter(|p| p.is_dir())
        .collect();
    assert!(
        dirs.len() > 7,
        "the walk found only {} crate(s) with sources, so it cannot have seen the workspace",
        dirs.len()
    );
    let mut out = collect(&mut dirs);
    // **Build scripts, which are not under `src`.** `aplan-attest/build.rs`
    // asks `git` for the commit the binary was built from, and it runs on an
    // adopter's machine when they build the crate. Both scans were blind to it
    // until this was found (`A1-354`, audit): a walk of `*/src` is not a walk
    // of the code that ships.
    out.extend(
        crates
            .iter()
            .map(|c| c.join("build.rs"))
            .filter(|p| p.is_file()),
    );
    out.sort();
    out
}

/// Every `.rs` file under the shipped crates **and** `xtask`.
///
/// `A1-270`'s gate wants `xtask` because it writes generated artefacts into the
/// repository, so it is code that can clobber a plan file. `A1-354`'s does not,
/// because running `cargo` and `git` is what a build tool is for and nothing
/// links it into the binary an adopter runs.
pub fn all_sources(root: &Path) -> Vec<PathBuf> {
    let mut out = shipped_sources(root);
    let mut xtask = vec![root.join("xtask/src")];
    out.extend(collect(&mut xtask));
    out.sort();
    out
}

fn collect(dirs: &mut Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The production lines of `text`, numbered from one.
///
/// A `#[cfg(test)]` item is skipped, rather than everything after the first one,
/// and where it ends is decided by brace depth over code with **string literals
/// and comments blanked out first** (`code_only`).
///
/// Two simpler rules were tried and both failed on the first run, for the same
/// reason: a gate that counts delimiters in raw source is reading the contents
/// of string literals as if they were code.
///
/// - Counting braces directly: a `}` inside a string in `aplan-attest`'s test
///   module closed the count early, and `A1-270`'s gate reported three fixture
///   writes as defects.
/// - Ending the item at a line whose indentation matched the attribute's: in
///   `aplan-config` a multi-line string ends with `";` at column 0, which the
///   non-block-item escape hatch read as the end of `mod tests`, and the gate
///   reported a test helper called `write`.
///
/// So the lexing is done once, properly, and shared. `no_test_code_survives`
/// asserts the result: a `#[test]` attribute only ever appears inside a
/// `#[cfg(test)]` item, so one surviving into the production view means the skip
/// failed somewhere, which is how both bugs above would have been caught at once.
pub fn production_lines(text: &str) -> Vec<(usize, &str)> {
    let raw: Vec<&str> = text.lines().collect();
    let code = code_only(text);
    let mut out = Vec::new();
    let mut inside = false;
    let mut opened = false;
    let mut depth = 0i32;
    for (index, line) in code.iter().enumerate() {
        if inside {
            depth += net_braces(line);
            if !opened && depth > 0 {
                opened = true;
            }
            if opened && depth <= 0 {
                inside = false;
            }
            continue;
        }
        if line.trim_start().starts_with("#[cfg(test)]") {
            inside = true;
            depth = net_braces(line);
            opened = depth > 0;
            // An attribute on a one-line item carries its own close, and one on
            // an item with no block at all (`#[cfg(test)] use ..;`) is over at
            // its semicolon. Neither shape exists in this workspace today --
            // all seventy-odd are `mod <name> {` -- but a run that never found
            // a brace would otherwise swallow the rest of the file, which is
            // the defect this function exists to remove.
            if !opened && line.trim_end().ends_with(';') {
                inside = false;
            }
            continue;
        }
        out.push((index + 1, raw[index]));
    }
    out
}

/// `text`, line by line, with the contents of string literals and comments
/// replaced by spaces so that braces inside them cannot be miscounted.
///
/// A line comment keeps its `//` marker, so `is_comment` still recognises one.
/// Char literals are consumed when a closing quote is in reach, which keeps
/// `'{'` from counting as a brace while leaving a lifetime alone.
fn code_only(text: &str) -> Vec<String> {
    enum State {
        Code,
        Block,
        Str,
        Raw(usize),
    }
    let mut state = State::Code;
    let mut out = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut kept = String::with_capacity(line.len());
        let mut i = 0;
        while i < chars.len() {
            match state {
                State::Code => {
                    let next = chars.get(i + 1).copied();
                    if chars[i] == '/' && next == Some('/') {
                        kept.push_str("//");
                        break;
                    }
                    if chars[i] == '/' && next == Some('*') {
                        state = State::Block;
                        i += 2;
                        continue;
                    }
                    if chars[i] == 'r' && matches!(next, Some('"' | '#')) {
                        let mut hashes = 0;
                        let mut j = i + 1;
                        while chars.get(j) == Some(&'#') {
                            hashes += 1;
                            j += 1;
                        }
                        if chars.get(j) == Some(&'"') {
                            state = State::Raw(hashes);
                            i = j + 1;
                            continue;
                        }
                    }
                    if chars[i] == '"' {
                        state = State::Str;
                        i += 1;
                        continue;
                    }
                    if chars[i] == '\'' {
                        // `'a'` and `'\n'`; anything longer is a lifetime.
                        if chars.get(i + 2) == Some(&'\'') {
                            i += 3;
                            continue;
                        }
                        if chars.get(i + 1) == Some(&'\\') && chars.get(i + 3) == Some(&'\'') {
                            i += 4;
                            continue;
                        }
                    }
                    kept.push(chars[i]);
                    i += 1;
                }
                State::Str => {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == '"' {
                        state = State::Code;
                    }
                    i += 1;
                }
                State::Raw(hashes) => {
                    if chars[i] == '"' {
                        let mut seen = 0;
                        let mut j = i + 1;
                        while seen < hashes && chars.get(j) == Some(&'#') {
                            seen += 1;
                            j += 1;
                        }
                        if seen == hashes {
                            state = State::Code;
                            i = j;
                            continue;
                        }
                    }
                    i += 1;
                }
                State::Block => {
                    if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        state = State::Code;
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
            }
        }
        out.push(kept);
    }
    out
}

fn net_braces(line: &str) -> i32 {
    let opens = i32::try_from(line.matches('{').count()).unwrap_or(i32::MAX);
    let closes = i32::try_from(line.matches('}').count()).unwrap_or(i32::MAX);
    opens - closes
}

/// Whether this line is a comment, so a marker quoted in prose is not a hit.
pub fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
}

/// The name of the function this line declares, if it declares one.
pub fn enclosing_fn(line: &str) -> Option<&str> {
    let t = line.trim_start();
    for prefix in [
        "pub(crate) async fn ",
        "pub(super) async fn ",
        "pub async fn ",
        "pub(crate) const fn ",
        "pub const fn ",
        "pub(crate) fn ",
        "pub(super) fn ",
        "async fn ",
        "const fn ",
        "pub fn ",
        "fn ",
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return Some(rest.split(['(', '<', ' ']).next().unwrap_or(rest));
        }
    }
    None
}
