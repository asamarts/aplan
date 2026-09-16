//! Policy scoped to a subset of the plan (`A1-183`).
//!
//! Every dial in `.aplan.yml` was whole-plan, and the one exception --
//! `evidence.exempt_tags` -- is the degenerate case of scoping: "required
//! strength is *none* for these tags". These pin the general form of it, the
//! precedence between scopes, and that the whole-plan setting still applies
//! where no scope matches.
//!
//! The subset is named with the four dimensions the command line takes
//! (`A1-214`, ADR-0026), so what a subset *is* has one definition and two
//! renderings.

mod common;

fn plan(dir: &std::path::Path, config: &str) {
    let tasks = dir.join("aplan/tasks");
    std::fs::create_dir_all(&tasks).expect("tasks dir");
    std::fs::write(dir.join(".aplan.yml"), config).expect("config");
    for (id, tag) in [("T-1", "cross-cutting"), ("T-2", "spike")] {
        std::fs::write(
            tasks.join(format!("{id}.md")),
            format!(
                "---\n\
                 id: {id}\n\
                 title: closed on a commit\n\
                 phase: 0\n\
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
