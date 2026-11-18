//! Boundary matrix for ayaml's native limits through the aplan boundary.
//!
//! Each case sets one ceiling, proves the accepted boundary, then proves the
//! first refused neighbour. Product defaults are covered separately by the
//! depth, document and alias fixtures in `yaml_contract.rs`.

use aplan_core::load_plan;
use yaml_compat::{Limits, Value};

fn limits() -> Limits {
    Limits::default()
}

macro_rules! limits {
    ($($field:ident: $value:expr),* $(,)?) => {{
        let mut limits = Limits::default();
        $(limits.$field = $value;)*
        limits
    }};
}

#[test]
fn input_bytes_boundary() {
    let mut limits = limits();
    limits.max_input_bytes = 8;
    assert!(yaml_compat::parse_value_with_limits("12345678", &limits).is_ok());
    let error = yaml_compat::parse_value_with_limits("123456789", &limits).unwrap_err();
    assert!(error.to_string().contains("input bytes"));
}

#[test]
fn depth_boundary() {
    let limits = limits! { max_depth: 8 };
    let nest = |n: usize| {
        let mut text = "0".to_owned();
        for _ in 0..n {
            text = format!("[{text}]");
        }
        text
    };
    assert!(yaml_compat::parse_value_with_limits(&nest(8), &limits).is_ok());
    let error = yaml_compat::parse_value_with_limits(&nest(9), &limits).unwrap_err();
    assert!(error.to_string().contains("collection depth"));
}

#[test]
fn document_count_boundary() {
    let limits = limits! { max_documents: 1 };
    assert!(yaml_compat::parse_documents_with_limits("---\na\n", &limits).is_ok());
    let error = yaml_compat::parse_documents_with_limits("---\na\n---\nb\n", &limits).unwrap_err();
    assert!(error.to_string().contains("documents"));
}

#[test]
fn event_boundary() {
    let accepted = limits! { max_events: 5 };
    assert!(yaml_compat::parse_value_with_limits("a\n", &accepted).is_ok());
    let refused = limits! { max_events: 4 };
    let error = yaml_compat::parse_value_with_limits("a\n", &refused).unwrap_err();
    assert!(error.to_string().contains("events"));
}

#[test]
fn alias_expansion_bytes_boundary() {
    let limits =
        limits! { max_alias_expansion_bytes: 1, alias_bytes_per_input_byte: 0, node_cost_bytes: 1 };
    assert!(yaml_compat::parse_value_with_limits("a: &a 1\nb: *a\n", &limits).is_ok());
    let error = yaml_compat::parse_value_with_limits("a: &a [1]\nb: *a\n", &limits).unwrap_err();
    assert!(error.to_string().contains("alias expansion"));
}

#[test]
fn scalar_bytes_boundary() {
    let limits = limits! { max_scalar_bytes: 8 };
    assert!(yaml_compat::parse_value_with_limits("12345678", &limits).is_ok());
    let error = yaml_compat::parse_value_with_limits("123456789", &limits).unwrap_err();
    assert!(error.to_string().contains("scalar bytes"));
}

#[test]
fn collection_entries_boundary() {
    let limits = limits! { max_collection_entries: 2 };
    assert!(yaml_compat::parse_value_with_limits("[1, 2]", &limits).is_ok());
    let error = yaml_compat::parse_value_with_limits("[1, 2, 3]", &limits).unwrap_err();
    assert!(error.to_string().contains("collection entries"));
}

#[test]
fn anchor_boundary() {
    let limits = limits! { max_anchors: 1 };
    assert!(yaml_compat::parse_value_with_limits("a: &a 1\n", &limits).is_ok());
    let error = yaml_compat::parse_value_with_limits("a: &a 1\nb: &b 2\n", &limits).unwrap_err();
    assert!(error.to_string().contains("anchors"));
}

#[test]
fn alias_dereference_boundary() {
    let limits = limits! { max_alias_dereferences: 1 };
    assert!(yaml_compat::parse_value_with_limits("a: &a 1\nb: [*a]\n", &limits).is_ok());
    let error =
        yaml_compat::parse_value_with_limits("a: &a 1\nb: [*a, *a]\n", &limits).unwrap_err();
    assert!(error.to_string().contains("alias dereferences"));
}

#[test]
fn output_bytes_boundary() {
    let limits = limits! { max_output_bytes: 4 };
    assert!(yaml_compat::to_string_value_with_limits(&Value::from("abc"), &limits).is_ok());
    let error =
        yaml_compat::to_string_value_with_limits(&Value::from("abcd"), &limits).unwrap_err();
    assert!(error.to_string().contains("output bytes"));
}

#[test]
fn scalar_bytes_can_be_the_lower_bound_when_input_is_larger() {
    let limits = limits! { max_input_bytes: 64, max_scalar_bytes: 8 };
    let error = yaml_compat::parse_value_with_limits("123456789", &limits).unwrap_err();
    assert!(error.to_string().contains("scalar bytes"), "{error}");
    assert!(!error.to_string().contains("input bytes"), "{error}");
}

#[test]
fn event_ceiling_shadows_collection_entries_at_defaults() {
    let mut text = String::from("[");
    for index in 0..=1_000_000 {
        if index > 0 {
            text.push(',');
        }
        text.push('x');
    }
    text.push_str("]\n");
    let error = yaml_compat::compat::from_str::<Value>(&text).unwrap_err();
    assert!(error.to_string().contains("events"), "{error}");
    assert!(!error.to_string().contains("collection entries"), "{error}");
}

#[test]
fn default_depth_and_document_ceilings_still_refuse() {
    let nest = |n: usize| {
        let mut text = "0".to_owned();
        for _ in 0..n {
            text = format!("[{text}]");
        }
        text
    };
    assert!(yaml_compat::compat::from_str::<Value>(&nest(128)).is_ok());
    assert!(yaml_compat::compat::from_str::<Value>(&nest(129)).is_err());
    assert!(yaml_compat::compat::from_str::<Value>("---\na: 1\n---\nb: 2\n").is_err());
}

fn load_task(text: &str) -> aplan_core::Plan {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::write(dir.path().join("T-1.md"), text).expect("write task");
    load_plan(dir.path())
}

#[test]
fn product_task_loader_refuses_depth_past_the_default() {
    let nest = |n: usize| {
        let mut text = "0".to_owned();
        for _ in 0..n {
            text = format!("[{text}]");
        }
        text
    };
    let task = |depth: usize| {
        format!(
            "---\nid: T-1\ntitle: {}\nphase: 0\nstatus: todo\neffort_days: 1\n---\n",
            nest(depth)
        )
    };
    let plan = load_task(&task(127));
    assert!(
        !plan
            .load_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("recursion limit")),
        "the default depth boundary must reach the schema, not the parser: {:?}",
        plan.load_diagnostics
    );
    let plan = load_task(&task(129));
    assert_eq!(plan.tasks.len(), 0);
    assert!(
        plan.load_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("recursion limit")),
        "depth refusal must reach the product diagnostic: {:?}",
        plan.load_diagnostics
    );
}

#[test]
fn product_task_loader_refuses_an_alias_bomb() {
    use std::fmt::Write as _;
    let mut frontmatter = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x]\n");
    for i in 1..9 {
        let previous = format!("*a{}", i - 1);
        let row = std::iter::repeat_n(previous.as_str(), 9)
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(frontmatter, "a{i}: &a{i} [{row}]").expect("write");
    }
    let plan = load_task(&format!(
        "---\nid: T-1\ntitle: bomb\nphase: 0\nstatus: todo\neffort_days: 1\n{frontmatter}---\n"
    ));
    assert_eq!(plan.tasks.len(), 0);
    assert!(
        plan.load_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("alias")),
        "alias-bomb refusal must reach the product diagnostic: {:?}",
        plan.load_diagnostics
    );
}
