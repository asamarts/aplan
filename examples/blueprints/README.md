# Blueprint examples

These are explicit input files for `aplan blueprint apply`, not built-ins and not a runtime
catalogue. Select one by path, inspect it, bind every declared parameter, and review a dry run
before applying it:

```sh
aplan blueprint apply --file examples/blueprints/pilot.yml --cut M-next \
  --set repository=widget --dry-run
```

The files demonstrate both supported arities without prescribing a methodology:

| file | shape | review before use |
|---|---|---|
| `pilot.yml` | one adoption task | repository-specific plan, configuration and pilot-log paths |
| `rule.yml` | one validation-rule task | the crate that owns the predicate, all required test surfaces and consumer documentation |
| `feature.yml` | a three-task graph | the design, implementation and proof split appropriate to the actual feature |

The pilot shape was compared with A1-071 and A1-140, whose real scopes contain only their pilot
logs. The rule shape was compared with A1-017 and A1-212, whose paths differ from each other and
from this example. Those instances establish useful recurring concerns, not one path set that all
pilots or rules share. Edit the proposed tasks to match the repository and the work in front of
you.

The integration census discovers every YAML file in this directory, applies each one to a fresh
scratch plan, and requires the resulting plan to pass `aplan check`. Adding an example therefore
also requires an executable census entry that supplies representative bindings and its expected
arity.

Promoting one of these files to a built-in or discoverable source is outside this feature. Such a
change first needs recorded adopter evidence and an explicit precedence and override policy for
repository, user and product sources; it must be planned as a separate task rather than smuggled
in with an example update.
