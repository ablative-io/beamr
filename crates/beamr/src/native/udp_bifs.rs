//! Completion-ring backed UDP socket BIFs.

use std::io;
use std::mem;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use crate::atom::{Atom, AtomTable};
use crate::io::resource::{FdInner, FdMode, FdResource, FdState};
use crate::io::{IoOp, IoResult, errno_to_atom};
use crate::native::{
    BifRegistryImpl, Capability, FileIoCompletion, FileIoContinuation, NativeRegistrationError,
    ProcessContext,
};
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;
use crate::term::boxed::{Cons, Tuple};

const DEFAULT_RECV_SIZE: usize = 65_535;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct UdpOpenOptions {
    ip: Ipv4Addr,
    mode: FdMode,
}

impl Default for UdpOpenOptions {
    fn default() -> Self {
        Self {
            ip: Ipv4Addr::new(0, 0, 0, 0),
            mode: FdMode::Passive,
        }
    }
}

/// Registers Erlang UDP BIFs.
pub fn register_udp_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");
    for (name, arity, function) in [
        ("udp_open", 1, udp_open_1 as crate::native::NativeFn),
        ("udp_open", 2, udp_open_2 as crate::native::NativeFn),
        ("udp_send", 4, udp_send as crate::native::NativeFn),
        ("udp_recv", 2, udp_recv_2 as crate::native::NativeFn),
        ("udp_recv", 3, udp_recv_3 as crate::native::NativeFn),
    ] {
        registry.register(
            erlang,
            atom_table.intern(name),
            arity,
            function,
            Capability::ExternalIo,
        )?;
    }
    Ok(())
}

/// erlang:udp_open/1.
pub fn udp_open_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [port] = args else {
        return Err(badarg());
    };
    open_udp_socket(*port, UdpOpenOptions::default(), context)
}

/// erlang:udp_open/2.
pub fn udp_open_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [port, options] = args else {
        return Err(badarg());
    };
    let parsed = parse_open_options(*options, context)?;
    open_udp_socket(*port, parsed, context)
}

/// erlang:udp_send/4.
pub fn udp_send(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if let Some(completion) = context.take_file_io_completion() {
        return finish_udp_send(completion, context);
    }

    let [socket, host, port, data] = args else {
        return Err(badarg());
    };
    let resource = FdResource::new(*socket).ok_or_else(badarg)?;
    if resource.state() != FdState::Open {
        return error_tuple(context, Atom::CLOSED);
    }
    let addr = SocketAddr::V4(SocketAddrV4::new(parse_ipv4(*host)?, parse_port(*port)?));
    let bytes = BinaryRef::new(*data)
        .ok_or_else(badarg)?
        .as_bytes(context.borrow_terms())
        .to_vec();
    let expected_len = bytes.len();
    context.submit_file_io(
        IoOp::SendMsg {
            fd: resource.fd(),
            data: bytes,
            addr,
        },
        FileIoContinuation::UdpSend { expected_len },
    )?;
    Ok(Term::atom(Atom::OK))
}

/// erlang:udp_recv/2.
pub fn udp_recv_2(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    udp_recv_impl(args, None, context)
}

/// erlang:udp_recv/3.
pub fn udp_recv_3(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [socket, length, timeout] = args else {
        return Err(badarg());
    };
    let timeout_ms = parse_timeout(*timeout, context)?;
    udp_recv_impl(&[*socket, *length], timeout_ms, context)
}

fn udp_recv_impl(
    args: &[Term],
    timeout_ms: Option<u64>,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    if let Some(completion) = context.take_file_io_completion() {
        return finish_udp_recv(completion, context);
    }

    let [socket, length] = args else {
        return Err(badarg());
    };
    let resource = FdResource::new(*socket).ok_or_else(badarg)?;
    if resource.state() != FdState::Open {
        return error_tuple(context, Atom::CLOSED);
    }
    if resource.mode() != FdMode::Passive {
        return Err(badarg());
    }
    let buf_len = parse_recv_len(*length)?;
    let ring = context.file_completion_ring().ok_or_else(badarg)?;
    let op_id = ring.submit(IoOp::RecvMsg {
        fd: resource.fd(),
        buf_len,
    });
    context.track_submitted_file_io(op_id, FileIoContinuation::UdpRecv)?;
    context.request_await_suspend(timeout_ms);
    Ok(Term::atom(Atom::OK))
}

fn open_udp_socket(
    port_term: Term,
    options: UdpOpenOptions,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    let port = parse_port(port_term)?;
    let owner_pid = context.pid().ok_or_else(badarg)?;
    let fd =
        create_udp_socket(options.ip, port).map_err(|error| Term::atom(error_reason(error)))?;
    let inner = Arc::new(FdInner::new(fd, owner_pid));
    inner.set_mode(options.mode);
    inner.set_controlling_process(owner_pid);
    let resource = context.alloc_fd_resource(Arc::clone(&inner))?;
    if options.mode != FdMode::Passive {
        submit_active_recv(context, inner)?;
    }
    Ok(resource)
}

fn create_udp_socket(ip: Ipv4Addr, port: u16) -> io::Result<i32> {
    // SAFETY: socket arguments request a plain IPv4 datagram socket and return a new fd on success.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let raw = libc::sockaddr_in {
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
        sin_len: mem::size_of::<libc::sockaddr_in>() as u8,
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from_ne_bytes(ip.octets()),
        },
        sin_zero: [0; 8],
    };
    // SAFETY: `raw` is a valid IPv4 sockaddr alive for the duration of bind.
    let rc = unsafe {
        libc::bind(
            fd,
            (&raw as *const libc::sockaddr_in).cast(),
            mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        let error = io::Error::last_os_error();
        // SAFETY: fd was just created by this function and is not exposed on bind failure.
        let _closed = unsafe { libc::close(fd) };
        Err(error)
    } else {
        Ok(fd)
    }
}

fn submit_active_recv(context: &mut ProcessContext, inner: Arc<FdInner>) -> Result<(), Term> {
    let ring = context.file_completion_ring().ok_or_else(badarg)?;
    let op_id = ring.submit(IoOp::RecvMsg {
        fd: inner.fd(),
        buf_len: DEFAULT_RECV_SIZE,
    });
    context.track_submitted_file_io(op_id, FileIoContinuation::UdpActiveRecv { fd: inner })
}

fn finish_udp_send(
    completion: FileIoCompletion,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    let expected_len = match completion.continuation {
        FileIoContinuation::UdpSend { expected_len } => expected_len,
        _ => return error_tuple(context, Atom::UNKNOWN_ERROR),
    };
    match completion.completion.result {
        Ok(IoResult::DatagramSent(bytes_sent)) if bytes_sent == expected_len => {
            Ok(Term::atom(Atom::OK))
        }
        Ok(IoResult::DatagramSent(bytes_sent)) => incomplete_tuple(context, bytes_sent),
        Ok(_) => error_tuple(context, Atom::UNKNOWN_ERROR),
        Err(error) => error_tuple(context, error_reason(error)),
    }
}

fn finish_udp_recv(
    completion: FileIoCompletion,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    match completion.continuation {
        FileIoContinuation::UdpRecv => {}
        _ => return error_tuple(context, Atom::UNKNOWN_ERROR),
    }
    match completion.completion.result {
        Ok(IoResult::DatagramReceived { bytes, data, addr }) => {
            let SocketAddr::V4(v4) = addr else {
                return error_tuple(context, Atom::EINVAL);
            };
            let datagram = data.get(..bytes).ok_or_else(badarg)?;
            let ip = ipv4_tuple(*v4.ip(), context)?;
            let port = Term::try_small_int(i64::from(v4.port())).ok_or_else(badarg)?;
            // AR-1 site 10. `ip` is a SINGLE boxed tuple — `ipv4_tuple` ends in
            // `alloc_tuple` — held live across the datagram's `alloc_binary`, so
            // it takes `with_rooted` directly rather than the accumulator. `port`
            // is a small int and cannot go stale.
            let payload = context.with_rooted(&[ip], |context, roots| {
                let binary = context.alloc_binary(datagram)?;
                // Re-read AFTER the binary: a collection there forwards `ip`, and
                // the pre-fix body used the stale copy from before.
                let ip = context.rooted(roots, 0)?;
                context.alloc_tuple(&[ip, port, binary])
            })?;
            ok_tuple(context, payload)
        }
        Ok(_) => error_tuple(context, Atom::UNKNOWN_ERROR),
        Err(error) => error_tuple(context, error_reason(error)),
    }
}

fn parse_open_options(options: Term, context: &ProcessContext) -> Result<UdpOpenOptions, Term> {
    let mut parsed = UdpOpenOptions::default();
    let mut tail = options;
    while tail != Term::NIL {
        let cons = Cons::new(tail).ok_or_else(badarg)?;
        let tuple = Tuple::new(cons.head()).ok_or_else(badarg)?;
        if tuple.arity() != 2 {
            return Err(badarg());
        }
        let key = tuple.get(0).ok_or_else(badarg)?;
        let value = tuple.get(1).ok_or_else(badarg)?;
        if atom_name_is(key, "ip", context)? {
            parsed.ip = parse_ipv4(value)?;
        } else if atom_name_is(key, "active", context)? {
            parsed.mode = parse_active(value, context)?;
        } else {
            return Err(badarg());
        }
        tail = cons.tail();
    }
    Ok(parsed)
}

fn parse_active(term: Term, context: &ProcessContext) -> Result<FdMode, Term> {
    if term == Term::atom(Atom::TRUE) {
        Ok(FdMode::Active)
    } else if term == Term::atom(Atom::FALSE) {
        Ok(FdMode::Passive)
    } else if atom_name_is(term, "once", context)? {
        Ok(FdMode::ActiveOnce)
    } else {
        Err(badarg())
    }
}

fn atom_name_is(term: Term, expected: &str, context: &ProcessContext) -> Result<bool, Term> {
    let atom = term.as_atom().ok_or_else(badarg)?;
    Ok(context
        .atom_table()
        .and_then(|table| table.resolve(atom))
        .is_some_and(|name| name == expected))
}

fn parse_ipv4(term: Term) -> Result<Ipv4Addr, Term> {
    let tuple = Tuple::new(term).ok_or_else(badarg)?;
    if tuple.arity() != 4 {
        return Err(badarg());
    }
    let a = parse_octet(tuple.get(0).ok_or_else(badarg)?)?;
    let b = parse_octet(tuple.get(1).ok_or_else(badarg)?)?;
    let c = parse_octet(tuple.get(2).ok_or_else(badarg)?)?;
    let d = parse_octet(tuple.get(3).ok_or_else(badarg)?)?;
    Ok(Ipv4Addr::new(a, b, c, d))
}

fn parse_octet(term: Term) -> Result<u8, Term> {
    term.as_small_int()
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(badarg)
}

fn parse_port(term: Term) -> Result<u16, Term> {
    term.as_small_int()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(badarg)
}

fn parse_recv_len(term: Term) -> Result<usize, Term> {
    let len = term
        .as_small_int()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(badarg)?;
    if len == 0 {
        Ok(DEFAULT_RECV_SIZE)
    } else {
        Ok(len)
    }
}

fn parse_timeout(term: Term, context: &ProcessContext) -> Result<Option<u64>, Term> {
    if atom_name_is(term, "infinity", context).unwrap_or(false) {
        return Ok(None);
    }
    term.as_small_int()
        .and_then(|value| u64::try_from(value).ok())
        .map(Some)
        .ok_or_else(badarg)
}

fn ipv4_tuple(ip: Ipv4Addr, context: &mut ProcessContext) -> Result<Term, Term> {
    let octets = ip.octets();
    let a = Term::try_small_int(i64::from(octets[0])).ok_or_else(badarg)?;
    let b = Term::try_small_int(i64::from(octets[1])).ok_or_else(badarg)?;
    let c = Term::try_small_int(i64::from(octets[2])).ok_or_else(badarg)?;
    let d = Term::try_small_int(i64::from(octets[3])).ok_or_else(badarg)?;
    context.alloc_tuple(&[a, b, c, d])
}

fn ok_tuple(context: &mut ProcessContext, value: Term) -> Result<Term, Term> {
    context.alloc_tuple(&[Term::atom(Atom::OK), value])
}

fn error_tuple(context: &mut ProcessContext, reason: Atom) -> Result<Term, Term> {
    context.alloc_tuple(&[Term::atom(Atom::ERROR), Term::atom(reason)])
}

fn incomplete_tuple(context: &mut ProcessContext, bytes_sent: usize) -> Result<Term, Term> {
    let count = i64::try_from(bytes_sent)
        .ok()
        .and_then(Term::try_small_int)
        .ok_or_else(badarg)?;
    let reason = context.alloc_tuple(&[Term::atom(Atom::INCOMPLETE), count])?;
    context.alloc_tuple(&[Term::atom(Atom::ERROR), reason])
}

fn error_reason(error: io::Error) -> Atom {
    error
        .raw_os_error()
        .map(errno_to_atom)
        .unwrap_or(Atom::UNKNOWN_ERROR)
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;
    use std::time::Duration;

    use crate::io::CompletionRing;
    use crate::io::ring::IoCompletion;
    use crate::native::FileIoFacility;
    use crate::process::Process;

    struct MockFileIoFacility {
        submissions: Mutex<Vec<(u64, IoOp, FileIoContinuation)>>,
    }

    impl MockFileIoFacility {
        fn new() -> Self {
            Self {
                submissions: Mutex::new(Vec::new()),
            }
        }
    }

    impl CompletionRing for MockFileIoFacility {
        fn submit(&self, op: IoOp) -> u64 {
            let mut submissions = self.submissions.lock().expect("submissions lock");
            let op_id = (submissions.len() + 1) as u64;
            submissions.push((0, op, FileIoContinuation::UdpRecv));
            op_id
        }

        fn poll_completions(&self, _timeout: Duration) -> Vec<IoCompletion> {
            Vec::new()
        }

        fn pending_count(&self) -> usize {
            self.submissions.lock().map_or(0, |ops| ops.len())
        }

        fn shutdown(&self) {}
    }

    impl FileIoFacility for MockFileIoFacility {
        fn submit_file_io(&self, pid: u64, op: IoOp, continuation: FileIoContinuation) -> u64 {
            let mut submissions = self.submissions.lock().expect("submissions lock");
            let op_id = (submissions.len() + 1) as u64;
            submissions.push((pid, op, continuation));
            op_id
        }

        fn track_submitted_file_io(&self, pid: u64, op_id: u64, continuation: FileIoContinuation) {
            let mut submissions = self.submissions.lock().expect("submissions lock");
            let index = usize::try_from(op_id.saturating_sub(1)).unwrap_or_default();
            if let Some((stored_pid, _op, stored_continuation)) = submissions.get_mut(index) {
                *stored_pid = pid;
                *stored_continuation = continuation;
            } else {
                submissions.push((pid, IoOp::Nop, continuation));
            }
        }

        fn take_file_io_completion(&self, _pid: u64) -> Option<FileIoCompletion> {
            None
        }

        fn cancel_pending_file_io_for_pid(&self, _pid: u64) {}

        fn ring(&self) -> &dyn CompletionRing {
            self
        }
    }

    fn context_with_process(process: &mut Process) -> ProcessContext<'_> {
        let mut context = ProcessContext::new();
        context.attach_process(process, 0);
        context.set_atom_table(Some(Arc::new(AtomTable::with_common_atoms())));
        context
    }

    fn small(value: i64) -> Term {
        Term::try_small_int(value).unwrap_or(Term::NIL)
    }

    fn tuple(context: &mut ProcessContext, terms: &[Term]) -> Term {
        context.alloc_tuple(terms).unwrap_or(Term::NIL)
    }

    fn list(context: &mut ProcessContext, terms: &[Term]) -> Term {
        let mut tail = Term::NIL;
        for term in terms.iter().rev() {
            tail = context.alloc_cons(*term, tail).unwrap_or(Term::NIL);
        }
        tail
    }

    #[test]
    fn udp_open_zero_returns_passive_fd_resource_bound_to_udp_socket() {
        let mut process = Process::new(10, 512);
        let mut context = context_with_process(&mut process);

        let socket = udp_open_1(&[small(0)], &mut context).expect("udp_open/1");
        let resource = FdResource::new(socket).expect("fd resource");

        assert_eq!(resource.state(), FdState::Open);
        assert_eq!(resource.mode(), FdMode::Passive);
        let mut addr: libc::sockaddr_in = unsafe { mem::zeroed() };
        let mut len = mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockname(
                resource.fd(),
                (&mut addr as *mut libc::sockaddr_in).cast(),
                &mut len,
            )
        };
        assert_eq!(rc, 0);
        assert_eq!(addr.sin_family as i32, libc::AF_INET);
        assert_ne!(u16::from_be(addr.sin_port), 0);
    }

    #[test]
    fn udp_open_parses_ip_and_active_once_options() {
        let mut process = Process::new(11, 512);
        let facility = Arc::new(MockFileIoFacility::new());
        let mut context = context_with_process(&mut process);
        context.set_file_io_facility(Some(facility.clone()));
        let ip_atom = Term::atom(context.atom_table().unwrap().intern("ip"));
        let active_atom = Term::atom(context.atom_table().unwrap().intern("active"));
        let once_atom = Term::atom(context.atom_table().unwrap().intern("once"));
        let ip_value = tuple(&mut context, &[small(127), small(0), small(0), small(1)]);
        let ip_option = tuple(&mut context, &[ip_atom, ip_value]);
        let active_option = tuple(&mut context, &[active_atom, once_atom]);
        let options = list(&mut context, &[ip_option, active_option]);

        let socket = udp_open_2(&[small(0), options], &mut context).expect("udp_open/2");
        let resource = FdResource::new(socket).expect("fd resource");

        assert_eq!(resource.mode(), FdMode::ActiveOnce);
        assert_eq!(resource.owner_pid(), 11);
        let submissions = facility.submissions.lock().expect("submissions lock");
        assert!(submissions.iter().any(|(_, op, continuation)| {
            matches!(
                op,
                IoOp::RecvMsg {
                    buf_len: DEFAULT_RECV_SIZE,
                    ..
                }
            ) && matches!(continuation, FileIoContinuation::UdpActiveRecv { .. })
        }));
    }

    #[test]
    fn udp_recv_zero_length_submits_default_sized_recvmsg() {
        let mut process = Process::new(12, 512);
        let facility = Arc::new(MockFileIoFacility::new());
        let mut context = context_with_process(&mut process);
        context.set_file_io_facility(Some(facility.clone()));
        let socket = udp_open_1(&[small(0)], &mut context).expect("udp_open/1");

        assert_eq!(
            udp_recv_2(&[socket, small(0)], &mut context),
            Ok(Term::atom(Atom::OK))
        );

        let submissions = facility.submissions.lock().expect("submissions lock");
        assert!(submissions.iter().any(|(_, op, continuation)| {
            matches!(
                op,
                IoOp::RecvMsg {
                    buf_len: DEFAULT_RECV_SIZE,
                    ..
                }
            ) && matches!(continuation, FileIoContinuation::UdpRecv)
        }));
    }

    #[test]
    fn registered_udp_bifs_have_external_io_capability() {
        let atom_table = AtomTable::with_common_atoms();
        let registry = BifRegistryImpl::new();

        register_udp_bifs(&registry, &atom_table).expect("register UDP BIFs");

        let erlang = atom_table.intern("erlang");
        for (name, arity) in [
            ("udp_open", 1),
            ("udp_open", 2),
            ("udp_send", 4),
            ("udp_recv", 2),
            ("udp_recv", 3),
        ] {
            let entry = registry
                .lookup(erlang, atom_table.intern(name), arity)
                .expect("registered UDP BIF");
            assert_eq!(entry.capability, Capability::ExternalIo);
        }
    }
}

#[cfg(test)]
mod ar1_row4_site10_tests {
    // ⛔ DEFECT-ASSERTING TESTS — READ THIS BEFORE TRUSTING A GREEN.
    //
    // These pin the MEASURED CORRUPT SURFACE of AR-1 row 4 at f993280. They do
    // NOT assert correct behaviour, so a green here means "the defect is still
    // present, exactly as measured" — never "this site is safe".
    //
    // ⇒ THEY GO RED WHEN AR-1 IS FIXED, AND THAT IS THE POINT. The fix lane
    // INVERTS them to assert correctness rather than deleting them; the pinned
    // counts below are the surface the fix has to move.

    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    use super::finish_udp_recv;
    use crate::atom::Atom;
    use crate::io::ring::{IoCompletion, IoResult};
    use crate::native::ProcessContext;
    use crate::native::context::{FileIoCompletion, FileIoContinuation};
    use crate::process::Process;
    use crate::term::Term;
    // ⚠️ `BinaryRef`, NOT `Binary`. `Binary::new` accepts only BoxedTag::Binary
    // — an INLINE heap binary — and returns None for a ProcBin. Since
    // `alloc_binary` promotes anything over 64 bytes to a ProcBin, a `Binary`
    // reader cannot see the payload of the 1024-byte datagram AT ALL, and
    // reports it as missing at every cell including the no-pressure ones. That
    // is the reader's limit, not the site's defect. `BinaryRef` handles both.
    use crate::term::binary_ref::BinaryRef;
    use crate::term::boxed::Tuple;

    const OCTETS: [u8; 4] = [10, 20, 30, 40];
    const PORT: u16 = 4242;

    /// Which body the cell drives.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Arm {
        Fixed,
        UnrootedReplica,
    }

    /// ⛔⛔ THE SYNTHETIC POSITIVE — `finish_udp_recv`'s DatagramReceived arm
    /// EXACTLY AS IT WAS BEFORE THE FIX, and it must stay that way.
    ///
    /// Only that arm is reproduced: the other arms allocate nothing past an
    /// error atom and cannot witness the defect, so copying them would add bytes
    /// without adding evidence.
    /// ⛔ Do NOT migrate it onto `with_rooted`.
    fn finish_udp_recv_unrooted_replica(
        completion: FileIoCompletion,
        context: &mut ProcessContext,
    ) -> Result<Term, Term> {
        let Ok(IoResult::DatagramReceived { bytes, data, addr }) = completion.completion.result
        else {
            return Err(Term::atom(Atom::BADARG));
        };
        let SocketAddr::V4(v4) = addr else {
            return Err(Term::atom(Atom::BADARG));
        };
        let datagram = data.get(..bytes).ok_or_else(super::badarg)?;
        let ip = super::ipv4_tuple(*v4.ip(), context)?;
        let port = Term::try_small_int(i64::from(v4.port())).ok_or_else(super::badarg)?;
        let binary = context.alloc_binary(datagram)?;
        let payload = context.alloc_tuple(&[ip, port, binary])?;
        super::ok_tuple(context, payload)
    }

    fn udp_round_trip(
        datagram_len: usize,
        heap: usize,
        margin: usize,
        arm: Arm,
    ) -> (usize, Result<(), String>) {
        let mut process = Process::new(10, heap);
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);

        // ⛔ SAME PRE-FILL EXIT AS SITE 5, AND FOR THE SAME MEASURED REASON.
        // The site-5 version of this loop had no give-up condition and SPUN
        // FOREVER at margins 0/1/2: the descent step is one filler allocation
        // (~6 words), so a finer margin is only reachable via a collection,
        // which frees this unrooted filler and pushes `available` back up. The
        // achieved margin is RETURNED so a cell that missed its request is
        // reported at what it actually got.
        let mut filler = Vec::new();
        let mut last_available = usize::MAX;
        let achieved = loop {
            let available = context.process_heap().map(|h| h.available()).unwrap_or(0);
            if available <= margin {
                break available;
            }
            if available >= last_available {
                break available;
            }
            last_available = available;
            match context.alloc_binary(&[0xEF; 32]) {
                Ok(term) => filler.push(term),
                Err(_) => break available,
            }
        };

        let outcome = (|| -> Result<(), String> {
            let data: Vec<u8> = (0..datagram_len)
                .map(|i| u8::try_from(i % 251).unwrap_or(0))
                .collect();
            let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(OCTETS), PORT));
            let completion = FileIoCompletion {
                op_id: 2,
                continuation: FileIoContinuation::UdpRecv,
                completion: IoCompletion {
                    op_id: 2,
                    result: Ok(IoResult::DatagramReceived {
                        bytes: datagram_len,
                        data: data.clone(),
                        addr,
                    }),
                },
            };

            let term = match arm {
                Arm::Fixed => finish_udp_recv(completion, &mut context),
                Arm::UnrootedReplica => finish_udp_recv_unrooted_replica(completion, &mut context),
            }
            .map_err(|_| "finish_udp_recv returned an error term".to_string())?;

            let outer = Tuple::new(term).ok_or_else(|| "result is not a tuple".to_string())?;
            if outer.arity() != 2 {
                return Err(format!("result arity {} not 2", outer.arity()));
            }
            let payload = outer.get(1).ok_or_else(|| "no payload slot".to_string())?;
            let payload =
                Tuple::new(payload).ok_or_else(|| "payload is not a tuple".to_string())?;
            if payload.arity() != 3 {
                return Err(format!("payload arity {} not 3", payload.arity()));
            }

            // THE CARRIER: `ip` was allocated BEFORE the datagram binary and is live
            // across it. If it went stale this is where it shows.
            let ip = payload.get(0).ok_or_else(|| "no ip slot".to_string())?;
            let ip = Tuple::new(ip)
                .ok_or_else(|| "ip is not a tuple — carrier `ip` went stale".to_string())?;
            if ip.arity() != 4 {
                return Err(format!(
                    "ip arity {} not 4 — carrier `ip` went stale",
                    ip.arity()
                ));
            }
            for (index, want) in OCTETS.iter().enumerate() {
                let octet = ip
                    .get(index)
                    .ok_or_else(|| format!("ip octet {index} absent — carrier `ip` went stale"))?;
                let got = octet.as_small_int().ok_or_else(|| {
                    format!("ip octet {index} is not an int — carrier `ip` went stale")
                })?;
                if got != i64::from(*want) {
                    return Err(format!(
                        "ip octet {index} is {got}, expected {want} — carrier `ip` went stale"
                    ));
                }
            }

            let binary = payload.get(2).ok_or_else(|| "no binary slot".to_string())?;
            let binary =
                BinaryRef::new(binary).ok_or_else(|| "payload binary missing".to_string())?;
            if binary.as_bytes(context.borrow_terms()) != data.as_slice() {
                return Err("datagram contents differ".to_string());
            }
            Ok(())
        })();
        (achieved, outcome)
    }

    /// One sweep of the whole size x margin grid against one body. Returns the
    /// corrupt rows, the clean count, the smallest achieved margin, and how many
    /// cells ended with no pressure applied.
    fn sweep(arm: Arm) -> (Vec<String>, usize, usize, usize, usize) {
        let mut cells = Vec::new();
        // Datagram sizes chosen either side of REFC_BINARY_THRESHOLD (64), since
        // that is where the heap cost stops scaling with the payload.
        for datagram_len in [32usize, 64, 1024] {
            for margin in [0usize, 1, 2, 4, 8, 12, 16, 24, 32, 64, 128, 512] {
                let (achieved, result) = udp_round_trip(datagram_len, 2048, margin, arm);
                let verdict = match result {
                    Ok(()) => "ok".to_string(),
                    Err(reason) => reason,
                };
                let line = format!(
                    "[{arm:?}] datagram {datagram_len:>5} margin req {margin:>4} got \
                     {achieved:>5} : {verdict}"
                );
                eprintln!("{line}");
                cells.push((datagram_len, achieved, verdict));
            }
        }

        let corrupted: Vec<String> = cells
            .iter()
            .filter(|(_, _, v)| v != "ok" && !v.contains("returned an error term"))
            .map(|(len, achieved, v)| format!("datagram {len} achieved margin {achieved}: {v}"))
            .collect();
        let clean = cells.iter().filter(|(_, _, v)| v == "ok").count();
        let floor = cells
            .iter()
            .map(|(_, achieved, _)| *achieved)
            .min()
            .unwrap_or(0);
        // A clean cell whose ACHIEVED margin is near the full heap had no
        // pressure applied and is not evidence of anything about the site.
        let no_pressure = cells
            .iter()
            .filter(|(_, achieved, _)| *achieved > 2048 / 2)
            .count();

        eprintln!(
            "site 10 [{arm:?}]: {} corrupt, {clean} clean; pre-fill floor {floor} words; \
             {no_pressure} of {} cells ended with more than half the heap free (NO pressure \
             applied, not evidence)",
            corrupted.len(),
            cells.len()
        );
        for row in &corrupted {
            eprintln!("site 10 [{arm:?}] RED {row}");
        }

        (corrupted, clean, floor, no_pressure, cells.len())
    }

    /// AR-1 row 4, site 10 (`ip`) — ✅ INVERTED.
    ///
    /// The control arm is asserted FIRST and keeps the pre-fix probe's claims:
    /// some cell must be clean (or the reader is broken rather than the site
    /// defective) and some cell must corrupt (or the sweep failed to apply
    /// pressure, which is UNRESOLVED and not a defence). The pre-fill floor and
    /// the no-pressure count are reported for BOTH arms so a reader can see the
    /// fixed arm's cleanliness was measured under the same pressure, not under
    /// none.
    #[test]
    fn ar1_site10_finish_udp_recv_band() {
        // ⛔⛔ POSITIVE CONTROL FIRST, and it licenses everything below it.
        let (control_red, control_clean, _, control_no_pressure, control_cells) =
            sweep(Arm::UnrootedReplica);

        assert!(
            control_clean > 0,
            "control: some cell must be clean, or the reader is broken rather than the site \
             defective"
        );
        assert!(
            !control_red.is_empty(),
            "POSITIVE CONTROL DEAD: the unrooted replica no longer corrupts the carrier at any \
             cell. The pressure regime is gone, so the fixed arm's success below would mean \
             nothing."
        );
        assert!(
            control_no_pressure < control_cells,
            "POSITIVE CONTROL IS VACUOUS: all {control_cells} cells ended with more than half \
             the heap free, so the sweep never applied pressure to anything"
        );

        // ✅ THE CLAIM. Same grid, same heap, through the rooted body.
        let (fixed_red, fixed_clean, _, fixed_no_pressure, fixed_cells) = sweep(Arm::Fixed);

        assert!(
            fixed_red.is_empty(),
            "site 10 is NOT rooted: {} cells still lost the carrier, while the replica corrupted \
             {} in the same run.\n{}",
            fixed_red.len(),
            control_red.len(),
            fixed_red.join("\n")
        );
        assert!(
            fixed_clean > 0,
            "site 10: the fixed arm produced no clean cell at all — a dead reader, not a \
             defended site"
        );
        assert!(
            fixed_no_pressure < fixed_cells,
            "site 10: every fixed-arm cell ended with more than half the heap free, so its \
             cleanliness was measured under NO pressure and proves nothing"
        );
    }
}
