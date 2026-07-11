//! Configurable output sinks and resource lifecycle support used by I/O BIFs.

pub mod bridge;
pub mod facility;
pub mod resource;
pub mod ring;
pub mod standard_io;
#[cfg(not(target_os = "linux"))]
pub mod thread_pool;
#[cfg(target_os = "linux")]
pub mod uring;

use std::io::Write;

pub use bridge::{IoCompletionBridge, IoWakeTarget, PendingIo, PendingIoRegistry, ResultMode};
pub use facility::{CompletionRingIoFacility, IoError, IoFacility};
pub use standard_io::StandardIoServer;

use crate::atom::Atom;

pub use ring::{CompletionRing, IoCompletion, IoOp, IoResult, StatxData};
#[cfg(not(target_os = "linux"))]
pub use thread_pool::ThreadPoolRing;
#[cfg(target_os = "linux")]
pub use uring::IoUringRing;

/// Configuration for constructing the platform completion ring.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RingConfig {
    /// Linux io_uring queue depth. Defaults to 256.
    pub ring_depth: u32,
    /// Non-Linux fallback worker count. Defaults to 4.
    pub fallback_pool_size: usize,
}

/// Historical default thread-name prefix for fallback workers whose caller
/// does not name the service — the prefix every ring used before spec §5
/// introduced service-distinct names. [`create_ring`] and [`try_create_ring`]
/// keep this default so pre-§5 embedder call sites stay source-compatible.
pub const DEFAULT_RING_THREAD_PREFIX: &str = "beamr-io-thread-pool";
/// Thread-name prefix for the file-IO ring's fallback workers (spec §5).
///
/// Each ring gets a service-distinct prefix so the OS-thread probe and the
/// service inventory can attribute a worker to the right service; before this
/// the file, standard, and generic rings all named their workers
/// `beamr-io-thread-pool-*` and collided three ways.
pub const FILE_IO_RING_THREAD_PREFIX: &str = "beamr-file-io";
/// Thread-name prefix for the standard-IO ring's fallback workers (spec §5).
pub const STANDARD_IO_RING_THREAD_PREFIX: &str = "beamr-standard-io";
/// Thread-name prefix for the generic-IO ring's fallback workers (spec §5).
pub const GENERIC_IO_RING_THREAD_PREFIX: &str = "beamr-generic-io";

#[cfg(test)]
mod tests {
    use super::errno_to_atom;
    use crate::atom::Atom;

    /// Compile-pins the pre-§5 embedder surface: both ring factories accept a
    /// bare `RingConfig` (docs/design/beamr/briefs/B-100.md), and the fallback
    /// workers keep the historical `beamr-io-thread-pool-*` names embedders
    /// may already match on.
    #[test]
    fn one_argument_ring_factories_stay_source_compatible_with_the_default_prefix() {
        let config = super::RingConfig {
            ring_depth: 8,
            fallback_pool_size: 1,
        };

        let ring = super::create_ring(config);
        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            ring.worker_thread_names(),
            vec!["beamr-io-thread-pool-0".to_owned()],
            "one-argument create_ring must keep the historical default \
             worker prefix"
        );
        drop(ring);

        let fallible = super::try_create_ring(config);
        #[cfg(not(target_os = "linux"))]
        assert!(
            fallible.is_ok(),
            "one-argument try_create_ring must construct the fallback ring"
        );
        #[cfg(target_os = "linux")]
        drop(fallible);
    }

    #[test]
    fn errno_mapping_returns_erlang_reason_atoms() {
        assert_eq!(errno_to_atom(libc::ENOENT), Atom::ENOENT);
        assert_eq!(errno_to_atom(libc::EACCES), Atom::EACCES);
        assert_eq!(errno_to_atom(libc::EEXIST), Atom::EEXIST);
        assert_eq!(errno_to_atom(libc::EISDIR), Atom::EISDIR);
        assert_eq!(errno_to_atom(libc::ENOTDIR), Atom::ENOTDIR);
        assert_eq!(errno_to_atom(libc::ENOSPC), Atom::ENOSPC);
        assert_eq!(errno_to_atom(libc::EMFILE), Atom::EMFILE);
        assert_eq!(errno_to_atom(libc::ENFILE), Atom::ENFILE);
        assert_eq!(errno_to_atom(libc::EBADF), Atom::EBADF);
        assert_eq!(errno_to_atom(libc::EPIPE), Atom::EPIPE);
        assert_eq!(errno_to_atom(libc::EAGAIN), Atom::EAGAIN);
        assert_eq!(errno_to_atom(libc::EINVAL), Atom::EINVAL);
        assert_eq!(errno_to_atom(libc::ENOTEMPTY), Atom::ENOTEMPTY);
        assert_eq!(errno_to_atom(libc::EXDEV), Atom::EXDEV);
        assert_eq!(errno_to_atom(libc::ELOOP), Atom::ELOOP);
        assert_eq!(errno_to_atom(libc::EROFS), Atom::EROFS);
        assert_eq!(errno_to_atom(libc::ENAMETOOLONG), Atom::ENAMETOOLONG);
        assert_eq!(errno_to_atom(libc::EPERM), Atom::EPERM);
        assert_eq!(errno_to_atom(libc::ECONNREFUSED), Atom::ECONNREFUSED);
        assert_eq!(errno_to_atom(libc::ECONNRESET), Atom::ECONNRESET);
        assert_eq!(errno_to_atom(libc::EINPROGRESS), Atom::EINPROGRESS);
        assert_eq!(errno_to_atom(libc::ENOTCONN), Atom::ENOTCONN);
        assert_eq!(errno_to_atom(i32::MAX), Atom::UNKNOWN_ERROR);
    }
}

/// Map OS errno values into Erlang-style file error reason atoms.
#[must_use]
pub fn errno_to_atom(errno: i32) -> Atom {
    match errno {
        libc::ENOENT => Atom::ENOENT,
        libc::EACCES => Atom::EACCES,
        libc::EEXIST => Atom::EEXIST,
        libc::EISDIR => Atom::EISDIR,
        libc::ENOTDIR => Atom::ENOTDIR,
        libc::ENOSPC => Atom::ENOSPC,
        libc::EMFILE => Atom::EMFILE,
        libc::ENFILE => Atom::ENFILE,
        libc::EBADF => Atom::EBADF,
        libc::EPIPE => Atom::EPIPE,
        libc::EAGAIN => Atom::EAGAIN,
        libc::EINVAL => Atom::EINVAL,
        libc::ENOTEMPTY => Atom::ENOTEMPTY,
        libc::EXDEV => Atom::EXDEV,
        libc::ELOOP => Atom::ELOOP,
        libc::EROFS => Atom::EROFS,
        libc::ENAMETOOLONG => Atom::ENAMETOOLONG,
        libc::EPERM => Atom::EPERM,
        libc::ECONNREFUSED => Atom::ECONNREFUSED,
        libc::ECONNRESET => Atom::ECONNRESET,
        libc::EINPROGRESS => Atom::EINPROGRESS,
        libc::ENOTCONN => Atom::ENOTCONN,
        _ => Atom::UNKNOWN_ERROR,
    }
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            ring_depth: 256,
            fallback_pool_size: 4,
        }
    }
}

/// Construct the platform-appropriate completion ring with the historical
/// default worker prefix, [`DEFAULT_RING_THREAD_PREFIX`].
///
/// Callers that need service-distinct worker names for spec §5 inventory
/// attribution use [`create_ring_with_prefix`].
#[must_use]
pub fn create_ring(config: RingConfig) -> Box<dyn CompletionRing> {
    create_ring_with_prefix(config, DEFAULT_RING_THREAD_PREFIX)
}

/// Fallible platform ring construction with the historical default worker
/// prefix, [`DEFAULT_RING_THREAD_PREFIX`].
///
/// Callers that need service-distinct worker names for spec §5 inventory
/// attribution use [`try_create_ring_with_prefix`].
pub fn try_create_ring(config: RingConfig) -> std::io::Result<Box<dyn CompletionRing>> {
    try_create_ring_with_prefix(config, DEFAULT_RING_THREAD_PREFIX)
}

/// Construct the platform-appropriate completion ring.
///
/// `thread_name_prefix` names this ring's fallback worker threads (spec §5) so
/// the inventory can attribute them to the owning service; it is ignored on
/// Linux, whose io_uring backend owns no named OS worker threads.
#[must_use]
pub fn create_ring_with_prefix(
    config: RingConfig,
    thread_name_prefix: &str,
) -> Box<dyn CompletionRing> {
    #[cfg(target_os = "linux")]
    {
        let _ = thread_name_prefix;
        match try_create_ring_with_prefix(config, thread_name_prefix) {
            Ok(ring) => ring,
            Err(error) => Box::new(ring::FailedRing::new(error)),
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        Box::new(ThreadPoolRing::with_prefix(
            config.fallback_pool_size,
            thread_name_prefix,
        ))
    }
}

/// Fallible platform ring construction for callers that want backend initialization errors.
///
/// `thread_name_prefix` names the fallback worker threads (spec §5); ignored on
/// Linux (io_uring owns no named OS worker threads).
pub fn try_create_ring_with_prefix(
    config: RingConfig,
    thread_name_prefix: &str,
) -> std::io::Result<Box<dyn CompletionRing>> {
    #[cfg(target_os = "linux")]
    {
        let _ = thread_name_prefix;
        IoUringRing::new(config.ring_depth).map(|ring| Box::new(ring) as Box<dyn CompletionRing>)
    }

    #[cfg(not(target_os = "linux"))]
    {
        Ok(Box::new(ThreadPoolRing::with_prefix(
            config.fallback_pool_size,
            thread_name_prefix,
        )))
    }
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
