//! Evidence resolvers, one per kind. Adding a kind is a new `impl`, not a
//! change to the engine (HLD §5, the `EvidenceResolver` extension point).

use std::path::Path;
use std::process::Command;

use aplan_core::task::{Evidence, EvidenceKind, Task};
use aplan_core::{Diagnostic, code};
use globset::{GlobBuilder, GlobSetBuilder};

use aplan_config::TestRunner;

use crate::Error;
use crate::repo::Repo;

/// Resolve one kind of evidence against the repository, emitting diagnostics.
///
/// `resolve` returns `Err` only for a genuine internal failure (a corrupt repo,
/// a malformed glob in the task), a *finding* about the evidence is a pushed
/// diagnostic, not an error, so one questionable citation never aborts the run.
pub trait EvidenceResolver {
    /// The evidence kind this resolver handles.
    fn kind(&self) -> EvidenceKind;

    /// Resolve `ev` (which belongs to `task`, loaded from `file`).
    fn resolve(
        &self,
        repo: &Repo,
        task: &Task,
        file: &Path,
        ev: &Evidence,
        out: &mut Vec<Diagnostic>,
    ) -> Result<(), Error>;
}

/// `commit` evidence: L1 (the SHA resolves, `APLAN011`) and L2 (its diff touches
/// one of the task's `paths`, `APLAN012`).
#[derive(Debug)]
pub struct CommitResolver;

impl EvidenceResolver for CommitResolver {
    fn kind(&self) -> EvidenceKind {
        EvidenceKind::Commit
    }

    fn resolve(
        &self,
        repo: &Repo,
        task: &Task,
        file: &Path,
        ev: &Evidence,
        out: &mut Vec<Diagnostic>,
    ) -> Result<(), Error> {
        // An immutable SHA is the only honest `commit` citation: a branch, tag,
        // `HEAD`, or `:/message` search resolves too, but points at different
        // commits over time and is controllable by a fork, so evidence built on
        // it is not the "resolved to a fact" this tool promises. Require a hex
        // object id (abbreviated to 7+, up to a full sha256).
        if !is_sha_shaped(&ev.reference) {
            out.push(
                Diagnostic::error(
                    code::COMMIT_UNRESOLVED,
                    format!(
                        "commit `{}` is not a SHA: cite an immutable commit id, not a branch, \
                         tag, or ref",
                        ev.reference
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Cite a commit SHA (7 or more hex characters) that exists in this repository.",
                ),
            );
            return Ok(());
        }

        let Some(commit) = repo.resolve_commit(&ev.reference) else {
            out.push(
                Diagnostic::error(
                    code::COMMIT_UNRESOLVED,
                    format!(
                        "commit `{}` does not resolve to a commit in this repository",
                        ev.reference
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Cite a commit SHA that exists in this repository, or remove the evidence.",
                ),
            );
            // Relevance is meaningless for a commit that does not exist.
            return Ok(());
        };

        // `is_sha_shaped` only checks the *shape*: a branch or tag named all-hex
        // (e.g. a tag `deadbeef`) is hex-shaped and `revparse_single` resolves it,
        // which would defeat the "immutable SHA, not a mutable ref" guarantee for
        // fork-controllable ref names. A real SHA (or prefix) is a prefix of the
        // resolved object id; a hex-named ref is not.
        if !commit
            .id()
            .to_string()
            .starts_with(&ev.reference.to_lowercase())
        {
            out.push(
                Diagnostic::error(
                    code::COMMIT_UNRESOLVED,
                    format!(
                        "commit `{}` is a ref name, not a commit id: cite the immutable SHA",
                        ev.reference
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Cite the commit's SHA (`git rev-parse <ref>`), not a branch or tag name.",
                ),
            );
            return Ok(());
        }

        // L2 needs `paths`; with none declared, relevance is unverifiable, that
        // is a gap in the task, not a violation of this rule (a future
        // `APLAN019`-style check can flag the empty `paths`).
        if task.paths.is_empty() {
            return Ok(());
        }

        // A catch-all glob (`**`, `*`, `**/*`) matches every file, so it cannot
        // bind a commit to the *specific* work a task owns. Because `paths` comes
        // from an untrusted task file, a match-everything claim must FAIL
        // relevance, not trivially satisfy it: otherwise any real commit could be
        // cited as evidence for a task that declares `paths: ['**']`.
        if let Some(wild) = task.paths.iter().find(|p| is_match_everything(p)) {
            out.push(
                Diagnostic::error(
                    code::COMMIT_IRRELEVANT,
                    format!(
                        "this task's `paths` include `{wild}`, which matches every file: a commit \
                         cannot be bound to the specific work this task owns"
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Replace the catch-all glob with the specific files this task owns.",
                ),
            );
            return Ok(());
        }

        let set = build_globset(&task.paths, &task.id)?;
        let touched = repo.commit_paths(&commit)?;
        if !touched.iter().any(|p| set.is_match(p)) {
            let short = short_sha(&ev.reference);
            out.push(
                Diagnostic::error(
                    code::COMMIT_IRRELEVANT,
                    format!(
                        "commit `{short}` touches none of this task's `paths` ({})",
                        task.paths.join(", ")
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Cite a commit that changes a file this task owns, or correct the task's \
                     `paths`.",
                ),
            );
        }
        Ok(())
    }
}

/// `file` evidence: L1 (the path exists at the cited rev, or at `HEAD`,
/// `APLAN015`). The reference is `path` or `path@rev`.
#[derive(Debug)]
pub struct FileResolver;

impl EvidenceResolver for FileResolver {
    fn kind(&self) -> EvidenceKind {
        EvidenceKind::File
    }

    fn resolve(
        &self,
        repo: &Repo,
        task: &Task,
        file: &Path,
        ev: &Evidence,
        out: &mut Vec<Diagnostic>,
    ) -> Result<(), Error> {
        let (path, rev) = split_rev(&ev.reference);
        if !repo.path_exists_at(rev, path)? {
            let at = rev.map_or_else(|| "HEAD".to_string(), |r| format!("`{r}`"));
            out.push(
                Diagnostic::error(
                    code::FILE_UNRESOLVED,
                    format!("file `{path}` does not exist at {at}"),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Point the `file` evidence at a path that exists at the cited rev (or at \
                     HEAD).",
                ),
            );
        }
        Ok(())
    }
}

/// `test` evidence: L1 (the runner lists a test with that name, `APLAN013`) and
/// L3 (under `--execute`, the runner runs it and it passes, `APLAN014`).
#[derive(Debug)]
pub struct TestResolver {
    runner: TestRunner,
    execute: bool,
}

impl TestResolver {
    pub fn new(runner: TestRunner, execute: bool) -> Self {
        Self { runner, execute }
    }

    /// The test names the runner reports. A trailing `: test` / `: benchmark`
    /// (cargo's `--list` format) is stripped so a bare name matches.
    fn list_tests(&self) -> Result<Vec<String>, Error> {
        let out = spawn(&self.runner.list)?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                let name = line
                    .strip_suffix(": test")
                    .or_else(|| line.strip_suffix(": benchmark"))
                    .unwrap_or(line);
                Some(name.trim().to_string())
            })
            .collect())
    }
}

impl EvidenceResolver for TestResolver {
    fn kind(&self) -> EvidenceKind {
        EvidenceKind::Test
    }

    fn resolve(
        &self,
        _repo: &Repo,
        task: &Task,
        file: &Path,
        ev: &Evidence,
        out: &mut Vec<Diagnostic>,
    ) -> Result<(), Error> {
        // L1: is a test with this name in the corpus the runner lists?
        if !self.list_tests()?.iter().any(|n| n == &ev.reference) {
            out.push(
                Diagnostic::error(
                    code::TEST_UNRESOLVED,
                    format!(
                        "test `{}` is not one the configured runner lists",
                        ev.reference
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Name the test exactly as `test.list` reports it, or fix the evidence.",
                ),
            );
            // A test that does not exist cannot be run.
            return Ok(());
        }

        // L3: under `--execute`, run it and check it passes.
        if self.execute
            && !spawn(&self.runner.run_argv(&ev.reference))?
                .status
                .success()
        {
            out.push(
                Diagnostic::error(
                    code::TEST_FAILED,
                    format!("test `{}` failed under `--execute`", ev.reference),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction("Make the cited test pass, or stop citing it as evidence."),
            );
        }
        Ok(())
    }
}

/// Spawn `argv` with no shell. This is the trust boundary: the test name is one
/// of these argv elements, never a shell string, so a task file cannot inject a
/// command through it (HLD §5.5).
fn spawn(argv: &[String]) -> Result<std::process::Output, Error> {
    let (program, rest) = argv
        .split_first()
        .ok_or_else(|| Error::Exec("empty command".to_string()))?;
    Command::new(program)
        .args(rest)
        .output()
        .map_err(|e| Error::Exec(format!("`{}`: {e}", argv.join(" "))))
}

/// Split a `file` reference into `(path, rev)`. `path@rev` yields a rev; a bare
/// path (or one whose `@` leaves either side empty) resolves at `HEAD`.
///
/// A rev is required to contain no `/`, so a path with an `@` segment (a scoped
/// npm package, `pkg/@scope/x.js`) is not misread as `path=pkg/`,
/// `rev=scope/x.js`.
fn split_rev(reference: &str) -> (&str, Option<&str>) {
    match reference.rsplit_once('@') {
        Some((p, r)) if !p.is_empty() && !r.is_empty() && !r.contains('/') => (p, Some(r)),
        _ => (reference, None),
    }
}

/// Whether `s` is shaped like a git object id: 7 to 64 hex characters (an
/// abbreviated SHA through a full sha256), and nothing else.
fn is_sha_shaped(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `glob` matches every file, a catch-all with no literal path segment
/// (`**`, `*`, `**/*`, `*/**`). Such a glob is not a real ownership claim.
fn is_match_everything(glob: &str) -> bool {
    let literal: String = glob
        .chars()
        .filter(|c| !matches!(c, '*' | '/') && !c.is_whitespace())
        .collect();
    literal.is_empty() && glob.contains('*')
}

/// The first 12 characters of a SHA, for a readable message. Falls back to the
/// whole reference when it is shorter (an abbreviated SHA or a ref name).
fn short_sha(reference: &str) -> &str {
    reference.get(..12).unwrap_or(reference)
}

fn build_globset(paths: &[String], task_id: &str) -> Result<globset::GlobSet, Error> {
    let mut builder = GlobSetBuilder::new();
    for p in paths {
        // `literal_separator(true)`: a single `*` matches within a path segment
        // and does not cross `/`, so `paths: ['src/*']` owns `src/a.rs` but not
        // `src/sub/deep.rs`. Crossing directories needs an explicit `**`, which
        // is the gitignore/ripgrep convention authors expect.
        let glob = GlobBuilder::new(p)
            .literal_separator(true)
            .build()
            .map_err(|_| Error::BadGlob(task_id.to_string(), p.clone()))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|_| Error::BadGlob(task_id.to_string(), paths.join(", ")))
}
