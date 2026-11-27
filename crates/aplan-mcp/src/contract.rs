//! Versioned MCP contract types, generated JSON Schemas, and structured/text
//! parity helpers (`A2-118`).
//!
//! Every tool input schema and output schema is generated from concrete Rust
//! types via `schemars::schema_for!`, wrapped in an object-enveloped
//! [`ToolResultEnvelope`] so both handshake-era (`2025-06-18` / `2025-11-25`)
//! object-only structured-content clients and `2026-07-28` clients validate
//! identically.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::tools::TOOLS;

/// Explicit MCP application contract version advertised by schemas, identity,
/// facts, tool envelopes, and resources.
pub const MCP_CONTRACT_VERSION: &str = "aplan/mcp/v2";

/// Protocol revisions covered by the compatibility matrix (oldest first).
pub const SUPPORTED_PROTOCOL_REVISIONS: &[&str] = &[
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    "2025-11-25",
    "2026-07-28",
];

/// Hard upper bound on serialized bytes returned by a single MCP tool result.
pub const MAX_TOOL_RESULT_BYTES: usize = 512 * 1024;

/// Hard upper bound on serialized bytes returned by a single MCP resource read.
pub const MAX_RESOURCE_BYTES: usize = 64 * 1024;

/// Maximum accepted length of a resource URI in bytes.
pub const MAX_URI_BYTES: usize = 1024;

/// Maximum accepted length of a ready-collection continuation cursor in bytes.
pub const MAX_CURSOR_BYTES: usize = 128;

/// Maximum accepted length of a completed-history continuation cursor in bytes
/// (matching `aplan_app::completed`).
pub const MAX_COMPLETED_CURSOR_BYTES: usize = 512;

/// Maximum accepted decoded byte length of one query filter or bound value on
/// bounded MCP collection resources.
pub const MAX_FILTER_VALUE_BYTES: usize = 128;

/// Maximum repeated query values permitted per selector dimension (`cut`,
/// `phase`, `owner`, `tag`) on bounded MCP collection resources.
pub const MAX_FILTER_VALUES_PER_DIMENSION: usize = 16;

/// Default item limit for bounded collection resources when omitted.
pub const DEFAULT_PAGE_LIMIT: usize = 20;

/// Hard upper bound on `limit` for bounded collection resources.
pub const MAX_PAGE_LIMIT: usize = 50;

/// Semantic effect produced by one MCP tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    /// Observed the plan, repository, or server identity without writing files.
    ReadOnly,
    /// Wrote or updated repository plan/claim state through the guarded engine.
    Mutated,
    /// Refused before writing because arguments, policy, or evidence failed.
    Refused,
}

impl ToolEffect {
    /// Stable lowercase token used in both structured envelopes and concise text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutated => "mutated",
            Self::Refused => "refused",
        }
    }
}

/// Resolved repository/plan selection provenance when a caller supplies an
/// explicit `root` argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContextProvenance {
    /// How the explicit `root` path resolved (`repository_root` or `plan_root`).
    pub selection: String,
    /// Relative repository root marker (always `"."` to avoid leaking host paths).
    pub repository_root: String,
    /// Plan root relative to the selected repository.
    pub plan_root: String,
    /// Selected configuration file relative to the repository, if present.
    pub configuration: Option<String>,
    /// Whether resolving `root` followed a symbolic link.
    pub resolved_symlink: bool,
}

/// Cryptographic identity of the plan and configuration inputs backing a
/// bounded MCP resource response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InputIdentity {
    /// Content digest of the loaded plan snapshot.
    pub plan_sha256: String,
    /// Content digest of the active configuration, or `"none"` when unconfigured.
    pub config_sha256: String,
    /// Where the inputs were read from (`working_tree`).
    pub source: String,
    /// Trust boundary (`hermetic` or `repository`).
    pub trust: String,
}

/// Object-enveloped structured result returned in `structuredContent` by every
/// MCP tool.
///
/// Using an explicit object envelope guarantees compatibility with both
/// handshake-era object-only structured-content clients (`2025-06-18` /
/// `2025-11-25`) and `2026-07-28` clients, while exposing `denied`, `effect`,
/// and `subject` without requiring callers to parse human compatibility text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolResultEnvelope<T> {
    /// Explicit contract version (`aplan/mcp/v2`).
    pub contract_version: String,
    /// Name of the invoked MCP tool.
    pub tool: String,
    /// Primary subject of the invocation (task id, `"plan"`, or `"server"`).
    pub subject: String,
    /// Whether the plan verdict or tool gate denied the request.
    pub denied: bool,
    /// Semantic effect of the call (`read_only`, `mutated`, or `refused`).
    pub effect: ToolEffect,
    /// Resolved repository context when the caller passed an explicit `root`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextProvenance>,
    /// Typed concrete tool payload when the tool completed (even when a check or
    /// verify verdict is `denied: true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Sanitized refusal reason when the tool refused (`effect == refused`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Concrete typed tool request inputs (schema-derived, deny_unknown_fields)
// ---------------------------------------------------------------------------

/// Input for `aplan_identity`: takes no arguments because identity describes
/// this server process rather than any repository.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityInput {}

/// Input for tools scoped only by an optional explicit repository/plan `root`
/// (`aplan_ready`, `aplan_check`, `aplan_verify`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RootOnlyInput {
    /// Repository root containing `.aplan.yml`, or that repository's configured
    /// plan directory. The server resolves plan, configuration, traces, evidence
    /// providers and Git as one context; an ambiguous partial directory is
    /// refused. Omit this field to use the server's launch context. An explicit
    /// selection returns `{context, result}` in legacy mode and populates
    /// `context` in structured mode so the resolved frame is auditable.
    #[serde(default)]
    pub root: Option<String>,
}

/// Input for `aplan_why`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WhyInput {
    /// Task id.
    pub id: String,
    /// Walk through cuts into their membership as well. Off by default: a cut is
    /// reported as itself, with `behind` counting what was folded, because the
    /// complete closure of a sequenced plan is mostly the roadmap you can query
    /// separately. Every answer names its `traversal`.
    #[serde(default)]
    pub all: Option<bool>,
    /// Repository root containing `.aplan.yml`, or that repository's configured
    /// plan directory. Omit to use the server's launch context.
    #[serde(default)]
    pub root: Option<String>,
}

/// Input for single-task tools (`aplan_show`, `aplan_release`, `aplan_start`,
/// `aplan_stop`, `aplan_review`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskIdInput {
    /// Task id.
    pub id: String,
    /// Repository root containing `.aplan.yml`, or that repository's configured
    /// plan directory. Omit to use the server's launch context.
    #[serde(default)]
    pub root: Option<String>,
}

/// Input for `aplan_dep`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DepInput {
    /// The task that comes second.
    pub id: String,
    /// The task it comes after, which must finish first.
    pub after: String,
    /// True to record the edge, false to remove it.
    pub add: bool,
    /// Repository root containing `.aplan.yml`, or that repository's configured
    /// plan directory. Omit to use the server's launch context.
    #[serde(default)]
    pub root: Option<String>,
}

/// Input for `aplan_close`. Deliberately accepts ONLY `id`, `evidence`, and
/// optional `root` so no caller can supply a self-assessment score.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseInput {
    /// Task id to close.
    pub id: String,
    /// Evidence as `kind:ref`, e.g. `commit:d213c9a`, `test:mod::name`,
    /// `file:path`, `url:https://...`. Required: a status is a claim, and this
    /// server will not make it without proof to resolve.
    pub evidence: Vec<String>,
    /// Repository root containing `.aplan.yml`, or that repository's configured
    /// plan directory. Omit to use the server's launch context.
    #[serde(default)]
    pub root: Option<String>,
}

/// Input for `aplan_claim`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimInput {
    /// The task to claim.
    pub id: String,
    /// The name to record. Otherwise `APLAN_OWNER`, then per-user `me`, this
    /// repository's `git config user.name`, then the operating system's user.
    /// Never read from `.aplan.yml`.
    #[serde(default, rename = "as")]
    pub as_actor: Option<String>,
    /// Who the task is being taken from. Required when somebody holds it, and
    /// checked rather than taken on trust.
    #[serde(default)]
    pub from: Option<String>,
    /// Repository root containing `.aplan.yml`, or that repository's configured
    /// plan directory. Omit to use the server's launch context.
    #[serde(default)]
    pub root: Option<String>,
}

/// Live initial status admitted by `aplan_add`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddInitialStatus {
    Todo,
    InProgress,
    Blocked,
}

/// Version impact admitted by `aplan_add`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddVersionImpact {
    None,
    Fix,
    Feature,
    Breaking,
}

/// Input for `aplan_add`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInput {
    /// What the task is, in a line.
    pub title: String,
    /// Explicit id. An ordinary add may omit it to mint from the plan
    /// convention; a non-empty members request requires it.
    #[serde(default)]
    pub id: Option<String>,
    /// Reporting phase. An ordinary add may omit it to derive from the selected
    /// cut or plan; a non-empty members request requires it.
    #[serde(default)]
    pub phase: Option<u32>,
    /// The cut to place it in: its milestone id (`M-v0.8`) or the label it
    /// reports itself under (`v0.8`). A plan with `require_cut_membership`
    /// rejects a task in no cut.
    #[serde(default)]
    pub cut: Option<String>,
    /// Estimate in days.
    #[serde(default)]
    pub effort_days: Option<f64>,
    /// Globs this task owns. Without them its future evidence cannot be bound to
    /// it (`APLAN023`).
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    /// Document sections this task answers to, as `path/to/doc.md#anchor`. An
    /// anchor that does not resolve is refused (`APLAN008`).
    #[serde(default)]
    pub traces: Option<Vec<String>>,
    /// Subject or reserved tags.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Hard predecessor ids.
    #[serde(default)]
    pub blocked_by: Option<Vec<String>>,
    /// Existing live task ids moved from exactly one source cut each into the
    /// new milestone this add creates. A non-empty list requires `id`, `phase`,
    /// and a `tags` entry equal to `milestone`, forbids `cut`, and reserves
    /// `blocked_by` for milestone predecessors.
    #[serde(default)]
    pub members: Option<Vec<String>>,
    /// Durable owner name.
    #[serde(default)]
    pub owner: Option<String>,
    /// Initial live status. Terminal history uses the CLI evidence-resolving
    /// manifest ledger.
    #[serde(default)]
    pub status: Option<AddInitialStatus>,
    /// Advisory predecessor ids.
    #[serde(default)]
    pub prefer_after: Option<Vec<String>>,
    /// Decision-record task ids.
    #[serde(default)]
    pub ratified_by: Option<Vec<String>>,
    /// Release or container member ids.
    #[serde(default)]
    pub contains: Option<Vec<String>>,
    /// RFC 3339 instant for an initially blocked task.
    #[serde(default)]
    pub revisit_at: Option<String>,
    /// Declared version impact.
    #[serde(default)]
    pub version_impact: Option<AddVersionImpact>,
    /// Why the declared version impact applies.
    #[serde(default)]
    pub version_impact_reason: Option<String>,
    /// Repository root containing `.aplan.yml`, or that repository's configured
    /// plan directory. Omit to use the server's launch context.
    #[serde(default)]
    pub root: Option<String>,
}

// ---------------------------------------------------------------------------
// Concrete typed tool output payloads (schema-derived for outputSchema)
// ---------------------------------------------------------------------------

/// Structured payload for `aplan_identity`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IdentityPayload {
    /// Identity document schema identifier (`aplan/tool-identity/v1`).
    pub schema: String,
    /// Package version of this server executable.
    pub version: String,
    /// SHA-256 digest of the build's `facts` document.
    pub facts_sha256: String,
    /// Explicit MCP contract version advertised by this server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_contract_version: Option<String>,
    /// Source commit revision, if embedded at build time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// SHA-256 digest of the running binary image, if readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_sha256: Option<String>,
}

/// Structured payload for `aplan_ready`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadyPayload {
    /// Ready task rows in deterministic rank order.
    pub ready: Vec<Map<String, Value>>,
    /// Claim coordination availability and durability summary.
    pub coordination: Map<String, Value>,
    /// Unix epoch seconds when claims were observed.
    pub claim_observed_at_epoch_seconds: i64,
    /// Total number of tasks currently ready.
    #[serde(default)]
    pub count: usize,
    /// Additional tool fields preserved verbatim from the application projection.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_why`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WhyPayload {
    /// Direct or transitive blockers.
    pub blocked_by: Vec<Map<String, Value>>,
    /// Traversal mode (`cut_bounded` or `transitive`).
    pub traversal: String,
    /// Additional projection fields preserved verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_check` and `aplan_verify`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidationPayload {
    /// Whether the plan fails the configured verdict policy.
    pub denied: bool,
    /// Number of error diagnostics.
    pub errors: usize,
    /// Number of warning diagnostics.
    pub warnings: usize,
    /// Complete diagnostic entries.
    pub diagnostics: Vec<Map<String, Value>>,
    /// Additional verification/report fields preserved verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_add`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddPayload {
    /// Created task or milestone id.
    pub id: String,
    /// Created file path.
    pub file: String,
    /// Non-blocking warning notes.
    #[serde(default)]
    pub notes: Vec<String>,
    /// Additional authoring fields (e.g. moved members).
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_show`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShowPayload {
    /// Show schema identifier (`aplan/show/v3`).
    pub schema: String,
    /// Task id.
    pub id: String,
    /// Task title.
    pub title: String,
    /// Current task status.
    pub status: String,
    /// Complete typed show projection fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_dep`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DepPayload {
    /// Dependent task id.
    pub id: String,
    /// Predecessor task id.
    pub after: String,
    /// Whether the dependency edge changed.
    pub changed: bool,
    /// Modified file path when `changed` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Non-blocking notes when `changed` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<String>>,
}

/// Structured payload for `aplan_close`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClosePayload {
    /// Closed task id.
    pub closed: String,
    /// Canonical RFC 3339 UTC timestamp recorded at close.
    pub closed_at: String,
    /// Updated task file path.
    pub file: String,
    /// Resolved evidence citations recorded on the task.
    pub evidence: Vec<String>,
    /// Version of aplan that performed the close.
    pub aplan_version: String,
    /// Additional reconciliation and release-version fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_claim`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimPayload {
    /// Claimed task id.
    pub id: String,
    /// Holder name now holding the claim.
    pub held_by: String,
    /// Complete claim record.
    pub claim: Map<String, Value>,
    /// Coordination scope.
    pub coordination: Map<String, Value>,
    /// Additional claim fields (`displaced`, `taken_from`, `stable_owner`, `warnings`).
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_release`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReleasePayload {
    /// Task id.
    pub id: String,
    /// Whether an active claim was released.
    pub changed: bool,
    /// Coordination scope.
    pub coordination: Map<String, Value>,
    /// Additional release fields (`was_held_by`, `released`, `held_by`).
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_start` and `aplan_stop`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkingPayload {
    /// Task id.
    pub id: String,
    /// Resulting status (`in_progress` or `todo`).
    pub status: String,
    /// Whether the task status changed.
    pub changed: bool,
    /// Additional claim/file/notes fields when `changed` is true.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured payload for `aplan_review`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewPayload {
    /// Reviewed task id.
    pub id: String,
    /// Whether trace fingerprints changed.
    pub changed: bool,
    /// Number of stamped traces.
    pub stamped: usize,
    /// Map of trace reference to SHA-256 fingerprint.
    pub trace_fingerprints: BTreeMap<String, String>,
    /// Task file path.
    pub file: String,
    /// Non-blocking notes.
    #[serde(default)]
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Bounded MCP resource & collection-page schemas
// ---------------------------------------------------------------------------

/// Typed bounded collection page returned by MCP collection resource templates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CollectionPage<T> {
    /// Explicit MCP contract version (`aplan/mcp/v2`).
    pub contract_version: String,
    /// Canonical URI of this page resource.
    pub resource: String,
    /// Cryptographic input identity bound to every continuation cursor.
    pub input_identity: InputIdentity,
    /// Deterministic ordering rule applied before slicing.
    pub order: String,
    /// Applied maximum item count for this page (`1..=50`).
    pub limit: usize,
    /// Number of items included in `items`.
    pub returned: usize,
    /// Total matching items across the entire snapshot before slicing.
    pub total_matching: usize,
    /// `true` when this page reaches the end of the matching collection.
    pub complete: bool,
    /// `true` when `items` was shortened below `limit` to stay within
    /// [`MAX_RESOURCE_BYTES`].
    pub truncated_by_size_cap: bool,
    /// Opaque continuation token for the next page when `!complete`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Canonical RFC 6570 URI for the next page when `!complete`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_uri: Option<String>,
    /// Bounded page items.
    pub items: Vec<T>,
}

/// One compact task entry inside `aplan://collections/ready{?limit,cursor}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReadyCollectionItem {
    /// Task id.
    pub id: String,
    /// Task title.
    pub title: String,
    /// Status (`todo` or `in_progress`).
    pub status: String,
    /// Phase number.
    pub phase: u32,
    /// Derived ready rank (1-based).
    pub rank: usize,
    /// Effort estimate in days, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_days: Option<f64>,
    /// Owning cut label or milestone id, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<String>,
    /// Durable owner, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Active local claim holder, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_by: Option<String>,
}

/// Bounded summary resource returned by `aplan://plan/summary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanSummaryResource {
    /// Explicit MCP contract version (`aplan/mcp/v2`).
    pub contract_version: String,
    /// Canonical URI (`aplan://plan/summary`).
    pub resource: String,
    /// Cryptographic identity of the plan and configuration snapshot.
    pub input_identity: InputIdentity,
    /// Always `true` for the bounded aggregate summary.
    pub complete: bool,
    /// Total loaded tasks (including milestones).
    pub total_tasks: usize,
    /// Number of currently ready tasks.
    pub ready_count: usize,
    /// Number of milestone cuts in the plan.
    pub cut_count: usize,
    /// Task counts grouped by status name.
    pub status_counts: BTreeMap<String, usize>,
    /// Structural error diagnostic count under the configured policy.
    pub errors: usize,
    /// Structural warning diagnostic count under the configured policy.
    pub warnings: usize,
    /// Whether the structural check is denied under the configured verdict.
    pub denied: bool,
}

/// Focused single-task resource returned by `aplan://tasks/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskDetailResource {
    /// Explicit MCP contract version (`aplan/mcp/v2`).
    pub contract_version: String,
    /// Canonical URI (`aplan://tasks/{id}`).
    pub resource: String,
    /// Cryptographic identity of the plan and configuration snapshot.
    pub input_identity: InputIdentity,
    /// Whether the task projection is complete within size bounds.
    pub complete: bool,
    /// Whether the task projection was truncated to satisfy size bounds.
    pub truncated: bool,
    /// Complete structured `Show` projection (without prose body).
    pub task: ShowPayload,
}

/// Bounded completed-task history resource returned by
/// `aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}`
/// (`A2-119`).
///
/// Embeds the shared [`aplan_core::CompletedResponse`] directly without a
/// transport-specific completion model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompletedCollectionResource {
    /// Explicit MCP contract version (`aplan/mcp/v2`).
    pub contract_version: String,
    /// Canonical URI of this completed-history page resource.
    pub resource: String,
    /// Cryptographic identity of the plan and configuration snapshot.
    pub input_identity: InputIdentity,
    /// Deterministic ordering rule (`closed_at_desc` or `closed_at_asc`).
    pub order: String,
    /// `true` when this page reaches the end of all matching completed tasks.
    pub complete: bool,
    /// `true` when the page limit was reduced below the requested limit to stay
    /// within [`MAX_RESOURCE_BYTES`].
    pub truncated_by_size_cap: bool,
    /// Canonical RFC 6570 URI for the next page when `!complete`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_uri: Option<String>,
    /// Exact shared application response from `aplan_app::completed_tasks`.
    pub completed: aplan_core::CompletedResponse,
}

// ---------------------------------------------------------------------------
// Schema generation (input schemas, output schemas, and derived contract doc)
// ---------------------------------------------------------------------------

fn generate_object_schema<T: JsonSchema>() -> Value {
    let mut value = serde_json::to_value(schemars::schema_for!(T))
        .expect("schemars output is always valid JSON");
    if let Some(obj) = value.as_object_mut() {
        // Ensure every top-level tool schema is explicitly `"type": "object"`
        // so strict MCP clients and older validators accept it cleanly.
        obj.entry("type".to_string())
            .or_insert_with(|| Value::String("object".to_string()));
        obj.remove("$schema");
    }
    value
}

/// Generate the JSON Schema for a tool's input arguments from its concrete
/// typed request struct.
///
/// # Panics
///
/// Panics if `name` is not a member of [`TOOLS`].
#[must_use]
pub fn input_schema_for_tool(name: &str) -> Value {
    match name {
        "aplan_identity" => generate_object_schema::<IdentityInput>(),
        "aplan_ready" | "aplan_check" | "aplan_verify" => generate_object_schema::<RootOnlyInput>(),
        "aplan_why" => generate_object_schema::<WhyInput>(),
        "aplan_add" => {
            let mut schema = generate_object_schema::<AddInput>();
            if let Some(obj) = schema.as_object_mut() {
                // Preserve the conditional milestone-members creation rule directly
                // in the generated input schema so agents see it before calling.
                obj.insert(
                    "allOf".to_string(),
                    serde_json::json!([{
                        "if": {
                            "required": ["members"],
                            "properties": { "members": { "minItems": 1 } }
                        },
                        "then": {
                            "required": ["id", "phase", "tags"],
                            "properties": {
                                "tags": { "contains": { "const": "milestone" } }
                            },
                            "not": { "required": ["cut"] }
                        }
                    }]),
                );
                if let Some(members) = obj
                    .get_mut("properties")
                    .and_then(Value::as_object_mut)
                    .and_then(|p| p.get_mut("members"))
                    .and_then(Value::as_object_mut)
                {
                    members.insert("uniqueItems".to_string(), Value::Bool(true));
                }
            }
            schema
        }
        "aplan_show" | "aplan_release" | "aplan_start" | "aplan_stop" | "aplan_review" => {
            generate_object_schema::<TaskIdInput>()
        }
        "aplan_dep" => generate_object_schema::<DepInput>(),
        "aplan_close" => generate_object_schema::<CloseInput>(),
        "aplan_claim" => generate_object_schema::<ClaimInput>(),
        other => panic!("unknown MCP tool `{other}` in input_schema_for_tool"),
    }
}

/// Generate the object-enveloped JSON Schema for a tool's structured output
/// (`structuredContent`) from its concrete typed output envelope struct.
///
/// # Panics
///
/// Panics if `name` is not a member of [`TOOLS`].
#[must_use]
pub fn output_schema_for_tool(name: &str) -> Value {
    match name {
        "aplan_identity" => generate_object_schema::<ToolResultEnvelope<IdentityPayload>>(),
        "aplan_ready" => generate_object_schema::<ToolResultEnvelope<ReadyPayload>>(),
        "aplan_why" => generate_object_schema::<ToolResultEnvelope<WhyPayload>>(),
        "aplan_check" | "aplan_verify" => {
            generate_object_schema::<ToolResultEnvelope<ValidationPayload>>()
        }
        "aplan_add" => generate_object_schema::<ToolResultEnvelope<AddPayload>>(),
        "aplan_show" => generate_object_schema::<ToolResultEnvelope<ShowPayload>>(),
        "aplan_dep" => generate_object_schema::<ToolResultEnvelope<DepPayload>>(),
        "aplan_close" => generate_object_schema::<ToolResultEnvelope<ClosePayload>>(),
        "aplan_claim" => generate_object_schema::<ToolResultEnvelope<ClaimPayload>>(),
        "aplan_release" => generate_object_schema::<ToolResultEnvelope<ReleasePayload>>(),
        "aplan_start" | "aplan_stop" => {
            generate_object_schema::<ToolResultEnvelope<WorkingPayload>>()
        }
        "aplan_review" => generate_object_schema::<ToolResultEnvelope<ReviewPayload>>(),
        other => panic!("unknown MCP tool `{other}` in output_schema_for_tool"),
    }
}

/// Validate raw JSON arguments against a tool's concrete typed request struct
/// before dispatch (`#[serde(deny_unknown_fields)]`).
///
/// # Errors
///
/// Returns a concise refusal message when an unknown field, missing required
/// field, or wrong JSON type is supplied.
pub fn validate_tool_arguments(name: &str, args: &Value) -> Result<(), String> {
    fn check<T: for<'de> Deserialize<'de>>(name: &str, args: &Value) -> Result<(), String> {
        serde_json::from_value::<T>(args.clone())
            .map(|_| ())
            .map_err(|e| format!("bad arguments for `{name}`: {e}"))
    }
    match name {
        "aplan_identity" => check::<IdentityInput>(name, args),
        "aplan_ready" | "aplan_check" | "aplan_verify" => check::<RootOnlyInput>(name, args),
        "aplan_why" => check::<WhyInput>(name, args),
        "aplan_add" => check::<AddInput>(name, args),
        "aplan_show" | "aplan_release" | "aplan_start" | "aplan_stop" | "aplan_review" => {
            check::<TaskIdInput>(name, args)
        }
        "aplan_dep" => check::<DepInput>(name, args),
        "aplan_close" => check::<CloseInput>(name, args),
        "aplan_claim" => check::<ClaimInput>(name, args),
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// Build the concise, terminal-safe compatibility text view for one tool
/// outcome.
///
/// Always carries the exact `tool`, `subject`, `verdict` (`allowed` / `denied`),
/// and `effect` (`read_only` / `mutated` / `refused`) so structured-vs-text
/// parity audits can assert exact agreement without reparsing JSON.
#[must_use]
pub fn render_concise_tool_text(
    tool: &str,
    subject: &str,
    denied: bool,
    effect: ToolEffect,
    detail: &str,
) -> String {
    let verdict = if denied { "denied" } else { "allowed" };
    let clean_subject = aplan_output::sanitize::visible(subject);
    let clean_detail = aplan_output::sanitize::visible(detail);
    if clean_detail.is_empty() {
        format!(
            "{tool}: subject={clean_subject} verdict={verdict} effect={}",
            effect.as_str()
        )
    } else {
        format!(
            "{tool}: subject={clean_subject} verdict={verdict} effect={} -- {clean_detail}",
            effect.as_str()
        )
    }
}

/// Derived MCP contract schema document checked as a committed artifact
/// (`schemas/v1/mcp-contract.json` via `cargo xtask gen-schema`).
#[must_use]
pub fn derived_schema() -> Value {
    let mut tools = Map::new();
    for spec in TOOLS {
        tools.insert(
            spec.name.to_string(),
            serde_json::json!({
                "name": spec.name,
                "description": spec.description,
                "inputSchema": input_schema_for_tool(spec.name),
                "outputSchema": output_schema_for_tool(spec.name),
            }),
        );
    }
    let resources = serde_json::json!({
        "aplan://plan/summary": {
            "name": "plan_summary",
            "mimeType": "application/json",
            "schema": generate_object_schema::<PlanSummaryResource>(),
        },
        "aplan://tasks/{id}": {
            "name": "task_detail",
            "mimeType": "application/json",
            "schema": generate_object_schema::<TaskDetailResource>(),
        },
        "aplan://collections/ready{?limit,cursor}": {
            "name": "ready_tasks_page",
            "mimeType": "application/json",
            "schema": generate_object_schema::<CollectionPage<ReadyCollectionItem>>(),
        },
        "aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}": {
            "name": "completed_tasks_page",
            "mimeType": "application/json",
            "schema": generate_object_schema::<CompletedCollectionResource>(),
        },
    });
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "contract_version": MCP_CONTRACT_VERSION,
        "supported_protocol_revisions": SUPPORTED_PROTOCOL_REVISIONS,
        "limits": {
            "max_tool_result_bytes": MAX_TOOL_RESULT_BYTES,
            "max_resource_bytes": MAX_RESOURCE_BYTES,
            "max_uri_bytes": MAX_URI_BYTES,
            "max_cursor_bytes": MAX_CURSOR_BYTES,
            "max_completed_cursor_bytes": MAX_COMPLETED_CURSOR_BYTES,
            "max_filter_value_bytes": MAX_FILTER_VALUE_BYTES,
            "max_filter_values_per_dimension": MAX_FILTER_VALUES_PER_DIMENSION,
            "default_page_limit": DEFAULT_PAGE_LIMIT,
            "max_page_limit": MAX_PAGE_LIMIT,
        },
        "cache_policy": {
            "ttl_ms": 0,
            "cache_scope": "private",
        },
        "tools": tools,
        "resources": resources,
    })
}

/// Exhaustive read-surface output-disposition inventory derived directly from
/// [`TOOLS`], [`crate::resources::RESOURCES`], and
/// [`crate::resources::RESOURCE_TEMPLATES`] (`A2-136`).
///
/// Every MCP tool and resource must be classified here: adding a new tool or
/// resource without declaring its output-volume disposition fails tests so an
/// unbounded response cannot silently slip onto the protocol surface.
#[must_use]
pub fn mcp_read_surface_inventory() -> Vec<aplan_output::stream::ReadSurfaceEntry> {
    use aplan_output::stream::{
        OutputDisposition, ReadChannel, ReadLatencyClass, ReadSurfaceEntry,
    };

    let mut entries = Vec::new();

    for spec in TOOLS {
        let (potentially_unbounded, latency_class) = match spec.name {
            // Mutating tools are excluded from the read-surface inventory.
            "aplan_add" | "aplan_dep" | "aplan_close" | "aplan_claim" | "aplan_release"
            | "aplan_start" | "aplan_stop" | "aplan_review" => continue,
            // Read tools classified by latency and work contract.
            "aplan_identity" | "aplan_why" => (false, ReadLatencyClass::LocalBoundedRead),
            "aplan_ready" => (true, ReadLatencyClass::OutputDominatedRead),
            "aplan_show" => (false, ReadLatencyClass::EvidenceBackedInspection),
            "aplan_check" => (true, ReadLatencyClass::FullStructuralRead),
            "aplan_verify" => (true, ReadLatencyClass::ExternalRefresh),
            other => panic!(
                "MCP tool `{other}` is missing an explicit output disposition classification"
            ),
        };
        entries.push(ReadSurfaceEntry {
            surface: format!("mcp:tool:{}", spec.name),
            channel: ReadChannel::McpTool,
            target: spec.name.to_string(),
            format: "json-rpc/structured".to_string(),
            potentially_unbounded,
            latency_class,
            disposition: OutputDisposition::ExplicitlyCapped,
            atomic_error_boundary: "complete JSON-RPC response checked against MAX_TOOL_RESULT_BYTES (512 KiB) before framing; oversized results become a bounded structured refusal",
            rationale: "Single JSON-RPC tool frames cannot stream incrementally; hard 512 KiB byte cap keeps memory bounded and directs callers to paginated aplan:// resources.",
        });
    }

    for spec in crate::resources::RESOURCES {
        let (latency_class, disposition) = match spec.uri {
            "aplan://plan/summary" => (
                ReadLatencyClass::LocalBoundedRead,
                OutputDisposition::ExplicitlyCapped,
            ),
            other => {
                panic!("MCP static resource `{other}` is missing an explicit output disposition")
            }
        };
        entries.push(ReadSurfaceEntry {
            surface: format!("mcp:resource:{}", spec.uri),
            channel: ReadChannel::McpResource,
            target: spec.uri.to_string(),
            format: spec.mime_type.to_string(),
            potentially_unbounded: false,
            latency_class,
            disposition,
            atomic_error_boundary: "bounded aggregate JSON validated within MAX_RESOURCE_BYTES (64 KiB) before framing",
            rationale: "Fixed-size aggregate plan summary bounded well below MAX_RESOURCE_BYTES (64 KiB).",
        });
    }

    for spec in crate::resources::RESOURCE_TEMPLATES {
        let (potentially_unbounded, latency_class, disposition, boundary, rationale) = match spec
            .uri_template
        {
            "aplan://tasks/{id}" => (
                false,
                ReadLatencyClass::LocalBoundedRead,
                OutputDisposition::ExplicitlyCapped,
                "single-task JSON verified <= MAX_RESOURCE_BYTES (64 KiB) before framing; oversized task refuses cleanly",
                "Focused single-task projection without body, strictly capped at 64 KiB.",
            ),
            "aplan://collections/ready{?limit,cursor}" => (
                true,
                ReadLatencyClass::OutputDominatedRead,
                OutputDisposition::Paginated,
                "page sliced to limit <= 50 and dynamically shrunk to <= MAX_RESOURCE_BYTES (64 KiB) with snapshot-bound cursor before framing",
                "Potentially unbounded ready collection served as bounded deterministic pages with input-identity-bound cursors.",
            ),
            "aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}" => {
                (
                    true,
                    ReadLatencyClass::RepositoryRead,
                    OutputDisposition::Paginated,
                    "page sliced to limit <= 50 and dynamically shrunk to <= MAX_RESOURCE_BYTES (64 KiB) with snapshot-bound cursor before framing",
                    "Chronological completion history (with optional bounded Git history inference) served as bounded deterministic pages with input-identity-bound cursors.",
                )
            }
            other => {
                panic!("MCP resource template `{other}` is missing an explicit output disposition")
            }
        };
        entries.push(ReadSurfaceEntry {
            surface: format!("mcp:resource:{}", spec.uri_template),
            channel: ReadChannel::McpResource,
            target: spec.uri_template.to_string(),
            format: spec.mime_type.to_string(),
            potentially_unbounded,
            latency_class,
            disposition,
            atomic_error_boundary: boundary,
            rationale,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_schema_enumerates_all_tools_resources_limits_and_concise_text() {
        let schema = derived_schema();
        assert_eq!(schema["contract_version"], MCP_CONTRACT_VERSION);
        let tools = schema["tools"].as_object().expect("tools object");
        assert_eq!(tools.len(), TOOLS.len());
        for spec in TOOLS {
            assert!(tools.contains_key(spec.name), "missing tool {}", spec.name);
        }
        let resources = schema["resources"].as_object().expect("resources object");
        assert_eq!(resources.len(), 4);

        let inventory = mcp_read_surface_inventory();
        assert_eq!(
            inventory.len(),
            10,
            "6 read tools + 1 static resource + 3 resource templates"
        );
        assert!(
            inventory
                .iter()
                .any(|e| e.target == "aplan://collections/ready{?limit,cursor}"
                    && e.disposition == aplan_output::stream::OutputDisposition::Paginated)
        );
        assert!(inventory.iter().any(|e| e.target == "aplan_ready"
            && e.disposition == aplan_output::stream::OutputDisposition::ExplicitlyCapped));

        let err = validate_tool_arguments("aplan_unknown", &serde_json::json!({}))
            .expect_err("unknown tool must fail validation");
        assert!(err.contains("unknown tool"), "{err}");

        let empty_detail =
            render_concise_tool_text("aplan_ready", "plan", false, ToolEffect::ReadOnly, "");
        assert_eq!(
            empty_detail,
            "aplan_ready: subject=plan verdict=allowed effect=read_only"
        );
        let with_detail =
            render_concise_tool_text("aplan_close", "T-1", true, ToolEffect::Refused, "blocked");
        assert!(with_detail.ends_with("-- blocked"), "{with_detail}");
    }

    #[test]
    #[should_panic(expected = "unknown MCP tool")]
    fn input_schema_for_unknown_tool_panics() {
        let _ = input_schema_for_tool("aplan_does_not_exist");
    }

    #[test]
    #[should_panic(expected = "unknown MCP tool")]
    fn output_schema_for_unknown_tool_panics() {
        let _ = output_schema_for_tool("aplan_does_not_exist");
    }
}
