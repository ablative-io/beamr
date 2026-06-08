//! Process-to-process standard I/O protocol support for `io` BIFs.

use crate::term::Term;

/// Facility used by I/O BIFs to send `{io_request, From, ReplyAs, Request}`
/// messages to group leaders or explicit I/O devices.
pub trait IoProtocolFacility: Send + Sync {
    /// Deliver `message` to `target_pid`, copying it into the target heap when required.
    fn send_io_request(&self, target_pid: u64, message: Term) -> bool;
}
