# ADR-0041: The version question is asked the way every other question is asked

Date: 2026-09-23

Status: accepted. Supersedes ADR-0040's decisions 1 and 3; the rest of ADR-0040 stands.

## Context

ADR-0040 designed the version surface two days ago against three investigations. Auditing
`A1-320` put that surface next to the CLI that has to carry it, which the design itself
never did, and two of its four decisions do not survive the comparison.

## What the audit found

**The selector was a third spelling of something the CLI already spells.** ADR-0040 chose
`aplan version next --of <id>`. Measured against the surface that exists:

| asking about one node | spelling |
|---|---|
| `aplan why <ID>` | positional |
| `aplan show <ID>` | positional |
| `aplan summary --cut <CUT>` | a filter over many, not a selection of one |

A node is named positionally here, and `--of` is a coinage. This project refuses to tell a
project its vocabulary is wrong (`A1-037`); inventing a third one for itself is the same
fault turned inward.

**The rung was decided for the wrong reason.** ADR-0040 put the whole command in `verify`
because ADR-0038's window reads git, reasoning that "one command must not sit on two rungs
depending on the plan's shape". That conflates two things. The *window* needs git. A *named
node* does not: what a task owes is its declared impact or its level's floor (ADR-0034), and
what a cut owes is an aggregation over its members. Both are answerable from the plan alone.

The original objection was real but is about invisibility -- an answer that silently needs
git because the plan happens to model no release node. An argument is not invisible.

## Decision

**1. The node is named positionally: `aplan version next [<ID>]`.** Same shape as `why` and
`show`, and `<ID>` may name a task, a cut or a release, because all three are tasks in the
model (ADR-0008) and a cut and a release are the same kind of node (ADR-0022).

**2. The rung follows the argument, which the caller can see.**

| invocation | rung | why |
|---|---|---|
| `aplan version next <ID>` | `check` | the plan alone answers it |
| `aplan version next` | `verify` | the window is the newest release tag (ADR-0038), which reads git |

A command whose rung depends on an argument is honest: the caller chose the argument. A
command whose rung depends on whether somebody has filed a release node is not, and that is
what ADR-0040 was trying to avoid and prevented by making everything expensive instead.

**The bare form must say which window it used**, in both formats, because the two windows --
a release node, and everything since the last tag -- give different answers and the caller
cannot see which applied. This is the honesty ADR-0040 bought with the wrong instrument.

## What stands from ADR-0040

Decisions 2, 4 and 5 are unchanged and are not restated here: a bare version and no vendor
output forms; the current version read by a declared command; nothing here writes a version.

## Consequences

`A1-313` is amended again, and this is the second time the surface has moved under it without
a line of it being written -- which is the staging working rather than failing. It is also the
argument for `A1-320` having existed at all: both of these would have been discovered during
the implementation, against code.
