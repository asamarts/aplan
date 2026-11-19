# Roadmap

Public development started from a reviewed source snapshot with a new Git history and a new
evidence ledger. The live plan under `aplan/` is the source of truth, and this page explains its
order. `ROADMAP.status.md` is the generated view of where each cut stands, and `aplan ready` lists
what can be started now.

On 2026-11-18 every open issue and every open item of the pre-public backlog moved into this plan,
each rewritten so that its reason stands on public material alone (`A2-019`). The cuts below are
sequenced by their milestones' `blocked_by` edges, so `ready` lists work in priority order. To pull
a task forward, move it into an earlier cut with `aplan mv` rather than starting it out of order.
Each section's `**Tasks:**` line is checked against its milestone by `aplan check-docs`.

Some issue requests ask for behaviour an earlier decision ruled out, among them trace retargeting
(#30) and what clears `APLAN045` (#31) in `v0.8`, and the four in `adoption`. Each has a decision
task that blocks its implementation task.

## Cuts

### v0.7, Security, safe parsing and gates that tell the truth

It clears the RustSec advisories that turned the scheduled CI red (`A2-016`), keeps the source
clean under Rust 1.100 while the pin stays at 1.99.1 (`A2-017`), finds the cause of the hosted TUI
latency regression before any budget moves (`A2-018`), and stops the render gate failing on slower
hosted runners (`A2-164`). It replaces the C-derived YAML bridge with the safe Rust parser
(`A2-009`), fixes the reported cases where a check passed on input it never looked at or a report
said something false (#18, #24, #27, #29, #33 items 1 and 6, #26, #28), and makes the Action's
install sturdier and better documented (#35, #36). It closes what an audit of the increment found:
the parser switch's unchecked corners (`A2-166`), the files it refuses that v0.6.0 read (`A2-169`),
a README recipe that failed as written (`A2-167`), a per-task walk of the roadmap (`A2-168`),
what a second audit found in those fixes (`A2-177`), what an audit of the tag rule found
(`A2-179`), and a progress row that can show a run's end without its start (`A2-178`). It moves
the open pre-public backlog and every open issue into this plan (`A2-019`) and ends with the
v0.7.0 release (`A2-020`).

**Tasks:** `A2-009`, `A2-016`, `A2-017`, `A2-018`, `A2-019`, `A2-020`, `A2-021`, `A2-022`, `A2-023`, `A2-024`, `A2-025`, `A2-026`, `A2-027`, `A2-028`, `A2-079`, `A2-137`, `A2-164`, `A2-166`, `A2-167`, `A2-168`, `A2-169`, `A2-177`, `A2-178`, `A2-179`

### v0.8, Correctness and the most reported adoption failures

Sequenced after `M-v0.7`. The remaining wrong answers: a mismatched build judging a plan,
generated agent contracts that refuse a correct plan, claims that outlive the work, drift that
passes before a commit and fails after it, and evidence or MCP refusals that contradict the state
they report.

**Tasks:** `A2-034`, `A2-050`, `A2-051`, `A2-052`, `A2-053`, `A2-054`, `A2-055`, `A2-056`, `A2-062`, `A2-081`, `A2-085`, `A2-091`, `A2-095`, `A2-096`, `A2-097`, `A2-099`, `A2-101`, `A2-106`, `A2-122`, `A2-140`, `A2-142`, `A2-147`, `A2-152`, `A2-153`, `A2-156`, `A2-157`, `A2-160`, `A2-161`

### hardening, Integrity and hardening

Sequenced after `M-v0.8`. Integrity work that is not yet a reported wrong answer: diagnostics and
generated guidance that can mislead, ratification and precedence edge cases, and the cost of
resolving test evidence inside the plan gate. It also makes the TUI end by its signal, not with an
error, when its terminal hangs up, and holds the audit's lower-severity findings: MCP replies lost
when a client closes its input (`A2-170`), TUI ranking on keys that change only the selection
(`A2-171`), CI that skips the tests reading a changed plan or README (`A2-172`), YAML limits
not yet proven through product reads (`A2-173`), and real-terminal tests that fail when the host is busy
(`A2-176`).

**Tasks:** `A2-069`, `A2-087`, `A2-127`, `A2-131`, `A2-133`, `A2-141`, `A2-145`, `A2-162`, `A2-165`, `A2-170`, `A2-171`, `A2-172`, `A2-173`, `A2-176`

### adoption, Adoption and ergonomics

Sequenced after `M-v0.8`. Authoring, mutation and reporting ergonomics found by adopters and in
issues, including the decision tasks that gate issue-requested changes to earlier decisions:
recording note evidence from the CLI (#19), how parked effort counts (#20), restamping several
tasks at once (#22) and roadmap order (#23). Each decision is recorded, and any superseded ADR
updated, before its implementation task can start.

**Tasks:** `A2-029`, `A2-030`, `A2-031`, `A2-032`, `A2-033`, `A2-035`, `A2-036`, `A2-037`, `A2-038`, `A2-039`, `A2-040`, `A2-041`, `A2-042`, `A2-043`, `A2-044`, `A2-045`, `A2-046`, `A2-047`, `A2-048`, `A2-049`, `A2-057`, `A2-058`, `A2-059`, `A2-060`, `A2-061`, `A2-063`, `A2-064`, `A2-065`, `A2-066`, `A2-067`, `A2-068`, `A2-070`, `A2-071`, `A2-072`, `A2-073`, `A2-074`, `A2-075`, `A2-076`, `A2-077`, `A2-080`, `A2-082`, `A2-083`, `A2-084`, `A2-086`, `A2-090`, `A2-092`, `A2-093`, `A2-094`, `A2-098`, `A2-100`, `A2-102`, `A2-103`, `A2-104`, `A2-105`, `A2-107`, `A2-123`, `A2-124`, `A2-125`, `A2-126`, `A2-128`, `A2-129`, `A2-130`, `A2-132`, `A2-138`, `A2-143`, `A2-144`, `A2-146`, `A2-148`, `A2-149`, `A2-150`, `A2-151`, `A2-154`, `A2-155`, `A2-158`, `A2-159`, `A2-163`, `A2-174`, `A2-175`

### workflow, Workflow ergonomics

Sequenced after `M-v0.8`. Everyday workflow surfaces: trustworthy completion history, readable
terminal Markdown, the capture workflow over MCP, versioned structured MCP outputs, and bounded
read latency across every read surface, ending in an audit of the whole cut. That audit
(`A2-117`) waits on the TUI release-quality cut, as `A2-089` requires.

**Tasks:** `A2-108`, `A2-109`, `A2-110`, `A2-111`, `A2-112`, `A2-113`, `A2-114`, `A2-115`, `A2-116`, `A2-117`, `A2-118`, `A2-119`, `A2-120`, `A2-134`, `A2-135`, `A2-136`, `A2-139`

### tui-quality, TUI release quality

Sequenced after `M-v0.8`. An evaluation of first-use and expert workflows with real participants,
and an adversarial release audit of the terminal interface.

**Tasks:** `A2-088`, `A2-089`

### qualification, Product qualification

Sequenced after `M-v0.8`. Repeated, predeclared product evaluations of the public baseline across
independent model provider families.

**Tasks:** `A2-121`

### backlog, Considered and deferred

Questions considered and deliberately deferred, with their reasoning attached. A member that
becomes worth doing moves into the cut that will do it.

**Tasks:** `A2-078`
