//! Build tasks for aplan.
//!
//! - `gen-schema`: the committed `schemas/v1/task.json` is *derived* from the
//!   Rust types, and `--check` fails when the two disagree. That is the only
//!   reason the schema can be trusted as documentation.
//! - `gen-roadmap`: the committed `ROADMAP.status.md` is *generated* from the plan
//!   by `aplan roadmap --format markdown`, and `--check` fails on drift (ADR-0009).
//!   The plan is the source of truth; that file is a generated view of it.
//! - `lint`: the house-style gate. It forbids the typographic characters that
//!   read as machine-generated (em and en dashes, curly quotes, the ellipsis),
//!   the non-ASCII dash variants that stand in for a plain `-`, and the invisible
//!   zero-width and exotic-space characters (which hide in a diff and double as a
//!   trojan-source vector), so the repo stays plain-ASCII. Functional Unicode is
//!   deliberately untouched: the arrow used in cycle output, the section sign in
//!   doc cross-references, and the non-ASCII letters in the slug tests are fine.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// A function that derives a JSON Schema from a Rust type.
type SchemaFn = fn() -> serde_json::Value;

/// The committed schemas, each derived from a Rust type. Adding a schema-derived
/// type is one row here.
const SCHEMAS: &[(&str, SchemaFn)] = &[
    ("schemas/v1/task.json", aplan_core::load::derived_schema),
    ("schemas/v1/config.json", aplan_config::derived_schema),
];

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let check = args.any(|a| a == "--check");

    match cmd.as_deref() {
        Some("gen-schema") => gen_schema(check),
        Some("gen-roadmap") => gen_roadmap(check),
        Some("lint") => lint(),
        Some(other) => bail!(
            "unknown xtask `{other}` (expected: gen-schema [--check], gen-roadmap [--check], lint)"
        ),
        None => {
            eprintln!("usage: cargo xtask <gen-schema [--check] | gen-roadmap [--check] | lint>");
            Ok(())
        }
    }
}

fn gen_schema(check: bool) -> Result<()> {
    let root = workspace_root()?;
    for (rel, derive) in SCHEMAS {
        gen_one(&root, rel, &derive(), check)?;
    }
    Ok(())
}

fn gen_one(root: &Path, rel: &str, schema: &serde_json::Value, check: bool) -> Result<()> {
    let mut rendered = serde_json::to_string_pretty(schema)?;
    rendered.push('\n');
    write_or_check(root, rel, &rendered, check, "gen-schema")
}

/// Generate `ROADMAP.status.md`, the plan's machine-checkable half rendered by
/// `aplan roadmap --format markdown` (ADR-0009). It calls the library directly,
/// as `gen-schema` calls `derived_schema`, so the `spec` CI job need not build the
/// `aplan` binary. `--check` fails when the committed file and the plan disagree.
fn gen_roadmap(check: bool) -> Result<()> {
    let root = workspace_root()?;
    let plan = aplan_core::load_plan(&root.join("aplan"));
    let graph = aplan_core::Graph::new(plan.tasks.iter().map(|lt| &lt.task));
    let rendered = aplan_output::render_roadmap_markdown(&graph.roadmap());
    write_or_check(&root, "ROADMAP.status.md", &rendered, check, "gen-roadmap")
}

/// Write a generated file, or (with `check`) fail if the committed copy differs.
/// The one shape every generated-and-gated artifact shares (ADR-0009): a whole-file
/// compare, regenerated with a documented command.
fn write_or_check(root: &Path, rel: &str, rendered: &str, check: bool, regen: &str) -> Result<()> {
    let path = root.join(rel);
    if check {
        let on_disk = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if on_disk != rendered {
            bail!(
                "{rel} is stale: the committed copy and its generated form disagree.\n\
                 Run `cargo xtask {regen}` and commit the result."
            );
        }
        println!("{rel} is up to date");
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {rel}");
    Ok(())
}

/// Typographic characters forbidden in tracked text, each with the plain-ASCII
/// intent. Written as `\u{...}` escapes so this source stays lint-clean.
///
/// Three families, all of which read as machine-generated or are outright
/// invisible: the visible tells (fancy dashes, curly quotes, the ellipsis), the
/// non-ASCII dash variants that a model or a copy-paste substitutes for a plain
/// `-`, and the zero-width / exotic-space characters that are invisible in a diff
/// and double as a trojan-source vector. Functional Unicode (the arrow in cycle
/// output, the section sign, accented letters in the slug tests) is deliberately
/// left alone.
const FORBIDDEN: &[(char, &str)] = &[
    // Visible tells.
    (
        '\u{2014}',
        "em dash (use ',' ':' ';' parentheses, or split the sentence)",
    ),
    ('\u{2013}', "en dash (use '-' or 'to')"),
    ('\u{2018}', "left single quote (use ')"),
    ('\u{2019}', "right single quote / curly apostrophe (use ')"),
    ('\u{201C}', "left double quote (use straight \")"),
    ('\u{201D}', "right double quote (use straight \")"),
    ('\u{2026}', "ellipsis character (use three dots)"),
    // Dash variants that stand in for a plain ASCII '-'.
    (
        '\u{2015}',
        "horizontal bar (use '-', or split the sentence)",
    ),
    ('\u{2012}', "figure dash (use '-')"),
    ('\u{2212}', "minus sign (use '-')"),
    ('\u{2010}', "Unicode hyphen (use ASCII '-')"),
    ('\u{2011}', "non-breaking hyphen (use ASCII '-')"),
    // Invisible: zero-width characters and exotic spaces.
    ('\u{00A0}', "non-breaking space (use a normal space)"),
    ('\u{202F}', "narrow no-break space (use a normal space)"),
    ('\u{2009}', "thin space (use a normal space)"),
    ('\u{200A}', "hair space (use a normal space)"),
    ('\u{200B}', "zero-width space (delete it)"),
    ('\u{200C}', "zero-width non-joiner (delete it)"),
    ('\u{200D}', "zero-width joiner (delete it)"),
    ('\u{2060}', "word joiner (delete it)"),
    ('\u{FEFF}', "zero-width no-break space / BOM (delete it)"),
];

fn describe(c: char) -> Option<&'static str> {
    FORBIDDEN.iter().find(|(f, _)| *f == c).map(|(_, d)| *d)
}

/// Every forbidden character in `text`, as `(line, column, char)`, both
/// 1-indexed. Split out from the file walk so it is directly testable.
fn scan(text: &str) -> Vec<(usize, usize, char)> {
    let mut hits = Vec::new();
    for (li, line) in text.lines().enumerate() {
        for (ci, c) in line.chars().enumerate() {
            if describe(c).is_some() {
                hits.push((li + 1, ci + 1, c));
            }
        }
    }
    hits
}

/// The style gate: no typographic tells in any tracked text file.
fn lint() -> Result<()> {
    let root = workspace_root()?;
    let files = tracked_files(&root)?;

    let mut violations = 0usize;
    let mut files_hit = 0usize;
    for rel in &files {
        // Non-UTF-8 (binary) or unreadable files are not text; skip them.
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        let hits = scan(&text);
        if hits.is_empty() {
            continue;
        }
        files_hit += 1;
        for (line, col, c) in hits {
            violations += 1;
            let desc = describe(c).unwrap_or("forbidden character");
            println!("{rel}:{line}:{col}: U+{:04X} {desc}", c as u32);
        }
    }

    if violations == 0 {
        println!("style: clean ({} files scanned)", files.len());
        Ok(())
    } else {
        bail!(
            "{violations} style violation(s) across {files_hit} file(s). \
             Replace each with plain ASCII (hints above)."
        )
    }
}

/// The repo's tracked files, via `git ls-files`. The lint is dev tooling, not
/// the hermetic `aplan check`, so shelling out to git here is fine.
fn tracked_files(root: &Path) -> Result<Vec<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .context("running `git ls-files` (the lint scans tracked files)")?;
    if !out.status.success() {
        bail!(
            "`git ls-files` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn workspace_root() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR is `<root>/xtask`.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.parent()
        .map(std::path::Path::to_path_buf)
        .context("locating workspace root")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_each_typographic_tell() {
        // Built from escapes so this test source stays lint-clean.
        for (c, _) in FORBIDDEN {
            let s = format!("a{c}b");
            assert_eq!(
                scan(&s),
                vec![(1, 2, *c)],
                "should flag U+{:04X}",
                *c as u32
            );
        }
    }

    #[test]
    fn allows_functional_unicode_and_plain_ascii() {
        // Arrow (U+2192), section sign (U+00A7), accented letters, emoji: all fine.
        assert!(scan("A \u{2192} B; see \u{00A7}5; caf\u{00E9}; \u{1F30D}").is_empty());
        assert!(scan("plain ASCII: a, b; c (d). 2013-2014").is_empty());
    }

    #[test]
    fn reports_one_based_line_and_column() {
        let em = '\u{2014}';
        let hits = scan(&format!("line one\nx{em}y"));
        assert_eq!(hits, vec![(2, 2, em)]);
    }
}
