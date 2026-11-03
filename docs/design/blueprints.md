# Blueprints: reusable plan fragments

> A proposal, not a decision. It records the evidence, the strongest case, the minimum design,
> and the questions the `blueprints` cut must settle. ADR-0062 accepts the name, two-arity
> concept, explicit-file lifecycle and non-goals; the remaining tasks implement that boundary.
> This proposal was audited against the current `add --from` implementation and primary-source
> prior art on 2026-10-05 (`A1-396`). The three candidate shapes were then written and applied
> with that implementation on 2026-10-30; this document is amended in place with the result
> (`A1-383`, section 12).

## 1. The proposal in one sentence

A blueprint is a named, inspectable recipe for creating one or more ordinary aplan tasks, with
declared string parameters and symbolic references between tasks in the same recipe.

It is a compiler front end for the existing task model and atomic mutation path. It is not a new
task type, a workflow engine, a methodology, or a program that continues to control the tasks it
created.

## 2. The strongest honest case

The two useful arities solve related but different repetition:

- **One task:** preserve fields people repeatedly forget: paths, tags, a body outline, effort and
  trace conventions. Examples are an aplan rule change, an incident review, a compatibility
  review, or a release note. This is where this repository has direct recurrence.
- **A task set:** preserve topology as well as fields. Examples are design -> implementation ->
  integration test -> documentation, an incident -> remediation -> regression test -> postmortem,
  or a repeated release checklist. A plain copied checklist loses typed dependencies;
  a blueprint mints ids and resolves the local graph atomically.

The practical value is consistency without pretending that completion is reusable. A blueprint
can create the same questions and proof obligations each time, but every resulting task remains an
ordinary task whose status and evidence must be earned in that repository.

The 2026-10-30 spike narrowed the design argument. The one-task `review` and `rule` shapes already
work as copied manifests and do not save meaningful work merely by replacing editor substitutions
with CLI substitutions. The multi-task `feature` shape does: a copied manifest makes the author
choose collision-free ids and update every dependency reference, while local keys remove both the
choices and their consistency risk. One-entry blueprints remain a natural, nearly free arity of the
same document, but they are examples rather than the independent reason to build the feature.

This is especially useful to agents: an agent can inspect a selected file, bind a small parameter
set, preview the exact post-state, and submit ordinary task files for review instead of recreating
repository conventions from prose. Humans get the same benefit through a deterministic CLI.

## 3. Scope evidence

The implementation spike applied one-task and multi-task manifests through the existing
`add --from` path. One-task manifests already work as copied files and gain little from
parameter substitution alone. Multi-task manifests expose the load-bearing gap: authors must
choose collision-free ids and update every internal relationship consistently.

That result justifies two additions and no broader subsystem:

1. bind declared string parameters once; and
2. resolve local keys to minted ids across the complete atomic batch.

The evidence does not justify discovery, catalogues, composition, replay, installation, remote
fetching, or a continuing relationship between the recipe and the tasks it created. Those remain
outside the accepted V1 boundary.

## 4. What exists already, and the actual gaps

`aplan add --from file.yml` already accepts several task entries, validates the resulting plan,
and writes all or none. A static manifest is therefore a reusable task set today. The 2026-10-30
execution spike confirmed only two gaps that a copied manifest cannot provide safely:

1. declared parameters, so repeated text is bound once rather than edited at every occurrence;
2. local keys, so relationships survive ids being minted at application time.

The other proposed gaps were corrected or removed. An explicit, reviewed file path is adequate;
repository discovery, `list` and `show` save no authorship work. The applied tasks are self-contained,
so a one-shot v1 needs no provenance field, recipe revision, replay or update promise. None of the
three shapes needed composition. Copying a reviewed file is the complete v1 sharing policy, not a
distribution subsystem. Section 12 records each disposition and the exact refusal that established
the two remaining gaps.

`A1-343` is a predecessor, not a blueprint feature: it decides which existing task-model fields a
bulk manifest may carry. Blueprints should reuse the resulting entry type rather than fork it.

## 5. Terminology

The accepted term is **blueprint** (ADR-0062). A blueprint contains one or more task entries; the
one-entry case needs no second noun.

`template` is already overloaded by task-body and filename scaffolds. `bundle` is a worse
collision: spec-kit's current
[bundle model](https://github.com/github/spec-kit/blob/main/docs/reference/bundles.md) is a
versioned package of extensions, presets, workflows and steps, with installation, provenance and
catalog resolution. aplan already interoperates with spec-kit, so using that word for tasks would
give the same adopter two incompatible meanings.

ADR-0062 records the naming and scope decision completed by `A1-384`.

## 6. Minimum language

The V1 spelling below is derived from the parser types; `schemas/v1/blueprint.json` is the
machine-readable contract.

```yaml
schema: aplan/blueprint/v1
name: feature
title: A feature from design through verification
description: |
  Creates a small feature graph in an existing cut.

parameters:
  - name: feature
    description: The feature name used in task prose.

tasks:
  - key: design
    title: "Decide how {{feature}} works"
    effort: 1
    tags: [design]
    body: |
      ## Why

      Record the decision and its constraints.

  - key: implement
    title: "Implement {{feature}}"
    effort: 3
    blocked_by: [local:design]

  - key: verify
    title: "Prove {{feature}} fails when reverted"
    effort: 1
    blocked_by: [local:implement]
```

The cut is an application argument, such as `--cut M-v0.9`, rather than an unused parameter in
the file. Every declared parameter in the example is consumed. Version concepts stay separate:

- `schema` identifies the DSL contract the parser understands;
- a future blueprint `revision` would identify an author's recipe revision, but v1 has none; a
  later update/replay proposal must earn both the lifecycle and the field with new evidence.

### Parameter semantics

The deliberately small v1 rule is:

- parameters are strings and every V1 parameter is required; the language has no default key;
- a supplied value must contain visible text. An absent, blank, whitespace-only or zero-width value
  does not satisfy a required declaration;
- every parameter is declared and used, and every placeholder names a declaration;
- parameter names begin with an ASCII letter or `_`, followed by ASCII letters, digits, `_` or
  `-`; duplicate declarations and caller-supplied names the document does not declare are refused;
- a future default must be a literal deterministic string and must ship with complete binding
  semantics rather than turning absence into an implicit empty value;
- `{{name}}` binds a parameter in the blueprint title and description and in every string-valued
  task field. The blueprint `name`, parameter declarations and task-local `key` are structural;
  a real placeholder in a structural field is refused;
- substitution occurs independently in each typed string slot **after YAML is deserialised**, never
  in raw YAML. A value containing newlines, `:`, brackets or list markers remains one value in the
  existing slot and cannot inject a field, task, list item or relationship;
- `{{{{` emits a literal `{{` and `}}}}` emits a literal `}}`. Unmatched delimiters and malformed
  placeholder names are refusals, so escaped literal text is structurally distinct from an
  unresolved placeholder; and
- supplied values are opaque and non-recursive. If a value contains `{{other}}`, those bytes stay
  literal rather than becoming a second parameter program; and
- validation tracks placeholders structurally instead of rejecting any output text that happens
  to contain `{{`.

There are no environment-derived defaults, conditionals, loops, expressions, shell commands or
network reads.

### Reference semantics

A local key begins with an ASCII letter or `_`, followed by ASCII letters, digits, `_` or `-`.
Keys are unique within the document. Every value in `blocked_by`, `prefer_after`, `ratified_by`
and `contains` uses exactly one explicit namespace:

- `local:design` names the task whose blueprint key is `design` and resolves to the id minted for
  that task in this application;
- `task:A1-123` names the existing task `A1-123` and resolves to that id; and
- a bare string is refused even if it happens to match a local key or existing task today.

This makes a collision unambiguous: when a blueprint key is `A1-123` and the plan already has task
`A1-123`, `local:A1-123` names the new sibling and `task:A1-123` names the existing task. An empty
namespace target, missing local key, duplicate key or unknown existing task is a refusal before any
plan bytes are written.

Blueprint entries cannot set `id`: every application mints fresh ids. They also cannot set their
manifest `cut` field: V1 has one required application cut, resolved by id or label before minting,
and every created entry joins it. The target must already exist. Creating a cut inside the same
blueprint or distributing entries across cuts is outside the one-target V1 contract.

### Post-state and ids

Application reuses the same lock, id allocator and post-state validation as `add --from`. It mints
all sibling ids against one scratch post-state, resolves the complete graph, and then sends ordinary
manifest entries through the shared authoring planner. Validation covers the complete proposed
plan, and any denied finding leaves the working tree unchanged. If another writer changes the plan
before publication, the guarded writer returns stale; the caller re-reads and re-previews the whole
blueprint so sequential ids are minted again rather than partially retaining the first attempt.

A dry run cannot reserve sequential ids. It reports **predicted ids for the plan snapshot it
read**, clearly labelled as predictions. A later apply reacquires the lock and may print different
ids if the plan changed. The ids in a successful apply's output are exact. Dry-run output should
also carry a machine-readable source identity and digest so a reviewer can match what was previewed
without pretending the preview reserved state.

## 7. Source, trust and provenance

### V1: one explicit local file

The minimum useful version accepts an explicit local `--file`, exactly as `add --from` does. The
spike showed no authorship work saved by a catalog or logical-name lookup, so v1 has no discovery,
source precedence, duplicate-name or shadowing semantics. The selected path and content digest
belong in dry-run and apply output so the reviewed input can be identified without installing it.

Explicitly selected local files are plan-authoring data, not executable programs: the parser has no
commands, expressions or fetches. They can still request invalid plan changes, so the existing
atomic post-state gate is the trust boundary that matters.

### Later catalogs require new usage evidence

Per-user catalogs, built-ins, installation and remote fetching are separate product decisions, not
requirements smuggled into source precedence. spec-kit's project/user/built-in stack is useful
prior art, but it also carries trust indicators, provenance and removal/update semantics because
it is a distribution system. aplan should not copy that subsystem before it has evidence that a
reviewed repository file is insufficient.

There is no fetch-and-apply path in v1. Sharing means reviewing and copying a file locally, then
selecting its path explicitly. The spike recorded that boundary and dropped `A1-388` and
`A1-394`; a later catalog or distribution design starts with new adopter evidence rather than
reviving either by implication.

### Provenance follows lifecycle

Application is one-shot. The created tasks are self-contained, and source identity plus digest in
the command output and git history are sufficient. There is no task-model provenance field or
author-controlled recipe revision in v1. If a future command promises update, replay or drift
detection, it needs a canonical source identity, recipe revision or digest and bound parameters. Copier's
[update model](https://copier.readthedocs.io/en/latest/updating/) illustrates why answers and
template provenance become necessary once generated output can be updated.

The spike made the lifecycle decision and dropped `A1-391`. A model field is not justified merely
because provenance sounds aligned with aplan's thesis: evidence proves task status, whereas recipe
origin is history for a one-shot authoring input.

## 8. CLI and agent surface

The initial surface has one operation over an explicit file. Final command spelling belongs to
`A1-389`; ADR-0062 fixes the operation's lifecycle boundary but deliberately leaves its syntax to
that implementation task. The required shape is:

```console
aplan blueprint apply --file feature.yml --cut M-v0.9 --set feature="bulk close" --dry-run
aplan blueprint apply --file feature.yml --cut M-v0.9 --set feature="bulk close"
```

The file itself is the inspection surface. `apply --dry-run` parses it, validates bindings and
renders the complete proposed post-state; without `--dry-run`, the same operation commits through
the shared mutation path. Human, JSON and agent formats describe the same result. There is no
`list` or `show` until a real discovery mechanism earns those verbs.

Agents are a primary user, so `A1-389` must either add MCP parity for preview and apply through the
same mutation gate, or record a concrete deferral with a usable CLI/agent-format path. It may not
leave parity as an unowned question. The command also owes every existing command and mutation
census, CLI facts, HLD/README/CHANGELOG coverage and a behaviour name for any widened existing
guarantee.

## 9. Composition and shipped content

None of the three materialised shapes needed composition. `A1-390` is therefore dropped rather
than implemented because the original ask used the word "modular". The language gains no
conditions, loops, expressions or includes. A future inclusion design requires a concrete repeated
shape and a new task; it does not inherit approval from this proposal.

The repository should initially ship **examples and conformance fixtures, not built-in
methodologies**. `rule` and `review` are useful examples for aplan contributors, but presenting them
as universal built-ins contradicts the claim that aplan does not prescribe a planning method.
`feature` is equally opinionated. `A1-392` owns the evidence threshold for promoting an example to
a built-in later, and its census must discover whatever directory is actually chosen.

## 10. Sequencing and ownership

| order | task | decision or deliverable |
|---:|---|---|
| 1 | `A1-383` | materialise candidate shapes with `add --from`; remove unsupported gaps |
| 2 | `A1-384` | name, arity, one-shot explicit-file lifecycle and non-goals ADR |
| 3 | `A1-385` | types and derived schema, reusing the manifest entry |
| 4 | `A1-386` | total, parsed-field parameter binding |
| 5 | `A1-387` | namespaced references, cut target and atomic graph resolution |
| 6 | `A1-389` | one apply operation, renderers, MCP disposition and every surface census |
| 7 | `A1-392` | examples/conformance fixtures; no runtime built-ins |
| 8 | `A1-393` | user, agent, HLD, changelog and upgrade documentation |

`A1-388`, `A1-390`, `A1-391` and `A1-394` were dropped by the spike: discovery, composition,
persistent provenance and distribution did not survive contact with the three concrete files.

`M-blueprints` is blocked by all of them and by `M-v0.9`, which now means the coherent remaining
generality work rather than every later finding that once accumulated in that cut. It does not
wait on all of `M-adoption-next`: `A1-387` names `A1-343` directly, at the first blueprint step
that needs the broader manifest entry. The tasks carry the same internal edges, so `aplan ready`
exposes the decision order instead of presenting twelve dependent jobs as though they could start
together.

## 11. Decision rule

Proceed with the explicit-file, one-shot minimum. The feature graph reduced fourteen
instance-specific scalar sites, including five id/reference sites and three collision-free id
choices, to a feature value and cut argument. That is meaningful because it removes topology work,
not because it replaces YAML with new syntax. Treat discovery, catalogs, composition, built-ins,
replay and distribution as absent features that must earn new evidence. Every generated task must
remain understandable without the blueprint that created it.

## 12. Execution record: 2026-10-30

`A1-383` used the built `aplan` binary in a fresh Git repository under `~/.tmp`, initialised it
with `aplan init`, and created one live `M-spike` cut. The three candidate files below were then
passed to `aplan add --from` exactly as shown. They are ordinary manifests, not proposed blueprint
syntax.

### 12.1 The files that were applied

The one-task review:

```yaml
- title: Review widget integration boundaries
  cut: M-spike
  effort: 2
  tags: [review]
  paths:
    - docs/integrations/widget.md
    - aplan/**
    - .aplan.yml
  body: |
    ## Why

    Review the `widget` integration and record concrete findings.

    ## Acceptance criteria

    - [ ] the plan exists and passes `aplan check`
    - [ ] every finding is dispositioned to a task or a recorded non-finding
```

The one-task rule:

```yaml
- title: Report live tasks with blank owners
  cut: M-spike
  effort: 1
  tags: [rules]
  paths:
    - crates/aplan-core/src/diagnostic.rs
    - crates/aplan-core/src/validate.rs
    - crates/aplan-e2e/tests/coverage.rs
    - crates/aplan-e2e/tests/rule_tests.rs
    - docs/rule-catalogue.md
    - CHANGELOG.md
  body: |
    ## Why

    `APLAN999` reports a live task whose owner is blank.

    ## Acceptance criteria

    - [ ] `APLAN999` fires for a blank owner and stays silent for an absent owner
    - [ ] unit and end-to-end tests assert the task id, message and agent instruction
    - [ ] the rule catalogue and changelog announce `APLAN999`
```

The three-task feature:

```yaml
- id: S-100
  title: Decide bulk archive semantics
  cut: M-spike
  effort: 1
  tags: [design]
  paths: [docs/adr/**]
  body: |
    ## Why

    Decide the contract and exclusions for bulk archive.
- id: S-101
  title: Implement bulk archive
  cut: M-spike
  effort: 3
  paths: [src/**]
  blocked_by: [S-100]
  body: |
    ## Why

    Implement bulk archive against the recorded contract.
- id: S-102
  title: Prove and document bulk archive
  cut: M-spike
  effort: 1
  paths: [tests/**, README.md, CHANGELOG.md]
  blocked_by: [S-101]
  body: |
    ## Why

    Prove bulk archive fails when reverted and document its use.
```

All three applied with exit 0. The feature was first dry-run and named the same three explicit ids
and the `M-spike` edit that the subsequent apply wrote. The final plan contained ten tasks and
reported:

```text
ok: 10 tasks, 0 warning(s), DAG acyclic
```

`aplan show S-101 --format json` reported `S-100` in `waits_on`, and `S-102` in `releases`, so the
workaround preserved the intended topology rather than merely producing three files.

### 12.2 Exact refusals and workarounds

The parameter wrapper from section 6 is outside the manifest grammar:

```text
aplan: blueprint-wrapper.yml is not a list of tasks: invalid type: map, expected a sequence.
Each entry is a mapping with a `title` and any of the fields `aplan add` takes as flags
```

Putting a local key on a manifest entry is refused before application:

```text
aplan: local-key.yml is not a list of tasks: .[0]: unknown field `key`, expected one of
`title`, `id`, `phase`, `cut`, `effort`, `owner`, `tags`, `paths`, `traces`, `blocked_by`,
`body` at line 1 column 3. Each entry is a mapping with a `title` and any of the fields
`aplan add` takes as flags
```

Removing `key` but retaining `blocked_by: [local:design]` does not create an accidental string
convention; application refuses it as an ordinary missing task:

```text
aplan: entry 2 (`Implement bulk export`): blocked by task(s) the plan does not have and this file
does not declare: local:design
```

An unresolved placeholder is not refused. A dry run proposed a literal task, which confirms that
parameter syntax cannot be layered on by documentation alone:

```text
would write S-106  todo  Review {{repository}} integration boundaries
  -> aplan/S-106.md
```

Finally, applying the successful feature file a second time refused atomically:

```text
aplan: entry 1 (`Decide bulk archive semantics`): task `S-100` already exists in aplan/S-100.md
```

The plain-manifest workaround was a second copy for `bulk restore`: choose `S-103`, `S-104` and
`S-105`, then update both `blocked_by` references. That edited copy applied with exit 0 and the
plan still checked. This is the gap local keys close.

### 12.3 Work saved, measured as review sites

Elapsed typing time would depend more on the editor than the design, so the exercise counted the
instance-specific scalar sites a reviewer must verify. A global replace can make several edits in
one operation; the count is review surface, not a claim about keystrokes.

| shape | tasks | plain-manifest instance work | bound blueprint input | result |
|---|---:|---|---|---|
| review | 1 | repository at 3 sites; cut at 1 | repository and cut | 4 sites to 2 inputs, but no topology; not meaningful alone |
| rule | 1 | code at 3 sites; predicate/title prose at 3; cut at 1 | code, prose values and cut | preserves a useful scaffold, but copying already does; not meaningful alone |
| feature | 3 | feature at 6 sites; cut at 3; 3 id declarations and 2 references | feature and cut; local keys are static | 14 sites to 2 inputs, removes 3 collision-free id choices and 2 consistency checks |

The feature row is the warrant for implementation. The other two are conformance examples showing
that the same language naturally handles arity one; they are not evidence for catalogs or built-in
methodologies.

### 12.4 Disposition of every proposed gap

| proposed gap | spike result | plan consequence |
|---|---|---|
| declared parameters | confirmed: placeholders are literal and six feature occurrences must agree | retain `A1-386` |
| local keys | confirmed: `key` is unknown, `local:design` is missing, and fixed ids collide | retain `A1-387` |
| schema/type wrapper | confirmed by the top-level-map refusal; it must remain distinct from the manifest list while reusing `Entry` | retain `A1-385` |
| discovery and inspection | removed: explicit `--from` paths and dry-run were sufficient; no authorship work was saved | drop `A1-388`; remove `list` and `show` from `A1-389` |
| persistent provenance and replay | removed: emitted tasks are self-contained; one-shot source identity belongs in output and Git history | drop `A1-391`; record the non-goal in `A1-384` |
| composition | removed: no candidate shape includes another | drop `A1-390` |
| built-ins | removed from runtime scope: the three opinionated shapes remain examples and conformance fixtures | retain the narrowed `A1-392` |
| catalogs, installation and distribution | removed: reviewing and copying a local file completed the exercise | drop `A1-394`; a future design needs new evidence |
| atomic apply and dry-run | already supplied by `add --from`; the blueprint adapter must reuse them rather than rebuild them | narrow `A1-389` to one explicit-file apply operation |

The cut therefore falls from twelve active tasks to eight. That is the spike doing its job: the
remaining implementation adds two capabilities plain manifests demonstrably lack, rather than a
second plan-authoring subsystem around capabilities they already have.
