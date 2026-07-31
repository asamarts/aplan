# ci/ - self-hosted CI runner

aplan's CI runs entirely on a self-hosted GitHub Actions runner, so it consumes
zero GitHub-hosted minutes and incurs no Actions billing (the pre-public repository-boundary decision). This
directory holds everything that stands up and feeds that runner:

- `Containerfile` - the runner image (Debian + Rust toolchain + the agent).
- `runner/{setup,entrypoint,teardown}.sh` - container lifecycle.
- `scripts/*.sh` - one script per CI gate, called by the workflow and by the
  local `pre-push` hook, so the identical check runs in both places.
- `githooks/pre-push` - opt-in local gate bundle.
- `env.example` - config template (copied to the gitignored `ci/.env`).

## What runs where

`.github/workflows/ci.yml` targets `runs-on: [self-hosted, linux, aplan]` for
every job. There is no `ubuntu-latest` job, so nothing bills. If the runner is
offline, jobs queue as pending (and cost nothing) until it returns.

The gates: `fmt` (rustfmt), `clippy`, `test`, `spec` (gen-schema and gen-roadmap
`--check`), `style` (house-style lint), `dogfood` (`aplan check` + `verify` on
this repo's own plan).

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

Update the Rust toolchain or runner agent version: edit `Containerfile` (the
`RUST_VERSION` / `RUNNER_VERSION` args), then rebuild:

```sh
bash ci/runner/teardown.sh           # keeps the identity + cache volumes
bash ci/runner/setup.sh              # rebuilds the image, re-attaches the runner
```

After a `teardown` + `setup` (an image rebuild, say), the log may show
`A session for this runner already exists` for up to a minute or so. GitHub holds
the previous session briefly after the old container released it; the agent keeps
retrying and reaches `Listening for Jobs` on its own. Wait for that line before
concluding anything is wrong: re-registering with a fresh token at this point
fixes nothing and only churns the identity.

Fully remove the runner (drops the cargo caches too; the identity volume is kept
unless you also `podman volume rm aplan-runner-config`):

```sh
bash ci/runner/teardown.sh --purge
```

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

## Security notes

- The runner is a rootless `podman` container running as a non-root `runner`
  user, with a `--pids-limit` fork-bomb guard.
- The agent runs with `--disableupdate` and a baked-in toolchain, so versions
  change only by a deliberate image rebuild.
- **Tripwire:** this repository is private, so no untrusted fork pull request can
  run on the box. If aplan is ever made public, add a routing job (see the pre-public repository-boundary decision)
  that sends fork pull requests to an ephemeral hosted runner and only trusted
  events to the box, before making it public.
