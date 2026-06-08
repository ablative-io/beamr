//! Distribution control message framing and SEND/REG_SEND handling.

use std::fmt;

use crate::atom::{Atom, AtomTable};
use crate::etf::decode::{DecodeError, decode_term};
use crate::etf::encode::{EncodeError, encode_term};
use crate::native::ProcessContext;
use crate::process::Process;
use crate::term::Term;
use crate::term::boxed::Tuple;
use crate::term::pid_ref::PidRef;

/// Distribution control opcode for PID-to-PID send.
pub const SEND: i64 = 2;
/// Distribution control opcode for registered-name send.
pub const REG_SEND: i64 = 6;

/// Error raised when a remote send cannot be completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributionSendError {
    /// The target node has no usable distribution connection.
    NoConnection,
    /// The target PID or message cannot be encoded for distribution.
    Encode,
}

impl fmt::Display for DistributionSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConnection => formatter.write_str("noconnection"),
            Self::Encode => formatter.write_str("distribution encode failed"),
        }
    }
}

/// Facility used by opcodes and BIFs to send a message to a remote PID.
pub trait DistributionSendFacility: Send + Sync {
    /// Encode and send `message` to `target` on its remote node.
    fn send_remote(&self, target: Term, message: Term) -> Result<(), DistributionSendError>;
}

/// Scheduler-safe delivery target for incoming decoded control messages.
pub trait ControlDelivery: Send + Sync {
    /// Decode `payload_etf` for `target_pid` and enqueue it in the target mailbox.
    fn deliver_payload(&self, target_pid: u64, payload_etf: &[u8]) -> bool;
}

/// Registry lookup used by incoming REG_SEND controls.
pub trait ControlRegistry: Send + Sync {
    /// Resolve a registered atom name to a local pid.
    fn whereis(&self, name: Atom) -> Option<u64>;
}

/// Decoded distribution control message.
///
/// Fields are extracted values rather than raw Terms because the decode
/// process heap is temporary — boxed Terms would become dangling after return.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlMessage {
    /// `{2, Cookie, ToPid}` — stores extracted pid number.
    Send { to_pid: u64 },
    /// `{6, FromPid, Cookie, ToName}` — stores extracted name atom.
    RegSend { to_name: Atom },
}

/// Errors while decoding or handling a distribution control frame.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ControlError {
    /// The frame prefix or lengths were invalid.
    InvalidFrame,
    /// ETF decoding failed.
    Decode(DecodeError),
    /// Control tuple shape was not SEND or REG_SEND.
    InvalidControl,
}

impl From<DecodeError> for ControlError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

/// Encode a framed SEND control and payload.
pub fn encode_send_frame(
    cookie: Term,
    to_pid: Term,
    message: Term,
    atom_table: &AtomTable,
) -> Result<Vec<u8>, EncodeError> {
    let mut process = Process::new(0, 32);
    let mut context = ProcessContext::new();
    context.attach_process(&mut process, 0);
    let control = context
        .alloc_tuple(&[Term::small_int(SEND), cookie, to_pid])
        .map_err(|_| EncodeError::UnsupportedTerm)?;
    encode_frame(control, message, atom_table)
}

/// Encode a framed REG_SEND control and payload.
pub fn encode_reg_send_frame(
    from_pid: Term,
    cookie: Term,
    to_name: Atom,
    message: Term,
    atom_table: &AtomTable,
) -> Result<Vec<u8>, EncodeError> {
    let mut process = Process::new(0, 32);
    let mut context = ProcessContext::new();
    context.attach_process(&mut process, 0);
    let control = context
        .alloc_tuple(&[
            Term::small_int(REG_SEND),
            from_pid,
            cookie,
            Term::atom(to_name),
        ])
        .map_err(|_| EncodeError::UnsupportedTerm)?;
    encode_frame(control, message, atom_table)
}

fn encode_frame(
    control: Term,
    message: Term,
    atom_table: &AtomTable,
) -> Result<Vec<u8>, EncodeError> {
    let control_etf = encode_term(control, atom_table)?;
    let payload_etf = encode_term(message, atom_table)?;
    let control_len = u32::try_from(control_etf.len()).map_err(|_| EncodeError::UnsupportedTerm)?;
    let payload_len = u32::try_from(payload_etf.len()).map_err(|_| EncodeError::UnsupportedTerm)?;
    let mut frame = Vec::with_capacity(8 + control_etf.len() + payload_etf.len());
    frame.extend_from_slice(&control_len.to_be_bytes());
    frame.extend_from_slice(&payload_len.to_be_bytes());
    frame.extend_from_slice(&control_etf);
    frame.extend_from_slice(&payload_etf);
    Ok(frame)
}

/// Split a frame produced by [`encode_send_frame`] or [`encode_reg_send_frame`].
pub fn split_frame(frame: &[u8]) -> Result<(&[u8], &[u8]), ControlError> {
    let header = frame.get(..8).ok_or(ControlError::InvalidFrame)?;
    let control_len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    let payload_len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let control_start = 8_usize;
    let payload_start = control_start
        .checked_add(control_len)
        .ok_or(ControlError::InvalidFrame)?;
    let end = payload_start
        .checked_add(payload_len)
        .ok_or(ControlError::InvalidFrame)?;
    if end != frame.len() {
        return Err(ControlError::InvalidFrame);
    }
    let control = frame
        .get(control_start..payload_start)
        .ok_or(ControlError::InvalidFrame)?;
    let payload = frame
        .get(payload_start..end)
        .ok_or(ControlError::InvalidFrame)?;
    Ok((control, payload))
}

/// Decode a control ETF term.
pub fn decode_control(
    control_etf: &[u8],
    atom_table: &AtomTable,
) -> Result<ControlMessage, ControlError> {
    let mut process = Process::new(0, 64);
    let mut context = ProcessContext::new();
    context.attach_process(&mut process, 0);
    let term = decode_term(control_etf, &mut context, atom_table)?;
    let tuple = Tuple::new(term).ok_or(ControlError::InvalidControl)?;
    match tuple.get(0).and_then(Term::as_small_int) {
        Some(SEND) if tuple.arity() == 3 => {
            let to = tuple.get(2).ok_or(ControlError::InvalidControl)?;
            let to_pid = PidRef::new(to)
                .ok_or(ControlError::InvalidControl)?
                .pid_number();
            Ok(ControlMessage::Send { to_pid })
        }
        Some(REG_SEND) if tuple.arity() == 4 => {
            let to_name = tuple
                .get(3)
                .and_then(Term::as_atom)
                .ok_or(ControlError::InvalidControl)?;
            Ok(ControlMessage::RegSend { to_name })
        }
        _ => Err(ControlError::InvalidControl),
    }
}

/// Handle an incoming frame by decoding the control term and delivering the payload.
pub fn handle_frame(
    control_etf: &[u8],
    payload_etf: &[u8],
    atom_table: &AtomTable,
    delivery: &dyn ControlDelivery,
    registry: Option<&dyn ControlRegistry>,
) -> Result<bool, ControlError> {
    match decode_control(control_etf, atom_table)? {
        ControlMessage::Send { to_pid } => {
            Ok(delivery.deliver_payload(to_pid, payload_etf))
        }
        ControlMessage::RegSend { to_name } => {
            let Some(registry) = registry else {
                return Ok(false);
            };
            let Some(pid) = registry.whereis(to_name) else {
                return Ok(false);
            };
            Ok(delivery.deliver_payload(pid, payload_etf))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    struct TestDelivery {
        messages: Mutex<HashMap<u64, Vec<Term>>>,
        atom_table: AtomTable,
    }

    impl TestDelivery {
        fn new() -> Self {
            Self {
                messages: Mutex::new(HashMap::new()),
                atom_table: AtomTable::with_common_atoms(),
            }
        }
    }

    impl ControlDelivery for TestDelivery {
        fn deliver_payload(&self, target_pid: u64, payload_etf: &[u8]) -> bool {
            let mut process = Process::new(target_pid, 64);
            let mut context = ProcessContext::new();
            context.attach_process(&mut process, 0);
            let Ok(message) = decode_term(payload_etf, &mut context, &self.atom_table) else {
                return false;
            };
            self.messages
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .entry(target_pid)
                .or_default()
                .push(message);
            true
        }
    }

    struct TestRegistry(Atom, u64);

    impl ControlRegistry for TestRegistry {
        fn whereis(&self, name: Atom) -> Option<u64> {
            (name == self.0).then_some(self.1)
        }
    }

    #[test]
    fn send_control_delivers_payload_to_pid() {
        let atom_table = AtomTable::with_common_atoms();
        let frame = encode_send_frame(
            Term::atom(Atom::OK),
            Term::pid(7),
            Term::atom(Atom::OK),
            &atom_table,
        )
        .expect("frame encodes");
        let (control, payload) = split_frame(&frame).expect("frame splits");
        let delivery = TestDelivery::new();

        assert_eq!(
            handle_frame(control, payload, &atom_table, &delivery, None),
            Ok(true)
        );
        let messages = delivery
            .messages
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(
            messages.get(&7).and_then(|values| values.first()).copied(),
            Some(Term::atom(Atom::OK))
        );
    }

    #[test]
    fn reg_send_control_resolves_name_before_delivery() {
        let atom_table = AtomTable::with_common_atoms();
        let name = atom_table.intern("receiver");
        let frame = encode_reg_send_frame(
            Term::pid(1),
            Term::atom(Atom::OK),
            name,
            Term::atom(Atom::OK),
            &atom_table,
        )
        .expect("frame encodes");
        let (control, payload) = split_frame(&frame).expect("frame splits");
        let delivery = TestDelivery::new();
        let registry = TestRegistry(name, 9);

        assert_eq!(
            handle_frame(control, payload, &atom_table, &delivery, Some(&registry)),
            Ok(true)
        );
        let messages = delivery
            .messages
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(
            messages.get(&9).and_then(|values| values.first()).copied(),
            Some(Term::atom(Atom::OK))
        );
    }
}
