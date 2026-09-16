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
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, Implementation, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::tools::{self, TOOLS};

/// The server: a plan root, the policy `check` runs under, and what this build
/// is.
#[derive(Debug, Clone)]
pub struct AplanServer {
    root: PathBuf,
    policy: Arc<crate::Policy>,
    /// This binary's `aplan/tool-identity/v1` document, handed over by the
    /// caller rather than computed here.
    ///
    /// The identity is a fact about the *serving executable*, and the crate that
    /// can state it is the one that owns the facts document it hashes. Computing
    /// a second one here is precisely what `A1-259` forbids: two documents that
    /// agree until they do not. Passed at construction because the server is
    /// that executable, so there is nothing to keep it current against.
    identity: Arc<serde_json::Value>,
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
    pub fn new(root: PathBuf, policy: crate::Policy, identity: serde_json::Value) -> Self {
        Self {
            root,
            policy: Arc::new(policy),
            identity: Arc::new(identity),
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
            identity: Arc::new(identity),
            unconfigured: Some(Arc::new(said)),
        }
    }

    /// Dispatch one call. Split out so it is a plain function returning a value,
    /// which is what makes the wire behaviour testable without a transport.
    fn dispatch(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        let str_arg = |key: &str| -> Option<String> {
            args.get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        let root = tools::resolve_root(str_arg("root").as_deref(), &self.root);

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

        match name {
            "aplan_ready" => tools::ready(&root)
                .map(|r| serde_json::json!(r))
                .map_err(|e| e.to_string()),
            "aplan_why" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                // The struct serialises with `blocked_by` at the top, which is
                // what this response has always been, plus the three facts the
                // CLI's human output already showed (`A1-182`).
                let all = tools::all_flag(args.get("all")).map_err(|e| e.to_string())?;
                tools::why(&root, &id, all)
                    .map(|b| serde_json::json!(b))
                    .map_err(|e| e.to_string())
            }
            "aplan_verify" => tools::verify(&root, &self.policy).map_err(|e| e.to_string()),
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
                tools::add(&root, &add, &self.policy).map_err(|e| e.to_string())
            }
            "aplan_show" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                tools::show(&root, &id, &self.policy).map_err(|e| e.to_string())
            }
            "aplan_dep" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                let after = str_arg("after").ok_or("`after` is required")?;
                let add = args
                    .get("add")
                    .and_then(serde_json::Value::as_bool)
                    .ok_or("`add` is required: true to add the edge, false to remove it")?;
                tools::dep(&root, &id, &after, add, &self.policy).map_err(|e| e.to_string())
            }
            "aplan_check" => tools::check(&root, &self.policy).map_err(|e| e.to_string()),
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
                tools::close(&root, &id, &evidence, &self.policy).map_err(|e| e.to_string())
            }
            other => Err(format!("unknown tool `{other}`")),
        }
    }
}

/// The JSON Schema for a tool's arguments.
///
/// Written out rather than derived, because the surface is eight tools and the
/// schema *is* the contract an agent reads. `close` is the one that matters: its
/// only inputs are a task id and evidence citations, and there is deliberately
/// no field through which a caller could assert completion.
fn input_schema(name: &str) -> serde_json::Value {
    let root = serde_json::json!({
        "type": "string",
        "description": "Plan root. Defaults to the server's configured root.",
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
                "owner": { "type": "string" },
                "root": root,
            },
        "required": ["title"],
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
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send {
        let args = request
            .arguments
            .map_or_else(|| serde_json::json!({}), serde_json::Value::Object);

        // A refusal is a *result*, not a protocol error: the caller asked a
        // well-formed question and the answer is no, with the reason. Reporting
        // it as a transport failure would leave an agent unable to tell "I may
        // not do that" from "the server broke".
        let out = match self.dispatch(&request.name, &args) {
            Ok(value) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&value).unwrap_or_else(|e| e.to_string()),
            )])),
            Err(message) => Ok(CallToolResult::error(vec![ContentBlock::text(message)])),
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
) -> std::io::Result<()> {
    let service = rmcp::serve_server(
        AplanServer::new(root, policy, identity),
        rmcp::transport::io::stdio(),
    )
    .await
    .map_err(std::io::Error::other)?;
    service.waiting().await.map_err(std::io::Error::other)?;
    Ok(())
}
