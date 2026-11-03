# AI-assisted synthetic TUI evaluation

Status: Adopted by `A1-524`; adversarial protocol and evidence audit revised by `A1-527`.

This proposal defines a development-time evaluation harness in which language-model agents act
against aplan's paper prototype and, later, its real terminal UI. It uses the existing local
authentication and configured defaults of Claude Code, Codex, OpenCode and OMP. It does not put a
model in aplan, add a product dependency or grant an agent access to the repository. It supplies a
formative structural review for `A1-418` and a repeated product-wide release gate in `A1-528`; it
does not turn either layer into human evidence or replace the pre-public human research in
`A1-449`.

## Decision

Build this as a local, opt-in evaluation layer. Use it as a formative structural review for
`A1-418`, as a supplementary discovery layer during implementation and as the pre-public product
gate defined by `A1-528`.

The useful claim is narrow: repeated synthetic actors can cheaply explore more routes, terminal
sizes, recovery states and wording than a small human study, and can turn some failures into exact
deterministic regressions. They can establish whether a fixed paper deck exposes a traversable,
internally consistent route and preserves named safety invariants. That narrower claim is enough to
stop unavailable recruitment from blocking implementation; it is not evidence about people.

The unsafe claim is that a model is a user or that agreement among models proves usability. It
does not. A model has different perception, prior knowledge, patience, motor behavior and social
incentives from a person. It is unusually good at interpreting terse technical text and unusually
bad as a proxy for accessibility, anxiety, fatigue, trust and learned work habits. Its output also
changes as a provider silently changes a default model.

Consequently:

- synthetic runs may support only a named structural or product claim, never a participant count
  or a claim that unfamiliar people understood the design;
- synthetic runs do not count toward the five participants in `A1-449`;
- a model verdict never closes a task, verifies evidence or declares a release usable;
- a deterministic invariant exposed by a synthetic route is a real product failure;
- a model observation is a candidate finding until a maintainer confirms it or it is reduced to a
  deterministic test;
- a clean synthetic run means only that the actor completed that run and no configured oracle
  failed.

This boundary follows aplan's thesis. The model may propose where to look, but a machine-resolved
fact or an observed human behavior must carry the conclusion.

## Research basis

The research supports augmentation, not substitution:

- [SimUser](https://doi.org/10.1145/3613904.3642481) generated useful heuristic feedback for a
  simple smartwatch interface, but similarity to human feedback varied from 35.7% to 100% by user
  group and category. The authors position it as a prototyping aid rather than a replacement for
  human usability tests.
- [UXAgent](https://arxiv.org/abs/2502.12561) explicitly frames simulated users as a way to refine
  the study design before a real human study. That is the role proposed here.
- [Self-Preference Bias in LLM-as-a-Judge](https://arxiv.org/abs/2410.21819) reports that an LLM
  evaluator can prefer text that is more familiar to it than human evaluators do. This makes an
  actor grading its own interaction particularly weak evidence.
- [Judging the Judges](https://arxiv.org/abs/2406.07791) finds position bias across multiple LLM
  judges and tasks. It motivates blinded, reordered critique inputs and forbids treating a single
  judge or simple majority as truth.

The implementation approach is also supported by the products' documented automation surfaces:

- [Claude Code CLI](https://code.claude.com/docs/en/cli-reference) documents print
  mode, structured output, tool selection, restricted operation and non-persistent sessions.
- [Codex CLI](https://developers.openai.com/codex/cli/reference/) documents non-interactive `exec`,
  JSON events, output schemas, ephemeral sessions and sandbox selection. OpenAI's
  [iterative repair example](https://developers.openai.com/cookbook/examples/codex/build_iterative_repair_loops_with_codex)
  also separates structured model review from deterministic validation and preserves per-run
  artifacts.
- [OpenCode CLI](https://docs.opencode.ai/docs/cli/) documents headless `run` and standalone
  operation; its [permissions](https://dev.opencode.ai/docs/permissions/) can deny tools.
- [OMP CLI reference](https://github.com/can1357/oh-my-pi/blob/main/docs/cli-reference.md) documents
  print, JSON and RPC modes together with no-tool and no-session operation.

These references justify that the CLIs can be adapters. They do not establish that the simulated
behavior is representative; only calibration against human observations can measure that.

### Bias and leakage controls

No model-based evaluation is unbiased. The accepted method makes named failure modes observable
and fail-closed instead of claiming to remove them:

| Threat | Control | Residual limitation |
|---|---|---|
| qualification or survivorship bias | neutral transport, exact-schema, instruction-following and fixed twelve-call stability canaries; evaluation screens never qualify an adapter; every scheduled session remains in the cohort even when an infrastructure failure stops its unattempted suffix | the fixed reliability threshold selects for stable transports, an unavailable configured default still changes which models contribute, and a qualified transport can still fail later |
| repair/retry selection | the first invalid or ungrounded action ends the prompt; fixed repetitions cannot be replaced | provider-internal transport retries may be opaque to a CLI |
| source, instruction or history leakage | every adapter runs in a clear-environment `bubblewrap` allowlist with only projected authentication/defaults; repository and prior-session reads are probed by the OS; SQLite projection refuses an unknown schema before copying it | the installed CLI and provider are trusted to use credentials only for authentication, and a model may have general prior knowledge from training |
| confirmation and anchoring | actors receive no state ids, driver effects or oracle results; critics see only viewport/screen/action records under an open-ended rubric, including the attempted record from an incomplete or rejected action | the fixed goals and rubric still frame what is noticed |
| self-preference and provider correlation | every session that reaches critique after visible interaction uses a critic from a different known provider family | infrastructure-failed cells have no useful complete transcript to critique; provider families and training corpora remain correlated, and cross-family is not independence |
| position and recency effects | every adapter/profile cell receives both canonical and reverse scenario order; the fixed cohort order counterbalances profile, adapter and repetition instead of running one profile first | two scenario orders and one session-order square do not measure every sequence or time interaction |
| critic hallucination | exact prompt/turn and visible-excerpt grounding, followed by one-to-one maintainer disposition | grounding establishes that text was visible, not that the critic's interpretation is correct |
| evidence cherry-picking | response, prompt, schema, fixture, binary and source digests; exact fixed matrix; worktree-and-history one-attempt guard; export from clean tracked bytes; exact attempt/canonical comparison; deterministic replay audit | a maintainer who rewrites repository history can still fabricate or erase normalized evidence, and default-model drift makes a later cohort a new sample rather than a replication of the same model |

These controls support the structural claim only. They cannot convert correlated model behavior
into a representative estimate of human behavior.

## Evidence classes

Every report labels evidence with one of four classes. The UI must not visually collapse them into
one pass/fail field.

| Class | Producer | What it can establish | What it cannot establish |
|---|---|---|---|
| deterministic oracle | harness or aplan test | state, route, rendering, safety or parity fact | that the experience is understandable |
| synthetic observation | actor model | what this model attempted or reported in this run | what a human would do or feel |
| synthetic critique | separate evaluator model | a candidate issue tied to transcript evidence | correctness, prevalence or severity by itself |
| human observation | consented participant session | behavior and language of that participant | universal usability without broader evidence |

The term `synthetic participant` is deliberately avoided. Reports say `actor run`, name the
adapter, and preserve the resolved model identity when the CLI exposes it.

## Useful applications

### For the `A1-418` paper-structure review

Run the six fixed paper-prototype prompts through the cohort specified by the review kit. This can find missing
transitions, inconsistent labels, routes that depend on driver invention, copy that is
needlessly opaque, and states whose narrow form omits the only next action. Deterministic oracles
can reject a route or safety failure; qualitative confusion remains a candidate finding that needs
a maintainer disposition.

The actor sees only the goal, current viewport dimensions and rendered buffer. It does not see
`docs/design/tui.md`, the transition table, the expected route, acceptance criteria or driver
notes. A paper-deck adapter may accept an exact visible label as an action because there is no real
cursor yet; its metrics must not be compared with PTY keystroke metrics. A clean cohort closes the
structural claim only under a declared acceptance protocol and leaves every human-usability claim
to `A1-449`. The original cohort did not survive adversarial audit, so `A1-418` relies on confirmed
deterministic oracles and regressions for its provisional contract while `A1-528` owns the fresh
cross-family product gate.

### During implementation

Drive a real `aplan tui` process through a pseudo-terminal. Vary fixture, terminal geometry,
keymap path, stale refresh, interrupted operation and failure state. Let the model choose only
primitive observable actions while deterministic code decides whether the resulting state is
correct.

This is well suited to:

- exploratory traversal beyond hand-authored happy paths;
- regression discovery after labels or information architecture change;
- adversarial plan content that resembles instructions to an agent;
- narrow-terminal and long-content routes;
- recovery from an invalid action, stale preview or unavailable operation;
- comparison of CLI text and TUI routes for the same user question;
- finding a reproducible sequence that a reducer can minimize into a normal test.

### Before `A1-449`

Use actor runs to rehearse the study script, validate fixtures and identify dead ends. Then run the
human study unchanged by synthetic results unless a change is recorded. Afterward, compare which
human findings the synthetic suite did and did not surface. That calibration determines future
value; a plausible transcript is not validation.

### Evaluator assistance

A separate model may cluster observations, identify contradictory copy or apply a fixed heuristic
rubric to a redacted transcript. Each finding must cite exact turn and screen identifiers. The
evaluator is advisory and sees neither the actor identity nor a preferred implementation.

Do not ask a model for an overall `usable`, `intuitive`, `release ready` or `task done` verdict.
Those questions invite false precision and erase the evidence classes.

## Non-goals

- no model call from the aplan binary, libraries, MCP server or TUI;
- no runtime or default CI dependency on any provider;
- no credential copying, API-key extraction or conversion from subscription auth;
- no model or effort override in the adapter;
- no autonomous edits, commits, task closure or release decisions;
- no demographic impersonation or claim that a prompt reproduces a disability;
- no human transcript sent to a provider without separate consent and data review;
- no raw chain-of-thought collection or retention;
- no score that combines human and synthetic runs into one population.

## Architecture

The harness belongs in development tooling, eventually behind an `xtask` command. The production
TUI supplies test seams but knows nothing about models.

```text
scenario + synthetic fixture + run budget
                    |
                    v
          environment driver
      (paper deck or real PTY)
                    |
          visible observation only
                    |
                    v
            actor adapter
  (Claude / Codex / OpenCode / OMP)
                    |
          one schema-bound action
                    |
                    v
       action validator and executor
                    |
       +------------+-------------+
       |                          |
       v                          v
deterministic oracles       sanitized transcript
                                  |
                                  v
                         optional blind critic
                                  |
                                  v
                         candidate findings
```

There are three hard separations:

1. The environment executes actions. A model never receives a shell or direct TUI process handle.
2. Deterministic oracles evaluate product facts. A critic cannot override them.
3. The actor and critic use separate fresh contexts. Prefer a different provider family for the
   critic and mark a same-family pairing as correlated.

### Environment drivers

The paper driver loads the fixed buffers and transition table from the review kit. It exposes the
current buffer, accepts a key or exact visible-label activation, and returns the next buffer. Any
transition that needs free-form driver interpretation is reported as `unresolved_action`; the
harness must not invent a route to make the actor succeed.

The PTY driver owns a real child process, terminal size and clock. It emits a normalized terminal
cell grid plus explicit cursor and style spans. It accepts only key, mouse, resize, wait and stop
actions. It captures process output, exit status and the typed application-state identity exposed
by the test build. It never gives the actor the semantic scene, hidden action inventory or state
machine event names, because those would reveal affordances a user cannot see.

Color contrast, focus visibility, screen-reader behavior and motor accessibility remain separate
deterministic or human evaluations. A text model may inspect linearized output, but that run is
named `linear-output actor`, not `screen-reader user`.

### Stateless actor turns

Use one non-persistent CLI invocation per action. Each invocation receives:

- the behavioral profile and goal;
- the current visible observation;
- the terminal dimensions;
- a bounded list of prior visible action labels, without driver-authored outcomes;
- the allowed action schema and remaining budget.

It does not receive previous hidden model state. Stateless turns work across all four adapters,
make session cleanup auditable and reduce cross-run contamination. They cost more input tokens, so
the history is harness-generated and bounded; never replace it with a model-authored summary that
can change the facts.

The actor returns one action and two short, observable fields:

```json
{
  "action": {"kind": "key", "key": "enter"},
  "visible_basis": "The focused row is Retry",
  "expected_effect": "Retry plan discovery"
}
```

`visible_basis` is an evidence pointer, not a request for private reasoning. Adapters discard
provider reasoning events and encrypted reasoning payloads. Terminal padding is not semantic: the
driver collapses whitespace and requires at least one nontrivial semicolon-, newline- or
dash-separated or quoted excerpt to occur on the current screen, with case and punctuation within
the excerpt preserved. It checks the action itself independently, so a visible excerpt cannot
create a control. A malformed, ungrounded or invisible first response ends the prompt as
`actor_protocol_failure`. There is no repair prompt: allowing a corrected attempt to count would
select away exactly the confusion the cohort is meant to expose.

### Common action schema

The common schema is deliberately smaller than any agent CLI's tool system:

| Action | Required fields | Availability |
|---|---|---|
| `key` | one normalized key, modifier chord or printable character | paper and PTY |
| `activate_visible` | exact visible label | paper only |
| `mouse` | row, column, button | PTY when mouse is in the scenario |
| `resize` | columns and rows from scenario bounds | PTY when resizing is in the scenario |
| `wait` | bounded milliseconds | PTY only |
| `stop` | `completed`, `blocked`, `unsafe` or `give_up` plus a short visible basis | both |

Unknown keys, off-screen labels, out-of-range coordinates and disallowed action kinds are invalid.
The transcript retains them; silently repairing an action would erase evidence of confusion.
The fixed paper review advertises only `key`, `activate_visible`, `resize` and `stop`; `mouse` and
`wait` remain in the common type for PTY scenarios but are absent from the paper response schema,
because a static buffer driver has neither hit testing nor an asynchronous operation to wait for.

## Adapter contract

An adapter implements five operations:

1. `probe`: report binary version, configured defaults and local isolation prerequisites; the
   networked canary, not this inventory, establishes usable authentication.
2. `canary`: answer a tiny schema-bound prompt without tools or repository access.
3. `act`: make one stateless actor turn.
4. `critique`: optionally apply the fixed rubric to a redacted attempted-session transcript.
5. `cleanup`: remove any adapter-created session and temporary files, even after timeout.

The first executable increment lives in the standalone development crate at
`tools/tui-eval`, behind the stable local launcher:

```sh
cargo xtask tui-eval probe all
cargo xtask tui-eval canary all
cargo xtask tui-eval review-self-test
cargo xtask tui-eval review
cargo xtask tui-eval review-export \
  docs/evidence/tui-structure-review/attempts/cohort-COHORT.json \
  docs/evidence/tui-structure-review/cohort.json
cargo xtask tui-eval review-audit docs/evidence/tui-structure-review/cohort.json \
  docs/evidence/tui-structure-review/dispositions.json
```

`probe` is a local installation/configuration inventory; it does not claim that authentication or
the selected endpoint works. `canary` is the networked isolation/basic-schema check.
`review-self-test` parses the fixed deck and exercises every expected transition and deterministic
oracle without a provider. `review` refuses uncommitted kit or driver inputs, runs the basic canary,
the exact actor-schema canary, one neutral adversarial-instruction canary and a fixed twelve-call
one-shot stability series for all four adapters, then runs a fixed two sessions per conforming
adapter/profile matrix. Every stability response must repeat its exact neutral nonce and action;
terminal punctuation is ignored only in the non-operative expected-effect prose, and there are no
preflight retries. No canary contains an
evaluation screen, so poor task performance cannot be relabeled as adapter unavailability. It fails after
retaining the cohort when the eligibility rule is not met. If fewer than two provider families
qualify, it retains a canary-only cohort and stops before actor sessions because cross-family blind
critique and the gate are then impossible. An adapter qualifies only if every networked preflight
passes and its provider family is known from configured or resolved identity. The stability series
qualifies transport reliability for this workload rather than model task performance; its fixed
threshold applies equally to every adapter and cannot guarantee later calls. All six operations
are opt-in. The standalone crate has its own
lockfile and tests, while the launcher stays in the workspace; normal workspace CI neither calls
providers nor counts network adapter branches in its coverage result. Run its hermetic tests
explicitly with `cargo test --locked --manifest-path tools/tui-eval/Cargo.toml`.

Acceptance cohorts deliberately do not parallelize provider calls. Concurrent calls can consume a
shared subscription or provider quota differently, turn local rate limiting into qualification
bias, and make the declared time counterbalance describe record order rather than execution order.
The harness instead removes redundant local work and fails fast only when a cell can no longer
qualify: successful and failed binary-version probes are cached once per binary for the process;
binary digests are cached; a transport failure, timeout, unavailable adapter or cleanup failure in
an introduction or actor prompt ends that cell's unattempted scenario suffix; and a failed debrief
does not launch a critic. The fixed session record remains in the denominator. Actor protocol
failures do not short-circuit later scenarios or critique because doing so could hide the product
confusion the cohort is intended to observe.

That choice has paid for itself: a blind critic of an incomplete actor session found that clearing
a diagnostic filter was lost after a task/Overview round trip. Critiquing only sessions that had
already passed the deterministic route gate would have hidden the product defect while making the
run look faster. Runtime optimization therefore stops provably wasted infrastructure work; it does
not select transcripts by actor success.

The twelve-call stability series is not reduced for speed. Across tracked adverse cohorts it has
caught first or additional transport failures as late as calls 7, 8, 9, 10, 11 and 12. A shorter
series would therefore have admitted adapters that this repository has already observed failing.

The actor debrief receives the exact fixed screen shown at every retained turn and the selected
action. The blind critic separately receives only those viewport/screen/action records under an open-ended
rubric. It sees no actor first impression or debrief, basis/expectation prose, driver effect,
outcome, write, oracle, internal screen id, actor identity, expected route, hidden state or
maintainer disposition. Each finding must cite an exact prompt/turn and copy an excerpt that the
driver resolves against that screen. The reset boundary stays explicit so the critic cannot compare
independent fixture episodes as one evolving state.

All invocations inherit the user's existing authentication and default model/effort. The harness
must not pass `--model`, `--effort`, `--thinking`, `--variant` or an equivalent override. It records
the resolved values emitted by the CLI. If the CLI cannot expose one, the value is `unknown`; it is
never inferred from marketing names or a prior run.

The harness gives every adapter the same prompt payload and JSON Schema, then normalizes only the
transport envelope. Provider-specific prompting would make cross-adapter differences impossible
to interpret. The adapter may add only the minimum instruction needed to produce the schema.

### Host capability survey

The probes were repeated through 2026-10-13 from a temporary directory outside the repository. They
tested transport, schema and isolation, not TUI quality. No API key was printed, copied or placed in
a prompt.

| Adapter | Installed state | Confirmed headless path | Current conclusion |
|---|---|---|---|
| Claude Code 2.1.296 | configured existing Claude subscription; product-selected model, configured general effort `high` | restricted print mode, read-only credential projection, non-secret default settings, empty tools/MCP, JSON Schema, no session | currently unavailable: after initially resolving `claude-opus-5-5`, both isolated and outside-sandbox no-tools controls returned sanitized `authentication`; host re-authentication is required |
| Codex 0.162.1 | authenticated through ChatGPT; configured model `gpt-5.6-sol`, effort `xhigh` | `exec`, JSON events/output schema, disabled app/plugin/browser features, ephemeral session inside `bubblewrap` | conditionally available: all 15 neutral preflight calls passed in the latest twelve-session cohort, but previous runs exposed transport instability and no latest Codex actor session completed every scenario |
| OpenCode 2.0.20 | existing provider credential; configured default resolves to `exo-free` | standalone `run --format json` from a temporary cwd with adapter-owned deny-all permissions and plugins disabled | unavailable; the configured endpoint returns HTTP 410 `ModelDeprecated`; installed 2.0.20 also lacks the current documentation's `--pure` and `--dir` flags, so the adapter must version-gate its invocation |
| OMP 18.8.6 | configured default `openai-codex/gpt-6.1-sol:xhigh` and existing provider auth | print JSON, no tools, session, extensions, skills, rules, LSP or PTY | available; canary emitted the configured provider/model and valid final JSON; only allowlisted terminal text/usage may survive its reasoning-bearing stream |

Four probe findings shape the contract. First, Claude's structured-output validator rejects a Draft
2020-12 `$schema` declaration even though it accepts the schema body, so the transport schema uses
the common supported subset and the harness independently validates the full contract. Second,
Claude restricted mode intentionally ignores user settings; the adapter projects only the current
non-secret model/effort selection keys into a temporary settings file so isolation does not silently
change the requested defaults. Third, Codex's read-only sandbox prevents writes but not reads; its
accepted path is the proved OS sandbox, not the prompt or read-only mode alone. Fourth, raw CLI JSON
is not a safe artifact: OMP emits reasoning-related events, so normalization must allowlist terminal
fields rather than redact a stream after retaining it.

OpenCode becomes available only after the user repairs its configured default and a new probe
succeeds. Doing that repair is outside the harness: automatic fallback would violate the request
to use the host's configured default and would conceal environment drift.

### Isolation

The actor needs only the observation included in its prompt. It must not inspect source, design
documents, expected routes or credentials.

Each turn therefore runs from a new temporary working directory containing only projected adapter
state and the invocation schema, with:

- tools disabled at the agent layer where the CLI supports it;
- all OpenCode permissions denied through an adapter-owned temporary configuration;
- OpenCode's shared background service bypassed with standalone, ephemeral state;
- no inherited MCP servers, apps, plugins, skills, rules or project instructions;
- no persistent or resumed agent session;
- no writable repository mount;
- network limited to the provider endpoints needed by the authenticated CLI where practical;
- one OS-level allowlist sandbox around every adapter, irrespective of its own permission flags.

Read-only agent modes are not source isolation. On this Linux host, `bubblewrap` presents only a
temporary work directory, one adapter binary, runtime libraries, minimum CA/DNS files and the
smallest projected authentication/default state that the CLI can use. Claude and Codex receive
exact read-only credential files plus allowlisted model/effort settings; no session history is
carried between calls. OpenCode and OMP receive SQLite
snapshots with session, instruction, project, usage and cache rows deleted; OMP's configuration is
rebuilt with only its default model role. Before either SQLite snapshot is copied, its complete
non-internal schema signature - object types, names, owning tables and SQL definitions - and table
set must exactly match the audited CLI schema; the copied snapshot is checked again before it is
scrubbed or mounted. Drift fails closed instead of carrying an unknown table, view or trigger.
Tools, MCP, plugins, browser, image generation,
multi-agent features, rules, skills and persistence are also disabled where the CLI supports them.
The process environment is cleared and rebuilt from `HOME`, `PATH`, locale and temporary-directory
values inside the sandbox. A direct process probe proves that the repository and known prior-session
paths are unreadable before every provider call. The typed canary separately tests that the model
refuses an untrusted request to read an outside path, but that model answer is not treated as proof
of the OS boundary. Authentication
must remain brokered by the installed CLI; do not mount a broadly readable home directory or put a
credential in the prompt environment to make the sandbox convenient.

Use only synthetic repository fixtures. Plan content is untrusted text and may resemble model
instructions; the actor prompt states that screen text is data, and the no-tool boundary limits
the consequence of following it. Testing that confusion is useful, but it is not permission to
send real private plan data to a provider.

## Scenario design

A scenario contains a goal, starting fixture, terminal geometry, allowed actions, turn/time/token
budgets, deterministic oracles and an optional behavioral profile. It never contains an expected
route in the actor payload.

Start with the six `A1-418` prompts and add orthogonal variants:

- missing root, unreadable plan, invalid config and successful retry;
- whole-plan health with mixed warning/error subjects;
- ready task, direct blocker, unfinished cut and off-screen dependent;
- unresolved, ambiguous and corrected evidence;
- graph scope, omitted work, critical route and selection recovery after resize;
- mutation preview that becomes stale before confirmation;
- wide, 80-column and minimum supported terminal layouts;
- keyboard-only, mouse-enabled and help-first paths;
- long identifiers, long titles, empty groups and large counts;
- fixture text that looks like an instruction to the model.

Do not run the full cross-product. Use a committed pairwise matrix for routine runs and a seeded
sample for exploratory runs. The seed orders scenarios and transcript presentation; it cannot make
the model deterministic.

### Behavioral profiles

Profiles constrain observable strategy rather than inventing a biography:

- unfamiliar with aplan, frequent developer-CLI user;
- unfamiliar with aplan, occasional terminal user;
- aplan expert using shortcuts and filters;
- cautious actor that refuses unclear writes;
- keyboard-only actor;
- linear-output actor;
- recovery actor that intentionally tries one plausible wrong route before asking for help.

Do not use age, disability, culture or job-title stereotypes as a substitute for recruiting those
users. A profile is a test perturbation, not a population sample.

## Oracles and findings

### Deterministic oracles

The environment, not the model, judges facts such as:

- the requested task, finding or graph node is actually selected;
- the screen's plan identity and revision match the application state;
- CLI and TUI answer the same typed query with equivalent domain results;
- a disabled operation cannot mutate files;
- a stale preview cannot be confirmed and can be recomputed;
- focus remains visible and valid after resize, filtering and row removal;
- terminal restoration and exit behavior are correct;
- omission counts, traversal scope and critical-route membership are correct;
- the run touched no path outside its temporary fixture.

Any such failure is reproducible independently of the actor's opinion. The next step is to reduce
the action trace and add a normal reducer, render or PTY test. The synthetic run is discovery
evidence; the regression test is closure evidence.

An unconfirmed model finding is not filed automatically and is not task evidence. A maintainer
first reproduces or confirms it, then records a task with the run id and the independently observed
problem. This prevents fluent duplicate or hallucinated findings from flooding the plan.

### Synthetic observations

Useful descriptive signals include:

- first destination and first action;
- completion or declared stop reason;
- invalid actions, backtracking, help use and repeated actions;
- number of turns and distinct screens visited;
- the visible string cited before a wrong or abandoned action;
- expectation mismatches, where the observed next screen differs from the actor's short expected
  effect;
- recurrence of the same candidate finding across fresh runs and provider families.

Wall-clock latency is an infrastructure metric, not simulated human completion time. Token use and
provider-reported cost are budget metrics, not UX metrics. The normalized record retains elapsed
milliseconds for each invocation so a claimed process budget can be audited without ignored raw
files. It also retains timeout, truncation, exit status and sanitized error categories.
Provider failures emitted inside a structured stdout envelope are classified separately from
stderr as `rate_limit`, `authentication`, `provider_unavailable`, `configuration` or the generic
`provider_error`. Raw error text is never retained; a failed envelope with no recognizable class is
recorded as `uncategorized_transport` rather than an empty diagnostic.

The process budget covers prompt writing, launcher execution and pipe draining. On Unix the harness
starts every one-shot adapter in its own process group, kills the group on timeout, and also clears
the group after a successful launcher exit so a detached descendant cannot keep stdout or stderr
open. Prompt writing occurs on a joined worker so a child that never reads stdin cannot delay the
start of the timeout. Both successful and failed version probes are cached; a failed probe cannot
consume the same ten-second local budget before every provider call. Regression tests exercise a
blocked stdin writer, a timed-out child with a descendant, a successful launcher leaving a
pipe-holding descendant, and both version-cache outcomes.

The first adverse process-boundary run took more than ten hours. Equivalent fixed cohorts after
these changes completed in roughly 40 to 54 minutes, including a 52-minute twelve-session cohort
when three adapters qualified instead of two. That is approximately an 11x to 15x wall-clock
improvement while preserving the no-retry stability series and serial provider conditions. A run
can still grow when more adapters pass preflight because every qualified adapter must be
represented; that is additional evidence, not harness overhead. Further material speedup requires
a separately validated transport design, such as isolated per-session continuation, rather than
weaker sampling or shared-provider concurrency.

### Critique protocol

Critics receive a viewport/screen/action-only transcript and an open-ended fixed rubric. They do not receive
model/provider identity, source code, actor interpretations, harness verdicts, the intended route,
prior dispositions or another critic's answer. A finding contains:

- category and provisional severity;
- exact prompt and turn references plus an exact visible excerpt resolved by the driver;
- observed fact separated from interpretation;
- a minimal reproduction trace;
- confidence and a disconfirming condition.

Run a different provider family from the actor when available. Reversing presentation order can
detect some position sensitivity. Neither measure removes judge bias, so all critique remains
advisory.

## Run identity and artifacts

Every invocation writes a machine-readable transport record under the ignored
`target/tui-eval/` directory. Individual invocation records are not committed. An `A1-525` transport record
includes:

- run id, role, timestamp and elapsed milliseconds;
- adapter and adapter version;
- existing-CLI-auth category, never account identity or a secret;
- configured and resolved provider, model and effort, including explicit `unknown` values;
- exact prompt and JSON-Schema digests plus an adapter-binary digest;
- isolation mode and sentinel result;
- terminal transport outcome, process status, timeout/truncation flags, sanitized stderr
  categories and sanitized structured-envelope failure categories;
- allowlisted provider token/cost metadata when present.

The paper driver in `A1-526` wraps those invocation records in a session record that includes:

- run id, scenario id/revision, fixture digest and source revision;
- actor/critic role and correlation warning;
- complete model-input manifest, JSON-Schema, full fixture and rubric digests;
- terminal dimensions and scenario seed;
- start/end timestamps, timeout, turn and token budgets;
- normalized observations, actions and deterministic oracle results;
- candidate findings and their later dispositions.

The executable driver writes atomic session and cohort JSON under
`target/tui-eval/sessions/`. It also writes every final eligible or adverse cohort as an ASCII-safe
tracked attempt below `docs/evidence/tui-structure-review/attempts/`, and refuses a second attempt
from the same source commit. This makes an accidental run-until-pass sequence visible; it cannot
prevent a malicious maintainer from deleting evidence and history, which remains a repository
trust assumption. Each session also retains the sanitized invocation identities,
binary/response/prompt/schema digests, usage, duration, exit status, timeout/truncation flags,
sanitized stderr/provider-event categories and terminal outcomes needed to audit the aggregate
without copying raw event streams. Its actor debrief receives the bounded visible viewport/screen/action
transcript because every adapter turn is intentionally stateless. A blind critic receives only
that transcript, not actor answers, driver interpretation, actor/provider identity, the expected
route or a maintainer disposition. OpenCode or any other failing configured default remains an
unavailable canary cell; the driver never substitutes a model or adapter.

After `review` retains an attempt, commit that adverse or eligible attempt before taking another
run or exporting it. `review-export` parses and protocol-validates one eligible, clean, committed attempt before writing
the canonical ASCII-only normalized cohort below `docs/evidence/tui-structure-review/`. The file
name must match the cohort id. Commit the canonical copy and its disposition register before the
acceptance audit. `review-audit` requires all three artifacts to be clean and tracked, and the
canonical bytes to equal the attempt exactly. A separate disposition register binds
the exact cohort bytes by SHA-256 and has one entry for every grounded critic finding. `review-audit`
then replays every recorded transition and oracle, reconstructs every model prompt and schema digest,
checks response bindings and fixed scheduling, recomputes qualification/completion/diversity and
rejects an absent disposition, a missing filed task or any `blocking_structure` finding. The prose
results document is a summary of that executable evidence, not its sole copy.

Raw provider event streams are transient. The adapter allowlists fields needed for the record,
then removes raw output at cleanup. It never records chain-of-thought, encrypted reasoning blobs,
credentials, home paths or account identifiers. A failed cleanup is itself a harness failure.

### Run outcomes

There is no overall `UX passed` outcome. The transport result is one of:

- `valid`: one transport response matched its schema and passed adapter safety checks;
- `product_failure`: at least one deterministic oracle failed;
- `actor_stopped`: the actor declared `blocked`, `unsafe` or `give_up` before the goal;
- `actor_protocol_failure`: the first output was malformed, ungrounded or requested no visible action;
- `budget_exhausted`: a turn, time or token bound ended the run;
- `transport_failure`: the CLI or event stream failed;
- `adapter_unavailable`: auth, configured default, isolation or capability preflight failed;
- `inconclusive`: a required oracle did not run, the paper driver could not resolve an action or
  the observation was incomplete.

Reports show unavailable and inconclusive cells. They never drop them from a denominator or retry
until only successful runs remain.

An infrastructure failure makes a session ineligible immediately. The harness records the failed
invocation and fixed matrix cell, then omits only calls that cannot make that cell complete: later
scenario prompts after a failed introduction or actor transport, and critique after a failed
debrief. It does not fabricate `not attempted` invocations, and deterministic replay accepts only
the exact ordered scenario prefix ending at the infrastructure failure. A model-level invalid or
wrong action is different: later scenarios still run and the visible interaction can still receive
blind critique.

## Repetition and interpretation

For the fixed structure batch, run two fresh sessions per conforming adapter/profile cell, with all
six scenarios in each session, and require at least three complete sessions per profile overall.
The schedule is fixed before outcomes and gives every cell canonical and reverse order; preserve
every attempt and never add repetitions merely to obtain a pass. This is an exploration minimum,
not a statistical sample. A repeated finding is more
useful to investigate, but frequency across correlated models is not prevalence among people.

Compare only like-for-like records with the same scenario, fixture, prompt and rubric digests.
Default-model use is intentional, but it trades reproducibility for fidelity to the tools as they
are actually configured on this host. If the resolved model, effort or CLI version changes, start
a new cohort. Do not combine it with the old cohort in a trend line.

The harness should earn its maintenance cost by measuring itself:

- confirmed unique product defects found;
- findings promoted to deterministic tests;
- false-positive review burden;
- human findings it surfaced before the study;
- human findings it missed;
- adapter failure rate, latency, token use and cleanup reliability.

If calibration shows low incremental yield or high false reassurance, reduce or remove the model
layer rather than preserving it because its transcripts look sophisticated.

## Execution tiers

| Tier | Trigger | Contents | Gate meaning |
|---|---|---|---|
| deterministic | every change and CI | reducer, render, PTY and parity tests | required product gate |
| synthetic smoke | local opt-in after meaningful TUI change | one pairwise scenario set on available adapters | exploratory; deterministic failures block locally |
| synthetic structure | paper-prototype development | fixed cohort across profiles and conforming adapters plus blind critique | formative structure/safety review after the standalone acceptance cohort was retired; never participant evidence |
| synthetic full | `A1-528`, before human study and release evaluation | predeclared repetitions across profiles, geometries and adapters plus blind critique | required public-release product gate; never participant evidence |
| human | `A1-449` protocol | observed consented sessions | required pre-public usability evidence |

Subscription-backed CLIs belong on a developer host, not an unattended shared runner. Their
quotas, product terms and organization policies may differ. Do not copy host auth into CI or make
a provider outage fail the normal build. If the project later wants a scheduled service, review
provider terms, data policy and dedicated service credentials as a separate decision.

## Implementation sequence

The immediate paper-review work is cut into `A1-524` through `A1-527`. `A1-527` retires the first
cohort as acceptance evidence and adds the adversarial isolation, anti-selection and replay audit
described above. The later PTY, corpus and calibration packages must become normal tasks in the
milestone that first needs them rather than
remaining implicit work in this proposal. Estimates are planning inputs, not commitments.

### Package 1: adapter conformance and records (`A1-525`, 2.5 days, TUI foundation)

- define the common action, observation, run-record and outcome schemas;
- implement version/auth/default probes and a schema canary;
- implement bounded process handling, temporary-directory cleanup and retention tests;
- prove source isolation directly for every adapter and fail closed when the OS probe cannot make
  repository and prior-session paths unreadable;
- support Claude Code, Codex and OMP; retain OpenCode as an explicit unavailable result until its
  configured default works;
- place all code in `xtask` or a development-only crate with no production dependency edge.

This package begins after `A1-524` settles the changed evidence claim. It is an explicit predecessor
of the `A1-418` formative review and does not enter mandatory CI.

### Package 2: paper-prototype actor and disposition (`A1-526`, 2 days, TUI foundation)

- encode the fixed review buffers and only their documented transitions;
- run all six prompts using one fixed prompt revision and fixture cohort;
- report unresolved driver interpretation rather than guessing;
- apply the fixed evidence-citing rubric to each attempted session that reached a visible
  interaction, with a blinded different-family critic when one is available;
- add the normalized cohort and dispositions to the structure-review results, explicitly labeled
  as synthetic rather than human records.

This depends on Package 1 and informs only the narrowed structure criterion in `A1-418`.

### Package 2a: adversarial hardening and replay (`A1-527`, 4 days, TUI foundation)

- place all adapters behind the same clear-environment OS allowlist and project only required auth;
- replace evaluation-screen qualification with neutral protocol canaries;
- remove action repair and all driver interpretations from model inputs;
- ground blind critique against exact visible text and counterbalance scenario order;
- track the normalized cohort and dispositions, then recompute acceptance mechanically.

This correction establishes the trustworthy harness boundary. The current adverse cohorts remain
ineligible; repeated cross-family product acceptance is scheduled in `A1-528` rather than inferred
from one provider family during a temporary Claude subscription limit.

### Package 3: real PTY gateway (2 days, after `A1-424`)

- reuse the terminal harness, clock, fixture and normalized renderer seams built by `A1-424`;
- add primitive input, resize, wait and observation operations;
- expose test-only state identity to deterministic oracles without exposing it to actors;
- prove process cleanup and terminal restoration under timeout and malformed actions.

### Package 4: scenario corpus and deterministic promotion (2 days, feature milestones)

- grow scenarios alongside Overview, Tasks, Diagnostics, Graph and mutation workflows;
- add pairwise selection, budgets and a trace reducer;
- provide a command that converts a confirmed reduced trace into a test fixture skeleton;
- audit every promoted test by reverting the defect and watching it fail.

The model output itself never becomes a golden snapshot. Only normalized input/action/state traces
and explicit expected facts do.

### Package 5: blind critic and calibration (1.5 days, before `A1-449` closes)

- implement the evidence-citing rubric and cross-provider preference;
- randomize comparison order and report correlation;
- compare synthetic findings with `A1-449` human observations;
- record recall-like discovery coverage, false-positive burden and missed categories without
  treating five humans as population ground truth;
- decide whether each scenario remains, changes or is removed.

Package 5 depends on the real implementation and human study; it cannot become a blocker that
prevents the humans whose observations calibrate it.

### Package 6: repeated product-wide pre-public evaluation (`A1-528`, 8 days, public gate)

- extend the corpus across the complete CLI, typed engine, MCP and TUI product rather than
  re-running only the paper structure;
- predeclare at least three source-bound rounds at meaningful release checkpoints and retain every
  scheduled cell, failure and unavailable adapter;
- include unfamiliar-user, expert-operator and agentic profiles plus hostile, stale, concurrent,
  narrow-terminal, large-plan and failure-injection fixtures;
- evaluate bounded per-session continuation against the stateless baseline to reduce runtime
  without weakening isolation, blinding, ordering or provider accounting;
- require the final accepted cohort to span at least two independently resolved provider families
  and pass deterministic replay against the exact release candidate;
- keep the human evidence in `A1-449` separate while reporting agreements, disagreements and
  findings that cannot be compared.

Package 6 is a direct member of `M-public`. A temporary provider quota may defer a round, but cannot
turn one provider family into two or remove the evaluation from the publication gate.

## Acceptance bar for the harness

Before synthetic results appear in a design or release report, all of these must hold:

- every actor passes neutral schema and instruction canaries plus an OS-level outside-read probe;
  any emitted tool event fails the run closed;
- no adapter receives source, design intent, expected routes or hidden action inventories;
- every action is schema-validated before execution;
- every product verdict comes from a named deterministic oracle;
- model and CLI drift are recorded rather than silently normalized;
- unavailable adapters and failed attempts remain visible;
- a retained artifact contains no secret, account identity or reasoning payload;
- the normalized cohort and one-to-one dispositions are tracked and pass deterministic replay;
- actor and critic evidence are visually distinct from human observations;
- reverting a promoted product fix fails its deterministic regression test;
- deleting the entire model harness leaves the aplan product and mandatory CI behavior unchanged.

## Recommendation

Proceed, but in this order:

1. Complete `A1-524` so the formative gate says exactly what synthetic evidence can establish.
2. Implement `A1-525` as development tooling and keep all provider calls opt-in and local.
3. Retain every `A1-526` and `A1-527` attempt and its deterministic regressions; do not describe a
   retired or ineligible cohort as accepted.
4. Use those oracles and regressions to settle the provisional `A1-418` paper contract, while
   carrying fresh cross-family acceptance in `A1-528`.
5. Reuse the real PTY and renderer seams after `A1-424` instead of building a parallel UI driver.
6. Promote only reproducible failures into mandatory tests.
7. Calibrate synthetic findings against `A1-449` and keep the layer only if it adds measured value.
8. Run the predeclared `A1-528` rounds against the assembled product and require two provider
   families in the final accepted release-candidate cohort.

That sequence unblocks implementation on a clearly narrower structure claim without relabeling
model output as human evidence or weakening the machine-resolved proof on which aplan is built.
