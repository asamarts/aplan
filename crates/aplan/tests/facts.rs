//! The `facts.json` drift gate (HLD Section 16: "generate the counts, never
//! hand-write them", after a README claimed six invariants over a table of ten).
//!
//! `facts.json` is generated from the code by `aplan::facts()`. This test pins the
//! committed file to that output, so adding a rule, a format, a command, or an
//! evidence kind without regenerating turns CI red. It runs in the normal
//! `build + test` job, so no extra CI wiring is needed.

#[test]
fn facts_json_is_up_to_date() {
    let generated = format!("{}\n", aplan::facts());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../facts.json");
    // Fail with a clear cause if the file is missing, rather than an empty string
    // that the assertion below would report as a huge, misleading "stale" diff.
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert_eq!(
        on_disk, generated,
        "facts.json is stale. Regenerate it with `cargo run -p aplan -- facts > facts.json` \
         and commit the result."
    );
}
