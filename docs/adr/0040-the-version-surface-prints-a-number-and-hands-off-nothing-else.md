# ADR-0040: The version surface prints a number, and hands off nothing else

Date: 2026-09-21

Status: accepted; **its decisions 1 and 3 are superseded by ADR-0041**, which names the
node positionally rather than with `--of`, and lets the rung follow the argument rather
than putting every invocation in `verify`. Decisions 2, 4 and 5 stand.

Extends ADR-0034 (the ladder and the floors), which could not decide the surface because
the surface depended on things nobody had measured.

## Context

`A1-311` decided the ladder -- `none < fix < feature < breaking` -- and the floors a level
supplies. `A1-320` is the join: three investigations have now reported, and the surface is
designed against them rather than around them.

| finding | what it settled |
|---|---|
| `A1-317`, ADR-0038 | a release carries its version, recorded at close; the window with no release node is the newest release tag reachable from HEAD |
| `A1-318`, ADR-0039 | the *syntactic* half -- do the files agree -- is a shell gate here and deliberately not a feature, so this surface is the semantic half only |
| `A1-319`, the spike | both hand-off forms work: `cargo-release` takes an argument, `release-please` takes a `Release-As:` footer, and the squash-merge caveat is void |

## Decision

**1. The surface is one question asked of one node, and the answer is a version.**

```
aplan version next            what the next release must be
aplan version next --of <id>  what that cut, or that task, owes on its own
```

`--of` rather than a second command because `A1-313` already requires "an answer that can
be asked of any level", and a cut and a release are the same kind of node (ADR-0022).

**2. It prints a bare version and nothing else, and no vendor forms.** The spike proved
two consumers, and both are one line of shell away from a bare number:

```sh
cargo release "$(aplan version next)" --no-publish        # an argument
git commit -m "feat: x" -m "Release-As: $(aplan version next)"   # a footer
```

Teaching this tool to emit `Release-As:` would be teaching it what release-please is, which
is the refusal ADR-0034 already made about writing versions and ADR-0026 made about query
languages. The forms are recorded in the spike as *demonstrated consumers*, not as output
modes. A `--format json` carrying the number and what raised it is the machine form, because
that is this tool's own vocabulary rather than somebody else's.

**3. It resolves in `verify`, not `check`.** The window is "everything closed since the
newest release tag reachable from HEAD" (ADR-0038), which reads git. Where the plan models a
release node the window is that node, and then `check` can answer -- but one command must not
sit on two rungs depending on the plan's shape, so the whole thing is `verify` and says why
in a shallow clone, as `APLAN045` does.

**4. The current version is read by a declared command**, never parsed. `.aplan.yml` gets a
key under `version`, invoked the way `test.list` and `url.check` are. A repository that
declares none gets the derivation and no comparison, with the reason stated rather than a
silent skip. This is ADR-0034's refusal held: this tool does not know what a `pyproject.toml`
means.

**5. Nothing here writes a version.** `ci/scripts/release.sh` already refuses a tag that
disagrees with `[workspace.package].version` and refuses to tag a pre-release at all, which
is what `cargo-dist` needs from us and the reason the spike could not drive it: it owns no
version to be handed one.

## What the spikes ruled out, and why

- **`cargo-dist` as a hand-off target.** It reacts to a tag and expects the manifests to
  already agree. There is nothing to hand it. A refused integration, not a gap.
- **Requiring a squash-merge workflow.** The `Release-As:` caveat was tested in the shape it
  was stated -- a plain merge of three commits with the override on the oldest -- and
  release-please 17.11.2 honoured it. The design is not narrowed to squash-merging
  repositories, which it would have been on the documentation alone.
- **An `.alint.yml` for the syntactic half** (ADR-0039), which would have made a dependency
  of a sibling tool out of one shell script.
- **Modelling this repository's own releases** (ADR-0038), because two of its three closed no
  cut.

## Consequences

`A1-313` is confirmed against this and gains one dependency it did not have: its arithmetic
is "an aggregation up the hierarchy", and `A1-333` measured that the hierarchy reads zero
above a sub-cut. Deriving a version through a traversal that loses four-level work would
report a confidently wrong number, which is worse than reporting none.

One thing no task owned and now does: **recording the derived version into the release node
when it closes** (ADR-0038 decision 1). `A1-313` computes it and `A1-314` checks the artifact
against it; neither writes it down. Filed as `A1-339`.
