//! Evidence resolvers, one per kind. Adding a kind is a new `impl`, not a
//! change to the engine (HLD §5, the `EvidenceResolver` extension point).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;

use aplan_core::task::{Evidence, EvidenceKind, Status, Task, split_anchor, split_rev};
use aplan_core::{Diagnostic, code};
use globset::GlobSetBuilder;

use aplan_config::{TestCoverage, TestRunner};

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

    /// Judge the task's citations of this kind *together*, once every one has
    /// been resolved.
    ///
    /// Some findings are about a task's proof as a whole rather than any single
    /// citation: one focused commit among sprawling ones is proof (`APLAN024`),
    /// and a test covering the task's own code settles the question however many
    /// other components it also cites (`APLAN025`). Judging those per citation
    /// reports work that is properly evidenced, and makes the verdict depend on
    /// which citation happened to come first.
    fn finish_task(
        &self,
        _repo: &Repo,
        _task: &Task,
        _file: &Path,
        _out: &mut Vec<Diagnostic>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Report what is wrong with the *run* rather than with a task, once every
    /// task has been resolved (`A1-066`).
    ///
    /// A configured test runner that will not run is the case this exists for:
    /// it is nobody's task's fault, it must not be reported once per citation,
    /// and it used to abort the whole verification, which took every unrelated
    /// finding with it. A finding with no task attached is how the report says
    /// "this is about your configuration", while still being a finding rather
    /// than an exit code with no rule name on it.
    fn finish_plan(&self, _out: &mut Vec<Diagnostic>) {}

    /// What this resolver's configured commands cost, named (`A1-309`).
    ///
    /// Empty for a resolver that spawns nothing, which is most of them: the
    /// commit and file resolvers read git and never fork.
    ///
    /// The name is a **subject**, ready to be read in a sentence -- "the
    /// `aplan-core` test runner", "the `url.check` checker" -- because two
    /// kinds of command report here and the line that prints them should not
    /// have to know which is which (`A1-310`, audit: the caller called every
    /// one of them a test runner).
    fn spend(&self) -> Vec<(String, Cost)> {
        Vec::new()
    }
}

/// `commit` evidence: L1 (the SHA resolves, `APLAN011`) and L2 (its diff touches
/// one of the task's `paths`, `APLAN012`).
#[derive(Debug)]
pub struct CommitResolver {
    /// Paths a citation may touch outside the task's `paths` before `APLAN024`
    /// calls the task's proof diluted.
    max_unrelated_files: usize,
    /// Citations left unresolved because the clone is shallow, counted so the
    /// run says it once instead of blaming each task (`A1-177`).
    shallow_unresolved: std::cell::Cell<usize>,
    /// Citations that resolved but sit on the clone's boundary, so their diff
    /// cannot be computed and `APLAN012` must not be evaluated (`A1-177` audit).
    shallow_unjudged: std::cell::Cell<usize>,
}

impl CommitResolver {
    #[must_use]
    pub fn new(max_unrelated_files: Option<usize>) -> Self {
        Self {
            max_unrelated_files: max_unrelated_files.unwrap_or(DEFAULT_MAX_UNRELATED_FILES),
            shallow_unresolved: std::cell::Cell::new(0),
            shallow_unjudged: std::cell::Cell::new(0),
        }
    }
}

/// `citation` or `citations`.
fn plural(n: usize) -> &'static str {
    if n == 1 { "citation" } else { "citations" }
}

/// Files outside a task's `paths` that a cited commit may touch before the
/// citation is reported as mostly about other work.
///
/// Fifty because that is the scale the finding was measured at (a 124-file
/// scaffold, an 80-file split), and because this repository's own history
/// survives it: every citation here is a focused commit.
pub const DEFAULT_MAX_UNRELATED_FILES: usize = 50;

impl EvidenceResolver for CommitResolver {
    fn kind(&self) -> EvidenceKind {
        EvidenceKind::Commit
    }

    fn finish_plan(&self, out: &mut Vec<Diagnostic>) {
        self.report_shallow(out);
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
            // A shallow clone has the tip and nothing else, so git says "no such
            // object" about a citation that is perfectly correct in the full
            // history. Blaming the task here is how this told an adopter to
            // delete 164 correct citations (`A1-177`); the clone is reported
            // once instead, in `finish_plan`, and the citation is left
            // unresolved rather than wrong -- the shape `A1-066` gave a runner
            // that could not be asked.
            if repo.is_shallow() {
                self.shallow_unresolved
                    .set(self.shallow_unresolved.get() + 1);
                return Ok(());
            }
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

        // The clone stops here, so this commit's parent is absent and its diff
        // cannot be computed. `commit_paths` would diff it against the empty
        // tree -- the rule for a *root* commit -- which makes it touch every file
        // in the repository, so `APLAN012` would pass for any `paths` at all.
        // Left unjudged and counted, with the same one-per-run report as an
        // unresolvable citation (`A1-177`, found by its audit).
        if repo.is_truncated_at(&commit) {
            self.shallow_unjudged.set(self.shallow_unjudged.get() + 1);
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

        report_unbound_commit(repo, task, file, &commit, &ev.reference, out)
    }

    fn finish_task(
        &self,
        repo: &Repo,
        task: &Task,
        file: &Path,
        out: &mut Vec<Diagnostic>,
    ) -> Result<(), Error> {
        report_merge_evidence(repo, task, file, out);
        report_dilution(repo, task, file, self.max_unrelated_files, out)
    }
}

/// `APLAN012`'s second half: the citation resolves, but does its diff touch
/// anything this task owns?
///
/// Extracted from [`CommitResolver::resolve`] so that function stays inside the
/// length this workspace lints for; it is one question and reads better named.
///
/// A task whose `paths` will not compile is silent here: `APLAN032` has already
/// reported that from the hermetic side, and every path-based rule is
/// inapplicable until it is fixed (`A1-066`).
fn report_unbound_commit(
    repo: &Repo,
    task: &Task,
    file: &Path,
    commit: &git2::Commit<'_>,
    reference: &str,
    out: &mut Vec<Diagnostic>,
) -> Result<(), Error> {
    let Some(set) = build_globset(&task.paths) else {
        return Ok(());
    };
    let touched = repo.commit_paths(commit)?;
    if touched.iter().any(|p| set.is_match(p)) {
        return Ok(());
    }
    let short = short_sha(reference);
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
            "Cite a commit that changes a file this task owns, or correct the task's `paths`.",
        ),
    );
    Ok(())
}

/// `APLAN024`: the citations overlap, but are they *about* this task?
///
/// A per-task check, not a per-evidence one, because the finding is about the
/// task's proof as a whole: a task citing the commit that did the work and the
/// merge that carried it is evidenced, and warning about the second would be
/// noise. Running it here rather than inside [`CommitResolver`] also keeps it
/// clear of that resolver's guards, each of which returns early, so a bogus SHA
/// in the first slot cannot suppress the finding about the real citation behind
/// it.
///
/// Counted over the same path set `APLAN012` calls "touched", which includes
/// both sides of a rename, so a commit that moves a tree counts each file twice.
/// Consistency with the rule this one extends is worth more than a prettier
/// number, and the message says `path(s)` rather than `file(s)` so it does not
/// claim otherwise.
/// `APLAN035`: the task's `commit` evidence is a merge, and nothing else
/// (`A1-145`).
///
/// A merge's diff covers everything both sides touched, so a `paths` check
/// against one passes for work the task did not do. The first pilot closed a
/// task on the merge of an unrelated pull request that way, and every gate
/// agreed (pre-public adoption evidence, item 11).
///
/// **Judged over the citations together**, on `APLAN024`'s reasoning: one
/// commit that did the work is proof, and a merge listed beside it is context.
/// Warning about the extra citation would push an author to delete something
/// true, which is the noise that rule was reshaped to avoid (`A1-062`,
/// `A1-070`).
///
/// A *warning*: "almost never the commit that did the work" is not never, and a
/// repository that merges every feature with `--no-ff` has a merge that is the
/// honest citation.
///
pub fn report_merge_evidence(repo: &Repo, task: &Task, file: &Path, out: &mut Vec<Diagnostic>) {
    let mut merges: Vec<(String, usize)> = Vec::new();
    for ev in task
        .evidence
        .iter()
        .filter(|e| e.kind == EvidenceKind::Commit && !e.reference.trim().is_empty())
    {
        let Some(commit) = repo.resolve_commit(&ev.reference) else {
            // Unresolvable is `APLAN011`'s finding, not this rule's.
            continue;
        };
        if commit.parent_count() > 1 {
            merges.push((ev.reference.clone(), commit.parent_count()));
        } else {
            // A commit that did work settles it.
            return;
        }
    }
    for (reference, parents) in merges {
        out.push(
            Diagnostic::warning(
                code::MERGE_COMMIT_EVIDENCE,
                format!(
                    "commit `{}` has {parents} parents, so its diff covers everything both \
                     sides touched: a `paths` check against a merge passes for work this task \
                     did not do",
                    short_sha(&reference)
                ),
            )
            .with_task(&task.id)
            .with_file(file)
            .with_agent_instruction(
                "Cite the commit that made the change (`git log --no-merges -- <paths>` finds \
                 it). A merge is almost never that commit; if this repository merges every \
                 feature with `--no-ff` and the merge really is where the work landed, say so \
                 in the task body.",
            ),
        );
    }
}

pub fn report_dilution(
    repo: &Repo,
    task: &Task,
    file: &Path,
    max_unrelated_files: usize,
    out: &mut Vec<Diagnostic>,
) -> Result<(), Error> {
    // Only a `done` task is claiming its evidence proves something finished; on
    // anything else a sprawling citation is work in progress, not an overclaim.
    if max_unrelated_files == 0 || task.status != Status::Done || task.paths.is_empty() {
        return Ok(());
    }
    // A catch-all `paths` is `APLAN012`'s error, and it would make every commit
    // look perfectly focused here.
    if task.paths.iter().any(|p| is_match_everything(p)) {
        return Ok(());
    }
    // A cited test says what a diff cannot, so it settles the question.
    //
    // Counted structurally, at the rung `require_min_strength` counts kinds at:
    // the citation is not resolved here. Where a runner is configured `APLAN013`
    // independently rejects a name it cannot find, so the only way to quiet this
    // rule with a fictional test is to have opted out of test resolution
    // altogether, which is a louder decision than this warning.
    if task
        .evidence
        .iter()
        .any(|e| e.kind == EvidenceKind::Test && !e.reference.trim().is_empty())
    {
        return Ok(());
    }

    // Gathered before the globset is built, so a task whose citations this rule
    // never counts cannot turn a malformed glob into a hard error the other
    // rules would not have raised.
    let mut cited: Vec<(&str, Vec<PathBuf>)> = Vec::new();
    for e in task
        .evidence
        .iter()
        .filter(|e| e.kind == EvidenceKind::Commit)
    {
        if !is_sha_shaped(&e.reference) {
            continue;
        }
        let Some(commit) = repo.resolve_commit(&e.reference) else {
            continue;
        };
        // A hex-shaped ref name is `APLAN011`'s finding, not proof to weigh.
        if !commit
            .id()
            .to_string()
            .starts_with(&e.reference.to_lowercase())
        {
            continue;
        }
        cited.push((&e.reference, repo.commit_paths(&commit)?));
    }
    if cited.is_empty() {
        return Ok(());
    }

    let Some(set) = build_globset(&task.paths) else {
        return Ok(());
    };
    // The task's *best* citation: one focused commit is proof, and the rest are
    // context. Commits `APLAN012` has already rejected outright belong to that
    // rule and are not counted here.
    let mut best: Option<(&str, usize)> = None;
    let mut counted = 0usize;
    for (reference, touched) in &cited {
        let unrelated = touched.iter().filter(|p| !set.is_match(p)).count();
        if touched.len() == unrelated {
            continue; // Touches none of the `paths`: `APLAN012` owns this one.
        }
        counted += 1;
        if best.is_none_or(|(_, fewest)| unrelated < fewest) {
            best = Some((reference, unrelated));
        }
    }

    let Some((reference, unrelated)) = best.filter(|(_, u)| *u > max_unrelated_files) else {
        return Ok(());
    };
    let short = short_sha(reference);
    // Naming the commit the number came from: with several citations the count
    // is the *least* diluted one's, and attributing it to any other commit would
    // be a diagnostic stating something untrue about that commit.
    let lead = if counted == 1 {
        format!(
            "the only commit proving this task, `{short}`, touches {unrelated} path(s) outside its `paths`"
        )
    } else {
        format!(
            "every commit cited here is mostly other work: the closest, `{short}`, touches \
             {unrelated} path(s) outside this task's `paths`"
        )
    };
    out.push(
        Diagnostic::warning(
            code::DILUTED_EVIDENCE,
            format!(
                "{lead}: it overlaps them, so `APLAN012` passes, but a commit that size would \
                 overlap almost any `paths`"
            ),
        )
        .with_task(&task.id)
        .with_file(file)
        .with_agent_instruction(
            "Cite the commit that did this task's work, or add `test` evidence; if the work \
             really did arrive in one large commit, say so in the task body.",
        ),
    );
    Ok(())
}

/// The section a `file` citation names, checked in the file at that rev.
///
/// `None` when it resolves. Reuses `APLAN015`, and the message says which half
/// failed, because "the document is not there" and "that section is not in it"
/// send a reader to different places.
fn unresolved_anchor(
    repo: &Repo,
    rev: Option<&str>,
    // `path` is normalised and is what git is asked for; `written` is how the
    // citation spells it and is what every message names. A reader has to find
    // it in their own file, and naming a spelling they did not write is the
    // defect `A1-254` set out to remove, one step along (`A1-254`, audit).
    path: &str,
    written: &str,
    anchor: &str,
    task: &Task,
    file: &Path,
) -> Result<Option<Diagnostic>, Error> {
    let at = rev.map_or_else(|| "HEAD".to_string(), |r| format!("`{r}`"));
    if anchor.is_empty() {
        return Ok(Some(
            Diagnostic::error(
                code::FILE_UNRESOLVED,
                format!("file `{written}#` ends in an empty anchor"),
            )
            .with_task(&task.id)
            .with_file(file)
            .with_agent_instruction(format!(
                "Name the section after the `#`, or cite `{written}` on its own to mean the whole \
                 document."
            )),
        ));
    }
    let Some(text) = repo.text_at(rev, path)? else {
        // Tracked (the caller checked) and unreadable as text: a binary, or one
        // this build cannot decode. Nothing to look for a heading in, and the
        // citation should not pass on the strength of an anchor nobody read.
        return Ok(Some(
            Diagnostic::error(
                code::FILE_UNRESOLVED,
                format!("file `{written}` at {at} is not text, so `#{anchor}` cannot be resolved"),
            )
            .with_task(&task.id)
            .with_file(file)
            .with_agent_instruction(format!(
                "Cite `{written}` without an anchor, or point the citation at a text document whose \
                 sections can be named."
            )),
        ));
    };
    // The same reader `traces` uses, so a heading or a table row means here what
    // it means there, and a citation copied from a rendered URL resolves.
    if aplan_core::anchor::resolvable_anchors(&text).contains(anchor.to_lowercase().as_str()) {
        return Ok(None);
    }
    Ok(Some(
        Diagnostic::error(
            code::FILE_UNRESOLVED,
            format!("file `{written}` exists at {at}, and `#{anchor}` matches no heading or table row in it"),
        )
        .with_task(&task.id)
        .with_file(file)
        .with_agent_instruction(format!(
            "Correct the anchor to a section `{written}` has, or cite `{written}` on its own to mean \
             the whole document."
        )),
    ))
}

/// `APLAN015` for a citation that names something this repository cannot hold
/// (`A1-249`).
///
/// `file` evidence resolves against this repository's history: `path_exists_at`
/// asks git whether the path is a blob in a commit's tree. An absolute path, or
/// one that climbs out with `..`, is not a path in any tree here, so the answer
/// is always "no" -- and reporting that as "does not exist at HEAD" is false in
/// the plain reading, because the file the author cited is on their disk and
/// they can see it.
///
/// The remedy is not a spelling correction. Evidence that a deliverable landed
/// in another repository is a `url` naming it at a commit on its forge, or a
/// task in that repository's own plan; a `file` citation has one slot for a rev
/// and it is a rev of this repository (ADR-0028).
fn outside_this_repository(path: &str, task: &Task, file: &std::path::Path) -> Option<Diagnostic> {
    let normalised = aplan_core::anchor::normalise(std::path::Path::new(path));
    if !normalised.is_absolute() && !normalised.starts_with("..") {
        return None;
    }
    Some(
        Diagnostic::error(
            code::FILE_UNRESOLVED,
            format!("file `{path}` is outside this repository, so no commit here can contain it"),
        )
        .with_task(&task.id)
        .with_file(file)
        .with_agent_instruction(
            "A `file` citation resolves against this repository's history. For a deliverable \
             that landed elsewhere, cite `url` evidence naming it at a commit on its forge, or \
             track the work in that repository's own plan.",
        ),
    )
}

/// `file` evidence: L1 (the path exists at the cited rev, or at `HEAD`,
/// `APLAN015`). The reference is `path`, `path@rev`, `path#anchor`, or
/// `path#anchor@rev`.
///
/// **The anchor** is the half `A1-159` added, and it came from a repository
/// whose entire notion of proof is a section of a reviewed document: 54 of
/// `p0000019`'s 59 document references name one, all of them in a field that
/// project calls `evidence`. Without it a citation either loses which section is
/// the proof or fails outright, so every closed task in that plan read as
/// unproven while citing something real.
///
/// It resolves with the same reader `traces` uses (`crate::anchor`), so a
/// heading or a table row named here means what it means there, and it stays
/// `file` on the ladder: `require_min_strength` already counts `file` among the
/// strong kinds, and naming a section only narrows what the citation claims.
///
/// **Not fingerprinted**, unlike a `traces` anchor. A stamp would re-open a task
/// whenever its proof was edited, which is a churn cost no pilot has asked for,
/// and `traces` is where a document that must not drift already belongs.
#[derive(Debug)]
pub struct FileResolver;

impl CommitResolver {
    /// The one finding a shallow clone earns, in place of one per citation.
    fn report_shallow(&self, out: &mut Vec<Diagnostic>) {
        let unresolved = self.shallow_unresolved.get();
        let unjudged = self.shallow_unjudged.get();
        if unresolved + unjudged == 0 {
            return;
        }
        // Both halves in one sentence, because they are one cause and an adopter
        // fixes them with one flag.
        let boundary = |n: usize| {
            format!(
                "{} on the clone's boundary, where what a commit touches cannot be computed",
                if n == 1 { "sits" } else { "sit" }
            )
        };
        let what = match (unresolved, unjudged) {
            (n, 0) => format!("{n} `commit` {} could not be resolved", plural(n)),
            (0, m) => format!("{m} `commit` {} {}", plural(m), boundary(m)),
            (n, m) => format!(
                "{n} `commit` {} could not be resolved, and {m} {}",
                plural(n),
                boundary(m)
            ),
        };
        out.push(
            Diagnostic::error(
                code::COMMIT_UNRESOLVED,
                format!(
                    "this is a shallow clone: {what}. That is a fact about the checkout, not \
                     about the plan"
                ),
            )
            .with_agent_instruction(
                "Check out the full history and run this again: `fetch-depth: 0` on \
                 `actions/checkout`, or `git fetch --unshallow` locally. Do not remove the \
                 citations; nothing here says they are wrong.",
            ),
        );
    }
}

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
        // `@rev` first, then `#anchor`: a rev never contains a `#`, and this is
        // the order the reference is written in.
        let (reference, rev) = split_rev(&ev.reference);
        let (path, anchor) = split_anchor(reference);
        // A rev holding a `#` is the two parts written the other way round
        // (`path@rev#section`), and reporting a missing file at a rev nobody
        // could resolve sends the reader looking for the document instead of at
        // the order they typed (`A1-159`'s audit).
        if let Some(rev) = rev
            && rev.contains('#')
        {
            out.push(
                Diagnostic::error(
                    code::FILE_UNRESOLVED,
                    format!(
                        "`{}` names a rev containing `#`: a `file` citation is written \
                         `path#section@rev`, section first",
                        ev.reference
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Write the section before the rev: `path#section@rev`.".to_string(),
                ),
            );
            return Ok(());
        }
        // A path that leaves this repository can never be a path *in* it, and
        // saying "does not exist at HEAD" about a sibling file the author can see
        // on disk sends them to check a path that is fine (`A1-249`). Purely
        // lexical, because a git path is repository-relative by definition: there
        // is no rev here at which `/etc/passwd` or `../sibling/x` is a blob, and
        // nothing is read to find that out.
        if let Some(diagnostic) = outside_this_repository(path, task, file) {
            out.push(diagnostic);
            return Ok(());
        }
        // Git stores one spelling of a path, and an author writes whichever one
        // reads best: `./f.txt`, `docs/../docs/x.md`. A `traces` target has
        // always been normalised before it is resolved, so the two fields
        // disagreed about the same string and the message blamed a file that was
        // there (`A1-254`). Locators, not identities: `./f.txt` and `f.txt` name
        // one blob, and accepting both proves nothing extra.
        //
        // The *written* form stays in every message. A reader has to find the
        // citation in their own file, and a message naming a spelling they did
        // not use is the same defect one step along.
        let looked_up = aplan_core::anchor::normalise(std::path::Path::new(path));
        let path_for_git = looked_up.to_string_lossy();
        let path_for_git = path_for_git.as_ref();
        if !repo.path_exists_at(rev, path_for_git)? {
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
            // "It is not there" and "it is not yours" are different faults, and a
            // path that does not exist cannot be judged for relevance.
            return Ok(());
        }

        // The section, when one is named. One code for both halves, as
        // `APLAN008` already does for a `traces` target and its anchor: the
        // fault is the same one -- the citation does not resolve -- at a finer
        // grain, and a reader who sees `APLAN015` should not have to learn a
        // second number for it.
        if let Some(anchor) = anchor
            && let Some(diagnostic) =
                unresolved_anchor(repo, rev, path_for_git, path, anchor, task, file)?
        {
            out.push(diagnostic);
            return Ok(());
        }

        // L2, the half `APLAN012` had and this did not: a file that exists and
        // lies outside the task's `paths` proves the repository has that file
        // (`A1-045`).
        //
        // With no `paths` there is nothing to judge against, which is
        // `APLAN023`'s finding and not this one's: reporting both would give one
        // gap two codes.
        if task.paths.is_empty() {
            return Ok(());
        }
        // The same reading `APLAN012` takes of a catch-all: `paths` comes from an
        // untrusted task file, so a claim to own every file must fail relevance
        // rather than satisfy it trivially. Said once per task there, and once
        // here, because a task may cite either kind.
        if let Some(wild) = task.paths.iter().find(|p| is_match_everything(p)) {
            out.push(
                Diagnostic::error(
                    code::FILE_IRRELEVANT,
                    format!(
                        "this task's `paths` include `{wild}`, which matches every file: a cited \
                         file cannot be bound to the specific work this task owns"
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

        let Some(set) = build_globset(&task.paths) else {
            return Ok(());
        };
        if !set.is_match(path_for_git) {
            out.push(
                Diagnostic::error(
                    code::FILE_IRRELEVANT,
                    format!(
                        "file `{path}` is outside this task's `paths` ({}): it exists, which is \
                         all the citation proves",
                        task.paths.join(", ")
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Cite a file this task owns, or correct the task's `paths` if they do not \
                     name all of it. This cannot tell which of the two is wrong.",
                ),
            );
        }
        Ok(())
    }
}

/// How long `test.list` may take before the runner is reported as unaskable
/// (`A1-121`).
///
/// Ten minutes, because `cargo test --list` compiles: on a cold CI cache that is
/// legitimately minutes, and a limit that fires on a slow-but-working runner
/// would be a worse defect than the hang it prevents. `test.timeout_seconds`
/// overrides it, and `0` removes it.
const LIST_TIMEOUT_SECS: u64 = 600;

/// The name the miss probe runs (`A1-167`). Long, so a runner that matches its
/// argument as a substring cannot select a real test with it, and fixed, so two
/// runs of `verify` on one tree do the same thing.
pub(crate) const MISS_SENTINEL: &str = "aplan-probe-a-name-no-test-can-have-8f3c1d";

/// How long `url.check` may take per citation (`A1-121`).
///
/// Thirty seconds, because this is one network request. Two orders of magnitude
/// below the runner's limit on purpose: a black-holed packet does not refuse, it
/// goes quiet, so the hang is this key's *ordinary* failure rather than an
/// exotic one. `url.timeout_seconds` overrides it, and `0` removes it.
const URL_TIMEOUT_SECS: u64 = 30;

/// `test` evidence: L1 (the runner lists a test with that name, `APLAN013`) and
/// L3 (under `--execute`, the runner runs it and it passes, `APLAN014`).
#[derive(Debug)]
pub struct TestResolver {
    runners: Vec<TestRunner>,
    repo_root: PathBuf,
    execute: bool,
    /// What each runner listed, computed once.
    ///
    /// Without this, every runner is asked for every citation: a plan with 20
    /// `test` citations against 5 runners spawns 100 processes, and a `list`
    /// command is often `cargo test --list`, which *compiles*. That turns
    /// `verify` from seconds into minutes exactly in the repositories this
    /// feature exists for. The corpus cannot change during a run, so asking
    /// twice was never buying anything.
    listings: RefCell<Vec<Option<Vec<String>>>>,
    /// What went wrong with a runner, once per runner (`A1-066`).
    ///
    /// A runner that will not run used to abort the whole verification: a `dir`
    /// that does not exist took every other task's findings with it and exited
    /// `2`. The failure still has to be loud -- `A1-120` made it an abort
    /// precisely so a broken runner could not quietly invalidate the citations
    /// that depend on it -- so it is now an error *finding* reported once, and
    /// the citations that would have used this runner are skipped rather than
    /// reported as names it does not list.
    faults: RefCell<Vec<Option<RunnerFault>>>,
    /// Which failing tests have already shown their output in this run
    /// (`A1-365`), keyed the way `executed` is: by runner and name.
    shown: RefCell<std::collections::BTreeSet<(usize, String)>>,
    /// What a *miss* looks like for each runner, learned once from a sentinel
    /// and only when a citation has passed under `--execute` (`A1-167`).
    misses: RefCell<Vec<Option<MissProfile>>>,
    /// Per runner: how many passing executions were indistinguishable from a
    /// miss, and the first of them, so the finding names one.
    unproven: RefCell<Vec<(usize, Option<String>)>>,
    /// What each distinct test concluded, for the life of this run (`A1-243`).
    ///
    /// One runner and one exact listed name select one execution, so a second
    /// citation of the same test learns nothing by running it again. Measured
    /// on `p003`: 86 `test` citations over nine distinct names took about 55
    /// minutes, against about three for the nine suites run once, because the
    /// cost grew with citation reuse rather than with the executable set --
    /// which punishes exactly the evidence model this tool is for.
    ///
    /// Keyed by runner and name, and never shared beyond this resolver, which
    /// lives for one `verify` invocation.
    executed: RefCell<std::collections::BTreeMap<(usize, String), Outcome>>,
    /// Per runner: what its commands cost, merged across every one this run
    /// spawned -- peaks kept, wall clock added (`A1-309`).
    costs: RefCell<Vec<Cost>>,
    /// Per runner: the tests it says it will not run, asked once (`A1-300`).
    ///
    /// `None` where the runner declares no `skipped` command, which is every
    /// runner that has not opted in, and where the command could not be asked.
    skips: RefCell<Vec<Option<Vec<String>>>>,
    /// Where `.aplan.yml` is, for the findings that are about it.
    config_file: Option<PathBuf>,
}

/// The names a listing command printed, one per line, with cargo's trailing
/// `: test` / `: benchmark` stripped.
///
/// Shared by `test.list` and `test.skipped`, because the second answers in the
/// first's names and the comparison is between the two. Parsing it a second way
/// went wrong immediately: splitting a line on `:` truncated
/// `scale::tests::a_name` to `scale`, so an ignored test anywhere but the top of
/// a target was never recognised and the gate it feeds stayed silent (`A1-300`,
/// audit).
fn listed_names(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
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
        .collect()
}

/// What running a name that cannot exist looks like, for one runner (`A1-167`).
///
/// `--execute` reads a zero exit status as "the cited test passed". That is only
/// worth what the runner makes it worth: `vitest run -t` matches a test's own
/// title while `vitest list` prints `file > suite > test`, so a citation of the
/// listed name selects nothing, every test is skipped, and the status is `0`.
/// Exit status alone cannot separate that from a pass -- `cargo test -- --exact
/// <no such test>` exits `0` too, and that is what a correctly configured cargo
/// runner does.
///
/// So the runner is asked what a miss looks like, and judged against its own
/// answer rather than against anything this tool knows about any runner.
/// Measured, on the three the pilots use: `pytest` 9.1.1 exits `4`, and is
/// settled by `by_status` alone. `cargo` exits `0` but prints `running 0 tests`
/// where a hit prints `running 1 test`, so its output settles it. `vitest`
/// 4.1.11 exits `0` and prints the same bytes either way but for a clock and a
/// duration, which is what the digit mask is for.
#[derive(Debug, Clone)]
struct MissProfile {
    /// The runner said "nothing matched" with a non-zero status, so a zero
    /// status means something ran and no comparison is needed.
    by_status: bool,
    /// A miss's output with every run of digits replaced, so a clock and a
    /// duration cannot read as a difference.
    masked: Vec<String>,
}

/// A runner that could not do its job, and which job (`A1-066`).
///
/// The distinction earns its keep: a runner that could not be *asked what it
/// lists* leaves every citation's L1 unknown, so no citation may be reported as
/// unlisted. A runner that listed fine and then could not *run* a test under
/// `--execute` has told us everything L1 needs, and suppressing L1 findings for
/// the rest of the plan on the strength of an L3 failure would hide real ones.
#[derive(Debug, Clone)]
struct RunnerFault {
    /// Whether the fault means this runner's listing is unknown.
    listing_unknown: bool,
    /// The clause the finding reads, after "the `<runner>` test runner ".
    what: String,
}

impl TestResolver {
    #[must_use]
    pub fn new(
        runners: Vec<TestRunner>,
        repo_root: PathBuf,
        execute: bool,
        config_file: Option<PathBuf>,
    ) -> Self {
        let listings = RefCell::new(vec![None; runners.len()]);
        let faults = RefCell::new(vec![None; runners.len()]);
        let misses = RefCell::new(vec![None; runners.len()]);
        let unproven = RefCell::new(vec![(0, None); runners.len()]);
        let executed = RefCell::new(std::collections::BTreeMap::new());
        let skips = RefCell::new(vec![None; runners.len()]);
        let costs = RefCell::new(vec![Cost::default(); runners.len()]);
        let shown = RefCell::new(std::collections::BTreeSet::new());
        Self {
            runners,
            repo_root,
            execute,
            listings,
            faults,
            shown,
            misses,
            unproven,
            executed,
            costs,
            skips,
            config_file,
        }
    }

    /// How a runner is named in a diagnostic: its `name`, else its `dir`, else
    /// its position, so an ambiguity is always attributable to something the
    /// author can find in their config.
    fn label(&self, index: usize) -> String {
        let r = &self.runners[index];
        r.name
            .clone()
            .or_else(|| r.dir.clone())
            .unwrap_or_else(|| format!("runner {}", index + 1))
    }

    /// The test names one runner reports, from the cache or by asking it once.
    ///
    /// `None` when the runner could not be asked. The reason is cached with the
    /// listing, so a run spawns each `list` command at most once whether it
    /// works or not, and [`Self::finish_plan`] reports it exactly once
    /// (`A1-066`).
    fn list_tests(&self, index: usize) -> Option<Vec<String>> {
        if let Some(cached) = self.listings.borrow()[index].as_ref() {
            return Some(cached.clone());
        }
        if self.faults.borrow()[index]
            .as_ref()
            .is_some_and(|f| f.listing_unknown)
        {
            return None;
        }
        match self.list_tests_uncached(index) {
            Ok(names) => {
                self.listings.borrow_mut()[index] = Some(names.clone());
                Some(names)
            }
            Err(why) => {
                self.fault(
                    index,
                    true,
                    format!(
                        "could not be asked what it lists, so no `test` citation it would have \
                         resolved could be checked: {why}"
                    ),
                );
                None
            }
        }
    }

    /// What this runner declared, in the units a cgroup takes (`A1-310`).
    ///
    /// A size that will not parse cannot reach here: `TestRunner::validate`
    /// refuses it when the file loads, which is where a value the adopter wrote
    /// belongs.
    fn caps_of(&self, index: usize) -> Caps {
        let runner = &self.runners[index];
        Caps {
            memory_bytes: runner
                .max_memory
                .as_deref()
                .and_then(|s| aplan_config::parse_size(s).ok()),
            max_processes: runner.max_processes,
        }
    }

    /// `wrap` prepended, if the runner declared one (`A1-310`).
    ///
    /// Outside the command and inside the cage, so a declared `systemd-run` or
    /// `nice` is itself subject to whatever the dials set.
    fn wrapped(&self, index: usize, argv: &[String]) -> Vec<String> {
        match &self.runners[index].wrap {
            Some(prefix) => prefix.iter().chain(argv).cloned().collect(),
            None => argv.to_vec(),
        }
    }

    /// Run one citation under `--execute`, or say nothing ran it (`A1-310`).
    ///
    /// `Ok(None)` means the command was stopped at a declared cap and the
    /// finding for that is already reported: the caller has nothing left to
    /// judge. Extracted from `resolve` so that function stays inside the length
    /// this workspace lints for.
    fn execute_citation(
        &self,
        runner: usize,
        ev: &Evidence,
        task: &Task,
        file: &Path,
        out: &mut Vec<Diagnostic>,
    ) -> Executed {
        if !self.execute {
            return Executed::NotAsked;
        }
        // Run it, or reuse what this run already learned about it (`A1-243`).
        // `None` is a command that could not be launched: reported once against
        // the runner and deliberately not remembered, because a launch that
        // fails beside one that succeeded is a transient -- a fork under
        // pressure -- rather than a fact about the configuration.
        let Some(outcome) = self.outcome_of(runner, &ev.reference) else {
            return Executed::NotAsked;
        };
        // Every *citation* is judged, even where only one execution happened:
        // the finding is about the citations, and `APLAN037` counts them.
        match outcome {
            Outcome::Stopped(breach) => {
                out.push(self.stopped(runner, breach, ev, task, file));
                Executed::Stopped
            }
            Outcome::Failed(shown) => {
                // Attached to the first citation of this test and referred to
                // afterwards (`A1-365`, criterion 5): one failing test cited by
                // many tasks must not print its output once per citation --
                // measured on one adopter's plan, 86 citations over nine names.
                // Two distinct failing tests stay distinguishable because the
                // excerpt is keyed by runner and name.
                Executed::Failed(self.first_sighting(runner, &ev.reference, shown))
            }
            Outcome::Passed { proves_nothing } => {
                if proves_nothing {
                    self.note_unproven(runner, &ev.reference);
                }
                Executed::Passed
            }
        }
    }

    /// What this test concluded, running it only if this run has not already.
    fn outcome_of(&self, runner: usize, name: &str) -> Option<Outcome> {
        let key = (runner, name.to_string());
        if let Some(hit) = self.executed.borrow().get(&key) {
            return Some(hit.clone());
        }
        let (outcome, settled) = self.execute_once(runner, name)?;
        // Only a verdict this run can stand behind is kept. The probe that
        // decides `APLAN037` is deliberately retried rather than cached when it
        // cannot be run, so remembering a judgement made in its absence would
        // outlive the reason for it: measured, a sentinel that failed once left
        // every later citation of that name judged by a probe that never
        // completed, and a real false green went unreported where it had been
        // caught before (`A1-243`, audit).
        if settled {
            self.executed.borrow_mut().insert(key, outcome.clone());
        }
        Some(outcome)
    }

    /// Spawn `test.run` for one name, once.
    ///
    /// The second half of the pair says whether the verdict is **settled** --
    /// whether this run learned enough to stand behind it for every other
    /// citation of the name. Everything is, except a pass judged while the
    /// miss-probe could not be run (`A1-243`, audit).
    fn execute_once(&self, runner: usize, name: &str) -> Option<(Outcome, bool)> {
        let spawned = spawn_capped(
            &self.wrapped(runner, &self.runners[runner].run_argv(name)),
            &self.dir_of(&self.runners[runner]),
            "test.run",
            // Unbounded on purpose: under `--execute` a long run is the
            // point rather than a symptom (`A1-121`, and the reasoning is
            // on `test.timeout_seconds` where an adopter reads it).
            std::time::Duration::ZERO,
            self.caps_of(runner),
        );
        if let Ok((_, cost)) = &spawned {
            self.record_cost(runner, *cost);
            // A command stopped at its cap neither passed nor failed, and
            // saying "failed" is the misreading `A1-300` exists to prevent.
            if let Some(breach) = cost.breach {
                // A fact about the limit and the test rather than about this
                // moment: running it again spends the same to learn the same.
                return Some((Outcome::Stopped(breach), true));
            }
        }
        match spawned.map(|(ran, _)| ran) {
            Ok(Ran::Exited(out)) if out.status.success() => {
                let judged = self.proves_nothing(runner, &out);
                Some((
                    Outcome::Passed {
                        proves_nothing: judged.unwrap_or(false),
                    },
                    judged.is_some(),
                ))
            }
            Ok(Ran::Exited(out)) => {
                // The one place the streams still exist. They were captured and
                // then dropped here, which is the whole of the defect
                // (`A1-365`).
                let bound = crate::excerpt::Bound {
                    lines: self.runners[runner]
                        .output_lines
                        .unwrap_or(crate::excerpt::DEFAULT_LINES),
                    ..crate::excerpt::Bound::default()
                };
                Some((
                    Outcome::Failed(crate::excerpt::of(&out.stdout, &out.stderr, bound)),
                    true,
                ))
            }
            Ok(Ran::TimedOut(_)) => unreachable!("no limit was set"),
            Err(why) => {
                self.fault(
                    runner,
                    false,
                    format!(
                        "could not run the tests cited against it under `--execute`, so \
                             they are unverified rather than passing: {why}"
                    ),
                );
                None
            }
        }
    }

    /// The first citation of `name` in this run, and what it may show
    /// (`A1-365`).
    ///
    /// One failing test cited by many tasks printed its output once per
    /// citation, which on one adopter's plan would be 86 copies over nine
    /// distinct names. The excerpt goes to whichever citation is judged first
    /// and the rest point at it, so the bound is on the *run* rather than on
    /// each finding.
    ///
    /// Every finding still names the test, so two distinct failing tests remain
    /// distinguishable whatever order they are read in.
    fn first_sighting(&self, runner: usize, name: &str, shown: Option<String>) -> Shown {
        let key = (runner, name.to_string());
        if self.shown.borrow().contains(&key) {
            return Shown::Already;
        }
        self.shown.borrow_mut().insert(key);
        match shown {
            Some(text) => Shown::Excerpt(text),
            None => Shown::Nothing,
        }
    }

    /// `APLAN014` for a test that ran and failed (`A1-365`).
    ///
    /// Extracted because the message now has three shapes and the caller is at
    /// the length this workspace lints for: what the test printed, a pointer to
    /// where it was printed, and the bare statement for a runner that printed
    /// nothing or was told to show nothing.
    fn failed(ev: &Evidence, task: &Task, file: &Path, shown: Shown) -> Diagnostic {
        let name = &ev.reference;
        let mut detail = Vec::new();
        let (message, instruction) = match shown {
            Shown::Excerpt(text) => {
                // The lines go in `detail`, not into the message: a message is
                // prose every surface folds to its own width, and reflowing an
                // assertion would rearrange the thing being read (`A1-365`).
                detail = text.lines().map(str::to_string).collect();
                (
                    format!("test `{name}` failed under `--execute`, and printed:"),
                    format!(
                        "Read the output beneath the finding: it is the tail of what the \
                         runner printed, bounded and redacted. Then make `{name}` pass, or \
                         stop citing it as evidence. `test.output_lines` widens it, and `0` \
                         suppresses it for a repository whose tests print things it would \
                         rather not publish."
                    ),
                )
            }
            Shown::Already => (
                format!("test `{name}` failed under `--execute`"),
                format!(
                    "Another finding for `{name}` in this run carries what it printed; it ran \
                     once and is reported against every citation. Make it pass, or stop citing \
                     it as evidence."
                ),
            ),
            Shown::Nothing => (
                format!("test `{name}` failed under `--execute` and printed nothing"),
                format!(
                    "Nothing was captured, so the runner is silent on failure or \
                     `test.output_lines: 0` is set. Run `{name}` through the configured \
                     command to see why, then make it pass or stop citing it as evidence."
                ),
            ),
        };
        Diagnostic::error(code::TEST_FAILED, message)
            .with_task(&task.id)
            .with_file(file)
            .with_agent_instruction(instruction)
            .with_detail(detail)
    }

    /// The finding for a command the kernel stopped at its cap (`A1-310`).
    ///
    /// Extracted so `resolve` stays inside the length this workspace lints for,
    /// and because it is one thought: a citation whose command was stopped is
    /// neither proved nor disproved, and the fix is the limit or the test.
    fn stopped(
        &self,
        runner: usize,
        breach: Breach,
        ev: &Evidence,
        task: &Task,
        file: &Path,
    ) -> Diagnostic {
        Diagnostic::error(
            code::TEST_FAILED,
            format!(
                "test `{}` was stopped under `--execute`: the command {}, so it neither passed \
                 nor failed",
                ev.reference,
                self.breach_clause(runner, breach)
            ),
        )
        .with_task(&task.id)
        .with_file(file)
        .with_agent_instruction(
            "Raise this runner's limit in `.aplan.yml` if the command needs the room, or make \
             the test cost less. The citation is neither proved nor disproved until the command \
             runs inside it.",
        )
    }

    /// A runner stopped while it was still listing its tests (`A1-310`).
    ///
    /// Nobody's task: a listing that was stopped is a fault of the
    /// configuration, and lands where a listing that could not be asked already
    /// lands (ADR-0033). Separate from the `Err` it returns so the fault and the
    /// error are one thought, and so a test can read both without a kernel.
    fn stopped_while_listing(&self, index: usize, breach: Breach) -> Error {
        self.fault(
            index,
            true,
            format!(
                "was stopped while listing its tests: it {}, so no `test` citation it would \
                 have resolved could be checked",
                self.breach_clause(index, breach)
            ),
        );
        Error::Exec(format!(
            "`test.list` for {}: stopped at its declared limit",
            self.label(index)
        ))
    }

    /// Say what a stopped command means, for the finding that reports it.
    fn breach_clause(&self, index: usize, breach: Breach) -> String {
        let runner = &self.runners[index];
        match breach {
            Breach::Memory => format!(
                "exceeded this runner's `max_memory` ({})",
                runner.max_memory.as_deref().unwrap_or("declared")
            ),
            Breach::Processes => format!(
                "exceeded this runner's `max_processes` ({})",
                runner
                    .max_processes
                    .map_or_else(|| "declared".into(), |n| n.to_string())
            ),
        }
    }

    /// Fold one command's cost into its runner's running total (`A1-309`).
    ///
    /// Peaks are kept rather than added: two commands that each reached 8 GB,
    /// one after the other, never held 16 GB between them, and a report saying
    /// so would send an adopter to size a limit at twice what they need. Wall
    /// clock is added, because the run really did spend both.
    fn record_cost(&self, index: usize, c: Cost) {
        self.costs.borrow_mut()[index].fold(c);
    }

    /// Record what went wrong with a runner, keeping the first reason: a second
    /// failure of the same runner is the same fault seen again.
    fn fault(&self, index: usize, listing_unknown: bool, what: String) {
        let mut faults = self.faults.borrow_mut();
        if faults[index].is_none() {
            faults[index] = Some(RunnerFault {
                listing_unknown,
                what,
            });
        }
    }

    /// How many runners could not say what they list, which is the fault that
    /// makes "no runner lists this" an unproven claim.
    fn unlisted_count(&self) -> usize {
        self.faults
            .borrow()
            .iter()
            .filter(|f| f.as_ref().is_some_and(|f| f.listing_unknown))
            .count()
    }

    /// Ask one runner what it lists. A trailing `: test` / `: benchmark`
    /// (cargo's `--list` format) is stripped so a bare name matches.
    fn list_tests_uncached(&self, index: usize) -> Result<Vec<String>, Error> {
        let runner = &self.runners[index];
        let limit =
            std::time::Duration::from_secs(runner.timeout_seconds.unwrap_or(LIST_TIMEOUT_SECS));
        let (ran, cost) = spawn_capped(
            &self.wrapped(index, &runner.list),
            &self.dir_of(runner),
            "test.list",
            limit,
            self.caps_of(index),
        )?;
        self.record_cost(index, cost);
        if let Some(breach) = cost.breach {
            return Err(self.stopped_while_listing(index, breach));
        }
        let out = match ran {
            Ran::Exited(out) => out,
            // Nothing was learned, which is exactly what the listing failure
            // above means, so it takes the same route (`A1-121`).
            Ran::TimedOut(after) => {
                return Err(Error::Exec(format!(
                    "`test.list` for {}: `{}` had not returned after {}s and was killed",
                    self.label(index),
                    one_line(&runner.list.join(" ")),
                    after.as_secs()
                )));
            }
        };
        // A runner that could not answer must not read as a runner that listed
        // nothing. Ignoring the exit status meant a package that failed to
        // compile reported every citation against it as "not one the configured
        // runner lists", which is the wrong reason and sends the reader to fix
        // the citation. `A1-120` made it likely rather than theoretical: eight
        // runners, so any one of them breaking silently invalidates its
        // citations. A failure here is a configuration or environment fault, so
        // it is a finding about the *configuration*, reported once and loudly,
        // rather than a reason to stop and say nothing about the plan
        // (`A1-066`).
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let detail = stderr.lines().next().unwrap_or("no output").trim();
            return Err(Error::Exec(format!(
                "`test.list` for {}: `{}` exited {} ({detail})",
                self.label(index),
                runner.list.join(" "),
                out.status
                    .code()
                    .map_or_else(|| "by signal".into(), |c| c.to_string()),
            )));
        }
        Ok(listed_names(&out.stdout))
    }

    /// Where a runner runs: its `dir` under the repository root, or the root.
    fn dir_of(&self, runner: &TestRunner) -> PathBuf {
        match &runner.dir {
            Some(d) => self.repo_root.join(d),
            None => self.repo_root.clone(),
        }
    }

    /// `APLAN025`: the runners list these tests, but do they cover this task?
    ///
    /// Judged over the task's citations together, on `APLAN024`'s reasoning: one
    /// test covering the code the task owns is proof, and a cross-component
    /// integration test alongside it is context. `p00025`'s `WH2-13` is the case
    /// that settled this, citing two tests in the component it changed plus an
    /// `xtask` test that parses the spec's own examples through that component;
    /// warning there would have been warning about a repository doing the right
    /// thing, and with `deny_warnings` set it would have failed the build for it.
    ///
    /// Applies whatever the task's `status`, like `APLAN012` and unlike
    /// `APLAN024`: the question is whether the citation identifies the right code
    /// at all, not how strong a completion claim is.
    fn report_foreign_tests(&self, task: &Task, file: &Path, out: &mut Vec<Diagnostic>) {
        // Relevance needs `paths`; with none declared this is `APLAN023`'s
        // finding, and reporting it twice would not add anything.
        if task.paths.is_empty() {
            return;
        }
        let mut foreign: Vec<(String, String)> = Vec::new();
        for ev in task
            .evidence
            .iter()
            .filter(|e| e.kind == EvidenceKind::Test && !e.reference.trim().is_empty())
        {
            let listing = self.runners_listing(&ev.reference);
            let [only] = listing.as_slice() else {
                // Listed by none or by several: `APLAN013` owns both, and
                // neither says anything about locality.
                continue;
            };
            let claimed = covered_prefixes(&self.runners[*only], &ev.reference);
            if claimed.is_empty() {
                // A runner claiming nothing cannot demonstrate a mismatch, and
                // this rule reports only what it can demonstrate.
                return;
            }
            if task
                .paths
                .iter()
                .any(|p| claimed.iter().any(|c| reaches_dir(c, p)))
            {
                // The task cites a test covering its own code. That settles it.
                return;
            }
            foreign.push((ev.reference.clone(), self.claim_of(*only, &ev.reference)));
        }
        if foreign.is_empty() {
            return;
        }
        let paths = task.paths.join(", ");
        let message = if let [(test, claim)] = foreign.as_slice() {
            // Observational, like the multi-citation form beside it. It used to
            // end "so the citation borrows proof from another part of the
            // repository", which is one explanation of no overlap and not the
            // one that turned out to be true: the third pilot's citations were
            // correct and its `paths` had lost an entry, so the asserted cause
            // and the real one had opposite remedies. `paths` is the least
            // verified input in the model and every rule that reads it treats it
            // as ground truth, so a rule triangulating against it may state what
            // it saw and not why (`A1-070`, pre-public adoption evidence).
            format!(
                "test `{test}` is listed by a runner that {claim}, which this task's `paths` \
                 ({paths}) do not reach"
            )
        } else {
            let each: Vec<String> = foreign
                .iter()
                .map(|(t, claim)| format!("`{t}` from a runner that {claim}"))
                .collect();
            format!(
                "none of the {} tests cited here covers this task's `paths` ({paths}): {}",
                foreign.len(),
                each.join(", ")
            )
        };
        out.push(
            Diagnostic::warning(code::TEST_IRRELEVANT, message)
                .with_task(&task.id)
                .with_file(file)
                // Symmetric, like `APLAN012`'s "or correct the task's `paths`".
                // It used to read "or widen `paths` if the work really does span
                // that component", which frames editing `paths` as an
                // accommodation for work that genuinely reaches elsewhere. The
                // case that produced this rule's finding was the other one: the
                // `paths` had simply lost an entry, so an agent applying that
                // condition would have concluded the work does not span and gone
                // to change the citation, which was right. The message above
                // stopped asserting a cause; the line an agent acts on has to
                // stop implying one (`A1-070`).
                .with_agent_instruction(
                    "Cite a test that covers the code this task owns, or correct `paths` if they \
                     do not name all of it. This cannot tell which of the two is wrong.",
                ),
        );
    }

    /// How a runner describes what it covers, named the way the config was
    /// written so the author looks for the key they actually used.
    fn claim_of(&self, index: usize, test: &str) -> String {
        let runner = &self.runners[index];
        let label = self.label(index);
        if let Some(entry) = matching_coverage(runner, test) {
            return format!(
                "is called `{label}` and covers `{}` for tests matching `{}`",
                entry.paths.join("`, `"),
                entry.tests.join("`, `")
            );
        }
        match &runner.covers {
            Some(c) if !c.is_empty() => {
                format!("is called `{label}` and covers `{}`", c.join("`, `"))
            }
            _ => format!(
                "is called `{label}` and runs in `{}`",
                covered_prefixes(runner, test).join("`, `")
            ),
        }
    }

    /// The listed test a citation most likely meant, if any.
    ///
    /// The pilot's mistake was `guider::name` where the runner lists the test
    /// bare: `cargo test -- --list` prints unqualified names, but you *run* an
    /// integration test with the target prefix, so the wrong form is the one an
    /// author reaches for (pre-public adoption evidence, item 10). That is a path
    /// mismatch rather than a typo, so edit distance alone would not have found
    /// it; both directions of `::` qualification are tried first, and a
    /// single-edit typo after that.
    fn near_match(&self, name: &str) -> Option<String> {
        let bare = name.rsplit("::").next().unwrap_or(name);
        for i in 0..self.runners.len() {
            // A runner that could not be asked has no near match to offer, and
            // its silence is reported on its own. `continue`, not `?`: one
            // unaskable runner must not cost the suggestion a working one has
            // (`A1-066`, found by its audit).
            let Some(listed) = self.list_tests(i) else {
                continue;
            };
            // The citation is qualified and the runner lists it bare.
            if let Some(hit) = listed.iter().find(|n| *n == bare)
                && hit != name
            {
                return Some(hit.clone());
            }
            // The reverse: the runner qualifies and the citation does not.
            if let Some(hit) = listed
                .iter()
                .find(|n| n.rsplit("::").next().unwrap_or(n) == name)
            {
                return Some(hit.clone());
            }
            if let Some(hit) = listed
                .iter()
                .find(|n| aplan_core::is_one_edit_apart(n, name))
            {
                return Some(hit.clone());
            }
        }
        None
    }

    /// The tests one runner says it will not run, asked once (`A1-300`).
    ///
    /// Asked the way `list` is asked, so this tool learns nothing about any
    /// particular harness: the repository names the command. A runner that
    /// declares none has not opted in and gets `None`, which is silence rather
    /// than a claim.
    ///
    /// A command that cannot be asked is a fault about the *configuration*, the
    /// same as a `list` that fails, but it is deliberately **not** recorded as
    /// one that invalidates citations: a missing skip list costs a check the
    /// repository asked for, and reporting every citation against the runner as
    /// unverified would be a larger failure than the one it came from.
    fn skipped_tests(&self, index: usize) -> Option<Vec<String>> {
        if let Some(cached) = self.skips.borrow()[index].as_ref() {
            return Some(cached.clone());
        }
        let runner = &self.runners[index];
        let argv = runner.skipped.as_ref()?;
        // Asked once, whatever the answer was. A failure records a fault and no
        // cached list, so without this the broken command is spawned again for
        // every task that cites a test -- 265 times on this repository's own
        // plan, for one fault that is already reported. A runner that has failed
        // at anything is not asked a further question; the run is failing loudly
        // either way.
        if self.faults.borrow()[index].is_some() {
            return None;
        }
        let limit =
            std::time::Duration::from_secs(runner.timeout_seconds.unwrap_or(LIST_TIMEOUT_SECS));
        // A runner that could not be asked must not read as a runner that skips
        // nothing, which is what returning `None` quietly would mean: the key is
        // there to catch a citation nothing ran, and a broken command would
        // switch that off while the plan went green. Same channel as a `list`
        // that fails -- one finding about the configuration, on `.aplan.yml`.
        let spawned = spawn_capped(
            &self.wrapped(index, argv),
            &self.dir_of(runner),
            "test.skipped",
            limit,
            self.caps_of(index),
        );
        if let Ok((_, cost)) = &spawned {
            self.record_cost(index, *cost);
        }
        let what = match spawned.map(|(ran, _)| ran) {
            Ok(Ran::Exited(out)) if out.status.success() => {
                let names = listed_names(&out.stdout);
                self.skips.borrow_mut()[index] = Some(names.clone());
                return Some(names);
            }
            Ok(Ran::Exited(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let detail = stderr.lines().next().unwrap_or("no output").trim();
                format!(
                    "could not be asked which tests it will not run: `{}` exited {} ({detail})",
                    one_line(&argv.join(" ")),
                    out.status
                        .code()
                        .map_or_else(|| "by signal".into(), |c| c.to_string()),
                )
            }
            Ok(Ran::TimedOut(after)) => format!(
                "could not be asked which tests it will not run: `{}` had not returned after {}s \
                 and was killed",
                one_line(&argv.join(" ")),
                after.as_secs()
            ),
            Err(why) => format!("could not be asked which tests it will not run: {why}"),
        };
        self.fault(index, false, what);
        None
    }

    /// Whether every `test` citation on this task names something its runner
    /// will not run (`A1-300`).
    ///
    /// **Every one, not any.** Citing a nightly-gated test beside a test that
    /// runs is an honest thing to do -- the claim still has executed proof, and
    /// the gated citation says where the rest of it lives. What is not honest is
    /// a task whose every test is one nothing ran, which is what `--execute` was
    /// reporting green.
    ///
    /// **Tests only, and the finding says so.** A task may also cite a commit,
    /// and that commit still resolves; what it does not do is execute. The rung
    /// `--execute` claims is the one nothing here reached, so the message is
    /// about the tests this task cites rather than about its proof as a whole.
    ///
    /// **Whatever the status**, like the rest of `APLAN014`: a citation that
    /// cannot be executed is as broken on a `todo` task as on a closed one, and
    /// `--execute` runs a cited test whatever the status of the task citing it.
    fn all_cited_tests_are_skipped(&self, task: &Task) -> Vec<String> {
        let cited: Vec<&str> = task
            .evidence
            .iter()
            .filter(|e| e.kind == EvidenceKind::Test)
            .map(|e| e.reference.as_str())
            .collect();
        if cited.is_empty() {
            return Vec::new();
        }
        let mut skipped = Vec::new();
        for name in cited {
            let listed = self.runners_listing(name);
            // A citation no runner lists is `APLAN013`'s, and a citation whose
            // runner declares no skip list is one this cannot speak about.
            // Either way it is not *known* to be skipped, so the task has a
            // citation this rule cannot call unrun, and says nothing.
            if !listed.iter().any(|i| {
                self.skipped_tests(*i)
                    .is_some_and(|s| s.iter().any(|n| n == name))
            }) {
                return Vec::new();
            }
            skipped.push(name.to_string());
        }
        skipped
    }

    /// Which runners list this name. A runner that could not be asked lists
    /// nothing here and is reported separately, so a citation it might have
    /// resolved is never reported as one nothing lists without that caveat.
    ///
    /// Every runner is asked, rather than stopping at the first hit, because
    /// two runners reporting the same name is the case that must not be resolved
    /// by accident of ordering. (That paragraph was sitting on `near_match`,
    /// which it does not describe, until `A1-300` moved it back.)
    fn runners_listing(&self, name: &str) -> Vec<usize> {
        let mut found = Vec::new();
        for i in 0..self.runners.len() {
            if self
                .list_tests(i)
                .is_some_and(|listed| listed.iter().any(|n| n == name))
            {
                found.push(i);
            }
        }
        found
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
        // L1: does any configured runner list a test with this name?
        let listing = self.runners_listing(&ev.reference);
        let runner = match listing.as_slice() {
            [] => {
                let which = if self.runners.len() > 1 {
                    format!("any of the {} configured runners list", self.runners.len())
                } else {
                    "the configured runner lists".to_string()
                };
                // A near match is the whole difference between this being a typo
                // and being a dead end: the pilot spent the time because nothing
                // said what the runner *did* list (`A1-109`).
                let near = self.near_match(&ev.reference);
                // A runner that could not be asked may well list this name, so
                // "no runner lists it" is not a fact this run established. The
                // runner's own finding is the error; adding one per citation
                // would report the same fault N times and send the reader to
                // fix citations that may be perfectly correct (`A1-066`).
                if self.unlisted_count() > 0 {
                    return Ok(());
                }
                let message = match &near {
                    Some(hit) => {
                        format!("test `{}` is not one {which}, but `{hit}` is", ev.reference)
                    }
                    None => format!("test `{}` is not one {which}", ev.reference),
                };
                let instruction = match &near {
                    Some(hit) => format!("Cite `{hit}`, which is how `test.list` reports it."),
                    None => "Name the test exactly as `test.list` reports it, or fix the evidence."
                        .to_string(),
                };
                out.push(
                    Diagnostic::error(code::TEST_UNRESOLVED, message)
                        .with_task(&task.id)
                        .with_file(file)
                        .with_agent_instruction(instruction),
                );
                // A test that does not exist cannot be run.
                return Ok(());
            }
            [only] => *only,
            // Two runners reporting the same name means the citation does not
            // identify a test. Picking one would make the verdict depend on the
            // order of a config file, which is the kind of quiet guess this tool
            // exists to refuse, so it fails closed and names both.
            many => {
                let names: Vec<String> = many.iter().map(|i| self.label(*i)).collect();
                out.push(
                    Diagnostic::error(
                        code::TEST_UNRESOLVED,
                        format!(
                            "test `{}` is listed by more than one runner ({}), so the citation \
                             does not say which one",
                            ev.reference,
                            names.join(", ")
                        ),
                    )
                    .with_task(&task.id)
                    .with_file(file)
                    .with_agent_instruction(
                        "Make the cited name unique, or narrow a runner's `list` so only one \
                         reports it.",
                    ),
                );
                return Ok(());
            }
        };

        // L3: under `--execute`, run it with the runner that listed it.
        //
        // A non-zero exit is read as the test failing, and unlike `test.list`
        // that is sound rather than lucky: this runner already listed the name,
        // so its package built. What it cannot distinguish is a *malformed*
        // `run` template, which exits non-zero for a reason that has nothing to
        // do with the test. Cargo returns 101 for both, so telling them apart
        // would mean parsing stderr, and a heuristic here would report a
        // configuration fault as a failing test with more confidence than it
        // deserves. Recorded rather than guessed at (`A1-120`, audit).
        // A `run` that cannot be *launched* -- the directory went away, the
        // program is not there -- is a fault of the runner rather than a failing
        // test, and reporting it as `APLAN014` would say the cited test failed
        // when nothing ran it. Recorded and reported once, like the listing
        // failure above, instead of ending the run (`A1-066`).
        let shown = match self.execute_citation(runner, ev, task, file, out) {
            // Stopped at its cap: the finding is already pushed, and a citation
            // whose command never ran is neither proved nor disproved.
            Executed::Stopped | Executed::NotAsked | Executed::Passed => return Ok(()),
            Executed::Failed(shown) => shown,
        };
        out.push(Self::failed(ev, task, file, shown));
        Ok(())
    }

    fn finish_task(
        &self,
        _repo: &Repo,
        task: &Task,
        file: &Path,
        out: &mut Vec<Diagnostic>,
    ) -> Result<(), Error> {
        self.report_foreign_tests(task, file, out);
        // `--execute` only: without it nothing claimed the test ran, so there is
        // no false green to report. The citation still resolves, and that is
        // what L2 says.
        if self.execute {
            for name in self.all_cited_tests_are_skipped(task) {
                out.push(
                    Diagnostic::error(
                        code::TEST_FAILED,
                        format!(
                            "test `{name}` was not run under `--execute`: the runner lists it \
                             among the tests it skips, and no test this task cites was run"
                        ),
                    )
                    .with_task(&task.id)
                    .with_file(file)
                    .with_agent_instruction(
                        "Cite a test this runner actually runs as well, so the claim has proof \
                         the gate executes. The skipped citation may stay: it says where the \
                         rest of the proof lives.",
                    ),
                );
            }
        }
        Ok(())
    }

    /// One finding per runner that could not be asked, attached to no task
    /// because no task is at fault (`A1-066`).
    fn spend(&self) -> Vec<(String, Cost)> {
        self.costs
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("the `{}` test runner", self.label(i)), *c))
            .collect()
    }

    fn finish_plan(&self, out: &mut Vec<Diagnostic>) {
        // Before the fault loop rather than after. No test can tell the
        // difference today, and that is a property of `runners_listing` asking
        // every runner rather than of this: were it ever to stop, this would be
        // the first thing to ask a runner the citations never needed, and a
        // `list` that failed here would record a fault the loop above had
        // already passed -- a runner failing silently, which is the exact shape
        // of the defect `APLAN036` exists to end. Free to order correctly now.
        self.report_empty_listings(out);
        self.report_unproven_runs(out);
        for (i, fault) in self.faults.borrow().iter().enumerate() {
            let Some(fault) = fault else { continue };
            let mut d = Diagnostic::error(
                code::TEST_UNRESOLVED,
                format!("the `{}` test runner {}", self.label(i), fault.what),
            )
            .with_agent_instruction(
                "Fix the runner in `.aplan.yml`: check that its `dir` exists and that its \
                 commands run there. Until they do, every citation that depends on it is \
                 unverified rather than wrong.",
            );
            // The declaration's own file, so the finding has a location like
            // every other one, on `APLAN029`'s reasoning: an annotation lands on
            // `.aplan.yml` in the diff rather than being a locationless sentence.
            if let Some(path) = &self.config_file {
                d = d.with_file(path);
            }
            out.push(d);
        }
    }
}

impl TestResolver {
    /// Whether this passing execution proved anything.
    ///
    /// Only a *passing* run can be a false green, which is why this is not asked
    /// of a failing one: a run that selected nothing and failed is a spurious
    /// failure, already reported as `APLAN014` and already sending the author to
    /// look at the run.
    /// `None` when the probe that decides it could not be run, which is not the
    /// same as `Some(false)`: one says nothing was learned, the other claims the
    /// run proved something. They were one value before, and the caller now
    /// caches on the difference (`A1-243`, audit).
    fn proves_nothing(&self, runner: usize, out: &std::process::Output) -> Option<bool> {
        // The repository's own claim, taken at its word and costing nothing: no
        // sentinel is spawned for a runner that has declared it selects exactly.
        if self.runners[runner].run_selects == Some(true) {
            return Some(false);
        }
        let profile = self.miss_profile(runner)?;
        if profile.by_status {
            return Some(false);
        }
        Some(masked_lines(out) == profile.masked)
    }

    /// Count one citation whose pass proved nothing (`APLAN037`).
    ///
    /// Per citation, not per execution, and that separation is the point:
    /// `A1-243` runs one command for a name however many tasks cite it, while
    /// the finding is about the citations and says how many there are. Counting
    /// executions instead would report "(1 citation)" for twelve of them.
    fn note_unproven(&self, runner: usize, name: &str) {
        let mut unproven = self.unproven.borrow_mut();
        unproven[runner].0 += 1;
        if unproven[runner].1.is_none() {
            unproven[runner].1 = Some(name.to_string());
        }
    }

    /// Ask this runner what a miss looks like, once.
    ///
    /// Once, and compared after masking digits, because the obvious alternative
    /// does not survive contact: running the sentinel twice and treating any
    /// line the two disagree about as noise was implemented, measured, and
    /// abandoned. Two back-to-back vitest runs printed the *same* `Start at`
    /// second, so the clock line was never learned as noise, and the check then
    /// fired or stayed silent depending on whether a second had ticked between
    /// two processes. A rule whose verdict depends on a clock is worse than no
    /// rule: `attest` exists on the promise that two runs at one commit agree.
    ///
    /// A digit mask is the same idea made deterministic. What it costs is a
    /// runner whose only difference between a hit and a miss is a *number*: this
    /// cannot tell `1 passed` from `0 passed`, and says so rather than guessing,
    /// which is `run_selects`. Every runner the pilots use differs in words too
    /// (`running 1 test` against `running 0 tests`).
    fn miss_profile(&self, runner: usize) -> Option<MissProfile> {
        if let Some(cached) = self.misses.borrow()[runner].as_ref() {
            return Some(cached.clone());
        }
        let miss = self.run_sentinel(runner)?;
        let profile = MissProfile {
            by_status: !miss.status.success(),
            masked: masked_lines(&miss),
        };
        self.misses.borrow_mut()[runner] = Some(profile.clone());
        Some(profile)
    }

    /// Run this runner against a name no test can have.
    ///
    /// Long and unmistakable on purpose: a runner may match its argument as a
    /// substring (`vitest run -t` does), so a short sentinel could select a real
    /// test and the probe would learn what a *hit* looks like.
    fn run_sentinel(&self, runner: usize) -> Option<std::process::Output> {
        let spawned = spawn_capped(
            &self.wrapped(runner, &self.runners[runner].run_argv(MISS_SENTINEL)),
            &self.dir_of(&self.runners[runner]),
            "test.run",
            std::time::Duration::ZERO,
            self.caps_of(runner),
        );
        if let Ok((_, cost)) = &spawned {
            self.record_cost(runner, *cost);
            // A probe the kernel stopped never ran, so it describes nothing. It
            // took the same route as a launch failure before this, except that
            // it also handed back a killed process's empty output as "what a
            // miss looks like" (`A1-310`, audit).
            if cost.breach.is_some() {
                return None;
            }
        }
        match spawned.map(|(ran, _)| ran) {
            Ok(Ran::Exited(out)) => Some(out),
            // A probe that cannot run says nothing about the runner. The `run`
            // command's own failure is reported where a citation needed it, and
            // this is deliberately not cached: a launch that fails while the
            // cited run beside it succeeded is a transient (a fork under
            // pressure), not a fact about the configuration, so the next
            // citation may as well ask again.
            Ok(Ran::TimedOut(_)) | Err(_) => None,
        }
    }

    /// `APLAN037`: this runner's passing runs are indistinguishable from misses.
    fn report_unproven_runs(&self, out: &mut Vec<Diagnostic>) {
        for index in 0..self.runners.len() {
            let (count, example) = self.unproven.borrow()[index].clone();
            let Some(example) = example else { continue };
            let also = match count {
                1 => String::new(),
                n => format!(" ({n} citations)"),
            };
            let mut d = Diagnostic::warning(
                code::RUN_PROVES_NOTHING,
                format!(
                    "the `{}` test runner ran test `{example}`{also} and produced exactly \
                     what it produces for a name that does not exist, so `--execute` did \
                     not prove the test ran",
                    self.label(index)
                ),
            )
            .with_agent_instruction(
                "The names this runner's `test.list` prints are not the names its \
                 `test.run` selects. Run one of them by hand and check that the test \
                 really runs; then change `list` to print what `run` selects, or `run` \
                 to select what `list` prints. If the runner is right and simply says \
                 nothing either way, declare `run_selects: true` on it.",
            );
            if let Some(path) = &self.config_file {
                d = d.with_file(path);
            }
            out.push(d);
        }
    }

    /// `APLAN036`: a configured runner that answered, and named nothing.
    ///
    /// Only in a run that was already resolving `test` evidence, which costs
    /// that run nothing: resolving one `test` citation already asks *every*
    /// runner, because finding which runner lists a name means asking all of
    /// them, so by here the answers are cached. The guard runs the other way --
    /// a plan that cites only commits performs no listings, and this returns
    /// without asking rather than compiling the workspace to check something
    /// that plan has no stake in. It also keeps the finding scoped to the case
    /// that matters: five runners, citations against three, and the blind one
    /// is whichever component nobody has cited.
    fn report_empty_listings(&self, out: &mut Vec<Diagnostic>) {
        if self.listings.borrow().iter().all(Option::is_none) {
            return;
        }
        for index in 0..self.runners.len() {
            // Declared inert: a component that genuinely has no tests yet.
            if self.runners[index].expect_tests == Some(false) {
                continue;
            }
            // A runner that could not be asked is the fault above, not this.
            let Some(names) = self.list_tests(index) else {
                continue;
            };
            if !names.is_empty() {
                continue;
            }
            let mut d = Diagnostic::warning(
                code::RUNNER_LISTS_NOTHING,
                format!(
                    "the `{}` test runner lists no tests, so every citation against it will \
                     fail as a name nothing lists",
                    self.label(index)
                ),
            )
            .with_agent_instruction(
                "Check the runner in `.aplan.yml`: its `dir`, and that its `list` command \
                 prints test names when run there by hand. If the component genuinely has no \
                 tests yet, say so with `expect_tests: false` rather than deleting the runner.",
            );
            if let Some(path) = &self.config_file {
                d = d.with_file(path);
            }
            out.push(d);
        }
    }
}

/// What one spawned command cost: how many processes it reached, how much memory
/// they held together, and how long it took (`A1-309`).
///
/// **Summed RSS, and the report says so.** Adding each process's `VmRSS`
/// overcounts memory shared between them: measured on this project's own
/// harness, a cargo run reached 30.5 GB summed against 5.1 GB proportional, a
/// factor of six, while a vitest run reached 33.0 GB against 25.6 GB, a factor
/// of one and a third. The proportional figure is truer and costs an order of
/// magnitude more to collect -- `smaps_rollup` is walked per mapping, per
/// process, per sample -- so this takes the cheap one and names it, because an
/// adopter sizing a limit needs to know which number they hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cost {
    /// Most processes seen alive in the command's process group at once.
    pub peak_processes: usize,
    /// Largest sum of those processes' `VmRSS`, in kibibytes.
    pub peak_rss_kb: u64,
    /// Wall clock, start to exit.
    pub elapsed: std::time::Duration,
    /// What the command did to its caps, read from the cgroup rather than
    /// guessed from an exit status (`A1-310`).
    pub breach: Option<Breach>,
    /// Set when caps were declared and could not be applied (`A1-310`).
    pub unenforced: Option<Unenforced>,
    /// Whether the sampler could read anything at all. `false` on a platform
    /// with no `/proc`, which reports its absence rather than a zero: nought
    /// processes and nought bytes is a measurement, and it would be a lie.
    pub measured: bool,
}

impl Cost {
    /// Fold one command's cost into a running total for its subject (`A1-309`).
    ///
    /// Peaks are kept rather than added: two commands that each reached 8 GB,
    /// one after the other, never held 16 GB between them. Time is summed,
    /// because it was really spent twice. Everything that says something went
    /// wrong is kept once it has been said.
    ///
    /// One copy, because there are two callers: the second was written by hand
    /// and left `breach` out, so a stopped `test.run` reported no breach to
    /// anyone reading `verify_measured` (`A1-310`, audit).
    fn fold(&mut self, c: Self) {
        self.peak_processes = self.peak_processes.max(c.peak_processes);
        self.peak_rss_kb = self.peak_rss_kb.max(c.peak_rss_kb);
        self.elapsed += c.elapsed;
        self.measured |= c.measured;
        self.unenforced = self.unenforced.or(c.unenforced);
        self.breach = self.breach.or(c.breach);
    }
}

/// What one execution of one test concluded, kept for the run (`A1-243`).
///
/// Distinct from [`Executed`], which is what happened to one *citation*: a
/// dozen citations of one name share one of these and each get their own.
// `Clone` without `Copy`: `Failed` now carries the excerpt, which is a `String`
// (`A1-365`). The memoised copy is read once per citation of the name, which is
// a handful of clones of a bounded few lines.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// It passed. `proves_nothing` when the output was indistinguishable from
    /// what this runner produces for a name that does not exist (`APLAN037`).
    Passed { proves_nothing: bool },
    /// It ran and failed, which is a failure for every citation naming it.
    ///
    /// Carries what it printed, bounded and redacted (`A1-365`). The pilot's
    /// finding was that this variant used to carry nothing, so a failure that
    /// only happened inside a hosted runner could not be diagnosed from the CI
    /// record at all. `None` where the runner suppressed it with
    /// `output_lines: 0`, or where it printed nothing.
    Failed(Option<String>),
    /// The kernel stopped it at a declared cap (`A1-310`).
    Stopped(Breach),
}

/// What a failed citation may show of the output (`A1-365`).
enum Shown {
    /// The bounded, redacted tail of what the command printed.
    Excerpt(String),
    /// Another finding in this run carries it: the test ran once, and its
    /// output is reported once however many tasks cite it.
    Already,
    /// The command printed nothing, or the runner was told to show nothing.
    Nothing,
}

/// What `--execute` did with one citation (`A1-310`).
enum Executed {
    /// `--execute` was not asked for, so nothing was run and nothing is claimed.
    NotAsked,
    /// It ran and passed.
    Passed,
    /// It ran and failed, with what to show for it (`A1-365`).
    ///
    /// `Shown::Excerpt` on the first citation of this test in the run, and
    /// `Shown::Already` on the rest, so the output appears once however many
    /// tasks cite it.
    Failed(Shown),
    /// It was stopped at a declared cap. The finding is already reported, and
    /// the citation is neither proved nor disproved.
    Stopped,
}

/// The caps one runner declares, in the units a cgroup takes (`A1-310`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Caps {
    /// `memory.max` and `memory.swap.max`, in bytes.
    memory_bytes: Option<u64>,
    /// `pids.max`.
    max_processes: Option<u32>,
}

impl Caps {
    /// Whether anything was actually declared.
    fn any(self) -> bool {
        self.memory_bytes.is_some() || self.max_processes.is_some()
    }
}

/// Why a declared cap is not in force (`A1-310`, ADR-0033).
///
/// The two cases need different advice, so they are different values: a Mac has
/// no remedy, and a container has one its operator can apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unenforced {
    /// No cgroup v2 on this platform at all -- macOS, Windows, cgroup v1.
    NoCgroups,
    /// cgroup v2 is mounted, and this process may not create a child group:
    /// the usual cause is a container that was not given a delegated subtree.
    NotDelegated,
}

/// What a command did to its caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breach {
    /// The kernel killed it for exceeding `memory.max`: `memory.events` counted
    /// an `oom_kill`.
    Memory,
    /// `fork()` was refused for exceeding `pids.max`: `pids.events` counted a
    /// `max`. **Not a kill** -- the command carries on and exits however it
    /// likes, which is why this is read rather than inferred from the status.
    Processes,
}

impl Breach {
    /// The config key this breach is of, without the block that declares it.
    ///
    /// One mapping, because there are two callers and a third would have been
    /// the second chance to write `max_memory` where `max_processes` belonged:
    /// the key names what the adopter has to change, and a message naming the
    /// wrong one sends them to a limit that was never reached.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Breach::Memory => "max_memory",
            Breach::Processes => "max_processes",
        }
    }
}

/// One cgroup, created for one command and removed with it.
///
/// Plain file writes, which is what survives `unsafe_code = "forbid"`: no
/// `setrlimit` through `pre_exec`, no Job Object through FFI.
struct Cage {
    dir: std::path::PathBuf,
}

impl Cage {
    /// Make a cgroup for this command under a root already chosen, or say why
    /// not.
    ///
    /// Split out because every line of it is an ordinary file write, and that
    /// is what makes the cage testable where no cgroup exists: handed a plain
    /// directory it does exactly what it does under `/sys/fs/cgroup`, and a
    /// test can then read back the knobs it wrote. That matters here rather
    /// than being a nicety -- this project's CI runs in a rootless container
    /// with `/sys/fs/cgroup` mounted read-only, so a test that insisted on a
    /// real cgroup would be a test that never ran on the machine that gates
    /// the code (`A1-310`).
    fn build_in(root: &std::path::Path, caps: Caps) -> Result<Self, Unenforced> {
        let dir = root.join(format!("aplan-{}-{}", std::process::id(), unique()));
        std::fs::create_dir_all(&dir).map_err(|_| Unenforced::NotDelegated)?;
        let cage = Self { dir };
        if let Some(bytes) = caps.memory_bytes {
            // Both files, and this is the whole point: prototyped, a process
            // under a 64MB `memory.max` alone reached 3GB with `oom_kill` still
            // 0, because the pages went to swap (`A1-308`, ADR-0033).
            cage.write("memory.max", &bytes.to_string())?;
            cage.write("memory.swap.max", "0")?;
        }
        if let Some(n) = caps.max_processes {
            cage.write("pids.max", &n.to_string())?;
        }
        // Proved writable here rather than discovered by the command. The
        // joiner exits 125 when it cannot enrol itself, and under `--execute`
        // that reads as a cited test that failed -- the exact misreading
        // `A1-300` exists to prevent, for a fault that is the platform's rather
        // than the test's. Opened and not written: the write is a pid, and this
        // process does not belong in the cage (`A1-310`, audit).
        cgroup_knob_is_writable(&cage.dir.join("cgroup.procs"))?;
        Ok(cage)
    }

    fn write(&self, file: &str, value: &str) -> Result<(), Unenforced> {
        write_cgroup_knob(&self.dir.join(file), value)
    }

    /// What the kernel counted, read after the command exited.
    ///
    /// Read, never inferred: a memory breach is a SIGKILL and looks like one,
    /// but a pids breach only makes `fork()` fail, and the command then exits
    /// with its own error -- indistinguishable from an ordinary failure from
    /// outside (`A1-308`, audit).
    fn breach(&self) -> Option<Breach> {
        let counted = |file: &str, key: &str| -> bool {
            std::fs::read_to_string(self.dir.join(file))
                .ok()
                .and_then(|text| {
                    text.lines()
                        .find_map(|l| l.strip_prefix(key))
                        .and_then(|n| n.trim().parse::<u64>().ok())
                })
                .is_some_and(|n| n > 0)
        };
        if counted("memory.events", "oom_kill") {
            return Some(Breach::Memory);
        }
        if counted("pids.events", "max") {
            return Some(Breach::Processes);
        }
        None
    }

    /// The shell that puts the command in this cgroup before it becomes the
    /// command.
    ///
    /// `pre_exec` would be the direct way and needs `unsafe`, which this
    /// workspace forbids. Moving the pid *after* spawning would leave a race:
    /// anything the child forked in between would stay outside the cap, which
    /// is exactly the runaway a cap exists to stop. The child joins the group
    /// itself, then `exec`s, so nothing it starts is ever outside -- the same
    /// `sh -c` route `kill_group` already takes for the same reason.
    fn joiner(&self, argv: &[String]) -> Vec<String> {
        let mut out = vec![
            "sh".to_string(),
            "-c".to_string(),
            // `sh -c SCRIPT NAME ARGS...` binds NAME to `$0` and starts the
            // arguments at `$1`, so the path travels as `$0` and `"$@"` is
            // already exactly the command: nothing is spliced into the script
            // text, and nothing needs quoting. Interpolating the path instead
            // would put a directory name the kernel chose inside a shell
            // script, and a cgroup name may contain a quote.
            "echo $$ > \"$0\" || exit 125; exec \"$@\"".to_string(),
            self.dir.join("cgroup.procs").display().to_string(),
        ];
        out.extend(argv.iter().cloned());
        out
    }

    /// Kill everything still in the group, whatever it did to escape.
    ///
    /// This is the half a process-group signal cannot do. `kill_group` signals
    /// the group the child leads, and anything that called `setsid` has left
    /// that group -- measured: a `test.list` that timed out left a `setsid`
    /// child alive, holding the cgroup open so it could not be removed, and
    /// still holding the memory the cap existed to bound (`A1-310`, audit).
    /// Membership of a cgroup is the one thing a process cannot leave on its
    /// own, which is why the kernel offers this.
    ///
    /// Best effort: `cgroup.kill` arrived in Linux 5.14, and an older kernel
    /// simply has no such file.
    fn kill(&self) {
        let _ = write_cgroup_knob(&self.dir.join("cgroup.kill"), "1");
    }
}

impl Drop for Cage {
    fn drop(&mut self) {
        // Usually empty by now: the command and everything it started have
        // exited, or `kill_group` has seen to them. `kill` covers what neither
        // did, and only then is the directory removable -- a non-empty cgroup
        // cannot be removed, so without this a timeout left the group behind
        // with a live process in it.
        self.kill();
        // `cgroup.kill` delivers the signal and returns; it does not wait for
        // the processes to die, and a cgroup is not removable until the last
        // one has gone. The first attempt succeeds in the ordinary case, where
        // the command exited on its own and there was nothing to kill; the
        // retries are for the timeout that had something to kill, which is the
        // case that otherwise left one directory per run under this user's
        // slice (`A1-310`, audit). Bounded, because an unremovable cgroup is
        // not worth a hang: it is an empty directory the next run does not use.
        for _ in 0..50 {
            if std::fs::remove_dir(&self.dir).is_ok() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

/// Set one cgroup control file (`A1-310`).
///
/// A plain `fs::write`, and it has to be: these are kernel pseudo-files, where
/// the write *is* the operation and a compare-lock-rename would be meaningless
/// -- there is nothing to rename onto, and the kernel rejects a partial write
/// rather than tearing. `every_plan_write_goes_through_save` exempts it by name
/// for that reason; it never touches a plan file.
fn write_cgroup_knob(path: &std::path::Path, value: &str) -> Result<(), Unenforced> {
    std::fs::write(path, value).map_err(|_| Unenforced::NotDelegated)
}

/// Whether one cgroup control file would take a write, without making one.
///
/// For `cgroup.procs`, whose write is a pid this process must not send: putting
/// `aplan` in the cage is exactly what the joiner exists to avoid. Proved here
/// rather than discovered by the command, because the joiner exits 125 when it
/// cannot enrol itself, and under `--execute` that reads as a cited test that
/// failed -- the misreading `A1-300` exists to prevent, for a fault that is the
/// platform's (`A1-310`, audit).
///
/// `create` because the same call has to work against an ordinary directory,
/// which is what makes the cage testable where no cgroup exists; the kernel
/// ignores it for a file that is already there, checked against a real cgroup
/// rather than assumed. Exempt from `every_plan_write_goes_through_save` beside
/// the write it guards, and for the same reason: it never touches a plan file.
fn cgroup_knob_is_writable(path: &std::path::Path) -> Result<(), Unenforced> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map(drop)
        .map_err(|_| Unenforced::NotDelegated)
}

/// Where this process may create a cgroup that actually has controllers.
///
/// **Not this process's own cgroup**, which is the mistake that made the first
/// version report every ordinary desktop session as undelegated. A child group
/// only gets `memory.max` and `pids.max` if its *parent* lists those controllers
/// in `cgroup.subtree_control`, and a leaf scope -- a login session, a tmux
/// pane, a systemd service -- has that empty, because a cgroup may not both
/// hold processes and delegate controllers to children. Creating a group there
/// succeeds and yields a directory with no knobs in it.
///
/// So the search walks up to the nearest ancestor that does delegate, which for
/// a systemd user session is `user@<uid>.service` and inside a delegated
/// container is the container's own root (`A1-310`).
fn cgroup_root() -> Result<std::path::PathBuf, Unenforced> {
    let own = std::fs::read_to_string("/proc/self/cgroup").map_err(|_| Unenforced::NoCgroups)?;
    root_under(&own, std::path::Path::new("/sys/fs/cgroup"))
}

/// [`cgroup_root`], against a given `/proc/self/cgroup` and mount point.
///
/// The two answers are different findings and so are told apart here rather
/// than folded into one "no": a machine with no cgroup v2 has no remedy, and a
/// container with no delegated subtree has one its operator can apply
/// (ADR-0033). Taking both inputs keeps the whole of that decision testable on
/// a platform that has neither.
fn root_under(own: &str, mount: &std::path::Path) -> Result<std::path::PathBuf, Unenforced> {
    // cgroup v2 has exactly one line, `0::<path>`. Anything else is v1, or a
    // kernel with no cgroups at all.
    let path = own
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .ok_or(Unenforced::NoCgroups)?
        .trim();
    let mut dir = mount.join(path.trim_start_matches('/'));
    if !dir.is_dir() {
        return Err(Unenforced::NoCgroups);
    }
    loop {
        if delegates(&dir) {
            return Ok(dir);
        }
        let Some(parent) = dir.parent().map(std::path::Path::to_path_buf) else {
            return Err(Unenforced::NotDelegated);
        };
        if dir == mount || parent == dir {
            // Above the mount is not this hierarchy, and the mount root itself
            // is the last candidate rather than a place to stop short of: in a
            // container it *is* the delegated root, and on a host an
            // unprivileged `mkdir` there fails and lands in the same answer.
            return Err(Unenforced::NotDelegated);
        }
        dir = parent;
    }
}

/// Whether a cgroup hands its children the controllers this needs.
fn delegates(dir: &std::path::Path) -> bool {
    std::fs::read_to_string(dir.join("cgroup.subtree_control")).is_ok_and(|c| {
        let enabled: Vec<&str> = c.split_whitespace().collect();
        enabled.contains(&"memory") || enabled.contains(&"pids")
    })
}

/// Enough to tell two commands of one process apart.
///
/// Nanoseconds since the epoch, truncated deliberately rather than cast: the
/// value only has to differ between two cgroups this process makes, and the pid
/// is already in the name beside it.
fn unique() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| {
        u64::try_from(d.as_nanos() & u128::from(u64::MAX)).unwrap_or(0)
    })
}

/// A running sampler, stopped and joined however this function leaves.
///
/// A guard rather than a closure because `spawn_in_cage` has three exits that do
/// not run to the end -- two `?` on a failed `wait`, and the timeout -- and a
/// sampler left running holds a thread waking four times a second for the life
/// of the process. `Drop` catches every path, including one added later
/// (`A1-309`, audit).
struct Sampling {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<Cost>>,
    /// Started beside the child, so the wall clock is the *command's*.
    ///
    /// The sampler cannot time it: it notices the stop flag only after its
    /// current sleep, which backs off to a second, so a command that ended at
    /// 4.0s could be reported at 5.0s. Measured once at 82ms of inflation,
    /// which was luck rather than a bound (`A1-309`, audit).
    started: std::time::Instant,
}

impl Sampling {
    fn start(pgid: u32) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let theirs = std::sync::Arc::clone(&stop);
        let handle = std::thread::spawn(move || sample_group(pgid, &theirs));
        Self {
            stop,
            handle: Some(handle),
            started: std::time::Instant::now(),
        }
    }

    /// Stop it and take what it saw. Idempotent: a second call sees no handle.
    fn stop(&mut self) -> Cost {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let elapsed = self.started.elapsed();
        let mut cost = self
            .handle
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        cost.elapsed = elapsed;
        cost
    }
}

impl Drop for Sampling {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Sample one process group until told to stop, keeping the peaks.
///
/// Reads `/proc` rather than a cgroup deliberately. A cgroup carries
/// `memory.peak` and `pids.peak` for nothing, and is unavailable in exactly the
/// place this has to work: inside an undelegated container, which is what this
/// project's own CI runs in, `/sys/fs/cgroup` is read-only and creating a
/// sub-cgroup is denied (`A1-308`, audit). `/proc` is readable there, and the
/// child leads its own process group already, which is the handle this needs.
#[cfg(unix)]
fn sample_group(pgid: u32, stop: &std::sync::atomic::AtomicBool) -> Cost {
    use std::sync::atomic::Ordering;
    let mut cost = Cost::default();
    let mut samples = 0u32;
    loop {
        let mut procs = 0usize;
        let mut rss = 0u64;
        if let Ok(entries) = std::fs::read_dir("/proc") {
            // Readable `/proc` is what "measured" means -- *this platform can be
            // asked* -- not "the command was still alive when we looked". A
            // command that exits inside the first sampling interval leaves an
            // empty group, and reporting that as "could not be measured on this
            // platform" blamed the platform for a race. The same misattribution
            // `A1-300` exists to prevent, in the reporting half (`A1-309`, audit).
            cost.measured = true;
            for e in entries.flatten() {
                let name = e.file_name();
                let Some(member) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
                    continue;
                };
                // Field 5 of `stat` is the process group, and field 2 is the
                // executable name in parentheses, which may itself contain
                // spaces or a `)`. Splitting after the *last* `)` is the only
                // parse that survives a program called `foo bar) baz`.
                let Ok(stat) = std::fs::read_to_string(format!("/proc/{member}/stat")) else {
                    continue;
                };
                let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else {
                    continue;
                };
                let mut fields = rest.split_whitespace();
                // after the `)` the next fields are state, ppid, pgrp
                if fields.nth(2).and_then(|p| p.parse::<u32>().ok()) != Some(pgid) {
                    continue;
                }
                procs += 1;
                if let Ok(status) = std::fs::read_to_string(format!("/proc/{member}/status")) {
                    for line in status.lines() {
                        if let Some(v) = line.strip_prefix("VmRSS:") {
                            rss += v
                                .split_whitespace()
                                .next()
                                .and_then(|n| n.parse::<u64>().ok())
                                .unwrap_or(0);
                            break;
                        }
                    }
                }
            }
        }
        cost.peak_processes = cost.peak_processes.max(procs);
        cost.peak_rss_kb = cost.peak_rss_kb.max(rss);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        // Backs off, because the walk is not free and its cost is the *box's*,
        // not the command's: finding one process group means reading every
        // `/proc/<pid>/stat`, and this host has 2,663 processes, which measured
        // 41ms a walk -- 16% of a core at four samples a second, sustained for
        // as long as the longest test suite runs.
        //
        // A process group is also not a tree: a child reparented to init stays
        // in the group, so the cheaper walk of `/proc/<pid>/task/*/children`
        // would under-report exactly the runaway worth reporting. The rate
        // gives way instead of the definition. Fine for the first three
        // seconds, where a short command's whole life is, then once a second,
        // which is 4% of a core and plenty for a suite that runs for minutes.
        samples += 1;
        std::thread::sleep(if samples < 12 {
            std::time::Duration::from_millis(250)
        } else {
            std::time::Duration::from_secs(1)
        });
    }
    // `elapsed` is deliberately left at zero: the guard overwrites it with the
    // command's own wall clock, which this thread cannot know.
    cost
}

/// Nothing to read without `/proc`: the absence is reported, never a zero.
#[cfg(not(unix))]
fn sample_group(_pgid: u32, _stop: &std::sync::atomic::AtomicBool) -> Cost {
    Cost::default()
}

/// Spawn `argv` with no shell. This is the trust boundary: the test name is one
/// of these argv elements, never a shell string, so a task file cannot inject a
/// command through it (HLD §5.5).
/// Run a configured command, naming the config key it came from so a failure
/// says which half of `.aplan.yml` to look at.
/// A process's output as lines, both streams, with every run of digits replaced.
///
/// Both streams because a runner may report to either and the comparison is only
/// as good as what it sees; ordering between them is not preserved by any pipe,
/// so they are concatenated in a fixed order rather than interleaved by time.
///
/// Digits go because a clock and a duration are the two things a runner stamps
/// its own output with, and neither says whether a test ran. Replacing them is
/// deterministic, which measuring them across two runs was not.
fn masked_lines(out: &std::process::Output) -> Vec<String> {
    let mask = |s: &str| {
        let mut out = String::with_capacity(s.len());
        let mut in_digits = false;
        for c in s.chars() {
            if c.is_ascii_digit() {
                if !in_digits {
                    out.push('#');
                    in_digits = true;
                }
            } else {
                out.push(c);
                in_digits = false;
            }
        }
        out
    };
    let mut lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(&mask)
        .collect();
    lines.extend(String::from_utf8_lossy(&out.stderr).lines().map(&mask));
    lines
}

/// A command as one line, whatever it contains.
///
/// An argument may hold a newline (`printf '%s\n'` is an ordinary `list`
/// command), and this text is quoted inside a diagnostic message. A diagnostic
/// that spans lines breaks every format that treats one finding as one line, so
/// the control characters are shown as escapes rather than obeyed (`A1-066`,
/// found while testing it).
fn one_line(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// What a pipe-reading thread read, or nothing if it panicked.
fn join_or_empty(handle: std::thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
}

/// Kill a timed-out child and everything it started.
///
/// The group, on unix, because the direct child is often a shell that forked.
/// Elsewhere, the child alone: this is the portable floor, and the platforms
/// this tool ships for are unix.
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // Through a shell's `kill` *builtin*, not through FFI: this workspace
        // *forbids* `unsafe`, and `libc::killpg` is the only other way to signal
        // a group. Via `sh -c` rather than by running `kill` directly, which is
        // the version that shipped first and failed: `kill` is a builtin, and
        // the CI container this project runs on has no `/bin/kill`, so the
        // group survived and every timeout waited for the command it had just
        // given up on.
        //
        // The child leads its own group (`process_group(0)` above), so the
        // negated pid names the group and nothing else. `SIGKILL` rather than
        // `SIGTERM`: this command has already ignored its deadline, and a second
        // grace period is another wait for the same answer.
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("kill -KILL -{} 2>/dev/null", child.id()))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    // Whether or not the group signal landed, the direct child is killed: on
    // non-unix this is the whole mechanism, and on unix it is the fallback for a
    // system with no `kill` binary.
    let _ = child.kill();
}

/// What running a configured command produced (`A1-121`).
#[derive(Debug)]
enum Ran {
    /// It exited, with this status and output.
    Exited(std::process::Output),
    /// It was still running when its limit expired, and was killed. Nothing was
    /// learned: this is not "the command said no".
    TimedOut(std::time::Duration),
}

/// Spawn `argv` with no shell. This is the trust boundary: the test name is one
/// of these argv elements, never a shell string, so a task file cannot inject a
/// command through it (HLD section 5.5).
///
/// Run a configured command, naming the config key it came from so a failure
/// says which half of `.aplan.yml` to look at.
///
/// `limit` of zero means no limit, which is what a repository asks for by
/// setting `timeout_seconds: 0`.
///
/// **Why not `Command::output()`.** It waits for the child to exit, however long
/// that is, and three config keys name commands this tool does not understand.
/// A `url.check` that never returns held `verify` forever, and a CI job that
/// hangs is worse than one that fails, because nothing reports it and the runner
/// is held until the job's own timeout (`A1-121`).
///
/// The output is drained by a thread per pipe rather than read after waiting: a
/// child that fills a pipe buffer blocks until someone reads it, so polling for
/// exit while holding unread pipes would turn a chatty command into a false
/// timeout.
///
/// **The child is killed with its descendants.** Measured rather than assumed: a
/// `url.check` of `sh -c 'sleep 48; true'` forks, so killing the shell left the
/// `sleep` running on the machine, once per timed-out citation. The child is put
/// in a process group of its own and the group is signalled, so a command that
/// spawns its own children does not leave them behind.
///
/// The cost, stated because it is a real one: a child in its own group no longer
/// receives the terminal's `SIGINT`, so interrupting `aplan verify` by hand
/// leaves the command it was running to finish on its own. That trade buys a
/// bounded, silent-in-CI leak becoming none, and the timeout now ends the
/// command in the case the interrupt was compensating for.
///
/// Every configured command goes through here, inside whatever caps the block
/// that declared it asked for -- `test.list`, `test.run`, `test.skipped` and
/// `url.check` alike, which is what ADR-0033 decided and what `A1-310`'s audit
/// found `url.check` had been left out of.
///
/// The root is found per command rather than once: it is five small reads of
/// `/proc` beside a `fork` and an `exec`, and finding it once would mean caching
/// a fact about a machine for the life of a process that outlives it.
fn spawn_capped(
    argv: &[String],
    dir: &Path,
    what: &str,
    limit: std::time::Duration,
    caps: Caps,
) -> Result<(Ran, Cost), Error> {
    spawn_in_cage(argv, dir, what, limit, caps, &cgroup_root())
}

/// Run a command a config key declares, once, and return what it printed
/// (`A1-313`).
///
/// [`spawn_capped`] with the four keys every declaring block carries applied,
/// and every outcome that is not "it exited zero with output" turned into an
/// [`Error::Exec`] that names the key. It is the seam a caller outside this
/// module uses, because the cage, the timeout and the cost sampler are this
/// module's and a fourth hand-written spawn would be the fourth chance to leave
/// one of them out -- which is how `url.check` came to declare `max_memory` and
/// never apply it (`A1-310`, audit).
///
/// Not a resolver: this answers one question for the run rather than resolving a
/// citation, so it has no [`EvidenceResolver`] to hang off and no cost to fold
/// into a subject's total.
pub(crate) fn run_declared(
    argv: Vec<String>,
    wrap: Option<&[String]>,
    dir: &Path,
    what: &str,
    timeout_seconds: u64,
    max_memory: Option<&str>,
    max_processes: Option<u32>,
) -> Result<String, Error> {
    let caps = Caps {
        memory_bytes: max_memory.and_then(|s| aplan_config::parse_size(s).ok()),
        max_processes,
    };
    let argv: Vec<String> = match wrap {
        Some(prefix) => prefix.iter().chain(argv.iter()).cloned().collect(),
        None => argv,
    };
    let limit = std::time::Duration::from_secs(timeout_seconds);
    let (ran, cost) = spawn_capped(&argv, dir, what, limit, caps)?;
    declared_outcome(what, ran, cost)
}

/// Run a declared command and report whether it *exited zero*, rather than what
/// it printed (`A1-314`).
///
/// [`run_declared`]'s sibling, for the checker whose whole answer is its exit
/// status. A non-zero exit is the answer here and an error there, which is the
/// one thing the two cannot share: `version.check` says "not compatible" by
/// refusing, and reporting that as a command that could not be run would turn
/// the finding this rule exists for into a configuration fault.
///
/// Everything that is *not* an exit -- a command that could not be launched,
/// one the kernel stopped, one that ran out of time -- is still an error, and
/// no claim is called false on the strength of it.
pub(crate) fn run_declared_ok(
    argv: Vec<String>,
    wrap: Option<&[String]>,
    dir: &Path,
    what: &str,
    timeout_seconds: u64,
    max_memory: Option<&str>,
    max_processes: Option<u32>,
) -> Result<(bool, Cost), Error> {
    let caps = Caps {
        memory_bytes: max_memory.and_then(|s| aplan_config::parse_size(s).ok()),
        max_processes,
    };
    let argv: Vec<String> = match wrap {
        Some(prefix) => prefix.iter().chain(argv.iter()).cloned().collect(),
        None => argv,
    };
    let limit = std::time::Duration::from_secs(timeout_seconds);
    let (ran, cost) = spawn_capped(&argv, dir, what, limit, caps)?;
    // The cost is carried out whichever way the answer goes, because a command
    // that took eleven minutes and nine gigabytes is worth reporting even when
    // it said no -- and *especially* when it was stopped for saying it
    // (`A1-309`, `A1-310`).
    declared_result(what, ran, cost).map(|out| (out.status.success(), cost))
}

/// What [`run_declared`] makes of what the command did.
///
/// Separated from the spawn because the three failing outcomes are the ones
/// worth pinning and the hardest to produce: a cap breach needs a delegated
/// cgroup, and a timeout needs a command that hangs for as long as the test is
/// willing to wait. Here they are values, and the messages they produce are
/// asserted rather than hoped for.
fn declared_outcome(what: &str, ran: Ran, cost: Cost) -> Result<String, Error> {
    let out = declared_result(what, ran, cost)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let detail = String::from_utf8_lossy(&out.stderr);
    let detail = detail.lines().next().unwrap_or("no output").trim();
    Err(Error::Exec(format!(
        "`{what}`: the command exited {}: {detail}",
        out.status
            .code()
            .map_or("abnormally".to_string(), |c| c.to_string())
    )))
}

/// Everything that is not an exit, as an error; any exit, as itself
/// (`A1-314`).
///
/// **The split is the one thing the two callers cannot share.** A non-zero exit
/// is a *fault* for a command that was asked to print something and an *answer*
/// for one that was asked a yes-or-no question, so the decision belongs to the
/// caller and everything before it belongs here: a command that could not be
/// launched, one the kernel stopped, one that ran out of time. None of those is
/// an answer, and no claim may be called false on the strength of one.
fn declared_result(what: &str, ran: Ran, cost: Cost) -> Result<std::process::Output, Error> {
    // A command the kernel stopped answered nothing, and the adopter's own cap
    // must not be reported as the command's answer (ADR-0033).
    if let Some(breach) = cost.breach {
        return Err(Error::Exec(format!(
            "`{what}`: it was stopped because it exceeded `{what}.{}`, so it neither \
             answered nor refused",
            breach.key()
        )));
    }
    match ran {
        // **A death by signal is not an exit.** `ExitStatus::success()` is false
        // for a command the kernel killed and `code()` is `None`, so a checker
        // that segfaulted read as "the answer is no" -- and `version.check`'s no
        // means *the artifact is incompatible*. A crashing `cargo semver-checks`
        // would have reported a break that was not there, which is the one thing
        // a checker that could not be asked must never do (`A1-314`, audit).
        Ran::Exited(out) if out.status.code().is_none() => Err(Error::Exec(format!(
            "`{what}`: it was killed rather than exiting ({}), so it neither answered nor \
             refused",
            out.status
        ))),
        Ran::Exited(out) => Ok(out),
        Ran::TimedOut(after) => Err(Error::Exec(format!(
            "`{what}`: it was still running after {}s and was stopped, so nothing was \
             learned. Raise `{what}.timeout_seconds`, or `0` for no limit",
            after.as_secs()
        ))),
    }
}

/// [`spawn_capped`], against a cage root already found (`A1-310`).
///
/// Both answers this takes are real states of a real machine -- a delegated
/// cgroup, or a platform that has none -- and each takes a different branch
/// here. Passed in rather than looked up so both branches are reachable on one
/// machine: a box with cgroups would otherwise never run the second, and a
/// container never the first, and this project's CI is the container.
fn spawn_in_cage(
    argv: &[String],
    dir: &Path,
    what: &str,
    limit: std::time::Duration,
    caps: Caps,
    root: &Result<std::path::PathBuf, Unenforced>,
) -> Result<(Ran, Cost), Error> {
    use std::io::Read as _;

    // Before the cage, not after. The joiner below makes *any* argv non-empty,
    // so checking after it would make this unreachable for a capped command --
    // and an empty `test.skipped` would then run `sh -c '... exec "$@"'` with no
    // arguments, where `exec` is a no-op and the shell exits 0. Declaring a
    // limit would turn a configuration error into a silent "this runner skips
    // nothing" (`A1-310`, audit: reproduced end to end).
    if argv.is_empty() {
        return Err(Error::Exec(format!("`{what}`: the command is empty")));
    }

    // The cage, and the command that puts itself in it. A cap that cannot be
    // applied is carried out in `Cost` rather than failing the command: the
    // decision (ADR-0033) is that an unenforceable platform is told once and the
    // run proceeds, not that it is stopped.
    let (caged, unenforced) = if caps.any() {
        let built = match root {
            Ok(root) => Cage::build_in(root, caps),
            Err(why) => Err(*why),
        };
        match built {
            Ok(cage) => (Some(cage), None),
            Err(why) => (None, Some(why)),
        }
    } else {
        (None, None)
    };
    let owned;
    let argv = match &caged {
        Some(cage) => {
            owned = cage.joiner(argv);
            &owned[..]
        }
        None => argv,
    };

    let (program, rest) = argv
        .split_first()
        .ok_or_else(|| Error::Exec(format!("`{what}`: the command is empty")))?;
    let fail = |e: std::io::Error| {
        Error::Exec(format!(
            "`{what}`: `{}` in {}: {e}",
            one_line(&argv.join(" ")),
            dir.display()
        ))
    };
    let mut command = Command::new(program);
    command
        .args(rest)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Its own group, so the kill below reaches whatever it started.
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(fail)?;

    // Measured from outside the child, in its own thread, because the two paths
    // below differ: the bounded one polls and the unbounded one blocks in
    // `wait`, and `test.run` under `--execute` -- the one that matters most --
    // is the unbounded one (`A1-309`).
    let mut sampling = Sampling::start(child.id());
    let mut finish = move |ran: Ran| {
        let mut cost = sampling.stop();
        cost.unenforced = unenforced;
        cost.breach = caged.as_ref().and_then(Cage::breach);
        (ran, cost)
    };

    let out_thread = child.stdout.take().map(|mut p| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = p.read_to_end(&mut buf);
            buf
        })
    });
    let err_thread = child.stderr.take().map(|mut p| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = p.read_to_end(&mut buf);
            buf
        })
    });

    // No limit means no polling: `test.run` under `--execute` is deliberately
    // unbounded, and waking every few milliseconds for the length of a test suite
    // would be this function's own busy-wait rather than the adopter's
    // (`A1-121`, found by its audit).
    if limit.is_zero() {
        let status = child.wait().map_err(fail)?;
        return Ok(finish(Ran::Exited(std::process::Output {
            status,
            stdout: out_thread.map(join_or_empty).unwrap_or_default(),
            stderr: err_thread.map(join_or_empty).unwrap_or_default(),
        })));
    }

    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(fail)? {
            break status;
        }
        if started.elapsed() >= limit {
            kill_group(&mut child);
            // Reaped, so no zombie is left behind.
            let _ = child.wait();
            // The readers are *not* joined here. A reader returns when every
            // copy of its pipe is closed, so joining would make this function's
            // bound depend on the kill having worked -- and the whole point of
            // the bound is that it holds when something has gone wrong. They
            // end on their own when the writer closes, and there is at most one
            // pair per timed-out command (`A1-121`).
            drop(out_thread);
            drop(err_thread);
            return Ok(finish(Ran::TimedOut(limit)));
        }
        // Long enough that polling costs nothing, short enough that a fast
        // command is not delayed by it.
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let stdout = out_thread.map(join_or_empty).unwrap_or_default();
    let stderr = err_thread.map(join_or_empty).unwrap_or_default();
    Ok(finish(Ran::Exited(std::process::Output {
        status,
        stdout,
        stderr,
    })))
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

/// A runner's `dir` as a `foo/`-shaped prefix. `None` when it names the
/// repository root, which claims nothing about which component owns what.
fn normalize_dir(dir: &str) -> Option<String> {
    let trimmed = dir.trim().trim_start_matches("./").trim_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return None;
    }
    Some(format!("{trimmed}/"))
}

/// Whether any configured runner makes a locality claim that could reach these
/// tests (`A1-274`).
///
/// `APLAN025` reports a cited test that does not cover the code a task owns, and
/// it can only do that where a runner says what it covers: the rule returns
/// early on `claimed.is_empty()`, because *a runner claiming nothing cannot
/// demonstrate a mismatch*. So a repository whose runners declare no `covers`,
/// `dir` or `covers_by_test` has the rule permanently inert.
///
/// Exposed for `vacuity`, which reported those subjects as *checked* -- an
/// answer, and the wrong one, from the command whose purpose is telling a rule
/// that ran from a rule that could not. It reads configuration and test *names*
/// only: no subprocess, no repository, so the report stays hermetic.
#[must_use]
pub fn claims_locality(runners: &aplan_config::TestRunners, tests: &[String]) -> bool {
    let all: Vec<&TestRunner> = match runners {
        aplan_config::TestRunners::One(r) => vec![r],
        aplan_config::TestRunners::Many(rs) => rs.iter().collect(),
    };
    all.iter()
        .any(|r| tests.iter().any(|t| !covered_prefixes(r, t).is_empty()))
}

/// What a runner claims its tests cover, as `foo/`-shaped prefixes.
///
/// `covers` wins over `dir` when both are set, being the more specific claim:
/// `dir` says where the runner runs, which for a runner selecting a component by
/// argument is routinely the repository root. Empty means the runner claims
/// nothing, which is every single-runner config.
fn covered_prefixes(runner: &TestRunner, test: &str) -> Vec<String> {
    // A named subset wins: one runner's tests need not cover one thing, and a
    // repository-wide invariant living in a tools crate is the case that proved
    // it (`A1-069`).
    if let Some(entry) = matching_coverage(runner, test) {
        return entry
            .paths
            .iter()
            .map(|g| literal_dir(g).to_string())
            .collect();
    }
    match &runner.covers {
        Some(covers) if !covers.is_empty() => {
            covers.iter().map(|g| literal_dir(g).to_string()).collect()
        }
        _ => runner
            .dir
            .as_deref()
            .and_then(normalize_dir)
            .into_iter()
            .collect(),
    }
}

/// The leading literal directories of a glob: everything before the first
/// wildcard, truncated to the last `/`, so `src/aplan-*/x` yields `src/`.
///
/// Never compiles the glob, so an unusable one degrades to a wider prefix rather
/// than failing the run.
fn literal_dir(glob: &str) -> &str {
    let cut = glob.find(['*', '?', '[', '{']).unwrap_or(glob.len());
    let head = &glob[..cut];
    match head.rfind('/') {
        Some(i) => &head[..=i],
        None => "",
    }
}

/// The first `covers_by_test` entry naming this test, if any.
fn matching_coverage<'a>(runner: &'a TestRunner, test: &str) -> Option<&'a TestCoverage> {
    runner
        .covers_by_test
        .as_ref()?
        .iter()
        .find(|e| e.tests.iter().any(|p| test_name_matches(p, test)))
}

/// Match a test name against a pattern where `*` stands for any run of
/// characters (`invariants::*`).
///
/// Deliberately not a glob library: a test name is not a path, `::` is not a
/// separator, and a total matcher cannot be malformed, so a typo in a pattern
/// cannot abort a run the way a bad path glob can (`A1-066`).
fn test_name_matches(pattern: &str, name: &str) -> bool {
    let Some((first, rest_parts)) = pattern
        .split('*')
        .collect::<Vec<_>>()
        .split_first()
        .map(|(f, r)| (*f, r.to_vec()))
    else {
        return false;
    };
    if rest_parts.is_empty() {
        return pattern == name;
    }
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    for (i, part) in rest_parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == rest_parts.len() - 1 {
            // After the last `*` the pattern is anchored at the end.
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// Whether a task path glob can reach anything under `dir`.
///
/// Compared on the glob's leading *literal* directories, everything before the
/// first wildcard truncated to the last `/`, so `src/aplan-*/x` contributes
/// `src/`. A glob with no literal directory at all (`**`, `*.rs`, `README.md`)
/// could match at any depth, so it counts as reaching everywhere: this rule
/// reports a mismatch it can demonstrate, and stays quiet otherwise.
fn reaches_dir(dir: &str, glob: &str) -> bool {
    let literal = literal_dir(glob);
    literal.is_empty() || literal.starts_with(dir) || dir.starts_with(literal)
}

/// The first 12 characters of a SHA, for a readable message. Falls back to the
/// whole reference when it is shorter (an abbreviated SHA or a ref name).
fn short_sha(reference: &str) -> &str {
    reference.get(..12).unwrap_or(reference)
}

/// `url` evidence: L1 (the configured command answers, `APLAN016`).
///
/// Present only when `.aplan.yml` sets `url.check`, so a default run never
/// touches the network and `check` keeps its hermetic contract (`A1-041`).
#[derive(Debug)]
pub struct UrlResolver {
    check: Vec<String>,
    /// What this checker's command may spend, and what it runs under
    /// (`A1-310`). ADR-0033 covers *every* spawned command, not the test
    /// runner's alone: a checker is argv an adopter chose exactly as a runner
    /// is, and the machine does not care which one ate it.
    caps: Caps,
    wrap: Option<Vec<String>>,
    /// What the checker's commands cost, summed the way a runner's are
    /// (`A1-309`). Measured before this and then discarded, so the one command
    /// a plain `verify` can spawn reported nothing (`A1-310`, audit).
    cost: RefCell<Cost>,
    repo_root: PathBuf,
    /// One verdict per url, so a plan citing the same page from ten tasks makes
    /// one request. The listing cache exists for the same reason (`A1-054`).
    seen: RefCell<std::collections::BTreeMap<String, bool>>,
    /// Why the checker itself could not be run, if it could not (`A1-066`).
    ///
    /// The third instance of the shape `A1-066` was filed about: a `url.check`
    /// naming a program that is not installed aborted the whole run, so every
    /// unrelated finding was lost to a key that is opt-in and advisory. It is a
    /// finding now, and no `url` citation is reported as unanswered on the
    /// strength of a checker that never ran.
    fault: RefCell<Option<String>>,
    /// Where `.aplan.yml` is, for the finding that is about it.
    config_file: Option<PathBuf>,
    /// How long one check may take, in seconds. Zero means no limit
    /// (`A1-121`).
    timeout_seconds: u64,
    /// The urls whose check was abandoned, and after how long. A third outcome
    /// beside answered and did not answer: nothing was learned.
    timed_out: RefCell<std::collections::BTreeMap<String, u64>>,
}

impl UrlResolver {
    #[must_use]
    /// Built from the whole `url` block rather than from fields picked out of
    /// it: every key it grows reaches the spawn, which is how `max_memory` came
    /// to be declared and never applied (`A1-310`, audit).
    pub fn new(
        url: &aplan_config::UrlCheck,
        repo_root: PathBuf,
        config_file: Option<PathBuf>,
    ) -> Self {
        Self {
            check: url.check.clone(),
            caps: Caps {
                memory_bytes: url
                    .max_memory
                    .as_deref()
                    .and_then(|s| aplan_config::parse_size(s).ok()),
                max_processes: url.max_processes,
            },
            wrap: url.wrap.clone(),
            cost: RefCell::new(Cost::default()),
            repo_root,
            seen: RefCell::new(std::collections::BTreeMap::new()),
            fault: RefCell::new(None),
            config_file,
            timeout_seconds: url.timeout_seconds.unwrap_or(URL_TIMEOUT_SECS),
            timed_out: RefCell::new(std::collections::BTreeMap::new()),
        }
    }

    /// Whether the url answered: `None` when nothing asked it, because the
    /// checker could not be run.
    fn reachable(&self, url: &str) -> Option<bool> {
        if self.fault.borrow().is_some() {
            return None;
        }
        if let Some(hit) = self.seen.borrow().get(url) {
            return Some(*hit);
        }
        let argv: Vec<String> = self
            .check
            .iter()
            .map(|a| {
                if a == aplan_config::URL_PLACEHOLDER {
                    url.to_string()
                } else {
                    a.clone()
                }
            })
            .collect();
        // A non-zero exit is the answer this rule wants (the url did not
        // answer). A checker that cannot be *launched* has answered nothing, and
        // saying "did not answer" about it would report the adopter's missing
        // `curl` as a dead link (`A1-066`).
        let limit = std::time::Duration::from_secs(self.timeout_seconds);
        let argv = match &self.wrap {
            Some(prefix) => prefix.iter().chain(argv.iter()).cloned().collect(),
            None => argv,
        };
        let spawned = spawn_capped(&argv, &self.repo_root, "url.check", limit, self.caps);
        if let Ok((_, cost)) = &spawned {
            self.cost.borrow_mut().fold(*cost);
            // A checker the kernel stopped answered nothing, and saying "this
            // url did not answer" about it would report the adopter's own cap
            // as a dead link. The fault channel is where ADR-0033 puts a breach
            // in a command that is not `test.run`.
            if let Some(breach) = cost.breach {
                *self.fault.borrow_mut() = Some(format!(
                    "the configured `url.check` command was stopped: it exceeded `url.{}`, so \
                     it neither answered nor refused",
                    breach.key()
                ));
                return None;
            }
        }
        match spawned.map(|(ran, _)| ran) {
            Ok(Ran::Exited(out)) => {
                let ok = out.status.success();
                self.seen.borrow_mut().insert(url.to_string(), ok);
                Some(ok)
            }
            // A limit that expired is a third outcome: not answered, not
            // refused, *not checked*. Reported per url, because it is this url's
            // check that was abandoned (`A1-121`).
            Ok(Ran::TimedOut(after)) => {
                self.timed_out
                    .borrow_mut()
                    .insert(url.to_string(), after.as_secs());
                None
            }
            Err(why) => {
                *self.fault.borrow_mut() = Some(format!(
                    "the configured `url.check` command could not be run: {why}"
                ));
                None
            }
        }
    }
}

impl EvidenceResolver for UrlResolver {
    fn kind(&self) -> EvidenceKind {
        EvidenceKind::Url
    }

    /// What the checker cost, when it ran at all.
    ///
    /// Nothing for a plan that cites no url: the checker is never spawned, and
    /// a zero would read as a measurement rather than as an absence.
    fn spend(&self) -> Vec<(String, Cost)> {
        let cost = *self.cost.borrow();
        // Untouched means no url was ever checked: a plan citing none never
        // spawns the checker, and a zero would read as a measurement.
        if cost == Cost::default() {
            return Vec::new();
        }
        vec![("the `url.check` checker".to_string(), cost)]
    }

    fn resolve(
        &self,
        _repo: &Repo,
        task: &Task,
        file: &Path,
        ev: &Evidence,
        out: &mut Vec<Diagnostic>,
    ) -> Result<(), Error> {
        // HTTPS only, which HLD section 5.5 called for and this rule's first
        // implementation dropped. The *citation* comes from a task file, and a
        // task file can arrive in a fork pull request, so the adopter's command
        // cannot defend itself: `curl -sfI {url}` fetches whatever the task
        // names. Measured before this existed: a task file made the checker
        // request `file:///etc/passwd` and `http://169.254.169.254/`, the cloud
        // metadata address. Only this tool can refuse to hand it over.
        //
        // Reported rather than skipped, because a citation nothing checked must
        // not read like one that answered, and phrased as *not checked* rather
        // than *did not answer*: nothing was asked (`A1-041`).
        // Case-insensitively: a scheme is case-insensitive (RFC 3986), and
        // `HTTPS://EXAMPLE.COM` was being reported as unchecked, which is a
        // false negative on a citation that is perfectly good.
        if !ev
            .reference
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
        {
            out.push(
                Diagnostic::warning(
                    code::URL_UNREACHABLE,
                    format!(
                        "url `{}` was not checked: only `https` citations are fetched",
                        ev.reference
                    ),
                )
                .with_task(&task.id)
                .with_file(file)
                .with_agent_instruction(
                    "Cite the `https` form. A citation names the target, and a task file can \
                     arrive from a fork, so this tool will not hand an arbitrary scheme or \
                     address to the configured checker.",
                ),
            );
            return Ok(());
        }
        // `Some(false)` is the only case with something to say here:
        // `Some(true)` answered, and `None` means nothing learned -- either the
        // checker could not be run at all, which its own finding reports once
        // (`A1-066`), or this url's check was abandoned at its limit, which is
        // reported per citation just below (`A1-121`).
        if self.reachable(&ev.reference) != Some(false) {
            if let Some(after) = self.timed_out.borrow().get(&ev.reference) {
                out.push(
                    Diagnostic::warning(
                        code::URL_UNREACHABLE,
                        // "not checked", not "did not answer": nothing was
                        // learned, and this rule already keeps those apart for
                        // the non-https case (`A1-041`).
                        format!(
                            "url `{}` was not checked: the configured checker had not returned \
                             after {after}s and was killed",
                            ev.reference
                        ),
                    )
                    .with_task(&task.id)
                    .with_file(file)
                    .with_agent_instruction(
                        "Give `url.check` its own timeout (`--max-time` for curl), or raise \
                         `url.timeout_seconds`. A check that was abandoned says nothing about \
                         the link.",
                    ),
                );
            }
            return Ok(());
        }
        out.push(
            Diagnostic::warning(
                code::URL_UNREACHABLE,
                // "did not answer", not "is wrong": a link down for an hour is
                // not a plan that is wrong, and saying otherwise would train an
                // adopter to ignore the rule.
                format!("url `{}` did not answer when it was checked", ev.reference),
            )
            .with_task(&task.id)
            .with_file(file)
            .with_agent_instruction(
                "Check the link. If it has moved, cite the new one; if it is gone, the proof \
                 is gone with it and the task needs different evidence.",
            ),
        );
        Ok(())
    }
    /// The checker that could not be run, reported once and about the config
    /// rather than about any task (`A1-066`).
    fn finish_plan(&self, out: &mut Vec<Diagnostic>) {
        let Some(why) = self.fault.borrow().clone() else {
            return;
        };
        let mut d = Diagnostic::warning(
            code::URL_UNREACHABLE,
            // The lead-in belongs to whoever set the fault: a checker that was
            // stopped at its cap *did* run, and saying it "could not be run"
            // about it would be the one wording `A1-300` exists to prevent,
            // one command over (`A1-310`, audit).
            format!("no `url` citation was checked: {why}"),
        )
        .with_agent_instruction(
            // Neutral about *why* it said nothing, because there are now two
            // ways: it could not be launched, or it was stopped at a cap it
            // was given. The distinction is in the message; the remedy is the
            // same key either way (`A1-310`, audit).
            "Fix `url.check` in `.aplan.yml`, or remove it. A checker that does not answer \
             leaves every `url` citation unchecked, which is not the same as a url that \
             did not answer.",
        );
        if let Some(path) = &self.config_file {
            d = d.with_file(path);
        }
        out.push(d);
    }
}

/// `APLAN019`: every one of a task's `paths` globs matches a tracked file.
///
/// Per glob, not per task, so the message names the dead entry rather than the
/// list it sits in. A task with no `paths` is `APLAN023`'s business, not this
/// rule's: there the remedy is to add a glob and here it is to fix one, and the
/// pilot that found both said reporting them alike is the confusion
/// (pre-public adoption evidence, item 27).
///
/// A `done` task is judged at the revision its evidence cites, not at `HEAD`.
/// Its `paths` were a claim about the code as it stood when the work happened,
/// so deleting that code later must not turn a correct glob into a finding whose
/// suggested remedy is to "correct" it. Only consulted when a glob has already
/// failed against `HEAD`, so the ordinary case costs nothing (`A1-022`).
pub fn report_unmatched_paths(
    repo: &Repo,
    task: &Task,
    file: &Path,
    tracked: &[PathBuf],
    out: &mut Vec<Diagnostic>,
) -> Result<(), Error> {
    // A cancelled task's ownership claim binds nothing, and telling an author to
    // repair the globs of work they have abandoned is noise.
    //
    // Nor does a task whose citations are judged against nothing (`A1-138`). This
    // rule explains itself with "so every rule that judges evidence against it
    // has nothing to bind"; where the task cites no `commit`, `file` or `test`,
    // that clause is vacuously true and the finding reports a harm that is not
    // yet possible. Planning past the current tree is what a plan is: a task that
    // *builds* `crates/foo` declares `crates/foo/**` before it exists, and the
    // fourth pilot had 140 such findings across 71 `todo` tasks under
    // `deny_warnings` and declined the upgrade over them (pre-public adoption evidence
    // item 24, and the correction that sharpened it into this scope).
    //
    // The case the rule exists for is untouched, because it is the case where a
    // citation exists: a closed task whose globs match nothing had evidence that
    // bound to nothing. `close` applies the same scope at the moment evidence
    // appears (`close.evidence_resolves_before_write`), so the rule and the close
    // gate now agree about when a glob matters instead of disagreeing.
    if task.status == Status::Dropped || task.paths.is_empty() || !task.binds_paths() {
        return Ok(());
    }

    // One pass over the tracked set for the whole task, stopping as soon as every
    // glob has matched. Scanning per glob instead made this rule 95% of
    // `verify`'s runtime on a 20k-file repository, because a dead glob walks the
    // whole set and the live ones each walked it again (measured, `A1-022`).
    let Some(unmatched) = unmatched_globs(&task.paths, tracked) else {
        return Ok(());
    };

    // A dead glob beside a live one that the task's evidence *does* bind through
    // is a stale declaration, not an unbound claim (`A1-141`). `A1-138` scoped
    // this rule to where its stated consequence can occur; read once more, the
    // consequence is that the task's evidence binds to nothing, and evidence
    // bound through another glob is bound. Nothing is ever bound *through* an
    // empty glob, so a per-glob reading would silence the rule everywhere.
    //
    // The fourth pilot met this at the moment it matters: `close` refuses what
    // `verify` would fail, so under `deny_warnings` a bound citation could not
    // close a task with a stale sibling. It filed the question against its own
    // interest, with 43 tasks where the rule is unambiguously right and 28 where
    // it is this case (pre-public adoption evidence item 26).
    //
    // It also settles a disagreement between two rules: `APLAN031` asks whether a
    // cited file is inside the `paths` *set*, and this asked its question of each
    // glob alone. Both read the set now.
    if binds_through_paths(repo, task, tracked)? {
        return Ok(());
    }

    // Only now, and only for a closed task, is history worth reading.
    let historic = if task.status == Status::Done {
        historic_paths(repo, task)?
    } else {
        Vec::new()
    };

    for (glob, matcher) in unmatched {
        if historic.iter().any(|p| matcher.is_match(p)) {
            continue;
        }
        // The bare-directory case is the one every pilot hit, so it gets the
        // remedy rather than the generic advice: a glob naming a directory that
        // *is* tracked needs `/**`, and saying so turns this from a puzzle into
        // a one-character fix.
        let looks_like_a_directory = looks_like_a_tracked_directory(&glob, tracked);
        let instruction = if looks_like_a_directory {
            format!(
                "`{glob}` names a directory, and a glob without a wildcard names one file. \
                 Write `{glob}/**` to own everything under it."
            )
        } else {
            format!(
                "Correct `{glob}` to name files this task owns, or remove it. Nothing \
                 the repository tracks matches it."
            )
        };
        // Now that the rule only fires where a citation is judged against the
        // paths, it can say *which* one, rather than a general clause about
        // "every rule that judges evidence" that was vacuously true for the task
        // in front of the reader (`A1-138`, `A1-091`'s convention: a diagnostic
        // may name only what the predicate returned).
        let mut kinds: Vec<&str> = task
            .evidence
            .iter()
            .filter(|e| !e.reference.trim().is_empty())
            .filter_map(|e| match e.kind {
                aplan_core::EvidenceKind::Commit => Some("commit"),
                aplan_core::EvidenceKind::File => Some("file"),
                aplan_core::EvidenceKind::Test => Some("test"),
                _ => None,
            })
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        // "commit and file and test" is not a sentence. Two join with `and`,
        // three or more take the list form, which is the house idiom the
        // consumer-facing sweep already checks for (`A1-089`).
        let cited = match kinds.as_slice() {
            [one] => (*one).to_string(),
            [a, b] => format!("{a} and {b}"),
            many => format!(
                "{} and {}",
                many[..many.len() - 1].join(", "),
                many[many.len() - 1]
            ),
        };
        let binds = if kinds.len() > 1 { "bind" } else { "binds" };
        out.push(
            Diagnostic::warning(
                code::PATHS_UNMATCHED,
                format!(
                    "`paths` entry `{glob}` matches no tracked file, so the {cited} this task \
                     cites {binds} to nothing through it"
                ),
            )
            .with_task(&task.id)
            .with_file(file)
            .with_agent_instruction(instruction),
        );
    }
    Ok(())
}

/// Whether `glob` names a directory the repository tracks, rather than a file.
///
/// The shape every pilot hit: `paths: [src]` owns the file `src` and nothing
/// under it, so a repository full of `src/**` matches none of it. Shared
/// between `APLAN019` and `APLAN053` because both reach the same wrong
/// conclusion otherwise -- "nothing matches these paths" -- and only this
/// distinguishes a glob that needs one character from work that does not exist
/// (`A1-359`, audit). Two rules diagnosing the same thing two ways is how one
/// of them ends up giving advice that cannot work.
fn looks_like_a_tracked_directory(glob: &str, tracked: &[PathBuf]) -> bool {
    !glob.contains('*') && tracked.iter().any(|p| p.starts_with(Path::new(glob)))
}

/// `APLAN053`: a task says it is under way and the repository holds nothing it
/// claims (`A1-359`).
///
/// **The derived half of a stored claim.** `in_progress` arrives with nothing to
/// resolve against, unlike `done`, whose citations `verify` resolves -- and an
/// unresolvable claim is the failure this tool exists to expose. So the status
/// is stored *and* checked: the premise is not "do not store claims", it is "do
/// not take claims on trust".
///
/// Read from the index rather than from history, which is what makes it cheap
/// and what makes it fair: a file written and staged counts, so a task started
/// a minute ago that has created what it owns is silent. The tracked set is
/// already computed once per `verify` for `APLAN019`, so this costs one pass
/// over globs that were going to be compiled anyway.
///
/// Silent for a task with no `paths`, because there is then nothing to judge the
/// claim by and `APLAN023` already asks for them. Silent for globs that will not
/// compile, which is `APLAN032`'s question rather than this one -- an unusable
/// glob matching nothing must not be read as the work not existing.
pub fn report_unstarted_in_progress(
    task: &Task,
    file: &Path,
    tracked: &[PathBuf],
    out: &mut Vec<Diagnostic>,
) {
    if task.status != Status::InProgress || task.paths.is_empty() {
        return;
    }
    let Some(set) = build_globset(&task.paths) else {
        return;
    };
    if tracked.iter().any(|p| set.is_match(p)) {
        return;
    }
    // **The bare-directory case gets the remedy that works** (`A1-359`, audit).
    // `paths: [src]` with `src/a.rs` tracked matches nothing, and telling that
    // author to `git add` their files is advice they have already followed: the
    // glob needs `/**`. `APLAN019` has said so since `A1-022` and is silent
    // here, because a task that is merely under way cites nothing for its globs
    // to bind -- so this rule has to say it or nobody does.
    let directory = task
        .paths
        .iter()
        .find(|glob| looks_like_a_tracked_directory(glob, tracked));
    let instruction = match directory {
        Some(glob) => format!(
            "`{glob}` names a directory, and a glob without a wildcard names one file. \
             Write `{glob}/**` to own everything under it. The work may well have started; \
             nothing can tell while the declaration matches nothing."
        ),
        None => "Either the work has not started, in which case `aplan stop <id>` says so, \
                 or it has and its files are not added yet, in which case `git add` them. If \
                 the task owns files that do not exist yet, its `paths` are a plan rather \
                 than a claim and the status is the thing to change."
            .to_string(),
    };
    out.push(
        Diagnostic::warning(
            code::UNSTARTED_IN_PROGRESS,
            "this task is `in_progress`, and nothing the repository tracks matches its \
             `paths`, so there is nothing to show the work has started",
        )
        .with_task(&task.id)
        .with_file(file)
        .with_agent_instruction(instruction),
    );
}

/// Whether any citation this task makes binds through its `paths`.
///
/// Only `commit` and `file` can: those are the kinds a glob judges
/// (`APLAN012`, `APLAN031`). A `test` is bound by runner coverage rather than by
/// a path match (`APLAN025`), and a `note` or `url` by nothing, so a task citing
/// only those has nothing bound through the set and its dead globs are still
/// worth naming.
fn binds_through_paths(repo: &Repo, task: &Task, tracked: &[PathBuf]) -> Result<bool, Error> {
    let Some(set) = build_globset(&task.paths) else {
        return Ok(false);
    };
    // Judged against what the repository tracks *now*, which is what keeps
    // `A1-022`'s distinction: a live task's `paths` are a claim about the code as
    // it stands, so a commit that touched a file since deleted does not make a
    // dead glob live again. A closed task is judged at the revision its evidence
    // cites, and that is handled below by `historic_paths`.
    //
    // The glob test comes first and does the filtering: measured on 400 tasks
    // each citing a commit that touches 400 files, ordering it this way leaves
    // the membership check running only for the paths a glob already accepted.
    // The set is here so that what remains is a lookup rather than a scan, not
    // because the scan was hot.
    let tracked: std::collections::BTreeSet<&Path> = tracked.iter().map(PathBuf::as_path).collect();
    let binds = |path: &Path| set.is_match(path) && tracked.contains(path);

    // Files first, whatever order the citations are written in: a file is a
    // lookup and a commit is a diff, and one bound citation is the whole answer.
    for ev in &task.evidence {
        if ev.kind == EvidenceKind::File {
            // Membership of `tracked` is the existence check: a path the
            // repository tracks is a path that is there, so this asks git nothing
            // and works in a repository with no commits yet, which `close`
            // already treats as a real situation (`A1-109`).
            let (path, _) = split_rev(&ev.reference);
            if binds(Path::new(path)) {
                return Ok(true);
            }
        }
    }
    for ev in &task.evidence {
        if ev.kind != EvidenceKind::Commit {
            continue;
        }
        let Some(commit) = repo.resolve_commit(&ev.reference) else {
            continue;
        };
        if repo.commit_paths(&commit)?.iter().any(|p| binds(p)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The globs in `paths` that match nothing in `tracked`, with their compiled
/// matchers, or `None` when they all match.
///
/// Returns the matcher rather than the string alone so the caller cannot rebuild
/// it and need a branch for a failure that cannot happen: the malformed ones are
/// dropped here, once. Skipped rather than reported, because a bad glob is a
/// different fault raised as a hard error where it matters, and reporting it here
/// too would give one defect two severities.
fn unmatched_globs(
    paths: &[String],
    tracked: &[PathBuf],
) -> Option<Vec<(String, globset::GlobMatcher)>> {
    let mut builder = GlobSetBuilder::new();
    let mut usable: Vec<(&String, globset::GlobMatcher)> = Vec::new();
    for p in paths {
        if let Ok(g) = aplan_core::glob::compile(p) {
            builder.add(g.clone());
            usable.push((p, g.compile_matcher()));
        }
    }
    let set = builder.build().ok()?;
    let mut seen = vec![false; usable.len()];
    let mut left = usable.len();
    if left == 0 {
        return None;
    }
    for path in tracked {
        for i in set.matches(path) {
            if !seen[i] {
                seen[i] = true;
                left -= 1;
            }
        }
        if left == 0 {
            return None;
        }
    }
    Some(
        usable
            .into_iter()
            .zip(seen)
            .filter(|(_, hit)| !hit)
            .map(|((g, m), _)| (g.clone(), m))
            .collect(),
    )
}

/// The tree at each commit a `done` task cites, so a glob that was live when the
/// work happened is not reported as dead now.
fn historic_paths(repo: &Repo, task: &Task) -> Result<Vec<PathBuf>, Error> {
    let mut paths = Vec::new();
    for ev in &task.evidence {
        if ev.kind != EvidenceKind::Commit {
            continue;
        }
        if let Some(commit) = repo.resolve_commit(&ev.reference) {
            paths.extend(repo.tree_paths(&commit)?);
        }
    }
    Ok(paths)
}

/// The same compilation, for a caller outside this module (`A1-151`): one
/// definition of what a `paths` entry matches, or a measurement would disagree
/// with the rule it is about.
pub(crate) fn globset_for(paths: &[String]) -> Option<globset::GlobSet> {
    build_globset(paths)
}

fn build_globset(paths: &[String]) -> Option<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in paths {
        // An entry that will not compile makes every path-based rule
        // inapplicable to this task, and `APLAN032` has already said so from the
        // hermetic side. Silence here, rather than an error that aborts the run
        // and takes every other task's findings with it (`A1-066`).
        builder.add(aplan_core::glob::compile(p).ok()?);
    }
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        Breach, Cage, Caps, Cost, EvidenceResolver as _, LIST_TIMEOUT_SECS, Ran, TestResolver,
        URL_TIMEOUT_SECS, Unenforced, UrlResolver, listed_names, report_unmatched_paths,
        report_unstarted_in_progress, root_under, spawn_in_cage, test_name_matches,
    };
    use aplan_config::TestRunner;
    use aplan_core::code;
    use std::path::{Path, PathBuf};

    /// **Each way a declared command can fail says which, and names the key**
    /// (`A1-313`).
    ///
    /// Asserted against values rather than against a machine, because two of the
    /// three need a state a test cannot cheaply produce: a cap breach needs a
    /// delegated cgroup, and a timeout needs a command that hangs for as long as
    /// the suite is willing to wait. The messages are what the adopter acts on,
    /// and hoping they are right is how `url.check` came to declare a limit it
    /// never applied (`A1-310`, audit).
    #[test]
    fn every_way_a_declared_command_fails_names_the_key() {
        let ran = |argv: &str| {
            let out = std::process::Command::new(argv)
                .output()
                .expect("a program every unix has");
            Ran::Exited(out)
        };
        let breached = |b: Breach| Cost {
            breach: Some(b),
            ..Cost::default()
        };

        // Stopped at a cap: neither an answer nor a refusal, and the key named
        // is the one that stopped it.
        for (breach, key) in [
            (Breach::Memory, "max_memory"),
            (Breach::Processes, "max_processes"),
        ] {
            let err = super::declared_outcome("version.current", ran("true"), breached(breach))
                .expect_err("a stopped command answered nothing");
            let said = err.to_string();
            assert!(
                said.contains(&format!("`version.current.{key}`")) && said.contains("neither"),
                "{said}"
            );
        }

        // It exited non-zero, which is the command refusing rather than the
        // machine stopping it.
        let err = super::declared_outcome("version.current", ran("false"), Cost::default())
            .expect_err("a non-zero exit is not an answer");
        assert!(err.to_string().contains("exited 1"), "{err}");

        // Its limit expired: nothing was learned, and the message says which key
        // to raise rather than leaving the reader to find it.
        let err = super::declared_outcome(
            "version.current",
            Ran::TimedOut(std::time::Duration::from_secs(7)),
            Cost::default(),
        )
        .expect_err("an abandoned command answered nothing");
        let said = err.to_string();
        assert!(
            said.contains("after 7s") && said.contains("version.current.timeout_seconds"),
            "{said}"
        );

        // And the one that worked, so the pair is not three refusals.
        assert_eq!(
            super::declared_outcome("version.current", ran("true"), Cost::default())
                .expect("it exited zero"),
            ""
        );
    }

    /// [`spawn_capped`] with nothing declared, which is what most of these
    /// cases are about: the machinery around a command, not the cap on it.
    fn spawn_limited(
        argv: &[String],
        dir: &Path,
        what: &str,
        limit: std::time::Duration,
    ) -> Result<(Ran, Cost), super::Error> {
        super::spawn_capped(argv, dir, what, limit, Caps::default())
    }

    /// A `url` block with only a timeout to vary.
    fn url_check(timeout_seconds: Option<u64>) -> aplan_config::UrlCheck {
        aplan_config::UrlCheck {
            check: vec!["true".into()],
            timeout_seconds,
            max_memory: None,
            max_processes: None,
            wrap: None,
        }
    }

    /// The defaults are a contract with an adopter, not an implementation
    /// detail: they are written in `README.md` and `docs/upgrading.md`, and a
    /// silent change to either number would leave those documents lying
    /// (`A1-121`, found by its audit -- nothing pinned them).
    #[test]
    fn the_documented_defaults_are_the_ones_a_run_uses() {
        assert_eq!(URL_TIMEOUT_SECS, 30, "the number `README.md` states");
        assert_eq!(LIST_TIMEOUT_SECS, 600, "the number `README.md` states");

        // And they are what an unconfigured runner or checker actually gets,
        // which is the half a constant alone does not prove.
        let url = UrlResolver::new(&url_check(None), PathBuf::from("."), None);
        assert_eq!(url.timeout_seconds, URL_TIMEOUT_SECS);
        let configured = UrlResolver::new(&url_check(Some(7)), PathBuf::from("."), None);
        assert_eq!(configured.timeout_seconds, 7, "the config wins");
    }

    /// A command that exits normally is not affected by the machinery around it:
    /// its output arrives whole, on both pipes, bounded or not.
    #[test]
    fn a_command_that_exits_returns_its_output_whichever_path_it_took() {
        for limit in [
            std::time::Duration::ZERO,
            std::time::Duration::from_secs(30),
        ] {
            let argv = vec![
                "sh".to_string(),
                "-c".to_string(),
                "printf out; printf err >&2; exit 3".to_string(),
            ];
            let (ran, _cost) =
                spawn_limited(&argv, Path::new("."), "test.list", limit).expect("it ran");
            let Ran::Exited(out) = ran else {
                panic!("a command that exits must not read as a timeout");
            };
            assert_eq!(out.stdout, b"out", "limit {limit:?}");
            assert_eq!(out.stderr, b"err", "limit {limit:?}");
            assert_eq!(out.status.code(), Some(3), "limit {limit:?}");
        }
    }

    /// A command that writes more than a pipe buffer holds must not be mistaken
    /// for one that hung: the readers drain while the wait polls, which is why
    /// they are threads.
    #[test]
    fn a_chatty_command_is_not_a_timeout() {
        let argv = vec![
            "sh".to_string(),
            "-c".to_string(),
            // Comfortably past the 64KiB a pipe holds on Linux.
            "for i in $(seq 1 20000); do echo aplan-fills-the-pipe-buffer; done".to_string(),
        ];
        let (ran, _cost) = spawn_limited(
            &argv,
            Path::new("."),
            "test.list",
            std::time::Duration::from_secs(30),
        )
        .expect("it ran");
        match ran {
            Ran::Exited(out) => assert!(out.stdout.len() > 65_536, "{}", out.stdout.len()),
            Ran::TimedOut(_) => panic!("a command blocked on a full pipe read as a timeout"),
        }
    }

    /// The resolver's own defaulting, for the runner side.
    #[test]
    fn a_runner_with_no_timeout_configured_gets_the_default() {
        let runner = aplan_config::TestRunner {
            output_lines: None,
            skipped: None,
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            run_selects: None,
            name: None,
            dir: None,
            covers: None,
            covers_by_test: None,
            list: vec!["true".into()],
            run: vec!["true".into(), aplan_config::PLACEHOLDER.into()],
            timeout_seconds: None,
        };
        let resolver = TestResolver::new(vec![runner], PathBuf::from("."), false, None);
        assert_eq!(
            resolver.runners[0]
                .timeout_seconds
                .unwrap_or(LIST_TIMEOUT_SECS),
            LIST_TIMEOUT_SECS
        );
    }

    fn task_owning(paths: &[&str], status: aplan_core::Status) -> aplan_core::Task {
        aplan_core::Task {
            id: "A-1".to_string(),
            title: "t".to_string(),
            phase: 0,
            status,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            // A citation, because the rule is scoped to tasks that make one
            // (`A1-138`). A `test` rather than a `file` or a `commit`: those bind
            // *through* a glob, and a citation that binds silences the rule
            // (`A1-141`), which would leave these cases testing nothing. A test
            // is bound by runner coverage instead, so it keeps the scope
            // satisfied while these stay about the glob-versus-tracked-set
            // predicate.
            evidence: vec![aplan_core::Evidence {
                kind: aplan_core::EvidenceKind::Test,
                reference: "some::test".to_string(),
                captured: None,
            }],
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    fn unmatched(paths: &[&str], tracked: &[&str]) -> Vec<aplan_core::Diagnostic> {
        unmatched_with(paths, tracked, aplan_core::Status::Done)
    }

    fn unmatched_with(
        paths: &[&str],
        tracked: &[&str],
        status: aplan_core::Status,
    ) -> Vec<aplan_core::Diagnostic> {
        // An empty repository: these cases are about the glob-versus-tracked-set
        // predicate, and a task with no commit evidence never consults history.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        git2::Repository::init(dir.path()).expect("init");
        let repo = crate::repo::Repo::discover(dir.path()).expect("discover");
        let tracked: Vec<PathBuf> = tracked.iter().map(PathBuf::from).collect();
        let mut out = Vec::new();
        report_unmatched_paths(
            &repo,
            &task_owning(paths, status),
            Path::new("aplan/tasks/A-1.md"),
            &tracked,
            &mut out,
        )
        .expect("no git access is needed without commit evidence");
        out
    }

    /// One `APLAN053` run: a task in `status` owning `paths`, against a
    /// repository tracking `tracked`.
    fn unstarted(
        paths: &[&str],
        tracked: &[&str],
        status: aplan_core::Status,
    ) -> Vec<aplan_core::Diagnostic> {
        let mut task = task_owning(paths, status);
        task.status = status;
        let tracked: Vec<PathBuf> = tracked.iter().map(PathBuf::from).collect();
        let mut out = Vec::new();
        report_unstarted_in_progress(&task, Path::new("aplan/tasks/A-1.md"), &tracked, &mut out);
        out
    }

    #[test]
    fn aplan053_says_the_repository_holds_nothing_for_the_claim() {
        // The message is the product. It has to say *why* the status is being
        // questioned -- that nothing matching the `paths` is tracked -- rather
        // than merely that the status is `in_progress`, which the reader can
        // see. And the remedy has to name both ways out, because which one is
        // right is the author's call: stop the task, or add the files.
        let out = unstarted(
            &["src/not-yet/**"],
            &["src/other.rs"],
            aplan_core::Status::InProgress,
        );
        assert_eq!(out.len(), 1, "the claim is contradicted, so it is reported");
        assert_eq!(out[0].code, code::UNSTARTED_IN_PROGRESS);
        assert!(
            out[0].message.contains("in_progress") && out[0].message.contains("`paths`"),
            "must say which claim and what was judged: {}",
            out[0].message
        );
        let fix = out[0]
            .agent_instruction
            .as_deref()
            .expect("a rule an author has to act on carries an instruction");
        assert!(
            fix.contains("aplan stop") && fix.contains("git add"),
            "must name both ways out: {fix}"
        );
        assert_eq!(out[0].task.as_deref(), Some("A-1"), "names the task");
    }

    #[test]
    fn aplan053_is_silent_where_it_has_nothing_to_judge_the_claim_by() {
        // Three silences, each for its own reason, and all three matter: a rule
        // that fires on any of them is one an adopter switches off.
        assert!(
            unstarted(
                &["src/started.rs"],
                &["src/started.rs"],
                aplan_core::Status::InProgress
            )
            .is_empty(),
            "the work exists, so there is nothing to report"
        );
        assert!(
            unstarted(&[], &["src/a.rs"], aplan_core::Status::InProgress).is_empty(),
            "no `paths` means nothing to judge the claim by; `APLAN023` asks for them"
        );
        for status in [
            aplan_core::Status::Todo,
            aplan_core::Status::Done,
            aplan_core::Status::Dropped,
            aplan_core::Status::Blocked,
        ] {
            assert!(
                unstarted(&["src/not-yet/**"], &["src/other.rs"], status).is_empty(),
                "`{status}` claims nothing about work having started"
            );
        }
    }

    #[test]
    fn aplan053_offers_the_wildcard_when_the_glob_is_a_tracked_directory() {
        // The remedy has to fit the cause. `paths: [src]` with `src/a.rs`
        // tracked matches nothing, and the generic advice -- "add your files"
        // -- is advice this author has already followed. `APLAN019` has said
        // the right thing since `A1-022` and is silent here, because a task
        // that is merely under way cites nothing for its globs to bind.
        let out = unstarted(&["src"], &["src/a.rs"], aplan_core::Status::InProgress);
        assert_eq!(out.len(), 1);
        let fix = out[0].agent_instruction.as_deref().expect("an instruction");
        assert!(
            fix.contains("`src/**`") && fix.contains("names a directory"),
            "the generic advice was given for the directory case: {fix}"
        );
        assert!(
            !fix.contains("git add"),
            "told to add files that are already added: {fix}"
        );

        // And the genuinely-missing case keeps the advice that fits it.
        let out = unstarted(
            &["lib/not-yet/**"],
            &["src/a.rs"],
            aplan_core::Status::InProgress,
        );
        let fix = out[0].agent_instruction.as_deref().expect("an instruction");
        assert!(
            fix.contains("git add") && fix.contains("aplan stop"),
            "the directory advice leaked into the missing-work case: {fix}"
        );
    }

    #[test]
    fn aplan053_does_not_read_an_unusable_glob_as_work_that_is_missing() {
        // `APLAN032`'s question, not this one. A glob that will not compile
        // matches nothing, and reading that as "the work does not exist" would
        // report a second finding for one defect, against the wrong rule.
        assert!(
            unstarted(
                &["src/[unclosed"],
                &["src/a.rs"],
                aplan_core::Status::InProgress
            )
            .is_empty(),
            "a broken glob is a declaration fault rather than absent work"
        );
    }

    #[test]
    fn aplan019_names_the_dead_glob_and_not_the_list_it_sits_in() {
        // The message is the product: a task with three globs, one dead, must be
        // told *which*. Reporting the list would leave the author to find it.
        let out = unmatched(
            &["src/**", "docs/**", "gone/**"],
            &["src/a.rs", "docs/x.md"],
        );
        assert_eq!(out.len(), 1, "only the dead glob is reported");
        assert_eq!(out[0].code, code::PATHS_UNMATCHED);
        assert!(
            out[0].message.contains("`gone/**`"),
            "must name the dead glob: {}",
            out[0].message
        );
        assert!(
            !out[0].message.contains("src/**") && !out[0].message.contains("docs/**"),
            "must not name the living ones: {}",
            out[0].message
        );
    }

    #[test]
    fn aplan019_offers_the_wildcard_when_the_glob_is_a_tracked_directory() {
        // The shape every pilot hit. The remedy is one character, and saying so
        // is the difference between a fix and a puzzle (`A1-022`).
        let out = unmatched(&["libs/core/src"], &["libs/core/src/a.rs"]);
        assert_eq!(out.len(), 1);
        let fix = out[0]
            .agent_instruction
            .as_deref()
            .expect("the remedy is the point");
        assert!(
            fix.contains("`libs/core/src/**`"),
            "must offer the wildcard form: {fix}"
        );
        assert!(fix.contains("names a directory"), "{fix}");
    }

    #[test]
    fn aplan019_is_silent_when_every_glob_matches() {
        assert!(unmatched(&["src/**"], &["src/a.rs"]).is_empty());
        // No paths at all is `APLAN023`'s finding, not this one: there the remedy
        // is to add a glob, here it is to fix one.
        assert!(unmatched(&[], &["src/a.rs"]).is_empty());
    }

    #[test]
    fn aplan019_exempts_a_dropped_task() {
        // A cancelled task's ownership claim binds nothing, and telling an author
        // to repair globs for abandoned work is noise.
        assert!(
            unmatched_with(&["gone/**"], &["src/a.rs"], aplan_core::Status::Dropped).is_empty()
        );
        assert_eq!(
            unmatched_with(&["gone/**"], &["src/a.rs"], aplan_core::Status::Todo).len(),
            1,
            "status is not the scope: a live task citing something judged against \
             its paths is reported, and catching it before the close is the point"
        );
    }

    /// The scope is what the task *cites*, not what its status says (`A1-138`).
    /// Proves the named behaviour `paths.unmatched_needs_evidence` (`A1-211`).
    #[test]
    fn aplan019_is_silent_when_nothing_binds_to_the_paths() {
        let mut nothing_cited = task_owning(&["gone/**"], aplan_core::Status::Done);
        nothing_cited.evidence = Vec::new();
        assert!(
            report_into(&nothing_cited).is_empty(),
            "no citation, so the harm the message describes cannot occur"
        );

        // A `note` and a `url` are judged against nothing, so neither makes a
        // dead glob reportable. `APLAN017` is the rule with an opinion about
        // whether they are strong enough proof; this one has none.
        let mut noted = task_owning(&["gone/**"], aplan_core::Status::Done);
        noted.evidence = vec![
            aplan_core::Evidence {
                kind: aplan_core::EvidenceKind::Note,
                reference: "reviewed by hand".to_string(),
                captured: Some("2026-08-15".to_string()),
            },
            aplan_core::Evidence {
                kind: aplan_core::EvidenceKind::Url,
                reference: "https://example.invalid/x".to_string(),
                captured: None,
            },
        ];
        assert!(report_into(&noted).is_empty());

        // And a citation that is all whitespace is not a citation, by the same
        // reasoning `APLAN005` uses for a `done` claim.
        let mut blank = task_owning(&["gone/**"], aplan_core::Status::Done);
        blank.evidence = vec![aplan_core::Evidence {
            kind: aplan_core::EvidenceKind::Commit,
            reference: "\u{200b}".to_string(),
            captured: None,
        }];
        assert!(report_into(&blank).is_empty());
    }

    /// The message names every kind bound to the paths, and reads as a sentence
    /// when there is more than one (`A1-138`).
    #[test]
    fn aplan019_names_what_is_unbound_and_says_it_as_english() {
        let ev = |kind: aplan_core::EvidenceKind, r: &str| aplan_core::Evidence {
            kind,
            reference: r.to_string(),
            captured: None,
        };
        let say = |evidence: Vec<aplan_core::Evidence>| {
            let mut t = task_owning(&["gone/**"], aplan_core::Status::Done);
            t.evidence = evidence;
            report_into(&t)
                .first()
                .map(|d| d.message.clone())
                .unwrap_or_default()
        };

        assert!(
            say(vec![ev(aplan_core::EvidenceKind::Commit, "abc123")])
                .contains("so the commit this task cites binds to nothing through it"),
            "one kind"
        );
        assert!(
            say(vec![
                ev(aplan_core::EvidenceKind::Commit, "abc123"),
                ev(aplan_core::EvidenceKind::File, "src/a.rs"),
            ])
            .contains("so the commit and file this task cites bind to nothing"),
            "two kinds"
        );
        // Three would read "commit and file and test" under a plain join.
        let three = say(vec![
            ev(aplan_core::EvidenceKind::Commit, "abc123"),
            ev(aplan_core::EvidenceKind::File, "src/a.rs"),
            ev(aplan_core::EvidenceKind::Test, "some::test"),
        ]);
        assert!(
            three.contains("so the commit, file and test this task cites bind to nothing"),
            "{three}"
        );
        // A kind cited twice is named once: the reader is told what is unbound,
        // not how many citations there are.
        let twice = say(vec![
            ev(aplan_core::EvidenceKind::Commit, "abc123"),
            ev(aplan_core::EvidenceKind::Commit, "def456"),
        ]);
        assert!(
            twice.contains("the commit this task cites binds"),
            "{twice}"
        );
    }

    /// Run the rule over one task, against a tracked set it does not match.
    fn report_into(task: &aplan_core::Task) -> Vec<aplan_core::Diagnostic> {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        git2::Repository::init(dir.path()).expect("init");
        let repo = crate::repo::Repo::discover(dir.path()).expect("discover");
        let mut out = Vec::new();
        report_unmatched_paths(
            &repo,
            task,
            Path::new("aplan/tasks/A-1.md"),
            &[PathBuf::from("src/a.rs")],
            &mut out,
        )
        .expect("no history is read without commit evidence");
        out
    }

    #[test]
    fn aplan019_ignores_a_malformed_glob_rather_than_reporting_it_twice() {
        // A bad glob is a different fault, raised as a hard error where it
        // matters. Reporting it here too would give one defect two severities.
        assert!(unmatched(&["src/["], &["src/a.rs"]).is_empty());
    }

    #[test]
    fn a_pattern_without_a_wildcard_is_an_exact_name() {
        assert!(test_name_matches("a::b", "a::b"));
        assert!(!test_name_matches("a::b", "a::b::c"));
        assert!(!test_name_matches("a::b", "x::a::b"));
    }

    #[test]
    fn a_wildcard_spans_the_module_separator() {
        // `::` is not a path separator, so `invariants::*` has to reach a test
        // nested two modules down. Getting this wrong would silently fail to
        // match every real cargo test name.
        assert!(test_name_matches(
            "invariants::*",
            "invariants::tests::the_web_skeleton_exists"
        ));
        assert!(!test_name_matches("invariants::*", "unit::tests::a_helper"));
    }

    #[test]
    fn a_wildcard_never_matches_the_same_text_twice() {
        // The pitfall in every hand-rolled matcher: two parts satisfied by one
        // run of characters. `a*a` needs two `a`s, and `*b*bc` needs a `b` before
        // a separate `bc`.
        assert!(!test_name_matches("a*a", "a"));
        assert!(!test_name_matches("*b*bc", "abc"));
        assert!(!test_name_matches("*a*a*a", "aa"));
        assert!(test_name_matches("*a*a*a", "aaa"));
        assert!(test_name_matches("*ab*ab", "abab"));
    }

    #[test]
    fn several_wildcards_match_in_order() {
        assert!(test_name_matches("a*b*c", "aXbYc"));
        assert!(test_name_matches("a*b*c", "abc"));
        assert!(!test_name_matches("a*b*c", "acb"));
        assert!(test_name_matches("a**b", "ab"));
    }

    #[test]
    fn a_wildcard_anchors_at_each_end() {
        assert!(test_name_matches("*::a_helper", "unit::tests::a_helper"));
        assert!(!test_name_matches(
            "*::a_helper",
            "unit::tests::a_helper_two"
        ));
        assert!(test_name_matches("*invariants*", "crate::invariants::x"));
        assert!(test_name_matches("*", "anything at all"));
    }

    #[test]
    fn a_pattern_matches_nothing_rather_than_erroring() {
        // A total matcher: a pattern a glob library would reject is just a
        // pattern that does not match, so a typo cannot abort the run.
        assert!(!test_name_matches("[unclosed", "a::b"));
        assert!(!test_name_matches("", "a::b"));
        assert!(test_name_matches("", ""));
    }

    /// A runner whose `skipped` command fails is asked once, not once per task.
    ///
    /// The failure path records a fault and caches no list, so the guard is the
    /// only thing standing between one broken command and one spawn per citation
    /// -- 265 of them on this repository's own plan, for a fault already
    /// reported (`A1-300`, audit).
    #[test]
    fn a_skipped_command_that_fails_is_asked_once() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let counter = tmp.path().join("spawns");
        let runner = skip_runner(
            &[
                "sh",
                "-c",
                &format!("echo x >> {}; exit 3", counter.display()),
            ],
            None,
        );
        let resolver = TestResolver::new(vec![runner], tmp.path().to_path_buf(), true, None);
        for _ in 0..5 {
            assert!(
                resolver.skipped_tests(0).is_none(),
                "a command that exits 3 answers nothing"
            );
        }
        let spawns = std::fs::read_to_string(&counter).unwrap_or_default();
        assert_eq!(spawns.lines().count(), 1, "asked {spawns:?}");
    }

    /// A runner whose `skipped` command answers is asked once, not once per task.
    ///
    /// The cache is the difference between one spawn and one per citation: this
    /// repository's own plan carries 563 of them across ten runners, and
    /// `--list --ignored` compiles.
    #[test]
    fn a_skipped_command_that_answers_is_asked_once() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let counter = tmp.path().join("spawns");
        let runner = skip_runner(
            &[
                "sh",
                "-c",
                &format!("echo x >> {}; printf 'gated::one\n'", counter.display()),
            ],
            None,
        );
        let resolver = TestResolver::new(vec![runner], tmp.path().to_path_buf(), true, None);
        for _ in 0..4 {
            assert_eq!(
                resolver.skipped_tests(0).as_deref(),
                Some(["gated::one".to_string()].as_slice()),
                "the same answer every time"
            );
        }
        let spawns = std::fs::read_to_string(&counter).unwrap_or_default();
        assert_eq!(spawns.lines().count(), 1, "asked {spawns:?}");
    }

    /// A `skipped` command that never returns is a fault, not a silence.
    ///
    /// It shares `test.list`'s limit, for `test.list`'s reason: the question is
    /// asked before anything runs, so a command that hangs holds the gate open
    /// rather than making it slow (`A1-121`).
    #[test]
    fn a_skipped_command_that_hangs_is_killed_and_reported() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let runner = skip_runner(&["sleep", "30"], Some(1));
        let resolver = TestResolver::new(vec![runner], tmp.path().to_path_buf(), true, None);
        assert!(resolver.skipped_tests(0).is_none(), "nothing was learned");
        let faults = resolver.faults.borrow();
        let what = &faults[0].as_ref().expect("a fault was recorded").what;
        assert!(what.contains("had not returned after 1s"), "{what}");
        assert!(
            what.starts_with("could not be asked which tests it will not run"),
            "{what}"
        );
    }

    /// And one that cannot be launched at all: a program that is not there.
    #[test]
    fn a_skipped_command_that_cannot_be_launched_is_reported() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let runner = skip_runner(&["no-such-program-anywhere"], None);
        let resolver = TestResolver::new(vec![runner], tmp.path().to_path_buf(), true, None);
        assert!(resolver.skipped_tests(0).is_none(), "nothing was learned");
        let faults = resolver.faults.borrow();
        let what = &faults[0].as_ref().expect("a fault was recorded").what;
        assert!(what.contains("no-such-program-anywhere"), "{what}");
        assert!(
            what.starts_with("could not be asked which tests it will not run"),
            "{what}"
        );
    }

    /// A runner that declares `skipped` and nothing else that matters here.
    fn skip_runner(skipped: &[&str], timeout_seconds: Option<u64>) -> aplan_config::TestRunner {
        aplan_config::TestRunner {
            output_lines: None,
            skipped: Some(skipped.iter().map(|a| (*a).to_string()).collect()),
            max_memory: None,
            max_processes: None,
            wrap: None,
            expect_tests: None,
            run_selects: None,
            name: None,
            dir: None,
            covers: None,
            covers_by_test: None,
            timeout_seconds,
            list: vec!["true".into()],
            run: vec!["true".into()],
        }
    }

    /// The sampler sees the whole process group, not just the command.
    ///
    /// The number an adopter needs is what the *harness* reached -- measured,
    /// 160 processes for vitest against the one process `aplan` spawned -- so a
    /// sampler that counted only the direct child would report 1 and be useless.
    /// The child leads its own process group, which is what makes the rest
    /// findable (`A1-309`).
    #[test]
    fn the_sampler_counts_the_whole_process_group() {
        let (ran, cost) = spawn_limited(
            &[
                "sh".into(),
                "-c".into(),
                // Twelve children, alive long enough for at least one sample.
                "for i in $(seq 1 12); do sleep 1 & done; wait".into(),
            ],
            Path::new("."),
            "test.list",
            std::time::Duration::from_secs(30),
        )
        .expect("it ran");
        assert!(matches!(ran, Ran::Exited(_)), "the command finished");
        assert!(
            cost.measured,
            "/proc was readable, so something was measured"
        );
        assert!(
            cost.peak_processes > 1,
            "the group holds more than the command itself: {cost:?}"
        );
        assert!(
            cost.elapsed >= std::time::Duration::from_millis(500),
            "wall clock is the command's, not zero: {cost:?}"
        );
    }

    /// A runner that declares caps, for the reporting half of `A1-310`.
    fn capped_runner(max_memory: Option<&str>, max_processes: Option<u32>) -> TestRunner {
        TestRunner {
            max_memory: max_memory.map(ToString::to_string),
            max_processes,
            ..skip_runner(&["true"], None)
        }
    }

    fn a_task() -> aplan_core::Task {
        aplan_core::Task {
            id: "T-1".to_string(),
            title: "t".to_string(),
            phase: 0,
            status: aplan_core::Status::Done,
            owner: None,
            effort_days: None,
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            evidence: vec![a_citation()],
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: std::collections::BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
        }
    }

    fn a_citation() -> aplan_core::Evidence {
        aplan_core::Evidence {
            kind: aplan_core::EvidenceKind::Test,
            reference: "hog".to_string(),
            captured: None,
        }
    }

    /// What a limit parses to, which is what the cgroup is actually set from.
    #[test]
    fn a_declared_cap_reaches_the_cage_in_the_units_it_takes() {
        let r = TestResolver::new(
            vec![capped_runner(Some("64M"), Some(4))],
            PathBuf::from("."),
            false,
            None,
        );
        assert_eq!(
            r.caps_of(0),
            Caps {
                memory_bytes: Some(64 * 1024 * 1024),
                max_processes: Some(4),
            }
        );
        assert!(r.caps_of(0).any());

        let none = TestResolver::new(
            vec![capped_runner(None, None)],
            PathBuf::from("."),
            false,
            None,
        );
        assert_eq!(none.caps_of(0), Caps::default());
        assert!(
            !none.caps_of(0).any(),
            "a runner that declared nothing must not get a cgroup"
        );
    }

    /// A stopped command is reported as *stopped*, never as a failing test.
    ///
    /// The wording is the whole finding: `A1-300` exists because "the test
    /// failed" about a test that never ran sends an author to the wrong file.
    #[test]
    fn a_stopped_command_says_which_cap_stopped_it_and_that_nothing_was_proved() {
        let r = TestResolver::new(
            vec![capped_runner(Some("64M"), Some(4))],
            PathBuf::from("."),
            true,
            None,
        );
        let (task, ev) = (a_task(), a_citation());

        let memory = r.stopped(0, Breach::Memory, &ev, &task, Path::new("plan/t.md"));
        assert_eq!(memory.code, code::TEST_FAILED);
        assert!(
            memory.message.contains("was stopped under `--execute`")
                && memory.message.contains("neither passed nor failed"),
            "{}",
            memory.message
        );
        assert!(
            memory.message.contains("`max_memory` (64M)"),
            "it names the adopter's own value, not the parsed one: {}",
            memory.message
        );

        let processes = r.stopped(0, Breach::Processes, &ev, &task, Path::new("plan/t.md"));
        assert!(
            processes.message.contains("`max_processes` (4)"),
            "{}",
            processes.message
        );
    }

    /// The clause still reads when the breach is not the cap that was declared.
    ///
    /// A cgroup outlives no command, but the two counters are read
    /// independently, so nothing guarantees the breached one is the one this
    /// runner wrote. Saying `max_memory (declared)` is honest; a panic or an
    /// empty pair of brackets is not.
    #[test]
    fn a_breach_without_a_declared_cap_still_reads() {
        let r = TestResolver::new(
            vec![capped_runner(None, None)],
            PathBuf::from("."),
            true,
            None,
        );
        assert!(r.breach_clause(0, Breach::Memory).contains("(declared)"));
        assert!(r.breach_clause(0, Breach::Processes).contains("(declared)"));
    }

    /// A listing stopped at a cap is the configuration's fault, not a task's.
    #[test]
    fn a_listing_stopped_at_a_cap_is_reported_against_the_configuration() {
        let r = TestResolver::new(
            vec![capped_runner(Some("64M"), None)],
            PathBuf::from("."),
            false,
            None,
        );
        let err = r.stopped_while_listing(0, Breach::Memory);
        assert!(
            format!("{err}").contains("stopped at its declared limit"),
            "{err}"
        );

        let faults = r.faults.borrow();
        let fault = faults[0].as_ref().expect("the runner carries the fault");
        assert!(
            fault.listing_unknown,
            "a runner that could not list is a runner whose listing is unknown"
        );
        assert!(
            fault.what.contains("was stopped while listing its tests")
                && fault.what.contains("`max_memory` (64M)"),
            "{}",
            fault.what
        );
    }

    /// `wrap` goes outside the command and really runs.
    ///
    /// Proved by the wrapper changing the environment the command observes: a
    /// prefix that were merely prepended to a list and never executed would
    /// leave the variable empty.
    #[test]
    fn a_runner_runs_every_command_under_its_declared_wrap() {
        let runner = TestRunner {
            wrap: Some(vec!["env".into(), "APLAN_WRAPPED=yes".into()]),
            list: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n' \"${APLAN_WRAPPED:-bare}\"".into(),
            ],
            ..skip_runner(&["true"], None)
        };
        let r = TestResolver::new(vec![runner], PathBuf::from("."), false, None);
        assert_eq!(
            r.wrapped(0, &["printf".to_string()]),
            vec!["env", "APLAN_WRAPPED=yes", "printf"],
            "the prefix goes outside the command"
        );
        assert_eq!(
            r.list_tests_uncached(0).expect("it ran"),
            vec!["yes".to_string()],
            "the wrapper ran, rather than being carried around unused"
        );
    }

    /// A command runs inside its cage, and runs anyway where there is none.
    ///
    /// Both halves are real machines -- a delegated cgroup, and a platform with
    /// none -- and both are asserted here rather than one of them waiting for
    /// the right host. The caged half uses a temporary directory, which is what
    /// a cage is made of: the command still enrols itself before it execs, and
    /// its pid is in the file to prove it.
    #[test]
    fn a_command_runs_inside_its_cage_and_runs_uncaged_where_there_is_none() {
        let caps = Caps {
            memory_bytes: Some(64 * 1024 * 1024),
            max_processes: Some(64),
        };
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let (ran, cost) = spawn_in_cage(
            &["printf".to_string(), "caged".to_string()],
            Path::new("."),
            "test.run",
            std::time::Duration::from_secs(30),
            caps,
            &Ok(tmp.path().to_path_buf()),
        )
        .expect("it ran");
        assert_eq!(cost.unenforced, None, "a root was available: {cost:?}");
        assert_eq!(cost.breach, None, "printf breaches nothing");
        match ran {
            Ran::Exited(out) => assert_eq!(out.stdout, b"caged", "the command itself still ran"),
            Ran::TimedOut(_) => panic!("printf is not slow"),
        }
        let cage = std::fs::read_dir(tmp.path())
            .expect("read")
            .next()
            .expect("one cage was made")
            .expect("entry")
            .path();
        assert_eq!(
            std::fs::read_to_string(cage.join("memory.max")).expect("memory.max"),
            (64 * 1024 * 1024).to_string(),
            "the cap was written before the command started"
        );

        // And with nowhere to put it: the command runs, the run is told, and
        // nothing pretends the limit was applied (ADR-0033).
        let (ran, cost) = spawn_in_cage(
            &["printf".to_string(), "bare".to_string()],
            Path::new("."),
            "test.run",
            std::time::Duration::from_secs(30),
            caps,
            &Err(Unenforced::NoCgroups),
        )
        .expect("it ran");
        assert_eq!(cost.unenforced, Some(Unenforced::NoCgroups));
        match ran {
            Ran::Exited(out) => assert_eq!(out.stdout, b"bare", "unprotected, not unrun"),
            Ran::TimedOut(_) => panic!("printf is not slow"),
        }

        // A runner that declared nothing gets no cage at all, so an
        // unenforceable platform is silent about a run that asked for nothing.
        let (_, cost) = spawn_in_cage(
            &["true".to_string()],
            Path::new("."),
            "test.run",
            std::time::Duration::from_secs(30),
            Caps::default(),
            &Err(Unenforced::NoCgroups),
        )
        .expect("it ran");
        assert_eq!(cost.unenforced, None, "nothing was asked for: {cost:?}");
    }

    /// A cap must not swallow the error that the command is missing.
    ///
    /// The joiner makes any argv non-empty, so a guard placed after it can
    /// never fire for a capped command. Reproduced end to end before the fix:
    /// `skipped: []` reported `APLAN013` with no cap declared and went silently
    /// green with one, because `exec` with no arguments is a no-op and the
    /// shell exits 0 (`A1-310`, audit).
    #[test]
    fn an_empty_command_is_refused_whether_or_not_a_cap_is_declared() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        for (caps, root) in [
            (Caps::default(), Err(Unenforced::NoCgroups)),
            (
                Caps {
                    memory_bytes: Some(64 * 1024 * 1024),
                    max_processes: None,
                },
                Ok(tmp.path().to_path_buf()),
            ),
        ] {
            let err = spawn_in_cage(
                &[],
                Path::new("."),
                "test.skipped",
                std::time::Duration::ZERO,
                caps,
                &root,
            )
            .expect_err("an empty command is not a command");
            assert!(
                format!("{err}").contains("the command is empty"),
                "a declared cap must not hide it: {err}"
            );
        }
    }

    /// A stopped command reaches the reported cost, not only the finding.
    ///
    /// `Cost` is public and `verify_measured` hands this vector to a caller, so
    /// a `breach` left at `None` after a command was stopped is a field that
    /// lies about the run (`A1-310`, audit).
    #[test]
    fn a_breach_survives_into_what_the_run_reports() {
        let r = TestResolver::new(
            vec![capped_runner(Some("64M"), None)],
            PathBuf::from("."),
            true,
            None,
        );
        r.record_cost(
            0,
            Cost {
                breach: Some(Breach::Memory),
                measured: true,
                ..Cost::default()
            },
        );
        // And a later quiet command does not erase it: the run still contained
        // one that was stopped.
        r.record_cost(
            0,
            Cost {
                measured: true,
                ..Cost::default()
            },
        );
        assert_eq!(r.spend()[0].1.breach, Some(Breach::Memory));
        assert_eq!(
            r.spend()[0].0,
            "the `runner 1` test runner",
            "the name is a subject a note can read in a sentence"
        );
    }

    /// The cage can kill what the process group cannot reach.
    #[test]
    fn a_cage_can_kill_its_whole_group() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let cage = Cage::build_in(tmp.path(), Caps::default()).expect("a cage");
        cage.kill();
        assert_eq!(
            std::fs::read_to_string(cage.dir.join("cgroup.kill")).expect("cgroup.kill"),
            "1",
            "the kernel kills every process in the group when this is written"
        );
    }

    /// A cage is the files it writes, so it can be built anywhere.
    ///
    /// Under `/sys/fs/cgroup` these are kernel knobs; under a temp directory
    /// they are ordinary files with the same names and contents. That is what
    /// makes the mechanism testable on a machine with no cgroups -- this
    /// project's CI among them (`A1-310`).
    #[test]
    fn a_cage_writes_both_memory_knobs_and_the_process_one() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let cage = Cage::build_in(
            tmp.path(),
            Caps {
                memory_bytes: Some(64 * 1024 * 1024),
                max_processes: Some(4),
            },
        )
        .expect("a plain directory takes plain writes");
        let knob = |f: &str| std::fs::read_to_string(cage.dir.join(f)).expect(f);
        assert_eq!(knob("memory.max"), (64 * 1024 * 1024).to_string());
        assert_eq!(
            knob("memory.swap.max"),
            "0",
            "a cap the pages can page out of is not a cap"
        );
        assert_eq!(knob("pids.max"), "4");

        // And a root that cannot hold a directory is the honest `NotDelegated`,
        // which is what a read-only `/sys/fs/cgroup` in a container looks like.
        let not_a_dir = tmp.path().join("file");
        std::fs::write(&not_a_dir, "").expect("write");
        assert_eq!(
            Cage::build_in(&not_a_dir, Caps::default()).err(),
            Some(Unenforced::NotDelegated)
        );
    }

    /// The `url` block's limits reach the spawn, and nothing is claimed before
    /// a url is checked.
    ///
    /// ADR-0033 covers every spawned command; the build had put the keys on
    /// `test` alone, so this is the half the audit found missing (`A1-310`).
    #[test]
    fn a_url_checker_carries_the_limits_its_block_declares() {
        let quiet = UrlResolver::new(&url_check(None), PathBuf::from("."), None);
        assert_eq!(quiet.caps, Caps::default());
        assert!(
            quiet.spend().is_empty(),
            "no url was checked, so there is nothing to report rather than a zero"
        );

        let capped = UrlResolver::new(
            &aplan_config::UrlCheck {
                max_memory: Some("256M".into()),
                max_processes: Some(16),
                wrap: Some(vec!["nice".into()]),
                ..url_check(None)
            },
            PathBuf::from("."),
            None,
        );
        assert_eq!(
            capped.caps,
            Caps {
                memory_bytes: Some(256 * 1024 * 1024),
                max_processes: Some(16),
            }
        );
        assert_eq!(capped.wrap.as_deref(), Some(&["nice".to_string()][..]));
    }

    /// A knob the platform will not take a write on is refused when the cage is
    /// built, not discovered by the command that depended on it.
    #[test]
    fn a_cage_refuses_a_root_that_cannot_hold_one() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let file = tmp.path().join("not-a-directory");
        std::fs::write(&file, "").expect("write");
        assert_eq!(
            Cage::build_in(&file, Caps::default()).err(),
            Some(Unenforced::NotDelegated),
            "a read-only `/sys/fs/cgroup` in a container looks like this"
        );
    }

    /// Two commands of one process never share a cage.
    ///
    /// They would if the name were fixed, because `create_dir_all` is happy
    /// with a directory that already exists -- and a cage that leaked, because
    /// something in it would not die, carries `memory.events` counters that
    /// would then read as the *next* command's breach. A false "stopped at its
    /// cap" about a command that stayed inside it (`A1-310`, audit: mutation
    /// testing found nothing pinned this).
    #[test]
    fn each_command_gets_a_cage_of_its_own() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let first = Cage::build_in(tmp.path(), Caps::default()).expect("a cage");
        let second = Cage::build_in(tmp.path(), Caps::default()).expect("another");
        assert_ne!(first.dir, second.dir);

        // And a counter left in one is not read from the other.
        std::fs::write(first.dir.join("memory.events"), "oom_kill 1\n").expect("write");
        assert_eq!(first.breach(), Some(Breach::Memory));
        assert_eq!(second.breach(), None, "a neighbour's breach is not mine");
    }

    /// A breach is read from the kernel's counters, never inferred.
    #[test]
    fn a_breach_is_whatever_the_event_counters_say() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let case = |name: &str, files: &[(&str, &str)]| {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).expect("mkdir");
            for (f, contents) in files {
                std::fs::write(dir.join(f), contents).expect("write");
            }
            std::mem::ManuallyDrop::new(Cage { dir }).breach()
        };

        assert_eq!(case("silent", &[]), None, "no counters, no breach");
        assert_eq!(
            case("quiet", &[("memory.events", "low 0\noom 0\noom_kill 0\n")]),
            None,
            "a counter at zero is a command that stayed inside its cap"
        );
        assert_eq!(
            case("oom", &[("memory.events", "low 0\noom 1\noom_kill 1\n")]),
            Some(Breach::Memory)
        );
        assert_eq!(
            case("forked", &[("pids.events", "max 7\n")]),
            Some(Breach::Processes)
        );
        assert_eq!(
            case("garbage", &[("pids.events", "max plenty\n")]),
            None,
            "an unreadable counter is not a breach; it is nothing learned"
        );
    }

    /// The command joins the cgroup itself, then becomes the command.
    ///
    /// Both halves matter and both have been wrong: `sh -c SCRIPT NAME ARGS`
    /// binds `NAME` to `$0`, so a `shift` here ate the program name and the
    /// command never ran at all. The same binding is what carries the cgroup
    /// path, which is why the quote in the directory name below is deliberate.
    #[test]
    fn the_joiner_enrols_the_child_before_it_execs() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        // Under a directory whose name would end a shell string: the path is a
        // name the kernel chose, not one this code did, and interpolating it
        // into the script would make that name executable.
        let root = tmp.path().join("it's here");
        std::fs::create_dir_all(&root).expect("mkdir");
        let cage = Cage::build_in(&root, Caps::default()).expect("a cage");
        let argv = cage.joiner(&["printf".to_string(), "ran".to_string()]);
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .expect("it ran");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "ran",
            "the command kept its own program name and arguments"
        );
        let joined = std::fs::read_to_string(cage.dir.join("cgroup.procs")).expect("procs");
        assert!(
            joined.trim().parse::<u32>().is_ok(),
            "a pid was written before the exec, not after it: {joined:?}"
        );
    }

    /// The cage goes under the nearest ancestor that delegates, not under the
    /// process's own cgroup, which has no controllers to give.
    #[test]
    fn the_cage_root_is_the_nearest_ancestor_that_delegates() {
        let tmp = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let mount = tmp.path();
        let leaf = mount.join("user.slice/user@1000.service/app.scope");
        std::fs::create_dir_all(&leaf).expect("mkdir");
        // A leaf holds processes, so it delegates nothing; the service above it
        // is where a systemd user session hands out the controllers.
        std::fs::write(leaf.join("cgroup.subtree_control"), "").expect("write");
        let service = mount.join("user.slice/user@1000.service");
        std::fs::write(service.join("cgroup.subtree_control"), "cpu memory pids\n").expect("write");

        assert_eq!(
            root_under("0::/user.slice/user@1000.service/app.scope\n", mount),
            Ok(service.clone()),
            "not the process's own cgroup, which has no controllers"
        );

        // Either controller is enough to be worth using: a cgroup that
        // delegates `pids` and not `memory` still bounds `max_processes`, and
        // the write of the one it lacks is what reports the rest honestly.
        std::fs::write(service.join("cgroup.subtree_control"), "pids\n").expect("write");
        assert_eq!(
            root_under("0::/user.slice/user@1000.service/app.scope\n", mount),
            Ok(service.clone()),
            "`memory` or `pids`, not both"
        );

        // Nothing between here and the mount delegates: a container that was
        // given no subtree of its own, which has a remedy its operator can apply.
        std::fs::write(service.join("cgroup.subtree_control"), "").expect("write");
        assert_eq!(
            root_under("0::/user.slice/user@1000.service/app.scope\n", mount),
            Err(Unenforced::NotDelegated)
        );

        // No cgroup v2 at all: a v1 hierarchy, or a path that is not there.
        assert_eq!(
            root_under("2:cpu:/foo\n1:name=systemd:/foo\n", mount),
            Err(Unenforced::NoCgroups),
            "cgroup v1 has no `0::` line, and v1 is not what this writes"
        );
        assert_eq!(
            root_under("0::/gone\n", mount),
            Err(Unenforced::NoCgroups),
            "a path the kernel does not have is not a delegation problem"
        );
    }

    /// Wall clock is the command's, not the sampler's.
    ///
    /// The sampler sleeps between walks, so if it timed itself it would add up
    /// to one whole interval after the command had already exited -- and the
    /// interval backs off to a second. A quarter-second command reporting "1s"
    /// is a number an adopter would size a timeout from (`A1-309`, audit).
    #[test]
    fn wall_clock_is_the_commands_not_the_samplers() {
        let started = std::time::Instant::now();
        let (_, cost) = spawn_limited(
            &["true".into()],
            Path::new("."),
            "test.list",
            std::time::Duration::from_secs(10),
        )
        .expect("it ran");
        let outer = started.elapsed();
        assert!(
            cost.elapsed <= outer,
            "the command cannot have taken longer than the call around it: \
             reported {:?}, actual at most {outer:?}",
            cost.elapsed
        );
        assert!(
            cost.elapsed < std::time::Duration::from_millis(250),
            "`true` does not take a sampling interval: {:?}",
            cost.elapsed
        );
    }

    /// A command that exits before the first sample is still *measured*.
    ///
    /// `measured` says whether this platform can be asked, not whether the
    /// command was still alive when it was. A fast command leaves an empty
    /// process group, and reporting that as "could not be measured on this
    /// platform" blames the platform for a race -- the same misattribution
    /// `A1-300` exists to prevent, arriving in the reporting half (`A1-309`,
    /// audit).
    #[test]
    fn a_command_gone_before_the_first_sample_is_still_measured() {
        let (_, cost) = spawn_limited(
            &["true".into()],
            Path::new("."),
            "test.list",
            std::time::Duration::from_secs(10),
        )
        .expect("it ran");
        assert!(
            cost.measured,
            "`/proc` is readable here, so the platform can be asked: {cost:?}"
        );
    }

    /// A command that forks nothing still reports itself, and reports honestly.
    #[test]
    fn a_quiet_command_costs_what_it_costs() {
        let (_, cost) = spawn_limited(
            &["true".into()],
            Path::new("."),
            "test.list",
            std::time::Duration::from_secs(10),
        )
        .expect("it ran");
        // It may exit before the first sample lands, which is honest: what must
        // never happen is a *large* number from a command that did nothing.
        assert!(cost.peak_processes <= 2, "{cost:?}");
        assert!(
            cost.peak_rss_kb < 512 * 1024,
            "under half a gibibyte: {cost:?}"
        );
    }

    /// A listed name keeps its module path.
    ///
    /// `test.skipped` parsed the listing a second way -- `line.split(':')` --
    /// and a Rust test name is `module::tests::name`, so every name below the
    /// top of a target came back as its first segment. It matched no citation,
    /// so the gate that reads it reported nothing and looked like a gate that
    /// had found nothing wrong (`A1-300`, audit). Both commands share one parser
    /// now, and this is what it has to keep.
    #[test]
    fn a_listed_name_keeps_the_module_path_cargo_printed() {
        let out = b"scale::tests::the_same_shape_writes_the_same_bytes_twice: test\n\
                    check_does_not_care_how_wide_a_cut_is: test\n\
                    \n\
                    bench::wide: benchmark\n\
                    a name with spaces and: a colon in it\n";
        assert_eq!(
            listed_names(out),
            vec![
                "scale::tests::the_same_shape_writes_the_same_bytes_twice",
                "check_does_not_care_how_wide_a_cut_is",
                "bench::wide",
                // Only cargo's two suffixes are stripped; a colon anywhere else
                // is part of the name, because the name is whatever the runner
                // printed and a citation has to match it exactly.
                "a name with spaces and: a colon in it",
            ]
        );
    }
}
