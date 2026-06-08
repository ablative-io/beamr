//! Distribution control-message parsing and lifecycle dispatch.
//!
//! The external distribution protocol represents control operations as tuples
//! whose first element is the numeric operation code. This module keeps that
//! numeric table explicit, ignores unknown future opcodes, and provides small
//! seams that the scheduler/connection layers can plug into as distribution is
//! completed.

use crate::{
    atom::Atom,
    process::ExitReason,
    term::{Term, boxed::Tuple, pid_ref::PidRef},
};

/// Distribution control operation codes understood by beamr.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(i64)]
pub enum ControlOp {
    /// LINK: `{1, FromPid, ToPid}`.
    Link = 1,
    /// SEND: `{2, Cookie, ToPid}`.
    Send = 2,
    /// EXIT: `{3, FromPid, ToPid, Reason}`.
    Exit = 3,
    /// UNLINK: `{4, FromPid, ToPid}`.
    Unlink = 4,
    /// REG_SEND: `{6, FromPid, Cookie, ToName}`.
    RegSend = 6,
    /// EXIT2: `{8, FromPid, ToPid, Reason}`.
    Exit2 = 8,
    /// MONITOR_P: `{19, FromPid, ToPid, Ref}`.
    MonitorP = 19,
    /// DEMONITOR_P: `{20, FromPid, ToPid, Ref}`.
    DemonitorP = 20,
    /// MONITOR_P_EXIT: `{21, FromPid, ToPid, Ref, Reason}`.
    MonitorPExit = 21,
    /// SPAWN_REQUEST: `{29, ...}`.
    SpawnRequest = 29,
    /// SPAWN_REPLY: `{31, ...}`.
    SpawnReply = 31,
}

impl ControlOp {
    /// Decode a numeric control opcode.
    #[must_use]
    pub const fn from_opcode(opcode: i64) -> Option<Self> {
        match opcode {
            1 => Some(Self::Link),
            2 => Some(Self::Send),
            3 => Some(Self::Exit),
            4 => Some(Self::Unlink),
            6 => Some(Self::RegSend),
            8 => Some(Self::Exit2),
            19 => Some(Self::MonitorP),
            20 => Some(Self::DemonitorP),
            21 => Some(Self::MonitorPExit),
            29 => Some(Self::SpawnRequest),
            31 => Some(Self::SpawnReply),
            _ => None,
        }
    }

    /// Numeric opcode written on the wire.
    #[must_use]
    pub const fn opcode(self) -> i64 {
        self as i64
    }
}

/// Collision-safe process identity for distribution lifecycle state.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct DistributedPid {
    /// Remote node for external PIDs; `None` means this is a local immediate PID.
    pub node: Option<Atom>,
    /// PID number component.
    pub pid_number: u64,
    /// PID serial component. Local immediate PIDs use serial zero.
    pub serial: u64,
}

impl DistributedPid {
    /// Convert a local or remote PID term into a stable identity.
    pub fn from_term(term: Term) -> Option<Self> {
        let pid = PidRef::new(term)?;
        Some(Self {
            node: pid.node(),
            pid_number: pid.pid_number(),
            serial: pid.serial(),
        })
    }

    /// Construct a local PID identity.
    #[must_use]
    pub const fn local(pid_number: u64) -> Self {
        Self {
            node: None,
            pid_number,
            serial: 0,
        }
    }

    /// Construct a remote PID identity.
    #[must_use]
    pub const fn remote(node: Atom, pid_number: u64, serial: u64) -> Self {
        Self {
            node: Some(node),
            pid_number,
            serial,
        }
    }
}

/// Outbound control message captured before distribution framing/ETF encoding.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OutboundControlMessage {
    /// Operation to send.
    pub op: ControlOp,
    /// Source PID term.
    pub from: Term,
    /// Destination PID term.
    pub to: Term,
    /// Exit reason for EXIT/EXIT2 messages.
    pub reason: Option<ExitReason>,
}

impl OutboundControlMessage {
    /// Build a LINK control.
    #[must_use]
    pub const fn link(from: Term, to: Term) -> Self {
        Self {
            op: ControlOp::Link,
            from,
            to,
            reason: None,
        }
    }

    /// Build an UNLINK control.
    #[must_use]
    pub const fn unlink(from: Term, to: Term) -> Self {
        Self {
            op: ControlOp::Unlink,
            from,
            to,
            reason: None,
        }
    }

    /// Build an EXIT control propagated through a link.
    #[must_use]
    pub const fn exit(from: Term, to: Term, reason: ExitReason) -> Self {
        Self {
            op: ControlOp::Exit,
            from,
            to,
            reason: Some(reason),
        }
    }

    /// Build an EXIT2 control for explicit `exit/2` delivery.
    #[must_use]
    pub const fn exit2(from: Term, to: Term, reason: ExitReason) -> Self {
        Self {
            op: ControlOp::Exit2,
            from,
            to,
            reason: Some(reason),
        }
    }
}

/// Sink used by lifecycle code to hand encoded-control responsibility to the
/// distribution connection layer.
pub trait ControlMessageSink {
    /// Send a control message to the given remote node.
    fn send_control(
        &mut self,
        node: Atom,
        message: OutboundControlMessage,
    ) -> Result<(), ControlError>;
}

/// Errors from parsing or emitting control messages.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ControlError {
    /// The term was not a valid control tuple for the requested operation.
    MalformedControl,
    /// A PID argument was expected to name a remote node but did not.
    NotRemotePid,
    /// The sink failed to enqueue/write the control message.
    SendFailed,
}

/// Diagnostics hook for forward-compatible ignored messages.
pub trait ControlDiagnostics {
    /// Called when a future/unknown opcode is ignored.
    fn unknown_opcode(&mut self, opcode: i64);

    /// Called when a known opcode has a malformed control tuple.
    fn malformed_control(&mut self, op: Option<ControlOp>);
}

/// No-op diagnostics used in production until the crate adopts a logging facade.
#[derive(Copy, Clone, Debug, Default)]
pub struct NoopDiagnostics;

impl ControlDiagnostics for NoopDiagnostics {
    fn unknown_opcode(&mut self, _opcode: i64) {}

    fn malformed_control(&mut self, _op: Option<ControlOp>) {}
}

/// Handler table for decoded incoming control messages.
pub trait ControlMessageHandler {
    /// Basic SEND. Payload delivery is owned by the distribution data-message path.
    fn handle_send(&mut self, _tuple: Tuple) {}

    /// Registered SEND. Name lookup/delivery lands in later distribution work.
    fn handle_reg_send(&mut self, _tuple: Tuple) {}

    /// LINK establishes a cross-node link on the local side.
    fn handle_link(
        &mut self,
        _from: DistributedPid,
        _from_term: Term,
        _to: DistributedPid,
        _to_term: Term,
    ) {
    }

    /// UNLINK removes a cross-node link on the local side.
    fn handle_unlink(&mut self, _from: DistributedPid, _to: DistributedPid) {}

    /// EXIT propagates a linked-process exit.
    fn handle_exit(&mut self, _from: DistributedPid, _to: DistributedPid, _reason: ExitReason) {}

    /// EXIT2 delivers an explicit exit signal.
    fn handle_exit2(&mut self, _from: DistributedPid, _to: DistributedPid, _reason: ExitReason) {}

    /// MONITOR_P stub; dispatch arm intentionally exists for later monitor work.
    fn handle_monitor_p(&mut self, _tuple: Tuple) {}

    /// DEMONITOR_P stub; dispatch arm intentionally exists for later monitor work.
    fn handle_demonitor_p(&mut self, _tuple: Tuple) {}

    /// MONITOR_P_EXIT stub; dispatch arm intentionally exists for later monitor work.
    fn handle_monitor_p_exit(&mut self, _tuple: Tuple) {}

    /// SPAWN_REQUEST stub; dispatch arm intentionally exists for later spawn work.
    fn handle_spawn_request(&mut self, _tuple: Tuple) {}

    /// SPAWN_REPLY stub; dispatch arm intentionally exists for later spawn work.
    fn handle_spawn_reply(&mut self, _tuple: Tuple) {}
}

/// Dispatch a decoded control tuple to the handler for its opcode.
///
/// Unknown opcodes are deliberately ignored after notifying diagnostics so new
/// peers can add protocol operations without breaking older beamr nodes.
pub fn dispatch_control_message<H, D>(term: Term, handler: &mut H, diagnostics: &mut D)
where
    H: ControlMessageHandler,
    D: ControlDiagnostics,
{
    let Some(tuple) = Tuple::new(term) else {
        diagnostics.malformed_control(None);
        return;
    };
    let Some(opcode) = tuple.get(0).and_then(Term::as_small_int) else {
        diagnostics.malformed_control(None);
        return;
    };
    let Some(op) = ControlOp::from_opcode(opcode) else {
        diagnostics.unknown_opcode(opcode);
        return;
    };

    if !dispatch_known(tuple, op, handler) {
        diagnostics.malformed_control(Some(op));
    }
}

fn dispatch_known<H>(tuple: Tuple, op: ControlOp, handler: &mut H) -> bool
where
    H: ControlMessageHandler,
{
    match op {
        ControlOp::Send => {
            handler.handle_send(tuple);
            true
        }
        ControlOp::RegSend => {
            handler.handle_reg_send(tuple);
            true
        }
        ControlOp::Link => dispatch_link(tuple, handler),
        ControlOp::Exit => dispatch_exit(tuple, handler, false),
        ControlOp::Unlink => dispatch_unlink(tuple, handler),
        ControlOp::MonitorP => {
            handler.handle_monitor_p(tuple);
            true
        }
        ControlOp::DemonitorP => {
            handler.handle_demonitor_p(tuple);
            true
        }
        ControlOp::MonitorPExit => {
            handler.handle_monitor_p_exit(tuple);
            true
        }
        ControlOp::Exit2 => dispatch_exit(tuple, handler, true),
        ControlOp::SpawnRequest => {
            handler.handle_spawn_request(tuple);
            true
        }
        ControlOp::SpawnReply => {
            handler.handle_spawn_reply(tuple);
            true
        }
    }
}

fn dispatch_link<H>(tuple: Tuple, handler: &mut H) -> bool
where
    H: ControlMessageHandler,
{
    let Some((from, from_term, to, to_term)) = pid_pair_terms(tuple) else {
        return false;
    };
    handler.handle_link(from, from_term, to, to_term);
    true
}

fn dispatch_unlink<H>(tuple: Tuple, handler: &mut H) -> bool
where
    H: ControlMessageHandler,
{
    let Some((from, to)) = pid_pair(tuple) else {
        return false;
    };
    handler.handle_unlink(from, to);
    true
}

fn dispatch_exit<H>(tuple: Tuple, handler: &mut H, explicit: bool) -> bool
where
    H: ControlMessageHandler,
{
    let Some((from, to)) = pid_pair(tuple) else {
        return false;
    };
    let Some(reason) = tuple.get(3).and_then(exit_reason_from_term) else {
        return false;
    };
    if explicit {
        handler.handle_exit2(from, to, reason);
    } else {
        handler.handle_exit(from, to, reason);
    }
    true
}

fn pid_pair(tuple: Tuple) -> Option<(DistributedPid, DistributedPid)> {
    let (from, _, to, _) = pid_pair_terms(tuple)?;
    Some((from, to))
}

fn pid_pair_terms(tuple: Tuple) -> Option<(DistributedPid, Term, DistributedPid, Term)> {
    if tuple.arity() < 3 {
        return None;
    }
    let from_term = tuple.get(1)?;
    let to_term = tuple.get(2)?;
    let from = DistributedPid::from_term(from_term)?;
    let to = DistributedPid::from_term(to_term)?;
    Some((from, from_term, to, to_term))
}

/// Convert currently-supported exit reason atoms into runtime exit reasons.
#[must_use]
pub fn exit_reason_from_term(term: Term) -> Option<ExitReason> {
    match term.as_atom()? {
        Atom::NORMAL => Some(ExitReason::Normal),
        Atom::KILL => Some(ExitReason::Kill),
        Atom::KILLED => Some(ExitReason::Killed),
        Atom::ERROR => Some(ExitReason::Error),
        _ => None,
    }
}

/// Testable local lifecycle state for cross-node links and incoming exit signals.
///
/// Remote links are stored as full `(node, pid, serial)` identities rather than
/// collapsing to `pid_number`, avoiding collisions between different nodes.
#[derive(Clone, Debug, Default)]
pub struct ControlLifecycleState {
    links: Vec<RemoteLink>,
    delivered_exits: Vec<(DistributedPid, DistributedPid, ExitReason)>,
}

/// Stored cross-node link retaining both original PID terms for outbound control.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RemoteLink {
    /// Left side identity.
    pub left: DistributedPid,
    /// Original left-side PID term.
    pub left_term: Term,
    /// Right side identity.
    pub right: DistributedPid,
    /// Original right-side PID term.
    pub right_term: Term,
}

impl ControlLifecycleState {
    /// Establish a link while preserving insertion order and suppressing duplicates.
    pub fn establish_link(&mut self, left: DistributedPid, right: DistributedPid) -> bool {
        let left_term = pid_term(left).unwrap_or(Term::NIL);
        let right_term = pid_term(right).unwrap_or(Term::NIL);
        self.establish_link_terms(left, left_term, right, right_term)
    }

    /// Establish a link retaining the original PID terms for outbound controls.
    pub fn establish_link_terms(
        &mut self,
        left: DistributedPid,
        left_term: Term,
        right: DistributedPid,
        right_term: Term,
    ) -> bool {
        if left == right || self.has_link(left, right) {
            return false;
        }
        self.links.push(RemoteLink {
            left,
            left_term,
            right,
            right_term,
        });
        true
    }

    /// Remove a link in either direction.
    pub fn remove_link(&mut self, left: DistributedPid, right: DistributedPid) -> bool {
        let before = self.links.len();
        self.links
            .retain(|link| !same_link(link.left, link.right, left, right));
        before != self.links.len()
    }

    /// Return true when a link exists in either direction.
    #[must_use]
    pub fn has_link(&self, left: DistributedPid, right: DistributedPid) -> bool {
        self.links
            .iter()
            .any(|link| same_link(link.left, link.right, left, right))
    }

    /// Linked pairs in insertion order.
    #[must_use]
    pub fn links(&self) -> &[RemoteLink] {
        &self.links
    }

    /// Record delivery of an inbound EXIT/EXIT2 signal.
    pub fn deliver_exit(&mut self, from: DistributedPid, to: DistributedPid, reason: ExitReason) {
        self.delivered_exits.push((from, to, reason));
        self.remove_link(from, to);
    }

    /// Delivered exit signals in arrival order.
    #[must_use]
    pub fn delivered_exits(&self) -> &[(DistributedPid, DistributedPid, ExitReason)] {
        &self.delivered_exits
    }
}

impl ControlMessageHandler for ControlLifecycleState {
    fn handle_link(
        &mut self,
        from: DistributedPid,
        from_term: Term,
        to: DistributedPid,
        to_term: Term,
    ) {
        self.establish_link_terms(from, from_term, to, to_term);
    }

    fn handle_unlink(&mut self, from: DistributedPid, to: DistributedPid) {
        self.remove_link(from, to);
    }

    fn handle_exit(&mut self, from: DistributedPid, to: DistributedPid, reason: ExitReason) {
        self.deliver_exit(from, to, reason);
    }

    fn handle_exit2(&mut self, from: DistributedPid, to: DistributedPid, reason: ExitReason) {
        self.deliver_exit(from, to, reason);
    }
}

fn same_link(
    stored_left: DistributedPid,
    stored_right: DistributedPid,
    left: DistributedPid,
    right: DistributedPid,
) -> bool {
    (stored_left == left && stored_right == right) || (stored_left == right && stored_right == left)
}

fn pid_term(pid: DistributedPid) -> Option<Term> {
    if pid.node.is_none() {
        Term::try_pid(pid.pid_number)
    } else {
        None
    }
}

/// Send a LINK control to a remote PID and record the local half of the link.
pub fn link_remote<S>(
    sink: &mut S,
    state: &mut ControlLifecycleState,
    local_pid: Term,
    remote_pid: Term,
) -> Result<(), ControlError>
where
    S: ControlMessageSink,
{
    let local = DistributedPid::from_term(local_pid).ok_or(ControlError::MalformedControl)?;
    let remote = DistributedPid::from_term(remote_pid).ok_or(ControlError::MalformedControl)?;
    let node = remote.node.ok_or(ControlError::NotRemotePid)?;
    sink.send_control(node, OutboundControlMessage::link(local_pid, remote_pid))?;
    state.establish_link_terms(local, local_pid, remote, remote_pid);
    Ok(())
}

/// Send an UNLINK control to a remote PID and remove local lifecycle state.
pub fn unlink_remote<S>(
    sink: &mut S,
    state: &mut ControlLifecycleState,
    local_pid: Term,
    remote_pid: Term,
) -> Result<(), ControlError>
where
    S: ControlMessageSink,
{
    let local = DistributedPid::from_term(local_pid).ok_or(ControlError::MalformedControl)?;
    let remote = DistributedPid::from_term(remote_pid).ok_or(ControlError::MalformedControl)?;
    let node = remote.node.ok_or(ControlError::NotRemotePid)?;
    sink.send_control(node, OutboundControlMessage::unlink(local_pid, remote_pid))?;
    state.remove_link(local, remote);
    Ok(())
}

/// Send linked-process EXIT controls for every remote peer linked to `exiting_pid`.
pub fn propagate_remote_exit<S>(
    sink: &mut S,
    state: &mut ControlLifecycleState,
    exiting_pid: Term,
    reason: ExitReason,
) -> Result<(), ControlError>
where
    S: ControlMessageSink,
{
    let source = DistributedPid::from_term(exiting_pid).ok_or(ControlError::MalformedControl)?;
    let peers: Vec<(DistributedPid, Term)> = state
        .links()
        .iter()
        .filter_map(|link| {
            if link.left == source {
                Some((link.right, link.right_term))
            } else if link.right == source {
                Some((link.left, link.left_term))
            } else {
                None
            }
        })
        .collect();

    for (peer, peer_term) in peers {
        let Some(node) = peer.node else {
            continue;
        };
        if peer_term == Term::NIL {
            return Err(ControlError::MalformedControl);
        }
        sink.send_control(
            node,
            OutboundControlMessage::exit(exiting_pid, peer_term, reason),
        )?;
        state.remove_link(source, peer);
    }
    Ok(())
}

/// Send an explicit EXIT2 control to a remote PID.
pub fn exit2_remote<S>(
    sink: &mut S,
    from_pid: Term,
    remote_pid: Term,
    reason: ExitReason,
) -> Result<(), ControlError>
where
    S: ControlMessageSink,
{
    let remote = DistributedPid::from_term(remote_pid).ok_or(ControlError::MalformedControl)?;
    let node = remote.node.ok_or(ControlError::NotRemotePid)?;
    sink.send_control(
        node,
        OutboundControlMessage::exit2(from_pid, remote_pid, reason),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::boxed::{write_external_pid, write_tuple};

    #[derive(Default)]
    struct RecordingDiagnostics {
        unknown: Vec<i64>,
        malformed: Vec<Option<ControlOp>>,
    }

    impl ControlDiagnostics for RecordingDiagnostics {
        fn unknown_opcode(&mut self, opcode: i64) {
            self.unknown.push(opcode);
        }

        fn malformed_control(&mut self, op: Option<ControlOp>) {
            self.malformed.push(op);
        }
    }

    #[derive(Default)]
    struct RecordingHandler {
        calls: Vec<ControlOp>,
    }

    impl ControlMessageHandler for RecordingHandler {
        fn handle_send(&mut self, _tuple: Tuple) {
            self.calls.push(ControlOp::Send);
        }

        fn handle_reg_send(&mut self, _tuple: Tuple) {
            self.calls.push(ControlOp::RegSend);
        }

        fn handle_link(
            &mut self,
            _from: DistributedPid,
            _from_term: Term,
            _to: DistributedPid,
            _to_term: Term,
        ) {
            self.calls.push(ControlOp::Link);
        }

        fn handle_unlink(&mut self, _from: DistributedPid, _to: DistributedPid) {
            self.calls.push(ControlOp::Unlink);
        }

        fn handle_exit(&mut self, _from: DistributedPid, _to: DistributedPid, _reason: ExitReason) {
            self.calls.push(ControlOp::Exit);
        }

        fn handle_exit2(
            &mut self,
            _from: DistributedPid,
            _to: DistributedPid,
            _reason: ExitReason,
        ) {
            self.calls.push(ControlOp::Exit2);
        }

        fn handle_monitor_p(&mut self, _tuple: Tuple) {
            self.calls.push(ControlOp::MonitorP);
        }

        fn handle_demonitor_p(&mut self, _tuple: Tuple) {
            self.calls.push(ControlOp::DemonitorP);
        }

        fn handle_monitor_p_exit(&mut self, _tuple: Tuple) {
            self.calls.push(ControlOp::MonitorPExit);
        }

        fn handle_spawn_request(&mut self, _tuple: Tuple) {
            self.calls.push(ControlOp::SpawnRequest);
        }

        fn handle_spawn_reply(&mut self, _tuple: Tuple) {
            self.calls.push(ControlOp::SpawnReply);
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        sent: Vec<(Atom, OutboundControlMessage)>,
    }

    impl ControlMessageSink for RecordingSink {
        fn send_control(
            &mut self,
            node: Atom,
            message: OutboundControlMessage,
        ) -> Result<(), ControlError> {
            self.sent.push((node, message));
            Ok(())
        }
    }

    fn local_pid(pid: u64) -> Term {
        Term::pid(pid)
    }

    fn remote_pid(heap: &mut [u64], node: Atom, pid: u64) -> Term {
        write_external_pid(heap, node, pid, 0).expect("external pid fits")
    }

    fn tuple(heap: &mut [u64], elements: &[Term]) -> Term {
        write_tuple(heap, elements).expect("tuple fits")
    }

    fn control_tuple(heap: &mut [u64], op: ControlOp) -> Term {
        let elements = match op {
            ControlOp::Send => [
                Term::small_int(op.opcode()),
                Term::atom(Atom::NIL),
                local_pid(2),
                Term::NIL,
                Term::NIL,
            ],
            ControlOp::RegSend => [
                Term::small_int(op.opcode()),
                local_pid(1),
                Term::atom(Atom::NIL),
                Term::atom(Atom::OK),
                Term::NIL,
            ],
            ControlOp::Link | ControlOp::Unlink => [
                Term::small_int(op.opcode()),
                local_pid(1),
                local_pid(2),
                Term::NIL,
                Term::NIL,
            ],
            ControlOp::Exit | ControlOp::Exit2 => [
                Term::small_int(op.opcode()),
                local_pid(1),
                local_pid(2),
                Term::atom(Atom::ERROR),
                Term::NIL,
            ],
            ControlOp::MonitorP | ControlOp::DemonitorP => [
                Term::small_int(op.opcode()),
                local_pid(1),
                local_pid(2),
                Term::small_int(123),
                Term::NIL,
            ],
            ControlOp::MonitorPExit => [
                Term::small_int(op.opcode()),
                local_pid(1),
                local_pid(2),
                Term::small_int(123),
                Term::atom(Atom::ERROR),
            ],
            ControlOp::SpawnRequest | ControlOp::SpawnReply => [
                Term::small_int(op.opcode()),
                local_pid(1),
                local_pid(2),
                Term::NIL,
                Term::NIL,
            ],
        };
        tuple(heap, &elements)
    }

    #[test]
    fn control_op_mapping_is_exact() {
        assert_eq!(ControlOp::from_opcode(1), Some(ControlOp::Link));
        assert_eq!(ControlOp::from_opcode(2), Some(ControlOp::Send));
        assert_eq!(ControlOp::from_opcode(3), Some(ControlOp::Exit));
        assert_eq!(ControlOp::from_opcode(4), Some(ControlOp::Unlink));
        assert_eq!(ControlOp::from_opcode(6), Some(ControlOp::RegSend));
        assert_eq!(ControlOp::from_opcode(8), Some(ControlOp::Exit2));
        assert_eq!(ControlOp::from_opcode(19), Some(ControlOp::MonitorP));
        assert_eq!(ControlOp::from_opcode(20), Some(ControlOp::DemonitorP));
        assert_eq!(ControlOp::from_opcode(21), Some(ControlOp::MonitorPExit));
        assert_eq!(ControlOp::from_opcode(29), Some(ControlOp::SpawnRequest));
        assert_eq!(ControlOp::from_opcode(31), Some(ControlOp::SpawnReply));
        assert_eq!(ControlOp::from_opcode(255), None);
    }

    #[test]
    fn dispatches_each_known_opcode_to_correct_handler() {
        let ops = [
            ControlOp::Send,
            ControlOp::RegSend,
            ControlOp::Link,
            ControlOp::Exit,
            ControlOp::Unlink,
            ControlOp::MonitorP,
            ControlOp::DemonitorP,
            ControlOp::MonitorPExit,
            ControlOp::Exit2,
            ControlOp::SpawnRequest,
            ControlOp::SpawnReply,
        ];

        for op in ops {
            let mut heap = [0_u64; 8];
            let term = control_tuple(&mut heap, op);
            let mut handler = RecordingHandler::default();
            let mut diagnostics = RecordingDiagnostics::default();

            dispatch_control_message(term, &mut handler, &mut diagnostics);

            assert_eq!(handler.calls, vec![op]);
            assert_eq!(diagnostics.unknown, Vec::<i64>::new());
            assert_eq!(diagnostics.malformed, Vec::<Option<ControlOp>>::new());
        }
    }

    #[test]
    fn unknown_opcode_is_logged_and_ignored() {
        let mut heap = [0_u64; 3];
        let term = tuple(&mut heap, &[Term::small_int(999), local_pid(1)]);
        let mut handler = RecordingHandler::default();
        let mut diagnostics = RecordingDiagnostics::default();

        dispatch_control_message(term, &mut handler, &mut diagnostics);

        assert_eq!(handler.calls, Vec::<ControlOp>::new());
        assert_eq!(diagnostics.unknown, vec![999]);
        assert_eq!(diagnostics.malformed, Vec::<Option<ControlOp>>::new());
    }

    #[test]
    fn inbound_link_and_unlink_mutate_collision_safe_link_state() {
        let mut remote_heap = [0_u64; 4];
        let remote = remote_pid(&mut remote_heap, Atom::OK, 1);
        let local = local_pid(1);
        let mut tuple_heap = [0_u64; 4];
        let link_term = tuple(
            &mut tuple_heap,
            &[Term::small_int(ControlOp::Link.opcode()), remote, local],
        );
        let mut state = ControlLifecycleState::default();
        let mut diagnostics = NoopDiagnostics;

        dispatch_control_message(link_term, &mut state, &mut diagnostics);

        let remote_id = DistributedPid::remote(Atom::OK, 1, 0);
        let local_id = DistributedPid::local(1);
        assert!(state.has_link(remote_id, local_id));
        assert!(state.has_link(local_id, remote_id));
        assert_eq!(state.links()[0].left_term, remote);
        assert_eq!(state.links()[0].right_term, local);

        let mut unlink_heap = [0_u64; 4];
        let unlink_term = tuple(
            &mut unlink_heap,
            &[Term::small_int(ControlOp::Unlink.opcode()), remote, local],
        );
        dispatch_control_message(unlink_term, &mut state, &mut diagnostics);

        assert!(!state.has_link(remote_id, local_id));
    }

    #[test]
    fn outbound_remote_link_writes_link_control_and_records_state() {
        let mut remote_heap = [0_u64; 4];
        let remote = remote_pid(&mut remote_heap, Atom::OK, 42);
        let local = local_pid(7);
        let mut sink = RecordingSink::default();
        let mut state = ControlLifecycleState::default();

        assert_eq!(link_remote(&mut sink, &mut state, local, remote), Ok(()));

        assert_eq!(
            sink.sent,
            vec![(Atom::OK, OutboundControlMessage::link(local, remote))]
        );
        assert!(state.has_link(
            DistributedPid::local(7),
            DistributedPid::remote(Atom::OK, 42, 0)
        ));
    }

    #[test]
    fn linked_remote_exit_produces_exit_control() {
        let mut remote_heap = [0_u64; 4];
        let remote = remote_pid(&mut remote_heap, Atom::OK, 42);
        let local = local_pid(7);
        let mut sink = RecordingSink::default();
        let mut state = ControlLifecycleState::default();
        assert_eq!(link_remote(&mut sink, &mut state, local, remote), Ok(()));
        sink.sent.clear();

        assert_eq!(
            propagate_remote_exit(&mut sink, &mut state, local, ExitReason::Error),
            Ok(())
        );

        assert_eq!(sink.sent.len(), 1);
        assert_eq!(sink.sent[0].0, Atom::OK);
        assert_eq!(sink.sent[0].1.op, ControlOp::Exit);
        assert_eq!(sink.sent[0].1.from, local);
        assert_eq!(sink.sent[0].1.to, remote);
        assert_eq!(sink.sent[0].1.reason, Some(ExitReason::Error));
        assert!(!state.has_link(
            DistributedPid::local(7),
            DistributedPid::remote(Atom::OK, 42, 0)
        ));
    }

    #[test]
    fn inbound_exit2_delegates_to_exit_delivery_state() {
        let mut remote_heap = [0_u64; 4];
        let remote = remote_pid(&mut remote_heap, Atom::OK, 42);
        let local = local_pid(7);
        let mut state = ControlLifecycleState::default();
        state.establish_link(
            DistributedPid::remote(Atom::OK, 42, 0),
            DistributedPid::local(7),
        );
        let mut tuple_heap = [0_u64; 5];
        let exit2_term = tuple(
            &mut tuple_heap,
            &[
                Term::small_int(ControlOp::Exit2.opcode()),
                remote,
                local,
                Term::atom(Atom::ERROR),
            ],
        );
        let mut diagnostics = NoopDiagnostics;

        dispatch_control_message(exit2_term, &mut state, &mut diagnostics);

        assert_eq!(
            state.delivered_exits(),
            &[(
                DistributedPid::remote(Atom::OK, 42, 0),
                DistributedPid::local(7),
                ExitReason::Error,
            )]
        );
        assert!(!state.has_link(
            DistributedPid::remote(Atom::OK, 42, 0),
            DistributedPid::local(7)
        ));
    }

    #[test]
    fn explicit_exit2_to_remote_pid_writes_exit2_control() {
        let mut remote_heap = [0_u64; 4];
        let remote = remote_pid(&mut remote_heap, Atom::OK, 42);
        let local = local_pid(7);
        let mut sink = RecordingSink::default();

        assert_eq!(
            exit2_remote(&mut sink, local, remote, ExitReason::Kill),
            Ok(())
        );

        assert_eq!(
            sink.sent,
            vec![(
                Atom::OK,
                OutboundControlMessage::exit2(local, remote, ExitReason::Kill)
            )]
        );
    }
}
