# TUI demonstrations

`tui-work-loop.tape` is the reproducible source for `tui-work-loop.gif`. It drives the real aplan
binary through a pseudo-terminal; it is not a mockup or a hand-edited animation.

From the repository root:

```sh
mbx build -p aplan
vhs validate docs/demos/tui-work-loop.tape
vhs docs/demos/tui-work-loop.tape
```

The tape requires VHS, Git and `sed`. This artifact was generated with VHS 0.12.1. VHS uses its
own terminal recorder plus FFmpeg; `vhs validate` checks the tape without executing it.

The tracked input is `docs/fixtures/tui-work-loop`. A run copies it into a unique directory under
`~/.tmp`, initializes a disposable Git repository, and removes only that validated scratch path on
success. The fixture contains no configured programs or network operations. The scenario first
shows a real `APLAN009`, repairs and commits the fixture inputs, returns to a clean diagnostic run,
then closes the task through the guarded TUI evidence workflow. The generated GIF is a product
demonstration, not evidence for the fixture's task or a replacement for the executable PTY tests.
