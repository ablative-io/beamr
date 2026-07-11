//! The distribution service bundle (spec §3.6).
//!
//! Distribution is ONE coherent service, not a scatter of fields: a node's
//! configuration, ONE heartbeat-enabled [`ConnectionManager`] backing every
//! path (listener, outbound send, pg, control, and the [`NetKernel`] facade),
//! the outbound [`DistSender`], and the net-kernel runtime. Carried in a
//! [`ServiceMode`](super::service::ServiceMode) so `distribution: None` is a
//! true absence — NEITHER runtime exists — while `Some(config)` is an
//! `Owned` bundle whose two tokio runtime workers are inventoried under one
//! §5 entry and JOINED at shutdown.
//!
//! The prior shape built TWO disjoint `ConnectionManager`s — the real one and a
//! second fresh one handed to `NetKernel`, so `connect_node`/`nodes/0` consulted
//! a table nothing else touched. That second manager is gone: one manager backs
//! everything (spec §3.6, the two-site acceptance line).

use std::sync::Arc;

use super::service::{ServiceIdentity, ServiceInstanceId, ShutdownService};
use crate::atom::AtomTable;
use crate::distribution::connection::{ConnectionManager, HeartbeatConfig};
use crate::distribution::sender::DistSender;
use crate::distribution::{DistributionConfig, NetKernel};

/// The owned distribution bundle carried inside a `ServiceMode` (spec §3.6).
pub(super) struct DistributionService {
    config: DistributionConfig,
    /// The ONE connection manager. Heartbeat-enabled; shared (cheap `Arc` clone)
    /// with the sender and the net-kernel facade so a single table backs
    /// listener/send/pg/control traffic AND `connect_node`/`nodes/0`.
    connections: ConnectionManager,
    /// Async outbound sender, owning the "beamr-dist-send" runtime that also
    /// drives the connection read/accept and heartbeat tasks. `None` under
    /// replay (no runtime, no outbound traffic) or if the runtime build failed.
    sender: Option<DistSender>,
    /// Synchronous net-kernel facade, owning the "beamr-net-kernel" runtime that
    /// drives blocking `connect_node` calls. Shares `connections`.
    net_kernel: Arc<NetKernel>,
    /// Process-wide identity of this bundle (spec §5): two inventory entries with
    /// equal ids ARE the same instance.
    instance: ServiceInstanceId,
    /// Runtime workers this bundle REQUESTED at construction — the net-kernel
    /// worker always, plus the sender worker when outbound traffic is driven
    /// (skipped under replay). Stable across shutdown, so the §5 `configured`
    /// count stays the construction request while `actual` truthfully drops to
    /// zero after the join; a build failure shows as `actual < configured`.
    configured_runtimes: usize,
}

impl DistributionService {
    /// Build the owned bundle: one heartbeat-enabled manager backing the sender
    /// and the net-kernel facade. The outbound sender is skipped when
    /// `with_sender` is false (replay has no runtime and no outbound traffic).
    pub(super) fn build(
        config: DistributionConfig,
        atom_table: Arc<AtomTable>,
        node_name: &str,
        creation: u32,
        with_sender: bool,
    ) -> Self {
        // ONE manager, heartbeat-enabled, built before any clone so `with_heartbeat`
        // (which needs unique ownership) takes effect, then shared into the sender
        // and the net-kernel facade.
        let connections = ConnectionManager::new(
            atom_table,
            Arc::clone(&config.resolver),
            config.cookie.clone(),
            node_name,
            creation,
        )
        .with_heartbeat(HeartbeatConfig::with_defaults());
        // Build the async outbound sender and bind its owned runtime handle to the
        // manager so the read/accept tasks are driven in production, where no
        // ambient tokio runtime exists.
        let sender = if with_sender {
            let sender = DistSender::new(connections.clone());
            if let Some(sender) = &sender {
                connections.set_runtime_handle(sender.handle());
            }
            sender
        } else {
            None
        };
        // The net-kernel facade shares the SAME manager (single-manager rule).
        let net_kernel = Arc::new(NetKernel::new(connections.clone()));
        let configured_runtimes = 1 + usize::from(with_sender);
        Self {
            config,
            connections,
            sender,
            net_kernel,
            instance: ServiceInstanceId::mint(),
            configured_runtimes,
        }
    }

    /// The bundle's distribution configuration.
    pub(super) fn config(&self) -> &DistributionConfig {
        &self.config
    }

    /// The single connection manager backing every distribution path.
    pub(super) fn connections(&self) -> &ConnectionManager {
        &self.connections
    }

    /// The outbound sender, or `None` under replay / on a runtime build failure.
    pub(super) fn sender(&self) -> Option<&DistSender> {
        self.sender.as_ref()
    }

    /// The net-kernel facade (shares this bundle's manager).
    pub(super) fn net_kernel(&self) -> &Arc<NetKernel> {
        &self.net_kernel
    }

    /// OS thread names of BOTH runtime workers (spec §5 `thread_names`): the
    /// sender's "beamr-dist-send" worker (when present and not yet joined) and
    /// the net-kernel's "beamr-net-kernel" worker.
    pub(super) fn runtime_thread_names(&self) -> Vec<String> {
        let mut names = self
            .sender
            .as_ref()
            .map(DistSender::worker_thread_names)
            .unwrap_or_default();
        names.extend(self.net_kernel.worker_thread_names());
        names
    }

    /// Runtime workers requested at construction (spec §5 `configured`).
    pub(super) fn configured_runtimes(&self) -> usize {
        self.configured_runtimes
    }

    /// Whether the proactive net-tick (heartbeat) is enabled (spec §3.7).
    pub(super) fn heartbeat_enabled(&self) -> bool {
        self.connections.heartbeat_enabled()
    }

    /// Count of heartbeat tasks spawned to date, for the §5 task-class policy line.
    pub(super) fn heartbeat_tasks_spawned(&self) -> u64 {
        self.connections.heartbeat_tasks_spawned()
    }
}

impl ServiceIdentity for DistributionService {
    fn instance_id(&self) -> ServiceInstanceId {
        self.instance
    }
}

impl ShutdownService for DistributionService {
    /// Ownership-ordered teardown (spec §4): connections FIRST, then runtimes.
    /// `disconnect_all` closes every active connection through the ordinary
    /// down path — write half closed (immediate FIN), read loop woken, table
    /// entry removed, Down events DELIVERED before it returns — and aborts
    /// in-flight dials, so shutdown is connection-complete (§3.6), not merely
    /// thread-complete: a retained connection handle or a stopped-but-undropped
    /// scheduler holds no open distribution socket. Then the sender is stopped —
    /// aborting the drain and joining the "beamr-dist-send" runtime, which also
    /// aborts the read/accept and heartbeat lifecycle tasks it drives — then the
    /// net-kernel runtime is joined. Both joins are synchronous, so BOTH runtime
    /// workers are gone before this returns. Idempotent (`disconnect_all` on an
    /// empty table is a no-op; each backend's `shutdown` is take-once).
    fn shutdown(&self) {
        self.connections.disconnect_all();
        if let Some(sender) = &self.sender {
            sender.shutdown();
        }
        self.net_kernel.shutdown();
    }
}
