use crate::atom::Atom;
use crate::scheduler::{Interest, ReadinessError, ReadinessToken};
use std::os::fd::RawFd;

/// In-slice readiness registration and one-shot rearm facility.
pub trait ReadinessFacility: Send + Sync {
    /// Arm `fd` and route its marker back to `pid` on this scheduler.
    ///
    /// The facility supplies its scheduler's route-home; consumers provide no
    /// scheduler identity and receive a typed refusal before they can park.
    fn register(
        &self,
        fd: RawFd,
        interest: Interest,
        pid: u64,
        marker: Atom,
    ) -> Result<ReadinessToken, ReadinessError>;

    /// Re-arm a triggered one-shot registration after draining the fd.
    fn rearm(&self, token: &ReadinessToken, interest: Interest) -> Result<(), ReadinessError>;
}
