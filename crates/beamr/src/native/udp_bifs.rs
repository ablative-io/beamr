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
use crate::term::binary::Binary;
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
    let bytes = Binary::new(*data).ok_or_else(badarg)?.as_bytes().to_vec();
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
    context.request_suspend(timeout_ms);
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
            let binary = context.alloc_binary(datagram)?;
            let payload = context.alloc_tuple(&[ip, port, binary])?;
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
