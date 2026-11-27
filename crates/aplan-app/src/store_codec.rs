//! Compact, strictly-bounded binary codec for private `PlanStore` generations
//! (`FORMAT_EPOCH = 5`, `A2-134`).
//!
//! The on-disk cache is private, disposable, host-local derived state. Earlier
//! JSON generations deserialized and immediately re-serialized 8.64 MiB on a
//! 10,000-task plan solely to check integrity, leaving exact-hit reads well
//! above the 100 ms p95 budget despite zero YAML parses.
//!
//! This codec stores a fixed 56-byte header (`magic`, `format_epoch`,
//! `payload_len`, `blake3(payload)`) followed by a compact payload. Integrity
//! is verified on the raw payload slice before a single byte is decoded, and
//! every count and byte length is range-checked against both explicit caps and
//! the remaining slice length prior to allocation. Zero `unsafe` code and zero
//! extra dependencies are used.

use std::collections::BTreeMap;
use std::sync::Arc;

use aplan_core::diagnostic::Location;
use aplan_core::load::LoadedTask;
use aplan_core::task::{ClosedAtSource, Evidence, EvidenceKind, Status, Task, VersionImpact};

use super::{
    CachedAgentContractProvenance, CachedArtifact, CachedCandidate, CachedCriteria,
    CachedDiagnostic, CachedLocation, CachedPlanDigest, CachedReport, CachedTask, EncodedPath,
    FORMAT_EPOCH, FileObservation, FileType, Generation, MAX_CACHE_BYTES, MAX_CANDIDATES,
    MAX_DIAGNOSTIC_TEXT_BYTES, MAX_PATH_BYTES, MAX_REPORT_DIAGNOSTICS, MAX_REPORT_TEXT_BYTES,
    TimeValue,
};

const MAGIC_BYTES: [u8; 16] = *b"aplan-plan-v6\0\0\0";
const HEADER_LEN: usize = 16 + 4 + 4 + 32;
const MAX_STRING_BYTES: usize = MAX_DIAGNOSTIC_TEXT_BYTES;
const MAX_TASK_LIST_ITEMS: usize = 100_000;
const MAX_TASK_FIELDS: usize = 256;
const MAX_REPORTS: usize = 8;

/// Encode a generation into a self-contained checksummed binary frame.
///
/// `plan_tasks` supplies the authoritative [`LoadedTask`] storage for any
/// [`CachedArtifact::TaskRef`] entry moved into the snapshot without cloning.
///
/// Returns `None` if the encoded payload would exceed [`MAX_CACHE_BYTES`] or
/// contain non-representable values.
#[cfg(test)]
pub(super) fn encode_generation(
    generation: &Generation,
    plan_tasks: &[LoadedTask],
) -> Option<Vec<u8>> {
    encode_generation_parts(
        &generation.magic,
        generation.format_epoch,
        generation.semantic_epoch,
        &generation.platform,
        &generation.schema_sha256,
        generation.writer_identity.as_deref(),
        &generation.root,
        generation.configuration_file.as_ref(),
        generation.created_at,
        generation.plan_input_digest.as_ref(),
        &generation.candidates,
        &generation.reports,
        plan_tasks,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_generation_parts(
    magic: &str,
    format_epoch: u32,
    semantic_epoch: u32,
    platform: &str,
    schema_sha256: &str,
    writer_identity: Option<&str>,
    root: &EncodedPath,
    configuration_file: Option<&EncodedPath>,
    created_at: TimeValue,
    plan_input_digest: Option<&CachedPlanDigest>,
    candidates: &[CachedCandidate],
    reports: &[CachedReport],
    plan_tasks: &[LoadedTask],
) -> Option<Vec<u8>> {
    let estimated = HEADER_LEN.saturating_add(
        candidates
            .len()
            .saturating_mul(256)
            .min(16 * 1024 * 1024)
            .saturating_add(512),
    );
    let mut frame = Vec::with_capacity(estimated);
    frame.resize(HEADER_LEN, 0);

    let mut enc = Encoder(&mut frame);
    enc.write_str(magic);
    enc.write_u32(format_epoch);
    enc.write_u32(semantic_epoch);
    enc.write_str(platform);
    enc.write_str(schema_sha256);
    enc.write_opt_str(writer_identity);
    enc.write_encoded_path(root);
    enc.write_opt_encoded_path(configuration_file);
    enc.write_time_value(created_at);
    match plan_input_digest {
        Some(cached) => {
            enc.write_u8(1);
            enc.write_encoded_path(&cached.root_spelling);
            enc.write_encoded_path(&cached.identity_root);
            enc.write_str(&cached.sha256);
        }
        None => enc.write_u8(0),
    }

    enc.write_varint(candidates.len() as u64);
    for candidate in candidates {
        if !enc.write_candidate(candidate, plan_tasks) {
            return None;
        }
    }

    enc.write_varint(reports.len() as u64);
    for report in reports {
        if !enc.write_report(report) {
            return None;
        }
    }

    if u64::try_from(frame.len()).ok()? > MAX_CACHE_BYTES {
        return None;
    }
    let payload_len = u32::try_from(frame.len().checked_sub(HEADER_LEN)?).ok()?;
    let checksum: [u8; 32] = blake3::hash(&frame[HEADER_LEN..]).into();

    frame[..16].copy_from_slice(&MAGIC_BYTES);
    frame[16..20].copy_from_slice(&FORMAT_EPOCH.to_le_bytes());
    frame[20..24].copy_from_slice(&payload_len.to_le_bytes());
    frame[24..56].copy_from_slice(&checksum);
    Some(frame)
}

/// Verify the frame header and BLAKE3 checksum on raw bytes, then decode the
/// generation with strict pre-allocation bounds.
pub(super) fn decode_generation(bytes: &[u8]) -> Option<Generation> {
    if bytes.len() < HEADER_LEN || u64::try_from(bytes.len()).ok()? > MAX_CACHE_BYTES {
        return None;
    }
    if bytes[..16] != MAGIC_BYTES {
        return None;
    }
    let frame_epoch = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
    if frame_epoch != FORMAT_EPOCH {
        return None;
    }
    let payload_len = usize::try_from(u32::from_le_bytes(bytes[20..24].try_into().ok()?)).ok()?;
    let expected_checksum = &bytes[24..56];
    let payload = &bytes[HEADER_LEN..];
    if payload.len() != payload_len {
        return None;
    }
    if blake3::hash(payload).as_bytes() != expected_checksum {
        return None;
    }

    let mut dec = Decoder {
        data: payload,
        pos: 0,
    };
    let magic = dec.read_str(64)?;
    let format_epoch = dec.read_u32()?;
    let semantic_epoch = dec.read_u32()?;
    let platform = dec.read_str(32)?;
    let schema_sha256 = dec.read_str(64)?;
    let writer_identity = dec.read_opt_str(256)?;
    let root = dec.read_encoded_path()?;
    let configuration_file = dec.read_opt_encoded_path()?;
    let created_at = dec.read_time_value()?;
    let plan_input_digest = match dec.read_u8()? {
        0 => None,
        1 => Some(CachedPlanDigest {
            root_spelling: dec.read_encoded_path()?,
            identity_root: dec.read_encoded_path()?,
            sha256: dec.read_str(64)?,
        }),
        _ => return None,
    };

    let candidate_count = dec.read_count(MAX_CANDIDATES)?;
    let mut candidates = Vec::with_capacity(candidate_count);
    for _ in 0..candidate_count {
        candidates.push(dec.read_candidate()?);
    }

    let report_count = dec.read_count(MAX_REPORTS)?;
    let mut reports = Vec::with_capacity(report_count);
    for _ in 0..report_count {
        reports.push(dec.read_report()?);
    }

    if dec.remaining() != 0 {
        return None;
    }

    Some(Generation {
        magic,
        format_epoch,
        semantic_epoch,
        platform,
        schema_sha256,
        writer_identity,
        root,
        configuration_file,
        created_at,
        plan_input_digest,
        candidates,
        reports,
    })
}

struct Encoder<'a>(&'a mut Vec<u8>);

impl Encoder<'_> {
    fn write_u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn write_bool(&mut self, value: bool) {
        self.0.push(u8::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    #[cfg(unix)]
    fn write_i64(&mut self, value: i64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn write_varint(&mut self, mut value: u64) {
        while value >= 0x80 {
            let low = u8::try_from(value & 0x7f).unwrap_or(0);
            self.0.push(low | 0x80);
            value >>= 7;
        }
        self.0.push(u8::try_from(value).unwrap_or(0));
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_varint(bytes.len() as u64);
        self.0.extend_from_slice(bytes);
    }

    fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    fn write_opt_str(&mut self, value: Option<&str>) {
        match value {
            Some(text) => {
                self.write_u8(1);
                self.write_str(text);
            }
            None => self.write_u8(0),
        }
    }

    fn write_opt_usize(&mut self, value: Option<usize>) {
        match value {
            Some(v) => {
                self.write_u8(1);
                self.write_varint(v as u64);
            }
            None => self.write_u8(0),
        }
    }

    fn write_str_vec(&mut self, values: &[String]) {
        self.write_varint(values.len() as u64);
        for item in values {
            self.write_str(item);
        }
    }

    fn write_str_map(&mut self, values: &BTreeMap<String, String>) {
        self.write_varint(values.len() as u64);
        for (k, v) in values {
            self.write_str(k);
            self.write_str(v);
        }
    }

    fn write_encoded_path(&mut self, path: &EncodedPath) {
        self.write_u8(path.encoding);
        self.write_bytes(&path.bytes);
    }

    fn write_opt_encoded_path(&mut self, path: Option<&EncodedPath>) {
        match path {
            Some(path) => {
                self.write_u8(1);
                self.write_encoded_path(path);
            }
            None => self.write_u8(0),
        }
    }

    fn write_time_value(&mut self, value: TimeValue) {
        self.write_bool(value.before_epoch);
        self.write_varint(value.seconds);
        self.write_varint(u64::from(value.nanoseconds));
    }

    fn write_observation(&mut self, obs: &FileObservation) {
        self.write_varint(obs.length);
        match obs.modified {
            Some(time) => {
                self.write_u8(1);
                self.write_time_value(time);
            }
            None => self.write_u8(0),
        }
        self.write_u8(obs.file_type.to_tag());
        self.write_opt_encoded_path(obs.symlink_target.as_ref());
        #[cfg(unix)]
        {
            self.write_varint(obs.device);
            self.write_varint(obs.inode);
            self.write_varint(u64::from(obs.mode));
            self.write_i64(obs.change_seconds);
            self.write_i64(obs.change_nanoseconds);
        }
    }

    fn write_candidate(&mut self, candidate: &CachedCandidate, plan_tasks: &[LoadedTask]) -> bool {
        self.write_encoded_path(&candidate.path);
        self.0.extend_from_slice(&candidate.sha256);
        self.write_observation(&candidate.observation);
        match &candidate.artifact {
            CachedArtifact::NotTask => self.write_u8(0),
            CachedArtifact::Task(cached) => {
                self.write_u8(1);
                self.write_cached_task(cached);
            }
            CachedArtifact::TaskRef {
                semantic_sha256,
                task_index,
            } => {
                let Some(loaded) = plan_tasks.get(*task_index) else {
                    return false;
                };
                self.write_u8(1);
                self.write_loaded_task(*semantic_sha256, loaded);
            }
            CachedArtifact::Uncacheable => self.write_u8(2),
        }
        true
    }

    fn write_loaded_task(&mut self, semantic_sha256: [u8; 32], loaded: &LoadedTask) {
        self.0.extend_from_slice(&semantic_sha256);
        self.write_task(&loaded.task);
        self.write_varint(loaded.fields.len() as u64);
        for (name, loc) in &loaded.fields {
            self.write_str(name);
            self.write_varint(loc.line as u64);
            self.write_varint(loc.column as u64);
        }
        let c = &loaded.criteria;
        self.write_varint(c.total as u64);
        self.write_varint(c.ticked as u64);
        self.write_opt_usize(c.first_unticked);
        self.write_opt_str(c.scaffold_prompt);
        self.write_opt_usize(c.scaffold_ticked);
        self.write_opt_usize(c.scaffold_unticked);
        self.write_varint(c.prompts as u64);
        self.write_bool(c.why_untouched);
    }

    fn write_cached_task(&mut self, cached: &CachedTask) {
        self.0.extend_from_slice(&cached.semantic_sha256);
        self.write_task(&cached.task);
        self.write_varint(cached.fields.len() as u64);
        for (name, loc) in &cached.fields {
            self.write_str(name);
            self.write_varint(loc.line as u64);
            self.write_varint(loc.column as u64);
        }
        let c = &cached.criteria;
        self.write_varint(c.total as u64);
        self.write_varint(c.ticked as u64);
        self.write_opt_usize(c.first_unticked);
        self.write_opt_str(c.scaffold_prompt.as_deref());
        self.write_opt_usize(c.scaffold_ticked);
        self.write_opt_usize(c.scaffold_unticked);
        self.write_varint(c.prompts as u64);
        self.write_bool(c.why_untouched);
    }

    fn write_task(&mut self, task: &Task) {
        // Destructure exhaustively so adding any field to `Task` breaks
        // compilation until the binary cache codec explicitly handles it.
        let Task {
            id,
            title,
            phase,
            status,
            owner,
            effort_days,
            blocked_by,
            prefer_after,
            ratified_by,
            contains,
            integrates,
            evidence,
            paths,
            traces,
            trace_fingerprints,
            evidence_fingerprints,
            tags,
            revisit_at,
            version_impact,
            version_impact_reason,
            version,
            closed_at,
            closed_at_source,
        } = task;

        self.write_str(id);
        self.write_str(title);
        self.write_varint(u64::from(*phase));
        self.write_u8(status_to_tag(*status));
        self.write_opt_str(owner.as_deref());
        match effort_days {
            Some(days) => {
                self.write_u8(1);
                self.0.extend_from_slice(&days.to_le_bytes());
            }
            None => self.write_u8(0),
        }
        self.write_str_vec(blocked_by);
        self.write_str_vec(prefer_after);
        self.write_str_vec(ratified_by);
        self.write_str_vec(contains);
        self.write_str_vec(integrates);

        self.write_varint(evidence.len() as u64);
        for item in evidence {
            self.write_u8(evidence_kind_to_tag(item.kind));
            self.write_str(&item.reference);
            self.write_opt_str(item.captured.as_deref());
        }

        self.write_str_vec(paths);
        self.write_str_vec(traces);
        self.write_str_map(trace_fingerprints);
        self.write_str_map(evidence_fingerprints);
        self.write_str_vec(tags);
        self.write_opt_str(revisit_at.as_deref());
        match version_impact {
            Some(impact) => {
                self.write_u8(1);
                self.write_u8(version_impact_to_tag(*impact));
            }
            None => self.write_u8(0),
        }
        self.write_opt_str(version_impact_reason.as_deref());
        self.write_opt_str(version.as_deref());
        self.write_opt_str(closed_at.as_deref());
        match closed_at_source {
            Some(source) => {
                self.write_u8(1);
                self.write_u8(closed_at_source_to_tag(*source));
            }
            None => self.write_u8(0),
        }
    }

    fn write_report(&mut self, report: &CachedReport) -> bool {
        self.write_str(&report.key);
        self.write_varint(report.tasks_checked as u64);
        self.write_varint(report.diagnostics.len() as u64);
        for diag in &report.diagnostics {
            self.write_str(&diag.code);
            self.write_str(&diag.severity);
            self.write_opt_str(diag.task.as_deref());
            self.write_opt_encoded_path(diag.file.as_ref());
            self.write_str(&diag.message);
            self.write_opt_str(diag.agent_instruction.as_deref());
            match &diag.repair {
                Some(repair) => {
                    let Ok(repair_json) = serde_json::to_vec(repair) else {
                        return false;
                    };
                    self.write_u8(1);
                    self.write_bytes(&repair_json);
                }
                None => self.write_u8(0),
            }
            match diag.location {
                Some(loc) => {
                    self.write_u8(1);
                    self.write_varint(loc.line as u64);
                    self.write_varint(loc.column as u64);
                }
                None => self.write_u8(0),
            }
            self.write_str_vec(&diag.detail);
        }
        match &report.agent_contract_provenance {
            Some(prov) => {
                self.write_u8(1);
                self.write_encoded_path(&prov.file);
                self.write_opt_str(prov.written_by.as_deref());
                self.write_str(&prov.checked_by);
                self.write_str(&prov.note);
            }
            None => self.write_u8(0),
        }
        true
    }
}

struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    const fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_slice(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn read_u8(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(byte)
    }

    fn read_bool(&mut self) -> Option<bool> {
        match self.read_u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn read_u32(&mut self) -> Option<u32> {
        let bytes: [u8; 4] = self.read_slice(4)?.try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    #[cfg(unix)]
    fn read_i64(&mut self) -> Option<i64> {
        let bytes: [u8; 8] = self.read_slice(8)?.try_into().ok()?;
        Some(i64::from_le_bytes(bytes))
    }

    fn read_f64(&mut self) -> Option<f64> {
        let bytes: [u8; 8] = self.read_slice(8)?.try_into().ok()?;
        let value = f64::from_le_bytes(bytes);
        value.is_finite().then_some(value)
    }

    fn read_varint(&mut self) -> Option<u64> {
        let mut result = 0_u64;
        let mut shift = 0_u32;
        for _ in 0..10 {
            let byte = self.read_u8()?;
            let low = u64::from(byte & 0x7f);
            if shift == 63 && low > 1 {
                return None;
            }
            result |= low << shift;
            if (byte & 0x80) == 0 {
                return Some(result);
            }
            shift += 7;
        }
        None
    }

    fn read_usize(&mut self) -> Option<usize> {
        usize::try_from(self.read_varint()?).ok()
    }

    fn read_count(&mut self, max_count: usize) -> Option<usize> {
        let count = self.read_usize()?;
        if count > max_count || count > self.remaining() {
            return None;
        }
        Some(count)
    }

    fn read_bytes(&mut self, max_len: usize) -> Option<Vec<u8>> {
        let len = self.read_usize()?;
        if len > max_len || len > self.remaining() {
            return None;
        }
        Some(self.read_slice(len)?.to_vec())
    }

    fn read_str(&mut self, max_len: usize) -> Option<String> {
        let len = self.read_usize()?;
        if len > max_len || len > self.remaining() {
            return None;
        }
        let slice = self.read_slice(len)?;
        std::str::from_utf8(slice).ok().map(str::to_string)
    }

    #[allow(clippy::option_option)]
    fn read_opt_str(&mut self, max_len: usize) -> Option<Option<String>> {
        match self.read_u8()? {
            0 => Some(None),
            1 => Some(Some(self.read_str(max_len)?)),
            _ => None,
        }
    }

    #[allow(clippy::option_option)]
    fn read_opt_usize(&mut self) -> Option<Option<usize>> {
        match self.read_u8()? {
            0 => Some(None),
            1 => Some(Some(self.read_usize()?)),
            _ => None,
        }
    }

    fn read_str_vec(&mut self, max_count: usize, max_item_len: usize) -> Option<Vec<String>> {
        let count = self.read_count(max_count)?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(self.read_str(max_item_len)?);
        }
        Some(out)
    }

    fn read_str_map(
        &mut self,
        max_count: usize,
        max_item_len: usize,
    ) -> Option<BTreeMap<String, String>> {
        let count = self.read_count(max_count)?;
        let mut out = BTreeMap::new();
        for _ in 0..count {
            let k = self.read_str(max_item_len)?;
            let v = self.read_str(max_item_len)?;
            out.insert(k, v);
        }
        Some(out)
    }

    fn read_encoded_path(&mut self) -> Option<EncodedPath> {
        let encoding = self.read_u8()?;
        let bytes = self.read_bytes(MAX_PATH_BYTES)?;
        Some(EncodedPath { encoding, bytes })
    }

    #[allow(clippy::option_option)]
    fn read_opt_encoded_path(&mut self) -> Option<Option<EncodedPath>> {
        match self.read_u8()? {
            0 => Some(None),
            1 => Some(Some(self.read_encoded_path()?)),
            _ => None,
        }
    }

    fn read_time_value(&mut self) -> Option<TimeValue> {
        let before_epoch = self.read_bool()?;
        let seconds = self.read_varint()?;
        let nanoseconds = u32::try_from(self.read_varint()?).ok()?;
        Some(TimeValue {
            before_epoch,
            seconds,
            nanoseconds,
        })
    }

    fn read_observation(&mut self) -> Option<FileObservation> {
        let length = self.read_varint()?;
        let modified = match self.read_u8()? {
            0 => None,
            1 => Some(self.read_time_value()?),
            _ => return None,
        };
        let file_type = FileType::from_tag(self.read_u8()?)?;
        let symlink_target = self.read_opt_encoded_path()?;
        #[cfg(unix)]
        {
            let device = self.read_varint()?;
            let inode = self.read_varint()?;
            let mode = u32::try_from(self.read_varint()?).ok()?;
            let change_seconds = self.read_i64()?;
            let change_nanoseconds = self.read_i64()?;
            Some(FileObservation {
                length,
                modified,
                file_type,
                symlink_target,
                device,
                inode,
                mode,
                change_seconds,
                change_nanoseconds,
            })
        }
        #[cfg(not(unix))]
        {
            Some(FileObservation {
                length,
                modified,
                file_type,
                symlink_target,
            })
        }
    }

    fn read_candidate(&mut self) -> Option<CachedCandidate> {
        let path = self.read_encoded_path()?;
        let sha256: [u8; 32] = self.read_slice(32)?.try_into().ok()?;
        let observation = self.read_observation()?;
        let artifact = match self.read_u8()? {
            0 => CachedArtifact::NotTask,
            1 => CachedArtifact::Task(Arc::new(self.read_cached_task()?)),
            2 => CachedArtifact::Uncacheable,
            _ => return None,
        };
        Some(CachedCandidate {
            path,
            sha256,
            observation,
            artifact,
        })
    }

    fn read_cached_task(&mut self) -> Option<CachedTask> {
        let semantic_sha256: [u8; 32] = self.read_slice(32)?.try_into().ok()?;
        let task = self.read_task()?;
        let field_count = self.read_count(MAX_TASK_FIELDS)?;
        let mut fields = BTreeMap::new();
        for _ in 0..field_count {
            let name = self.read_str(256)?;
            let line = self.read_usize()?;
            let column = self.read_usize()?;
            fields.insert(name, Location { line, column });
        }
        let total = self.read_usize()?;
        let ticked = self.read_usize()?;
        let first_unticked = self.read_opt_usize()?;
        let scaffold_prompt = self.read_opt_str(MAX_STRING_BYTES)?;
        let scaffold_ticked = self.read_opt_usize()?;
        let scaffold_unticked = self.read_opt_usize()?;
        let prompts = self.read_usize()?;
        let why_untouched = self.read_bool()?;
        Some(CachedTask {
            semantic_sha256,
            task,
            fields,
            criteria: CachedCriteria {
                total,
                ticked,
                first_unticked,
                scaffold_prompt,
                scaffold_ticked,
                scaffold_unticked,
                prompts,
                why_untouched,
            },
        })
    }

    fn read_task(&mut self) -> Option<Task> {
        let id = self.read_str(MAX_STRING_BYTES)?;
        let title = self.read_str(MAX_STRING_BYTES)?;
        let phase = u32::try_from(self.read_varint()?).ok()?;
        let status = status_from_tag(self.read_u8()?)?;
        let owner = self.read_opt_str(MAX_STRING_BYTES)?;
        let effort_days = match self.read_u8()? {
            0 => None,
            1 => Some(self.read_f64()?),
            _ => return None,
        };
        let blocked_by = self.read_str_vec(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let prefer_after = self.read_str_vec(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let ratified_by = self.read_str_vec(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let contains = self.read_str_vec(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let integrates = self.read_str_vec(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;

        let evidence_count = self.read_count(MAX_TASK_LIST_ITEMS)?;
        let mut evidence = Vec::with_capacity(evidence_count);
        for _ in 0..evidence_count {
            let kind = evidence_kind_from_tag(self.read_u8()?)?;
            let reference = self.read_str(MAX_STRING_BYTES)?;
            let captured = self.read_opt_str(MAX_STRING_BYTES)?;
            evidence.push(Evidence {
                kind,
                reference,
                captured,
            });
        }

        let paths = self.read_str_vec(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let traces = self.read_str_vec(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let trace_fingerprints = self.read_str_map(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let evidence_fingerprints = self.read_str_map(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let tags = self.read_str_vec(MAX_TASK_LIST_ITEMS, MAX_STRING_BYTES)?;
        let revisit_at = self.read_opt_str(MAX_STRING_BYTES)?;
        let version_impact = match self.read_u8()? {
            0 => None,
            1 => Some(version_impact_from_tag(self.read_u8()?)?),
            _ => return None,
        };
        let version_impact_reason = self.read_opt_str(MAX_STRING_BYTES)?;
        let version = self.read_opt_str(MAX_STRING_BYTES)?;
        let closed_at = self.read_opt_str(MAX_STRING_BYTES)?;
        let closed_at_source = match self.read_u8()? {
            0 => None,
            1 => Some(closed_at_source_from_tag(self.read_u8()?)?),
            _ => return None,
        };

        Some(Task {
            id,
            title,
            phase,
            status,
            owner,
            effort_days,
            blocked_by,
            prefer_after,
            ratified_by,
            contains,
            integrates,
            evidence,
            paths,
            traces,
            trace_fingerprints,
            evidence_fingerprints,
            tags,
            revisit_at,
            version_impact,
            version_impact_reason,
            version,
            closed_at,
            closed_at_source,
        })
    }

    fn read_report(&mut self) -> Option<CachedReport> {
        let key = self.read_str(64)?;
        let tasks_checked = self.read_usize()?;
        let diag_count = self.read_count(MAX_REPORT_DIAGNOSTICS)?;
        let mut diagnostics = Vec::with_capacity(diag_count);
        let mut aggregate_text = 0_usize;
        for _ in 0..diag_count {
            let code = self.read_str(64)?;
            let severity = self.read_str(32)?;
            let task = self.read_opt_str(MAX_STRING_BYTES)?;
            let file = self.read_opt_encoded_path()?;
            let message = self.read_str(MAX_DIAGNOSTIC_TEXT_BYTES)?;
            let agent_instruction = self.read_opt_str(MAX_DIAGNOSTIC_TEXT_BYTES)?;
            let (repair, repair_bytes) = match self.read_u8()? {
                0 => (None, 0),
                1 => {
                    let raw = self.read_bytes(MAX_DIAGNOSTIC_TEXT_BYTES)?;
                    let len = raw.len();
                    let parsed = serde_json::from_slice(&raw).ok()?;
                    (Some(parsed), len)
                }
                _ => return None,
            };
            let location = match self.read_u8()? {
                0 => None,
                1 => Some(CachedLocation {
                    line: self.read_usize()?,
                    column: self.read_usize()?,
                }),
                _ => return None,
            };
            let detail = self.read_str_vec(4096, MAX_DIAGNOSTIC_TEXT_BYTES)?;
            let local = code
                .len()
                .saturating_add(severity.len())
                .saturating_add(task.as_ref().map_or(0, String::len))
                .saturating_add(message.len())
                .saturating_add(agent_instruction.as_ref().map_or(0, String::len))
                .saturating_add(detail.iter().map(String::len).sum::<usize>())
                .saturating_add(repair_bytes);
            if local > MAX_DIAGNOSTIC_TEXT_BYTES {
                return None;
            }
            aggregate_text = aggregate_text.saturating_add(local);
            if aggregate_text > MAX_REPORT_TEXT_BYTES {
                return None;
            }
            diagnostics.push(CachedDiagnostic {
                code,
                severity,
                task,
                file,
                message,
                agent_instruction,
                repair,
                location,
                detail,
            });
        }
        let agent_contract_provenance = match self.read_u8()? {
            0 => None,
            1 => {
                let file = self.read_encoded_path()?;
                let written_by = self.read_opt_str(MAX_DIAGNOSTIC_TEXT_BYTES)?;
                let checked_by = self.read_str(MAX_DIAGNOSTIC_TEXT_BYTES)?;
                let note = self.read_str(MAX_DIAGNOSTIC_TEXT_BYTES)?;
                Some(CachedAgentContractProvenance {
                    file,
                    written_by,
                    checked_by,
                    note,
                })
            }
            _ => return None,
        };
        Some(CachedReport {
            key,
            tasks_checked,
            diagnostics,
            agent_contract_provenance,
        })
    }
}

const fn status_to_tag(status: Status) -> u8 {
    match status {
        Status::Todo => 0,
        Status::InProgress => 1,
        Status::Blocked => 2,
        Status::Done => 3,
        Status::Dropped => 4,
    }
}

const fn status_from_tag(tag: u8) -> Option<Status> {
    match tag {
        0 => Some(Status::Todo),
        1 => Some(Status::InProgress),
        2 => Some(Status::Blocked),
        3 => Some(Status::Done),
        4 => Some(Status::Dropped),
        _ => None,
    }
}

const fn evidence_kind_to_tag(kind: EvidenceKind) -> u8 {
    match kind {
        EvidenceKind::Commit => 0,
        EvidenceKind::Url => 1,
        EvidenceKind::File => 2,
        EvidenceKind::Test => 3,
        EvidenceKind::Note => 4,
    }
}

const fn evidence_kind_from_tag(tag: u8) -> Option<EvidenceKind> {
    match tag {
        0 => Some(EvidenceKind::Commit),
        1 => Some(EvidenceKind::Url),
        2 => Some(EvidenceKind::File),
        3 => Some(EvidenceKind::Test),
        4 => Some(EvidenceKind::Note),
        _ => None,
    }
}

const fn version_impact_to_tag(impact: VersionImpact) -> u8 {
    match impact {
        VersionImpact::None => 0,
        VersionImpact::Fix => 1,
        VersionImpact::Feature => 2,
        VersionImpact::Breaking => 3,
    }
}

const fn version_impact_from_tag(tag: u8) -> Option<VersionImpact> {
    match tag {
        0 => Some(VersionImpact::None),
        1 => Some(VersionImpact::Fix),
        2 => Some(VersionImpact::Feature),
        3 => Some(VersionImpact::Breaking),
        _ => None,
    }
}

const fn closed_at_source_to_tag(source: ClosedAtSource) -> u8 {
    match source {
        ClosedAtSource::Local => 0,
        ClosedAtSource::Imported => 1,
    }
}

const fn closed_at_source_from_tag(tag: u8) -> Option<ClosedAtSource> {
    match tag {
        0 => Some(ClosedAtSource::Local),
        1 => Some(ClosedAtSource::Imported),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_rejects_truncation_corruption_trailing_bytes_and_invalid_tags() {
        assert!(decode_generation(&[]).is_none());
        assert!(decode_generation(&[0u8; HEADER_LEN - 1]).is_none());

        // Validly framed empty-ish payload fails on bad magic/epoch/checksum.
        let mut frame = vec![0u8; HEADER_LEN + 4];
        assert!(decode_generation(&frame).is_none());
        frame[..16].copy_from_slice(&MAGIC_BYTES);
        frame[16..20].copy_from_slice(&(FORMAT_EPOCH + 1).to_le_bytes());
        assert!(decode_generation(&frame).is_none());
        frame[16..20].copy_from_slice(&FORMAT_EPOCH.to_le_bytes());
        frame[20..24].copy_from_slice(&99_u32.to_le_bytes());
        assert!(decode_generation(&frame).is_none());

        // Hostile count larger than remaining payload fails without allocating.
        let mut hostile_payload = Vec::new();
        let mut enc = Encoder(&mut hostile_payload);
        enc.write_varint(100_000);
        let mut dec = Decoder {
            data: &hostile_payload,
            pos: 0,
        };
        assert!(dec.read_count(MAX_CANDIDATES).is_none());

        // Round-trip every enum tag and reject invalid tags.
        for status in Status::ALL {
            assert_eq!(status_from_tag(status_to_tag(*status)), Some(*status));
        }
        assert!(status_from_tag(99).is_none());

        for kind in EvidenceKind::ALL {
            assert_eq!(
                evidence_kind_from_tag(evidence_kind_to_tag(*kind)),
                Some(*kind)
            );
        }
        assert!(evidence_kind_from_tag(99).is_none());

        for impact in [
            VersionImpact::None,
            VersionImpact::Fix,
            VersionImpact::Feature,
            VersionImpact::Breaking,
        ] {
            assert_eq!(
                version_impact_from_tag(version_impact_to_tag(impact)),
                Some(impact)
            );
        }
        assert!(version_impact_from_tag(99).is_none());

        for source in ClosedAtSource::ALL {
            assert_eq!(
                closed_at_source_from_tag(closed_at_source_to_tag(*source)),
                Some(*source)
            );
        }
        assert!(closed_at_source_from_tag(99).is_none());
    }

    fn sample_observation(file_type: FileType, with_modified: bool) -> FileObservation {
        FileObservation {
            length: 512,
            modified: with_modified.then_some(TimeValue {
                before_epoch: true,
                seconds: 123,
                nanoseconds: 456,
            }),
            file_type,
            symlink_target: (!with_modified)
                .then(|| EncodedPath::from_path(std::path::Path::new("target.md"))),
            #[cfg(unix)]
            device: 7,
            #[cfg(unix)]
            inode: 42,
            #[cfg(unix)]
            mode: 0o100_644,
            #[cfg(unix)]
            change_seconds: 1_700_000_000,
            #[cfg(unix)]
            change_nanoseconds: 999,
        }
    }

    fn sample_task(id: &str, with_effort: bool) -> Task {
        let mut trace_fingerprints = BTreeMap::new();
        trace_fingerprints.insert("DESIGN.md#s1".to_string(), "deadbeef".to_string());
        let mut evidence_fingerprints = BTreeMap::new();
        evidence_fingerprints.insert("file:src/lib.rs".to_string(), "cafebabe".to_string());
        Task {
            id: id.to_string(),
            title: format!("Task {id}"),
            phase: 2,
            status: Status::Done,
            owner: Some("alice".to_string()),
            effort_days: with_effort.then_some(2.5),
            blocked_by: vec!["A-0".to_string()],
            prefer_after: vec!["A-1".to_string()],
            ratified_by: vec!["ADR-0001".to_string()],
            contains: vec![],
            integrates: vec!["T-000".to_string()],
            evidence: vec![Evidence {
                kind: EvidenceKind::Test,
                reference: "suite::ok".to_string(),
                captured: Some("2026-01-01".to_string()),
            }],
            paths: vec!["src/lib.rs".to_string()],
            traces: vec!["DESIGN.md#s1".to_string()],
            trace_fingerprints,
            evidence_fingerprints,
            tags: vec!["audit".to_string()],
            revisit_at: Some("2026-06-01".to_string()),
            version_impact: Some(VersionImpact::Feature),
            version_impact_reason: Some("adds codec".to_string()),
            version: Some("0.8.0".to_string()),
            closed_at: Some("2026-01-02T03:04:05Z".to_string()),
            closed_at_source: Some(ClosedAtSource::Local),
        }
    }

    fn sample_report() -> CachedReport {
        CachedReport {
            key: "a".repeat(64),
            tasks_checked: 2,
            diagnostics: vec![
                CachedDiagnostic {
                    code: "APLAN001".to_string(),
                    severity: "error".to_string(),
                    task: Some("T-1".to_string()),
                    file: Some(EncodedPath::from_path(std::path::Path::new("tasks/T-1.md"))),
                    message: "duplicate".to_string(),
                    agent_instruction: Some("fix it".to_string()),
                    repair: Some(aplan_core::Repair::ReplaceEvidence {
                        from: Evidence {
                            kind: EvidenceKind::File,
                            reference: "old.rs".to_string(),
                            captured: None,
                        },
                        to: Evidence {
                            kind: EvidenceKind::File,
                            reference: "new.rs".to_string(),
                            captured: Some("2026-01-01".to_string()),
                        },
                    }),
                    location: Some(CachedLocation { line: 4, column: 2 }),
                    detail: vec!["extra detail".to_string()],
                },
                CachedDiagnostic {
                    code: "APLAN009".to_string(),
                    severity: "warning".to_string(),
                    task: None,
                    file: None,
                    message: "missing effort".to_string(),
                    agent_instruction: None,
                    repair: None,
                    location: None,
                    detail: vec![],
                },
            ],
            agent_contract_provenance: Some(CachedAgentContractProvenance {
                file: EncodedPath::from_path(std::path::Path::new("AGENTS.md")),
                written_by: Some("aplan 0.8.0-dev".to_string()),
                checked_by: "aplan 0.8.0-dev".to_string(),
                note: "matches policy".to_string(),
            }),
        }
    }

    fn sample_generation_and_tasks() -> (Generation, Vec<LoadedTask>) {
        let mut fields = BTreeMap::new();
        fields.insert("id".to_string(), Location { line: 2, column: 1 });
        let loaded_task = LoadedTask {
            task: sample_task("T-2", false),
            path: std::path::PathBuf::from("tasks/T-2.md"),
            fields: fields.clone(),
            criteria: aplan_core::load::Criteria {
                total: 2,
                ticked: 1,
                first_unticked: Some(12),
                scaffold_prompt: None,
                scaffold_ticked: Some(10),
                scaffold_unticked: Some(12),
                prompts: 1,
                why_untouched: true,
            },
        };
        let cached_task = CachedTask {
            semantic_sha256: [0x11; 32],
            task: sample_task("T-1", true),
            fields,
            criteria: CachedCriteria {
                total: 3,
                ticked: 2,
                first_unticked: Some(8),
                scaffold_prompt: Some("prompt".to_string()),
                scaffold_ticked: Some(6),
                scaffold_unticked: Some(8),
                prompts: 2,
                why_untouched: true,
            },
        };
        let generation = Generation {
            magic: "aplan-plan-v1".to_string(),
            format_epoch: FORMAT_EPOCH,
            semantic_epoch: 1,
            platform: "linux".to_string(),
            schema_sha256: "b".repeat(64),
            writer_identity: Some("writer".to_string()),
            root: EncodedPath::from_path(std::path::Path::new("/repo/aplan")),
            configuration_file: Some(EncodedPath::from_path(std::path::Path::new(
                "/repo/.aplan.yml",
            ))),
            created_at: TimeValue {
                before_epoch: false,
                seconds: 1_700_000_100,
                nanoseconds: 123,
            },
            plan_input_digest: Some(CachedPlanDigest {
                root_spelling: EncodedPath::from_path(std::path::Path::new("aplan")),
                identity_root: EncodedPath::from_path(std::path::Path::new("/repo/aplan")),
                sha256: "c".repeat(64),
            }),
            candidates: vec![
                CachedCandidate {
                    path: EncodedPath::from_path(std::path::Path::new("notes.md")),
                    sha256: [0x01; 32],
                    observation: sample_observation(FileType::Other, false),
                    artifact: CachedArtifact::NotTask,
                },
                CachedCandidate {
                    path: EncodedPath::from_path(std::path::Path::new("tasks/T-1.md")),
                    sha256: [0x02; 32],
                    observation: sample_observation(FileType::File, true),
                    artifact: CachedArtifact::Task(Arc::new(cached_task)),
                },
                CachedCandidate {
                    path: EncodedPath::from_path(std::path::Path::new("tasks/T-2.md")),
                    sha256: [0x03; 32],
                    observation: sample_observation(FileType::Directory, true),
                    artifact: CachedArtifact::TaskRef {
                        semantic_sha256: [0x22; 32],
                        task_index: 0,
                    },
                },
                CachedCandidate {
                    path: EncodedPath::from_path(std::path::Path::new("broken.md")),
                    sha256: [0x04; 32],
                    observation: sample_observation(FileType::File, true),
                    artifact: CachedArtifact::Uncacheable,
                },
            ],
            reports: vec![sample_report()],
        };
        (generation, vec![loaded_task])
    }

    fn frame_payload(payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
        frame.extend_from_slice(&MAGIC_BYTES);
        frame.extend_from_slice(&FORMAT_EPOCH.to_le_bytes());
        frame.extend_from_slice(&(u32::try_from(payload.len()).expect("len")).to_le_bytes());
        frame.extend_from_slice(blake3::hash(payload).as_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn codec_round_trips_full_generation_with_all_variants_reports_and_provenance() {
        let (generation, tasks) = sample_generation_and_tasks();
        let bytes = encode_generation(&generation, &tasks).expect("encode generation");
        let decoded = decode_generation(&bytes).expect("decode generation");
        assert_eq!(decoded.candidates.len(), 4);
        assert_eq!(decoded.reports.len(), 1);
        assert_eq!(decoded.reports[0].diagnostics.len(), 2);
        assert!(decoded.reports[0].diagnostics[0].repair.is_some());
        assert!(decoded.reports[0].diagnostics[0].location.is_some());
        assert!(decoded.reports[0].agent_contract_provenance.is_some());
        assert!(decoded.plan_input_digest.is_some());

        // Out-of-bounds TaskRef fails encoding cleanly instead of panicking.
        assert!(encode_generation(&generation, &[]).is_none());

        // Checksum mismatch and trailing payload byte are both rejected.
        let mut bad_checksum = bytes.clone();
        bad_checksum[HEADER_LEN] ^= 0xff;
        assert!(decode_generation(&bad_checksum).is_none());

        let mut trailing_payload = bytes[HEADER_LEN..].to_vec();
        trailing_payload.push(0x00);
        assert!(decode_generation(&frame_payload(&trailing_payload)).is_none());
    }

    #[test]
    fn codec_rejects_invalid_primitives_and_tagged_record_errors() {
        let mut d = Decoder { data: &[2], pos: 0 };
        assert!(d.read_bool().is_none());
        let mut d = Decoder {
            data: &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02],
            pos: 0,
        };
        assert!(d.read_varint().is_none());
        let mut d = Decoder {
            data: &[0x80; 10],
            pos: 0,
        };
        assert!(d.read_varint().is_none());

        let mut buf = Vec::new();
        Encoder(&mut buf).write_varint(99);
        assert!(Decoder { data: &buf, pos: 0 }.read_bytes(10).is_none());
        assert!(Decoder { data: &buf, pos: 0 }.read_str(10).is_none());
        assert!(Decoder { data: &[2], pos: 0 }.read_opt_str(10).is_none());
        assert!(Decoder { data: &[2], pos: 0 }.read_opt_usize().is_none());
        assert!(
            Decoder { data: &[2], pos: 0 }
                .read_opt_encoded_path()
                .is_none()
        );

        // Invalid `modified` tag in observation and invalid artifact tag in candidate.
        let mut bad_obs = Vec::new();
        {
            let mut enc = Encoder(&mut bad_obs);
            enc.write_varint(0);
            enc.write_u8(2);
        }
        assert!(
            Decoder {
                data: &bad_obs,
                pos: 0
            }
            .read_observation()
            .is_none()
        );

        let mut bad_cand = Vec::new();
        {
            let mut enc = Encoder(&mut bad_cand);
            enc.write_encoded_path(&EncodedPath::from_path(std::path::Path::new("a.md")));
            bad_cand.extend_from_slice(&[0u8; 32]);
            let mut enc = Encoder(&mut bad_cand);
            enc.write_observation(&sample_observation(FileType::File, false));
            enc.write_u8(9);
        }
        assert!(
            Decoder {
                data: &bad_cand,
                pos: 0
            }
            .read_candidate()
            .is_none()
        );
    }

    #[test]
    fn codec_rejects_invalid_generation_task_and_report_tags_and_text_overflows() {
        // Invalid plan_input_digest tag in full generation payload.
        let mut bad_gen = Vec::new();
        {
            let mut enc = Encoder(&mut bad_gen);
            enc.write_str("m");
            enc.write_u32(FORMAT_EPOCH);
            enc.write_u32(1);
            enc.write_str("linux");
            enc.write_str("s");
            enc.write_opt_str(None);
            enc.write_encoded_path(&EncodedPath::from_path(std::path::Path::new("r")));
            enc.write_opt_encoded_path(None);
            enc.write_time_value(TimeValue {
                before_epoch: false,
                seconds: 1,
                nanoseconds: 0,
            });
            enc.write_u8(2);
        }
        assert!(decode_generation(&frame_payload(&bad_gen)).is_none());

        // Invalid tags inside Task: effort_days (tag 2), version_impact (tag 2), closed_at_source (tag 2).
        for bad_slot in 0..3 {
            let mut buf = Vec::new();
            let mut enc = Encoder(&mut buf);
            enc.write_str("T-1");
            enc.write_str("Title");
            enc.write_varint(1);
            enc.write_u8(0);
            enc.write_opt_str(None);
            enc.write_u8(if bad_slot == 0 { 2 } else { 0 });
            for _ in 0..7 {
                enc.write_varint(0);
            }
            enc.write_varint(0);
            enc.write_varint(0);
            enc.write_varint(0);
            enc.write_opt_str(None);
            enc.write_u8(if bad_slot == 1 { 2 } else { 0 });
            enc.write_opt_str(None);
            enc.write_opt_str(None);
            enc.write_opt_str(None);
            enc.write_u8(if bad_slot == 2 { 2 } else { 0 });
            assert!(Decoder { data: &buf, pos: 0 }.read_task().is_none());
        }

        // Invalid repair/location/provenance tags and diagnostic text budget overflow in CachedReport.
        let make_report_buf =
            |repair_tag: u8, loc_tag: u8, prov_tag: u8, diags: usize, msg: &str| {
                let mut buf = Vec::new();
                let mut enc = Encoder(&mut buf);
                enc.write_str(&"a".repeat(64));
                enc.write_varint(1);
                enc.write_varint(diags as u64);
                for _ in 0..diags {
                    enc.write_str("APLAN001");
                    enc.write_str("error");
                    enc.write_opt_str(None);
                    enc.write_opt_encoded_path(None);
                    enc.write_str(msg);
                    enc.write_opt_str(None);
                    enc.write_u8(repair_tag);
                    enc.write_u8(loc_tag);
                    enc.write_varint(0);
                }
                enc.write_u8(prov_tag);
                buf
            };
        for buf in [
            make_report_buf(2, 0, 0, 1, "m"),
            make_report_buf(0, 2, 0, 1, "m"),
            make_report_buf(0, 0, 2, 1, "m"),
            make_report_buf(0, 0, 0, 1, &"x".repeat(MAX_DIAGNOSTIC_TEXT_BYTES)),
            make_report_buf(
                0,
                0,
                0,
                (MAX_REPORT_TEXT_BYTES / 10_000) + 2,
                &"y".repeat(10_000),
            ),
        ] {
            assert!(Decoder { data: &buf, pos: 0 }.read_report().is_none());
        }
    }
}
