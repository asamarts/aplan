use crate::common;
use std::io::Write as _;

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("aplan-timings-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan directory");
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: timing fixture\nphase: 1\nstatus: todo\n---\n\nprivate body\n",
    )
    .expect("task");
    dir
}

fn without_timing_line(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|line| !line.starts_with("aplan: timings "))
        .fold(String::new(), |mut output, line| {
            output.push_str(line);
            output.push('\n');
            output
        })
}

/// A `ready` document records when its claim snapshot was observed. Two
/// processes are allowed to cross a wall-clock second while reading the same
/// plan, so compare the stable answer while still requiring each process to
/// emit the timestamp with its public type.
fn stable_ready(output: &str) -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(output).expect("ready JSON");
    let observed = value
        .as_object_mut()
        .expect("ready JSON object")
        .remove("claim_observed_at_epoch_seconds")
        .expect("claim observation instant");
    assert!(
        observed.as_i64().is_some(),
        "claim observation instant is not an integer: {observed}"
    );
    value
}

/// The diagnostic is intentionally outside stable stdout and reports work,
/// not plan content. It also proves the default read does not retain candidate
/// bytes merely to answer `ready`.
#[test]
fn timings_report_phases_and_work_without_changing_machine_output() {
    let dir = fixture();
    let plain = common::run(
        dir.path(),
        &[
            "--no-config",
            "--root",
            "aplan",
            "ready",
            "--format",
            "json",
        ],
    );
    let measured = common::run(
        dir.path(),
        &[
            "--timings",
            "--cache",
            "off",
            "--no-config",
            "--root",
            "aplan",
            "ready",
            "--format",
            "json",
        ],
    );

    assert_eq!(measured.code, plain.code);
    assert_eq!(stable_ready(&measured.stdout), stable_ready(&plain.stdout));
    assert_eq!(without_timing_line(&measured.stderr), plain.stderr);
    let line = measured
        .stderr
        .lines()
        .find(|line| line.starts_with("aplan: timings "))
        .expect("timing diagnostic");
    for field in [
        "total_ms=",
        "config_ms=",
        "load_ms=",
        "validate_ms=",
        "project_ms=",
        "render_ms=",
        "config_loads=1",
        "plan_loads=1",
        "candidates=1",
        "readable=1",
        "tasks_parsed=1",
        "retained_files=0",
        "retained_bytes=0",
        "graph_builds=1",
        "cache_hits=0",
        "cache_reused=0",
        "cache_parsed=1",
        "cache_bytes_read=0",
        "cache_bytes_written=0",
        "trace_reads=0",
        "trace_bytes_read=0",
        "markdown_parses=0",
        "structured_parses=0",
    ] {
        assert!(line.contains(field), "missing `{field}` in {line}");
    }
    assert!(
        !line.contains("private body"),
        "timings exposed source text"
    );
}

/// The CLI diagnostic carries A1-322's snapshot invariant end to end: two task
/// references to one Markdown source are one read and one parse, while a second
/// structured source is counted separately.
#[test]
fn timings_count_each_shared_trace_document_once() {
    let dir = fixture();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndetect_drift: true\n",
    )
    .expect("config");
    let markdown = "# Intro\n\nShared fact.\n";
    let yaml = "record:\n  state: ready\n";
    std::fs::write(dir.path().join("D.md"), markdown).expect("Markdown trace");
    std::fs::write(dir.path().join("spec.yaml"), yaml).expect("YAML trace");
    for (id, trace) in [
        ("A-1", "D.md#intro"),
        ("A-2", "D.md#intro"),
        ("A-3", "spec.yaml#/record"),
    ] {
        std::fs::write(
            dir.path().join(format!("aplan/tasks/{id}.md")),
            format!(
                "---\nid: {id}\ntitle: traced timing fixture\nphase: 1\nstatus: todo\n\
                 effort_days: 1\ntraces:\n- {trace}\n---\n"
            ),
        )
        .expect("task");
    }

    let run = common::run(
        dir.path(),
        &["--timings", "--cache", "off", "check", "--format", "json"],
    );
    assert_eq!(run.code, 0, "{}", run.text());
    let timing = run
        .stderr
        .lines()
        .find(|line| line.starts_with("aplan: timings "))
        .expect("timing diagnostic");
    let byte_count = format!("trace_bytes_read={}", markdown.len() + yaml.len());
    for field in [
        "trace_reads=2",
        byte_count.as_str(),
        "markdown_parses=1",
        "structured_parses=1",
    ] {
        assert!(timing.contains(field), "missing `{field}` in {timing}");
    }
}

#[test]
fn timings_survive_a_usage_error_without_changing_its_exit() {
    let dir = fixture();
    let run = common::run(
        dir.path(),
        &["--timings", "--no-config", "--root", "missing", "ready"],
    );
    assert_eq!(run.code, 2, "{}", run.text());
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .contains("plan root `missing` is not a directory")
    );
    assert!(run.stderr.contains("aplan: timings "));
    assert!(run.stderr.contains("plan_loads=0"));
}

#[test]
fn show_retains_source_only_for_body_and_reuses_its_validation_graph() {
    let dir = fixture();
    let ordinary = common::run(
        dir.path(),
        &[
            "--timings",
            "--no-config",
            "--root",
            "aplan",
            "show",
            "A-1",
            "--no-resolve",
        ],
    );
    assert_eq!(ordinary.code, 0, "{}", ordinary.text());
    let timing = ordinary
        .stderr
        .lines()
        .find(|line| line.starts_with("aplan: timings "))
        .expect("timing diagnostic");
    assert!(timing.contains("retained_files=0"), "{timing}");
    assert!(timing.contains("retained_bytes=0"), "{timing}");
    assert!(timing.contains("graph_builds=1"), "{timing}");

    let with_body = common::run(
        dir.path(),
        &[
            "--timings",
            "--no-config",
            "--root",
            "aplan",
            "show",
            "A-1",
            "--body",
            "--no-resolve",
        ],
    );
    assert_eq!(with_body.code, 0, "{}", with_body.text());
    assert!(with_body.stdout.contains("private body"));
    let timing = with_body
        .stderr
        .lines()
        .find(|line| line.starts_with("aplan: timings "))
        .expect("timing diagnostic");
    assert!(timing.contains("retained_files=1"), "{timing}");
    assert!(!timing.contains("retained_bytes=0"), "{timing}");
    assert!(timing.contains("graph_builds=1"), "{timing}");
}

/// The committed harness is part of the performance contract, not an example
/// command that can rot independently of the timing diagnostic it consumes.
#[cfg(target_os = "linux")]
#[test]
fn benchmark_harness_measures_external_and_internal_phases() {
    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/read-performance.py");
    let output = std::process::Command::new("python3")
        .current_dir(&workspace)
        .arg(script)
        .arg("--binary")
        .arg(common::binary())
        .arg("--profile")
        .arg("integration-test")
        .arg("--root")
        .arg(dir.path().join("aplan"))
        .arg("--task-id")
        .arg("A-1")
        .arg("--runs")
        .arg("1")
        .arg("--warmups")
        .arg("0")
        .arg("--generated-tasks")
        .arg("3")
        .arg("--timeout-seconds")
        .arg("5")
        .output()
        .expect("run read-performance harness");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8(output.stdout).expect("UTF-8 benchmark report");
    for expected in [
        "# Read-path baseline: integration-test",
        "| corpus / command | wall min | wall p50 | wall p95 | max |",
        "## CLI phase medians",
        "## Deterministic work",
        "| generated-3 / check |",
    ] {
        assert!(
            report.contains(expected),
            "missing `{expected}` in:\n{report}"
        );
    }
}

/// A fast, stable failure or a mixture of two executable/source states is not
/// a performance result. These refusal paths prevent an accidental usage-error
/// benchmark from looking excellent and a concurrent rebuild from looking
/// reproducible.
#[cfg(target_os = "linux")]
#[test]
fn benchmark_harness_refuses_invalid_or_mixed_samples() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/read-performance.py");
    let invoke = |binary: &std::path::Path, task_id: &str| {
        std::process::Command::new("python3")
            .current_dir(&workspace)
            .env("REAL_APLAN", common::binary())
            .arg(&script)
            .arg("--binary")
            .arg(binary)
            .arg("--profile")
            .arg("refusal-test")
            .arg("--root")
            .arg(dir.path().join("aplan"))
            .arg("--task-id")
            .arg(task_id)
            .arg("--runs")
            .arg("1")
            .arg("--warmups")
            .arg("0")
            .arg("--generated-tasks")
            .arg("0")
            .arg("--timeout-seconds")
            .arg("5")
            .output()
            .expect("run read-performance harness")
    };

    let failed = invoke(common::binary(), "missing-task");
    assert!(!failed.status.success(), "an error path was accepted");
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("refusing to benchmark an error path"),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );

    let wrapper = dir.path().join("moving-aplan");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\n\"$REAL_APLAN\" \"$@\"\nstatus=$?\nprintf '\n' >> \"$0\"\nexit \"$status\"\n",
    )
    .expect("write moving binary wrapper");
    let mut permissions = std::fs::metadata(&wrapper)
        .expect("wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&wrapper, permissions).expect("make wrapper executable");

    let moved = invoke(&wrapper, "A-1");
    assert!(!moved.status.success(), "a moving binary was accepted");
    assert!(
        String::from_utf8_lossy(&moved.stderr)
            .contains("benchmark binary changed while samples were running"),
        "{}",
        String::from_utf8_lossy(&moved.stderr)
    );
}

/// The timeout owns a process tree, not only its direct child: otherwise an
/// opaque command can retain build locks and both captured pipes after the
/// benchmark has reported that it stopped.
#[cfg(target_os = "linux")]
#[test]
fn benchmark_harness_kills_timed_out_process_group() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/read-performance.py");
    let child_pid = dir.path().join("descendant.pid");
    let sleeper = dir.path().join("sleeping-aplan");
    std::fs::write(
        &sleeper,
        "#!/bin/sh\nsleep 30 &\nprintf '%s\n' \"$!\" > \"$PID_FILE\"\nsleep 30\n",
    )
    .expect("write sleeping wrapper");
    let mut permissions = std::fs::metadata(&sleeper)
        .expect("sleeper metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&sleeper, permissions).expect("make sleeper executable");
    let timed_out = std::process::Command::new("python3")
        .current_dir(&workspace)
        .env("PID_FILE", &child_pid)
        .arg(&script)
        .arg("--binary")
        .arg(&sleeper)
        .arg("--profile")
        .arg("timeout-test")
        .arg("--root")
        .arg(dir.path().join("aplan"))
        .arg("--task-id")
        .arg("A-1")
        .arg("--runs")
        .arg("1")
        .arg("--warmups")
        .arg("0")
        .arg("--generated-tasks")
        .arg("0")
        .arg("--timeout-seconds")
        .arg("0.2")
        .output()
        .expect("run timing-out harness");
    assert!(
        !timed_out.status.success(),
        "a timed-out sample was accepted"
    );
    assert!(
        String::from_utf8_lossy(&timed_out.stderr).contains("command exceeded 0.2s"),
        "{}",
        String::from_utf8_lossy(&timed_out.stderr)
    );
    let child_pid = std::fs::read_to_string(child_pid)
        .expect("descendant pid")
        .trim()
        .to_string();
    let state = std::fs::read_to_string(format!("/proc/{child_pid}/stat"))
        .ok()
        .and_then(|stat| stat.split_whitespace().nth(2).map(str::to_string));
    assert!(
        state.as_deref().is_none_or(|state| state == "Z"),
        "timed-out descendant {child_pid} remained alive in state {state:?}"
    );
}

#[cfg(target_os = "linux")]
fn assert_receipt_phases(case: &serde_json::Value) {
    for phase in [
        "receipt_manifest",
        "receipt_material",
        "receipt_parse",
        "receipt_identity",
        "receipt_authentication",
        "receipt_catalogue",
    ] {
        assert!(
            case["receipt_phase_p50_ms"][phase].is_number(),
            "missing receipt phase `{phase}`: {case}"
        );
    }
}

fn assert_case_fingerprints(case: &serde_json::Value) {
    for field in ["stdout_sha256", "stderr_sha256", "verdict_sha256"] {
        assert_eq!(
            case[field].as_str().map(str::len),
            Some(64),
            "missing result fingerprint `{field}` in {case}"
        );
    }
    for field in [
        "source_sha256",
        "binary_sha256",
        "protocol_sha256",
        "policy_sha256",
        "runner_sha256",
        "environment_sha256",
    ] {
        assert_eq!(
            case["identity"][field].as_str().map(str::len),
            Some(64),
            "missing fingerprint `{field}` in {case}"
        );
    }
}

#[test]
fn evidence_gate_corpus_exercises_the_rolestat_shape_and_fault_control() {
    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/evidence-gate-performance.py");
    let output = std::process::Command::new("python3")
        .current_dir(&workspace)
        .arg(script)
        .arg("--binary")
        .arg(common::binary())
        .arg("--profile")
        .arg("integration-test")
        .arg("--runs")
        .arg("1")
        .arg("--cold-delay-ms")
        .arg("0")
        .arg("--slow-delay-ms")
        .arg("0")
        .arg("--timeout-seconds")
        .arg("30")
        .arg("--scratch-root")
        .arg(dir.path())
        .arg("--case")
        .arg("structural-check")
        .arg("--case")
        .arg("no-provider")
        .arg("--case")
        .arg("catalogue-cold")
        .arg("--case")
        .arg("catalogue-warm")
        .arg("--case")
        .arg("slow-runner")
        .arg("--case")
        .arg("failing-runner")
        .arg("--case")
        .arg("receipt-current")
        .output()
        .expect("run evidence-gate performance corpus");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("benchmark report JSON");
    let corpus = &report["corpus"];
    assert_eq!(corpus["runner_count"], 8);
    assert_eq!(corpus["distinct_tests"], 199);
    assert_eq!(corpus["citations"], 241);
    assert_eq!(corpus["repeated_citations"], 42);
    assert_eq!(corpus["receipt_counters"]["provider_process_spawns"], 0);
    assert_eq!(corpus["receipt_counters"]["imported_providers"], 8);
    assert_eq!(corpus["receipt_counters"]["imported_outcomes"], 199);

    let cases = report["cases"].as_array().expect("case array");
    assert_eq!(cases.len(), 7);
    for case in cases {
        let provider_spawns = match case["name"].as_str() {
            Some("structural-check" | "no-provider" | "receipt-current") => 0,
            _ => 8,
        };
        assert_eq!(case["work"]["provider_process_spawns"], provider_spawns);
        assert_eq!(case["work"]["catalogue_spawns"], provider_spawns);
        assert_eq!(case["work"]["batch_spawns"], 0);
        assert_eq!(case["work"]["selected_tests"], 0);
        assert_eq!(case["work"]["citations_examined"], 241);
        assert_case_fingerprints(case);
    }
    assert_eq!(cases[0]["name"], "structural-check");
    assert_eq!(cases[1]["name"], "no-provider");
    assert_eq!(cases[2]["name"], "catalogue-cold");
    assert_eq!(cases[3]["name"], "catalogue-warm");
    assert_eq!(cases[4]["name"], "slow-runner");
    assert_eq!(cases[5]["name"], "failing-runner");
    assert_eq!(cases[6]["name"], "receipt-current");
    assert!(cases[..5].iter().all(|case| case["exit_code"] == 0));
    assert_ne!(cases[5]["exit_code"], 0);
    assert_eq!(cases[6]["exit_code"], 0);
    assert_eq!(cases[6]["work"]["provider_process_spawns"], 0);
    assert_eq!(cases[6]["work"]["receipt_imports"], 8);
    assert_eq!(cases[6]["work"]["imported_outcomes"], 199);
    assert_receipt_phases(&cases[6]);
    assert_ne!(
        cases[2]["identity"]["policy_sha256"],
        cases[5]["identity"]["policy_sha256"]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn evidence_gate_corpus_refuses_incomplete_and_mixed_sample_identities() {
    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/evidence-gate-performance.py");
    let identity = |policy: &str| {
        serde_json::json!({
            "source_sha256": "1".repeat(64),
            "binary_sha256": "2".repeat(64),
            "protocol_sha256": "3".repeat(64),
            "policy_sha256": policy,
            "runner_sha256": "4".repeat(64),
            "environment_sha256": "5".repeat(64),
        })
    };
    let run = |name: &str, value: serde_json::Value| {
        let path = dir.path().join(name);
        std::fs::write(&path, serde_json::to_vec(&value).expect("identity JSON"))
            .expect("identity fixture");
        std::process::Command::new("python3")
            .current_dir(&workspace)
            .arg(&script)
            .arg("--validate-sample-identities")
            .arg(path)
            .output()
            .expect("validate sample identities")
    };

    let stable = run(
        "stable.json",
        serde_json::json!([identity(&"5".repeat(64)), identity(&"5".repeat(64))]),
    );
    assert!(
        stable.status.success(),
        "{}",
        String::from_utf8_lossy(&stable.stderr)
    );

    let mixed = run(
        "mixed.json",
        serde_json::json!([identity(&"5".repeat(64)), identity(&"6".repeat(64))]),
    );
    assert!(!mixed.status.success(), "mixed identities were accepted");
    assert!(
        String::from_utf8_lossy(&mixed.stderr).contains("refusing mixed benchmark samples"),
        "{}",
        String::from_utf8_lossy(&mixed.stderr)
    );

    let incomplete = run(
        "incomplete.json",
        serde_json::json!([{
            "source_sha256": "1".repeat(64),
            "binary_sha256": "2".repeat(64),
        }]),
    );
    assert!(
        !incomplete.status.success(),
        "incomplete identity was accepted"
    );
    assert!(
        String::from_utf8_lossy(&incomplete.stderr).contains("sample identity is incomplete"),
        "{}",
        String::from_utf8_lossy(&incomplete.stderr)
    );

    let malformed = run(
        "malformed.json",
        serde_json::json!([identity("not-a-digest")]),
    );
    assert!(
        !malformed.status.success(),
        "malformed identity was accepted"
    );
    assert!(
        String::from_utf8_lossy(&malformed.stderr).contains("malformed SHA-256 fields"),
        "{}",
        String::from_utf8_lossy(&malformed.stderr)
    );

    let controls = std::process::Command::new("python3")
        .current_dir(&workspace)
        .arg(&script)
        .arg("--check-receipt-negative-controls")
        .output()
        .expect("exercise receipt negative controls");
    assert!(
        controls.status.success(),
        "{}",
        String::from_utf8_lossy(&controls.stderr)
    );
    let controls: serde_json::Value =
        serde_json::from_slice(&controls.stdout).expect("negative-control JSON");
    for name in [
        "missing",
        "stale",
        "malformed",
        "incomplete",
        "replayed",
        "mismatched-material",
    ] {
        assert_eq!(controls[name], name);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn evidence_gate_corpus_requires_explicit_direct_replay_opt_in() {
    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/evidence-gate-performance.py");
    let output = std::process::Command::new("python3")
        .current_dir(&workspace)
        .arg(script)
        .arg("--binary")
        .arg(common::binary())
        .arg("--case")
        .arg("direct-replay")
        .arg("--scratch-root")
        .arg(dir.path())
        .output()
        .expect("refuse an accidental direct replay");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("direct-replay") && stderr.contains("require --include-direct"),
        "{stderr}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn evidence_gate_execution_modes_have_mutation_tested_verdict_parity() {
    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/evidence-gate-performance.py");
    let digest = "a".repeat(64);
    let cases = ["direct-replay", "batched-serial", "bounded-concurrent"]
        .map(|name| {
            serde_json::json!({
                "name": name,
                "exit_code": 0,
                "verdict_sha256": digest.clone(),
            })
        })
        .to_vec();
    let run = |name: &str, cases: &[serde_json::Value]| {
        let path = dir.path().join(name);
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"cases": cases})).expect("parity JSON"),
        )
        .expect("parity fixture");
        std::process::Command::new("python3")
            .current_dir(&workspace)
            .arg(&script)
            .arg("--validate-execution-parity")
            .arg(path)
            .output()
            .expect("validate execution parity")
    };

    let equal = run("equal.json", &cases);
    assert!(
        equal.status.success(),
        "{}",
        String::from_utf8_lossy(&equal.stderr)
    );

    let mut changed_verdict = cases.clone();
    changed_verdict[1]["verdict_sha256"] = serde_json::json!("b".repeat(64));
    let changed_verdict = run("changed-verdict.json", &changed_verdict);
    assert!(!changed_verdict.status.success());
    assert!(
        String::from_utf8_lossy(&changed_verdict.stderr).contains("execution modes disagreed"),
        "{}",
        String::from_utf8_lossy(&changed_verdict.stderr)
    );

    let mut changed_exit = cases.clone();
    changed_exit[2]["exit_code"] = serde_json::json!(1);
    let changed_exit = run("changed-exit.json", &changed_exit);
    assert!(!changed_exit.status.success());
    assert!(
        String::from_utf8_lossy(&changed_exit.stderr).contains("bounded-concurrent"),
        "{}",
        String::from_utf8_lossy(&changed_exit.stderr)
    );

    let missing = run("missing.json", &cases[..2]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("bounded-concurrent"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    let mut duplicate = cases.clone();
    duplicate.push(cases[0].clone());
    let duplicate = run("duplicate.json", &duplicate);
    assert!(!duplicate.status.success());
    assert!(
        String::from_utf8_lossy(&duplicate.stderr).contains("duplicate execution case"),
        "{}",
        String::from_utf8_lossy(&duplicate.stderr)
    );

    let mut malformed = cases.clone();
    malformed[0]["verdict_sha256"] = serde_json::json!("not-a-digest");
    let malformed = run("malformed.json", &malformed);
    assert!(!malformed.status.success());
    assert!(
        String::from_utf8_lossy(&malformed.stderr).contains("cases are malformed"),
        "{}",
        String::from_utf8_lossy(&malformed.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn execution_parity_removes_only_volatile_observation_provenance() {
    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/evidence-gate-performance.py");
    let base = serde_json::json!({
        "schema": "aplan/report/v1",
        "errors": 0,
        "diagnostics": [],
        "evidence_observation": {
            "state": "current",
            "scope": "whole_plan",
            "provider_source": "direct",
            "completeness": "complete",
            "identity": "first",
            "resolution_identity": "resolution-one",
            "observed_at_epoch_seconds": 1,
            "resolver": "aplan 1",
            "strength": "test",
            "outcome": "passed",
            "detail": "complete direct observation",
            "refresh": {"shadow_compared": true}
        }
    });
    let canonicalize = |name: &str, value: &serde_json::Value| {
        let path = dir.path().join(name);
        std::fs::write(&path, serde_json::to_vec(value).expect("verdict JSON"))
            .expect("verdict fixture");
        let output = std::process::Command::new("python3")
            .current_dir(&workspace)
            .arg(&script)
            .arg("--canonicalize-verdict")
            .arg(path)
            .output()
            .expect("canonicalize verdict");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };

    let expected = canonicalize("base.json", &base);
    let mut volatile = base.clone();
    volatile["evidence_observation"]["identity"] = serde_json::json!("second");
    volatile["evidence_observation"]["resolution_identity"] = serde_json::json!("resolution-two");
    volatile["evidence_observation"]["observed_at_epoch_seconds"] = serde_json::json!(2);
    assert_eq!(
        canonicalize("volatile.json", &volatile),
        expected,
        "identity and observation time are the only removed fields"
    );

    for (index, (field, changed)) in [
        ("state", serde_json::json!("stale")),
        ("scope", serde_json::json!("selected_tasks")),
        ("provider_source", serde_json::json!("receipt")),
        ("completeness", serde_json::json!("partial")),
        ("resolver", serde_json::json!("aplan 2")),
        ("strength", serde_json::json!("commit")),
        ("outcome", serde_json::json!("failed")),
        ("detail", serde_json::json!("different claim")),
        ("refresh", serde_json::json!({"shadow_compared": false})),
    ]
    .into_iter()
    .enumerate()
    {
        let mut mutation = base.clone();
        mutation["evidence_observation"][field] = changed;
        assert_ne!(
            canonicalize(&format!("semantic-{index}.json"), &mutation),
            expected,
            "semantic observation field `{field}` was excluded from parity"
        );
    }
}

#[cfg(target_os = "linux")]
fn consume_rollout(
    workspace: &std::path::Path,
    script: &std::path::Path,
    bundle: &std::path::Path,
    scratch: &std::path::Path,
    route: &str,
    privilege: &str,
) -> std::process::Output {
    std::process::Command::new("python3")
        .current_dir(workspace)
        .arg(script)
        .arg("consume")
        .arg("--bundle")
        .arg(bundle)
        .arg("--route")
        .arg(route)
        .arg("--repository")
        .arg("owner/repository")
        .arg("--workflow")
        .arg("Evidence rollout audit")
        .arg("--run-id")
        .arg("17")
        .arg("--event")
        .arg("workflow_dispatch")
        .arg("--privilege")
        .arg(privilege)
        .arg("--runs")
        .arg("1")
        .arg("--scratch-root")
        .arg(scratch)
        .arg("--timeout-seconds")
        .arg("30")
        .output()
        .expect("consume rollout bundle")
}

#[cfg(target_os = "linux")]
fn successful_rollout(output: &std::process::Output, operation: &str) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{operation}: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("rollout report JSON")
}

#[cfg(target_os = "linux")]
fn assert_rollout_fingerprints(manifest: &serde_json::Value) {
    for field in [
        "payload_sha256",
        "binary_sha256",
        "source_sha256",
        "fixture_source_sha256",
        "protocol_sha256",
        "policy_sha256",
        "runner_sha256",
        "environment_sha256",
        "provider_environment_sha256",
    ] {
        assert_eq!(manifest[field].as_str().map(str::len), Some(64), "{field}");
    }
}

#[cfg(target_os = "linux")]
fn copy_rollout_bundle(source: &std::path::Path, destination: &std::path::Path) {
    std::fs::create_dir(destination).expect("copied bundle directory");
    for name in ["manifest.json", "payload.tar.gz"] {
        std::fs::copy(source.join(name), destination.join(name))
            .expect("copy rollout bundle entry");
    }
}

#[cfg(target_os = "linux")]
fn write_rollout_manifest(bundle: &std::path::Path, manifest: &serde_json::Value) {
    std::fs::write(
        bundle.join("manifest.json"),
        serde_json::to_vec_pretty(manifest).expect("manifest JSON"),
    )
    .expect("write rollout manifest");
}

#[cfg(target_os = "linux")]
fn replace_rollout_payload_with_hostile_archive(bundle: &std::path::Path, kind: &str) {
    const SCRIPT: &str = r#"
import hashlib
import io
import json
import pathlib
import sys
import tarfile

bundle = pathlib.Path(sys.argv[1])
kind = sys.argv[2]
payload = bundle / "payload.tar.gz"

def add(archive, name, member_type=tarfile.REGTYPE, data=b"x", linkname=""):
    member = tarfile.TarInfo(name)
    member.type = member_type
    member.linkname = linkname
    if member_type == tarfile.REGTYPE:
        member.size = len(data)
        archive.addfile(member, io.BytesIO(data))
    else:
        archive.addfile(member)

with tarfile.open(payload, "w:gz") as archive:
    if kind == "traversal":
        add(archive, "fixture/../escape")
    elif kind == "symlink":
        add(archive, "fixture/link", tarfile.SYMTYPE, linkname="outside")
    elif kind == "hardlink":
        add(archive, "fixture/link", tarfile.LNKTYPE, linkname="fixture/target")
    elif kind == "fifo":
        add(archive, "fixture/fifo", tarfile.FIFOTYPE)
    elif kind == "duplicate":
        add(archive, "fixture/value")
        add(archive, "fixture/./value")
    elif kind == "collision":
        add(archive, "fixture/parent")
        add(archive, "fixture/parent/child")
    else:
        raise SystemExit("unknown hostile archive kind")

manifest_path = bundle / "manifest.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
manifest["payload_sha256"] = hashlib.sha256(payload.read_bytes()).hexdigest()
manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
"#;
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(SCRIPT)
        .arg(bundle)
        .arg(kind)
        .output()
        .expect("construct hostile rollout archive");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
fn assert_malformed_rollout_manifests_are_refused(
    workspace: &std::path::Path,
    script: &std::path::Path,
    bundle: &std::path::Path,
    scratch: &std::path::Path,
    manifest: &serde_json::Value,
) {
    for (index, (field, value, expected)) in [
        (
            "direct.verdict_sha256",
            serde_json::json!("not-a-digest"),
            "invalid direct verdict_sha256",
        ),
        (
            "direct.exit_code",
            serde_json::json!(true),
            "invalid direct exit code",
        ),
        (
            "environment_sha256",
            serde_json::json!("not-a-digest"),
            "invalid environment_sha256",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let malformed = scratch.join(format!("malformed-{index}"));
        copy_rollout_bundle(bundle, &malformed);
        let mut changed = manifest.clone();
        if let Some((parent, child)) = field.split_once('.') {
            changed[parent][child] = value;
        } else {
            changed[field] = value;
        }
        write_rollout_manifest(&malformed, &changed);
        let refused = consume_rollout(workspace, script, &malformed, scratch, "receipt", "trusted");
        assert!(
            !refused.status.success(),
            "malformed `{field}` was accepted"
        );
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains(expected),
            "{field}: {}",
            String::from_utf8_lossy(&refused.stderr)
        );
    }
}

#[cfg(target_os = "linux")]
fn assert_hostile_rollout_archives_are_refused(
    workspace: &std::path::Path,
    script: &std::path::Path,
    bundle: &std::path::Path,
    scratch: &std::path::Path,
) {
    for (kind, expected) in [
        ("traversal", "unsafe path"),
        ("symlink", "unsafe member"),
        ("hardlink", "unsafe member"),
        ("fifo", "unsafe member"),
        ("duplicate", "repeats a path"),
        ("collision", "places a member below a file"),
    ] {
        let hostile = scratch.join(format!("hostile-{kind}"));
        copy_rollout_bundle(bundle, &hostile);
        replace_rollout_payload_with_hostile_archive(&hostile, kind);
        let refused = consume_rollout(workspace, script, &hostile, scratch, "receipt", "trusted");
        assert!(
            !refused.status.success(),
            "hostile archive `{kind}` was accepted"
        );
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains(expected),
            "{kind}: {}",
            String::from_utf8_lossy(&refused.stderr)
        );
    }
}

#[cfg(target_os = "linux")]
fn assert_stale_outer_digest_is_refused(
    workspace: &std::path::Path,
    script: &std::path::Path,
    bundle: &std::path::Path,
    scratch: &std::path::Path,
) {
    std::fs::OpenOptions::new()
        .append(true)
        .open(bundle.join("payload.tar.gz"))
        .expect("open payload")
        .write_all(b"corrupt")
        .expect("corrupt payload");
    let corrupt = consume_rollout(workspace, script, bundle, scratch, "receipt", "trusted");
    assert!(!corrupt.status.success(), "corrupt artifact was accepted");
    assert!(
        String::from_utf8_lossy(&corrupt.stderr).contains("payload_sha256 mismatch"),
        "{}",
        String::from_utf8_lossy(&corrupt.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn evidence_rollout_artifact_reconstructs_trust_and_keeps_direct_rollback() {
    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = workspace.join("ci/scripts/evidence-gate-rollout.py");
    let bundle = dir.path().join("bundle");
    let identity = [
        ("--repository", "owner/repository"),
        ("--workflow", "Evidence rollout audit"),
        ("--run-id", "17"),
        ("--event", "workflow_dispatch"),
        ("--privilege", "trusted"),
    ];
    let mut produce = std::process::Command::new("python3");
    produce
        .current_dir(&workspace)
        .arg(&script)
        .arg("produce")
        .arg("--binary")
        .arg(common::binary())
        .arg("--output")
        .arg(&bundle)
        .arg("--scratch-root")
        .arg(dir.path())
        .arg("--timeout-seconds")
        .arg("30");
    for (flag, value) in identity {
        produce.arg(flag).arg(value);
    }
    let produced = produce.output().expect("produce rollout bundle");
    let manifest = successful_rollout(&produced, "produce");
    assert_eq!(manifest["schema"], "aplan/evidence-rollout-bundle/v1");
    assert_rollout_fingerprints(&manifest);

    let receipt = consume_rollout(
        &workspace,
        &script,
        &bundle,
        dir.path(),
        "receipt",
        "trusted",
    );
    let receipt_report = successful_rollout(&receipt, "receipt consumer");
    assert_eq!(receipt_report["route"], "receipt");
    assert_eq!(receipt_report["provider_process_spawns_per_run"], 0);
    assert!(receipt_report["receipt_phase_p50_ms"].is_object());

    let direct = consume_rollout(
        &workspace,
        &script,
        &bundle,
        dir.path(),
        "direct",
        "trusted",
    );
    let direct_report = successful_rollout(&direct, "direct consumer");
    assert_eq!(direct_report["route"], "direct");
    assert_eq!(direct_report["provider_process_spawns_per_run"], 16);
    assert_eq!(
        receipt_report["verdict_sha256"], direct_report["verdict_sha256"],
        "receipt and direct routes did not preserve the same verdict"
    );

    let untrusted = consume_rollout(
        &workspace,
        &script,
        &bundle,
        dir.path(),
        "receipt",
        "untrusted",
    );
    assert!(
        !untrusted.status.success(),
        "producer privilege was forgeable"
    );
    assert!(
        String::from_utf8_lossy(&untrusted.stderr).contains("producer context does not match"),
        "{}",
        String::from_utf8_lossy(&untrusted.stderr)
    );

    assert_malformed_rollout_manifests_are_refused(
        &workspace,
        &script,
        &bundle,
        dir.path(),
        &manifest,
    );
    assert_hostile_rollout_archives_are_refused(&workspace, &script, &bundle, dir.path());

    assert_stale_outer_digest_is_refused(&workspace, &script, &bundle, dir.path());
}

#[cfg(target_os = "linux")]
#[test]
fn evidence_gate_corpus_forbids_background_git_maintenance() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let real_git = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("locate real Git");
    assert!(real_git.status.success(), "Git must be available");
    let real_git = String::from_utf8(real_git.stdout)
        .expect("Git path is UTF-8")
        .trim()
        .to_owned();
    let bin = dir.path().join("bin");
    let policy_log = dir.path().join("git-policy.log");
    std::fs::create_dir(&bin).expect("fake Git bin directory");
    let git = bin.join("git");
    std::fs::write(
        &git,
        r#"#!/bin/sh
set -eu
if [ "${GIT_CONFIG_COUNT:-}" = 2 ]; then
  [ "${GIT_CONFIG_KEY_0:-}" = gc.auto ]
  [ "${GIT_CONFIG_VALUE_0:-}" = 0 ]
  [ "${GIT_CONFIG_KEY_1:-}" = maintenance.auto ]
  [ "${GIT_CONFIG_VALUE_1:-}" = false ]
  printf '%s\n' "$*" >> "$APLAN_GIT_POLICY_LOG"
fi
exec "$APLAN_REAL_GIT" "$@"
"#,
    )
    .expect("fake Git");
    let mut permissions = std::fs::metadata(&git)
        .expect("fake Git metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&git, permissions).expect("executable fake Git");

    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .expect("fake Git PATH");
    let output = std::process::Command::new("python3")
        .current_dir(&workspace)
        .arg(workspace.join("ci/scripts/evidence-gate-performance.py"))
        .arg("--binary")
        .arg(common::binary())
        .arg("--case")
        .arg("no-provider")
        .arg("--runs")
        .arg("1")
        .arg("--scratch-root")
        .arg(dir.path())
        .env("APLAN_GIT_POLICY_LOG", &policy_log)
        .env("APLAN_REAL_GIT", real_git)
        .env("PATH", path)
        .output()
        .expect("run corpus through policy-checking Git");
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let policy_calls = std::fs::read_to_string(policy_log).expect("policy-observed Git calls");
    assert!(
        policy_calls.lines().any(|line| line.starts_with("init ")),
        "fixture creation did not carry the policy: {policy_calls}"
    );
    assert!(
        policy_calls.lines().any(|line| line == "rev-parse HEAD"),
        "source observation did not carry the policy: {policy_calls}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn evidence_gate_corpus_refuses_source_that_moves_during_a_sample() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = fixture();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = dir.path().join("source");
    std::fs::create_dir(&source).expect("source fixture");
    let marker = source.join("tracked");
    std::fs::write(&marker, "before\n").expect("source marker");
    for args in [
        &["init", "-q"][..],
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=f@invalid",
            "add",
            ".",
        ][..],
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=f@invalid",
            "commit",
            "-q",
            "-m",
            "fixture",
        ][..],
    ] {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&source)
            .output()
            .expect("prepare source fixture");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let wrapper = dir.path().join("moving-source-aplan");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\nprintf 'after\\n' >> \"$SOURCE_MARKER\"\nexec \"$REAL_APLAN\" \"$@\"\n",
    )
    .expect("moving-source wrapper");
    let mut permissions = std::fs::metadata(&wrapper)
        .expect("wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&wrapper, permissions).expect("executable wrapper");

    let output = std::process::Command::new("python3")
        .current_dir(&source)
        .arg(workspace.join("ci/scripts/evidence-gate-performance.py"))
        .arg("--binary")
        .arg(wrapper)
        .arg("--case")
        .arg("structural-check")
        .arg("--runs")
        .arg("1")
        .arg("--scratch-root")
        .arg(dir.path())
        .env("REAL_APLAN", common::binary())
        .env("SOURCE_MARKER", marker)
        .output()
        .expect("run moving-source benchmark");
    assert!(!output.status.success(), "moving source was accepted");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Git source changed while its sample was running"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
