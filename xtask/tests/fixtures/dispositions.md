# A pilot log, in the shapes the disposition gate has to tell apart

This file is a fixture for `A1-111`. The two findings that prompted that task were
repaired in pre-public adoption evidence before the gate existed, so the gate is proved
here instead: a live document that has been fixed cannot demonstrate a gate that
catches it.

## 1. A finding whose disposition names a placeholder

The shape that sat unnoticed from 2026-08-11: an id-looking word that is not an id,
in a sentence that claims the work is somewhere.

Tracked as `A1-nnn`.

## 2. A finding whose disposition names a task

Tracked as `A1-040`.

## 3. A finding filed under a milestone that does not exist

The second verb and a dotted prefix, so neither is carried by the first case alone.

Filed under `M-v0.N`.

## 4. Prose that teaches the convention

Write a placeholder in lowercase (`A1-nnn`) when you mean "a task id" rather than a
specific task. This paragraph must pass: a gate that fails the file explaining the
convention has misread the problem.

That matters because dispositions are the whole point: an item that says
"Tracked as `A1-nnn`" when no such task exists is worse than an item with no
disposition, since it reads as handled. Quoting a claim is not making one.

## 5. A disposition naming an id that could exist and does not

Tracked as `A1-999`.

## 6. A block showing the shape it warns about

```
Tracked as `A1-nnn`.
```

The fence is the document's way of showing its own format, and nothing in it is a
claim about this plan.
