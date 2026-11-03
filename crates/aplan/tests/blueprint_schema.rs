//! The committed blueprint schema is generated from the parser types (`A1-385`).
//!
//! Keeping this in the product integration runner makes the artifact part of the same ordinary
//! test inventory as every other published contract. `cargo xtask gen-schema --check` remains the
//! write/check interface; this test proves what that gate compares.

/// Reverting either the parser shape or its committed artifact makes this fail.
#[test]
fn committed_blueprint_schema_is_derived_from_the_parser_types() {
    let mut generated = serde_json::to_string_pretty(&aplan_interop::blueprint::derived_schema())
        .expect("schema is JSON");
    generated.push('\n');

    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1/blueprint.json");
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    assert_eq!(
        committed, generated,
        "the blueprint schema drifted; run `cargo xtask gen-schema` and commit the result"
    );
}
