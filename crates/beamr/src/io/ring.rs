//! Backend-agnostic completion I/O ring types.

use std::io;
use std::net::SocketAddr;
use std::os::fd::RawFd;
use std::time::Duration;

/// Platform-independent description of an asynchronous I/O operation.
#[derive(Debug, Clone)]
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
    /// Connect an existing socket to `addr`.
    Connect { fd: RawFd, addr: SocketAddr },
    /// Close a file descriptor.
    Close { fd: RawFd },
    /// Synchronise a file descriptor.
    Fsync { fd: RawFd },
    /// Open `path` relative to `dir_fd`.
    Openat {
        dir_fd: RawFd,
        path: String,
        flags: i32,
        mode: u32,
    },
    /// Query file status for `path` relative to `dir_fd`.
    Statx {
        dir_fd: RawFd,
        path: String,
        flags: i32,
        mask: u32,
    },
    /// Complete without performing I/O.
    Nop,
}

/// Completion payload returned by a [`CompletionRing`].
#[derive(Debug)]
pub struct IoCompletion {
    /// Monotonic operation identifier assigned during submission.
    pub op_id: u64,
    /// Backend-neutral result for the completed operation.
    pub result: io::Result<IoResult>,
}

/// Backend-neutral operation result values.
#[derive(Debug, Clone)]
pub enum IoResult {
    /// Read byte count and owned buffer containing the bytes read.
    BytesRead(usize, Vec<u8>),
    /// Number of bytes written.
    BytesWritten(usize),
    /// Accepted file descriptor and peer address.
    Accepted(RawFd, SocketAddr),
    /// Connect completed successfully.
    Connected,
    /// Close completed successfully.
    Closed,
    /// Sync completed successfully.
    Synced,
    /// Newly opened file descriptor.
    Opened(RawFd),
    /// File metadata returned by `statx` or a fallback metadata query.
    StatResult(StatxData),
    /// Generic successful completion.
    Completed,
}

/// Backend-neutral subset of Linux `statx` metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatxData {
    pub mask: u32,
    pub block_size: u32,
    pub attributes: u64,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub attributes_mask: u64,
    pub atime_sec: i64,
    pub atime_nsec: u32,
    pub btime_sec: i64,
    pub btime_nsec: u32,
    pub ctime_sec: i64,
    pub ctime_nsec: u32,
    pub mtime_sec: i64,
    pub mtime_nsec: u32,
    pub rdev_major: u32,
    pub rdev_minor: u32,
    pub dev_major: u32,
    pub dev_minor: u32,
}

/// Platform-specific completion I/O implementation.
pub trait CompletionRing: Send + Sync {
    /// Submit an I/O operation and return its monotonic operation id.
    fn submit(&self, op: IoOp) -> u64;

    /// Poll for completions, waiting up to `timeout` for the first result.
    fn poll_completions(&self, timeout: Duration) -> Vec<IoCompletion>;

    /// Return the number of operations submitted but not yet completed.
    fn pending_count(&self) -> usize;

    /// Shut the ring down and release worker resources.
    fn shutdown(&self);
}
