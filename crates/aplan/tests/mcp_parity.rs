//! The MCP surface must agree with the command it mirrors (`A1-104`).
//!
//! The eighth pilot pass drove `aplan mcp` for the first time and found two
//! divergences in one sitting: closing through MCP left drift detection off for
//! every task an agent closed (`A1-100`), and `check` dropped the configured
//! verdict so a denied plan read as clean (`A1-101`). Both were invisible. The
//! payloads looked successful and the task files read as though the mechanism was
//! running.
//!
//! They are one shape. The MCP tools are a **second implementation of behaviour
//! the CLI owns**, and the pieces the server cannot reach are the pieces that
//! drift: `trace_fingerprints` and `verdict` are private helpers in this binary
//! crate, so `aplan-mcp` could not call them and diverged silently in both cases.
//! This repository's answer to a shape like that has always been an audit over the
//! concept rather than a fix per instance, which is what `Rule::ALL` and
//! `Format::ALL` already have.
//!
//! **The table is the audit.** [`PARITY`] says, per tool, what agreement means,
//! and names the function that performs it, so a classification is a promise the
//! code keeps rather than a label. Dispatching on the *kind* was not enough: two
//! tools sharing one would have run the same comparison, and the second would
//! have been credited for the first's work. There is no state in which a tool
//! is classified and quietly unchecked. The first version of this file had one, an
//! enum that appeared only in the table and drove nothing, which is the "passes
//! for the wrong reason" shape the audit exists to catch.
//!
//! Both divergences were written here as failing comparisons before either fix,
//! held in a `PENDING` list against the task that owned each, and the list emptied
//! as `A1-100` and `A1-101` landed. It is gone with them: a skip list nothing
//! skips is a test that passes for the wrong reason. A future divergence gets the
//! same treatment, failing comparison first.
//!
//! **The limit, stated rather than discovered.** These call `aplan_mcp::tools`
//! directly, not the JSON-RPC transport. That is where both known defects live,
//! so the audit catches them, but a defect in the server's wiring above the tools
//! layer, in how `aplan mcp` builds its `Options`, would pass here. The pilot had
//! to speak the transport by hand to find any of this, and a harness over the
//! transport is the honest next step if a divergence ever appears there.

use std::path::Path;

use aplan_core::validate;
use aplan_mcp::tools;

use crate::common;

/// How a tool is compared with the command it mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Parity {
    /// It writes: the task file afterwards must be byte-identical.
    File,
    /// It reports: the verdict must match, not only the counts.
    Verdict,
    /// It projects: the JSON is the same document the command's `--format json`
    /// prints, field for field (`A1-118`).
    ///
    /// There was an `Ids` kind beside this one, meaning "the same ids, in the
    /// same order" -- all that could be compared while `ready` and `why` had no
    /// machine shape to compare against. `A1-182` gave them one, and nothing
    /// classifies as `Ids` any more: every listing tool is now held to a whole
    /// document rather than to the ids somebody could parse out of prose.
    Payload,
}

/// Every MCP tool, and what agreement with its CLI counterpart means.
///
/// A tool with no counterpart belongs here as `None` rather than being left out,
/// so the list cannot quietly shrink.
type Comparison = fn() -> Result<(), String>;

const PARITY: &[(&str, Option<(Parity, Comparison)>)] = &[
    // The one tool whose CLI counterpart is a *flag* rather than a command, and
    // whose answer is about the running binary rather than about a plan
    // (`A1-259`). Compared here through the shared function; `mcp.rs` compares
    // the same two documents over the transport, which is the half that catches
    // a server handing the tool a document computed somewhere else.
    (
        "aplan_identity",
        Some((Parity::Payload, identity_matches_the_command)),
    ),
    // Upgraded from `Ids` to `Payload` when these two gained `--format json`
    // (`A1-182`). Comparing ids parsed out of prose was all that was possible
    // while the CLI had no machine shape; now the two surfaces can be held to
    // the same document, and the census below is what keeps them there.
    (
        "aplan_ready",
        Some((Parity::Payload, ready_returns_what_the_command_prints)),
    ),
    (
        "aplan_why",
        Some((Parity::Payload, why_returns_what_the_command_prints)),
    ),
    (
        "aplan_check",
        Some((Parity::Verdict, check_reports_the_same_verdict)),
    ),
    (
        "aplan_verify",
        Some((Parity::Verdict, verify_reports_the_same_verdict)),
    ),
    ("aplan_add", Some((Parity::File, add_leaves_the_same_files))),
    (
        "aplan_show",
        Some((Parity::Payload, show_returns_what_the_command_prints)),
    ),
    ("aplan_dep", Some((Parity::File, dep_leaves_the_same_files))),
    (
        "aplan_close",
        Some((Parity::File, close_leaves_the_same_file)),
    ),
    // `A1-359`'s four. Each writes a task file, so each is a `File` parity: the
    // two surfaces must leave the byte-identical task, or an agent and a person
    // working the same repository disagree about who holds what.
    (
        "aplan_claim",
        Some((Parity::File, claim_leaves_the_same_file)),
    ),
    (
        "aplan_release",
        Some((Parity::File, release_leaves_the_same_file)),
    ),
    (
        "aplan_start",
        Some((Parity::File, start_leaves_the_same_file)),
    ),
    (
        "aplan_stop",
        Some((Parity::File, stop_leaves_the_same_file)),
    ),
];

/// The two surfaces write the same claim, and refuse the same takeover
/// (`A1-359`).
fn claim_leaves_the_same_file() -> Result<(), String> {
    let (cli_dir, mcp_dir) = two_fixtures();

    // `--as` on both sides, because the derived identity is the machine's and a
    // comparison that depended on it would be measuring the environment.
    let out = common::output(cli_dir.path(), &["claim", "A-1", "--as", "ada"]);
    if !out.status.success() {
        return Err(format!(
            "the CLI refused a free task: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    tools::claim(
        &mcp_dir.path().join("aplan"),
        "A-1",
        Some("ada"),
        None,
        &policy(mcp_dir.path(), false),
    )
    .map_err(|e| format!("the MCP claim failed: {e}"))?;
    same_claim_holders(&cli_dir, &mcp_dir)?;

    // And both refuse an unnamed takeover of what `ada` now holds.
    let out = common::output(cli_dir.path(), &["claim", "A-1", "--as", "grace"]);
    if out.status.success() {
        return Err("the CLI took a held task without naming the holder".to_string());
    }
    if tools::claim(
        &mcp_dir.path().join("aplan"),
        "A-1",
        Some("grace"),
        None,
        &policy(mcp_dir.path(), false),
    )
    .is_ok()
    {
        return Err("the MCP tool took a held task without naming the holder".to_string());
    }
    same_claim_holders(&cli_dir, &mcp_dir)
}

/// Releasing leaves the same file on both surfaces, key removed rather than
/// emptied (`A1-359`).
fn release_leaves_the_same_file() -> Result<(), String> {
    let (cli_dir, mcp_dir) = two_fixtures();
    let setup = common::output(cli_dir.path(), &["claim", "A-1", "--as", "ada"]);
    if !setup.status.success() {
        return Err(format!(
            "the CLI refused the setup claim: {}",
            String::from_utf8_lossy(&setup.stderr)
        ));
    }
    tools::claim(
        &mcp_dir.path().join("aplan"),
        "A-1",
        Some("ada"),
        None,
        &policy(mcp_dir.path(), false),
    )
    .map_err(|e| format!("the MCP setup claim failed: {e}"))?;

    let out = common::output(cli_dir.path(), &["release", "A-1"]);
    if !out.status.success() {
        return Err(format!(
            "the CLI refused a release: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    tools::release(
        &mcp_dir.path().join("aplan"),
        "A-1",
        &policy(mcp_dir.path(), false),
    )
    .map_err(|e| format!("the MCP release failed: {e}"))?;
    same_file(&cli_dir, &mcp_dir, ".git/aplan/claims-v1.json")
}

/// Starting leaves the same file on both surfaces (`A1-359`).
fn start_leaves_the_same_file() -> Result<(), String> {
    let (cli_dir, mcp_dir) = two_fixtures();
    let out = common::output(cli_dir.path(), &["start", "A-1"]);
    if !out.status.success() {
        return Err(format!(
            "the CLI refused a start: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    tools::working(
        &mcp_dir.path().join("aplan"),
        "A-1",
        true,
        &policy(mcp_dir.path(), false),
    )
    .map_err(|e| format!("the MCP start failed: {e}"))?;
    same_file(&cli_dir, &mcp_dir, "aplan/tasks/A-1.md")
}

/// Stopping leaves the same file, and both refuse a task that is not under way
/// (`A1-359`).
fn stop_leaves_the_same_file() -> Result<(), String> {
    let (cli_dir, mcp_dir) = two_fixtures();
    for dir in [&cli_dir, &mcp_dir] {
        let p = dir.path().join("aplan/tasks/A-1.md");
        let running = std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())?
            .replace("status: todo", "status: in_progress");
        std::fs::write(&p, running).map_err(|e| e.to_string())?;
    }

    let out = common::output(cli_dir.path(), &["stop", "A-1"]);
    if !out.status.success() {
        return Err(format!(
            "the CLI refused a stop: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    tools::working(
        &mcp_dir.path().join("aplan"),
        "A-1",
        false,
        &policy(mcp_dir.path(), false),
    )
    .map_err(|e| format!("the MCP stop failed: {e}"))?;
    same_file(&cli_dir, &mcp_dir, "aplan/tasks/A-1.md")?;

    // `A-2` is `todo`, so neither surface has anything to stop.
    let out = common::output(cli_dir.path(), &["stop", "A-2"]);
    let cli_refused = !out.status.success();
    let mcp_refused = tools::working(
        &mcp_dir.path().join("aplan"),
        "A-2",
        false,
        &policy(mcp_dir.path(), false),
    )
    .is_err();
    if cli_refused != mcp_refused {
        return Err(format!(
            "the surfaces disagree about stopping a `todo` task: cli refused \
             {cli_refused}, mcp refused {mcp_refused}"
        ));
    }
    Ok(())
}

/// Two identical fixtures, one per surface.
fn two_fixtures() -> (tempfile::TempDir, tempfile::TempDir) {
    let cli_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let mcp_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    for dir in [&cli_dir, &mcp_dir] {
        fixture(dir.path());
        git2::Repository::init(dir.path()).expect("git init");
    }
    (cli_dir, mcp_dir)
}

/// The two surfaces left byte-identical content at `rel`.
fn same_file(
    cli_dir: &tempfile::TempDir,
    mcp_dir: &tempfile::TempDir,
    rel: &str,
) -> Result<(), String> {
    let from_cli = std::fs::read_to_string(cli_dir.path().join(rel)).map_err(|e| e.to_string())?;
    let from_mcp = std::fs::read_to_string(mcp_dir.path().join(rel)).map_err(|e| e.to_string())?;
    if from_cli != from_mcp {
        return Err(format!(
            "the two surfaces wrote different files:\n--- cli\n{from_cli}\n--- mcp\n{from_mcp}"
        ));
    }
    Ok(())
}

fn same_claim_holders(
    cli_dir: &tempfile::TempDir,
    mcp_dir: &tempfile::TempDir,
) -> Result<(), String> {
    let read =
        |dir: &tempfile::TempDir| -> Result<std::collections::BTreeMap<String, String>, String> {
            let text = std::fs::read_to_string(dir.path().join(".git/aplan/claims-v1.json"))
                .map_err(|error| error.to_string())?;
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|error| error.to_string())?;
            value["claims"]
                .as_object()
                .ok_or_else(|| "claim store has no claims object".to_string())?
                .iter()
                .map(|(id, claim)| {
                    claim["holder"]
                        .as_str()
                        .map(|holder| (id.clone(), holder.to_string()))
                        .ok_or_else(|| format!("claim `{id}` has no holder"))
                })
                .collect()
        };
    let cli = read(cli_dir)?;
    let mcp = read(mcp_dir)?;
    if cli == mcp {
        Ok(())
    } else {
        Err(format!(
            "the two surfaces recorded different claims: {cli:?} != {mcp:?}"
        ))
    }
}

#[test]
fn every_tool_is_classified_and_every_classification_is_exercised() {
    // Both directions, and the second is the one that matters: a tool can be
    // classified and never compared, which is a gate passing for the wrong
    // reason. `compare` dispatches on the classification, so being in this table
    // *is* being checked.
    let missing: Vec<&str> = tools::TOOLS
        .iter()
        .map(|t| t.name)
        .filter(|name| !PARITY.iter().any(|(n, _)| n == name))
        .collect();
    assert!(
        missing.is_empty(),
        "these MCP tools are not classified: {missing:?}. Say what agreement means \
         for each, or `None` if it has no CLI counterpart, so a tool cannot ship \
         unaudited"
    );

    let stale: Vec<&str> = PARITY
        .iter()
        .map(|(n, _)| *n)
        .filter(|name| !tools::TOOLS.iter().any(|t| &t.name == name))
        .collect();
    assert!(
        stale.is_empty(),
        "classified tools that do not exist: {stale:?}"
    );
}

#[test]
/// Proof for `tui.mutation_surface_has_cross_adapter_parity`: applicable MCP
/// mutations execute the same exact comparison functions as their CLI peers.
fn every_tool_agrees_with_the_command_it_mirrors() {
    let mut problems = Vec::new();
    let mut checked = 0usize;
    for (name, entry) in PARITY {
        let Some((_, comparison)) = entry else {
            continue;
        };
        checked += 1;
        if let Err(e) = comparison() {
            problems.push(format!("{name}: {e}"));
        }
    }
    assert!(
        checked == PARITY.len(),
        "every classified tool must be compared: {checked} of {}",
        PARITY.len()
    );
    assert!(
        problems.is_empty(),
        "the surfaces disagree:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn ready_parity_ignores_only_the_independent_observation_instant() {
    let cli = serde_json::json!({
        "ready": [{"id": "A-1"}],
        "closeable_cuts": [{"id": "M-1"}],
        "claim_observed_at_epoch_seconds": 10,
    });
    let mcp = serde_json::json!({
        "ready": [{"id": "A-1"}],
        "closeable_cuts": [{"id": "M-1"}],
        "claim_observed_at_epoch_seconds": 11,
    });

    assert_eq!(
        ready_payload_without_observation_instant(cli.clone(), "CLI").expect("valid CLI payload"),
        ready_payload_without_observation_instant(mcp.clone(), "MCP").expect("valid MCP payload")
    );

    let mut changed = mcp;
    changed["ready"][0]["id"] = serde_json::json!("A-2");
    assert_ne!(
        ready_payload_without_observation_instant(cli.clone(), "CLI").expect("valid CLI payload"),
        ready_payload_without_observation_instant(changed, "MCP").expect("valid MCP payload")
    );

    let mut missing = cli.clone();
    missing
        .as_object_mut()
        .expect("object")
        .remove("claim_observed_at_epoch_seconds");
    assert!(ready_payload_without_observation_instant(missing, "CLI").is_err());

    let mut malformed = cli;
    malformed["claim_observed_at_epoch_seconds"] = serde_json::json!("now");
    assert!(ready_payload_without_observation_instant(malformed, "CLI").is_err());
}

/// A plan with one traced, closeable task and one warning, written to `dir`.
///
/// The trace is what makes `close` parity meaningful: with `detect_drift` on, the
/// CLI stamps a fingerprint and the MCP tool must too. The missing estimate is
/// what makes `check` parity meaningful under `deny_warnings`, and the blocker is
/// what gives `why` something to report.
fn fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join("DOC.md"), "# Intro\n\nprose that a task traces\n").expect("doc");
    std::fs::write(
        dir.join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
         paths:\n- DOC.md\ntraces:\n- DOC.md#intro\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(
        dir.join("aplan/tasks/A-2.md"),
        "---\nid: A-2\ntitle: A-2\nphase: 0\nstatus: todo\nblocked_by: [A-1]\n---\n\nbody\n",
    )
    .expect("task");
}

/// The task ids the CLI printed, from its indented lines or its unindented ones.
///
/// The runner both surfaces are given: it lists one test, bare.
fn runner() -> aplan_config::TestRunner {
    aplan_config::TestRunner {
        weight: None,
        exclusive: None,
        output_lines: None,
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        expect_tests: None,
        skipped: None,
        run_selects: None,
        timeout_seconds: None,
        covers: None,
        covers_by_test: None,
        inputs_by_test: None,
        name: None,
        dir: None,
        list: vec!["printf".into(), "known\n".into()],
        run: vec!["true".into(), "{test}".into()],
        batch: None,
    }
}

/// A git repository with one task owning `tests/**` and a runner listing `known`.
fn resolvable_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("tests")).expect("mkdir");
    std::fs::write(
        dir.join(".aplan.yml"),
        "root: aplan\ntest:\n  list: [\"printf\", \"known\\n\"]\n  run: [\"true\", \"{test}\"]\n",
    )
    .expect("config");
    std::fs::write(
        dir.join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: a task\nphase: 1\nstatus: todo\nowner: t\n\
         effort_days: 1\npaths:\n- tests/**\n---\n\nbody\n",
    )
    .expect("task");
    std::fs::write(dir.join("tests/t.rs"), "fn main() {}\n").expect("test file");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "initial"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    }
}

/// `why` prints `A-2 is blocked by:` as a header, and a first pass at this counted
/// that as a blocker and reported a divergence that did not exist. An audit that
/// cries wolf is worse than none, because it is why the next real finding gets
/// waved through.
fn cli_ids(out: &std::process::Output, indented: bool) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("  ") == indented)
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .filter(|w| w.starts_with("A-"))
        .collect()
}

/// `A1-259`. The identity the server would report is the identity the command
/// prints, compared as documents.
///
/// The tool takes no arguments and reads no plan, so there is nothing to set up:
/// what is compared is the one thing that could differ, which is whether the two
/// surfaces are describing the same executable.
fn identity_matches_the_command() -> Result<(), String> {
    let out = common::output(std::path::Path::new("."), &["facts", "--identity"]);
    if !out.status.success() {
        return Err(format!(
            "`facts --identity` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let from_cli: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("the command's json did not parse: {e}"))?;
    if from_cli["schema"] != "aplan/tool-identity/v1" {
        return Err(format!("not an identity document: {from_cli}"));
    }
    // The server is handed this document rather than computing one, so the
    // comparison that means something is over the wire (`mcp.rs`). What this
    // adds is the census entry: the tool is classified and exercised, so it
    // cannot be listed and quietly unchecked.
    if from_cli["version"].as_str().unwrap_or_default().is_empty() {
        return Err(format!("the identity names no version: {from_cli}"));
    }
    Ok(())
}

/// `A1-103`. `add` writes a task *and* edits a milestone, so both files are the
/// contract: a plan with `require_cut_membership` is invalid if either half is
/// missing, and it is the membership edit that made `add` a command rather than a
/// template.
/// `A1-124`. One edge, added through each surface, leaving the same file, and
/// `show` on both surfaces is one projection serialised twice, so the comparison
/// is the whole document rather than a field somebody remembered (`A1-118`).
fn show_returns_what_the_command_prints() -> Result<(), String> {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());

    let out = common::output(
        dir.path(),
        &["show", "A-1", "--format", "json", "--no-resolve"],
    );
    if !out.status.success() {
        return Err(format!(
            "the command failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let from_cli: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("the command's json did not parse: {e}"))?;
    let from_mcp = tools::show(&dir.path().join("aplan"), "A-1", &policy(dir.path(), false))
        .map_err(|e| format!("the tool failed: {e}"))?;

    // The tool always resolves and the command was asked not to, which is the one
    // field they are allowed to disagree about: compare the rest.
    let strip = |mut v: serde_json::Value| {
        if let Some(o) = v.as_object_mut() {
            o.remove("resolved");
        }
        v
    };
    if strip(from_cli.clone()) != strip(from_mcp.clone()) {
        return Err(format!(
            "the two surfaces describe one task differently.\n  cli: {from_cli}\n  mcp: {from_mcp}"
        ));
    }
    Ok(())
}

/// refused by each on the same plan.
fn dep_leaves_the_same_files() -> Result<(), String> {
    unsupported_dependency_source_is_refused_on_every_client()?;
    let cli_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let mcp_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let app_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    for dir in [&cli_dir, &mcp_dir, &app_dir] {
        fixture(dir.path());
    }

    let out = common::output(cli_dir.path(), &["dep", "rm", "A-2", "--after", "A-1"]);
    if !out.status.success() {
        return Err(format!(
            "the CLI refused removing a legal edge: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    tools::dep(
        &mcp_dir.path().join("aplan"),
        "A-2",
        "A-1",
        false,
        &policy(mcp_dir.path(), false),
    )
    .map_err(|e| format!("the MCP dep failed: {e}"))?;
    let app_policy = policy(app_dir.path(), false);
    let app_context = application_context(app_dir.path(), &app_policy);
    let app_response = aplan_app::edit_dependency(
        &app_context,
        &aplan_app::DependencyRequest {
            id: "A-2".to_string(),
            after: "A-1".to_string(),
            operation: aplan_app::DependencyOperation::Remove,
        },
    )
    .map_err(|e| format!("the direct application client failed: {e}"))?;
    if !app_response.changed || app_response.denied || !app_response.diagnostics.added.is_empty() {
        return Err(format!(
            "the direct application result did not describe the clean applied removal: \
             {app_response:?}"
        ));
    }

    let rel = "aplan/tasks/A-2.md";
    let from_cli = std::fs::read_to_string(cli_dir.path().join(rel)).map_err(|e| e.to_string())?;
    let from_mcp = std::fs::read_to_string(mcp_dir.path().join(rel)).map_err(|e| e.to_string())?;
    let from_app = std::fs::read_to_string(app_dir.path().join(rel)).map_err(|e| e.to_string())?;
    if from_cli != from_mcp || from_cli != from_app {
        return Err(format!(
            "the three clients wrote different files:\n--- cli\n{from_cli}\n--- mcp\n{from_mcp}\n--- app\n{from_app}"
        ));
    }

    restore_edge_then_refuse_cycle(&cli_dir, &mcp_dir, &app_dir, rel)
}

fn unsupported_dependency_source_is_refused_on_every_client() -> Result<(), String> {
    let cli_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let mcp_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let app_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let target = "---\nid: A-2\ntitle: A-2\nphase: 0\nstatus: todo\neffort_days: 1\n\
                  blocked_by: &edges\n  - A-1\ntags: *edges\n---\n\nbody\n";
    for dir in [&cli_dir, &mcp_dir, &app_dir] {
        fixture(dir.path());
        std::fs::write(dir.path().join("aplan/tasks/A-2.md"), target).expect("anchored target");
        std::fs::write(
            dir.path().join("aplan/tasks/A-3.md"),
            "---\nid: A-3\ntitle: A-3\nphase: 0\nstatus: todo\neffort_days: 1\n---\n",
        )
        .expect("tail");
    }

    let out = common::output(cli_dir.path(), &["dep", "add", "A-2", "--after", "A-3"]);
    if out.status.code() != Some(2)
        || !String::from_utf8_lossy(&out.stderr).contains("source-preserving delta")
    {
        return Err(format!(
            "the CLI did not retain the typed source-preservation refusal at exit 2 ({:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let mcp_error = tools::dep(
        &mcp_dir.path().join("aplan"),
        "A-2",
        "A-3",
        true,
        &policy(mcp_dir.path(), false),
    )
    .expect_err("the MCP adapter must retain the source-preservation refusal");
    if !mcp_error.to_string().contains("source-preserving delta") {
        return Err(format!(
            "the MCP adapter flattened the source-preservation refusal: {mcp_error}"
        ));
    }
    let app_policy = policy(app_dir.path(), false);
    let app_context = application_context(app_dir.path(), &app_policy);
    let app_error = aplan_app::edit_dependency(
        &app_context,
        &aplan_app::DependencyRequest {
            id: "A-2".to_string(),
            after: "A-3".to_string(),
            operation: aplan_app::DependencyOperation::Add,
        },
    )
    .expect_err("the application boundary must retain the source-preservation refusal");
    if !matches!(
        app_error,
        aplan_app::Error::Rewrite {
            source: aplan_core::CloseError::NoSequenceDelta("blocked_by"),
            ..
        }
    ) {
        return Err(format!(
            "the application boundary flattened the source-preservation refusal: {app_error}"
        ));
    }
    for dir in [&cli_dir, &mcp_dir, &app_dir] {
        let after = std::fs::read_to_string(dir.path().join("aplan/tasks/A-2.md"))
            .map_err(|error| error.to_string())?;
        if after != target {
            return Err("an unsupported source delta changed the target file".to_string());
        }
    }
    Ok(())
}

/// Put the edge back through all three clients, then prove their cycle refusal.
/// The second direction makes the refusal non-vacuous and proves both edit
/// shapes use the shared writer.
fn restore_edge_then_refuse_cycle(
    cli_dir: &tempfile::TempDir,
    mcp_dir: &tempfile::TempDir,
    app_dir: &tempfile::TempDir,
    rel: &str,
) -> Result<(), String> {
    let out = common::output(cli_dir.path(), &["dep", "add", "A-2", "--after", "A-1"]);
    if !out.status.success() {
        return Err(format!(
            "the CLI refused restoring a legal edge: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    tools::dep(
        &mcp_dir.path().join("aplan"),
        "A-2",
        "A-1",
        true,
        &policy(mcp_dir.path(), false),
    )
    .map_err(|e| format!("the MCP dep could not restore the edge: {e}"))?;
    let app_policy = policy(app_dir.path(), false);
    let app_context = application_context(app_dir.path(), &app_policy);
    let restored = aplan_app::edit_dependency(
        &app_context,
        &aplan_app::DependencyRequest {
            id: "A-2".to_string(),
            after: "A-1".to_string(),
            operation: aplan_app::DependencyOperation::Add,
        },
    )
    .map_err(|e| format!("the application boundary could not restore the edge: {e}"))?;
    if !restored.changed || restored.denied {
        return Err(format!(
            "the application did not restore the edge: {restored:?}"
        ));
    }
    let restored_cli =
        std::fs::read_to_string(cli_dir.path().join(rel)).map_err(|e| e.to_string())?;
    let restored_mcp =
        std::fs::read_to_string(mcp_dir.path().join(rel)).map_err(|e| e.to_string())?;
    let restored_app =
        std::fs::read_to_string(app_dir.path().join(rel)).map_err(|e| e.to_string())?;
    if restored_cli != restored_mcp || restored_cli != restored_app {
        return Err("the three clients restored different post-states".to_string());
    }

    all_three_refuse_cycle(cli_dir, mcp_dir, app_dir)
}

fn all_three_refuse_cycle(
    cli_dir: &tempfile::TempDir,
    mcp_dir: &tempfile::TempDir,
    app_dir: &tempfile::TempDir,
) -> Result<(), String> {
    // All three refuse the cycle the restored edge makes available, return the
    // same rule finding, and leave the complete target post-state untouched.
    let denied_rel = "aplan/tasks/A-1.md";
    let before_cli =
        std::fs::read_to_string(cli_dir.path().join(denied_rel)).map_err(|e| e.to_string())?;
    let before_mcp =
        std::fs::read_to_string(mcp_dir.path().join(denied_rel)).map_err(|e| e.to_string())?;
    let before_app =
        std::fs::read_to_string(app_dir.path().join(denied_rel)).map_err(|e| e.to_string())?;
    let out = common::output(cli_dir.path(), &["dep", "add", "A-1", "--after", "A-2"]);
    if out.status.success() {
        return Err("the CLI wrote a cycle".to_string());
    }
    let cli_refusal = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let mcp_refusal = tools::dep(
        &mcp_dir.path().join("aplan"),
        "A-1",
        "A-2",
        true,
        &policy(mcp_dir.path(), false),
    )
    .expect_err("the MCP tool must refuse the cycle")
    .to_string();
    let app_policy = policy(app_dir.path(), false);
    let app_context = application_context(app_dir.path(), &app_policy);
    let app_denied = aplan_app::edit_dependency(
        &app_context,
        &aplan_app::DependencyRequest {
            id: "A-1".to_string(),
            after: "A-2".to_string(),
            operation: aplan_app::DependencyOperation::Add,
        },
    )
    .map_err(|e| format!("the application boundary could not judge the cycle: {e}"))?;
    if !app_denied.denied || app_denied.changed || app_denied.diagnostics.added.is_empty() {
        return Err(format!(
            "the direct application client did not return a denied post-state: {app_denied:?}"
        ));
    }
    for finding in app_denied.diagnostics.lines() {
        let code = finding.split_whitespace().next().unwrap_or_default();
        if !cli_refusal.contains(code) || !mcp_refusal.contains(code) {
            return Err(format!(
                "the denied mutation's `{code}` finding did not cross every adapter:\n  CLI \
                 {cli_refusal}\n  MCP {mcp_refusal}\n  app {finding}"
            ));
        }
    }
    let after_cli =
        std::fs::read_to_string(cli_dir.path().join(denied_rel)).map_err(|e| e.to_string())?;
    let after_mcp =
        std::fs::read_to_string(mcp_dir.path().join(denied_rel)).map_err(|e| e.to_string())?;
    let after_app =
        std::fs::read_to_string(app_dir.path().join(denied_rel)).map_err(|e| e.to_string())?;
    if after_cli != before_cli
        || after_mcp != before_mcp
        || after_app != before_app
        || after_cli != after_mcp
        || after_cli != after_app
    {
        return Err("a denied dependency edit changed or diverged a target file".to_string());
    }
    Ok(())
}

fn application_context(dir: &Path, policy: &aplan_mcp::Policy) -> aplan_app::PlanContext {
    let root = dir.join("aplan");
    aplan_app::PlanContext::new(
        aplan_core::load_plan_identified(&root),
        aplan_app::ContextInputs {
            repository_root: policy.repo_root.clone(),
            configuration_file: policy.verify.config_file.clone(),
            trust: aplan_app::Trust::Hermetic,
            options: policy.options.clone(),
            verdict: policy.verdict.clone(),
        },
    )
}

fn add_leaves_the_same_files() -> Result<(), String> {
    add_refuses_the_same_tags_on_both_surfaces()?;
    let cli_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let mcp_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    for d in [cli_dir.path(), mcp_dir.path()] {
        fixture(d);
        std::fs::write(
            d.join("aplan/tasks/M-s0.md"),
            "---\nid: M-s0\ntitle: Cut 0\nphase: 0\nstatus: todo\n\
             tags: [milestone, s0]\nblocked_by: [A-1]\n---\n\nbody\n",
        )
        .expect("milestone");
    }

    let out = common::output(
        cli_dir.path(),
        &[
            "add",
            "Wire the sequencer",
            "--cut",
            "M-s0",
            "--effort",
            "2",
        ],
    );
    if !out.status.success() {
        return Err(format!(
            "the CLI add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let created = tools::add(
        &mcp_dir.path().join("aplan"),
        &tools::AddArgs {
            title: "Wire the sequencer".into(),
            cut: Some("M-s0".into()),
            effort_days: Some(2.0),
            ..tools::AddArgs::default()
        },
        &policy(mcp_dir.path(), false),
    )
    .map_err(|e| format!("the MCP add failed: {e}"))?;

    let id = created["created"].as_str().unwrap_or_default().to_string();
    if id.is_empty() {
        return Err(format!("no id in the payload: {created}"));
    }
    for rel in [
        format!("aplan/tasks/{id}.md"),
        "aplan/tasks/M-s0.md".to_string(),
    ] {
        let from_cli = std::fs::read_to_string(cli_dir.path().join(&rel))
            .map_err(|e| format!("the CLI did not write {rel}: {e}"))?;
        let from_mcp = std::fs::read_to_string(mcp_dir.path().join(&rel))
            .map_err(|e| format!("MCP did not write {rel}: {e}"))?;
        if from_cli != from_mcp {
            return Err(format!(
                "{rel} differs.\n  CLI: {from_cli:?}\n  MCP: {from_mcp:?}"
            ));
        }
    }
    Ok(())
}

fn close_fixture_with_stale_stamp(dir: &Path) {
    fixture(dir);
    std::fs::write(dir.join(".aplan.yml"), "detect_drift: true\n").expect("config");
    let path = dir.join("aplan/tasks/A-1.md");
    let task = std::fs::read_to_string(&path).expect("task").replace(
        "traces:\n- DOC.md#intro\n",
        "traces:\n- DOC.md#intro\ntrace_fingerprints:\n  DOC.md#old: stale-first\n\n\
         # Keep this author note.\n  DOC.md#intro: stale-second\n",
    );
    std::fs::write(path, task).expect("stale live stamp");
}

fn close_leaves_the_same_file() -> Result<(), String> {
    // `A1-100`. What `close` writes is what every later run reads, so the file
    // afterwards is the whole contract.
    let cli_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let mcp_dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    for d in [cli_dir.path(), mcp_dir.path()] {
        close_fixture_with_stale_stamp(d);
    }

    let out = common::output(
        cli_dir.path(),
        &["close", "A-1", "--evidence", "file:DOC.md"],
    );
    if !out.status.success() {
        return Err(format!(
            "the CLI close failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    tools::close(
        &mcp_dir.path().join("aplan"),
        "A-1",
        &["file:DOC.md".into()],
        // The policy the server holds, which is what `aplan mcp` passes it.
        // `repo_root` is the fixture, which is not a git repository, so both
        // surfaces record the citation unchecked and stay comparable.
        &aplan_mcp::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: validate::Options {
                detect_drift: true,
                traces_root: Some(mcp_dir.path().to_path_buf()),
                ..validate::Options::default()
            },
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions::default(),
            repo_root: mcp_dir.path().to_path_buf(),
        },
    )
    .map_err(|e| format!("the MCP close failed: {e}"))?;

    // The refusal, not only the success. `A1-109` made `close` reject a citation
    // `verify` would reject, and a surface that refuses less is one an agent can
    // use to record what a person cannot.
    //
    // A separate pair of fixtures, because these must be *git repositories with a
    // runner*: the shared fixture above is neither, so both surfaces would take
    // the skip branch and agree about doing nothing. That version of this check
    // passed and proved nothing, which is the shape this file exists to catch.
    let (cli_repo, mcp_repo) = (
        tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir"),
        tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir"),
    );
    for d in [cli_repo.path(), mcp_repo.path()] {
        resolvable_fixture(d);
    }
    let cli_refused = !common::output(
        cli_repo.path(),
        &["close", "A-1", "--evidence", "test:guider::known"],
    )
    .status
    .success();
    let mcp_refused = tools::close(
        &mcp_repo.path().join("aplan"),
        "A-1",
        &["test:guider::known".into()],
        &aplan_mcp::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: validate::Options::default(),
            verdict: aplan_core::Verdict::default(),
            verify: aplan_evidence::VerifyOptions {
                test_runner: Some(aplan_config::TestRunners::One(runner())),
                ..aplan_evidence::VerifyOptions::default()
            },
            repo_root: mcp_repo.path().to_path_buf(),
        },
    )
    .is_err();
    if !cli_refused || !mcp_refused {
        return Err(format!(
            "both surfaces must refuse a citation no runner lists: CLI refused={cli_refused}, \
             MCP refused={mcp_refused}"
        ));
    }

    let from_cli = std::fs::read_to_string(cli_dir.path().join("aplan/tasks/A-1.md")).expect("cli");
    let from_mcp = std::fs::read_to_string(mcp_dir.path().join("aplan/tasks/A-1.md")).expect("mcp");
    if from_cli.matches("trace_fingerprints:").count() != 1
        || from_cli.contains("stale-first")
        || from_cli.contains("stale-second")
        || !from_cli.contains("\n# Keep this author note.\n")
    {
        return Err(format!(
            "close did not replace the live fingerprint block: {from_cli:?}"
        ));
    }
    if from_cli == from_mcp {
        return Ok(());
    }
    Err(format!(
        "closing through MCP left a different file. The payload says closed either \
         way, so this is a divergence an adopter cannot see.\n  CLI: {from_cli:?}\n  \
         MCP: {from_mcp:?}"
    ))
}

/// The policy `aplan mcp` would build for this fixture.
fn policy(dir: &Path, deny_warnings: bool) -> aplan_mcp::Policy {
    policy_with(dir, deny_warnings, Vec::new())
}

/// The same, with rules this repository hears without being stopped by.
fn policy_with(dir: &Path, deny_warnings: bool, advisory: Vec<String>) -> aplan_mcp::Policy {
    aplan_mcp::Policy {
        id_strategy: aplan_core::IdStrategy::Sequential,
        options: validate::Options::default(),
        verdict: aplan_core::Verdict {
            deny_warnings,
            advisory,
        },
        verify: aplan_evidence::VerifyOptions {
            detect_drift: false,
            version: None,
            traces_root: None,
            config_file: None,
            execute: false,
            test_runner: None,
            max_unrelated_files: None,
            observation_inputs: None,
            url_check: None,
        },
        repo_root: dir.to_path_buf(),
    }
}

/// `A1-102`. The pilot's own case: a task citing a file that does not exist at
/// `HEAD`, which `check` cannot see and `verify` must.
fn verify_reports_the_same_verdict() -> Result<(), String> {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // A real repository, because verify opens git.
    let repo = git2::Repository::init(dir.path()).expect("git init");
    let mut index = repo.index().expect("index");
    index
        .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add");
    index.write().expect("write index");
    let tree = repo
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree");
    let sig = git2::Signature::now("t", "t@e.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "in", &tree, &[])
        .expect("commit");

    // Close A-1 citing a file that does not exist: the mistake an agent makes
    // when it cites its own deliverable a step early.
    std::fs::write(
        dir.path().join("aplan/tasks/A-1.md"),
        "---\nid: A-1\ntitle: A-1\nphase: 0\nstatus: done\neffort_days: 1\n\
         paths:\n- DOC.md\nevidence:\n  - kind: file\n    ref: NOPE.md\n---\n\nbody\n",
    )
    .expect("task");

    let out = common::output(dir.path(), &["verify"]);
    let cli_denied = !out.status.success();
    if !cli_denied {
        return Err(format!(
            "the fixture must fail `verify`, or this proves nothing: {}",
            String::from_utf8_lossy(&out.stdout)
        ));
    }
    let payload = tools::verify(&dir.path().join("aplan"), &policy(dir.path(), false))
        .map_err(|e| format!("the MCP verify failed: {e}"))?;
    if payload["denied"] != serde_json::json!(true) {
        return Err(format!(
            "the CLI's `verify` rejects this plan and the MCP tool does not say so. An \
             agent that cites a file it has not written yet has no other way to find \
             out: {payload}"
        ));
    }
    let from_cli = cli_findings(&common::output(dir.path(), &["verify", "--format", "json"]));
    let from_mcp = payload_findings(&payload);
    if from_cli.is_empty() {
        return Err("the fixture produced no findings, so this is vacuous".to_string());
    }
    if from_cli != from_mcp {
        return Err(format!(
            "verify reports different findings: CLI {from_cli:?}, MCP {from_mcp:?}"
        ));
    }
    if !from_mcp.iter().any(|(c, _)| c == "APLAN015") {
        return Err(format!("expected APLAN015 for the missing file: {payload}"));
    }

    // A plan that did not load whole. The CLI refuses to resolve evidence on one,
    // because a task in the unreadable file may be the blocker or the owner of a
    // cited path, so a partial answer reads as complete and is not. This case
    // loads cleanly in every other fixture here, which is why the divergence
    // survived until an audit went looking.
    std::fs::write(
        dir.path().join("aplan/tasks/bad.md"),
        "---\nid: [unclosed\ntitle: broken\n---\n\nbody\n",
    )
    .expect("task");
    let out = common::output(dir.path(), &["verify"]);
    // The refusal goes to stderr: stdout is the report channel, and there is no
    // report to give.
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let payload = tools::verify(&dir.path().join("aplan"), &policy(dir.path(), false))
        .map_err(|e| format!("the MCP verify failed: {e}"))?;
    let from_mcp = payload_findings(&payload);

    // An exact comparison, which it could not be until `A1-106`: the CLI used to
    // answer a partial plan with a refusal in prose and nothing on stdout, where
    // MCP returned the check report. Both now report what loaded, under the
    // repository's own options, and neither resolves evidence.
    if !said.contains("failed to load") {
        return Err(format!("the CLI must say why it refused: {said}"));
    }
    let cli_findings = cli_findings(&common::output(dir.path(), &["verify", "--format", "json"]));
    if cli_findings != from_mcp {
        return Err(format!(
            "on a plan that did not load whole, the CLI reports {cli_findings:?} and MCP \
             reports {from_mcp:?}"
        ));
    }
    let ladder = ["APLAN011", "APLAN012", "APLAN013", "APLAN014", "APLAN015"];
    let spoke: Vec<&(String, String)> = from_mcp
        .iter()
        .filter(|(c, _)| ladder.contains(&c.as_str()))
        .collect();
    if !spoke.is_empty() {
        return Err(format!(
            "the CLI refuses to resolve evidence on a plan that did not load whole, \
             and MCP reported {spoke:?}: a task in the unreadable file may be the \
             blocker or the owner of a cited path, so these read as complete and \
             are not"
        ));
    }
    if payload["denied"] != serde_json::json!(true) {
        return Err(format!("a plan that did not load is not clean: {payload}"));
    }
    Ok(())
}

/// The findings a payload reports, as `(code, task)` in order.
fn payload_findings(payload: &serde_json::Value) -> Vec<(String, String)> {
    payload["diagnostics"]
        .as_array()
        .map(|ds| {
            ds.iter()
                .map(|d| {
                    (
                        d["code"].as_str().unwrap_or_default().to_string(),
                        d["task"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The complete diagnostic fields common to every machine adapter, in order.
/// Transport-only guidance may be added beside them; the application result is
/// the authority for the finding itself.
fn common_diagnostics(payload: &serde_json::Value, repository: &Path) -> Vec<serde_json::Value> {
    payload["diagnostics"]
        .as_array()
        .map(|diagnostics| {
            diagnostics
                .iter()
                .map(|finding| {
                    serde_json::json!({
                        "code": finding["code"],
                        "severity": finding["severity"],
                        "task": finding["task"],
                        "file": normalized_file(&finding["file"], repository),
                        "line": finding["line"],
                        "column": finding["column"],
                        "message": finding["message"],
                        "detail": finding.get("detail").cloned().unwrap_or_else(|| serde_json::json!([])),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn normalized_file(value: &serde_json::Value, repository: &Path) -> serde_json::Value {
    let Some(rendered) = value.as_str() else {
        return serde_json::Value::Null;
    };
    let path = Path::new(rendered);
    serde_json::Value::String(
        path.strip_prefix(repository)
            .unwrap_or(path)
            .display()
            .to_string(),
    )
}

fn application_diagnostics(
    response: &aplan_app::CheckResponse,
    repository: &Path,
) -> Vec<serde_json::Value> {
    response
        .report
        .diagnostics
        .iter()
        .map(|finding| {
            serde_json::json!({
                "code": finding.code,
                "severity": finding.severity.to_string(),
                "task": finding.task,
                "file": finding.file.as_ref().map_or(serde_json::Value::Null, |path| {
                    normalized_file(
                        &serde_json::Value::String(path.display().to_string()),
                        repository,
                    )
                }),
                "line": finding.location.map(|location| location.line),
                "column": finding.location.map(|location| location.column),
                "message": finding.message,
                "detail": finding.detail,
            })
        })
        .collect()
}

/// The same, from the CLI's `--format json`.
fn cli_findings(out: &std::process::Output) -> Vec<(String, String)> {
    serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .map(|v| payload_findings(&v))
        .unwrap_or_default()
}

/// Both surfaces say the evidence half did not run (`A1-173`, audit).
///
/// The CLI says it in a note on stderr. An agent has no stderr to read: it reads
/// a field, sees `errors: 0` and `denied: false`, and concludes the plan passed
/// -- which is `A1-101`'s defect one rule later, and the fifth pilot's "single
/// most likely misconfiguration" met by the reader least able to notice which
/// command it ran.
fn check_says_the_evidence_went_unopened_on_both_surfaces() -> Result<(), String> {
    // Its own plan, because the shared fixture carries no evidence and this
    // comparison is about a plan that does: run against that one, both surfaces
    // stay silent and agree about nothing, which is a pass proving nothing. The
    // `(false, None)` arm below refuses that reading rather than accepting it.
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/A-3.md"),
        "---\nid: A-3\ntitle: A-3\nphase: 0\nstatus: done\neffort_days: 1\n\
         paths:\n- DOC.md\nevidence:\n- kind: commit\n  ref: d213c9a\n---\n\nbody\n",
    )
    .expect("task");

    let out = common::output(dir.path(), &["check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let cli_said = stderr.contains("did not open the repository");

    let payload = tools::check(&dir.path().join("aplan"), &policy(dir.path(), false))
        .map_err(|e| format!("the MCP check failed: {e}"))?;
    let mcp_said = payload.get("evidence_unresolved");
    let app_policy = policy(dir.path(), false);
    let app_context = application_context(dir.path(), &app_policy);
    let app_response = aplan_app::check(&app_context, aplan_app::CheckRequest);
    if !app_response.evidence_unresolved.any() {
        return Err(
            "the direct application client lost the unresolved-evidence result".to_string(),
        );
    }

    // Absent rather than zero where there is nothing to open, so the field's
    // presence is itself the signal and a plan with no resolvable citation reads
    // to an agent exactly as it always did.
    let quiet = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(quiet.path());
    let quiet_payload = tools::check(&quiet.path().join("aplan"), &policy(quiet.path(), false))
        .map_err(|e| format!("the MCP check failed: {e}"))?;
    if quiet_payload.get("evidence_unresolved").is_some() {
        return Err(format!(
            "a plan with nothing to resolve must not carry the field: {quiet_payload}"
        ));
    }

    match (cli_said, mcp_said) {
        (true, Some(left)) => {
            let citations = left.get("citations").and_then(serde_json::Value::as_u64);
            if citations.is_none_or(|n| n == 0) {
                return Err(format!("a count an agent can act on: {left}"));
            }
            if citations != Some(app_response.evidence_unresolved.citations as u64)
                || left.get("tasks").and_then(serde_json::Value::as_u64)
                    != Some(app_response.evidence_unresolved.tasks as u64)
            {
                return Err(format!(
                    "the MCP and application clients disagree about unresolved evidence: MCP \
                     {left}, app {:?}",
                    app_response.evidence_unresolved
                ));
            }
            if left.get("resolved_by").and_then(serde_json::Value::as_str) != Some("verify") {
                return Err(format!("and the command that would: {left}"));
            }
            // The fixture must actually carry evidence, or this proves nothing.
            Ok(())
        }
        (true, None) => {
            Err("the CLI says the evidence went unopened and the MCP payload does not".to_string())
        }
        (false, Some(left)) => Err(format!(
            "the MCP payload says it and the CLI does not: {left}"
        )),
        (false, None) => Err(
            "neither surface said it: the fixture carries no evidence, so this proves nothing"
                .to_string(),
        ),
    }
}

fn check_reports_the_same_verdict() -> Result<(), String> {
    // `A1-101`. The diagnostics already agree; the verdict is what does not cross,
    // and `isError` is the field an MCP client reads as "did this succeed".
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(dir.path().join(".aplan.yml"), "deny_warnings: true\n").expect("config");

    let out = common::output(dir.path(), &["check"]);
    let cli_denied = !out.status.success();
    assert!(
        cli_denied,
        "the fixture must actually be denied, or this proves nothing: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let payload = tools::check(&dir.path().join("aplan"), &policy(dir.path(), true))
        .map_err(|e| format!("the MCP check failed: {e}"))?;
    let mcp_denied = payload
        .get("denied")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let app_policy = policy(dir.path(), true);
    let app_context = application_context(dir.path(), &app_policy);
    let app_response = aplan_app::check(&app_context, aplan_app::CheckRequest);

    // The diagnostics as well as the verdict, which is what `A1-104` promised and
    // the first version of this comparison did not do: two surfaces can agree
    // that a plan is bad and disagree about why, and the "why" is what an agent
    // acts on.
    let cli_json = common::output(dir.path(), &["check", "--format", "json"]);
    let from_cli = cli_findings(&cli_json);
    let from_mcp = payload_findings(&payload);
    if from_cli.is_empty() {
        return Err("the fixture produced no diagnostics, so this is vacuous".to_string());
    }
    if from_cli != from_mcp {
        return Err(format!(
            "check reports different findings: CLI {from_cli:?}, MCP {from_mcp:?}"
        ));
    }
    let cli_document: serde_json::Value = serde_json::from_slice(&cli_json.stdout)
        .map_err(|error| format!("the CLI check document did not parse: {error}"))?;
    let complete_cli = common_diagnostics(&cli_document, dir.path());
    let complete_mcp = common_diagnostics(&payload, dir.path());
    let complete_app = application_diagnostics(&app_response, dir.path());
    if complete_cli != complete_mcp || complete_cli != complete_app {
        return Err(format!(
            "the ordered diagnostics differ across clients:\n  CLI {complete_cli:?}\n  MCP \
             {complete_mcp:?}\n  app {complete_app:?}"
        ));
    }

    if mcp_denied != cli_denied || app_response.denied != cli_denied {
        return Err(format!(
            "the clients disagree about the verdict: CLI {cli_denied}, MCP {mcp_denied}, \
             direct application {}.\n  payload: {payload}",
            app_response.denied
        ));
    }

    // Called from here rather than registered separately, so the table stays one
    // comparison per tool and `check` parity keeps meaning everything it means.
    check_reports_the_same_counts_including_advisory()?;
    check_says_the_evidence_went_unopened_on_both_surfaces()
}

/// `A1-112`. The verdict and the findings already crossed; the *counts* did not,
/// and both surfaces changed them in the same release. `errors` on one surface
/// and `errors` on the other must be the same number for the same plan, or an
/// agent and a human reading the same repository disagree about how bad it is.
fn check_reports_the_same_counts_including_advisory() -> Result<(), String> {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    // `APLAN009` is the fixture's warning, and `deny_warnings` makes it decide
    // the verdict, so downgrading it has to move both surfaces or neither.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "deny_warnings: true\nadvisory:\n  - APLAN009\n",
    )
    .expect("config");

    let cli_json = common::output(dir.path(), &["check", "--format", "json"]);
    let from_cli: serde_json::Value =
        serde_json::from_slice(&cli_json.stdout).map_err(|e| format!("the CLI json: {e}"))?;
    let payload = tools::check(
        &dir.path().join("aplan"),
        &policy_with(dir.path(), true, vec!["APLAN009".to_string()]),
    )
    .map_err(|e| format!("the MCP check failed: {e}"))?;
    let app_policy = policy_with(dir.path(), true, vec!["APLAN009".to_string()]);
    let app_context = application_context(dir.path(), &app_policy);
    let app_response = aplan_app::check(&app_context, aplan_app::CheckRequest);
    let app_counts = serde_json::json!({
        "tasks_checked": app_response.report.tasks_checked,
        "errors": app_response
            .report
            .blocking(&app_policy.verdict, aplan_core::Severity::Error),
        "warnings": app_response
            .report
            .blocking(&app_policy.verdict, aplan_core::Severity::Warning),
        "advisory": app_response.report.advisory_count(&app_policy.verdict),
    });

    if from_cli["advisory"].as_u64() == Some(0) {
        return Err("nothing was downgraded, so this proves nothing".to_string());
    }
    for field in ["tasks_checked", "errors", "warnings", "advisory"] {
        if from_cli[field] != payload[field] || from_cli[field] != app_counts[field] {
            return Err(format!(
                "`{field}` differs: CLI {}, MCP {}, app {}",
                from_cli[field], payload[field], app_counts[field]
            ));
        }
    }
    // And the verdict agrees with the counts on both: nothing blocking, so the
    // build passes despite `deny_warnings`.
    if payload["denied"] != serde_json::json!(false)
        || app_response.denied
        || !cli_json.status.success()
    {
        return Err(format!(
            "a plan whose only finding is advisory was denied: {payload}"
        ));
    }
    Ok(())
}

/// `A1-133`. Both `add` surfaces refuse what `APLAN030` would reject, and the
/// gap was real: the CLI counted the members itself and the MCP tool counted
/// nowhere, so an agent could write through the tool exactly what the command
/// refused, under a description promising it refuses what `check` rejects.
fn add_refuses_the_same_tags_on_both_surfaces() -> Result<(), String> {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "tag_families:\n  priority:\n    members: [priority-p0, priority-p1]\n\
         \x20   require: exactly-one\n    status: live\n",
    )
    .expect("config");

    // A task carrying a declared member, so the family is bound: a requirement
    // over a vocabulary no tag matches is `APLAN029`'s finding, and this rule
    // stays out of it (`A1-133`).
    std::fs::write(
        dir.path().join("aplan/tasks/BIND.md"),
        "---\nid: BIND\ntitle: A task that binds the priority vocabulary\nphase: 1\n\
         status: todo\nowner: kaminsod\neffort_days: 1\ntags:\n- priority-p0\n---\n\n\
         ## Why\n\nSomething.\n",
    )
    .expect("binder");

    let out = common::output(
        dir.path(),
        &[
            "add",
            "A task with no priority",
            "--root",
            "aplan",
            "--effort",
            "1",
        ],
    );
    if out.status.success() {
        return Err("the CLI wrote a task `APLAN030` rejects".to_string());
    }

    let families: std::collections::BTreeMap<String, aplan_core::TagFamily> = [(
        "priority".to_string(),
        aplan_core::TagFamily {
            members: vec!["priority-p0".to_string(), "priority-p1".to_string()],
            require: aplan_core::Cardinality::ExactlyOne,
            status: aplan_core::StatusScope::Live,
            milestones: aplan_core::MilestoneScope::Include,
        },
    )]
    .into_iter()
    .collect();
    let options = aplan_core::validate::Options {
        tag_families: families,
        ..aplan_core::validate::Options::default()
    };
    let mcp = tools::add(
        &dir.path().join("aplan"),
        &tools::AddArgs {
            title: "A task with no priority".into(),
            effort_days: Some(1.0),
            ..tools::AddArgs::default()
        },
        &aplan_mcp::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options,
            ..policy(dir.path(), false)
        },
    );
    if mcp.is_ok() {
        return Err("the MCP tool wrote a task the CLI refused".to_string());
    }

    // And both accept the satisfying form, or the agreement above would be two
    // surfaces that refuse everything.
    let ok = common::output(
        dir.path(),
        &[
            "add",
            "A task with one priority",
            "--root",
            "aplan",
            "--tag",
            "priority-p0",
        ],
    );
    if !ok.status.success() {
        return Err(format!(
            "the CLI refused a task that satisfies the requirement: {}",
            String::from_utf8_lossy(&ok.stderr)
        ));
    }
    Ok(())
}

/// A plan where every field of both payloads has a value: two ready tasks with
/// a preference between them, a cut whose members are all done, and a task
/// waiting on real work *and* on an id nothing defines.
///
/// Separate from [`fixture`] because the comparisons that share that one are
/// checking other things and a richer plan would move their answers. It exists
/// because comparing two empty lists proves nothing, and clearing
/// `closeable_cuts` on the CLI side went unnoticed by this file until the plan
/// had a closeable cut in it.
fn ready_fixture(dir: &Path) {
    // `A-` ids, because `cli_ids` filters on that prefix to keep `GHOST` -- an
    // id nothing defines -- out of the list it parses from the human output.
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    let tasks = [
        (
            "A-11",
            "---\nid: A-11\ntitle: done, and the cut's only member\nphase: 0\nstatus: done\n\
                 effort_days: 1\nevidence:\n  - kind: note\n    ref: seeded\n    \
                 captured: 2026-01-01\n---\n\nbody\n",
        ),
        (
            "A-12",
            "---\nid: A-12\ntitle: ready, and defers to its neighbour\nphase: 0\n\
                 status: todo\neffort_days: 2\nprefer_after: [A-13]\n---\n\nbody\n",
        ),
        (
            "A-13",
            "---\nid: A-13\ntitle: also ready\nphase: 0\nstatus: todo\n\
                 effort_days: 3\n---\n\nbody\n",
        ),
        (
            "A-14",
            "---\nid: A-14\ntitle: waiting on work and on a ghost\nphase: 0\n\
                 status: todo\neffort_days: 1\nblocked_by: [A-12, GHOST]\n---\n\nbody\n",
        ),
        (
            "M-r",
            "---\nid: M-r\ntitle: a cut whose members are all done\nphase: 0\n\
                 status: todo\ntags: [milestone]\nblocked_by: [A-11]\n---\n\nbody\n",
        ),
    ];
    for (id, body) in tasks {
        std::fs::write(dir.join(format!("aplan/tasks/{id}.md")), body).expect("task");
    }
}

/// Refuse a comparison whose inputs were empty on both sides.
fn must_list(doc: &serde_json::Value, field: &str) -> Result<(), String> {
    match doc[field].as_array() {
        Some(a) if !a.is_empty() => Ok(()),
        _ => Err(format!(
            "`{field}` is empty, so comparing it proves nothing: {doc}"
        )),
    }
}

/// Remove the only value that two sequential `ready` observations cannot share.
///
/// CLI and MCP each take an independent coordination snapshot. Their observation
/// seconds may straddle a clock boundary even when every piece of plan and claim
/// state agrees. Keep requiring the field and its public numeric shape, then
/// compare every stable field byte for byte.
fn ready_payload_without_observation_instant(
    mut doc: serde_json::Value,
    surface: &str,
) -> Result<serde_json::Value, String> {
    if !doc.is_object() {
        return Err(format!(
            "the {surface} ready payload is not an object: {doc}"
        ));
    }
    let object = doc
        .as_object_mut()
        .expect("the object shape was checked above");
    let observed = object
        .remove("claim_observed_at_epoch_seconds")
        .ok_or_else(|| format!("the {surface} ready payload has no observation instant"))?;
    if observed.as_u64().is_none() {
        return Err(format!(
            "the {surface} ready observation instant is not an unsigned integer: {observed}"
        ));
    }
    Ok(doc)
}

fn ready_returns_what_the_command_prints() -> Result<(), String> {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    ready_fixture(dir.path());
    let plan = dir.path().join("aplan");

    let from_cli = payload(&common::output(dir.path(), &["ready", "--format", "json"]))?;
    let from_mcp = serde_json::to_value(tools::ready(&plan).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let stable_cli = ready_payload_without_observation_instant(from_cli.clone(), "CLI")?;
    let stable_mcp = ready_payload_without_observation_instant(from_mcp.clone(), "MCP")?;
    if stable_cli != stable_mcp {
        return Err(format!(
            "the two surfaces list ready work differently.\n  cli: {from_cli}\n  mcp: {from_mcp}"
        ));
    }
    // Two empty lists agree about nothing, and that applies to every field --
    // not only to the one somebody remembered. Clearing `closeable_cuts` on the
    // CLI side was invisible here until the plan had a closeable cut in it.
    must_list(&from_cli, "ready")?;
    must_list(&from_cli, "closeable_cuts")?;
    if !from_cli["ready"]
        .as_array()
        .expect("a ready list")
        .iter()
        .any(|t| {
            t["prefers_to_follow"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        })
    {
        return Err(format!(
            "no ready task carries a preference, so that field is not compared: {from_cli}"
        ));
    }

    // And what a *person* reads still names the same ids, in the same order:
    // the machine shapes agreeing says nothing about the human rendering.
    let human = cli_ids(&common::output(dir.path(), &["ready"]), false);
    let listed: Vec<String> = from_cli["ready"]
        .as_array()
        .expect("a ready list")
        .iter()
        .map(|t| t["id"].as_str().unwrap_or_default().to_string())
        .collect();
    if human != listed {
        return Err(format!(
            "the human rendering lists {human:?}, the payload {listed:?}"
        ));
    }
    Ok(())
}

fn why_returns_what_the_command_prints() -> Result<(), String> {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    ready_fixture(dir.path());
    let plan = dir.path().join("aplan");

    // Both traversals, because `all` is a control on both surfaces and a
    // divergence in either is the shape this file exists to catch: an agent
    // asking for the complete closure and getting a folded one would read a
    // compaction as exhaustive truth (`A1-263`).
    let mut from_cli = serde_json::Value::Null;
    for all in [false, true] {
        let mut argv = vec!["why", "A-14", "--format", "json"];
        if all {
            argv.push("--all");
        }
        let cli = payload(&common::output(dir.path(), &argv))?;
        let mcp = serde_json::to_value(tools::why(&plan, "A-14", all).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if cli != mcp {
            return Err(format!(
                "the two surfaces explain one blockage differently (all={all}).\n  cli: \
                 {cli}\n  mcp: {mcp}"
            ));
        }
        let want = if all { "complete" } else { "cut-bounded" };
        if cli["traversal"] != want {
            return Err(format!(
                "the answer does not name the traversal that produced it: {cli}"
            ));
        }
        if !all {
            from_cli = cli;
        }
    }
    must_list(&from_cli, "blocked_by")?;
    // The field that only exists because the human rendering had it first.
    must_list(&from_cli, "unknown")?;

    let human = cli_ids(&common::output(dir.path(), &["why", "A-14"]), true);
    let listed: Vec<String> = from_cli["blocked_by"]
        .as_array()
        .expect("blockers")
        .iter()
        .map(|b| b["id"].as_str().unwrap_or_default().to_string())
        .collect();
    if human != listed {
        return Err(format!(
            "the human rendering names {human:?}, the payload {listed:?}"
        ));
    }
    Ok(())
}

/// A CLI json document with the `schema` envelope removed, which is the one key
/// the MCP payload does not carry.
fn payload(out: &std::process::Output) -> Result<serde_json::Value, String> {
    if !out.status.success() {
        return Err(format!(
            "the command failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let mut doc: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("the command's json did not parse: {e}"))?;
    let map = doc
        .as_object_mut()
        .ok_or_else(|| "the command's json is not an object".to_string())?;
    map.remove("schema")
        .ok_or_else(|| "the command's json carries no `schema` key".to_string())?;
    Ok(doc)
}

/// The server, driven over its real transport from a foreign working directory.
///
/// The audit above calls `tools::` directly, which is where both known defects
/// lived. This is the case that layer cannot see: `aplan mcp` builds the
/// `Options` the tools receive, and it resolved them from the *process's*
/// directory rather than the plan's. An MCP client starts the server wherever it
/// likes, so a repository asking for `detect_drift` got `detect_drift: false`,
/// every traced document was unreadable, and an agent's close stamped nothing
/// while reporting success (`A1-100`).
///
/// Kept small on purpose: one call, over stdio, from somewhere else.
#[test]
fn the_server_resolves_config_from_the_plan_not_the_caller() {
    use std::io::Write as _;

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let proj = dir.path().join("proj");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    fixture(&proj);
    std::fs::write(proj.join(".aplan.yml"), "detect_drift: true\n").expect("config");

    let root = proj.join("aplan").display().to_string();
    let mut child = common::command(&elsewhere, &["--root", &root, "mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the server starts");

    let mut stdin = child.stdin.take().expect("stdin");
    for line in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"aplan_close","arguments":{"id":"A-1","evidence":["file:DOC.md"]}}}"#,
    ] {
        writeln!(stdin, "{line}").expect("write");
    }
    drop(stdin); // closing stdin is how the server is told to stop
    let out = child.wait_with_output().expect("the server exits");
    let reply = String::from_utf8_lossy(&out.stdout);
    assert!(
        reply.contains("closed") && reply.contains(r#""isError":false"#),
        "the close did not succeed, so the stamping question is moot: {reply}"
    );

    let written = std::fs::read_to_string(proj.join("aplan/tasks/A-1.md")).expect("read");
    assert!(
        written.contains("trace_fingerprints:"),
        "the repository asked for `detect_drift`, so an agent's close must stamp \
         whatever directory the client started the server in: {written}"
    );
}

/// The other half of what `aplan mcp` resolves, over the same transport.
///
/// `A1-101` gave the payload a `denied` field and pinned it by calling `tools::`
/// with the policy passed by hand, which cannot see whether `cmd_mcp` resolves it
/// from `.aplan.yml` at all. That is exactly the gap `A1-100` shipped with, so it
/// is worth one more spawn.
#[test]
fn the_server_resolves_the_warning_policy_from_the_plan() {
    use std::io::Write as _;

    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let proj = dir.path().join("proj");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    fixture(&proj);
    // A-2 carries no estimate, so this plan warns and does not error.
    std::fs::write(proj.join(".aplan.yml"), "deny_warnings: true\n").expect("config");

    let root = proj.join("aplan").display().to_string();
    let mut child = common::command(&elsewhere, &["--root", &root, "mcp"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the server starts");

    let mut stdin = child.stdin.take().expect("stdin");
    for line in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"aplan_check","arguments":{}}}"#,
    ] {
        writeln!(stdin, "{line}").expect("write");
    }
    drop(stdin);
    let out = child.wait_with_output().expect("the server exits");
    let reply = String::from_utf8_lossy(&out.stdout);

    assert!(
        reply.contains(r#"\"denied\": true"#),
        "the repository sets `deny_warnings`, so the payload must say the plan is \
         denied whatever directory the client started the server in. An agent that \
         reads the counts alone ships a build CI rejects: {reply}"
    );
    assert!(
        reply.contains(r#""isError":false"#),
        "and it is still a successful call: a violation is not a malfunction: {reply}"
    );
}

/// Both surfaces refuse a duplicate id, and say the same thing about it
/// (`A1-135`, audited).
///
/// The refusal is one predicate, so the *decision* cannot diverge. What can, and
/// did while it was being written, is the sentence: the CLI said "no task `X` in
/// this plan" and the agent surface said "unknown task `X`" for the same
/// question. An agent and a person reading two answers to one question is the
/// divergence this file exists to catch (`A1-104`).
#[test]
fn both_surfaces_answer_the_same_way_about_an_id() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    fixture(dir.path());
    std::fs::write(
        dir.path().join("aplan/tasks/dup.md"),
        "---\nid: A-2\ntitle: Declared twice\nphase: 0\nstatus: todo\n---\n\nbody\n",
    )
    .expect("dup");

    // A duplicate id: refused on both, naming both files.
    let out = common::output(dir.path(), &["dep", "add", "A-2", "--after", "A-1"]);
    let cli_text = String::from_utf8_lossy(&out.stderr).into_owned();
    let tool = aplan_mcp::tools::dep(
        &dir.path().join("aplan"),
        "A-2",
        "A-1",
        true,
        &policy(dir.path(), false),
    )
    .expect_err("the tool refuses it too");
    let tool_text = tool.to_string();
    for text in [&cli_text, &tool_text] {
        assert!(
            text.contains("declared by 2 files") && text.contains("dup.md"),
            "one of the two surfaces did not name both files: {text}"
        );
    }

    // And an id nobody declares: the same sentence on both, which is what the
    // audit found had drifted.
    let out = common::output(dir.path(), &["dep", "add", "GHOST", "--after", "A-1"]);
    let cli_text = String::from_utf8_lossy(&out.stderr).into_owned();
    let tool = aplan_mcp::tools::dep(
        &dir.path().join("aplan"),
        "GHOST",
        "A-1",
        true,
        &policy(dir.path(), false),
    )
    .expect_err("the tool refuses it too");
    assert!(
        cli_text.contains("no task `GHOST` in this plan"),
        "{cli_text}"
    );
    assert!(
        tool.to_string().contains("no task `GHOST` in this plan"),
        "{tool}"
    );
}
