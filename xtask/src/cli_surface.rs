//! Does the suite invoke the surface the tool ships? (`A1-208`)
//!
//! `ROADMAP.md` section 5 asks for *exhaustiveness guaranteed, not hoped for*
//! across four audit dimensions. This is the third, and it was deferred in a
//! comment inside the corpus to a cut that has since closed.
//!
//! **Both halves are enumerated, neither is listed by hand.** The surface comes
//! from clap's own command tree, so a flag added tomorrow is covered without
//! anyone remembering. What ran comes from two exact sources and no third: the
//! recorder `A1-207` put in the spawn helper, and the `$ aplan` lines in the
//! `trycmd` corpus, which `trycmd` spawns itself and which declare their
//! invocation verbatim. A table saying a flag is covered is a status; the log is
//! the evidence.
//!
//! **Resolution is clap's, not a parser written here.** Every recorded argv is
//! handed back to `try_get_matches_from`, which is the only thing that knows
//! `--config` may precede the subcommand, that `--root` takes a value, and which
//! command a bare word selects. A hand-rolled reader failed twice during
//! `A1-207`'s research for exactly those reasons.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};

/// One thing the CLI offers that a test can exercise.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Member {
    /// A command or subcommand, by its full path: `dep add`.
    Command(String),
    /// A long flag on one command.
    Flag { command: String, long: String },
    /// A long flag clap propagates to every command.
    ///
    /// **One behaviour, not thirty-one.** `--root` appears in every
    /// subcommand's help because it is global; requiring a test per command
    /// would be thirty-one tests of one code path, and an audit that demands
    /// busywork is one that gets turned off.
    Global(String),
    /// An **optional** positional: `aplan init <dir>`.
    ///
    /// A required positional is not a member, because the command cannot run
    /// without it and is therefore covered whenever the command is. An optional
    /// one is a second behaviour behind the same command name, and the first
    /// version of this gate counted neither -- so `init <dir>` was part of the
    /// surface and invisible to the audit (`A1-208`, audit).
    Positional { command: String, name: String },
}

impl std::fmt::Display for Member {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Member::Command(path) => write!(f, "aplan {path}"),
            Member::Flag { command, long } => write!(f, "aplan {command} --{long}"),
            Member::Global(long) => write!(f, "--{long} (global)"),
            Member::Positional { command, name } => write!(f, "aplan {command} <{name}>"),
        }
    }
}

/// `--help` and `--version` are clap's, not the tool's: they exist on every
/// command whether or not anyone declared them, and `A1-205` already gates their
/// rendering.
fn is_claps_own(long: &str) -> bool {
    matches!(long, "help" | "version")
}

/// Everything the binary offers, walked from clap.
pub fn surface(root: &clap::Command) -> BTreeSet<Member> {
    let mut out = BTreeSet::new();
    walk(root, "", &mut out);
    out
}

fn walk(cmd: &clap::Command, prefix: &str, out: &mut BTreeSet<Member>) {
    let path = if prefix.is_empty() {
        String::new()
    } else if prefix == "aplan" {
        cmd.get_name().to_string()
    } else {
        format!("{prefix} {}", cmd.get_name())
    };
    if !path.is_empty() {
        out.insert(Member::Command(path.clone()));
    }
    for arg in cmd.get_arguments() {
        if arg.is_positional() {
            if !arg.is_required_set() && !path.is_empty() {
                out.insert(Member::Positional {
                    command: path.clone(),
                    name: arg.get_id().to_string(),
                });
            }
            continue;
        }
        let Some(long) = arg.get_long() else { continue };
        if is_claps_own(long) {
            continue;
        }
        if arg.is_global_set() {
            out.insert(Member::Global(long.to_string()));
        } else {
            out.insert(Member::Flag {
                command: path.clone(),
                long: long.to_string(),
            });
        }
    }
    let next = if prefix.is_empty() { "aplan" } else { &path };
    for sub in cmd.get_subcommands() {
        walk(sub, next, out);
    }
}

/// What one recorded invocation exercised, resolved by clap.
///
/// Three answers, not two. An argv clap *rejects* exercises nothing, and that is
/// not a hole: a corpus case like `aplan nope` exists precisely because it is
/// invalid, and crediting the flags in a rejected line would count coverage for
/// code that never ran.
///
/// But clap also returns `Err` for `--help` and `--version`, which are neither
/// invalid nor nothing: `aplan add --help` runs `add`. Five of the eleven
/// "refusals" the first version counted were these, so the number was right and
/// the sentence explaining it was wrong (`A1-208`, audit). They credit their
/// command and no flags -- `--help` is clap's own and is not in the surface.
fn exercised_by(root: &clap::Command, argv: &[String]) -> (BTreeSet<Member>, Refusal) {
    let mut out = BTreeSet::new();
    let full: Vec<String> = std::iter::once("aplan".to_string())
        .chain(argv.iter().cloned())
        .collect();
    match root.clone().try_get_matches_from(&full) {
        Ok(matches) => {
            collect(root, &matches, "", &mut out);
            (out, Refusal::None)
        }
        Err(e) if is_a_request_for_text(e.kind()) => {
            if let Some(path) = command_path(root, argv) {
                out.insert(Member::Command(path));
            }
            (out, Refusal::AskedForText)
        }
        Err(_) => (out, Refusal::Rejected),
    }
}

/// Why an argv produced no matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// It parsed.
    None,
    /// `--help` or `--version`: the command ran and printed text.
    AskedForText,
    /// clap would not accept it, which is what an invalid-input case is for.
    Rejected,
}

/// `--help` and `--version` leave clap as errors and are not errors.
fn is_a_request_for_text(kind: clap::error::ErrorKind) -> bool {
    matches!(
        kind,
        clap::error::ErrorKind::DisplayHelp
            | clap::error::ErrorKind::DisplayVersion
            | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    )
}

/// The deepest command the leading words of `argv` name.
///
/// Only for the help case, where clap has told us it will not hand back matches.
/// It reads words, so it stops at the first one that is not a subcommand -- which
/// is right here and would be wrong anywhere else, and is why the rest of this
/// file asks clap instead.
fn command_path(root: &clap::Command, argv: &[String]) -> Option<String> {
    let mut cmd = root;
    let mut path = Vec::new();
    for word in argv {
        if word.starts_with('-') {
            break;
        }
        let Some(sub) = cmd.find_subcommand(word) else {
            break;
        };
        path.push(sub.get_name().to_string());
        cmd = sub;
    }
    (!path.is_empty()).then(|| path.join(" "))
}

fn collect(
    cmd: &clap::Command,
    matches: &clap::ArgMatches,
    prefix: &str,
    out: &mut BTreeSet<Member>,
) {
    let path = if prefix.is_empty() {
        String::new()
    } else if prefix == "aplan" {
        cmd.get_name().to_string()
    } else {
        format!("{prefix} {}", cmd.get_name())
    };
    if !path.is_empty() {
        out.insert(Member::Command(path.clone()));
    }
    for arg in cmd.get_arguments() {
        if arg.is_positional() {
            if !arg.is_required_set()
                && !path.is_empty()
                && matches!(
                    matches.value_source(arg.get_id().as_str()),
                    Some(clap::parser::ValueSource::CommandLine)
                )
            {
                out.insert(Member::Positional {
                    command: path.clone(),
                    name: arg.get_id().to_string(),
                });
            }
            continue;
        }
        let Some(long) = arg.get_long() else { continue };
        if is_claps_own(long) {
            continue;
        }
        // Given on the command line, as against defaulted or unset. A flag with
        // a default value is "present" in every run, and counting that would
        // report the whole surface exercised by one invocation.
        let given = matches
            .try_get_one::<bool>(arg.get_id().as_str())
            .ok()
            .flatten()
            .is_some_and(|v| *v)
            || matches!(
                matches.value_source(arg.get_id().as_str()),
                Some(clap::parser::ValueSource::CommandLine)
            );
        if !given {
            continue;
        }
        if arg.is_global_set() {
            out.insert(Member::Global(long.to_string()));
        } else {
            out.insert(Member::Flag {
                command: path.clone(),
                long: long.to_string(),
            });
        }
    }
    if let Some((name, sub_matches)) = matches.subcommand()
        && let Some(sub) = cmd.find_subcommand(name)
    {
        let next = if prefix.is_empty() { "aplan" } else { &path };
        collect(sub, sub_matches, next, out);
    }
}

/// Every invocation the suite made, from the two sources that observe rather
/// than claim.
pub fn invocations(root: &Path, log: Option<&Path>) -> Result<Vec<Vec<String>>> {
    let mut out = Vec::new();
    if let Some(log) = log {
        let text = std::fs::read_to_string(log)
            .with_context(|| format!("reading the invocation log at {}", log.display()))?;
        for line in text.lines() {
            let argv: Vec<String> = serde_json::from_str(line)
                .with_context(|| format!("a recorded invocation is not JSON: {line}"))?;
            out.push(argv);
        }
    }
    out.extend(corpus_invocations(root)?);
    Ok(out)
}

/// The corpus half: `trycmd` spawns the binary itself, so nothing the recorder
/// can see, and nothing it needs to -- a `.trycmd` file declares its invocation
/// verbatim, one command per line, and a case that did not run fails the suite.
pub fn corpus_invocations(root: &Path) -> Result<Vec<Vec<String>>> {
    let mut out = Vec::new();
    let mut files = Vec::new();
    collect_trycmd(&root.join("crates/aplan/tests/cmd"), &mut files);
    if files.is_empty() {
        bail!(
            "no `.trycmd` files under crates/aplan/tests/cmd, so half the suite's invocations \
             are missing and every flag they cover would be reported unexercised"
        );
    }
    files.sort();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("$ aplan") else {
                continue;
            };
            out.push(shell_words(rest));
        }
    }
    Ok(out)
}

fn collect_trycmd(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_trycmd(&path, out);
        } else if path.extension().is_some_and(|e| e == "trycmd") {
            out.push(path);
        }
    }
}

/// Split a corpus line into argv, honouring the quotes the cases use.
///
/// The same job `common::argv_of` does for documented invocations, and the same
/// reason: `aplan add "Ship it"` is one argument, and a reader splitting on
/// spaces would see two.
fn shell_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, ' ') => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, _) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// What the audit found: the surface, what ran, and what never did.
#[derive(Debug)]
pub struct Audit {
    pub surface: BTreeSet<Member>,
    pub exercised: BTreeSet<Member>,
    pub residue: BTreeSet<Member>,
    pub invocations: usize,
    /// The half the spawn helper observed, kept apart from the half the corpus
    /// declares: a recording that holds nothing is a run that did not happen,
    /// and the corpus would hide that in the total.
    pub recorded: usize,
    /// The invocations clap would not accept, named rather than counted: a
    /// *valid* one that stopped parsing is coverage lost silently, and a number
    /// ticking up says nothing about which.
    pub rejected: Vec<String>,
    /// `--help` and `--version` runs, which are neither invalid nor nothing.
    pub asked_for_text: usize,
}

pub fn audit(root: &Path, log: Option<&Path>) -> Result<Audit> {
    let cmd = aplan::cli_command();
    let surface = surface(&cmd);
    let recorded = match log {
        Some(log) => invocations(root, Some(log))?.len() - corpus_invocations(root)?.len(),
        None => 0,
    };
    let calls = invocations(root, log)?;
    let mut exercised = BTreeSet::new();
    let mut rejected = Vec::new();
    let mut asked_for_text = 0_usize;
    for argv in &calls {
        let (found, why) = exercised_by(&cmd, argv);
        match why {
            Refusal::None => {}
            Refusal::AskedForText => asked_for_text += 1,
            Refusal::Rejected => rejected.push(argv.join(" ")),
        }
        exercised.extend(found);
    }
    rejected.sort();
    rejected.dedup();
    let residue: BTreeSet<Member> = surface.difference(&exercised).cloned().collect();
    Ok(Audit {
        surface,
        exercised,
        residue,
        invocations: calls.len(),
        recorded,
        rejected,
        asked_for_text,
    })
}

/// The residue, grouped for a reader: a command with six unexercised flags is
/// one problem, not six.
pub fn by_command(residue: &BTreeSet<Member>) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for member in residue {
        match member {
            Member::Command(path) => out
                .entry(path.clone())
                .or_default()
                .push("(never run)".into()),
            Member::Flag { command, long } => {
                out.entry(command.clone())
                    .or_default()
                    .push(format!("--{long}"));
            }
            Member::Global(long) => out
                .entry("(global)".to_string())
                .or_default()
                .push(format!("--{long}")),
            Member::Positional { command, name } => {
                out.entry(command.clone())
                    .or_default()
                    .push(format!("<{name}>"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree with the shapes that matter: a global, a per-command flag, a
    /// nested subcommand, and a flag with a value.
    fn toy() -> clap::Command {
        // Built, so clap has added `--help` and the rest of its own arguments.
        // An unbuilt `Command` carries only what the caller declared, which
        // made the assertion about clap's own flags pass against a tree that
        // did not contain any (`A1-208`, audit).
        let mut cmd = raw_toy();
        cmd.build();
        cmd
    }

    fn raw_toy() -> clap::Command {
        clap::Command::new("aplan")
            .arg(clap::Arg::new("root").long("root").global(true).num_args(1))
            .subcommand(
                clap::Command::new("check").arg(
                    clap::Arg::new("deny")
                        .long("deny-warnings")
                        .action(clap::ArgAction::SetTrue),
                ),
            )
            .subcommand(clap::Command::new("init").arg(clap::Arg::new("dir")))
            .subcommand(clap::Command::new("show").arg(clap::Arg::new("id").required(true)))
            .subcommand(clap::Command::new("dep").subcommand(
                clap::Command::new("add").arg(clap::Arg::new("after").long("after").num_args(1)),
            ))
    }

    fn argv(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    /// The members half of [`exercised_by`], for the tests that are about what
    /// an invocation covers rather than why it was refused.
    fn covered(root: &clap::Command, args: &[String]) -> BTreeSet<Member> {
        exercised_by(root, args).0
    }

    /// The surface is what clap declares, with the two clap adds itself left
    /// out: `--help` exists on every command whether or not anyone wrote it, and
    /// counting it would report a third of the surface unexercised forever.
    #[test]
    fn the_surface_is_clap_s_own_tree_without_clap_s_own_flags() {
        let found = surface(&toy());
        assert!(found.contains(&Member::Command("check".into())));
        assert!(found.contains(&Member::Command("dep add".into())));
        assert!(found.contains(&Member::Flag {
            command: "check".into(),
            long: "deny-warnings".into()
        }));
        assert!(found.contains(&Member::Global("root".into())));
        assert!(
            !found
                .iter()
                .any(|m| matches!(m, Member::Flag { long, .. } if long == "help")),
            "clap's own flags are in the surface: {found:?}"
        );
    }

    /// **A global flag is one behaviour, not one per command.** Requiring a test
    /// per command for `--root` would be thirty-one tests of one code path, and
    /// an audit that demands busywork is one that gets turned off.
    #[test]
    fn a_global_flag_is_one_member_however_many_commands_show_it() {
        let found = surface(&toy());
        let globals: Vec<&Member> = found
            .iter()
            .filter(|m| matches!(m, Member::Global(_)))
            .collect();
        assert_eq!(globals.len(), 1, "{globals:?}");
        assert!(
            !found
                .iter()
                .any(|m| matches!(m, Member::Flag { long, .. } if long == "root")),
            "a global was also counted per command: {found:?}"
        );
    }

    /// Resolution is clap's. A global given *before* the subcommand is the case
    /// a hand-rolled reader gets wrong -- `trusted_config.rs` passes `--config`
    /// that way -- and the value after it is a value, not a command.
    #[test]
    fn a_global_before_the_subcommand_is_resolved_to_both() {
        let ran = covered(&toy(), &argv("--root somewhere check"));
        assert!(ran.contains(&Member::Global("root".into())), "{ran:?}");
        assert!(ran.contains(&Member::Command("check".into())), "{ran:?}");
        assert!(
            !ran.contains(&Member::Command("somewhere".into())),
            "the value after a flag was read as a command: {ran:?}"
        );
    }

    /// A flag that was not given is not exercised, which is the whole point: an
    /// invocation of a command must not credit every flag the command takes.
    #[test]
    fn only_the_flags_actually_given_are_counted() {
        let bare = covered(&toy(), &argv("check"));
        assert!(bare.contains(&Member::Command("check".into())));
        assert!(
            !bare.contains(&Member::Flag {
                command: "check".into(),
                long: "deny-warnings".into()
            }),
            "a flag nobody passed was counted: {bare:?}"
        );
        let with = covered(&toy(), &argv("check --deny-warnings"));
        assert!(with.contains(&Member::Flag {
            command: "check".into(),
            long: "deny-warnings".into()
        }));
    }

    /// A nested subcommand resolves to its full path, so `dep add` is not
    /// credited to `dep`.
    #[test]
    fn a_nested_subcommand_resolves_to_its_full_path() {
        let ran = covered(&toy(), &argv("dep add --after X"));
        assert!(ran.contains(&Member::Command("dep add".into())), "{ran:?}");
        assert!(ran.contains(&Member::Flag {
            command: "dep add".into(),
            long: "after".into()
        }));
    }

    /// An argv clap refuses contributes nothing. The corpus holds `aplan nope`
    /// *because* it is invalid, and counting the words in a rejected line would
    /// credit coverage to a run that never reached the code.
    #[test]
    fn an_invocation_clap_refuses_exercises_nothing() {
        assert!(covered(&toy(), &argv("nope")).is_empty());
        assert!(covered(&toy(), &argv("check --invented")).is_empty());
        assert_eq!(exercised_by(&toy(), &argv("nope")).1, Refusal::Rejected);
    }

    /// **`--help` is not a refusal.** clap returns `Err` for it, and reading that
    /// as "invalid input" made five of eleven counted refusals a
    /// misdescription -- `aplan add --help` runs `add` (`A1-208`, audit).
    #[test]
    fn asking_for_help_runs_the_command_it_asks_about() {
        let (members, why) = exercised_by(&toy(), &argv("check --help"));
        assert_eq!(why, Refusal::AskedForText);
        assert!(
            members.contains(&Member::Command("check".into())),
            "the command was not credited: {members:?}"
        );
        assert!(
            !members
                .iter()
                .any(|m| matches!(m, Member::Flag { long, .. } if long == "help")),
            "`--help` is clap's own and is not in the surface: {members:?}"
        );

        let (members, why) = exercised_by(&toy(), &argv("--help"));
        assert_eq!(why, Refusal::AskedForText);
        assert!(members.is_empty(), "the root is not a member: {members:?}");
    }

    /// The word-reading path is used only where clap has declined to hand back
    /// matches, and it stops at the first word that is not a subcommand.
    #[test]
    fn the_help_path_reads_words_only_as_far_as_the_commands_go() {
        assert_eq!(
            command_path(&toy(), &argv("dep add --help")).as_deref(),
            Some("dep add")
        );
        assert_eq!(
            command_path(&toy(), &argv("check T-1")).as_deref(),
            Some("check")
        );
        assert_eq!(command_path(&toy(), &argv("--help")), None);
        assert_eq!(command_path(&toy(), &argv("nope")), None);
    }

    /// A *required* positional is not a member and an **optional** one is.
    ///
    /// The command cannot run without the first, so it is covered whenever the
    /// command is; the second is a second behaviour behind the same name.
    /// `aplan init <dir>` is one, and the first version of this gate counted
    /// neither kind -- so it was part of the surface and invisible.
    #[test]
    fn an_optional_positional_is_a_member_and_a_required_one_is_not() {
        let found = surface(&toy());
        assert!(
            found.contains(&Member::Positional {
                command: "init".into(),
                name: "dir".into()
            }),
            "the optional positional is not in the surface: {found:?}"
        );
        assert!(
            !found.iter().any(|m| matches!(
                m,
                Member::Positional { name, .. } if name == "id"
            )),
            "a required positional was counted: {found:?}"
        );

        assert!(
            covered(&toy(), &argv("init"))
                .iter()
                .all(|m| !matches!(m, Member::Positional { .. }))
        );
        let given = covered(&toy(), &argv("init somewhere"));
        assert!(
            given.contains(&Member::Positional {
                command: "init".into(),
                name: "dir".into()
            }),
            "giving the positional did not count it: {given:?}"
        );
    }

    /// **Nothing in the surface is reachable only by a short flag.**
    ///
    /// The walk skips an argument with no long name, which is silent by
    /// construction: a `-x` with no `--x` would be part of the surface and
    /// invisible to this gate. Nothing declares one today, and this is what says
    /// so on the commit that changes it.
    #[test]
    fn no_argument_is_reachable_only_by_a_short_flag() {
        let mut short_only = Vec::new();
        let mut cmd = aplan::cli_command();
        cmd.build();
        let mut stack = vec![cmd];
        while let Some(cmd) = stack.pop() {
            for arg in cmd.get_arguments() {
                if arg.get_long().is_none() && arg.get_short().is_some() {
                    short_only.push(format!("{} {}", cmd.get_name(), arg.get_id()));
                }
            }
            stack.extend(cmd.get_subcommands().cloned());
        }
        assert!(
            short_only.is_empty(),
            "argument(s) the surface cannot see, because they have no long name: {short_only:?}"
        );
    }

    /// The corpus half is split the way the shell would: `aplan add "Ship it"`
    /// is one argument, and a reader splitting on spaces sees two.
    #[test]
    fn a_corpus_line_keeps_a_quoted_argument_whole() {
        let words = shell_words(" add \"Ship it\" --effort 1");
        assert_eq!(words, vec!["add", "Ship it", "--effort", "1"]);
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    /// The corpus half is read from files, and a directory with none is a
    /// refusal rather than a quiet zero: half the suite's invocations missing
    /// would report every flag they cover as unexercised, which is an accusation
    /// rather than a measurement.
    #[test]
    fn an_empty_corpus_directory_is_a_refusal_not_an_empty_list() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let err = corpus_invocations(dir.path()).expect_err("an empty corpus must refuse");
        assert!(format!("{err}").contains("no `.trycmd` files"), "{err}");
    }

    /// And a real corpus parses: one command per line, `$ aplan` stripped, and
    /// nothing else in the file mistaken for an invocation.
    #[test]
    fn the_corpus_yields_the_invocations_it_declares() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let cmd = dir.path().join("crates/aplan/tests/cmd");
        std::fs::create_dir_all(&cmd).expect("mkdir");
        std::fs::write(
            cmd.join("a.trycmd"),
            "Prose about `aplan check`, which is not an invocation.\n\n```\n\
             $ aplan check --format json\n? 1\n$ aplan add \"Ship it\" --effort 1\n```\n",
        )
        .expect("case");
        let found = corpus_invocations(dir.path()).expect("parsed");
        assert_eq!(
            found,
            vec![
                vec!["check", "--format", "json"],
                vec!["add", "Ship it", "--effort", "1"],
            ],
            "prose or output was read as an invocation: {found:?}"
        );
    }

    /// The residue reads as one problem per command, because a command with six
    /// unexercised flags is one thing to fix.
    #[test]
    fn the_residue_is_grouped_by_the_command_it_belongs_to() {
        let residue: BTreeSet<Member> = [
            Member::Flag {
                command: "add".into(),
                long: "owner".into(),
            },
            Member::Flag {
                command: "add".into(),
                long: "out".into(),
            },
            Member::Global("root".into()),
            Member::Command("attest".into()),
        ]
        .into_iter()
        .collect();
        let grouped = by_command(&residue);
        assert_eq!(grouped["add"], vec!["--out", "--owner"]);
        assert_eq!(grouped["(global)"], vec!["--root"]);
        assert_eq!(grouped["attest"], vec!["(never run)"]);
    }

    /// A member prints as the invocation that would exercise it, which is what a
    /// reader does next.
    #[test]
    fn a_member_reads_as_the_command_that_would_cover_it() {
        assert_eq!(
            Member::Flag {
                command: "dep add".into(),
                long: "after".into()
            }
            .to_string(),
            "aplan dep add --after"
        );
        assert_eq!(Member::Command("attest".into()).to_string(), "aplan attest");
        assert_eq!(Member::Global("root".into()).to_string(), "--root (global)");
    }

    /// The whole audit, over both sources, against the real command tree.
    ///
    /// The parts are tested above; this is the one that says they compose --
    /// that a flag reached only by the recorder and a flag reached only by the
    /// corpus both count, and that the residue is what is left.
    #[test]
    fn the_audit_reads_the_recorder_and_the_corpus_together() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let cmd = dir.path().join("crates/aplan/tests/cmd");
        std::fs::create_dir_all(&cmd).expect("mkdir");
        // Only the corpus knows about this one.
        std::fs::write(
            cmd.join("a.trycmd"),
            "$ aplan summary --format json
",
        )
        .expect("case");
        // Only the recording knows about this one.
        let log = dir.path().join("log.jsonl");
        std::fs::write(&log, "[\"ready\",\"--owner\",\"kim\"]\n").expect("log");

        let found = audit(dir.path(), Some(&log)).expect("audited");
        assert_eq!(found.invocations, 2, "both sources were read");
        assert!(found.exercised.contains(&Member::Command("summary".into())));
        assert!(found.exercised.contains(&Member::Flag {
            command: "ready".into(),
            long: "owner".into()
        }));
        // And the rest of a real CLI is not credited by two invocations.
        assert!(
            found.residue.len() > 100,
            "two invocations covered {} of {} members",
            found.exercised.len(),
            found.surface.len()
        );
        assert_eq!(
            found.surface.len(),
            found.exercised.len() + found.residue.len(),
            "the residue is not the difference"
        );
    }

    /// The two halves are counted apart, so a recording that observed nothing is
    /// visible behind a corpus that always declares something.
    ///
    /// Without the split, an empty log reads as 126 invocations and the gate
    /// reports most of the surface unexercised -- blaming the suite for a run
    /// that never happened (`A1-208`, audit).
    #[test]
    fn an_empty_recording_is_visible_behind_the_corpus() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let cmd = dir.path().join("crates/aplan/tests/cmd");
        std::fs::create_dir_all(&cmd).expect("mkdir");
        std::fs::write(cmd.join("a.trycmd"), "$ aplan summary\n").expect("case");
        let log = dir.path().join("empty.jsonl");
        std::fs::write(&log, "").expect("log");

        let found = audit(dir.path(), Some(&log)).expect("audited");
        assert_eq!(found.recorded, 0, "an empty recording counted as something");
        assert_eq!(found.invocations, 1, "the corpus half is still read");

        std::fs::write(&log, "[\"ready\"]\n").expect("log");
        let found = audit(dir.path(), Some(&log)).expect("audited");
        assert_eq!(found.recorded, 1);
        assert_eq!(found.invocations, 2);
    }

    /// Reading a log that is not there says which file, rather than reporting a
    /// clean surface from no evidence at all.
    #[test]
    fn a_missing_invocation_log_is_an_error_naming_the_path() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let missing = dir.path().join("nowhere.jsonl");
        let err = invocations(dir.path(), Some(&missing)).expect_err("must refuse");
        assert!(format!("{err}").contains("nowhere.jsonl"), "{err}");
    }
}
