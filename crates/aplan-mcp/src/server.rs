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
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, InitializeResult, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, MetaObject, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, ResourceContents,
    ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use sha2::{Digest as _, Sha256};

use crate::contract::{
    self, ContextProvenance, InputIdentity, MAX_TOOL_RESULT_BYTES, MCP_CONTRACT_VERSION,
    ReadyCollectionItem, ToolEffect,
};
use crate::resources::{self, ParsedResourceUri, ResourceError};
use crate::tools::{self, TOOLS};

type StoreKey = (PathBuf, Option<PathBuf>);
type PlanStores = Arc<Mutex<std::collections::BTreeMap<StoreKey, aplan_app::PlanStore>>>;

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

/// Complete structured + legacy outcome of one MCP tool invocation.
#[derive(Debug, Clone)]
pub struct StructuredDispatchOutcome {
    /// Whether this outcome represents a tool-level refusal (`isError: true`).
    pub is_error: bool,
    /// Object-enveloped `structuredContent` conforming to the tool's output
    /// schema.
    pub structured: serde_json::Value,
    /// Concise, terminal-safe compatibility text naming subject, verdict, and
    /// semantic effect.
    pub concise_text: String,
    /// Legacy JSON (or refusal string) emitted on pre-`2025-06-18` text-only
    /// protocol revisions (`2024-11-05`, `2025-03-26`).
    pub legacy_text: String,
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
    /// Running build identity and advertised capability inventory used to
    /// enforce `.aplan.yml`'s pre-schema `requires` contract on explicit-root
    /// calls (`ADR-0068`, `A2-193`).
    running_build: Option<Arc<aplan_config::RunningBuild>>,
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
        running_build: Option<aplan_config::RunningBuild>,
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
            running_build: running_build.map(Arc::new),
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
            running_build: None,
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
        let resolved = aplan_app::resolve_explicit_root_for_build(
            explicit,
            self.running_build.as_deref(),
        )
        .map_err(|error| {
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
        let initial_policy = policy_for_resolved(&resolved, options.clone());
        let prefetched = agent_root
            .join("AGENTS.md")
            .is_file()
            .then(|| self.plan_snapshot(&resolved.plan_root, &initial_policy))
            .transpose()?;
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
        let policy = policy_for_resolved(&resolved, options);
        let selection_str = serde_json::to_value(resolved.selection)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{:?}", resolved.selection));
        let provenance = ContextProvenance {
            selection: selection_str,
            repository_root: ".".to_string(),
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
            if said.contains("does not satisfy `requires`") {
                return Err(format!(
                    "{said}\n\nThis server does not satisfy this repository's `requires` build \
                     contract, so it can do nothing but identify itself: call `aplan_identity` \
                     to inspect this build and switch to the required build (see \
                     `docs/upgrading.md`)."
                ));
            }
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
            "aplan_review" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                tools::review(root, &id, policy).map_err(|e| e.to_string())
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

    /// Dispatch one call and assemble both the schema-conforming
    /// object-enveloped `structuredContent` + concise text (for `2025-06-18+`)
    /// and the legacy JSON text (for `2024-11-05` / `2025-03-26`).
    pub fn dispatch_structured(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> StructuredDispatchOutcome {
        let requested_subject = requested_subject(name, args);
        if let Err(message) = contract::validate_tool_arguments(name, args) {
            return refusal_outcome(name, &requested_subject, message);
        }
        match self.dispatch(name, args) {
            Ok(wrapped) => {
                let legacy_text =
                    serde_json::to_string_pretty(&wrapped).unwrap_or_else(|e| e.to_string());
                if legacy_text.len() > MAX_TOOL_RESULT_BYTES {
                    return refusal_outcome(
                        name,
                        &requested_subject,
                        format!(
                            "tool `{name}` result ({} bytes) exceeds the {MAX_TOOL_RESULT_BYTES}-byte \
                             MCP tool response limit; use bounded MCP resources (`aplan://collections/ready` \
                             or `aplan://tasks/{{id}}`) instead",
                            legacy_text.len()
                        ),
                    );
                }
                let (context_val, payload) =
                    if args.get("root").is_some() && wrapped.get("context").is_some() {
                        (
                            wrapped.get("context").cloned(),
                            wrapped.get("result").cloned().unwrap_or(wrapped.clone()),
                        )
                    } else {
                        (None, wrapped)
                    };
                let (subject, denied, effect, detail) =
                    classify_tool_outcome(name, &requested_subject, &payload);
                let mut envelope = serde_json::json!({
                    "contract_version": MCP_CONTRACT_VERSION,
                    "tool": name,
                    "subject": subject,
                    "denied": denied,
                    "effect": effect,
                    "data": payload,
                });
                if let Some(ctx) = context_val
                    && let Some(obj) = envelope.as_object_mut()
                {
                    obj.insert("context".to_string(), ctx);
                }
                let concise_text =
                    contract::render_concise_tool_text(name, &subject, denied, effect, &detail);
                StructuredDispatchOutcome {
                    is_error: false,
                    structured: envelope,
                    concise_text,
                    legacy_text,
                }
            }
            Err(message) => refusal_outcome(name, &requested_subject, message),
        }
    }

    fn input_identity_for(&self, plan: &aplan_core::IdentifiedPlan, trust: &str) -> InputIdentity {
        let mut plan_hash = Sha256::new();
        plan_hash.update(b"aplan-mcp-plan-v1:");
        let mut sorted_inputs: Vec<_> = plan.inputs.iter().collect();
        sorted_inputs.sort_by(|a, b| a.path.cmp(&b.path));
        for input in sorted_inputs {
            let rel = input
                .path
                .strip_prefix(&plan.root)
                .unwrap_or(&input.path)
                .to_string_lossy()
                .replace('\\', "/");
            plan_hash.update(rel.as_bytes());
            plan_hash.update(b"\0");
            match &input.content {
                aplan_core::load::PlanFileContent::Bytes(bytes) => {
                    let sha = hex_encode(&Sha256::digest(bytes));
                    plan_hash.update(b"sha256:");
                    plan_hash.update(sha.as_bytes());
                }
                aplan_core::load::PlanFileContent::Sha256(hex) => {
                    plan_hash.update(b"sha256:");
                    plan_hash.update(hex.as_bytes());
                }
                aplan_core::load::PlanFileContent::ReadError { kind, .. } => {
                    plan_hash.update(b"err:");
                    plan_hash.update(format!("{kind:?}").as_bytes());
                }
            }
            plan_hash.update(b"\n");
        }
        let plan_sha256 = hex_encode(&plan_hash.finalize());

        let config_sha256 = match self.policy.verify.config_file.as_deref() {
            Some(path) => match std::fs::read(path) {
                Ok(bytes) => {
                    let mut h = Sha256::new();
                    h.update(&bytes);
                    hex_encode(&h.finalize())
                }
                Err(_) => "unreadable".to_string(),
            },
            None => "none".to_string(),
        };

        InputIdentity {
            plan_sha256,
            config_sha256,
            source: "working_tree".to_string(),
            trust: trust.to_string(),
        }
    }

    #[allow(clippy::too_many_lines)] // Exhaustive resource-URI dispatch keeps all confined aplan:// routes together.
    fn read_resource_payload(
        &self,
        uri: &str,
    ) -> Result<(String, serde_json::Value, InputIdentity, bool), ResourceError> {
        if let Some(said) = &self.unconfigured {
            return Err(ResourceError::InvalidParams(format!(
                "{said} -- server is unconfigured; call `aplan_identity` to inspect this build"
            )));
        }
        let parsed = resources::parse_resource_uri(uri)?;
        let root = self.root.as_path();
        let policy = &self.policy;

        match parsed {
            ParsedResourceUri::PlanSummary => {
                let (plan, cache) = self
                    .plan_snapshot_with_report_cache(root, policy, None)
                    .map_err(ResourceError::Internal)?;
                let identity = self.input_identity_for(&plan, "hermetic");
                let claims = aplan_app::claims::snapshot(root, &plan.plan)
                    .map_err(|e| ResourceError::Internal(e.to_string()))?;
                let ready = tools::ready_of(&plan.plan, &claims);
                let ready_count = ready.ready.len();
                let check_val = tools::check_of_cached(plan.clone(), policy, cache)
                    .map_err(|e| ResourceError::Internal(e.to_string()))?;
                let errors = usize::try_from(check_val["errors"].as_u64().unwrap_or(0))
                    .unwrap_or(usize::MAX);
                let warnings = usize::try_from(check_val["warnings"].as_u64().unwrap_or(0))
                    .unwrap_or(usize::MAX);
                let denied = check_val["denied"].as_bool().unwrap_or(false);
                let summary = resources::build_plan_summary(
                    &plan.plan,
                    ready_count,
                    errors,
                    warnings,
                    denied,
                    identity.clone(),
                );
                let canonical = summary.resource.clone();
                let value = serde_json::to_value(summary)
                    .map_err(|e| ResourceError::Internal(e.to_string()))?;
                Ok((canonical, value, identity, true))
            }
            ParsedResourceUri::TaskDetail { id } => {
                let plan = self
                    .plan_snapshot(root, policy)
                    .map_err(ResourceError::Internal)?;
                if !plan.plan.tasks.iter().any(|t| t.task.id == id) {
                    return Err(ResourceError::NotFound(format!(
                        "no task `{}` in this plan",
                        aplan_output::sanitize::visible(&id)
                    )));
                }
                let identity = self.input_identity_for(&plan, "repository");
                let seed = self.report_seed(root, policy);
                let cached = seed.as_ref().and_then(|(_, report)| report.clone());
                let claims = aplan_app::claims::snapshot(root, &plan.plan)
                    .map_err(|e| ResourceError::Internal(e.to_string()))?;
                let (show_val, structural) =
                    tools::show_of_cached_with_claims(&plan.plan, &id, policy, cached, &claims)
                        .map_err(|e| match e {
                            tools::ToolError::Args(msg) | tools::ToolError::Refused(msg) => {
                                ResourceError::NotFound(msg)
                            }
                            tools::ToolError::Plan(msg) => ResourceError::Internal(msg),
                        })?;
                if let Some((key, None)) = seed {
                    self.remember_report(root, policy, &key, &structural);
                }
                let detail = resources::build_task_detail(&id, show_val, identity.clone())?;
                let canonical = detail.resource.clone();
                let complete = detail.complete;
                let value = serde_json::to_value(detail)
                    .map_err(|e| ResourceError::Internal(e.to_string()))?;
                Ok((canonical, value, identity, complete))
            }
            ParsedResourceUri::ReadyCollection { limit, cursor } => {
                let plan = self
                    .plan_snapshot(root, policy)
                    .map_err(ResourceError::Internal)?;
                let identity = self.input_identity_for(&plan, "hermetic");
                let claims = aplan_app::claims::snapshot(root, &plan.plan)
                    .map_err(|e| ResourceError::Internal(e.to_string()))?;
                let ready = tools::ready_of(&plan.plan, &claims);
                let task_by_id: std::collections::BTreeMap<&str, &aplan_core::Task> = plan
                    .plan
                    .tasks
                    .iter()
                    .map(|loaded| (loaded.task.id.as_str(), &loaded.task))
                    .collect();
                let mut cut_by_task: std::collections::BTreeMap<&str, String> =
                    std::collections::BTreeMap::new();
                for loaded in &plan.plan.tasks {
                    if loaded.task.is_milestone() {
                        let label = aplan_core::roadmap::cut_label(&loaded.task).to_string();
                        for member in &loaded.task.blocked_by {
                            cut_by_task
                                .entry(member.as_str())
                                .or_insert_with(|| label.clone());
                        }
                    }
                }
                let items: Vec<ReadyCollectionItem> = ready
                    .ready
                    .iter()
                    .enumerate()
                    .map(|(idx, row)| {
                        let task = task_by_id.get(row.id.as_str()).copied();
                        let cut = cut_by_task.get(row.id.as_str()).cloned();
                        ReadyCollectionItem {
                            id: row.id.clone(),
                            title: row.title.clone(),
                            status: task
                                .map_or_else(|| "todo".to_string(), |t| t.status.to_string()),
                            phase: task.map_or(0, |t| t.phase),
                            rank: idx + 1,
                            effort_days: row.effort_days,
                            cut,
                            owner: row.owner.clone(),
                            held_by: row.held_by.clone(),
                        }
                    })
                    .collect();
                let page =
                    resources::build_ready_page(&items, limit, cursor.as_deref(), &identity)?;
                let canonical = page.resource.clone();
                let complete = page.complete;
                let value = serde_json::to_value(page)
                    .map_err(|e| ResourceError::Internal(e.to_string()))?;
                Ok((canonical, value, identity, complete))
            }
            ParsedResourceUri::CompletedCollection(query) => {
                let plan = self
                    .plan_snapshot(root, policy)
                    .map_err(ResourceError::Internal)?;
                if plan
                    .plan
                    .load_diagnostics
                    .iter()
                    .any(|d| d.severity == aplan_core::Severity::Error)
                {
                    return Err(ResourceError::Internal(
                        "the plan does not load cleanly; `aplan_check` says what failed"
                            .to_string(),
                    ));
                }
                let app_input_identity = aplan_app::plan_inputs_identity(&plan.inputs);
                let trust = if query.history == aplan_core::HistoryPolicy::Never {
                    "hermetic"
                } else {
                    "repository"
                };
                let identity = self.input_identity_for(&plan, trust);
                let (repo_root, plan_root_rel) = resolve_repo_and_plan_rel(root, &policy.repo_root);
                let page = resources::build_completed_page(
                    &plan.plan,
                    &app_input_identity,
                    &repo_root,
                    &plan_root_rel,
                    &query,
                    &identity,
                )?;
                let canonical = page.resource.clone();
                let complete = page.complete;
                let value = serde_json::to_value(page)
                    .map_err(|e| ResourceError::Internal(e.to_string()))?;
                Ok((canonical, value, identity, complete))
            }
        }
    }
}

fn resolve_repo_and_plan_rel(
    plan_root: &std::path::Path,
    fallback_repo_root: &std::path::Path,
) -> (PathBuf, PathBuf) {
    let probe = if plan_root.exists() {
        plan_root
    } else {
        fallback_repo_root
    };
    let (repo_root, resolved_rel) = match aplan_evidence::Repo::discover(probe)
        .or_else(|_| aplan_evidence::Repo::discover(fallback_repo_root))
    {
        Ok(repo) => {
            if let Some(workdir) = repo
                .effective_workdir(probe)
                .or_else(|| repo.effective_workdir(fallback_repo_root))
            {
                let rel = relative_under_root(&workdir, plan_root);
                (workdir, rel)
            } else {
                (
                    fallback_repo_root.to_path_buf(),
                    relative_under_root(fallback_repo_root, plan_root),
                )
            }
        }
        Err(_) => (
            fallback_repo_root.to_path_buf(),
            relative_under_root(fallback_repo_root, plan_root),
        ),
    };
    (repo_root, resolved_rel)
}

fn relative_under_root(base: &std::path::Path, target: &std::path::Path) -> PathBuf {
    let base_abs = std::fs::canonicalize(base)
        .or_else(|_| std::path::absolute(base))
        .unwrap_or_else(|_| base.to_path_buf());
    let target_abs = std::fs::canonicalize(target)
        .or_else(|_| std::path::absolute(target))
        .unwrap_or_else(|_| target.to_path_buf());
    match target_abs.strip_prefix(&base_abs) {
        Ok(rel) => rel.to_path_buf(),
        Err(_) => PathBuf::from("aplan"),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn requested_subject(name: &str, args: &serde_json::Value) -> String {
    if name == "aplan_identity" {
        return "server".to_string();
    }
    if let Some(id) = args.get("id").and_then(serde_json::Value::as_str)
        && !id.is_empty()
    {
        return id.to_string();
    }
    "plan".to_string()
}

fn refusal_outcome(name: &str, subject: &str, message: String) -> StructuredDispatchOutcome {
    let clean_error = aplan_output::sanitize::visible(&message).into_owned();
    let structured = serde_json::json!({
        "contract_version": MCP_CONTRACT_VERSION,
        "tool": name,
        "subject": subject,
        "denied": true,
        "effect": ToolEffect::Refused,
        "error": clean_error,
    });
    let concise_text =
        contract::render_concise_tool_text(name, subject, true, ToolEffect::Refused, &clean_error);
    StructuredDispatchOutcome {
        is_error: true,
        structured,
        concise_text,
        legacy_text: message,
    }
}

#[allow(clippy::too_many_lines)] // Exhaustive tool-outcome classification mirrors `dispatch`.
fn classify_tool_outcome(
    name: &str,
    requested_subject: &str,
    payload: &serde_json::Value,
) -> (String, bool, ToolEffect, String) {
    match name {
        "aplan_identity" => {
            let ver = payload["version"].as_str().unwrap_or("unknown");
            (
                "server".to_string(),
                false,
                ToolEffect::ReadOnly,
                format!("version {ver}"),
            )
        }
        "aplan_ready" => {
            let count = payload["ready"].as_array().map_or(0, Vec::len);
            (
                "plan".to_string(),
                false,
                ToolEffect::ReadOnly,
                format!("{count} ready task(s)"),
            )
        }
        "aplan_why" => {
            let blockers = payload["blocked_by"].as_array().map_or(0, Vec::len);
            let traversal = payload["traversal"].as_str().unwrap_or("cut_bounded");
            (
                requested_subject.to_string(),
                false,
                ToolEffect::ReadOnly,
                format!("{blockers} blocker(s) ({traversal})"),
            )
        }
        "aplan_check" | "aplan_verify" => {
            let denied = payload["denied"].as_bool().unwrap_or(false);
            let errors = payload["errors"].as_u64().unwrap_or(0);
            let warnings = payload["warnings"].as_u64().unwrap_or(0);
            (
                "plan".to_string(),
                denied,
                ToolEffect::ReadOnly,
                format!("{errors} error(s), {warnings} warning(s)"),
            )
        }
        "aplan_show" => {
            let id = payload["id"]
                .as_str()
                .unwrap_or(requested_subject)
                .to_string();
            let status = payload["status"].as_str().unwrap_or("unknown");
            (id, false, ToolEffect::ReadOnly, format!("status {status}"))
        }
        "aplan_add" => {
            let id = payload["id"]
                .as_str()
                .unwrap_or(requested_subject)
                .to_string();
            (id, false, ToolEffect::Mutated, "created".to_string())
        }
        "aplan_dep" => {
            let id = payload["id"]
                .as_str()
                .unwrap_or(requested_subject)
                .to_string();
            let changed = payload["changed"].as_bool().unwrap_or(false);
            let effect = if changed {
                ToolEffect::Mutated
            } else {
                ToolEffect::ReadOnly
            };
            (
                id,
                false,
                effect,
                if changed {
                    "dependency updated".to_string()
                } else {
                    "no dependency change".to_string()
                },
            )
        }
        "aplan_close" => {
            let id = payload["closed"]
                .as_str()
                .unwrap_or(requested_subject)
                .to_string();
            (id, false, ToolEffect::Mutated, "closed".to_string())
        }
        "aplan_claim" => {
            let id = payload["id"]
                .as_str()
                .unwrap_or(requested_subject)
                .to_string();
            let holder = payload["held_by"].as_str().unwrap_or("unknown");
            (
                id,
                false,
                ToolEffect::Mutated,
                format!("claimed by {holder}"),
            )
        }
        "aplan_release" => {
            let id = payload["id"]
                .as_str()
                .unwrap_or(requested_subject)
                .to_string();
            let changed = payload["changed"].as_bool().unwrap_or(false);
            let effect = if changed {
                ToolEffect::Mutated
            } else {
                ToolEffect::ReadOnly
            };
            (
                id,
                false,
                effect,
                if changed {
                    "claim released".to_string()
                } else {
                    "no active claim".to_string()
                },
            )
        }
        "aplan_start" | "aplan_stop" => {
            let id = payload["id"]
                .as_str()
                .unwrap_or(requested_subject)
                .to_string();
            let status = payload["status"].as_str().unwrap_or("unknown");
            let changed = payload["changed"].as_bool().unwrap_or(false);
            let effect = if changed {
                ToolEffect::Mutated
            } else {
                ToolEffect::ReadOnly
            };
            (
                id,
                false,
                effect,
                format!("status {status} (changed={changed})"),
            )
        }
        "aplan_review" => {
            let id = payload["id"]
                .as_str()
                .unwrap_or(requested_subject)
                .to_string();
            let changed = payload["changed"].as_bool().unwrap_or(false);
            let stamped = payload["stamped"].as_u64().unwrap_or(0);
            let effect = if changed {
                ToolEffect::Mutated
            } else {
                ToolEffect::ReadOnly
            };
            (id, false, effect, format!("{stamped} trace(s) stamped"))
        }
        _ => (
            requested_subject.to_string(),
            false,
            ToolEffect::ReadOnly,
            String::new(),
        ),
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

/// The JSON Schema for a tool's arguments, generated from its concrete typed
/// request struct in [`crate::contract`].
#[cfg(test)]
fn input_schema(name: &str) -> serde_json::Value {
    contract::input_schema_for_tool(name)
}

/// The tool list exactly as it goes on the wire.
///
/// Public so a test can pin it: the surface is a cost an agent pays on every
/// call, so it must not grow by accident.
pub fn wire_tools() -> Vec<Tool> {
    TOOLS
        .iter()
        .map(|spec| {
            let in_schema = contract::input_schema_for_tool(spec.name);
            let out_schema = contract::output_schema_for_tool(spec.name);
            // `Tool` is `#[non_exhaustive]`, so it is built from its default
            // rather than a struct literal: fields rmcp adds later then arrive
            // with their own defaults instead of breaking the build.
            let mut tool = Tool::default();
            tool.name = spec.name.into();
            tool.description = Some(spec.description.into());
            tool.input_schema = Arc::new(
                in_schema
                    .as_object()
                    .cloned()
                    .expect("input schemas are JSON objects"),
            );
            tool.output_schema = Some(Arc::new(
                out_schema
                    .as_object()
                    .cloned()
                    .expect("output schemas are JSON objects"),
            ));
            tool
        })
        .collect()
}

fn is_2026_07_28_or_later(context: &RequestContext<RoleServer>) -> bool {
    context
        .protocol_version()
        .is_some_and(|v| v.as_str() >= ProtocolVersion::V_2026_07_28.as_str())
}

fn supports_structured_content(context: &RequestContext<RoleServer>) -> bool {
    context
        .protocol_version()
        .is_none_or(|v| v.as_str() >= ProtocolVersion::V_2025_06_18.as_str())
}

impl ServerHandler for AplanServer {
    fn get_info(&self) -> InitializeResult {
        let mut info = InitializeResult::default();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        info.server_info = Implementation::new("aplan", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(format!(
            "Validate and advance an evidence-gated plan (contract {MCP_CONTRACT_VERSION}). \
             `aplan_close` requires evidence: a citation something else resolves, never an \
             assessment of your own work. Bounded read resources are available under `aplan://`."
        ));
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
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send {
        let mut result = ListToolsResult::default();
        result.tools = wire_tools();
        if is_2026_07_28_or_later(&context) {
            result.ttl_ms = Some(0);
            result.cache_scope = Some(CacheScope::Private);
        }
        std::future::ready(Ok(result))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send {
        let mut result = ListResourcesResult::default();
        result.resources = resources::wire_resources();
        if is_2026_07_28_or_later(&context) {
            result.ttl_ms = Some(0);
            result.cache_scope = Some(CacheScope::Private);
        }
        std::future::ready(Ok(result))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send
    {
        let mut result = ListResourceTemplatesResult::default();
        result.resource_templates = resources::wire_resource_templates();
        if is_2026_07_28_or_later(&context) {
            result.ttl_ms = Some(0);
            result.cache_scope = Some(CacheScope::Private);
        }
        std::future::ready(Ok(result))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResponse, McpError>> + Send {
        let is_current = is_2026_07_28_or_later(&context);
        let out = match self.read_resource_payload(&request.uri) {
            Ok((canonical_uri, value, identity, complete)) => {
                let text = serde_json::to_string_pretty(&value).unwrap_or_else(|e| e.to_string());
                let mut meta = MetaObject::new();
                meta.insert(
                    "contractVersion".to_string(),
                    serde_json::json!(MCP_CONTRACT_VERSION),
                );
                meta.insert(
                    "inputIdentity".to_string(),
                    serde_json::to_value(&identity).unwrap_or(serde_json::Value::Null),
                );
                meta.insert("complete".to_string(), serde_json::json!(complete));
                meta.insert("ttlMs".to_string(), serde_json::json!(0));
                meta.insert("cacheScope".to_string(), serde_json::json!("private"));
                let mut result =
                    ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                        uri: canonical_uri,
                        mime_type: Some("application/json".to_string()),
                        text,
                        meta: Some(meta),
                    }]);
                if is_current {
                    result.ttl_ms = Some(0);
                    result.cache_scope = Some(CacheScope::Private);
                }
                Ok(ReadResourceResponse::Complete(result))
            }
            Err(ResourceError::NotFound(message)) => {
                Err(McpError::resource_not_found(message, None))
            }
            Err(ResourceError::InvalidParams(message)) => {
                Err(McpError::invalid_params(message, None))
            }
            Err(ResourceError::Internal(message)) => Err(McpError::internal_error(message, None)),
        };
        std::future::ready(out)
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
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
        let outcome = self.dispatch_structured(&request.name, &args);
        let result = if supports_structured_content(&context) {
            let mut res = if outcome.is_error {
                CallToolResult::error(vec![ContentBlock::text(outcome.concise_text)])
            } else {
                CallToolResult::success(vec![ContentBlock::text(outcome.concise_text)])
            };
            res.structured_content = Some(outcome.structured);
            res
        } else if outcome.is_error {
            CallToolResult::error(vec![ContentBlock::text(outcome.legacy_text)])
        } else {
            CallToolResult::success(vec![ContentBlock::text(outcome.legacy_text)])
        };
        std::future::ready(Ok(CallToolResponse::Complete(result)))
    }
}

fn policy_for_resolved(
    resolved: &aplan_app::ResolvedRoot,
    options: aplan_core::Options,
) -> crate::Policy {
    crate::Policy {
        verify: aplan_app::configured_verification(
            &resolved.config,
            resolved.configuration_file.clone(),
            options.traces_root.clone(),
        ),
        options,
        verdict: aplan_app::configured_verdict(&resolved.config),
        repo_root: resolved.repository_root.clone(),
        id_strategy: resolved.config.id_strategy.unwrap_or_default(),
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
    running_build: aplan_config::RunningBuild,
) -> std::io::Result<()> {
    let service = rmcp::serve_server(
        AplanServer::new(
            root,
            policy,
            identity,
            cache_mode,
            cache_writer_identity,
            written_by,
            Some(running_build),
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
            ("0.8.0-dev".to_string(), "0123456789ab".to_string()),
            None,
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
            ("0.8.0-dev".to_string(), "0123456789ab".to_string()),
            None,
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
        let canonical_selected =
            std::fs::canonicalize(selected.path()).expect("canonical selected");
        assert_eq!(context.policy.repo_root, canonical_selected);
        assert_eq!(
            context.policy.id_strategy,
            aplan_core::IdStrategy::Ulid,
            "the selected repository's ID policy must travel with its plan"
        );
        assert!(context.policy.verify.test_runner.is_some());
        assert_eq!(
            context.policy.options.traces_root,
            Some(canonical_selected.join("docs"))
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

    #[test]
    fn explicit_root_enforces_requires_against_injected_running_build_before_unknown_keys() {
        let launch = repository("aplan-mcp-req-launch-", "aplan", "root: aplan\n");
        std::fs::write(
            launch.path().join("aplan/A-1.md"),
            task("A-1", "launch", true, false),
        )
        .expect("launch task");

        let selected = repository(
            "aplan-mcp-req-selected-",
            "aplan",
            "requires:\n  min_version: '0.8.0'\n  capabilities: [build.tui]\n\
             future_config_key: 123\n",
        );
        std::fs::write(
            selected.path().join("aplan/B-1.md"),
            task("B-1", "selected", true, false),
        )
        .expect("selected task");

        let mut server = server_for(launch.path());
        server.running_build = Some(std::sync::Arc::new(aplan_config::RunningBuild {
            version: "0.8.0-dev".to_string(),
            source: None,
            facts_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            advertised_capabilities: ["check".to_string()].into_iter().collect(),
        }));

        let err = server
            .dispatch("aplan_check", &serde_json::json!({"root": selected.path()}))
            .expect_err("explicit root violating `requires` must be refused");
        assert!(err.contains("does not satisfy `requires`"), "{err}");
        assert!(err.contains("0.8.0"), "{err}");
        assert!(err.contains("build.tui"), "{err}");
        assert!(err.contains("source=unknown"), "{err}");
        assert!(
            !err.contains("future_config_key"),
            "pre-schema requires must fire before unknown keys:\n{err}"
        );
    }
}
