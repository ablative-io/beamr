//! Registering file descriptors with the readiness service, and delivering
//! a process's marker message when one is ready.

use super::readiness::{Interest, ReadinessError, ReadinessToken};
use super::service::ServiceMode;
use super::{Scheduler, SharedState, execution, timer_integration};
use crate::term::Term;
use std::sync::Arc;

impl SharedState {
    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn readiness_register(
        self: &Arc<Self>,
        fd: std::os::fd::RawFd,
        interest: Interest,
        pid: u64,
        marker: crate::atom::Atom,
    ) -> Result<ReadinessToken, ReadinessError> {
        self.readiness_consumer
            .as_ref()
            .ok_or(ReadinessError::Disabled)?
            .register(fd, interest, pid, marker)
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn readiness_rearm(
        self: &Arc<Self>,
        token: &ReadinessToken,
        interest: Interest,
    ) -> Result<(), ReadinessError> {
        self.readiness_consumer
            .as_ref()
            .ok_or(ReadinessError::Disabled)?
            .rearm(token, interest)
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn readiness_deregister(&self, token: ReadinessToken) {
        if let Some(consumer) = &self.readiness_consumer {
            consumer.deregister(token);
        }
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn purge_readiness_state(&self, pid: u64) {
        if let Some(consumer) = &self.readiness_consumer {
            consumer.deregister_pid(pid);
        }
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn deregister_shared_readiness(&self) {
        if matches!(self.readiness, ServiceMode::Shared(_))
            && let Some(consumer) = &self.readiness_consumer
        {
            consumer.deregister_all();
        }
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn deliver_readiness_marker(
        &self,
        pid: u64,
        marker: crate::atom::Atom,
    ) -> bool {
        let delivered = timer_integration::deliver_term_to_mailbox(self, pid, Term::atom(marker));
        if delivered.is_ok() {
            execution::wake_process(self, pid);
        }
        delivered.is_ok()
    }
}

impl Scheduler {
    /// Host-side acknowledged readiness deregistration.
    #[cfg(feature = "readiness")]
    pub fn readiness_deregister(&self, token: ReadinessToken) {
        self.shared.readiness_deregister(token);
    }
}
