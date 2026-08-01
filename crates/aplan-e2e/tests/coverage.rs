//! The scenario corpus and the coverage-drift audits (ROADMAP §5).
//!
//! Two things live here, sharing one table of scenarios:
//!
//! 1. **The scenario corpus**, for every rule, a *fires* case (a plan that
//!    violates it) and a *silent* case (a near-miss that does not). Each is
//!    materialised to a real temp directory and run through the actual loader
//!    and validator, so this is a genuine end-to-end exercise, not a unit test
//!    with mocks.
//!
//! 2. **The coverage-drift audit**, `every_rule_has_a_fires_and_a_silent_scenario`
//!    walks [`aplan_core::Rule::ALL`] and fails, listing what is missing, if any
//!    rule lacks either case. Because the rule set is generated from the code
//!    (not hand-listed here), adding a rule without covering it turns CI red.
//!    A rule that ships with only its passing case lets its failure path
//!    silently regress; this is the mechanism that forbids that.

use aplan_config::{PLACEHOLDER, TestRunner};
use aplan_core::task::{Evidence, EvidenceKind, Status, Task};
use aplan_core::{Options, Plan, Rule, Severity, load::LoadedTask, load_plan, validate};
use aplan_output::Format;
use git2::Repository;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Expect {
    /// The plan violates the rule: its code must be present.
    Fires,
    /// A deliberate near-miss: the rule is live but its code must be absent.
    Silent,
}

use Expect::{Fires, Silent};

// Each bool toggles one rule's precondition; a struct of switches is the point.
#[allow(clippy::struct_excessive_bools)]
struct Scenario {
    name: &'static str,
    rule: Rule,
    expect: Expect,
    /// `(relative path, contents)` written under the scenario's temp root.
    files: Vec<(String, String)>,
    /// Resolve `traces` anchors against the temp root (enables `APLAN008`).
    traces_root: bool,
    /// Require an `effort_days` estimate (enables `APLAN009`).
    require_effort: bool,
    /// Require commit/test/file evidence on `done` tasks (enables `APLAN017`).
    require_strong: bool,
    /// Detect stale-`done` drift (enables `APLAN018`; also enables `traces_root`).
    detect_drift: bool,
    /// Require every non-milestone task to be a member of exactly one cut
    /// (enables `APLAN022`).
    require_cut_membership: bool,
}

impl Scenario {
    fn traces(mut self) -> Self {
        self.traces_root = true;
        self
    }
    fn effort(mut self) -> Self {
        self.require_effort = true;
        self
    }
    fn strong(mut self) -> Self {
        self.require_strong = true;
        self
    }
    fn drift(mut self) -> Self {
        self.detect_drift = true;
        self.traces_root = true; // drift reads the traced document
        self
    }
    fn cut_membership(mut self) -> Self {
        self.require_cut_membership = true;
        self
    }
}

fn scen(name: &'static str, rule: Rule, expect: Expect, files: Vec<(String, String)>) -> Scenario {
    Scenario {
        name,
        rule,
        expect,
        files,
        traces_root: false,
        require_effort: false,
        require_strong: false,
        detect_drift: false,
        require_cut_membership: false,
    }
}

/// Wrap a YAML frontmatter body as a task file.
fn taskfile(body: &str) -> String {
    format!("---\n{}\n---\n\nbody\n", body.trim())
}

/// A task file at `tasks/<name>.md`.
fn tf(name: &str, body: &str) -> (String, String) {
    (format!("tasks/{name}.md"), taskfile(body))
}

/// A plain document at the temp root, for `traces` to resolve against.
fn doc(name: &str, contents: &str) -> (String, String) {
    (name.to_string(), contents.to_string())
}

/// The corpus: a fires/silent pair per rule. Split into groups only to keep each
/// function readable; the audit sees them concatenated. One pair per rule keeps
/// the [`Rule::ALL`] correspondence easy to eyeball.
fn corpus() -> Vec<Scenario> {
    let mut all = loader_and_identity_rules();
    all.extend(graph_edge_rules());
    all.extend(evidence_and_trace_rules());
    all.extend(effort_note_and_deadlock_rules());
    all.extend(milestone_rules());
    all.extend(drift_rules());
    all
}

/// `APLAN018`: a closed task's traced section is unchanged since closure.
fn drift_rules() -> Vec<Scenario> {
    let doc_body = "# Intro\n\nprose\n";
    // The stamp that matches the live section, so the silent case is genuine.
    let good = aplan_core::anchor_fingerprint(doc_body, "intro").expect("fingerprint");
    let stamped = |fp: &str| {
        format!(
            "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
             evidence:\n  - kind: commit\n    ref: abc1234\n\
             traces:\n  - DOC.md#intro\n\
             trace_fingerprints:\n  DOC.md#intro: '{fp}'"
        )
    };
    vec![
        // Stamp does not match the section: the section drifted since closure.
        scen(
            "trace-drift-fires",
            Rule::TraceDrift,
            Fires,
            vec![
                tf("a", &stamped("0000000000000000")),
                doc("DOC.md", doc_body),
            ],
        )
        .drift(),
        // Stamp matches the live section: no drift.
        scen(
            "trace-drift-silent",
            Rule::TraceDrift,
            Silent,
            vec![tf("a", &stamped(&good)), doc("DOC.md", doc_body)],
        )
        .drift(),
    ]
}

/// `APLAN001`, `APLAN002`: the loader and task identity.
fn loader_and_identity_rules() -> Vec<Scenario> {
    vec![
        // APLAN001, the loader rejects malformed frontmatter.
        scen(
            "schema-fires",
            Rule::Schema,
            Fires,
            vec![tf("bad", "id: [unclosed\ntitle: x\nphase: 0\nstatus: todo")],
        ),
        scen(
            "schema-silent",
            Rule::Schema,
            Silent,
            vec![tf("ok", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo")],
        ),
        // APLAN002, task ids are unique.
        scen(
            "duplicate-id-fires",
            Rule::DuplicateId,
            Fires,
            vec![
                tf("one", "id: DUP\ntitle: one\nphase: 0\nstatus: todo"),
                tf("two", "id: DUP\ntitle: two\nphase: 0\nstatus: todo"),
            ],
        ),
        scen(
            "duplicate-id-silent",
            Rule::DuplicateId,
            Silent,
            vec![
                tf("one", "id: A-1\ntitle: one\nphase: 0\nstatus: todo"),
                tf("two", "id: A-2\ntitle: two\nphase: 0\nstatus: todo"),
            ],
        ),
    ]
}

/// `APLAN003`, `APLAN004`, `APLAN006`: the `blocked_by` graph shape.
fn graph_edge_rules() -> Vec<Scenario> {
    vec![
        // APLAN003, blocked_by references a task that exists.
        scen(
            "unknown-dep-fires",
            Rule::UnknownDep,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nblocked_by: [GHOST]",
            )],
        ),
        scen(
            "unknown-dep-silent",
            Rule::UnknownDep,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nblocked_by: [A-2]",
                ),
                tf("b", "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo"),
            ],
        ),
        // APLAN004, the dependency graph is acyclic.
        scen(
            "cycle-fires",
            Rule::Cycle,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nblocked_by: [A-2]",
                ),
                tf(
                    "b",
                    "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo\nblocked_by: [A-1]",
                ),
            ],
        ),
        scen(
            "cycle-silent",
            Rule::Cycle,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nblocked_by: [A-2]",
                ),
                tf("b", "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo"),
            ],
        ),
        // APLAN006, no task blocks itself.
        scen(
            "self-dep-fires",
            Rule::SelfDep,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nblocked_by: [A-1]",
            )],
        ),
        scen(
            "self-dep-silent",
            Rule::SelfDep,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nblocked_by: [A-2]",
                ),
                tf("b", "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo"),
            ],
        ),
    ]
}

/// `APLAN005`, `APLAN007`, `APLAN008`: evidence-gated `done` and trace anchors.
fn evidence_and_trace_rules() -> Vec<Scenario> {
    vec![
        // APLAN005, done carries evidence.
        scen(
            "done-without-evidence-fires",
            Rule::DoneWithoutEvidence,
            Fires,
            vec![tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: done")],
        ),
        scen(
            "done-without-evidence-silent",
            Rule::DoneWithoutEvidence,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234",
            )],
        ),
        // APLAN007, a done task does not depend on an unfinished one.
        scen(
            "done-before-blocker-fires",
            Rule::DoneBeforeBlocker,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nblocked_by: [A-2]\nevidence:\n  - kind: commit\n    ref: abc1234",
                ),
                tf("b", "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo"),
            ],
        ),
        scen(
            "done-before-blocker-silent",
            Rule::DoneBeforeBlocker,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nblocked_by: [A-2]\nevidence:\n  - kind: commit\n    ref: abc1234",
                ),
                tf(
                    "b",
                    "id: A-2\ntitle: A-2\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: def5678",
                ),
            ],
        ),
        // APLAN008, traces anchors resolve to real headings.
        scen(
            "dangling-trace-fires",
            Rule::DanglingTrace,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - NOPE.md#missing",
            )],
        )
        .traces(),
        scen(
            "dangling-trace-silent",
            Rule::DanglingTrace,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - DOC.md#intro",
                ),
                doc("DOC.md", "# Intro\n\nprose\n"),
            ],
        )
        .traces(),
        // APLAN017, done evidence meets the configured minimum strength.
        scen(
            "weak-evidence-fires",
            Rule::WeakEvidence,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: note\n    ref: a phone call\n    captured: 2026-07-01",
            )],
        )
        .strong(),
        scen(
            "weak-evidence-silent",
            Rule::WeakEvidence,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234",
            )],
        )
        .strong(),
    ]
}

/// `APLAN009`, `APLAN010`, `APLAN020`: effort, dated notes, and deadlock.
fn effort_note_and_deadlock_rules() -> Vec<Scenario> {
    vec![
        // APLAN009, active tasks carry an effort estimate (warning).
        scen(
            "missing-effort-fires",
            Rule::MissingEffort,
            Fires,
            vec![tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo")],
        )
        .effort(),
        scen(
            "missing-effort-silent",
            Rule::MissingEffort,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 2",
            )],
        )
        .effort(),
        // APLAN010, a note used as evidence carries a date.
        scen(
            "undated-note-fires",
            Rule::UndatedNote,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: note\n    ref: spoke to the division",
            )],
        ),
        scen(
            "undated-note-silent",
            Rule::UndatedNote,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: note\n    ref: spoke to the division\n    captured: 2026-07-14",
            )],
        ),
        // APLAN020, a live task is not blocked by a dropped one.
        scen(
            "dropped-blocker-fires",
            Rule::DroppedBlocker,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nblocked_by: [A-2]",
                ),
                tf("b", "id: A-2\ntitle: A-2\nphase: 0\nstatus: dropped"),
            ],
        ),
        scen(
            "dropped-blocker-silent",
            Rule::DroppedBlocker,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nblocked_by: [A-2]",
                ),
                tf(
                    "b",
                    "id: A-2\ntitle: A-2\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234",
                ),
            ],
        ),
    ]
}

/// `APLAN021` and `APLAN022`: milestone hygiene and membership totality.
fn milestone_rules() -> Vec<Scenario> {
    vec![
        // APLAN021, a task tagged `milestone` must be shaped like one.
        scen(
            "milestone-hygiene-fires",
            Rule::MilestoneHygiene,
            Fires,
            // Tagged `milestone` but with no members: the cut scopes no work.
            vec![tf(
                "m",
                "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\ntags: [milestone]",
            )],
        ),
        scen(
            "milestone-hygiene-silent",
            Rule::MilestoneHygiene,
            Silent,
            // A well-formed milestone: a member, no effort, no paths.
            vec![
                tf(
                    "m",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\ntags: [milestone]\nblocked_by: [A-1]",
                ),
                tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
            ],
        ),
        // APLAN022, every non-milestone task belongs to exactly one cut.
        scen(
            "membership-totality-fires",
            Rule::MembershipTotality,
            Fires,
            // A cut (M-1) with member A-1, and B-1 which belongs to no cut.
            vec![
                tf(
                    "m",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\ntags: [milestone, v0.1]\nblocked_by: [A-1]",
                ),
                tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
                tf("b", "id: B-1\ntitle: B-1\nphase: 0\nstatus: todo"),
            ],
        )
        .cut_membership(),
        scen(
            "membership-totality-silent",
            Rule::MembershipTotality,
            Silent,
            // Every non-milestone task is a member of exactly one cut.
            vec![
                tf(
                    "m",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\ntags: [milestone, v0.1]\nblocked_by: [A-1]",
                ),
                tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
            ],
        )
        .cut_membership(),
    ]
}

/// Materialise a scenario to a temp directory and return the `(code, severity)`
/// of every diagnostic the validator reports over it.
fn diagnostics_for(s: &Scenario) -> Vec<(&'static str, Severity)> {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, contents) in &s.files {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, contents).expect("write fixture");
    }
    let opts = Options {
        require_effort: s.require_effort,
        traces_root: s.traces_root.then(|| dir.path().to_path_buf()),
        require_min_strength: s
            .require_strong
            .then(|| vec![EvidenceKind::Commit, EvidenceKind::Test, EvidenceKind::File]),
        detect_drift: s.detect_drift,
        require_cut_membership: s.require_cut_membership,
        ..Default::default()
    };
    let report = validate(&load_plan(dir.path()), &opts);
    report
        .diagnostics
        .iter()
        .map(|d| (d.code, d.severity))
        .collect()
}

/// The e2e layer: every scenario behaves as declared. A *fires* case must
/// surface its rule; a *silent* case must not. A guard also catches a fixture
/// that simply failed to load (an unexpected `APLAN001`), so a typo in the
/// corpus cannot masquerade as a passing *silent* case.
#[test]
fn scenario_corpus_behaves_as_declared() {
    let mut failures = Vec::new();
    for s in corpus() {
        let diags = diagnostics_for(&s);
        let codes: Vec<&str> = diags.iter().map(|(c, _)| *c).collect();
        let present = codes.contains(&s.rule.code());

        if s.rule != Rule::Schema && codes.contains(&Rule::Schema.code()) {
            failures.push(format!(
                "{}: fixture failed to load (unexpected APLAN001); codes = {codes:?}",
                s.name
            ));
            continue;
        }

        let ok = match s.expect {
            Expect::Fires => present,
            Expect::Silent => !present,
        };
        if !ok {
            failures.push(format!(
                "{}: expected {} to be {:?}, but codes = {codes:?}",
                s.name,
                s.rule.code(),
                s.expect
            ));
        }

        // Severity agreement: a rule must fire at the severity it *declares*
        // (`Rule::severity()`), so `facts.json` and the docs read a value the
        // firing site actually honours.
        if s.expect == Fires && present {
            let fired = diags
                .iter()
                .find(|(c, _)| *c == s.rule.code())
                .map(|(_, sev)| *sev);
            if fired != Some(s.rule.severity()) {
                failures.push(format!(
                    "{}: {} fired at {:?}, but Rule::severity() declares {:?}",
                    s.name,
                    s.rule.code(),
                    fired,
                    s.rule.severity()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "scenario expectations failed:\n  {}",
        failures.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// The verify corpus. These rules only fire under `aplan verify`, which opens a
// real git repository, so, unlike the check corpus, a scenario cannot be pure
// static data: the commit SHA cited as evidence is only known after committing.
// Each case therefore builds its own repo and returns the tasks to resolve.
// ---------------------------------------------------------------------------

struct VerifyCase {
    name: &'static str,
    rule: Rule,
    expect: Expect,
    /// Populate the freshly-initialised repo and return the tasks to verify,
    /// their evidence citing the SHAs it just committed.
    build: fn(&Repository) -> Vec<Task>,
    /// The trusted `test` runner, for the `test` rules. `None` for git rules.
    runner: Option<TestRunner>,
    /// Run `test` evidence (rung L3). Only meaningful with a runner.
    execute: bool,
}

/// A git-backed case (commit/file rules); no runner, no execution.
fn git_case(
    name: &'static str,
    rule: Rule,
    expect: Expect,
    build: fn(&Repository) -> Vec<Task>,
) -> VerifyCase {
    VerifyCase {
        name,
        rule,
        expect,
        build,
        runner: None,
        execute: false,
    }
}

/// A `test`-rule case: carries a runner, and may execute (rung L3).
fn test_case(
    name: &'static str,
    rule: Rule,
    expect: Expect,
    build: fn(&Repository) -> Vec<Task>,
    runner: TestRunner,
    execute: bool,
) -> VerifyCase {
    VerifyCase {
        name,
        rule,
        expect,
        build,
        runner: Some(runner),
        execute,
    }
}

fn verify_corpus() -> Vec<VerifyCase> {
    vec![
        // APLAN011, the commit SHA resolves.
        git_case(
            "commit-unresolved-fires",
            Rule::CommitUnresolved,
            Fires,
            v011_fires,
        ),
        git_case(
            "commit-unresolved-silent",
            Rule::CommitUnresolved,
            Silent,
            v011_silent,
        ),
        // APLAN012, the commit touches one of the task's paths.
        git_case(
            "commit-irrelevant-fires",
            Rule::CommitIrrelevant,
            Fires,
            v012_fires,
        ),
        git_case(
            "commit-irrelevant-silent",
            Rule::CommitIrrelevant,
            Silent,
            v012_silent,
        ),
        // APLAN015, the file exists at the cited rev (here, HEAD).
        git_case(
            "file-unresolved-fires",
            Rule::FileUnresolved,
            Fires,
            v015_fires,
        ),
        git_case(
            "file-unresolved-silent",
            Rule::FileUnresolved,
            Silent,
            v015_silent,
        ),
        // APLAN013, the runner lists a test with that name.
        test_case(
            "test-unresolved-fires",
            Rule::TestUnresolved,
            Fires,
            v_test_ghost,
            list_runner("known", true),
            false,
        ),
        test_case(
            "test-unresolved-silent",
            Rule::TestUnresolved,
            Silent,
            v_test_known,
            list_runner("known", true),
            false,
        ),
        // APLAN014, under --execute the test passes.
        test_case(
            "test-failed-fires",
            Rule::TestFailed,
            Fires,
            v_test_known,
            list_runner("known", false),
            true,
        ),
        test_case(
            "test-failed-silent",
            Rule::TestFailed,
            Silent,
            v_test_known,
            list_runner("known", true),
            true,
        ),
    ]
}

/// A runner that lists exactly `name` and whose `run` passes (`true`) or fails
/// (`false`). Uses the `{test}` placeholder so the name is a standalone arg.
fn list_runner(name: &str, passes: bool) -> TestRunner {
    let run_program = if passes { "true" } else { "false" };
    TestRunner {
        list: vec!["printf".into(), format!("{name}\n")],
        run: vec![run_program.into(), PLACEHOLDER.into()],
    }
}

fn v011_fires(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done("A", vec![commit_ev("deadbeefdeadbeef")], Vec::new())]
}
fn v011_silent(repo: &Repository) -> Vec<Task> {
    let sha = git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done("A", vec![commit_ev(&sha)], Vec::new())]
}
fn v012_fires(repo: &Repository) -> Vec<Task> {
    let sha = git_commit(repo, &[("docs/x.md", "prose")], "docs only");
    vec![done("A", vec![commit_ev(&sha)], vec!["src/**".into()])]
}
fn v012_silent(repo: &Repository) -> Vec<Task> {
    let sha = git_commit(repo, &[("src/a.rs", "x")], "src");
    vec![done("A", vec![commit_ev(&sha)], vec!["src/**".into()])]
}
fn v015_fires(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done("A", vec![file_ev("src/nope.rs")], Vec::new())]
}
fn v015_silent(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done("A", vec![file_ev("src/a.rs")], Vec::new())]
}
// `test` rules do not touch git; the repo just has to exist for `verify`.
fn v_test_ghost(_repo: &Repository) -> Vec<Task> {
    vec![done("A", vec![test_ev("ghost")], Vec::new())]
}
fn v_test_known(_repo: &Repository) -> Vec<Task> {
    vec![done("A", vec![test_ev("known")], Vec::new())]
}

/// Write `files` into the repo work tree and commit them, returning the SHA.
fn git_commit(repo: &Repository, files: &[(&str, &str)], msg: &str) -> String {
    let workdir = repo.workdir().expect("non-bare repo").to_path_buf();
    for (p, contents) in files {
        let full = workdir.join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
        .unwrap()
        .to_string()
}

fn done(id: &str, evidence: Vec<Evidence>, paths: Vec<String>) -> Task {
    Task {
        id: id.to_string(),
        title: id.to_string(),
        phase: 0,
        status: Status::Done,
        owner: None,
        effort_days: None,
        blocked_by: Vec::new(),
        evidence,
        paths,
        traces: Vec::new(),
        trace_fingerprints: std::collections::BTreeMap::new(),
        tags: Vec::new(),
    }
}

fn commit_ev(sha: &str) -> Evidence {
    Evidence {
        kind: EvidenceKind::Commit,
        reference: sha.to_string(),
        captured: None,
    }
}
fn file_ev(reference: &str) -> Evidence {
    Evidence {
        kind: EvidenceKind::File,
        reference: reference.to_string(),
        captured: None,
    }
}
fn test_ev(name: &str) -> Evidence {
    Evidence {
        kind: EvidenceKind::Test,
        reference: name.to_string(),
        captured: None,
    }
}

/// Materialise a verify case's repo and return the rule codes `verify` reports.
fn verify_diagnostics(case: &VerifyCase) -> Vec<(&'static str, Severity)> {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = Repository::init(dir.path()).expect("git init");
    let tasks = (case.build)(&repo);
    let plan = Plan {
        tasks: tasks
            .into_iter()
            .map(|task| LoadedTask {
                task,
                path: "plan/tasks/x.md".into(),
            })
            .collect(),
        load_diagnostics: Vec::new(),
    };
    let opts = aplan_evidence::VerifyOptions {
        execute: case.execute,
        test_runner: case.runner.clone(),
    };
    let report = aplan_evidence::verify(&plan, dir.path(), &opts).expect("verify runs");
    report
        .diagnostics
        .iter()
        .map(|d| (d.code, d.severity))
        .collect()
}

/// The verify e2e layer: each git-backed scenario behaves as declared.
#[test]
fn verify_corpus_behaves_as_declared() {
    let mut failures = Vec::new();
    for case in verify_corpus() {
        let diags = verify_diagnostics(&case);
        let codes: Vec<&str> = diags.iter().map(|(c, _)| *c).collect();
        let present = codes.contains(&case.rule.code());
        let ok = match case.expect {
            Expect::Fires => present,
            Expect::Silent => !present,
        };
        if !ok {
            failures.push(format!(
                "{}: expected {} to be {:?}, but codes = {codes:?}",
                case.name,
                case.rule.code(),
                case.expect
            ));
        }

        // Severity agreement, as for the check corpus: a verify rule must fire at
        // the severity it declares, so `facts.json` cannot lie about APLAN011-015.
        if case.expect == Fires && present {
            let fired = diags
                .iter()
                .find(|(c, _)| *c == case.rule.code())
                .map(|(_, sev)| *sev);
            if fired != Some(case.rule.severity()) {
                failures.push(format!(
                    "{}: {} fired at {:?}, but Rule::severity() declares {:?}",
                    case.name,
                    case.rule.code(),
                    fired,
                    case.rule.severity()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "verify scenario expectations failed:\n  {}",
        failures.join("\n  ")
    );
}

/// The load-bearing audit. Every rule in [`Rule::ALL`] must have both a *fires*
/// and a *silent* scenario, in either the hermetic `check` corpus or the
/// git-backed `verify` corpus. Adding a rule to the registry without covering it
/// fails the build with the exact list of what is missing.
#[test]
fn every_rule_has_a_fires_and_a_silent_scenario() {
    let mut covered: Vec<(Rule, Expect)> = Vec::new();
    covered.extend(corpus().iter().map(|s| (s.rule, s.expect)));
    covered.extend(verify_corpus().iter().map(|c| (c.rule, c.expect)));

    let mut missing = Vec::new();
    for &rule in Rule::ALL {
        for expect in [Expect::Fires, Expect::Silent] {
            if !covered.contains(&(rule, expect)) {
                let which = match expect {
                    Expect::Fires => "FIRES",
                    Expect::Silent => "SILENT",
                };
                missing.push(format!(
                    "{} ({}), no {which} scenario",
                    rule.code(),
                    rule.title()
                ));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "coverage drift, every rule needs a fires *and* a silent scenario. \
         Add the missing case(s) to `corpus()` (hermetic) or `verify_corpus()` (git) here:\n  {}",
        missing.join("\n  ")
    );
}

/// The audit generalises past rules: every output [`Format`] must be exercised.
/// The `exercised` list is what a human maintains; the audit asserts it equals
/// [`Format::ALL`], so adding a variant without a test fails here. The
/// byte-level output of each format is pinned by the CLI corpus in
/// `crates/aplan/tests/cmd/` (`violations.trycmd` renders all four).
#[test]
fn every_format_is_exercised() {
    let exercised = [
        Format::Human,
        Format::Json,
        Format::Agent,
        Format::Github,
        Format::Sarif,
    ];
    for f in Format::ALL {
        assert!(
            exercised.contains(f),
            "format {f:?} is in Format::ALL but not exercised, add it here and give it a case",
        );
    }
    assert_eq!(
        exercised.len(),
        Format::ALL.len(),
        "the exercised list and Format::ALL disagree on the number of formats",
    );
}

// Deferred audit dimensions (ROADMAP §5), tracked openly rather than left as
// silent gaps:
//   - every CLI flag / exit code -> a trycmd case. The trycmd corpus already
//     covers the current surface; the *forcing* audit needs an enumerable model
//     of clap's flags, which lands with the config work (v0.4).
//   - every schema field -> a dispatch entry. There is no evidence-dispatch
//     table to audit against until the evidence ladder (v0.3).
