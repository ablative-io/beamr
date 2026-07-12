use crate::atom::Atom;
use crate::scheduler::{Interest, ReadinessError, ReadinessToken};
use std::os::fd::RawFd;

/// In-slice readiness registration and one-shot rearm facility.
pub trait ReadinessFacility: Send + Sync {
    fn register(
        &self,
        fd: RawFd,
        interest: Interest,
        pid: u64,
        marker: Atom,
    ) -> Result<ReadinessToken, ReadinessError>;

    fn rearm(&self, token: &ReadinessToken, interest: Interest) -> Result<(), ReadinessError>;
}
