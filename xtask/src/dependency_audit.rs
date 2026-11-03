//! Executable dependency-shape policy for the shipped CLI and TUI (`A2-002`).
//!
//! Workspace metadata supplies the locked package census and every declared
//! direct dependency. Product shape is narrower: workspace metadata unifies
//! member features and can make headless `aplan` appear to contain
//! `aplan-tui`, so Cargo's own selected-package tree supplies the default and
//! headless reachable sets. No package list is copied by hand.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const POLICY: &str = "ci/dependency-policy.json";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema: String,
    target: String,
    budgets: Budgets,
    binary_budget: BinaryBudget,
    terminal_stack: BTreeMap<String, String>,
    forbidden_progress_packages: BTreeSet<String>,
    admissions: Vec<Admission>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Budgets {
    lock: usize,
    default: usize,
    headless: usize,
    tui_increment: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BinaryBudget {
    owner: String,
    measured_default_bytes: u64,
    measured_headless_bytes: u64,
    maximum_increment_bytes: u64,
    maximum_increment_basis_points: u64,
    measurement: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    owner: String,
    kind: String,
    dependency: String,
    target: String,
    behavior: String,
    task: String,
}

impl Admission {
    fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.owner, self.kind, self.dependency, self.target
        )
    }
}

#[derive(Debug)]
struct ProductGraph {
    packages: BTreeMap<String, BTreeSet<String>>,
}

impl ProductGraph {
    fn count(&self) -> usize {
        self.packages.values().map(BTreeSet::len).sum()
    }

    fn versions(&self, package: &str) -> BTreeSet<String> {
        self.packages.get(package).cloned().unwrap_or_default()
    }

    fn contains(&self, package: &str) -> bool {
        self.packages.contains_key(package)
    }

    fn identities(&self) -> BTreeSet<String> {
        self.packages
            .iter()
            .flat_map(|(name, versions)| {
                versions
                    .iter()
                    .map(move |version| format!("{name}@{version}"))
            })
            .collect()
    }
}

fn cargo() -> String {
    std::env::var("APLAN_CARGO")
        .or_else(|_| std::env::var("CARGO"))
        .unwrap_or_else(|_| "cargo".to_owned())
}

fn metadata(root: &Path, arguments: &[&str]) -> Result<serde_json::Value> {
    let output = Command::new(cargo())
        .args(["metadata", "--format-version", "1"])
        .args(arguments)
        .current_dir(root)
        .output()
        .context("running Cargo metadata for the dependency audit")?;
    if !output.status.success() {
        bail!(
            "Cargo metadata failed for the dependency audit:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing dependency-audit Cargo metadata")
}

fn workspace_metadata(root: &Path) -> Result<serde_json::Value> {
    metadata(root, &["--locked"])
}

/// The temporary YAML bridge has two independently versioned layers. Pinning
/// only the public wrapper still lets a lock refresh select a newer compatible
/// engine, which would make the executable compatibility evidence describe a
/// different parser than the committed policy reviewed by A2-002.
fn audit_yaml_bridge(metadata: &serde_json::Value) -> Result<()> {
    let packages = metadata["packages"]
        .as_array()
        .context("workspace Cargo metadata omitted its packages")?;
    let core = packages
        .iter()
        .find(|package| package["name"] == "aplan-core")
        .context("workspace contains no aplan-core package")?;
    let dependencies = core["dependencies"]
        .as_array()
        .context("aplan-core metadata omitted dependencies")?;

    for (package, rename, requirement) in [
        ("yaml_serde", "yaml_compat", "=0.10.7"),
        ("libyaml-rs", "yaml_engine_pin", "=0.3.0"),
    ] {
        let dependency = dependencies
            .iter()
            .find(|dependency| dependency["name"] == package && dependency["rename"] == rename)
            .with_context(|| {
                format!("aplan-core must declare YAML bridge layer `{package}` as `{rename}`")
            })?;
        let found = dependency["req"]
            .as_str()
            .context("YAML bridge dependency omitted its version requirement")?;
        if found != requirement {
            bail!(
                "YAML bridge layer `{rename}` must stay pinned to `{requirement}`, found `{found}`"
            );
        }
    }
    Ok(())
}

fn product_graph_with_environment(
    root: &Path,
    target: &str,
    defaults: bool,
    environment: &[(&str, &str)],
) -> Result<ProductGraph> {
    let mut command = Command::new(cargo());
    command.args([
        "tree",
        "--color",
        "never",
        "--locked",
        "--package",
        "aplan",
        "--edges",
        "normal,build",
        "--target",
        target,
        "--prefix",
        "none",
        "--format",
        "{p}",
    ]);
    if !defaults {
        command.arg("--no-default-features");
    }
    let output = command
        .envs(environment.iter().copied())
        .current_dir(root)
        .output()
        .context("running Cargo tree for the dependency audit")?;
    if !output.status.success() {
        bail!(
            "Cargo tree failed for the dependency audit:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let output = std::str::from_utf8(&output.stdout).context("Cargo tree output is not UTF-8")?;
    let mut packages = BTreeMap::<String, BTreeSet<String>>::new();
    for line in output.lines() {
        let line = line.strip_suffix(" (*)").unwrap_or(line);
        let line = line.strip_suffix(" (proc-macro)").unwrap_or(line);
        let package = line.split(" (/").next().unwrap_or(line);
        let (name, version) = package
            .rsplit_once(" v")
            .with_context(|| format!("cannot parse Cargo tree package line `{line}`"))?;
        packages
            .entry(name.to_owned())
            .or_default()
            .insert(version.to_owned());
    }
    Ok(ProductGraph { packages })
}

fn product_graph(root: &Path, target: &str, defaults: bool) -> Result<ProductGraph> {
    product_graph_with_environment(root, target, defaults, &[])
}

fn dependency_key(owner: &str, dependency: &serde_json::Value) -> Result<String> {
    let name = dependency["name"]
        .as_str()
        .context("Cargo dependency has no name")?;
    let kind = dependency["kind"].as_str().unwrap_or("normal");
    let target = dependency["target"].as_str().unwrap_or("all");
    Ok(format!("{owner}:{kind}:{name}:{target}"))
}

fn audit_admissions(root: &Path, metadata: &serde_json::Value, policy: &Policy) -> Result<()> {
    let packages = metadata["packages"]
        .as_array()
        .context("workspace Cargo metadata omitted its packages")?;
    let by_name: BTreeMap<&str, &serde_json::Value> = packages
        .iter()
        .filter_map(|package| package["name"].as_str().map(|name| (name, package)))
        .collect();
    let expected: BTreeSet<String> = policy.admissions.iter().map(Admission::key).collect();
    if expected.len() != policy.admissions.len() {
        bail!("{POLICY} contains a duplicate dependency admission");
    }
    for admission in &policy.admissions {
        if admission.behavior.trim().is_empty() {
            bail!(
                "dependency admission `{}` names no user behavior",
                admission.key()
            );
        }
        if !root
            .join("aplan/tasks")
            .join(format!("{}.md", admission.task))
            .is_file()
        {
            bail!(
                "dependency admission `{}` names missing task `{}`",
                admission.key(),
                admission.task
            );
        }
        let package = by_name
            .get(admission.owner.as_str())
            .with_context(|| format!("dependency owner `{}` is not a package", admission.owner))?;
        let found = package["dependencies"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|dependency| dependency_key(&admission.owner, dependency))
            .collect::<Result<BTreeSet<_>>>()?;
        if !found.contains(&admission.key()) {
            bail!(
                "dependency admission `{}` does not match Cargo metadata",
                admission.key()
            );
        }
    }

    let tui = by_name
        .get("aplan-tui")
        .context("workspace contains no aplan-tui package")?;
    let actual_tui = tui["dependencies"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|dependency| dependency_key("aplan-tui", dependency))
        .collect::<Result<BTreeSet<_>>>()?;
    let mapped_tui: BTreeSet<String> = policy
        .admissions
        .iter()
        .filter(|admission| admission.owner == "aplan-tui")
        .map(Admission::key)
        .collect();
    if actual_tui != mapped_tui {
        let missing = actual_tui
            .difference(&mapped_tui)
            .cloned()
            .collect::<Vec<_>>();
        let stale = mapped_tui
            .difference(&actual_tui)
            .cloned()
            .collect::<Vec<_>>();
        bail!("TUI dependency admission drift; unmapped={missing:?}, stale={stale:?}");
    }
    Ok(())
}

fn check_budget(name: &str, actual: usize, maximum: usize) -> Result<()> {
    if actual > maximum {
        bail!("dependency budget `{name}` exceeded: {actual} > {maximum}");
    }
    Ok(())
}

fn audit_graphs(
    policy: &Policy,
    lock_count: usize,
    default: &ProductGraph,
    headless: &ProductGraph,
) -> Result<()> {
    check_budget("lock_packages", lock_count, policy.budgets.lock)?;
    check_budget("default_packages", default.count(), policy.budgets.default)?;
    check_budget(
        "headless_packages",
        headless.count(),
        policy.budgets.headless,
    )?;
    let increment = default
        .identities()
        .difference(&headless.identities())
        .count();
    check_budget(
        "tui_increment_packages",
        increment,
        policy.budgets.tui_increment,
    )?;

    if headless.contains("aplan-tui") {
        bail!("headless aplan unexpectedly resolves aplan-tui");
    }
    for (package, version) in &policy.terminal_stack {
        let expected = BTreeSet::from([version.clone()]);
        let observed = default.versions(package);
        if observed != expected {
            bail!("terminal package `{package}` resolved {observed:?}, expected {expected:?}");
        }
        if headless.contains(package) {
            bail!("headless aplan unexpectedly resolves terminal package `{package}`");
        }
    }
    for package in &policy.forbidden_progress_packages {
        if default.contains(package) || headless.contains(package) {
            bail!(
                "external progress package `{package}` entered the product graph; the product uses its internal renderer"
            );
        }
    }
    Ok(())
}

fn audit_binary_budget(root: &Path, budget: &BinaryBudget) -> Result<()> {
    if budget.measurement.trim().is_empty() {
        bail!("binary dependency budget has no measurement method");
    }
    if !root
        .join("aplan/tasks")
        .join(format!("{}.md", budget.owner))
        .is_file()
    {
        bail!(
            "binary dependency budget owner `{}` is not a task",
            budget.owner
        );
    }
    let increment = budget
        .measured_default_bytes
        .checked_sub(budget.measured_headless_bytes)
        .context("measured default binary is smaller than its headless control")?;
    let basis_points = increment
        .saturating_mul(10_000)
        .checked_div(budget.measured_headless_bytes)
        .context("measured headless binary has zero bytes")?;
    if increment > budget.maximum_increment_bytes
        || basis_points > budget.maximum_increment_basis_points
    {
        bail!(
            "recorded binary increment exceeds its owned budget: {increment} bytes, {basis_points} basis points"
        );
    }
    Ok(())
}

/// Audit the exact locked supply shape and print the measured package counts.
pub fn gate(root: &Path) -> Result<()> {
    let policy: Policy = serde_json::from_slice(
        &std::fs::read(root.join(POLICY)).with_context(|| format!("reading {POLICY}"))?,
    )
    .with_context(|| format!("parsing {POLICY}"))?;
    if policy.schema != "aplan/dependency-policy/v1" {
        bail!("{POLICY} has unsupported schema `{}`", policy.schema);
    }
    let workspace = workspace_metadata(root)?;
    audit_yaml_bridge(&workspace)?;
    audit_admissions(root, &workspace, &policy)?;
    audit_binary_budget(root, &policy.binary_budget)?;
    let lock_count = workspace["packages"]
        .as_array()
        .context("workspace Cargo metadata omitted its packages")?
        .len();
    let default = product_graph(root, &policy.target, true)?;
    let headless = product_graph(root, &policy.target, false)?;
    audit_graphs(&policy, lock_count, &default, &headless)?;
    let increment = default
        .identities()
        .difference(&headless.identities())
        .count();
    println!(
        "dependency-audit: lock={lock_count} default={} headless={} tui_increment={increment} target={}",
        default.count(),
        headless.count(),
        policy.target
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(packages: &[(&str, &str)]) -> ProductGraph {
        let mut selected = BTreeMap::<String, BTreeSet<String>>::new();
        for (name, version) in packages {
            selected
                .entry((*name).to_owned())
                .or_default()
                .insert((*version).to_owned());
        }
        ProductGraph { packages: selected }
    }

    fn test_policy() -> Policy {
        Policy {
            schema: "aplan/dependency-policy/v1".to_owned(),
            target: "test-target".to_owned(),
            budgets: Budgets {
                lock: 3,
                default: 3,
                headless: 1,
                tui_increment: 2,
            },
            binary_budget: BinaryBudget {
                owner: "A1-test".to_owned(),
                measured_default_bytes: 115,
                measured_headless_bytes: 100,
                maximum_increment_bytes: 20,
                maximum_increment_basis_points: 2_000,
                measurement: "test measurement".to_owned(),
            },
            terminal_stack: BTreeMap::from([("terminal".to_owned(), "2.0.0".to_owned())]),
            forbidden_progress_packages: BTreeSet::from(["progress".to_owned()]),
            admissions: Vec::new(),
        }
    }

    #[test]
    fn committed_dependency_policy_matches_cargo_metadata() {
        gate(&crate::workspace_root().expect("workspace root"))
            .expect("committed dependency policy matches Cargo metadata");
    }

    #[test]
    fn dependency_census_is_invariant_under_forced_cargo_color() {
        let root = crate::workspace_root().expect("workspace root");
        let target = "x86_64-unknown-linux-gnu";
        let plain = product_graph(&root, target, true).expect("plain Cargo tree");
        let forced =
            product_graph_with_environment(&root, target, true, &[("CARGO_TERM_COLOR", "always")])
                .expect("Cargo tree with forced color environment");
        assert_eq!(plain.identities(), forced.identities());
    }

    #[test]
    fn yaml_bridge_wrapper_and_engine_are_exactly_pinned() {
        let root = crate::workspace_root().expect("workspace root");
        let metadata = workspace_metadata(&root).expect("workspace metadata");
        audit_yaml_bridge(&metadata).expect("both YAML bridge layers are exact");

        for (rename, bad_requirement) in [("yaml_compat", "^0.10.7"), ("yaml_engine_pin", "^0.3.0")]
        {
            let mut changed = metadata.clone();
            let dependency = changed["packages"]
                .as_array_mut()
                .expect("packages")
                .iter_mut()
                .find(|package| package["name"] == "aplan-core")
                .expect("aplan-core")["dependencies"]
                .as_array_mut()
                .expect("dependencies")
                .iter_mut()
                .find(|dependency| dependency["rename"] == rename)
                .expect("YAML bridge dependency");
            dependency["req"] = serde_json::Value::String(bad_requirement.to_owned());
            let error = audit_yaml_bridge(&changed).expect_err("a floating layer must fail");
            assert!(error.to_string().contains(rename), "{error:#}");
        }
    }

    #[test]
    fn product_graph_policy_rejects_each_budget_and_shape_escape() {
        let policy = test_policy();
        let default = graph(&[("aplan", "1.0.0"), ("terminal", "2.0.0")]);
        let headless = graph(&[("aplan", "1.0.0")]);
        audit_graphs(&policy, 3, &default, &headless).expect("bounded shape");

        let error = audit_graphs(&policy, 4, &default, &headless)
            .expect_err("lock budget must be enforced");
        assert!(error.to_string().contains("lock_packages"));

        let error = audit_graphs(
            &policy,
            3,
            &graph(&[
                ("aplan", "1.0.0"),
                ("terminal", "2.0.0"),
                ("extra", "1.0.0"),
                ("more", "1.0.0"),
            ]),
            &headless,
        )
        .expect_err("default budget must be enforced");
        assert!(error.to_string().contains("default_packages"));

        let error = audit_graphs(
            &policy,
            3,
            &default,
            &graph(&[("aplan", "1.0.0"), ("extra", "1.0.0")]),
        )
        .expect_err("headless budget must be enforced");
        assert!(error.to_string().contains("headless_packages"));

        let mut narrow = test_policy();
        narrow.budgets.tui_increment = 0;
        let error = audit_graphs(&narrow, 3, &default, &headless)
            .expect_err("TUI increment budget must be enforced");
        assert!(error.to_string().contains("tui_increment_packages"));

        let error = audit_graphs(&policy, 3, &default, &graph(&[("aplan-tui", "1.0.0")]))
            .expect_err("headless must not resolve the TUI");
        assert!(error.to_string().contains("aplan-tui"));

        let error = audit_graphs(
            &policy,
            3,
            &graph(&[("aplan", "1.0.0"), ("terminal", "1.0.0")]),
            &headless,
        )
        .expect_err("terminal version drift must be refused");
        assert!(error.to_string().contains("terminal package"));

        let error = audit_graphs(&policy, 3, &default, &graph(&[("terminal", "2.0.0")]))
            .expect_err("headless terminal linkage must be refused");
        assert!(error.to_string().contains("headless"));

        let error = audit_graphs(
            &policy,
            3,
            &graph(&[
                ("aplan", "1.0.0"),
                ("terminal", "2.0.0"),
                ("progress", "1.0.0"),
            ]),
            &headless,
        )
        .expect_err("rejected progress stack must stay absent");
        assert!(error.to_string().contains("external progress package"));
    }

    #[test]
    fn binary_budget_refuses_vacuity_missing_owners_and_regressions() {
        let root = tempfile::Builder::new()
            .prefix("dependency-audit-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary policy root");
        std::fs::create_dir_all(root.path().join("aplan/tasks")).expect("task directory");
        std::fs::write(
            root.path().join("aplan/tasks/A1-test.md"),
            "---\nid: A1-test\n---\n",
        )
        .expect("budget owner");
        let mut budget = test_policy().binary_budget;
        audit_binary_budget(root.path(), &budget).expect("bounded binary delta");

        budget.measurement.clear();
        assert!(
            audit_binary_budget(root.path(), &budget)
                .expect_err("measurement must be named")
                .to_string()
                .contains("no measurement")
        );
        budget.measurement = "test measurement".to_owned();
        budget.owner = "missing".to_owned();
        assert!(
            audit_binary_budget(root.path(), &budget)
                .expect_err("owner must resolve")
                .to_string()
                .contains("not a task")
        );
        budget.owner = "A1-test".to_owned();
        budget.measured_default_bytes = 99;
        assert!(
            audit_binary_budget(root.path(), &budget)
                .expect_err("default must not be smaller than headless")
                .to_string()
                .contains("smaller")
        );
        budget.measured_default_bytes = 121;
        assert!(
            audit_binary_budget(root.path(), &budget)
                .expect_err("owned ceiling must be enforced")
                .to_string()
                .contains("exceeds")
        );
    }

    #[test]
    fn binary_policy_refuses_unchecked_measurement_fields() {
        let root = crate::workspace_root().expect("workspace root");
        let mut policy: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(POLICY)).expect("committed dependency policy"),
        )
        .expect("dependency policy JSON");
        policy["binary_budget"]["foundation_default_bytes"] = 1_u64.into();

        let error = serde_json::from_value::<Policy>(policy)
            .expect_err("an unchecked historical measurement must be refused");
        assert!(
            error.to_string().contains("foundation_default_bytes"),
            "{error:#}"
        );
    }
}
