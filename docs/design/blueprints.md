# Blueprints: reusable plan fragments

> A proposal, not a decision. It records the evidence, the strongest case, the minimum design,
> and the questions the `blueprints` cut must settle. The implementation still requires an ADR.
> This proposal was audited against the current `add --from` implementation and primary-source
> prior art on 2026-10-05 (`A1-396`).

## 1. The proposal in one sentence

A blueprint is a named, inspectable recipe for creating one or more ordinary aplan tasks, with
declared string parameters and symbolic references between tasks in the same recipe.

It is a compiler front end for the existing task model and atomic mutation path. It is not a new
task type, a workflow engine, a methodology, or a program that continues to control the tasks it
created.

## 2. The strongest honest case

The two useful arities solve related but different repetition:

- **One task:** preserve fields people repeatedly forget: paths, tags, a body outline, effort and
  trace conventions. Examples are an aplan rule change, an adoption pilot, an incident review or
  a release note. This is where this repository has direct recurrence.
- **A task set:** preserve topology as well as fields. Examples are design -> implementation ->
  integration test -> documentation, an incident -> remediation -> regression test -> postmortem,
  or a repeated release/adoption checklist. A plain copied checklist loses typed dependencies;
  a blueprint mints ids and resolves the local graph atomically.

The practical value is consistency without pretending that completion is reusable. A blueprint
can create the same questions and proof obligations each time, but every resulting task remains an
ordinary task whose status and evidence must be earned in that repository.

This is especially useful to agents: an agent can inspect a named shape, bind a small parameter
set, preview the exact post-state, and submit ordinary task files for review instead of recreating
repository conventions from prose. Humans get the same benefit through a deterministic CLI.

## 3. What the evidence does and does not establish

### Adjacent open-source trackers

The demand scan found no request for reusable task sets in the closest adjacent trackers:

- Backlog.md [#1016](https://github.com/MrLesk/Backlog.md/issues/1016) requests configurable task
  **filenames**, while [#618](https://github.com/MrLesk/Backlog.md/issues/618) and
  [#581](https://github.com/MrLesk/Backlog.md/issues/581) concern milestone management.
- claude-task-master [#591](https://github.com/eyaltoledano/claude-task-master/issues/591) calls
  itself a template system, but templates the LLM system prompt rather than a task or task set.

So this cut is not justified by market pull, and it must not delay the public milestone.

### Broader project-management prior art

The concept is nevertheless established and legible. Linear has both
[issue templates](https://linear.app/docs/issue-templates) and
[project templates](https://linear.app/docs/project-templates), with the latter able to carry
issues and milestones. Asana supports creating repeated projects from
[custom templates](https://help.asana.com/s/article/create-projects-quickly-with-templates), and
Microsoft Planner supports
[custom plan templates](https://support.microsoft.com/en-us/planner/use-custom-templates-microsoft-planner).
That supports the use case, not demand for this implementation.

### First-party recurrence

The original proposal's counts mixed substring mentions with frontmatter declarations. The
reproducible 2026-10-05 snapshot is:

| measure | count | command or definition |
|---|---:|---|
| task files | 411 | `find aplan/tasks -maxdepth 1 -name '*.md'` |
| pilot logs | 17 | Markdown files in `private pre-public adoption records`, excluding `README.md` |
| task titles containing `pilot` | 25 | a noisy upper bound; includes tasks about pilot-log gates and findings |
| tasks whose `paths` contain `crates/aplan-core/src/diagnostic.rs` | 24 | exact `aplan tasks --format json` path membership |
| tasks whose `paths` contain `docs/rule-catalogue.md` | 8 | the same exact membership test |
| tasks whose `paths` contain `docs/upgrading.md` | 34 | the same exact membership test |

The counts establish repeated single-task conventions and repeated adoption exercises. They do
not establish that all instances share one exact path set, or that multi-task blueprints are
already demanded. The spike in `A1-383` must materialise representative shapes with today's
`add --from` before the cut treats any proposed field as necessary.

## 4. What exists already, and the actual gaps

`aplan add --from file.yml` already accepts several task entries, validates the resulting plan,
and writes all or none. A static manifest is therefore a reusable task set today. Blueprints add
only what a copied manifest cannot provide:

1. declared parameters, so an instance does not require editing a copy;
2. local keys, so relationships survive ids being minted at application time;
3. discovery and inspection;
4. a deliberate answer about provenance and later replay;
5. optionally, bounded static composition after the basic path is proved; and
6. eventually, a sharing policy, if copying a reviewed file is insufficient.

`A1-343` is a predecessor, not a blueprint feature: it decides which existing task-model fields a
bulk manifest may carry. Blueprints should reuse the resulting entry type rather than fork it.

## 5. Terminology

The proposed term is **blueprint**. A blueprint contains one or more task entries; the one-entry
case needs no second noun.

`template` is already overloaded by task-body and filename scaffolds. `bundle` is a worse
collision: spec-kit's current
[bundle model](https://github.com/github/spec-kit/blob/main/docs/reference/bundles.md) is a
versioned package of extensions, presets, workflows and steps, with installation, provenance and
catalog resolution. aplan already interoperates with spec-kit, so using that word for tasks would
give the same adopter two incompatible meanings.

The naming and scope remain an ADR decision owned by `A1-384`.

## 6. Minimum language

This sketch is illustrative; `A1-385` owns the derived schema and final spelling.

```yaml
schema: aplan/blueprint/v1
name: feature
title: A feature from design through verification
description: |
  Creates a small feature graph in an existing cut.

parameters:
  - name: feature
    description: The feature name used in task prose.
    required: true

tasks:
  - key: design
    title: "Decide how {{feature}} works"
    effort_days: 1
    tags: [design]
    body: |
      ## Why

      Record the decision and its constraints.

  - key: implement
    title: "Implement {{feature}}"
    effort_days: 3
    blocked_by: [local:design]

  - key: verify
    title: "Prove {{feature}} fails when reverted"
    effort_days: 1
    blocked_by: [local:implement]
```

The cut is an application argument, such as `--cut M-v0.9`, rather than an unused parameter in
the file. Every declared parameter in the example is consumed. Version concepts stay separate:

- `schema` identifies the DSL contract the parser understands;
- a future blueprint `revision` would identify an author's recipe revision, but exists only if
  `A1-391` decides update/replay or persistent provenance needs it.

### Parameter semantics

The deliberately small v1 rule is:

- parameters are strings;
- every parameter is declared and used, and every placeholder names a declaration;
- parameters are required unless they have a literal, deterministic default;
- substitution occurs only in documented string fields **after YAML is deserialised**, never in
  raw YAML, so a value cannot inject a field, list item or relationship;
- literal placeholder delimiters have one documented escape; and
- validation tracks placeholders structurally instead of rejecting any output text that happens
  to contain `{{`.

There are no environment-derived defaults, conditionals, loops, expressions, shell commands or
network reads.

### Reference semantics

A local key and an existing task id must be syntactically distinct. The sketch uses
`local:design`; an external edge would use an explicit form such as `task:A1-123`. Bare strings
must not change meaning according to whether a coincidentally named task exists.

`A1-387` owns the final grammar, all relationship fields, targeting an existing cut and atomic
resolution. A local reference that is absent, duplicated or in the wrong field is a refusal before
anything is written.

### Post-state and ids

Application reuses the same lock, id allocator and post-state validation as `add --from`.
Validation covers the complete proposed plan, and any finding leaves the working tree unchanged.

A dry run cannot reserve sequential ids. It reports **predicted ids for the plan snapshot it
read**, clearly labelled as predictions. A later apply reacquires the lock and may print different
ids if the plan changed. The ids in a successful apply's output are exact. Dry-run output should
also carry a machine-readable source identity and digest so a reviewer can match what was previewed
without pretending the preview reserved state.

## 7. Discovery, trust and provenance

### Stage 1: repository-local only

The minimum useful version discovers blueprints below the plan root, for example
`aplan/blueprints/*.yml`, and may accept an explicit local `--file`. Discovery is sorted and
deterministic. It refuses duplicate logical names, case-folding collisions on relevant platforms,
paths that escape the root, unsafe symlink resolution and an invalid selected source. It never
silently falls through from an invalid higher-priority source to a different recipe.

Repository-local files are plan-authoring data, not executable programs: the parser has no
commands, expressions or fetches. They can still request invalid plan changes, so the existing
atomic post-state gate is the trust boundary that matters.

### Stage 2: only after usage evidence

Per-user catalogs, built-ins, installation and remote fetching are separate product decisions, not
requirements smuggled into source precedence. spec-kit's project/user/built-in stack is useful
prior art, but it also carries trust indicators, provenance and removal/update semantics because
it is a distribution system. aplan should not copy that subsystem before it has evidence that a
reviewed repository file is insufficient.

There is no fetch-and-apply path in v1. Sharing means reviewing and copying a file into the
repository. `A1-388` owns deterministic local discovery; `A1-394` records the later distribution
decision.

### Provenance follows lifecycle

If application is one-shot, the created tasks are self-contained and source provenance can remain
in command output and git history. If a future command promises update, replay or drift detection,
it needs a canonical source identity, recipe revision or digest and bound parameters. Copier's
[update model](https://copier.readthedocs.io/en/latest/updating/) illustrates why answers and
template provenance become necessary once generated output can be updated.

`A1-391` must decide the lifecycle before adding a task-model field. A model field is not justified
merely because provenance sounds aligned with aplan's thesis: evidence proves task status, whereas
recipe origin may only be history.

## 8. CLI and agent surface

The initial surface is:

```console
aplan blueprint list
aplan blueprint show feature
aplan blueprint apply feature --cut M-v0.9 --set feature="bulk close" --dry-run
aplan blueprint apply feature --cut M-v0.9 --set feature="bulk close"
```

`list` reports deterministic names and origins. `show` parses and validates the selected blueprint,
then describes its parameters and task graph. `apply` renders the proposed post-state and, without
`--dry-run`, commits it through the shared mutation path. Human, JSON and agent formats describe
the same result.

Agents are a primary user, so `A1-389` must either add MCP parity for inspection and apply through
the same mutation gate, or record a concrete deferral with a usable CLI/agent-format path. It may
not leave parity as an unowned question. The command also owes every existing command and mutation
census, CLI facts, HLD/README/CHANGELOG coverage and a behaviour name for any widened existing
guarantee.

## 9. Composition and shipped content

Composition is not required for the first useful release. If the spike proves it useful, `A1-390`
may add static inclusion only:

- each inclusion has an alias that namespaces its local keys;
- parameter bindings are explicit;
- cycles report the full inclusion path; and
- maximum depth, total expanded blueprints and total generated tasks are bounded before expansion
  can consume unbounded resources.

The language still gains no conditions, loops or expressions. If composition does not survive the
spike, the task is dropped rather than implemented because the original ask used the word
"modular".

The repository should initially ship **examples and conformance fixtures, not built-in
methodologies**. `rule` and `pilot` are useful examples for aplan contributors, but presenting them
as universal built-ins contradicts the claim that aplan does not prescribe a planning method.
`feature` is equally opinionated. `A1-392` owns the evidence threshold for promoting an example to
a built-in later, and its census must discover whatever directory is actually chosen.

## 10. Sequencing and ownership

| order | task | decision or deliverable |
|---:|---|---|
| 1 | `A1-383` | materialise candidate shapes with `add --from`; remove unsupported gaps |
| 2 | `A1-384` | name, arity and non-goals ADR |
| 3 | `A1-391` | one-shot versus replay/update, and therefore provenance/revision needs |
| 4 | `A1-385` | types and derived schema, informed by those decisions |
| 5 | `A1-386` | total, parsed-field parameter binding |
| 6 | `A1-387` | namespaced references, cut target and atomic graph resolution |
| 7 | `A1-388` | deterministic repository-local discovery and path safety |
| 8 | `A1-389` | CLI, renderers, MCP disposition and every surface census |
| 9 | `A1-390` | bounded static composition, only if the spike supports it |
| 10 | `A1-392` | examples/conformance fixtures; built-ins only with evidence |
| 11 | `A1-394` | record the sharing/distribution boundary; no v1 fetch path |
| 12 | `A1-393` | adopter, agent, HLD, changelog and upgrade documentation |

`M-blueprints` is blocked by all of them and by `M-v0.9`, which now means the coherent remaining
generality work rather than every later finding that once accumulated in that cut. It does not
wait on all of `M-adoption-next`: `A1-387` names `A1-343` directly, at the first blueprint step
that needs the broader manifest entry. The tasks carry the same internal edges, so `aplan ready`
exposes the decision order instead of presenting twelve dependent jobs as though they could start
together.

## 11. Decision rule

Proceed with the repository-local, one-shot minimum only if the spike shows a meaningful reduction
over `add --from`. Treat broader catalogs, composition, built-ins and distribution as separable
features that must earn their own evidence. The proposal succeeds if it makes repeated task
authorship smaller and safer while every generated task remains understandable without the
blueprint that created it.
