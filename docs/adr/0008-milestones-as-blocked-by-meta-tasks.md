# ADR-0008: Milestones as blocked_by meta-tasks

- Status: Accepted
- Date: 2026-07-24
- Deciders: kaminsod

> **Correction (2026-07-25), after an adversarial audit of A1-014.** The decision stands,
> but this record originally overclaimed. A `done` milestone proves its *listed* members are
> done (`APLAN007`), not that the cut is *exhaustively* complete: membership is
> author-controlled and may omit or empty out. The "proof of completeness" language below is
> corrected to say exactly that, membership *totality* is named as a planned rule (since
> shipped as `APLAN022`, opt-in via `require_cut_membership`, `A1-018`), the
> milestone's own evidence is characterised honestly (L1, a declaration), and `APLAN021`
> (milestone hygiene) was added to keep the marker tag honest. See A1-017.
>
> **Correction (2026-07-25), superseded by ADR-0009 §4.** This ADR says the milestone body
> "carries the cut's narrative (theme and exit criteria)". ADR-0009 splits that: the cut's
> *rationale* lives in ROADMAP.md §3, and the milestone body carries only its *exit-criteria*
> (the duplicate `## Theme` is removed at the v0.5 cutover). The body is an exit-criteria
> home, not a narrative one.

## Context

The v0.5 self-hosting cutover (ROADMAP §6) requires the plan to *express* its own
roadmap: which release cut ("v0.3") a task belongs to, each cut's theme and exit
criteria, and whether the cut is met. Today that structure lives only in ROADMAP §10's
hand-maintained table, the exact drift the tool exists to end. `aplan roadmap` cannot
render a roadmap the plan cannot carry, so the model needs a representation of a *cut*.

The binding constraint is minimalism. RESEARCH §5's lesson from TaskJuggler is to borrow
the validity check and refuse the scheduler: over-modeling (resources, calendars, a
second entity type) is what sinks planning tools. `aplan-core` is a deliberate dependency
sink with three sanctioned extension points (EvidenceResolver, Format, Importer); a cut
is none of those, so it must be expressed within the existing task model, not bolted on.

Two facts already hold: a task carries free-form `tags`, and `blocked_by` edges are
validated (`APLAN003` rejects a blocker that does not exist, `APLAN006` a self-block).

## Decision

A **milestone is an ordinary task** tagged `milestone`, `blocked_by` the tasks in its cut,
carrying the cut's narrative (theme and exit criteria) in its markdown body. No new field,
no new edge type, no new entity. Membership is the `blocked_by` set.

This makes two things that a planning tool usually only asserts into machine-checked facts:

- **Cut membership is a checked claim.** A mistyped member id fails `aplan check`
  (`APLAN003`) the moment it is written, with no new rule. The generated roadmap therefore
  cannot silently omit a task.
- **A cut cannot be silently marked done over unfinished work.** A milestone is `done` only
  when every *listed* member is `done` (`APLAN007` forbids a `done` task with an undone
  blocker) *and* it carries evidence that `aplan verify` resolves (`APLAN005`, and
  `APLAN017`'s configured strength). This proves the *listed scope* is complete, strictly
  more than a tag-only cut, which cannot prove even that. It does *not* on its own prove the
  cut is *exhaustive*: membership is author-controlled, so a task that genuinely belongs to
  the cut but is omitted from `blocked_by` is not caught here. Making membership total is the
  author's responsibility until the planned totality rule (see Consequences) enforces it.
  Within that scope, this is ADR-0001's thesis, a status is a claim not a proof, applied to
  cuts.

A milestone is a checkpoint over work, not work, so the **work-views exclude it**, keyed on
`Task::is_milestone` (the one place the convention is defined):

- `aplan ready` never offers a checkpoint as workable;
- `aplan critical-path` never puts a zero-effort checkpoint on the plan's spine;
- `aplan summary` never counts it in a phase rollup;
- `APLAN009` (missing effort estimate) never asks it for one.

A milestone carries no `effort_days` and no `paths` (enforced by `APLAN021`): it is not
effort and owns no files. Its cut version is a second tag (`v0.3`) so `aplan roadmap` can
group and order cuts without parsing prose. A milestone's own evidence is the commit in
which the cut was declared complete; because it owns no files, that evidence stays at L1 (the
commit resolves, `APLAN011`), a dated declaration rather than a file-bound proof. The
completeness signal is structural (`APLAN007` over the listed members); `verify` resolves
each member's own evidence independently, it does not re-derive the milestone's completeness
from them.

## Consequences

- The cut to task mapping migrates from ROADMAP §10's table into the plan and becomes
  generable (`gen-roadmap --check`, v0.5), with referential integrity for free.
- Closing a cut reuses `aplan close` unchanged: it derives the commit and (with
  `detect_drift`) stamps the body. Authoring the milestones surfaced that §10's table had
  dropped `A1-012` from v0.4, precisely the omission generation prevents.
- The initial runtime delta was `Task::is_milestone` plus four exclusion sites, no schema
  change and no new crate; the A1-014 audit then added one rule (`APLAN021`) and made
  `critical-path` treat a milestone as a zero-weight passthrough (so a task depending on a
  whole cut is not severed from its work).
- Membership need not be total: a cross-cutting task (`A1-006`, the house-style gate)
  belongs to no cut and is a member of no milestone. Because nothing yet forces every work
  task into a cut, "the cut is done" proves only that the *listed* members are done; a
  planned advanced-integrity rule (membership totality, since shipped as `APLAN022`, `A1-018`,
  opt-in) would require every non-milestone task to be a member of exactly one cut, or tagged
  cross-cutting, closing that gap. It is
  deliberately not in this cut.
- `APLAN021` (milestone hygiene) keeps the marker honest: a task tagged `milestone` must
  have members and carry no effort or paths, so a real task mis-tagged `milestone` (silently
  dropped from every work-view) or an empty cut is warned, not swallowed. The `milestone`
  tag is itself an unvalidated tag, the very thing this ADR rejects for *membership*;
  `APLAN021` is the mitigation, and the `is_milestone` string match is exact and
  case-sensitive by design.

## Alternatives rejected

- **Tag-only membership** (each task tagged with its cut, e.g. `tags: [v0.3]`). Rejected:
  tags are unvalidated, so a typo (`v0.3` vs `v03`) silently drops a task from its cut and
  the generated roadmap lies, the exact failure mode `aplan` exists to catch. Membership
  must be a checked claim, and only `blocked_by` gives that today.
- **A new `milestone:` field on every task** (each task names its cut). Rejected: it is new
  model surface for a relation `blocked_by` already carries, against the minimalism
  constraint, and it inverts ownership (a cut should name its members, not be named by
  each of them from afar).
- **A first-class `milestones/` collection** with its own schema and loader. Rejected: it
  is the over-modeling RESEARCH §5 warns against, a second entity to load, validate, and
  thread through every command, and it forfeits the reuse (close, evidence, drift, verify,
  the DAG) that the meta-task approach gets for nothing.
- **The `phase:` field alone** (group by the existing 0/1 field). Rejected: `phase:` is the
  coarse legacy axis (0 = core, 1 = post-core), it carries no per-cut narrative and no
  checkable "is it met?" claim, and the cut is a finer slice than the phase (plan/README).

## References

- ROADMAP.md §3 (v0.5), §6 (the cutover), §10 (the cut to task table this replaces).
- the validator boundary: check plan validity without becoming a scheduler.
- ADR-0001 (status is a falsifiable claim): the ethos this applies to cuts.
- HLD.md §5.6 (drift fingerprints), §8 (CLI surface).
