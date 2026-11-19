---
id: M-hardening
title: Integrity and hardening
phase: 2
status: todo
blocked_by:
- M-v0.8
- A2-069
- A2-087
- A2-127
- A2-131
- A2-133
- A2-141
- A2-145
- A2-162
- A2-165
- A2-170
- A2-171
- A2-172
- A2-173
- A2-176
- A2-180
tags:
- milestone
- hardening
---

## Why

Integrity work that is not yet a reported wrong answer: diagnostics and generated guidance that
can mislead, ratification and precedence edge cases, and the cost of resolving test evidence
inside the plan gate. It also holds what an audit of v0.7 found below that bar: MCP replies lost
when a client closes its input, the TUI ranking its work on keys that change only the selection,
CI that skips the tests reading a changed plan or README, the YAML limits not yet proven
through product reads, real-terminal tests that fail when the host is busy, and nine TUI timings
the reference host cannot yet budget.

## Exit criteria

- [ ] every member is done or explicitly dropped with its reason

Sequenced after `M-v0.8` on purpose, so `aplan ready` lists the work in priority order. Move a task into an earlier cut with `aplan mv` to pull it forward.
