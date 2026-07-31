# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately via GitHub's **Report a vulnerability**
button (repository → *Security* → *Advisories*), not a public issue. We aim to acknowledge
within a few days.

## Threat model

`aplan` reads a repository's plan; `aplan verify` resolves evidence against git, and
`aplan verify --execute` runs named tests. The security-relevant invariants:

- **`aplan check` is hermetic.** It opens no network sockets and spawns no subprocess, it
  reads files and computes. This is enforced by a test (`aplan-e2e/tests/hermetic.rs`): the
  crates the check path lives in name no process, socket, or git API, and `unsafe_code` is
  forbidden workspace-wide, so there is no raw-syscall escape hatch. The `jsonschema`
  dependency is pinned `default-features = false` to disable remote schema resolution.
- **A task file can never run a command.** `aplan verify --execute` runs a test, and a task
  file can arrive in a fork pull request, so the task file supplies a test *name*, never a
  command. The name is substituted, as a *whole* argument, into a runner template defined
  only in the trusted top-level `.aplan.yml`, and the command is spawned with an argv vector
  and no shell. A runner template that would splice the name inside a larger argument (the
  one way a shell-using runner could be turned against a task file) is rejected when the
  config loads. So the worst a hostile name can do is fail to match a test. Pinned by
  `a_hostile_test_name_cannot_inject_a_command` in `aplan-evidence`; a Kani proof of the
  same property is planned. `--execute` is off by default and CI must not enable it for
  fork pull requests.
- **`url` evidence is not fetched by default.** When enabled it will be HTTPS-only with
  redirects disabled (SSRF hardening), a timeout, a body-size cap, and a content-addressed
  cache.

A report that breaks any of these, a way to make `check` reach the network, or a task
file that causes command execution, is a genuine vulnerability, and the most valuable
kind to send.

## Supported versions

Pre-1.0: only the latest commit on `main` is supported.
