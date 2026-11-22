---
id: M-v0.8
title: Correctness and the most reported adoption failures
phase: 2
status: todo
blocked_by:
- M-v0.7
- A2-034
- A2-050
- A2-051
- A2-052
- A2-053
- A2-054
- A2-055
- A2-056
- A2-062
- A2-081
- A2-085
- A2-091
- A2-095
- A2-096
- A2-097
- A2-099
- A2-101
- A2-106
- A2-122
- A2-140
- A2-142
- A2-147
- A2-152
- A2-153
- A2-156
- A2-157
- A2-160
- A2-161
- A2-182
- A2-183
- A2-192
tags:
- milestone
- v0.8
---

## Why

The remaining defects where the tool gives a wrong answer, accepts what it should refuse, or
writes a plan its own gate rejects, plus the adoption failures reported most often: a stale
or mismatched build judging a plan, generated agent contracts that block a correct plan, claims
that outlive the work, and drift that passes before the commit and fails after it.

## Exit criteria

- [ ] every member is done, with a regression test for each corrected wrong answer

Sequenced after `M-v0.7` on purpose, so `aplan ready` lists the work in priority order. Move a task into an earlier cut with `aplan mv` to pull it forward.
