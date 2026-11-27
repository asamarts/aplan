//! Bounded, URI-addressed MCP resources, templates, and tamper-evident
//! collection-page cursors (`A2-118`).
//!
//! Application collection pagination (`aplan://collections/ready?limit=...&cursor=...`)
//! is deliberately distinct from MCP protocol catalogue pagination
//! (`tools/list`, `resources/list`, `resources/templates/list`). Cursors are
//! size-bounded, strictly parsed, and cryptographically bound to both the
//! loaded input identity (`plan_sha256`, `config_sha256`) and canonical request
//! parameters so changed repository inputs or modified cursors are refused
//! rather than broadening a selection.

use std::collections::BTreeMap;
use std::path::Path;

use rmcp::model::{Resource, ResourceTemplate};
use sha2::{Digest as _, Sha256};

use crate::contract::{
    CollectionPage, CompletedCollectionResource, DEFAULT_PAGE_LIMIT, InputIdentity,
    MAX_COMPLETED_CURSOR_BYTES, MAX_CURSOR_BYTES, MAX_FILTER_VALUE_BYTES,
    MAX_FILTER_VALUES_PER_DIMENSION, MAX_PAGE_LIMIT, MAX_RESOURCE_BYTES, MAX_URI_BYTES,
    MCP_CONTRACT_VERSION, PlanSummaryResource, ReadyCollectionItem, ShowPayload,
    TaskDetailResource,
};

/// One static MCP resource advertised by `resources/list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceSpec {
    pub uri: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub mime_type: &'static str,
    pub origin_command: &'static str,
    pub rationale: &'static str,
}

/// One parameterized MCP resource template advertised by
/// `resources/templates/list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceTemplateSpec {
    pub uri_template: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub mime_type: &'static str,
    pub origin_command: &'static str,
    pub rationale: &'static str,
}

/// Complete closed inventory of static MCP resources.
pub const RESOURCES: &[ResourceSpec] = &[ResourceSpec {
    uri: "aplan://plan/summary",
    name: "plan_summary",
    title: "Plan summary",
    description: "Bounded aggregate counts, structural verdict summary, and cryptographic input \
                  identity for the active plan without dumping task lists or prose.",
    mime_type: "application/json",
    origin_command: "summary",
    rationale: "Read-only aggregate health belongs on a bounded resource rather than expanding \
                the action tool surface.",
}];

/// Complete closed inventory of parameterized MCP resource templates.
pub const RESOURCE_TEMPLATES: &[ResourceTemplateSpec] = &[
    ResourceTemplateSpec {
        uri_template: "aplan://tasks/{id}",
        name: "task_detail",
        title: "Task detail",
        description: "Focused structured read of one identified task (matching `aplan show` \
                      without prose body), carrying explicit input identity and completeness.",
        mime_type: "application/json",
        origin_command: "show",
        rationale: "Provides URI-addressable focused task inspection with input identity and \
                    private zero-freshness cache metadata.",
    },
    ResourceTemplateSpec {
        uri_template: "aplan://collections/ready{?limit,cursor}",
        name: "ready_tasks_page",
        title: "Ready tasks page",
        description: "Bounded, canonically ordered (`ready_rank_asc`) page of startable tasks \
                      with `limit` (default 20, max 50), 64 KiB size cap, and snapshot-bound \
                      opaque continuation cursors.",
        mime_type: "application/json",
        origin_command: "ready",
        rationale: "Bounded paged collection read keeps context predictable without adding \
                    presentation-specific rollup tools.",
    },
    ResourceTemplateSpec {
        uri_template: "aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}",
        name: "completed_tasks_page",
        title: "Completed tasks history page",
        description: "Bounded, deterministically ordered completion-history page projecting \
                      the shared `completed_tasks` engine with optional `cut`, `phase`, `owner`, \
                      `tag`, `since`, `until`, `oldest`, `history` (`auto`/`never`/`required`), \
                      `limit` (default 20, max 50), 64 KiB cap, and snapshot-bound cursors.",
        mime_type: "application/json",
        origin_command: "completed",
        rationale: "Completion history is a bounded collection read rather than an action tool, \
                    projecting the shared `completed_tasks` response directly without adding an \
                    `aplan_completed` rollup tool.",
    },
];

/// Build the wire [`Resource`] list for `resources/list`.
#[must_use]
pub fn wire_resources() -> Vec<Resource> {
    RESOURCES
        .iter()
        .map(|spec| {
            Resource::new(spec.uri, spec.name)
                .with_title(spec.title)
                .with_description(spec.description)
                .with_mime_type(spec.mime_type)
        })
        .collect()
}

/// Build the wire [`ResourceTemplate`] list for `resources/templates/list`.
#[must_use]
pub fn wire_resource_templates() -> Vec<ResourceTemplate> {
    RESOURCE_TEMPLATES
        .iter()
        .map(|spec| {
            ResourceTemplate::new(spec.uri_template, spec.name)
                .with_title(spec.title)
                .with_description(spec.description)
                .with_mime_type(spec.mime_type)
        })
        .collect()
}

/// Errors produced when parsing, validating, or reading an MCP resource URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceError {
    /// URI is syntactically valid and confined to `aplan://`, but names no
    /// known resource or task (`RESOURCE_NOT_FOUND` on handshake-era peers,
    /// `INVALID_PARAMS` on `2026-07-28`).
    NotFound(String),
    /// URI, query parameter, or continuation cursor is malformed, oversized,
    /// tampered, stale, or escapes confinement (`INVALID_PARAMS`).
    InvalidParams(String),
    /// Underlying plan or repository read failed (`INTERNAL_ERROR`).
    Internal(String),
}

/// Confined, typed query parameters for
/// `aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedUriQuery {
    pub cuts: Vec<String>,
    pub phases: Vec<u32>,
    pub owners: Vec<String>,
    pub tags: Vec<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub oldest: bool,
    pub history: aplan_core::HistoryPolicy,
    pub limit: usize,
    pub cursor: Option<String>,
}

/// Parsed, confined target of an `aplan://` resource URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedResourceUri {
    PlanSummary,
    TaskDetail {
        id: String,
    },
    ReadyCollection {
        limit: usize,
        cursor: Option<String>,
    },
    CompletedCollection(CompletedUriQuery),
}

/// Strictly parse and confine an MCP resource URI.
///
/// Rejects oversized URIs, non-`aplan://` schemes, path traversal segments,
/// backslashes, control characters, unknown query keys, and out-of-range limits.
///
/// # Errors
///
/// Returns [`ResourceError::InvalidParams`] on malformed/hostile URIs or
/// [`ResourceError::NotFound`] on unknown `aplan://` paths.
pub fn parse_resource_uri(uri: &str) -> Result<ParsedResourceUri, ResourceError> {
    if uri.is_empty() || uri.len() > MAX_URI_BYTES {
        return Err(ResourceError::InvalidParams(format!(
            "resource URI must be 1..={MAX_URI_BYTES} bytes"
        )));
    }
    if uri.chars().any(|c| c.is_control() || c == '\\') {
        return Err(ResourceError::InvalidParams(
            "resource URI contains forbidden control or backslash characters".to_string(),
        ));
    }
    let Some(rest) = uri.strip_prefix("aplan://") else {
        return Err(ResourceError::InvalidParams(
            "resource URI must use the `aplan://` scheme; host paths and network URLs are refused"
                .to_string(),
        ));
    };
    if rest.contains('#') {
        return Err(ResourceError::InvalidParams(
            "resource URI fragments are not permitted".to_string(),
        ));
    }
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    if path.split('/').any(|seg| seg == "." || seg == "..") {
        return Err(ResourceError::InvalidParams(
            "resource URI path traversal segments are refused".to_string(),
        ));
    }

    if path == "plan/summary" {
        if query.is_some() {
            return Err(ResourceError::InvalidParams(
                "`aplan://plan/summary` accepts no query parameters".to_string(),
            ));
        }
        return Ok(ParsedResourceUri::PlanSummary);
    }

    if let Some(id) = path.strip_prefix("tasks/") {
        if query.is_some() {
            return Err(ResourceError::InvalidParams(
                "`aplan://tasks/{id}` accepts no query parameters".to_string(),
            ));
        }
        validate_task_id(id)?;
        return Ok(ParsedResourceUri::TaskDetail { id: id.to_string() });
    }

    if path == "collections/ready" {
        let (limit, cursor) = parse_collection_query(query)?;
        return Ok(ParsedResourceUri::ReadyCollection { limit, cursor });
    }

    if path == "collections/completed" {
        let parsed = parse_completed_query(query)?;
        return Ok(ParsedResourceUri::CompletedCollection(parsed));
    }

    Err(ResourceError::NotFound(format!(
        "unknown aplan resource `{}`",
        aplan_output::sanitize::visible(uri)
    )))
}

fn validate_task_id(id: &str) -> Result<(), ResourceError> {
    if id.is_empty() || id.len() > 128 {
        return Err(ResourceError::InvalidParams(
            "task id in `aplan://tasks/{id}` must be 1..=128 characters".to_string(),
        ));
    }
    if id.contains('/')
        || id.contains('%')
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ResourceError::InvalidParams(format!(
            "invalid task id `{}` in resource URI",
            aplan_output::sanitize::visible(id)
        )));
    }
    Ok(())
}

fn parse_collection_query(query: Option<&str>) -> Result<(usize, Option<String>), ResourceError> {
    let mut limit: Option<usize> = None;
    let mut cursor: Option<String> = None;
    let Some(q) = query else {
        return Ok((DEFAULT_PAGE_LIMIT, None));
    };
    if q.is_empty() {
        return Err(ResourceError::InvalidParams(
            "empty query string is not canonical".to_string(),
        ));
    }
    for pair in q.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(ResourceError::InvalidParams(
                "query parameter must have `key=value` form".to_string(),
            ));
        };
        match key {
            "limit" => {
                if limit.is_some() {
                    return Err(ResourceError::InvalidParams(
                        "duplicate `limit` query parameter".to_string(),
                    ));
                }
                let parsed: usize = value.parse().map_err(|_| {
                    ResourceError::InvalidParams(
                        "`limit` must be a positive integer in 1..=50".to_string(),
                    )
                })?;
                if parsed == 0 || parsed > MAX_PAGE_LIMIT {
                    return Err(ResourceError::InvalidParams(format!(
                        "`limit` must be in 1..={MAX_PAGE_LIMIT} (got {parsed})"
                    )));
                }
                limit = Some(parsed);
            }
            "cursor" => {
                if cursor.is_some() {
                    return Err(ResourceError::InvalidParams(
                        "duplicate `cursor` query parameter".to_string(),
                    ));
                }
                if value.is_empty() || value.len() > MAX_CURSOR_BYTES {
                    return Err(ResourceError::InvalidParams(format!(
                        "`cursor` must be 1..={MAX_CURSOR_BYTES} ASCII bytes"
                    )));
                }
                cursor = Some(value.to_string());
            }
            other => {
                return Err(ResourceError::InvalidParams(format!(
                    "unknown collection query parameter `{}` (allowed: `limit`, `cursor`)",
                    aplan_output::sanitize::visible(other)
                )));
            }
        }
    }
    Ok((limit.unwrap_or(DEFAULT_PAGE_LIMIT), cursor))
}

fn percent_encode_query_value(value: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode_query_value(raw: &str, key: &str) -> Result<String, ResourceError> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(ResourceError::InvalidParams(format!(
                    "malformed percent-encoding in `{key}` query parameter"
                )));
            }
            let Some(hi) = hex_nibble(bytes[i + 1]) else {
                return Err(ResourceError::InvalidParams(format!(
                    "invalid hex digit in `{key}` query parameter"
                )));
            };
            let Some(lo) = hex_nibble(bytes[i + 2]) else {
                return Err(ResourceError::InvalidParams(format!(
                    "invalid hex digit in `{key}` query parameter"
                )));
            };
            decoded.push((hi << 4) | lo);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let text = String::from_utf8(decoded).map_err(|_| {
        ResourceError::InvalidParams(format!("`{key}` query parameter is not valid UTF-8"))
    })?;
    if text.chars().any(|c| c.is_control() || c == '\\') {
        return Err(ResourceError::InvalidParams(format!(
            "`{key}` query parameter contains forbidden control or backslash characters"
        )));
    }
    Ok(text)
}

fn check_confined_filter_value(decoded: &str, key: &str) -> Result<(), ResourceError> {
    if decoded.is_empty() || decoded.len() > MAX_FILTER_VALUE_BYTES {
        return Err(ResourceError::InvalidParams(format!(
            "`{key}` must be 1..={MAX_FILTER_VALUE_BYTES} bytes"
        )));
    }
    if decoded.split('/').any(|seg| seg == "." || seg == "..") {
        return Err(ResourceError::InvalidParams(format!(
            "`{key}` query parameter may not contain path traversal segments"
        )));
    }
    Ok(())
}

fn push_filter_value(
    target: &mut Vec<String>,
    decoded: String,
    key: &str,
) -> Result<(), ResourceError> {
    check_confined_filter_value(&decoded, key)?;
    if target.len() >= MAX_FILTER_VALUES_PER_DIMENSION {
        return Err(ResourceError::InvalidParams(format!(
            "at most {MAX_FILTER_VALUES_PER_DIMENSION} `{key}` values are allowed"
        )));
    }
    target.push(decoded);
    Ok(())
}

#[allow(clippy::too_many_lines)] // Exhaustive strict parser for all 10 RFC 6570 template parameters.
fn parse_completed_query(query: Option<&str>) -> Result<CompletedUriQuery, ResourceError> {
    let mut cuts: Vec<String> = Vec::new();
    let mut phases: Vec<u32> = Vec::new();
    let mut owners: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut since: Option<String> = None;
    let mut until: Option<String> = None;
    let mut oldest: Option<bool> = None;
    let mut history: Option<aplan_core::HistoryPolicy> = None;
    let mut limit: Option<usize> = None;
    let mut cursor: Option<String> = None;

    let Some(q) = query else {
        return Ok(CompletedUriQuery {
            cuts,
            phases,
            owners,
            tags,
            since,
            until,
            oldest: false,
            history: aplan_core::HistoryPolicy::Auto,
            limit: DEFAULT_PAGE_LIMIT,
            cursor,
        });
    };
    if q.is_empty() {
        return Err(ResourceError::InvalidParams(
            "empty query string is not canonical".to_string(),
        ));
    }

    for pair in q.split('&') {
        let Some((key, raw_value)) = pair.split_once('=') else {
            return Err(ResourceError::InvalidParams(
                "query parameter must have `key=value` form".to_string(),
            ));
        };
        if raw_value.is_empty() {
            return Err(ResourceError::InvalidParams(format!(
                "`{}` query parameter must not be empty",
                aplan_output::sanitize::visible(key)
            )));
        }
        match key {
            "cut" => {
                let decoded = percent_decode_query_value(raw_value, "cut")?;
                push_filter_value(&mut cuts, decoded, "cut")?;
            }
            "phase" => {
                let parsed: u32 = raw_value.parse().map_err(|_| {
                    ResourceError::InvalidParams(
                        "`phase` must be a canonical non-negative integer".to_string(),
                    )
                })?;
                if parsed.to_string() != raw_value {
                    return Err(ResourceError::InvalidParams(
                        "`phase` must use canonical base-10 digits without leading zeros or signs"
                            .to_string(),
                    ));
                }
                if phases.len() >= MAX_FILTER_VALUES_PER_DIMENSION {
                    return Err(ResourceError::InvalidParams(format!(
                        "at most {MAX_FILTER_VALUES_PER_DIMENSION} `phase` values are allowed"
                    )));
                }
                phases.push(parsed);
            }
            "owner" => {
                let decoded = percent_decode_query_value(raw_value, "owner")?;
                push_filter_value(&mut owners, decoded, "owner")?;
            }
            "tag" => {
                let decoded = percent_decode_query_value(raw_value, "tag")?;
                push_filter_value(&mut tags, decoded, "tag")?;
            }
            "since" => {
                if since.is_some() {
                    return Err(ResourceError::InvalidParams(
                        "duplicate `since` query parameter".to_string(),
                    ));
                }
                let decoded = percent_decode_query_value(raw_value, "since")?;
                check_confined_filter_value(&decoded, "since")?;
                since = Some(decoded);
            }
            "until" => {
                if until.is_some() {
                    return Err(ResourceError::InvalidParams(
                        "duplicate `until` query parameter".to_string(),
                    ));
                }
                let decoded = percent_decode_query_value(raw_value, "until")?;
                check_confined_filter_value(&decoded, "until")?;
                until = Some(decoded);
            }
            "oldest" => {
                if oldest.is_some() {
                    return Err(ResourceError::InvalidParams(
                        "duplicate `oldest` query parameter".to_string(),
                    ));
                }
                let val = match raw_value {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(ResourceError::InvalidParams(
                            "`oldest` must be `true` or `false`".to_string(),
                        ));
                    }
                };
                oldest = Some(val);
            }
            "history" => {
                if history.is_some() {
                    return Err(ResourceError::InvalidParams(
                        "duplicate `history` query parameter".to_string(),
                    ));
                }
                let parsed = match raw_value {
                    "auto" => aplan_core::HistoryPolicy::Auto,
                    "never" => aplan_core::HistoryPolicy::Never,
                    "required" => aplan_core::HistoryPolicy::Required,
                    _ => {
                        return Err(ResourceError::InvalidParams(
                            "`history` must be `auto`, `never`, or `required`".to_string(),
                        ));
                    }
                };
                history = Some(parsed);
            }
            "limit" => {
                if limit.is_some() {
                    return Err(ResourceError::InvalidParams(
                        "duplicate `limit` query parameter".to_string(),
                    ));
                }
                let parsed: usize = raw_value.parse().map_err(|_| {
                    ResourceError::InvalidParams(format!(
                        "`limit` must be a positive integer in 1..={MAX_PAGE_LIMIT}"
                    ))
                })?;
                if parsed == 0 || parsed > MAX_PAGE_LIMIT || parsed.to_string() != raw_value {
                    return Err(ResourceError::InvalidParams(format!(
                        "`limit` must be a canonical integer in 1..={MAX_PAGE_LIMIT} (got `{}`)",
                        aplan_output::sanitize::visible(raw_value)
                    )));
                }
                limit = Some(parsed);
            }
            "cursor" => {
                if cursor.is_some() {
                    return Err(ResourceError::InvalidParams(
                        "duplicate `cursor` query parameter".to_string(),
                    ));
                }
                let decoded = percent_decode_query_value(raw_value, "cursor")?;
                if decoded.is_empty() || decoded.len() > MAX_COMPLETED_CURSOR_BYTES {
                    return Err(ResourceError::InvalidParams(format!(
                        "`cursor` must be 1..={MAX_COMPLETED_CURSOR_BYTES} bytes"
                    )));
                }
                cursor = Some(decoded);
            }
            other => {
                return Err(ResourceError::InvalidParams(format!(
                    "unknown completed query parameter `{}` (allowed: `cut`, `phase`, `owner`, \
                     `tag`, `since`, `until`, `oldest`, `history`, `limit`, `cursor`)",
                    aplan_output::sanitize::visible(other)
                )));
            }
        }
    }

    Ok(CompletedUriQuery {
        cuts,
        phases,
        owners,
        tags,
        since,
        until,
        oldest: oldest.unwrap_or(false),
        history: history.unwrap_or(aplan_core::HistoryPolicy::Auto),
        limit: limit.unwrap_or(DEFAULT_PAGE_LIMIT),
        cursor,
    })
}

/// Format a canonical `aplan://collections/completed` URI in template parameter
/// order (`cut`, `phase`, `owner`, `tag`, `since`, `until`, `oldest`, `history`,
/// `limit`, `cursor`) with RFC 3986 percent-encoding on values.
#[must_use]
pub fn canonical_completed_uri(
    applied: &aplan_core::AppliedCompletedRequest,
    limit: usize,
    cursor: Option<&str>,
) -> String {
    let mut params: Vec<String> = Vec::new();
    for cut in &applied.selector.cuts {
        params.push(format!("cut={}", percent_encode_query_value(cut)));
    }
    for phase in &applied.selector.phases {
        params.push(format!("phase={phase}"));
    }
    for owner in &applied.selector.owners {
        params.push(format!("owner={}", percent_encode_query_value(owner)));
    }
    for tag in &applied.selector.tags {
        params.push(format!("tag={}", percent_encode_query_value(tag)));
    }
    if let Some(since) = &applied.since {
        params.push(format!("since={}", percent_encode_query_value(since)));
    }
    if let Some(until) = &applied.until {
        params.push(format!("until={}", percent_encode_query_value(until)));
    }
    params.push(format!("oldest={}", applied.oldest_first));
    params.push(format!("history={}", applied.history.as_str()));
    params.push(format!("limit={limit}"));
    if let Some(tok) = cursor {
        params.push(format!("cursor={}", percent_encode_query_value(tok)));
    }
    format!("aplan://collections/completed?{}", params.join("&"))
}

/// Build one bounded, deterministically ordered page for
/// `aplan://collections/completed{?cut,phase,owner,tag,since,until,oldest,history,limit,cursor}`
/// directly over [`aplan_app::completed_tasks`] (`A2-119`).
///
/// Enforces BOTH `limit <= MAX_PAGE_LIMIT` and `MAX_RESOURCE_BYTES` (shrinking
/// `completed.items` via [`aplan_app::truncate_completed_response`] while
/// preserving the request's `applied.limit` cursor binding so continuation
/// pages work cleanly even when a later page also hits the 64 KiB cap).
///
/// # Errors
///
/// Returns [`ResourceError::InvalidParams`] on invalid bounds, reversed bounds,
/// stale/malformed/tampered/oversized cursors, `history=required` failures,
/// oversized canonical next-page URIs, or a single row exceeding 64 KiB.
pub fn build_completed_page(
    plan: &aplan_core::load::Plan,
    app_input_identity: &str,
    repo_root: &Path,
    plan_root_rel: &Path,
    query: &CompletedUriQuery,
    input_identity: &InputIdentity,
) -> Result<CompletedCollectionResource, ResourceError> {
    let request = aplan_app::CompletedRequest {
        selector: aplan_core::select::Selector {
            cuts: query.cuts.clone(),
            phases: query.phases.clone(),
            tags: query.tags.clone(),
            owners: query.owners.clone(),
            statuses: Vec::new(),
            unclaimed: false,
        },
        since: query.since.clone(),
        until: query.until.clone(),
        oldest_first: query.oldest,
        history: query.history,
        limit: Some(query.limit),
        cursor: query.cursor.clone(),
        basis_revision: None,
        limits: aplan_core::HistoryLimits::default(),
    };
    let mut response =
        aplan_app::completed_tasks(plan, app_input_identity, repo_root, plan_root_rel, &request)
            .map_err(|err| ResourceError::InvalidParams(err.to_string()))?;

    let order = if response.applied.oldest_first {
        "closed_at_asc"
    } else {
        "closed_at_desc"
    };
    let canonical_uri =
        canonical_completed_uri(&response.applied, query.limit, query.cursor.as_deref());
    let mut truncated_by_size_cap = false;

    loop {
        let next_uri = response
            .next_cursor
            .as_deref()
            .map(|tok| canonical_completed_uri(&response.applied, query.limit, Some(tok)));
        if let Some(uri) = &next_uri
            && uri.len() > MAX_URI_BYTES
        {
            return Err(ResourceError::InvalidParams(format!(
                "canonical continuation URI exceeds the {MAX_URI_BYTES}-byte URI limit"
            )));
        }
        let page = CompletedCollectionResource {
            contract_version: MCP_CONTRACT_VERSION.to_string(),
            resource: canonical_uri.clone(),
            input_identity: input_identity.clone(),
            order: order.to_string(),
            complete: !response.truncated,
            truncated_by_size_cap,
            next_uri,
            completed: response.clone(),
        };
        let serialized_len = serde_json::to_string_pretty(&page)
            .map_err(|e| ResourceError::Internal(format!("serializing completed page: {e}")))?
            .len();
        if serialized_len <= MAX_RESOURCE_BYTES || response.items.len() <= 1 {
            if serialized_len > MAX_RESOURCE_BYTES {
                return Err(ResourceError::InvalidParams(format!(
                    "single completed-history entry exceeds the {MAX_RESOURCE_BYTES}-byte \
                     resource limit"
                )));
            }
            return Ok(page);
        }
        let keep = (response.items.len() / 2).max(1);
        aplan_app::truncate_completed_response(&mut response, keep);
        truncated_by_size_cap = true;
    }
}

/// Strictly parsed, tamper-evident, snapshot-bound continuation cursor.
///
/// Format: `v1.<offset>.<limit>.<request_digest_16hex>.<input_digest_16hex>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageCursor {
    pub offset: usize,
    pub limit: usize,
    pub request_digest: String,
    pub input_digest: String,
}

impl PageCursor {
    /// Compute the 16-hex binding digest for a canonical request scope
    /// (`collection`, `order`, `limit`, plus any selector parameters).
    #[must_use]
    pub fn request_binding(collection: &str, order: &str, limit: usize) -> String {
        let mut h = Sha256::new();
        h.update(b"aplan-mcp-cursor-req-v1:");
        h.update(collection.as_bytes());
        h.update(b"|");
        h.update(order.as_bytes());
        h.update(b"|");
        h.update(limit.to_string().as_bytes());
        hex_prefix_16(&h.finalize())
    }

    /// Compute the 16-hex binding digest for a loaded [`InputIdentity`].
    #[must_use]
    pub fn input_binding(identity: &InputIdentity) -> String {
        let mut h = Sha256::new();
        h.update(b"aplan-mcp-cursor-input-v1:");
        h.update(identity.plan_sha256.as_bytes());
        h.update(b"|");
        h.update(identity.config_sha256.as_bytes());
        hex_prefix_16(&h.finalize())
    }

    /// Encode this cursor as an opaque URI-safe token guaranteed <=
    /// [`MAX_CURSOR_BYTES`].
    #[must_use]
    pub fn encode(&self) -> String {
        format!(
            "v1.{}.{}.{}.{}",
            self.offset, self.limit, self.request_digest, self.input_digest
        )
    }

    /// Strictly parse a cursor token and verify it against the current request
    /// and input identity.
    ///
    /// # Errors
    ///
    /// Refuses malformed, oversized, tampered, cross-request, or stale cursors.
    pub fn decode_and_verify(
        token: &str,
        expected_request: &str,
        expected_input: &str,
        expected_limit: usize,
    ) -> Result<Self, ResourceError> {
        if token.is_empty() || token.len() > MAX_CURSOR_BYTES {
            return Err(ResourceError::InvalidParams(format!(
                "continuation cursor exceeds maximum length of {MAX_CURSOR_BYTES} bytes"
            )));
        }
        if !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.')
        {
            return Err(ResourceError::InvalidParams(
                "malformed continuation cursor: invalid characters".to_string(),
            ));
        }
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 5 || parts[0] != "v1" {
            return Err(ResourceError::InvalidParams(
                "malformed continuation cursor: expected v1 token".to_string(),
            ));
        }
        let offset: usize = parts[1].parse().map_err(|_| {
            ResourceError::InvalidParams("malformed continuation cursor: bad offset".to_string())
        })?;
        let limit: usize = parts[2].parse().map_err(|_| {
            ResourceError::InvalidParams("malformed continuation cursor: bad limit".to_string())
        })?;
        let req_hex = parts[3];
        let input_hex = parts[4];
        if offset == 0
            || limit == 0
            || limit > MAX_PAGE_LIMIT
            || !is_hex16(req_hex)
            || !is_hex16(input_hex)
        {
            return Err(ResourceError::InvalidParams(
                "malformed continuation cursor: invalid bounds or digest format".to_string(),
            ));
        }
        // Reject non-canonical spellings (e.g. leading zeros on offset/limit).
        let canonical = format!("v1.{offset}.{limit}.{req_hex}.{input_hex}");
        if canonical != token {
            return Err(ResourceError::InvalidParams(
                "malformed continuation cursor: non-canonical encoding".to_string(),
            ));
        }
        if input_hex != expected_input {
            return Err(ResourceError::InvalidParams(
                "stale continuation cursor: plan or configuration inputs changed since the cursor \
                 was issued; restart from page 1"
                    .to_string(),
            ));
        }
        if limit != expected_limit || req_hex != expected_request {
            return Err(ResourceError::InvalidParams(
                "continuation cursor does not match this collection, order, or limit; a cursor \
                 cannot broaden or alter a query"
                    .to_string(),
            ));
        }
        Ok(Self {
            offset,
            limit,
            request_digest: req_hex.to_string(),
            input_digest: input_hex.to_string(),
        })
    }
}

fn hex_prefix_16(digest: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(16);
    for b in digest.iter().take(8) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn is_hex16(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Build the bounded `aplan://plan/summary` resource.
#[must_use]
pub fn build_plan_summary(
    plan: &aplan_core::load::Plan,
    ready_count: usize,
    errors: usize,
    warnings: usize,
    denied: bool,
    input_identity: InputIdentity,
) -> PlanSummaryResource {
    let mut status_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut cut_count = 0usize;
    for task in &plan.tasks {
        *status_counts
            .entry(task.task.status.to_string())
            .or_default() += 1;
        if task.task.is_milestone() {
            cut_count += 1;
        }
    }
    PlanSummaryResource {
        contract_version: MCP_CONTRACT_VERSION.to_string(),
        resource: "aplan://plan/summary".to_string(),
        input_identity,
        complete: true,
        total_tasks: plan.tasks.len(),
        ready_count,
        cut_count,
        status_counts,
        errors,
        warnings,
        denied,
    }
}

/// Build a bounded `aplan://tasks/{id}` resource from a computed `Show` payload.
///
/// # Errors
///
/// Returns [`ResourceError::InvalidParams`] if the serialized task payload
/// exceeds [`MAX_RESOURCE_BYTES`].
pub fn build_task_detail(
    id: &str,
    show_value: serde_json::Value,
    input_identity: InputIdentity,
) -> Result<TaskDetailResource, ResourceError> {
    let task: ShowPayload = serde_json::from_value(show_value)
        .map_err(|e| ResourceError::Internal(format!("encoding task detail: {e}")))?;
    let resource = TaskDetailResource {
        contract_version: MCP_CONTRACT_VERSION.to_string(),
        resource: format!("aplan://tasks/{id}"),
        input_identity,
        complete: true,
        truncated: false,
        task,
    };
    let bytes = serde_json::to_string_pretty(&resource)
        .map_err(|e| ResourceError::Internal(format!("serializing task detail: {e}")))?;
    if bytes.len() > MAX_RESOURCE_BYTES {
        return Err(ResourceError::InvalidParams(format!(
            "task resource `aplan://tasks/{id}` exceeds the {MAX_RESOURCE_BYTES}-byte resource limit"
        )));
    }
    Ok(resource)
}

/// Build one bounded, deterministically ordered page for
/// `aplan://collections/ready{?limit,cursor}`.
///
/// Enforces BOTH `limit <= MAX_PAGE_LIMIT` and `MAX_RESOURCE_BYTES` (shrinking
/// `items` cleanly and advancing `next_cursor` if a full `limit` slice would
/// exceed 64 KiB).
///
/// # Errors
///
/// Returns [`ResourceError::InvalidParams`] if `cursor` is invalid, stale,
/// mismatched, or points past the end of the snapshot.
pub fn build_ready_page(
    all_items: &[ReadyCollectionItem],
    limit: usize,
    cursor: Option<&str>,
    input_identity: &InputIdentity,
) -> Result<CollectionPage<ReadyCollectionItem>, ResourceError> {
    const COLLECTION: &str = "collections/ready";
    const ORDER: &str = "ready_rank_asc";

    let req_binding = PageCursor::request_binding(COLLECTION, ORDER, limit);
    let input_binding = PageCursor::input_binding(input_identity);

    let offset = match cursor {
        None => 0,
        Some(tok) => {
            let parsed = PageCursor::decode_and_verify(tok, &req_binding, &input_binding, limit)?;
            if parsed.offset >= all_items.len() {
                return Err(ResourceError::InvalidParams(
                    "continuation cursor offset is beyond the end of this collection".to_string(),
                ));
            }
            parsed.offset
        }
    };

    let total_matching = all_items.len();
    let max_end = offset.saturating_add(limit).min(total_matching);
    let mut take = max_end.saturating_sub(offset);
    let mut truncated_by_size_cap = false;

    let canonical_uri = match cursor {
        Some(tok) => format!("aplan://{COLLECTION}?limit={limit}&cursor={tok}"),
        None => format!("aplan://{COLLECTION}?limit={limit}"),
    };

    // Shrink `take` until the serialized JSON page fits within MAX_RESOURCE_BYTES.
    loop {
        let end = offset.saturating_add(take);
        let complete = end >= total_matching;
        let (next_cursor, next_uri) = if complete {
            (None, None)
        } else {
            let tok = PageCursor {
                offset: end,
                limit,
                request_digest: req_binding.clone(),
                input_digest: input_binding.clone(),
            }
            .encode();
            let uri = format!("aplan://{COLLECTION}?limit={limit}&cursor={tok}");
            (Some(tok), Some(uri))
        };
        let page = CollectionPage {
            contract_version: MCP_CONTRACT_VERSION.to_string(),
            resource: canonical_uri.clone(),
            input_identity: input_identity.clone(),
            order: ORDER.to_string(),
            limit,
            returned: take,
            total_matching,
            complete,
            truncated_by_size_cap,
            next_cursor,
            next_uri,
            items: all_items[offset..end].to_vec(),
        };
        let serialized_len = serde_json::to_string_pretty(&page)
            .map_err(|e| ResourceError::Internal(format!("serializing ready page: {e}")))?
            .len();
        if serialized_len <= MAX_RESOURCE_BYTES || take <= 1 {
            if serialized_len > MAX_RESOURCE_BYTES {
                return Err(ResourceError::InvalidParams(format!(
                    "single collection entry exceeds the {MAX_RESOURCE_BYTES}-byte resource limit"
                )));
            }
            return Ok(page);
        }
        take = (take / 2).max(1);
        truncated_by_size_cap = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(plan_sha: &str, cfg_sha: &str) -> InputIdentity {
        InputIdentity {
            plan_sha256: plan_sha.to_string(),
            config_sha256: cfg_sha.to_string(),
            source: "working_tree".to_string(),
            trust: "hermetic".to_string(),
        }
    }

    #[test]
    fn hostile_and_malformed_uris_are_confined_and_refused() {
        for bad in [
            "",
            "file:///etc/passwd",
            "https://example.invalid/tasks/A-1",
            "aplan://tasks/../etc/passwd",
            "aplan://tasks/A-1/extra",
            "aplan://tasks/A-1?limit=5",
            "aplan://tasks/A-1#frag",
            "aplan://tasks/A-1\n",
            "aplan://tasks/A\\1",
            "aplan://plan/summary?x=1",
            "aplan://collections/ready?",
            "aplan://collections/ready?limit=0",
            "aplan://collections/ready?limit=51",
            "aplan://collections/ready?limit=abc",
            "aplan://collections/ready?limit=5&limit=6",
            "aplan://collections/ready?unknown=1",
            "aplan://collections/completed?",
            "aplan://collections/completed?at=HEAD",
            "aplan://collections/completed?root=/etc",
            "aplan://collections/completed?path=../../etc/passwd",
            "aplan://collections/completed?limit=0",
            "aplan://collections/completed?limit=51",
            "aplan://collections/completed?limit=05",
            "aplan://collections/completed?oldest=yes",
            "aplan://collections/completed?history=always",
            "aplan://collections/completed?since=2026-01-01&since=2026-01-02",
            "aplan://collections/completed?until=2026-01-01&until=2026-01-02",
            "aplan://collections/completed?phase=abc",
            "aplan://collections/completed?phase=01",
            "aplan://collections/completed?cut=%ZZ",
            "aplan://collections/completed?cut=a%00b",
            "aplan://collections/completed?cut=../secret",
            &format!(
                "aplan://collections/completed?cursor={}",
                "a".repeat(MAX_COMPLETED_CURSOR_BYTES + 1)
            ),
            &format!("aplan://tasks/{}", "a".repeat(MAX_URI_BYTES + 1)),
        ] {
            let err = parse_resource_uri(bad)
                .expect_err(&format!("expected refusal for hostile URI {bad:?}"));
            assert!(
                matches!(err, ResourceError::InvalidParams(_)),
                "expected InvalidParams for {bad:?}, got {err:?}"
            );
        }

        let missing = parse_resource_uri("aplan://unknown/view")
            .expect_err("unknown aplan path must report NotFound");
        assert!(matches!(missing, ResourceError::NotFound(_)), "{missing:?}");
    }

    #[test]
    fn cursor_rejects_staleness_tampering_broadening_and_oversize() {
        let items: Vec<ReadyCollectionItem> = (1..=5)
            .map(|i| ReadyCollectionItem {
                id: format!("A-{i}"),
                title: format!("Task {i}"),
                status: "todo".to_string(),
                phase: 1,
                rank: i,
                effort_days: Some(1.0),
                cut: None,
                owner: None,
                held_by: None,
            })
            .collect();

        let id1 = identity("1111111111111111", "none");
        let page1 = build_ready_page(&items, 2, None, &id1).expect("page 1");
        assert_eq!(page1.returned, 2);
        assert_eq!(page1.total_matching, 5);
        assert!(!page1.complete);
        let cursor1 = page1.next_cursor.as_deref().expect("cursor 1");
        assert!(cursor1.len() <= MAX_CURSOR_BYTES);

        // Valid page 2 under identical input identity and limit.
        let page2 = build_ready_page(&items, 2, Some(cursor1), &id1).expect("page 2");
        assert_eq!(page2.items[0].id, "A-3");
        assert!(!page2.complete);
        let cursor2 = page2.next_cursor.as_deref().expect("cursor 2");

        // Valid page 3 reaches end.
        let page3 = build_ready_page(&items, 2, Some(cursor2), &id1).expect("page 3");
        assert_eq!(page3.items.len(), 1);
        assert!(page3.complete);
        assert!(page3.next_cursor.is_none());

        // Changed input identity (plan changed) refuses stale cursor.
        let id2 = identity("2222222222222222", "none");
        let stale = build_ready_page(&items, 2, Some(cursor1), &id2)
            .expect_err("changed plan must refuse stale cursor");
        assert!(
            matches!(stale, ResourceError::InvalidParams(ref m) if m.contains("stale")),
            "{stale:?}"
        );

        // Broadening limit with page 1's cursor is refused.
        let broadened = build_ready_page(&items, 10, Some(cursor1), &id1)
            .expect_err("broadened limit must refuse cursor");
        assert!(matches!(broadened, ResourceError::InvalidParams(_)));

        // Tampering with the offset or digest is refused.
        let tampered_same_len =
            format!("v1.2.2.0000000000000000.{}", &cursor1[cursor1.len() - 16..]);
        assert!(build_ready_page(&items, 2, Some(&tampered_same_len), &id1).is_err());
        // Non-canonical leading-zero offset is refused.
        let non_canonical = cursor1.replacen("v1.2.", "v1.02.", 1);
        assert!(build_ready_page(&items, 2, Some(&non_canonical), &id1).is_err());
        // Oversized cursor is refused.
        let huge = "a".repeat(MAX_CURSOR_BYTES + 1);
        assert!(build_ready_page(&items, 2, Some(&huge), &id1).is_err());
        // Offset still valid for same binding only if offset < total; verify tampered offset 99 fails.
        let past_end = cursor1.replacen("v1.2.", "v1.99.", 1);
        assert!(build_ready_page(&items, 2, Some(&past_end), &id1).is_err());
    }

    #[test]
    fn ready_page_shrinks_under_64kib_size_cap_and_continues_cleanly() {
        let items: Vec<ReadyCollectionItem> = (1..=40)
            .map(|i| ReadyCollectionItem {
                id: format!("BIG-{i}"),
                title: "x".repeat(3_000),
                status: "todo".to_string(),
                phase: 1,
                rank: i,
                effort_days: Some(1.0),
                cut: None,
                owner: None,
                held_by: None,
            })
            .collect();

        let id = identity("abcdef0123456789", "none");
        let page = build_ready_page(&items, 40, None, &id).expect("capped page");
        assert!(page.truncated_by_size_cap);
        assert!(!page.complete);
        assert!(page.returned < 40 && page.returned >= 1);
        let serialized = serde_json::to_string_pretty(&page).expect("serialize");
        assert!(
            serialized.len() <= MAX_RESOURCE_BYTES,
            "page exceeded 64 KiB cap: {}",
            serialized.len()
        );
        let next = build_ready_page(&items, 40, page.next_cursor.as_deref(), &id)
            .expect("continuation after cap");
        assert_eq!(next.items[0].rank, page.returned + 1);
    }

    #[test]
    fn completed_uri_canonical_round_trip() {
        let applied = aplan_core::AppliedCompletedRequest {
            since: Some("2026-10-01T00:00:00Z".to_string()),
            until: Some("2026-10-31T23:59:59Z".to_string()),
            oldest_first: true,
            history: aplan_core::HistoryPolicy::Never,
            limit: Some(15),
            selector: aplan_core::AppliedCompletedSelector {
                cuts: vec!["v0.8".to_string()],
                phases: vec![2],
                owners: vec!["alice smith".to_string()],
                tags: vec!["mcp".to_string(), "history".to_string()],
            },
        };
        let uri = canonical_completed_uri(&applied, 15, Some("v1|abc|unversioned|fp|1|100|A-1"));
        assert!(uri.len() <= MAX_URI_BYTES);
        let ParsedResourceUri::CompletedCollection(parsed) =
            parse_resource_uri(&uri).expect("canonical URI parses cleanly")
        else {
            panic!("expected CompletedCollection");
        };
        assert_eq!(parsed.cuts, vec!["v0.8"]);
        assert_eq!(parsed.phases, vec![2]);
        assert_eq!(parsed.owners, vec!["alice smith"]);
        assert_eq!(parsed.tags, vec!["mcp", "history"]);
        assert_eq!(parsed.since.as_deref(), Some("2026-10-01T00:00:00Z"));
        assert_eq!(parsed.until.as_deref(), Some("2026-10-31T23:59:59Z"));
        assert!(parsed.oldest);
        assert_eq!(parsed.history, aplan_core::HistoryPolicy::Never);
        assert_eq!(parsed.limit, 15);
        assert_eq!(
            parsed.cursor.as_deref(),
            Some("v1|abc|unversioned|fp|1|100|A-1")
        );
        // Re-emitting from the parsed query is idempotent.
        let reparsed_uri =
            canonical_completed_uri(&applied, parsed.limit, parsed.cursor.as_deref());
        assert_eq!(reparsed_uri, uri);
    }

    #[test]
    fn completed_page_shrinks_under_64kib_size_cap_and_continues_cleanly() {
        // Build a plan with 35 large done tasks so limit=35 exceeds 64 KiB on
        // BOTH page 1 and page 2, proving continuous size-cap pagination.
        let dir = tempfile::Builder::new()
            .prefix("mcp-completed-cap-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let tasks: Vec<aplan_core::load::LoadedTask> = (1..=35)
            .map(|i| aplan_core::load::LoadedTask {
                path: dir.path().join(format!("aplan/tasks/C-{i:02}.md")),
                task: aplan_core::Task {
                    id: format!("C-{i:02}"),
                    title: format!("Task {i:02} {}", "x".repeat(3_500)),
                    phase: 1,
                    status: aplan_core::Status::Done,
                    owner: Some("alice".to_string()),
                    effort_days: Some(1.0),
                    blocked_by: Vec::new(),
                    prefer_after: Vec::new(),
                    ratified_by: Vec::new(),
                    contains: Vec::new(),
                    integrates: Vec::new(),
                    evidence: Vec::new(),
                    paths: Vec::new(),
                    traces: Vec::new(),
                    trace_fingerprints: BTreeMap::new(),
                    evidence_fingerprints: BTreeMap::new(),
                    tags: vec!["mcp".to_string()],
                    revisit_at: None,
                    version_impact: None,
                    version_impact_reason: None,
                    version: None,
                    closed_at: Some(format!("2026-10-10T00:{i:02}:00Z")),
                    closed_at_source: Some(aplan_core::ClosedAtSource::Local),
                },
                criteria: aplan_core::load::Criteria::default(),
                fields: BTreeMap::new(),
            })
            .collect();
        let plan = aplan_core::load::Plan {
            tasks,
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        };
        let id = identity("1234567890abcdef", "none");
        let mut cursor: Option<String> = None;
        let mut collected = Vec::new();
        let mut capped_pages = 0usize;
        loop {
            let q = CompletedUriQuery {
                cuts: Vec::new(),
                phases: Vec::new(),
                owners: Vec::new(),
                tags: Vec::new(),
                since: None,
                until: None,
                oldest: false,
                history: aplan_core::HistoryPolicy::Never,
                limit: 35,
                cursor: cursor.clone(),
            };
            let page =
                build_completed_page(&plan, "app-sha-1", dir.path(), Path::new("aplan"), &q, &id)
                    .expect("capped completed page");
            let bytes = serde_json::to_string_pretty(&page).expect("serialize");
            assert!(
                bytes.len() <= MAX_RESOURCE_BYTES,
                "completed page exceeded 64 KiB cap: {}",
                bytes.len()
            );
            if page.truncated_by_size_cap {
                capped_pages += 1;
            }
            collected.extend(page.completed.items.iter().map(|r| r.id.clone()));
            if page.complete {
                assert!(page.next_uri.is_none());
                assert!(page.completed.next_cursor.is_none());
                break;
            }
            let next_uri = page.next_uri.as_deref().expect("next_uri");
            let ParsedResourceUri::CompletedCollection(next_q) =
                parse_resource_uri(next_uri).expect("next_uri parses")
            else {
                panic!("expected CompletedCollection next_uri");
            };
            assert_eq!(next_q.limit, 35);
            cursor = next_q.cursor;
        }
        assert!(capped_pages >= 2, "expected multiple size-capped pages");
        assert_eq!(collected.len(), 35);
        assert_eq!(collected.first().map(String::as_str), Some("C-35"));
        assert_eq!(collected.last().map(String::as_str), Some("C-01"));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn resource_uri_parsing_percent_decoding_and_cursor_verification_cover_all_refusals() {
        // URI confinement, scheme, length, and ready collection query errors.
        let long_uri = format!("aplan://tasks/{}", "A".repeat(MAX_URI_BYTES + 10));
        for bad in [
            "https://example.com/tasks/T-1",
            long_uri.as_str(),
            "aplan://unknown/thing",
            "aplan://collections/ready?",
            "aplan://collections/ready?noequals",
            "aplan://collections/ready?limit=5&limit=6",
            "aplan://collections/ready?limit=abc",
            "aplan://collections/ready?limit=0",
            "aplan://collections/ready?cursor=a&cursor=b",
            "aplan://collections/ready?bogus=1",
        ] {
            assert!(
                parse_resource_uri(bad).is_err(),
                "expected URI refusal for `{bad}`"
            );
        }

        // Percent-decoding & filter confinement errors on `collections/completed`.
        let too_many_cuts = (0..=MAX_FILTER_VALUES_PER_DIMENSION)
            .map(|i| format!("cut=c{i}"))
            .collect::<Vec<_>>()
            .join("&");
        let too_many_phases = (0..=MAX_FILTER_VALUES_PER_DIMENSION)
            .map(|i| format!("phase={i}"))
            .collect::<Vec<_>>()
            .join("&");
        let huge_cursor = "x".repeat(MAX_COMPLETED_CURSOR_BYTES + 1);
        for query in [
            "",
            "noequals",
            "cut=",
            "cut=%",
            "cut=%2",
            "cut=%ZZ",
            "cut=%2Z",
            "cut=%FF",
            "cut=%00",
            "cut=%5C",
            "cut=..",
            "cut=a/..",
            "phase=abc",
            "phase=01",
            too_many_cuts.as_str(),
            too_many_phases.as_str(),
            "since=2026-01-01&since=2026-01-02",
            "until=2026-01-01&until=2026-01-02",
            "oldest=true&oldest=false",
            "oldest=maybe",
            "history=auto&history=never",
            "history=invalid",
            "limit=5&limit=6",
            "limit=abc",
            "limit=0",
            "limit=05",
            "cursor=a&cursor=b",
            &format!("cursor={huge_cursor}"),
            "unknown_key=1",
        ] {
            let uri = format!("aplan://collections/completed?{query}");
            assert!(
                parse_resource_uri(&uri).is_err(),
                "expected completed URI refusal for `{uri}`"
            );
        }

        // `PageCursor::decode_and_verify` error branches.
        let req = PageCursor::request_binding("ready", "canonical", 10);
        let inp = "0123456789abcdef";
        let too_long_tok = "a".repeat(MAX_CURSOR_BYTES + 1);
        for tok in [
            "",
            too_long_tok.as_str(),
            "v1.1.10.0123456789abcdef.0123456789abcde!",
            "v2.1.10.0123456789abcdef.0123456789abcdef",
            "v1.bad.10.0123456789abcdef.0123456789abcdef",
            "v1.1.bad.0123456789abcdef.0123456789abcdef",
            "v1.0.10.0123456789abcdef.0123456789abcdef",
            "v1.01.10.0123456789abcdef.0123456789abcdef",
            &format!("v1.1.10.{req}.ffffffffffffffff"),
            &format!("v1.1.10.ffffffffffffffff.{inp}"),
        ] {
            assert!(
                PageCursor::decode_and_verify(tok, &req, inp, 10).is_err(),
                "expected cursor refusal for `{tok}`"
            );
        }

        // Single completed item exceeding MAX_RESOURCE_BYTES is refused cleanly.
        let dir = tempfile::Builder::new()
            .prefix("mcp-oversized-completed-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let giant_task = aplan_core::load::LoadedTask {
            path: std::path::PathBuf::from("aplan/tasks/G-1.md"),
            task: aplan_core::Task {
                id: "G-1".to_string(),
                title: "X".repeat(MAX_RESOURCE_BYTES + 1024),
                phase: 1,
                status: aplan_core::Status::Done,
                owner: None,
                effort_days: Some(1.0),
                blocked_by: Vec::new(),
                prefer_after: Vec::new(),
                ratified_by: Vec::new(),
                contains: Vec::new(),
                integrates: Vec::new(),
                evidence: Vec::new(),
                paths: Vec::new(),
                traces: Vec::new(),
                trace_fingerprints: BTreeMap::new(),
                evidence_fingerprints: BTreeMap::new(),
                tags: Vec::new(),
                revisit_at: None,
                version_impact: None,
                version_impact_reason: None,
                version: None,
                closed_at: Some("2026-10-10T00:00:00Z".to_string()),
                closed_at_source: Some(aplan_core::ClosedAtSource::Local),
            },
            criteria: aplan_core::load::Criteria::default(),
            fields: BTreeMap::new(),
        };
        let plan = aplan_core::Plan {
            tasks: vec![giant_task],
            load_diagnostics: Vec::new(),
            unloadable: BTreeMap::new(),
        };
        let identity = InputIdentity {
            plan_sha256: "a".repeat(64),
            config_sha256: "b".repeat(64),
            source: "working_tree".to_string(),
            trust: "hermetic".to_string(),
        };
        let q = CompletedUriQuery {
            cuts: Vec::new(),
            phases: Vec::new(),
            owners: Vec::new(),
            tags: Vec::new(),
            since: None,
            until: None,
            oldest: false,
            history: aplan_core::HistoryPolicy::Never,
            limit: 10,
            cursor: None,
        };
        let err = build_completed_page(
            &plan,
            "app-id",
            dir.path(),
            Path::new("aplan"),
            &q,
            &identity,
        )
        .expect_err("single entry exceeding 64 KiB cap must be refused");
        assert!(matches!(err, ResourceError::InvalidParams(_)), "{err:?}");
    }
}
