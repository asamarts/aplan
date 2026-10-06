# ADR-0044: A write surface earns its place against the actor that has no editor

Date: 2026-10-01
Status: Accepted
Tasks: `A1-359`
Extends ADR-0020 (a closed task is not reopened) and `A1-077`'s test for a mutation. Supersedes
nothing: `A1-077`'s reasoning was right about the actor it had in mind, and this names a second one.

## Context

`A1-077` set the test every mutating command in this tool has been held to since: **can hand-editing
break an invariant that `check` does not catch on the next run?** On that test it shipped `mv` and
`drop` and refused `status <id> <status>`, with a reason worth quoting because it is the one being
revisited:

> Hand-editing a status breaks no invariant that `check` does not catch on the next run: an
> estimate, a `paths` glob, a live status transition are all edits the tool re-validates without
> needing to have made them. A command for them would be write surface bought with no invariant to
> maintain, and every command that writes is a command that can corrupt a plan.

That is correct, and it contains an assumption it does not state: *hand-editing is available*. It is
available to a person with a text editor. It is not available to an agent consuming this tool
through its MCP server, which has a fixed set of tools and no file-editing primitive at all. For
that actor the sentence "hand-editing is fine as it is" is not a weaker argument; it is false.

The plan bears out the consequence. `Status` has five values and three are reachable by command:
`todo` from `add`, `done` from `close`, `dropped` from `drop`. `in_progress` and `blocked` arrive
only from `import --format roadmap-json`, carried across from another tool, and `close::set_status`
has exactly one caller, which passes `Dropped`. So a status could arrive as an unverifiable claim
from elsewhere, be reasoned about by every rule that reads it -- `in_progress` is `is_workable()`,
so `ready` offers it and `stale` counts it live -- and never be corrected, only ended.

A second gap sits beside it and is not about statuses at all. Two agents sharing a repository have
no way to say "I am on this". `owner` exists and no command writes it after creation.

## Decision

**A write surface may also earn its place by maintaining an invariant for an actor that cannot
hand-edit, or by maintaining one hand-editing provably cannot.** `A1-077`'s test stands; this adds
the second clause and names the first's hidden premise.

Four commands ship, and they earn their place by two different arguments, which is the point of
separating them:

**`claim` and `release` pass `A1-077`'s original test unchanged.** The invariant is *mutual
exclusion*. Two agents both read "unassigned", both write their name, the last writer wins, and
`check` reports a perfectly valid plan over two actors doing the same work. No rule can catch that,
because nothing about the result is invalid. `aplan_core::save::replace` is a compare-and-swap --
"the file no longer holds what the caller read" -- so a claim routed through it refuses the second
claimant. That is an invariant hand-editing cannot maintain, which is exactly the bar.

Measured while building it: checking the holder against the *plan snapshot* rather than against the
bytes about to be swapped reopens the race, because the snapshot predates the read. Two of eight
simultaneous claimants won. The check and the `expected` have to describe the same state.

**`start` and `stop` earn it by the new clause.** There is no invariant here that hand-editing
breaks; there is an actor for which hand-editing does not exist. They therefore ship on both
surfaces in one change rather than CLI-first, because the surface they are justified by is the MCP
one.

**A stored claim is checked rather than trusted.** This is the part that keeps the decision
consistent with the product's premise. `done` is believed because `verify` resolves what it cites;
`in_progress` would be a claim with nothing to resolve against, and a stale claim is the failure
this tool exists to expose in other people's plans. `A1-224` refused `close --criteria-met` on that
exact ground. So `APLAN053` reports an `in_progress` the repository holds nothing for, and the
premise is stated precisely: **not "do not store claims", but "do not take claims on trust".**

**Identity is a label, and never comes from the repository's config.** `--as`, then `APLAN_OWNER`,
then `git config user.name`, then the operating system's user; a refusal names all four. Not
`.aplan.yml`, because several actors share one repository and a name recorded there would be the
wrong one by construction. Measured on one machine the three derived sources disagree, which is why
the explicit forms come first. The exclusion comes from the compare-and-swap rather than from who
the caller says they are, so nothing here is a security boundary and nothing pretends to be.

## Consequences

The test for a future mutation now has two clauses, and the second one can be abused: "an agent
cannot do this" is true of every file edit, so it would justify a CRUD surface if read alone. It is
not read alone. The clause is that the actor cannot do it *and* the thing being maintained is an
invariant the plan cannot express -- for `start`/`stop`, that a status nobody can set is a status
nobody can correct. A command that merely saves a person some typing still fails `A1-077`.

`ready` and `stale` now name the holder, and `--unclaimed` filters to work nobody holds. The default
set is deliberately unchanged: a caller reading `ready` today must not silently start seeing less.

`blocked` keeps its meaning and gains no command. "Explicitly parked for a reason outside the
dependency graph" is precisely what no repository state records, so there is nothing to derive it
from and nothing to check it against -- and a command to set it would be the write surface this ADR
says has to be argued for, with no argument available.
