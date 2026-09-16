# ADR-0015: A table row is an anchor, so drift is scoped to the fact

Date: 2026-08-11
Status: Accepted
Tasks: `A1-095`
Extends ADR-0009's `traces` model. Supersedes nothing.

## Context

`traces` names a document section, and `APLAN018` warns when that section changes
after a task was closed. The section runs from a heading to the next heading of
the same or a shallower level, so the unit of drift is the heading.

That is right for an ADR, where `## Decision` is one decision and a change to it
is a change to that decision. It is wrong for the shape a register naturally
takes.

The fourth pilot hit this doing routine work, which is the condition that matters
(pre-public adoption evidence, item 14). `constitution.md` is twelve invariants in one
table under one heading, and rows flip from `Planned` to `Live` as gates land.
`RD0-11` implements invariant 7 and traced the heading. Closing `RD0-15`, which
implements invariant **11**, edited a different row and re-flagged `RD0-11`:

```
warning[APLAN018] RD0-11: traced section `constitution.md#p00020-constitution`
  has changed since this task was closed
```

Roughly ten more such edits are scheduled. Every one re-flags every task tracing
that table, and the answer is `--restamp` every time, because it genuinely is.
After the third, an adopter stops reading the warning.

**The cost is to the whole rule, not to one document.** A warning that is always
correct to dismiss trains dismissal of the firings that are real, and it degrades
slowly enough that nobody notices the mechanism stopped working. The two
workarounds available were to stop tracing the file, which drops a real signal, or
to accept the noise, which destroys it.

Splitting the table into twelve `###` sections would give twelve anchors and make
the document worse to serve the tool. A plan format that deforms the documents it
points at has the relationship backwards.

## Decision

**An anchor may name a table row, addressed by its own first cell, and a
row-scoped trace fingerprints that row alone.**

```yaml
traces:
  - constitution.md#invariant-7
```

No new field, no new grammar, and no change to `trace_fingerprints`, which is
keyed by the trace string and simply sees a different key. Row anchors are minted
with the heading slug rule, so `| Invariant 7 | ... |` is `#invariant-7`.

Three details decide the rest:

1. **Headings resolve first.** A document with both a `## Decision` and a
   `| Decision |` row means the heading by `#decision`. Every existing plan
   therefore fingerprints exactly what it did before, byte for byte.
2. **Rows are minted on their own counter.** Sharing the headings' disambiguation
   counter would move a row's anchor when an unrelated heading was added, which is
   the positional brittleness this decision exists to avoid.
3. **The header row is not an anchor.** Markdown says precisely which row it is,
   the one the delimiter follows, and it names the columns rather than a fact.
   Anchoring it would offer `#invariant` for a table of invariants, which is a
   trap rather than a feature.

## Consequences

**Opt-in by construction.** A plan that names only headings is unchanged: same
anchors, same fingerprints, same warnings. Nothing needs migrating.

**`APLAN008` widens with it.** A row-scoped trace would otherwise be reported
dangling on the day it was written, so anchor resolution now covers headings and
rows, and the message says "matches no heading or table row" rather than naming
headings alone.

**What this gives up.** A row's anchor follows its first cell, so *renaming* the
first cell breaks the trace, which `APLAN008` then reports as dangling rather than
as drift. That is the right failure: renaming the row's key is a change to what
the row is, and being told the trace no longer resolves is more useful than being
told its content moved. It is the price of content addressing, and the
alternatives are worse.

**Rows only, for now.** A list item is the same problem in a different shape and
is not covered. Nobody has hit it; when someone does, the mechanism generalises
the same way.

## Alternatives

| Option | Why not |
|---|---|
| `#anchor:7`, a row index | Positional. Inserting a row above shifts every one below, which is the brittleness the pilot named and asked us to skip |
| A line range | The same defect, more general and more brittle; the pilot called it the option to skip and was right |
| A separator, `#anchor\|row-key` | A grammar change and a schema change for what the anchor namespace already expresses. GitHub links neither form, so it buys no compatibility either |
| Report *what* changed instead | Valuable and not an alternative: `trace_fingerprints` stores a hash, not the content, so there is nothing to diff. `A1-096` shipped the version that costs no storage, naming the commit to diff from |
| Split the document into more headings | Deforms the document to suit the tool |
