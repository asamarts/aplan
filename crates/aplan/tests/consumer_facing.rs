//! `aplan`'s own decision numbers must not appear in what it writes into, or
//! says to, somebody else's repository (`A1-044`).
//!
//! "ADR-0009" resolves in this repository and carries the reasoning. In a
//! consumer's it is a citation to a decision record that does not exist, and the
//! reader cannot tell whether it was deleted or never existed. The p0000004
//! pilot found it in a generated `ROADMAP.status.md` committed to that project,
//! in the one file whose entire purpose is that generated output and
//! hand-written narrative cannot disagree.
//!
//! The audit checks **rendered artifacts**, not source text. Help comes from doc
//! comments and a body is assembled at runtime, so grepping for a string literal
//! would miss both. Each case below is something a consumer actually keeps: a
//! file committed in their repository, or the text their contributors read.
//!
//! Seven surfaces, and the list took two passes to get right. The first version
//! covered the four the finding named plus `facts`; reviewing it turned up three
//! more that are just as consumer-facing and were not checked at all: the
//! `.aplan.yml` scaffold `init` commits into their repository, the MCP tool
//! descriptions an agent reads while working there, and error messages. Only the
//! last is checked at the source level, for the reason given at [`SURFACES`].
//!
//! **What `A1-087` changed.** That source-level half read `#[error(` and nothing
//! else, so the audit covered 35 strings and the sweep stopped there: every
//! `anyhow::bail!` refusal, every `.context(...)` an error is wrapped in, every
//! interop `Notice`, every line of command output, every `agent_instruction` and
//! every `Diagnostic` message was read by nothing. This was proven rather than
//! argued: `ADR-0013` planted in the `mv` refusal passed all eight audits in this
//! file. The same sweep now reads seven kinds and some three hundred strings, and
//! a meta-test plants a violation in each one so a surface that silently stops
//! matching fails here rather than in a consumer's terminal.
//!
//! The count is deliberately vague in this sentence and exact in the floors
//! below: a precise total in prose is a number that goes stale on the next commit
//! that adds a message, and this file said 299 for two commits after it was 301.
//! The floors are the authority, they are per kind, and they are asserted.
//!
//! **Two limits, stated rather than discovered later.** A message assembled into a
//! variable and passed in (`let m = format!(...); bail!(m)`) is not read, because
//! the literal is outside the call's span; no production site does this today, and
//! all nine `let x = format!(` in `crates/*/src` build ids, tags, labels or
//! frontmatter rather than messages. And `xtask/` is not swept: it is this
//! repository's own build tool and speaks to nobody else.
//!
//! Rendered artifacts stay rendered: help, the generated roadmap, the scaffold,
//! the MCP descriptions and `facts` are checked as output, which is stronger than
//! reading their source, and diagnostics are additionally covered by construction
//! where the scenario corpus lives (`crates/aplan-e2e/tests/coverage.rs`).

mod common;

use aplan_core::Status;
use aplan_core::task::Task;

/// A citation to this project's own bookkeeping, in text addressed elsewhere.
///
/// Decision records and **task ids** both. `ADR-0009` was the case that started
/// this (`A1-044`); a task id is the same thing with a different prefix, and
/// `A1-106` nearly put one into `aplan tasks --help`, where it would reach a
/// consumer through `facts` as well. A rule code is deliberately *not* a leak: it
/// is shared vocabulary a consumer's CI matches on.
fn leaks(text: &str) -> Option<String> {
    let mut from = 0usize;
    while let Some(rel) = text[from..].find("ADR-") {
        let at = from + rel;
        let tail = &text[at..];
        let end = tail
            .char_indices()
            .find(|(_, c)| !c.is_ascii_alphanumeric() && *c != '-')
            .map_or(tail.len(), |(i, _)| i);
        // A citation carries a number. The bare prefix `ADR-` is how
        // `roadmap_json` recognises a reference in somebody else's document, and
        // reading it as a leak would make the wider sweep below fail on a
        // constant that cites nothing (`A1-255`).
        if tail[..end].chars().any(|c| c.is_ascii_digit()) {
            return Some(tail[..end].to_string());
        }
        from = at + "ADR-".len();
    }
    task_id(text)
}

/// A task id: one letter, one digit, a dash, three digits, as this plan spells
/// them (`A1-106`, `P1-003`). Bounded tightly so ordinary prose and rule codes do
/// not match.
///
/// **It cannot tell a citation from an example**, and on its first run it caught
/// both: a real leak in `aplan tasks --help`, and `add --help` illustrating the id
/// convention with a worked pair. Changing the prefix did not help, since the
/// shape is what matches. That example now states the rule instead, which is
/// better for a consumer anyway: what they need is "prefix and width kept, number
/// incremented", not two ids from somebody else's plan.
fn task_id(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len().saturating_sub(5) {
        let w = &chars[i..i + 6];
        let boundary_before = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
        let boundary_after = chars.get(i + 6).is_none_or(|c| !c.is_ascii_alphanumeric());
        if boundary_before
            && boundary_after
            && w[0].is_ascii_uppercase()
            && w[1].is_ascii_digit()
            && w[2] == '-'
            && w[3..].iter().all(char::is_ascii_digit)
        {
            return Some(w.iter().collect());
        }
    }
    None
}

#[test]
fn no_help_text_cites_this_projects_adrs() {
    // Walks the whole command tree, so a subcommand added later is covered
    // without anyone remembering this file exists.
    let mut found: Vec<String> = Vec::new();
    let mut stack = vec![aplan::cli_command()];
    while let Some(cmd) = stack.pop() {
        let name = cmd.get_name().to_string();
        let mut texts: Vec<String> = [cmd.get_about(), cmd.get_long_about(), cmd.get_before_help()]
            .into_iter()
            .flatten()
            .map(ToString::to_string)
            .collect();
        for arg in cmd.get_arguments() {
            texts.extend(
                [arg.get_help(), arg.get_long_help()]
                    .into_iter()
                    .flatten()
                    .map(ToString::to_string),
            );
        }
        for text in texts {
            if let Some(hit) = leaks(&text) {
                found.push(format!("`aplan {name} --help` cites {hit}"));
            }
        }
        stack.extend(cmd.get_subcommands().cloned());
    }
    assert!(
        found.is_empty(),
        "help is read beside the consumer's own docs/adr/, where these do not resolve \
         (A1-044). State the guarantee and keep the number in a `//` comment:\n  {}",
        found.join("\n  ")
    );
}

/// **Help does not offer this repository's own plan as evidence** (`A1-235`).
///
/// The sibling of `no_help_text_cites_this_projects_adrs`, and the same reason:
/// help is read beside the reader's own repository, where a measurement taken on
/// *ours* describes nothing they can check. It is worse than a dangling
/// reference, because a number reads as a fact about the command rather than as
/// a citation they can see is foreign.
///
/// Found by writing one. `tree`'s deprecation notice announced the coming default
/// with "on this project's own plan `--depth 2` is 149 lines of which 133 are
/// `done`" -- taken from one task, on one day, stale within a week (150 and 11 by
/// the time it shipped), and reading as a general claim. The measurement belongs
/// in `CHANGELOG.md`, which is this repository's own document, and the help
/// states the guarantee.
#[test]
fn no_help_text_offers_this_projects_plan_as_evidence() {
    // Phrases, not numbers: help may legitimately quote a size in an example.
    // What it may not do is claim the reader is looking at ours.
    const BORROWED: [&str; 4] = [
        "this project's own plan",
        "this project's plan",
        "this repository's own plan",
        "this repository's plan",
    ];
    let found: Vec<String> = rendered_help()
        .into_iter()
        .flat_map(|(name, text)| {
            let lower = text.to_lowercase();
            BORROWED
                .iter()
                .filter(move |p| lower.contains(*p))
                .map(move |p| format!("`{name} --help` says {p:?}"))
        })
        .collect();
    assert!(
        found.is_empty(),
        "help offers this repository's plan as evidence, which the reader cannot check \
         against their own (`A1-044`'s reasoning). State the guarantee and put the \
         measurement in `CHANGELOG.md`:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn no_imported_task_body_cites_this_projects_adrs() {
    // The worst site, because it is the most durable: this text is written into
    // the consumer's task file, committed there, and never regenerated.
    let body = aplan_interop::emit::provenance_body("kep.yaml", "kep");
    assert_eq!(leaks(&body), None, "provenance_body: {body}");
    assert!(
        body.contains("lossy by\ndesign") || body.contains("lossy by design"),
        "the guarantee itself must survive the edit, not just the citation: {body}"
    );
}

#[test]
fn no_generated_roadmap_cites_this_projects_adrs() {
    let task = Task {
        id: "A-1".into(),
        title: "A task".into(),
        phase: 1,
        status: Status::Todo,
        owner: None,
        effort_days: Some(1.0),
        blocked_by: Vec::new(),
        prefer_after: Vec::new(),
        evidence: Vec::new(),
        paths: Vec::new(),
        traces: Vec::new(),
        trace_fingerprints: std::collections::BTreeMap::new(),
        tags: Vec::new(),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
    };
    let graph = aplan_core::Graph::new(std::iter::once(&task));
    let markdown = aplan_output::render_roadmap_markdown(&graph.roadmap(), None);
    assert_eq!(leaks(&markdown), None, "{markdown}");
    assert!(
        markdown.contains("gated by `aplan verify` in CI"),
        "the footer's guarantee must survive the edit: {markdown}"
    );
}

#[test]
fn no_scaffolded_config_cites_this_projects_adrs() {
    // `aplan init` writes this into the consumer's repository as `.aplan.yml`,
    // where it is committed and never regenerated: the same durability as an
    // imported task body, and it was not covered by the first version of this
    // audit. `APLAN009` and friends are fine and deliberate here, a rule code is
    // a consumer contract; an ADR number is this project's private bookkeeping.
    let scaffold = aplan_config::SCAFFOLD;
    assert_eq!(
        leaks(scaffold),
        None,
        "the .aplan.yml scaffold cites an ADR"
    );
    assert!(
        scaffold.contains("APLAN009"),
        "rule codes are shared vocabulary and must not be stripped by mistake"
    );
}

#[test]
fn no_mcp_tool_description_cites_this_projects_adrs() {
    // Descriptions are loaded into an agent's context while it works in the
    // consumer's repository, which is exactly the reader who cannot resolve the
    // number, and an agent is the reader most likely to act on a dangling one.
    let leaking: Vec<&str> = aplan_mcp::tools::TOOLS
        .iter()
        .filter(|t| leaks(t.description).is_some())
        .map(|t| t.name)
        .collect();
    assert!(leaking.is_empty(), "MCP tools citing an ADR: {leaking:?}");
}

#[test]
fn no_generated_facts_cite_this_projects_adrs() {
    // `aplan facts` is how a consumer's tooling asks what this tool checks, and
    // `facts.json` is committed here for the same purpose. It reuses the clap
    // `about` strings, so this is the same leak arriving by a second route: the
    // help fix above was found stale by *this* file's gate, not by the help one.
    let facts = aplan::facts();
    assert_eq!(leaks(&facts), None, "aplan facts cites a decision record");
}

/// The call sites whose string arguments a consumer reads, and the name each is
/// reported under.
///
/// Source-level, and the reason is worth stating: this text is rendered only when
/// its occasion happens, and reaching all of it from a test would mean provoking
/// every I/O failure, every git failure and every refusal the tool can produce.
/// The call site is where the text lives, so reading it is exact rather than a
/// sampling, and it cannot match a doc comment by accident.
///
/// `bail!` catches `anyhow::bail!` and `anyhow!` catches `anyhow::anyhow!`, since
/// each is matched as a suffix of the path.
const SURFACES: &[(&str, &str)] = &[
    ("#[error(", "error"),
    ("bail!(", "refusal"),
    ("anyhow!(", "refusal"),
    (".context(", "context"),
    (".with_context(", "context"),
    ("Notice::new(", "notice"),
    ("with_agent_instruction(", "instruction"),
    ("Diagnostic::error(", "diagnostic"),
    ("Diagnostic::warning(", "diagnostic"),
    ("writeln!(", "output"),
    ("write!(", "output"),
    ("println!(", "output"),
    ("eprintln!(", "output"),
];

/// One consumer-facing string, and where it came from.
#[derive(Debug, Clone)]
struct Consumer {
    file: String,
    kind: &'static str,
    text: String,
}

/// The source with `#[cfg(test)]` modules removed.
///
/// A fixture is not addressed to anybody: `roadmap_json.rs` carries `"ADR-0001"`
/// and `"Cited research (ADR-0003, SPEC-0009)"` as *input* to importer tests, and
/// a sweep that read those would report the tool leaking its own decisions while
/// pointing at a string nobody is ever shown.
fn without_test_modules(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(i) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        match after.find('{') {
            Some(open) => match balanced(&after[open..], '{', '}') {
                Some(end) => rest = &after[open + end..],
                None => return out,
            },
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The offset just past the delimiter that closes the one `s` opens, skipping
/// over string and char literals so a brace inside `"}"` does not count.
fn balanced(s: &str, open: char, close: char) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i] as char;
        match c {
            '"' => i += raw_or_plain_string_len(&s[i..])?.0,
            '\'' if b.get(i + 1).is_some_and(|n| *n != b'\'') && i + 2 < b.len() => {
                // A char literal or a lifetime; skip the literal form only.
                let close_at = s[i + 1..].find('\'').map_or(0, |j| j + 2);
                i += if close_at > 0 && close_at <= 4 {
                    close_at
                } else {
                    1
                };
            }
            'r' if s[i..].starts_with("r\"") || s[i..].starts_with("r#") => {
                i += raw_or_plain_string_len(&s[i..])?.0;
            }
            _ => {
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                i += 1;
            }
        }
    }
    None
}

/// The byte length of the string literal starting at `s`, and its content, for
/// both `"..."` (with escapes) and `r#"..."#`.
fn raw_or_plain_string_len(s: &str) -> Option<(usize, String)> {
    let b = s.as_bytes();
    if b.first() == Some(&b'r') {
        let hashes = s[1..].chars().take_while(|c| *c == '#').count();
        let open = 1 + hashes + 1; // r, hashes, quote
        if s.as_bytes().get(1 + hashes) != Some(&b'"') {
            return None;
        }
        let terminator = format!("\"{}", "#".repeat(hashes));
        let end = s[open..].find(&terminator)? + open;
        return Some((end + terminator.len(), s[open..end].to_string()));
    }
    if b.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    let mut content = String::new();
    while i < b.len() {
        match b[i] {
            b'\\' => {
                // Decode, rather than keeping the source form. A sweep that held
                // `\n(statuses...` reads the `n` as a word character, and the
                // unresolved-plural property then reports a welded plural in a
                // line that has none. The properties must see the message a
                // consumer sees, not the way it was spelled in Rust.
                match b.get(i + 1) {
                    Some(b'n') => content.push('\n'),
                    Some(b't') => content.push('\t'),
                    Some(b'r') => content.push('\r'),
                    Some(b'0') => content.push('\0'),
                    // A line continuation: backslash-newline eats the newline and
                    // the indentation after it, which is how every long message in
                    // this codebase is written.
                    Some(b'\n') => {
                        i += 2;
                        while b.get(i).is_some_and(u8::is_ascii_whitespace) {
                            i += 1;
                        }
                        continue;
                    }
                    Some(other) => content.push(*other as char),
                    None => {}
                }
                i += 2;
            }
            b'"' => return Some((i + 1, content)),
            _ => {
                let ch = s[i..].chars().next()?;
                content.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    None
}

/// Every string literal inside a span of Rust source, concatenated per literal.
fn string_literals(span: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < span.len() {
        if !span.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &span[i..];
        if rest.starts_with('"') || rest.starts_with("r\"") || rest.starts_with("r#") {
            if let Some((len, content)) = raw_or_plain_string_len(rest) {
                out.push(content);
                i += len;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Every string literal in a whole source file, skipping comments and char
/// literals (`A1-255`).
///
/// [`string_literals`] is given a *call-site argument span*, where a quote is
/// almost always a string's. Handed a whole file it pairs the quote inside
/// `// the item's "first sentence"` with a later one and returns the code
/// between them as a literal, which is how this function came to exist: the
/// first version of the wide sweep reported twenty-seven ADR citations that were
/// comments, and a sweep that invents its own input proves nothing about the
/// real one (`A1-255`, audit of its own change).
fn literals_outside_comments(src: &str) -> Vec<String> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut depth = 0usize; // nested `/* */`
    while i < b.len() {
        if depth > 0 {
            if b[i..].starts_with(b"*/") {
                depth -= 1;
                i += 2;
            } else if b[i..].starts_with(b"/*") {
                depth += 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            depth = 1;
            i += 2;
            continue;
        }
        // A char literal, including `'"'` and `'\''`. A lifetime (`'a`) has no
        // closing quote and is left to fall through as ordinary bytes.
        if b[i] == b'\'' {
            let end = if b[i..].starts_with(b"'\\") {
                src[i + 2..].find('\'').map(|k| i + 2 + k + 1)
            } else if i + 2 < b.len() && b[i + 2] == b'\'' {
                Some(i + 3)
            } else {
                None
            };
            if let Some(e) = end {
                i = e;
                continue;
            }
            i += 1;
            continue;
        }
        if !src.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &src[i..];
        if (rest.starts_with('"') || rest.starts_with("r\"") || rest.starts_with("r#"))
            && let Some((len, content)) = raw_or_plain_string_len(rest)
        {
            out.push(content);
            i += len;
            continue;
        }
        i += 1;
    }
    out
}

/// **The scanner reads literals and nothing else.**
///
/// Each case here is one the first version got wrong, or would have: a quote
/// inside a line comment, inside a block comment, and inside a char literal.
/// Getting any of them wrong does not fail loudly -- it invents a "literal"
/// made of the code in between, which is how a sweep starts proving things
/// about text nobody wrote (`A1-255`).
#[test]
fn the_literal_scanner_skips_comments_and_char_literals() {
    let src = r##"
// a comment with a "quote in it
let a = "first";
/* a block comment with a "quote, and /* a nested one "too */ still inside */
let b = '"';
let c = "second";
let d = '\'';
let e = r#"raw "inside" it"#;
"##;
    let got = literals_outside_comments(src);
    assert_eq!(
        got,
        vec![
            "first".to_string(),
            "second".to_string(),
            "raw \"inside\" it".to_string()
        ],
        "a quote in a comment or a char literal is not a string"
    );
}

/// The wide sweep is wider than the call-site one, which is the whole point, and
/// a number here says by how much rather than leaving it to be assumed.
#[test]
fn the_wide_sweep_sees_more_than_the_call_site_one() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let count = |texts: Vec<String>| {
        texts
            .iter()
            .flat_map(|t| backticked(t))
            .filter(|s| s.starts_with("aplan "))
            .count()
    };
    let narrow = count(harvest(&root).strings.into_iter().map(|c| c.text).collect());
    let wide = count(all_literals(&root).into_iter().map(|(_, t)| t).collect());
    assert!(
        wide > narrow,
        "the wide sweep found {wide} invocations and the call-site one {narrow}: if they are \
         equal this change bought nothing"
    );
}

/// Every command's rendered help, by command name (`A1-255`, audit).
///
/// `--help` is the most-read consumer surface this tool has, and it is written
/// in **doc comments**, which are not literals and not call-site arguments. So
/// neither the call-site sweep nor the wider literal one saw it: an invocation
/// named there reached every reader and was checked by nothing, demonstrated by
/// planting `aplan add --identifier X` in `--cut`'s help and watching every gate
/// stay green.
///
/// Rendered rather than scraped from the source, for the reason `A1-252` gave
/// for checking its own message that way: this is the text a reader copies, and
/// clap decides what it looks like.
fn rendered_help() -> Vec<(String, String)> {
    fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<(String, String)>) {
        let here = if prefix.is_empty() {
            cmd.get_name().to_string()
        } else {
            format!("{prefix} {}", cmd.get_name())
        };
        for sub in cmd.get_subcommands() {
            walk(sub, &here, out);
        }
        out.push((here, cmd.clone().render_long_help().to_string()));
    }
    let mut out = Vec::new();
    walk(&aplan::cli_command(), "", &mut out);
    out
}

/// Every string literal in non-test source, whatever it is for (`A1-255`).
///
/// [`harvest`] reads the *arguments of known call sites*, which is what makes
/// its `kind` labels meaningful and is also a premise: that consumer text is
/// written at one of them. A message built in a helper and interpolated into a
/// `bail!` is an argument of none, so it was swept by nothing -- renaming `--id`
/// to a flag that does not exist inside such a message left every gate green
/// (`A1-252`, audit).
///
/// The properties that need the label keep reading `harvest`. The one that does
/// not is the invocation check: a command line named anywhere in this source is
/// advice if a reader ever sees it, and checking one that no reader sees costs
/// nothing. So that property reads this instead, and the premise stops being
/// load-bearing.
fn all_literals(root: &std::path::Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "tests") {
                    continue; // a test's own strings are addressed to nobody
                }
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs")
                && let Ok(text) = std::fs::read_to_string(&p)
            {
                let file = p.display().to_string();
                for lit in literals_outside_comments(&without_test_modules(&text)) {
                    out.push((file.clone(), lit));
                }
            }
        }
    }
    out
}

/// What one file's scan produced.
struct Scan {
    strings: Vec<Consumer>,
    /// Call sites whose argument span could not be read.
    ///
    /// Tracked because the alternative is a silent skip. If the span scanner ever
    /// meets something it mishandles, the message it was guarding vanishes from
    /// the sweep and every audit here still passes, which is the precise failure
    /// `A1-087` exists to end. A drop must be loud.
    unreadable: Vec<String>,
}

/// Every consumer-facing string in one source file.
fn scan(file: &str, src: &str) -> Scan {
    let src = without_test_modules(src);
    let mut strings = Vec::new();
    let mut unreadable = Vec::new();
    for (needle, kind) in SURFACES {
        let mut at = 0usize;
        while let Some(i) = src[at..].find(needle) {
            let start = at + i + needle.len() - 1; // at the '('
            at = start + 1;
            let Some(end) = balanced(&src[start..], '(', ')') else {
                unreadable.push(format!("{file}: {needle} at byte {start}"));
                continue;
            };
            for text in string_literals(&src[start..start + end]) {
                if text.trim().is_empty() {
                    continue;
                }
                strings.push(Consumer {
                    file: file.to_string(),
                    kind,
                    text,
                });
            }
        }
    }
    Scan {
        strings,
        unreadable,
    }
}

/// Every consumer-facing string in one source file.
fn consumer_strings(file: &str, src: &str) -> Vec<Consumer> {
    scan(file, src).strings
}

/// Every consumer-facing string in the workspace's source.
fn harvest(root: &std::path::Path) -> Scan {
    let mut out = Scan {
        strings: Vec::new(),
        unreadable: Vec::new(),
    };
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "tests") {
                    continue; // a test's own strings are addressed to nobody
                }
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    let s = scan(&p.display().to_string(), &text);
                    out.strings.extend(s.strings);
                    out.unreadable.extend(s.unreadable);
                }
            }
        }
    }
    out
}

#[test]
fn no_consumer_facing_string_cites_this_projects_adrs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let Scan {
        strings,
        unreadable,
    } = harvest(&root);

    // A call site the scanner cannot read is a message that silently left the
    // sweep. Every audit here would still pass, which is the exact shape of the
    // hole this task closed, so it fails loudly instead.
    assert!(
        unreadable.is_empty(),
        "the span scanner could not read {} call site(s), so their text is swept by nothing:\n  {}",
        unreadable.len(),
        unreadable.join("\n  ")
    );

    // Before `A1-087` this scanned `#[error(` alone, so 41% of the surface was
    // swept by nothing: a planted `ADR-0013` in the `mv` refusal passed every
    // audit in this file. Each kind is counted, because a pattern that quietly
    // stops matching leaves a gate that passes for the wrong reason, and a total
    // alone would hide one surface going to zero.
    for (kind, least) in [
        ("error", 30),
        ("refusal", 33),
        ("context", 11),
        ("notice", 25),
        ("instruction", 38),
        ("diagnostic", 36),
        ("output", 85),
    ] {
        let n = strings.iter().filter(|c| c.kind == kind).count();
        assert!(
            n >= least,
            "the sweep found only {n} `{kind}` strings, fewer than the {least} known to exist, \
             so that pattern has stopped matching and this gate passes vacuously"
        );
    }

    // Over every literal, not only the ones at a known call site. A message
    // built in a helper reaches a reader exactly as one written at a `bail!`
    // does, and was swept by neither this nor the invocation check (`A1-255`).
    // The per-kind counts above still read `harvest`, because a ratchet against
    // one pattern silently ceasing to match needs the label.
    let leaking: Vec<String> = all_literals(&root)
        .into_iter()
        .chain(
            rendered_help()
                .into_iter()
                .map(|(n, t)| (format!("`{n} --help`"), t)),
        )
        .filter_map(|(file, text)| {
            leaks(&text).map(|hit| format!("{file}: {hit} in {:?}", one_line(&text)))
        })
        .collect();
    assert!(
        leaking.is_empty(),
        "text a consumer reads cites a decision record only this repository has (A1-044):\n  {}",
        leaking.join("\n  ")
    );
}

/// A message shortened for a failure report.
fn one_line(text: &str) -> String {
    let flat = text.replace('\n', " ");
    if flat.chars().count() > 60 {
        format!("{}...", flat.chars().take(57).collect::<String>())
    } else {
        flat
    }
}

#[test]
fn the_sweep_reaches_every_kind_of_consumer_facing_string() {
    // The gate on the gate. `A1-087` exists because this sweep read `#[error(`
    // alone and nothing said so: a planted `ADR-0013` in a `bail!` passed all
    // eight audits here. A widened sweep that silently missed a surface would
    // fail exactly the same way, so each is planted and located by name.
    let src = r#"
        #[derive(Debug)]
        enum E {
            #[error("cannot read {0} ADR-1001")]
            Io(String),
        }
        fn f() -> Result<()> {
            anyhow::bail!("refusing to move a milestone ADR-1002");
            read(p).context("reading the plan ADR-1011")?;
            write(p).with_context(|| format!("writing {p} ADR-1012"))?;
            Err(anyhow!("no such cut ADR-1003"))
        }
        fn g() {
            notices.push(Notice::new("dropped the estimate ADR-1004"));
            Diagnostic::error(code::X, "a done task ADR-1005")
                .with_agent_instruction("re-stamp with `aplan close` ADR-1006");
            Diagnostic::warning(code::Y, "advisory ADR-1007");
            writeln!(out, "ok: {n} tasks ADR-1008")?;
            println!("wrote {rel} ADR-1009");
            eprintln!("usage: aplan ADR-1010");
        }
    "#;

    let found = consumer_strings("synthetic.rs", src);
    let hits: Vec<&str> = found.iter().map(|c| c.text.as_str()).collect();
    for (n, kind) in [
        (1001, "error"),
        (1002, "refusal"),
        (1003, "refusal"),
        (1011, "context"),
        (1012, "context"),
        (1004, "notice"),
        (1005, "diagnostic"),
        (1006, "instruction"),
        (1007, "diagnostic"),
        (1008, "output"),
        (1009, "output"),
        (1010, "output"),
    ] {
        let needle = format!("ADR-{n}");
        let got = found.iter().find(|c| c.text.contains(&needle));
        assert!(
            got.is_some(),
            "the sweep missed a {kind} string: no literal carrying `{needle}`\n\
             it found: {hits:#?}"
        );
        assert_eq!(
            got.expect("checked above").kind,
            kind,
            "`{needle}` was found but filed under the wrong surface"
        );
    }
}

#[test]
fn a_fixture_inside_a_test_module_is_addressed_to_nobody() {
    // `roadmap_json.rs` carries `"ADR-0001"` as importer *input*. Sweeping it
    // would report the tool leaking a decision record while pointing at a string
    // no consumer is ever shown, and a false positive in a gate is how the gate
    // gets disabled.
    let src = r#"
        fn real() { bail!("a real refusal") }
        #[cfg(test)]
        mod tests {
            const FIXTURE: &str = "{\"adrs\": [\"ADR-0001\"]}";
            fn t() { bail!("a fixture refusal ADR-0002") }
        }
    "#;
    let found = consumer_strings("synthetic.rs", src);
    assert!(found.iter().any(|c| c.text.contains("a real refusal")));
    assert!(
        !found.iter().any(|c| c.text.contains("ADR-000")),
        "a `#[cfg(test)]` module is not consumer-facing: {found:#?}"
    );
}

#[test]
fn a_literal_is_read_through_escapes_raw_strings_and_nested_parens() {
    // The span scanner counts parentheses, so a `)` inside a message must not end
    // it early, and a `"` inside a raw string must not start a literal.
    let src = r##"
        fn f() {
            bail!("could not edit `{path}` (line {n}); change it by hand");
            bail!(r#"a "quoted" refusal"#);
            bail!("an escaped \" quote and a ) paren");
        }
    "##;
    let texts: Vec<String> = consumer_strings("s.rs", src)
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("change it by hand")),
        "a `)` inside the message truncated the span: {texts:#?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("a \"quoted\" refusal")),
        "a raw string was not read: {texts:#?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("an escaped")),
        "an escaped quote ended the literal early: {texts:#?}"
    );
}

#[test]
fn a_char_literal_paren_does_not_unbalance_the_span() {
    // `resolve.rs` really does write `glob.find(['*', '?', '[', '{'])`. A scanner
    // that counted the `(` in a `'('` would run the span past its own call and
    // swallow whatever came next, which fails by over-reporting rather than
    // loudly, and so would not be noticed.
    let src = r#"
        fn f() {
            bail!("bad glob `{}` at {}", g, g.find('(').unwrap());
            writeln!(out, "the next message")?;
        }
    "#;
    let texts: Vec<String> = consumer_strings("s.rs", src)
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("bad glob")),
        "the refusal was lost: {texts:#?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("the next message")),
        "the span ran past its call and swallowed the next one: {texts:#?}"
    );
}

#[test]
fn the_audit_can_tell_a_citation_from_ordinary_prose() {
    // The check itself, since a leak detector that never fires is worse than none.
    assert_eq!(leaks("gated by `aplan verify` in CI."), None);
    assert_eq!(leaks("see ADR-0009 for why"), Some("ADR-0009".to_string()));
    assert_eq!(leaks("(ADR-0012)"), Some("ADR-0012".to_string()));
    // A word that merely starts the same way is not a citation.
    assert_eq!(leaks("ADRENALINE"), None);
    // A task id is this project's bookkeeping too, and it reaches a consumer
    // through `facts` as well as `--help`.
    assert_eq!(leaks("tracked as A1-106"), Some("A1-106".to_string()));
    assert_eq!(leaks("see P1-003"), Some("P1-003".to_string()));
    // A rule code is shared vocabulary, not a leak.
    assert_eq!(leaks("APLAN024 reports this"), None);
    // And ordinary text is not an id.
    assert_eq!(leaks("v0.8 ships"), None);
    assert_eq!(leaks("HLD 5.5"), None);
}

// --- Mechanical properties over the swept text (`A1-089`) --------------------
//
// `A1-087` collected the text; these are the checks that collection makes
// possible. Their honest yield differs and is stated per property rather than
// implied, because a gate sold as finding bugs when it only guards regressions
// is the same overclaim these gates exist to catch.

/// Every backticked span in a string, without the backticks.
///
/// **Deliberately duplicated** in `crates/aplan-e2e/tests/coverage.rs`, and the
/// reason is layering rather than laziness. `argv_of` was shared through
/// `tests/common/` because both its users are in this crate; `aplan-e2e` sits at
/// the top of the dependency graph, so making `aplan` dev-depend on it to reach
/// a twelve-line text helper would invert that and leave a cycle waiting for the
/// day `aplan-e2e` needs the binary crate. Both copies carry the same unit test,
/// so a divergence shows up as a failure rather than as a puzzle.
fn backticked(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        match after.find('`') {
            Some(close) => {
                out.push(after[..close].to_string());
                rest = &after[close + 1..];
            }
            None => break,
        }
    }
    out
}

#[test]
fn every_invocation_a_message_names_actually_parses() {
    // Finds nothing today: every backticked `aplan ...` in the tool's own text is
    // real. It is a regression guard, and it guards a failure worse than it
    // sounds, because an instruction naming a flag that does not exist is advice
    // a reader will try, fail at, and then distrust the rest of. The same gate
    // over `README.md` caught `aplan mv --to` within minutes of it being written
    // (`A1-092`), so the failure mode is demonstrated, not hypothetical.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut problems = Vec::new();
    let mut checked = 0usize;

    for (file, text) in all_literals(&root).into_iter().chain(
        rendered_help()
            .into_iter()
            .map(|(n, t)| (format!("`{n} --help`"), t)),
    ) {
        for span in backticked(&text) {
            if !span.starts_with("aplan ") {
                continue;
            }
            checked += 1;
            let argv = common::argv_of(&span);
            if let Err(e) = aplan::cli_command().try_get_matches_from(&argv) {
                // The question is whether this command and these flags *exist*,
                // not whether the line is runnable as written. An instruction
                // naming `aplan close --restamp` is naming a form; demanding a
                // complete invocation would force every message to invent an id,
                // which is worse advice, not better. So a missing positional is
                // fine and an argument the CLI does not have is not.
                use clap::error::ErrorKind::{InvalidSubcommand, UnknownArgument, ValueValidation};
                if matches!(
                    e.kind(),
                    UnknownArgument | InvalidSubcommand | ValueValidation
                ) {
                    let first = e.to_string().lines().next().unwrap_or_default().to_string();
                    problems.push(format!("{file}: `{span}`\n      {first}"));
                }
            }
        }
    }

    // Two floors, because the sweep now has two halves and either could stop
    // matching on its own. The numbers are below what is there (72 invocations
    // across 34 commands) and above nothing, which is the only way a "passes
    // vacuously" guard is worth having (`A1-255`).
    assert!(
        checked >= 60,
        "the sweep found {checked} invocations to check, fewer than the 60 known to be there, \
         so it has stopped matching and this property passes vacuously"
    );
    let commands = rendered_help().len();
    assert!(
        commands >= 30,
        "the command tree rendered {commands} commands, fewer than the 30 it has, so the help \
         half of this sweep is reading almost nothing"
    );
    assert!(
        problems.is_empty(),
        "text a consumer reads names an invocation the CLI rejects:\n  {}",
        problems.join("\n  ")
    );
}

/// An unresolved plural: a parenthesised alternative welded onto a word.
///
/// `1 task(s)` is the house idiom and stays. `entr(y/ies)` and `capabilit(ies)`
/// both shipped, and both read as a template nobody finished.
fn unresolved_plural(text: &str) -> Option<String> {
    let bytes: Vec<char> = text.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        if *c != '(' || i == 0 || !bytes[i - 1].is_ascii_alphabetic() {
            continue;
        }
        // Ending the whole scan here is correct, not a slip: with no `)` after
        // this point, no later `(` can close either, so nothing remains to find.
        let close = bytes[i..].iter().position(|c| *c == ')')? + i;
        let inner: String = bytes[i + 1..close].iter().collect();
        if inner != "s" {
            let start = i.saturating_sub(12);
            return Some(bytes[start..=close].iter().collect());
        }
    }
    None
}

/// Evidence a reader is told to cite must be spellable in a shell (`A1-156`).
///
/// `commit:<sha>` is not a metavariable in an unquoted shell word: `<` is
/// redirection, so the line fails at parse time and does something other than
/// what it says. The fifth pilot's audit parsed every command block rather than
/// trusting the fences around them, and found the shape in the generated agent
/// contract and in the remediation of five import formats -- a contributor who
/// copied the instruction literally was stopped before this tool could report a
/// single plan finding.
///
/// Keyed on the evidence spelling rather than on whether a string "looks like a
/// command", because that judgement is what let the shape spread across five
/// modules in the first place. Prose naming a *form* -- `aplan tree <id>`, where
/// nothing is concrete and the reader is choosing the subject -- keeps its angle
/// brackets and is not this test's business.
#[test]
fn no_evidence_citation_is_spelled_with_shell_redirection() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let kinds = ["commit", "url", "file", "test", "note"];
    let mut problems = Vec::new();
    let mut checked = 0usize;

    for c in harvest(&root).strings {
        for kind in kinds {
            let needle = format!("{kind}:");
            let mut rest = c.text.as_str();
            while let Some(i) = rest.find(&needle) {
                rest = &rest[i + needle.len()..];
                checked += 1;
                if rest.starts_with('<') {
                    problems.push(format!("{}: `{kind}:<...`", c.file));
                }
            }
        }
    }

    assert!(
        checked >= 10,
        "the sweep found {checked} citation spelling(s), so it has stopped matching and this \
         passes vacuously"
    );
    assert!(
        problems.is_empty(),
        "a citation a reader is told to write is shell redirection, not a placeholder:\n  {}",
        problems.join("\n  ")
    );
}

/// A gap a reader sees as a gap: a run of spaces welded into a sentence.
///
/// Rust joins a `\`-continued string with no separator, so a line broken for the
/// source and re-indented leaves the indentation *inside* the message. It looks
/// like nothing in the diff and like a paste error in the output. Four found on
/// one sweep, one of them in `facts.json`, which is a byte-gated consumer
/// document (`A1-189`, audit).
///
/// **Four spaces, and only in a string with no newline of its own.** A literal
/// that carries `\n` is a fixture or a multi-line block, where indentation is
/// the layout rather than an accident; three spaces occur in a spec-kit test
/// document on purpose. Four in a single-line message is not something anybody
/// types.
///
/// **Except before a `#`**, which is a comment column. The generated agent
/// contract writes a `sh` block whose comments are aligned -- `aplan ready` and
/// a run of spaces and `# what can be started now` -- and that is layout a
/// reader wants. Alignment before a comment marker is the one shape where a run
/// of spaces means something, and it is the only exception this needs.
fn welded_gap(text: &str) -> Option<String> {
    if text.contains('\n') {
        return None;
    }
    let bytes: Vec<char> = text.chars().collect();
    let mut run = 0;
    for (i, c) in bytes.iter().enumerate() {
        if *c == ' ' {
            run += 1;
            continue;
        }
        if run >= 4 && i > run && *c != '#' {
            let start = i.saturating_sub(run + 8);
            return Some(bytes[start..=i].iter().collect());
        }
        run = 0;
    }
    None
}

/// Over every literal, not only the call sites `harvest` labels.
///
/// The same reasoning `every_invocation_a_message_names_actually_parses` uses:
/// a string a reader ever sees is one this applies to, and checking one nobody
/// sees costs nothing. It is also not optional here -- the widest of the four
/// this was written for is a `Behaviour`'s `what`, which is a struct field and
/// reaches `facts.json`, a byte-gated consumer document that no call-site scan
/// would ever have looked at.
#[test]
fn no_message_carries_a_run_of_spaces_from_its_own_source_indentation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let problems: Vec<String> = all_literals(&root)
        .iter()
        .filter_map(|(file, text)| welded_gap(text).map(|hit| format!("{file}: ...{hit}...")))
        .collect();
    assert!(
        problems.is_empty(),
        "a message carries its own source indentation, which a reader sees as a gap:\n  {}\n\
         A `\\`-continued string joins with no separator; put the space before the backslash.",
        problems.join("\n  ")
    );
}

#[test]
fn the_gap_check_reads_a_welded_line_and_leaves_a_layout_alone() {
    // Both directions, for the reason the plural check has both: a checker that
    // never fires is worse than none.
    assert!(welded_gap("no graph.              `--membership` draws them").is_some());
    assert_eq!(welded_gap("ok: 103 task(s), 0 warning(s)"), None);
    assert_eq!(welded_gap("a  b"), None, "two is a typo, not this defect");
    assert_eq!(
        welded_gap("---\nid: T-001\n    ref: x\n"),
        None,
        "a fixture's indentation is its layout"
    );
    assert_eq!(welded_gap("    leading indentation is a layout too"), None);
    assert_eq!(
        welded_gap("aplan ready                 # what can be started now"),
        None,
        "a comment column in a code block is layout a reader wants"
    );
}

#[test]
fn no_message_leaves_a_plural_unresolved() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let problems: Vec<String> = harvest(&root)
        .strings
        .iter()
        .filter_map(|c| unresolved_plural(&c.text).map(|hit| format!("{}: {hit}", c.file)))
        .collect();
    assert!(
        problems.is_empty(),
        "a message reads as a template nobody finished:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_plural_check_permits_the_house_idiom_and_rejects_the_rest() {
    // Both directions, because a checker that never fires is worse than none and
    // one that fires on `1 task(s)` would be turned off the same day.
    assert_eq!(unresolved_plural("ok: 103 task(s), 0 warning(s)"), None);
    assert_eq!(unresolved_plural("touches 60 path(s) outside"), None);
    assert_eq!(
        unresolved_plural("evidence must meet one of (commit, test, file)"),
        None,
        "a parenthetical after a space is prose, not a welded plural"
    );
    assert!(unresolved_plural("dropped 3 entr(y/ies)").is_some());
    assert!(unresolved_plural("2 capabilit(ies) missing").is_some());
}

#[test]
fn backticks_are_read_in_pairs_and_an_unclosed_one_ends_the_scan() {
    // The same cases as the copy in the other crate; if the two ever disagree,
    // one of these fails.
    assert_eq!(
        backticked("re-stamp with `aplan close --restamp`, or reopen"),
        vec!["aplan close --restamp"]
    );
    assert_eq!(backticked("`a` and `b`"), vec!["a", "b"]);
    assert_eq!(backticked("no ticks here"), Vec::<String>::new());
    assert_eq!(backticked("an `unclosed span"), Vec::<String>::new());
    assert_eq!(backticked("``"), vec![""]);
}
