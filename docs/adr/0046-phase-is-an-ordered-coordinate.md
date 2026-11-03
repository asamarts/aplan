# ADR-0046: Phase is an ordered coordinate, not a second containment level

Date: 2026-10-09
Status: Accepted
Tasks: `A1-187`
Supersedes ADR-0014 and ADR-0042 only where they say a cut is the only grouping the model knows.
Their decisions against a second containment level stand.

## Context

ADR-0014 decided that a project needing epics inside cuts should use a declared tag family rather
than adding another membership relation. Its text made a broader claim: "the cut ... remains the
only grouping the model knows." That was already false. Every task had a required numeric `phase`,
and `aplan summary` grouped work by it.

The mismatch is no longer a harmless wording problem. Phase now reaches all of these surfaces:

- the required v1 task schema and every task file;
- the default human summary and `aplan/summary/v2`;
- `aplan/tasks/v1`, `aplan/show/v1`, and the MCP show projection;
- `add`, batch add, import, move, supersede, and MCP add;
- both id strategies, which use it when choosing a prefix for a new id; and
- five import names, whose source formats have no equivalent aplan coordinate.

The mechanisms also do different jobs. A cut owns work through checked edges. It affects readiness,
completion and releases. A tag family declares a project vocabulary and may impose optional or
exactly-one cardinality. A phase is one ordered number on each task. It creates no edge, owns no
work, gates nothing, and supplies no execution order.

The current plan demonstrates why those roles need not collapse. Its 470 work tasks occupy phases
0, 1 and 2; 41 are cross-cutting and belong to no cut. A cut may legally span phases, while a phase
may span many cuts. Changing a task's phase does not rekey its stable id. The phase is only an input
when a later id is minted, so an id convention may reflect the coordinate without becoming an
invariant the model has to enforce.

## Decision

**1. `phase` remains a required `u32` in the v1 task model.** It is the built-in ordered reporting
coordinate. `aplan summary` uses it by default and orders its rows numerically. A repository that
does not need several reporting epochs may use one value everywhere; the value is still explicit,
not an implicit default hidden from consumers.

**2. A cut remains the only task-membership level.** A task belongs to a cut because that cut's
milestone lists it. Cut membership is checked, affects the graph, and can gate completion. Phase
does none of those things. ADR-0014's rejection of a second membership field therefore stands,
as does ADR-0042's distinction between owning work and borrowing a descendant's rollup.

**3. Declared tag families remain the project-defined grouping mechanism.** `roadmap.group_by`
and `summary.group_by` select such a family when a repository needs epic, kind, team, stage, or
another named vocabulary. `APLAN026` checks spelling and `APLAN030` can check cardinality. Phase is
not an escape from those checks: it is one built-in ordered coordinate with fixed numeric syntax,
not a repository-defined vocabulary.

**4. Phase is an id-minting hint, not identity.** Sequential ids prefer the family already used in
the requested phase and adapt a phase-bearing prefix for a new phase. ULID ids retain the same
derived prefix. Existing ids remain stable when `mv --phase` changes the coordinate; `rekey` is the
separate explicit operation for changing identity. No rule infers phase from an id or requires the
two to agree.

**5. Importers assign a destination phase and never equate source hierarchy with it.** The five
names are `kep`, `checklist`, `roadmap-json`, `spec-kit`, and `openspec`. Each uses the shared
`Options::phase`, exposed as `import --phase` and defaulting to 1. In particular, a `roadmap-json`
phase is a source entity imported as a peer task with its source edges and provenance; it is not the
numeric phase of the destination model. An executable table is keyed to the flavor registry so a
new importer cannot omit this answer silently.

**6. The existing consumer contracts do not change.** Task YAML and `schemas/v1/task.json` keep a
required phase. The default summary keeps its phase rows and `aplan/summary/v2`. Tasks, show, and
MCP show keep the scalar. Checklist export continues to report phase as unrepresentable when work
spans several values, and roadmap-json export does not write an aplan phase into a foreign concept
with the same English name.

**7. There is no deprecation runway.** Nothing is being retired or made optional, so filing an
announcement/removal pair would make a false promise. A future removal requires a versioned schema
migration and evidence that the field's harm outweighs changing every task and consumer; it may not
arrive as an unannounced v1 omission.

## Consequences

- ADR-0014's phrase "only grouping the model knows" is retired. The precise invariant is one
  task-membership level, plus one built-in ordered reporting coordinate and declared tag-family
  views.
- Existing plans, import commands, ids, and versioned JSON documents keep their meaning and shape.
  There are no upgrade steps.
- Every task continues to pay for one explicit integer. That permanent cost is accepted because it
  already supports cross-cutting work, default rollups, stable consumer contracts, and established
  id conventions. Making it nullable would keep the surface while weakening the answer.
- A repository must not use phase as an epic vocabulary or execution gate. Use a declared family
  for the former and dependency/cut edges for the latter.
- A future task may revisit the decision, but it must propose the replacement for all contracts and
  all five importer mappings together. `aplan deprecations` is the required runway if that proposal
  schedules removal.

## Alternatives

| option | why not |
|---|---|
| Replace phase with a declared family | Requires a configuration declaration and string tag on every task, changes singular membership into policy-dependent cardinality, breaks four machine contracts, and leaves id minting needing another input |
| Make phase optional | Keeps the conceptual and implementation surface, adds `null` or hidden defaults everywhere, and makes the default summary less explicit |
| Derive phase from the cut | Erases the intentional independence, has no answer for cross-cutting work, and would make a legal cut spanning phases impossible |
| Retire it immediately | Breaks every v1 task file and consumer with no demonstrated correctness failure and no deprecation runway |
| Give phase execution semantics | Conflates a report coordinate with dependency truth; `blocked_by`, cut sequencing, and readiness already express executable order |
