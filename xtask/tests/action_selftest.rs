//! The action self-test tests the action, not the last release (`A1-278`).
//!
//! `Action self-test` runs `action.yml` from the checkout against a **pinned
//! release**, and it used to point that release at this repository's own plan.
//! `.aplan.yml` tracks `main` and has grown eight keys since the newest tag, so
//! the job had quietly become a compatibility test between today's configuration
//! and the last one released. It only triggers on a change to `action.yml`, so
//! nobody saw it for a month, and then it failed on `unknown field
//! effort_days_per_week` with nothing about the action wrong.
//!
//! The fix is a fixture plan written to what the pinned release can parse. What
//! makes it stay fixed is here: the two ways it comes undone are re-pointing the
//! job at `aplan/`, and adding a modern key to the fixture's config. Both are
//! one-line edits that look harmless and turn the job red weeks later.
//!
//! Read from the repository's own files rather than by running anything: a
//! GitHub workflow cannot be executed here, and asserting on the YAML is what is
//! actually available. It is narrow on purpose.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits under the workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Every top-level `.aplan.yml` key the pinned release knows.
///
/// Read out of the released binary rather than out of this checkout, which is
/// the whole point: `aplan facts` on the artifact published for the pinned tag
/// reports what *it* accepts, and the source tree reports what the next release
/// will. Taking it from the wrong one is how the fixture drifts ahead of the
/// release and the workflow goes red a month later with nothing wrong in
/// `action.yml` (`A1-278`).
///
/// ```console
/// $ ./aplan --version
/// aplan 0.3.0
/// $ ./aplan facts | jq -r '.config_keys[] | split(".")[0]' | sort -u
/// ```
///
/// Twenty top-level keys of the sixty-one `v0.3.0` accepts; the rest are nested
/// under one of these and arrive with it.
const KEYS_V0_3_0: &[&str] = &[
    "advisory",
    "deny_warnings",
    "detect_drift",
    "docs",
    "effort_days_per_week",
    "evidence",
    "id_strategy",
    "no_traces",
    "policy",
    "require_cut_membership",
    "require_effort",
    "roadmap",
    "root",
    "stale_after_days",
    "tag_families",
    "test",
    "traces_external",
    "traces_root",
    "url",
    "version",
];

/// The workflow checks the fixture, and not this repository's plan.
#[test]
fn the_self_test_runs_against_the_fixture_plan() {
    let wf = read(".github/workflows/action-selftest.yml");
    assert!(
        wf.contains("working-directory: ci/fixtures/action-selftest"),
        "the self-test no longer points at the fixture, so a pinned release is \
         reading a configuration that tracks `main` again"
    );
    // Every invocation of the action, not just the first: the second run is the
    // `verify` case, and leaving that one on the repository's plan would fail
    // for the same reason with half the symptom. Counted rather than fixed at a
    // number, so adding a step does not need this test edited -- only the
    // property kept.
    let runs = wf.matches("uses: ./").count();
    let in_fixture = wf
        .matches("working-directory: ci/fixtures/action-selftest")
        .count();
    assert!(
        runs >= 2,
        "the self-test stopped running the action twice:\n{wf}"
    );
    assert!(
        in_fixture >= runs,
        "{runs} step(s) run the action and only {in_fixture} name the fixture, so one reads a \
         plan outside it:\n{wf}"
    );
    assert!(
        !wf.contains("root: aplan\n"),
        "an action step still names this repository's plan root:\n{wf}"
    );
}

/// And the fixture stays parseable by the version the workflow pins.
///
/// The failure this prevents is the original one, reintroduced: a key added here
/// out of habit is invisible until the workflow next runs, which may be a month.
#[test]
fn the_fixture_config_uses_only_keys_the_pinned_release_knows() {
    let cfg = read("ci/fixtures/action-selftest/.aplan.yml");
    let unknown: Vec<&str> = cfg
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        // Top-level keys only: a nested one belongs to a key already checked.
        .filter(|l| !l.starts_with([' ', '\t', '-']))
        .filter_map(|l| l.split_once(':').map(|(k, _)| k))
        .filter(|k| !KEYS_V0_3_0.contains(k))
        .collect();
    assert!(
        unknown.is_empty(),
        "the fixture's config uses {unknown:?}, which the release the workflow pins cannot \
         parse. Widen `KEYS_V0_3_0` only when the pinned version is raised too"
    );
}

/// The fixture's plan is one the pinned release can load.
///
/// From the same source as the keys above, and for the same reason: `aplan
/// schema` on the published artifact reports the task shape *it* validates
/// against. `v0.3.0` added `prefer_after`, `revisit_at`, `version_impact` and
/// `version_impact_reason` to what `v0.2.0` knew.
#[test]
fn the_fixture_tasks_carry_only_fields_the_pinned_release_knows() {
    const FIELDS: &[&str] = &[
        "blocked_by",
        "effort_days",
        "evidence",
        "id",
        "owner",
        "paths",
        "phase",
        "prefer_after",
        "revisit_at",
        "status",
        "tags",
        "title",
        "trace_fingerprints",
        "traces",
        "version_impact",
        "version_impact_reason",
    ];
    let dir = repo_root().join("ci/fixtures/action-selftest/tasks");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("fixture tasks") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&path).expect("read task");
        let front = text
            .split("---")
            .nth(1)
            .unwrap_or_else(|| panic!("{} has no frontmatter", path.display()));
        for line in front.lines() {
            if line.starts_with([' ', '\t', '-']) || line.trim().is_empty() {
                continue;
            }
            let Some((key, _)) = line.split_once(':') else {
                continue;
            };
            assert!(
                FIELDS.contains(&key),
                "{} declares `{key}`, which the release the workflow pins does not know",
                path.display()
            );
        }
    }
    assert!(
        seen >= 2,
        "the fixture lost its tasks: {seen} found in {dir:?}"
    );
}
