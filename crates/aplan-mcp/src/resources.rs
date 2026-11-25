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

use rmcp::model::{Resource, ResourceTemplate};
use sha2::{Digest as _, Sha256};

use crate::contract::{
    CollectionPage, DEFAULT_PAGE_LIMIT, InputIdentity, MAX_CURSOR_BYTES, MAX_PAGE_LIMIT,
    MAX_RESOURCE_BYTES, MAX_URI_BYTES, MCP_CONTRACT_VERSION, PlanSummaryResource,
    ReadyCollectionItem, ShowPayload, TaskDetailResource,
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
    let bytes = serde_json::to_vec(&resource)
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
        let serialized_len = serde_json::to_vec(&page)
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
            &format!("aplan://tasks/{}", "a".repeat(600)),
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
        let serialized = serde_json::to_vec(&page).expect("serialize");
        assert!(
            serialized.len() <= MAX_RESOURCE_BYTES,
            "page exceeded 64 KiB cap: {}",
            serialized.len()
        );
        let next = build_ready_page(&items, 40, page.next_cursor.as_deref(), &id)
            .expect("continuation after cap");
        assert_eq!(next.items[0].rank, page.returned + 1);
    }
}
