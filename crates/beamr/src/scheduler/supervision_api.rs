//! Link, monitor and exit-signal operations an embedder calls on the
//! scheduler, each routed through the facility the BIFs use.

use super::{Scheduler, supervision_integration};
use crate::distribution::remote_link::{DistributionControlFacility, RemoteLinkError};
use crate::process::{ExitReason, RemotePid};
use std::sync::Arc;

impl Scheduler {
    pub fn set_trap_exit(
        &self,
        pid: u64,
        value: bool,
    ) -> Result<bool, crate::native::links::LinkError> {
        let facility = supervision_integration::SchedulerLinkFacility {
            shared: Arc::clone(&self.shared),
        };
        crate::native::LinkFacility::set_trap_exit(&facility, pid, value)
    }

    /// Establish a unidirectional monitor and return its complete result.
    ///
    /// [`crate::native::supervision::MonitorResult::immediate_down`] is true
    /// when the target already had a retained exit tombstone and the resulting
    /// DOWN was admitted to the watcher's mailbox (or to its executing-slot
    /// pending messages). It remains false for an ordinary live monitor and
    /// when a tombstoned target has no live watcher slot that can accept DOWN.
    pub fn monitor_with_result(
        &self,
        watcher_pid: u64,
        target_pid: u64,
    ) -> Result<
        crate::native::supervision::MonitorResult,
        crate::native::supervision::SupervisionError,
    > {
        let facility = supervision_integration::SchedulerSupervisionFacility {
            shared: Arc::clone(&self.shared),
        };
        crate::native::supervision::SupervisionFacility::monitor(&facility, watcher_pid, target_pid)
    }
    /// Establish a unidirectional monitor from `watcher_pid` to `target_pid`,
    /// returning the monitor reference.
    ///
    /// Delegates to the existing pid-keyed `SupervisionFacility` used by the
    /// `monitor/2` BIF, so a `{'DOWN', ref, process, pid, reason}` message is
    /// delivered to the watcher via the same `deliver_down_messages` path when
    /// the target exits — there is no native-specific monitor handling. Works
    /// uniformly for bytecode and native targets.
    pub fn monitor(
        &self,
        watcher_pid: u64,
        target_pid: u64,
    ) -> Result<u64, crate::native::supervision::SupervisionError> {
        self.monitor_with_result(watcher_pid, target_pid)
            .map(|result| result.reference)
    }
    /// Send an exit signal to `target_pid` with `reason`, the embedding-side
    /// equivalent of `erlang:exit/2`.
    ///
    /// Delegates to the existing pid-keyed `SupervisionFacility`: an abnormal
    /// reason terminates a non-trapping target through `cleanup_exited_process`
    /// (propagating to its links and monitors) or is delivered as an
    /// `{'EXIT', from, reason}` message to a trapping target. No native-specific
    /// path is involved.
    pub fn exit_signal(
        &self,
        from_pid: u64,
        target_pid: u64,
        reason: ExitReason,
    ) -> Result<(), crate::native::supervision::SupervisionError> {
        let facility = supervision_integration::SchedulerSupervisionFacility {
            shared: Arc::clone(&self.shared),
        };
        crate::native::supervision::SupervisionFacility::exit_signal(
            &facility, from_pid, target_pid, reason,
        )
    }
    /// Establish a cross-node link between local `local_pid` and `remote`,
    /// the embedding-side equivalent of `link/1` on an external pid.
    ///
    /// Records the local half-link and sends a wire LINK over the already
    /// established connection to `remote.node` (no auto-dial: connect first,
    /// then link). Errors: [`RemoteLinkError::BadTarget`] when `local_pid` is
    /// dead or absent, or when a `remote` pid component exceeds the wire's
    /// u32 range; [`RemoteLinkError::NoConnection`] when no connection to
    /// `remote.node` exists — in either error case no local half-link is left
    /// behind. `remote.serial` is normalized to 0 (the wire link identity):
    /// the peer mints every EXIT/UNLINK `from` pid with serial 0, so a
    /// nonzero serial could never match the stored link when the exit signal
    /// arrives. Delegates to the same facility the `link/1` BIF uses.
    pub fn link_remote(&self, local_pid: u64, remote: RemotePid) -> Result<(), RemoteLinkError> {
        let facility = supervision_integration::SchedulerDistributionControlFacility {
            shared: Arc::clone(&self.shared),
        };
        facility.link_remote(local_pid, remote)
    }
    /// Remove a cross-node link between local `local_pid` and `remote`, the
    /// embedding-side equivalent of `unlink/1` on an external pid.
    ///
    /// Removes the local half-link and sends a best-effort wire UNLINK (an
    /// absent connection drops it — the peer's own down handling severs its
    /// half). `remote.serial` is normalized to 0, mirroring
    /// [`Scheduler::link_remote`], so a nonzero serial cannot miss the stored
    /// half-link. Delegates to the same facility the `unlink/1` BIF uses.
    pub fn unlink_remote(&self, local_pid: u64, remote: RemotePid) -> Result<(), RemoteLinkError> {
        let facility = supervision_integration::SchedulerDistributionControlFacility {
            shared: Arc::clone(&self.shared),
        };
        facility.unlink_remote(local_pid, remote)
    }
}
