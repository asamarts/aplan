//! A thin, typed wrapper over the parts of `git2` the resolvers need.
//!
//! `git2` is built with `default-features = false`: `aplan verify` reads local
//! objects (revparse, tree lookup, tree-to-tree diff) and never speaks a network
//! transport, so the `https`/`ssh` features are dead weight, and leaving them
//! out keeps "the deterministic path touches no network" true at the link level.

use std::path::{Path, PathBuf};

use git2::{Commit, Repository};

use crate::Error;

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
