//! `aplan vacuity`, what each rule could actually have failed on (`A1-114`).
//!
//! A rule that evaluates to "pass" and a rule that had nothing to evaluate
//! produce the same output, and the second is the more common state. The second
//! pilot named the general form of that while reporting a capability check whose
//! shell substitution had been eaten: it asserted nothing, exited `0`, and every
//! gate passed.

use std::path::Path;

use crate::common;
use common::aplan;

/// The p00025 case, end to end: every `done` task cites a commit and none
/// declares `paths`, so `APLAN012` ran on all of them and could not have failed
/// on one. Reported *as that*, rather than left for `APLAN023` to say something
/// adjacent about the same tasks.
#[test]
fn a_plan_with_no_paths_reports_the_relevance_rule_as_blind() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    for id in ["T-1", "T-2"] {
        done(dir.path(), id, &[], "commit:abc1234");
    }

    let (out, code) = aplan(dir.path(), &["vacuity"]);
    assert_eq!(code, 0, "a measurement is not a verdict: {out}");
    assert!(out.contains("APLAN012"), "{out}");

    let (json, _) = aplan(dir.path(), &["vacuity", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    let rule = |code: &str| -> serde_json::Value {
        doc["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .find(|r| r["rule"] == code)
            .unwrap_or_else(|| panic!("{code} is missing from {json}"))
            .clone()
    };
    let relevance = rule("APLAN012");
    assert_eq!(relevance["subjects"], 2, "{json}");
    assert_eq!(relevance["blind"], 2, "{json}");
    assert_eq!(relevance["checked"], 0, "{json}");
    assert_eq!(relevance["needs"][0], "paths", "{json}");

    // And this is `APLAN023`'s subject too, deliberately: the two say different
    // things about the same tasks, and neither replaces the other.
    let (check, _) = aplan(dir.path(), &["check"]);
    assert!(check.contains("APLAN023"), "{check}");
}

/// Declaring `paths` is what moves a subject from blind to checked, which is the
/// number an adopter watches come down.
#[test]
fn declaring_paths_is_what_the_number_measures() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    done(dir.path(), "T-1", &["src/**"], "commit:abc1234");
    done(dir.path(), "T-2", &[], "commit:abc1234");

    let (json, _) = aplan(dir.path(), &["vacuity", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("json");
    let relevance = doc["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|r| r["rule"] == "APLAN012")
        .expect("APLAN012");
    assert_eq!(relevance["subjects"], 2, "{json}");
    assert_eq!(relevance["blind"], 1, "{json}");
    assert_eq!(relevance["checked"], 1, "{json}");
}

/// A rule has no subject in a task it does not apply to, which is a different
/// thing from having nothing to bind against. Counting the two together would
/// make one number mean two things.
#[test]
fn a_task_a_rule_does_not_apply_to_is_not_counted_as_blind() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    // Live, so no evidence rule has a subject in it.
    std::fs::write(
        dir.path().join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: t\nphase: 1\nstatus: todo\neffort_days: 1\n---\n\nbody\n",
    )
    .expect("task");

    let (out, code) = aplan(dir.path(), &["vacuity"]);
    assert_eq!(code, 0, "{out}");
    // No *evidence* rule has a subject here, which is what this is about. The
    // report is no longer empty, because `A1-269` gives a row to a rule the
    // configuration switched off -- `APLAN022` under no `require_cut_membership`
    // has a real subject in this task and really did not run. That is a
    // different fact from the one asserted here, and asserting the whole report
    // was empty conflated them.
    for evidence_rule in ["APLAN012", "APLAN023", "APLAN024", "APLAN031"] {
        assert!(
            !out.lines().any(|l| l.starts_with(evidence_rule)),
            "`{evidence_rule}` has no subject in a live task: {out}"
        );
    }
}

/// The report says what a rule needed, because "blind" without the reason is a
/// number nobody can act on.
#[test]
fn the_report_names_what_each_rule_needed() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    done(dir.path(), "T-1", &[], "commit:abc1234");

    let (out, _) = aplan(dir.path(), &["vacuity"]);
    assert!(out.contains("`paths`"), "{out}");
    assert!(
        out.contains("could not have failed"),
        "and what blind means: {out}"
    );
}

/// It reads the plan and never git, so it belongs beside `check` rather than in
/// the `verify` family: whether a predicate had anything to bind against is a
/// property of the task file.
#[test]
fn it_is_hermetic() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(dir.path());
    done(dir.path(), "T-1", &[], "commit:abc1234");
    assert!(!dir.path().join(".git").exists(), "no repository here");

    let (out, code) = aplan(dir.path(), &["vacuity"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("APLAN012"), "{out}");
}

fn plan(dir: &Path) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::write(dir.join(".aplan.yml"), "root: aplan\n").expect("config");
}

fn done(dir: &Path, id: &str, paths: &[&str], evidence: &str) {
    let (kind, reference) = evidence.split_once(':').expect("kind:ref");
    let paths = if paths.is_empty() {
        String::new()
    } else {
        let mut block = String::from("paths:\n");
        for p in paths {
            block.push_str("  - ");
            block.push_str(p);
            block.push('\n');
        }
        block
    };
    std::fs::write(
        dir.join(format!("aplan/tasks/{id}.md")),
        format!(
            "---\nid: {id}\ntitle: {id}\nphase: 1\nstatus: done\neffort_days: 1\n{paths}\
             evidence:\n  - kind: {kind}\n    ref: '{reference}'\n---\n\nbody\n"
        ),
    )
    .expect("task");
}

// --- `A1-269`: a rule the configuration switched off ------------------------
//
// `vacuity` measured what the *task* carried and reported a rule the config had
// stopped as having checked its subjects -- not silence, but a wrong answer from
// the command built to tell those two apart. `p003`'s `K0-010` fixture is where
// that cost something: it changed a traced section on purpose, expected the
// stale fingerprint to be refused, and its fixture-local config omitted
// `detect_drift`, so the case it was written for never ran.

/// A closed task whose traced section is stamped, which is `APLAN018`'s subject.
fn stamped(dir: &Path, config: &str) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("docs")).expect("docs");
    std::fs::write(dir.join(".aplan.yml"), config).expect("config");
    std::fs::write(
        dir.join("docs/d.md"),
        "# Decision\n\n## The choice\n\nWe picked B.\n",
    )
    .expect("doc");
    std::fs::write(
        dir.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: a decision\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths:\n- \"docs/**\"\ntraces:\n- docs/d.md#the-choice\n\
         evidence:\n  - kind: file\n    ref: docs/d.md\n\
         trace_fingerprints:\n  docs/d.md#the-choice: deadbeefdeadbeef\n---\n\n\
         ## Acceptance criteria\n\n- [x] done\n",
    )
    .expect("task");
}

/// **The case that cost a pilot the case it was testing.**
#[test]
fn a_rule_the_config_switched_off_says_so_instead_of_checked() {
    let dir = tempfile::Builder::new()
        .prefix("vacuity-off-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    stamped(dir.path(), "root: aplan\n");

    let out = common::run(dir.path(), &["vacuity"]);
    assert!(out.ok(), "a measurement is not a verdict: {out:?}");
    let text = out.text();
    let row = text
        .lines()
        .find(|l| l.starts_with("APLAN018"))
        .unwrap_or_else(|| panic!("no row for a rule with a subject: {text}"));
    assert!(
        row.contains("not run") && row.contains("detect_drift"),
        "and says which key stopped it: {row}"
    );
    // `checked` is the column the defect was in: it read `1`.
    let counts: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(
        counts[2], "0",
        "a rule that did not run checked nothing: {row}"
    );

    // With the switch on, the same plan reports it as checked, so the column is
    // about the configuration and not about this task.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "root: aplan\ndetect_drift: true\n",
    )
    .expect("config");
    let on = common::run(dir.path(), &["vacuity"]);
    let on_text = on.text();
    let row = on_text
        .lines()
        .find(|l| l.starts_with("APLAN018"))
        .expect("still a row");
    assert!(!row.contains("not run"), "{row}");
    assert_eq!(row.split_whitespace().collect::<Vec<_>>()[2], "1", "{row}");
}

/// **Nothing new warns, and it still exits `0` whatever it finds.**
///
/// `detect_drift` being off is a policy, not a defect, and a tool that nags
/// about its own optional features is one adopters configure into silence.
#[test]
fn it_measures_and_never_fails_a_build() {
    let dir = tempfile::Builder::new()
        .prefix("vacuity-exit-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    stamped(dir.path(), "root: aplan\n");

    for args in [&["vacuity"][..], &["vacuity", "--format", "json"]] {
        let out = common::run(dir.path(), args);
        assert_eq!(out.code, 0, "{args:?}: {out:?}");
    }
    // And `check` says nothing about it: the report is where this belongs.
    let checked = common::run(dir.path(), &["check"]);
    assert!(
        checked.ok() && !checked.text().contains("detect_drift"),
        "{checked:?}"
    );
}

/// The JSON carries it under a name, absent where the rule ran.
#[test]
fn the_json_names_it_and_omits_it_where_the_rule_ran() {
    let dir = tempfile::Builder::new()
        .prefix("vacuity-json-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    stamped(dir.path(), "root: aplan\n");

    let out = common::run(dir.path(), &["vacuity", "--format", "json"]);
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    let rules = doc["rules"].as_array().expect("rules");
    let row = rules
        .iter()
        .find(|r| r["rule"] == "APLAN018")
        .expect("a row for it");
    assert_eq!(row["not_run"], "detect_drift");
    let ran = rules
        .iter()
        .find(|r| r["rule"] != "APLAN018" && r["not_run"].is_null())
        .expect("a rule that ran");
    assert!(
        ran.get("not_run").is_none_or(serde_json::Value::is_null),
        "absent, not present-and-null, for a rule that ran: {ran}"
    );
}

// --- `A1-274`: rules a resolver's configuration leaves nothing for ----------

/// A repository citing a test and a url, so every resolver-gated rule has a
/// subject. `config` is spliced in whole so each case can say what it leaves out.
fn cited(dir: &Path, config: &str) -> String {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(dir.join(".aplan.yml"), config).expect("config");
    std::fs::write(dir.join("src/a.rs"), "fn main() {}\n").expect("src");
    for args in [
        &["init", "-q", "."][..],
        &["config", "user.email", "t@example.com"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-q", "-m", "work"],
    ] {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git");
    let sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
    std::fs::write(
        dir.join("aplan/tasks/T-1.md"),
        format!(
            "---\n\
             id: T-1\n\
             title: T-1\n\
             phase: 1\n\
             status: done\n\
             effort_days: 1\n\
             paths:\n\
             - \"src/**\"\n\
             evidence:\n\
             \x20 - kind: commit\n\
             \x20   ref: {sha}\n\
             \x20 - kind: test\n\
             \x20   ref: some_test\n\
             \x20 - kind: url\n\
             \x20   ref: https://x.test/r\n\
             ---\n\n\
             ## Acceptance criteria\n\n\
             - [x] done\n"
        ),
    )
    .expect("task");
    sha
}

/// **Every claim `resolver_side_off` makes is true of the rules** (`A1-274`).
///
/// `A1-269` measures its switches by running the rules again with each on. That
/// reaches the hermetic rules and none of the evidence ones, which `verify`
/// produces by opening the repository -- so the resolver-side claims are
/// *stated*, and this is what stops a statement outliving the rule it is about.
/// Each case puts a plan in the configuration the claim is about and fails if
/// the rule speaks anyway.
#[test]
fn every_resolver_gated_rule_is_really_silent() {
    // (config, the rules it claims are inert)
    let cases: &[(&str, &[&str])] = &[
        (
            "root: aplan\n",
            &["APLAN013", "APLAN014", "APLAN025", "APLAN036", "APLAN037"],
        ),
        (
            // A runner that lists the test but claims no locality: `APLAN025`
            // returns early because a runner claiming nothing cannot
            // demonstrate a mismatch. This is `p00002`'s configuration, where
            // claiming something would be false.
            "root: aplan\ntest:\n  list: [\"echo\", \"some_test\"]\n  run: [\"true\", \"{test}\"]\n",
            &["APLAN025"],
        ),
    ];
    for (config, inert) in cases {
        let dir = tempfile::Builder::new()
            .prefix("resolver-off-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        cited(dir.path(), config);

        let verified = common::run(dir.path(), &["verify", "--execute"]);
        // Or every assertion below passes because nothing ran: the first
        // version of this fixture had malformed YAML and this test was green
        // for that reason (`A1-274`).
        assert!(
            !verified.text().contains("failed to load"),
            "the fixture does not load, so nothing here is being tested: {}",
            verified.text()
        );
        for code in *inert {
            assert!(
                !verified.text().contains(code),
                "`{code}` is reported as inert in this configuration and it spoke: {}",
                verified.text()
            );
        }

        // And `vacuity` says so where the rule has a subject at all.
        let vac = common::run(dir.path(), &["vacuity"]);
        let text = vac.text();
        for row in text
            .lines()
            .filter(|l| inert.iter().any(|c| l.starts_with(c)))
        {
            assert!(
                row.contains("not run"),
                "a rule nothing can resolve is not reported as having checked: {row}"
            );
            assert_eq!(row.split_whitespace().collect::<Vec<_>>()[2], "0", "{row}");
        }
    }
}

/// `url` has no checker by default, which is a deliberate policy rather than an
/// omission -- and is exactly why `APLAN016` needs saying rather than assuming.
#[test]
fn a_url_citation_with_no_checker_is_reported_as_not_run() {
    let dir = tempfile::Builder::new()
        .prefix("url-off-")
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    cited(dir.path(), "root: aplan\n");

    let vac = common::run(dir.path(), &["vacuity"]);
    let text = vac.text();
    let row = text
        .lines()
        .find(|l| l.starts_with("APLAN016"))
        .unwrap_or_else(|| panic!("a rule with a subject and no row is the other half: {text}"));
    assert!(
        row.contains("not run") && row.contains("url.check"),
        "{row}"
    );
}

// --- `A2-135`: single-pass vacuity work counters and multi-rule keys --------

fn shared_trace_vacuity_fixture(dir: &Path, config: Option<&str>) {
    std::fs::create_dir_all(dir.join("aplan/tasks")).expect("mkdir");
    std::fs::create_dir_all(dir.join("docs")).expect("docs");
    if let Some(cfg) = config {
        std::fs::write(dir.join(".aplan.yml"), cfg).expect("config");
    }
    std::fs::write(
        dir.join("docs/spec.md"),
        "# Spec\n\n## Alpha\n\nUpdated body.\n\n| Item | Value |\n|---|---|\n| beta | row |\n",
    )
    .expect("md");
    std::fs::write(dir.join("docs/spec.yaml"), "service:\n  mode: strict\n").expect("yaml");
    std::fs::write(
        dir.join("aplan/tasks/M-1.md"),
        "---\nid: M-1\ntitle: cut one\nphase: 1\nstatus: todo\ntags: [milestone]\n\
         blocked_by: [T-1]\n---\n",
    )
    .expect("milestone");
    std::fs::write(
        dir.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: live without effort\nphase: 1\nstatus: todo\n\
         traces:\n- docs/spec.md#alpha\n- docs/spec.yaml#/service\n---\n",
    )
    .expect("T-1");
    std::fs::write(
        dir.join("aplan/tasks/T-2.md"),
        "---\nid: T-2\ntitle: drifted and weak\nphase: 1\nstatus: done\neffort_days: 1\n\
         version_impact: none\npaths:\n- \"docs/**\"\n\
         traces:\n- docs/spec.md#alpha\n- docs/spec.yaml#/service\n\
         evidence:\n  - kind: note\n    ref: reviewed\n    captured: \"2026-10-01\"\n\
         trace_fingerprints:\n  docs/spec.md#alpha: deadbeefdeadbeef\n\
         \x20 docs/spec.yaml#/service: 0000000000000000\n---\n\n\
         ## Acceptance criteria\n\n- [x] done\n",
    )
    .expect("T-2");
    std::fs::write(
        dir.join("aplan/tasks/T-3.md"),
        "---\nid: T-3\ntitle: unstamped and retargeted\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths:\n- \"docs/**\"\n\
         traces:\n- docs/spec.md#alpha\n- docs/spec.md#beta\n- docs/spec.yaml#/service\n\
         evidence:\n  - kind: file\n    ref: docs/spec.md\n\
         trace_fingerprints:\n  docs/spec.md#beta: \"heading:deadbeefdeadbeef\"\n---\n\n\
         ## Acceptance criteria\n\n- [x] done\n",
    )
    .expect("T-3");
}

fn assert_shared_trace_timings(label: &str, stderr: &str) {
    let timing = stderr
        .lines()
        .find(|line| line.starts_with("aplan: timings "))
        .unwrap_or_else(|| panic!("{label}: missing timing diagnostic: {stderr}"));
    for field in [
        "graph_builds=1",
        "trace_reads=2",
        "markdown_parses=1",
        "structured_parses=1",
    ] {
        assert!(
            timing.contains(field),
            "{label}: missing `{field}` in {timing}"
        );
    }
}

fn assert_human_vacuity_rows(label: &str, stdout: &str, expect_trace_drift_row: bool) {
    let human_codes: Vec<&str> = stdout
        .lines()
        .filter_map(|line| {
            let first = line.split_whitespace().next()?;
            first.starts_with("APLAN").then_some(first)
        })
        .collect();
    let mut sorted_human_codes = human_codes.clone();
    sorted_human_codes.sort_unstable();
    assert_eq!(
        human_codes, sorted_human_codes,
        "{label} human: rows must preserve canonical Rule::ALL order:\n{stdout}"
    );
    let expected: &[(&str, &str)] = if expect_trace_drift_row {
        &[
            ("APLAN017", "evidence.require_min_strength"),
            ("APLAN018", "detect_drift"),
            ("APLAN022", "require_cut_membership"),
            ("APLAN028", "detect_drift"),
            ("APLAN044", "version.require_impact"),
            ("APLAN060", "detect_drift"),
        ]
    } else {
        // Under `--cut M-1`, `M-1` holds only live `T-1` so `APLAN018` has 0
        // subjects and human rendering omits 0-subject rows while keeping the
        // remaining disabled rules that have subjects in the cut.
        &[
            ("APLAN017", "evidence.require_min_strength"),
            ("APLAN022", "require_cut_membership"),
            ("APLAN028", "detect_drift"),
            ("APLAN044", "version.require_impact"),
            ("APLAN060", "detect_drift"),
        ]
    };
    for &(code, key) in expected {
        let row = stdout
            .lines()
            .find(|line| line.starts_with(code))
            .unwrap_or_else(|| panic!("{label} human: missing `{code}`:\n{stdout}"));
        assert!(
            row.contains("not run") && row.contains(key),
            "{label} human: expected `{code}` to cite `{key}`: {row}"
        );
        assert_eq!(
            row.split_whitespace().nth(2),
            Some("0"),
            "{label} human: disabled rule must report 0 checked: {row}"
        );
    }
}

/// `A2-135` AC 3 & AC 4: `vacuity` performs one graph construction and at most
/// one parse per trace document, both unscoped and with `--cut`, including when
/// `.aplan.yml` is absent or explicitly disables switches and when multiple
/// rules sit behind `detect_drift`.
#[test]
fn vacuity_performs_one_graph_build_and_one_parse_per_trace_document() {
    for (label, config, extra_args) in [
        ("absent-config", None, &["--root", "aplan"][..]),
        (
            "explicitly-disabled",
            Some(
                "root: aplan\nrequire_effort: false\ndetect_drift: false\n\
                 require_cut_membership: false\n",
            ),
            &[][..],
        ),
        (
            "scoped-cut",
            Some(
                "root: aplan\nrequire_effort: false\ndetect_drift: false\n\
                 require_cut_membership: false\n",
            ),
            &["--cut", "M-1"][..],
        ),
    ] {
        let dir = tempfile::Builder::new()
            .prefix(&format!("vacuity-work-{label}-"))
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        shared_trace_vacuity_fixture(dir.path(), config);

        let mut argv = vec!["--timings", "--cache", "off", "vacuity", "--format", "json"];
        argv.extend_from_slice(extra_args);
        let run = common::run(dir.path(), &argv);
        assert_eq!(run.code, 0, "{label}: {}", run.text());
        assert_shared_trace_timings(label, &run.stderr);

        let doc: serde_json::Value = serde_json::from_str(&run.stdout).expect("vacuity json");
        let rules = doc["rules"].as_array().expect("rules");
        let codes: Vec<&str> = rules.iter().filter_map(|r| r["rule"].as_str()).collect();
        let mut sorted_codes = codes.clone();
        sorted_codes.sort_unstable();
        assert_eq!(
            codes, sorted_codes,
            "{label}: vacuity JSON rows must stay in canonical Rule::ALL ascending order"
        );
        let not_run = |code: &str| -> Option<String> {
            rules
                .iter()
                .find(|r| r["rule"] == code)
                .and_then(|r| r.get("not_run"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        for (code, key) in [
            ("APLAN017", "evidence.require_min_strength"),
            ("APLAN018", "detect_drift"),
            ("APLAN022", "require_cut_membership"),
            ("APLAN028", "detect_drift"),
            ("APLAN044", "version.require_impact"),
            ("APLAN060", "detect_drift"),
        ] {
            assert_eq!(
                not_run(code).as_deref(),
                Some(key),
                "{label}: expected `{code}` under `{key}`: {}",
                run.stdout
            );
        }
        let expected_effort = config.map(|_| "require_effort");
        assert_eq!(not_run("APLAN009").as_deref(), expected_effort, "{label}");

        let mut human_argv = vec!["--timings", "--cache", "off", "vacuity"];
        human_argv.extend_from_slice(extra_args);
        let human = common::run(dir.path(), &human_argv);
        assert_eq!(human.code, 0, "{label} human: {}", human.text());
        assert_shared_trace_timings(&format!("{label} human"), &human.stderr);
        assert_human_vacuity_rows(label, &human.stdout, label != "scoped-cut");
    }
}
