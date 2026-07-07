//! Connection lifecycle event types for distribution links.
//!
//! Home of the legacy single-slot [`ConnectionDownHook`] (the
//! `register_connection_down` target). Types moved verbatim from
//! `connection.rs`; `connection.rs` re-exports them so existing import paths
//! keep compiling.

use std::sync::{Arc, RwLock};

use crate::atom::Atom;

/// Reason a distribution connection left the active table.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ConnectionDownReason {
    /// The peer closed its side of the connection cleanly.
    PeerClosed,
    /// A read operation reported an error.
    ReadError,
    /// A write operation reported an error.
    WriteError,
    /// A write exceeded its deadline (peer connected but not reading; kernel
    /// send buffer full). Treated as a terminal write failure by the outbound
    /// sender so a wedged peer cannot stall the shared drain.
    WriteTimeout,
    /// The local node explicitly closed the connection.
    ManualDisconnect,
    /// The proactive net-tick observed no inbound traffic (data frame or
    /// keepalive) within the configured liveness deadline: the peer is silently
    /// partitioned (no FIN/RST), so the link is marked down so the
    /// connection-event hub fires (pg-purge, noconnection delivery, embedder
    /// subscribers). Monitor-DOWN is work item A.
    HeartbeatTimeout,
}

/// Event emitted when a connection is removed from the active connection table.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ConnectionDownEvent {
    /// Node name key whose connection went down.
    pub node: Atom,
    /// Why the connection was removed.
    pub reason: ConnectionDownReason,
}

type ConnectionDownCallback = dyn Fn(ConnectionDownEvent) + Send + Sync + 'static;

/// Per-manager callback registration for connection-down notifications.
#[derive(Clone, Default)]
pub struct ConnectionDownHook {
    callback: Arc<RwLock<Option<Arc<ConnectionDownCallback>>>>,
}

impl ConnectionDownHook {
    /// Create an empty connection-down callback slot.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace the connection-down callback.
    pub fn register<F>(&self, callback: F)
    where
        F: Fn(ConnectionDownEvent) + Send + Sync + 'static,
    {
        let mut slot = self
            .callback
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *slot = Some(Arc::new(callback));
    }

    /// Remove the registered callback.
    pub fn unregister(&self) {
        let mut slot = self
            .callback
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *slot = None;
    }

    /// Return true when a callback is registered.
    #[must_use]
    pub fn is_registered(&self) -> bool {
        self.callback
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
    }

    pub(crate) fn invoke(&self, event: ConnectionDownEvent) {
        let callback = self
            .callback
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(callback) = callback {
            callback(event);
        }
    }
}
