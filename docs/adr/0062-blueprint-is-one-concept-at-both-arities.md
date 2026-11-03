# ADR-0062: Blueprint is one concept at both arities

Date: 2026-10-30
Status: Accepted
Tasks: `A1-384`

## Context

`A1-383` applied two one-task candidate files and one three-task candidate with the existing
`add --from` path. The one-task files preserved useful scaffolds but already worked as copied
manifests. The three-task file exposed the material gap: each application required three free ids
and two matching dependency edits. Declared parameters and local keys can remove that repeated
instance and topology work while still producing ordinary tasks.

The feature needs one name before its schema and command become public contracts. Two adjacent
names are already occupied:

- `template` names task-body scaffolding in `aplan add` and filename patterns in Backlog.md;
- `bundle` names a versioned, installable package of extensions, presets, workflows and steps in
  spec-kit. Aplan already has a spec-kit adapter, so one adopter can encounter both products.

Using either word would make one term carry incompatible meanings. Calling the single-task and
multi-task cases different things would create a second problem: both consume the same fields,
bindings, validation and atomic application path. Their only difference is how many entries the
file contains.

The spike also found no need for source discovery, inclusion, persistent provenance, replay or a
distribution system. Those mechanisms would add lifecycle and trust promises that an explicit,
reviewed local file did not need.

## Decision

**1. The public term is `blueprint`.** A blueprint is one constrained plan-authoring document
containing one or more task entries. A one-entry blueprint and a many-entry blueprint are two
arities of the same concept, not a task template and a task bundle. They share one schema and one
application operation.

**2. A blueprint compiles to ordinary plan changes.** It may bind declared string parameters,
mint task ids and resolve symbolic relationships before proposing a post-state through the
existing atomic mutation gate. It creates no blueprint task type, status, evidence kind or
continuing controller. The resulting tasks must remain understandable after the input file is
gone.

**3. V1 applies one explicit local file as a one-shot operation.** Each invocation selects one
file directly. The file is the inspection surface; application parses and validates that exact
input, then either publishes the complete accepted post-state or no plan bytes. There is no source
discovery, logical-name lookup, precedence, replay, update, drift reconciliation or application
history.

**4. The absent source mechanisms are deliberate.** V1 has no composition or includes, runtime
built-ins, catalog, installation, fetch, remote source or network access. Sharing means reviewing
and copying a file, then selecting that local file explicitly. Repository examples may
demonstrate the language, but they are conformance material rather than runtime methodologies.
Any later discovery or distribution proposal needs new adopter evidence and a complete trust and
lifecycle decision.

**5. A blueprint is not a workflow or a methodology.** It does not execute work, advance status,
prescribe a planning system or prove any generated task. It is also not a substitute for a plan
authored from a real document: an author must still read the requirement, decision, incident or
other source, decide what work and evidence it implies, and review the proposed plan. A blueprint
can preserve recurring structure only after those judgments exist.

**6. Source identity is an operation result, not task provenance.** Dry-run and successful apply
output identify the selected path and content digest so a reviewer can bind the proposal or
result to the input bytes. That identity and digest belong with the resulting change in ordinary
Git history, for example by committing the reviewed source or recording the apply result. V1 adds
no task-model provenance field, author-controlled recipe revision or replay metadata. If a future
operation promises update or drift detection, that new lifecycle must first decide canonical
source identity, revision, bound parameters and migration semantics.

The derived DSL and exact parameter/reference grammar belong to `A1-385` through `A1-387`; the
explicit-file command and renderings belong to `A1-389`. This decision fixes their product and
lifecycle boundary without choosing syntax they own.

## Consequences

- One schema must accept one or many entries without forking into two dialects.
- One-entry blueprints remain useful examples, but the measured topology savings of a multi-entry
  application are the warrant for the feature.
- Preview and apply output must carry the same selected path and digest; neither output implies
  that predicted ids are reserved.
- Generated tasks carry only ordinary task facts and earn status and evidence in the ordinary
  way. Blueprint origin is not evidence for their claims.
- `A1-388`, `A1-390`, `A1-391` and `A1-394` remain dropped. Their discovery, composition,
  persistent-provenance and distribution scopes do not return by implication.

## Alternatives

| option | why rejected |
|---|---|
| `template` | It already names task-body and filename scaffolds, and says nothing about a typed local graph. |
| `bundle` | It collides with spec-kit's installed tooling package and falsely implies versioning, catalogs and distribution. |
| `recipe` | It suggests procedural execution; the input is constrained data compiled into an ordinary plan mutation. |
| Separate names for one task and a task set | The entries have identical semantics, so two nouns would manufacture a distinction and invite schema and command drift. |
| Persistent origin on every task | One-shot output is self-contained; a model field would impose a replay contract and schema cost without a consumer. |
| Discovery, catalogs or built-ins in V1 | The spike saved no authorship work with lookup, and runtime content would introduce precedence and methodology policy. |
| Includes or composition in V1 | None of the materialised candidates reused another fragment; inclusion would add a language before a demonstrated case. |
