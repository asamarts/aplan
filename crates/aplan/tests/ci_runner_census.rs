//! **Every job says where it runs, and no pull request can reach the box**
//! (`A1-117`, audit).
//!
//! The rule it enforces is narrow on purpose: **a job pinned to the box is
//! only safe if a pull request cannot cause it to run.** A job pinned to a
//! hosted runner is outside that, because it can never reach the box.
//!
//! `A1-117` routed `ci.yml` and refused the two workflows a pull request must
//! not reach. Nothing then stopped the next job from hard-coding
//! `[self-hosted, linux, aplan]` again, and nothing had: the task's own account
//! of its surface said "seven jobs across three workflows", and the repository
//! has nine across four. `scale.yml` was never examined by that task. It is in
//! fact safe -- it has no pull-request trigger -- but nothing established that,
//! which is the difference between a conclusion and a coincidence.
//!
//! So this is the census shape the repository uses elsewhere: every job is
//! either routed, or pinned with the argument for why pinning it is safe, and a
//! new one is a failure rather than an omission nobody notices.
//!
//! **It parses the YAML rather than scanning lines.** The same audit that found
//! this gap found two gates fooled by reading string literals as structure, and
//! a security gate is the worst place to repeat that.

use std::collections::BTreeSet;

/// Jobs that keep a hard-coded self-hosted runner, and why each is safe.
///
/// Safe here means exactly one thing: **a pull request cannot cause this job to
/// run.** For the two that a pull request *can* trigger, the argument is the
/// job-level `if` this census checks for; for the one it cannot, the argument is
/// the trigger list.
const PINNED: &[(&str, &str, &str)] = &[
    (
        "release.yml",
        "release",
        "triggered only by a `v*` tag and `workflow_dispatch`, neither of which a pull request can \
         raise, and it carries the job-level refusal anyway so that adding a trigger later cannot \
         quietly make it reachable (`A1-117`)",
    ),
    (
        "action-selftest.yml",
        "selftest",
        "reachable by a pull request, by design: its paths filter names `action.yml`, which is \
         what a contributor editing the action would touch. It drives the action against this \
         repository's own release artefacts, so it refuses an untrusted event at the job level \
         rather than routing it somewhere else (`A1-117`)",
    ),
    (
        "scale.yml",
        "scale",
        "triggered only by `schedule` and `workflow_dispatch`. A pull request cannot raise either, \
         so there is nothing to route and nothing to refuse. Pinned rather than routed because the \
         nightly measurement is the point of the job and a hosted runner would measure a different \
         machine (`A1-229`). This entry exists because `A1-117` never examined this file",
    ),
];

/// The expression a routed job uses.
const ROUTED: &str = "needs.route.outputs.runner";

fn workflows() -> Vec<(String, serde_yaml_ng::Value)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .expect("the workflows directory")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "yml" || x == "yaml") {
            let text = std::fs::read_to_string(&path).expect("read a workflow");
            let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text)
                .unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()));
            out.push((
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                doc,
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The triggers a workflow declares. `on` is a plain string key under YAML 1.2,
/// which is what this parser implements, but a 1.1 reader would see the boolean
/// `true`; both are accepted so the census does not depend on that distinction.
fn triggers(doc: &serde_yaml_ng::Value) -> BTreeSet<String> {
    let on = doc
        .get("on")
        .or_else(|| doc.get(serde_yaml_ng::Value::Bool(true)));
    let mut out = BTreeSet::new();
    match on {
        Some(serde_yaml_ng::Value::String(s)) => {
            out.insert(s.clone());
        }
        Some(serde_yaml_ng::Value::Sequence(xs)) => {
            for x in xs {
                if let Some(s) = x.as_str() {
                    out.insert(s.to_string());
                }
            }
        }
        Some(serde_yaml_ng::Value::Mapping(m)) => {
            for (k, _) in m {
                if let Some(s) = k.as_str() {
                    out.insert(s.to_string());
                }
            }
        }
        _ => {}
    }
    out
}

fn runs_on(job: &serde_yaml_ng::Value) -> String {
    match job.get("runs-on") {
        Some(serde_yaml_ng::Value::String(s)) => s.clone(),
        Some(serde_yaml_ng::Value::Sequence(xs)) => xs
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(","),
        other => format!("{other:?}"),
    }
}

#[test]
fn every_job_is_routed_or_pinned_with_its_reason() {
    let mut unaccounted = Vec::new();
    let mut jobs_seen = 0usize;
    let files = workflows();

    for (file, doc) in &files {
        let Some(jobs) = doc.get("jobs").and_then(|j| j.as_mapping()) else {
            panic!("{file} declares no jobs");
        };
        for (name, job) in jobs {
            let name = name.as_str().unwrap_or_default().to_string();
            jobs_seen += 1;
            let on = runs_on(job);
            // The router itself, and anything taking its answer, is accounted for.
            if on.contains(ROUTED) || on.contains("github.event.repository.private") {
                continue;
            }
            // A job pinned to a *hosted* runner cannot reach the box, whatever
            // a pull request does to the workflow, so it needs no argument. This
            // census is about the box: `hosted-path` was its first catch, and
            // the answer was that the rule, not the table, was wrong.
            if !on.contains("self-hosted") {
                continue;
            }
            if PINNED
                .iter()
                .any(|(f, j, _)| *f == file.as_str() && *j == name.as_str())
            {
                continue;
            }
            unaccounted.push(format!("{file}:{name} runs on `{on}`"));
        }
    }

    assert!(
        unaccounted.is_empty(),
        "{} job(s) that neither take their runner from the router nor say why they keep a fixed \
         one (`A1-117`). A pull request supplies the workflow that runs, so a job pinned to the \
         box is only safe if a pull request cannot cause it to run -- add it to `PINNED` with that \
         argument, or route it:\n  - {}",
        unaccounted.len(),
        unaccounted.join("\n  - ")
    );

    // A census that found nothing would pass. Both floors are what the surface
    // actually is today, so shrinking it is a decision somebody has to make here.
    assert!(
        files.len() >= 4,
        "only {} workflow(s) read; `A1-117` undercounted this surface once already",
        files.len()
    );
    assert!(jobs_seen >= 9, "only {jobs_seen} job(s) read");
    for (file, job, why) in PINNED {
        assert!(
            why.len() > 80,
            "{file}:{job} is pinned with no argument worth reading: {why:?}"
        );
    }
}

#[test]
fn a_pinned_job_in_a_pull_request_workflow_refuses_the_event() {
    let mut exposed = Vec::new();
    for (file, doc) in workflows() {
        let pr = triggers(&doc)
            .iter()
            .any(|t| t == "pull_request" || t == "pull_request_target");
        if !pr {
            continue;
        }
        let Some(jobs) = doc.get("jobs").and_then(|j| j.as_mapping()) else {
            continue;
        };
        for (name, job) in jobs {
            let name = name.as_str().unwrap_or_default().to_string();
            let on = runs_on(job);
            if on.contains(ROUTED) || on.contains("github.event.repository.private") {
                continue;
            }
            if !on.contains("self-hosted") {
                continue;
            }
            // Pinned to the box, in a workflow a pull request can trigger. The
            // only thing that makes that safe is a condition on the *job*: a
            // step cannot refuse, because the job must be assigned a runner
            // first, and that assignment is the thing being prevented.
            let guard = job.get("if").and_then(|v| v.as_str()).unwrap_or_default();
            // Either form is accepted, because either is fail-closed: an
            // allow-list of the events the workflow is for, or an explicit
            // exclusion of both pull-request events. What is refused is a guard
            // that *admits* one, and a guard that does not mention the event at
            // all.
            let names_the_event = guard.contains("github.event_name");
            let admits_a_pr = guard.contains("== 'pull_request'")
                || guard.contains("== 'pull_request_target'")
                || (guard.contains("pull_request") && !guard.contains("!= 'pull_request'"));
            if !names_the_event || admits_a_pr {
                exposed.push(format!(
                    "{file}:{name} is pinned to `{on}` in a workflow a pull request can trigger, \
                     and its job-level `if` is {guard:?}"
                ));
            }
        }
    }
    assert!(
        exposed.is_empty(),
        "{} job(s) a pull request can send to the self-hosted runner (`A1-117`, ADR-0016). The \
         refusal has to be a job-level `if`, not a step: a step runs only after the job has been \
         assigned a runner, which is what must not happen:\n  - {}",
        exposed.len(),
        exposed.join("\n  - ")
    );
}
