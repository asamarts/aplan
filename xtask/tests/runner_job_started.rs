//! The private runner's cleanup hook is a deletion boundary, so this test runs
//! the production script over real directories rather than restating its shell
//! predicates in Rust (`A1-632`).

use std::os::unix::fs::symlink;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use crate::fixture_executable::install_executable;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a workspace parent")
        .to_path_buf()
}

fn hook(target: &Path, cap_gib: &str) -> Output {
    Command::new("/bin/bash")
        .arg(workspace_root().join("ci/runner/job-started.sh"))
        .arg(target)
        .env_clear()
        // The script must use /bin/sh's `kill` builtin. In particular there is
        // no executable named `kill` (or anything else) on this PATH.
        .env("PATH", "/path-with-no-executables")
        .env("TARGET_CAP_GIB", cap_gib)
        .output()
        .expect("run the production job-started hook")
}

fn hook_until_removed(target: &Path, allocation: &Path, cap_gib: &str) {
    let mut last_output = None;
    for _ in 0..100 {
        let output = hook(target, cap_gib);
        assert!(output.status.success(), "hook failed: {output:?}");
        if !allocation.exists() {
            return;
        }
        last_output = Some(output);
    }
    let output = last_output.expect("at least one hook attempt");
    panic!(
        "the hook never reclaimed the proved-dead allocation after 100 conservative scans:\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn legacy_reclaimer(target: &Path) -> Output {
    Command::new("/usr/bin/python3")
        .arg(workspace_root().join("ci/runner/reclaim-coverage.py"))
        .arg("--legacy-quiescent")
        .arg(target)
        .env_clear()
        .output()
        .expect("run the production legacy reclaimer")
}

fn python_reclaimer_contract(program: &str) -> Output {
    Command::new("/usr/bin/python3")
        .arg("-c")
        .arg(program)
        .arg(workspace_root().join("ci/runner/reclaim-coverage.py"))
        .env_clear()
        .output()
        .expect("exercise the production Python reclaimer contract")
}

fn directory(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    std::fs::create_dir(&path).unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
    std::fs::write(path.join("artifact"), b"fixture")
        .unwrap_or_else(|error| panic!("populate {}: {error}", path.display()));
    path
}

fn process_identity(pid: u32) -> serde_json::Value {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .expect("boot id")
        .trim()
        .to_string();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("process stat");
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .expect("stat command delimiter")
        .1
        .split_whitespace()
        .collect();
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("process status");
    let uid: u32 = status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .expect("Uid line")
        .split_whitespace()
        .nth(1)
        .expect("real uid")
        .parse()
        .expect("numeric uid");
    serde_json::json!({
        "pid": pid,
        "boot_id": boot_id,
        "start_ticks": fields[19].parse::<u64>().expect("start ticks"),
        "uid": uid,
    })
}

fn previous_boot_identity(pid: u32) -> serde_json::Value {
    let mut identity = process_identity(std::process::id());
    identity["pid"] = serde_json::json!(pid);
    identity["start_ticks"] = serde_json::json!(1);
    let current_boot = identity["boot_id"].as_str().expect("current boot identity");
    identity["boot_id"] =
        serde_json::json!(if current_boot == "00000000-0000-0000-0000-000000000000" {
            "11111111-1111-1111-1111-111111111111"
        } else {
            "00000000-0000-0000-0000-000000000000"
        });
    identity
}

fn owner_record(
    directory: &Path,
    owner: &serde_json::Value,
    child: Option<&serde_json::Value>,
    process_group: Option<u32>,
    lease: &str,
) {
    let phase = if child.is_some() {
        "running"
    } else {
        "prepared"
    };
    let mut bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1,
        "phase": phase,
        "owner": owner,
        "child": child,
        "process_group": process_group,
        "lease": lease,
    }))
    .expect("serialize owner record");
    bytes.push(b'\n');
    std::fs::write(directory.join(".aplan-coverage-owner-v1.json"), bytes)
        .expect("write owner record");
}

fn wait_for(path: &Path, exists: bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while path.exists() != exists && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        path.exists(),
        exists,
        "timed out waiting for {}",
        path.display()
    );
}

fn wait_for_live_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = "the file does not exist".to_string();
    loop {
        match std::fs::read_to_string(path) {
            Ok(contents) => match contents.trim().parse::<u32>() {
                Ok(pid) if Path::new(&format!("/proc/{pid}")).is_dir() => return pid,
                Ok(pid) => observed = format!("PID {pid} is not live"),
                Err(error) => observed = format!("contents {contents:?} are not a PID: {error}"),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => observed = format!("the file cannot be read: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {} to publish a live PID; last observation: {observed}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct FakeCoverage {
    bin: PathBuf,
    target: PathBuf,
    configured: PathBuf,
    ready: PathBuf,
}

fn fake_coverage(root: &Path) -> FakeCoverage {
    let bin = root.join("bin");
    std::fs::create_dir(&bin).expect("create fake bin");
    let cargo = bin.join("cargo");
    install_executable(
        &cargo,
        r#"#!/bin/sh
case "$1" in
  metadata)
    printf '{"target_directory":"%s"}\n' "$FAKE_TARGET"
    ;;
  llvm-cov)
    printf '%s\n' "$$" > "$FAKE_READY"
    exec /bin/sleep 60
    ;;
  *)
    echo "unexpected fake cargo arguments: $*" >&2
    exit 2
    ;;
esac
"#,
    );
    FakeCoverage {
        bin,
        target: root.join("ordinary-target"),
        configured: root.join("llvm-cov-target"),
        ready: root.join("cargo.pid"),
    }
}

fn start_fake_coverage(fixture: &FakeCoverage) -> std::process::Child {
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let mut search_path = fixture.bin.clone().into_os_string();
    search_path.push(":");
    search_path.push(inherited_path);
    Command::new(crate::common::xtask())
        .args(["coverage", "--check"])
        .env("PATH", search_path)
        .env("FAKE_TARGET", &fixture.target)
        .env("FAKE_READY", &fixture.ready)
        .env("CARGO_LLVM_COV_TARGET_DIR", &fixture.configured)
        .env_remove("CARGO_LLVM_COV_BUILD_DIR")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start xtask coverage")
}

#[test]
fn the_real_hook_preserves_live_and_hostile_entries_and_bounds_build_output() {
    let fixture = tempfile::Builder::new()
        .prefix("runner-job-started-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create hook fixture");
    let target = fixture.path().join("cargo-target");
    std::fs::create_dir(&target).expect("create target mount");

    let live = directory(
        &target,
        &format!(".aplan-coverage-{}-0", std::process::id()),
    );
    let dead = directory(&target, ".aplan-coverage-999999999-7");
    owner_record(
        &live,
        &process_identity(std::process::id()),
        None,
        None,
        "aplan-coverage-v1-live-hook",
    );
    owner_record(
        &dead,
        &previous_boot_identity(999_999_999),
        None,
        None,
        "aplan-coverage-v1-dead-hook",
    );
    let unrecorded = directory(&target, ".aplan-coverage-999999998-0");
    let malformed = directory(&target, ".aplan-coverage-not-a-pid-1");
    let extra_suffix = directory(&target, ".aplan-coverage-999999999-1-extra");

    let outside = directory(fixture.path(), "outside");
    let coverage_link = target.join(".aplan-coverage-999999999-8");
    symlink(&outside, &coverage_link).expect("create coverage-shaped symlink");
    let output_link = target.join("aarch64-hostile");
    symlink(&outside, &output_link).expect("create output-shaped symlink");

    let removable = [
        "debug",
        "release",
        "doc",
        "tmp",
        "package",
        "llvm-cov-target",
        "msrv",
        "x86_64-unknown-linux-musl",
        "wasm32-wasip1",
        "aarch64-apple-darwin",
    ]
    .map(|name| directory(&target, name));
    let unknown = directory(&target, "operator-notes");

    // A zero-GiB test cap forces the same over-cap branch production reaches at
    // 100 GiB without allocating a giant sparse fixture.
    let output = hook(&target, "0");
    assert!(
        output.status.success(),
        "hook failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("removing stale .aplan-coverage-999999999-7"),
        "hook omitted the stale-allocation verdict:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("clearing build output"),
        "hook omitted the over-cap verdict:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    assert!(live.is_dir(), "a live coverage run was removed");
    assert!(!dead.exists(), "a dead coverage run was retained");
    assert!(
        unrecorded.is_dir(),
        "an unrecorded coverage run was treated as proved stale"
    );
    assert!(
        malformed.is_dir(),
        "a malformed candidate was treated as owned"
    );
    assert!(
        extra_suffix.is_dir(),
        "a non-exact candidate was treated as owned"
    );
    assert!(coverage_link.is_symlink(), "a coverage symlink was removed");
    assert!(
        output_link.is_symlink(),
        "a build-output symlink was removed"
    );
    assert!(
        outside.join("artifact").is_file(),
        "a symlink target was touched"
    );
    assert!(unknown.is_dir(), "an unknown target-root entry was removed");
    for path in removable {
        assert!(
            !path.exists(),
            "build output {} was retained",
            path.display()
        );
    }
}

#[test]
fn the_real_hook_retains_reused_pids_and_malformed_owner_records() {
    let fixture = tempfile::Builder::new()
        .prefix("runner-job-started-records-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create hook fixture");
    let target = fixture.path().join("cargo-target");
    std::fs::create_dir(&target).expect("create target mount");
    let reused = directory(
        &target,
        &format!(".aplan-coverage-{}-1", std::process::id()),
    );
    let mut reused_identity = process_identity(std::process::id());
    reused_identity["start_ticks"] = serde_json::json!(
        reused_identity["start_ticks"]
            .as_u64()
            .expect("start ticks")
            .saturating_add(1)
    );
    owner_record(
        &reused,
        &reused_identity,
        None,
        None,
        "aplan-coverage-v1-reused-hook",
    );
    let malformed = directory(&target, ".aplan-coverage-999999997-0");
    std::fs::write(malformed.join(".aplan-coverage-owner-v1.json"), b"not json")
        .expect("write malformed owner record");
    let oversized_pid = directory(&target, ".aplan-coverage-4294967296-0");
    let mut oversized_identity = process_identity(std::process::id());
    oversized_identity["pid"] = serde_json::json!(4_294_967_296_u64);
    owner_record(
        &oversized_pid,
        &oversized_identity,
        None,
        None,
        "aplan-coverage-v1-oversized-pid",
    );

    let output = hook(&target, "1");
    assert!(output.status.success(), "hook failed: {output:?}");
    assert!(reused.is_dir(), "a reused PID was treated as the old owner");
    assert!(
        malformed.is_dir(),
        "a malformed owner record was treated as an ownership proof"
    );
    assert!(
        oversized_pid.is_dir(),
        "a PID outside the producer's u32 domain was accepted"
    );
}

#[test]
fn the_python_reclaimer_refuses_every_malformed_current_boot_identity() {
    let output = python_reclaimer_contract(
        r#"
import runpy
import sys

contract = runpy.run_path(sys.argv[1])
valid_boot_id = contract["valid_boot_id"]
assert valid_boot_id("01234567-89ab-cdef-0123-456789abcdef")
for value in (
    "",
    "01234567-89AB-CDEF-0123-456789ABCDEF",
    "01234567-89ab-cdef-0123-456789abcde",
    "01234567-89ab-cdef-0123-456789abcdef0",
    "0123456789ab-cdef-0123-456789abcdef",
    "01234567-89ab-cdef-0123-456789abcdeg",
    " 01234567-89ab-cdef-0123-456789abcdef",
    None,
    True,
):
    assert not valid_boot_id(value), value
"#,
    );
    assert!(
        output.status.success(),
        "Python reclaimer identity contract failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn quiescent_migration_removes_only_exact_ownerless_legacy_allocations() {
    let fixture = tempfile::Builder::new()
        .prefix("runner-legacy-coverage-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create legacy fixture");
    let target = fixture.path().join("cargo-target");
    std::fs::create_dir(&target).expect("create target mount");
    let legacy = directory(&target, ".aplan-coverage-999999999-0");
    let malformed = directory(&target, ".aplan-coverage-999999998-0");
    std::fs::write(malformed.join(".aplan-coverage-owner-v1.json"), b"not json")
        .expect("write malformed owner record");
    let outside = directory(fixture.path(), "legacy-outside");
    let link = target.join(".aplan-coverage-999999997-0");
    symlink(&outside, &link).expect("create legacy-shaped symlink");

    let output = legacy_reclaimer(&target);
    assert!(output.status.success(), "reclaimer failed: {output:?}");
    assert!(!legacy.exists(), "the ownerless legacy allocation remained");
    assert!(
        malformed.is_dir(),
        "an existing but malformed owner record was treated as legacy"
    );
    assert!(link.is_symlink(), "a legacy-shaped symlink was removed");
    assert!(
        outside.join("artifact").is_file(),
        "a legacy-shaped symlink target was touched"
    );
}

#[test]
fn a_symlink_target_mount_is_a_refusal_that_touches_nothing() {
    let fixture = tempfile::Builder::new()
        .prefix("runner-job-started-root-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create hook fixture");
    let outside = directory(fixture.path(), "outside-target");
    let link = fixture.path().join("target-link");
    symlink(&outside, &link).expect("create target symlink");

    let output = hook(&link, "0");
    assert!(!output.status.success(), "a symlink target was accepted");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("refusing symlink target mount"),
        "refusal did not name its reason: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(outside.join("artifact").is_file(), "the target was changed");
}

#[test]
fn an_under_cap_target_keeps_build_output_but_still_reclaims_stale_coverage() {
    let fixture = tempfile::Builder::new()
        .prefix("runner-job-started-under-cap-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create hook fixture");
    let target = fixture.path().join("cargo-target");
    std::fs::create_dir(&target).expect("create target mount");
    let build_output = directory(&target, "debug");
    let stale = directory(&target, ".aplan-coverage-999999999-0");
    owner_record(
        &stale,
        &previous_boot_identity(999_999_999),
        None,
        None,
        "aplan-coverage-v1-under-cap",
    );

    let output = hook(&target, "1");
    assert!(
        output.status.success(),
        "hook failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        build_output.is_dir(),
        "ordinary build output was removed below the threshold"
    );
    assert!(!stale.exists(), "stale coverage output was not reclaimed");
}

#[test]
fn the_real_hook_keeps_a_killed_owners_child_then_reclaims_the_dead_tree() {
    use std::os::unix::process::CommandExt as _;

    let fixture = tempfile::Builder::new()
        .prefix("runner-job-started-process-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create hook process fixture");
    let target = fixture.path().join("cargo-target");
    std::fs::create_dir(&target).expect("create target mount");
    let descendant_pid_file = fixture.path().join("descendant.pid");
    let lease = "aplan-coverage-v1-hook-child";
    let mut owner = Command::new("/bin/sleep")
        .arg("60")
        .spawn()
        .expect("spawn coverage owner");
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("sleep 60 & echo $! > \"$1\"; wait")
        .arg("coverage-child")
        .arg(&descendant_pid_file)
        .env("APLAN_COVERAGE_LEASE", lease)
        .process_group(0);
    let mut child = command.spawn().expect("spawn coverage child");
    let process_group = child.id();
    let descendant_pid = wait_for_live_pid(&descendant_pid_file);
    let allocation = directory(&target, &format!(".aplan-coverage-{}-0", owner.id()));
    owner_record(
        &allocation,
        &process_identity(owner.id()),
        Some(&process_identity(child.id())),
        Some(process_group),
        lease,
    );

    owner.kill().expect("kill owner");
    owner.wait().expect("reap owner");
    let output = hook(&target, "1");
    assert!(output.status.success(), "hook failed: {output:?}");
    assert!(
        allocation.is_dir(),
        "the hook removed an allocation with a live descendant"
    );

    child.kill().expect("kill recorded child");
    child.wait().expect("reap recorded child");
    let output = hook(&target, "1");
    assert!(output.status.success(), "hook failed: {output:?}");
    assert!(
        allocation.is_dir(),
        "the hook removed an allocation with a live group descendant"
    );

    let status = Command::new("/bin/sh")
        .args([
            "-c",
            "kill -KILL \"-$1\" 2>/dev/null",
            "coverage-test",
            &process_group.to_string(),
        ])
        .status()
        .expect("kill coverage process group");
    assert!(status.success(), "could not kill the fixture process group");
    wait_for(&PathBuf::from(format!("/proc/{descendant_pid}")), false);

    hook_until_removed(&target, &allocation, "1");
}

#[test]
fn coverage_termination_stops_the_child_cleans_the_allocation_and_preserves_the_signal() {
    let fixture = tempfile::Builder::new()
        .prefix("coverage-termination-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create termination fixture");
    let coverage = fake_coverage(fixture.path());
    let child = start_fake_coverage(&coverage);
    let allocation = fixture
        .path()
        .join(format!(".aplan-coverage-{}-0", child.id()));
    let cargo_pid = wait_for_live_pid(&coverage.ready);
    wait_for(&allocation, true);

    let status = Command::new("/bin/sh")
        .args([
            "-c",
            "kill -TERM \"$1\"",
            "coverage-test",
            &child.id().to_string(),
        ])
        .status()
        .expect("signal xtask coverage");
    assert!(status.success(), "could not signal xtask coverage");
    let status = child
        .wait_with_output()
        .expect("wait for terminated coverage");
    assert_eq!(
        status.status.signal(),
        Some(15),
        "termination was translated instead of preserved: {status:?}"
    );
    assert!(
        !allocation.exists(),
        "catchable termination left its private coverage allocation"
    );
    wait_for(&PathBuf::from(format!("/proc/{cargo_pid}")), false);
}

#[test]
fn a_hard_killed_run_is_reclaimed_only_after_its_real_child_tree_exits() {
    let fixture = tempfile::Builder::new()
        .prefix("coverage-hard-kill-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("create hard-kill fixture");
    let coverage = fake_coverage(fixture.path());
    let mut owner = start_fake_coverage(&coverage);
    let allocation = fixture
        .path()
        .join(format!(".aplan-coverage-{}-0", owner.id()));
    let cargo_pid = wait_for_live_pid(&coverage.ready);
    wait_for(&allocation, true);

    owner.kill().expect("hard kill coverage owner");
    let status = owner.wait().expect("reap hard-killed owner");
    assert_eq!(status.signal(), Some(9), "owner did not die by SIGKILL");
    let output = hook(fixture.path(), "1");
    assert!(output.status.success(), "hook failed: {output:?}");
    assert!(
        allocation.is_dir(),
        "the Rust owner record did not protect its live cargo child"
    );

    let status = Command::new("/bin/sh")
        .args([
            "-c",
            "kill -KILL \"-$1\" 2>/dev/null; kill -KILL \"$1\" 2>/dev/null; true",
            "coverage-test",
            &cargo_pid.to_string(),
        ])
        .status()
        .expect("kill fake cargo process group");
    assert!(status.success(), "could not kill fake cargo process group");
    wait_for(&PathBuf::from(format!("/proc/{cargo_pid}")), false);
    let output = hook(fixture.path(), "1");
    assert!(output.status.success(), "hook failed: {output:?}");
    assert!(
        !allocation.exists(),
        "the Python hook did not reclaim the dead Rust allocation"
    );
}
