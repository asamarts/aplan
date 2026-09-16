//! GitHub-compatible heading anchors, and the content fingerprint APLAN018 uses.
//!
//! Two consumers share this: `validate` resolves a `traces` anchor to a real
//! heading (APLAN008) and compares a stored fingerprint against the live one
//! (APLAN018), and `close` computes the fingerprint to stamp at close time. The
//! logic is content-addressed, not location- or time-addressed, so a fingerprint
//! survives rebases, file moves, and trailing-whitespace / line-ending churn. A
//! reflow or reindent of the section is a real content edit and does register,
//! because whitespace can be significant (code, YAML, a Makefile recipe) (HLD 5.6).

use std::collections::{BTreeMap, BTreeSet};

/// A heading found in a document: its GitHub anchor, its 0-based line, and its
/// level (1-6). Levels bound the section a fingerprint covers.
struct Heading {
    anchor: String,
    /// The heading text as written, before slugging.
    ///
    /// The anchor cannot stand in for it: slugging drops the dots, so a `### v0.8,
    /// Adoption` heading anchors as `v08-adoption` and a caller matching a
    /// heading against a cut label would be matching against a mangled copy of it
    /// (`A1-165`).
    title: String,
    line: usize,
    level: u8,
}

/// Every heading in `text`, in document order, with GitHub's anchors,
/// disambiguated exactly as `github-slugger` does (a repeated slug gains a `-1`,
/// `-2`, ... suffix keyed on the base slug).
///
/// Both forms GitHub anchors are recognised: ATX (`# Title`) and setext (a text
/// line underlined by `===` for h1 or `---` for h2). Fenced code blocks
/// (```` ``` ````/`~~~`) are tracked and their contents skipped, so a `#` line
/// inside a code sample is not mistaken for a heading, GitHub does not anchor
/// those.
fn headings(text: &str) -> Vec<Heading> {
    // Strip a leading BOM the way GitHub (and the task loader) does; without this
    // a `\u{feff}# Title` first line is not seen as a heading, so a valid anchor
    // would be reported dangling (`APLAN008`) and could not be fingerprinted.
    let text = text.trim_start_matches('\u{feff}');
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut fence: Option<(char, usize)> = None; // (marker, run length) when open
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some((ch, len, bare)) = code_fence(line) {
            match fence {
                None => fence = Some((ch, len)),
                // A closing fence must have no info string (CommonMark); a
                // ```lang line while a block is open is content, not a close.
                Some((fch, flen)) if ch == fch && len >= flen && bare => fence = None,
                Some(_) => {}
            }
            i += 1;
            continue;
        }
        if fence.is_some() {
            i += 1;
            continue;
        }
        if let Some((title, level)) = atx_heading(line) {
            out.push(Heading {
                anchor: mint(&mut counts, title),
                title: title.to_string(),
                line: i,
                level,
            });
            i += 1;
            continue;
        }
        // Setext: a non-blank text line whose very next line underlines it. A
        // blank line before the underline would make the `---`/`===` a thematic
        // break, so requiring the text line to be non-blank is the disambiguator.
        if is_setext_text(line)
            && let Some(level) = lines.get(i + 1).copied().and_then(setext_level)
        {
            out.push(Heading {
                anchor: mint(&mut counts, line.trim()),
                title: line.trim().to_string(),
                line: i,
                level,
            });
            i += 2; // consume the underline too
            continue;
        }
        i += 1;
    }
    out
}

/// One heading and the lines beneath it.
#[derive(Debug, Clone)]
pub struct Section {
    /// The heading text as written, emphasis and links intact.
    pub title: String,
    /// The heading's level, 1 to 6.
    pub level: u8,
    /// The lines under the heading, the heading line itself excluded.
    pub body: String,
    /// The heading's own 0-based line.
    pub line: usize,
    /// One past the section's last line, so `line + 1 .. end` is `body`.
    ///
    /// Carried so a caller that needs a *narrower* body than "down to the next
    /// heading that is not deeper" can compute one without re-finding the
    /// headings: `A1-165` has to stop a cut's section where another cut's
    /// section begins, which is a subsection and so inside this one.
    pub end: usize,
}

/// Every heading in `text` with the section it opens, in document order.
///
/// The section runs to the next heading that is not deeper, which is the same
/// boundary `anchor_fingerprint` uses -- deliberately the same, and by the same
/// `headings` scan, so a document is cut into sections one way here rather than
/// two ways that agree until they do not. Subsections are therefore part of
/// their parent's body as well as sections in their own right.
#[must_use]
pub fn sections(text: &str) -> Vec<Section> {
    let text = text.trim_start_matches('\u{feff}');
    let hs = headings(text);
    let lines: Vec<&str> = text.lines().collect();
    hs.iter()
        .enumerate()
        .map(|(i, h)| {
            let end = hs[i + 1..]
                .iter()
                .find(|n| n.level <= h.level)
                .map_or(lines.len(), |n| n.line);
            // `h.line + 1` skips the heading itself; a setext heading's underline
            // is the line after that and is left in, where it is inert.
            let start = (h.line + 1).min(end);
            Section {
                title: h.title.clone(),
                level: h.level,
                body: lines[start..end].join("\n"),
                line: h.line,
                end,
            }
        })
        .collect()
}

/// The lines of `text` that are not inside a fenced code block, with their
/// 0-based line numbers.
///
/// Exposed because a document's *examples* are not its content, and every reader
/// of a document has to agree about that or they disagree in the worst way. The
/// heading scan and the table-row scan have skipped fences since they were
/// written; `A1-165` shipped a third reader -- "the line beginning with this
/// marker" -- that did not, so a fenced sample of a plan document's own format
/// was read as a declaration of membership, while the same sample as a table was
/// correctly ignored. One rule, in one place, is the only way that stays true.
#[must_use]
pub fn unfenced_lines(text: &str) -> Vec<(usize, &str)> {
    let text = text.trim_start_matches('\u{feff}');
    let mut fence: Option<(char, usize)> = None;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if let Some((ch, len, bare)) = code_fence(line) {
            match fence {
                None => fence = Some((ch, len)),
                Some((fch, flen)) if ch == fch && len >= flen && bare => fence = None,
                Some(_) => {}
            }
            continue;
        }
        if fence.is_none() {
            out.push((i, line));
        }
    }
    out
}

/// A heading's text with its link targets removed, so `[a](b)` reads as `[a]`.
///
/// The rule GitHub applies when it mints an anchor, exposed for a caller that
/// needs the heading's words rather than its slug: slugging folds the case and
/// drops the dots, so a `v0.8` heading slugs as `v08` and nothing matching a cut
/// label against it is matching against the label.
#[must_use]
pub fn heading_text(title: &str) -> String {
    strip_link_targets(title)
}

/// The first cell of every table row in `text`, in document order.
///
/// ADR-0015 settled that a table row is addressed by its own first cell, and this
/// is that rule read for a different purpose: a document that lists a cut's tasks
/// as a table declares each member in the anchor position, and the prose in the
/// remaining cells is prose. Measured on the pilot whose plan carries a task
/// table per cut, that distinction is the whole difference between a usable check
/// and an unusable one -- six of its rows mention another cut's task in the
/// second cell, and reading the whole row calls all six a false declaration
/// (`A1-165`).
///
/// Header rows and fenced code are skipped, because `table_rows` skips them.
#[must_use]
pub fn table_row_cells(text: &str) -> Vec<String> {
    // Stripped here as well, so these line indices are `table_rows`' own.
    let text = text.trim_start_matches('\u{feff}');
    let lines: Vec<&str> = text.lines().collect();
    table_rows(text)
        .into_iter()
        .filter_map(|r| lines.get(r.line).and_then(|l| first_cell(l)))
        .map(str::to_string)
        .collect()
}

/// Every anchor GitHub would mint for the headings in `text`.
pub fn heading_anchors(text: &str) -> BTreeSet<String> {
    headings(text).into_iter().map(|h| h.anchor).collect()
}

/// Every anchor a `traces` entry may name: the headings, and the table rows a
/// document addresses by their first cell (`A1-095`).
///
/// `APLAN008` resolves against this rather than the headings alone, or a
/// row-scoped trace would be reported dangling on the day it was written.
pub fn resolvable_anchors(text: &str) -> BTreeSet<String> {
    let mut out = heading_anchors(text);
    out.extend(table_rows(text).into_iter().map(|r| r.anchor));
    out
}

/// One row of a markdown table, addressable by its first cell (`A1-095`).
struct Row {
    anchor: String,
    line: usize,
}

/// Every table row in `text`, anchored by its own first cell.
///
/// A heading is the wrong unit for a document whose shape is a register: p00020
/// keeps twelve invariants in one table under one heading, so a task tracing that
/// heading is re-flagged by an edit to any of the other eleven and the answer is
/// `--restamp` every time (pre-public adoption evidence, item 14). A row is the fact.
///
/// Minted with the heading slug rule, so `| Invariant 7 | ... |` is
/// `#invariant-7`, and on a **separate** counter from the headings: sharing one
/// would make a row's anchor move when an unrelated heading was added, which is
/// the positional brittleness this is here to avoid.
fn table_rows(text: &str) -> Vec<Row> {
    let text = text.trim_start_matches('\u{feff}');
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut fence: Option<(char, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some((ch, len, bare)) = code_fence(line) {
            match fence {
                None => fence = Some((ch, len)),
                Some((fch, flen)) if ch == fch && len >= flen && bare => fence = None,
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let Some(cell) = first_cell(line) else {
            continue;
        };
        // The header row names the columns, not a fact, and markdown says exactly
        // which row it is: the one the delimiter follows. Anchoring it would offer
        // `#invariant` for a table of invariants, which is a trap.
        if lines.get(i + 1).is_some_and(|n| is_delimiter_row(n)) {
            continue;
        }
        out.push(Row {
            anchor: mint(&mut counts, cell),
            line: i,
        });
    }
    out
}

/// The first cell of a table row, or `None` for anything that is not one.
///
/// A row is a line starting with `|` and holding a second `|`. The separator
/// (`|---|`) is not a row, and neither is a cell that slugifies to nothing, since
/// an anchor nobody can write is not an anchor.
fn first_cell(line: &str) -> Option<&str> {
    if is_delimiter_row(line) {
        return None;
    }
    let t = line.trim();
    let rest = t.strip_prefix('|')?;
    let end = rest.find('|')?;
    let cell = rest[..end].trim();
    (!slugify(cell).is_empty()).then_some(cell)
}

/// The `|---|---|` line under a table's header.
fn is_delimiter_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// The content fingerprint of the section anchored at `anchor` in `text`, or
/// `None` if no heading mints that anchor.
///
/// The section runs from the anchor's heading to the next heading of the same or
/// a higher level (or end of document), so it includes subsections. Only trailing
/// whitespace is stripped and line endings are normalised (via `lines()`) before
/// hashing; leading whitespace, blank lines, and internal spacing are kept, so a
/// change to indentation-significant content (a Python block, a YAML nesting, a
/// Makefile recipe) is caught. A fingerprint therefore survives trailing-space
/// and line-ending churn, rebases, and file moves, but a reflow or reindent of the
/// section IS a content edit and does register. The hash is a stable,
/// dependency-free FNV-1a; this is drift detection, not security, so collision
/// resistance is not a concern.
pub fn anchor_fingerprint(text: &str, anchor: &str) -> Option<String> {
    let text = text.trim_start_matches('\u{feff}');
    // GitHub anchors are lowercase; fold the caller's fragment to match, so a
    // stamp and its later check agree however the `traces` entry was cased.
    let anchor = anchor.to_lowercase();
    let hs = headings(text);
    let Some(idx) = hs.iter().position(|h| h.anchor == anchor) else {
        // Not a heading: a table row addressed by its first cell, hashed alone so
        // an edit to a sibling row is not this task's drift (`A1-095`).
        //
        // Headings are tried first, so a document with both a `## Decision` and a
        // `| Decision |` row means the heading by `#decision`, which keeps every
        // existing plan's fingerprint exactly as it was.
        let row = table_rows(text).into_iter().find(|r| r.anchor == anchor)?;
        let line = text.lines().nth(row.line)?;
        return Some(fnv1a_hex(line.trim_end()));
    };
    let level = hs[idx].level;
    let start = hs[idx].line;
    let lines: Vec<&str> = text.lines().collect();
    // The section ends at the next heading that is not deeper than this one.
    let end = hs[idx + 1..]
        .iter()
        .find(|h| h.level <= level)
        .map_or(lines.len(), |h| h.line);

    Some(fnv1a_hex(&section_text(&lines[start..end])))
}

/// Whether `stored` still describes the section `anchor` names.
///
/// Accepts the form written before trailing blank lines were trimmed, because
/// that is what every stamp in every plan already holds. Without it this change
/// would re-flag every traced task in every repository at once, which is the
/// warning-flood it exists to prevent (`A1-122`).
///
/// The compatibility is one-way and self-clearing: `close --restamp` writes the
/// trimmed form, so a plan converges as tasks are re-reviewed for other reasons.
#[must_use]
pub fn fingerprint_holds(text: &str, anchor: &str, stored: &str) -> bool {
    if anchor_fingerprint(text, anchor).as_deref() == Some(stored) {
        return true;
    }
    legacy_fingerprint(text, anchor).as_deref() == Some(stored)
}

/// The fingerprint this tool wrote before `A1-122`: the same section text with
/// its trailing blank lines left in.
fn legacy_fingerprint(text: &str, anchor: &str) -> Option<String> {
    let text = text.trim_start_matches('\u{feff}');
    let anchor = anchor.to_lowercase();
    let hs = headings(text);
    let idx = hs.iter().position(|h| h.anchor == anchor)?;
    let level = hs[idx].level;
    let start = hs[idx].line;
    let lines: Vec<&str> = text.lines().collect();
    let end = hs[idx + 1..]
        .iter()
        .find(|h| h.level <= level)
        .map_or(lines.len(), |h| h.line);
    Some(fnv1a_hex(
        &lines[start..end]
            .iter()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

/// A section's text, trailing blank lines removed.
///
/// The blank line before the *next* heading belongs to the document's layout,
/// not to this section's meaning: with it included, appending a new section
/// re-flags whoever traced the previously-last one, which is drift reported for
/// an edit that did not touch the section. It cost this repository a re-stamp of
/// `A1-138` the day after it closed, for a pilot log that only grew (`A1-122`).
fn section_text(lines: &[&str]) -> String {
    let mut end = lines.len();
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    lines[..end]
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A stable 16-hex-digit FNV-1a hash of `s`. Deterministic across platforms and
/// toolchains (unlike `std`'s `DefaultHasher`), which a stored fingerprint needs.
fn fnv1a_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Mint the next free anchor for `title`, mirroring github-slugger's stateful
/// `while (occurrences[result]) ...` loop.
fn mint(counts: &mut BTreeMap<String, u32>, title: &str) -> String {
    let base = slugify(title);
    let mut anchor = base.clone();
    while counts.contains_key(&anchor) {
        let n = counts.entry(base.clone()).or_insert(0);
        *n += 1;
        anchor = format!("{base}-{n}");
    }
    counts.insert(anchor.clone(), 0);
    anchor
}

/// A code-fence line: 3+ backticks or 3+ tildes, up to 3 spaces of indent,
/// returning `(marker, run length, bare)` where `bare` means no info string
/// follows the run. Only a bare fence may *close* a block (an info string on a
/// close is content, not a close); an opening fence may carry one. A backtick
/// fence's info string may not contain a backtick, which keeps an inline
/// `` `code` `` span from reading as a fence. Only used to toggle fence tracking,
/// so this is an approximation, not the whole fenced-code grammar.
fn code_fence(line: &str) -> Option<(char, usize, bool)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = rest.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let run = rest.chars().take_while(|&c| c == ch).count();
    if run < 3 {
        return None;
    }
    if ch == '`' && rest[run..].contains('`') {
        return None;
    }
    let bare = rest[run..].trim().is_empty();
    Some((ch, run, bare))
}

/// The level of a setext underline: `1` for a run of `=`, `2` for a run of `-`
/// (up to 3 spaces of indent, trailing spaces allowed); `None` otherwise.
fn setext_level(line: &str) -> Option<u8> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let trimmed = line[indent..].trim_end();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.bytes().all(|b| b == b'=') {
        Some(1)
    } else if trimmed.bytes().all(|b| b == b'-') {
        Some(2)
    } else {
        None
    }
}

/// Whether `line` can be setext heading *text*: non-blank, and not itself an ATX
/// heading, a code fence, or a setext underline.
fn is_setext_text(line: &str) -> bool {
    !line.trim().is_empty()
        && atx_heading(line).is_none()
        && code_fence(line).is_none()
        && setext_level(line).is_none()
}

/// If `line` is an ATX heading, return `(title, level)` with the opening marker
/// and any optional closing `#` sequence removed; otherwise `None`.
///
/// Handles 0-3 spaces of indentation, 1-6 opening `#`, the space (or end of line)
/// the marker requires, and a whitespace-separated trailing `#` run. Fenced code
/// blocks are tracked by [`headings`], so a `#`-prefixed line inside a code fence
/// never reaches this function as a heading.
fn atx_heading(line: &str) -> Option<(&str, u8)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None; // four or more spaces is an indented code block
    }
    let line = &line[indent..];
    let level = line.len() - line.trim_start_matches('#').len();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &line[level..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None; // `#foo` is not a heading; `# foo` is
    }
    let title = rest.trim();
    // A closing `#` run is content unless whitespace separates it from the text
    // (`## Foo ##` -> `Foo`), or the heading is nothing but `#`s.
    let title = match title.rsplit_once([' ', '\t']) {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b == b'#') => {
            head.trim_end()
        }
        _ if !title.is_empty() && title.bytes().all(|b| b == b'#') => "",
        _ => title,
    };
    Some((title, u8::try_from(level).unwrap_or(6)))
}

/// Slugify a heading the way GitHub does (via `github-slugger`): lowercase
/// (Unicode-aware), drop every character that is not a letter, digit, `_`, or
/// `-`, and turn each space into `-`.
///
/// Punctuation is *removed*, not hyphenated, so `2.1 Foo: bar` slugs to
/// `21-foo-bar`, never `2-1-foo-bar`. Spaces are not coalesced, so `a - b` slugs
/// to `a---b`. Unicode letters are kept and lowercased, not transliterated
/// (`Über` -> `über`, not `uber`).
/// A heading's text with the *targets* of inline links and images removed.
///
/// `github-slugger` runs on the rendered heading, so `## [Linked](http://e.com)`
/// is the text `Linked` and the anchor is `linked`. This crate slugifies the raw
/// source line, and punctuation-stripping alone left the URL's letters behind:
/// `linkedhttpecom`, an anchor nobody can copy from the rendered document. An
/// author who copied the real one was told their trace was dangling, and the only
/// way to satisfy the tool was to write the mangled form, which then does not link
/// (`A1-097`).
///
/// Removes what does not render: the `(...)` after a `]`, and the `[ref]` of a
/// reference link. The remaining brackets and the `!` of an image are ordinary
/// punctuation and `slugify` drops them, so `![Alt](src)` slugs as `alt`, the alt
/// text being what a reader sees.
fn strip_link_targets(title: &str) -> String {
    let chars: Vec<char> = title.chars().collect();
    let mut out = String::with_capacity(title.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ']'
            && let Some(&next) = chars.get(i + 1)
            && let Some(close) = matching(&chars, i + 1, next)
        {
            out.push(']');
            i = close + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The index closing the delimiter that `open` opens at `from`, or `None` when
/// `open` is not one of the pair characters or nothing closes it.
fn matching(chars: &[char], from: usize, open: char) -> Option<usize> {
    let close = match open {
        '(' => ')',
        '[' => ']',
        _ => return None,
    };
    let mut depth = 0usize;
    for (i, c) in chars.iter().enumerate().skip(from) {
        if *c == open {
            depth += 1;
        } else if *c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn slugify(title: &str) -> String {
    let title = strip_link_targets(title);
    let mut slug = String::with_capacity(title.len());
    for c in title.chars() {
        if c == ' ' {
            slug.push('-');
        } else if c == '_' || c == '-' {
            slug.push(c);
        } else if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        }
        // Any other character, punctuation, symbols, emoji, other whitespace, is
        // stripped, exactly as github-slugger's `replace(regex, '')` does.
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_matches_github() {
        // The example printed in the README. The previous algorithm mapped `.`
        // to `-` and produced `2-1-...`, so the README's own trace failed.
        assert_eq!(
            slugify("2.1 Delaware: the filing channel"),
            "21-delaware-the-filing-channel"
        );
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("a - b"), "a---b");
        assert_eq!(slugify("foo_bar-baz"), "foo_bar-baz");
        assert_eq!(slugify("Über Café"), "über-café");
        assert_eq!(slugify("Hello 🌍 World"), "hello--world");
    }

    #[test]
    fn atx_heading_detection() {
        assert_eq!(atx_heading("## Foo bar"), Some(("Foo bar", 2)));
        assert_eq!(atx_heading("## Foo ##"), Some(("Foo", 2)));
        assert_eq!(atx_heading("   ### Indented"), Some(("Indented", 3)));
        assert_eq!(atx_heading("###"), Some(("", 3)));
        assert_eq!(atx_heading("# Top"), Some(("Top", 1)));
        // Not headings:
        assert_eq!(atx_heading("#no-space"), None);
        assert_eq!(atx_heading("####### too many"), None);
        assert_eq!(atx_heading("    indented code"), None);
        assert_eq!(atx_heading("#include <stdio.h>"), None);
        assert_eq!(atx_heading("plain text"), None);
    }

    #[test]
    fn heading_anchors_disambiguate_repeats() {
        let anchors = heading_anchors("# Setup\n## Setup\n### Setup\n");
        assert!(anchors.contains("setup"));
        assert!(anchors.contains("setup-1"));
        assert!(anchors.contains("setup-2"));
        assert!(!anchors.contains("setup-3"));
    }

    #[test]
    fn setext_headings_get_anchors() {
        let anchors = heading_anchors("Intro Title\n===========\n\nSecond Level\n------------\n");
        assert!(anchors.contains("intro-title"), "setext h1: {anchors:?}");
        assert!(anchors.contains("second-level"), "setext h2: {anchors:?}");
    }

    /// A `---` after a blank line is a thematic break, not a setext underline, so
    /// the paragraph above it must NOT become a heading (no false anchor).
    #[test]
    fn a_thematic_break_is_not_a_setext_heading() {
        let anchors = heading_anchors("Just a paragraph.\n\n---\n");
        assert!(
            !anchors.contains("just-a-paragraph"),
            "paragraph over a thematic break became a heading: {anchors:?}"
        );
    }

    /// A `#` line inside a fenced code block is not a heading (GitHub does not
    /// anchor it), so it mints no anchor.
    #[test]
    fn a_heading_inside_a_code_fence_is_not_an_anchor() {
        let anchors = heading_anchors("# Real\n\n```\n# Fake In Fence\n```\n");
        assert!(anchors.contains("real"));
        assert!(
            !anchors.contains("fake-in-fence"),
            "code-fence content minted an anchor: {anchors:?}"
        );
    }

    const DOC: &str = "\
# Top

intro line

## Section A

alpha content
beta content

### Sub A1

nested content

## Section B

other content
";

    #[test]
    fn fingerprint_covers_a_section_and_its_subsections() {
        let a = anchor_fingerprint(DOC, "section-a").expect("anchor resolves");
        // `Section A` runs to `Section B` (same level 2), so it includes `Sub A1`.
        let with_sub_changed = DOC.replace("nested content", "nested CHANGED");
        let a2 = anchor_fingerprint(&with_sub_changed, "section-a").expect("resolves");
        assert_ne!(
            a, a2,
            "a change in a subsection must change the fingerprint"
        );
        // A change in a *different* top-level section does not.
        let other = DOC.replace("other content", "other CHANGED");
        let a3 = anchor_fingerprint(&other, "section-a").expect("resolves");
        assert_eq!(
            a, a3,
            "an unrelated section must not change the fingerprint"
        );
    }

    #[test]
    fn fingerprint_ignores_trailing_ws_and_eol_but_catches_significant_change() {
        let base = anchor_fingerprint(DOC, "section-a").expect("resolves");
        // Trailing whitespace and CRLF-vs-LF churn do not register.
        let churn = DOC.replace(
            "alpha content\nbeta content",
            "alpha content   \r\nbeta content\t",
        );
        assert_eq!(
            anchor_fingerprint(&churn, "section-a").expect("resolves"),
            base,
            "trailing-whitespace / EOL churn must not read as drift"
        );
        // But a leading-whitespace (indentation) change DOES register, because
        // whitespace is significant in code/YAML/Makefiles. This is the false
        // negative the prior normalisation had.
        let reindent = DOC.replace("alpha content", "    alpha content");
        assert_ne!(
            anchor_fingerprint(&reindent, "section-a").expect("resolves"),
            base,
            "indentation is significant content and must register"
        );
        // A blank-line (structural) change registers.
        let blanks = DOC.replace(
            "alpha content\nbeta content",
            "alpha content\n\nbeta content",
        );
        assert_ne!(
            anchor_fingerprint(&blanks, "section-a").expect("resolves"),
            base
        );
        // And a word change registers.
        let changed = DOC.replace("alpha content", "alpha DIFFERENT");
        assert_ne!(
            anchor_fingerprint(&changed, "section-a").expect("resolves"),
            base
        );
    }

    /// A leading BOM on the traced document must not hide its first heading.
    #[test]
    fn a_bom_does_not_hide_the_first_heading() {
        let doc = "\u{feff}# Overview\n\nbody\n";
        assert!(heading_anchors(doc).contains("overview"));
        assert!(anchor_fingerprint(doc, "overview").is_some());
    }

    /// An info string on a *closing* fence does not close the block, so a heading
    /// after it stays hidden until a bare fence closes the block.
    #[test]
    fn an_info_string_does_not_close_a_fence() {
        // The inner ```python is not a close; the block runs to the bare ```.
        let anchors = heading_anchors("## A\n\n```\ncode\n```python\nmore\n```\n\n## B\n");
        assert!(anchors.contains("a"));
        assert!(
            anchors.contains("b"),
            "heading after the closed block: {anchors:?}"
        );
    }

    #[test]
    fn fingerprint_is_none_for_an_unknown_anchor() {
        assert!(anchor_fingerprint(DOC, "does-not-exist").is_none());
    }
}

#[cfg(test)]
mod row_anchors {
    use super::*;

    const TABLE: &str = "\
# Constitution

| Invariant | Status | Gate |
|---|---|---|
| Invariant 7 | Live | RD0-11 |
| Invariant 11 | Planned | RD0-15 |
";

    /// A list item is not an anchor, and the boundary is pinned so it cannot move
    /// by accident.
    ///
    /// Declined deliberately (`A1-174`): a table row has a first cell, a short
    /// name that survives the prose around it being rewritten, and that is what
    /// makes a row anchor stable while its content drifts. A bullet has no such
    /// position, so its identity would be its whole text -- reword it and the
    /// reference breaks instead of reporting drift, which is the opposite of what
    /// ADR-0015 bought. Measured across nine repositories: of 16,632 list items,
    /// 71.5% are plain prose and only 10% carry a leading span short enough to be
    /// a name.
    #[test]
    fn a_list_item_is_not_an_anchor() {
        let doc = concat!(
            "## Follow-ups\n\n",
            "- Follow-up one: the first thing\n",
            "- **Follow-up two**: the second thing\n",
            "- `follow-up-three`: the third\n\n",
            "| Row one | detail |\n|---|---|\n| Row two | detail |\n",
        );
        let anchors = resolvable_anchors(doc);
        // The heading and the table row are anchors.
        assert!(anchors.contains("follow-ups"), "{anchors:?}");
        assert!(anchors.contains("row-two"), "{anchors:?}");
        // None of the three bullet shapes is, however it is written.
        for not_an_anchor in [
            "follow-up-one-the-first-thing",
            "follow-up-one",
            "follow-up-two",
            "follow-up-three",
        ] {
            assert!(
                !anchors.contains(not_an_anchor),
                "`{not_an_anchor}` became an anchor: {anchors:?}"
            );
        }
    }

    #[test]
    fn a_row_is_addressable_by_its_first_cell() {
        let anchors = resolvable_anchors(TABLE);
        assert!(
            anchors.contains("constitution"),
            "the heading still resolves"
        );
        assert!(anchors.contains("invariant-7"), "{anchors:?}");
        assert!(anchors.contains("invariant-11"), "{anchors:?}");
        assert!(
            !anchors.contains("invariant"),
            "the header names the columns, not a fact: {anchors:?}"
        );
    }

    #[test]
    fn editing_a_sibling_row_does_not_move_this_row() {
        // The whole point. p00020 has twelve invariants under one heading and ten
        // more edits scheduled; before this, each one re-flagged every task that
        // traced the table (pre-public adoption evidence, item 14).
        let before = anchor_fingerprint(TABLE, "invariant-7").expect("row");
        let edited = TABLE.replace("| Invariant 11 | Planned |", "| Invariant 11 | Live |");
        assert_ne!(TABLE, edited, "the fixture must actually change");
        let after = anchor_fingerprint(&edited, "invariant-7").expect("row");
        assert_eq!(
            before, after,
            "invariant 7 did not change; nothing may say it did"
        );

        // And the heading still sees the whole table, which is the old behaviour
        // and still the right answer for a document that is one fact.
        let h_before = anchor_fingerprint(TABLE, "constitution").expect("heading");
        let h_after = anchor_fingerprint(&edited, "constitution").expect("heading");
        assert_ne!(h_before, h_after, "the section did change");
    }

    #[test]
    fn editing_the_traced_row_does_move_it() {
        let before = anchor_fingerprint(TABLE, "invariant-7").expect("row");
        let edited = TABLE.replace("| Invariant 7 | Live |", "| Invariant 7 | Retired |");
        let after = anchor_fingerprint(&edited, "invariant-7").expect("row");
        assert_ne!(
            before, after,
            "the traced fact changed and must be reported"
        );
    }

    #[test]
    fn inserting_a_row_above_does_not_move_the_rows_below() {
        // The reason this is addressed by content and not by position: a line
        // range or an `#anchor:7` index shifts the moment a row is inserted.
        let before = anchor_fingerprint(TABLE, "invariant-11").expect("row");
        let inserted = TABLE.replace(
            "| Invariant 7 | Live | RD0-11 |",
            "| Invariant 6 | Live | RD0-09 |\n| Invariant 7 | Live | RD0-11 |",
        );
        let after = anchor_fingerprint(&inserted, "invariant-11").expect("row");
        assert_eq!(before, after, "inserting above must not disturb it");
    }

    #[test]
    fn a_heading_wins_a_collision_so_no_existing_plan_changes_meaning() {
        let doc = "# Decision\n\nprose\n\n| Decision | x |\n|---|---|\n| a | b |\n";
        let by_anchor = anchor_fingerprint(doc, "decision").expect("resolves");
        let heading_only = "# Decision\n\nprose\n\n| Decision | x |\n|---|---|\n| a | b |\n";
        assert_eq!(
            by_anchor,
            anchor_fingerprint(heading_only, "decision").expect("resolves"),
            "the heading is what `#decision` has always meant"
        );
        assert!(
            by_anchor.len() == 16,
            "and it is the section hash, not a row hash"
        );
    }

    #[test]
    fn a_separator_row_and_an_unslugifiable_cell_are_not_anchors() {
        // A delimiter is not a row, and a cell that slugifies to nothing cannot be
        // written as an anchor, so it is not one.
        let anchors = resolvable_anchors("| :--- | ---: |\n| *** | x |\n");
        assert!(anchors.is_empty(), "{anchors:?}");
    }

    #[test]
    fn a_table_inside_a_code_fence_is_not_a_table() {
        let doc = "# Doc\n\n```\n| Invariant 7 | shown, not kept |\n```\n";
        assert!(!resolvable_anchors(doc).contains("invariant-7"));
    }
}

#[cfg(test)]
mod row_anchor_limits {
    use super::*;

    #[test]
    fn two_rows_with_the_same_first_cell_are_told_apart_by_order() {
        // The documented cost of content addressing when the content repeats:
        // `mint` disambiguates by order of appearance, so swapping two rows that
        // share a first cell swaps which one `#invariant-7` means. A register
        // with two rows keyed alike is already ambiguous to a reader; this test
        // exists so the behaviour is known rather than discovered.
        let a = "| Invariant 7 | first |\n| Invariant 7 | second |\n";
        let b = "| Invariant 7 | second |\n| Invariant 7 | first |\n";
        assert!(resolvable_anchors(a).contains("invariant-7-1"));
        assert_ne!(
            anchor_fingerprint(a, "invariant-7"),
            anchor_fingerprint(b, "invariant-7"),
            "reordering rows that share a key changes what the anchor names"
        );
    }

    #[test]
    fn a_cell_carrying_markup_slugifies_the_way_a_heading_does() {
        // Rows and headings must agree, or an author would have to learn two
        // rules. Both go through `slugify`.
        let doc = "| `APLAN024` | x |\n| **Bold** | x |\n";
        let anchors = resolvable_anchors(doc);
        assert!(anchors.contains("aplan024"), "{anchors:?}");
        assert!(anchors.contains("bold"), "{anchors:?}");
        assert_eq!(
            resolvable_anchors("## `APLAN024`\n").contains("aplan024"),
            anchors.contains("aplan024"),
            "a heading and a row with the same text mint the same anchor"
        );
    }
}

#[cfg(test)]
mod link_slugs {
    /// Appending a section must not re-flag whoever traced the one before it
    /// (`A1-122`).
    ///
    /// The section ran to end of file, so it had no trailing blank; once another
    /// heading followed it, the blank line separating them joined the text and
    /// the hash changed. Nothing in the section did. This cost a re-stamp of
    /// `A1-138` the day after it closed, on a pilot log that only grew, which is
    /// drift training a reader to dismiss drift.
    #[test]
    fn a_section_keeps_its_fingerprint_when_the_document_grows_after_it() {
        let last = "# Doc\n\n## Item\n\nThe text.\n";
        let grown = "# Doc\n\n## Item\n\nThe text.\n\n## Later\n\nAdded afterwards.\n";
        let before = super::anchor_fingerprint(last, "item").expect("resolves");
        let after = super::anchor_fingerprint(grown, "item").expect("resolves");
        assert_eq!(
            before, after,
            "appending a sibling section is not this section's drift"
        );

        // And a real edit still moves it, or the fix would be a silence.
        let edited = "# Doc\n\n## Item\n\nThe text, reworded.\n\n## Later\n\nAdded.\n";
        assert_ne!(
            super::anchor_fingerprint(edited, "item").expect("resolves"),
            after
        );
    }

    /// A stamp written before the fix still holds, so no plan is re-flagged
    /// wholesale by the upgrade (`A1-122`).
    #[test]
    fn a_stamp_from_before_the_trim_is_still_accepted() {
        let grown = "# Doc\n\n## Item\n\nThe text.\n\n## Later\n\nAdded afterwards.\n";
        // What the old code hashed: the same lines with the trailing blank left in.
        let legacy = super::fnv1a_hex("## Item\n\nThe text.\n");
        assert!(
            super::fingerprint_holds(grown, "item", &legacy),
            "an existing stamp must not become drift"
        );
        // The current form holds too, and a stamp of something else does not.
        let current = super::anchor_fingerprint(grown, "item").expect("resolves");
        assert!(super::fingerprint_holds(grown, "item", &current));
        assert!(!super::fingerprint_holds(grown, "item", "0000000000000000"));
    }

    use super::*;

    #[test]
    fn a_linked_heading_slugs_to_its_text_the_way_github_does() {
        // The defect: the URL's letters survived punctuation-stripping, so the
        // anchor was `linkedhttpecom` and a trace copied from the rendered
        // document's URL was reported dangling (`A1-097`).
        assert_eq!(slugify("[Linked](http://e.com)"), "linked");
        assert!(
            heading_anchors("## [Delaware](https://corp.delaware.gov/filing)").contains("delaware"),
            "an author copies `#delaware` from the rendered document"
        );
    }

    #[test]
    fn a_reference_link_and_an_image_slug_to_what_a_reader_sees() {
        assert_eq!(slugify("[Text][ref]"), "text");
        // An image renders as its alt text, which is what a reader has to go on.
        assert_eq!(slugify("![Alt](src.png)"), "alt");
        // A link whose target carries parentheses still closes correctly.
        assert_eq!(slugify("[Wiki](https://e.com/A_(disambiguation))"), "wiki");
    }

    #[test]
    fn emphasis_and_code_spans_are_pinned_rather_than_working_by_accident() {
        // Asterisk emphasis and code spans already worked, because `slugify` drops
        // `*` and backticks as punctuation. Pinned so a change to link handling
        // cannot quietly take them with it.
        assert_eq!(slugify("**Bold** text"), "bold-text");
        assert_eq!(slugify("`APLAN024` dilution"), "aplan024-dilution");
    }

    #[test]
    fn underscore_emphasis_is_deliberately_not_stripped() {
        // The boundary, and the reason it sits here. GitHub slugs the *rendered*
        // heading, so `_under_` is the text `under` and the anchor is `under`.
        // Matching that would mean stripping `_` as emphasis, and `_` is
        // load-bearing in the identifiers this domain is full of:
        // `require_cut_membership`, `covers_by_test`, `trace_fingerprints`. A
        // heading naming a config key is common; a heading in underscore emphasis
        // is not, and no pilot has written one.
        assert_eq!(slugify("_under_"), "_under_");
        assert_eq!(slugify("covers_by_test"), "covers_by_test");
    }

    #[test]
    fn text_that_merely_looks_like_a_link_is_left_alone() {
        // No `]` before the parenthesis, so nothing is a link target and the
        // ordinary punctuation rule applies.
        assert_eq!(slugify("Rollup (by phase)"), "rollup-by-phase");
        assert_eq!(slugify("Array[0] indexing"), "array0-indexing");
        // An unclosed target is not a target; strip nothing rather than eat the
        // rest of the heading.
        assert_eq!(slugify("[Broken](unclosed"), "brokenunclosed");
    }

    /// Proves the named behaviour `traces.table_rows` (`A1-211`).
    #[test]
    fn a_table_row_cell_follows_the_same_rule_as_a_heading() {
        // Rows and headings share `slugify`, so an author learns one rule
        // (ADR-0015).
        let doc = "| [Invariant 7](spec.md#i7) | Live |\n| x | y |\n";
        assert!(
            resolvable_anchors(doc).contains("invariant-7"),
            "{:?}",
            resolvable_anchors(doc)
        );
    }
}

/// A path with `.` and `..` resolved lexically, without touching the filesystem
/// (`A1-143`).
///
/// `Path::canonicalize` answers a different question: it needs the file to exist
/// and follows symlinks, so a trace naming a sibling that is not checked out
/// would have no answer -- and "is this target outside the repository" has to be
/// answerable from the plan alone, or the rule inherits the very dependency it
/// exists to report.
///
/// `..` at the start of a relative path is kept, since there is nothing to pop:
/// `../x` normalises to `../x` and stays outside any root it is joined to.
#[must_use]
pub fn normalise(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop a real segment; keep `..` when there is nothing to pop, so
                // an escaping path stays escaping.
                let popped = out
                    .file_name()
                    .is_some_and(|_| out.components().next_back() != Some(Component::ParentDir))
                    && out.pop();
                if !popped {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod normalise_tests {
    use super::normalise;
    use std::path::Path;

    #[test]
    fn a_path_that_escapes_stays_escaped() {
        assert_eq!(normalise(Path::new("a/../../b")), Path::new("../b"));
        assert_eq!(
            normalise(Path::new("../sibling/doc.md")),
            Path::new("../sibling/doc.md")
        );
        assert_eq!(normalise(Path::new("a/./b/../c")), Path::new("a/c"));
        assert_eq!(normalise(Path::new("docs/x.md")), Path::new("docs/x.md"));
        // Two escapes in a row are both kept: popping one would let
        // `a/../../..` look like it stays inside.
        assert_eq!(normalise(Path::new("../../x")), Path::new("../../x"));
    }
}
