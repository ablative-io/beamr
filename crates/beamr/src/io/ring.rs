//! Backend-neutral completion ring types shared by I/O lifecycle code.

#[cfg(target_os = "linux")]
use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::os::fd::RawFd;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// I/O operation accepted by a completion ring.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IoOp {
    /// Read up to `buf_len` bytes from `fd` at `offset`.
    Read {
        fd: RawFd,
        buf_len: usize,
        offset: u64,
    },
    /// Write `data` to `fd` at `offset`.
    Write {
        fd: RawFd,
        data: Vec<u8>,
        offset: u64,
    },
    /// Accept a connection from a listening socket.
    Accept { listener_fd: RawFd },
    /// Connect a socket to `addr`.
    Connect { fd: RawFd, addr: SocketAddr },
    /// Close a raw file descriptor asynchronously.
    Close { fd: RawFd },
    /// Synchronize file contents to stable storage.
    Fsync { fd: RawFd },
    /// Open a path relative to `dir_fd`.
    Openat {
        dir_fd: RawFd,
        path: PathBuf,
        flags: i32,
        mode: u32,
    },
    /// Query file metadata relative to `dir_fd`.
    Statx {
        dir_fd: RawFd,
        path: PathBuf,
        flags: i32,
        mask: u32,
    },
    /// List a directory's entry filenames.
    ListDir { path: PathBuf },
    /// Create a directory.
    MakeDir { path: PathBuf },
    /// Delete a file.
    DelFile { path: PathBuf },
    /// Delete an empty directory.
    DelDir { path: PathBuf },
    /// Rename a file or directory.
    Rename {
        source: PathBuf,
        destination: PathBuf,
    },
    /// Send one datagram to `addr` with sendmsg-style address inclusion.
    SendMsg {
        fd: RawFd,
        data: Vec<u8>,
        addr: SocketAddr,
    },
    /// Receive one datagram and its source address with recvmsg-style address inclusion.
    RecvMsg { fd: RawFd, buf_len: usize },
    /// Complete without performing I/O.
    Nop,
}

/// Portable subset of statx-style metadata returned by completion rings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StatxData {
    /// Mask of fields populated by the backend.
    pub mask: u32,
    /// File type and permissions.
    pub mode: u32,
    /// File size in bytes.
    pub size: u64,
    /// Number of allocated 512-byte blocks.
    pub blocks: u64,
    /// Device major number for the inode owner device, when available.
    pub dev_major: u32,
    /// Device minor number for the inode owner device, when available.
    pub dev_minor: u32,
    /// Inode number.
    pub inode: u64,
    /// Link count.
    pub nlink: u64,
    /// Owning user id.
    pub uid: u32,
    /// Owning group id.
    pub gid: u32,
    /// Last access time, seconds since the Unix epoch.
    pub atime_sec: i64,
    /// Last modification time, seconds since the Unix epoch.
    pub mtime_sec: i64,
    /// Last status-change time, seconds since the Unix epoch.
    pub ctime_sec: i64,
}

/// Successful operation result produced by a completion ring.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IoResult {
    /// Bytes read and the buffer containing exactly those bytes.
    BytesRead(usize, Vec<u8>),
    /// Number of bytes written.
    BytesWritten(usize),
    /// Accepted file descriptor and peer address.
    Accepted(RawFd, SocketAddr),
    /// Connect completed successfully.
    Connected,
    /// File descriptor closed.
    Closed,
    /// Fsync completed successfully.
    Synced,
    /// Opened file descriptor.
    Opened(RawFd),
    /// Stat result.
    StatResult(StatxData),
    /// Datagram bytes sent.
    DatagramSent(usize),
    /// Datagram bytes received with peer address and exact data buffer.
    DatagramReceived {
        bytes: usize,
        data: Vec<u8>,
        addr: SocketAddr,
    },
    /// Directory entry filename list.
    DirList(Vec<Vec<u8>>),
    /// Generic successful completion.
    Completed,
}

/// Completion emitted by a ring for a submitted operation.
#[derive(Debug)]
pub struct IoCompletion {
    /// Operation id returned by [`CompletionRing::submit`].
    pub op_id: u64,
    /// Backend-decoded result.
    pub result: io::Result<IoResult>,
}

/// Called by a ring each time it publishes a completion, so the ring's one
/// consumer can wait for completions instead of polling for them
/// (EMBEDDER-COMPOSITION-SPEC §3.8.1 W13, B-181 R3).
pub type CompletionNotifier = std::sync::Arc<dyn Fn() + Send + Sync>;

/// The notifier a ring holds, shared with the threads that publish its
/// completions. Installed once; a ring publishes into it after every
/// completion becomes visible to `poll_completions`.
#[derive(Clone, Default)]
pub(crate) struct NotifierSlot(std::sync::Arc<std::sync::OnceLock<CompletionNotifier>>);

impl NotifierSlot {
    /// Install the notifier. Refused (false) when one is installed already.
    pub(crate) fn install(&self, notifier: CompletionNotifier) -> bool {
        self.0.set(notifier).is_ok()
    }

    /// Tell the consumer a completion was published.
    pub(crate) fn notify(&self) {
        if let Some(notifier) = self.0.get() {
            notifier();
        }
    }
}

impl std::fmt::Debug for NotifierSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotifierSlot")
            .field("installed", &self.0.get().is_some())
            .finish()
    }
}

/// Completion-ring interface used by platform-specific I/O backends.
pub trait CompletionRing: Send + Sync {
    /// Submit an operation and return its ring-assigned monotonically increasing operation id.
    fn submit(&self, op: IoOp) -> u64;

    /// Poll for completions, waiting up to `timeout` for the first completion.
    fn poll_completions(&self, timeout: Duration) -> Vec<IoCompletion>;

    /// Return the number of operations submitted but not yet completed.
    fn pending_count(&self) -> usize;

    /// OS thread names of this ring's LIVE worker pool, in spawn order.
    ///
    /// Empty by default: backends whose completions are serviced by the kernel
    /// or an inline queue (io_uring, the replay/failed rings) own no named OS
    /// worker threads. The blocking thread-pool fallback overrides this so the
    /// service inventory (spec §5) can attribute its workers by name; a
    /// shut-down pool reports empty.
    fn worker_thread_names(&self) -> Vec<String> {
        Vec::new()
    }

    /// Worker threads this ring was ASKED to run, independent of how many
    /// spawned successfully or remain live — the `configured` half of the
    /// service inventory's requested-vs-live split (spec §5). Zero by default
    /// for backends that own no OS worker threads.
    fn requested_worker_count(&self) -> usize {
        0
    }

    /// Install the notifier this ring calls after every completion it
    /// publishes. Returns false when the ring cannot notify (the default, for
    /// a ring that only supports polling) or a notifier is installed already;
    /// a consumer that parks waiting for completions must refuse such a ring
    /// rather than wait on it.
    fn install_completion_notifier(&self, notifier: CompletionNotifier) -> bool {
        drop(notifier);
        false
    }

    /// Stop accepting work and cleanly shut down backend workers.
    fn shutdown(&self);
}

/// Ring used only when a requested platform backend cannot be constructed.
#[cfg(target_os = "linux")]
pub(crate) struct FailedRing {
    next_op_id: AtomicU64,
    error_kind: io::ErrorKind,
    error_message: String,
    completions: Mutex<VecDeque<IoCompletion>>,
    notifier: NotifierSlot,
}

#[cfg(target_os = "linux")]
impl FailedRing {
    pub(crate) fn new(error: io::Error) -> Self {
        Self {
            next_op_id: AtomicU64::new(1),
            error_kind: error.kind(),
            error_message: error.to_string(),
            completions: Mutex::new(VecDeque::new()),
            notifier: NotifierSlot::default(),
        }
    }
}

#[cfg(target_os = "linux")]
impl CompletionRing for FailedRing {
    fn submit(&self, _op: IoOp) -> u64 {
        let op_id = self.next_op_id.fetch_add(1, Ordering::Relaxed);
        let completion = IoCompletion {
            op_id,
            result: Err(io::Error::new(
                self.error_kind,
                format!(
                    "completion ring backend unavailable: {}",
                    self.error_message
                ),
            )),
        };
        if let Ok(mut completions) = self.completions.lock() {
            completions.push_back(completion);
        }
        // W13: the completion is published on the submitting thread, which
        // may not be the consumer; notify after the push is visible.
        self.notifier.notify();
        op_id
    }

    fn poll_completions(&self, _timeout: Duration) -> Vec<IoCompletion> {
        self.completions
            .lock()
            .map(|mut completions| completions.drain(..).collect())
            .unwrap_or_default()
    }

    fn pending_count(&self) -> usize {
        0
    }

    fn install_completion_notifier(&self, notifier: CompletionNotifier) -> bool {
        self.notifier.install(notifier)
    }

    fn shutdown(&self) {}
}
