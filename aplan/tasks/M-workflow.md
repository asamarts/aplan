---
id: M-workflow
title: Workflow ergonomics
phase: 2
status: todo
blocked_by:
- M-v0.8
- A2-108
- A2-109
- A2-110
- A2-111
- A2-112
- A2-113
- A2-114
- A2-115
- A2-116
- A2-117
- A2-118
- A2-119
- A2-120
- A2-134
- A2-135
- A2-136
- A2-139
tags:
- milestone
- workflow
---

## Why

Everyday workflow surfaces: trustworthy completion history, readable terminal Markdown, the
capture workflow over MCP, versioned structured MCP outputs, and bounded read latency across
every read surface, ending in an audit of the whole cut. That audit (`A2-117`) waits on the TUI
release-quality cut (`M-tui-quality`), as `A2-089` requires.

## Exit criteria

- [ ] every member is done, and the closing audit records no unresolved finding

Sequenced after `M-v0.8` on purpose, so `aplan ready` lists the work in priority order. Move a task into an earlier cut with `aplan mv` to pull it forward.
