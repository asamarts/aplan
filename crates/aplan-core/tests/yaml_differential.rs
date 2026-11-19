//! Test-only differential oracle for the aplan YAML boundary.
//!
//! The baseline remains a dev-dependency only. Normal product graphs contain
//! `ayaml` alone; this test compares the readers the product uses,
//! `aplan_core::yaml`, against the pinned bridge that `A2-009` replaced.

use std::fs;
use std::path::{Path, PathBuf};

use yaml_compat::Value;

fn walk(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_dir() {
            if !matches!(name, "target" | ".git" | "dist" | "node_modules" | "vendor") {
                walk(&path, out);
            }
            continue;
        }
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("yml" | "yaml")
        ) {
            out.push(path);
        }
    }
}

fn frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some(&rest[..end])
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
}

fn documents(root: &Path) -> Vec<(PathBuf, String)> {
    let mut paths = Vec::new();
    walk(root, &mut paths);
    let mut out = Vec::new();
    for path in paths {
        if let Ok(text) = fs::read_to_string(&path) {
            out.push((path, text));
        }
    }
    out.extend(task_frontmatters(root));
    out
}

fn task_frontmatters(root: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let tasks = root.join("aplan/tasks");
    if let Ok(entries) = fs::read_dir(tasks) {
        let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            if path.extension().and_then(|ext| ext.to_str()) == Some("md")
                && let Ok(text) = fs::read_to_string(&path)
                && let Some(front) = frontmatter(&text)
            {
                out.push((path, front.to_owned()));
            }
        }
    }
    out
}

fn normalized(value: &Value) -> serde_json::Value {
    serde_json::to_value(value).expect("ayaml value is JSON-serializable")
}

fn baseline_normalized(value: &yaml_serde_baseline::Value) -> serde_json::Value {
    serde_json::to_value(value).expect("baseline value is JSON-serializable")
}

#[test]
fn every_aplan_yaml_document_matches_the_pinned_bridge() {
    let docs = documents(workspace_root());
    // A floor, not a target: the public tree held 201 documents (30 YAML files and 171 task
    // front matters) when this ran first. It catches a walk that went to the wrong root or
    // found nothing, which would make every comparison below pass vacuously.
    assert!(
        docs.len() > 150,
        "expected a substantial aplan corpus, found {} document(s)",
        docs.len()
    );

    let mut compared = 0usize;
    let mut both_error = 0usize;
    for (path, text) in docs {
        // The product reads every document through `aplan_core::yaml`, so the corpus is read the
        // same way (`A2-179`).
        let ayaml = aplan_core::yaml::from_str::<Value>(&text);
        let baseline = yaml_serde_baseline::from_str::<yaml_serde_baseline::Value>(&text);
        match (ayaml, baseline) {
            (Ok(ayaml), Ok(baseline)) => {
                let expected = baseline_normalized(&baseline);
                assert_eq!(
                    normalized(&ayaml),
                    expected,
                    "normalized values differ for {}",
                    path.display()
                );
                // aplan's own formats use the default reader; on this corpus it
                // must read what the compatibility reader and the bridge read.
                let default =
                    aplan_core::yaml::from_str_strict::<Value>(&text).unwrap_or_else(|error| {
                        panic!("the default reader refused {}: {error}", path.display())
                    });
                assert_eq!(
                    normalized(&default),
                    expected,
                    "the default reader differs for {}",
                    path.display()
                );

                let emitted = yaml_compat::to_string(&ayaml).expect("ayaml emission");
                let reparsed =
                    yaml_serde_baseline::from_str::<yaml_serde_baseline::Value>(&emitted)
                        .unwrap_or_else(|error| {
                            panic!(
                                "bridge cannot reparse ayaml output for {}: {error}\n{emitted}",
                                path.display()
                            )
                        });
                assert_eq!(
                    baseline_normalized(&reparsed),
                    expected,
                    "ayaml emission changed semantics for {}\n{emitted}",
                    path.display()
                );
                compared += 1;
            }
            (Err(ayaml), Err(baseline)) => {
                let ayaml_location = ayaml.location().map(|loc| (loc.line(), loc.column()));
                let baseline_location = baseline.location().map(|loc| (loc.line(), loc.column()));
                assert_eq!(
                    ayaml_location.is_some(),
                    baseline_location.is_some(),
                    "one parser lost the error location for {}: ayaml={ayaml:?} baseline={baseline:?}",
                    path.display()
                );
                both_error += 1;
            }
            (Ok(_), Err(baseline)) => panic!(
                "ayaml accepted but the bridge rejected {}: {baseline}",
                path.display()
            ),
            (Err(ayaml), Ok(_)) => panic!(
                "the bridge accepted but ayaml rejected {}: {ayaml}",
                path.display()
            ),
        }
    }
    assert!(compared > 0, "no documents were compared");
    eprintln!("differential documents: {compared} equal, {both_error} both rejected");
}

/// The loader reads front matter twice: as the `serde_json::Value` its schema
/// validates, and as a typed `Task`. Both reads must agree with the bridge for
/// every task in this plan, so a typed difference cannot hide behind equal
/// generic values.
#[test]
fn every_task_front_matter_decodes_to_the_same_types_under_both_parsers() {
    let tasks = task_frontmatters(workspace_root());
    // A floor, not a target, for the same reason as the corpus floor above.
    assert!(
        tasks.len() > 150,
        "expected a substantial task corpus, found {} task(s)",
        tasks.len()
    );
    for (path, front) in &tasks {
        let ayaml = aplan_core::yaml::from_str::<serde_json::Value>(front)
            .unwrap_or_else(|error| panic!("ayaml JSON read of {}: {error}", path.display()));
        let bridge = yaml_serde_baseline::from_str::<serde_json::Value>(front)
            .unwrap_or_else(|error| panic!("bridge JSON read of {}: {error}", path.display()));
        assert_eq!(ayaml, bridge, "JSON values differ for {}", path.display());

        let ayaml = aplan_core::yaml::from_str::<aplan_core::Task>(front)
            .unwrap_or_else(|error| panic!("ayaml typed read of {}: {error}", path.display()));
        let bridge = yaml_serde_baseline::from_str::<aplan_core::Task>(front)
            .unwrap_or_else(|error| panic!("bridge typed read of {}: {error}", path.display()));
        assert_eq!(ayaml, bridge, "typed tasks differ for {}", path.display());
    }
}

/// What both parsers did with one input, reduced to something comparable.
#[derive(Debug, PartialEq)]
enum Outcome {
    Read(serde_json::Value),
    Refused,
}

fn outcome<T, E>(result: Result<T, E>, normalize: impl Fn(&T) -> serde_json::Value) -> Outcome {
    result.map_or(Outcome::Refused, |value| Outcome::Read(normalize(&value)))
}

/// JSON cannot hold every YAML value (a sequence as a key, say). Both sides map
/// such a value to the same marker, so the comparison still requires that both
/// parsers read it, and compares the value itself wherever JSON can express it.
fn representable(value: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(value)
        .unwrap_or_else(|_| serde_json::Value::String("<not representable as JSON>".to_owned()))
}

fn nested_flow(depth: usize) -> String {
    format!("{}0{}", "[".repeat(depth), "]".repeat(depth))
}

fn alias_bomb() -> String {
    use std::fmt::Write as _;
    let mut text = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x]\n");
    for level in 1..9 {
        let previous = format!("*a{}", level - 1);
        let row = [previous.as_str(); 9].join(", ");
        writeln!(text, "a{level}: &a{level} [{row}]").expect("write to a String");
    }
    text
}

/// Hostile and degenerate inputs, each read by both parsers as a generic
/// `Value`, as the loader's `serde_json::Value` and as a typed `Task`.
fn hostile_inputs() -> Vec<(&'static str, String)> {
    let mut inputs = tagged_inputs();
    inputs.extend([
        ("empty document", String::new()),
        ("comment only", "# nothing here\n".to_owned()),
        ("explicit empty document", "---\n...\n".to_owned()),
        ("null document", "~\n".to_owned()),
        ("duplicate key", "a: 1\na: 2\n".to_owned()),
        ("nested duplicate key", "a:\n  b: 1\n  b: 2\n".to_owned()),
        ("second document", "a: 1\n---\nb: 2\n".to_owned()),
        ("unterminated flow sequence", "a: [1, 2\n".to_owned()),
        ("unindented flow continuation", "a: [1,\n2]\n".to_owned()),
        ("tab indentation", "a:\n\tb: 1\n".to_owned()),
        ("undefined alias", "a: *missing\n".to_owned()),
        ("self-referencing alias", "a: &a [*a]\n".to_owned()),
        ("byte order mark", "\u{feff}a: 1\n".to_owned()),
        (
            "byte order mark before a document start",
            "\u{feff}---\na: 1\n".to_owned(),
        ),
        (
            "question mark starting a flow scalar",
            "a: [?x]\n".to_owned(),
        ),
        (
            "anchor name with a colon",
            "a: &x:y 1\nb: *x:y\n".to_owned(),
        ),
        (
            "enum as a single-entry mapping",
            "id: T-1\ntitle: t\nstatus: {todo: ~}\nphase: 1\n".to_owned(),
        ),
        ("control character", "a: \"\u{1}\"\n".to_owned()),
        ("integer past u64", "a: 18446744073709551616\n".to_owned()),
        ("non-string key", "? [a, b]\n: 1\n".to_owned()),
        (
            "literal merge key",
            "base: &b {x: 1}\nderived:\n  <<: *b\n  y: 2\n".to_owned(),
        ),
        ("redefined anchor", "a: &x 1\nb: &x 2\nc: *x\n".to_owned()),
        ("long scalar", format!("a: {}\n", "x".repeat(64 * 1024))),
        ("deep flow nesting", nested_flow(10_000)),
        ("alias bomb", alias_bomb()),
    ]);
    inputs
}

/// The hostile inputs that carry a tag, where the bridge's tag rule applies (`A2-169`,
/// `A2-179`), and one spelled like a tag that is not one.
fn tagged_inputs() -> Vec<(&'static str, String)> {
    vec![
        ("undefined tag handle", "a: !e!x 1\n".to_owned()),
        (
            "declared tag handle",
            "%TAG !e! tag:example.com,2000:\n---\na: !e!x 1\n".to_owned(),
        ),
        ("local tag", "a: !custom 1\n".to_owned()),
        ("verbatim local tag", "a: !<!custom> 1\n".to_owned()),
        (
            "declared primary handle",
            "%TAG ! tag:example.com,2000:\n---\na: !x 1\n".to_owned(),
        ),
        ("timestamp tag", "a: !!timestamp 2026-01-01\n".to_owned()),
        ("binary tag", "a: !!binary aGVsbG8=\n".to_owned()),
        (
            "verbatim global tag",
            "a: !<tag:example.com,2000:x> hello\n".to_owned(),
        ),
        ("global tag on a mapping", "a: !!set {x, y}\n".to_owned()),
        ("global tag on a sequence", "a: !!binary [1]\n".to_owned()),
        ("non-specific tag", "a: ! 1\n".to_owned()),
        ("integer tag on a leading zero", "a: !!int 017\n".to_owned()),
        ("null tag with no value", "a: !!null\n".to_owned()),
        (
            "global tag on a key",
            "!!timestamp 2026-01-01: x\n".to_owned(),
        ),
        (
            "task title with a global tag",
            "id: T-1\ntitle: !!timestamp 2026-01-01\nstatus: todo\nphase: 1\n".to_owned(),
        ),
        (
            "task status with a global tag",
            "id: T-1\ntitle: t\nstatus: !<tag:example.com,2000:s> todo\nphase: 1\n".to_owned(),
        ),
        (
            "task title with a local tag",
            "id: T-1\ntitle: !custom t\nstatus: todo\nphase: 1\n".to_owned(),
        ),
        (
            "task estimate with a global tag",
            "id: T-1\ntitle: t\nstatus: todo\nphase: 1\neffort_days: !<tag:example.com,2000:n> 2\n"
                .to_owned(),
        ),
        (
            "task estimate with a string tag",
            "id: T-1\ntitle: t\nstatus: todo\nphase: 1\neffort_days: !!str 2\n".to_owned(),
        ),
        (
            "task phase with a float tag",
            "id: T-1\ntitle: t\nstatus: todo\nphase: !!float 1\n".to_owned(),
        ),
        (
            "task estimate with a core tag on a quoted scalar",
            "id: T-1\ntitle: t\nstatus: todo\nphase: 1\neffort_days: !!int \"2\"\n".to_owned(),
        ),
        (
            "task estimate with a float tag on a quoted scalar",
            "id: T-1\ntitle: t\nstatus: todo\nphase: 1\neffort_days: !!float \"2.5\"\n".to_owned(),
        ),
        (
            "task title with the non-specific tag",
            "id: T-1\ntitle: ! t\nstatus: todo\nphase: 1\n".to_owned(),
        ),
        (
            "task status with a global tag on a mapping key",
            "id: T-1\ntitle: t\nstatus: {\"<x>\": todo}\nphase: 1\n".to_owned(),
        ),
    ]
}

/// Where the parsers knowingly part, as (input, read, why). Some let the product
/// read a value the bridge refused where YAML's core schema defines it: a core tag
/// on a quoted scalar, `!!int` on a number with a leading zero, and `!!null` with
/// no value. The others are refusals where the bridge read a value, or a generic
/// value no product read turns into output.
const KNOWN_DIVERGENCES: &[(&str, &str, &str)] = &[
    (
        "duplicate key",
        "default-reader JSON",
        "the default reader, used for aplan's own formats, refuses a duplicate key that the \
         bridge's JSON read resolved to the last value",
    ),
    (
        "nested duplicate key",
        "default-reader JSON",
        "the same refusal, inside a nested mapping",
    ),
    (
        "unindented flow continuation",
        "default-reader JSON",
        "the default reader requires a flow collection's continuation lines to be indented, as \
         YAML does; the compatibility reader still accepts them as libyaml did",
    ),
    (
        "byte order mark before a document start",
        "default-reader JSON",
        "the default reader takes a byte order mark before `---`, which YAML allows at the start \
         of a stream and the bridge refused (A2-009, recorded by A2-179)",
    ),
    (
        "question mark starting a flow scalar",
        "default-reader JSON",
        "in a flow collection `?x` is the plain scalar `?x`, as YAML 1.2 defines it; libyaml read \
         it as a key with no value, and the compatibility reader still does (A2-009, recorded by \
         A2-179)",
    ),
    (
        "anchor name with a colon",
        "default-reader JSON",
        "YAML 1.2 lets an anchor's name hold a colon, which libyaml refused (A2-009, recorded by \
         A2-179)",
    ),
    (
        "enum as a single-entry mapping",
        "Task",
        "`ayaml` reads an enum written as a mapping with one key, as the variant that key names; \
         the bridge took only a tag or a string. A task's loader validates its front matter as \
         JSON first, where the schema refuses a mapping for `status`, so no task loads this way; \
         `.aplan.yml`'s `id_strategy: {ulid: ~}` does (A2-009, recorded by A2-179)",
    ),
    (
        "task estimate with a string tag",
        "Task",
        "`!!str 2` is the string `2`, as YAML defines it, which a number field refuses; the \
         bridge's typed read ignored a core tag and parsed the text (A2-169)",
    ),
    (
        "task phase with a float tag",
        "Task",
        "`!!float 1` is the float 1.0, which an integer field refuses; the bridge's typed read \
         parsed the text as an integer, ignoring the tag (A2-169)",
    ),
    (
        "task estimate with a core tag on a quoted scalar",
        "Task",
        "`!!int \"2\"` is the integer 2, as YAML defines it; the bridge's numeric reads took only \
         a plain scalar, so it refused every quoted one whatever its tag. Refusing it would copy \
         that quirk rather than a rule (A2-169, A2-179)",
    ),
    (
        "task estimate with a float tag on a quoted scalar",
        "Task",
        "`!!float \"2.5\"` is the float 2.5, refused by the bridge for the same reason (A2-179)",
    ),
    (
        "integer tag on a leading zero",
        "Value",
        "YAML 1.2's core schema reads `!!int 017` as 17; the bridge, reading YAML 1.1's octal \
         rules, refused it (A2-179)",
    ),
    (
        "integer tag on a leading zero",
        "JSON",
        "the same reading, as JSON",
    ),
    (
        "integer tag on a leading zero",
        "default-reader JSON",
        "the same reading, by the default reader",
    ),
    (
        "null tag with no value",
        "Value",
        "an empty scalar tagged `!!null` is null, as YAML defines it; the bridge refused a tag \
         with no value (A2-179)",
    ),
    (
        "null tag with no value",
        "JSON",
        "the same reading, as JSON",
    ),
    (
        "null tag with no value",
        "default-reader JSON",
        "the same reading, by the default reader",
    ),
    (
        "non-specific tag",
        "Value",
        "both keep the non-specific tag `!` in a generic value; YAML makes the scalar beneath it \
         a string, as `ayaml` does, where the bridge resolved it as a number. Read as JSON, both \
         refuse it (A2-179)",
    ),
    (
        "task title with the non-specific tag",
        "Value",
        "the same tag on a task's title. Both refuse it as JSON, the read the loader validates \
         first, so neither loads the task",
    ),
    (
        "integer past u64",
        "Value",
        "ayaml's generic value holds a 128-bit integer the bridge refused; read as JSON or as a \
         task, both still refuse it, and of the product reads that keep a generic value only the \
         KEP importer's `kep-number` turns one into output, and it refuses it itself (A2-166)",
    ),
];

#[test]
fn hostile_inputs_meet_the_same_outcome_under_both_parsers() {
    let mut divergent = Vec::new();
    let mut details = Vec::new();
    for (name, text) in hostile_inputs() {
        let generic = (
            outcome(aplan_core::yaml::from_str::<Value>(&text), representable),
            outcome(
                yaml_serde_baseline::from_str::<yaml_serde_baseline::Value>(&text),
                representable,
            ),
        );
        let json = (
            outcome(
                aplan_core::yaml::from_str::<serde_json::Value>(&text),
                Clone::clone,
            ),
            outcome(
                yaml_serde_baseline::from_str::<serde_json::Value>(&text),
                Clone::clone,
            ),
        );
        let typed = (
            outcome(
                aplan_core::yaml::from_str::<aplan_core::Task>(&text),
                |task| serde_json::to_value(task).expect("task serializes"),
            ),
            outcome(
                yaml_serde_baseline::from_str::<aplan_core::Task>(&text),
                |task| serde_json::to_value(task).expect("task serializes"),
            ),
        );
        // The default reader, which aplan's own formats use (captures read it as
        // JSON before their schema check), against the same bridge.
        let default_json = (
            outcome(
                aplan_core::yaml::from_str_strict::<serde_json::Value>(&text),
                Clone::clone,
            ),
            outcome(
                yaml_serde_baseline::from_str::<serde_json::Value>(&text),
                Clone::clone,
            ),
        );
        for (target, (ayaml, bridge)) in [
            ("Value", generic),
            ("JSON", json),
            ("Task", typed),
            ("default-reader JSON", default_json),
        ] {
            if ayaml != bridge {
                divergent.push((name, target));
                details.push(format!(
                    "{name} as {target}: ayaml={ayaml:?} bridge={bridge:?}"
                ));
            }
        }
    }
    let divergent: std::collections::BTreeSet<(&str, &str)> = divergent.into_iter().collect();
    let known: std::collections::BTreeSet<(&str, &str)> = KNOWN_DIVERGENCES
        .iter()
        .map(|(name, target, _)| (*name, *target))
        .collect();
    // Exact, both ways: a new divergence is a compatibility regression, and a
    // known one that disappeared leaves a stale reason above.
    assert_eq!(
        divergent,
        known,
        "the parsers' divergences changed:\n{}",
        details.join("\n")
    );
}
