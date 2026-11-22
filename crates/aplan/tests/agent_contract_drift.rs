//! `APLAN041` reaches the surfaces a repository actually gates with (`A1-273`).
//!
//! The rule lives in `aplan-core` and compares a block the *caller* renders, so
//! the half that can go wrong is the wiring: a surface that never fills
//! `agents_block` reports nothing and looks exactly like a repository whose
//! contract is current. That is the shape of both known MCP divergences
//! (`A1-100`, `A1-101`), and `mcp_parity.rs` says in its own header that a
//! defect in how `aplan mcp` builds its `Options` would pass there.
//!
//! So each surface is driven end to end over one fixture, whose `AGENTS.md`
//! carries a block that does not say what the plan's `.aplan.yml` says.

use std::path::Path;

use crate::common;
use common::aplan;

/// A plan whose `AGENTS.md` block is stale against its own config.
///
/// `require_min_strength` is the key `p00002` changed, and the block claims the
/// policy it had the day before: the case this rule exists for, in the shape it
/// was found in.
fn stale_repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::write(
        root.join(".aplan.yml"),
        "root: aplan\nevidence:\n  require_min_strength: [test]\n",
    )
    .expect("config");
    std::fs::write(
        root.join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: A task in a stale-contract fixture\nphase: 1\n\
         status: todo\nowner: kaminsod\neffort_days: 1\n---\n\n## Why\n\nSomething.\n",
    )
    .expect("task");
    std::fs::write(
        root.join("AGENTS.md"),
        format!(
            "# Fixture\n\nits own prose\n\n{}\n\n## The plan is the source of truth\n\n\
             - A `done` task must cite `commit`, `file` or `test` evidence.\n\n{}\n\nmore prose\n",
            aplan_core::contract::BEGIN,
            aplan_core::contract::END
        ),
    )
    .expect("AGENTS.md");
    dir
}

/// Bring the file up to date the way the finding says to.
fn regenerate(dir: &Path) {
    let (out, code) = aplan(dir, &["export-agents-md", "--write", "AGENTS.md"]);
    assert_eq!(code, 0, "{out}");
}

fn reports_drift(dir: &Path, args: &[&str]) -> String {
    let (out, _) = aplan(dir, args);
    out
}

/// `check` is the hermetic gate, and this rule reads a file rather than git, so
/// it belongs to that half of the ladder.
#[test]
fn check_reports_a_stale_contract_and_stops_once_it_is_regenerated() {
    let dir = stale_repo();
    let before = reports_drift(dir.path(), &["check"]);
    assert!(before.contains("APLAN041"), "{before}");
    assert!(
        before.contains("aplan export-agents-md --write"),
        "the remedy is the command: {before}"
    );

    regenerate(dir.path());
    let after = reports_drift(dir.path(), &["check"]);
    assert!(
        !after.contains("APLAN041"),
        "the fix the finding names did not clear it:\n{after}"
    );
}

/// `verify` is a superset of `check` (`A1-058`), and an adopter wiring one
/// command into CI reaches for this one, so silence here would leave the rule
/// unreachable for exactly the repository that automated its gate.
#[test]
fn verify_reports_it_too() {
    let dir = stale_repo();
    // `verify` opens the repository, so the fixture needs one.
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.invalid"]);
    git(&["config", "user.name", "t"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "fixture"]);

    let out = reports_drift(dir.path(), &["verify"]);
    assert!(out.contains("APLAN041"), "{out}");
}

/// The block is what an *agent* reads, and this is the agent's `check`: a rule
/// that reached only the CLI would miss the reader it was written for.
#[test]
fn the_agent_surface_reports_it_as_well() {
    let dir = stale_repo();
    let (out, _) = aplan(dir.path(), &["check", "--format", "agent"]);
    assert!(out.contains("APLAN041"), "{out}");
    // The agent format carries the instruction as a field, which is what an
    // agent acts on: a message it has to parse English out of is not one.
    assert!(
        out.contains("export-agents-md --write"),
        "no instruction to act on: {out}"
    );
}

/// A repository that never generated a block hears nothing, through the real
/// command rather than through the rule's own unit test: the CLI decides which
/// file to look at, and pointing it at the wrong one would nag every adopter.
#[test]
fn a_repository_with_no_contract_file_is_silent() {
    let dir = stale_repo();
    std::fs::remove_file(dir.path().join("AGENTS.md")).expect("rm");
    let out = reports_drift(dir.path(), &["check"]);
    assert!(!out.contains("APLAN041"), "{out}");
    assert!(out.contains("ok: 1 tasks"), "{out}");
}

/// The three ways this rule was reporting a contract that was not stale, found
/// by its own audit. Each is a *false* finding, which is worse than a missing
/// one: it names a remedy that would make the file wrong.
mod not_actually_stale {
    use super::*;

    /// The block is rendered from the policy, so it means nothing when the
    /// policy is not the tree's.
    ///
    /// `--no-config` reads none at all and renders from this tool's defaults,
    /// which differ from any real repository's. Measured on `aplan` itself
    /// before the fix: a clean `AGENTS.md` reported as stale.
    #[test]
    fn a_run_that_reads_no_config_has_no_policy_to_judge_the_block_by() {
        let dir = stale_repo();
        // Stale under the tree's own policy, so this is measuring the scoping
        // rather than a file that happened to match.
        assert!(reports_drift(dir.path(), &["check"]).contains("APLAN041"));

        let out = reports_drift(dir.path(), &["check", "--no-config"]);
        assert!(
            !out.contains("APLAN041"),
            "judged a contract against defaults the repository never adopted:\n{out}"
        );
    }

    /// `--config` naming a policy that is not the one the tree declares is the
    /// reviewing case the flag exists for: a trusted file, deliberately not
    /// theirs.
    ///
    /// The trusted policy is written **inside** the tree and pointed at its real
    /// plan, which is the arrangement the first version of this guard let
    /// through: it asked whether the named file was inside the repository, and a
    /// file can be inside a repository without being the policy that repository
    /// declares. Placing it outside would pass either version, and prove
    /// nothing (`A1-273`, audit).
    #[test]
    fn a_trusted_config_that_is_not_the_trees_own_does_not_judge_its_contract() {
        let dir = stale_repo();
        std::fs::create_dir_all(dir.path().join("review")).expect("subdir");
        let policy = dir.path().join("review/trusted.yml");
        std::fs::write(&policy, format!("root: {}/aplan\n", dir.path().display())).expect("policy");

        // The same plan, under the tree's own policy, does report -- so what is
        // measured below is the scoping and not a file that happened to match.
        assert!(reports_drift(dir.path(), &["check"]).contains("APLAN041"));

        let out = reports_drift(
            dir.path(),
            &["check", "--config", policy.to_str().expect("utf8")],
        );
        assert!(
            !out.contains("APLAN041"),
            "judged this tree's contract against a policy it never adopted:\n{out}"
        );
    }

    /// And the other use of the same flag -- being explicit in your own tree --
    /// keeps the rule, because losing a gate because a workflow spelled out its
    /// config file would be worse than either.
    #[test]
    fn a_config_named_inside_the_tree_is_still_the_trees_own() {
        let dir = stale_repo();
        let out = reports_drift(dir.path(), &["check", "--config", ".aplan.yml"]);
        assert!(out.contains("APLAN041"), "the gate went quiet:\n{out}");
    }

    /// A plan that did not load whole cannot say what its own contract should
    /// contain: the block names the id convention the plan teaches, so a plan
    /// read from a subset renders a subset's block.
    ///
    /// Measured before the fix on a repository whose task files were all
    /// unreadable: `failed: 0 error(s), 1 warning(s) across 0 tasks`, the single
    /// finding being a stale contract that was not stale -- and the remedy it
    /// named would have written a block claiming the plan teaches no ids.
    #[test]
    fn a_plan_that_did_not_load_is_not_a_stale_contract() {
        let dir = stale_repo();
        assert!(reports_drift(dir.path(), &["check"]).contains("APLAN041"));

        // A `title` whose value opens a nested mapping: the load error an
        // adopter actually makes.
        let task = dir.path().join("aplan/tasks/A-1.md");
        let text = std::fs::read_to_string(&task).expect("read");
        std::fs::write(
            &task,
            text.replace("title: A task in a", "title: A task: in a"),
        )
        .expect("write");

        let out = reports_drift(dir.path(), &["check"]);
        assert!(
            out.contains("APLAN001"),
            "the real problem is reported: {out}"
        );
        assert!(
            !out.contains("APLAN041"),
            "a plan nobody can read is not a stale contract:\n{out}"
        );
    }

    /// The generated block names the plan directory, and it used to name it
    /// relative to the *working directory*: run from a subdirectory, the block
    /// said planning lives under `../aplan/`.
    ///
    /// That is a defect in `export-agents-md` rather than in the rule -- a
    /// committed artefact whose whole point is reading the same on the next
    /// checkout -- and `APLAN041` is what surfaced it, by reporting a contract
    /// that was current as stale.
    #[test]
    fn the_block_names_the_plan_directory_the_same_way_from_anywhere() {
        let dir = stale_repo();
        std::fs::create_dir_all(dir.path().join("src")).expect("subdir");

        let (from_root, _) = aplan(dir.path(), &["export-agents-md"]);
        let (from_below, _) = aplan(
            &dir.path().join("src"),
            &["export-agents-md", "--root", "../aplan"],
        );
        assert!(
            from_root.contains("`aplan/`"),
            "the repository-relative path: {from_root}"
        );
        assert_eq!(
            from_root, from_below,
            "the block a committed file carries depends on the directory it was generated from"
        );
    }
}

/// Proves `agent_contract.provenance_does_not_block_policy` (`A2-122`,
/// ADR-0072): build-only provenance skew (including two same-version `-dev`
/// builds and a first-time contributor on an unpinned `PATH`) does NOT fire
/// `APLAN041` or block `check`, `verify`, `export-agents-md --check`, or MCP
/// `aplan_check`, even under `deny_warnings: true` without `advisory`, while
/// keeping the non-blocking provenance signal visible with both identities and
/// the `.aplan.yml` `requires` pin route.
#[test]
fn provenance_only_drift_does_not_block_check_verify_or_mcp() {
    let dir = stale_repo();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\nevidence:\n  require_min_strength: [commit]\n",
    )
    .expect("config");
    regenerate(dir.path());

    // Simulate a second same-version `-dev` build writing the identical policy
    // block with a different surface digest.
    let (version, surface) = aplan::written_by();
    let mine = format!("<!-- written by aplan {version}, surface {surface} -->");
    let other_surface = if surface == "000000000000" {
        "111111111111"
    } else {
        "000000000000"
    };
    let theirs = format!("<!-- written by aplan {version}, surface {other_surface} -->");
    let agents_path = dir.path().join("AGENTS.md");
    let doctored = std::fs::read_to_string(&agents_path)
        .expect("read AGENTS.md")
        .replace(&mine, &theirs);
    std::fs::write(&agents_path, &doctored).expect("write doctored AGENTS.md");

    // `check` under `deny_warnings: true` passes with 0 warnings and emits the
    // non-blocking provenance note naming both builds and `.aplan.yml` `requires`.
    let (check_out, check_code) = aplan(dir.path(), &["check"]);
    assert_eq!(
        check_code, 0,
        "provenance-only drift blocked `check`:\n{check_out}"
    );
    assert!(!check_out.contains("APLAN041"), "{check_out}");
    assert!(check_out.contains(&surface), "{check_out}");
    assert!(check_out.contains(other_surface), "{check_out}");
    assert!(check_out.contains("requires"), "{check_out}");
    assert!(
        !check_out.contains("Run `aplan export-agents-md --write"),
        "must not tell a contributor on unpinned PATH to regenerate:\n{check_out}"
    );

    // `check --format json` carries zero warnings and the structured
    // `agent_contract_provenance` object.
    let json_raw = common::output(dir.path(), &["check", "--format", "json"]);
    assert!(json_raw.status.success(), "{json_raw:?}");
    let doc: serde_json::Value = serde_json::from_slice(&json_raw.stdout).expect("check json");
    assert_eq!(doc["warnings"], 0, "{doc}");
    assert_eq!(doc["errors"], 0, "{doc}");
    assert!(
        doc["agent_contract_provenance"]["written_by"]
            .as_str()
            .is_some_and(|w| w.contains(other_surface)),
        "{doc}"
    );
    assert!(
        doc["agent_contract_provenance"]["checked_by"]
            .as_str()
            .is_some_and(|c| c.contains(&surface)),
        "{doc}"
    );

    // `export-agents-md --check` passes too while surfacing the same note.
    let (export_out, export_code) =
        aplan(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert_eq!(export_code, 0, "{export_out}");
    assert!(export_out.contains(other_surface), "{export_out}");
    assert!(export_out.contains("requires"), "{export_out}");

    // `verify` under `deny_warnings: true` passes too.
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.invalid"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "fixture"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }
    let (verify_out, verify_code) = aplan(dir.path(), &["verify"]);
    assert_eq!(
        verify_code, 0,
        "provenance-only drift blocked `verify`:\n{verify_out}"
    );
    assert!(!verify_out.contains("APLAN041"), "{verify_out}");
    assert!(verify_out.contains(other_surface), "{verify_out}");

    assert_machine_and_mcp_provenance_surfaces(dir.path(), other_surface, &surface);
}

fn assert_machine_and_mcp_provenance_surfaces(
    root: &std::path::Path,
    other_surface: &str,
    my_surface: &str,
) {
    // `check --format agent` and `verify --format json` both emit clean JSON
    // with 0 warnings and the structured `agent_contract_provenance` object.
    for args in [
        vec!["check", "--format", "agent"],
        vec!["verify", "--format", "json"],
    ] {
        let raw = common::output(root, &args);
        assert!(raw.status.success(), "`{args:?}` failed: {raw:?}");
        let doc: serde_json::Value = serde_json::from_slice(&raw.stdout)
            .unwrap_or_else(|e| panic!("`{args:?}` must emit pure JSON ({e})"));
        assert_eq!(doc["warnings"], 0, "{args:?}: {doc}");
        assert_eq!(doc["errors"], 0, "{args:?}: {doc}");
        assert!(
            doc["agent_contract_provenance"]["written_by"]
                .as_str()
                .is_some_and(|w| w.contains(other_surface)),
            "{args:?}: {doc}"
        );
        assert!(
            doc["agent_contract_provenance"]["checked_by"]
                .as_str()
                .is_some_and(|c| c.contains(my_surface)),
            "{args:?}: {doc}"
        );
    }

    // MCP `aplan_check` and `aplan_verify` both report `denied: false`,
    // `warnings: 0`, and carry `agent_contract_provenance`.
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let check_call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"aplan_check","arguments":{}}}"#;
    let verify_call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"aplan_verify","arguments":{}}}"#;
    let mut child = common::command(root, &["mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn mcp");
    {
        use std::io::Write as _;
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{init}").expect("init");
        writeln!(stdin, "{check_call}").expect("check");
        writeln!(stdin, "{verify_call}").expect("verify");
    }
    let mcp_out = child.wait_with_output().expect("mcp exits");
    let stdout = String::from_utf8_lossy(&mcp_out.stdout);
    for id in [2, 3] {
        let needle = format!(r#""id":{id}"#);
        let line = stdout
            .lines()
            .find(|line| line.contains(&needle))
            .unwrap_or_else(|| panic!("expected id:{id} response:\n{stdout}"));
        let rpc: serde_json::Value = serde_json::from_str(line).expect("jsonrpc response");
        assert_eq!(rpc["result"]["isError"], serde_json::Value::Bool(false));
        let text = rpc["result"]["content"][0]["text"]
            .as_str()
            .expect("mcp text content");
        let payload: serde_json::Value = serde_json::from_str(text).expect("json payload");
        assert_eq!(payload["denied"], serde_json::Value::Bool(false), "{text}");
        assert_eq!(payload["errors"], serde_json::json!(0), "{text}");
        assert_eq!(payload["warnings"], serde_json::json!(0), "{text}");
        assert!(!text.contains("APLAN041"), "{text}");
        assert!(
            payload["agent_contract_provenance"]["written_by"]
                .as_str()
                .is_some_and(|w| w.contains(other_surface)),
            "{text}"
        );
    }
}

/// `A2-122` / `A2-196`: a pre-ADR-0037 generated block carrying no provenance
/// comment passes `check` and `export-agents-md --check` with a non-blocking
/// note when repository policy matches, and still blocks with `APLAN041` when
/// policy drifts.
#[test]
fn pre_adr0037_block_without_provenance_passes_when_policy_matches_and_blocks_on_policy_drift() {
    let dir = stale_repo();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\nevidence:\n  require_min_strength: [commit]\n",
    )
    .expect("config");
    regenerate(dir.path());

    let agents_path = dir.path().join("AGENTS.md");
    let stripped: String = std::fs::read_to_string(&agents_path)
        .expect("read AGENTS.md")
        .lines()
        .filter(|line| {
            !line
                .trim_start()
                .starts_with(aplan_core::contract::WRITTEN_BY_PREFIX)
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&agents_path, stripped).expect("write pre-ADR-0037 AGENTS.md");

    let (check_out, check_code) = aplan(dir.path(), &["check"]);
    assert_eq!(
        check_code, 0,
        "absent provenance with matching policy must not block:\n{check_out}"
    );
    assert!(!check_out.contains("APLAN041"), "{check_out}");
    assert!(
        check_out.contains("carries no writer provenance comment"),
        "{check_out}"
    );

    let (export_out, export_code) =
        aplan(dir.path(), &["export-agents-md", "--check", "AGENTS.md"]);
    assert_eq!(export_code, 0, "{export_out}");
    assert!(
        export_out.contains("carries no writer provenance comment"),
        "{export_out}"
    );

    // Real policy drift on the same pre-ADR-0037 block still fires APLAN041.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\nevidence:\n  require_min_strength: [file]\n",
    )
    .expect("drifted config");
    let (drift_out, drift_code) = aplan(dir.path(), &["check"]);
    assert_eq!(
        drift_code, 1,
        "policy drift must fire APLAN041:\n{drift_out}"
    );
    assert!(drift_out.contains("APLAN041"), "{drift_out}");
}

/// Proves `agent_contract.provenance_does_not_block_policy` (`A2-122`,
/// ADR-0072): a real repository-policy change (including runner configuration)
/// still fires `APLAN041`, blocks under `deny_warnings: true`, and retains the
/// exact repair command.
#[test]
fn policy_only_drift_still_blocks_with_exact_repair() {
    let dir = stale_repo();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\nevidence:\n  require_min_strength: [test]\n",
    )
    .expect("config");
    regenerate(dir.path());

    // Add a configured test runner -- a real repository policy change.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\nevidence:\n  require_min_strength: [test]\n\
         test:\n  - name: unit\n    list: [echo, t]\n    run: [echo, '{test}']\n",
    )
    .expect("updated config");

    let (out, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 1, "runner config drift must still block:\n{out}");
    assert!(out.contains("APLAN041"), "{out}");
    assert!(
        out.contains("aplan export-agents-md --write AGENTS.md"),
        "{out}"
    );

    regenerate(dir.path());
    let (clean, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 0, "regenerating clears real policy drift:\n{clean}");
    assert!(!clean.contains("APLAN041"), "{clean}");
}

/// `A2-122`: duplicate or malformed provenance comments block `check` under
/// `deny_warnings: true` with `APLAN041` rather than being silently normalized.
#[test]
fn malformed_or_duplicate_provenance_blocks_check() {
    let dir = stale_repo();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndeny_warnings: true\nevidence:\n  require_min_strength: [commit]\n",
    )
    .expect("config");
    regenerate(dir.path());

    let (version, surface) = aplan::written_by();
    let mine = format!("<!-- written by aplan {version}, surface {surface} -->");
    let path = dir.path().join("AGENTS.md");
    let text = std::fs::read_to_string(&path).expect("read");
    std::fs::write(
        &path,
        text.replace(
            &mine,
            &format!("{mine}\n<!-- written by aplan 0.7.0, surface 000000000000 -->"),
        ),
    )
    .expect("write duplicate provenance");

    let (out, code) = aplan(dir.path(), &["check"]);
    assert_eq!(code, 1, "duplicate provenance must fire APLAN041:\n{out}");
    assert!(out.contains("APLAN041"), "{out}");
    assert!(out.contains("malformed provenance"), "{out}");
}
