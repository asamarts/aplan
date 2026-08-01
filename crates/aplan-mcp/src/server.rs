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

use aplan_core::validate;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, Implementation, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::tools::{self, TOOLS};

/// The server: a plan root, and the policy `check` runs under.
#[derive(Debug, Clone)]
pub struct AplanServer {
    root: PathBuf,
    options: Arc<validate::Options>,
}

impl AplanServer {
    /// A server over `root`, checking with `options` (the repository's
    /// `.aplan.yml` policy, resolved by the caller exactly as the CLI resolves
    /// it, so MCP and the CLI judge a plan identically).
    pub fn new(root: PathBuf, options: validate::Options) -> Self {
        Self {
            root,
            options: Arc::new(options),
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

        match name {
            "aplan_ready" => tools::ready(&root)
                .map(|t| serde_json::json!({ "ready": t }))
                .map_err(|e| e.to_string()),
            "aplan_why" => {
                let id = str_arg("id").ok_or("`id` is required")?;
                tools::why(&root, &id)
                    .map(|b| serde_json::json!({ "blocked_by": b }))
                    .map_err(|e| e.to_string())
            }
            "aplan_check" => tools::check(&root, &self.options).map_err(|e| e.to_string()),
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
                tools::close(&root, &id, &evidence).map_err(|e| e.to_string())
            }
            other => Err(format!("unknown tool `{other}`")),
        }
    }
}

/// The JSON Schema for a tool's arguments.
///
/// Written out rather than derived, because the surface is four tools and the
/// schema *is* the contract an agent reads. `close` is the one that matters: its
/// only inputs are a task id and evidence citations, and there is deliberately
/// no field through which a caller could assert completion.
fn input_schema(name: &str) -> serde_json::Value {
    let root = serde_json::json!({
        "type": "string",
        "description": "Plan root. Defaults to the server's configured root.",
    });
    match name {
        "aplan_why" => serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Task id." }, "root": root },
            "required": ["id"],
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
        // `ready` and `check` take only the optional root.
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

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut result = ListToolsResult::default();
        result.tools = wire_tools();
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = request
            .arguments
            .map_or_else(|| serde_json::json!({}), serde_json::Value::Object);

        // A refusal is a *result*, not a protocol error: the caller asked a
        // well-formed question and the answer is no, with the reason. Reporting
        // it as a transport failure would leave an agent unable to tell "I may
        // not do that" from "the server broke".
        match self.dispatch(&request.name, &args) {
            Ok(value) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::to_string_pretty(&value).unwrap_or_else(|e| e.to_string()),
            )])),
            Err(message) => Ok(CallToolResult::error(vec![ContentBlock::text(message)])),
        }
    }
}

/// Serve over stdio until the client disconnects.
pub async fn serve(root: PathBuf, options: validate::Options) -> std::io::Result<()> {
    let service = rmcp::serve_server(
        AplanServer::new(root, options),
        rmcp::transport::io::stdio(),
    )
    .await
    .map_err(std::io::Error::other)?;
    service.waiting().await.map_err(std::io::Error::other)?;
    Ok(())
}
