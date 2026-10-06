//! Anchoring into a structured document: YAML and JSON (`A1-191`, ADR-0035).
//!
//! `traces` binds a task to the thing it answers to, and until now that thing
//! had to be a markdown heading or table row. The sixth pilot's authoritative
//! artefacts are YAML -- an error catalogue, a state machine, a threat manifest
//! -- and the only way to trace one was to anchor the *prose that narrates it*,
//! which is a second copy of the fact in a tool whose whole argument is that
//! second copies drift (pre-public adoption evidence, item 2).
//!
//! **One value model for both formats.** YAML and JSON are parsed into
//! [`serde_json::Value`], so the address, the fingerprint and the near miss have
//! one implementation rather than two that must be kept agreeing. YAML that JSON
//! cannot represent -- a non-string key, an alias to a cycle -- fails to parse
//! here and is reported as a document this tool cannot address, which is honest
//! rather than a silent partial reading.
//!
//! **Two spellings, one meaning.** `#/states/provisioning` is an RFC 6901 JSON
//! Pointer; `#$.states.provisioning` is an RFC 9535 `JSONPath`. Each is
//! self-identifying by its first character, so a reader always knows which is
//! meant, and both compile to the same selector list.
//!
//! **Only the singular subset of `JSONPath`**, which RFC 9535 defines in its own
//! terms (section 2.3.5.1): a query syntactically restricted so it produces at
//! most one node whatever the input. An anchor whose meaning is "one node" is
//! the only thing a fingerprint can be taken of, and it is also the line this
//! project already drew: ADR-0026 refuses a query language, and accepting
//! wildcards, descendants, slices or filters here would be adding one through a
//! side door. The refusal names the construct, because "invalid anchor" teaches
//! nobody which half of their expression was the problem.

use std::collections::BTreeSet;
use std::fmt::Write as _;

/// A structured document format this tool can address into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Yaml,
    Json,
}

/// The format of `rel`, by extension, or `None` for a document addressed as
/// markdown (which is every other extension, including no extension at all).
#[must_use]
pub fn format_for(rel: &str) -> Option<Format> {
    let ext = std::path::Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    match ext.as_str() {
        "yaml" | "yml" => Some(Format::Yaml),
        "json" => Some(Format::Json),
        _ => None,
    }
}

/// Parse a structured document into the shared value model.
///
/// `None` when the text is not well formed, or is YAML that JSON cannot
/// represent. The caller reports that as a document it cannot address rather
/// than guessing at a partial reading.
#[must_use]
pub fn parse(text: &str, format: Format) -> Option<serde_json::Value> {
    match format {
        Format::Json => serde_json::from_str(text).ok(),
        Format::Yaml => serde_yaml_ng::from_str(text).ok(),
    }
}

/// One step of an address: a member name, or an array index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Name(String),
    Index(usize),
}

/// Why an anchor is not an address this tool accepts.
///
/// Each names the construct rather than saying "invalid": an author who wrote a
/// filter needs to be told that filters are refused, not that their anchor is
/// wrong somewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// What the author wrote that cannot be an address, in their own text.
    pub construct: String,
    /// Why it cannot be one, as a sentence continuing "... because".
    pub because: &'static str,
}

/// The two ways an address can be spelled, so a near miss is offered in the one
/// the author was already using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spelling {
    Pointer,
    Path,
}

/// Which spelling `anchor` uses, or `None` if it announces neither.
#[must_use]
pub fn spelling_of(anchor: &str) -> Option<Spelling> {
    match anchor.chars().next() {
        Some('/') => Some(Spelling::Pointer),
        Some('$') => Some(Spelling::Path),
        _ => None,
    }
}

/// Compile an anchor into the steps it addresses.
///
/// # Errors
///
/// Returns the construct that cannot be part of an address, and why.
pub fn parse_anchor(anchor: &str) -> Result<Vec<Step>, Refusal> {
    match spelling_of(anchor) {
        Some(Spelling::Pointer) => Ok(parse_pointer(anchor)),
        Some(Spelling::Path) => parse_path(anchor),
        None => Err(Refusal {
            construct: anchor.to_string(),
            because: "an address into a structured document starts with `/` for a JSON Pointer \
                      (`#/states/provisioning`) or `$` for a JSONPath (`#$.states.provisioning`), \
                      so a reader can tell which is meant",
        }),
    }
}

/// RFC 6901. `~1` is a literal `/` and `~0` a literal `~`, in that order.
fn parse_pointer(anchor: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    // The leading `/` opens the first token, so skipping one empty prefix is the
    // whole of the standard's framing.
    for token in anchor[1..].split('/') {
        let name = token.replace("~1", "/").replace("~0", "~");
        steps.push(Step::Name(name));
    }
    steps
}

/// RFC 9535, restricted to singular queries: name and index selectors only.
fn parse_path(anchor: &str) -> Result<Vec<Step>, Refusal> {
    let refuse = |construct: &str, because: &'static str| Refusal {
        construct: construct.to_string(),
        because,
    };
    let mut steps = Vec::new();
    let rest = &anchor[1..];
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                if chars.get(i + 1) == Some(&'.') {
                    return Err(refuse(
                        "..",
                        "a descendant segment can match at many depths at once, and an anchor \
                         addresses one node",
                    ));
                }
                i += 1;
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-')
                {
                    i += 1;
                }
                if i == start {
                    let what: String = chars[start..].iter().take(1).collect();
                    if what == "*" {
                        return Err(refuse(
                            "*",
                            "a wildcard selects every member, and an anchor addresses one node",
                        ));
                    }
                    return Err(refuse(
                        &format!(".{what}"),
                        "a name selector after `.` has to be a name",
                    ));
                }
                steps.push(Step::Name(chars[start..i].iter().collect()));
            }
            '[' => {
                let close = chars[i..].iter().position(|c| *c == ']').map(|p| p + i);
                let Some(close) = close else {
                    return Err(refuse("[", "a bracket selector is never closed"));
                };
                let inner: String = chars[i + 1..close].iter().collect();
                steps.push(bracket_step(&inner)?);
                i = close + 1;
            }
            c => {
                return Err(refuse(
                    &c.to_string(),
                    "a JSONPath segment starts with `.` or `[`",
                ));
            }
        }
    }
    Ok(steps)
}

/// One bracket selector, accepted only where it can address a single node.
fn bracket_step(inner: &str) -> Result<Step, Refusal> {
    let refuse = |construct: &str, because: &'static str| Refusal {
        construct: construct.to_string(),
        because,
    };
    let trimmed = inner.trim();
    if trimmed.starts_with('?') {
        return Err(refuse(
            &format!("[{inner}]"),
            "a filter selects whatever matches it, which is not a fixed address",
        ));
    }
    if trimmed == "*" {
        return Err(refuse(
            "[*]",
            "a wildcard selects every member, and an anchor addresses one node",
        ));
    }
    if trimmed.contains(':') {
        return Err(refuse(
            &format!("[{inner}]"),
            "a slice selects a range, and an anchor addresses one node",
        ));
    }
    if trimmed.contains(',') {
        return Err(refuse(
            &format!("[{inner}]"),
            "a union selects several nodes, and an anchor addresses one node",
        ));
    }
    if let Some(name) = quoted(trimmed) {
        return Ok(Step::Name(name));
    }
    if let Ok(n) = trimmed.parse::<usize>() {
        return Ok(Step::Index(n));
    }
    if trimmed.starts_with('-') {
        return Err(refuse(
            &format!("[{inner}]"),
            "an index counted from the end addresses a different element whenever the array \
             grows, and a stored anchor has to keep meaning what it meant",
        ));
    }
    Err(refuse(
        &format!("[{inner}]"),
        "a bracket selector here is a quoted member name or a whole-number index",
    ))
}

/// The contents of a quoted name selector, with its escapes resolved.
fn quoted(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let quote = chars.next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    if !s.ends_with(quote) || s.chars().count() < 2 {
        return None;
    }
    let inner: String = s[1..s.len() - 1].to_string();
    Some(
        inner
            .replace("\\\\", "\u{0}")
            .replace(&format!("\\{quote}"), &quote.to_string())
            .replace('\u{0}', "\\"),
    )
}

/// The node `steps` addresses, or `None` if the document has no such node.
#[must_use]
pub fn resolve<'a>(root: &'a serde_json::Value, steps: &[Step]) -> Option<&'a serde_json::Value> {
    let mut at = root;
    for step in steps {
        at = match (step, at) {
            (Step::Name(n), serde_json::Value::Object(map)) => map.get(n)?,
            (Step::Index(i), serde_json::Value::Array(items)) => items.get(*i)?,
            // RFC 6901 has one token type: a token addressing an array is an
            // index if it reads as one. A pointer therefore arrives here as
            // names throughout, and `/items/1` has to mean the second element
            // rather than a member called "1".
            (Step::Name(n), serde_json::Value::Array(items)) => {
                items.get(n.parse::<usize>().ok()?)?
            }
            // A name into an array, or an index into an object, is a shape the
            // document does not have. Reported as "no such node" rather than as
            // a malformed anchor: the anchor is well formed and simply wrong.
            _ => return None,
        };
    }
    Some(at)
}

/// The fingerprint of an addressed node: its canonical form, hashed.
///
/// Canonical rather than the source bytes, which is the point of giving a
/// structured format its own resolver. `serde_json`'s map is ordered, so
/// re-serialising sorts keys; comments and layout are not part of the value and
/// never reach this. Reindenting a file, reflowing a comment or reordering two
/// keys is therefore not drift, while changing, adding or removing a value is.
/// Hashing the source text instead would re-open every task that traced into a
/// file somebody ran a formatter over, which is the false positive that teaches
/// a repository to switch drift off.
#[must_use]
pub fn fingerprint(node: &serde_json::Value) -> String {
    crate::anchor::fnv1a_hex(&canonical(node))
}

/// The node as canonical JSON: sorted keys, no whitespace.
fn canonical(node: &serde_json::Value) -> String {
    // `serde_json::Map` is a `BTreeMap` unless `preserve_order` is on, which this
    // workspace does not enable, so `to_string` is already key-sorted. Asserted
    // by a test rather than trusted, because enabling that feature anywhere in
    // the tree would silently change every stored fingerprint.
    node.to_string()
}

/// Every address in `root`, in `spelling`, for a near miss.
///
/// Capped, because a near miss is a suggestion and a document with a hundred
/// thousand nodes should not turn one diagnostic into a memory problem. The cap
/// is generous next to a spec catalogue or a manifest, which is what this is for.
#[must_use]
pub fn addresses(root: &serde_json::Value, spelling: Spelling, cap: usize) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk(root, &mut Vec::new(), spelling, cap, &mut out);
    out
}

fn walk(
    node: &serde_json::Value,
    at: &mut Vec<Step>,
    spelling: Spelling,
    cap: usize,
    out: &mut BTreeSet<String>,
) {
    if out.len() >= cap {
        return;
    }
    if !at.is_empty() {
        out.insert(render(at, spelling));
    }
    match node {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                at.push(Step::Name(k.clone()));
                walk(v, at, spelling, cap, out);
                at.pop();
            }
        }
        serde_json::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                at.push(Step::Index(i));
                walk(v, at, spelling, cap, out);
                at.pop();
            }
        }
        _ => {}
    }
}

/// Render an address in one spelling, so a suggestion is offered in the form the
/// author was already writing.
#[must_use]
pub fn render(steps: &[Step], spelling: Spelling) -> String {
    match spelling {
        Spelling::Pointer => {
            let mut s = String::new();
            for step in steps {
                s.push('/');
                match step {
                    Step::Name(n) => s.push_str(&n.replace('~', "~0").replace('/', "~1")),
                    Step::Index(i) => s.push_str(&i.to_string()),
                }
            }
            s
        }
        Spelling::Path => {
            let mut s = String::from("$");
            for step in steps {
                match step {
                    Step::Name(n) if plain_name(n) => {
                        s.push('.');
                        s.push_str(n);
                    }
                    Step::Name(n) => {
                        let escaped = n.replace('\\', "\\\\").replace('\'', "\\'");
                        let _ = write!(s, "['{escaped}']");
                    }
                    Step::Index(i) => {
                        let _ = write!(s, "[{i}]");
                    }
                }
            }
            s
        }
    }
}

/// Whether a name can be written after a `.` rather than in brackets.
fn plain_name(n: &str) -> bool {
    !n.is_empty()
        && !n.starts_with(|c: char| c.is_ascii_digit())
        && n.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn doc() -> serde_json::Value {
        parse(
            "states:\n  provisioning:\n    next: [ready, failed]\n    timeout_s: 300\n\
             items:\n  - name: first\n  - name: second\nodd keys:\n  a/b: 1\n  'c~d': 2\n",
            Format::Yaml,
        )
        .expect("the fixture parses")
    }

    #[test]
    fn the_extension_decides_which_resolver_runs() {
        assert_eq!(format_for("spec.yaml"), Some(Format::Yaml));
        assert_eq!(format_for("spec.YML"), Some(Format::Yaml));
        assert_eq!(format_for("api.json"), Some(Format::Json));
        // Markdown and everything else keep the heading resolver, which is what
        // makes `#foo` unambiguous: the document settled it (ADR-0035).
        assert_eq!(format_for("DOC.md"), None);
        assert_eq!(format_for("src/lib.rs"), None);
        assert_eq!(format_for("Makefile"), None);
    }

    /// Proves `traces.structured_documents`, with the three tests beside it:
    /// the behaviour a consumer pins with `--require`, because ADR-0035 adds
    /// no rule code, command, config key, format or evidence kind that
    /// `facts` could otherwise show it by.
    #[test]
    fn both_spellings_address_the_same_node() {
        let d = doc();
        let by_pointer = resolve(&d, &parse_anchor("/states/provisioning").expect("pointer"));
        let by_path = resolve(&d, &parse_anchor("$.states.provisioning").expect("path"));
        assert!(by_pointer.is_some());
        assert_eq!(by_pointer, by_path, "one node, two ways of writing it");
        assert_eq!(
            fingerprint(by_pointer.expect("node")),
            fingerprint(by_path.expect("node")),
            "so the fingerprint cannot depend on which spelling was used"
        );
    }

    #[test]
    fn an_index_and_an_awkward_key_are_addressable() {
        let d = doc();
        let at = |a: &str| resolve(&d, &parse_anchor(a).expect(a)).cloned();
        assert_eq!(at("$.items[1].name"), Some(serde_json::json!("second")));
        assert_eq!(at("/items/1/name"), Some(serde_json::json!("second")));
        // RFC 6901 escapes: `~1` is a slash and `~0` a tilde, and the order
        // matters -- resolving `~0` first would turn `~01` into a slash.
        assert_eq!(at("/odd keys/a~1b"), Some(serde_json::json!(1)));
        assert_eq!(at("/odd keys/c~0d"), Some(serde_json::json!(2)));
        // A key a dotted path cannot spell is why the bracket form exists.
        assert_eq!(at("$['odd keys']['a/b']"), Some(serde_json::json!(1)));
        // A well-formed address to a node that is not there is `None`, not an
        // error: the anchor is fine and the document simply lacks it.
        assert_eq!(at("/states/retiring"), None);
        // A name into an array is a shape the document does not have.
        assert_eq!(at("/items/name"), None);
    }

    /// Proves `traces.structured_documents`: the singular subset is the whole
    /// of what an anchor may be, and refusing the rest by name is how ADR-0026
    /// stays true while ADR-0035 widens what a trace can point at.
    #[test]
    fn every_construct_that_could_match_more_than_one_node_is_refused_by_name() {
        for (anchor, fragment) in [
            ("$.states[*]", "wildcard"),
            ("$.*", "wildcard"),
            ("$..name", "descendant"),
            ("$.items[?(@.name)]", "filter"),
            ("$.items[0:2]", "slice"),
            ("$.items[0,1]", "union"),
            ("$.items[-1]", "counted from the end"),
        ] {
            let refusal = parse_anchor(anchor).expect_err(anchor);
            assert!(
                refusal.because.contains(fragment),
                "{anchor} must be refused for being a {fragment}: {}",
                refusal.because
            );
            assert!(
                !refusal.construct.is_empty(),
                "and must name what the author wrote: {anchor}"
            );
        }
    }

    #[test]
    fn an_anchor_that_announces_neither_spelling_is_refused_with_both() {
        let refusal = parse_anchor("states.provisioning").expect_err("no marker");
        assert!(
            refusal.because.contains("JSON Pointer") && refusal.because.contains("JSONPath"),
            "the refusal names both forms, since the author has to pick one: {}",
            refusal.because
        );
    }

    /// The fingerprint is of the value, not of the bytes (ADR-0035).
    ///
    /// Proves `traces.structured_documents`: an adopter pinning that name is
    /// promised drift on a changed value and silence on a reformatted file.
    #[test]
    fn layout_is_not_drift_and_a_value_is() {
        let fp = |text: &str| {
            let v = parse(text, Format::Yaml).expect("parses");
            fingerprint(resolve(&v, &parse_anchor("/a").expect("ptr")).expect("node"))
        };
        let base = fp("a:\n  x: 1\n  y: 2\n");
        assert_eq!(base, fp("a:\n    y: 2\n    x: 1\n"), "reorder and reindent");
        assert_eq!(base, fp("a:\n  # a note\n  x: 1\n  y: 2\n"), "a comment");
        assert_eq!(
            base,
            fp("{\"a\": {\"y\": 2, \"x\": 1}}"),
            "even another format"
        );
        assert_ne!(base, fp("a:\n  x: 1\n  y: 3\n"), "a changed value is drift");
        assert_ne!(base, fp("a:\n  x: 1\n  y: 2\n  z: 3\n"), "an added key is");
    }

    /// The canonical form depends on `serde_json`'s map being sorted, which is
    /// true unless `preserve_order` is enabled anywhere in the tree. Turning
    /// that on would silently change every stored fingerprint, so it is asserted
    /// rather than trusted.
    #[test]
    fn the_value_model_sorts_its_keys() {
        let v: serde_json::Value = serde_json::from_str(r#"{"b":1,"a":2}"#).expect("json");
        assert_eq!(
            v.to_string(),
            r#"{"a":2,"b":1}"#,
            "serde_json's map is ordered; `preserve_order` must not be enabled"
        );
    }

    #[test]
    fn a_near_miss_is_offered_in_the_spelling_the_author_used() {
        let d = doc();
        let ptr = addresses(&d, Spelling::Pointer, 5_000);
        assert!(ptr.contains("/states/provisioning"), "{ptr:?}");
        assert!(ptr.contains("/items/0/name"), "{ptr:?}");
        let path = addresses(&d, Spelling::Path, 5_000);
        assert!(path.contains("$.states.provisioning"), "{path:?}");
        assert!(path.contains("$.items[0].name"), "{path:?}");
        // A key that cannot go after a dot is rendered in brackets, so every
        // suggestion is one the author can paste back.
        assert!(path.contains("$['odd keys']"), "{path:?}");
        for a in &path {
            assert!(parse_anchor(a).is_ok(), "suggested `{a}` does not parse");
        }
        for a in &ptr {
            assert!(parse_anchor(a).is_ok(), "suggested `{a}` does not parse");
        }
    }

    #[test]
    fn the_address_walk_is_capped() {
        let mut wide = serde_json::Map::new();
        for i in 0..500 {
            wide.insert(format!("k{i}"), serde_json::json!(i));
        }
        let v = serde_json::Value::Object(wide);
        assert_eq!(addresses(&v, Spelling::Pointer, 10).len(), 10);
    }

    /// The parser's own limits are what stop a hostile document reaching the
    /// walk below, so they are asserted rather than assumed.
    ///
    /// A `traces` target is named by a task file, and a task file can arrive in
    /// a pull request from a fork (HLD 5.5). An alias bomb and a document nested
    /// past the parser's depth limit both fail to parse, so `addresses` and
    /// `resolve` never see them. Measured: both parsers stop at 128 levels, which
    /// is generous next to a spec catalogue and is why the refusal says so rather
    /// than only "it does not parse" (`A1-191`, audit).
    #[test]
    fn a_document_built_to_exhaust_a_parser_is_refused_before_it_is_walked() {
        let nest = |n: usize| {
            let mut s = String::from("x:\n");
            for i in 1..n {
                s.push_str(&"  ".repeat(i));
                s.push_str("x:\n");
            }
            s.push_str(&"  ".repeat(n));
            s.push_str("x: 1\n");
            s
        };
        assert!(
            parse(&nest(100), Format::Yaml).is_some(),
            "100 deep is fine"
        );
        assert!(parse(&nest(200), Format::Yaml).is_none(), "past the limit");
        assert!(
            parse(
                &format!("{}1{}", "[".repeat(200), "]".repeat(200)),
                Format::Json
            )
            .is_none(),
            "and JSON has the same guard"
        );

        // The billion-laughs shape: nine aliases nine times over, which expands
        // to more values than there is memory for if the parser lets it.
        let mut bomb = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x]\n");
        for i in 1..9 {
            let prev = format!("*a{}", i - 1);
            let row = std::iter::repeat_n(prev.as_str(), 9)
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(bomb, "a{i}: &a{i} [{row}]");
        }
        assert!(
            parse(&bomb, Format::Yaml).is_none(),
            "an alias bomb is refused"
        );

        // More than one document in a file is refused too, which is why the
        // message names it: a Kubernetes manifest is the commonest shape.
        assert!(parse("---\na: 1\n---\nb: 2\n", Format::Yaml).is_none());
    }

    /// `$` and `/` address the whole document, and that is a whole-file trace
    /// drift *can* watch (`A1-191`).
    ///
    /// Worth its own test because it is the one answer to the case `A1-297` left
    /// open: a markdown trace with no anchor cannot be stamped, and a structured
    /// one that names the root can.
    #[test]
    fn the_root_is_addressable_so_a_whole_document_can_be_watched() {
        let d = doc();
        let root = resolve(&d, &parse_anchor("$").expect("a bare $ is the root"));
        assert_eq!(root, Some(&d), "`$` is the document itself");
        assert!(
            fingerprint(root.expect("root")).len() == 16,
            "and it stamps"
        );

        // The value changing changes the stamp; layout still does not.
        let a = parse("b: 2\na: 1\n", Format::Yaml).expect("parses");
        let b = parse("a: 1\nb: 2\n", Format::Yaml).expect("parses");
        assert_eq!(fingerprint(&a), fingerprint(&b), "key order is not content");
        let c = parse("a: 1\nb: 3\n", Format::Yaml).expect("parses");
        assert_ne!(fingerprint(&a), fingerprint(&c), "a value is");
    }

    #[test]
    fn yaml_json_cannot_represent_is_refused_whole() {
        // A non-string key has no JSON equivalent, so the document does not
        // parse into the shared model and the caller reports that rather than
        // addressing the half it could read.
        assert!(parse("? [1, 2]\n: value\n", Format::Yaml).is_none());
        assert!(parse("not: [valid", Format::Yaml).is_none());
        assert!(parse("{\"unclosed\": 1", Format::Json).is_none());
    }
}
