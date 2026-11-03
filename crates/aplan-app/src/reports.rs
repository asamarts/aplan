//! Aggregate report projections shared by interactive delivery adapters.
//!
//! A workspace captures repository observations once. Changing a selector or a
//! stale-report date then remains a pure projection over identified inputs;
//! cursor movement never opens Git or re-reads configuration.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use aplan_core::Graph;
use aplan_core::deprecation::Deprecations;
use aplan_core::summary::{GroupedSummary, Summary};
use aplan_core::task::{EvidenceKind, Status, Task, VersionFloors};
use aplan_core::vacuity::RuleReach;
use aplan_evidence::Breadth;
use aplan_output::{PathBreadth, PathsReport, StaleReport, StaleTask};

use crate::{InputIdentity, PlanContext, Selector, TaskBrowserRow, Trust};

pub const DEFAULT_REPORT_PATH_SAMPLE: usize = 100;
pub const DEFAULT_REPORT_STALE_DAYS: u32 = 30;

/// The analyses available in the report hub, in navigation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Roadmap,
    Summary,
    Version,
    Paths,
    Vacuity,
    Stale,
    Deprecations,
}

impl ReportKind {
    pub const ALL: [Self; 7] = [
        Self::Roadmap,
        Self::Summary,
        Self::Version,
        Self::Paths,
        Self::Vacuity,
        Self::Stale,
        Self::Deprecations,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Roadmap => "Roadmap",
            Self::Summary => "Summary",
            Self::Version => "Version",
            Self::Paths => "Paths",
            Self::Vacuity => "Vacuity",
            Self::Stale => "Stale work",
            Self::Deprecations => "Deprecations",
        }
    }

    #[must_use]
    pub const fn selector_applies(self) -> bool {
        matches!(
            self,
            Self::Summary | Self::Paths | Self::Vacuity | Self::Stale
        )
    }

    #[must_use]
    pub const fn advisory(self) -> bool {
        matches!(
            self,
            Self::Paths | Self::Vacuity | Self::Stale | Self::Deprecations
        )
    }
}

/// User-controlled report parameters. Repository-reading parameters are fixed
/// when the workspace is captured; projection-only values can change freely.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportRequest {
    pub selector: Selector,
    pub stale_as_of: Option<String>,
    pub stale_window_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportState {
    Ready,
    Unavailable(String),
    Invalid(String),
}

/// Why a report row does or does not have task contributors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportRowProvenance {
    Tasks,
    Configuration,
    NoSubjects,
}

/// One report row. `task_ids` are the contributors to this exact aggregate,
/// not a selector to recompute the report under; `provenance` keeps an empty
/// subject set distinct from a configuration fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    pub key: String,
    pub label: String,
    pub summary: String,
    pub provenance: ReportRowProvenance,
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SummaryReport {
    Phases(Summary),
    Groups(GroupedSummary),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionCurrentSource {
    Configured(Vec<String>),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionNode {
    pub id: String,
    pub owes: aplan_core::task::VersionImpact,
    pub raised_by: Option<String>,
    pub recorded: Option<String>,
    pub contributors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionReport {
    pub current_source: VersionCurrentSource,
    pub floors: VersionFloors,
    pub nodes: Vec<VersionNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReportBody {
    Roadmap(aplan_core::roadmap::Roadmap),
    Summary(SummaryReport),
    Version(VersionReport),
    Paths(PathsReport),
    Vacuity(Vec<RuleReach>),
    Stale(StaleReport),
    Deprecations(Deprecations),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReportProjection {
    pub input: InputIdentity,
    pub kind: ReportKind,
    pub state: ReportState,
    pub scope: String,
    pub advisory: bool,
    pub rows: Vec<ReportRow>,
    pub body: Option<ReportBody>,
}

/// Immutable plan facts and repository observations for the report hub.
#[derive(Debug, Clone)]
pub struct ReportWorkspace {
    input: InputIdentity,
    tasks: Vec<Task>,
    paths: Vec<PathBuf>,
    catalogue: Vec<TaskBrowserRow>,
    coordination: aplan_core::ClaimScope,
    claim_observed_at_epoch_seconds: i64,
    config: aplan_config::Config,
    vacuity_off: BTreeMap<&'static str, &'static str>,
    breadth: Result<Breadth, String>,
    touched: Result<(BTreeMap<PathBuf, i64>, i64), String>,
}

impl PartialEq for ReportWorkspace {
    fn eq(&self, other: &Self) -> bool {
        self.input == other.input
            && self.tasks == other.tasks
            && self.paths == other.paths
            && self.catalogue == other.catalogue
            && self.coordination == other.coordination
            && self.claim_observed_at_epoch_seconds == other.claim_observed_at_epoch_seconds
            && format!("{:?}", self.config) == format!("{:?}", other.config)
            && self.vacuity_off == other.vacuity_off
            && self.breadth == other.breadth
            && self.touched == other.touched
    }
}

impl ReportWorkspace {
    /// Build a workspace whose repository-reading analyses are explicitly
    /// unavailable. Useful to adapters presenting an already-owned synthetic
    /// or imported snapshot without implying that Git was consulted.
    #[must_use]
    pub fn without_repository(
        input: InputIdentity,
        tasks: Vec<Task>,
        catalogue: Vec<TaskBrowserRow>,
        reason: impl Into<String>,
    ) -> Self {
        let reason = reason.into();
        let paths = tasks.iter().map(|_| PathBuf::new()).collect();
        let coordination = catalogue.first().map_or_else(
            || aplan_core::ClaimScope::unavailable(reason.clone()),
            |row| row.coordination.clone(),
        );
        let claim_observed_at_epoch_seconds = catalogue
            .first()
            .map_or(0, |row| row.claim_observed_at_epoch_seconds);
        Self {
            input,
            tasks,
            paths,
            catalogue,
            coordination,
            claim_observed_at_epoch_seconds,
            config: aplan_config::Config::default(),
            vacuity_off: BTreeMap::new(),
            breadth: Err(reason.clone()),
            touched: Err(reason),
        }
    }

    #[must_use]
    pub fn project(&self, kind: ReportKind, request: &ReportRequest) -> ReportProjection {
        let selected = self.selected_ids(kind, &request.selector);
        let scope = if kind.selector_applies() {
            let description = request.selector.describe();
            if description.is_empty() {
                "selected tasks: all".to_string()
            } else {
                format!("selected tasks: {}", description.join("; "))
            }
        } else {
            "whole plan (filters do not change this report)".to_string()
        };
        let answer = match kind {
            ReportKind::Roadmap => Ok(self.roadmap()),
            ReportKind::Summary => Ok(self.summary(&selected)),
            ReportKind::Version => Ok(self.version()),
            ReportKind::Paths => self.paths(&selected),
            ReportKind::Vacuity => Ok(self.vacuity(&selected)),
            ReportKind::Stale => self.stale(&selected, request),
            ReportKind::Deprecations => Ok(self.deprecations()),
        };
        match answer {
            Ok((body, rows)) => ReportProjection {
                input: self.input.clone(),
                kind,
                state: ReportState::Ready,
                scope,
                advisory: kind.advisory(),
                rows,
                body: Some(body),
            },
            Err(state) => ReportProjection {
                input: self.input.clone(),
                kind,
                state,
                scope,
                advisory: kind.advisory(),
                rows: Vec::new(),
                body: None,
            },
        }
    }

    fn selected_ids(&self, kind: ReportKind, selector: &Selector) -> BTreeSet<String> {
        if !kind.selector_applies() {
            return self.tasks.iter().map(|task| task.id.clone()).collect();
        }
        self.catalogue
            .iter()
            .filter(|row| {
                selector.matches_fields_claimed(
                    row.phase,
                    row.status,
                    row.owner.as_deref(),
                    &row.tags,
                    row.cut.as_deref(),
                    row.milestone.as_deref(),
                    row.coordination.available.then(|| row.claim.is_some()),
                )
            })
            .map(|row| row.id.clone())
            .collect()
    }

    fn roadmap(&self) -> (ReportBody, Vec<ReportRow>) {
        let graph = Graph::new(self.tasks.iter());
        let mut report = graph
            .roadmap()
            .with_throughput(self.config.effort_days_per_week);
        if let Some(family) = self
            .config
            .roadmap
            .as_ref()
            .and_then(|roadmap| roadmap.group_by.as_deref())
            .and_then(|name| self.config.tag_families.as_ref()?.get(name))
        {
            report = report.grouped_by(family.members());
        }
        let mut rows = report
            .cuts
            .iter()
            .map(|cut| {
                let (weeks, remaining) = report.weeks_of(cut);
                ReportRow {
                    key: cut.milestone_id.clone(),
                    label: format!("{} {}", cut.version, cut.title).trim().to_string(),
                    summary: format!(
                        "{}/{} done; {} ready; {:.1}d{}; critical {:.1}d",
                        cut.done,
                        cut.total,
                        cut.ready,
                        cut.effort_days,
                        weeks.map_or_else(String::new, |all| format!(
                            " / {all:.1}w ({:.1}w left)",
                            remaining.unwrap_or_default()
                        )),
                        cut.critical_path.total_effort_days
                    ),
                    provenance: ReportRowProvenance::Tasks,
                    task_ids: std::iter::once(cut.milestone_id.clone())
                        .chain(cut.members.iter().map(|task| task.id.clone()))
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        if !report.cross_cutting.is_empty() {
            let (weeks, remaining) = report.cross_cutting_weeks();
            rows.push(ReportRow {
                key: "cross-cutting".to_string(),
                label: "Cross-cutting work".to_string(),
                summary: format!(
                    "{} task(s); {:.1}d{}",
                    report.cross_cutting.len(),
                    report.cross_cutting_effort_days,
                    weeks.map_or_else(String::new, |all| format!(
                        " / {all:.1}w ({:.1}w left)",
                        remaining.unwrap_or_default()
                    ))
                ),
                provenance: ReportRowProvenance::Tasks,
                task_ids: report
                    .cross_cutting
                    .iter()
                    .map(|task| task.id.clone())
                    .collect(),
            });
        }
        (ReportBody::Roadmap(report), rows)
    }

    fn summary(&self, selected: &BTreeSet<String>) -> (ReportBody, Vec<ReportRow>) {
        let graph = Graph::new(self.tasks.iter());
        let keep = |id: &str| selected.contains(id);
        let group = self
            .config
            .summary
            .as_ref()
            .and_then(|summary| summary.group_by.as_deref())
            .and_then(|name| Some((name, self.config.tag_families.as_ref()?.get(name)?)));
        if let Some((name, family)) = group {
            let report = graph
                .summary_grouped_by_of(name, family.members(), &keep)
                .with_throughput(self.config.effort_days_per_week);
            let rows = report
                .groups
                .iter()
                .map(|group| {
                    let ids = self
                        .tasks
                        .iter()
                        .filter(|task| selected.contains(&task.id) && !task.is_milestone())
                        .filter(|task| match &group.label {
                            Some(label) => task.tags.contains(label),
                            None => !family
                                .members()
                                .iter()
                                .any(|member| task.tags.contains(member)),
                        })
                        .map(|task| task.id.clone())
                        .collect();
                    ReportRow {
                        key: group
                            .label
                            .clone()
                            .unwrap_or_else(|| "ungrouped".to_string()),
                        label: group
                            .label
                            .clone()
                            .unwrap_or_else(|| "Ungrouped".to_string()),
                        summary: format!(
                            "{}/{} done; {} ready; {:.1}d; {} dropped",
                            group.done, group.tasks, group.ready, group.effort_days, group.dropped
                        ),
                        provenance: ReportRowProvenance::Tasks,
                        task_ids: ids,
                    }
                })
                .collect();
            return (ReportBody::Summary(SummaryReport::Groups(report)), rows);
        }
        let report = graph
            .summary_of(&keep)
            .with_throughput(self.config.effort_days_per_week);
        let rows = report
            .phases
            .iter()
            .map(|phase| ReportRow {
                key: format!("phase-{}", phase.phase),
                label: format!("Phase {}", phase.phase),
                summary: format!(
                    "{}/{} done; {:.1}d; {} dropped; {} ready overall",
                    phase.done, phase.tasks, phase.effort_days, phase.dropped, report.ready
                ),
                provenance: ReportRowProvenance::Tasks,
                task_ids: self
                    .tasks
                    .iter()
                    .filter(|task| {
                        selected.contains(&task.id)
                            && !task.is_milestone()
                            && task.phase == phase.phase
                    })
                    .map(|task| task.id.clone())
                    .collect(),
            })
            .collect();
        (ReportBody::Summary(SummaryReport::Phases(report)), rows)
    }

    fn version(&self) -> (ReportBody, Vec<ReportRow>) {
        let graph = Graph::new(self.tasks.iter());
        let policy = self.config.version.clone().unwrap_or_default();
        let floors = policy.floors();
        let current_source = policy
            .current
            .as_ref()
            .map_or(VersionCurrentSource::Missing, |command| {
                VersionCurrentSource::Configured(command.command.clone())
            });
        let mut candidates = self
            .tasks
            .iter()
            .filter(|task| task.is_milestone() || task.version_impact.is_some())
            .collect::<Vec<_>>();
        candidates.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        let nodes = candidates
            .into_iter()
            .filter_map(|task| {
                let owed = aplan_core::version::owed(&graph, floors, &task.id)?;
                Some(VersionNode {
                    id: task.id.clone(),
                    owes: owed.impact,
                    raised_by: owed.raised_by,
                    recorded: task.version.clone(),
                    contributors: aplan_core::version::reached(&graph, floors, &task.id)
                        .into_iter()
                        .collect(),
                })
            })
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        if current_source == VersionCurrentSource::Missing {
            rows.push(ReportRow {
                key: "current-source".to_string(),
                label: "Current version source missing".to_string(),
                summary: "version.current.command is not configured; owed impacts remain available"
                    .to_string(),
                provenance: ReportRowProvenance::Configuration,
                task_ids: Vec::new(),
            });
        }
        rows.extend(nodes.iter().map(|node| ReportRow {
            key: node.id.clone(),
            label: node.id.clone(),
            summary: format!(
                "owes {}; raised by {}; recorded {}",
                node.owes,
                node.raised_by.as_deref().unwrap_or("its level"),
                node.recorded.as_deref().unwrap_or("none")
            ),
            provenance: ReportRowProvenance::Tasks,
            task_ids: node.contributors.clone(),
        }));
        (
            ReportBody::Version(VersionReport {
                current_source,
                floors,
                nodes,
            }),
            rows,
        )
    }

    fn paths(&self, selected: &BTreeSet<String>) -> ReportAnswer {
        let breadth = self.breadth.clone().map_err(ReportState::Unavailable)?;
        let by_id: BTreeMap<&str, &Task> = self
            .tasks
            .iter()
            .filter(|task| selected.contains(&task.id) && !task.is_milestone())
            .map(|task| (task.id.as_str(), task))
            .collect();
        let rows = path_measurements(&by_id, &breadth);
        let without_ids = by_id
            .values()
            .filter(|task| task.paths.is_empty())
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        let unmeasurable_ids = breadth
            .unmeasurable
            .iter()
            .filter(|id| selected.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let report = PathsReport {
            sampled: breadth.sampled,
            asked_for: DEFAULT_REPORT_PATH_SAMPLE,
            tasks: rows,
            without_paths: without_ids.len(),
            unmeasurable: unmeasurable_ids.len(),
        };
        let mut drill = report
            .tasks
            .iter()
            .map(|task| ReportRow {
                key: task.id.clone(),
                label: format!("{} {}", task.id, task.title),
                summary: if report.sampled == 0 {
                    "no commits sampled; no breadth measurement".to_string()
                } else if task.hits == 0 {
                    format!("measured zero of {} sampled commits", report.sampled)
                } else {
                    format!(
                        "{} of {} sampled commits ({}%)",
                        task.hits, report.sampled, task.percent
                    )
                },
                provenance: ReportRowProvenance::Tasks,
                task_ids: vec![task.id.clone()],
            })
            .collect::<Vec<_>>();
        if report.sampled == 0 {
            let unmeasurable = breadth
                .unmeasurable
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let no_sample_ids = by_id
                .values()
                .filter(|task| !task.paths.is_empty() && !unmeasurable.contains(task.id.as_str()))
                .map(|task| task.id.clone())
                .collect::<Vec<_>>();
            if !no_sample_ids.is_empty() {
                drill.push(ReportRow {
                    key: "no-sample".to_string(),
                    label: "No commit history sampled".to_string(),
                    summary: format!(
                        "{} task(s) have measurable paths, but zero commits were available",
                        no_sample_ids.len()
                    ),
                    provenance: ReportRowProvenance::Tasks,
                    task_ids: no_sample_ids,
                });
            }
        }
        if !without_ids.is_empty() {
            drill.push(ReportRow {
                key: "without-paths".to_string(),
                label: "No paths declared".to_string(),
                summary: format!(
                    "{} unmatched task(s); no measurement attempted",
                    without_ids.len()
                ),
                provenance: ReportRowProvenance::Tasks,
                task_ids: without_ids,
            });
        }
        if !unmeasurable_ids.is_empty() {
            drill.push(ReportRow {
                key: "unmeasurable".to_string(),
                label: "Invalid path patterns".to_string(),
                summary: format!("{} task(s) were not measured", unmeasurable_ids.len()),
                provenance: ReportRowProvenance::Tasks,
                task_ids: unmeasurable_ids,
            });
        }
        Ok((ReportBody::Paths(report), drill))
    }

    fn vacuity(&self, selected: &BTreeSet<String>) -> (ReportBody, Vec<ReportRow>) {
        let tasks = self
            .tasks
            .iter()
            .filter(|task| selected.contains(&task.id))
            .collect::<Vec<_>>();
        let mut off = self.vacuity_off.clone();
        off.extend(resolver_side_off(&self.config, &tasks));
        let reaches = aplan_core::vacuity::of_with(&tasks, &off);
        let rows = reaches
            .iter()
            .map(|reach| {
                let applies = reach.rule.applies_to();
                let ids: Vec<String> = tasks
                    .iter()
                    .filter(|task| applies.iter().all(|binding| binding.holds(task)))
                    .map(|task| task.id.clone())
                    .collect();
                ReportRow {
                    key: reach.rule.code().to_string(),
                    label: format!("{} {}", reach.rule.code(), reach.rule.title()),
                    summary: reach.off.map_or_else(
                        || {
                            format!(
                                "{} subject(s); {} checked; {} unbound",
                                reach.subjects,
                                reach.checked(),
                                reach.unbound
                            )
                        },
                        |key| {
                            format!(
                                "switched off by {key}; {} subject(s), 0 checked",
                                reach.subjects
                            )
                        },
                    ),
                    provenance: if ids.is_empty() {
                        ReportRowProvenance::NoSubjects
                    } else {
                        ReportRowProvenance::Tasks
                    },
                    task_ids: ids,
                }
            })
            .collect();
        (ReportBody::Vacuity(reaches), rows)
    }

    fn stale(&self, selected: &BTreeSet<String>, request: &ReportRequest) -> ReportAnswer {
        let window = i64::from(
            request
                .stale_window_days
                .or(self.config.stale_after_days)
                .unwrap_or(DEFAULT_REPORT_STALE_DAYS),
        );
        let parsed_as_of = match request.stale_as_of.as_deref() {
            None => None,
            Some(date) => {
                let Some(seconds) =
                    aplan_core::validate::iso_date_to_epoch(date).map(|seconds| seconds + 86_399)
                else {
                    return Err(ReportState::Invalid(format!(
                        "`{date}` is not an ISO-8601 date (YYYY-MM-DD)"
                    )));
                };
                Some((date, seconds))
            }
        };
        // Validate caller-controlled syntax before consulting the repository;
        // an unavailable provider must not disguise a malformed parameter.
        let (touched, head_time) = self.touched.clone().map_err(ReportState::Unavailable)?;
        let now = match parsed_as_of {
            None => head_time,
            Some((date, seconds)) => {
                if seconds < head_time {
                    return Err(ReportState::Invalid(format!(
                        "`{date}` precedes HEAD; choose a date at or after the last commit"
                    )));
                }
                seconds
            }
        };
        let (stale, fresh, untracked) = self.stale_tasks(selected, &touched, now, window);
        let report = StaleReport {
            window_days: window,
            tasks: stale,
            fresh,
            untracked,
            as_of: request.stale_as_of.clone(),
            coordination: self.coordination.clone(),
            claim_observed_at_epoch_seconds: self.claim_observed_at_epoch_seconds,
        };
        let rows = report
            .tasks
            .iter()
            .map(|task| ReportRow {
                key: task.id.clone(),
                label: format!("{} {}", task.id, task.title),
                summary: format!(
                    "{} days idle; {}; {}",
                    task.idle_days,
                    task.status,
                    if task.ready { "ready" } else { "not ready" }
                ),
                provenance: ReportRowProvenance::Tasks,
                task_ids: vec![task.id.clone()],
            })
            .collect();
        Ok((ReportBody::Stale(report), rows))
    }

    fn stale_tasks(
        &self,
        selected: &BTreeSet<String>,
        touched: &BTreeMap<PathBuf, i64>,
        now: i64,
        window: i64,
    ) -> (Vec<StaleTask>, usize, usize) {
        let graph = Graph::new(self.tasks.iter());
        let ready = graph
            .ready()
            .iter()
            .map(|task| task.id.as_str())
            .collect::<BTreeSet<_>>();
        let catalogue_by_id = self
            .catalogue
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        let mut stale = Vec::new();
        let (mut fresh, mut untracked) = (0usize, 0usize);
        for (task, path) in self.tasks.iter().zip(&self.paths).filter(|(task, _)| {
            selected.contains(&task.id)
                && !task.is_milestone()
                && (task.status.is_workable() || task.status == Status::Blocked)
        }) {
            let Some(seconds) = touched.get(path) else {
                untracked += 1;
                continue;
            };
            let idle_days = (now - seconds).max(0) / 86_400;
            if idle_days <= window {
                fresh += 1;
                continue;
            }
            let row = catalogue_by_id.get(task.id.as_str()).copied();
            let claim = row.and_then(|row| row.claim.clone());
            let observed_at = row.map_or(0, |row| row.claim_observed_at_epoch_seconds);
            stale.push(StaleTask {
                id: task.id.clone(),
                title: task.title.clone(),
                status: task.status.to_string(),
                idle_days,
                ready: ready.contains(task.id.as_str()),
                owner: task.owner.clone(),
                held_by: claim.as_ref().map(|record| record.holder.clone()),
                claim_age_seconds: claim.as_ref().map(|record| {
                    observed_at
                        .saturating_sub(record.claimed_at_epoch_seconds)
                        .max(0)
                }),
                claim,
            });
        }
        stale.sort_by(|a, b| {
            b.ready
                .cmp(&a.ready)
                .then(b.idle_days.cmp(&a.idle_days))
                .then(a.id.cmp(&b.id))
        });
        (stale, fresh, untracked)
    }

    fn deprecations(&self) -> (ReportBody, Vec<ReportRow>) {
        let tasks = self.tasks.iter().collect::<Vec<_>>();
        let graph = Graph::new(tasks.iter().copied());
        let report = aplan_core::deprecation::of(&tasks, &graph.roadmap());
        let mut rows = report
            .runways
            .iter()
            .map(|runway| ReportRow {
                key: runway.announced.id.clone(),
                label: format!("{} {}", runway.announced.id, runway.announced.title),
                summary: if runway.faults.is_empty() {
                    format!(
                        "{} removal task(s); runway is complete",
                        runway.removals.len()
                    )
                } else {
                    runway
                        .faults
                        .iter()
                        .map(|fault| format!("{}: {}", fault.kind(), fault.what()))
                        .collect::<Vec<_>>()
                        .join("; ")
                },
                provenance: ReportRowProvenance::Tasks,
                task_ids: std::iter::once(runway.announced.id.clone())
                    .chain(runway.removals.iter().map(|task| task.id.clone()))
                    .collect(),
            })
            .collect::<Vec<_>>();
        rows.extend(report.unannounced.iter().map(|task| ReportRow {
            key: task.id.clone(),
            label: format!("{} {}", task.id, task.title),
            summary: "removal has no announcement task".to_string(),
            provenance: ReportRowProvenance::Tasks,
            task_ids: vec![task.id.clone()],
        }));
        (ReportBody::Deprecations(report), rows)
    }
}

type ReportAnswer = Result<(ReportBody, Vec<ReportRow>), ReportState>;

fn path_measurements(tasks: &BTreeMap<&str, &Task>, breadth: &Breadth) -> Vec<PathBreadth> {
    let mut rows = breadth
        .counts
        .iter()
        .filter_map(|(id, hits)| {
            let task = tasks.get(id.as_str())?;
            Some(PathBreadth {
                id: id.clone(),
                title: task.title.clone(),
                hits: *hits,
                percent: u32::try_from(hits * 100 / breadth.sampled.max(1)).unwrap_or(u32::MAX),
                first_path: task.paths.first().cloned().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.hits.cmp(&a.hits).then(a.id.cmp(&b.id)));
    rows
}

/// Capture all aggregate inputs. Repository failures remain typed per report so
/// hermetic reports continue to work and the unavailable analysis says why.
#[must_use]
pub fn report_workspace(context: &PlanContext, catalogue: Vec<TaskBrowserRow>) -> ReportWorkspace {
    let tasks = context
        .plan
        .tasks
        .iter()
        .map(|loaded| loaded.task.clone())
        .collect::<Vec<_>>();
    let repository_root = &context.identity.repository_root;
    let canonical_repository =
        std::fs::canonicalize(repository_root).unwrap_or_else(|_| repository_root.clone());
    let paths = context
        .plan
        .tasks
        .iter()
        .map(|loaded| {
            std::fs::canonicalize(&loaded.path)
                .ok()
                .and_then(|path| {
                    path.strip_prefix(&canonical_repository)
                        .ok()
                        .map(PathBuf::from)
                })
                .unwrap_or_else(|| loaded.path.clone())
        })
        .collect::<Vec<_>>();
    let repository_allowed = matches!(
        context.identity.trust,
        Trust::RepositoryRead | Trust::ConfiguredProcesses
    );
    let with_paths = tasks
        .iter()
        .filter(|task| !task.is_milestone() && !task.paths.is_empty())
        .map(|task| (task.id.clone(), task.paths.clone()))
        .collect::<Vec<_>>();
    let live_paths = tasks
        .iter()
        .zip(&paths)
        .filter(|(task, _)| {
            !task.is_milestone() && (task.status.is_workable() || task.status == Status::Blocked)
        })
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    let denied = "repository reads are unavailable in this hermetic snapshot".to_string();
    let breadth = if repository_allowed {
        aplan_evidence::commit_breadth(repository_root, &with_paths, DEFAULT_REPORT_PATH_SAMPLE)
            .map_err(|error| error.to_string())
    } else {
        Err(denied.clone())
    };
    let touched = if repository_allowed {
        aplan_evidence::last_touched(repository_root, &live_paths)
            .map_err(|error| error.to_string())
    } else {
        Err(denied)
    };
    // Every catalogue row carries the same coherent claim snapshot. An empty
    // plan has no row on which to carry it, so observe coordination directly
    // rather than misreporting a healthy scope as unavailable.
    let (coordination, claim_observed_at_epoch_seconds) = catalogue.first().map_or_else(
        || {
            let snapshot =
                crate::claims::snapshot(repository_root, context.plan()).unwrap_or_else(|error| {
                    aplan_core::ClaimSnapshot::unavailable(crate::epoch_now(), error.to_string())
                });
            (snapshot.scope, snapshot.observed_at_epoch_seconds)
        },
        |row| {
            (
                row.coordination.clone(),
                row.claim_observed_at_epoch_seconds,
            )
        },
    );
    ReportWorkspace {
        input: context.identity.clone(),
        tasks,
        paths,
        catalogue,
        coordination,
        claim_observed_at_epoch_seconds,
        config: context.report_config.clone().unwrap_or_default(),
        vacuity_off: aplan_core::vacuity::switched_off(context.plan(), context.options()),
        breadth,
        touched,
    }
}

fn resolver_side_off(
    config: &aplan_config::Config,
    tasks: &[&Task],
) -> BTreeMap<&'static str, &'static str> {
    let mut out = BTreeMap::new();
    let cited = tasks
        .iter()
        .flat_map(|task| &task.evidence)
        .filter(|evidence| evidence.kind == EvidenceKind::Test)
        .map(|evidence| evidence.reference.clone())
        .collect::<Vec<_>>();
    match &config.test {
        None => {
            out.insert(aplan_core::code::TEST_UNRESOLVED, "test");
            out.insert(aplan_core::code::TEST_FAILED, "test");
            out.insert(aplan_core::code::TEST_IRRELEVANT, "test");
            out.insert(aplan_core::code::RUNNER_LISTS_NOTHING, "test");
            out.insert(aplan_core::code::RUN_PROVES_NOTHING, "test");
        }
        Some(runners) if !aplan_evidence::claims_locality(runners, &cited) => {
            out.insert(aplan_core::code::TEST_IRRELEVANT, "test.covers");
        }
        Some(_) => {}
    }
    if config.url.is_none() {
        out.insert(aplan_core::code::URL_UNREACHABLE, "url.check");
    }
    out
}
