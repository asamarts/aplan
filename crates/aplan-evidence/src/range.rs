//! Commit-range structural plan validation (`A2-156`).
//!
//! Walks every commit introduced by a caller-supplied `BASE..TIP` (or `..TIP`)
//! range directly from Git tree objects without checking out over or mutating
//! the working tree, executing no historical hooks/commands/runners, following
//! no symlinks, and never reading or populating the working-tree plan cache.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use aplan_core::{Diagnostic, Verdict, load_plan_files, validate};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::Error;

/// Deterministic upper bound on commits audited in a single `--range` run.
pub const MAX_RANGE_COMMITS: usize = 1000;

/// Schema identifier for machine-readable commit-range audit reports.
pub const RANGE_REPORT_SCHEMA: &str = "aplan/range-check/v1";

/// Options controlling a commit-range structural validation run (`A2-156`).
#[derive(Debug, Clone)]
pub struct RangeCheckOptions {
    /// Git revspec: `BASE..TIP` (commits reachable from `TIP` excluding `BASE`)
    /// or `..TIP` (all commits reachable from `TIP`, including root commits).
    pub range: String,
    /// Stop at the first invalid commit instead of auditing the full range.
    pub fail_fast: bool,
    /// Upper bound on commits allowed in the range before refusing.
    pub max_commits: usize,
    /// Suppress `APLAN009` missing-effort warnings, matching `check`.
    pub no_require_effort: bool,
    /// Skip `APLAN008` / `APLAN018` / `APLAN060` trace resolution, matching `check`.
    pub no_traces: bool,
    /// Explicit CLI override for `deny_warnings` (`Some(true)` / `Some(false)`),
    /// or `None` to honour each commit's own `.aplan.yml`.
    pub deny_warnings_override: Option<bool>,
}

impl RangeCheckOptions {
    /// Default options for a given range spec.
    #[must_use]
    pub fn new(range: impl Into<String>) -> Self {
        Self {
            range: range.into(),
            fail_fast: false,
            max_commits: MAX_RANGE_COMMITS,
            no_require_effort: false,
            no_traces: false,
            deny_warnings_override: None,
        }
    }
}

/// Serializable diagnostic entry inside a per-commit range audit record.
#[derive(Debug, Clone, Serialize)]
pub struct RangeDiagnostic {
    pub code: String,
    pub severity: String,
    pub task: Option<String>,
    pub file: Option<String>,
    pub message: String,
    pub agent_instruction: Option<String>,
}

impl From<&Diagnostic> for RangeDiagnostic {
    fn from(d: &Diagnostic) -> Self {
        let severity = match d.severity {
            aplan_core::Severity::Error => "error",
            aplan_core::Severity::Warning => "warning",
        };
        Self {
            code: d.code.to_string(),
            severity: severity.to_string(),
            task: d.task.clone(),
            file: d.file.as_ref().map(|p| p.display().to_string()),
            message: d.message.clone(),
            agent_instruction: d.agent_instruction.clone(),
        }
    }
}

/// Audit outcome for one commit in the range.
#[derive(Debug, Clone, Serialize)]
pub struct CommitAudit {
    /// Full 40-hex commit SHA.
    pub commit: String,
    /// Abbreviated 12-hex commit SHA.
    pub short_commit: String,
    /// First line of the commit message.
    pub subject: String,
    /// Whether this commit passed under its effective verdict policy.
    pub clean: bool,
    /// Number of tasks validated at this commit.
    pub tasks_checked: usize,
    /// If non-`None`, the earlier commit SHA whose proved-identical relevant
    /// inputs were reused rather than re-validated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reused_from: Option<String>,
    /// Complete ordinary diagnostics for this commit (or a config-load failure).
    pub diagnostics: Vec<RangeDiagnostic>,
}

/// Complete machine- and human-renderable report for `aplan check --range`.
#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct RangeReport {
    pub schema: &'static str,
    pub range: String,
    pub base: Option<String>,
    pub tip: String,
    pub commits_total: usize,
    pub commits_audited: usize,
    pub commits_failed: usize,
    pub empty_range: bool,
    pub stopped_early: bool,
    /// Whether every audited commit passed.
    pub range_clean: bool,
    /// Whether the tip commit itself passed (distinguishes a green tip from an
    /// invalid intermediate commit).
    pub tip_clean: bool,
    pub commits: Vec<CommitAudit>,
}

/// Validate every plan state introduced by `opts.range` in the Git repository
/// at `repo_root` without mutating the working tree (`A2-156`).
///
/// # Errors
///
/// Returns an error if the repository cannot be opened, the range syntax is
/// invalid, any revision is missing, histories are unrelated, the repository
/// is a shallow clone with an incomplete boundary, or the range exceeds
/// `opts.max_commits`.
#[allow(clippy::too_many_lines)]
pub fn check_commit_range(
    repo_root: &Path,
    opts: &RangeCheckOptions,
) -> Result<RangeReport, Error> {
    let repo = git2::Repository::discover(repo_root)
        .map_err(|_| Error::NotARepository(repo_root.to_path_buf()))?;

    let (base_spec, tip_spec) = parse_range_spec(&opts.range)?;
    let tip_commit = resolve_commit(&repo, tip_spec)?;
    let tip_id = tip_commit.id();
    let tip_sha = tip_id.to_string();

    let (base_id, base_sha) = if let Some(b) = base_spec {
        let base_commit = resolve_commit(&repo, b)?;
        let bid = base_commit.id();
        if bid == tip_id {
            return Ok(RangeReport {
                schema: RANGE_REPORT_SCHEMA,
                range: opts.range.clone(),
                base: Some(bid.to_string()),
                tip: tip_sha,
                commits_total: 0,
                commits_audited: 0,
                commits_failed: 0,
                empty_range: true,
                stopped_early: false,
                range_clean: true,
                tip_clean: true,
                commits: Vec::new(),
            });
        }
        let is_ancestor = repo.graph_descendant_of(tip_id, bid)?;
        if !is_ancestor {
            return Err(Error::InvalidRange(format!(
                "invalid commit range `{}`: base `{b}` ({}) is not an ancestor of tip `{tip_spec}` ({tip_sha}); histories are unrelated or range is reversed",
                opts.range,
                short_oid(bid)
            )));
        }
        (Some(bid), Some(bid.to_string()))
    } else {
        if repo.is_shallow() {
            return Err(Error::InvalidRange(
                "cannot audit an open-ended commit range (`..TIP`) in a shallow clone: \
                 history is incomplete; fetch full history (`git fetch --unshallow`) first"
                    .to_string(),
            ));
        }
        (None, None)
    };

    let mut walk = repo.revwalk()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)?;
    walk.push(tip_id)?;
    if let Some(bid) = base_id {
        walk.hide(bid)?;
    }

    let mut oids = Vec::new();
    for oid_res in walk {
        let oid = oid_res?;
        oids.push(oid);
        if oids.len() > opts.max_commits {
            return Err(Error::InvalidRange(format!(
                "commit range `{}` contains more than {} commits (bounded limit); \
                 narrow the range before auditing",
                opts.range, opts.max_commits
            )));
        }
    }

    // In a shallow clone, ensure every walked commit's parents actually exist
    // in the object database so we never silently treat a truncated history as
    // a complete audit.
    if repo.is_shallow() {
        for &oid in &oids {
            let commit = repo.find_commit(oid)?;
            for i in 0..commit.parent_count() {
                let pid = commit.parent_id(i)?;
                if repo.find_commit(pid).is_err() {
                    return Err(Error::InvalidRange(format!(
                        "shallow clone boundary reached at commit `{}` (missing parent `{}`): \
                         refusing to report an incomplete commit-range audit as clean",
                        short_oid(oid),
                        short_oid(pid)
                    )));
                }
            }
        }
    }

    let commits_total = oids.len();
    let mut audits = Vec::with_capacity(commits_total);
    let mut cache: BTreeMap<String, CachedCommitOutcome> = BTreeMap::new();
    let mut stopped_early = false;

    for oid in oids {
        let commit = repo.find_commit(oid)?;
        let audit = audit_single_commit(&repo, &commit, opts, &mut cache)?;
        let failed = !audit.clean;
        audits.push(audit);
        if opts.fail_fast && failed {
            stopped_early = true;
            break;
        }
    }

    let commits_audited = audits.len();
    let commits_failed = audits.iter().filter(|a| !a.clean).count();
    let range_clean = commits_failed == 0;
    let tip_clean = if stopped_early {
        audits.last().filter(|a| a.commit == tip_sha).map_or_else(
            || audit_single_commit(&repo, &tip_commit, opts, &mut cache).is_ok_and(|a| a.clean),
            |a| a.clean,
        )
    } else {
        audits.last().is_none_or(|a| a.clean)
    };

    Ok(RangeReport {
        schema: RANGE_REPORT_SCHEMA,
        range: opts.range.clone(),
        base: base_sha,
        tip: tip_sha,
        commits_total,
        commits_audited,
        commits_failed,
        empty_range: commits_total == 0,
        stopped_early,
        range_clean,
        tip_clean,
        commits: audits,
    })
}

#[derive(Clone)]
struct CachedCommitOutcome {
    source_commit: String,
    clean: bool,
    tasks_checked: usize,
    diagnostics: Vec<RangeDiagnostic>,
}

fn parse_range_spec(range: &str) -> Result<(Option<&str>, &str), Error> {
    let trimmed = range.trim();
    if trimmed.contains("...") {
        return Err(Error::InvalidRange(format!(
            "invalid commit range `{trimmed}`: symmetric-difference `...` is not supported; \
             use `BASE..TIP` or `..TIP`"
        )));
    }
    let Some((left, right)) = trimmed.split_once("..") else {
        return Err(Error::InvalidRange(format!(
            "invalid commit range `{trimmed}`: expected `BASE..TIP` or `..TIP`"
        )));
    };
    let tip = right.trim();
    if tip.is_empty() {
        return Err(Error::InvalidRange(format!(
            "invalid commit range `{trimmed}`: missing tip revision after `..`"
        )));
    }
    let base = left.trim();
    Ok(((!base.is_empty()).then_some(base), tip))
}

fn resolve_commit<'a>(repo: &'a git2::Repository, spec: &str) -> Result<git2::Commit<'a>, Error> {
    let obj = repo
        .revparse_single(spec)
        .map_err(|e| Error::InvalidRange(format!("cannot resolve revision `{spec}`: {e}")))?;
    obj.peel_to_commit()
        .map_err(|e| Error::InvalidRange(format!("revision `{spec}` does not name a commit: {e}")))
}

fn short_oid(oid: git2::Oid) -> String {
    let s = oid.to_string();
    s.get(..12).unwrap_or(&s).to_string()
}

fn read_regular_blob(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    rel: &Path,
) -> Option<(git2::Oid, Vec<u8>)> {
    let entry = tree.get_path(rel).ok()?;
    // Historical bytes are repository data only: never follow symlinks (0o120000).
    if entry.kind() != Some(git2::ObjectType::Blob) || entry.filemode() == 0o120_000 {
        return None;
    }
    let blob = repo.find_blob(entry.id()).ok()?;
    Some((entry.id(), blob.content().to_vec()))
}

fn is_safe_repo_rel_path(rel: &Path) -> bool {
    if rel.is_absolute() {
        return false;
    }
    for comp in rel.components() {
        if matches!(
            comp,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_lines)]
fn audit_single_commit(
    repo: &git2::Repository,
    commit: &git2::Commit<'_>,
    opts: &RangeCheckOptions,
    cache: &mut BTreeMap<String, CachedCommitOutcome>,
) -> Result<CommitAudit, Error> {
    let full_sha = commit.id().to_string();
    let short_sha = short_oid(commit.id());
    let subject = commit
        .message()
        .ok()
        .and_then(|m| m.lines().next())
        .unwrap_or("(no commit message)")
        .to_string();
    let tree = commit.tree()?;

    // 1. Read historical `.aplan.yml` as pure data into an isolated staging dir
    //    (auto-cleaned on drop). Never execute historical commands or hooks.
    let staging = tempfile::Builder::new()
        .prefix("aplan-range-")
        .tempdir()
        .map_err(|e| Error::InvalidRange(format!("creating staging dir: {e}")))?;
    let config_blob = read_regular_blob(repo, &tree, Path::new(".aplan.yml"));
    let config_oid = config_blob.as_ref().map(|(oid, _)| oid.to_string());

    let config = if let Some((_, bytes)) = &config_blob {
        let cfg_path = staging.path().join(".aplan.yml");
        std::fs::write(&cfg_path, bytes)
            .map_err(|e| Error::InvalidRange(format!("staging `.aplan.yml`: {e}")))?;
        match aplan_config::load_from(&aplan_config::ConfigSource::File(cfg_path), staging.path()) {
            Ok((cfg, _)) => cfg,
            Err(err) => {
                return Ok(CommitAudit {
                    commit: full_sha,
                    short_commit: short_sha,
                    subject,
                    clean: false,
                    tasks_checked: 0,
                    reused_from: None,
                    diagnostics: vec![RangeDiagnostic {
                        code: "CONFIG".to_string(),
                        severity: "error".to_string(),
                        task: None,
                        file: Some(".aplan.yml".to_string()),
                        message: format!("invalid `.aplan.yml` at this commit: {err}"),
                        agent_instruction: Some(
                            "Fix `.aplan.yml` in this commit so the historical plan configuration \
                             loads cleanly."
                                .to_string(),
                        ),
                    }],
                });
            }
        }
    } else {
        aplan_config::Config::default()
    };

    let plan_root_rel = config
        .root
        .as_deref()
        .map_or_else(|| PathBuf::from("aplan"), PathBuf::from);
    if !is_safe_repo_rel_path(&plan_root_rel) {
        return Ok(CommitAudit {
            commit: full_sha,
            short_commit: short_sha,
            subject,
            clean: false,
            tasks_checked: 0,
            reused_from: None,
            diagnostics: vec![RangeDiagnostic {
                code: "CONFIG".to_string(),
                severity: "error".to_string(),
                task: None,
                file: Some(".aplan.yml".to_string()),
                message: format!(
                    "historical `root` path `{}` escapes the repository tree",
                    plan_root_rel.display()
                ),
                agent_instruction: None,
            }],
        });
    }

    // 2. Collect all regular-file blobs under `<plan_root>` from this commit's tree.
    let mut plan_oids: BTreeMap<String, String> = BTreeMap::new();
    let mut plan_files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    if let Ok(entry) = tree.get_path(&plan_root_rel)
        && entry.kind() == Some(git2::ObjectType::Tree)
        && let Ok(plan_tree) = repo.find_tree(entry.id())
    {
        plan_tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob)
                && entry.filemode() != 0o120_000
                && let Ok(name) = entry.name()
            {
                let rel_str = format!("{dir}{name}");
                let rel_path = PathBuf::from(&rel_str);
                if aplan_core::is_plan_file_path(&rel_path)
                    && let Ok(blob) = repo.find_blob(entry.id())
                {
                    plan_oids.insert(rel_str, entry.id().to_string());
                    plan_files.push((rel_path, blob.content().to_vec()));
                }
            }
            git2::TreeWalkResult::Ok
        })?;
    }

    // 3. Load plan purely in memory (never touches the working-tree cache).
    let plan = load_plan_files(&plan_root_rel, plan_files);

    // 4. Identify referenced trace documents (and their blob OIDs) so any edit
    //    to a trace-bearing document invalidates the proved-identical digest
    //    even when zero task files changed.
    let traces_base_rel = config
        .traces_root
        .as_deref()
        .map_or_else(PathBuf::new, PathBuf::from);
    let skip_traces = opts.no_traces || config.no_traces.unwrap_or(false);
    let mut trace_doc_oids: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut trace_blobs: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    if !skip_traces {
        let mut referenced_docs: BTreeSet<String> = BTreeSet::new();
        for lt in &plan.tasks {
            for tr in &lt.task.traces {
                let doc = tr.split_once('#').map_or(tr.as_str(), |(d, _)| d);
                if !doc.is_empty() {
                    referenced_docs.insert(doc.to_string());
                }
            }
        }
        for doc in referenced_docs {
            let joined = if traces_base_rel.as_os_str().is_empty() {
                PathBuf::from(&doc)
            } else {
                traces_base_rel.join(&doc)
            };
            let norm = aplan_core::anchor::normalise(&joined);
            if !is_safe_repo_rel_path(&norm) {
                trace_doc_oids.insert(doc, None);
                continue;
            }
            if let Some((oid, bytes)) = read_regular_blob(repo, &tree, &norm) {
                trace_doc_oids.insert(doc, Some(oid.to_string()));
                trace_blobs.push((norm, bytes));
            } else {
                trace_doc_oids.insert(doc, None);
            }
        }
    }

    // 5. Compute proved-identical relevant-inputs digest and reuse if seen.
    let digest = compute_inputs_digest(
        config_oid.as_deref(),
        &plan_oids,
        &trace_doc_oids,
        opts.no_require_effort,
        skip_traces,
        opts.deny_warnings_override,
    );
    if let Some(hit) = cache.get(&digest) {
        return Ok(CommitAudit {
            commit: full_sha,
            short_commit: short_sha,
            subject,
            clean: hit.clean,
            tasks_checked: hit.tasks_checked,
            reused_from: Some(hit.source_commit.clone()),
            diagnostics: hit.diagnostics.clone(),
        });
    }

    // 6. Stage ONLY the referenced non-symlink trace documents into the temp
    //    staging directory and run ordinary `aplan_core::validate`.
    let staged_traces_root = if skip_traces {
        None
    } else {
        for (rel, bytes) in &trace_blobs {
            let dest = staging.path().join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error::InvalidRange(format!("staging trace dir: {e}")))?;
            }
            std::fs::write(dest, bytes)
                .map_err(|e| Error::InvalidRange(format!("staging trace file: {e}")))?;
        }
        let base = if traces_base_rel.as_os_str().is_empty() {
            staging.path().to_path_buf()
        } else {
            staging.path().join(&traces_base_rel)
        };
        Some(base)
    };

    let mut validate_opts = aplan_core::validate::Options {
        require_effort: !opts.no_require_effort && config.require_effort.unwrap_or(true),
        detect_drift: config.detect_drift.unwrap_or(false),
        require_cut_membership: config.require_cut_membership.unwrap_or(false),
        require_impact: config
            .version
            .as_ref()
            .filter(|v| v.requires_impact())
            .map(aplan_config::VersionPolicy::floors),
        traces_root: staged_traces_root,
        traces_repository: Some(staging.path().to_path_buf()),
        ..Default::default()
    };
    if let Some(ev) = &config.evidence {
        validate_opts
            .require_min_strength
            .clone_from(&ev.require_min_strength);
        if let Some(exempt) = &ev.exempt_tags {
            validate_opts.exempt_tags.clone_from(exempt);
        }
    }
    if let Some(owners) = &config.owners {
        validate_opts.owners = owners.iter().cloned().collect();
    }

    let report = validate(&plan, &validate_opts);
    let verdict = Verdict {
        deny_warnings: opts
            .deny_warnings_override
            .unwrap_or_else(|| config.deny_warnings.unwrap_or(false)),
        advisory: config.advisory.clone().unwrap_or_default(),
    };
    let clean = !report.denied_by(&verdict) && plan.unloadable.is_empty();
    let diagnostics: Vec<RangeDiagnostic> = report
        .diagnostics
        .iter()
        .map(RangeDiagnostic::from)
        .collect();

    cache.insert(
        digest,
        CachedCommitOutcome {
            source_commit: short_sha.clone(),
            clean,
            tasks_checked: report.tasks_checked,
            diagnostics: diagnostics.clone(),
        },
    );

    Ok(CommitAudit {
        commit: full_sha,
        short_commit: short_sha,
        subject,
        clean,
        tasks_checked: report.tasks_checked,
        reused_from: None,
        diagnostics,
    })
}

fn compute_inputs_digest(
    config_oid: Option<&str>,
    plan_oids: &BTreeMap<String, String>,
    trace_doc_oids: &BTreeMap<String, Option<String>>,
    no_require_effort: bool,
    no_traces: bool,
    deny_warnings_override: Option<bool>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"aplan-range-v1\0");
    hasher.update(config_oid.unwrap_or("none").as_bytes());
    hasher.update(b"\0");
    hasher.update([u8::from(no_require_effort), u8::from(no_traces)]);
    hasher.update([match deny_warnings_override {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }]);
    for (k, v) in plan_oids {
        hasher.update(b"P:");
        hasher.update(k.as_bytes());
        hasher.update(b"=");
        hasher.update(v.as_bytes());
        hasher.update(b"\0");
    }
    for (k, v) in trace_doc_oids {
        hasher.update(b"T:");
        hasher.update(k.as_bytes());
        hasher.update(b"=");
        hasher.update(v.as_deref().unwrap_or("missing").as_bytes());
        hasher.update(b"\0");
    }
    let bytes = hasher.finalize();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn range_spec_and_path_safety_reject_unsupported_and_escaping_inputs() {
        let defaults = RangeCheckOptions::new("..HEAD");
        assert_eq!(defaults.max_commits, MAX_RANGE_COMMITS);
        assert!(!defaults.fail_fast);

        assert!(parse_range_spec("a...b").is_err());
        assert!(parse_range_spec("HEAD").is_err());
        assert!(parse_range_spec("HEAD..").is_err());
        assert_eq!(parse_range_spec("..HEAD").unwrap(), (None, "HEAD"));
        assert_eq!(parse_range_spec("a..b").unwrap(), (Some("a"), "b"));

        assert!(!is_safe_repo_rel_path(Path::new("/abs/plan")));
        assert!(!is_safe_repo_rel_path(Path::new("../escape")));
        assert!(is_safe_repo_rel_path(Path::new("aplan/tasks")));
    }

    #[test]
    fn audit_range_rejects_bad_ranges_invalid_historical_config_and_shallow_clones() {
        let dir = tempfile::Builder::new()
            .prefix("range-audit-edges-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("origin");
        std::fs::create_dir_all(root.join("aplan/tasks")).expect("dirs");
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "t@example.com"]);
        git(&root, &["config", "user.name", "Test"]);

        std::fs::write(
            root.join("aplan/tasks/T-1.md"),
            "---\nid: T-1\ntitle: Seed\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
        )
        .expect("task");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "c1"]);
        let c1 = git(&root, &["rev-parse", "HEAD"]);

        // Commit 2: invalid `.aplan.yml` in historical commit.
        std::fs::write(root.join(".aplan.yml"), "not_valid: [unclosed\n").expect("bad cfg");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "c2 bad cfg"]);

        // Commit 3: escaping `root` in historical `.aplan.yml`.
        std::fs::write(root.join(".aplan.yml"), "root: ../outside\n").expect("escape cfg");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "c3 escape root"]);
        let c3 = git(&root, &["rev-parse", "HEAD"]);

        // Commit 4: clean valid `.aplan.yml`.
        std::fs::write(root.join(".aplan.yml"), "root: aplan\n").expect("good cfg");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "c4 good"]);
        let c4 = git(&root, &["rev-parse", "HEAD"]);

        // Reversed range (`c4..c1`) is refused.
        assert!(check_commit_range(&root, &RangeCheckOptions::new(format!("{c4}..{c1}"))).is_err());

        // `max_commits` bound is enforced.
        let mut bounded = RangeCheckOptions::new(format!("{c1}..{c4}"));
        bounded.max_commits = 1;
        assert!(check_commit_range(&root, &bounded).is_err());

        // Invalid historical `.aplan.yml` and escaping `root` surface CONFIG diagnostics.
        let report = check_commit_range(&root, &RangeCheckOptions::new(format!("{c1}..{c3}")))
            .expect("range report");
        assert!(!report.range_clean);
        assert_eq!(report.commits.len(), 2);
        assert_eq!(report.commits[0].diagnostics[0].code, "CONFIG");
        assert!(
            report.commits[0].diagnostics[0]
                .message
                .contains("invalid `.aplan.yml`")
        );
        assert_eq!(report.commits[1].diagnostics[0].code, "CONFIG");
        assert!(
            report.commits[1].diagnostics[0]
                .message
                .contains("escapes the repository tree")
        );

        // Shallow clone refuses open-ended `..HEAD`.
        let shallow = dir.path().join("shallow");
        let file_url = format!("file://{}", root.display());
        git(
            dir.path(),
            &[
                "clone",
                "--depth",
                "1",
                &file_url,
                shallow.to_str().unwrap(),
            ],
        );
        let open_err = check_commit_range(&shallow, &RangeCheckOptions::new("..HEAD"))
            .expect_err("open-ended range in shallow clone must fail");
        assert!(open_err.to_string().contains("shallow clone"), "{open_err}");

        let repo_handle = git2::Repository::open(&shallow).expect("open shallow");
        let head_tree = repo_handle
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap();
        assert!(read_regular_blob(&repo_handle, &head_tree, Path::new("aplan")).is_none());
    }

    #[test]
    fn audit_range_covers_happy_path_diagnostics_traces_overrides_and_shallow_boundary() {
        let dir = tempfile::Builder::new()
            .prefix("range-audit-full-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let root = dir.path().join("origin");
        std::fs::create_dir_all(root.join("aplan/tasks")).expect("dirs");
        std::fs::create_dir_all(root.join("docs")).expect("docs");
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "t@example.com"]);
        git(&root, &["config", "user.name", "Test"]);

        // c1: base commit with config exercising traces_root, owners, exempt_tags, deny_warnings.
        std::fs::write(
            root.join(".aplan.yml"),
            "root: aplan\ntraces_root: docs\nowners: [alice]\ndeny_warnings: true\nevidence:\n  require_min_strength: [file]\n  exempt_tags: [spike]\n",
        )
        .expect("cfg");
        std::fs::write(root.join("docs/spec.md"), "# Spec\n\n## Sec\n\nBody.\n").expect("spec");
        std::fs::write(
            root.join("aplan/tasks/T-1.md"),
            "---\nid: T-1\ntitle: Valid\nphase: 1\nstatus: todo\nowner: alice\neffort_days: 1\ntraces:\n- spec.md#sec\n- ../outside.md#bad\n- missing.md#none\n---\n\nbody\n",
        )
        .expect("T-1");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "c1"]);
        let c1 = git(&root, &["rev-parse", "HEAD"]);

        // c2: adds task with warning (missing effort_days) to exercise RangeDiagnostic::from.
        std::fs::write(
            root.join("aplan/tasks/T-2.md"),
            "---\nid: T-2\ntitle: Missing effort\nphase: 1\nstatus: todo\nowner: alice\n---\n\nbody\n",
        )
        .expect("T-2");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "c2 warn"]);

        // c3: touches unrelated README to hit cache reuse.
        std::fs::write(root.join("README.md"), "hello\n").expect("readme");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "c3 readme"]);
        let c3 = git(&root, &["rev-parse", "HEAD"]);

        // With `deny_warnings_override = Some(false)` and `no_traces = true`, c2/c3 pass and reuse.
        let mut lenient = RangeCheckOptions::new(format!("{c1}..{c3}"));
        lenient.deny_warnings_override = Some(false);
        lenient.no_traces = true;
        let rep_lenient = check_commit_range(&root, &lenient).expect("lenient range");
        assert!(rep_lenient.range_clean, "{rep_lenient:?}");
        assert_eq!(rep_lenient.commits.len(), 2);
        assert!(!rep_lenient.commits[0].diagnostics.is_empty());
        assert!(rep_lenient.commits[1].reused_from.is_some());

        // With `deny_warnings_override = Some(true)` and traces enabled, warnings deny and traces stage.
        let mut strict = RangeCheckOptions::new(format!("{c1}..{c3}"));
        strict.deny_warnings_override = Some(true);
        let rep_strict = check_commit_range(&root, &strict).expect("strict range");
        assert!(!rep_strict.range_clean);

        // Shallow clone with unrelated orphan base reaches the shallow boundary parent check.
        let shallow = dir.path().join("shallow");
        let file_url = format!("file://{}", root.display());
        git(
            dir.path(),
            &[
                "clone",
                "--depth",
                "1",
                &file_url,
                shallow.to_str().unwrap(),
            ],
        );
        git(&shallow, &["config", "user.email", "t@example.com"]);
        git(&shallow, &["config", "user.name", "Test"]);
        let tip = git(&shallow, &["rev-parse", "HEAD"]);
        git(&shallow, &["checkout", "--orphan", "base-only", "-q"]);
        git(&shallow, &["rm", "-rf", "-q", "."]);
        git(&shallow, &["commit", "--allow-empty", "-q", "-m", "orphan"]);
        let orphan = git(&shallow, &["rev-parse", "HEAD"]);
        git(
            &shallow,
            &[
                "merge",
                "--allow-unrelated-histories",
                "-q",
                "-m",
                "merge",
                &tip,
            ],
        );
        let merged = git(&shallow, &["rev-parse", "HEAD"]);
        let shallow_rep = check_commit_range(
            &shallow,
            &RangeCheckOptions::new(format!("{orphan}..{merged}")),
        )
        .expect("bounded merge range inside shallow clone audits reachable commits");
        assert_eq!(shallow_rep.commits.len(), 2);
        assert!(!shallow_rep.range_clean);
    }
}
