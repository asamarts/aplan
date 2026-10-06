# ADR-0039: Version consistency is a shell gate in this repository, not a feature of this tool

Date: 2026-09-20

Status: accepted

## Context

Two questions hide under "version checking", and this plan had been treating them as one
(`A1-318`):

- **Which number should the next version be?** A semantic question about what changed.
  That is `A1-311` through `A1-315`, and it needs a plan to answer.
- **Do the places that state a version agree?** A syntactic question about files. It needs
  no plan at all.

HLD section 17 promised the second and it did not exist. The sibling tool `alint` has it:
`check-version-pins.sh` plus a one-line `.alint.yml` rule of `kind: command`. The question
was which tool owns it here, and `A1-318` said to write both shapes and answer with the diff
in front of you.

## What the measurement showed

**Writing it as an `alint` rule is writing a shell script.** `alint`'s rule is four lines
wrapping `bash ci/scripts/check-version-pins.sh`; the substance is the script. So the choice
is not "alint or aplan", it is "does this repository take a dependency and a config file in
order to invoke one shell script that CI can invoke directly".

**Copying `alint`'s script would have been wrong.** It asserts install snippets equal
`[workspace.package].version`. Correct there; wrong here, because this workspace carries
`X.Y.Z-dev` between releases (`A1-060`) and no published release ever equals it. The gate
must compare against the newest release **tag**. That difference is invisible until you run
it, and it would have failed on every commit.

**Teaching this tool to do it would contradict ADR-0026 and ADR-0034.** It means parsing
`action.yml`, `README.md` and `Cargo.toml` for version strings -- files this tool has no
business understanding. `A1-313` already settled the shape for the semantic half: the version
is read by a *declared command*, not parsed. A declared command is a shell script. All three
roads arrive at the same artifact.

**And the promise as written is not implementable**, which is the finding worth more than the
decision. Two attempts failed on this repository's own text:

- *Scoping by file fails.* `docs/upgrading.md` line 4 says a repository needs a feature
  "newer than `v0.3.0`" -- must move when v0.4.0 ships. Line 5 says most of the page "shipped
  in `v0.3.0`" -- must not. Same string, same file, opposite obligations.
- *Scoping by syntax fails.* Matching every `uses: <owner>/aplan@vX.Y.Z` found four, and all
  four were quotations: a CHANGELOG entry about the v0.2.0 pin problem, two task bodies, and
  a pilot log recording what an adopter ran. A record of an instruction is not an
  instruction, and nothing in the string says which it is.

## Decision

**1. A shell gate in this repository, run by CI directly.** `ci/scripts/check-version-pins.sh`,
in the `spec` bundle beside the other contract gates. No `.alint.yml` and no dependency on a
sibling tool to run one script.

**2. It is scoped by what a document is for**, not by file glob or regex. In scope are the
documents that tell an adopter what to run: `README.md`, `docs/upgrading.md`, `action.yml`.
Excluded, each because it is a record rather than an instruction: `CHANGELOG.md`,
`aplan/tasks/**`, pre-public adoption evidence, `docs/adr/**`.

**3. It compares against the newest release tag reachable from HEAD**, selected exactly as
ADR-0038 selects the derivation window -- by history, never by tag date, since this
repository carries `archive/*` tags that are not ancestors of `main`.

**4. HLD section 17's promise is amended rather than kept.** It claimed a gate over "any
documented version string"; what exists is a gate over install pins in instructing documents.
The narrower sentence is the true one and the section now says so.

**5. An adopter who does not use `alint` gets nothing from this, and that is correct.** This
is a gate about *this repository's* documentation, not a feature of the tool. An adopter with
the same problem writes the same fifty lines, and the thing this project owes them is the
semantic half -- which version the next release must be -- not a regex over their README.

## Consequences

The syntactic half is closed here and is deliberately not a product feature, so `A1-313` and
the rest of the versioning cut are unambiguously about the semantic half. A stale pin is
proved to fail the gate. The cost is a gate whose scope lists must be maintained by hand: a
new instructing document is not covered until somebody adds it, which is the same cost
`alint` pays and the reason both lists carry their reasons inline.
