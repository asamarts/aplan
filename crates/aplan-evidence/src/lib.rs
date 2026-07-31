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
//! `.aplan.yml` and spawned with no shell (see [`config`], HLD §5.5).
//!
//! Adding an evidence kind is a new [`EvidenceResolver`] impl, not a change to
//! [`verify`]. `url` reachability stays absent: it is opt-in, non-deterministic,
//! and needs config keys that are v0.4.

mod repo;
mod resolve;

pub use repo::Repo;
pub use resolve::{CommitResolver, EvidenceResolver, FileResolver, TestResolver};

use std::path::Path;

use aplan_config::TestRunner;
use aplan_core::{Plan, Report};

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
    #[error("task `{0}`: `{1}` is not a valid path glob")]
    BadGlob(String, String),
    #[error("running the test runner: {0}")]
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
    /// The trusted test runner from `.aplan.yml`. `None` leaves `test` evidence
    /// at L0 (present-and-well-formed), the way `url` is not fetched by default.
    pub test_runner: Option<TestRunner>,
}

/// The full SHA of `HEAD` in the repository at `repo_root`, for `aplan close` to
/// record the commit just made as evidence. Errors if there is no repository or
/// no commit yet.
pub fn head_sha(repo_root: &Path) -> Result<String, Error> {
    Repo::discover(repo_root)?
        .head_sha()
        .ok_or_else(|| Error::NoCommits(repo_root.to_path_buf()))
}

/// Resolve every task's evidence against the repository at `repo_root`.
///
/// Returns a [`Report`] of findings (empty when all evidence resolves), or an
/// [`Error`] if the repo cannot be opened or a task carries a malformed glob.
pub fn verify(plan: &Plan, repo_root: &Path, opts: &VerifyOptions) -> Result<Report, Error> {
    let repo = Repo::discover(repo_root)?;
    let mut resolvers: Vec<Box<dyn EvidenceResolver>> =
        vec![Box::new(CommitResolver), Box::new(FileResolver)];
    // The test resolver joins only when a runner is configured; without one,
    // `test` evidence has no L1/L3 and stays at L0.
    if let Some(runner) = &opts.test_runner {
        resolvers.push(Box::new(TestResolver::new(runner.clone(), opts.execute)));
    }

    let mut diagnostics = Vec::new();
    for lt in &plan.tasks {
        for ev in &lt.task.evidence {
            for r in &resolvers {
                if r.kind() == ev.kind {
                    r.resolve(&repo, &lt.task, &lt.path, ev, &mut diagnostics)?;
                }
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_config::PLACEHOLDER;
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
        let dir = tempfile::tempdir().unwrap();
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
            evidence,
            paths,
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
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
            list: list.iter().map(|s| (*s).to_string()).collect(),
            run: run.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn plan_of(task: Task) -> Plan {
        Plan {
            tasks: vec![LoadedTask {
                task,
                path: PathBuf::from("plan/tasks/x.md"),
            }],
            load_diagnostics: Vec::new(),
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
    }

    #[test]
    fn a_real_but_irrelevant_commit_fires_012() {
        let (dir, repo) = init();
        let sha = commit(&repo, &[("docs/notes.md", "prose")], "docs only");
        let task = done_task("A", vec![commit_ev(&sha)], vec!["src/**".into()]);
        let report = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap();
        assert_eq!(codes(&report), vec![code::COMMIT_IRRELEVANT]);
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
            assert_eq!(
                codes(&report),
                vec![code::COMMIT_IRRELEVANT],
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
        assert_eq!(
            codes(&report),
            vec![code::COMMIT_IRRELEVANT],
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
        let dir = tempfile::tempdir().unwrap();
        let task = done_task("A", vec![commit_ev("abc123")], vec![]);
        let err = verify(&plan_of(task), dir.path(), &VerifyOptions::default()).unwrap_err();
        assert!(matches!(err, Error::NotARepository(_)), "{err:?}");
    }

    #[test]
    fn test_evidence_resolves_lists_and_executes() {
        let (dir, _repo) = init();
        // `list` prints two names, one per line; `run` passes (`true`).
        let good = runner(&["printf", "alpha\nbeta\n"], &["true", PLACEHOLDER]);
        let opts = VerifyOptions {
            execute: true,
            test_runner: Some(good.clone()),
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
            execute: true,
            test_runner: Some(failing.clone()),
        };
        let r = verify(
            &plan_of(done_task("C", vec![test_ev("alpha")], vec![])),
            dir.path(),
            &opts_fail,
        )
        .unwrap();
        assert_eq!(codes(&r), vec![code::TEST_FAILED]);

        // Without --execute the test is never run: only existence (L1) is checked.
        let opts_noexec = VerifyOptions {
            execute: false,
            test_runner: Some(failing),
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
            execute: true,
            test_runner: Some(runner(&["printf", &list_arg], &["true", PLACEHOLDER])),
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
