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

/// Every anchor GitHub would mint for the headings in `text`.
pub fn heading_anchors(text: &str) -> BTreeSet<String> {
    headings(text).into_iter().map(|h| h.anchor).collect()
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
    let idx = hs.iter().position(|h| h.anchor == anchor)?;
    let level = hs[idx].level;
    let start = hs[idx].line;
    let lines: Vec<&str> = text.lines().collect();
    // The section ends at the next heading that is not deeper than this one.
    let end = hs[idx + 1..]
        .iter()
        .find(|h| h.level <= level)
        .map_or(lines.len(), |h| h.line);

    let normalised = lines[start..end]
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    Some(fnv1a_hex(&normalised))
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
fn slugify(title: &str) -> String {
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
