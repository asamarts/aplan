# ADR-0007: A four-code exit contract

- Status: Accepted
- Date: 2026-07-24
- Deciders: kaminsod

## Context

`aplan` is meant to run in CI, and its exit code is the interface a workflow acts
on. Until now the contract was three codes: `0` clean, `1` at least one error
(violations), `2` "usage or internal failure". Bundling *usage* and *internal*
into a single `2` loses the distinction a consumer most wants at 2 a.m.: **is this
my fault or the tool's?** A malformed `.aplan.yml`, a bad flag, and an `aplan`
panic all exited `2`, so a workflow could not decide between "fix your input" and
"file a bug", and neither could a human reading the log.

`alint`, the sibling linter this project borrows conventions from, uses a
four-code contract for exactly this reason. Splitting the codes is a **breaking
change** to a documented contract, so the cost is lowest before there are any
consumers: the repository is pre-1.0, private, and unreleased.

## Decision

Adopt four exit codes:

| Code | Meaning |
|---|---|
| `0` | clean (warnings allowed) |
| `1` | at least one error (violations) |
| `2` | usage or configuration error, the invocation, the input, or `.aplan.yml` |
| `3` | internal error, a bug in `aplan` |

`2` is the **default** for any error the CLI surfaces, because most CLI errors are
the user's (a bad flag, an unknown task, a config that will not parse, a task in
the wrong state to close). `3` is reserved: an error is tagged internal only where
`aplan` itself misbehaved, today the sole case is a `close`/`restamp` that fails
its own round-trip self-check (`CloseError::Produced`), a "this should never
happen" invariant. A hard panic stays at `101`, the Rust convention for an
unhandled crash; the contract governs the errors `aplan` surfaces gracefully.

The codes are carried in `facts.json` (`exit_codes`), so tooling reads them from
one generated place rather than a hand-copied table, and the docs (README, HLD
§8) state the same four.

Because every error path that actually fires today is a usage/config error, the
observable behaviour of the existing CLI corpus is unchanged (every case that
exited `2` still does); `3` is a new, reserved code that no current path reaches
in normal use. So this "breaking change" refines the contract without regressing
any tested behaviour, which is why it is safe to land now rather than defer.

## Consequences

- A consumer's CI can branch on the code: `2` means "fix your input or config",
  `3` means "open an issue against aplan".
- The internal/usage split is enforced by a marker (`Bug`) on the error, not by
  guessing from the message, and is unit-tested (`errors_map_to_usage_unless_
  tagged_a_bug`).
- New internal invariants get `3` for free by tagging their error; new usage
  errors get `2` by default, so the contract does not rot as commands are added.
- The change is not observable in the black-box corpus (no case moved from `2`),
  so the trycmd fixtures did not need regeneration.

## Alternatives rejected

- **Keep three codes.** Rejected: it permanently conflates the user's fault with
  aplan's, the one distinction a CI most needs, and the cost of splitting only
  rises with adoption.
- **Catch panics and remap `101` to `3`.** Rejected for now: a panic is a distinct,
  louder signal (a stack trace, not a graceful message), and `catch_unwind` adds
  machinery for a case that should not occur. `3` covers the internal errors aplan
  detects and reports; `101` covers the ones it does not.

## References

- HLD.md §8 (CLI surface), §17 (distribution, version as single source of truth).
- ADR-0001 (status is a falsifiable claim): the same "the tool must not lie about
  itself" ethos, applied to its own exit code.
