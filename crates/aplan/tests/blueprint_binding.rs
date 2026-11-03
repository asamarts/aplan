//! Total, typed parameter binding for parsed blueprint documents (`A1-386`).

use std::collections::BTreeMap;

use aplan_interop::blueprint::{BindingError, SCHEMA, bind_parameters, read};

fn source(dir: &std::path::Path, task: &str) -> std::path::PathBuf {
    let path = dir.join("feature.yml");
    std::fs::write(
        &path,
        format!(
            "schema: {SCHEMA}\nname: feature\ntitle: Feature work\n\
             description: A bound feature graph.\nparameters:\n  - name: feature\n    \
             description: The feature name.\ntasks:\n{task}"
        ),
    )
    .expect("blueprint source");
    path
}

/// Proves `blueprint.parameters_bind_totally_after_parsing`: missing,
/// undeclared and unused values are refusals, while opaque values cannot add a
/// YAML field, task or relationship after the document has been parsed.
#[test]
fn binding_is_total_field_aware_and_non_recursive() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let path = source(
        dir.path(),
        "  - key: implement\n    title: 'Implement {{feature}}'\n    paths: ['src/{{feature}}/**']\n    \
         blocked_by: ['task:{{feature}}']\n    body: 'Literal {{{{feature}}}}; bound {{feature}}'\n",
    );
    let blueprint = read(&path).expect("parsed blueprint");
    let value = "archive:\n- not-a-task {{opaque}}";
    let bound = bind_parameters(
        &blueprint,
        &BTreeMap::from([("feature".to_string(), value.to_string())]),
    )
    .expect("complete binding");
    assert_eq!(bound.tasks.len(), 1);
    let task = &bound.tasks[0].task;
    assert_eq!(task.paths.len(), 1);
    assert_eq!(task.blocked_by.len(), 1);
    assert!(task.title.contains(value), "{}", task.title);
    assert_eq!(task.blocked_by[0], format!("task:{value}"));
    let expected_body = format!("Literal {{{{feature}}}}; bound {value}");
    assert_eq!(task.body.as_deref(), Some(expected_body.as_str()));
    assert!(task.title.contains("{{opaque}}"), "{}", task.title);

    assert!(matches!(
        bind_parameters(&blueprint, &BTreeMap::new()),
        Err(BindingError::MissingBinding { name }) if name == "feature"
    ));
    assert!(matches!(
        bind_parameters(
            &blueprint,
            &BTreeMap::from([("feature".to_string(), "\u{200b}".to_string())])
        ),
        Err(BindingError::EmptyBinding { name }) if name == "feature"
    ));

    let undeclared_path = source(
        dir.path(),
        "  - key: implement\n    title: 'Implement {{missing}}'\n",
    );
    let undeclared = read(&undeclared_path).expect("undeclared source still parses");
    assert!(matches!(
        bind_parameters(
            &undeclared,
            &BTreeMap::from([("feature".to_string(), "archive".to_string())])
        ),
        Err(BindingError::UnknownPlaceholder { name, .. }) if name == "missing"
    ));

    let unused_path = source(
        dir.path(),
        "  - key: implement\n    title: Implement archive\n",
    );
    let unused = read(&unused_path).expect("unused source parses");
    assert!(matches!(
        bind_parameters(
            &unused,
            &BTreeMap::from([("feature".to_string(), "archive".to_string())])
        ),
        Err(BindingError::UnusedParameter { name }) if name == "feature"
    ));

    assert!(
        !dir.path().join("aplan").exists(),
        "binding is a pure compile step and a refusal wrote plan bytes"
    );
}
