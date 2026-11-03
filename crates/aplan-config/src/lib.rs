//! `.aplan.yml`: the repository's policy, in one schema-derived type.
//!
//! This crate owns two files, and the line between them is load-bearing.
//! `.aplan.yml` is the repository's: shared, committed, binding on everybody
//! who works the checkout. [`user`] is the per-user one, which is about the
//! *actor* and holds nothing any rule reads -- so it cannot weaken what a
//! repository requires, by construction rather than by a precedence rule
//! (`A1-364`).
//!
//! A repo declares its policy once, in `.aplan.yml`, rather than threading flags
//! through CI. The `Config` type is the single source of that schema, exactly as
//! `aplan_core::task::Task` is for a task: `schemas/v1/config.json` is *derived*
//! from it (`cargo xtask gen-schema`) and gated by `--check`.
//!
//! Two rules give the file teeth:
//!
//! - it is `deny_unknown_fields`, so a key the tool does not understand is a
//!   reported error, never silently ignored. A config the tool ignores is drift
//!   on day one, the exact failure `aplan` exists to catch.
//! - the `test` runner is the trust boundary (HLD §5.5): it is the only place a
//!   command may be named, never a task file. [`TestRunner`] carries that
//!   guarantee (a whole-element placeholder, validated at load, spawned with no
//!   shell), so it lives here, in trusted top-level config, and nowhere else.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use aplan_core::EvidenceKind;
use aplan_core::capture::{CapturePolicy, CaptureProfile};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The whole-element token replaced by the test name in a `run` template.
pub const PLACEHOLDER: &str = "{test}";

/// The whole-element token replaced by an aplan-owned batch manifest path.
pub const BATCH_MANIFEST_PLACEHOLDER: &str = "{manifest}";

/// The whole-element token replaced by an initially absent batch-result path.
pub const BATCH_RESULT_PLACEHOLDER: &str = "{result}";

/// The batch interchange major version this build accepts.
pub const TEST_BATCH_PROTOCOL: u32 = 1;

/// The whole-element token replaced by the citation in a `url.check` template.
pub const URL_PLACEHOLDER: &str = "{url}";

/// The config file, at the repository root.
pub mod user;

pub const CONFIG_FILE: &str = ".aplan.yml";

/// Which tags name a release when `version.release_tag` says nothing (ADR-0038).
///
/// `v*` because it is what this project and its pilots tag with. It is a default
/// rather than a rule: the glob exists so a repository that tags
/// `release-2026-09` is not told its vocabulary is wrong (`A1-037`).
pub const DEFAULT_RELEASE_TAG: &str = "v*";

/// A failure loading or validating `.aplan.yml`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid test runner in `.aplan.yml`: {0}")]
    BadRunner(String),
    /// A `url.check` that could not be acted on. Distinct from
    /// [`BadRunner`](Self::BadRunner) so the message names the key that is
    /// wrong: reporting a url problem as an "invalid test runner" sends the
    /// reader to the wrong half of their config.
    #[error("invalid `url.check` in `.aplan.yml`: {0}")]
    BadUrlCheck(String),
    /// A reusable-provider input declaration the observation store cannot
    /// identify exactly.
    #[error("invalid `evidence.observation_inputs` in `.aplan.yml`: {0}")]
    BadObservationInputs(String),
    /// An `advisory` entry that names no rule.
    #[error("invalid `advisory` in `.aplan.yml`: {0}")]
    BadAdvisory(String),
    /// A `policy` entry the tool cannot act on (`A1-183`).
    #[error("invalid `policy` entry: {0}")]
    BadPolicy(String),
    /// A `effort_days_per_week` nothing can be divided by (`A1-147`). Its own
    /// variant for the reason [`BadUrlCheck`](Self::BadUrlCheck) has one:
    /// reporting a throughput as an "invalid test runner" sends the reader to
    /// the wrong half of their config, which this key did until the corpus
    /// printed the message next to the input.
    #[error("invalid `effort_days_per_week` in `.aplan.yml`: {0}")]
    BadThroughput(String),
    /// A `docs.membership` entry the check could not be run from. Its own
    /// variant for the reason [`BadUrlCheck`](Self::BadUrlCheck) has one.
    #[error("invalid `docs.membership` in `.aplan.yml`: {0}")]
    BadDocs(String),
    /// A `docs.paths` declaration the document gate could not act on.
    #[error("invalid `docs.paths` in `.aplan.yml`: {0}")]
    BadDocPaths(String),
    /// A view grouping by a tag family the repository did not declare.
    #[error("invalid grouping in `.aplan.yml`: {0}")]
    BadGrouping(String),
    /// A `tag_families` vocabulary the tool cannot act on.
    #[error("invalid `tag_families` in `.aplan.yml`: {0}")]
    BadTagFamilies(String),
    /// An `owners` vocabulary the tool cannot act on.
    #[error("invalid `owners` in `.aplan.yml`: {0}")]
    BadOwners(String),
    /// A `version` block the tool cannot act on (`A1-313`). Its own variant for
    /// the reason [`BadUrlCheck`](Self::BadUrlCheck) has one.
    #[error("invalid `version` in `.aplan.yml`: {0}")]
    BadVersion(String),
    /// Capture kinds or profiles the portable repository policy cannot apply.
    #[error("invalid `capture` in `.aplan.yml`: {0}")]
    BadCapture(String),
    #[error("could not read `.aplan.yml`: {0}")]
    Io(String),
    #[error("could not parse `.aplan.yml`: {0}")]
    Parse(String),
    /// A key this build does not accept, reported against the schema-derived
    /// inventory rather than by re-reading serde's text.
    ///
    /// Separate from [`Parse`](Self::Parse) because the two need different
    /// answers. A syntax error is a mistake in the file; an unknown key is
    /// *either* a typo *or* a key from a newer `aplan`, and the message that
    /// cannot tell those apart is the one the first adopter met: it read exactly
    /// like a misspelling while being a binary one release behind
    /// (`A1-068`, pre-public adoption evidence).
    #[error("{0}")]
    UnknownKeys(UnknownKeys),
}

/// The keys a file used that this build does not accept, with a near match each
/// where there is one.
#[derive(Debug, Clone)]
pub struct UnknownKeys {
    /// The file the keys came from, for the message.
    pub path: String,
    /// `(dotted key, nearest accepted key if one is a single edit away)`.
    pub keys: Vec<(String, Option<String>)>,
    /// What the file still fails on once these keys are removed, if anything.
    ///
    /// Carried so the adopter is not told half the answer twice: `serde` stops
    /// at its first error, so a type error further down the file would otherwise
    /// wait until they had fixed these and run again.
    pub remaining: Option<String>,
}

impl std::fmt::Display for UnknownKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: ", self.path)?;
        let plural = if self.keys.len() == 1 { "key" } else { "keys" };
        write!(f, "unknown {plural} ")?;
        for (i, (key, near)) in self.keys.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "`{key}`")?;
            // The typo case leads, because it is the common one and this must not
            // weaken it: an adopter who mistyped wants the correction, not a
            // meditation on binary versions.
            if let Some(near) = near {
                write!(f, " (did you mean `{near}`?)")?;
            }
        }
        if let Some(remaining) = &self.remaining {
            let that = if self.keys.len() == 1 {
                "that is"
            } else {
                "those are"
            };
            write!(
                f,
                "; and once {that} fixed the file still fails: {remaining}"
            )?;
        }
        Ok(())
    }
}

/// The parsed `.aplan.yml`. Every field is optional: an absent key means "use
/// the built-in default", and a CLI flag overrides whatever is here (the CLI
/// owns precedence). Unknown keys are rejected, not ignored.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Portable capture kinds and named creation/promotion profiles.
    ///
    /// This is repository policy, never per-user configuration: the same
    /// capture request must persist the same effective values for every actor.
    /// Profiles contain only inert defaults and cannot set task status,
    /// dependencies, evidence, traces or commands.
    #[serde(default)]
    pub capture: Option<CaptureConfig>,
    /// What a task is expected to say about the released version (`A1-312`,
    /// ADR-0034). Absent by default, and a plan that never declares it is
    /// unaffected.
    #[serde(default)]
    pub version: Option<VersionPolicy>,
    /// Require an `effort_days` estimate on active tasks (`APLAN009`). Default:
    /// `true`. The `--no-require-effort` flag overrides this to `false`.
    #[serde(default)]
    pub require_effort: Option<bool>,
    /// Resolve `traces` anchors (`APLAN008`) against this root. Default: the
    /// current directory. The `--traces-root` flag overrides it.
    #[serde(default)]
    pub traces_root: Option<String>,
    /// Skip `traces` checking entirely. Default: `false`. The `--no-traces` flag
    /// overrides it.
    #[serde(default)]
    pub no_traces: Option<bool>,
    /// Roots outside this repository that a `traces` target may name (`A1-143`,
    /// ADR-0021). Resolved against `traces_root`, exactly as a trace target is,
    /// so `["../p0000007"]` means the sibling beside the root the traces
    /// themselves are read from.
    ///
    /// Absent, and a trace that escapes the repository is refused (`APLAN034`).
    /// The refusal is the default because such a trace resolves *fully* -- a bad
    /// anchor is caught, drift fires across the boundary -- whenever the sibling
    /// happens to be checked out, and reports a per-task `APLAN008` on the day a
    /// job runs without it. That is an undeclared hard dependency on another
    /// repository's presence and ref, which is the class of failure this tool
    /// exists to remove: a plan that passes on one machine and not another, for
    /// a reason the plan does not state.
    ///
    /// Declaring one makes the dependency the repository's stated position: its
    /// absence is then a configuration error naming this key, rather than a
    /// finding about a task that is fine.
    #[serde(default)]
    pub traces_external: Option<Vec<String>>,
    /// The test runner(s) for `aplan verify`. A single mapping, or a sequence
    /// when one command cannot list every test in the repository.
    #[serde(default)]
    pub test: Option<TestRunners>,
    /// How to check that a `url` citation is reachable (`APLAN016`).
    ///
    /// A configured command rather than a built-in HTTP client, for the reason
    /// `test` is: the tool must not decide on an adopter's behalf what touching
    /// the network means. It keeps the trust boundary where the repository can
    /// see it, adds no TLS or async runtime to a binary whose other work is
    /// hermetic, and makes the rule opt-in by construction -- no key, no
    /// network, which is what `check`'s hermetic contract requires.
    ///
    /// Timeout and redirect policy live in the command, where they are visible
    /// and the adopter's to set, rather than as knobs this tool invents.
    #[serde(default)]
    pub url: Option<UrlCheck>,
    /// Evidence-strength policy (`APLAN017`).
    #[serde(default)]
    pub evidence: Option<EvidenceConfig>,
    /// Policy scoped to a subset of the plan, narrowest match winning
    /// (`A1-183`).
    ///
    /// Every other dial here is whole-plan, and the one exception --
    /// `evidence.exempt_tags` -- is the degenerate case of this: "required
    /// strength is *none* for these tags". A repository that wants stricter
    /// evidence on a release cut than on ordinary work, or none on spikes, can
    /// only say it by exemption today, which is all or nothing.
    ///
    /// The subset is named with the same five dimensions `--cut`, `--phase`,
    /// `--tag`, `--owner` and `--status` take on the command line, with the same reading:
    /// within a dimension any, across dimensions all (ADR-0026's selector,
    /// `A1-214`). One model of what a subset is, two renderings of it.
    #[serde(default)]
    pub policy: Option<Vec<ScopedPolicy>>,
    /// Rule codes this repository hears without failing its build on them.
    ///
    /// The severity stays the rule's own: `APLAN022` is an error in `facts` and
    /// in the catalogue whatever is listed here. What this changes is the
    /// *verdict*, which is where `deny_warnings` already lives. It exists because
    /// the check most useful during a migration was the one a migration could not
    /// turn on until it was finished: four tasks of thirty-three without a cut
    /// kept `APLAN022` off for the other twenty-nine
    /// (pre-public adoption evidence, item 3, `A1-112`).
    ///
    /// Every finding downgraded this way is still printed, and the run says how
    /// many there were, because a build that passed on a downgraded rule must not
    /// read like one that had nothing to say.
    #[serde(default)]
    pub advisory: Option<Vec<String>>,
    /// Turn on stale-`done` drift detection (`APLAN018`, `APLAN054`): `aplan
    /// close` stamps traced sections and mutable file evidence, then `check` and
    /// `verify` report changes under the same references. Off unless set.
    #[serde(default)]
    pub detect_drift: Option<bool>,
    /// Require every non-milestone task to be a member of exactly one cut
    /// (`APLAN022`): in exactly one milestone's `blocked_by`, unless the task is
    /// tagged `cross-cutting`. This makes a milestone's `done` prove the cut is
    /// *exhaustively* complete, not just that its listed members are. Off unless
    /// set; only meaningful once the plan uses milestone meta-tasks (ADR-0008).
    #[serde(default)]
    pub require_cut_membership: Option<bool>,
    /// Tag vocabularies this plan uses, as family name to its members
    /// (`APLAN026`).
    ///
    /// A second task-membership dimension ends up in tags, because tags are
    /// what exists: a pilot put 22 epics there and a build/measure/review
    /// taxonomy beside them, and nothing noticed a typo in either. Declaring
    /// the vocabulary makes a claim the repository already believes checkable,
    /// without making tags into a containment model.
    ///
    /// ```yaml
    /// tag_families:
    ///   epic: [epic-a, epic-b]
    ///   kind: [kind-b, kind-m, kind-r]
    /// ```
    ///
    /// Absent, nothing changes. Tags outside every declared family stay legal
    /// and silent, or the feature would be a migration.
    #[serde(default)]
    pub tag_families: Option<BTreeMap<String, TagFamily>>,
    /// Canonical values for the task model's free-text `owner` field.
    ///
    /// Absent leaves every owner legal, exactly as before. When declared,
    /// `APLAN056` reports a task whose owner is not a member and suggests a
    /// declared spelling when it is one edit away. This is deliberately a
    /// field-specific vocabulary rather than a generic mapping: `owner` is the
    /// only free-text categorical scalar in the task model today, and a generic
    /// mechanism with one member would expose field names as a second schema.
    #[serde(default)]
    pub owners: Option<Vec<String>>,
    /// How long a live task may sit unchanged before `aplan stale` names it,
    /// in days (`A1-020`). Default 30; `--older-than` overrides it.
    ///
    /// A window, not a threshold for failing anything: the command reports and
    /// never blocks, because a task file's date is a proxy for the work's state.
    #[serde(default)]
    pub stale_after_days: Option<u32>,
    /// Effort days this repository absorbs in a week, for the *derived*
    /// duration `summary` and `roadmap` report (`A1-147`, ADR-0023).
    ///
    /// One ratio, declared once, and nothing else about time: no start date, no
    /// calendar, no per-cut duration. A duration is then arithmetic over numbers
    /// the plan already carries -- `effort_days` and cut membership -- so it
    /// cannot drift from them, which is the failure the sixth pilot measured
    /// three times in nine audits of its own hand-written schedule
    /// (pre-public adoption evidence item 2).
    ///
    /// It is a *capacity*, not a promise: `7.5` says "five days at an assumed
    /// 1.5x", and the derived weeks say what that capacity implies about the
    /// effort in the plan. Absent, nothing is reported.
    #[serde(default)]
    pub effort_days_per_week: Option<f64>,
    /// How `aplan roadmap` renders (`A1-078`).
    #[serde(default)]
    pub roadmap: Option<RoadmapConfig>,
    /// How `aplan summary` rolls work up (`A1-186`).
    #[serde(default)]
    pub summary: Option<SummaryConfig>,
    /// How `aplan add` mints an id when the caller does not pass `--id`
    /// (`A1-047`). Default: `sequential`, the derivation every existing plan
    /// already gets.
    ///
    /// `ulid` is for a repository with authors working offline in parallel,
    /// where two of them derive the same next id and find out at the merge.
    /// It costs readability -- a ULID is 26 characters nobody types from memory
    /// -- so it is a trade a repository makes once, knowingly, rather than a
    /// default that arrives with an upgrade.
    #[serde(default)]
    pub id_strategy: Option<aplan_core::IdStrategy>,
    /// Directory holding the plan's task files. Default: `aplan`. A relative value
    /// is resolved against this config file's directory, so `root: plan` means
    /// `<repo>/plan` wherever the command runs. The `--root` flag overrides it.
    #[serde(default)]
    pub root: Option<String>,
    /// Which prose `aplan check-docs` checks, and where that prose declares a
    /// cut's membership (`A1-165`, `A1-203`).
    #[serde(default)]
    pub docs: Option<DocsConfig>,
    /// Treat any warning as a finding that fails the command, so `check` and
    /// `verify` exit `1` rather than `0` when a plan has warnings and nothing
    /// worse. Off unless set.
    ///
    /// For a repository whose policy is that every gate either passes or blocks:
    /// the advisory tier is useful by default (`APLAN023` warns because a
    /// citation may be perfectly honest and the repository is the judge of that),
    /// and this is how a repository says it would rather be the judge in the
    /// strict direction. The exit code is unchanged: `1` still means "at least
    /// one finding", not a new code a consumer's CI has to learn (ADR-0007).
    #[serde(default)]
    pub deny_warnings: Option<bool>,
}

/// Repository-owned capture organization and portable defaults.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfig {
    /// Additional lower-case ASCII kebab-case kinds. `task`, `idea` and `note`
    /// are always present and must not be repeated here.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Explicitly selected profiles; there is no implicit personal default.
    #[serde(default)]
    pub profiles: BTreeMap<String, CaptureProfile>,
}

impl Config {
    /// Resolve the selected repository capture policy, or the built-ins when
    /// the key is absent (including `--no-config`).
    ///
    /// The loader has already validated this shape, so failure here indicates
    /// a programmatically constructed `Config` that skipped [`Config::validate_capture`].
    #[must_use]
    pub fn capture_policy(&self) -> CapturePolicy {
        self.capture
            .as_ref()
            .map_or_else(CapturePolicy::default, |config| {
                CapturePolicy::try_new(&config.kinds, &config.profiles)
                    .expect("loaded capture policy is valid")
            })
    }

    /// Reject capture policy that is ambiguous, vacuous or non-portable.
    ///
    /// # Errors
    /// When a configured kind, profile or one of its admitted defaults is not
    /// a value the capture loader can interpret deterministically.
    pub fn validate_capture(&self) -> Result<(), Error> {
        let Some(config) = &self.capture else {
            return Ok(());
        };
        CapturePolicy::try_new(&config.kinds, &config.profiles)
            .map(|_| ())
            .map_err(Error::BadCapture)
    }
}

/// A declared tag vocabulary: its members, and optionally how many of them a
/// task must carry.
///
/// Untagged so the list shorthand keeps working unchanged. `epic: [epic-a]` and
/// `epic: {members: [epic-a]}` are the same declaration, and a repository that
/// never wanted cardinality never learns the longer form exists (`A1-133`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum TagFamily {
    /// `epic: [epic-a, epic-b]`, the shape every existing config uses.
    Members(Vec<String>),
    /// The same, plus a cardinality rule and the tasks it applies to.
    Spec(TagFamilySpec),
}

// Hand-written rather than `#[derive(Deserialize)]` with `untagged`, because
// `untagged` throws the inner error away. Every mistake inside the long form,
// an unknown `require` value, a misspelled key, a `members` that is not a list,
// came back as "data did not match any variant of untagged enum TagFamily",
// which names neither the key nor the value and is the same sentence for all
// three. Dispatching on the shape by hand keeps `serde`'s own message, so an
// unknown `require` reports the variants it expected (`A1-133`).
impl<'de> serde::Deserialize<'de> for TagFamily {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Shape;
        impl<'de> serde::de::Visitor<'de> for Shape {
            type Value = TagFamily;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a list of tags, or a mapping with `members`")
            }

            fn visit_seq<A>(self, seq: A) -> Result<TagFamily, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                serde::Deserialize::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))
                    .map(TagFamily::Members)
            }

            fn visit_map<A>(self, map: A) -> Result<TagFamily, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                serde::Deserialize::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(TagFamily::Spec)
            }
        }
        deserializer.deserialize_any(Shape)
    }
}

impl TagFamily {
    /// The declared members, whichever shape declared them.
    #[must_use]
    pub fn members(&self) -> &[String] {
        match self {
            TagFamily::Members(m) => m,
            TagFamily::Spec(s) => &s.members,
        }
    }

    /// How many members a task in scope must carry.
    #[must_use]
    pub fn require(&self) -> Cardinality {
        match self {
            TagFamily::Members(_) => Cardinality::Optional,
            TagFamily::Spec(s) => s.require.unwrap_or(Cardinality::Optional),
        }
    }

    /// Which tasks the cardinality applies to.
    #[must_use]
    pub fn scope(&self) -> (StatusScope, MilestoneScope) {
        match self {
            TagFamily::Members(_) => (StatusScope::Any, MilestoneScope::Include),
            TagFamily::Spec(s) => (
                s.status.unwrap_or(StatusScope::Any),
                s.milestones.unwrap_or(MilestoneScope::Include),
            ),
        }
    }
}

/// The long form of a [`TagFamily`].
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TagFamilySpec {
    /// The tags this family contains, as whole tags.
    pub members: Vec<String>,
    /// How many of them a task in scope carries. Absent means `optional`, which
    /// is what a bare list declares.
    #[serde(default)]
    pub require: Option<Cardinality>,
    /// Which statuses the requirement applies to. Absent means every status.
    #[serde(default)]
    pub status: Option<StatusScope>,
    /// Whether milestone tasks are subject to it. Absent means they are.
    #[serde(default)]
    pub milestones: Option<MilestoneScope>,
}

/// How many members of a family a task in scope must carry (`APLAN030`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Cardinality {
    /// Any number, including none. What a bare list declares, and the default.
    Optional,
    /// At most one, so two members of the same family is a finding.
    AtMostOne,
    /// At least one, so a task carrying none is a finding.
    AtLeastOne,
    /// Exactly one, which is `at-most-one` and `at-least-one` together.
    ExactlyOne,
}

/// Which statuses a cardinality applies to.
///
/// Two separate keys rather than a list of predicates, so a contradiction cannot
/// be written: `[live, done]` would be a rule that can never fire, which is the
/// vacuity this project keeps finding in other people's gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StatusScope {
    /// Every status, the default.
    Any,
    /// `todo`, `in_progress` and `blocked`: work that has not finished.
    Live,
    /// `done` only.
    Done,
    /// `dropped` only.
    Dropped,
    /// `done` or `dropped`.
    Terminal,
}

/// Whether milestone tasks are subject to a cardinality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MilestoneScope {
    /// Milestones are judged like any other task, the default.
    Include,
    /// Milestones are exempt, for a vocabulary that classifies work.
    Exclude,
    /// Only milestones, for a vocabulary that classifies cuts.
    Only,
}

/// A policy that applies to part of the plan rather than all of it (`A1-183`).
///
/// **Narrowest match wins, and among equals the first entry in the file.**
/// Narrowness is the count of dimensions the scope names, which is the usual
/// reading of "more specific"; the whole-plan settings are the scope that names
/// none, so they lose to any scope that matches. Reading down the list is how a
/// reader resolves a tie, which is a rule that needs no merge.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopedPolicy {
    /// The subset this applies to. An empty scope is refused rather than read as
    /// "everything": the whole-plan keys already say that, and a scope that
    /// silently became global is the shape a reader would least expect.
    pub when: Scope,
    /// The dials this scope sets. Absent keys fall through to the next widest
    /// scope that matches, and finally to the whole-plan value.
    #[serde(default)]
    pub evidence: Option<ScopedEvidence>,
}

/// The subset a [`ScopedPolicy`] applies to: the five dimensions `--cut`,
/// `--phase`, `--tag`, `--owner` and `--status` name on the command line (`A1-214`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Cut labels or milestone ids. A task matches if it is in any of them.
    #[serde(default)]
    pub cut: Vec<String>,
    /// Numeric phases. A task matches if it carries any of them.
    #[serde(default)]
    pub phase: Vec<u32>,
    /// Tags. A task matches if it carries any of them.
    #[serde(default)]
    pub tag: Vec<String>,
    /// Owners. A task matches if any is its owner.
    #[serde(default)]
    pub owner: Vec<String>,
    /// Statuses. A task matches if it is in any of them.
    #[serde(default)]
    pub status: Vec<String>,
}

impl Scope {
    /// The subset this names, as the selector every other surface uses.
    ///
    /// # Errors
    ///
    /// When a `status` is not one the model has, listed so the reader does not
    /// have to find them. The command line refuses `--status donn` by name, and
    /// this refusing it differently -- or not at all -- is the drift `A1-183`
    /// exists to remove. Dropping an unparseable status silently was worse than
    /// inconsistent: the dimension stayed *named* for precedence but constrained
    /// nothing, so a typo widened the scope to the whole plan and a policy meant
    /// for spikes switched the requirement off everywhere (`A1-183`'s audit).
    pub fn selector(&self) -> Result<aplan_core::select::Selector, Error> {
        let mut statuses = Vec::new();
        for word in &self.status {
            let parsed = aplan_core::Status::ALL
                .iter()
                .find(|s| s.as_str() == word)
                .ok_or_else(|| {
                    let known: Vec<String> = aplan_core::Status::ALL
                        .iter()
                        .map(|s| format!("`{}`", s.as_str()))
                        .collect();
                    Error::BadPolicy(format!(
                        "`when.status` has `{word}`, which is not a status: {}",
                        known.join(", ")
                    ))
                })?;
            statuses.push(*parsed);
        }
        Ok(aplan_core::select::Selector {
            cuts: self.cut.clone(),
            phases: self.phase.clone(),
            tags: self.tag.clone(),
            owners: self.owner.clone(),
            statuses,
            // A `policy` scope says which work is judged to which standard, and
            // "whoever happens to hold it right now" is not that: a claim moves
            // between actors while the standard the work is held to does not.
            // So the config side does not offer it (`A1-359`).
            unclaimed: false,
        })
    }

    /// How many dimensions this names, which is what "narrowest" counts.
    #[must_use]
    pub fn named_dimensions(&self) -> usize {
        usize::from(!self.cut.is_empty())
            + usize::from(!self.phase.is_empty())
            + usize::from(!self.tag.is_empty())
            + usize::from(!self.owner.is_empty())
            + usize::from(!self.status.is_empty())
    }
}

/// The evidence dials a scope may set.
///
/// Only the two that `evidence.exempt_tags` already scopes between them, because
/// the first scoped dial should be the one whose degenerate case is already in
/// the file rather than a new idea (`A1-183`).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopedEvidence {
    /// Evidence kinds strong enough to close a task in this subset. An empty
    /// list is "no requirement here", which is what `exempt_tags` says globally.
    #[serde(default)]
    pub require_min_strength: Option<Vec<EvidenceKind>>,
    /// Tags that exempt a task in this subset from the requirement.
    #[serde(default)]
    pub exempt_tags: Option<Vec<String>>,
}

/// How strong a `done` task's evidence must be (`APLAN017`), and which tasks are
/// exempt because nothing stronger exists.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceConfig {
    /// Evidence kinds strong enough to close a task. A `done` task must carry at
    /// least one of these; otherwise `APLAN017` fires. Absent leaves the rule off.
    #[serde(default)]
    pub require_min_strength: Option<Vec<EvidenceKind>>,
    /// Tags that exempt a task from the requirement (a phone call, a regulator
    /// letter, where a dated `note` is genuinely the best evidence there is).
    #[serde(default)]
    pub exempt_tags: Option<Vec<String>>,
    /// How many paths a cited commit may touch *outside* a task's `paths` before
    /// `APLAN024` calls the citation diluted. Default 50; `0` turns the rule off.
    ///
    /// Paths rather than files: this counts the same set `APLAN012` calls
    /// "touched", which includes both sides of a rename.
    ///
    /// Counted outside rather than in total, deliberately: a large change wholly
    /// inside a task's `paths` is focused, and a small commit cannot swamp
    /// anything however little of it matches. What the rule is looking for is a
    /// citation that drags in a lot of unrelated work, which is what makes
    /// `APLAN012`'s overlap test pass by accident.
    #[serde(default)]
    pub max_unrelated_files: Option<usize>,
    /// Exhaustive repository-relative material inputs to configured evidence
    /// discovery, as glob patterns.
    ///
    /// This is a trust claim, not a performance hint. A persisted test
    /// catalogue is reusable only while every matched input, the provider
    /// configuration, inherited environment and configured executable
    /// identity are unchanged. Absent means provider observations are
    /// deliberately non-cacheable. Keep generated outputs out of this set: a
    /// provider changing its own declared input while it runs cannot publish a
    /// current observation.
    #[serde(default)]
    pub observation_inputs: Option<Vec<String>>,
}

/// One runner, or several.
///
/// Untagged so today's single mapping keeps parsing exactly as written: every
/// existing adopter, this repository included, has one, and a config change that
/// made them all edit their file would be a poor trade for a feature most of them
/// do not need.
///
/// Several is not an exotic case. An umbrella repository of independent projects
/// (no root workspace, so no `cargo test` sees more than one), a polyglot tree
/// with a runner per language, and a repository separating unit from e2e runs all
/// hit the same wall: one `list` command cannot see the whole corpus, so most
/// tasks cannot cite `test` evidence at all and fall back to weaker kinds
/// (`A1-054`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
// One of these exists per process, built once from `.aplan.yml` and read for the
// rest of the run, so the variants differing by a couple of hundred bytes costs
// nothing measurable -- and boxing the common case would put an indirection in
// front of every runner lookup to save it. Allowed knowingly rather than
// silenced: it grew past the threshold when `A1-310` added the limit keys.
#[allow(clippy::large_enum_variant)]
pub enum TestRunners {
    /// The original form: exactly one runner for the whole repository.
    One(TestRunner),
    /// One runner per component, language, or test kind.
    Many(Vec<TestRunner>),
    /// Explicit scheduling policy and the runners it governs. The legacy
    /// mapping and sequence forms remain serial; concurrency is opt-in.
    Scheduled(TestRunnerGroup),
}

/// One aggregate scheduling budget over every configured test-provider stage.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestSchedule {
    /// Weighted capacity available to provider work. Every runner consumes its
    /// declared `weight` (one by default) for the lifetime of one command.
    #[schemars(range(min = 1))]
    pub capacity: u32,
}

impl Default for TestSchedule {
    fn default() -> Self {
        Self { capacity: 1 }
    }
}

/// The opt-in scheduled form of `test` configuration.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestRunnerGroup {
    /// Aggregate weighted provider budget.
    pub schedule: TestSchedule,
    /// Configured runners in deterministic evidence-identity order.
    #[schemars(length(min = 1))]
    pub runners: Vec<TestRunner>,
}

impl<'de> Deserialize<'de> for TestRunners {
    /// Dispatch on the shape rather than deriving `untagged`.
    ///
    /// A derived untagged enum reports only "data did not match any variant of
    /// untagged enum `TestRunners`" when a runner has a typo in it, which loses
    /// the one thing the author needs: the name of the offending key. Deciding
    /// on sequence-versus-mapping first means the inner `deny_unknown_fields`
    /// error is the one that surfaces.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let value = yaml_compat::Value::deserialize(deserializer)?;
        if value.is_sequence() {
            Vec::<TestRunner>::deserialize(value)
                .map(TestRunners::Many)
                .map_err(D::Error::custom)
        } else if value.as_mapping().is_some_and(|mapping| {
            mapping.contains_key(yaml_compat::Value::String("runners".into()))
                || mapping.contains_key(yaml_compat::Value::String("schedule".into()))
        }) {
            TestRunnerGroup::deserialize(value)
                .map(TestRunners::Scheduled)
                .map_err(D::Error::custom)
        } else {
            TestRunner::deserialize(value)
                .map(TestRunners::One)
                .map_err(D::Error::custom)
        }
    }
}

impl TestRunners {
    /// Every runner, in the order the file declares them.
    #[must_use]
    pub fn all(&self) -> &[TestRunner] {
        match self {
            TestRunners::One(r) => std::slice::from_ref(r),
            TestRunners::Many(rs) => rs,
            TestRunners::Scheduled(group) => &group.runners,
        }
    }

    /// The one aggregate budget governing provider work. Existing mapping and
    /// sequence forms deliberately remain serial for compatibility.
    #[must_use]
    pub fn schedule(&self) -> TestSchedule {
        match self {
            Self::One(_) | Self::Many(_) => TestSchedule::default(),
            Self::Scheduled(group) => group.schedule,
        }
    }

    /// Whether the repository opted into overlapping provider stages.
    #[must_use]
    pub fn has_explicit_schedule(&self) -> bool {
        matches!(self, Self::Scheduled(_))
    }

    /// Reject any runner that could not receive a test name safely, and a
    /// sequence with nothing in it.
    ///
    /// An empty `test: []` parses, and would then reject every `test` citation
    /// with a message implying a runner exists. A config that cannot do anything
    /// is a config error, not a silent no-op.
    ///
    /// # Errors
    ///
    /// [`Error::BadRunner`] for an empty sequence, or the first invalid runner's.
    pub fn validate(&self) -> Result<(), Error> {
        if self.all().is_empty() {
            return Err(Error::BadRunner(
                "`test` is an empty list; remove the key, or give it a runner".into(),
            ));
        }
        if !self.has_explicit_schedule()
            && self
                .all()
                .iter()
                .any(|runner| runner.weight.is_some() || runner.exclusive.is_some())
        {
            return Err(Error::BadRunner(
                "`test.weight` and `test.exclusive` require the explicit `test.schedule` / `test.runners` form"
                    .into(),
            ));
        }
        for r in self.all() {
            r.validate()?;
            if r.weight() > self.schedule().capacity {
                return Err(Error::BadRunner(format!(
                    "`test` runner weight {} exceeds `test.schedule.capacity` {}",
                    r.weight(),
                    self.schedule().capacity
                )));
            }
        }
        Ok(())
    }
}

/// One complete adapter invocation for every exact test this runner must execute.
///
/// The adapter consumes an aplan-owned JSON manifest and publishes the
/// versioned JSON result. Test names never enter the command line. Framework
/// selection and native report parsing stay in the adapter; aplan validates the
/// complete requested/result set before accepting any outcome.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestBatch {
    /// Protocol major. Only [`TEST_BATCH_PROTOCOL`] is accepted by this build.
    #[schemars(range(min = 1, max = 1))]
    pub protocol: u32,
    /// Adapter argv. Each path placeholder occurs exactly once as a whole
    /// non-command argument.
    #[schemars(length(min = 1))]
    pub run: Vec<String>,
}

impl TestBatch {
    fn validate(&self) -> Result<(), Error> {
        if self.protocol != TEST_BATCH_PROTOCOL {
            return Err(Error::BadRunner(format!(
                "`test.batch.protocol` must be {TEST_BATCH_PROTOCOL} (found {})",
                self.protocol
            )));
        }
        if self.run.is_empty() {
            return Err(Error::BadRunner(
                "`test.batch.run` is an empty command".into(),
            ));
        }
        if self.run.iter().any(|argument| argument.trim().is_empty()) {
            return Err(Error::BadRunner(
                "`test.batch.run` contains a blank argument".into(),
            ));
        }
        for placeholder in [BATCH_MANIFEST_PLACEHOLDER, BATCH_RESULT_PLACEHOLDER] {
            let count = self
                .run
                .iter()
                .filter(|argument| argument.as_str() == placeholder)
                .count();
            if count != 1 {
                return Err(Error::BadRunner(format!(
                    "`test.batch.run` must contain `{placeholder}` exactly once, as a whole argument (found {count})"
                )));
            }
            if self.run.first().map(String::as_str) == Some(placeholder) {
                return Err(Error::BadRunner(format!(
                    "`{placeholder}` may not be the command (the first element) of `test.batch.run`"
                )));
            }
            if self
                .run
                .iter()
                .any(|argument| argument.as_str() != placeholder && argument.contains(placeholder))
            {
                return Err(Error::BadRunner(format!(
                    "`test.batch.run` may use `{placeholder}` only as a whole argument, never spliced inside another one"
                )));
            }
        }
        if self
            .run
            .iter()
            .any(|argument| argument.contains(PLACEHOLDER))
        {
            return Err(Error::BadRunner(format!(
                "`test.batch.run` may not contain `{PLACEHOLDER}`; exact names belong only in the manifest"
            )));
        }
        Ok(())
    }

    /// Substitute only the two standalone aplan-owned path arguments.
    #[must_use]
    pub fn run_argv(&self, manifest: &str, result: &str) -> Vec<String> {
        self.run
            .iter()
            .map(|argument| match argument.as_str() {
                BATCH_MANIFEST_PLACEHOLDER => manifest.to_string(),
                BATCH_RESULT_PLACEHOLDER => result.to_string(),
                _ => argument.clone(),
            })
            .collect()
    }
}

/// How to list and run tests, for `aplan verify`. Commands are argv vectors,
/// spawned without a shell (HLD section 5.5).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestRunner {
    /// Aggregate scheduler capacity consumed while one of this runner's
    /// provider commands is active. Default: 1. Raise it for harnesses that
    /// fan out internally so top-level concurrency accounts for their cost.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub weight: Option<u32>,
    /// Named capacity-one resources this runner must hold exclusively. The
    /// scheduler acquires the complete sorted set atomically, so two runners
    /// cannot overlap on a fixed port, database, compiler lock or fixture.
    #[serde(default)]
    pub exclusive: Option<Vec<String>>,
    /// What to call this runner in a diagnostic. Optional, and only useful when
    /// there is more than one: an ambiguous test name has to be able to say
    /// which runners reported it.
    #[serde(default)]
    pub name: Option<String>,
    /// Directory to run `list` and `run` in, relative to the repository root.
    /// Absent means the repository root.
    ///
    /// Part of the feature rather than an extra: a JavaScript or Python runner
    /// usually has to run inside its package directory, and without this the
    /// multi-runner support would cover Rust and little else.
    #[serde(default)]
    pub dir: Option<String>,
    /// Path globs this runner's tests cover, for `APLAN025`.
    ///
    /// Separate from [`dir`](Self::dir) because *where a runner runs* is not
    /// *what it covers*: `cargo test --manifest-path server/Cargo.toml`, an npm
    /// workspace filter, and `pytest --rootdir` all select a component while
    /// running at the repository root, which is usually where the lockfile is.
    /// Reading `dir` alone left `APLAN025` unable to fire on any real pilot
    /// (`A1-065`).
    ///
    /// Takes precedence over `dir` when both are set, being the more specific
    /// claim. Absent, and with no `dir` either, the runner makes no claim about
    /// locality and `APLAN025` stays silent.
    #[serde(default)]
    pub covers: Option<Vec<String>>,
    /// Paths covered by *some* of this runner's tests, named by test.
    ///
    /// One runner's tests do not always cover one thing. A repository whose only
    /// harness that can read the tree is a tools crate keeps its repository-wide
    /// invariants there: those tests assert facts about directories they do not
    /// live in, while the rest of the same crate's tests cover the crate. A
    /// single `covers` cannot say both, and saying the coarse thing made
    /// `APLAN025` report proof as borrowed (`A1-069`).
    ///
    /// The first entry whose `tests` match wins; anything unmatched falls back to
    /// `covers`, then `dir`.
    #[serde(default)]
    pub covers_by_test: Option<Vec<TestCoverage>>,
    /// Exhaustive repository inputs for subsets of this runner's tests.
    ///
    /// This is deliberately separate from `covers_by_test`: coverage says what
    /// a passing test proves, while an input dependency says what can change
    /// that test's result. Incremental evidence refresh may reuse an earlier
    /// outcome only when every currently listed test matches exactly one entry.
    /// A missing or overlapping match widens to the complete runner.
    #[serde(default)]
    pub inputs_by_test: Option<Vec<TestInputDependency>>,
    /// argv that prints the tests this runner **will not run**, one per line, in
    /// the names `list` uses (`A1-300`).
    ///
    /// `--execute` reads a zero exit status as "the cited test passed", and a
    /// runner that *skips* a test exits zero too. `cargo test -- --exact <an
    /// ignored test>` prints `running 1 test`, `0 passed`, `1 ignored`, and
    /// exits `0`: the test was selected, was not run, and the citation reads as
    /// proof. `APLAN037` does not catch it and should not -- that rule asks
    /// whether a passing run is *indistinguishable from a miss*, and this one is
    /// plainly distinguishable; the runner said what it did.
    ///
    /// So the runner is asked, the same way `list` is asked, rather than this
    /// tool learning what "ignored" looks like in any particular harness. For
    /// cargo: `[cargo, test, -q, -p, <crate>, '--', --list, --ignored]`.
    ///
    /// **Declaring it is opting in.** A repository that deliberately cites a
    /// nightly-gated test and knows it simply leaves this out, and nothing
    /// changes. The same one-sided shape as `run_selects`.
    ///
    /// **What it cannot cover**: a test skipped at *runtime*, by a guard inside
    /// the test body, which no harness lists in advance because nothing knows
    /// until the test runs. What this asks about is a skip the harness declares
    /// -- `#[ignore]`, a collection marker, a nightly gate -- which is the kind
    /// a citation can be checked against before the run.
    #[serde(default)]
    pub skipped: Option<Vec<String>>,
    /// How many lines of a failed test's output a finding may carry. Default:
    /// eight; `0` suppresses it (`A1-365`).
    ///
    /// **Present by default, which is a decision with a recorded cost.** A pilot
    /// rejected two cited tests that failed only inside its nested-Podman
    /// runner, and the whole actionable output was "failed under `--execute`"
    /// with the instruction to make the test pass: both streams had been
    /// captured and thrown away, so the cause could only be found by entering
    /// the runner by hand. `test.list` has retained one line of stderr for a
    /// launch failure since it was written, and this is that precedent applied
    /// to the other half. An opt-in would reproduce the pilot's experience
    /// exactly: they met the opaque failure first, and would only then have
    /// discovered a key existed.
    ///
    /// The cost is that a CI log can contain a little output nobody explicitly
    /// asked for. Three things bound it: the default is small, every line goes
    /// through the sanitizer a generated document uses, and recognisable
    /// credential shapes are masked. Set `0` for a repository whose tests print
    /// things it would rather not publish -- redaction is a best-effort filter
    /// over shapes that look like credentials, not a secret scanner, and the
    /// upgrade guidance says so in those words.
    #[serde(default)]
    pub output_lines: Option<usize>,
    /// The most output from each of stdout and stderr that aplan retains in its
    /// own process, as a size (`1M`, `256K`). Default: `1M` (`A1-366`).
    ///
    /// This is not [`max_memory`](Self::max_memory): that bounds the child where
    /// cgroups are available, while this bounds the parent on every platform.
    /// Nor is it [`output_lines`](Self::output_lines), which bounds only the
    /// sanitized excerpt written to a finding after capture. The pipe is always
    /// drained so a command cannot deadlock; bytes past this limit are discarded
    /// while the first and last halves are retained, and a failure excerpt says
    /// how many bytes were dropped.
    ///
    /// The limit is per stream rather than shared between them. That keeps the
    /// result deterministic -- a scheduling race between the stdout and stderr
    /// reader threads cannot decide which stream receives the budget -- and
    /// bounds retained command output at twice this value in total.
    #[serde(default)]
    pub max_output: Option<String>,
    /// Whether this runner is expected to list any tests at all. Default: `true`.
    ///
    /// A runner that lists nothing is usually a mistake -- a `dir` that moved, a
    /// manifest path that is wrong -- and until `APLAN036` nothing said so: every
    /// citation against it failed later as "no runner lists this", which blames
    /// the citation. Sometimes it is not a mistake: a component that genuinely
    /// has no tests yet is correct and inert, and the first repository to
    /// configure five runners had two of them in exactly that state.
    ///
    /// Set `false` to say so. That is the whole point of the field: without it a
    /// repository has to choose between a permanent warning and deleting a runner
    /// it will need next month.
    #[serde(default)]
    pub expect_tests: Option<bool>,
    /// The most memory this runner's commands may hold, as a size (`4G`, `512M`)
    /// (`A1-310`, ADR-0033).
    ///
    /// Enforced by a cgroup, which sets `memory.max` **and** `memory.swap.max`:
    /// prototyped, a process under a 64MB `memory.max` alone allocated 3GB with
    /// `oom_kill` still `0`, because the pages went to swap. A cap that a box
    /// with swap turns into a paging target is not a cap.
    ///
    /// What a command spends is the harness's and varies by two orders of
    /// magnitude: measured on one machine, `cargo test` warm peaks at 156MB,
    /// `go test ./...` at 1.2GB, and `npm test` (vitest) at 33GB. Size it from
    /// what your own run reports rather than from a number here.
    #[serde(default)]
    pub max_memory: Option<String>,
    /// The most processes this runner's commands may have alive at once
    /// (`A1-310`, ADR-0033).
    ///
    /// Enforced as the cgroup's `pids.max`. **A breach is not a kill**: `fork()`
    /// returns `EAGAIN` and the command usually exits with its own error, so it
    /// is detected by reading the cgroup's `pids.events` afterwards rather than
    /// inferred from an exit status.
    #[serde(default)]
    pub max_processes: Option<u32>,
    /// argv prepended to every command this runner runs (`A1-310`, ADR-0033).
    ///
    /// The escape hatch beside the dials, and the shape HLD 5.5 already implies
    /// for `url.check`: `nice`, `taskset`, `cgexec`, a container runner, or a
    /// cgroup property the dials do not model. `[systemd-run, --user, --scope,
    /// -p, MemoryMax=4G, --]` is the whole of `max_memory` written by hand.
    ///
    /// Top-level config only, like every other command here: a task file must
    /// never be able to name one (HLD 5.5).
    #[serde(default)]
    pub wrap: Option<Vec<String>>,
    /// Whether this runner's `run` command selects exactly the name `list`
    /// printed. Default: unknown, and `APLAN037` finds out by asking.
    ///
    /// The rung `--execute` claims is that the cited test *passed*, and that is
    /// only worth what the runner makes it worth: a `run` command whose filter
    /// does not match the names `list` prints selects nothing, skips everything
    /// and exits `0`, which reads as a pass. `APLAN037` learns what a miss looks
    /// like by running a name no test can have, and compares.
    ///
    /// Set `true` where the runner is right and simply says nothing either way,
    /// which no probe can tell from a runner that ran nothing. It is a claim the
    /// repository makes and nothing checks -- so it also skips the probe's extra
    /// run of `test.run`, which is the honest trade: a claim in exchange for the
    /// cost of the question.
    ///
    /// `false` is the same as leaving it out. The key is a one-sided claim, like
    /// `expect_tests`: saying a runner does *not* select exactly is saying the
    /// probe is right, and silencing a rule on the strength of agreeing with it
    /// would be the one reading nobody wants.
    #[serde(default)]
    pub run_selects: Option<bool>,
    /// argv that prints the available test names to stdout, one per line. A
    /// trailing `: test` / `: benchmark` (cargo's `--list` format) is stripped.
    ///
    /// **This runs on a plain `verify`, not only under `--execute`**, because
    /// resolving a `test` citation means asking what a runner lists. For a cargo
    /// runner that *compiles*: measured, a plain `verify` on a crate with no
    /// `target/` left one behind, and tests that do not compile surface as
    /// `APLAN013` about the runner. So the gate needs whatever your build needs
    /// -- a workspace that path-depends on a sibling checkout fails here if the
    /// sibling is absent -- and it costs what a build costs: this repository's
    /// own plan takes about 31 seconds warm across ten runners
    /// (`docs/upgrading.md` has the numbers and the commands).
    ///
    /// A runner also sees exactly one feature set, so a test behind a flag is
    /// invisible to a default listing and its citations cannot resolve. Cite the
    /// name this command prints, not the path you would write in Rust: for an
    /// integration test `cargo test -- --list` prints the bare function name,
    /// and the near-match in `APLAN013` names the right form when you get it
    /// wrong.
    ///
    /// Every other non-empty line is a name, including a summary sentence the
    /// runner prints at the end of its own listing. `pytest --collect-only -q`
    /// ends with `5 tests collected in 0.01s` and gives no way to suppress it
    /// (`A1-034`), so that line is a name too. It is harmless until the runner
    /// collects nothing, when the summary is the *only* line and a runner that
    /// found no tests looks like one with a test in it (`A1-064`).
    pub list: Vec<String>,
    /// argv that runs one test. Exactly one element must be [`PLACEHOLDER`]; it
    /// is replaced, whole, by the test name.
    ///
    /// **`--execute` believes this command's exit status**, so it is worth
    /// knowing whether yours earns that. The assumption is that the name `list`
    /// prints is the name this command selects: `cargo test -- --exact <name>`
    /// and `pytest <node id>` both honour it, and a filter flag over a different
    /// namespace does not. `vitest list` prints `file > suite > test` while
    /// `vitest run -t` matches the test's title, so the listed name selects
    /// nothing, every test is skipped, and the exit status is `0` -- which reads
    /// here as the test passing (`A1-034`, `A1-167`).
    ///
    /// The one-command check: run your `run` command with a name no test could
    /// have. If it fails, the exit status means something. If it succeeds,
    /// `--execute` cannot tell a pass from a miss for this runner, and the
    /// citations it resolves are worth their L1 claim rather than their L3 one.
    pub run: Vec<String>,
    /// Optional one-process adapter for every distinct cited test this runner
    /// owns. Legacy `run` remains required and is the compatible fallback.
    #[serde(default)]
    pub batch: Option<TestBatch>,
    /// How long `list` may take before this runner is reported as one that could
    /// not be asked, in seconds. Default: 600. `0` means no limit (`A1-121`).
    ///
    /// Ten minutes because `cargo test --list` *compiles*: on a cold cache in CI
    /// that is minutes, legitimately, and a limit that fires on a slow runner
    /// would be a worse defect than the hang it prevents. It bounds the failure
    /// rather than policing the duration.
    ///
    /// **`run` is deliberately not bounded by this.** Under `--execute` the
    /// runner is running a real test, where a long run is the point rather than
    /// a symptom, and a limit there would report a suite that is merely slow as
    /// a runner fault. `--execute` is opt-in and off by default; the harness's
    /// own timeout is the right place for that decision.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// How to check a `url` citation, for `aplan verify` (`APLAN016`).
///
/// An argv vector, spawned without a shell (HLD section 5.5), exactly one
/// element of which is [`URL_PLACEHOLDER`]. Exit zero means reachable.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UrlCheck {
    /// argv that exits zero when the cited url is reachable, e.g.
    /// `["curl", "-sfI", "--max-time", "10", "{url}"]`.
    ///
    /// Give it one of its own too (`--max-time` for curl, `--timeout` for wget):
    /// the command knows what it is doing, and this key is the backstop for when
    /// it does not.
    pub check: Vec<String>,
    /// How long the checker may take per url before the citation is reported as
    /// *not checked*, in seconds. Default: 30. `0` means no limit (`A1-121`).
    ///
    /// Thirty because this is one network request and a page that has not
    /// answered in half a minute has not answered. Lower than the runner's limit
    /// by two orders of magnitude on purpose: the failure a network check meets
    /// is the silent one, since a black-holed packet does not refuse, it goes
    /// quiet, so a hang here is the ordinary case rather than the exotic one.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// The most memory this checker's command may hold, as a size (`256M`)
    /// (`A1-310`, ADR-0033).
    ///
    /// The same key `test` takes, and enforced the same way. Here because
    /// ADR-0033 decided the cap covers *every* spawned command rather than the
    /// test runner's alone: the machine does not care which command ate it, and
    /// a checker is argv an adopter chose exactly as a runner is.
    #[serde(default)]
    pub max_memory: Option<String>,
    /// The most processes this checker's command may have alive at once
    /// (`A1-310`, ADR-0033).
    #[serde(default)]
    pub max_processes: Option<u32>,
    /// The most output from each stream that aplan retains in its own process,
    /// as a size. Default: `1M` (`A1-366`).
    ///
    /// Independent of `max_memory`, which applies to the checker process only.
    /// Output past this bound is still drained so the checker cannot block on a
    /// full pipe.
    #[serde(default)]
    pub max_output: Option<String>,
    /// argv prepended to the checker's command (`A1-310`, ADR-0033).
    ///
    /// Top-level config only, like the command itself: a task file names a url,
    /// never a command to run against it (HLD 5.5).
    #[serde(default)]
    pub wrap: Option<Vec<String>>,
}

/// What a task is expected to say about the released version (`A1-312`,
/// ADR-0034).
///
/// Off unless asked for, and cheap when asked for: a task's **level** supplies a
/// floor, so with this on and nothing else configured a plan that declares no
/// impacts at all still judges clean. The only thing an author writes by hand is
/// what a floor cannot supply -- a `breaking` change inside one task, or a `none`
/// with the reason it is none.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VersionPolicy {
    /// Whether a task claiming less than its level implies must say why
    /// (`APLAN044`). Default: `false`.
    #[serde(default)]
    pub require_impact: Option<bool>,
    /// What each level owes before any member speaks. Default: a unit of work
    /// `fix`, a cut `feature`, a release nothing of its own.
    #[serde(default)]
    pub levels: Option<LevelFloors>,
    /// How to read the version the artifact ships today (`A1-313`, ADR-0040).
    ///
    /// A command, never a parse. This tool does not know what a `pyproject.toml`
    /// means, what a `version` key in one of four build backends refers to, or
    /// which of a monorepo's eleven manifests is the one that ships -- and a
    /// guess about any of those is a wrong number stated confidently, which is
    /// the one thing the arithmetic must never produce.
    ///
    /// It prints the version on stdout and exits zero. The first non-empty line
    /// is taken, trimmed, with a leading `v` removed -- because `git describe`
    /// and `cargo pkgid` disagree about that one character and neither is wrong.
    ///
    /// Without it the plan still answers what a node *owes*, which needs nothing
    /// outside the plan. What it cannot do is turn that into a number, because
    /// the number is relative to one this tool has no way to learn.
    #[serde(default)]
    pub current: Option<DeclaredCommand>,
    /// How to ask the artifact whether it is compatible with the last release
    /// (`A1-314`, `APLAN049`).
    ///
    /// **Exit zero when it is, non-zero when it is not.** That is the question
    /// `cargo semver-checks check-release` already answers natively, and asking
    /// it this way is what keeps this tool's vocabulary out of somebody else's
    /// checker: a command that had to print `none|fix|feature|breaking` would be
    /// this project teaching an ecosystem its words, which is the refusal
    /// `A1-037` makes about `vN.N` one layer along.
    ///
    /// It is the L3 rung of a version claim, and it behaves like the other one:
    /// opt-in, run only where declared, reported as a finding about the *claim*
    /// rather than as a crash, and bounded by the limits above -- an API differ
    /// is exactly the kind of command that compiles a workspace twice.
    ///
    /// Nothing here reimplements API diffing. The tool runs the checker the
    /// repository declares and reports what it said, exactly as it runs
    /// `test.run`.
    #[serde(default)]
    pub check: Option<DeclaredCommand>,
    /// Which tags name a release, as a glob (`A1-317`, ADR-0038). Default: `v*`.
    ///
    /// ADR-0038 decided the window with no release node is everything closed
    /// since the nearest release tag reachable from `HEAD`, and that a
    /// repository tagging things other than releases has to say which tags are
    /// releases. `v*` is what this project and its pilots use; a project tagging
    /// `release-2026-09` says so here rather than being told its vocabulary is
    /// wrong (`A1-037`).
    #[serde(default)]
    pub release_tag: Option<String>,
}

/// A command `.aplan.yml` declares under `version` (`A1-313`, `A1-314`).
///
/// **One type for the two of them**, because they are the same idea with
/// different questions: `version.current` prints the version the artifact ships,
/// and `version.check` exits zero when the artifact is compatible with the last
/// release. What each *means* is documented on the key that takes it; what they
/// share -- how it is spawned and what it may spend -- is documented here.
///
/// It carries the same five limit keys `test` and `url` carry, because ADR-0033
/// covers *every* spawned command: the machine does not care which one ate it.
/// A third implementation of the configured-checker idea is what `A1-185` is
/// about, and adding a fourth here rather than a second would have been that
/// task's subject arriving twice in one cut.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredCommand {
    /// argv, e.g. `["ci/scripts/version.sh"]`.
    ///
    /// Spawned with no shell (HLD section 5.5). A pipeline is declared as one:
    /// `["sh", "-c", "..."]` is this repository's business to write and not this
    /// tool's to synthesise.
    pub command: Vec<String>,
    /// How long it may take before the version is reported as *not read*, in
    /// seconds. Default: 30. `0` means no limit (`A1-121`).
    ///
    /// The same thirty seconds `url.check` gets, and for the same reason: this
    /// is one question with one answer, and a command that has not printed a
    /// version in half a minute is not going to.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// The most memory this command may hold, as a size (`256M`) (`A1-310`,
    /// ADR-0033).
    #[serde(default)]
    pub max_memory: Option<String>,
    /// The most processes it may have alive at once (`A1-310`, ADR-0033).
    #[serde(default)]
    pub max_processes: Option<u32>,
    /// The most output from each stream that aplan retains in its own process,
    /// as a size. Default: `1M` (`A1-366`).
    ///
    /// Independent of `max_memory`, which applies to the declared command only.
    /// Output past this bound is still drained so the command cannot block on a
    /// full pipe.
    #[serde(default)]
    pub max_output: Option<String>,
    /// argv prepended to it (`A1-310`, ADR-0033).
    #[serde(default)]
    pub wrap: Option<Vec<String>>,
}

/// The policy shared by every command a repository asks `aplan` to spawn.
///
/// The declaring shapes stay separate: a test runner has `list` and `run`, a
/// url checker has `check`, and a version command has `command`. What happens
/// around those argv vectors is one contract, though: timeout, resource and
/// capture limits, and an optional wrapper. This borrowed view makes that
/// contract one input to validation and execution without changing any YAML
/// shape (`A1-185`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandPolicy<'a> {
    /// A per-command timeout override, in seconds. Zero means no timeout.
    pub timeout_seconds: Option<u64>,
    /// The configured memory cap, before parsing into bytes.
    pub max_memory: Option<&'a str>,
    /// The configured process cap.
    pub max_processes: Option<u32>,
    /// The configured retained-output cap, before parsing into bytes.
    pub max_output: Option<&'a str>,
    /// The argv prefix placed outside the declared command.
    pub wrap: Option<&'a [String]>,
}

impl CommandPolicy<'_> {
    /// Apply the command kind's documented timeout default.
    #[must_use]
    pub fn timeout_or(self, default_seconds: u64) -> u64 {
        self.timeout_seconds.unwrap_or(default_seconds)
    }
}

/// The floor each level of the plan carries (`A1-312`, ADR-0034).
///
/// A **floor**, not an assertion: aggregation takes the strongest of the floors
/// and the claims, so a `breaking` task raises the cut that holds it. Only a
/// floor composes -- a ceiling would let a break ship inside a compatible
/// release.
///
/// Configuration rather than a constant, and that is the point. The shipped
/// default does not fit the plan that ships it: this repository has eight done
/// cuts against two tags, so a cut here is a planning bucket rather than a
/// delivery. A map nobody can change is a map that fits one project.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelFloors {
    /// A unit of work. Default: `fix`.
    #[serde(default)]
    pub task: Option<aplan_core::task::VersionImpact>,
    /// A milestone holding work (ADR-0022). Default: `feature`.
    #[serde(default)]
    pub cut: Option<aplan_core::task::VersionImpact>,
    /// A milestone closing cuts (ADR-0022). Default: `none` -- "every release is
    /// a major" is version by ceremony rather than by compatibility, and wrong
    /// for any project past 1.0. A team that versions that way sets it here.
    #[serde(default)]
    pub release: Option<aplan_core::task::VersionImpact>,
}

impl VersionPolicy {
    /// Whether the rule is on.
    #[must_use]
    pub fn requires_impact(&self) -> bool {
        self.require_impact.unwrap_or(false)
    }

    /// The glob that says which tags are releases, defaulted (ADR-0038).
    #[must_use]
    pub fn release_tag(&self) -> &str {
        self.release_tag.as_deref().unwrap_or(DEFAULT_RELEASE_TAG)
    }

    /// This policy's floors, with the shipped defaults filled in.
    #[must_use]
    pub fn floors(&self) -> aplan_core::task::VersionFloors {
        let shipped = aplan_core::task::VersionFloors::default();
        let l = self.levels.as_ref();
        aplan_core::task::VersionFloors {
            task: l.and_then(|l| l.task).unwrap_or(shipped.task),
            cut: l.and_then(|l| l.cut).unwrap_or(shipped.cut),
            release: l.and_then(|l| l.release).unwrap_or(shipped.release),
        }
    }
}

impl Config {
    /// Reject an `advisory` entry that names no rule.
    ///
    /// A typo'd code would silently downgrade nothing, which reads as a
    /// repository that made a choice when it made none: the vacuity this project
    /// keeps finding elsewhere (`A1-112`).
    ///
    /// # Errors
    /// [`Error::BadAdvisory`] naming the unknown code.
    /// Reject a `docs.membership` the check could not be run from.
    pub fn validate_docs(&self) -> Result<(), Error> {
        let Some(docs) = &self.docs else {
            return Ok(());
        };
        if let Some(paths) = &docs.paths {
            if paths.is_empty() {
                return Err(Error::BadDocPaths(
                    "it is an empty list; remove the key, or name a document".into(),
                ));
            }
            for pattern in paths {
                if aplan_core::task::is_vacuous_ref(pattern) {
                    return Err(Error::BadDocPaths(
                        "it contains a blank pattern, which names no document".into(),
                    ));
                }
                if let Err(error) = aplan_core::glob::compile(pattern) {
                    return Err(Error::BadDocPaths(format!(
                        "pattern `{pattern}` is not a usable glob: {error}"
                    )));
                }
            }
        }
        if let Some(membership) = &docs.membership {
            if membership.is_empty() {
                return Err(Error::BadDocs(
                    "`docs.membership` is an empty list; remove the key, or name a document".into(),
                ));
            }
            for entry in membership {
                entry.validate()?;
            }
        }
        Ok(())
    }

    pub fn validate_advisory(&self) -> Result<(), Error> {
        let Some(codes) = &self.advisory else {
            return Ok(());
        };
        for code in codes {
            if !aplan_core::Rule::ALL.iter().any(|r| r.code() == code) {
                return Err(Error::BadAdvisory(format!(
                    "`{code}` is not a rule this build knows; `aplan facts` lists them"
                )));
            }
        }
        Ok(())
    }
}

impl ScopedPolicy {
    /// Reject a scope the tool cannot act on (`A1-183`).
    ///
    /// # Errors
    ///
    /// When the scope names no dimension, or sets nothing. Either is a entry
    /// that reads as a decision and makes none, which is the vacuity this
    /// project keeps finding elsewhere (`A1-112`).
    pub fn validate(&self) -> Result<(), Error> {
        // Refused here rather than dropped later: see `Scope::selector`.
        self.when.selector()?;
        if self.when.named_dimensions() == 0 {
            return Err(Error::BadPolicy(
                "`when` names no subset. An empty scope is not \"the whole plan\": the top-level \
                 keys already say that, and a scope that silently became global is the last thing \
                 a reader would expect. Name a `cut`, `phase`, `tag`, `owner` or `status`, or \
                 delete the \
                 entry"
                    .into(),
            ));
        }
        let sets_nothing = self
            .evidence
            .as_ref()
            .is_none_or(|e| e.require_min_strength.is_none() && e.exempt_tags.is_none());
        if sets_nothing {
            return Err(Error::BadPolicy(
                "it selects a subset and sets no policy for it, so it changes nothing. Give it an \
                 `evidence.require_min_strength` or an `evidence.exempt_tags`, or delete it"
                    .into(),
            ));
        }
        Ok(())
    }
}

impl VersionPolicy {
    /// Reject a `version` block that could not be acted on.
    ///
    /// # Errors
    /// [`Error::BadVersion`] naming what about it cannot be acted on.
    pub fn validate(&self) -> Result<(), Error> {
        // Before the command, because this key is answered whether or not one is
        // declared: an empty glob matches no tag, so the window would silently
        // become "everything this plan ever closed" on a repository that has
        // tagged eleven releases.
        if self
            .release_tag
            .as_deref()
            .is_some_and(|t| t.trim().is_empty())
        {
            return Err(Error::BadVersion(
                "`version.release_tag` is empty; remove the key to take the default `v*`, \
                 or give it the glob this repository's release tags match"
                    .into(),
            ));
        }
        for (key, declared) in [("current", &self.current), ("check", &self.check)] {
            let Some(declared) = declared else { continue };
            declared.validate(&format!("version.{key}"))?;
        }
        Ok(())
    }
}

impl DeclaredCommand {
    /// The spawning policy this command shares with test and url checkers.
    #[must_use]
    pub fn command_policy(&self) -> CommandPolicy<'_> {
        CommandPolicy {
            timeout_seconds: self.timeout_seconds,
            max_memory: self.max_memory.as_deref(),
            max_processes: self.max_processes,
            max_output: self.max_output.as_deref(),
            wrap: self.wrap.as_deref(),
        }
    }

    /// Reject a declared command that could not be acted on, naming the key that
    /// declared it.
    ///
    /// # Errors
    /// [`Error::BadVersion`] naming `key` and what about it cannot be acted on.
    pub fn validate(&self, key: &str) -> Result<(), Error> {
        validate_limits(key, self.command_policy()).map_err(Error::BadVersion)?;
        if self.command.is_empty() {
            return Err(Error::BadVersion(format!(
                "`{key}.command` is empty; remove the key, or give it a command to run"
            )));
        }
        // The same refusal `url.check` makes, for the same reason: a blank
        // element reaches the spawn as an empty program name and fails with
        // `No such file or directory`, which reads as a missing binary rather
        // than as the config mistake it is.
        if self.command.iter().any(|a| a.trim().is_empty()) {
            return Err(Error::BadVersion(format!(
                "`{key}.command` contains a blank argument; every element must be a real \
                 program name or argument"
            )));
        }
        Ok(())
    }
}

impl UrlCheck {
    /// The spawning policy this checker shares with test and version commands.
    #[must_use]
    pub fn command_policy(&self) -> CommandPolicy<'_> {
        CommandPolicy {
            timeout_seconds: self.timeout_seconds,
            max_memory: self.max_memory.as_deref(),
            max_processes: self.max_processes,
            max_output: self.max_output.as_deref(),
            wrap: self.wrap.as_deref(),
        }
    }

    /// Reject a command that could not receive a url safely.
    pub fn validate(&self) -> Result<(), Error> {
        validate_limits("url", self.command_policy()).map_err(Error::BadUrlCheck)?;
        if self.check.is_empty() {
            return Err(Error::BadUrlCheck(
                "it is empty; remove the key, or give it a command".into(),
            ));
        }
        // A blank element reaches the spawn as an empty program name and fails
        // with `No such file or directory`, which reads as a missing binary
        // rather than as the config mistake it is. `test.covers` refuses blanks
        // for the same reason.
        if self.check.iter().any(|a| a.trim().is_empty()) {
            return Err(Error::BadUrlCheck(
                "it contains a blank argument; every element must be a real program name or \
                 argument"
                    .into(),
            ));
        }
        let holes = self
            .check
            .iter()
            .filter(|e| e.as_str() == URL_PLACEHOLDER)
            .count();
        if holes != 1 {
            return Err(Error::BadUrlCheck(format!(
                "it must contain `{URL_PLACEHOLDER}` exactly once, as a whole argument, so \
                 the url is passed as data and never spliced into a command (found {holes})"
            )));
        }
        Ok(())
    }
}

/// What `aplan check-docs` reads by default and how it interprets cut
/// membership declarations.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocsConfig {
    /// Documents whose task references `aplan check-docs` checks when no
    /// paths are passed on the command line.
    ///
    /// Each value may be a literal path or a glob. Relative values resolve from
    /// the configuration file's directory. A single `*` stays within one path
    /// segment; crossing directories needs `**`. Every declared pattern must
    /// match at least one file when the command runs, so a misspelled or
    /// outgrown declaration cannot turn the gate into a vacuous pass.
    ///
    /// Command-line paths override this list for a one-off check. Documents in
    /// `membership` join the implicit set automatically and need not be
    /// repeated here.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    /// Documents that declare a cut's membership, and where in them it is
    /// declared.
    ///
    /// A plan document listing a cut's tasks beside a `M-<cut>` milestone holds
    /// two copies of one fact, and until something reads both, one of them is
    /// ungated. Declared rather than inferred: measured on the two real documents
    /// of this shape, reading a cut's whole section instead of its declaration
    /// calls fourteen ordinary cross-references false declarations, and a
    /// document's shape means whatever its author intended (ADR-0019).
    ///
    /// Absent, `check-docs` checks references alone, exactly as before.
    #[serde(default)]
    pub membership: Option<Vec<MembershipDoc>>,
}

/// One document that declares cut membership.
///
/// ```yaml
/// docs:
///   membership:
///     - doc: ROADMAP.md
///       declared_by: line
///       marker: "**Tasks:**"
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MembershipDoc {
    /// The document. A relative value resolves from the configuration file. It
    /// joins the default no-argument document set automatically. Explicit
    /// command-line paths still select only the documents named there, so a
    /// one-file run does not report every cut in a document nobody asked about.
    pub doc: String,
    /// Which shape the declaration takes.
    pub declared_by: DeclaredBy,
    /// For `declared_by: line`, the marker the line begins with.
    #[serde(default)]
    pub marker: Option<String>,
}

/// The two shapes a cut's membership is declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeclaredBy {
    /// A line beginning with `marker`, whose ids are the members.
    Line,
    /// A table under the cut's heading, one member per row, in the row's first
    /// cell (ADR-0015). The remaining cells are prose and are not read.
    Table,
}

impl MembershipDoc {
    /// Reject a declaration the check could not be run from.
    ///
    /// A `marker` beside `declared_by: table` is refused rather than ignored: a
    /// config key the tool silently drops is a claim the repository believes and
    /// nothing honours, which is the failure this whole tool argues against.
    pub fn validate(&self) -> Result<(), Error> {
        if aplan_core::task::is_vacuous_ref(&self.doc) {
            return Err(Error::BadDocs(
                "an entry with no `doc` names no document, and would be read on no run".into(),
            ));
        }
        match (self.declared_by, &self.marker) {
            (DeclaredBy::Line, None) => Err(Error::BadDocs(format!(
                "`{}` says `declared_by: line` and gives no `marker`; the line has to \
                 begin with something (`marker: \"**Tasks:**\"`)",
                self.doc
            ))),
            (DeclaredBy::Line, Some(m)) if aplan_core::task::is_vacuous_ref(m) => {
                Err(Error::BadDocs(format!(
                    "`{}` has an empty `marker`, which every line begins with",
                    self.doc
                )))
            }
            (DeclaredBy::Table, Some(_)) => Err(Error::BadDocs(format!(
                "`{}` says `declared_by: table` and also gives a `marker`, which a table \
                 declaration does not use; remove one",
                self.doc
            ))),
            _ => Ok(()),
        }
    }
}

impl DocsConfig {
    /// Resolve the documents this configuration declares.
    ///
    /// Literal and glob paths are resolved from `base`, which is the directory
    /// containing the selected or discovered configuration. Membership
    /// documents are literal declarations and join `paths` without requiring a
    /// second spelling. Overlapping declarations are canonicalised, deduplicated
    /// and returned in a stable order.
    ///
    /// # Errors
    ///
    /// A literal that is not a file, a glob that matches no files, an unreadable
    /// directory, or a glob that was not validated first.
    pub fn document_paths(&self, base: &Path) -> Result<Vec<PathBuf>, Error> {
        let mut documents = BTreeMap::new();
        for pattern in self.paths.iter().flatten() {
            let matches = expand_document_pattern(base, pattern)?;
            if matches.is_empty() {
                return Err(Error::BadDocPaths(format!(
                    "pattern `{pattern}` matched no files relative to `{}`",
                    base.display()
                )));
            }
            for path in matches {
                insert_document(&mut documents, path, "a declared pattern", true)?;
            }
        }
        for membership in self.membership.iter().flatten() {
            let path = if Path::new(&membership.doc).is_absolute() {
                PathBuf::from(&membership.doc)
            } else {
                base.join(&membership.doc)
            };
            insert_document(&mut documents, path, "`docs.membership[].doc`", false)?;
        }
        let mut paths: Vec<PathBuf> = documents.into_values().collect();
        paths.sort();
        Ok(paths)
    }
}

fn insert_document(
    documents: &mut BTreeMap<PathBuf, PathBuf>,
    path: PathBuf,
    source: &str,
    from_paths: bool,
) -> Result<(), Error> {
    let invalid = |message| {
        if from_paths {
            Error::BadDocPaths(message)
        } else {
            Error::BadDocs(message)
        }
    };
    if !path.is_file() {
        return Err(invalid(format!(
            "{source} names `{}`, which is not a file",
            path.display()
        )));
    }
    let identity = std::fs::canonicalize(&path).map_err(|error| {
        invalid(format!(
            "could not resolve declared document `{}`: {error}",
            path.display()
        ))
    })?;
    documents
        .entry(identity)
        .and_modify(|existing| {
            if path < *existing {
                existing.clone_from(&path);
            }
        })
        .or_insert(path);
    Ok(())
}

fn expand_document_pattern(base: &Path, pattern: &str) -> Result<Vec<PathBuf>, Error> {
    let spelling = aplan_core::glob::spelling(pattern);
    let compiled = aplan_core::glob::compile(&spelling)
        .map_err(|error| Error::BadDocPaths(format!("`{pattern}`: {error}")))?
        .compile_matcher();
    let pattern_path = Path::new(&spelling);
    let mut prefix = PathBuf::new();
    for component in pattern_path.components() {
        let text = component.as_os_str().to_string_lossy();
        if text
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | '{'))
        {
            break;
        }
        prefix.push(component.as_os_str());
    }

    // A literal needs no tree walk and gets the sharper `not a file` error
    // from `insert_document`. It is still returned here so the caller has one
    // deduplication path for both shapes.
    if prefix == pattern_path {
        let path = if pattern_path.is_absolute() {
            pattern_path.to_path_buf()
        } else {
            base.join(pattern_path)
        };
        return Ok(vec![path]);
    }

    let walk_root = if prefix.is_absolute() {
        prefix.clone()
    } else {
        base.join(&prefix)
    };
    // A single `*` cannot cross a separator, so `*.md` has no reason to read
    // `.git`, `target`, or any other subtree. Only `**` makes the walk
    // unbounded. Besides being faster, this means an inaccessible directory
    // outside a pattern's possible depth cannot make an otherwise usable
    // declaration fail.
    let remaining_depth = if spelling.contains("**") {
        None
    } else {
        Some(
            pattern_path
                .components()
                .count()
                .saturating_sub(prefix.components().count()),
        )
    };
    let mut selected = Vec::new();
    let matches_pattern = |path: &Path| compiled.is_match(path);
    walk_matching_files(
        &walk_root,
        &prefix,
        remaining_depth,
        true,
        &matches_pattern,
        &mut selected,
    )
    .map_err(|error| {
        Error::BadDocPaths(format!(
            "could not expand pattern `{pattern}` from `{}`: {error}",
            base.display()
        ))
    })?;
    selected.sort();
    Ok(selected)
}

fn walk_matching_files(
    actual: &Path,
    candidate: &Path,
    remaining_depth: Option<usize>,
    root: bool,
    matcher: &impl Fn(&Path) -> bool,
    selected: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    // Follow a symlink in the fixed prefix (`linked-docs/*.md`), as ordinary
    // path lookup does, but never follow one discovered while recursing: that
    // makes symlink cycles impossible without denying a useful top-level
    // declaration.
    let metadata = match if root {
        std::fs::metadata(actual)
    } else {
        std::fs::symlink_metadata(actual)
    } {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let kind = metadata.file_type();
    if kind.is_file() || (kind.is_symlink() && actual.is_file()) {
        if matcher(candidate) {
            selected.push(actual.to_path_buf());
        }
        return Ok(());
    }
    if !kind.is_dir() {
        return Ok(());
    }
    if remaining_depth == Some(0) {
        return Ok(());
    }

    let mut entries = std::fs::read_dir(actual)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        walk_matching_files(
            &entry.path(),
            &candidate.join(entry.file_name()),
            remaining_depth.map(|depth| depth.saturating_sub(1)),
            false,
            matcher,
            selected,
        )?;
    }
    Ok(())
}

/// Roadmap rendering options.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoadmapConfig {
    /// The tag family whose members group a cut's tasks in the roadmap view.
    ///
    /// Must name a family declared in [`tag_families`](Config::tag_families):
    /// grouping by a vocabulary nothing validates would render headings a typo
    /// could invent (ADR-0014, `A1-078`).
    #[serde(default)]
    pub group_by: Option<String>,
}

/// Summary rollup options.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SummaryConfig {
    /// The tag family whose members replace phases as the summary's rows.
    ///
    /// Must name a family declared in [`Config::tag_families`]. Tasks carrying
    /// more than one member appear in every matching row, while the total still
    /// counts each task once (`A1-186`).
    #[serde(default)]
    pub group_by: Option<String>,
}

/// Paths a named subset of a runner's tests covers.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestCoverage {
    /// Test-name patterns, where `*` matches any run of characters
    /// (`invariants::*`). Matched against the name as the runner lists it.
    pub tests: Vec<String>,
    /// Path globs the matching tests cover. `['**']` says "the whole
    /// repository", which is what a repository-wide invariant asserts.
    pub paths: Vec<String>,
}

impl TestCoverage {
    /// Reject a coverage claim the tool cannot act on.
    fn validate(&self) -> Result<(), Error> {
        for (field, values) in [("tests", &self.tests), ("paths", &self.paths)] {
            if values.is_empty() {
                return Err(Error::BadRunner(format!(
                    "`test.covers_by_test.{field}` is an empty list; remove the entry, or name \
                     what it covers"
                )));
            }
            if values.iter().any(|v| v.trim().is_empty()) {
                return Err(Error::BadRunner(format!(
                    "`test.covers_by_test.{field}` contains a blank entry"
                )));
            }
        }
        Ok(())
    }
}

/// Repository inputs whose change can affect a named subset of tests.
///
/// Unlike [`TestCoverage`], this is execution-selection policy. It is an
/// exhaustive trust claim and therefore has its own type and configuration
/// key; proof locality must never be mistaken for invalidation locality.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestInputDependency {
    /// Test-name glob patterns, matched against the complete current catalogue.
    pub tests: Vec<String>,
    /// Repository-relative path globs that can affect those tests.
    pub paths: Vec<String>,
}

impl TestInputDependency {
    fn validate(&self) -> Result<(), Error> {
        for (field, values) in [("tests", &self.tests), ("paths", &self.paths)] {
            if values.is_empty() {
                return Err(Error::BadRunner(format!(
                    "`test.inputs_by_test.{field}` is an empty list; every entry must name both tests and their inputs"
                )));
            }
            if values
                .iter()
                .any(|value| aplan_core::task::is_vacuous_ref(value))
            {
                return Err(Error::BadRunner(format!(
                    "`test.inputs_by_test.{field}` contains a blank entry"
                )));
            }
        }
        for pattern in &self.tests {
            globset::Glob::new(pattern).map_err(|error| {
                Error::BadRunner(format!(
                    "invalid `test.inputs_by_test.tests` pattern `{pattern}`: {error}"
                ))
            })?;
        }
        for pattern in &self.paths {
            let path = Path::new(pattern);
            if path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                return Err(Error::BadRunner(format!(
                    "`test.inputs_by_test.paths` entry `{pattern}` must stay below the repository root"
                )));
            }
            globset::Glob::new(pattern).map_err(|error| {
                Error::BadRunner(format!(
                    "invalid `test.inputs_by_test.paths` pattern `{pattern}`: {error}"
                ))
            })?;
        }
        Ok(())
    }
}

/// A size written the way an adopter writes one: `4G`, `512M`, `1048576`.
///
/// Binary multiples, because that is what `memory.max` takes and what every
/// tool in this space means by `G`. A bare number is bytes. Refused rather than
/// guessed at: an unparseable limit is a limit nobody can act on, and silently
/// reading `4GB` as four bytes would cap a test suite at nothing (`A1-310`).
pub fn parse_size(text: &str) -> Result<u64, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("a size, such as `4G` or `512M`, and not an empty string".into());
    }
    let (digits, scale) = match t.as_bytes()[t.len() - 1] {
        b'K' | b'k' => (&t[..t.len() - 1], 1024u64),
        b'M' | b'm' => (&t[..t.len() - 1], 1024 * 1024),
        b'G' | b'g' => (&t[..t.len() - 1], 1024 * 1024 * 1024),
        b'T' | b't' => (&t[..t.len() - 1], 1024u64 * 1024 * 1024 * 1024),
        _ => (t, 1),
    };
    let n: u64 = digits
        .trim()
        .parse()
        .map_err(|_| format!("`{text}` is not a size; write `4G`, `512M`, or a number of bytes"))?;
    n.checked_mul(scale)
        .filter(|b| *b > 0)
        .ok_or_else(|| format!("`{text}` is not a size this tool can act on"))
}

/// Check the four limit keys a spawning command may declare (`A1-310`,
/// `A1-366`).
///
/// Shared by `test`, `url` and version commands, which must refuse the
/// same values: two copies of this drifted the moment one of them grew a case.
/// `key` names the block for the message, so a reader is told which one to fix.
fn validate_limits(key: &str, limits: CommandPolicy<'_>) -> Result<(), String> {
    // A limit that cannot be parsed is refused here rather than at the moment
    // a command would have run under it: the value is the adopter's, and this
    // is the same load-time strictness an unknown key gets (`A1-310`).
    if let Some(size) = limits.max_memory {
        parse_size(size).map_err(|why| format!("`{key}.max_memory` is {why}"))?;
    }
    if let Some(size) = limits.max_output {
        parse_size(size).map_err(|why| format!("`{key}.max_output` is {why}"))?;
    }
    if limits.max_processes == Some(0) {
        return Err(format!(
            "`{key}.max_processes` is 0, which no command can satisfy; remove the key, or \
             give it the most processes this may have alive at once"
        ));
    }
    match limits.wrap {
        Some([]) => Err(format!(
            "`{key}.wrap` is an empty list; remove the key, or name the command to run \
             the command under"
        )),
        Some(w) if w.iter().any(|a| a.trim().is_empty()) => {
            Err(format!("`{key}.wrap` contains a blank argument"))
        }
        _ => Ok(()),
    }
}

impl TestRunner {
    /// Weighted aggregate capacity this runner consumes while active.
    #[must_use]
    pub fn weight(&self) -> u32 {
        self.weight.unwrap_or(1)
    }

    /// Named capacity-one resources, in deterministic acquisition order.
    #[must_use]
    pub fn exclusive_resources(&self) -> BTreeSet<String> {
        self.exclusive
            .as_ref()
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }

    /// The spawning policy every command of this runner shares with url and
    /// version checkers.
    #[must_use]
    pub fn command_policy(&self) -> CommandPolicy<'_> {
        CommandPolicy {
            timeout_seconds: self.timeout_seconds,
            max_memory: self.max_memory.as_deref(),
            max_processes: self.max_processes,
            max_output: self.max_output.as_deref(),
            wrap: self.wrap.as_deref(),
        }
    }

    fn validate_scheduler_reservation(&self) -> Result<(), Error> {
        if self.weight == Some(0) {
            return Err(Error::BadRunner(
                "`test` runner weight is 0, which no provider command can consume".into(),
            ));
        }
        let Some(resources) = &self.exclusive else {
            return Ok(());
        };
        if resources.is_empty() {
            return Err(Error::BadRunner(
                "`test` runner exclusive resource list is empty; remove it, or name a resource"
                    .into(),
            ));
        }
        let mut unique = BTreeSet::new();
        for resource in resources {
            if resource.trim().is_empty() || resource != resource.trim() {
                return Err(Error::BadRunner(
                    "`test` runner exclusive resources must be non-blank and have no surrounding whitespace"
                        .into(),
                ));
            }
            if resource.chars().any(|character| {
                character.is_control()
                    || matches!(
                        character,
                        '\u{200B}'
                            | '\u{200C}'
                            | '\u{200D}'
                            | '\u{2060}'
                            | '\u{FEFF}'
                            | '\u{00AD}'
                            | '\u{200E}'
                            | '\u{200F}'
                            | '\u{061C}'
                    )
            }) {
                return Err(Error::BadRunner(
                    "`test` runner exclusive resources may not contain control or invisible formatting characters"
                        .into(),
                ));
            }
            if !unique.insert(resource) {
                return Err(Error::BadRunner(format!(
                    "`test` runner repeats exclusive resource `{resource}`"
                )));
            }
        }
        Ok(())
    }

    /// Reject a runner that could not receive a test name safely.
    pub fn validate(&self) -> Result<(), Error> {
        validate_limits("test", self.command_policy()).map_err(Error::BadRunner)?;
        self.validate_scheduler_reservation()?;
        // Config the tool cannot act on is an error, never a silent no-op: an
        // empty `covers` reads as a locality claim and makes none.
        match &self.covers {
            Some(c) if c.is_empty() => {
                return Err(Error::BadRunner(
                    "`test.covers` is an empty list; remove the key, or name the paths this \
                     runner covers"
                        .into(),
                ));
            }
            Some(c) if c.iter().any(|p| p.trim().is_empty()) => {
                return Err(Error::BadRunner(
                    "`test.covers` contains a blank path glob".into(),
                ));
            }
            _ => {}
        }
        if let Some(by_test) = &self.covers_by_test {
            if by_test.is_empty() {
                return Err(Error::BadRunner(
                    "`test.covers_by_test` is an empty list; remove the key, or name the tests \
                     that cover something else"
                        .into(),
                ));
            }
            for entry in by_test {
                entry.validate()?;
            }
        }
        if let Some(by_test) = &self.inputs_by_test {
            if by_test.is_empty() {
                return Err(Error::BadRunner(
                    "`test.inputs_by_test` is an empty list; remove the key to refresh the complete runner, or declare every test's inputs"
                        .into(),
                ));
            }
            for entry in by_test {
                entry.validate()?;
            }
        }
        if self.list.is_empty() {
            return Err(Error::BadRunner("`test.list` is an empty command".into()));
        }
        if self.run.is_empty() {
            return Err(Error::BadRunner("`test.run` is an empty command".into()));
        }
        let holes = self
            .run
            .iter()
            .filter(|e| e.as_str() == PLACEHOLDER)
            .count();
        if holes != 1 {
            return Err(Error::BadRunner(format!(
                "`test.run` must contain `{PLACEHOLDER}` exactly once, as a whole argument \
                 (found {holes})"
            )));
        }
        // The placeholder must never be the program (argv[0]): `run: ['{test}']`
        // would make the untrusted test name the command that is executed. The
        // command must be a fixed, trusted program, this is the core of the trust
        // boundary (HLD 5.5), and `validate` is where it is enforced.
        if self.run.first().map(String::as_str) == Some(PLACEHOLDER) {
            return Err(Error::BadRunner(format!(
                "`{PLACEHOLDER}` may not be the command (the first element) of `test.run`: the \
                 program to run must be a fixed command, never the test name"
            )));
        }
        // The name must never be concatenated into a larger argument: that is
        // the only way a shell-using runner could be injected by a task file.
        if self
            .run
            .iter()
            .any(|e| e.as_str() != PLACEHOLDER && e.contains(PLACEHOLDER))
        {
            return Err(Error::BadRunner(format!(
                "`test.run` may use `{PLACEHOLDER}` only as a whole argument, never spliced \
                 inside another one"
            )));
        }
        if let Some(batch) = &self.batch {
            batch.validate()?;
        }
        Ok(())
    }

    /// The argv to run `test_name`. The name replaces only the standalone
    /// placeholder element, so it is never spliced into a shell string.
    pub fn run_argv(&self, test_name: &str) -> Vec<String> {
        self.run
            .iter()
            .map(|e| {
                if e == PLACEHOLDER {
                    test_name.to_string()
                } else {
                    e.clone()
                }
            })
            .collect()
    }
}

/// Load `.aplan.yml`, discovered by walking up from `start` to the filesystem
/// root (the nearest one wins), as git and most linters find their config. So
/// `check` and `verify`, given any path inside the repo, converge on the same
/// top-level file. Absent is not an error: it yields the default (empty) config,
/// so a repo with no `.aplan.yml` works unchanged.
pub fn load_config(start: &Path) -> Result<Config, Error> {
    Ok(load_config_located(start)?.0)
}

/// Like [`load_config`], but also returns the directory of the `.aplan.yml` that
/// was loaded (`None` when none was found). A *relative* path in the config, such
/// as `traces_root`, must be resolved against that directory, not the process's
/// current directory, or the verdict would depend on where the command was run
/// from, exactly the location dependence walking-up discovery exists to remove.
/// Where a run's configuration comes from (`A1-155`).
///
/// Discovery walks *up from a path the caller chose*, which is the right default
/// for a person in their own repository and the wrong one for a trusted binary
/// reading somebody else's checkout: the fifth pilot showed a review run whose
/// `--root` and `--repo` pointed at a candidate tree selecting that tree's
/// `test` and `url` programs, from a launcher that had deliberately supplied its
/// own binary and working directory (pre-public adoption evidence).
///
/// So the source is a choice, and an explicit one does not fall back: a named
/// file that is missing is an error, never a quiet return to discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// Walk up from this path looking for `.aplan.yml`. The default.
    Discover,
    /// Read exactly this file, and consult nothing else.
    File(PathBuf),
    /// Read nothing. Distinct from finding nothing, and the caller says so.
    None,
}

/// Load configuration from the source the caller selected.
///
/// Returns the config and the directory its *relative* values resolve against --
/// the selected file's directory, the discovered file's directory, or `None`
/// when nothing was read.
///
/// # Errors
///
/// [`Error::Io`] when a named file cannot be read, plus the parse and validation
/// errors [`load_config_located`] raises.
pub fn load_from(source: &ConfigSource, start: &Path) -> Result<(Config, Option<PathBuf>), Error> {
    match source {
        ConfigSource::Discover => load_config_located(start),
        // No fallback, deliberately: a typo in `--config` must not degrade to
        // whatever the tree being reviewed happens to carry, which is the whole
        // point of naming one.
        ConfigSource::File(path) => read_config(path),
        ConfigSource::None => Ok((Config::default(), None)),
    }
}

pub fn load_config_located(start: &Path) -> Result<(Config, Option<PathBuf>), Error> {
    let Some(path) = discovered_config(start) else {
        return Ok((Config::default(), None));
    };
    read_config(&path)
}

/// Read, parse and validate exactly this file.
///
/// Split out so discovery and an explicit `--config` cannot diverge in what they
/// accept: one parser, one validation, one set of messages (`A1-155`).
fn read_config(path: &Path) -> Result<(Config, Option<PathBuf>), Error> {
    let bytes = std::fs::read(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    let config = parse_config_bytes(&bytes, path)?;
    let dir = path.parent().map(Path::to_path_buf);
    Ok((config, dir))
}

/// Decode, parse and validate configuration bytes associated with `path`.
///
/// Kept public for a repository reader: committed content is bytes, and an
/// invalid UTF-8 config must fail the same way a filesystem read does rather
/// than silently look absent (`A1-194`).
///
/// # Errors
///
/// [`Error::Io`] for invalid UTF-8, then the errors from [`parse_config`].
pub fn parse_config_bytes(bytes: &[u8], path: &Path) -> Result<Config, Error> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    parse_config(text, path)
}

/// Parse and validate configuration text associated with `path`.
///
/// The ordinary loader and revision-addressed reporting meet here, so a
/// committed `.aplan.yml` is judged by the same unknown-key inventory,
/// semantic checks, and path-bearing messages as the working copy (`A1-194`).
/// `path` names the source for diagnostics; this function does not read it.
///
/// # Errors
///
/// The same parse, unknown-key and semantic errors as [`load_from`].
pub fn parse_config(text: &str, path: &Path) -> Result<Config, Error> {
    let config: Config = yaml_compat::from_str(text).map_err(|e| {
        // Ask the schema which keys this build accepts rather than parsing
        // serde's sentence back out: the inventory is derived from the same type
        // the parser uses, so it cannot drift, and it can name *every* offending
        // key instead of the first one serde happened to reach.
        let (unknown, remaining) = scan_unknown(text);
        if unknown.is_empty() {
            Error::Parse(format!("{}: {e}", path.display()))
        } else {
            Error::UnknownKeys(UnknownKeys {
                path: path.display().to_string(),
                keys: unknown,
                remaining,
            })
        }
    })?;
    validate_all(&config)?;
    Ok(config)
}

/// Every semantic check a parsed config must pass.
///
/// Factored out so the "what does this file still fail on" probe behind an
/// unknown key runs the same checks the load does. Without that, a file with an
/// unknown key *and* a runner with no `{test}` placeholder reported the key,
/// then the runner on the next run, which is the half-an-answer-twice this task
/// exists to stop (`A1-068`).
fn validate_all(config: &Config) -> Result<(), Error> {
    config.validate_capture()?;
    if let Some(runner) = &config.test {
        runner.validate()?;
    }
    if let Some(url) = &config.url {
        url.validate()?;
    }
    if let Some(version) = &config.version {
        version.validate()?;
    }
    if let Some(evidence) = &config.evidence {
        evidence.validate_observation_inputs()?;
    }
    // A scope that names no subset, or sets no policy for the one it names, is an
    // entry that reads as a decision and makes none (`A1-183`).
    for scoped in config.policy.as_deref().unwrap_or_default() {
        scoped.validate()?;
    }
    config.validate_advisory()?;
    // A declared vocabulary the tool cannot act on is an error, never a silent
    // no-op, exactly as an empty runner list is (`A1-076`).
    config.validate_tag_families()?;
    config.validate_owners()?;
    config.validate_throughput()?;
    config.validate_docs()?;
    config.validate_grouping()
}

impl EvidenceConfig {
    fn validate_observation_inputs(&self) -> Result<(), Error> {
        let Some(inputs) = &self.observation_inputs else {
            return Ok(());
        };
        if inputs.is_empty() {
            return Err(Error::BadObservationInputs(
                "the list is empty; remove it to disable reusable provider observations, or name every material input"
                    .into(),
            ));
        }
        for pattern in inputs {
            if aplan_core::task::is_vacuous_ref(pattern) {
                return Err(Error::BadObservationInputs(
                    "the list contains a blank path glob".into(),
                ));
            }
            let path = Path::new(pattern);
            if path.is_absolute()
                || pattern
                    .split('/')
                    .any(|component| matches!(component, "." | ".."))
                || path
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err(Error::BadObservationInputs(format!(
                    "entry `{pattern}` must be repository-relative and may not contain `.` or `..`"
                )));
            }
            let first = path
                .components()
                .next()
                .and_then(|component| match component {
                    std::path::Component::Normal(value) => value.to_str(),
                    _ => None,
                })
                .unwrap_or_default();
            if first.chars().any(|character| "*?[]{}!".contains(character)) {
                return Err(Error::BadObservationInputs(format!(
                    "entry `{pattern}` must begin with a literal file or directory so ordinary inspection can enumerate only its declared roots"
                )));
            }
            if matches!(first, ".git" | "target") {
                return Err(Error::BadObservationInputs(format!(
                    "entry `{pattern}` names provider metadata or generated output rather than a stable material input"
                )));
            }
            globset::Glob::new(pattern).map_err(|error| {
                Error::BadObservationInputs(format!(
                    "entry `{pattern}` is not a valid glob: {error}"
                ))
            })?;
        }
        Ok(())
    }
}

/// The nearest `.aplan.yml` at or above `start`, or `None` if there is none up to
/// the filesystem root.
/// The `.aplan.yml` discovery would read from `start`, if there is one.
///
/// Public so a caller can ask *which file* discovery would have chosen without
/// reading it, which is how `APLAN041` tells the tree's own policy from a
/// trusted one named with `--config` (`A1-273`). Comparing directories was the
/// first attempt and was wrong: a file inside the repository is not thereby the
/// file the repository declares. Revision reporting also uses the location to
/// refuse an implicitly discovered file outside the selected repository: that
/// file has no state at the named commit (`A1-194`).
#[must_use]
pub fn discovered_config(start: &Path) -> Option<PathBuf> {
    find_config(start)
}

/// Every path configuration discovery examines, nearest first.
///
/// This is the same walk [`discovered_config`] uses, exposed so an interactive
/// repair screen can say exactly what was searched without reproducing the
/// discovery algorithm. An unresolvable start yields no candidates, just as
/// discovery itself yields no selected file.
#[must_use]
pub fn config_search_paths(start: &Path) -> Vec<PathBuf> {
    // Absolute, so `ancestors()` reaches the real root rather than stopping at
    // a relative path with no parent (`.` has none).
    std::fs::canonicalize(start).map_or_else(
        |_| Vec::new(),
        |start| start.ancestors().map(|dir| dir.join(CONFIG_FILE)).collect(),
    )
}

/// Rewrite only `policy[].when.cut` entries equal to `old`.
///
/// The configuration file is reviewed prose as well as data, so this is a
/// surgical edit: comments, key order, list form, spacing and line endings are
/// retained. The result is parsed as generic YAML and compared with the exact
/// semantic edit before it is returned. `Ok(None)` means no policy named the
/// id. The caller still decides whether the name is an id or an unchanged cut
/// label; this function deliberately knows only the configuration document.
///
/// # Errors
///
/// When the input is not valid YAML, or a supported semantic edit cannot be
/// represented without changing anything else in the document.
pub fn rekey_policy_cuts(file: &str, old: &str, new: &str) -> Result<Option<String>, String> {
    let original: yaml_compat::Value =
        yaml_compat::from_str(file).map_err(|e| format!("invalid configuration: {e}"))?;
    let mut expected = original.clone();
    let wanted = replace_policy_values(&mut expected, old, new);
    if wanted == 0 {
        return Ok(None);
    }

    let mut out = String::with_capacity(file.len() + wanted * new.len().saturating_sub(old.len()));
    let mut in_policy = false;
    let mut cut_indent: Option<usize> = None;
    let mut changed = 0usize;
    for line in file.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let eol = &line[content.len()..];
        let indent = content.len() - content.trim_start_matches(' ').len();
        let trimmed = &content[indent..];

        if indent == 0 && !trimmed.is_empty() && !trimmed.starts_with('#') {
            in_policy = mapping_value(trimmed, "policy").is_some();
            cut_indent = None;
        }
        if !in_policy {
            out.push_str(line);
            continue;
        }

        if let Some(active) = cut_indent
            && indent <= active
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
        {
            cut_indent = None;
        }

        if let Some(after) = mapping_value(trimmed, "cut") {
            let leading = &content[..indent];
            let key_len = trimmed.len() - after.len();
            let key = &trimmed[..key_len];
            let value = after.trim_start_matches([' ', '\t']);
            let spacing = &after[..after.len() - value.len()];
            if value.is_empty() || value.starts_with('#') {
                cut_indent = Some(indent);
                out.push_str(line);
            } else if value.starts_with('[') {
                let (rewritten, n) = rewrite_flow_sequence(value, old, new)?;
                out.push_str(leading);
                out.push_str(key);
                out.push_str(spacing);
                out.push_str(&rewritten);
                out.push_str(eol);
                changed += n;
            } else {
                out.push_str(line);
            }
            continue;
        }

        if cut_indent.is_some() {
            let (rewritten, did_change) = rewrite_block_item(content, old, new)?;
            out.push_str(&rewritten);
            out.push_str(eol);
            changed += usize::from(did_change);
        } else {
            out.push_str(line);
        }
    }

    if changed != wanted {
        return Err(format!(
            "could identify {wanted} policy cut reference(s) in the parsed configuration but could safely rewrite only {changed}; nothing was written"
        ));
    }
    let produced: yaml_compat::Value = yaml_compat::from_str(&out)
        .map_err(|e| format!("the policy rewrite did not parse ({e}); nothing was written"))?;
    if produced != expected {
        return Err(
            "the policy rewrite changed more than the selected cut ids; nothing was written"
                .to_string(),
        );
    }
    Ok(Some(out))
}

fn mapping_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let after = line.strip_prefix(key)?;
    let after = after.trim_start_matches([' ', '\t']);
    after.strip_prefix(':')
}

fn replace_policy_values(value: &mut yaml_compat::Value, old: &str, new: &str) -> usize {
    let Some(root) = value.as_mapping_mut() else {
        return 0;
    };
    let key = yaml_compat::Value::String("policy".into());
    let Some(entries) = root
        .get_mut(&key)
        .and_then(yaml_compat::Value::as_sequence_mut)
    else {
        return 0;
    };
    let mut changed = 0;
    for entry in entries {
        let Some(entry) = entry.as_mapping_mut() else {
            continue;
        };
        let when_key = yaml_compat::Value::String("when".into());
        let Some(when) = entry
            .get_mut(&when_key)
            .and_then(yaml_compat::Value::as_mapping_mut)
        else {
            continue;
        };
        let cut_key = yaml_compat::Value::String("cut".into());
        let Some(cuts) = when
            .get_mut(&cut_key)
            .and_then(yaml_compat::Value::as_sequence_mut)
        else {
            continue;
        };
        for cut in cuts {
            if cut.as_str() == Some(old) {
                *cut = yaml_compat::Value::String(new.into());
                changed += 1;
            }
        }
    }
    changed
}

fn rewrite_flow_sequence(value: &str, old: &str, new: &str) -> Result<(String, usize), String> {
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut close = None;
    let mut chars = value.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == ']' => {
                close = Some(index);
                break;
            }
            _ => {}
        }
    }
    let close = close.ok_or_else(|| {
        "a `policy[].when.cut` flow list spans lines or has no closing bracket; nothing was written"
            .to_string()
    })?;
    let inside = &value[1..close];
    let mut out = String::from("[");
    let mut start = 0;
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut changed = 0;
    let mut chars = inside.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == ',' => {
                let (item, did_change) = rewrite_scalar_slice(&inside[start..index], old, new)?;
                out.push_str(&item);
                out.push(',');
                changed += usize::from(did_change);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let (item, did_change) = rewrite_scalar_slice(&inside[start..], old, new)?;
    out.push_str(&item);
    changed += usize::from(did_change);
    out.push_str(&value[close..]);
    Ok((out, changed))
}

fn rewrite_block_item(line: &str, old: &str, new: &str) -> Result<(String, bool), String> {
    let Some(dash) = line.find('-') else {
        return Ok((line.to_string(), false));
    };
    if !line[..dash].trim().is_empty() {
        return Ok((line.to_string(), false));
    }
    let after_dash = &line[dash + 1..];
    if !after_dash.is_empty() && !after_dash.starts_with([' ', '\t']) {
        return Ok((line.to_string(), false));
    }
    let (rewritten, changed) = rewrite_scalar_slice(after_dash, old, new)?;
    Ok((format!("{}-{rewritten}", &line[..dash]), changed))
}

fn rewrite_scalar_slice(slice: &str, old: &str, new: &str) -> Result<(String, bool), String> {
    let lead = slice.len() - slice.trim_start_matches([' ', '\t']).len();
    let mut quote = FlowQuote::None;
    let mut escaped = false;
    let mut comment = None;
    let mut previous_whitespace = false;
    let mut chars = slice.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        match quote {
            FlowQuote::Double if escaped => escaped = false,
            FlowQuote::Double if ch == '\\' => escaped = true,
            FlowQuote::Double if ch == '"' => quote = FlowQuote::None,
            FlowQuote::Single if ch == '\'' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    let _ = chars.next();
                } else {
                    quote = FlowQuote::None;
                }
            }
            FlowQuote::None if ch == '\'' => quote = FlowQuote::Single,
            FlowQuote::None if ch == '"' => quote = FlowQuote::Double,
            FlowQuote::None if ch == '#' && previous_whitespace => {
                comment = Some(index);
                break;
            }
            _ => {}
        }
        previous_whitespace = ch.is_whitespace();
    }
    let before_comment = &slice[..comment.unwrap_or(slice.len())];
    let end = before_comment.trim_end_matches([' ', '\t']).len();
    if end <= lead {
        return Ok((slice.to_string(), false));
    }
    let token = &slice[lead..end];
    let parsed: String = yaml_compat::from_str(token).map_err(|e| {
        format!("could not read a `policy[].when.cut` item `{token}` ({e}); nothing was written")
    })?;
    if parsed != old {
        return Ok((slice.to_string(), false));
    }
    let replacement = scalar_like(token, new);
    Ok((
        format!("{}{}{}", &slice[..lead], replacement, &slice[end..]),
        true,
    ))
}

#[derive(Clone, Copy)]
enum FlowQuote {
    None,
    Single,
    Double,
}

fn scalar_like(old: &str, new: &str) -> String {
    if old.starts_with('"') {
        return serde_json::to_string(new).unwrap_or_else(|_| format!("{new:?}"));
    }
    if old.starts_with('\'') {
        return format!("'{}'", new.replace('\'', "''"));
    }
    let rendered = yaml_compat::to_string(new).unwrap_or_else(|_| new.to_string());
    let rendered = rendered.trim_end_matches('\n');
    if rendered.contains('\n') {
        serde_json::to_string(new).unwrap_or_else(|_| format!("{new:?}"))
    } else {
        rendered.to_string()
    }
}

fn find_config(start: &Path) -> Option<PathBuf> {
    config_search_paths(start)
        .into_iter()
        // Stop at the first `.aplan.yml` that *exists as anything*, not just a
        // regular file: a `.aplan.yml` that is a directory (or a broken symlink)
        // must become a clear read error, not be skipped so the tool silently
        // falls back to defaults, which is the config drift this tool preaches
        // against. `load_config` then reports the unreadable file (exit 2).
        .find(|candidate| candidate.symlink_metadata().is_ok())
}

/// The JSON Schema derived from [`Config`]. `cargo xtask gen-schema` writes this
/// to `schemas/v1/config.json`, and `--check` fails on drift.
pub fn derived_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(Config);
    serde_json::to_value(schema).expect("schema is serialisable")
}

/// The starter `.aplan.yml` that `aplan init` writes: every setting shown at its
/// default, but commented out, so the file is a *valid no-op* until the reader
/// opts into a policy. That matches the tool's ethos, every rule beyond the core
/// is off until a repo turns it on, so `init` hands you a documented menu, not an
/// imposed policy. The runner and evidence examples are the exact shapes the
/// `aplan-config` tests prove parse, so uncommenting them yields a valid file.
pub const SCAFFOLD: &str = r#"# .aplan.yml, this repository's `aplan` policy.
#
# Every setting is optional and this file may be empty: an absent key uses the
# built-in default, and a command-line flag overrides whatever is set here. Editor
# autocomplete and validation come from schemas/v1/config.json. Unknown keys are a
# reported error, never silently ignored.
#
# Uncomment and edit the settings you want. Where a setting has a default, the
# value shown is that default; where it has none, the value shown is an example
# and the setting does nothing until you write one of your own.

# The directory holding the plan's task files (default: aplan), resolved against
# this file's directory. The --root flag overrides it.
# root: aplan

# Durable intake lives as one versioned YAML file per capture below
# `<root>/.inbox/`. `task`, `idea` and `note` are always valid kinds. Repository
# config may append portable kinds and named profiles; there is deliberately no
# per-user capture policy or implicit default profile.
# capture:
#   kinds: [question]
#   profiles:
#     triage:
#       kind: idea
#       labels: [inbox]
#       body: '## Context'
#       promotion:
#         phase: 2
#         effort_days: 1
#         tags: [feature]

# Warn when an active task has no `effort_days` estimate (APLAN009).
# require_effort: true

# How long a live task may sit unchanged in git before `aplan stale` names it,
# in days. A window rather than a threshold for failing anything: `stale` reports
# and never blocks, and `--older-than` overrides this for one run.
# stale_after_days: 30

# Resolve `traces:` anchors (APLAN008) against this directory, relative to the
# repo root. To turn the check off entirely instead, set `no_traces: true`.
# traces_root: .
# no_traces: false

# Traced anchors that leave this repository (APLAN034). A `traces` target
# resolving into a sibling checkout is an undeclared dependency on that checkout
# being present at the right ref: it passes on one machine and reports APLAN008
# on another. Declaring the prefix says the dependency is intended; anything not
# declared is refused rather than silently resolved.
# traces_external: ['../sibling-repo/']

# Minimum evidence strength (APLAN017). A `done` task's evidence must include at
# least one kind listed here, so a bare `note` no longer closes it the way a
# `commit` or a `test` does. Tasks carrying an exempt tag are spared, for the work
# where a dated note genuinely is the best evidence there is (a phone call).
#
# Which value is a stage rather than a taste. Start wide, as shown: a plan
# authored before its implementation has commits at best, and demanding the
# strongest rung now leaves every task uncloseable until code arrives. Narrow to
# [test] once a runner exists to point at; aplan's own plan runs that, and
# reached it after configuring a runner per crate.
# `observation_inputs` is an exhaustive trust claim about repository inputs to
# configured test discovery. Omit it unless every material input is known;
# omission disables cross-process catalogue reuse without disabling verification.
# evidence:
#   require_min_strength: [commit, test, file]
#   exempt_tags: [external]
#   observation_inputs: [Cargo.toml, Cargo.lock, 'src/**', 'tests/**']

# The same evidence dials, scoped to part of the plan. Each entry names a subset
# with the five dimensions the command line takes (within a dimension any, across
# dimensions all) and sets the dials for it. The narrowest matching scope wins per
# dial, and among equals the first entry here; the whole-plan settings above are
# the scope that names nothing, so any match beats them. An empty
# `require_min_strength: []` means nothing is required in that scope.
# policy:
#   - when:
#       phase: [1]
#       tag: [spike]
#     evidence:
#       require_min_strength: []

# Stale-`done` drift detection (APLAN018, APLAN054). When on, `aplan close`
# fingerprints traced sections and mutable file evidence. `check` reports changed
# traces; `verify` reports changed evidence. Both compare committed content. Clear
# either warning by re-reviewing and `aplan close --restamp <id>`.
# detect_drift: false

# Treat a warning as a finding, so `check` and `verify` exit 1 rather than 0 when
# the plan has warnings and nothing worse. Off by default: the advisory tier is
# useful (APLAN023 warns because a citation may be honest and you are the judge).
# Turn it on if your policy is that every gate either passes or blocks. The flags
# --deny-warnings / --no-deny-warnings override this for one run.
# deny_warnings: false

# How `aplan add` mints an id when you do not pass `--id`. `sequential` continues
# the convention the plan already uses, which is what makes it fit a repository
# rather than impose a format on one, and has dstask's documented failure: two
# authors offline derive the same next id and find out at the merge. `ulid`
# removes the collision instead of detecting it, and costs a memorable id. A
# choice made once, knowingly; ids already written are never rewritten.
# id_strategy: sequential

# What a week of this project absorbs, in effort days. Declare it and `roadmap`
# turns effort into calendar weeks; leave it out and it reports effort only. One
# ratio and nothing else about time -- no start date, no calendar, no per-cut
# duration -- so a duration stays arithmetic over numbers the plan already
# carries and cannot drift from them. Measure it rather than assume it.
# effort_days_per_week: 10

# Every non-milestone task belongs to exactly one cut, or carries the
# `cross-cutting` tag (APLAN022). A cut's `done` then proves the cut is complete
# rather than merely reached. Turning this on is also what makes a cut's shape
# load-bearing: a cut is created *with* its work, so a first plan is one
# `aplan add --from` file rather than a task and a cut added separately (README,
# under Starting a plan).
# require_cut_membership: false

# Rules this repository hears without being stopped by. Reported in full and at
# their declared severity, and left out of the verdict, so a check that is most
# useful *during* a migration can be on before the migration finishes: turn on
# require_cut_membership above, list APLAN022 here, see every task that is in no
# cut, and take it off this list when the last one is placed. The severity stays
# the rule's own everywhere it is reported; what you are configuring is the
# verdict. A code this build does not know is an error, not a silent no-op.
# advisory: [APLAN022]

# How `aplan roadmap` renders. `group_by` names a tag family (declared below)
# whose members become headings within each cut, which is how a project with a
# second task-membership dimension gets its view; aplan models one membership
# level on purpose. It must name a declared family, or a typo would invent a
# heading.
# roadmap:
#   group_by: epic

# How `aplan summary` rolls up work. With no block it groups by phase, exactly
# as before. `group_by` names a declared tag family; a task carrying more than
# one member appears in every matching row, while totals still count it once.
# summary:
#   group_by: kind

# Which documents `aplan check-docs` checks when no paths are passed. Values
# are relative to this file and may be literals or globs. A single `*` stays
# within one directory; use `**` to cross directories. Explicit command-line
# paths override this list for a one-off check.
#
# `membership` adds its own `doc` to this set automatically, so a
# membership document need not be repeated under `paths`. It says where a
# narrative declares a cut's tasks beside its `M-<cut>` milestone.
# `declared_by` is `line` (a marked line, named by `marker`) or `table`
# (a task table's first cell). Fenced code is never a declaration, so a document
# may show its own format.
# docs:
#   paths: [README.md, 'docs/*.md']
#   membership:
#     - doc: ROADMAP.md
#       declared_by: line
#       marker: "**Tasks:**"

# Tag vocabularies this plan uses (APLAN026). A second task-membership dimension
# ends up in tags, because tags are what exists, and then nothing notices a typo.
# Declaring the vocabulary makes that checkable: a tag carrying a family's prefix
# that the family does not list, or one edit away from a member, is reported.
# Tags outside every declared family stay legal and silent.
# tag_families:
#   epic: [epic-a, epic-b]
#   kind: [kind-b, kind-m, kind-r]

# Canonical values for task `owner` fields (APLAN056). With no list, owner stays
# free text. Once declared, an unfamiliar owner is reported and a one-edit typo
# gets a correction; an unassigned task stays valid. Owner selectors in
# `policy[].when` use the same vocabulary.
# owners: [ada, grace]

# A family may also say how many of it a task carries (APLAN030). The list above
# is the short form of `{members: [...]}` with no requirement, so nothing changes
# until you ask for one. `require` is optional | at-most-one | at-least-one |
# exactly-one; `status` is any | live | done | dropped | terminal; `milestones`
# is include | exclude | only. Only declared members count, so a near miss is
# APLAN026's finding rather than proof the requirement was met.
# tag_families:
#   priority:
#     members: [priority-p0, priority-p1, priority-p2]
#     require: exactly-one
#     status: live
#     milestones: exclude

# How a `url` citation is checked (APLAN016). Without this, a `url` is recorded
# and never resolved. `check` names the command to run, with `{url}` (a whole
# argument, never spliced into another, spawned with no shell) replaced by the
# citation; a check that has not answered in `timeout_seconds` (default 30) is
# reported as *not checked* rather than as a failure, because nothing was
# learned. Give the command its own limit too, as the example does: this key is
# the backstop for when it has none.
# url:
#   check: [curl, -sfI, --max-time, 10, '{url}']
#   timeout_seconds: 30
#
# It takes the same limits a test runner does, because the cap covers every
# command this tool spawns rather than the runner's alone (APLAN016 names the
# configuration when the checker is stopped, and no url is called dead on the
# strength of a check that never answered):
# url:
#   check: [curl, -sfI, --max-time, 10, '{url}']
#   max_memory: 256M
#   max_processes: 16
#   max_output: 1M
#   wrap: [nice, -n, '10', --]

# The test runner for `aplan verify --execute` (rung L3). `list` prints the
# available test names, one per line; `run` runs one, with `{test}` (a whole
# argument, never spliced into another, spawned with no shell) replaced by the
# name. This is the only place a command may be named (HLD 5.5).
# test:
#   list: [cargo, test, --, --list]
#   run: [cargo, test, --, --exact, '{test}']
#   batch:                       # optional; `run` remains the fallback
#     protocol: 1
#     run: [ci/aplan-test-batch, '{manifest}', '{result}']
#
# `skipped` prints the tests this runner will *not* run, one per line, in the
# names `list` prints. A runner that skips the cited test still exits 0, and a
# zero exit is what `--execute` reads as a pass, so a task whose every test
# citation is one of these has no executed proof at all (APLAN014). Declaring it
# is opting in: leave it out and nothing changes.
# test:
#   list: [cargo, test, --, --list]
#   run: [cargo, test, --, --exact, '{test}']
#   skipped: [cargo, test, --, --list, --ignored]
#
# What a task is expected to say about the released version (APLAN044). Off
# unless asked for. A task's *level* supplies a floor -- a unit of work a `fix`,
# a cut a `feature`, a release nothing of its own -- so with this on and nothing
# else configured, a plan that declares no impacts at all still judges clean.
#
# The words describe the **change**, not the arithmetic: `none`, `fix`,
# `feature`, `breaking`. `breaking` stays true at every version, where `major`
# does not -- below 1.0 a breaking change bumps the *minor*, and the scheme does
# that sum rather than the author.
#
# The floors are configurable rather than fixed because the right map differs
# between projects. Where cuts are planning buckets rather than deliveries, the
# cut floor is set to `none`.
# version:
#   require_impact: true
#   levels:
#     task: fix
#     cut: feature
#     release: none
#
# How the version this repository ships today is read, for `aplan version` and
# APLAN048. A command, never a parse: this tool does not know what a
# `pyproject.toml` means, which of a monorepo's manifests is the one that ships,
# or what a `version` key in one of four build backends refers to -- and a guess
# about any of those is a wrong number stated confidently.
#
# It prints the version on stdout and exits zero. The first non-empty line is
# taken, trimmed, with one leading `v` removed, because `git describe` prints one
# and `cargo pkgid` does not. It takes the same five limit keys every other
# declared command does.
#
# Without it, `aplan version <ID>` still answers what a node owes, which needs
# nothing outside the plan. What it cannot do is turn that into a number.
#
# `release_tag` is which tags name a release, as a glob, default `v*`. The window
# with no release node is everything closed since the nearest matching tag
# *reachable from HEAD* -- reachability, not recency, so a tag on a side branch
# cannot become the baseline by being cut an hour later.
# version:
#   current:
#     command: [ci/scripts/version.sh]
#     timeout_seconds: 30
#     max_output: 1M
#   release_tag: 'v*'
#
# Whether the artifact agrees with what the plan claims about compatibility
# (APLAN049). `check` is argv that **exits zero when this artifact is compatible
# with its last release**, and non-zero when it is not -- which is what
# `cargo semver-checks check-release` already does natively.
#
# Asked that way on purpose: a command that had to print `none`, `fix`, `feature`
# or `breaking` would be this tool teaching your ecosystem its vocabulary. It
# runs the checker you declare and reports what it said; it never diffs an API.
#
# It fires one way only. A checker finding a break the plan does not claim is an
# error; one finding none where the plan claims `breaking` is silent, because
# claiming more is always allowed and an API differ cannot see a behavioural
# break at all. A checker that could not be *asked* is a configuration fault, and
# no claim is called false on the strength of it.
#
# Bounded like every other declared command, and worth bounding: an API differ
# compiles the workspace, often twice. The default limit is ten minutes.
# version:
#   check:
#     command: [cargo, semver-checks, check-release]
#     timeout_seconds: 900
#     max_memory: 8G
#     max_output: 1M
#
# What a runner may spend, if this platform can hold it to that (APLAN014 when a
# cited test's own command is stopped, APLAN013 when it was `list` or `skipped`). `max_memory` becomes a cgroup's `memory.max` *and*
# `memory.swap.max`, because the first alone turns a cap into a paging target;
# `max_processes` becomes `pids.max`. On a platform with no cgroup v2, or in a
# container that was not given a cgroup subtree of its own, the run says so once
# and proceeds unprotected -- the limit is a promise this tool keeps where it can
# and names where it cannot. `max_output` is different: it bounds bytes aplan
# retains from each of stdout and stderr in its own process, defaults to 1M, and
# works on every platform. Pipes are still drained, with their first and last
# halves retained, so the bound cannot deadlock or silently cut a failure tail.
#
# `wrap` is the escape hatch beside them: a prefix every command of this runner
# runs under, for `nice`, `taskset`, a container runner, or a cgroup property
# these two dials do not model.
# test:
#   list: [cargo, test, --, --list]
#   run: [cargo, test, --, --exact, '{test}']
#   max_memory: 4G
#   max_processes: 64
#   max_output: 1M
#   wrap: [nice, -n, '10', --]
#
# What a runner spends is the harness's, not this tool's. The legacy shapes
# below start one configured provider at a time. An explicit schedule may admit
# several runners, so give a nested harness a conservative `weight`; the fan-out
# past each top-level command is still the harness's own. Measured on one
# machine, `cargo test -q -p <crate>` warm peaks at 4 processes and 156MB,
# `go test ./...` at 34 and 1.2GB, and `npm test` (vitest) at 160 and 25.6GB.
#
# The environment is passed through, so a harness's own dials already work with
# no key here: CARGO_BUILD_JOBS, GOMAXPROCS, RAYON_NUM_THREADS, or a flag in the
# command itself. That is the portable answer, and it works today.
#
# When one command cannot list every test (an umbrella of independent projects, a
# polyglot tree, unit versus e2e), `test:` takes a sequence instead. A citation
# resolves if exactly one runner lists it; two runners listing the same name is an
# error rather than a guess. `dir` is where the runner runs, relative to the root.
#
# `covers` is what the runner's tests cover, which is not the same thing: a runner
# that selects a component with --manifest-path (or an npm filter, or --rootdir)
# still runs at the root. APLAN025 uses it to catch a task citing a test from a
# component it does not own; a runner declaring neither makes no such claim.
# test:
#   - name: libs
#     covers: [libs/**]
#     list: [cargo, test, --manifest-path, libs/Cargo.toml, --, --list]
#     run: [cargo, test, --manifest-path, libs/Cargo.toml, --, --exact, '{test}']
#   - name: web
#     dir: web
#     list: [pnpm, test, --list]
#     run: [pnpm, test, '{test}']
#
# Concurrency is an explicit third shape; the mapping and sequence above stay
# serial. `capacity` is one aggregate weighted budget. A runner consumes one
# unit unless it declares `weight`, which accounts for nested fan-out.
# `exclusive` names capacity-one resources shared inside this aplan invocation;
# every name and the full weight are reserved together, so acquisition cannot
# deadlock. The names are not machine-wide or distributed locks.
# test:
#   schedule: {capacity: 4}
#   runners:
#     - name: libs
#       weight: 2
#       exclusive: [compiler-cache]
#       list: [cargo, test, --manifest-path, libs/Cargo.toml, --, --list]
#       run: [cargo, test, --manifest-path, libs/Cargo.toml, --, --exact, '{test}']
#     - name: web
#       weight: 1
#       exclusive: [browser-port]
#       dir: web
#       list: [pnpm, test, --list]
#       run: [pnpm, test, '{test}']
#
# When one runner holds tests that cover different things, name the exception
# rather than widening the claim. A repository-wide invariant usually lives in a
# tools crate and asserts a fact about a tree it is not part of; without this,
# APLAN025 reads it as proof borrowed from another component.
# test:
#   - name: xtask
#     covers: [xtask/**]
#     covers_by_test:
#       - tests: [invariants::*]     # `*` matches any run of characters
#         paths: ['**']
#     list: [cargo, test, --manifest-path, xtask/Cargo.toml, --, --list]
#     run: [cargo, test, --manifest-path, xtask/Cargo.toml, --, --exact, '{test}']
"#;

/// The dotted keys a document uses that this build does not accept, each with a
/// near match where one is a single edit away.
///
/// Walks the document rather than the error text. Descends into a mapping only
/// where the accepted-key inventory has children for that path, which is what
/// keeps `tag_families`, whose keys are the adopter's own vocabulary, from being
/// reported as twenty unknown keys. That rule is derived from the same inventory
/// as the check itself, so a future free-form map is handled without anyone
/// remembering to add it to a list (`A1-068`).
#[must_use]
pub fn unknown_keys(text: &str) -> Vec<(String, Option<String>)> {
    scan_unknown(text).0
}

/// The unknown keys, and what the file *still* fails on once they are gone.
///
/// The second half exists because reporting only the unknown keys hid whatever
/// else was wrong. `serde` stops at its first error, so a file with an unknown
/// key on line 1 and a type error on line 5 reports the unknown key; replacing
/// that message with a better list of unknown keys still leaves the type error
/// undiscovered until the adopter fixes the first thing and runs again. That is
/// the "told half the answer twice" this task set out to stop, one error along.
///
/// It is answered by removing the unknown keys and re-parsing rather than by
/// reading `serde`'s sentence: whatever fails *then* is by construction a
/// different problem, and it is exactly what the adopter would have met next.
#[must_use]
fn scan_unknown(text: &str) -> (Vec<(String, Option<String>)>, Option<String>) {
    let Ok(mut doc) = yaml_compat::from_str::<yaml_compat::Value>(text) else {
        return (Vec::new(), None);
    };
    let accepted = accepted_keys();
    let mut out = Vec::new();
    walk_unknown(&mut doc, "", &accepted, &mut out);
    if out.is_empty() {
        return (out, None);
    }
    // The document as it would be after taking this message's advice: a key with
    // a near match is *renamed* to it and one without is removed. Removing both
    // would invent errors the real fix never produces, since dropping a
    // misspelled `test.lst` leaves a runner with no `list` and reports a missing
    // field the adopter is about to supply by correcting the spelling.
    let remaining = yaml_compat::to_string(&doc).ok().and_then(|cleaned| {
        match yaml_compat::from_str::<Config>(&cleaned) {
            Err(e) => Some(strip_position(&e.to_string())),
            // Parsing is only half of loading: the semantic checks are where a
            // runner with no `{test}` placeholder or an unknown `advisory` code
            // is caught, and they are as much "what you meet next" as a type
            // error is.
            Ok(cleaned) => validate_all(&cleaned).err().map(|e| e.to_string()),
        }
    });
    (out, remaining)
}

/// One node of [`scan_unknown`]'s walk, removing what it reports.
fn walk_unknown(
    node: &mut yaml_compat::Value,
    prefix: &str,
    accepted: &[String],
    out: &mut Vec<(String, Option<String>)>,
) {
    // A sequence of runners: `test:` takes one mapping or a list of them, and
    // each element's keys are `test.*` either way.
    if let Some(seq) = node.as_sequence_mut() {
        for item in seq {
            walk_unknown(item, prefix, accepted, out);
        }
        return;
    }
    let Some(map) = node.as_mapping_mut() else {
        return;
    };
    let mut drop_these: Vec<(yaml_compat::Value, Option<String>)> = Vec::new();
    let mut keep = yaml_compat::Mapping::new();
    for (k, v) in map.iter_mut() {
        let Some(name) = k.as_str() else { continue };
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        if !accepted.iter().any(|a| a == &path) {
            let near = accepted
                .iter()
                .find(|a| aplan_core::is_one_edit_apart(a, &path))
                .cloned();
            // The last segment, because the map this key sits in is addressed by
            // the prefix already: `test.covers` is inserted here as `covers`.
            let rename_to = near
                .as_deref()
                .map(|n| n.rsplit('.').next().unwrap_or(n).to_string());
            keep.insert(k.clone(), v.clone());
            out.push((path, near));
            drop_these.push((k.clone(), rename_to));
            continue;
        }
        // Known, and worth descending into only where the inventory says this
        // path has children. Where it does not, the mapping below is the
        // adopter's own data.
        let child = format!("{path}.");
        if accepted.iter().any(|a| a.starts_with(&child)) {
            walk_unknown(v, &path, accepted, out);
        }
    }
    for (k, rename_to) in drop_these {
        map.remove(&k);
        // Renamed rather than dropped where there is a correction to apply, and
        // only where the corrected name is free: overwriting a key the file
        // already has would answer a question nobody asked.
        if let Some(name) = rename_to {
            let key = yaml_compat::Value::String(name);
            if !map.contains_key(&key)
                && let Some(v) = keep.remove(&k)
            {
                map.insert(key, v);
            }
        }
    }
}

/// Drop a `serde` message's trailing position.
///
/// The position is real, and it belongs to a document the reader does not have:
/// the file with its unknown keys already fixed. Quoting `line 1` for something
/// on line 5 of their file would be a confident wrong answer, and they get an
/// accurate one from the next run.
fn strip_position(message: &str) -> String {
    match message.find(" at line ") {
        Some(i) => message[..i].to_string(),
        None => message.to_string(),
    }
}

/// Every key this build accepts, as dotted paths (`test.covers`,
/// `evidence.require_min_strength`), sorted.
///
/// Derived from the same schema the file is validated against, so it cannot
/// drift from what the parser will actually take. It exists because a version
/// string is not a capability: an adopter staging a binary needs to ask "does
/// this build know `test.covers`?" and, before this, nothing in any output
/// answered that (`A1-060`).
#[must_use]
pub fn accepted_keys() -> Vec<String> {
    let schema = schemars::schema_for!(Config);
    let root = schema.as_value().clone();
    let defs = root
        .get("$defs")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    collect_keys(&root, &defs, "", &mut seen, &mut out);
    out.sort();
    out.dedup();
    out
}

/// Walk a schema node's `properties`, following `$ref` and the unions an
/// `Option<T>` or an untagged enum renders as, appending dotted paths.
///
/// Guarded by the set of `(reference, prefix)` pairs already expanded rather
/// than by a depth limit: following a `$ref` is not nesting, and counting it as
/// such stopped the walk one hop short of `test.covers`, which is exactly the
/// key this was built to expose.
fn collect_keys(
    node: &serde_json::Value,
    defs: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    seen: &mut BTreeSet<(String, String)>,
    out: &mut Vec<String>,
) {
    if let Some(reference) = node.get("$ref").and_then(serde_json::Value::as_str) {
        if let Some(name) = reference.strip_prefix("#/$defs/") {
            // Keyed by prefix too: the same type reached under two different
            // paths contributes keys under each.
            if seen.insert((name.to_string(), prefix.to_string()))
                && let Some(target) = defs.get(name)
            {
                collect_keys(target, defs, prefix, seen, out);
            }
        }
        return;
    }
    // `Option<T>`, and the one-or-many runner form, both render as a union;
    // every branch describes keys the parser will accept.
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(branches) = node.get(key).and_then(serde_json::Value::as_array) {
            for branch in branches {
                collect_keys(branch, defs, prefix, seen, out);
            }
        }
    }
    // A sequence of runners accepts the same keys as one runner.
    if let Some(items) = node.get("items") {
        collect_keys(items, defs, prefix, seen, out);
    }
    if let Some(props) = node
        .get("properties")
        .and_then(serde_json::Value::as_object)
    {
        for (name, child) in props {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            out.push(path.clone());
            collect_keys(child, defs, &path, seen, out);
        }
    }
}

impl Config {
    /// Reject a view grouping the tool cannot act on.
    ///
    /// # Errors
    /// When either `group_by` names a family `tag_families` does not declare.
    pub fn validate_grouping(&self) -> Result<(), Error> {
        if let Some(family) = self.roadmap.as_ref().and_then(|r| r.group_by.as_ref()) {
            self.validate_group_by("roadmap.group_by", family)?;
        }
        if let Some(family) = self.summary.as_ref().and_then(|r| r.group_by.as_ref()) {
            self.validate_group_by("summary.group_by", family)?;
        }
        Ok(())
    }

    /// Backwards-compatible entry point for callers validating the roadmap
    /// block on its own.
    ///
    /// # Errors
    /// When `roadmap.group_by` names an undeclared tag family.
    pub fn validate_roadmap(&self) -> Result<(), Error> {
        let Some(family) = self.roadmap.as_ref().and_then(|r| r.group_by.as_ref()) else {
            return Ok(());
        };
        self.validate_group_by("roadmap.group_by", family)
    }

    fn validate_group_by(&self, key: &str, family: &str) -> Result<(), Error> {
        // Grouping by a vocabulary nothing validates would render a heading a
        // typo invented, which is the defect `APLAN026` exists to prevent.
        if !self
            .tag_families
            .as_ref()
            .is_some_and(|families| families.contains_key(family))
        {
            return Err(Error::BadGrouping(format!(
                "`{key}` names `{family}`, which `tag_families` does not declare; declare it, \
                 or drop the key"
            )));
        }
        Ok(())
    }

    /// Reject a throughput nothing can be divided by (`A1-147`).
    ///
    /// Zero, negative and non-finite are all mistakes, and the alternative to
    /// refusing them is a duration rendered `inf` or a silently ignored key --
    /// both of which are a repository declaring something and the tool doing
    /// something else, which is the failure this project exists to name.
    ///
    /// # Errors
    /// When `effort_days_per_week` is present and not a positive finite number.
    pub fn validate_throughput(&self) -> Result<(), Error> {
        match self.effort_days_per_week {
            Some(w) if !w.is_finite() || w <= 0.0 => Err(Error::BadThroughput(format!(
                "it is `{w}`; it must be a positive number of effort days a week (e.g. `5` for \
                 one person, or `7.5` for five days at an assumed 1.5x)"
            ))),
            _ => Ok(()),
        }
    }

    /// Reject a tag vocabulary the tool cannot act on.
    ///
    /// # Errors
    /// When a family is empty or names a blank tag.
    pub fn validate_tag_families(&self) -> Result<(), Error> {
        let Some(families) = &self.tag_families else {
            return Ok(());
        };
        for (name, family) in families {
            if name.trim().is_empty() {
                return Err(Error::BadTagFamilies(
                    "`tag_families` has a family with a blank name".into(),
                ));
            }
            let members = family.members();
            if members.is_empty() {
                return Err(Error::BadTagFamilies(format!(
                    "`tag_families.{name}` is empty; remove it, or list the tags it contains"
                )));
            }
            if members.iter().any(|m| m.trim().is_empty()) {
                return Err(Error::BadTagFamilies(format!(
                    "`tag_families.{name}` contains a blank tag"
                )));
            }
            // A member listed twice makes `exactly-one` unsatisfiable in a way
            // nothing else would explain: a task carrying it counts once, but the
            // declaration reads as though two different tags were meant.
            let mut seen = BTreeSet::new();
            if let Some(dup) = members.iter().find(|m| !seen.insert(m.as_str())) {
                return Err(Error::BadTagFamilies(format!(
                    "`tag_families.{name}` lists `{dup}` twice"
                )));
            }
        }
        Ok(())
    }

    /// Reject an owner vocabulary the validator could not apply exactly.
    ///
    /// An empty list or a blank member declares a policy while naming no usable
    /// value. Duplicates make the list read as though two identities exist where
    /// matching sees one. Owner-scoped policy entries use the same vocabulary:
    /// accepting a spelling here that the same declaration calls non-canonical
    /// would make the repository state two incompatible policies.
    pub fn validate_owners(&self) -> Result<(), Error> {
        let Some(owners) = &self.owners else {
            return Ok(());
        };
        if owners.is_empty() {
            return Err(Error::BadOwners(
                "it is an empty list; remove the key, or name an owner".into(),
            ));
        }
        if owners
            .iter()
            .any(|owner| aplan_core::task::is_vacuous_ref(owner))
        {
            return Err(Error::BadOwners(
                "it contains a blank owner, which names no identity".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        if let Some(duplicate) = owners.iter().find(|owner| !seen.insert(owner.as_str())) {
            return Err(Error::BadOwners(format!("it lists `{duplicate}` twice")));
        }
        for owner in self
            .policy
            .iter()
            .flatten()
            .flat_map(|policy| &policy.when.owner)
        {
            if !seen.contains(owner.as_str()) {
                let suggestion = owners
                    .iter()
                    .find(|known| aplan_core::validate::is_one_edit_apart(known, owner))
                    .map_or_else(String::new, |known| format!("; did you mean `{known}`?"));
                return Err(Error::BadOwners(format!(
                    "`policy[].when.owner` names undeclared owner `{owner}`{suggestion}"
                )));
            }
        }
        Ok(())
    }
}

/// Why `aplan init` could not scaffold a config.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error(".aplan.yml already exists at {0}; refusing to overwrite")]
    Exists(PathBuf),
    #[error("could not write {0}: {1}")]
    Io(PathBuf, String),
}

/// Write [`SCAFFOLD`] to `dir/.aplan.yml`, returning the path written, or refuse
/// if anything is already at that path. The refusal is the whole safety property:
/// `init` never clobbers a policy a repo has already committed to. It writes only
/// on the success path, so a refused `init` changes nothing on disk.
pub fn init(dir: &Path) -> Result<PathBuf, InitError> {
    use std::io::Write as _;
    let path = dir.join(CONFIG_FILE);
    // `create_new` (O_EXCL) is atomic and does not follow a final symlink, so it
    // closes the check-then-write race a `symlink_metadata` + `write` would leave
    // open: nothing at that path is ever overwritten, whatever it is (a file, a
    // directory, or a symlink a concurrent writer just planted).
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => {
            f.write_all(SCAFFOLD.as_bytes())
                .map_err(|e| InitError::Io(path.clone(), e.to_string()))?;
            Ok(path)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(InitError::Exists(path)),
        Err(e) => Err(InitError::Io(path, e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proof for behaviour `rekey.policy_cut_block_comments`: comment-only
    /// suffixes on the key are YAML trivia, not a scalar value.
    #[test]
    fn policy_cut_rekey_is_surgical_and_confined_to_when_cut() {
        let input = "root: aplan\r\n# old id stays in prose: M-old\r\npolicy:\r\n  - when:\r\n      cut: [M-old, \"release\\\"]label\"] # keep\r\n    evidence:\r\n      require_min_strength: [test]\r\n  - when:\r\n      cut: # scoped by milestone id\r\n        - 'M-old' # exact id\r\n        - label\r\nversion:\r\n  levels:\r\n    cut: fix\r\n";
        let updated = rekey_policy_cuts(input, "M-old", "M-new")
            .expect("rewrite")
            .expect("two references");

        assert_eq!(
            updated,
            "root: aplan\r\n# old id stays in prose: M-old\r\npolicy:\r\n  - when:\r\n      cut: [M-new, \"release\\\"]label\"] # keep\r\n    evidence:\r\n      require_min_strength: [test]\r\n  - when:\r\n      cut: # scoped by milestone id\r\n        - 'M-new' # exact id\r\n        - label\r\nversion:\r\n  levels:\r\n    cut: fix\r\n"
        );
        assert_eq!(
            rekey_policy_cuts(input, "absent", "new").expect("no edit"),
            None
        );
    }

    #[test]
    fn policy_cut_rekey_handles_scalar_neighbours_and_single_quoted_flow_items() {
        for input in [
            "plain scalar\n",
            "policy: {not: a-sequence}\n",
            "policy:\n  - scalar\n  - evidence: {}\n  - when: scalar\n  - when:\n      cut: scalar\n",
        ] {
            assert_eq!(
                rekey_policy_cuts(input, "M-old", "M-new").expect("irrelevant shapes are safe"),
                None
            );
        }

        let input =
            "policy:\n  - when:\n      cut: ['comma, stays', 'it''s ] safe', 'M-old'] # keep\n";
        assert_eq!(
            rekey_policy_cuts(input, "M-old", "M-new")
                .expect("rewrite")
                .expect("reference"),
            "policy:\n  - when:\n      cut: ['comma, stays', 'it''s ] safe', 'M-new'] # keep\n"
        );
    }

    #[test]
    fn policy_cut_rekey_refuses_unsupported_or_ambiguous_edits() {
        let invalid = rekey_policy_cuts("policy: [\n", "M-old", "M-new")
            .expect_err("invalid YAML must be refused");
        assert!(invalid.contains("invalid configuration"), "{invalid}");

        let tagged = "policy:\n  - when:\n      cut: !!seq [M-old]\n";
        let unsupported = rekey_policy_cuts(tagged, "M-old", "M-new")
            .expect_err("a representation the surgical writer cannot preserve must be refused");
        assert!(
            unsupported.contains("safely rewrite only 0"),
            "{unsupported}"
        );

        let ambiguous = "policy:\n  - when:\n      cut: !!seq [M-old]\n    cut: [M-old]\n";
        let mismatch = rekey_policy_cuts(ambiguous, "M-old", "M-new")
            .expect_err("rewriting the wrong equal-looking field must be refused");
        assert!(
            mismatch.contains("changed more than the selected cut ids"),
            "{mismatch}"
        );
    }

    #[test]
    fn policy_cut_scalar_rewriter_covers_empty_invalid_and_quoted_items() {
        let missing_close = rewrite_flow_sequence("[M-old", "M-old", "M-new")
            .expect_err("a multiline flow list is not a safe line edit");
        assert!(
            missing_close.contains("no closing bracket"),
            "{missing_close}"
        );

        assert_eq!(
            rewrite_block_item("      no item", "M-old", "M-new").expect("plain line"),
            ("      no item".into(), false)
        );
        assert_eq!(
            rewrite_block_item("key - M-old", "M-old", "M-new").expect("not an item"),
            ("key - M-old".into(), false)
        );
        assert_eq!(
            rewrite_block_item("      -M-old", "M-old", "M-new").expect("missing spacing"),
            ("      -M-old".into(), false)
        );

        assert_eq!(
            rewrite_scalar_slice("   # comment", "M-old", "M-new").expect("empty token"),
            ("   # comment".into(), false)
        );
        assert_eq!(
            rewrite_scalar_slice(" other # keep", "M-old", "M-new").expect("near miss"),
            (" other # keep".into(), false)
        );
        let invalid =
            rewrite_scalar_slice(" [unterminated", "M-old", "M-new").expect_err("invalid scalar");
        assert!(invalid.contains("could not read"), "{invalid}");

        assert_eq!(scalar_like("\"old\"", "M-new"), "\"M-new\"");
        assert_eq!(scalar_like("'old'", "M'new"), "'M''new'");
        let multiline = scalar_like("old", "M-new\nline");
        assert_eq!(
            yaml_compat::from_str::<String>(&multiline).expect("valid YAML scalar"),
            "M-new\nline"
        );
    }

    fn runner(run: &[&str]) -> TestRunner {
        TestRunner {
            weight: None,
            exclusive: None,
            max_memory: None,
            max_processes: None,
            max_output: None,
            wrap: None,
            output_lines: None,
            expect_tests: None,
            skipped: None,
            run_selects: None,
            timeout_seconds: None,
            covers: None,
            covers_by_test: None,
            inputs_by_test: None,
            name: None,
            dir: None,
            list: vec!["true".into()],
            run: run.iter().map(|s| (*s).to_string()).collect(),
            batch: None,
        }
    }

    fn batch(run: &[&str]) -> TestBatch {
        TestBatch {
            protocol: TEST_BATCH_PROTOCOL,
            run: run.iter().map(|argument| (*argument).to_string()).collect(),
        }
    }

    fn runner_with_batch(run: &[&str]) -> TestRunner {
        let mut runner = runner(&["true", PLACEHOLDER]);
        runner.batch = Some(batch(run));
        runner
    }

    #[test]
    fn a_hostile_test_name_stays_one_unmodified_argv_element() {
        let argv = runner(&["cargo", "test", "--exact", PLACEHOLDER]).run_argv("; rm -rf / #");
        assert_eq!(argv, vec!["cargo", "test", "--exact", "; rm -rf / #"]);
        let argv = runner(&["prog", PLACEHOLDER]).run_argv("--help");
        assert_eq!(argv, vec!["prog", "--help"]);
    }

    #[test]
    fn a_substring_placeholder_runner_is_rejected() {
        let bad = runner(&["sh", "-c", &format!("run {PLACEHOLDER}")]);
        assert!(bad.validate().is_err());
    }

    #[test]
    fn run_needs_exactly_one_placeholder() {
        assert!(runner(&["true"]).validate().is_err());
        assert!(runner(&[PLACEHOLDER, PLACEHOLDER]).validate().is_err());
        assert!(runner(&["true", PLACEHOLDER]).validate().is_ok());
    }

    /// The placeholder must not be the program: `run: ['{test}']` would execute
    /// the untrusted test name as a command (the trust-boundary breach).
    #[test]
    fn run_rejects_the_placeholder_as_the_program() {
        assert!(
            runner(&[PLACEHOLDER]).validate().is_err(),
            "a bare `{{test}}` run makes the test name the program"
        );
        assert!(
            runner(&[PLACEHOLDER, "--flag"]).validate().is_err(),
            "the placeholder as argv[0] must be rejected regardless of later args"
        );
        // The safe shape (fixed program, name as an argument) still validates.
        assert!(
            runner(&["cargo", "test", "--exact", PLACEHOLDER])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn batch_paths_stay_whole_unmodified_argv_elements() {
        let batch = batch(&[
            "adapter",
            "--input",
            BATCH_MANIFEST_PLACEHOLDER,
            "--output",
            BATCH_RESULT_PLACEHOLDER,
        ]);
        assert_eq!(
            batch.run_argv("manifest ; rm", "result $(touch bad)"),
            [
                "adapter",
                "--input",
                "manifest ; rm",
                "--output",
                "result $(touch bad)",
            ]
        );
    }

    #[test]
    fn batch_requires_the_supported_protocol_and_two_exact_path_placeholders() {
        let valid = [
            "adapter",
            BATCH_MANIFEST_PLACEHOLDER,
            BATCH_RESULT_PLACEHOLDER,
        ];
        assert!(runner_with_batch(&valid).validate().is_ok());

        let mut wrong_protocol = runner_with_batch(&valid);
        wrong_protocol.batch.as_mut().unwrap().protocol = TEST_BATCH_PROTOCOL + 1;
        assert!(wrong_protocol.validate().is_err());

        for invalid in [
            vec!["adapter", BATCH_RESULT_PLACEHOLDER],
            vec!["adapter", BATCH_MANIFEST_PLACEHOLDER],
            vec![
                "adapter",
                BATCH_MANIFEST_PLACEHOLDER,
                BATCH_MANIFEST_PLACEHOLDER,
                BATCH_RESULT_PLACEHOLDER,
            ],
            vec![
                "adapter",
                BATCH_MANIFEST_PLACEHOLDER,
                BATCH_RESULT_PLACEHOLDER,
                BATCH_RESULT_PLACEHOLDER,
            ],
            vec!["adapter", "--input={manifest}", BATCH_RESULT_PLACEHOLDER],
            vec!["adapter", BATCH_MANIFEST_PLACEHOLDER, "--output={result}"],
            vec![
                "adapter",
                BATCH_MANIFEST_PLACEHOLDER,
                BATCH_RESULT_PLACEHOLDER,
                " ",
            ],
        ] {
            assert!(
                runner_with_batch(&invalid).validate().is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn batch_never_turns_a_path_or_test_name_into_the_program() {
        for invalid in [
            vec![
                BATCH_MANIFEST_PLACEHOLDER,
                "adapter",
                BATCH_RESULT_PLACEHOLDER,
            ],
            vec![
                BATCH_RESULT_PLACEHOLDER,
                "adapter",
                BATCH_MANIFEST_PLACEHOLDER,
            ],
            vec![
                "adapter",
                BATCH_MANIFEST_PLACEHOLDER,
                BATCH_RESULT_PLACEHOLDER,
                PLACEHOLDER,
            ],
        ] {
            assert!(
                runner_with_batch(&invalid).validate().is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn batch_rejects_unknown_fields_instead_of_guessing_a_protocol() {
        let error = yaml_compat::from_str::<TestBatch>(
            "protocol: 1\nrun: [adapter, '{manifest}', '{result}']\ncomplete: true\n",
        )
        .expect_err("an unknown protocol field must be refused")
        .to_string();
        assert!(error.contains("unknown field `complete`"), "{error}");
    }

    #[test]
    fn reusable_provider_inputs_are_explicit_safe_repository_globs() {
        let valid = parse_config(
            "evidence:\n  observation_inputs: [Cargo.toml, 'src/**', 'tests/*.rs']\n",
            Path::new(".aplan.yml"),
        )
        .expect("literal repository roots with valid globs are identifiable");
        assert_eq!(
            valid
                .evidence
                .expect("evidence")
                .observation_inputs
                .expect("inputs"),
            ["Cargo.toml", "src/**", "tests/*.rs"]
        );

        for (yaml, wanted) in [
            ("evidence:\n  observation_inputs: []\n", "list is empty"),
            (
                "evidence:\n  observation_inputs: ['  ']\n",
                "blank path glob",
            ),
            (
                "evidence:\n  observation_inputs: [\"\\u200b\"]\n",
                "blank path glob",
            ),
            (
                "evidence:\n  observation_inputs: ['/src/**']\n",
                "repository-relative",
            ),
            (
                "evidence:\n  observation_inputs: ['*/src/**']\n",
                "literal file or directory",
            ),
            (
                "evidence:\n  observation_inputs: ['src/../Cargo.toml']\n",
                "may not contain `.` or `..`",
            ),
            (
                "evidence:\n  observation_inputs: ['src/./lib.rs']\n",
                "may not contain `.` or `..`",
            ),
            (
                "evidence:\n  observation_inputs: ['.git/**']\n",
                "metadata or generated output",
            ),
            (
                "evidence:\n  observation_inputs: ['target/**']\n",
                "metadata or generated output",
            ),
            (
                "evidence:\n  observation_inputs: ['src/[']\n",
                "not a valid glob",
            ),
        ] {
            let error = parse_config(yaml, Path::new(".aplan.yml"))
                .expect_err("an unidentifiable provider-input declaration must be refused")
                .to_string();
            assert!(error.contains("evidence.observation_inputs"), "{error}");
            assert!(error.contains(wanted), "expected {wanted:?} in {error:?}");
        }
    }

    /// Every key `facts` advertises, in one document the parser must accept.
    ///
    /// `deny_unknown_fields` is on, so this fails if the list ever advertises a
    /// key the parser would reject, and the coverage assertion below fails if a
    /// new key is added without being represented here.
    const EVERY_ACCEPTED_KEY: &str = "\
root: aplan
capture:
  kinds: [question]
  profiles:
    triage:
      kind: question
require_effort: true
policy:
  - when:
      cut: [M-v0.9]
      phase: [2]
      tag: [spike]
      owner: [kim]
      status: [done]
    evidence:
      require_min_strength: [test]
      exempt_tags: [untestable]
tag_families:
  epic: [epic-a]
owners: [kim]
roadmap:
  group_by: epic
summary:
  group_by: epic
traces_root: .
no_traces: false
traces_external: ['../sibling']
detect_drift: false
deny_warnings: false
require_cut_membership: true
evidence:
  require_min_strength: [commit, test, file]
  exempt_tags: [external]
  max_unrelated_files: 50
  observation_inputs: [Cargo.toml, Cargo.lock, 'src/**', 'tests/**']
test:
  schedule: {capacity: 4}
  runners:
    - name: libs
      weight: 2
      exclusive: [compiler]
      dir: libs
      expect_tests: true
      run_selects: true
      output_lines: 8
      skipped: [cargo, test, '--', --list, --ignored]
      max_memory: 4G
      max_processes: 64
      max_output: 1M
      wrap: [nice, -n, '10', --]
      covers: [libs/**]
      covers_by_test:
        - tests: [invariants::*]
          paths: ['**']
      inputs_by_test:
        - tests: [invariants::*]
          paths: ['libs/**']
      list: [cargo, test]
      run: [cargo, test, '{test}']
      batch:
        protocol: 1
        run: [ci/aplan-test-batch, '{manifest}', '{result}']
      timeout_seconds: 600
version:
  require_impact: true
  release_tag: 'v*'
  current:
    command: [ci/scripts/current-version.sh]
    timeout_seconds: 30
    max_memory: 256M
    max_processes: 16
    max_output: 1M
    wrap: [nice, -n, '10', --]
  check:
    command: [cargo, semver-checks, check-release]
    timeout_seconds: 600
    max_memory: 8G
    max_processes: 64
    max_output: 1M
    wrap: [nice, -n, '10', --]
  levels:
    task: fix
    cut: feature
    release: none
url:
  check: [curl, -sfI, --max-time, '10', '{url}']
  timeout_seconds: 30
  max_memory: 256M
  max_processes: 16
  max_output: 1M
  wrap: [nice, -n, '10', --]
docs:
  paths: [README.md, 'docs/*.md']
  membership:
    - doc: ROADMAP.md
      declared_by: line
      marker: '**Tasks:**'
advisory: [APLAN022]
id_strategy: sequential
effort_days_per_week: 7.5
stale_after_days: 30
";

    #[test]
    fn a_url_check_must_take_the_url_as_a_whole_argument() {
        // The same property `{test}` has, and for the same reason: the citation
        // is data an author wrote, so it is passed as one argv element and never
        // spliced into a command. A template with the hole inside a larger string
        // would interpolate it into a shell word (`A1-041`).
        let check = |args: &[&str]| UrlCheck {
            max_memory: None,
            max_processes: None,
            max_output: None,
            wrap: None,
            timeout_seconds: None,
            check: args.iter().map(|a| (*a).to_string()).collect(),
        };
        assert!(check(&["curl", "-sfI", URL_PLACEHOLDER]).validate().is_ok());
        assert!(check(&[]).validate().is_err(), "an empty command");
        assert!(check(&["curl"]).validate().is_err(), "no placeholder");
        assert!(
            check(&["sh", "-c", "curl {url}"]).validate().is_err(),
            "the hole must be a whole argument, not part of one"
        );
        assert!(
            check(&["curl", URL_PLACEHOLDER, URL_PLACEHOLDER])
                .validate()
                .is_err(),
            "twice is ambiguous"
        );
        // A blank element spawns an empty program name and fails with `No such
        // file or directory`, which reads as a missing binary rather than the
        // config mistake it is. Caught by a surviving mutation: the check was
        // added without this (`A1-041`, audit).
        assert!(
            check(&["", URL_PLACEHOLDER]).validate().is_err(),
            "a blank program name"
        );
        assert!(
            check(&["curl", "  ", URL_PLACEHOLDER]).validate().is_err(),
            "a blank argument"
        );
    }

    /// A scope that names no subset is refused rather than read as "everything"
    /// (`A1-183`).
    #[test]
    fn a_policy_scope_that_names_nothing_is_refused() {
        let entry = ScopedPolicy {
            when: Scope::default(),
            evidence: Some(ScopedEvidence {
                require_min_strength: Some(vec![]),
                exempt_tags: None,
            }),
        };
        let text = entry.validate().unwrap_err().to_string();
        assert!(text.contains("names no subset"), "{text}");
        // And it says what to write instead, rather than only that it is wrong.
        assert!(text.contains("`cut`"), "{text}");
    }

    /// And one that selects a subset but sets nothing for it changes nothing,
    /// which is the vacuity an empty runner list is refused for (`A1-183`).
    #[test]
    fn a_policy_scope_that_sets_nothing_is_refused() {
        let entry = ScopedPolicy {
            when: Scope {
                tag: vec!["spike".into()],
                ..Scope::default()
            },
            evidence: None,
        };
        let text = entry.validate().unwrap_err().to_string();
        assert!(text.contains("sets no policy"), "{text}");
    }

    /// Narrowness is the count of dimensions named, which is what the precedence
    /// rule sorts on.
    #[test]
    fn narrowness_counts_the_dimensions_a_scope_names() {
        assert_eq!(Scope::default().named_dimensions(), 0);
        assert_eq!(
            Scope {
                tag: vec!["a".into(), "b".into()],
                ..Scope::default()
            }
            .named_dimensions(),
            1,
            "two values in one dimension is still one dimension, because listing \
             two tags widens rather than narrows"
        );
        assert_eq!(
            Scope {
                tag: vec!["a".into()],
                owner: vec!["kim".into()],
                phase: vec![2, 3],
                ..Scope::default()
            }
            .named_dimensions(),
            3,
            "several phase values still name one dimension"
        );
    }

    #[test]
    fn a_bad_url_check_names_the_url_key_not_the_test_runner() {
        // Found while building this: the first version reused `BadRunner`, so a
        // url mistake was reported as an "invalid test runner" and sent the
        // reader to the wrong half of their config.
        let err = UrlCheck {
            max_memory: None,
            max_processes: None,
            max_output: None,
            wrap: None,
            timeout_seconds: None,
            check: vec!["curl".into()],
        }
        .validate()
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("url.check"), "{text}");
        assert!(!text.contains("test runner"), "{text}");
    }

    /// The sizes an adopter writes, and the ones that must be refused.
    #[test]
    fn a_size_is_read_the_way_it_is_written() {
        assert_eq!(parse_size("4G"), Ok(4 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("512M"), Ok(512 * 1024 * 1024));
        assert_eq!(parse_size("64k"), Ok(64 * 1024));
        assert_eq!(parse_size(" 2T "), Ok(2 * 1024_u64.pow(4)));
        // A bare number is bytes, which is what `memory.max` itself takes.
        assert_eq!(parse_size("1048576"), Ok(1024 * 1024));

        // Refused rather than guessed at. `4GB` is the one that matters: read
        // as a number it would be four bytes and cap a suite at nothing.
        for bad in ["", "  ", "4GB", "four", "-1", "4.5G", "G", "0"] {
            assert!(parse_size(bad).is_err(), "`{bad}` must be refused");
        }
    }

    /// A limit nobody can act on is refused at load, like an unknown key.
    #[test]
    fn a_runner_with_an_unreadable_limit_does_not_load() {
        let bad_size = TestRunner {
            weight: None,
            exclusive: None,
            max_memory: Some("4GB".into()),
            ..runner(&["true"])
        };
        let err = bad_size.validate().expect_err("`4GB` is not a size");
        assert!(format!("{err}").contains("max_memory"), "{err}");

        let zero = TestRunner {
            weight: None,
            exclusive: None,
            max_processes: Some(0),
            ..runner(&["true"])
        };
        let err = zero
            .validate()
            .expect_err("no command runs in zero processes");
        assert!(format!("{err}").contains("max_processes"), "{err}");

        let bad_output = TestRunner {
            weight: None,
            exclusive: None,
            max_output: Some("all-of-it".into()),
            ..runner(&["true"])
        };
        let err = bad_output
            .validate()
            .expect_err("an output bound must be a size");
        assert!(format!("{err}").contains("test.max_output"), "{err}");

        // A wrapper is a command, and a command with no name is not one. Both
        // shapes YAML makes easy to write by accident are refused by name, so
        // the reader is told which key to fix rather than watching every
        // command run bare.
        let empty = TestRunner {
            weight: None,
            exclusive: None,
            wrap: Some(Vec::new()),
            ..runner(&["true"])
        };
        let err = empty.validate().expect_err("an empty prefix wraps nothing");
        assert!(format!("{err}").contains("empty list"), "{err}");

        let blank = TestRunner {
            weight: None,
            exclusive: None,
            wrap: Some(vec!["nice".into(), "  ".into()]),
            ..runner(&["true"])
        };
        let err = blank.validate().expect_err("a blank argument is not one");
        assert!(format!("{err}").contains("blank argument"), "{err}");

        // The same four keys on `url`, refused by the same code: two copies
        // of this check drifted the moment one of them grew a case, which is
        // why `validate_limits` is shared and why this asserts through the
        // other caller (`A1-310`, audit).
        let checker = |max_memory: Option<&str>,
                       max_processes: Option<u32>,
                       max_output: Option<&str>,
                       wrap: Option<Vec<String>>| UrlCheck {
            check: vec!["curl".into(), URL_PLACEHOLDER.into()],
            timeout_seconds: None,
            max_memory: max_memory.map(ToString::to_string),
            max_processes,
            max_output: max_output.map(ToString::to_string),
            wrap,
        };
        let err = checker(Some("4GB"), None, None, None)
            .validate()
            .expect_err("`4GB` is not a size");
        assert!(format!("{err}").contains("`url.max_memory`"), "{err}");
        let err = checker(None, Some(0), None, None)
            .validate()
            .expect_err("no command runs in zero processes");
        assert!(format!("{err}").contains("`url.max_processes`"), "{err}");
        let err = checker(None, None, Some("lots"), None)
            .validate()
            .expect_err("an output bound must be a size");
        assert!(format!("{err}").contains("`url.max_output`"), "{err}");
        let err = checker(None, None, None, Some(Vec::new()))
            .validate()
            .expect_err("an empty prefix wraps nothing");
        assert!(format!("{err}").contains("`url.wrap`"), "{err}");
        checker(
            Some("256M"),
            Some(16),
            Some("1M"),
            Some(vec!["nice".into()]),
        )
        .validate()
        .expect("every limit is readable");

        // And a runner that declares all four well loads.
        let good = TestRunner {
            weight: None,
            exclusive: None,
            max_memory: Some("4G".into()),
            max_processes: Some(64),
            max_output: Some("1M".into()),
            wrap: Some(vec!["nice".into(), "-n".into(), "10".into()]),
            ..runner(&["true", PLACEHOLDER])
        };
        good.validate().expect("every limit is readable");
    }

    #[test]
    fn every_key_facts_advertises_is_one_the_parser_accepts() {
        // The claim `config_keys` makes to a consumer is "this build accepts
        // these". `deny_unknown_fields` means an advertised key the parser does
        // not know fails here rather than in their image build.
        let c: Config = yaml_compat::from_str(EVERY_ACCEPTED_KEY)
            .expect("a document of every advertised key must parse");
        assert!(c.test.expect("a runner").validate().is_ok());

        // ... and the reverse: every advertised key is represented above, so a
        // new one cannot be added without this test being taught about it.
        // Pinned like `Rule::ALL`'s count is: the coverage check below matches on
        // a leaf name, so a new top-level key that happens to share a leaf with a
        // nested one would slip past it. This will not.
        let keys = accepted_keys();
        assert_eq!(
            keys.len(),
            124,
            "config keys changed; teach the fixture above, then update this count: {keys:?}"
        );
        for key in keys {
            let leaf = key.rsplit('.').next().unwrap_or(&key);
            assert!(
                EVERY_ACCEPTED_KEY.contains(&format!("{leaf}:")),
                "`{key}` is advertised but not exercised by this fixture"
            );
        }
    }

    /// The example YAML in the scaffold, one block per commented run.
    ///
    /// A line counts as an example when, with its `# ` removed, it opens a
    /// mapping key or a list item. Prose does not: `require_cut_membership
    /// above, list APLAN022 here` has no colon after the word, and every
    /// paragraph in the file is prose of that shape. Measured rather than
    /// assumed -- the split yields twenty-one blocks and no paragraph.
    fn scaffold_examples(scaffold: &str) -> Vec<String> {
        let looks_like_yaml = |line: &str| {
            let t = line.trim_start();
            t.starts_with("- ")
                || t.split_once(':').is_some_and(|(key, rest)| {
                    !key.is_empty()
                        && key
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                        && (rest.is_empty() || rest.starts_with(' '))
                })
        };
        let mut out = Vec::new();
        let mut run: Vec<&str> = Vec::new();
        for line in scaffold.lines() {
            let uncommented = line
                .strip_prefix('#')
                .map(|l| l.strip_prefix(' ').unwrap_or(l));
            match uncommented.filter(|l| looks_like_yaml(l)) {
                Some(yaml) => run.push(yaml),
                None if run.is_empty() => {}
                None => out.push(std::mem::take(&mut run).join("\n")),
            }
        }
        if !run.is_empty() {
            out.push(run.join("\n"));
        }
        out
    }

    /// **Every example in the scaffold is one this build's parser accepts.**
    ///
    /// The file `aplan init` writes is the only documentation of the policy
    /// surface an adopter meets without reading the source, and nothing read it:
    /// it is all comments, so it parses as an empty config whatever the examples
    /// say. A misspelled key, a list written as a scalar, or an enum value that
    /// was renamed would sit there being copied (`A1-292`, audit).
    ///
    /// Parsing only, not `validate`: `traces_external` names a directory that has
    /// to exist and `roadmap.group_by` names a family declared elsewhere in the
    /// file, so both are legitimately refused out of context. What a reader
    /// copies wrong is the *shape*, and that is what this reads.
    #[test]
    fn every_example_in_the_scaffold_is_one_the_parser_accepts() {
        let examples = scaffold_examples(SCAFFOLD);
        assert!(
            examples.len() > 15,
            "only {} example(s) were found, so the split has stopped matching: {examples:#?}",
            examples.len()
        );
        for example in &examples {
            let parsed = yaml_compat::from_str::<Config>(example);
            assert!(
                parsed.is_ok(),
                "the scaffold shows an example this build refuses:\n{example}\n{:?}",
                parsed.err()
            );
        }
    }

    #[test]
    fn the_example_split_reads_yaml_and_leaves_prose_alone() {
        // Both directions. The prose line is the one that made this necessary:
        // it names a key and is not a declaration of one.
        let scaffold = "# turn on require_cut_membership above, list APLAN022 here\n\
                        # deny_warnings: false\n\
                        \n\
                        # An unrelated paragraph.\n\
                        # test:\n\
                        #   list: [cargo, test]\n";
        assert_eq!(
            scaffold_examples(scaffold),
            vec![
                "deny_warnings: false".to_string(),
                "test:\n  list: [cargo, test]".to_string(),
            ]
        );
    }

    /// Whether the scaffold shows `key` as a line a reader can uncomment.
    ///
    /// As the key would be written in YAML, so a *mention* does not count: the
    /// scaffold's `advisory` comment said "turn on `require_cut_membership` above"
    /// while showing no such key, and a check keyed on the bare word would have
    /// read that as documentation.
    fn scaffold_shows(scaffold: &str, key: &str) -> bool {
        let line = format!("{key}:");
        scaffold.lines().any(|l| {
            // One `#` and at most one space, then column zero. **Not
            // `trim_start_matches`**: that flattens the indentation, so the
            // nested `evidence:` inside the `policy` example satisfied the
            // top-level `evidence` key, and deleting the top-level block would
            // have left this passing (`A1-292`, audit).
            let uncommented = l.strip_prefix('#').unwrap_or(l);
            let uncommented = uncommented.strip_prefix(' ').unwrap_or(uncommented);
            uncommented.starts_with(&line)
        })
    }

    #[test]
    fn the_scaffold_check_reads_a_key_and_not_a_mention_of_one() {
        // Both directions, for the reason the plural check has both: a checker
        // that never fires is worse than none.
        assert!(scaffold_shows("# id_strategy: sequential", "id_strategy"));
        assert!(scaffold_shows("root: aplan", "root"));
        assert!(!scaffold_shows(
            "# turn on require_cut_membership above",
            "require_cut_membership"
        ));
        assert!(!scaffold_shows("# detect_drift: false", "drift"));
        // A *nested* key of the same name is not the top-level one: the `policy`
        // example carries an `evidence:` two levels down.
        assert!(!scaffold_shows(
            "#     evidence:\n#       exempt_tags: []",
            "evidence"
        ));
    }

    /// **Every top-level key is in the file `aplan init` writes.**
    ///
    /// The scaffold's own header says "the values shown are the defaults", which
    /// is a claim of completeness, and it was not true of eight of nineteen:
    /// `policy` (added the day before), `id_strategy` (which `README.md` calls
    /// the one key whose cost everyone reading the plan pays), `docs`,
    /// `effort_days_per_week`, `stale_after_days`, `traces_external`, `url` --
    /// and `require_cut_membership`, which the scaffold's own `advisory` comment
    /// told the reader to "turn on above" while never showing it (`A1-292`).
    ///
    /// Top-level only, for `unannounced_config_keys`' reason: a nested key is
    /// part of the shape of the key above it, and demanding a line for each
    /// would ask the scaffold to restate the schema.
    #[test]
    fn every_top_level_key_is_shown_in_the_scaffold() {
        let mut top: Vec<String> = accepted_keys()
            .into_iter()
            .map(|k| k.split('.').next().unwrap_or(&k).to_string())
            .collect();
        top.sort();
        top.dedup();
        assert!(top.len() > 15, "only {} key(s) walked", top.len());
        let missing: Vec<&String> = top
            .iter()
            .filter(|k| !scaffold_shows(SCAFFOLD, k))
            .collect();
        assert!(missing.is_empty(), "`aplan init` never shows {missing:?}");
    }

    #[test]
    fn a_key_facts_does_not_advertise_is_refused() {
        // The other half of the promise: the list is exhaustive, so a key absent
        // from it must not quietly work.
        let err = yaml_compat::from_str::<Config>("requre_effort: true\n")
            .expect_err("an unknown key must be refused");
        assert!(err.to_string().contains("requre_effort"), "{err}");
        assert!(
            !accepted_keys().iter().any(|k| k == "requre_effort"),
            "a refused key must not be advertised"
        );
    }

    #[test]
    fn a_covers_by_test_the_tool_cannot_act_on_is_an_error() {
        for yaml in [
            "test:\n  covers_by_test: []\n  list: [cargo, test]\n  run: [cargo, test, '{test}']\n",
            "test:\n  covers_by_test:\n    - tests: []\n      paths: ['**']\n  \
             list: [cargo, test]\n  run: [cargo, test, '{test}']\n",
            "test:\n  covers_by_test:\n    - tests: ['a::*']\n      paths: []\n  \
             list: [cargo, test]\n  run: [cargo, test, '{test}']\n",
            "test:\n  covers_by_test:\n    - tests: ['  ']\n      paths: ['**']\n  \
             list: [cargo, test]\n  run: [cargo, test, '{test}']\n",
        ] {
            let c: Config = yaml_compat::from_str(yaml).expect("it parses");
            let err = c.test.expect("a runner").validate().unwrap_err();
            assert!(err.to_string().contains("covers_by_test"), "{err}");
        }
    }

    #[test]
    fn incremental_input_dependencies_are_distinct_validated_config() {
        let valid: Config = yaml_compat::from_str(
            "test:\n  list: [cargo, test, '--', --list]\n  run: [cargo, test, '{test}']\n  inputs_by_test:\n    - tests: ['unit::*']\n      paths: ['src/**', 'fixtures/unit/**']\n",
        )
        .expect("valid input dependencies");
        let runner = valid.test.expect("runner");
        runner.all()[0].validate().expect("valid runner");
        assert_eq!(
            runner.all()[0]
                .inputs_by_test
                .as_ref()
                .expect("input dependencies")[0]
                .paths,
            vec!["src/**", "fixtures/unit/**"]
        );
        for source in [
            "test:\n  list: [cargo, test]\n  run: [cargo, test, '{test}']\n  inputs_by_test: []\n",
            "test:\n  list: [cargo, test]\n  run: [cargo, test, '{test}']\n  inputs_by_test:\n    - tests: []\n      paths: ['src/**']\n",
            "test:\n  list: [cargo, test]\n  run: [cargo, test, '{test}']\n  inputs_by_test:\n    - tests: ['unit::*']\n      paths: []\n",
            "test:\n  list: [cargo, test]\n  run: [cargo, test, '{test}']\n  inputs_by_test:\n    - tests: ['unit::[']\n      paths: ['src/**']\n",
            "test:\n  list: [cargo, test]\n  run: [cargo, test, '{test}']\n  inputs_by_test:\n    - tests: ['unit::*']\n      paths: ['../src/**']\n",
        ] {
            let config: Config = yaml_compat::from_str(source).expect("shape parses");
            let error = config.test.expect("runner").all()[0]
                .validate()
                .expect_err("invalid dependency declaration");
            assert!(error.to_string().contains("inputs_by_test"), "{error}");
        }
        assert!(
            accepted_keys()
                .iter()
                .any(|key| key == "test.inputs_by_test.paths")
        );
    }

    #[test]
    fn accepted_keys_reaches_nested_runner_keys() {
        let keys = accepted_keys();
        // The key this exists for: an adopter asks "does this build know
        // `test.covers`?", and a top-level-only list would answer wrongly.
        for expected in [
            "root",
            "deny_warnings",
            "evidence.require_min_strength",
            "evidence.max_unrelated_files",
            "evidence.observation_inputs",
            "test",
            "test.covers",
            "test.dir",
            "test.list",
            "test.run",
        ] {
            assert!(
                keys.iter().any(|k| k == expected),
                "`{expected}` missing from {keys:?}"
            );
        }
    }

    #[test]
    fn accepted_keys_are_sorted_unique_and_derived_from_the_schema() {
        let keys = accepted_keys();
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(keys, sorted, "a consumer diffs this, so it must be stable");

        // Derived, not hand-listed: every top-level property of the schema the
        // file is validated against has to appear, or the two can drift.
        let schema = schemars::schema_for!(Config);
        let props = schema
            .as_value()
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("the config schema has properties");
        for name in props.keys() {
            assert!(keys.iter().any(|k| k == name), "`{name}` missing");
        }
    }

    #[test]
    fn grouping_by_an_undeclared_family_is_an_error() {
        // Grouping by a vocabulary nothing validates would render a heading a
        // typo invented, which is the defect `APLAN026` exists to prevent.
        let c: Config = yaml_compat::from_str("roadmap:\n  group_by: epic\n").expect("parses");
        let err = c.validate_roadmap().unwrap_err();
        assert!(err.to_string().contains("tag_families"), "{err}");
        let err = c.validate_grouping().unwrap_err();
        assert!(err.to_string().contains("roadmap.group_by"), "{err}");

        let ok: Config =
            yaml_compat::from_str("tag_families:\n  epic: [epic-a]\nroadmap:\n  group_by: epic\n")
                .expect("parses");
        assert!(ok.validate_roadmap().is_ok());

        let absent: Config = yaml_compat::from_str("root: aplan\n").expect("parses");
        assert!(absent.validate_roadmap().is_ok());
        assert!(absent.validate_grouping().is_ok());
    }

    #[test]
    fn summary_grouping_names_a_declared_family_or_is_refused() {
        let c: Config = yaml_compat::from_str("summary:\n  group_by: epic\n").expect("parses");
        let err = c.validate_grouping().unwrap_err();
        assert!(err.to_string().contains("summary.group_by"), "{err}");
        assert!(err.to_string().contains("tag_families"), "{err}");
        assert!(!err.to_string().contains("test runner"), "{err}");

        let ok: Config =
            yaml_compat::from_str("tag_families:\n  epic: [epic-a]\nsummary:\n  group_by: epic\n")
                .expect("parses");
        assert!(ok.validate_grouping().is_ok());
    }

    #[test]
    fn a_tag_family_the_tool_cannot_act_on_is_an_error() {
        for (yaml, named) in [
            ("tag_families:\n  epic: []\n", "is empty"),
            ("tag_families:\n  epic: ['  ']\n", "blank tag"),
            (
                "tag_families:\n  epic: [epic-a, epic-a]\n",
                "`epic-a` twice",
            ),
        ] {
            let c: Config = yaml_compat::from_str(yaml).expect("it parses");
            let err = c.validate_tag_families().unwrap_err();
            assert!(err.to_string().contains("epic"), "{err}");
            assert!(err.to_string().contains(named), "{err}");
            assert!(
                err.to_string()
                    .starts_with("invalid `tag_families` in `.aplan.yml`"),
                "{err}"
            );
            assert!(!err.to_string().contains("test runner"), "{err}");
        }

        let blank: Config =
            yaml_compat::from_str("tag_families:\n  '  ': [epic-a]\n").expect("it parses");
        let err = blank.validate_tag_families().unwrap_err();
        assert!(err.to_string().contains("blank name"), "{err}");
        assert!(!err.to_string().contains("test runner"), "{err}");
    }

    #[test]
    fn declaring_no_tag_families_is_valid_and_the_default() {
        let c: Config = yaml_compat::from_str("require_effort: true\n").expect("parses");
        assert!(c.tag_families.is_none());
        assert!(c.validate_tag_families().is_ok());
    }

    #[test]
    fn an_owner_vocabulary_is_nonempty_substantive_and_unique() {
        for (yaml, wanted) in [
            ("owners: []\n", "empty list"),
            ("owners: ['  ']\n", "blank owner"),
            ("owners: [\"\\u200b\"]\n", "blank owner"),
            ("owners: [ada, ada]\n", "lists `ada` twice"),
        ] {
            let config: Config = yaml_compat::from_str(yaml).expect("configuration parses");
            let error = config
                .validate_owners()
                .expect_err("an unusable vocabulary must be refused")
                .to_string();
            assert!(error.contains(wanted), "expected {wanted:?} in {error:?}");
        }

        let absent: Config = yaml_compat::from_str("{}").expect("configuration parses");
        absent
            .validate_owners()
            .expect("an absent vocabulary preserves free-text owners");
        let declared: Config =
            yaml_compat::from_str("owners: [ada, grace]\n").expect("configuration parses");
        declared
            .validate_owners()
            .expect("distinct substantive owners form a vocabulary");
    }

    #[test]
    fn owner_scopes_use_the_declared_vocabulary_when_one_exists() {
        let yaml = "owners: [kaminsod]\npolicy:\n  - when:\n      owner: [kaminsdo]\n    evidence:\n      require_min_strength: [test]\n";
        let typo: Config = yaml_compat::from_str(yaml).expect("configuration parses");
        let error = typo
            .validate_owners()
            .expect_err("a policy scope no legal task can match must be refused")
            .to_string();
        assert!(error.contains("`kaminsdo`"), "{error}");
        assert!(error.contains("did you mean `kaminsod`?"), "{error}");
        let loaded = parse_config(yaml, Path::new(".aplan.yml"))
            .expect_err("the ordinary loader must run owner validation")
            .to_string();
        assert!(loaded.contains("invalid `owners`"), "{loaded}");

        let exact: Config = yaml_compat::from_str(
            "owners: [kaminsod]\npolicy:\n  - when:\n      owner: [kaminsod]\n    evidence:\n      require_min_strength: [test]\n",
        )
        .expect("configuration parses");
        exact
            .validate_owners()
            .expect("a declared owner remains a usable policy selector");

        let open: Config = yaml_compat::from_str(
            "policy:\n  - when:\n      owner: [anyone]\n    evidence:\n      require_min_strength: [test]\n",
        )
        .expect("configuration parses");
        open.validate_owners()
            .expect("without a vocabulary owner strings remain open");
    }

    #[test]
    fn a_covers_the_tool_cannot_act_on_is_an_error() {
        // Consistent with `test: []`: config that reads as a claim and makes
        // none is rejected rather than ignored.
        for yaml in [
            "test:\n  covers: []\n  list: [cargo, test]\n  run: [cargo, test, '{test}']\n",
            "test:\n  covers: ['  ']\n  list: [cargo, test]\n  run: [cargo, test, '{test}']\n",
        ] {
            let c: Config = yaml_compat::from_str(yaml).expect("it parses");
            let err = c.test.expect("a runner").validate().unwrap_err();
            assert!(err.to_string().contains("covers"), "{err}");
        }
    }

    #[test]
    fn a_runner_can_declare_covers_without_a_dir() {
        // The shape `A1-065` exists for: locality declared by a runner that
        // still runs at the repository root.
        let c: Config = yaml_compat::from_str(
            "test:\n  - name: server\n    covers: [server/**]\n    list: [cargo, test]\n\
             \n    run: [cargo, test, '{test}']\n",
        )
        .expect("covers must parse");
        let r = c.test.expect("a runner");
        assert!(r.validate().is_ok());
        assert_eq!(
            r.all()[0].covers.as_deref(),
            Some(&["server/**".to_string()][..])
        );
        assert_eq!(r.all()[0].dir, None, "covers must not require a dir");
    }

    #[test]
    fn a_full_config_round_trips() {
        let yml = "\
require_effort: false
traces_root: docs
no_traces: false
test:
  list: [cargo, test, --, --list]
  run: [cargo, test, --exact, '{test}']
";
        let cfg: Config = yaml_compat::from_str(yml).unwrap();
        assert_eq!(cfg.require_effort, Some(false));
        assert_eq!(cfg.traces_root.as_deref(), Some("docs"));
        let runners = cfg.test.expect("runner present");
        runners.validate().unwrap();
        assert_eq!(runners.all().len(), 1, "one mapping is one runner");
        assert_eq!(
            runners.all()[0].run_argv("x"),
            vec!["cargo", "test", "--exact", "x"]
        );
    }

    #[test]
    fn capture_configuration_is_portable_strict_and_reaches_effective_policy() {
        let config = parse_config(
            "capture:\n  kinds: [question]\n  profiles:\n    triage:\n      kind: question\n      labels: [inbox]\n      body: context\n      promotion:\n        phase: 2\n        effort_days: 1\n        tags: [feature]\n",
            Path::new(".aplan.yml"),
        )
        .expect("capture config");
        let policy = config.capture_policy();
        assert!(policy.kinds().contains("task"));
        assert!(policy.kinds().contains("question"));
        assert!(policy.profiles().contains_key("triage"));

        for (input, expected) in [
            ("capture:\n  kinds: ['']\n", "blank or invisible"),
            ("capture:\n  kinds: [Task]\n", "lower-case ASCII"),
            ("capture:\n  kinds: [task]\n", "more than once"),
            (
                "capture:\n  profiles:\n    triage:\n      evidence: []\n",
                "unknown field",
            ),
            (
                "capture:\n  profiles:\n    triage:\n      promotion:\n        effort_days: 0\n",
                "positive finite",
            ),
        ] {
            let error = parse_config(input, Path::new(".aplan.yml"))
                .expect_err("invalid capture config")
                .to_string();
            assert!(error.contains(expected), "{input:?}: {error}");
        }
    }

    #[test]
    fn an_empty_config_is_all_defaults() {
        let cfg: Config = yaml_compat::from_str("{}").unwrap();
        assert_eq!(cfg.require_effort, None);
        assert!(cfg.traces_root.is_none());
        assert!(cfg.test.is_none());
        assert_eq!(cfg.detect_drift, None);
    }

    #[test]
    fn detect_drift_parses() {
        let cfg: Config = yaml_compat::from_str("detect_drift: true\n").unwrap();
        assert_eq!(cfg.detect_drift, Some(true));
    }

    #[test]
    fn root_key_parses() {
        let cfg: Config = yaml_compat::from_str("root: myplan\n").unwrap();
        assert_eq!(cfg.root.as_deref(), Some("myplan"));
    }

    #[test]
    fn the_evidence_strength_block_parses_typed_kinds() {
        let yml =
            "evidence:\n  require_min_strength: [commit, test, file]\n  exempt_tags: [phone]\n";
        let cfg: Config = yaml_compat::from_str(yml).unwrap();
        let ev = cfg.evidence.expect("evidence block");
        assert_eq!(
            ev.require_min_strength.unwrap(),
            vec![EvidenceKind::Commit, EvidenceKind::Test, EvidenceKind::File]
        );
        assert_eq!(ev.exempt_tags.unwrap(), vec!["phone".to_string()]);
        // An unknown evidence kind is a typed parse error, not silently accepted.
        assert!(
            yaml_compat::from_str::<Config>("evidence:\n  require_min_strength: [bogus]\n")
                .is_err()
        );
    }

    #[test]
    fn an_unknown_key_is_rejected_not_ignored() {
        // The ethos: a config key the tool does not understand is drift.
        let err = yaml_compat::from_str::<Config>("polciy: strict\n").unwrap_err();
        assert!(err.to_string().contains("polciy") || err.to_string().contains("unknown"));
    }

    #[test]
    fn the_derived_schema_forbids_unknown_properties() {
        let schema = derived_schema();
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    }

    fn write(dir: &Path, contents: &str) {
        std::fs::write(dir.join(CONFIG_FILE), contents).unwrap();
    }

    #[test]
    fn a_repo_with_no_config_loads_the_default() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        let cfg = load_config(dir.path()).unwrap();
        assert!(cfg.require_effort.is_none() && cfg.test.is_none());
    }

    #[test]
    fn an_empty_or_comment_only_file_is_the_default_not_an_error() {
        for body in ["", "   \n\n", "# just a comment\n"] {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .unwrap();
            write(dir.path(), body);
            let cfg = load_config(dir.path()).unwrap_or_else(|e| panic!("{body:?}: {e}"));
            assert!(cfg.require_effort.is_none(), "{body:?} should be default");
        }
    }

    /// A `.aplan.yml` that is a directory must be a reported error, not silently
    /// skipped so the tool falls back to defaults (config drift, day one).
    #[test]
    fn a_directory_named_aplan_yml_is_an_error_not_a_silent_default() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        std::fs::create_dir(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(
            matches!(load_config(dir.path()), Err(Error::Io(_))),
            "a directory `.aplan.yml` must error, not default"
        );
    }

    #[test]
    fn a_config_is_read_from_the_start_directory() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        write(dir.path(), "require_effort: false\n");
        assert_eq!(load_config(dir.path()).unwrap().require_effort, Some(false));
    }

    #[test]
    fn a_config_is_discovered_by_walking_up_from_a_subdirectory() {
        // The file is at the repo root; loading from a nested path still finds it.
        let repo = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        write(repo.path(), "require_effort: false\n");
        let nested = repo.path().join("plan").join("tasks");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(load_config(&nested).unwrap().require_effort, Some(false));
    }

    #[test]
    fn malformed_yaml_and_unknown_keys_and_bad_runners_are_reported() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();

        write(dir.path(), "require_effort: [not a bool\n");
        assert!(matches!(load_config(dir.path()), Err(Error::Parse(_))));

        // An unknown key is its own variant now, because a syntax error has one
        // cause and this has two (`A1-068`).
        write(dir.path(), "polciy: strict\n");
        assert!(matches!(
            load_config(dir.path()),
            Err(Error::UnknownKeys(_))
        ));

        write(dir.path(), "test:\n  list: [t]\n  run: [t]\n"); // no placeholder
        assert!(matches!(load_config(dir.path()), Err(Error::BadRunner(_))));
    }

    /// Both declaration shapes read the same way, which is what makes the list a
    /// shorthand rather than a second feature.
    #[test]
    fn a_list_and_a_mapping_declare_the_same_family() {
        let write_and_load = |body: &str| {
            let dir = tempfile::Builder::new()
                .tempdir_in(aplan_core::fixture::fixture_root())
                .unwrap();
            write(dir.path(), body);
            load_config(dir.path()).unwrap().tag_families.unwrap()
        };
        let short = write_and_load("tag_families:\n  epic: [epic-a, epic-b]\n");
        let long = write_and_load("tag_families:\n  epic:\n    members: [epic-a, epic-b]\n");
        let (s, l) = (&short["epic"], &long["epic"]);
        assert_eq!(s.members(), l.members());
        assert_eq!(s.require(), Cardinality::Optional);
        assert_eq!(l.require(), Cardinality::Optional);
        assert_eq!(s.scope(), (StatusScope::Any, MilestoneScope::Include));
        assert_eq!(l.scope(), (StatusScope::Any, MilestoneScope::Include));

        // And the long form carries what it was asked for.
        let asked = write_and_load(
            "tag_families:\n  p:\n    members: [a]\n    require: at-most-one\n\
             \x20   status: terminal\n    milestones: only\n",
        );
        let p = &asked["p"];
        assert_eq!(p.require(), Cardinality::AtMostOne);
        assert_eq!(p.scope(), (StatusScope::Terminal, MilestoneScope::Only));
    }

    /// A family that is neither a list nor a mapping names both shapes, rather
    /// than reporting whichever variant `serde` tried last.
    #[test]
    fn a_family_that_is_neither_shape_says_what_it_expected() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        write(dir.path(), "tag_families:\n  epic: 5\n");
        let e = load_config(dir.path()).unwrap_err().to_string();
        assert!(
            e.contains("a list of tags, or a mapping with `members`"),
            "{e}"
        );
    }

    /// A member listed twice makes `exactly-one` unsatisfiable in a way nothing
    /// else would explain, so it is refused where it is written.
    #[test]
    fn a_family_listing_a_member_twice_is_refused() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        write(dir.path(), "tag_families:\n  epic: [epic-a, epic-a]\n");
        let e = load_config(dir.path()).unwrap_err().to_string();
        assert!(e.contains("lists `epic-a` twice"), "{e}");
    }

    /// The sentence an adopter reads, in each of its shapes. The end-to-end tests
    /// drive it through the binary, where coverage is not attributed, so the
    /// wording had no unit test at all.
    #[test]
    fn the_unknown_key_message_reads_correctly_in_each_shape() {
        let one = UnknownKeys {
            path: ".aplan.yml".into(),
            keys: vec![("requre_effort".into(), Some("require_effort".into()))],
            remaining: None,
        };
        assert_eq!(
            one.to_string(),
            ".aplan.yml: unknown key `requre_effort` (did you mean `require_effort`?)"
        );

        let several = UnknownKeys {
            path: ".aplan.yml".into(),
            keys: vec![("zzz".into(), None), ("yyy".into(), None)],
            remaining: Some("require_effort: invalid type".into()),
        };
        assert_eq!(
            several.to_string(),
            ".aplan.yml: unknown keys `zzz`, `yyy`; and once those are fixed the file still \
             fails: require_effort: invalid type"
        );

        // Singular agreement, which is the half a plural-only string gets wrong.
        let one_remaining = UnknownKeys {
            path: ".aplan.yml".into(),
            keys: vec![("zzz".into(), None)],
            remaining: Some("boom".into()),
        };
        assert!(
            one_remaining.to_string().contains("once that is fixed"),
            "{one_remaining}"
        );
    }

    /// The typo case, which is the common one and must stay the loud one.
    #[test]
    fn an_unknown_key_one_edit_from_a_real_one_suggests_it() {
        assert_eq!(
            unknown_keys("requre_effort: false\n"),
            vec![(
                "requre_effort".to_string(),
                Some("require_effort".to_string())
            )]
        );
        // Nested, since the key that started this was `test.covers`.
        assert_eq!(
            unknown_keys("test:\n  covrs: [a]\n"),
            vec![("test.covrs".to_string(), Some("test.covers".to_string()))]
        );
    }

    /// A key nothing resembles is still named, without a guess: suggesting the
    /// nearest of twenty-six unrelated keys would be worse than saying nothing.
    #[test]
    fn an_unknown_key_resembling_nothing_gets_no_guess() {
        assert_eq!(
            unknown_keys("zzzzzzzz: 1\n"),
            vec![("zzzzzzzz".to_string(), None)]
        );
    }

    /// Every offending key, not the first one the parser reached. An adopter who
    /// fixes one and meets the next has been told half the answer twice.
    #[test]
    fn every_unknown_key_is_reported_at_once() {
        let found = unknown_keys("polciy: strict\nzzz: 1\nrequire_effort: true\n");
        let names: Vec<&str> = found.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            vec!["polciy", "zzz"],
            "the valid key must not appear"
        );
    }

    /// `tag_families`' keys are the adopter's own vocabulary, not this build's.
    /// Descending into them would report a correct config as twenty mistakes,
    /// and the rule that prevents it is derived from the same inventory as the
    /// check, so a future free-form map needs nobody to remember it.
    #[test]
    fn a_free_form_mapping_is_not_walked_as_if_its_keys_were_ours() {
        assert!(
            unknown_keys("tag_families:\n  epic: [epic-a]\n  kind: [kind-r]\n").is_empty(),
            "an adopter's own vocabulary was reported as unknown keys"
        );
    }

    /// `test:` takes one runner or a list of them, and a key is `test.*` either
    /// way, so the walk has to enter a sequence.
    #[test]
    fn a_runner_sequence_is_walked_element_by_element() {
        let found = unknown_keys(
            "test:\n  - name: a\n    list: [x]\n    run: [y]\n  - name: b\n    covrs: [z]\n",
        );
        assert_eq!(
            found,
            vec![("test.covrs".to_string(), Some("test.covers".to_string()))]
        );
    }

    /// The whole worked example this repository ships must report nothing, or the
    /// check is one that fires on correct configuration.
    #[test]
    fn this_repositorys_own_config_has_no_unknown_keys() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.aplan.yml"),
        )
        .expect("this repository's own config");
        assert!(unknown_keys(&text).is_empty(), "{:?}", unknown_keys(&text));
    }

    /// A syntax error is not an unknown key, and must not be reported as one:
    /// the file never parsed, so there are no keys to judge.
    #[test]
    fn a_document_that_does_not_parse_yields_no_unknown_keys() {
        assert!(unknown_keys("require_effort: [not a bool\n").is_empty());
    }

    #[test]
    fn init_writes_a_scaffold_that_loads_as_the_default() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        let path = init(dir.path()).unwrap();
        assert_eq!(path, dir.path().join(CONFIG_FILE));
        // The file it wrote is valid and a no-op: `load_config` reads it back as the
        // built-in default, so `init` imposes nothing until the user opts in.
        let cfg = load_config(dir.path()).unwrap();
        assert!(cfg.require_effort.is_none() && cfg.test.is_none() && cfg.evidence.is_none());
    }

    #[test]
    fn init_refuses_to_overwrite_an_existing_config() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .unwrap();
        write(dir.path(), "require_effort: false\n");
        assert!(matches!(init(dir.path()), Err(InitError::Exists(_))));
        // The existing policy is untouched: the refusal wrote nothing.
        assert_eq!(load_config(dir.path()).unwrap().require_effort, Some(false));
    }

    #[test]
    fn the_scaffold_examples_are_the_shapes_the_tests_prove_valid() {
        // The commented runner and evidence examples are byte-identical to the
        // shapes `a_full_config_round_trips` and `the_evidence_strength_block_...`
        // prove parse and validate, so uncommenting them yields a valid file. If
        // an example is edited, this fails until the proving test is updated too.
        //
        // What this cannot prove is that the argv is one the *program* accepts,
        // and that is how `cargo test --exact {test}` shipped here for months:
        // cargo rejects it ("unexpected argument '--exact'"), because `--exact`
        // is a libtest flag and has to follow `--`. Everything on this side of
        // the boundary was green, since substitution and parsing were all it
        // checked (`A1-054`).
        assert!(SCAFFOLD.contains("run: [cargo, test, --, --exact, '{test}']"));
        assert!(SCAFFOLD.contains("list: [cargo, test, --, --list]"));
        assert!(SCAFFOLD.contains("require_min_strength: [commit, test, file]"));
    }

    /// `A1-054`: several runners, because one `list` command cannot see the
    /// whole corpus of an umbrella or polyglot repository.
    #[test]
    fn a_single_runner_mapping_still_parses_exactly_as_written() {
        // Every existing adopter has this form. A change that made them all edit
        // their config to keep working would be a poor trade.
        let c: Config = yaml_compat::from_str(
            "test:\n  list: [cargo, test, --, --list]\n  run: [cargo, test, --, --exact, '{test}']\n",
        )
        .expect("the single-runner form must keep parsing");
        let runners = c.test.expect("a runner");
        assert_eq!(runners.all().len(), 1);
        assert!(!runners.has_explicit_schedule());
        assert_eq!(runners.schedule().capacity, 1);
        assert!(runners.validate().is_ok());
        assert_eq!(
            runners.all()[0].dir,
            None,
            "an unset dir means the repo root"
        );
    }

    #[test]
    fn a_sequence_of_runners_parses_with_name_and_dir() {
        let c: Config = yaml_compat::from_str(
            "test:\n  - name: libs\n    list: [cargo, test]\n    run: [cargo, test, '{test}']\n\
             \n  - name: web\n    dir: web\n    list: [pnpm, test]\n    run: [pnpm, test, '{test}']\n",
        )
        .expect("a sequence of runners must parse");
        let runners = c.test.expect("runners");
        assert_eq!(runners.all().len(), 2);
        assert!(!runners.has_explicit_schedule());
        assert_eq!(runners.schedule().capacity, 1);
        assert_eq!(runners.all()[1].name.as_deref(), Some("web"));
        assert_eq!(
            runners.all()[1].dir.as_deref(),
            Some("web"),
            "a runner that cannot run at the repo root is most of the point"
        );
        assert!(runners.validate().is_ok());
    }

    #[test]
    fn scheduled_runners_are_explicit_weighted_and_exclusive() {
        let c: Config = yaml_compat::from_str(
            "test:\n  schedule:\n    capacity: 4\n  runners:\n    - name: rust\n      weight: 3\n      exclusive: [compiler, database]\n      list: [cargo, test]\n      run: [cargo, test, '{test}']\n    - name: docs\n      list: [mdtest, --list]\n      run: [mdtest, '{test}']\n",
        )
        .expect("the scheduled form parses");
        let runners = c.test.expect("runners");
        assert_eq!(runners.schedule().capacity, 4);
        assert_eq!(runners.all().len(), 2);
        assert_eq!(runners.all()[0].weight(), 3);
        assert_eq!(runners.all()[1].weight(), 1);
        assert_eq!(
            runners.all()[0].exclusive_resources(),
            ["compiler".to_string(), "database".to_string()]
                .into_iter()
                .collect()
        );
        runners.validate().expect("a usable weighted schedule");
    }

    #[test]
    fn unusable_provider_schedules_are_refused_at_load() {
        for (yaml, wanted) in [
            (
                "test:\n  schedule: {capacity: 0}\n  runners:\n    - list: ['true']\n      run: ['true', '{test}']\n",
                "capacity",
            ),
            (
                "test:\n  schedule: {capacity: 1}\n  runners:\n    - weight: 2\n      list: ['true']\n      run: ['true', '{test}']\n",
                "weight 2 exceeds",
            ),
            (
                "test:\n  schedule: {capacity: 2}\n  runners:\n    - weight: 0\n      list: ['true']\n      run: ['true', '{test}']\n",
                "weight is 0",
            ),
            (
                "test:\n  schedule: {capacity: 2}\n  runners:\n    - exclusive: []\n      list: ['true']\n      run: ['true', '{test}']\n",
                "exclusive resource list is empty",
            ),
            (
                "test:\n  schedule: {capacity: 2}\n  runners:\n    - exclusive: [' port ']\n      list: ['true']\n      run: ['true', '{test}']\n",
                "no surrounding whitespace",
            ),
            (
                "test:\n  schedule: {capacity: 2}\n  runners:\n    - exclusive: [\"port\\u200b\"]\n      list: ['true']\n      run: ['true', '{test}']\n",
                "invisible formatting",
            ),
            (
                "test:\n  schedule: {capacity: 2}\n  runners:\n    - exclusive: [port, port]\n      list: ['true']\n      run: ['true', '{test}']\n",
                "repeats exclusive resource",
            ),
            (
                "test:\n  schedule: {capacity: 2}\n  runners: []\n",
                "empty list",
            ),
        ] {
            let error = parse_config(yaml, Path::new(".aplan.yml"))
                .expect_err("an unusable schedule must be refused")
                .to_string();
            assert!(error.contains(wanted), "expected {wanted:?} in {error:?}");
        }

        for yaml in [
            "test:\n  weight: 1\n  list: ['true']\n  run: ['true', '{test}']\n",
            "test:\n  - exclusive: [port]\n    list: ['true']\n    run: ['true', '{test}']\n",
        ] {
            let error = parse_config(yaml, Path::new(".aplan.yml"))
                .expect_err("scheduler controls outside a schedule must be refused")
                .to_string();
            assert!(error.contains("require the explicit"), "{error}");
        }
    }

    #[test]
    fn every_runner_in_a_sequence_is_validated_not_just_the_first() {
        // The trust boundary is per runner: one unsafe template is enough.
        let c: Config = yaml_compat::from_str(
            "test:\n  - list: ['true']\n    run: [cargo, '{test}']\n\
             \n  - list: ['true']\n    run: ['{test}']\n",
        )
        .expect("parses");
        let err = c.test.unwrap().validate().unwrap_err();
        assert!(
            err.to_string().contains("may not be the command"),
            "the second runner makes the test name the program: {err}"
        );
    }

    #[test]
    fn an_empty_runner_list_is_a_config_error_not_a_silent_no_op() {
        // `test: []` parses, and would then reject every `test` citation with a
        // message implying a runner exists.
        let c: Config = yaml_compat::from_str("test: []\n").expect("parses");
        let err = c.test.unwrap().validate().unwrap_err();
        assert!(err.to_string().contains("empty list"), "{err}");
    }

    #[test]
    fn a_typo_in_a_runner_names_the_key_rather_than_the_enum() {
        // A derived `untagged` enum reports only "data did not match any variant",
        // which hides the one thing the author needs.
        for yaml in [
            "test:\n  list: ['true']\n  run: ['true', '{test}']\n  typo: x\n",
            "test:\n  - list: ['true']\n    run: ['true', '{test}']\n    typo: x\n",
        ] {
            let err = yaml_compat::from_str::<Config>(yaml)
                .unwrap_err()
                .to_string();
            assert!(err.contains("unknown field `typo`"), "{err}");
            assert!(
                err.contains("`dir`"),
                "the message should list what is allowed: {err}"
            );
        }
    }

    /// **The version is read by a command, never parsed** (ADR-0040).
    ///
    /// The key carries the same five limits every other spawning block does,
    /// because ADR-0033 covers *every* spawned command: `url.check` was declared
    /// with `max_memory` and never applied it, and this is the block that would
    /// have repeated it.
    #[test]
    fn the_current_version_is_a_declared_command_with_the_usual_limits() {
        let c: Config = yaml_compat::from_str(
            "version:\n  current:\n    command: [ci/version.sh]\n    timeout_seconds: 5\n    \
             max_memory: 256M\n    max_processes: 8\n    max_output: 1M\n    wrap: [nice, '--']\n  \
             release_tag: 'release-*'\n",
        )
        .expect("it parses");
        let v = c.version.as_ref().expect("the block");
        let current = v.current.as_ref().expect("the command");
        assert_eq!(current.command, vec!["ci/version.sh".to_string()]);
        assert_eq!(current.timeout_seconds, Some(5));
        assert_eq!(current.max_memory.as_deref(), Some("256M"));
        assert_eq!(current.max_processes, Some(8));
        assert_eq!(current.max_output.as_deref(), Some("1M"));
        assert_eq!(v.release_tag(), "release-*");
        v.validate().expect("it is usable");
        // And the default, which is what every plan measured so far tags with.
        let bare: Config =
            yaml_compat::from_str("version:\n  require_impact: true\n").expect("it parses");
        assert_eq!(
            bare.version.as_ref().expect("the block").release_tag(),
            DEFAULT_RELEASE_TAG
        );
    }

    /// The three public declarations stay shaped for their own questions, but
    /// the policy around every spawned command is one typed contract
    /// (`A1-185`). If a shared limit is added to only one view, this stops
    /// compiling or comparing equal before execution can drift.
    #[test]
    fn test_url_and_version_expose_one_command_policy_contract() {
        let c: Config = yaml_compat::from_str(
            r"test:
  list: [tests, list]
  run: [tests, run, '{test}']
  timeout_seconds: 17
  max_memory: 256M
  max_processes: 8
  max_output: 2M
  wrap: [cage]
url:
  check: [links, '{url}']
  timeout_seconds: 17
  max_memory: 256M
  max_processes: 8
  max_output: 2M
  wrap: [cage]
version:
  current:
    command: [version]
    timeout_seconds: 17
    max_memory: 256M
    max_processes: 8
    max_output: 2M
    wrap: [cage]
",
        )
        .expect("all three declaration shapes parse");
        let test = c.test.as_ref().expect("test").all()[0].command_policy();
        let url = c.url.as_ref().expect("url").command_policy();
        let version = c
            .version
            .as_ref()
            .and_then(|version| version.current.as_ref())
            .expect("version.current")
            .command_policy();

        assert_eq!(test, url);
        assert_eq!(url, version);
        assert_eq!(test.timeout_or(600), 17);
    }

    /// A `version` block that could not be acted on is refused at load, and the
    /// message names the key rather than the block (`A1-310`'s lesson).
    ///
    /// **Both declared commands, because they share one validator** (`A1-314`):
    /// a message naming `version.current` for a `version.check` mistake would
    /// send the reader to the wrong key, which is the defect `BadUrlCheck` has
    /// its own variant for.
    #[test]
    fn an_unusable_version_block_is_refused_by_name() {
        let cases = [
            (
                "version:\n  check:\n    command: []\n",
                "`version.check.command`",
            ),
            (
                "version:\n  check:\n    command: [x]\n    max_processes: 0\n",
                "`version.check.max_processes`",
            ),
            ("version:\n  current:\n    command: []\n", "command"),
            ("version:\n  current:\n    command: ['']\n", "blank"),
            ("version:\n  release_tag: '  '\n", "release_tag"),
            (
                "version:\n  current:\n    command: [x]\n    max_memory: lots\n",
                "max_memory",
            ),
            (
                "version:\n  current:\n    command: [x]\n    max_output: lots\n",
                "max_output",
            ),
            (
                "version:\n  current:\n    command: [x]\n    max_processes: 0\n",
                "max_processes",
            ),
            (
                "version:\n  current:\n    command: [x]\n    wrap: []\n",
                "wrap",
            ),
        ];
        for (yaml, wanted) in cases {
            let c: Config = yaml_compat::from_str(yaml).expect("it parses");
            let err = c
                .version
                .as_ref()
                .expect("the block")
                .validate()
                .expect_err(yaml);
            let said = err.to_string();
            assert!(
                said.contains(wanted) && said.contains("`version`"),
                "{yaml}\n  said: {said}"
            );
        }
    }

    /// An empty `release_tag` is refused **whether or not a command is
    /// declared**, which is the ordering defect this test exists for: written
    /// the obvious way, the check sat after the early return and an empty glob
    /// silently widened the window to the whole of history.
    #[test]
    fn an_empty_release_tag_is_refused_without_a_command_too() {
        let c: Config = yaml_compat::from_str("version:\n  release_tag: ''\n").expect("parses");
        c.version
            .as_ref()
            .expect("the block")
            .validate()
            .expect_err("an empty glob matches no tag");
    }

    /// The version policy is off until asked for, and its floors fall back to
    /// the shipped ones a level at a time (`A1-312`, ADR-0034).
    #[test]
    fn the_version_floors_fill_in_only_what_the_adopter_left_out() {
        use aplan_core::task::VersionImpact as V;

        let off: Config = yaml_compat::from_str("").expect("empty loads");
        assert!(off.version.is_none(), "silence asks for nothing");

        // Declared, but with `require_impact` left out: a `version` block that
        // only sets floors has not turned the rule on, and reading `levels` as
        // consent would enable a gate nobody asked for.
        let floors_only: Config =
            yaml_compat::from_str("version:\n  levels:\n    cut: none\n").expect("it parses");
        let p = floors_only.version.as_ref().expect("the block is there");
        assert!(!p.requires_impact());
        assert_eq!(
            p.floors(),
            aplan_core::task::VersionFloors {
                task: V::Fix,
                cut: V::None,
                release: V::None
            },
            "the level named is the adopter's and the rest are the shipped ones"
        );

        let on: Config =
            yaml_compat::from_str("version:\n  require_impact: true\n").expect("it parses");
        let p = on.version.as_ref().expect("the block is there");
        assert!(p.requires_impact());
        assert_eq!(
            p.floors(),
            aplan_core::task::VersionFloors::default(),
            "asking for the rule and nothing else gets the shipped floors"
        );

        // Every level overridden, including one raised: the map is the adopter's
        // in both directions.
        let all: Config = yaml_compat::from_str(
            "version:\n  require_impact: true\n  levels:\n    task: none\n    cut: fix\n    \
             release: breaking\n",
        )
        .expect("it parses");
        assert_eq!(
            all.version.expect("the block").floors(),
            aplan_core::task::VersionFloors {
                task: V::None,
                cut: V::Fix,
                release: V::Breaking
            }
        );
    }
}

#[cfg(test)]
mod throughput_tests {
    use super::*;

    /// A throughput nothing can be divided by is refused at load, not ignored
    /// (`A1-147`). Ignoring it would be the tool doing something other than what
    /// the repository declared, which is the drift this project is about.
    #[test]
    fn an_unusable_throughput_is_refused_and_says_what_one_looks_like() {
        for bad in ["0", "-5", "0.0"] {
            let text = format!("effort_days_per_week: {bad}\n");
            let c: Config = yaml_compat::from_str(&text).expect("it is a number");
            let err = c.validate_throughput().expect_err("must be refused");
            let msg = err.to_string();
            assert!(
                msg.contains("`effort_days_per_week` in `.aplan.yml`") && msg.contains("positive"),
                "the message must name the key and what a good value is: {msg}"
            );
        }
    }

    /// The shapes that are fine: absent, and any positive number.
    #[test]
    fn a_positive_throughput_and_no_throughput_both_load() {
        let none: Config = yaml_compat::from_str("").expect("empty loads");
        assert_eq!(none.effort_days_per_week, None);
        none.validate_throughput()
            .expect("silence is not a mistake");

        let declared: Config =
            yaml_compat::from_str("effort_days_per_week: 7.5\n").expect("it parses");
        assert_eq!(declared.effort_days_per_week, Some(7.5));
        declared
            .validate_throughput()
            .expect("a positive number is fine");
    }
}

#[cfg(test)]
mod docs_tests {
    use super::*;

    /// A declaration that can never select a document is refused while loading
    /// configuration, before `check-docs` can mistake it for an empty clean set
    /// (`A1-203`).
    #[test]
    fn unusable_document_path_declarations_are_refused() {
        for (yaml, wanted) in [
            ("docs:\n  paths: []\n", "empty list"),
            ("docs:\n  paths: ['  ']\n", "blank pattern"),
            ("docs:\n  paths: [\"\\u200b\"]\n", "blank pattern"),
            ("docs:\n  paths: ['docs/[']\n", "not a usable glob"),
        ] {
            let config: Config = yaml_compat::from_str(yaml).expect("the shape parses");
            let message = config
                .validate_docs()
                .expect_err("a vacuous declaration must be refused")
                .to_string();
            assert!(
                message.contains("`docs.paths`") && message.contains(wanted),
                "the message must name the declaration and its defect: {message}"
            );
        }
    }

    /// A `docs.membership` the check could not be run from is refused at load
    /// rather than ignored (`A1-165`).
    ///
    /// The `table` + `marker` case is the one worth having: serde accepts it
    /// happily and the tool has no use for the marker, so silently dropping it
    /// would leave the repository believing a declaration nothing honours -- which
    /// is the drift this whole tool argues against, arriving in its own config.
    #[test]
    fn an_unusable_membership_declaration_is_refused() {
        for (yaml, wanted) in [
            (
                "docs:\n  membership:\n    - doc: ROADMAP.md\n      declared_by: line\n",
                "marker",
            ),
            (
                "docs:\n  membership:\n    - doc: ROADMAP.md\n      declared_by: line\n      marker: \"  \"\n",
                "every line begins with",
            ),
            (
                "docs:\n  membership:\n    - doc: PLAN.md\n      declared_by: table\n      marker: \"**Tasks:**\"\n",
                "does not use",
            ),
            ("docs:\n  membership: []\n", "empty list"),
            // Found by the audit: serde takes it, the path canonicalises to the
            // config's own directory, and it silently matches no document ever.
            (
                "docs:\n  membership:\n    - doc: \"\"\n      declared_by: table\n",
                "names no document",
            ),
            (
                "docs:\n  membership:\n    - doc: \"\\u200b\"\n      declared_by: table\n",
                "names no document",
            ),
            (
                "docs:\n  membership:\n    - doc: PLAN.md\n      declared_by: line\n      marker: \"\\u200b\"\n",
                "every line begins with",
            ),
        ] {
            let c: Config = yaml_compat::from_str(yaml).expect("it parses");
            let msg = c.validate_docs().expect_err("must be refused").to_string();
            assert!(
                msg.contains("`docs.membership` in `.aplan.yml`") && msg.contains(wanted),
                "the message must name the key and the mistake: {msg}"
            );
        }
    }

    /// The shapes that are fine: absent, and each of the two declarations.
    #[test]
    fn the_membership_declarations_that_work_all_load() {
        let none: Config = yaml_compat::from_str("").expect("empty loads");
        assert!(none.docs.is_none());
        none.validate_docs().expect("silence is not a mistake");

        for yaml in [
            "docs:\n  membership:\n    - doc: ROADMAP.md\n      declared_by: line\n      marker: \"**Tasks:**\"\n",
            "docs:\n  membership:\n    - doc: PLAN.md\n      declared_by: table\n",
            // `docs` with no `membership` is a section that asks for nothing.
            "docs: {}\n",
        ] {
            let c: Config = yaml_compat::from_str(yaml).expect("it parses");
            c.validate_docs().expect("a usable declaration");
        }
    }

    /// The declared set is a union rather than a list of runs: globs may
    /// overlap literals, and a membership document is already a document
    /// declaration. Each physical file is returned once, in a stable order.
    #[test]
    fn document_paths_expand_globs_deduplicate_and_fold_in_membership() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("docs/nested")).expect("directories");
        for path in ["docs/a.md", "docs/b.md", "docs/nested/c.md", "PLAN.md"] {
            std::fs::write(dir.path().join(path), path).expect("document");
        }
        let yaml = "\
docs:
  paths:
    - docs/*.md
    - docs/**/*.md
    - PLAN.md
  membership:
    - doc: PLAN.md
      declared_by: table
";
        let config: Config = yaml_compat::from_str(yaml).expect("config");
        config.validate_docs().expect("valid declarations");
        let documents = config
            .docs
            .expect("docs")
            .document_paths(dir.path())
            .expect("resolved documents");
        let relative: Vec<PathBuf> = documents
            .iter()
            .map(|path| {
                path.strip_prefix(dir.path())
                    .expect("inside fixture")
                    .into()
            })
            .collect();
        assert_eq!(
            relative,
            [
                PathBuf::from("PLAN.md"),
                PathBuf::from("docs/a.md"),
                PathBuf::from("docs/b.md"),
                PathBuf::from("docs/nested/c.md"),
            ]
        );
    }

    /// Every declaration has to bind now. A missing literal, a glob whose last
    /// file disappeared, and a directory passed as though it were a document
    /// are three different easy ways to make a gate check nothing.
    #[test]
    fn document_path_resolution_refuses_every_vacuous_filesystem_shape() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("docs")).expect("directory");

        for (yaml, wanted) in [
            ("docs:\n  paths: [missing.md]\n", "not a file"),
            ("docs:\n  paths: ['docs/*.md']\n", "matched no files"),
            ("docs:\n  paths: [docs]\n", "not a file"),
            (
                "docs:\n  membership:\n    - doc: missing.md\n      declared_by: table\n",
                "`docs.membership`",
            ),
        ] {
            let config: Config = yaml_compat::from_str(yaml).expect("config");
            config.validate_docs().expect("syntactically valid");
            let message = config
                .docs
                .expect("docs")
                .document_paths(dir.path())
                .expect_err("the empty gate must be refused")
                .to_string();
            assert!(
                message.contains(wanted),
                "expected {wanted:?} in {message:?}"
            );
        }
    }
}

#[cfg(test)]
mod published_schema_tests {
    use super::*;

    /// **Every key the schema publishes says what it is.**
    ///
    /// `run_selects` carried no description for the whole of its existence, and
    /// not because nobody wrote one: the paragraph was written, and sat above
    /// `max_memory` with no blank line between them, so Rust read the two
    /// blocks as one comment. The published schema gave the memory cap an
    /// opening paragraph about test selection and gave the selection key
    /// nothing at all.
    ///
    /// A doc comment that drifts onto the next field is not a thing review
    /// catches -- both keys look documented in the source, and the diff that
    /// did it moved a field, not a comment. The generated artifact is where it
    /// is visible, so that is where it is asserted. Found while auditing
    /// `A1-354`'s own fixture, which had to enumerate these keys and read their
    /// documentation to do it.
    #[test]
    fn every_published_key_says_what_it_is() {
        let schema = derived_schema();
        let mut silent = std::collections::BTreeSet::new();
        describe(&schema, "", &mut silent);
        if let Some(defs) = schema.get("$defs").and_then(serde_json::Value::as_object) {
            for (name, def) in defs {
                describe(def, name, &mut silent);
            }
        }
        assert!(
            silent.is_empty(),
            "{} configuration key(s) published with no description, so an adopter reading the \
             schema is told the key exists and nothing else: {:?}",
            silent.len(),
            silent
        );
        // A walk that finds nothing to check passes. The surface is large and
        // this number only has to be a floor the walk cannot reach by accident.
        let mut all = std::collections::BTreeSet::new();
        count(&schema, "", &mut all);
        if let Some(defs) = schema.get("$defs").and_then(serde_json::Value::as_object) {
            for (name, def) in defs {
                count(def, name, &mut all);
            }
        }
        assert!(
            all.len() > 40,
            "the walk saw only {} key(s), so it cannot have read the schema",
            all.len()
        );
    }

    fn describe(
        node: &serde_json::Value,
        path: &str,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        if let Some(properties) = node
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (key, value) in properties {
                let child = format!("{path}.{key}");
                let said = value
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if said.trim().is_empty() {
                    out.insert(child.clone());
                }
                describe(value, &child, out);
            }
        }
    }

    fn count(node: &serde_json::Value, path: &str, out: &mut std::collections::BTreeSet<String>) {
        if let Some(properties) = node
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (key, value) in properties {
                let child = format!("{path}.{key}");
                out.insert(child.clone());
                count(value, &child, out);
            }
        }
    }
}
