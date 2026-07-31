//! Docs-vs-facts gate: the rules and commands the tool actually has (from
//! `aplan::facts()`, the generated source of truth) must be documented, so a new
//! rule or command cannot ship undocumented, the "six invariants over a table of
//! ten" class of drift this project was burned by (HLD Section 16).
//!
//! This binds the *lists* to the code. Prose *counts* ("eleven invariants") stay
//! hand-written until `gen-roadmap` (v0.5) generates the doc sections outright.

use serde_json::Value;

fn repo_file(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn every_rule_and_command_in_facts_is_documented() {
    let facts: Value = serde_json::from_str(&aplan::facts()).expect("facts() is JSON");
    let readme = repo_file("README.md");
    let hld = repo_file("HLD.md");

    let mut missing = Vec::new();

    // Every rule code must appear in the HLD rule catalog (the authoritative
    // list). The README describes rules selectively, so it is not required to
    // name every code.
    for rule in facts["rules"].as_array().expect("rules array") {
        let code = rule["code"].as_str().unwrap();
        if !hld.contains(code) {
            missing.push(format!("rule {code} is not documented in HLD.md"));
        }
    }

    // Every command must appear in both the README command list and the HLD CLI
    // surface table, `aplan <name>`.
    for cmd in facts["commands"].as_array().expect("commands array") {
        let needle = format!("aplan {}", cmd["name"].as_str().unwrap());
        if !readme.contains(&needle) {
            missing.push(format!("command `{needle}` is not in README.md"));
        }
        if !hld.contains(&needle) {
            missing.push(format!("command `{needle}` is not in HLD.md"));
        }
    }

    assert!(
        missing.is_empty(),
        "docs have drifted from facts.json (run `aplan facts` for the truth):\n  {}",
        missing.join("\n  ")
    );
}
