//! Structured YAML parser for `test` runner configuration (`A2-037`, `A2-038`).
//!
//! `serde` stops at the first missing field and phrases type mismatches in Rust
//! terms (`expected a sequence`, `expected struct TestSchedule`), which forced
//! adopters writing `test.runners` by hand through seven separate refusals
//! (issue #34). Parsing from [`yaml_compat::Value`] collects every missing
//! field, misplaced key, and configuration-terms type error with its full key
//! path in one pass while keeping the exact serde schema intact.

use crate::{
    TestBatch, TestCoverage, TestInputDependency, TestRunner, TestRunnerGroup, TestRunners,
    TestSchedule,
};

#[cfg(test)]
pub(crate) const SCHEDULE_KEYS: &[&str] = &["capacity"];
#[cfg(test)]
pub(crate) const GROUP_KEYS: &[&str] = &["schedule", "runners"];
pub(crate) const RUNNER_KEYS: &[&str] = &[
    "weight",
    "exclusive",
    "name",
    "dir",
    "covers",
    "covers_by_test",
    "inputs_by_test",
    "skipped",
    "output_lines",
    "max_output",
    "expect_tests",
    "max_memory",
    "max_processes",
    "wrap",
    "run_selects",
    "list",
    "run",
    "batch",
    "timeout_seconds",
];
#[cfg(test)]
pub(crate) const BATCH_KEYS: &[&str] = &["protocol", "run"];
#[cfg(test)]
pub(crate) const COVERAGE_KEYS: &[&str] = &["tests", "paths"];
#[cfg(test)]
pub(crate) const INPUT_DEP_KEYS: &[&str] = &["tests", "paths"];

const SHAPE_SCHEDULE: &str = "expected a mapping such as {capacity: 1}";
const SHAPE_CAPACITY: &str = "expected a positive integer such as 1";
const SHAPE_RUNNERS: &str = "expected a list of runner mappings";
const SHAPE_LIST: &str =
    "expected a command argv list of strings such as [cargo, test, --, --list]";
const SHAPE_RUN: &str =
    "expected a command argv list of strings such as [cargo, test, --, --exact, '{test}']";
const SHAPE_SKIPPED: &str =
    "expected a command argv list of strings such as [cargo, test, --, --list, --ignored]";
const SHAPE_WRAP: &str = "expected a command argv list of strings such as [nice, -n, '10']";
const SHAPE_COVERS: &str = "expected a list of path glob strings such as [crates/my-crate/**]";
const SHAPE_EXCLUSIVE: &str = "expected a list of resource name strings such as [database]";
const SHAPE_BATCH: &str =
    "expected a mapping such as {protocol: 1, run: [adapter, '{manifest}', '{result}']}";
const SHAPE_BATCH_PROTOCOL: &str = "expected an integer protocol version such as 1";
const SHAPE_BATCH_RUN: &str =
    "expected a command argv list of strings such as [adapter, '{manifest}', '{result}']";
const SHAPE_COVERS_BY_TEST: &str =
    "expected a list of mappings such as [{tests: ['suite::*'], paths: ['src/**']}]";
const SHAPE_COVERS_BY_TEST_ITEM: &str =
    "expected a mapping such as {tests: ['suite::*'], paths: ['src/**']}";
const SHAPE_GLOB_TESTS: &str = "expected a list of test-name glob strings";
const SHAPE_GLOB_PATHS: &str = "expected a list of path glob strings";

/// Whether `path` is a known `test` key placed at the wrong nesting level.
///
/// [`crate::walk_unknown`] skips these paths so they are reported as misplaced
/// keys naming where they belong instead of as unknown keys from a newer build.
pub(crate) fn is_misplaced_test_path(path: &str) -> bool {
    if matches!(
        path,
        "test.capacity"
            | "test.protocol"
            | "test.runners.schedule"
            | "test.runners.runners"
            | "test.runners.capacity"
            | "test.runners.protocol"
            | "test.schedule.schedule"
            | "test.schedule.runners"
            | "test.schedule.protocol"
    ) {
        return true;
    }
    if let Some(key) = path.strip_prefix("test.schedule.") {
        return RUNNER_KEYS.contains(&key);
    }
    false
}

/// Whether `path` expects a sequence of mappings, so a mapping written there is
/// a shape error rather than a collection of unknown config keys.
pub(crate) fn is_test_sequence_of_mappings_path(path: &str) -> bool {
    matches!(
        path,
        "test.runners"
            | "test.covers_by_test"
            | "test.inputs_by_test"
            | "test.runners.covers_by_test"
            | "test.runners.inputs_by_test"
    )
}

/// Strip a redundant leading `test: ` added by the YAML deserializer when our
/// diagnostic already begins with a `test` path.
pub(crate) fn strip_duplicate_test_prefix(message: &str) -> &str {
    match message.strip_prefix("test: ") {
        Some(rest)
            if rest.starts_with("test.")
                || rest.starts_with("test[")
                || rest.starts_with("test:") =>
        {
            rest
        }
        _ => message,
    }
}

#[derive(Default)]
struct Diagnostics {
    misplaced: Vec<String>,
    missing: Vec<String>,
    type_errors: Vec<String>,
    unknown_keys: Vec<String>,
    semantic_notes: Vec<String>,
}

impl Diagnostics {
    fn has_parse_errors(&self) -> bool {
        !self.misplaced.is_empty()
            || !self.missing.is_empty()
            || !self.type_errors.is_empty()
            || !self.unknown_keys.is_empty()
    }

    fn format(&self) -> String {
        let mut parts = Vec::new();
        parts.extend(self.misplaced.iter().cloned());
        if !self.missing.is_empty() {
            let label = if self.missing.len() == 1 {
                "missing required field"
            } else {
                "missing required fields"
            };
            parts.push(format!("{label}: {}", self.missing.join(", ")));
        }
        parts.extend(self.type_errors.iter().cloned());
        parts.extend(self.semantic_notes.iter().cloned());
        parts.extend(self.unknown_keys.iter().cloned());
        parts.join("; ")
    }
}

pub(crate) fn parse_test_runners(value: &yaml_compat::Value) -> Result<TestRunners, String> {
    let mut diag = Diagnostics::default();
    let unwrapped = unwrap_tagged(value);
    let parsed = if let Some(seq) = unwrapped.as_sequence() {
        parse_runner_sequence(seq, "test", false, &mut diag).map(TestRunners::Many)
    } else if let Some(map) = unwrapped.as_mapping() {
        let is_scheduled = map.contains_key(yaml_compat::Value::String("runners".into()))
            || map.contains_key(yaml_compat::Value::String("schedule".into()));
        if is_scheduled {
            parse_scheduled_group(map, &mut diag).map(TestRunners::Scheduled)
        } else {
            parse_runner_mapping(map, "test", RunnerContainer::Single, &mut diag)
                .map(TestRunners::One)
        }
    } else {
        diag.type_errors.push(format!(
            "test: expected a runner mapping, a list of runner mappings, or a scheduled \
             {{schedule: {{capacity: 1}}, runners: [...]}} mapping, got {}",
            describe_value(unwrapped)
        ));
        None
    };

    if diag.has_parse_errors() {
        return Err(diag.format());
    }
    parsed.ok_or_else(|| diag.format())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunnerContainer {
    Single,
    Sequence,
    Scheduled,
}

fn parse_scheduled_group(
    map: &yaml_compat::Mapping,
    diag: &mut Diagnostics,
) -> Option<TestRunnerGroup> {
    let mut schedule_val = None;
    let mut runners_val = None;
    for (key, value) in map {
        let Some(name) = key_as_str(key, "test", diag) else {
            continue;
        };
        match name {
            "schedule" => schedule_val = Some(value),
            "runners" => runners_val = Some(value),
            "capacity" => diag.misplaced.push(
                "test.capacity: misplaced key; `capacity` belongs inside `test.schedule` \
                 (for example `test.schedule: {capacity: 1}`)"
                    .into(),
            ),
            "protocol" => diag.misplaced.push(
                "test.protocol: misplaced key; `protocol` belongs inside a runner's `batch` \
                 mapping (`test.runners[i].batch.protocol`)"
                    .into(),
            ),
            runner_key if RUNNER_KEYS.contains(&runner_key) => diag.misplaced.push(format!(
                "test.{runner_key}: misplaced key; `{runner_key}` belongs on each runner under \
                 `test.runners[i].{runner_key}`, not beside `test.schedule` / `test.runners`"
            )),
            other => diag.unknown_keys.push(format!(
                "test.{other}: unknown field `{other}`, expected `schedule` or `runners`"
            )),
        }
    }

    let schedule = if let Some(val) = schedule_val {
        parse_schedule(val, "test.schedule", diag)
    } else {
        diag.missing
            .push(format!("test.schedule ({SHAPE_SCHEDULE})"));
        None
    };

    let runners = if let Some(val) = runners_val {
        let inner = unwrap_tagged(val);
        if let Some(seq) = inner.as_sequence() {
            if seq.is_empty() {
                diag.semantic_notes.push(
                    "`test.runners` is an empty list; remove the key, or give it a runner".into(),
                );
            }
            parse_runner_sequence(seq, "test.runners", true, diag)
        } else {
            diag.type_errors.push(format!(
                "test.runners: {SHAPE_RUNNERS}, got {}",
                describe_value(inner)
            ));
            None
        }
    } else {
        diag.missing.push(format!("test.runners ({SHAPE_RUNNERS})"));
        None
    };

    match (schedule, runners) {
        (Some(schedule), Some(runners)) => Some(TestRunnerGroup { schedule, runners }),
        _ => None,
    }
}

fn parse_schedule(
    value: &yaml_compat::Value,
    path: &str,
    diag: &mut Diagnostics,
) -> Option<TestSchedule> {
    let inner = unwrap_tagged(value);
    let Some(map) = inner.as_mapping() else {
        diag.type_errors.push(format!(
            "{path}: {SHAPE_SCHEDULE}, got {}",
            describe_value(inner)
        ));
        return None;
    };
    let mut capacity_val = None;
    for (key, val) in map {
        let Some(name) = key_as_str(key, path, diag) else {
            continue;
        };
        match name {
            "capacity" => capacity_val = Some(val),
            "schedule" => diag.misplaced.push(format!(
                "{path}.schedule: misplaced key; `capacity` belongs directly inside \
                 `test.schedule` (for example `test.schedule: {{capacity: 1}}`)"
            )),
            "runners" => diag.misplaced.push(format!(
                "{path}.runners: misplaced key; `runners` belongs one level up at `test.runners`"
            )),
            "protocol" => diag.misplaced.push(format!(
                "{path}.protocol: misplaced key; `protocol` belongs inside a runner's `batch` \
                 mapping (`test.runners[i].batch.protocol`)"
            )),
            runner_key if RUNNER_KEYS.contains(&runner_key) => diag.misplaced.push(format!(
                "{path}.{runner_key}: misplaced key; `{runner_key}` belongs on each runner under \
                 `test.runners[i].{runner_key}`, not inside `test.schedule`"
            )),
            other => diag.unknown_keys.push(format!(
                "{path}.{other}: unknown field `{other}`, expected `capacity`"
            )),
        }
    }
    let Some(val) = capacity_val else {
        diag.missing
            .push(format!("{path}.capacity ({SHAPE_CAPACITY})"));
        return None;
    };
    let capacity = parse_u32(val, &format!("{path}.capacity"), SHAPE_CAPACITY, diag)?;
    if capacity == 0 {
        diag.semantic_notes
            .push(format!("`{path}.capacity` is 0; {SHAPE_CAPACITY}"));
    }
    Some(TestSchedule { capacity })
}

fn parse_runner_sequence(
    seq: &[yaml_compat::Value],
    base: &str,
    scheduled: bool,
    diag: &mut Diagnostics,
) -> Option<Vec<TestRunner>> {
    let container = if scheduled {
        RunnerContainer::Scheduled
    } else {
        RunnerContainer::Sequence
    };
    let mut runners = Vec::with_capacity(seq.len());
    let mut all_ok = true;
    for (index, item) in seq.iter().enumerate() {
        let path = format!("{base}[{index}]");
        let inner = unwrap_tagged(item);
        let Some(map) = inner.as_mapping() else {
            diag.type_errors.push(format!(
                "{path}: expected a runner mapping (for example {{list: [...], run: [..., '{{test}}']}}), \
                 got {}",
                describe_value(inner)
            ));
            all_ok = false;
            continue;
        };
        if let Some(runner) = parse_runner_mapping(map, &path, container, diag) {
            runners.push(runner);
        } else {
            all_ok = false;
        }
    }
    all_ok.then_some(runners)
}

#[derive(Default)]
struct RawRunnerFields<'a> {
    weight: Option<&'a yaml_compat::Value>,
    exclusive: Option<&'a yaml_compat::Value>,
    name: Option<&'a yaml_compat::Value>,
    dir: Option<&'a yaml_compat::Value>,
    covers: Option<&'a yaml_compat::Value>,
    covers_by_test: Option<&'a yaml_compat::Value>,
    inputs_by_test: Option<&'a yaml_compat::Value>,
    skipped: Option<&'a yaml_compat::Value>,
    output_lines: Option<&'a yaml_compat::Value>,
    max_output: Option<&'a yaml_compat::Value>,
    expect_tests: Option<&'a yaml_compat::Value>,
    max_memory: Option<&'a yaml_compat::Value>,
    max_processes: Option<&'a yaml_compat::Value>,
    wrap: Option<&'a yaml_compat::Value>,
    run_selects: Option<&'a yaml_compat::Value>,
    list: Option<&'a yaml_compat::Value>,
    run: Option<&'a yaml_compat::Value>,
    batch: Option<&'a yaml_compat::Value>,
    timeout_seconds: Option<&'a yaml_compat::Value>,
}

fn collect_runner_fields<'a>(
    map: &'a yaml_compat::Mapping,
    path: &str,
    container: RunnerContainer,
    diag: &mut Diagnostics,
) -> RawRunnerFields<'a> {
    let mut raw = RawRunnerFields::default();
    for (key, val) in map {
        let Some(name) = key_as_str(key, path, diag) else {
            continue;
        };
        match name {
            "weight" => raw.weight = Some(val),
            "exclusive" => raw.exclusive = Some(val),
            "name" => raw.name = Some(val),
            "dir" => raw.dir = Some(val),
            "covers" => raw.covers = Some(val),
            "covers_by_test" => raw.covers_by_test = Some(val),
            "inputs_by_test" => raw.inputs_by_test = Some(val),
            "skipped" => raw.skipped = Some(val),
            "output_lines" => raw.output_lines = Some(val),
            "max_output" => raw.max_output = Some(val),
            "expect_tests" => raw.expect_tests = Some(val),
            "max_memory" => raw.max_memory = Some(val),
            "max_processes" => raw.max_processes = Some(val),
            "wrap" => raw.wrap = Some(val),
            "run_selects" => raw.run_selects = Some(val),
            "list" => raw.list = Some(val),
            "run" => raw.run = Some(val),
            "batch" => raw.batch = Some(val),
            "timeout_seconds" => raw.timeout_seconds = Some(val),
            "schedule" => {
                let target = match container {
                    RunnerContainer::Scheduled => {
                        "`schedule` belongs one level up at `test.schedule` \
                         (expected a mapping such as {capacity: 1})"
                    }
                    RunnerContainer::Sequence | RunnerContainer::Single => {
                        "`schedule` belongs at `test.schedule` using the scheduled \
                         `test: {schedule: {capacity: 1}, runners: [...]}` form"
                    }
                };
                diag.misplaced
                    .push(format!("{path}.schedule: misplaced key; {target}"));
            }
            "runners" => diag.misplaced.push(format!(
                "{path}.runners: misplaced key; `runners` belongs one level up at `test.runners` \
                 (alongside `test.schedule`)"
            )),
            "capacity" => diag.misplaced.push(format!(
                "{path}.capacity: misplaced key; `capacity` belongs at `test.schedule.capacity`"
            )),
            "protocol" => diag.misplaced.push(format!(
                "{path}.protocol: misplaced key; `protocol` belongs inside `{path}.batch.protocol`"
            )),
            other => {
                let allowed = RUNNER_KEYS
                    .iter()
                    .map(|k| format!("`{k}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                diag.unknown_keys.push(format!(
                    "{path}.{other}: unknown field `{other}`, expected one of {allowed}"
                ));
            }
        }
    }
    raw
}

struct IdentityAndCoverage {
    weight: Option<u32>,
    exclusive: Option<Vec<String>>,
    name: Option<String>,
    dir: Option<String>,
    covers: Option<Vec<String>>,
    covers_by_test: Option<Vec<TestCoverage>>,
    inputs_by_test: Option<Vec<TestInputDependency>>,
}

struct ExecutionAndLimits {
    skipped: Option<Vec<String>>,
    output_lines: Option<usize>,
    max_output: Option<String>,
    expect_tests: Option<bool>,
    max_memory: Option<String>,
    max_processes: Option<u32>,
    wrap: Option<Vec<String>>,
    run_selects: Option<bool>,
    batch: Option<TestBatch>,
    timeout_seconds: Option<u64>,
}

fn parse_identity_and_coverage(
    raw: &RawRunnerFields<'_>,
    path: &str,
    diag: &mut Diagnostics,
) -> Option<IdentityAndCoverage> {
    let f = |name| format!("{path}.{name}");
    let weight = parse_opt_u32(
        raw.weight,
        &f("weight"),
        "expected a positive integer such as 1",
        diag,
    );
    let exclusive = parse_opt_string_list(
        raw.exclusive,
        &f("exclusive"),
        SHAPE_EXCLUSIVE,
        "expected a resource name string",
        diag,
    );
    let name = parse_opt_string(raw.name, &f("name"), "expected a string such as unit", diag);
    let dir = parse_opt_string(
        raw.dir,
        &f("dir"),
        "expected a relative directory path string such as packages/web",
        diag,
    );
    let covers = parse_opt_string_list(
        raw.covers,
        &f("covers"),
        SHAPE_COVERS,
        "expected a path glob string",
        diag,
    );
    let covers_by_test = parse_opt_coverage_list(raw.covers_by_test, &f("covers_by_test"), diag);
    let inputs_by_test = parse_opt_input_dep_list(raw.inputs_by_test, &f("inputs_by_test"), diag);
    Some(IdentityAndCoverage {
        weight: weight.ok()?,
        exclusive: exclusive.ok()?,
        name: name.ok()?,
        dir: dir.ok()?,
        covers: covers.ok()?,
        covers_by_test: covers_by_test.ok()?,
        inputs_by_test: inputs_by_test.ok()?,
    })
}

fn parse_execution_and_limits(
    raw: &RawRunnerFields<'_>,
    path: &str,
    diag: &mut Diagnostics,
) -> Option<ExecutionAndLimits> {
    let f = |name| format!("{path}.{name}");
    let skipped = parse_opt_string_list(
        raw.skipped,
        &f("skipped"),
        SHAPE_SKIPPED,
        "expected a string argument",
        diag,
    );
    let output_lines = parse_opt_usize(
        raw.output_lines,
        &f("output_lines"),
        "expected a non-negative integer such as 8",
        diag,
    );
    let max_output = parse_opt_string(
        raw.max_output,
        &f("max_output"),
        "expected a size string such as 1M or 256K",
        diag,
    );
    let expect_tests = parse_opt_bool(
        raw.expect_tests,
        &f("expect_tests"),
        "expected a boolean (true or false)",
        diag,
    );
    let max_memory = parse_opt_string(
        raw.max_memory,
        &f("max_memory"),
        "expected a size string such as 4G or 512M",
        diag,
    );
    let max_processes = parse_opt_u32(
        raw.max_processes,
        &f("max_processes"),
        "expected a positive integer such as 64",
        diag,
    );
    let wrap = parse_opt_string_list(
        raw.wrap,
        &f("wrap"),
        SHAPE_WRAP,
        "expected a string argument",
        diag,
    );
    let run_selects = parse_opt_bool(
        raw.run_selects,
        &f("run_selects"),
        "expected a boolean (true or false)",
        diag,
    );
    let batch = parse_opt_batch(raw.batch, &f("batch"), diag);
    let timeout_seconds = parse_opt_u64(
        raw.timeout_seconds,
        &f("timeout_seconds"),
        "expected a non-negative integer of seconds such as 600",
        diag,
    );
    Some(ExecutionAndLimits {
        skipped: skipped.ok()?,
        output_lines: output_lines.ok()?,
        max_output: max_output.ok()?,
        expect_tests: expect_tests.ok()?,
        max_memory: max_memory.ok()?,
        max_processes: max_processes.ok()?,
        wrap: wrap.ok()?,
        run_selects: run_selects.ok()?,
        batch: batch.ok()?,
        timeout_seconds: timeout_seconds.ok()?,
    })
}

fn parse_runner_mapping(
    map: &yaml_compat::Mapping,
    path: &str,
    container: RunnerContainer,
    diag: &mut Diagnostics,
) -> Option<TestRunner> {
    let raw = collect_runner_fields(map, path, container, diag);
    let identity = parse_identity_and_coverage(&raw, path, diag);
    let execution = parse_execution_and_limits(&raw, path, diag);
    let list = if let Some(val) = raw.list {
        parse_string_list(
            val,
            &format!("{path}.list"),
            SHAPE_LIST,
            "expected a string argument",
            diag,
        )
    } else {
        diag.missing.push(format!("{path}.list ({SHAPE_LIST})"));
        None
    };
    let run = if let Some(val) = raw.run {
        parse_string_list(
            val,
            &format!("{path}.run"),
            SHAPE_RUN,
            "expected a string argument",
            diag,
        )
    } else {
        diag.missing.push(format!("{path}.run ({SHAPE_RUN})"));
        None
    };

    let (identity, execution, list, run) = match (identity, execution, list, run) {
        (Some(identity), Some(execution), Some(list), Some(run)) => {
            (identity, execution, list, run)
        }
        (_, _, _, Some(run_argv)) => {
            if let Err(crate::Error::BadRunner(reason)) =
                crate::validate_runner_run_argv(path, &run_argv)
            {
                diag.semantic_notes.push(reason);
            }
            return None;
        }
        _ => return None,
    };
    let runner = TestRunner {
        weight: identity.weight,
        exclusive: identity.exclusive,
        name: identity.name,
        dir: identity.dir,
        covers: identity.covers,
        covers_by_test: identity.covers_by_test,
        inputs_by_test: identity.inputs_by_test,
        skipped: execution.skipped,
        output_lines: execution.output_lines,
        max_output: execution.max_output,
        expect_tests: execution.expect_tests,
        max_memory: execution.max_memory,
        max_processes: execution.max_processes,
        wrap: execution.wrap,
        run_selects: execution.run_selects,
        list,
        run,
        batch: execution.batch,
        timeout_seconds: execution.timeout_seconds,
    };
    if let Err(crate::Error::BadRunner(reason)) = runner.validate_at(path) {
        diag.semantic_notes.push(reason);
    }
    Some(runner)
}

fn parse_opt_batch(
    value: Option<&yaml_compat::Value>,
    path: &str,
    diag: &mut Diagnostics,
) -> Result<Option<TestBatch>, ()> {
    let Some(val) = value else {
        return Ok(None);
    };
    let inner = unwrap_tagged(val);
    if inner.is_null() {
        return Ok(None);
    }
    let Some(map) = inner.as_mapping() else {
        diag.type_errors.push(format!(
            "{path}: {SHAPE_BATCH}, got {}",
            describe_value(inner)
        ));
        return Err(());
    };
    let mut protocol_val = None;
    let mut run_val = None;
    for (key, v) in map {
        let Some(name) = key_as_str(key, path, diag) else {
            continue;
        };
        match name {
            "protocol" => protocol_val = Some(v),
            "run" => run_val = Some(v),
            other => diag.unknown_keys.push(format!(
                "{path}.{other}: unknown field `{other}`, expected `protocol` or `run`"
            )),
        }
    }
    let protocol = if let Some(v) = protocol_val {
        parse_u32(v, &format!("{path}.protocol"), SHAPE_BATCH_PROTOCOL, diag)
    } else {
        diag.missing
            .push(format!("{path}.protocol ({SHAPE_BATCH_PROTOCOL})"));
        None
    };
    let run = if let Some(v) = run_val {
        parse_string_list(
            v,
            &format!("{path}.run"),
            SHAPE_BATCH_RUN,
            "expected a string argument",
            diag,
        )
    } else {
        diag.missing.push(format!("{path}.run ({SHAPE_BATCH_RUN})"));
        None
    };
    match (protocol, run) {
        (Some(protocol), Some(run)) => Ok(Some(TestBatch { protocol, run })),
        _ => Err(()),
    }
}

fn parse_opt_coverage_list(
    value: Option<&yaml_compat::Value>,
    path: &str,
    diag: &mut Diagnostics,
) -> Result<Option<Vec<TestCoverage>>, ()> {
    parse_opt_pair_list(value, path, diag, |tests, paths| TestCoverage {
        tests,
        paths,
    })
}

fn parse_opt_input_dep_list(
    value: Option<&yaml_compat::Value>,
    path: &str,
    diag: &mut Diagnostics,
) -> Result<Option<Vec<TestInputDependency>>, ()> {
    parse_opt_pair_list(value, path, diag, |tests, paths| TestInputDependency {
        tests,
        paths,
    })
}

fn parse_opt_pair_list<T>(
    value: Option<&yaml_compat::Value>,
    path: &str,
    diag: &mut Diagnostics,
    build: impl Fn(Vec<String>, Vec<String>) -> T,
) -> Result<Option<Vec<T>>, ()> {
    let Some(val) = value else {
        return Ok(None);
    };
    let inner = unwrap_tagged(val);
    if inner.is_null() {
        return Ok(None);
    }
    let Some(seq) = inner.as_sequence() else {
        diag.type_errors.push(format!(
            "{path}: {SHAPE_COVERS_BY_TEST}, got {}",
            describe_value(inner)
        ));
        return Err(());
    };
    let mut out = Vec::with_capacity(seq.len());
    let mut all_ok = true;
    for (idx, item) in seq.iter().enumerate() {
        let item_path = format!("{path}[{idx}]");
        let item_inner = unwrap_tagged(item);
        let Some(map) = item_inner.as_mapping() else {
            diag.type_errors.push(format!(
                "{item_path}: {SHAPE_COVERS_BY_TEST_ITEM}, got {}",
                describe_value(item_inner)
            ));
            all_ok = false;
            continue;
        };
        let mut tests_val = None;
        let mut paths_val = None;
        for (key, v) in map {
            let Some(name) = key_as_str(key, &item_path, diag) else {
                continue;
            };
            match name {
                "tests" => tests_val = Some(v),
                "paths" => paths_val = Some(v),
                other => diag.unknown_keys.push(format!(
                    "{item_path}.{other}: unknown field `{other}`, expected `tests` or `paths`"
                )),
            }
        }
        let tests = if let Some(v) = tests_val {
            parse_string_list(
                v,
                &format!("{item_path}.tests"),
                SHAPE_GLOB_TESTS,
                "expected a test-name glob string",
                diag,
            )
        } else {
            diag.missing
                .push(format!("{item_path}.tests ({SHAPE_GLOB_TESTS})"));
            None
        };
        let paths = if let Some(v) = paths_val {
            parse_string_list(
                v,
                &format!("{item_path}.paths"),
                SHAPE_GLOB_PATHS,
                "expected a path glob string",
                diag,
            )
        } else {
            diag.missing
                .push(format!("{item_path}.paths ({SHAPE_GLOB_PATHS})"));
            None
        };
        match (tests, paths) {
            (Some(tests), Some(paths)) => out.push(build(tests, paths)),
            _ => all_ok = false,
        }
    }
    if all_ok { Ok(Some(out)) } else { Err(()) }
}

fn unwrap_tagged(mut value: &yaml_compat::Value) -> &yaml_compat::Value {
    while let yaml_compat::Value::Tagged(tagged) = value {
        value = &tagged.value;
    }
    value
}

fn key_as_str<'a>(
    key: &'a yaml_compat::Value,
    parent: &str,
    diag: &mut Diagnostics,
) -> Option<&'a str> {
    let inner = unwrap_tagged(key);
    if let Some(s) = inner.as_str() {
        Some(s)
    } else {
        diag.type_errors.push(format!(
            "{parent}: expected string keys in mapping, got {}",
            describe_value(inner)
        ));
        None
    }
}

fn parse_opt_string(
    value: Option<&yaml_compat::Value>,
    path: &str,
    expected: &str,
    diag: &mut Diagnostics,
) -> Result<Option<String>, ()> {
    let Some(val) = value else {
        return Ok(None);
    };
    let inner = unwrap_tagged(val);
    if inner.is_null() {
        return Ok(None);
    }
    if let Some(s) = inner.as_str() {
        Ok(Some(s.to_string()))
    } else {
        diag.type_errors
            .push(format!("{path}: {expected}, got {}", describe_value(inner)));
        Err(())
    }
}

fn parse_opt_bool(
    value: Option<&yaml_compat::Value>,
    path: &str,
    expected: &str,
    diag: &mut Diagnostics,
) -> Result<Option<bool>, ()> {
    let Some(val) = value else {
        return Ok(None);
    };
    let inner = unwrap_tagged(val);
    if inner.is_null() {
        return Ok(None);
    }
    if let Some(b) = inner.as_bool() {
        Ok(Some(b))
    } else {
        diag.type_errors
            .push(format!("{path}: {expected}, got {}", describe_value(inner)));
        Err(())
    }
}

fn parse_u32(
    value: &yaml_compat::Value,
    path: &str,
    expected: &str,
    diag: &mut Diagnostics,
) -> Option<u32> {
    let inner = unwrap_tagged(value);
    if let Some(n) = inner.as_u64().and_then(|v| u32::try_from(v).ok()) {
        Some(n)
    } else {
        diag.type_errors
            .push(format!("{path}: {expected}, got {}", describe_value(inner)));
        None
    }
}

fn parse_opt_u32(
    value: Option<&yaml_compat::Value>,
    path: &str,
    expected: &str,
    diag: &mut Diagnostics,
) -> Result<Option<u32>, ()> {
    let Some(val) = value else {
        return Ok(None);
    };
    let inner = unwrap_tagged(val);
    if inner.is_null() {
        return Ok(None);
    }
    parse_u32(inner, path, expected, diag).map(Some).ok_or(())
}

fn parse_opt_u64(
    value: Option<&yaml_compat::Value>,
    path: &str,
    expected: &str,
    diag: &mut Diagnostics,
) -> Result<Option<u64>, ()> {
    let Some(val) = value else {
        return Ok(None);
    };
    let inner = unwrap_tagged(val);
    if inner.is_null() {
        return Ok(None);
    }
    if let Some(n) = inner.as_u64() {
        Ok(Some(n))
    } else {
        diag.type_errors
            .push(format!("{path}: {expected}, got {}", describe_value(inner)));
        Err(())
    }
}

fn parse_opt_usize(
    value: Option<&yaml_compat::Value>,
    path: &str,
    expected: &str,
    diag: &mut Diagnostics,
) -> Result<Option<usize>, ()> {
    let Some(val) = value else {
        return Ok(None);
    };
    let inner = unwrap_tagged(val);
    if inner.is_null() {
        return Ok(None);
    }
    if let Some(n) = inner.as_u64().and_then(|v| usize::try_from(v).ok()) {
        Ok(Some(n))
    } else {
        diag.type_errors
            .push(format!("{path}: {expected}, got {}", describe_value(inner)));
        Err(())
    }
}

fn parse_opt_string_list(
    value: Option<&yaml_compat::Value>,
    path: &str,
    expected_list: &str,
    expected_item: &str,
    diag: &mut Diagnostics,
) -> Result<Option<Vec<String>>, ()> {
    let Some(val) = value else {
        return Ok(None);
    };
    let inner = unwrap_tagged(val);
    if inner.is_null() {
        return Ok(None);
    }
    parse_string_list(inner, path, expected_list, expected_item, diag)
        .map(Some)
        .ok_or(())
}

fn parse_string_list(
    value: &yaml_compat::Value,
    path: &str,
    expected_list: &str,
    expected_item: &str,
    diag: &mut Diagnostics,
) -> Option<Vec<String>> {
    let inner = unwrap_tagged(value);
    let Some(seq) = inner.as_sequence() else {
        diag.type_errors.push(format!(
            "{path}: {expected_list}, got {}",
            describe_value(inner)
        ));
        return None;
    };
    let mut items = Vec::with_capacity(seq.len());
    let mut all_ok = true;
    for (idx, item) in seq.iter().enumerate() {
        let item_inner = unwrap_tagged(item);
        if let Some(s) = item_inner.as_str() {
            items.push(s.to_string());
        } else {
            diag.type_errors.push(format!(
                "{path}[{idx}]: {expected_item}, got {}",
                describe_value(item_inner)
            ));
            all_ok = false;
        }
    }
    all_ok.then_some(items)
}

fn describe_value(value: &yaml_compat::Value) -> String {
    match unwrap_tagged(value) {
        yaml_compat::Value::Null => "null".into(),
        yaml_compat::Value::Bool(b) => format!("boolean `{b}`"),
        yaml_compat::Value::Number(n) => format!("number `{n}`"),
        yaml_compat::Value::String(s) => {
            if s.chars().count() > 40 {
                let prefix: String = s.chars().take(40).collect();
                format!("string {prefix:?}...")
            } else {
                format!("string {s:?}")
            }
        }
        yaml_compat::Value::Sequence(_) => "a list".into(),
        yaml_compat::Value::Mapping(_) => "a mapping".into(),
        yaml_compat::Value::Tagged(_) => unreachable!("unwrap_tagged strips Tagged"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_yaml(yaml: &str) -> Result<TestRunners, String> {
        let val: yaml_compat::Value = yaml_compat::from_str(yaml).expect("valid yaml");
        parse_test_runners(&val)
    }

    #[test]
    fn test_runner_parser_covers_remaining_error_and_edge_branches() {
        // Non-string keys, misplaced `protocol`, and unknown fields at group/schedule/runner/batch/pair levels.
        let group_err =
            parse_yaml("schedule:\n  123: bad\n  extra: 1\nprotocol: 1\nweird_top: 2\n42: nope\n")
                .expect_err("group errors");
        assert!(
            group_err.contains("test.protocol: misplaced key"),
            "{group_err}"
        );
        assert!(
            group_err.contains("test.weird_top: unknown field"),
            "{group_err}"
        );
        assert!(group_err.contains("expected string keys"), "{group_err}");
        assert!(
            group_err.contains("test.schedule.extra: unknown field"),
            "{group_err}"
        );
        assert!(group_err.contains("test.schedule.capacity"), "{group_err}");
        assert!(group_err.contains("test.runners"), "{group_err}");

        // Non-mapping item in runner sequence & non-string key in runner mapping.
        let seq_err = parse_yaml("- 123\n- 99: bad\n  run: ['true']\n").expect_err("seq errors");
        assert!(
            seq_err.contains("test[0]: expected a runner mapping"),
            "{seq_err}"
        );
        assert!(
            seq_err.contains("test[1]: expected string keys"),
            "{seq_err}"
        );
        assert!(seq_err.contains("test[1].list"), "{seq_err}");
        assert!(seq_err.contains("{test}"), "{seq_err}");

        // Batch unknown field + non-string key, and covers_by_test null / non-mapping / unknown / non-string key.
        let nested_err = parse_yaml(
            "list: ['true']\n\
             run: ['true', '{test}']\n\
             batch:\n  1: bad\n  extra: 2\n\
             covers_by_test:\n  - 123\n  - 7: bad\n    extra: 1\n\
             inputs_by_test: null\n\
             name: 123\n\
             expect_tests: null\n\
             timeout_seconds: null\n\
             output_lines: null\n\
             wrap: null\n",
        )
        .expect_err("nested errors");
        assert!(
            nested_err.contains("test.batch.extra: unknown field"),
            "{nested_err}"
        );
        assert!(
            nested_err.contains("test.covers_by_test[0]"),
            "{nested_err}"
        );
        assert!(
            nested_err.contains("test.covers_by_test[1].extra: unknown field"),
            "{nested_err}"
        );
        assert!(nested_err.contains("test.name:"), "{nested_err}");

        // `describe_value` shapes: null, boolean, >40-char string, sequence, mapping, and tagged values.
        let long_str = "x".repeat(55);
        assert_eq!(describe_value(&yaml_compat::Value::Null), "null");
        assert_eq!(
            describe_value(&yaml_compat::Value::Bool(true)),
            "boolean `true`"
        );
        assert!(describe_value(&yaml_compat::Value::String(long_str)).ends_with("..."));
        assert_eq!(
            describe_value(&yaml_compat::Value::Sequence(vec![])),
            "a list"
        );
        assert_eq!(
            describe_value(&yaml_compat::Value::Mapping(yaml_compat::Mapping::new())),
            "a mapping"
        );
        let tagged = yaml_compat::Value::Tagged(Box::new(yaml_compat::TaggedValue {
            tag: yaml_compat::Tag::new("!", "custom"),
            value: yaml_compat::Value::Null,
        }));
        assert_eq!(describe_value(&tagged), "null");

        // Null optional fields in a valid runner parse cleanly.
        let ok = parse_yaml(
            "list: ['true']\n\
             run: ['true', '{test}']\n\
             covers_by_test: null\n\
             expect_tests: null\n\
             timeout_seconds: null\n\
             output_lines: null\n\
             wrap: null\n",
        )
        .expect("null optional fields");
        assert_eq!(ok.all().len(), 1);
    }
}
