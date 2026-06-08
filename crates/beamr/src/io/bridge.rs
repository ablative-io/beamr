//! Completion bridge from backend I/O completions to scheduler wakeups.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::atom::Atom;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

use dashmap::DashMap;

use crate::io::resource::{FdInner, FdMode, FdState};
use crate::term::Term;

use super::{CompletionRing, IoCompletion, IoOp, IoResult};

/// How an I/O completion should be delivered to the waiting process.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ResultMode {
    /// Resume the process with the result stored in x(0).
    XRegister,
    /// Send the result as a mailbox message.
    Message,
    /// Consume the result without waking a process.
    Discard,
}

/// Process currently waiting for a ring operation to complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingIo {
    /// Waiting process id.
    pub pid: u64,
    /// Completion delivery mode.
    pub result_mode: ResultMode,
    /// Rich metadata for completions that cannot be represented by a small immediate term.
    pub kind: PendingIoKind,
}

/// Rich completion delivery metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingIoKind {
    /// Existing generic completion conversion path.
    Generic,
    /// Active TCP read loop completion.
    ActiveTcpRead {
        socket: Arc<FdInner>,
        buf_len: usize,
    },
}

/// Scheduler-owned active TCP message to materialize on the receiver heap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActiveTcpEvent {
    /// `{tcp, Socket, Data}`.
    Data { socket: Arc<FdInner>, data: Vec<u8> },
    /// `{tcp_closed, Socket}`.
    Closed { socket: Arc<FdInner> },
    /// `{tcp_error, Socket, Reason}`.
    Error { socket: Arc<FdInner>, reason: Atom },
}

/// Concurrent registry of ring operation ids to waiting processes.
#[derive(Debug, Default)]
pub struct PendingIoRegistry {
    pending: DashMap<u64, PendingIo>,
}

impl PendingIoRegistry {
    /// Register `pid` as waiting for `op_id`.
    pub fn register(&self, op_id: u64, pid: u64, mode: ResultMode) {
        self.register_pending(
            op_id,
            PendingIo {
                pid,
                result_mode: mode,
                kind: PendingIoKind::Generic,
            },
        );
    }

    /// Register an active TCP read operation for completion delivery.
    pub fn register_active_tcp_read(&self, op_id: u64, socket: Arc<FdInner>, buf_len: usize) {
        self.register_pending(
            op_id,
            PendingIo {
                pid: socket.controlling_process(),
                result_mode: ResultMode::Message,
                kind: PendingIoKind::ActiveTcpRead { socket, buf_len },
            },
        );
    }

    /// Register a fully specified pending operation.
    pub fn register_pending(&self, op_id: u64, pending: PendingIo) {
        self.pending.insert(op_id, pending);
    }

    /// Remove and return the waiting process for `op_id`, if any.
    pub fn take(&self, op_id: u64) -> Option<PendingIo> {
        self.pending.remove(&op_id).map(|(_, pending)| pending)
    }
}

/// Scheduler-facing completion delivery surface used by the bridge poller.
pub trait IoWakeTarget: Send + Sync {
    /// Wake `pid` and arrange for `term` to be placed in x(0) on resume.
    fn wake_with_io_result(&self, pid: u64, term: Term);

    /// Enqueue `term` as an I/O completion message for `pid`.
    fn send_io_message(&self, pid: u64, term: Term);

    /// Enqueue an active TCP event, materializing boxed terms on the receiver heap.
    fn send_active_tcp_event(&self, pid: u64, event: ActiveTcpEvent);
}

/// Lifecycle handle for the dedicated I/O completion poller thread.
pub struct IoCompletionBridge {
    shutdown: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl IoCompletionBridge {
    /// Start a completion poller thread.
    #[must_use]
    pub fn start(
        ring: Arc<dyn CompletionRing>,
        registry: Arc<PendingIoRegistry>,
        scheduler: Arc<dyn IoWakeTarget>,
    ) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let handle = std::thread::Builder::new()
            .name("beamr-io-completion".to_string())
            .spawn(move || {
                while !shutdown_for_thread.load(Ordering::Acquire) {
                    let completions = ring.poll_completions(Duration::from_millis(100));
                    for completion in completions {
                        dispatch_completion(
                            ring.as_ref(),
                            &registry,
                            scheduler.as_ref(),
                            completion,
                        );
                    }
                }
            })
            .unwrap_or_else(|error| {
                shutdown.store(true, Ordering::Release);
                panic!("failed to spawn beamr-io-completion thread: {error}");
            });

        Self {
            shutdown,
            handle: Mutex::new(Some(handle)),
        }
    }

    /// Request poller shutdown and join the thread once.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let handle = match self.handle.lock() {
            Ok(mut guard) => guard.take(),
            Err(error) => error.into_inner().take(),
        };
        if let Some(handle) = handle.filter(|handle| handle.thread().id() != thread::current().id())
            && let Err(payload) = handle.join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}

impl Drop for IoCompletionBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn dispatch_completion(
    ring: &dyn CompletionRing,
    registry: &PendingIoRegistry,
    scheduler: &dyn IoWakeTarget,
    completion: IoCompletion,
) {
    let Some(pending) = registry.take(completion.op_id) else {
        return;
    };
    if let PendingIoKind::ActiveTcpRead { socket, buf_len } = pending.kind {
        dispatch_active_tcp_read(
            ring,
            registry,
            scheduler,
            socket,
            buf_len,
            completion.result,
        );
        return;
    }
    if pending.result_mode == ResultMode::Discard {
        return;
    }
    let term = io_completion_to_term(completion.result);
    match pending.result_mode {
        ResultMode::XRegister => scheduler.wake_with_io_result(pending.pid, term),
        ResultMode::Message => scheduler.send_io_message(pending.pid, term),
        ResultMode::Discard => {}
    }
}

/// Submit an active TCP read if the socket is currently open and active.
pub fn submit_active_tcp_read(
    ring: &dyn CompletionRing,
    registry: &PendingIoRegistry,
    socket: Arc<FdInner>,
    buf_len: usize,
) -> Option<u64> {
    if socket.state() != FdState::Open
        || !matches!(socket.mode(), FdMode::Active | FdMode::ActiveOnce)
    {
        return None;
    }
    let op_id = ring.submit(IoOp::Read {
        fd: socket.fd(),
        buf_len,
        offset: u64::MAX,
    });
    registry.register_active_tcp_read(op_id, socket, buf_len);
    Some(op_id)
}

fn dispatch_active_tcp_read(
    ring: &dyn CompletionRing,
    registry: &PendingIoRegistry,
    scheduler: &dyn IoWakeTarget,
    socket: Arc<FdInner>,
    buf_len: usize,
    result: io::Result<IoResult>,
) {
    let pid = socket.controlling_process();
    match result {
        Ok(IoResult::BytesRead(0, _)) => {
            socket.set_mode(FdMode::Passive);
            scheduler.send_active_tcp_event(pid, ActiveTcpEvent::Closed { socket });
        }
        Ok(IoResult::BytesRead(bytes_read, bytes)) => {
            let data = bytes
                .get(..bytes_read)
                .map_or_else(|| bytes.clone(), <[u8]>::to_vec);
            let previous_mode = socket.mode();
            if previous_mode == FdMode::ActiveOnce {
                socket.set_mode(FdMode::Passive);
            }
            scheduler.send_active_tcp_event(
                pid,
                ActiveTcpEvent::Data {
                    socket: Arc::clone(&socket),
                    data,
                },
            );
            if previous_mode == FdMode::Active && socket.mode() == FdMode::Active {
                let _submitted = submit_active_tcp_read(ring, registry, socket, buf_len);
            }
        }
        Ok(_) => {
            socket.set_mode(FdMode::Passive);
            scheduler.send_active_tcp_event(
                pid,
                ActiveTcpEvent::Error {
                    socket,
                    reason: Atom::UNKNOWN_ERROR,
                },
            );
        }
        Err(error) => {
            socket.set_mode(FdMode::Passive);
            let reason = error
                .raw_os_error()
                .map(super::errno_to_atom)
                .unwrap_or(Atom::UNKNOWN_ERROR);
            scheduler.send_active_tcp_event(pid, ActiveTcpEvent::Error { socket, reason });
        }
    }
}

fn io_completion_to_term(result: io::Result<IoResult>) -> Term {
    match result {
        Ok(IoResult::BytesRead(count, _)) | Ok(IoResult::BytesWritten(count)) => {
            usize_to_term(count)
        }
        Ok(IoResult::Accepted(fd, _)) | Ok(IoResult::Opened(fd)) => i64_to_term(i64::from(fd)),
        Ok(IoResult::Connected)
        | Ok(IoResult::Closed)
        | Ok(IoResult::Synced)
        | Ok(IoResult::StatResult(_))
        | Ok(IoResult::Completed) => Term::small_int(0),
        Err(error) => match error.raw_os_error() {
            Some(code) => i64_to_term(-i64::from(code)),
            None => Term::small_int(-1),
        },
    }
}

fn usize_to_term(value: usize) -> Term {
    match i64::try_from(value).ok().and_then(Term::try_small_int) {
        Some(term) => term,
        None => Term::NIL,
    }
}

fn i64_to_term(value: i64) -> Term {
    match Term::try_small_int(value) {
        Some(term) => term,
        None => Term::NIL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    use std::sync::{Condvar, Mutex};

    #[test]
    fn registry_register_and_take_removes_pending_entry() {
        let registry = PendingIoRegistry::default();
        registry.register(7, 42, ResultMode::XRegister);

        assert_eq!(
            registry.take(7),
            Some(PendingIo {
                pid: 42,
                result_mode: ResultMode::XRegister,
                kind: PendingIoKind::Generic,
            })
        );
        assert_eq!(registry.take(7), None);
    }

    #[test]
    fn registry_is_safe_for_concurrent_registration() {
        let registry = Arc::new(PendingIoRegistry::default());
        let mut handles = Vec::new();
        for worker in 0..8_u64 {
            let registry = Arc::clone(&registry);
            handles.push(std::thread::spawn(move || {
                for op in 0..32_u64 {
                    registry.register(worker * 100 + op, worker, ResultMode::Message);
                }
            }));
        }
        for handle in handles {
            assert!(handle.join().is_ok());
        }
        for worker in 0..8_u64 {
            for op in 0..32_u64 {
                assert_eq!(
                    registry.take(worker * 100 + op),
                    Some(PendingIo {
                        pid: worker,
                        result_mode: ResultMode::Message,
                        kind: PendingIoKind::Generic,
                    })
                );
            }
        }
    }

    struct MockRing {
        completions: Mutex<Vec<IoCompletion>>,
        shutdown: AtomicBool,
    }

    impl CompletionRing for MockRing {
        fn submit(&self, _op: super::super::IoOp) -> u64 {
            1
        }

        fn poll_completions(&self, _timeout: Duration) -> Vec<IoCompletion> {
            self.completions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .drain(..)
                .collect()
        }

        fn pending_count(&self) -> usize {
            0
        }

        fn shutdown(&self) {
            self.shutdown.store(true, Ordering::Release);
        }
    }

    #[derive(Default)]
    struct MockWakeTarget {
        x_result: Mutex<Option<(u64, Term)>>,
        message: Mutex<Option<(u64, Term)>>,
        notifications: (Mutex<usize>, Condvar),
    }

    impl MockWakeTarget {
        fn wait_for_notifications(&self, count: usize) {
            let (lock, condvar) = &self.notifications;
            let mut guard = lock.lock().unwrap_or_else(|error| error.into_inner());
            while *guard < count {
                guard = condvar
                    .wait(guard)
                    .unwrap_or_else(|error| error.into_inner());
            }
        }

        fn notify(&self) {
            let (lock, condvar) = &self.notifications;
            let mut guard = lock.lock().unwrap_or_else(|error| error.into_inner());
            *guard += 1;
            condvar.notify_all();
        }
    }

    impl IoWakeTarget for MockWakeTarget {
        fn wake_with_io_result(&self, pid: u64, term: Term) {
            *self
                .x_result
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some((pid, term));
            self.notify();
        }

        fn send_io_message(&self, pid: u64, term: Term) {
            *self
                .message
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some((pid, term));
            self.notify();
        }

        fn send_active_tcp_event(&self, pid: u64, _event: ActiveTcpEvent) {
            *self
                .message
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some((pid, Term::small_int(0)));
            self.notify();
        }
    }

    #[test]
    fn bridge_dispatches_x_register_completion_and_shuts_down() {
        let ring = Arc::new(MockRing {
            completions: Mutex::new(vec![IoCompletion {
                op_id: 9,
                result: Ok(IoResult::BytesWritten(5)),
            }]),
            shutdown: AtomicBool::new(false),
        });
        let registry = Arc::new(PendingIoRegistry::default());
        registry.register(9, 77, ResultMode::XRegister);
        let target = Arc::new(MockWakeTarget::default());

        let bridge = IoCompletionBridge::start(ring, registry, target.clone());
        target.wait_for_notifications(1);
        bridge.shutdown();
        bridge.shutdown();

        assert_eq!(
            *target
                .x_result
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            Some((77, Term::small_int(5)))
        );
    }

    #[test]
    fn bridge_dispatches_message_completion() {
        let ring = Arc::new(MockRing {
            completions: Mutex::new(vec![IoCompletion {
                op_id: 10,
                result: Err(io::Error::from_raw_os_error(2)),
            }]),
            shutdown: AtomicBool::new(false),
        });
        let registry = Arc::new(PendingIoRegistry::default());
        registry.register(10, 88, ResultMode::Message);
        let target = Arc::new(MockWakeTarget::default());

        let bridge = IoCompletionBridge::start(ring, registry, target.clone());
        target.wait_for_notifications(1);
        bridge.shutdown();

        assert_eq!(
            *target
                .message
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            Some((88, Term::small_int(-2)))
        );
    }
}
