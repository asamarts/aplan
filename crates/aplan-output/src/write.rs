//! Byte-boundary retrying stdout writer for transient `WouldBlock` (`EAGAIN`).
//!
//! When a caller inherits a non-blocking stdout descriptor (for example a hook
//! or harness piping a large report), `std::io::Write::write_all` retries
//! `Interrupted` (`EINTR`) but returns `WouldBlock` immediately after a partial
//! write. Ignoring `WouldBlock` truncates the report; restarting the renderer
//! duplicates the prefix already written.
//!
//! [`RetryingWriter`] retries `WouldBlock` at the byte-write boundary with
//! bounded exponential backoff so a draining downstream pipe receives every
//! byte intact, a stalled reader that never drains hits the deadline instead of
//! spinning forever, `BrokenPipe` propagates unchanged for clean exit semantics,
//! and unrelated I/O errors remain ordinary internal failures.

use std::io::{self, ErrorKind, Write};
use std::time::{Duration, Instant};

/// Default upper bound on waiting for a non-blocking descriptor to become
/// writable during a single write or flush call.
pub const DEFAULT_WOULD_BLOCK_BUDGET: Duration = Duration::from_secs(10);

const INITIAL_BACKOFF: Duration = Duration::from_millis(1);
const MAX_BACKOFF: Duration = Duration::from_millis(25);

/// A [`Write`] adapter that retries transient `WouldBlock` and `Interrupted`
/// errors at the byte boundary with a bounded wait budget.
#[derive(Debug)]
pub struct RetryingWriter<W> {
    inner: W,
    max_wait: Duration,
}

impl<W> RetryingWriter<W> {
    /// Wrap `inner` with [`DEFAULT_WOULD_BLOCK_BUDGET`].
    pub const fn new(inner: W) -> Self {
        Self {
            inner,
            max_wait: DEFAULT_WOULD_BLOCK_BUDGET,
        }
    }

    /// Wrap `inner` with an explicit wait budget for `WouldBlock` retries.
    pub const fn with_budget(inner: W, max_wait: Duration) -> Self {
        Self { inner, max_wait }
    }

    /// Recover the underlying writer.
    pub fn into_inner(self) -> W {
        self.inner
    }
}

/// Lock `std::io::stdout()` behind a [`RetryingWriter`].
pub fn stdout() -> RetryingWriter<io::StdoutLock<'static>> {
    RetryingWriter::new(io::stdout().lock())
}

/// Lock `anstream::stdout()` behind a [`RetryingWriter`].
pub fn anstream_stdout() -> RetryingWriter<anstream::AutoStream<io::StdoutLock<'static>>> {
    RetryingWriter::new(anstream::stdout().lock())
}

impl<W: Write> Write for RetryingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut started: Option<Instant> = None;
        let mut delay = INITIAL_BACKOFF;
        loop {
            match self.inner.write(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    let start = *started.get_or_insert_with(Instant::now);
                    let elapsed = start.elapsed();
                    if elapsed >= self.max_wait {
                        return Err(e);
                    }
                    let remaining = self.max_wait.saturating_sub(elapsed);
                    std::thread::sleep(delay.min(remaining));
                    delay = (delay * 2).min(MAX_BACKOFF);
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut started: Option<Instant> = None;
        let mut delay = INITIAL_BACKOFF;
        loop {
            match self.inner.flush() {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    let start = *started.get_or_insert_with(Instant::now);
                    let elapsed = start.elapsed();
                    if elapsed >= self.max_wait {
                        return Err(e);
                    }
                    let remaining = self.max_wait.saturating_sub(elapsed);
                    std::thread::sleep(delay.min(remaining));
                    delay = (delay * 2).min(MAX_BACKOFF);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Step {
        Partial(usize),
        WouldBlock,
        Interrupted,
        BrokenPipe,
        Other,
    }

    struct ScriptedWriter {
        steps: std::collections::VecDeque<Step>,
        written: Vec<u8>,
    }

    impl ScriptedWriter {
        fn new(steps: impl IntoIterator<Item = Step>) -> Self {
            Self {
                steps: steps.into_iter().collect(),
                written: Vec::new(),
            }
        }
    }

    impl Write for ScriptedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match self.steps.pop_front() {
                Some(Step::Partial(max)) => {
                    let n = max.min(buf.len());
                    self.written.extend_from_slice(&buf[..n]);
                    Ok(n)
                }
                Some(Step::WouldBlock) => Err(io::Error::new(
                    ErrorKind::WouldBlock,
                    "resource temporarily unavailable",
                )),
                Some(Step::Interrupted) => Err(io::Error::new(ErrorKind::Interrupted, "eintr")),
                Some(Step::BrokenPipe) => Err(io::Error::new(ErrorKind::BrokenPipe, "broken pipe")),
                Some(Step::Other) => Err(io::Error::other("disk fault")),
                None => {
                    self.written.extend_from_slice(buf);
                    Ok(buf.len())
                }
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct AlwaysWouldBlock;

    impl Write for AlwaysWouldBlock {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                ErrorKind::WouldBlock,
                "resource temporarily unavailable",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(
                ErrorKind::WouldBlock,
                "resource temporarily unavailable",
            ))
        }
    }

    #[test]
    fn retrying_writer_handles_partial_writes_and_would_block_without_duplication_or_truncation() {
        let payload = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mock = ScriptedWriter::new([
            Step::Partial(5),
            Step::WouldBlock,
            Step::Interrupted,
            Step::Partial(7),
            Step::WouldBlock,
            Step::WouldBlock,
            Step::Partial(11),
            Step::WouldBlock,
        ]);
        let mut writer = RetryingWriter::with_budget(mock, Duration::from_secs(1));
        writer.write_all(payload).expect("write completes");
        writer.flush().expect("flush completes");
        assert_eq!(writer.into_inner().written, payload);
    }

    #[test]
    fn retrying_writer_bounds_persistent_would_block_without_spinning_forever() {
        let mut writer = RetryingWriter::with_budget(AlwaysWouldBlock, Duration::from_millis(25));
        let started = Instant::now();
        let err = writer
            .write_all(b"stuck")
            .expect_err("persistent WouldBlock must time out");
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "bounded budget took too long: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn retrying_writer_preserves_broken_pipe_and_other_io_errors() {
        let mut pipe = RetryingWriter::with_budget(
            ScriptedWriter::new([Step::Partial(3), Step::BrokenPipe]),
            Duration::from_secs(1),
        );
        let err = pipe
            .write_all(b"abcdef")
            .expect_err("BrokenPipe must propagate");
        assert_eq!(err.kind(), ErrorKind::BrokenPipe);

        let mut other =
            RetryingWriter::with_budget(ScriptedWriter::new([Step::Other]), Duration::from_secs(1));
        let err = other
            .write_all(b"abcdef")
            .expect_err("unrelated I/O fault must propagate");
        assert_eq!(err.kind(), ErrorKind::Other);
    }

    struct ScriptedFlushWriter {
        steps: std::collections::VecDeque<Step>,
    }

    impl Write for ScriptedFlushWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            match self.steps.pop_front() {
                None | Some(Step::Partial(_)) => Ok(()),
                Some(Step::WouldBlock) => Err(io::Error::from(ErrorKind::WouldBlock)),
                Some(Step::Interrupted) => Err(io::Error::from(ErrorKind::Interrupted)),
                Some(Step::BrokenPipe) => Err(io::Error::from(ErrorKind::BrokenPipe)),
                Some(Step::Other) => Err(io::Error::other("flush failed")),
            }
        }
    }

    #[test]
    fn retrying_writer_flush_retries_transient_errors_bounds_stall_and_preserves_empty_write() {
        let mut ok = RetryingWriter::with_budget(
            ScriptedFlushWriter {
                steps: [Step::Interrupted, Step::WouldBlock].into_iter().collect(),
            },
            Duration::from_secs(1),
        );
        assert_eq!(ok.write(&[]).expect("empty write"), 0);
        ok.flush()
            .expect("flush retries Interrupted and WouldBlock");
        let _inner = ok.into_inner();

        let mut timeout = RetryingWriter::with_budget(AlwaysWouldBlock, Duration::from_millis(15));
        let err = timeout.flush().expect_err("stalled flush must time out");
        assert_eq!(err.kind(), ErrorKind::WouldBlock);

        let mut other = RetryingWriter::with_budget(
            ScriptedFlushWriter {
                steps: [Step::Other].into_iter().collect(),
            },
            Duration::from_secs(1),
        );
        let err = other.flush().expect_err("other flush fault propagates");
        assert_eq!(err.kind(), ErrorKind::Other);
    }
}
