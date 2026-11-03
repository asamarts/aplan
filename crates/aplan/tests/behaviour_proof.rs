//! Every named behaviour names the tests that prove it (`A1-211`).
//!
//! `aplan facts` publishes eighteen behaviours and `--require <name>` answers
//! from that list. The answer was structural: the name was in `ALL`, so the
//! build "had" the behaviour. Nothing connected the name to code, and nothing
//! connected it to a test, so a behaviour that quietly stopped being true went
//! on being asserted by name -- which is the one failure a consumer pins
//! `--require` to catch. Fourteen of the eighteen were named in no test source
//! at all when this was written.
//!
//! [`Behaviour::proof`] closes the near half: the field is not optional, so a
//! new behaviour cannot be added without saying what would fail if it broke.
//! This file closes the far half, by refusing a name that is not a test in this
//! workspace, and by requiring the test to name the behaviour back. The back
//! reference is what a reader at the test site sees: it says this test is
//! load-bearing for a promise a consumer can pin, so weakening it is not a
//! local decision.
//!
//! What this cannot check is whether the named test really exercises the
//! behaviour. Nothing mechanical can: that is the same trust `aplan`'s own
//! `test:` citations ask for, and the answer is the same one -- name it, so the
//! claim is reviewable rather than absent. Each pairing here was checked by
//! breaking the behaviour and watching the named test fail (`A1-211`'s audit).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The workspace, from this crate's manifest.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

/// Every Rust source under `crates/`, as (path relative to the workspace, text).
///
/// The proofs are spread across six crates: a behaviour is a promise about what
/// the tool does, and the test that pins it lives beside the code that does it,
/// not in one directory. Scanning `crates/aplan/tests` alone would find four of
/// the eighteen and pass by reading almost nothing.
fn sources() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .expect("reading a source dir")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                // `target` can appear per-crate under some cargo settings, and
                // holds generated copies that would double every name.
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path
                    .strip_prefix(&root)
                    .expect("under the workspace")
                    .to_string_lossy()
                    .into_owned();
                out.push((
                    rel,
                    std::fs::read_to_string(&path).expect("reading a source"),
                ));
            }
        }
    }
    out.sort();
    out
}

/// One test function: where its text begins (including the doc comment above
/// the attribute) and where it ends.
struct Span {
    file: String,
    start: usize,
    end: usize,
}

/// The `#[test]` functions in one source, each spanning its doc comment, its
/// attributes, and its body -- and nothing after it.
///
/// Written line-wise rather than as one regex-shaped match because the
/// attribute and the `fn` are not always adjacent: a `#[test]` may be followed
/// by `#[ignore]` or a `#[cfg(...)]` before the signature, and a scan that
/// required adjacency would silently miss those and report the test as absent.
///
/// The end of a span is the function's own closing brace, found as the first
/// line holding `}` at the signature's indentation, which is what `rustfmt`
/// writes and `cargo fmt --check` enforces here. Running the span to the *next*
/// test instead is what the first version did, and for the last test in a file
/// that meant running it to the end of the file: the gate below then accepted a
/// behaviour name written five hundred lines away in unrelated code, which is
/// the entire value of the back reference gone (`A1-211`, audit).
fn tests_in(file: &str, text: &str) -> Vec<(String, Span)> {
    let lines: Vec<(usize, &str)> = {
        let mut v = Vec::new();
        let mut at = 0;
        for line in text.split_inclusive('\n') {
            v.push((at, line));
            at += line.len();
        }
        v
    };
    let is_attr = |s: &str| s.trim_start().starts_with("#[");
    let is_doc = |s: &str| s.trim_start().starts_with("//");
    // name, start of its text, the `#[test]` line, end of its body.
    let mut found: Vec<(String, usize, usize, usize)> = Vec::new();
    for (i, (_, line)) in lines.iter().enumerate() {
        let t = line.trim();
        if t != "#[test]" && t != "#[tokio::test]" {
            continue;
        }
        // Forward to the signature, over any further attributes.
        let mut j = i + 1;
        let mut name = None;
        while j < lines.len() {
            let t = lines[j].1.trim();
            if t.is_empty() || is_attr(t) || is_doc(t) {
                j += 1;
                continue;
            }
            if let Some(rest) = t
                .strip_prefix("fn ")
                .or_else(|| t.strip_prefix("async fn "))
            {
                name = rest.split('(').next().map(|s| s.trim().to_string());
            }
            break;
        }
        let Some(name) = name else { continue };
        // The signature's indentation, and so the indentation `rustfmt` writes
        // its closing brace at.
        let signature = lines[j].1;
        let indent = &signature[..signature.len() - signature.trim_start().len()];
        let closer = format!("{indent}}}");
        let body_ends = lines[j + 1..]
            .iter()
            .find(|(_, l)| l.trim_end() == closer)
            .map_or(text.len(), |(at, l)| at + l.len());
        // Back over the doc comment and any attributes above, which is where an
        // author writes what the test is for -- and so where the behaviour name
        // has to be allowed to sit.
        let mut k = i;
        while k > 0 {
            let t = lines[k - 1].1.trim();
            if is_doc(t) || is_attr(t) {
                k -= 1;
            } else {
                break;
            }
        }
        found.push((name, lines[k].0, i, body_ends));
    }
    // A span ends at the test's own closing brace, and never runs into the next
    // test's text even if that brace was not where it was expected.
    let mut out = Vec::new();
    for (n, (name, start, _, body_ends)) in found.iter().enumerate() {
        let next = found.get(n + 1).map_or(text.len(), |next| next.1);
        let end = (*body_ends).min(next);
        out.push((
            name.clone(),
            Span {
                file: file.to_string(),
                start: *start,
                end,
            },
        ));
    }
    out
}

/// Cargo prefixes tests in a consolidated integration source with the module
/// name declared by the one harness. The harness census requires that name to
/// equal the source stem, so this derives the registered name without running a
/// nested test build (`A1-630`).
fn integration_module(file: &str) -> Option<String> {
    let relative = Path::new(file);
    let tests = relative.parent()?;
    if tests.file_name()? != "tests" || relative.extension()? != "rs" {
        return None;
    }
    let package = tests.parent()?;
    let manifest =
        std::fs::read_to_string(workspace_root().join(package).join("Cargo.toml")).ok()?;
    if !manifest
        .lines()
        .any(|line| line.trim() == "autotests = false")
    {
        return None;
    }
    relative
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

/// Every `#[test]` in the workspace, by name. A name defined in more than one
/// file lands in the `Vec`, and a proof citing it is ambiguous rather than
/// resolved.
fn every_test() -> BTreeMap<String, Vec<(Span, String)>> {
    let mut out: BTreeMap<String, Vec<(Span, String)>> = BTreeMap::new();
    for (file, text) in sources() {
        let module = integration_module(&file);
        for (name, span) in tests_in(&file, &text) {
            let body = text[span.start..span.end].to_string();
            let registered = module
                .as_ref()
                .map_or(name.clone(), |module| format!("{module}::{name}"));
            out.entry(registered).or_default().push((span, body));
        }
    }
    assert!(
        out.len() > 1_000,
        "the scan found {} tests, so it is reading almost nothing and would pass \
         whatever the proofs said",
        out.len()
    );
    out
}

#[test]
fn the_scan_uses_registered_module_names_for_consolidated_integration_tests() {
    let tests = every_test();
    assert!(tests.contains_key(
        "behaviour_proof::the_scan_uses_registered_module_names_for_consolidated_integration_tests"
    ));
    assert!(
        !tests.contains_key(
            "the_scan_uses_registered_module_names_for_consolidated_integration_tests"
        )
    );
    assert!(
        tests.contains_key(
            "tui_portability::external_editor_handoff_restores_modes_and_refreshes_identity"
        ),
        "the portable source must keep the module prefix of the consolidated target"
    );
}

/// **Every behaviour names tests that exist, and they name it back.**
#[test]
fn every_behaviour_is_proved_by_a_test_that_names_it() {
    let tests = every_test();
    let mut unproved = Vec::new();
    for b in aplan_core::behaviour::ALL {
        if b.proof.is_empty() {
            unproved.push(format!("{}: names no test", b.name));
            continue;
        }
        for proof in b.proof {
            match tests.get(*proof) {
                None => unproved.push(format!(
                    "{}: names `{proof}`, which is not a `#[test]` in this workspace",
                    b.name
                )),
                Some(found) if found.len() > 1 => {
                    let files: Vec<_> = found.iter().map(|(s, _)| s.file.as_str()).collect();
                    unproved.push(format!(
                        "{}: names `{proof}`, which is defined in {} places ({}), so the \
                         citation does not say which",
                        b.name,
                        found.len(),
                        files.join(", ")
                    ));
                }
                Some(found) => {
                    let (span, body) = &found[0];
                    if !body.contains(b.name) {
                        unproved.push(format!(
                            "{}: names `{proof}` ({}), which does not name the behaviour back",
                            b.name, span.file
                        ));
                    }
                }
            }
        }
    }
    assert!(
        unproved.is_empty(),
        "a behaviour `--require` answers for is only as good as what would fail if it \
         broke:\n  {}",
        unproved.join("\n  ")
    );
}

/// **The scan finds tests wherever the proofs live**, which is the assumption
/// the gate above rests on.
///
/// A behaviour is a promise about what the tool does, and the test that pins it
/// lives beside the code that does it: nineteen of the thirty-nine proofs are
/// in `crates/aplan/tests`, and the other twenty are spread across five further
/// crates. The first version of `sources` read the one directory, so those
/// twenty would have been reported as tests that do not exist -- loud rather
/// than silent, but only until someone "fixed" it by pointing the proofs at
/// whatever that directory happened to contain.
#[test]
fn the_scan_reaches_every_crate_that_holds_a_proof() {
    let tests = every_test();
    let mut crates: std::collections::BTreeSet<&str> = std::collections::BTreeSet::default();
    for b in aplan_core::behaviour::ALL {
        for proof in b.proof {
            if let Some(found) = tests.get(*proof) {
                let file = found[0].0.file.as_str();
                crates.insert(file.split('/').nth(1).unwrap_or(file));
            }
        }
    }
    assert!(
        crates.len() > 1 && crates.iter().any(|c| *c != "aplan"),
        "every proof resolves inside {crates:?}, so the scan is reading one crate's tests \
         and a behaviour proved beside its own code could not be cited at all"
    );
}

/// **A test that is defined twice cannot be cited**, and the gate says so
/// rather than silently checking whichever copy it found first.
#[test]
fn an_ambiguous_citation_is_reported_rather_than_resolved() {
    let one = "#[test]\nfn shared_name() { let _ = \"some.behaviour\"; }\n";
    let a = tests_in("a.rs", one);
    let b = tests_in("b.rs", one);
    assert_eq!(a.len(), 1);
    let mut map: BTreeMap<String, Vec<(Span, String)>> = BTreeMap::new();
    for (name, span) in a.into_iter().chain(b) {
        let body = one[span.start..span.end].to_string();
        map.entry(name).or_default().push((span, body));
    }
    assert_eq!(
        map["shared_name"].len(),
        2,
        "two files defining the same test name must collide here, or the gate above \
         would check one of them and report nothing about the other"
    );
}

/// **The parser sees the shapes this workspace actually writes**: a doc comment
/// above the attribute, a further attribute between the attribute and the
/// signature, and an `async` signature.
#[test]
fn a_test_is_found_through_the_attributes_between_it_and_its_name() {
    let text = "\
/// A doc comment naming some.behaviour.
#[test]
#[ignore = \"slow\"]
fn plain() {}

#[tokio::test]
async fn asynchronous() {}
";
    let found = tests_in("x.rs", text);
    let names: Vec<_> = found.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["plain", "asynchronous"],
        "an attribute between `#[test]` and `fn` must not hide the test: it would be \
         reported as a proof that does not exist"
    );
    let (_, span) = &found[0];
    assert!(
        text[span.start..span.end].contains("some.behaviour"),
        "the doc comment above the attribute is where the behaviour name is written, \
         so it has to be inside the span the gate searches"
    );
}

/// **A behaviour name written after the test does not count as the test naming
/// it.**
///
/// The hole the first version had, and it was not hypothetical: the proof for
/// `traces.table_rows` is the last `#[test]` in `anchor.rs`, whose span then ran
/// to the end of a file holding several hundred further lines. Deleting the back
/// reference from the test and writing the name in an unrelated comment below it
/// left the gate green (`A1-211`, audit).
#[test]
fn a_mention_after_the_test_is_outside_its_span() {
    let text = "\
mod tests {
    #[test]
    fn the_proof() {
        assert!(true);
    }
}

// An unrelated remark mentioning some.behaviour in passing.
fn something_else() {}
";
    let found = tests_in("x.rs", text);
    assert_eq!(found.len(), 1, "one test");
    let (_, span) = &found[0];
    let body = &text[span.start..span.end];
    assert!(
        body.contains("assert!(true)"),
        "the span must hold the test it is for: {body:?}"
    );
    assert!(
        !body.contains("some.behaviour"),
        "a name written after the closing brace is not this test naming it: {body:?}"
    );
}

/// **No behaviour name is unique only by accident.**
///
/// Two guards, both about the `contains` the gate above does. A duplicate name
/// makes `--require` ambiguous at the consumer's end, and a name that is a
/// substring of another makes the back reference ambiguous at this end: a test
/// naming `a.b_c` also contains `a.b`, so `a.b` could cite it and pass while
/// naming nothing.
#[test]
fn a_behaviour_name_is_neither_repeated_nor_contained_in_another() {
    let names: Vec<&str> = aplan_core::behaviour::ALL.iter().map(|b| b.name).collect();
    let mut wrong = Vec::new();
    for (i, a) in names.iter().enumerate() {
        for (j, b) in names.iter().enumerate() {
            if i == j {
                continue;
            }
            if a == b {
                wrong.push(format!("`{a}` is listed twice"));
            } else if b.contains(a) {
                wrong.push(format!(
                    "`{a}` is a substring of `{b}`, so a test naming `{b}` reads as naming `{a}`"
                ));
            }
        }
    }
    wrong.sort();
    wrong.dedup();
    assert!(
        wrong.is_empty(),
        "behaviour names:\n  {}",
        wrong.join("\n  ")
    );
}
