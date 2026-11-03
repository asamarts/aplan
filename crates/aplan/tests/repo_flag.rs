//! `--repo` points six commands at a repository that is not the current
//! directory (`A1-208`).
//!
//! One behaviour on six commands, and the suite exercised it on none of them:
//! `A1-207`'s recording plus the corpus showed `--repo` reaching `stale` and
//! nothing else, while `verify`, `paths`, `show`, `attest`, `close` and `amend`
//! all accept it. A flag six commands take and no test invokes is a flag nobody
//! would notice losing.
//!
//! The property is the one the flag exists for and is the same on all six: **the
//! repository opened is the one named, not the one the process is standing in.**
//! Asserted by standing somewhere that is not a repository at all, so a command
//! that ignored the flag fails in a way nothing else explains.

use std::path::Path;

use crate::common;

/// A plan whose closed task cites a commit, in a directory that is deliberately
/// **not** a git repository.
fn plan_outside_a_repo(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: a closed task\nphase: 0\nstatus: done\nowner: k\n\
         effort_days: 1\npaths:\n- f.txt\nevidence:\n  - kind: commit\n    ref: HEAD\n\
         ---\n\nbody\n",
    )
    .expect("task");
}

/// A real repository, elsewhere.
fn repository(dir: &Path) {
    std::fs::create_dir_all(dir).expect("mkdir");
    std::fs::write(dir.join("f.txt"), "x\n").expect("file");
    let repo = git2::Repository::init(dir).expect("git init");
    let mut index = repo.index().expect("index");
    index
        .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add");
    index.write().expect("write index");
    let tree = repo
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree");
    let sig = git2::Signature::now("t", "t@e.com").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "one", &tree, &[])
        .expect("commit");
}

/// Whether the answer is "there is no repository here".
fn complains_about_git(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("not a git repository")
        || lower.contains("could not find repository")
        || lower.contains("is not a git repository")
}

/// **Every command that takes `--repo` opens the repository it names.**
///
/// Enumerated from clap rather than listed here, so a seventh command gaining
/// the flag is covered on the commit that declares it -- which is the whole
/// argument of `A1-208`, applied to the one flag this file is about.
#[test]
fn every_command_taking_repo_opens_the_one_it_names() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let elsewhere = dir.path().join("elsewhere");
    repository(&elsewhere);
    let named = elsewhere.display().to_string();

    // What each command needs before `--repo` is reached at all. A command
    // refused for a missing argument never opens anything, and would pass this
    // test twice over.
    let extra = |command: &str| -> Vec<String> {
        match command {
            "show" | "close" => vec!["T-1".into()],
            // A commit citation, not a test one: `test:` resolves through the
            // configured runner and never opens git, so the command would
            // succeed without a repository and prove nothing here.
            "amend" => vec![
                "T-1".into(),
                "--add-evidence".into(),
                "commit:1234567".into(),
            ],
            _ => Vec::new(),
        }
    };

    let cli = aplan::cli_command();
    let takes_repo: Vec<String> = cli
        .get_subcommands()
        .filter(|sub| {
            sub.get_arguments()
                .filter_map(clap::Arg::get_long)
                .any(|l| l == "repo")
        })
        .map(|sub| sub.get_name().to_string())
        .collect();
    assert!(
        takes_repo.len() >= 6,
        "only {} command(s) take `--repo`, so the walk found less than the surface: {takes_repo:?}",
        takes_repo.len()
    );

    let mut ignored = Vec::new();
    let mut opens_git = 0_usize;
    for command in &takes_repo {
        let mut args: Vec<String> = vec![command.clone()];
        args.extend(extra(command));
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();

        // A plan of its own per command, and a second for the run with the
        // flag: `close` and `amend` write, so a shared fixture would make the
        // answer depend on which command clap happens to declare first.
        let here_plan = dir.path().join(format!("{command}-here"));
        let there_plan = dir.path().join(format!("{command}-there"));
        plan_outside_a_repo(&here_plan);
        plan_outside_a_repo(&there_plan);

        let here = common::run(&here_plan, &borrowed);
        let mut with = borrowed.clone();
        with.extend(["--repo", &named]);
        let there = common::run(&there_plan, &with);

        // The weakest honest property, and it holds for all six: naming a
        // repository changes the answer.
        if here.text() == there.text() && here.code == there.code {
            ignored.push(format!("`aplan {command}`: `--repo` changed nothing"));
        }

        // And the sharper one, wherever the command opens git at all: standing
        // outside a repository it says so, and pointed at one it stops saying
        // so. `amend` is the exception, and not a benign one -- it writes the
        // citation without resolving it, which is `A1-238`.
        if complains_about_git(&here.text()) {
            opens_git += 1;
            if complains_about_git(&there.text()) {
                ignored.push(format!(
                    "`aplan {command} --repo <elsewhere>` opened the current directory anyway"
                ));
            }
        } else if command != "amend" {
            ignored.push(format!(
                "`aplan {command}` opened no repository outside one, so nothing here tests \
                 `--repo`: {}",
                here.text()
            ));
        }
    }

    assert!(
        opens_git >= 5,
        "only {opens_git} command(s) opened git at all, so this file is testing almost nothing"
    );
    assert!(
        ignored.is_empty(),
        "{} problem(s) with `--repo`:\n  {}",
        ignored.len(),
        ignored.join("\n  ")
    );
}
