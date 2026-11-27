//! Single-pass bounded Git history inference for legacy `done` tasks without
//! `closed_at` (`A2-110`, ADR-0074).
//!
//! Key guarantees:
//! - Walks commits reachable from the named `basis_revision` at most once
//!   (`O(commits)`), never once per task (`O(tasks * commits)`).
//! - Caches parsed historical task blobs by Git blob OID so identical blobs
//!   across commits are parsed at most once.
//! - Repository-data-only: reads only regular-file blobs inside `plan_root_rel`
//!   directly from the Git object database; never executes repository config
//!   and never follows worktree symlinks.
//! - Recorded `closed_at` values always win; inferred timestamps carry
//!   `history_first_seen` provenance with their selected commit OID and are
//!   never persisted as `closed_at`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::time::Instant;

use aplan_core::{
    ClosedAtSource, CompletionHistoryError, CompletionHistoryReport, CompletionTime,
    HistoryCompleteness, HistoryLimits, HistoryPolicy, Status, Task, format_closed_at,
    parse_canonical_closed_at,
};
use git2::{FileMode, ObjectType, Oid, Repository, Sort, Tree, TreeWalkMode, TreeWalkResult};

/// Parsed minimal task header from one historical Git blob.
#[derive(Debug, Clone)]
enum ParsedBlob {
    Task { id: String, status: Status },
    NotATask,
    Unparseable(String),
    TooLarge,
}

/// Snapshot of a plan subtree at one commit: map from task id -> `(path, blob_oid, status)`.
#[derive(Debug, Clone, Default)]
struct PlanSnapshot {
    /// `task_id -> (repo_rel_path, status)` for all parseable tasks under `plan_root_rel`.
    tasks: BTreeMap<String, (String, Status)>,
    /// `repo_rel_path -> (task_id, status)` for unambiguous rename/rekey detection.
    by_path: BTreeMap<String, (String, Status)>,
}

struct WalkContext<'a> {
    repo: &'a Repository,
    plan_root_prefix: String,
    limits: &'a HistoryLimits,
    blob_cache: HashMap<Oid, ParsedBlob>,
    tree_cache: HashMap<Oid, PlanSnapshot>,
    total_blob_bytes: usize,
    bounded_hit: bool,
    unparseable_files: BTreeSet<String>,
}

impl<'a> WalkContext<'a> {
    fn new(repo: &'a Repository, plan_root_rel: &Path, limits: &'a HistoryLimits) -> Self {
        let mut prefix = plan_root_rel
            .to_string_lossy()
            .replace('\\', "/")
            .trim_matches('/')
            .to_string();
        if prefix == "." {
            prefix.clear();
        }
        Self {
            repo,
            plan_root_prefix: prefix,
            limits,
            blob_cache: HashMap::new(),
            tree_cache: HashMap::new(),
            total_blob_bytes: 0,
            bounded_hit: false,
            unparseable_files: BTreeSet::new(),
        }
    }

    /// Return the Git tree OID of `plan_root_prefix` inside `commit_tree`, if present.
    fn plan_subtree_oid(&self, commit_tree: &Tree<'_>) -> Option<Oid> {
        if self.plan_root_prefix.is_empty() {
            return Some(commit_tree.id());
        }
        let entry = commit_tree
            .get_path(Path::new(&self.plan_root_prefix))
            .ok()?;
        (entry.kind() == Some(ObjectType::Tree)).then(|| entry.id())
    }

    /// Parse all Markdown regular-file blobs inside `subtree_oid` (cached by subtree OID).
    fn snapshot_subtree(&mut self, subtree_oid: Oid) -> PlanSnapshot {
        if let Some(cached) = self.tree_cache.get(&subtree_oid) {
            return cached.clone();
        }
        let Ok(subtree) = self.repo.find_tree(subtree_oid) else {
            return PlanSnapshot::default();
        };
        let mut entries: Vec<(String, Oid)> = Vec::new();
        let _ = subtree.walk(TreeWalkMode::PreOrder, |dir, entry| {
            let Ok(name) = entry.name() else {
                return TreeWalkResult::Ok;
            };
            // Prune reserved `.inbox` subtree just like `aplan_core::load`.
            if name == ".inbox" && entry.kind() == Some(ObjectType::Tree) {
                return TreeWalkResult::Skip;
            }
            // Repository-data-only: read only regular file blobs, never symlinks.
            if entry.kind() != Some(ObjectType::Blob) {
                return TreeWalkResult::Ok;
            }
            if !matches!(entry.filemode(), x if x == i32::from(FileMode::Blob) || x == i32::from(FileMode::BlobExecutable))
            {
                return TreeWalkResult::Ok;
            }
            if !Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                return TreeWalkResult::Ok;
            }
            let rel = if dir.is_empty() {
                name.to_string()
            } else {
                format!("{dir}{name}")
            };
            entries.push((rel, entry.id()));
            TreeWalkResult::Ok
        });

        let mut snapshot = PlanSnapshot::default();
        for (rel, blob_oid) in entries {
            match self.parse_blob(blob_oid, &rel) {
                ParsedBlob::Task { id, status } => {
                    snapshot.tasks.insert(id.clone(), (rel.clone(), status));
                    snapshot.by_path.insert(rel, (id, status));
                }
                ParsedBlob::NotATask => {}
                ParsedBlob::Unparseable(reason) => {
                    self.unparseable_files.insert(format!("{rel}: {reason}"));
                }
                ParsedBlob::TooLarge => {
                    self.bounded_hit = true;
                }
            }
        }
        self.tree_cache.insert(subtree_oid, snapshot.clone());
        snapshot
    }

    fn parse_blob(&mut self, blob_oid: Oid, rel: &str) -> ParsedBlob {
        if let Some(cached) = self.blob_cache.get(&blob_oid) {
            return cached.clone();
        }
        let Ok(blob) = self.repo.find_blob(blob_oid) else {
            let res = ParsedBlob::Unparseable("missing blob object".to_string());
            self.blob_cache.insert(blob_oid, res.clone());
            return res;
        };
        let bytes = blob.content();
        if bytes.len() > self.limits.max_blob_bytes {
            let res = ParsedBlob::TooLarge;
            self.blob_cache.insert(blob_oid, res.clone());
            return res;
        }
        if self.total_blob_bytes.saturating_add(bytes.len()) > self.limits.max_total_blob_bytes {
            self.bounded_hit = true;
            return ParsedBlob::TooLarge;
        }
        self.total_blob_bytes += bytes.len();
        let Ok(text) = std::str::from_utf8(bytes) else {
            let res = ParsedBlob::Unparseable(format!("`{rel}` is not valid UTF-8"));
            self.blob_cache.insert(blob_oid, res.clone());
            return res;
        };
        let Some((fm, _body)) = aplan_core::load::split_frontmatter(text) else {
            let res = ParsedBlob::NotATask;
            self.blob_cache.insert(blob_oid, res.clone());
            return res;
        };
        let parsed = match aplan_core::yaml::from_str::<Task>(fm) {
            Ok(task) => ParsedBlob::Task {
                id: task.id,
                status: task.status,
            },
            Err(err) => ParsedBlob::Unparseable(err.to_string()),
        };
        self.blob_cache.insert(blob_oid, parsed.clone());
        parsed
    }
}

/// Resolve completion times for `tasks` using recorded `closed_at` values and
/// (when permitted by `policy`) one bounded Git history pass (`A2-110`,
/// ADR-0074).
///
/// # Errors
/// Returns [`CompletionHistoryError`] when `basis_rev` cannot be resolved or
/// when `policy` is [`HistoryPolicy::Required`] and complete reachable history
/// is unavailable or incomplete.
#[allow(clippy::too_many_lines)]
pub fn resolve_completion_times(
    repo_root: &Path,
    plan_root_rel: &Path,
    tasks: &[&Task],
    basis_rev: Option<&str>,
    policy: HistoryPolicy,
    limits: &HistoryLimits,
) -> Result<CompletionHistoryReport, CompletionHistoryError> {
    let mut resolved = BTreeMap::<String, CompletionTime>::new();
    let mut legacy_ids = BTreeSet::<String>::new();

    for task in tasks {
        if task.status != Status::Done {
            continue;
        }
        if let Some(at) = task.closed_at.as_deref()
            && let Some(epoch) = parse_canonical_closed_at(at)
        {
            let origin = task.closed_at_source.unwrap_or(ClosedAtSource::Local);
            resolved.insert(
                task.id.clone(),
                CompletionTime::recorded(at.to_string(), epoch, origin),
            );
        } else {
            legacy_ids.insert(task.id.clone());
        }
    }

    let opened = Repository::discover(repo_root).ok();
    let resolved_basis = match (&opened, basis_rev) {
        (Some(repo), Some(rev)) => {
            let obj = repo.revparse_single(rev).map_err(|err| {
                CompletionHistoryError::InvalidRevision {
                    revision: rev.to_string(),
                    detail: err.message().to_string(),
                }
            })?;
            let commit =
                obj.peel_to_commit()
                    .map_err(|err| CompletionHistoryError::InvalidRevision {
                        revision: rev.to_string(),
                        detail: err.message().to_string(),
                    })?;
            Some((commit.id(), commit.id().to_string()))
        }
        (Some(repo), None) => repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok())
            .map(|c| (c.id(), format!("working_tree@{}", c.id()))),
        (None, Some(rev)) => {
            return Err(CompletionHistoryError::InvalidRevision {
                revision: rev.to_string(),
                detail: "not a Git repository".to_string(),
            });
        }
        (None, None) => None,
    };

    let basis_revision = resolved_basis
        .as_ref()
        .map_or_else(|| "unversioned".to_string(), |(_, label)| label.clone());

    // Fast path: every requested `done` task already carries a recorded `closed_at`.
    if legacy_ids.is_empty() {
        return Ok(CompletionHistoryReport {
            basis_revision,
            policy,
            completeness: HistoryCompleteness::Complete,
            limitations: Vec::new(),
            commits_walked: 0,
            resolved,
        });
    }

    // `HistoryPolicy::Never`: skip Git completely; legacy tasks stay `unknown`.
    if policy == HistoryPolicy::Never {
        for id in &legacy_ids {
            resolved.insert(id.clone(), CompletionTime::unknown());
        }
        return Ok(CompletionHistoryReport {
            basis_revision,
            policy,
            completeness: HistoryCompleteness::HistorySkipped,
            limitations: vec![
                "Git history inference was disabled (`--history never`); legacy `done` tasks \
                 without `closed_at` remain `unknown`"
                    .to_string(),
            ],
            commits_walked: 0,
            resolved,
        });
    }

    let (Some(repo), Some((basis_oid, _))) = (opened.as_ref(), resolved_basis.as_ref()) else {
        let detail = "not a Git repository or no commits exist at basis revision".to_string();
        if policy == HistoryPolicy::Required {
            return Err(CompletionHistoryError::Unavailable { detail });
        }
        for id in &legacy_ids {
            resolved.insert(id.clone(), CompletionTime::unknown());
        }
        return Ok(CompletionHistoryReport {
            basis_revision,
            policy,
            completeness: HistoryCompleteness::Unavailable,
            limitations: vec![detail],
            commits_walked: 0,
            resolved,
        });
    };

    walk_history(
        repo,
        plan_root_rel,
        *basis_oid,
        basis_revision,
        policy,
        limits,
        &legacy_ids,
        resolved,
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_history(
    repo: &Repository,
    plan_root_rel: &Path,
    basis_oid: Oid,
    basis_revision: String,
    policy: HistoryPolicy,
    limits: &HistoryLimits,
    legacy_ids: &BTreeSet<String>,
    mut resolved: BTreeMap<String, CompletionTime>,
) -> Result<CompletionHistoryReport, CompletionHistoryError> {
    let started = Instant::now();
    let is_shallow = repo.is_shallow();
    let mut ctx = WalkContext::new(repo, plan_root_rel, limits);

    // Inspect the committed plan tree at `basis_oid`. Any legacy task that is
    // NOT `done` at `basis_oid` (for example, an untracked or uncommitted
    // working-tree `done` task) has no committed transition and stays `unknown`.
    let basis_commit =
        repo.find_commit(basis_oid)
            .map_err(|err| CompletionHistoryError::InvalidRevision {
                revision: basis_oid.to_string(),
                detail: err.message().to_string(),
            })?;
    let basis_tree =
        basis_commit
            .tree()
            .map_err(|err| CompletionHistoryError::InvalidRevision {
                revision: basis_oid.to_string(),
                detail: err.message().to_string(),
            })?;
    let basis_done: BTreeSet<String> = match ctx.plan_subtree_oid(&basis_tree) {
        Some(sub_oid) => {
            let snap = ctx.snapshot_subtree(sub_oid);
            legacy_ids
                .iter()
                .filter(|id| {
                    snap.tasks
                        .get(id.as_str())
                        .is_some_and(|(_, st)| *st == Status::Done)
                })
                .cloned()
                .collect()
        }
        None => BTreeSet::new(),
    };

    // Propagate `(current_task_id, tracked_historical_id)` along commits in the
    // contiguous `done` component starting at `basis_oid`. Carrying both ids
    // lets us follow unambiguous rekeys (`OLD -> NEW`) while attributing the
    // final candidate back to `current_task_id`.
    let mut propagate: HashMap<Oid, BTreeSet<(String, String)>> = HashMap::new();
    if !basis_done.is_empty() {
        let initial: BTreeSet<(String, String)> = basis_done
            .iter()
            .map(|id| (id.clone(), id.clone()))
            .collect();
        propagate.insert(basis_oid, initial);
    }

    let mut candidates: BTreeMap<String, BTreeSet<(i64, String)>> = BTreeMap::new();
    let mut commits_walked = 0usize;
    let mut shallow_hit = false;

    if !propagate.is_empty() {
        let mut revwalk = repo
            .revwalk()
            .map_err(|err| CompletionHistoryError::Unavailable {
                detail: err.message().to_string(),
            })?;
        revwalk
            .set_sorting(Sort::TOPOLOGICAL | Sort::TIME)
            .map_err(|err| CompletionHistoryError::Unavailable {
                detail: err.message().to_string(),
            })?;
        revwalk
            .push(basis_oid)
            .map_err(|err| CompletionHistoryError::InvalidRevision {
                revision: basis_oid.to_string(),
                detail: err.message().to_string(),
            })?;

        for oid_res in revwalk {
            if propagate.is_empty() {
                break;
            }
            let Ok(oid) = oid_res else {
                ctx.unparseable_files
                    .insert("revwalk encountered an unreadable commit object".to_string());
                continue;
            };
            let Some(want) = propagate.remove(&oid) else {
                continue;
            };
            if commits_walked >= limits.max_commits
                || limits
                    .max_duration
                    .is_some_and(|limit| started.elapsed() >= limit)
            {
                ctx.bounded_hit = true;
                break;
            }
            commits_walked += 1;

            let Ok(commit) = repo.find_commit(oid) else {
                ctx.unparseable_files
                    .insert(format!("missing commit object `{oid}`"));
                continue;
            };
            let parent_count = commit.parent_count();
            if parent_count == 0 {
                if is_shallow && is_shallow_graft(repo, oid) {
                    shallow_hit = true;
                } else {
                    let ts = commit.time().seconds();
                    let sha = oid.to_string();
                    for (origin_id, _) in want {
                        candidates
                            .entry(origin_id)
                            .or_default()
                            .insert((ts, sha.clone()));
                    }
                }
                continue;
            }

            let Ok(commit_tree) = commit.tree() else {
                ctx.unparseable_files
                    .insert(format!("missing tree for commit `{oid}`"));
                continue;
            };
            let commit_sub = ctx.plan_subtree_oid(&commit_tree);
            let mut parent_subs: Vec<(Oid, Option<Oid>)> = Vec::with_capacity(parent_count);
            for idx in 0..parent_count {
                if let Ok(parent) = commit.parent(idx) {
                    let p_sub = parent.tree().ok().and_then(|t| ctx.plan_subtree_oid(&t));
                    parent_subs.push((parent.id(), p_sub));
                }
            }

            // Fast path: if any parent has the exact same plan subtree OID as
            // `commit`, every tracked task in `want` is unchanged in that
            // parent without opening or parsing any blobs.
            let commit_snap = if parent_subs.iter().any(|(_, ps)| *ps == commit_sub) {
                None
            } else {
                commit_sub.map(|sub| ctx.snapshot_subtree(sub))
            };

            let mut parent_snaps: Vec<(Oid, Option<PlanSnapshot>, bool)> =
                Vec::with_capacity(parent_subs.len());
            for (p_oid, p_sub) in parent_subs {
                if p_sub == commit_sub {
                    parent_snaps.push((p_oid, None, true));
                } else {
                    let snap = p_sub.map(|sub| ctx.snapshot_subtree(sub));
                    parent_snaps.push((p_oid, snap, false));
                }
            }

            for (origin_id, tracked_id) in want {
                let mut done_parents: Vec<(Oid, String)> = Vec::new();
                for (p_oid, p_snap, identical_subtree) in &parent_snaps {
                    if *identical_subtree {
                        done_parents.push((*p_oid, tracked_id.clone()));
                        continue;
                    }
                    let Some(snap) = p_snap else {
                        continue;
                    };
                    if snap
                        .tasks
                        .get(tracked_id.as_str())
                        .is_some_and(|(_, st)| *st == Status::Done)
                    {
                        done_parents.push((*p_oid, tracked_id.clone()));
                        continue;
                    }
                    // Unambiguous 1-to-1 rekey detection: if `tracked_id` was
                    // introduced as `done` in `commit` at a path (or as the
                    // sole newly added `done` id) while one unique `done` id
                    // disappeared from `parent`, follow that predecessor id.
                    if let Some(c_snap) = &commit_snap
                        && let Some(prev_id) = detect_unambiguous_rekey(c_snap, snap, &tracked_id)
                    {
                        done_parents.push((*p_oid, prev_id));
                    }
                }

                if done_parents.is_empty() {
                    // `ADR-0074` section 6: `tracked_id` was NOT `done` in ANY
                    // parent of `commit`, so `commit` is a minimal transition
                    // candidate.
                    candidates
                        .entry(origin_id)
                        .or_default()
                        .insert((commit.time().seconds(), oid.to_string()));
                } else {
                    for (p_oid, next_tracked) in done_parents {
                        propagate
                            .entry(p_oid)
                            .or_default()
                            .insert((origin_id.clone(), next_tracked));
                    }
                }
            }
        }
    }

    for id in legacy_ids {
        if let Some(set) = candidates.get(id)
            && let Some((epoch, sha)) = set.iter().next()
            && let Ok(instant) = format_closed_at(*epoch)
        {
            let incomparable = if set.len() > 1 { set.len() } else { 0 };
            resolved.insert(
                id.clone(),
                CompletionTime::history_first_seen(instant, *epoch, sha.clone(), incomparable),
            );
            continue;
        }
        resolved.insert(id.clone(), CompletionTime::unknown());
    }

    let mut limitations = Vec::new();
    if shallow_hit {
        limitations.push(
            "repository is shallow; history walk reached a shallow graft boundary before \
             resolving all legacy `done` tasks"
                .to_string(),
        );
    }
    if ctx.bounded_hit {
        limitations.push(format!(
            "history walk reached its configured budget (max_commits={}, max_total_blob_bytes={}) \
             before resolving all legacy `done` tasks",
            limits.max_commits, limits.max_total_blob_bytes
        ));
    }
    for bad in &ctx.unparseable_files {
        limitations.push(format!("unparseable historical task file: {bad}"));
    }

    let completeness = if shallow_hit {
        HistoryCompleteness::IncompleteShallow
    } else if ctx.bounded_hit {
        HistoryCompleteness::IncompleteBounded
    } else if !ctx.unparseable_files.is_empty() {
        HistoryCompleteness::IncompleteUnobservable
    } else {
        HistoryCompleteness::Complete
    };

    if policy == HistoryPolicy::Required && completeness != HistoryCompleteness::Complete {
        return Err(CompletionHistoryError::Incomplete {
            basis_revision,
            completeness,
            detail: limitations.join("; "),
        });
    }

    Ok(CompletionHistoryReport {
        basis_revision,
        policy,
        completeness,
        limitations,
        commits_walked,
        resolved,
    })
}

/// Detect an unambiguous 1-to-1 rekey (`OLD -> NEW`) between `parent` and
/// `child` where `tracked_id` is `done` in `child`, absent in `parent`, and
/// either:
/// 1. the exact same file path held `OLD` (`done`) in `parent` and `OLD` is
///    gone from `child`, OR
/// 2. `tracked_id` is the sole newly added `done` task in `child` and exactly
///    one `done` task `OLD` disappeared from `parent`.
fn detect_unambiguous_rekey(
    child: &PlanSnapshot,
    parent: &PlanSnapshot,
    tracked_id: &str,
) -> Option<String> {
    let (child_path, _) = child.tasks.get(tracked_id)?;
    if let Some((old_id, old_status)) = parent.by_path.get(child_path)
        && *old_status == Status::Done
        && old_id != tracked_id
        && !child.tasks.contains_key(old_id.as_str())
    {
        return Some(old_id.clone());
    }
    let added_done: Vec<&str> = child
        .tasks
        .iter()
        .filter(|(id, (_, st))| *st == Status::Done && !parent.tasks.contains_key(id.as_str()))
        .map(|(id, _)| id.as_str())
        .collect();
    let removed_done: Vec<&str> = parent
        .tasks
        .iter()
        .filter(|(id, (_, st))| *st == Status::Done && !child.tasks.contains_key(id.as_str()))
        .map(|(id, _)| id.as_str())
        .collect();
    if added_done.as_slice() == [tracked_id] && removed_done.len() == 1 {
        return Some(removed_done[0].to_string());
    }
    None
}

/// Check whether `oid` is listed in `<git-common-dir>/shallow` (a shallow graft boundary).
fn is_shallow_graft(repo: &Repository, oid: Oid) -> bool {
    let hex = oid.to_string();
    let shallow_path = repo.commondir().join("shallow");
    std::fs::read_to_string(shallow_path)
        .is_ok_and(|contents| contents.lines().any(|line| line.trim() == hex))
}

#[cfg(test)]
mod tests {

    use std::path::Path;
    use std::process::Command;

    use super::resolve_completion_times;
    use aplan_core::{
        ClosedAtSource, CompletionHistoryError, CompletionTimeSource, HistoryCompleteness,
        HistoryLimits, HistoryPolicy,
    };

    fn git(root: &Path, args: &[&str], commit_ts: Option<i64>) -> String {
        let mut cmd = Command::new("git");
        cmd.current_dir(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "tester")
            .env("GIT_AUTHOR_EMAIL", "tester@example.com")
            .env("GIT_COMMITTER_NAME", "tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.com");
        if let Some(ts) = commit_ts {
            // Set author time deliberately different from committer time so tests
            // prove committer time (not author time) is reported per ADR-0074.
            cmd.env("GIT_AUTHOR_DATE", format!("{} +0000", ts - 3600));
            cmd.env("GIT_COMMITTER_DATE", format!("{ts} +0000"));
        }
        let out = cmd.output().expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn init_repo(root: &Path) {
        git(root, &["init", "-q", "-b", "main", "."], None);
        std::fs::create_dir_all(root.join("aplan/tasks")).expect("mkdir");
    }

    fn write_file(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir parent");
        }
        std::fs::write(path, body).expect("write file");
    }

    fn commit_all(root: &Path, msg: &str, commit_ts: i64) -> String {
        git(root, &["add", "-A"], None);
        git(root, &["commit", "-q", "-m", msg], Some(commit_ts));
        git(root, &["rev-parse", "HEAD"], None)
    }

    fn task_yaml(id: &str, status: &str, extra: &str) -> String {
        format!(
            "---\nid: {id}\ntitle: Task {id}\nphase: 1\nstatus: {status}\neffort_days: 1\n{extra}---\n\nBody.\n"
        )
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn single_pass_history_walk_resolves_recorded_legacy_renames_rekeys_reverts_and_untracked() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path();
        init_repo(root);

        // Commit 1 (2026-10-01T00:00:00Z = 1_790_812_800): all tasks start `todo`.
        write_file(
            root,
            "aplan/tasks/T-REC.md",
            &task_yaml("T-REC", "todo", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-IMP.md",
            &task_yaml("T-IMP", "todo", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-LIN.md",
            &task_yaml("T-LIN", "todo", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-REN.md",
            &task_yaml("T-REN", "todo", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-REKEY-OLD.md",
            &task_yaml("T-REKEY-OLD", "todo", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-REV.md",
            &task_yaml("T-REV", "todo", ""),
        );
        let _c0 = commit_all(root, "initial todos", 1_790_812_800);

        // Commit 2 (2026-10-02T12:00:00Z = 1_790_942_400): close T-LIN, T-REN,
        // T-REKEY-OLD, T-REV, T-REC (with recorded closed_at), T-IMP (imported).
        write_file(
            root,
            "aplan/tasks/T-REC.md",
            &task_yaml("T-REC", "done", "closed_at: 2026-10-11T19:24:08Z\n"),
        );
        write_file(
            root,
            "aplan/tasks/T-IMP.md",
            &task_yaml(
                "T-IMP",
                "done",
                "closed_at: 2026-10-10T10:00:00Z\nclosed_at_source: imported\n",
            ),
        );
        write_file(
            root,
            "aplan/tasks/T-LIN.md",
            &task_yaml("T-LIN", "done", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-REN.md",
            &task_yaml("T-REN", "done", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-REKEY-OLD.md",
            &task_yaml("T-REKEY-OLD", "done", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-REV.md",
            &task_yaml("T-REV", "done", ""),
        );
        let c_close_1 = commit_all(root, "first closes", 1_790_942_400);

        // Commit 3 (2026-10-03T12:00:00Z = 1_791_028_800): revert T-REV to todo,
        // move T-REN to a subdirectory, rekey T-REKEY-OLD -> T-REKEY-NEW.
        write_file(
            root,
            "aplan/tasks/T-REV.md",
            &task_yaml("T-REV", "todo", ""),
        );
        std::fs::remove_file(root.join("aplan/tasks/T-REN.md")).expect("rm T-REN");
        write_file(
            root,
            "aplan/tasks/archive/T-REN-moved.md",
            &task_yaml("T-REN", "done", ""),
        );
        std::fs::remove_file(root.join("aplan/tasks/T-REKEY-OLD.md")).expect("rm T-REKEY-OLD");
        write_file(
            root,
            "aplan/tasks/T-REKEY-NEW.md",
            &task_yaml("T-REKEY-NEW", "done", ""),
        );
        let _c_mid = commit_all(root, "revert rename rekey", 1_791_028_800);

        // Commit 4 (2026-10-04T12:00:00Z = 1_791_115_200): re-close T-REV.
        write_file(
            root,
            "aplan/tasks/T-REV.md",
            &task_yaml("T-REV", "done", ""),
        );
        let c_reclose = commit_all(root, "reclose T-REV", 1_791_115_200);

        // Untracked working-tree `done` task (never committed).
        write_file(
            root,
            "aplan/tasks/T-UNTRACKED.md",
            &task_yaml("T-UNTRACKED", "done", ""),
        );

        let plan = aplan_core::load_plan(&root.join("aplan"));
        let refs: Vec<&aplan_core::Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let report = resolve_completion_times(
            root,
            Path::new("aplan"),
            &refs,
            None,
            HistoryPolicy::Auto,
            &HistoryLimits::default(),
        )
        .expect("resolve completion times");

        assert_eq!(report.completeness, HistoryCompleteness::Complete);
        assert_eq!(report.basis_revision, format!("working_tree@{c_reclose}"));
        // Single pass walked at most the 3 relevant commits (c_reclose, c_mid, c_close_1).
        assert!(
            report.commits_walked <= 4,
            "commits_walked={}",
            report.commits_walked
        );

        // Recorded values win over Git history and preserve local vs imported origin.
        let rec = &report.resolved["T-REC"];
        assert_eq!(rec.source, CompletionTimeSource::Recorded);
        assert_eq!(rec.recorded_origin, Some(ClosedAtSource::Local));
        assert_eq!(rec.instant.as_deref(), Some("2026-10-11T19:24:08Z"));
        assert_eq!(rec.commit, None);

        let imp = &report.resolved["T-IMP"];
        assert_eq!(imp.source, CompletionTimeSource::Recorded);
        assert_eq!(imp.recorded_origin, Some(ClosedAtSource::Imported));
        assert_eq!(imp.instant.as_deref(), Some("2026-10-10T10:00:00Z"));

        // Linear, renamed, and unambiguously rekeyed tasks all resolve to `c_close_1`.
        for id in ["T-LIN", "T-REN", "T-REKEY-NEW"] {
            let item = &report.resolved[id];
            assert_eq!(item.source, CompletionTimeSource::HistoryFirstSeen, "{id}");
            assert_eq!(item.commit.as_deref(), Some(c_close_1.as_str()), "{id}");
            assert_eq!(
                item.instant.as_deref(),
                Some("2026-10-02T12:00:00Z"),
                "{id}"
            );
            assert_eq!(item.incomparable_candidates, 0, "{id}");
        }

        // Reverted + re-closed task resolves to its current episode (`c_reclose`), NOT `c_close_1`.
        let rev = &report.resolved["T-REV"];
        assert_eq!(rev.source, CompletionTimeSource::HistoryFirstSeen);
        assert_eq!(rev.commit.as_deref(), Some(c_reclose.as_str()));
        assert_eq!(rev.instant.as_deref(), Some("2026-10-04T12:00:00Z"));

        // Untracked working-tree `done` task stays `unknown`.
        let untracked = &report.resolved["T-UNTRACKED"];
        assert_eq!(untracked.source, CompletionTimeSource::Unknown);
        assert_eq!(untracked.instant, None);
        assert_eq!(untracked.commit, None);

        // Inferred timestamps are NEVER persisted into task files on disk.
        let lin_disk =
            std::fs::read_to_string(root.join("aplan/tasks/T-LIN.md")).expect("read T-LIN");
        assert!(
            !lin_disk.contains("closed_at"),
            "history inference must never persist closed_at:\n{lin_disk}"
        );
    }

    #[test]
    fn merge_incomparable_candidates_and_skewed_or_equal_timestamps_select_deterministically() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path();
        init_repo(root);

        // Base commit: T-PAR, T-ONE, T-EQ all start `todo`.
        write_file(
            root,
            "aplan/tasks/T-PAR.md",
            &task_yaml("T-PAR", "todo", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-ONE.md",
            &task_yaml("T-ONE", "todo", ""),
        );
        write_file(root, "aplan/tasks/T-EQ.md", &task_yaml("T-EQ", "todo", ""));
        let _base = commit_all(root, "base", 1_790_812_800);

        // Create branch `side` where T-PAR closes EARLIER (1_790_900_000) and T-ONE closes,
        // and T-EQ closes at 1_790_950_000.
        git(root, &["checkout", "-q", "-b", "side"], None);
        write_file(
            root,
            "aplan/tasks/T-PAR.md",
            &task_yaml("T-PAR", "done", ""),
        );
        write_file(
            root,
            "aplan/tasks/T-ONE.md",
            &task_yaml("T-ONE", "done", ""),
        );
        let c_side_par = commit_all(root, "side closes T-PAR and T-ONE", 1_790_900_000);
        write_file(root, "aplan/tasks/T-EQ.md", &task_yaml("T-EQ", "done", ""));
        let c_side_eq = commit_all(root, "side closes T-EQ", 1_790_950_000);

        // Back on `main` (first parent of the upcoming merge!): T-PAR closes LATER
        // (1_790_920_000) so the selected candidate (`c_side_par`) is NOT first!
        // T-EQ closes at the EXACT SAME timestamp (1_790_950_000) to test OID tie-break.
        git(root, &["checkout", "-q", "main"], None);
        write_file(
            root,
            "aplan/tasks/T-PAR.md",
            &task_yaml("T-PAR", "done", ""),
        );
        let _c_main_par = commit_all(root, "main closes T-PAR later", 1_790_920_000);
        write_file(root, "aplan/tasks/T-EQ.md", &task_yaml("T-EQ", "done", ""));
        let c_main_eq = commit_all(root, "main closes T-EQ same instant", 1_790_950_000);

        // Merge `side` into `main` (with a skewed earlier merge timestamp to prove
        // merge commit is NOT falsely chosen over parent transitions).
        git(
            root,
            &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
            Some(1_790_850_000),
        );

        let plan = aplan_core::load_plan(&root.join("aplan"));
        let refs: Vec<&aplan_core::Task> = plan.tasks.iter().map(|lt| &lt.task).collect();
        let report = resolve_completion_times(
            root,
            Path::new("aplan"),
            &refs,
            None,
            HistoryPolicy::Required,
            &HistoryLimits::default(),
        )
        .expect("complete merge history");

        assert_eq!(report.completeness, HistoryCompleteness::Complete);

        // T-PAR: two incomparable candidates; selected is `c_side_par` (earlier time, second parent).
        let par = &report.resolved["T-PAR"];
        assert_eq!(par.source, CompletionTimeSource::HistoryFirstSeen);
        assert_eq!(par.commit.as_deref(), Some(c_side_par.as_str()));
        assert_eq!(par.incomparable_candidates, 2);

        // T-ONE: closed only on `side`; merge commit is NOT a candidate.
        let one = &report.resolved["T-ONE"];
        assert_eq!(one.source, CompletionTimeSource::HistoryFirstSeen);
        assert_eq!(one.commit.as_deref(), Some(c_side_par.as_str()));
        assert_eq!(one.incomparable_candidates, 0);

        // T-EQ: equal committer timestamps on two incomparable branches; tie-breaks
        // deterministically by lexicographically smallest full commit OID.
        let eq = &report.resolved["T-EQ"];
        let expected_eq = c_side_eq.as_str().min(c_main_eq.as_str());
        assert_eq!(eq.source, CompletionTimeSource::HistoryFirstSeen);
        assert_eq!(eq.commit.as_deref(), Some(expected_eq));
        assert_eq!(eq.incomparable_candidates, 2);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn history_policies_no_git_shallow_bounded_and_unparseable_outcomes() {
        // 1. Non-Git directory: Auto -> Unavailable, Never -> HistorySkipped, Required -> Error.
        let nogit_dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let nogit = nogit_dir.path();
        write_file(nogit, "aplan/tasks/L-1.md", &task_yaml("L-1", "done", ""));
        write_file(
            nogit,
            "aplan/tasks/R-1.md",
            &task_yaml("R-1", "done", "closed_at: 2026-10-11T19:24:08Z\n"),
        );
        let plan = aplan_core::load_plan(&nogit.join("aplan"));
        let refs: Vec<&aplan_core::Task> = plan.tasks.iter().map(|lt| &lt.task).collect();

        let auto = resolve_completion_times(
            nogit,
            Path::new("aplan"),
            &refs,
            None,
            HistoryPolicy::Auto,
            &HistoryLimits::default(),
        )
        .expect("auto without git");
        assert_eq!(auto.basis_revision, "unversioned");
        assert_eq!(auto.completeness, HistoryCompleteness::Unavailable);
        assert_eq!(auto.resolved["L-1"].source, CompletionTimeSource::Unknown);
        assert_eq!(auto.resolved["R-1"].source, CompletionTimeSource::Recorded);

        let never = resolve_completion_times(
            nogit,
            Path::new("aplan"),
            &refs,
            None,
            HistoryPolicy::Never,
            &HistoryLimits::default(),
        )
        .expect("never without git");
        assert_eq!(never.completeness, HistoryCompleteness::HistorySkipped);

        let req_err = resolve_completion_times(
            nogit,
            Path::new("aplan"),
            &refs,
            None,
            HistoryPolicy::Required,
            &HistoryLimits::default(),
        )
        .expect_err("required without git must fail");
        assert!(matches!(
            req_err,
            CompletionHistoryError::Unavailable { .. }
        ));

        // When all requested tasks carry `recorded` closed_at, even `Required` without Git succeeds!
        let only_rec: Vec<&aplan_core::Task> =
            refs.iter().copied().filter(|t| t.id == "R-1").collect();
        let rec_ok = resolve_completion_times(
            nogit,
            Path::new("aplan"),
            &only_rec,
            None,
            HistoryPolicy::Required,
            &HistoryLimits::default(),
        )
        .expect("all recorded succeeds even without git");
        assert_eq!(rec_ok.completeness, HistoryCompleteness::Complete);
        assert_eq!(rec_ok.commits_walked, 0);

        // 2. Bounded, shallow, and unparseable historical shape in a real Git repo.
        let origin_dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("origin tempdir");
        let origin = origin_dir.path();
        init_repo(origin);
        write_file(origin, "aplan/tasks/S-1.md", &task_yaml("S-1", "todo", ""));
        let _c1 = commit_all(origin, "c1", 1_790_812_800);
        write_file(origin, "aplan/tasks/S-1.md", &task_yaml("S-1", "done", ""));
        // Also commit an unparseable historical task file to test IncompleteUnobservable.
        write_file(
            origin,
            "aplan/tasks/BROKEN.md",
            "---\nid: BROKEN\nstatus: [not-a-status\n---\n\nBad.\n",
        );
        let _c2 = commit_all(origin, "c2", 1_790_900_000);
        std::fs::remove_file(origin.join("aplan/tasks/BROKEN.md")).expect("rm broken");
        write_file(origin, "README.md", "touch\n");
        let _c3 = commit_all(origin, "c3", 1_790_990_000);

        let origin_plan = aplan_core::load_plan(&origin.join("aplan"));
        let origin_refs: Vec<&aplan_core::Task> =
            origin_plan.tasks.iter().map(|lt| &lt.task).collect();

        // Bounded commit budget (`max_commits: 1`) -> IncompleteBounded under Auto, Error under Required.
        let tight = HistoryLimits {
            max_commits: 1,
            ..HistoryLimits::default()
        };
        let bounded = resolve_completion_times(
            origin,
            Path::new("aplan"),
            &origin_refs,
            None,
            HistoryPolicy::Auto,
            &tight,
        )
        .expect("bounded auto");
        assert_eq!(bounded.completeness, HistoryCompleteness::IncompleteBounded);
        assert!(matches!(
            resolve_completion_times(
                origin,
                Path::new("aplan"),
                &origin_refs,
                None,
                HistoryPolicy::Required,
                &tight,
            ),
            Err(CompletionHistoryError::Incomplete {
                completeness: HistoryCompleteness::IncompleteBounded,
                ..
            })
        ));

        // Full walk encounters `BROKEN.md` in `c2` -> IncompleteUnobservable (never silently omitted).
        let unobs = resolve_completion_times(
            origin,
            Path::new("aplan"),
            &origin_refs,
            None,
            HistoryPolicy::Auto,
            &HistoryLimits::default(),
        )
        .expect("unobservable auto");
        assert_eq!(
            unobs.completeness,
            HistoryCompleteness::IncompleteUnobservable
        );
        assert!(
            unobs.limitations.iter().any(|l| l.contains("BROKEN.md")),
            "limitations must name unparseable file: {:?}",
            unobs.limitations
        );

        // Shallow clone (`--depth 1`) -> IncompleteShallow under Auto, Error under Required.
        let shallow_dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("shallow tempdir");
        let shallow = shallow_dir.path().join("clone");
        let file_url = format!("file://{}", origin.display());
        git(
            shallow_dir.path(),
            &[
                "clone",
                "-q",
                "--depth",
                "1",
                &file_url,
                shallow.to_str().expect("utf8"),
            ],
            None,
        );
        let shallow_plan = aplan_core::load_plan(&shallow.join("aplan"));
        let shallow_refs: Vec<&aplan_core::Task> =
            shallow_plan.tasks.iter().map(|lt| &lt.task).collect();
        let shallow_report = resolve_completion_times(
            &shallow,
            Path::new("aplan"),
            &shallow_refs,
            None,
            HistoryPolicy::Auto,
            &HistoryLimits::default(),
        )
        .expect("shallow auto");
        assert_eq!(
            shallow_report.completeness,
            HistoryCompleteness::IncompleteShallow
        );
        assert!(matches!(
            resolve_completion_times(
                &shallow,
                Path::new("aplan"),
                &shallow_refs,
                None,
                HistoryPolicy::Required,
                &HistoryLimits::default(),
            ),
            Err(CompletionHistoryError::Incomplete {
                completeness: HistoryCompleteness::IncompleteShallow,
                ..
            })
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn history_resolver_covers_root_plans_blob_bounds_bad_blobs_and_invalid_revisions() {
        use aplan_core::{Status, Task};
        use std::collections::BTreeMap;

        let dir = tempfile::Builder::new()
            .prefix("history-edges-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");

        let done_legacy = Task {
            id: "T-1".to_string(),
            title: "Legacy".to_string(),
            phase: 1,
            status: Status::Done,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            ratified_by: Vec::new(),
            contains: Vec::new(),
            integrates: Vec::new(),
            evidence: Vec::new(),
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: BTreeMap::new(),
            evidence_fingerprints: BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
            version: None,
            closed_at: None,
            closed_at_source: None,
        };
        let todo_task = Task {
            id: "T-TODO".to_string(),
            status: Status::Todo,
            ..done_legacy.clone()
        };

        // Explicit revision on a non-existent path outside any repo fails with `InvalidRevision`.
        let unrooted = Path::new("/");
        if git2::Repository::discover(unrooted).is_err() {
            assert!(matches!(
                resolve_completion_times(
                    unrooted,
                    Path::new("."),
                    &[&done_legacy, &todo_task],
                    Some("HEAD"),
                    HistoryPolicy::Auto,
                    &HistoryLimits::default(),
                ),
                Err(CompletionHistoryError::InvalidRevision { .. })
            ));
            assert!(matches!(
                resolve_completion_times(
                    unrooted,
                    Path::new("."),
                    &[&done_legacy],
                    None,
                    HistoryPolicy::Required,
                    &HistoryLimits::default(),
                ),
                Err(CompletionHistoryError::Unavailable { .. })
            ));
        }

        // Root-level plan (`plan_root_rel == "."`) with non-md, non-task md, invalid UTF-8,
        // malformed YAML, and oversized blobs exercises every snapshot/blob branch.
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).expect("repo");
        git(&repo_dir, &["init", "-q"], None);
        git(&repo_dir, &["config", "user.email", "t@example.com"], None);
        git(&repo_dir, &["config", "user.name", "Test"], None);
        std::fs::write(repo_dir.join("notes.txt"), "not markdown\n").expect("txt");
        std::fs::write(repo_dir.join("README.md"), "# No frontmatter\n").expect("readme");
        std::fs::write(repo_dir.join("bad_utf8.md"), [0xff, 0xfe, 0x00]).expect("utf8");
        std::fs::write(repo_dir.join("bad_yaml.md"), "---\nid: [unclosed\n---\n").expect("yaml");
        std::fs::write(
            repo_dir.join("T-1.md"),
            "---\nid: T-1\ntitle: Legacy\nphase: 1\nstatus: done\neffort_days: 1\n---\n\nbody\n",
        )
        .expect("task");
        git(&repo_dir, &["add", "-A"], None);
        git(&repo_dir, &["commit", "-q", "-m", "seed root plan"], None);

        // Invalid revision and non-commit blob revision fail cleanly.
        assert!(matches!(
            resolve_completion_times(
                &repo_dir,
                Path::new("."),
                &[&done_legacy],
                Some("refs/heads/no-such-branch"),
                HistoryPolicy::Auto,
                &HistoryLimits::default(),
            ),
            Err(CompletionHistoryError::InvalidRevision { .. })
        ));
        let blob_oid = git(&repo_dir, &["rev-parse", "HEAD:notes.txt"], None);
        assert!(matches!(
            resolve_completion_times(
                &repo_dir,
                Path::new("."),
                &[&done_legacy],
                Some(&blob_oid),
                HistoryPolicy::Auto,
                &HistoryLimits::default(),
            ),
            Err(CompletionHistoryError::InvalidRevision { .. })
        ));

        // Tiny `max_blob_bytes` triggers `TooLarge` / bounded completeness.
        let tight_single = HistoryLimits {
            max_blob_bytes: 8,
            ..HistoryLimits::default()
        };
        let bounded = resolve_completion_times(
            &repo_dir,
            Path::new("."),
            &[&done_legacy],
            None,
            HistoryPolicy::Auto,
            &tight_single,
        )
        .expect("bounded report");
        assert_eq!(bounded.completeness, HistoryCompleteness::IncompleteBounded);

        // Tight total blob budget also triggers `TooLarge` / bounded completeness.
        let tight_total = HistoryLimits {
            max_total_blob_bytes: 20,
            ..HistoryLimits::default()
        };
        let bounded_total = resolve_completion_times(
            &repo_dir,
            Path::new("."),
            &[&done_legacy],
            None,
            HistoryPolicy::Auto,
            &tight_total,
        )
        .expect("bounded total report");
        assert_eq!(
            bounded_total.completeness,
            HistoryCompleteness::IncompleteBounded
        );

        // Normal limits parse root plan cleanly and report unparseable files as limitations.
        let normal = resolve_completion_times(
            &repo_dir,
            Path::new("."),
            &[&done_legacy, &todo_task],
            None,
            HistoryPolicy::Auto,
            &HistoryLimits::default(),
        )
        .expect("normal root plan");
        assert!(normal.resolved.contains_key("T-1"));
        assert!(!normal.limitations.is_empty());

        // Same-path rekey (`OLD-1` -> `NEW-1` both `done` at the identical file path).
        let mut parent = super::PlanSnapshot::default();
        parent
            .tasks
            .insert("OLD-1".into(), ("aplan/T.md".into(), Status::Done));
        parent
            .by_path
            .insert("aplan/T.md".into(), ("OLD-1".into(), Status::Done));
        let mut child = super::PlanSnapshot::default();
        child
            .tasks
            .insert("NEW-1".into(), ("aplan/T.md".into(), Status::Done));
        child
            .by_path
            .insert("aplan/T.md".into(), ("NEW-1".into(), Status::Done));
        assert_eq!(
            super::detect_unambiguous_rekey(&child, &parent, "NEW-1").as_deref(),
            Some("OLD-1")
        );
    }
}
