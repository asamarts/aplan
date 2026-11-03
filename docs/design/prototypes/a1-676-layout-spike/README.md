# A1-676 layout spike

This is a research harness, not product code or a workspace member. It compares the current lane
scene, a deliberately small aplan-owned ranked-card prototype and `graphlay` 0.3.1 on real and
generated graphs. The exact dependency version and Rust 1.88 declaration make the dependency probe
repeatable.

From the repository root, with a current debug `aplan` binary:

```sh
mbx run --locked --manifest-path docs/design/prototypes/a1-676-layout-spike/Cargo.toml -- \
  "$PWD/target/debug/aplan" "$PWD"
```

The harness reads only JSON output from that binary. It does not mutate the plan. It intentionally
runs both orientations and several effort profiles, so a debug run takes tens of seconds. The
recorded method, build identity and interpreted results are in
[`../../graph-layout-evaluation.md`](../../graph-layout-evaluation.md).
