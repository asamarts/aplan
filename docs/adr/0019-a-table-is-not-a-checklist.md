# ADR-0019: A table is not a checklist, and there is no table importer

Date: 2026-08-14
Status: Accepted
Tasks: `A1-129`
Extends ADR-0012's import contract. Supersedes nothing.

## Context

The fifth pilot is a greenfield programme plan: 1,887 lines of Markdown with **45
tables and 451 data rows, and zero `- [ ]` rows**. Pointed at the only Markdown
importer it produced three words and an exit code:

```console
$ aplan import PLAN.md --format checklist --dry-run
aplan: PLAN.md: no importable proposal found
$ echo $?
2
```

The refusal was right and said nothing. It did not name what it read, what it
wanted, or what would work instead, and it offered no measurement of the document
it declined (pre-public adoption evidence).

The question that produced this ADR is whether the answer is a table importer.

## Decision

**There is no generic table importer, and this is a boundary rather than a
backlog item.**

**1. A checkbox has one meaning; a table row has none.** `- [ ]` is a proposal
wherever it appears, which is why `--format checklist` can read a document it has
never seen and be right about it. The fifth pilot's 45 tables carry streams,
gates, risks, deliverables, metrics, budgets, role assignments and comparisons.
Perhaps two of them are work. Only the author knows which, and a rule that
guessed would be recording a guess as a fact, which is what ADR-0012 clause 3
already forbids for a single dropped field.

**2. An interface that "never guesses" is a schema language, and it belongs
outside.** To import without guessing, an adopter would have to declare, per
table, that it is work and which column is the title. That declaration is a
mapping DSL living in `.aplan.yml`: a second place where a plan's meaning is
written down, competing with the plan. It would also be a worse `jq`, since the
real cases are "the third column of the table under this heading, unless the row
is a total".

**3. The extraction is the consumer's, and it is documented and tested.**
`docs/importing-proposals.md` carries a three-pass `awk` recipe, and this
repository runs that exact pipeline in its own suite, so the published commands
are ones that have been executed. Its first draft named the wrong column and swept
three tables into one import, which is what an untested recipe does.

**4. What `aplan` owes is a refusal that teaches.** It now names the shapes it
reads, measures what the document has instead (`45 table(s) with 451 data
row(s)`), and points at the recipe. An empty file says it is empty rather than
reporting the tables it does not have.

## Consequences

- An adopter whose plan is tables writes an extraction once, in their own
  repository, next to the document it reads. The mapping is reviewable there.
- Whatever the extraction produces goes through the checklist importer unchanged,
  so a ticked row is still a claim, generated ids are still reported, and `check`
  still judges the result. The gate does not move.
- The cost is real and stated: the extraction is one more thing to maintain if the
  source is restructured. It is smaller than owning a mapping language.
- If a source format arrives whose rows *do* have one meaning, that is a new
  flavor under ADR-0012's existing rules, not a general table importer, and this
  ADR does not stand in its way.
- If enough pilots write near-identical extractions, that is evidence this
  decision was wrong, and the way to find out is to let them write them.
