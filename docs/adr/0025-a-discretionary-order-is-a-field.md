# ADR-0025: A discretionary order is a field, and it never blocks

Date: 2026-08-17
Status: Accepted
Tasks: `A1-150`
**Supersedes ADR-0017.** Its derivation stands unchanged; this adds the field it deferred, and
carries the instance it said a successor would have to carry.

> **Filed as `ADR-0020`, renumbered to `ADR-0025` on 2026-08-31 by `A1-222`.** Two files were
> filed as `ADR-0020`: this one, and `ADR-0020: An outcome is superseded, never reopened`, dated
> 2026-08-14. That one held the number first, so this one moved. `0025` keeps the sequence in date
> order, since `ADR-0021` through `ADR-0024` share this record's date. A citation reading
> "ADR-0020" in a commit older than 2026-08-31 means whichever of the two its sentence describes;
> after that date it means the supersede decision only. The decision below is unchanged.

## Context

ADR-0017 decided that ordering is derived from the graph rather than declared on a task, and
deferred a discretionary-order field rather than rejecting it:

> A `prefer_after:` field that ranks without blocking is a coherent design and may prove necessary.
> It is deferred because a derived order plus editable dependencies may make it unnecessary, and
> the way to find out is use rather than argument. The decision point is the dogfooding period in
> the pre-public repository-boundary decision: if at the end of it there is a recorded instance of wanting "X before Y" that neither a
> dependency nor the derived order could express, that instance is the evidence an ADR for the
> field would need. If there is none, the field was never needed.

the pre-public repository-boundary decision asks the same question as the second of the three its period must answer.

**The instance arrived.** A pass over six pilot logs (2026-08-17) produced ten open findings and a
maintainer's order over them:

1. `A1-143`, the p0000007 fleet's only candidate fix: a `../sibling` traces target is an undeclared
   hard dependency, so a plan passes on one machine and fails on another.
2. `A1-145`, the strongest remaining hole in the evidence ladder: a `commit` citation that resolves
   and is still the wrong commit.
3. `A1-134`, `A1-139`, `A1-144`, `A1-135`: the fourth pilot's remaining friction, small and
   unrelated to each other.
4. `A1-146`, `A1-147`, `A1-148`: the sixth pilot's three decisions, taken together because each
   asks whether something is in scope.
5. `A1-149`: the third pilot's documentation finding.

Neither mechanism expresses it. **Not a dependency**: `A1-145` does not need `A1-143`, and the
p00013 decisions do not need the p00020 fixes; writing those edges would corrupt `ready` (it would
stop offering the work), `critical-path` (it would lengthen a chain that does not exist) and
`APLAN007`, which is precisely what ADR-0017 measured. **Not the derived order**: rank reads what a
task releases, then the critical path, then effort, and these ten are unrelated leaves of
comparable size, so what it produces is effort-then-id -- an accident of estimation rather than a
judgement about what matters next.

## Decision

**1. `prefer_after:` is a task field**, a list of ids this task prefers to follow.

**2. It never blocks.** A task whose preference is unmet is ready, offered, and offered *later*.
This is the line between mandatory and discretionary logic, and it is the whole reason the field
can exist without weakening the model: nothing that decides whether work may start reads it.

**3. Exactly one consumer.** `ready` honours it, as a stable topological pass over the derived
order: a preference moves a task after the offered tasks it names and changes nothing else, so two
tasks with no preference between them keep the order rank gave them. `critical-path`, `why`,
`APLAN007` and every evidence rule do not read the field at all. A reader asking "why can I not
start this" must never be shown an opinion.

**4. A preference that cannot be honoured is a finding, not a silent no-op** (`APLAN033`, error):
an unknown id, the task itself, a pair that is already a `blocked_by` edge (the plan would say two
different things about one pair), or a cycle (which has no order at all). The severity is the same
as a dependency defect because none of these is a preference the tool declined to follow: each is a
preference that says nothing.

**5. Editable through the tool** (`aplan dep add --prefer`), on `A1-124`'s reasoning: a model whose
ordering mechanism needs a text editor is unmaintainable, and that argument does not weaken because
the edge is discretionary.

## Consequences

- `ready`'s order can now be changed by a declaration, which is a departure from ADR-0017's "every
  input is already checkable". The mitigation is that the declaration is *visible*: `--explain`
  names the preference beside the derived reasons, so a reader can tell a judgement from a
  derivation rather than wondering why the order looks wrong.
- A plan that uses no preference ranks exactly as it did, which is what keeps this from being a
  migration.
- The field is a new way to be wrong -- an order that reflects a decision nobody remembers -- and
  the answer is the same as for every other claim here: it is checked where it can be
  (`APLAN033`), and it is printed where it is acted on.
- If preferences start standing in for dependencies, the symptom will be `APLAN033`'s
  already-a-dependency case firing, and that is the signal to revisit this rather than to relax it.
