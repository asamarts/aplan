//! The versioned blueprint document (`A1-385`).
//!
//! A blueprint is a typed wrapper around the ordinary [`manifest::Entry`]. It adds a local key
//! for each task and parameter declarations for later binding, but deliberately carries no
//! recipe revision, provenance, discovery or execution language. ADR-0062 fixes that one-shot,
//! explicit-file boundary.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::manifest;

/// The one blueprint language this build understands.
pub const SCHEMA: &str = "aplan/blueprint/v1";

/// The language contract named by a document's `schema` key.
///
/// This is parser compatibility, not an author-controlled recipe revision. V1 has no revision or
/// provenance field because applying a blueprint is one-shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum Schema {
    /// The initial explicit-file blueprint language.
    #[serde(rename = "aplan/blueprint/v1")]
    V1,
}

/// One declared string parameter.
///
/// Every V1 parameter is required. The language has no default key.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    /// The name used by placeholders such as `{{feature}}`.
    pub name: String,
    /// What value the author must supply and where the blueprint uses it.
    pub description: String,
}

/// One task in a blueprint: exactly one local key plus an ordinary manifest entry.
///
/// The flattened shape is intentional. An adopter who knows `aplan add --from` sees the same
/// fields at the same level. The typed embedding keeps the two surfaces from acquiring independent
/// field lists.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The unique local name later entries use to refer to this task before it has a minted id.
    pub key: String,
    /// The fields accepted by one `aplan add --from` entry.
    #[serde(flatten)]
    pub task: manifest::Entry,
}

/// One explicit-file blueprint.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Blueprint {
    /// The language contract. Always `aplan/blueprint/v1`; it is not a recipe revision.
    pub schema: Schema,
    /// A short human label for reports. It is not a catalog key or source identity.
    #[schemars(regex(
        pattern = r"[^\s\u0000-\u001F\u007F-\u009F\u00AD\u061C\u200B-\u200F\u2060\uFEFF]"
    ))]
    pub name: String,
    /// A human-readable title for the task shape this file creates.
    #[schemars(regex(
        pattern = r"[^\s\u0000-\u001F\u007F-\u009F\u00AD\u061C\u200B-\u200F\u2060\uFEFF]"
    ))]
    pub title: String,
    /// What the blueprint creates and when an author should use it.
    #[schemars(regex(
        pattern = r"[^\s\u0000-\u001F\u007F-\u009F\u00AD\u061C\u200B-\u200F\u2060\uFEFF]"
    ))]
    pub description: String,
    /// Declared string parameters. Every declaration is required in V1.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// One or more ordinary task entries, each with a local key.
    #[schemars(length(min = 1))]
    pub tasks: Vec<Entry>,
}

/// What went wrong reading a blueprint.
#[derive(Debug)]
pub enum BlueprintError {
    /// The file could not be read.
    Io {
        /// The selected explicit file.
        path: std::path::PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// The file is not a V1 blueprint document.
    Parse {
        /// The selected explicit file.
        path: std::path::PathBuf,
        /// What the parser said.
        said: String,
    },
    /// A descriptive field contains no visible text.
    InvalidMetadata {
        /// The selected explicit file.
        path: std::path::PathBuf,
        /// The top-level key whose value is visually empty.
        field: &'static str,
    },
    /// The document parsed but carries no task entry.
    Empty {
        /// The selected explicit file.
        path: std::path::PathBuf,
    },
}

/// Why a parsed blueprint could not be bound to one complete parameter set.
///
/// Binding consumes only typed string values after YAML deserialisation. It
/// never rewrites source text, field names, sequence boundaries or parameter
/// values a second time.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindingError {
    /// A declaration cannot be addressed by the placeholder grammar or a
    /// future `--set NAME=VALUE` spelling.
    #[error(
        "blueprint parameter `{name}` is not a valid name; use an ASCII letter or `_` first, then ASCII letters, digits, `_`, or `-`"
    )]
    InvalidParameterName {
        /// The rejected declaration name.
        name: String,
    },
    /// A declaration must explain the value it requests.
    #[error("blueprint parameter `{name}` has no visible description")]
    InvalidParameterDescription {
        /// The declaration with the empty description.
        name: String,
    },
    /// Two declarations would make one binding ambiguous.
    #[error("blueprint parameter `{name}` is declared more than once")]
    DuplicateParameter {
        /// The duplicated declaration name.
        name: String,
    },
    /// The caller supplied a name this document does not declare.
    #[error("binding `{name}` is not declared by this blueprint")]
    UnknownBinding {
        /// The unexpected binding name.
        name: String,
    },
    /// A used, required V1 parameter has no supplied value.
    #[error("required blueprint parameter `{name}` has no binding")]
    MissingBinding {
        /// The missing parameter name.
        name: String,
    },
    /// Presence alone is not a useful required value when no reader can see
    /// it.
    #[error("required blueprint parameter `{name}` has no visible value")]
    EmptyBinding {
        /// The parameter whose supplied value is visually empty.
        name: String,
    },
    /// A declaration with no placeholder is dead input rather than an implied
    /// future use.
    #[error("blueprint parameter `{name}` is declared but no field uses `{{{{{name}}}}}`")]
    UnusedParameter {
        /// The unused declaration name.
        name: String,
    },
    /// A placeholder is not licensed by the declaration table.
    #[error("blueprint field `{field}` uses undeclared parameter `{name}`")]
    UnknownPlaceholder {
        /// The typed field carrying the placeholder.
        field: String,
        /// The undeclared name.
        name: String,
    },
    /// Local graph symbols are deliberately static and cannot be changed by a
    /// content binding.
    #[error("blueprint field `{field}` is structural and cannot use parameter `{name}`")]
    StructuralPlaceholder {
        /// The structural field.
        field: String,
        /// The parameter named there.
        name: String,
    },
    /// Delimiters must form either one placeholder or the documented literal
    /// escape.
    #[error("blueprint field `{field}` has malformed placeholder syntax: {detail}")]
    MalformedPlaceholder {
        /// The typed field carrying the malformed text.
        field: String,
        /// The actionable parse detail.
        detail: String,
    },
}

/// Why a bound blueprint cannot become one ordinary manifest mutation.
///
/// The namespaces are part of the language rather than a lookup heuristic:
/// `local:key` always names a sibling key and `task:id` always names an
/// existing task. Bare strings are refused even when one lookup happens to
/// succeed, so adding a coincidentally named task cannot change an old
/// blueprint's graph.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReferenceError {
    /// A key must be a stable, compact symbol rather than arbitrary prose.
    #[error(
        "blueprint task {task} has invalid local key `{key}`; use an ASCII letter or `_` first, then ASCII letters, digits, `_`, or `-`"
    )]
    InvalidKey {
        /// One-based task position in the document.
        task: usize,
        /// The rejected key.
        key: String,
    },
    /// One local symbol cannot identify two minted tasks.
    #[error("blueprint local key `{key}` is claimed by tasks {first} and {second}")]
    DuplicateKey {
        /// The ambiguous key.
        key: String,
        /// One-based first declaration.
        first: usize,
        /// One-based repeated declaration.
        second: usize,
    },
    /// Every application must mint fresh ids.
    #[error(
        "blueprint task `{key}` declares fixed id `{id}`; a reusable blueprint must mint its id at application time"
    )]
    FixedId {
        /// The entry's local key.
        key: String,
        /// The forbidden fixed id.
        id: String,
    },
    /// V1 has one explicit application target, not per-entry placement.
    #[error(
        "blueprint task `{key}` declares cut `{cut}`; V1 uses one application cut for the complete blueprint"
    )]
    EntryCut {
        /// The entry's local key.
        key: String,
        /// The forbidden per-entry cut.
        cut: String,
    },
    /// Relationship strings must say which namespace they use.
    #[error(
        "blueprint field `{field}` has ambiguous reference `{reference}`; every relationship must begin with `local:` or `task:`"
    )]
    AmbiguousReference {
        /// The exact typed relationship slot.
        field: String,
        /// The rejected bare reference.
        reference: String,
    },
    /// A namespace with no visible target is not a reference.
    #[error("blueprint field `{field}` has `{namespace}:` with no visible target")]
    EmptyReference {
        /// The exact typed relationship slot.
        field: String,
        /// `local` or `task`.
        namespace: &'static str,
    },
    /// A local relationship must resolve within this document.
    #[error(
        "blueprint field `{field}` names local key `{key}`, but that name names no blueprint task key"
    )]
    MissingLocal {
        /// The exact typed relationship slot.
        field: String,
        /// The absent sibling key.
        key: String,
    },
    /// The application layer failed to assign an id to a validated key.
    #[error("blueprint task key `{key}` was not assigned a minted id")]
    MissingAssignment {
        /// The unassigned key.
        key: String,
    },
    /// Assignment is exact rather than an ignored caller map.
    #[error("minted id assignment names unknown blueprint key `{key}`")]
    UnknownAssignment {
        /// The extra key.
        key: String,
    },
    /// The application target must be a real cut id resolved by the caller.
    #[error("a blueprint application needs one visible existing cut id")]
    InvalidApplicationCut,
    /// An allocator answer must already be safe as a task id and file stem.
    #[error("blueprint task key `{key}` was assigned unusable id `{id}`")]
    InvalidAssignedId {
        /// The key receiving the invalid id.
        key: String,
        /// The allocator answer.
        id: String,
    },
    /// Two siblings cannot compile to one task id.
    #[error("blueprint task keys `{first}` and `{second}` were both assigned id `{id}`")]
    DuplicateAssignedId {
        /// First key using the id.
        first: String,
        /// Second key using the id.
        second: String,
        /// The duplicated id.
        id: String,
    },
}

impl std::fmt::Display for BlueprintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "reading {}: {source}", path.display()),
            Self::Parse { path, said } => {
                write!(f, "{} is not an {SCHEMA} blueprint: {said}", path.display())
            }
            Self::InvalidMetadata { path, field } => write!(
                f,
                "{} has no visible text in blueprint field `{field}`",
                path.display()
            ),
            Self::Empty { path } => write!(
                f,
                "{} names no blueprint tasks, so there is nothing to apply",
                path.display()
            ),
        }
    }
}

impl std::error::Error for BlueprintError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { .. } | Self::InvalidMetadata { .. } | Self::Empty { .. } => None,
        }
    }
}

/// Read one explicit local blueprint file.
///
/// # Errors
///
/// [`BlueprintError`] when the file cannot be read, does not match the V1 types, or names no
/// tasks.
pub fn read(path: &std::path::Path) -> Result<Blueprint, BlueprintError> {
    let text = std::fs::read_to_string(path).map_err(|source| BlueprintError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse(path, &text)
}

/// Parse blueprint bytes already captured by a delivery adapter.
///
/// # Errors
///
/// [`BlueprintError::Parse`] when `text` does not match the V1 types, or
/// [`BlueprintError::Empty`] when it names no tasks.
pub fn parse(path: &std::path::Path, text: &str) -> Result<Blueprint, BlueprintError> {
    let blueprint: Blueprint = yaml_compat::from_str(text).map_err(|error| {
        let said = describe_parse_error(&error.to_string());
        BlueprintError::Parse {
            path: path.to_path_buf(),
            said,
        }
    })?;
    for (field, value) in [
        ("name", blueprint.name.as_str()),
        ("title", blueprint.title.as_str()),
        ("description", blueprint.description.as_str()),
    ] {
        if aplan_core::task::is_vacuous_ref(value) {
            return Err(BlueprintError::InvalidMetadata {
                path: path.to_path_buf(),
                field,
            });
        }
    }
    if blueprint.tasks.is_empty() {
        return Err(BlueprintError::Empty {
            path: path.to_path_buf(),
        });
    }
    Ok(blueprint)
}

/// Bind every declared parameter in a parsed blueprint.
///
/// Placeholders are `{{name}}`. `{{{{` and `}}}}` produce literal `{{` and
/// `}}`. Substitution is deliberately non-recursive: placeholder-looking text
/// in a supplied value remains ordinary text. The blueprint `name`, parameter
/// declarations and task `key` values are structural and are not
/// parameterised.
///
/// All other string slots are bound independently after YAML parsing. A value
/// containing `:`, a newline, brackets or list markers therefore remains one
/// string in one already-typed slot; it cannot add a mapping key, list member,
/// task or relationship.
///
/// # Errors
///
/// [`BindingError`] when declarations, bindings or placeholder syntax are not
/// total and unambiguous.
pub fn bind_parameters(
    blueprint: &Blueprint,
    bindings: &BTreeMap<String, String>,
) -> Result<Blueprint, BindingError> {
    let declared = declarations(blueprint)?;
    if let Some(name) = bindings.keys().find(|name| !declared.contains(*name)) {
        return Err(BindingError::UnknownBinding { name: name.clone() });
    }
    if let Some(name) = declared.iter().find(|name| !bindings.contains_key(*name)) {
        return Err(BindingError::MissingBinding { name: name.clone() });
    }
    if let Some((name, _)) = bindings
        .iter()
        .find(|(_, value)| aplan_core::task::is_vacuous_ref(value))
    {
        return Err(BindingError::EmptyBinding { name: name.clone() });
    }

    let mut binder = Binder {
        declared: &declared,
        bindings,
        used: BTreeSet::new(),
    };
    let mut bound = blueprint.clone();
    bound.name = binder.bind("name", &blueprint.name, false)?;
    bound.title = binder.bind("title", &blueprint.title, true)?;
    bound.description = binder.bind("description", &blueprint.description, true)?;
    for (index, parameter) in bound.parameters.iter_mut().enumerate() {
        parameter.description = binder.bind(
            &format!("parameters[{index}].description"),
            &blueprint.parameters[index].description,
            false,
        )?;
    }
    for (index, entry) in bound.tasks.iter_mut().enumerate() {
        let source = &blueprint.tasks[index];
        entry.key = binder.bind(&format!("tasks[{index}].key"), &source.key, false)?;
        entry.task = bind_entry(&mut binder, index, &source.task)?;
    }

    if let Some(name) = declared.iter().find(|name| !binder.used.contains(*name)) {
        return Err(BindingError::UnusedParameter { name: name.clone() });
    }
    Ok(bound)
}

/// Validate the application-only structure and return task keys in source
/// order.
///
/// Parsing deliberately preserves manifest entry parity, including `id` and
/// `cut`. Application narrows those two fields: ids are always minted, and one
/// explicit cut argument places the complete graph. Keeping this semantic
/// check here avoids forking the schema while ensuring a delivery adapter
/// cannot silently ignore either value.
///
/// # Errors
///
/// [`ReferenceError`] when a key is invalid or duplicated, an entry tries to
/// fix its id or cut, or any graph field is not a total namespaced reference.
pub fn application_keys(blueprint: &Blueprint) -> Result<Vec<String>, ReferenceError> {
    let mut declared = BTreeMap::<String, usize>::new();
    for (index, entry) in blueprint.tasks.iter().enumerate() {
        let task = index + 1;
        if !valid_local_key(&entry.key) {
            return Err(ReferenceError::InvalidKey {
                task,
                key: entry.key.clone(),
            });
        }
        if let Some(first) = declared.insert(entry.key.clone(), task) {
            return Err(ReferenceError::DuplicateKey {
                key: entry.key.clone(),
                first,
                second: task,
            });
        }
        if let Some(id) = &entry.task.id {
            return Err(ReferenceError::FixedId {
                key: entry.key.clone(),
                id: id.clone(),
            });
        }
        if let Some(cut) = &entry.task.cut {
            return Err(ReferenceError::EntryCut {
                key: entry.key.clone(),
                cut: cut.clone(),
            });
        }
    }
    let keys = declared.keys().cloned().collect::<BTreeSet<_>>();
    for (entry_index, entry) in blueprint.tasks.iter().enumerate() {
        for (field, values) in relationship_fields(&entry.task) {
            for (value_index, value) in values.iter().enumerate() {
                validate_reference(
                    &format!("tasks[{entry_index}].{field}[{value_index}]"),
                    value,
                    &keys,
                )?;
            }
        }
    }
    Ok(blueprint
        .tasks
        .iter()
        .map(|entry| entry.key.clone())
        .collect())
}

/// Compile a validated, bound blueprint into ordinary manifest entries.
///
/// `assigned` is the id allocator's complete answer for this plan snapshot.
/// Local references become those ids, existing-task references lose their
/// explicit `task:` namespace, and every entry joins `cut`. The resulting
/// entries intentionally go through ordinary manifest authoring and its
/// complete post-state gate.
///
/// # Errors
///
/// [`ReferenceError`] when the graph or assignment is incomplete or
/// ambiguous.
pub fn resolve_application(
    blueprint: &Blueprint,
    assigned: &BTreeMap<String, String>,
    cut: &str,
) -> Result<Vec<manifest::Entry>, ReferenceError> {
    if aplan_core::task::is_vacuous_ref(cut) {
        return Err(ReferenceError::InvalidApplicationCut);
    }
    let keys = application_keys(blueprint)?;
    let declared = keys.iter().cloned().collect::<BTreeSet<_>>();
    if let Some(key) = assigned.keys().find(|key| !declared.contains(*key)) {
        return Err(ReferenceError::UnknownAssignment { key: key.clone() });
    }
    if let Some(key) = keys.iter().find(|key| !assigned.contains_key(*key)) {
        return Err(ReferenceError::MissingAssignment { key: key.clone() });
    }
    let mut by_id = BTreeMap::<&str, &str>::new();
    for key in &keys {
        let id = assigned
            .get(key)
            .expect("the complete assignment was checked above");
        if !crate::emit::is_usable_id(id) {
            return Err(ReferenceError::InvalidAssignedId {
                key: key.clone(),
                id: id.clone(),
            });
        }
        if let Some(first) = by_id.insert(id, key) {
            return Err(ReferenceError::DuplicateAssignedId {
                first: first.to_string(),
                second: key.clone(),
                id: id.clone(),
            });
        }
    }

    blueprint
        .tasks
        .iter()
        .enumerate()
        .map(|(entry_index, entry)| {
            let mut task = entry.task.clone();
            task.id = Some(
                assigned
                    .get(&entry.key)
                    .expect("the complete assignment was checked above")
                    .clone(),
            );
            task.cut = Some(cut.to_string());
            for (field, values) in relationship_fields_mut(&mut task) {
                for (value_index, value) in values.iter_mut().enumerate() {
                    *value = resolve_reference(
                        &format!("tasks[{entry_index}].{field}[{value_index}]"),
                        value,
                        &declared,
                        assigned,
                    )?;
                }
            }
            Ok(task)
        })
        .collect()
}

fn valid_local_key(key: &str) -> bool {
    valid_parameter_name(key)
}

fn relationship_fields(entry: &manifest::Entry) -> [(&'static str, &[String]); 5] {
    [
        ("blocked_by", &entry.blocked_by),
        ("blocks", &entry.blocks),
        ("prefer_after", &entry.prefer_after),
        ("ratified_by", &entry.ratified_by),
        ("contains", &entry.contains),
    ]
}

fn relationship_fields_mut(entry: &mut manifest::Entry) -> [(&'static str, &mut Vec<String>); 5] {
    [
        ("blocked_by", &mut entry.blocked_by),
        ("blocks", &mut entry.blocks),
        ("prefer_after", &mut entry.prefer_after),
        ("ratified_by", &mut entry.ratified_by),
        ("contains", &mut entry.contains),
    ]
}

fn validate_reference(
    field: &str,
    reference: &str,
    keys: &BTreeSet<String>,
) -> Result<(), ReferenceError> {
    if let Some(key) = reference.strip_prefix("local:") {
        if aplan_core::task::is_vacuous_ref(key) {
            return Err(ReferenceError::EmptyReference {
                field: field.to_string(),
                namespace: "local",
            });
        }
        if !keys.contains(key) {
            return Err(ReferenceError::MissingLocal {
                field: field.to_string(),
                key: key.to_string(),
            });
        }
        Ok(())
    } else if let Some(id) = reference.strip_prefix("task:") {
        if aplan_core::task::is_vacuous_ref(id) {
            Err(ReferenceError::EmptyReference {
                field: field.to_string(),
                namespace: "task",
            })
        } else {
            Ok(())
        }
    } else {
        Err(ReferenceError::AmbiguousReference {
            field: field.to_string(),
            reference: reference.to_string(),
        })
    }
}

fn resolve_reference(
    field: &str,
    reference: &str,
    keys: &BTreeSet<String>,
    assigned: &BTreeMap<String, String>,
) -> Result<String, ReferenceError> {
    validate_reference(field, reference, keys)?;
    if let Some(key) = reference.strip_prefix("local:") {
        assigned
            .get(key)
            .cloned()
            .ok_or_else(|| ReferenceError::MissingAssignment {
                key: key.to_string(),
            })
    } else {
        Ok(reference
            .strip_prefix("task:")
            .expect("validated namespaced task reference")
            .to_string())
    }
}

fn declarations(blueprint: &Blueprint) -> Result<BTreeSet<String>, BindingError> {
    let mut declared = BTreeSet::new();
    for parameter in &blueprint.parameters {
        if !valid_parameter_name(&parameter.name) {
            return Err(BindingError::InvalidParameterName {
                name: parameter.name.clone(),
            });
        }
        if aplan_core::task::is_vacuous_ref(&parameter.description) {
            return Err(BindingError::InvalidParameterDescription {
                name: parameter.name.clone(),
            });
        }
        if !declared.insert(parameter.name.clone()) {
            return Err(BindingError::DuplicateParameter {
                name: parameter.name.clone(),
            });
        }
    }
    Ok(declared)
}

fn valid_parameter_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn bind_entry(
    binder: &mut Binder<'_>,
    index: usize,
    entry: &manifest::Entry,
) -> Result<manifest::Entry, BindingError> {
    let at = |field: &str| format!("tasks[{index}].{field}");
    Ok(manifest::Entry {
        title: binder.bind(&at("title"), &entry.title, true)?,
        id: bind_optional(binder, &at("id"), entry.id.as_deref())?,
        phase: entry.phase,
        cut: bind_optional(binder, &at("cut"), entry.cut.as_deref())?,
        effort: entry.effort,
        owner: bind_optional(binder, &at("owner"), entry.owner.as_deref())?,
        status: entry.status,
        tags: bind_list(binder, &at("tags"), &entry.tags)?,
        paths: bind_list(binder, &at("paths"), &entry.paths)?,
        traces: bind_list(binder, &at("traces"), &entry.traces)?,
        blocked_by: bind_list(binder, &at("blocked_by"), &entry.blocked_by)?,
        blocks: bind_list(binder, &at("blocks"), &entry.blocks)?,
        prefer_after: bind_list(binder, &at("prefer_after"), &entry.prefer_after)?,
        ratified_by: bind_list(binder, &at("ratified_by"), &entry.ratified_by)?,
        contains: bind_list(binder, &at("contains"), &entry.contains)?,
        revisit_at: bind_optional(binder, &at("revisit_at"), entry.revisit_at.as_deref())?,
        version_impact: entry.version_impact,
        version_impact_reason: bind_optional(
            binder,
            &at("version_impact_reason"),
            entry.version_impact_reason.as_deref(),
        )?,
        evidence: entry
            .evidence
            .iter()
            .enumerate()
            .map(|(evidence_index, evidence)| {
                Ok(aplan_core::Evidence {
                    kind: evidence.kind,
                    reference: binder.bind(
                        &format!("tasks[{index}].evidence[{evidence_index}].ref"),
                        &evidence.reference,
                        true,
                    )?,
                    captured: bind_optional(
                        binder,
                        &format!("tasks[{index}].evidence[{evidence_index}].captured"),
                        evidence.captured.as_deref(),
                    )?,
                })
            })
            .collect::<Result<Vec<_>, BindingError>>()?,
        body: bind_optional(binder, &at("body"), entry.body.as_deref())?,
    })
}

fn bind_optional(
    binder: &mut Binder<'_>,
    field: &str,
    value: Option<&str>,
) -> Result<Option<String>, BindingError> {
    value
        .map(|value| binder.bind(field, value, true))
        .transpose()
}

fn bind_list(
    binder: &mut Binder<'_>,
    field: &str,
    values: &[String],
) -> Result<Vec<String>, BindingError> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| binder.bind(&format!("{field}[{index}]"), value, true))
        .collect()
}

struct Binder<'a> {
    declared: &'a BTreeSet<String>,
    bindings: &'a BTreeMap<String, String>,
    used: BTreeSet<String>,
}

impl Binder<'_> {
    fn bind(
        &mut self,
        field: &str,
        source: &str,
        parameters_allowed: bool,
    ) -> Result<String, BindingError> {
        let mut output = String::with_capacity(source.len());
        let mut remaining = source;
        while let Some((offset, opening)) = next_delimiter(remaining) {
            output.push_str(&remaining[..offset]);
            remaining = &remaining[offset..];
            if opening {
                if let Some(rest) = remaining.strip_prefix("{{{{") {
                    output.push_str("{{");
                    remaining = rest;
                    continue;
                }
                let Some(close) = remaining[2..].find("}}") else {
                    return Err(BindingError::MalformedPlaceholder {
                        field: field.to_string(),
                        detail: "`{{` has no matching `}}`; write `{{{{` for a literal `{{`"
                            .to_string(),
                    });
                };
                let name = &remaining[2..2 + close];
                if !valid_parameter_name(name) {
                    return Err(BindingError::MalformedPlaceholder {
                        field: field.to_string(),
                        detail: format!("`{{{{{name}}}}}` does not contain a valid parameter name"),
                    });
                }
                if !self.declared.contains(name) {
                    return Err(BindingError::UnknownPlaceholder {
                        field: field.to_string(),
                        name: name.to_string(),
                    });
                }
                if !parameters_allowed {
                    return Err(BindingError::StructuralPlaceholder {
                        field: field.to_string(),
                        name: name.to_string(),
                    });
                }
                let value =
                    self.bindings
                        .get(name)
                        .ok_or_else(|| BindingError::MissingBinding {
                            name: name.to_string(),
                        })?;
                self.used.insert(name.to_string());
                output.push_str(value);
                remaining = &remaining[2 + close + 2..];
            } else if let Some(rest) = remaining.strip_prefix("}}}}") {
                output.push_str("}}");
                remaining = rest;
            } else {
                return Err(BindingError::MalformedPlaceholder {
                    field: field.to_string(),
                    detail: "`}}` has no matching `{{`; write `}}}}` for a literal `}}`"
                        .to_string(),
                });
            }
        }
        output.push_str(remaining);
        Ok(output)
    }
}

fn next_delimiter(value: &str) -> Option<(usize, bool)> {
    match (value.find("{{"), value.find("}}")) {
        (Some(open), Some(close)) if open <= close => Some((open, true)),
        (_, Some(close)) => Some((close, false)),
        (Some(open), None) => Some((open, true)),
        (None, None) => None,
    }
}

/// Every property a derived object schema accepts, including flattened and referenced types.
fn schema_property_names<T: JsonSchema>() -> Vec<String> {
    fn walk(
        node: &serde_json::Value,
        defs: &serde_json::Map<String, serde_json::Value>,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        if let Some(properties) = node
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            out.extend(properties.keys().cloned());
        }
        if let Some(reference) = node.get("$ref").and_then(serde_json::Value::as_str)
            && let Some(name) = reference.strip_prefix("#/$defs/")
            && let Some(target) = defs.get(name)
        {
            walk(target, defs, out);
        }
        for union in ["allOf", "anyOf", "oneOf"] {
            if let Some(branches) = node.get(union).and_then(serde_json::Value::as_array) {
                for branch in branches {
                    walk(branch, defs, out);
                }
            }
        }
    }

    let schema = schemars::schema_for!(T);
    let root = schema.as_value();
    let defs = root
        .get("$defs")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut names = std::collections::BTreeSet::new();
    walk(root, &defs, &mut names);
    names.into_iter().collect()
}

/// Add the accepted set which serde cannot report for a flattened entry.
///
/// The path comes from the typed YAML deserializer. The names come from the same derived schemas
/// the editor sees, so adding an entry or document field updates this remedy without another
/// list.
fn describe_parse_error(said: &str) -> String {
    if !said.contains("unknown field") {
        return said.to_string();
    }
    let (location, accepted) = if said.starts_with("tasks[") {
        ("tasks[]", schema_property_names::<Entry>())
    } else if said.starts_with("parameters[") {
        ("parameters[]", schema_property_names::<Parameter>())
    } else {
        ("the blueprint", schema_property_names::<Blueprint>())
    };
    let accepted = accepted
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{said}; accepted fields in {location}: {accepted}")
}

/// The JSON Schema derived from [`Blueprint`].
#[must_use]
pub fn derived_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Blueprint)).expect("schema is serialisable")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use schemars::JsonSchema;

    use super::{
        BindingError, Blueprint, BlueprintError, Entry, Parameter, ReferenceError, SCHEMA,
        application_keys, bind_parameters, derived_schema, parse, read, resolve_application,
    };

    fn document(tasks: &str) -> String {
        format!(
            "schema: {SCHEMA}\nname: feature\ntitle: Feature delivery\n\
             description: Creates a small feature graph.\n\
             parameters:\n  - name: feature\n    description: The feature name.\n\
             tasks:\n{tasks}"
        )
    }

    fn visually_empty_values() -> [&'static str; 13] {
        [
            "",
            "   ",
            "\u{00a0}",
            "\u{00ad}",
            "\u{061c}",
            "\u{200b}",
            "\u{200c}",
            "\u{200d}",
            "\u{200e}",
            "\u{200f}",
            "\u{2060}",
            "\u{feff}",
            "\u{200b} \u{200e}",
        ]
    }

    #[test]
    fn one_and_many_task_blueprints_share_one_document_shape() {
        let one = parse(
            std::path::Path::new("one.yml"),
            &document("  - key: design\n    title: Decide it\n    effort: 1\n"),
        )
        .expect("one task");
        assert_eq!(one.tasks.len(), 1);
        assert_eq!(one.tasks[0].key, "design");
        assert_eq!(one.tasks[0].task.title, "Decide it");
        assert_eq!(one.parameters[0].name, "feature");

        let many = parse(
            std::path::Path::new("many.yml"),
            &document(concat!(
                "  - key: design\n",
                "    title: Decide it\n",
                "    effort: 1\n",
                "  - key: implement\n",
                "    title: Implement it\n",
                "    effort: 3\n",
                "    blocked_by: [local:design]\n",
            )),
        )
        .expect("many tasks use the same type");
        assert_eq!(many.tasks.len(), 2);
        assert_eq!(
            many.tasks[1].task.blocked_by,
            vec!["local:design".to_string()]
        );
    }

    #[test]
    fn top_level_metadata_refuses_every_visually_empty_neighbour() {
        let valid = document("  - key: design\n    title: Decide it\n");
        for (field, original) in [
            ("name", "feature"),
            ("title", "Feature delivery"),
            ("description", "Creates a small feature graph."),
        ] {
            for empty in visually_empty_values() {
                let wrong = valid.replacen(
                    &format!("{field}: {original}\n"),
                    &format!("{field}: '{empty}'\n"),
                    1,
                );
                let error = parse(std::path::Path::new("empty.yml"), &wrong)
                    .expect_err("visually empty metadata")
                    .to_string();
                assert!(error.contains(&format!("`{field}`")), "{error}");
                assert!(error.contains("visible text"), "{error}");
            }
        }
    }

    #[test]
    fn schema_and_parser_agree_on_substantive_metadata() {
        let schema = derived_schema();
        let validator = jsonschema::validator_for(&schema).expect("blueprint schema compiles");
        let valid = serde_json::json!({
            "schema": SCHEMA,
            "name": "f\u{00f6}rm",
            "title": "D\u{00e9}cider la forme",
            "description": "Cr\u{00e9}e une t\u{00e2}che.",
            "tasks": [{"key": "design", "title": "Decide it"}]
        });
        assert!(validator.is_valid(&valid));

        for field in ["name", "title", "description"] {
            for empty in visually_empty_values() {
                let mut wrong = valid.clone();
                wrong[field] = serde_json::Value::String(empty.to_string());
                assert!(
                    !validator.is_valid(&wrong),
                    "schema accepted visually empty `{field}` value {empty:?}"
                );
            }
        }
    }

    /// Proves `blueprint.parameters_bind_totally_after_parsing` across every
    /// typed string slot. A hostile value stays one value in each existing
    /// slot, and its placeholder-looking suffix is not evaluated again.
    #[test]
    fn binding_is_total_field_aware_and_non_recursive() {
        let source = format!(
            "schema: {SCHEMA}\nname: feature\ntitle: 'title {{{{value}}}}'\n\
             description: 'description {{{{value}}}}'\nparameters:\n  - name: value\n    \
             description: Instance text.\ntasks:\n  - key: static-key\n    title: 'task {{{{value}}}}'\n    \
             id: 'id-{{{{value}}}}'\n    cut: 'cut-{{{{value}}}}'\n    owner: 'owner {{{{value}}}}'\n    \
             tags: ['tag {{{{value}}}}']\n    paths: ['path {{{{value}}}}']\n    \
             traces: ['trace {{{{value}}}}']\n    blocked_by: ['block {{{{value}}}}']\n    \
             blocks: ['blocks {{{{value}}}}']\n    \
             prefer_after: ['prefer {{{{value}}}}']\n    ratified_by: ['ratify {{{{value}}}}']\n    \
             contains: ['contain {{{{value}}}}']\n    revisit_at: 'visit {{{{value}}}}'\n    \
             version_impact: feature\n    version_impact_reason: 'reason {{{{value}}}}'\n    \
             evidence:\n      - kind: note\n        ref: 'ref {{{{value}}}}'\n        \
             captured: 'captured {{{{value}}}}'\n    body: 'body {{{{value}}}}'\n"
        );
        let blueprint = parse(std::path::Path::new("complete.yml"), &source).expect("parse");
        let value = "x:\n- not-a-member {{opaque}}";
        let bound = bind_parameters(
            &blueprint,
            &BTreeMap::from([("value".to_string(), value.to_string())]),
        )
        .expect("bind every typed string field");

        assert_eq!(bound.tasks.len(), 1);
        assert_eq!(bound.tasks[0].key, "static-key");
        assert_eq!(bound.name, "feature");
        for actual in [&bound.title, &bound.description] {
            assert!(actual.contains(value), "{actual:?}");
        }
        let task = &bound.tasks[0].task;
        for actual in [
            task.title.as_str(),
            task.id.as_deref().expect("id"),
            task.cut.as_deref().expect("cut"),
            task.owner.as_deref().expect("owner"),
            task.tags[0].as_str(),
            task.paths[0].as_str(),
            task.traces[0].as_str(),
            task.blocked_by[0].as_str(),
            task.blocks[0].as_str(),
            task.prefer_after[0].as_str(),
            task.ratified_by[0].as_str(),
            task.contains[0].as_str(),
            task.revisit_at.as_deref().expect("revisit"),
            task.version_impact_reason
                .as_deref()
                .expect("impact reason"),
            task.evidence[0].reference.as_str(),
            task.evidence[0].captured.as_deref().expect("captured"),
            task.body.as_deref().expect("body"),
        ] {
            assert!(actual.contains(value), "{actual:?}");
        }
        for values in [
            &task.tags,
            &task.paths,
            &task.traces,
            &task.blocked_by,
            &task.blocks,
            &task.prefer_after,
            &task.ratified_by,
            &task.contains,
        ] {
            assert_eq!(values.len(), 1, "a value injected a list member");
        }
        assert_eq!(task.evidence.len(), 1);
        assert!(
            task.title.contains("{{opaque}}"),
            "bound values are opaque, not a second source program: {}",
            task.title
        );
    }

    #[test]
    fn binding_refuses_every_incomplete_or_ambiguous_neighbour() {
        let parsed = || {
            parse(
                std::path::Path::new("binding.yml"),
                &document("  - key: design\n    title: 'Build {{feature}}'\n"),
            )
            .expect("binding fixture")
        };
        let values = BTreeMap::from([("feature".to_string(), "archive".to_string())]);

        assert!(matches!(
            bind_parameters(&parsed(), &BTreeMap::new()),
            Err(BindingError::MissingBinding { name }) if name == "feature"
        ));
        assert!(matches!(
            bind_parameters(
                &parsed(),
                &BTreeMap::from([("feature".to_string(), "\u{200b}".to_string())])
            ),
            Err(BindingError::EmptyBinding { name }) if name == "feature"
        ));

        let mut unknown_placeholder = parsed();
        unknown_placeholder.tasks[0].task.title = "Build {{missing}}".to_string();
        assert!(matches!(
            bind_parameters(&unknown_placeholder, &values),
            Err(BindingError::UnknownPlaceholder { name, .. }) if name == "missing"
        ));

        let mut unused = parsed();
        unused.tasks[0].task.title = "Build archive".to_string();
        assert!(matches!(
            bind_parameters(&unused, &values),
            Err(BindingError::UnusedParameter { name }) if name == "feature"
        ));

        let mut unknown_binding = values.clone();
        unknown_binding.insert("extra".to_string(), "unused".to_string());
        assert!(matches!(
            bind_parameters(&parsed(), &unknown_binding),
            Err(BindingError::UnknownBinding { name }) if name == "extra"
        ));

        let mut duplicate = parsed();
        duplicate.parameters.push(duplicate.parameters[0].clone());
        assert!(matches!(
            bind_parameters(&duplicate, &values),
            Err(BindingError::DuplicateParameter { name }) if name == "feature"
        ));

        let mut invalid_name = parsed();
        invalid_name.parameters[0].name = "not valid".to_string();
        assert!(matches!(
            bind_parameters(&invalid_name, &values),
            Err(BindingError::InvalidParameterName { .. })
        ));

        let mut empty_description = parsed();
        empty_description.parameters[0].description = "\u{200b}".to_string();
        assert!(matches!(
            bind_parameters(&empty_description, &values),
            Err(BindingError::InvalidParameterDescription { .. })
        ));

        let mut structural_description = parsed();
        structural_description.parameters[0].description = "Describe {{feature}}".to_string();
        assert!(matches!(
            bind_parameters(&structural_description, &values),
            Err(BindingError::StructuralPlaceholder { field, name })
                if field == "parameters[0].description" && name == "feature"
        ));

        let mut structural = parsed();
        structural.tasks[0].key = "{{feature}}".to_string();
        assert!(matches!(
            bind_parameters(&structural, &values),
            Err(BindingError::StructuralPlaceholder { field, name })
                if field == "tasks[0].key" && name == "feature"
        ));

        for malformed in ["{{feature", "feature }}", "{{}}", "{{bad name}}"] {
            let mut blueprint = parsed();
            blueprint.tasks[0].task.title = malformed.to_string();
            assert!(matches!(
                bind_parameters(&blueprint, &values),
                Err(BindingError::MalformedPlaceholder { .. })
            ));
        }
    }

    #[test]
    fn zero_and_multiple_parameters_are_both_total() {
        let none = parse(
            std::path::Path::new("none.yml"),
            &format!(
                "schema: {SCHEMA}\nname: plain\ntitle: No parameters\n\
                 description: No values are needed.\ntasks:\n  - key: only\n    title: Plain task\n"
            ),
        )
        .expect("parameter-free blueprint");
        assert_eq!(
            bind_parameters(&none, &BTreeMap::new())
                .expect("a parameter-free document is already total")
                .tasks[0]
                .task
                .title,
            "Plain task"
        );

        let mut multiple = none;
        multiple.parameters = vec![
            Parameter {
                name: "first".to_string(),
                description: "First value.".to_string(),
            },
            Parameter {
                name: "second-value".to_string(),
                description: "Second value.".to_string(),
            },
        ];
        multiple.title = "{{second-value}} then {{first}}".to_string();
        let bound = bind_parameters(
            &multiple,
            &BTreeMap::from([
                ("first".to_string(), "one".to_string()),
                ("second-value".to_string(), "two".to_string()),
            ]),
        )
        .expect("all declarations supplied and used");
        assert_eq!(bound.title, "two then one");

        assert!(matches!(
            bind_parameters(
                &multiple,
                &BTreeMap::from([("first".to_string(), "one".to_string())])
            ),
            Err(BindingError::MissingBinding { name }) if name == "second-value"
        ));
    }

    #[test]
    fn placeholder_escapes_are_structural_and_do_not_count_as_parameter_use() {
        let mut blueprint = parse(
            std::path::Path::new("escape.yml"),
            &document(
                "  - key: design\n    title: 'Literal {{{{feature}}}} and bound {{{{feature}}}}'\n    \
                 body: 'opening {{{{ and closing }}}}'\n",
            ),
        )
        .expect("escape fixture");
        // The YAML source above has doubled Rust-format braces only where
        // `document` itself formats nothing. Make the first occurrence the
        // four-brace escape and retain the second as a real placeholder.
        blueprint.tasks[0].task.title = "Literal {{{{feature}}}} and bound {{feature}}".to_string();
        blueprint.tasks[0].task.body = Some("opening {{{{ and closing }}}}".to_string());
        let bound = bind_parameters(
            &blueprint,
            &BTreeMap::from([("feature".to_string(), "{{opaque}}".to_string())]),
        )
        .expect("escaped delimiters and one real use");
        assert_eq!(
            bound.tasks[0].task.title,
            "Literal {{feature}} and bound {{opaque}}"
        );
        assert_eq!(
            bound.tasks[0].task.body.as_deref(),
            Some("opening {{ and closing }}")
        );

        let mut escaped_only = blueprint;
        escaped_only.tasks[0].task.title = "Literal {{{{feature}}}}".to_string();
        assert!(matches!(
            bind_parameters(
                &escaped_only,
                &BTreeMap::from([("feature".to_string(), "unused".to_string())])
            ),
            Err(BindingError::UnusedParameter { name }) if name == "feature"
        ));
    }

    fn property_names<T: JsonSchema>() -> BTreeSet<String> {
        fn walk(
            node: &serde_json::Value,
            defs: &serde_json::Map<String, serde_json::Value>,
            out: &mut BTreeSet<String>,
        ) {
            if let Some(properties) = node
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                out.extend(properties.keys().cloned());
            }
            if let Some(reference) = node.get("$ref").and_then(serde_json::Value::as_str)
                && let Some(name) = reference.strip_prefix("#/$defs/")
                && let Some(target) = defs.get(name)
            {
                walk(target, defs, out);
            }
            for union in ["allOf", "anyOf", "oneOf"] {
                if let Some(branches) = node.get(union).and_then(serde_json::Value::as_array) {
                    for branch in branches {
                        walk(branch, defs, out);
                    }
                }
            }
        }

        let schema = schemars::schema_for!(T);
        let root = schema.as_value();
        let defs = root
            .get("$defs")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut out = BTreeSet::new();
        walk(root, &defs, &mut out);
        out
    }

    #[test]
    fn a_blueprint_entry_is_the_manifest_entry_plus_only_key() {
        let manifest = property_names::<crate::manifest::Entry>();
        let mut blueprint = property_names::<Entry>();
        assert!(blueprint.remove("key"), "the local key is published");
        assert_eq!(
            blueprint, manifest,
            "the two entry surfaces cannot become separate dialects"
        );
    }

    #[test]
    fn schema_is_language_compatibility_not_recipe_revision() {
        let wrong = document("  - key: design\n    title: Decide it\n").replacen(
            SCHEMA,
            "feature-recipe/v2",
            1,
        );
        let error = parse(std::path::Path::new("wrong.yml"), &wrong)
            .expect_err("an author-controlled revision is not a schema")
            .to_string();
        assert!(error.contains("feature-recipe/v2"), "{error}");
        assert!(error.contains(SCHEMA), "{error}");

        let revision = document("  - key: design\n    title: Decide it\n").replacen(
            "name: feature\n",
            "name: feature\nrevision: 7\n",
            1,
        );
        let error = parse(std::path::Path::new("revision.yml"), &revision)
            .expect_err("v1 has no recipe revision")
            .to_string();
        assert!(error.contains("unknown field `revision`"), "{error}");
        assert!(
            error.contains("schema") && error.contains("tasks"),
            "{error}"
        );
    }

    #[test]
    fn application_assignment_is_exact_unique_and_usable() {
        let source = document(concat!(
            "  - key: design\n",
            "    title: Decide it\n",
            "  - key: implement\n",
            "    title: Implement it\n",
            "    blocked_by: [local:design, task:EXISTING]\n",
        ));
        let blueprint = parse(std::path::Path::new("graph.yml"), &source).expect("blueprint");
        assert_eq!(
            application_keys(&blueprint).expect("keys"),
            ["design", "implement"]
        );

        for (assigned, cut, expected) in [
            (
                BTreeMap::from([("design".to_string(), "A-1".to_string())]),
                "M-v1",
                ReferenceError::MissingAssignment {
                    key: "implement".to_string(),
                },
            ),
            (
                BTreeMap::from([
                    ("design".to_string(), "A-1".to_string()),
                    ("implement".to_string(), "A-2".to_string()),
                    ("other".to_string(), "A-3".to_string()),
                ]),
                "M-v1",
                ReferenceError::UnknownAssignment {
                    key: "other".to_string(),
                },
            ),
            (
                BTreeMap::from([
                    ("design".to_string(), "A-1".to_string()),
                    ("implement".to_string(), "A-1".to_string()),
                ]),
                "M-v1",
                ReferenceError::DuplicateAssignedId {
                    first: "design".to_string(),
                    second: "implement".to_string(),
                    id: "A-1".to_string(),
                },
            ),
            (
                BTreeMap::from([
                    ("design".to_string(), "../bad".to_string()),
                    ("implement".to_string(), "A-2".to_string()),
                ]),
                "M-v1",
                ReferenceError::InvalidAssignedId {
                    key: "design".to_string(),
                    id: "../bad".to_string(),
                },
            ),
            (
                BTreeMap::from([
                    ("design".to_string(), "A-1".to_string()),
                    ("implement".to_string(), "A-2".to_string()),
                ]),
                "\u{200b}",
                ReferenceError::InvalidApplicationCut,
            ),
        ] {
            assert_eq!(
                resolve_application(&blueprint, &assigned, cut).expect_err("invalid assignment"),
                expected
            );
        }

        let resolved = resolve_application(
            &blueprint,
            &BTreeMap::from([
                ("design".to_string(), "A-1".to_string()),
                ("implement".to_string(), "A-2".to_string()),
            ]),
            "M-v1",
        )
        .expect("complete assignment");
        assert_eq!(resolved[1].blocked_by, ["A-1", "EXISTING"]);
    }

    #[test]
    fn one_shot_v1_publishes_only_the_accepted_document_and_parameter_fields() {
        assert_eq!(
            property_names::<Blueprint>(),
            BTreeSet::from([
                "description".to_string(),
                "name".to_string(),
                "parameters".to_string(),
                "schema".to_string(),
                "tasks".to_string(),
                "title".to_string(),
            ]),
            "v1 has no revision, provenance, discovery, composition or execution fields"
        );
        assert_eq!(
            property_names::<Parameter>(),
            BTreeSet::from(["description".to_string(), "name".to_string()]),
            "every v1 parameter is required, with no implicit or environment-derived default"
        );
    }

    #[test]
    fn unknown_fields_name_the_accepted_set_at_each_level() {
        let top = document("  - key: design\n    title: Decide it\n").replacen(
            "name: feature\n",
            "name: feature\ncatalog: shared\n",
            1,
        );
        let error = parse(std::path::Path::new("top.yml"), &top)
            .expect_err("top-level unknown")
            .to_string();
        assert!(error.contains("unknown field `catalog`"), "{error}");
        assert!(
            error.contains("schema") && error.contains("tasks"),
            "{error}"
        );

        let parameter = document("  - key: design\n    title: Decide it\n").replacen(
            "    description: The feature name.\n",
            "    description: The feature name.\n    required: true\n",
            1,
        );
        let error = parse(std::path::Path::new("parameter.yml"), &parameter)
            .expect_err("parameter unknown")
            .to_string();
        assert!(error.contains("unknown field `required`"), "{error}");
        assert!(
            error.contains("name") && error.contains("description"),
            "{error}"
        );

        let task = document("  - key: design\n    title: Decide it\n    efort: 1\n");
        let error = parse(std::path::Path::new("task.yml"), &task)
            .expect_err("entry unknown")
            .to_string();
        assert!(error.contains("unknown field `efort`"), "{error}");
        for accepted in ["key", "title", "effort", "blocked_by", "blocks", "body"] {
            assert!(
                error.contains(accepted),
                "`{accepted}` absent from: {error}"
            );
        }
    }

    #[test]
    fn an_empty_blueprint_is_a_refusal_in_the_parser_and_schema() {
        let empty = document("");
        assert!(matches!(
            parse(std::path::Path::new("empty.yml"), &empty),
            Err(BlueprintError::Empty { .. })
        ));
        let schema = derived_schema();
        let tasks = schema.pointer("/properties/tasks").expect("tasks property");
        assert_eq!(
            tasks.get("minItems").and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn every_published_key_says_what_it_is() {
        fn walk(node: &serde_json::Value, path: &str, silent: &mut BTreeSet<String>) {
            if let Some(properties) = node
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                for (name, property) in properties {
                    let child = if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{path}.{name}")
                    };
                    let description = property
                        .get("description")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    if description.trim().is_empty() {
                        silent.insert(child.clone());
                    }
                    walk(property, &child, silent);
                }
            }
        }

        let schema = derived_schema();
        let mut silent = BTreeSet::new();
        walk(&schema, "", &mut silent);
        if let Some(defs) = schema.get("$defs").and_then(serde_json::Value::as_object) {
            for (name, definition) in defs {
                walk(definition, name, &mut silent);
            }
        }
        assert!(
            silent.is_empty(),
            "published blueprint keys without descriptions: {silent:?}"
        );
        assert!(
            property_names::<super::Blueprint>().len() >= 6,
            "the audit must see the top-level surface"
        );
    }

    #[test]
    fn published_descriptions_are_self_contained() {
        let published = derived_schema().to_string();
        for internal_reference in ["A1-", "ADR-", "docs/pilots/", "[`"] {
            assert!(
                !published.contains(internal_reference),
                "published schema contains repository-internal reference `{internal_reference}`"
            );
        }
    }

    #[test]
    fn file_errors_keep_the_selected_path_and_cause() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let missing = dir.path().join("missing-blueprint.yml");
        let error = read(&missing).expect_err("missing");
        assert!(matches!(error, BlueprintError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("missing-blueprint.yml"));
        assert!(std::error::Error::source(&error).is_some());
    }
}
