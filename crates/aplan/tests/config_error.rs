//! An unknown config key says which of its two causes it is (`A1-068`).
//!
//! The first adopter added `test.covers` to `.aplan.yml` before staging the
//! binary that knows it, and their pre-push hook runs `aplan`, so the whole gate
//! refused to load and they could not push:
//!
//! ```text
//! aplan: loading .aplan.yml: could not parse `.aplan.yml`: unknown field `covers`,
//! expected one of `name`, `dir`, `list`, `run` at line 10 column 1
//! ```
//!
//! Rejecting unknown keys is deliberate and stays. What was missing is that the
//! sentence is *identical* for a misspelling and for a key from a newer `aplan`,
//! and the one fact that separates them, the running version, was the one thing
//! the tool did not print (pre-public adoption evidence).

use std::process::Command;

use crate::common;
use common::aplan;

/// An empty plan and the given `.aplan.yml`.
fn repo(config: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    std::fs::create_dir_all(dir.path().join("aplan/tasks")).expect("plan dir");
    std::fs::write(dir.path().join(".aplan.yml"), config).expect("config");
    dir
}

/// The version, unconditionally, because it is what tells the two cases apart.
#[test]
fn an_unknown_key_names_the_running_version_and_what_it_accepts() {
    let dir = repo("zzzzzzzz: 1\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "a config problem is a usage error:\n{out}");
    assert!(out.contains("unknown key `zzzzzzzz`"), "{out}");
    assert!(
        out.contains(&format!("this is aplan {}", env!("CARGO_PKG_VERSION"))),
        "the running version is the fact that settles it:\n{out}"
    );
    assert!(
        out.contains("config key(s); `aplan facts` lists them"),
        "{out}"
    );
    assert!(
        out.contains("`aplan facts --identity`"),
        "same-version dev builds need build identity guidance:\n{out}"
    );
    assert!(
        out.contains("reads exactly like a typo from here"),
        "the note has to say *why* the version matters:\n{out}"
    );
}

/// And a plain typo still reads as a typo. The note must not turn every
/// misspelling into a meditation on binary versions, so the correction leads.
#[test]
fn a_typo_still_reads_as_a_typo_and_gets_its_correction() {
    let dir = repo("requre_effort: false\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("unknown key `requre_effort` (did you mean `require_effort`?)"),
        "{out}"
    );
    // The suggestion comes before the version note, in that order, on the line an
    // adopter reads first.
    let suggestion = out.find("did you mean").expect("a suggestion");
    let note = out.find("this is aplan").expect("the version note");
    assert!(
        suggestion < note,
        "the version note buried the correction:\n{out}"
    );
}

/// The pilot's own case, one release on: a nested key the running build does not
/// know. Nothing about `test.covrs` is special except that it is where the
/// original failure was, and serde reported it as a bare `covers` with no path.
#[test]
fn a_nested_unknown_key_is_named_with_its_path() {
    let dir = repo("test:\n  name: x\n  list: [a]\n  run: [b]\n  covrs: [src/**]\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("unknown key `test.covrs` (did you mean `test.covers`?)"),
        "the key must be named with its path, not bare:\n{out}"
    );
}

/// Every offending key at once. An adopter who fixes one, re-runs, and meets the
/// next has been told half the answer twice.
#[test]
fn several_unknown_keys_are_all_named() {
    let dir = repo("polciy: strict\nzzzzzzzz: 1\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("unknown keys"), "the plural:\n{out}");
    assert!(out.contains("`polciy`"), "{out}");
    assert!(out.contains("`zzzzzzzz`"), "{out}");
}

/// A free-form mapping is the adopter's own vocabulary. Reporting it would make
/// a correct config fail with twenty invented mistakes, which is worse than the
/// defect this task fixes.
#[test]
fn a_declared_tag_vocabulary_is_not_reported_as_unknown_keys() {
    let dir = repo("tag_families:\n  epic: [epic-a, epic-b]\n  kind: [kind-r]\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 0, "an adopter's own tag names were rejected:\n{out}");
}

/// A syntax error has one cause, so it must not carry a note about two. The
/// note would be noise on a file that never parsed and has no keys to judge.
#[test]
fn a_syntax_error_is_still_a_syntax_error_and_carries_no_version_note() {
    let dir = repo("require_effort: [not a bool\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("could not parse"), "{out}");
    assert!(!out.contains("unknown key"), "{out}");
    assert!(!out.contains("this is aplan"), "{out}");
}

/// Every command that reads a plan gets the same answer, and `--root` does not
/// change it.
///
/// Writing this found that it did. `ready`, `summary`, `tasks`, `critical-path`
/// and `graph` do not read config keys of their own, so they only met
/// `.aplan.yml` while resolving the root, and passing `--root` returned before
/// that. The same five commands exit 2 without the flag, so `--root` was
/// deciding the verdict: a confident answer, exit 0, from a repository whose
/// configuration could not load. That is the failure this repository already
/// names for a plan that did not load, one input over.
#[test]
fn every_command_reports_it_the_same_way_with_root_given() {
    let dir = repo("requre_effort: false\n");
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .output()
        .expect("git init");
    for args in [
        vec!["check", "--root", "aplan"],
        vec!["verify", "--root", "aplan"],
        vec!["ready", "--root", "aplan"],
        vec!["roadmap", "--root", "aplan"],
        vec!["summary", "--root", "aplan"],
        vec!["tasks", "--root", "aplan"],
        vec!["critical-path", "--root", "aplan"],
        vec!["graph", "--root", "aplan"],
    ] {
        let (out, code) = aplan(dir.path(), &args);
        assert_eq!(code, 2, "`{}` disagreed:\n{out}", args[0]);
        assert!(
            out.contains("did you mean `require_effort`?") && out.contains("this is aplan"),
            "`{}` reported it differently:\n{out}",
            args[0]
        );
    }
}

/// The control for the test above: without `--root` the same five commands
/// already failed, which is how the flag was shown to be the thing deciding it
/// rather than the command.
#[test]
fn the_same_commands_fail_without_the_flag_too() {
    let dir = repo("requre_effort: false\n");
    for cmd in ["ready", "summary", "tasks", "critical-path", "graph"] {
        let (out, code) = aplan(dir.path(), &[cmd]);
        assert_eq!(code, 2, "`{cmd}` disagreed:\n{out}");
    }
}

// ---------------------------------------------------------------------------
// What the audit of this task found. Reporting the unknown keys *replaced* the
// error rather than completing it, so whatever else was wrong with the file
// waited for the next run: the same half-an-answer-twice this task set out to
// stop, one error along.
// ---------------------------------------------------------------------------

/// `serde` stops at its first error, so a file with an unknown key above a type
/// error reports the key, and replacing that message with a better list of keys
/// still left the type error undiscovered. Sniffing `serde`'s sentence would not
/// have found it either, for exactly that reason.
#[test]
fn a_type_error_behind_an_unknown_key_is_reported_at_the_same_time() {
    let dir = repo("zzzzzzzz: 1\nrequire_effort: 5\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("unknown key `zzzzzzzz`"), "{out}");
    assert!(
        out.contains("once that is fixed the file still fails"),
        "{out}"
    );
    assert!(out.contains("expected a boolean"), "{out}");
}

/// Parsing is only half of loading. A runner with no `{test}` placeholder is
/// caught by a semantic check after the parse, and it is as much "what you meet
/// next" as a type error is, so the probe runs the same checks the load does.
#[test]
fn a_semantic_error_behind_an_unknown_key_is_reported_too() {
    let dir = repo("zzzzzzzz: 1\ntest:\n  name: x\n  list: [a]\n  run: [b]\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("unknown key `zzzzzzzz`"), "{out}");
    assert!(out.contains("must contain `{test}`"), "{out}");
}

/// The probe reports what the adopter will meet *after taking this advice*, so a
/// key with a correction is renamed rather than dropped. Dropping a misspelled
/// `test.lst` leaves a runner with no `list` and invents a missing-field error
/// the real fix never produces.
#[test]
fn a_correctable_typo_does_not_invent_a_missing_field() {
    // The runner is otherwise valid, including the `{test}` placeholder: a
    // fixture with a second real fault would prove nothing about the invented
    // one.
    let dir = repo("test:\n  name: x\n  lst: [a]\n  run: [b, '{test}']\n");
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("unknown key `test.lst` (did you mean `test.list`?)"),
        "{out}"
    );
    assert!(
        !out.contains("still fails"),
        "correcting the spelling is the whole fix, and it claimed otherwise:\n{out}"
    );
    assert!(!out.contains("missing field"), "{out}");
}

/// The position `serde` reports belongs to the document with the unknown keys
/// already fixed, which is not the file the reader has. Quoting `line 1` for
/// something on line 2 of their file would be a confident wrong answer.
#[test]
fn the_second_error_does_not_claim_a_line_number_from_a_file_nobody_has() {
    let dir = repo("zzzzzzzz: 1\nrequire_effort: 5\n");
    let (out, _) = aplan(dir.path(), &["check", "--root", "aplan"]);
    let tail = out
        .split("still fails:")
        .nth(1)
        .expect("the second error")
        .to_string();
    assert!(
        !tail.contains(" at line "),
        "it quoted a position from the cleaned document:\n{tail}"
    );
}

/// The remedy the note names has to work in the state that produced the note.
/// `facts`, `facts --identity`, and `--version` read no plan and must stay
/// reachable when the config or task frontmatter cannot load, or the advice
/// sends the reader to a second failure (`A1-068`, `A2-194`).
#[test]
fn the_commands_the_note_names_still_work_with_the_config_broken() {
    let dir = repo("requre_effort: false\n");
    for args in [
        vec!["facts"],
        vec!["facts", "--identity"],
        vec!["--version"],
    ] {
        let (out, code) = aplan(dir.path(), &args);
        assert_eq!(
            code,
            0,
            "`{}` is unreachable when it is needed:\n{out}",
            args.join(" ")
        );
    }
    // And `facts` really does list the keys the note promises, while
    // `facts --identity` prints valid JSON build identity.
    let (out, _) = aplan(dir.path(), &["facts"]);
    let doc: serde_json::Value = serde_json::from_str(&out).expect("facts json");
    assert!(
        doc["config_keys"]
            .as_array()
            .expect("config_keys")
            .iter()
            .any(|k| k == "require_effort"),
        "{out}"
    );
    let (id_out, id_code) = aplan(dir.path(), &["facts", "--identity"]);
    assert_eq!(id_code, 0, "{id_out}");
    let id_doc: serde_json::Value = serde_json::from_str(&id_out).expect("identity json");
    assert_eq!(id_doc["schema"], "aplan/tool-identity/v1");
    assert!(id_doc["facts_sha256"].is_string(), "{id_out}");
}

/// `A2-194`: unknown task frontmatter fields report `APLAN001`, name the
/// unknown field cleanly, point to `aplan schema`, `aplan facts`, and
/// `aplan facts --identity`, and leave `aplan facts --identity` working.
#[test]
fn unknown_task_frontmatter_field_points_to_facts_identity() {
    let dir = repo("root: aplan\n");
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: Unknown field\nphase: 1\nstatus: todo\n\
         effort_days: 1\nfuture_task_key: true\n---\n\nbody\n",
    )
    .expect("task");

    let (out, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("APLAN001"), "{out}");
    assert!(
        out.contains("unknown frontmatter field `future_task_key`"),
        "{out}"
    );
    assert!(out.contains("aplan schema"), "{out}");
    assert!(out.contains("aplan facts"), "{out}");
    assert!(out.contains("aplan facts --identity"), "{out}");

    let (id_out, id_code) = aplan(dir.path(), &["facts", "--identity"]);
    assert_eq!(id_code, 0, "{id_out}");
    let id_doc: serde_json::Value = serde_json::from_str(&id_out).expect("identity json");
    assert_eq!(id_doc["schema"], "aplan/tool-identity/v1");
}

/// `A2-193` / `ADR-0068`: pre-schema `requires` fails closed (exit 2) BEFORE
/// reporting unknown future config keys, names both declared requirements and
/// running build identity, and keeps inspection/recovery commands reachable.
#[test]
fn requires_mismatch_fails_closed_before_unknown_keys_and_leaves_inspection_reachable() {
    let dir = repo(
        "requires:\n  min_version: '99.0.0'\n  capabilities: [future.capability]\n\
         future_v99_key: true\n",
    );
    let (out, code) = aplan(dir.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "build-contract mismatch must exit 2:\n{out}");
    assert!(out.contains("does not satisfy `requires`"), "{out}");
    assert!(out.contains("99.0.0"), "{out}");
    assert!(out.contains("future.capability"), "{out}");
    assert!(
        out.contains(&format!("version={}", env!("CARGO_PKG_VERSION"))),
        "{out}"
    );
    assert!(out.contains("source="), "{out}");
    assert!(out.contains("facts_sha256="), "{out}");
    assert!(out.contains("docs/upgrading.md"), "{out}");
    assert!(
        !out.contains("future_v99_key"),
        "pre-schema requires must fail BEFORE unknown future keys:\n{out}"
    );

    // Every inspection and recovery surface must remain reachable.
    for args in [
        vec!["facts"],
        vec!["facts", "--identity"],
        vec!["facts", "--require", "requires"],
        vec!["--version"],
        vec!["schema"],
        vec!["--no-config", "check", "--root", "aplan"],
    ] {
        let (ok_out, ok_code) = aplan(dir.path(), &args);
        assert_eq!(
            ok_code,
            0,
            "`{}` must remain reachable while `requires` refuses the build:\n{ok_out}",
            args.join(" ")
        );
    }

    // MCP server stays alive for `aplan_identity` and refuses plan tools with
    // the build-contract mismatch message.
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let id_call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"aplan_identity","arguments":{}}}"#;
    let check_call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"aplan_check","arguments":{}}}"#;
    let mut child = common::command(dir.path(), &["mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn mcp");
    {
        use std::io::Write as _;
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{init}").expect("write init");
        writeln!(stdin, "{id_call}").expect("write id");
        writeln!(stdin, "{check_call}").expect("write check");
    }
    let output = child.wait_with_output().expect("mcp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("aplan/tool-identity/v1"),
        "aplan_identity must still answer when `requires` fails:\n{stdout}"
    );
    assert!(
        stdout.contains("does not satisfy `requires`") && stdout.contains("99.0.0"),
        "MCP plan tools must fail closed with the build-contract refusal:\n{stdout}"
    );
    assert!(
        !stdout.contains("reads exactly like a typo from an older one"),
        "MCP requires refusal must not append the generic unknown-key typo footer:\n{stdout}"
    );
}

/// `A2-193` / `A2-196`: verify `requires` end-to-end across the `build.tui`
/// compile-time feature boundary (default vs `--no-default-features`),
/// explicit-root MCP `aplan_check(root: ...)` over stdio, and revision-addressed
/// `summary --rev HEAD`.
#[test]
fn requires_enforced_across_build_tui_feature_explicit_mcp_root_and_git_rev() {
    // 1. `requires.capabilities: [build.tui]` tracks whether this binary was
    // compiled with the `tui` feature.
    let tui_req = repo("root: aplan\nrequires:\n  capabilities: [build.tui]\n");
    let (tui_out, tui_code) = aplan(tui_req.path(), &["check"]);
    #[cfg(feature = "tui")]
    assert_eq!(
        tui_code, 0,
        "default build must satisfy build.tui:\n{tui_out}"
    );
    #[cfg(not(feature = "tui"))]
    {
        assert_eq!(
            tui_code, 2,
            "headless build must fail closed on build.tui:\n{tui_out}"
        );
        assert!(tui_out.contains("build.tui"), "{tui_out}");
    }

    // 2. Explicit-root MCP `aplan_check(root: ...)` over stdio enforces
    // `requires` on the target repository before unknown future keys even when
    // the server launched in an unconstrained repository.
    let launch_repo = repo("root: aplan\n");
    let strict_repo = repo("requires:\n  min_version: '99.0.0'\nfuture_v99_key: true\n");
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let explicit_call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "aplan_check",
            "arguments": { "root": strict_repo.path().to_string_lossy() }
        }
    });
    let mut child = common::command(launch_repo.path(), &["mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn mcp");
    {
        use std::io::Write as _;
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{init}").expect("write init");
        writeln!(stdin, "{explicit_call}").expect("write explicit call");
    }
    let mcp_out = child.wait_with_output().expect("mcp exits");
    let stdout = String::from_utf8_lossy(&mcp_out.stdout);
    assert!(
        stdout.contains("does not satisfy `requires`") && stdout.contains("99.0.0"),
        "explicit-root MCP call must refuse unsatisfied requires:\n{stdout}"
    );
    assert!(
        !stdout.contains("future_v99_key"),
        "explicit-root MCP call must refuse requires before unknown keys:\n{stdout}"
    );

    // 3. Revision-addressed config reads (`summary --at HEAD`) enforce
    // `requires` on committed `.aplan.yml` before unknown future keys.
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.invalid"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "committed requires"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(strict_repo.path())
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }
    let (rev_out, rev_code) = aplan(strict_repo.path(), &["summary", "--at", "HEAD"]);
    assert_eq!(
        rev_code, 2,
        "`summary --at HEAD` must fail closed on committed requires:\n{rev_out}"
    );
    assert!(rev_out.contains("does not satisfy `requires`"), "{rev_out}");
    assert!(!rev_out.contains("future_v99_key"), "{rev_out}");
}

/// `A2-193`: `requires.min_version` refuses pre-release strings and
/// `requires.binary_sha256` is explicitly rejected per ADR-0068.
#[test]
fn requires_refuses_prerelease_min_version_and_binary_sha256() {
    let dev_pin = repo("requires:\n  min_version: '0.8.0-dev'\n");
    let (out, code) = aplan(dev_pin.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("bare released `X.Y.Z`"), "{out}");
    assert!(out.contains("requires.capabilities"), "{out}");
    assert!(out.contains("requires.source"), "{out}");
    assert!(out.contains("requires.facts_sha256"), "{out}");

    let bin_pin = repo(
        "requires:\n  binary_sha256: '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'\n",
    );
    let (out, code) = aplan(bin_pin.path(), &["check", "--root", "aplan"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("binary_sha256"), "{out}");
    assert!(out.contains("docs/upgrading.md"), "{out}");
    assert!(out.contains("external"), "{out}");
}

/// Proof for `config.test_runner_reports_all_missing_and_misplaced_fields`
/// (`A2-037`) and `config.test_runner_errors_name_expected_shape_and_runner_path`
/// (`A2-038`), reproducing every row of issue #34 end to end through `aplan check`.
#[test]
fn issue_34_test_runners_sequence_reaches_valid_config_in_one_step() {
    // Row 1: one error names all three missing fields with their full paths and
    // shapes so the author can jump straight to a valid config in one edit.
    let row1 = repo("test:\n  runners:\n    - name: unit\n");
    let (out1, code1) = aplan(row1.path(), &["check", "--root", "aplan"]);
    assert_eq!(code1, 2, "{out1}");
    for required in [
        "test.schedule",
        "{capacity: 1}",
        "test.runners[0].list",
        "test.runners[0].run",
        "{test}",
    ] {
        assert!(
            out1.contains(required),
            "row 1 omitted `{required}`:\n{out1}"
        );
    }

    // Applying every requirement from row 1 reaches a valid config in one step.
    std::fs::write(
        row1.path().join(".aplan.yml"),
        "test:\n  schedule:\n    capacity: 1\n  runners:\n    - name: unit\n      list: ['true']\n      run: ['true', '{test}']\n",
    )
    .expect("fixed config");
    let (ok_out, ok_code) = aplan(row1.path(), &["check", "--root", "aplan"]);
    assert_eq!(ok_code, 0, "one-step fix must succeed:\n{ok_out}");

    // Row 2: scalar `list: "true"` describes the wanted argv list shape in
    // configuration terms (never Rust's `expected a sequence`) and still reports
    // the remaining missing fields.
    let row2 = repo("test:\n  runners:\n    - name: unit\n      list: \"true\"\n");
    let (out2, code2) = aplan(row2.path(), &["check", "--root", "aplan"]);
    assert_eq!(code2, 2, "{out2}");
    assert!(
        out2.contains("test.runners[0].list: expected a command argv list of strings"),
        "{out2}"
    );
    assert!(
        out2.contains("test.schedule") && out2.contains("test.runners[0].run"),
        "{out2}"
    );
    assert!(!out2.contains("expected a sequence"), "{out2}");

    // Row 5: misplaced `schedule` inside a runner names `test.schedule` instead
    // of misreporting an unknown key from a newer binary.
    let row5 = repo(
        "test:\n  runners:\n    - name: unit\n      list: ['true']\n      run: ['true']\n      schedule: nightly\n",
    );
    let (out5, code5) = aplan(row5.path(), &["check", "--root", "aplan"]);
    assert_eq!(code5, 2, "{out5}");
    assert!(
        out5.contains("test.runners[0].schedule: misplaced key")
            && out5.contains("`test.schedule`"),
        "{out5}"
    );
    assert!(!out5.contains("unknown key"), "{out5}");
    assert!(!out5.contains("this is aplan"), "{out5}");

    // Row 6: `test.schedule: nightly` states the mapping shape in config terms,
    // never `expected struct TestSchedule`.
    let row6 = repo(
        "test:\n  schedule: nightly\n  runners:\n    - name: unit\n      list: ['true']\n      run: ['true']\n",
    );
    let (out6, code6) = aplan(row6.path(), &["check", "--root", "aplan"]);
    assert_eq!(code6, 2, "{out6}");
    assert!(
        out6.contains("test.schedule: expected a mapping such as {capacity: 1}"),
        "{out6}"
    );
    assert!(!out6.contains("TestSchedule"), "{out6}");

    // Row 7: runner validation names `test.runners[0].run`, never `test.run`.
    let row7 = repo(
        "test:\n  schedule:\n    capacity: 1\n  runners:\n    - name: unit\n      list: ['true']\n      run: ['true']\n",
    );
    let (out7, code7) = aplan(row7.path(), &["check", "--root", "aplan"]);
    assert_eq!(code7, 2, "{out7}");
    assert!(out7.contains("`test.runners[0].run`"), "{out7}");
    assert!(!out7.contains("`test.run`"), "{out7}");
}
