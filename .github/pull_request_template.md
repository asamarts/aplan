<!-- Keep the tool honest: a change that makes a doc, count, or status disagree
     with the code is a bug, not a detail. -->

## What and why

<!-- One or two sentences. Link any task (aplan/tasks/A*.md) or ADR. -->

## Checklist

- [ ] `cargo test --workspace` passes (new behaviour has a test, both a firing and a
      silent case for a rule)
- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`
      are clean
- [ ] `cargo xtask gen-schema --check` passes (schema regenerated if types changed)
- [ ] `cargo run -p aplan -- check` and `cargo run -p aplan -- verify` are green
      (structural and whole-plan L1/L2 dogfood)
- [ ] docs updated where the change touches them (README rule table, HLD, ROADMAP,
      CHANGELOG), no doc left disagreeing with the code
