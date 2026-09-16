# ADR-0021: A trace that leaves the repository is declared, or refused

Date: 2026-08-17
Status: Accepted
Tasks: `A1-143`
Extends ADR-0015's anchor model. Supersedes nothing.

## Context

`traces` resolution is a filesystem read against `traces_root`, and nothing constrained where the
path could point. The fifth pilot (`A1-140`, pre-public adoption evidence item 4c) probed what a
fleet actually does with that, and the result was the opposite of what it first looked like:

```
$ aplan add "..." --traces '../p0000007/docs/STATUS.md#phase-1--single-process-mvp'
would write ZZ-001  todo  ...        # APLAN008 did NOT fire: the anchor resolved
```

The trace does not check *nothing*. It checks **fully**. Verified against a scratch sibling: a bad
cross-repo anchor is refused with `APLAN008`; `detect_drift` fires across the boundary when the
sibling's section changes; and with the sibling absent, `check` fails hard with "traces target does
not exist".

So a cross-repo trace is an **undeclared hard dependency** on another repository being present at
an exact relative path and ref. The fleet's CI checks siblings out at `../<name>` for `cargo`, so
such a trace would appear to work there -- and then fail the day a job runs without the sibling, or
raise a drift warning against a sibling that legitimately moved to a different ref. Nothing
declares the dependency, so nothing warns until it fails: **a plan that passes on one machine and
not another, for a reason the plan does not state**, which is the failure this tool exists to
prevent.

There is a second reader of the same fact. A task file can arrive in a fork's pull request, and an
unconstrained `traces` target is a path this tool will open and read: `/etc/passwd#anything` is a
trace. `A1-041` made exactly this argument for `url` evidence when it refused every scheme but
`https`, on the grounds that the adopter's configured command cannot defend itself and only the
tool can decline to hand it the target.

## Decision

**A `traces` target that resolves outside `traces_root` is an error (`APLAN034`) unless the
repository declares its root in `traces_external`.**

**1. Refused by default.** The repository-scoped model is deliberate (HLD non-goals), and the
default has to be the safe one: a plan that reads another repository's files is unusual enough that
saying so costs one line, while the failure it prevents is silent until it is not.

**2. Declared roots resolve exactly as they do today**, including drift across the boundary. The
declaration does not weaken any check; it records that the repository intends to depend on the
sibling.

**3. The absence of a declared root is a configuration error, not a finding.** `traces_external`
naming a directory that is not there stops the run with exit `2` per ADR-0007, naming the key. The
tasks that trace into it are not at fault, and reporting them would be reporting the symptom.

**4. The test is lexical *and*, where the file exists, followed through symlinks.** Whether a
target leaves the repository has to be answerable from the plan and the config alone, or the rule's
answer depends on whether the sibling happens to be checked out -- the very dependency it exists to
surface. But a symbolic link committed inside the repository is lexically inside it and reads
whatever it points at, and a task file and the link can arrive in the same pull request, so where
the target exists the link is followed too. Both tests must pass.

**5. The boundary is the repository, not `traces_root`.** They are usually the same directory and
are not required to be: under `traces_root: docs`, a trace of `../README.md` climbs out of the root
and stays inside the repository. The repository is where `.aplan.yml` sits; with no config to
locate, the root is the only boundary there is.

**6. A declaration is resolved against the config file's directory**, so it means the same thing
however a run is invoked. Resolving it against `traces_root` instead lets `--traces-root docs`
redefine `../sibling` as `<repo>/sibling`, and a config key whose meaning a flag can change is not
a declaration.

**7. Nothing else changes.** An in-repository trace, however it is spelled (`./docs/x.md`,
`docs/../docs/x.md`), is untouched, and `APLAN008` remains the rule about whether an anchor
resolves.

## Consequences

- The comparison is done in absolute, lexically normalised terms so that a root, a declaration and
  a target meet in one frame. What each of them *is* -- how `traces_root` is spelled, what the
  config declares -- is unchanged, and so is every path this tool prints.
- A repository that traces a sibling gains one config key and loses the surprise; a repository that
  does not is unaffected, which is most of them.
- The rule cannot tell "a sibling this fleet intends" from "a path a fork's pull request wrote".
  That is the point of the declaration: the repository decides once, in a file it controls, rather
  than per task file.
- The multi-repo edge stays where the pilot left it. `blocked_by`, `commit` and `test` evidence,
  cross-repo cuts and portfolio rollup remain the documented deferral; this ADR only stops the one
  mechanism that crossed the boundary silently.
