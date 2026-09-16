# ci/ - self-hosted CI runner

aplan's CI runs entirely on a self-hosted GitHub Actions runner, so it consumes
zero GitHub-hosted minutes and incurs no Actions billing (the pre-public repository-boundary decision). This
directory holds everything that stands up and feeds that runner:

- `Containerfile` - the runner image (Debian + Rust toolchain + `cargo-llvm-cov` for
  the coverage gate + the agent).
- `runner/{setup,entrypoint,teardown}.sh` - container lifecycle.
- `scripts/*.sh` - one script per CI gate, called by the workflow and by the
  local `pre-push` hook, so the identical check runs in both places.
- `githooks/pre-push` - opt-in local gate bundle.
- `env.example` - config template (copied to the gitignored `ci/.env`).

## What runs where

`.github/workflows/ci.yml` targets `runs-on: [self-hosted, linux, aplan]` for
every job. There is no `ubuntu-latest` job, so nothing bills. If the runner is
offline, jobs queue as pending (and cost nothing) until it returns.

The gates: `fmt` (rustfmt), `clippy`, `doc` (intra-doc links, `RUSTDOCFLAGS=-D
warnings`), `test`, `spec` (gen-schema and gen-roadmap `--check`), `style`
(house-style lint), `coverage` (per-crate line-coverage floors), `dogfood`
(`aplan check` + `verify` on this repo's own plan).

**`.github/workflows/scale.yml` is the exception, and it is not on the push
path.** It runs `ci/scripts/scale.sh` nightly (04:17 UTC) and on
`workflow_dispatch`, on the same self-hosted runner. That gate generates 24,000
task files and runs for about twenty seconds -- a 29s job on the runner,
measured 2026-09-09 -- which is not a per-commit cost, so it is
not in `ci.yml` and not in `preflight.sh`, and the test it runs is `#[ignore]`d so
an ordinary `cargo test` skips it (`A1-229`).

What it asserts is a *ratio*, and it asserts it on `load_plan` and `validate`
**separately**: `check` on a fixed number of tasks spread across four cut widths,
slowest against fastest, rather than a millisecond budget. The two phases are
timed apart because loading is O(tasks) and identical in every shape -- 87% of the
total here -- so a ratio on the sum would have needed a *25x* regression in
`validate` before it moved, where the attributed one fails at 4x. A
wall-clock floor on this box would be flaky, and worse than flaky -- `A1-228`
published an 11% difference between two plan shapes that was contention from a CI
run on this same machine, and re-measuring with the runner idle reversed the sign.
A ratio survives a busy box because every shape is slowed together. Run it by hand
with `ci/scripts/scale.sh`, or
`gh workflow run scale.yml`.

`coverage` is its own job with a longer timeout: `cargo llvm-cov` rebuilds the
workspace instrumented into its own target subdirectory, sharing nothing with the
ordinary test build, so folding it into `test` would roughly double that job for
every push. Its disk lands on the same `/build`-backed volume as the rest.

## One-time provisioning (on the runner host)

Requirements: an always-on Linux host with `podman` (rootless is fine), roughly
2 cores and 4 GB free. This can be any machine you keep on.

1. Clone the repo on the host and `cd` into it.
2. On the repository owner's account, mint a runner registration token:
   Settings -> Actions -> Runners -> New self-hosted runner (Linux). Copy the
   token from the `./config.sh --token <TOKEN>` line.
3. Configure and start:
   ```sh
   cp ci/env.example ci/.env
   # edit ci/.env: set GITHUB_TOKEN to the token from step 2
   bash ci/runner/setup.sh
   ```
   `setup.sh` builds the image, creates the persistent volumes, starts the
   container, and installs a `systemd --user` unit (with linger) so the runner
   returns after a reboot.
4. Confirm the runner shows **Idle** under Settings -> Actions -> Runners, then
   push a commit and watch the jobs run on the box:
   ```sh
   podman logs -f aplan-runner
   ```

The registration token is short-lived and used once; the durable runner identity
is persisted to the `aplan-runner-config` volume, so restarts and reboots need no
new token.

## Where the runner's disk usage lands (`/build`)

All of the runner's heavy disk usage, the image layers, the container's writable
layer (which includes the `_work` checkout tree), and the cargo registry and
`CARGO_TARGET_DIR` volumes, lives wherever this host's rootless podman is
configured to store data, its `graphroot`. `setup.sh` uses podman *named* volumes
rather than host bind mounts precisely so they inherit that location automatically,
with no path hard-coded in the repo.

On the current runner host that `graphroot` is the dedicated `/build` drive, set
once in `~/.config/containers/storage.conf`:

```
[storage]
driver = "overlay"
graphroot = "/build/containers/rootless"
```

So the runner's volumes resolve to `/build/containers/rootless/volumes/aplan-runner-*`
and nothing lands on `/` or `/home` (the same arrangement the alint runner already
uses on this host). Verify it before relying on it:

```sh
podman info --format '{{.Store.GraphRoot}}'   # expect a path under /build
df -h "$(podman info --format '{{.Store.VolumePath}}')"
```

A fresh runner host must point `graphroot` at its build drive the same way before
running `setup.sh`; otherwise the volumes default to `~/.local/share/containers`
on the home filesystem.

## Day to day

```sh
podman logs -f aplan-runner          # follow runner output
podman restart aplan-runner          # restart (e.g. after editing .env)
systemctl --user status container-aplan-runner.service   # reboot-persistence unit
```

Update the Rust toolchain, the runner agent, or `cargo-llvm-cov`: edit
`Containerfile` (the `RUST_VERSION` / `RUNNER_VERSION` / `LLVM_COV_VERSION`
args), then rebuild:

```sh
bash ci/runner/teardown.sh           # keeps the identity + cache volumes
bash ci/runner/setup.sh              # rebuilds the image, re-attaches the runner
```

After a `teardown` + `setup`, or any `podman restart`, the log shows
`A session for this runner already exists` and the agent retries until GitHub
releases the session it is still holding. Measured at about three minutes on
2026-08-06, not the "up to a minute" this said before. It reaches
`Listening for Jobs` on its own and the registration is unaffected. Wait for that line before
concluding anything is wrong: re-registering with a fresh token at this point
fixes nothing and only churns the identity.

Fully remove the runner (drops the cargo caches too; the identity volume is kept
unless you also `podman volume rm aplan-runner-config`):

```sh
bash ci/runner/teardown.sh --purge
```

## Cutting a release

`main` carries a pre-release version (`0.3.0-dev`), on purpose: a staged build must never
report the same string as a published release, or a consumer pinning by `--version` pins
nothing (`A1-060`). So a release is an explicit bump, in this order:

1. set `[workspace.package].version` to the release version, and the six internal
   `path` dependencies in `[workspace.dependencies]` with it (a stale one fails the build
   loudly, not silently)
2. run a build so `Cargo.lock` records the new version: `release.sh` builds `--locked`, so
   a lockfile still naming the old one fails the release with
   `cannot update the lock file ... because --locked was passed`
3. commit both, then tag `vX.Y.Z` matching the version exactly
4. set the version back to the next `-dev` after the release lands

`ci/scripts/release.sh` refuses to build a tagged release while the version carries a
pre-release suffix, and says this. Skipping step 1 fails at the tag, not after publishing.

## Run the gates locally

```sh
bash ci/scripts/preflight.sh         # fmt + clippy + test + spec + style + dogfood
```

Enable the pre-push hook so this runs automatically before every push:

```sh
git config core.hooksPath ci/githooks
# bypass once: git push --no-verify
# skip a gate: PREFLIGHT_SKIP=test git push
```

## Confirming a push was actually checked

```sh
bash ci/scripts/pushed.sh            # waits for the run on HEAD and reports it
```

**Run creation is not immediate and is not bounded.** Measured on 2026-08-26, two
pushes to `main` waited **40 seconds** and **18 minutes** before a run object
existed at all, with the runner online and idle and nothing else queued. Querying
`actions/runs?head_sha=...` shortly after a push and finding `total_count: 0` is
therefore not evidence that the push failed to trigger anything, and twice that
day it was read as though it were.

**Do not dispatch by hand while waiting.** The workflow's `concurrency` group is
`CI-<ref>` with `cancel-in-progress`, so a `workflow_dispatch` and the push run it
was standing in for are in the same group: on `2f327d1` the dispatched run was
cancelled twenty seconds after it started, by the run it was meant to replace.
`pushed.sh` waits and reports; it never dispatches.

The local hook is what makes the wait unimportant. With `core.hooksPath` set,
`preflight.sh` has already passed before the push left the machine, so a late or
missing run delays the confirmation rather than the check.

## Running aplan in someone else's CI

Everything above is about *this* repository's runner. This section is about a
different repository running `aplan` as a gate, which is the case `README.md`
documents with the GitHub Action -- and the one path a cross-account adopter
cannot use.

`asamarts/aplan` is private, and a private repository's action is usable only
where its **Actions access policy** allows it. Measured on this repository, whose
owner is a personal account rather than an organisation:

```console
$ gh api repos/asamarts/aplan/actions/permissions/access
{"access_level":"none"}
```

`none` means no other repository at all, including ones the same account owns.
Raising it to `user` would let `asamarts/*` use the action; it can go no further,
because sharing beyond the owner is an organisation feature. So for a repository
under **another account** this is not a setting anybody can change and not
something a token fixes -- `uses: asamarts/aplan@v0.2.0` fails to resolve before
any of the adopter's plan is read. Two pilots hit it in a row, which is why it is
written down here rather than left to be rediscovered.

The four ways out, with what each costs:

| Path | Cost |
|---|---|
| Stage the binary into a self-hosted runner image | an image to build, and nothing else. No token, and provenance rests on an image the operator built |
| Download the release asset in the workflow | a cross-account fine-grained PAT in a repository secret, on every repository that runs the gate |
| `cargo install --git` | the same token, and a full build on every CI run |
| Commit the binary | a multi-megabyte opaque blob in the repository's history |

The first is what `p0000004` invented and `p00025` copied, and it is the one
described below. It suits a repository that has a self-hosted runner already or
wants one; a repository on hosted runners pays the second row's token instead,
and the same digest pin applies.

### Staging the binary

Get a binary. Either the published release, which is statically linked and comes
with its checksum -- but check first that it carries what your gate needs, because
the newest release is `v0.2.0` and everything since is on `main` only, which is
why the second pilot to do this builds rather than downloads:

```sh
gh release download v0.2.0 --repo asamarts/aplan \
  --pattern 'aplan-v0.2.0-x86_64-unknown-linux-musl.tar.gz*' --dir /tmp/aplan-dl --clobber
( cd /tmp/aplan-dl && sha256sum -c aplan-v0.2.0-x86_64-unknown-linux-musl.tar.gz.sha256 )
tar -xzf /tmp/aplan-dl/aplan-v0.2.0-x86_64-unknown-linux-musl.tar.gz -C /tmp/aplan-dl
```

or a build from a checkout, when the gate needs something no release carries yet.
It must be statically linked or the image will not start it, and
[`docs/upgrading.md`](../docs/upgrading.md) gives the containerised build that
works along with the two traps on the direct route.

Stage it into the image's build context. For a runner set up the way this file
describes -- rootless, built from a checkout on the host -- that is a directory in
the consumer's own repository:

```sh
install -m 0755 /tmp/aplan-dl/aplan ci/vendor/aplan
sha256sum ci/vendor/aplan
```

Where the image is instead built by a dedicated service account, install as that
account (`sudo install -o <user> -g <user> ...`) into whatever directory that
account's build context lives in. Both pilots differ here and neither is more
right; what matters is that the file is readable by whoever runs the build.

Keep the binary itself out of git and commit only a README beside it saying which
version is staged. Record that `sha256sum` in the consumer's `Containerfile` and
have the build check it, so the image says which binary it has rather than
whichever one was lying in the directory:

```dockerfile
ARG APLAN_SHA256=<the digest above>
COPY vendor/aplan /usr/local/bin/aplan
RUN echo "${APLAN_SHA256}  /usr/local/bin/aplan" | sha256sum -c - \
 && aplan --version
```

Where the staged binary is a build rather than `v0.2.0`, assert the *capability*
the gate depends on rather than the version string, so an image built from the
wrong binary fails at build time instead of during a run:

```sh
aplan facts --require test.covers --require APLAN024
```

`--require` is itself newer than `v0.2.0`, so on the published release the smoke
test is `aplan --version` and nothing finer. That is one more reason to build:
a staged binary that can state what it supports is checkable, and one that can
only state a version is not.

Bump the pinned version and the digest together; they are one fact written twice
and this is the arrangement's only place to get that wrong.

### On hosted runners, where there is no image to bake into

The second row of the table, spelled out, because "pays a token" is not something
a reader can act on. `gh` is preinstalled on GitHub-hosted runners, so the install
is the same three commands as above with a secret in front of them:

```yaml
- name: Install aplan
  env:
    GH_TOKEN: ${{ secrets.APLAN_RELEASE_TOKEN }}
  run: |
    gh release download v0.2.0 --repo asamarts/aplan \
      --pattern 'aplan-v0.2.0-x86_64-unknown-linux-musl.tar.gz*' --clobber
    sha256sum -c aplan-v0.2.0-x86_64-unknown-linux-musl.tar.gz.sha256
    tar -xzf aplan-v0.2.0-x86_64-unknown-linux-musl.tar.gz
    sudo install -m 0755 aplan /usr/local/bin/aplan
- run: aplan check --format github
```

Whichever row you took, `aplan check` fails the job on an error and not on a
warning, so an advisory rule the plan switched on deliberately gates nothing here
until `deny_warnings: true` is in the consumer's `.aplan.yml`. `README.md`'s
*In CI* section has the reasoning; the trap is that turning the rule on and
automating the gate are two decisions nobody makes in the same sitting.

Whichever row you took, `aplan check` fails the job on an error and not on a
warning, and 21 of aplan's 44 rules are warnings -- so a good deal of what it
checks annotates the pull request without gating it until `deny_warnings: true`
is in the consumer's `.aplan.yml`. `README.md`'s *In CI* section has the reasoning; the trap is that
turning the rule on and automating the gate are two decisions nobody makes in the
same sitting.

`APLAN_RELEASE_TOKEN` is a fine-grained PAT scoped to `asamarts/aplan` with
read-only Contents, and nothing else. Two things it is worth being clear-eyed
about before choosing this row. The secret is a standing cross-account credential
in every repository that runs the gate, which is the cost the image path does not
pay. And a workflow triggered by a fork pull request must not see it, so the gate
either runs only on trusted events or an outside contributor's pull request runs
no plan check -- the same routing question this repository has open for itself.

The `sha256sum -c` line is not decoration: it is what makes the download as
pinned as the staged binary, and the published `.sha256` asset is there to be
used. Every command in the block was run against the real release while this was
written, including the digest check, which was confirmed to fail on a wrong digest
as well as to pass on the right one. The workflow around them is the shape, not a
tested artefact: nothing here has run a GitHub-hosted job.

### What retires this

The repository going public, after which the action resolves for everyone and
this section is a historical note. That is settled as *not yet*, and the condition
is two things rather than a date: v1.0 readiness, and at least a month after it of
continuously planning real work with the tool -- a measurement, not a wait
(the pre-public repository-boundary decision, whose trigger the pre-public repository-boundary decision narrowed). So this is the documented path for the
foreseeable future rather than a stopgap about to disappear.

## Security notes

- The runner is a rootless `podman` container running as a non-root `runner`
  user, with a `--pids-limit` fork-bomb guard.
- The agent runs with `--disableupdate` and a baked-in toolchain, so versions
  change only by a deliberate image rebuild.
- **Tripwire:** this repository is private, so no untrusted fork pull request can
  run on the box. If aplan is ever made public, add a routing job (see the pre-public repository-boundary decision)
  that sends fork pull requests to an ephemeral hosted runner and only trusted
  events to the box, before making it public.
