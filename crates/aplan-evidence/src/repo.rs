//! A thin, typed wrapper over the parts of `git2` the resolvers need.
//!
//! `git2` is built with `default-features = false`: `aplan verify` reads local
//! objects (revparse, tree lookup, tree-to-tree diff) and never speaks a network
//! transport, so the `https`/`ssh` features are dead weight, and leaving them
//! out keeps "the deterministic path touches no network" true at the link level.

use std::path::{Path, PathBuf};

use git2::{Commit, Repository};

use crate::Error;

/// What a breadth measurement found, and what it could not measure.
///
/// `sampled` is load-bearing: zero commits sampled is not a row of zeroes, it is
/// no measurement at all, and a caller that cannot tell the two apart prints a
/// table of confident zeroes for a repository it never read (`A1-151`'s audit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breadth {
    /// `(id, hits)` for each task whose `paths` compiled, in the order given.
    pub counts: Vec<(String, usize)>,
    /// Ids whose `paths` did not compile, so nothing was measured for them.
    pub unmeasurable: Vec<String>,
    /// Commits actually walked, which is never more than the history holds.
    pub sampled: usize,
}

/// The repository `aplan verify` resolves evidence against.
pub struct Repo {
    inner: Repository,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repo")
            .field("path", &self.inner.path())
            .finish()
    }
}

impl Repo {
    /// Discover the repository containing `root`, walking up as git itself does.
    pub fn discover(root: &Path) -> Result<Self, Error> {
        let inner =
            Repository::discover(root).map_err(|_| Error::NotARepository(root.to_path_buf()))?;
        Ok(Self { inner })
    }

    /// Whether this clone was made with a depth limit, so most of history is
    /// absent rather than missing.
    ///
    /// `actions/checkout` defaults to `fetch-depth: 1`, which is the common way
    /// to meet this: git answers every `commit` citation with "no such object"
    /// and it is telling the truth about the clone rather than about the plan
    /// (`A1-177`).
    #[must_use]
    pub fn is_shallow(&self) -> bool {
        self.inner.is_shallow()
    }

    /// Whether this commit's history is cut off here, so its diff cannot be
    /// computed.
    ///
    /// A commit records how many parents it has; a shallow clone stops fetching
    /// at some point, so the boundary commit records a parent that is not in the
    /// object store. `commit_paths` treats a missing parent as "a root commit,
    /// diff against the empty tree", which for a *root* is right and for a
    /// boundary makes the diff the entire tree -- so `APLAN012` passed for any
    /// path at all, in a shallow clone, on a citation a full clone rejects
    /// (`A1-177`, found by its audit).
    #[must_use]
    pub fn is_truncated_at(&self, commit: &Commit<'_>) -> bool {
        // `.git/shallow`, which is git's own record of where the history stops.
        // Not `parent_count()`: libgit2 applies the graft, so a boundary commit
        // reports zero parents and is indistinguishable from a root by that
        // route -- which is how the first version of this check silently did
        // nothing.
        let Ok(listed) = std::fs::read_to_string(self.inner.path().join("shallow")) else {
            return false;
        };
        let id = commit.id().to_string();
        listed.lines().any(|l| l.trim() == id)
    }

    /// Resolve a revspec (a SHA, an abbreviated SHA, a ref, a tag, ...) to a
    /// commit. `None` when it names nothing in this repository.
    pub fn resolve_commit(&self, spec: &str) -> Option<Commit<'_>> {
        self.inner.revparse_single(spec).ok()?.peel_to_commit().ok()
    }

    /// The full SHA of `HEAD`, for `aplan close` to record the commit just made.
    /// `None` on an unborn branch (a repo with no commits yet).
    pub fn head_sha(&self) -> Option<String> {
        Some(
            self.inner
                .head()
                .ok()?
                .peel_to_commit()
                .ok()?
                .id()
                .to_string(),
        )
    }

    /// The repo-relative paths a commit's diff touches, against its first parent
    /// (or the empty tree, for a root commit). Sorted and deduplicated so the
    /// result is deterministic.
    /// Every path the tree at `commit` contains, for `APLAN019`'s judgement of a
    /// closed task.
    ///
    /// A `done` task describes history: its `paths` were an ownership claim about
    /// the code as it stood when the work happened. Judging them against `HEAD`
    /// alone reported a task whose code was later deleted as carrying a dead
    /// glob, and advised correcting a glob that had been right all along
    /// (`A1-022`).
    pub fn tree_paths(&self, commit: &Commit<'_>) -> Result<Vec<PathBuf>, Error> {
        let mut paths = Vec::new();
        commit
            .tree()?
            .walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
                // `Ok` rather than `Some` since git2 0.21, where the name
                // became a `Result` whose `Err` is what the old `None` meant:
                // a path git holds as bytes that are not UTF-8.
                if entry.kind() == Some(git2::ObjectType::Blob) {
                    if let Ok(name) = entry.name() {
                        paths.push(PathBuf::from(format!("{dir}{name}")));
                    }
                }
                git2::TreeWalkResult::Ok
            })?;
        Ok(paths)
    }

    pub fn commit_paths(&self, commit: &Commit<'_>) -> Result<Vec<PathBuf>, Error> {
        let new_tree = commit.tree()?;
        let old_tree = match commit.parent(0) {
            Ok(parent) => Some(parent.tree()?),
            Err(_) => None, // a root commit is diffed against the empty tree
        };
        let diff = self
            .inner
            .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;

        let mut paths = Vec::new();
        for delta in diff.deltas() {
            // Both sides, so a rename or delete still counts as touching the path.
            for side in [delta.new_file().path(), delta.old_file().path()]
                .into_iter()
                .flatten()
            {
                paths.push(side.to_path_buf());
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// Whether `path` names a committed *file* (a blob) in the tree of `rev` (or
    /// `HEAD`, when `rev` is `None`). A rev that does not resolve is reported as
    /// "does not exist there" rather than an error, the citation is what is wrong.
    ///
    /// A tree entry for a *directory* resolves fine but is not a committed
    /// artefact, so `file: crates` must not pass `APLAN015`; only a blob (a real
    /// file, including a committed symlink) counts.
    /// Every path the index tracks, for `APLAN019`.
    ///
    /// The index rather than `HEAD`'s tree, so a file that is staged but not yet
    /// committed counts as tracked: an author who has just written the code a
    /// task owns should not be told the glob is dead.
    /// Per task, how many of the last `sample` commits touch its `paths`.
    ///
    /// Counts are in the order given, alongside how many commits were actually
    /// sampled -- a repository shorter than `sample` is common, and a percentage
    /// against a number the caller assumed would be wrong. A repository with no
    /// commits samples zero, and that is not a count of zero: it is nothing to
    /// count against, which the caller has to be able to tell apart.
    ///
    /// A task whose `paths` do not compile is returned as unmeasurable rather
    /// than as zero. `APLAN032` already reports the broken glob from the
    /// hermetic side and this is not the place to raise it again (`A1-066`), but
    /// a zero here would be this measurement *claiming* no recent commit touches
    /// those paths, which it never checked.
    ///
    /// # Errors
    ///
    /// [`Error`] when the walk or a diff fails.
    pub fn commit_breadth(
        &self,
        tasks: &[(String, Vec<String>)],
        sample: usize,
    ) -> Result<Breadth, Error> {
        let mut measurable: Vec<(&String, globset::GlobSet)> = Vec::new();
        let mut unmeasurable: Vec<String> = Vec::new();
        for (id, paths) in tasks {
            match crate::resolve::globset_for(paths) {
                Some(set) => measurable.push((id, set)),
                None => unmeasurable.push(id.clone()),
            }
        }
        let mut hits = vec![0usize; measurable.len()];

        let Some(head) = self.inner.head().ok().and_then(|h| h.peel_to_commit().ok()) else {
            return Ok(Breadth {
                counts: Vec::new(),
                unmeasurable,
                sampled: 0,
            });
        };
        let mut walk = self.inner.revwalk()?;
        walk.push(head.id())?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let mut sampled = 0;
        for oid in walk.take(sample) {
            let commit = self.inner.find_commit(oid?)?;
            // The same function `APLAN012` resolves a citation with, so this
            // measurement cannot drift from the check it describes: one
            // definition of what a commit touched, not two that agree today.
            let touched = self.commit_paths(&commit)?;
            sampled += 1;
            for (n, (_, set)) in measurable.iter().enumerate() {
                if touched.iter().any(|p| set.is_match(p)) {
                    hits[n] += 1;
                }
            }
        }
        Ok(Breadth {
            counts: measurable
                .into_iter()
                .zip(hits)
                .map(|((id, _), n)| (id.clone(), n))
                .collect(),
            unmeasurable,
            sampled,
        })
    }

    /// The newest commit time per path, and `HEAD`'s own commit time.
    ///
    /// Walks `HEAD` once in topological, newest-first order and diffs each
    /// commit against its first parent. A path drops out of the wanted set the
    /// moment it is seen, so the walk can stop early once every path is
    /// accounted for -- which on a long history is most of the saving.
    ///
    /// A path git has never seen is simply absent from the map: an untracked
    /// task file has no history to have sat idle for, and inventing a date for
    /// it would be the report making something up.
    ///
    /// # Errors
    ///
    /// [`Error`] when the walk or a diff fails.
    pub fn last_touched(
        &self,
        paths: &[PathBuf],
    ) -> Result<(std::collections::BTreeMap<PathBuf, i64>, i64), Error> {
        let mut wanted: std::collections::BTreeSet<PathBuf> = paths.iter().cloned().collect();
        let mut seen: std::collections::BTreeMap<PathBuf, i64> = std::collections::BTreeMap::new();

        // A repository with no commits has no history to read, and that is an
        // answer rather than an error: nothing has sat idle yet.
        let Some(head) = self.inner.head().ok().and_then(|h| h.peel_to_commit().ok()) else {
            return Ok((seen, 0));
        };
        let head_time = head.time().seconds();

        let mut walk = self.inner.revwalk()?;
        walk.push(head.id())?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        for oid in walk {
            if wanted.is_empty() {
                break;
            }
            let commit = self.inner.find_commit(oid?)?;
            let time = commit.time().seconds();
            let tree = commit.tree()?;
            let parent = commit.parent(0).ok().and_then(|p| p.tree().ok());
            let diff = self
                .inner
                .diff_tree_to_tree(parent.as_ref(), Some(&tree), None)?;
            for delta in diff.deltas() {
                for path in [delta.new_file().path(), delta.old_file().path()]
                    .into_iter()
                    .flatten()
                {
                    let path = path.to_path_buf();
                    if wanted.remove(&path) {
                        seen.insert(path, time);
                    }
                }
            }
        }
        Ok((seen, head_time))
    }

    pub fn tracked_paths(&self) -> Result<Vec<PathBuf>, Error> {
        let index = self.inner.index()?;
        Ok(index
            .iter()
            .filter_map(|e| {
                std::str::from_utf8(&e.path)
                    .ok()
                    .map(|p| PathBuf::from(p.to_string()))
            })
            .collect())
    }

    /// A tracked file's text at a rev, or `None` when it is absent or not UTF-8.
    ///
    /// Read at the rev rather than off disk, because a citation is checked
    /// against what the repository holds: an uncommitted edit to the proof must
    /// not make a citation resolve for one person and fail for the next
    /// (`A1-159`).
    ///
    /// # Errors
    ///
    /// [`Error`] when the commit or tree cannot be read.
    pub fn text_at(&self, rev: Option<&str>, path: &str) -> Result<Option<String>, Error> {
        let commit = match rev {
            Some(r) => match self.resolve_commit(r) {
                Some(c) => c,
                None => return Ok(None),
            },
            None => self.inner.head()?.peel_to_commit()?,
        };
        let Ok(entry) = commit.tree()?.get_path(Path::new(path)) else {
            return Ok(None);
        };
        let Ok(blob) = entry.to_object(&self.inner).and_then(|o| o.peel_to_blob()) else {
            return Ok(None);
        };
        Ok(String::from_utf8(blob.content().to_vec()).ok())
    }

    pub fn path_exists_at(&self, rev: Option<&str>, path: &str) -> Result<bool, Error> {
        let commit = match rev {
            Some(r) => match self.resolve_commit(r) {
                Some(c) => c,
                None => return Ok(false),
            },
            None => self.inner.head()?.peel_to_commit()?,
        };
        let entry = commit.tree()?.get_path(Path::new(path)).ok();
        Ok(matches!(
            entry.and_then(|e| e.kind()),
            Some(git2::ObjectType::Blob)
        ))
    }
}
