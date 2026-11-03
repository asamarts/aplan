//! Executable contract for the workspace's logical `yaml_compat` dependency.
//!
//! The root manifest is the one place that selects the backend. These tests
//! pin the compatibility surface callers depend on without adding a public
//! facade to the dependency-sink crate.

use yaml_compat::{Mapping, Value};

fn key(name: &str) -> Value {
    Value::String(name.to_owned())
}

#[test]
fn aliases_anchors_tags_and_literal_merge_keys_keep_the_compatibility_semantics() {
    let input = "\
base: &base
  value: 7
copy: *base
tagged: !subject text
merged:
  <<: *base
";
    let value: Value = yaml_compat::from_str(input).expect("compatibility document parses");
    let root = value.as_mapping().expect("root mapping");
    assert_eq!(root[&key("copy")], root[&key("base")]);

    let Value::Tagged(tagged) = &root[&key("tagged")] else {
        panic!("tag survives");
    };
    assert_eq!(tagged.tag.to_string(), "!subject");
    assert_eq!(tagged.value, Value::String("text".to_owned()));

    let merged = root[&key("merged")].as_mapping().expect("nested mapping");
    assert!(
        merged.contains_key("<<"),
        "merge syntax remains a literal key unless a caller explicitly applies it"
    );
}

#[test]
fn duplicate_keys_and_trailing_documents_are_rejected_with_locations() {
    let duplicate = yaml_compat::from_str::<Value>("item: one\nitem: two\n")
        .expect_err("a generic mapping must not silently replace a duplicate key");
    let location = duplicate.location().expect("duplicate has a location");
    assert_eq!((location.line(), location.column()), (1, 1));

    let trailing = yaml_compat::from_str::<Value>("---\none\n---\ntwo\n")
        .expect_err("the convenience API accepts exactly one document");
    assert!(trailing.to_string().contains("more than one document"));
}

#[test]
fn ambiguous_scalars_keep_yaml_1_2_compatible_types() {
    let value: Value = yaml_compat::from_str(
        "yes_word: yes\ndate_like: 2026-10-29\nleading_zero: 0123\nhex: 0x1f\nnull_value: ~\n",
    )
    .expect("scalar fixture parses");
    let map = value.as_mapping().expect("mapping");
    assert_eq!(map[&key("yes_word")].as_str(), Some("yes"));
    assert_eq!(map[&key("date_like")].as_str(), Some("2026-10-29"));
    assert_eq!(map[&key("leading_zero")].as_str(), Some("0123"));
    assert_eq!(map[&key("hex")].as_u64(), Some(31));
    assert!(map[&key("null_value")].is_null());
}

#[test]
fn invalid_utf8_is_refused_at_the_byte_boundary() {
    let error = yaml_compat::from_slice::<Value>(&[b'k', b':', b' ', 0xff])
        .expect_err("non-UTF-8 input is not YAML text");
    assert!(error.to_string().contains("UTF-8"));
}

#[test]
fn depth_and_alias_stress_shapes_finish_without_losing_values() {
    let mut nested = "0".to_owned();
    for _ in 0..128 {
        nested = format!("[{nested}]");
    }
    let value: Value = yaml_compat::from_str(&nested).expect("supported depth parses");
    let mut cursor = &value;
    for _ in 0..128 {
        cursor = &cursor.as_sequence().expect("sequence level")[0];
    }
    assert_eq!(cursor.as_u64(), Some(0));

    let mut aliases = String::from("base: &base {value: payload}\nitems:\n");
    for _ in 0..1_000 {
        aliases.push_str("  - *base\n");
    }
    let value: Value =
        yaml_compat::from_str(&aliases).expect("bounded alias stress fixture parses");
    let items = value["items"].as_sequence().expect("items sequence");
    assert_eq!(items.len(), 1_000);
    assert!(items.iter().all(|item| item["value"] == "payload"));
}

#[test]
fn mapping_serialization_is_stable_and_round_trips() {
    let mut mapping = Mapping::new();
    mapping.insert(key("second"), Value::Number(2.into()));
    mapping.insert(key("first"), Value::Number(1.into()));
    let value = Value::Mapping(mapping);

    let first = yaml_compat::to_string(&value).expect("serialize");
    let second = yaml_compat::to_string(&value).expect("serialize again");
    assert_eq!(first, second);
    assert!(first.find("second:").unwrap() < first.find("first:").unwrap());
    assert_eq!(
        yaml_compat::from_str::<Value>(&first).expect("round trip"),
        value
    );
}
