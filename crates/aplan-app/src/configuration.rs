//! Repository configuration projected into application policy.
//!
//! Delivery adapters may choose different inputs, but they must not maintain
//! different interpretations of one `.aplan.yml`. This module is the shared
//! boundary between the configuration document and the core/evidence options
//! consumed by the CLI, MCP, and interactive application layers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aplan_config::{Cardinality, Config, MilestoneScope, StatusScope};
use aplan_core::validate::{EvidenceScope, Options, TagFamily};

/// How an explicit long-lived-adapter root was interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplicitRootKind {
    /// The selected directory owns `.aplan.yml` (or is a config-free Git root),
    /// so configuration chooses the plan directory below it.
    Repository,
    /// The selected directory is the configured plan directory itself.
    Plan,
}

/// One coherent repository/configuration/plan selection.
#[derive(Debug, Clone)]
pub struct ResolvedRoot {
    pub selection: ExplicitRootKind,
    pub plan_root: PathBuf,
    pub repository_root: PathBuf,
    pub configuration_file: Option<PathBuf>,
    pub configuration_directory: Option<PathBuf>,
    pub config: Config,
    pub followed_symlink: bool,
}

/// Why an explicit adapter root could not denote one coherent context.
#[derive(Debug, thiserror::Error)]
pub enum RootResolutionError {
    #[error(
        "`root` is blank; omit it to use the server's configured repository, or name a directory"
    )]
    Blank,
    #[error("root `{path}` does not exist or cannot be resolved: {source}")]
    Missing {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("root `{0}` is not a directory")]
    NotDirectory(PathBuf),
    #[error("reading configuration selected by root `{root}`: {source}")]
    Configuration {
        root: PathBuf,
        source: aplan_config::Error,
    },
    #[error(
        "root `{selected}` is inside the repository but is not its configured plan root `{configured}`; pass either `{repository}` or `{configured}`"
    )]
    Ambiguous {
        selected: PathBuf,
        repository: PathBuf,
        configured: PathBuf,
    },
    #[error(
        "root `{selected}` selects configuration `{configuration}`, whose plan root `{plan}` is not a directory"
    )]
    MissingPlan {
        selected: PathBuf,
        configuration: PathBuf,
        plan: PathBuf,
    },
    #[error(
        "config-free repository root `{repository}` has no default plan directory `{plan}`; create it, add `.aplan.yml` with `root`, or pass an existing plan directory"
    )]
    MissingDefaultPlan { repository: PathBuf, plan: PathBuf },
    #[error(
        "config-free root `{selected}` is inside Git worktree `{repository}` but is neither the repository root nor its default plan root `{plan}`"
    )]
    AmbiguousConfigFree {
        selected: PathBuf,
        repository: PathBuf,
        plan: PathBuf,
    },
}

/// Resolve an explicit root supplied to a long-lived adapter.
///
/// A directory carrying `.aplan.yml` is a repository-form selection. A
/// configured plan directory is the task-directory form. In a config-free Git
/// worktree only the worktree root and its conventional `aplan` child are
/// meaningful; a directory outside Git remains a plan-form selection for
/// backwards compatibility. Every returned path is canonical, so a symlink
/// selects its target's configuration and repository rather than mixing the
/// link's lexical parent with the target's task bytes.
pub fn resolve_explicit_root(root: &str) -> Result<ResolvedRoot, RootResolutionError> {
    if root.trim().is_empty() {
        return Err(RootResolutionError::Blank);
    }
    let requested = PathBuf::from(root);
    let absolute = std::path::absolute(&requested).unwrap_or_else(|_| requested.clone());
    let selected = requested
        .canonicalize()
        .map_err(|source| RootResolutionError::Missing {
            path: absolute.clone(),
            source,
        })?;
    if !selected.is_dir() {
        return Err(RootResolutionError::NotDirectory(absolute));
    }
    let followed_symlink = selected != normalise_absolute(&absolute);

    if let Some(resolved) = resolve_local_configuration(&selected, followed_symlink)? {
        return Ok(resolved);
    }

    let git_root = git_worktree_root(&selected);
    if let Some(configuration_file) = aplan_config::discovered_config(&selected) {
        let belongs_to_worktree = git_root
            .as_ref()
            .is_none_or(|repository| configuration_file.starts_with(repository));
        let (config, directory) = aplan_config::load_from(
            &aplan_config::ConfigSource::File(configuration_file.clone()),
            &selected,
        )
        .map_err(|source| RootResolutionError::Configuration {
            root: selected.clone(),
            source,
        })?;
        let declared = configured_plan_root(&config, directory.as_deref());
        let plan_root = declared.canonicalize().ok();
        if plan_root.as_deref() == Some(selected.as_path()) {
            return Ok(ResolvedRoot {
                selection: ExplicitRootKind::Plan,
                repository_root: plan_repository(&selected),
                plan_root: selected,
                configuration_file: Some(configuration_file),
                configuration_directory: directory,
                config,
                followed_symlink,
            });
        }
        // A config outside a nested worktree is not that worktree's policy
        // unless it explicitly names this plan. This keeps a config-free nested
        // repository from inheriting an outer checkout's unrelated defaults.
        if belongs_to_worktree {
            let configured = canonical_plan(&selected, &configuration_file, declared)?;
            return Err(RootResolutionError::Ambiguous {
                selected,
                repository: directory.unwrap_or_else(|| plan_repository(&configured)),
                configured,
            });
        }
    }

    resolve_config_free(selected, git_root, followed_symlink)
}

fn resolve_local_configuration(
    selected: &Path,
    followed_symlink: bool,
) -> Result<Option<ResolvedRoot>, RootResolutionError> {
    let local_config = selected.join(aplan_config::CONFIG_FILE);
    if local_config.symlink_metadata().is_err() {
        return Ok(None);
    }
    let (config, directory) = aplan_config::load_from(
        &aplan_config::ConfigSource::File(local_config.clone()),
        selected,
    )
    .map_err(|source| RootResolutionError::Configuration {
        root: selected.to_path_buf(),
        source,
    })?;
    let plan_root = configured_plan_root(&config, directory.as_deref());
    let plan_root = canonical_plan(selected, &local_config, plan_root)?;
    Ok(Some(ResolvedRoot {
        selection: ExplicitRootKind::Repository,
        repository_root: plan_repository(&plan_root),
        plan_root,
        configuration_file: Some(local_config),
        configuration_directory: directory,
        config,
        followed_symlink,
    }))
}

fn resolve_config_free(
    selected: PathBuf,
    git_root: Option<PathBuf>,
    followed_symlink: bool,
) -> Result<ResolvedRoot, RootResolutionError> {
    let Some(repository_root) = git_root else {
        return Ok(ResolvedRoot {
            selection: ExplicitRootKind::Plan,
            repository_root: selected
                .parent()
                .map_or_else(|| selected.clone(), Path::to_path_buf),
            plan_root: selected,
            configuration_file: None,
            configuration_directory: None,
            config: Config::default(),
            followed_symlink,
        });
    };
    let conventional = repository_root.join("aplan");
    let actual_plan = conventional.canonicalize().ok();
    let (selection, plan_root) = if selected == repository_root {
        let Some(actual_plan) = actual_plan else {
            return Err(RootResolutionError::MissingDefaultPlan {
                repository: repository_root,
                plan: conventional,
            });
        };
        (ExplicitRootKind::Repository, actual_plan)
    } else if actual_plan.as_deref() == Some(selected.as_path()) {
        (ExplicitRootKind::Plan, selected)
    } else {
        return Err(RootResolutionError::AmbiguousConfigFree {
            selected,
            repository: repository_root,
            plan: conventional,
        });
    };
    let repository_root = plan_repository(&plan_root);
    Ok(ResolvedRoot {
        selection,
        plan_root,
        repository_root,
        configuration_file: None,
        configuration_directory: None,
        config: Config::default(),
        followed_symlink,
    })
}

fn configured_plan_root(config: &Config, config_dir: Option<&Path>) -> PathBuf {
    let declared = PathBuf::from(config.root.as_deref().unwrap_or("aplan"));
    match config_dir {
        Some(directory) if declared.is_relative() => directory.join(declared),
        _ => declared,
    }
}

fn canonical_plan(
    selected: &Path,
    configuration: &Path,
    plan: PathBuf,
) -> Result<PathBuf, RootResolutionError> {
    let Ok(canonical) = plan.canonicalize() else {
        return Err(RootResolutionError::MissingPlan {
            selected: selected.to_path_buf(),
            configuration: configuration.to_path_buf(),
            plan,
        });
    };
    if !canonical.is_dir() {
        return Err(RootResolutionError::MissingPlan {
            selected: selected.to_path_buf(),
            configuration: configuration.to_path_buf(),
            plan: canonical,
        });
    }
    Ok(canonical)
}

fn normalise_absolute(path: &Path) -> PathBuf {
    aplan_core::anchor::normalise(path)
}

/// The nearest Git worktree containing `path`, recognizing ordinary and linked
/// worktree markers without opening repository history or configuration.
#[must_use]
pub fn git_worktree_root(path: &Path) -> Option<PathBuf> {
    let absolute = path
        .canonicalize()
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf());
    absolute.ancestors().find_map(|candidate| {
        candidate
            .join(".git")
            .symlink_metadata()
            .ok()
            .filter(|metadata| {
                let kind = metadata.file_type();
                kind.is_dir() || kind.is_file()
            })
            .map(|_| candidate.to_path_buf())
    })
}

/// The Git worktree containing a plan, or the plan's parent outside Git.
#[must_use]
pub fn plan_repository(root: &Path) -> PathBuf {
    git_worktree_root(root).unwrap_or_else(|| {
        root.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    })
}

/// Fill the generated-agent-contract rule from one already loaded plan.
///
/// `tools` is supplied by the delivery adapter so this application crate does
/// not depend on the MCP crate whose policy it also serves.
#[must_use]
pub fn configured_agents_block(
    mut options: Options,
    plan_root: &Path,
    agent_root: &Path,
    config: &Config,
    plan: &aplan_core::Plan,
    written_by: Option<(&str, &str)>,
    tools: &[(String, String)],
) -> Options {
    if plan
        .load_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == aplan_core::Severity::Error)
    {
        return options;
    }
    let path = agent_root.join("AGENTS.md");
    if !path.is_file() {
        return options;
    }
    let contract = configured_agent_contract(
        relative_plan_dir(plan_root, agent_root),
        config,
        plan,
        tools,
    );
    let provenance =
        written_by.map(|(version, surface)| aplan_output::WrittenBy { version, surface });
    let rendered = aplan_output::render_agents_md(&contract, provenance.as_ref());
    let current = match aplan_core::contract::block_in(&rendered) {
        aplan_core::contract::Block::Present(inner) => inner.to_string(),
        _ => rendered,
    };
    let path = std::env::current_dir().map_or(path.clone(), |cwd| {
        path.strip_prefix(cwd).unwrap_or(&path).to_path_buf()
    });
    options.agents_block = Some(aplan_core::AgentsBlock { path, current });
    options
}

/// Assemble the generated agent contract from one repository's policy and
/// plan shape.
///
/// The delivery adapter owns how the plan path should be written relative to
/// its output file and which tools it exposes. Everything else is shared so a
/// CLI export and an MCP drift check cannot silently describe different
/// contracts.
#[must_use]
pub fn configured_agent_contract(
    plan_dir: Option<String>,
    config: &Config,
    plan: &aplan_core::Plan,
    tools: &[(String, String)],
) -> aplan_output::AgentContract {
    let evidence = config.evidence.as_ref();
    aplan_output::AgentContract {
        plan_dir,
        id_examples: aplan_interop::ids::example_ids(plan),
        scoped_evidence: config
            .policy
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|policy| {
                let selector = policy.when.selector().unwrap_or_default();
                let kinds = policy.evidence.as_ref()?.require_min_strength.as_ref()?;
                let requirement = if kinds.is_empty() {
                    "no evidence strength is required".to_string()
                } else {
                    format!(
                        "a `done` task must cite {} evidence",
                        kinds
                            .iter()
                            .map(|kind| format!("`{}`", kind.as_str()))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    )
                };
                Some((selector.describe().join(", "), requirement))
            })
            .collect(),
        min_strength: evidence
            .and_then(|policy| policy.require_min_strength.as_ref())
            .map(|kinds| kinds.iter().map(|kind| kind.as_str().to_string()).collect())
            .unwrap_or_default(),
        exempt_tags: evidence
            .and_then(|policy| policy.exempt_tags.as_ref())
            .cloned()
            .unwrap_or_default(),
        advisory: config.advisory.clone().unwrap_or_default(),
        switches: [
            (
                aplan_output::Switch::DenyWarnings,
                config.deny_warnings.unwrap_or(false),
            ),
            (
                aplan_output::Switch::DetectDrift,
                config.detect_drift.unwrap_or(false),
            ),
            (
                aplan_output::Switch::RequireCutMembership,
                config.require_cut_membership.unwrap_or(false),
            ),
            (
                aplan_output::Switch::RequireEffort,
                config.require_effort.unwrap_or(true),
            ),
            (aplan_output::Switch::UrlCheck, config.url.is_some()),
        ]
        .into_iter()
        .filter_map(|(switch, enabled)| enabled.then_some(switch))
        .collect(),
        runners: config
            .test
            .as_ref()
            .map(|runners| {
                runners
                    .all()
                    .iter()
                    .filter_map(|runner| runner.name.clone())
                    .collect()
            })
            .unwrap_or_default(),
        runner_count: config
            .test
            .as_ref()
            .map_or(0, |runners| runners.all().len()),
        tools: tools.to_vec(),
    }
}

fn relative_plan_dir(plan_root: &Path, base: &Path) -> Option<String> {
    let resolve = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let plan = resolve(plan_root);
    let relative = plan
        .strip_prefix(resolve(base))
        .map_or_else(|_| plan_root.to_path_buf(), Path::to_path_buf);
    let text = relative.display().to_string();
    (!text.is_empty() && text != ".").then_some(text)
}

/// Project a repository's warning policy without any one-run override.
#[must_use]
pub fn configured_verdict(config: &Config) -> aplan_core::Verdict {
    aplan_core::Verdict {
        deny_warnings: config.deny_warnings.unwrap_or(false),
        advisory: config.advisory.clone().unwrap_or_default(),
    }
}

/// Adapter-specific inputs layered over one loaded configuration.
#[derive(Debug, Default)]
pub struct ConfigurationInputs<'a> {
    /// Directory relative configuration values resolve from.
    pub directory: Option<&'a Path>,
    /// Exact configuration file selected, for located diagnostics and identity.
    pub file: Option<PathBuf>,
    /// One-run override disabling the effort rule.
    pub no_require_effort: bool,
    /// One-run trace-root override.
    pub traces_root: Option<PathBuf>,
    /// One-run override disabling trace resolution.
    pub no_traces: bool,
    /// Adapter default when neither configuration nor a flag selects a root.
    pub default_traces: Option<PathBuf>,
    /// Repository boundary against which trace escapes are judged.
    pub traces_repository: Option<PathBuf>,
}

/// Project one loaded configuration into the structural validation policy.
///
/// A long-lived adapter supplies the selected repository as `default_traces`,
/// rather than inheriting its process working directory.
#[must_use]
pub fn configured_options(config: &Config, inputs: ConfigurationInputs<'_>) -> Options {
    let ConfigurationInputs {
        directory: config_dir,
        file: configuration_file,
        no_require_effort,
        traces_root,
        no_traces,
        default_traces,
        traces_repository,
    } = inputs;
    let require_effort = !no_require_effort && config.require_effort.unwrap_or(true);
    let traces_root = if no_traces {
        None
    } else if let Some(path) = traces_root {
        Some(path)
    } else if config.no_traces == Some(true) {
        None
    } else if let Some(path) = &config.traces_root {
        let path = PathBuf::from(path);
        match config_dir {
            Some(directory) if path.is_relative() => Some(directory.join(path)),
            _ => Some(path),
        }
    } else {
        default_traces
    };
    let (require_min_strength, exempt_tags) = config.evidence.as_ref().map_or_else(
        || (None, Vec::new()),
        |evidence| {
            (
                evidence.require_min_strength.clone(),
                evidence.exempt_tags.clone().unwrap_or_default(),
            )
        },
    );
    let traces_external = config
        .traces_external
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|declared| {
            config_dir.map_or_else(|| PathBuf::from(&declared), |base| base.join(&declared))
        })
        .collect();
    let evidence_scopes = config
        .policy
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|policy| EvidenceScope {
            selector: policy.when.selector().unwrap_or_default(),
            named_dimensions: policy.when.named_dimensions(),
            require_min_strength: policy
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.require_min_strength.clone()),
            exempt_tags: policy
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.exempt_tags.clone()),
        })
        .collect();

    Options {
        require_effort,
        require_impact: config
            .version
            .as_ref()
            .filter(|version| version.requires_impact())
            .map(aplan_config::VersionPolicy::floors),
        traces_root,
        traces_repository,
        traces_external,
        require_min_strength,
        evidence_scopes,
        exempt_tags,
        agents_block: None,
        detect_drift: config.detect_drift.unwrap_or(false),
        require_cut_membership: config.require_cut_membership.unwrap_or(false),
        tag_families: configured_tag_families(config),
        owners: config
            .owners
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        config_file: configuration_file,
    }
}

/// Project configured evidence providers for a non-executing verification.
#[must_use]
pub fn configured_verification(
    config: &Config,
    configuration_file: Option<PathBuf>,
    traces_root: Option<PathBuf>,
) -> aplan_evidence::VerifyOptions {
    aplan_evidence::VerifyOptions {
        detect_drift: config.detect_drift.unwrap_or(false),
        traces_root,
        execute: false,
        test_runner: config.test.clone(),
        max_unrelated_files: config
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.max_unrelated_files),
        observation_inputs: config
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.observation_inputs.clone()),
        url_check: config.url.clone(),
        config_file: configuration_file,
        version: config.version.clone(),
    }
}

fn configured_tag_families(config: &Config) -> BTreeMap<String, TagFamily> {
    let Some(families) = &config.tag_families else {
        return BTreeMap::new();
    };
    families
        .iter()
        .map(|(name, family)| {
            let (status, milestones) = family.scope();
            (
                name.clone(),
                TagFamily {
                    members: family.members().to_vec(),
                    require: match family.require() {
                        Cardinality::Optional => aplan_core::Cardinality::Optional,
                        Cardinality::AtMostOne => aplan_core::Cardinality::AtMostOne,
                        Cardinality::AtLeastOne => aplan_core::Cardinality::AtLeastOne,
                        Cardinality::ExactlyOne => aplan_core::Cardinality::ExactlyOne,
                    },
                    status: match status {
                        StatusScope::Any => aplan_core::StatusScope::Any,
                        StatusScope::Live => aplan_core::StatusScope::Live,
                        StatusScope::Done => aplan_core::StatusScope::Done,
                        StatusScope::Dropped => aplan_core::StatusScope::Dropped,
                        StatusScope::Terminal => aplan_core::StatusScope::Terminal,
                    },
                    milestones: match milestones {
                        MilestoneScope::Include => aplan_core::MilestoneScope::Include,
                        MilestoneScope::Exclude => aplan_core::MilestoneScope::Exclude,
                        MilestoneScope::Only => aplan_core::MilestoneScope::Only,
                    },
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> tempfile::TempDir {
        let directory = tempfile::Builder::new()
            .prefix("aplan-explicit-root-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary repository");
        std::fs::create_dir(directory.path().join(".git")).expect("git marker");
        directory
    }

    #[test]
    fn configured_paths_share_the_configuration_frame() {
        let config = aplan_config::parse_config(
            "root: plan\ntraces_root: docs\ntraces_external: [../shared]\n",
            Path::new("/repo/.aplan.yml"),
        )
        .expect("config");
        let options = configured_options(
            &config,
            ConfigurationInputs {
                directory: Some(Path::new("/repo")),
                file: Some(PathBuf::from("/repo/.aplan.yml")),
                no_require_effort: false,
                traces_root: None,
                no_traces: false,
                default_traces: Some(PathBuf::from("/wrong-process-directory")),
                traces_repository: Some(PathBuf::from("/repo")),
            },
        );
        assert_eq!(options.traces_root, Some(PathBuf::from("/repo/docs")));
        assert_eq!(
            options.traces_external,
            vec![PathBuf::from("/repo/../shared")]
        );
        assert_eq!(options.traces_repository, Some(PathBuf::from("/repo")));
        assert_eq!(options.config_file, Some(PathBuf::from("/repo/.aplan.yml")));
    }

    #[test]
    fn an_explicit_trace_root_keeps_cli_precedence_over_config() {
        let config = aplan_config::parse_config(
            "no_traces: true\ntraces_root: ignored\n",
            Path::new("/repo/.aplan.yml"),
        )
        .expect("config");
        let explicit = configured_options(
            &config,
            ConfigurationInputs {
                directory: Some(Path::new("/repo")),
                file: Some(PathBuf::from("/repo/.aplan.yml")),
                no_require_effort: false,
                traces_root: Some(PathBuf::from("/chosen")),
                no_traces: false,
                default_traces: Some(PathBuf::from("/default")),
                traces_repository: Some(PathBuf::from("/repo")),
            },
        );
        assert_eq!(explicit.traces_root, Some(PathBuf::from("/chosen")));

        let disabled = configured_options(
            &config,
            ConfigurationInputs {
                directory: Some(Path::new("/repo")),
                file: Some(PathBuf::from("/repo/.aplan.yml")),
                no_require_effort: false,
                traces_root: Some(PathBuf::from("/chosen")),
                no_traces: true,
                default_traces: Some(PathBuf::from("/default")),
                traces_repository: Some(PathBuf::from("/repo")),
            },
        );
        assert_eq!(disabled.traces_root, None);
    }

    #[test]
    fn repository_and_task_forms_resolve_one_context() {
        let repository = repository();
        let plan = repository.path().join("plans/tasks");
        std::fs::create_dir_all(&plan).expect("plan");
        std::fs::write(
            repository.path().join(".aplan.yml"),
            "root: plans/tasks\ndeny_warnings: true\ntraces_root: docs\n",
        )
        .expect("config");
        std::fs::create_dir(repository.path().join("docs")).expect("docs");

        let from_repository =
            resolve_explicit_root(repository.path().to_str().expect("utf8")).expect("repository");
        let from_plan = resolve_explicit_root(plan.to_str().expect("utf8")).expect("plan");
        assert_eq!(from_repository.selection, ExplicitRootKind::Repository);
        assert_eq!(from_plan.selection, ExplicitRootKind::Plan);
        assert_eq!(from_repository.plan_root, from_plan.plan_root);
        assert_eq!(from_repository.repository_root, from_plan.repository_root);
        assert_eq!(
            from_repository.configuration_file,
            from_plan.configuration_file
        );
        assert!(from_plan.config.deny_warnings.unwrap_or(false));
    }

    #[test]
    fn a_nested_directory_cannot_partially_override_the_configured_plan() {
        let repository = repository();
        let plan = repository.path().join("aplan");
        let nested = plan.join("tasks");
        std::fs::create_dir_all(&nested).expect("nested plan");
        std::fs::write(repository.path().join(".aplan.yml"), "root: aplan\n").expect("config");

        let error = resolve_explicit_root(nested.to_str().expect("utf8"))
            .expect_err("nested directory must be refused");
        assert!(
            matches!(error, RootResolutionError::Ambiguous { .. }),
            "{error}"
        );
        assert!(
            error.to_string().contains("configured plan root"),
            "{error}"
        );
    }

    #[test]
    fn config_free_git_roots_have_only_two_meanings() {
        let repository = repository();
        let plan = repository.path().join("aplan");
        std::fs::create_dir(&plan).expect("plan");
        let other = repository.path().join("notes");
        std::fs::create_dir(&other).expect("other");

        assert_eq!(
            resolve_explicit_root(repository.path().to_str().expect("utf8"))
                .expect("repository")
                .selection,
            ExplicitRootKind::Repository
        );
        assert_eq!(
            resolve_explicit_root(plan.to_str().expect("utf8"))
                .expect("plan")
                .selection,
            ExplicitRootKind::Plan
        );
        assert!(matches!(
            resolve_explicit_root(other.to_str().expect("utf8")),
            Err(RootResolutionError::AmbiguousConfigFree { .. })
        ));
    }

    #[test]
    fn a_config_free_directory_outside_git_keeps_the_legacy_plan_meaning() {
        let holder = tempfile::Builder::new()
            .prefix("aplan-config-free-plan-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary holder");
        let plan = holder.path().join("tasks");
        std::fs::create_dir(&plan).expect("plan");

        let resolved =
            resolve_explicit_root(plan.to_str().expect("utf8")).expect("config-free plan");
        assert_eq!(resolved.selection, ExplicitRootKind::Plan);
        assert_eq!(
            resolved.plan_root,
            plan.canonicalize().expect("canonical plan")
        );
        assert_eq!(
            resolved.repository_root,
            holder.path().canonicalize().expect("canonical parent")
        );
        assert!(resolved.configuration_file.is_none());
        assert!(resolved.configuration_directory.is_none());
    }

    #[test]
    fn a_nested_git_worktree_does_not_inherit_an_outer_configuration() {
        let outer = repository();
        std::fs::write(outer.path().join(".aplan.yml"), "root: outer-plan\n").expect("config");
        std::fs::create_dir(outer.path().join("outer-plan")).expect("outer plan");
        let nested = outer.path().join("nested");
        std::fs::create_dir(&nested).expect("nested repository");
        std::fs::write(nested.join(".git"), "gitdir: elsewhere\n").expect("worktree marker");
        std::fs::create_dir(nested.join("aplan")).expect("nested plan");

        let resolved =
            resolve_explicit_root(nested.to_str().expect("utf8")).expect("nested repository");
        assert_eq!(resolved.selection, ExplicitRootKind::Repository);
        assert!(resolved.configuration_file.is_none());
        assert_eq!(
            resolved.repository_root,
            nested.canonicalize().expect("canonical")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_uses_its_targets_context() {
        use std::os::unix::fs::symlink;

        let holder = repository();
        let target = repository();
        std::fs::create_dir(target.path().join("work")).expect("plan");
        std::fs::write(target.path().join(".aplan.yml"), "root: work\n").expect("config");
        let alias = holder.path().join("foreign");
        symlink(target.path(), &alias).expect("alias");

        let resolved = resolve_explicit_root(alias.to_str().expect("utf8")).expect("alias");
        assert!(resolved.followed_symlink);
        assert_eq!(resolved.selection, ExplicitRootKind::Repository);
        assert_eq!(
            resolved.repository_root,
            target.path().canonicalize().expect("canonical target")
        );
        assert_eq!(resolved.plan_root, target.path().join("work"));
    }

    #[test]
    fn invalid_explicit_roots_fail_before_context_selection() {
        assert!(matches!(
            resolve_explicit_root(" \t"),
            Err(RootResolutionError::Blank)
        ));
        let directory = repository();
        let file = directory.path().join("file");
        std::fs::write(&file, "not a directory").expect("file");
        assert!(matches!(
            resolve_explicit_root(file.to_str().expect("utf8")),
            Err(RootResolutionError::NotDirectory(_))
        ));
        assert!(matches!(
            resolve_explicit_root(directory.path().join("missing").to_str().expect("utf8")),
            Err(RootResolutionError::Missing { .. })
        ));

        let configured = repository();
        std::fs::write(configured.path().join(".aplan.yml"), "root: absent\n").expect("config");
        assert!(matches!(
            resolve_explicit_root(configured.path().to_str().expect("utf8")),
            Err(RootResolutionError::MissingPlan { .. })
        ));

        let invalid = repository();
        std::fs::write(invalid.path().join(".aplan.yml"), "unknown_key: true\n")
            .expect("invalid config");
        assert!(matches!(
            resolve_explicit_root(invalid.path().to_str().expect("utf8")),
            Err(RootResolutionError::Configuration { .. })
        ));

        let config_free = repository();
        assert!(matches!(
            resolve_explicit_root(config_free.path().to_str().expect("utf8")),
            Err(RootResolutionError::MissingDefaultPlan { .. })
        ));
    }
}
