# Quickstart

## Starting a plan

A release cut is a `milestone` task whose `blocked_by` list names its work. When cut membership is
required, neither an empty cut nor an unassigned first task is valid on its own. Create both in one
atomic `aplan add --from` operation:

```yaml
# plan.yml
- id: M-v0.1
  title: First cut
  tags: [milestone]
- id: T-001
  title: Decide what done looks like
  cut: M-v0.1
  effort: 1
- id: T-002
  title: Wire the gate into CI
  cut: M-v0.1
  effort: 2
```

```console
$ aplan init
wrote ./.aplan.yml
edit it to set your policy; every setting is optional and documented inline.
$ aplan add --from plan.yml
wrote M-v0.1  todo  First cut
wrote T-001  todo  Decide what done looks like
wrote T-002  todo  Wire the gate into CI
  `M-v0.1` holds 2 of them, in its own file
$ aplan check
ok: 3 tasks, 0 warning(s), DAG acyclic
```

The operation writes nothing unless the complete post-state is valid. Entries may refer to other
entries in the same file regardless of order. Run `aplan add --help` for every supported field.

## Daily work

```sh
aplan ready
aplan why T-001
aplan start T-001
# implement and test
aplan close T-001 --evidence test:REGISTERED_TEST_NAME
aplan check
aplan verify
```

Use `aplan test list` to discover exact configured test names. A cited test must be listed by one
configured runner. Use `aplan show T-001` to inspect one task and its evidence, and `aplan facts` to
inspect the build's supported rules, commands, formats, configuration keys, and behaviors.

## CI

Run `aplan check` for structural validation. Run `aplan verify` with full Git history when evidence
must resolve. Add `--execute` only in a trusted environment when configured test commands should
run. For GitHub Actions setup, see [the repository README](../README.md#github-actions).
