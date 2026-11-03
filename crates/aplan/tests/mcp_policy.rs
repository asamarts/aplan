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
