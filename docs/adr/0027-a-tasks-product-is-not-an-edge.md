# ADR-0027: A task's product is not an edge

Date: 2026-09-02
Status: Accepted
Tasks: `A1-248`
Extends ADR-0020 (an outcome is superseded, never reopened). Supersedes nothing.

> **Corrected on 2026-09-02 by `A1-248`'s own audit. The decision is unchanged.** Point 2 said no
> field could close the gap. That is wrong: a field plus a rule refuses a silence exactly as an
> acceptance criterion does, and `APLAN030` already reads a per-tag requirement out of `.aplan.yml`,
> so the machinery would be cheaper than the point implied. The argument is cost and generality
> rather than impossibility, and point 2 is rewritten to say so. Point 3 quoted `A1-224`'s
> measurement of 188 closed tasks as though it were current, and is re-measured. Point 4 is new: the
> bar for adding a field was already set by ADR-0017 and cleared by ADR-0025, and an ADR arguing
> against a field owed that standard an answer. The last consequence claimed more than
> pre-public adoption evidence records about which of its tasks are closed.

## Context

The ninth pilot is the first whose plan contains research. `p00002` has five tasks whose product
is *more tasks*: a spike asking what the corpus still cannot express, a review round whose findings
become probes, an evaluation that files what it could not explain. Every one ends its acceptance
criteria with some wording of "any follow-up work is filed as tasks before this closes"
(pre-public adoption evidence, item 5).

Nothing checks that. A spike can close, honestly, with a real commit citation, having produced
nothing, and the plan cannot distinguish a spike that found nothing from one whose findings were
never written down. That is `done` with no evidence one level up: an assertion about work that
nothing falsifies, which is the shape this tool exists to refuse.

The pilot filed it as a question rather than a request, because the answer is not obvious.
`blocked_by` points the wrong way, `traces` points at documents rather than tasks, and a `produces:`
list would be a claim nothing could check at the moment it matters, before the follow-ups exist.

## Decision

**A task's product is not a new edge. What a spike owes is an acceptance criterion, like every other
statement of what done means.**

**1. The same question was answered for the closest analogue.** `supersede` files a successor for a
task whose outcome stopped being enough, which is a finished task producing a new one: the same
shape. ADR-0020 decided the link "lives in the successor's body, not in a new field. It is prose
because nothing checks it, and a field nothing checks is a claim this tool would have to distrust."
That argument is stronger here, not weaker. `supersede` knows the pair at the moment it creates it
and still declined to record it; a `produces:` list is written by hand, after the fact, and can name
any task that already exists. What is unchecked there is the link's *content*. Whether a rule could
check that something was said at all is a different question, and it is the next point.

**2. A field could refuse the silence, and that is the only part of the gap anything can close.**
The distinction that matters -- a spike that found nothing, against a spike whose findings were
never written down -- is not visible to anything the plan or the repository records. `produces: none`
and a ticked box are equally unfalsifiable, so neither form checks the claim, and no design
available here does.

What either form can do is refuse the *silence*. A `produces:` key, plus a rule requiring it of
tasks carrying a declared tag, would report an investigation that closed saying nothing at all, and
the machinery for that is not hypothetical: `APLAN030` already reads a tag family's cardinality
requirement out of `.aplan.yml` and refuses a task that does not carry one.

So the choice is between two mechanisms buying the same thing, and it is settled on cost and
generality rather than on capability. The field costs a frontmatter key, a config key naming which
tags require it, a rule, a schema entry, and a carrying cost in every importer and exporter that has
to decide what to do with it (ADR-0012). The criterion costs nothing new: `aplan add` already
scaffolds `- [ ] state what done means for this task, in this repository's terms`, which is where
the sentence belongs and the one place every adopter meets it.

**3. It is the general case, and a field here would fix it for one tag.** "Any follow-up is filed
before this closes" is a statement of what done means, written where this tool already asks for one.
It goes unenforced because `close` reads no acceptance criteria at all. Re-measured over this plan
on 2026-09-02: of 213 closed tasks, 196 are fully ticked, 7 have none ticked, 10 carry no criteria
section, and **none is partially ticked**. That zero is the informative number, and it is the same
zero `A1-224` found at 188 closed tasks: nobody is making a per-criterion judgement and leaving one
open, so a gate here would catch a forgotten step rather than override a decision. A rule reading
checkboxes only for tasks carrying an investigation tag would solve that general defect for one tag
and leave every other criterion exactly as unread.

**4. The bar a new field has to clear is already written down, and this one does not clear it.**
ADR-0017 deferred `prefer_after` rather than rejecting it, and said what would earn it: a "recorded
instance of wanting `X before Y` that neither a dependency nor the derived order could express".
ADR-0025 added the field when that instance arrived. What made it a field rather than a note is that
it **changes an answer**: `ready` orders by it, so a build that ignored it answers the tool's central
question differently. `produces:` has the recorded instance, in p00002's five tasks, and fails the
other half. Nothing would order, block, offer or refuse differently for reading it, beyond the
refusal an acceptance criterion already carries. If that ever stops being true, the ADR proposing
the field has this bar to meet, and meeting it is what would make it worth its cost.

**5. Naming the shape of the work needs nothing new.** ADR-0014 settled that one modelled grouping
level plus a declared tag family is enough for a second, and `p00002` is the case it was arguing
about: its `kind` family, `require: exactly-one`, carries a nine-stage loop including
`kind-research`, `kind-design` and `kind-review`. A plan can already say "this is a spike" in a
vocabulary the tool validates.

**6. The body is where a link like this belongs.** The frontmatter is data and the body is prose that
is never parsed (HLD section 4). "This investigation produced these tasks" is real and worth writing
down, and writing it in the follow-up's body is what ADR-0020 already asks of a successor.

## How a plan expresses it

A task whose product is more tasks says so in its own acceptance criteria, in the same form as any
other criterion, and the follow-ups name the investigation that produced them in their bodies:

```markdown
## Acceptance criteria

- [ ] the corpus cases that `data` blocks still cannot express are listed
- [ ] each one is filed as a task, or recorded here as deliberately not worth filing
```

The second line is the one that matters, and the reason it is written as a disjunction is point 2:
"found nothing" is a legitimate outcome, and a criterion that admits only one answer teaches an
author to tick it regardless.

## Consequences

- Nothing new in the schema, the importers, the exporters or the MCP surface.
- The enforcement arrives with `A1-224`, when `close` reads acceptance criteria at all. Until then
  this convention is maintained by hand, which is what it is today and what the 180 ticked tasks
  above were maintained by.
- `aplan` says nothing about spikes specifically. A repository that wants the shape named uses a
  declared tag family, per ADR-0014.
- **What an adopter meets.** The generated `AGENTS.md` block does not mention acceptance criteria at
  all, so the only place the tool states the form is `aplan add`'s scaffold line. That is enough for
  the convention, since the sentence goes where the tool already asks for one, and it is not enough
  for the enforcement. Documenting the enforcement belongs with `A1-224`, which is what makes
  criteria load-bearing in the first place; writing it before then would describe a gate that does
  not run.
- **What would overturn this.** A pilot log showing investigations closed with the criterion ticked
  and no follow-ups filed, in a repository where investigations are a large share of the plan. That
  is a checkable claim about a real adoption rather than an intuition about one, and it is the
  evidence this decision should be revisited on. There is no such log today, and `p00002` cannot yet
  be one: it is a day old, two of its 64 tasks are closed, and the nine that are `kind-research` or
  `kind-design` are not among them (pre-public adoption evidence).
