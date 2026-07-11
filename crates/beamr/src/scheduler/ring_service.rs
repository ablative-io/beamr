//! `ServiceMode` payloads for the platform completion rings (spec §3.3–§3.5).
//!
//! The file, standard, and generic IO rings are backend trait objects
//! (`Arc<dyn CompletionRing>`), so they cannot be carried in a
//! [`ServiceMode`](super::service::ServiceMode) directly — its
//! `Owned(T)`/`Shared(Arc<T>)` arms need a concrete, identity-bearing `T`.
//! These newtypes give each ring a process-unique
//! [`ServiceInstanceId`](super::service::ServiceInstanceId) and the
//! [`ShutdownService`] the owner drives, so `shutdown_owned` joins an Owned ring
//! and leaves a Shared one for its owner (spec §4).

use std::sync::Arc;

use super::service::{ServiceIdentity, ServiceInstanceId, ShutdownService};
use crate::io::{CompletionRing, StandardIoServer};

/// A completion ring carried in a `ServiceMode` (the file and generic rings).
///
/// The wrapper is what mints and holds the ring's identity: cloning a
/// [`ServiceMode::Shared`](super::service::ServiceMode::Shared) `Arc<RingService>`
/// into a second scheduler propagates the one id, so the process-wide dedup
/// (spec §5/§9) counts a shared ring's threads once.
pub(super) struct RingService {
    ring: Arc<dyn CompletionRing>,
    instance: ServiceInstanceId,
}

impl RingService {
    /// Wrap a freshly constructed ring, minting its process-unique identity.
    pub(super) fn new(ring: Arc<dyn CompletionRing>) -> Self {
        Self {
            ring,
            instance: ServiceInstanceId::mint(),
        }
    }

    /// Borrow the backend ring for submission and polling.
    pub(super) fn ring(&self) -> &Arc<dyn CompletionRing> {
        &self.ring
    }

    /// Worker threads the ring holds live right now (spec §5 `actual`).
    pub(super) fn worker_thread_names(&self) -> Vec<String> {
        self.ring.worker_thread_names()
    }

    /// Worker threads the ring was asked to run (spec §5 `configured`).
    pub(super) fn requested_worker_count(&self) -> usize {
        self.ring.requested_worker_count()
    }
}

impl ServiceIdentity for RingService {
    fn instance_id(&self) -> ServiceInstanceId {
        self.instance
    }
}

impl ShutdownService for RingService {
    fn shutdown(&self) {
        self.ring.shutdown();
    }
}

/// The standard-IO ring bundled with its group-leader server (spec §3.4).
///
/// Held `Owned` when the scheduler runs standard IO (process 0 live) and
/// `Disabled` when it does not — NEVER a live ring behind a disabled facade,
/// whose completion poll loop would hang a normal worker forever
/// (`io/standard_io.rs`).
pub(super) struct StandardIoService {
    server: StandardIoServer,
    instance: ServiceInstanceId,
}

impl StandardIoService {
    /// Wrap a server, minting the ring's process-unique identity.
    pub(super) fn new(server: StandardIoServer) -> Self {
        Self {
            server,
            instance: ServiceInstanceId::mint(),
        }
    }

    /// Borrow the group-leader server.
    pub(super) fn server(&self) -> &StandardIoServer {
        &self.server
    }
}

impl ServiceIdentity for StandardIoService {
    fn instance_id(&self) -> ServiceInstanceId {
        self.instance
    }
}

impl ShutdownService for StandardIoService {
    fn shutdown(&self) {
        self.server.shutdown();
    }
}
