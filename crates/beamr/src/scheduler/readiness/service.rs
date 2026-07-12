use super::core::{ReadinessCore, RouteHome, ServiceConsumerId};
use super::types::{Interest, ReadinessBuildError, ReadinessError, ReadinessToken};
use crate::atom::Atom;
use crate::scheduler::service::{ServiceIdentity, ServiceInstanceId, ShutdownService};
use std::os::fd::RawFd;
use std::sync::Arc;

/// The readiness service carried in a scheduler `ServiceMode`.
pub(in crate::scheduler) struct ReadinessService {
    core: Arc<ReadinessCore>,
    instance: ServiceInstanceId,
}

impl ReadinessService {
    pub(in crate::scheduler) fn build_owned(
        route_home: RouteHome,
    ) -> Result<Self, ReadinessBuildError> {
        Ok(Self {
            core: ReadinessCore::build(Some(route_home))?,
            instance: ServiceInstanceId::mint(),
        })
    }

    pub(in crate::scheduler) fn consumer(&self, route_home: RouteHome) -> ReadinessConsumer {
        let mut route = self.core.take_initial_route().unwrap_or(route_home);
        route.consumer = ServiceConsumerId::mint();
        ReadinessConsumer {
            core: Arc::clone(&self.core),
            route,
        }
    }

    pub(in crate::scheduler) fn poll_thread_names(&self) -> Vec<String> {
        self.core.poll_thread_names()
    }

    pub(in crate::scheduler) fn poll_fd_classes(&self) -> Vec<&'static str> {
        vec!["poll", "waker"]
    }

    /// Test-only observability: the §3.3 gate proves sweep-success by
    /// counting records (no record ⇒ no possible delivery attempt). The §5
    /// inventory deliberately does NOT read this — taking the table lock on
    /// every inventory build is the idle-cost class this service kills
    /// (reviewer-of-record A-2).
    #[cfg(test)]
    pub(in crate::scheduler) fn live_registration_count(&self) -> usize {
        self.core.live_registration_count()
    }

    #[cfg(test)]
    pub(in crate::scheduler) fn poll_iterations(&self) -> u64 {
        self.core.poll_iterations()
    }

    #[cfg(test)]
    pub(in crate::scheduler) fn panic_next_delivery(&self) {
        self.core.panic_next_delivery();
    }

    #[cfg(test)]
    pub(in crate::scheduler) fn inject_stale_readable(&self, token: ReadinessToken) {
        self.core.inject_stale_readable(token);
    }
}

impl ServiceIdentity for ReadinessService {
    fn instance_id(&self) -> ServiceInstanceId {
        self.instance
    }
}

impl ShutdownService for ReadinessService {
    fn shutdown(&self) {
        self.core.shutdown();
    }
}

impl Drop for ReadinessService {
    fn drop(&mut self) {
        self.core.shutdown();
    }
}

/// Process-wide shared readiness service, built once and injected by an embedder.
#[derive(Clone)]
pub struct SharedReadiness(pub(in crate::scheduler) Arc<ReadinessService>);

impl SharedReadiness {
    pub fn new() -> Result<Self, ReadinessBuildError> {
        Ok(Self(Arc::new(ReadinessService {
            core: ReadinessCore::build(None)?,
            instance: ServiceInstanceId::mint(),
        })))
    }
}

/// Per-scheduler registration handle carrying that scheduler's route-home.
#[derive(Clone)]
pub(in crate::scheduler) struct ReadinessConsumer {
    core: Arc<ReadinessCore>,
    route: RouteHome,
}

impl ReadinessConsumer {
    pub(in crate::scheduler) fn register(
        &self,
        fd: RawFd,
        interest: Interest,
        pid: u64,
        marker: Atom,
    ) -> Result<ReadinessToken, ReadinessError> {
        let Some(shared) = self.route.scheduler.upgrade() else {
            return Err(ReadinessError::TeardownInProgress);
        };
        let Some(_admission) = shared.try_reserve_teardown_admission() else {
            return Err(ReadinessError::TeardownInProgress);
        };
        self.core
            .register(self.route.clone(), fd, interest, pid, marker)
    }

    pub(in crate::scheduler) fn rearm(
        &self,
        token: &ReadinessToken,
        interest: Interest,
    ) -> Result<(), ReadinessError> {
        let Some(shared) = self.route.scheduler.upgrade() else {
            return Err(ReadinessError::TeardownInProgress);
        };
        let Some(_admission) = shared.try_reserve_teardown_admission() else {
            return Err(ReadinessError::TeardownInProgress);
        };
        self.core.rearm(token, interest)
    }

    pub(in crate::scheduler) fn deregister(&self, token: ReadinessToken) {
        self.core.deregister(token);
    }

    pub(in crate::scheduler) fn deregister_all(&self) {
        self.core.deregister_all_for(self.route.consumer);
    }

    pub(in crate::scheduler) fn deregister_pid(&self, pid: u64) {
        self.core.deregister_pid(self.route.consumer, pid);
    }
}
