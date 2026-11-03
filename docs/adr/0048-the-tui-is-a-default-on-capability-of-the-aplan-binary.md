# ADR-0048: The TUI is a default-on capability of the aplan binary

Date: 2026-10-14
Status: Superseded in part by ADR-0060
Tasks: `A1-419`

## Context

Aplan needs a substantial interactive interface before its public release without turning every
screen into another owner of plan meaning. The CLI, MCP server and TUI must share the same typed
application operations, diagnostics, graph projections and mutation gates. The rendering layer
still needs terminal layout, buffers, widgets, event decoding and safe terminal ownership.

There are two independent choices. The dependency choice is between a maintained terminal renderer,
an additional application framework and a renderer built here. The delivery choice is between one
unconditional binary, a feature-gated binary and a companion executable. A graph-layout crate is a
third concern: it must not enter merely because its scene API resembles what an interactive client
needs.

The spike measured the minimal and default renderer graphs, built candidates on the workspace MSRV,
tested the current external graph-layout release, and linked each packaging shape into a detached
copy of the real workspace. Detailed version and host measurements are evidence for this
decision, not architecture, and are omitted from the public technical record.

## Decision

**1. Ratatui with its Crossterm backend is the terminal rendering boundary.** Disable Ratatui's
top-level defaults and enable only its Crossterm integration. Consume Crossterm through Ratatui's
selected re-export so the renderer and event types cannot drift onto two backend versions. The
resolved backend defaults, including underline-color support, are accepted and recorded rather
than described as removed.

**2. Aplan owns the application architecture.** `aplan-app` remains the typed operation boundary.
The TUI adds a small explicit model/update/view loop, typed effects and view state. It does not add
a component framework, declarative application runtime or second domain model. Ratatui owns
terminal mechanics; aplan owns plan semantics. Building another general terminal renderer here is
also rejected.

**3. The standard product is one binary with a default-on `tui` feature.** `aplan-tui` is an
internal library crate and an optional dependency of the composing `aplan` crate. The `aplan`
crate's default `tui` feature enables it. Standard builds, installation and release artifacts will
carry `aplan tui` in the same executable as the other commands. Release automation will name and
test the feature explicitly instead of relying only on its default status.

An explicit `--no-default-features` source build omits the TUI library and terminal stack. Its
parser retains a visible `tui` stub that explains how to enable the capability, and `facts` reports
whether the capability was compiled. This headless form is supported for constrained source
builders; it is not an official release artifact.

**4. There is no companion executable.** A sibling binary would preserve the CLI's bytes but add
artifact discovery, subprocess handoff, packaging omissions and possible version skew. It also
conflicts with the existing one-static-binary release contract. The measured pair was larger in
aggregate than the integrated result. A feature-free build supplies dependency isolation without
adding those runtime failure modes.

**5. The aplan lane layout remains the graph engine.** Extract its geometry into a semantic scene
consumed by both the CLI painter and TUI viewport. The scene carries stable node and edge identity,
bounds, viewport iteration, hit testing and omission notes while `GraphView` remains the only owner
of graph meaning. Do not add a general layered-layout dependency until a released candidate meets
the workspace MSRV and terminal-width contract and a measured user task demonstrates that lanes
cannot express a needed crossing, orientation or overview.

**6. Every further TUI dependency is opt-in.** Layout caching, macros, async event streams,
input/search widgets, fuzzy matching, clipboard support, effects, snapshot libraries and PTY
harnesses enter only with a named behavior, license and MSRV review, resolved feature graph and a
focused test or benchmark. Test helpers remain development-only. Existing workspace availability
does not by itself admit a crate into the TUI architecture.

## Consequences

- A normal installation has one discovery path and one versioned artifact. No subprocess boundary
  can disagree with the parent process's root, configuration or trust selection.
- The default release pays the small measured linked-size cost of the terminal stack. Source
  builders who do not need it can recover the measured pre-TUI binary and dependency boundary.
- Default and headless builds have deliberately different capabilities, so command help, `facts`,
  tests and release checks must name which capability is present. The feature is not permission to
  let official artifacts omit the TUI.
- The reducer and effect loop remain straightforward to test without terminal codes or an async
  runtime. Terminal ownership and PTY behavior still require their own boundary tests.
- The graph scene can support selection and hit testing without parsing rendered glyphs, copying
  the plan graph or coupling `aplan-output` to Ratatui.
- Dependency proposals have an evidence bar. Convenience and popularity are not sufficient, and a
  hard MSRV failure stops performance comparison until compatibility exists.

## Alternatives

| option | why not |
|---|---|
| Ratatui default features | Admits calendar support, layout caching and macros without a measured product need; disabling top-level defaults preserves the required renderer and backend |
| `tuirealm`, `iocraft` or Cursive | Adds or replaces the application/view model while aplan already has typed operations and needs only a small reducer; one candidate also resolves a second Crossterm line |
| Hand-build the terminal renderer | Reimplements layout, buffers, widgets and terminal integration without making aplan's domain behavior more specific or safer |
| Non-optional integrated dependency | Produces the same official artifact but removes the measured headless source build for no product benefit |
| Default-off TUI feature | Makes an ordinary install omit a standard capability and creates two classes of normal users |
| Companion executable | Adds artifact and handoff failure modes, weakens `cargo install aplan` as the complete install story and is larger in aggregate |
| Adopt a general DAG renderer now | The evaluated release fails the workspace MSRV and terminal-width contracts, while the existing lane renderer is smaller on the real labelled graph and already expresses the required interaction scene |
