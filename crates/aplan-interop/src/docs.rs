//! Does this document name tasks that exist (`A1-148`)?
//!
//! Every adopter builds this. This repository gates its own prose with
//! `cargo xtask check-docs`, the sixth pilot wrote `DN0-22` to parse ids out of
//! 28,000 words of `PLAN.md`, and the reason both exist is that an item reading
//! "tracked as `DN0-19`" when no such task exists **reads as handled**, which is
//! worse than an item with no disposition at all
//! (pre-public adoption evidence item 3).
//!
//! **Ids come from the plan, never from a pattern.** This tool deliberately does
//! not own id format (`A1-029`, and `A1-047`'s `id_strategy` config key): the
//! plans this has been measured against write `A1-042`, `AL1-07`, `WH2-06`,
//! `RD1-01`, `K0-3`, `M-v0.8`, `M-phase-2`, `M-p0-design` and `M-s1`, and a
//! regex describing all of those would also describe half the version strings
//! in a changelog. So the *families* are learned from the plan's own ids: a word
//! must carry a taught prefix followed by the family's numeric core, including
//! dot-separated digits or hyphen-separated suffixes only where the plan uses
//! them.
//!
//! What that costs, measured before it shipped: over this repository's 43
//! documents and four pilots' prose, 1,211 references were found and **two
//! false positives**, both from the same cause -- `M-v0.N`, which is how prose
//! writes "a milestone", and the ULID example in three documents, read as an id
//! built from the ULID's leading digits. Both were a token taken to end where
//! the digits ran out. A reference has to end at a word boundary as well as start at one, and
//! with that rule the false positives are zero across all five repositories and
//! the 575 document-revisions in their history.

use std::collections::BTreeSet;

/// One reference to a task id found in a document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference {
    /// The id as it was written.
    pub id: String,
    /// 1-based line, so the report is a place a reader can go.
    pub line: usize,
    /// 1-based column, in characters, which no report prints.
    ///
    /// It exists so `dangling` can keep its promise to return findings in the
    /// order they appear. That order used to be free: one scan, one pass, push
    /// order was document order. `A1-164` added a second pass for near-miss
    /// prefixes, and a second pass has no idea where its findings sit relative
    /// to the first's, so the audit of that task found the two merged by
    /// sorting on `(line, id)` -- alphabetical within a line, which is document
    /// order only by luck.
    pub column: usize,
    /// The taught prefixes this one is a digit's typo away from, when the
    /// reference's own prefix is not one the plan teaches (`A1-164`).
    ///
    /// Empty for an ordinary dangling reference, whose prefix is taught and
    /// whose number is simply not a task. **Every** near prefix, not the first:
    /// `A2-` is one digit from `A0-` and `A1-` alike, and printing one of them
    /// reads as knowledge rather than as the coin flip it is -- the rule
    /// `nearest` already follows for numbers.
    pub mistyped_prefix: Vec<String>,
}

/// What the plan teaches about how its ids are written (`A1-148`).
///
/// A prefix, whether ids under it carry dots, and whether their numeric core may
/// be followed by hyphen-separated words. All are learned from the plan's own
/// ids and none is a pattern this crate owns: `A1-042` teaches `A1-` without
/// dots or a suffix, `M-v0.8` teaches `M-v` with dots, `M-phase-2` teaches
/// `M-phase-`, and `M-p0-design` teaches a suffix-bearing `M-p` family.
///
/// The dot is why this is a type rather than a set of strings. `M-v0.N` is how
/// prose writes "a milestone" and must not read as `M-v0`; `aplan/tasks/T-999.md`
/// is a filename and *must* read as `T-999`, because a task file named in prose
/// that does not exist is the exact mistake this looks for. The two are the same
/// shape -- digits, a dot, letters -- and only the plan can say which is which.
#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    /// Longest prefix first so `M-v` wins over `M-` where a plan teaches both.
    entries: Vec<Family>,
}

#[derive(Debug, Clone)]
struct Family {
    prefix: String,
    dotted: bool,
    hyphenated_suffix: bool,
}

fn sort_families(entries: &mut [Family]) {
    entries.sort_by(|a, b| {
        b.prefix
            .chars()
            .count()
            .cmp(&a.prefix.chars().count())
            .then(a.prefix.cmp(&b.prefix))
    });
}

/// The characters that make a candidate part of a surrounding word rather
/// than a standalone identifier. Keep this single predicate on both sides of
/// the scanner: accepting `A1-042_suffix` as the known `A1-042` while rejecting
/// `prefix_A1-042` would make the documented word-boundary contract depend on
/// which side the punctuation sits.
fn continues_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `chars` is one or more non-empty, hyphen-separated alphanumeric
/// words and contains a letter. The caller hands this the separator itself.
/// Requiring a letter keeps a numeric tail such as `M-phase-2` in the ordinary
/// final-number family; [`crate::ids::contains_a_ulid`] separately keeps a
/// minted ULID's numeric beginning from teaching a spurious prefix.
fn hyphenated_suffix(chars: &[char]) -> bool {
    if chars.first() != Some(&'-') {
        return false;
    }
    let mut segment_len = 0;
    let mut saw_letter = false;
    for c in &chars[1..] {
        if *c == '-' {
            if segment_len == 0 {
                return false;
            }
            segment_len = 0;
        } else if c.is_alphanumeric() {
            segment_len += 1;
            saw_letter |= c.is_alphabetic();
        } else {
            return false;
        }
    }
    segment_len > 0 && saw_letter
}

impl Vocabulary {
    /// Learn it from the plan's ids.
    #[must_use]
    pub fn of(ids: &BTreeSet<String>) -> Self {
        let mut by_prefix: std::collections::BTreeMap<String, (bool, bool)> =
            std::collections::BTreeMap::new();
        for id in ids {
            if crate::ids::contains_a_ulid(id) {
                continue;
            }
            // By characters, never by bytes: an id is whatever the plan wrote,
            // and slicing a multi-byte character in half would panic on someone
            // else's plan rather than decline to learn from it.
            let chars: Vec<char> = id.chars().collect();
            let suffix = chars
                .windows(2)
                .enumerate()
                .filter_map(|(i, pair)| {
                    (pair[0].is_ascii_digit()
                        && pair[1] == '-'
                        && hyphenated_suffix(&chars[i + 1..]))
                    .then_some(i + 1)
                })
                .next_back();
            let numeric_end = suffix.unwrap_or(chars.len());
            let digits = numeric_end
                - chars[..numeric_end]
                    .iter()
                    .rev()
                    .take_while(|c| c.is_ascii_digit() || **c == '.')
                    .count();
            // A prefix of nothing would match every number in the document.
            if digits == 0 || digits >= numeric_end {
                continue;
            }
            let head: String = chars[..digits].iter().collect();
            // A prefix has to be a *word*, not punctuation. An id like `-42`
            // would otherwise teach `-`, and every negative number in the
            // document would read as a reference to a task.
            if !head.chars().any(char::is_alphanumeric) {
                continue;
            }
            let dotted = chars[digits..numeric_end].contains(&'.');
            let entry = by_prefix.entry(head).or_insert((false, false));
            entry.0 |= dotted;
            entry.1 |= suffix.is_some();
        }
        let mut entries: Vec<Family> = by_prefix
            .into_iter()
            .map(|(prefix, (dotted, hyphenated_suffix))| Family {
                prefix,
                dotted,
                hyphenated_suffix,
            })
            .collect();
        sort_families(&mut entries);
        Self { entries }
    }

    /// Nothing was learned, so nothing in a document can be told from an
    /// ordinary word. The caller has to refuse rather than report a clean run.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The prefixes, longest first. For tests and for a message that has to say
    /// what was learned.
    #[must_use]
    pub fn prefixes(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|family| family.prefix.as_str())
            .collect()
    }

    fn matching(&self, chars: &[char], at: usize) -> Option<(usize, bool, bool)> {
        self.entries
            .iter()
            .find(|family| {
                let prefix_len = family.prefix.chars().count();
                starts_with(chars, at, &family.prefix)
                    && chars
                        .get(at + prefix_len)
                        .is_some_and(|c| c.is_ascii_digit() || (family.dotted && *c == '.'))
            })
            .map(|family| {
                (
                    family.prefix.chars().count(),
                    family.dotted,
                    family.hyphenated_suffix,
                )
            })
    }

    /// Prefixes a digit's typo away from a taught one, each with the prefix it
    /// was meant to be (`A1-164`).
    ///
    /// **One digit, substituted.** A plan teaching `DN0-` through `DN7-` is one
    /// keystroke from `DN8-`, and a reference under it reads as tracked while
    /// naming nothing -- the mistake `A1-999` cannot be, because inventing a
    /// number nobody used takes more than a slip.
    ///
    /// Only where both characters are digits, which is what keeps this from
    /// reading foreign vocabulary as a typo: `A2-` against a plan teaching `A1-`
    /// is a near miss, and `AI-2027` in prose is not, though it differs in one
    /// position too. A letter is where another project's ids live.
    #[must_use]
    pub fn near_prefixes(&self) -> std::collections::BTreeMap<String, (bool, bool, Vec<String>)> {
        let taught: BTreeSet<&str> = self
            .entries
            .iter()
            .map(|family| family.prefix.as_str())
            .collect();
        let mut out = std::collections::BTreeMap::new();
        for family in &self.entries {
            let chars: Vec<char> = family.prefix.chars().collect();
            for (i, c) in chars.iter().enumerate() {
                if !c.is_ascii_digit() {
                    continue;
                }
                for d in '0'..='9' {
                    if d == *c {
                        continue;
                    }
                    let mut variant = chars.clone();
                    variant[i] = d;
                    let variant: String = variant.into_iter().collect();
                    if taught.contains(variant.as_str()) {
                        continue;
                    }
                    let entry = out
                        .entry(variant)
                        .or_insert_with(|| (family.dotted, family.hyphenated_suffix, Vec::new()));
                    entry.0 |= family.dotted;
                    entry.1 |= family.hyphenated_suffix;
                    if !entry.2.contains(&family.prefix) {
                        entry.2.push(family.prefix.clone());
                    }
                }
            }
        }
        out
    }
}

/// Every reference in `text` to something shaped like one of the plan's ids.
///
/// Both ends are word boundaries. The start rule keeps `xT-042` from reading as
/// a reference; the end rule is what the first measurement added, without which
/// `M-v0.N` reports a missing `M-v0` and every ULID in a document reports a
/// missing task built from its leading digits. Between them, a family always
/// has a numeric core and carries dots or single-hyphen suffix segments only
/// where the plan taught that shape.
///
/// A hyphen at the front is *not* a boundary character, which the audit of this
/// task had to fix: this project writes `--` where another would write an em
/// dash, so `the clause--T-042` is ordinary prose, and treating the dash as part
/// of the preceding word made the reference invisible. Within a suffix, only a
/// hyphen followed by a non-empty alphanumeric segment continues the id, so the
/// same doubled dash still separates two references. A dangling id hiding
/// behind punctuation is the one failure a gate like this cannot have.
#[must_use]
pub fn references(text: &str, vocabulary: &Vocabulary) -> Vec<Reference> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut line = 1;
    let mut line_start = 0;
    for i in 0..chars.len() {
        if chars[i] == '\n' {
            line += 1;
            line_start = i + 1;
            continue;
        }
        if i > 0 && continues_word(chars[i - 1]) {
            continue;
        }
        let Some((prefix_len, dotted, hyphenated)) = vocabulary.matching(&chars, i) else {
            continue;
        };
        let start = i + prefix_len;
        let mut end = start;
        while end < chars.len() && (chars[end].is_ascii_digit() || (dotted && chars[end] == '.')) {
            end += 1;
        }
        // A trailing dot is the sentence's, not the id's: `ships in M-v0.8.`
        while end > start && chars[end - 1] == '.' {
            end -= 1;
        }
        if end == start {
            continue; // The prefix with no number after it is just a word.
        }
        if hyphenated {
            while chars.get(end) == Some(&'-') {
                let mut next = end + 1;
                while chars.get(next).is_some_and(|c| c.is_alphanumeric()) {
                    next += 1;
                }
                if next == end + 1 {
                    break;
                }
                end = next;
            }
        }
        // The end boundary. A ULID continues into letters, so it is not a
        // reference to the id its leading digits spell.
        if chars.get(end).is_some_and(|c| continues_word(*c)) {
            continue;
        }
        // A dot only matters where the plan's own ids use dots. There, a dot
        // sitting against the token means the token was cut short, and the only
        // innocent reason for that is a sentence ending: `ships in M-v0.8.`
        // Anything else is a template -- `M-v0.N`, and `M-v0.{N-1}`, which this
        // task's audit found in a task body after the first rule let it through.
        // Where the plan's ids carry no dots there is nothing to cut short, and
        // `aplan/tasks/T-999.md` is a filename naming a task that must resolve.
        if dotted
            && chars.get(end) == Some(&'.')
            && chars.get(end + 1).is_some_and(|c| !c.is_whitespace())
        {
            continue;
        }
        out.push(Reference {
            mistyped_prefix: Vec::new(),
            id: chars[i..end].iter().collect(),
            line,
            column: i - line_start + 1,
        });
    }
    out
}

/// The references naming no task in the plan, in the order they appear.
///
/// Deduplicated per id and line: the same id twice on one line is one mistake,
/// and reporting it twice would be the same noise `APLAN026` avoids.
#[must_use]
pub fn dangling(text: &str, ids: &BTreeSet<String>) -> Vec<Reference> {
    let vocabulary = Vocabulary::of(ids);
    let mut seen = BTreeSet::new();
    let mut out: Vec<Reference> = references(text, &vocabulary)
        .into_iter()
        .filter(|r| !ids.contains(&r.id))
        .filter(|r| seen.insert((r.id.clone(), r.line)))
        .collect();

    // And the references whose *prefix* is the typo (`A1-164`). Scanned with a
    // vocabulary built from the near misses, so the boundary rules -- what
    // starts a word, where the digits end, a trailing dot belonging to the
    // sentence -- are the ones above rather than a second reading of them.
    let near = vocabulary.near_prefixes();
    if !near.is_empty() {
        let mut entries: Vec<Family> = near
            .iter()
            .map(|(variant, (dotted, hyphenated_suffix, _))| Family {
                prefix: variant.clone(),
                dotted: *dotted,
                hyphenated_suffix: *hyphenated_suffix,
            })
            .collect();
        sort_families(&mut entries);
        let variants = Vocabulary { entries };
        for mut r in references(text, &variants) {
            if ids.contains(&r.id) || !seen.insert((r.id.clone(), r.line)) {
                continue;
            }
            r.mistyped_prefix = near
                .iter()
                .find(|(v, _)| r.id.starts_with(v.as_str()))
                .map(|(_, (_, _, taught))| taught.clone())
                .unwrap_or_default();
            out.push(r);
        }
    }
    // Document order, which this function's first line promises and the two
    // passes above cannot produce on their own.
    out.sort_by_key(|r| (r.line, r.column));
    out
}

/// Where in a cut's section its membership is declared.
///
/// Two shapes, and both are measured rather than imagined: this repository's
/// `ROADMAP.md` declares a cut's tasks on a `**Tasks:**` line, and the pilot that
/// asked for this carries a task table per cut, one member per row (ADR-0015's
/// first cell). A third shape is not offered on the argument that a document's
/// shape means whatever its author intended (ADR-0019) -- which is also why this
/// is declared in `.aplan.yml` and not guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredBy {
    /// Every id on a line beginning with this marker, once leading whitespace is
    /// trimmed, as in a line reading `**Tasks:**` and then the cut's ids.
    Line(String),
    /// Every id in a table row's first cell.
    Table,
}

/// A cut whose two copies of its membership disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipProblem {
    /// The milestone whose `blocked_by` is one of the two copies.
    pub milestone: String,
    /// Members of the milestone the document does not declare.
    pub undeclared: Vec<String>,
    /// Ids the document declares that the milestone does not include.
    pub unplanned: Vec<String>,
    /// The section holds no declaration at all: no marked line, or no table.
    ///
    /// Reported as its own thing rather than as every member being undeclared,
    /// which is the same fact spelled as a hundred and twenty ids. A section that
    /// declares nothing for a cut that *has* nothing is not this: both copies say
    /// the same, so nothing is reported.
    pub no_declaration: bool,
    /// More than one heading in the document names this cut.
    ///
    /// Reported rather than resolved. Taking the first was the original
    /// behaviour, and the audit found what that costs: a document whose second
    /// `v0.1` section disagreed with the milestone passed, because the first one
    /// agreed. Which of two sections is *the* declaration is the author's
    /// question, and a gate that picks one is guessing on the reader's behalf.
    pub repeated_section: bool,
}

/// The ids `declared_by` marks as this section's membership, and whether the
/// section holds a declaration at all.
///
/// The two are different questions. A `**Tasks:**` line listing nothing declares
/// an empty cut; a section with no such line declares nothing, and the difference
/// is whether the document is making a claim or is silent.
fn declared_in(
    body: &str,
    declared_by: &DeclaredBy,
    vocabulary: &Vocabulary,
) -> (BTreeSet<String>, bool) {
    let lines: Vec<String> = match declared_by {
        // Fenced code is skipped, exactly as the table reader skips it: a
        // document showing what its own format looks like is not declaring
        // anything, and the audit of this task found the two readers disagreeing
        // about that -- the same sample was a declaration written as a line and
        // an example written as a table.
        DeclaredBy::Line(marker) => aplan_core::anchor::unfenced_lines(body)
            .into_iter()
            .map(|(_, l)| l)
            .filter(|l| l.trim_start().starts_with(marker.as_str()))
            .map(str::to_string)
            .collect(),
        DeclaredBy::Table => aplan_core::anchor::table_row_cells(body),
    };
    let found = !lines.is_empty();
    let ids = references(&lines.join("\n"), vocabulary)
        .into_iter()
        .map(|r| r.id)
        .collect();
    (ids, found)
}

/// The cut a heading names, if it opens one: its first word, undecorated.
///
/// A heading is prose, and its first word arrives wearing whatever the author put
/// on it. The audit of this task measured what a narrower reading costs, and the
/// answer is silence, which is the worst thing a gate can do: `### **v0.1**, ...`
/// and `### [v0.1](#x), ...` and `### v0.1: ...` are all ordinary markdown, and
/// all three went unchecked while reporting `ok`. So link targets come off with
/// the rule that mints anchors, the separator set is the punctuation a heading
/// actually uses between a label and a title, and the decoration is trimmed from
/// both ends.
///
/// Public because this repository's own narrative gate needs the same answer
/// (`A1-172`): it asks whether a `###` section is a cut, and a second reader of
/// heading decoration is a second place for `**v0.8**` and `[v0.8](#x)` to be
/// read differently from the shipped command.
#[must_use]
pub fn heading_label(title: &str) -> String {
    aplan_core::anchor::heading_text(title)
        .split([' ', '\t', ',', ':', ';'])
        .find(|w| !w.is_empty())
        .unwrap_or_default()
        .trim_matches(['`', '*', '_', '[', ']'])
        .to_string()
}

/// Whether `title` is the heading of `milestone`'s section.
///
/// Both spellings the two real documents use: the milestone's own id
/// (`` ### `M-s0` -- Repository and gates ``) and the cut label the id is built
/// from (`### v0.8, Adoption`). The heading's first word only, so a heading that
/// merely mentions a cut in passing is not its section.
fn heads_section_for(title: &str, milestone: &str) -> bool {
    let first = heading_label(title);
    first == milestone || milestone.strip_prefix("M-") == Some(first.as_str())
}

/// Every cut whose narrative and whose milestone disagree about its membership.
///
/// A plan document and a milestone are two copies of one fact, and one of them is
/// ungated until something reads both. This repository caught exactly that on
/// itself while the feature was being specified -- a task added to `M-v0.8` and
/// not to `ROADMAP.md`.
///
/// **A milestone with no section in this document is silent**, not a finding.
/// Measured before it was decided: `M-v0.1` here shipped before the document grew
/// cut sections and is described by a heading naming no cut, so reporting it
/// would make the check fail on the day an adopter turned it on, for a milestone
/// that is finished. A document declares membership for the cuts it describes.
///
/// Both directions are reported, because the copies drift both ways: a member the
/// narrative never mentions reads as unplanned work, and a declaration the
/// milestone lacks reads as a promise nobody scheduled.
#[must_use]
pub fn membership(
    text: &str,
    milestones: &std::collections::BTreeMap<String, BTreeSet<String>>,
    declared_by: &DeclaredBy,
    ids: &BTreeSet<String>,
) -> Vec<MembershipProblem> {
    let vocabulary = Vocabulary::of(ids);
    let sections = aplan_core::anchor::sections(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (milestone, all) in milestones {
        // A `blocked_by` entry that is itself a cut is that cut's *predecessor*
        // (ADR-0013), not a member of it, and the members inherit the edge. The
        // roadmap has always filtered these out; this did not, so the first cut
        // this repository sequenced asked its own narrative to declare `M-v0.8`
        // as one of the tasks in it.
        let members: &BTreeSet<String> = &all
            .iter()
            .filter(|b| !milestones.contains_key(*b))
            .cloned()
            .collect();
        let matching: Vec<&aplan_core::anchor::Section> = sections
            .iter()
            .filter(|s| heads_section_for(&s.title, milestone))
            .collect();
        let [section] = matching[..] else {
            if matching.len() > 1 {
                out.push(MembershipProblem {
                    milestone: milestone.clone(),
                    undeclared: Vec::new(),
                    unplanned: Vec::new(),
                    no_declaration: false,
                    repeated_section: true,
                });
            }
            continue;
        };
        let body = cut_body(&lines, section, &sections, milestones);
        let (declared, found) = declared_in(&body, declared_by, &vocabulary);
        if !found {
            // A cut with no members and a section that lists none agree.
            if !members.is_empty() {
                out.push(MembershipProblem {
                    milestone: milestone.clone(),
                    undeclared: Vec::new(),
                    unplanned: Vec::new(),
                    no_declaration: true,
                    repeated_section: false,
                });
            }
            continue;
        }
        let undeclared: Vec<String> = members.difference(&declared).cloned().collect();
        let unplanned: Vec<String> = declared.difference(members).cloned().collect();
        if !undeclared.is_empty() || !unplanned.is_empty() {
            out.push(MembershipProblem {
                milestone: milestone.clone(),
                undeclared,
                unplanned,
                no_declaration: false,
                repeated_section: false,
            });
        }
    }
    out
}

/// A cut's section, stopping where another cut's section begins.
///
/// `sections` gives a heading everything down to the next heading that is not
/// deeper, which includes its subsections -- right for a cut whose detail lives
/// under it, and wrong for a document that nests one cut's heading inside
/// another's. The audit found the cost: the outer cut read the inner one's
/// declaration as its own and reported the inner cut's tasks as work it had never
/// heard of. ADR-0014 says a plan has one task-membership level, so this is a
/// document this tool did not expect rather than one it should refuse; taking
/// the narrower body reads it the way its author plainly meant.
fn cut_body(
    lines: &[&str],
    section: &aplan_core::anchor::Section,
    sections: &[aplan_core::anchor::Section],
    milestones: &std::collections::BTreeMap<String, BTreeSet<String>>,
) -> String {
    let start = section.line + 1;
    let end = sections
        .iter()
        .filter(|s| s.line > section.line && s.line < section.end)
        .filter(|s| milestones.keys().any(|m| heads_section_for(&s.title, m)))
        .map(|s| s.line)
        .min()
        .unwrap_or(section.end);
    lines[start.min(end)..end].join("\n")
}

/// The ids a dangling reference could have meant: those a single edit away.
///
/// One edit, judged by the helper `APLAN026` uses for a mistyped tag, so "near"
/// means one thing in this codebase. **Every** near id is returned rather than
/// the first, because `A1-041` is one edit from `A1-042` and from `A1-141`
/// alike, and picking one of those to print reads as knowledge rather than as
/// the coin flip it is. A task that never existed has no near match and gets no
/// invented suggestion.
#[must_use]
pub fn nearest<'a>(id: &str, ids: &'a BTreeSet<String>) -> Vec<&'a str> {
    ids.iter()
        .filter(|known| aplan_core::is_one_edit_apart(known, id))
        .map(String::as_str)
        .collect()
}

fn starts_with(chars: &[char], at: usize, prefix: &str) -> bool {
    prefix
        .chars()
        .enumerate()
        .all(|(offset, c)| chars.get(at + offset) == Some(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn prefixes_come_from_the_plans_own_ids() {
        let v = Vocabulary::of(&ids(&[
            "A1-042",
            "M-v0.8",
            "M-phase-2",
            "WH2-06",
            "K0-3",
            "ALR-01",
        ]));
        // Longest first, so `M-phase-` and `M-v` are tried before anything they
        // would both match.
        assert_eq!(
            v.prefixes(),
            vec!["M-phase-", "ALR-", "WH2-", "A1-", "K0-", "M-v"]
        );
    }

    /// Whether a dot continues an id is the plan's answer, not this crate's
    /// (found by this task's audit). The same three characters -- digits, dot,
    /// letters -- are a prose template under one prefix and a filename under
    /// another.
    #[test]
    fn a_dot_continues_an_id_only_where_the_plan_says_so() {
        // `M-v0.8` teaches dots: `M-v0.N` is prose, not a reference.
        let dotted = ids(&["M-v0.8", "A1-042"]);
        assert!(dangling("each cut is `M-v0.N`", &dotted).is_empty());
        // The same template with arithmetic in it, which a task body writes when
        // it means "the cut before this one".
        assert!(dangling("no `M-v0.N` lists `M-v0.{N-1}`", &dotted).is_empty());
        // And the sentence that really does end on a reference.
        assert_eq!(dangling("this ships in M-v0.9.", &dotted).len(), 1);
        // `A1-042` teaches none, so the dot ends the id and the filename names
        // a task -- one that does not exist, which is the whole point.
        let out = dangling("see `aplan/tasks/A1-999.md`", &dotted);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "A1-999");
        // And the file of a task that does exist is silent.
        assert!(dangling("see `aplan/tasks/A1-042.md`", &dotted).is_empty());
    }

    /// An id is whatever the plan wrote, and this must decline rather than
    /// panic on one it did not expect.
    #[test]
    fn an_id_carrying_a_multibyte_character_is_read_by_characters() {
        let v = Vocabulary::of(&ids(&["\u{00c5}L1-07", "task\u{2014}9"]));
        assert_eq!(
            v.prefixes(),
            vec!["task\u{2014}", "\u{00c5}L1-"],
            "no panic"
        );
    }

    /// A plan that teaches no prefix teaches nothing, which is the safe answer:
    /// a ULID ends in a letter as often as a digit, and a prefix taken from one
    /// would match a different document every time.

    #[test]
    fn an_id_that_is_not_prefix_plus_number_contributes_nothing() {
        assert!(Vocabulary::of(&ids(&["A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD"])).is_empty());
        assert!(Vocabulary::of(&ids(&["42"])).is_empty());
    }

    /// The two false positives the measurement found, in the words the documents
    /// wrote them (`A1-148`).
    #[test]
    fn a_reference_ends_at_a_word_boundary() {
        let known = ids(&["A1-047", "M-v0.8"]);
        assert!(
            dangling("each cut is `M-v0.N`", &known).is_empty(),
            "`M-v0.N` is how the prose writes \"a milestone\""
        );
        assert!(
            dangling("`A1-047` becomes `A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD`", &known).is_empty(),
            "the ULID example is not a reference to `A1-01`"
        );
        // And the boundary it always had, at the other end.
        assert!(dangling("xA1-999 is not a reference", &known).is_empty());
    }

    /// Proof for `check_docs.identifier_boundaries_are_symmetric`: a token
    /// which continues with a word character is not the shorter known or
    /// unknown id on either side. This exercises every scanner mode, including
    /// the near-prefix pass, because they deliberately share one boundary.
    #[test]
    fn identifier_word_boundaries_are_symmetric_across_every_family() {
        let known = ids(&["A1-557", "M-v0.8", "M-p0-design"]);
        let joined = "prefix_A1-999 A1-999_suffix A1-557_suffix \
                      M-v0.8_suffix M-p0-design_suffix A2-999_suffix";

        assert!(
            references(joined, &Vocabulary::of(&known)).is_empty(),
            "no joined token is a standalone reference"
        );
        assert!(
            dangling(joined, &known).is_empty(),
            "near-prefix pass agrees"
        );

        assert_eq!(
            dangling(
                "Punctuation: A1-999.md, M-v0.9; M-p0-review--A1-557.",
                &known
            )
            .into_iter()
            .map(|reference| reference.id)
            .collect::<Vec<_>>(),
            vec!["A1-999", "M-v0.9", "M-p0-review"],
            "punctuation, filenames and doubled hyphens remain boundaries"
        );
    }

    /// This project's own em dash is two hyphens, so a reference can arrive with
    /// one glued to its front. Found by the audit: the id was silently skipped,
    /// which is a dangling reference hiding behind punctuation.
    #[test]
    fn a_dash_in_front_does_not_hide_a_reference() {
        let known = ids(&["A1-042"]);
        assert_eq!(dangling("the clause--A1-999 and on", &known).len(), 1);
        assert_eq!(dangling("a range A1-999--A1-042", &known).len(), 1);
        // The boundary that still holds: a reference does not continue a word.
        assert!(dangling("xA1-999", &known).is_empty());
    }

    /// Proof for `check_docs.complete_multi_hyphen_ids`: once the plan teaches
    /// a numeric family with hyphenated suffixes, the complete token is the
    /// reference. Known ids resolve and an unknown sibling is reported under
    /// the spelling the document actually used, rather than as its numeric
    /// prefix (`A1-557`, P00024 pilot item 6).
    #[test]
    fn complete_multi_hyphen_ids_are_resolved_and_reported_without_truncation() {
        let known = ids(&["M-p0-design", "M-p0-build", "M-p1-read", "A1-557", "M-v0.1"]);
        let text = "`M-p0-design`, M-p0-build; M-p1-read. Unknown: `M-p0-review`. \
                    Ordinary: `A1-557` and `M-v0.1`.";

        assert_eq!(
            references(text, &Vocabulary::of(&known))
                .into_iter()
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec![
                "M-p0-design",
                "M-p0-build",
                "M-p1-read",
                "M-p0-review",
                "A1-557",
                "M-v0.1"
            ]
        );
        assert_eq!(
            dangling(text, &known)
                .into_iter()
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec!["M-p0-review"],
            "the finding names the complete unknown token"
        );
    }

    /// The suffix boundary is the hyphen after the numeric core, not an earlier
    /// digit that happens to be part of the prefix. Picking the first eligible
    /// hyphen in `A1-042-patch` teaches `A`, causing ordinary prose beginning
    /// with that letter to become reference candidates.
    #[test]
    fn a_hyphenated_suffix_keeps_digits_that_belong_to_the_prefix() {
        let known = ids(&["A1-042-patch", "A1-057-2fa"]);
        let vocabulary = Vocabulary::of(&known);
        assert_eq!(vocabulary.prefixes(), vec!["A1-"]);
        assert_eq!(
            references(
                "`A1-042-patch`--`A1-057-2fa`; `A1-999-missing`.",
                &vocabulary
            )
            .into_iter()
            .map(|reference| reference.id)
            .collect::<Vec<_>>(),
            vec!["A1-042-patch", "A1-057-2fa", "A1-999-missing"]
        );
    }

    /// Longest-prefix selection applies only among families that can consume
    /// the numeric core at this position. A numeric subfamily and a named cut
    /// family may legitimately overlap; the former must not mask the latter.
    #[test]
    fn an_overlapping_numeric_family_does_not_mask_a_hyphenated_suffix() {
        let known = ids(&["M-p0-design", "M-p0-001"]);
        assert_eq!(
            references(
                "Known: M-p0-design and M-p0-001; missing: M-p0-review.",
                &Vocabulary::of(&known)
            )
            .into_iter()
            .map(|reference| reference.id)
            .collect::<Vec<_>>(),
            vec!["M-p0-design", "M-p0-001", "M-p0-review"]
        );
    }

    /// Selecting only a family that can consume the next character must not
    /// narrow the older dotted-family behavior. A plan may teach `.8` as its
    /// numeric core just as it may teach `0.8`; both were accepted before
    /// suffix-bearing families existed.
    #[test]
    fn an_overlap_guard_preserves_a_dotted_core_that_begins_with_a_dot() {
        let known = ids(&["M-v.8", "M-v0-design"]);
        assert!(dangling("Known: M-v.8 and M-v0-design.", &known).is_empty());
        assert_eq!(
            dangling("Missing: M-v.9 and M-v0-review.", &known)
                .into_iter()
                .map(|reference| reference.id)
                .collect::<Vec<_>>(),
            vec!["M-v.9", "M-v0-review"]
        );
    }

    /// Proof for `check_docs.near_prefixes_prefer_longest_family`: the typo
    /// vocabulary is derived after the primary one, but it still has the same
    /// longest-prefix contract. Here `A1-` and `A11-` are both near variants;
    /// taking lexical order would report `A1-1` for `A11-999` and recommend the
    /// wrong family.
    #[test]
    fn overlapping_near_prefixes_are_matched_longest_first() {
        let known = ids(&["A2-001", "A01-001"]);
        let dangling = dangling("Mistyped: A11-999.", &known);
        assert_eq!(dangling.len(), 1);
        assert_eq!(dangling[0].id, "A11-999");
        assert_eq!(dangling[0].mistyped_prefix, vec!["A01-"]);
    }

    /// A plan can teach a prefix that is not a word, and then every number in
    /// the document reads as a reference. It teaches nothing instead.
    #[test]
    fn a_prefix_of_punctuation_is_not_learned() {
        assert!(Vocabulary::of(&ids(&["-42", ".5"])).is_empty());
        // And a plan that also has real ids is unaffected by the odd one.
        assert_eq!(
            Vocabulary::of(&ids(&["-42", "T-01"])).prefixes(),
            vec!["T-"]
        );
        assert!(dangling("a budget of -50 and -42 here", &ids(&["-42", "T-01"])).is_empty());
    }

    #[test]
    fn a_reference_to_no_task_is_reported_with_its_line() {
        let known = ids(&["A1-042", "M-v0.8"]);
        let out = dangling(
            "first line\ntracked as `A1-099`.\nand M-v0.8 ships it.\n",
            &known,
        );
        assert_eq!(
            out,
            vec![Reference {
                mistyped_prefix: Vec::new(),
                id: "A1-099".to_string(),
                line: 2,
                column: 13
            }]
        );
    }

    /// A trailing full stop belongs to the sentence. The same bug the xtask
    /// prototype had to learn: `M-v0.8.` matched nothing, so a dangling
    /// reference could hide behind punctuation.
    #[test]
    fn punctuation_does_not_hide_a_reference() {
        let known = ids(&["M-v0.8"]);
        assert_eq!(dangling("this ships in M-v0.9.", &known).len(), 1);
        assert!(dangling("this ships in M-v0.8.", &known).is_empty());
    }

    #[test]
    fn the_same_id_twice_on_a_line_is_one_finding() {
        let known = ids(&["A1-042"]);
        let out = dangling("`A1-099` blocks `A1-099`\n`A1-099` again\n", &known);
        assert_eq!(out.len(), 2, "once per line, not once per mention");
    }

    #[test]
    fn a_typo_gets_every_candidate_and_an_invention_gets_none() {
        let known = ids(&["A1-141", "A1-042", "A1-140"]);
        // `A1-041` is one edit from two of these. Naming both is the honest
        // answer; naming the first would read as knowledge.
        assert_eq!(nearest("A1-041", &known), vec!["A1-042", "A1-141"]);
        assert_eq!(
            nearest("A1-142", &known),
            vec!["A1-042", "A1-140", "A1-141"]
        );
        assert!(nearest("A1-999", &known).is_empty(), "nothing is invented");
    }
    /// The pilot's case: eight cuts `DN0-` through `DN7-`, so `DN8-01` is one
    /// keystroke away and reads as tracked while naming nothing (`A1-164`).
    #[test]
    fn a_prefix_one_digit_from_a_taught_one_is_a_dangling_reference() {
        let plan = ids(&["DN0-01", "DN7-04"]);
        let found = dangling("Tracked as `DN8-01`, which nobody defined.", &plan);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].id, "DN8-01");
        assert_eq!(
            found[0].mistyped_prefix,
            vec!["DN0-".to_string(), "DN7-".to_string()],
            "every near prefix, because picking one reads as knowledge"
        );
    }

    /// Both kinds of finding on one line, the near miss written first.
    ///
    /// `dangling` promises document order, and until this task's audit it sorted
    /// on `(line, id)`, so the near miss below sorted *after* the plain dangling
    /// reference that appears eleven columns to its right. Alphabetical order
    /// looks like document order in every example where the ids happen to
    /// ascend, which is why this fixture makes them descend.
    #[test]
    fn two_findings_on_one_line_are_reported_left_to_right() {
        let plan = ids(&["A1-042"]);
        let found = dangling("see `A2-001` and `A1-999` both.", &plan);
        assert_eq!(
            found.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["A2-001", "A1-999"],
            "the order they appear, not the order they sort"
        );
        assert_eq!((found[0].column, found[1].column), (6, 19));
    }

    fn cut(pairs: &[(&str, &[&str])]) -> std::collections::BTreeMap<String, BTreeSet<String>> {
        pairs
            .iter()
            .map(|(m, ids)| {
                (
                    (*m).to_string(),
                    ids.iter().map(|s| (*s).to_string()).collect(),
                )
            })
            .collect()
    }

    /// A cut's two copies of its membership, disagreeing in each direction.
    ///
    /// The `A1-164` case is the real one and is the first assertion: a task added
    /// to the milestone and to no narrative. It went in and out of this
    /// repository's plan while nothing but `cargo xtask` could see it.
    #[test]
    fn a_cut_whose_two_copies_disagree_says_which_way() {
        let plan = ids(&["A1-042", "A1-043", "A1-044", "M-v0.8"]);
        let doc = "### v0.8, Adoption\n\n**Tasks:** `A1-042`, `A1-043`\n";
        let declared = DeclaredBy::Line("**Tasks:**".to_string());

        // A member the narrative omits.
        let out = membership(
            doc,
            &cut(&[("M-v0.8", &["A1-042", "A1-043", "A1-044"])]),
            &declared,
            &plan,
        );
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].undeclared, vec!["A1-044".to_string()]);
        assert!(out[0].unplanned.is_empty());

        // A declaration the milestone does not include.
        let out = membership(doc, &cut(&[("M-v0.8", &["A1-042"])]), &declared, &plan);
        assert_eq!(out[0].unplanned, vec!["A1-043".to_string()]);
        assert!(out[0].undeclared.is_empty());

        // And agreement is silence.
        assert!(
            membership(
                doc,
                &cut(&[("M-v0.8", &["A1-042", "A1-043"])]),
                &declared,
                &plan
            )
            .is_empty()
        );
    }

    /// The distinction the whole feature rests on: prose in a cut's section may
    /// name another cut's task, and only the declaration says membership.
    ///
    /// Measured before it was built, because the alternative is tempting and
    /// wrong: reading the whole section instead calls six of the pilot's ordinary
    /// cross-references false declarations, and eight of this repository's.
    #[test]
    fn a_mention_in_the_section_is_not_a_declaration() {
        let plan = ids(&["A1-042", "A1-090", "M-v0.8"]);
        let doc = "### v0.8, Adoption\n\n**Tasks:** `A1-042`\n\nCompare `A1-090`, a later cut.\n";
        assert!(
            membership(
                doc,
                &cut(&[("M-v0.8", &["A1-042"])]),
                &DeclaredBy::Line("**Tasks:**".to_string()),
                &plan
            )
            .is_empty(),
            "a mention outside the declaration is not membership"
        );
    }

    /// The pilot's shape: a task table per cut, the member in the first cell and
    /// prose in the second (ADR-0015).
    #[test]
    fn a_table_declares_its_members_in_the_first_cell() {
        let plan = ids(&["DN0-01", "DN0-02", "DN1-05", "M-s0"]);
        let doc = "### `M-s0` -- Repository and gates\n\n| Task | Deliverable |\n|---|---|\n| `DN0-01` | Repo skeleton (licences wait for `DN1-05`) |\n| `DN0-02` | Toolchain pin |\n";
        assert!(
            membership(
                doc,
                &cut(&[("M-s0", &["DN0-01", "DN0-02"])]),
                &DeclaredBy::Table,
                &plan
            )
            .is_empty(),
            "the second cell is prose, and `DN1-05` belongs to another cut"
        );
        let out = membership(
            doc,
            &cut(&[("M-s0", &["DN0-01"])]),
            &DeclaredBy::Table,
            &plan,
        );
        assert_eq!(out[0].unplanned, vec!["DN0-02".to_string()]);
    }

    /// Every decoration a heading's first word arrives wearing.
    ///
    /// The audit's sharpest finding, because the failure was *silence*: the first
    /// version trimmed backticks and split on space and comma, so `**v0.1**`,
    /// `[v0.1](#x)` and `v0.1:` were all sections it never found, and the run
    /// said `ok` over a cut it had not looked at. Each of these is ordinary
    /// markdown that an adopter's plan document could open with.
    #[test]
    fn a_heading_is_found_through_its_decoration() {
        let plan = ids(&["A1-042", "A1-043", "M-v0.8"]);
        let declared = DeclaredBy::Line("**Tasks:**".to_string());
        for heading in [
            "### v0.8, Adoption",
            "### **v0.8**, Adoption",
            "### _v0.8_ Adoption",
            "### `v0.8`, Adoption",
            "### [v0.8](#v08), Adoption",
            "### v0.8: Adoption",
            "### v0.8 -- Adoption",
            "### `M-v0.8` -- Adoption",
        ] {
            let doc = format!("{heading}\n\n**Tasks:** `A1-042`\n");
            let out = membership(
                &doc,
                &cut(&[("M-v0.8", &["A1-042", "A1-043"])]),
                &declared,
                &plan,
            );
            assert_eq!(
                out.len(),
                1,
                "`{heading}` was not read as the cut's section, so nothing was checked"
            );
            assert_eq!(out[0].undeclared, vec!["A1-043".to_string()]);
        }
    }

    /// A document's example of its own format is not a declaration.
    ///
    /// The two readers disagreed about this: `table_row_cells` has skipped fenced
    /// code since it was written, and the marked-line reader did not, so the same
    /// sample was an example as a table and a declaration as a line.
    #[test]
    fn a_declaration_inside_a_code_fence_is_an_example() {
        let plan = ids(&["A1-042", "M-v0.8"]);
        let fenced = "### v0.8, Adoption\n\n**Tasks:** `A1-042`\n\nQuoted from the guide:\n\n```console\n**Tasks:** `A1-099`\n```\n";
        assert!(
            membership(
                fenced,
                &cut(&[("M-v0.8", &["A1-042"])]),
                &DeclaredBy::Line("**Tasks:**".to_string()),
                &plan
            )
            .is_empty(),
            "a fenced sample declared `A1-099`"
        );
        // The same sample as a table, which was always read this way.
        let fenced_table =
            "### v0.8, Adoption\n\n| Task |\n|---|\n| `A1-042` |\n\n```\n| `A1-099` |\n```\n";
        assert!(
            membership(
                fenced_table,
                &cut(&[("M-v0.8", &["A1-042"])]),
                &DeclaredBy::Table,
                &plan
            )
            .is_empty()
        );
    }

    /// Two headings naming one cut is the author's ambiguity, not the tool's to
    /// resolve. Taking the first passed a document whose second section was wrong.
    #[test]
    fn two_headings_for_one_cut_are_reported_rather_than_resolved() {
        let plan = ids(&["A1-042", "A1-043", "M-v0.8"]);
        let doc = "### v0.8, plan\n\n**Tasks:** `A1-042`, `A1-043`\n\n### v0.8, retrospective\n\n**Tasks:** `A1-042`\n";
        let out = membership(
            doc,
            &cut(&[("M-v0.8", &["A1-042", "A1-043"])]),
            &DeclaredBy::Line("**Tasks:**".to_string()),
            &plan,
        );
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].repeated_section, "{out:?}");
        assert!(out[0].undeclared.is_empty(), "the ambiguity is the finding");
    }

    /// A cut's section stops where another cut's begins.
    ///
    /// `sections` hands a heading its subsections too, which is right until one of
    /// them is another cut: the outer one then read the inner one's declaration as
    /// its own and reported the inner cut's tasks as work nobody planned.
    #[test]
    fn a_cut_section_does_not_swallow_a_nested_cut() {
        let plan = ids(&["A1-042", "A1-043", "M-v0.8", "M-v0.9"]);
        let doc = "### v0.8, Adoption\n\n**Tasks:** `A1-042`\n\n#### v0.9, nested under it\n\n**Tasks:** `A1-043`\n";
        let out = membership(
            doc,
            &cut(&[("M-v0.8", &["A1-042"]), ("M-v0.9", &["A1-043"])]),
            &DeclaredBy::Line("**Tasks:**".to_string()),
            &plan,
        );
        assert!(out.is_empty(), "{out:?}");
    }

    /// A cut that follows another is not a member of it, in either direction.
    ///
    /// ADR-0013 sequences two cuts by naming one in the other's `blocked_by`, and
    /// the roadmap has always filtered such an entry out of membership. This did
    /// not, and the first cut this repository sequenced found it: `M-public`
    /// follows `M-v0.8`, and the check asked `ROADMAP.md` to declare `M-v0.8`
    /// among the tasks in the public cut.
    #[test]
    fn a_cuts_predecessor_is_not_one_of_its_members() {
        let plan = ids(&["A1-042", "M-v0.8", "M-public"]);
        let doc = "### public, Going public\n\n**Tasks:** `A1-042`\n";
        let out = membership(
            doc,
            &cut(&[("M-public", &["A1-042", "M-v0.8"]), ("M-v0.8", &[])]),
            &DeclaredBy::Line("**Tasks:**".to_string()),
            &plan,
        );
        assert!(
            out.is_empty(),
            "a predecessor cut was read as a member: {out:?}"
        );
    }

    /// A milestone the document says nothing about is not a finding.    /// A milestone the document says nothing about is not a finding.
    ///
    /// `M-v0.1` here is exactly that: it shipped before `ROADMAP.md` grew cut
    /// sections, and is described by a heading naming no cut. Reporting it would
    /// make this check fail on the day an adopter turned it on, over a cut that is
    /// finished.
    #[test]
    fn a_milestone_with_no_section_is_silent() {
        let plan = ids(&["A0-001", "M-v0.1", "A1-042", "M-v0.8"]);
        let doc = "## 2. Where we are, v0.1 (shipped)\n\nNarrative.\n\n### v0.8, Adoption\n\n**Tasks:** `A1-042`\n";
        let out = membership(
            doc,
            &cut(&[("M-v0.1", &["A0-001"]), ("M-v0.8", &["A1-042"])]),
            &DeclaredBy::Line("**Tasks:**".to_string()),
            &plan,
        );
        assert!(out.is_empty(), "{out:?}");
    }

    /// A section that holds no declaration at all says so once, rather than
    /// reporting every member of the cut as undeclared.
    #[test]
    fn a_section_declaring_nothing_is_its_own_finding() {
        let plan = ids(&["A1-042", "A1-043", "M-v0.8"]);
        let doc = "### v0.8, Adoption\n\nProse, and no declaration.\n";
        let declared = DeclaredBy::Line("**Tasks:**".to_string());
        let out = membership(
            doc,
            &cut(&[("M-v0.8", &["A1-042", "A1-043"])]),
            &declared,
            &plan,
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].no_declaration, "{out:?}");
        assert!(
            out[0].undeclared.is_empty(),
            "the same fact spelled as every member is noise, not detail"
        );
        // A cut with nothing in it and a section listing nothing agree.
        assert!(membership(doc, &cut(&[("M-v0.8", &[])]), &declared, &plan).is_empty());
    }

    /// The heading has to *be* the cut's, not mention it.
    #[test]
    fn only_the_headings_first_token_names_the_section() {
        let plan = ids(&["A1-042", "M-v0.8"]);
        let mentions = "### Why v0.8 slipped\n\nProse about the cut.\n";
        assert!(
            membership(
                mentions,
                &cut(&[("M-v0.8", &["A1-042"])]),
                &DeclaredBy::Line("**Tasks:**".to_string()),
                &plan
            )
            .is_empty(),
            "a heading mentioning a cut is not the cut's section"
        );
    }

    /// A letter is where another project's ids live, so a prefix differing there    /// A letter is where another project's ids live, so a prefix differing there
    /// is foreign vocabulary rather than a typo. Without this the gate reads
    /// `AI-2027` in prose as a mistyped `A1-` task.
    #[test]
    fn a_prefix_differing_in_a_letter_is_not_a_near_miss() {
        let plan = ids(&["A1-042"]);
        let text = "Prose about `AI-2027`, `ADR-0012`, `Zebra-01` and `B1-042`.";
        assert!(
            dangling(text, &plan).is_empty(),
            "{:?}",
            dangling(text, &plan)
        );
    }

    /// A taught prefix is never its own near miss: `A0-` and `A1-` both exist
    /// here, and neither may be reported as a typo of the other.
    #[test]
    fn a_taught_prefix_is_not_reported_as_a_typo() {
        let plan = ids(&["A0-001", "A1-042"]);
        let near = Vocabulary::of(&plan).near_prefixes();
        assert!(!near.contains_key("A0-"), "{near:?}");
        assert!(!near.contains_key("A1-"), "{near:?}");
        assert!(near.contains_key("A2-"), "{near:?}");
        // And a real id under a taught prefix is still not a finding.
        assert!(dangling("`A0-001` and `A1-042`.", &plan).is_empty());
    }

    /// The ordinary case is unchanged: a taught prefix with a number nobody
    /// used is still a dangling reference, and carries no prefix advice.
    #[test]
    fn a_taught_prefix_with_an_unknown_number_reports_as_before() {
        let plan = ids(&["A1-042"]);
        let found = dangling("Tracked as `A1-999`.", &plan);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "A1-999");
        assert!(found[0].mistyped_prefix.is_empty(), "{found:?}");
    }

    /// A plan whose prefixes carry no digit teaches no near miss at all, which
    /// is the common shape (`T-`, `RD-`) and must stay silent rather than
    /// inventing variants of the letters.
    #[test]
    fn a_prefix_with_no_digit_has_no_near_miss() {
        let plan = ids(&["T-042", "M-v0.8"]);
        assert!(
            Vocabulary::of(&plan).near_prefixes().is_empty(),
            "{:?}",
            Vocabulary::of(&plan).near_prefixes()
        );
        // `T-999` and `M-v9.9` are ordinary dangling references under taught
        // prefixes; `Q-042` is foreign vocabulary and stays silent.
        let found = dangling("`T-999` and `Q-042` and `M-v9.9`.", &plan);
        let ids: Vec<&str> = found.iter().map(|r| r.id.as_str()).collect();
        // Left to right, which is not alphabetical: this assertion read
        // `["M-v9.9", "T-999"]` until the audit, and that it passed is the
        // clearest evidence the reordering was invisible.
        assert_eq!(ids, vec!["T-999", "M-v9.9"], "{ids:?}");
    }
}
