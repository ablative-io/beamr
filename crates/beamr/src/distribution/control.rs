//! Distribution control messages for remote link lifecycle.

use std::sync::{Arc, Mutex};

use crate::atom::Atom;
use crate::distribution::connection::ConnectionManager;
use crate::process::{ExitReason, RemotePid};

/// Distribution control operation codes used by Erlang distribution.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ControlOp {
    /// LINK control message.
    Link,
    /// EXIT control message.
    Exit,
    /// UNLINK control message.
    Unlink,
}

impl ControlOp {
    /// Numeric distribution control opcode.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Link => 1,
            Self::Exit => 3,
            Self::Unlink => 4,
        }
    }

    /// Decode a supported distribution control opcode.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Link),
            3 => Some(Self::Exit),
            4 => Some(Self::Unlink),
            _ => None,
        }
    }
}

/// Local endpoint identity encoded in outbound distribution control messages.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LocalPid {
    /// Local node atom.
    pub node: Atom,
    /// Local process id number.
    pub pid_number: u64,
    /// Local pid serial.
    pub serial: u64,
}

/// Decoded or recorded remote-link control message.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ControlMessage {
    /// Distribution control operation.
    pub op: ControlOp,
    /// Source endpoint.
    pub from: RemotePid,
    /// Target endpoint.
    pub to: RemotePid,
    /// Exit reason for EXIT controls.
    pub reason: Option<ExitReason>,
}

/// Error returned by outbound distribution control helpers.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ControlError {
    /// No route or connection is available to the target node.
    NoConnection,
    /// The remote endpoint does not name a process on the expected target node.
    BadTarget,
}

/// Backend used by BIFs and scheduler hooks to route distribution controls.
pub trait DistributionControlFacility: Send + Sync {
    /// Establish a remote link by sending LINK to the remote node.
    fn link_remote(&self, caller_pid: u64, target: RemotePid) -> Result<(), ControlError>;

    /// Remove a remote link by sending UNLINK to the remote node.
    fn unlink_remote(&self, caller_pid: u64, target: RemotePid) -> Result<(), ControlError>;

    /// Propagate a local process exit to a linked remote process.
    fn exit_remote(
        &self,
        caller_pid: u64,
        target: RemotePid,
        reason: ExitReason,
    ) -> Result<(), ControlError>;
}

/// In-memory control router used by the scheduler until the wire control reader
/// owns full ETF framing. Tests can inspect recorded messages and inject inbound
/// controls through the same lifecycle methods used by decoded wire messages.
#[derive(Clone, Debug, Default)]
pub struct ControlRouter {
    messages: Arc<Mutex<Vec<ControlMessage>>>,
}

impl ControlRouter {
    /// Create an empty control router.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an outbound LINK control.
    pub fn send_link(&self, local_node: Atom, caller_pid: u64, target: RemotePid) {
        self.push(ControlMessage {
            op: ControlOp::Link,
            from: local_remote_pid(local_node, caller_pid),
            to: target,
            reason: None,
        });
    }

    /// Record an outbound UNLINK control.
    pub fn send_unlink(&self, local_node: Atom, caller_pid: u64, target: RemotePid) {
        self.push(ControlMessage {
            op: ControlOp::Unlink,
            from: local_remote_pid(local_node, caller_pid),
            to: target,
            reason: None,
        });
    }

    /// Record an outbound EXIT control.
    pub fn send_exit(
        &self,
        local_node: Atom,
        caller_pid: u64,
        target: RemotePid,
        reason: ExitReason,
    ) {
        self.push(ControlMessage {
            op: ControlOp::Exit,
            from: local_remote_pid(local_node, caller_pid),
            to: target,
            reason: Some(reason),
        });
    }

    /// Snapshot recorded messages in send order.
    #[must_use]
    pub fn messages(&self) -> Vec<ControlMessage> {
        self.messages
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn push(&self, message: ControlMessage) {
        self.messages
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(message);
    }
}

/// Send a control frame to an already-connected node.
///
/// Full external-term payload framing is owned by the lower distribution layers;
/// this helper provides the LINK/UNLINK/EXIT send seam and reports missing or
/// failed connections as `noconnection` to the scheduler.
pub async fn send_control_frame(
    connections: &ConnectionManager,
    target_node: Atom,
    frame: &[u8],
) -> Result<(), ControlError> {
    let Some(connection) = connections.get_connection(target_node) else {
        return Err(ControlError::NoConnection);
    };
    connection
        .write_raw(frame)
        .await
        .map_err(|_| ControlError::NoConnection)
}

fn local_remote_pid(local_node: Atom, pid_number: u64) -> RemotePid {
    RemotePid {
        node: local_node,
        pid_number,
        serial: 0,
    }
}
