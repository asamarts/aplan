//! The MCP selection policy is an executable census of both public surfaces
//! (`A1-195`, corrected by `A1-492`).
//!
//! Tool parity answers whether an exposed tool agrees with the command it
//! projects. This answers the question one level earlier: why that operation
//! has a tool at all, what the narrower projections leave behind, and whether
//! every command and every tool was classified deliberately. The command side
//! comes from clap and the tool side from the server registry, so neither live
//! inventory is copied into this test.

use std::collections::{BTreeMap, BTreeSet};

use aplan_mcp::tools::{
    CliOnlyReason, MCP_COMMAND_POLICY, McpCommandDisposition, McpCommandPolicy, TOOLS,
};

fn audit(commands: &[&str], tools: &[&str], policy: &[McpCommandPolicy]) -> Vec<String> {
    let commands: BTreeSet<&str> = commands.iter().copied().collect();
    let tools: BTreeSet<&str> = tools.iter().copied().collect();
    let mut findings = Vec::new();

    let mut command_rows: BTreeMap<&str, usize> = BTreeMap::new();
    let mut tool_rows: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for row in policy {
        *command_rows.entry(row.command).or_default() += 1;
        match row.disposition {
            McpCommandDisposition::Mirrored { tools } => {
                if tools.is_empty() {
                    findings.push(format!("mirrored command `{}` names no tool", row.command));
                }
            }
            McpCommandDisposition::Projected { tools, boundary } => {
                if tools.is_empty() {
                    findings.push(format!("projected command `{}` names no tool", row.command));
                }
                if boundary.trim().is_empty() {
                    findings.push(format!(
                        "projected command `{}` does not state its boundary",
                        row.command
                    ));
                }
            }
            McpCommandDisposition::Excluded { reason } => {
                if reason.explanation().trim().is_empty() {
                    findings.push(format!(
                        "excluded command `{}` has no explanation",
                        row.command
                    ));
                }
            }
        }
        for tool in row.disposition.tools() {
            tool_rows.entry(tool).or_default().push(row.command);
        }
    }

    for (command, count) in &command_rows {
        if *count != 1 {
            findings.push(format!("command `{command}` has {count} policy rows"));
        }
    }

    let classified_commands: BTreeSet<&str> = command_rows.keys().copied().collect();
    for command in commands.difference(&classified_commands) {
        findings.push(format!("command `{command}` is unclassified"));
    }
    for command in classified_commands.difference(&commands) {
        findings.push(format!("policy command `{command}` is stale"));
    }

    for (tool, commands) in &tool_rows {
        if commands.len() != 1 {
            findings.push(format!(
                "tool `{tool}` is classified by {} command rows: {}",
                commands.len(),
                commands.join(", ")
            ));
        }
    }
    let classified_tools: BTreeSet<&str> = tool_rows.keys().copied().collect();
    for tool in tools.difference(&classified_tools) {
        findings.push(format!("tool `{tool}` is unclassified"));
    }
    for tool in classified_tools.difference(&tools) {
        findings.push(format!("policy tool `{tool}` is stale"));
    }

    findings
}

#[test]
fn every_cli_command_has_exactly_one_mcp_disposition() {
    let cli = aplan::cli_command();
    let commands: Vec<&str> = cli.get_subcommands().map(clap::Command::get_name).collect();
    let tools: Vec<&str> = TOOLS.iter().map(|tool| tool.name).collect();
    let findings = audit(&commands, &tools, MCP_COMMAND_POLICY);
    assert!(
        findings.is_empty(),
        "the MCP selection policy drifted from its public surfaces:\n  {}",
        findings.join("\n  ")
    );
}

#[test]
fn selection_audit_rejects_every_drift_direction() {
    const BASE: &[McpCommandPolicy] = &[
        McpCommandPolicy {
            command: "one",
            disposition: McpCommandDisposition::Mirrored {
                tools: &["tool-one"],
            },
        },
        McpCommandPolicy {
            command: "two",
            disposition: McpCommandDisposition::Excluded {
                reason: CliOnlyReason::AlternateView,
            },
        },
    ];
    assert!(audit(&["one", "two"], &["tool-one"], BASE).is_empty());

    let cases: &[(&str, Vec<String>, &str)] = &[
        (
            "unclassified command",
            audit(&["one", "two", "three"], &["tool-one"], BASE),
            "command `three` is unclassified",
        ),
        (
            "stale command",
            audit(&["one"], &["tool-one"], BASE),
            "policy command `two` is stale",
        ),
        (
            "unclassified tool",
            audit(&["one", "two"], &["tool-one", "tool-two"], BASE),
            "tool `tool-two` is unclassified",
        ),
        (
            "stale tool",
            audit(&["one", "two"], &[], BASE),
            "policy tool `tool-one` is stale",
        ),
    ];
    for (case, findings, expected) in cases {
        assert!(
            findings.iter().any(|finding| finding == expected),
            "{case} was not detected: {findings:?}"
        );
    }

    let duplicate_command = [BASE[0], BASE[0], BASE[1]];
    assert!(
        audit(&["one", "two"], &["tool-one"], &duplicate_command)
            .iter()
            .any(|finding| finding == "command `one` has 2 policy rows")
    );

    let duplicate_tool = [
        BASE[0],
        McpCommandPolicy {
            command: "two",
            disposition: McpCommandDisposition::Mirrored {
                tools: &["tool-one"],
            },
        },
    ];
    assert!(
        audit(&["one", "two"], &["tool-one"], &duplicate_tool)
            .iter()
            .any(|finding| finding.contains("tool `tool-one` is classified by 2 command rows"))
    );

    let blank_boundary = [McpCommandPolicy {
        command: "one",
        disposition: McpCommandDisposition::Projected {
            tools: &["tool-one"],
            boundary: " \t",
        },
    }];
    assert!(
        audit(&["one"], &["tool-one"], &blank_boundary)
            .iter()
            .any(|finding| finding == "projected command `one` does not state its boundary")
    );
}

#[test]
fn every_mcp_resource_and_template_is_classified_confined_and_driven() {
    let cli = aplan::cli_command();
    let commands: BTreeSet<&str> = cli.get_subcommands().map(clap::Command::get_name).collect();
    let tool_names: BTreeSet<&str> = TOOLS.iter().map(|tool| tool.name).collect();

    let live_resources = aplan_mcp::resources::wire_resources();
    assert_eq!(
        live_resources.len(),
        aplan_mcp::resources::RESOURCES.len(),
        "wire resources drifted from RESOURCES registry"
    );
    let mut seen_names = BTreeSet::new();
    for (spec, wire) in aplan_mcp::resources::RESOURCES
        .iter()
        .zip(live_resources.iter())
    {
        assert!(
            seen_names.insert(spec.name),
            "duplicate resource name `{}`",
            spec.name
        );
        assert_eq!(wire.uri, spec.uri);
        assert_eq!(wire.name, spec.name);
        assert!(spec.uri.starts_with("aplan://"), "{}", spec.uri);
        assert_eq!(spec.mime_type, "application/json");
        assert!(
            !spec.rationale.trim().is_empty(),
            "resource `{}` missing rationale",
            spec.name
        );
        assert!(
            commands.contains(spec.origin_command),
            "resource `{}` names unknown CLI origin `{}`",
            spec.name,
            spec.origin_command
        );
        assert!(
            !tool_names.contains(spec.name),
            "resource `{}` collided with an action tool name",
            spec.name
        );
        // Drive the parser for each static resource URI.
        aplan_mcp::resources::parse_resource_uri(spec.uri)
            .unwrap_or_else(|e| panic!("static resource `{}` failed to parse: {e:?}", spec.uri));
    }

    let live_templates = aplan_mcp::resources::wire_resource_templates();
    assert_eq!(
        live_templates.len(),
        aplan_mcp::resources::RESOURCE_TEMPLATES.len(),
        "wire resource templates drifted from RESOURCE_TEMPLATES registry"
    );
    for (spec, wire) in aplan_mcp::resources::RESOURCE_TEMPLATES
        .iter()
        .zip(live_templates.iter())
    {
        assert!(
            seen_names.insert(spec.name),
            "duplicate template name `{}`",
            spec.name
        );
        assert_eq!(wire.uri_template, spec.uri_template);
        assert_eq!(wire.name, spec.name);
        assert!(
            spec.uri_template.starts_with("aplan://"),
            "{}",
            spec.uri_template
        );
        assert_eq!(spec.mime_type, "application/json");
        assert!(
            !spec.rationale.trim().is_empty(),
            "template `{}` missing rationale",
            spec.name
        );
        assert!(
            commands.contains(spec.origin_command),
            "template `{}` names unknown CLI origin `{}`",
            spec.name,
            spec.origin_command
        );
        // Drive a representative instantiated URI for every declared template.
        let sample = match spec.name {
            "task_detail" => "aplan://tasks/A-1",
            "ready_tasks_page" => "aplan://collections/ready?limit=5",
            "completed_tasks_page" => {
                "aplan://collections/completed?cut=v0.8&phase=2&owner=alice&tag=mcp&oldest=true&history=never&limit=5"
            }
            other => panic!("unhandled MCP resource template `{other}` in audit"),
        };
        aplan_mcp::resources::parse_resource_uri(sample)
            .unwrap_or_else(|e| panic!("template `{}` sample `{sample}` failed: {e:?}", spec.name));
    }

    // Confirm `completed` is served via bounded resource template rather than
    // growing the action tool surface with an `aplan_completed` rollup tool.
    assert!(
        !tool_names.contains("aplan_completed"),
        "completed history must stay a bounded resource, not an action tool"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn completed_resource_agrees_with_cli_and_application_on_every_row_and_truncation() {
    use std::io::Write as _;
    use std::process::Stdio;

    let dir = tempfile::Builder::new()
        .prefix("mcp-completed-parity-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    let tasks = root.join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("tasks dir");

    let run_git = |cwd: &std::path::Path, args: &[&str], date: Option<&str>| {
        let mut cmd = std::process::Command::new("git");
        cmd.args(args).current_dir(cwd);
        if let Some(ts) = date {
            cmd.env("GIT_AUTHOR_DATE", ts).env("GIT_COMMITTER_DATE", ts);
        }
        let status = cmd.status().expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    };

    run_git(root, &["init", "--quiet"], None);
    run_git(
        root,
        &["config", "user.email", "test@example.invalid"],
        None,
    );
    run_git(root, &["config", "user.name", "Test"], None);

    // Commit 1: legacy task starts as `todo`, plus a milestone cut `v1.0`.
    std::fs::write(
        tasks.join("M-v1.md"),
        "---\nid: M-v1\ntitle: Cut v1.0\nphase: 1\nstatus: in_progress\ntags:\n- milestone\n\
         blocked_by:\n- C-INF\n- C-REC-1\n- C-REC-2\n---\n\nbody\n",
    )
    .expect("milestone");
    std::fs::write(
        tasks.join("C-INF.md"),
        "---\nid: C-INF\ntitle: Legacy inferred task\nphase: 1\nstatus: todo\nowner: bob\n\
         effort_days: 1\ntags:\n- mcp\n---\n\nbody\n",
    )
    .expect("C-INF todo");
    run_git(root, &["add", "."], None);
    run_git(
        root,
        &["commit", "--quiet", "-m", "seed"],
        Some("2026-10-01T10:00:00Z"),
    );

    // Commit 2: legacy task transitions to `done` without `closed_at` (Git-inferred),
    // and two recorded tasks are closed with explicit `closed_at`.
    std::fs::write(
        tasks.join("C-INF.md"),
        "---\nid: C-INF\ntitle: Legacy inferred task\nphase: 1\nstatus: done\nowner: bob\n\
         effort_days: 1\ntags:\n- mcp\n---\n\nbody\n",
    )
    .expect("C-INF done");
    std::fs::write(
        tasks.join("C-REC-1.md"),
        "---\nid: C-REC-1\ntitle: Recorded newer\nphase: 1\nstatus: done\nowner: alice\n\
         effort_days: 1\ntags:\n- mcp\nclosed_at: \"2026-10-15T12:00:00Z\"\n\
         closed_at_source: local\n---\n\nbody\n",
    )
    .expect("C-REC-1");
    std::fs::write(
        tasks.join("C-REC-2.md"),
        "---\nid: C-REC-2\ntitle: Recorded older\nphase: 1\nstatus: done\nowner: alice\n\
         effort_days: 1\ntags:\n- mcp\nclosed_at: \"2026-10-05T08:30:00Z\"\n\
         closed_at_source: local\n---\n\nbody\n",
    )
    .expect("C-REC-2");
    run_git(root, &["add", "."], None);
    run_git(
        root,
        &["commit", "--quiet", "-m", "close tasks"],
        Some("2026-10-10T09:00:00Z"),
    );

    let read_mcp_resource = |uri: &str| -> serde_json::Value {
        let mut child = crate::common::command(root, &["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn aplan mcp");
        {
            let mut stdin = child.stdin.take().expect("stdin");
            for req in [
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "clientInfo": { "name": "audit", "version": "0" }
                    }
                }),
                serde_json::json!({
                    "jsonrpc": "2.0", "method": "notifications/initialized"
                }),
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 2, "method": "resources/read",
                    "params": { "uri": uri }
                }),
            ] {
                writeln!(stdin, "{req}").expect("write request");
            }
            stdin.flush().ok();
        }
        let out = child.wait_with_output().expect("mcp wait");
        assert!(
            out.status.success(),
            "mcp failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let reply = String::from_utf8(out.stdout)
            .expect("utf8")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("jsonrpc line"))
            .find(|v| v.get("id") == Some(&serde_json::json!(2)))
            .expect("reply id=2");
        let text = reply["result"]["contents"][0]["text"]
            .as_str()
            .expect("resource text");
        serde_json::from_str(text).expect("resource json")
    };

    // Compare Page 1 across CLI, Application, and MCP resource (truncated = true).
    let cli_p1 = crate::common::run(
        root,
        &[
            "completed",
            "--tag",
            "mcp",
            "--limit",
            "2",
            "--format",
            "json",
        ],
    );
    assert_eq!(cli_p1.code, 0, "{}{}", cli_p1.stdout, cli_p1.stderr);
    let cli_p1_json: serde_json::Value = serde_json::from_str(&cli_p1.stdout).expect("cli p1 json");

    let identified = aplan_core::load_plan_identified(&root.join("aplan"));
    let app_identity = aplan_app::plan_inputs_identity(&identified.inputs);
    let app_p1 = aplan_app::completed_tasks(
        &identified.plan,
        &app_identity,
        root,
        std::path::Path::new("aplan"),
        &aplan_app::CompletedRequest {
            selector: aplan_core::select::Selector {
                tags: vec!["mcp".to_string()],
                ..Default::default()
            },
            limit: Some(2),
            ..Default::default()
        },
    )
    .expect("app p1");
    let app_p1_json = serde_json::to_value(&app_p1).expect("app p1 json");

    let mcp_p1_res = read_mcp_resource("aplan://collections/completed?tag=mcp&limit=2");
    assert_eq!(mcp_p1_res["complete"], false);
    assert_eq!(mcp_p1_res["truncated_by_size_cap"], false);
    assert_eq!(mcp_p1_res["order"], "closed_at_desc");
    assert_eq!(
        mcp_p1_res["completed"], app_p1_json,
        "MCP resource and application response diverged on page 1"
    );
    assert_eq!(
        mcp_p1_res["completed"], cli_p1_json,
        "MCP resource and CLI response diverged on page 1"
    );

    // Compare Page 2 across CLI, Application, and MCP resource (truncated = false).
    let next_uri = mcp_p1_res["next_uri"].as_str().expect("next_uri");
    let cursor = cli_p1_json["next_cursor"].as_str().expect("next_cursor");

    let cli_p2 = crate::common::run(
        root,
        &[
            "completed",
            "--tag",
            "mcp",
            "--limit",
            "2",
            "--cursor",
            cursor,
            "--format",
            "json",
        ],
    );
    assert_eq!(cli_p2.code, 0, "{}{}", cli_p2.stdout, cli_p2.stderr);
    let cli_p2_json: serde_json::Value = serde_json::from_str(&cli_p2.stdout).expect("cli p2 json");

    let app_p2 = aplan_app::completed_tasks(
        &identified.plan,
        &app_identity,
        root,
        std::path::Path::new("aplan"),
        &aplan_app::CompletedRequest {
            selector: aplan_core::select::Selector {
                tags: vec!["mcp".to_string()],
                ..Default::default()
            },
            limit: Some(2),
            cursor: Some(cursor.to_string()),
            ..Default::default()
        },
    )
    .expect("app p2");
    let app_p2_json = serde_json::to_value(&app_p2).expect("app p2 json");

    let mcp_p2_res = read_mcp_resource(next_uri);
    assert_eq!(mcp_p2_res["complete"], true);
    assert!(mcp_p2_res.get("next_uri").is_none());
    assert_eq!(
        mcp_p2_res["completed"], app_p2_json,
        "MCP resource and application response diverged on page 2"
    );
    assert_eq!(
        mcp_p2_res["completed"], cli_p2_json,
        "MCP resource and CLI response diverged on page 2"
    );
}
