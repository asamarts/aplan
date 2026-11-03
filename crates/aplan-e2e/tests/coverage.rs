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
//!
//! **If you changed *when* a rule fires, name the behaviour** (`A1-142`). A
//! scenario that moves from `Fires` to `Silent`, or a fixture that has to gain a
//! citation to keep firing, is a behavioural change to a shipped rule, and
//! `--require <code>` cannot tell the two builds apart. `A1-138` and `A1-141`
//! both moved `APLAN019`'s scope; the first shipped unnamed.
//!

use std::fmt::Write as _;

use aplan_config::{PLACEHOLDER, TestRunner, TestRunners};
use aplan_core::task::{Evidence, EvidenceKind, Status, Task};
use aplan_core::{Options, Plan, Rule, Severity, load::LoadedTask, load_plan, validate};
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
    /// Hold tasks to their level's version floor (enables `APLAN044`).
    require_impact: bool,
    /// Declare a tag vocabulary (enables `APLAN026`).
    tag_families: Option<aplan_core::TagFamily>,
    /// Declare canonical owners (enables `APLAN056`).
    owners: &'static [&'static str],
    /// The generated block `export-agents-md` would write now, compared against
    /// the scenario's own `AGENTS.md` (enables `APLAN041`). `None` is a caller
    /// that cannot render one, which switches the rule off.
    agents_block: Option<&'static str>,
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
    /// The documented shape: members are whole tags carrying the family's name.
    fn tag_families(self) -> Self {
        self.tag_families_of(&["epic-a", "epic-b"])
    }

    /// A family declared with members of the author's choosing, which is how the
    /// fourth pilot's mis-declaration becomes a fixture (`A1-085`).
    fn tag_families_of(mut self, members: &[&'static str]) -> Self {
        self.tag_families = Some(aplan_core::TagFamily::members(
            members.iter().map(|m| (*m).to_string()).collect(),
        ));
        self
    }

    /// A family carrying a cardinality, for `APLAN030` (`A1-133`).
    fn tag_family(mut self, family: aplan_core::TagFamily) -> Self {
        self.tag_families = Some(family);
        self
    }

    fn owners(mut self, owners: &'static [&'static str]) -> Self {
        self.owners = owners;
        self
    }

    /// What the contract would say now, for `APLAN041` (`A1-273`). The file it
    /// is compared against is an ordinary scenario file at `AGENTS.md`.
    fn agents_block(mut self, current: &'static str) -> Self {
        self.agents_block = Some(current);
        self
    }
}

fn scen(name: &'static str, rule: Rule, expect: Expect, files: Vec<(String, String)>) -> Scenario {
    Scenario {
        name,
        rule,
        expect,
        files,
        require_impact: false,
        traces_root: false,
        require_effort: false,
        require_strong: false,
        detect_drift: false,
        require_cut_membership: false,
        tag_families: None,
        owners: &[],
        agents_block: None,
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

/// One canonical inbox record, for `APLAN057`.
fn cf(id: &str, body: &str) -> (String, String) {
    (format!(".inbox/{id}.yml"), body.to_string())
}

/// An `AGENTS.md` carrying a generated block, for `APLAN041` (`A1-273`).
///
/// Prose either side of the markers, because that is the shape the block is
/// written into: a repository keeps its own guidance around the fenced part.
fn agents_md(block: &str) -> (String, String) {
    (
        "AGENTS.md".to_string(),
        format!(
            "# A repository\n\nits own guidance\n\n{}\n{block}\n{}\n\nmore guidance\n",
            aplan_core::contract::BEGIN,
            aplan_core::contract::END
        ),
    )
}

/// A task file whose *body* matters, which `APLAN038` is the only rule to read
/// (`A1-224`): it counts the acceptance criteria the body states and how many
/// are ticked.
fn tf_body(name: &str, frontmatter: &str, body: &str) -> (String, String) {
    (
        format!("tasks/{name}.md"),
        format!("---\n{}\n---\n\n{body}", frontmatter.trim()),
    )
}

/// A plain document at the temp root, for `traces` to resolve against.
fn doc(name: &str, contents: &str) -> (String, String) {
    (name.to_string(), contents.to_string())
}

/// The corpus: a fires/silent pair per rule. Split into groups only to keep each
/// function readable; the audit sees them concatenated. One pair per rule keeps
/// the [`Rule::ALL`] correspondence easy to eyeball.
/// `APLAN038`, the one rule that reads the task's **body** (`A1-224`).
///
/// Its own group because that is what makes it different from every other rule
/// in this corpus: the rest are answered by frontmatter, and this one counts
/// the checkboxes under the heading an author wrote for themselves. `tf_body`
/// exists for exactly these two.
fn body_criteria_rules() -> Vec<Scenario> {
    vec![
        // APLAN038, `done` over criteria the body states and nobody answered.
        // The evidence is there; what is missing is the author's own answer.
        scen(
            "criteria-unanswered-fires",
            Rule::CriteriaUnanswered,
            Fires,
            vec![tf_body(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234",
                "## Acceptance criteria\n\n- [x] the first thing\n- [ ] the second thing\n",
            )],
        ),
        // APLAN040, one `paths` entry written twice.
        scen(
            "duplicate-path-fires",
            Rule::DuplicatePath,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 paths: [\"src/**\", \"src/**\"]",
            )],
        ),
        // The same two entries naming two trees, which is the ordinary case.
        scen(
            "duplicate-path-silent",
            Rule::DuplicatePath,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 paths: [\"src/**\", \"ci/**\"]",
            )],
        ),
        // APLAN042, a revisit instant nothing can act on, and one on a task
        // that is not parked. Two ways one property fails (`A1-264`).
        scen(
            "revisit-unparseable-fires",
            Rule::RevisitUnusable,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: blocked\neffort_days: 1\n\
                 revisit_at: next tuesday",
            )],
        ),
        scen(
            "revisit-not-parked-fires",
            Rule::RevisitUnusable,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 revisit_at: 2026-09-10T05:00:00Z",
            )],
        ),
        // A real instant, on a task that is parked: the shape the field exists
        // for.
        scen(
            "revisit-usable-silent",
            Rule::RevisitUnusable,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: blocked\neffort_days: 1\n\
                 revisit_at: 2026-09-10T05:00:00Z",
            )],
        ),
        // APLAN041, the generated `AGENTS.md` block gone stale. `p00002`'s
        // case: the block named `commit` as acceptable evidence for a day after
        // `require_min_strength` had dropped it (`A1-273`).
        scen(
            "stale-agent-contract-fires",
            Rule::StaleAgentContract,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1",
                ),
                agents_md("- A `done` task must cite `commit`, `file` or `test` evidence."),
            ],
        )
        .agents_block("- A `done` task must cite `file` or `test` evidence."),
        // The same file, saying what the policy says.
        scen(
            "stale-agent-contract-silent",
            Rule::StaleAgentContract,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1",
                ),
                agents_md("- A `done` task must cite `file` or `test` evidence."),
            ],
        )
        .agents_block("- A `done` task must cite `file` or `test` evidence."),
        // Half a marker pair: the quietest edit that would switch a
        // marker-scoped rule off, so it is a finding with its own sentence.
        scen(
            "stale-agent-contract-half-block-fires",
            Rule::StaleAgentContract,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1",
                ),
                (
                    "AGENTS.md".to_string(),
                    format!(
                        "# A repository\n\nits own guidance\n\n{}\n- a rule\n\nmore guidance\n",
                        aplan_core::contract::BEGIN
                    ),
                ),
            ],
        )
        .agents_block("- a rule"),
        // And the repository that never asked: prose, no markers, so nothing
        // here has standing to call it stale.
        scen(
            "stale-agent-contract-unmarked-silent",
            Rule::StaleAgentContract,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1",
                ),
                (
                    "AGENTS.md".to_string(),
                    "# A repository\n\nhand-written guidance, and no generated block.\n"
                        .to_string(),
                ),
            ],
        )
        .agents_block("- A `done` task must cite `file` or `test` evidence."),
        // APLAN039, the scaffold `add` wrote, ticked as though answered.
        scen(
            "scaffold-ticked-fires",
            Rule::ScaffoldTicked,
            Fires,
            vec![tf_body(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234",
                "## Acceptance criteria\n\n- [x] state what done means for this task, in this repository's terms\n",
            )],
        ),
        // A real criterion in its place, which is what the prompt is asking for.
        scen(
            "scaffold-ticked-silent",
            Rule::ScaffoldTicked,
            Silent,
            vec![tf_body(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234",
                "## Acceptance criteria\n\n- [x] the renderer emits SVG\n",
            )],
        ),
        // APLAN044, a closed task claiming less than its level implies with no
        // reason. The level floors a unit of work at `fix`; this one says it
        // changed nothing that ships and does not say why.
        Scenario {
            require_impact: true,
            ..scen(
                "version-impact-unexplained-fires",
                Rule::VersionImpactUnexplained,
                Fires,
                vec![tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nversion_impact: none\nevidence:\n  - kind: commit\n    ref: abc1234",
                )],
            )
        },
        // Silent with the reason given: overriding the floor downwards is
        // legitimate -- an operational record can change nothing that ships --
        // and the rule is about doing it in silence.
        Scenario {
            require_impact: true,
            ..scen(
                "version-impact-explained-is-silent",
                Rule::VersionImpactUnexplained,
                Silent,
                vec![tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nversion_impact: none\nversion_impact_reason: an operational record, which ships nothing\nevidence:\n  - kind: commit\n    ref: abc1234",
                )],
            )
        },
        // And silent when nothing is declared at all, which is the ordinary
        // task: the level supplies the floor and the plan still knows what it
        // owes. This is what makes the rule adoptable rather than a demand that
        // three hundred tasks be annotated.
        Scenario {
            require_impact: true,
            ..scen(
                "version-impact-undeclared-is-silent",
                Rule::VersionImpactUnexplained,
                Silent,
                vec![tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234",
                )],
            )
        },
        // APLAN043, the scaffold left beside the answer. The obvious hand-edit --
        // append your own `## Acceptance criteria` to a scaffolded file -- leaves
        // two of them and the prompt under the first.
        scen(
            "scaffold-leftover-fires",
            Rule::ScaffoldLeftover,
            Fires,
            vec![tf_body(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1",
                "## Acceptance criteria\n\n- [ ] the renderer emits SVG\n\n                 ## Acceptance criteria\n\n                 - [ ] state what done means for this task, in this repository's terms\n",
            )],
        ),
        // Ticked, on a live task: `APLAN039` is closed-tasks-only, so without this
        // the scaffold survives an answer and a tick reports it to nobody.
        scen(
            "scaffold-leftover-ticked-fires",
            Rule::ScaffoldLeftover,
            Fires,
            vec![tf_body(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: in_progress\neffort_days: 1",
                "## Acceptance criteria\n\n- [ ] the renderer emits SVG\n\n                 ## Acceptance criteria\n\n                 - [x] state what done means for this task, in this repository's terms\n",
            )],
        ),
        // The prompt on its own is what `add` wrote and nobody has answered yet,
        // which is between the author and `APLAN038` at close. This rule is about
        // the scaffold *surviving* an answer.
        scen(
            "scaffold-leftover-silent",
            Rule::ScaffoldLeftover,
            Silent,
            vec![tf_body(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1",
                "## Acceptance criteria\n\n                 - [ ] state what done means for this task, in this repository's terms\n",
            )],
        ),
        // Ticked, which is what closing a task is supposed to look like.
        scen(
            "criteria-unanswered-silent",
            Rule::CriteriaUnanswered,
            Silent,
            vec![tf_body(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: abc1234",
                "## Acceptance criteria\n\n- [x] the first thing\n- [x] the second thing\n",
            )],
        ),
    ]
}

fn corpus() -> Vec<Scenario> {
    let mut all = loader_and_identity_rules();
    all.extend(graph_edge_rules());
    all.extend(evidence_and_trace_rules());
    all.extend(ratification_rules());
    all.extend(structured_trace_rules());
    all.extend(effort_note_and_deadlock_rules());
    all.extend(external_trace_rules());
    all.extend(emptied_cut_rules());
    all.extend(milestone_rules());
    all.extend(cancelled_cut_rules());
    all.extend(tag_family_rules());
    all.extend(owner_vocabulary_rules());
    all.extend(drift_rules());
    all.extend(interpolated_reason_branches());
    all.extend(partial_stamp_rules());
    all.extend(body_criteria_rules());
    all.extend(reserved_tag_rules());
    all
}

/// The *second* sentence of rules that interpolate their reason (`A1-094`).
///
/// The corpus supplies one scenario per rule, so a rule saying several different
/// things had only the branch some scenario happened to reach recorded in
/// `docs/rule-catalogue.md`. Three separate audits met this one branch at a time
/// before anyone swept for it. The unpinned branch is by construction the
/// less-travelled one, which is where a wrong sentence survives longest.
fn interpolated_reason_branches() -> Vec<Scenario> {
    vec![
        // APLAN010 said "requires a `captured` date"; this is the other half, a
        // date that is present and not a date.
        scen(
            "undated-note-malformed-date-fires",
            Rule::UndatedNote,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
                 evidence:\n  - kind: note\n    ref: it is done\n    captured: someday",
            )],
        ),
        // APLAN001 said only "invalid YAML frontmatter". An unquoted `: ` is the
        // commonest way to write invalid frontmatter by accident, and it earns a
        // sentence naming the field (`A1-072`).
        scen(
            "schema-unquoted-colon-fires",
            Rule::Schema,
            Fires,
            vec![tf(
                "bad",
                "id: A-1\ntitle: ship it: finally\nphase: 0\nstatus: todo",
            )],
        ),
        // APLAN020 and APLAN007 each say one thing about a *declared* blocker and
        // another about a cut this task's cut follows (ADR-0013). Only the
        // declared sentence was pinned, and an adopter on a sequenced plan meets
        // the other one.
        scen(
            "dropped-blocker-inherited-fires",
            Rule::DroppedBlocker,
            Fires,
            vec![
                tf(
                    "m0",
                    "id: M-s0\ntitle: M-s0\nphase: 0\nstatus: dropped\n\
                     tags: [milestone, s0]\nblocked_by: [A-1]",
                ),
                tf("a1", "id: A-1\ntitle: A-1\nphase: 0\nstatus: dropped"),
                tf(
                    "m1",
                    "id: M-s1\ntitle: M-s1\nphase: 1\nstatus: todo\n\
                     tags: [milestone, s1]\nblocked_by: [B-1, M-s0]",
                ),
                tf("b1", "id: B-1\ntitle: B-1\nphase: 1\nstatus: todo"),
            ],
        ),
        // APLAN021's third reason, the one the catalogue used to record as
        // deliberately absent. It joins its reasons, so this is a distinct
        // sentence and a milestone declaring `paths` is an easy mistake: the
        // field is legal on every other task.
        scen(
            "milestone-hygiene-declares-paths-fires",
            Rule::MilestoneHygiene,
            Fires,
            vec![
                tf(
                    "m",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                     blocked_by: [A-1]\npaths:\n  - src/**",
                ),
                tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
            ],
        ),
        scen(
            "done-before-blocker-inherited-fires",
            Rule::DoneBeforeBlocker,
            Fires,
            vec![
                tf(
                    "m0",
                    "id: M-s0\ntitle: M-s0\nphase: 0\nstatus: todo\n\
                     tags: [milestone, s0]\nblocked_by: [A-1]",
                ),
                tf("a1", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
                tf(
                    "m1",
                    "id: M-s1\ntitle: M-s1\nphase: 1\nstatus: todo\n\
                     tags: [milestone, s1]\nblocked_by: [B-1, M-s0]",
                ),
                tf(
                    "b1",
                    "id: B-1\ntitle: B-1\nphase: 1\nstatus: done\n\
                     evidence:\n  - kind: commit\n    ref: abc1234",
                ),
            ],
        ),
    ]
}

/// `APLAN028`: a closed task stamps every trace that can be stamped.
fn partial_stamp_rules() -> Vec<Scenario> {
    let doc_body = "# Intro\n\n| 7 | a |\n| 11 | b |\n";
    let good = aplan_core::anchor_fingerprint(doc_body, "intro").expect("fingerprint");
    vec![
        // The migration state: retargeted from the section to a row, so the old
        // key is orphaned and the new trace is unstamped. Before `A1-098` this
        // plan was clean and drift was off for the task.
        scen(
            "partial-trace-stamp-fires",
            Rule::UnstampedTrace,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
                     evidence:\n  - kind: commit\n    ref: abc1234\n\
                     traces:\n  - DOC.md#7\n\
                     trace_fingerprints:\n  DOC.md#intro: '0000000000000000'",
                ),
                doc("DOC.md", doc_body),
            ],
        )
        .drift(),
        // Two unstamped: a second sentence, since the message reads "them" and
        // "these sections". `A1-094`'s lesson, applied while the branch is being
        // written rather than by a later audit.
        scen(
            "partial-trace-stamp-several-fires",
            Rule::UnstampedTrace,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
                     evidence:\n  - kind: commit\n    ref: abc1234\n\
                     traces:\n  - DOC.md#7\n  - DOC.md#11\n\
                     trace_fingerprints:\n  DOC.md#intro: '0000000000000000'",
                ),
                doc("DOC.md", doc_body),
            ],
        )
        .drift(),
        // Stamped what it traces: silent. The near-miss that matters, since a
        // rule firing here would make `close` unusable.
        scen(
            "partial-trace-stamp-silent",
            Rule::UnstampedTrace,
            Silent,
            vec![
                tf(
                    "a",
                    &format!(
                        "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
                         evidence:\n  - kind: commit\n    ref: abc1234\n\
                         traces:\n  - DOC.md#intro\n\
                         trace_fingerprints:\n  DOC.md#intro: '{good}'"
                    ),
                ),
                doc("DOC.md", doc_body),
            ],
        )
        .drift(),
        // Stamps nothing at all, which used to be exempt: the same sentence
        // about every trace instead of about one. Measured at nine live cases
        // against zero for the partial shape above (`A1-297`).
        scen(
            "no-trace-stamp-fires",
            Rule::UnstampedTrace,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
                     evidence:\n  - kind: commit\n    ref: abc1234\n\
                     traces:\n  - DOC.md#intro",
                ),
                doc("DOC.md", doc_body),
            ],
        )
        .drift(),
        // A trace with no anchor, which `close --restamp` cannot stamp: silent,
        // because the fix this rule names provably would not clear it. The near
        // miss that keeps the removal of the exemption honest (`A1-297`).
        scen(
            "unstampable-trace-silent",
            Rule::UnstampedTrace,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
                     evidence:\n  - kind: commit\n    ref: abc1234\n\
                     traces:\n  - DOC.md",
                ),
                doc("DOC.md", doc_body),
            ],
        )
        .drift(),
    ]
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
        // The same rule over a structured document, which says `node` rather
        // than `section` (`A1-191`). Its own scenario because a rule with two
        // wordings and one scenario leaves the less-travelled one unpinned,
        // which is what `A1-088`'s audit found.
        scen(
            "trace-drift-node-fires",
            Rule::TraceDrift,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
                     evidence:\n  - kind: commit\n    ref: abc1234\n\
                     traces:\n  - spec.yaml#/states/provisioning\n\
                     trace_fingerprints:\n  spec.yaml#/states/provisioning: '0000000000000000'",
                ),
                doc(
                    "spec.yaml",
                    "states:\n  provisioning:\n    timeout_s: 300\n",
                ),
            ],
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
        // APLAN057, captures have a separate strict V1 loader and never rely on
        // the task-frontmatter rule for visibility.
        scen(
            "capture-invalid-fires",
            Rule::CaptureInvalid,
            Fires,
            vec![cf(
                "I-01K8Z3A4H00000000000000000",
                "schema: aplan/capture/v1\nid: [unterminated\n",
            )],
        ),
        scen(
            "capture-invalid-silent",
            Rule::CaptureInvalid,
            Silent,
            vec![cf(
                "I-01K8Z3A4H00000000000000000",
                "schema: aplan/capture/v1\nid: I-01K8Z3A4H00000000000000000\ntitle: valid intake\nkind: task\nstate: open\ncaptured_at: '2026-01-01T00:00:00Z'\norigin: cli\n",
            )],
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

/// `APLAN008` over a structured document (`A1-191`, ADR-0035).
///
/// Its own group for the reason the others have one: the function above is at
/// the line limit, and these are a coherent set rather than an overflow.
fn structured_trace_rules() -> Vec<Scenario> {
    vec![
        // The three structured-document branches (`A1-191`, ADR-0035). Each is a
        // different sentence because each has a different remedy, which is the
        // whole reason they are told apart: fix the document, fix the anchor's
        // shape, or fix which node it names.
        scen(
            "structured-anchor-addresses-nothing-fires",
            Rule::DanglingTrace,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - spec.yaml#/states/provisionng",
                ),
                doc("spec.yaml", "states:\n  provisioning:\n    timeout_s: 300\n"),
            ],
        )
        .traces(),
        scen(
            "structured-anchor-is-not-an-address-fires",
            Rule::DanglingTrace,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - spec.yaml#$.states[*]",
                ),
                doc("spec.yaml", "states:\n  provisioning:\n    timeout_s: 300\n"),
            ],
        )
        .traces(),
        scen(
            "structured-document-is-unaddressable-fires",
            Rule::DanglingTrace,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - spec.yaml#/states",
                ),
                doc("spec.yaml", "states: [unclosed\n"),
            ],
        )
        .traces(),
        // And the pair's silent half in both spellings, since a resolver that
        // refused everything would pass the three above on its own.
        scen(
            "structured-anchor-silent",
            Rule::DanglingTrace,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - spec.yaml#/states/provisioning\n  - api.json#$.paths['/users'].get",
                ),
                doc("spec.yaml", "states:\n  provisioning:\n    timeout_s: 300\n"),
                doc("api.json", "{\"paths\": {\"/users\": {\"get\": {\"summary\": \"list\"}}}}"),
            ],
        )
        .traces(),
    ]
}

/// `APLAN051` and `APLAN052`: the five words that change what this tool does
/// (`A1-212`, ADR-0014).
///
/// Its own group for the reason the one below gives, and because the pair belongs
/// together: one is about a word spelled wrongly and the other about a word used
/// where it cannot mean what the rule reads it as.
fn reserved_tag_rules() -> Vec<Scenario> {
    vec![
        // APLAN051, a tag one edit from a word that changes what this tool does
        // (`A1-212`). `milestne` does not misname a cut -- it dissolves one, and
        // `check` then advises an estimate on what it no longer knows is a
        // checkpoint.
        scen(
            "reserved-tag-near-miss-fires",
            Rule::ReservedTagNearMiss,
            Fires,
            vec![
                tf(
                    "a",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\n\
                 tags:\n  - milestne\nblocked_by:\n  - W-1",
                ),
                tf(
                    "b",
                    "id: W-1\ntitle: W-1\nphase: 0\nstatus: todo\neffort_days: 1",
                ),
            ],
        ),
        // The shape it must not touch: the word spelled correctly, and an
        // ordinary tag that is nowhere near one of the five.
        scen(
            "reserved-tag-near-miss-silent",
            Rule::ReservedTagNearMiss,
            Silent,
            vec![
                tf(
                    "a",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\n\
                 tags:\n  - milestone\n  - backend\nblocked_by:\n  - W-1",
                ),
                tf(
                    "b",
                    "id: W-1\ntitle: W-1\nphase: 0\nstatus: todo\neffort_days: 1",
                ),
            ],
        ),
        // APLAN052, `cross-cutting` on a task that is in a cut (`A1-212`). The
        // tag cannot mean "belongs to no cut" here, and it switches `APLAN022`
        // off for a task that satisfies it anyway.
        scen(
            "cross-cutting-in-a-cut-fires",
            Rule::CrossCuttingInACut,
            Fires,
            vec![
                tf(
                    "a",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\n\
                 tags:\n  - milestone\nblocked_by:\n  - W-1",
                ),
                tf(
                    "b",
                    "id: W-1\ntitle: W-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 tags:\n  - cross-cutting",
                ),
            ],
        ),
        // The shape it exists to allow, and the asymmetry `A1-212` turned on: on
        // a task in no cut the same tag is what `APLAN022` reads it as, and
        // "belongs to no cut" and "concerns several components" are
        // indistinguishable by looking. Silent.
        scen(
            "cross-cutting-in-a-cut-silent",
            Rule::CrossCuttingInACut,
            Silent,
            vec![tf(
                "a",
                "id: W-1\ntitle: W-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 tags:\n  - cross-cutting",
            )],
        ),
    ]
}

/// `APLAN046`: a ratification constrains counting done, not starting (`A1-306`,
/// ADR-0036).
///
/// Its own group so the function above stays under the line limit, which is the
/// reason every other group here exists.
fn ratification_rules() -> Vec<Scenario> {
    vec![
        // APLAN046, a ratification constrains counting done (`A1-306`, ADR-0036).
        scen(
            "unratified-fires",
            Rule::Unratified,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\n\
                     evidence:\n  - kind: commit\n    ref: abc1234\n\
                     ratified_by:\n  - A-2",
                ),
                tf("b", "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo"),
            ],
        ),
        // The near miss that is the whole point: the ratifier is unfinished and
        // the work is *not* blocked by it, so nothing should report the task as
        // unstartable. Only its `done` claim is refused.
        scen(
            "unratified-silent",
            Rule::Unratified,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\n\
                     ratified_by:\n  - A-2",
                ),
                tf("b", "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo"),
            ],
        ),
        // APLAN047, a `contains` that closes a loop (`A1-333`, ADR-0042). Two
        // checkpoints each claiming the other's work: the rollup has no answer,
        // because whichever is walked first swallows the other.
        scen(
            "contains-unusable-fires",
            Rule::ContainsUnusable,
            Fires,
            vec![
                tf(
                    "a",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\n\
                     tags:\n  - milestone\ncontains:\n  - M-2\nblocked_by:\n  - M-2",
                ),
                tf(
                    "b",
                    "id: M-2\ntitle: M-2\nphase: 0\nstatus: todo\n\
                     tags:\n  - milestone\ncontains:\n  - M-1\nblocked_by:\n  - W-1",
                ),
                tf(
                    "c",
                    "id: W-1\ntitle: W-1\nphase: 0\nstatus: todo\neffort_days: 1",
                ),
            ],
        ),
        // APLAN050, a recorded `version` on something that is not a release
        // (`A1-339`, ADR-0038). A cut is a planning bucket rather than a
        // delivery, and a version on one reads as a claim that it shipped.
        scen(
            "version-record-unusable-fires",
            Rule::VersionRecordUnusable,
            Fires,
            vec![tf(
                "a",
                "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\n\
                 tags:\n  - milestone\nversion: 1.2.3",
            )],
        ),
        // The shape it exists to allow: the same field on the node that really
        // does ship, carrying a version this tool can read back.
        scen(
            "version-record-unusable-silent",
            Rule::VersionRecordUnusable,
            Silent,
            vec![tf(
                "a",
                "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\n\
                 tags:\n  - milestone\n  - release\nversion: 1.2.3",
            )],
        ),
        // The shape this rule must not touch: a checkpoint containing a real
        // checkpoint, which is the whole point of ADR-0042 and the case a
        // stricter rule would have refused.
        scen(
            "contains-unusable-silent",
            Rule::ContainsUnusable,
            Silent,
            vec![
                tf(
                    "a",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\n\
                     tags:\n  - milestone\ncontains:\n  - M-2\nblocked_by:\n  - M-2",
                ),
                tf(
                    "b",
                    "id: M-2\ntitle: M-2\nphase: 0\nstatus: todo\n\
                     tags:\n  - milestone\nblocked_by:\n  - W-1",
                ),
                tf(
                    "c",
                    "id: W-1\ntitle: W-1\nphase: 0\nstatus: todo\neffort_days: 1",
                ),
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
        // The *other* half of APLAN008, and a different sentence: the document
        // exists and the heading does not. Added by the audit of `A1-088`, which
        // found the catalogue pinning one wording per rule while rules with two
        // branches left the less-travelled one unrecorded. This is the branch an
        // adopter meets after renaming a heading, which is the commoner mistake.
        scen(
            "dangling-trace-anchor-fires",
            Rule::DanglingTrace,
            Fires,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - DOC.md#renamed",
                ),
                doc("DOC.md", "# Intro\n\nprose\n"),
            ],
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
/// `APLAN021`'s widened test: a cut holding no live work (`A1-134`).
///
/// Its own group, like the one below, because the list these came from had
/// reached the workspace's function-length limit.
fn emptied_cut_rules() -> Vec<Scenario> {
    vec![
        // APLAN021's widened test: a cut emptied of work still lists its
        // predecessor, so the empty check was silent on it (`A1-134`).
        scen(
            "milestone-hygiene-emptied-fires",
            Rule::MilestoneHygiene,
            Fires,
            vec![
                tf(
                    "a",
                    "id: M-s0\ntitle: M-s0\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                     blocked_by: [A-1]",
                ),
                tf(
                    "b",
                    "id: M-s1\ntitle: M-s1\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                     blocked_by: [M-s0]",
                ),
                tf("c", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
            ],
        ),
        // A release is a checkpoint over cuts, and the same shape without the
        // tag is an emptied cut: the marker is the whole difference
        // (`A1-146`, ADR-0022).
        scen(
            "milestone-hygiene-release-silent",
            Rule::MilestoneHygiene,
            Silent,
            vec![
                tf(
                    "a",
                    "id: M-s0\ntitle: M-s0\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                     blocked_by: [A-1]",
                ),
                tf(
                    "b",
                    "id: R-v0.1\ntitle: R-v0.1\nphase: 0\nstatus: todo\n\
                     tags: [milestone, release, v0.1]\nblocked_by: [M-s0]",
                ),
                tf("c", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
            ],
        ),
        // A release holding work directly is the second grouping axis ADR-0014
        // refused, reported against the release rather than against the task.
        scen(
            "milestone-hygiene-release-holds-work-fires",
            Rule::MilestoneHygiene,
            Fires,
            vec![
                tf(
                    "a",
                    "id: R-v0.1\ntitle: R-v0.1\nphase: 0\nstatus: todo\n\
                     tags: [milestone, release, v0.1]\nblocked_by: [A-1]",
                ),
                tf("b", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
            ],
        ),
        // The other way a cut empties: its work was cancelled rather than moved.
        // `A1-081` exempts that from `APLAN020` on purpose, so this rule is the
        // only one that can say it -- and the wording only stays pinned while a
        // scenario reaches it, which it stopped doing when the audit exempted a
        // *cancelled* cut from the same check (`A1-134`).
        scen(
            "milestone-hygiene-all-dropped-fires",
            Rule::MilestoneHygiene,
            Fires,
            vec![
                tf(
                    "a",
                    "id: M-s0\ntitle: M-s0\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                     blocked_by: [A-1]",
                ),
                tf("b", "id: A-1\ntitle: A-1\nphase: 0\nstatus: dropped"),
            ],
        ),
        // The near-miss: the same sequenced pair with work in the later cut.
        scen(
            "milestone-hygiene-emptied-silent",
            Rule::MilestoneHygiene,
            Silent,
            vec![
                tf(
                    "a",
                    "id: M-s0\ntitle: M-s0\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                     blocked_by: [A-1]",
                ),
                tf(
                    "b",
                    "id: M-s1\ntitle: M-s1\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                     blocked_by: [M-s0, A-2]",
                ),
                tf("c", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
                tf("d", "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo"),
            ],
        ),
    ]
}

/// `APLAN034`, a `traces` target that leaves the repository undeclared
/// (`A1-143`).
///
/// Its own group rather than an eleventh entry in the list below, which the
/// workspace's own function-length lint had already grown to the limit of.
fn external_trace_rules() -> Vec<Scenario> {
    vec![
        // APLAN034, a `traces` target that leaves the repository undeclared. The
        // fifth pilot's only candidate fix: such a target resolves *fully*
        // wherever the sibling is checked out, so nothing said anything until a
        // job ran without it (`A1-143`, pre-public adoption evidence 4c).
        scen(
            "external-trace-fires",
            Rule::ExternalTrace,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - ../sibling/DOC.md#x",
            )],
        )
        .traces()
        .effort(),
        // The near-miss: the same shape of trace, inside the repository, spelled
        // with the `..` that makes it look like an escape and is not one.
        scen(
            "external-trace-silent",
            Rule::ExternalTrace,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntraces:\n  - tasks/../DOC.md#a-section",
                ),
                ("DOC.md".to_string(), "# Doc\n\n## A section\n\ntext\n".to_string()),
            ],
        )
        .traces()
        .effort(),
    ]
}

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
        // APLAN033, a `prefer_after` entry that cannot be honoured. The
        // discretionary order (`A1-150`, ADR-0025) is the one field a plan can
        // carry that changes nothing but the offer, so the rule that guards it
        // has to be about preferences nobody could act on.
        scen(
            "unusable-preference-fires",
            Rule::UnusablePreference,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nprefer_after:\n  - GHOST",
            )],
        )
        .effort(),
        // The near-miss: the same field, naming a task that exists. A preference
        // is legal and silent; only an unusable one is reported.
        scen(
            "unusable-preference-silent",
            Rule::UnusablePreference,
            Silent,
            vec![
                tf(
                    "a",
                    "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nprefer_after:\n  - A-2",
                ),
                tf("b", "id: A-2\ntitle: A-2\nphase: 0\nstatus: todo"),
            ],
        )
        .effort(),
        // APLAN032, every `paths` entry is a glob this tool can compile. A
        // hermetic rule about a task's own field, and the reason it is hermetic
        // is the defect: the ladder used to compile these globs and abort,
        // hiding every other task's findings behind one task's typo (`A1-066`).
        scen(
            "bad-path-glob-fires",
            Rule::BadPathGlob,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\npaths:\n  - 'src/['",
            )],
        )
        .effort(),
        // The near-miss: the same character class, closed. A glob with `[` in it
        // is not the defect; an unclosed one is.
        scen(
            "bad-path-glob-silent",
            Rule::BadPathGlob,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\npaths:\n  - 'src/[a-z]*.rs'",
            )],
        )
        .effort(),
        // APLAN023, a `done` task's path-checkable evidence can be path-checked.
        // The fires case is the one the p0000004 pilot found: `done`, a real
        // `commit` citation, and no `paths`, which `check` and `verify` both
        // reported clean while `APLAN012` had nothing to compare against.
        scen(
            "uncheckable-evidence-fires",
            Rule::UncheckableEvidence,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\nevidence:\n  - kind: commit\n    ref: 1b741aee6ca9",
            )],
        )
        .effort(),
        // The near-miss: the same task, with `paths` declared, so the citation is
        // bindable and the rule has nothing to say.
        scen(
            "uncheckable-evidence-silent",
            Rule::UncheckableEvidence,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: done\npaths:\n  - src/**\nevidence:\n  - kind: commit\n    ref: 1b741aee6ca9",
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
        // APLAN055, an authored capture date is not accepted as ersatz
        // freshness metadata on a machine-resolved citation.
        scen(
            "inapplicable-capture-fires",
            Rule::InapplicableCapture,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 evidence:\n  - kind: file\n    ref: docs/proof.md\n    captured: 2026-07-14",
            )],
        ),
        // The one kind for which the field is provenance keeps its established
        // shape and still belongs to APLAN010 alone.
        scen(
            "inapplicable-capture-silent",
            Rule::InapplicableCapture,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\neffort_days: 1\n\
                 evidence:\n  - kind: note\n    ref: spoke to the division\n    captured: 2026-07-14",
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
        // APLAN021's second reason: it has members, but it is carrying work. The
        // message interpolates the reason, so this is a different sentence and
        // the catalogue recorded neither it nor the `paths` variant until the
        // audit of `A1-088` looked.
        scen(
            "milestone-hygiene-carries-work-fires",
            Rule::MilestoneHygiene,
            Fires,
            vec![
                tf(
                    "m",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: todo\ntags: [milestone]\n\
                     blocked_by: [A-1]\neffort_days: 3",
                ),
                tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
            ],
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
    ]
}

/// A family requiring exactly one member on every task.
fn exactly_one_epic() -> aplan_core::TagFamily {
    aplan_core::TagFamily {
        members: vec!["epic-a".to_string(), "epic-b".to_string()],
        require: aplan_core::Cardinality::ExactlyOne,
        status: aplan_core::StatusScope::Any,
        milestones: aplan_core::MilestoneScope::Include,
    }
}

/// The declared-vocabulary rules, `APLAN026` and `APLAN029`, split out for the
/// same reason `cancelled_cut_rules` was: the line-limit gate noticing a
/// function had become a list. They belong together anyway, since one judges a
/// tag against a family and the other the family against the plan.
fn tag_family_rules() -> Vec<Scenario> {
    vec![
        // APLAN026, a tag that looks like a member of a declared family is one.
        scen(
            "unknown-tag-fires",
            Rule::UnknownTag,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntags: [epci-b]",
            )],
        )
        .tag_families(),
        // APLAN029, a declared tag family is carried by at least one task. The
        // fourth pilot's exact configuration: the family name read as a prefix
        // and the members as suffixes, with tasks tagged `epic:a`. Every layer
        // accepted it and the roadmap grouped nothing (`A1-085`).
        scen(
            "unused-tag-family-fires",
            Rule::UnusedTagFamily,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntags: [epic:a]",
            )],
        )
        .tag_families_of(&["a", "b"]),
        scen(
            "unused-tag-family-silent",
            Rule::UnusedTagFamily,
            Silent,
            // One task carrying a declared member is all it takes, and the tag
            // beside it belongs to no family, which stays legal.
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntags: [epic-b, anything]",
            )],
        )
        .tag_families(),
        // APLAN030, a task carries the number of members a declared family
        // requires. The fifth pilot needs exactly one priority on live work, and
        // before this both "two priorities" and "no priority" were clean
        // (`A1-133`, pre-public adoption evidence).
        scen(
            "tag-cardinality-fires",
            Rule::TagCardinality,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntags: [epic-a, epic-b]",
            )],
        )
        .tag_family(exactly_one_epic()),
        scen(
            "tag-cardinality-silent",
            Rule::TagCardinality,
            Silent,
            // One member, which is what the family asks for.
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntags: [epic-a]",
            )],
        )
        .tag_family(exactly_one_epic()),
        scen(
            "unknown-tag-silent",
            Rule::UnknownTag,
            Silent,
            // A declared member, and a tag outside every family: both legal.
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\ntags: [epic-b, anything]",
            )],
        )
        .tag_families(),
    ]
}

fn owner_vocabulary_rules() -> Vec<Scenario> {
    vec![
        scen(
            "owner-vocabulary-fires",
            Rule::OwnerVocabulary,
            Fires,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nowner: kaminsdo",
            )],
        )
        .owners(&["kaminsod"]),
        scen(
            "owner-vocabulary-silent",
            Rule::OwnerVocabulary,
            Silent,
            vec![tf(
                "a",
                "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo\nowner: kaminsod",
            )],
        )
        .owners(&["kaminsod"]),
    ]
}

/// `APLAN027`, split out because `milestone_rules` outgrew the line limit when
/// this arrived: the gate noticing a function had become a list.
fn cancelled_cut_rules() -> Vec<Scenario> {
    vec![
        // APLAN027, a cancelled cut's members are cancelled too.
        scen(
            "cancelled-cut-live-work-fires",
            Rule::CancelledCutLiveWork,
            Fires,
            vec![
                tf(
                    "m",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: dropped\n\
                     tags: [milestone, v0.1]\nblocked_by: [A-1]",
                ),
                tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: todo"),
            ],
        ),
        scen(
            "cancelled-cut-live-work-silent",
            Rule::CancelledCutLiveWork,
            Silent,
            // The cut is cancelled and so is its work, which is coherent. The
            // predecessor cut it follows is live, which is ordinary.
            vec![
                tf(
                    "m0",
                    "id: M-0\ntitle: M-0\nphase: 0\ntags: [milestone, v0.1]\n\
                     status: todo\nblocked_by: [B-1]",
                ),
                tf("b", "id: B-1\ntitle: B-1\nphase: 0\nstatus: todo"),
                tf(
                    "m1",
                    "id: M-1\ntitle: M-1\nphase: 0\nstatus: dropped\n\
                     tags: [milestone, v0.2]\nblocked_by: [A-1, M-0]",
                ),
                tf("a", "id: A-1\ntitle: A-1\nphase: 0\nstatus: dropped"),
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

/// The validator options a scenario's switches ask for.
///
/// One implementation, because there were two: `diagnostics_for` and
/// `scenario_run` built this literal side by side, so a switch added to one and
/// not the other would leave half the corpus silently not exercising a rule --
/// which is the defect a coverage audit exists to catch, in the audit
/// (`A1-273`).
fn options_for(s: &Scenario, dir: &std::path::Path) -> Options {
    Options {
        require_effort: s.require_effort,
        // The shipped defaults: a unit of work owes a fix, a cut a feature, a
        // release nothing of its own (ADR-0034).
        require_impact: s.require_impact.then_some(aplan_core::task::VersionFloors {
            task: aplan_core::task::VersionImpact::Fix,
            cut: aplan_core::task::VersionImpact::Feature,
            release: aplan_core::task::VersionImpact::None,
        }),
        traces_root: s.traces_root.then(|| dir.to_path_buf()),
        require_min_strength: s
            .require_strong
            .then(|| vec![EvidenceKind::Commit, EvidenceKind::Test, EvidenceKind::File]),
        detect_drift: s.detect_drift,
        require_cut_membership: s.require_cut_membership,
        tag_families: if let Some(members) = &s.tag_families {
            [("epic".to_string(), members.clone())]
                .into_iter()
                .collect()
        } else {
            std::collections::BTreeMap::new()
        },
        owners: s.owners.iter().map(|owner| (*owner).to_string()).collect(),
        agents_block: s.agents_block.map(|current| aplan_core::AgentsBlock {
            path: dir.join("AGENTS.md"),
            current: current.to_string(),
        }),
        ..Default::default()
    }
}

/// Materialise a scenario and return every diagnostic in full.
fn full_diagnostics_for(s: &Scenario) -> Vec<aplan_core::Diagnostic> {
    scenario_run(s).0
}

/// The diagnostics a scenario produces, and the temp root they may name.
///
/// Split out so the catalogue (`A1-088`) can strip that root: a message quoting
/// `/tmp/.tmpLNZh1R/tasks/one.md` is a different string on every run, and a
/// committed artefact full of them could never compare equal to itself.
fn scenario_run(s: &Scenario) -> (Vec<aplan_core::Diagnostic>, std::path::PathBuf) {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    for (rel, contents) in &s.files {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, contents).expect("write fixture");
    }
    let opts = options_for(s, dir.path());
    let mut diagnostics = validate(&load_plan(dir.path()), &opts).diagnostics;
    diagnostics.extend(
        aplan_core::capture::load_inbox(
            dir.path(),
            &aplan_core::capture::CapturePolicy::default(),
            &aplan_core::capture::FixedClock(2_000_000_000),
        )
        .diagnostics,
    );
    (diagnostics, dir.path().to_path_buf())
}

/// No diagnostic tells a consumer to go and read a decision record this project
/// keeps and they do not have (`A1-044`).
///
/// The sibling audit in `crates/aplan/tests/consumer_facing.rs` covers what
/// `aplan` *renders* (help, `facts`, the roadmap footer, an imported task body).
/// Diagnostics are the other half of what an adopter reads, in their CI log and
/// in SARIF, and their text is built at runtime from several pieces. Running the
/// corpus is what makes this complete rather than a sample: `Rule::ALL` is
/// already gated to have a scenario here, so every rule's message and
/// `agent_instruction` passes through this check by construction.
#[test]
fn no_diagnostic_cites_this_projects_adrs() {
    let mut leaks = Vec::new();
    for s in corpus() {
        for d in full_diagnostics_for(&s) {
            for (what, text) in [
                ("message", d.message.clone()),
                (
                    "agent_instruction",
                    d.agent_instruction.clone().unwrap_or_default(),
                ),
            ] {
                if text.contains("ADR-") {
                    leaks.push(format!("{} {what}: {text}", d.code));
                }
            }
        }
    }
    leaks.sort();
    leaks.dedup();
    assert!(
        leaks.is_empty(),
        "a diagnostic is read in the consumer's repository, where this project's ADRs do not \
         exist (A1-044). State the reason, and keep the number in a `//` comment:\n  {}",
        leaks.join("\n  ")
    );
}

/// Materialise a scenario to a temp directory and return the `(code, severity)`
/// of every diagnostic the validator reports over it.
fn diagnostics_for(s: &Scenario) -> Vec<(&'static str, Severity)> {
    scenario_run(s)
        .0
        .into_iter()
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
    /// Whether referent fingerprints are compared. False is the shipped default.
    detect_drift: bool,
    /// The configured `url` checker, for `APLAN016`. `None` leaves `url`
    /// evidence at L0, which is every other case.
    url_check: Option<aplan_config::UrlCheck>,
    /// The `version` policy, for `APLAN048`. `None` leaves the version
    /// unjudged, which is every other case and the shipped default.
    version: Option<aplan_config::VersionPolicy>,
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
        detect_drift: false,
        url_check: None,
        version: None,
    }
}

/// An `APLAN049` case: the same repository, plus a declared compatibility
/// checker whose exit status is the whole answer (`A1-314`).
///
/// `true` and `false` are the checker, because what is under test is the rule
/// and not an API differ. That is the same substitution `APLAN016` makes for the
/// network.
fn compat_case(name: &'static str, expect: Expect, declares: &str, compatible: bool) -> VerifyCase {
    let mut case = version_case(name, expect, v048_silent, declares);
    case.rule = Rule::ImpactContradicted;
    if let Some(v) = case.version.as_mut() {
        v.check = Some(aplan_config::DeclaredCommand {
            command: vec![if compatible { "true" } else { "false" }.to_string()],
            timeout_seconds: None,
            max_memory: None,
            max_processes: None,
            max_output: None,
            wrap: None,
        });
    }
    case
}

/// An `APLAN048` case: a git repository with a release tag, and a declared
/// command that prints the version the tree claims to ship (`A1-313`).
///
/// The command is `echo`, spawned with no shell exactly as an adopter's would
/// be. What it prints is the whole variable: the plan and the tag are the same
/// in both halves of the pair, and only the number the repository declares, and
/// the impact one task claims, move.
fn version_case(
    name: &'static str,
    expect: Expect,
    build: fn(&Repository) -> Vec<Task>,
    declares: &str,
) -> VerifyCase {
    VerifyCase {
        name,
        rule: Rule::VersionBehind,
        expect,
        build,
        runner: None,
        execute: false,
        detect_drift: false,
        url_check: None,
        version: Some(aplan_config::VersionPolicy {
            require_impact: None,
            levels: None,
            release_tag: None,
            check: None,
            current: Some(aplan_config::DeclaredCommand {
                command: vec!["echo".to_string(), declares.to_string()],
                timeout_seconds: None,
                max_memory: None,
                max_processes: None,
                max_output: None,
                wrap: None,
            }),
        }),
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
        detect_drift: false,
        url_check: None,
        version: None,
    }
}

fn verify_corpus() -> Vec<VerifyCase> {
    vec![
        // APLAN011, the commit SHA resolves in `HEAD` history.
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
        // APLAN048, the released version against what closed since the last
        // release tag (`A1-313`, ADR-0038). A `breaking` task closed on top of
        // `v0.3.0`, and the tree declares `0.3.1`: a patch release carrying an
        // incompatible change, which is the failure this whole feature exists
        // to prevent and the last moment it is still preventable.
        version_case("version-behind-fires", Fires, v048_fires, "0.3.1"),
        // The near miss, and the reason the pair is worth having: the same tree,
        // the same tag, the same bump, and the task claims `fix`. `0.3.1` is
        // then exactly what the plan requires, and a rule that merely noticed
        // "a version moved" would report it.
        version_case("version-behind-silent", Silent, v048_silent, "0.3.1"),
        // The other message this rule has, and the reason it is a branch rather
        // than a second rule: a version *older* than the tag it descends from is
        // the same subject and a completely different remedy. Without a scenario
        // the catalogue would record one of the two wordings and an adopter
        // would meet the other in their CI log (`APLAN021`'s lesson).
        version_case("version-behind-older-fires", Fires, v048_silent, "0.2.0"),
        // APLAN049, the artifact against the claim (`A1-314`). The checker says
        // the artifact is not compatible with its last release, and the window
        // claims a `fix`: a break shipping in a compatible release, which is the
        // gap between a field being filled in and the field being true.
        compat_case("impact-contradicted-fires", Fires, "0.3.1", false),
        // The near miss the pair exists for: the same plan, the same claim, and
        // a checker that found no break. A rule that merely noticed "a checker
        // was configured" would report it.
        compat_case("impact-contradicted-silent", Silent, "0.3.1", true),
        // APLAN045, a live task's traced section moved while it waited (`A1-301`).
        git_case("stale-premise-fires", Rule::StalePremise, Fires, v045_fires),
        git_case(
            "stale-premise-silent",
            Rule::StalePremise,
            Silent,
            v045_silent,
        ),
        // APLAN024, the citation is about this task rather than mostly other work.
        git_case(
            "diluted-evidence-fires",
            Rule::DilutedEvidence,
            Fires,
            v024_fires,
        ),
        git_case(
            "diluted-evidence-silent",
            Rule::DilutedEvidence,
            Silent,
            v024_silent,
        ),
        // APLAN025, the cited test belongs to code the task's `paths` cover.
        test_case(
            "test-irrelevant-fires",
            Rule::TestIrrelevant,
            Fires,
            v025_fires,
            rooted_runner("known", "component"),
            false,
        ),
        test_case(
            "test-irrelevant-silent",
            Rule::TestIrrelevant,
            Silent,
            v025_silent,
            rooted_runner("known", "component"),
            false,
        ),
        // ... and the same rule via `covers`, for a runner that selects its
        // component by argument and so runs at the repository root (`A1-065`).
        test_case(
            "test-irrelevant-covers-fires",
            Rule::TestIrrelevant,
            Fires,
            v025_fires,
            covering_runner("known", "component/**"),
            false,
        ),
        test_case(
            "test-irrelevant-covers-silent",
            Rule::TestIrrelevant,
            Silent,
            v025_silent,
            covering_runner("known", "component/**"),
            false,
        ),
        // ... and a repository-wide invariant, which lives where the harness is
        // and asserts a fact about a tree it is not part of (`A1-069`).
        test_case(
            "test-irrelevant-invariant-silent",
            Rule::TestIrrelevant,
            Silent,
            v025_fires,
            invariant_runner("known", "component/**"),
            false,
        ),
        // The third wording this rule can produce, and the one nothing exercised:
        // a `covers_by_test` subset narrower than the runner. The catalogue is
        // what makes a message change a reviewable diff, so a variant missing
        // from it is one that can drift in silence (`A1-070`).
        test_case(
            "test-irrelevant-subset-fires",
            Rule::TestIrrelevant,
            Fires,
            v025_fires,
            subset_runner("known", "component/**"),
            false,
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
        // The same rule, on the section a citation names (`A1-159`). The
        // catalogue is where an adopter learns what a rule *says*, and these
        // three sentences send a reader somewhere the missing-file one does not.
        git_case(
            "file-anchor-unresolved-fires",
            Rule::FileUnresolved,
            Fires,
            v015_anchor_fires,
        ),
        git_case(
            "file-anchor-empty-fires",
            Rule::FileUnresolved,
            Fires,
            v015_anchor_empty,
        ),
        git_case(
            "file-anchor-not-text-fires",
            Rule::FileUnresolved,
            Fires,
            v015_anchor_binary,
        ),
        // The same rule on a citation that leaves the repository (`A1-249`,
        // ADR-0028). Its own sentence, because "does not exist at HEAD" is false
        // about a file the author can open, and the remedy is not a spelling
        // correction.
        git_case(
            "file-outside-the-repository-fires",
            Rule::FileUnresolved,
            Fires,
            v015_outside,
        ),
        git_case(
            "file-anchor-silent",
            Rule::FileUnresolved,
            Silent,
            v015_anchor_silent,
        ),
        VerifyCase {
            name: "evidence-drift-fires",
            rule: Rule::EvidenceDrift,
            expect: Fires,
            build: v054_fires,
            runner: None,
            execute: false,
            detect_drift: true,
            url_check: None,
            version: None,
        },
        VerifyCase {
            name: "evidence-drift-silent",
            rule: Rule::EvidenceDrift,
            expect: Silent,
            build: v054_silent,
            runner: None,
            execute: false,
            detect_drift: true,
            url_check: None,
            version: None,
        },
        // APLAN031, a cited file lies inside the task's `paths`. `APLAN012`'s
        // question, asked of the other kind that names an artefact (`A1-045`).
        git_case(
            "file-irrelevant-fires",
            Rule::FileIrrelevant,
            Fires,
            v031_fires,
        ),
        git_case(
            "file-irrelevant-silent",
            Rule::FileIrrelevant,
            Silent,
            v031_silent,
        ),
        // The near-miss that decides whether the rule is worth having: a task
        // declaring `paths` it cannot be bound by must fail relevance rather
        // than satisfy it, exactly as `APLAN012` reads a catch-all.
        git_case(
            "file-irrelevant-catch-all-fires",
            Rule::FileIrrelevant,
            Fires,
            v031_catch_all,
        ),
        // APLAN035, a cited merge commit. Measured before it shipped: zero of
        // the 153 commit citations across this repository's plan and two
        // pilots' name a merge, while 23 of the first pilot's 123 commits are
        // merges -- so it is silent wherever the plan is right, and the plan
        // that got it wrong is where the rule came from (`A1-145`).
        git_case(
            "merge-commit-evidence-fires",
            Rule::MergeCommitEvidence,
            Fires,
            v035_fires,
        ),
        git_case(
            "merge-commit-evidence-silent",
            Rule::MergeCommitEvidence,
            Silent,
            v035_silent,
        ),
        // APLAN016, a cited url answers when checked. The "network" is a shell
        // exit code, because what is under test is the rule, not curl.
        VerifyCase {
            name: "url-unreachable-fires",
            rule: Rule::UrlUnreachable,
            expect: Fires,
            build: v016_fires,
            runner: None,
            execute: false,
            detect_drift: false,
            url_check: Some(url_check(false)),
            version: None,
        },
        VerifyCase {
            name: "url-unreachable-silent",
            rule: Rule::UrlUnreachable,
            expect: Silent,
            build: v016_fires,
            runner: None,
            execute: false,
            detect_drift: false,
            url_check: Some(url_check(true)),
            version: None,
        },
        // No checker configured is the default, and must reach no verdict at all:
        // `check` is hermetic by contract and a default run touches no network.
        VerifyCase {
            name: "url-unreachable-silent-without-a-checker",
            rule: Rule::UrlUnreachable,
            expect: Silent,
            build: v016_fires,
            runner: None,
            execute: false,
            detect_drift: false,
            url_check: None,
            version: None,
        },
        // The https-only branch: a citation naming another scheme is reported as
        // *not checked*, and the checker never sees it (`A1-041`, audit).
        VerifyCase {
            name: "url-not-https-fires",
            rule: Rule::UrlUnreachable,
            expect: Fires,
            build: v016_not_https,
            runner: None,
            execute: false,
            detect_drift: false,
            url_check: Some(url_check(true)),
            version: None,
        },
        // APLAN019, each `paths` glob matches a tracked file.
        git_case(
            "paths-unmatched-fires",
            Rule::PathsUnmatched,
            Fires,
            v019_fires,
        ),
        git_case(
            "paths-unmatched-silent",
            Rule::PathsUnmatched,
            Silent,
            v019_silent,
        ),
        git_case(
            "paths-unmatched-silent-all-match",
            Rule::PathsUnmatched,
            Silent,
            v019_silent_all_match,
        ),
        // The multi-glob shape, which nothing exercised until `A1-141`: a dead
        // glob beside a live one the citation binds through is a stale
        // declaration, not an unbound claim.
        git_case(
            "paths-unmatched-silent-bound-sibling",
            Rule::PathsUnmatched,
            Silent,
            v019_silent_bound_sibling,
        ),
        // The bare-directory shape every pilot hit, and the branch that offers the
        // one-character fix rather than the generic advice.
        git_case(
            "paths-unmatched-bare-directory-fires",
            Rule::PathsUnmatched,
            Fires,
            v019_bare_directory,
        ),
        // APLAN053, a stored `in_progress` the repository can contradict
        // (`A1-359`). Beside `APLAN019` because both read the tracked set.
        git_case(
            "unstarted-in-progress-fires",
            Rule::UnstartedInProgress,
            Fires,
            v053_fires,
        ),
        git_case(
            "unstarted-in-progress-silent",
            Rule::UnstartedInProgress,
            Silent,
            v053_silent,
        ),
        git_case(
            "unstarted-in-progress-bare-directory-fires",
            Rule::UnstartedInProgress,
            Fires,
            v053_fires_on_a_bare_directory,
        ),
        git_case(
            "unstarted-in-progress-silent-no-paths",
            Rule::UnstartedInProgress,
            Silent,
            v053_silent_no_paths,
        ),
        git_case(
            "unstarted-in-progress-silent-not-started",
            Rule::UnstartedInProgress,
            Silent,
            v053_silent_not_started,
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
        // The near-match branch (`A1-109`). The pilot cited `guider::name` where
        // the runner lists it bare, so the citation is one `::` prefix away from
        // resolving and the message says which name is the real one. A new branch
        // of an interpolated reason is exactly what `A1-094` requires a scenario
        // for, or the catalogue would not show the sentence an adopter reads.
        test_case(
            "test-unresolved-near-match-fires",
            Rule::TestUnresolved,
            Fires,
            v_test_qualified,
            list_runner("known", true),
            false,
        ),
        // APLAN036, a configured runner that answers and names nothing. The
        // repository that found this configures five runners and two of them are
        // legitimately empty, so the benign case has to be expressible or an
        // adopter chooses between a permanent finding and deleting a runner
        // (pre-public adoption evidence item 13, `A1-064`).
        test_case(
            "runner-lists-nothing-fires",
            Rule::RunnerListsNothing,
            Fires,
            v_test_ghost,
            silent_runner(None),
            false,
        ),
        // Silent two ways, and both matter. A runner that lists the cited test:
        test_case(
            "runner-lists-nothing-silent",
            Rule::RunnerListsNothing,
            Silent,
            v_test_known,
            list_runner("known", true),
            false,
        ),
        // ... and a runner that lists nothing and says it expects none.
        test_case(
            "runner-lists-nothing-declared-inert-silent",
            Rule::RunnerListsNothing,
            Silent,
            v_test_ghost,
            silent_runner(Some(false)),
            false,
        ),
        // APLAN037: the same `run` command, and the difference is what `list`
        // prints. Cited by the listed name nothing is selected, the status is 0,
        // and only the *output* separates that from a pass.
        test_case(
            "run-proves-nothing-fires",
            Rule::RunProvesNothing,
            Fires,
            v_test_listed_name,
            filter_runner("tests/math.test.js > adds", None),
            true,
        ),
        // Cited by the name the filter actually selects, a test really runs.
        test_case(
            "run-proves-nothing-silent",
            Rule::RunProvesNothing,
            Silent,
            v_test_title,
            filter_runner("adds", None),
            true,
        ),
        // And the declared claim, for a runner that says nothing either way.
        test_case(
            "run-proves-nothing-declared-selective-silent",
            Rule::RunProvesNothing,
            Silent,
            v_test_listed_name,
            filter_runner("tests/math.test.js > adds", Some(true)),
            true,
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
        // APLAN014 again, and the second thing it says: the runner lists the
        // cited test among the ones it will not run, so `--execute` exiting zero
        // proved nothing (`A1-300`).
        test_case(
            "test-skipped-fires",
            Rule::TestFailed,
            Fires,
            v_test_known,
            skipping_runner("known", "known"),
            true,
        ),
        // The same runner, skipping something else: the citation ran.
        test_case(
            "test-skipped-silent",
            Rule::TestFailed,
            Silent,
            v_test_known,
            skipping_runner("known", "a-different-test"),
            true,
        ),
    ]
}

/// A runner that passes everything and declares one test it will not run.
///
/// `printf` for both lists, as `list_runner` does: the mechanism under test is
/// that the runner is *asked*, not anything about cargo (`A1-300`).
fn skipping_runner(lists: &str, skips: &str) -> TestRunner {
    TestRunner {
        weight: None,
        exclusive: None,
        skipped: Some(vec!["printf".into(), format!("{skips}\n")]),
        ..list_runner(lists, true)
    }
}

/// One `run` command that really filters, paired with a `list` that prints
/// either the names it selects or names it does not (`A1-167`).
///
/// The mechanism rather than a stand-in for it: the argument is matched against
/// each test's *title*, the way `vitest run -t` does, and the summary line says
/// which happened. Paired with a listing of titles the citation runs a test;
/// paired with a listing of `file > suite > test` it selects nothing, prints
/// what a miss prints, and exits `0`. A fixture that merely exited `0` would
/// assert the same thing while demonstrating none of it.
fn filter_runner(lists: &str, declares_selective: Option<bool>) -> TestRunner {
    let filter = "hit=0; for t in adds zero; do case ''$t'' in *''$1''*) hit=1 ;; esac; done; \
                  [ $hit = 1 ] && echo ''Tests  1 passed (2)'' || echo ''Tests  2 skipped (2)''"
        .replace("''", "\"");
    TestRunner {
        weight: None,
        exclusive: None,
        output_lines: None,
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        expect_tests: None,
        skipped: None,
        run_selects: declares_selective,
        timeout_seconds: None,
        covers: None,
        covers_by_test: None,
        inputs_by_test: None,
        name: Some("vitest-shaped".into()),
        dir: None,
        list: vec!["printf".into(), format!("{lists}\n")],
        run: vec![
            "sh".into(),
            "-c".into(),
            filter,
            "sh".into(),
            PLACEHOLDER.into(),
        ],
        batch: None,
    }
}

/// A runner that answers and names nothing, which is `APLAN036`'s subject.
///
/// `printf ""` rather than a failing command: a runner that *cannot* be asked is
/// the `APLAN013` fault, and the whole point of `APLAN036` is that these two need
/// different fixes and used to be indistinguishable.
fn silent_runner(expect_tests: Option<bool>) -> TestRunner {
    TestRunner {
        weight: None,
        exclusive: None,
        output_lines: None,
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        skipped: None,
        expect_tests,
        run_selects: None,
        timeout_seconds: None,
        covers: None,
        covers_by_test: None,
        inputs_by_test: None,
        name: Some("blind".into()),
        dir: None,
        list: vec!["printf".into(), String::new()],
        run: vec!["true".into(), PLACEHOLDER.into()],
        batch: None,
    }
}

/// A runner that lists exactly `name` and whose `run` passes (`true`) or fails
/// (`false`). Uses the `{test}` placeholder so the name is a standalone arg.
fn list_runner(name: &str, passes: bool) -> TestRunner {
    let run_program = if passes { "true" } else { "false" };
    TestRunner {
        weight: None,
        exclusive: None,
        output_lines: None,
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        expect_tests: None,
        skipped: None,
        run_selects: None,
        timeout_seconds: None,
        covers: None,
        covers_by_test: None,
        inputs_by_test: None,
        name: None,
        dir: None,
        list: vec!["printf".into(), format!("{name}\n")],
        run: vec![run_program.into(), PLACEHOLDER.into()],
        batch: None,
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
    // `src/` is tracked first, so this fires because the *cited commit* touches
    // nothing the task owns, not because the task owns nothing. Before
    // `APLAN019` existed the two were indistinguishable here, and the fixture
    // took the second reading without saying so: exactly the confusion
    // pre-public adoption evidence item 18 reports from a real plan (`A1-022`).
    git_commit(repo, &[("src/a.rs", "x")], "src exists");
    let sha = git_commit(repo, &[("docs/x.md", "prose")], "docs only");
    vec![done("A", vec![commit_ev(&sha)], vec!["src/**".into()])]
}
fn v012_silent(repo: &Repository) -> Vec<Task> {
    let sha = git_commit(repo, &[("src/a.rs", "x")], "src");
    vec![done("A", vec![commit_ev(&sha)], vec!["src/**".into()])]
}
/// `APLAN024`: the citation overlaps, but the commit is mostly other work.
///
/// One file inside `src/**` and sixty outside it, which is the shape the p00025
/// pilot measured: a scaffold commit any `paths` list would overlap.
fn v024_fires(repo: &Repository) -> Vec<Task> {
    let mut files: Vec<(String, String)> = vec![("src/a.rs".to_string(), "x".to_string())];
    for i in 0..60 {
        files.push((format!("elsewhere/f{i}.txt"), "x".to_string()));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    let sha = git_commit(repo, &refs, "scaffold");
    vec![done("A", vec![commit_ev(&sha)], vec!["src/**".into()])]
}

/// The near-miss: a commit just as large, wholly inside the task's `paths`. Size
/// is not the finding; being about something else is.
fn v024_silent(repo: &Repository) -> Vec<Task> {
    let files: Vec<(String, String)> = (0..60)
        .map(|i| (format!("src/f{i}.rs"), "x".to_string()))
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    let sha = git_commit(repo, &refs, "a big change, all of it this task's");
    vec![done("A", vec![commit_ev(&sha)], vec!["src/**".into()])]
}

/// A checker that always answers, or never. `true` exits zero.
fn url_check(reachable: bool) -> aplan_config::UrlCheck {
    aplan_config::UrlCheck {
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        timeout_seconds: None,
        check: vec![
            if reachable { "true" } else { "false" }.to_string(),
            aplan_config::URL_PLACEHOLDER.to_string(),
        ],
    }
}
fn v016_fires(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done(
        "A",
        vec![Evidence {
            kind: EvidenceKind::Url,
            reference: "https://example.invalid/receipt".to_string(),
            captured: None,
        }],
        vec!["src/**".to_string()],
    )]
}

fn v016_not_https(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done(
        "A",
        vec![Evidence {
            kind: EvidenceKind::Url,
            reference: "http://169.254.169.254/latest/meta-data/".to_string(),
            captured: None,
        }],
        vec!["src/**".to_string()],
    )]
}

/// A dead glob on a task whose citation is judged against it. The `file`
/// citation is what makes the finding possible: it is bound to the task by its
/// `paths`, and those reach nothing (`A1-138`).
fn v019_fires(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done(
        "A",
        vec![file_ev("src/a.rs")],
        vec!["src/gone/**".to_string()],
    )]
}
/// The same dead glob, on a task citing nothing. This is the *silent* case since
/// `A1-138`, and it is the one the fourth pilot met 140 times: a plan that names
/// the tree a later cut will create has forward-looking globs by construction,
/// and nothing is bound to them yet, so the rule's own stated consequence cannot
/// occur (pre-public adoption evidence item 24).
/// `APLAN053` fires: a task says work has started and the repository holds
/// nothing matching the `paths` it claims (`A1-359`).
fn v053_fires(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    let mut t = done("A", Vec::new(), vec!["src/not-yet/**".to_string()]);
    t.status = aplan_core::Status::InProgress;
    vec![t]
}
/// `APLAN053` silent: the same claim, with the work present. Read from the
/// index, so a file written and staged counts -- which is what keeps the rule
/// off a task started a minute ago that has created what it owns.
fn v053_silent(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/started.rs", "x")], "init");
    let mut t = done("A", Vec::new(), vec!["src/started.rs".to_string()]);
    t.status = aplan_core::Status::InProgress;
    vec![t]
}
/// `APLAN053` fires with the remedy that fits: the glob names a tracked
/// directory, so the work may well exist and the declaration is what matches
/// nothing (`A1-359`, audit). Here so the *wording* is pinned in the catalogue,
/// because it is the one an adopter acts on and `APLAN019`'s equivalent has
/// been met by every pilot.
fn v053_fires_on_a_bare_directory(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    let mut t = done("A", Vec::new(), vec!["src".to_string()]);
    t.status = aplan_core::Status::InProgress;
    vec![t]
}
/// `APLAN053` silent: `in_progress` with no `paths` at all. There is then
/// nothing to judge the claim by, and `APLAN023` already asks for them, so a
/// finding here would be that rule said twice.
fn v053_silent_no_paths(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    let mut t = done("A", Vec::new(), Vec::new());
    t.status = aplan_core::Status::InProgress;
    vec![t]
}
/// `APLAN053` silent: a `todo` task whose globs match nothing. The rule is
/// about a claim that work has *started*; a plan that names the tree a later
/// cut will create is what a plan is (`APLAN019`'s own scope, `A1-138`).
fn v053_silent_not_started(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    let mut t = done("A", Vec::new(), vec!["src/not-yet/**".to_string()]);
    t.status = aplan_core::Status::Todo;
    vec![t]
}
fn v019_silent(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    let mut t = done("A", Vec::new(), vec!["src/gone/**".to_string()]);
    t.status = aplan_core::Status::Todo;
    vec![t]
}
/// The fourth pilot's item 26: `paths` with a dead sibling, and a citation that
/// binds through the live one. Under `deny_warnings` this refused a close whose
/// evidence was bound (`A1-141`).
fn v019_silent_bound_sibling(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done(
        "A",
        vec![file_ev("src/a.rs")],
        vec!["src/**".to_string(), "fixtures/**".to_string()],
    )]
}
/// And the other silence, which is the ordinary one: every glob matches.
fn v019_silent_all_match(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done(
        "A",
        vec![file_ev("src/a.rs")],
        vec!["src/**".to_string()],
    )]
}
/// `paths: [src]` where `src/` is tracked: the glob names one file, that name is
/// a directory, and nothing matches. The remedy is `src/**`.
fn v019_bare_directory(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done(
        "A",
        vec![file_ev("src/a.rs")],
        vec!["src".to_string()],
    )]
}

fn v015_fires(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done("A", vec![file_ev("src/nope.rs")], Vec::new())]
}
/// A deliverable that landed in another repository, which is the shape the ninth
/// pilot brought (pre-public adoption evidence item 6). It exists on disk and in no
/// tree here.
fn v015_outside(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done(
        "A",
        vec![file_ev("../sibling/docs/log.md")],
        Vec::new(),
    )]
}
fn v015_silent(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done("A", vec![file_ev("src/a.rs")], Vec::new())]
}

fn stamped_file_task(content: &str) -> Task {
    let mut task = done("A", vec![file_ev("docs/proof.md")], Vec::new());
    task.evidence_fingerprints.insert(
        "file:docs/proof.md".to_string(),
        aplan_core::anchor::content_fingerprint(content.as_bytes()),
    );
    task
}

/// `APLAN054`: the file still exists at the same reference, but its committed
/// contents are not what closure reviewed.
fn v054_fires(repo: &Repository) -> Vec<Task> {
    let reviewed = "reviewed proof\n";
    git_commit(repo, &[("docs/proof.md", reviewed)], "review proof");
    git_commit(
        repo,
        &[("docs/proof.md", "different proof\n")],
        "rewrite proof",
    );
    vec![stamped_file_task(reviewed)]
}

/// The same mutable reference and a stamp of exactly what `HEAD` contains.
fn v054_silent(repo: &Repository) -> Vec<Task> {
    let reviewed = "reviewed proof\n";
    git_commit(repo, &[("docs/proof.md", reviewed)], "review proof");
    vec![stamped_file_task(reviewed)]
}
/// A citation naming a section the document does not have. The shape a project
/// whose proof is a reviewed document brings (`A1-159`, from the `p0000019`
/// pilot).
fn v015_anchor_fires(repo: &Repository) -> Vec<Task> {
    git_commit(
        repo,
        &[("docs/proof.md", "# Proof\n\n## Result\n\nHeld.\n")],
        "init",
    );
    vec![done(
        "A",
        vec![file_ev("docs/proof.md#outcome")],
        Vec::new(),
    )]
}
/// `path#` with nothing after it: a typo rather than a missing section, and the
/// remedy is different.
fn v015_anchor_empty(repo: &Repository) -> Vec<Task> {
    git_commit(
        repo,
        &[("docs/proof.md", "# Proof\n\n## Result\n\nHeld.\n")],
        "init",
    );
    vec![done("A", vec![file_ev("docs/proof.md#")], Vec::new())]
}
/// A tracked file with no text to find a heading in. The citation must not pass
/// on the strength of an anchor nothing read.
fn v015_anchor_binary(repo: &Repository) -> Vec<Task> {
    let workdir = repo.workdir().expect("non-bare").to_path_buf();
    std::fs::create_dir_all(workdir.join("docs")).expect("mkdir");
    std::fs::write(workdir.join("docs/scan.bin"), [0xff, 0xfe, 0x00, 0x01]).expect("write");
    git_commit(repo, &[("docs/keep.md", "# Keep\n")], "init");
    vec![done("A", vec![file_ev("docs/scan.bin#result")], Vec::new())]
}
/// The section is there, so the citation is proof.
fn v015_anchor_silent(repo: &Repository) -> Vec<Task> {
    git_commit(
        repo,
        &[("docs/proof.md", "# Proof\n\n## Result\n\nHeld.\n")],
        "init",
    );
    vec![done("A", vec![file_ev("docs/proof.md#result")], Vec::new())]
}
/// A file that exists, outside the task's `paths`: it proves the repository has
/// that file (`A1-045`). The fourth pilot closed a task about ADR authorship
/// with a Containerfile in one command.
fn v031_fires(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x"), ("README.md", "hi")], "init");
    vec![done(
        "A",
        vec![file_ev("README.md")],
        vec!["src/**".to_string()],
    )]
}
/// The same citation, inside the paths the task declares.
fn v031_silent(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x"), ("README.md", "hi")], "init");
    vec![done(
        "A",
        vec![file_ev("src/a.rs")],
        vec!["src/**".to_string()],
    )]
}
/// `paths` that own every file cannot bind a citation to the specific work, and
/// `paths` come from a task file: a match-everything claim fails relevance
/// rather than satisfying it.
fn v031_catch_all(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("src/a.rs", "x")], "init");
    vec![done("A", vec![file_ev("src/a.rs")], vec!["**".to_string()])]
}

// `test` rules do not touch git; the repo just has to exist for `verify`.
/// A runner rooted in one component of the repository, so it makes a claim
/// about which code it covers.
fn rooted_runner(name: &str, dir: &str) -> TestRunner {
    TestRunner {
        weight: None,
        exclusive: None,
        output_lines: None,
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        expect_tests: None,
        skipped: None,
        run_selects: None,
        timeout_seconds: None,
        covers: None,
        covers_by_test: None,
        inputs_by_test: None,
        name: Some(dir.to_string()),
        dir: Some(dir.to_string()),
        list: vec!["printf".into(), format!("{name}\n")],
        run: vec!["true".into(), PLACEHOLDER.into()],
        batch: None,
    }
}
/// A runner that declares what it covers while running at the repository root,
/// which is the shape `dir` cannot express.
/// The runner name a repository covering `component/**` would actually write.
fn runner_name(covers: &str) -> String {
    covers
        .trim_end_matches("/**")
        .trim_end_matches('/')
        .to_string()
}

fn covering_runner(name: &str, covers: &str) -> TestRunner {
    TestRunner {
        weight: None,
        exclusive: None,
        output_lines: None,
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        expect_tests: None,
        skipped: None,
        run_selects: None,
        timeout_seconds: None,
        covers: Some(vec![covers.to_string()]),
        covers_by_test: None,
        inputs_by_test: None,
        // A runner name, not the glob it covers. Setting it to `covers` made the
        // rule catalogue read "a runner that is called `component/**` and covers
        // `component/**`", which is not a shape any adopter would write and is
        // the example they learn the message from (`A1-070`, `A1-088`).
        name: Some(runner_name(covers)),
        dir: None,
        list: vec!["printf".into(), format!("{name}\n")],
        run: vec!["true".into(), PLACEHOLDER.into()],
        batch: None,
    }
}
/// A runner covering the whole tree, whose named subset covers one component:
/// the inverse of [`invariant_runner`], and the shape that makes the message
/// name the subset it matched rather than the runner's own claim.
fn subset_runner(name: &str, covers: &str) -> TestRunner {
    TestRunner {
        weight: None,
        exclusive: None,
        output_lines: None,
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        expect_tests: None,
        skipped: None,
        run_selects: None,
        timeout_seconds: None,
        covers: Some(vec!["**".to_string()]),
        covers_by_test: Some(vec![aplan_config::TestCoverage {
            tests: vec![format!("{name}*")],
            paths: vec![covers.to_string()],
        }]),
        inputs_by_test: None,
        name: Some(runner_name(covers)),
        dir: None,
        list: vec!["printf".into(), format!("{name}\n")],
        run: vec!["true".into(), PLACEHOLDER.into()],
        batch: None,
    }
}

/// A runner whose named subset covers the whole tree, while the runner itself
/// covers only its own component.
fn invariant_runner(name: &str, covers: &str) -> TestRunner {
    TestRunner {
        weight: None,
        exclusive: None,
        output_lines: None,
        max_memory: None,
        max_processes: None,
        max_output: None,
        wrap: None,
        expect_tests: None,
        skipped: None,
        run_selects: None,
        timeout_seconds: None,
        covers: Some(vec![covers.to_string()]),
        covers_by_test: Some(vec![aplan_config::TestCoverage {
            tests: vec![format!("{name}*")],
            paths: vec!["**".to_string()],
        }]),
        inputs_by_test: None,
        name: Some(runner_name(covers)),
        dir: None,
        list: vec!["printf".into(), format!("{name}\n")],
        run: vec!["true".into(), PLACEHOLDER.into()],
        batch: None,
    }
}
/// The runner's directory has to exist for it to be spawned there.
fn v025_fires(repo: &Repository) -> Vec<Task> {
    // `elsewhere/` is tracked too, so this fires because the runner covers a
    // different component, not because the task's `paths` name nothing. Same
    // correction as `v012_fires`.
    git_commit(
        repo,
        &[("component/t.rs", "x"), ("elsewhere/e.rs", "x")],
        "init",
    );
    vec![done(
        "A",
        vec![test_ev("known")],
        vec!["elsewhere/**".to_string()],
    )]
}
fn v025_silent(repo: &Repository) -> Vec<Task> {
    git_commit(repo, &[("component/t.rs", "x")], "init");
    vec![done(
        "A",
        vec![test_ev("known")],
        vec!["component/**".to_string()],
    )]
}
fn v_test_ghost(_repo: &Repository) -> Vec<Task> {
    vec![done("A", vec![test_ev("ghost")], Vec::new())]
}
fn v_test_known(_repo: &Repository) -> Vec<Task> {
    vec![done("A", vec![test_ev("known")], Vec::new())]
}
/// A citation of the name a *listing* prints, where the `run` filter matches a
/// test's own title -- which is the vitest shape `APLAN037` exists for.
fn v_test_listed_name(_repo: &Repository) -> Vec<Task> {
    vec![done(
        "A",
        vec![test_ev("tests/math.test.js > adds")],
        Vec::new(),
    )]
}
fn v_test_title(_repo: &Repository) -> Vec<Task> {
    vec![done("A", vec![test_ev("adds")], Vec::new())]
}
/// A citation the runner would list if the module prefix were dropped.
fn v_test_qualified(_repo: &Repository) -> Vec<Task> {
    vec![done("A", vec![test_ev("guider::known")], Vec::new())]
}

/// Write `files` into the repo work tree and commit them, returning the SHA.
/// `APLAN035` fires: the task cites the merge rather than the commit that did
/// the work, which is what `git rev-parse HEAD` hands you at the wrong moment
/// (`A1-145`, pre-public adoption evidence item 11).
fn v035_fires(repo: &Repository) -> Vec<Task> {
    let base = git_commit(repo, &[("f.txt", "one")], "first");
    let work = git_commit(repo, &[("src/a.rs", "work")], "the work");
    let merge = git_merge(repo, &work, &base);
    vec![done(
        "A",
        vec![commit_ev(&merge)],
        vec!["src/**".to_string()],
    )]
}
/// The same plan, citing the commit that did the work.
fn v035_silent(repo: &Repository) -> Vec<Task> {
    let base = git_commit(repo, &[("f.txt", "one")], "first");
    let work = git_commit(repo, &[("src/a.rs", "work")], "the work");
    git_merge(repo, &work, &base);
    vec![done(
        "A",
        vec![commit_ev(&work)],
        vec!["src/**".to_string()],
    )]
}

/// A merge of two commits, taking the first one's tree.
fn git_merge(repo: &Repository, first: &str, second: &str) -> String {
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let a = repo
        .find_commit(git2::Oid::from_str(first).unwrap())
        .unwrap();
    let b = repo
        .find_commit(git2::Oid::from_str(second).unwrap())
        .unwrap();
    let tree = a.tree().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Merge", &tree, &[&a, &b])
        .unwrap()
        .to_string()
}

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
        prefer_after: Vec::new(),
        ratified_by: Vec::new(),
        contains: Vec::new(),
        evidence,
        paths,
        traces: Vec::new(),
        trace_fingerprints: std::collections::BTreeMap::new(),
        evidence_fingerprints: std::collections::BTreeMap::new(),
        tags: Vec::new(),
        revisit_at: None,
        version_impact: None,
        version_impact_reason: None,
        version: None,
    }
}

/// A live task tracing one anchored section, for `APLAN045` (`A1-301`).
///
/// The task's own file is committed by the fixture, because the baseline this
/// rule reads is the commit that last touched it.
fn live_tracing(id: &str, trace: &str) -> Task {
    let mut t = done(id, Vec::new(), Vec::new());
    t.status = Status::Todo;
    t.traces = vec![trace.to_string()];
    t
}

/// A repository that released `v0.3.0`, then closed one task (`A1-313`).
///
/// The task file is committed *before* the tag with `status: todo` and after it
/// with `status: done`, because that is what "closed since the last release"
/// means and what the window reads: a plan held only in memory would make every
/// task look newly closed, which is the shape that would have made this pair
/// pass without measuring anything.
fn v048_repo(repo: &Repository, impact: &str) -> Vec<Task> {
    git_commit(
        repo,
        &[
            ("src/a.rs", "x"),
            (
                "plan/tasks/x.md",
                "---\nid: A-1\ntitle: A-1\nphase: 0\nstatus: todo\n---\n",
            ),
        ],
        "the plan, before the release",
    );
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.tag("v0.3.0", head.as_object(), &sig, "release", false)
        .expect("tag");
    let sha = git_commit(
        repo,
        &[
            ("src/a.rs", "y"),
            (
                "plan/tasks/x.md",
                "---\nid: A-1\ntitle: A-1\nphase: 0\nstatus: done\n---\n",
            ),
        ],
        "the work",
    );
    let mut task = done(
        "A-1",
        vec![Evidence {
            kind: EvidenceKind::Commit,
            reference: sha,
            captured: None,
        }],
        vec!["src/**".to_string()],
    );
    task.version_impact = Some(match impact {
        "breaking" => aplan_core::task::VersionImpact::Breaking,
        _ => aplan_core::task::VersionImpact::Fix,
    });
    vec![task]
}

fn v048_fires(repo: &Repository) -> Vec<Task> {
    v048_repo(repo, "breaking")
}

fn v048_silent(repo: &Repository) -> Vec<Task> {
    v048_repo(repo, "fix")
}

/// The section moved after the task was last touched, which is the finding.
fn v045_fires(repo: &Repository) -> Vec<Task> {
    git_commit(
        repo,
        &[
            ("DOC.md", "# Intro\n\nthe ground as it was\n"),
            ("plan/tasks/x.md", "the task, as committed"),
        ],
        "the task, written against the document",
    );
    git_commit(
        repo,
        &[("DOC.md", "# Intro\n\nthis has since been built\n")],
        "the document moved",
    );
    vec![live_tracing("A", "DOC.md#intro")]
}

/// The document moved and the traced *section* did not, which is the near miss
/// that keeps this from being the 91%-of-everything rule a file-level
/// comparison would have been (`A1-301`, measured).
fn v045_silent(repo: &Repository) -> Vec<Task> {
    git_commit(
        repo,
        &[
            ("DOC.md", "# Intro\n\nthe ground\n\n# Other\n\nelsewhere\n"),
            ("plan/tasks/x.md", "the task, as committed"),
        ],
        "the task, written against the document",
    );
    git_commit(
        repo,
        &[(
            "DOC.md",
            "# Intro\n\nthe ground\n\n# Other\n\nelsewhere, rewritten\n",
        )],
        "a different section moved",
    );
    vec![live_tracing("A", "DOC.md#intro")]
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
    verify_run(case)
        .0
        .iter()
        .map(|d| (d.code, d.severity))
        .collect()
}

/// The full diagnostics a verify case produces, and the temp root they may name.
///
/// `verify_diagnostics` reduced these to `(code, severity)` at the point of
/// production, so the message and the `agent_instruction` of every ladder rule
/// were unreachable. The catalogue needs them: without this, seven of the
/// twenty-five rules would be recorded as saying nothing at all.
fn verify_run(case: &VerifyCase) -> (Vec<aplan_core::Diagnostic>, std::path::PathBuf) {
    let (diagnostics, root, _) = verify_run_with_plan(case);
    (diagnostics, root)
}

/// The same, keeping the plan it ran over.
///
/// The vacuity gate needs the tasks beside the diagnostics: its question is
/// whether the rule that fired had declared a subject in the task it named
/// (`A1-114`).
fn verify_run_with_plan(
    case: &VerifyCase,
) -> (Vec<aplan_core::Diagnostic>, std::path::PathBuf, Plan) {
    let dir = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("tempdir");
    let repo = Repository::init(dir.path()).expect("git init");
    let tasks = (case.build)(&repo);
    let plan = Plan {
        tasks: tasks
            .into_iter()
            .map(|task| LoadedTask {
                task,
                path: "plan/tasks/x.md".into(),
                fields: aplan_core::load::FieldLines::new(),
                criteria: aplan_core::load::Criteria::default(),
            })
            .collect(),
        load_diagnostics: Vec::new(),
        unloadable: std::collections::BTreeMap::new(),
    };
    let opts = aplan_evidence::VerifyOptions {
        detect_drift: case.detect_drift,
        version: case.version.clone(),
        traces_root: None,
        config_file: None,
        execute: case.execute,
        test_runner: case.runner.clone().map(TestRunners::One),
        max_unrelated_files: None,
        observation_inputs: None,
        url_check: case.url_check.clone(),
    };
    let report = aplan_evidence::verify(&plan, dir.path(), &opts).expect("verify runs");
    (report.diagnostics, dir.path().to_path_buf(), plan)
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

/// A rule's declared applicability is checked against the rule's own behaviour
/// (`A1-114`).
///
/// `Rule::applies_to` and `Rule::binds_against` are what `aplan vacuity` counts
/// with, and putting them on the rule's row makes them *adjacent* to the rule
/// rather than derived from it: nothing stops a declaration drifting from the
/// predicate it describes, which is the defect this task was written to avoid,
/// one level further in.
///
/// This is the gate that closes it, and it derives from behaviour rather than
/// from a second reading of the code. A rule that fires on a task must have
/// declared a subject there, and must not have declared that task blind: a rule
/// cannot report a finding about something it had nothing to compare against.
/// The corpus fires every rule on purpose, so every declaration is exercised.
#[test]
fn a_rules_declared_applicability_agrees_with_where_it_fires() {
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for case in verify_corpus() {
        if case.expect != Fires {
            continue;
        }
        let (diags, _root, plan) = verify_run_with_plan(&case);
        let applies = case.rule.applies_to();
        let binds = case.rule.binds_against();
        if applies.is_empty() && binds.is_empty() {
            continue;
        }
        for d in diags.iter().filter(|d| d.code == case.rule.code()) {
            let Some(id) = d.task.as_deref() else {
                continue;
            };
            let Some(task) = plan.tasks.iter().map(|lt| &lt.task).find(|t| t.id == id) else {
                continue;
            };
            checked += 1;
            if let Some(missing) = applies.iter().find(|b| !b.holds(task)) {
                failures.push(format!(
                    "{}: {} fired on `{id}`, which `applies_to` says is not a subject ({missing:?} \
                     does not hold)",
                    case.name,
                    case.rule.code()
                ));
            }
            if let Some(missing) = binds.iter().find(|b| !b.holds(task)) {
                failures.push(format!(
                    "{}: {} fired on `{id}`, which `binds_against` calls blind ({missing:?} does \
                     not hold), so the rule reported a finding about something it had nothing to \
                     compare against",
                    case.name,
                    case.rule.code()
                ));
            }
        }
    }

    assert!(
        checked >= 4,
        "the gate checked {checked} firing(s), so it has stopped matching and passes vacuously -- \
         which would be this task's own subject"
    );
    assert!(
        failures.is_empty(),
        "a rule's declared applicability disagrees with where it fires:\n  {}",
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

// `every_format_is_exercised` lived here and could not fail: it built a list of
// every `Format` variant and asserted `Format::ALL` was contained in it, which
// is the enum compared against a copy of itself. The real unit is a (command,
// format) pair, and the audit that walks all forty of them has to run the binary
// -- which this crate cannot do, since it does not depend on `aplan`. It lives
// in `crates/aplan/tests/format_surfaces.rs` (`A1-210`).

// Deferred audit dimensions (ROADMAP §5), tracked openly rather than left as
// silent gaps:
//   - every CLI flag / exit code -> a trycmd case. The trycmd corpus already
//     covers the current surface; the *forcing* audit needs an enumerable model
//     of clap's flags, which lands with the config work (v0.4).
//   - every schema field -> a dispatch entry. There is no evidence-dispatch
//     table to audit against until the evidence ladder (v0.3).

// --- The rule catalogue ------------------------------------------------------
//
// What every rule *says*, committed and gated (`A1-088`).
//
// Three text defects shipped from this repository and every one was found by
// reading rather than by a gate: `APLAN024` named the first cited commit while
// scoring the worst (it reached the p00025 pilot), `APLAN026` named the family a
// tag was nearest to rather than the one it claimed, and a footer told readers
// that suppressed findings repeated a fix that in fact differs per finding. In
// all three the predicate was right and only the sentence was wrong, and a
// sentence changes without any test noticing.
//
// This makes the wording a reviewable artefact: `docs/rule-catalogue.md` is
// generated from the corpus, committed, and compared on every run, so changing
// what a rule says shows up as a diff next to the code that changed it.
//
// **Why a test and not `cargo xtask`**, which is how `schemas/v1/*.json` and
// `ROADMAP.status.md` are gated. Those are derived from library types that
// `xtask` can call. This is derived from the scenario corpus, which lives in
// this test binary: reaching it from `xtask` would mean hoisting some nine
// hundred lines of fixtures into a library so a build tool could read them,
// which is a large change made entirely for a gate. The repository already gates
// golden files from tests (`trycmd`, with `TRYCMD=overwrite`), and CI runs the
// tests, so drift fails the build either way.

/// Where the committed catalogue lives.
const CATALOGUE: &str = "docs/rule-catalogue.md";

/// The environment variable that rewrites it, mirroring `TRYCMD=overwrite`.
const OVERWRITE: &str = "APLAN_CATALOGUE";

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Whether a bare token is a git object id.
///
/// Bounded to the lengths git actually prints, an abbreviation through a full
/// id. The honest limit: an English word of seven or more letters drawn only
/// from `a-f` would also match, and `defaced` is such a word. None of the
/// twenty-five rules says anything of the kind, and the alternative rules are
/// worse. Requiring a letter would miss a short id that came out all digits,
/// which happens for roughly one abbreviation in twenty-seven and would make the
/// gate fail at random, and pinning the real ids would force a regeneration
/// every time a fixture's *content* changed, which says nothing about wording.
fn looks_like_a_sha(token: &str) -> bool {
    (7..=40).contains(&token.len()) && token.chars().all(|c| c.is_ascii_hexdigit())
}

/// Strip the two things that differ between runs of the same code.
///
/// A temp root (`/tmp/.tmpLNZh1R/...`) is fresh per run, and a commit SHA depends
/// on the committer timestamp, so both change without any wording changing. A
/// catalogue carrying them would fail on every run and be turned off within a
/// day, which is a worse outcome than not having one.
fn normalise(text: &str, root: &std::path::Path) -> String {
    let text = text.replace(&root.display().to_string(), "<tmp>");
    // Any bare hex run long enough to be a SHA becomes one placeholder.
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if looks_like_a_sha(token) {
            out.push_str("<sha>");
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// Every distinct thing each rule says, rendered deterministically.
///
/// Grouped by [`Rule::ALL`] rather than by scenario, and sorted within a rule, so
/// the file depends on the rule set and the wording and not on the order
/// scenarios happen to sit in. Adding a scenario that produces no new wording
/// leaves this byte-identical.
///
/// **Both corpora**, which the first version of this got wrong. `validate` is
/// hermetic and raises none of the evidence-ladder rules, so reading only the
/// check corpus recorded `APLAN011`-`APLAN015`, `APLAN024` and `APLAN025` as
/// producing no diagnostic at all: a generated file asserting something false
/// about seven rules, in the task written to stop exactly that.
fn render_catalogue() -> String {
    let mut said: std::collections::BTreeMap<&str, std::collections::BTreeSet<(String, String)>> =
        std::collections::BTreeMap::new();
    let mut record = |d: &aplan_core::Diagnostic, root: &std::path::Path| {
        said.entry(d.code).or_default().insert((
            normalise(&d.message, root),
            normalise(d.agent_instruction.as_deref().unwrap_or_default(), root),
        ));
    };
    for s in corpus() {
        let (diags, root) = scenario_run(&s);
        for d in &diags {
            record(d, &root);
        }
    }
    for case in verify_corpus() {
        let (diags, root) = verify_run(&case);
        for d in &diags {
            record(d, &root);
        }
    }

    let mut out = String::new();
    out.push_str("# What every rule says, as the corpus exercises it\n\n");
    out.push_str(
        "Generated from the scenario corpus in `crates/aplan-e2e/tests/coverage.rs`, committed,\n\
         and compared on every test run. Do not edit by hand: change the rule, then regenerate\n\
         with `APLAN_CATALOGUE=overwrite cargo test -p aplan-e2e --test integration \
         coverage::the_rule_catalogue_is_committed_and_current`.\n\n\
         A rule's message and its `agent_instruction` are what an adopter and an agent act on.\n\
         They are pinned here so that changing one is a reviewable diff rather than something\n\
         discovered in a consumer's CI log (`A1-088`).\n\n\
         **What this does not pin.** A rule that interpolates its reason says several different\n\
         sentences, and only the ones some scenario reaches appear below. The title says *as the\n\
         corpus exercises it* because that is the honest scope. `A1-094` swept for the gaps and\n\
         lists the ones left unreached, each with the reason its fixture is not worth building\n\
         yet; adding a scenario for one is how a wording joins this file.\n\n",
    );

    for rule in Rule::ALL {
        let code = rule.code();
        let sev = match rule.severity() {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let title = rule.title();
        let _ = writeln!(out, "## {code} ({sev})\n\n{title}\n");
        match said.get(code) {
            None => out.push_str(
                "_No scenario in the corpus produces this diagnostic._\n\n\
                 That is a gap: `every_rule_has_a_fires_and_a_silent_scenario` requires a\n\
                 *fires* case, so a rule reaching this line means its case stopped firing.\n\n",
            ),
            Some(pairs) => {
                for (message, instruction) in pairs {
                    let _ = writeln!(out, "- **says:** {message}");
                    if instruction.is_empty() {
                        out.push_str("  - _carries no `agent_instruction`_\n");
                    } else {
                        let _ = writeln!(out, "  - **fix:** {instruction}");
                    }
                }
                out.push('\n');
            }
        }
    }
    out
}

#[test]
fn the_rule_catalogue_is_committed_and_current() {
    let rendered = render_catalogue();
    let path = repo_root().join(CATALOGUE);

    if std::env::var(OVERWRITE).as_deref() == Ok("overwrite") {
        std::fs::create_dir_all(path.parent().expect("docs/")).expect("create docs/");
        std::fs::write(&path, &rendered).expect("write the catalogue");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        committed, rendered,
        "{CATALOGUE} and the rules disagree. If the new wording is what you meant, run\n  \
         {OVERWRITE}=overwrite cargo test -p aplan-e2e --test integration \
         coverage::the_rule_catalogue_is_committed_and_current\nand commit the result, \
         reading the diff: it is what every adopter will read."
    );
}

/// Wordings the corpus currently reaches. Raise it when a branch is added.
const WORDINGS: usize = 36;

#[test]
fn the_catalogue_pins_every_rule_and_is_not_vacuous() {
    // A generated file that silently became a header and nothing else would
    // compare equal to itself forever. This asserts it says something about every
    // rule, which is the property that makes it worth committing.
    let rendered = render_catalogue();
    let missing: Vec<&str> = Rule::ALL
        .iter()
        .map(|r| r.code())
        .filter(|c| !rendered.contains(&format!("## {c} (")))
        .collect();
    assert!(missing.is_empty(), "absent from the catalogue: {missing:?}");

    // Per rule, not a total: a count alone passes when one rule falls to zero and
    // another gains a wording, and the first version of this test did exactly
    // that while seven rules said nothing at all.
    let silent: Vec<&str> = Rule::ALL
        .iter()
        .map(|r| r.code())
        .filter(|c| {
            let section = rendered
                .split(&format!("## {c} ("))
                .nth(1)
                .unwrap_or_default();
            let body = section.split("\n## ").next().unwrap_or_default();
            !body.contains("- **says:** ")
        })
        .collect();
    assert!(
        silent.is_empty(),
        "these rules produce no diagnostic in either corpus, so their `fires` case has stopped \
         firing: {silent:?}"
    );

    // A floor on the total, on top of the per-rule check above. A rule with two
    // branches keeps saying something after one of them loses its scenario, so
    // the per-rule test cannot see that; this can. Raise it when a branch is
    // added, which is the same discipline as the coverage floors.
    let says = rendered.matches("- **says:** ").count();
    assert!(
        says >= WORDINGS,
        "the catalogue pins {says} wordings, below the {WORDINGS} known to exist: a scenario \
         that reached one of a rule's branches has stopped reaching it"
    );

    assert!(
        !rendered.contains("_No scenario in the corpus produces this diagnostic._"),
        "the catalogue records a rule as saying nothing, which is the shape of the defect \
         `A1-088` shipped and corrected: it read only the hermetic corpus and reported all seven \
         evidence-ladder rules as silent"
    );
}

#[test]
fn the_normaliser_scrubs_ids_and_roots_and_leaves_prose_alone() {
    let root = std::path::Path::new("/tmp/.tmpABC123");
    assert_eq!(
        normalise("commit `d213c9a` in /tmp/.tmpABC123/tasks/x.md", root),
        "commit `<sha>` in <tmp>/tasks/x.md"
    );
    // An abbreviation that came out all digits is still an id.
    assert!(looks_like_a_sha("1234567"));
    assert!(looks_like_a_sha(&"a".repeat(40)));
    // Ordinary words and short numbers are not.
    assert!(!looks_like_a_sha("done"));
    assert!(!looks_like_a_sha("60"));
    assert!(!looks_like_a_sha("milestone"));
    assert!(!looks_like_a_sha(&"1".repeat(41)), "longer than any git id");
    assert_eq!(
        normalise("is `done` but blocker `A-2` is `todo`", root),
        "is `done` but blocker `A-2` is `todo`",
        "a message with no id and no temp root must survive untouched"
    );
}

/// Every backticked span in a string, without the backticks.
///
/// **Deliberately duplicated** in `crates/aplan/tests/consumer_facing.rs`. This
/// crate sits at the top of the dependency graph, so sharing the helper the other
/// way would make the binary crate dev-depend on its own end-to-end suite and
/// leave a cycle waiting to happen. Both copies carry the same unit test, so a
/// divergence fails rather than hides.
fn backticked(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        match after.find('`') {
            Some(close) => {
                out.push(
                    after[..close]
                        .lines()
                        .map(str::trim)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                rest = &after[close + 1..];
            }
            None => break,
        }
    }
    out
}

#[test]
fn a_diagnostic_names_only_ids_its_own_scenario_contains() {
    // `A1-089`. The one property here with any reach into the class that hurt: a
    // message naming an entity that is not the one the verdict is about.
    //
    // **Its limit, stated so nobody mistakes it for more.** It catches a message
    // naming something *absent entirely*. It cannot catch `APLAN024` naming the
    // wrong one of two commits that are both present, which is the defect that
    // reached the p00025 pilot. Only `A1-091`, making a predicate return the
    // entity it judged, prevents that.
    //
    // Scoped to things shaped like a task id, because a message legitimately
    // backticks field names (`blocked_by`), statuses (`done`), commands
    // (`aplan close`), globs (`src/**`) and sibling rule codes (`APLAN012`), none
    // of which are in a fixture.
    let mut problems = Vec::new();
    let mut checked = 0usize;

    for s in corpus() {
        let inputs: String = s.files.iter().map(|(_, c)| c.as_str()).collect();
        for d in full_diagnostics_for(&s) {
            let texts = [
                d.message.clone(),
                d.agent_instruction.clone().unwrap_or_default(),
            ];
            for span in texts.iter().flat_map(|t| backticked(t)) {
                if !looks_like_a_task_id(&span) {
                    continue;
                }
                checked += 1;
                if !inputs.contains(&span) {
                    problems.push(format!(
                        "{}: {} names `{span}`, which is in no file of scenario `{}`",
                        s.name, d.code, s.name
                    ));
                }
            }
        }
    }

    assert!(
        checked >= 10,
        "only {checked} id-shaped names were checked, so this property has stopped matching"
    );
    assert!(
        problems.is_empty(),
        "a diagnostic names an id that does not exist in the plan it is about:\n  {}",
        problems.join("\n  ")
    );
}

/// Whether a backticked span looks like a task id rather than a field name, a
/// status, a command, a glob or a rule code.
fn looks_like_a_task_id(span: &str) -> bool {
    if span.starts_with("APLAN") || span.contains(' ') || span.contains('/') {
        return false;
    }
    let Some(first) = span.chars().next() else {
        return false;
    };
    // An upper-case initial and nothing but id characters after it, which admits
    // `A-1`, `M-s0`, `DUP` and `GHOST` and rejects `done` and `blocked_by`.
    //
    // Deliberately *not* "and contains a digit": `DUP` and `GHOST` are ids in this
    // corpus and carry none. An earlier version of this comment claimed the digit
    // rule while the code never implemented it, and named those two as examples of
    // it, which is the defect this task exists to catch written into the task
    // itself.
    first.is_ascii_uppercase()
        && span.len() >= 2
        && span
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[test]
fn the_id_shape_test_admits_ids_and_rejects_vocabulary() {
    assert!(looks_like_a_task_id("A-1"));
    assert!(looks_like_a_task_id("GHOST"));
    assert!(looks_like_a_task_id("M-s0"));
    assert!(!looks_like_a_task_id("done"), "a status");
    assert!(!looks_like_a_task_id("blocked_by"), "a field name");
    assert!(!looks_like_a_task_id("APLAN012"), "a sibling rule code");
    assert!(!looks_like_a_task_id("aplan close"), "a command");
    assert!(!looks_like_a_task_id("src/**"), "a glob");
}

#[test]
fn backticks_are_read_in_pairs_and_an_unclosed_one_ends_the_scan() {
    // The same cases as the copy in `crates/aplan/tests/consumer_facing.rs`; if
    // the two ever disagree, one of these fails.
    assert_eq!(
        backticked("re-stamp with `aplan close --restamp`, or reopen"),
        vec!["aplan close --restamp"]
    );
    assert_eq!(backticked("`a` and `b`"), vec!["a", "b"]);
    assert_eq!(backticked("no ticks here"), Vec::<String>::new());
    assert_eq!(backticked("an `unclosed span"), Vec::<String>::new());
    assert_eq!(backticked("``"), vec![""]);
    assert_eq!(backticked("`a  b`"), vec!["a  b"]);
    assert_eq!(
        backticked("run `aplan check\r\n    --format json` now"),
        vec!["aplan check --format json"],
        "terminal wrapping is presentation, not part of the invocation"
    );
}

/// Every rule that names a field places its finding on that field (`A1-202`).
///
/// The unit audit checks that each declared field exists in the schema, and the
/// loader test checks that the scanner can find such a field. Neither one runs a
/// rule. This does: it walks the same corpus `Rule::ALL` is already gated
/// against, so a rule that declares a field and then reports a finding somewhere
/// else -- or nowhere -- is caught by construction rather than by whichever two
/// rules somebody thought to check by hand.
///
/// A field the scenario's own file does not carry is skipped, not failed:
/// `APLAN005` is *about* a missing `evidence:` and `APLAN009` about a missing
/// `effort_days:`, so there is no line for either to name, and saying so is the
/// specification rather than a gap in it.
///
/// **What it does not reach.** The hermetic corpus only. `verify_corpus()`
/// builds its tasks in memory, with no file behind them and so no field lines to
/// find, and rewriting that harness to write real files would be a large change
/// for one assertion. The evidence half is covered instead by
/// `verify_places_an_evidence_finding_where_check_would` in `crates/aplan`,
/// which runs the real loader over a real repository.
#[test]
fn a_rule_that_names_a_field_places_its_finding_on_it() {
    let mut wrong = Vec::new();
    let mut checked = 0_usize;

    for s in corpus() {
        if s.expect != Expect::Fires {
            continue;
        }
        let Some(field) = s.rule.points_at() else {
            continue;
        };
        for d in full_diagnostics_for(&s) {
            if d.code != s.rule.code() {
                continue;
            }
            let Some(file) = &d.file else {
                continue;
            };
            // The scenario holds the text it wrote, so the expected line comes
            // from the fixture rather than from a number typed here.
            let Some((_, contents)) = s.files.iter().find(|(rel, _)| file.ends_with(rel)) else {
                continue;
            };
            // Found by searching the file directly, and deliberately *not* with
            // `field_lines`. The first draft of this audit computed the
            // expectation with the same function it was checking, so shifting
            // every line by one moved both sides together and the audit passed
            // the one mutation it exists to catch.
            let key = format!("{field}:");
            let mut hits = contents
                .lines()
                .enumerate()
                .filter(|(_, l)| l.starts_with(&key))
                .map(|(n, _)| n + 1);
            let (Some(want), None) = (hits.next(), hits.next()) else {
                continue;
            };

            checked += 1;
            if d.location.map(|l| l.line) != Some(want) {
                wrong.push(format!(
                    "{} ({}) declares `{field}` and should be at line {want}, but is at {:?}",
                    s.rule.code(),
                    s.name,
                    d.location.map(|l| l.line)
                ));
            }
        }
    }

    assert!(
        checked > 12,
        "only {checked} finding(s) were placed, so this audit is not reaching the corpus"
    );
    assert!(
        wrong.is_empty(),
        "{} finding(s) are not on the field their rule names:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}
