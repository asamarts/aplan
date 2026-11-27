//! Bounded-memory output streaming, atomic-error preflight, and read-surface
//! output-disposition vocabulary (`A2-136`).
//!
//! Ordinary bounded responses complete inside the 100 ms read latency budget
//! without streaming overhead. Unscoped tables, graphs, roadmaps, and large
//! diagnostic reports can grow to megabytes on a 10,000-task corpus; buffering
//! the whole rendering before writing byte 0 delays first bytes behind output
//! volume and allocates memory proportional to the response.
//!
//! [`BOUNDED_RESPONSE_THRESHOLD_BYTES`] (256 KiB) is the empirically measured
//! boundary between bounded reads and output-dominated reads across terminal
//! buffers (4 KiB PTY), pipe buffers (64 KiB Linux default; 4x pipe capacity =
//! 256 KiB), MCP protocol caps (64 KiB resources, 512 KiB tool envelopes), and
//! the 206 / 1,000 / 10,000-task corpora (see
//! `docs/design/read-path-performance.md`).
//!
//! [`StreamingWriter`] wraps [`crate::write::RetryingWriter`] in a fixed
//! [`STREAM_BUFFER_CAPACITY`] (16 KiB) [`std::io::BufWriter`]:
//! - Peak output-layer buffering stays O(1) (`<= 16 KiB`) regardless of output
//!   size.
//! - Tiny writes (such as pretty-printed JSON tokens or table cells) coalesce
//!   into 16 KiB writes instead of millions of per-token syscalls.
//! - First bytes flush after at most 16 KiB so downstream consumers and pipes
//!   observe prompt TTFB and can terminate production immediately on
//!   [`std::io::ErrorKind::BrokenPipe`].
//! - Machine formats run a deterministic preflight before writing byte 0 so a
//!   late data error never publishes a plausible partial document.

use std::io::{self, BufWriter, ErrorKind, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use crate::write::RetryingWriter;

/// Measured boundary (256 KiB) separating bounded responses from
/// output-dominated responses (`A2-136`).
///
/// Derived from measured pipe/terminal transfer latency, peak buffering, and
/// MCP framing bounds documented in `docs/design/read-path-performance.md`.
pub const BOUNDED_RESPONSE_THRESHOLD_BYTES: usize = 256 * 1024;

/// Fixed per-stream write buffer capacity (16 KiB = 4 memory pages, one quarter
/// of the default 64 KiB Linux pipe buffer).
///
/// Keeps output buffering O(1) while coalescing fine-grained formatter and JSON
/// serializer writes and delivering prompt first bytes.
pub const STREAM_BUFFER_CAPACITY: usize = 16 * 1024;

static OUTPUT_BYTES_WRITTEN: AtomicU64 = AtomicU64::new(0);
static PEAK_OUTPUT_BUFFER_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Process-wide deterministic output-buffering work counters consumed by
/// `--timings`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputWork {
    pub bytes_written: u64,
    pub peak_buffer_bytes: usize,
}

/// Read the current process-wide output counters.
#[must_use]
pub fn output_work() -> OutputWork {
    OutputWork {
        bytes_written: OUTPUT_BYTES_WRITTEN.load(Ordering::Relaxed),
        peak_buffer_bytes: PEAK_OUTPUT_BUFFER_BYTES.load(Ordering::Relaxed),
    }
}

/// Reset the process-wide peak output-buffer watermark at the start of a timed
/// command run.
pub fn reset_peak_output_buffer() {
    PEAK_OUTPUT_BUFFER_BYTES.store(0, Ordering::Relaxed);
}

/// Record bytes actually written directly to stdout by a buffered writer
/// (`crate::write::stdout` / `crate::write::anstream_stdout`) and update peak
/// buffered output bytes (`A2-136`).
pub(crate) fn record_direct_stdout_write(bytes: usize, writer_total: usize) {
    OUTPUT_BYTES_WRITTEN.fetch_add(bytes as u64, Ordering::Relaxed);
    update_global_peak(writer_total);
}

/// Record a whole-document buffered write so `--timings` reports honest peak
/// buffering for buffered surfaces alongside streamed ones.
pub fn record_buffered_output(bytes: usize) {
    OUTPUT_BYTES_WRITTEN.fetch_add(bytes as u64, Ordering::Relaxed);
    update_global_peak(bytes);
}

fn update_global_peak(candidate: usize) {
    let mut current = PEAK_OUTPUT_BUFFER_BYTES.load(Ordering::Relaxed);
    while candidate > current {
        match PEAK_OUTPUT_BUFFER_BYTES.compare_exchange_weak(
            current,
            candidate,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

/// Local per-writer telemetry returned by [`StreamingWriter::finish`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamStats {
    /// Total payload bytes written through the writer.
    pub bytes_written: u64,
    /// Peak bytes buffered inside this writer at any moment (`<= capacity`).
    pub peak_buffer_bytes: usize,
    /// Number of explicit flush calls completed.
    pub flushes: usize,
    /// Fixed capacity configured for this writer.
    pub capacity: usize,
}

/// Bounded-memory [`Write`] adapter combining [`BufWriter`] coalescing with
/// [`RetryingWriter`] non-blocking `WouldBlock` recovery.
#[derive(Debug)]
pub struct StreamingWriter<W: Write> {
    inner: BufWriter<RetryingWriter<W>>,
    capacity: usize,
    bytes_written: u64,
    peak_buffer_bytes: usize,
    flushes: usize,
}

impl<W: Write> StreamingWriter<W> {
    /// Wrap `inner` with [`STREAM_BUFFER_CAPACITY`] and default `WouldBlock`
    /// retry budget.
    pub fn new(inner: W) -> Self {
        Self::with_capacity(inner, STREAM_BUFFER_CAPACITY)
    }

    /// Wrap `inner` with an explicit buffer capacity (clamped to at least 1
    /// byte so buffering invariants remain well-defined).
    pub fn with_capacity(inner: W, capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            inner: BufWriter::with_capacity(cap, RetryingWriter::new(inner)),
            capacity: cap,
            bytes_written: 0,
            peak_buffer_bytes: 0,
            flushes: 0,
        }
    }

    /// Wrap `inner` with explicit buffer capacity and `WouldBlock` retry
    /// budget.
    pub fn with_capacity_and_budget(inner: W, capacity: usize, max_wait: Duration) -> Self {
        let cap = capacity.max(1);
        Self {
            inner: BufWriter::with_capacity(cap, RetryingWriter::with_budget(inner, max_wait)),
            capacity: cap,
            bytes_written: 0,
            peak_buffer_bytes: 0,
            flushes: 0,
        }
    }

    /// Flush all buffered bytes to the underlying sink, propagate any I/O
    /// error (including `BrokenPipe`), and return local stream statistics.
    ///
    /// # Errors
    ///
    /// Propagates any write or flush error from the underlying sink.
    pub fn finish(mut self) -> io::Result<StreamStats> {
        self.flush()?;
        Ok(self.stats())
    }

    /// Flush and recover the underlying sink alongside local stream statistics.
    ///
    /// # Errors
    ///
    /// Propagates any write or flush error from the underlying sink.
    pub fn into_inner_with_stats(mut self) -> io::Result<(W, StreamStats)> {
        self.flush()?;
        let stats = self.stats();
        let retrying = self
            .inner
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        Ok((retrying.into_inner(), stats))
    }

    /// Snapshot current local statistics without flushing.
    #[must_use]
    pub const fn stats(&self) -> StreamStats {
        StreamStats {
            bytes_written: self.bytes_written,
            peak_buffer_bytes: self.peak_buffer_bytes,
            flushes: self.flushes,
            capacity: self.capacity,
        }
    }

    fn observe_buffered(&mut self, incoming: usize) {
        let current = self.inner.buffer().len();
        // A write larger than `capacity` bypasses the internal buffer in
        // `BufWriter`, so the peak resident buffer stays bounded by `capacity`.
        let observed = current.max(incoming.min(self.capacity)).min(self.capacity);
        if observed > self.peak_buffer_bytes {
            self.peak_buffer_bytes = observed;
        }
        update_global_peak(observed);
    }
}

impl<W: Write> Write for StreamingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.observe_buffered(buf.len());
        let n = self.inner.write(buf)?;
        self.bytes_written = self.bytes_written.saturating_add(n as u64);
        OUTPUT_BYTES_WRITTEN.fetch_add(n as u64, Ordering::Relaxed);
        let post = self.inner.buffer().len().min(self.capacity);
        if post > self.peak_buffer_bytes {
            self.peak_buffer_bytes = post;
        }
        update_global_peak(post);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()?;
        self.flushes += 1;
        Ok(())
    }
}

/// Lock `std::io::stdout()` behind a bounded [`StreamingWriter`].
pub fn stdout() -> StreamingWriter<io::StdoutLock<'static>> {
    StreamingWriter::new(io::stdout().lock())
}

/// ANSI-aware streaming stdout writer that strips ANSI escapes **before**
/// counting and buffering when color is disabled, while preserving native
/// `anstream` terminal handling when color is enabled (`A2-136`).
#[derive(Debug)]
pub struct StyledStreamingWriter {
    inner: StyledStreamingInner,
}

#[derive(Debug)]
enum StyledStreamingInner {
    Strip(crate::write::StrippingWriter<StreamingWriter<io::StdoutLock<'static>>>),
    Styled(StreamingWriter<anstream::AutoStream<io::StdoutLock<'static>>>),
}

impl StyledStreamingWriter {
    /// Flush both the ANSI adapter and bounded buffer and return stream stats.
    ///
    /// # Errors
    ///
    /// Propagates any write or flush error from stdout (including `BrokenPipe`).
    pub fn finish(self) -> io::Result<StreamStats> {
        match self.inner {
            StyledStreamingInner::Strip(mut strip) => {
                strip.flush()?;
                strip.into_inner().finish()
            }
            StyledStreamingInner::Styled(styled) => styled.finish(),
        }
    }
}

impl Write for StyledStreamingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            StyledStreamingInner::Strip(strip) => strip.write(buf),
            StyledStreamingInner::Styled(styled) => styled.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.inner {
            StyledStreamingInner::Strip(strip) => strip.flush(),
            StyledStreamingInner::Styled(styled) => styled.flush(),
        }
    }
}

/// Lock `stdout()` behind an ANSI-aware bounded [`StyledStreamingWriter`].
pub fn anstream_stdout() -> StyledStreamingWriter {
    let choice = anstream::AutoStream::choice(&io::stdout());
    let inner = if choice == anstream::ColorChoice::Never {
        StyledStreamingInner::Strip(crate::write::StrippingWriter::new(StreamingWriter::new(
            io::stdout().lock(),
        )))
    } else {
        StyledStreamingInner::Styled(StreamingWriter::new(anstream::stdout().lock()))
    };
    StyledStreamingWriter { inner }
}

/// Verify that `value` is finite before streaming a machine document.
///
/// # Errors
///
/// Returns an explanatory message if `value` is `NaN` or infinite.
pub fn check_finite_f64(value: f64, label: &str) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("cannot serialize non-finite float for `{label}`"))
    }
}

/// Verify that an optional float is finite when present before streaming a
/// machine document.
///
/// # Errors
///
/// Returns an explanatory message if `value` contains `NaN` or an infinity.
pub fn check_finite_opt_f64(value: Option<f64>, label: &str) -> Result<(), String> {
    if let Some(v) = value {
        check_finite_f64(v, label)?;
    }
    Ok(())
}

/// Preflight `value` before emitting byte 0, then stream pretty-printed JSON
/// plus a trailing newline through a bounded writer.
///
/// A preflight refusal returns [`ErrorKind::InvalidData`] with zero bytes
/// written so downstream consumers never see a truncated or invalid document.
///
/// # Errors
///
/// Returns [`ErrorKind::InvalidData`] on preflight failure, or propagates any
/// serialization or downstream I/O error (including `BrokenPipe`).
pub fn stream_json_pretty<T, W>(
    writer: &mut W,
    value: &T,
    preflight: impl FnOnce(&T) -> Result<(), String>,
) -> io::Result<()>
where
    T: serde::Serialize,
    W: Write,
{
    if let Err(reason) = preflight(value) {
        return Err(io::Error::new(ErrorKind::InvalidData, reason));
    }
    serde_json::to_writer_pretty(&mut *writer, value).map_err(|e| json_to_io_error(&e))?;
    writeln!(writer)?;
    writer.flush()
}

/// Map a [`serde_json::Error`] back to its underlying [`io::ErrorKind`] when
/// the failure originated in the downstream writer (so `BrokenPipe` stays
/// `ErrorKind::BrokenPipe` for clean pipe-closure exit semantics).
#[must_use]
pub fn json_to_io_error(error: &serde_json::Error) -> io::Error {
    if let Some(kind) = error.io_error_kind() {
        io::Error::new(kind, error.to_string())
    } else {
        io::Error::new(ErrorKind::InvalidData, error.to_string())
    }
}

/// Zero-allocation [`serde::Serialize`] sequence adapter that streams items
/// from an iterator factory directly into a serializer without materializing
/// an intermediate `Vec<serde_json::Value>`.
pub struct SeqIter<F>(pub F);

impl<F> std::fmt::Debug for SeqIter<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SeqIter(..)")
    }
}

impl<F, I, T> serde::Serialize for SeqIter<F>
where
    F: Fn() -> I,
    I: IntoIterator<Item = T>,
    T: serde::Serialize,
{
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq((self.0)())
    }
}

/// Output-volume disposition assigned to every read surface in the derived
/// inventory (`A2-136`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputDisposition {
    /// Complete document assembled in memory before writing (inherently bounded
    /// by metadata/single-task scope or atomic whole-document contract).
    Buffered,
    /// Emitted incrementally through a bounded-memory [`StreamingWriter`]
    /// (`<= 16 KiB` buffer) after any fallible preflight succeeds.
    Streamed,
    /// Bounded per-request page with explicit item limit and snapshot-bound
    /// continuation cursor.
    Paginated,
    /// Bounded by an explicit hard byte, item, or canvas-cell ceiling that
    /// truncates or refuses deterministically rather than growing unboundedly.
    ExplicitlyCapped,
}

impl OutputDisposition {
    /// Stable lowercase token used in docs, diagnostics, and audit tables.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buffered => "buffered",
            Self::Streamed => "streamed",
            Self::Paginated => "paginated",
            Self::ExplicitlyCapped => "explicitly_capped",
        }
    }
}

/// Read latency and work-budget class assigned to every read surface in the
/// derived inventory (`A2-134`, `docs/design/read-path-performance.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadLatencyClass {
    /// Single-task or fixed-cardinality local plan read (sub-100 ms p95).
    LocalBoundedRead,
    /// Whole-plan structural validation or configured-document check
    /// (sub-100 ms p95 on exact index hit and one ordinary edit).
    FullStructuralRead,
    /// Task inspection combining structural state with stored evidence
    /// observations (sub-100 ms p95; zero configured provider spawns).
    EvidenceBackedInspection,
    /// Read consulting Git repository history or worktree metadata (plan
    /// overhead below 100 ms; Git work reported separately).
    RepositoryRead,
    /// Explicit external provider execution (excluded from the 100 ms
    /// completion SLO; bounded, cancellable, observable and reusable).
    ExternalRefresh,
    /// Potentially large unscoped table, graph, roadmap or collection response
    /// (first bounded response below 100 ms, then O(1)-buffered streaming,
    /// pagination or explicit cap).
    OutputDominatedRead,
}

impl ReadLatencyClass {
    /// Every latency class in canonical documentation order.
    pub const ALL: &'static [Self] = &[
        Self::LocalBoundedRead,
        Self::FullStructuralRead,
        Self::EvidenceBackedInspection,
        Self::RepositoryRead,
        Self::ExternalRefresh,
        Self::OutputDominatedRead,
    ];

    /// Stable lowercase token used in docs, diagnostics, and audit tables.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalBoundedRead => "local_bounded_read",
            Self::FullStructuralRead => "full_structural_read",
            Self::EvidenceBackedInspection => "evidence_backed_inspection",
            Self::RepositoryRead => "repository_read",
            Self::ExternalRefresh => "external_refresh",
            Self::OutputDominatedRead => "output_dominated_read",
        }
    }
}

/// Transport channel owning a read surface in the derived inventory (`A2-136`,
/// `A2-134`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadChannel {
    Cli,
    McpTool,
    McpResource,
    TuiOperation,
}

/// One entry in the exhaustive derived read-surface latency and
/// output-disposition inventory (`A2-136`, `A2-134`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReadSurfaceEntry {
    /// Canonical stable identifier (`cli:<cmd>:<format>`, `mcp:tool:<name>`,
    /// `mcp:resource:<uri>`, `tui:<kind>:<name>`).
    pub surface: String,
    /// Transport channel.
    pub channel: ReadChannel,
    /// CLI command name, MCP tool name, MCP resource URI/template, or TUI
    /// read operation name.
    pub target: String,
    /// Output format or protocol representation.
    pub format: String,
    /// Whether the underlying projection can grow linearly with plan, history,
    /// or repository size when unscoped.
    pub potentially_unbounded: bool,
    /// Assigned read latency and work-budget class (`A2-134`).
    pub latency_class: ReadLatencyClass,
    /// Assigned output-volume disposition.
    pub disposition: OutputDisposition,
    /// How atomic error semantics are preserved before emitting output bytes.
    pub atomic_error_boundary: &'static str,
    /// Concise architectural rationale for the chosen disposition.
    pub rationale: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_writer_bounds_peak_buffer_and_preserves_exact_bytes() {
        let mut sink = Vec::new();
        let mut writer = StreamingWriter::with_capacity(&mut sink, 64);
        assert_eq!(writer.write(&[]).expect("empty write"), 0);
        for i in 0..500 {
            writeln!(writer, "row-{i:04}: deterministic streaming line payload").expect("write");
        }
        let stats = writer.finish().expect("finish");
        assert_eq!(stats.capacity, 64);
        assert!(
            stats.peak_buffer_bytes <= 64,
            "peak buffer {} exceeded capacity 64",
            stats.peak_buffer_bytes
        );
        assert_eq!(stats.bytes_written, sink.len() as u64);
        assert!(stats.flushes >= 1);
        assert!(sink.starts_with(b"row-0000:"));
        assert!(sink.ends_with(b"row-0499: deterministic streaming line payload\n"));

        let work = output_work();
        assert!(work.bytes_written >= stats.bytes_written);
        assert!(format!("{:?}", SeqIter(|| [1u8, 2u8])).contains("SeqIter"));
    }

    #[test]
    fn streaming_writer_into_inner_and_budget_propagate_broken_pipe() {
        struct FailAfter {
            remaining: usize,
            written: Vec<u8>,
        }
        impl Write for FailAfter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                if self.remaining == 0 {
                    return Err(io::Error::new(ErrorKind::BrokenPipe, "closed"));
                }
                let take = buf.len().min(self.remaining);
                self.written.extend_from_slice(&buf[..take]);
                self.remaining -= take;
                Ok(take)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut ok_writer =
            StreamingWriter::with_capacity_and_budget(Vec::new(), 32, Duration::from_millis(50));
        ok_writer.write_all(b"hello world").expect("write");
        let (buf, stats) = ok_writer.into_inner_with_stats().expect("into_inner");
        assert_eq!(buf, b"hello world");
        assert_eq!(stats.bytes_written, 11);

        let mut broken = StreamingWriter::with_capacity(
            FailAfter {
                remaining: 10,
                written: Vec::new(),
            },
            8,
        );
        broken.flush().expect("empty flush succeeds");
        let err = broken
            .write_all(b"0123456789abcdefghij")
            .and_then(|()| broken.finish().map(|_| ()))
            .expect_err("BrokenPipe must propagate");
        assert_eq!(err.kind(), ErrorKind::BrokenPipe);
    }

    #[test]
    fn stream_json_pretty_preflights_before_byte_zero_and_preserves_broken_pipe() {
        #[derive(serde::Serialize)]
        struct Row {
            value: f64,
        }
        struct BrokenSink;
        impl Write for BrokenSink {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(ErrorKind::BrokenPipe, "pipe closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut sink = Vec::new();
        let err = stream_json_pretty(&mut sink, &Row { value: f64::NAN }, |row| {
            check_finite_f64(row.value, "value")
        })
        .expect_err("NaN must be refused before byte 0");
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(
            sink.is_empty(),
            "preflight failure must write zero bytes, got {:?}",
            String::from_utf8_lossy(&sink)
        );

        let mut ok_sink = Vec::new();
        stream_json_pretty(&mut ok_sink, &Row { value: 1.5 }, |row| {
            check_finite_opt_f64(Some(row.value), "value")
        })
        .expect("finite value streams");
        assert_eq!(
            String::from_utf8(ok_sink).expect("utf8"),
            "{\n  \"value\": 1.5\n}\n"
        );
        assert!(check_finite_opt_f64(None, "none").is_ok());

        let err = stream_json_pretty(&mut BrokenSink, &Row { value: 2.0 }, |_| Ok(()))
            .expect_err("BrokenPipe inside JSON stream");
        assert_eq!(err.kind(), ErrorKind::BrokenPipe);
        BrokenSink.flush().expect("broken sink flush");

        // Non-IO serde_json error maps to InvalidData.
        let parse_err = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let data_err = json_to_io_error(&parse_err);
        assert_eq!(data_err.kind(), ErrorKind::InvalidData);

        assert_eq!(OutputDisposition::Buffered.as_str(), "buffered");
        assert_eq!(OutputDisposition::Streamed.as_str(), "streamed");
        assert_eq!(OutputDisposition::Paginated.as_str(), "paginated");
        assert_eq!(
            OutputDisposition::ExplicitlyCapped.as_str(),
            "explicitly_capped"
        );
        let class_labels: Vec<&str> = ReadLatencyClass::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            class_labels,
            vec![
                "local_bounded_read",
                "full_structural_read",
                "evidence_backed_inspection",
                "repository_read",
                "external_refresh",
                "output_dominated_read",
            ]
        );
        record_buffered_output(12);
        reset_peak_output_buffer();
        let mut out = anstream_stdout();
        assert_eq!(out.write(b"").expect("empty write to anstream_stdout"), 0);
        out.flush().expect("flush anstream_stdout");
        let _ = out.finish().expect("finish anstream_stdout");
        let mut styled = StyledStreamingWriter {
            inner: StyledStreamingInner::Styled(StreamingWriter::new(anstream::stdout().lock())),
        };
        assert_eq!(styled.write(b"").expect("empty write to styled stream"), 0);
        styled.flush().expect("flush styled stream");
        let _ = styled.finish().expect("finish styled stream");
        let mut raw = stdout();
        assert_eq!(raw.write(b"").expect("empty write to stdout"), 0);
        let _ = raw.finish().expect("finish stdout");
    }
}
