# ADR-0043: Closing a task causes nothing outside the plan

Date: 2026-09-26

Status: superseded in part by ADR-0061

Answers `A1-316`. Extends HLD section 5.5 (the trust boundary) from *what may run* to
*what may decide that something runs*, and holds ADR-0040 decision 5 ("nothing here
writes a version") at the level it generalises to.

## Context

The question arrived as "could closing a task trigger release-please?", and it is worth
answering properly because the answer generalises: **may this tool cause an effect outside
the plan?**

Today it cannot, and that is measured rather than assumed. Every command this tool spawns
comes from one of five keys, all of them top level in `.aplan.yml`:

| key | what it does |
|---|---|
| `test.list` | names the tests a runner knows, so a citation can be checked against them |
| `test.run` | runs a named test, under `verify --execute` only |
| `url.check` | fetches an `https` citation |
| `version.current.command` | prints the version the artifact ships today |
| `version.check.command` | asks whether the artifact matches its declared impact |

Every one of them **observes**. And `close` reaches almost none of them: it resolves the
citation it is about to record with `execute: false`, `url_check: None` and `version: None`,
so of the five it can reach exactly one, `test.list`, which it asks so it can refuse a
citation `verify` would reject -- an observation made in order to say no. What `close`
writes is one task file.

Those three refusals are already in the code with their reasons beside them, and the reasons
are not this ADR's:

```rust
// Deliberately not `config.url`: `close` must not reach the network to
// record a citation. A link down for an hour would block a closure, and
// `APLAN016` is a warning precisely because it is not a statement about
// the plan (`A1-041`).
url_check: None,
```

So the narrow question -- may a closure fire something -- is being asked of a command that
has already been narrowed three times on grounds of availability and scope. The general
question has not been asked, and a trigger would be the first key whose purpose is to change
something rather than to observe it, and the first whose firing is decided by a file that can
arrive in a pull request from a fork.

## Decision

**No. `close` sets a status, attaches evidence and writes a task file. Nothing else happens,
and nothing else may be made to happen. This tool reports state; something else reacts to
it.**

**1. A trigger obeys HLD 5.5's letter and breaks the rule it protects.** The rule is that a
task file must never be able to name a command, and every spawning key is top-level config
for that reason. A trigger would keep that: the command would still come from trusted
config. What it would move across the boundary is the *decision*. An attacker who cannot
choose what runs could choose **that** it runs, and choose when -- by opening a pull request
that closes a task.

This is the reduction webhook security makes from the other side: a request that is
*authentic* is not thereby *safe*, because the payload is still data from somewhere you do
not control. A task file is authentic -- it is in the repository -- and it is not safe.

**2. The MCP case is the sharper one, and it is what `close` being inert buys.** `close` is
one of nine MCP tools, and `aplan-mcp` exists to hold one rule: an MCP `close` goes through
the same evidence gate as the CLI, because the anti-pattern it refuses by example is an
agent grading its own homework. That rule is worth something only because the *output* of a
`close` is inert. Today the worst an agent's `close` can do is write a task file that a gate
then refuses. With a trigger, an agent's `close` could publish -- and the evidence gate,
which decides whether the claim is *true*, was never designed to decide whether the effect
is *wanted*.

**3. `A1-155` bounds which command, not whether.** Naming an explicit trusted config lets a
reviewer of somebody else's checkout decide which programs may run, and `--no-config` lets
them decide that none may. That is the right answer to "whose commands are these" and it is
not an answer to "who decided to fire one". The timing and the occurrence would still come
from the task file, on every invocation that is not a deliberate untrusted review -- which is
almost all of them.

**4. The refusal is not a gap, because the job has a home.** What a person wanted is a
release driven by a plan reaching a state. That is a CI trigger reading a tool's output,
which is what CI already does everywhere: `on: push`, a workflow, an exit code. The
alternative is written down in the next section rather than left as an exercise.

**5. What would have to change to revisit it.** Not a better sandbox and not a narrower
command list. A trigger becomes discussable only when the decision to fire can be attributed
to something outside the task file -- a signature over the closing commit, a branch
protection, an approval -- **and** the bound is stated in terms a rule can enforce, not in
prose. "Only on the default branch" is prose; a rule that reads the ref and refuses is not.
Until then this is a refusal, not a deferral.

## What the job looks like without it

Closing a task is a commit. A workflow reacts to the commit, asks the plan a question, and
acts on the answer. Nothing in the plan decides that the workflow runs; the branch does.

```yaml
# .github/workflows/release.yml
on:
  push:
    branches: [main]

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }        # the window is measured over history
      - run: cargo install aplan        # or a pinned binary
      # The plan's own gate first: a version derived from a plan that does not
      # validate is a number nobody should act on.
      - run: aplan check && aplan verify
      - id: v
        run: echo "next=$(aplan version)" >> "$GITHUB_OUTPUT"
      - run: cargo release "${{ steps.v.outputs.next }}" --no-publish --execute
```

Two properties this keeps that a trigger would not. The workflow runs **on the default
branch only**, so a fork's pull request cannot reach it -- the bound is GitHub's, enforced
where such bounds belong. And `aplan version` is a *read*: if the plan is wrong the step
fails and nothing has been published, where a trigger inside `close` would already have
fired by the time anything could disagree.

`release-please` is the same shape with a footer instead of an argument, which ADR-0040
decision 2 records as a demonstrated consumer rather than an output mode:

```sh
git commit -m "feat: x" -m "Release-As: $(aplan version)"
```

## What was ruled out, and why

- **A config key such as `on_close.command`.** Top-level, so it obeys HLD 5.5's letter, and
  refused for decision 1: the firing would be the task file's to decide. It would also make
  `close` non-inert for MCP, which is decision 2.
- **A `close --then <command>` flag.** The command comes from argv, so the trust argument
  does not apply -- and neither does the need. `aplan close A-1 && something` is already
  that, written where the person reading it expects to find it. A flag would buy nothing and
  would put a spawn inside a mutation, which is where one is hardest to notice.
- **An exit code meaning "a release became closeable".** It overloads ADR-0007's four-code
  contract, where the codes are about *this invocation*, with a fact about the plan. `aplan
  ready` already reports a closeable cut, and a workflow can ask it in a line of shell.
- **Emitting a vendor form** (`Release-As:`, a `cargo-release` invocation). Already refused
  by ADR-0040 decision 2, and the refusal is the same one: teaching this tool what
  release-please is.

## Consequences

`close` stays inert, and the MCP crate's stated rule keeps the meaning it was written with:
the gate decides whether a claim is true, and nothing downstream of it acts.

A person who wanted release-please driven from a closure writes the workflow above. That is
a real cost -- a file they did not have to write -- and it is the cost of the boundary rather
than an oversight. It is written down here so the next person asking finds the answer and
the alternative in the same place.

This decision is what `verify --execute` being off by default, and `close`'s three refusals
above, already imply -- said once for the whole tool rather than arrived at again per feature.
A future key that wants to *do* something rather than *observe* something is refused by this
ADR and not by a fresh argument.

Nothing enforces it. Reverting this decision by adding a key or a flag would leave every test
in the workspace green, which is the state `policy.rs` already calls "not a policy". Filed as
`A1-354` rather than folded in here, because deciding the policy and building the gate that
holds it are different work and the decision should not wait for the gate.
