# ADR-0020: An outcome is superseded, never reopened

Date: 2026-08-14
Status: Accepted
Tasks: `A1-130`
Extends ADR-0014's status model. Supersedes nothing.

## Context

`APLAN018` warns when a closed task's traced section changes. Its remedy has two
branches: re-review, and if the outcome still holds, `aplan close --restamp`;
otherwise **"reopen the task"**.

There is no reopen. `drop` explicitly refuses to revive terminal work, and
hand-editing `status: done` back to `todo` is the thing this tool exists to stop.
The fifth pilot found it by reading the diagnostic and looking for the command
(pre-public adoption evidence). We published a remedy that is not executable.

So the question is whether to build reopen or to name something else.

## Decision

**There is no reopen, and the supported repair is `aplan supersede <id>
<title>`,** which files the successor and re-stamps the predecessor as reviewed.

**1. Reopening would assert something false.** The work *was* done. Its commit
really did touch those paths; its test really did pass; the cut it completed
really did complete. What changed is the ground underneath, afterwards. A plan
that moved the task back to `todo` would be claiming the work never happened,
which is a worse record than the one it replaced.

**2. Reopening is not local, and cannot be made local.** A reopened member of a
finished cut makes that cut's `done` false, which makes every downstream cut's
premise false. `APLAN007` would then report a done milestone depending on
unfinished work: an error about a plan that was correct until the mutation. The
alternative, reopening the task and silently leaving the cut closed, is a plan
that contradicts itself.

**3. Evidence has nowhere good to go.** Keep it and a `todo` task carries proof
of work it has not done; clear it and the record of what was once proved is
destroyed. Superseding needs neither: the predecessor keeps its evidence because
the evidence is still true about the moment it describes.

**4. The re-stamp is honest, and necessary.** A fingerprint records that the task
was *reviewed against this content*, not that the outcome is still valid, which is
why `--restamp` is the remedy for a drift that still holds. Superseding is also a
review, and the review's conclusion is the successor. Without the re-stamp the
warning would nag forever about a drift somebody has already dispositioned, and a
warning nobody can clear is one everybody learns to ignore.

**5. The successor inherits `paths` and `traces`, and nothing else.** Both are the
predecessor's own answers to questions the successor asks again: the same code,
the same section, which is what drifted. It carries no evidence, no blockers, and
no tags, because those would be guesses. It is `todo`, in the predecessor's phase.

## Consequences

- `APLAN018`, the rule catalogue, `--help`, `README.md` and `HLD.md` name an
  operation that exists.
- A finished cut stays finished. That is the point: superseding changes nothing
  about the past, so nothing downstream of it needs re-deciding.
- The link between predecessor and successor lives in the successor's body, not in
  a new field. It is prose because nothing checks it, and a field nothing checks
  is a claim this tool would have to distrust; ADR-0013's "no new field" applies.
- Two writes, and everything fallible happens before either of them: the plan is
  validated, both file contents are built, and the predecessor is probed for
  writability by opening it without truncating. An unwritable predecessor is a
  refusal that writes nothing, which is what "refuse before writing" means
  everywhere else here. The successor is written first and the re-stamp second,
  so an I/O failure between them leaves an extra `todo` task and an open drift
  warning, both of which `check` reports; the other order would leave a task
  silently re-stamped with nothing recording why.
- **There is no lock, here or anywhere in this tool.** A concurrent edit of the
  predecessor between the read and the write loses one of them, exactly as it
  does for `close` and `amend`. The pre-flight is a check, not a lock, and a lock
  over a directory of files that git already versions would be a new failure mode
  (a stale lock file) traded for a race that git recovers from.
- **A cut is not superseded, its members are.** A milestone's `done` means its
  members are terminal, so a successor to one would be work the cut does not
  contain, filed into no cut. It is refused, naming the two things that are not
  refused: supersede the member, or add the work to a live cut.
- It is CLI-only, like `amend`. The MCP surface is six tools chosen so an agent
  can pick work and close it; the repairs are for a person deciding that a
  finished thing stopped being enough, which is not a judgement to hand an agent.
- If a task genuinely was never done, that is not this. It is a mistaken closure,
  and the honest repair is to fix the record by hand and say so in the commit,
  which no command should make easy.
