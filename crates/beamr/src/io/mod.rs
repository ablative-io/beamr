//! Configurable output sinks and completion I/O abstractions.

#[cfg(target_os = "linux")]
use std::io;
use std::io::Write;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "linux")]
use std::time::Duration;

pub mod ring;
#[cfg(not(target_os = "linux"))]
pub mod thread_pool;
#[cfg(target_os = "linux")]
pub mod uring;

pub use ring::{CompletionRing, IoCompletion, IoOp, IoResult, StatxData};
#[cfg(not(target_os = "linux"))]
pub use thread_pool::ThreadPoolRing;
#[cfg(target_os = "linux")]
pub use uring::IoUringRing;

/// Configuration for platform completion ring construction.
#[derive(Clone, Debug)]
pub struct RingConfig {
    /// Linux io_uring submission/completion queue depth.
    pub ring_depth: u32,
    /// Number of blocking fallback workers on non-Linux platforms.
    pub fallback_pool_size: usize,
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            ring_depth: 256,
            fallback_pool_size: 4,
        }
    }
}

/// Build the platform-appropriate completion ring.
///
/// On Linux this returns an `io_uring` backend when construction succeeds and
/// falls back to a completion ring that reports submission errors if the kernel
/// refuses ring construction. Non-Linux platforms use the blocking thread-pool
/// development fallback.
#[must_use]
pub fn create_ring(config: RingConfig) -> Box<dyn CompletionRing> {
    create_ring_for_platform(config)
}

#[cfg(target_os = "linux")]
fn create_ring_for_platform(config: RingConfig) -> Box<dyn CompletionRing> {
    match IoUringRing::new(config.ring_depth) {
        Ok(ring) => Box::new(ring),
        Err(error) => Box::new(UnavailableRing::new(error)),
    }
}

#[cfg(not(target_os = "linux"))]
fn create_ring_for_platform(config: RingConfig) -> Box<dyn CompletionRing> {
    Box::new(ThreadPoolRing::new(config.fallback_pool_size))
}

#[cfg(target_os = "linux")]
struct UnavailableRing {
    message: String,
    next_op_id: AtomicU64,
    completions: crossbeam_channel::Receiver<IoCompletion>,
    sender: crossbeam_channel::Sender<IoCompletion>,
}

#[cfg(target_os = "linux")]
impl UnavailableRing {
    fn new(error: io::Error) -> Self {
        let (sender, completions) = crossbeam_channel::unbounded();
        Self {
            message: error.to_string(),
            next_op_id: AtomicU64::new(1),
            completions,
            sender,
        }
    }
}

#[cfg(target_os = "linux")]
impl CompletionRing for UnavailableRing {
    fn submit(&self, _op: IoOp) -> u64 {
        let op_id = self.next_op_id.fetch_add(1, Ordering::Relaxed);
        let _ = self.sender.send(IoCompletion {
            op_id,
            result: Err(io::Error::other(format!(
                "io_uring backend unavailable: {}",
                self.message
            ))),
        });
        op_id
    }

    fn poll_completions(&self, timeout: Duration) -> Vec<IoCompletion> {
        let mut completions = Vec::new();
        match self.completions.recv_timeout(timeout) {
            Ok(completion) => completions.push(completion),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => return completions,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return completions,
        }
        while let Ok(completion) = self.completions.try_recv() {
            completions.push(completion);
        }
        completions
    }

    fn pending_count(&self) -> usize {
        0
    }

    fn shutdown(&self) {}
}

/// Output target for `io` module BIFs.
pub trait IoSink: Send + Sync {
    /// Write bytes to the sink.
    fn write(&self, bytes: &[u8]);
}

/// Default output sink that intentionally discards all bytes.
#[derive(Debug, Default)]
pub struct NullSink;

impl IoSink for NullSink {
    fn write(&self, _bytes: &[u8]) {}
}

/// Output sink that writes directly to process stdout.
#[derive(Debug, Default)]
pub struct StdoutSink;

impl IoSink for StdoutSink {
    fn write(&self, bytes: &[u8]) {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(bytes);
        let _ = stdout.flush();
    }
}
