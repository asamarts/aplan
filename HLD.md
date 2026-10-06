# HLD: `aplan` high-level design

> This is a design document, not a specification. Per
> [ADR-0001 of `alint`](https://github.com/asamarts/alint): *never try to make English
> prose a regenerable source of truth.* The Rust types remain the spec. Where this
> document and the code disagree, **the code is right and this document is stale.**

---

## 1. Purpose, scope, non-goals

`aplan` is a file-based, in-repository project planning format and the validator that
enforces it. Tasks are Markdown files with YAML frontmatter, committed beside the work.
A plan is a set of falsifiable claims, and `aplan` refuses claims that do not satisfy
the configured structural and evidence policy.

In scope are the task format, dependency graph, invariants, evidence verification, CLI,
CI integration, agent interfaces, and interactive terminal interface.

The following remain out of scope:

- resource scheduling, calendars, and Gantt planning;
- comments, attachments, notifications, and assignment workflows;
- personal task-list management;
- bidirectional synchronization with remote trackers; and
- model-based validation. Product checks are deterministic and explainable.

---

## 2. Thesis

> **A status is a claim, not a proof.**
> **An edge is a claim, not a fact.**

A completed task is credible only when its evidence resolves under a declared policy.
A dependency is credible only when both ends exist and the graph remains valid. These
claims must fail loudly on missing references, cycles, stale traces, or evidence that
does not bind to the work it is offered to prove.

The evidence ladder in Section 5 separates structural validation from repository
resolution and explicitly trusted execution. `check` stays hermetic. `verify`
opens the repository and may resolve configured providers. `--execute` is always an
explicit trust decision.

---

## 3. Architectural constraints

- The task schema carries typed evidence because an evidence gate cannot be reliable
  when proof exists only as prose.
- The validator is deterministic, offline by default, and independent of any one
  agent, forge, or hosted service.
- The editable source remains repository-local text. Generated views never become a
  second source of status.
- CLI, MCP, and TUI adapters share typed application operations and the same
  post-state validation gates.
- Forge integration is one-way projection. Remote edits do not silently rewrite the
  authoritative plan.
- The format and schemas remain open, versioned consumer contracts.
- A compact interface is preferred when a broader surface would duplicate policy or
  load unnecessary context.

---

## 4. Domain model and format

A task is one markdown file. The frontmatter is data; the body is prose and is never
parsed. One file per task is a load-bearing choice: task-master's monolithic
`tasks.json` is **merge-hostile** across parallel agents and branches (pre-public product research §6).

```markdown
---
id: P0-003
title: Confirm Delaware ICIS XML filing access
phase: 0
status: in_progress          # todo | in_progress | blocked | done | dropped
owner: kaminsod
effort_days: 3
blocked_by: [P0-001]
paths:                       # NEW (§5.2), what this task is allowed to touch
  - crates/aplan-core/src/**
traces:
  - INITIAL_RESEARCH.md#21-delaware-the-filing-channel
evidence:
  - kind: commit             # commit | url | file | test | note
    ref: d213c9a
  - kind: note
    ref: Division of Corporations confirmed by phone
    captured: 2026-07-09     # required for `note`; an undated note is a rumour
tags: [delaware, blocking]
---

## Why
## Acceptance criteria
- [ ] Written confirmation of the access model
```

**`Status` semantics.** `Done` is the only status that satisfies a dependency.
`Dropped` deliberately does **not**, a dropped blocker means its dependents can never
legitimately start, and pretending otherwise silently unblocks work that has no
foundation. `APLAN020` raises this to a hard error, so a deadlocked plan fails `check`
rather than sitting there looking healthy. No surveyed tool checks this.

`Blocked` is *parked*, not merely unstarted: it has no unmet blockers in the graph, so
`ready` must exclude it by status. A tool that answers "what can I work on now" with a
task whose status is literally `blocked` is contradicting its own plan.

**Milestones are a convention, not a new type.** A cut (a release slice) is an ordinary
task tagged `milestone`, `blocked_by` the tasks in that cut, its body holding the cut's
exit-criteria (its rationale lives in ROADMAP §3; ADR-0009 §4). Membership is therefore a
checked `blocked_by` edge, and a milestone cannot be
`done` while a *listed* member is undone (`APLAN007`, plus resolved evidence), proof of the
listed scope rather than of exhaustive membership (made exhaustive by `APLAN022`, opt-in via
`require_cut_membership`).
The work-views (`ready`, `critical-path`, `summary`, `APLAN009`) skip milestones, because a
checkpoint is not work, keyed on `Task::is_milestone`. `critical-path`'s weighted walk does read
`contains` as an ordering edge, which is not that field becoming a dependency (ADR-0042): on a plan
`check` accepts it is the same edge `APLAN047` already requires in `blocked_by`, and it keeps a plan
part-way through adopting `contains` from showing a chain that contradicts its own rollup
(`A1-345`).
`APLAN021` keeps the marker honest (a
milestone has members and carries no effort or paths). No new field or edge type (ADR-0008);
the sequencing itself stays in ROADMAP.md.

**A task's product is not an edge either.** Work whose deliverable is *more work* -- a spike, a
review round, an evaluation -- says so in its own acceptance criteria, and the follow-ups name the
investigation that produced them in their bodies. There is no `produces:` field. Neither form checks
the claim -- `produces: none` and a ticked box are equally unfalsifiable -- so what is on offer is
two ways of refusing the same silence, and the criterion is the one that already exists and
generalises to every other statement of what done means. `supersede` reached the same answer for the
same edge shape one release earlier (ADR-0020, ADR-0027), and a field that changes no answer is the
half of ADR-0025's bar this one fails. The enforcement is the general one: `close` reading
acceptance criteria at all (`A1-224`).

**A release is a milestone tagged `release`** (ADR-0022, `A1-146`). It is a checkpoint over *cuts*
rather than over work: its `blocked_by` names the cuts it closes, `APLAN007` refuses to call it done
while one is open, and `roadmap` rolls up their work. A task belongs to exactly one cut still, since
a release claims none, and `APLAN021` reports a release that holds work directly. No new field: the
second grouping level stays in tags where ADR-0014 put it.

**Membership stays an edge** (ADR-0030, `A1-228`). Whether a cut's members should be a `cut:` field
on the task instead was a spike, because it reopened an accepted ADR. Three measured symptoms had
been traced to the edge, and all three now have shipped fixes that left the model alone: `A1-216`
derived grouping from the pair, `A1-227` found the quadratic was a traversal defect, and `A1-226`
fixed the write loss with a compare-and-swap and a lock. Both shapes were then measured at 25,000
tasks, on a fixture where both render a comparable graph, and are within 1.6% on every cell at both
cut widths (898ms against 897ms at forty cuts).
The remaining write contention is the sequential id, proved by 24 agents into 24 *different* cuts
refusing 22 of 24 exactly as they do into one, which is `A1-230` and not this. The migration that is
not being paid: 2,457 task files across fifteen plans, a drift-gated schema change, and a public
`cut` field going from derived to declared under the same name.

**And the level above a cut is a repository** (ADR-0029, `A1-200`). One repository holds one plan,
and `blocked_by` names a task in *this* plan and nothing else: no qualified id, no second `root`, no
cross-plan edge. `p00011` was read to decide it (pre-public adoption evidence) because it had built the
alternative by hand -- 148 stream documents in one tree, 23 of them unfinished -- and every field
that distinguished a stream turned out to be a status, a `paths` list or an edge the model already
has, with `owner` and its one policy flag taking a single value across all of them. Its costs came
from splitting a plan rather than from holding one: of ten unresolvable references, five pointed at
*finished* streams that a directory move had put out of view, which cannot happen where a done task
keeps its id and stays. This is what keeps `APLAN003` answerable ("exists" means "is in this plan"),
`APLAN022` countable, and every rollup a single answer.

**New field: `prefer_after`.** A list of task ids this one prefers to follow: **discretionary**
order, which `ready` honours and nothing else reads (ADR-0025, superseding ADR-0017). It never
blocks, so a task whose preference is unmet is still ready and still offered, only offered later,
and `critical-path`, `why` and `APLAN007` do not read it at all -- a reader asking "why can I not
start this" must never be shown an opinion. `APLAN033` refuses a preference that cannot be
honoured: an unknown id, the task itself, a pair that is already a `blocked_by` edge, or a cycle.

**And no second declared signal, for now** (ADR-0031, `A1-231`). `prefer_after` is pairwise, so
"this one matters" against forty-seven others is forty-seven edges; the question was whether an
ordered declared family should rank instead. Measured across the twelve plans on this host, 88% of
207 ready tasks score nothing on the first two derived terms, which is the case for one. Two things
refuse it for now. This repository's own plan carries **0.04** dependency edges per live task, the
lowest of any plan with more than twenty live tasks, where the rest run 0.40 to 2.11 -- so the doubt
was measured on the least-connected plan of its size available. And
`releases` walks one step: counting what sits *transitively* behind a task, over dependency edges
only, discriminates on seven of ten plans -- `p00016` from 23 of 24 ready tasks with no signal to 21
of 24 with one. Adopting an opinion while a derivation is uncounted would trade what the tool can
check for what it cannot, so `A1-298` counts it and the measurement is re-taken after.

**New field: `paths`.** A glob list naming the files this task is responsible for. It
is what turns evidence from a citation into a checkable relation (§5.2). Optional;
absent means relevance-checking is skipped for that task.

**IDs.** Human-readable `<letter><phase>-<nnn>` by convention. This project's own tasks
use `A<phase>` (`A` for aplan, `A1-004`); the illustrative examples above use `P` for a
hypothetical project. dstask's documented footgun is that sequential IDs **collide across
machines**. `aplan add` derives the next id from the convention the plan already uses
(`A1-046` -> `A1-047`, `AL1-07` -> `AL1-08`), and refuses rather than renumbering when
the id it would write already exists; two people generating the same id offline is caught
by `APLAN002` at the merge. `id_strategy: sequential | ulid` **is built** (`A1-047`):
`ulid` removes the collision instead of detecting it, minting 48 bits of timestamp and 80
random bits in place of the counter, and keeping the plan's own prefix because that is the
readable half and nothing about it collides (`A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD`). `sequential`
stays the default, since a 26-character id is a real cost paid by everyone who reads the
plan and a repository with one author has nothing to buy with it. A plan holding ULIDs is
not read as a sequence to continue, so a repository may set the key, unset it, and keep its
own convention either way.

---

## 5. Evidence and the verification ladder

**This is the core of the design.** Evidence is not a string that exists; it is a
claim about the world that can be resolved, and sometimes executed.

### 5.1 Four rungs

| Rung | Name | Question it answers | Needs | Command |
|---|---|---|---|---|
| **L0** | **Asserted** | Is the evidence field present and well-formed? | nothing | `aplan check` |
| **L1** | **Resolved** | Does the referent exist? | a git dir | `aplan verify` |
| **L2** | **Relevant** | Does the referent connect to *this* task's claim? | git + `paths` | `aplan verify` |
| **L3** | **Executed** | Does the named test actually pass, now? | a runner | `aplan verify --execute` |

Per evidence kind:

| Kind | L1 Resolved | L2 Relevant | L3 Executed |
|---|---|---|---|
| `commit` | SHA resolves to a commit in this repo | the commit's tree-diff touches ≥1 path in `paths` | n/a |
| `file` | path exists at `HEAD` (or at the cited rev) | path matches `paths`; file is non-empty | n/a |
| `test` | a test with that name exists in the corpus | its file matches `paths` | the test **passes** |
| `url` | syntactically valid | n/a | reachable (opt-in, cached, off by default) |
| `note` | `captured:` is present and is a valid ISO-8601 date | n/a | n/a |

`note` remains the escape hatch, because most Phase 0 work on the motivating project
is phone calls. But it is now visibly **the weakest rung on a published ladder**,
rather than an undifferentiated peer of a commit SHA. Config can ban it per-tag or
per-phase (`require_evidence`), which is the honest way to let a team tighten the
screw on itself without us pretending we removed the escape hatch.

### 5.2 Why `paths` exists

Without it, L2 is impossible and the whole scheme is cosmetic. A dishonest or careless
agent cites a real commit that has nothing to do with the task. With `paths`, the
question "does the cited commit touch anything this task claims responsibility for?"
becomes a tree-diff, and the answer is a diagnostic.

This is the single mechanism that separates us from a linter that checks a field is
non-empty, **and it is also what separates us from the execution-harnesses.** loki-mode
and prd-taskmaster *run tests*, which aplan does only at opt-in L3. But none of them
checks the **binding** between a task's claim and a specific artifact: that *this* cited
commit touches *this* task's declared files. That claim-to-artifact binding, verified
deterministically and persisted in the plan, is the differentiator, not "we execute and
they don't," which is false. State it that way; the first knowledgeable reader will
notice if we don't.

### 5.3 Command split, and why

`aplan check` stays **hermetic**: no git, no network, no subprocess. It is what runs
on every push, in a pre-commit hook, and inside an editor on every keystroke. Fast,
deterministic, offline.

`aplan verify` opens the repository. It runs in CI on pull requests and on `main`.
`--execute` additionally runs tests, and is opt-in even there.

This split matters because it keeps the fast path fast and the dangerous path explicit,
and because it keeps the *deterministic* path deterministic. L1 (commit resolves) and L2
(commit touches `paths`) are pure functions of the repo. L3 (run a test) and `url`
reachability are not, tests flake, networks flake. That non-determinism is precisely why
they are opt-in and off by default: we headline "deterministic, no LLM," and the rungs
that would undercut that adjective are the ones a team must deliberately switch on.

### 5.4 The `APLAN008` slug algorithm (fixed in `0fef234`)

`APLAN008`, the rule the README singles out as the one people underestimate, once
**did not implement GitHub's anchor algorithm.** The old `validate.rs::slugify` mapped
`.` to `-` and handled only ASCII, so `## 2.1 Delaware: the filing channel` anchored as
`2-1-delaware-...` where GitHub produces `21-delaware-...`, the `traces` example printed
in our own README did not resolve, the exact drift this tool
exists to catch. It is fixed; documented here as the reference algorithm and as a
cautionary tale about a validator whose own docs drifted.

The algorithm now implemented, per the canonical
[`github-slugger`](https://github.com/Flet/github-slugger) source:

1. Lowercase (Unicode-aware).
2. Remove every character that is not a Unicode letter, Unicode digit, `_`, `-`, or a
   space. (So `.`, `:`, `!`, `(`, `)`, and emoji all vanish. `über`, `привет`, `你好`
   survive intact, we do **not** use the `slug` crate, which transliterates.)
3. Replace each remaining space with `-`, *after* stripping, so `"a - b"` → `a---b` and
   runs of spaces are not coalesced.
4. Deduplicate with a stateful per-document counter: `setup`, `setup-1`, `setup-2`.

Heading detection was also tightened (proper ATX: 1-6 `#` plus a required space, 0-3
indent, closing-`#` stripping) and the `title.contains(anchor)` fallback that masked the
bug was removed. Five tests pin it (`readme_trace_example_resolves`,
`slugify_matches_github`, `heading_anchors_disambiguate_repeats`, `atx_heading_detection`,
`duplicate_heading_resolves_by_suffix`). **Remaining follow-up:** a property test against
`comrak`'s `Anchorizer` (GitHub-compatible, actively maintained) for exotic-Unicode
divergence, deferred rather than pull a heavyweight parser into dev-deps; and fenced
code-block tracking (a `#`-line inside a fence is still read as a heading).

### 5.5 Trust boundary, the security-critical part

`aplan verify --execute` runs a test command. A task file can arrive in a pull request
from a fork. **A task file must therefore never be able to name a command.**

Mirroring `alint`'s ADR-0004 (spawning rule kinds are top-level-config-only):

- Evidence of kind `test` carries a **test name**, never a command line. The name is
  substituted into a runner template configured in the **top-level `.aplan.yml`
  only**, and never shell-interpolated. `{}` placeholder, argv vector, no shell.
- `--execute` is **off by default**, and CI must not enable it for `pull_request`
  events from forks.
- `url` evidence is **not fetched by default** (`APLAN016`, `A1-041`). When enabled by
  `url.check`, the fetch is a **configured command** rather than a built-in client, so
  redirect policy, timeout and body-size caps live in that command where the repository
  owns them (`curl --max-time`, and curl does not follow redirects unless asked). The one
  part the command cannot own is **which target it is given**, because the citation comes
  from a task file and a task file can arrive from a fork: `aplan` therefore hands over
  **`https` citations only**, and reports any other scheme as not checked. Measured while
  auditing `A1-041`: before that restriction a task file made the checker request
  `file:///etc/passwd` and `http://169.254.169.254/`, the cloud metadata address.
  A content-addressed cache is not built; the resolver asks once per url per run.
- `aplan check` spawns nothing and opens no sockets, ever. That property is worth a
  test.

### 5.6 Anti-drift, and one idea to hold loosely

Because `traces` anchors are checked and evidence carries a `captured:` date, we can
detect a subtler drift: **a document section changed after the task that owned it was
closed.**

This is **not novel**, and we should say so. `doorstop` has done it since **2014** (v0.8;
the tool itself dates to 2013): a link stores a **fingerprint, a content hash, its
"stamp", of the parent item at the time of review**, and validation flags the link
**suspect** when the parent's stamp no longer matches, cleared explicitly via `doorstop
clear`. That mechanism is better than anything we would invent, it is content-addressed
rather than time-addressed, so it survives rebases, file moves, and reformatting, all of
which would defeat a naive `captured:`-vs-`git log` comparison. One caveat to inherit
honestly: in doorstop a suspect link is a *warning* by default (it gates CI only under
`--error-all`), while a dangling link is a hard error, decide our own severity
deliberately.

**Adopt doorstop's design:** store the anchor's content hash alongside the evidence when
a task closes; `APLAN018` fires when the hash no longer matches. Ship it at **advisory
severity**, behind config, once L1/L2 are solid. Do not let a clever rule discredit a
load-bearing one.

**Landed** (`A1-010`, opt-in via `detect_drift`). The fingerprint is the FNV-1a of the
traced *section* (the anchor's heading through the next same-or-higher heading),
whitespace-normalised so it survives reindentation and reflow, content-addressed so it
survives rebases and file moves. `aplan close` stamps `trace_fingerprints:` per resolvable
anchor; `check` warns when the live section no longer matches; `aplan close --restamp`
re-takes the stamp after a deliberate re-review (doorstop's `clear`). A task with no stamp
is unverifiable, not in violation, so adoption is incremental and false-positive-free.

---

## 6. Rule catalog

Stable codes. Adding one is a breaking change for consumers, so they live in one
module (`diagnostic::code`) rather than being sprinkled through the validator.

**Today (shipped).**

| Code | Rule | Severity | Rung |
|---|---|---|---|
| `APLAN001` | Frontmatter satisfies the derived schema | error | n/a |
| `APLAN002` | Task ids are unique | error | n/a |
| `APLAN003` | `blocked_by` references a task that exists | error | n/a |
| `APLAN004` | The dependency graph is acyclic | error | n/a |
| `APLAN005` | `done` carries at least one piece of evidence | error | L0 |
| `APLAN006` | No task blocks itself | error | n/a |
| `APLAN007` | A `done` task does not depend on an unfinished one | error | n/a |
| `APLAN008` | `traces` anchors resolve to a real heading or node | error | n/a |
| `APLAN009` | Active tasks carry an effort estimate | **warning** | n/a |
| `APLAN010` | A `note` used as evidence carries a date | error | L0 |
| `APLAN020` | A live task is not blocked by a `dropped` one | error | L0 |
| `APLAN021` | A `milestone` has members and carries no effort or paths | **warning** | n/a |
| `APLAN022` | Every task is a member of exactly one cut, or is `cross-cutting` (opt-in, `require_cut_membership`) | error | n/a |

`APLAN020` is numbered out of sequence deliberately: `APLAN011`-`APLAN019` were
already reserved for the ladder below when the deadlock rule shipped, and a rule code
is a contract with consumers, it is not renumbered for tidiness. `APLAN021` (milestone
hygiene, ADR-0008) is the first code past the reserved ladder block; it warns because
the `milestone` tag silently excludes a task from every work-view, so a mis-tag or an
empty cut must be surfaced, not swallowed.

**The ladder, and evidence policy.**

| Code | Rule | Severity | Rung | Status |
|---|---|---|---|---|
| `APLAN011` | `commit` evidence resolves to a commit in this repository | error | L1 | done |
| `APLAN012` | `commit` evidence touches at least one path in `paths` | error | L2 | done |
| `APLAN013` | `test` evidence names a test that exists | error | L1 | done |
| `APLAN014` | `test` evidence names a test that **passes**, and a failure carries a bounded, redacted tail of what the runner printed so it can be diagnosed from the CI record | error | L3 | done |
| `APLAN015` | `file` evidence resolves at the cited rev | error | L1 | done |
| `APLAN016` | `url` evidence is reachable | warning | L1 | exists (`A1-041`, opt-in) |
| `APLAN017` | Evidence meets the configured minimum strength | error | n/a | done (opt-in) |
| `APLAN018` | A closed task's `traces` anchor has changed since closure | warning | n/a | done (opt-in) |
| `APLAN019` | `paths` globs match at least one tracked file, once the task cites a `commit`, `file` or `test`: where nothing is bound to the paths, nothing binds to nothing (`A1-138`) | warning | L1 | exists (`A1-022`) |
| `APLAN023` | A `done` task's `commit`/`file` evidence is path-checkable at all | warning | L0 | done |
| `APLAN024` | A `done` task's commit evidence is about that task, not mostly other work | warning | L2 | done (`A1-062`) |
| `APLAN025` | A cited test belongs to the part of the repository the task's `paths` cover | warning | L2 | done (`A1-063`, `A1-065`, `A1-069`) |
| `APLAN026` | A tag that looks like a member of a declared family is one | warning | L0 | done (`A1-076`) |
| `APLAN027` | A cancelled cut's members are cancelled too | warning | L0 | done (`A1-084`) |
| `APLAN028` | A `done` task stamps every trace it stamps any of | error | n/a | done (`A1-098`) |
| `APLAN029` | A declared tag family is carried by at least one task | warning | L0 | done (`A1-085`) |
| `APLAN030` | A task carries the number of members a declared family requires | error | L0 | done (`A1-133`) |
| `APLAN031` | A cited `file` is inside the task's `paths` | error | L2 | done (`A1-045`) |
| `APLAN032` | Every `paths` entry is a glob this tool can compile | error | L0 | done (`A1-066`) |
| `APLAN033` | A `prefer_after` entry names a task that can actually be preferred | error | L0 | done (`A1-150`) |
| `APLAN034` | A `traces` target outside this repository is declared as external | error | L0 | done (`A1-143`) |
| `APLAN035` | Commit evidence names a commit that did work, not a merge | warning | L1 | done (`A1-145`) |
| `APLAN036` | A configured test runner lists at least one test, or says it expects none | warning | L1 | done (`A1-064`) |
| `APLAN037` | A test that passed under `--execute` is distinguishable from one the runner never ran | warning | L3 | done (`A1-167`) |
| `APLAN038` | A `done` task has answered the acceptance criteria its own body states | warning | L0 | done (`A1-224`) |
| `APLAN039` | A `done` task has not ticked a prompt this tool wrote as though it were a criterion | warning | L0 | done (`A1-271`) |
| `APLAN040` | A task declares each `paths` entry once | warning | L0 | done (`A1-267`) |
| `APLAN041` | The generated `AGENTS.md` block says what this repository's policy says | warning | L0 | done (`A1-273`) |
| `APLAN042` | A `revisit_at` names a real instant, on a task that is parked | warning | L0 | done (`A1-264`) |
| `APLAN043` | A live task does not carry the scaffold's own prompt beside the criteria its author wrote | warning | L0 | done (`A1-299`) |
| `APLAN045` | A live task's traced section is unchanged since the task was last touched | warning | L1 | done (`A1-301`) |
| `APLAN046` | A done task's ratifiers are done | error | L0 | done (`A1-306`) |
| `APLAN047` | A `contains` names a milestone that exists, and closes no loop | error | L0 | done (`A1-333`) |
| `APLAN048` | The released version has moved as far as the closed tasks require | error | L2 | done (`A1-313`) |
| `APLAN049` | The artifact agrees with what the plan claims about compatibility | error | L3 | done (`A1-314`) |
| `APLAN050` | A recorded `version` is on a release, and is a version this tool can read | error | L0 | done (`A1-339`) |
| `APLAN051` | A tag is not one edit from a word that changes what this tool does | warning | L0 | done (`A1-212`) |
| `APLAN052` | `cross-cutting` is not on a task that is a member of a cut | warning | L0 | done (`A1-212`) |
| `APLAN053` | A task that is `in_progress` has something in the repository to show for it: its declared `paths` match at least one tracked file | warning | L1 | done (`A1-359`) |
| `APLAN044` | A closed task claims less of the released version than its level implies, without saying why | warning | L0 | done (`A1-312`) |

Every diagnostic carries a stable code, a severity, an optional task id, an optional
file, a message, and an `agent_instruction`. A **byte span** into the source YAML, so a
renderer can underline `/status` rather than name it, is *planned* (§7), not yet on
`Diagnostic`.

---

## 7. Crate architecture

Uses `aplan-<role>`, with the bare `aplan` package providing the library and binary so
`cargo install aplan` works. `aplan-core` is the runtime dependency sink: it has zero
intra-workspace dependencies, enforced by a test. Runtime support crates depend on it;
the `aplan` package composes those crates into the public CLI, and `xtask` plus
`aplan-e2e` are workspace-only development packages.

```text
                  aplan (library + CLI)
                           |
       +---------+---------+---------+---------+
       |         |         |         |         |
    config    output    evidence  interop     mcp
       |         |         |         |         |
       +---------+---------+---------+---------+
                           |
                       aplan-core

    aplan-attest -> aplan-core
    xtask and aplan-e2e -> the runtime crates they exercise
```

| Crate | Responsibility | Publish | Status |
|---|---|---|---|
| `aplan-core` | task model, loader, graph, validate, diagnostic | yes | **exists** |
| `aplan-output` | formatters: human, json, agent, github, sarif, markdown, mermaid, dot, text | yes | **exists** |
| `aplan` | library + thin binary; clap CLI; `facts()` introspection | yes | **exists** |
| `xtask` | `gen-schema` (exists); `gen-facts` shipped as `aplan facts` + `tests/facts.rs`; `gen-roadmap` (exists, `A1-016`) | no | **exists** |
| `aplan-evidence` | the verification ladder; commit, file, test and URL resolvers | yes | **exists** |
| `aplan-config` | `.aplan.yml` loading and schema-derived `Config` | yes | **exists** |
| `aplan-interop` | import/export adapters and bulk task manifests | yes | **exists** |
| `aplan-lsp` | LSP server behind `aplan lsp` | yes (internal) | **planned** |
| `aplan-mcp` | MCP server behind `aplan mcp` | yes | **exists** |
| `aplan-forge` | one-way mirror to GitHub / GitLab | yes (internal) | **planned** |
| `aplan-attest` | tamper-evident audit export (`aplan attest`) | yes | **exists** |
| `aplan-tui` | btop-style terminal UI | yes (internal) | **planned** |
| `aplan-testkit` | scenario DSL, tree-spec materializer, proptest strategies | no | **planned** (materializer currently folded into `aplan-e2e`) |
| `aplan-e2e` | scenario corpus + coverage-drift audits | no | **exists** (rule + format audits) |
| `aplan-bench` | criterion benches + fixture generator | no | **planned** |

Throughout this document, tables mark a feature **exists** or **planned**; *which cut*
delivers each planned item is sequenced only in [ROADMAP §10](ROADMAP.md), HLD carries
no phase numbers, so the two cannot drift.

**Error handling:** library crates use typed errors, while executable orchestration in
the `aplan` binary and `xtask` may use `anyhow` for context. Exit-code routing distinguishes
usage/configuration errors from internal failures.

**On `miette`.** `alint` declares it and never uses it, its human renderer is
hand-rolled ANSI over `anstyle`. We have a stronger case than `alint` does, because we
validate YAML frontmatter and can point a caret at a byte range. But `miette` 7.6.0 has
not cut a release since April 2025 (repo active, just slow). **Decision: hand-roll the
human renderer on `anstyle` now, but record byte spans in `Diagnostic` from day one**
so adopting `miette` later is a renderer change, not a data-model change.

---

## 8. CLI surface

Exit codes are a contract. `aplan` adopts `alint`'s four-code contract (ADR-0007,
`A1-012`), which distinguishes a config error from an internal bug so a consumer's CI
can tell *your* fault from *aplan's*:

| Code | Meaning |
|---|---|
| `0` | clean (warnings allowed) |
| `1` | at least one error |
| `2` | usage or configuration error (the invocation, the input, or `.aplan.yml`) |
| `3` | internal error (a bug in `aplan`) |

Most CLI errors are the user's, so `2` is the default; only aplan's own invariant
failures (a `close` that fails its self-check) exit `3`. A hard panic remains `101`, the
Rust convention for an unhandled crash. `facts.json` carries these codes so tooling reads
them from one place, and the version is the single source of truth (HLD §17).

| Command | Purpose | Status |
|---|---|---|
| `aplan check` | validate; hermetic; fails on any error | exists |
| `aplan verify` | resolve evidence (L1/L2); `--execute` for L3 | exists (`A1-004`) |
| `aplan ready` | tasks whose blockers are all `done`, ordered by a derived rank: what a task releases, then the critical path, then effort, then id. `--explain` says which (`A1-125`) | exists |
| `aplan why` | `<id>`: what blocks a task -- its own blockers and any unfinished cut, reported as itself with a count of what is behind it; `--all` walks into a cut's membership. An answer that folded anything says so, and the JSON always names its `traversal` | exists (`A1-263`) |
| `aplan version` | `<id>`: what that node owes the released version -- its own claim, or its level's floor, or the strongest of what its members owe. Answered from the plan alone, so it is `check`'s rung; `--format json` names the member that raised it. Without an id it needs the release window and refuses until that exists | partial (`A1-313`) |
| `aplan graph` | Mermaid, whole plan or scoped by `--cut` and `--tag`, which intersect. Edges leaving the view are noted rather than drawn (`--format dot` for Graphviz, planned) | exists (`A1-126`) |
| `aplan show` | `<id>`: one task whole: its `paths`, `traces` and `evidence`, plus what the graph and the ladder resolve about it (cut, ready, both edge directions, and the findings attached to that id). `--format json` carries the same content, `--body` includes the prose, `--no-resolve` skips the ladder and says so | exists (`A1-118`) |
| `aplan tree` | `<id>`: one task's neighbourhood in both directions, `--depth N`: what must precede it, and what it releases. The inspection a *derived* order needs (ADR-0017), so `ready`'s rank is checkable rather than asserted | exists (`A1-126`) |
| `aplan schema` | the derived JSON Schema | exists |
| `aplan facts` | the tool's generated facts (rules, formats, commands, kinds) | exists (`A1-011`) |
| `aplan paths` | what share of recent commits each task's `paths` would match, against the plan's own median; advisory, never blocking | exists (`A1-151`) |
| `aplan due` | `--as-of <instant>` (RFC 3339): parked tasks whose `revisit_at` has arrived, longest overdue first. Due for a look, never unblocked; the only command that reads a clock | exists (`A1-264`) |
| `aplan stale` | live tasks unchanged in git for longer than a window, readiest first; parked work included, since a park is a decision with a half-life; advisory, never blocking | exists (`A1-020`, `A1-275`) |
| `aplan vacuity` | per rule, the tasks it had a subject in and the tasks whose predicate had nothing to bind against -- the ones it ran on and could not have failed. Hermetic, advisory | exists (`A1-114`) |
| `aplan deprecations` | the runways a plan declares -- announced in one cut, removed in a later one -- and the ways that pair breaks. Hermetic, advisory, `--format markdown` for the table a project publishes | exists (`A1-021`) |
| `aplan attest` | the verified plan as a commit-stamped document, optionally attached as a git note | exists (`A1-040`) |
| `aplan check-docs <files>` | every task id a document names resolves to a task | exists (`A1-148`) |
| `aplan export-roadmap-json <file>` | the plan's statuses and edges written back into a `roadmap.json` a foreign build reads, merged into the file so the fields this model does not hold are carried through. `--check` gates it | exists (`A1-033`) |
| `aplan export-agents-md` | the agent contract the plan and `.aplan.yml` imply, fenced and gate-able | exists (`A1-038`) |
| `aplan export` | `--format <shape>`: the plan written back out in somebody else's shape, the other half of `import`. This build writes `checklist`, where a cut is a `##` heading and a `done` task is a ticked box; the other shapes `import` reads are refused by name with the one field a generated document would have had to invent. What the shape could not carry goes to standard error, so a redirected document is only the document. `--out` writes, `--check` gates | exists (`A1-189`) |
| `aplan summary` | tasks + effort per phase, ready count | exists (`A1-002`) |
| `aplan tasks` | the plan as data, so any other rollup is a `jq` line | exists (`A1-075`) |
| `aplan drop` | drop a task, or a whole cut, recording the cancellation | exists (`A1-077`) |
| `aplan mv` | `<id>...`: move tasks between cuts, editing both milestones or neither. A set is one move: every refusal runs before any file is written, so a member that cannot move refuses the whole set | exists (`A1-077`, `A1-127`) |
| `aplan claim` | `<id> [--as NAME] [--from HOLDER]`: record who holds a task, through the compare-and-swap writer, so two actors cannot both be told they hold it. Taking a held task needs `--from` naming the holder, which is checked | exists (`A1-359`, ADR-0044) |
| `aplan release` | `<id>`: drop a claim, naming whose it was. Any caller may run it, because a claim with no expiry needs a way out that does not depend on the holder being reachable | exists (`A1-359`) |
| `aplan start` | `<id>`: record that work has begun, setting `in_progress`. A stored claim, checked by `APLAN053` rather than trusted, and refused on a terminal task because nothing reopens finished work | exists (`A1-359`, ADR-0044) |
| `aplan stop` | `<id>`: record that work has stopped, returning the task to `todo`. Not a closure: it claims nothing about what was done | exists (`A1-359`, ADR-0044) |
| `aplan critical-path` | `[<ID>]`: longest path over `effort_days`, **advisory, never a gate**. With an id, the longest chain that *ends at* that task, cut, container or release: what must finish before that checkpoint can. The whole graph is kept and only the endpoint is chosen, which is why this is sound where the refused `--cut` is not -- filtering first turns a fragment into a false path | exists (`A1-013`, `A1-345`) |
| `aplan init` | scaffold `.aplan.yml`; refuses to overwrite | exists (`A1-009`) |
| `aplan close` | `<id>`: set `done` + attach evidence; derives `HEAD`; resolves the evidence and refuses what `verify` would reject (`--no-resolve` opts out); never commits | exists (`A1-008`, `A1-109`) |
| `aplan amend` | `<id> --replace-evidence <from> <to>`: repair a closed task's evidence in place: replace one citation, both named, resolved exactly as `close` resolves one; leaves `trace_fingerprints` alone | exists (`A1-110`) |
| `aplan dep` | `add <id> --after <other>`, and `dep rm`: add or remove one `blocked_by` edge, the only per-task ordering the model has. Refuses through the shared post-state check: a cycle, an unknown id, a cancelled blocker, a `done` task acquiring an unfinished one. A milestone's `blocked_by` is its membership rather than its ordering, so that edit belongs to `mv` and is refused here | exists (`A1-124`) |
| `aplan tag` | `add <id> <tag>`, `rm`, and `rename <from> <to>` across the plan: the tag edits, which were creation-only until a family's cardinality made `APLAN030` an error and brought them inside `A1-077`'s test. Judged by the shared post-state check before writing, and written through the locked compare-and-swap path, so a rename is all files or none. The five words that decide what a plan *is* are refused on either side of a `rename`, because that edit reaches every task at once and `tag rename milestone epic` dissolves every cut in the plan, and `tag rm` refuses to take `milestone` off a cut that still holds work, which is the same loss one task at a time and quieter; `add`, `mv` and `drop` change what a task is | exists (`A1-213`) |
| `aplan supersede` | `<id> <title>`: file the successor to a closed task whose traced outcome no longer holds, and re-stamp the closed one as reviewed. The predecessor keeps its status and its evidence: there is no reopen (ADR-0020). The successor inherits the predecessor's `paths`, `traces` and subject tags -- not the five reserved words, which say what a task *is* rather than what it is about, `cross-cutting` among them because a waiver is not inherited -- and `--tag` says them outright instead; `--cut` is part of the plan the mutation gate judges, so a placement it makes is not then reported as a task in no cut | exists (`A1-130`, `A1-251`) |
| `aplan add` | scaffold a new task file: next id in the plan's own convention, `--cut` placement, no invented fields | exists (`A1-029`) |
| `aplan import` | `<source>`: read another tool's plan; `--format kep` (`A1-019`), `checklist` (`A1-030`) and `roadmap-json` (`A1-031`) exist, Backlog.md / task-master / spec-kit / beads planned. `--cut` places what it writes and `--tag` gives it the vocabulary the plan requires, because a source document supplies neither; the run names any declared `tag_families` member the import would leave missing, as it already names cut membership and effort | partial (`A1-019`, `A1-030`, `A1-031`, `A1-357`) |
| `aplan mirror` | one-way push to a forge | planned |
| `aplan lsp` | editor server | planned |
| `aplan mcp` | agent server: `identity`, `ready`, `why`, `check`, `verify`, `add`, `show`, `dep`, `close`, `claim`, `release`, `start`, `stop` over stdio | exists (`A1-024`, `A1-102`, `A1-103`, `A1-124`, `A1-118`, `A1-359`) |
| `aplan tui` | the terminal UI | planned |
| `aplan roadmap` | render the plan grouped by cut: rollups, per-cut critical path, `--format markdown` companion (ADR-0009) | exists (`A1-015`) |
| `aplan migrate` | schema version upgrades | when `v2` exists |

**Flag conventions**, from `alint`: `--format` is a bare `String` on the clap struct
plus a hand-written `FromStr` on `aplan_output::Format`, specifically so `aplan-output`
carries **no clap dependency** and stays usable as a library. Global flags use
`global = true`. Cross-subcommand constraints that clap cannot express declaratively
are enforced by hand with a clear error, not silently ignored.

`aplan close P0-003 --evidence commit:d213c9a` is *easier* than editing YAML by hand,
and `aplan close P0-003` with no flag is easier still: it derives the current `HEAD`.
Evidence is friction; friction is the top adoption risk (pre-public product research §11). Where we can
derive evidence for free, the commit currently being made, the test that just passed, we
do (`A1-008`; deriving the just-passed test awaits `aplan close --execute`). The write
lands in the working tree, never an auto-commit: surprise writes are the top-reacted
git-native pain (pre-public product research §15).

---

## 9. Configuration

`.aplan.yml` at the repository root, **shipped** in `A1-001` (the `aplan-config` crate).
It is discovered by walking up from the start path, so `check` and `verify`, given any
path inside the repo, converge on the same top-level file. It is optional: with none, the
built-in defaults apply.

The `Config` type is `schemars`-derived into `schemas/v1/config.json`, gated by
`cargo xtask gen-schema --check`, exactly as `Task` is. Validation is serde with
`#[serde(deny_unknown_fields)]`, so an unknown key is a reported error, never ignored
(as `alint` also validates its config with **serde, not the `jsonschema` crate**; the
committed schema exists for editor autocomplete and the tests). A config the tool ignores
would be drift on day one, the failure this tool exists to catch.

The shipped shape, every key optional:

```yaml
require_effort: true                            # APLAN009, default true
traces_root: .                                  # APLAN008 root, default the cwd
no_traces: false                                # or switch trace checking off
test:                                           # the runner for `verify --execute`
  list: ["cargo", "test", "--", "--list"]       # prints test names, one per line
  run:  ["cargo", "test", "--exact", "{test}"]  # `{test}` is a whole argument, no shell
evidence:                                       # APLAN017, off unless set
  require_min_strength: [commit, test, file]    # a `done` task needs one of these...
  exempt_tags: [phone]                          # ...unless tagged, where a note is best
detect_drift: true                              # APLAN018, stamp + watch traced sections
effort_days_per_week: 7.5                       # ADR-0023, derive durations; no rule, no dates
```

**The trust boundary lives here** (§5.5). The `test` runner and `url.check` are the only
places a command may be named, and both are top-level config only, never a task file.
`{test}` and `{url}` are each substituted as a whole argv element and spawned with no
shell, so a task file (which can arrive in a fork PR) can never inject a command; a
template that would splice the placeholder into a larger argument is rejected at load.
What a task file *can* choose is a `url` citation's target, which is why only `https`
citations are handed to the checker (§5.5).

**Which config is trusted is the caller's choice, and `aplan` cannot make it** (`A1-155`).
Discovery walks up from the plan root or the repository, so pointing `--root` and `--repo` at
somebody else's checkout selects *their* `test` and `url` programs -- correct for your own
repository, wrong for a review. `--config <file>` reads exactly that file and discovers none;
`--no-config` reads none at all, so no configured command can run and `test`/`url` citations stay
unchecked rather than passing. A named file that is missing is an error, never a fallback to the
tree under review. Relative values inside a selected file resolve against *its* directory, while
the plan's own location remains where git history is read.

Choosing and authenticating the trusted file stays the caller's responsibility: `aplan` reads the
path it is given. A reviewer that needs more than "this path is mine" -- a signature, a digest, a
file only root can write -- has to establish that outside the tool, exactly as it establishes that
the binary it runs is the one it pinned.

**Precedence:** a CLI flag overrides the file, which overrides the default
(`if cli.flag { .. } else { config.field.unwrap_or(default) }`). A flag misused on the
wrong subcommand **errors loudly** rather than being ignored.

**Planned keys**, each shipping with the rule or command that reads it, never before, so
the tool never carries a key it ignores: a `url` reachability switch (`APLAN016`, network
off by default), a `root` key so `--root` need not be repeated, and a `version` field.

**No `extends:` in v1.** `alint` needs remote ruleset composition; a plan validator does
not, and the SRI/SSRF hardening that comes with it is cost we should not pay yet.

---

## 10. Output

One report, many consumers. `alint` ships eight formatters; we should converge.

| Format | Consumer | Status |
|---|---|---|
| `human` | a person reading a failure, coloured, file paths, later carets | exists |
| `json` | deterministic automation; `aplan/check-report/v1` | exists |
| `agent` | every diagnostic carries `agent_instruction`; `aplan/agent-report/v1` | exists |
| `github` | workflow-command annotations, inline on the PR | exists |
| `sarif` | GitHub code scanning, and the lingua franca of static analysis | exists (`A1-023`) |
| `gitlab` | Code Quality report | planned |
| `junit` | generic CI test-report ingestion | planned |
| `markdown` | a PR comment / job summary | exists |
| `mermaid` | generated Mermaid graph views, never hand-edited | exists |
| `dot` | generated Graphviz graph views, never hand-edited | exists |
| `text` | stable plain-text command output | exists |

Colour is a property of the *stream*, not of a formatter: every human renderer writes through
`anstream`, which styles a terminal and strips into a pipe, and `--color` writes the
process-global choice `anstream` consults rather than being threaded to each call site
(`A1-286`). That is what lets one flag reach the renderers, the CLI's own refusals and `clap`'s
usage errors alike. The machine formats gain nothing from it at any setting, which is asserted
rather than assumed.

Two ideas worth stealing from `alint`'s `agent` format: an **`argv` field** giving the
exact command an agent can run to fix one diagnostic (regression-tested against the CLI
parser, so it can never name a flag the binary does not accept), and a **strict
stdout/stderr split** so progress noise never contaminates the parseable stream.

**SARIF earns its Phase-1 slot.** It is how a validator becomes visible in GitHub's
security tab and in every static-analysis dashboard on earth, and no competitor emits
it.

---

## 11. Interfaces

### CI
An `action.yml` composite action, the one-line adoption path (`A1-003`). It installs
a pinned release and runs `aplan check --format github`. `verify` runs as a separate,
opt-in step, because it needs `fetch-depth: 0` to see history, and must not enable
`--execute` for fork pull requests (§5.5).

Also: a `.pre-commit-hooks.yaml`, since `check` is hermetic and fast enough to run on
every commit.

### Agents
Two surfaces, and both matter.

1. **`--format agent`.** Structured stdout an agent's shell loop can consume. This is
   `alint`'s entire agent story, and it is a good one: no protocol, no server.
2. **An MCP server** (`aplan mcp`). Unlike `alint`, we should ship one. beads,
   Backlog.md, task-master, mdplanner and saga-mcp all expose MCP; in this category it
   is table stakes, not differentiation. The official Rust SDK,
   [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) **2.2.0 (2026-07-08)**, is
   maintained under the Model Context Protocol org with a weekly cadence and is
   production-usable.

   Expose a **deliberately small** tool surface, `ready`, `why`, `check`, `verify`, `add`, `show`, `dep`, `close`.
   task-master's ~36 MCP tools load ~21k tokens by default and it had to ship an env var
   to trim them (pre-public product research §6). Context bloat is a measured, published cost: an ETH
   Zurich study over 138 repos found context files raise inference cost >20% and can
   *lower* success. **A small tool surface is a feature.**

   `close` must go through the same evidence gate as everything else. An MCP `close`
   that skips verification would reproduce `mcp-shrimp-task-manager`'s `verify_task`,
   which accepts a `score: 0-100` **supplied by the calling agent** and auto-completes
   at ≥80. That is the anti-pattern this project exists to refuse.

### Editors
`aplan lsp` over stdio, and every editor integration is a thin config that spawns it,
`alint` proves this scales to eight editors without reimplementing diagnostics anywhere.

**Important correction for the stack:** `alint` uses `tower-lsp` 0.20, which is **dead**
frozen since 2023-08-11. The maintained successor is
[`tower-lsp-server`](https://github.com/tower-lsp-community/tower-lsp-server) **0.23.0
(2025-12-07)**, same tower/async API. (The alternative, rust-analyzer's `lsp-server`
0.8.0, is healthy and rust-lang-backed but is a low-level synchronous scaffold: you own
the dispatch loop.) **Use `tower-lsp-server`. `alint` should migrate too.**

LSP features, in value order: diagnostics on `did_open`/`did_change`/`did_save`;
completion of task ids inside `blocked_by`; go-to-definition from a `blocked_by` entry
to that task's file; hover showing a blocker's status and title; go-to-definition from a
`traces` anchor into the document heading. That last one makes the anchor a real link,
which is the whole point of `traces`.

### TUI
ROADMAP v0.8. btop/htop **feel**, not features: dense panels, box drawing, keybind hints,
mouse support, theming.

- **`ratatui` 0.30.2** (2026-06-19) is canonical and unchallenged; **`crossterm` 0.29**
  is the backend everyone uses. Skip `tui-realm`, no mature app uses it.
- **Architecture: immediate-mode**, which is ratatui's native model. Copy
  [`bottom`](https://github.com/ClementTsang/bottom), it is the closest btop analogue
  and has the cleanest skeleton: a unified event enum over an mpsc channel, a background
  data-collector thread, one central `App` mutated in a flat `match`, a dedicated canvas
  draw layer. Steal [`gitui`](https://github.com/gitui-org/gitui)'s `commands()`-driven
  contextual keybind hint bar.
- **Testing is a solved problem**: `TestBackend` renders to an in-memory `Buffer`, and
  ratatui publishes an official `insta` snapshot recipe. There is no excuse for an
  untested TUI.
- **DAG rendering is the one genuinely weak spot in the ecosystem.** There is no mature
  drop-in. `petgraph` gives the graph and `toposort` but no layout; `rust-sugiyama` emits
  coordinates only; `layout-rs` is SVG-only. **Plan to hand-roll**: `toposort` → layer
  assignment → box-drawing/braille renderer, which is what `git-graph`, `serie`, and
  `git log --graph` each did with bespoke code.

  *Re-measured and settled (`A1-218`).* The survey above was taken in June and its figures
  were stale by August: measured on crates.io on 2026-08-31, `ascii-dag` is `0.10.3`, not
  `0.9.1`, with 72,419 downloads of which 65,188 are recent: a crate on a steep curve
  rather than a quiet one, though still one maintainer.

  The conclusion survives, on a closer call than "hand-roll" suggests. Rendered over this
  plan's own dependency graph (83 nodes, 67 edges) and given the titles a reader needs,
  `ascii-dag` draws 331 rows by 97 columns against the shipped renderer's 159 by 95. The
  crate fits a terminal; it costs twice the rows and does not bound its width, because in a
  layered picture the label length feeds the widest layer and `max_width` is computed
  rather than configured. The shipped renderer lays the graph out in *lanes*, so width
  follows edges in flight and height follows the plan, which a terminal already scrolls.
  `A1-218` carries the full table and the prototype that produced it.

  The claim that **our DAG is mostly linear with fan-out was wrong** and is the reason the
  layered reading looked tractable. It is wide and shallow: before `A1-216` separated
  grouping from constraint it was nine layers with a widest layer of 177, and three
  quarters of its edges were a cut listing its members. Only after that separation is it
  the shape this paragraph assumed.

---

## 12. Integrations

**git.** Read-only, and only in `verify`. Needs: does a SHA resolve; read a file at a
rev; did a commit touch a path (a tree-diff filtered by pathspec). Both candidates do
all three. **`gix` 0.85.0** is pure Rust, needs no C toolchain, and is one of the most
active crates in the ecosystem, but is pre-1.0 with breaking minors, so pin exactly.
**`git2` 0.21.0** is mature and its `diff_tree_to_tree` + `DiffOptions::pathspec` is the
better-documented path for exactly the operation `APLAN012` needs.

*Recommendation:* start with **`git2`** for `APLAN011`/`APLAN012`, because path-diff is
the one operation where libgit2's maturity is a real advantage and correctness here is
load-bearing. Revisit `gix` once the ladder is proven; the C-toolchain-free build is
worth real money for distribution.

**Forges.** One-way mirror only (pre-public product research §8). A CI job renders the *already
validated* plan into GitHub Issues, sub-issues, and native dependencies for stakeholder
visibility, each marked read-only: *"managed by `aplan`; edit the plan file."*

We validate the DAG **before** we mirror, which is quietly delicious, because GitHub's
own dependencies (GA 2025-08-21, confirmed) have **no documented cycle detection**, are
capped at 50 per relation, and are advisory (nothing in the docs prevents closing a
blocked issue). *`aplan` can give GitHub Issues a property GitHub does not give itself.*
That is a launch blog post. (Do not claim "same-repo only", the `gh` CLI accepts
cross-repo blocked-by URLs, so that earlier claim is retracted.)

Never sync back. If tracker-side edits must be accepted, they arrive as a **proposed PR
a human reviews**.

---

## 13. Interop

Being the checker that runs over *other people's* artifacts is our distribution
strategy, and yes, we both **compete with** Backlog.md and beads (§3.9) and **import
from** them here. That is deliberate embrace-and-extend: import is how we convert a
competitor's users, not a contradiction. `aplan import` should read, at minimum:

| Source | Shape | Notes |
|---|---|---|
| **Backlog.md** | md + YAML frontmatter, `dependencies[]` | Nearest neighbour. Near-lossless. |
| **task-master** | `tasks.json` | Emphasize that we split it into one file per task, curing its documented merge conflicts. |
| **spec-kit** | `specs/NNN/tasks.md` | Ordering is prose; import is lossy and must say so. |
| **beads** | `issues.jsonl` export | Dolt is the source of truth there; we read the export. |

Import must be **honest about loss**. A lossy import that silently drops a field is
precisely the drift this tool exists to detect. Emit a diagnostic per dropped field.

---

## 14. Determinism, performance, scale

**Determinism is a correctness property**, not a nicety: byte-identical output across
runs, no `HashMap` iteration order leaking into diagnostics. `aplan-core` already uses
`BTreeMap` throughout and sorts diagnostics by a *total* `(code, task, file, message)`
key (the determinism fix, `(code, task)` alone let ties reorder with walk order). Test it.

**Cycle detection.** Keep the iterative DFS so deep cyclic input cannot exhaust the call stack. Add a `proptest` that throws deep and pathological graphs at it.

**Adopt `petgraph` 0.8.3** anyway. Not for cycle *detection*, we have that, but for
cycle **diagnostics**: `tarjan_scc` returns the actual strongly-connected components, so
we can report *which* tasks form the cycle rather than one path through it. We also get
a tested `toposort` for free, which `summary` and `critical-path` both want. It is
ubiquitous, dual-licensed, and cheap.

**Targets.** `check` on 10,000 tasks in under 100 ms, cold. The walk is already
`ignore`-based and parallelizable. Bench with `criterion` **0.8.2**, revived under a
new org (`criterion-rs`) and healthy again; `divan` has gone quiet (no release in ~15
months). For load-immune regression gating, `alint` uses deterministic
instruction-count benching, which is worth copying once we have a perf claim to defend.

---

## 15. Schema versioning and format stability

The committed format is a consumer contract. Therefore:
- `schemas/v1/` tracks the frontmatter envelope. Additive, non-breaking changes bump an
  internal `version` field in place.
- A new field on `Task` fails CI until the schema is regenerated and committed, the
  change is *forced into the diff where a reviewer sees it* (ADR-0002).
- `aplan migrate` exists before `v2` does.
- A stable, versioned, machine-checked format is required for reliable automation.

---

## 16. Quality infrastructure

Copying `alint`'s tiering, which is unusually good:

| Layer | Mechanism |
|---|---|
| Unit | inline `#[cfg(test)] mod tests` in `aplan-core` and `aplan-output`; no hand-typed count here, a generated one lands with `facts.json` |
| CLI black-box | **`trycmd` 1.2.0**, `.toml` cases + `.stdout` fixtures. **Pins the exit-code contract.** |
| Snapshot | **`insta` 1.48.0**, the standard; also the official ratatui TUI recipe |
| Property | **`proptest` 1.11.0**, the cycle finder, and `slugify` against `comrak`'s `Anchorizer` |
| Corpus e2e | **landed** in `aplan-e2e/tests/coverage.rs` as an in-Rust fires/silent table per rule, materialised to real temp trees; migrates to one `#[test]` per scenario YAML later |
| Coverage-drift audit | **landed**, meta-tests walk `Rule::ALL` / `Format::ALL`; a rule or format without a test fails CI (ROADMAP §5) |
| Bench | **`criterion` 0.8.2** |
| Drift gates | `xtask gen-schema --check` and `gen-roadmap --check`; `facts.json` via `tests/facts.rs` |
| Dogfood | `aplan check` on `aplan/`, *a plan tool whose own plan does not validate has no business shipping* |

**Immediate gap, closed.** The CLI crate once had **zero tests** and the exit-code
contract the README calls "a contract" was asserted nowhere. Both are now fixed: the
`trycmd` corpus (`crates/aplan/tests/cmd/`) pins every command, format, and exit code, and
the `aplan-e2e` coverage audit forbids a rule or format from shipping untested. Property
testing is not wired yet: the cycle-finder property test (§4) is the next slice, and
`proptest` will be added to the workspace when it lands rather than declared unused before.

**Borrow `facts.json`.** `alint` generates a machine-readable project-facts file from
the code (rule count, output formats, subcommands) and gates it in CI, after a real
incident: a README claim of "60 rule kinds" was *ten behind reality for months*. We have
already lived the small version of this, a README claiming "six invariants" above a
table of ten. **Generate the counts. Never hand-write them.**

**Landed** (`A1-011`). `facts.json` at the repo root is generated by `aplan facts` from
the code's enumerable registries: the rules (with severity) from `Rule::ALL`, the formats
from `Format::ALL`, the evidence kinds from `EvidenceKind::ALL`, the subcommands
introspected from the clap tree, the MCP tools from `aplan_mcp::TOOLS` (`A1-119`), and
(`A1-060`) `config_keys`, every `.aplan.yml` key
this build accepts, walked out of the derived config schema. That last one is a
*capability* rather than an inventory: a consumer staging a binary needs to assert what
it can do, and a version string cannot answer it. `tests/facts.rs` pins the committed file to that output,
so it fails CI on drift, exactly as `gen-schema --check` gates the schema. Two enablers
shipped with it: rule severity became a declared property (`Rule::severity()`, asserted
against every firing site by the coverage audit), and the CLI moved into the `aplan`
library so its command tree is introspectable. `gen-roadmap` (§18, v0.5) is the next layer
on this foundation.

---

## 17. Distribution

`alint` hand-rolls its release pipeline. We should not, and the reason is verified:
**`cargo-dist` (now `dist`) is alive**, v0.32.0, 2026-05-22, repo active. The
"Astral forked it because it died" story is *inverted*: Astral's fork is **archived**,
and its README points back upstream.

| Channel | Mechanism |
|---|---|
| GitHub Releases | `dist`, plan → build → host → publish → announce |
| `cargo install` | crates.io (`aplan`, plus internal crates cargo forces us to publish) |
| `cargo binstall` | free; `dist`'s artifact naming is binstall-compatible |
| Homebrew | custom tap, formula generated per release |
| `npx` | platform-specific packages as `optionalDependencies` + a JS shim (esbuild/Biome model) |
| Docker | multi-stage + `cargo-chef`, musl static → distroless |
| GitHub Action | composite `action.yml` that installs a pinned release |
| pre-commit | `.pre-commit-hooks.yaml` |

Single source of truth for the version is `[workspace.package].version`. The gate that
keeps the documentation honest is `ci/scripts/check-version-pins.sh`, and it is
**narrower than the sentence that used to stand here**, which promised a gate failing
"when any documented version string disagrees" (`A1-318`).

That sentence is not implementable, which was measured rather than argued. The unit is
an occurrence and not a file: `docs/upgrading.md` says on one line that a repository
needs a feature "newer than `v0.3.0`", which must move when v0.4.0 ships, and on the
next that most of the page "shipped in `v0.3.0`", which must not. Nor is syntax enough:
matching every `uses: ...@vX.Y.Z` found four, all of them quotations -- a CHANGELOG
entry, two task bodies, a pilot log. A record of an instruction is not an instruction.

So the gate checks install pins in the documents that *instruct* an adopter, and leaves
records alone. And it compares against the newest **release tag**, not the workspace
version: this workspace carries `X.Y.Z-dev` between releases (`A1-060`), so no published
release ever equals it. `alint`'s equivalent gate does compare against the workspace
version, correctly for `alint` and wrongly for this: inherit the lesson, not the script.

---

## 18. Roadmap

**Sequencing rationale lives in [ROADMAP.md](ROADMAP.md); the plan (`aplan/tasks/`) is the
source of truth for what `aplan` builds and its status, generated into `ROADMAP.status.md`
(ADR-0009).** This section used to carry the phase plan; two roadmaps drift the moment one
is edited, so it defers rather than duplicates.

The shape, non-authoritatively: v0.1 (core, shipped) → **v0.2** the verification
foundation (the test-and-proof scaffold that gates the risky work) → **v0.3** the
evidence ladder (`aplan verify`; the launch-blocking differentiator; `A1-004`) →
**v0.4** config and ergonomics (`A1-001`, `A1-002`) → **v0.5** the self-hosting cutover
(the plan becomes the roadmap's source of truth) → **v0.6** surfaces (`A1-003`) → v0.7
advanced integrity → v0.8 the TUI → v1.0 the committed surface. ROADMAP.md carries the
rationale, the per-cut testing and formal-verification plan, and the cutover checklist;
this document is the design that sits under it.

---

## 19. Decisions that need an ADR

Each of these is a real fork in the road, not a formality.

1. **ADR-0003: numeric rule codes.** `alint` has *no* numeric codes, rule identity is a
   snake_case string in a registry, deliberately open to plugins. We use `APLAN001`. This
   is a **divergence, not a mirror**, and it is defensible (our rule set is closed and
   fixed; `rustc`/`clippy`/ESLint all number theirs; a stable code is a consumer
   contract). Write it down so nobody "fixes" it later.
2. **ADR-0004: evidence verification levels.** The ladder, and why `check` is hermetic
   while `verify` is not.
3. **ADR-0005: trust boundary for `--execute`.** Why a task file can never name a
   command. Mirrors `alint`'s ADR-0004.
4. **ADR-0006: one-way mirror only.** Cite the git-bug bridge failures; make the refusal
   permanent and public.

Two more were on this list and have since been written: the exit-code contract is
`ADR-0007`, and `paths` as the relevance mechanism is `ADR-0008`. A list telling a reader
to write what already exists sends them to do work that is done, so an entry leaves it when
its record lands (`A1-222`, which added the gate that noticed).

---

## 20. Open questions

- **Kiro (AWS), researched; the claim survives.** Kiro is GA (IDE v1.0.116, 2026-07-09)
  with `PreTaskExec` hooks that block on exit code 2, but hooks block *starting* a task,
  not the checkbox transition; there is no `PreTaskComplete` trigger, so a hook cannot
  veto marking a task done, and its spec workflow does not verify completion. Pre-empt
  "just build a gate in a hook" (RESEARCH §14.5). *(Backlog.md's cycle detection, once an
  open question here, is settled: no, see §3.5.)*
- Should `traces` support line ranges, not just heading anchors? Anchors are stable
  across edits; line numbers are not. Probably no.
- Signed evidence (git notes / sigstore) is **no longer an open question**, it is planned
  as `aplan attest` (ROADMAP v0.6), promoted on 2026 market demand (RESEARCH §15).
- Multi-repo plans. We are repo-scoped, which is our moat (offline CI) and our ceiling
  (portfolio rollup). Do not solve this before someone asks twice.
