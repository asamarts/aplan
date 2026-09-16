# ADR-0026: Reachability, not a query language

Date: 2026-08-31
Status: Accepted
Tasks: `A1-221`
Extends ADR-0017 (ordering is derived, not declared) and follows the line `A1-214` drew for
selection. Supersedes nothing.

> **Filed as `ADR-0025`, renumbered to `ADR-0026` on 2026-08-31 by `A1-222`.** `A1-222` had
> reserved `0025` in writing for a 2026-08-17 decision being moved off a duplicated number, and
> this ADR took it the day before that task ran. `0025` belongs to the 2026-08-17 block; this
> decision is dated 2026-08-31, so it is the one that moves. The decision below is unchanged.

## Context

Two questions arrived together and were mistaken for one: should a view be scoped by a query, and
should this tool have a query engine. Answering the first with a filter grammar answers the second
by accident, which is how tools acquire one.

The measurement separates them. Every predicate over a *row* of a plan is a `jq` line once
`aplan tasks` carries the field: status, owner, tag, cut, evidence kind, effort. A consumer with a
pipeline already has them, and `A1-075` decided that surface deliberately.

What no downstream tool can do is **reachability**. What closing this unblocks, what lies between
these two, what no milestone can reach: each is a traversal over a graph the consumer would have to
rebuild from `blocked_by` before it could ask. This tool already holds that graph. `ready`, `why`
and `critical-path` are each one hard-coded walk of it, which is the evidence that the traversal is
the tool's job and that the tool had been answering it one question at a time.

## Decision

**The tool owns reachability. `jq` owns rows.**

Four traversals, named and closed:

- `upstream:<id>` -- everything it waits on, transitively.
- `downstream:<id>` -- everything that waits on it.
- `between:<id>..<id>` -- everything on a path from one to the other.
- `unreachable` -- everything no milestone can reach.

`unreachable` is the one that does not take the caller's choice of edges. It asks about
*containment*, and containment is the membership relation: asked over dependency edges a milestone
reaches only other milestones, so the answer would be "almost every task" and would look like a
result. It walks membership whatever it is given, and the view says so.

A fifth is where a grammar starts, so there is no fifth. **Within `--reach`** there is no `and`, no
`or`, no `not`, no comparison operator and no field access. A request for one is answered with
`aplan tasks --format json` and the pipe that follows it.

That is a rule about the *grammar*, not about flags. `A1-214`'s selector -- `--cut`, `--tag`,
`--owner`, `--status`, a conjunction of equality tests over declared dimensions -- is the shape this
composes with, named below, and is not what "no field access" refuses. The distinction is the whole
of it: a fixed set of dimensions each with its own flag can be read off `--help` and cannot grow an
operator; an expression that names a field can grow every operator there is.

**A traversal says which graph it walked, which is not always the one it was asked for.** ADR-0008 models a cut as a meta-task that is
`blocked_by` its members, so `blocked_by` carries grouping and constraint at once (`A1-216`).
Reachability over grouping answers "what is in this cut", which `--cut` already does; over
dependencies it answers what a reader means. The two are different graphs, and a view that did not
say would leave the reader to assume.

**A traversal is a selection, and selections intersect.** `--cut X --reach downstream:Y` is one
question -- what Y unblocks, within X -- and not two. That is `A1-214`'s shape, and reachability
composes with it rather than being a second way to narrow the same view.

## Consequences

**What this refuses, plainly.** This tool will not grow a general filter grammar. Somebody will
want a fifth traversal, then a predicate, then a conjunction, and each will be individually
reasonable; this record exists so that argument starts from the boundary rather than from nothing.

**What it costs.** A question that is a predicate *and* a traversal -- "what does this unblock, among
tasks owned by X" -- needs two steps: the traversal here, and `jq` over `aplan tasks`. That is a real
cost and it is the price of not owning a language.

**What would reopen it.** Evidence that the two-step is the common case rather than the rare one:
users routinely piping traversal output back into a second query. A gate that needs a query
language for ordinary use has chosen the wrong boundary.
