//! The `aplan_close` description says what `aplan_close` does (`A1-131`).
//!
//! It said two contradictory things. "Refuses exactly what the CLI refuses" was
//! made true by `A1-109`, which taught close to resolve evidence before writing;
//! "Recorded here, not resolved: run `aplan_verify` to check it against the
//! repository" was the sentence from before that, left in place. An agent reading
//! it was told to distrust or duplicate a gate that had already run
//! (pre-public adoption evidence).
//!
//! `A1-119` did not catch it, because that task holds the *command help* to the
//! tool list. `A1-104` did not, because parity compares behaviour between the two
//! surfaces rather than each tool's description against its own behaviour. This
//! is that missing guard, and it fails in both directions: a description that
//! promises a check close does not run, and one that omits a check it does.

use std::io::Write as _;
use std::path::Path;
use std::process::Command;

mod common;

/// The kinds `close` resolves before writing, and what a bad citation of each
/// looks like. Each must be named by the description *and* refused in fact.
const RESOLVED: &[(&str, &str, &str)] = &[
    (
        "commit",
        "a cited commit must exist and touch the task's `paths`",
        "commit:0000000000000000000000000000000000000000",
    ),
    ("file", "a cited file must exist", "file:nowhere/at/all.rs"),
    (
        "test",
        "a cited test must be one a configured runner lists",
        "test:nothing_lists_this",
    ),
];

fn description() -> String {
    aplan_mcp::tools::TOOLS
        .iter()
        .find(|t| t.name == "aplan_close")
        .expect("aplan_close is a tool")
        .description
        .to_string()
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("plan dir");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(root.join("src/a.rs"), "x\n").expect("src file");
    // The same runner the MCP policy below carries, so a `test` citation is
    // *unresolvable* on both surfaces rather than unchecked on one of them: with
    // no runner there is nothing to resolve against, which is a different claim
    // and one the description makes separately.
    std::fs::write(
        root.join(".aplan.yml"),
        "test:\n  list: [printf, \"a_test_that_exists\\n\"]\n  run: ['true', '{test}']\n",
    )
    .expect("config");
    std::fs::write(
        root.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: A task to close in this fixture\nphase: 1\nstatus: todo\n\
         owner: kaminsod\neffort_days: 1\npaths:\n- src/**\n---\n\n## Why\n\nSomething.\n",
    )
    .expect("task");
    for args in [vec!["init", "-q"], vec!["add", "-A"]] {
        Command::new("git")
            .args(&args)
            .current_dir(root)
            .output()
            .expect("git");
    }
    Command::new("git")
        .args([
            "-c",
            "user.email=a@b.c",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ])
        .current_dir(root)
        .output()
        .expect("git commit");
    dir
}

fn runner() -> aplan_config::TestRunners {
    aplan_config::TestRunners::One(aplan_config::TestRunner {
        max_memory: None,
        max_processes: None,
        wrap: None,
        expect_tests: None,
        skipped: None,
        run_selects: None,
        timeout_seconds: None,
        name: None,
        dir: None,
        covers: None,
        covers_by_test: None,
        list: vec!["printf".into(), "a_test_that_exists\n".into()],
        run: vec!["true".into(), "{test}".into()],
    })
}

fn policy(root: &Path, url_check: Option<aplan_config::UrlCheck>) -> aplan_mcp::Policy {
    aplan_mcp::Policy {
        id_strategy: aplan_core::IdStrategy::Sequential,
        options: aplan_core::validate::Options::default(),
        // Warnings block, which is the only setting under which a `url` finding
        // could refuse a closure at all.
        verdict: aplan_core::Verdict {
            deny_warnings: true,
            advisory: Vec::new(),
        },
        verify: aplan_evidence::VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(runner()),
            max_unrelated_files: None,
            url_check,
        },
        repo_root: root.to_path_buf(),
    }
}

fn close(
    dir: &Path,
    citation: &str,
    url_check: Option<aplan_config::UrlCheck>,
) -> Result<(), String> {
    aplan_mcp::tools::close(
        &dir.join("aplan"),
        "T-1",
        &[citation.to_string()],
        &policy(dir, url_check),
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// The guard: every clause of the description, against what close does.
#[test]
fn the_close_description_matches_what_close_does() {
    let d = description();

    // Over-promising: it must not claim a check this surface cannot run.
    assert!(
        d.contains("never runs a test"),
        "close does not execute tests, and the description has to say so: {d}"
    );
    assert!(
        d.contains("never fetches a url"),
        "close does not reach the network, and the description has to say so: {d}"
    );

    // Under-promising: each kind it *does* resolve is named, and refused in fact.
    for (kind, clause, citation) in RESOLVED {
        // The clause, not the bare word: `d.contains("test")` was satisfied by
        // "never runs a test", so dropping the promise about resolving one would
        // have left this passing. A test that passes for the wrong reason is what
        // this whole file is about.
        assert!(
            d.contains(clause),
            "close resolves `{kind}` and the description does not promise it: {d}"
        );
        let dir = repo();
        let refused = close(dir.path(), citation, None);
        assert!(
            refused.is_err(),
            "the description says `{kind}` is resolved before writing, and `{citation}` was \
             accepted"
        );
        // And nothing was written, so the claim "before it is written" holds.
        let task = std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read");
        assert!(task.contains("status: todo"), "{task}");
    }

    // The `unchecked` clause, which is the other half of the criterion: with no
    // runner configured a `test` citation cannot be resolved, and the payload has
    // to name it rather than let silence read as a pass.
    assert!(d.contains("`unchecked`"), "{d}");
    let dir = repo();
    let payload = aplan_mcp::tools::close(
        &dir.path().join("aplan"),
        "T-1",
        &["test:nothing_lists_this".to_string()],
        &aplan_mcp::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            verify: aplan_evidence::VerifyOptions {
                config_file: None,
                test_runner: None,
                ..policy(dir.path(), None).verify
            },
            ..policy(dir.path(), None)
        },
    )
    .expect("recorded, since nothing could check it");
    assert_eq!(
        payload["unchecked"],
        serde_json::json!(["test"]),
        "{payload}"
    );

    // And the contradiction that produced this task cannot come back.
    assert!(
        !d.contains("Recorded here, not resolved"),
        "the description still tells an agent the citation was not resolved: {d}"
    );
}

/// The other half of "never fetches a url", and the divergence that was hiding
/// behind the wording: the CLI's close reaches no network to record a citation,
/// because a link down for an hour must not block a closure. This surface passed
/// the whole policy, whose `url_check` belongs to `aplan_verify`, so under
/// `deny_warnings` an agent was refused a closure the command records.
#[test]
fn a_url_citation_closes_here_exactly_as_it_does_on_the_command() {
    let failing = aplan_config::UrlCheck {
        max_memory: None,
        max_processes: None,
        wrap: None,
        timeout_seconds: None,
        check: vec!["false".into(), "{url}".into()],
    };

    // The MCP surface, with a checker configured that fails every url.
    let dir = repo();
    let mcp = close(
        dir.path(),
        "url:https://example.invalid/x",
        Some(failing.clone()),
    );
    assert!(mcp.is_ok(), "the agent surface refused: {mcp:?}");

    // The command, same repository, same configuration.
    let dir = repo();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "deny_warnings: true\nurl:\n  check: [false, \"{url}\"]\n",
    )
    .expect("config");
    let out = common::output(
        dir.path(),
        &[
            "close",
            "T-1",
            "--root",
            "aplan",
            "--evidence",
            "url:https://example.invalid/x",
        ],
    );
    assert!(
        out.status.success(),
        "the command refused: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `aplan_verify` still checks urls. Narrowing `close` must not narrow the tool
/// whose whole job is to resolve the plan against the world.
#[test]
fn verify_still_checks_urls_after_close_stopped() {
    let dir = repo();
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: A closed task citing a url\nphase: 1\nstatus: done\n\
         owner: kaminsod\neffort_days: 1\npaths:\n- src/**\n\
         evidence:\n  - kind: url\n    ref: https://example.invalid/x\n---\n\n\
         ## Why\n\nSomething.\n",
    )
    .expect("task");
    let payload = aplan_mcp::tools::verify(
        &dir.path().join("aplan"),
        &policy(
            dir.path(),
            Some(aplan_config::UrlCheck {
                max_memory: None,
                max_processes: None,
                wrap: None,
                timeout_seconds: None,
                check: vec!["false".into(), "{url}".into()],
            }),
        ),
    )
    .expect("verify runs");
    let text = payload.to_string();
    assert!(
        text.contains("APLAN016"),
        "verify stopped checking urls too:\n{text}"
    );
}

/// The same guard, one tool over, because the defect was there too.
///
/// `aplan_add` promised "Refuses before writing what `check` would reject after",
/// which is a total claim it does not keep: under `require_cut_membership` it
/// writes a task in no cut, which `check` rejects. The command says so to a human
/// and the payload said nothing to an agent, so the surface that exists for
/// agents was the one that did not mention the plan it had just broken.
#[test]
fn the_add_description_matches_what_add_does() {
    let d = aplan_mcp::tools::TOOLS
        .iter()
        .find(|t| t.name == "aplan_add")
        .expect("aplan_add is a tool")
        .description;

    // `A1-131` narrowed this claim because it was false: `add` wrote a task in no
    // cut under `require_cut_membership`, which `check` rejects. `A1-113` made it
    // true by running the rules on the plan the mutation would produce, so the
    // case that falsified it is a refusal now.
    assert!(
        d.contains("Refuses before writing what `check` would reject after"),
        "{d}"
    );
    assert!(d.contains("named in `notes`"), "{d}");

    let dir = repo();
    let refused = aplan_mcp::tools::add(
        &dir.path().join("aplan"),
        &aplan_mcp::tools::AddArgs {
            title: "A task nobody placed in a cut".into(),
            effort_days: Some(1.0),
            ..aplan_mcp::tools::AddArgs::default()
        },
        &aplan_mcp::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options {
                require_cut_membership: true,
                ..aplan_core::validate::Options::default()
            },
            ..policy(dir.path(), None)
        },
    )
    .expect_err("refused, since `check` would reject it");
    assert!(
        refused.to_string().contains("APLAN022"),
        "the refusal names the rule: {refused}"
    );
}

/// A `cross-cutting` task belongs to no cut deliberately, so the note must not
/// fire on it: a disclosure that cries wolf is one an agent learns to ignore.
#[test]
fn a_cross_cutting_task_gets_no_membership_note() {
    let dir = repo();
    let payload = aplan_mcp::tools::add(
        &dir.path().join("aplan"),
        &aplan_mcp::tools::AddArgs {
            title: "A task that belongs to no cut on purpose".into(),
            effort_days: Some(1.0),
            tags: vec!["cross-cutting".to_string()],
            ..aplan_mcp::tools::AddArgs::default()
        },
        &aplan_mcp::Policy {
            id_strategy: aplan_core::IdStrategy::Sequential,
            options: aplan_core::validate::Options {
                require_cut_membership: true,
                ..aplan_core::validate::Options::default()
            },
            ..policy(dir.path(), None)
        },
    )
    .expect("written");
    assert!(
        payload["notes"].as_array().expect("notes").is_empty(),
        "{payload}"
    );
}

// ---------------------------------------------------------------------------
// `close.evidence_resolves_before_write` (`A1-132`). The name is only worth
// requiring if it means something exact, so this is what it means: on both
// surfaces, an unresolvable citation is refused and the task file is untouched,
// including when it arrives beside citations that do resolve.
// ---------------------------------------------------------------------------

/// The behaviour is advertised, and it is the name an adopter requires.
#[test]
fn the_widening_is_addressable_by_name() {
    let named = aplan_core::behaviour::ALL
        .iter()
        .any(|b| b.name == "close.evidence_resolves_before_write");
    assert!(named, "the behaviour is not advertised");

    let out = common::output(
        std::path::Path::new("."),
        &["facts", "--require", "close.evidence_resolves_before_write"],
    );
    assert!(
        out.status.success(),
        "this build does not advertise it: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// All or nothing, on both surfaces: one unresolvable citation among several
/// that resolve refuses the whole closure and leaves the file untouched. A
/// partial write is the failure the name promises against.
/// Proves the named behaviour `close.evidence_resolves_before_write` (`A1-211`).
#[test]
fn one_unresolvable_citation_refuses_the_whole_closure_on_both_surfaces() {
    let good = |dir: &Path| {
        let sha = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .expect("rev-parse");
        format!("commit:{}", String::from_utf8_lossy(&sha.stdout).trim())
    };

    for bad in [
        "commit:0000000000000000000000000000000000000000",
        "file:nowhere/at/all.rs",
        "test:nothing_lists_this",
    ] {
        // The agent surface.
        let dir = repo();
        let before = std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read");
        let refused = aplan_mcp::tools::close(
            &dir.path().join("aplan"),
            "T-1",
            &[good(dir.path()), bad.to_string()],
            &policy(dir.path(), None),
        );
        assert!(
            refused.is_err(),
            "`{bad}` was accepted beside a good citation"
        );
        assert_eq!(
            before,
            std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read"),
            "`{bad}`: the file was written despite the refusal"
        );

        // The command, same citations, same repository.
        let dir = repo();
        let before = std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read");
        let out = common::output(
            dir.path(),
            &[
                "close",
                "T-1",
                "--root",
                "aplan",
                "--evidence",
                &good(dir.path()),
                "--evidence",
                bad,
            ],
        );
        assert!(
            !out.status.success(),
            "the command accepted `{bad}`: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(
            before,
            std::fs::read_to_string(dir.path().join("aplan/tasks/T-1.md")).expect("read"),
            "`{bad}`: the command wrote despite refusing"
        );
    }
}

/// The url kind is the exception the behaviour has to be honest about: neither
/// surface resolves one while closing, so an unreachable url is recorded rather
/// than refused, on both. A name that implied otherwise would be the defect it
/// exists to prevent.
#[test]
fn a_url_is_recorded_rather_than_resolved_on_both_surfaces() {
    let failing = aplan_config::UrlCheck {
        max_memory: None,
        max_processes: None,
        wrap: None,
        timeout_seconds: None,
        check: vec!["false".into(), "{url}".into()],
    };
    let dir = repo();
    assert!(
        close(dir.path(), "url:https://example.invalid/x", Some(failing)).is_ok(),
        "the agent surface resolved a url while closing"
    );

    let dir = repo();
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "deny_warnings: true\nurl:\n  check: [false, \"{url}\"]\n",
    )
    .expect("config");
    let out = common::output(
        dir.path(),
        &[
            "close",
            "T-1",
            "--root",
            "aplan",
            "--evidence",
            "url:https://example.invalid/x",
        ],
    );
    assert!(
        out.status.success(),
        "the command resolved a url while closing"
    );
}

/// The one qualification the behaviour's own sentence has to carry: the command
/// has an explicit opt-out, and the agent surface has none.
///
/// `facts` prints that sentence into a consumer's terminal, so an unqualified
/// "resolves before it writes" would be a claim with a documented exception,
/// which is the shape this repository keeps finding in other people's tools.
#[test]
fn the_only_way_past_the_gate_is_explicit_and_says_so() {
    let what = aplan_core::behaviour::ALL
        .iter()
        .find(|b| b.name == "close.evidence_resolves_before_write")
        .expect("advertised")
        .what;
    assert!(
        what.contains("--no-resolve"),
        "the exception is unstated: {what}"
    );

    // It is a real exception: the citation is recorded, and the skip is named.
    let dir = repo();
    let out = common::output(
        dir.path(),
        &[
            "close",
            "T-1",
            "--root",
            "aplan",
            "--no-resolve",
            "--evidence",
            "commit:0000000000000000000000000000000000000000",
        ],
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{text}");
    assert!(
        text.contains("recorded without being resolved"),
        "the skip has to be named: {text}"
    );

    // And an agent cannot take it. Asserted over the transport rather than
    // against an internal, because what matters is the schema an agent is
    // actually handed: it accepts an id, evidence and a root, and nothing that
    // turns the gate off.
    let props = close_tool_properties();
    let mut names: Vec<String> = props.keys().cloned().collect();
    names.sort();
    assert_eq!(
        names,
        vec!["evidence", "id", "root"],
        "the agent surface grew a way past the gate"
    );
}

/// The `aplan_close` input schema as an agent receives it, over stdio.
fn close_tool_properties() -> serde_json::Map<String, serde_json::Value> {
    let dir = repo();
    let requests = [
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "audit", "version": "0" }
            }
        }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    ];
    let mut input = String::new();
    for r in &requests {
        input.push_str(&serde_json::to_string(r).expect("request"));
        input.push('\n');
    }

    let mut child = common::command(dir.path(), &["mcp", "--root", "aplan"])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawning the server");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("writing requests");
    let out = child.wait_with_output().expect("the server exits");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    let listed = text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["id"] == 2)
        .expect("a tools/list reply");
    listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["name"] == "aplan_close")
        .expect("aplan_close is listed")["inputSchema"]["properties"]
        .as_object()
        .cloned()
        .expect("properties")
}
