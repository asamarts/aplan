//! A thin, typed wrapper over the parts of `git2` the repository consumers need.
//!
//! `git2` is built with `default-features = false`: `aplan verify` reads local
//! objects (revparse, tree lookup, tree-to-tree diff) and never speaks a network
//! transport, so the `https`/`ssh` features are dead weight, and leaving them
//! out keeps "the deterministic path touches no network" true at the link level.
//!
//! This is deliberately a concrete Git facade, not a generic VCS trait
//! (`A1-201`). Every adopter measured so far uses Git-compatible storage (which
//! includes `jj` repositories with a Git backend), while a trait for Mercurial
//! or Fossil would make the verification ladder carry unexercised abstractions.
//! Keeping Git behind one owned-value boundary still leaves that refactor
//! possible if an adopter supplies the missing use case; it does not pretend the
//! use case already exists.

use std::cell::RefCell;
use std::collections::BTreeMap;
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

/// A release tag, and the commit it points at (`A1-313`, ADR-0038).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTag {
    /// The tag's short name, as `git tag` prints it.
    pub name: String,
    /// The full SHA of the commit it peels to.
    pub commit: String,
}

/// A resolved commit and its relationship to another commit (`A1-201`).
///
/// Owned strings keep `git2`'s lifetime-bearing types behind [`Repo`]. A caller
/// that needs only an attestable fact should not acquire a second route to the
/// repository just to ask whether one commit is in another's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommit {
    /// The full object id the revision resolved to.
    pub sha: String,
    /// Whether the commit it was compared with is this commit or a descendant.
    pub reachable: bool,
}

/// Whether the verified repository's `HEAD` reaches a locally resolved commit.
///
/// A shallow checkout has a third answer: libgit2 can prove reachability along
/// the history it has, but a negative walk may only mean that the connecting
/// history was not fetched. Keeping that distinct prevents a checkout defect
/// from being blamed on a task (`A1-177`, `A1-572`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeadReachability {
    Reachable,
    Unreachable,
    UnknownShallow,
}

/// The committed state an attestation binds to (`A1-201`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadTree {
    /// The full object id of `HEAD`.
    pub commit: String,
    /// Git's identity for the complete root tree, including modes and names
    /// that cannot be represented as UTF-8 plan paths.
    pub tree: String,
    /// Every UTF-8 path in that tree mapped to its blob object id.
    pub blobs: BTreeMap<String, String>,
}

/// Task-file candidates below one directory in a committed tree (`A1-194`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedPlanFiles {
    /// The full SHA the caller's revision resolved to.
    pub commit: String,
    /// Whether `root` exists as a tree at that commit.
    pub root_found: bool,
    /// Candidate Markdown bytes keyed by paths relative to `root`, in lexical
    /// order. Non-plan blobs are never read into memory.
    pub files: BTreeMap<PathBuf, Vec<u8>>,
}

/// How the work tree differs from `HEAD` (`A1-201`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorktreeChanges {
    /// Tracked paths with an index or work-tree change.
    pub modified: usize,
    /// Untracked, non-ignored paths.
    pub untracked: usize,
}

/// One deterministic Git status entry used to identify an observed work tree.
///
/// Paths remain raw repository-relative bytes. Converting them to UTF-8 for an
/// identity would make two different non-UTF-8 paths indistinguishable or omit
/// them entirely, even though either can change repository-backed reports.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorktreeStatusEntry {
    pub path: Vec<u8>,
    pub status_bits: u32,
}

/// The counts shown to people and the exact ordered entries behind them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeStatus {
    pub changes: WorktreeChanges,
    pub entries: Vec<WorktreeStatusEntry>,
}

/// One repository path changed across an incremental-refresh boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChangedPath {
    /// UTF-8 repository-relative path. Non-UTF-8 paths make selection refuse.
    pub path: String,
    /// True when this side of the delta did not exist at the base revision.
    /// An unmatched introduced path widens refresh rather than being guessed
    /// unrelated.
    pub introduced: bool,
}

fn detect_renames(diff: &mut git2::Diff<'_>) -> Result<(), String> {
    let mut options = git2::DiffFindOptions::new();
    options.renames(true).renames_from_rewrites(true);
    diff.find_similar(Some(&mut options))
        .map_err(|error| format!("detecting renamed paths: {error}"))
}

fn changed_paths(diff: &git2::Diff<'_>) -> Result<Vec<ChangedPath>, String> {
    let mut paths = BTreeMap::<String, bool>::new();
    for delta in diff.deltas() {
        let mut add = |bytes: Option<&[u8]>, introduced: bool| -> Result<(), String> {
            let Some(bytes) = bytes.filter(|bytes| !bytes.is_empty()) else {
                return Ok(());
            };
            let path = std::str::from_utf8(bytes).map_err(|_| {
                "a changed repository path is not UTF-8; incremental selection cannot represent it and refuses"
                    .to_string()
            })?;
            paths
                .entry(path.to_string())
                .and_modify(|was_introduced| *was_introduced |= introduced)
                .or_insert(introduced);
            Ok(())
        };
        match delta.status() {
            git2::Delta::Added | git2::Delta::Untracked | git2::Delta::Copied => {
                add(delta.new_file().path_bytes(), true)?;
            }
            git2::Delta::Deleted => add(delta.old_file().path_bytes(), false)?,
            git2::Delta::Renamed => {
                add(delta.old_file().path_bytes(), false)?;
                add(delta.new_file().path_bytes(), true)?;
            }
            git2::Delta::Ignored | git2::Delta::Unmodified => {}
            git2::Delta::Modified
            | git2::Delta::Typechange
            | git2::Delta::Unreadable
            | git2::Delta::Conflicted => {
                let bytes = delta.new_file().path_bytes();
                if bytes.is_none_or(<[u8]>::is_empty) {
                    add(delta.old_file().path_bytes(), false)?;
                } else {
                    add(bytes, false)?;
                }
            }
        }
    }
    Ok(paths
        .into_iter()
        .map(|(path, introduced)| ChangedPath { path, introduced })
        .collect())
}

impl WorktreeChanges {
    /// Whether the work tree names exactly the committed state.
    #[must_use]
    pub fn is_clean(self) -> bool {
        self.modified == 0 && self.untracked == 0
    }
}

impl ReleaseTag {
    /// The version this tag names, if its name spells one this tool understands.
    ///
    /// One leading `v` is dropped because that is the convention the default
    /// pattern is written for; everything else is the project's own spelling and
    /// gets no answer rather than a guess, which is ADR-0034's refusal about
    /// `CalVer` arriving one layer out.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        let bare = self.name.strip_prefix('v').unwrap_or(&self.name);
        aplan_core::version::triple(bare).map(|_| bare)
    }
}

/// The repository `aplan verify` resolves evidence against.
pub struct Repo {
    inner: Repository,
    /// Ancestry is asked once by L1 and again by each secondary commit rule.
    /// Cache it by both observed tips and clone completeness so one verification
    /// does not walk the same history several times per citation (`A1-572`).
    head_reachability: RefCell<BTreeMap<(String, String, bool), HeadReachability>>,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repo")
            .field("path", &self.inner.path())
            .field(
                "cached_head_reachability",
                &self.head_reachability.borrow().len(),
            )
            .finish()
    }
}

impl Repo {
    /// Discover the repository containing `root`, walking up as git itself does.
    pub fn discover(root: &Path) -> Result<Self, Error> {
        let inner =
            Repository::discover(root).map_err(|_| Error::NotARepository(root.to_path_buf()))?;
        Ok(Self {
            inner,
            head_reachability: RefCell::new(BTreeMap::new()),
        })
    }

    /// Canonical Git administrative directory shared by every linked worktree.
    ///
    /// This is Git's own repository boundary, not a path inferred from a
    /// worktree's `.git` spelling. Local coordination state belongs here so
    /// separate worktree files cannot each claim the same task independently.
    #[must_use]
    pub fn common_dir(&self) -> &Path {
        self.inner.commondir()
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

    /// Establish whether `commit` belongs to the history rooted at `HEAD`.
    ///
    /// Equality is explicit because libgit2's descendant predicate is strict.
    /// In a shallow checkout, a false or failed graph walk is unknown rather
    /// than evidence that two locally present objects are unrelated.
    #[must_use]
    pub(crate) fn head_reachability(&self, commit: &Commit<'_>) -> HeadReachability {
        let Some(head) = self.resolve_commit("HEAD") else {
            return HeadReachability::Unreachable;
        };
        let shallow = self.is_shallow();
        let key = (head.id().to_string(), commit.id().to_string(), shallow);
        if let Some(answer) = self.head_reachability.borrow().get(&key) {
            return *answer;
        }
        let answer = if head.id() == commit.id() {
            HeadReachability::Reachable
        } else {
            match self.inner.graph_descendant_of(head.id(), commit.id()) {
                Ok(true) => HeadReachability::Reachable,
                Ok(false) | Err(_) if shallow => HeadReachability::UnknownShallow,
                Ok(false) | Err(_) => HeadReachability::Unreachable,
            }
        };
        self.head_reachability.borrow_mut().insert(key, answer);
        answer
    }

    /// Resolve a revspec to its full commit id without leaking a git lifetime.
    #[must_use]
    pub fn resolve_revision(&self, spec: &str) -> Option<String> {
        Some(self.resolve_commit(spec)?.id().to_string())
    }

    /// Read task-file candidates below `root` at `spec`, without checking out or
    /// mutating the repository (`A1-194`).
    ///
    /// `None` means the revision does not resolve. A resolved revision whose
    /// root is absent (or is a file) returns `root_found: false`, which keeps an
    /// absent plan distinct from a valid empty plan. Paths are relative to the
    /// requested root and file bytes are exact; non-candidate blobs are not
    /// loaded, and the plan loader owns content UTF-8 errors. A gitlink is
    /// refused because its files are not part of the selected repository tree.
    pub fn plan_files_at(
        &self,
        spec: &str,
        root: &Path,
    ) -> Result<Option<CommittedPlanFiles>, Error> {
        use std::path::Component;

        if root.is_absolute()
            || root.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(Error::InvalidRepositoryPath(root.to_path_buf()));
        }
        let Some(commit) = self.resolve_commit(spec) else {
            return Ok(None);
        };
        let commit_id = commit.id().to_string();
        let top = commit.tree()?;
        let (tree, selected_root) = if root.as_os_str().is_empty() || root == Path::new(".") {
            (top.clone(), PathBuf::new())
        } else {
            let mut selected = root.to_path_buf();
            let mut found = None;
            let mut followed_link = false;
            for _ in 0..40 {
                // A committed link such as `plan -> .` resolves to the top
                // tree. `Tree::get_path("")` does not name that tree, so make
                // the repository-root case explicit instead of reporting the
                // present link as an absent plan (`A1-498`).
                if selected.as_os_str().is_empty() || selected == Path::new(".") {
                    found = Some(top.clone());
                    break;
                }
                if let Ok(entry) = top.get_path(&selected) {
                    match entry.kind() {
                        Some(git2::ObjectType::Tree) => {
                            found = Some(entry.to_object(&self.inner)?.peel_to_tree()?);
                            break;
                        }
                        Some(git2::ObjectType::Commit) => {
                            return Err(Error::RepositorySubmodule(selected));
                        }
                        _ => {}
                    }
                }
                let Some(next) = symlink_redirect(&self.inner, &top, &selected)? else {
                    // Once a committed link redirected this lookup, a missing
                    // target is invalid historical input, not evidence that
                    // the original root was absent. Keep a direct missing (or
                    // non-directory) root as `root_found: false` (`A1-498`).
                    if followed_link && top.get_path(&selected).is_err() {
                        return Err(Error::InvalidRepositorySymlink(root.to_path_buf()));
                    }
                    return Ok(Some(CommittedPlanFiles {
                        commit: commit_id,
                        root_found: false,
                        files: BTreeMap::new(),
                    }));
                };
                followed_link = true;
                selected = next;
            }
            (
                found.ok_or_else(|| Error::RepositorySymlinkLoop(root.to_path_buf()))?,
                selected,
            )
        };

        let mut files = BTreeMap::new();
        collect_tree_files(
            &self.inner,
            &top,
            &tree,
            &selected_root,
            Path::new(""),
            &mut files,
        )?;
        Ok(Some(CommittedPlanFiles {
            commit: commit_id,
            root_found: true,
            files,
        }))
    }

    /// The bytes a checkout would expose at `path`, following only symbolic
    /// links committed in the selected tree (`A1-194`).
    ///
    /// This differs deliberately from [`Self::blob_at`], which returns the
    /// exact blob stored *at* a path and therefore returns a symlink's target
    /// text. A historical filesystem input such as `.aplan.yml` needs the file
    /// the committed link selects, while evidence fingerprinting needs the
    /// exact Git object. Links escaping the repository are refused.
    pub fn checkout_blob_at(&self, spec: &str, path: &Path) -> Result<Option<Vec<u8>>, Error> {
        use std::path::Component;

        if path.is_absolute()
            || path.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(Error::InvalidRepositoryPath(path.to_path_buf()));
        }
        let Some(commit) = self.resolve_commit(spec) else {
            return Ok(None);
        };
        let top = commit.tree()?;
        if let Ok(entry) = top.get_path(path)
            && entry.kind() != Some(git2::ObjectType::Blob)
        {
            return Err(Error::RepositoryNotFile(path.to_path_buf()));
        }
        let begins_with_link = symlink_redirect(&self.inner, &top, path)?.is_some();
        let content = blob_following_symlinks(&self.inner, &top, path)?;
        if content.is_none() && begins_with_link {
            return Err(Error::InvalidRepositorySymlink(path.to_path_buf()));
        }
        Ok(content)
    }

    /// Whether `spec` was resolved as an object id rather than as a reference.
    ///
    /// Comparing the resulting object id with the spelling is not sufficient:
    /// a branch can be named after the prefix of the commit it currently
    /// targets. `revparse_ext` carries the selected reference separately, so a
    /// moving ref remains distinguishable even in that collision.
    #[must_use]
    pub fn resolves_as_commit_id(&self, spec: &str) -> bool {
        let Ok((object, reference)) = self.inner.revparse_ext(spec) else {
            return false;
        };
        reference.is_none()
            && object.peel_to_commit().is_ok_and(|commit| {
                commit
                    .id()
                    .to_string()
                    .starts_with(&spec.to_ascii_lowercase())
            })
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

    /// `HEAD` and every blob it contains, read in one tree walk (`A1-201`).
    ///
    /// The map is ordered so a consumer cannot accidentally make a provenance
    /// document depend on libgit2's traversal order. Non-UTF-8 paths are skipped,
    /// matching every path-facing operation in this facade: the plan model can
    /// cite only UTF-8 text.
    pub fn head_tree(&self) -> Result<HeadTree, Error> {
        let head = self.inner.head()?.peel_to_commit()?;
        let mut blobs = BTreeMap::new();
        let tree = head.tree()?;
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob)
                && let Ok(name) = entry.name()
            {
                blobs.insert(format!("{dir}{name}"), entry.id().to_string());
            }
            git2::TreeWalkResult::Ok
        })?;
        Ok(HeadTree {
            commit: head.id().to_string(),
            tree: tree.id().to_string(),
            blobs,
        })
    }

    /// Resolve `spec` and say whether `descendant` reaches it (`A1-201`).
    ///
    /// `None` means the object is absent from this checkout. That distinction is
    /// particularly important in a shallow clone: [`Repo::is_shallow`] tells the
    /// evidence ladder why it is absent, while this method reports only the fact
    /// an attestation can reproduce. `descendant` is explicit so a caller can
    /// bind several lookups to one snapshot even if a branch moves concurrently.
    /// Keeping both answers on this facade prevents attestation from growing a
    /// subtly different rev-parse implementation.
    #[must_use]
    pub fn resolve_from(&self, descendant: &str, spec: &str) -> Option<ResolvedCommit> {
        let descendant = self.resolve_commit(descendant)?;
        let commit = self.resolve_commit(spec)?;
        let reachable = commit.id() == descendant.id()
            || self
                .inner
                .graph_descendant_of(descendant.id(), commit.id())
                .unwrap_or(false);
        Some(ResolvedCommit {
            sha: commit.id().to_string(),
            reachable,
        })
    }

    /// Count tracked changes and untracked, non-ignored paths (`A1-201`).
    pub fn worktree_changes(&self) -> Result<WorktreeChanges, Error> {
        Ok(self.worktree_status()?.changes)
    }

    /// Observe the exact ordered work-tree status without reading file bytes.
    ///
    /// This is intentionally cheaper than rebuilding repository-backed report
    /// projections. A consumer can hash the raw path and status-bit pairs to
    /// notice staged, tracked and untracked transitions without inheriting
    /// libgit2's iteration order.
    pub fn worktree_status(&self) -> Result<WorktreeStatus, Error> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);
        let statuses = self.inner.statuses(Some(&mut opts))?;
        let mut changes = WorktreeChanges::default();
        let mut entries = Vec::new();
        for entry in statuses.iter() {
            let status = entry.status();
            if status.is_empty() {
                continue;
            }
            if status.contains(git2::Status::WT_NEW) {
                changes.untracked += 1;
            } else {
                changes.modified += 1;
            }
            entries.push(WorktreeStatusEntry {
                path: entry.path_bytes().to_vec(),
                status_bits: status.bits(),
            });
        }
        entries.sort();
        Ok(WorktreeStatus { changes, entries })
    }

    /// Whether Git's ignore rules exclude one repository-relative path.
    /// Incremental-refresh outputs may live inside a checkout only when Git
    /// itself excludes them from the change boundary.
    pub fn path_is_ignored(&self, path: &Path) -> Result<bool, String> {
        self.inner
            .status_should_ignore(path)
            .map_err(|error| format!("checking whether `{}` is ignored: {error}", path.display()))
    }

    /// Derive added, deleted, renamed and modified paths between two revisions.
    ///
    /// The base must be reachable from the head. Missing or shallow history and
    /// paths the plan's UTF-8 glob vocabulary cannot represent are refusals.
    pub fn changed_paths_between(
        &self,
        base: &str,
        head: &str,
    ) -> Result<(String, String, Vec<ChangedPath>), String> {
        let base = self
            .resolve_commit(base)
            .ok_or_else(|| format!("base revision `{base}` is absent from this checkout"))?;
        let head = self
            .resolve_commit(head)
            .ok_or_else(|| format!("head revision `{head}` is absent from this checkout"))?;
        if base.id() != head.id()
            && !self
                .inner
                .graph_descendant_of(head.id(), base.id())
                .map_err(|error| format!("checking base/head ancestry: {error}"))?
        {
            return Err(format!(
                "base `{}` is not an ancestor of head `{}`; selection would omit an unknown history segment",
                base.id(),
                head.id()
            ));
        }
        let base_tree = base
            .tree()
            .map_err(|error| format!("reading base tree: {error}"))?;
        let head_tree = head
            .tree()
            .map_err(|error| format!("reading head tree: {error}"))?;
        let mut diff = self
            .inner
            .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None)
            .map_err(|error| format!("deriving base/head changes: {error}"))?;
        detect_renames(&mut diff)?;
        Ok((
            base.id().to_string(),
            head.id().to_string(),
            changed_paths(&diff)?,
        ))
    }

    /// Derive changes from one committed base through the current index,
    /// tracked work tree and untracked non-ignored files.
    pub fn changed_paths_to_worktree(
        &self,
        base: &str,
    ) -> Result<(String, String, Vec<ChangedPath>), String> {
        let base = self
            .resolve_commit(base)
            .ok_or_else(|| format!("base revision `{base}` is absent from this checkout"))?;
        let head = self
            .resolve_commit("HEAD")
            .ok_or_else(|| "the repository has no HEAD commit".to_string())?;
        if base.id() != head.id()
            && !self
                .inner
                .graph_descendant_of(head.id(), base.id())
                .map_err(|error| format!("checking base/HEAD ancestry: {error}"))?
        {
            return Err(format!(
                "base `{}` is not an ancestor of HEAD `{}`; dirty selection would omit an unknown history segment",
                base.id(),
                head.id()
            ));
        }
        let base_tree = base
            .tree()
            .map_err(|error| format!("reading base tree: {error}"))?;
        let mut options = git2::DiffOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);
        let mut diff = self
            .inner
            .diff_tree_to_workdir_with_index(Some(&base_tree), Some(&mut options))
            .map_err(|error| format!("deriving dirty working-tree changes: {error}"))?;
        detect_renames(&mut diff)?;
        Ok((
            base.id().to_string(),
            head.id().to_string(),
            changed_paths(&diff)?,
        ))
    }

    /// Attach `body` to `target` under `refs/notes/<notes>`.
    ///
    /// Existing notes are never overwritten. The caller chooses the namespace,
    /// but repository discovery, identity lookup, target resolution and the
    /// write itself all stay behind the same seam as repository reads
    /// (`A1-201`). An explicit target is required because a document assembled
    /// from one snapshot must not move when another process advances `HEAD`
    /// before the write (`A1-496`).
    pub fn attach_note(&self, notes: &str, target: &str, body: &str) -> Result<String, Error> {
        let commit = self.inner.revparse_single(target)?.peel_to_commit()?;
        let sig = self.inner.signature()?;
        let notes_ref = format!("refs/notes/{notes}");
        Ok(self
            .inner
            .note(&sig, &sig, Some(&notes_ref), commit.id(), body, false)?
            .to_string())
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
                if entry.kind() == Some(git2::ObjectType::Blob)
                    && let Ok(name) = entry.name()
                {
                    paths.push(PathBuf::from(format!("{dir}{name}")));
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

    /// The commit each path was last changed by, in one walk (`A1-301`).
    ///
    /// The sibling of [`Repo::last_touched`], which answers *when* for staleness
    /// by age. This answers *which commit*, because a live task's premise is read
    /// against the document as it stood when somebody last touched the task, and
    /// that needs a revision rather than a timestamp.
    ///
    /// One walk for every path asked about, not one per task: a plan with nine
    /// hundred live traced tasks would otherwise walk the history nine hundred
    /// times. A path with no commit is absent rather than an error, which is the
    /// answer for a task file that has never been committed -- there is no
    /// baseline to read it against, so nothing can be said about it.
    pub fn last_touched_commit(
        &self,
        paths: &[PathBuf],
    ) -> Result<std::collections::BTreeMap<PathBuf, String>, Error> {
        let mut wanted: std::collections::BTreeSet<PathBuf> = paths.iter().cloned().collect();
        let mut seen: std::collections::BTreeMap<PathBuf, String> =
            std::collections::BTreeMap::new();
        if wanted.is_empty() {
            return Ok(seen);
        }
        let Some(head) = self.inner.head().ok().and_then(|h| h.peel_to_commit().ok()) else {
            return Ok(seen);
        };
        let mut walk = self.inner.revwalk()?;
        walk.push(head.id())?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        for oid in walk {
            if wanted.is_empty() {
                break;
            }
            let oid = oid?;
            let commit = self.inner.find_commit(oid)?;
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
                        seen.insert(path, oid.to_string());
                    }
                }
            }
        }
        Ok(seen)
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

    /// The nearest tag matching `pattern` that is reachable from `HEAD`
    /// (`A1-313`, ADR-0038).
    ///
    /// **Reachability, not recency**, which ADR-0038 decided against this
    /// repository's own tag set: two `archive/` tags here sit on commits that
    /// are not ancestors of `main`, and one of them shares a day with `v0.3.0`.
    /// A date-ordered "latest tag" would have ranked it first had it been cut an
    /// hour later, so the window is found by walking history the way
    /// `git describe` does.
    ///
    /// **A pattern, not every tag.** A repository that tags things other than
    /// releases has to say which tags are releases, and `v*` is a default rather
    /// than a rule (`A1-037`).
    ///
    /// `None` when no matching tag is reachable, which is the honest answer for
    /// a first release and for a shallow clone that cannot see one. The two are
    /// not the same thing, and the caller asks [`Repo::is_shallow`] to tell them
    /// apart.
    ///
    /// # Errors
    /// [`Error`] when the history cannot be walked.
    pub fn newest_release_tag(&self, pattern: &str) -> Result<Option<ReleaseTag>, Error> {
        let Ok(matcher) = globset::Glob::new(pattern).map(|g| g.compile_matcher()) else {
            // An unusable glob matches nothing rather than everything. The
            // config refuses an empty one at load; a malformed one reaching here
            // must not silently widen the window to every tag in the repository.
            return Ok(None);
        };
        // Tag object to commit, because a lightweight tag *is* the commit and an
        // annotated one is an object pointing at it. Several tags on one commit
        // is ordinary -- a re-tag, or a release that shipped nothing new -- so
        // the map holds a list and the tie is broken below.
        let mut tagged: std::collections::BTreeMap<git2::Oid, Vec<String>> =
            std::collections::BTreeMap::new();
        self.inner.tag_foreach(|oid, name| {
            let Ok(name) = std::str::from_utf8(name) else {
                return true;
            };
            let short = name.strip_prefix("refs/tags/").unwrap_or(name);
            if !matcher.is_match(short) {
                return true;
            }
            if let Ok(commit) = self
                .inner
                .find_object(oid, None)
                .and_then(|o| o.peel_to_commit())
            {
                tagged
                    .entry(commit.id())
                    .or_default()
                    .push(short.to_string());
            }
            true
        })?;
        if tagged.is_empty() {
            return Ok(None);
        }
        let Some(head) = self.inner.head().ok().and_then(|h| h.peel_to_commit().ok()) else {
            return Ok(None);
        };
        let mut walk = self.inner.revwalk()?;
        walk.push(head.id())?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        for oid in walk {
            let oid = oid?;
            if let Some(names) = tagged.get(&oid) {
                // **The highest version, not the alphabetically last.** Sorting
                // names puts `v0.9.0` after `v0.10.0`, so a repository that
                // reached a double-digit minor and tagged twice on one commit
                // measured from the older release and answered a version it had
                // already shipped (`A1-313`, audit). A name that spells no
                // version sorts below every one that does, because there is
                // nothing to compare it by.
                let name = names
                    .iter()
                    .max_by(|a, b| {
                        let key =
                            |n: &str| aplan_core::version::triple(n.strip_prefix('v').unwrap_or(n));
                        key(a).cmp(&key(b)).then_with(|| a.cmp(b))
                    })
                    .cloned()
                    .unwrap_or_default();
                return Ok(Some(ReleaseTag {
                    name,
                    commit: oid.to_string(),
                }));
            }
        }
        Ok(None)
    }

    /// The repository's working directory, resolved through any symlink.
    ///
    /// **What a plan path has to be made relative to** (`A1-313`, audit). The
    /// loader's paths are absolute whenever `.aplan.yml` declares a `root`,
    /// because discovery canonicalises before joining; git wants a path relative
    /// to the repository. Stripping the caller's `--repo` instead worked only
    /// where that happened to be spelled the same way, which in this repository
    /// it was -- it declares no `root` at all, so the default relative one hid
    /// the defect from every test and every dogfood run.
    ///
    /// `None` for a bare repository, which has no working tree to be relative
    /// to.
    #[must_use]
    pub fn workdir(&self) -> Option<PathBuf> {
        let dir = self.inner.workdir()?;
        Some(std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()))
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
        Ok(self
            .blob_at(rev, path)?
            .and_then(|content| String::from_utf8(content).ok()))
    }

    /// A tracked file's exact committed bytes at a rev, or `None` when it is
    /// absent. Evidence freshness uses bytes rather than text for a whole-file
    /// citation so a PDF, image or other binary artefact can be stamped too.
    ///
    /// Like [`Self::text_at`], this reads the commit tree and never the working
    /// copy: a dirty edit is not yet the proof `file:path` at `HEAD` cites.
    pub fn blob_at(&self, rev: Option<&str>, path: &str) -> Result<Option<Vec<u8>>, Error> {
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
        Ok(Some(blob.content().to_vec()))
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

/// Preserve one Git name's bytes where the host can represent them. The core
/// plan loader accepts `PathBuf`, not UTF-8 filenames, and silently dropping a
/// task whose *name* is not text makes a historical report differ from a
/// checkout.
// On Unix every byte name is representable; on other hosts the UTF-8 branch
// can refuse one. Clippy sees only the current cfg and otherwise calls the
// cross-platform return type needlessly wrapped.
#[allow(clippy::unnecessary_wraps)]
fn entry_name(entry: &git2::TreeEntry<'_>) -> Option<std::ffi::OsString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        Some(std::ffi::OsString::from_vec(entry.name_bytes().to_vec()))
    }
    #[cfg(not(unix))]
    {
        entry.name().ok().map(std::ffi::OsString::from)
    }
}

/// Recursively collect a tree without libgit2's UTF-8 `Tree::walk` callback.
/// Git path bytes are valid on Unix even when they are not text; the callback
/// aborts before yielding such a directory, while direct tree iteration keeps
/// the exact names available (`A1-194` audit).
fn collect_tree_files(
    repo: &Repository,
    top: &git2::Tree<'_>,
    tree: &git2::Tree<'_>,
    selected_root: &Path,
    dir: &Path,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), Error> {
    for entry in tree {
        let Some(name) = entry_name(&entry) else {
            continue;
        };
        let relative = dir.join(name);
        match entry.kind() {
            Some(git2::ObjectType::Tree) => {
                if !aplan_core::is_plan_directory_path(&relative) {
                    continue;
                }
                let child = repo.find_tree(entry.id())?;
                collect_tree_files(repo, top, &child, selected_root, &relative, files)?;
            }
            Some(git2::ObjectType::Blob) => {
                if !aplan_core::is_plan_file_path(&relative) {
                    continue;
                }
                let content = if entry.filemode() == 0o120_000 {
                    blob_following_symlinks(repo, top, &selected_root.join(&relative))?
                } else {
                    Some(repo.find_blob(entry.id())?.content().to_vec())
                };
                // A broken file link is not a file to the filesystem loader
                // either, so it is absent rather than a synthetic task.
                if let Some(content) = content {
                    files.insert(relative, content);
                }
            }
            Some(git2::ObjectType::Commit) if aplan_core::is_plan_directory_path(&relative) => {
                return Err(Error::RepositorySubmodule(relative));
            }
            _ => {}
        }
    }
    Ok(())
}

/// If a component of `path` is a committed symbolic link, return the target
/// with its remaining suffix. Git stores a symlink as a blob, so `get_path`
/// cannot walk through it by itself; a checkout can, and a revision report must
/// not accidentally follow today's worktree link instead (`A1-194` audit).
fn symlink_redirect(
    repo: &Repository,
    top: &git2::Tree<'_>,
    path: &Path,
) -> Result<Option<PathBuf>, Error> {
    let parts: Vec<_> = path.components().collect();
    let mut prefix = PathBuf::new();
    for (index, component) in parts.iter().enumerate() {
        prefix.push(component.as_os_str());
        let Ok(entry) = top.get_path(&prefix) else {
            return Ok(None);
        };
        if entry.kind() == Some(git2::ObjectType::Tree) {
            continue;
        }
        if entry.kind() != Some(git2::ObjectType::Blob) || entry.filemode() != 0o120_000 {
            return Ok(None);
        }
        let blob = entry.to_object(repo)?.peel_to_blob()?;
        let target = symlink_target(blob.content())
            .ok_or_else(|| Error::InvalidRepositorySymlink(prefix.clone()))?;
        let mut joined = prefix
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(target);
        for remaining in &parts[index + 1..] {
            joined.push(remaining.as_os_str());
        }
        return normalize_repository_path(&joined)
            .map(Some)
            .ok_or(Error::InvalidRepositorySymlink(prefix));
    }
    Ok(None)
}

// The same host-dependent representability as [`entry_name`].
#[allow(clippy::unnecessary_wraps)]
fn symlink_target(bytes: &[u8]) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).ok().map(PathBuf::from)
    }
}

/// Read a blob after following only the symbolic links stored in `top`.
/// Worktree links are never consulted, so a historical task link cannot move
/// when the current checkout does (`A1-194` audit).
fn blob_following_symlinks(
    repo: &Repository,
    top: &git2::Tree<'_>,
    path: &Path,
) -> Result<Option<Vec<u8>>, Error> {
    let mut selected = path.to_path_buf();
    for _ in 0..40 {
        if let Ok(entry) = top.get_path(&selected)
            && entry.kind() == Some(git2::ObjectType::Blob)
            && entry.filemode() != 0o120_000
        {
            return Ok(Some(
                entry.to_object(repo)?.peel_to_blob()?.content().to_vec(),
            ));
        }
        let Some(next) = symlink_redirect(repo, top, &selected)? else {
            return Ok(None);
        };
        selected = next;
    }
    Err(Error::RepositorySymlinkLoop(path.to_path_buf()))
}

/// Normalise a repository-relative path without allowing a symlink target to
/// climb above the tree it is being read from.
fn normalize_repository_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(part) => out.push(part),
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod incremental_change_tests {
    use super::*;

    struct Fixture {
        dir: tempfile::TempDir,
        repo: Repository,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("tempdir");
            let repo = Repository::init(dir.path()).expect("git init");
            Self { dir, repo }
        }

        fn write(&self, path: impl AsRef<Path>, contents: &str) {
            let path = self.dir.path().join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(path, contents).expect("write");
        }

        fn remove(&self, path: impl AsRef<Path>) {
            std::fs::remove_file(self.dir.path().join(path)).expect("remove");
        }

        fn commit_all(&self, message: &str) -> String {
            let mut index = self.repo.index().expect("index");
            index
                .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .expect("stage additions and changes");
            index.update_all(["*"], None).expect("stage deletions");
            index.write().expect("write index");
            let tree_id = index.write_tree().expect("write tree");
            let tree = self.repo.find_tree(tree_id).expect("tree");
            let signature = git2::Signature::now("Test", "test@example.invalid").expect("sig");
            let parent = self
                .repo
                .head()
                .ok()
                .and_then(|head| head.peel_to_commit().ok());
            let parents = parent.iter().collect::<Vec<_>>();
            self.repo
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    message,
                    &tree,
                    &parents,
                )
                .expect("commit")
                .to_string()
        }

        fn discover(&self) -> Repo {
            Repo::discover(self.dir.path()).expect("discover")
        }
    }

    #[test]
    fn committed_boundary_reports_add_modify_delete_and_both_rename_sides() {
        let fixture = Fixture::new();
        fixture.write("modified.txt", "before\n");
        fixture.write("deleted.txt", "delete\n");
        fixture.write("old-name.txt", "rename exactly\n");
        let base = fixture.commit_all("base");

        fixture.write("modified.txt", "after\n");
        fixture.remove("deleted.txt");
        std::fs::rename(
            fixture.dir.path().join("old-name.txt"),
            fixture.dir.path().join("new-name.txt"),
        )
        .expect("rename");
        fixture.write("added.txt", "added\n");
        let head = fixture.commit_all("head");

        let (resolved_base, resolved_head, changed) = fixture
            .discover()
            .changed_paths_between(&base, &head)
            .expect("safe boundary");
        assert_eq!(resolved_base, base);
        assert_eq!(resolved_head, head);
        assert_eq!(
            changed,
            vec![
                ChangedPath {
                    path: "added.txt".into(),
                    introduced: true,
                },
                ChangedPath {
                    path: "deleted.txt".into(),
                    introduced: false,
                },
                ChangedPath {
                    path: "modified.txt".into(),
                    introduced: false,
                },
                ChangedPath {
                    path: "new-name.txt".into(),
                    introduced: true,
                },
                ChangedPath {
                    path: "old-name.txt".into(),
                    introduced: false,
                },
            ]
        );
    }

    #[test]
    fn dirty_boundary_includes_index_worktree_and_untracked_changes() {
        let fixture = Fixture::new();
        fixture.write("tracked.txt", "before\n");
        let base = fixture.commit_all("base");
        fixture.write("tracked.txt", "after\n");
        fixture.write("staged.txt", "staged\n");
        let mut index = fixture.repo.index().expect("index");
        index.add_path(Path::new("staged.txt")).expect("stage");
        index.write().expect("write index");
        fixture.write("untracked.txt", "untracked\n");

        let (_, head, changed) = fixture
            .discover()
            .changed_paths_to_worktree(&base)
            .expect("dirty boundary");
        assert_eq!(head, base);
        assert_eq!(
            changed,
            vec![
                ChangedPath {
                    path: "staged.txt".into(),
                    introduced: true,
                },
                ChangedPath {
                    path: "tracked.txt".into(),
                    introduced: false,
                },
                ChangedPath {
                    path: "untracked.txt".into(),
                    introduced: true,
                },
            ]
        );
    }

    #[test]
    fn missing_or_unrelated_history_refuses_selection() {
        let fixture = Fixture::new();
        fixture.write("tracked.txt", "base\n");
        let base = fixture.commit_all("base");
        fixture.write("tracked.txt", "head\n");
        let head = fixture.commit_all("head");
        let tree = fixture
            .repo
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("commit")
            .tree()
            .expect("tree");
        let signature = git2::Signature::now("Test", "test@example.invalid").expect("sig");
        let unrelated = fixture
            .repo
            .commit(None, &signature, &signature, "unrelated", &tree, &[])
            .expect("side root")
            .to_string();
        let repo = fixture.discover();

        assert!(repo.changed_paths_between("absent", &head).is_err());
        assert!(repo.changed_paths_between(&unrelated, &head).is_err());
        assert!(repo.changed_paths_between(&base, &head).is_ok());
        assert!(repo.changed_paths_to_worktree(&unrelated).is_err());
    }

    #[test]
    fn dirty_boundary_uses_git_ignore_rules_without_hiding_visible_paths() {
        let fixture = Fixture::new();
        fixture.write(".gitignore", "ignored/\n");
        fixture.write("tracked.txt", "base\n");
        let base = fixture.commit_all("base");
        fixture.write("ignored/output.json", "ignored\n");
        fixture.write("visible/output.json", "visible\n");

        let repo = fixture.discover();
        assert!(
            repo.path_is_ignored(Path::new("ignored/output.json"))
                .expect("ignored")
        );
        assert!(
            !repo
                .path_is_ignored(Path::new("visible/output.json"))
                .expect("visible")
        );
        let (_, _, changed) = repo
            .changed_paths_to_worktree(&base)
            .expect("dirty boundary");
        assert_eq!(
            changed,
            vec![ChangedPath {
                path: "visible/output.json".into(),
                introduced: true,
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_changed_path_refuses_selection() {
        use std::os::unix::ffi::OsStringExt as _;

        let fixture = Fixture::new();
        fixture.write("tracked.txt", "base\n");
        let base = fixture.commit_all("base");
        let name = std::ffi::OsString::from_vec(vec![b'n', b'o', b'n', b'-', 0xff]);
        fixture.write(PathBuf::from(name), "untracked\n");

        let error = fixture
            .discover()
            .changed_paths_to_worktree(&base)
            .expect_err("non-UTF-8 path must refuse");
        assert!(error.contains("not UTF-8"), "{error}");
    }
}
