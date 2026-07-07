//! LINK/UNLINK/EXIT/EXIT2 control codec, the distribution opcode table, and the
//! sink-bundle dispatcher.
//!
//! Wire shape (framing unchanged, payload always `NIL`):
//!
//! | Op | Control tuple | Reason rule |
//! |----|--------------|-------------|
//! | LINK = 1 | `{1, FromExtPid, ToExtPid}` | — |
//! | EXIT = 3 | `{3, FromExtPid, ToExtPid, ReasonAtom}` | always terminal (Kill pre-converted to Killed by the sender) |
//! | UNLINK = 4 | `{4, FromExtPid, ToExtPid}` | — |
//! | EXIT2 = 8 | `{8, FromExtPid, ToExtPid, ReasonAtom}` | raw reason; MAY carry `kill` (untrappable at the receiver) |
//!
//! Both pid fields are NEW_PID_EXT **external** pids. `From` is minted as
//! `(local_node, from_pid, serial 0, creation 0)`; `To` is the target
//! [`RemotePid`] verbatim with creation 0. Node-less pids in either position
//! are rejected at decode. The serial-0 `from` convention is self-consistent:
//! a LINK's `from = (A, pid, 0)` is exactly the [`RemotePid`] the peer stores,
//! and exactly what a later EXIT's `from` must equal for the link-removal
//! equality gate to hit.
//!
//! Known deviation from OTP >= 26: beamr uses plain UNLINK (4) rather than
//! UNLINK_ID (35) / UNLINK_ID_ACK (36). beamr peers are beamr-only and beamr's
//! local unlink has no ack either, so plain UNLINK is internally consistent;
//! the unlink/exit crossing race is accepted v1 behavior.
//!
//! # Delivery & ordering contract (DC-1..DC-6, normative)
//!
//! - **DC-1 (send-or-down; no silent arm).** For every enqueued control C
//!   against a pinned connection G, exactly one of: (a) C is written to G's
//!   socket in per-node FIFO order; (b) G is marked down (overflow, write
//!   error, write timeout, or encode failure). Down fires the connection-down
//!   hook, which supplies DC-3.
//! - **DC-2 (generation pinning).** A control enqueued against generation G is
//!   written to G's socket or not at all; controls never leak onto a
//!   post-redial connection. After any down+redial, cross-node link state
//!   between the pair starts empty on both sides and must be re-established by
//!   fresh LINKs.
//! - **DC-3 (noconnection backstop — scope-honest).** On connection-down, pg
//!   purge runs, then noconnection delivery to every local process holding a
//!   remote link over that node — linked processes ONLY. A lost link-EXIT is
//!   therefore never a lost signal (coarsened to `noconnection`). EXIT2 to an
//!   unlinked target has NO backstop and is best-effort.
//! - **DC-4 (exactly-once exit per link).** For each remote link, exactly one
//!   of {wire EXIT(3), noconnection} is applied: both paths consume the link
//!   entry under the target's slot lock; a link-exit with no entry is a no-op.
//! - **DC-5 (per-node control FIFO).** All link controls for a node traverse
//!   one bounded channel, one drain task, and one per-connection writer mutex —
//!   a LINK enqueued before an EXIT is written before it.
//! - **DC-6 (cross-lane ordering, honest statement).** A wire EXIT(3) produced
//!   by a process's death never overtakes messages that process sent before
//!   dying (SENDs complete their socket write inside the sending BIF). NOT
//!   guaranteed: an EXIT2 (async lane) may be overtaken by a subsequent SEND
//!   from the same caller; no ordering exists between link controls and pg
//!   frames.

use crate::atom::{Atom, AtomTable};
use crate::distribution::control::{
    self, ControlDelivery, ControlError, ControlMessage, ControlRegistry, PG_UPDATE, PgDelivery,
    REG_SEND, SEND,
};
use crate::etf::decode::decode_term;
use crate::etf::encode::EncodeError;
use crate::native::ProcessContext;
use crate::process::{ExitReason, Process, RemotePid};
use crate::term::Term;
use crate::term::boxed::Tuple;
use crate::term::pid_ref::PidRef;

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

/// Reserved beamr-private monitor opcode. No encoder/decoder ships yet.
///
/// beamr-private (outside OTP's 1..=31 range) because OTP's MONITOR_P family
/// (19/20/21) carries real `Reference` terms the term layer cannot represent
/// (no creation component; local monitor refs are small integers). Staged
/// tuple shapes, with integer refs allocated by the watcher node from the
/// `1 << 56` per-node partition:
/// `{102, 1, WatcherExtPid, TargetExtPid, RefInt}` monitor;
/// `{102, 2, WatcherExtPid, TargetExtPid, RefInt}` demonitor;
/// `{102, 3, TargetExtPid, WatcherExtPid, RefInt, ReasonAtom}` down.
pub const BEAMR_MONITOR: i64 = 102;

/// Pattern constants for the decode match arms (values from [`ControlOp`]).
const LINK: i64 = ControlOp::Link.opcode();
const EXIT: i64 = ControlOp::Exit.opcode();
const UNLINK: i64 = ControlOp::Unlink.opcode();
const EXIT2: i64 = ControlOp::Exit2.opcode();

/// Encode a framed LINK control `{1, FromExtPid, ToExtPid}` with a NIL payload.
///
/// `from` is minted as `(local_node, from_pid, serial 0, creation 0)`; `to` is
/// the target [`RemotePid`] verbatim with creation 0. Restart aliasing (D5):
/// creation is not carried on the wire (boxed external pids are 3-component),
/// so a peer restart during a partition can alias pre-restart pids until the
/// reconnect handshake replaces the connection.
pub fn encode_link_frame(
    local_node: Atom,
    from_pid: u64,
    to: RemotePid,
    atom_table: &AtomTable,
) -> Result<Vec<u8>, EncodeError> {
    encode_control_frame(ControlOp::Link, local_node, from_pid, to, None, atom_table)
}

/// Encode a framed UNLINK control `{4, FromExtPid, ToExtPid}` with a NIL
/// payload. Pid conventions match [`encode_link_frame`].
pub fn encode_unlink_frame(
    local_node: Atom,
    from_pid: u64,
    to: RemotePid,
    atom_table: &AtomTable,
) -> Result<Vec<u8>, EncodeError> {
    encode_control_frame(
        ControlOp::Unlink,
        local_node,
        from_pid,
        to,
        None,
        atom_table,
    )
}

/// Encode a framed EXIT (op 3, link-exit) or EXIT2 (op 8, `exit/2`) control
/// `{Op, FromExtPid, ToExtPid, ReasonAtom}` with a NIL payload.
///
/// `op` must be [`ControlOp::Exit`] or [`ControlOp::Exit2`]; any other op is
/// an encode error. Pid conventions match [`encode_link_frame`]. EXIT reasons
/// must already be terminal at the call site (Kill pre-converted to Killed);
/// EXIT2 carries the raw reason and MAY carry `kill`.
pub fn encode_exit_frame(
    op: ControlOp,
    local_node: Atom,
    from_pid: u64,
    to: RemotePid,
    reason: ExitReason,
    atom_table: &AtomTable,
) -> Result<Vec<u8>, EncodeError> {
    if !matches!(op, ControlOp::Exit | ControlOp::Exit2) {
        return Err(EncodeError::UnsupportedTerm);
    }
    encode_control_frame(op, local_node, from_pid, to, Some(reason), atom_table)
}

/// Shared encoder body mirroring `encode_pg_update_frame`: a 64-word temporary
/// process heap, external pids for **both** endpoints, NIL payload.
fn encode_control_frame(
    op: ControlOp,
    local_node: Atom,
    from_pid: u64,
    to: RemotePid,
    reason: Option<ExitReason>,
    atom_table: &AtomTable,
) -> Result<Vec<u8>, EncodeError> {
    let mut process = Process::new(0, 64);
    let mut context = ProcessContext::new();
    context.attach_process(&mut process, 0);
    // Serial 0: local immediate PIDs have no serial component, and the wire
    // identity for a locally-hosted endpoint is (local_node, from_pid, 0).
    let from = context
        .alloc_external_pid(local_node, from_pid, 0)
        .map_err(|_| EncodeError::UnsupportedTerm)?;
    let to = context
        .alloc_external_pid(to.node, to.pid_number, to.serial)
        .map_err(|_| EncodeError::UnsupportedTerm)?;
    let control = match reason {
        None => context.alloc_tuple(&[Term::small_int(op.opcode()), from, to]),
        Some(reason) => {
            context.alloc_tuple(&[Term::small_int(op.opcode()), from, to, reason.as_term()])
        }
    }
    .map_err(|_| EncodeError::UnsupportedTerm)?;
    control::encode_frame(control, Term::NIL, atom_table)
}

/// Map a wire reason atom onto a runtime exit reason.
///
/// Unknown atoms coerce to [`ExitReason::Error`] — deliberately lethal for
/// non-trapping targets, hostile-input-only (beamr peers emit only the six
/// known atoms); dropping the frame would lose a death signal. Non-atom
/// reason terms are rejected by the decode arms before reaching this.
pub(crate) fn exit_reason_from_wire(atom: Atom) -> ExitReason {
    match atom {
        Atom::NORMAL => ExitReason::Normal,
        Atom::KILL => ExitReason::Kill,
        Atom::KILLED => ExitReason::Killed,
        Atom::NOCONNECTION => ExitReason::NoConnection,
        Atom::NOPROC => ExitReason::NoProc,
        _ => ExitReason::Error,
    }
}

/// Full addressed decode of a control ETF term: opcode match for ALL arms.
///
/// SEND/REG_SEND are inline; PG_UPDATE delegates to `control::decode_pg_update`;
/// LINK/UNLINK/EXIT/EXIT2 are the link-control arms. When `local_node` is
/// `Some`:
///
/// - a SEND whose to-pid carries node `Some(n) != local` decodes to
///   [`ControlError::MisAddressed`] (R6 fix); a node-less to-pid is tolerated
///   for legacy peers and treated as local;
/// - a LINK/UNLINK/EXIT/EXIT2 whose `to` node differs from `local` decodes to
///   [`ControlError::MisAddressed`].
///
/// A node-less pid in either position of a LINK/UNLINK/EXIT/EXIT2 is
/// [`ControlError::InvalidControl`] (strict: only new beamr emits these ops).
pub fn decode_control_addressed(
    control_etf: &[u8],
    atom_table: &AtomTable,
    local_node: Option<Atom>,
) -> Result<ControlMessage, ControlError> {
    let mut process = Process::new(0, 64);
    let mut context = ProcessContext::new();
    context.attach_process(&mut process, 0);
    let term = decode_term(control_etf, &mut context, atom_table)?;
    let tuple = Tuple::new(term).ok_or(ControlError::InvalidControl)?;
    match tuple.get(0).and_then(Term::as_small_int) {
        Some(SEND) if tuple.arity() == 3 => {
            let to = PidRef::new(tuple.get(2).ok_or(ControlError::InvalidControl)?)
                .ok_or(ControlError::InvalidControl)?;
            // R6: an external to-pid naming another node is a routing error.
            // A node-less to-pid (no node, or the wire's `nonode@nohost`
            // marker — local pids encode as NEW_PID_EXT carrying that name)
            // is tolerated for legacy peers and treated as local.
            if let (Some(local), Some(node)) = (local_node, to.node())
                && node != local
                && !is_node_less_marker(atom_table, node)
            {
                return Err(ControlError::MisAddressed);
            }
            Ok(ControlMessage::Send {
                to_pid: to.pid_number(),
            })
        }
        Some(REG_SEND) if tuple.arity() == 4 => {
            let to_name = tuple
                .get(3)
                .and_then(Term::as_atom)
                .ok_or(ControlError::InvalidControl)?;
            Ok(ControlMessage::RegSend { to_name })
        }
        Some(PG_UPDATE) if tuple.arity() == 5 => control::decode_pg_update(&tuple),
        Some(LINK) if tuple.arity() == 3 => {
            let (from, to_pid) = link_endpoints(&tuple, atom_table, local_node)?;
            Ok(ControlMessage::Link { from, to_pid })
        }
        Some(UNLINK) if tuple.arity() == 3 => {
            let (from, to_pid) = link_endpoints(&tuple, atom_table, local_node)?;
            Ok(ControlMessage::Unlink { from, to_pid })
        }
        Some(EXIT) if tuple.arity() == 4 => {
            let (from, to_pid) = link_endpoints(&tuple, atom_table, local_node)?;
            let reason = wire_reason(&tuple)?;
            Ok(ControlMessage::Exit {
                from,
                to_pid,
                reason,
            })
        }
        Some(EXIT2) if tuple.arity() == 4 => {
            let (from, to_pid) = link_endpoints(&tuple, atom_table, local_node)?;
            let reason = wire_reason(&tuple)?;
            Ok(ControlMessage::Exit2 {
                from,
                to_pid,
                reason,
            })
        }
        _ => Err(ControlError::InvalidControl),
    }
}

/// Extract `(from, to_pid)` from a `{Op, FromExtPid, ToExtPid, ..}` tuple.
///
/// Both pids must be external and carry a real node name — a node-less pid
/// (no node, or the wire's `nonode@nohost` marker) in either position rejects
/// the frame (strict: only new beamr emits these ops, and it always encodes
/// both endpoints as node-carrying external pids). A `to` naming a node other
/// than `local_node` is misaddressed.
fn link_endpoints(
    tuple: &Tuple,
    atom_table: &AtomTable,
    local_node: Option<Atom>,
) -> Result<(RemotePid, u64), ControlError> {
    let from = PidRef::new(tuple.get(1).ok_or(ControlError::InvalidControl)?)
        .ok_or(ControlError::InvalidControl)?
        .remote_pid()
        .ok_or(ControlError::InvalidControl)?;
    if is_node_less_marker(atom_table, from.node) {
        return Err(ControlError::InvalidControl);
    }
    let to = PidRef::new(tuple.get(2).ok_or(ControlError::InvalidControl)?)
        .ok_or(ControlError::InvalidControl)?;
    let to_node = to.node().ok_or(ControlError::InvalidControl)?;
    if is_node_less_marker(atom_table, to_node) {
        return Err(ControlError::InvalidControl);
    }
    if let Some(local) = local_node
        && to_node != local
    {
        return Err(ControlError::MisAddressed);
    }
    Ok((from, to.pid_number()))
}

/// True when `node` is the wire marker for a node-less (local immediate) pid.
///
/// Local pids have no on-wire node-less representation: the ETF encoder writes
/// them as NEW_PID_EXT carrying the `nonode@nohost` node name, so an addressed
/// decode must recognise that name as "no node" rather than as a foreign node.
fn is_node_less_marker(atom_table: &AtomTable, node: Atom) -> bool {
    atom_table.resolve(node) == Some(crate::etf::encode::NONODE_NOHOST)
}

/// Extract the 4th tuple element as a wire reason atom. Non-atom reason terms
/// are shape corruption and reject the frame.
fn wire_reason(tuple: &Tuple) -> Result<ExitReason, ControlError> {
    let atom = tuple
        .get(3)
        .and_then(Term::as_atom)
        .ok_or(ControlError::InvalidControl)?;
    Ok(exit_reason_from_wire(atom))
}

/// Scheduler-side sink for inbound link controls (mirror of [`PgDelivery`]).
pub trait LinkControlDelivery: Send + Sync {
    /// Apply an inbound LINK from `from` to local pid `to_pid`.
    fn apply_link(&self, from: RemotePid, to_pid: u64);
    /// Apply an inbound UNLINK from `from` to local pid `to_pid`.
    fn apply_unlink(&self, from: RemotePid, to_pid: u64);
    /// op 3: no-op unless a remote link from `from` exists on the target (DC-4).
    fn apply_link_exit(&self, from: RemotePid, to_pid: u64, reason: ExitReason);
    /// op 8: exit-signal rules regardless of links; never touches link state.
    fn apply_exit2(&self, from: RemotePid, to_pid: u64, reason: ExitReason);
}

/// Sink bundle — absorbs future sink families (monitors) without signature
/// churn.
pub struct ControlSinks<'a> {
    /// Mailbox delivery target for SEND/REG_SEND payloads.
    pub delivery: &'a dyn ControlDelivery,
    /// Registered-name resolver for REG_SEND; `None` drops REG_SEND frames.
    pub registry: Option<&'a dyn ControlRegistry>,
    /// Process-group sink for PG_UPDATE; `None` drops pg frames.
    pub pg: Option<&'a dyn PgDelivery>,
    /// Link-control sink for LINK/UNLINK/EXIT/EXIT2; `None` drops them.
    pub links: Option<&'a dyn LinkControlDelivery>,
    /// Authenticated peer the frame arrived from. When `Some`, a
    /// LINK/UNLINK/EXIT/EXIT2 frame whose `from.node` differs is dropped
    /// (`Ok(false)`) — from-forgery rejection.
    pub origin_node: Option<Atom>,
    /// This node's name; enables [`ControlError::MisAddressed`] validation (R6).
    pub local_node: Option<Atom>,
}

/// Decode a control frame and route it to the matching sink.
///
/// SEND/REG_SEND/PG_UPDATE arms behave exactly as the legacy `handle_frame`
/// path. LINK/UNLINK/EXIT/EXIT2: origin check first, then route to `links`
/// (`None` => `Ok(false)`).
pub fn dispatch_frame(
    control_etf: &[u8],
    payload_etf: &[u8],
    atom_table: &AtomTable,
    sinks: &ControlSinks<'_>,
) -> Result<bool, ControlError> {
    match decode_control_addressed(control_etf, atom_table, sinks.local_node)? {
        ControlMessage::Send { to_pid } => Ok(sinks.delivery.deliver_payload(to_pid, payload_etf)),
        ControlMessage::RegSend { to_name } => {
            let Some(registry) = sinks.registry else {
                return Ok(false);
            };
            let Some(pid) = registry.whereis(to_name) else {
                return Ok(false);
            };
            Ok(sinks.delivery.deliver_payload(pid, payload_etf))
        }
        ControlMessage::PgJoin {
            scope,
            group,
            node,
            pid_number,
            serial,
        } => {
            let Some(pg) = sinks.pg else {
                return Ok(false);
            };
            pg.apply_pg_join(scope, group, node, pid_number, serial);
            Ok(true)
        }
        ControlMessage::PgLeave {
            scope,
            group,
            node,
            pid_number,
            serial,
        } => {
            let Some(pg) = sinks.pg else {
                return Ok(false);
            };
            pg.apply_pg_leave(scope, group, node, pid_number, serial);
            Ok(true)
        }
        ControlMessage::Link { from, to_pid } => {
            let Some(links) = link_sink(sinks, from) else {
                return Ok(false);
            };
            links.apply_link(from, to_pid);
            Ok(true)
        }
        ControlMessage::Unlink { from, to_pid } => {
            let Some(links) = link_sink(sinks, from) else {
                return Ok(false);
            };
            links.apply_unlink(from, to_pid);
            Ok(true)
        }
        ControlMessage::Exit {
            from,
            to_pid,
            reason,
        } => {
            let Some(links) = link_sink(sinks, from) else {
                return Ok(false);
            };
            links.apply_link_exit(from, to_pid, reason);
            Ok(true)
        }
        ControlMessage::Exit2 {
            from,
            to_pid,
            reason,
        } => {
            let Some(links) = link_sink(sinks, from) else {
                return Ok(false);
            };
            links.apply_exit2(from, to_pid, reason);
            Ok(true)
        }
    }
}

/// Origin check first (from-forgery rejection, `Ok(false)` drop), then the
/// link sink (`None` => `Ok(false)` drop).
fn link_sink<'a>(sinks: &ControlSinks<'a>, from: RemotePid) -> Option<&'a dyn LinkControlDelivery> {
    if let Some(origin) = sinks.origin_node
        && from.node != origin
    {
        return None;
    }
    sinks.links
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::distribution::control::{SPAWN_REPLY, encode_send_frame, split_frame};
    use crate::etf::encode::encode_term;
    use crate::term::boxed::write_external_pid;

    fn remote(node: Atom, pid_number: u64, serial: u64) -> RemotePid {
        RemotePid {
            node,
            pid_number,
            serial,
        }
    }

    /// Build a raw control frame from a control tuple constructed on a
    /// temporary process heap by `build`.
    fn hostile_frame(
        atom_table: &AtomTable,
        build: impl FnOnce(&mut ProcessContext) -> Term,
    ) -> Vec<u8> {
        let mut process = Process::new(0, 256);
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);
        let control = build(&mut context);
        control::encode_frame(control, Term::NIL, atom_table).expect("frame encodes")
    }

    fn ext_pid(context: &mut ProcessContext, node: Atom, pid_number: u64, serial: u64) -> Term {
        context
            .alloc_external_pid(node, pid_number, serial)
            .expect("external pid allocates")
    }

    // ── opcode table / reservation ──────────────────────────────────────

    #[test]
    fn beamr_private_opcodes_are_outside_otp_control_range() {
        // Both beamr-private opcodes must stay above the OTP table (which tops
        // out at SPAWN_REPLY = 31) so they can never collide with a standard
        // control message; `from_opcode` must reject them.
        const _: () = assert!(PG_UPDATE > SPAWN_REPLY && BEAMR_MONITOR > SPAWN_REPLY);
        assert!(ControlOp::from_opcode(BEAMR_MONITOR).is_none());
        assert!(ControlOp::from_opcode(PG_UPDATE).is_none());
    }

    #[test]
    fn control_op_round_trips_all_tabled_opcodes() {
        for op in [
            ControlOp::Link,
            ControlOp::Send,
            ControlOp::Exit,
            ControlOp::Unlink,
            ControlOp::RegSend,
            ControlOp::Exit2,
            ControlOp::MonitorP,
            ControlOp::DemonitorP,
            ControlOp::MonitorPExit,
            ControlOp::SpawnRequest,
            ControlOp::SpawnReply,
        ] {
            assert_eq!(ControlOp::from_opcode(op.opcode()), Some(op));
        }
        assert_eq!(ControlOp::from_opcode(255), None);
    }

    // ── exact wire bytes (§1.1) ─────────────────────────────────────────

    #[test]
    fn link_frame_encodes_the_pinned_exact_bytes() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        let frame = encode_link_frame(node_a, 42, remote(node_b, 7, 3), &atom_table)
            .expect("link frame encodes");

        #[rustfmt::skip]
        let control: &[u8] = &[
            0x83,                                     // VERSION
            0x68, 0x03,                               // SMALL_TUPLE_EXT, arity 3
            0x61, 0x01,                               // SMALL_INTEGER_EXT 1 (opcode)
            0x58,                                     // NEW_PID_EXT (FromPid)
            0x77, 0x06, b'a', b'@', b'h', b'o', b's', b't',
            0x00, 0x00, 0x00, 0x2A,                   //   id = 42
            0x00, 0x00, 0x00, 0x00,                   //   serial = 0
            0x00, 0x00, 0x00, 0x00,                   //   creation = 0
            0x58,                                     // NEW_PID_EXT (ToPid)
            0x77, 0x06, b'b', b'@', b'h', b'o', b's', b't',
            0x00, 0x00, 0x00, 0x07,                   //   id = 7
            0x00, 0x00, 0x00, 0x03,                   //   serial = 3
            0x00, 0x00, 0x00, 0x00,                   //   creation = 0
        ];
        let payload: &[u8] = &[0x83, 0x6A]; // NIL
        let mut expected = Vec::new();
        expected.extend_from_slice(&(control.len() as u32).to_be_bytes());
        expected.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        expected.extend_from_slice(control);
        expected.extend_from_slice(payload);
        assert_eq!(control.len(), 47);
        assert_eq!(frame, expected);
    }

    // ── round-trips ─────────────────────────────────────────────────────

    #[test]
    fn link_frame_round_trips_through_split_and_addressed_decode() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        let frame = encode_link_frame(node_a, 42, remote(node_b, 7, 3), &atom_table)
            .expect("link frame encodes");
        let (control, payload) = split_frame(&frame).expect("frame splits");
        assert_eq!(
            payload,
            encode_term(Term::NIL, &atom_table)
                .expect("nil encodes")
                .as_slice()
        );

        let decoded =
            decode_control_addressed(control, &atom_table, Some(node_b)).expect("control decodes");
        assert_eq!(
            decoded,
            ControlMessage::Link {
                from: remote(node_a, 42, 0),
                to_pid: 7,
            }
        );
    }

    #[test]
    fn unlink_frame_round_trips_through_split_and_addressed_decode() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        let frame = encode_unlink_frame(node_a, 5, remote(node_b, 9, 1), &atom_table)
            .expect("unlink frame encodes");
        let (control, _payload) = split_frame(&frame).expect("frame splits");

        let decoded =
            decode_control_addressed(control, &atom_table, Some(node_b)).expect("control decodes");
        assert_eq!(
            decoded,
            ControlMessage::Unlink {
                from: remote(node_a, 5, 0),
                to_pid: 9,
            }
        );
    }

    #[test]
    fn exit_frame_round_trips_carrying_the_terminal_reason() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        let frame = encode_exit_frame(
            ControlOp::Exit,
            node_a,
            8,
            remote(node_b, 11, 0),
            ExitReason::Killed,
            &atom_table,
        )
        .expect("exit frame encodes");
        let (control, _payload) = split_frame(&frame).expect("frame splits");

        let decoded =
            decode_control_addressed(control, &atom_table, Some(node_b)).expect("control decodes");
        assert_eq!(
            decoded,
            ControlMessage::Exit {
                from: remote(node_a, 8, 0),
                to_pid: 11,
                reason: ExitReason::Killed,
            }
        );
    }

    #[test]
    fn exit2_frame_round_trips_carrying_a_raw_kill() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        let frame = encode_exit_frame(
            ControlOp::Exit2,
            node_a,
            3,
            remote(node_b, 4, 2),
            ExitReason::Kill,
            &atom_table,
        )
        .expect("exit2 frame encodes");
        let (control, _payload) = split_frame(&frame).expect("frame splits");

        let decoded =
            decode_control_addressed(control, &atom_table, Some(node_b)).expect("control decodes");
        assert_eq!(
            decoded,
            ControlMessage::Exit2 {
                from: remote(node_a, 3, 0),
                to_pid: 4,
                reason: ExitReason::Kill,
            }
        );
    }

    #[test]
    fn exit_encoder_rejects_non_exit_ops() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        for op in [ControlOp::Link, ControlOp::Unlink, ControlOp::Send] {
            assert_eq!(
                encode_exit_frame(
                    op,
                    node_a,
                    1,
                    remote(node_b, 2, 0),
                    ExitReason::Error,
                    &atom_table,
                ),
                Err(EncodeError::UnsupportedTerm)
            );
        }
    }

    // ── reason mapping (ruling 10) ──────────────────────────────────────

    #[test]
    fn exit_reason_from_wire_maps_known_atoms_and_coerces_unknown_to_error() {
        let atom_table = AtomTable::with_common_atoms();
        assert_eq!(exit_reason_from_wire(Atom::NORMAL), ExitReason::Normal);
        assert_eq!(exit_reason_from_wire(Atom::KILL), ExitReason::Kill);
        assert_eq!(exit_reason_from_wire(Atom::KILLED), ExitReason::Killed);
        assert_eq!(exit_reason_from_wire(Atom::ERROR), ExitReason::Error);
        assert_eq!(
            exit_reason_from_wire(Atom::NOCONNECTION),
            ExitReason::NoConnection
        );
        assert_eq!(exit_reason_from_wire(Atom::NOPROC), ExitReason::NoProc);
        // Unknown atom reasons coerce to Error — deliberately lethal,
        // hostile-input-only; dropping the frame would lose a death signal.
        let weird = atom_table.intern("weird_reason");
        assert_eq!(exit_reason_from_wire(weird), ExitReason::Error);
    }

    #[test]
    fn exit_decode_coerces_an_unknown_reason_atom_to_error() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");
        let weird = atom_table.intern("weird_reason");

        let frame = hostile_frame(&atom_table, |context| {
            let from = ext_pid(context, node_a, 1, 0);
            let to = ext_pid(context, node_b, 2, 0);
            context
                .alloc_tuple(&[Term::small_int(EXIT), from, to, Term::atom(weird)])
                .expect("tuple allocates")
        });
        let (control, _payload) = split_frame(&frame).expect("frame splits");

        assert_eq!(
            decode_control_addressed(control, &atom_table, Some(node_b)),
            Ok(ControlMessage::Exit {
                from: remote(node_a, 1, 0),
                to_pid: 2,
                reason: ExitReason::Error,
            })
        );
    }

    // ── hostile / malformed frames (scenario 9, decode half) ───────────

    #[test]
    fn unknown_opcode_is_invalid_control() {
        let atom_table = AtomTable::with_common_atoms();
        let node_b = atom_table.intern("b@host");

        let frame = hostile_frame(&atom_table, |context| {
            let from = ext_pid(context, node_b, 1, 0);
            let to = ext_pid(context, node_b, 2, 0);
            context
                .alloc_tuple(&[Term::small_int(77), from, to])
                .expect("tuple allocates")
        });
        let (control, _payload) = split_frame(&frame).expect("frame splits");

        assert_eq!(
            decode_control_addressed(control, &atom_table, Some(node_b)),
            Err(ControlError::InvalidControl)
        );
    }

    #[test]
    fn wrong_arities_are_invalid_control() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        // LINK with an EXIT-shaped arity-4 tuple.
        let link_arity_4 = hostile_frame(&atom_table, |context| {
            let from = ext_pid(context, node_a, 1, 0);
            let to = ext_pid(context, node_b, 2, 0);
            context
                .alloc_tuple(&[Term::small_int(LINK), from, to, Term::atom(Atom::NORMAL)])
                .expect("tuple allocates")
        });
        // EXIT with a LINK-shaped arity-3 tuple (reason missing).
        let exit_arity_3 = hostile_frame(&atom_table, |context| {
            let from = ext_pid(context, node_a, 1, 0);
            let to = ext_pid(context, node_b, 2, 0);
            context
                .alloc_tuple(&[Term::small_int(EXIT), from, to])
                .expect("tuple allocates")
        });

        for frame in [link_arity_4, exit_arity_3] {
            let (control, _payload) = split_frame(&frame).expect("frame splits");
            assert_eq!(
                decode_control_addressed(control, &atom_table, Some(node_b)),
                Err(ControlError::InvalidControl)
            );
        }
    }

    #[test]
    fn node_less_pids_on_link_controls_are_invalid_control() {
        // A local immediate pid has no node-less wire representation: the ETF
        // encoder writes it as NEW_PID_EXT carrying `nonode@nohost`, which the
        // addressed decode recognises as the node-less marker and rejects on
        // link controls (strict — only new beamr emits these ops, and it
        // always encodes node-carrying external pids in both positions).
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        // Node-less (local immediate) `from`.
        let local_from = hostile_frame(&atom_table, |context| {
            let to = ext_pid(context, node_b, 2, 0);
            context
                .alloc_tuple(&[Term::small_int(LINK), Term::pid(1), to])
                .expect("tuple allocates")
        });
        // Node-less (local immediate) `to` — strict even without a local node.
        let local_to = hostile_frame(&atom_table, |context| {
            let from = ext_pid(context, node_a, 1, 0);
            context
                .alloc_tuple(&[Term::small_int(UNLINK), from, Term::pid(2)])
                .expect("tuple allocates")
        });

        for frame in [local_from, local_to] {
            let (control, _payload) = split_frame(&frame).expect("frame splits");
            assert_eq!(
                decode_control_addressed(control, &atom_table, Some(node_b)),
                Err(ControlError::InvalidControl),
            );
            assert_eq!(
                decode_control_addressed(control, &atom_table, None),
                Err(ControlError::InvalidControl),
            );
        }
    }

    #[test]
    fn in_memory_node_less_pids_are_rejected_by_the_endpoint_extractor() {
        // Defense-in-depth for the (wire-unreachable) truly node-less case:
        // `PidRef::node()` returning `None` rejects the tuple directly.
        let atom_table = AtomTable::with_common_atoms();
        let node_b = atom_table.intern("b@host");
        let mut process = Process::new(0, 64);
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);

        let ext = ext_pid(&mut context, node_b, 2, 0);
        let from_local = context
            .alloc_tuple(&[Term::small_int(LINK), Term::pid(1), ext])
            .expect("tuple allocates");
        let to_local = context
            .alloc_tuple(&[Term::small_int(LINK), ext, Term::pid(1)])
            .expect("tuple allocates");
        for term in [from_local, to_local] {
            let tuple = Tuple::new(term).expect("tuple view");
            assert_eq!(
                link_endpoints(&tuple, &atom_table, Some(node_b)),
                Err(ControlError::InvalidControl)
            );
        }
    }

    #[test]
    fn non_atom_exit_reason_is_shape_corruption() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");

        for op in [EXIT, EXIT2] {
            let frame = hostile_frame(&atom_table, |context| {
                let from = ext_pid(context, node_a, 1, 0);
                let to = ext_pid(context, node_b, 2, 0);
                let reason = context
                    .alloc_tuple(&[Term::atom(Atom::ERROR), Term::small_int(1)])
                    .expect("reason tuple allocates");
                context
                    .alloc_tuple(&[Term::small_int(op), from, to, reason])
                    .expect("tuple allocates")
            });
            let (control, _payload) = split_frame(&frame).expect("frame splits");
            assert_eq!(
                decode_control_addressed(control, &atom_table, Some(node_b)),
                Err(ControlError::InvalidControl)
            );
        }
    }

    // ── misaddressing (R6) ──────────────────────────────────────────────

    #[test]
    fn link_control_addressed_to_a_third_node_is_misaddressed() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");
        let node_c = atom_table.intern("c@host");

        // LINK and EXIT frames whose `to` names c@host, decoded on b@host.
        let link = encode_link_frame(node_a, 1, remote(node_c, 2, 0), &atom_table)
            .expect("link frame encodes");
        let exit = encode_exit_frame(
            ControlOp::Exit,
            node_a,
            1,
            remote(node_c, 2, 0),
            ExitReason::Error,
            &atom_table,
        )
        .expect("exit frame encodes");

        for frame in [link, exit] {
            let (control, _payload) = split_frame(&frame).expect("frame splits");
            assert_eq!(
                decode_control_addressed(control, &atom_table, Some(node_b)),
                Err(ControlError::MisAddressed)
            );
        }
    }

    #[test]
    fn send_to_an_external_pid_naming_another_node_is_misaddressed() {
        let atom_table = AtomTable::with_common_atoms();
        let node_b = atom_table.intern("b@host");
        let node_c = atom_table.intern("c@host");

        let mut heap = [0_u64; 4];
        let to = write_external_pid(&mut heap, node_c, 7, 0).expect("external pid fits");
        let frame = encode_send_frame(Term::atom(Atom::OK), to, Term::atom(Atom::OK), &atom_table)
            .expect("send frame encodes");
        let (control, _payload) = split_frame(&frame).expect("frame splits");

        assert_eq!(
            decode_control_addressed(control, &atom_table, Some(node_b)),
            Err(ControlError::MisAddressed)
        );
        // Without a local node the legacy decode path applies (no validation).
        assert_eq!(
            decode_control_addressed(control, &atom_table, None),
            Ok(ControlMessage::Send { to_pid: 7 })
        );
    }

    #[test]
    fn send_to_a_node_less_pid_still_delivers_for_legacy_tolerance() {
        let atom_table = AtomTable::with_common_atoms();
        let node_b = atom_table.intern("b@host");

        let frame = encode_send_frame(
            Term::atom(Atom::OK),
            Term::pid(7),
            Term::atom(Atom::OK),
            &atom_table,
        )
        .expect("send frame encodes");
        let (control, _payload) = split_frame(&frame).expect("frame splits");

        assert_eq!(
            decode_control_addressed(control, &atom_table, Some(node_b)),
            Ok(ControlMessage::Send { to_pid: 7 })
        );
    }

    // ── decode budget (§1.2) ────────────────────────────────────────────

    #[test]
    fn worst_case_link_control_fits_the_64_word_decode_heap() {
        // Arithmetic: the worst new arm (EXIT/EXIT2) is an arity-4 tuple
        // (5 words) plus two boxed external pids (4 words each) = 13 << 64.
        const TUPLE_WORDS: usize = 5;
        const EXTERNAL_PID_WORDS: usize = 4;
        const _: () = assert!(TUPLE_WORDS + 2 * EXTERNAL_PID_WORDS <= 64);

        // Live check through the actual 64-word temp process, with the
        // longest node names a SMALL_ATOM_UTF8_EXT can carry mattering only
        // for byte length, not word count.
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a-rather-long-node-name@some-long-host.example.com");
        let node_b = atom_table.intern("b-rather-long-node-name@some-long-host.example.com");
        let frame = encode_exit_frame(
            ControlOp::Exit,
            node_a,
            u64::from(u32::MAX),
            remote(node_b, u64::from(u32::MAX), u64::from(u32::MAX)),
            ExitReason::NoConnection,
            &atom_table,
        )
        .expect("exit frame encodes");
        let (control, _payload) = split_frame(&frame).expect("frame splits");
        assert!(decode_control_addressed(control, &atom_table, Some(node_b)).is_ok());
    }

    // ── dispatch routing / forgery (scenario 9) ─────────────────────────

    struct NoopDelivery;

    impl ControlDelivery for NoopDelivery {
        fn deliver_payload(&self, _target_pid: u64, _payload_etf: &[u8]) -> bool {
            true
        }
    }

    /// `(op, from, to_pid, reason)` recorded per link-control application.
    type LinkRecord = (ControlOp, RemotePid, u64, Option<ExitReason>);

    #[derive(Default)]
    struct RecordingLinks {
        events: Mutex<Vec<LinkRecord>>,
    }

    impl RecordingLinks {
        fn events(&self) -> Vec<LinkRecord> {
            self.events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }

        fn record(&self, record: LinkRecord) {
            self.events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(record);
        }
    }

    impl LinkControlDelivery for RecordingLinks {
        fn apply_link(&self, from: RemotePid, to_pid: u64) {
            self.record((ControlOp::Link, from, to_pid, None));
        }

        fn apply_unlink(&self, from: RemotePid, to_pid: u64) {
            self.record((ControlOp::Unlink, from, to_pid, None));
        }

        fn apply_link_exit(&self, from: RemotePid, to_pid: u64, reason: ExitReason) {
            self.record((ControlOp::Exit, from, to_pid, Some(reason)));
        }

        fn apply_exit2(&self, from: RemotePid, to_pid: u64, reason: ExitReason) {
            self.record((ControlOp::Exit2, from, to_pid, Some(reason)));
        }
    }

    #[test]
    fn dispatch_routes_all_four_link_controls_to_the_links_sink() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");
        let delivery = NoopDelivery;
        let links = RecordingLinks::default();
        let sinks = ControlSinks {
            delivery: &delivery,
            registry: None,
            pg: None,
            links: Some(&links),
            origin_node: Some(node_a),
            local_node: Some(node_b),
        };

        let frames = [
            encode_link_frame(node_a, 1, remote(node_b, 2, 0), &atom_table)
                .expect("link frame encodes"),
            encode_unlink_frame(node_a, 1, remote(node_b, 2, 0), &atom_table)
                .expect("unlink frame encodes"),
            encode_exit_frame(
                ControlOp::Exit,
                node_a,
                1,
                remote(node_b, 2, 0),
                ExitReason::Error,
                &atom_table,
            )
            .expect("exit frame encodes"),
            encode_exit_frame(
                ControlOp::Exit2,
                node_a,
                1,
                remote(node_b, 2, 0),
                ExitReason::Kill,
                &atom_table,
            )
            .expect("exit2 frame encodes"),
        ];
        for frame in &frames {
            let (control, payload) = split_frame(frame).expect("frame splits");
            assert_eq!(
                dispatch_frame(control, payload, &atom_table, &sinks),
                Ok(true)
            );
        }

        let from = remote(node_a, 1, 0);
        assert_eq!(
            links.events(),
            vec![
                (ControlOp::Link, from, 2, None),
                (ControlOp::Unlink, from, 2, None),
                (ControlOp::Exit, from, 2, Some(ExitReason::Error)),
                (ControlOp::Exit2, from, 2, Some(ExitReason::Kill)),
            ]
        );
    }

    #[test]
    fn dispatch_drops_link_controls_whose_from_forges_another_origin() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");
        let node_c = atom_table.intern("c@host");
        let delivery = NoopDelivery;
        let links = RecordingLinks::default();
        // The authenticated origin is c@host, but the frame's `from` says a@host.
        let sinks = ControlSinks {
            delivery: &delivery,
            registry: None,
            pg: None,
            links: Some(&links),
            origin_node: Some(node_c),
            local_node: Some(node_b),
        };

        let frame = encode_link_frame(node_a, 1, remote(node_b, 2, 0), &atom_table)
            .expect("link frame encodes");
        let (control, payload) = split_frame(&frame).expect("frame splits");

        assert_eq!(
            dispatch_frame(control, payload, &atom_table, &sinks),
            Ok(false)
        );
        assert!(links.events().is_empty(), "forged frame must not apply");
    }

    #[test]
    fn dispatch_without_a_links_sink_drops_link_controls() {
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");
        let delivery = NoopDelivery;
        let sinks = ControlSinks {
            delivery: &delivery,
            registry: None,
            pg: None,
            links: None,
            origin_node: None,
            local_node: None,
        };

        let frame = encode_exit_frame(
            ControlOp::Exit,
            node_a,
            1,
            remote(node_b, 2, 0),
            ExitReason::Error,
            &atom_table,
        )
        .expect("exit frame encodes");
        let (control, payload) = split_frame(&frame).expect("frame splits");

        assert_eq!(
            dispatch_frame(control, payload, &atom_table, &sinks),
            Ok(false)
        );
    }

    #[test]
    fn legacy_handle_frame_now_drops_link_controls_instead_of_erroring() {
        // Ruling 9 behavior delta: opcodes 1/3/4/8 decode and return
        // Ok(false) through the compat wrapper (no links sink), not
        // Err(InvalidControl).
        let atom_table = AtomTable::with_common_atoms();
        let node_a = atom_table.intern("a@host");
        let node_b = atom_table.intern("b@host");
        let delivery = NoopDelivery;

        let frame = encode_link_frame(node_a, 1, remote(node_b, 2, 0), &atom_table)
            .expect("link frame encodes");
        let (control, payload) = split_frame(&frame).expect("frame splits");

        assert_eq!(
            control::handle_frame(control, payload, &atom_table, &delivery, None, None),
            Ok(false)
        );
    }
}
