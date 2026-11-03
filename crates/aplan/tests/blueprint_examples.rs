//! Executable census for the explicit blueprint examples (`A1-392`).
//!
//! An example is executable documentation: every YAML file under
//! `examples/blueprints` must have one driven entry below, must apply through
//! the public CLI to a fresh plan, and must leave a plan `check` accepts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::common;

struct ExampleCase {
    file: &'static str,
    bindings: &'static [(&'static str, &'static str)],
    expected_tasks: usize,
}

const CASES: &[ExampleCase] = &[
    ExampleCase {
        file: "feature.yml",
        bindings: &[("feature", "bulk archive")],
        expected_tasks: 3,
    },
    ExampleCase {
        file: "pilot.yml",
        bindings: &[("repository", "widget")],
        expected_tasks: 1,
    },
    ExampleCase {
        file: "rule.yml",
        bindings: &[
            ("code", "APLAN999"),
            ("subject", "live tasks with blank owners"),
        ],
        expected_tasks: 1,
    },
];

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn examples_root() -> PathBuf {
    repository_root().join("examples/blueprints")
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent");
    }
    std::fs::write(path, text).expect("fixture file");
}

fn scratch_plan() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("blueprint-example-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("temporary repository");
    write(&dir.path().join(".aplan.yml"), "root: aplan\n");
    write(
        &dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: Existing work\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/existing.rs]\n---\n",
    );
    write(
        &dir.path().join("aplan/tasks/M-example.md"),
        "---\nid: M-example\ntitle: Example cut\nphase: 1\nstatus: todo\nblocked_by: [A-1]\ntags: [milestone]\n---\n",
    );
    dir
}

fn discover_yaml(dir: &Path, root: &Path, found: &mut BTreeSet<String>) {
    for entry in
        std::fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
    {
        let entry = entry.expect("example directory entry");
        let path = entry.path();
        let kind = entry.file_type().expect("example file type");
        if kind.is_dir() {
            discover_yaml(&path, root, found);
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("example remains below its root")
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "README.md" {
            continue;
        }
        assert!(
            matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yml" | "yaml")
            ),
            "unsupported file in the blueprint example census: {relative}"
        );
        assert!(
            found.insert(relative.clone()),
            "duplicate example {relative}"
        );
    }
}

fn discovered_examples() -> BTreeSet<String> {
    let root = examples_root();
    assert!(
        root.join("README.md").is_file(),
        "examples need their boundary document"
    );
    let mut found = BTreeSet::new();
    discover_yaml(&root, &root, &mut found);
    found
}

fn registered_cases() -> BTreeMap<&'static str, &'static ExampleCase> {
    let mut registered = BTreeMap::new();
    for case in CASES {
        assert!(
            registered.insert(case.file, case).is_none(),
            "duplicate census entry for {}",
            case.file
        );
    }
    registered
}

fn bound(case: &ExampleCase) -> aplan_interop::blueprint::Blueprint {
    let path = examples_root().join(case.file);
    let parsed = aplan_interop::blueprint::read(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let bindings = case
        .bindings
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect();
    aplan_interop::blueprint::bind_parameters(&parsed, &bindings)
        .unwrap_or_else(|error| panic!("bind {}: {error}", path.display()))
}

/// The directory is the census: adding a YAML file without a driven case fails,
/// and every case goes through the shipped command before structural checking.
#[test]
fn every_blueprint_example_is_registered_applied_and_checked() {
    let discovered = discovered_examples();
    let registered = registered_cases();
    assert_eq!(
        discovered,
        registered.keys().map(|name| (*name).to_string()).collect(),
        "every example file needs one executable census entry and no stale entry may survive"
    );

    for case in registered.values() {
        let parsed = bound(case);
        assert_eq!(
            parsed.tasks.len(),
            case.expected_tasks,
            "{} changed arity without updating its review contract",
            case.file
        );

        let dir = scratch_plan();
        let source = examples_root().join(case.file);
        let source_text = source.to_string_lossy().into_owned();
        let mut arguments = vec![
            "blueprint".to_string(),
            "apply".to_string(),
            "--file".to_string(),
            source_text,
            "--cut".to_string(),
            "M-example".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ];
        for (name, value) in case.bindings {
            arguments.push("--set".to_string());
            arguments.push(format!("{name}={value}"));
        }
        let argument_refs = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        let applied = common::run(dir.path(), &argument_refs);
        assert_eq!(applied.code, 0, "{} did not apply: {applied:?}", case.file);
        let report: serde_json::Value =
            serde_json::from_str(&applied.stdout).unwrap_or_else(|error| {
                panic!("{} did not return JSON: {error}: {applied:?}", case.file)
            });
        assert_eq!(report["outcome"], "applied", "{}: {report:#}", case.file);
        assert_eq!(
            report["tasks"].as_array().map(Vec::len),
            Some(case.expected_tasks),
            "{}: {report:#}",
            case.file
        );
        assert_eq!(
            report["source_sha256"],
            aplan_attest::sha256_hex(&std::fs::read(&source).expect("example bytes")),
            "{} did not identify the bytes the census applied",
            case.file
        );

        let checked = common::run(dir.path(), &["check", "--format", "json"]);
        assert_eq!(
            checked.code, 0,
            "{} produced a plan check refused: {checked:?}",
            case.file
        );
        let verdict: serde_json::Value = serde_json::from_str(&checked.stdout)
            .unwrap_or_else(|error| panic!("check output was not JSON: {error}: {checked:?}"));
        assert_eq!(
            verdict["diagnostics"],
            serde_json::json!([]),
            "{}: {verdict:#}",
            case.file
        );
    }
}

/// Both supported arities are examples only. The runtime exposes one explicit
/// apply operation and refuses catalogue-shaped `list` and `show` commands.
#[test]
fn blueprint_examples_cover_both_arities_without_becoming_builtins() {
    let arities = CASES
        .iter()
        .map(|case| bound(case).tasks.len())
        .collect::<BTreeSet<_>>();
    assert_eq!(arities, BTreeSet::from([1, 3]));

    let help = common::run(&repository_root(), &["blueprint", "--help"]);
    assert_eq!(help.code, 0, "{help:?}");
    let commands = help
        .stdout
        .lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert!(
        commands
            .iter()
            .any(|line| line.trim_start().starts_with("apply ")),
        "{commands:?}"
    );
    assert!(
        commands.iter().all(|line| {
            let name = line.split_whitespace().next().unwrap_or_default();
            matches!(name, "apply" | "help")
        }),
        "an example escaped into the runtime command catalogue: {commands:?}"
    );
    for arguments in [
        ["blueprint", "list"].as_slice(),
        ["blueprint", "show", "pilot"].as_slice(),
    ] {
        let refused = common::run(&repository_root(), arguments);
        assert_eq!(
            refused.code, 2,
            "a catalogue command unexpectedly exists: {refused:?}"
        );
    }
}

fn task<'a>(plan: &'a aplan_core::Plan, id: &str) -> &'a aplan_core::Task {
    &plan
        .tasks
        .iter()
        .find(|loaded| loaded.task.id == id)
        .unwrap_or_else(|| panic!("representative task {id} is absent"))
        .task
}

fn paths(values: &[String]) -> BTreeSet<&str> {
    values.iter().map(String::as_str).collect()
}

/// The examples retain recurring concerns while the real tasks prove that
/// neither name denotes one universal path set or delivery methodology.
#[test]
fn pilot_and_rule_examples_are_representative_not_canonical() {
    let plan = aplan_core::load_plan(&repository_root().join("aplan"));
    assert!(
        plan.load_diagnostics.is_empty(),
        "the repository plan must load"
    );
    let private_epoch = plan.tasks.iter().any(|loaded| loaded.task.id == "A1-071");
    if !private_epoch {
        assert!(
            !plan.tasks.is_empty()
                && plan
                    .tasks
                    .iter()
                    .all(|loaded| loaded.task.id.starts_with("A2-")),
            "the historical examples may be absent only in the explicit A2 public-plan epoch"
        );
    }

    let pilot_case = CASES
        .iter()
        .find(|case| case.file == "pilot.yml")
        .expect("pilot case");
    let pilot = bound(pilot_case);
    let pilot_paths = paths(&pilot.tasks[0].task.paths);
    assert!(pilot_paths.contains("docs/pilots/widget.md"));
    for id in if private_epoch {
        &["A1-071", "A1-140"][..]
    } else {
        &[]
    } {
        let real = task(&plan, id);
        assert!(real.tags.iter().any(|tag| tag == "pilot"), "{id}");
        assert!(
            real.paths
                .iter()
                .any(|path| path.starts_with("docs/pilots/")),
            "{id}"
        );
        assert_ne!(
            pilot_paths,
            paths(&real.paths),
            "{id} must remain an instance, not a template claim"
        );
    }

    let rule_case = CASES
        .iter()
        .find(|case| case.file == "rule.yml")
        .expect("rule case");
    let rule = bound(rule_case);
    let rule_paths = paths(&rule.tasks[0].task.paths);
    for required in [
        "crates/aplan-core/src/diagnostic.rs",
        "crates/aplan-e2e/tests/coverage.rs",
        "docs/rule-catalogue.md",
        "CHANGELOG.md",
    ] {
        assert!(
            rule_paths.contains(required),
            "rule example omitted {required}"
        );
    }
    for id in if private_epoch {
        &["A1-017", "A1-212"][..]
    } else {
        &[]
    } {
        let real = task(&plan, id);
        assert!(real.tags.iter().any(|tag| tag == "rules"), "{id}");
        assert!(
            real.paths
                .iter()
                .any(|path| path.starts_with("crates/aplan-core/")),
            "{id}"
        );
        assert!(
            real.paths
                .iter()
                .any(|path| path.starts_with("crates/aplan-e2e/")),
            "{id}"
        );
        assert_ne!(
            rule_paths,
            paths(&real.paths),
            "{id} must remain an instance, not a template claim"
        );
    }
}
