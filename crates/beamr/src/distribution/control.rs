//! Distribution control message decoding and remote spawn helpers.

use crate::atom::Atom;
use crate::native::ProcessContext;
use crate::native::spawn::{SpawnError, SpawnFacility, SpawnOptions};
use crate::term::Term;
use crate::term::boxed::Tuple;
use crate::term::pid_ref::PidRef;

/// Distribution process-control operation codes used by Erlang distribution.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ControlOp {
    Link = 1,
    Send = 2,
    Exit = 3,
    Unlink = 4,
    RegSend = 6,
    Exit2 = 8,
    MonitorP = 19,
    DemonitorP = 20,
    MonitorPExit = 21,
    SpawnRequest = 29,
    SpawnReply = 31,
}

impl ControlOp {
    /// Convert a wire opcode to a known control operation.
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
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

    /// Return the numeric wire opcode.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Module/function/arguments entry point carried by SPAWN_REQUEST.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteMfa {
    pub module: Atom,
    pub function: Atom,
    pub args: Vec<Term>,
}

/// Parsed SPAWN_REQUEST control message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpawnRequest {
    pub request_id: u64,
    pub from: Term,
    pub group_leader: Term,
    pub mfa: RemoteMfa,
    pub options: SpawnOptions,
}

/// Parsed SPAWN_REPLY control message.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SpawnReply {
    pub request_id: u64,
    pub pid: Term,
}

/// Error returned while parsing a distribution control term.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ControlDecodeError {
    NotTuple,
    BadArity,
    UnknownOp,
    BadRequestId,
    BadMfa,
    BadOptions,
    BadPid,
}

/// Error returned by a local SPAWN_REQUEST handler.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SpawnRequestError {
    Decode(ControlDecodeError),
    MissingCallerPid,
    Spawn(SpawnError),
    PidOutOfRange,
}

/// Decode a `{29, ReqId, From, GroupLeader, {M,F,A}, OptList}` control term.
pub fn decode_spawn_request(
    term: Term,
    context: &ProcessContext<'_>,
) -> Result<SpawnRequest, ControlDecodeError> {
    let tuple = Tuple::new(term).ok_or(ControlDecodeError::NotTuple)?;
    if tuple.arity() != 6 {
        return Err(ControlDecodeError::BadArity);
    }
    let op = tuple
        .get(0)
        .and_then(|term| term.as_small_int())
        .and_then(|value| u8::try_from(value).ok())
        .and_then(ControlOp::from_u8)
        .ok_or(ControlDecodeError::UnknownOp)?;
    if op != ControlOp::SpawnRequest {
        return Err(ControlDecodeError::UnknownOp);
    }
    let request_id = parse_non_negative_u64(tuple.get(1).ok_or(ControlDecodeError::BadRequestId)?)
        .ok_or(ControlDecodeError::BadRequestId)?;
    let from = tuple.get(2).ok_or(ControlDecodeError::BadArity)?;
    let group_leader = tuple.get(3).ok_or(ControlDecodeError::BadArity)?;
    let mfa = parse_mfa(tuple.get(4).ok_or(ControlDecodeError::BadMfa)?)?;
    let options =
        parse_remote_spawn_options(tuple.get(5).ok_or(ControlDecodeError::BadOptions)?, context)?;

    Ok(SpawnRequest {
        request_id,
        from,
        group_leader,
        mfa,
        options,
    })
}

/// Decode a `{31, ReqId, Pid}` control term.
pub fn decode_spawn_reply(term: Term) -> Result<SpawnReply, ControlDecodeError> {
    let tuple = Tuple::new(term).ok_or(ControlDecodeError::NotTuple)?;
    if tuple.arity() != 3 {
        return Err(ControlDecodeError::BadArity);
    }
    let op = tuple
        .get(0)
        .and_then(|term| term.as_small_int())
        .and_then(|value| u8::try_from(value).ok())
        .and_then(ControlOp::from_u8)
        .ok_or(ControlDecodeError::UnknownOp)?;
    if op != ControlOp::SpawnReply {
        return Err(ControlDecodeError::UnknownOp);
    }
    let request_id = parse_non_negative_u64(tuple.get(1).ok_or(ControlDecodeError::BadRequestId)?)
        .ok_or(ControlDecodeError::BadRequestId)?;
    let pid = tuple.get(2).ok_or(ControlDecodeError::BadPid)?;
    if PidRef::new(pid).is_none() {
        return Err(ControlDecodeError::BadPid);
    }
    Ok(SpawnReply { request_id, pid })
}

/// Allocate a SPAWN_REQUEST control tuple on `context`'s process heap.
pub fn alloc_spawn_request(
    context: &mut ProcessContext<'_>,
    request: &SpawnRequest,
) -> Result<Term, Term> {
    let args = context.alloc_list(&request.mfa.args)?;
    let mfa = context.alloc_tuple(&[
        Term::atom(request.mfa.module),
        Term::atom(request.mfa.function),
        args,
    ])?;
    let opt_list = spawn_options_to_list(context, request.options)?;
    let op = Term::try_small_int(i64::from(ControlOp::SpawnRequest.as_u8())).ok_or_else(badarg)?;
    let req_id = Term::try_small_int(i64::try_from(request.request_id).map_err(|_| badarg())?)
        .ok_or_else(badarg)?;
    context.alloc_tuple(&[
        op,
        req_id,
        request.from,
        request.group_leader,
        mfa,
        opt_list,
    ])
}

/// Allocate a SPAWN_REPLY control tuple on `context`'s process heap.
pub fn alloc_spawn_reply(
    context: &mut ProcessContext<'_>,
    request_id: u64,
    pid: Term,
) -> Result<Term, Term> {
    let op = Term::try_small_int(i64::from(ControlOp::SpawnReply.as_u8())).ok_or_else(badarg)?;
    let req_id =
        Term::try_small_int(i64::try_from(request_id).map_err(|_| badarg())?).ok_or_else(badarg)?;
    context.alloc_tuple(&[op, req_id, pid])
}

/// Handle a decoded SPAWN_REQUEST by spawning locally with link/monitor options applied atomically.
///
/// The current scheduler spawn API is local-caller based; until remote link/monitor
/// metadata is represented in the scheduler, this uses the supplied local service
/// caller PID as the atomic-options owner rather than pretending the external
/// `From` PID is local.
pub fn handle_spawn_request(
    term: Term,
    context: &mut ProcessContext<'_>,
    spawn_facility: &dyn SpawnFacility,
) -> Result<Term, SpawnRequestError> {
    let request = decode_spawn_request(term, context).map_err(SpawnRequestError::Decode)?;
    let caller_pid = context.pid().ok_or(SpawnRequestError::MissingCallerPid)?;
    let result = spawn_facility
        .spawn_with_options(
            caller_pid,
            request.mfa.module,
            request.mfa.function,
            request.mfa.args,
            request.options,
        )
        .map_err(SpawnRequestError::Spawn)?;
    let pid_term = Term::try_pid(result.pid).ok_or(SpawnRequestError::PidOutOfRange)?;
    alloc_spawn_reply(context, request.request_id, pid_term)
        .map_err(|_| SpawnRequestError::PidOutOfRange)
}

fn parse_mfa(term: Term) -> Result<RemoteMfa, ControlDecodeError> {
    let tuple = Tuple::new(term).ok_or(ControlDecodeError::BadMfa)?;
    if tuple.arity() != 3 {
        return Err(ControlDecodeError::BadMfa);
    }
    let module = tuple
        .get(0)
        .and_then(|term| term.as_atom())
        .ok_or(ControlDecodeError::BadMfa)?;
    let function = tuple
        .get(1)
        .and_then(|term| term.as_atom())
        .ok_or(ControlDecodeError::BadMfa)?;
    let args = list_to_vec(tuple.get(2).ok_or(ControlDecodeError::BadMfa)?)
        .ok_or(ControlDecodeError::BadMfa)?;
    Ok(RemoteMfa {
        module,
        function,
        args,
    })
}

fn parse_remote_spawn_options(
    term: Term,
    context: &ProcessContext<'_>,
) -> Result<SpawnOptions, ControlDecodeError> {
    let atom_table = context.atom_table().ok_or(ControlDecodeError::BadOptions)?;
    let link_atom = atom_table.intern("link");
    let monitor_atom = atom_table.intern("monitor");
    let mut options = SpawnOptions::default();
    for option in list_to_vec(term).ok_or(ControlDecodeError::BadOptions)? {
        if option.as_atom() == Some(link_atom) {
            options.link = true;
        } else if option.as_atom() == Some(monitor_atom) {
            options.monitor = true;
        } else {
            return Err(ControlDecodeError::BadOptions);
        }
    }
    Ok(options)
}

fn spawn_options_to_list(
    context: &mut ProcessContext<'_>,
    options: SpawnOptions,
) -> Result<Term, Term> {
    let atom_table = context.atom_table().ok_or_else(badarg)?;
    let mut elements = Vec::new();
    if options.link {
        elements.push(Term::atom(atom_table.intern("link")));
    }
    if options.monitor {
        elements.push(Term::atom(atom_table.intern("monitor")));
    }
    context.alloc_list(&elements)
}

fn list_to_vec(term: Term) -> Option<Vec<Term>> {
    let mut elements = Vec::new();
    let mut current = term;
    loop {
        if current.is_nil() {
            return Some(elements);
        }
        let cons = crate::term::boxed::Cons::new(current)?;
        elements.push(cons.head());
        current = cons.tail();
    }
}

fn parse_non_negative_u64(term: Term) -> Option<u64> {
    let value = term.as_small_int()?;
    if value < 0 {
        return None;
    }
    u64::try_from(value).ok()
}

fn badarg() -> Term {
    Term::atom(crate::atom::Atom::BADARG)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::AtomTable;
    use crate::native::spawn::{SpawnMonitorResult, SpawnOptionsResult};
    use crate::process::Process;
    use crate::term::boxed::{Tuple, write_cons, write_external_pid, write_tuple};
    use std::sync::{Arc, Mutex};

    struct MockSpawnFacility {
        next_pid: u64,
        records: Mutex<Vec<(u64, Atom, Atom, Vec<Term>, SpawnOptions)>>,
    }

    impl MockSpawnFacility {
        fn new(next_pid: u64) -> Self {
            Self {
                next_pid,
                records: Mutex::new(Vec::new()),
            }
        }
    }

    impl SpawnFacility for MockSpawnFacility {
        fn spawn(
            &self,
            _caller_pid: u64,
            _module: Atom,
            _function: Atom,
            _args: Vec<Term>,
            _link_to: Option<u64>,
        ) -> Result<u64, SpawnError> {
            Ok(self.next_pid)
        }

        fn spawn_monitor(
            &self,
            _caller_pid: u64,
            _module: Atom,
            _function: Atom,
            _args: Vec<Term>,
        ) -> Result<SpawnMonitorResult, SpawnError> {
            Ok(SpawnMonitorResult {
                pid: self.next_pid,
                reference: 0,
            })
        }

        fn spawn_lambda(
            &self,
            _caller_pid: u64,
            _module: Atom,
            _lambda_index: u32,
            _link_to: Option<u64>,
        ) -> Result<u64, SpawnError> {
            Ok(self.next_pid)
        }

        fn spawn_lambda_monitor(
            &self,
            _caller_pid: u64,
            _module: Atom,
            _lambda_index: u32,
        ) -> Result<SpawnMonitorResult, SpawnError> {
            Ok(SpawnMonitorResult {
                pid: self.next_pid,
                reference: 0,
            })
        }

        fn spawn_with_options(
            &self,
            caller_pid: u64,
            module: Atom,
            function: Atom,
            args: Vec<Term>,
            options: SpawnOptions,
        ) -> Result<SpawnOptionsResult, SpawnError> {
            self.records
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((caller_pid, module, function, args, options));
            Ok(SpawnOptionsResult {
                pid: self.next_pid,
                reference: options.monitor.then_some(1),
            })
        }

        fn spawn_lambda_with_options(
            &self,
            _caller_pid: u64,
            _module: Atom,
            _lambda_index: u32,
            _options: SpawnOptions,
        ) -> Result<SpawnOptionsResult, SpawnError> {
            Ok(SpawnOptionsResult {
                pid: self.next_pid,
                reference: None,
            })
        }
    }

    #[test]
    fn decodes_spawn_request_with_link_monitor_options() {
        let atoms = Arc::new(AtomTable::with_common_atoms());
        let module = atoms.intern("sample");
        let function = atoms.intern("run");
        let link = atoms.intern("link");
        let monitor = atoms.intern("monitor");
        let mut process = Process::new(1, 128);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(atoms));
        context.attach_process(&mut process, 0);

        let mut arg_list_heap = [0_u64; 2];
        let arg_list =
            write_cons(&mut arg_list_heap, Term::small_int(7), Term::NIL).expect("arg list fits");
        let mut mfa_heap = [0_u64; 4];
        let mfa = write_tuple(
            &mut mfa_heap,
            &[Term::atom(module), Term::atom(function), arg_list],
        )
        .expect("mfa tuple fits");
        let mut opt2_heap = [0_u64; 2];
        let opt_tail = write_cons(&mut opt2_heap, Term::atom(monitor), Term::NIL)
            .expect("monitor option fits");
        let mut opt1_heap = [0_u64; 2];
        let opt_list =
            write_cons(&mut opt1_heap, Term::atom(link), opt_tail).expect("link option fits");
        let mut from_heap = [0_u64; 4];
        let from = write_external_pid(&mut from_heap, module, 99, 0).expect("from pid fits");
        let mut gl_heap = [0_u64; 4];
        let group_leader =
            write_external_pid(&mut gl_heap, module, 1, 0).expect("group leader fits");
        let mut request_heap = [0_u64; 7];
        let request_term = write_tuple(
            &mut request_heap,
            &[
                Term::small_int(29),
                Term::small_int(42),
                from,
                group_leader,
                mfa,
                opt_list,
            ],
        )
        .expect("request tuple fits");

        let request = decode_spawn_request(request_term, &context).expect("spawn request decodes");

        assert_eq!(request.request_id, 42);
        assert_eq!(request.from, from);
        assert_eq!(request.group_leader, group_leader);
        assert_eq!(request.mfa.module, module);
        assert_eq!(request.mfa.function, function);
        assert_eq!(request.mfa.args, vec![Term::small_int(7)]);
        assert!(request.options.link);
        assert!(request.options.monitor);
        assert_eq!(request.options.priority, None);
        assert_eq!(request.options.min_heap_size, None);
    }

    #[test]
    fn handle_spawn_request_creates_local_process_and_reply() {
        let atoms = Arc::new(AtomTable::with_common_atoms());
        let module = atoms.intern("sample");
        let function = atoms.intern("run");
        let link = atoms.intern("link");
        let mut process = Process::new(100, 128);
        let mut context = ProcessContext::new();
        context.set_pid(Some(100));
        context.set_atom_table(Some(atoms));
        context.attach_process(&mut process, 0);

        let mut mfa_heap = [0_u64; 4];
        let mfa = write_tuple(
            &mut mfa_heap,
            &[Term::atom(module), Term::atom(function), Term::NIL],
        )
        .expect("mfa tuple fits");
        let mut opt_heap = [0_u64; 2];
        let opt_list =
            write_cons(&mut opt_heap, Term::atom(link), Term::NIL).expect("option list fits");
        let mut request_heap = [0_u64; 7];
        let request = write_tuple(
            &mut request_heap,
            &[
                Term::small_int(29),
                Term::small_int(5),
                Term::pid(100),
                Term::pid(100),
                mfa,
                opt_list,
            ],
        )
        .expect("request tuple fits");
        let facility = MockSpawnFacility::new(321);

        let reply = handle_spawn_request(request, &mut context, &facility).expect("spawn handled");
        let decoded = decode_spawn_reply(reply).expect("reply decodes");

        assert_eq!(decoded.request_id, 5);
        assert_eq!(decoded.pid, Term::pid(321));
        let records = facility
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].0, 100);
        assert_eq!(records[0].1, module);
        assert_eq!(records[0].2, function);
        assert!(records[0].4.link);
        assert!(!records[0].4.monitor);
    }

    #[test]
    fn alloc_spawn_reply_encodes_op_31() {
        let atoms = Arc::new(AtomTable::with_common_atoms());
        let mut process = Process::new(1, 128);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(atoms));
        context.attach_process(&mut process, 0);

        let reply = alloc_spawn_reply(&mut context, 77, Term::pid(9)).expect("reply allocated");
        let tuple = Tuple::new(reply).expect("reply tuple");

        assert_eq!(tuple.arity(), 3);
        assert_eq!(tuple.get(0), Some(Term::small_int(31)));
        assert_eq!(tuple.get(1), Some(Term::small_int(77)));
        assert_eq!(tuple.get(2), Some(Term::pid(9)));
    }
}
