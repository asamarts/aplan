//! Every command's help fits the width it was given (`A1-205`).
//!
//! The defect, reported from an eighty-column terminal: `aplan --help` printed
//! one unbroken line per entry -- the longest 430 characters -- and the
//! *terminal* folded it at the screen edge, resuming at column zero. The
//! continuation then sat under the command names, so the list stopped reading as
//! two columns. `clap` was declared without `wrap_help`, so it never wrapped at
//! all.
//!
//! **A sample would prove nothing.** Checking `--help` and one subcommand says
//! nothing about the thirty-first one added later. This walks the command tree
//! `clap` builds from the derive, which is the shape `A1-115` named: enumerate
//! the surface from the type system, and give every member something to drive.
//! A subcommand or an option added tomorrow is covered the day it is filed,
//! without anyone remembering this file exists.

use aplan::cli_command;
use aplan_output::MAX_WIDTH;

mod common;

/// Every command in the tree, by the path a reader would type.
fn every_command(cmd: &clap::Command, prefix: &str, out: &mut Vec<(String, clap::Command)>) {
    let here = if prefix.is_empty() {
        cmd.get_name().to_string()
    } else {
        format!("{prefix} {}", cmd.get_name())
    };
    for sub in cmd.get_subcommands() {
        every_command(sub, &here, out);
    }
    out.push((here, cmd.clone()));
}

/// The tree, rendered at `width`.
///
/// The width is set on each command rather than only on the root: clap carries
/// it down while rendering a tree, but a subcommand cloned *out* of one has to
/// be told again, and a harness that forgot would report the binary as broken
/// when it is not. Checked against the real binary by
/// [`piped_help_ignores_columns`], which spawns it rather than building a
/// `Command`.
fn helps(width: usize) -> Vec<(String, String)> {
    tree()
        .into_iter()
        .map(|(name, cmd)| {
            let text = cmd.term_width(width).render_long_help().to_string();
            (name, text)
        })
        .collect()
}

/// Every command in the tree, unrendered, for a check that needs the command
/// itself rather than its help text.
fn tree() -> Vec<(String, clap::Command)> {
    let mut found = Vec::new();
    every_command(&cli_command(), "", &mut found);
    found
}

/// The invariant the defect broke: nothing is wider than the width.
///
/// Before the fix the longest line was 430 characters against a terminal of 80,
/// which is what the terminal then folded to column zero.
#[test]
fn no_command_renders_a_line_wider_than_the_width() {
    let mut offenders = Vec::new();
    for (name, text) in helps(MAX_WIDTH) {
        for line in text.lines() {
            let len = line.chars().count();
            if len > MAX_WIDTH {
                offenders.push(format!("`{name} --help`: {len} columns: {line}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "help wider than {MAX_WIDTH} columns, so the terminal folds it to column zero \
         and the continuation lands under the command names ({} case(s)):\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// The gate is only worth its run while the tree it walks is the real one.
#[test]
fn the_walk_reaches_every_command_the_tool_has() {
    let names: Vec<String> = helps(MAX_WIDTH).into_iter().map(|(n, _)| n).collect();
    assert!(
        names.len() > 30,
        "the walk found {} commands, which is fewer than this tool has: it is not \
         walking the tree",
        names.len()
    );
    for expected in ["aplan check", "aplan verify", "aplan dep add", "aplan mcp"] {
        assert!(
            names.iter().any(|n| n == expected),
            "`{expected}` is missing from the walk: {names:?}"
        );
    }
}

/// A continuation must be indented past the column its item starts in, which is
/// the actual complaint -- a short line is not the same as a wrapped one.
///
/// The first version of this test could not fail: it asserted that the line
/// began with `-` *or* had a non-empty first word, and every line has a
/// non-empty first word. That is the defect `A1-209` is about, shipped inside
/// the task that argued for gates which drive something.
///
/// **Where the defect can actually occur**, which the rewrite had to learn:
/// long help gives each option its own line and puts the description on the
/// next, so there is no name column to fold into. The two-column layouts are the
/// root's `Commands:` list and *short* help's `Options:`, and those are what this
/// walks. The item indent is read from the first item in each section rather than
/// assumed, because the two layouts use different ones.
#[test]
fn a_wrapped_description_stays_out_of_the_name_column() {
    let (mut checked_commands, mut checked_options) = (0_usize, 0_usize);
    for (name, cmd) in tree() {
        let items: Vec<String> = cmd
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .collect();

        for (kind, text) in [
            (
                "--help",
                cmd.clone().term_width(MAX_WIDTH).render_long_help(),
            ),
            ("-h", cmd.clone().term_width(MAX_WIDTH).render_help()),
        ] {
            let text = text.to_string();
            let mut section: Option<&str> = None;
            let mut item_indent: Option<usize> = None;

            for line in text.lines() {
                if !line.starts_with(' ') && line.ends_with(':') {
                    section = match line {
                        "Commands:" => Some("commands"),
                        "Options:" => Some("options"),
                        _ => None,
                    };
                    item_indent = None;
                    continue;
                }
                let Some(section) = section else { continue };
                if line.trim().is_empty() {
                    continue;
                }
                let indent = line.len() - line.trim_start().len();
                let first = line.split_whitespace().next().unwrap_or("");
                let looks_like_item = match section {
                    "options" => first.starts_with('-'),
                    _ => items.iter().any(|i| i == first) || first == "help",
                };

                match item_indent {
                    // The first item in the section fixes the column its
                    // siblings sit in.
                    None if looks_like_item => item_indent = Some(indent),
                    Some(col) if indent == col => {
                        if section == "options" {
                            checked_options += 1;
                        } else {
                            checked_commands += 1;
                        }
                        assert!(
                            looks_like_item,
                            "`{name} {kind}` folds a description into the name column: \
                             `{first}` is not one of this command's {section}, so this \
                             line is a continuation sitting at the item indent:\n  {line}"
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    // Counted per section, and derived rather than chosen. The first attempt
    // totalled them, and a mutation that broke command detection then *passed*:
    // the section was silently skipped and the options count alone satisfied the
    // floor. Each kind has to be reached on its own.
    let commands = tree().len();
    let subcommands = cli_command().get_subcommands().count();
    assert!(
        checked_options > commands,
        "only {checked_options} option lines across {commands} commands: the options \
         sections are not being reached"
    );
    assert!(
        checked_commands >= subcommands,
        "only {checked_commands} command lines, against {subcommands} subcommands at the \
         root alone: the commands sections are not being reached"
    );
}

/// `-h` renders differently from `--help` and is what a reader reaches for
/// first, so it is checked too rather than assumed to follow.
#[test]
fn short_help_wraps_as_well() {
    let mut offenders = Vec::new();
    for (name, cmd) in tree() {
        let text = cmd.clone().term_width(MAX_WIDTH).render_help().to_string();
        for line in text.lines() {
            let len = line.chars().count();
            if len > MAX_WIDTH {
                offenders.push(format!("`{name} -h`: {len} columns: {line}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "short help wider than {MAX_WIDTH} columns ({} case(s)):\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// It narrows for a small terminal rather than always printing at the cap.
///
/// Descriptions only. **`clap` never wraps a usage line** -- it is one string,
/// and `aplan amend` overrides it precisely because the generated one did not
/// fit at [`MAX_WIDTH`]. A usage line is the shortest true statement of how to
/// invoke a command, so there is a floor below which it cannot narrow and
/// asserting otherwise would be asserting something false.
#[test]
fn a_narrow_width_is_honoured_by_every_description() {
    let narrow = 40;
    let mut widest = 0;
    let mut worst = String::new();
    for (_, text) in helps(narrow) {
        let mut in_usage = false;
        for line in text.lines() {
            if line.starts_with("Usage:") {
                in_usage = true;
                continue;
            }
            if line.trim().is_empty() {
                in_usage = false;
                continue;
            }
            if in_usage {
                continue;
            }
            let len = line.chars().count();
            if len > widest {
                widest = len;
                worst = line.to_string();
            }
        }
    }
    assert!(
        widest <= narrow,
        "asked for {narrow} columns and a description rendered {widest}: {worst}"
    );
}

/// A redirected run must not depend on the environment that ran it, or the
/// corpus passes on one machine and fails on another.
#[test]
fn piped_help_ignores_columns() {
    let run = |columns: Option<&str>| {
        let mut cmd = common::command(std::path::Path::new("."), &["--help"]);
        match columns {
            Some(v) => cmd.env("COLUMNS", v),
            None => cmd.env_remove("COLUMNS"),
        };
        String::from_utf8_lossy(&cmd.output().expect("running aplan --help").stdout).into_owned()
    };

    let bare = run(None);
    assert_eq!(bare, run(Some("40")), "COLUMNS=40 changed piped help");
    assert_eq!(bare, run(Some("200")), "COLUMNS=200 changed piped help");
    assert!(
        bare.lines().all(|l| l.chars().count() <= MAX_WIDTH),
        "piped help exceeded {MAX_WIDTH} columns"
    );
}
