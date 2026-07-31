# ADR-0010: The plan directory is visible, namespaced, and configurable

- Status: Accepted
- Date: 2026-07-27
- Deciders: kaminsod

## Context

The default plan directory was `plan/`, and its location was hard-coded as the `--root`
default. Two problems.

`plan/` is generic. Many repositories already have a `plan/`, `planning/`, or `docs/plan/`
for something else, so the default collides and carries no signal of which tool owns it.
Durable, hand-edited, PR-reviewed content belongs in a visible directory named for its tool;
machine-managed state can justify a hidden directory. `aplan` stores reviewable Markdown, so
its default should be visible and namespaced.

And the location was not configurable. HLD section 9 already listed a `root:` config key as
planned so a repo need not repeat `--root`; without it, a repo that wants a different location
must thread the flag through every CI invocation.

The change is cheap before the first release, so no consumer breaks, the same window ADR-0007
used to change the exit-code contract. `aplan`'s loader is
already directory-structure-agnostic (it walks the root recursively and a task's identity is
its `id` field, never its path), so the directory is dumb storage and only the default name and
its configurability are at stake.

## Decision

1. **The default plan directory is `aplan/`**: visible (it is durable, hand-authored, reviewed
   content) and namespaced (no collision with a repo's existing `plan/`, and an unambiguous
   owner).
2. **The location is configurable.** A new optional `root:` key in `.aplan.yml` sets it (a
   relative value is resolved against the config's directory, so the verdict does not depend on
   the invocation directory). Precedence, matching every other knob: the `--root` flag overrides
   the `root:` key, which overrides the built-in default. A team that prefers a hidden directory
   sets `root: .aplan` in one line. `schema`, `facts`, and `init` never read the config, so they
   never fail on a missing or invalid one.
3. **Task files stay flat, and identity stays in the `id` field, not the path.** The loader
   ignores directory structure, so nesting is free and purely for human browsing; the
   `roadmap`/`summary`/`ready`/`graph` commands are the real navigation. Encoding a mutable
   attribute (cut, status, tag) in the directory would duplicate the frontmatter and turn a
   routine edit (re-cutting a task) into a file move, the drift this tool exists to prevent.
   Field-based identity keeps the option to nest later open at zero migration cost.

## Consequences

- `aplan`'s own plan moved from `plan/` to `aplan/` (a `git mv`, preserving history); the docs,
  the CI dogfood default, and the trycmd fixtures moved with it.
- Prior ADRs (0008, 0009) reference the old `plan/tasks/` path. They are immutable point-in-time
  records and are left as written; this ADR is the pointer to the rename.
- The `root:` key ships with the change it enables (this ADR), never before, so the tool never
  carries a config key it ignores. `schemas/v1/config.json` regenerates to include it, gated by
  `gen-schema --check` like every other schema-derived field.
