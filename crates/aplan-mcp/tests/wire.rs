//! The tool surface as an agent actually sees it.
//!
//! `tools::TOOLS` pins the names; this pins what goes on the wire, including
//! each tool's JSON Schema. The surface is context an agent pays for on every
//! call, and `aplan_close`'s schema is the contract that says evidence is
//! required and no self-assessment is accepted, so both are snapshot-checked
//! rather than trusted to stay put.

use aplan_mcp::server::wire_tools;

#[test]
fn the_wire_surface_is_exactly_four_tools() {
    let tools = wire_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        vec!["aplan_ready", "aplan_why", "aplan_check", "aplan_close"],
        "the MCP surface changed; growing it is a decision, not a refactor"
    );
    for t in &tools {
        assert!(t.description.is_some(), "{} has no description", t.name);
    }
}

#[test]
fn close_requires_evidence_and_offers_no_way_to_self_assess() {
    let tools = wire_tools();
    let close = tools
        .iter()
        .find(|t| t.name == "aplan_close")
        .expect("aplan_close");
    let schema = serde_json::to_value(&*close.input_schema).unwrap();

    let required: Vec<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(required.contains(&"id"));
    assert!(
        required.contains(&"evidence"),
        "evidence must be required: a status is a claim this server will not make without proof"
    );

    // The anti-pattern, refused at the schema level: `additionalProperties:
    // false` means a caller cannot even send a score, let alone have one honoured.
    assert_eq!(
        schema["additionalProperties"],
        serde_json::Value::Bool(false)
    );
    let props: Vec<&str> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        props.len(),
        3,
        "aplan_close takes id, evidence, root and nothing else: {props:?}"
    );
    for forbidden in ["score", "confidence", "completeness", "percent", "status"] {
        assert!(
            !props.contains(&forbidden),
            "`{forbidden}` is not an input an agent may supply"
        );
    }
}

#[test]
fn every_tool_declares_an_object_schema() {
    for t in wire_tools() {
        let schema = serde_json::to_value(&*t.input_schema).unwrap();
        assert_eq!(schema["type"], "object", "{} has no object schema", t.name);
    }
}
