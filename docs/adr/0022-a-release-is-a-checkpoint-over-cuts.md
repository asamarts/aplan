# ADR-0022: A release is a checkpoint over cuts, and says so

Date: 2026-08-17
Status: Accepted
Tasks: `A1-146`
Extends ADR-0008 (a milestone is an ordinary task) and ADR-0013 (a milestone naming a milestone is
sequencing). Supersedes nothing.

## Context

The sixth pilot has eight cuts and three delivery points: `v0.1` at week 46, `v0.5` at 72, `v1.0`
at 85. Each release closes a cut, and the release is what the project is judged on -- a cut is
internal, a release is what a user sees. The obvious encoding, a second milestone whose `blocked_by`
names the *tasks* in the release, is refused, correctly, by `APLAN022`: a task in two milestones is
a task in two cuts.

So a plan could express when work is grouped and not when it ships (pre-public adoption evidence item 1),
and every workaround was lossy: a `release:` tag is checkable vocabulary but not a node in the
graph, so `ready`, `why` and `critical-path` cannot see it; making the release the cut collapses
eight cuts into three; `cross-cutting` is an escape hatch, not a second axis.

**The encoding that needs no new model was measured before anything was built**: a release as a
milestone whose `blocked_by` names the *cuts* it closes. On a plan of that shape:

- `check` accepts it. `APLAN022` counts task membership, and a release claims no tasks.
- `ready` is unaffected: a release is a milestone, and every work view excludes milestones.
- `why R-v0.1` answers with the cuts and, transitively, their work -- which is the question.
- `APLAN007` refuses to call the release done while a cut it names is open, which is the guarantee
  an adopter wants from it.
- `roadmap` rendered it `0/0 done`: true of the milestone's own `blocked_by`, useless as an answer.
- `APLAN021` reported it as a cut with no work in it (`A1-134`).

The last two are the finding. **The state a release produces is identical to a cut emptied of its
work**, because ADR-0013 already gave the milestone-to-milestone edge a meaning: "this cut follows
that one". A release naming its cuts and an emptied cut naming its predecessor are the same graph.
The difference is intent, and intent has to be declared.

## Decision

**1. A release is a milestone carrying the `release` tag.** It is a checkpoint over cuts: no
effort, no paths, excluded from every work view, exactly as a cut is. This is a narrowing of an
existing kind rather than a new one, which is why it needs no new field (ADR-0008's "no new field
or edge type" holds), and the second grouping level stays in tags where ADR-0014 put it.

**2. Its members are cuts.** A release holding work directly is reported (`APLAN021`), because a
task in a release and a cut is a task in two cuts -- the thing `APLAN022` refuses from the other
end. The rule names the release rather than each task caught by it.

**3. `APLAN021` exempts a release from the empty-cut test.** Without the tag the two shapes are
indistinguishable, and warning on every release would be the rule being wrong about the encoding
this ADR endorses.

**4. Its rollup is the work of the cuts it closes**, read one level through. A release of releases
is not a shape this model has, and recursion would double-count. The release *borrows* that work:
the cut still owns it, so a task appears in its cut and in the release, and a consumer summing every
cut's total skips releases (`Cut::release` says which).

**5. Nothing about scheduling.** A release says which cuts it closes, not when they land. Dates
remain out of the model (`A1-147` decides the arithmetic question separately).

## Consequences

- An adopter with releases writes one tag and gets `ready`, `why`, `critical-path`, `APLAN007` and
  a truthful rollup, with no new field to learn and no migration for a plan that has none.
- `APLAN021` reads two tags now (`milestone`, `release`), which is more structure in tags than this
  project had before. The alternative was a field, and ADR-0014 already decided that direction.
- A plan that puts work directly in a release gets two findings for one mistake (`APLAN021` on the
  release, `APLAN022` on each task). That is deliberate: they are different readers -- one is fixing
  the release, the other is fixing a task -- and both point at the same remedy.
- If releases ever need ordering among themselves, ADR-0013's sequencing already applies unchanged,
  since a release is a milestone.
