//! Build tasks for aplan.
//!
//! - `gen-schema`: the committed files under `schemas/v1/`, including task and
//!   capture records, are *derived* from Rust types, and `--check` fails when
//!   they disagree. That is the only reason the schemas can be trusted as
//!   documentation.
//! - `gen-roadmap`: the committed `ROADMAP.status.md` is *generated* from the plan
//!   by `aplan roadmap --format markdown`, and `--check` fails on drift (ADR-0009).
//!   The plan is the source of truth; that file is a generated view of it.
//! - `check-docs`: ROADMAP.md's hand-written narrative must agree with the plan
//!   it describes. `gen-roadmap` gates the *generated* companion; nothing gated
//!   the prose, and v0.6 closed while its narrative claimed seven deliverables
//!   that were never tasks (ROADMAP section 3 records the correction). It also
//!   checks the pilot friction logs (`private pre-public adoption records`), where an item claiming to be
//!   "tracked as A1-nnn" when no such task exists reads as handled, which is worse
//!   than an item with no disposition at all.
//! - `lint`: the house-style gate. It forbids the typographic characters that
//!   read as machine-generated (em and en dashes, curly quotes, the ellipsis),
//!   the non-ASCII dash variants that stand in for a plain `-`, and the invisible
//!   zero-width and exotic-space characters (which hide in a diff and double as a
//!   trojan-source vector), so the repo stays plain-ASCII. Functional Unicode is
//!   deliberately untouched: the arrow used in cycle output, the section sign in
//!   doc cross-references, and the non-ASCII letters in the slug tests are fine.
//! - `validation`: derive the lightweight runtime gate table and selector policy
//!   from `ci/validation.yml`, then audit that local preflight and Actions drive them.
//! - `validation-select`: compatibility launcher for the compiler-free production
//!   selector. Preflight invokes that script directly and pays no `xtask` build.
//! - `tui-reference`: derive the committed key/action and explicit-boundary
//!   reference from the same executable catalogues as TUI help and dispatch.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// A function that derives a JSON Schema from a Rust type.
type SchemaFn = fn() -> serde_json::Value;

/// The committed schemas, each derived from a Rust type. Adding a schema-derived
/// type is one row here.
const SCHEMAS: &[(&str, SchemaFn)] = &[
    (
        "schemas/v1/test-evidence-receipt.json",
        aplan_attest::receipt::derived_schema,
    ),
    (
        "schemas/v1/test-evidence-import-set.json",
        aplan_attest::receipt::derived_import_set_schema,
    ),
    (
        "schemas/v1/blueprint.json",
        aplan_interop::blueprint::derived_schema,
    ),
    (
        "schemas/v1/capture.json",
        aplan_core::capture::derived_schema,
    ),
    ("schemas/v1/task.json", aplan_core::load::derived_schema),
    ("schemas/v1/config.json", aplan_config::derived_schema),
    // The per-user file is a second schema because it is a second *type*, not a
    // subset of the first: it holds nothing any rule reads, which is what keeps
    // it from weakening a repository's policy (`A1-364`).
    (
        "schemas/v1/user-config.json",
        aplan_config::user::derived_schema,
    ),
    ("schemas/v1/tasks.json", aplan_core::export::derived_schema),
];

mod cli_surface;
mod dependency_audit;
mod msrv;
mod tui_eval_launcher;
mod validation;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let rest: Vec<String> = args.collect();
    let check = rest.iter().any(|a| a == "--check");
    let allow_regression = rest.iter().any(|a| a == "--allow-regression");

    match cmd.as_deref() {
        Some("gen-schema") => gen_schema(check),
        Some("gen-roadmap") => gen_roadmap(check),
        Some("tui-reference") => tui_reference(check),
        Some("lint") => lint(),
        Some("check-docs") => check_docs(),
        Some("cli-surface") => cli_surface_gate(check),
        Some("coverage") => coverage(check, allow_regression),
        Some("msrv") => msrv::gate(&workspace_root()?),
        Some("dependency-audit") => dependency_audit::gate(&workspace_root()?),
        Some("validation") => validation::command(&rest, &workspace_root()?),
        Some("validation-select") => validation::selector_command(&rest, &workspace_root()?),
        Some("tui-eval") => tui_eval_launcher::command(&rest, &workspace_root()?),
        Some(other) => bail!(
            "unknown xtask `{other}` (expected: gen-schema [--check], gen-roadmap [--check], tui-reference [--check], lint, check-docs, msrv, dependency-audit, validation [--check], validation-select [arguments], coverage [--check | --allow-regression], tui-eval <probe|canary|review-self-test|review|review-export|review-audit> [arguments])"
        ),
        None => {
            eprintln!(
                "usage: cargo xtask <gen-schema [--check] | gen-roadmap [--check] | tui-reference [--check] | lint | check-docs | msrv | dependency-audit | validation [--check] | validation-select [arguments] | coverage [--check | --allow-regression] | tui-eval <probe|canary|review-self-test|review|review-export|review-audit> [arguments]>"
            );
            Ok(())
        }
    }
}

fn tui_reference(check: bool) -> Result<()> {
    let root = workspace_root()?;
    let rendered = aplan_tui::generated_action_reference_markdown();
    write_or_check(
        &root,
        "docs/tui-reference.md",
        &rendered,
        check,
        "tui-reference",
    )
}

/// The committed per-crate line-coverage floors.
const COVERAGE_FLOORS: &str = "coverage.json";

/// The dedicated directory cargo-llvm-cov uses for instrumented objects.
///
/// Isolation may be anchored only at a path with this final component. The
/// command creates a private parent beside it and recursively removes only a
/// same-named child it created there. That keeps an empty, malformed or overly
/// broad environment override from publication cleanup at an ordinary Cargo
/// target, a workspace, or a shared cache (`A1-457`).
const LLVM_COV_TARGET: &str = "llvm-cov-target";

/// Bound libtest fan-out inside the instrumented coverage process (`A1-603`).
///
/// Coverage makes every test and child process materially more expensive. The
/// ordinary host-sized libtest schedule repeatedly starved process-sensitive
/// fixtures after they had published an idle screen or before they could
/// publish readiness. Four retains useful parallelism without oversubscribing
/// the four-vCPU public runner we will use after publication.
const COVERAGE_TEST_THREADS: &str = "4";

/// Ownership record inside each private coverage allocation (`A1-633`).
///
/// The basename's PID is useful to an operator, but is not an ownership proof:
/// the PID can be reused, and a killed parent can leave Cargo or a test alive.
/// This record binds the PID to its Linux boot/start identity and gives the
/// complete child tree an inherited lease. A later run removes the directory
/// only when every recorded signal says that tree is gone.
const COVERAGE_OWNER: &str = ".aplan-coverage-owner-v1.json";
const COVERAGE_LEASE_ENV: &str = "APLAN_COVERAGE_LEASE";
const MAX_COVERAGE_OWNER_BYTES: u64 = 16 * 1024;

static COVERAGE_LEASE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(unix)]
static COVERAGE_INTERRUPTED_BY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// How far a crate may rise above its floor before the floor is called stale.
///
/// A floor nobody raises stops being a floor: it drifts into a number that was
/// true once, and new untested code hides in the gap it left behind. Two points
/// is wide enough that ordinary churn does not trip it and narrow enough that a
/// crate cannot quietly bank a large surplus.
const RATCHET_SLACK: f64 = 2.0;

/// Line coverage per crate, measured by `cargo llvm-cov`, gated against
/// [`COVERAGE_FLOORS`].
///
/// **What this gate does and does not prove.** It catches code that no test
/// executes at all, which is a real and recurring failure here: `check-roadmap`
/// shipped with no tests, and `xtask` is still the least-covered crate in the
/// workspace. It proves nothing about whether the lines that ran were *checked*.
/// `APLAN023` shipped with full line coverage of its rule and three defects in
/// the inputs next to the one it was written for (`A1-043`, `A1-046`). Treat a
/// green number as the floor it is, never as evidence the tests are thorough.
fn coverage(check: bool, allow_regression: bool) -> Result<()> {
    let root = workspace_root()?;
    let measured = measure_coverage(&root);
    #[cfg(unix)]
    resume_coverage_signal();
    let measured = measured?;
    let measured_pct: std::collections::BTreeMap<String, f64> = measured
        .iter()
        .map(|(k, v)| (k.clone(), as_f64(&pct(*v))))
        .collect();
    let committed = read_floors(&root).unwrap_or_default();

    // Always show the numbers, both when passing and when failing. A gate that
    // prints only a verdict cannot be audited from a CI log: a measurement that
    // silently covered a fraction of the workspace would read exactly like a
    // good one, and the first version of this command did precisely that.
    for (name, actual) in &measured_pct {
        let floor = committed.get(name).copied();
        let (covered, total) = measured[name];
        let against = floor.map_or_else(|| "no floor".to_string(), |f| format!("floor {f:.0}%"));
        println!("  {name:<16} {actual:>6.1}%  ({covered}/{total} lines, {against})");
    }

    if !check {
        // Regenerating is the documented fix for a floor that has been outgrown,
        // which makes it the obvious thing to run when the gate complains at all.
        // Left unguarded, that same keystroke silently rewrites a floor *downward*
        // after a regression, so the remedy for one failure is the bypass for the
        // other. Lowering has to be asked for.
        let new_floors: std::collections::BTreeMap<String, f64> = measured
            .iter()
            .map(|(k, v)| (k.clone(), as_f64(&floor_pct(*v))))
            .collect();
        let dropping = floors_that_would_drop(&new_floors, &committed);
        if !dropping.is_empty() && !allow_regression {
            bail!(
                "refusing to lower a coverage floor:\n  {}\nCoverage went down. Add tests, or \
                 pass --allow-regression to accept it and say why in the commit.",
                dropping.join("\n  ")
            );
        }
        let mut rendered = serde_json::to_string_pretty(&serde_json::json!({
            "//": "Per-crate minimum line coverage, enforced by `cargo xtask coverage --check`. \
                   Generated by `cargo xtask coverage`, which refuses to lower a floor without \
                   --allow-regression, so a drop is always a deliberate, reviewable diff.",
            "crates": measured.iter().map(|(k, v)| (k.clone(), floor_pct(*v)))
                .collect::<serde_json::Map<_, _>>(),
        }))?;
        rendered.push('\n');
        return write_or_check(&root, COVERAGE_FLOORS, &rendered, false, "coverage");
    }

    let committed = read_floors(&root)?;
    let problems = compare_floors(&measured_pct, &committed);
    if !problems.is_empty() {
        bail!(
            "coverage floors not met:\n  {}\n{}",
            problems.join("\n  "),
            checked_by()
        );
    }
    println!(
        "coverage: {} crates at or just above their floors",
        measured.len()
    );
    Ok(())
}

/// Which floors a regeneration would move *down*, and by how much.
///
/// The guard that makes lowering deliberate. A crate that is measured but has no
/// committed floor yet is not a drop, and a floor whose crate has disappeared is
/// [`compare_floors`]'s business, not this one.
fn floors_that_would_drop(
    new_floors: &std::collections::BTreeMap<String, f64>,
    committed: &std::collections::BTreeMap<String, f64>,
) -> Vec<String> {
    committed
        .iter()
        .filter_map(|(name, &floor)| {
            let new = *new_floors.get(name)?;
            (new < floor).then(|| format!("{name}: {floor:.0}% -> {new:.0}%"))
        })
        .collect()
}

/// The committed floors, as a map. Missing or malformed is an error for
/// `--check` and merely "nothing to compare against" when generating.
fn read_floors(root: &Path) -> Result<std::collections::BTreeMap<String, f64>> {
    let text = std::fs::read_to_string(root.join(COVERAGE_FLOORS))
        .with_context(|| format!("reading {COVERAGE_FLOORS}"))?;
    let json: serde_json::Value = serde_json::from_str(&text)?;
    let crates = json
        .get("crates")
        .and_then(serde_json::Value::as_object)
        .context("coverage.json has no `crates` object")?;
    Ok(crates.iter().map(|(k, v)| (k.clone(), as_f64(v))).collect())
}

/// Compare measured coverage against the committed floors, in both directions.
///
/// Separated from the `cargo llvm-cov` call so the decisions are testable without
/// a five-minute instrumented build; the measurement is I/O, this is the policy.
fn compare_floors(
    measured: &std::collections::BTreeMap<String, f64>,
    floors: &std::collections::BTreeMap<String, f64>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, &actual) in measured {
        let Some(&floor) = floors.get(name) else {
            problems.push(format!(
                "{name}: no floor committed (measured {actual:.1}%). Run `cargo xtask coverage`."
            ));
            continue;
        };
        if actual < floor {
            problems.push(format!(
                "{name}: {actual:.1}% is below its floor of {floor:.1}%. Add tests, or lower the \
                 floor deliberately and say why in the commit."
            ));
        } else if actual >= floor + RATCHET_SLACK {
            problems.push(format!(
                "{name}: {actual:.1}% has outgrown its floor of {floor:.1}%. Run \
                 `cargo xtask coverage` to raise it, so the gain cannot be lost silently."
            ));
        }
    }
    // A floor for a crate that no longer exists is a claim about nothing, and it
    // would go on passing forever.
    for name in floors.keys() {
        if !measured.contains_key(name) {
            problems.push(format!(
                "{name}: has a floor but was not measured (renamed or removed?). Run \
                 `cargo xtask coverage`."
            ));
        }
    }
    problems
}

/// Run `cargo llvm-cov` and total the lines per crate.
///
/// Summed from the per-file counts rather than averaged from the per-file
/// percentages: a 3-line file at 0% and a 900-line file at 99% do not average to
/// anything meaningful, and averaging would let a large regression hide behind a
/// crate's many small, fully-covered modules.
fn measure_coverage(root: &Path) -> Result<std::collections::BTreeMap<String, (u64, u64)>> {
    let configured_dirs = coverage_artifact_dirs(root)?;
    // Install before allocation rather than only around the Cargo child. A
    // catchable signal in the create/cleanup windows must still flow through
    // ordinary cleanup; hard kills remain the next-run recovery case.
    #[cfg(unix)]
    let signal_owner = CoverageSignalOwner::install()
        .context("installing coverage termination handlers before allocating artifacts")?;
    let measured = with_isolated_coverage_artifacts(&configured_dirs, |artifact_dirs| {
        run_coverage(
            root,
            artifact_dirs,
            #[cfg(unix)]
            &signal_owner,
        )
    });
    #[cfg(unix)]
    drop(signal_owner);
    measured
}

/// Resolve the directories cargo-llvm-cov would use, through the same Cargo
/// metadata and environment precedence it uses.
///
/// `CARGO_TARGET_DIR` may point outside the checkout, and Cargo configuration
/// may give build artifacts a separate directory. Guessing from the workspace
/// path therefore either misses the stale objects or deletes the wrong tree.
fn coverage_artifact_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .current_dir(root)
        .output()
        .context("identifying cargo-llvm-cov's artifact directories with `cargo metadata`")?;
    if !out.status.success() {
        bail!(
            "cannot identify cargo-llvm-cov's artifact directories, so coverage was not run:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let metadata: serde_json::Value = serde_json::from_slice(&out.stdout)
        .context("parsing `cargo metadata` while identifying coverage artifacts")?;
    let llvm_target = cargo_llvm_cov_env("CARGO_LLVM_COV_TARGET_DIR")?;
    let llvm_build = cargo_llvm_cov_env("CARGO_LLVM_COV_BUILD_DIR")?;
    coverage_artifact_dirs_from_metadata(
        root,
        &metadata,
        llvm_target.as_deref(),
        llvm_build.as_deref(),
    )
}

/// Read an override with cargo-llvm-cov's own empty/unicode semantics.
fn cargo_llvm_cov_env(key: &str) -> Result<Option<String>> {
    match std::env::var(key) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("reading cargo-llvm-cov override `{key}`"))
        }
    }
}

/// Pure half of [`coverage_artifact_dirs`], kept separate so unsafe path shapes
/// and external Cargo targets can be exercised without deleting real builds.
fn coverage_artifact_dirs_from_metadata(
    root: &Path,
    metadata: &serde_json::Value,
    llvm_target: Option<&str>,
    llvm_build: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    if let Some(raw_target) = llvm_target {
        let target =
            dedicated_coverage_dir(root, Path::new(raw_target), "CARGO_LLVM_COV_TARGET_DIR")?;
        let build = if let Some(raw_build) = llvm_build {
            dedicated_coverage_dir(root, Path::new(raw_build), "CARGO_LLVM_COV_BUILD_DIR")?
        } else {
            target.clone()
        };
        dirs.push(target);
        if !dirs.contains(&build) {
            dirs.push(build);
        }
        return Ok(dirs);
    }

    // cargo-llvm-cov ignores CARGO_LLVM_COV_BUILD_DIR unless its target-dir
    // override is also present. Mirror that rather than cleaning a directory
    // the ensuing command will not use.
    let target = metadata_directory(metadata, "target_directory")?;
    let build = if metadata.get("build_directory").is_some() {
        metadata_directory(metadata, "build_directory")?
    } else {
        target.clone()
    };
    let target = dedicated_coverage_dir(
        root,
        &target.join(LLVM_COV_TARGET),
        "cargo metadata target_directory",
    )?;
    let build = dedicated_coverage_dir(
        root,
        &build.join(LLVM_COV_TARGET),
        "cargo metadata build_directory",
    )?;
    dirs.push(target);
    if !dirs.contains(&build) {
        dirs.push(build);
    }
    Ok(dirs)
}

fn metadata_directory(metadata: &serde_json::Value, key: &str) -> Result<PathBuf> {
    let raw = metadata
        .get(key)
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("`cargo metadata` returned no `{key}` directory"))?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() || raw.is_empty() {
        bail!("`cargo metadata` returned unsafe `{key}` directory `{raw}`");
    }
    Ok(path)
}

/// Resolve and bound the location beside which an isolated run is allocated.
fn dedicated_coverage_dir(root: &Path, raw: &Path, source: &str) -> Result<PathBuf> {
    if raw.as_os_str().is_empty() {
        bail!("{source} is empty; refusing to guess what coverage artifacts to remove");
    }
    let path = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || path.file_name() != Some(std::ffi::OsStr::new(LLVM_COV_TARGET))
    {
        bail!(
            "{source} resolves to `{}`, which is not a dedicated `{LLVM_COV_TARGET}` directory; \
             refusing recursive cleanup. Point it at a path ending in `{LLVM_COV_TARGET}` or \
             unset the override",
            path.display()
        );
    }
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageProcessIdentity {
    pid: u32,
    boot_id: String,
    start_ticks: u64,
    uid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum CoverageOwnerPhase {
    Prepared,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageOwnerRecord {
    version: u8,
    phase: CoverageOwnerPhase,
    owner: CoverageProcessIdentity,
    child: Option<CoverageProcessIdentity>,
    process_group: Option<u32>,
    lease: String,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoverageProcessState {
    Gone,
    Matches,
    Reused,
    Unverifiable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoverageTreeState {
    Gone,
    #[cfg(target_os = "linux")]
    Live,
    Unverifiable,
}

fn new_coverage_owner_record() -> Result<CoverageOwnerRecord> {
    let owner = current_coverage_process_identity()?;
    let sequence = COVERAGE_LEASE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let lease = format!(
        "aplan-coverage-v1-{}-{}-{sequence}",
        owner.pid, owner.start_ticks
    );
    Ok(CoverageOwnerRecord {
        version: 1,
        phase: CoverageOwnerPhase::Prepared,
        owner,
        child: None,
        process_group: None,
        lease,
    })
}

#[cfg(target_os = "linux")]
fn linux_boot_id() -> Result<String> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .context("reading the Linux boot identity for coverage ownership")?;
    let id = id.trim();
    if !valid_coverage_boot_id(id) {
        bail!("the Linux boot identity is empty or malformed");
    }
    Ok(id.to_string())
}

#[cfg(target_os = "linux")]
fn proc_uid(status: &str) -> Result<u32> {
    let line = status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .context("/proc status has no Uid field")?;
    line.split_whitespace()
        .nth(1)
        .context("/proc status has an empty Uid field")?
        .parse()
        .context("parsing the real UID from /proc status")
}

#[cfg(target_os = "linux")]
fn proc_start_and_group(stat: &str) -> Result<(u64, u32)> {
    // `comm` is parenthesized and may itself contain spaces or `)`, so split at
    // the final delimiter. The remainder starts at field 3 (`state`); process
    // group is field 5/index 2 and starttime is field 22/index 19.
    let (_, fields) = stat
        .rsplit_once(") ")
        .context("/proc stat has no final command delimiter")?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    let process_group = fields
        .get(2)
        .context("/proc stat has no process-group field")?
        .parse()
        .context("parsing the process group from /proc stat")?;
    let start_ticks = fields
        .get(19)
        .context("/proc stat has no starttime field")?
        .parse()
        .context("parsing the starttime from /proc stat")?;
    Ok((start_ticks, process_group))
}

#[cfg(target_os = "linux")]
fn proc_entry_is_gone(error: &std::io::Error) -> bool {
    // procfs may report ESRCH when a process disappears after directory
    // enumeration but before a file read. Linux fixes ESRCH at errno 3; it is
    // not mapped to ErrorKind::NotFound by every Rust/libc combination.
    const LINUX_ESRCH: i32 = 3;
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(LINUX_ESRCH)
}

#[cfg(target_os = "linux")]
fn linux_process_details(
    pid: u32,
    boot_id: &str,
) -> Result<Option<(CoverageProcessIdentity, u32)>> {
    let directory = PathBuf::from(format!("/proc/{pid}"));
    let stat = match std::fs::read_to_string(directory.join("stat")) {
        Ok(stat) => stat,
        Err(error) if proc_entry_is_gone(&error) => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading /proc/{pid}/stat")),
    };
    let status = match std::fs::read_to_string(directory.join("status")) {
        Ok(status) => status,
        Err(error) if proc_entry_is_gone(&error) => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading /proc/{pid}/status")),
    };
    let (start_ticks, process_group) = proc_start_and_group(&stat)?;
    Ok(Some((
        CoverageProcessIdentity {
            pid,
            boot_id: boot_id.to_string(),
            start_ticks,
            uid: proc_uid(&status)?,
        },
        process_group,
    )))
}

#[cfg(target_os = "linux")]
fn current_coverage_process_identity() -> Result<CoverageProcessIdentity> {
    let boot_id = linux_boot_id()?;
    linux_process_details(std::process::id(), &boot_id)?
        .map(|(identity, _)| identity)
        .context("the current process disappeared while recording coverage ownership")
}

#[cfg(not(target_os = "linux"))]
#[allow(clippy::unnecessary_wraps)] // Keep one fallible ownership interface on every platform.
fn current_coverage_process_identity() -> Result<CoverageProcessIdentity> {
    // Other platforms still get private allocations and ordinary cleanup. A
    // later process cannot prove a stale tree from portable Rust APIs, so its
    // sweep retains the record instead of treating an unknown PID as dead.
    Ok(CoverageProcessIdentity {
        pid: std::process::id(),
        boot_id: "unavailable".to_string(),
        start_ticks: 0,
        uid: 0,
    })
}

#[cfg(target_os = "linux")]
fn coverage_process_state(expected: &CoverageProcessIdentity) -> CoverageProcessState {
    let Ok(boot_id) = linux_boot_id() else {
        return CoverageProcessState::Unverifiable;
    };
    if boot_id != expected.boot_id {
        return CoverageProcessState::Gone;
    }
    match linux_process_details(expected.pid, &boot_id) {
        Ok(None) => CoverageProcessState::Gone,
        Ok(Some((actual, _))) if actual == *expected => CoverageProcessState::Matches,
        Ok(Some(_)) => CoverageProcessState::Reused,
        Err(_) => CoverageProcessState::Unverifiable,
    }
}

#[cfg(target_os = "linux")]
fn coverage_descendants_state(record: &CoverageOwnerRecord) -> CoverageTreeState {
    let Ok(boot_id) = linux_boot_id() else {
        return CoverageTreeState::Unverifiable;
    };
    if boot_id != record.owner.boot_id {
        return CoverageTreeState::Gone;
    }

    let lease = format!("{COVERAGE_LEASE_ENV}={}", record.lease).into_bytes();
    let mut uncertain = false;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return CoverageTreeState::Unverifiable;
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if proc_entry_is_gone(&error) => continue,
            Err(_) => {
                uncertain = true;
                continue;
            }
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let details = match linux_process_details(pid, &boot_id) {
            Ok(Some(details)) => details,
            Ok(None) => continue,
            Err(_) => {
                uncertain = true;
                continue;
            }
        };
        let (identity, process_group) = details;
        if identity.uid != record.owner.uid {
            continue;
        }
        if record.process_group == Some(process_group) {
            return CoverageTreeState::Live;
        }
        // Linux may deny `environ` for an unrelated same-UID process under
        // ptrace policy. That says nothing about this allocation. The recorded
        // child identity and complete process-group scan remain authoritative;
        // an inherited readable lease additionally catches descendants that
        // deliberately moved groups.
        if let Ok(environment) = std::fs::read(entry.path().join("environ"))
            && environment
                .split(|byte| *byte == 0)
                .any(|value| value == lease)
        {
            return CoverageTreeState::Live;
        }
    }
    if uncertain {
        CoverageTreeState::Unverifiable
    } else {
        CoverageTreeState::Gone
    }
}

#[cfg(target_os = "linux")]
fn coverage_tree_state(record: &CoverageOwnerRecord) -> CoverageTreeState {
    match coverage_process_state(&record.owner) {
        CoverageProcessState::Matches => return CoverageTreeState::Live,
        CoverageProcessState::Reused | CoverageProcessState::Unverifiable => {
            return CoverageTreeState::Unverifiable;
        }
        CoverageProcessState::Gone => {}
    }
    if let Some(child) = &record.child {
        match coverage_process_state(child) {
            CoverageProcessState::Matches => return CoverageTreeState::Live,
            CoverageProcessState::Reused | CoverageProcessState::Unverifiable => {
                return CoverageTreeState::Unverifiable;
            }
            CoverageProcessState::Gone => {}
        }
    }
    coverage_descendants_state(record)
}

#[cfg(not(target_os = "linux"))]
fn coverage_tree_state(_record: &CoverageOwnerRecord) -> CoverageTreeState {
    CoverageTreeState::Unverifiable
}

fn valid_coverage_lease(lease: &str) -> bool {
    !lease.is_empty()
        && lease.len() <= 256
        && lease
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(target_os = "linux")]
fn valid_coverage_boot_id(boot_id: &str) -> bool {
    boot_id.len() == 36
        && boot_id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

#[cfg(not(target_os = "linux"))]
fn valid_coverage_boot_id(boot_id: &str) -> bool {
    boot_id == "unavailable"
}

#[cfg(target_os = "linux")]
fn valid_coverage_identity(identity: &CoverageProcessIdentity) -> bool {
    identity.pid != 0 && identity.start_ticks != 0 && valid_coverage_boot_id(&identity.boot_id)
}

#[cfg(not(target_os = "linux"))]
fn valid_coverage_identity(identity: &CoverageProcessIdentity) -> bool {
    identity.pid != 0
        && identity.start_ticks == 0
        && identity.uid == 0
        && valid_coverage_boot_id(&identity.boot_id)
}

fn consistent_coverage_owner(record: &CoverageOwnerRecord) -> bool {
    if record.version != 1
        || !valid_coverage_lease(&record.lease)
        || !valid_coverage_identity(&record.owner)
    {
        return false;
    }
    match record.phase {
        CoverageOwnerPhase::Prepared => record.child.is_none() && record.process_group.is_none(),
        CoverageOwnerPhase::Running => record.child.as_ref().is_some_and(|child| {
            valid_coverage_identity(child)
                && child.boot_id == record.owner.boot_id
                && child.uid == record.owner.uid
                && record.process_group == Some(child.pid)
        }),
    }
}

fn read_coverage_owner(run_dir: &Path) -> Result<CoverageOwnerRecord> {
    use std::io::Read as _;

    let path = run_dir.join(COVERAGE_OWNER);
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("inspecting coverage owner record `{}`", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_COVERAGE_OWNER_BYTES
    {
        bail!(
            "coverage owner record `{}` is not a bounded real file",
            path.display()
        );
    }
    let file = std::fs::File::open(&path)
        .with_context(|| format!("opening coverage owner record `{}`", path.display()))?;
    let opened = file
        .metadata()
        .with_context(|| format!("identifying coverage owner record `{}`", path.display()))?;
    if !same_file_identity(&metadata, &opened) {
        bail!(
            "coverage owner record `{}` changed while it was opened",
            path.display()
        );
    }
    if opened.len() > MAX_COVERAGE_OWNER_BYTES {
        bail!(
            "coverage owner record `{}` is not a bounded real file",
            path.display()
        );
    }
    let capacity = usize::try_from(opened.len()).context("coverage owner record is too large")?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_COVERAGE_OWNER_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading coverage owner record `{}`", path.display()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_COVERAGE_OWNER_BYTES {
        bail!(
            "coverage owner record `{}` grew beyond its size bound while being read",
            path.display()
        );
    }
    let record: CoverageOwnerRecord = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing coverage owner record `{}`", path.display()))?;
    if !consistent_coverage_owner(&record) {
        bail!("coverage owner record `{}` is inconsistent", path.display());
    }
    Ok(record)
}

fn write_coverage_owner(run_dir: &Path, record: &CoverageOwnerRecord) -> Result<()> {
    use std::io::Write as _;

    let sequence = COVERAGE_LEASE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = run_dir.join(format!(
        ".{COVERAGE_OWNER}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let destination = run_dir.join(COVERAGE_OWNER);
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("creating coverage owner record `{}`", temporary.display()))?;
        let mut bytes = serde_json::to_vec_pretty(record)?;
        bytes.push(b'\n');
        file.write_all(&bytes)
            .with_context(|| format!("writing coverage owner record `{}`", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing coverage owner record `{}`", temporary.display()))?;
        drop(file);
        std::fs::rename(&temporary, &destination).with_context(|| {
            format!(
                "publishing coverage owner record `{}`",
                destination.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn coverage_candidate_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(".aplan-coverage-")?;
    let (pid, attempt) = rest.split_once('-')?;
    if pid.starts_with('0')
        || pid.is_empty()
        || !pid.bytes().all(|byte| byte.is_ascii_digit())
        || attempt.is_empty()
        || (attempt.len() > 1 && attempt.starts_with('0'))
        || !attempt.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let pid = pid.parse().ok()?;
    (pid != 0).then_some(pid)
}

fn same_file_identity(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        before.dev() == after.dev() && before.ino() == after.ino()
    }
    #[cfg(not(unix))]
    {
        before.is_dir() == after.is_dir() && before.len() == after.len()
    }
}

fn sweep_stale_coverage_runs(parent: &Path) -> Result<usize> {
    let mut removed = 0;
    for entry in std::fs::read_dir(parent)
        .with_context(|| format!("scanning coverage allocations in `{}`", parent.display()))?
    {
        let entry = entry
            .with_context(|| format!("reading a coverage allocation in `{}`", parent.display()))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(pid) = coverage_candidate_pid(&name) else {
            continue;
        };
        let path = entry.path();
        let Ok(before) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !before.is_dir() || before.file_type().is_symlink() {
            continue;
        }
        let Ok(record) = read_coverage_owner(&path) else {
            // Missing, partial, hostile and future-version records are all
            // unverifiable. Retention is the safe answer.
            continue;
        };
        if record.owner.pid != pid || coverage_tree_state(&record) != CoverageTreeState::Gone {
            continue;
        }
        let Ok(after) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !after.is_dir() || after.file_type().is_symlink() {
            continue;
        }
        if !same_file_identity(&before, &after) {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                eprintln!("coverage: removed stale allocation `{}`", path.display());
                removed += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("removing stale coverage allocation `{}`", path.display())
                });
            }
        }
    }
    Ok(removed)
}

/// Give one measurement artifact directories no earlier or concurrent run can
/// share, and remove only those run-owned directories afterwards.
///
/// Cleaning a shared directory and then starting Cargo is not an isolation
/// boundary: another invocation can recreate it between those operations. Each
/// run therefore atomically creates a private parent beside the configured
/// `llvm-cov-target`, puts one identically named child inside it, and overrides
/// cargo-llvm-cov to use that child. Recursive removal is still restricted to a
/// final component we created; the private parent is removed non-recursively.
fn with_isolated_coverage_artifacts<T>(
    configured_dirs: &[PathBuf],
    measure: impl FnOnce(&[PathBuf]) -> Result<T>,
) -> Result<T> {
    let artifact_dirs = create_isolated_coverage_artifacts(configured_dirs)?;
    let measured = measure(&artifact_dirs);
    let cleaned = remove_isolated_coverage_artifacts(&artifact_dirs);
    match (measured, cleaned) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(measure_error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{measure_error:#}\nadditionally, coverage artifact cleanup failed: {cleanup_error:#}"
        )),
    }
}

fn create_isolated_coverage_artifacts(configured_dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    if configured_dirs.is_empty() {
        bail!("coverage isolation was given no configured artifact directory");
    }
    let owner = new_coverage_owner_record()?;
    let mut artifacts = Vec::new();
    let mut swept = std::collections::BTreeSet::new();
    for configured in configured_dirs {
        if configured.file_name() != Some(std::ffi::OsStr::new(LLVM_COV_TARGET)) {
            let _ = remove_isolated_coverage_artifacts(&artifacts);
            bail!(
                "coverage artifact path `{}` is not a dedicated `{LLVM_COV_TARGET}` directory",
                configured.display()
            );
        }
        let parent = configured.parent().with_context(|| {
            format!(
                "coverage artifact path `{}` has no parent for an isolated run",
                configured.display()
            )
        })?;
        if let Err(error) = std::fs::create_dir_all(parent) {
            let _ = remove_isolated_coverage_artifacts(&artifacts);
            return Err(error).with_context(|| {
                format!(
                    "creating the parent of isolated coverage artifacts at `{}`",
                    parent.display()
                )
            });
        }
        if swept.insert(parent.to_path_buf())
            && let Err(error) = sweep_stale_coverage_runs(parent)
        {
            let _ = remove_isolated_coverage_artifacts(&artifacts);
            return Err(error);
        }

        let mut created = None;
        for attempt in 0..1_000_u16 {
            let run_dir = parent.join(format!(".aplan-coverage-{}-{attempt}", std::process::id()));
            match std::fs::create_dir(&run_dir) {
                Ok(()) => {
                    if let Err(error) = write_coverage_owner(&run_dir, &owner) {
                        let _ = std::fs::remove_dir(&run_dir);
                        let _ = remove_isolated_coverage_artifacts(&artifacts);
                        return Err(error);
                    }
                    let artifact = run_dir.join(LLVM_COV_TARGET);
                    // Cargo must create the target itself. When handed an
                    // existing empty directory it does not install its
                    // `CACHEDIR.TAG`, and cargo-llvm-cov's own final `cargo
                    // clean` then refuses the directory as potentially
                    // unrelated. The exclusive parent is the allocation; the
                    // child is the path reserved for Cargo inside it.
                    created = Some(artifact);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    let _ = remove_isolated_coverage_artifacts(&artifacts);
                    return Err(error).with_context(|| {
                        format!(
                            "creating an isolated coverage run beside `{}`",
                            configured.display()
                        )
                    });
                }
            }
        }
        let Some(created) = created else {
            let _ = remove_isolated_coverage_artifacts(&artifacts);
            bail!(
                "could not allocate an isolated coverage run beside `{}` after 1000 attempts",
                configured.display()
            );
        };
        artifacts.push(created);
    }
    Ok(artifacts)
}

fn remove_isolated_coverage_artifacts(artifact_dirs: &[PathBuf]) -> Result<()> {
    let mut problems = Vec::new();
    for artifact in artifact_dirs.iter().rev() {
        let metadata = match std::fs::symlink_metadata(artifact) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(run_dir) = artifact.parent() else {
                    problems.push(format!("`{}` has no run directory", artifact.display()));
                    continue;
                };
                if let Err(error) = remove_current_coverage_owner(run_dir) {
                    problems.push(error.to_string());
                    continue;
                }
                if let Err(error) = std::fs::remove_dir(run_dir) {
                    problems.push(format!(
                        "removing unused coverage run `{}` non-recursively: {error}",
                        run_dir.display()
                    ));
                }
                continue;
            }
            Err(error) => {
                problems.push(format!("inspecting `{}`: {error}", artifact.display()));
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            problems.push(format!(
                "`{}` is not the real directory this run created; refusing recursive cleanup",
                artifact.display()
            ));
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(artifact) {
            problems.push(format!("removing `{}`: {error}", artifact.display()));
            continue;
        }
        let Some(run_dir) = artifact.parent() else {
            problems.push(format!("`{}` has no run directory", artifact.display()));
            continue;
        };
        if let Err(error) = remove_current_coverage_owner(run_dir) {
            problems.push(error.to_string());
            continue;
        }
        if let Err(error) = std::fs::remove_dir(run_dir) {
            problems.push(format!(
                "removing exclusive coverage run `{}` non-recursively: {error}",
                run_dir.display()
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        bail!(
            "coverage artifacts could not be cleaned; no result was accepted:\n  {}",
            problems.join("\n  ")
        )
    }
}

fn remove_current_coverage_owner(run_dir: &Path) -> Result<()> {
    let owner = run_dir.join(COVERAGE_OWNER);
    match std::fs::symlink_metadata(&owner) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            std::fs::remove_file(&owner)
                .with_context(|| format!("removing `{}`", owner.display()))?;
            Ok(())
        }
        Ok(_) => bail!(
            "`{}` is not the real ownership file this run created; refusing cleanup",
            owner.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!("coverage owner record `{}` is missing", owner.display())
        }
        Err(error) => Err(error).with_context(|| format!("inspecting `{}`", owner.display())),
    }
}

#[cfg(target_os = "linux")]
fn coverage_process_identity(pid: u32) -> Result<CoverageProcessIdentity> {
    let boot_id = linux_boot_id()?;
    linux_process_details(pid, &boot_id)?
        .map(|(identity, _)| identity)
        .with_context(|| format!("coverage child {pid} exited before its identity was recorded"))
}

#[cfg(not(target_os = "linux"))]
#[allow(clippy::unnecessary_wraps)] // Keep one fallible ownership interface on every platform.
fn coverage_process_identity(pid: u32) -> Result<CoverageProcessIdentity> {
    Ok(CoverageProcessIdentity {
        pid,
        boot_id: "unavailable".to_string(),
        start_ticks: 0,
        uid: 0,
    })
}

#[cfg(unix)]
#[derive(Debug)]
struct CoverageSignalOwner {
    handle: signal_hook::iterator::Handle,
    thread: Option<std::thread::JoinHandle<()>>,
    child_pid: std::sync::Arc<std::sync::atomic::AtomicU32>,
    finished: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(unix)]
impl CoverageSignalOwner {
    fn install() -> std::io::Result<Self> {
        use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

        COVERAGE_INTERRUPTED_BY.store(0, std::sync::atomic::Ordering::SeqCst);
        let mut signals = signal_hook::iterator::Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM])?;
        let handle = signals.handle();
        let child_pid = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_child = std::sync::Arc::clone(&child_pid);
        let thread_finished = std::sync::Arc::clone(&finished);
        let thread = std::thread::Builder::new()
            .name("coverage-termination".to_string())
            .spawn(move || {
                let Some(signal) = signals.forever().next() else {
                    return;
                };
                COVERAGE_INTERRUPTED_BY.store(signal, std::sync::atomic::Ordering::SeqCst);
                loop {
                    let pid = thread_child.load(std::sync::atomic::Ordering::SeqCst);
                    if pid != 0 {
                        kill_coverage_process_group(pid);
                        return;
                    }
                    if thread_finished.load(std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    std::thread::yield_now();
                }
            })?;
        Ok(Self {
            handle,
            thread: Some(thread),
            child_pid,
            finished,
        })
    }

    fn publish_child(&self, pid: u32) {
        self.child_pid
            .store(pid, std::sync::atomic::Ordering::SeqCst);
    }

    fn clear_child(&self) {
        self.child_pid.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(unix)]
impl Drop for CoverageSignalOwner {
    fn drop(&mut self) {
        self.finished
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(unix)]
fn kill_coverage_process_group(process_group: u32) {
    let _ = Command::new("/bin/sh")
        .args([
            "-c",
            "kill -KILL \"-$1\" 2>/dev/null; kill -KILL \"$1\" 2>/dev/null; true",
            "aplan-coverage",
            &process_group.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn kill_coverage_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    kill_coverage_process_group(child.id());
    let _ = child.kill();
}

#[cfg(unix)]
fn resume_coverage_signal() {
    let signal = COVERAGE_INTERRUPTED_BY.swap(0, std::sync::atomic::Ordering::SeqCst);
    if signal == 0 {
        return;
    }
    // Cleanup has returned before this handoff. Preserve the caller-visible
    // reason instead of translating Ctrl-C/SIGTERM into a generic exit 1.
    let _ = signal_hook::low_level::emulate_default_handler(signal);
    signal_hook::low_level::exit(128_i32.saturating_add(signal));
}

/// Invoke cargo-llvm-cov inside this run's isolated artifact directories.
fn run_coverage(
    root: &Path,
    artifact_dirs: &[PathBuf],
    #[cfg(unix)] signal_owner: &CoverageSignalOwner,
) -> Result<std::collections::BTreeMap<String, (u64, u64)>> {
    let command = coverage_command();
    run_coverage_command_with_owner(
        root,
        artifact_dirs,
        command,
        #[cfg(unix)]
        signal_owner,
    )
}

fn coverage_command() -> Command {
    let mut command = Command::new("cargo");
    command
        .args(["llvm-cov", "--workspace", "--json", "--summary-only"])
        .env("RUST_TEST_THREADS", COVERAGE_TEST_THREADS);
    command
}

#[cfg(all(test, target_os = "linux"))]
fn run_coverage_command(
    root: &Path,
    artifact_dirs: &[PathBuf],
    command: Command,
) -> Result<std::collections::BTreeMap<String, (u64, u64)>> {
    #[cfg(unix)]
    let signal_owner = CoverageSignalOwner::install()
        .context("installing coverage termination handlers before starting the test command")?;
    let measured = run_coverage_command_with_owner(
        root,
        artifact_dirs,
        command,
        #[cfg(unix)]
        &signal_owner,
    );
    #[cfg(unix)]
    drop(signal_owner);
    measured
}

fn run_coverage_command_with_owner(
    root: &Path,
    artifact_dirs: &[PathBuf],
    mut command: Command,
    #[cfg(unix)] signal_owner: &CoverageSignalOwner,
) -> Result<std::collections::BTreeMap<String, (u64, u64)>> {
    let target = artifact_dirs
        .first()
        .context("coverage isolation produced no target directory")?;
    let build = artifact_dirs.get(1).unwrap_or(target);
    let mut owner = read_coverage_owner(
        target
            .parent()
            .context("isolated coverage target has no private parent")?,
    )?;
    for artifact in artifact_dirs.iter().skip(1) {
        let sibling = read_coverage_owner(
            artifact
                .parent()
                .context("isolated coverage build directory has no private parent")?,
        )?;
        if sibling != owner {
            bail!("isolated coverage directories disagree about their owner");
        }
    }

    command
        .env("CARGO_LLVM_COV_TARGET_DIR", target)
        .env("CARGO_LLVM_COV_BUILD_DIR", build)
        .env(COVERAGE_LEASE_ENV, &owner.lease)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command.spawn().context(
        "running `cargo llvm-cov` (install it with `cargo install cargo-llvm-cov` and \
         `rustup component add llvm-tools-preview`)",
    )?;
    #[cfg(unix)]
    signal_owner.publish_child(child.id());

    let child_identity = match coverage_process_identity(child.id()) {
        Ok(identity) => identity,
        Err(error) => {
            kill_coverage_child(&mut child);
            let _ = child.wait();
            #[cfg(unix)]
            signal_owner.clear_child();
            return Err(error).context("recording cargo-llvm-cov's process identity");
        }
    };
    owner.phase = CoverageOwnerPhase::Running;
    owner.process_group = Some(child.id());
    owner.child = Some(child_identity);
    for artifact in artifact_dirs {
        let run_dir = artifact
            .parent()
            .context("isolated coverage directory has no private parent")?;
        if let Err(error) = write_coverage_owner(run_dir, &owner) {
            kill_coverage_child(&mut child);
            let _ = child.wait();
            #[cfg(unix)]
            signal_owner.clear_child();
            return Err(error).context("publishing cargo-llvm-cov ownership");
        }
    }

    let output = child.wait_with_output();
    #[cfg(unix)]
    signal_owner.clear_child();
    let out = output.context("waiting for cargo-llvm-cov")?;
    if !out.status.success() {
        bail!(
            "cargo llvm-cov failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let files = json
        .pointer("/data/0/files")
        .and_then(serde_json::Value::as_array)
        .context("cargo llvm-cov produced no per-file data")?;

    total_per_crate(files, root)
}

/// How many lines a measured file actually has, or `None` if it cannot be read.
///
/// `None` rather than an error: a file llvm-cov names and this process cannot
/// open is not a coverage defect, and refusing on it would make the gate fail
/// for a reason that has nothing to do with coverage.
fn physical_lines(filename: &str) -> Option<u64> {
    let text = std::fs::read_to_string(filename).ok()?;
    u64::try_from(text.lines().count()).ok()
}

/// Sum llvm-cov's per-file line counts into per-crate totals.
///
/// Split from the measurement above so the summing is testable without an
/// instrumented build, which is the same division `coverage` already makes
/// between reading numbers and deciding about them.
fn total_per_crate(
    files: &[serde_json::Value],
    root: &Path,
) -> Result<std::collections::BTreeMap<String, (u64, u64)>> {
    let mut per_crate: std::collections::BTreeMap<String, (u64, u64)> =
        std::collections::BTreeMap::new();
    for f in files {
        let name = f
            .get("filename")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let Some(krate) = crate_of(name, root) else {
            continue;
        };
        let lines = f.pointer("/summary/lines");
        let count = lines
            .and_then(|l| l.get("count"))
            .and_then(serde_json::Value::as_u64);
        let covered = lines
            .and_then(|l| l.get("covered"))
            .and_then(serde_json::Value::as_u64);
        if let (Some(count), Some(covered)) = (count, covered) {
            // **A file cannot have more coverable lines than lines** (`A1-457`,
            // audit). Per-run isolation above is the invariant; this remains a
            // defense-in-depth refusal for a corrupted or externally modified
            // run. It is not sufficient by itself: two merged mappings can
            // still total fewer coverable lines than the source file has
            // physical lines.
            //
            // It is a *wrong* number rather than a missing one, which is the
            // failure this repository refuses everywhere else, and it is
            // invisible in the per-crate totals: `aplan-evidence` read 64.5%
            // against 98.1% with every test passing and the covered count
            // unchanged to four lines. What gave it away is the impossibility
            // this checks -- `src/lib.rs` reporting 6,761 coverable lines for a
            // 5,594-line file -- so that is what is checked, rather than the
            // duplicate filename llvm-cov does not produce.
            if let Some(lines) = physical_lines(name)
                && count > lines
            {
                bail!(
                    "`cargo llvm-cov` reports {count} coverable lines in `{name}`, which \
                     has {lines}. Its mappings were merged or otherwise corrupted even \
                     though this measurement used an isolated `llvm-cov-target`, so every \
                     percentage is wrong rather than missing. No result was accepted and \
                     the run-owned artifacts are removed automatically; rerun after any \
                     external coverage process has stopped"
                );
            }
            let e = per_crate.entry(krate).or_default();
            e.0 += covered;
            e.1 += count;
        }
    }
    if per_crate.is_empty() {
        bail!("cargo llvm-cov reported no files inside the workspace");
    }
    Ok(per_crate)
}

/// The crate a measured file belongs to: the directory under `crates/`, or
/// `xtask`. Paths outside the workspace (the registry, the toolchain) are `None`.
fn crate_of(filename: &str, root: &Path) -> Option<String> {
    let rel = Path::new(filename)
        .strip_prefix(root)
        .ok()
        .unwrap_or_else(|| Path::new(filename));
    let mut parts = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    match parts.next()?.as_str() {
        "crates" => parts.next(),
        "xtask" => Some("xtask".to_string()),
        _ => None,
    }
}

/// Covered/total as a percentage, truncated to one decimal.
///
/// Truncated, not rounded: a floor is a lower bound, and rounding 91.96 up to
/// 92.0 would write a floor the code does not actually meet.
fn pct((covered, count): (u64, u64)) -> serde_json::Value {
    if count == 0 {
        return serde_json::json!(100.0);
    }
    #[allow(clippy::cast_precision_loss)]
    let raw = covered as f64 / count as f64 * 100.0;
    serde_json::json!((raw * 10.0).floor() / 10.0)
}

/// The floor written for a measured percentage: the whole percent below it.
///
/// Not the measured value itself. Coverage is not identical across toolchain
/// versions, and a floor pinned to the tenth would turn CI red on a rustc bump
/// that moved a number by 0.1, which teaches people to regenerate the file
/// without reading it. A whole percent leaves under a point of slack, still far
/// tighter than the [`RATCHET_SLACK`] that catches a floor going stale.
fn floor_pct(counts: (u64, u64)) -> serde_json::Value {
    let raw = as_f64(&pct(counts));
    serde_json::json!(raw.floor())
}

fn as_f64(v: &serde_json::Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn gen_schema(check: bool) -> Result<()> {
    let root = workspace_root()?;
    for (rel, derive) in SCHEMAS {
        gen_one(&root, rel, &derive(), check)?;
    }
    Ok(())
}

fn gen_one(root: &Path, rel: &str, schema: &serde_json::Value, check: bool) -> Result<()> {
    let mut rendered = serde_json::to_string_pretty(schema)?;
    rendered.push('\n');
    write_or_check(root, rel, &rendered, check, "gen-schema")
}

/// Generate `ROADMAP.status.md`, the plan's machine-checkable half rendered by
/// `aplan roadmap --format markdown` (ADR-0009). It calls the library directly,
/// as `gen-schema` calls `derived_schema`, so the `spec` CI job need not build the
/// `aplan` binary. `--check` fails when the committed file and the plan disagree.
/// The plan directory, relative to the workspace root. Named once: the
/// generators and the note that explains them must read the same directory
/// (`A1-144`).
const PLAN_DIR: &str = "aplan";

/// Plan files that differ between the working tree and what is committed
/// (`A1-144`).
///
/// Every generated artefact here reads the *working tree*, and the gate compares
/// against what CI checks out. A task file written and not yet committed makes
/// the rollup name work CI cannot see; a task file deleted and not yet committed
/// makes it omit work CI still has. Both fail there and nowhere else -- measured
/// twice in one session for the first shape, and the second is its mirror, found
/// by auditing the fix.
///
/// **Modified files are deliberately not listed.** A task edited and not yet
/// committed is the *normal* state at the moment a rollup is regenerated: close
/// a task, regenerate, commit both. A note on that would fire on nearly every
/// run and teach its reader to skip it, which is worse than the hazard it
/// describes.
///
/// Silent where git cannot answer: this is a note about a difference, and a
/// generator that failed because `git` was missing, or because the source was
/// unpacked from an archive, would be a worse hazard than the one being
/// reported.
fn plan_files_out_of_step(root: &Path) -> Vec<(&'static str, String)> {
    let ask = |args: &[&str]| -> Vec<String> {
        let Ok(out) = Command::new("git").args(args).current_dir(root).output() else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| {
                std::path::Path::new(l)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            })
            .map(ToString::to_string)
            .collect()
    };
    let mut out: Vec<(&'static str, String)> =
        ask(&["ls-files", "--others", "--exclude-standard", PLAN_DIR])
            .into_iter()
            .map(|f| ("does not track", f))
            .collect();
    out.extend(
        ask(&["ls-files", "--deleted", PLAN_DIR])
            .into_iter()
            .map(|f| ("still has, deleted here", f)),
    );
    out
}

/// The note an author needs at the moment they could act on it, or `None` when
/// the working tree and a clean checkout agree.
///
/// A note, never a refusal: writing a task and regenerating before committing is
/// the normal order, and a gate that forbade it would be wrong about the
/// workflow it exists to serve (`A1-144`).
fn untracked_plan_note(root: &Path, what: &str) -> Option<String> {
    let out_of_step = plan_files_out_of_step(root);
    if out_of_step.is_empty() {
        return None;
    }
    let listed: Vec<String> = out_of_step
        .iter()
        .map(|(kind, file)| format!("{file} (git {kind})"))
        .collect();
    Some(format!(
        "note: the plan directory holds {} file(s) that differ from what is committed ({}), so \
         {what} reads a plan a clean checkout does not have. Commit them, or expect this to \
         disagree in CI.",
        listed.len(),
        listed.join(", ")
    ))
}

/// Print it, where a generator or a gate is about to read the plan.
fn note_untracked_plan_files(root: &Path, what: &str) {
    if let Some(note) = untracked_plan_note(root, what) {
        eprintln!("{note}");
    }
}

fn gen_roadmap(check: bool) -> Result<()> {
    let root = workspace_root()?;
    note_untracked_plan_files(&root, "this rollup");
    let plan = aplan_core::load_plan(&root.join(PLAN_DIR));
    let graph = aplan_core::Graph::new(plan.tasks.iter().map(|lt| &lt.task));
    // Named truthfully: this repository's generator is the xtask, not the bare
    // command, and the banner is what a reader runs when the drift gate fires
    // (`A1-059`).
    // The declared throughput, so the committed rollup carries the same derived
    // durations `aplan roadmap` prints (`A1-147`). A generator that skipped this
    // would make the committed artifact the one surface that cannot answer "how
    // long is this cut", which is the divergence `A1-146` found in the renderers
    // and this file would have reintroduced.
    // Propagated, never defaulted: a `.aplan.yml` this generator cannot read
    // would otherwise write a rollup with the durations silently missing, and
    // the drift gate would compare that artifact against itself and pass. A
    // generator that quietly produces a different document is the failure this
    // repository gates everything else against.
    let per_week = aplan_config::load_config(&root)?.effort_days_per_week;
    let wb = aplan::written_by();
    let rendered = aplan_output::render_roadmap_markdown(
        &graph.roadmap().with_throughput(per_week),
        Some("cargo xtask gen-roadmap"),
        Some(&aplan_output::WrittenBy {
            version: &wb.0,
            surface: &wb.1,
        }),
    );
    write_or_check(&root, "ROADMAP.status.md", &rendered, check, "gen-roadmap")
}

/// Write a generated file, or (with `check`) fail if the committed copy differs.
/// The one shape every generated-and-gated artifact shares (ADR-0009): a whole-file
/// compare, regenerated with a documented command.
/// Which build is doing the checking, as a sentence a gate can append
/// (`A1-325`, ADR-0037).
///
/// Every gate that compares a committed artifact against a freshly computed one
/// is green by construction wherever both sides come from the same build, which
/// is always on the machine where it is run. The number it reports is only
/// comparable to the committed one if the same `aplan` produced both, so the
/// gate that reports a difference says which `aplan` it is. Shared so the three
/// gates cannot drift into three phrasings of one fact.
fn checked_by() -> String {
    let (version, surface) = aplan::written_by();
    format!(
        "This is aplan {version}, surface {surface}. If the committed copy was written by a \
         different build, that is the likely cause rather than an edit: check what wrote it \
         before rewriting it."
    )
}

fn write_or_check(root: &Path, rel: &str, rendered: &str, check: bool, regen: &str) -> Result<()> {
    let path = root.join(rel);
    if check {
        let on_disk = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if on_disk != rendered {
            // Which build is complaining. A generated artifact regenerated by a
            // *different* build produces a diff of everything whose shape
            // changed, with nothing saying why, and the comparison is green by
            // construction wherever both sides are the same build -- which is
            // always, on the machine where it is run. Naming the build here
            // costs one line and turns "thirty lines changed" into a question
            // somebody can answer (`A1-325`, pre-public adoption evidence item 13).
            bail!(
                "{rel} is stale: the committed copy and its generated form disagree.\n\
                 Run `cargo xtask {regen}` and commit the result.\n{}",
                checked_by()
            );
        }
        println!("{rel} is up to date");
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {rel}");
    Ok(())
}

/// Typographic characters forbidden in tracked text, each with the plain-ASCII
/// intent. Written as `\u{...}` escapes so this source stays lint-clean.
///
/// Three families, all of which read as machine-generated or are outright
/// invisible: the visible tells (fancy dashes, curly quotes, the ellipsis), the
/// non-ASCII dash variants that a model or a copy-paste substitutes for a plain
/// `-`, and the zero-width / exotic-space characters that are invisible in a diff
/// and double as a trojan-source vector. Functional Unicode (the arrow in cycle
/// output, the section sign, accented letters in the slug tests) is deliberately
/// left alone.
const FORBIDDEN: &[(char, &str)] = &[
    // Visible tells.
    (
        '\u{2014}',
        "em dash (use ',' ':' ';' parentheses, or split the sentence)",
    ),
    ('\u{2013}', "en dash (use '-' or 'to')"),
    ('\u{2018}', "left single quote (use ')"),
    ('\u{2019}', "right single quote / curly apostrophe (use ')"),
    ('\u{201C}', "left double quote (use straight \")"),
    ('\u{201D}', "right double quote (use straight \")"),
    ('\u{2026}', "ellipsis character (use three dots)"),
    // Dash variants that stand in for a plain ASCII '-'.
    (
        '\u{2015}',
        "horizontal bar (use '-', or split the sentence)",
    ),
    ('\u{2012}', "figure dash (use '-')"),
    ('\u{2212}', "minus sign (use '-')"),
    ('\u{2010}', "Unicode hyphen (use ASCII '-')"),
    ('\u{2011}', "non-breaking hyphen (use ASCII '-')"),
    // Invisible: zero-width characters and exotic spaces.
    ('\u{00A0}', "non-breaking space (use a normal space)"),
    ('\u{202F}', "narrow no-break space (use a normal space)"),
    ('\u{2009}', "thin space (use a normal space)"),
    ('\u{200A}', "hair space (use a normal space)"),
    ('\u{200B}', "zero-width space (delete it)"),
    ('\u{200C}', "zero-width non-joiner (delete it)"),
    ('\u{200D}', "zero-width joiner (delete it)"),
    ('\u{2060}', "word joiner (delete it)"),
    ('\u{FEFF}', "zero-width no-break space / BOM (delete it)"),
];

fn describe(c: char) -> Option<&'static str> {
    FORBIDDEN.iter().find(|(f, _)| *f == c).map(|(_, d)| *d)
}

/// Every forbidden character in `text`, as `(line, column, char)`, both
/// 1-indexed. Split out from the file walk so it is directly testable.
fn scan_text(text: &str) -> Vec<(usize, usize, char)> {
    let mut hits = Vec::new();
    for (li, line) in text.lines().enumerate() {
        for (ci, c) in line.chars().enumerate() {
            if describe(c).is_some() {
                hits.push((li + 1, ci + 1, c));
            }
        }
    }
    hits
}

/// The style gate: no typographic tells in any tracked text file.
/// Where the house style records what it does not apply to.
///
/// Optional: absent means the style applies everywhere, which is what this
/// repository did before there was a file (`A1-052`).
const STYLE_CONFIG: &str = ".house-style.yml";

/// Path prefixes the house style does not apply to.
///
/// Prefixes, not globs, and that is a stated limit rather than an oversight: a
/// glob needs a matcher, and every case this exists for is a directory. An entry
/// is matched against the repository-relative path, so `private pre-public adoption records` exempts
/// everything beneath it and `docs/pilots` exempts the same paths plus a file of
/// that name.
fn style_exemptions(root: &std::path::Path) -> Result<Vec<String>> {
    let path = root.join(STYLE_CONFIG);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let value: yaml_compat::Value =
        yaml_compat::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let Some(list) = value.get("exempt").and_then(|e| e.as_sequence()) else {
        return Ok(Vec::new());
    };
    list.iter()
        .map(|e| {
            let entry = e.as_str().ok_or_else(|| {
                anyhow::anyhow!("`exempt` in {STYLE_CONFIG} must be a list of path prefixes")
            })?;
            // A blank entry is not "everything", it is a typo. Making it mean
            // everything would turn a stray line into a silently disabled gate,
            // which is the direction this project refuses to fail in. Declining
            // is spelled `**` or `.`, deliberately and visibly.
            if entry.trim().is_empty() {
                anyhow::bail!(
                    "`exempt` in {STYLE_CONFIG} has a blank entry; write `**` to decline the \
                     style entirely, or remove the line"
                );
            }
            Ok(entry.to_string())
        })
        .collect()
}

fn is_exempt(rel: &str, exemptions: &[String]) -> bool {
    exemptions.iter().any(|e| {
        let e = e.trim_end_matches("/**").trim_end_matches('/');
        // `**` and `.` mean the whole repository, which is how a repository
        // *declines* the style rather than narrowing it. Without them there was
        // no spelling that did: `A1-052` asked for adopt, narrow or decline, and
        // only two of the three worked (audit).
        if e == "**" || e == "." {
            return true;
        }
        rel == e || rel.starts_with(&format!("{e}/"))
    })
}

fn lint() -> Result<()> {
    let root = workspace_root()?;
    let files = tracked_files(&root)?;
    // The typographic policy is this repository's, and a pilot log is a
    // consumer's writing about their repository, kept here by our convention.
    // Forcing an adopter into our dialect made the rule leak out of its scope
    // (pre-public adoption evidence, item 5).
    let exemptions = style_exemptions(&root)?;
    let scan = lint_files(&root, &files, &exemptions);

    for line in &scan.reports {
        println!("{line}");
    }
    if scan.violations == 0 {
        // The count says what was *not* looked at, because a gate that silently
        // skips files is one nobody can audit. And a run that looked at nothing
        // says so rather than saying "clean", which is the vacuity this project
        // keeps finding in other people's gates.
        println!("{}", clean_summary(&scan));
        Ok(())
    } else {
        bail!(
            "{} style violation(s) across {} file(s). \
             Replace each with plain ASCII (hints above).",
            scan.violations,
            scan.files_hit
        )
    }
}

/// The line a clean run prints.
///
/// Its own function because it is consumer-facing text and this project holds
/// what a tool says to the same standard as what it does. The "not applied" case
/// survived its mutation while it lived inside `lint` (`A1-052`, audit).
fn clean_summary(scan: &LintScan) -> String {
    if scan.scanned == 0 && scan.exempted > 0 {
        format!(
            "style: not applied ({} files exempt via {STYLE_CONFIG}, none scanned)",
            scan.exempted
        )
    } else if scan.exempted == 0 {
        format!("style: clean ({} files scanned)", scan.scanned)
    } else {
        format!(
            "style: clean ({} files scanned, {} exempt via {STYLE_CONFIG})",
            scan.scanned, scan.exempted
        )
    }
}

/// What a lint pass found, separated from how it is printed so the pass itself
/// can be tested.
///
/// Extracted because disabling the exemption in `lint` survived its mutation:
/// `is_exempt` was tested and the gate that calls it was not, which is the shape
/// `A1-107` names (`A1-052`).
#[derive(Default)]
struct LintScan {
    violations: usize,
    files_hit: usize,
    scanned: usize,
    exempted: usize,
    reports: Vec<String>,
}

fn lint_files(root: &std::path::Path, files: &[String], exemptions: &[String]) -> LintScan {
    let mut scan = LintScan::default();
    for rel in files {
        if is_exempt(rel, exemptions) {
            scan.exempted += 1;
            continue;
        }
        // Non-UTF-8 (binary) or unreadable files are not text; skip them.
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        scan.scanned += 1;
        let hits = scan_text(&text);
        if hits.is_empty() {
            continue;
        }
        scan.files_hit += 1;
        for (line, col, c) in hits {
            scan.violations += 1;
            let desc = describe(c).unwrap_or("forbidden character");
            scan.reports
                .push(format!("{rel}:{line}:{col}: U+{:04X} {desc}", c as u32));
        }
    }
    scan
}

/// The repo's tracked files, via `git ls-files`. The lint is dev tooling, not
/// the hermetic `aplan check`, so shelling out to git here is fine.
fn tracked_files(root: &Path) -> Result<Vec<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .context("running `git ls-files` (the lint scans tracked files)")?;
    if !out.status.success() {
        bail!(
            "`git ls-files` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// What the reader is told a reference looks like, when this gate fails.
///
/// No longer a pattern this file owns: the ids are the plan's, and a word counts
/// as a reference when it carries a prefix one of them uses (`A1-148`). Saying
/// "A#-###" here would describe an older, narrower scanner and send a reader
/// looking for the wrong mistake.
const ID_PATTERN_HELP: &str = "a word carrying a prefix the plan's own ids use";

/// Every task/milestone id referenced in `text`.
///
/// Factored out because this half generalises and the other does not. Resolving
/// ids named in a document is format-agnostic and mirrors `APLAN008` (which
/// asserts the opposite direction: a task's `traces` anchor resolves to a real
/// heading). If the p0000019 pilot (`A1-036`) shows the same failure shape in a
/// JSON narrative, this is the piece that lifts into `aplan-core` as a rule.
/// Deciding a cut's *membership declaration* cannot lift: it needs to know what a
/// cut section is, and every project shapes that differently.
/// The documents this repository declared once in `.aplan.yml`.
///
/// This used to encode `ROADMAP.md`, `CHANGELOG.md`,
/// `docs/upgrading.md` and pre-public adoption evidence here. That was the exact
/// second list `A1-203` exists to remove: adding a document could leave the
/// gate green unless somebody remembered a Rust edit. The shipped command and
/// this repository-specific companion now expand the same declaration through
/// `aplan-config`.
fn declared_doc_sources(root: &Path) -> Result<Vec<(String, String)>> {
    let (config, config_dir) =
        aplan_config::load_config_located(root).context("loading document declarations")?;
    let docs = config
        .docs
        .as_ref()
        .context("no `docs.paths` or `docs.membership` declaration in .aplan.yml")?;
    let base = config_dir.as_deref().unwrap_or(root);
    let paths = docs
        .document_paths(base)
        .context("resolving declared documents")?;
    if paths.is_empty() {
        bail!("the `docs` declaration names no documents");
    }

    paths
        .into_iter()
        .map(|path| {
            let name = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .display()
                .to_string();
            let body = std::fs::read_to_string(&path)
                .with_context(|| format!("reading declared document {}", path.display()))?;
            Ok((name, body))
        })
        .collect()
}

/// Why this gate cannot run, if it cannot (`A1-148`, its audit).
///
/// Split out to be testable without a broken plan on disk, the way
/// `root_from_config` is. Two ways to be unusable, and both were live: a plan
/// with a file that failed to load is a vocabulary with holes, and comparing
/// prose against it produces *accusations* -- with one task file broken, this
/// gate reported that `ROADMAP.md` names a task that does not exist, naming the
/// very task whose file would not parse. A plan that teaches no id prefix (an
/// empty or vanished plan directory, or ULID ids) leaves nothing to recognise,
/// so every document passes and the gate is decorative.
fn plan_unusable(unloadable: usize, known: &BTreeSet<String>) -> Option<String> {
    if unloadable > 0 {
        return Some(format!(
            "{unloadable} task file(s) failed to load, so the plan cannot say which ids exist; \
             run `aplan check` and fix those first"
        ));
    }
    if aplan_interop::docs::Vocabulary::of(known).is_empty() {
        return Some(format!(
            "the plan teaches no id prefix ({} task(s) loaded), so nothing in a document can be \
             told from an ordinary word and this gate would pass on anything",
            known.len()
        ));
    }
    None
}

/// Every task id a document names, from the plan's own ids (`A1-148`).
///
/// This used to be forty lines of hand-rolled scanning that knew `A1-nnn` and
/// `M-vN.N` by heart, which is exactly the check `aplan check-docs` now ships to
/// adopters. The gate this repository runs on itself is the one it hands out,
/// or the two drift and the shipped one is the untested copy.
/// Every reference in `text` that names no task, by the shipped command's own
/// reckoning.
///
/// The tempting spelling is "the ids this document mentions, minus the plan's",
/// and it is how this gate came to disagree with the command it hands out:
/// `A1-164` taught `dangling` to report a reference whose *prefix* is a digit's
/// typo, a subtraction cannot produce one, and `aplan check-docs` reported a pilot
/// log that `cargo xtask check-docs` called clean. The comment above this function
/// says why that must not happen; this is the call that keeps it true.
fn undefined_ids(text: &str, ids: &BTreeSet<String>) -> Vec<String> {
    aplan_interop::docs::dangling(text, ids)
        .into_iter()
        .map(|r| r.id)
        .collect()
}

/// The heading a line sits under, so a finding is named and not only numbered.
fn finding_at(text: &str, line: usize) -> String {
    aplan_core::anchor::sections(text)
        .into_iter()
        .filter(|s| s.line + 1 < line)
        .max_by_key(|s| s.line)
        .map_or(String::new(), |s| format!(" ({})", s.title.trim()))
}

/// A disposition that names a placeholder rather than a task (`A1-111`).
///
/// `check-docs` exists partly for this: an item claiming to be tracked under an
/// id that does not exist **reads as handled**, which is worse than an item with
/// no disposition at all. Two findings in pre-public adoption evidence ended with
/// the line ``Tracked as `A1-nnn`.`` at that log's highest severity, and sat
/// from 2026-08-11 until a review went looking, because `A1-nnn` is not an id
/// and so was not a reference anything could resolve.
///
/// **Banning the placeholder would be wrong**, and that is the whole difficulty.
/// pre-public adoption evidence teaches `A1-nnn` as the way to write "a task id"
/// rather than a specific task, the way the docs write `M-v0.N` for "a
/// milestone", and it uses it twice to say so. A gate that failed the file
/// teaching the convention would have misread the problem.
///
/// So this reads the *sentence*, not the token. What is caught is a claim of
/// tracking -- `tracked`/`filed` followed by `as`, `in`, `by` or `under` --
/// whose target wears a prefix the plan taught and is not a task. A placeholder
/// in prose about how to write ids is untouched, because no claim precedes it.
///
/// Two boundaries, both deliberate. A claim inside a `code span` is quoting one
/// rather than making one, which is how a document may show the shape it is
/// warning about. And a target whose tail is all digits is left to
/// [`undefined_ids`], which already reports it and says what it might have
/// meant: reporting it twice, in two voices, would be worse than either.
///
/// What it does not catch, measured: a disposition written in any other words.
/// The corpus uses `tracked as` 61 times and `filed as` 6, and the one-off
/// phrasings beside them (`settled by`, `answered in`, `fixed under`) are as
/// often ordinary prose as dispositions, so widening the verb list buys a false
/// positive per phrase for a case that has not happened.
fn placeholder_dispositions(text: &str, ids: &BTreeSet<String>) -> Vec<(usize, String, String)> {
    const VERBS: [&str; 2] = ["tracked", "filed"];
    const PREPOSITIONS: [&str; 4] = ["as", "in", "by", "under"];
    let vocabulary = aplan_interop::docs::Vocabulary::of(ids);
    let prefixes = vocabulary.prefixes();
    if prefixes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (first_line, paragraph) in paragraphs(text) {
        let chars: Vec<char> = paragraph.chars().collect();
        let quoted = code_span_mask(&chars);
        for at in 0..chars.len() {
            if quoted[at] || (at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '_')) {
                continue;
            }
            let Some(after) = VERBS.iter().find_map(|v| word_at(&chars, at, v)) else {
                continue;
            };
            let after = skip(&chars, after, char::is_whitespace);
            let Some(after) = PREPOSITIONS.iter().find_map(|p| word_at(&chars, after, p)) else {
                continue;
            };
            // Whatever decorates the target: a space, a backtick, the emphasis a
            // log puts on the disposition it wants read first.
            let start = skip(&chars, after, |c| {
                c.is_whitespace() || "`\"'*([".contains(c)
            });
            let end = skip(&chars, start, |c| c.is_alphanumeric() || "._-".contains(c));
            let target: String = chars[start..end]
                .iter()
                .collect::<String>()
                .trim_end_matches(['.', ',', ';', ':'])
                .to_string();
            let Some(prefix) = prefixes.iter().find(|p| target.starts_with(**p)) else {
                continue;
            };
            if ids.contains(&target) {
                continue;
            }
            // A number is a reference that resolved to nothing, and is reported
            // as one; this is for the ids that were never written as ids.
            let tail = &target[prefix.len()..];
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            // The *target's* line, not the claim's. They differ only where the
            // sentence wrapped, and there the id is what the reader has come to
            // change.
            let line = first_line + chars[..start].iter().filter(|c| **c == '\n').count() + 1;
            // The claim as written, so a log saying `filed under` is not told it
            // said `tracked as`. A wrapped one is read back as one line.
            let claim: String = chars[at..after]
                .iter()
                .collect::<String>()
                .replace('\n', " ");
            out.push((line, claim, target));
        }
    }
    out
}

/// The document's paragraphs: runs of unfenced, non-blank lines, with the line
/// each starts on.
///
/// A paragraph rather than a line, because this repository wraps its prose at
/// about a hundred columns and a disposition wraps with it.
/// pre-public adoption evidence ends a line with `Tracked as` and opens the next with
/// the id, which a line-at-a-time scan cannot see -- and neither could it see a
/// quotation that opens before the wrap and closes after it, which is exactly
/// what pre-public adoption evidence does in the paragraph explaining this rule.
/// Both are the same fact: a sentence is not a line.
///
/// A paragraph is also the right span for the pairing. Markdown ends a code span
/// and a quotation at a blank line, so pairing across the whole document would
/// let one stray backtick twenty paragraphs earlier decide whether a claim is a
/// claim.
fn paragraphs(text: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut current: Option<(usize, Vec<&str>)> = None;
    let mut previous: Option<usize> = None;
    for (index, line) in aplan_core::anchor::unfenced_lines(text) {
        // A blank line ends a paragraph, and so does a fence: `unfenced_lines`
        // skips what is inside one, so a gap in the indices is a gap in prose.
        if (line.trim().is_empty() || previous.is_some_and(|p| index != p + 1))
            && let Some((start, lines)) = current.take()
        {
            out.push((start, lines.join("\n")));
        }
        if !line.trim().is_empty() {
            match &mut current {
                Some((_, lines)) => lines.push(line),
                None => current = Some((index, vec![line])),
            }
        }
        previous = Some(index);
    }
    if let Some((start, lines)) = current {
        out.push((start, lines.join("\n")));
    }
    out
}

/// Where a paragraph quotes rather than speaks: `code spans` and "quotations".
///
/// Both, because a document that warns about a disposition has to be able to
/// show one. pre-public adoption evidence and pre-public adoption evidence each write
/// `an item that says "Tracked as ``A1-nnn``"` in the course of explaining why
/// that is bad, and a gate that failed the two files teaching the convention
/// would have misread the problem exactly as thoroughly as no gate at all.
///
/// Quotations are straight `"` only, which is not a shortcut: the house style
/// forbids the curly ones, and `xtask lint` is what keeps that true.
fn code_span_mask(chars: &[char]) -> Vec<bool> {
    let mut mask = vec![false; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            // A quotation runs to its partner on the same line; an unpaired one
            // quotes nothing, exactly as an unclosed backtick opens nothing.
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '"') {
                let end = i + 1 + end;
                for m in mask.iter_mut().take(end + 1).skip(i) {
                    *m = true;
                }
                i = end + 1;
                continue;
            }
            i += 1;
            continue;
        }
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let open = skip(chars, i, |c| c == '`') - i;
        let mut at = i + open;
        let close = loop {
            if at >= chars.len() {
                break None;
            }
            if chars[at] == '`' {
                let run = skip(chars, at, |c| c == '`') - at;
                if run == open {
                    break Some(at + run);
                }
                at += run;
            } else {
                at += 1;
            }
        };
        match close {
            Some(end) => {
                for m in mask.iter_mut().take(end).skip(i) {
                    *m = true;
                }
                i = end;
            }
            // An unclosed backtick opens no span, so the rest of the line is
            // prose and a claim in it is still a claim.
            None => i += open,
        }
    }
    mask
}

/// The index after `word` at `at`, when it stands there as a whole word.
fn word_at(chars: &[char], at: usize, word: &str) -> Option<usize> {
    let expected: Vec<char> = word.chars().collect();
    let end = at + expected.len();
    if end > chars.len() {
        return None;
    }
    if !chars[at..end]
        .iter()
        .zip(&expected)
        .all(|(c, e)| c.to_ascii_lowercase() == *e)
    {
        return None;
    }
    if end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        return None;
    }
    Some(end)
}

/// The first index at or after `from` whose character `keep` rejects.
fn skip(chars: &[char], from: usize, keep: impl Fn(char) -> bool) -> usize {
    let mut i = from;
    while i < chars.len() && keep(chars[i]) {
        i += 1;
    }
    i
}

/// The narrative must agree with the plan it describes.
///
/// Three checks, and the second is the one that would have caught the v0.6
/// failure. A bare "every id a section mentions is a member" rule is wrong:
/// prose legitimately makes forward references and corrections (v0.6's own
/// correction table names tasks in later cuts). So a cut *declares* its members
/// on a `**Tasks:**` line, and that declaration is what is compared; mentions
/// elsewhere stay free, and only have to resolve.
/// Does the suite invoke the surface the tool ships? (`A1-208`)
///
/// The residue is ratcheted the way `coverage.json` is: it may shrink and may
/// not grow. A number that can only go one way is what makes an audit of
/// something this large usable -- the alternative is a gate that fails on the
/// first commit and is then turned off.
///
/// The log is a by-product of the suite run `ci/scripts/test.sh` already does,
/// rather than a second run of it: this repository pays for its own CI
/// (the pre-public repository-boundary decision), and a gate that doubles the test time to measure the test time is
/// one nobody keeps.
fn cli_surface_gate(check: bool) -> Result<()> {
    let root = workspace_root()?;
    let log = std::env::var_os(INVOCATION_LOG).map(std::path::PathBuf::from);
    if log.is_none() {
        // Half the evidence is half an answer. Judging on the corpus alone would
        // report most of the surface unexercised and blame the suite for it,
        // which is the accusation this gate must never make (`A1-208`, audit).
        if check {
            bail!(
                "{INVOCATION_LOG} is not set, so only the `trycmd` corpus can be read and \
                 most of the surface would look unexercised. Run `ci/scripts/test.sh`, which \
                 records the other half and then runs this."
            );
        }
        eprintln!(
            "note: {INVOCATION_LOG} is not set, so only the `trycmd` corpus is read and the \
             count below is not the whole suite."
        );
    }
    let audit = cli_surface::audit(&root, log.as_deref())?;

    // A named log that recorded nothing is a run that did not happen, and the
    // residue from one is an accusation rather than a measurement. The size of a
    // *partial* log is not something this can judge -- a threshold picked here
    // would be the hand-kept number the whole gate avoids -- so the fix for that
    // is upstream: `ci/scripts/test.sh` gives every run its own file and deletes
    // it afterwards, because a shared path let one run truncate another's and
    // this gate then reported two thirds of the surface unexercised (`A1-208`,
    // audit).
    if log.is_some() && audit.recorded == 0 {
        bail!(
            "a recording file was named and holds no invocations, so the suite did not run; \
             re-run `ci/scripts/test.sh`."
        );
    }

    for line in cli_surface_report(&audit) {
        println!("{line}");
    }

    let path = root.join(CLI_SURFACE);
    let recorded: Ratchet = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();

    let (now, problems) = cli_surface_holds(&audit, &recorded.held);
    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("  {problem}");
        }
        bail!(
            "{} problem(s) with the holds in {CLI_SURFACE}",
            problems.len()
        );
    }
    for hold in &recorded.held {
        println!("  held: {} -- {}", hold.member, hold.why);
    }

    if check {
        println!("{}", cli_surface_verdict(now, recorded.unexercised)?);
        return Ok(());
    }
    let rendered = serde_json::to_string_pretty(&Ratchet {
        unexercised: now,
        held: recorded.held,
    })? + "\n";
    write_or_check(&root, CLI_SURFACE, &rendered, false, "cli-surface")
}

/// The residue after the holds are applied, and everything wrong with the holds
/// themselves.
///
/// **A hold has to do something or it is decoration.** The first version carried
/// `held` from one run to the next and consulted it nowhere: holding a member
/// left it in the count, so the only way to accept a gap was to raise the floor,
/// and the reason nobody read was never checked against anything. That is the
/// "appeared only in the table and drove nothing" defect `AGENTS.md` names, in
/// the gate written to find it (`A1-208`, audit).
///
/// So a hold is checked three ways, and each is a way it can rot: it must name a
/// member the surface still has, it must carry a reason worth reading, and it
/// must still be unexercised -- a held member somebody has since covered is a
/// note telling the next reader not to bother, and it is retired the way a
/// written ADR reservation is.
fn cli_surface_holds(audit: &cli_surface::Audit, held: &[Held]) -> (usize, Vec<String>) {
    let known: std::collections::BTreeMap<String, &cli_surface::Member> = audit
        .surface
        .iter()
        .map(|member| (member.to_string(), member))
        .collect();
    let mut problems = Vec::new();
    let mut excused = std::collections::BTreeSet::new();
    for hold in held {
        let Some(member) = known.get(&hold.member) else {
            problems.push(format!(
                "`{}` is held and is not in the surface any more; drop the hold",
                hold.member
            ));
            continue;
        };
        if hold.why.trim().len() < 30 {
            problems.push(format!(
                "`{}` is held with no reason worth reading: {:?}",
                hold.member, hold.why
            ));
        }
        if audit.exercised.contains(*member) {
            problems.push(format!(
                "`{}` is held as unexercised and something exercises it; drop the hold",
                hold.member
            ));
            continue;
        }
        excused.insert(hold.member.clone());
    }
    let now = audit
        .residue
        .iter()
        .filter(|member| !excused.contains(&member.to_string()))
        .count();
    (now, problems)
}

/// What the run says about itself, before any verdict.
///
/// Always the numbers, passing or failing, for the reason `coverage` prints its
/// table either way: a measurement that silently covered a fraction of the
/// surface reads exactly like a good one in a CI log.
fn cli_surface_report(audit: &cli_surface::Audit) -> Vec<String> {
    let mut lines = vec![format!(
        "cli-surface: {} member(s), {} exercised, {} not, from {} invocation(s) \
         ({} asking for help or a version, {} clap rejects)",
        audit.surface.len(),
        audit.exercised.len(),
        audit.residue.len(),
        audit.invocations,
        audit.asked_for_text,
        audit.rejected.len(),
    )];
    for (command, members) in cli_surface::by_command(&audit.residue) {
        lines.push(format!("  {command}: {}", members.join(" ")));
    }
    // Named, not counted. A rejected invocation is normally a deliberate
    // invalid-input case, but a *valid* one that stopped parsing -- a renamed
    // flag, a changed argument -- is coverage lost, and a number going from six
    // to seven says nothing about which (`A1-208`, audit).
    for argv in &audit.rejected {
        lines.push(format!("  rejected: aplan {argv}"));
    }
    lines
}

/// What the ratchet says about a count, separated from the run that produced it
/// so the three answers can be tested without a suite behind them.
///
/// **Shrinking is a failure too**, and deliberately: a residue that fell and was
/// not committed means the next person to add an unexercised flag is measured
/// against a floor that is no longer true, and the gate stays green while the
/// surface rots. The fix is one command, and the message is the command.
fn cli_surface_verdict(now: usize, floor: usize) -> Result<String> {
    if now > floor {
        bail!(
            "the unexercised surface grew from {floor} to {now}. Exercise what you added, or \
             hold it in {CLI_SURFACE} with a reason.\n{}",
            checked_by()
        );
    }
    if now < floor {
        bail!(
            "the unexercised surface shrank from {floor} to {now}, which is good news the \
             ratchet has not been told about. Run `cargo xtask cli-surface` and commit \
             {CLI_SURFACE}.\n{}",
            checked_by()
        );
    }
    Ok(format!("cli-surface: at its floor of {now}"))
}

/// The environment variable `common::run` records to, named here because this
/// gate reads what that helper writes (`A1-207`).
const INVOCATION_LOG: &str = "APLAN_TEST_INVOCATIONS";

/// The ratchet file, beside `coverage.json` and read the same way.
const CLI_SURFACE: &str = "cli-surface.json";

/// What may not grow, and what is deliberately not counted.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Ratchet {
    /// Members of the surface no invocation reached.
    unexercised: usize,
    /// Members held with a reason: a member with no answer states who owns it,
    /// rather than an assertion that the gap is fine (`AGENTS.md`).
    #[serde(default)]
    held: Vec<Held>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Held {
    member: String,
    why: String,
}

fn check_docs() -> Result<()> {
    let root = workspace_root()?;
    // The same hazard, the same note: this gate compares the narrative against
    // the plan *on disk*, so an uncommitted task makes them disagree here and
    // agree in CI, or the reverse (`A1-144`).
    note_untracked_plan_files(&root, "this comparison");
    let text = std::fs::read_to_string(root.join("ROADMAP.md")).context("reading ROADMAP.md")?;
    let plan = aplan_core::load_plan(&root.join(PLAN_DIR));
    // A plan that did not load is a vocabulary with holes in it, and comparing
    // prose against one produces *accusations*: with a single task file broken,
    // this gate reported that `ROADMAP.md` names a task that does not exist,
    // naming the very task whose file failed to parse. Worse, a plan directory
    // that vanished leaves no ids at all, so nothing can be recognised and the
    // gate passes green. Both are refusals now (`A1-148`, this task's audit).
    let known: BTreeSet<String> = plan.tasks.iter().map(|lt| lt.task.id.clone()).collect();
    if let Some(refusal) = plan_unusable(plan.unloadable.len(), &known) {
        for (id, path) in &plan.unloadable {
            eprintln!("  {id}: {}", path.display());
        }
        bail!(refusal);
    }

    let mut problems: Vec<String> = Vec::new();

    // 1. Every id named in the narrative or a pilot log resolves to a real task.
    //    A log's dispositions are its whole point (pre-public adoption evidence): an item
    //    reading "tracked as A1-nnn" with no such task looks handled and is not.
    let sources = declared_doc_sources(&root)?;
    for (name, body) in &sources {
        for id in undefined_ids(body, &known) {
            problems.push(format!(
                "{name} names `{id}`, which is not a task in the plan"
            ));
        }
        for (line, claim, target) in placeholder_dispositions(body, &known) {
            problems.push(format!(
                "{name}:{line}{} says \"{claim} `{target}`\", which is not a task in the \
                 plan; a disposition naming a placeholder reads as handled",
                finding_at(body, line)
            ));
        }
    }

    // 2. Every rule is announced. A rule code is a consumer contract that is
    //    never renumbered (`AGENTS.md`), so one reaching a release unmentioned is
    //    a contract change delivered silently. `APLAN024` and `APLAN025` both
    //    shipped that way: closed with evidence, pushed green, and absent from the
    //    changelog, because nothing here read that file (`A1-067`).
    //
    //    The whole file, not the `Unreleased` section. They are the same thing
    //    today, and the distinction the task worried about resolves itself: a rule
    //    added and removed before a release leaves `Rule::ALL`, so nothing asks
    //    for it.
    let changelog =
        std::fs::read_to_string(root.join("CHANGELOG.md")).context("reading CHANGELOG.md")?;
    problems.extend(unannounced_rules(&changelog));
    problems.extend(unannounced_config_keys(&changelog));

    // 3 and 4. Every cut section declares its membership, and it matches.
    //
    // **The matching half is the shipped command's**, run from this repository's
    // own `docs.membership` by `ci/scripts/preflight.sh` (`A1-165`). It is not
    // repeated here, for the reason check 1 is not: a gate this repository runs on
    // itself and does not hand out is the shipped one going untested, which is
    // exactly how `A1-164` reached adopters with a near-miss pass nothing here
    // could see.
    //
    // What stays is the half that is about *this* document's convention rather
    // than about the plan: a `### vN.N` heading is a cut whether or not a
    // milestone exists for it, so an unplanned cut has to say `**Tasks:** none
    // yet` rather than pass by being silent. The shipped check cannot ask that --
    // it iterates the milestones, so a cut with no milestone is a section it never
    // looks at, which is the right default for a document it did not author.
    let labels = cut_labels_of(
        plan.tasks
            .iter()
            .map(|lt| (lt.task.id.as_str(), lt.task.tags.as_slice())),
    );
    let sections = cut_sections(&text, &labels);

    // The mirror of the check below, and the half `A1-172` did not close: a
    // milestone can exist, be live, and hold work, with no section in the
    // narrative at all -- and every gate was silent about it. It matters more
    // since `A1-170` made creating a cut a one-line command, because a cut can
    // now be born, filled and sequenced without anybody opening this document.
    //
    // **Non-terminal only**, which is what makes `M-v0.1` right to be silent
    // about: it is `done`, holds two closed tasks, and predates the document it
    // is missing from. A cut nobody can add to any more is history, and the
    // narrative describing where the work is going owes it nothing.
    //
    // A `release` is not exempt. It is a checkpoint rather than work
    // (`A1-146`), and a reader needs a checkpoint described as much as a cut --
    // this plan's one live release has a section, so exempting them would be
    // inventing a rule for a case that does not exist.
    //
    // `ROADMAP.md` only, because that is where this repository's cut sections
    // live. A cut described in `CHANGELOG.md` and nowhere else is still
    // undocumented for the reader looking for what is planned.
    let described: BTreeSet<&str> = sections.iter().map(|(cut, _)| cut.as_str()).collect();
    let live = plan.tasks.iter().filter_map(|lt| {
        (lt.task.is_milestone() && !lt.task.status.is_terminal()).then_some(lt.task.id.as_str())
    });
    for id in undescribed_cuts(live, &described) {
        problems.push(format!(
            "{id}: is live and holds work, and `ROADMAP.md` has no `###` section for it; write \
             one, or close the cut"
        ));
    }

    for (cut, body) in sections {
        let milestone = format!("M-{cut}");
        let exists = plan.tasks.iter().any(|lt| lt.task.id == milestone);
        match (declared_tasks(&body), exists) {
            // An unplanned cut that says so, and a planned one that declares its
            // tasks: the two shapes that are right. *Which* tasks is
            // `aplan check-docs`'s answer now, not this one's.
            (Some(Declaration::None), false) | (Some(Declaration::Tasks), true) => {}
            (Some(Declaration::None), true) => problems.push(format!(
                "{cut}: the narrative says there is no milestone, but `{milestone}` exists"
            )),
            (Some(Declaration::Tasks), false) => problems.push(format!(
                "{cut}: the narrative declares tasks, but there is no `{milestone}` milestone"
            )),
            (None, false) => problems.push(format!(
                "{cut}: no `{milestone}` milestone and no declaration; add `**Tasks:** none yet \
                 (no milestone)` so an unplanned cut is explicit rather than silent"
            )),
            (None, true) => problems.push(format!(
                "{cut}: `{milestone}` exists but the narrative declares no `**Tasks:**` line"
            )),
        }
    }

    // 5. The decision record checks itself: unique numbers, filenames that agree
    //    with their headings, and citations that resolve. Nothing asked any of
    //    these until `A1-222` found all three broken at once.
    let adr = adr_scan(&root, &tracked_files(&root)?);
    problems.extend(adr.problems.iter().cloned());

    if problems.is_empty() {
        println!(
            "check-docs: {} doc(s) agree with the plan, {} ADR(s) with {} citation(s) resolving",
            sources.len(),
            adr.records,
            adr.citations,
        );
        for line in adr.aside() {
            println!("  {line}");
        }
        return Ok(());
    }
    for p in &problems {
        eprintln!("  {p}");
    }
    bail!(
        "{} disagreement(s) between the docs and the plan (a reference is {ID_PATTERN_HELP})",
        problems.len()
    )
}

/// Every rule the changelog *announces*, as against merely mentions.
///
/// A code is announced when it is the subject of a top-level entry, or when a
/// range covers it (`APLAN001`-`APLAN010` announces ten rules and is the honest
/// way to describe a founding set).
///
/// **Not `changelog.contains(code)`**, which is the predicate `A1-092` deleted
/// from this very file as useless and which `A1-067` then shipped a few hundred
/// lines away. It passes on any incidental mention, so `APLAN029` would be
/// "announced" by a sentence in `APLAN012`'s entry saying the two are related,
/// and it fails a rule announced only by a range. Ten of the twenty-six rules
/// were passing that way, none of them for the reason the gate believed.
fn announced_rules(changelog: &str) -> BTreeSet<String> {
    let mut announced = BTreeSet::new();

    // A range: `APLANaaa`-`APLANbbb`, covering everything between.
    let bytes: Vec<char> = changelog.chars().collect();
    let codes: Vec<(usize, u16)> = code_positions(&bytes);
    for pair in codes.windows(2) {
        let (end_of_first, low) = (pair[0].0 + 8, pair[0].1);
        let (start_of_second, high) = (pair[1].0, pair[1].1);
        let between: String = bytes[end_of_first..start_of_second].iter().collect();
        if between.trim_matches('`').trim() == "-" && low <= high {
            for n in low..=high {
                announced.insert(format!("APLAN{n:03}"));
            }
        }
    }

    // The subject of an entry: any code in the opening paragraph of a top-level
    // bullet. A code first named three paragraphs into somebody else's entry is
    // a mention, not an announcement.
    for opening in entry_openings(changelog) {
        for (_, n) in code_positions(&opening.chars().collect::<Vec<char>>()) {
            announced.insert(format!("APLAN{n:03}"));
        }
    }
    announced
}

/// The opening paragraph of every top-level changelog entry.
///
/// What "announced" means, in one place, because two checks need it and the
/// audit of `A1-169` found them disagreeing: the rule check has always required
/// an entry's opening paragraph, and the config-key check shipped a
/// `changelog.contains` -- the predicate the comment above condemns by name,
/// added a few lines below it.
fn entry_openings(changelog: &str) -> Vec<&str> {
    changelog
        .split("\n- ")
        .map(|entry| entry.split("\n\n").next().unwrap_or_default())
        .collect()
}

/// Every `APLANnnn` in `text`, as `(char index, number)`.
fn code_positions(text: &[char]) -> Vec<(usize, u16)> {
    let mut out = Vec::new();
    for i in 0..text.len().saturating_sub(7) {
        if text[i..i + 5].iter().collect::<String>() == "APLAN"
            && text[i + 5..i + 8].iter().all(char::is_ascii_digit)
        {
            let n: u16 = text[i + 5..i + 8]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or_default();
            out.push((i, n));
        }
    }
    out
}

/// Rules the changelog never announces, as a problem line, or empty.
///
/// Split out so it can be tested in both directions: a gate nobody has watched
/// fail is a gate nobody knows the shape of.
/// Every top-level `.aplan.yml` key this build accepts and `CHANGELOG.md` never
/// names.
///
/// A config key is the consumer contract a rule code is, with a sharper failure:
/// an unknown key is refused, so a repository that adopts a key its staged binary
/// predates loses **every command that reads config**, not one gate. `A1-169`
/// found the tool doing that to this repository with its own release binary, and
/// found `docs` -- added the day before -- named nowhere in the changelog.
///
/// Top-level only. A nested key is part of the shape of the key above it:
/// `test.name` and `test.dir` arrive when a repository adopts `test`, and
/// demanding a line for each would ask the changelog to restate the schema.
///
/// Announced means what it means for a rule code: named in the opening paragraph
/// of a top-level entry, not merely present in the file. The first version of
/// this check used `changelog.contains`, which is what `announced_rules` refuses
/// by name three lines up, and which would have passed a key first mentioned deep
/// inside somebody else's entry.
///
/// **Not the upgrade guide's table**, which was the first design and does not
/// survive measurement: that table is scoped to what changed since a date, so ten
/// of eighteen keys predate it and belong nowhere in it. And a row is matched by
/// finding the key's name in its first cell, which misfires -- of eight rows that
/// name one, `advisory` there means a count in `--format json` output rather than
/// the config key. A gate over prose that already has a false positive in eight
/// cases is the `contains` predicate this repository has twice deleted.
fn unannounced_config_keys(changelog: &str) -> Option<String> {
    let openings = entry_openings(changelog);
    let unannounced: Vec<String> = aplan_config::accepted_keys()
        .into_iter()
        .filter(|k| !k.contains('.'))
        .filter(|k| {
            let quoted = format!("`{k}`");
            !openings.iter().any(|o| o.contains(&quoted))
        })
        .collect();
    (!unannounced.is_empty()).then(|| {
        format!(
            "CHANGELOG.md never names the config key(s) {}: an unknown key is refused, so a \
             repository adopting one its binary predates loses every command that reads config. \
             Name it in the entry that adds it",
            unannounced.join(", ")
        )
    })
}

fn unannounced_rules(changelog: &str) -> Option<String> {
    let announced = announced_rules(changelog);
    let unannounced: Vec<&str> = aplan_core::Rule::ALL
        .iter()
        .map(|r| r.code())
        .filter(|code| !announced.contains(*code))
        .collect();
    (!unannounced.is_empty()).then(|| {
        format!(
            "CHANGELOG.md never announces {}: a rule code is a consumer contract, so \
             shipping one unannounced is a contract change nobody was told about. A \
             mention inside another entry does not count; make it the subject of its \
             own, or name it in a range",
            unannounced.join(", ")
        )
    })
}

/// Live cuts the narrative has no section for (`A1-181`).
///
/// The mirror of [`cut_sections`], and the half `A1-172` did not close: a
/// milestone can exist, be live, and hold work, with no section in the document
/// at all -- and every gate was silent about it. It matters more since `A1-170`
/// made creating a cut a one-line command, because a cut can now be born, filled
/// and sequenced without anybody opening the narrative.
///
/// **Non-terminal only**, which is what makes `M-v0.1` right to be silent about:
/// it is `done`, holds two closed tasks, and predates the document it is missing
/// from. A cut nobody can add to any more is history, and a narrative describing
/// where the work is going owes it nothing.
///
/// A `release` is not exempt. It is a checkpoint rather than work (`A1-146`),
/// and a reader needs a checkpoint described as much as a cut -- this plan's one
/// live release has a section, so exempting them would be inventing a rule for a
/// case that does not exist.
///
/// `ROADMAP.md` only, because that is where this repository's cut sections live,
/// and a `###` there only. Both measured rather than assumed: a cut whose
/// heading is level two, and a cut described only in `CHANGELOG.md`, are each
/// reported -- which is the intended answer in both cases, and the message says
/// `###` so the first reads as the convention it is rather than as a gate that
/// cannot see.
fn undescribed_cuts<'a>(
    live_milestones: impl Iterator<Item = &'a str>,
    described: &BTreeSet<&str>,
) -> Vec<&'a str> {
    live_milestones
        .filter(|id| {
            id.strip_prefix("M-")
                .is_none_or(|label| !described.contains(label))
        })
        .collect()
}

/// The cut labels this plan teaches: a milestone's id without its `M-`.
///
/// Tagged `milestone`, not merely named like one (ADR-0008: a cut *is* a task
/// carrying that tag). Nothing in this repository's own plan distinguishes the
/// two, so the filter is intent rather than a measured difference -- which is
/// why it is a function with a test rather than a line in the middle of a gate.
fn cut_labels_of<'a>(tasks: impl Iterator<Item = (&'a str, &'a [String])>) -> BTreeSet<String> {
    tasks
        .filter(|(_, tags)| tags.iter().any(|tag| tag == "milestone"))
        .filter_map(|(id, _)| id.strip_prefix("M-").map(str::to_string))
        .collect()
}

/// The narrative's cut sections, with their bodies.
///
/// Two rules, both borrowed rather than re-derived (`A1-172`).
///
/// **Which heading is a cut** used to be `starts_with('v') && contains('.')`,
/// which is not what a cut label is: ADR-0013 says a cut may be labelled
/// anything that orders naturally, and names `s2` and `phase-2` as examples. So
/// the `### public, Going public` section was invisible here, and the half of
/// this gate that lives in `xtask` -- an unplanned cut must *say* it has no
/// milestone rather than pass by being silent -- did not apply to it.
///
/// It cannot simply iterate the milestones, which is what the shipped
/// `check-docs` does: a cut with no milestone is the case this half exists for,
/// so a rule that only recognises cuts that exist would be blind exactly where
/// it is needed. It learns the vocabulary instead: a label is a cut if the plan
/// has a milestone by that name, or if it is one of that plan's own labels with
/// a different number on the end -- `v0.9` where `v0.8` exists. A heading that
/// is neither is not a cut, which is the property the narrow rule bought.
///
/// **What the label is** is [`aplan_interop::docs::heading_label`], the shipped
/// command's own reader, so `**v0.8**` and `[v0.8](#x)` and `v0.8:` are read
/// here exactly as they are there. And the sections come from
/// [`aplan_core::anchor::sections`], which knows what a fenced block is: the
/// hand-rolled scan this replaces would have read a `### v9.9` inside a fenced
/// example as a cut this repository had promised.
///
/// **Two boundaries, both deliberate.** A plan that declares no milestones
/// teaches no labels, so nothing here is a cut and this half says nothing: with
/// no vocabulary there is no rule, and inventing one is what this replaced. And
/// a cut with *two* sections is not reported here either -- the shipped command
/// answers that (`more than one heading here names M-public`), which is the same
/// division of labour as the membership half.
fn cut_sections(text: &str, labels: &BTreeSet<String>) -> Vec<(String, String)> {
    let vocabulary = aplan_interop::docs::Vocabulary::of(labels);
    aplan_core::anchor::sections(text)
        .into_iter()
        .filter(|section| section.level == 3)
        .filter_map(|section| {
            let label = aplan_interop::docs::heading_label(&section.title);
            is_cut_label(&label, labels, &vocabulary).then_some((label, section.body))
        })
        .collect()
}

/// Whether `label` names a cut of this plan, or one it could have.
fn is_cut_label(
    label: &str,
    labels: &BTreeSet<String>,
    vocabulary: &aplan_interop::docs::Vocabulary,
) -> bool {
    if label.is_empty() {
        return false;
    }
    if labels.contains(label) {
        return true;
    }
    vocabulary.prefixes().iter().any(|prefix| {
        label.strip_prefix(prefix).is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
    })
}

/// The ids a cut section declares on its `**Tasks:**` line.
fn declared_tasks(body: &str) -> Option<Declaration> {
    let line = body
        .lines()
        .find(|l| l.trim_start().starts_with("**Tasks:**"))?;
    if line.contains("none yet") {
        return Some(Declaration::None);
    }
    // *That* it declares tasks, not which: which is `aplan check-docs`'s answer
    // now, and reading the line here again would be the second reader this file
    // exists not to have.
    Some(Declaration::Tasks)
}

/// What a cut section declares about its membership.
///
/// "No milestone yet" is a *declaration*, not an absence: an unplanned cut has to
/// say so, because a cut that passes by being silent is exactly how a promise
/// goes unnoticed.
enum Declaration {
    Tasks,
    None,
}

fn workspace_root() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR is `<root>/xtask`.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.parent()
        .map(std::path::Path::to_path_buf)
        .context("locating workspace root")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_runner_bounds_instrumented_test_threads() {
        let command = coverage_command();
        let threads = command
            .get_envs()
            .find_map(|(key, value)| (key == "RUST_TEST_THREADS").then_some(value))
            .flatten();
        assert_eq!(threads, Some(std::ffi::OsStr::new(COVERAGE_TEST_THREADS)));
        assert_eq!(COVERAGE_TEST_THREADS, "4");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["llvm-cov", "--workspace", "--json", "--summary-only"].map(std::ffi::OsStr::new)
        );
    }

    fn floors(pairs: &[(&str, f64)]) -> std::collections::BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[cfg(target_os = "linux")]
    fn wait_for_path_state(path: &Path, exists: bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while path.exists() != exists && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            path.exists(),
            exists,
            "timed out waiting for {}",
            path.display()
        );
    }

    #[cfg(target_os = "linux")]
    fn wait_for_live_pid(path: &Path) -> u32 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut observed = "the file does not exist".to_string();
        loop {
            match std::fs::read_to_string(path) {
                Ok(contents) => match contents.trim().parse::<u32>() {
                    Ok(pid) if Path::new(&format!("/proc/{pid}")).is_dir() => return pid,
                    Ok(pid) => observed = format!("PID {pid} is not live"),
                    Err(error) => {
                        observed = format!("contents {contents:?} are not a PID: {error}");
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => observed = format!("the file cannot be read: {error}"),
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {} to publish a live PID; last observation: {observed}",
                path.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// The disposition gate, in every shape the corpus actually writes
    /// (`A1-111`).
    ///
    /// Against a committed fixture rather than the live log: the two p0000004
    /// findings that prompted this were dispositioned properly before the gate
    /// existed, and a document that has been fixed cannot demonstrate a gate
    /// that catches it.
    #[test]
    fn a_disposition_naming_a_placeholder_is_caught_and_one_naming_a_task_is_not() {
        let fixture = include_str!("../tests/fixtures/dispositions.md");
        let ids: BTreeSet<String> = ["A1-040", "A1-111", "M-v0.8"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let found = placeholder_dispositions(fixture, &ids);
        let targets: Vec<&str> = found.iter().map(|(_, _, t)| t.as_str()).collect();
        assert_eq!(
            targets,
            vec!["A1-nnn", "M-v0.N"],
            "the placeholder and the milestone placeholder, and nothing else: {found:?}"
        );

        // Every reported line is the line the target is actually on. Without
        // this the number is decorative, and a paragraph-scoped scan makes it
        // easy to report an offset into the paragraph instead: the compiler
        // caught that shadowing once, and these tests did not.
        let lines: Vec<&str> = fixture.split('\n').collect();
        for (line, _, target) in &found {
            assert!(
                lines[line - 1].contains(target.as_str()),
                "line {line} is `{}`, which does not hold `{target}`",
                lines[line - 1]
            );
        }

        // The finding is named, not merely the line, because a log's reader
        // needs to know *which* item is lying about being handled.
        let (line, ..) = found[0];
        assert!(
            finding_at(fixture, line).contains("A finding whose disposition names a placeholder"),
            "{}",
            finding_at(fixture, line)
        );

        // And a plan that teaches no prefix cannot be helped by any of this,
        // rather than matching every word in the document.
        assert!(placeholder_dispositions(fixture, &BTreeSet::new()).is_empty());
    }

    /// The four shapes that must *not* fire, each for its own reason.
    #[test]
    fn a_quoted_claim_a_fenced_one_and_a_resolvable_id_all_pass() {
        let ids: BTreeSet<String> = ["A1-040"].into_iter().map(str::to_string).collect();
        let passes = |text: &str| placeholder_dispositions(text, &ids).is_empty();

        // Quoting a claim is not making one, which is how the two files that
        // explain the convention are allowed to show it.
        assert!(passes(
            "an item that says \"Tracked as `A1-nnn`\" reads as handled"
        ));
        assert!(passes(
            "the literal line ``Tracked as `A1-nnn`.`` sat there"
        ));
        // A placeholder with no claim in front of it is the documented
        // convention doing its job.
        assert!(passes(
            "Write a placeholder in lowercase (`A1-nnn`) for a task id."
        ));
        // A fence is a document showing its own format.
        assert!(passes("```\nTracked as `A1-nnn`.\n```\n"));
        // A real disposition.
        assert!(passes("Tracked as `A1-040`."));
        // And a number is a reference that resolved to nothing, reported by
        // `undefined_ids` with a suggestion; saying it twice would be worse.
        assert!(passes("Tracked as `A1-999`."));

        // A finding tracked to a *milestone* that exists. Its tail is not all
        // digits, so nothing upstream of the id check excuses it: this is the
        // only shape that proves a real id is looked up rather than assumed
        // from the shape of the word.
        let dotted: BTreeSet<String> = ["M-v0.8"].into_iter().map(str::to_string).collect();
        assert!(
            placeholder_dispositions("Tracked as `M-v0.8`.", &dotted).is_empty(),
            "a milestone that exists is a disposition like any other"
        );
        assert!(
            !placeholder_dispositions("Tracked as `M-v0.9`.", &dotted).is_empty(),
            "and one that does not is not"
        );

        // A disposition that wraps. This repository sets its prose at about a
        // hundred columns, so a claim and its target land on different lines
        // whenever the sentence is long enough -- pre-public adoption evidence has
        // one -- and a scan that reads a line at a time cannot see it.
        let wrapped = placeholder_dispositions(
            "the finding is unusual among the projects adopting. Tracked as\n`A1-nnn`.",
            &ids,
        );
        assert_eq!(wrapped.len(), 1, "a sentence is not a line: {wrapped:?}");
        assert_eq!(
            wrapped[0].0, 2,
            "reported where the target is, not where the claim starts"
        );
        assert_eq!(
            wrapped[0].1, "Tracked as",
            "the claim as written, unwrapped"
        );
        // Wrapped the other way, a quotation opening before the wrap and
        // closing after it is still a quotation. This is the paragraph in
        // pre-public adoption evidence that explains the rule.
        assert!(passes(
            "**A placeholder in a disposition is rejected.** \"Tracked as\n`A1-nnn`\" claims the work is somewhere."
        ));
        // But a blank line ends both, so a stray mark cannot reach across it.
        assert!(
            !passes("a \"quotation opens here\n\nTracked as `A1-nnn`."),
            "a paragraph break ends a quotation"
        );

        // A fence ends a paragraph even with no blank line around it. Without
        // that, a quotation opening before the fence would pair with one after
        // and mask a claim that has nothing to do with either.
        assert!(
            !passes("a \"quotation\n```\nfenced\n```\nand Tracked as `A1-nnn`\" done."),
            "prose either side of a fence is two paragraphs, not one"
        );

        // And the fence itself is skipped, whichever marker it uses. A
        // ``` fence is swallowed by the code-span mask as well, which hides
        // whether the fence is being honoured at all; `~~~` is the marker that
        // separates the two.
        assert!(passes("~~~\nTracked as `A1-nnn`.\n~~~\n"));

        // An unpaired quotation mark quotes nothing. Otherwise one stray `"`
        // would switch the gate off for the rest of the line, which is the way
        // a gate stops working without anybody noticing.
        assert!(
            !passes("The \"close command. Tracked as `A1-nnn`."),
            "an unclosed quote must not silence the claim after it"
        );

        // A disposition written without backticks is still a disposition, and
        // the sentence's full stop is the sentence's, not the id's -- the rule
        // `references` already applies to every other id in the prose.
        let bare = placeholder_dispositions("Tracked as A1-nnn.", &ids);
        assert_eq!(bare.len(), 1, "{bare:?}");
        assert_eq!(bare[0].2, "A1-nnn", "the dot ends the sentence");

        // The longest prefix wins, which only a plan teaching a prefix of a
        // prefix can show. `M-v9` is `M-v` plus a number, so it is a reference
        // that resolved to nothing; read as `M-` plus `v9` it would be a
        // placeholder, and reported twice in two voices.
        let nested: BTreeSet<String> = ["M-1", "M-v2"].into_iter().map(str::to_string).collect();
        assert!(
            placeholder_dispositions("Tracked as `M-v9`.", &nested).is_empty(),
            "the longest prefix the plan teaches is the one that applies"
        );

        // A word, not a prefix of one, in both directions. `backfiled` is
        // English and `trackedas` is not, and the second is the only shape that
        // separates a boundary check on the end of the word from none.
        assert!(passes("backfiled as `A1-nnn`"));
        assert!(passes("trackedas `A1-nnn`"));

        // The claim itself, in each preposition, so narrowing one is a visible
        // change rather than a silent hole.
        for claim in [
            "Tracked as `A1-nnn`.",
            "tracked in `A1-nnn`",
            "Tracked by `A1-nnn`",
            "filed under `A1-nnn`",
            "**Filed as `A1-nnn`**",
        ] {
            assert!(
                !passes(claim),
                "a claim of tracking, however it is written: {claim}"
            );
        }
    }

    /// The note the generators print when the working tree holds work a clean
    /// checkout does not (`A1-144`).
    ///
    /// Driven against a real git repository rather than a stubbed listing: what
    /// is being checked is that `git ls-files --others` is asked the right
    /// question and that the answer becomes a sentence an author can act on.
    ///
    /// **Not driven through the xtask binary**, and the reason is recorded
    /// rather than left implied: that binary resolves its root from
    /// `CARGO_MANIFEST_DIR` at compile time, so it can only ever read *this*
    /// repository's plan -- and creating an untracked task file there mid-run
    /// would race the tests that read the live plan (`crates/aplan/tests/policy.rs`
    /// among them). A shared mutable directory is a worse test than a temporary
    /// repository.
    #[test]
    fn an_untracked_task_file_is_noted_for_the_author() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("aplan/tasks")).expect("dirs");
        for (name, args) in [
            ("init", vec!["init", "-q"]),
            ("email", vec!["config", "user.email", "t@t"]),
            ("name", vec!["config", "user.name", "t"]),
        ] {
            let ok = Command::new("git")
                .args(&args)
                .current_dir(root)
                .status()
                .expect("git")
                .success();
            assert!(ok, "git {name} failed");
        }
        std::fs::write(root.join("aplan/tasks/A-1.md"), "---\nid: A-1\n---\n").expect("task");
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(root)
            .status()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-qm", "committed"])
            .current_dir(root)
            .status()
            .expect("git commit");

        // A clean tree says nothing: the note is about a difference, and there
        // is none.
        assert_eq!(untracked_plan_note(root, "this rollup"), None);

        // The case that turned CI red twice: a task written and not committed.
        std::fs::write(root.join("aplan/tasks/A-2.md"), "---\nid: A-2\n---\n").expect("task");
        let note = untracked_plan_note(root, "this rollup").expect("the note");
        assert!(note.contains("A-2.md"), "it must name the file: {note}");
        assert!(
            note.contains("does not track"),
            "which state it is in: {note}"
        );
        assert!(
            note.contains("clean checkout") && note.contains("CI"),
            "it must say what it means for the gate: {note}"
        );
        assert!(
            !note.contains("error") && !note.contains("refus"),
            "a note is not a refusal: {note}"
        );

        // The mirror, found by auditing the fix: a task deleted and not yet
        // committed makes the rollup *omit* work a clean checkout still has,
        // which fails the gate in CI exactly the same way.
        std::fs::remove_file(root.join("aplan/tasks/A-2.md")).expect("rm");
        std::fs::remove_file(root.join("aplan/tasks/A-1.md")).expect("rm");
        let note = untracked_plan_note(root, "this rollup").expect("the note");
        assert!(
            note.contains("A-1.md") && note.contains("deleted here"),
            "a deletion is the same hazard and must be named as one: {note}"
        );

        // A file *modified* and not committed is the normal state at the moment
        // a rollup is regenerated -- close a task, regenerate, commit both -- so
        // it is deliberately not listed. A note that fired on every run would
        // teach its reader to skip it.
        Command::new("git")
            .args(["checkout", "--", "aplan/tasks/A-1.md"])
            .current_dir(root)
            .status()
            .expect("git checkout");
        std::fs::write(
            root.join("aplan/tasks/A-1.md"),
            "---\nid: A-1\nstatus: done\n---\n",
        )
        .expect("edit");
        assert_eq!(
            untracked_plan_note(root, "this rollup"),
            None,
            "an edited task is the normal state, not a hazard"
        );
        Command::new("git")
            .args(["checkout", "--", "aplan/tasks/A-1.md"])
            .current_dir(root)
            .status()
            .expect("git checkout");

        // A file git ignores is not a surprise to CI, and a file that is not a
        // task is not this note's business.
        std::fs::write(root.join(".gitignore"), "aplan/tasks/ignored.md\n").expect("ignore");
        std::fs::write(root.join("aplan/tasks/ignored.md"), "x").expect("file");
        std::fs::write(root.join("aplan/tasks/notes.txt"), "x").expect("file");
        assert_eq!(
            untracked_plan_note(root, "this rollup"),
            None,
            "an ignored file is no surprise to CI, and a file that is not a task is not \
             this note's business -- so there is nothing to say at all"
        );
    }

    /// A file measured twice is refused rather than summed (`A1-457`, audit).
    ///
    /// The CI runner mounts one `CARGO_TARGET_DIR` for every job, so
    /// `llvm-cov-target` carries profiles from earlier commits. A file whose
    /// content changed then has two line mappings and summing them halves the
    /// percentage -- `aplan-evidence` read 64.5% against 98.1% on the commit
    /// before, with the covered count unchanged to four lines and every test
    /// passing. A wrong number is worse than a missing one, so this says so.
    #[test]
    fn more_coverable_lines_than_lines_is_refused_rather_than_measured() {
        // A real file, because the check compares against what it has. This
        // source file is the one at hand.
        let here = concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs");
        let entry = |covered: u64, count: u64| {
            serde_json::json!({
                "filename": here,
                "summary": {"lines": {"covered": covered, "count": count}},
            })
        };

        // The workspace root, so the path relativises to `xtask/src/main.rs`.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits in the workspace");

        let plausible = [entry(9, 10)];
        assert_eq!(
            total_per_crate(&plausible, root).expect("a measurable file"),
            [("xtask".to_string(), (9, 10))].into_iter().collect()
        );

        let impossible = [entry(10, 10_000_000)];
        let err =
            total_per_crate(&impossible, root).expect_err("more coverable lines than the file has");
        let said = err.to_string();
        assert!(said.contains("main.rs"), "it names the file: {said}");
        assert!(said.contains("llvm-cov-target"), "and the remedy: {said}");
    }

    /// Cargo's resolved target and build directories, not the checkout, decide
    /// where the dedicated instrumented trees live (`A1-457`).
    #[test]
    fn coverage_isolation_follows_external_cargo_directories_and_touches_only_its_children() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let external = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("external cargo cache");
        let cargo_target = external.path().join("cargo-target");
        let cargo_build = external.path().join("cargo-build");
        let metadata = serde_json::json!({
            "target_directory": cargo_target,
            "build_directory": cargo_build,
        });
        let dirs = coverage_artifact_dirs_from_metadata(root.path(), &metadata, None, None)
            .expect("external directories resolve");
        assert_eq!(
            dirs,
            vec![
                cargo_target.join(LLVM_COV_TARGET),
                cargo_build.join(LLVM_COV_TARGET),
            ]
        );

        for configured in &dirs {
            std::fs::create_dir_all(configured).expect("old shared coverage directory");
            std::fs::write(configured.join("stale-object"), "old build").expect("stale object");
        }
        std::fs::write(external.path().join("keep"), "ordinary cache").expect("neighbour");
        let mut isolated = Vec::new();
        let answer = with_isolated_coverage_artifacts(&dirs, |run_dirs| {
            isolated = run_dirs.to_vec();
            for (configured, run_dir) in dirs.iter().zip(run_dirs) {
                assert_ne!(
                    run_dir, configured,
                    "the stale shared directory is not reused"
                );
                assert_eq!(run_dir.file_name(), configured.file_name());
                assert_eq!(
                    run_dir.parent().and_then(Path::parent),
                    configured.parent(),
                    "the private run stays on the configured filesystem"
                );
                assert!(configured.join("stale-object").is_file());
                assert!(!run_dir.join("stale-object").exists());
                std::fs::create_dir(run_dir)?;
                std::fs::write(run_dir.join("new-object"), "this build")?;
            }
            Ok(7_u8)
        })
        .expect("isolated measurement");
        assert_eq!(answer, 7, "the measurement runs inside fresh directories");
        assert!(
            isolated.iter().all(|dir| !dir.exists()),
            "both private directories were removed: {isolated:?}"
        );
        assert!(
            dirs.iter().all(|dir| dir.join("stale-object").is_file()),
            "pre-existing shared artifacts were neither used nor deleted"
        );
        assert!(
            external.path().join("keep").is_file(),
            "nothing beside the exact targets was removed"
        );
    }

    /// The command-facing half reads the live Cargo configuration and returns
    /// bounded bases for private runs without touching them (`A1-457`).
    #[test]
    fn coverage_identification_follows_live_cargo_metadata_without_cleaning() {
        let root = workspace_root().expect("workspace");
        let dirs = coverage_artifact_dirs(&root).expect("resolved Cargo artifact directories");
        assert!(!dirs.is_empty(), "Cargo always has a target directory");
        assert!(
            dirs.len() <= 2,
            "only target and build directories: {dirs:?}"
        );
        for dir in dirs {
            assert!(
                dir.is_absolute(),
                "Cargo resolved an absolute path: {dir:?}"
            );
            assert_eq!(
                dir.file_name(),
                Some(std::ffi::OsStr::new(LLVM_COV_TARGET)),
                "isolation remains anchored at cargo-llvm-cov's generated directory"
            );
        }
    }

    /// An override can move the artifacts, but cannot anchor isolation at a
    /// broad directory (`A1-457`).
    #[test]
    fn coverage_isolation_refuses_an_override_that_is_not_a_dedicated_directory() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let err = coverage_artifact_dirs_from_metadata(
            root.path(),
            &serde_json::json!({}),
            Some("/tmp"),
            None,
        )
        .expect_err("a broad override must be refused");
        let said = err.to_string();
        assert!(said.contains("CARGO_LLVM_COV_TARGET_DIR"), "{said}");
        assert!(said.contains(LLVM_COV_TARGET), "{said}");
        assert!(said.contains("refusing recursive cleanup"), "{said}");
    }

    /// Relative overrides have the same workspace-root base cargo-llvm-cov
    /// gives them, while a separate build override remains a second exact
    /// isolation base (`A1-457`).
    #[test]
    fn coverage_isolation_mirrors_relative_override_precedence() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let dirs = coverage_artifact_dirs_from_metadata(
            root.path(),
            &serde_json::json!({}),
            Some("target/llvm-cov-target"),
            Some("build/llvm-cov-target"),
        )
        .expect("relative dedicated overrides");
        assert_eq!(
            dirs,
            vec![
                root.path().join("target/llvm-cov-target"),
                root.path().join("build/llvm-cov-target"),
            ]
        );

        let metadata_target = root.path().join("metadata-target");
        let dirs = coverage_artifact_dirs_from_metadata(
            root.path(),
            &serde_json::json!({"target_directory": metadata_target}),
            None,
            Some("ignored/llvm-cov-target"),
        )
        .expect("a build-only override is ignored");
        assert_eq!(
            dirs,
            vec![metadata_target.join(LLVM_COV_TARGET)],
            "cargo-llvm-cov ignores its build override unless its target override is set"
        );
    }

    /// Cargo metadata should resolve both artifact roots absolutely. Refuse a
    /// malformed new `build_directory` field rather than broadening isolation
    /// through an assumed base (`A1-457`).
    #[test]
    fn coverage_isolation_refuses_an_unresolved_metadata_build_directory() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let err = coverage_artifact_dirs_from_metadata(
            root.path(),
            &serde_json::json!({
                "target_directory": root.path().join("target"),
                "build_directory": "relative-build",
            }),
            None,
            None,
        )
        .expect_err("an unresolved metadata directory must be refused");
        let said = err.to_string();
        assert!(said.contains("build_directory"), "{said}");
        assert!(said.contains("unsafe"), "{said}");
    }

    /// Missing or unresolved inputs are refusals, while a configured directory
    /// that does not exist yet gets a private clean run (`A1-457`).
    #[test]
    fn coverage_isolation_distinguishes_bad_identification_from_a_new_target() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let err =
            coverage_artifact_dirs_from_metadata(root.path(), &serde_json::json!({}), None, None)
                .expect_err("missing Cargo metadata must stop the measurement");
        assert!(err.to_string().contains("target_directory"), "{err}");

        let err = coverage_artifact_dirs_from_metadata(
            root.path(),
            &serde_json::json!({}),
            Some(""),
            None,
        )
        .expect_err("an unresolved override must not become the workspace");
        assert!(err.to_string().contains("is empty"), "{err}");

        let absent = root.path().join("not-created").join(LLVM_COV_TARGET);
        let measured = with_isolated_coverage_artifacts(&[absent], |dirs| {
            assert!(
                !dirs[0].exists(),
                "Cargo, not the isolator, creates the artifact child"
            );
            Ok("measured")
        })
        .expect("an absent configured directory accepts an isolated run");
        assert_eq!(measured, "measured");
    }

    /// A target whose private parent cannot be allocated prevents measurement,
    /// and a configured symlink is never recursively removed (`A1-457`).
    #[test]
    fn a_coverage_allocation_failure_stops_before_measurement() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let blocked_parent = root.path().join("not-a-directory");
        std::fs::write(&blocked_parent, "blocking file").expect("blocking file");
        let target = blocked_parent.join(LLVM_COV_TARGET);
        let measured = std::cell::Cell::new(false);
        let err = with_isolated_coverage_artifacts(std::slice::from_ref(&target), |_| {
            measured.set(true);
            Ok(())
        })
        .expect_err("allocation must stop the measurement");
        assert!(!measured.get(), "the measurement callback was reached");
        let said = err.to_string();
        assert!(
            said.contains(&blocked_parent.display().to_string()),
            "{said}"
        );
        assert!(said.contains("parent"), "{said}");

        #[cfg(unix)]
        {
            let destination = root.path().join("symlink-destination");
            std::fs::create_dir(&destination).expect("destination");
            std::fs::write(destination.join("keep"), "not ours").expect("destination marker");
            let symlink = root.path().join(LLVM_COV_TARGET);
            std::os::unix::fs::symlink(&destination, &symlink).expect("configured symlink");
            with_isolated_coverage_artifacts(&[symlink], |_| Ok(()))
                .expect("the run uses a private sibling, not the configured symlink");
            assert!(
                destination.join("keep").is_file(),
                "the configured symlink and its destination were untouched"
            );
        }
    }

    /// Cleanup failure invalidates an otherwise successful measurement. The
    /// private parent is removed non-recursively, so an unexpected neighbour is
    /// reported rather than erased (`A1-457`, audit).
    #[test]
    fn a_coverage_cleanup_failure_rejects_the_measurement() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let configured = root.path().join(LLVM_COV_TARGET);
        let err = with_isolated_coverage_artifacts(&[configured], |dirs| {
            let run_dir = dirs[0].parent().expect("private run");
            std::fs::write(run_dir.join("unexpected-neighbour"), "do not delete")?;
            Ok("a result that cleanup must invalidate")
        })
        .expect_err("cleanup failure must reject the result");
        let said = err.to_string();
        assert!(said.contains("no result was accepted"), "{said}");
        assert!(said.contains("non-recursively"), "{said}");
    }

    /// `A1-633`: only an exact allocation with a complete, dead ownership
    /// record is reclaimed. A live owner, PID reuse, a missing record and a
    /// symlink all retain the candidate.
    #[cfg(target_os = "linux")]
    #[test]
    fn stale_coverage_sweep_refuses_every_unverifiable_neighbour() {
        use std::os::unix::fs::symlink;

        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("coverage sweep fixture");
        let parent = root.path();
        let current = current_coverage_process_identity().expect("current identity");

        let stale = parent.join(".aplan-coverage-4294967295-0");
        std::fs::create_dir(&stale).expect("stale allocation");
        std::fs::write(stale.join("artifact"), "stale").expect("stale artifact");
        let mut stale_owner = CoverageOwnerRecord {
            version: 1,
            phase: CoverageOwnerPhase::Prepared,
            owner: current.clone(),
            child: None,
            process_group: None,
            lease: "aplan-coverage-v1-stale-fixture".to_string(),
        };
        stale_owner.owner.pid = u32::MAX;
        write_coverage_owner(&stale, &stale_owner).expect("stale owner record");

        let live = parent.join(format!(".aplan-coverage-{}-0", current.pid));
        std::fs::create_dir(&live).expect("live allocation");
        let live_owner = CoverageOwnerRecord {
            lease: "aplan-coverage-v1-live-fixture".to_string(),
            ..stale_owner.clone()
        };
        let live_owner = CoverageOwnerRecord {
            owner: current.clone(),
            ..live_owner
        };
        write_coverage_owner(&live, &live_owner).expect("live owner record");

        let reused = parent.join(format!(".aplan-coverage-{}-1", current.pid));
        std::fs::create_dir(&reused).expect("reused allocation");
        let mut reused_owner = live_owner.clone();
        reused_owner.owner.start_ticks = reused_owner.owner.start_ticks.saturating_add(1);
        reused_owner.lease = "aplan-coverage-v1-reused-fixture".to_string();
        write_coverage_owner(&reused, &reused_owner).expect("reused owner record");

        let missing = parent.join(".aplan-coverage-4294967294-0");
        std::fs::create_dir(&missing).expect("unrecorded allocation");
        let malformed = parent.join(".aplan-coverage-not-a-pid-0");
        std::fs::create_dir(&malformed).expect("malformed neighbour");
        let outside = parent.join("outside");
        std::fs::create_dir(&outside).expect("symlink destination");
        std::fs::write(outside.join("keep"), "outside").expect("outside marker");
        let link = parent.join(".aplan-coverage-4294967293-0");
        symlink(&outside, &link).expect("coverage-shaped symlink");

        assert_eq!(sweep_stale_coverage_runs(parent).expect("sweep"), 1);
        assert!(!stale.exists(), "the proved-stale allocation was retained");
        assert!(live.is_dir(), "the live owner was removed");
        assert!(reused.is_dir(), "a reused PID was treated as the old owner");
        assert!(missing.is_dir(), "a missing ownership record was trusted");
        assert!(malformed.is_dir(), "a malformed name was treated as owned");
        assert!(link.is_symlink(), "a coverage-shaped symlink was removed");
        assert!(
            outside.join("keep").is_file(),
            "a symlink target was touched"
        );
    }

    /// `A1-633`: the ownership grammar distinguishes an exact process from a
    /// reused PID and refuses malformed proc/name/lease inputs one boundary at
    /// a time.
    #[cfg(target_os = "linux")]
    #[test]
    fn proc_disappearance_is_distinct_from_an_unverifiable_error() {
        assert!(proc_entry_is_gone(&std::io::Error::from(
            std::io::ErrorKind::NotFound
        )));
        assert!(proc_entry_is_gone(&std::io::Error::from_raw_os_error(3)));
        assert!(!proc_entry_is_gone(&std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        )));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn coverage_ownership_identity_and_name_predicates_are_strict() {
        assert_eq!(
            proc_uid("Name:\tx\nUid:\t1000\t1000\t1000\t1000\n").unwrap(),
            1000
        );
        assert!(
            proc_uid("Name:\tx\n")
                .unwrap_err()
                .to_string()
                .contains("Uid")
        );
        assert!(
            proc_uid("Uid:\n")
                .unwrap_err()
                .to_string()
                .contains("empty")
        );
        assert!(proc_uid("Uid:\tnot-a-number\n").is_err());

        let mut fields = vec!["0"; 20];
        fields[0] = "S";
        fields[2] = "17";
        fields[19] = "12345";
        let stat = format!("9 (a tricky ) process) {}", fields.join(" "));
        assert_eq!(proc_start_and_group(&stat).unwrap(), (12_345, 17));
        assert!(proc_start_and_group("no delimiter").is_err());
        assert!(proc_start_and_group("1 (x) S").is_err());
        fields[2] = "bad";
        assert!(proc_start_and_group(&format!("1 (x) {}", fields.join(" "))).is_err());
        fields[2] = "17";
        fields[19] = "bad";
        assert!(proc_start_and_group(&format!("1 (x) {}", fields.join(" "))).is_err());

        for (name, expected) in [
            (".aplan-coverage-1-0", Some(1)),
            (".aplan-coverage-42-9", Some(42)),
            (".aplan-coverage-0-0", None),
            (".aplan-coverage-01-0", None),
            (".aplan-coverage-1-00", None),
            (".aplan-coverage-1-x", None),
            (".aplan-coverage-1-0-extra", None),
            (".aplan-coverage-4294967296-0", None),
            ("aplan-coverage-1-0", None),
        ] {
            assert_eq!(coverage_candidate_pid(name), expected, "candidate {name}");
        }
        for lease in ["x", "aplan-coverage-v1-1_2.3"] {
            assert!(valid_coverage_lease(lease), "valid lease {lease}");
        }
        let overlong = "x".repeat(257);
        for lease in ["", "has space", "has/slash", overlong.as_str()] {
            assert!(!valid_coverage_lease(lease), "invalid lease {lease}");
        }

        let current = current_coverage_process_identity().expect("current identity");
        assert_eq!(
            coverage_process_state(&current),
            CoverageProcessState::Matches
        );
        let mut reused = current.clone();
        reused.start_ticks = reused.start_ticks.saturating_add(1);
        assert_eq!(
            coverage_process_state(&reused),
            CoverageProcessState::Reused
        );
        let mut old_boot = current.clone();
        old_boot.boot_id = "00000000-0000-0000-0000-000000000000".to_string();
        assert_eq!(
            coverage_process_state(&old_boot),
            CoverageProcessState::Gone
        );
        let mut absent = current.clone();
        absent.pid = u32::MAX;
        assert_eq!(coverage_process_state(&absent), CoverageProcessState::Gone);

        let record = CoverageOwnerRecord {
            version: 1,
            phase: CoverageOwnerPhase::Running,
            owner: absent.clone(),
            child: Some(current.clone()),
            process_group: Some(current.pid),
            lease: "aplan-coverage-v1-current-child".to_string(),
        };
        assert_eq!(coverage_tree_state(&record), CoverageTreeState::Live);
        let record = CoverageOwnerRecord {
            child: Some(reused),
            ..record
        };
        assert_eq!(
            coverage_tree_state(&record),
            CoverageTreeState::Unverifiable
        );
        let record = CoverageOwnerRecord {
            phase: CoverageOwnerPhase::Prepared,
            owner: old_boot,
            child: None,
            process_group: None,
            lease: "aplan-coverage-v1-old-boot".to_string(),
            version: 1,
        };
        assert_eq!(coverage_tree_state(&record), CoverageTreeState::Gone);
    }

    /// `A1-633`: a syntactically valid record is still rejected when its
    /// process identities or process-group relationship could not have been
    /// produced by the coverage lifecycle.
    #[cfg(target_os = "linux")]
    #[test]
    fn coverage_owner_record_requires_consistent_process_identities() {
        let current = current_coverage_process_identity().expect("current identity");
        assert!(valid_coverage_identity(&current));
        let mut malformed_identity = current.clone();
        malformed_identity.boot_id = "not-a-linux-boot-id".to_string();
        assert!(!valid_coverage_identity(&malformed_identity));
        malformed_identity = current.clone();
        malformed_identity.start_ticks = 0;
        assert!(!valid_coverage_identity(&malformed_identity));

        let record = CoverageOwnerRecord {
            version: 1,
            phase: CoverageOwnerPhase::Running,
            owner: current.clone(),
            child: Some(current.clone()),
            process_group: Some(current.pid),
            lease: "aplan-coverage-v1-consistent-record".to_string(),
        };
        assert!(consistent_coverage_owner(&record));
        let mut inconsistent = record.clone();
        inconsistent.process_group = Some(current.pid.saturating_add(1));
        assert!(!consistent_coverage_owner(&inconsistent));
        inconsistent = record;
        inconsistent.child.as_mut().expect("child").uid = current.uid.saturating_add(1);
        assert!(!consistent_coverage_owner(&inconsistent));
    }

    /// `A1-633`: an ownership record is trusted only as one bounded real file
    /// with the exact v1 shape and a phase consistent with its child fields.
    #[cfg(target_os = "linux")]
    #[test]
    fn coverage_owner_record_refuses_missing_hostile_and_inconsistent_files() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("owner-record fixture");
        let valid = new_coverage_owner_record().expect("valid owner");

        let absent = fixture.path().join("absent");
        std::fs::create_dir(&absent).expect("absent-record allocation");
        assert!(read_coverage_owner(&absent).is_err());

        let real = fixture.path().join("real");
        std::fs::create_dir(&real).expect("real allocation");
        write_coverage_owner(&real, &valid).expect("write valid record");
        assert_eq!(read_coverage_owner(&real).unwrap(), valid);

        let inconsistent = fixture.path().join("inconsistent");
        std::fs::create_dir(&inconsistent).expect("inconsistent allocation");
        let mut bad = valid.clone();
        bad.version = 2;
        write_coverage_owner(&inconsistent, &bad).expect("write future record");
        assert!(
            read_coverage_owner(&inconsistent)
                .unwrap_err()
                .to_string()
                .contains("inconsistent")
        );

        let malformed = fixture.path().join("malformed");
        std::fs::create_dir(&malformed).expect("malformed allocation");
        std::fs::write(malformed.join(COVERAGE_OWNER), "not json").expect("malformed record");
        assert!(
            read_coverage_owner(&malformed)
                .unwrap_err()
                .to_string()
                .contains("parsing")
        );

        let oversized = fixture.path().join("oversized");
        std::fs::create_dir(&oversized).expect("oversized allocation");
        std::fs::write(oversized.join(COVERAGE_OWNER), vec![b'x'; 16 * 1024 + 1])
            .expect("oversized record");
        assert!(
            read_coverage_owner(&oversized)
                .unwrap_err()
                .to_string()
                .contains("bounded real file")
        );

        let linked = fixture.path().join("linked");
        std::fs::create_dir(&linked).expect("linked allocation");
        symlink(real.join(COVERAGE_OWNER), linked.join(COVERAGE_OWNER))
            .expect("linked owner record");
        assert!(
            read_coverage_owner(&linked)
                .unwrap_err()
                .to_string()
                .contains("bounded real file")
        );

        let directory = fixture.path().join("directory");
        std::fs::create_dir_all(directory.join(COVERAGE_OWNER)).expect("directory-shaped record");
        assert!(read_coverage_owner(&directory).is_err());
    }

    /// `A1-633`: current-run cleanup does not silently accept a removed or
    /// replaced owner record merely because the instrumented child is absent.
    #[test]
    fn coverage_cleanup_requires_the_real_owner_record_it_created() {
        let fixture = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("cleanup owner fixture");
        let configured = fixture.path().join(LLVM_COV_TARGET);
        let err = with_isolated_coverage_artifacts(std::slice::from_ref(&configured), |dirs| {
            let owner = dirs[0]
                .parent()
                .expect("run directory")
                .join(COVERAGE_OWNER);
            std::fs::remove_file(owner)?;
            Ok(())
        })
        .expect_err("missing owner must invalidate cleanup");
        assert!(err.to_string().contains("owner record"), "{err:#}");

        let configured = fixture.path().join("second").join(LLVM_COV_TARGET);
        let err = with_isolated_coverage_artifacts(std::slice::from_ref(&configured), |dirs| {
            let owner = dirs[0]
                .parent()
                .expect("run directory")
                .join(COVERAGE_OWNER);
            std::fs::remove_file(&owner)?;
            std::fs::create_dir(&owner)?;
            Ok(())
        })
        .expect_err("directory-shaped owner must invalidate cleanup");
        assert!(
            err.to_string().contains("not the real ownership file"),
            "{err:#}"
        );

        let configured = fixture.path().join("third").join(LLVM_COV_TARGET);
        let err = with_isolated_coverage_artifacts(
            std::slice::from_ref(&configured),
            |dirs| -> Result<()> {
                let owner = dirs[0]
                    .parent()
                    .expect("run directory")
                    .join(COVERAGE_OWNER);
                std::fs::remove_file(owner)?;
                Err(anyhow::anyhow!("measurement also failed"))
            },
        )
        .expect_err("both failures must be preserved");
        let said = format!("{err:#}");
        assert!(said.contains("measurement also failed"), "{said}");
        assert!(said.contains("cleanup failed"), "{said}");
        assert!(said.contains("owner record"), "{said}");
    }

    /// `A1-633`: installing and dropping the signal owner without a signal is
    /// inert, including after a child identity has been published.
    #[cfg(unix)]
    #[test]
    fn coverage_signal_owner_is_inert_on_normal_completion() {
        let owner = CoverageSignalOwner::install().expect("install signal owner");
        owner.publish_child(u32::MAX);
        owner.clear_child();
        assert_eq!(owner.child_pid.load(std::sync::atomic::Ordering::SeqCst), 0);
        drop(owner);
        assert_eq!(
            COVERAGE_INTERRUPTED_BY.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        resume_coverage_signal();
    }

    /// `A1-633`: the real command lifecycle publishes its child identity and
    /// still returns a parsed report through the isolated-allocation cleanup.
    /// A fake producer keeps this focused on ownership rather than recursively
    /// invoking an instrumented Cargo build from the unit-test process.
    #[cfg(target_os = "linux")]
    #[test]
    fn coverage_command_runs_inside_and_cleans_its_owned_allocation() {
        let fixture = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("coverage command fixture");
        let source = fixture.path().join("xtask/src/main.rs");
        std::fs::create_dir_all(source.parent().expect("source parent")).expect("source tree");
        std::fs::write(&source, "covered line\n").expect("source");
        let report = serde_json::json!({
            "data": [{
                "files": [{
                    "filename": source,
                    "summary": {"lines": {"covered": 1, "count": 1}}
                }]
            }]
        })
        .to_string();
        let configured = fixture.path().join("target/llvm-cov-target");
        let mut allocated = None;

        let measured =
            with_isolated_coverage_artifacts(std::slice::from_ref(&configured), |artifact_dirs| {
                allocated = artifact_dirs
                    .first()
                    .and_then(|path| path.parent())
                    .map(Path::to_path_buf);
                let mut command = Command::new("/bin/sh");
                command.args([
                    "-c",
                    "/bin/sleep 0.1; printf '%s' \"$1\"",
                    "coverage-fixture",
                    &report,
                ]);
                run_coverage_command(fixture.path(), artifact_dirs, command)
            })
            .expect("measure fake report");

        assert_eq!(measured.get("xtask"), Some(&(1, 1)));
        assert!(
            !allocated.expect("private allocation").exists(),
            "the successful command left its private allocation behind"
        );
    }

    /// `A1-633`: a child failure is still an ordinary return through the
    /// isolation wrapper, so its stderr is reported only after the owned
    /// allocation has been removed.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_failed_coverage_command_cleans_its_owned_allocation() {
        let fixture = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("coverage failure fixture");
        let configured = fixture.path().join("target/llvm-cov-target");
        let mut allocated = None;

        let error =
            with_isolated_coverage_artifacts(std::slice::from_ref(&configured), |artifact_dirs| {
                allocated = artifact_dirs
                    .first()
                    .and_then(|path| path.parent())
                    .map(Path::to_path_buf);
                let mut command = Command::new("/bin/sh");
                command.args(["-c", "printf 'measured failure' >&2; exit 7"]);
                run_coverage_command(fixture.path(), artifact_dirs, command)
            })
            .expect_err("the failed producer must be reported");

        assert!(error.to_string().contains("measured failure"), "{error:#}");
        assert!(
            !allocated.expect("private allocation").exists(),
            "the failed command left its private allocation behind"
        );
    }

    /// `A1-633`: killing the recorded parent cannot make its inherited lease
    /// look stale while an uncooperative child remains. Once the complete group
    /// is gone, the next sweep reclaims the allocation.
    #[cfg(target_os = "linux")]
    #[test]
    fn stale_coverage_sweep_waits_for_a_killed_owners_live_child() {
        use std::os::unix::process::CommandExt as _;

        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("coverage process fixture");
        let descendant_pid_file = root.path().join("descendant.pid");
        let lease = "aplan-coverage-v1-live-child-fixture";
        let mut owner = Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("coverage owner process");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("sleep 60 & echo $! > \"$1\"; wait")
            .arg("coverage-child")
            .arg(&descendant_pid_file)
            .env(COVERAGE_LEASE_ENV, lease)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = command.spawn().expect("coverage child process");
        let process_group = child.id();
        let descendant_pid = wait_for_live_pid(&descendant_pid_file);
        let owner_identity = coverage_process_identity(owner.id()).expect("owner identity");
        let child_identity = coverage_process_identity(child.id()).expect("child identity");
        let allocation = root
            .path()
            .join(format!(".aplan-coverage-{}-0", owner.id()));
        std::fs::create_dir(&allocation).expect("coverage allocation");
        std::fs::write(allocation.join("artifact"), "live").expect("coverage artifact");
        write_coverage_owner(
            &allocation,
            &CoverageOwnerRecord {
                version: 1,
                phase: CoverageOwnerPhase::Running,
                owner: owner_identity,
                child: Some(child_identity),
                process_group: Some(process_group),
                lease: lease.to_string(),
            },
        )
        .expect("running owner record");

        owner.kill().expect("kill owning parent");
        owner.wait().expect("reap owning parent");
        assert_eq!(
            sweep_stale_coverage_runs(root.path()).expect("live sweep"),
            0
        );
        assert!(
            allocation.is_dir(),
            "a live descendant's allocation was removed"
        );

        child.kill().expect("kill recorded child");
        child.wait().expect("reap recorded child");
        assert_eq!(
            sweep_stale_coverage_runs(root.path()).expect("group sweep"),
            0
        );
        assert!(
            allocation.is_dir(),
            "a live process-group descendant's allocation was removed"
        );

        kill_coverage_process_group(process_group);
        let descendant_proc = PathBuf::from(format!("/proc/{descendant_pid}"));
        wait_for_path_state(&descendant_proc, false);
        // A same-boot sweep deliberately retains the allocation if any `/proc`
        // entry disappears while the full process table is being inspected.
        // Parallel tests create that harmless uncertainty frequently, and a
        // later scan is the production recovery path. Prove the dead tree is
        // eventually reclaimed without weakening that fail-closed policy.
        let mut removed = 0;
        for _ in 0..100 {
            removed += sweep_stale_coverage_runs(root.path()).expect("stale sweep");
            if removed == 1 {
                break;
            }
        }
        assert_eq!(
            removed, 1,
            "the proved-dead allocation survived 100 conservative scans"
        );
        assert!(!allocation.exists(), "the dead process tree was retained");
    }

    /// Two overlapping measurements against one configured target get distinct
    /// directories, and ending either run cannot remove the other's objects
    /// (`A1-457`, audit).
    #[test]
    fn overlapping_coverage_measurements_are_isolated() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let configured = root.path().join(LLVM_COV_TARGET);
        let mut outer_path = None;
        with_isolated_coverage_artifacts(std::slice::from_ref(&configured), |outer| {
            outer_path = Some(outer[0].clone());
            std::fs::create_dir(&outer[0])?;
            std::fs::write(outer[0].join("outer-object"), "outer")?;
            let mut inner_path = None;
            with_isolated_coverage_artifacts(std::slice::from_ref(&configured), |inner| {
                inner_path = Some(inner[0].clone());
                assert_ne!(inner[0], outer[0]);
                assert!(outer[0].join("outer-object").is_file());
                std::fs::create_dir(&inner[0])?;
                std::fs::write(inner[0].join("inner-object"), "inner")?;
                Ok(())
            })?;
            assert!(outer[0].join("outer-object").is_file());
            assert!(!inner_path.expect("inner path").exists());
            Ok(())
        })
        .expect("overlapping isolated runs");
        assert!(!outer_path.expect("outer path").exists());
    }

    /// The false negative that filed `A1-457`: two mappings can be merged while
    /// their sum remains below the source file's physical line count.
    #[test]
    fn a_mixed_mapping_below_the_physical_line_count_cannot_reach_floor_comparison() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("workspace");
        let source = root.path().join("xtask/src/main.rs");
        std::fs::create_dir_all(source.parent().expect("source parent")).expect("source tree");
        std::fs::write(&source, "line\n".repeat(100)).expect("source");
        let entry = |covered: u64, count: u64| {
            serde_json::json!({
                "filename": source,
                "summary": {"lines": {"covered": covered, "count": count}},
            })
        };

        let mixed = total_per_crate(&[entry(15, 30)], root.path())
            .expect("the old physical-line heuristic accepts 30 below 100");
        let mixed_pct = mixed
            .iter()
            .map(|(name, value)| (name.clone(), as_f64(&pct(*value))))
            .collect();
        let floor = floors(&[("xtask", 90.0)]);
        assert!(
            !compare_floors(&mixed_pct, &floor).is_empty(),
            "the accepted mixed mapping would have reached and failed the floor"
        );

        let configured = root.path().join(LLVM_COV_TARGET);
        std::fs::create_dir_all(&configured).expect("old shared coverage target");
        let stale = configured.join("mixed-mapping");
        std::fs::write(&stale, serde_json::to_vec(&entry(15, 30)).unwrap()).expect("stale mapping");
        let stale_reached_measurement = std::cell::Cell::new(false);
        let measured: BTreeMap<String, (u64, u64)> =
            with_isolated_coverage_artifacts(&[configured], |run_dirs| {
                stale_reached_measurement.set(run_dirs[0].join("mixed-mapping").exists());
                assert!(stale.exists(), "the old shared mapping was not consulted");
                Ok([("xtask".to_string(), (9, 10))].into_iter().collect())
            })
            .expect("isolated measurement");
        assert!(
            !stale_reached_measurement.get(),
            "the mixed mapping survived into measurement"
        );
        let measured_pct = measured
            .iter()
            .map(|(name, value)| (name.clone(), as_f64(&pct(*value))))
            .collect();
        assert!(
            compare_floors(&measured_pct, &floor).is_empty(),
            "only the one-build measurement reaches floor comparison"
        );
    }

    /// Files outside the workspace are skipped, not counted as a crate.
    #[test]
    fn coverage_counts_only_files_inside_the_workspace() {
        let root = Path::new("/w");
        let files = [
            serde_json::json!({
                "filename": "/home/x/.cargo/registry/src/serde/lib.rs",
                "summary": {"lines": {"covered": 1, "count": 1000}},
            }),
            serde_json::json!({
                "filename": "/w/xtask/src/main.rs",
                "summary": {"lines": {"covered": 5, "count": 10}},
            }),
        ];
        assert_eq!(
            total_per_crate(&files, root).expect("the workspace file counts"),
            [("xtask".to_string(), (5, 10))].into_iter().collect()
        );
        let error = total_per_crate(&files[..1], root)
            .expect_err("a report containing no workspace file must be refused");
        assert!(error.to_string().contains("no files inside the workspace"));
    }

    #[test]
    fn coverage_below_a_floor_is_reported() {
        let p = compare_floors(
            &floors(&[("aplan-core", 90.0)]),
            &floors(&[("aplan-core", 92.0)]),
        );
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("below its floor"), "{}", p[0]);
    }

    #[test]
    fn coverage_at_or_just_above_a_floor_is_accepted() {
        // Exactly at the floor passes: a floor is a minimum, not a target to beat.
        assert!(compare_floors(&floors(&[("a", 92.0)]), &floors(&[("a", 92.0)])).is_empty());
        assert!(compare_floors(&floors(&[("a", 93.9)]), &floors(&[("a", 92.0)])).is_empty());
    }

    #[test]
    fn a_floor_that_has_been_outgrown_is_reported() {
        // The ratchet: a crate that gained two points must bank them, or the gain
        // is free to be lost again without the gate noticing.
        let p = compare_floors(&floors(&[("a", 94.0)]), &floors(&[("a", 92.0)]));
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("outgrown"), "{}", p[0]);
    }

    #[test]
    fn a_new_crate_needs_a_floor_and_a_removed_one_loses_its_own() {
        let added = compare_floors(&floors(&[("new", 100.0)]), &floors(&[]));
        assert!(added[0].contains("no floor committed"), "{}", added[0]);
        let removed = compare_floors(&floors(&[]), &floors(&[("gone", 90.0)]));
        assert!(removed[0].contains("was not measured"), "{}", removed[0]);
    }

    #[test]
    fn regenerating_reports_the_floors_it_would_lower() {
        // Regenerating is the documented fix for an outgrown floor, so it is the
        // reflex when the gate complains at all. Unguarded, the same keystroke
        // erases a regression.
        let dropping = floors_that_would_drop(
            &floors(&[("a", 90.0), ("b", 99.0)]),
            &floors(&[("a", 95.0), ("b", 99.0)]),
        );
        assert_eq!(dropping, vec!["a: 95% -> 90%".to_string()]);
    }

    #[test]
    fn regenerating_is_silent_about_rises_and_about_crates_with_no_floor_yet() {
        assert!(
            floors_that_would_drop(&floors(&[("a", 99.0)]), &floors(&[("a", 95.0)])).is_empty()
        );
        assert!(floors_that_would_drop(&floors(&[("new", 40.0)]), &floors(&[])).is_empty());
        // A floor whose crate vanished is `compare_floors`'s finding, not a drop.
        assert!(floors_that_would_drop(&floors(&[]), &floors(&[("gone", 90.0)])).is_empty());
    }

    #[test]
    fn floors_are_read_from_the_committed_file() {
        // A `TempDir`, so it goes when the test does. The hand-built directory
        // this replaces was named by pid, created in `/tmp` and never removed:
        // one leaked directory in RAM per run, forever (`A1-257`).
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        let dir = tmp.path();
        std::fs::write(
            dir.join(COVERAGE_FLOORS),
            r#"{"//": "note", "crates": {"aplan-core": 97.0, "xtask": 49.0}}"#,
        )
        .unwrap();
        let read = read_floors(dir).unwrap();
        assert_eq!(read.get("aplan-core"), Some(&97.0));
        assert_eq!(read.len(), 2, "the `//` note must not be read as a crate");

        // A file that parses but has no floors is a broken gate, not an empty one:
        // silently treating it as "no crates" would pass every crate forever.
        std::fs::write(dir.join(COVERAGE_FLOORS), r#"{"crates": []}"#).unwrap();
        assert!(read_floors(dir).is_err());
        std::fs::write(dir.join(COVERAGE_FLOORS), "not json").unwrap();
        assert!(read_floors(dir).is_err());
        // No hand cleanup: `tmp` removes itself when the test ends, which is
        // the point of using it (`A1-257`).
    }

    #[test]
    fn a_committed_floor_is_the_whole_percent_below_the_measurement() {
        // Tolerance for a toolchain that measures the same code a tenth lower;
        // it must never round *up* to a floor the code does not meet.
        assert_eq!(floor_pct((977, 1000)).as_f64(), Some(97.0));
        assert_eq!(floor_pct((999, 1000)).as_f64(), Some(99.0));
        assert_eq!(floor_pct((1, 1)).as_f64(), Some(100.0));
    }

    #[test]
    fn a_percentage_is_truncated_not_rounded() {
        // 91.96 must not be written as a floor of 92.0, which the code does not meet.
        assert_eq!(pct((9196, 10000)).as_f64(), Some(91.9));
        assert_eq!(pct((1, 3)).as_f64(), Some(33.3));
        assert_eq!(pct((1, 1)).as_f64(), Some(100.0));
        // A crate with no measurable lines is not a coverage failure.
        assert_eq!(pct((0, 0)).as_f64(), Some(100.0));
    }

    #[test]
    fn crate_of_maps_a_measured_file_to_its_crate() {
        let root = Path::new("/w");
        assert_eq!(
            crate_of("/w/crates/aplan-core/src/validate.rs", root).as_deref(),
            Some("aplan-core")
        );
        assert_eq!(
            crate_of("/w/xtask/src/main.rs", root).as_deref(),
            Some("xtask")
        );
        // Anything outside the workspace (the registry, the toolchain) is not ours
        // to have a floor for, and counting it would dilute every crate's number.
        assert_eq!(crate_of("/home/u/.cargo/registry/src/x/lib.rs", root), None);
        assert_eq!(crate_of("/w/README.md", root), None);
    }

    #[test]
    fn declared_doc_sources_follow_config_instead_of_a_built_in_list() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("notes")).expect("dirs");
        std::fs::write(dir.path().join("CUSTOM.md"), "custom").expect("custom");
        std::fs::write(dir.path().join("notes/b.md"), "b").expect("b");
        std::fs::write(dir.path().join("notes/a.md"), "a").expect("a");
        // These are names the removed implementation knew by heart. Their
        // presence must not put them in the set unless config says so.
        std::fs::write(dir.path().join("ROADMAP.md"), "not declared").expect("roadmap");
        std::fs::write(dir.path().join("CHANGELOG.md"), "not declared").expect("changelog");
        std::fs::write(
            dir.path().join(".aplan.yml"),
            "docs:\n  paths: [CUSTOM.md, 'notes/*.md']\n",
        )
        .expect("config");

        let got = declared_doc_sources(dir.path()).expect("collecting");
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["CUSTOM.md", "notes/a.md", "notes/b.md"],
            "sorted, globbed, and only what config declares"
        );
        assert_eq!(got[0].1, "custom", "the declared body must be read");
    }

    #[test]
    fn a_declared_source_that_matches_nothing_is_a_refusal() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(
            dir.path().join(".aplan.yml"),
            "docs:\n  paths: ['missing/*.md']\n",
        )
        .expect("config");
        let error = format!(
            "{:#}",
            declared_doc_sources(dir.path()).expect_err("a declaration over nothing must not pass")
        );
        assert!(error.contains("matched no files"), "{error}");
    }

    /// The plan this repository's own prose is checked against, in miniature.
    /// The scanning itself is `aplan-interop`'s and tested there; what these pin
    /// is that this gate hands it the plan's ids and gets this repository's own
    /// shapes back.
    fn plan_ids() -> BTreeSet<String> {
        [
            "A1-019", "A1-037", "A1-042", "M-v0.7", "M-v0.8", "M-v0.9", "M-v0.10",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
    }

    /// The two ways this gate refuses rather than reports (`A1-148`'s audit).
    #[test]
    fn a_plan_it_cannot_read_is_a_refusal_not_a_verdict() {
        let good = plan_ids();
        assert!(plan_unusable(0, &good).is_none(), "a plan it can read");

        let broken = plan_unusable(1, &good).expect("a file that failed to load");
        assert!(broken.contains("aplan check"), "{broken}");

        // Nothing loaded at all: no prefixes, so every document would pass.
        let empty = plan_unusable(0, &BTreeSet::new()).expect("nothing to recognise");
        assert!(empty.contains("no id prefix"), "{empty}");

        // Ids that teach nothing are the same hole with files present.
        let ulids: BTreeSet<String> = ["A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD".to_string()]
            .into_iter()
            .collect();
        assert!(plan_unusable(0, &ulids).is_some(), "ULIDs teach no prefix");
    }

    /// The gate this repository runs on itself asks the shipped command's
    /// question, not a cheaper one that resembles it.
    ///
    /// It stopped doing so once already. Check 1 used to ask "which mentions are
    /// not in the plan", which is `dangling`'s answer minus the near-miss pass
    /// `A1-164` added, so `aplan check-docs pre-public adoption evidence` reported a
    /// reference that `cargo xtask check-docs` called clean -- the shipped copy
    /// finding what the tested copy could not. Subtracting from a mention list
    /// is what makes that possible, so the assertion is that this gate reports
    /// a finding no subtraction can produce.
    #[test]
    fn the_repositorys_own_gate_sees_what_the_shipped_one_sees() {
        let plan = plan_ids();
        let text = "Tracked as `A2-042`.";
        // The predicate this gate used to use, spelled out here so the test can
        // show it answering "clean" on a document the shipped command reports.
        let vocabulary = aplan_interop::docs::Vocabulary::of(&plan);
        let mentioned_but_unknown: Vec<String> = aplan_interop::docs::references(text, &vocabulary)
            .into_iter()
            .map(|r| r.id)
            .filter(|id| !plan.contains(id))
            .collect();
        assert!(
            mentioned_but_unknown.is_empty(),
            "a near-miss prefix is not a mention, which is the whole trap"
        );
        assert_eq!(
            undefined_ids(text, &plan),
            vec!["A2-042".to_string()],
            "and it is still a reference naming no task"
        );
    }

    fn cut_labels(of: &[&str]) -> BTreeSet<String> {
        of.iter().map(|s| (*s).to_string()).collect()
    }

    /// Which `###` heading is a cut, in every label ADR-0013 allows (`A1-172`).
    ///
    /// The rule used to be `starts_with('v') && contains('.')`, so this
    /// repository's own `### public, Going public` section was invisible to the
    /// gate that exists to stop an unplanned cut passing in silence.
    #[test]
    fn a_cut_section_is_any_label_the_plans_own_vocabulary_allows() {
        let labels = cut_labels(&["v0.7", "v0.8", "public"]);
        let doc = "## 3. Cuts\n\n\
                   ### v0.7, Advanced integrity\n\nbody one\n\n\
                   ### Not a cut\n\nignored\n\n\
                   ### public, Going public\n\nbody two\n\n\
                   ### v0.9, Unplanned\n\nbody three\n";
        let cuts = cut_sections(doc, &labels);
        assert_eq!(
            cuts.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(),
            // `public` because the plan has it; `v0.9` because the plan's own
            // labels teach `v` plus a number, and an unplanned cut is exactly
            // what this half of the gate is for.
            vec!["v0.7", "public", "v0.9"]
        );
        assert!(cuts[0].1.contains("body one"));
        assert!(
            !cuts[0].1.contains("ignored"),
            "a non-cut heading ends the section"
        );
    }

    /// A live cut the narrative never describes is reported; a finished one is
    /// not (`A1-181`).
    ///
    /// Against a fixture rather than the live plan, because the live plan is
    /// the case that must *pass*: every one of this repository's three live
    /// cuts has a section, so run against it this proves only that nothing is
    /// wrong today.
    #[test]
    fn a_live_cut_with_no_section_is_reported_and_a_finished_one_is_not() {
        let described: BTreeSet<&str> = ["v0.7", "v0.8", "public"].into_iter().collect();

        // The live cut nobody wrote a section for.
        assert_eq!(
            undescribed_cuts(["M-v0.7", "M-v0.95"].into_iter(), &described),
            vec!["M-v0.95"]
        );

        // A finished cut never reaches this: the caller filters on status, and
        // `M-v0.1` -- done, two closed tasks, older than the document it is
        // missing from -- is the case the silence was built for.
        assert!(undescribed_cuts(std::iter::empty(), &described).is_empty());

        // Every live cut described is silence.
        assert!(
            undescribed_cuts(["M-v0.7", "M-v0.8", "M-public"].into_iter(), &described).is_empty()
        );

        // A milestone whose id teaches no label cannot be matched to a section,
        // so it is reported rather than skipped: silence there would be the
        // gate quietly not covering a shape.
        assert_eq!(
            undescribed_cuts(["Q-1"].into_iter(), &described),
            vec!["Q-1"]
        );
    }

    /// A cut is a task *tagged* `milestone`, not one merely named like one
    /// (ADR-0008, and `A1-172`'s audit).
    ///
    /// Nothing in this repository's own plan distinguishes the two -- every id
    /// starting with `M-` is a milestone -- so without this the filter is an
    /// intention no test holds to.
    #[test]
    fn only_a_tagged_milestone_teaches_a_cut_label() {
        let milestone = vec!["milestone".to_string()];
        let ordinary: Vec<String> = vec!["docs".to_string()];
        let labels = cut_labels_of(
            [
                ("M-v0.8", milestone.as_slice()),
                ("M-notes", ordinary.as_slice()),
                ("A1-042", milestone.as_slice()),
            ]
            .into_iter(),
        );
        assert_eq!(labels, cut_labels(&["v0.8"]), "{labels:?}");
    }

    /// A checkout with CRLF line endings is the same document (`A1-172`,
    /// second audit).
    ///
    /// The label is the heading's first word, and a heading with no comma after
    /// it ends in whatever the file's line ending is. If a carriage return rode
    /// along, `v0.8` would not equal `v0.8` and this gate would go quiet on a
    /// Windows checkout without failing -- the worst way for a gate to stop
    /// working.
    ///
    /// It does not, and the reason is one level down rather than here:
    /// `anchor::sections` splits with `str::lines`, which drops a trailing
    /// `\r`. So this asserts the property rather than the mechanism, which is
    /// the right thing to hold when the mechanism is somebody else's -- mutating
    /// this file cannot break it, and changing how sections are read can.
    #[test]
    fn a_crlf_checkout_reads_the_same_cuts() {
        let labels = cut_labels(&["v0.8", "public"]);
        for doc in [
            "### v0.8\r\n\r\nbody\r\n",
            "### v0.8, Adoption\r\n\r\nbody\r\n",
            "### **v0.8**\r\n\r\nbody\r\n",
        ] {
            let cuts = cut_sections(doc, &labels);
            assert_eq!(
                cuts.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(),
                vec!["v0.8"],
                "{doc:?}"
            );
        }
        assert_eq!(cut_sections("### public\r\n\r\nbody\r\n", &labels).len(), 1);
    }

    /// The boundary the vocabulary rule buys: a plan that teaches no labels has
    /// no cuts to recognise, and this half says nothing rather than guessing
    /// (`A1-172`, audit).
    ///
    /// Pinned as a decision, not left as an accident. The pattern this replaced
    /// would have called `### v0.9` a cut in a plan with no milestones at all;
    /// with nothing to learn from, the honest answer is silence, which is the
    /// same default the shipped command takes for a milestone whose section a
    /// document does not have.
    #[test]
    fn a_plan_that_teaches_no_labels_recognises_no_cuts() {
        let none = BTreeSet::new();
        assert!(cut_sections("### v0.9, Unplanned\n\nbody\n", &none).is_empty());
        // And one milestone is enough to teach the shape of the rest.
        let one = cut_labels(&["v0.8"]);
        assert_eq!(cut_sections("### v0.9, Unplanned\n\nbody\n", &one).len(), 1);
    }

    /// And a heading that is not a cut is still not one, which is the property
    /// the narrow rule bought and the reason it was written that way.
    #[test]
    fn a_heading_that_is_not_a_cut_is_still_not_one() {
        let labels = cut_labels(&["v0.8", "public"]);
        for heading in [
            "### How to read this",
            "### verify, and what it proves",
            "### v, a bare prefix",
            "### publication, not the cut",
            "### 3. Numbered but not a label",
            // Case-sensitive, because the shipped command compares a heading's
            // first word to a milestone id with `==`. A rule that differed here
            // would be the second reader this borrows from the first to avoid.
            "### V0.8, Adoption",
            "### PUBLIC, Going public",
        ] {
            let doc = format!("{heading}\n\nbody\n");
            assert!(
                cut_sections(&doc, &labels).is_empty(),
                "{heading} is not a cut"
            );
        }
    }

    /// The label is read the way the shipped command reads it, decoration and
    /// all, and a heading inside a fence is not a heading (`A1-172`).
    #[test]
    fn a_decorated_heading_is_a_cut_and_a_fenced_one_is_not() {
        let labels = cut_labels(&["v0.8"]);
        for heading in [
            "### **v0.8**, Adoption",
            "### [v0.8](#x), Adoption",
            "### `v0.8`: Adoption",
        ] {
            let doc = format!("{heading}\n\nbody\n");
            let cuts = cut_sections(&doc, &labels);
            assert_eq!(cuts.len(), 1, "{heading}");
            assert_eq!(cuts[0].0, "v0.8", "{heading}");
        }
        // The scan this replaced read every line, so an example in a fenced
        // block was a cut this repository had promised.
        let fenced = "## Cuts\n\n```\n### v0.9, An example\n```\n\nprose\n";
        assert!(
            cut_sections(fenced, &labels).is_empty(),
            "a fence is not prose"
        );

        // A cut is a `###`, and only that. The heading above them names the
        // section that holds them, and a heading below one is a detail of a cut
        // rather than a second cut -- so neither is one, however cut-like the
        // words are.
        let nested = "## v0.8, the section that holds the cuts\n\n\
                      ### v0.8, Adoption\n\nbody\n\n\
                      #### v0.9, a detail\n\nmore\n";
        let cuts = cut_sections(nested, &labels);
        assert_eq!(
            cuts.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(),
            vec!["v0.8"],
            "one cut, at one level: {cuts:?}"
        );
    }

    /// A config key this build accepts and the changelog never *announces* is
    /// reported.
    ///
    /// The sharper half of the rule-code check beside it: an unknown key is
    /// *refused*, so adopting one your binary predates costs every command that
    /// reads config rather than one gate (`A1-169`). Nested keys are excluded on
    /// purpose -- they arrive with the key above them.
    ///
    /// The first assertion is the one the audit added. A bag of backticked names
    /// is what a `contains` predicate accepts and an announcement is not, so a
    /// fixture that is only a list of keys has to *fail*: the shipped check used
    /// `changelog.contains` and this test passed, because the test was written
    /// against the same weakness.
    #[test]
    fn a_config_key_absent_from_the_changelog_is_reported_by_name() {
        let keys = aplan_config::accepted_keys();
        // Every key named, but three paragraphs into somebody else's entry. That
        // is a mention, and the predicate this check shipped with accepted it.
        let buried = format!(
            "- **Something else** (`APLAN001`). The opening paragraph.\n\n  Later, in passing: {}\n",
            keys.iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(
            unannounced_config_keys(&buried).is_some(),
            "a key named inside another entry is mentioned, not announced"
        );

        // The shape that does announce: one top-level entry per key.
        let entries = keys
            .iter()
            .map(|k| format!("- **A change** (`{k}`). What it does.\n"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            unannounced_config_keys(&entries),
            None,
            "an entry per key announces every key"
        );

        let top = keys
            .iter()
            .find(|k| !k.contains('.'))
            .expect("at least one top-level key");
        let missing_one = entries.replace(&format!("(`{top}`)"), "(nothing)");
        let problem = unannounced_config_keys(&missing_one).expect("one key is unannounced");
        assert!(
            problem.contains(top.as_str()),
            "the key must be named: {problem}"
        );

        // A nested key going unmentioned is not a finding, or the changelog would
        // have to restate the schema.
        let nested = keys
            .iter()
            .find(|k| k.contains('.'))
            .expect("at least one nested key");
        assert_eq!(
            unannounced_config_keys(&entries.replace(&format!("(`{nested}`)"), "(nothing)")),
            None,
            "`{nested}` arrives with the key above it"
        );
    }

    #[test]
    fn a_rule_absent_from_the_changelog_is_reported_by_name() {
        // `A1-067`. `APLAN024` and `APLAN025` each shipped closed, green and
        // unmentioned, because nothing read `CHANGELOG.md`. The failure has to
        // name the codes, or the fix begins with a search.
        let every = aplan_core::Rule::ALL
            .iter()
            .map(|r| r.code())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            unannounced_rules(&every),
            None,
            "a changelog naming every rule is clean"
        );

        let last = aplan_core::Rule::ALL
            .last()
            .expect("the registry is not empty")
            .code();
        let missing_one = every.replace(last, "");
        let problem = unannounced_rules(&missing_one).expect("one rule is unannounced");
        assert!(problem.contains(last), "the code must be named: {problem}");
        assert!(
            !problem.contains("APLAN001"),
            "and only the missing one: {problem}"
        );

        // The empty file is the state this gate was written for, and it must name
        // every rule rather than stopping at the first.
        let problem = unannounced_rules("").expect("all rules are unannounced");
        for rule in aplan_core::Rule::ALL {
            assert!(
                problem.contains(rule.code()),
                "{} missing: {problem}",
                rule.code()
            );
        }
    }

    #[test]
    fn a_mention_inside_another_entry_is_not_an_announcement() {
        // `A1-092` deleted `contains` from this file as useless, and `A1-067`
        // shipped it again a few hundred lines away. A rule named only in passing
        // inside somebody else's entry counted as announced, which is how a new
        // rule could reach a release with nothing written about it.
        let mentioned = "- **Something else** (`A1-001`). Body.\n\nThis extends \
                         APLAN012 and behaves like APLAN003.\n";
        let announced = announced_rules(mentioned);
        assert!(
            !announced.contains("APLAN012") && !announced.contains("APLAN003"),
            "a mention in an entry's later prose is not its subject: {announced:?}"
        );

        // The subject of its own entry is.
        let owned = "- **`APLAN012` binds a commit to the task's paths** (`A1-004`).\n";
        assert!(announced_rules(owned).contains("APLAN012"));
    }

    #[test]
    fn a_range_announces_everything_it_covers() {
        // The founding set is described as `APLAN001`-`APLAN010`, which is the
        // honest way to write it. Under `contains` those ten passed only because
        // they happened to be mentioned elsewhere, so deleting an unrelated
        // sentence would have failed the gate for a rule that *was* announced.
        let ranged = "- Eleven invariants, `APLAN001`-`APLAN010` and `APLAN020`.\n";
        let announced = announced_rules(ranged);
        for n in 1..=10 {
            assert!(
                announced.contains(&format!("APLAN{n:03}")),
                "the range must cover APLAN{n:03}: {announced:?}"
            );
        }
        assert!(
            announced.contains("APLAN020"),
            "and the one named beside it"
        );
        assert!(
            !announced.contains("APLAN011"),
            "but nothing past the range: {announced:?}"
        );
    }

    #[test]
    fn an_unplanned_cut_declares_that_it_has_no_milestone() {
        assert!(matches!(
            declared_tasks("**Tasks:** none yet (no milestone)\n"),
            Some(Declaration::None)
        ));
        // Silence is not a declaration: that is how a promise goes unnoticed.
        assert!(declared_tasks("Prose with no declaration.\n").is_none());
        assert!(matches!(
            declared_tasks("**Tasks:** `A1-003`, `A1-019`\n"),
            Some(Declaration::Tasks)
        ));
    }

    #[test]
    fn a_repository_can_decline_the_style_and_is_told_it_did() {
        // `A1-052` asked for adopt, narrow, or decline, and only two of the three
        // worked: no spelling exempted everything (audit).
        for everything in ["**", "."] {
            assert!(is_exempt("README.md", &[everything.to_string()]));
            assert!(is_exempt(
                "crates/aplan/src/lib.rs",
                &[everything.to_string()]
            ));
        }
        // And the run says so rather than saying "clean", because a gate that
        // looked at nothing is not a gate that passed.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join("a.md"), format!("x{}y\n", '\u{2014}')).expect("write");
        let scan = lint_files(dir.path(), &["a.md".to_string()], &["**".to_string()]);
        assert_eq!(scan.scanned, 0);
        assert_eq!(scan.exempted, 1);
        assert_eq!(scan.violations, 0, "an exempt file is not read");
    }

    #[test]
    fn a_run_that_looked_at_nothing_does_not_report_itself_clean() {
        // "clean" after scanning zero files is the vacuity this project keeps
        // finding in other people's gates, so the wording is a decision and is
        // tested as one.
        let declined = LintScan {
            scanned: 0,
            exempted: 362,
            ..LintScan::default()
        };
        let line = clean_summary(&declined);
        assert!(line.contains("not applied"), "{line}");
        assert!(
            !line.contains("clean"),
            "it must not read as a pass: {line}"
        );

        let normal = LintScan {
            scanned: 357,
            exempted: 4,
            ..LintScan::default()
        };
        assert!(clean_summary(&normal).contains("clean (357 files scanned, 4 exempt"));

        let no_config = LintScan {
            scanned: 361,
            ..LintScan::default()
        };
        assert_eq!(
            clean_summary(&no_config),
            "style: clean (361 files scanned)",
            "with no exemptions the line is what it always was"
        );
    }

    #[test]
    fn a_blank_exemption_is_a_typo_rather_than_a_licence() {
        // Making a stray line mean "everything" would turn a typo into a silently
        // disabled gate. Declining is spelled out loud.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::write(dir.path().join(STYLE_CONFIG), "exempt:\n  - \"\"\n").expect("write");
        let err = style_exemptions(dir.path()).expect_err("blank is refused");
        assert!(err.to_string().contains("blank entry"), "{err}");
        assert!(
            err.to_string().contains("**"),
            "it names the way to decline: {err}"
        );
    }

    #[test]
    fn this_repositorys_own_style_config_exempts_the_pilot_logs() {
        // The wiring between `lint` and the config file has no test of its own,
        // and this is the part of it most likely to rot: the file being renamed,
        // reshaped, or losing the entry. If it does, an adopter's log silently
        // comes back under our typography (`A1-052`).
        let root = workspace_root().expect("in the workspace");
        let exemptions = style_exemptions(&root).expect("the committed config parses");
        assert!(
            is_exempt("docs/pilots/p00025.md", &exemptions),
            "a pilot log must stay exempt: {exemptions:?}"
        );
        assert!(
            !is_exempt("README.md", &exemptions),
            "and the style still applies to this repository's own prose"
        );
    }

    #[test]
    fn the_lint_pass_actually_skips_an_exempt_file() {
        // The predicate was tested and the gate that calls it was not, so
        // disabling the exemption inside `lint` survived its mutation. This tests
        // the pass (`A1-052`).
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("docs/pilots")).expect("mkdir");
        let tell = format!("a sentence {} with an em dash\n", '\u{2014}');
        std::fs::write(dir.path().join("docs/pilots/x.md"), &tell).expect("write");
        std::fs::write(dir.path().join("README.md"), &tell).expect("write");
        let files = vec!["docs/pilots/x.md".to_string(), "README.md".to_string()];

        // Without an exemption both are violations.
        let bare = lint_files(dir.path(), &files, &[]);
        assert_eq!(bare.violations, 2, "{:?}", bare.reports);
        assert_eq!(bare.exempted, 0);
        assert_eq!(bare.scanned, 2);

        // With one, the exempt file is not read at all and the other still fails.
        let scoped = lint_files(dir.path(), &files, &["docs/pilots/".to_string()]);
        assert_eq!(scoped.violations, 1, "{:?}", scoped.reports);
        assert_eq!(scoped.exempted, 1);
        assert_eq!(
            scoped.scanned, 1,
            "an exempt file is not counted as scanned"
        );
        assert!(
            scoped.reports.iter().all(|r| r.starts_with("README.md")),
            "{:?}",
            scoped.reports
        );
    }

    #[test]
    fn the_exemption_list_is_read_from_the_repository_or_absent() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        // No file at all is this repository's behaviour before there was one:
        // the style applies everywhere, and nothing relaxes silently.
        assert!(
            style_exemptions(dir.path())
                .expect("absent is not an error")
                .is_empty()
        );

        std::fs::write(
            dir.path().join(STYLE_CONFIG),
            "exempt:\n  - docs/pilots/\n  - vendor/\n",
        )
        .expect("write");
        assert_eq!(
            style_exemptions(dir.path()).expect("parses"),
            vec!["docs/pilots/".to_string(), "vendor/".to_string()]
        );

        // A file with no `exempt` key is not a failure: it is a file that exempts
        // nothing, which is a thing a repository may want to say explicitly.
        std::fs::write(dir.path().join(STYLE_CONFIG), "# nothing here\n").expect("write");
        assert!(
            style_exemptions(dir.path())
                .expect("no key is fine")
                .is_empty()
        );

        // But a list of the wrong shape is an error rather than a silent empty
        // list, because a config the tool cannot act on must not read as one it
        // agreed with.
        std::fs::write(dir.path().join(STYLE_CONFIG), "exempt:\n  - 42\n").expect("write");
        let err = style_exemptions(dir.path()).expect_err("a number is not a path");
        assert!(err.to_string().contains("path prefixes"), "{err}");

        std::fs::write(dir.path().join(STYLE_CONFIG), "exempt: [\n").expect("write");
        assert!(
            style_exemptions(dir.path()).is_err(),
            "unparseable YAML is an error, not an empty list"
        );
    }

    #[test]
    fn the_style_applies_everywhere_unless_a_prefix_says_otherwise() {
        // Absent config, or an empty list, is today's behaviour: nothing relaxes
        // silently (`A1-052`).
        assert!(!is_exempt("README.md", &[]));
        let exempt = vec!["docs/pilots/".to_string()];
        assert!(is_exempt("docs/pilots/p00025.md", &exempt));
        assert!(is_exempt("docs/pilots/README.md", &exempt));
        // A prefix is a path prefix, not a string prefix: a sibling directory
        // whose name merely starts the same way is not exempt.
        assert!(!is_exempt("docs/pilots-archive/old.md", &exempt));
        assert!(!is_exempt("docs/adr/0001-x.md", &exempt));
        assert!(!is_exempt("README.md", &exempt));
        // Written with or without the trailing slash, and with a glob tail, since
        // all three are what a person types.
        for spelling in ["docs/pilots", "docs/pilots/", "docs/pilots/**"] {
            assert!(
                is_exempt("docs/pilots/p00025.md", &[spelling.to_string()]),
                "spelling: {spelling}"
            );
        }
    }

    #[test]
    fn flags_each_typographic_tell() {
        // Built from escapes so this test source stays lint-clean.
        for (c, _) in FORBIDDEN {
            let s = format!("a{c}b");
            assert_eq!(
                scan_text(&s),
                vec![(1, 2, *c)],
                "should flag U+{:04X}",
                *c as u32
            );
        }
    }

    #[test]
    fn allows_functional_unicode_and_plain_ascii() {
        // Arrow (U+2192), section sign (U+00A7), accented letters, emoji: all fine.
        assert!(scan_text("A \u{2192} B; see \u{00A7}5; caf\u{00E9}; \u{1F30D}").is_empty());
        assert!(scan_text("plain ASCII: a, b; c (d). 2013-2014").is_empty());
    }

    #[test]
    fn reports_one_based_line_and_column() {
        let em = '\u{2014}';
        let hits = scan_text(&format!("line one\nx{em}y"));
        assert_eq!(hits, vec![(2, 2, em)]);
    }

    /// A decision record directory, built defect by defect.
    ///
    /// Against a fixture rather than the live `docs/adr/`, for the reason the
    /// disposition gate is: the three defects `A1-222` found are now fixed, and a
    /// repository that has been repaired cannot demonstrate the gate that catches
    /// it. Each test below puts one back.
    fn adr_dir(files: &[(&str, &str)]) -> (tempfile::TempDir, Vec<String>) {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        for (rel, body) in files {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&path, body).expect("write");
        }
        let names = files.iter().map(|(rel, _)| (*rel).to_string()).collect();
        (dir, names)
    }

    const GOOD: &[(&str, &str)] = &[
        ("docs/adr/0001-a-thing.md", "# ADR-0001: A thing\n\nBody.\n"),
        ("docs/adr/0002-another.md", "# ADR-0002: Another\n\nBody.\n"),
    ];

    fn scan_of(extra: &[(&str, &str)]) -> AdrScan {
        let mut files: Vec<(&str, &str)> = GOOD.to_vec();
        files.extend_from_slice(extra);
        let (dir, names) = adr_dir(&files);
        adr_scan(dir.path(), &names)
    }

    /// The baseline, without which every test below passes by accident: a
    /// well-formed directory and a citation that resolves produce no problems at
    /// all.
    #[test]
    fn a_record_that_is_filed_once_and_cited_correctly_is_silent() {
        let scan = scan_of(&[(
            "HLD.md",
            "Decided in ADR-0002, and linked as\n\
                               docs/adr/0002-another.md here.\n",
        )]);
        assert!(
            scan.problems.is_empty(),
            "a clean record set reported {:?}",
            scan.problems
        );
        assert_eq!(scan.records, 2);
        assert_eq!(scan.citations, 1, "the path form is not a second citation");
    }

    /// Defect 1, the one `A1-222` was filed for: two files both declaring
    /// `ADR-0020`. Both are named, because "there is a duplicate" without saying
    /// which files is a report nobody can act on.
    #[test]
    fn two_files_filed_under_one_number_are_caught_and_both_are_named() {
        let scan = scan_of(&[(
            "docs/adr/0002-a-second-answer.md",
            "# ADR-0002: A second answer\n",
        )]);
        let report = scan.problems.join("\n");
        assert!(report.contains("ADR-0002 is filed 2 times"), "{report}");
        assert!(report.contains("0002-another.md"), "{report}");
        assert!(report.contains("0002-a-second-answer.md"), "{report}");
    }

    /// Defect 2: a filename and a heading that disagree. The file resolves by
    /// number, so nothing else notices; a reader following one number lands on a
    /// record announcing the other.
    #[test]
    fn a_filename_that_disagrees_with_its_heading_is_caught() {
        let scan = scan_of(&[("docs/adr/0003-mislabelled.md", "# ADR-0009: Mislabelled\n")]);
        let report = scan.problems.join("\n");
        assert!(
            report.contains("filename says ADR-0003") && report.contains("heading says ADR-0009"),
            "{report}"
        );
    }

    /// And the two halves of that, each alone: a file with no parseable number,
    /// and one whose number lives only in its name.
    #[test]
    fn a_record_that_declares_its_number_once_or_not_at_all_is_caught() {
        let unnamed = scan_of(&[("docs/adr/notes.md", "# ADR-0003: Notes\n")]);
        assert!(
            unnamed
                .problems
                .iter()
                .any(|p| p.contains("does not begin `NNNN-`")),
            "{:?}",
            unnamed.problems
        );
        let unheaded = scan_of(&[("docs/adr/0003-silent.md", "Some prose, no heading.\n")]);
        assert!(
            unheaded
                .problems
                .iter()
                .any(|p| p.contains("no `# ADR-0003: ` heading")),
            "{:?}",
            unheaded.problems
        );
    }

    /// Defect 3: a citation naming a record that does not exist. This is the one
    /// `crates/aplan-interop/src/docs.rs` carried, where the cited ADR was real
    /// but the thing attributed to it was not.
    #[test]
    fn a_citation_naming_no_record_is_caught_and_a_resolving_one_is_not() {
        let scan = scan_of(&[("HLD.md", "As ADR-0001 says, and as ADR-0042 does not.\n")]);
        let report = scan.problems.join("\n");
        assert!(report.contains("cites ADR-0042"), "{report}");
        assert!(
            !report.contains("ADR-0001"),
            "a live citation was reported: {report}"
        );
    }

    /// The same for the form a renumbering actually breaks. `A1-222` moved two
    /// files, and a stale path keeps rendering as a link that goes nowhere --
    /// which no number check can see, because the number is still right.
    #[test]
    fn a_path_to_a_renamed_record_is_caught_even_though_its_number_resolves() {
        let scan = scan_of(&[(
            "HLD.md",
            "Recorded at docs/adr/0001-a-thing-renamed.md (ADR-0001).\n",
        )]);
        let report = scan.problems.join("\n");
        assert!(report.contains("0001-a-thing-renamed.md"), "{report}");
        assert!(report.contains("not a file"), "{report}");
    }

    /// A directory with nothing in it fails rather than passing green, which is
    /// the direction `check_docs` already refuses to fail in: with no records,
    /// every citation is unresolvable and the gate would either accuse the whole
    /// repository or, reading it the other way, approve it.
    #[test]
    fn a_decision_record_directory_that_is_empty_is_a_refusal_not_a_pass() {
        let (dir, names) = adr_dir(&[("HLD.md", "Nothing here.\n")]);
        let scan = adr_scan(dir.path(), &names);
        assert!(
            scan.problems
                .iter()
                .any(|p| p.contains("holds no decision records")),
            "{:?}",
            scan.problems
        );
    }

    /// Where a citation counts, and where it is data.
    ///
    /// `consumer_facing.rs` builds twelve `ADR-1nnn` codes *because* no such
    /// record exists, and an `aplan-evidence` fixture commits a file under
    /// `docs/adr/` for a glob to bind to. Reading either as a citation makes the
    /// gate report on its own test data -- which it did, twenty times, before
    /// this rule (`A1-222`).
    #[test]
    fn rust_is_read_at_its_comments_and_markdown_outside_its_fences() {
        let code = scan_of(&[(
            "crates/x/src/lib.rs",
            "//! Recorded as ADR-0001.\n\
             fn f() { let _ = \"ADR-0042\"; let _ = \"docs/adr/0042-nope.md\"; }\n",
        )]);
        assert!(
            code.problems.is_empty(),
            "a fixture string was read as a citation: {:?}",
            code.problems
        );
        assert_eq!(code.citations, 1, "the doc comment is the citation");

        let quoted = scan_of(&[(
            "docs/pilots/p.md",
            "Their output:\n\n```console\n$ cargo xtask check-docs\ndocs/adr/0000-template.md:11: no\nADR-0042 missing\n```\n\nOurs is ADR-0001.\n",
        )]);
        assert!(
            quoted.problems.is_empty(),
            "a consumer's console was read as this repository's prose: {:?}",
            quoted.problems
        );
    }

    /// A pilot log is read, not skipped. Excluding those files to keep one
    /// foreign citation quiet would drop the forty-two citations of *our*
    /// records they carry, so the boundary is per citation: prose that hands a
    /// number to somebody else says so, and the gate says which ones it took that
    /// way.
    #[test]
    fn a_citation_the_prose_attributes_elsewhere_is_reported_rather_than_dropped() {
        let scan = scan_of(&[(
            "docs/pilots/p00025.md",
            "`p00025`'s ADR-0042 requires a banner, unlike ADR-0001.\n",
        )]);
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.foreign.len(), 1, "{:?}", scan.foreign);
        assert!(scan.foreign[0].contains("ADR-0042"), "{:?}", scan.foreign);
        assert!(
            scan.aside()
                .iter()
                .any(|l| l.contains("attributed to another repository")),
            "the skip is invisible: {:?}",
            scan.aside()
        );
    }

    /// **Attribution is consulted only after resolution fails.** This repository
    /// writes "this repository's ADR-0009" to distinguish its records from a
    /// pilot's, so reading the possessive as "not ours" stopped resolving five
    /// real citations -- silently, since an unresolved citation is never checked
    /// against anything (`A1-222`, audit).
    #[test]
    fn a_possessive_does_not_stop_a_citation_of_ours_from_resolving() {
        let scan = scan_of(&[(
            "HLD.md",
            "What it implements is this repository's ADR-0001.\n",
        )]);
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert!(
            scan.foreign.is_empty(),
            "a citation that resolves was filed as somebody else's: {:?}",
            scan.foreign
        );
        assert_eq!(scan.citations, 1);
    }

    /// A reserved number resolves, and is named on every run. `HLD.md` section 19
    /// holds six decisions by the number they will take, and a forward reference
    /// to one is not a false claim -- but it is not a record either, so it stays
    /// visible rather than passing as though it were.
    #[test]
    fn a_reserved_number_resolves_and_says_so_while_an_unreserved_one_does_not() {
        let reserved = scan_of(&[
            (
                "HLD.md",
                "## 19. Decisions that need an ADR\n\n1. **ADR-0042: a thing.**\n",
            ),
            ("ROADMAP.md", "Planned as ADR-0042.\n"),
        ]);
        assert!(reserved.problems.is_empty(), "{:?}", reserved.problems);
        assert_eq!(reserved.reserved.len(), 2, "{:?}", reserved.reserved);
        assert!(
            reserved
                .aside()
                .iter()
                .any(|l| l.contains("reserved in HLD.md")),
            "{:?}",
            reserved.aside()
        );

        let unreserved = scan_of(&[
            (
                "HLD.md",
                "## 19. Decisions that need an ADR\n\n1. **ADR-0043: a thing.**\n",
            ),
            ("ROADMAP.md", "Planned as ADR-0042.\n"),
        ]);
        assert!(
            unreserved
                .problems
                .iter()
                .any(|p| p.contains("cites ADR-0042")),
            "{:?}",
            unreserved.problems
        );
    }

    /// A reservation is retired when its record lands. Two sat on the list after
    /// shipping, telling a reader to write `ADR-0007` and `ADR-0008` -- both of
    /// which had been written (`A1-222`).
    #[test]
    fn a_reservation_whose_record_exists_is_caught() {
        let scan = scan_of(&[(
            "HLD.md",
            "## 19. Decisions that need an ADR\n\n1. **ADR-0002: already written.**\n",
        )]);
        let report = scan.problems.join("\n");
        assert!(report.contains("reserves ADR-0002"), "{report}");
        assert!(report.contains("retire the reservation"), "{report}");
    }

    /// Prose around the list is prose. The paragraph that retires an entry names
    /// the numbers it retires, and reading those as reservations would make a
    /// retirement impossible to write down.
    #[test]
    fn only_a_numbered_entry_reserves_a_number() {
        let scan = scan_of(&[
            (
                "HLD.md",
                "## 19. Decisions that need an ADR\n\n1. **ADR-0043: a thing.**\n\n\
             Two more have since been written: ADR-0001 and ADR-0002.\n\n\
             ## 20. Open questions\n\n1. **ADR-0044: not a reservation.**\n",
            ),
            ("ROADMAP.md", "Planned as ADR-0044.\n"),
        ]);
        let report = scan.problems.join("\n");
        assert!(
            !report.contains("reserves ADR-0001") && !report.contains("reserves ADR-0002"),
            "a retirement paragraph was read as a reservation: {report}"
        );
        assert!(
            report.contains("cites ADR-0044"),
            "a number outside the section reserved anyway: {report}"
        );
    }

    /// Four digits, exactly. `ADR-000` in a fixture and `ADR-00xx` in a task file
    /// are not citations, and `MADR-0001` is not one either.
    #[test]
    fn only_a_four_digit_number_after_a_word_boundary_is_a_citation() {
        let scan = scan_of(&[(
            "HLD.md",
            "Not ADR-000, not ADR-00xx, not MADR-0042, not ADR-00421. But ADR-0001 is.\n",
        )]);
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.citations, 1, "loose reading found extra citations");
    }

    /// A hold has to do something, and each of the three ways it can rot has to
    /// fail. The first version carried the list from run to run and consulted it
    /// nowhere, so a hold changed no count and its reason was never read.
    #[test]
    fn a_hold_excuses_one_member_and_rots_three_ways() {
        let flag = cli_surface::Member::Flag {
            command: "add".into(),
            long: "owner".into(),
        };
        let other = cli_surface::Member::Flag {
            command: "add".into(),
            long: "out".into(),
        };
        let audit = cli_surface::Audit {
            surface: [flag.clone(), other.clone()].into_iter().collect(),
            exercised: [other.clone()].into_iter().collect(),
            residue: [flag.clone()].into_iter().collect(),
            invocations: 1,
            recorded: 1,
            rejected: Vec::new(),
            asked_for_text: 0,
        };
        let reason = "no test can invoke it without a second checkout, which is A1-999";

        // It excuses the member it names, and only that one.
        let (left, problems) = cli_surface_holds(
            &audit,
            &[Held {
                member: flag.to_string(),
                why: reason.into(),
            }],
        );
        assert_eq!(left, 0, "the hold did not excuse anything");
        assert!(problems.is_empty(), "{problems:?}");

        // A reason nobody can read is not a reason.
        let (_, problems) = cli_surface_holds(
            &audit,
            &[Held {
                member: flag.to_string(),
                why: "later".into(),
            }],
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("no reason worth reading")),
            "{problems:?}"
        );

        // A hold on something the suite now covers is a note telling the next
        // reader not to bother.
        let (_, problems) = cli_surface_holds(
            &audit,
            &[Held {
                member: other.to_string(),
                why: reason.into(),
            }],
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("something exercises it")),
            "{problems:?}"
        );

        // And a hold on a member the surface no longer has.
        let (_, problems) = cli_surface_holds(
            &audit,
            &[Held {
                member: "aplan add --gone".into(),
                why: reason.into(),
            }],
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("not in the surface any more")),
            "{problems:?}"
        );
    }

    /// The report names what was not covered, rather than only counting it: a
    /// number nobody can act on is the shape of gate that gets turned off.
    #[test]
    fn the_surface_report_names_every_command_with_a_gap() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let cmd = dir.path().join("crates/aplan/tests/cmd");
        std::fs::create_dir_all(&cmd).expect("mkdir");
        std::fs::write(
            cmd.join("a.trycmd"),
            "$ aplan summary\n$ aplan nope --wat\n$ aplan add --help\n",
        )
        .expect("case");
        let audit = cli_surface::audit(dir.path(), None).expect("audited");

        let lines = cli_surface_report(&audit);
        assert!(lines[0].starts_with("cli-surface: "), "{lines:?}");
        assert!(lines[0].contains("3 invocation(s)"), "{lines:?}");
        assert!(lines[0].contains("1 asking for help"), "{lines:?}");
        assert!(lines[0].contains("1 clap rejects"), "{lines:?}");
        let body = lines[1..].join("\n");
        assert!(
            body.contains("attest") && body.contains("--owner"),
            "the report counted the gap without naming it:\n{body}"
        );
        // A rejected invocation is named too: normally it is a deliberate
        // invalid-input case, but a *valid* one that stopped parsing is coverage
        // lost, and a count going up says nothing about which.
        assert!(
            body.contains("rejected: aplan nope --wat"),
            "the refusal was counted and not named:\n{body}"
        );
    }

    /// The ratchet answers three ways, and two of them are failures.
    ///
    /// A residue that *shrank* fails as loudly as one that grew: leaving the
    /// good news uncommitted measures the next change against a floor that is no
    /// longer true, and the gate then passes while the surface rots.
    #[test]
    fn the_surface_ratchet_refuses_growth_and_uncommitted_shrinkage() {
        assert!(
            cli_surface_verdict(0, 0)
                .expect("at the floor")
                .contains("floor of 0")
        );

        let grew = cli_surface_verdict(3, 0).expect_err("growth must fail");
        assert!(format!("{grew}").contains("grew from 0 to 3"), "{grew}");
        assert!(format!("{grew}").contains("hold it in"), "{grew}");

        let shrank = cli_surface_verdict(1, 4).expect_err("shrinkage must be committed");
        assert!(
            format!("{shrank}").contains("shrank from 4 to 1"),
            "{shrank}"
        );
        assert!(
            format!("{shrank}").contains("cargo xtask cli-surface"),
            "the fix is not named: {shrank}"
        );
    }

    /// **A record that git has never seen is still a record.** The gate read
    /// `git ls-files`, so a new file was invisible to it -- and a duplicate
    /// number is created *before* it is committed. Reproducing `A1-222`'s own
    /// defect in this repository passed green, which is how the hole was found
    /// (this task's audit).
    #[test]
    fn a_record_that_is_not_tracked_yet_is_read_anyway() {
        let (dir, mut names) = adr_dir(&[
            ("docs/adr/0001-a-thing.md", "# ADR-0001: A thing\n"),
            ("docs/adr/0001-a-rival.md", "# ADR-0001: A rival\n"),
        ]);
        names.retain(|n| n != "docs/adr/0001-a-rival.md");
        let scan = adr_scan(dir.path(), &names);
        assert_eq!(scan.records, 2, "an untracked record was not read");
        assert!(
            scan.problems
                .iter()
                .any(|p| p.contains("ADR-0001 is filed 2 times")),
            "{:?}",
            scan.problems
        );
    }

    /// The attribution survives the backticks prose actually wraps it in. Written
    /// `` `p00025`'s `ADR-0006` ``, the character before the citation is a
    /// backtick rather than the possessive, and reading it strictly filed a
    /// foreign record as a dead citation of ours.
    #[test]
    fn an_attribution_is_read_through_the_backticks_around_the_citation() {
        let scan = scan_of(&[(
            "docs/pilots/p.md",
            "`p00025`'s `ADR-0042` requires a banner.\n",
        )]);
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.foreign.len(), 1, "{:?}", scan.foreign);
    }

    /// With no reservation list, a forward reference is just a dead citation.
    /// The section is found by its heading, so losing it must fail loudly rather
    /// than quietly widening what resolves.
    #[test]
    fn a_missing_reservation_section_resolves_nothing_extra() {
        let scan = scan_of(&[
            (
                "HLD.md",
                "## 19. Something else entirely\n\n1. **ADR-0042: a thing.**\n",
            ),
            ("ROADMAP.md", "Planned as ADR-0042.\n"),
        ]);
        assert!(
            scan.problems
                .iter()
                .filter(|p| p.contains("cites ADR-0042"))
                .count()
                == 2,
            "{:?}",
            scan.problems
        );
        assert!(scan.reserved.is_empty(), "{:?}", scan.reserved);
    }

    /// Section 19 cites another project's numbering too, and reserving that
    /// would quietly make the same number resolve everywhere. The attribution is
    /// honoured on the reservation side as well as the citation side.
    #[test]
    fn a_foreign_number_named_in_the_reservation_list_reserves_nothing() {
        let scan = scan_of(&[
            (
                "HLD.md",
                "## 19. Decisions that need an ADR\n\n1. **A thing.** Mirrors `alint`'s \
                 ADR-0042.\n",
            ),
            ("ROADMAP.md", "Planned as ADR-0042.\n"),
        ]);
        assert!(
            scan.problems.iter().any(|p| p.contains("cites ADR-0042")),
            "another project's number was reserved as ours: {:?}",
            scan.problems
        );
    }

    /// The records are flat, and a nested one would be read by nothing: not the
    /// duplicate check, not the filename check, not the citation resolver. It is
    /// named rather than skipped, because "a record the gate cannot see" is the
    /// class that already got past it once.
    #[test]
    fn a_record_filed_in_a_subdirectory_is_named_rather_than_ignored() {
        let scan = scan_of(&[(
            "docs/adr/superseded/0001-a-rival.md",
            "# ADR-0001: A rival\n",
        )]);
        let report = scan.problems.join("\n");
        assert!(report.contains("docs/adr/superseded/"), "{report}");
        assert!(report.contains("read by nothing here"), "{report}");
    }

    /// A fence that never closes hides every line after it, and the gate would
    /// report nothing about a file it stopped reading a third of the way in.
    #[test]
    fn a_file_that_ends_inside_a_fence_is_reported_rather_than_half_read() {
        let scan = scan_of(&[(
            "docs/notes.md",
            "Fine so far.\n\n```console\n$ a command\n\nAnd then ADR-0042.\n",
        )]);
        let report = scan.problems.join("\n");
        assert!(report.contains("docs/notes.md:3"), "{report}");
        assert!(report.contains("never closed"), "{report}");
    }

    /// A glob is not a link: prose describing the directory writes one, and
    /// demanding it name a file would be the gate inventing a chore.
    #[test]
    fn a_glob_over_the_directory_is_not_read_as_a_dead_link() {
        let scan = scan_of(&[(
            "aplan/tasks/A1-222.md",
            "Enumerate docs/adr/*.md and check.\n",
        )]);
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
    }
}

/// One file in `docs/adr/`, as it declares itself twice: once in its name and
/// once in its heading.
struct AdrFile {
    rel: String,
    /// The number in the filename, `NNNN-slug.md`.
    filed: Option<u16>,
    /// The number in the `# ADR-NNNN:` heading.
    headed: Option<u16>,
}

/// What the decision-record gate found, separated from how it prints for the
/// reason `LintScan` is: so the pass itself can be tested.
#[derive(Default)]
struct AdrScan {
    problems: Vec<String>,
    records: usize,
    citations: usize,
    /// Citations the prose hands to another repository. Reported, never dropped
    /// in silence.
    foreign: Vec<String>,
    /// Citations of a number that is reserved and not yet written. Reported for
    /// the same reason.
    reserved: Vec<String>,
}

impl AdrScan {
    /// What the run did *not* resolve against a record, named rather than
    /// counted. The instinct is `clean_summary`'s: a gate that skips in silence
    /// is one nobody can audit, and both of these skips are judgement calls
    /// about somebody else's prose.
    ///
    /// One line per kind, folded, rather than one line for everything: named
    /// inline, twenty-three skips made a 990-character line that no terminal
    /// could show and no reader could scan, in a repository whose own `A1-206`
    /// is about exactly that (`A1-222`, audit).
    fn aside(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (count, what, entries) in [
            ("attributed to another repository", &self.foreign),
            ("reserved in HLD.md section 19", &self.reserved),
        ]
        .map(|(what, entries)| (entries.len(), what, entries))
        {
            if entries.is_empty() {
                continue;
            }
            lines.push(aplan_output::fold::fold(
                &format!("{count} citation(s) {what}: {}", entries.join(", ")),
                ASIDE_WIDTH,
                4,
            ));
        }
        lines
    }
}

/// The width the asides fold to. Not the terminal's: this is a gate, and its
/// output is read in a CI log as often as beside a prompt.
const ASIDE_WIDTH: usize = 96;

/// The decision record's own integrity, which nothing checked until three
/// defects shipped together (`A1-222`): two files both filed as `ADR-0020`, a
/// citation naming a decision no ADR records, and `ADR-0017` superseded with
/// nothing on it saying so.
///
/// Three questions. A number must be unique, and must agree with its filename --
/// both mechanical. The third, "every citation resolves", is the one that needs
/// care, because this repository's prose is partly *about other repositories*:
/// the pilot logs cite ours forty-two times (`ADR-0015` five times in
/// `p00020.md` alone) and cite `p00025`'s `ADR-0006` in the same directory.
/// Excluding those files to keep the foreign one quiet would drop the forty-two
/// real ones, so the boundary is drawn per citation instead of per file.
///
/// **A citation is this repository's unless the sentence hands it to somebody
/// else**, which prose already spells `` `p00025`'s ADR-0006 ``. Attribution is
/// therefore load-bearing rather than decorative, and the ones taken that way are
/// listed by name on every run: a gate that skips without saying what it skipped
/// is the vacuity this project keeps finding in other people's gates
/// (`clean_summary`).
fn adr_scan(root: &Path, files: &[String]) -> AdrScan {
    let mut scan = AdrScan::default();

    // 1. What the records claim to be.
    let mut nested = Vec::new();
    let records = adr_files(root, &mut nested);
    scan.records = records.len();
    for dir in nested {
        scan.problems.push(format!(
            "{dir} holds decision records in a subdirectory; they are read by nothing here, so \
             a number duplicated inside it resolves as though it were free"
        ));
    }

    // A directory that vanished is not a clean run. The same refusal
    // `check_docs` makes about an unloadable plan: nothing to check and nothing
    // recognised means every citation below would fail, or none would.
    if records.is_empty() {
        scan.problems.push(
            "docs/adr/ holds no decision records; the gate has nothing to resolve \
                   citations against"
                .to_string(),
        );
        return scan;
    }

    let seen = adr_numbering(&records, &mut scan.problems);

    // 2. What the repository says about them. Both forms: the number a reader
    //    cites, and the path a reader clicks. The second is what a renumbering
    //    breaks, and it breaks silently.
    let known: BTreeSet<u16> = seen.keys().copied().collect();
    let on_disk: BTreeSet<&str> = records.iter().map(|r| r.rel.as_str()).collect();
    let held = adr_reservations(root);
    // A record cites other records, and an untracked one is not in `files`.
    let mut sources: Vec<String> = files.to_vec();
    sources.extend(
        records
            .iter()
            .map(|record| record.rel.clone())
            .filter(|rel| !files.contains(rel)),
    );
    for number in held.intersection(&known) {
        scan.problems.push(format!(
            "HLD.md reserves ADR-{number:04} as a decision that still needs writing, and {} \
             records it; retire the reservation",
            seen[number].join(", ")
        ));
    }
    for rel in &sources {
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        let prose = Prose::of(rel);
        // A fence that never closes swallows the rest of the file, and the gate
        // reports nothing about lines it never read. Nothing in this repository
        // has one today; the point is that acquiring one would go unnoticed
        // (`A1-222`, audit).
        if let Some(opened) = unclosed_fence(&text, prose) {
            scan.problems.push(format!(
                "{rel}:{opened}: a ``` block is never closed, so the rest of the file is not \
                 read; close it, or the citations below it are unchecked"
            ));
        }
        // A record's own heading declares its number; it does not cite it. Left
        // in, every record would resolve itself and the count would report
        // twenty-two citations nobody wrote.
        let declaration = on_disk
            .contains(rel.as_str())
            .then(|| heading_line(&text))
            .flatten();
        for (line, number, attributed) in adr_citations(&text, prose) {
            if Some(line) == declaration {
                continue;
            }
            scan.citations += 1;
            let at = format!("{rel}:{line} (ADR-{number:04})");
            if known.contains(&number) {
                // A record exists, so the citation lands somewhere whoever wrote
                // it can be held to.
            } else if attributed {
                // Nothing to land on, and the sentence says whose it is.
                scan.foreign.push(at);
            } else if held.contains(&number) {
                scan.reserved.push(at);
            } else {
                scan.problems.push(format!(
                    "{rel}:{line}: cites ADR-{number:04}, which no file in docs/adr/ records \
                     and HLD.md does not reserve; name the decision that exists, or attribute \
                     it to the repository whose record it is"
                ));
            }
        }
        for (line, path) in adr_paths(&text, prose) {
            if !on_disk.contains(path.as_str()) {
                scan.problems.push(format!(
                    "{rel}:{line}: links `{path}`, which is not a file; a renumbered record \
                     leaves this reading as a live link"
                ));
            }
        }
    }
    scan
}

/// The numbers this repository has reserved and not yet written.
///
/// `HLD.md` section 19 is a list of decisions that need an ADR, each named by the
/// number it will take, and citing one is a forward reference rather than a false
/// claim. This is the *held entry* `AGENTS.md` means: the number has an owner and
/// no record, and the honest state is to say so rather than to leave the number
/// looking free.
///
/// **Reserving is not free**, so the gate retires a reservation the moment its
/// record exists: a list telling a reader to write something already written
/// sends them to do work that is done. `ADR-0007` and `ADR-0008` had both sat
/// there since shipping (`A1-222`).
///
/// A missing section means no reservations, which fails loudly on every forward
/// reference rather than quietly widening what resolves.
fn adr_reservations(root: &Path) -> BTreeSet<u16> {
    let Ok(text) = std::fs::read_to_string(root.join("HLD.md")) else {
        return BTreeSet::new();
    };
    let mut inside = false;
    let mut held = BTreeSet::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            inside = line.contains("need an ADR");
            continue;
        }
        // A numbered entry is a reservation. Prose around the list is prose:
        // the paragraph retiring two of them names both, and reading that as a
        // reservation would make the retirement un-recordable.
        let entry = line
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !inside || !entry {
            continue;
        }
        for (at, _) in line.match_indices("ADR-") {
            let digits: String = line[at + 4..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            // `alint`'s numbering is not ours, and section 19 cites it.
            if line[..at]
                .trim_end()
                .trim_end_matches('`')
                .trim_end()
                .ends_with("'s")
            {
                continue;
            }
            if let Some(number) = adr_number(&digits) {
                held.insert(number);
            }
        }
    }
    held
}

/// The line a markdown fence opens on when nothing closes it.
///
/// `Prose::lines` skips fenced blocks, so a stray fence marker makes every line
/// after it invisible -- and a gate silent about what it did not read is the
/// failure this one exists to prevent.
fn unclosed_fence(text: &str, prose: Prose) -> Option<usize> {
    if prose != Prose::Markdown {
        return None;
    }
    let mut opened = None;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            opened = if opened.is_some() {
                None
            } else {
                Some(index + 1)
            };
        }
    }
    opened
}

/// Which file answers to which number, and every way the two declarations can
/// fail to agree.
///
/// A record declares its number twice, in its name and in its heading, and
/// nothing made them agree. A disagreement is not cosmetic: the file resolves by
/// one number and announces the other, so a reader following a citation lands on
/// a record about something else.
fn adr_numbering(records: &[AdrFile], problems: &mut Vec<String>) -> BTreeMap<u16, Vec<String>> {
    let mut seen: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for record in records {
        match (record.filed, record.headed) {
            (Some(in_name), Some(in_heading)) => {
                if in_name != in_heading {
                    problems.push(format!(
                        "{}: the filename says ADR-{in_name:04} and the heading says \
                         ADR-{in_heading:04}; a reader following a citation lands on whichever \
                         one they read first",
                        record.rel
                    ));
                }
                seen.entry(in_heading).or_default().push(record.rel.clone());
            }
            (None, _) => problems.push(format!(
                "{}: the filename does not begin `NNNN-`, so nothing can cite it by number",
                record.rel
            )),
            (Some(in_name), None) => {
                problems.push(format!(
                    "{}: no `# ADR-{in_name:04}: ` heading; the number lives only in the filename",
                    record.rel
                ));
                seen.entry(in_name).or_default().push(record.rel.clone());
            }
        }
    }
    for (number, holders) in &seen {
        if holders.len() > 1 {
            problems.push(format!(
                "ADR-{number:04} is filed {} times ({}); a citation of it names no single decision",
                holders.len(),
                holders.join(", ")
            ));
        }
    }
    seen
}

/// Every record in `docs/adr/`, read from the directory rather than from
/// `git ls-files`.
///
/// A record is a file, and a duplicate number is created *before* it is
/// committed -- so reading the index made a new file invisible to the one check
/// written to catch it, and reproducing `A1-222`'s own defect passed green (this
/// task's audit). Citations elsewhere still come from the tracked set, which is
/// what keeps `target/` out.
fn adr_files(root: &Path, nested: &mut Vec<String>) -> Vec<AdrFile> {
    let mut paths: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.join("docs/adr")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_markdown(&name) {
                paths.push(format!("docs/adr/{name}"));
            } else if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                // Not read, and said so. The records are flat; a nested one
                // would be invisible to every check here, which is the shape
                // that already got past this gate once (`A1-222`, audit).
                nested.push(format!("docs/adr/{name}/"));
            }
        }
    }
    paths.sort();
    nested.sort();
    paths
        .iter()
        .filter_map(|rel| {
            let text = std::fs::read_to_string(root.join(rel)).ok()?;
            Some(adr_file(rel, &text))
        })
        .collect()
}

/// Markdown, whatever case the extension is written in.
fn is_markdown(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// A record's two self-declarations, read without inferring one from the other:
/// the whole point is to catch them disagreeing.
fn adr_file(rel: &str, text: &str) -> AdrFile {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let filed = name
        .strip_suffix(".md")
        .and_then(|stem| stem.split_once('-'))
        .and_then(|(number, _)| adr_number(number));
    let headed = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| line.trim().strip_prefix("# ADR-"))
        .and_then(|rest| rest.split_once(':'))
        .and_then(|(number, _)| adr_number(number));
    AdrFile {
        rel: rel.to_string(),
        filed,
        headed,
    }
}

/// The line a record declares its own number on, which is the first non-empty
/// one.
fn heading_line(text: &str) -> Option<usize> {
    text.lines()
        .position(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .filter(|_| {
            text.lines()
                .find(|line| !line.trim().is_empty())
                .is_some_and(|line| line.trim().starts_with("# ADR-"))
        })
}

/// Exactly four digits. `ADR-000` in a test fixture and `ADR-00xx` in a task
/// file are not citations, and reading them loosely would make the gate report
/// on its own examples.
fn adr_number(s: &str) -> Option<u16> {
    (s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok())?
}

/// Where a citation can live, which is prose and not data.
///
/// A citation is a claim to a reader that a decision record exists. A string a
/// test builds is neither: `consumer_facing.rs` constructs twelve `ADR-1nnn`
/// codes precisely *because* no such record exists, and a fixture repository in
/// `aplan-evidence` commits a file under `docs/adr/` to give a glob something to
/// bind to. Reading Rust whole rather than at its comments reports forty-two
/// problems here, every one of them a string inside a test: twelve in
/// `consumer_facing.rs`, three in that fixture, and twenty-seven in this gate's
/// own tests.
///
/// So Rust is read at its comments, and markdown outside its fences. The fence
/// rule has one job: a pilot log quotes the *consumer's* console, so
/// pre-public adoption evidence carries a template path emitted by a repository
/// that is not this one. Nothing else is excluded -- in particular the pilot
/// logs themselves are read, because they cite this repository's records
/// forty-two times.
///
/// A citation in a *user-facing string* is not missed by this: `A1-044` forbids
/// one outright, and `consumer_facing.rs` is the gate that enforces it.
#[derive(Clone, Copy, PartialEq)]
enum Prose {
    Markdown,
    RustComments,
    Whole,
}

impl Prose {
    fn of(rel: &str) -> Self {
        let extension = std::path::Path::new(rel)
            .extension()
            .map(std::ffi::OsStr::to_ascii_lowercase);
        match extension.as_deref().and_then(|e| e.to_str()) {
            Some("md") => Prose::Markdown,
            Some("rs") => Prose::RustComments,
            _ => Prose::Whole,
        }
    }

    /// The lines to read, with markdown's fenced blocks already dropped.
    fn lines(self, text: &str) -> Vec<(usize, &str)> {
        let mut kept = Vec::new();
        let mut fenced = false;
        for (index, line) in text.lines().enumerate() {
            if self == Prose::Markdown && line.trim_start().starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if fenced {
                continue;
            }
            if self == Prose::RustComments && !line.trim_start().starts_with("//") {
                continue;
            }
            kept.push((index + 1, line));
        }
        kept
    }
}

/// Every `ADR-NNNN` in a file's prose, each with whether the sentence hands it to
/// somebody else.
///
/// The possessive is *not* read as "not ours". It cannot be: this repository's
/// own prose writes "this repository's ADR-0009" and "this project's ADR-0009"
/// to distinguish its records from a pilot's, so treating the form as foreign
/// would stop resolving four real citations to keep two quiet. Attribution is
/// consulted only where resolution has already failed, which is the sole place
/// it changes an answer (`A1-222`, audit).
fn adr_citations(text: &str, prose: Prose) -> Vec<(usize, u16, bool)> {
    let mut found = Vec::new();
    for (line_number, line) in prose.lines(text) {
        for (at, _) in line.match_indices("ADR-") {
            // `MADR-0001` is not a citation of `ADR-0001`.
            if line[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric())
            {
                continue;
            }
            let rest = &line[at + 4..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let Some(number) = adr_number(&digits) else {
                continue;
            };
            // `` `p00025`'s ADR-0006 `` is p00025's record, and this repository
            // has no file to resolve it against. The possessive is the
            // attribution, and it is the only one prose already writes.
            //
            // Read on the citation's own line, so a possessive left at the end
            // of the line above is not seen. That fails towards reporting: the
            // citation is judged as ours and, if it resolves to nothing, named.
            // A wrapped attribution is a line break away from being fixed, and
            // the alternative -- joining lines before scanning -- would lose the
            // line number the report is worth reading for.
            let attributed = line[..at]
                .trim_end()
                .trim_end_matches('`')
                .trim_end()
                .ends_with("'s");
            found.push((line_number, number, attributed));
        }
    }
    found
}

/// Every path into `docs/adr/`, which is the citation form a renumbering breaks
/// without changing a word of the sentence around it.
///
/// A glob is not a link. Prose describing the directory as a whole writes one,
/// and demanding that it name a file would be the gate inventing a chore.
fn adr_paths(text: &str, prose: Prose) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (line_number, line) in prose.lines(text) {
        for (at, _) in line.match_indices("docs/adr/") {
            let rest = &line[at..];
            let Some(end) = rest.find(".md").map(|e| e + 3) else {
                continue;
            };
            let path = &rest[..end];
            let name = &path["docs/adr/".len()..];
            if name.contains(|c: char| c.is_whitespace() || c == '`' || c == '*') {
                continue;
            }
            found.push((line_number, path.to_string()));
        }
    }
    found
}
