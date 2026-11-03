//! Atomic parking and unparking of existing portfolio work.
//!
//! Parking is deliberately a multi-task guarded mutation. A loop of status
//! edits can temporarily expose work through `ready` or hide only part of a
//! portfolio from `due`; this preview computes, validates, and publishes the
//! complete requested post-state as one [`crate::ChangeSet`].

use std::collections::BTreeSet;

use aplan_core::{Plan, Status};

use crate::{ChangeEvidence, ChangeSet, FileChange, PlanContext, SemanticEffect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortfolioOperation {
    Park {
        reason: String,
        revisit_at: Option<String>,
        tags: Vec<String>,
    },
    Unpark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioRequest {
    pub ids: Vec<String>,
    pub operation: PortfolioOperation,
}

#[derive(Debug)]
pub struct PortfolioPreview {
    change: ChangeSet,
    affected: Vec<String>,
}

impl PortfolioPreview {
    #[must_use]
    pub const fn change_set(&self) -> &ChangeSet {
        &self.change
    }

    #[must_use]
    pub fn affected(&self) -> &[String] {
        &self.affected
    }

    #[must_use]
    pub fn into_change_set(self) -> ChangeSet {
        self.change
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PortfolioError {
    #[error("{0}")]
    Arguments(String),
    #[error("{0}")]
    Plan(String),
    #[error("{0}")]
    Refused(String),
    #[error("could not rewrite `{id}`: {source}")]
    Rewrite {
        id: String,
        #[source]
        source: aplan_core::CloseError,
    },
}

pub fn preview_portfolio(
    context: &PlanContext,
    request: &PortfolioRequest,
) -> Result<PortfolioPreview, PortfolioError> {
    require_clean(context.plan())?;
    validate_request(request)?;
    let claims = crate::claims::snapshot(&context.identity().repository_root, context.plan())
        .map_err(|error| {
            PortfolioError::Refused(format!(
                "claims could not be checked, so parking state cannot be changed safely: {error}"
            ))
        })?;
    if !claims.scope.available {
        return Err(PortfolioError::Refused(format!(
            "claims are unavailable, so parking state cannot be changed safely: {}",
            claims
                .scope
                .reason
                .as_deref()
                .unwrap_or("no shared coordination scope")
        )));
    }

    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    for id in &request.ids {
        if !seen.insert(id) {
            return Err(PortfolioError::Arguments(format!(
                "task `{id}` was named more than once; name each task once"
            )));
        }
        let target = unique_task(context.plan(), id)?;
        if let Some(claim) = claims.claim(id) {
            return Err(PortfolioError::Refused(format!(
                "`{id}` is claimed by `{}`; release the claim before changing its parking state",
                claim.holder
            )));
        }
        validate_target(target.task.status, id, &request.operation)?;
        targets.push(target);
    }

    let mut after = aplan_core::mutation::clone_of(context.plan());
    let mut files = Vec::with_capacity(targets.len());
    let mut effects = Vec::new();
    for target in targets {
        let id = &target.task.id;
        let original = context.source_text(&target.path).ok_or_else(|| {
            PortfolioError::Refused(format!(
                "`{id}` changed while the edit was being prepared; reload and try again"
            ))
        })?;
        let (updated, to) =
            rewrite_target(original, &target.task, &request.operation).map_err(|source| {
                PortfolioError::Rewrite {
                    id: id.clone(),
                    source,
                }
            })?;
        let parsed = aplan_core::mutation::task_of(&updated)
            .map_err(|error| PortfolioError::Plan(format!("`{id}`: {error}")))?;
        after = aplan_core::mutation::with_replaced(&after, id, parsed);
        files.push(FileChange::replacement(
            target.path.clone(),
            original.to_string(),
            updated,
        ));
        effects.push(SemanticEffect::TaskStatusChanged {
            task: id.clone(),
            from: target.task.status,
            to,
        });
        if let PortfolioOperation::Park { tags, .. } = &request.operation {
            for tag in aplan_interop::emit::deduped(tags) {
                if !target.task.tags.contains(&tag) {
                    effects.push(SemanticEffect::TagAdded {
                        task: id.clone(),
                        tag,
                    });
                }
            }
        }
    }
    let diagnostics = aplan_core::mutation::delta(context.plan(), &after, context.options());
    let denied = diagnostics.refuses(context.verdict());
    Ok(PortfolioPreview {
        affected: request.ids.clone(),
        change: ChangeSet::from_parts(
            context.identity().clone(),
            files,
            effects,
            diagnostics,
            denied,
            ChangeEvidence::NotRequired,
        ),
    })
}

fn validate_request(request: &PortfolioRequest) -> Result<(), PortfolioError> {
    if request.ids.is_empty() {
        return Err(PortfolioError::Arguments(
            "park/unpark needs at least one task".to_string(),
        ));
    }
    if request
        .ids
        .iter()
        .any(|id| aplan_core::task::is_vacuous_ref(id))
    {
        return Err(PortfolioError::Arguments(
            "a task id may not be blank or visually empty".to_string(),
        ));
    }
    if let PortfolioOperation::Park {
        reason,
        revisit_at,
        tags,
    } = &request.operation
    {
        if aplan_core::task::is_vacuous_ref(reason) {
            return Err(PortfolioError::Arguments(
                "parking needs a visible reason to record in each task body".to_string(),
            ));
        }
        if tags.iter().any(|tag| aplan_core::task::is_vacuous_ref(tag)) {
            return Err(PortfolioError::Arguments(
                "a parking tag may not be blank or visually empty".to_string(),
            ));
        }
        if let Some(at) = revisit_at
            && aplan_core::validate::rfc3339_to_epoch(at).is_none()
        {
            return Err(PortfolioError::Arguments(format!(
                "`--revisit-at` must be an RFC 3339 instant, not `{at}`"
            )));
        }
    }
    Ok(())
}

fn validate_target(
    status: Status,
    id: &str,
    operation: &PortfolioOperation,
) -> Result<(), PortfolioError> {
    match operation {
        PortfolioOperation::Park { .. } if status == Status::Todo => Ok(()),
        PortfolioOperation::Park { .. } if status == Status::InProgress => {
            Err(PortfolioError::Refused(format!(
                "`{id}` is `in_progress`; stop active work before parking it"
            )))
        }
        PortfolioOperation::Park { .. } if status == Status::Blocked => {
            Err(PortfolioError::Refused(format!(
                "`{id}` is already parked; unpark it before recording a new parking decision"
            )))
        }
        PortfolioOperation::Park { .. } => Err(PortfolioError::Refused(format!(
            "`{id}` is `{status}`; terminal work cannot be parked"
        ))),
        PortfolioOperation::Unpark if status == Status::Blocked => Ok(()),
        PortfolioOperation::Unpark if status.is_terminal() => Err(PortfolioError::Refused(
            format!("`{id}` is `{status}`; terminal work cannot be revived"),
        )),
        PortfolioOperation::Unpark => Err(PortfolioError::Refused(format!(
            "`{id}` is `{status}`, not parked"
        ))),
    }
}

fn rewrite_target(
    original: &str,
    task: &aplan_core::Task,
    operation: &PortfolioOperation,
) -> Result<(String, Status), aplan_core::CloseError> {
    match operation {
        PortfolioOperation::Park {
            reason,
            revisit_at,
            tags,
        } => {
            let mut updated = aplan_core::close::set_status(original, Status::Blocked)?;
            updated = aplan_core::close::set_revisit_at(&updated, revisit_at.as_deref())?;
            let mut merged = task.tags.clone();
            merged.extend(tags.iter().cloned());
            updated =
                aplan_core::close::set_tags(&updated, &aplan_interop::emit::deduped(&merged))?;
            append_reason(&mut updated, reason);
            Ok((updated, Status::Blocked))
        }
        PortfolioOperation::Unpark => {
            let updated = aplan_core::close::set_status(original, Status::Todo)?;
            let updated = aplan_core::close::set_revisit_at(&updated, None)?;
            Ok((updated, Status::Todo))
        }
    }
}

fn append_reason(file: &mut String, reason: &str) {
    if !file.ends_with('\n') {
        file.push('\n');
    }
    if !file.ends_with("\n\n") {
        file.push('\n');
    }
    file.push_str("## Parked\n\n");
    file.push_str(reason.trim());
    file.push('\n');
}

fn require_clean(plan: &Plan) -> Result<(), PortfolioError> {
    if plan.load_diagnostics.is_empty() {
        Ok(())
    } else {
        Err(PortfolioError::Plan(
            "the plan does not load cleanly; repair it before changing parking state".to_string(),
        ))
    }
}

fn unique_task<'a>(
    plan: &'a Plan,
    id: &str,
) -> Result<&'a aplan_core::load::LoadedTask, PortfolioError> {
    if let Some(reason) = aplan_core::mutation::ambiguous_id(plan, id) {
        return Err(PortfolioError::Refused(reason));
    }
    plan.tasks
        .iter()
        .find(|loaded| loaded.task.id == id)
        .ok_or_else(|| PortfolioError::Arguments(format!("no task `{id}` in this plan")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{CommitOutcome, ContextInputs, Trust, commit_change_set};

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, text).expect("write");
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let output = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .output()
            .expect("git init");
        assert!(output.status.success());
        write(
            &dir.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/one.rs]\n---\n\nOriginal body.\n",
        );
        write(
            &dir.path().join("aplan/tasks/A-2.md"),
            "---\nid: A-2\ntitle: two\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/two.rs]\n---\n",
        );
        dir
    }

    fn context(dir: &Path) -> PlanContext {
        PlanContext::new(
            aplan_core::load_plan_identified(&dir.join("aplan")),
            ContextInputs {
                repository_root: dir.to_path_buf(),
                configuration_file: None,
                trust: Trust::RepositoryRead,
                options: aplan_core::Options::default(),
                verdict: aplan_core::Verdict::default(),
            },
        )
        .with_report_config(aplan_config::Config::default())
    }

    #[test]
    fn portfolio_preview_publishes_every_parked_file_or_none_and_unparks_without_erasing_history() {
        let dir = fixture();
        let initial = context(dir.path());
        let preview = preview_portfolio(
            &initial,
            &PortfolioRequest {
                ids: vec!["A-1".to_string(), "A-2".to_string()],
                operation: PortfolioOperation::Park {
                    reason: "External decision is pending.".to_string(),
                    revisit_at: Some("2030-01-02T03:04:05Z".to_string()),
                    tags: vec!["post-mvp".to_string()],
                },
            },
        )
        .expect("preview park");
        assert_eq!(preview.change_set().files().len(), 2);
        assert_eq!(preview.affected(), ["A-1", "A-2"]);
        assert!(preview.change_set().files().iter().all(|file| {
            file.after().contains("status: blocked")
                && file.after().contains("## Parked")
                && file.after().contains("post-mvp")
        }));
        assert!(matches!(
            commit_change_set(&initial, preview.into_change_set()),
            CommitOutcome::Applied(_)
        ));

        let parked = context(dir.path());
        let preview = preview_portfolio(
            &parked,
            &PortfolioRequest {
                ids: vec!["A-1".to_string(), "A-2".to_string()],
                operation: PortfolioOperation::Unpark,
            },
        )
        .expect("preview unpark");
        assert!(preview.change_set().files().iter().all(|file| {
            file.after().contains("status: todo")
                && !file.after().contains("revisit_at:")
                && file.after().contains("## Parked")
                && file.after().contains("post-mvp")
        }));
    }

    #[test]
    fn one_invalid_portfolio_target_refuses_the_complete_preview() {
        let dir = fixture();
        let active = dir.path().join("aplan/tasks/A-2.md");
        let text = std::fs::read_to_string(&active).expect("active");
        std::fs::write(&active, text.replace("status: todo", "status: in_progress"))
            .expect("activate");
        let context = context(dir.path());
        let error = preview_portfolio(
            &context,
            &PortfolioRequest {
                ids: vec!["A-1".to_string(), "A-2".to_string()],
                operation: PortfolioOperation::Park {
                    reason: "Not yet.".to_string(),
                    revisit_at: None,
                    tags: Vec::new(),
                },
            },
        )
        .expect_err("active target refuses all");
        assert!(error.to_string().contains("stop active work"));
        let untouched =
            std::fs::read_to_string(dir.path().join("aplan/tasks/A-1.md")).expect("untouched");
        assert!(untouched.contains("status: todo"));
        assert!(!untouched.contains("## Parked"));
    }

    #[test]
    fn portfolio_boundaries_refuse_invalid_requests_unreadable_plans_and_uncoordinated_repositories()
     {
        let dir = fixture();
        let loaded = context(dir.path());
        for ids in [Vec::new(), vec!["\u{200b}".to_string()]] {
            let error = preview_portfolio(
                &loaded,
                &PortfolioRequest {
                    ids,
                    operation: PortfolioOperation::Unpark,
                },
            )
            .expect_err("invalid target list");
            assert!(
                error.to_string().contains("at least one")
                    || error.to_string().contains("visually empty")
            );
        }

        write(
            &dir.path().join("aplan/tasks/broken.md"),
            "---\nid: broken\ntitle: [\n---\n",
        );
        let broken = context(dir.path());
        assert!(
            preview_portfolio(
                &broken,
                &PortfolioRequest {
                    ids: vec!["A-1".to_string()],
                    operation: PortfolioOperation::Unpark,
                },
            )
            .expect_err("an incomplete plan cannot be mutated")
            .to_string()
            .contains("does not load cleanly")
        );

        let outside_git = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        write(
            &outside_git.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/one.rs]\n---\n",
        );
        let uncoordinated = context(outside_git.path());
        let error = preview_portfolio(
            &uncoordinated,
            &PortfolioRequest {
                ids: vec!["A-1".to_string()],
                operation: PortfolioOperation::Park {
                    reason: "Wait.".to_string(),
                    revisit_at: None,
                    tags: Vec::new(),
                },
            },
        )
        .expect_err("claim scope must be observable");
        assert!(
            error.to_string().contains("claims are unavailable"),
            "{error}"
        );

        let no_newline = fixture();
        write(
            &no_newline.path().join("aplan/tasks/A-1.md"),
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\npaths: [src/one.rs]\n---\n\nBody without newline",
        );
        let preview = preview_portfolio(
            &context(no_newline.path()),
            &PortfolioRequest {
                ids: vec!["A-1".to_string()],
                operation: PortfolioOperation::Park {
                    reason: "Wait.".to_string(),
                    revisit_at: None,
                    tags: Vec::new(),
                },
            },
        )
        .expect("body without final newline remains editable");
        assert!(
            preview.change_set().files()[0]
                .after()
                .contains("Body without newline\n\n## Parked")
        );
    }
}
