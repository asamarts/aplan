//! Reading a markdown checklist roadmap into tasks (`A1-030`).
//!
//! `- [x]` items under `##` headings is the most common planning shape there is,
//! and it is what the first pilot kept before it adopted `aplan`
//! (ADOPTION-ANALYSIS section 2.1). Importing it turns the commonest starting
//! point into a plan in one command.
//!
//! The honesty rules (ADR-0012) bite harder here than for a KEP, because a
//! checkbox carries almost nothing:
//!
//! - **A ticked box is a claim, not proof.** It imports as `todo` with a tag
//!   recording what the source said, and the report names what would close it.
//!   There is nothing in a checkbox to resolve, so `done` would be the tool
//!   asserting something it cannot check, which is the failure it exists to catch.
//! - **A checklist has no ids**, and a task without one cannot be referred to. So
//!   this importer *invents* them, which is the opposite of dropping a field, and
//!   it says so in the report rather than letting the reader assume the source
//!   supplied them.
//! - **Nesting and order are real information the model does not hold.** A nested
//!   item becomes a task of its own and the flattening is reported; sequence
//!   becomes nothing at all, because inferring `blocked_by` from position would
//!   be a guess dressed as a dependency.

use std::collections::{BTreeMap, BTreeSet};

use aplan_core::task::{Evidence, EvidenceKind, Status, Task};

use crate::export::{Export, ExportError, ExportOptions, PlanTarget};
use crate::report::{Notice, NoticeKind, Report};
use crate::{ImplementedAs, Import, ImportError, Options, ProposalSource};

/// The markdown checklist flavor. Read by [`ProposalSource`], written by
/// [`PlanTarget`].
#[derive(Debug)]
pub struct Checklist;

/// The tag recording that the source's box was ticked.
///
/// One spelling, read by both halves: the importer writes it and the exporter
/// reads it back to tell a claim it could not prove from a task the plan really
/// closed. Two copies of a tag name is exactly the drift this tool exists to
/// report.
pub const CHECKED_TAG: &str = "checklist:checked";

/// One `- [ ]` / `- [x]` item, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    /// Item text, with wrapped continuation lines joined.
    text: String,
    /// Whether the box was ticked.
    checked: bool,
    /// Heading the item sits under, verbatim.
    section: Option<String>,
    /// Indent depth: 0 for a top-level item, more for a nested one.
    depth: usize,
    /// 1-based line the item started on, so a notice can point at it.
    line: usize,
}

/// A heading and the prose under it, before the first item.
#[derive(Debug, Default, Clone)]
struct Section {
    title: String,
    prose: String,
}

/// Split a checklist document into sections and items.
///
/// Deliberately line-based rather than a markdown parser: the shape being read is
/// a convention (`- [ ]` under `##`), not a language, and a full parser would
/// bring an opinion about every other construct in a file it should be passing
/// through untouched.
fn scan(contents: &str) -> (Vec<Item>, BTreeMap<String, Section>, usize) {
    let mut items: Vec<Item> = Vec::new();
    let mut sections: BTreeMap<String, Section> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut seen_item_in_section = false;
    // Whether the line above could still be continued. A wrapped item ends at a
    // blank line or a heading, the way it does for a reader: without this, an
    // indented paragraph *after* an item was glued onto its title, and so was
    // indented prose belonging to the next section entirely.
    let mut continuing = false;
    // Inside a ``` or ~~~ fence, nothing is content. A roadmap that documents
    // its own syntax in a fenced block was otherwise importing the example as a
    // task, and a `##` line inside the fence became a section that real tasks
    // below it were then filed under.
    let mut fence: Option<String> = None;
    let mut skipped_empty = 0usize;

    for (i, raw) in contents.lines().enumerate() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();

        if let Some(open) = &fence {
            if trimmed.starts_with(open.as_str()) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_marker(trimmed) {
            fence = Some(marker);
            continuing = false;
            continue;
        }

        if trimmed.is_empty() {
            continuing = false;
            continue;
        }

        if let Some(rest) = heading(trimmed) {
            current = Some(rest.to_string());
            seen_item_in_section = false;
            continuing = false;
            sections.entry(rest.to_string()).or_insert(Section {
                title: rest.to_string(),
                prose: String::new(),
            });
            continue;
        }

        if let Some((checked, text)) = checkbox(trimmed) {
            seen_item_in_section = true;
            if text.is_empty() {
                // `- [ ]` with nothing after it. Skipped, because a task with no
                // title is one nobody can find (and the schema refuses it), but
                // counted, because dropping a line in silence is what an
                // importer here is forbidden to do.
                continuing = false;
                skipped_empty += 1;
            } else {
                let indent = line.len() - trimmed.len();
                continuing = true;
                items.push(Item {
                    text: text.to_string(),
                    checked,
                    section: current.clone(),
                    // Two spaces per level is the markdown convention; four is
                    // the other one. Either way, any indent means nested.
                    depth: indent / 2,
                    line: i + 1,
                });
            }
            continue;
        }

        // A wrapped continuation of the item above: indented, not blank, and not
        // separated from it by a blank line or a heading. p0000004's roadmap
        // wraps nearly every item, so dropping these would truncate most titles
        // mid-sentence.
        if continuing
            && line.starts_with(char::is_whitespace)
            && let Some(last) = items.last_mut()
        {
            last.text.push(' ');
            last.text.push_str(trimmed);
            continue;
        }
        continuing = false;

        // Prose under a heading, before its first item.
        if !seen_item_in_section
            && let Some(name) = &current
            && let Some(section) = sections.get_mut(name)
        {
            if !section.prose.is_empty() {
                section.prose.push(' ');
            }
            section.prose.push_str(trimmed);
        }
    }

    (items, sections, skipped_empty)
}

/// The opening marker of a fenced code block, if this line is one.
///
/// The marker is returned rather than reduced to a bool so the *closing* fence
/// has to be the same character and at least as long. A four-backtick block
/// exists precisely to contain three-backtick ones, and a bool would let the
/// inner fence close the outer block and hand the rest of the example back as
/// content.
fn fence_marker(line: &str) -> Option<String> {
    for c in ['`', '~'] {
        let run: String = line.chars().take_while(|x| *x == c).collect();
        if run.len() >= 3 {
            return Some(run);
        }
    }
    None
}

/// `## Title` or `### Title` (but not `#`, which is the document title).
fn heading(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("##")?;
    let rest = rest.trim_start_matches('#');
    let title = rest.trim();
    (!title.is_empty()).then_some(title)
}

/// `- [ ] text` / `* [x] text`, returning (checked, text).
///
/// The text may be empty: `- [ ]` on its own *is* a checkbox line, and the
/// caller counts it rather than passing over it, because an importer that drops
/// a line without saying so is the thing ADR-0012 clause 3 forbids.
fn checkbox(line: &str) -> Option<(bool, &str)> {
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))?;
    let rest = rest.trim_start();
    let (mark, rest) = match rest.strip_prefix("[ ]") {
        Some(r) => (false, r),
        None => (
            true,
            rest.strip_prefix("[x]")
                .or_else(|| rest.strip_prefix("[X]"))?,
        ),
    };
    Some((mark, rest.trim()))
}

/// A heading rendered as a tag: lowercase, non-alphanumerics collapsed to `-`.
///
/// Non-ASCII is dropped rather than transliterated. A tag is matched literally by
/// `.aplan.yml` and by `APLAN022`'s `cross-cutting`, so it has to be typeable;
/// the heading itself survives verbatim in the task body either way.
/// A section heading as a tag.
///
/// Alphanumeric in the Unicode sense, not the ASCII one. The ASCII test deleted
/// every accented letter and every non-Latin script: `Cafe naive resume` written
/// with its accents became `caf-na-ve-r-sum`, and a heading in a non-Latin script
/// produced nothing at all (pre-public adoption evidence, item 4).
///
/// Preserved rather than transliterated. Transliteration needs a table, and a
/// table is a guess about a language nobody here reads; it also loses
/// information that the tag is meant to carry. Nothing else in this tool imposes
/// an ASCII restriction: `check` accepts a unicode title, `add` writes one back
/// unchanged, and the task schema says a tag is a string. This was the one place
/// that disagreed with its own crate (`A1-051`).
///
/// A plain ASCII heading slugifies exactly as it did before, because the Unicode
/// test is a superset of the ASCII one.
/// Combining marks that belong to the letter before them.
///
/// The same text can be written two ways: `Caf\u{e9}` with a precomposed letter,
/// or `Cafe` followed by U+0301, which is what many editors and macOS produce.
/// A combining mark is not alphanumeric, so without this the second form still
/// lost its accent -- the very defect this rule was changed to fix, surviving in
/// a different encoding (`A1-051`, audit).
///
/// These are the combining-diacritic blocks proper, which cover Latin, Greek and
/// Cyrillic. **Script-specific marks live in their own blocks and are not here**:
/// a heading in Devanagari or Arabic still loses its marks, and answering that
/// properly needs Unicode category data, which means a dependency or a hand-rolled
/// table. A table is the guess this rule already refused for transliteration, so
/// the limit is stated rather than approximated.
const COMBINING: &[std::ops::RangeInclusive<char>] = &[
    '\u{0300}'..='\u{036f}',
    '\u{1ab0}'..='\u{1aff}',
    '\u{1dc0}'..='\u{1dff}',
    '\u{20d0}'..='\u{20f0}',
    '\u{fe20}'..='\u{fe2f}',
];

fn is_combining(c: char) -> bool {
    COMBINING.iter().any(|r| r.contains(&c))
}

pub(crate) fn tag_for(section: &str) -> String {
    let mut out = String::new();
    let mut kept_letter = false;
    for c in section.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            kept_letter = true;
        } else if is_combining(c) && kept_letter {
            // Only after a letter that was kept: a mark with nothing to attach to
            // is not a letter, it is a stray.
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
            kept_letter = false;
        } else {
            kept_letter = false;
        }
    }
    out.trim_matches('-').to_string()
}

/// Abbreviations whose full stop does not end a sentence.
///
/// Deliberately short. A long list is a guess about prose nobody has read; these
/// are the ones that actually occur in the plans this importer has been pointed
/// at, and anything missing costs a title that is shorter than it should be
/// rather than one that is wrong.
const ABBREVIATIONS: &[&str] = &[
    "e.g.", "i.e.", "etc.", "cf.", "vs.", "approx.", "no.", "fig.",
];

/// Closing markup that may sit between a full stop and the space after it.
///
/// `exposed.**` is how a bold sentence ends, and a rule that demanded whitespace
/// immediately after the stop would miss every one of them. Measured on
/// `p00025`'s own plan, where the longest title ends its first sentence exactly
/// that way (`A1-055`).
const CLOSERS: &[char] = &['*', '`', ')', ']', '"', '\'', '_'];

/// Separators that introduce a description after a lead-in.
///
/// `p00025` writes `**`idl` layer (delegated)** \u{2014} `cargo xtask ...`: TypeSpec
/// ...`, where the lead-in is the title a person would write and everything after
/// the dash is the body. Kept to the three that occur: an em dash, an en dash,
/// and a double hyphen, each surrounded by spaces, plus a colon followed by one.
/// A bare hyphen is not here, because hyphenated words would split on it.
const SEPARATORS: &[&str] = &[" \u{2014} ", " \u{2013} ", " -- ", ": "];

/// A title shorter than this many *characters* is not a title, it is a fragment.
///
/// Characters rather than bytes: a byte floor would demand three times as much
/// text of a plan written in a language whose characters are three bytes wide.
///
/// Measured on the file this was built against: without a floor, one item cut to
/// `(Superseded by S6`, which says less than the whole line did. Whatever the
/// rule proposes below this length, the next candidate is used instead
/// (`A1-055`).
const MIN_TITLE: usize = 20;

/// The title for a checklist item: the shortest honest prefix of what was
/// written, or `None` when the whole item already is one.
///
/// Two rules, and the *shorter* result wins rather than the first to match. That
/// is not arbitrary: `p00025`'s longest item has a 213-character first sentence
/// and a 38-character lead-in, so preferring the sentence would keep a title
/// nobody can read while a better one sat in the same string.
///
/// Never invents: every result is a prefix of the item, so the split is
/// reversible by reading the body, which keeps the item verbatim.
/// Where a title stops being a title and becomes a paragraph.
///
/// Only used to *count*, never to reject: an import that refused long items would
/// be refusing the document rather than describing it (`A1-129`).
const LONG_TITLE: usize = 120;

pub(crate) fn title_for(text: &str) -> Option<&str> {
    let candidates = [first_sentence(text), lead_in(text)];
    // No floor here: it belongs to `lead_in`, which produces fragments. A first
    // sentence is a complete thought, so a short one is a good title, and a floor
    // that rejected it would keep a whole paragraph instead. Found by a surviving
    // mutation, which is how a redundant guard announces itself (`A1-055`).
    candidates
        .into_iter()
        .flatten()
        .filter(|c| c.len() < text.trim().len() && emphasis_is_balanced(c))
        .min_by_key(|c| c.len())
}

/// Whether a candidate title closes every emphasis marker it opens.
///
/// The invariant belongs on the *result*, not on the scan: the item that started
/// this task ends its first sentence at `exposed.**`, where the stop sits inside
/// bold and the closing marker is taken with it, so a scanner that skipped stops
/// inside emphasis would reject the very case it was built for. What must never
/// happen is a title like `**Ship it.`, which renders as literal asterisks or
/// swallows whatever follows it.
///
/// `lead_in` tracks emphasis as well, and that is not redundant: tracking lets it
/// try the *next* separator, where this filter can only reject (`A1-055`).
fn emphasis_is_balanced(title: &str) -> bool {
    let mut in_code = false;
    let (mut strong, mut em) = (0_usize, 0_usize);
    let mut chars = title.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            in_code = !in_code;
            continue;
        }
        // Asterisks inside a code span are globs (`ci/scripts/*`), not emphasis.
        if in_code || ch != '*' {
            continue;
        }
        if chars.peek() == Some(&'*') {
            chars.next();
            strong += 1;
        } else {
            em += 1;
        }
    }
    !in_code && strong % 2 == 0 && em % 2 == 0
}

/// The text before the first top-level separator.
fn lead_in(text: &str) -> Option<&str> {
    let mut in_code = false;
    let mut in_strong = false;
    let mut in_em = false;
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '`' {
            in_code = !in_code;
            continue;
        }
        // Emphasis is tracked for the same reason code spans are: a cut inside
        // one leaves the marker unclosed, and `**Close the prompt-model gap`
        // renders as literal asterisks or swallows whatever follows. Measured on
        // a constructed case while auditing, because the real file's emphasis
        // happened to be short enough that the floor hid it (`A1-055`).
        if !in_code && ch == '*' {
            if chars.peek().is_some_and(|(_, c)| *c == '*') {
                chars.next();
                in_strong = !in_strong;
            } else {
                in_em = !in_em;
            }
            continue;
        }
        if in_code || in_strong || in_em {
            continue;
        }
        for sep in SEPARATORS {
            if text[i..].starts_with(sep) {
                let head = text[..i].trim_end_matches([' ', ':']).trim();
                // A separator with nothing after it is punctuation, not a lead-in.
                // And a head below the floor is a fragment, so the *next*
                // separator is tried rather than giving up: one item opens
                // `(Superseded by S6 \u{2014} originally ...)`, where the first
                // dash cuts at seventeen characters and the second gives a title
                // that says something (`A1-055`).
                if head.chars().count() >= MIN_TITLE && !text[i + sep.len()..].trim().is_empty() {
                    return Some(head);
                }
            }
        }
    }
    None
}

/// The item's first sentence, when the item is longer than one.
///
/// `None` when the whole item is a single sentence, so a short item keeps its
/// text as its title and gains no duplicated body. A title is never invented:
/// this only ever returns a prefix of what was written.
fn first_sentence(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut in_code = false;
    for (i, ch) in text.char_indices() {
        if ch == '`' {
            in_code = !in_code;
            continue;
        }
        // A stop inside a code span is punctuation in a path, a version, or a
        // command, never the end of a sentence.
        if in_code || !matches!(ch, '.' | '!' | '?') {
            continue;
        }
        // `v0.2.0` and `spec.sh`: a stop between two non-spaces is not a stop.
        let mut end = i + ch.len_utf8();
        while text[end..].starts_with(CLOSERS) {
            end += text[end..].chars().next().map_or(0, char::len_utf8);
        }
        let rest = &text[end..];
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let head = &text[..end];
        if ABBREVIATIONS
            .iter()
            .any(|a| head.to_ascii_lowercase().ends_with(a))
        {
            continue;
        }
        // A stop at the very end means the whole item is one sentence.
        if rest.trim().is_empty() {
            return None;
        }
        let _ = bytes;
        return Some(head.trim());
    }
    None
}

/// The body a checklist task starts with: where it came from, and the prose that
/// sat above it, which would otherwise be the one thing lost by importing a list
/// of one-liners.
fn body_for(source_name: &str, item: &Item, section: Option<&Section>) -> String {
    use std::fmt::Write as _;

    let mut body = format!(
        "## Why\n\nImported from `{source_name}` line {} by `aplan import --format checklist`.\n",
        item.line
    );
    // When the title is the item's first sentence, the item itself has to survive
    // somewhere or the import has silently dropped what it read. Only when it was
    // shortened: a one-sentence item would otherwise carry its own title twice
    // (`A1-055`).
    if title_for(&item.text).is_some() {
        let _ = write!(body, "\nThe checklist item, in full:\n\n> {}\n", item.text);
    }
    if let Some(section) = section {
        let _ = write!(body, "\nFrom the section \"{}\".\n", section.title);
        if !section.prose.is_empty() {
            let _ = write!(body, "\n> {}\n", section.prose);
        }
    }
    // The constant, not a copy of its text: a rule reads it to refuse the
    // prompt back as an answer, and a copy here would drift out of that the
    // day somebody reworded one of them (`A1-271`).
    let _ = write!(
        body,
        "\n## Acceptance criteria\n\n- [ ] {}\n",
        aplan_core::scaffold::criterion_prompt()
    );
    body
}

/// What a ticked box becomes, and what the reader is told about it.
pub(crate) fn resolve_checked(
    id: &str,
    source_name: &str,
    options: &Options,
    report: &mut Report,
) -> (Status, Vec<Evidence>) {
    match &options.implemented_as {
        ImplementedAs::Done { as_of } => {
            report.push(Notice::new(
                source_name,
                NoticeKind::Provenance,
                format!(
                    "{id} imported `done` from a ticked box, which is note-level provenance \
                     only; attach real proof with `aplan close {id} --evidence commit:COMMIT_SHA`"
                ),
            ));
            (
                Status::Done,
                vec![Evidence {
                    kind: EvidenceKind::Note,
                    reference: format!("imported from {source_name}: the checklist box was ticked"),
                    captured: Some(as_of.clone()),
                }],
            )
        }
        ImplementedAs::Todo => {
            report.push(Notice::new(
                source_name,
                NoticeKind::Provenance,
                format!(
                    "{id} was ticked in the source; imported as `todo` (a checkbox is a claim, \
                     not proof). Close it with `aplan close {id} --evidence commit:COMMIT_SHA`"
                ),
            ));
            (Status::Todo, Vec::new())
        }
    }
}

/// Name what the conversion could not carry (ADR-0012 clause 3).
fn report_losses(items: &[Item], source_name: &str, report: &mut Report) {
    // A heading with nothing alphanumeric in it cannot become a tag, so the
    // section grouping is lost for those items. `import` is lossy by design and
    // *says so*, and this was the one loss it made in silence: the pilot's run
    // reported provenance, placement and effort, and said nothing about the
    // characters it had discarded (pre-public adoption evidence, item 4).
    let mut untaggable: Vec<&str> = items
        .iter()
        .filter_map(|i| i.section.as_deref())
        .filter(|h| tag_for(h).is_empty())
        .collect();
    untaggable.sort_unstable();
    untaggable.dedup();
    if !untaggable.is_empty() {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            format!(
                "{} section heading(s) have nothing a tag can be made from, so their items \
                 carry no section tag: {}",
                untaggable.len(),
                untaggable.join(", ")
            ),
        ));
    }

    if items.iter().any(|i| i.depth > 0) {
        let nested = items.iter().filter(|i| i.depth > 0).count();
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            format!(
                "{nested} nested item(s) became tasks of their own: the model has no subtasks, \
                 so the nesting itself is not represented"
            ),
        ));
    }
    // An importer that rewrites what it read has to say so, which is ADR-0012's
    // whole reasoning: lossy by design, and it says where (`A1-055`).
    let shortened = items
        .iter()
        .filter(|i| title_for(&i.text).is_some())
        .count();
    if shortened > 0 {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            format!(
                "{shortened} title(s) were shortened to the item's first sentence or lead-in; \
                 each item is preserved in full in its task body"
            ),
        ));
    }
    // The other half of that number, and the one an import is judged on at
    // programme scale: an item with no shorter candidate becomes its own title,
    // and a long one is a paragraph in a list nobody can read. Counted so a large
    // import can be judged before it is committed rather than task by task
    // afterwards (`A1-129`).
    let verbatim = items.len() - shortened;
    let wants_rewriting = items
        .iter()
        .filter(|i| title_for(&i.text).is_none() && i.text.trim().chars().count() > LONG_TITLE)
        .count();
    if verbatim > 0 {
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            // The rewrite clause only when there is one to do. An import whose
            // titles are all short said "0 of them longer than 120 characters"
            // on every run, which is a line about nothing in a report whose job
            // is to say what happened.
            match wants_rewriting {
                0 => format!("{verbatim} title(s) are the item as written"),
                n => format!(
                    "{verbatim} title(s) are the item as written, {n} of them longer than \
                     {LONG_TITLE} characters and wanting a rewrite before anyone reads a list \
                     of them"
                ),
            },
        ));
    }
    if items.len() > 1 {
        report.push(Notice::new(
            source_name,
            NoticeKind::DroppedField,
            "list order is not a dependency: nothing was written to `blocked_by`, because \
             inferring one from position would be a guess recorded as a fact"
                .to_string(),
        ));
    }
}

/// What a document contains, when it contains no checklist item.
///
/// The refusal is right and its silence was not: the fifth pilot passed a
/// 1,887-line programme plan and got three words and an exit code, which said
/// nothing about the 451 rows it declined or about what would work instead
/// (`A1-129`, pre-public adoption evidence). This importer reads one shape and reads it
/// unambiguously, so it must say which shape it read and which it wanted.
fn declined(contents: &str) -> String {
    // Outside fenced blocks, exactly as `scan` reads it. A document that shows a
    // table in an example has no tables, and counting them would be this function
    // disagreeing with the scanner it is explaining: the same mistake the scanner
    // already learned not to make, where a fenced example was imported as a task.
    let lines: Vec<&str> = content_lines(contents);
    let table_lines = lines
        .iter()
        .filter(|l| l.trim_start().starts_with('|'))
        .count();
    // A separator row is what makes a run of pipes a table, so counting them
    // counts tables.
    let tables = lines.iter().filter(|l| is_table_separator(l)).count();
    let data_rows = table_lines.saturating_sub(tables * 2);
    let headings = lines
        .iter()
        .filter(|l| l.trim_start().starts_with('#'))
        .count();

    let mut saw = Vec::new();
    if tables > 0 {
        saw.push(format!("{tables} table(s) with {data_rows} data row(s)"));
    }
    if headings > 0 {
        saw.push(format!("{headings} heading(s)"));
    }
    let saw = if saw.is_empty() {
        "no tables and no headings".to_string()
    } else {
        saw.join(" and ")
    };
    // The table clause only where there are tables. A prose document told that "a
    // table row has no single meaning" is being answered about something it does
    // not contain, which is the wrong-noun message this project files tasks about.
    let why = if tables > 0 {
        " A table row has no single meaning the way a checkbox does, so nothing here guesses \
         which of them are work."
    } else {
        ""
    };
    format!(
        ": this reads `- [ ]` and `- [x]` rows, and the document has none. It has {saw}.{why} \
         List the items you mean as `- [ ]` rows and import that \
         (`docs/importing-proposals.md`, under \"A table is not a checklist\")"
    )
}

/// The document's lines with fenced blocks removed.
///
/// Split out because two readers of one document must agree about what is in it:
/// `scan` skips fences so a roadmap documenting its own syntax is not imported as
/// a task, and [`declined`] describes what `scan` found, so it has to skip the
/// same lines or it reports tables that are examples (`A1-129`).
fn content_lines(contents: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut fence: Option<String> = None;
    for raw in contents.lines() {
        let trimmed = raw.trim_start();
        if let Some(open) = &fence {
            if trimmed.starts_with(open.as_str()) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_marker(trimmed) {
            fence = Some(marker);
            continue;
        }
        out.push(raw);
    }
    out
}

/// Whether a line is a Markdown table's header separator (`| --- | --- |`).
fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|')
        && t.len() > 1
        && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
        && t.contains('-')
}

impl ProposalSource for Checklist {
    fn flavor(&self) -> &'static str {
        "checklist"
    }

    fn parse(
        &self,
        source_name: &str,
        contents: &str,
        options: &Options,
    ) -> Result<Import, ImportError> {
        if contents.trim().is_empty() {
            return Err(ImportError::Empty {
                source_name: source_name.to_string(),
                detail: Some(": the file is empty".to_string()),
            });
        }
        let (items, sections, skipped_empty) = scan(contents);
        if items.is_empty() {
            return Err(ImportError::Empty {
                source_name: source_name.to_string(),
                detail: Some(declined(contents)),
            });
        }

        let mut report = Report::new();
        let prefix = options.id_prefix.as_str();
        report.push(Notice::new(
            source_name,
            NoticeKind::Provenance,
            format!(
                "a checklist has no task ids, so {} were generated as `{prefix}-001` onward; \
                 rename them before they are referred to elsewhere, or re-run with \
                 `--id-prefix`",
                items.len()
            ),
        ));
        report_losses(&items, source_name, &mut report);
        if skipped_empty > 0 {
            report.push(Notice::new(
                source_name,
                NoticeKind::DroppedField,
                format!(
                    "{skipped_empty} checkbox line(s) had no text and became no task: a task with no \
                     title is one nobody can find"
                ),
            ));
        }

        let mut tasks = Vec::with_capacity(items.len());
        for (n, item) in items.iter().enumerate() {
            let id = format!("{prefix}-{:03}", n + 1);
            let (status, evidence) = if item.checked {
                resolve_checked(&id, source_name, options, &mut report)
            } else {
                (Status::Todo, Vec::new())
            };
            let mut tags: Vec<String> = Vec::new();
            if let Some(section) = &item.section {
                let tag = tag_for(section);
                if !tag.is_empty() {
                    tags.push(tag);
                }
            }
            if item.checked {
                tags.push(CHECKED_TAG.to_string());
            }
            tasks.push(Task {
                id,
                title: title_for(&item.text).unwrap_or(&item.text).to_string(),
                phase: options.phase,
                status,
                owner: None,
                effort_days: None,
                blocked_by: Vec::new(),
                prefer_after: Vec::new(),
                ratified_by: Vec::new(),
                contains: Vec::new(),
                evidence,
                paths: Vec::new(),
                traces: Vec::new(),
                trace_fingerprints: BTreeMap::new(),
                evidence_fingerprints: std::collections::BTreeMap::new(),
                tags,
                // An importer never invents one: a revisit is a decision about
                // work somebody parked, and no source format carries it.
                revisit_at: None,
                version_impact: None,
                version_impact_reason: None,
                version: None,
            });
        }

        let bodies = items
            .iter()
            .map(|i| {
                body_for(
                    source_name,
                    i,
                    i.section.as_ref().and_then(|s| sections.get(s)),
                )
            })
            .collect();

        Ok(Import {
            tasks,
            report,
            bodies,
        })
    }
}

// -- The other half: writing a plan back out as a checklist (`A1-189`). --------

/// One field the target shape has no place for: its name, whether a task carries
/// it, and what its absence costs the reader.
///
/// A named type rather than a bare tuple so the table reads as a table.
struct NotCarried(&'static str, fn(&Task) -> bool, &'static str);

/// The task fields a checklist row has no place for, and what each costs the
/// reader of the generated file.
///
/// A table rather than nine hand-written notices, for the reason the importer's
/// `report_losses` is one function: the honesty rule is the same for every field,
/// and a field added to the model with no row here is a silent omission, which is
/// what ADR-0012 clause 3 forbids. `every_task_field_is_carried_or_named` walks
/// the model against it.
const NOT_CARRIED: &[NotCarried] = &[
    NotCarried(
        "evidence",
        |t| !t.evidence.is_empty(),
        "the proof a closed task cites; a ticked box asserts the claim with nothing behind it",
    ),
    NotCarried(
        "paths",
        |t| !t.paths.is_empty(),
        "what a commit has to touch for it to be about this task",
    ),
    NotCarried(
        "traces",
        |t| !t.traces.is_empty(),
        "the documents the task answers to",
    ),
    NotCarried("owner", |t| t.owner.is_some(), "who holds it"),
    NotCarried(
        "effort_days",
        |t| t.effort_days.is_some(),
        "the estimate the order is derived from",
    ),
    NotCarried(
        "prefer_after",
        |t| !t.prefer_after.is_empty(),
        "the discretionary order, which is not a dependency and has nowhere to go in a list",
    ),
    NotCarried(
        "ratified_by",
        |t| !t.ratified_by.is_empty(),
        "who has to bless this before it counts as finished, which a tick cannot record",
    ),
    NotCarried(
        "tags",
        |t| !t.tags.is_empty(),
        "labels a heading cannot hold, because a heading here is a cut",
    ),
    NotCarried(
        "trace_fingerprints",
        |t| !t.trace_fingerprints.is_empty(),
        "the recorded shape of each traced anchor, which is what makes a moved heading a finding",
    ),
    NotCarried(
        "evidence_fingerprints",
        |t| !t.evidence_fingerprints.is_empty(),
        "the recorded shape of mutable file proof, which is what makes changed evidence a finding",
    ),
    NotCarried(
        "revisit_at",
        |t| t.revisit_at.is_some(),
        "the instant a parked task is to be looked at again",
    ),
    NotCarried(
        "version_impact",
        |t| t.version_impact.is_some(),
        "what the task does to the released version, which a tick cannot say",
    ),
    NotCarried(
        "version_impact_reason",
        |t| t.version_impact_reason.is_some(),
        "why that claim is lower than the task's level implies, which is the half a reviewer reads",
    ),
];

/// The rows of the document, grouped the way the plan groups work.
#[derive(Debug, Default)]
struct Grouped<'a> {
    /// Tasks in no cut, in id order. Written before the first heading, because a
    /// heading here is a cut and inventing one would assert a grouping the plan
    /// does not have.
    loose: Vec<&'a Task>,
    /// `(cut, its members in id order)`, cuts in id order, only where a cut has
    /// a row to write.
    cuts: Vec<(&'a Task, Vec<&'a Task>)>,
    /// Ids listed by more than one cut, so the placement notice can count them.
    in_two_cuts: Vec<&'a str>,
    /// Milestones with no row to write, which produce no heading.
    empty_cuts: usize,
    /// Milestones declaring `contains`, counted over the whole plan rather than
    /// over `cuts` (`A1-333`): a container's rows live under the checkpoints
    /// *beneath* it, so it frequently has none of its own and never appears
    /// there. Counting it there reported nothing and looked like coverage.
    containers: usize,
    /// Releases recording the version they produced. Counted here for the reason
    /// `containers` is: only a release may carry `version` (`APLAN050`), this
    /// exporter writes no milestone as a row, and a `NOT_CARRIED` entry over the
    /// rows could therefore never fire -- which is a claim of coverage rather
    /// than coverage (`A1-333`, `A1-339`).
    recorded_versions: usize,
    /// Cancelled cuts that still hold live work, whose members were written
    /// loose instead of under a heading that reads as a live section.
    cancelled_cuts: usize,
    /// Non-milestone tasks left out because they are `dropped`.
    dropped: usize,
}

/// Split the plan into loose work and cuts, using the one membership rule.
///
/// Membership is read through [`aplan_core::edge_kind`] rather than re-derived
/// here, so the exporter and `graph` cannot disagree about what a cut holds
/// (ADR-0030).
fn group<'a>(tasks: &[&'a Task]) -> Grouped<'a> {
    let by_id: BTreeMap<&'a str, &'a Task> = tasks.iter().map(|t| (t.id.as_str(), *t)).collect();
    // A `dropped` task is left out rather than written unticked: an empty box
    // reads as outstanding work, and abandoned work is not outstanding. Counted,
    // because leaving it out in silence is the omission this crate forbids.
    //
    // Deliberately *only* the status. Whether a task is work at all is
    // `edge_kind`'s answer inside a cut and `is_milestone`'s outside one, and
    // folding both questions into this closure made the membership rule below
    // redundant with it: deleting that rule changed no output and broke no test,
    // which is a guard that is not guarding (`A1-189`, audit).
    let live = |t: &Task| t.status != Status::Dropped;

    let mut out = Grouped {
        dropped: by_id
            .values()
            .filter(|t| !t.is_milestone() && t.status == Status::Dropped)
            .count(),
        containers: by_id.values().filter(|t| !t.contains.is_empty()).count(),
        recorded_versions: by_id.values().filter(|t| t.version.is_some()).count(),
        ..Grouped::default()
    };
    // Which cut wrote each id, so a task named by two cuts appears once. First in
    // id order wins, which is a rule rather than a coin toss, and the reader is
    // told it happened.
    let mut placed: BTreeMap<&'a str, &'a str> = BTreeMap::new();

    // A cancelled cut is not a heading. The same reasoning the dropped rows above
    // stand on, applied to structure: `## A cancelled cut` over live rows reads
    // as a live section, and a document a person opens has no room to explain
    // that the plan says otherwise. Its members fall loose and the reader is
    // told. `APLAN027` is what tells the *author* the plan disagrees with itself;
    // this only refuses to amplify it.
    for cut in by_id
        .values()
        .filter(|t| t.is_milestone() && t.status != Status::Dropped)
    {
        let mut members: Vec<&'a Task> = Vec::new();
        for named in cut.blocked_by.iter().map(String::as_str) {
            let Some(task) = by_id.get(named) else {
                continue; // an id this plan does not have, which is `APLAN003`
            };
            // **The one membership rule, asked rather than restated.** A cut
            // listing a cut is sequencing and a release listing its cuts is a
            // checkpoint, and neither is work to write a row for. Deriving that
            // here from `is_milestone` would be a second answer to the question
            // `graph` already asks this way, and the two would drift.
            if aplan_core::edge_kind(task, cut) != aplan_core::EdgeKind::Membership {
                continue;
            }
            if !live(task) {
                continue; // dropped, and counted once above
            }
            match placed.get(named) {
                // Two different cuts naming one task. Rare and legal, and
                // writing it twice would be a document that counts its own work
                // wrong.
                Some(first) if *first != cut.id.as_str() => out.in_two_cuts.push(named),
                // The *same* cut naming it twice is a botched edit, not a
                // placement question, and reporting it as two cuts would send
                // the reader looking for a second cut that does not exist.
                Some(_) => {}
                None => {
                    placed.insert(named, cut.id.as_str());
                    members.push(task);
                }
            }
        }
        members.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        if members.is_empty() {
            out.empty_cuts += 1;
        } else {
            out.cuts.push((cut, members));
        }
    }

    // Counted *after* every live cut has placed what it holds, not while they
    // are being walked: a task two cuts name, one of them cancelled, is written
    // under the live one, and counting it during the walk claimed it was above
    // the first heading depending on which cut sorted first.
    out.cancelled_cuts = by_id
        .values()
        .filter(|t| t.is_milestone() && t.status == Status::Dropped)
        .filter(|cut| {
            cut.blocked_by.iter().any(|b| {
                by_id.get(b.as_str()).is_some_and(|t| {
                    aplan_core::edge_kind(t, cut) == aplan_core::EdgeKind::Membership
                        && live(t)
                        && !placed.contains_key(b.as_str())
                })
            })
        })
        .count();

    out.loose = by_id
        .values()
        .filter(|t| !t.is_milestone() && live(t))
        .filter(|t| !placed.contains_key(t.id.as_str()))
        .copied()
        .collect();
    out.in_two_cuts.sort_unstable();
    out.in_two_cuts.dedup();
    out
}

/// One row: the box the plan's status implies, and the title verbatim.
///
/// `- [x]` is `done` and nothing else. A checkbox has two states and the model
/// has five, and mapping `in_progress` onto the ticked one would be this tool
/// asserting a completion it refused to take on trust in the other direction.
fn row(task: &Task) -> String {
    let box_ = if task.status == Status::Done {
        "[x]"
    } else {
        "[ ]"
    };
    format!("- {box_} {}\n", one_row(&task.title))
}

/// Plan-supplied text, made safe to be one line of a document somebody opens.
///
/// **Two jobs, and every caller wants both**, which is the reasoning
/// `aplan-output`'s own `one_line` already carries for the generated roadmap
/// table. A control character steers the terminal of whoever `cat`s this file,
/// and whitespace that would break the row makes a second line the reader beside
/// this parses as prose belonging to the row above.
///
/// A generated file is a *worse* destination than a terminal rendering, not a
/// safer one: the bytes are committed, and the next person to meet them never
/// ran this tool. Writing them out unfiltered gave two different documents for
/// one plan, because the stream writer strips an escape and its parameters while
/// a file keeps them byte for byte (`A1-189`, audit).
fn one_row(text: &str) -> String {
    aplan_output::sanitize::visible(text)
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render the grouped plan, deterministically.
fn emit(grouped: &Grouped) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    for task in &grouped.loose {
        out.push_str(&row(task));
    }
    for (cut, members) in &grouped.cuts {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(out, "## {}\n\n", one_row(&cut.title));
        for task in members {
            out.push_str(&row(task));
        }
    }
    out
}

/// Name what the shape could not carry, which is the import policy read
/// backwards (ADR-0012 clause 3).
fn report_export_losses(grouped: &Grouped, to: &str, report: &mut Report) {
    let written: Vec<&Task> = grouped
        .loose
        .iter()
        .chain(grouped.cuts.iter().flat_map(|(_, m)| m.iter()))
        .copied()
        .collect();

    report.not_carried(
        to,
        "id",
        &format!(
            "{} row(s) are written without one, because a checklist row has no id field; \
             importing this file back generates fresh ids",
            written.len()
        ),
    );
    let unticked_but_live = written
        .iter()
        .filter(|t| matches!(t.status, Status::InProgress | Status::Blocked))
        .count();
    if unticked_but_live > 0 {
        report.not_carried(
            to,
            "status",
            &format!(
                "{unticked_but_live} task(s) that are in progress or blocked are written as an \
                 empty box, because a checkbox has two states and the plan has five"
            ),
        );
    }
    // `contains` is a milestone's field, and this exporter writes no milestone as
    // a row -- a heading here *is* a cut. So it is counted over the cuts rather
    // than over `written`, where the `NOT_CARRIED` table would never have seen
    // it: a table entry that cannot fire is a claim of coverage, not coverage
    // (`A1-333`).
    if grouped.containers > 0 {
        report.not_carried(
            to,
            "contains",
            &format!(
                "{} cut(s) contain another: a checklist has one level of heading, \
                 so the checkpoints beneath them are flattened and the nesting is lost",
                grouped.containers
            ),
        );
    }
    if grouped.recorded_versions > 0 {
        report.not_carried(
            to,
            "version",
            &format!(
                "{} release(s) record the version they produced: a checklist is work to do, \
                 and what already shipped has no box to tick",
                grouped.recorded_versions
            ),
        );
    }
    if grouped.dropped > 0 {
        report.not_carried(
            to,
            "status",
            &format!(
                "{} dropped task(s) are not written at all, because an empty box reads as \
                 outstanding work",
                grouped.dropped
            ),
        );
    }
    for NotCarried(field, carried, cost) in NOT_CARRIED {
        let n = written.iter().filter(|t| carried(t)).count();
        if n > 0 {
            report.not_carried(to, field, &format!("{n} task(s) carry it: {cost}"));
        }
    }
    // Every entry here is a dependency: membership points *at* a milestone
    // (`edge_kind`), and no row written by this exporter is one.
    let with_edges = written.iter().filter(|t| !t.blocked_by.is_empty()).count();
    if with_edges > 0 {
        report.not_carried(
            to,
            "blocked_by",
            &format!(
                "{with_edges} task(s) wait on another, and row order is not a dependency, so \
                 nothing here records it"
            ),
        );
    }
    report_phase_loss(&written, to, report);
    report_export_placement(grouped, to, report);
    report_text_fidelity(&written, grouped, to, report);
}

/// A checklist has no phase slot even when every row shares one value.
fn report_phase_loss(written: &[&Task], to: &str, report: &mut Report) {
    let phases: BTreeSet<u32> = written.iter().map(|t| t.phase).collect();
    match phases.len() {
        0 => {}
        1 => {
            let phase = phases.first().expect("one phase");
            report.not_carried(
                to,
                "phase",
                &format!(
                    "all written tasks carry {phase}; preserve it when importing this checklist \
                     again with `--phase {phase}`"
                ),
            );
        }
        count => {
            report.not_carried(
                to,
                "phase",
                &format!(
                    "the work spans {count} values ({}) and a checklist is one list, so no \
                     single import flag can restore them",
                    phases
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
    }
}

/// Where the rows landed.
fn report_export_placement(grouped: &Grouped, to: &str, report: &mut Report) {
    if let Some(first) = grouped.loose.first() {
        // The ids this plan actually has, not a pair invented for the sentence:
        // a reader told to run `aplan mv T-001 --cut M-core` against a plan with
        // neither is being shown a command that fails. The import half already
        // names real ids in its own advice for the same reason.
        let how = match grouped.cuts.first() {
            Some((cut, _)) => format!(". `aplan mv {} --cut {}` files one", first.id, cut.id),
            None => {
                ". This plan declares no cuts, so there is no heading to file one under".to_string()
            }
        };
        report.push(Notice::new(
            to,
            NoticeKind::Placement,
            format!(
                "{} task(s) are in no cut and are written before the first heading: a heading \
                 here is a cut, and inventing one would assert a grouping the plan does not \
                 have{how}",
                grouped.loose.len()
            ),
        ));
    }
    if !grouped.in_two_cuts.is_empty() {
        report.push(Notice::new(
            to,
            NoticeKind::Placement,
            format!(
                "{} task(s) are listed by more than one cut and are written under the first: {}",
                grouped.in_two_cuts.len(),
                grouped.in_two_cuts.join(", ")
            ),
        ));
    }
    if grouped.cancelled_cuts > 0 {
        report.push(Notice::new(
            to,
            NoticeKind::Placement,
            format!(
                "{} cancelled cut(s) still hold live work, and are written as no heading: their \
                 members are above the first one. A heading here reads as a live section, and \
                 the plan says that cut is not",
                grouped.cancelled_cuts
            ),
        ));
    }
    if grouped.empty_cuts > 0 {
        report.push(Notice::new(
            to,
            NoticeKind::Placement,
            format!(
                "{} milestone(s) have no row to write and so become no heading: a release holds \
                 cuts rather than work, and a cut whose members are all dropped holds none",
                grouped.empty_cuts
            ),
        ));
    }
    let rows = grouped.loose.len() + grouped.cuts.iter().map(|(_, m)| m.len()).sum::<usize>();
    if rows > 1 {
        report.push(Notice::new(
            to,
            NoticeKind::Placement,
            "rows are in id order, which carries no meaning: the plan's own order is derived \
             from what blocks what, and writing that here would reorder the file every time an \
             estimate changed"
                .to_string(),
        ));
    }
}

/// How faithfully the text was carried, which is what a reader has to know
/// before re-importing this file.
fn report_text_fidelity(written: &[&Task], grouped: &Grouped, to: &str, report: &mut Report) {
    let claimed = written
        .iter()
        .filter(|t| t.status != Status::Done && t.tags.iter().any(|g| g == CHECKED_TAG))
        .count();
    if claimed > 0 {
        report.push(Notice::new(
            to,
            NoticeKind::Provenance,
            format!(
                "{claimed} task(s) were ticked in the checklist they were imported from and are \
                 not `done` here, so their boxes are written empty: a tick is a claim, and only \
                 evidence closes a task"
            ),
        ));
    }
    // Text this had to change to make it one line of a document a person opens.
    // Reported because it is the one case where a row is not the title verbatim,
    // and a reader comparing the two would otherwise find a difference nothing
    // announced.
    let retyped = written
        .iter()
        .map(|t| t.title.as_str())
        .chain(grouped.cuts.iter().map(|(c, _)| c.title.as_str()))
        .filter(|text| one_row(text) != *text)
        .count();
    if retyped > 0 {
        report.push(Notice::new(
            to,
            NoticeKind::Provenance,
            format!(
                "{retyped} title(s) carry a line break or a control character and are written \
                 with it collapsed or replaced by `{}`: a row is one line, and a document is \
                 opened by people who never ran this tool",
                aplan_output::sanitize::MARKER
            ),
        ));
    }
    // The one asymmetry in the round trip, stated rather than discovered: this
    // writes a title verbatim and the importer shortens a long one, so a file
    // regenerated from a re-import of this one would differ in exactly these
    // rows.
    let shortened = written
        .iter()
        .filter(|t| title_for(&t.title).is_some())
        .count();
    if shortened > 0 {
        report.push(Notice::new(
            to,
            NoticeKind::Provenance,
            format!(
                "{shortened} title(s) are written in full and would be shortened to their first \
                 sentence or lead-in if this file were imported again"
            ),
        ));
    }
}

impl PlanTarget for Checklist {
    fn flavor(&self) -> &'static str {
        "checklist"
    }

    fn render(&self, tasks: &[&Task], options: &ExportOptions) -> Result<Export, ExportError> {
        let grouped = group(tasks);
        if grouped.loose.is_empty() && grouped.cuts.is_empty() {
            let milestones = tasks.iter().filter(|t| t.is_milestone()).count();
            return Err(ExportError::Empty {
                detail: Some(format!(
                    ": this writes one `- [ ]` row per task, and the plan has none to write. It \
                     has {} task(s), {milestones} of them milestones, {} dropped",
                    tasks.len(),
                    grouped.dropped
                )),
            });
        }
        let mut report = Report::new();
        report_export_losses(&grouped, &options.destination, &mut report);
        Ok(Export {
            contents: emit(&grouped),
            report,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P0000004: &str = "# p0000004 -- Roadmap\n\
        \n\
        Derived from RESEARCH.md section 9.\n\
        \n\
        ## Phase 0 -- de-risk the hard 20%\n\
        \n\
        Prove the riskiest pieces independently of any GUI framework.\n\
        \n\
        - [x] `p0000004-core`: tokio runtime + a minimal INDI client (adopt/fork the `indi`\n\
        \x20     crate; drop the `libcfitsio` dependency in favor of `fitsrs`).\n\
        - [ ] `p0000004-render`: tiled `wgpu` viewer + real-time STF/MTF stretch shader.\n\
        \n\
        ## Phase 1 -- desktop MVP\n\
        \n\
        - [ ] `p0000004-desktop`: `eframe`/`egui` shell.\n\
        \x20 - [ ] a nested item\n";

    fn opts() -> Options {
        Options::default()
    }

    #[test]
    fn a_title_is_the_first_sentence_and_the_item_survives_in_full() {
        // p00025 writes checklist items as miniature changelog entries, so a
        // faithful import made an 832-character title that nobody can read in
        // `ready`, a roadmap table, or a diagnostic (pre-public adoption evidence,
        // item 3). Measured on that file: 832 characters became 65.
        let text = "**Close the gap that `schema` exposed.** The generated schema is faithful \
                    to the core, and the core is not the model.";
        assert_eq!(
            first_sentence(text),
            Some("**Close the gap that `schema` exposed.**"),
            "the stop is inside bold markup, which is how a bold sentence ends"
        );
    }

    #[test]
    fn the_shorter_of_the_sentence_and_the_lead_in_wins() {
        // `p00025`'s longest item has a 213-character first sentence and a
        // 38-character lead-in. Preferring the sentence, or simply taking the
        // first rule that matched, would keep a title nobody can read while a
        // better one sat in the same string (`A1-055`).
        let text = "**`idl` layer (delegated)** \u{2014} `cargo xtask spec idl`: TypeSpec becomes \
                    an OpenAPI document with a banner. Then the gate recompiles it.";
        assert_eq!(title_for(text), Some("**`idl` layer (delegated)**"));
    }

    #[test]
    fn a_lead_in_below_the_floor_tries_the_next_separator() {
        // Without this, one real item cut to `(Superseded by S6`, which says less
        // than the whole line did, so the rule gave up and kept the whole line.
        let text = "(Superseded by S6 \u{2014} originally one workspace) `Cargo.toml` \u{2014} \
                    the resolver, the lints, and the release profile";
        let got = title_for(text).expect("a title");
        assert!(got.len() >= MIN_TITLE, "a fragment is not a title: {got}");
        assert!(got.starts_with("(Superseded by S6"), "{got}");
        assert!(
            got.ends_with("`Cargo.toml`"),
            "it takes the second cut: {got}"
        );
    }

    #[test]
    fn a_heading_keeps_its_letters_whatever_script_they_are_in() {
        // The pilot's exact heading. Before this, `Cafe` lost its accent, `naive`
        // became `na-ve`, `resume` became `r-sum`, and the CJK run vanished
        // without trace (pre-public adoption evidence, item 4).
        assert_eq!(
            tag_for("Caf\u{e9} na\u{ef}ve \u{4e2d}\u{6587}\u{5b57} r\u{e9}sum\u{e9}"),
            "caf\u{e9}-na\u{ef}ve-\u{4e2d}\u{6587}\u{5b57}-r\u{e9}sum\u{e9}"
        );
        // A heading in a non-Latin script produced an empty tag; now it produces
        // the heading.
        assert_eq!(
            tag_for("\u{4e2d}\u{6587}\u{5b57}"),
            "\u{4e2d}\u{6587}\u{5b57}"
        );
    }

    #[test]
    fn a_decomposed_letter_keeps_its_accent_too() {
        // The same word written two ways. Many editors and macOS produce the
        // second, and without the combining-mark rule it still became `cafe`:
        // the defect this task fixed, surviving in a different encoding.
        let precomposed = "Caf\u{e9}";
        let decomposed = "Cafe\u{301}";
        assert_eq!(tag_for(precomposed), "caf\u{e9}");
        assert_eq!(tag_for(decomposed), "cafe\u{301}");
        // A mark with nothing before it is a stray, not a letter.
        assert_eq!(tag_for("\u{301}alpha"), "alpha");
        // And a mark after a separator does not reattach across it.
        assert_eq!(tag_for("a - \u{301}b"), "a-b");
    }

    #[test]
    fn a_plain_ascii_heading_is_slugified_exactly_as_before() {
        // The near-miss: the Unicode test is a superset of the ASCII one, so
        // nothing about an ASCII heading may change. A rule that fixed the first
        // case by changing this one would be a different defect.
        assert_eq!(tag_for("Foundation"), "foundation");
        assert_eq!(tag_for("CI and release"), "ci-and-release");
        assert_eq!(tag_for("  Spaced  --  out  "), "spaced-out");
        assert_eq!(tag_for("v0.2 rollout"), "v0-2-rollout");
    }

    #[test]
    fn a_heading_with_nothing_to_slugify_is_reported_rather_than_silently_untagged() {
        // `import` is lossy by design and says so. This was the one loss it made
        // in silence.
        let src = "## !!! ???\n\n- [ ] item\n";
        let import = Checklist
            .parse("s.md", src, &opts())
            .expect("a checklist parses");
        assert!(
            import.tasks[0].tags.is_empty(),
            "an all-separator tag is not a tag"
        );
        assert!(
            import
                .report
                .notices()
                .iter()
                .any(|n| n.message.contains("nothing a tag can be made from")),
            "the loss must be reported: {:?}",
            import.report.notices()
        );
        // And a heading that does slugify says nothing, or the notice is noise.
        let ok = Checklist
            .parse("s.md", "## Caf\u{e9}\n\n- [ ] item\n", &opts())
            .expect("parses");
        assert!(
            !ok.report
                .notices()
                .iter()
                .any(|n| n.message.contains("nothing a tag can be made from")),
            "a heading that slugifies must not be reported"
        );
    }

    #[test]
    fn a_title_is_never_cut_inside_emphasis() {
        // `**Close the prompt-model format gap` renders as literal asterisks, or
        // swallows whatever follows it. A title with unbalanced markup is exactly
        // the unreadable title this task exists to remove, so a separator inside
        // emphasis is not a separator, the same way one inside a code span is not.
        //
        // The real file's emphasis happened to be short enough that the length
        // floor hid this, so it took a constructed case to find (`A1-055`).
        for text in [
            "**Close the prompt-model format gap: the schema** and the rest of it",
            "**Ship the aggregator thing \u{2014} properly** and then all the rest",
            "*One emphasised clause: and its tail* plus a trailing part of it",
        ] {
            let got = title_for(text);
            if let Some(t) = got {
                assert_eq!(
                    t.matches("**").count() % 2,
                    0,
                    "unbalanced strong emphasis in {t:?}"
                );
                assert_eq!(
                    t.replace("**", "").matches('*').count() % 2,
                    0,
                    "unbalanced emphasis in {t:?}"
                );
            }
        }
        // A separator *after* the emphasis closes is still a separator, or this
        // guard would have turned the rule off rather than corrected it.
        assert_eq!(
            title_for("**The bindings layer**: the generated types and their banner"),
            Some("**The bindings layer**")
        );

        // The same invariant for the sentence rule, which cut `**Ship it.` and
        // left the bold open.
        assert_eq!(
            title_for("**Ship it. Then the rest** and more besides here"),
            None,
            "an unbalanced first sentence is not a title"
        );
        // And the case this task started from must still work: the stop is inside
        // bold and the closing marker comes with it, so the result is balanced and
        // is exactly the title wanted.
        assert_eq!(
            title_for("**Close the gap that `schema` exposed.** The rest follows here."),
            Some("**Close the gap that `schema` exposed.**")
        );
    }

    #[test]
    fn a_short_first_sentence_is_a_good_title() {
        // The floor applies to fragments, not to sentences: "Ship it." says what
        // it says, and rejecting it would keep the whole paragraph instead.
        assert_eq!(
            title_for("Ship it. Then wire the aggregator and the registry."),
            Some("Ship it.")
        );
    }

    #[test]
    fn an_item_with_no_sentence_and_no_separator_is_left_alone() {
        // A comma-separated list of filenames has no honest boundary, and this
        // importer invents nothing. Measured on the real file: three such items
        // remain long, and shortening them would mean choosing a cut the author
        // did not write.
        let list =
            "`cli/dist/pre-commit-hooks.yaml`, `.gitignore`, `.gitattributes`, `.editorconfig`";
        assert_eq!(title_for(list), None);
    }

    #[test]
    fn a_one_sentence_item_keeps_its_whole_text_as_its_title() {
        // No body duplication, and no title invented: a short item is untouched.
        assert_eq!(first_sentence("Wire the remaining delegated layers"), None);
        assert_eq!(title_for("Wire the remaining delegated layers"), None);
        assert_eq!(first_sentence("Add the derive step for crates.yaml."), None);
    }

    #[test]
    fn the_split_survives_the_punctuation_that_actually_occurs() {
        // Each of these is a full stop that does not end a sentence, and each was
        // taken from the plans this importer has been pointed at (`A1-055`).
        for one_sentence in [
            "Run `ci/scripts/spec.sh` before the gate",
            "Pin the action at `v0.2.0` and no later",
            "Everything the layer emits, e.g. the banner and the checksum",
            "Byte-compare in a temp dir (non-mutating)",
        ] {
            assert_eq!(
                first_sentence(one_sentence),
                None,
                "must not split: {one_sentence}"
            );
        }
        // And a real second sentence still splits, or the guard above would be
        // satisfied by a function that never splits anything.
        assert_eq!(
            first_sentence("Pin it at `v0.2.0`. Then check the banner."),
            Some("Pin it at `v0.2.0`."),
            "a genuine sentence boundary still cuts"
        );
        // A stop *inside* a code span, followed by a space, is the only case the
        // code-span guard is load-bearing for: the whitespace rule alone already
        // rejects `spec.sh` and `v0.2.0`. The real file has seven such spans.
        // Caught by a surviving mutation, so until now the guard was either
        // unnecessary or untested.
        assert_eq!(
            first_sentence("Run `cargo x. y` first. Then the gate."),
            Some("Run `cargo x. y` first."),
            "a full stop inside a code span is punctuation, not prose"
        );
    }

    #[test]
    fn it_reads_items_headings_and_wrapped_lines() {
        let import = Checklist.parse("ROADMAP.md", P0000004, &opts()).unwrap();
        assert_eq!(import.tasks.len(), 4, "{:?}", import.tasks);

        // A wrapped continuation belongs to the item above it, not to nothing:
        // p0000004 wraps nearly every line, so dropping these would truncate most
        // titles mid-sentence.
        assert!(
            import.tasks[0].title.ends_with("in favor of `fitsrs`)."),
            "{}",
            import.tasks[0].title
        );
        assert!(import.tasks[0].title.contains("tokio runtime"));

        assert_eq!(import.tasks[0].tags[0], "phase-0-de-risk-the-hard-20");
        assert_eq!(import.tasks[2].tags[0], "phase-1-desktop-mvp");
        // The document title (`#`) is not a section.
        assert!(!import.tasks[0].tags.iter().any(|t| t.contains("p0000004")));
    }

    #[test]
    fn a_ticked_box_is_a_claim_not_a_status() {
        let import = Checklist.parse("ROADMAP.md", P0000004, &opts()).unwrap();
        let ticked = &import.tasks[0];
        assert_eq!(ticked.status, Status::Todo, "a checkbox proves nothing");
        assert!(ticked.tags.iter().any(|t| t == "checklist:checked"));
        assert!(ticked.evidence.is_empty());
        assert!(
            import
                .report
                .notices()
                .iter()
                .any(|n| n.message.contains("claim, not proof")),
            "the report must name the fix: {:?}",
            import.report.notices()
        );
    }

    #[test]
    fn implemented_as_done_takes_the_weaker_record_and_dates_it() {
        let options = Options {
            implemented_as: ImplementedAs::Done {
                as_of: "2026-08-04".into(),
            },
            ..Options::default()
        };
        let import = Checklist.parse("ROADMAP.md", P0000004, &options).unwrap();
        let ticked = &import.tasks[0];
        assert_eq!(ticked.status, Status::Done);
        assert_eq!(ticked.evidence.len(), 1);
        assert_eq!(ticked.evidence[0].kind, EvidenceKind::Note);
        // `APLAN010` needs the date, and aplan never invents provenance.
        assert_eq!(ticked.evidence[0].captured.as_deref(), Some("2026-08-04"));
    }

    #[test]
    fn nesting_and_order_are_reported_rather_than_invented() {
        let import = Checklist.parse("ROADMAP.md", P0000004, &opts()).unwrap();
        let messages: Vec<&str> = import
            .report
            .notices()
            .iter()
            .map(|n| n.message.as_str())
            .collect();
        assert!(
            messages.iter().any(|m| m.contains("nested item")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("list order")),
            "{messages:?}"
        );
        // Nothing may be written to `blocked_by` from position alone.
        assert!(import.tasks.iter().all(|t| t.blocked_by.is_empty()));
    }

    #[test]
    fn generated_ids_are_declared_because_the_source_had_none() {
        let import = Checklist.parse("ROADMAP.md", P0000004, &opts()).unwrap();
        assert_eq!(import.tasks[0].id, "T-001");
        assert_eq!(import.tasks[3].id, "T-004");
        assert!(
            import
                .report
                .notices()
                .iter()
                .any(|n| n.message.contains("no task ids")),
            "inventing an id is the opposite of dropping a field, and must be said"
        );

        let options = Options {
            id_prefix: "AL1".into(),
            ..Options::default()
        };
        let import = Checklist.parse("ROADMAP.md", P0000004, &options).unwrap();
        assert_eq!(import.tasks[0].id, "AL1-001");
    }

    #[test]
    fn the_prose_above_an_item_survives_in_its_body() {
        let import = Checklist.parse("ROADMAP.md", P0000004, &opts()).unwrap();
        let body = &import.bodies[0];
        assert!(body.contains("Prove the riskiest pieces"), "{body}");
        assert!(body.contains("Phase 0"), "{body}");
        assert!(
            body.contains("line 9"),
            "the body must point at the source line: {body}"
        );
    }

    /// The three ways the first version of the scanner read a realistic roadmap
    /// wrongly. All were found by feeding it the constructs a roadmap actually
    /// contains, rather than the ones the parser was written for.
    const TRICKY: &str = "# Roadmap\n\
        \n\
        ## Phase 0\n\
        \n\
        - [ ] First item\n\
        \n\
        \x20 An indented paragraph that is NOT a continuation.\n\
        \n\
        ## Phase 1\n\
        \n\
        \x20 Indented prose under a new heading.\n\
        \n\
        - [ ] Second item\n\
        \n\
        Example of the syntax:\n\
        \n\
        ```markdown\n\
        - [ ] documentation, not a task\n\
        ## Not a heading either\n\
        ```\n\
        \n\
        - [ ] Third item\n\
        - [ ]\n";

    #[test]
    fn a_blank_line_ends_a_wrapped_item() {
        let import = Checklist.parse("r.md", TRICKY, &opts()).unwrap();
        assert_eq!(
            import.tasks[0].title, "First item",
            "an indented paragraph below a blank line is prose, not the rest of the title"
        );
    }

    #[test]
    fn a_heading_ends_a_wrapped_item() {
        // The worst of the three: indented prose belonging to the *next* section
        // was glued onto the last item of the previous one.
        let import = Checklist.parse("r.md", TRICKY, &opts()).unwrap();
        assert!(
            !import.tasks[0].title.contains("Indented prose"),
            "{}",
            import.tasks[0].title
        );
    }

    #[test]
    fn a_fenced_block_is_not_content() {
        // A roadmap that documents its own syntax was importing the example as a
        // task, and the `##` inside the fence became a section that real tasks
        // below it were then filed under.
        let import = Checklist.parse("r.md", TRICKY, &opts()).unwrap();
        assert_eq!(import.tasks.len(), 3, "{:?}", import.tasks);
        assert!(
            !import
                .tasks
                .iter()
                .any(|t| t.title.contains("documentation")),
            "{:?}",
            import.tasks
        );
        assert_eq!(
            import.tasks[2].title, "Third item",
            "the task after the fence is a real one"
        );
        assert_eq!(
            import.tasks[2].tags[0], "phase-1",
            "a heading inside a fence must not become the section: {:?}",
            import.tasks[2].tags
        );
    }

    #[test]
    fn a_checkbox_with_no_text_is_counted_rather_than_passed_over() {
        let import = Checklist.parse("r.md", TRICKY, &opts()).unwrap();
        assert!(
            import
                .report
                .notices()
                .iter()
                .any(|n| n.message.contains("no text")),
            "a dropped line has to be said out loud: {:?}",
            import.report.notices()
        );
    }

    #[test]
    fn a_longer_fence_is_not_closed_by_a_shorter_one() {
        let doc = "## S\n\n````\n```\n- [ ] inside\n````\n\n- [ ] outside\n";
        let import = Checklist.parse("r.md", doc, &opts()).unwrap();
        assert_eq!(import.tasks.len(), 1, "{:?}", import.tasks);
        assert_eq!(import.tasks[0].title, "outside");
    }

    #[test]
    fn a_document_with_no_checkboxes_is_not_an_import() {
        // p0000004's roadmap after it adopted aplan: prose, no boxes. Reading it
        // as "zero tasks, success" would be a silent no-op.
        let err = Checklist
            .parse("ROADMAP.md", "# Roadmap\n\nJust prose.\n", &opts())
            .unwrap_err();
        assert!(matches!(err, ImportError::Empty { .. }), "{err:?}");
        let err = Checklist.parse("ROADMAP.md", "   \n", &opts()).unwrap_err();
        assert!(matches!(err, ImportError::Empty { .. }), "{err:?}");
    }

    #[test]
    fn checkbox_recognises_the_spellings_that_occur() {
        assert_eq!(checkbox("- [ ] a"), Some((false, "a")));
        assert_eq!(checkbox("* [x] a"), Some((true, "a")));
        assert_eq!(checkbox("+ [X] a"), Some((true, "a")));
        // A box with no text is still a checkbox line; the caller counts it
        // rather than passing over it in silence.
        assert_eq!(checkbox("- [ ]"), Some((false, "")));
        // Not a checkbox at all: a plain bullet, a link, ordinary prose.
        assert_eq!(checkbox("- plain bullet"), None);
        assert_eq!(checkbox("- [link](x)"), None);
        assert_eq!(checkbox("text"), None);
    }

    #[test]
    fn heading_ignores_the_document_title_and_empty_headings() {
        assert_eq!(heading("## Phase 0"), Some("Phase 0"));
        assert_eq!(heading("### Sub"), Some("Sub"));
        assert_eq!(
            heading("# Title"),
            None,
            "`#` is the document, not a section"
        );
        assert_eq!(heading("##"), None);
        assert_eq!(heading("not a heading"), None);
    }

    #[test]
    fn a_tag_is_typeable_or_it_is_not_a_tag() {
        assert_eq!(
            tag_for("Phase 0 -- de-risk the hard 20%"),
            "phase-0-de-risk-the-hard-20"
        );
        assert_eq!(tag_for("v0.8: Adoption"), "v0-8-adoption");
        // A heading with nothing typeable yields no tag rather than `-`.
        assert_eq!(tag_for("!!!"), "");
    }
    // -- The export half (`A1-189`). ------------------------------------------

    fn plain(id: &str, title: &str, status: Status) -> Task {
        Task {
            id: id.to_string(),
            title: title.to_string(),
            phase: 1,
            status,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: BTreeMap::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    fn cut(id: &str, title: &str, members: &[&str]) -> Task {
        let mut m = plain(id, title, Status::Todo);
        m.tags = vec!["milestone".to_string()];
        m.blocked_by = members.iter().map(|s| (*s).to_string()).collect();
        m
    }

    fn out(tasks: &[&Task]) -> Export {
        Checklist
            .render(tasks, &ExportOptions::default())
            .expect("the fixture has rows to write")
    }

    #[test]
    fn a_cut_becomes_a_heading_and_its_members_become_rows() {
        // The plan's own grouping, not a guess from a tag: membership is an edge
        // (ADR-0030), and reading it through `edge_kind` is what stops this
        // renderer and `graph` disagreeing about what a cut holds.
        let a = plain("T-001", "Build the core", Status::Done);
        let b = plain("T-002", "Build the renderer", Status::Todo);
        let m = cut("M-core", "Core validator", &["T-001", "T-002"]);
        let export = out(&[&a, &b, &m]);
        assert_eq!(
            export.contents,
            "## Core validator\n\n- [x] Build the core\n- [ ] Build the renderer\n"
        );
    }

    #[test]
    fn only_a_done_task_gets_a_ticked_box() {
        // A checkbox has two states and the model has five. Mapping
        // `in_progress` onto the ticked one would be this tool asserting a
        // completion, which is the claim it refuses to take on trust in the
        // other direction.
        let live = plain("T-001", "Half done", Status::InProgress);
        let stuck = plain("T-002", "Waiting", Status::Blocked);
        let export = out(&[&live, &stuck]);
        assert_eq!(export.contents, "- [ ] Half done\n- [ ] Waiting\n");
        let report = export.report.to_string();
        assert!(
            report.contains("`status`") && report.contains("2 task(s)"),
            "both were flattened into an empty box in silence: {report}"
        );
    }

    #[test]
    fn a_dropped_task_is_left_out_and_the_report_says_so() {
        // An empty box reads as outstanding work, and abandoned work is not
        // outstanding. Left out, and counted, because leaving it out in silence
        // is the omission ADR-0012 clause 3 forbids.
        let live = plain("T-001", "Real work", Status::Todo);
        let gone = plain("T-002", "Abandoned", Status::Dropped);
        let export = out(&[&live, &gone]);
        assert_eq!(export.contents, "- [ ] Real work\n");
        let report = export.report.to_string();
        assert!(
            report.contains("1 dropped task(s) are not written"),
            "{report}"
        );
    }

    #[test]
    fn a_task_in_no_cut_is_written_before_the_first_heading() {
        // Inverse-exact rather than tidy: the importer reads a row above the
        // first `##` as belonging to no section, so a task belonging to no cut
        // goes there. The alternative was inventing a heading, which asserts a
        // grouping the plan does not have.
        let loose = plain("T-000", "Unfiled", Status::Todo);
        let member = plain("T-001", "Filed", Status::Todo);
        let m = cut("M-core", "Core", &["T-001"]);
        let export = out(&[&loose, &member, &m]);
        assert_eq!(export.contents, "- [ ] Unfiled\n\n## Core\n\n- [ ] Filed\n");
        assert!(
            export.report.to_string().contains("in no cut"),
            "{}",
            export.report
        );
    }

    #[test]
    fn a_task_two_cuts_both_list_is_written_once_and_named() {
        // Rare and legal: nothing forbids two milestones listing one id. Writing
        // it twice would be a document that counts its own work wrong, and
        // choosing silently would be a placement nobody could predict.
        let shared = plain("T-001", "Shared work", Status::Todo);
        let first = cut("M-a", "Alpha", &["T-001"]);
        let second = cut("M-b", "Beta", &["T-001"]);
        let export = out(&[&shared, &first, &second]);
        assert_eq!(export.contents.matches("Shared work").count(), 1);
        assert!(
            export.contents.starts_with("## Alpha"),
            "{}",
            export.contents
        );
        let report = export.report.to_string();
        assert!(
            report.contains("more than one cut") && report.contains("T-001"),
            "{report}"
        );
    }

    #[test]
    fn a_release_writes_no_heading_because_it_holds_cuts_rather_than_work() {
        // A release lists cuts, and a cut listing a cut is a dependency edge,
        // not membership. So a release has no rows, and a heading with nothing
        // under it is noise rather than information.
        let task = plain("T-001", "Work", Status::Todo);
        let c = cut("M-core", "Core", &["T-001"]);
        let mut release = cut("M-v1", "v1.0", &["M-core"]);
        release.tags.push("release".to_string());
        let export = out(&[&task, &c, &release]);
        assert!(!export.contents.contains("v1.0"), "{}", export.contents);
        assert!(
            export.report.to_string().contains("no row to write"),
            "{}",
            export.report
        );
    }

    #[test]
    fn a_plans_own_escape_cannot_reach_the_generated_document() {
        // The defence `A1-289` shipped for every human surface, at a worse
        // destination than a terminal: these bytes are committed, and the next
        // person to meet them never ran this tool. Written unfiltered, the same
        // plan produced two different documents -- the stream writer strips an
        // escape and its parameters, a file keeps them -- so this is also what
        // makes the two write paths agree.
        let steering = plain("T-001", "erase \u{1b}[2J the screen", Status::Todo);
        // And the half that steers the *reading* rather than the terminal: this
        // title displays as `safe restrictive tail` if the override survives.
        let reordering = plain(
            "T-002",
            "safe\u{202e} evitcirtser\u{202c} tail",
            Status::Todo,
        );
        let export = out(&[&steering, &reordering]);
        // Per line: the newlines between rows are the document's own, and the
        // question is what is *in* a row.
        assert!(
            !export
                .contents
                .lines()
                .any(|line| line.chars().any(char::is_control)),
            "a control character reached a row: {:?}",
            export.contents
        );
        assert!(
            !export.contents.contains('\u{202e}') && !export.contents.contains('\u{202c}'),
            "a bidi override reached the document: {:?}",
            export.contents
        );
        assert!(
            export.contents.contains("[2J") && export.contents.contains("evitcirtser"),
            "the text was deleted rather than made inert: {:?}",
            export.contents
        );
        assert!(
            export.report.to_string().contains("2 title(s) carry"),
            "the rewrite happened in silence: {}",
            export.report
        );
    }

    #[test]
    fn a_line_break_in_a_title_does_not_become_a_second_row() {
        // A row is one line. A newline in a title makes a second line that is
        // not a checkbox, and the reader beside this glues it onto the row above
        // as a wrapped continuation, so the two halves of the round trip would
        // disagree about how many tasks there are.
        let split = plain("T-001", "first half\nsecond half", Status::Todo);
        let export = out(&[&split]);
        assert_eq!(export.contents, "- [ ] first half second half\n");
        let back = Checklist
            .parse("regenerated.md", &export.contents, &Options::default())
            .expect("one row");
        assert_eq!(back.tasks.len(), 1);
    }

    #[test]
    fn a_cut_listing_a_task_twice_is_one_cut_not_two() {
        // A duplicate edge is a botched edit, and `check` accepts it. Reporting
        // it as "listed by more than one cut" sends the reader looking for a
        // second cut that does not exist.
        let task = plain("T-001", "Work", Status::Todo);
        let mut cut = cut("M-a", "Alpha", &["T-001"]);
        cut.blocked_by.push("T-001".to_string());
        let export = out(&[&task, &cut]);
        assert_eq!(export.contents.matches("- [ ] Work").count(), 1);
        assert!(
            !export.report.to_string().contains("more than one cut"),
            "one cut was reported as two: {}",
            export.report
        );
    }

    #[test]
    fn a_cancelled_cut_is_not_a_heading_over_live_work() {
        // The dropped-row rule applied to structure. `APLAN027` is what tells
        // the author the plan disagrees with itself; this refuses to amplify it
        // into a document that reads as a live section.
        let live = plain("T-001", "Still live work", Status::Todo);
        let mut cancelled = cut("M-a", "A cancelled cut", &["T-001"]);
        cancelled.status = Status::Dropped;
        let export = out(&[&live, &cancelled]);
        assert_eq!(export.contents, "- [ ] Still live work\n");
        let report = export.report.to_string();
        assert!(report.contains("1 cancelled cut(s)"), "{report}");
    }

    #[test]
    fn a_task_a_live_cut_also_holds_is_not_reported_as_left_loose() {
        // The order trap: counted while the cuts were being walked, a task named
        // by a cancelled cut *and* a live one was reported as written above the
        // first heading whenever the cancelled cut sorted first. It is written
        // under the live cut, and the cancelled one has nothing to say about it.
        let shared = plain("T-001", "Shared work", Status::Todo);
        let mut cancelled = cut("M-a", "A cancelled cut", &["T-001"]);
        cancelled.status = Status::Dropped;
        let live = cut("M-b", "A live cut", &["T-001"]);
        let export = out(&[&shared, &cancelled, &live]);
        assert_eq!(
            export.contents, "## A live cut\n\n- [ ] Shared work\n",
            "the row belongs to the live cut"
        );
        let report = export.report.to_string();
        assert!(
            !report.contains("cancelled cut(s) still hold live work"),
            "nothing was left loose, and the report says otherwise: {report}"
        );
        assert!(
            !report.contains("in no cut"),
            "the row is in a cut: {report}"
        );
    }

    #[test]
    fn a_cut_sequenced_after_another_does_not_swallow_it_as_a_member() {
        // The `edge_kind` half, and the reason it is asked rather than derived
        // from `is_milestone` here: a cut whose `blocked_by` names an earlier cut
        // is sequencing, and its rows are its own.
        let first = plain("T-001", "Early work", Status::Todo);
        let second = plain("T-002", "Later work", Status::Todo);
        let early = cut("M-a", "Alpha", &["T-001"]);
        let mut late = cut("M-b", "Beta", &["T-002"]);
        late.blocked_by.push("M-a".to_string());
        let export = out(&[&first, &second, &early, &late]);
        assert_eq!(
            export.contents,
            "## Alpha\n\n- [ ] Early work\n\n## Beta\n\n- [ ] Later work\n"
        );
        assert!(
            !export.report.to_string().contains("more than one cut"),
            "the sequencing edge was read as membership: {}",
            export.report
        );
    }

    #[test]
    fn a_plan_with_no_rows_is_refused_rather_than_written_empty() {
        // An empty document is a lie a consumer cannot tell from a plan with no
        // work left, so this refuses and says what the plan does hold.
        let m = cut("M-core", "Core", &[]);
        let err = Checklist
            .render(&[&m], &ExportOptions::default())
            .expect_err("a plan with nothing to write is refused");
        let text = err.to_string();
        assert!(text.contains("1 task(s)"), "{text}");
        assert!(text.contains("1 of them milestones"), "{text}");
    }

    #[test]
    fn a_tick_that_was_only_a_claim_is_written_as_an_empty_box() {
        // The round trip's sharpest surprise, so it is stated rather than met:
        // an import turns a ticked box into a `todo` task plus this tag, and
        // writing it back ticked would re-assert the claim the tool refused.
        let mut claimed = plain("T-001", "Imported claim", Status::Todo);
        claimed.tags = vec![CHECKED_TAG.to_string()];
        let export = out(&[&claimed]);
        assert_eq!(export.contents, "- [ ] Imported claim\n");
        assert!(
            export
                .report
                .to_string()
                .contains("were ticked in the checklist"),
            "{}",
            export.report
        );
    }

    #[test]
    fn the_document_is_the_same_bytes_however_the_tasks_arrive() {
        // Byte-identical across runs is the property a generated file gated in
        // CI stands on (HLD section 14), and the caller's order is not something
        // this can rely on.
        let a = plain("T-001", "One", Status::Todo);
        let b = plain("T-002", "Two", Status::Done);
        let m = cut("M-core", "Core", &["T-002", "T-001"]);
        let forwards = out(&[&a, &b, &m]);
        let backwards = out(&[&m, &b, &a]);
        assert_eq!(forwards.contents, backwards.contents);
        assert_eq!(forwards.report, backwards.report);
    }

    /// **The census: every field of the task model is carried or named.**
    ///
    /// Walked from `Task`'s own serialised keys rather than from a list somebody
    /// typed, which is the discipline `NoticeKind::ALL` and `Rule::ALL` already
    /// carry here. A field added to the model that this exporter silently
    /// discards fails here, which is the whole of ADR-0012 clause 3 read in the
    /// outgoing direction.
    #[test]
    fn every_task_field_is_carried_or_named() {
        let mut rich = plain("T-001", "Rich task", Status::Done);
        rich.owner = Some("alice".to_string());
        rich.effort_days = Some(2.0);
        rich.blocked_by = vec!["T-000".to_string()];
        rich.prefer_after = vec!["T-000".to_string()];
        rich.evidence = vec![Evidence {
            kind: EvidenceKind::Note,
            reference: "it happened".to_string(),
            captured: Some("2026-01-01".to_string()),
        }];
        rich.paths = vec!["src/**".to_string()];
        rich.traces = vec!["docs/x.md#y".to_string()];
        rich.trace_fingerprints = [("docs/x.md#y".to_string(), "abc".to_string())]
            .into_iter()
            .collect();
        rich.evidence_fingerprints = [("file:docs/proof.md".to_string(), "def".to_string())]
            .into_iter()
            .collect();
        rich.tags = vec!["cross-cutting".to_string()];
        rich.revisit_at = Some("2026-12-01T00:00:00Z".to_string());
        // A second phase and a second status, so the two fields that are only
        // reported when the plan exercises them are exercised.
        let mut other = plain("T-002", "Other task", Status::InProgress);
        other.phase = 2;
        let gone = plain("T-003", "Gone", Status::Dropped);

        let export = out(&[&rich, &other, &gone]);
        let report = export.report.to_string();
        let value = serde_json::to_value(&rich).expect("a task serialises");
        let fields: Vec<String> = value
            .as_object()
            .expect("a task is an object")
            .keys()
            .cloned()
            .collect();
        assert!(
            fields.len() >= 14,
            "only {} field(s) were walked, so the census is not reaching the model: {fields:?}",
            fields.len()
        );

        for field in &fields {
            // The one field the shape carries: it is the row's text.
            if field == "title" {
                assert!(
                    export.contents.contains("Rich task"),
                    "the title is what a row says, and it is not in the document"
                );
                continue;
            }
            assert!(
                report.contains(&format!("`{field}`")),
                "`{field}` left the plan without a word about it:\n{report}"
            );
        }
    }

    #[test]
    fn a_long_title_is_written_in_full_and_the_asymmetry_is_stated() {
        // The one place the round trip is not a fixed point: this writes a title
        // verbatim and the importer shortens a long one, so a file regenerated
        // after a re-import differs in exactly these rows. Stated in the report
        // rather than left for a reader to discover from a diff.
        let long = plain(
            "T-001",
            "Close the gap that the schema exposed. The generated schema is faithful to the \
             core, and the core is not the model.",
            Status::Todo,
        );
        let export = out(&[&long]);
        assert!(export.contents.contains("and the core is not the model."));
        assert!(
            export.report.to_string().contains("would be shortened"),
            "{}",
            export.report
        );
    }

    #[test]
    fn what_this_writes_is_what_the_importer_reads_back() {
        // The round trip, in the crate that owns both halves: the document this
        // renders is parsed by the reader beside it, and the rows, the boxes and
        // the section each come back meaning what they meant.
        let done = plain("T-001", "Build the core", Status::Done);
        let todo = plain("T-002", "Build the renderer", Status::Todo);
        let loose = plain("T-003", "Write the docs", Status::Todo);
        let m = cut("M-core", "Core validator", &["T-001", "T-002"]);
        let document = out(&[&done, &todo, &loose, &m]).contents;

        let options = Options {
            implemented_as: ImplementedAs::Done {
                as_of: "2026-01-01".to_string(),
            },
            ..Options::default()
        };
        let back = Checklist
            .parse("regenerated.md", &document, &options)
            .expect("the regenerated file is one this importer accepts");

        let titles: Vec<&str> = back.tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Write the docs", "Build the core", "Build the renderer"],
            "every row came back, in the order it was written"
        );
        assert_eq!(
            back.tasks[1].status,
            Status::Done,
            "the ticked box came back"
        );
        assert_eq!(back.tasks[0].status, Status::Todo);
        assert!(
            back.tasks[1].tags.iter().any(|t| t == "core-validator"),
            "the heading came back as the section tag: {:?}",
            back.tasks[1].tags
        );
        assert!(
            back.tasks[0].tags.is_empty(),
            "the row above the first heading belongs to no section: {:?}",
            back.tasks[0].tags
        );
    }

    #[test]
    fn regenerating_from_what_a_regeneration_produced_changes_nothing() {
        // A fixed point, which is the property that catches a renderer emitting
        // something its own parser reads differently. Not from the *first* pass:
        // that one loses the cuts, because an import creates no milestones and
        // says so. From the second onward the document is stable.
        let done = plain("T-001", "Build the core", Status::Done);
        let todo = plain("T-002", "Build the renderer", Status::Todo);
        let m = cut("M-core", "Core validator", &["T-001", "T-002"]);
        let first = out(&[&done, &todo, &m]).contents;

        let options = Options {
            implemented_as: ImplementedAs::Done {
                as_of: "2026-01-01".to_string(),
            },
            ..Options::default()
        };
        let round = |doc: &str| {
            let back = Checklist
                .parse("regenerated.md", doc, &options)
                .expect("the regenerated file parses");
            let refs: Vec<&Task> = back.tasks.iter().collect();
            out(&refs).contents
        };
        let second = round(&first);
        let third = round(&second);
        assert_eq!(
            second, third,
            "the document is not stable under regeneration"
        );
    }
}
