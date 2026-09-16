# ADR-0032: A generator merges, an exporter renders, and that is why there are both

Date: 2026-09-09
Status: Accepted
Tasks: `A1-189`
Extends ADR-0012 (an importer is lossy, and says so) to the outgoing direction. Supersedes
nothing.

## Context

`A1-189` asked a question this project could not leave open: `import --format` reads five shapes
and there was no `export --format` at all. What existed instead were two hand-built commands,
`aplan export-roadmap-json` and `aplan export-agents-md`, each for exactly one artefact.

The task's own wording is the constraint: *"They are the two existing answers and should not
become the two exceptions."* Adding a third way to write a file, with the two older ways left
beside it unexplained, would leave the surface with three answers to one question and no rule
saying which applies.

So `export --format checklist` is one half of the answer, and this is the other: **what the two
existing commands are, and why folding them in would lose something.**

## Decision

**There are two kinds of write here, and they stay two commands.**

An **exporter** (`aplan export --format <shape>`) renders. Its only input is the plan. It
produces bytes; where those bytes go is the caller's choice, and with no `--out` they go to
standard output. Nothing at the destination is read, so a destination need not exist.

A **generator** (`aplan export-roadmap-json`, `aplan export-agents-md`) merges. **The target file
is an input**, and the command replaces the part of it the plan owns while carrying everything
else through untouched:

- `export-roadmap-json` is a merge by design, and the command's own help says why in one line:
  that document carries fields this model does not hold, and a file generated from the plan alone
  "would validate here and break there". The file is its own template. A milestone the plan has and the
  file does not is *reported* rather than invented, because where it belongs in a foreign schema
  is not something this tool can know.
- `export-agents-md` writes a fenced block *inside* a file the repository owns, between markers,
  and its own code says the reason: it keeps "whatever the repository already wrote", and never
  prepends, because a file starting with a generated block reads as a generated file and that one
  is not.

Folding those into `export --format X --out FILE` would make `--out` mean two things: "write here"
for one shape and "read this, then rewrite part of it" for another. The second silently makes the
destination load-bearing. A reader who deleted the file first would get, from one flavour, the
same document back, and from another, a document with every field the model does not hold now
missing.

## Consequences

**`export --format` refuses `roadmap-json` by name and points at the generator.** The refusal
names `artifact` -- a field the document carries and the model does not -- and says to run
`aplan export-roadmap-json roadmap.json` instead. So the split is not folklore in an ADR: it is
in the output of the command an adopter actually typed.

**A refusal names the field, not the policy.** Each read-only shape in `aplan_interop::FLAVORS`
carries the one field a generated document would have had to invent, and
`every_read_only_flavor_names_the_field_it_could_not_carry` walks the table rather than a
hand-written list. "Unsupported" is not an answer this tool is allowed to give.

**One table, both directions.** `FLAVORS` is what `import` validates `--format` against and what
`export` dispatches on, so a shape cannot be readable on one side and unknown to the other.

**The honesty rule is the same rule, read backwards.** An import reports what the *task model*
could not hold; an export reports what the *target shape* could not. They are separate methods on
`Report` (`dropped` and `not_carried`) rather than one reworded, because a reader of the report
has to be able to tell which end the loss happened at.

**What is not decided here.** Which further shapes earn an export half. `checklist` earned one
because a `- [ ]` list has one meaning in both directions; `kep`, `spec-kit` and `openspec` each
carry structure the model does not, and each says so by name. A fourth shape argues its own case.

## What would reopen this

A generator whose target file stops being an input -- if `roadmap.json` were ever fully derivable
from the plan, the distinction this ADR rests on would be gone for that shape, and it should fold
into `export --format`. Or an exporter that turns out to need the destination read: that would be
a generator wearing the wrong name, and the two would have to be reconciled rather than left to
diverge.
