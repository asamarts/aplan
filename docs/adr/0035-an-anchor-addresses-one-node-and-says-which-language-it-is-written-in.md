# ADR-0035: An anchor addresses one node, and says which language it is written in

Date: 2026-09-17
Status: Accepted
Tasks: `A1-191`
Extends ADR-0015 (a `traces` anchor may name a table row as well as a heading) to
documents that are not prose. Constrained by ADR-0026 (this tool owes
reachability, not a query language). Supersedes nothing.

## Context

`traces` binds a task to the thing it answers to, and `APLAN018` re-opens a
closed task when that thing changes. Until now the thing had to be a markdown
heading or table row, so what a repository could put under drift detection was
decided by what it happened to write in prose.

The sixth pilot's authoritative artefacts are YAML: an error catalogue that
generates three languages, a state machine, a pricing table, a threat manifest of
122 records. A task implementing a state transition could not trace the state. The
available workaround was to anchor the *prose that narrates* the manifest, which
is a second copy of the fact, in a tool whose central argument is that second
copies drift (pre-public adoption evidence, item 2).

**The demand is real and narrower than it first looked.** That same pilot
corrected itself after prototyping: 0 of its 150 migrated tasks needed a YAML
anchor, because the wave table is markdown and ADR-0015 already resolves a table
row by its first cell. What still wants one is the spec catalogues and the threat
manifest. The log then says plainly that a pilot cannot say how many adopters
have machine-readable sources of truth at all.

**The corpus cannot answer the question, and it is worth saying why.** Across
thirteen plans on this host, all 6,270 declared traces are markdown and all are
anchored. That measures what `APLAN008` accepts, not what anybody wanted: a tool
that refuses a kind of target collects no evidence of demand for it. The pilot
logs are the only uncensored signal here, and one pilot with a narrow, self-
corrected want is what they contain.

We build it anyway, and the reason is a difference in kind rather than degree.
An unmeasured *rule* fires on plans that already exist and costs every adopter
who did nothing wrong. An unmeasured *capability* is opt-in: a repository that
traces no YAML cannot be affected by YAML anchoring. The risk is maintenance
surface, which is ours, rather than false positives, which are theirs.

## Decision

### Two spellings, each self-identifying

An anchor into a `.yaml`, `.yml` or `.json` document is either

- a **JSON Pointer** (RFC 6901) when it starts with `/`: `spec.yaml#/states/provisioning`
- a **JSONPath** (RFC 9535) when it starts with `$`: `spec.yaml#$.states.provisioning`

and anything else is refused, naming both forms. Requiring the marker is what
lets a reader tell the two apart at a glance, and `$` is required by RFC 9535
anyway. The two compile to the same list of steps, so there is one resolver, one
fingerprint and one near miss rather than two of each that must be kept agreeing.

**Which resolver runs is decided by the file's extension, not by the anchor.**
A `.md` file has headings; a `.yaml` file has nodes. Nothing about `#foo` has to
be ambiguous, because the document it points into already settled the question.

### Only JSONPath's singular subset

RFC 9535 defines a **singular query** (section 2.3.5.1) as one syntactically
restricted so that it yields at most one node whatever the input. That is exactly
what an anchor must be: a fingerprint is taken *of a node*, and an expression
that can address three nodes has no fingerprint.

So name selectors and whole-number index selectors are accepted, and wildcards,
descendant segments, slices, unions and filters are refused **by name**, each
with the reason it cannot be an address. "Invalid anchor" teaches nobody which
half of their expression was the problem.

This is also the line ADR-0026 already drew. Accepting the rest of JSONPath would
be adding a query language through a side door, one rule at a time, in the tool
that refused one on purpose.

**A negative index is refused too**, though RFC 9535 allows it in a singular
query. `$.items[-1]` addresses a different element whenever the array grows, and
a stored anchor has to keep meaning what it meant. RFC 9535's own Normalized
Paths exclude negative indices for the same reason.

### The fingerprint is of the canonical subtree

The addressed node is re-serialised canonically -- keys sorted, no whitespace,
comments and layout absent because they are not part of the value -- and that is
hashed.

So reindenting a file, reflowing a comment, or reordering two keys is **not**
drift, while changing a value, adding a key or removing one **is**. This is a
better answer than markdown gets, and it is the point of giving a structured
format its own resolver instead of reading it as text: hashing the source bytes
would re-open every closed task that traced into a file somebody ran a formatter
over, which is precisely the false positive that teaches a repository to switch
drift off.

### A document this tool cannot fully read is refused whole

YAML that JSON cannot represent -- a non-string key, an alias to a cycle -- does
not parse into the shared value model, and the trace is refused with that as the
reason. Addressing part of a document we could only partly read would make the
fingerprint mean something the author did not choose.

## What this does not do

**Source symbols.** `src/lib.rs#important` is the most valuable anchor nobody can
have: it needs a parser per language, which is the runner-specific work this tool
declines everywhere else, and it has no measured demand at all -- it appears in
`A1-191`'s own synthetic table and in no pilot log. Declined, and recorded here
so the next person starts from the boundary.

**Line ranges.** The cheap alternative to a symbol, and honest about being
fragile: every insertion above the range silently re-points it. An anchor whose
meaning changes when an unrelated line is added is not an address. Declined.

**The root is addressable, so a structured document can be watched whole.**
`spec.yaml#$` is a legitimate address and stamps like any other node, which is
the answer for a manifest somebody wants watched in its entirety rather than
record by record. It is worth stating because it is the one case where the gap
below has a remedy.

**A markdown whole-file trace is still unstampable**, and still silent. `APLAN028` demands
a stamp for exactly the traces `close --restamp` can produce (`A1-297`), and a
trace with no anchor produces none. Measured while that task was written: zero of
414 closed-task traces across thirteen plans carry no anchor, so a rule here
would have no subjects at all.

So it stays silent, and this is the record of why rather than a task filed
against a population of none. `APLAN028`'s own row points here, so a reader who
meets the silence finds the reasoning instead of an open question. If a plan ever
does trace a whole document, the decision is cheap to revisit, and the
measurement above is the thing to re-run first.

## Consequences

An adopter whose source of truth is a spec file can put it under drift detection
without narrating it in prose first. `APLAN008` gains three failure modes that
are told apart because their remedies differ: the document is not addressable,
the anchor is not an address, or it addresses nothing. `--require
traces.structured_documents` answers whether a build has this, because the change
adds no rule code, command, config key, format or evidence kind and would
otherwise be invisible to `facts` (`A1-099`).

The maintenance surface is one module, two parsers this workspace already
depended on, and a value model shared with the schema validator.

**It costs more per trace than markdown does, and the number is recorded rather
than discovered later.** Measured on 200 closed tasks each addressing a distinct
record of one 120-record document: `check` takes 1205 ms against the structured
resolver and 517 ms against the markdown one, because a document is read and
parsed once per trace and a stamped trace is parsed twice -- once to resolve the
address and once to take the fingerprint. The repetition is not new, markdown
does the same thing, but parsing YAML is dearer than scanning headings, so the
new kind makes an old shape visible. A run-scoped document cache is the fix and
is `A1-322`'s, because it is a change to how `validate` is threaded rather than
to what this decision says.

The parsers' own limits are load-bearing here and are asserted by a test: a
`traces` target is named by a task file, a task file can arrive from a fork
(HLD 5.5), and both parsers refuse an alias bomb and anything nested past 128
levels before the address walk ever sees it.
