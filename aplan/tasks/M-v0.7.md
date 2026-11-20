---
id: M-v0.7
title: Security, safe parsing and gates that tell the truth
phase: 2
status: todo
blocked_by:
- A2-009
- A2-016
- A2-017
- A2-018
- A2-019
- A2-020
- A2-021
- A2-022
- A2-023
- A2-024
- A2-025
- A2-026
- A2-027
- A2-028
- A2-079
- A2-137
- A2-164
- A2-166
- A2-167
- A2-168
- A2-169
- A2-177
- A2-178
- A2-179
- A2-181
tags:
- milestone
- v0.7
---

## Why

The first public increment after v0.6.0. It clears the supply-chain advisories that turned the
scheduled CI red, keeps the source clean under the next stable compiler, finds the cause of the
hosted TUI latency regression, stops the render gate failing on slower hosted runners, replaces
the C-derived YAML bridge with the safe Rust parser, and fixes the reported findings where a
gate passed, or a report said something false, for reasons the user could not see. It also
closes what an audit of the increment found: the parser switch's gaps, the files it refuses that
v0.6.0 read, a README recipe that failed as written, a per-task walk of the roadmap, what a
second audit found in those fixes (`A2-177`), what an audit of the tag rule found (`A2-179`), and
a progress row that can show a run's end without its start (`A2-178`). It moves every open issue
and the open pre-public backlog into this plan, and ends with the v0.7.0 release.

## Exit criteria

- [ ] every member is done and v0.7.0 is published from one validated source revision
- [ ] scheduled CI, scale and latest-stable workflows are green on the current hosted images
