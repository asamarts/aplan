//! The verification ladder (HLD §5).
//!
//! `aplan check` asks only whether evidence is *present and well-formed* (rung
//! L0), it is hermetic: no git, no network, no subprocess. `aplan verify` opens
//! the repository and asks whether the evidence *resolves* (L1) and is *relevant*
//! (L2):
//!
//! - a `commit` SHA resolves to a real commit (`APLAN011`) whose diff touches at
//!   least one of the task's `paths` (`APLAN012`);
//! - a `file` path exists at the cited rev, or at `HEAD` (`APLAN015`).
//!
//! The claim-to-artifact binding of L2, *this* cited commit touches *this*
//! task's declared files, is the differentiator (HLD §5.2): it is what a linter
//! that only checks a field is non-empty cannot do, and what the
//! execution-harnesses that run tests do not check either.
//!
//! `test` evidence adds two more rungs: L1 (the test exists, `APLAN013`) and L3
//! (it passes, `APLAN014`, under `--execute`). Both need a runner, and the
//! runner is the security-critical part: a task file names only a test, never a
//! command, and the name is substituted into a template from the trusted
//! `.aplan.yml` and spawned with no shell (see `aplan_config`, HLD §5.5).
//!
//! Adding an evidence kind is a new [`EvidenceResolver`] impl, not a change to
//! [`verify`]. `url` reachability stays absent: it is opt-in, non-deterministic,
//! and needs config keys that are v0.4.

mod repo;
mod resolve;

pub use repo::{Breadth, Repo};
pub use resolve::claims_locality;
pub use resolve::report_dilution;
pub use resolve::{Breach, Cost, Unenforced};
pub use resolve::{CommitResolver, EvidenceResolver, FileResolver, TestResolver, UrlResolver};

use std::path::Path;

/// The runner types this crate takes in [`VerifyOptions`].
///
/// Re-exported because they are part of this crate's public surface: a caller
/// that constructs `VerifyOptions` has to name `test_runner`'s type, and one
/// that depends on `aplan-evidence` and not on `aplan-config` could not
/// (`A1-253`, found writing the agent surface's test).
pub use aplan_config::{TestRunner, TestRunners};
use aplan_core::{EvidenceKind, Plan, Report};

/// A failure that prevents verification from running at all, distinct from a
/// *finding*, which is a diagnostic in the returned [`Report`].
///
/// Loading and validating the runner is `aplan_config`'s job (and error type);
/// by the time a runner reaches [`verify`] it is already trusted.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // Command-neutral: this is raised by both `verify` (resolving evidence) and
    // `close` (deriving HEAD), so the message must not name one of them.
    #[error("`{0}` is not a git repository")]
    NotARepository(std::path::PathBuf),
    #[error("the repository at `{0}` has no commits yet")]
    NoCommits(std::path::PathBuf),
    /// A configured command could not be run. The message names *which* key
    /// configured it: reporting a `url.check` failure as "running the test
    /// runner" sends the reader to the wrong half of their config (`A1-041`).
    #[error("running {0}")]
    Exec(String),
    #[error(transparent)]
    Git(#[from] git2::Error),
}

/// Options for a verification run.
#[derive(Debug, Default, Clone)]
pub struct VerifyOptions {
    /// Run `test` evidence (rung L3), not just resolve it. Off by default, and
    /// CI must not enable it for pull requests from forks (HLD §5.5).
    pub execute: bool,
    /// The trusted test runner(s) from `.aplan.yml`. `None` leaves `test`
    /// evidence at L0 (present-and-well-formed), the way `url` is not fetched by
    /// default. Several runners are supported because one `list` command cannot
    /// see the whole corpus of an umbrella or polyglot repository (`A1-054`).
    pub test_runner: Option<TestRunners>,
    /// Files a cited commit may touch outside a task's `paths` before `APLAN024`
    /// reports the citation as diluted. `None` uses the built-in default; `0`
    /// turns the rule off.
    pub max_unrelated_files: Option<usize>,
    /// The command that decides whether a `url` citation answers (`APLAN016`).
    /// `None` leaves `url` evidence at L0, the way `test` has no L1 without a
    /// runner: no key, no network (`A1-041`).
    pub url_check: Option<aplan_config::UrlCheck>,
    /// Where `.aplan.yml` is, so a finding about the *configuration* has a
    /// location like every other one (`A1-066`, on `APLAN029`'s reasoning: an
    /// annotation lands on the file in the diff rather than being a
    /// locationless sentence). `None` where there is no config to blame.
    pub config_file: Option<std::path::PathBuf>,
}

/// The full SHA of `HEAD` in the repository at `repo_root`, for `aplan close` to
/// record the commit just made as evidence. Errors if there is no repository or
/// no commit yet.
pub fn head_sha(repo_root: &Path) -> Result<String, Error> {
    Repo::discover(repo_root)?
        .head_sha()
        .ok_or_else(|| Error::NoCommits(repo_root.to_path_buf()))
}

/// The resolvers this configuration builds, which is what decides whether a
/// citation gets opened at all.
///
/// **One list, read twice.** `verify` runs them, and [`unresolved_by_verify`]
/// asks which kinds they cover so a run can say what it did not open. Deriving
/// that answer separately from the config would be a second implementation of
/// this decision, and the two would disagree the first time a resolver's
/// condition changed (`A1-253`).
fn resolvers_for(opts: &VerifyOptions, repo_root: &Path) -> Vec<Box<dyn EvidenceResolver>> {
    let mut resolvers: Vec<Box<dyn EvidenceResolver>> = vec![
        Box::new(CommitResolver::new(opts.max_unrelated_files)),
        Box::new(FileResolver),
    ];
    // The url resolver joins only when a checker is configured, for the same
    // reason: an adopter who has not asked for network access does not get it.
    if let Some(url) = &opts.url_check {
        resolvers.push(Box::new(UrlResolver::new(
            url,
            repo_root.to_path_buf(),
            opts.config_file.clone(),
        )));
    }
    // The test resolver joins only when a runner is configured; without one,
    // `test` evidence has no L1/L3 and stays at L0.
    if let Some(runners) = &opts.test_runner {
        resolvers.push(Box::new(TestResolver::new(
            runners.all().to_vec(),
            repo_root.to_path_buf(),
            opts.execute,
            opts.config_file.clone(),
        )));
    }
    resolvers
}

/// What a `verify` run under these options will not open, and why.
///
/// The mirror of [`aplan_core::unresolved_by_check`], which counts what a
/// hermetic run leaves for this one. Here the reason is the configuration
/// rather than the command: a `test` citation stays at L0 when no runner is
/// declared, and a `url` one when no checker is, so the run resolves the other
/// kinds and reported `evidence resolved` over citations nothing opened
/// (`A1-253`, pre-public adoption evidence item 3).
pub use aplan_core::Unopened;

/// Count the citations this configuration leaves unopened.
///
/// `note` is excluded, exactly as `unresolved_by_check` excludes it: a dated
/// note is recorded rather than resolved by design, so counting it here would
/// make the note fire on every plan that has one and mean nothing.
#[must_use]
pub fn unresolved_by_verify(plan: &Plan, repo_root: &Path, opts: &VerifyOptions) -> Unopened {
    let covered: Vec<EvidenceKind> = resolvers_for(opts, repo_root)
        .iter()
        .map(|r| r.kind())
        .collect();
    let mut out = Unopened::default();
    for lt in &plan.tasks {
        let mut here = 0;
        for ev in &lt.task.evidence {
            if ev.kind == EvidenceKind::Note || covered.contains(&ev.kind) {
                continue;
            }
            here += 1;
            if !out.kinds.contains(&ev.kind) {
                out.kinds.push(ev.kind);
            }
        }
        if here > 0 {
            out.tasks += 1;
            out.citations += here;
        }
    }
    out
}

/// Resolve every task's evidence against the repository at `repo_root`.
///
/// Returns a [`Report`] of findings (empty when all evidence resolves), or an
/// [`Error`] if the repository cannot be opened. A defect in one task is a
/// finding about that task, never a reason to stop and report nothing about the
/// rest of the plan (`A1-066`).
/// `verify`, and what the configured commands cost while doing it (`A1-309`).
///
/// A separate entry point rather than a field on [`Report`]: that type is
/// constructed in thirty-two places and serialised into every output format, so
/// widening it would be a consumer contract change for a number most consumers
/// do not want. A caller that wants the figure asks for it.
pub fn verify_measured(
    plan: &Plan,
    repo_root: &Path,
    opts: &VerifyOptions,
) -> Result<(Report, Vec<(String, Cost)>), Error> {
    let mut spend = Vec::new();
    let report = verify_inner(plan, repo_root, opts, &mut spend)?;
    Ok((report, spend))
}

pub fn verify(plan: &Plan, repo_root: &Path, opts: &VerifyOptions) -> Result<Report, Error> {
    verify_inner(plan, repo_root, opts, &mut Vec::new())
}

fn verify_inner(
    plan: &Plan,
    repo_root: &Path,
    opts: &VerifyOptions,
    spend: &mut Vec<(String, Cost)>,
) -> Result<Report, Error> {
    let repo = Repo::discover(repo_root)?;
    let resolvers = resolvers_for(opts, repo_root);

    // `APLAN019`'s input, read once: the index does not change under us, and a
    // plan with a hundred tasks would otherwise walk it a hundred times. Computed
    // only when some task actually declares `paths`.
    let tracked = if plan.tasks.iter().any(|lt| !lt.task.paths.is_empty()) {
        repo.tracked_paths()?
    } else {
        Vec::new()
    };

    let mut diagnostics = Vec::new();
    for lt in &plan.tasks {
        // Independent of evidence: a task's ownership claim can be dead whether
        // or not it cites anything, and catching it on a `todo` task is the point
        // (`APLAN019`, `A1-022`).
        resolve::report_unmatched_paths(&repo, &lt.task, &lt.path, &tracked, &mut diagnostics)?;
        for ev in &lt.task.evidence {
            for r in &resolvers {
                if r.kind() == ev.kind {
                    r.resolve(&repo, &lt.task, &lt.path, ev, &mut diagnostics)?;
                }
            }
        }
        // Findings about the task's proof as a whole, judged once every citation
        // of that kind has been resolved.
        for r in &resolvers {
            r.finish_task(&repo, &lt.task, &lt.path, &mut diagnostics)?;
        }
    }

    // What is wrong with the run rather than with a task: a runner that could
    // not be asked is reported here, once, having taken nothing else down with
    // it (`A1-066`).
    for r in &resolvers {
        spend.extend(r.spend());
        r.finish_plan(&mut diagnostics);
    }

    // The same placement the validator does, for the same reason it does it:
    // an evidence finding is about the `evidence:` field, and a run that named
    // the line for one rule and not for another would be the very inconsistency
    // `A1-202` set out to remove (`A1-202`).
    aplan_core::validate::locate_findings(&mut diagnostics, plan);

    // The same total order the validator uses, so `verify` output is stable
    // across runners.
    diagnostics.sort_by(|a, b| {
        a.code
            .cmp(b.code)
            .then(a.task.cmp(&b.task))
            .then(a.file.cmp(&b.file))
            .then(a.message.cmp(&b.message))
    });

    Ok(Report {
        diagnostics,
        tasks_checked: plan.tasks.len(),
    })
}

/// What resolving a proposed closure found, and what nothing was configured to
/// check.
#[derive(Debug)]
pub struct CloseResolution {
    /// Findings against the evidence as if the task were already closed.
    pub report: Report,
    /// Evidence kinds present in the citation that no resolver looked at, so a
    /// caller can say they were *skipped* rather than let silence read as passed.
    pub unchecked: Vec<EvidenceKind>,
    /// Whether the reason nothing was checked is that there was no repository to
    /// open, rather than nothing configured to do the checking.
    ///
    /// Two causes reach a caller as one empty result, and they send a reader to
    /// different places: a missing `test` runner is a config question, and a plan
    /// kept outside a repository is not. Saying "nothing is configured" to
    /// somebody standing outside one is the shape of wrong turn `A1-244` records
    /// about `APLAN018` (`A1-238`, audit).
    pub repository_unreadable: bool,
}

impl CloseResolution {
    /// Whether `close` must refuse rather than record.
    ///
    /// Deliberately the same predicate the build uses: whatever would fail
    /// `verify` fails `close`, so the two cannot disagree about the same
    /// citation. A repository that treats warnings as blocking gets that here
    /// too, rather than a second severity policy invented for closing
    /// (`A1-109`).
    ///
    /// The whole [`Verdict`](aplan_core::Verdict), not one flag of it. Taking
    /// only `deny_warnings` was that disagreement rebuilt: a repository that made
    /// a rule advisory got a green `verify` and a `close` that refused the same
    /// citation, so the tool contradicted itself about its central claim again,
    /// one config key later (`A1-112`).
    #[must_use]
    pub fn refuses(&self, policy: &aplan_core::Verdict) -> bool {
        self.report.denied_by(policy)
    }

    /// The kinds nothing checked, named for a human.
    #[must_use]
    pub fn unchecked_names(&self) -> Vec<&'static str> {
        self.unchecked.iter().map(|k| k.as_str()).collect()
    }
}

/// Resolve the evidence a `close` proposes, before anything is written.
///
/// The first pilot closed a task citing three integration tests, was told
/// `closed`, and `verify` rejected all three seconds later: the citations used
/// `guider::name` where the runner lists them bare
/// (pre-public adoption evidence, item 10). The tool contradicted itself about its
/// own central claim, and the task sat `done` on evidence the project's own gate
/// rejects (`A1-109`).
///
/// This is deliberately not a second implementation of the ladder. It builds the
/// one-task plan that closing would produce and runs [`verify`] on it, so `close`
/// and `verify` cannot drift: whatever one accepts, the other does.
///
/// Lives here rather than in the CLI so the MCP surface resolves identically
/// (`A1-104`); four pieces of shared behaviour have already been found on the
/// wrong side of that boundary.
pub fn resolve_proposed(
    task: &aplan_core::Task,
    path: &Path,
    evidence: &[aplan_core::Evidence],
    repo_root: &Path,
    opts: &VerifyOptions,
) -> Result<CloseResolution, Error> {
    let mut proposed = task.clone();
    proposed.status = aplan_core::Status::Done;
    proposed.evidence = evidence.to_vec();

    // Which kinds a resolver would actually look at. `url` has no rung built
    // (`APLAN016`, `A1-041`) and a `note` is unresolvable by construction, so
    // both are reported as unchecked rather than quietly accepted.
    let mut unchecked: Vec<EvidenceKind> = Vec::new();
    for ev in evidence {
        let checked = match ev.kind {
            EvidenceKind::Commit | EvidenceKind::File => true,
            EvidenceKind::Test => opts.test_runner.is_some(),
            EvidenceKind::Url | EvidenceKind::Note => false,
        };
        if !checked && !unchecked.contains(&ev.kind) {
            unchecked.push(ev.kind);
        }
    }

    let plan = Plan {
        tasks: vec![aplan_core::load::LoadedTask {
            task: proposed,
            // No `fields`: this is the task as it *would* be, not as it is on
            // disk, so the file's line numbers do not describe it (`A1-202`).
            fields: aplan_core::load::FieldLines::new(),
            // Nor criteria: this resolves one proposed citation, and the body
            // is not what it is about (`A1-224`).
            criteria: aplan_core::load::Criteria::default(),
            path: path.to_path_buf(),
        }],
        ..Plan::default()
    };
    match verify(&plan, repo_root, opts) {
        Ok(report) => Ok(CloseResolution {
            report,
            unchecked,
            repository_unreadable: false,
        }),
        // A plan kept outside a repository, or in one with no commits yet, is a
        // real situation and not a reason to refuse a closure: `close` was
        // hermetic before this and must not start demanding git. Every kind
        // becomes unchecked, so the caller says so instead of implying a pass.
        Err(Error::NotARepository(_) | Error::NoCommits(_)) => {
            for ev in evidence {
                if !unchecked.contains(&ev.kind) {
                    unchecked.push(ev.kind);
                }
            }
            Ok(CloseResolution {
                report: Report {
                    diagnostics: Vec::new(),
                    tasks_checked: 1,
                },
                unchecked,
                repository_unreadable: true,
            })
        }
        Err(e) => Err(e),
    }
}

/// When each path was last changed, and when the repository last moved
/// (`A1-020`).
///
/// FIELD-RESEARCH 7.3 found "how long has this sat unchanged" the
/// highest-signal drift metric in the wild, recoverable from git without a
/// database: systemd deletes the line, and git's own process evicts a topic from
/// `seen` after three inactive weeks. Both read history rather than a tracker.
///
/// **One walk, not one per file.** A `git log -1 -- <path>` per task is a
/// history walk per task; this walks `HEAD` once, newest first, and records the
/// first commit that touched each path. On this repository that is one pass over
/// ~400 commits instead of 164 of them.
///
/// **`now` is `HEAD`'s commit time, not the clock.** A report that changes
/// between two runs at one commit cannot be quoted, and the question is about
/// the repository rather than about the reader's afternoon. In an active
/// repository the two agree to within a commit.
///
/// # Errors
///
/// [`Error`] when the repository cannot be opened or walked.
pub fn last_touched(
    repo_root: &Path,
    paths: &[std::path::PathBuf],
) -> Result<(std::collections::BTreeMap<std::path::PathBuf, i64>, i64), Error> {
    let repo = Repo::discover(repo_root)?;
    repo.last_touched(paths)
}

/// How many of the last `sample` commits would satisfy each task's `paths`
/// (`A1-151`).
///
/// The first pilot's finding was a task declaring `crates/p0000004-core/**`,
/// where nearly all that project's work happens: `APLAN012` asks whether the
/// cited commit touched the declared paths, and at that breadth almost any
/// commit does. Breadth against the *file tree* does not separate it -- it
/// matches 22% of tracked files, while legitimately narrow tasks match more --
/// but breadth against *commits* does (pre-public adoption evidence item 11).
///
/// This measures; it judges nothing. What the number means depends on the plan
/// it sits in, which is the whole finding of the measurement behind `A1-151`: at
/// 47% the pilot's task is *below* its own repository's median, while the same
/// number in a well-partitioned plan would be extraordinary.
///
/// One walk of `HEAD`, `sample` commits deep, diffing each against its first
/// parent -- the shape `APLAN024` already pays for, once per run rather than
/// once per task.
///
/// # Errors
///
/// [`Error`] when the repository cannot be opened or walked.
pub fn commit_breadth(
    repo_root: &Path,
    tasks: &[(String, Vec<String>)],
    sample: usize,
) -> Result<Breadth, Error> {
    let repo = Repo::discover(repo_root)?;
    repo.commit_breadth(tasks, sample)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_config::PLACEHOLDER;
    use aplan_config::TestRunner;
    use aplan_core::task::{Evidence, EvidenceKind, Status, Task};
    use aplan_core::{Plan, code, load::LoadedTask};
    use git2::{Repository, Signature};
    use std::path::PathBuf;

    /// Write `files` into the repo's work tree and commit them, returning the
    /// commit SHA as a hex string.
    fn commit(repo: &Repository, files: &[(&str, &str)], msg: &str) -> String {
        let workdir = repo.workdir().expect("non-bare repo").to_path_buf();
        for (p, contents) in files {
            let full = workdir.join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, contents).unwrap();
        }
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
            .unwrap()
            .to_string()
    }

    /// A repo in a tempdir, kept alive alongside it.
    fn init() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        (dir, repo)
    }

    fn done_task(id: &str, evidence: Vec<Evidence>, paths: Vec<String>) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 0,
            status: Status::Done,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            evidence,
            paths,
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        }
    }

    fn commit_ev(sha: &str) -> Evidence {
        Evidence {
            kind: EvidenceKind::Commit,
            reference: sha.to_string(),
            captured: None,
        }
    }

    fn file_ev(reference: &str) -> Evidence {
        Evidence {
            kind: EvidenceKind::File,
            reference: reference.to_string(),
            captured: None,
        }
    }

    fn test_ev(name: &str) -> Evidence {
        Evidence {
            kind: EvidenceKind::Test,
            reference: name.to_string(),
            captured: None,
        }
    }

    fn runner(list: &[&str], run: &[&str]) -> TestRunner {
        TestRunner {
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers: None,
            covers_by_test: None,
            name: None,
            dir: None,
            list: list.iter().map(|s| (*s).to_string()).collect(),
            run: run.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// **The kinds a configuration can resolve are exactly the resolvers it
    /// builds**, which is the assumption `unresolved_by_verify` rests on.
    ///
    /// It reads the same list `verify` runs, so the two cannot disagree about
    /// what was opened. This pins the mapping itself: a resolver that started
    /// joining unconditionally, or stopped joining at all, would change what a
    /// run reports it did not check, and that is a consumer-visible number
    /// (`A1-253`).
    #[test]
    fn a_resolver_joins_exactly_when_its_configuration_declares_it() {
        let (dir, _repo) = init();
        let kinds = |opts: &VerifyOptions| -> Vec<EvidenceKind> {
            resolvers_for(opts, dir.path())
                .iter()
                .map(|r| r.kind())
                .collect()
        };

        let bare = VerifyOptions::default();
        assert_eq!(
            kinds(&bare),
            vec![EvidenceKind::Commit, EvidenceKind::File],
            "git is always there, so these two never need declaring"
        );

        let with_test = VerifyOptions {
            test_runner: Some(TestRunners::One(runner(&["true"], &["true", PLACEHOLDER]))),
            ..VerifyOptions::default()
        };
        assert!(
            kinds(&with_test).contains(&EvidenceKind::Test),
            "a declared runner is what puts `test` on the ladder"
        );
        assert!(
            !kinds(&bare).contains(&EvidenceKind::Test),
            "and without one it is not, which is the whole finding"
        );
    }

    /// **A `test` citation with no runner is counted, and a `note` is not.**
    ///
    /// The note is the boundary: it is recorded rather than resolved by design,
    /// so counting it would fire this on every plan holding one and say nothing.
    #[test]
    fn unresolved_by_verify_counts_what_no_resolver_covers() {
        let (dir, _repo) = init();
        let opts = VerifyOptions::default();

        let cited = plan_of(done_task("A", vec![test_ev("a_name")], vec![]));
        let left = unresolved_by_verify(&cited, dir.path(), &opts);
        assert_eq!((left.citations, left.tasks), (1, 1));
        assert_eq!(left.kinds, vec![EvidenceKind::Test]);
        assert_eq!(left.kinds_said(), "`test`");

        let noted = plan_of(done_task(
            "A",
            vec![Evidence {
                kind: EvidenceKind::Note,
                reference: "a phone call".into(),
                captured: Some("2026-09-02".into()),
            }],
            vec![],
        ));
        assert!(
            !unresolved_by_verify(&noted, dir.path(), &opts).any(),
            "a dated note is not a citation anything was going to open"
        );

        let opened = plan_of(done_task("A", vec![file_ev("f.txt")], vec![]));
        assert!(
            !unresolved_by_verify(&opened, dir.path(), &opts).any(),
            "a kind with a resolver is opened, whatever the resolver then finds"
        );
    }

    /// **Counting what was not opened opens nothing.**
    ///
    /// `unresolved_by_verify` builds the resolver list a second time, and would
    /// be a second execution of the adopter's runner if constructing one ran
    /// anything. It does not, and this is the guard: the fixture's `list`
    /// command leaves a marker, and the marker must not appear.
    #[test]
    fn counting_does_not_run_the_configured_commands() {
        let (dir, _repo) = init();
        let marker = dir.path().join("ran");
        let opts = VerifyOptions {
            test_runner: Some(TestRunners::One(runner(
                &["touch", marker.to_str().expect("utf8")],
                &["true", PLACEHOLDER],
            ))),
            ..VerifyOptions::default()
        };
        let plan = plan_of(done_task("A", vec![test_ev("a_name")], vec![]));

        let _ = unresolved_by_verify(&plan, dir.path(), &opts);
        assert!(
            !marker.exists(),
            "counting is a question about the configuration, not a second run of it"
        );

        // And the fixture can leave the marker, so its absence above is evidence
        // rather than a runner that was never going to run.
        let _ = verify(&plan, dir.path(), &opts);
        assert!(
            marker.exists(),
            "this runner does leave a marker when something actually asks it to list"
        );
    }

    fn plan_of(task: Task) -> Plan {
        Plan {
            tasks: vec![LoadedTask {
                task,
                path: PathBuf::from("plan/tasks/x.md"),
                fields: aplan_core::load::FieldLines::new(),
                criteria: aplan_core::load::Criteria::default(),
            }],
            load_diagnostics: Vec::new(),
            unloadable: std::collections::BTreeMap::new(),
        }
    }

    fn codes(report: &Report) -> Vec<&str> {
        report.diagnostics.iter().map(|d| d.code).collect()
    }

    #[test]
    fn a_real_relevant_commit_is_clean() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("src/load.rs", "fn main() {}")], "add loader");
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(report.is_clean(), "{:?}", report.diagnostics);
    }

    /// `APLAN031`: a cited file that exists and is not this task's (`A1-045`).
    ///
    /// `APLAN012` makes a commit citation mean "a commit that touched this work".
    /// A file citation meant "the repository has this file" until now, and
    /// `require_min_strength` counts `file` among the strong kinds -- so the
    /// fourth pilot closed a task about ADR authorship with a Containerfile, in
    /// one command, and `check` and `verify` both said `ok`.
    #[test]
    fn a_file_outside_the_tasks_paths_fires_031() {
        let (dir, repo) = init();
        commit(
            &repo,
            &[("src/load.rs", "fn main() {}"), ("README.md", "hi")],
            "init",
        );

        // The pilot's shape exactly: a real file, and not this task's.
        let outside = done_task("A", vec![file_ev("README.md")], vec!["src/**".into()]);
        let report = verify(&plan_of(outside), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_IRRELEVANT]);
        assert!(
            report.diagnostics[0]
                .message
                .contains("outside this task's"),
            "{:?}",
            report.diagnostics[0]
        );

        // Inside them, and silent: the citation now says something about the task.
        let inside = done_task("A", vec![file_ev("src/load.rs")], vec!["src/**".into()]);
        let report = verify(&plan_of(inside), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(report.is_clean(), "{:?}", report.diagnostics);

        // With no `paths` there is nothing to judge against, and that gap is
        // `APLAN023`'s to report: one gap, one code (`A1-043`).
        let unbound = done_task("A", vec![file_ev("README.md")], Vec::new());
        let report = verify(&plan_of(unbound), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::FILE_IRRELEVANT),
            "{:?}",
            report.diagnostics
        );

        // A file that does not exist is `APLAN015`, and is not also reported as
        // irrelevant: "it is not there" and "it is not yours" are different
        // faults with different remedies. Outside the paths as well as missing,
        // so the second code would appear if the first did not stop.
        let missing = done_task(
            "A",
            vec![file_ev("elsewhere/nope.rs")],
            vec!["src/**".into()],
        );
        let report = verify(&plan_of(missing), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_UNRESOLVED]);

        // `paths` come from a task file, so a claim to own every file must fail
        // relevance rather than satisfy it, exactly as `APLAN012` reads one.
        let wild = done_task("A", vec![file_ev("README.md")], vec!["**".into()]);
        let report = verify(&plan_of(wild), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_IRRELEVANT]);
        assert!(
            report.diagnostics[0].message.contains("matches every file"),
            "{:?}",
            report.diagnostics[0]
        );
    }

    /// `APLAN024`: the citation overlaps, but is it *about* this task?
    ///
    /// `tag` keeps successive commits disjoint: re-writing the same paths with
    /// the same bytes produces an empty diff, and a fixture like that would test
    /// nothing while looking like it tested a lot.
    fn sprawling_commit(repo: &Repository, tag: &str, inside: usize, outside: usize) -> String {
        let mut owned: Vec<(String, String)> = Vec::new();
        for i in 0..inside {
            owned.push((format!("src/{tag}{i}.rs"), tag.to_string()));
        }
        for i in 0..outside {
            owned.push((format!("elsewhere/{tag}{i}.txt"), tag.to_string()));
        }
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        commit(repo, &refs, "sprawl")
    }

    #[test]
    fn a_commit_mostly_about_other_work_is_reported() {
        // The shape the p00025 pilot measured: one relevant file in a scaffold
        // commit, so `APLAN012` passes on an overlap any `paths` list would have.
        let (dir, repo) = init();
        let sha = sprawling_commit(&repo, "a", 1, 60);
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::DILUTED_EVIDENCE)
            .expect("the dilution must be reported");
        assert!(d.message.contains("60 path(s) outside"), "{}", d.message);
        // Advisory: the citation may be honest, and `APLAN012` already passed.
        assert!(report.is_clean(), "{:?}", report.diagnostics);
    }

    #[test]
    fn size_alone_is_not_the_finding() {
        // A commit just as large, wholly inside the task's `paths`, is focused.
        let (dir, repo) = init();
        let sha = sprawling_commit(&repo, "a", 61, 0);
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::DILUTED_EVIDENCE),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_task_with_other_proof_stays_quiet() {
        // A cited test says what a diff cannot, so the dilution stops mattering.
        let (dir, repo) = init();
        let sha = sprawling_commit(&repo, "a", 1, 60);
        let task = done_task(
            "A",
            vec![commit_ev(&sha), test_ev("mod::name")],
            vec!["src/**".into()],
        );
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::DILUTED_EVIDENCE),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn one_focused_citation_is_proof_however_many_others_sprawl() {
        // A task citing the commit that did the work and the merge that carried
        // it is evidenced; warning about the second would be noise.
        let (dir, repo) = init();
        let sprawl = sprawling_commit(&repo, "a", 1, 60);
        let focused = commit(&repo, &[("src/a.rs", "y")], "the actual work");
        let task = done_task(
            "A",
            vec![commit_ev(&sprawl), commit_ev(&focused)],
            vec!["src/**".into()],
        );
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::DILUTED_EVIDENCE),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn the_reported_count_belongs_to_the_commit_the_message_names() {
        // Two sprawling citations of different sizes. The count comes from the
        // least-diluted commit, so naming the *first* one attributes a number to a
        // commit that does not have it.
        let (dir, repo) = init();
        let worst = sprawling_commit(&repo, "w", 1, 200);
        let best = sprawling_commit(&repo, "a", 1, 60);
        let task = done_task(
            "A",
            vec![commit_ev(&worst), commit_ev(&best)],
            vec!["src/**".into()],
        );
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::DILUTED_EVIDENCE)
            .expect("both citations sprawl, so the task is diluted");
        assert!(
            d.message.contains(&best[..12]) && d.message.contains("60 path(s)"),
            "the count is the best commit's, so the message must name it: {}",
            d.message
        );
    }

    #[test]
    fn an_unresolvable_first_citation_does_not_hide_the_dilution() {
        // Every guard in `check_relevance` returns before the dilution check, so
        // anchoring the report to the literal first commit lets a bogus SHA in
        // slot one suppress the finding about the real citation behind it.
        let (dir, repo) = init();
        let sha = sprawling_commit(&repo, "a", 1, 60);
        let task = done_task(
            "A",
            vec![
                commit_ev("0123456789abcdef0123456789abcdef01234567"),
                commit_ev(&sha),
            ],
            vec!["src/**".into()],
        );
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            codes(&report).contains(&code::DILUTED_EVIDENCE),
            "an unresolved citation is `APLAN011`'s finding, not a reason to go quiet: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn only_a_done_task_is_claiming_its_evidence_proves_anything() {
        // The rule reads "a *done* task's commit evidence". On anything else a
        // sprawling citation is work in progress, not an overclaim.
        let (dir, repo) = init();
        let sha = sprawling_commit(&repo, "a", 1, 60);
        let mut task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);
        task.status = Status::InProgress;
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::DILUTED_EVIDENCE),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_commit_touching_none_of_the_paths_belongs_to_aplan012() {
        // Every path is unrelated, so this would score as maximally diluted. It
        // is already an error, and a warning restating it as a near-miss would
        // muddle which rule the reader has to answer.
        let (dir, repo) = init();
        let sha = sprawling_commit(&repo, "a", 0, 60);
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(codes(&report).contains(&code::COMMIT_IRRELEVANT));
        assert!(
            !codes(&report).contains(&code::DILUTED_EVIDENCE),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_task_this_rule_never_counts_cannot_turn_a_bad_glob_into_an_error() {
        // A per-task check runs on tasks a per-evidence one never reached, so it
        // must not compile a globset it has no citation to match against: doing
        // so turns a malformed glob into a hard error that aborts the whole run
        // and hides every other finding with it.
        let (dir, _repo) = init();
        let mut task = done_task("A", vec![], vec!["src/[".into()]);
        task.evidence = vec![Evidence {
            kind: EvidenceKind::Note,
            reference: "done by hand".into(),
            captured: None,
        }];
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default());
        assert!(
            report.is_ok(),
            "verify aborted on a glob no evidence rule needed: {:?}",
            report.err()
        );
    }

    #[test]
    fn the_threshold_is_configurable_and_zero_turns_it_off() {
        let (dir, repo) = init();
        let sha = sprawling_commit(&repo, "a", 1, 60);
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);

        let strict = VerifyOptions {
            config_file: None,
            max_unrelated_files: Some(10),
            url_check: None,
            ..VerifyOptions::default()
        };
        let report = verify(&plan_of(task.clone()), dir.path(), &strict).unwrap();
        assert!(codes(&report).contains(&code::DILUTED_EVIDENCE));

        let off = VerifyOptions {
            config_file: None,
            max_unrelated_files: Some(0),
            url_check: None,
            ..VerifyOptions::default()
        };
        let report = verify(&plan_of(task), dir.path(), &off).unwrap();
        assert!(
            !codes(&report).contains(&code::DILUTED_EVIDENCE),
            "0 must turn the rule off: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_bogus_sha_fires_011() {
        let (dir, repo) = init();
        commit(&repo, &[("src/a.rs", "x")], "init");
        let task = done_task(
            "A",
            vec![commit_ev("deadbeefdeadbeef")],
            vec!["src/**".into()],
        );
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::COMMIT_UNRESOLVED]);
        assert!(
            report.diagnostics[0].message.contains("deadbeef"),
            "the message must quote the SHA it could not resolve, or a task \
             citing several commits says nothing actionable: {}",
            report.diagnostics[0].message
        );
    }

    /// A closed task describes history. Deleting the code it owned must not turn
    /// a glob that was right into a finding advising you to "correct" it
    /// (`A1-022`, found auditing).
    #[test]
    fn aplan019_judges_a_closed_task_at_the_rev_its_evidence_cites() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("old/gone.rs", "x")], "the work");
        commit(&repo, &[("other/keep.rs", "x")], "later work");
        // Delete what the task owned, the way a real repository eventually does.
        std::fs::remove_file(dir.path().join("old/gone.rs")).expect("rm");
        let mut index = repo.index().expect("index");
        index
            .remove_path(std::path::Path::new("old/gone.rs"))
            .expect("remove");
        index.write().expect("write index");

        let task = done_task("A", vec![commit_ev(&sha)], vec!["old/**".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::PATHS_UNMATCHED),
            "a glob that was live at the cited commit is not dead: {:?}",
            report
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    /// The other half: history is not a blanket excuse. A glob that never matched
    /// anything, at any revision, is still reported on a closed task -- which is
    /// the pilot's case (pre-public adoption evidence, item 13).
    #[test]
    fn aplan019_still_fires_when_a_closed_tasks_glob_never_matched() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("src/a.rs", "x")], "the work");
        let task = done_task("A", vec![commit_ev(&sha)], vec!["libs/core/src".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            codes(&report).contains(&code::PATHS_UNMATCHED),
            "a glob dead at every revision is still dead: {:?}",
            codes(&report)
        );
    }

    /// The historical reading is earned by being closed, not by citing a commit.
    /// A live task's `paths` are a claim about the code as it stands, so a glob
    /// whose files were deleted is dead for it even though the cited commit
    /// contained them. Caught by a surviving mutation: dropping the status guard
    /// A dead glob beside a live one the evidence binds through (`A1-141`).
    ///
    /// The fourth pilot's item 26, filed against its own interest: `close`
    /// refuses what `verify` would fail, so under `deny_warnings` a *bound*
    /// citation could not close a task whose `paths` carried a stale sibling. 43
    /// of its tasks have no satisfied glob at all, where the rule is right; 28
    /// have a satisfied sibling, which is this.
    /// Proves the named behaviour `paths.unmatched_ignores_a_bound_sibling` (`A1-211`).
    #[test]
    fn aplan019_is_silent_when_the_evidence_binds_through_another_glob() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("docs/adr/0001.md", "x")], "the work");

        // The pilot's shape: two globs, one dead, a citation bound through the
        // other.
        let bound_file = done_task(
            "A",
            vec![file_ev("docs/adr/0001.md")],
            vec!["docs/adr/**".into(), "fixtures/**".into()],
        );
        let report = verify(&plan_of(bound_file), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::PATHS_UNMATCHED),
            "a stale sibling is not an unbound claim: {:?}",
            report.diagnostics
        );

        // The same for a commit, which is judged by the same globset.
        let bound_commit = done_task(
            "B",
            vec![commit_ev(&sha)],
            vec!["docs/adr/**".into(), "fixtures/**".into()],
        );
        let report = verify(
            &plan_of(bound_commit),
            dir.path(),
            &VerifyOptions::default(),
        )
        .unwrap();
        assert!(
            !codes(&report).contains(&code::PATHS_UNMATCHED),
            "{:?}",
            report.diagnostics
        );

        // And the case the rule is for is untouched: nothing binds through the
        // set, so the dead glob is named.
        let unbound = done_task(
            "C",
            vec![file_ev("docs/adr/0001.md")],
            vec!["fixtures/**".into()],
        );
        let report = verify(&plan_of(unbound), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            codes(&report).contains(&code::PATHS_UNMATCHED),
            "{:?}",
            report.diagnostics
        );

        // A task citing only a `test` binds nothing *through a glob* -- coverage
        // is what judges a test (`APLAN025`) -- so its dead globs are still worth
        // naming.
        let mut tested = done_task("D", Vec::new(), vec!["fixtures/**".into()]);
        tested.evidence = vec![aplan_core::Evidence {
            kind: aplan_core::EvidenceKind::Test,
            reference: "some::test".into(),
            captured: None,
        }];
        let report = verify(&plan_of(tested), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            codes(&report).contains(&code::PATHS_UNMATCHED),
            "{:?}",
            report.diagnostics
        );
    }

    /// The scope `A1-138` gave the rule, in the case the fourth pilot met 140
    /// times: a plan that names the tree a later cut will create.
    ///
    /// The glob is dead, the task is real, and nothing is bound to it yet, so the
    /// rule's own stated consequence -- "the evidence this task cites binds to
    /// nothing through it" -- cannot occur. Under `deny_warnings` these were a
    /// wall, and p00020 declined the upgrade over them rather than delete the
    /// globs, which would have surrendered the falsifiability `APLAN012` rests on
    /// (pre-public adoption evidence, item 24 and its correction).
    /// Proves the named behaviour `paths.unmatched_needs_evidence` (`A1-211`).
    #[test]
    fn aplan019_is_silent_where_nothing_is_bound_to_the_paths() {
        let (dir, repo) = init();
        commit(&repo, &[("src/a.rs", "x")], "init");

        // A task planning past the current tree, which is what a plan is.
        let mut forward = done_task("A", Vec::new(), vec!["crates/not-yet/**".into()]);
        forward.status = aplan_core::Status::Todo;
        let report = verify(&plan_of(forward), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::PATHS_UNMATCHED),
            "a forward-looking path with nothing bound to it: {:?}",
            codes(&report)
        );

        // And status is not the scope: a closed task whose only citation is a
        // note has nothing judged against its paths either. `APLAN017` is the
        // rule with an opinion about that citation's strength; this one has none.
        let mut noted = done_task("B", Vec::new(), vec!["crates/not-yet/**".into()]);
        noted.evidence = vec![aplan_core::Evidence {
            kind: aplan_core::EvidenceKind::Note,
            reference: "reviewed by hand".into(),
            captured: Some("2026-08-15".into()),
        }];
        let report = verify(&plan_of(noted), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            !codes(&report).contains(&code::PATHS_UNMATCHED),
            "a note is judged against nothing: {:?}",
            codes(&report)
        );

        // What the rule is for, unchanged: a citation that *is* judged against
        // the paths, and paths that reach nothing.
        let bound = done_task(
            "C",
            vec![file_ev("src/a.rs")],
            vec!["crates/not-yet/**".into()],
        );
        let report = verify(&plan_of(bound), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            codes(&report).contains(&code::PATHS_UNMATCHED),
            "the case the rule exists for: {:?}",
            codes(&report)
        );
        // And it names what is unbound rather than a clause about every rule
        // that might judge evidence (`A1-091`).
        let said = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::PATHS_UNMATCHED)
            .map(|d| d.message.clone())
            .expect("the finding");
        assert!(said.contains("the file this task cites"), "{said}");
    }

    /// passed every other test (`A1-022`).
    #[test]
    fn aplan019_does_not_excuse_a_live_task_with_history() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("old/gone.rs", "x")], "the work");
        commit(&repo, &[("other/keep.rs", "x")], "later work");
        std::fs::remove_file(dir.path().join("old/gone.rs")).expect("rm");
        let mut index = repo.index().expect("index");
        index
            .remove_path(std::path::Path::new("old/gone.rs"))
            .expect("remove");
        index.write().expect("write index");

        let mut task = done_task("A", vec![commit_ev(&sha)], vec!["old/**".into()]);
        task.status = aplan_core::Status::Todo;
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(
            codes(&report).contains(&code::PATHS_UNMATCHED),
            "a live task is judged at HEAD: {:?}",
            codes(&report)
        );
    }

    /// A closed task citing evidence that is not a resolvable commit has no
    /// history to be judged against, and must still be judged. Two shapes in one
    /// test: a `file` citation, which `historic_paths` skips by kind, and a SHA
    /// that resolves to nothing.
    #[test]
    fn aplan019_falls_back_to_head_when_a_closed_task_has_no_usable_history() {
        let (dir, repo) = init();
        commit(&repo, &[("src/a.rs", "x")], "the work");
        for evidence in [
            vec![file_ev("src/a.rs")],
            vec![commit_ev("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef")],
        ] {
            let task = done_task("A", evidence, vec!["libs/core/src".into()]);
            let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
            assert!(
                codes(&report).contains(&code::PATHS_UNMATCHED),
                "with no readable history the glob is judged at HEAD: {:?}",
                codes(&report)
            );
        }
    }

    #[test]
    fn a_real_but_irrelevant_commit_fires_012() {
        let (dir, repo) = init();
        // `src/` is tracked first, so this fires because the cited commit touches
        // nothing the task owns, not because the task owns nothing. Without that,
        // `APLAN019` fires too and the assertion below could not tell the two
        // readings apart, which is the confusion `A1-022` exists to end.
        commit(&repo, &[("src/a.rs", "x")], "src exists");
        let sha = commit(&repo, &[("docs/notes.md", "prose")], "docs only");
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::COMMIT_IRRELEVANT]);
        let m = &report.diagnostics[0].message;
        assert!(
            m.contains(&sha[..12]) && m.contains("src/**"),
            "the message must name the commit judged *and* the globs it missed: {m}"
        );
    }

    #[test]
    fn an_irrelevant_commit_without_paths_is_silent_on_012() {
        // No `paths` means relevance is unverifiable, not a violation of 012.
        let (dir, repo) = init();
        let sha = commit(&repo, &[("docs/notes.md", "prose")], "docs only");
        let task = done_task("A", vec![commit_ev(&sha)], vec![]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(report.is_clean(), "{:?}", report.diagnostics);
    }

    /// A `commit` citation must be an immutable SHA. A branch/tag/`HEAD`/`:/msg`
    /// resolves but is mutable and fork-controlled, so it must not pass L1.
    #[test]
    fn a_non_sha_commit_ref_fires_011() {
        let (dir, repo) = init();
        commit(&repo, &[("src/a.rs", "x")], "init");
        for reference in ["HEAD", "main", "v1.0", ":/init"] {
            let task = done_task("A", vec![commit_ev(reference)], vec!["src/**".into()]);
            let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
            assert_eq!(
                codes(&report),
                vec![code::COMMIT_UNRESOLVED],
                "a mutable ref ({reference}) must not resolve as commit evidence"
            );
        }
    }

    /// A branch or tag whose *name* is hex-shaped (e.g. `deadbeefcafe`) resolves
    /// via `revparse`, but is mutable and fork-controllable, so it must not pass
    /// as an immutable commit citation. A real SHA is a prefix of the resolved id.
    #[test]
    fn a_hex_named_ref_is_not_accepted_as_a_commit_sha() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("src/a.rs", "x")], "init");
        let oid = git2::Oid::from_str(&sha).unwrap();
        let obj = repo.find_commit(oid).unwrap();
        repo.branch("deadbeefcafe", &obj, false).unwrap();
        let task = done_task("A", vec![commit_ev("deadbeefcafe")], vec!["src/**".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(
            codes(&report),
            vec![code::COMMIT_UNRESOLVED],
            "a hex-named branch must not pass as an immutable SHA"
        );
    }

    /// A catch-all `paths` (`**`) matches every file, so it cannot bind a commit
    /// to specific work: it must FAIL relevance, not trivially pass it (the task
    /// file is untrusted).
    #[test]
    fn a_match_everything_paths_fires_012() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("docs/notes.md", "prose")], "docs only");
        for wild in ["**", "*", "**/*"] {
            let task = done_task("A", vec![commit_ev(&sha)], vec![wild.into()]);
            let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
            // `APLAN019` joins for `*` and `**/*` and is right to: with every
            // tracked file nested under `docs/`, a glob that cannot cross `/`
            // matches nothing at all. The assertion keeps `APLAN012` first and
            // named, because that is what this test is about.
            assert!(
                codes(&report).contains(&code::COMMIT_IRRELEVANT),
                "a catch-all glob ({wild}) must not make any commit relevant"
            );
        }
    }

    /// A single `*` must not cross `/`: `src/*` owns `src/a.rs` but not
    /// `src/sub/deep.rs` (gitignore/ripgrep semantics, `literal_separator`).
    #[test]
    fn a_star_does_not_cross_a_directory_separator() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("src/sub/deep.rs", "x")], "nested only");
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/*".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        // Both fire, and the pair is the point: `APLAN012` says the commit missed
        // the globs, `APLAN019` says the glob reaches nothing in this repository
        // at all. Before `A1-022` only the first was visible, and it read as a
        // defect in the evidence when the glob was the problem.
        assert_eq!(
            codes(&report),
            vec![code::COMMIT_IRRELEVANT, code::PATHS_UNMATCHED],
            "`src/*` must not match `src/sub/deep.rs`"
        );

        // But it does own a direct child.
        let sha2 = commit(&repo, &[("src/top.rs", "y")], "direct child");
        let ok = done_task("B", vec![commit_ev(&sha2)], vec!["src/*".into()]);
        let report = verify(&plan_of(ok), dir.path(), &VerifyOptions::default()).unwrap();
        assert!(report.is_clean(), "{:?}", report.diagnostics);
    }

    /// `file` evidence must resolve to a blob, not a directory: `file: src`
    /// (a directory) is not a committed artefact and must fire APLAN015.
    #[test]
    fn file_evidence_pointing_at_a_directory_fires_015() {
        let (dir, repo) = init();
        commit(&repo, &[("src/a.rs", "x")], "init");
        let task = done_task("A", vec![file_ev("src")], vec![]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(
            codes(&report),
            vec![code::FILE_UNRESOLVED],
            "a directory is not a committed file"
        );
    }

    #[test]
    fn a_present_file_is_clean_and_a_missing_one_fires_015() {
        let (dir, repo) = init();
        commit(&repo, &[("src/a.rs", "x")], "init");

        let ok = done_task("A", vec![file_ev("src/a.rs")], vec![]);
        assert!(
            verify(&plan_of(ok), dir.path(), &VerifyOptions::default())
                .unwrap()
                .is_clean()
        );

        let bad = done_task("B", vec![file_ev("src/nope.rs")], vec![]);
        let report = verify(&plan_of(bad), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_UNRESOLVED]);
        assert!(
            report.diagnostics[0].message.contains("src/nope.rs"),
            "the message must name the path that did not resolve: {}",
            report.diagnostics[0].message
        );
    }

    const PROOF: &str = "# Containment\n\n## Final containment result\n\nIt held.\n\n\
                         | Item | State |\n|---|---|\n| Writer interlock | closed |\n";

    /// The citation a repository whose proof is a reviewed document actually has
    /// (`A1-159`, from `A1-036`). Fifty-four of `p0000019`'s fifty-nine document
    /// references name a section, all of them in a field that project calls
    /// `evidence`, and every one of them resolves through this.
    /// Proves the named behaviour `evidence.file_names_a_section` (`A1-261`).
    #[test]
    fn a_file_citation_may_name_the_section_that_proves_it() {
        let (dir, repo) = init();
        commit(&repo, &[("docs/proof.md", PROOF)], "the proof");

        let ok = done_task(
            "A",
            vec![file_ev("docs/proof.md#final-containment-result")],
            vec!["docs/**".into()],
        );
        assert!(
            verify(&plan_of(ok), dir.path(), &VerifyOptions::default())
                .unwrap()
                .is_clean(),
            "an anchor that resolves is proof, not a fault"
        );

        // ADR-0015: a table row is an anchor too, and it means here what it
        // means in `traces`, because it is the same reader.
        let row = done_task(
            "B",
            vec![file_ev("docs/proof.md#writer-interlock")],
            vec!["docs/**".into()],
        );
        assert!(
            verify(&plan_of(row), dir.path(), &VerifyOptions::default())
                .unwrap()
                .is_clean()
        );
    }

    /// "The document is not there" and "that section is not in it" send a reader
    /// to different places, so the message says which -- under one code, as
    /// `APLAN008` already does for a `traces` target and its anchor.
    #[test]
    fn an_anchor_that_resolves_to_nothing_says_the_file_was_found() {
        let (dir, repo) = init();
        commit(&repo, &[("docs/proof.md", PROOF)], "the proof");

        let task = done_task(
            "A",
            vec![file_ev("docs/proof.md#no-such-section")],
            vec!["docs/**".into()],
        );
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_UNRESOLVED]);
        let message = &report.diagnostics[0].message;
        assert!(message.contains("exists at HEAD"), "{message}");
        assert!(message.contains("#no-such-section"), "{message}");

        // And the empty anchor, which is a typo rather than a missing section.
        let empty = done_task("B", vec![file_ev("docs/proof.md#")], vec!["docs/**".into()]);
        let report = verify(&plan_of(empty), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_UNRESOLVED]);
        assert!(
            report.diagnostics[0].message.contains("empty anchor"),
            "{:?}",
            report.diagnostics[0].message
        );
    }

    /// The section is read out of the repository at the cited rev, never off
    /// disk. An uncommitted edit to the proof must not make a citation resolve
    /// for its author and fail for everybody else.
    /// Proves the named behaviour `evidence.file_names_a_section` (`A1-261`).
    #[test]
    fn the_section_is_read_at_the_rev_and_not_from_the_working_tree() {
        let (dir, repo) = init();
        let first = commit(
            &repo,
            &[("docs/proof.md", "# Containment\n")],
            "no section yet",
        );
        commit(&repo, &[("docs/proof.md", PROOF)], "add the section");

        // At HEAD the section is there.
        let at_head = done_task(
            "A",
            vec![file_ev("docs/proof.md#final-containment-result")],
            vec!["docs/**".into()],
        );
        assert!(
            verify(&plan_of(at_head), dir.path(), &VerifyOptions::default())
                .unwrap()
                .is_clean()
        );

        // At the first rev it is not, whatever the file says now.
        let at_rev = || {
            done_task(
                "B",
                vec![file_ev(&format!(
                    "docs/proof.md#final-containment-result@{first}"
                ))],
                vec!["docs/**".into()],
            )
        };
        let report = verify(&plan_of(at_rev()), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_UNRESOLVED]);

        // Writing the section into the working tree changes nothing: the answer
        // is a function of the commit.
        std::fs::write(dir.path().join("docs/proof.md"), PROOF).unwrap();
        let report = verify(&plan_of(at_rev()), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(
            codes(&report),
            vec![code::FILE_UNRESOLVED],
            "an uncommitted edit must not settle a citation"
        );
    }

    /// A file that is tracked and is not text has no sections to name, and a
    /// citation must not pass on the strength of an anchor nothing read.
    #[test]
    fn an_anchor_on_a_file_that_is_not_text_is_refused() {
        let (dir, repo) = init();
        let workdir = repo.workdir().unwrap().to_path_buf();
        std::fs::create_dir_all(workdir.join("docs")).unwrap();
        std::fs::write(workdir.join("docs/scan.bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        commit(&repo, &[("docs/keep.md", "# Keep\n")], "a binary proof");

        let task = done_task(
            "A",
            vec![file_ev("docs/scan.bin#result")],
            vec!["docs/**".into()],
        );
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_UNRESOLVED]);
        assert!(
            report.diagnostics[0].message.contains("is not text"),
            "{:?}",
            report.diagnostics[0].message
        );

        // Without the anchor the same citation is fine: a binary artefact is a
        // perfectly good `file` citation, it just has no sections.
        let whole = done_task("B", vec![file_ev("docs/scan.bin")], vec!["docs/**".into()]);
        assert!(
            verify(&plan_of(whole), dir.path(), &VerifyOptions::default())
                .unwrap()
                .is_clean()
        );
    }

    /// `APLAN031` judges the path, not the reference: an anchor is a place in
    /// the file, so relevance must strip it before matching `paths`.
    /// Proves the named behaviour `evidence.file_names_a_section` (`A1-261`).
    #[test]
    fn relevance_is_judged_on_the_path_and_not_on_the_anchor() {
        let (dir, repo) = init();
        // `src/` exists so the out-of-paths case is `APLAN031` alone, rather
        // than `APLAN019` reporting a glob that matches nothing.
        commit(
            &repo,
            &[("docs/proof.md", PROOF), ("src/a.rs", "x")],
            "the proof",
        );

        let inside = done_task(
            "A",
            vec![file_ev("docs/proof.md#final-containment-result")],
            vec!["docs/**".into()],
        );
        assert!(
            verify(&plan_of(inside), dir.path(), &VerifyOptions::default())
                .unwrap()
                .is_clean()
        );

        let outside = done_task(
            "B",
            vec![file_ev("docs/proof.md#final-containment-result")],
            vec!["src/**".into()],
        );
        let report = verify(&plan_of(outside), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_IRRELEVANT]);
        assert!(
            report.diagnostics[0].message.contains("`docs/proof.md`"),
            "the path, without the anchor: {:?}",
            report.diagnostics[0].message
        );
    }

    /// `path@rev#section` is the two halves written the other way round, and the
    /// first version answered it with "file does not exist at
    /// `<sha>#final-result`" -- sending the reader after a missing document
    /// rather than at the order they typed.
    #[test]
    fn a_rev_holding_an_anchor_is_told_which_order_to_write() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("docs/proof.md", PROOF)], "the proof");

        let task = done_task(
            "A",
            vec![file_ev(&format!(
                "docs/proof.md@{sha}#final-containment-result"
            ))],
            vec!["docs/**".into()],
        );
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::FILE_UNRESOLVED]);
        let message = &report.diagnostics[0].message;
        assert!(message.contains("path#section@rev"), "{message}");
        assert!(
            !message.contains("does not exist"),
            "the document is there; the order is wrong: {message}"
        );
    }

    #[test]
    fn file_evidence_resolves_at_a_cited_rev() {
        let (dir, repo) = init();
        let first = commit(&repo, &[("src/a.rs", "x")], "add a");
        // Second commit removes the file, so it is gone at HEAD but present at `first`.
        std::fs::remove_file(dir.path().join("src/a.rs")).unwrap();
        commit(&repo, &[("src/b.rs", "y")], "remove a, add b");

        // At HEAD: gone -> fires.
        let at_head = done_task("A", vec![file_ev("src/a.rs")], vec![]);
        assert_eq!(
            codes(&verify(&plan_of(at_head), dir.path(), &VerifyOptions::default()).unwrap()),
            vec![code::FILE_UNRESOLVED]
        );

        // At the first rev: present -> clean.
        let at_rev = done_task("B", vec![file_ev(&format!("src/a.rs@{first}"))], vec![]);
        assert!(
            verify(&plan_of(at_rev), dir.path(), &VerifyOptions::default())
                .unwrap()
                .is_clean()
        );
    }

    #[test]
    fn a_root_commit_diffs_against_the_empty_tree() {
        // The very first commit has no parent; its diff must still yield paths,
        // so relevance works on a fresh repo's initial commit.
        let (dir, repo) = init();
        let sha = commit(&repo, &[("src/a.rs", "x")], "root");
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);
        assert!(
            verify(&plan_of(task), dir.path(), &VerifyOptions::default())
                .unwrap()
                .is_clean()
        );
    }

    #[test]
    fn a_non_repository_is_an_error_not_a_finding() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        let task = done_task("A", vec![commit_ev("abc123")], vec![]);
        let err = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap_err();
        assert!(matches!(err, Error::NotARepository(_)), "{err:?}");
    }

    /// `A1-054`: a repository whose tests are not reachable from one command.
    fn named_runner(name: &str, dir: Option<&str>, list: &[&str], run: &[&str]) -> TestRunner {
        TestRunner {
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers: None,
            covers_by_test: None,
            name: Some(name.to_string()),
            dir: dir.map(ToString::to_string),
            list: list.iter().map(|s| (*s).to_string()).collect(),
            run: run.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn a_second_runner_can_resolve_what_the_first_cannot_see() {
        // The umbrella case: no single command lists both corpora, so before this
        // the second component's tasks could not cite `test` evidence at all and
        // fell back to weaker kinds, quietly eroding `require_min_strength`.
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: true,
            test_runner: Some(TestRunners::Many(vec![
                named_runner("libs", None, &["printf", "alpha\n"], &["true", PLACEHOLDER]),
                named_runner("web", None, &["printf", "beta\n"], &["true", PLACEHOLDER]),
            ])),
            max_unrelated_files: None,
            url_check: None,
        };
        let r = verify(
            &plan_of(done_task("A", vec![test_ev("beta")], vec![])),
            dir.path(),
            &opts,
        )
        .unwrap();
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    /// A plan of several tasks, for the cases that are about reuse.
    fn plan_of_all(tasks: Vec<Task>) -> Plan {
        Plan {
            tasks: tasks
                .into_iter()
                .map(|task| LoadedTask {
                    path: PathBuf::from(format!("plan/tasks/{}.md", task.id)),
                    task,
                    fields: aplan_core::load::FieldLines::new(),
                    criteria: aplan_core::load::Criteria::default(),
                })
                .collect(),
            load_diagnostics: Vec::new(),
            unloadable: std::collections::BTreeMap::new(),
        }
    }

    /// A runner whose `run` records the name it was asked for, so a test can
    /// count what really ran.
    fn counting_runner(dir: &Path, passes: bool) -> TestRunner {
        let log = dir.join("ran.log").display().to_string();
        let exit = if passes { "exit 0" } else { "exit 1" };
        named_runner(
            "counting",
            None,
            &["printf", "alpha\nbeta\n"],
            &[
                "sh",
                "-c",
                &format!("printf '%s\\n' \"$0\" >> '{log}'; {exit}"),
                PLACEHOLDER,
            ],
        )
    }

    fn names_run(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("ran.log"))
            .unwrap_or_default()
            .lines()
            .map(ToString::to_string)
            .collect()
    }

    /// One runner and one exact name select one execution, however many tasks
    /// cite it (`A1-243`).
    ///
    /// Measured on `p003` before this: 86 citations over nine distinct names
    /// took about 55 minutes against about three for the nine suites run once,
    /// because the cost grew with citation reuse rather than with the
    /// executable set. Reverting the memoization makes this count 4 rather
    /// than 3, which is the assertion that pins it.
    #[test]
    fn each_distinct_cited_test_runs_once_however_many_tasks_cite_it() {
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: true,
            test_runner: Some(TestRunners::One(counting_runner(dir.path(), true))),
            max_unrelated_files: None,
            url_check: None,
        };
        let plan = plan_of_all(vec![
            done_task("A", vec![test_ev("alpha")], vec![]),
            done_task("B", vec![test_ev("alpha")], vec![]),
            done_task("C", vec![test_ev("beta")], vec![]),
        ]);
        let r = verify(&plan, dir.path(), &opts).unwrap();
        assert!(r.is_clean(), "{:?}", r.diagnostics);

        let mut ran = names_run(dir.path());
        ran.sort();
        let mut want = vec![
            "alpha".to_string(),
            "beta".to_string(),
            crate::resolve::MISS_SENTINEL.to_string(),
        ];
        want.sort();
        assert_eq!(
            ran, want,
            "two names and the sentinel, each exactly once -- three spawns for three citations"
        );
    }

    /// A shared failure is every citing task's failure, and still one run.
    #[test]
    fn a_failing_test_fails_every_citation_without_running_twice() {
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: true,
            test_runner: Some(TestRunners::One(counting_runner(dir.path(), false))),
            max_unrelated_files: None,
            url_check: None,
        };
        let plan = plan_of_all(vec![
            done_task("A", vec![test_ev("alpha")], vec![]),
            done_task("B", vec![test_ev("alpha")], vec![]),
        ]);
        let r = verify(&plan, dir.path(), &opts).unwrap();
        let failed: Vec<_> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::TEST_FAILED)
            .collect();
        assert_eq!(failed.len(), 2, "both citations are told: {failed:?}");
        assert_eq!(
            names_run(dir.path())
                .iter()
                .filter(|n| *n == "alpha")
                .count(),
            1,
            "and it was run once"
        );
    }

    /// Reusing one execution must not quietly shrink what `APLAN037` reports.
    ///
    /// The finding counts *citations*, and says so in its message. `A1-243`
    /// runs one command for a name however many tasks cite it, so counting
    /// executions instead would report "(1 citation)" about a dozen of them --
    /// the number an adopter uses to judge how much of their plan is unproven.
    #[test]
    fn a_reused_execution_still_counts_every_citation_it_proved_nothing_about() {
        let (dir, _repo) = init();
        let config = dir.path().join(".aplan.yml");
        // Exits 0 and says nothing, whatever name it is handed: a pass and a
        // miss are the same bytes.
        let opts = VerifyOptions {
            config_file: Some(config),
            execute: true,
            test_runner: Some(TestRunners::One(runner(
                &["printf", "alpha\n"],
                &["true", PLACEHOLDER],
            ))),
            ..VerifyOptions::default()
        };
        let plan = plan_of_all(vec![
            done_task("A", vec![test_ev("alpha")], vec![]),
            done_task("B", vec![test_ev("alpha")], vec![]),
            done_task("C", vec![test_ev("alpha")], vec![]),
        ]);
        let r = verify(&plan, dir.path(), &opts).unwrap();
        let found: Vec<_> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::RUN_PROVES_NOTHING)
            .collect();
        assert_eq!(found.len(), 1, "once per runner: {:?}", r.diagnostics);
        assert!(
            found[0].message.contains("(3 citations)"),
            "three citations were unproven, however many commands ran: {}",
            found[0].message
        );
    }

    /// Nothing is carried between runs: the memo lives on the resolver, which
    /// lives for one `verify`.
    #[test]
    fn a_second_verify_runs_the_tests_again() {
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: true,
            test_runner: Some(TestRunners::One(counting_runner(dir.path(), true))),
            max_unrelated_files: None,
            url_check: None,
        };
        let plan = plan_of_all(vec![done_task("A", vec![test_ev("alpha")], vec![])]);
        verify(&plan, dir.path(), &opts).unwrap();
        let first = names_run(dir.path()).len();
        verify(&plan, dir.path(), &opts).unwrap();
        assert_eq!(
            names_run(dir.path()).len(),
            first * 2,
            "a separate invocation shares nothing"
        );
    }

    fn url_opts(reachable: bool) -> VerifyOptions {
        VerifyOptions {
            config_file: None,
            url_check: Some(aplan_config::UrlCheck {
                max_memory: None,
                max_processes: None,
                wrap: None,
                timeout_seconds: None,
                check: vec![
                    if reachable { "true" } else { "false" }.to_string(),
                    aplan_config::URL_PLACEHOLDER.to_string(),
                ],
            }),
            ..VerifyOptions::default()
        }
    }

    fn url_task(refs: &[&str]) -> Plan {
        let evidence = refs
            .iter()
            .map(|r| Evidence {
                kind: EvidenceKind::Url,
                reference: (*r).to_string(),
                captured: None,
            })
            .collect();
        plan_of(done_task("A", evidence, vec![]))
    }

    #[test]
    fn aplan016_names_the_url_that_did_not_answer() {
        // The message is the product. It must quote the citation, and it must say
        // *did not answer*: a link down for an hour is not a plan that is wrong,
        // and calling it wrong trains an adopter to ignore the rule (`A1-041`).
        let (dir, _repo) = init();
        let report = verify(
            &url_task(&["https://example.invalid/receipt"]),
            dir.path(),
            &url_opts(false),
        )
        .unwrap();
        assert_eq!(codes(&report), vec![code::URL_UNREACHABLE]);
        let m = &report.diagnostics[0].message;
        assert!(
            m.contains("https://example.invalid/receipt"),
            "must quote the citation: {m}"
        );
        assert!(
            m.contains("did not answer"),
            "must report unreachability, not wrongness: {m}"
        );
    }

    #[test]
    fn aplan016_is_silent_when_the_checker_answers_and_absent_without_one() {
        let (dir, _repo) = init();
        let cited = url_task(&["https://example.invalid/receipt"]);
        // Not `is_clean()`, which means *no errors*: this rule is a warning, so
        // that assertion passed whether or not it fired. Caught by a mutation
        // that forced the checker on and left this test green (`A1-041`).
        assert!(
            verify(&cited, dir.path(), &url_opts(true))
                .unwrap()
                .diagnostics
                .is_empty(),
            "a reachable url is not a finding"
        );
        // The opt-in half, which is the contract: no key, no network, no verdict.
        assert!(
            verify(&cited, dir.path(), &VerifyOptions::default())
                .unwrap()
                .diagnostics
                .is_empty(),
            "without a configured checker the rule must not run at all"
        );
    }

    #[test]
    fn aplan016_hands_the_checker_https_only() {
        // The security finding from auditing `A1-041`. The citation comes from a
        // task file and a task file can arrive in a fork pull request, so the
        // adopter's command cannot defend itself: `curl -sfI {url}` fetches
        // whatever the task names. Measured before the restriction existed: a
        // task file made the checker request `file:///etc/passwd` and the cloud
        // metadata address.
        let (dir, _repo) = init();
        let log = dir.path().join("handed.log");
        let opts = VerifyOptions {
            config_file: None,
            url_check: Some(aplan_config::UrlCheck {
                max_memory: None,
                max_processes: None,
                wrap: None,
                timeout_seconds: None,
                check: vec![
                    "bash".into(),
                    "-c".into(),
                    format!("echo \"$0\" >> {}", log.display()),
                    aplan_config::URL_PLACEHOLDER.to_string(),
                ],
            }),
            ..VerifyOptions::default()
        };
        let hostile = url_task(&[
            "file:///etc/passwd",
            "http://169.254.169.254/latest/meta-data/",
        ]);
        let report = verify(&hostile, dir.path(), &opts).unwrap();

        assert!(
            !log.exists(),
            "the checker must not be handed a non-https citation at all"
        );
        // A scheme is case-insensitive (RFC 3986). Rejecting `HTTPS://` would be
        // a false negative on a citation that is perfectly good, and it is the
        // safe direction, so nothing else would have caught it.
        let upper = url_task(&["HTTPS://EXAMPLE.invalid/upper"]);
        assert!(
            verify(&upper, dir.path(), &opts)
                .unwrap()
                .diagnostics
                .is_empty(),
            "an uppercase scheme is still https"
        );
        // Reported, not skipped: a citation nothing checked must not read like
        // one that answered.
        assert_eq!(report.diagnostics.len(), 2, "both are reported");
        for d in &report.diagnostics {
            assert_eq!(d.code, code::URL_UNREACHABLE);
            assert!(
                d.message.contains("was not checked"),
                "nothing was asked, so it did not `fail to answer`: {}",
                d.message
            );
        }
    }

    #[test]
    fn aplan016_asks_once_per_url_however_many_tasks_cite_it() {
        // The same argument `A1-054` made for test runners: a plan citing one
        // regulator page from ten tasks must make one request, not ten.
        let (dir, _repo) = init();
        let counter = dir.path().join("asked.log");
        let opts = VerifyOptions {
            config_file: None,
            url_check: Some(aplan_config::UrlCheck {
                max_memory: None,
                max_processes: None,
                wrap: None,
                timeout_seconds: None,
                check: vec![
                    "bash".into(),
                    "-c".into(),
                    format!("echo x >> {}; exit 1", counter.display()),
                    aplan_config::URL_PLACEHOLDER.to_string(),
                ],
            }),
            ..VerifyOptions::default()
        };
        let same = "https://example.invalid/one";
        let report = verify(&url_task(&[same, same, same]), dir.path(), &opts).unwrap();
        assert_eq!(report.diagnostics.len(), 3, "each citation is reported");
        let asked = std::fs::read_to_string(&counter).unwrap().lines().count();
        assert_eq!(asked, 1, "the checker was asked {asked} times for one url");
    }

    /// The measurement and the check answer the same question, and this asks
    /// both.
    ///
    /// `commit_breadth` kept its own copy of "what did this commit touch",
    /// taking one side of each delta where `commit_paths` -- what `APLAN012`
    /// resolves a citation with -- takes both. That difference is invisible
    /// today, because a tree-to-tree diff with no rename detection reports a
    /// move as an add and a delete with the path on both sides, so this test
    /// passes against either version and is **not** a guard on the dedup: the
    /// guard is that there is one function now. What it does pin is the
    /// behaviour a reader would doubt -- that a commit which moves a file *out*
    /// of a task's `paths` still counts as touching them, exactly as the
    /// citation check counts it (`A1-151`'s audit).
    #[test]
    fn breadth_counts_a_commit_exactly_when_the_citation_check_would() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("init");
        commit(&repo, &[("src/a.rs", "a"), ("keep.txt", "k")], "base");
        // A rename out of `src/`, a deletion, and a plain edit: the three shapes
        // where "one side" and "both sides" could differ.
        std::fs::remove_file(dir.path().join("src/a.rs")).expect("rm");
        commit(&repo, &[("lib/a.rs", "a")], "rename out of src");
        commit(&repo, &[("keep.txt", "k2")], "unrelated edit");

        let r = Repo::discover(dir.path()).expect("discover");
        let paths = vec!["src/**".to_string()];
        let breadth = r
            .commit_breadth(&[("T".to_string(), paths.clone())], 100)
            .expect("breadth");

        // The same answer, computed the way a citation is checked.
        let set = crate::resolve::globset_for(&paths).expect("a glob");
        let mut walk = repo.revwalk().expect("revwalk");
        walk.push_head().expect("push");
        let mut expected = 0;
        for oid in walk {
            let c = repo.find_commit(oid.expect("oid")).expect("commit");
            if r.commit_paths(&c)
                .expect("paths")
                .iter()
                .any(|p| set.is_match(p))
            {
                expected += 1;
            }
        }
        assert_eq!(breadth.sampled, 3, "the fixture's history");
        assert_eq!(
            breadth.counts,
            vec![("T".to_string(), expected)],
            "breadth and the citation check must count the same commits"
        );
        assert_eq!(expected, 2, "base adds src/a.rs, the rename removes it");
    }

    /// A `paths` entry that will not compile is unmeasurable, not zero. Zero is
    /// the narrowest row the report can print, so a broken glob would read as
    /// the best-scoped task in the plan.
    #[test]
    fn a_path_that_does_not_compile_is_not_counted_as_zero() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("init");
        commit(&repo, &[("src/a.rs", "a")], "base");

        let r = Repo::discover(dir.path()).expect("discover");
        let breadth = r
            .commit_breadth(
                &[
                    ("GOOD".to_string(), vec!["src/**".to_string()]),
                    ("BAD".to_string(), vec!["src/[a".to_string()]),
                ],
                100,
            )
            .expect("breadth");
        assert_eq!(breadth.counts, vec![("GOOD".to_string(), 1)]);
        assert_eq!(breadth.unmeasurable, vec!["BAD".to_string()]);
    }

    /// No commits is not a row of zeroes: there is nothing to count against, and
    /// `sampled` is how the caller tells the two apart.
    #[test]
    fn a_repository_with_no_commits_samples_nothing() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        git2::Repository::init(dir.path()).expect("init");
        let r = Repo::discover(dir.path()).expect("discover");
        let breadth = r
            .commit_breadth(&[("T".to_string(), vec!["src/**".to_string()])], 100)
            .expect("breadth");
        assert_eq!(breadth.sampled, 0);
        assert!(
            breadth.counts.is_empty(),
            "no commit was read, so there is no count to report: {:?}",
            breadth.counts
        );
    }

    /// `head_sha` is what `close` derives evidence from when none is given, and
    /// what `A1-109`'s gate then resolves. It had no test in this crate.
    #[test]
    fn head_sha_reports_the_current_commit_and_says_when_there_is_none() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("init");
        let err = head_sha(dir.path()).expect_err("a repository with no commits has no HEAD");
        assert!(
            matches!(err, Error::NoCommits(_)),
            "an empty repository is `NoCommits`, not a generic failure: {err}"
        );

        let sha = commit(&repo, &[("a.rs", "x")], "first");
        assert_eq!(
            head_sha(dir.path()).expect("HEAD resolves"),
            sha,
            "the full SHA of HEAD, which is what `close` records"
        );

        let missing = dir.path().join("not-a-repo");
        std::fs::create_dir(&missing).expect("mkdir");
        assert!(
            matches!(head_sha(&missing), Err(Error::NotARepository(_)) | Ok(_)),
            "a directory inside the repo still discovers it; outside it does not"
        );
    }

    #[test]
    fn a_runner_that_could_not_answer_is_not_a_runner_that_listed_nothing() {
        // Found auditing `A1-120`. The exit status was ignored, so a package that
        // failed to compile listed nothing and every citation against it was
        // reported as "not one the configured runner lists" -- true of the empty
        // list, and the wrong reason, sending the reader to fix the citation.
        // Eight runners make it likely rather than theoretical.
        //
        // `A1-120` made it stop the run. `A1-066` keeps the loudness and drops
        // the stopping: it is an *error finding* with no task attached, because
        // no task is at fault, and the run still reports everything else.
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            test_runner: Some(TestRunners::One(runner(
                &[
                    "bash",
                    "-c",
                    "echo 'error: could not compile' >&2; exit 101",
                ],
                &["true", PLACEHOLDER],
            ))),
            ..VerifyOptions::default()
        };
        let plan = plan_of(done_task("A", vec![test_ev("a_real_test")], vec![]));
        let report = verify(&plan, dir.path(), &opts).expect("a broken runner is a finding");
        let about_runner = report
            .diagnostics
            .iter()
            .find(|d| d.task.is_none())
            .expect("a finding about the configuration, attached to no task");
        let text = &about_runner.message;
        assert_eq!(about_runner.severity, aplan_core::Severity::Error);
        assert!(text.contains("test.list"), "it must name the key: {text}");
        assert!(text.contains("101"), "and the exit status: {text}");
        // The runner's own words, which are what actually diagnose it.
        assert!(text.contains("could not compile"), "{text}");

        // And the citation that depended on it is not reported as a name nobody
        // lists: that is not a fact this run established, and reporting it would
        // send the reader to fix a citation that may be perfectly correct. The
        // wrong reason was the whole of `A1-120`'s finding; the fix keeps it.
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.task.as_deref() == Some("A")),
            "the citation was blamed for its runner: {:?}",
            report.diagnostics
        );
    }

    /// What an unusable glob costs, exactly (`A1-066`).
    ///
    /// The rules that *bind* evidence through `paths` go quiet, because there is
    /// no set to match against: silence, not a catch-all, which would make
    /// `APLAN012` pass on any citation at all. The entries that do compile are
    /// still judged, which is why `APLAN032`'s message claims the first and not
    /// the second.
    #[test]
    fn an_unusable_glob_silences_binding_and_nothing_else() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("f.txt", "x")], "work");
        let plan = plan_of(done_task(
            "A",
            vec![Evidence {
                kind: EvidenceKind::Commit,
                reference: sha.clone(),
                captured: None,
            }],
            vec!["src/[".to_string(), "gone/**".to_string()],
        ));
        let report = verify(&plan, dir.path(), &VerifyOptions::default()).expect("a report");

        // The cited commit touches nothing either glob could own, and `APLAN012`
        // says nothing: it has no usable set to judge with. A catch-all in place
        // of the broken glob would be silent here too *and* would silence the
        // line below, which is what tells the two apart.
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.code == aplan_core::code::COMMIT_IRRELEVANT),
            "a task whose `paths` will not compile was judged against them anyway: {:?}",
            report.diagnostics
        );
        // And the entry that does compile is still reported as dead.
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == aplan_core::code::PATHS_UNMATCHED
                    && d.message.contains("gone/**")),
            "the valid glob beside the broken one went unjudged: {:?}",
            report.diagnostics
        );
    }

    /// The third instance of the same shape, found by `A1-066`'s audit: a
    /// `url.check` naming a program that is not installed.
    ///
    /// It aborted the run, so an opt-in, advisory network check could cost a
    /// repository every finding it had. Now the checker is reported once, and
    /// no citation is called unanswered on the strength of a check that never
    /// ran -- which is the same distinction `A1-120` drew for a test runner.
    #[test]
    fn a_url_checker_that_cannot_run_is_a_finding_not_an_abort() {
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            url_check: Some(aplan_config::UrlCheck {
                max_memory: None,
                max_processes: None,
                wrap: None,
                timeout_seconds: None,
                check: vec![
                    "definitely-not-a-program-aplan-tests".to_string(),
                    aplan_config::URL_PLACEHOLDER.to_string(),
                ],
            }),
            ..VerifyOptions::default()
        };
        let plan = url_task(&["https://example.com/a"]);
        let report = verify(&plan, dir.path(), &opts).expect("a finding, not an abort");
        let about_config = report
            .diagnostics
            .iter()
            .find(|d| d.task.is_none())
            .expect("a finding about the configuration");
        assert_eq!(about_config.code, aplan_core::code::URL_UNREACHABLE);
        assert!(
            about_config.message.contains("could not be run"),
            "{}",
            about_config.message
        );
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("did not answer")),
            "a citation nothing asked was reported as a dead link: {:?}",
            report.diagnostics
        );
    }

    /// And under `--execute`: a runner that lists fine and then cannot launch
    /// its `run` command has not proved the test fails, so `APLAN014` must not
    /// say it did, and the run must not stop (`A1-066`, audit).
    #[test]
    fn a_run_command_that_will_not_launch_is_not_a_failing_test() {
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            execute: true,
            test_runner: Some(TestRunners::One(runner(
                &["printf", "a_real_test\n"],
                &["definitely-not-a-program-aplan-tests", PLACEHOLDER],
            ))),
            ..VerifyOptions::default()
        };
        let plan = plan_of(done_task("A", vec![test_ev("a_real_test")], vec![]));
        let report = verify(&plan, dir.path(), &opts).expect("a finding, not an abort");
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.code == aplan_core::code::TEST_FAILED),
            "a test nothing ran was reported as failing: {:?}",
            report.diagnostics
        );
        let about_runner = report
            .diagnostics
            .iter()
            .find(|d| d.task.is_none())
            .expect("the runner's fault is reported");
        assert!(
            about_runner.message.contains("could not run"),
            "{}",
            about_runner.message
        );
        // The L1 answer was known, so the citation itself is resolved and no
        // `APLAN013` appears: an execution fault must not suppress findings the
        // listing already settled.
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.task.as_deref() == Some("A")),
            "{:?}",
            report.diagnostics
        );
    }

    /// The two runner faults are not the same fault (`A1-066`, audit).
    ///
    /// A runner that could not say what it lists leaves every citation's L1
    /// unknown, so none may be reported as unlisted. A runner that listed fine
    /// and then could not *run* a test has already answered L1, and suppressing
    /// L1 findings on the strength of an execution fault would hide real ones --
    /// a typo'd citation going quiet because some other test would not launch.
    #[test]
    fn an_execution_fault_does_not_silence_what_the_listing_settled() {
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            execute: true,
            test_runner: Some(TestRunners::One(runner(
                &["printf", "a_real_test\n"],
                &["definitely-not-a-program-aplan-tests", PLACEHOLDER],
            ))),
            ..VerifyOptions::default()
        };
        let mut plan = plan_of(done_task("A", vec![test_ev("a_real_test")], vec![]));
        plan.tasks
            .extend(plan_of(done_task("B", vec![test_ev("no_such_test")], vec![])).tasks);
        let report = verify(&plan, dir.path(), &opts).expect("a report");
        assert!(
            report.diagnostics.iter().any(|d| {
                d.code == aplan_core::code::TEST_UNRESOLVED && d.task.as_deref() == Some("B")
            }),
            "a citation nothing lists went unreported because another test would not \
             launch: {:?}",
            report.diagnostics
        );
    }

    // --- a configured command that never returns (`A1-121`) ------------------

    /// The defect, as a test that would hang without the fix.
    ///
    /// `sleep 30` against a one-second limit: with the timeout this finishes in
    /// about a second, and a regression makes it take thirty, which is the
    /// bounded version of "forever" a test suite can afford to assert.
    /// Proves the named behaviour `verify.configured_commands_are_bounded` (`A1-211`).
    #[test]
    fn a_url_check_that_never_returns_is_abandoned_and_reported() {
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            url_check: Some(aplan_config::UrlCheck {
                max_memory: None,
                max_processes: None,
                wrap: None,
                check: vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "sleep 30".to_string(),
                    aplan_config::URL_PLACEHOLDER.to_string(),
                ],
                timeout_seconds: Some(1),
            }),
            ..VerifyOptions::default()
        };
        let started = std::time::Instant::now();
        let report = verify(&url_task(&["https://example.com/a"]), dir.path(), &opts)
            .expect("a finding, not a wait");
        let took = started.elapsed();
        assert!(
            took < std::time::Duration::from_secs(20),
            "the run waited {took:?}, so nothing bounded it"
        );

        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == aplan_core::code::URL_UNREACHABLE)
            .expect("the citation is reported");
        // Nothing was learned, and the message says which of the three outcomes
        // this is: not answered, not refused, *not checked*.
        assert!(
            d.message.contains("was not checked") && d.message.contains("killed"),
            "a check that was abandoned must not read as a dead link: {}",
            d.message
        );
        assert_eq!(d.severity, aplan_core::Severity::Warning);
    }

    /// The same for a runner that will not answer: an error, because a runner
    /// that cannot list is not a plan that is clean, and it takes the route
    /// `A1-066` built for a runner that could not be asked.
    /// Proves the named behaviour `verify.configured_commands_are_bounded` (`A1-211`).
    #[test]
    fn a_test_list_that_never_returns_is_abandoned_and_reported() {
        let (dir, _repo) = init();
        let mut runner = runner(&["sh", "-c", "sleep 30"], &["true", PLACEHOLDER]);
        runner.timeout_seconds = Some(1);
        let opts = VerifyOptions {
            test_runner: Some(TestRunners::One(runner)),
            ..VerifyOptions::default()
        };
        let started = std::time::Instant::now();
        let plan = plan_of(done_task("A", vec![test_ev("a_real_test")], vec![]));
        let report = verify(&plan, dir.path(), &opts).expect("a finding, not a wait");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "nothing bounded the listing"
        );
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.task.is_none())
            .expect("the runner's fault is reported");
        assert!(
            d.message.contains("had not returned after 1s") && d.message.contains("killed"),
            "{}",
            d.message
        );
        assert_eq!(d.severity, aplan_core::Severity::Error);
    }

    /// Killed with its descendants, not merely signalled (`A1-121`).
    ///
    /// The fixture *forks*: `sh -c 'sleep 3; touch <marker>'` runs `sleep` as a
    /// child of the shell rather than exec'ing it, so killing the direct child
    /// alone leaves the `sleep` running and the marker appears three seconds
    /// later. The gap between the limit and the sleep is deliberate: with the
    /// two equal, the marker and the kill race and the test decides nothing.
    /// Measured before the group kill existed: one stray process per timed-out
    /// citation, on the machine, silently.
    ///
    /// Asserted through the marker rather than through the elapsed time: the run
    /// is bounded whether or not the kill works, by design, so timing proves
    /// nothing about descendants. A file that never appears does.
    #[test]
    fn a_timed_out_command_leaves_no_descendants_behind() {
        let (dir, _repo) = init();
        let marker = dir.path().join("the-descendant-was-here");
        let mut runner = runner(
            &["sh", "-c", &format!("sleep 3; touch {}", marker.display())],
            &["true", PLACEHOLDER],
        );
        // Well below the sleep, so the command is killed mid-sleep.
        runner.timeout_seconds = Some(1);
        let opts = VerifyOptions {
            test_runner: Some(TestRunners::One(runner)),
            ..VerifyOptions::default()
        };
        let plan = plan_of(done_task("A", vec![test_ev("a_real_test")], vec![]));
        let _ = verify(&plan, dir.path(), &opts).expect("a report");

        // Long enough that a surviving `sleep` would have finished and written
        // it, short enough to keep the suite quick.
        std::thread::sleep(std::time::Duration::from_secs(5));
        assert!(
            !marker.exists(),
            "a descendant outlived the command it was started by, and wrote {}",
            marker.display()
        );
    }

    /// A limit of zero is how a repository says "no limit", and it must not read
    /// as "give up immediately".
    #[test]
    fn a_zero_limit_means_no_limit() {
        let (dir, _repo) = init();
        let mut runner = runner(&["printf", "a_real_test\n"], &["true", PLACEHOLDER]);
        runner.timeout_seconds = Some(0);
        let opts = VerifyOptions {
            test_runner: Some(TestRunners::One(runner)),
            ..VerifyOptions::default()
        };
        let plan = plan_of(done_task("A", vec![test_ev("a_real_test")], vec![]));
        let report = verify(&plan, dir.path(), &opts).expect("a report");
        assert!(
            report.diagnostics.is_empty(),
            "a runner that answered was reported anyway: {:?}",
            report.diagnostics
        );
    }

    /// A merge commit is almost never the commit that did the work (`A1-145`).
    ///
    /// The first pilot's case, reproduced: `git rev-parse HEAD` at the wrong
    /// moment names the merge of somebody else's pull request, whose diff
    /// touches the task's `paths` because a merge touches whatever either side
    /// touched -- so `APLAN012` passes and nothing else says anything.
    #[test]
    fn a_cited_merge_is_reported_and_the_commit_that_did_the_work_is_not() {
        let (dir, repo) = init();
        let base = commit(&repo, &[("f.txt", "one")], "first");
        let work = commit(&repo, &[("src/a.rs", "work")], "the work");

        // A second parent, so the merge combines both sides.
        let sig = git2::Signature::now("Test", "test@example.com").expect("sig");
        let base_commit = repo
            .find_commit(git2::Oid::from_str(&base).expect("oid"))
            .expect("base");
        let work_commit = repo
            .find_commit(git2::Oid::from_str(&work).expect("oid"))
            .expect("work");
        let tree = work_commit.tree().expect("tree");
        let merge = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "Merge",
                &tree,
                &[&work_commit, &base_commit],
            )
            .expect("merge")
            .to_string();

        let cite = |sha: &str| {
            plan_of(done_task(
                "A",
                vec![Evidence {
                    kind: EvidenceKind::Commit,
                    reference: sha.to_string(),
                    captured: None,
                }],
                vec!["src/**".to_string()],
            ))
        };

        let report = verify(&cite(&merge), dir.path(), &VerifyOptions::default()).expect("report");
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == aplan_core::code::MERGE_COMMIT_EVIDENCE)
            .expect("the merge is reported");
        assert_eq!(
            d.severity,
            aplan_core::Severity::Warning,
            "not an error: a `--no-ff` repository's merge can be the honest citation"
        );
        assert!(d.message.contains("2 parents"), "{}", d.message);

        // A merge listed *beside* the commit that did the work is context, not a
        // weak proof: judged over the citations together, the way `APLAN024`
        // judges dilution (`A1-145`, found by its audit -- the first version
        // warned per citation and would have pushed an author to delete a true
        // line).
        let both = plan_of(done_task(
            "A",
            vec![
                Evidence {
                    kind: EvidenceKind::Commit,
                    reference: merge.clone(),
                    captured: None,
                },
                Evidence {
                    kind: EvidenceKind::Commit,
                    reference: work.clone(),
                    captured: None,
                },
            ],
            vec!["src/**".to_string()],
        ));
        let report = verify(&both, dir.path(), &VerifyOptions::default()).expect("report");
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.code == aplan_core::code::MERGE_COMMIT_EVIDENCE),
            "a merge beside the commit that did the work was still reported: {:?}",
            report.diagnostics
        );

        // The commit that did the work is not reported, which is what makes the
        // warning about merges rather than about commits.
        let report = verify(&cite(&work), dir.path(), &VerifyOptions::default()).expect("report");
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.code == aplan_core::code::MERGE_COMMIT_EVIDENCE),
            "an ordinary commit was called a merge: {:?}",
            report.diagnostics
        );
    }

    /// The `A1-066` property, at the layer that had the defect: one runner that
    /// will not run must not cost an unrelated task its finding.
    #[test]
    fn a_broken_runner_does_not_take_the_rest_of_the_plan_with_it() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("src/a.rs", "x")], "work");
        let opts = VerifyOptions {
            config_file: None,
            test_runner: Some(TestRunners::One(runner(
                &["bash", "-c", "exit 3"],
                &["true", PLACEHOLDER],
            ))),
            ..VerifyOptions::default()
        };
        // One task whose runner is broken, and one whose commit citation is
        // simply wrong: the second finding is the one that used to vanish.
        let mut plan = plan_of(done_task("A", vec![test_ev("a_real_test")], vec![]));
        plan.tasks.extend(
            plan_of(done_task(
                "B",
                vec![Evidence {
                    kind: EvidenceKind::Commit,
                    reference: sha.clone(),
                    captured: None,
                }],
                vec!["elsewhere/**".to_string()],
            ))
            .tasks,
        );
        let report = verify(&plan, dir.path(), &opts).expect("a finding, not an abort");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == aplan_core::code::COMMIT_IRRELEVANT
                    && d.task.as_deref() == Some("B")),
            "the unrelated task's finding was lost: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn each_runner_is_asked_once_however_many_citations_there_are() {
        // Before this, every runner was asked for every citation: 20 citations
        // against 5 runners spawned 100 processes, and a `list` command is often
        // `cargo test --list`, which compiles. The corpus cannot change during a
        // run, so the second ask was never buying anything.
        let (dir, _repo) = init();
        let counter = dir.path().join("asked.log");
        // The command is inline rather than a script this test writes and then
        // executes. Writing a file and exec'ing it races with every other test
        // that spawns a process: a concurrent fork inherits the still-open write
        // descriptor and the exec fails with `ETXTBSY`. It never reproduced
        // locally in fifteen runs and failed once on CI, which is the worst
        // possible flake. There is no file to be busy now.
        let script = format!("echo x >> {}; printf 'alpha\\nbeta\\n'", counter.display());
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(runner(
                &["bash", "-c", &script],
                &["true", PLACEHOLDER],
            ))),
            max_unrelated_files: None,
            url_check: None,
        };
        // Three tasks, each citing a test.
        let plan = Plan {
            tasks: ["A", "B", "C"]
                .iter()
                .map(|id| aplan_core::load::LoadedTask {
                    task: done_task(id, vec![test_ev("alpha")], vec![]),
                    path: std::path::PathBuf::from(format!("plan/{id}.md")),
                    fields: aplan_core::load::FieldLines::new(),
                    criteria: aplan_core::load::Criteria::default(),
                })
                .collect(),
            load_diagnostics: Vec::new(),
            unloadable: std::collections::BTreeMap::new(),
        };
        let r = verify(&plan, dir.path(), &opts).unwrap();
        assert!(r.is_clean(), "{:?}", r.diagnostics);
        let asked = std::fs::read_to_string(&counter).unwrap().lines().count();
        assert_eq!(
            asked, 1,
            "the runner was asked {asked} times for 3 citations"
        );
    }

    /// `APLAN025`: the runner lists it, but does this task own that code?
    ///
    /// The runner is `printf`, which lists the name wherever it is rooted, so
    /// the only thing under test is the locality judgement.
    fn foreign_test_report(dir: Option<&str>, paths: Vec<String>) -> Report {
        foreign_test_report_with(dir, None, paths)
    }

    fn foreign_test_report_with(
        dir: Option<&str>,
        covers: Option<Vec<&str>>,
        paths: Vec<String>,
    ) -> Report {
        let (tmp, _repo) = init();
        if let Some(d) = dir.filter(|d| !d.trim().is_empty() && *d != ".") {
            std::fs::create_dir_all(tmp.path().join(d)).unwrap();
        }
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(TestRunner {
                max_memory: None,
                max_processes: None,
                wrap: None,
                expect_tests: None,
                skipped: None,
                run_selects: None,
                timeout_seconds: None,
                covers: covers.map(|c| c.iter().map(|s| (*s).to_string()).collect::<Vec<String>>()),
                ..named_runner(
                    "server",
                    dir,
                    &["printf", "app_builds\n"],
                    &["true", PLACEHOLDER],
                )
            })),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task("A", vec![test_ev("app_builds")], paths);
        verify(&plan_of(task), tmp.path(), &opts).unwrap()
    }

    #[test]
    fn a_test_from_a_component_the_task_does_not_own_is_reported() {
        // The p00025 shape: WH6-02 cited a test living in `server/` for work
        // done entirely under `p00025.org/`, and `APLAN013` passed it because a
        // runner really does list that name.
        let report = foreign_test_report(Some("server"), vec!["p00025.org/**".into()]);
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::TEST_IRRELEVANT)
            .expect("the borrowed proof must be reported");
        // Both sides, so the reader can see the mismatch rather than be told
        // about it: the coverage the citation resolved under, and the `paths`
        // that do not reach it.
        assert!(d.message.contains("server"), "{}", d.message);
        assert!(d.message.contains("p00025.org/**"), "{}", d.message);
        // And no cause. `paths` is the least verified input in the model and
        // every rule reading it treats it as ground truth, so a rule
        // triangulating against it may report what it saw and not why: the third
        // pilot's citations were right and its `paths` had lost an entry, and the
        // two explanations have opposite remedies (`A1-070`).
        assert!(
            !d.message.contains("borrows proof"),
            "the message asserts a cause it has not established: {}",
            d.message
        );
        assert!(
            !d.message.contains("another part of the repository"),
            "{}",
            d.message
        );
        // The remedy names both, symmetrically. It used to say "widen `paths` if
        // the work really does span that component", which reads as an
        // accommodation for work that genuinely reaches elsewhere; the case this
        // rule's finding came from was the other one, where the `paths` had lost
        // an entry. An agent applying that condition would have concluded the
        // work does not span, and gone to change the citation, which was right.
        let fix = d.agent_instruction.as_ref().expect("a fix line");
        assert!(fix.contains("Cite a test"), "{fix}");
        assert!(fix.contains("correct `paths`"), "{fix}");
        assert!(
            !fix.contains("really does span"),
            "the fix line still conditions correcting `paths` on the work spanning: {fix}"
        );
        // Advisory: a citation this rule cannot judge is answered by the author,
        // exactly as `APLAN012` is.
        assert!(report.is_clean(), "{:?}", report.diagnostics);
    }

    /// The pilot's own case, which is the one the wording got wrong: the
    /// citations were correct and unchanged, and the task's `paths` had lost the
    /// file it wrote. A message asserting the citation borrowed proof would have
    /// sent the reader to fix the one thing that was right.
    #[test]
    fn the_message_fits_a_task_whose_paths_lost_an_entry() {
        // `gen/schema/**` remains; the entry naming the runner's own tree was
        // removed, which is what the third pilot did by pattern.
        let report = foreign_test_report(Some("xtask"), vec!["gen/schema/**".into()]);
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::TEST_IRRELEVANT)
            .expect("reported");
        // Reads as an observation about two things that do not meet, so either
        // may be the one to change: the coverage the citation resolved under,
        // and the `paths` that do not reach it.
        assert!(
            d.message
                .contains("which this task's `paths` (gen/schema/**) do not reach"),
            "{}",
            d.message
        );
        assert!(d.message.contains("runs in `xtask/`"), "{}", d.message);
    }

    /// The multi-citation form was already neutral, and is the voice the singular
    /// one was brought to. It has to stay that way, so it is asserted rather than
    /// assumed.
    #[test]
    fn the_multi_citation_form_stays_an_observation_and_lists_what_it_saw() {
        let (tmp, _repo) = init();
        std::fs::create_dir_all(tmp.path().join("server")).unwrap();
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(named_runner(
                "server",
                Some("server"),
                &["printf", "a\nb\n"],
                &["true", PLACEHOLDER],
            ))),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task(
            "A",
            vec![test_ev("a"), test_ev("b")],
            vec!["p00025.org/**".into()],
        );
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::TEST_IRRELEVANT)
            .expect("reported");
        assert!(
            d.message.starts_with("none of the 2 tests cited here"),
            "{}",
            d.message
        );
        assert!(
            d.message.contains("`a` from a runner that"),
            "{}",
            d.message
        );
        assert!(
            d.message.contains("`b` from a runner that"),
            "{}",
            d.message
        );
        assert!(!d.message.contains("borrows proof"), "{}", d.message);
        // The `paths` themselves, and this is the load-bearing part: the third
        // pilot found their own mistake because the message printed them and
        // reading the list made the missing entry obvious. Nothing pinned it, so
        // the detail that made the rule work could have been dropped silently
        // (`A1-070`).
        assert!(
            d.message.contains("(p00025.org/**)"),
            "the paths the reader compares against are gone: {}",
            d.message
        );
    }

    #[test]
    fn a_runner_at_the_repository_root_makes_no_claim_about_locality() {
        // Every single-runner config is this, so the rule must be silent here or
        // it would fire on almost every repository that uses `test` evidence.
        for dir in [None, Some("."), Some("")] {
            let report = foreign_test_report(dir, vec!["p00025.org/**".into()]);
            assert!(
                !codes(&report).contains(&code::TEST_IRRELEVANT),
                "dir {dir:?}: {:?}",
                report.diagnostics
            );
        }
    }

    #[test]
    fn a_task_with_no_paths_is_not_reported_twice() {
        // Relevance is unverifiable without `paths`, which is `APLAN023`'s
        // finding; restating it here would just double the noise.
        let report = foreign_test_report(Some("server"), vec![]);
        assert!(
            !codes(&report).contains(&code::TEST_IRRELEVANT),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_test_from_the_code_the_task_owns_is_silent() {
        // Either nesting direction counts as reaching: the task may own the
        // runner's whole tree, or one directory inside it.
        for paths in [
            vec!["server/**".into()],
            vec!["server/p00025-server/src/**".into()],
            vec!["server/p00025-server/src/app.rs".into()],
            vec!["p00025.org/**".into(), "server/**".into()],
        ] {
            let report = foreign_test_report(Some("server"), paths.clone());
            assert!(
                !codes(&report).contains(&code::TEST_IRRELEVANT),
                "{paths:?}: {:?}",
                report.diagnostics
            );
        }
    }

    #[test]
    fn a_glob_that_could_match_at_any_depth_is_not_a_demonstrable_mismatch() {
        // `**/*.rs` has no literal directory, so it may well match inside the
        // runner's tree. The rule reports mismatches it can show, not ones it
        // merely suspects.
        for paths in [vec!["**/*.rs".into()], vec!["README.md".into()]] {
            let report = foreign_test_report(Some("server"), paths.clone());
            assert!(
                !codes(&report).contains(&code::TEST_IRRELEVANT),
                "{paths:?}: {:?}",
                report.diagnostics
            );
        }
    }

    #[test]
    fn a_runner_that_selects_a_component_without_changing_directory_can_say_so() {
        // The p00025 shape, and the whole reason for `A1-065`: five runners that
        // each select a component with `--manifest-path` while running at the
        // repository root. With only `dir` to read, `APLAN025` could not fire on
        // any of them.
        let report =
            foreign_test_report_with(None, Some(vec!["server/**"]), vec!["p00025.org/**".into()]);
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::TEST_IRRELEVANT)
            .expect("a runner with `covers` and no `dir` must still be checkable");
        // Named the way the config was written, or the author hunts for the
        // wrong key.
        assert!(d.message.contains("covers `server/**`"), "{}", d.message);
    }

    #[test]
    fn covers_that_overlaps_the_task_is_silent() {
        for covers in [vec!["p00025.org/**"], vec!["server/**", "p00025.org/**"]] {
            let report =
                foreign_test_report_with(None, Some(covers.clone()), vec!["p00025.org/**".into()]);
            assert!(
                !codes(&report).contains(&code::TEST_IRRELEVANT),
                "{covers:?}: {:?}",
                report.diagnostics
            );
        }
    }

    #[test]
    fn covers_beats_dir_in_both_directions() {
        // It is the more specific claim, so it must be able to both acquit and
        // convict a runner whose `dir` says otherwise.
        let acquitted = foreign_test_report_with(
            Some("server"),
            Some(vec!["p00025.org/**"]),
            vec!["p00025.org/**".into()],
        );
        assert!(
            !codes(&acquitted).contains(&code::TEST_IRRELEVANT),
            "covers should have overridden a mismatched dir: {:?}",
            acquitted.diagnostics
        );

        let convicted = foreign_test_report_with(
            Some("server"),
            Some(vec!["libs/**"]),
            vec!["server/**".into()],
        );
        assert!(
            codes(&convicted).contains(&code::TEST_IRRELEVANT),
            "covers should have overridden a matching dir: {:?}",
            convicted.diagnostics
        );
    }

    #[test]
    fn a_test_covering_the_task_settles_it_however_many_others_are_cited() {
        // `p00025`'s WH2-13, and the reason this is judged per task: it cites two
        // tests in the component it changed plus an `xtask` test that parses the
        // spec's own examples through that component. Warning there is warning
        // about a repository doing the right thing, and under `deny_warnings` it
        // would fail the build for it.
        let (tmp, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::Many(vec![
                TestRunner {
                    max_memory: None,
                    max_processes: None,
                    wrap: None,
                    expect_tests: None,
                    skipped: None,
                    run_selects: None,
                    timeout_seconds: None,
                    covers: Some(vec!["libs/**".into()]),
                    ..named_runner("libs", None, &["printf", "local\n"], &["true", PLACEHOLDER])
                },
                TestRunner {
                    max_memory: None,
                    max_processes: None,
                    wrap: None,
                    expect_tests: None,
                    skipped: None,
                    run_selects: None,
                    timeout_seconds: None,
                    covers: Some(vec!["xtask/**".into()]),
                    ..named_runner("xtask", None, &["printf", "spec\n"], &["true", PLACEHOLDER])
                },
            ])),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task(
            "A",
            vec![test_ev("spec"), test_ev("local")],
            vec!["libs/p00025-core/src/format.rs".into()],
        );
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        assert!(
            !codes(&report).contains(&code::TEST_IRRELEVANT),
            "a local citation is proof, and the cross-component one is context: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_task_whose_every_cited_test_is_foreign_is_still_reported() {
        // The other side of the same judgement: several citations, none of them
        // covering the task, is exactly the borrowed proof the rule is for.
        let (tmp, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::Many(vec![
                TestRunner {
                    max_memory: None,
                    max_processes: None,
                    wrap: None,
                    expect_tests: None,
                    skipped: None,
                    run_selects: None,
                    timeout_seconds: None,
                    covers: Some(vec!["server/**".into()]),
                    ..named_runner("server", None, &["printf", "srv\n"], &["true", PLACEHOLDER])
                },
                TestRunner {
                    max_memory: None,
                    max_processes: None,
                    wrap: None,
                    expect_tests: None,
                    skipped: None,
                    run_selects: None,
                    timeout_seconds: None,
                    covers: Some(vec!["xtask/**".into()]),
                    ..named_runner("xtask", None, &["printf", "spec\n"], &["true", PLACEHOLDER])
                },
            ])),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task(
            "A",
            vec![test_ev("spec"), test_ev("srv")],
            vec!["p00025.org/**".into()],
        );
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::TEST_IRRELEVANT)
            .expect("no cited test covers this task");
        // One finding for the task, not one per citation, and it names both.
        assert_eq!(
            codes(&report)
                .iter()
                .filter(|c| **c == code::TEST_IRRELEVANT)
                .count(),
            1
        );
        assert!(
            d.message.contains("spec") && d.message.contains("srv"),
            "{}",
            d.message
        );
    }

    #[test]
    fn a_runner_claiming_nothing_acquits_the_whole_task() {
        // A citation this tool cannot localise is not evidence of a mismatch,
        // and the rule reports only what it can demonstrate.
        let (tmp, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::Many(vec![
                TestRunner {
                    max_memory: None,
                    max_processes: None,
                    wrap: None,
                    expect_tests: None,
                    skipped: None,
                    run_selects: None,
                    timeout_seconds: None,
                    covers: Some(vec!["server/**".into()]),
                    ..named_runner("server", None, &["printf", "srv\n"], &["true", PLACEHOLDER])
                },
                named_runner(
                    "whole repo",
                    None,
                    &["printf", "any\n"],
                    &["true", PLACEHOLDER],
                ),
            ])),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task(
            "A",
            vec![test_ev("srv"), test_ev("any")],
            vec!["p00025.org/**".into()],
        );
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        assert!(
            !codes(&report).contains(&code::TEST_IRRELEVANT),
            "{:?}",
            report.diagnostics
        );
    }

    /// `A1-069`: a runner whose invariant tests cover the tree they assert about,
    /// while the rest of the same crate covers only itself.
    fn invariant_runner() -> TestRunner {
        TestRunner {
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers: Some(vec!["xtask/**".into()]),
            covers_by_test: Some(vec![aplan_config::TestCoverage {
                tests: vec!["invariants::*".into()],
                paths: vec!["**".into()],
            }]),
            ..named_runner(
                "xtask",
                None,
                &[
                    "printf",
                    "invariants::tests::the_web_skeleton_exists\nunit::tests::a_helper\n",
                ],
                &["true", PLACEHOLDER],
            )
        }
    }

    #[test]
    fn a_test_asserting_a_fact_about_a_tree_it_does_not_live_in_is_not_borrowed() {
        // p00025's WH5-01: the only harness that can read the tree is `xtask`, so
        // the test proving the web skeleton exists lives there and opens
        // `p00025.org/web/package.json`. Calling that borrowed is the opposite of
        // true, and `APLAN024` is what asks for such tests in the first place.
        let (tmp, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(invariant_runner())),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task(
            "A",
            vec![test_ev("invariants::tests::the_web_skeleton_exists")],
            vec!["p00025.org/web/**".into()],
        );
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        assert!(
            !codes(&report).contains(&code::TEST_IRRELEVANT),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn the_rest_of_the_same_runner_still_covers_only_itself() {
        // The narrowing must not become a blanket exemption: a test outside the
        // named subset falls back to the runner's `covers`, so the case the rule
        // was built for still fires from the very same runner.
        let (tmp, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(invariant_runner())),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task(
            "A",
            vec![test_ev("unit::tests::a_helper")],
            vec!["p00025.org/web/**".into()],
        );
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::TEST_IRRELEVANT)
            .expect("an ordinary xtask test still covers only xtask");
        assert!(d.message.contains("covers `xtask/**`"), "{}", d.message);
    }

    #[test]
    fn the_message_names_the_subset_when_one_matched() {
        // If a per-test claim is what decided it, the author has to be told which
        // one, or they cannot see why the verdict came out as it did.
        let (tmp, _repo) = init();
        let runner = TestRunner {
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers_by_test: Some(vec![aplan_config::TestCoverage {
                tests: vec!["invariants::*".into()],
                paths: vec!["libs/**".into()],
            }]),
            ..invariant_runner()
        };
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(runner)),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task(
            "A",
            vec![test_ev("invariants::tests::the_web_skeleton_exists")],
            vec!["p00025.org/web/**".into()],
        );
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::TEST_IRRELEVANT)
            .expect("the subset claims libs, which this task does not own");
        assert!(
            d.message.contains("for tests matching `invariants::*`"),
            "{}",
            d.message
        );
    }

    #[test]
    fn the_first_matching_subset_wins() {
        // Documented as first-match-wins, so a repository can put a narrow entry
        // ahead of a broad one. Untested, that is just a comment.
        let (tmp, _repo) = init();
        let runner = TestRunner {
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers: Some(vec!["xtask/**".into()]),
            covers_by_test: Some(vec![
                aplan_config::TestCoverage {
                    tests: vec!["invariants::web::*".into()],
                    paths: vec!["p00025.org/web/**".into()],
                },
                aplan_config::TestCoverage {
                    tests: vec!["invariants::*".into()],
                    paths: vec!["**".into()],
                },
            ]),
            ..named_runner(
                "xtask",
                None,
                &["printf", "invariants::web::skeleton\n"],
                &["true", PLACEHOLDER],
            )
        };
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(runner)),
            max_unrelated_files: None,
            url_check: None,
        };
        // The narrow entry claims only the web tree, so a task elsewhere is
        // reported even though the broad entry behind it would have covered.
        let task = done_task(
            "A",
            vec![test_ev("invariants::web::skeleton")],
            vec!["libs/**".into()],
        );
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.code == code::TEST_IRRELEVANT)
            .expect("the first entry decides, and it claims only the web tree");
        assert!(
            d.message.contains("p00025.org/web/**"),
            "the first entry's claim must be the one reported: {}",
            d.message
        );
    }

    #[test]
    fn a_subset_can_narrow_a_runner_that_would_otherwise_have_covered() {
        // The override runs both ways. `covers` says the whole tree, and the
        // subset says this particular test only proves something about xtask.
        let (tmp, _repo) = init();
        let runner = TestRunner {
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers: Some(vec!["**".into()]),
            covers_by_test: Some(vec![aplan_config::TestCoverage {
                tests: vec!["unit::*".into()],
                paths: vec!["xtask/**".into()],
            }]),
            ..named_runner(
                "xtask",
                None,
                &["printf", "unit::a\n"],
                &["true", PLACEHOLDER],
            )
        };
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(runner)),
            max_unrelated_files: None,
            url_check: None,
        };
        let task = done_task("A", vec![test_ev("unit::a")], vec!["libs/**".into()]);
        let report = verify(&plan_of(task), tmp.path(), &opts).unwrap();
        assert!(
            codes(&report).contains(&code::TEST_IRRELEVANT),
            "a subset must be able to narrow, not only widen: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn a_name_two_runners_list_is_refused_rather_than_guessed() {
        // Resolving it would make the verdict depend on the order of a config
        // file. The message has to name both, or the author cannot act on it.
        let (dir, _repo) = init();
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::Many(vec![
                named_runner("libs", None, &["printf", "same\n"], &["true", PLACEHOLDER]),
                named_runner("web", None, &["printf", "same\n"], &["true", PLACEHOLDER]),
            ])),
            max_unrelated_files: None,
            url_check: None,
        };
        let r = verify(
            &plan_of(done_task("A", vec![test_ev("same")], vec![])),
            dir.path(),
            &opts,
        )
        .unwrap();
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == aplan_core::diagnostic::code::TEST_UNRESOLVED)
            .expect("ambiguity is a finding");
        assert!(d.message.contains("more than one runner"), "{}", d.message);
        assert!(
            d.message.contains("libs") && d.message.contains("web"),
            "{}",
            d.message
        );
    }

    #[test]
    fn a_runner_runs_in_its_own_directory() {
        // Without `dir`, the feature covers Rust and little else: a JavaScript or
        // Python runner has to run inside its package directory.
        let (dir, _repo) = init();
        let sub = dir.path().join("web");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("names.txt"), "only_here\n").unwrap();
        let opts = VerifyOptions {
            config_file: None,
            execute: false,
            // `cat names.txt` resolves only if the process runs in `web/`.
            test_runner: Some(TestRunners::Many(vec![named_runner(
                "web",
                Some("web"),
                &["cat", "names.txt"],
                &["true", PLACEHOLDER],
            )])),
            max_unrelated_files: None,
            url_check: None,
        };
        let r = verify(
            &plan_of(done_task("A", vec![test_ev("only_here")], vec![])),
            dir.path(),
            &opts,
        )
        .unwrap();
        assert!(
            r.is_clean(),
            "the runner did not run in its dir: {:?}",
            r.diagnostics
        );
    }

    #[test]
    fn test_evidence_resolves_lists_and_executes() {
        let (dir, _repo) = init();
        // `list` prints two names, one per line; `run` passes (`true`).
        let good = runner(&["printf", "alpha\nbeta\n"], &["true", PLACEHOLDER]);
        let opts = VerifyOptions {
            config_file: None,
            execute: true,
            test_runner: Some(TestRunners::One(good.clone())),
            max_unrelated_files: None,
            url_check: None,
        };

        // Exists and passes -> clean.
        let r = verify(
            &plan_of(done_task("A", vec![test_ev("alpha")], vec![])),
            dir.path(),
            &opts,
        )
        .unwrap();
        assert!(r.is_clean(), "{:?}", r.diagnostics);

        // Not listed -> APLAN013.
        let r = verify(
            &plan_of(done_task("B", vec![test_ev("ghost")], vec![])),
            dir.path(),
            &opts,
        )
        .unwrap();
        assert_eq!(codes(&r), vec![code::TEST_UNRESOLVED]);

        // Exists but the runner exits non-zero under --execute -> APLAN014.
        let failing = runner(&["printf", "alpha\n"], &["false", PLACEHOLDER]);
        let opts_fail = VerifyOptions {
            config_file: None,
            execute: true,
            test_runner: Some(TestRunners::One(failing.clone())),
            max_unrelated_files: None,
            url_check: None,
        };
        let r = verify(
            &plan_of(done_task("C", vec![test_ev("alpha")], vec![])),
            dir.path(),
            &opts_fail,
        )
        .unwrap();
        assert_eq!(codes(&r), vec![code::TEST_FAILED]);
        assert!(
            r.diagnostics[0].message.contains("alpha"),
            "the message must name the test that failed: {}",
            r.diagnostics[0].message
        );

        // Without --execute the test is never run: only existence (L1) is checked.
        let opts_noexec = VerifyOptions {
            config_file: None,
            execute: false,
            test_runner: Some(TestRunners::One(failing)),
            max_unrelated_files: None,
            url_check: None,
        };
        let r = verify(
            &plan_of(done_task("D", vec![test_ev("alpha")], vec![])),
            dir.path(),
            &opts_noexec,
        )
        .unwrap();
        assert!(
            r.is_clean(),
            "no --execute must not run: {:?}",
            r.diagnostics
        );
    }

    /// `APLAN036`: a runner that answers and names nothing is reported against
    /// the configuration, once, and the benign case is expressible (`A1-064`).
    ///
    /// The distinction the rule exists for: a runner that *cannot be asked* is
    /// `APLAN013` and needs a different fix, and until this rule the two were
    /// indistinguishable -- both ended as citations that "no runner lists",
    /// which sends the author to the task file.
    /// `APLAN037`: a passing `--execute` run that a miss would have produced.
    ///
    /// The ladder's top rung says the cited test *passed*, and it is decided by
    /// an exit status the runner may not have earned. Each runner below is asked
    /// what a miss looks like and judged against its own answer, which is the
    /// only way cargo (exit `0` on a miss, and correct) and vitest (exit `0` on
    /// a miss, and blind) can be told apart at all.
    #[test]
    fn a_run_indistinguishable_from_a_miss_is_reported_against_the_configuration() {
        let (dir, _repo) = init();
        let plan = plan_of(done_task("A", vec![test_ev("known")], vec![]));
        let config = dir.path().join(".aplan.yml");
        let verdict = |r: TestRunner| {
            let opts = VerifyOptions {
                config_file: Some(config.clone()),
                test_runner: Some(TestRunners::One(r)),
                execute: true,
                ..VerifyOptions::default()
            };
            verify(&plan, dir.path(), &opts).unwrap()
        };
        let lists = ["printf", "known\\n"];

        // Says nothing, exits 0 whatever it is handed: a miss and a pass are the
        // same bytes, so `--execute` proved nothing.
        let r = verdict(runner(&lists, &["true", PLACEHOLDER]));
        let found: Vec<_> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::RUN_PROVES_NOTHING)
            .collect();
        assert_eq!(found.len(), 1, "once per runner: {:?}", r.diagnostics);
        assert!(
            found[0].task.is_none(),
            "the pairing of `list` and `run` is a fact about the configuration"
        );
        assert!(
            found[0].message.contains("did not prove the test ran"),
            "{}",
            found[0].message
        );
        assert_eq!(
            found[0].file.as_deref(),
            Some(config.as_path()),
            "it is about the runner, so it lands where the runner is declared"
        );

        // Distinguishable by *output*, which is the cargo case: a zero status on
        // a miss, and a hit that says something a miss does not.
        let echoes = ["sh", "-c", "echo ran: $1", "sh", PLACEHOLDER];
        assert!(
            !verdict(runner(&lists, &echoes))
                .diagnostics
                .iter()
                .any(|d| d.code == code::RUN_PROVES_NOTHING),
            "a runner whose output differs on a hit is not blind"
        );

        // Distinguishable by *status*, which is the pytest case, and costs no
        // comparison at all.
        let refuses = ["sh", "-c", "[ $1 = known ] || exit 4", "sh", PLACEHOLDER];
        assert!(
            !verdict(runner(&lists, &refuses))
                .diagnostics
                .iter()
                .any(|d| d.code == code::RUN_PROVES_NOTHING),
            "a runner that refuses an unknown name has already answered"
        );
        // A runner that stamps its own output with a number that changes every
        // run, and changes *width* -- 7 then 14, so a mask that replaced digits
        // one for one would still call the two outputs different, and a run of
        // digits has to collapse to a single token for this to work at all.
        // Both lines below are what a miss prints too, so this *is* blind;
        // without the digit mask the counter alone would read as a difference
        // and the false green would go unreported. This is the case that killed
        // the first design, where volatility was learned from two sentinel runs:
        // two vitest runs printed the same clock second often enough that the
        // verdict depended on whether a second had ticked.
        let counting = [
            "sh",
            "-c",
            "n=$(cat n.txt 2>/dev/null || echo 0); n=$((n+7)); echo $n > n.txt; \
             echo 'Tests  3 skipped (3)'; echo \"Duration  ${n}ms\"",
            "sh",
            PLACEHOLDER,
        ];
        let r = verdict(runner(&lists, &counting));
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::RUN_PROVES_NOTHING),
            "a number that changes every run is not a difference: {:?}",
            r.diagnostics
        );

        // Only a *passing* run can be a false green. This one prints exactly what
        // a miss prints and then fails, which `APLAN014` already reports and
        // already sends the author to look at; saying it twice, in two voices,
        // would be the flood `A1-066` refuses.
        let fails_loudly = [
            "sh",
            "-c",
            "echo 'Tests  3 skipped (3)'; [ $1 = known ] && exit 1; exit 0",
            "sh",
            PLACEHOLDER,
        ];
        let r = verdict(runner(&lists, &fails_loudly));
        assert!(
            r.diagnostics.iter().any(|d| d.code == code::TEST_FAILED),
            "the failure is the finding: {:?}",
            r.diagnostics
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::RUN_PROVES_NOTHING),
            "a run that failed is not a pass that proved nothing: {:?}",
            r.diagnostics
        );

        // A runner that answers on stderr has still answered. Both streams are
        // compared, because which one a runner reports on is its business.
        let on_stderr = [
            "sh",
            "-c",
            "[ $1 = known ] && echo ran >&2; exit 0",
            "sh",
            PLACEHOLDER,
        ];
        assert!(
            !verdict(runner(&lists, &on_stderr))
                .diagnostics
                .iter()
                .any(|d| d.code == code::RUN_PROVES_NOTHING),
            "stderr is output too"
        );
    }

    /// The probe's cost and its escape: asked once per runner, and declarable.
    #[test]
    fn a_miss_is_probed_once_per_runner_and_can_be_declared_away() {
        let (dir, _repo) = init();
        let plan = plan_of(done_task("A", vec![test_ev("known")], vec![]));
        let verdict = |r: TestRunner| {
            let opts = VerifyOptions {
                config_file: None,
                test_runner: Some(TestRunners::One(r)),
                execute: true,
                ..VerifyOptions::default()
            };
            verify(&plan, dir.path(), &opts).unwrap()
        };
        let lists = ["printf", "known\n"];

        // `false` is the same as leaving it out: the key is a one-sided claim,
        // and silencing a rule on the strength of agreeing with it would be the
        // one reading nobody wants.
        let mut agrees = runner(&lists, &["true", PLACEHOLDER]);
        agrees.run_selects = Some(false);
        assert!(
            verdict(agrees)
                .diagnostics
                .iter()
                .any(|d| d.code == code::RUN_PROVES_NOTHING),
            "`run_selects: false` is not a silencer"
        );

        // The declared claim, for a runner that is right and says nothing.
        let mut declared = runner(&lists, &["true", PLACEHOLDER]);
        declared.run_selects = Some(true);
        assert!(
            !verdict(declared)
                .diagnostics
                .iter()
                .any(|d| d.code == code::RUN_PROVES_NOTHING),
            "`run_selects: true` must silence it"
        );

        // The probe is asked once per runner, not once per citation: it is a
        // property of the `list`/`run` pairing, and a spawn per citation would
        // double the cost of `--execute` for an answer that cannot change.
        let two = plan_of(done_task(
            "A",
            vec![test_ev("known"), test_ev("other")],
            vec![],
        ));
        let log = dir.path().join("spawns.log");
        let counted = runner(
            &["printf", "known\nother\n"],
            &[
                "sh",
                "-c",
                "echo x >> spawns.log; exit 0",
                "sh",
                PLACEHOLDER,
            ],
        );
        let opts = VerifyOptions {
            config_file: None,
            test_runner: Some(TestRunners::One(counted)),
            execute: true,
            ..VerifyOptions::default()
        };
        let r = verify(&two, dir.path(), &opts).unwrap();
        let spawns = std::fs::read_to_string(&log).unwrap().lines().count();
        assert_eq!(spawns, 3, "two citations and one sentinel, not one each");

        // And one finding for the two of them, which says how many it covers.
        let found: Vec<_> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::RUN_PROVES_NOTHING)
            .collect();
        assert_eq!(found.len(), 1, "{:?}", r.diagnostics);
        assert!(
            found[0].message.contains("(2 citations)"),
            "the reader is told how much rests on it: {}",
            found[0].message
        );

        // Silent without `--execute`: nothing ran, so nothing is claimed.
        let opts = VerifyOptions {
            config_file: None,
            test_runner: Some(TestRunners::One(runner(&lists, &["true", PLACEHOLDER]))),
            ..VerifyOptions::default()
        };
        assert!(
            !verify(&plan, dir.path(), &opts)
                .unwrap()
                .diagnostics
                .iter()
                .any(|d| d.code == code::RUN_PROVES_NOTHING),
            "the rung is only claimed under `--execute`"
        );
    }

    /// Proves the named behaviour `verify.one_tasks_defect_does_not_stop_the_run` (`A1-211`).
    #[test]
    fn a_runner_that_lists_nothing_is_reported_against_the_configuration() {
        let (dir, repo) = init();
        let blind = runner(&["printf", ""], &["true", PLACEHOLDER]);
        let config = dir.path().join(".aplan.yml");
        let opts = VerifyOptions {
            config_file: Some(config.clone()),
            test_runner: Some(TestRunners::One(blind.clone())),
            ..VerifyOptions::default()
        };
        let plan = plan_of(done_task("A", vec![test_ev("ghost")], vec![]));
        let r = verify(&plan, dir.path(), &opts).unwrap();

        let empties: Vec<_> = r
            .diagnostics
            .iter()
            .filter(|d| d.code == code::RUNNER_LISTS_NOTHING)
            .collect();
        assert_eq!(empties.len(), 1, "once per runner: {:?}", r.diagnostics);
        assert!(
            empties[0].task.is_none(),
            "it is a fact about the configuration, not about a task"
        );
        assert!(
            empties[0].message.contains("lists no tests"),
            "{}",
            empties[0].message
        );
        assert_eq!(
            empties[0].file.as_deref(),
            Some(config.as_path()),
            "it is about the runner, so it lands where the runner is declared"
        );

        // Declared inert: the component genuinely has no tests yet.
        let mut inert = blind;
        inert.expect_tests = Some(false);
        let opts = VerifyOptions {
            config_file: None,
            test_runner: Some(TestRunners::One(inert)),
            ..VerifyOptions::default()
        };
        let r = verify(&plan, dir.path(), &opts).unwrap();
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::RUNNER_LISTS_NOTHING),
            "`expect_tests: false` must silence it: {:?}",
            r.diagnostics
        );

        // A runner that could not be *asked* is a different fault with a
        // different fix, and telling them apart is the whole point: one is a
        // command that fails, the other a command that succeeds and prints
        // nothing. The failure keeps `APLAN013`.
        // Beside a runner that answers, so the run is one this rule reports on
        // at all: a lone broken runner leaves the plan with no listing to read,
        // and would pass this by never being asked the question.
        let broken = runner(
            &["sh", "-c", "echo boom >&2; exit 2"],
            &["true", PLACEHOLDER],
        );
        let answers = runner(&["printf", "ghost\n"], &["true", PLACEHOLDER]);
        let opts = VerifyOptions {
            config_file: None,
            test_runner: Some(TestRunners::Many(vec![answers, broken])),
            ..VerifyOptions::default()
        };
        let r = verify(&plan, dir.path(), &opts).unwrap();
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == code::TEST_UNRESOLVED),
            "a runner that cannot be asked is still reported: {:?}",
            r.diagnostics
        );
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::RUNNER_LISTS_NOTHING),
            "a failing `list` is not an empty one: {:?}",
            r.diagnostics
        );

        // And a plan with no `test` evidence never asks. The listing command
        // here does not exist, so asking it would raise the fault above; a
        // clean report is the proof that nothing was spawned, which is what
        // keeps `verify` on a commit-only plan from compiling a workspace.
        let never = runner(&["aplan-no-such-command-a1-064"], &["true", PLACEHOLDER]);
        let opts = VerifyOptions {
            config_file: None,
            test_runner: Some(TestRunners::One(never)),
            ..VerifyOptions::default()
        };
        let head = commit(&repo, &[("src/x.rs", "fn x() {}")], "work");
        let commit_only = plan_of(done_task(
            "A",
            vec![commit_ev(&head)],
            vec!["src/**".to_string()],
        ));
        let r = verify(&commit_only, dir.path(), &opts).unwrap();
        assert!(
            !r.diagnostics
                .iter()
                .any(|d| d.code == code::TEST_UNRESOLVED || d.code == code::RUNNER_LISTS_NOTHING),
            "a plan with no `test` evidence must not ask any runner: {:?}",
            r.diagnostics
        );
    }

    #[test]
    fn test_evidence_is_unresolved_without_a_runner() {
        // No runner configured: test evidence stays at L0 and verify is clean.
        let (dir, _repo) = init();
        let r = verify(
            &plan_of(done_task("A", vec![test_ev("whatever")], vec![])),
            dir.path(),
            &VerifyOptions::default(),
        )
        .unwrap();
        assert!(r.is_clean(), "{:?}", r.diagnostics);
    }

    /// The security-critical property (HLD §5.5, A1-004): a hostile test name in
    /// a task file cannot run a command. If it were spliced into a shell this
    /// would create the sentinel; with argv spawning and a standalone
    /// placeholder it is just an argument `true` ignores.
    #[test]
    fn a_hostile_test_name_cannot_inject_a_command() {
        let (dir, _repo) = init();
        let sentinel = dir.path().join("PWNED");
        let payload = format!("x; touch {} #", sentinel.display());
        // The runner lists the payload as valid so resolution reaches --execute.
        let list_arg = format!("{payload}\n");
        let opts = VerifyOptions {
            config_file: None,
            execute: true,
            test_runner: Some(TestRunners::One(runner(
                &["printf", &list_arg],
                &["true", PLACEHOLDER],
            ))),
            max_unrelated_files: None,
            url_check: None,
        };
        let report = verify(
            &plan_of(done_task("A", vec![test_ev(&payload)], vec![])),
            dir.path(),
            &opts,
        )
        .unwrap();
        assert!(report.is_clean(), "{:?}", report.diagnostics);
        assert!(
            !sentinel.exists(),
            "the test name was interpreted as a command"
        );
    }
}
