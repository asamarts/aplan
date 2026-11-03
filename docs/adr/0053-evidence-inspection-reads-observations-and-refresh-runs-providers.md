# ADR-0053: Evidence inspection reads observations and refresh runs providers

Date: 2026-10-23
Status: Accepted
Tasks: `A1-590`

## Context

`aplan show ID` used to call the evidence verifier synchronously. Resolving a bare `test:` name
correctly asks every configured runner: stopping after the first match would miss ambiguity. On
this repository that means twelve `cargo test -- --list` processes before the command writes its
first byte. A configured process can compile, wait for a shared lock, contact another service or
never return. No implementation can promise a bounded read while opaque provider execution remains
on its default path.

Skipping providers and reporting a clean task would be worse. A previous provider result is useful
only when the tool can prove what it observed and whether every material input is unchanged. A
cached final verdict is also the wrong artifact: changing a task's paths can change test locality,
and adding a runner can turn a formerly unique name into an ambiguity, without changing the old
result's bytes.

The existing plan cache establishes the local precedent: derived state is disposable, bounded,
private, checksummed and atomically published, while source remains authoritative. Provider state
needs a separate identity because its inputs and trust boundary differ from Markdown parsing.

## Decision

**1. Ordinary inspection never starts a configured provider.** `aplan show ID` and `aplan_show`
read a retained observation when it is current. Repository-backed commit and file resolution may be
recomputed locally; configured `test.list`, URL, version and execution commands are forbidden on the
default path. `--no-resolve` additionally skips retained observations and repository resolution.
`--refresh-evidence` is the explicit slow CLI path. Whole-plan `verify` and the TUI's explicit
verification action are also refresh operations.

**2. The persisted test artifact is the complete raw catalogue, not a verdict.** A current
catalogue retains every configured runner's ordered identity, success/empty/fault state and exact
normalized names. Inspection feeds it back through the ordinary `TestResolver`. Ambiguity,
near-match repair, path locality and task findings are therefore derived again from the current
plan. Bare names still ask every applicable catalogue. This decision does not introduce a
runner-qualified task syntax; that would widen evidence identity and needs its own decision.

**3. Reuse requires an exhaustive material-input claim.**
`evidence.observation_inputs` is an optional list of repository-relative globs. It is a trust claim:
the repository says every file capable of changing configured provider discovery is named. The
observation identity additionally includes the complete provider configuration, inherited process
environment, resolved configured executable bytes, Rustup settings and installed-toolchain manifest
hashes when a Rust tool is configured, and the observation protocol. Rustup metadata is read as
bytes; ordinary inspection never starts `cargo`, `rustc` or `rustup` merely to identify it.
The declared paths are enumerated and content-hashed before and after refresh. Absence of the claim,
an unreadable or symbolic-link input, or an input that changes during refresh is non-cacheable; the
tool reports `unsupported` or `stale` instead of guessing. Generated provider outputs should not be
declared, because a provider that changes its own input cannot produce a stable observation.

The current format does not retain URL answers. Remote content has no material-input identity or
expiry contract in the configuration, so a default URL observation is `unsupported`; an explicit
refresh can still report its immediate answer. A later TTL or immutable-subject design can extend
the protocol without pretending an old network result is current.

**4. Provider and derived identities are distinct.** The provider observation binds provider
configuration, complete material inputs, environment, executable and applicable Rustup toolchain
identity, protocol and raw catalogue. A derived resolution separately binds the exact typed plan, evidence configuration and
repository observation to that provider observation. Refresh publishes the two identities in one
checksummed envelope. A
plan-only edit can reuse the raw catalogue and derive a new resolution without rerunning providers;
it cannot reuse the previous plan verdict.

**5. Freshness is typed at both levels.** Each citation and the aggregate expose `current`, `stale`,
`not_observed`, `refresh_failed` or `unsupported`. `current` says the resolver ran against current
inputs; it does not mean the citation passed. A current failed citation therefore carries a current
observation and an evidence finding. Stale, absent, partial or unsupported data never contributes a
clean evidence verdict. The compatibility field `resolved` remains and is true exactly when the
aggregate state is `current`. Because that changes the meaning and adds fields, show JSON advances
from `aplan/show/v2` to `aplan/show/v3`.

**6. Missing disposable state does not make reading a task fail.** Plain `show` returns success
when it can show the task, while naming non-current evidence. Automation that requires current
evidence uses `--require-current-evidence`; the task is still rendered, then the command returns the
ordinary violations status when any citation is not current. `verify` remains the authoritative
whole-plan gate and does not accept retained observations as a substitute for its explicit run.

**7. Refresh is bounded, observable and cancellation-safe.** Test catalogues use a fixed worker
bound (two by default), one listing process per configured runner and deterministic configuration
order in the final result. Progress goes to stderr and names runner, elapsed time, completed work
and remaining work; it does not invent a percentage for opaque commands. Existing per-command
time, memory, process and output limits still apply. The existing process-wide termination contract
owns child cleanup. A partial catalogue, provider fault, cancellation or input mutation cannot be
published as current. A short publication lock prevents an older concurrent refresh from replacing
a newer complete envelope.

**8. One application operation owns the semantics.** CLI, MCP and TUI call the same typed inspect
or refresh operation. MCP `show` is non-blocking by default and MCP `verify` refreshes. The TUI keeps
provider work on its bounded background worker and rejects completions whose input identity no
longer matches. Adapters render typed state; none parses another adapter's output or implements a
second cache policy.

## Consequences

- A cold ordinary `show` is fast and honest: local evidence can be recomputed, configured evidence
  says `not_observed` or `unsupported`, and no provider process starts.
- The first explicit refresh can still cost a build. Subsequent reads reuse exact raw names while
  declared material inputs remain unchanged.
- Repositories that want cross-process reuse must audit and maintain their input declaration. A
  missing input can make the trust claim unsound, so documentation calls it evidence policy rather
  than a cache tuning hint.
- Inherited environment is deliberately part of identity. Two shells with different provider
  environments may not share observations even when their source tree matches.
- The cache remains disposable and non-authoritative. Deleting it loses an optimization and
  freshness record, never evidence or task state.

## Alternatives

| option | why not |
|---|---|
| Keep synchronous resolution on `show` | an opaque command makes bounded read latency impossible |
| Stop after the first runner lists a name | silently accepts an ambiguous citation according to configuration order |
| Persist the previous clean/failed verdict | plan-only path and policy edits can change the answer without changing provider output |
| Key only by Git HEAD and config | dirty, untracked, environmental, executable and generated inputs can change a provider under the same commit |
| Hash the whole worktree implicitly | provider outputs such as `target/` change during discovery and large ignored trees make reads unbounded |
| Assume commands are cacheable without a declaration | turns missing input knowledge into false freshness rather than an explicit unsupported state |
| Treat missing observations as command failure | makes deletion of disposable optimization state break a read that can still show the task |
| Cache URL success indefinitely | network reachability has neither immutable identity nor an expiry contract |
