//! A tamper-evident record of what a plan proved, bound to a commit (`A1-040`).
//!
//! RESEARCH section 15 ranked a signed provenance export first among the
//! adjacent demands: `StrictDoc` #2012 wants "git hash into the export as auditor
//! evidence", doorstop #375 wants signed items, beads #4460 an append-only
//! provenance log. They are one ask -- *hand somebody outside the repository the
//! proof, in a form they can check without rerunning the tool*.
//!
//! This is the end of the evidence ladder rather than a new idea. `verify`
//! already resolves every citation against git; an attestation is that verdict
//! written down, pinned to the tree it was true of.
//!
//! **What it asserts, exactly.** At commit `<sha>`, this build of `aplan` ran
//! `verify` over this plan and found nothing blocking, and these are the
//! citations it resolved. Each `commit` citation is recorded as the **full sha**
//! its ref resolved to, and each `file` citation as the **blob sha** at that
//! commit, so a reader with the repository re-derives both without trusting this
//! document. What it does not assert is that the tests pass today, that the urls
//! answer today, or that the work is good: an attestation is a statement about a
//! tree, not a warranty.
//!
//! A `commit` citation also records whether the attested commit can **reach**
//! it. `verify` does not ask that -- `APLAN012` wants the cited commit's diff to
//! touch the task's `paths`, which a commit on an unmerged branch satisfies --
//! but a document about a tree must not let a reader assume work is in a history
//! it is not in, and an unreachable object is one `git gc` from gone.
//!
//! **Who verified, and what that is worth.** The `tool` object carries the
//! package version, the commit this binary was built from, the digest of its own
//! `facts` document -- the capability contract: rules, commands, config keys,
//! behaviours, formats, evidence kinds -- and the digest of the running
//! executable. Two builds that share a version cannot share the last two unless
//! they are the same build, which is the gap the fifth pilot measured
//! (pre-public adoption evidence, `A1-154`).
//!
//! What stays **outside** the claim, said plainly because a provenance record
//! that overstates itself is worse than none:
//!
//! - The binary digest is **self-reported**. It identifies rather than
//!   authenticates: a modified binary can print any string. Its value is that an
//!   auditor compares it with a digest recorded independently when the binary
//!   was pinned, which is how the pilot that asked for it works.
//! - Nothing here is signed. A `git note` is tamper-evident because git hashes
//!   it, not because anybody vouched for it.
//! - The `source` field is best effort and names a commit, not a tree. It cannot
//!   tell you whether the working tree matched that commit: the first version
//!   tried, with `git describe --dirty`, and this task's audit measured a clean
//!   tree reporting `-dirty` and -- worse -- the ordinary edit-rebuild loop
//!   reporting clean while the binary held the change. `binary_sha256`
//!   identifies the artifact, and a release pipeline that needs certainty sets
//!   `APLAN_SOURCE_REV` itself.
//! - An empty field means *this build could not tell you* -- no checkout to ask,
//!   or an executable it could not read -- rather than "not applicable".
//! - The toolchain, target and build flags are not recorded. Two builds of one
//!   commit on different compilers differ in `binary_sha256` and in nothing else
//!   this document holds.
//!
//! **No timestamp, deliberately.** The document is a function of the commit and
//! the plan, so two runs at the same commit produce the same bytes -- which is
//! what makes it checkable at all, and what lets `git notes` hold it without
//! churn. When it was made is what git already records about the note.
//!
//! That property is why an **untracked** file is refused along with a modified
//! one: a file git has never seen is not in the tree a reader checks out, and
//! the first version let one change the document while the commit stayed the
//! same (`A1-040`, found by its audit).

use std::collections::BTreeMap;
use std::path::Path;

use aplan_core::{Plan, Status};

/// Why an attestation could not be made.
#[derive(Debug, thiserror::Error)]
pub enum AttestError {
    /// The repository could not be opened, or has no commits.
    #[error("{0}")]
    Repo(String),
    /// The work tree carries changes that are not in the commit.
    ///
    /// Refused rather than recorded: the document binds a claim to a tree state,
    /// and a dirty tree is a state nobody else can reproduce. `--allow-dirty`
    /// does not exist for the same reason.
    ///
    /// **Untracked files count**, which this task's audit had to learn: an
    /// untracked task file changed the document while the commit stayed the
    /// same, so two people at one commit got different attestations and the
    /// document recorded work nobody else had. Ignored files do not count --
    /// they are what the repository has already said are not part of it.
    #[error(
        "the work tree does not match the commit: {modified} tracked path(s) changed and \
         {untracked} untracked path(s) are not in it, so an attestation would name a state \
         nobody else can reproduce; commit, stash or ignore them first"
    )]
    Dirty {
        /// Tracked paths that differ from the commit.
        modified: usize,
        /// Paths git does not track and has not been told to ignore.
        untracked: usize,
    },
}

/// One task's line in the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedTask {
    pub id: String,
    pub status: String,
    /// `kind:reference`, with a `commit` resolved to its full sha and a `file`
    /// carrying the blob sha it had at this commit.
    pub evidence: Vec<AttestedEvidence>,
}

/// One citation, as this commit resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedEvidence {
    pub kind: String,
    /// What the plan wrote.
    pub reference: String,
    /// What a reader can re-derive: the full commit sha, or the file's blob sha.
    /// `None` for the kinds git cannot pin -- `test`, `url`, `note` -- which is
    /// itself information about how strong the citation is.
    pub resolved: Option<String>,
    /// Whether the attested commit can reach this one, for a `commit` citation.
    ///
    /// `None` where the question does not apply. It matters here and nowhere
    /// else in the tool: `verify` asks whether a cited commit's diff touched the
    /// task's `paths` (`APLAN012`), which a commit on an unmerged branch
    /// satisfies perfectly well. An *attestation* is a claim about a tree, and a
    /// reader who checks out the attested commit will not find that work there,
    /// nor keep the object once git collects it. Recorded rather than refused,
    /// because a plan may honestly cite work that is real and not yet merged --
    /// what it may not do is let a document imply otherwise (`A1-040`, found by
    /// its audit).
    pub reachable: Option<bool>,
}

/// Who verified, in terms a reader can check rather than trust (`A1-154`).
///
/// The fifth pilot measured the gap: every development build from `main`
/// reported `aplan 0.3.0-dev`, while their own pin review had already found
/// different capability sets behind that one string, so an auditor could not
/// reconstruct what "verified" meant from the document alone
/// (pre-public adoption evidence).
///
/// Four fields, and what each is worth is different:
///
/// - `version` is the package version, and on its own it is what was already
///   there.
/// - `source` is the commit the build script saw, captured at build time. Empty
///   when there was no checkout to ask and no `APLAN_SOURCE_REV` given, which is
///   the honest answer for a build from a tarball. **Best effort**: a build
///   script runs when cargo thinks its inputs changed, so this names the commit
///   as of that moment rather than a fact anything checked. It is the weakest
///   of the four, and `binary_sha256` is the one that cannot go stale.
/// - `facts_sha256` is the **capability contract**: the digest of this build's
///   own `facts` document, which enumerates its rules, commands, config keys,
///   behaviours, formats and evidence kinds. Two builds whose surfaces differ
///   cannot share it, and nothing about it is caller-supplied.
/// - `binary_sha256` is this executable's digest, read from the running image.
///   **Self-reported**, which is worth stating plainly: it identifies rather
///   than authenticates, and its value is that an auditor can compare it with a
///   digest recorded independently when the binary was pinned -- which is
///   exactly how the pilot that asked for it works.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolIdentity {
    pub version: String,
    pub source: String,
    pub facts_sha256: String,
    pub binary_sha256: String,
}

/// The whole record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attestation {
    /// The commit every claim here is about.
    pub commit: String,
    /// Who verified: enough to tell two builds apart that share a version.
    pub tool: ToolIdentity,
    /// Whether the configured test commands were **run**, or the `test`
    /// citations merely resolved to names a runner lists.
    ///
    /// The difference is most of what a `test` citation is worth, and the
    /// document could not tell the two apart: with `--execute` the runner was
    /// asked to run the test, without it the claim is only that a runner knows
    /// the name (`A1-040`, found by its audit). A reader deciding how much this
    /// document is worth needs the distinction more than the tool does.
    ///
    /// **It says the commands were run, not that running them proved anything.**
    /// `A1-167` measured a runner whose filter matched no test: everything was
    /// skipped, the status was `0`, and it read as a pass. `true` here is
    /// therefore a claim about what this build did, and
    /// [`runners_that_proved_nothing`](Attestation::runners_that_proved_nothing)
    /// is what qualifies it (`A1-178`).
    pub tests_executed: bool,
    /// How many configured runners' passing executions could not be told from a
    /// run that selected nothing (`APLAN037`), and so proved nothing (`A1-178`).
    ///
    /// `tests_executed` says the commands were run. It cannot say they *meant*
    /// anything: `A1-167` measured a runner whose filter matched no test, which
    /// skipped everything, exited `0`, and read as a pass. `verify` reports that
    /// on the line above; without this the attestation carried the strongest
    /// claim this tool can make about a run that did not happen, in the one
    /// artefact whose whole value is being checkable by somebody who was not
    /// there.
    ///
    /// A count and not a refusal, because `verify` judges and `attest` records
    /// (`A1-040`): `APLAN037` is a warning, and a repository that has heard it
    /// and decided to live with it must still be able to attest. What it may not
    /// do is publish a document that hides it.
    ///
    /// Absent from the document when zero, so an attestation from a repository
    /// with nothing to qualify is byte-identical to the one it produced before
    /// this field existed -- which the determinism this command promises
    /// requires.
    pub runners_that_proved_nothing: usize,
    pub tasks: Vec<AttestedTask>,
}

impl Attestation {
    /// The canonical bytes: sorted keys, no timestamp, trailing newline.
    ///
    /// One serialisation, used for the file, for `git notes`, and for anything
    /// that signs it later. A second spelling of the same document would be a
    /// second thing to verify.
    #[must_use]
    pub fn to_json(&self) -> String {
        let tasks: Vec<serde_json::Value> = self
            .tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.id,
                    "status": t.status,
                    "evidence": t.evidence.iter().map(|e| serde_json::json!({
                        "kind": e.kind,
                        "reference": e.reference,
                        "resolved": e.resolved,
                        "reachable": e.reachable,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let mut doc = serde_json::json!({
            "schema": "aplan/attestation/v1",
            "commit": self.commit,
            "tool": {
                "version": self.tool.version,
                // Empty rather than absent: a reader must be able to tell "this
                // build could not say" from "this field is new".
                "source": self.tool.source,
                "facts_sha256": self.tool.facts_sha256,
                "binary_sha256": self.tool.binary_sha256,
            },
            "tests_executed": self.tests_executed,
            "tasks": tasks,
        });
        if self.runners_that_proved_nothing > 0
            && let Some(map) = doc.as_object_mut()
        {
            map.insert(
                "runners_that_proved_nothing".to_string(),
                serde_json::json!(self.runners_that_proved_nothing),
            );
        }
        format!(
            "{}\n",
            serde_json::to_string_pretty(&doc).expect("attestation serialises")
        )
    }
}

/// Refuse a tree that cannot be attested, before anything else looks at it.
///
/// Called first by the command, because the order is part of the answer: with
/// the check buried inside [`attest`], an untracked task file was reported as a
/// *verify* finding about a task the tree does not have, which sends the reader
/// to fix a plan whose real problem is that git has never seen it (`A1-040`,
/// found by its audit's own test). [`attest`] still checks, so a caller that
/// skips this cannot record a dirty tree by accident.
///
/// # Errors
///
/// [`AttestError::Repo`] when the repository cannot be read, and
/// [`AttestError::Dirty`] when the work tree differs from the commit.
pub fn refuse_dirty(repo_root: &Path) -> Result<(), AttestError> {
    let repo = git2::Repository::discover(repo_root)
        .map_err(|e| AttestError::Repo(format!("opening {}: {e}", repo_root.display())))?;
    let (modified, untracked) = dirty_paths(&repo)?;
    if modified > 0 || untracked > 0 {
        return Err(AttestError::Dirty {
            modified,
            untracked,
        });
    }
    Ok(())
}

/// Build the record for `plan` at the repository's `HEAD`.
///
/// The caller runs `verify` first and refuses on its verdict: this function
/// records, it does not judge, so the one place that decides what "verified"
/// means stays `verify` (`A1-040`).
///
/// # Errors
///
/// [`AttestError::Repo`] when the repository cannot be read, and
/// [`AttestError::Dirty`] when the work tree differs from the commit.
pub fn attest(
    plan: &Plan,
    repo_root: &Path,
    tool: ToolIdentity,
    tests_executed: bool,
    runners_that_proved_nothing: usize,
) -> Result<Attestation, AttestError> {
    let repo = git2::Repository::discover(repo_root)
        .map_err(|e| AttestError::Repo(format!("opening {}: {e}", repo_root.display())))?;
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(|e| AttestError::Repo(format!("reading HEAD: {e}")))?;

    let (modified, untracked) = dirty_paths(&repo)?;
    if modified > 0 || untracked > 0 {
        return Err(AttestError::Dirty {
            modified,
            untracked,
        });
    }

    // Blob shas for the tree, looked up once: a plan of 150 tasks citing files
    // would otherwise walk the tree per citation.
    let blobs = blob_shas(&head)?;

    let mut tasks: Vec<AttestedTask> = plan
        .tasks
        .iter()
        .map(|lt| &lt.task)
        // Only what claims to be finished. A `todo` task has nothing to attest,
        // and listing it would pad the document with rows that assert nothing.
        .filter(|t| t.status == Status::Done)
        .map(|t| AttestedTask {
            id: t.id.clone(),
            status: t.status.to_string(),
            evidence: t
                .evidence
                .iter()
                .map(|e| {
                    let (resolved, reachable) = resolve(&repo, head.id(), &blobs, e);
                    AttestedEvidence {
                        kind: e.kind.as_str().to_string(),
                        reference: e.reference.clone(),
                        resolved,
                        reachable,
                    }
                })
                .collect(),
        })
        .collect();
    // Deterministic order, whatever order the loader walked the files in.
    tasks.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(Attestation {
        commit: head.id().to_string(),
        tool,
        tests_executed,
        runners_that_proved_nothing,
        tasks,
    })
}

/// What a reader can re-derive for this citation, and whether the attested
/// commit can reach it.
fn resolve(
    repo: &git2::Repository,
    head: git2::Oid,
    blobs: &BTreeMap<String, String>,
    evidence: &aplan_core::task::Evidence,
) -> (Option<String>, Option<bool>) {
    match evidence.kind {
        aplan_core::task::EvidenceKind::Commit => {
            let Some(oid) = repo
                .revparse_single(&evidence.reference)
                .ok()
                .and_then(|obj| obj.peel_to_commit().ok())
                .map(|c| c.id())
            else {
                return (None, None);
            };
            // The commit itself is reachable from itself, which `git2` reports
            // as `false` because it answers "is a *descendant* of".
            let reachable = oid == head || repo.graph_descendant_of(head, oid).unwrap_or(false);
            (Some(oid.to_string()), Some(reachable))
        }
        aplan_core::task::EvidenceKind::File => (
            {
                // A `file` citation may carry a rev (`path@rev`) and an anchor
                // (`path#section`), which is a place in the file rather than a
                // second file (ADR-0015). The blob is indexed by path alone, so
                // both come off -- through the same functions resolution uses,
                // because a second copy of the rule is how `@` in a path came to
                // be truncated here and nowhere else (`A1-159`).
                let (reference, _) = aplan_core::task::split_rev(&evidence.reference);
                let (path, _) = aplan_core::task::split_anchor(reference);
                blobs.get(path).cloned()
            },
            // A file is read *at* the attested commit, so reachability is not a
            // question the kind can be asked.
            None,
        ),
        _ => (None, None),
    }
}

/// Every tracked path at this commit, with its blob sha.
fn blob_shas(head: &git2::Commit<'_>) -> Result<BTreeMap<String, String>, AttestError> {
    let tree = head
        .tree()
        .map_err(|e| AttestError::Repo(format!("reading the tree: {e}")))?;
    let mut out = BTreeMap::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
        // `Ok` rather than `Some` since git2 0.21: the name is now a `Result`
        // whose `Err` is the case the old `None` meant, a path git holds as
        // bytes that are not UTF-8. Skipping it is the behaviour this had
        // before and is not this bump's to change.
        if entry.kind() == Some(git2::ObjectType::Blob)
            && let Ok(name) = entry.name()
        {
            out.insert(format!("{dir}{name}"), entry.id().to_string());
        }
        git2::TreeWalkResult::Ok
    })
    .map_err(|e| AttestError::Repo(format!("walking the tree: {e}")))?;
    Ok(out)
}

/// How many paths differ between the work tree and the commit, split by kind.
///
/// Untracked paths are counted because an attestation is about a tree somebody
/// else can check out: a file git has never seen is not in that tree, and a plan
/// read partly from one is a plan nobody else has (`A1-040`, found by its audit,
/// and the hazard `A1-144` already named for the generated rollup). Ignored
/// paths are excluded, because ignoring one is the repository saying it is not
/// part of the tree.
fn dirty_paths(repo: &git2::Repository) -> Result<(usize, usize), AttestError> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| AttestError::Repo(format!("reading status: {e}")))?;
    let mut modified = 0;
    let mut untracked = 0;
    for entry in statuses.iter() {
        let status = entry.status();
        if status.is_empty() {
            continue;
        }
        if status.contains(git2::Status::WT_NEW) {
            untracked += 1;
        } else {
            modified += 1;
        }
    }
    Ok((modified, untracked))
}

/// Attach a document to `HEAD` as a git note, under `refs/notes/<r#ref>`.
///
/// The seam the acceptance criteria call "pluggable": a note is *attachment*,
/// not signature -- it is tamper-evident because git hashes it, not because
/// anybody signed it -- and a real signer (sigstore, GPG) is a second
/// implementation of this shape rather than a change to the document. The
/// unsigned document is the useful thing on day one, which is the point of
/// keeping them separate (`A1-040`).
///
/// # Errors
///
/// [`AttestError::Repo`] when the note cannot be written, including when one is
/// already attached: overwriting somebody else's record is not this command's
/// decision.
pub fn attach_note(repo_root: &Path, notes: &str, body: &str) -> Result<String, AttestError> {
    let repo = git2::Repository::discover(repo_root)
        .map_err(|e| AttestError::Repo(format!("opening {}: {e}", repo_root.display())))?;
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(|e| AttestError::Repo(format!("reading HEAD: {e}")))?;
    let sig = repo
        .signature()
        .map_err(|e| AttestError::Repo(format!("reading the git identity: {e}")))?;
    let notes_ref = format!("refs/notes/{notes}");
    repo.note(&sig, &sig, Some(&notes_ref), head.id(), body, false)
        .map(|oid| oid.to_string())
        .map_err(|e| {
            AttestError::Repo(format!(
                "attaching a note to {} on `{notes_ref}`: {e}",
                head.id()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_core::task::{Evidence, EvidenceKind, Task};

    fn task(id: &str, status: Status, evidence: Vec<Evidence>) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            phase: 1,
            status,
            owner: None,
            effort_days: Some(1.0),
            blocked_by: Vec::new(),
            prefer_after: Vec::new(),
            evidence,
            paths: Vec::new(),
            traces: Vec::new(),
            trace_fingerprints: BTreeMap::new(),
            tags: Vec::new(),
            revisit_at: None,
            version_impact: None,
            version_impact_reason: None,
        }
    }

    /// A fixed identity, so a serialisation test is about the serialisation.
    fn identity(version: &str) -> ToolIdentity {
        ToolIdentity {
            version: version.to_string(),
            source: "abc123".to_string(),
            facts_sha256: "f".repeat(64),
            binary_sha256: "b".repeat(64),
        }
    }

    fn evidence(kind: EvidenceKind, reference: &str) -> Evidence {
        Evidence {
            kind,
            reference: reference.to_string(),
            captured: None,
        }
    }

    /// The document is a function of its inputs, and the serialisation is where
    /// that becomes checkable: sorted keys, no clock, one trailing newline.
    #[test]
    fn the_serialisation_is_canonical_and_carries_no_clock() {
        let doc = Attestation {
            commit: "abc".to_string(),
            tool: identity("0.0.0"),
            tests_executed: false,
            runners_that_proved_nothing: 0,
            tasks: vec![AttestedTask {
                id: "A-01".to_string(),
                status: "done".to_string(),
                evidence: vec![AttestedEvidence {
                    kind: "commit".to_string(),
                    reference: "abc1234".to_string(),
                    resolved: Some("abc".to_string()),
                    reachable: Some(true),
                }],
            }],
        };
        let json = doc.to_json();
        assert_eq!(json, doc.to_json(), "two serialisations agree");
        assert!(json.ends_with("}\n"), "one trailing newline: {json:?}");
        // Sorted keys: `commit` before `schema` before `tasks`, whatever order
        // the struct declares them in.
        let commit = json.find("\"commit\"").expect("commit");
        let schema = json.find("\"schema\"").expect("schema");
        let tasks = json.find("\"tasks\"").expect("tasks");
        assert!(commit < schema && schema < tasks, "{json}");
        assert!(
            !json.contains("date") && !json.contains("time"),
            "a clock would make it a function of when it ran: {json}"
        );
    }

    /// What cannot be pinned says so, rather than being left out: a `test`
    /// citation is a name a runner lists, and git has nothing to hash for it.
    #[test]
    fn a_citation_git_cannot_pin_is_recorded_as_unresolved() {
        let doc = Attestation {
            commit: "abc".to_string(),
            tool: identity("t"),
            tests_executed: false,
            runners_that_proved_nothing: 0,
            tasks: vec![AttestedTask {
                id: "A-01".to_string(),
                status: "done".to_string(),
                evidence: vec![AttestedEvidence {
                    kind: "test".to_string(),
                    reference: "mod::it_works".to_string(),
                    resolved: None,
                    reachable: None,
                }],
            }],
        };
        let json = doc.to_json();
        assert!(json.contains("\"resolved\": null"), "{json}");
        assert!(
            json.contains("\"reachable\": null"),
            "a question the kind cannot be asked is null, not absent: {json}"
        );
        assert!(
            json.contains("mod::it_works"),
            "and it is still listed: {json}"
        );
    }

    /// Only a claim can be attested: a `todo` task has nothing to record, and
    /// listing it would pad the document with rows that assert nothing.
    #[test]
    fn only_finished_work_reaches_the_document() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("init");
        std::fs::write(dir.path().join("f.txt"), "x").expect("write");
        let sha = commit_all(&repo, "first");

        let done = task(
            "A-01",
            Status::Done,
            vec![evidence(EvidenceKind::Commit, &sha[..8])],
        );
        let todo = task("A-02", Status::Todo, Vec::new());
        let plan = plan_of(vec![done, todo]);

        let doc = attest(&plan, dir.path(), identity("test"), false, 0).expect("attests");
        assert_eq!(doc.tasks.len(), 1);
        assert_eq!(doc.tasks[0].id, "A-01");
        assert_eq!(
            doc.tasks[0].evidence[0].resolved.as_deref(),
            Some(sha.as_str()),
            "the short ref is resolved to the full sha"
        );
        assert_eq!(
            doc.tasks[0].evidence[0].reachable,
            Some(true),
            "the attested commit reaches its own citation"
        );
        assert!(
            !doc.tests_executed,
            "no test was run, and the document has to say which claim it is making"
        );
        assert!(
            attest(&plan, dir.path(), identity("test"), true, 0)
                .expect("attests")
                .tests_executed,
            "and the other way when `--execute` ran them"
        );
    }

    /// A citation to work on a branch nobody merged (`A1-040`, found by its
    /// audit).
    ///
    /// `verify` accepts it: `APLAN012` asks whether the cited commit's diff
    /// touched the task's `paths`, and a commit on a side branch does. The
    /// attestation is where that stops being enough, because a reader who checks
    /// out the attested commit will not find the work in its history.
    #[test]
    fn a_citation_the_attested_commit_cannot_reach_says_so() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("init");
        std::fs::write(dir.path().join("f.txt"), "x").expect("write");
        let base = commit_all(&repo, "first");

        // A commit made on a branch, then left behind by moving back.
        repo.set_head_detached(git2::Oid::from_str(&base).expect("oid"))
            .expect("detach");
        std::fs::write(dir.path().join("side.txt"), "y").expect("write");
        let side = commit_all(&repo, "on a branch nobody merged");
        repo.set_head_detached(git2::Oid::from_str(&base).expect("oid"))
            .expect("back");
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .expect("checkout");

        let plan = plan_of(vec![
            task(
                "A-01",
                Status::Done,
                vec![evidence(EvidenceKind::Commit, &base)],
            ),
            task(
                "A-02",
                Status::Done,
                vec![evidence(EvidenceKind::Commit, &side)],
            ),
        ]);
        let doc = attest(&plan, dir.path(), identity("test"), false, 0).expect("attests");

        assert_eq!(doc.commit, base, "attested at the base commit");
        assert_eq!(doc.tasks[0].evidence[0].reachable, Some(true));
        assert_eq!(
            doc.tasks[1].evidence[0].reachable,
            Some(false),
            "the side commit is real, resolvable, and not in this history"
        );
        // Still recorded rather than refused: the citation is not a lie, and the
        // document's job is to say exactly what is true of it.
        assert_eq!(
            doc.tasks[1].evidence[0].resolved.as_deref(),
            Some(side.as_str())
        );
    }

    fn plan_of(tasks: Vec<Task>) -> Plan {
        Plan {
            tasks: tasks
                .into_iter()
                .map(|task| aplan_core::load::LoadedTask {
                    path: std::path::PathBuf::from(format!("{}.md", task.id)),
                    fields: aplan_core::load::FieldLines::new(),
                    criteria: aplan_core::load::Criteria::default(),
                    task,
                })
                .collect(),
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        }
    }

    fn commit_all(repo: &git2::Repository, message: &str) -> String {
        let mut index = repo.index().expect("index");
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .expect("add");
        index.write().expect("write");
        let tree = repo
            .find_tree(index.write_tree().expect("tree"))
            .expect("tree");
        let sig = git2::Signature::now("t", "t@example.com").expect("sig");
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .expect("commit")
            .to_string()
    }
}

/// This build's identity, assembled from what it can know about itself
/// (`A1-154`).
///
/// `facts` is the build's own facts document, passed in rather than reached for:
/// this crate holds the record, and the surface it describes belongs to the
/// binary. That also makes the digest testable with a document of the caller's
/// choosing.
///
/// Every field is derived. Nothing here takes a label, a score, or a claim from
/// the caller, which is the difference between an identity and a signature line.
#[must_use]
pub fn identify(facts: &str) -> ToolIdentity {
    // `APLAN_SOURCE_REV` is empty exactly when the build had no checkout to ask
    // and no override -- the tarball case this crate documents as supported, and
    // the one a git hook creates by exporting a relative `GIT_DIR` that the
    // build script cannot resolve from its own directory.
    //
    // `clippy::manual_string_new` reads the empty literal and suggests
    // `String::new()`. That would be correct if the value were always empty, and
    // here it is the same expression whichever answer the build script found. The
    // lint fires under `-D warnings`, so **a build with no discoverable commit
    // did not compile at all** -- which is the opposite of the honest empty
    // answer three of this crate's doc comments promise (`A1-256`, audit).
    #[allow(clippy::manual_string_new)]
    let source = env!("APLAN_SOURCE_REV").to_string();
    ToolIdentity {
        version: env!("CARGO_PKG_VERSION").to_string(),
        source,
        facts_sha256: sha256_hex(facts.as_bytes()),
        // A binary that cannot read itself says nothing rather than guessing:
        // the field is empty, and the document is still useful for the three
        // identities that do not depend on the filesystem.
        binary_sha256: std::env::current_exe()
            .ok()
            .and_then(|path| std::fs::read(path).ok())
            .map_or_else(String::new, |bytes| sha256_hex(&bytes)),
    }
}

/// Lowercase hex SHA-256, the spelling `sha256sum` prints.
///
/// The one place a digest is computed, so the document's numbers are the ones an
/// auditor reproduces with the tool they already have.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    digest.iter().fold(String::with_capacity(64), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    })
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    /// The digests are the ones an auditor reproduces with the tool they have.
    ///
    /// Pinned against the NIST vector rather than against this implementation,
    /// so "our hash agrees with itself" cannot pass for "this is SHA-256".
    #[test]
    fn the_digest_is_sha256_as_everything_else_spells_it() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// Two builds that share a version must not share an identity, which is the
    /// whole finding (`A1-154`, pre-public adoption evidence).
    #[test]
    fn a_changed_capability_surface_changes_the_identity() {
        let one = identify("{\"commands\":[\"check\"]}\n");
        let two = identify("{\"commands\":[\"check\",\"verify\"]}\n");
        assert_eq!(one.version, two.version, "the version is the same string");
        assert_ne!(
            one.facts_sha256, two.facts_sha256,
            "and the capability contract is not"
        );
        // Nothing here is caller-supplied but the document being described.
        assert_eq!(one.facts_sha256.len(), 64);
        assert!(one.facts_sha256.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// The identity reaches the document, and a different one changes its bytes
    /// while the same one leaves them alone.
    #[test]
    fn the_document_changes_with_the_identity_and_not_otherwise() {
        let base = Attestation {
            commit: "abc".to_string(),
            tool: ToolIdentity {
                version: "0.1.0".to_string(),
                source: "rev1".to_string(),
                facts_sha256: "a".repeat(64),
                binary_sha256: "b".repeat(64),
            },
            tests_executed: false,
            runners_that_proved_nothing: 0,
            tasks: Vec::new(),
        };
        assert_eq!(
            base.to_json(),
            base.clone().to_json(),
            "same identity, same bytes"
        );

        for changed in [
            ToolIdentity {
                version: "0.2.0".to_string(),
                ..base.tool.clone()
            },
            ToolIdentity {
                source: "rev2".to_string(),
                ..base.tool.clone()
            },
            ToolIdentity {
                facts_sha256: "c".repeat(64),
                ..base.tool.clone()
            },
            ToolIdentity {
                binary_sha256: "d".repeat(64),
                ..base.tool.clone()
            },
        ] {
            let other = Attestation {
                tool: changed,
                ..base.clone()
            };
            assert_ne!(
                base.to_json(),
                other.to_json(),
                "every field of the identity is in the document"
            );
        }
    }

    /// A build that cannot say says nothing, rather than inventing a value: the
    /// field is present and empty, so a reader can tell "this build could not
    /// tell you" from "this field did not exist yet".
    #[test]
    fn an_unknown_field_is_empty_and_still_present() {
        let doc = Attestation {
            commit: "abc".to_string(),
            tool: ToolIdentity {
                version: "0.1.0".to_string(),
                source: String::new(),
                facts_sha256: "a".repeat(64),
                binary_sha256: String::new(),
            },
            tests_executed: false,
            runners_that_proved_nothing: 0,
            tasks: Vec::new(),
        };
        let json = doc.to_json();
        assert!(json.contains("\"source\": \"\""), "{json}");
        assert!(json.contains("\"binary_sha256\": \"\""), "{json}");
    }
}
