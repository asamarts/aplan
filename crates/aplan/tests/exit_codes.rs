//! Every exit code the tool publishes is produced by a real run (`A1-211`).
//!
//! ADR-0007 gives `aplan` four exit codes and `aplan facts` publishes them, so
//! a CI script can branch on `error` versus `usage` without hard-coding
//! numbers. Three of the four were exercised all over the suite, by runs of the
//! binary. `3` was asserted once, by `errors_map_to_usage_unless_tagged_a_bug`,
//! which calls `exit_code_for` on a `Bug`-tagged error and checks the mapping:
//! true, and a statement about a function rather than about the tool. **No run
//! of the binary had ever produced `3`**, and it is the code a consumer most
//! needs to be right, because it is the one that says *do not trust this run*
//! -- a script that treats it as "findings" reports a clean plan as dirty, and
//! one that treats it as usage retries forever.
//!
//! **`3` is only reachable through a defect**, by construction: `exit_code_for`
//! assigns it only to an error carrying the internal `Bug` marker. For years
//! this gate therefore rode one known writer failure after another: fixing an
//! accepted YAML source form meant finding the next defect which still reached
//! the marker. That is an incentive in exactly the wrong direction. `A1-726`
//! replaces the poisoned-plan fixture with one exact private environment fault
//! gated again by the integration harness's existing invocation ledger. It
//! enters the real binary before dispatch and crosses the ordinary renderer and
//! classifier, while an absent ledger or absent, empty and near-miss values do
//! nothing. No CLI argument exposes it, and no accepted plan input has to remain
//! broken merely so this gate can exercise the code a defect would take.

use std::path::Path;

use crate::common;

/// One invocation that must produce one code.
struct Probe {
    /// The key in `facts`' `exit_codes` map.
    code: &'static str,
    /// What the run is, for a failure message that says which probe rotted.
    what: &'static str,
    /// Something the run must say.
    ///
    /// Without this a probe passes on the right number for the wrong reason,
    /// and three of the four are easy to reach by accident: a fixture that fails
    /// to load exits `1` like a plan with a finding, and a typo in the arguments
    /// exits `2` like a root that is not there. The code alone would then be
    /// evidence that the fixture was broken (`A1-211`, audit).
    says: &'static str,
    /// Set up a plan under `dir` and run.
    invoke: fn(&Path) -> common::Run,
}

fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("exit-codes-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir")
}

/// A plan of one task, with whatever `frontmatter` the caller needs beyond the
/// required fields.
fn plan(dir: &Path, frontmatter: &str) {
    std::fs::create_dir_all(dir.join("aplan")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
    std::fs::write(
        dir.join("aplan/T-1.md"),
        format!(
            "---\nid: T-1\ntitle: a task\nphase: 1\nstatus: todo\neffort_days: 1\n\
             {frontmatter}---\n\n## Why\n\nBecause.\n"
        ),
    )
    .expect("task");
}

const INTERNAL_FAULT_ENV: &str = "APLAN_TEST_ONLY_FAULT";
const INTERNAL_FAULT_VALUE: &str = "internal-exit-census-v1";
const HARNESS_RECORD_ENV: &str = "APLAN_TEST_INVOCATIONS";

fn harness_record(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("internal-exit-census.invocations");
    std::fs::write(&path, []).expect("create the private harness marker");
    path
}

fn internal_fault(dir: &Path) -> common::Run {
    let out = common::command(dir, &["facts"])
        .env(INTERNAL_FAULT_ENV, INTERNAL_FAULT_VALUE)
        .env(HARNESS_RECORD_ENV, harness_record(dir))
        .output()
        .expect("run the binary with its private internal fault");
    common::Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

const PROBES: &[Probe] = &[
    Probe {
        code: "clean",
        says: "ok:",
        what: "`check` on a plan with nothing to report",
        invoke: |dir| {
            plan(dir, "");
            common::run(dir, &["check"])
        },
    },
    Probe {
        code: "error",
        says: "APLAN003",
        what: "`check` on a plan whose task is blocked by an id that does not exist",
        invoke: |dir| {
            plan(dir, "blocked_by: [T-404]\n");
            common::run(dir, &["check"])
        },
    },
    Probe {
        code: "usage",
        says: "is not a directory",
        what: "`check --root` naming a directory that is not there",
        invoke: |dir| {
            plan(dir, "");
            common::run(dir, &["check", "--root", "no-such-dir"])
        },
    },
    Probe {
        code: "internal",
        says: "deliberate internal-exit test fault",
        what: "the real binary's exact private internal-exit fault (`A1-726`)",
        invoke: internal_fault,
    },
];

#[test]
fn only_the_exact_private_environment_value_injects_the_internal_fault() {
    let dir = scratch();
    let harness = harness_record(dir.path());
    let absent = common::command(dir.path(), &["facts"])
        .env_remove(INTERNAL_FAULT_ENV)
        .env(HARNESS_RECORD_ENV, &harness)
        .output()
        .expect("facts without the private fault");
    assert_eq!(absent.status.code(), Some(0));
    for value in [
        "",
        "internal",
        "internal-exit-census",
        "internal-exit-census-v0",
        "internal-exit-census-v1-near-miss",
    ] {
        let out = common::command(dir.path(), &["facts"])
            .env(INTERNAL_FAULT_ENV, value)
            .env(HARNESS_RECORD_ENV, &harness)
            .output()
            .expect("near-miss facts run");
        assert_eq!(
            out.status.code(),
            Some(0),
            "near-miss fault value {value:?} changed an ordinary command: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let argument = common::run(dir.path(), &["facts", "--require", INTERNAL_FAULT_VALUE]);
    assert_ne!(
        argument.code, 3,
        "the private token became a CLI route to internal exit: {argument:?}"
    );
    assert!(
        !argument
            .text()
            .contains("deliberate internal-exit test fault")
    );

    let no_harness = common::command(dir.path(), &["facts"])
        .env(INTERNAL_FAULT_ENV, INTERNAL_FAULT_VALUE)
        .env_remove(HARNESS_RECORD_ENV)
        .output()
        .expect("fault token outside the test harness");
    assert_eq!(
        no_harness.status.code(),
        Some(0),
        "the private fault escaped its test-harness boundary: {}{}",
        String::from_utf8_lossy(&no_harness.stdout),
        String::from_utf8_lossy(&no_harness.stderr)
    );

    let stale_harness = common::command(dir.path(), &["facts"])
        .env(INTERNAL_FAULT_ENV, INTERNAL_FAULT_VALUE)
        .env(HARNESS_RECORD_ENV, dir.path().join("not-created"))
        .output()
        .expect("fault token with a stale harness path");
    assert_eq!(
        stale_harness.status.code(),
        Some(0),
        "a stale harness marker enabled the private fault: {}{}",
        String::from_utf8_lossy(&stale_harness.stdout),
        String::from_utf8_lossy(&stale_harness.stderr)
    );
}

/// The `exit_codes` map the running build publishes.
fn published() -> serde_json::Map<String, serde_json::Value> {
    let dir = scratch();
    let out = common::run(dir.path(), &["facts"]);
    assert_eq!(out.code, 0, "`facts` did not run: {}", out.text());
    let facts: serde_json::Value = serde_json::from_str(&out.stdout).expect("facts is json");
    facts
        .get("exit_codes")
        .and_then(|v| v.as_object())
        .cloned()
        .expect("`facts` publishes the exit codes")
}

/// **Every published code has a probe, and the probe produces it.**
///
/// Enumerated from `facts` rather than from a list here, so the two failures
/// that matter are both caught: a code with no probe, and a probe whose
/// invocation stopped producing the code it is named for.
#[test]
fn every_published_exit_code_is_produced_by_a_run() {
    let published = published();
    assert!(
        published.len() >= 4,
        "`facts` published {} exit code(s), so this gate is checking almost nothing",
        published.len()
    );

    let mut wrong = Vec::new();
    for (name, value) in &published {
        let Some(probe) = PROBES.iter().find(|p| p.code == name) else {
            wrong.push(format!(
                "`{name}` is published and no run in this file produces it"
            ));
            continue;
        };
        let expected = value.as_i64().expect("an exit code is a number");
        let dir = scratch();
        let run = (probe.invoke)(dir.path());
        let got = i64::from(run.code);
        if got != expected {
            wrong.push(format!(
                "`{name}` is published as {expected} and {} exited {got}: {}",
                probe.what,
                run.text().trim()
            ));
        } else if !run.text().contains(probe.says) {
            wrong.push(format!(
                "`{name}` exited {got} without saying `{}`, so the probe reached the code \
                 by some other route than {}: {}",
                probe.says,
                probe.what,
                run.text().trim()
            ));
        }
    }
    for probe in PROBES {
        if !published.contains_key(probe.code) {
            wrong.push(format!(
                "{} is written for `{}`, which this build does not publish",
                probe.what, probe.code
            ));
        }
    }
    assert!(wrong.is_empty(), "exit codes:\n  {}", wrong.join("\n  "));
}

/// **The four codes are four different numbers.**
///
/// A build that mapped `usage` and `internal` to the same number would pass the
/// gate above twice over while making the distinction ADR-0007 draws
/// unavailable to the script reading it.
#[test]
fn the_published_codes_are_distinct() {
    let published = published();
    let mut seen: std::collections::BTreeMap<i64, Vec<&str>> =
        std::collections::BTreeMap::default();
    for (name, value) in &published {
        seen.entry(value.as_i64().expect("a number"))
            .or_default()
            .push(name);
    }
    let collisions: Vec<_> = seen
        .iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(code, names)| format!("{code} is {}", names.join(" and ")))
        .collect();
    assert!(
        collisions.is_empty(),
        "a consumer branches on these, so two names sharing one number is a contract \
         defect: {}",
        collisions.join(", ")
    );
}

/// **`3` means the run proved nothing**, and says so in words a person can act
/// on rather than printing a diagnostic that reads like a finding.
///
/// The code alone is not the contract: a script branches on it, and a person
/// reads the line above it. `A1-091` says a message may name only what
/// happened, and what happened here is that `aplan` failed its own self-check.
#[test]
fn the_internal_code_says_it_is_aplans_fault_and_not_the_plans() {
    // The same route the gate above enumerates, so there is one place to repair
    // when it stops producing `3` rather than two that can disagree.
    let probe = PROBES
        .iter()
        .find(|p| p.code == "internal")
        .expect("an `internal` probe");
    let dir = scratch();
    let out = (probe.invoke)(dir.path());
    assert_eq!(out.code, 3, "{}", out.text());
    let text = out.text();
    assert!(
        text.contains("bug in aplan"),
        "exit 3 has to name itself as aplan's defect, or a reader files it against \
         their plan: {text}"
    );
    assert!(
        !text.contains("APLAN"),
        "a rule code here would read as a finding about the plan, which is the one \
         thing this exit code exists to distinguish: {text}"
    );
}
