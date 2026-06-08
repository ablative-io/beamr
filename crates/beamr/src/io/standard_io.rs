//! Scheduler-owned `standard_io` process for Erlang group-leader I/O requests.

use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::atom::Atom;
use crate::io::resource::FdInner;
use crate::io::ring::{CompletionRing, IoOp, IoResult};
use crate::process::heap::{DEFAULT_HEAP_SIZE, Heap};
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;
use crate::term::boxed::{Tuple, write_tuple};
use crate::term::shared_binary::{alloc_binary, alloc_binary_word_count};

const STDIN_FD: RawFd = 0;
const STDOUT_FD: RawFd = 1;
const STDERR_FD: RawFd = 2;
const READ_CHUNK: usize = 1024;

/// Handle retained by the scheduler for the standard I/O group leader process.
pub struct StandardIoProcess {
    pid: u64,
    sender: Sender<Term>,
    thread: Option<JoinHandle<()>>,
    _stdin: Arc<FdInner>,
    _stdout: Arc<FdInner>,
    _stderr: Arc<FdInner>,
}

impl StandardIoProcess {
    /// Start the standard I/O process loop.
    #[must_use]
    pub fn start(
        pid: u64,
        ring: Option<Arc<dyn CompletionRing>>,
        reply_sender: Arc<dyn StandardIoReplySender>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let stdin = Arc::new(FdInner::borrowed(STDIN_FD, pid));
        let stdout = Arc::new(FdInner::borrowed(STDOUT_FD, pid));
        let stderr = Arc::new(FdInner::borrowed(STDERR_FD, pid));
        let thread_stdin = Arc::clone(&stdin);
        let thread_stdout = Arc::clone(&stdout);
        let thread = thread::Builder::new()
            .name("beamr-standard-io".to_owned())
            .spawn(move || run_loop(receiver, ring, reply_sender, thread_stdin, thread_stdout))
            .ok();
        Self {
            pid,
            sender,
            thread,
            _stdin: stdin,
            _stdout: stdout,
            _stderr: stderr,
        }
    }

    /// PID used as the initial group leader.
    #[must_use]
    pub const fn pid(&self) -> u64 {
        self.pid
    }

    /// Enqueue an already copied `io_request` term for the standard I/O loop.
    pub fn send(&self, message: Term) -> bool {
        self.sender.send(message).is_ok()
    }
}

impl Drop for StandardIoProcess {
    fn drop(&mut self) {
        let (replacement, _receiver) = mpsc::channel();
        let old_sender = std::mem::replace(&mut self.sender, replacement);
        drop(old_sender);
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

/// Scheduler callback used by the standard I/O loop to deliver replies.
pub trait StandardIoReplySender: Send + Sync {
    /// Send a reply term to `pid`.
    fn send_reply(&self, pid: u64, message: Term) -> bool;
}

fn run_loop(
    receiver: Receiver<Term>,
    ring: Option<Arc<dyn CompletionRing>>,
    reply_sender: Arc<dyn StandardIoReplySender>,
    stdin: Arc<FdInner>,
    stdout: Arc<FdInner>,
) {
    let mut heap = Heap::new(DEFAULT_HEAP_SIZE * 8);
    while let Ok(message) = receiver.recv() {
        handle_message(
            message,
            &ring,
            reply_sender.as_ref(),
            &stdin,
            &stdout,
            &mut heap,
        );
    }
}

fn handle_message(
    message: Term,
    ring: &Option<Arc<dyn CompletionRing>>,
    reply_sender: &dyn StandardIoReplySender,
    stdin: &FdInner,
    stdout: &FdInner,
    heap: &mut Heap,
) {
    let Some((from_pid, reply_as, request)) = parse_io_request(message) else {
        return;
    };
    let result = handle_request(request, ring, stdin, stdout, heap).unwrap_or_else(|| {
        error_tuple(Atom::REQUEST, heap).unwrap_or_else(|| Term::atom(Atom::ERROR))
    });
    if let Some(reply) = alloc_tuple(heap, &[Term::atom(Atom::IO_REPLY), reply_as, result]) {
        let _sent = reply_sender.send_reply(from_pid, reply);
    }
}

fn parse_io_request(message: Term) -> Option<(u64, Term, Term)> {
    let tuple = Tuple::new(message)?;
    if tuple.arity() != 4 || tuple.get(0)? != Term::atom(Atom::IO_REQUEST) {
        return None;
    }
    let from_pid = tuple.get(1)?.as_pid()?;
    Some((from_pid, tuple.get(2)?, tuple.get(3)?))
}

fn handle_request(
    request: Term,
    ring: &Option<Arc<dyn CompletionRing>>,
    stdin: &FdInner,
    stdout: &FdInner,
    heap: &mut Heap,
) -> Option<Term> {
    let tuple = Tuple::new(request)?;
    match tuple.get(0)?.as_atom()? {
        atom if atom == Atom::PUT_CHARS => {
            if tuple.arity() != 3 || tuple.get(1)? != Term::atom(Atom::UNICODE) {
                return error_tuple(Atom::REQUEST, heap);
            }
            let bytes = iodata_bytes(tuple.get(2)?)?;
            write_all(stdout.fd(), bytes, ring).ok()?;
            Some(Term::atom(Atom::OK))
        }
        atom if atom == Atom::GET_LINE => {
            if tuple.arity() != 3 || tuple.get(1)? != Term::atom(Atom::UNICODE) {
                return error_tuple(Atom::REQUEST, heap);
            }
            let prompt = iodata_bytes(tuple.get(2)?)?;
            write_all(stdout.fd(), prompt, ring).ok()?;
            match read_until(stdin.fd(), b'\n', ring).ok()? {
                ReadData::Eof => Some(Term::atom(Atom::EOF)),
                ReadData::Bytes(bytes) => alloc_binary_term(heap, &bytes),
            }
        }
        atom if atom == Atom::GET_UNTIL => {
            let delimiter = request_delimiter(tuple.get(tuple.arity().saturating_sub(1))?)?;
            match read_until(stdin.fd(), delimiter, ring).ok()? {
                ReadData::Eof => Some(Term::atom(Atom::EOF)),
                ReadData::Bytes(bytes) => alloc_binary_term(heap, &bytes),
            }
        }
        _ => error_tuple(Atom::REQUEST, heap),
    }
}

fn request_delimiter(term: Term) -> Option<u8> {
    term.as_small_int()
        .and_then(|value| u8::try_from(value).ok())
        .or_else(|| {
            let bytes = iodata_bytes(term)?;
            bytes.first().copied()
        })
}

enum ReadData {
    Eof,
    Bytes(Vec<u8>),
}

fn read_until(
    fd: RawFd,
    delimiter: u8,
    ring: &Option<Arc<dyn CompletionRing>>,
) -> io::Result<ReadData> {
    let mut accumulated = Vec::new();
    loop {
        let chunk = read_chunk(fd, ring)?;
        if chunk.is_empty() {
            return if accumulated.is_empty() {
                Ok(ReadData::Eof)
            } else {
                Ok(ReadData::Bytes(accumulated))
            };
        }
        let found = chunk.iter().position(|byte| *byte == delimiter);
        match found {
            Some(index) => {
                accumulated.extend_from_slice(&chunk[..=index]);
                return Ok(ReadData::Bytes(accumulated));
            }
            None => accumulated.extend_from_slice(&chunk),
        }
    }
}

fn read_chunk(fd: RawFd, ring: &Option<Arc<dyn CompletionRing>>) -> io::Result<Vec<u8>> {
    if let Some(ring) = ring {
        let op_id = ring.submit(IoOp::Read {
            fd,
            buf_len: READ_CHUNK,
            offset: 0,
        });
        match wait_for_completion(ring.as_ref(), op_id)? {
            IoResult::BytesRead(count, mut data) => {
                data.truncate(count);
                Ok(data)
            }
            _ => Ok(Vec::new()),
        }
    } else {
        let mut data = vec![0_u8; READ_CHUNK];
        let count = io::stdin().lock().read(&mut data)?;
        data.truncate(count);
        Ok(data)
    }
}

fn write_all(fd: RawFd, data: Vec<u8>, ring: &Option<Arc<dyn CompletionRing>>) -> io::Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    if let Some(ring) = ring {
        let op_id = ring.submit(IoOp::Write {
            fd,
            data,
            offset: 0,
        });
        match wait_for_completion(ring.as_ref(), op_id)? {
            IoResult::BytesWritten(_) | IoResult::Completed => Ok(()),
            _ => Ok(()),
        }
    } else {
        let mut stdout = io::stdout().lock();
        stdout.write_all(&data)?;
        stdout.flush()
    }
}

fn wait_for_completion(ring: &dyn CompletionRing, op_id: u64) -> io::Result<IoResult> {
    loop {
        for completion in ring.poll_completions(Duration::from_millis(50)) {
            if completion.op_id == op_id {
                return completion.result;
            }
        }
    }
}

fn iodata_bytes(term: Term) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    collect_iodata(term, &mut bytes)?;
    Some(bytes)
}

fn collect_iodata(term: Term, out: &mut Vec<u8>) -> Option<()> {
    if term.is_nil() {
        return Some(());
    }
    if let Some(binary) = BinaryRef::new(term) {
        out.extend_from_slice(binary.as_bytes());
        return Some(());
    }
    if let Some(byte) = term
        .as_small_int()
        .and_then(|value| u8::try_from(value).ok())
    {
        out.push(byte);
        return Some(());
    }
    let cons = crate::term::boxed::Cons::new(term)?;
    collect_iodata(cons.head(), out)?;
    collect_iodata(cons.tail(), out)
}

fn alloc_binary_term(heap: &mut Heap, bytes: &[u8]) -> Option<Term> {
    let words = alloc_binary_word_count(bytes.len());
    let heap_words = heap.alloc_slice(words).ok()?;
    alloc_binary(heap_words, bytes)
}

fn error_tuple(reason: Atom, heap: &mut Heap) -> Option<Term> {
    alloc_tuple(heap, &[Term::atom(Atom::ERROR), Term::atom(reason)])
}

fn alloc_tuple(heap: &mut Heap, elements: &[Term]) -> Option<Term> {
    let words = heap.alloc_slice(1 + elements.len()).ok()?;
    write_tuple(words, elements)
}

/// Track completion ids claimed by standard_io so the global bridge ignores them.
#[derive(Debug, Default)]
pub struct StandardIoClaimedOps {
    claimed: std::sync::Mutex<HashSet<u64>>,
}

impl StandardIoClaimedOps {
    /// Claim an op id for synchronous standard_io polling.
    pub fn claim(&self, op_id: u64) {
        if let Ok(mut claimed) = self.claimed.lock() {
            claimed.insert(op_id);
        }
    }

    /// Return whether an op id was claimed by standard_io.
    #[must_use]
    pub fn take(&self, op_id: u64) -> bool {
        self.claimed
            .lock()
            .map(|mut claimed| claimed.remove(&op_id))
            .unwrap_or(false)
    }
}
