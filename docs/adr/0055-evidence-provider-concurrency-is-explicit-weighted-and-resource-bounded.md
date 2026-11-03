# ADR-0055: Evidence-provider concurrency is explicit, weighted and resource-bounded

Date: 2026-10-24
Status: Accepted
Tasks: `A1-643`

## Context

One evidence run can have several independent test providers. In the measured P0000021 shape, five
warm catalogue commands alone took about 9.0, 3.0, 2.8, 1.4 and 1.0 seconds; asking them serially
makes their sum the critical path. ADR-0054 reduces exact-test process count by batching identities,
but its one adapter per runner was also serial.

Unbounded parallelism is not an acceptable fix. A browser runner may own a fixed port or database,
a compiler may hold a cache lock, and one top-level provider may already fan out to many workers.
Per-command `max_memory` and `max_processes` constrain a child independently; they neither allocate
one aggregate budget nor describe logical resources shared by otherwise independent children.

The old configuration also carried two different concurrency meanings: ordinary verification ran
providers serially, while interactive observation refresh imposed an internal two-runner ceiling.
That hidden host choice could not express a repository's real resource constraints and made the same
providers behave differently by caller.

## Decision

**1. Existing forms stay serial; concurrency requires a new explicit shape.** A single `test`
mapping and the existing sequence of mappings have aggregate capacity one. They retain their
ordering and resource use without an inferred migration. A repository opts in with:

```yaml
test:
  schedule: {capacity: 4}
  runners:
    - name: rust
      weight: 2
      exclusive: [compiler-cache]
      list: [cargo, test, --workspace, --, --list]
      run: [cargo, test, --workspace, --, --exact, '{test}']
    - name: browser
      weight: 1
      exclusive: [browser-port]
      list: [ci/browser-tests, list]
      run: [ci/browser-tests, run, '{test}']
```

`capacity` is a positive integer. A runner's positive `weight` defaults to one and may not exceed
capacity. This replaces the interactive refresh's internal two-runner value: catalogue discovery,
whole-plan verification, test discovery, refresh, MCP and TUI all read the same repository policy.

**2. Named resources are capacity-one exclusions within one invocation.** `exclusive` is an
optional nonempty list of unique, nonblank, case-sensitive names. A job reserves its entire sorted
resource set and weight atomically or reserves nothing. It therefore cannot hold one lock while
waiting for another, and duplicate names are refused at configuration load rather than becoming a
self-deadlock. These names do not claim to be machine-wide or distributed locks; two independent
aplan processes still require an external coordinator if they share such a resource.

**3. One runner stage is the scheduled unit.** Each runner's catalogue command is one job. Under
`--execute`, that runner's optional skipped-list command followed by its one batch adapter is a
second job under one reservation. Keeping those two commands together prevents a peer from entering
the declared exclusive resource between discovery and execution. Legacy per-identity `test.run`
remains serial: repositories seeking execution fanout use the complete-result batch protocol rather
than multiplying opaque processes.

A job's weight is admission accounting, especially for harnesses with nested worker pools. It is
not an OS memory or process limit. Every configured command still receives the runner's existing
cgroup `max_memory` and `max_processes`, wrapper, output bound, directory and process-group owner.

**4. Scheduling and publication are deterministic and fail closed.** The coordinator considers
the configured queue in order, but may bypass an older job that cannot fit the currently free
weight/resources with the first later job that can. The queue is finite; releasing reservations
always makes the oldest compatible job eligible, so this cannot starve a job. Completion order is
live progress only. Catalogue rows, diagnostics, excerpts, evidence outcomes and cost ownership are
merged by configured runner and retain the existing final diagnostic sort.

One worker panic or provider failure becomes that runner's typed fault and does not suppress peers.
A fresh catalogue carries each runner's typed timeout, output-limit or unavailable outcome into
citation observations; it does not collapse every failure to unavailable or emit its diagnostic
twice. A cancelled, panicked, missing or partial runner can never become a complete clean catalogue.

**5. Termination closes the start boundary and drains active work.** Before every admission, the
coordinator reads A1-592's process-wide termination state. Once set, it starts no further job,
marks queued jobs cancelled and waits for active workers to return. Those workers use the existing
configured-command lifecycle, which kills and reaps each registered process group while continuing
to drain captured pipes. Observation publication already requires a complete stable catalogue, so
cancelled work cannot replace a current observation.

## Consequences

- Safe concurrency is repository policy rather than a caller-specific constant. Unknown budgets
  remain serial, which spends time instead of making an unsafe resource guess.
- A capacity is deliberately dimensionless. Authors choose weights from measured nested fanout;
  named resources express non-divisible constraints that a scalar cannot.
- The scheduler creates at most one worker per admitted runner, never one per queued runner or cited
  identity. It has no dependency beyond the standard library.
- Event-driven tests gate weighted capacity, atomic exclusions, compatible-job progress,
  cancellation, fault isolation and real catalogue/skipped/batch overlap without timing sleeps.
- The P0000021-shaped harness reports legacy direct replay, batched serial execution and the same
  batches under capacity four separately. Wall time remains characterization; exact process counts
  are the portable regression gate.

## Alternatives

| option | why not |
|---|---|
| Always use the host CPU count | ignores memory, nested fanout, fixed ports and shared fixtures; it also changes behaviour between machines |
| Preserve the hidden interactive limit of two | gives one caller a policy the repository cannot inspect and leaves verification serial |
| One thread per configured runner with a semaphore | creates unbounded queued threads and cannot reserve several named resources atomically |
| Infer weight from `max_memory` or `max_processes` | those are independent child ceilings, not actual or comparable aggregate consumption, and may be unenforced on the host |
| Parallelize legacy `test.run` per identity | restores process-count growth, complicates miss probes and encourages unbounded framework fanout; ADR-0054 is the attributable execution seam |
| Use a general async runtime or scheduler crate | the work set is finite and commands are blocking; a small scoped-thread coordinator covers the contract without adding a runtime dependency |
| Make exclusions machine-wide | requires cross-process lock ownership, crash recovery and namespace policy outside this invocation; the configuration does not claim those semantics |
