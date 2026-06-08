use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::fd::RawFd;
use std::sync::{Arc, Mutex};

use crate::atom::{Atom, AtomTable};
use crate::io::resource::{FdInner, FdResource, FdState};
use crate::io::{CompletionRing, IoCompletion, IoOp, IoResult};
use crate::native::{
    BifRegistryImpl, FileIoCompletion, FileIoContinuation, FileIoFacility, ProcessContext,
};
use crate::term::Term;
use crate::term::binary::Binary;
use crate::term::boxed::{Cons, Tuple};

use super::{register_tcp_bifs, tcp_connect, tcp_recv, tcp_send};

const PID: u64 = 107;
const CURRENT_POSITION: u64 = u64::MAX;

#[derive(Default)]
struct MockRing {
    next_op_id: Mutex<u64>,
    submitted: Mutex<Vec<IoOp>>,
}

impl MockRing {
    fn submitted(&self) -> Vec<IoOp> {
        self.submitted.lock().expect("submitted lock").clone()
    }
}

impl CompletionRing for MockRing {
    fn submit(&self, op: IoOp) -> u64 {
        self.submitted.lock().expect("submitted lock").push(op);
        let mut next = self.next_op_id.lock().expect("next op id lock");
        let op_id = *next;
        *next += 1;
        op_id
    }

    fn poll_completions(&self, _timeout: std::time::Duration) -> Vec<IoCompletion> {
        Vec::new()
    }

    fn pending_count(&self) -> usize {
        self.submitted.lock().map(|ops| ops.len()).unwrap_or(0)
    }

    fn shutdown(&self) {}
}

#[derive(Default)]
struct MockFileIoFacility {
    ring: MockRing,
    pending: Mutex<Vec<(u64, u64, FileIoContinuation)>>,
    completions: Mutex<VecDeque<FileIoCompletion>>,
}

impl MockFileIoFacility {
    fn push_completion(&self, continuation: FileIoContinuation, result: io::Result<IoResult>) {
        self.completions
            .lock()
            .expect("completions lock")
            .push_back(FileIoCompletion {
                op_id: 1,
                continuation,
                completion: IoCompletion { op_id: 1, result },
            });
    }

    fn submitted(&self) -> Vec<IoOp> {
        self.ring.submitted()
    }

    fn tracked(&self) -> Vec<(u64, u64, FileIoContinuation)> {
        self.pending.lock().expect("pending lock").clone()
    }
}

impl FileIoFacility for MockFileIoFacility {
    fn submit_file_io(&self, pid: u64, op: IoOp, continuation: FileIoContinuation) -> u64 {
        let op_id = self.ring.submit(op);
        self.track_submitted_file_io(pid, op_id, continuation);
        op_id
    }

    fn track_submitted_file_io(&self, pid: u64, op_id: u64, continuation: FileIoContinuation) {
        self.pending
            .lock()
            .expect("pending lock")
            .push((pid, op_id, continuation));
    }

    fn take_file_io_completion(&self, _pid: u64) -> Option<FileIoCompletion> {
        self.completions
            .lock()
            .expect("completions lock")
            .pop_front()
    }

    fn take_pending_file_io(&self, pid: u64) -> Option<(u64, FileIoContinuation)> {
        let mut pending = self.pending.lock().expect("pending lock");
        let index = pending
            .iter()
            .position(|(pending_pid, _, _)| *pending_pid == pid)?;
        let (_, op_id, continuation) = pending.remove(index);
        Some((op_id, continuation))
    }

    fn abandon_file_io(&self, op_id: u64) {
        let mut pending = self.pending.lock().expect("pending lock");
        pending.retain(|(_, pending_op_id, _)| *pending_op_id != op_id);
    }

    fn ring(&self) -> &dyn CompletionRing {
        &self.ring
    }
}

fn heap_context<'a>(
    process: &'a mut crate::process::Process,
    facility: Arc<MockFileIoFacility>,
) -> ProcessContext<'a> {
    let mut context = ProcessContext::new();
    context.set_file_io_facility(Some(facility));
    context.attach_process(process, 0);
    context
}

fn binary(context: &mut ProcessContext<'_>, bytes: &[u8]) -> Term {
    context.alloc_binary(bytes).expect("binary allocation")
}

fn tuple2(context: &mut ProcessContext<'_>, first: Term, second: Term) -> Term {
    context
        .alloc_tuple(&[first, second])
        .expect("tuple allocation")
}

fn list(context: &mut ProcessContext<'_>, values: &[Term]) -> Term {
    context.alloc_list(values).expect("list allocation")
}

fn fd_resource(context: &mut ProcessContext<'_>, fd: RawFd) -> Term {
    context
        .alloc_fd_resource(Arc::new(FdInner::new(fd, PID)))
        .expect("fd resource allocation")
}

fn closed_fd_resource(context: &mut ProcessContext<'_>) -> Term {
    let inner = Arc::new(FdInner::new(-1, PID));
    inner.mark_closed();
    context
        .alloc_fd_resource(inner)
        .expect("fd resource allocation")
}

fn tuple_result(term: Term) -> (Term, Term) {
    let tuple = Tuple::new(term).expect("tuple result");
    (
        tuple.get(0).expect("tuple tag"),
        tuple.get(1).expect("tuple value"),
    )
}

fn error_reason(term: Term) -> Term {
    let (tag, reason) = tuple_result(term);
    assert_eq!(tag, Term::atom(Atom::ERROR));
    reason
}

#[test]
fn registers_all_tcp_bifs() {
    let atom_table = AtomTable::with_common_atoms();
    let registry = BifRegistryImpl::new();

    register_tcp_bifs(&registry, &atom_table).expect("tcp bif registration");

    let erlang = atom_table.lookup("erlang").expect("erlang atom");
    for (name, arity) in [
        ("tcp_connect", 3),
        ("tcp_send", 2),
        ("tcp_recv", 2),
        ("tcp_recv", 3),
    ] {
        let function = atom_table.lookup(name).expect("registered function atom");
        let entry = registry.lookup(erlang, function, arity);
        assert!(entry.is_some(), "missing erlang:{name}/{arity}");
    }
}

#[test]
fn connect_submits_nonblocking_stream_socket_with_timeout_and_bind_options() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 256);
    let mut context = heap_context(&mut process, Arc::clone(&facility));
    let host = binary(&mut context, b"127.0.0.1");
    let local_ip = binary(&mut context, b"127.0.0.1");
    let timeout = tuple2(
        &mut context,
        Term::atom(Atom::TIMEOUT),
        Term::small_int(250),
    );
    let ip = tuple2(&mut context, Term::atom(Atom::IP), local_ip);
    let port = tuple2(&mut context, Term::atom(Atom::PORT), Term::small_int(0));
    let options = list(&mut context, &[timeout, ip, port]);

    let result = tcp_connect(&[host, Term::small_int(9), options], &mut context)
        .expect("connect submit placeholder");

    assert_eq!(result, Term::atom(Atom::OK));
    assert_eq!(
        context.take_suspend().expect("suspend").timeout_ms,
        Some(250)
    );
    match facility.submitted().as_slice() {
        [IoOp::Connect { fd, addr }] => {
            assert!(*fd >= 0);
            assert_eq!(*addr, SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9));
            close_raw_fd_for_test(*fd);
        }
        other => panic!("expected Connect submission, got {other:?}"),
    }
}

#[test]
fn connect_completion_returns_ok_resource_and_refused_errors_are_mapped() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 128);
    let mut context = heap_context(&mut process, Arc::clone(&facility));
    let fd = pipe_read_fd();

    facility.push_completion(
        FileIoContinuation::TcpConnect { fd },
        Ok(IoResult::Connected),
    );
    let result = tcp_connect(&[], &mut context).expect("connect completion result");
    let (tag, resource) = tuple_result(result);
    assert_eq!(tag, Term::atom(Atom::OK));
    assert_eq!(FdResource::new(resource).expect("fd resource").fd(), fd);

    facility.push_completion(
        FileIoContinuation::TcpConnect { fd: -1 },
        Err(io::Error::from_raw_os_error(libc::ECONNREFUSED)),
    );
    let result = tcp_connect(&[], &mut context).expect("refused error tuple");
    assert_eq!(error_reason(result), Term::atom(Atom::ECONNREFUSED));
}

#[test]
fn send_submits_write_and_resubmits_partial_suffix_until_complete() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 256);
    let mut context = heap_context(&mut process, Arc::clone(&facility));
    let resource = fd_resource(&mut context, pipe_read_fd());
    let data = binary(&mut context, b"abcdef");

    let result = tcp_send(&[resource, data], &mut context).expect("send submit placeholder");
    assert_eq!(result, Term::atom(Atom::OK));
    assert!(context.take_suspend().is_some());
    let first_fd = match facility.submitted().as_slice() {
        [IoOp::Write { fd, data, offset }] => {
            assert_eq!(data.as_slice(), b"abcdef");
            assert_eq!(*offset, CURRENT_POSITION);
            *fd
        }
        other => panic!("expected first Write submission, got {other:?}"),
    };

    let inner = match &facility.tracked()[0].2 {
        FileIoContinuation::TcpSend { fd, remaining } => {
            assert_eq!(remaining.as_slice(), b"abcdef");
            Arc::clone(fd)
        }
        other => panic!("expected TcpSend continuation, got {other:?}"),
    };
    facility.push_completion(
        FileIoContinuation::TcpSend {
            fd: Arc::clone(&inner),
            remaining: b"abcdef".to_vec(),
        },
        Ok(IoResult::BytesWritten(2)),
    );
    let result = tcp_send(&[resource, data], &mut context).expect("partial send result");
    assert_eq!(result, Term::atom(Atom::OK));
    assert!(context.take_suspend().is_some());
    match facility.submitted().as_slice() {
        [_, IoOp::Write { fd, data, offset }] => {
            assert_eq!(*fd, first_fd);
            assert_eq!(data.as_slice(), b"cdef");
            assert_eq!(*offset, CURRENT_POSITION);
        }
        other => panic!("expected second Write submission, got {other:?}"),
    }

    facility.push_completion(
        FileIoContinuation::TcpSend {
            fd: inner,
            remaining: b"cdef".to_vec(),
        },
        Ok(IoResult::BytesWritten(4)),
    );
    let result = tcp_send(&[resource, data], &mut context).expect("full send result");
    assert_eq!(result, Term::atom(Atom::OK));
}

#[test]
fn send_closed_and_reset_return_closed() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 128);
    let mut context = heap_context(&mut process, Arc::clone(&facility));
    let closed = closed_fd_resource(&mut context);
    let data = binary(&mut context, b"data");

    let result = tcp_send(&[closed, data], &mut context).expect("closed send error tuple");
    assert_eq!(error_reason(result), Term::atom(Atom::CLOSED));

    let resource = fd_resource(&mut context, pipe_read_fd());
    let inner = FdResource::new(resource).expect("fd resource").inner();
    facility.push_completion(
        FileIoContinuation::TcpSend {
            fd: Arc::clone(&inner),
            remaining: b"data".to_vec(),
        },
        Err(io::Error::from_raw_os_error(libc::ECONNRESET)),
    );
    let result = tcp_send(&[resource, data], &mut context).expect("reset error tuple");
    assert_eq!(error_reason(result), Term::atom(Atom::CLOSED));
    assert_eq!(inner.state(), FdState::Closed);
}

#[test]
fn recv_length_zero_returns_available_bytes_and_exact_length_resubmits() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 512);
    let mut context = heap_context(&mut process, Arc::clone(&facility));
    let resource = fd_resource(&mut context, pipe_read_fd());

    let result = tcp_recv(&[resource, Term::small_int(0)], &mut context)
        .expect("recv available submit placeholder");
    assert_eq!(result, Term::atom(Atom::OK));
    assert!(matches!(
        facility.submitted().as_slice(),
        [IoOp::Read { buf_len: 8192, offset, .. }] if *offset == CURRENT_POSITION
    ));
    let inner = match &facility.tracked()[0].2 {
        FileIoContinuation::TcpRecv { fd, .. } => Arc::clone(fd),
        other => panic!("expected TcpRecv continuation, got {other:?}"),
    };
    facility.push_completion(
        FileIoContinuation::TcpRecv {
            fd: Arc::clone(&inner),
            requested_len: 0,
            accumulated: Vec::new(),
            timeout_ms: None,
        },
        Ok(IoResult::BytesRead(3, b"abc".to_vec())),
    );
    let result = tcp_recv(&[resource, Term::small_int(0)], &mut context).expect("available result");
    let (tag, bytes) = tuple_result(result);
    assert_eq!(tag, Term::atom(Atom::OK));
    assert_eq!(Binary::new(bytes).expect("binary").as_bytes(), b"abc");

    let result = tcp_recv(&[resource, Term::small_int(5)], &mut context)
        .expect("exact recv submit placeholder");
    assert_eq!(result, Term::atom(Atom::OK));
    facility.push_completion(
        FileIoContinuation::TcpRecv {
            fd: Arc::clone(&inner),
            requested_len: 5,
            accumulated: Vec::new(),
            timeout_ms: None,
        },
        Ok(IoResult::BytesRead(2, b"he".to_vec())),
    );
    let result = tcp_recv(&[resource, Term::small_int(5)], &mut context).expect("partial recv");
    assert_eq!(result, Term::atom(Atom::OK));
    assert!(matches!(
        facility.submitted().as_slice(),
        [_, _, IoOp::Read { buf_len: 3, offset, .. }] if *offset == CURRENT_POSITION
    ));

    facility.push_completion(
        FileIoContinuation::TcpRecv {
            fd: inner,
            requested_len: 5,
            accumulated: b"he".to_vec(),
            timeout_ms: None,
        },
        Ok(IoResult::BytesRead(3, b"llo".to_vec())),
    );
    let result = tcp_recv(&[resource, Term::small_int(5)], &mut context).expect("exact result");
    let (tag, bytes) = tuple_result(result);
    assert_eq!(tag, Term::atom(Atom::OK));
    assert_eq!(Binary::new(bytes).expect("binary").as_bytes(), b"hello");
}

#[test]
fn recv_closed_and_timeout_return_error_tuples() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 256);
    let mut context = heap_context(&mut process, Arc::clone(&facility));
    let resource = fd_resource(&mut context, pipe_read_fd());
    let inner = FdResource::new(resource).expect("fd resource").inner();

    facility.push_completion(
        FileIoContinuation::TcpRecv {
            fd: Arc::clone(&inner),
            requested_len: 1,
            accumulated: Vec::new(),
            timeout_ms: None,
        },
        Ok(IoResult::BytesRead(0, Vec::new())),
    );
    let result = tcp_recv(&[resource, Term::small_int(1)], &mut context).expect("closed recv");
    assert_eq!(error_reason(result), Term::atom(Atom::CLOSED));
    assert_eq!(inner.state(), FdState::Closed);

    let resource = fd_resource(&mut context, pipe_read_fd());
    let result = tcp_recv(
        &[resource, Term::small_int(1), Term::small_int(10)],
        &mut context,
    )
    .expect("timeout recv submit placeholder");
    assert_eq!(result, Term::atom(Atom::OK));
    assert_eq!(
        context.take_suspend().expect("suspend").timeout_ms,
        Some(10)
    );
    let result = tcp_recv(
        &[resource, Term::small_int(1), Term::small_int(10)],
        &mut context,
    )
    .expect("timeout error tuple");
    assert_eq!(error_reason(result), Term::atom(Atom::TIMEOUT));
}

#[test]
fn malformed_options_and_lengths_are_badarg() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 128);
    let mut context = heap_context(&mut process, facility);
    let host = binary(&mut context, b"127.0.0.1");
    let bad_option = context
        .alloc_tuple(&[Term::atom(Atom::TIMEOUT)])
        .expect("bad option tuple");
    let options = list(&mut context, &[bad_option]);

    let result = tcp_connect(&[host, Term::small_int(9), options], &mut context);
    assert_eq!(result, Err(Term::atom(Atom::BADARG)));

    let resource = closed_fd_resource(&mut context);
    let result = tcp_recv(&[resource, Term::small_int(-1)], &mut context);
    assert!(
        result.is_ok(),
        "closed resource is reported before length parsing"
    );
    let resource = fd_resource(&mut context, pipe_read_fd());
    let result = tcp_recv(&[resource, Term::small_int(-1)], &mut context);
    assert_eq!(result, Err(Term::atom(Atom::BADARG)));
}

fn pipe_read_fd() -> RawFd {
    let mut fds = [0; 2];
    // SAFETY: `fds` points to two valid RawFd slots for libc to initialize.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(rc, 0);
    // SAFETY: close the write end so tests only manage the read end.
    let _closed = unsafe { libc::close(fds[1]) };
    fds[0]
}

fn close_raw_fd_for_test(fd: RawFd) {
    if fd >= 0 {
        // SAFETY: tests close the raw fd after asserting it was submitted and before any resource owns it.
        let _closed = unsafe { libc::close(fd) };
    }
}

#[test]
fn option_list_rejects_improper_tail() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 128);
    let mut context = heap_context(&mut process, facility);
    let host = binary(&mut context, b"127.0.0.1");
    let timeout = tuple2(&mut context, Term::atom(Atom::TIMEOUT), Term::small_int(1));
    let improper_options = context
        .alloc_cons(timeout, Term::atom(Atom::OK))
        .expect("improper options cons");

    let result = tcp_connect(&[host, Term::small_int(9), improper_options], &mut context);
    assert_eq!(result, Err(Term::atom(Atom::BADARG)));
}

#[test]
fn cons_helpers_still_build_proper_lists() {
    let facility = Arc::new(MockFileIoFacility::default());
    let mut process = crate::process::Process::new(PID, 128);
    let mut context = heap_context(&mut process, facility);
    let first = tuple2(&mut context, Term::atom(Atom::TIMEOUT), Term::small_int(1));
    let values = list(&mut context, &[first]);
    let cons = Cons::new(values).expect("proper list cons");
    assert_eq!(cons.head(), first);
    assert_eq!(cons.tail(), Term::NIL);
}
