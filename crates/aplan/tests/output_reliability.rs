//! Output reliability and machine-format contract tests (`A2-096`, `A2-095`, `A2-182`).

use crate::common;

use std::path::Path;

fn tmp(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("aplan-out-{name}-"))
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_task(dir: &Path, id: &str, body: &str) {
    let tasks = dir.join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("mkdir");
    std::fs::write(tasks.join(format!("{id}.md")), body).expect("write task");
}

fn parse_single_json_to_eof(raw: &str) -> serde_json::Value {
    let mut de = serde_json::Deserializer::from_str(raw);
    let value: serde_json::Value = serde::Deserialize::deserialize(&mut de)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\n--- stdout ---\n{raw}"));
    de.end().unwrap_or_else(|e| {
        panic!("stdout contains trailing data after JSON document: {e}\n--- stdout ---\n{raw}")
    });
    value
}

/// Proof for `agent.output_is_single_json_document`: `aplan check --format agent`
/// and `aplan verify --format agent` emit exactly one valid JSON document on
/// stdout across zero, one, and many findings even when unchecked evidence is
/// present, carrying `evidence_unresolved` inside the JSON object with no prose
/// appended.
#[test]
fn agent_format_emits_one_valid_json_document_across_finding_counts_and_unchecked_evidence() {
    // 1. Zero findings + unchecked evidence on `check --format agent`.
    let zero_dir = tmp("agent-zero");
    std::fs::write(zero_dir.path().join(".aplan.yml"), "root: aplan\n").expect("cfg");
    write_task(
        zero_dir.path(),
        "T-001",
        "---\nid: T-001\ntitle: done with commit\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [src/lib.rs]\nevidence:\n  - kind: commit\n    ref: 0123456\n---\n\nbody\n",
    );
    let out0 = common::run(zero_dir.path(), &["check", "--format", "agent"]);
    assert_eq!(out0.code, 0, "clean check failed: {}", out0.stderr);
    let doc0 = parse_single_json_to_eof(&out0.stdout);
    assert_eq!(doc0["schema"], "aplan/agent-report/v1");
    assert_eq!(doc0["diagnostics"].as_array().expect("array").len(), 0);
    assert_eq!(doc0["evidence_unresolved"]["citations"], 1);
    assert_eq!(doc0["evidence_unresolved"]["tasks"], 1);
    assert_eq!(doc0["evidence_unresolved"]["resolved_by"], "verify");
    assert!(
        !out0.stderr.contains("check` is structural"),
        "agent format should not emit prose note on stderr: {}",
        out0.stderr
    );

    // 2. One finding + unchecked evidence.
    write_task(
        zero_dir.path(),
        "T-002",
        "---\nid: T-002\ntitle: one bad blocker\nphase: 1\nstatus: todo\neffort_days: 1\n\
         blocked_by: [GHOST-1]\n---\n\nbody\n",
    );
    let out1 = common::run(zero_dir.path(), &["check", "--format", "agent"]);
    assert_eq!(out1.code, 1, "one finding should exit 1");
    let doc1 = parse_single_json_to_eof(&out1.stdout);
    assert_eq!(doc1["diagnostics"].as_array().expect("array").len(), 1);
    assert_eq!(doc1["evidence_unresolved"]["citations"], 1);
    assert_eq!(doc1["evidence_unresolved"]["resolved_by"], "verify");
    assert!(
        !out1.stderr.contains("check` is structural"),
        "agent format should not emit prose note on stderr: {}",
        out1.stderr
    );

    // 3. Many findings + unchecked evidence.
    for i in 3..=12 {
        write_task(
            zero_dir.path(),
            &format!("T-{i:03}"),
            &format!(
                "---\nid: T-{i:03}\ntitle: bad blocker {i}\nphase: 1\nstatus: todo\neffort_days: 1\n\
                 blocked_by: [GHOST-{i}]\n---\n\nbody\n"
            ),
        );
    }
    let out_many = common::run(zero_dir.path(), &["check", "--format", "agent"]);
    assert_eq!(out_many.code, 1, "many findings should exit 1");
    let doc_many = parse_single_json_to_eof(&out_many.stdout);
    assert_eq!(doc_many["diagnostics"].as_array().expect("array").len(), 11);
    assert_eq!(doc_many["evidence_unresolved"]["citations"], 1);
    assert_eq!(doc_many["evidence_unresolved"]["tasks"], 1);
    assert_eq!(doc_many["evidence_unresolved"]["resolved_by"], "verify");
    assert!(
        !out_many.stderr.contains("check` is structural"),
        "agent format should not emit prose note on stderr: {}",
        out_many.stderr
    );

    // 4. `verify --format agent` with unconfigured evidence kinds also emits one valid JSON document.
    let v_dir = tmp("agent-verify");
    std::fs::write(v_dir.path().join(".aplan.yml"), "root: aplan\n").expect("cfg");
    write_task(
        v_dir.path(),
        "V-001",
        "---\nid: V-001\ntitle: unconfigured test\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [src/lib.rs]\nevidence:\n  - kind: test\n    ref: suite::one\n---\n\nbody\n",
    );
    git(v_dir.path(), &["init", "-q", "."]);
    git(v_dir.path(), &["config", "user.email", "t@example.com"]);
    git(v_dir.path(), &["config", "user.name", "t"]);
    git(v_dir.path(), &["add", "-A"]);
    git(v_dir.path(), &["commit", "-q", "-m", "init"]);

    let v_out = common::run(v_dir.path(), &["verify", "--format", "agent"]);
    assert_eq!(v_out.code, 0, "verify failed: {}", v_out.stderr);
    let v_doc = parse_single_json_to_eof(&v_out.stdout);
    assert_eq!(v_doc["schema"], "aplan/agent-report/v1");
    assert_eq!(v_doc["evidence_unresolved"]["citations"], 1);
    assert_eq!(v_doc["evidence_unresolved"]["tasks"], 1);
    assert!(
        !v_out.stderr.contains("nothing is configured to check"),
        "agent verify should not emit prose note on stderr: {}",
        v_out.stderr
    );
}

/// Regression test for `A2-182`: refused mutation commands that do not accept
/// `--format` must never advise passing `--format agent`, while `check` and
/// `verify` human output keep their existing `--format agent` hint.
#[test]
fn refused_close_with_multiple_findings_never_advises_format_agent() {
    let dir = tmp("refused-close-no-format-hint");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("cfg");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn f() {}\n").expect("lib");
    write_task(
        dir.path(),
        "T-001",
        "---\nid: T-001\ntitle: close me\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [src/lib.rs]\n---\n\nbody\n",
    );
    git(dir.path(), &["init", "-q", "."]);
    git(dir.path(), &["config", "user.email", "t@example.com"]);
    git(dir.path(), &["config", "user.name", "t"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);

    // Two unresolvable commit citations trigger two APLAN011 findings during close.
    let close_out = common::run(
        dir.path(),
        &[
            "close",
            "T-001",
            "--evidence",
            "commit:deadbeef0001",
            "--evidence",
            "commit:deadbeef0002",
        ],
    );
    assert_eq!(close_out.code, 1, "close should refuse bad citations");
    let combined = format!("{}{}", close_out.stdout, close_out.stderr);
    assert!(
        !combined.contains("--format agent"),
        "refused `close` advised `--format agent` which it does not accept:\n{combined}"
    );
    assert_eq!(
        close_out.stdout.matches("fix:").count(),
        2,
        "refused `close` should show every finding's fix instead of suppressing:\n{}",
        close_out.stdout
    );

    // Meanwhile `aplan check` human output still keeps its `--format agent` hint.
    write_task(
        dir.path(),
        "T-002",
        "---\nid: T-002\ntitle: bad 1\nphase: 1\nstatus: todo\neffort_days: 1\n\
         blocked_by: [GHOST-A]\n---\n\nbody\n",
    );
    write_task(
        dir.path(),
        "T-003",
        "---\nid: T-003\ntitle: bad 2\nphase: 1\nstatus: todo\neffort_days: 1\n\
         blocked_by: [GHOST-B]\n---\n\nbody\n",
    );
    let check_out = common::run(dir.path(), &["check"]);
    assert_eq!(check_out.code, 1);
    assert!(
        check_out.stdout.contains("--format agent"),
        "`check` human output should keep its `--format agent` hint:\n{}",
        check_out.stdout
    );
}

/// End-to-end test for `A2-095`: driving a report larger than the OS pipe
/// buffer (> 128 KiB) through an `O_NONBLOCK` pipe produces byte-for-byte
/// identical output and preserves the validation verdict instead of failing
/// with `WouldBlock`.
#[cfg(unix)]
#[test]
fn large_report_over_nonblocking_pipe_matches_blocking_output_byte_for_byte() {
    let dir = tmp("nonblocking-pipe");
    std::fs::write(dir.path().join(".aplan.yml"), "root: aplan\n").expect("cfg");
    // Generate enough findings so both human and agent output exceed 128 KiB
    // (well over Linux's 64 KiB pipe buffer).
    for i in 1..=600 {
        write_task(
            dir.path(),
            &format!("NB-{i:04}"),
            &format!(
                "---\nid: NB-{i:04}\ntitle: nonblocking output stress task number {i:04} with extra text for pipe sizing\n\
                 phase: 1\nstatus: todo\neffort_days: 1\nblocked_by: [MISSING-TARGET-{i:04}]\n---\n\nbody\n"
            ),
        );
    }

    let bin = common::command(dir.path(), &["check"])
        .get_program()
        .to_os_string();
    let driver = r#"
import os, subprocess, sys, time

cwd = sys.argv[1]
out_path = sys.argv[2]
cmd = sys.argv[3:]

r_fd, w_fd = os.pipe()
os.set_blocking(w_fd, False)
proc = subprocess.Popen(cmd, cwd=cwd, stdout=w_fd, stderr=subprocess.PIPE)
os.close(w_fd)

chunks = []
while True:
    data = os.read(r_fd, 4096)
    if not data:
        break
    chunks.append(data)
    time.sleep(0.001)
os.close(r_fd)
_, stderr = proc.communicate()
with open(out_path, "wb") as f:
    f.write(b"".join(chunks))
sys.stderr.buffer.write(stderr)
sys.exit(proc.returncode)
"#;

    for (idx, args) in [&["check"][..], &["check", "--format", "agent"][..]]
        .into_iter()
        .enumerate()
    {
        let baseline = common::run(dir.path(), args);
        assert_eq!(baseline.code, 1, "baseline verdict should be 1");
        assert!(
            baseline.stdout.len() > 100_000,
            "report should exceed a 64 KiB pipe buffer, got {} bytes",
            baseline.stdout.len()
        );

        let captured_path = dir.path().join(format!("captured-{idx}.out"));
        let mut py = std::process::Command::new("python3");
        py.arg("-c")
            .arg(driver)
            .arg(dir.path())
            .arg(&captured_path)
            .arg(&bin)
            .args(args);
        let out = py.output().expect("run python3 non-blocking pipe driver");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(1),
            "verdict must come from validation (exit 1), not WouldBlock (exit 2); stderr:\n{stderr}"
        );
        let captured = std::fs::read(&captured_path).expect("read captured output");
        assert_eq!(
            captured.as_slice(),
            baseline.stdout.as_bytes(),
            "non-blocking pipe output differed from blocking baseline for {args:?}"
        );
    }
}
