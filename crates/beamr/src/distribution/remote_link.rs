//! Outbound distribution-control surface — the [`DistributionControlFacility`]
//! trait BIFs and embedder APIs route LINK/UNLINK/EXIT2 controls through, and
//! its error type. The scheduler's implementation encodes real wire frames and
//! hands them to the generation-pinned control lane (`DistSender`); delivery
//! semantics are the DC-1..DC-6 contract on `distribution::control_link`.

use crate::process::{ExitReason, RemotePid};

/// Error returned by outbound distribution control operations.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RemoteLinkError {
    /// No route or connection is available to the target node.
    NoConnection,
    /// The remote endpoint does not name a process on the expected target node.
    BadTarget,
}

/// Backend used by BIFs and scheduler hooks to route distribution controls.
pub trait DistributionControlFacility: Send + Sync {
    /// Establish a remote link by sending LINK to the remote node.
    fn link_remote(&self, caller_pid: u64, target: RemotePid) -> Result<(), RemoteLinkError>;

    /// Remove a remote link by sending UNLINK to the remote node.
    fn unlink_remote(&self, caller_pid: u64, target: RemotePid) -> Result<(), RemoteLinkError>;

    /// Send an `exit/2` exit signal (wire EXIT2) to a remote process.
    ///
    /// Best-effort fire-and-forget: delivered iff a connection to the target
    /// node is up when the signal is sent; there is no backstop for a dropped
    /// EXIT2 (unlike link exits, which coarsen to `noconnection`).
    fn exit_remote(
        &self,
        caller_pid: u64,
        target: RemotePid,
        reason: ExitReason,
    ) -> Result<(), RemoteLinkError>;
}
