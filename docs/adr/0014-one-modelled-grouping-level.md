# ADR-0014: One modelled grouping level, and the second is a declared tag family

Date: 2026-08-08
Status: Accepted
Tasks: `A1-032`
Supersedes nothing. Extends ADR-0008 (milestones as cuts) and ADR-0012 (a stage is a tag).

## Context

`aplan` models one grouping level: a milestone meta-task `blocked_by` the tasks
in its cut (ADR-0008). Two adopted projects nest one level deeper.

**`p0000019`** nests goal, phase, workstream, milestone. Its real
`roadmap.json`, imported today, is 21 phases and one workstream carrying 25
milestones, 47 tasks in total.

**`p00020`** nests goal, cut, epic: 98 tasks across 7 cuts and 22 epics
(pre-public adoption evidence, item 4).

`A1-032` deferred this decision until the second project arrived and the real
data could be read rather than described. Both conditions are now met, and two
things have changed since the question was framed.

## What the data actually shows

**`p0000019`'s nesting is nominal.** Its 21 phases carry **no items at all**;
each is an id, a title, a status and a blurb. The single workstream holds every
one of the 25 milestones. So the tree is wide and shallow, and flattening it
loses a label, not a structure. The distinction that project uses deliberately is
not the nesting: it is the five state fields on the workstream
(`artifact_status`, `authority`, `execution_status`,
`service_restoration_status`, `status_validity`), and **no amount of nesting
would carry those** because ADR-0012 already decided a stage is a tag and never a
status.

**`p00020`'s nesting is real**, and it is where the cost was observed rather than
predicted: 22 epics as `tags: [epic-a]`..`[epic-v]`, one validated grouping level
and one unvalidated one in the same frontmatter.

**Two of the four things a second level was wanted for have since shipped.**

| What a second level was for | Now |
|---|---|
| A typo must not silently drop a task from its group | `APLAN026`, a declared tag family (`A1-076`) |
| Roll effort up by group | `aplan tasks`, one `jq` line (`A1-075`) |
| Render tasks grouped, not as a flat block | **still missing** |
| Membership totality within the group | **still missing**, and unasked for |

**And the obvious mechanism is spoken for.** A milestone listing another
milestone in `blocked_by` used to mean nothing; ADR-0013 gave it "this cut
follows that one". Nesting through the same edge would now be ambiguous with
sequencing, so modelling a second level means genuinely new surface, not a reuse.

## Decision

**One modelled grouping level stands. A second level is a declared tag family.**

Concretely:

1. The cut, a milestone meta-task, remains the only grouping the model knows.
2. A project's second level is expressed as tags, declared in `tag_families`
   (`APLAN026`), which makes the vocabulary checkable without making it a model.
3. A stage, a lifecycle, or a per-group state is a tag and never a status, which
   restates ADR-0012 for hand-authored tasks so every project does not invent it
   again. `p0000019`'s `artifact_status: accepted` is the tag
   `artifact:accepted`, not a `Status` variant and not a second grouping level.
4. The rendering gap is real and is work, not a reason to change the model:
   `A1-078` teaches `roadmap` to group a cut's members by a declared family.

## Consequences

**What this gives up, stated plainly.** A tag family is a vocabulary, not a
containment: nothing enforces that every task belongs to exactly one epic the way
`APLAN022` enforces exactly one cut. A project that needs that totality does not
have it, and `A1-076`'s near-miss check is a weaker guarantee than membership.
Nobody has asked for it yet. If a project does, that is the moment to revisit
this, and the evidence to bring is a plan where an unassigned task caused real
harm.

**Why not model it anyway.** ADR-0008's minimalism is not frugality for its own
sake: every field is one more thing an importer must map, a consumer must
understand, and a schema must carry forever. The two projects that motivated the
question need rendering and validation, and both are reachable without it. Adding
a level to serve a label that `p0000019` does not populate and `p00020` can
declare would be paying the permanent cost for the temporary want.

**A divergence found while deciding, and tracked.** The `roadmap.json` importer
does not merely flatten the nesting, it **discards it**: a phase becomes a peer
task (`PHASE-00`), and a milestone imported from a workstream records nothing
about the workstream it came from. Under this decision it should emit the
container as a tag, so the information survives the import and the family can be
declared. `A1-079` carries that.

**Existing plans are unaffected.** No task file changes meaning, no schema field
is added, and a project that never declares a family behaves exactly as it does
today.

## Alternatives

| Option | Why not |
|---|---|
| A `group:` field on `Task` | New model surface, forever, for a level one project leaves empty and the other can declare |
| Nested milestones (a cut inside a cut) | ADR-0013 gave milestone-to-milestone `blocked_by` a different meaning; reusing it would be ambiguous |
| A second `Status` axis for lifecycle | ADR-0012 settled this: a stage is a tag, because a status carries a verdict and a stage does not |
| Do nothing at all | Leaves the rendering gap, which is the one real cost the pilots reported |
