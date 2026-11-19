//! Reaching the distribution bundle: its config, connections, listener and
//! process-group registry, refused honestly when distribution is disabled.

use super::distribution_service::DistributionService;
use super::{Scheduler, SharedState};
use crate::distribution::DistributionConfig;
use crate::distribution::connection::ConnectionManager;
use crate::distribution::pg::PgRegistry;
use std::sync::Arc;

impl SharedState {
    /// The owned distribution bundle, or `None` when distribution is `Disabled`
    /// (spec §3.6). Every distribution read path — pg propagation, outbound
    /// controls, remote-send, the net-kernel facade — routes through this so a
    /// `Disabled` bundle refuses honestly rather than touching an absent runtime.
    pub(in crate::scheduler) fn distribution(&self) -> Option<&DistributionService> {
        self.distribution.service()
    }

    /// Test-only: the owned distribution manager, panicking if `Disabled`. The
    /// in-crate tests that drive the connection table directly always construct
    /// an owned bundle.
    #[cfg(test)]
    pub(in crate::scheduler) fn distribution_connections_or_panic(&self) -> &ConnectionManager {
        self.distribution()
            .expect("distribution owned in test")
            .connections()
    }

    /// Test-only: the owned outbound sender, panicking if absent (the wire tests
    /// construct it via `make_shared_state_with_dist_sender`).
    #[cfg(test)]
    pub(in crate::scheduler) fn dist_sender_or_panic(
        &self,
    ) -> &crate::distribution::sender::DistSender {
        self.distribution()
            .and_then(|dist| dist.sender())
            .expect("dist sender present in test")
    }
}

impl Scheduler {
    /// The distribution configuration, or `None` when distribution is `Disabled`
    /// (spec §3.6/§6). Replaces `distribution_config()` per the §6
    /// keep-old-working amendment: the old `&DistributionConfig` signature cannot
    /// honestly represent a `Disabled` bundle (there is no config to borrow and
    /// the no-panic rule forbids the only signature-preserving out), so the break
    /// is loud and `try_`-named — the old name stays discoverable via the alias.
    #[doc(alias = "distribution_config")]
    #[must_use]
    pub fn try_distribution_config(&self) -> Option<&DistributionConfig> {
        self.shared.distribution().map(DistributionService::config)
    }
    /// The distribution connection manager, or `None` when distribution is
    /// `Disabled` (spec §3.6/§6). Replaces `distribution_connections()` per the
    /// same §6 amendment as [`Self::try_distribution_config`]: a `Disabled`
    /// bundle has no manager to clone.
    #[doc(alias = "distribution_connections")]
    #[must_use]
    pub fn try_distribution_connections(&self) -> Option<ConnectionManager> {
        self.shared
            .distribution()
            .map(|dist| dist.connections().clone())
    }
    /// Start accepting inbound distribution connections on `addr`.
    ///
    /// Accepted peers run the OTP handshake (authenticated by the configured
    /// cookie) before being registered under their advertised node name. The
    /// returned [`AcceptHandle`](crate::distribution::connection::AcceptHandle)
    /// owns the accept loop: the caller must keep it alive, as dropping it aborts
    /// the loop and stops accepting new connections.
    ///
    /// Returns [`ErrorKind::Unsupported`](std::io::ErrorKind::Unsupported) when
    /// distribution is `Disabled` (spec §3.6): there is no manager to listen on,
    /// surfaced as a typed unavailable error rather than a silent absence.
    pub async fn start_distribution_listener(
        &self,
        addr: std::net::SocketAddr,
    ) -> std::io::Result<crate::distribution::connection::AcceptHandle> {
        let Some(dist) = self.shared.distribution() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "distribution is disabled on this scheduler",
            ));
        };
        dist.connections().listen(addr).await
    }
    #[must_use]
    pub fn pg_registry(&self) -> Arc<PgRegistry> {
        Arc::clone(&self.shared.pg_registry)
    }
}
