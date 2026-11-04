//! Generate and audit the executable validation-profile table (`A1-611`).
//!
//! The YAML is the reviewed policy. The generated Bash table and content-bound
//! selector graph are deliberately data-only, so hooks and Actions can share
//! them without compiling `xtask` before classification. This command proves
//! both generated copies and every workflow consumer agree with that one policy.

use std::collections::BTreeSet;
use std::fmt::Write as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

const MANIFEST: &str = "ci/validation.yml";
const GENERATED: &str = "ci/scripts/validation-generated.sh";
const SELECTOR_GENERATED: &str = "ci/validation-selection.json";
const SELECTOR: &str = "ci/scripts/validation-select.py";
const WORKFLOW: &str = ".github/workflows/ci.yml";
const PREFLIGHT: &str = "ci/scripts/preflight.sh";
const ORDINARY_WORKFLOW_COMMAND: &str = "ci/scripts/preflight.sh --profile \"$APLAN_ORDINARY_PROFILE\" --base \"$APLAN_VALIDATION_BASE\" --head \"$APLAN_VALIDATION_HEAD\"";
const FULL_WORKFLOW_COMMAND: &str = "ci/scripts/validate.sh --profile full";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inventory {
    schema: u32,
    profiles: Profiles,
    pub(super) selection: Selection,
    pub(super) gates: Vec<Gate>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection {
    pub(super) schema: u32,
    pub(super) rust: RustSelection,
    pub(super) global_gates: Vec<String>,
    pub(super) paths: Vec<PathRule>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RustSelection {
    pub(super) gates: Vec<String>,
    pub(super) cross_cutting_packages: Vec<String>,
    pub(super) global_basenames: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum PathDisposition {
    Selected,
    Global,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PathRule {
    pub(super) name: String,
    pub(super) disposition: PathDisposition,
    pub(super) gates: Vec<String>,
    #[serde(default)]
    pub(super) exact: Vec<String>,
    #[serde(default)]
    pub(super) prefixes: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Profiles {
    development: Profile,
    full: Profile,
    release: Profile,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    assurance: Assurance,
    #[serde(default)]
    requires: Option<String>,
    #[serde(default)]
    same_source: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Assurance {
    Selected,
    Full,
    Release,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Gate {
    pub(super) name: String,
    command: String,
    input_class: InputClass,
    isolation: Vec<Isolation>,
    development: DevelopmentMembership,
    full: FullMembership,
    release: ReleaseMembership,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum InputClass {
    Repository,
    RustWorkspace,
    GeneratedContracts,
    DeclaredMsrv,
    DependencyPolicy,
    PlanStructure,
    PlanAndEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Isolation {
    None,
    SharedCargoTarget,
    IsolatedCargoTarget,
    IsolatedCoverageTarget,
    IsolatedAplanCache,
    FullGitHistory,
    NetworkAdvisoryDb,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum DevelopmentMembership {
    Always,
    Selectable,
    Deferred,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum FullMembership {
    Required,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ReleaseMembership {
    Inherited,
}

pub(crate) fn command(args: &[String], root: &Path) -> Result<()> {
    let check = match args {
        [] => false,
        [arg] if arg == "--check" => true,
        _ => bail!("usage: cargo xtask validation [--check]"),
    };
    let source =
        std::fs::read(root.join(MANIFEST)).with_context(|| format!("reading {MANIFEST}"))?;
    let inventory = load_inventory(root)?;
    audit_repository_wiring(root, &inventory)?;
    let rendered = render(&inventory);
    let workspace = selector_workspace(root)?;
    let selector = render_selector_inventory(&inventory, &source, &workspace)?;
    super::write_or_check(root, GENERATED, &rendered, check, "validation")?;
    super::write_or_check(
        root,
        SELECTOR_GENERATED,
        &selector,
        check,
        "validation selector policy",
    )?;
    audit_selector_census(root)
}

pub(crate) fn selector_command(args: &[String], root: &Path) -> Result<()> {
    let mut command = std::process::Command::new("python3");
    command.arg(root.join(SELECTOR));
    if !args.iter().any(|argument| argument == "--root") {
        command.arg("--root").arg(root);
    }
    let status = command
        .args(args)
        .status()
        .with_context(|| format!("running {SELECTOR}"))?;
    if !status.success() {
        bail!("{SELECTOR} exited with {status}");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct SelectorWorkspace {
    inputs: Vec<SelectorInput>,
    packages: Vec<SelectorPackage>,
    reverse: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
struct SelectorInput {
    path: String,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct SelectorPackage {
    name: String,
    root: String,
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
    resolve: Option<CargoResolve>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CargoResolve {
    nodes: Vec<CargoNode>,
}

#[derive(Debug, Deserialize)]
struct CargoNode {
    id: String,
    dependencies: Vec<String>,
}

fn selector_workspace(root: &Path) -> Result<SelectorWorkspace> {
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--all-features",
        ])
        .current_dir(root)
        .output()
        .context("running cargo metadata for the generated selector graph")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed for the generated selector graph: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let metadata: CargoMetadata = serde_json::from_slice(&output.stdout)
        .context("parsing cargo metadata for the generated selector graph")?;
    selector_workspace_from_metadata(root, metadata)
}

fn selector_workspace_from_metadata(
    root: &Path,
    metadata: CargoMetadata,
) -> Result<SelectorWorkspace> {
    let members: BTreeSet<_> = metadata.workspace_members.into_iter().collect();
    let mut id_to_name = std::collections::BTreeMap::new();
    let mut package_names = BTreeSet::new();
    let mut packages = Vec::new();
    let mut input_paths =
        BTreeSet::from([PathBuf::from("Cargo.toml"), PathBuf::from("Cargo.lock")]);
    for optional in ["rust-toolchain.toml", ".cargo/config.toml", ".cargo/config"] {
        if root.join(optional).is_file() {
            input_paths.insert(PathBuf::from(optional));
        }
    }
    for package in metadata
        .packages
        .into_iter()
        .filter(|package| members.contains(&package.id))
    {
        let directory = package
            .manifest_path
            .parent()
            .context("cargo metadata returned a manifest without a parent")?;
        let relative = directory.strip_prefix(root).with_context(|| {
            format!(
                "workspace package `{}` is outside the repository root",
                package.name
            )
        })?;
        if relative.as_os_str().is_empty() {
            bail!(
                "workspace package `{}` is rooted at the repository root",
                package.name
            );
        }
        if id_to_name
            .insert(package.id, package.name.clone())
            .is_some()
        {
            bail!("workspace contains a duplicate package identity");
        }
        if !package_names.insert(package.name.clone()) {
            bail!(
                "workspace contains duplicate package name `{}`",
                package.name
            );
        }
        input_paths.insert(
            package
                .manifest_path
                .strip_prefix(root)
                .context("workspace manifest is outside the repository root")?
                .to_path_buf(),
        );
        let relative = relative
            .to_str()
            .context("workspace package root is not UTF-8")?;
        packages.push(SelectorPackage {
            name: package.name,
            root: format!("{}/", relative.replace('\\', "/")),
        });
    }
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    if packages.is_empty() {
        bail!("cargo metadata returned no workspace packages");
    }
    let resolve = metadata
        .resolve
        .context("cargo metadata returned no resolved dependency graph")?;
    let mut reverse = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    for node in resolve.nodes {
        let Some(name) = id_to_name.get(&node.id) else {
            continue;
        };
        for dependency in node.dependencies {
            if let Some(dependency_name) = id_to_name.get(&dependency) {
                reverse
                    .entry(dependency_name.clone())
                    .or_default()
                    .insert(name.clone());
            }
        }
    }
    let reverse = reverse
        .into_iter()
        .map(|(name, dependents)| (name, dependents.into_iter().collect()))
        .collect();
    let inputs = selector_inputs(root, input_paths)?;
    Ok(SelectorWorkspace {
        inputs,
        packages,
        reverse,
    })
}

fn selector_inputs(root: &Path, input_paths: BTreeSet<PathBuf>) -> Result<Vec<SelectorInput>> {
    let mut inputs = input_paths
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(root.join(&path))
                .with_context(|| format!("reading selector graph input `{}`", path.display()))?;
            Ok(SelectorInput {
                path: path
                    .to_str()
                    .context("selector graph input path is not UTF-8")?
                    .replace('\\', "/"),
                sha256: sha256(&bytes),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    inputs.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(inputs)
}

fn sha256(source: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};

    Sha256::digest(source)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        })
}

fn render_selector_inventory(
    inventory: &Inventory,
    source: &[u8],
    workspace: &SelectorWorkspace,
) -> Result<String> {
    let document = serde_json::json!({
        "schema": "aplan/validation-selection-policy/v1",
        "source": {
            "path": MANIFEST,
            "sha256": sha256(source),
        },
        "inventory": inventory,
        "workspace": workspace,
    });
    let mut rendered = serde_json::to_string_pretty(&document)
        .context("serializing validation selector policy")?;
    rendered.push('\n');
    Ok(rendered)
}

fn audit_selector_census(root: &Path) -> Result<()> {
    let output = std::process::Command::new("python3")
        .arg(root.join(SELECTOR))
        .args(["--root", ".", "--census", "--format", "json"])
        .current_dir(root)
        .output()
        .with_context(|| format!("running {SELECTOR} census"))?;
    if !output.status.success() {
        bail!(
            "{SELECTOR} census failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(super) fn load_inventory(root: &Path) -> Result<Inventory> {
    let text = std::fs::read_to_string(root.join(MANIFEST))
        .with_context(|| format!("reading {MANIFEST}"))?;
    let inventory: Inventory =
        yaml_compat::from_str(&text).with_context(|| format!("parsing {MANIFEST}"))?;
    validate_inventory(root, &inventory)?;
    Ok(inventory)
}

fn validate_inventory(root: &Path, inventory: &Inventory) -> Result<()> {
    validate_inventory_shape(inventory)?;
    for gate in &inventory.gates {
        let path = root.join(&gate.command);
        let metadata = std::fs::metadata(&path).with_context(|| {
            format!(
                "validation gate `{}` is missing `{}`",
                gate.name, gate.command
            )
        })?;
        if !is_executable_file(&metadata) {
            bail!(
                "validation gate `{}` command `{}` is not an executable file",
                gate.name,
                gate.command
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable_file(metadata: &std::fs::Metadata) -> bool {
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_file(metadata: &std::fs::Metadata) -> bool {
    // Windows has no POSIX execute bits. The inventory's interpreter owns
    // executability there, so the portable assertion is that the path is a file.
    metadata.is_file()
}

fn validate_inventory_shape(inventory: &Inventory) -> Result<()> {
    if inventory.schema != 1 {
        bail!(
            "unsupported validation inventory schema {} (expected 1)",
            inventory.schema
        );
    }
    if inventory.profiles.development.assurance != Assurance::Selected
        || inventory.profiles.development.requires.is_some()
        || inventory.profiles.development.same_source
    {
        bail!("development must be the standalone `selected` assurance profile");
    }
    if inventory.profiles.full.assurance != Assurance::Full
        || inventory.profiles.full.requires.is_some()
        || inventory.profiles.full.same_source
    {
        bail!("full must be the standalone `full` assurance profile");
    }
    if inventory.profiles.release.assurance != Assurance::Release
        || inventory.profiles.release.requires.as_deref() != Some("full")
        || !inventory.profiles.release.same_source
    {
        bail!("release must require `full` on the same source identity");
    }
    if inventory.gates.is_empty() {
        bail!("validation inventory must declare at least one gate");
    }

    let mut names = BTreeSet::new();
    let mut commands = BTreeSet::new();
    let mut development_kinds = BTreeSet::new();
    for gate in &inventory.gates {
        if !valid_name(&gate.name) {
            bail!(
                "validation gate name `{}` must match [a-z][a-z0-9-]*",
                gate.name
            );
        }
        if !names.insert(gate.name.as_str()) {
            bail!("duplicate validation gate `{}`", gate.name);
        }
        let expected = format!("ci/scripts/{}.sh", gate.name);
        if gate.command != expected {
            bail!(
                "validation gate `{}` command must be `{expected}`, not `{}`",
                gate.name,
                gate.command
            );
        }
        if !commands.insert(gate.command.as_str()) {
            bail!("duplicate validation command `{}`", gate.command);
        }
        if gate.isolation.is_empty() {
            bail!(
                "validation gate `{}` has no isolation requirement",
                gate.name
            );
        }
        let isolation: BTreeSet<_> = gate.isolation.iter().copied().collect();
        if isolation.len() != gate.isolation.len() {
            bail!(
                "validation gate `{}` repeats an isolation requirement",
                gate.name
            );
        }
        if isolation.contains(&Isolation::None) && isolation.len() != 1 {
            bail!(
                "validation gate `{}` combines `none` with another isolation requirement",
                gate.name
            );
        }
        development_kinds.insert(development_name(gate.development));

        // These fields are one-variant enums on purpose. Deserialisation rejects
        // a subtractive spelling before this audit can accidentally ignore it.
        let FullMembership::Required = gate.full;
        let ReleaseMembership::Inherited = gate.release;
        let _ = gate.input_class;
    }
    for required in ["always", "selectable", "deferred"] {
        if !development_kinds.contains(required) {
            bail!("development profile has no `{required}` gate");
        }
    }
    validate_selection_shape(inventory)
}

fn validate_selection_shape(inventory: &Inventory) -> Result<()> {
    if inventory.selection.schema != 1 {
        bail!(
            "unsupported validation selection schema {} (expected 1)",
            inventory.selection.schema
        );
    }
    validate_selection_gate_references(inventory)?;
    validate_rust_selection(&inventory.selection.rust)?;
    validate_path_rules(inventory)
}

fn validate_selection_gate_references(inventory: &Inventory) -> Result<()> {
    let declared: BTreeSet<_> = inventory
        .gates
        .iter()
        .map(|gate| gate.name.as_str())
        .collect();
    let selectable: BTreeSet<_> = inventory
        .gates
        .iter()
        .filter(|gate| gate.development == DevelopmentMembership::Selectable)
        .map(|gate| gate.name.as_str())
        .collect();
    for (owner, gates) in std::iter::once(("selection.rust", &inventory.selection.rust.gates))
        .chain(std::iter::once((
            "selection.global_gates",
            &inventory.selection.global_gates,
        )))
        .chain(
            inventory
                .selection
                .paths
                .iter()
                .map(|rule| (rule.name.as_str(), &rule.gates)),
        )
    {
        let mut seen = BTreeSet::new();
        for gate in gates {
            if !declared.contains(gate.as_str()) {
                bail!("selection `{owner}` names unknown gate `{gate}`");
            }
            if !selectable.contains(gate.as_str()) {
                bail!("selection `{owner}` names non-selectable gate `{gate}`");
            }
            if !seen.insert(gate) {
                bail!("selection `{owner}` repeats gate `{gate}`");
            }
        }
    }
    if inventory.selection.global_gates.is_empty() {
        bail!("selection.global_gates must not be empty");
    }
    if BTreeSet::from_iter(inventory.selection.global_gates.iter().map(String::as_str))
        != selectable
    {
        bail!("selection.global_gates must name every selectable development gate exactly once");
    }
    Ok(())
}

fn validate_rust_selection(selection: &RustSelection) -> Result<()> {
    let mut cross_cutting = BTreeSet::new();
    for package in &selection.cross_cutting_packages {
        if !valid_name(package) {
            bail!("cross-cutting package name `{package}` is not kebab-case");
        }
        if !cross_cutting.insert(package) {
            bail!("cross-cutting package `{package}` is repeated");
        }
    }
    if selection.gates.is_empty() {
        bail!("selection.rust.gates must not be empty");
    }
    let mut global_basenames = BTreeSet::new();
    for basename in &selection.global_basenames {
        if basename.is_empty()
            || basename.contains('/')
            || basename == "."
            || basename == ".."
            || !global_basenames.insert(basename)
        {
            bail!("invalid or duplicate global Rust basename `{basename}`");
        }
    }
    if global_basenames.is_empty() {
        bail!("selection.rust.global_basenames must not be empty");
    }
    Ok(())
}

fn validate_path_rules(inventory: &Inventory) -> Result<()> {
    if inventory.selection.paths.is_empty() {
        bail!("selection.paths must not be empty");
    }
    let mut names = BTreeSet::new();
    let mut exact = BTreeSet::new();
    let mut prefixes = BTreeSet::new();
    for rule in &inventory.selection.paths {
        if !valid_name(&rule.name) || !names.insert(rule.name.as_str()) {
            bail!("selection path rule names must be unique kebab-case names");
        }
        if rule.exact.is_empty() && rule.prefixes.is_empty() {
            bail!("selection path rule `{}` matches no paths", rule.name);
        }
        if rule.disposition == PathDisposition::Global
            && BTreeSet::from_iter(rule.gates.iter())
                != BTreeSet::from_iter(inventory.selection.global_gates.iter())
        {
            bail!(
                "global selection rule `{}` must name exactly selection.global_gates",
                rule.name
            );
        }
        for path in &rule.exact {
            validate_relative_inventory_path(path, false)?;
            if !exact.insert(path) {
                bail!("selection path `{path}` is declared more than once");
            }
        }
        for prefix in &rule.prefixes {
            validate_relative_inventory_path(prefix, true)?;
            if !prefixes.insert(prefix) {
                bail!("selection prefix `{prefix}` is declared more than once");
            }
        }
    }
    Ok(())
}

fn validate_relative_inventory_path(path: &str, prefix: bool) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains("//")
        || path.split('/').any(|part| part == "." || part == "..")
        || path.contains('\\')
        || prefix != path.ends_with('/')
    {
        bail!("unsafe validation inventory path `{path}`");
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn development_name(membership: DevelopmentMembership) -> &'static str {
    match membership {
        DevelopmentMembership::Always => "always",
        DevelopmentMembership::Selectable => "selectable",
        DevelopmentMembership::Deferred => "deferred",
    }
}

fn audit_repository_wiring(root: &Path, inventory: &Inventory) -> Result<()> {
    let workflow = std::fs::read_to_string(root.join(WORKFLOW))
        .with_context(|| format!("reading {WORKFLOW}"))?;
    audit_workflow_text(&workflow, inventory)?;

    let preflight = std::fs::read_to_string(root.join(PREFLIGHT))
        .with_context(|| format!("reading {PREFLIGHT}"))?;
    if preflight.contains("PREFLIGHT_SKIP") {
        bail!("{PREFLIGHT} still exposes the subtractive PREFLIGHT_SKIP escape hatch");
    }
    let full_delegations = preflight
        .lines()
        .filter(|line| line.trim() == "exec \"$SCRIPT_DIR/validate.sh\" --profile full")
        .count();
    if full_delegations != 1 {
        bail!("{PREFLIGHT} does not delegate its exact contract to the full profile");
    }
    let development_delegations = preflight
        .lines()
        .filter(|line| line.trim() == "exec \"$SCRIPT_DIR/validate.sh\" \"${validation_args[@]}\"")
        .count();
    if development_delegations != 1 {
        bail!("{PREFLIGHT} does not delegate classified input to the development profile");
    }
    let selectors = preflight
        .lines()
        .filter(|line| {
            line.trim()
                == "python3 \"$SCRIPT_DIR/validation-select.py\" \"${selector_args[@]}\" --format arguments --timings"
        })
        .count();
    if selectors != 1 {
        bail!("{PREFLIGHT} does not run the versioned validation selector exactly once");
    }
    for gate in &inventory.gates {
        if preflight.contains(&gate.command) {
            bail!(
                "{PREFLIGHT} duplicates gate `{}` instead of using the inventory",
                gate.name
            );
        }
    }
    Ok(())
}

fn audit_workflow_text(workflow: &str, inventory: &Inventory) -> Result<()> {
    let document: yaml_compat::Value =
        yaml_compat::from_str(workflow).with_context(|| format!("parsing {WORKFLOW}"))?;
    let root = document
        .as_mapping()
        .with_context(|| format!("{WORKFLOW} must contain a YAML mapping"))?;
    let jobs = yaml_field(root, "jobs")
        .and_then(yaml_compat::Value::as_mapping)
        .with_context(|| format!("{WORKFLOW} must contain a `jobs` mapping"))?;
    let mut ordinary_driven = 0;
    let mut full_driven = 0;
    for (job_name, job) in jobs {
        let job_name = job_name
            .as_str()
            .with_context(|| format!("{WORKFLOW} has a non-string job name"))?;
        let job = job
            .as_mapping()
            .with_context(|| format!("{WORKFLOW} job `{job_name}` must be a mapping"))?;
        let Some(steps) = yaml_field(job, "steps") else {
            continue;
        };
        let steps = steps
            .as_sequence()
            .with_context(|| format!("{WORKFLOW} job `{job_name}` has non-sequence `steps`"))?;
        for (index, step) in steps.iter().enumerate() {
            let step = step.as_mapping().with_context(|| {
                format!(
                    "{WORKFLOW} job `{job_name}` step {} must be a mapping",
                    index + 1
                )
            })?;
            let Some(run) = yaml_field(step, "run") else {
                continue;
            };
            let run = run.as_str().with_context(|| {
                format!(
                    "{WORKFLOW} job `{job_name}` step {} has a non-string `run`",
                    index + 1
                )
            })?;

            for gate in &inventory.gates {
                if run.contains(&gate.command) {
                    bail!(
                        "{WORKFLOW} invokes `{}` directly instead of the profile-neutral entry point",
                        gate.command
                    );
                }
            }
            if !run.contains("ci/scripts/validate.sh") && !run.contains("ci/scripts/preflight.sh") {
                continue;
            }
            let profile = match run {
                ORDINARY_WORKFLOW_COMMAND => {
                    ordinary_driven += 1;
                    ordinary_profile(job)?
                }
                FULL_WORKFLOW_COMMAND => {
                    full_driven += 1;
                    "full"
                }
                _ => {
                    bail!(
                        "{WORKFLOW} job `{job_name}` step {} has a non-canonical validation invocation",
                        index + 1
                    )
                }
            };
            if yaml_field(step, "if").is_some()
                || yaml_field(step, "continue-on-error").is_some()
                || yaml_field(job, "continue-on-error").is_some()
            {
                bail!("{WORKFLOW} {profile} profile can be skipped or ignored by job/step policy");
            }
            if !has_full_history_checkout(&steps[..index]) {
                bail!(
                    "{WORKFLOW} {profile} profile requires a preceding checkout with `fetch-depth: 0`"
                );
            }
        }
    }
    match ordinary_driven {
        1 => {}
        0 => bail!("ordinary validation is not driven by {WORKFLOW}"),
        count => bail!("ordinary validation is driven {count} times by {WORKFLOW}"),
    }
    match full_driven {
        1 => {}
        0 => bail!("validation profile `full` is not driven by {WORKFLOW}"),
        count => bail!("validation profile `full` is driven {count} times by {WORKFLOW}"),
    }
    Ok(())
}

fn ordinary_profile(job: &yaml_compat::Mapping) -> Result<&str> {
    let profile = yaml_field(job, "env")
        .and_then(yaml_compat::Value::as_mapping)
        .and_then(|environment| yaml_field(environment, "APLAN_ORDINARY_PROFILE"))
        .and_then(yaml_compat::Value::as_str)
        .with_context(|| {
            format!(
                "{WORKFLOW} ordinary job must set APLAN_ORDINARY_PROFILE to `development` or `full`"
            )
        })?;
    match profile {
        "development" | "full" => Ok(profile),
        _ => bail!("{WORKFLOW} ordinary job has unknown APLAN_ORDINARY_PROFILE `{profile}`"),
    }
}

fn has_full_history_checkout(steps: &[yaml_compat::Value]) -> bool {
    // The last checkout is the effective working tree. An earlier full clone
    // does not satisfy the isolation contract if a later checkout replaces it
    // with a shallow one (or makes that replacement conditional).
    let Some(checkout) = steps.iter().rev().find_map(|step| {
        let step = step.as_mapping()?;
        yaml_field(step, "uses")
            .and_then(yaml_compat::Value::as_str)
            .is_some_and(|uses| uses.starts_with("actions/checkout@"))
            .then_some(step)
    }) else {
        return false;
    };
    let unconditional =
        yaml_field(checkout, "if").is_none() && yaml_field(checkout, "continue-on-error").is_none();
    let fetches_all = yaml_field(checkout, "with")
        .and_then(yaml_compat::Value::as_mapping)
        .and_then(|with| yaml_field(with, "fetch-depth"))
        .is_some_and(|depth| depth.as_u64() == Some(0) || depth.as_str() == Some("0"));
    unconditional && fetches_all
}

fn yaml_field<'a>(
    mapping: &'a yaml_compat::Mapping,
    field: &str,
) -> Option<&'a yaml_compat::Value> {
    mapping.get(yaml_compat::Value::String(field.to_owned()))
}

fn render(inventory: &Inventory) -> String {
    let mut out = String::from(
        "#!/usr/bin/env bash\n\
         # @generated by `cargo xtask validation`; edit ci/validation.yml.\n\
         # shellcheck shell=bash\n\n\
         readonly VALIDATION_SCHEMA=1\n\
         readonly VALIDATION_DEVELOPMENT_ASSURANCE='selected'\n\
         readonly VALIDATION_FULL_ASSURANCE='full'\n\
         readonly VALIDATION_RELEASE_ASSURANCE='release'\n\
         readonly VALIDATION_RELEASE_REQUIRES='full'\n\
         readonly VALIDATION_RELEASE_SAME_SOURCE=1\n\n",
    );
    let all: Vec<_> = inventory
        .gates
        .iter()
        .map(|gate| gate.name.as_str())
        .collect();
    let always: Vec<_> = inventory
        .gates
        .iter()
        .filter(|gate| gate.development == DevelopmentMembership::Always)
        .map(|gate| gate.name.as_str())
        .collect();
    let selectable: Vec<_> = inventory
        .gates
        .iter()
        .filter(|gate| gate.development == DevelopmentMembership::Selectable)
        .map(|gate| gate.name.as_str())
        .collect();
    let deferred: Vec<_> = inventory
        .gates
        .iter()
        .filter(|gate| gate.development == DevelopmentMembership::Deferred)
        .map(|gate| gate.name.as_str())
        .collect();
    render_array(&mut out, "VALIDATION_ALL_GATES", &all);
    render_array(&mut out, "VALIDATION_FULL_GATES", &all);
    render_array(&mut out, "VALIDATION_RELEASE_GATES", &all);
    render_array(&mut out, "VALIDATION_DEVELOPMENT_ALWAYS", &always);
    render_array(&mut out, "VALIDATION_DEVELOPMENT_SELECTABLE", &selectable);
    render_array(&mut out, "VALIDATION_DEVELOPMENT_DEFERRED", &deferred);
    out.push_str("declare -Ar VALIDATION_GATE_COMMANDS=(\n");
    for gate in &inventory.gates {
        writeln!(out, "    ['{}']='{}'", gate.name, gate.command)
            .expect("writing to a String cannot fail");
    }
    out.push_str(")\n");
    out.push_str("declare -Ar VALIDATION_GATE_INPUT_CLASSES=(\n");
    for gate in &inventory.gates {
        writeln!(
            out,
            "    ['{}']='{}'",
            gate.name,
            input_class_name(gate.input_class)
        )
        .expect("writing to a String cannot fail");
    }
    out.push_str(")\n");
    out
}

fn input_class_name(input_class: InputClass) -> &'static str {
    match input_class {
        InputClass::Repository => "repository",
        InputClass::RustWorkspace => "rust-workspace",
        InputClass::GeneratedContracts => "generated-contracts",
        InputClass::DeclaredMsrv => "declared-msrv",
        InputClass::DependencyPolicy => "dependency-policy",
        InputClass::PlanStructure => "plan-structure",
        InputClass::PlanAndEvidence => "plan-and-evidence",
    }
}

fn render_array(out: &mut String, name: &str, values: &[&str]) {
    writeln!(out, "readonly -a {name}=(").expect("writing to a String cannot fail");
    for value in values {
        writeln!(out, "    '{value}'").expect("writing to a String cannot fail");
    }
    out.push_str(")\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical() -> Inventory {
        yaml_compat::from_str(include_str!("../../ci/validation.yml"))
            .expect("canonical validation inventory parses")
    }

    fn workflow() -> String {
        format!(
            "jobs:\n  development:\n    if: github.event_name == 'push'\n    env:\n      APLAN_ORDINARY_PROFILE: development\n    steps:\n      - uses: actions/checkout@v7\n        with:\n          fetch-depth: 0\n      - run: {ORDINARY_WORKFLOW_COMMAND}\n  full:\n    if: github.event_name == 'schedule'\n    steps:\n      - uses: actions/checkout@v7\n        with:\n          fetch-depth: 0\n      - run: {FULL_WORKFLOW_COMMAND}\n"
        )
    }

    #[test]
    fn validation_inventory_orders_full_and_release_assurance() {
        let inventory = canonical();
        validate_inventory_shape(&inventory).expect("canonical inventory is valid");
        assert_eq!(inventory.profiles.full.assurance, Assurance::Full);
        assert_eq!(inventory.profiles.release.requires.as_deref(), Some("full"));
        assert!(inventory.profiles.release.same_source);
        assert!(
            inventory
                .gates
                .iter()
                .all(|gate| gate.full == FullMembership::Required
                    && gate.release == ReleaseMembership::Inherited)
        );
        let generated = render(&inventory);
        for gate in &inventory.gates {
            assert!(generated.contains(&format!("['{}']", gate.name)));
            assert!(generated.contains(&format!(
                "['{}']='{}'",
                gate.name,
                input_class_name(gate.input_class)
            )));
        }
    }

    #[test]
    fn generated_selector_graph_is_host_neutral_and_binds_every_workspace_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");
        let inventory = canonical();
        let source = include_bytes!("../../ci/validation.yml");
        let workspace = selector_workspace(root).expect("derive selector workspace graph");
        let rendered = render_selector_inventory(&inventory, source, &workspace)
            .expect("render selector inventory");
        assert!(
            !rendered.contains(root.to_str().expect("UTF-8 workspace root")),
            "generated policy must not retain the build host path"
        );
        let document: serde_json::Value =
            serde_json::from_str(&rendered).expect("parse rendered selector policy");
        let inputs = document["workspace"]["inputs"]
            .as_array()
            .expect("workspace inputs");
        let packages = document["workspace"]["packages"]
            .as_array()
            .expect("workspace packages");
        let bound: BTreeSet<_> = inputs
            .iter()
            .map(|input| input["path"].as_str().expect("input path"))
            .collect();
        for package in packages {
            let manifest = format!(
                "{}Cargo.toml",
                package["root"].as_str().expect("package root")
            );
            assert!(bound.contains(manifest.as_str()), "unbound `{manifest}`");
        }
        assert!(bound.contains("Cargo.toml"));
        assert!(bound.contains("Cargo.lock"));
        assert!(bound.contains("rust-toolchain.toml"));
        assert!(bound.contains(".cargo/config.toml"));
    }

    #[test]
    fn validation_inventory_rejects_duplicate_and_unknown_entries() {
        let source = include_str!("../../ci/validation.yml").replace("\r\n", "\n");
        let mut duplicate = canonical();
        duplicate.gates[1].name = duplicate.gates[0].name.clone();
        assert!(
            validate_inventory_shape(&duplicate)
                .unwrap_err()
                .to_string()
                .contains("duplicate validation gate")
        );

        let duplicate_profile = source.replacen(
            "profiles:\n",
            "profiles:\n  development:\n    assurance: selected\n",
            1,
        );
        let error = yaml_compat::from_str::<Inventory>(&duplicate_profile)
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate field `development`"), "{error}");

        let unknown_profile = source.replacen(
            "  full:\n",
            "  preview:\n    assurance: selected\n  full:\n",
            1,
        );
        let error = yaml_compat::from_str::<Inventory>(&unknown_profile)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown field `preview`"), "{error}");

        let unknown = source.replacen(
            "    assurance: selected",
            "    assurance: selected\n    mystery: true",
            1,
        );
        let error = yaml_compat::from_str::<Inventory>(&unknown)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown field `mystery`"), "{error}");
    }

    #[test]
    fn validation_inventory_rejects_unsafe_shapes() {
        let mut inventory = canonical();
        inventory.schema = 2;
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("unsupported validation inventory schema")
        );

        let mut inventory = canonical();
        inventory.profiles.development.same_source = true;
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("development must be")
        );

        let mut inventory = canonical();
        inventory.gates.clear();
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("at least one gate")
        );

        for isolation in [Vec::new(), vec![Isolation::None, Isolation::FullGitHistory]] {
            let mut inventory = canonical();
            inventory.gates[0].isolation = isolation;
            assert!(validate_inventory_shape(&inventory).is_err());
        }

        let mut inventory = canonical();
        inventory.selection.schema = 2;
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("selection schema")
        );

        let mut inventory = canonical();
        inventory.selection.rust.gates.clear();
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("rust.gates")
        );

        let mut inventory = canonical();
        inventory.selection.global_gates.pop();
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("every selectable development gate")
        );

        let mut inventory = canonical();
        inventory
            .selection
            .rust
            .cross_cutting_packages
            .push("aplan".to_owned());
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("is repeated")
        );

        let mut inventory = canonical();
        inventory.selection.paths[0].gates.pop();
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("must name exactly")
        );

        let mut inventory = canonical();
        inventory.selection.paths[0]
            .exact
            .push("/absolute".to_owned());
        assert!(
            validate_inventory_shape(&inventory)
                .unwrap_err()
                .to_string()
                .contains("unsafe validation inventory path")
        );
    }

    #[test]
    fn validation_workflow_audit_requires_both_canonical_profiles() {
        let inventory = canonical();
        let canonical = workflow();
        audit_workflow_text(&canonical, &inventory).expect("canonical workflow is valid");

        let rollback = canonical.replacen(
            "APLAN_ORDINARY_PROFILE: development",
            "APLAN_ORDINARY_PROFILE: full",
            1,
        );
        audit_workflow_text(&rollback, &inventory)
            .expect("one profile value performs the full-on-every-push rollback");

        let unknown_ordinary = canonical.replacen(
            "APLAN_ORDINARY_PROFILE: development",
            "APLAN_ORDINARY_PROFILE: preview",
            1,
        );
        let error = audit_workflow_text(&unknown_ordinary, &inventory)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown APLAN_ORDINARY_PROFILE"), "{error}");

        let incomplete = canonical
            .split_once("  full:\n")
            .expect("full job boundary")
            .0;
        let error = audit_workflow_text(incomplete, &inventory)
            .unwrap_err()
            .to_string();
        assert!(error.contains("profile `full` is not driven"), "{error}");

        let unknown_profile = canonical.replacen(
            FULL_WORKFLOW_COMMAND,
            "ci/scripts/validate.sh --profile preview",
            1,
        );
        let error = audit_workflow_text(&unknown_profile, &inventory)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("non-canonical validation invocation"),
            "{error}"
        );

        let direct = canonical.replace(
            &format!("      - run: {FULL_WORKFLOW_COMMAND}"),
            &format!(
                "      - run: {}\n      - run: {FULL_WORKFLOW_COMMAND}",
                inventory.gates[0].command
            ),
        );
        let error = audit_workflow_text(&direct, &inventory)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("directly instead of the profile-neutral entry point"),
            "{error}"
        );

        for policy in ["if: false", "continue-on-error: true"] {
            let conditional = canonical.replacen(
                &format!("      - run: {FULL_WORKFLOW_COMMAND}"),
                &format!("      - {policy}\n        run: {FULL_WORKFLOW_COMMAND}"),
                1,
            );
            let error = audit_workflow_text(&conditional, &inventory)
                .unwrap_err()
                .to_string();
            assert!(error.contains("can be skipped or ignored"), "{error}");
        }
    }

    #[test]
    fn validation_workflow_audit_rejects_ambiguous_execution_policy() {
        let inventory = canonical();
        let canonical = workflow();

        let malformed = canonical.replacen(
            FULL_WORKFLOW_COMMAND,
            &format!("bash {FULL_WORKFLOW_COMMAND}"),
            1,
        );
        let error = audit_workflow_text(&malformed, &inventory)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("non-canonical validation invocation"),
            "{error}"
        );

        let duplicate = canonical.replace(
            &format!("      - run: {FULL_WORKFLOW_COMMAND}"),
            &format!("      - run: {FULL_WORKFLOW_COMMAND}\n      - run: {FULL_WORKFLOW_COMMAND}"),
        );
        let error = audit_workflow_text(&duplicate, &inventory)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("profile `full` is driven 2 times"),
            "{error}"
        );

        let job_policy =
            canonical.replacen("  full:\n", "  full:\n    continue-on-error: true\n", 1);
        let error = audit_workflow_text(&job_policy, &inventory)
            .unwrap_err()
            .to_string();
        assert!(error.contains("can be skipped or ignored"), "{error}");

        let shallow = canonical.replacen("fetch-depth: 0", "fetch-depth: 1", 1);
        let error = audit_workflow_text(&shallow, &inventory)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "development profile requires a preceding checkout with `fetch-depth: 0`"
            ),
            "{error}"
        );

        let conditional_checkout = canonical.replacen(
            "      - uses: actions/checkout@v7",
            "      - uses: actions/checkout@v7\n        if: false",
            1,
        );
        let error = audit_workflow_text(&conditional_checkout, &inventory)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "development profile requires a preceding checkout with `fetch-depth: 0`"
            ),
            "{error}"
        );

        let superseded_checkout = canonical.replacen(
            &format!("      - run: {FULL_WORKFLOW_COMMAND}"),
            &format!("      - uses: actions/checkout@v7\n        with:\n          fetch-depth: 1\n      - run: {FULL_WORKFLOW_COMMAND}"),
            1,
        );
        let error = audit_workflow_text(&superseded_checkout, &inventory)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("full profile requires a preceding checkout with `fetch-depth: 0`"),
            "{error}"
        );
    }
}
