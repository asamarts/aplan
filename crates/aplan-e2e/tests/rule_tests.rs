//! Every rule is unit-tested in the crate that raises it (`A1-046`).
//!
//! `coverage.rs` gates the end-to-end half: each rule needs a plan that trips it
//! and a near-miss that does not. That pair is necessary and it is not
//! sufficient. `APLAN023` shipped with both, executed every line of its rule, and
//! was wrong on three inputs next to the one it was written for: a blank `paths`
//! entry silenced it, its message said "the commit and file exists", and its
//! `agent_instruction` promised a task citing only a `file` that `verify` would
//! check the cited *commit*. A scenario pair proves a rule fires somewhere; it
//! says nothing about the neighbours.
//!
//! So this audit asks a different question: does the crate that *emits* a
//! diagnostic have a test that names it? That is where the cheap cases live, the
//! ones where you vary one input at a time without materialising a git repo.
//! `APLAN023` would have failed this audit on the day it shipped.
//!
//! **What it cannot do.** It counts a mention, not an assertion, so a test that
//! names a code and checks nothing satisfies it. No source-text gate can do
//! better; `AGENTS.md` carries the part that matters, and a reviewer enforces it.
//! The audit's job is narrower: make "I only wrote the happy path" impossible to
//! do by accident.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `crates/aplan-e2e/..` -> the workspace root.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("locating the workspace root")
        .to_path_buf()
}

/// Every `.rs` file in the workspace's crates, excluding this crate: the
/// end-to-end corpus is gated by `coverage.rs`, and letting it satisfy this
/// audit would make the two checks one check.
fn sources(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.join("crates"), root.join("xtask")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if name != "target" && name != "aplan-e2e" {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The crate a source file belongs to.
fn crate_of(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    match parts.next()?.as_str() {
        "crates" => parts.next(),
        "xtask" => Some("xtask".to_string()),
        _ => None,
    }
}

/// Split a source file into (production, test) text.
///
/// A file under `tests/` is all test; otherwise the split is at `#[cfg(test)]`.
/// `diagnostic.rs` is deliberately contributed to neither side: it *declares*
/// every rule, so its production half would name every code as an emitter and
/// its registry guard, which asserts the whole list by construction, would
/// satisfy this audit for every rule at once.
fn split(path: &Path) -> (String, String) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (String::new(), String::new());
    };
    if path.file_name().is_some_and(|n| n == "diagnostic.rs") {
        return (String::new(), String::new());
    }
    if path.components().any(|c| c.as_os_str() == "tests") {
        return (String::new(), text);
    }
    match text.find("#[cfg(test)]") {
        Some(i) => (text[..i].to_string(), text[i..].to_string()),
        None => (text, String::new()),
    }
}

#[test]
fn every_rule_is_unit_tested_in_the_crate_that_raises_it() {
    let root = workspace_root();
    let mut production: BTreeMap<String, String> = BTreeMap::new();
    let mut tests: BTreeMap<String, String> = BTreeMap::new();
    for path in sources(&root) {
        let Some(krate) = crate_of(&path, &root) else {
            continue;
        };
        let (prod, test) = split(&path);
        production.entry(krate.clone()).or_default().push_str(&prod);
        tests.entry(krate).or_default().push_str(&test);
    }

    let constants = rule_constants(&root);
    let mut missing: Vec<String> = Vec::new();
    let mut unemitted: Vec<&str> = Vec::new();
    for rule in aplan_core::Rule::ALL {
        let konst = constants
            .get(rule.code())
            .unwrap_or_else(|| {
                panic!(
                    "{} is in Rule::ALL but not in the registry table",
                    rule.code()
                )
            })
            .clone();
        let emitters: Vec<&String> = production
            .iter()
            .filter(|(_, text)| mentions(&raising_only(text), &konst))
            .map(|(name, _)| name)
            .collect();
        if emitters.is_empty() {
            unemitted.push(rule.code());
            continue;
        }
        for krate in emitters {
            if !mentions(tests.get(krate).map_or("", String::as_str), &konst) {
                missing.push(format!(
                    "{}: raised in `{krate}` with no test there naming `{konst}`",
                    rule.code()
                ));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "every rule needs a unit test in the crate that raises it, not only the end-to-end \
         scenario pair in coverage.rs (A1-046):\n  {}",
        missing.join("\n  ")
    );
    assert!(
        unemitted.is_empty(),
        "these rules are in Rule::ALL but nothing outside diagnostic.rs raises them, so they \
         cannot fire: {unemitted:?}"
    );
}

/// Whether `text` names `konst` as a whole word, so `WEAK_EVIDENCE` is not
/// matched by a hypothetical `NOT_WEAK_EVIDENCE`.
/// The production text with *comparisons* against a rule code removed, so that
/// reading a code is not mistaken for raising one (`A1-178`).
///
/// The question this audit asks is which crate *raises* a rule, because that is
/// the crate whose unit tests owe it a case. A crate may also read a code --
/// `attest` counts `APLAN037` findings to qualify what an execution proved --
/// and demanding a unit test there would be demanding a test of somebody else's
/// rule, in a crate that cannot produce it.
///
/// Only the comparison idiom is removed, and only the whole of it: anything that
/// *constructs* a diagnostic still counts, so this can make the audit narrower
/// in exactly one shape and cannot make it blind to a raiser. `attest`'s filter
/// was the first read-only mention this repository had, which is why the
/// distinction had never been needed.
fn raising_only(text: &str) -> String {
    // A code named as a *table key* is the second read-only idiom, after the
    // comparison below. `vacuity` reports which rules a configuration switched
    // off, and the crate that assembles that table names their codes without
    // raising any of them (`A1-274`). Demanding a unit test there would be
    // demanding a test of a rule the crate cannot produce -- the same argument
    // the comparison stripping already makes.
    //
    // Narrow on purpose: only `insert(code::X,` counts, so a code passed to
    // anything that builds a diagnostic still marks the crate as raising it.
    let text: String = text
        .split("insert(")
        .enumerate()
        .map(|(n, part)| {
            if n == 0 {
                return part.to_string();
            }
            match part.split_once(',') {
                Some((first, rest)) if first.contains("code::") => format!(" ,{rest}"),
                _ => format!("insert({part}"),
            }
        })
        .collect();
    let mut chars: Vec<char> = text.chars().collect();
    // Both operators and both sides: `d.code == code::X` and `code::X != d.code`
    // are one idiom written four ways, and a reader who hit the spelling this
    // did not cover would be told to unit-test a rule their crate cannot raise.
    let is_token = |c: char| c.is_alphanumeric() || c == '_' || c == ':';
    let mut i = 0;
    while i + 1 < chars.len() {
        let operator = (chars[i] == '=' || chars[i] == '!') && chars[i + 1] == '=';
        if !operator {
            i += 1;
            continue;
        }
        // Backwards over the left operand, and forwards over the right. Blanked
        // rather than removed, so every other offset in the text is unchanged
        // and nothing downstream has to know this ran.
        let mut back = i;
        while back > 0 && chars[back - 1].is_whitespace() {
            back -= 1;
        }
        while back > 0 && is_token(chars[back - 1]) {
            back -= 1;
            chars[back] = ' ';
        }
        let mut forward = i + 2;
        while forward < chars.len() && chars[forward].is_whitespace() {
            forward += 1;
        }
        while forward < chars.len() && is_token(chars[forward]) {
            chars[forward] = ' ';
            forward += 1;
        }
        i = forward.max(i + 2);
    }
    chars.into_iter().collect()
}

/// Whether `text` names `konst` as a whole word, so `WEAK_EVIDENCE` is not
/// matched by a hypothetical `NOT_WEAK_EVIDENCE`.
/// The production text with *comparisons* against a rule code removed, so that
/// reading a code is not mistaken for raising one (`A1-178`).
///
/// The question this audit asks is which crate *raises* a rule, because that is
/// the crate whose unit tests owe it a case. A crate may also read a code --
/// `attest` counts `APLAN037` findings to qualify what an execution proved --
/// and demanding a unit test there would be demanding a test of somebody else's
/// rule, in a crate that cannot produce it.
///
/// Only the comparison idiom is removed, and only the whole of it: anything that
/// *constructs* a diagnostic still counts, so this can make the audit narrower
/// in exactly one shape and cannot make it blind to a raiser. `attest`'s filter
/// was the first read-only mention this repository had, which is why the
/// distinction had never been needed.
fn mentions(text: &str, konst: &str) -> bool {
    text.match_indices(konst).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + konst.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// Map each rule code to the `code::` constant it is raised through, read from
/// the `define_rules!` table itself.
///
/// Parsed rather than hand-listed: the table is the single row that generates the
/// variant, the constant, and the `Rule::ALL` entry, so reading it is the only
/// source that cannot drift from what the compiler sees. The caller panics if a
/// rule in `Rule::ALL` is absent here, which is what makes a parse that silently
/// stopped matching visible instead of turning the whole audit into a no-op.
fn rule_constants(root: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(root.join("crates/aplan-core/src/diagnostic.rs"))
        .expect("reading the rule registry");
    let mut out = BTreeMap::new();
    for line in text.lines() {
        // `Variant => CONSTANT = "APLANnnn", Severity,`
        let Some((lhs, rhs)) = line.split_once("=>") else {
            continue;
        };
        if !lhs.trim().chars().all(|c| c.is_alphanumeric() || c == '_') || lhs.trim().is_empty() {
            continue;
        }
        let Some((konst, rest)) = rhs.split_once('=') else {
            continue;
        };
        let code: String = rest
            .trim()
            .trim_start_matches('"')
            .chars()
            .take_while(|c| *c != '"')
            .collect();
        if code.starts_with("APLAN") {
            out.insert(code, konst.trim().to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{mentions, raising_only};

    /// Reading a code is not raising it (`A1-178`).
    #[test]
    fn a_comparison_against_a_code_is_not_an_emission() {
        // One idiom, four spellings: either operator, either side. A reader who
        // hit the one this did not cover would be told to unit-test a rule their
        // crate cannot raise.
        for read in [
            "d.code == aplan_core::diagnostic::code::RUN_PROVES_NOTHING",
            "d.code != code::RUN_PROVES_NOTHING",
            "code::RUN_PROVES_NOTHING == d.code",
            "code::RUN_PROVES_NOTHING != d.code",
        ] {
            assert!(
                mentions(read, "RUN_PROVES_NOTHING"),
                "the text does hold it: {read}"
            );
            assert!(
                !mentions(&raising_only(read), "RUN_PROVES_NOTHING"),
                "but as a comparison, which is a reader: {read}"
            );
        }
        let read = "d.code == code::RUN_PROVES_NOTHING";
        // A construction guarded by an unrelated comparison is untouched: the
        // operands go, the rest of the line stays.
        for guarded in [
            "if a == b { Diagnostic::warning(code::RUN_PROVES_NOTHING, m) }",
            // And the other order, which is what separates "strip the operands"
            // from "strip the line": everything before the operator would go.
            "Diagnostic::warning(code::RUN_PROVES_NOTHING, m); if a == b {}",
        ] {
            assert!(
                mentions(&raising_only(guarded), "RUN_PROVES_NOTHING"),
                "a comparison beside a raise does not hide it: {guarded}"
            );
        }
        // Constructing still counts, including across the line break the
        // formatter puts between the call and its first argument.
        let raise = "Diagnostic::warning(\n    code::RUN_PROVES_NOTHING,\n    message,\n)";
        assert!(
            mentions(&raising_only(raise), "RUN_PROVES_NOTHING"),
            "a raiser is still a raiser"
        );
        // And a crate that does both is still a raiser.
        let both = format!("{read}\n{raise}");
        assert!(mentions(&raising_only(&both), "RUN_PROVES_NOTHING"));
    }

    #[test]
    fn mentions_requires_a_whole_word() {
        assert!(mentions("code::WEAK_EVIDENCE,", "WEAK_EVIDENCE"));
        assert!(mentions("WEAK_EVIDENCE", "WEAK_EVIDENCE"));
        assert!(!mentions("NOT_WEAK_EVIDENCE", "WEAK_EVIDENCE"));
        assert!(!mentions("WEAK_EVIDENCE_2", "WEAK_EVIDENCE"));
    }
}

/// Each `#[test]` function body in a chunk of test source, brace-balanced.
///
/// The audit above concatenates a crate's whole test half, which is the right
/// granularity for "is this rule tested here at all" and far too coarse for "does
/// *that* test assert on the message": every crate mentions `.message` somewhere,
/// so a whole-file check would pass for a rule whose own test asserts nothing of
/// the kind.
/// The source with `//` line comments removed, so prose cannot be mistaken for
/// code. Written to keep the brace scan honest and reused for the assertion
/// search, where a comment saying "assert on the message" must not count as one.
fn without_line_comments(text: &str) -> String {
    text.lines()
        .map(|l| match l.find("//") {
            Some(i) if !l[..i].contains('"') => &l[..i],
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn test_bodies(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(i) = text[at..].find("#[test]") {
        let start = at + i;
        let Some(open) = text[start..].find('{') else {
            break;
        };
        let open = start + open;
        // Braces inside string literals do not nest, and counting them merges one
        // test into the next, which credits a rule with its neighbour's
        // assertions. Comments are stripped by the caller: an apostrophe in
        // ordinary prose ("the rule's message") is indistinguishable from a char
        // literal by this scan, and treating it as one swallowed every brace
        // until the next apostrophe.
        let mut depth = 0usize;
        let mut end = open;
        let mut in_str = false;
        let mut escaped = false;
        for (j, c) in text[open..].char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match c {
                '\\' if in_str => escaped = true,
                '"' => in_str = !in_str,
                '{' if !in_str => depth += 1,
                '}' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + j + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push(&text[start..end.max(open)]);
        at = end.max(open + 1);
    }
    out
}

/// Whether a test body asserts on what a diagnostic *says*, not only that it
/// fired.
///
/// Field access, not the bare words. The first version matched `message`
/// anywhere, and this file's own backfill added eight comments reading "the
/// message must name ...": a rule could then be credited by a *comment* about
/// asserting, which is the gate believing its own documentation.
fn asserts_on_text(body: &str) -> bool {
    [".message", ".agent_instruction"]
        .iter()
        .any(|needle| body.contains(needle))
}

#[test]
fn every_rule_has_a_test_asserting_what_it_says() {
    // `A1-090`. The sibling audit proves each rule has a test; this one asks what
    // that test checks. Fourteen of twenty-five asserted only that a diagnostic
    // with the right code came back, which is how `APLAN024` shipped naming the
    // wrong commit: its test proved the warning appeared on a diluted citation,
    // which was the interesting part, and nobody read the sentence.
    //
    // `docs/rule-catalogue.md` (`A1-088`) pins the wording of every rule already.
    // This is the other half and cannot be replaced by it: the catalogue makes a
    // change to a message *visible*, and this makes the rule's author state what
    // the message should say at the moment they decide what it says.
    let root = workspace_root();
    let mut tests: BTreeMap<String, String> = BTreeMap::new();
    for path in sources(&root) {
        let Some(krate) = crate_of(&path, &root) else {
            continue;
        };
        let (_, test) = split(&path);
        tests.entry(krate).or_default().push_str(&test);
    }

    let constants = rule_constants(&root);
    let mut untested: Vec<String> = Vec::new();
    for rule in aplan_core::Rule::ALL {
        let Some(konst) = constants.get(rule.code()) else {
            continue;
        };
        let asserted = tests.values().any(|text| {
            let code_only = without_line_comments(text);
            test_bodies(&code_only)
                .into_iter()
                .any(|body| mentions(body, konst) && asserts_on_text(body))
        });
        if !asserted {
            untested.push(format!("{} (`{konst}`)", rule.code()));
        }
    }

    assert!(
        untested.is_empty(),
        "these rules have a test that proves they fired and nothing that checks what they said. \
         A message is the product, not decoration: assert on the specific entity it must name \
         (AGENTS.md, \"a diagnostic may name only what the predicate returned\").\n  {}",
        untested.join("\n  ")
    );
}
