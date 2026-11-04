//! What ships next, read against the repository (`A1-313`, ADR-0038, ADR-0040).
//!
//! `aplan_core::version` answers what a node *owes*, which is a question about
//! the plan alone. Two things it cannot answer live here, because both read
//! something outside the plan:
//!
//! - **the window**: with no release node, what ships is everything closed since
//!   the nearest release tag reachable from `HEAD` (ADR-0038), which is a
//!   history walk and a set of blobs;
//! - **the version shipping today**: read by a command the repository declares,
//!   never parsed, because this tool does not know what a `pyproject.toml` means
//!   (ADR-0040).
//!
//! The arithmetic that joins them is [`required`], and it is one sum rather than
//! two so the number `aplan version` prints, the finding `APLAN048` makes and the
//! tag `ci/scripts/release.sh` accepts cannot disagree.

use std::path::Path;

use aplan_core::load::Plan;
use aplan_core::task::{Status, VersionFloors, VersionImpact};

use crate::repo::ReleaseTag;
use crate::{Error, Repo};

/// What a release closes, and where the answer came from (ADR-0038).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The release tag the window starts after. `None` for a first release,
    /// where the window is everything the plan has ever closed -- the honest
    /// answer, and one that is true exactly once in a repository's life, which
    /// is the run nobody tests.
    pub since: Option<ReleaseTag>,
    /// The ids that are `done` now and were not `done` then, sorted.
    pub closed: Vec<String>,
    /// Set when the checkout cannot answer the question, with the reason.
    ///
    /// A shallow clone has no history to walk, so no tag is reachable and every
    /// task looks newly closed. Reported rather than answered: a window that
    /// silently becomes "everything, ever" would hand a confident number to the
    /// one caller who most needs it to be right (`A1-177`, `A1-301`).
    pub unmeasurable: Option<String>,
}

impl Window {
    /// How this window would be described to somebody reading the answer, which
    /// ADR-0041 requires the bare form to say in both output formats.
    #[must_use]
    pub fn said(&self) -> String {
        match &self.since {
            Some(tag) => format!(
                "measured over {} task(s) closed since `{}`",
                self.closed.len(),
                tag.name
            ),
            None => format!(
                "measured over the whole plan -- {} task(s) closed, and no release tag \
                 reachable from HEAD",
                self.closed.len()
            ),
        }
    }
}

/// The tasks a release closes, against this repository (ADR-0038 decision 3).
///
/// A task counts when it is `done` now and was not `done` at the tag -- by id
/// rather than by path, so a task file that was renamed inside the window is one
/// task and not two. A task that did not exist at the tag counts, which is the
/// ordinary case.
///
/// # Errors
/// [`Error`] when the repository cannot be read.
pub fn release_window(
    repo: &Repo,
    repo_root: &Path,
    plan: &Plan,
    pattern: &str,
) -> Result<Window, Error> {
    let done: Vec<String> = plan
        .tasks
        .iter()
        .filter(|lt| lt.task.status == Status::Done)
        .map(|lt| lt.task.id.clone())
        .collect();
    if repo.is_shallow() {
        return Ok(Window {
            since: None,
            closed: Vec::new(),
            unmeasurable: Some(
                "this is a shallow clone, so no release tag is reachable and what closed \
                 since the last one cannot be read. That is a fact about the checkout, not \
                 about the plan"
                    .to_string(),
            ),
        });
    }
    let base = plan_base(repo, repo_root);
    let Some(tag) = repo.newest_release_tag(pattern)? else {
        // No tag reachable: the window is everything the plan has closed, which
        // is what a first release really contains.
        return Ok(Window {
            since: None,
            closed: done,
            unmeasurable: None,
        });
    };
    let then = statuses_at(repo, &base, plan, &tag.commit)?;
    let closed = done
        .into_iter()
        .filter(|id| then.get(id) != Some(&Status::Done))
        .collect();
    Ok(Window {
        since: Some(tag),
        closed,
        unmeasurable: None,
    })
}

/// What every task file said its status was at `rev`, keyed by the id it
/// declared there.
///
/// The directories are the ones this plan's task files live in, read from the
/// plan rather than assumed: a repository whose tasks are under `docs/plan/` is
/// as ordinary as one using `aplan/tasks/`, and a hardcoded directory would
/// answer "nothing was done then" for it -- silently, which would make every
/// task in the plan look newly closed.
///
/// **Matched as prefixes**, so a task that has since moved between
/// subdirectories is still found at the tag. Keying the answer by the id the
/// file declared does the same job for a rename, and between them a plan
/// reorganised inside the window reports the tasks it closed rather than all of
/// them.
fn statuses_at(
    repo: &Repo,
    base: &Path,
    plan: &Plan,
    rev: &str,
) -> Result<std::collections::BTreeMap<String, Status>, Error> {
    let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for lt in &plan.tasks {
        let path = git_path(base, &lt.path);
        let dir = path
            .rsplit_once('/')
            .map_or(String::new(), |(d, _)| d.to_string());
        dirs.insert(dir);
    }
    // The shortest prefixes only: with `aplan/tasks` in the set, `aplan/tasks/v1`
    // adds nothing and the comparison below would do it twice per file.
    let roots: Vec<String> = dirs
        .iter()
        .filter(|d| !dirs.iter().any(|other| other != *d && is_under(d, other)))
        .cloned()
        .collect();
    let Some(commit) = repo.resolve_commit(rev) else {
        return Ok(std::collections::BTreeMap::new());
    };
    let mut out = std::collections::BTreeMap::new();
    for path in repo.tree_paths(&commit)? {
        let path = path.to_string_lossy().replace('\\', "/");
        // The extension, case-insensitively, exactly as the loader reads it: a
        // `TASK.MD` is a task, and a walk that silently dropped one would make
        // its closure invisible to the window.
        if !std::path::Path::new(&path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        if !roots.iter().any(|root| is_under(&path, root)) {
            continue;
        }
        let Ok(Some(text)) = repo.text_at(Some(rev), &path) else {
            continue;
        };
        if let Some((id, status)) = aplan_core::load::id_and_status(&text) {
            out.insert(id, status);
        }
    }
    Ok(out)
}

/// Whether `path` is inside `root`, where an empty root means the repository
/// root itself and matches only what sits directly in it.
///
/// Spelled out because the empty case is the trap: joining `""` and `/` gives a
/// prefix every path starts with, so a plan whose task files sit at the top of
/// the repository would have matched every markdown file in it.
fn is_under(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return !path.contains('/');
    }
    path.strip_prefix(root)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// What plan paths are made relative to: the repository's working directory,
/// falling back to what the caller named (`A1-313`, audit).
///
/// The fallback is for a bare repository, which has no working tree. It is the
/// old behaviour and it is not good enough on its own: see [`Repo::workdir`].
fn plan_base(repo: &Repo, repo_root: &Path) -> std::path::PathBuf {
    repo.workdir().unwrap_or_else(|| repo_root.to_path_buf())
}

/// A repository-relative path, whatever spelling the loader recorded.
///
/// **The loader's paths are absolute whenever `.aplan.yml` declares a `root`**,
/// and relative when it does not, because discovery canonicalises the config's
/// directory before joining the declared root to it. Git wants the second. This
/// repository declares no `root`, so it took the relative path, which is why
/// every test and every dogfood run agreed with a walk that silently matched
/// nothing for an adopter who declares one (`A1-313`, audit).
///
/// Absolute and not under the base is left alone rather than guessed at: a plan
/// kept outside the repository is a real arrangement, and the window then reads
/// no statuses rather than the wrong ones.
fn git_path(base: &Path, path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let base = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    path.strip_prefix(&base)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The version the plan requires the next release to be (`A1-313`).
///
/// **Two floors, and the answer is the stronger.** What the plan demands is
/// relative to the *last release*: a window owing `breaking` on top of `0.3.0`
/// requires `0.4.0`. What the repository has already staged is relative to
/// nothing: a tree carrying `0.4.0-dev` has said the next number is `0.4.0`, and
/// a team's own staging decision is a claim this tool raises rather than
/// replaces.
///
/// Taking the stronger is the same composition the floors themselves use. Taking
/// `next_version(current, owed)` instead was written first and is wrong by a
/// version: `0.4.0-dev` owing `breaking` since `0.3.0` would become `0.5.0`,
/// skipping the very release being prepared.
///
/// `None` when the scheme is one this does not understand, which is honest
/// rather than a guess (ADR-0034).
#[must_use]
pub fn required(current: &str, since: Option<&str>, owes: VersionImpact) -> Option<String> {
    let staged = aplan_core::version::triple(current)?;
    // With no tag to measure from, the current version is the only baseline
    // there is, and the pre-release rule makes it the right one: `0.4.0-dev`
    // owing a fix is `0.4.0`.
    let demanded = aplan_core::version::next_version(since.unwrap_or(current), owes)?;
    let demanded_parts = aplan_core::version::triple(&demanded)?;
    if demanded_parts >= staged {
        Some(demanded)
    } else {
        // The staged number as the repository spelled it, with any pre-release
        // dropped -- never reassembled from the triple, which would hand back a
        // version nobody wrote.
        Some(
            current
                .split(['-', '+'])
                .next()
                .unwrap_or(current)
                .to_string(),
        )
    }
}

/// The version this repository ships today, by the command it declares
/// (ADR-0040 decision 4).
///
/// The first non-empty line of stdout, trimmed, with one leading `v` removed --
/// because `git describe` prints one and `cargo pkgid` does not, and neither is
/// wrong.
///
/// # Errors
/// [`Error::Exec`] naming `version.current` and what went wrong with it: not
/// launched, stopped at a declared cap, out of time, exited non-zero, or silent.
/// Every one of those is a fault of the configuration rather than a version, and
/// none of them may be reported as a number.
pub fn current_version(
    declared: &aplan_config::DeclaredCommand,
    repo_root: &Path,
) -> Result<String, Error> {
    let out =
        crate::resolve::run_declared(declared, repo_root, "version.current", CURRENT_TIMEOUT_SECS)?;
    let line = out
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    if line.is_empty() {
        return Err(Error::Exec(
            "`version.current`: the command printed nothing, so there is no version to \
             compare against"
                .to_string(),
        ));
    }
    // **A `v` only where a version follows it.** Stripping unconditionally made
    // `version = "0.3.1"` -- a careless `grep` away, and a thing a repository
    // really does print -- come back as `ersion = "0.3.1"`, so the diagnostic
    // that quotes it sent the reader looking for a string nothing produced
    // (`A1-316`, audit of the versioning cut).
    let bare = line
        .strip_prefix('v')
        .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(line);
    Ok(bare.to_string())
}

/// Whether the artifact is compatible with the last release, by the command the
/// repository declares (`A1-314`, `APLAN049`).
///
/// **Exit zero means compatible.** That is the question
/// `cargo semver-checks check-release` answers natively, and asking it this way
/// keeps this tool's vocabulary out of somebody else's checker: a command that
/// had to print `none|fix|feature|breaking` would be this project teaching an
/// ecosystem its words.
///
/// Nothing here diffs an API. It runs the checker and reports what it said,
/// exactly as it runs `test.run`.
///
/// Carries what it cost beside the answer, because an API differ is the most
/// expensive command this tool can spawn and an adopter sizing `max_memory` has
/// nothing to size it by otherwise (`A1-309`).
///
/// # Errors
/// [`Error::Exec`] naming `version.check` when the command could not be run,
/// was killed rather than exiting, was stopped at a declared cap, or ran out of
/// time. None of those is an answer, and no claim may be called false on the
/// strength of one.
pub fn artifact_is_compatible(
    declared: &aplan_config::DeclaredCommand,
    repo_root: &Path,
) -> Result<(bool, crate::Cost), Error> {
    crate::resolve::run_declared_ok(declared, repo_root, "version.check", CHECK_TIMEOUT_SECS)
}

/// How long `version.check` may take, in seconds.
///
/// Ten minutes, the same `test.list` gets and for the same reason: an API differ
/// *compiles*, often twice, and a limit that fires on a cold cache would be a
/// worse defect than the hang it prevents. It bounds the failure rather than
/// policing the duration.
pub const CHECK_TIMEOUT_SECS: u64 = 600;

/// How long `version.current` may take, in seconds.
///
/// The thirty `url.check` gets, and for the same reason: one question with one
/// answer, and a command that has not printed a version in half a minute is not
/// going to.
pub const CURRENT_TIMEOUT_SECS: u64 = 30;

/// What a window owes, with the floors this repository declared.
///
/// Thin, and here rather than in the caller so the CLI, `verify` and the release
/// script cannot each build the set a slightly different way.
#[must_use]
pub fn owed_in(plan: &Plan, window: &Window, floors: VersionFloors) -> aplan_core::version::Owed {
    let by_id: std::collections::BTreeMap<&str, &aplan_core::task::Task> = plan
        .tasks
        .iter()
        .map(|lt| (lt.task.id.as_str(), &lt.task))
        .collect();
    let closed: Vec<&aplan_core::task::Task> = window
        .closed
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .collect();
    aplan_core::version::owed_among(closed, floors)
}

/// How the next release's window was chosen, and what that choice saw
/// (`A1-348`).
///
/// **One decision, because there were two** and they disagreed. `aplan version`
/// preferred the release node the plan models; `APLAN048` always took the tag
/// window. Measured: a plan whose release holds an unfinished `breaking` task
/// had the command answer `2.0.0` while `verify` -- the gate an adopter wires
/// into CI -- reported `ok` on a tree declaring `1.0.1`. The release script
/// would then refuse the tag that CI had been green about all along, which is
/// the discovered-late failure this whole feature exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    /// No release node: everything closed since the nearest release tag.
    SinceTag {
        /// How many tasks closed in it.
        closed: usize,
    },
    /// The release node the plan models, plus what closed since the last tag
    /// that its members do not claim.
    Release {
        /// The node's id.
        id: String,
        /// How many closed tasks it does not claim.
        unclaimed: usize,
    },
    /// The plan models more than one open release, so which ships next is its
    /// question rather than this tool's. A guess here would be a number.
    Ambiguous {
        /// The open releases, in plan order.
        ids: Vec<String>,
    },
    /// The checkout could not answer, with the reason.
    Unmeasurable {
        /// What about the checkout stopped it.
        why: String,
    },
}

/// What the next release owes, over whichever window applies (`A1-348`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextRelease {
    /// The impact, and what raised it.
    pub owed: aplan_core::version::Owed,
    /// The release tag the arithmetic measures from, when one is reachable.
    pub since: Option<ReleaseTag>,
    /// Which window this came from, for saying so.
    pub window: Chosen,
}

impl NextRelease {
    /// How this would be described to somebody reading the answer, which
    /// ADR-0041 requires the derived form to say.
    #[must_use]
    pub fn said(&self) -> String {
        match &self.window {
            Chosen::SinceTag { closed } => match &self.since {
                Some(tag) => format!("measured over {closed} task(s) closed since `{}`", tag.name),
                None => format!(
                    "measured over the whole plan -- {closed} task(s) closed, and no release \
                     tag reachable from HEAD"
                ),
            },
            Chosen::Release { id, unclaimed } => {
                let beside = match (*unclaimed, self.since.as_ref()) {
                    (0, _) => String::new(),
                    (n, Some(tag)) => {
                        format!(
                            ", and {n} task(s) closed since `{}` that it does not claim",
                            tag.name
                        )
                    }
                    (n, None) => format!(", and {n} task(s) closed that it does not claim"),
                };
                format!("measured over the release `{id}` this plan models{beside}")
            }
            Chosen::Ambiguous { ids } => format!("not measured: {} open releases", ids.len()),
            Chosen::Unmeasurable { why } => format!("not measured: {why}"),
        }
    }
}

/// Choose the window and aggregate over it (`A1-348`, ADR-0038, ADR-0041).
///
/// **A release node's members are cuts** (ADR-0022) and a task no cut claims is
/// in none of them, so the hierarchy walk never reaches one: a plan with a
/// `cross-cutting` task declaring `breaking` answered `fix` for the release that
/// shipped it. The answer is therefore the stronger of what the members owe and
/// what closed since the last tag that they do not claim.
///
/// **A union, not a wider walk.** The node says *which cuts ship*, which is a
/// statement of intent no inference improves on; the tag says *when the last
/// release was*, which is the only thing that can tell a task shipping now from
/// one that shipped three releases ago. Sweeping every unclaimed task in the
/// plan instead would raise every release forever on a break that shipped once.
///
/// # Errors
/// [`Error`] when the repository cannot be read. An ambiguous plan and an
/// unanswerable checkout are *values* rather than errors, because both callers
/// have something to say about them.
pub fn next_release(
    repo: &Repo,
    repo_root: &Path,
    plan: &Plan,
    graph: &aplan_core::graph::Graph<'_>,
    floors: VersionFloors,
    pattern: &str,
) -> Result<NextRelease, Error> {
    let open: Vec<&aplan_core::task::Task> = plan
        .tasks
        .iter()
        .map(|lt| &lt.task)
        .filter(|t| t.is_release() && !t.status.is_terminal())
        .collect();
    if open.len() > 1 {
        return Ok(NextRelease {
            owed: nothing(),
            since: None,
            window: Chosen::Ambiguous {
                ids: open.iter().map(|t| t.id.clone()).collect(),
            },
        });
    }
    let window = release_window(repo, repo_root, plan, pattern)?;
    if let Some(why) = window.unmeasurable {
        return Ok(NextRelease {
            owed: nothing(),
            since: None,
            window: Chosen::Unmeasurable { why },
        });
    }
    let Some(release) = open.first() else {
        return Ok(NextRelease {
            owed: owed_in(plan, &window, floors),
            window: Chosen::SinceTag {
                closed: window.closed.len(),
            },
            since: window.since,
        });
    };
    let members = aplan_core::version::owed(graph, floors, &release.id).unwrap_or_else(nothing);
    let reached = aplan_core::version::reached(graph, floors, &release.id);
    let unclaimed = Window {
        closed: window
            .closed
            .iter()
            .filter(|id| !reached.contains(*id))
            .cloned()
            .collect(),
        ..window.clone()
    };
    let swept = owed_in(plan, &unclaimed, floors);
    // The stronger, and on a tie the one that *names* something: a declaration
    // is what a reader can argue with, and a floor is not. Preferring the node
    // unconditionally reported "because the cut is a feature" over a task that
    // had said so itself.
    let owed = if swept.impact > members.impact
        || (swept.impact == members.impact && members.raised_by.is_none())
    {
        swept
    } else {
        members
    };
    Ok(NextRelease {
        owed,
        window: Chosen::Release {
            id: release.id.clone(),
            unclaimed: unclaimed.closed.len(),
        },
        since: window.since,
    })
}

/// An answer that owes nothing and blames nobody.
fn nothing() -> aplan_core::version::Owed {
    aplan_core::version::Owed {
        impact: VersionImpact::None,
        raised_by: None,
    }
}

/// The version a release should record when it closes, or why it cannot be
/// (`A1-339`, ADR-0038).
///
/// **One decision, because two surfaces make it.** The command and `aplan_close`
/// both write the field, and each had its own copy of the derivation -- with the
/// window's own refusals missing from both. A plan with two open releases had
/// `aplan version` say "which one ships next is its question rather than this
/// tool's" and the closure record a number anyway; a shallow clone had the
/// command refuse and the closure write down the version that ships today. A
/// record derived from a window that declined to be measured is worse than no
/// record, because the whole point of the field is that a later run trusts it.
///
/// `Err` carries the reason, for a caller to say: a closure is never refused for
/// this, since holding a finished release hostage to an optional config key
/// would be a worse answer than closing without a number.
///
/// # Errors
/// The reason no version could be derived, as a sentence.
pub fn version_for_release(
    repo_root: &Path,
    plan: &Plan,
    graph: &aplan_core::graph::Graph<'_>,
    policy: &aplan_config::VersionPolicy,
) -> Result<String, String> {
    let Some(declared) = &policy.current else {
        return Err(
            "`.aplan.yml` declares no command that reads the version this artifact \
                    ships, so there is nothing to derive a number from"
                .to_string(),
        );
    };
    let repo = Repo::discover(repo_root).map_err(|e| e.to_string())?;
    let current = current_version(declared, repo_root).map_err(|e| e.to_string())?;
    let next = next_release(
        &repo,
        repo_root,
        plan,
        graph,
        policy.floors(),
        policy.release_tag(),
    )
    .map_err(|e| e.to_string())?;
    match &next.window {
        Chosen::Ambiguous { ids } => {
            return Err(format!(
                "this plan models {} open releases ({}), so which one ships next is its \
                 question rather than this tool's",
                ids.len(),
                ids.join(", ")
            ));
        }
        Chosen::Unmeasurable { why } => return Err(why.clone()),
        Chosen::Release { .. } | Chosen::SinceTag { .. } => {}
    }
    let since = next.since.as_ref().and_then(ReleaseTag::version);
    required(&current, since, next.owed.impact)
        .ok_or_else(|| format!("`{current}` is not a version this tool can do arithmetic on"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository in a scratch directory, with a helper for each thing these
    /// tests do to one.
    struct Fixture {
        dir: tempfile::TempDir,
        repo: git2::Repository,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("tempdir");
            let repo = git2::Repository::init(dir.path()).expect("git init");
            Self { dir, repo }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn write(&self, rel: &str, contents: &[u8]) {
            let p = self.dir.path().join(rel);
            std::fs::create_dir_all(p.parent().expect("a parent")).expect("mkdir");
            std::fs::write(&p, contents).expect("write");
        }

        /// A task file, at `rel`, declaring `id` and `status`.
        fn task(&self, rel: &str, id: &str, status: &str) {
            self.write(
                rel,
                format!("---\nid: {id}\ntitle: t\nphase: 0\nstatus: {status}\n---\n").as_bytes(),
            );
        }

        fn commit(&self, msg: &str) {
            let mut index = self.repo.index().expect("index");
            index
                .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                .expect("add");
            index.write().expect("write index");
            let tree = self
                .repo
                .find_tree(index.write_tree().expect("tree"))
                .expect("find tree");
            let sig = git2::Signature::now("T", "t@e.invalid").expect("sig");
            let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            self.repo
                .commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
                .expect("commit");
        }

        fn tag(&self, name: &str) {
            let sig = git2::Signature::now("T", "t@e.invalid").expect("sig");
            let head = self
                .repo
                .head()
                .expect("head")
                .peel_to_commit()
                .expect("commit");
            self.repo
                .tag(name, head.as_object(), &sig, "a tag", false)
                .expect("tag");
        }

        fn discover(&self) -> Repo {
            Repo::discover(self.dir.path()).expect("discover")
        }

        fn plan(&self, dir: &str) -> Plan {
            aplan_core::load::load_plan(&self.dir.path().join(dir))
        }
    }

    /// **Reachability, not recency** (ADR-0038).
    ///
    /// The deciding measurement of ADR-0038 was this repository's own tag set:
    /// two `archive/` tags sit on commits that are not ancestors of `main`, one
    /// of them on the same day as `v0.3.0`, and a date-ordered "latest tag"
    /// would have ranked it first had it been cut an hour later.
    #[test]
    fn a_tag_that_head_cannot_reach_is_not_the_window() {
        let f = Fixture::new();
        f.task("plan/tasks/A-1.md", "A-1", "todo");
        f.commit("first");
        f.tag("v0.1.0");
        // A second root, tagged later and higher, on no path to HEAD.
        let sig = git2::Signature::now("T", "t@e.invalid").expect("sig");
        let tree = f
            .repo
            .find_tree(f.repo.index().expect("index").write_tree().expect("tree"))
            .expect("tree");
        let side = f
            .repo
            .commit(None, &sig, &sig, "a side branch", &tree, &[])
            .expect("commit");
        f.repo
            .tag(
                "v9.9.9",
                &f.repo.find_object(side, None).expect("object"),
                &sig,
                "unreachable",
                false,
            )
            .expect("tag");

        assert_eq!(
            f.discover()
                .newest_release_tag("v*")
                .expect("readable")
                .map(|t| t.name),
            Some("v0.1.0".to_string()),
            "a higher tag HEAD cannot reach is not this repository's last release"
        );
    }

    /// Every matching tag on a side branch is the same answer as no tag at all:
    /// there is nothing `HEAD` can be measured from.
    #[test]
    fn a_release_tag_on_no_path_to_head_is_no_window() {
        let f = Fixture::new();
        f.task("plan/tasks/A-1.md", "A-1", "done");
        f.commit("first");
        let sig = git2::Signature::now("T", "t@e.invalid").expect("sig");
        let tree = f
            .repo
            .find_tree(f.repo.index().expect("index").write_tree().expect("tree"))
            .expect("tree");
        let side = f
            .repo
            .commit(None, &sig, &sig, "a side branch", &tree, &[])
            .expect("commit");
        f.repo
            .tag(
                "v0.1.0",
                &f.repo.find_object(side, None).expect("object"),
                &sig,
                "unreachable",
                false,
            )
            .expect("tag");
        assert_eq!(
            f.discover().newest_release_tag("v*").expect("readable"),
            None,
            "the only release tag is not an ancestor of HEAD"
        );
    }

    /// No matching tag at all is a first release, not an error.
    #[test]
    fn a_repository_with_no_release_tag_has_no_window_to_measure_from() {
        let f = Fixture::new();
        f.task("plan/tasks/A-1.md", "A-1", "done");
        f.commit("first");
        f.tag("nightly-2026-09-23");
        let repo = f.discover();
        assert_eq!(repo.newest_release_tag("v*").expect("readable"), None);
        let window =
            release_window(&repo, f.path(), &f.plan("plan/tasks"), "v*").expect("readable");
        assert!(window.since.is_none());
        assert_eq!(window.closed, vec!["A-1".to_string()]);
        assert!(
            window.said().contains("the whole plan"),
            "a first release contains all of it, and says so: {}",
            window.said()
        );
    }

    /// A glob that will not compile matches nothing rather than everything.
    ///
    /// The direction matters: widening is the silent failure. A pattern the
    /// config let through would otherwise make every tag in the repository a
    /// release candidate, which is a *wrong* window rather than no window.
    #[test]
    fn an_unusable_pattern_matches_no_tag() {
        let f = Fixture::new();
        f.task("plan/tasks/A-1.md", "A-1", "done");
        f.commit("first");
        f.tag("v0.1.0");
        assert_eq!(
            f.discover().newest_release_tag("[").expect("readable"),
            None
        );
    }

    /// An unborn `HEAD` has no history to walk, so there is nothing to reach the
    /// tag from.
    #[test]
    fn a_repository_with_no_head_reaches_no_tag() {
        let f = Fixture::new();
        f.task("plan/tasks/A-1.md", "A-1", "done");
        f.commit("first");
        f.tag("v0.1.0");
        f.repo
            .set_head("refs/heads/never-committed")
            .expect("set head");
        assert_eq!(
            f.discover().newest_release_tag("v*").expect("readable"),
            None
        );
    }

    /// **A shallow clone is told about, never answered** (`A1-177`, `A1-301`).
    ///
    /// Its history stops at the graft, so no tag is reachable and every task
    /// looks newly closed. A window that silently became "everything, ever"
    /// would hand a confident number to the one caller who most needs it right.
    #[test]
    fn a_shallow_clone_reports_the_checkout_rather_than_a_window() {
        let f = Fixture::new();
        f.task("plan/tasks/A-1.md", "A-1", "done");
        f.commit("first");
        f.tag("v0.1.0");
        let head = f
            .repo
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("commit")
            .id()
            .to_string();
        std::fs::write(f.path().join(".git/shallow"), format!("{head}\n")).expect("shallow");

        let window = release_window(
            &Repo::discover(f.path()).expect("discover"),
            f.path(),
            &f.plan("plan/tasks"),
            "v*",
        )
        .expect("readable");
        assert!(
            window
                .unmeasurable
                .as_deref()
                .is_some_and(|why| why.contains("shallow clone")),
            "{window:?}"
        );
        assert!(
            window.closed.is_empty(),
            "nothing may be claimed about a window that could not be read"
        );
    }

    /// A file the tag holds that is not readable as a task is skipped, not
    /// guessed at: a `.md` that is not UTF-8, and a rev that names nothing.
    #[test]
    fn a_blob_that_is_not_a_task_is_skipped() {
        let f = Fixture::new();
        f.task("plan/tasks/A-1.md", "A-1", "done");
        // Invalid UTF-8 in a file the directory scan will reach.
        f.write("plan/tasks/binary.md", &[0xff, 0xfe, 0x00, 0x41]);
        f.commit("first");
        let repo = f.discover();
        let plan = f.plan("plan/tasks");
        let at_head = statuses_at(&repo, f.path(), &plan, "HEAD").expect("readable");
        assert_eq!(at_head.len(), 1, "the unreadable blob is not a task");
        // And a rev that resolves to nothing is an empty answer rather than an
        // error: the caller is asking what was true then, and nothing was.
        assert!(
            statuses_at(&repo, f.path(), &plan, "no-such-rev")
                .expect("readable")
                .is_empty()
        );
    }

    /// **Reading a rev or a path that is not there is an absence, never an
    /// error** (`A1-313`).
    ///
    /// The window asks what a task file said at a commit, which means asking
    /// about revs and paths that may not exist -- a task added since the tag, a
    /// tag deleted while the answer was being computed. Each of these once had
    /// a plausible wrong answer: an error that took the whole run down, or an
    /// empty string that read as a task file saying nothing.
    #[test]
    fn a_rev_or_a_path_that_is_not_there_reads_as_absent() {
        let f = Fixture::new();
        f.task("plan/tasks/A-1.md", "A-1", "done");
        f.commit("first");
        let repo = f.discover();
        assert_eq!(
            repo.text_at(Some("no-such-rev"), "plan/tasks/A-1.md")
                .expect("readable"),
            None
        );
        // A directory is not a blob, and asking for its text is not an error.
        assert_eq!(repo.text_at(None, "plan/tasks").expect("readable"), None);
        assert!(
            !repo
                .path_exists_at(Some("no-such-rev"), "plan/tasks/A-1.md")
                .expect("readable")
        );
        assert!(
            repo.path_exists_at(None, "plan/tasks/A-1.md")
                .expect("readable")
        );
        // The handle names the repository it opened, which is what a caller
        // debugging a wrong window needs to see first.
        assert!(format!("{repo:?}").contains("Repo"));
    }

    /// A plan whose task files sit at the top of the repository.
    ///
    /// The trap this guards: an empty directory prefix joined with `/` is a
    /// prefix every path starts with, so the scan would have read every markdown
    /// file in the repository as a task.
    #[test]
    fn a_plan_at_the_repository_root_does_not_swallow_the_whole_tree() {
        assert!(is_under("A-1.md", ""));
        assert!(!is_under("docs/design.md", ""));
        assert!(is_under("plan/tasks/A-1.md", "plan/tasks"));
        assert!(is_under("plan/tasks/old/A-1.md", "plan/tasks"));
        assert!(!is_under("plan/tasks-2/A-1.md", "plan/tasks"));
    }

    /// **The plan raises a staged number; it does not replace it** (`A1-313`).
    ///
    /// The arithmetic taking `next_version(current, owed)` was written first and
    /// is wrong by a version: `0.4.0-dev` owing `breaking` since `0.3.0` became
    /// `0.5.0`, skipping the very release being prepared. What the plan demands
    /// is demanded of the number that *shipped*, and the staged number is the
    /// other floor.
    #[test]
    fn the_staged_number_is_a_floor_and_the_plan_raises_it() {
        // Staged above what the plan demands: the staging stands.
        assert_eq!(
            required("0.4.0-dev", Some("0.3.0"), VersionImpact::Feature).as_deref(),
            Some("0.4.0")
        );
        // The plan demands exactly what is staged, which is the release being
        // prepared rather than the one after it.
        assert_eq!(
            required("0.4.0-dev", Some("0.3.0"), VersionImpact::Breaking).as_deref(),
            Some("0.4.0"),
            "a break on top of 0.3.0 *is* 0.4.0; taking the next of the staged number \
             would skip it"
        );
        // The plan demands more than is staged, which is `APLAN048`'s finding.
        assert_eq!(
            required("0.3.1-dev", Some("0.3.0"), VersionImpact::Breaking).as_deref(),
            Some("0.4.0")
        );
        // Nothing staged yet: the tree still carries what it shipped.
        assert_eq!(
            required("0.3.0", Some("0.3.0"), VersionImpact::Fix).as_deref(),
            Some("0.3.1")
        );
    }

    /// With no reachable tag the current version is the only baseline there is,
    /// and the pre-release rule makes it the right one.
    #[test]
    fn a_first_release_measures_from_what_the_tree_declares() {
        assert_eq!(
            required("0.1.0-dev", None, VersionImpact::Feature).as_deref(),
            Some("0.1.0")
        );
        assert_eq!(
            required("1.0.0", None, VersionImpact::Breaking).as_deref(),
            Some("2.0.0")
        );
    }

    /// A scheme this does not understand gets no answer rather than a guess,
    /// at this layer too (ADR-0034).
    #[test]
    fn an_unreadable_scheme_is_refused_rather_than_guessed() {
        assert_eq!(
            required("2026.09.1", Some("0.3.0"), VersionImpact::Fix),
            None
        );
        assert_eq!(
            required("0.4.0", Some("not-a-tag"), VersionImpact::Fix),
            None
        );
    }

    /// The declared command's stdout is the version, and a `v` either way works.
    #[test]
    fn the_declared_command_is_read_as_one_line() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let read = |args: &[&str]| {
            current_version(
                &aplan_config::DeclaredCommand {
                    command: args.iter().map(ToString::to_string).collect(),
                    timeout_seconds: None,
                    max_memory: None,
                    max_processes: None,
                    max_output: None,
                    wrap: None,
                },
                dir.path(),
            )
        };
        assert_eq!(read(&["echo", "0.4.0"]).expect("it runs"), "0.4.0");
        // `git describe` prints a leading `v` and `cargo pkgid` does not; neither
        // is wrong, so one is dropped rather than refused.
        assert_eq!(read(&["echo", "v1.2.3"]).expect("it runs"), "1.2.3");
        assert_eq!(read(&["echo", "  0.4.0  "]).expect("it runs"), "0.4.0");
    }

    /// **A command that could not answer is a fault, never a version**
    /// (`A1-066`, `A1-300`).
    ///
    /// Each of these once had a plausible wrong answer -- an empty string, a
    /// zero, the previous tag -- and each would have been a number this tool
    /// invented and then did arithmetic on.
    #[test]
    fn a_command_that_did_not_answer_is_a_fault() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let read = |args: &[&str]| {
            current_version(
                &aplan_config::DeclaredCommand {
                    command: args.iter().map(ToString::to_string).collect(),
                    timeout_seconds: None,
                    max_memory: None,
                    max_processes: None,
                    max_output: None,
                    wrap: None,
                },
                dir.path(),
            )
        };
        for argv in [
            vec!["false"],
            vec!["true"],
            vec!["no-such-program-anywhere-4f2a"],
        ] {
            let err = read(&argv).expect_err("it must not answer");
            let said = err.to_string();
            assert!(
                said.contains("version.current"),
                "the message must name the key that configured it: {said}"
            );
        }
    }

    /// A task file moved between subdirectories inside the window is one task,
    /// not one closure and one disappearance.
    ///
    /// The directories come from the plan as it stands *now*, so the one a task
    /// used to live in is not in the set. Matching them as prefixes is what
    /// makes the old path readable, and keying by the declared id is what makes
    /// it the same task.
    #[test]
    fn a_task_that_moved_inside_the_window_is_still_read_at_the_tag() {
        // Proof for `repository.path_identity_is_portable`.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("git init");
        let write = |rel: &str, status: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().expect("a parent")).expect("mkdir");
            std::fs::write(
                &p,
                format!("---\nid: A-1\ntitle: t\nphase: 0\nstatus: {status}\n---\n"),
            )
            .expect("write");
        };
        let commit = |msg: &str| {
            let mut index = repo.index().expect("index");
            index
                .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                .expect("add");
            index.write().expect("write index");
            let tree = repo
                .find_tree(index.write_tree().expect("tree"))
                .expect("find tree");
            let sig = git2::Signature::now("T", "t@e.invalid").expect("sig");
            let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
                .expect("commit");
        };
        write("plan/tasks/archive/A-1.md", "done");
        commit("before the release");
        let sig = git2::Signature::now("T", "t@e.invalid").expect("sig");
        let head = repo.head().expect("head").peel_to_commit().expect("commit");
        repo.tag("v0.3.0", head.as_object(), &sig, "release", false)
            .expect("tag");
        std::fs::remove_file(dir.path().join("plan/tasks/archive/A-1.md")).expect("rm");
        write("plan/tasks/A-1.md", "done");
        commit("moved, not reopened");

        let handle = Repo::discover(dir.path()).expect("discover");
        let plan = aplan_core::load::load_plan(&dir.path().join("plan/tasks"));
        let window = release_window(&handle, dir.path(), &plan, "v*").expect("readable");
        assert!(
            window.closed.is_empty(),
            "it was already done at the tag, one directory over: {:?}",
            window.closed
        );
    }

    /// The window over this repository's own history: everything `done` now that
    /// was not `done` at the tag, keyed by id so a renamed file is one task.
    #[test]
    fn the_window_is_what_went_done_since_the_tag() {
        // Proof for `repository.path_identity_is_portable`.
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("git init");
        let write = |name: &str, status: &str| {
            let p = dir.path().join("plan/tasks").join(name);
            std::fs::create_dir_all(p.parent().expect("a parent")).expect("mkdir");
            std::fs::write(
                &p,
                format!(
                    "---\nid: {}\ntitle: t\nphase: 0\nstatus: {status}\n---\n",
                    name.trim_end_matches(".md")
                ),
            )
            .expect("write");
        };
        let commit = |msg: &str| {
            let mut index = repo.index().expect("index");
            index
                .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
                .expect("add");
            index.write().expect("write index");
            let tree = repo
                .find_tree(index.write_tree().expect("tree"))
                .expect("find tree");
            let sig = git2::Signature::now("T", "t@e.invalid").expect("sig");
            let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
                .expect("commit");
        };
        write("A-1.md", "done");
        write("A-2.md", "todo");
        commit("before the release");
        let sig = git2::Signature::now("T", "t@e.invalid").expect("sig");
        let head = repo.head().expect("head").peel_to_commit().expect("commit");
        repo.tag("v0.3.0", head.as_object(), &sig, "release", false)
            .expect("tag");
        // A tag that is *not* a release, on a later commit, to prove the pattern
        // is doing work rather than the walk finding whatever is nearest.
        write("A-2.md", "done");
        commit("the work");
        let head = repo.head().expect("head").peel_to_commit().expect("commit");
        repo.tag(
            "nightly-2026-09-23",
            head.as_object(),
            &sig,
            "not a release",
            false,
        )
        .expect("tag");

        let handle = Repo::discover(dir.path()).expect("discover");
        let plan = aplan_core::load::load_plan(&dir.path().join("plan/tasks"));
        let window =
            release_window(&handle, dir.path(), &plan, "v*").expect("the window is readable");
        assert_eq!(
            window.since.as_ref().map(|t| t.name.as_str()),
            Some("v0.3.0"),
            "the nightly tag is nearer and is not a release"
        );
        assert_eq!(
            window.closed,
            vec!["A-2".to_string()],
            "A-1 was already done at the tag"
        );
    }
}
