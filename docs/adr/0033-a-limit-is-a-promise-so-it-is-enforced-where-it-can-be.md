# ADR-0033: A limit is a promise, so it is enforced where it can be and named where it cannot

Date: 2026-09-12
Status: Accepted
Tasks: `A1-308`
Amended 2026-09-12 by `A1-308`'s own audit: two measured facts about the
mechanism, below under *What the audit found*. No decision changed.
Amended 2026-09-13 while `A1-310` built it: the unenforceable-platform warning
does not get a rule code, for a reason the build found. See *What building it
changed*. The decision it implements is unchanged.
Amended 2026-09-13 again by `A1-310`'s audit: the coverage decision below said
`url.check` and the build had left it out. Now implemented, with the three keys
on the `url` block. No decision changed; one was finished.
Extends HLD 5.5 (a task file must never name a command) to the resources a configured command
may use. Supersedes nothing.

## Context

`A1-300`'s dogfooding was killed twice by this host's supervisor for "low memory", and the
obvious reading -- that `aplan` is heavy -- is wrong in the way that decides the design.

**Measured.** `aplan verify --execute` peaks at **30 MB** and stays flat. `aplan` spawns one
child at a time: there is no `rayon`, no worker pool, and the two threads in `spawn_limited`
only drain the child's pipes. Every process beyond that one is the harness's own fan-out, and
that fan-out is not a property this tool can predict:

| harness | peak processes | peak summed RSS | peak PSS |
|---|---|---|---|
| `npm test` (vitest 4, a web workspace) | **160** | 33.0 GB | **25.6 GB** |
| `go test ./...` (a Go service) | 34 | 1.2 GB | 1.2 GB |
| `cargo test -q -p aplan-core` (warm) | 4 | 156 MB | 146 MB |

Two hundred times between the ends, on one machine, for three ordinary harnesses. The vitest
figure is the one that settles the design: 25.6 GB of *unique* memory (PSS, not the
shared-page overcount that inflates cargo's number) inside a **single** invocation.

**Where the peak sits differs too.** For cargo it is in `test.list`, because listing compiles;
for vitest and Go it is in `test.run`. A cap on one of them protects neither repository.

**What the no-`unsafe` rule leaves.** `Cargo.toml` sets `unsafe_code = "forbid"`, which decides
more here than any preference: `pre_exec` + `setrlimit` is out, and a Windows Job Object
through FFI with it. What survives is cgroup v2 through **plain file writes** -- `mkdir` a
scope under the delegated subtree, write `memory.max`, `memory.swap.max` and `pids.max`, write
the child's pid into `cgroup.procs` -- all `std::fs`, and `spawn_limited` already gives each
child its own process group, which is the handle this needs.

**`memory.max` alone is not a limit.** Prototyped on the host this was designed on: a process
joined a cgroup capped at 64 MB and allocated **3 GB**, with `memory.events` reporting
`oom_kill 0`. The pages went to swap. With `memory.swap.max = 0` beside it the same program was
SIGKILLed at once (`exit 137`, `oom_kill 1`). A design that writes only `memory.max` ships a
cap that silently becomes "resident set target, spill to disk".

## What the audit found

Two measurements taken after the decisions above, both of which qualify the mechanism without
changing what was decided.

**The cgroup is unavailable inside a container that is not delegated one -- including this
project's own CI.** Measured inside the `aplan-runner` podman container: `/sys/fs/cgroup` is
mounted read-only, `cgroup.subtree_control` is empty, and `mkdir` of a sub-cgroup is denied,
even though `cgroup.controllers` lists `cpu memory pids`. So on a containerised runner the dial
degrades to the warning decided below, and the honest guidance for that environment is the
`wrap` prefix or the harness's own flags. This is the common CI shape rather than an exotic
one, and an implementation that discovers it only at runtime will warn on every CI run; the
warning therefore has to distinguish *no cgroup v2 at all* (macOS, Windows) from *cgroup v2
present but not delegated* (a container), because the remedy differs: the first has none, the
second is a runner-configuration change.

**A `pids.max` breach is not a kill, and does not look like one.** `memory.max` with
`memory.swap.max` produces SIGKILL -- `exit 137`, `memory.events` recording `oom_kill 1` -- and
is unmistakable. `pids.max` instead makes `fork()` fail with `EAGAIN`: measured at `pids.max=3`,
the command printed `bash: fork: retry: Resource temporarily unavailable` and carried on to
exit with its own error. From the outside that is indistinguishable from a harness that failed
for its own reasons, which is precisely the misreading `A1-300` exists to prevent.

The cgroup records it either way, and that is what an implementation must read rather than
infer: `pids.events` carries a `max` counter and `memory.events` an `oom_kill` counter. A
breach is therefore detected by reading those two files after the command exits, never by
guessing from an exit status.

**The same files give the measurement for free.** `memory.peak` and `pids.peak` are readable on
the same cgroup -- a 300 MB allocation reported `memory.peak` of 303 MB in a probe. `A1-309`,
which reports what a run cost, and `A1-310`, which caps it, are therefore one mechanism read
two ways, and should not be built twice.

## Decision

**A dial, with a declared wrapper as the escape hatch.**

`max_memory` and `max_processes` are per-runner keys. They are enforced by placing the spawned
command in a cgroup with `memory.max` **and** `memory.swap.max` set from `max_memory`, and
`pids.max` from `max_processes`. `wrap` is an argv prefix the repository declares, for `nice`,
`taskset`, `cgexec`, a container runner, or any cgroup property this tool does not model.

This answers HLD 5.5 rather than passing it. That section says redirect policy, timeout and
body-size caps "live in that command where the repository owns them", and the wrapper is the
consistent extension of it. The dial exists beside it because the measured case that needs
protection -- a web workspace at 25.6 GB -- belongs to an adopter who should not have to learn
what `systemd-run` is to stop their laptop swapping. **A task file still names no command**,
which is the property 5.5 exists to protect: both keys live in top-level config only.

**The cap covers every spawned command**: `list`, `run`, `skipped` and `url.check`. The machine
does not care which one ate it, and the two measured worst cases sit in different commands. A
consequence to state plainly: a plain `aplan verify` can now be stopped by a limit, which is
new for a command that previously could fail only on findings.

**A platform that cannot enforce gets a warning, once per run, and the run proceeds.** macOS
has no cgroup v2; nor does a Linux host without a delegated subtree. The alternative considered
and rejected was a hard error: it would make one `.aplan.yml` unusable by a mixed team, and
`aplan check` -- which spawns nothing and is the command most often run -- has no stake in a
limit at all. The cost is stated rather than hidden: **on such a platform the run is
unprotected, and a warning is all that says so.** A repository that wants the guarantee sets
`deny_warnings`, which this project does.

**A breach is reported where the fault is, and never as a failing test.** `A1-300` established
the principle: a test that did not run must not be reported as one that failed. A command
SIGKILLed at its cap neither passed nor failed, and the fault is as likely to be the limit as
the test.

- A breach in `list`, `skipped` or `url.check` is a **runner fault** on `.aplan.yml`, through
  the `APLAN013` channel that already carries "the runner could not be asked".
- A breach in `run` is **`APLAN014`** on the task, worded as stopped at its limit -- the same
  widening `A1-300` made for a citation that was never run.

No new rule code. The cost of that choice, stated: a breach cannot be targeted by `advisory:`,
because it has no code of its own.

## Consequences

`A1-310` builds this. What it owes beyond the mechanism: both cgroup files written, not just
`memory.max`; the warning on an unenforceable platform; a fixture that really exceeds a limit
rather than one asserting the configuration was read; and the six gates a new config key owes
here -- `CHANGELOG` announcement, `EVERY_ACCEPTED_KEY` fixture and count, scaffold entry,
schema regeneration, `docs/upgrading.md` row, catalogue.

**Rejected: reducing the need instead of capping it.** One `test.run` per runner carrying many
names would cut 563 invocations to ten on this plan, and it would have done nothing for the
case that matters: vitest reaches 160 processes and 25.6 GB inside *one* invocation. Batching
is a performance idea, not a safety one, and selecting several tests at once is harness-specific
besides.

**Rejected for now: a dial over `aplan`'s own concurrency.** It spawns one child at a time, so
`max_parallel` would cap nothing. It becomes necessary the moment `verify` runs runners in
parallel, and belongs to that change rather than this one.

**Already available, and now documented**: `aplan` does not clear the child environment, so
`CARGO_BUILD_JOBS`, `RAYON_NUM_THREADS`, `GOMAXPROCS` and a harness's own flags already reach
the runner. Proven rather than assumed -- a runner whose `list` is
`sh -c 'echo "jobs-${CARGO_BUILD_JOBS:-unset}"'` resolves a citation of `jobs-7` under that
variable and fails to resolve it without. That is the mitigation an adopter can use today, on
every platform, with no key at all.

## What building it changed

**The warning is a note, not a rule code.** ADR-0033 said `deny_warnings` would
upgrade it, which assumed a code. `A1-310` tried that and withdrew it: every rule
here owes the corpus a fires *and* a silent scenario, and no scenario can produce
this one. On a machine where cgroups work the condition cannot be created, and a
test cannot pretend otherwise because `std::env::set_var` is `unsafe` in this
edition and the workspace forbids that. A rule this project's own gates could
never exercise is precisely what
`the_catalogue_pins_every_rule_and_is_not_vacuous` exists to catch.

So it is a line on stderr beside the run-cost note, naming which case it is in.
**The cost, stated rather than buried: `deny_warnings` does not turn an
unenforced limit into a blocked build.** A repository that needs the guarantee
has to assert it another way -- running the gate where cgroups work is the honest
one.

**The proof that the kernel enforces it cannot run where this project's gates
run.** CI here is a rootless container with `/sys/fs/cgroup` mounted read-only
and `cgroup.subtree_control` empty -- the audit's `NotDelegated` case exactly, so
`aplan`'s own CI is a machine its own feature cannot protect. A test that needs a
real cgroup therefore skips there, and the rest of the mechanism is written to be
testable without one: a cage is a directory of plain files, so handed a temporary
directory it writes the same names and contents it writes under
`/sys/fs/cgroup`, and the walk that finds a delegated root is a function of a
`/proc/self/cgroup` string and a mount path. Everything but the kernel's own half
is then covered on any machine, and the kernel's half lives in an integration
test that skips where there is nothing to prove it against.

**The audit found four ways this could still go wrong, and one thing it had not
done at all.** `url.check` was never capped, so the sentence above -- "the cap
covers every spawned command" -- described an intention rather than the code;
`url` now takes the same three keys, and the validation behind them is one
function so the two blocks cannot drift. Of the four: an empty command under a
cap was swallowed rather than refused, because the shell that enrols the child
makes any argv non-empty and the emptiness guard sat behind it (reproduced: a
`skipped: []` reported `APLAN013` without a cap and went green with one); a
timed-out command's `setsid` child survived the process-group kill and held the
cgroup open, so both the runaway and the directory leaked, which `cgroup.kill`
now ends; a stopped command's breach never reached the public `Cost`; and a
probe the kernel had stopped was still used as the profile of what a miss looks
like. Each is the same shape: a cap is a new way for a command not to have run,
and every place that assumed *ran or failed* had to learn a third answer.

**And the delegated root is not the process's own cgroup.** The first
implementation created the cage under whatever cgroup `aplan` was in, which on an
ordinary desktop session is a leaf scope: creating a child there succeeds and
yields a directory with no controllers in it, so every limit read as
*unenforceable* on machines where it plainly was not. A cgroup may not both hold
processes and delegate controllers, so the search walks up to the nearest
ancestor that does -- `user@<uid>.service` for a systemd session. Found by running
it, not by reading it.
