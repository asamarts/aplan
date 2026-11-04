//! The `rmcp` adapter: a thin shell over [`crate::tools`].
//!
//! Everything that decides *whether* something may happen lives in `tools`; this
//! module only moves JSON. Keeping it thin is the point: a rule that lived here
//! would be a rule the CLI does not have, and the guarantee this crate exists to
//! make is that both surfaces refuse the same things.
//!
//! `#[non_exhaustive]` on rmcp's model types forbids a struct literal from
//! another crate, so every value here is built from `Default` and then assigned
//! field by field. That is the only construction available, not a style choice,
//! hence the module-level allow below; it also means a field rmcp adds later
//! arrives with its own default instead of breaking this build.
#![allow(clippy::field_reassign_with_default)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::tools::{self, TOOLS};

type StoreKey = (PathBuf, Option<PathBuf>);
type PlanStores = Arc<Mutex<std::collections::BTreeMap<StoreKey, aplan_app::PlanStore>>>;

#[derive(Debug, serde::Serialize)]
struct ContextProvenance {
    selection: aplan_app::ExplicitRootKind,
    /// Paths are relative to the selected plan's repository. A configuration
    /// outside it is named as such rather than exposing an unrelated host path.
    repository_root: &'static str,
    plan_root: String,
    configuration: Option<String>,
    resolved_symlink: bool,
}

struct CallContext {
    root: PathBuf,
    policy: crate::Policy,
    provenance: Option<ContextProvenance>,
    prefetched: Option<aplan_core::IdentifiedPlan>,
}

impl CallContext {
    fn wrap(&self, value: serde_json::Value) -> serde_json::Value {
        match &self.provenance {
            Some(context) => serde_json::json!({
                "context": context,
                "result": value,
            }),
            None => value,
        }
    }
}

/// The server: a plan root, the policy `check` runs under, and what this build
/// is.
#[derive(Debug, Clone)]
pub struct AplanServer {
    root: PathBuf,
    policy: Arc<crate::Policy>,
    stores: PlanStores,
    cache_mode: aplan_app::CacheMode,
    cache_writer_identity: Option<String>,
    cache_root: Option<PathBuf>,
    /// This binary's `aplan/tool-identity/v1` document, handed over by the
    /// caller rather than computed here.
    ///
    /// The identity is a fact about the *serving executable*, and the crate that
    /// can state it is the one that owns the facts document it hashes. Computing
    /// a second one here is precisely what `A1-259` forbids: two documents that
    /// agree until they do not. Passed at construction because the server is
    /// that executable, so there is nothing to keep it current against.
    identity: Arc<serde_json::Value>,
    /// Provenance used by the generated `AGENTS.md` contract rule. Supplied by
    /// the binary because it owns the complete rendered-surface digest.
    written_by: Option<Arc<(String, String)>>,
    /// Why this server can do nothing but identify itself, when that is the
    /// case.
    ///
    /// A configuration this build cannot read used to stop `aplan mcp` before it
    /// served, which left an agent talking to a dead process in exactly the case
    /// where *which build are you* is the answer: an old binary rejects a config
    /// key a newer aplan added, and that is how a stale binary usually announces
    /// itself (`A1-259`, audit). The server starts; every tool but
    /// `aplan_identity` returns this, so nothing runs under a policy nobody
    /// chose.
    unconfigured: Option<Arc<String>>,
}

impl AplanServer {
    /// A server over `root`, checking with `options` (the repository's
    /// `.aplan.yml` policy, resolved by the caller exactly as the CLI resolves
    /// it, so MCP and the CLI judge a plan identically), reporting `identity`
    /// as its build.
    pub fn new(
        root: PathBuf,
        policy: crate::Policy,
        identity: serde_json::Value,
        cache_mode: aplan_app::CacheMode,
        cache_writer_identity: String,
        written_by: (String, String),
    ) -> Self {
        Self {
            root,
            policy: Arc::new(policy),
            stores: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            cache_mode,
            cache_writer_identity: Some(cache_writer_identity),
            cache_root: None,
            identity: Arc::new(identity),
            written_by: Some(Arc::new(written_by)),
            unconfigured: None,
        }
    }

    /// A server that can only say what it is, because its configuration could
    /// not be read.
    ///
    /// The plan root is irrelevant here and every tool but `aplan_identity`
    /// refuses with `said`.
    #[must_use]
    pub fn unconfigured(said: String, identity: serde_json::Value) -> Self {
        Self {
            root: PathBuf::from("."),
            policy: Arc::new(crate::Policy::default()),
            stores: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            cache_mode: aplan_app::CacheMode::Off,
            cache_writer_identity: None,
            cache_root: None,
            identity: Arc::new(identity),
            written_by: None,
            unconfigured: Some(Arc::new(said)),
        }
    }

    fn store_key(root: &std::path::Path, policy: &crate::Policy) -> (PathBuf, Option<PathBuf>) {
        let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        (root, policy.verify.config_file.clone())
    }

    fn plan_snapshot(
        &self,
        root: &std::path::Path,
        policy: &crate::Policy,
    ) -> Result<aplan_core::IdentifiedPlan, String> {
        let key = Self::store_key(root, policy);
        let mut stores = self
            .stores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let store = match stores.entry(key) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry
                    .get_mut()
                    .refresh()
                    .map_err(|error| format!("reading plan: {error}"))?;
                entry.into_mut()
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                let options = aplan_app::PlanStoreOptions {
                    mode: self.cache_mode,
                    configuration_file: policy.verify.config_file.clone(),
                    cache_root: self.cache_root.clone(),
                    writer_identity: self.cache_writer_identity.clone(),
                    // MCP responses expose typed task fields, never Markdown
                    // prose bodies, so exact digests are sufficient identity.
                    retain_inputs: false,
                    defer_publication: false,
                };
                let store = aplan_app::PlanStore::open(root, &options)
                    .map_err(|error| format!("reading plan: {error}"))?;
                entry.insert(store)
            }
        };
        Ok(store.snapshot().clone())
    }

    fn plan_snapshot_with_report_cache(
        &self,
        root: &std::path::Path,
        policy: &crate::Policy,
        prefetched: Option<aplan_core::IdentifiedPlan>,
    ) -> Result<
        (
            aplan_core::IdentifiedPlan,
            Option<aplan_app::StructuralReportCache>,
        ),
        String,
    > {
        let store_key = Self::store_key(root, policy);
        let plan = match prefetched {
            Some(plan) => plan,
            None => self.plan_snapshot(root, policy)?,
        };
        let mut stores = self
            .stores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(store) = stores.get_mut(&store_key) else {
            return Ok((plan, None));
        };
        let Some(key) = store.structural_report_key(&policy.options) else {
            return Ok((plan, None));
        };
        let cached = store.cached_structural_report(&key);
        let stores = Arc::clone(&self.stores);
        let options = policy.options.clone();
        let cache = aplan_app::StructuralReportCache::new(cached, move |report| {
            let mut stores = stores
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(store) = stores.get_mut(&store_key) {
                store.remember_structural_report(&key, &options, report);
            }
        });
        Ok((plan, Some(cache)))
    }

    fn snapshot_for_call(
        &self,
        root: &std::path::Path,
        policy: &crate::Policy,
        prefetched: &mut Option<aplan_core::IdentifiedPlan>,
    ) -> Result<aplan_core::IdentifiedPlan, String> {
        match prefetched.take() {
            Some(plan) => Ok(plan),
            None => self.plan_snapshot(root, policy),
        }
    }

    fn report_seed(
        &self,
        root: &std::path::Path,
        policy: &crate::Policy,
    ) -> Option<(String, Option<aplan_core::Report>)> {
        let store_key = Self::store_key(root, policy);
        let stores = self
            .stores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let store = stores.get(&store_key)?;
        let key = store.structural_report_key(&policy.options)?;
        let cached = store.cached_structural_report(&key);
        Some((key, cached))
    }

    fn remember_report(
        &self,
        root: &std::path::Path,
        policy: &crate::Policy,
        key: &str,
        report: &aplan_core::Report,
    ) {
        let store_key = Self::store_key(root, policy);
        let mut stores = self
            .stores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(store) = stores.get_mut(&store_key) {
            store.remember_structural_report(key, &policy.options, report);
        }
    }

    fn call_context(&self, explicit: Option<&str>) -> Result<CallContext, String> {
        let Some(explicit) = explicit else {
            return Ok(CallContext {
                root: self.root.clone(),
                policy: (*self.policy).clone(),
                provenance: None,
                prefetched: None,
            });
        };
        let resolved = aplan_app::resolve_explicit_root(explicit).map_err(|error| {
            format!(
                "cannot resolve MCP `root` as one repository context: {error}. The server's \
                 launch repository was not used as a fallback."
            )
        })?;
        let default_traces = resolved
            .configuration_directory
            .clone()
            .or_else(|| Some(resolved.repository_root.clone()));
        let mut options = aplan_app::configured_options(
            &resolved.config,
            aplan_app::ConfigurationInputs {
                directory: resolved.configuration_directory.as_deref(),
                file: resolved.configuration_file.clone(),
                no_require_effort: false,
                traces_root: None,
                no_traces: false,
                default_traces,
                traces_repository: Some(resolved.repository_root.clone()),
            },
        );
        refuse_missing_external_roots(&options)?;
        let tools = TOOLS
            .iter()
            .map(|tool| (tool.name.to_string(), tool.description.to_string()))
            .collect::<Vec<_>>();
        let agent_root = resolved
            .configuration_directory
            .as_deref()
            .unwrap_or(&resolved.repository_root);
        let initial_policy = crate::Policy {
            options: options.clone(),
            verdict: aplan_app::configured_verdict(&resolved.config),
            verify: aplan_app::configured_verification(
                &resolved.config,
                resolved.configuration_file.clone(),
                options.traces_root.clone(),
            ),
            repo_root: resolved.repository_root.clone(),
            id_strategy: resolved.config.id_strategy.unwrap_or_default(),
        };
        let prefetched = if agent_root.join("AGENTS.md").is_file() {
            Some(self.plan_snapshot(&resolved.plan_root, &initial_policy)?)
        } else {
            None
        };
        let written_by = self
            .written_by
            .as_deref()
            .map(|value| (value.0.as_str(), value.1.as_str()));
        if let Some(snapshot) = &prefetched {
            options = aplan_app::configured_agents_block(
                options,
                &resolved.plan_root,
                agent_root,
                &resolved.config,
                &snapshot.plan,
                written_by,
                &tools,
            );
        }
        let policy = crate::Policy {
            verify: aplan_app::configured_verification(
                &resolved.config,
                resolved.configuration_file.clone(),
                options.traces_root.clone(),
            ),
            options,
            verdict: aplan_app::configured_verdict(&resolved.config),
            repo_root: resolved.repository_root.clone(),
            id_strategy: resolved.config.id_strategy.unwrap_or_default(),
        };
        let provenance = ContextProvenance {
            selection: resolved.selection,
            repository_root: ".",
            plan_root: bounded_path(&resolved.repository_root, &resolved.plan_root),
            configuration: resolved
                .configuration_file
                .as_deref()
                .map(|path| bounded_path(&resolved.repository_root, path)),
            resolved_symlink: resolved.followed_symlink,
        };
        Ok(CallContext {
            root: resolved.plan_root,
            policy,
            provenance: Some(provenance),
            prefetched,
        })
    }

    /// Dispatch one call. Split out so it is a plain function returning a value,
    /// which is what makes the wire behaviour testable without a transport.
    // Keep the exhaustive tool match in one place: splitting it would recreate
    // the parity table's held-entry failure mode at the server boundary.
    #[allow(clippy::too_many_lines)]
    fn dispatch(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        let str_arg = |key: &str| -> Option<String> {
            args.get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        // Identity first, and before the refusal below: the whole reason a
        // server with no usable configuration starts at all is that something
        // has to be able to say which build is refusing (`A1-259`, audit).
        if name == "aplan_identity" {
            return Ok((*self.identity).clone());
        }
        if let Some(said) = &self.unconfigured {
            return Err(format!(
                "{said}\n\nThis server could not read its configuration, so it can do nothing \
                 but identify itself: call `aplan_identity` to see which build is refusing. A \
                 config key a newer aplan added reads exactly like a typo from an older one."
            ));
        }

        let explicit_root = match args.get("root") {
            None => None,
            Some(serde_json::Value::String(root)) => Some(root.as_str()),
            Some(_) => return Err("`root` must be a string naming a directory".to_string()),
        };
        let mut context = self.call_context(explicit_root)?;
        let mut prefetched = context.prefetched.take();
        let root = context.root.as_path();
        let policy = &context.policy;

        let value = match name {
            "aplan_ready" => self
                .snapshot_for_call(root, policy, &mut prefetched)
                .and_then(|plan| {
                    let claims = aplan_app::claims::snapshot(root, &plan.plan)
                        .map_err(|error| error.to_string())?;
                    Ok(serde_json::json!(tools::ready_of(&plan.plan, &claims)))
                }),
            "aplan_why" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                // The struct serialises with `blocked_by` at the top, which is
                // what this response has always been, plus the three facts the
                // CLI's human output already showed (`A1-182`).
                let all = tools::all_flag(args.get("all")).map_err(|e| e.to_string())?;
                let plan = self.snapshot_for_call(root, policy, &mut prefetched)?;
                tools::why_of(&plan.plan, &id, tools::walk_for(all))
                    .map(|b| serde_json::json!(b))
                    .map_err(|e| e.to_string())
            }
            "aplan_verify" => {
                let plan = self.snapshot_for_call(root, policy, &mut prefetched)?;
                let seed = self.report_seed(root, policy);
                let cached = seed.as_ref().and_then(|(_, report)| report.clone());
                let (value, structural) = tools::verify_of_cached(&plan.plan, policy, cached)
                    .map_err(|e| e.to_string())?;
                if let Some((key, None)) = seed {
                    self.remember_report(root, policy, &key, &structural);
                }
                Ok(value)
            }
            "aplan_add" => {
                // `root` is handled above and is not part of the task, so it is
                // dropped before deserialising into a struct that denies unknown
                // fields.
                let mut fields = args.clone();
                if let Some(obj) = fields.as_object_mut() {
                    obj.remove("root");
                }
                let add: tools::AddArgs =
                    serde_json::from_value(fields).map_err(|e| format!("bad arguments: {e}"))?;
                tools::add(root, &add, policy).map_err(|e| e.to_string())
            }
            "aplan_show" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                let plan = self.snapshot_for_call(root, policy, &mut prefetched)?;
                let seed = self.report_seed(root, policy);
                let cached = seed.as_ref().and_then(|(_, report)| report.clone());
                let claims = aplan_app::claims::snapshot(root, &plan.plan)
                    .map_err(|error| error.to_string())?;
                let (value, structural) =
                    tools::show_of_cached_with_claims(&plan.plan, &id, policy, cached, &claims)
                        .map_err(|e| e.to_string())?;
                if let Some((key, None)) = seed {
                    self.remember_report(root, policy, &key, &structural);
                }
                Ok(value)
            }
            // `A1-359`'s four. They land on this surface in the same change as
            // the commands, because the actor the status pair is justified by
            // reaches aplan only through here: an agent with tools and no
            // editor cannot hand-edit a status, which is what `A1-077`'s
            // refusal assumed it could.
            "aplan_claim" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                tools::claim(
                    root,
                    &id,
                    str_arg("as").as_deref(),
                    str_arg("from").as_deref(),
                    policy,
                )
                .map_err(|e| e.to_string())
            }
            "aplan_release" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                tools::release(root, &id, policy).map_err(|e| e.to_string())
            }
            "aplan_start" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                tools::working(root, &id, true, policy).map_err(|e| e.to_string())
            }
            "aplan_stop" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                tools::working(root, &id, false, policy).map_err(|e| e.to_string())
            }
            "aplan_dep" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                let after = str_arg("after").ok_or("`after` is required")?;
                let add = args
                    .get("add")
                    .and_then(serde_json::Value::as_bool)
                    .ok_or("`add` is required: true to add the edge, false to remove it")?;
                tools::dep(root, &id, &after, add, policy).map_err(|e| e.to_string())
            }
            "aplan_check" => {
                let (plan, cache) =
                    self.plan_snapshot_with_report_cache(root, policy, prefetched.take())?;
                tools::check_of_cached(plan, policy, cache).map_err(|e| e.to_string())
            }
            "aplan_close" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                let evidence: Vec<String> = args
                    .get("evidence")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                tools::close(root, &id, &evidence, policy).map_err(|e| e.to_string())
            }
            other => Err(format!("unknown tool `{other}`")),
        }?;
        Ok(context.wrap(value))
    }
}

fn refuse_missing_external_roots(options: &aplan_core::Options) -> Result<(), String> {
    let missing = options
        .traces_external
        .iter()
        .filter(|path| !path.is_dir())
        .map(|path| aplan_core::anchor::normalise(path).display().to_string())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "selected configuration declares absent `traces_external` root(s): {}. The explicit \
         MCP root is refused rather than being combined with the server's launch context.",
        missing.join(", ")
    ))
}

fn bounded_path(repository: &std::path::Path, path: &std::path::Path) -> String {
    let relative = path.strip_prefix(repository).ok();
    match relative {
        Some(path) if path.as_os_str().is_empty() => ".".to_string(),
        Some(path) => path.to_string_lossy().replace('\\', "/"),
        None => "<outside-selected-repository>".to_string(),
    }
}

/// The JSON Schema for a tool's arguments.
///
/// Written out rather than derived, because the schema *is* the contract an
/// agent reads. `close` is the one that matters: its
/// only inputs are a task id and evidence citations, and there is deliberately
/// no field through which a caller could assert completion.
fn input_schema(name: &str) -> serde_json::Value {
    let root = serde_json::json!({
        "type": "string",
        "description": "Repository root containing `.aplan.yml`, or that repository's configured plan directory. The server resolves plan, configuration, traces, evidence providers and Git as one context; an ambiguous partial directory is refused. Omit this field to use the server's launch context. An explicit selection returns `{context, result}` so the resolved frame is auditable.",
    });
    match name {
        // No arguments at all, not even `root`: the identity is a fact about
        // this process, and accepting a root would suggest otherwise.
        "aplan_identity" => serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        }),
        "aplan_why" => serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Task id." },
                "all": {
                    "type": "boolean",
                    "description": "Walk through cuts into their membership as well. Off \
                                     by default: a cut is reported as itself, with \
                                     `behind` counting what was folded, because the \
                                     complete closure of a sequenced plan is mostly the \
                                     roadmap you can query separately. Every answer \
                                     names its `traversal`.",
                },
                "root": root,
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        "aplan_add" => add_schema(&root),
        _ => other_schema(name, &root),
    }
}

/// `aplan_add`'s own schema: the one tool with more fields than the rest put
/// together, lifted out so [`input_schema`] stays a readable table.
fn add_schema(root: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "What the task is, in a line." },
                "id": {
                    "type": "string",
                    "description": "Explicit id. An ordinary add may omit it to mint from the plan convention; a non-empty members request requires it.",
                },
                "phase": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Reporting phase. An ordinary add may omit it to derive from the selected cut or plan; a non-empty members request requires it.",
                },
                "cut": {
                    "type": "string",
                    "description": "The cut to place it in: its milestone id (`M-v0.8`) or \
                                    the label it reports itself under (`v0.8`). A plan with \
                                    `require_cut_membership` rejects a task in no cut.",
                },
                "effort_days": { "type": "number", "description": "Estimate in days." },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Globs this task owns. Without them its future evidence \
                                    cannot be bound to it (`APLAN023`).",
                },
                "traces": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Document sections this task answers to, as \
                                    `path/to/doc.md#anchor`: what caused it, which exists, \
                                    rather than what it will write. An anchor that does not \
                                    resolve is refused (`APLAN008`).",
                },
                "tags": { "type": "array", "items": { "type": "string" } },
                "blocked_by": { "type": "array", "items": { "type": "string" } },
                "members": {
                    "type": "array",
                    "items": { "type": "string" },
                    "uniqueItems": true,
                    "description": "Existing live task ids moved from exactly one source cut each into the new milestone this add creates. A non-empty list requires id, phase and a tags entry equal to milestone, forbids cut, and reserves blocked_by for milestone predecessors.",
                },
                "owner": { "type": "string" },
                "status": {
                    "type": "string",
                    "enum": ["todo", "in_progress", "blocked"],
                    "description": "Initial live status. Terminal history uses the CLI evidence-resolving manifest ledger.",
                },
                "prefer_after": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Advisory predecessor ids.",
                },
                "ratified_by": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Decision-record task ids.",
                },
                "contains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Release or container member ids.",
                },
                "revisit_at": {
                    "type": "string",
                    "format": "date-time",
                    "description": "RFC 3339 instant for an initially blocked task.",
                },
                "version_impact": {
                    "type": "string",
                    "enum": ["none", "fix", "feature", "breaking"],
                },
                "version_impact_reason": {
                    "type": "string",
                    "description": "Why the declared version impact applies.",
                },
                "root": root,
            },
        "required": ["title"],
        "allOf": [{
            "if": {
                "required": ["members"],
                "properties": { "members": { "minItems": 1 } },
            },
            "then": {
                "required": ["id", "phase", "tags"],
                "properties": {
                    "tags": { "contains": { "const": "milestone" } },
                },
                "not": { "required": ["cut"] },
            },
        }],
        "additionalProperties": false,
    })
}

/// Every other tool's schema.
fn other_schema(name: &str, root: &serde_json::Value) -> serde_json::Value {
    match name {
        "aplan_show" => serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The task to show." },
                "root": root,
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        "aplan_dep" => serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The task that comes second." },
                "after": {
                    "type": "string",
                    "description": "The task it comes after, which must finish first.",
                },
                "add": {
                    "type": "boolean",
                    "description": "True to record the edge, false to remove it.",
                },
                "root": root,
            },
            "required": ["id", "after", "add"],
            "additionalProperties": false,
        }),
        "aplan_close" => serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Task id to close." },
                "evidence": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Evidence as `kind:ref`, e.g. `commit:d213c9a`, \
                                    `test:mod::name`, `file:path`, `url:https://...`. \
                                    Required: a status is a claim, and this server will not \
                                    make it without proof to resolve.",
                },
                "root": root,
            },
            "required": ["id", "evidence"],
            "additionalProperties": false,
        }),
        // `A1-359`'s four. **Each needs its own entry**: the fallback below
        // allows `root` alone with `additionalProperties: false`, so a tool
        // that takes an `id` and is left to it advertises a schema that
        // rejects its own required argument.
        "aplan_claim" => serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The task to claim." },
                "as": {
                    "type": "string",
                    "description": "The name to record. Otherwise `APLAN_OWNER`, then per-user \
                                    `me`, this repository's `git config user.name`, then the \
                                    operating system's user. Never read from `.aplan.yml`.",
                },
                "from": {
                    "type": "string",
                    "description": "Who the task is being taken from. Required when somebody \
                                    holds it, and checked rather than taken on trust.",
                },
                "root": root,
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        "aplan_release" => serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The task to release." },
                "root": root,
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        "aplan_start" | "aplan_stop" => serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The task whose `in_progress` status is being set or cleared.",
                },
                "root": root,
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        // `ready`, `check` and `verify` take only the optional root.
        _ => serde_json::json!({
            "type": "object",
            "properties": { "root": root },
            "additionalProperties": false,
        }),
    }
}

/// The tool list exactly as it goes on the wire.
///
/// Public so a test can pin it: the surface is a cost an agent pays on every
/// call, so it must not grow by accident.
pub fn wire_tools() -> Vec<Tool> {
    TOOLS
        .iter()
        .map(|spec| {
            let schema = input_schema(spec.name);
            // `Tool` is `#[non_exhaustive]`, so it is built from its default
            // rather than a struct literal: fields rmcp adds later then arrive
            // with their own defaults instead of breaking the build.
            let mut tool = Tool::default();
            tool.name = spec.name.into();
            tool.description = Some(spec.description.into());
            tool.input_schema = Arc::new(
                schema
                    .as_object()
                    .cloned()
                    .expect("input schemas are JSON objects"),
            );
            tool
        })
        .collect()
}

impl ServerHandler for AplanServer {
    fn get_info(&self) -> InitializeResult {
        let mut info = InitializeResult::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("aplan", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "Validate and advance an evidence-gated plan. `aplan_close` requires evidence: \
             a citation something else resolves, never an assessment of your own work."
                .to_string(),
        );
        info
    }

    // Neither of these awaits anything, and saying so is the point: the whole
    // dispatch below is the synchronous core this server wraps, and `rmcp` only
    // needs something that implements `Future`. Written as `ready` rather than
    // `async fn` so a reader is not told there is concurrency here that there is
    // not (`clippy::unused_async_trait_impl`, new in Rust 1.98).
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send {
        let mut result = ListToolsResult::default();
        result.tools = wire_tools();
        std::future::ready(Ok(result))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
        // `rmcp` 3.x widened the reply to an enum, so a handler can also say
        // "I need input first" or "I made you a task" (`A1-331`). This one
        // never does either: every tool here answers now or refuses now, and
        // `CallToolResponse::Complete` is what that is. Named rather than
        // `.into()`d at each site so the choice is visible to whoever adds a
        // tool that does want to ask a question back.
    ) -> impl std::future::Future<Output = Result<CallToolResponse, McpError>> + Send {
        let args = request
            .arguments
            .map_or_else(|| serde_json::json!({}), serde_json::Value::Object);

        // A refusal is a *result*, not a protocol error: the caller asked a
        // well-formed question and the answer is no, with the reason. Reporting
        // it as a transport failure would leave an agent unable to tell "I may
        // not do that" from "the server broke".
        let out = match self.dispatch(&request.name, &args) {
            Ok(value) => Ok(CallToolResponse::Complete(CallToolResult::success(vec![
                ContentBlock::text(
                    serde_json::to_string_pretty(&value).unwrap_or_else(|e| e.to_string()),
                ),
            ]))),
            Err(message) => Ok(CallToolResponse::Complete(CallToolResult::error(vec![
                ContentBlock::text(message),
            ]))),
        };
        std::future::ready(out)
    }
}

/// Serve over stdio until the client disconnects.
pub async fn serve_unconfigured(said: String, identity: serde_json::Value) -> std::io::Result<()> {
    let service = rmcp::serve_server(
        AplanServer::unconfigured(said, identity),
        rmcp::transport::io::stdio(),
    )
    .await
    .map_err(std::io::Error::other)?;
    service.waiting().await.map_err(std::io::Error::other)?;
    Ok(())
}

/// Serve `root` under `policy`, reporting `identity` as this build.
///
/// # Errors
///
/// When the transport cannot be established or the service ends abnormally.
pub async fn serve(
    root: PathBuf,
    policy: crate::Policy,
    identity: serde_json::Value,
    cache_mode: aplan_app::CacheMode,
    cache_writer_identity: String,
    written_by: (String, String),
) -> std::io::Result<()> {
    let service = rmcp::serve_server(
        AplanServer::new(
            root,
            policy,
            identity,
            cache_mode,
            cache_writer_identity,
            written_by,
        ),
        rmcp::transport::io::stdio(),
    )
    .await
    .map_err(std::io::Error::other)?;
    service.waiting().await.map_err(std::io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AplanServer, input_schema, refuse_missing_external_roots};

    #[test]
    fn add_wire_schema_enumerates_every_typed_argument() {
        let typed = serde_json::to_value(schemars::schema_for!(crate::tools::AddArgs))
            .expect("typed add schema");
        let typed = typed["properties"]
            .as_object()
            .expect("typed add properties")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        let wire_schema = input_schema("aplan_add");
        let wire = wire_schema["properties"]
            .as_object()
            .expect("wire add properties")
            .keys()
            .map(String::as_str)
            .filter(|field| *field != "root")
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(wire, typed);
    }

    fn task(id: &str, title: &str, effort: bool, trace: bool) -> String {
        format!(
            "---\nid: {id}\ntitle: {title}\nphase: 1\nstatus: todo\n{}paths:\n- src/**\n{}---\n\nbody\n",
            if effort { "effort_days: 1\n" } else { "" },
            if trace {
                "traces:\n- DESIGN.md#fact\n"
            } else {
                ""
            },
        )
    }

    fn repository(name: &str, plan: &str, config: &str) -> tempfile::TempDir {
        let repository = tempfile::Builder::new()
            .prefix(name)
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("repository");
        std::fs::create_dir(repository.path().join(".git")).expect("git marker");
        std::fs::create_dir_all(repository.path().join(plan)).expect("plan");
        std::fs::write(repository.path().join(".aplan.yml"), config).expect("config");
        repository
    }

    fn server_for(root: &std::path::Path) -> AplanServer {
        let policy = crate::Policy {
            repo_root: root.to_path_buf(),
            ..crate::Policy::default()
        };
        AplanServer::new(
            root.join("aplan"),
            policy,
            serde_json::json!({}),
            aplan_app::CacheMode::Off,
            "mcp-context-test".to_string(),
            ("test".to_string(), "surface".to_string()),
        )
    }

    #[test]
    fn explicit_context_refuses_every_absent_external_trace_root() {
        let temporary = tempfile::Builder::new()
            .prefix("aplan-mcp-external-roots-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary directory");
        let first = temporary.path().join("absent-one");
        let second = temporary.path().join("absent-two");
        let mut options = aplan_core::Options::default();
        options.traces_external = vec![first.clone(), second.clone()];

        let error = refuse_missing_external_roots(&options).expect_err("missing roots");

        assert!(error.contains("selected configuration declares absent `traces_external`"));
        assert!(error.contains(&first.display().to_string()), "{error}");
        assert!(error.contains(&second.display().to_string()), "{error}");
        assert!(error.contains("explicit MCP root is refused"), "{error}");
    }

    fn assert_selected_context(
        value: &serde_json::Value,
        selection: &str,
        selected_repository: &std::path::Path,
    ) {
        assert_eq!(value["context"]["selection"], selection);
        assert_eq!(value["context"]["repository_root"], ".");
        assert_eq!(value["context"]["plan_root"], "plans/tasks");
        assert_eq!(value["context"]["configuration"], ".aplan.yml");
        assert_eq!(value["result"]["tasks_checked"], 2);
        assert_eq!(value["result"]["warnings"], 1);
        assert_eq!(value["result"]["denied"], true);
        assert!(
            !value.to_string().contains("APLAN008"),
            "the selected trace root must resolve: {value}"
        );
        assert!(
            !value["context"]
                .to_string()
                .contains(selected_repository.to_str().expect("utf8")),
            "bounded provenance must not repeat an absolute host path: {}",
            value["context"]
        );
    }

    #[test]
    fn long_lived_server_reuses_the_shared_structural_report() {
        let temporary = tempfile::Builder::new()
            .prefix("aplan-mcp-store-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("temporary directory");
        let root = temporary.path().join("aplan");
        std::fs::create_dir(&root).expect("plan root");
        std::fs::write(
            root.join("A-1.md"),
            "---\nid: A-1\ntitle: one\nphase: 1\nstatus: todo\neffort_days: 1\n\
             traces:\n- DESIGN.md#fact\n---\n",
        )
        .expect("task");
        std::fs::write(temporary.path().join("DESIGN.md"), "# Fact\n\nbody\n")
            .expect("trace document");
        let mut policy = crate::Policy::default();
        policy.options.traces_root = Some(temporary.path().to_path_buf());
        policy.repo_root = temporary.path().to_path_buf();
        let mut server = AplanServer::new(
            root,
            policy,
            serde_json::json!({}),
            aplan_app::CacheMode::Auto,
            "mcp-cache-test".to_string(),
            ("test".to_string(), "test".to_string()),
        );
        server.cache_root = Some(temporary.path().join("cache"));

        let before = aplan_core::trace_document_work();
        server
            .dispatch("aplan_check", &serde_json::json!({}))
            .expect("cold check");
        let after_cold = aplan_core::trace_document_work();
        server
            .dispatch("aplan_check", &serde_json::json!({}))
            .expect("warm check");
        let after_warm = aplan_core::trace_document_work();
        assert_eq!(after_cold.markdown_parses - before.markdown_parses, 1);
        assert_eq!(after_warm.markdown_parses - after_cold.markdown_parses, 0);
        assert_eq!(after_warm.reads - after_cold.reads, 0);
    }

    #[test]
    fn explicit_root_selects_plan_policy_and_mutation_as_one_context() {
        // Proof for `mcp.explicit_root_resolves_one_repository_context`.
        // Proof for `mcp.context_paths_are_portable`.
        let launch = repository("aplan-mcp-launch-", "aplan", "root: aplan\n");
        std::fs::write(
            launch.path().join("aplan/A-1.md"),
            task("A-1", "launch", true, false),
        )
        .expect("launch task");
        let selected = repository(
            "aplan-mcp-selected-",
            "plans/tasks",
            "root: plans/tasks\ndeny_warnings: true\ntraces_root: docs\nid_strategy: ulid\n\
             test:\n  list: [runner, --list]\n  run: [runner, '{test}']\n",
        );
        std::fs::create_dir(selected.path().join("docs")).expect("docs");
        std::fs::write(
            selected.path().join("docs/DESIGN.md"),
            "# Fact\n\nselected trace\n",
        )
        .expect("trace");
        let selected_plan = selected.path().join("plans/tasks");
        std::fs::write(
            selected_plan.join("B-1.md"),
            task("B-1", "selected", true, true),
        )
        .expect("selected task");
        std::fs::write(
            selected_plan.join("B-2.md"),
            task("B-2", "warning", false, false),
        )
        .expect("warning task");

        let server = server_for(launch.path());
        let repository_root = selected.path().display().to_string();
        let plan_root = selected_plan.display().to_string();
        let from_repository = server
            .dispatch("aplan_check", &serde_json::json!({"root": repository_root}))
            .expect("repository-form check");
        let from_plan = server
            .dispatch("aplan_check", &serde_json::json!({"root": plan_root}))
            .expect("plan-form check");
        let traversal = selected.path().join("plans/..");
        let from_traversal = server
            .dispatch("aplan_check", &serde_json::json!({"root": traversal}))
            .expect("canonical traversal check");
        for (value, selection) in [(&from_repository, "repository"), (&from_plan, "plan")] {
            assert_selected_context(value, selection, selected.path());
        }
        assert_eq!(
            from_traversal["context"]["selection"], "repository",
            "a lexical traversal must select its canonical target rather than mix frames"
        );
        assert_eq!(
            from_traversal["result"]["tasks_checked"], 2,
            "the canonical target is the selected repository"
        );

        let context = server
            .call_context(Some(selected.path().to_str().expect("utf8")))
            .expect("resolved context");
        assert_eq!(context.policy.repo_root, selected.path());
        assert_eq!(
            context.policy.id_strategy,
            aplan_core::IdStrategy::Ulid,
            "the selected repository's ID policy must travel with its plan"
        );
        assert!(context.policy.verify.test_runner.is_some());
        assert_eq!(
            context.policy.options.traces_root,
            Some(selected.path().join("docs"))
        );

        server
            .dispatch(
                "aplan_start",
                &serde_json::json!({"root": selected.path(), "id": "B-1"}),
            )
            .expect("guarded write in selected repository");
        assert!(
            std::fs::read_to_string(selected_plan.join("B-1.md"))
                .expect("selected task after write")
                .contains("status: in_progress")
        );
        assert!(
            std::fs::read_to_string(launch.path().join("aplan/A-1.md"))
                .expect("launch task after write")
                .contains("status: todo")
        );
        server
            .dispatch(
                "aplan_stop",
                &serde_json::json!({"root": selected_plan, "id": "B-1"}),
            )
            .expect("task-form guarded write");
    }

    #[test]
    fn explicit_root_refuses_partial_and_missing_contexts_without_fallback() {
        // Proof for `mcp.explicit_root_resolves_one_repository_context`.
        let launch = repository("aplan-mcp-launch-", "aplan", "root: aplan\n");
        std::fs::write(
            launch.path().join("aplan/A-1.md"),
            task("A-1", "launch", true, false),
        )
        .expect("launch task");
        let selected = repository("aplan-mcp-selected-", "plans/tasks", "root: plans/tasks\n");
        let plan = selected.path().join("plans/tasks");
        let nested = plan.join("nested");
        std::fs::create_dir(&nested).expect("nested");
        std::fs::write(plan.join("B-1.md"), task("B-1", "selected", true, false))
            .expect("selected task");
        let server = server_for(launch.path());

        let error = server
            .dispatch(
                "aplan_start",
                &serde_json::json!({"root": nested, "id": "A-1"}),
            )
            .expect_err("partial root");
        assert!(error.contains("not its configured plan root"), "{error}");
        assert!(
            std::fs::read_to_string(launch.path().join("aplan/A-1.md"))
                .expect("launch task")
                .contains("status: todo"),
            "a refused explicit root must not fall back to the launch repository"
        );

        let error = server
            .dispatch(
                "aplan_check",
                &serde_json::json!({"root": selected.path().join("missing")}),
            )
            .expect_err("missing root");
        assert!(error.contains("does not exist"), "{error}");
        assert!(error.contains("was not used as a fallback"), "{error}");

        let error = server
            .dispatch("aplan_start", &serde_json::json!({"root": 7, "id": "A-1"}))
            .expect_err("a typed root must not be treated as an omitted root");
        assert!(error.contains("`root` must be a string"), "{error}");
        assert!(
            std::fs::read_to_string(launch.path().join("aplan/A-1.md"))
                .expect("launch task after malformed root")
                .contains("status: todo"),
            "an invalid explicit root must not mutate the launch repository"
        );
    }
}
