//! The CLI boundary and build-capability contract for `aplan tui`.

use crate::common;

/// `facts.reports_build_capabilities`: a disabled row is observable in the
/// document but is not advertised as an implementation `--require` can trust.
#[test]
fn a_build_capability_is_advertised_only_when_enabled() {
    let enabled = serde_json::json!({
        "build_capabilities": [{"name": "build.tui", "enabled": true}]
    });
    let disabled = serde_json::json!({
        "build_capabilities": [{"name": "build.tui", "enabled": false}]
    });
    assert!(aplan::advertised_names(&enabled).contains("build.tui"));
    assert!(!aplan::advertised_names(&disabled).contains("build.tui"));
}

/// `facts.reports_build_capabilities`: the standard build carries the TUI.
#[cfg(feature = "tui")]
#[test]
fn the_default_build_advertises_the_tui_capability() {
    let facts: serde_json::Value = serde_json::from_str(&aplan::facts()).expect("facts JSON");
    assert_eq!(
        facts["build_capabilities"],
        serde_json::json!([{"name": "build.tui", "enabled": true}])
    );
    assert!(aplan::advertised_names(&facts).contains("build.tui"));
}

/// The supported headless source build keeps capability discovery truthful.
#[cfg(not(feature = "tui"))]
#[test]
fn the_headless_build_reports_the_tui_capability_as_disabled() {
    let facts: serde_json::Value = serde_json::from_str(&aplan::facts()).expect("facts JSON");
    assert_eq!(
        facts["build_capabilities"],
        serde_json::json!([{"name": "build.tui", "enabled": false}])
    );
    assert!(!aplan::advertised_names(&facts).contains("build.tui"));
}

#[cfg(feature = "tui")]
#[test]
fn redirected_tui_refuses_with_actionable_advice_before_reading_a_plan() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let out = common::output(dir.path(), &["tui", "--root", "missing"]);
    let error = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{error}");
    assert!(
        error.contains("stdin") && error.contains("stdout"),
        "{error}"
    );
    assert!(
        error.contains("aplan check") && error.contains("aplan ready"),
        "{error}"
    );
    assert!(
        !error.contains("plan root"),
        "the plan was opened before the terminal refusal: {error}"
    );
}

/// The parser-visible headless stub refuses before touching the requested plan
/// and names both the build remedy and the machine-readable capability probe.
#[cfg(not(feature = "tui"))]
#[test]
fn the_headless_tui_stub_explains_how_to_enable_and_detect_it() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let out = common::output(dir.path(), &["tui", "--root", "missing"]);
    let error = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{error}");
    assert!(
        error.contains("does not include the terminal interface")
            && error.contains("--features tui")
            && error.contains("facts --require build.tui"),
        "{error}"
    );
    assert!(
        !error.contains("plan root"),
        "the plan was opened before the capability refusal: {error}"
    );
}
