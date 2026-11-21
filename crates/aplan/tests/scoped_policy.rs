//! Policy scoped to a subset of the plan (`A1-183`).
//!
//! Every dial in `.aplan.yml` was whole-plan, and the one exception --
//! `evidence.exempt_tags` -- is the degenerate case of scoping: "required
//! strength is *none* for these tags". These pin the general form of it, the
//! precedence between scopes, and that the whole-plan setting still applies
//! where no scope matches.
//!
//! The subset is named with the five dimensions the command line takes
//! (`A1-214`, ADR-0026), so what a subset *is* has one definition and two
//! renderings.

use crate::common;

fn plan(dir: &std::path::Path, config: &str) {
    let tasks = dir.join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("tasks dir");
    std::fs::write(dir.join(".aplan.yml"), config).expect("config");
    for (id, phase, tag) in [("T-1", 0, "cross-cutting"), ("T-2", 1, "spike")] {
        std::fs::write(
            tasks.join(format!("{id}.md")),
            format!(
                "---\n\
                 id: {id}\n\
                 title: closed on a commit\n\
                 phase: {phase}\n\
                 status: done\n\
                 owner: t\n\
                 effort_days: 1\n\
                 paths: [a.txt]\n\
                 tags: [{tag}]\n\
                 evidence:\n\
                 \x20 - kind: commit\n\
                 \x20   ref: deadbeef\n\
                 ---\n\n## Why\n\nx\n"
            ),
        )
        .expect("task");
    }
}

/// A numeric phase is the fifth shared policy dimension. This and the CLI
/// matrix are the executable proof for `selection.phase_coordinate`.
#[test]
fn a_scope_can_select_a_phase() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(
        dir.path(),
        "evidence:\n  require_min_strength: [test]\n\
         policy:\n  - when: { phase: [1] }\n    evidence:\n      require_min_strength: []\n",
    );
    let out = common::run(dir.path(), &["check"]);
    let all = format!("{}{}", out.stdout, out.stderr);
    assert!(
        all.contains("T-1"),
        "the phase-0 task lost the whole-plan requirement:\n{all}"
    );
    assert!(
        !all.contains("T-2"),
        "the phase-1 task was not selected by the scope:\n{all}"
    );

    let contract = common::run(dir.path(), &["export-agents-md"]).stdout;
    assert!(
        contract.contains("In phase `1`, no evidence strength is required"),
        "the generated contract hid the phase scope:\n{contract}"
    );
}

/// **A subset can carry its own evidence policy, and the rest of the plan keeps
/// the whole-plan one** (`A1-183`).
///
/// `require_min_strength: []` inside a scope is how `evidence.exempt_tags` is
/// expressible in the general form: it says "nothing is required here", which is
/// exactly what the exemption says globally.
#[test]
fn a_scope_can_exempt_its_subset_while_the_rest_of_the_plan_is_unchanged() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(
        dir.path(),
        "evidence:\n  require_min_strength: [test]\n\
         policy:\n  - when: { tag: [spike] }\n    evidence:\n      require_min_strength: []\n",
    );
    let out = common::run(dir.path(), &["check"]);
    assert!(
        out.stdout.contains("T-1") || out.stderr.contains("T-1"),
        "the task outside the scope lost the whole-plan requirement:\n{}{}",
        out.stdout,
        out.stderr
    );
    let all = format!("{}{}", out.stdout, out.stderr);
    assert!(
        !all.contains("T-2"),
        "the scoped task was still held to the whole-plan requirement:\n{all}"
    );
    // And the message never asks for one of nothing, which is what an empty
    // requirement produced before it was read as "no requirement".
    assert!(!all.contains("need one of: )"), "{all}");
}

/// A scope that names no subset is refused rather than read as the whole plan,
/// and the refusal says what to write instead (`A1-183`).
#[test]
fn a_scope_naming_no_subset_is_refused_by_name() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(
        dir.path(),
        "evidence:\n  require_min_strength: [test]\n\
         policy:\n  - when: {}\n    evidence:\n      require_min_strength: []\n",
    );
    let out = common::run(dir.path(), &["check"]);
    let all = format!("{}{}", out.stdout, out.stderr);
    assert!(all.contains("names no subset"), "{all}");
    assert!(
        all.contains("`cut`"),
        "the refusal does not say what to write: {all}"
    );
}

/// **The generated agent contract states the scoped policy, not only the
/// whole-plan one** (`A1-183`).
///
/// This file is the only thing an agent reads before it closes a task. Told one
/// sentence where two apply, it is told the wrong rule for the subset it is
/// working in, and the mistake it then makes looks like a judgement rather than
/// a missing line.
#[test]
fn the_generated_contract_states_a_scoped_policy_beside_the_whole_plan_one() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(
        dir.path(),
        "evidence:\n  require_min_strength: [test]\n\
         policy:\n  - when: { tag: [spike] }\n    evidence:\n      require_min_strength: []\n",
    );
    let out = common::run(dir.path(), &["export-agents-md"]);
    let text = out.stdout;
    assert!(
        text.contains("must cite `test` evidence"),
        "the whole-plan rule is missing:\n{text}"
    );
    assert!(
        text.contains("In tag `spike`, no evidence strength is required"),
        "the scoped rule is missing, so an agent working a spike is told the wrong \
         requirement:\n{text}"
    );
    // And it says which wins, because two rules without a precedence is worse
    // than one rule that is sometimes wrong.
    assert!(text.contains("narrowest scope that matches"), "{text}");
}

/// A scope may also make a subset **stricter**, which is the direction
/// `evidence.exempt_tags` could never express (`A1-183`).
///
/// The whole-plan setting accepts a `commit`; the scope demands a `test`, so the
/// task inside it is refused and the one outside is not. This is the case the
/// exemption mechanism had no way of saying at all.
#[test]
fn a_scope_can_tighten_a_subset_as_well_as_relax_it() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(
        dir.path(),
        "evidence:\n  require_min_strength: [commit]\n\
         policy:\n  - when: { tag: [spike] }\n    evidence:\n      require_min_strength: [test]\n",
    );
    let out = common::run(dir.path(), &["check"]);
    let all = format!("{}{}", out.stdout, out.stderr);
    assert!(
        all.contains("T-2"),
        "the scoped task was not held to the stricter requirement:\n{all}"
    );
    assert!(
        !all.contains("T-1"),
        "the task outside the scope was held to the scope's requirement:\n{all}"
    );

    // And the contract says which requirement applies where, in the strict
    // direction as well as the relaxed one.
    let contract = common::run(dir.path(), &["export-agents-md"]).stdout;
    assert!(
        contract.contains("In tag `spike`, a `done` task must cite `test` evidence"),
        "the stricter scope is not stated in the contract:\n{contract}"
    );
}

/// Two dimensions in one scope narrow rather than widen, which is the reading the
/// command line already has (`A1-214`, ADR-0026).
#[test]
fn naming_two_dimensions_in_a_scope_narrows_it() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    // `T-2` is the spike, and its owner is `t`; naming a different owner beside
    // the tag must stop the scope matching it at all.
    plan(
        dir.path(),
        "evidence:\n  require_min_strength: [test]\n\
         policy:\n  - when: { tag: [spike], owner: [nobody] }\n    evidence:\n      \
         require_min_strength: []\n",
    );
    let out = common::run(dir.path(), &["check"]);
    let all = format!("{}{}", out.stdout, out.stderr);
    assert!(
        all.contains("T-2"),
        "the scope matched on the tag alone, so the second dimension widened it \
         instead of narrowing it:\n{all}"
    );
}

/// **A status the model does not have is refused, by name, exactly as the command
/// line refuses it** (`A1-183`'s audit).
///
/// The two renderings of a subset must disagree about nothing, and this one
/// disagreed in the worst direction: `--status donn` was refused with the list of
/// real statuses, while `when: { status: [donn] }` was accepted and the word
/// quietly dropped. The dimension stayed *named*, so it still counted toward
/// precedence while constraining nothing -- a scope meant for one status matched
/// the whole plan, and a policy that switched the evidence requirement off
/// switched it off everywhere. `check` reported `ok`.
#[test]
fn a_status_the_model_does_not_have_is_refused_in_a_scope_too() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(
        dir.path(),
        "evidence:\n  require_min_strength: [test]\n\
         policy:\n  - when: { status: [donn] }\n    evidence:\n      require_min_strength: []\n",
    );
    let out = common::run(dir.path(), &["check"]);
    let all = format!("{}{}", out.stdout, out.stderr);
    assert!(
        all.contains("`donn`") && all.contains("is not a status"),
        "the typo was not refused by name:\n{all}"
    );
    // And it lists what a status may be, so the reader does not have to find them.
    assert!(all.contains("`in_progress`"), "{all}");
    assert!(
        !all.contains("ok:"),
        "the plan was accepted with a scope that constrains nothing:\n{all}"
    );
}

/// `APLAN058` (`A2-024`): a policy selector naming a cut or tag that cannot
/// match the plan warns with a did-you-mean, while valid milestone ids, cut
/// labels, task tags and `tag_families` members stay silent.
#[test]
fn a_scope_naming_an_unknown_cut_or_tag_warns_with_a_suggestion() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(
        dir.path(),
        "deny_warnings: true\n\
         evidence:\n  require_min_strength: [commit]\n\
         policy:\n  - when: { cut: [M-on] }\n    evidence:\n      require_min_strength: []\n\
         \x20 - when: { tag: [spke] }\n    evidence:\n      require_min_strength: []\n",
    );
    std::fs::write(
        dir.path().join("aplan/tasks/M-one.md"),
        "---\nid: M-one\ntitle: First cut\nphase: 0\nstatus: todo\ntags: [milestone, v0.1]\n\
         blocked_by: [T-2]\n---\n\n## Why\n\nx\n",
    )
    .expect("milestone");

    let out = common::run(dir.path(), &["check"]);
    let all = format!("{}{}", out.stdout, out.stderr);
    assert_eq!(out.code, 1, "deny_warnings must block on APLAN058:\n{all}");
    assert!(all.contains("APLAN058"), "{all}");
    assert!(
        all.contains("`M-on`") && all.contains("did you mean `M-one`?"),
        "cut typo did not suggest milestone id:\n{all}"
    );
    assert!(
        all.contains("`spke`") && all.contains("did you mean `spike`?"),
        "tag typo did not suggest carried tag:\n{all}"
    );

    // JSON output carries the config file path and warning severity for both findings.
    let json_out = common::run(dir.path(), &["check", "--format", "json"]);
    assert_eq!(json_out.code, 1, "{}", json_out.text());
    let doc: serde_json::Value =
        serde_json::from_str(&json_out.stdout).expect("valid json check report");
    let diags = doc["diagnostics"].as_array().expect("diagnostics array");
    let aplan058: Vec<&serde_json::Value> = diags
        .iter()
        .filter(|d| d["code"].as_str() == Some("APLAN058"))
        .collect();
    assert_eq!(aplan058.len(), 2, "{doc:#}");
    for d in aplan058 {
        assert_eq!(d["severity"].as_str(), Some("warning"), "{d:#}");
        assert!(
            d["file"]
                .as_str()
                .is_some_and(|f| f.ends_with(".aplan.yml")),
            "APLAN058 should point at .aplan.yml: {d:#}"
        );
    }

    // Fix the selectors: use the cut label (`v0.1`) and a tag declared only in
    // `tag_families` (`kind-docs`) alongside carried tag `spike`.
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "deny_warnings: true\n\
         tag_families:\n  kind: [kind-docs, spike]\n\
         evidence:\n  require_min_strength: [commit]\n\
         policy:\n  - when: { cut: [v0.1] }\n    evidence:\n      require_min_strength: []\n\
         \x20 - when: { tag: [kind-docs] }\n    evidence:\n      require_min_strength: []\n",
    )
    .expect("config");

    let clean = common::run(dir.path(), &["check"]);
    let clean_all = format!("{}{}", clean.stdout, clean.stderr);
    assert_eq!(clean.code, 0, "valid selectors must pass:\n{clean_all}");
    assert!(!clean_all.contains("APLAN058"), "{clean_all}");
}

fn assert_aplan059_findings(dir: &std::path::Path) {
    let out = common::run(dir, &["check", "--format", "json"]);
    assert_eq!(
        out.code,
        1,
        "deny_warnings must block on APLAN059:\n{}",
        out.text()
    );
    let doc: serde_json::Value = serde_json::from_str(&out.stdout).expect("json report");
    let aplan059: Vec<&serde_json::Value> = doc["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter(|d| d["code"].as_str() == Some("APLAN059"))
        .collect();
    assert_eq!(aplan059.len(), 2, "{doc:#}");
    assert!(
        aplan059.iter().any(|d| {
            let msg = d["message"].as_str().unwrap_or_default();
            msg.contains("`evidence.require_min_strength`")
                && msg.contains("`url`")
                && msg.contains("`url.check`")
        }),
        "missing top-level url warning: {aplan059:#?}"
    );
    assert!(
        aplan059.iter().any(|d| {
            let msg = d["message"].as_str().unwrap_or_default();
            msg.contains("`policy[0].evidence.require_min_strength`")
                && msg.contains("`test`")
                && msg.contains("not configured")
        }),
        "missing scoped test warning: {aplan059:#?}"
    );
    for d in &aplan059 {
        assert_eq!(d["severity"].as_str(), Some("warning"), "{d:#}");
        assert!(
            d["file"]
                .as_str()
                .is_some_and(|f| f.ends_with(".aplan.yml")),
            "APLAN059 should point at .aplan.yml: {d:#}"
        );
    }
}

/// `APLAN059` (`A2-025`, #33 item 6): requiring an evidence kind with no
/// configured resolver warns in `check` (blocking under `deny_warnings`),
/// names the policy entry and the missing `.aplan.yml` key consistently with
/// `vacuity`, and clears once the resolver is configured.
#[test]
fn a_policy_requiring_an_unconfigured_resolver_warns_and_agrees_with_vacuity() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    plan(
        dir.path(),
        "deny_warnings: true\n\
         evidence:\n  require_min_strength: [commit, url]\n\
         policy:\n  - when: { tag: [spike] }\n    evidence:\n      require_min_strength: [test]\n",
    );
    // Add a task citing a test and url so `vacuity` has subjects for both
    // resolver-gated rules and names the exact config keys `test` and `url.check`.
    std::fs::write(
        dir.path().join("aplan/tasks/T-3.md"),
        "---\nid: T-3\ntitle: cited test and url\nphase: 1\nstatus: done\neffort_days: 1\n\
         paths: [a.txt]\ntags: [spike]\nevidence:\n  - kind: test\n    ref: bogus_test\n\
         \x20 - kind: url\n    ref: https://example.invalid/\n---\n\n## Why\n\nx\n",
    )
    .expect("task");

    assert_aplan059_findings(dir.path());

    // `vacuity` uses the same `test` and `url.check` keys and never reports
    // `APLAN059` as switched off.
    let vac = common::run(dir.path(), &["vacuity"]);
    let vac_text = vac.text();
    assert!(
        vac_text.contains("APLAN013") && vac_text.contains("`test` is off"),
        "vacuity should report `test` off: {vac_text}"
    );
    assert!(
        vac_text.contains("APLAN016") && vac_text.contains("`url.check` is off"),
        "vacuity should report `url.check` off: {vac_text}"
    );
    assert!(
        !vac_text.contains("APLAN059"),
        "vacuity must not report APLAN059 as switched off: {vac_text}"
    );

    // Configure both resolvers (and update T-2 to cite a test so APLAN017 is
    // satisfied); `APLAN059` disappears cleanly.
    std::fs::write(
        dir.path().join("aplan/tasks/T-2.md"),
        "---\nid: T-2\ntitle: spike with test\nphase: 1\nstatus: done\nowner: t\n\
         effort_days: 1\npaths: [a.txt]\ntags: [spike]\nevidence:\n  - kind: test\n    ref: ok_test\n\
         ---\n\n## Why\n\nx\n",
    )
    .expect("T-2");
    std::fs::write(
        dir.path().join(".aplan.yml"),
        "deny_warnings: true\n\
         test:\n  list: [echo, ok_test]\n  run: [\"true\", \"{test}\"]\n\
         url:\n  check: [\"true\", \"{url}\"]\n\
         evidence:\n  require_min_strength: [commit, url]\n\
         policy:\n  - when: { tag: [spike] }\n    evidence:\n      require_min_strength: [test]\n",
    )
    .expect("config with resolvers");

    let clean = common::run(dir.path(), &["check"]);
    assert_eq!(
        clean.code,
        0,
        "configured resolvers must clear APLAN059:\n{}",
        clean.text()
    );
    assert!(!clean.text().contains("APLAN059"), "{}", clean.text());
}

/// The #33 item 6 end-to-end flow: with a scoped `require_min_strength: [test]`
/// and no `test` runner under `deny_warnings: true`, both `verify` and MCP
/// `check` block on `APLAN059`; once a runner is configured, `check` passes and
/// `close --evidence test:no_such_test_anywhere` is refused with `APLAN013`.
#[test]
fn unconfigured_test_policy_blocks_verify_and_mcp_until_a_runner_is_added() {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("aplan/tasks")).expect("tasks");
    std::fs::write(
        root.join(".aplan.yml"),
        "deny_warnings: true\n\
         policy:\n  - when: { tag: [kind-impl] }\n    evidence:\n      require_min_strength: [test]\n",
    )
    .expect("config");
    std::fs::write(
        root.join("aplan/tasks/T-1.md"),
        "---\nid: T-1\ntitle: impl task\nphase: 1\nstatus: todo\neffort_days: 1\n\
         paths: [a.txt]\ntags: [kind-impl]\n---\n\n## Why\n\nx\n",
    )
    .expect("task");
    std::fs::write(root.join("a.txt"), "hello\n").expect("file");
    for args in [
        &["init", "-q"][..],
        &["config", "user.email", "t@example.invalid"],
        &["config", "user.name", "t"],
        &["add", "-A"],
        &["commit", "-qm", "init"],
    ] {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}");
    }

    let verified = common::run(root, &["verify"]);
    assert_eq!(
        verified.code,
        1,
        "verify must deny on APLAN059:\n{}",
        verified.text()
    );
    assert!(verified.text().contains("APLAN059"), "{}", verified.text());

    let resolved = aplan_app::resolve_explicit_root(root.to_str().expect("utf8")).expect("root");
    let options = aplan_app::configured_options(
        &resolved.config,
        aplan_app::ConfigurationInputs {
            directory: resolved.configuration_directory.as_deref(),
            file: resolved.configuration_file.clone(),
            traces_repository: Some(resolved.repository_root.clone()),
            ..Default::default()
        },
    );
    let policy = aplan_mcp::Policy {
        id_strategy: resolved.config.id_strategy.unwrap_or_default(),
        options: options.clone(),
        verdict: aplan_app::configured_verdict(&resolved.config),
        verify: aplan_app::configured_verification(
            &resolved.config,
            resolved.configuration_file.clone(),
            options.traces_root.clone(),
        ),
        repo_root: resolved.repository_root.clone(),
    };
    let mcp = aplan_mcp::tools::check(&resolved.plan_root, &policy).expect("mcp check");
    assert_eq!(mcp["denied"], serde_json::json!(true), "{mcp:#}");
    assert!(
        mcp["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|d| d["code"].as_str() == Some("APLAN059")),
        "MCP check must report APLAN059: {mcp:#}"
    );

    // Wire a runner: APLAN059 clears, and closing with an unresolvable test is refused.
    std::fs::write(
        root.join(".aplan.yml"),
        "deny_warnings: true\n\
         test:\n  list: [echo, real_test]\n  run: [\"true\", \"{test}\"]\n\
         policy:\n  - when: { tag: [kind-impl] }\n    evidence:\n      require_min_strength: [test]\n",
    )
    .expect("config with runner");
    let ok = common::run(root, &["check"]);
    assert_eq!(ok.code, 0, "{}", ok.text());
    let refused = common::run(
        root,
        &["close", "T-1", "--evidence", "test:no_such_test_anywhere"],
    );
    assert_ne!(
        refused.code,
        0,
        "bogus test must be refused:\n{}",
        refused.text()
    );
    assert!(refused.text().contains("APLAN013"), "{}", refused.text());
}
