//! Standard group-leader I/O server helpers.

use std::time::Duration;

use crate::atom::{Atom, AtomTable};
use crate::io::{CompletionRing, IoOp, IoResult};
use crate::native::IoMessageFacility;
use crate::process::Process;
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;
use crate::term::boxed::{Cons, Tuple};

const STDIN_FD: i32 = 0;
const STDOUT_FD: i32 = 1;
const CURRENT_POSITION: u64 = u64::MAX;
const READ_CHUNK_SIZE: usize = 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const POLL_TIMEOUT: Duration = Duration::from_millis(10);

/// Runtime-owned standard I/O server state.
pub struct StandardIoServer {
    pid: u64,
    ring: std::sync::Arc<dyn CompletionRing>,
    atoms: StandardIoAtoms,
}

#[derive(Copy, Clone)]
struct StandardIoAtoms {
    io_request: Atom,
    io_reply: Atom,
    put_chars: Atom,
    get_line: Atom,
    get_until: Atom,
    unicode: Atom,
    request: Atom,
    eof: Atom,
}

impl StandardIoServer {
    /// Create a scheduler-owned standard I/O server process body.
    #[must_use]
    pub fn process(pid: u64) -> Process {
        let mut process = Process::new(pid, crate::process::heap::DEFAULT_HEAP_SIZE);
        process.set_group_leader(Term::pid(pid));
        process
    }

    /// Build server state for `pid` using the scheduler's completion ring.
    #[must_use]
    pub fn new(pid: u64, ring: std::sync::Arc<dyn CompletionRing>, atom_table: &AtomTable) -> Self {
        Self {
            pid,
            ring,
            atoms: StandardIoAtoms::new(atom_table),
        }
    }

    /// Drain and handle all currently arrived `io_request` messages.
    pub fn run_available(&self, process: &mut Process, messages: &dyn IoMessageFacility) {
        process.mailbox_mut().drain_arrival();
        while let Some(message) = process.mailbox_mut().remove_current_message() {
            self.handle_message(message, messages);
        }
    }

    fn handle_message(&self, message: Term, messages: &dyn IoMessageFacility) {
        let Some(tuple) = Tuple::new(message) else {
            return;
        };
        if tuple.arity() != 4 || tuple.get(0) != Some(Term::atom(self.atoms.io_request)) {
            return;
        }
        let Some(from) = tuple.get(1).and_then(Term::as_pid) else {
            return;
        };
        let Some(reply_as) = tuple.get(2) else {
            return;
        };
        let Some(request) = tuple.get(3) else {
            return;
        };
        let result = self.handle_request(request);
        if let Some(reply) = self.build_reply(reply_as, result) {
            let _sent = messages.send_message(self.pid, from, reply);
        }
    }

    fn handle_request(&self, request: Term) -> Term {
        let Some(tuple) = Tuple::new(request) else {
            return self.error_request();
        };
        if tuple.arity() < 3 {
            return self.error_request();
        }
        let Some(kind) = tuple.get(0).and_then(Term::as_atom) else {
            return self.error_request();
        };
        let encoding = tuple.get(1);
        if encoding != Some(Term::atom(self.atoms.unicode)) {
            return self.error_request();
        }
        if kind == self.atoms.put_chars {
            return tuple
                .get(2)
                .and_then(iodata_bytes)
                .and_then(|bytes| self.write_stdout(&bytes).then_some(Term::atom(Atom::OK)))
                .unwrap_or_else(|| self.error_request());
        }
        if kind == self.atoms.get_line {
            return tuple
                .get(2)
                .and_then(iodata_bytes)
                .and_then(|prompt| self.get_line(&prompt, b'\n'))
                .unwrap_or_else(|| self.error_request());
        }
        if kind == self.atoms.get_until {
            let delimiter = tuple
                .get(3)
                .and_then(iodata_bytes)
                .and_then(|bytes| bytes.first().copied())
                .unwrap_or(b'\n');
            return tuple
                .get(2)
                .and_then(iodata_bytes)
                .and_then(|prompt| self.get_line(&prompt, delimiter))
                .unwrap_or_else(|| self.error_request());
        }
        self.error_request()
    }

    fn write_stdout(&self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        let op_id = self.ring.submit(IoOp::Write {
            fd: STDOUT_FD,
            data: bytes.to_vec(),
            offset: CURRENT_POSITION,
        });
        self.wait_for_completion(op_id)
            .is_some_and(|result| matches!(result, IoResult::BytesWritten(_)))
    }

    fn get_line(&self, prompt: &[u8], delimiter: u8) -> Option<Term> {
        if !self.write_stdout(prompt) {
            return None;
        }
        let mut data = Vec::new();
        loop {
            let op_id = self.ring.submit(IoOp::Read {
                fd: STDIN_FD,
                buf_len: READ_CHUNK_SIZE,
                offset: CURRENT_POSITION,
            });
            let result = self.wait_for_completion(op_id)?;
            let IoResult::BytesRead(bytes_read, bytes) = result else {
                return None;
            };
            if bytes_read == 0 {
                return if data.is_empty() {
                    Some(Term::atom(self.atoms.eof))
                } else {
                    alloc_static_binary(&data)
                };
            }
            let read_bytes = &bytes[..bytes_read.min(bytes.len())];
            if let Some(position) = read_bytes.iter().position(|byte| *byte == delimiter) {
                data.extend_from_slice(&read_bytes[..=position]);
                return alloc_static_binary(&data);
            }
            data.extend_from_slice(read_bytes);
            if data.len() >= MAX_LINE_BYTES {
                return alloc_static_binary(&data);
            }
        }
    }

    fn wait_for_completion(&self, op_id: u64) -> Option<IoResult> {
        loop {
            for completion in self.ring.poll_completions(POLL_TIMEOUT) {
                if completion.op_id == op_id {
                    return completion.result.ok();
                }
            }
        }
    }

    fn build_reply(&self, reply_as: Term, result: Term) -> Option<Term> {
        let elements = [Term::atom(self.atoms.io_reply), reply_as, result];
        alloc_static_tuple(&elements)
    }

    fn error_request(&self) -> Term {
        let elements = [Term::atom(Atom::ERROR), Term::atom(self.atoms.request)];
        alloc_static_tuple(&elements).unwrap_or(Term::atom(Atom::ERROR))
    }
}

impl StandardIoAtoms {
    fn new(atom_table: &AtomTable) -> Self {
        Self {
            io_request: atom_table.intern("io_request"),
            io_reply: atom_table.intern("io_reply"),
            put_chars: atom_table.intern("put_chars"),
            get_line: atom_table.intern("get_line"),
            get_until: atom_table.intern("get_until"),
            unicode: atom_table.intern("unicode"),
            request: atom_table.intern("request"),
            eof: atom_table.intern("eof"),
        }
    }
}

fn iodata_bytes(term: Term) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    collect_iodata(term, &mut bytes).then_some(bytes)
}

fn collect_iodata(term: Term, out: &mut Vec<u8>) -> bool {
    if term.is_nil() {
        return true;
    }
    if let Some(binary) = BinaryRef::new(term) {
        out.extend_from_slice(binary.as_bytes());
        return true;
    }
    if let Some(byte) = term
        .as_small_int()
        .and_then(|value| u8::try_from(value).ok())
    {
        out.push(byte);
        return true;
    }
    let Some(cons) = Cons::new(term) else {
        return false;
    };
    collect_iodata(cons.head(), out) && collect_iodata(cons.tail(), out)
}

fn alloc_static_tuple(elements: &[Term]) -> Option<Term> {
    let words = Box::leak(vec![0u64; 1 + elements.len()].into_boxed_slice());
    crate::term::boxed::write_tuple(words, elements)
}

fn alloc_static_binary(bytes: &[u8]) -> Option<Term> {
    let words = crate::term::shared_binary::alloc_binary_word_count(bytes.len());
    let heap = Box::leak(vec![0u64; words].into_boxed_slice());
    crate::term::shared_binary::alloc_binary(heap, bytes)
}
