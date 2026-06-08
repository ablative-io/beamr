//! Completion-ring backed TCP client BIFs.

use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::os::fd::{IntoRawFd, RawFd};
use std::sync::Arc;

use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::atom::{Atom, AtomTable};
use crate::io::resource::{FdInner, FdResource, FdState};
use crate::io::{IoOp, IoResult, errno_to_atom};
use crate::native::{
    BifRegistryImpl, Capability, FileIoCompletion, FileIoContinuation, NativeRegistrationError,
    ProcessContext,
};
use crate::term::Term;
use crate::term::binary::Binary;
use crate::term::boxed::{Cons, Tuple};

const CURRENT_POSITION: u64 = u64::MAX;
const RECV_AVAILABLE_SIZE: usize = 8192;

/// Registers Erlang TCP BIFs.
pub fn register_tcp_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");
    for (name, arity, function) in [
        ("tcp_connect", 3, tcp_connect as crate::native::NativeFn),
        ("tcp_send", 2, tcp_send as crate::native::NativeFn),
        ("tcp_recv", 2, tcp_recv as crate::native::NativeFn),
        ("tcp_recv", 3, tcp_recv as crate::native::NativeFn),
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

/// erlang:tcp_connect/3.
pub fn tcp_connect(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if let Some(completion) = context.take_file_io_completion() {
        return finish_connect(completion, context);
    }
    if let Some((_op_id, continuation)) = context.take_pending_file_io() {
        if let FileIoContinuation::TcpConnect { fd } = continuation {
            close_raw_fd(fd);
            return error_tuple(context, Atom::TIMEOUT);
        }
        return error_tuple(context, Atom::UNKNOWN_ERROR);
    }

    let [host, port, options] = args else {
        return Err(badarg());
    };
    let options = parse_connect_options(*options)?;
    let remote_addr = match resolve_host_port(*host, *port) {
        Ok(addr) => addr,
        Err(reason) if reason.is_atom() => {
            if let Some(atom) = reason.as_atom() {
                return error_tuple(context, atom);
            }
            return Err(reason);
        }
        Err(reason) => return Err(reason),
    };
    let fd = match create_stream_socket(remote_addr, &options) {
        Ok(fd) => fd,
        Err(error) => return error_tuple(context, error_reason(error)),
    };
    context.submit_file_io_with_timeout(
        IoOp::Connect {
            fd,
            addr: remote_addr,
        },
        FileIoContinuation::TcpConnect { fd },
        options.timeout_ms,
    )?;
    Ok(Term::atom(Atom::OK))
}

/// erlang:tcp_send/2.
pub fn tcp_send(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if let Some(completion) = context.take_file_io_completion() {
        return finish_send(completion, context);
    }

    let [fd_term, data_term] = args else {
        return Err(badarg());
    };
    let resource = match open_fd_resource(*fd_term) {
        Ok(resource) => resource,
        Err(reason) if reason == Term::atom(Atom::CLOSED) => {
            return error_tuple(context, Atom::CLOSED);
        }
        Err(reason) => return Err(reason),
    };
    let data = Binary::new(*data_term)
        .ok_or_else(badarg)?
        .as_bytes()
        .to_vec();
    if data.is_empty() {
        return Ok(Term::atom(Atom::OK));
    }
    if let Err(reason) = submit_send(resource.inner(), data, context) {
        if reason == Term::atom(Atom::CLOSED) {
            return error_tuple(context, Atom::CLOSED);
        }
        return Err(reason);
    }
    Ok(Term::atom(Atom::OK))
}

/// erlang:tcp_recv/2,3.
pub fn tcp_recv(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if let Some(completion) = context.take_file_io_completion() {
        return finish_recv(completion, context);
    }
    if let Some((_op_id, continuation)) = context.take_pending_file_io() {
        if matches!(continuation, FileIoContinuation::TcpRecv { .. }) {
            return error_tuple(context, Atom::TIMEOUT);
        }
        return error_tuple(context, Atom::UNKNOWN_ERROR);
    }

    let (fd_term, len_term, timeout_ms) = match args {
        [fd_term, len_term] => (*fd_term, *len_term, None),
        [fd_term, len_term, timeout] => (*fd_term, *len_term, Some(parse_timeout(*timeout)?)),
        _ => return Err(badarg()),
    };
    let resource = match open_fd_resource(fd_term) {
        Ok(resource) => resource,
        Err(reason) if reason == Term::atom(Atom::CLOSED) => {
            return error_tuple(context, Atom::CLOSED);
        }
        Err(reason) => return Err(reason),
    };
    let requested_len = len_term
        .as_small_int()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(badarg)?;
    if let Err(reason) = submit_recv(
        resource.inner(),
        requested_len,
        Vec::new(),
        timeout_ms,
        context,
    ) {
        if reason == Term::atom(Atom::CLOSED) {
            return error_tuple(context, Atom::CLOSED);
        }
        return Err(reason);
    }
    Ok(Term::atom(Atom::OK))
}

fn finish_connect(
    completion: FileIoCompletion,
    context: &mut ProcessContext,
) -> Result<Term, Term> {
    let fd = match completion.continuation {
        FileIoContinuation::TcpConnect { fd } => fd,
        _ => return error_tuple(context, Atom::UNKNOWN_ERROR),
    };
    match completion.completion.result {
        Ok(IoResult::Connected) => {
            let owner_pid = context.pid().ok_or_else(badarg)?;
            let resource = context.alloc_fd_resource(Arc::new(FdInner::new(fd, owner_pid)))?;
            ok_tuple(context, resource)
        }
        Ok(_) => {
            close_raw_fd(fd);
            error_tuple(context, Atom::UNKNOWN_ERROR)
        }
        Err(error) => {
            close_raw_fd(fd);
            error_tuple(context, error_reason(error))
        }
    }
}

fn finish_send(completion: FileIoCompletion, context: &mut ProcessContext) -> Result<Term, Term> {
    let (fd, remaining) = match completion.continuation {
        FileIoContinuation::TcpSend { fd, remaining } => (fd, remaining),
        _ => return error_tuple(context, Atom::UNKNOWN_ERROR),
    };
    match completion.completion.result {
        Ok(IoResult::BytesWritten(bytes_written)) if bytes_written >= remaining.len() => {
            Ok(Term::atom(Atom::OK))
        }
        Ok(IoResult::BytesWritten(0)) => {
            fd.close_synchronously();
            error_tuple(context, Atom::CLOSED)
        }
        Ok(IoResult::BytesWritten(bytes_written)) => {
            let Some(next) = remaining.get(bytes_written..).map(<[u8]>::to_vec) else {
                return error_tuple(context, Atom::UNKNOWN_ERROR);
            };
            if let Err(reason) = submit_send(fd, next, context) {
                if reason == Term::atom(Atom::CLOSED) {
                    return error_tuple(context, Atom::CLOSED);
                }
                return Err(reason);
            }
            Ok(Term::atom(Atom::OK))
        }
        Ok(_) => error_tuple(context, Atom::UNKNOWN_ERROR),
        Err(error) => {
            let reason = tcp_send_error_reason(&error);
            if reason == Atom::CLOSED {
                fd.close_synchronously();
            }
            error_tuple(context, reason)
        }
    }
}

fn finish_recv(completion: FileIoCompletion, context: &mut ProcessContext) -> Result<Term, Term> {
    let (fd, requested_len, mut accumulated, timeout_ms) = match completion.continuation {
        FileIoContinuation::TcpRecv {
            fd,
            requested_len,
            accumulated,
            timeout_ms,
        } => (fd, requested_len, accumulated, timeout_ms),
        _ => return error_tuple(context, Atom::UNKNOWN_ERROR),
    };
    match completion.completion.result {
        Ok(IoResult::BytesRead(0, _)) => {
            fd.close_synchronously();
            error_tuple(context, Atom::CLOSED)
        }
        Ok(IoResult::BytesRead(bytes_read, bytes)) => {
            let Some(read_bytes) = bytes.get(..bytes_read) else {
                return error_tuple(context, Atom::UNKNOWN_ERROR);
            };
            accumulated.extend_from_slice(read_bytes);
            if requested_len == 0 || accumulated.len() >= requested_len {
                let result_bytes = if requested_len == 0 {
                    accumulated
                } else {
                    accumulated[..requested_len].to_vec()
                };
                let binary = context.alloc_binary(&result_bytes)?;
                ok_tuple(context, binary)
            } else {
                if let Err(reason) =
                    submit_recv(fd, requested_len, accumulated, timeout_ms, context)
                {
                    if reason == Term::atom(Atom::CLOSED) {
                        return error_tuple(context, Atom::CLOSED);
                    }
                    return Err(reason);
                }
                Ok(Term::atom(Atom::OK))
            }
        }
        Ok(_) => error_tuple(context, Atom::UNKNOWN_ERROR),
        Err(error) => error_tuple(context, tcp_recv_error_reason(&error)),
    }
}

fn submit_send(fd: Arc<FdInner>, data: Vec<u8>, context: &mut ProcessContext) -> Result<u64, Term> {
    if fd.state() != FdState::Open {
        return Err(Term::atom(Atom::CLOSED));
    }
    context.submit_file_io(
        IoOp::Write {
            fd: fd.fd(),
            data: data.clone(),
            offset: CURRENT_POSITION,
        },
        FileIoContinuation::TcpSend {
            fd,
            remaining: data,
        },
    )
}

fn submit_recv(
    fd: Arc<FdInner>,
    requested_len: usize,
    accumulated: Vec<u8>,
    timeout_ms: Option<u64>,
    context: &mut ProcessContext,
) -> Result<u64, Term> {
    if fd.state() != FdState::Open {
        return Err(Term::atom(Atom::CLOSED));
    }
    let remaining = requested_len.saturating_sub(accumulated.len());
    let buf_len = if requested_len == 0 {
        RECV_AVAILABLE_SIZE
    } else {
        remaining
    };
    context.submit_file_io_with_timeout(
        IoOp::Read {
            fd: fd.fd(),
            buf_len,
            offset: CURRENT_POSITION,
        },
        FileIoContinuation::TcpRecv {
            fd,
            requested_len,
            accumulated,
            timeout_ms,
        },
        timeout_ms,
    )
}

fn open_fd_resource(term: Term) -> Result<FdResource, Term> {
    let resource = FdResource::new(term).ok_or_else(badarg)?;
    if resource.state() != FdState::Open {
        return Err(Term::atom(Atom::CLOSED));
    }
    Ok(resource)
}

#[derive(Clone, Debug, Default)]
struct ConnectOptions {
    timeout_ms: Option<u64>,
    local_ip: Option<IpAddr>,
    local_port: Option<u16>,
}

fn parse_connect_options(options: Term) -> Result<ConnectOptions, Term> {
    let mut parsed = ConnectOptions::default();
    let mut tail = options;
    while tail != Term::NIL {
        let cons = Cons::new(tail).ok_or_else(badarg)?;
        parse_connect_option(cons.head(), &mut parsed)?;
        tail = cons.tail();
    }
    Ok(parsed)
}

fn parse_connect_option(option: Term, parsed: &mut ConnectOptions) -> Result<(), Term> {
    let tuple = Tuple::new(option).ok_or_else(badarg)?;
    if tuple.arity() != 2 {
        return Err(badarg());
    }
    let key = tuple.get(0).ok_or_else(badarg)?;
    let value = tuple.get(1).ok_or_else(badarg)?;
    if key == Term::atom(Atom::TIMEOUT) {
        parsed.timeout_ms = Some(parse_timeout(value)?);
    } else if key == Term::atom(Atom::IP) {
        parsed.local_ip = Some(parse_ip(value)?);
    } else if key == Term::atom(Atom::PORT) {
        parsed.local_port = Some(parse_port(value)?);
    } else {
        return Err(badarg());
    }
    Ok(())
}

fn parse_timeout(term: Term) -> Result<u64, Term> {
    term.as_small_int()
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(badarg)
}

fn parse_port(term: Term) -> Result<u16, Term> {
    term.as_small_int()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(badarg)
}

fn parse_ip(term: Term) -> Result<IpAddr, Term> {
    let bytes = Binary::new(term).ok_or_else(badarg)?.as_bytes();
    let text = std::str::from_utf8(bytes).map_err(|_| badarg())?;
    text.parse().map_err(|_| badarg())
}

fn resolve_host_port(host: Term, port: Term) -> Result<SocketAddr, Term> {
    let host_bytes = Binary::new(host).ok_or_else(badarg)?.as_bytes();
    let host = std::str::from_utf8(host_bytes).map_err(|_| badarg())?;
    let port = parse_port(port)?;
    let mut addrs = (host, port)
        .to_socket_addrs()
        .map_err(|_| Term::atom(Atom::HOST_NOT_FOUND))?;
    addrs.next().ok_or(Term::atom(Atom::HOST_NOT_FOUND))
}

fn create_stream_socket(addr: SocketAddr, options: &ConnectOptions) -> io::Result<RawFd> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    socket.set_nonblocking(true)?;
    if options.local_ip.is_some() || options.local_port.is_some() {
        let local_addr = SocketAddr::new(
            options.local_ip.unwrap_or_else(|| unspecified_ip(addr)),
            options.local_port.unwrap_or(0),
        );
        socket.bind(&SockAddr::from(local_addr))?;
    }
    Ok(socket.into_raw_fd())
}

fn unspecified_ip(remote: SocketAddr) -> IpAddr {
    match remote {
        SocketAddr::V4(_) => IpAddr::from([0, 0, 0, 0]),
        SocketAddr::V6(_) => IpAddr::from([0_u16; 8]),
    }
}

fn ok_tuple(context: &mut ProcessContext, value: Term) -> Result<Term, Term> {
    context.alloc_tuple(&[Term::atom(Atom::OK), value])
}

fn error_tuple(context: &mut ProcessContext, reason: Atom) -> Result<Term, Term> {
    context.alloc_tuple(&[Term::atom(Atom::ERROR), Term::atom(reason)])
}

fn error_reason(error: io::Error) -> Atom {
    error
        .raw_os_error()
        .map(errno_to_atom)
        .unwrap_or(Atom::UNKNOWN_ERROR)
}

fn tcp_send_error_reason(error: &io::Error) -> Atom {
    match error.raw_os_error() {
        Some(errno) if errno == libc::EPIPE || errno == libc::ECONNRESET => Atom::CLOSED,
        Some(errno) => errno_to_atom(errno),
        None => Atom::UNKNOWN_ERROR,
    }
}

fn tcp_recv_error_reason(error: &io::Error) -> Atom {
    match error.raw_os_error() {
        Some(errno) if errno == libc::ECONNRESET => Atom::CLOSED,
        Some(errno) => errno_to_atom(errno),
        None => Atom::UNKNOWN_ERROR,
    }
}

fn close_raw_fd(fd: RawFd) {
    if fd >= 0 {
        let owner = FdInner::new(fd, 0);
        owner.close_synchronously();
    }
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
#[path = "tcp_bifs_tests.rs"]
mod tests;
