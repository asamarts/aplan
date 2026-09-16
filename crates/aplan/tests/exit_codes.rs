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
//! **`3` is only reachable through a defect**, by construction: its single
//! producer is `CloseError::Produced`, which means `close` built frontmatter it
//! could not read back, and that is a bug in `aplan` rather than anything a
//! plan can ask for. Which is why the mapping was all anyone had tested. The
//! probe below therefore rides on a real one (`A1-250`): the writer emits list
//! entries as plain YAML scalars, so an argument that a plain scalar cannot
//! carry does not round-trip. When that is fixed the probe stops producing `3`
//! and this file fails -- which is the point. The gate enumerates the codes
//! from `facts`, so the repair is to find the next route to `3`, not to delete
//! the probe: dropping it fails the same assertion from the other side.

use std::path::Path;

mod common;

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
        says: "bug in aplan",
        what: "`dep add --after` naming an id that loads and cannot be written back \
               (`A1-250`)",
        invoke: |dir| {
            plan(dir, "");
            common::run(dir, &["dep", "add", "T-1", "--after", "  spaced"])
        },
    },
];

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
    let dir = scratch();
    plan(dir.path(), "");
    let out = common::run(dir.path(), &["dep", "add", "T-1", "--after", "  spaced"]);
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
