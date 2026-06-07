#![cfg(not(target_os = "linux"))]

//! Blocking thread-pool completion ring for non-Linux development.
//!
//! This backend simulates completion semantics with blocking std I/O on worker
//! threads. It is a development fallback; production async I/O is provided by
//! the Linux `io_uring` backend.

use super::ring::{CompletionRing, IoCompletion, IoOp, IoResult, StatxData};
use crossbeam_channel::{Receiver, Sender};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEFAULT_POOL_SIZE: usize = 4;
const AT_FDCWD: RawFd = -100;

enum WorkMessage {
    Op(u64, IoOp),
    Shutdown,
}

/// Non-Linux blocking development fallback for [`CompletionRing`].
pub struct ThreadPoolRing {
    work_sender: Sender<WorkMessage>,
    result_sender: Sender<IoCompletion>,
    result_receiver: Receiver<IoCompletion>,
    next_op_id: AtomicU64,
    pending: Arc<AtomicUsize>,
    shutdown: AtomicBool,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl ThreadPoolRing {
    /// Create a blocking fallback ring with `pool_size` workers.
    #[must_use]
    pub fn new(pool_size: usize) -> Self {
        let worker_count = if pool_size == 0 {
            DEFAULT_POOL_SIZE
        } else {
            pool_size
        };
        let (work_sender, work_receiver) = crossbeam_channel::unbounded();
        let (result_sender, result_receiver) = crossbeam_channel::unbounded();
        let pending = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::with_capacity(worker_count);

        for index in 0..worker_count {
            let worker_receiver = work_receiver.clone();
            let worker_results = result_sender.clone();
            let worker_pending = Arc::clone(&pending);
            let builder = thread::Builder::new().name(format!("beamr-io-fallback-{index}"));
            match builder
                .spawn(move || worker_loop(worker_receiver, worker_results, worker_pending))
            {
                Ok(handle) => workers.push(handle),
                Err(error) => {
                    eprintln!("failed to spawn beamr IO fallback worker {index}: {error}")
                }
            }
        }

        Self {
            work_sender,
            result_sender,
            result_receiver,
            next_op_id: AtomicU64::new(1),
            pending,
            shutdown: AtomicBool::new(false),
            workers: Mutex::new(workers),
        }
    }
}

impl CompletionRing for ThreadPoolRing {
    fn submit(&self, op: IoOp) -> u64 {
        let op_id = self.next_op_id.fetch_add(1, Ordering::Relaxed);
        self.pending.fetch_add(1, Ordering::Relaxed);

        let result = if self.shutdown.load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "completion ring is shut down",
            ))
        } else {
            self.work_sender
                .send(WorkMessage::Op(op_id, op))
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "IO worker channel closed"))
        };

        if let Err(error) = result {
            self.pending.fetch_sub(1, Ordering::Relaxed);
            let _ = self.result_sender.send(IoCompletion {
                op_id,
                result: Err(error),
            });
        }

        op_id
    }

    fn poll_completions(&self, timeout: Duration) -> Vec<IoCompletion> {
        let mut completions = Vec::new();
        match self.result_receiver.recv_timeout(timeout) {
            Ok(completion) => completions.push(completion),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => return completions,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return completions,
        }

        while let Ok(completion) = self.result_receiver.try_recv() {
            completions.push(completion);
        }

        completions
    }

    fn pending_count(&self) -> usize {
        self.pending.load(Ordering::Relaxed)
    }

    fn shutdown(&self) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return;
        }

        let worker_count = match self.workers.lock() {
            Ok(workers) => workers.len(),
            Err(_) => 0,
        };
        for _ in 0..worker_count {
            let _ = self.work_sender.send(WorkMessage::Shutdown);
        }

        if let Ok(mut workers) = self.workers.lock() {
            while let Some(handle) = workers.pop() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for ThreadPoolRing {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker_loop(
    receiver: Receiver<WorkMessage>,
    result_sender: Sender<IoCompletion>,
    pending: Arc<AtomicUsize>,
) {
    while let Ok(message) = receiver.recv() {
        match message {
            WorkMessage::Op(op_id, op) => {
                let result = execute_op(op);
                pending.fetch_sub(1, Ordering::Relaxed);
                let _ = result_sender.send(IoCompletion { op_id, result });
            }
            WorkMessage::Shutdown => break,
        }
    }
}

fn execute_op(op: IoOp) -> io::Result<IoResult> {
    match op {
        IoOp::Read {
            fd,
            buf_len,
            offset,
        } => {
            let file = file_for_fd(fd, false)?;
            let mut buffer = vec![0; buf_len];
            let bytes = file.read_at(&mut buffer, offset)?;
            buffer.truncate(bytes);
            Ok(IoResult::BytesRead(bytes, buffer))
        }
        IoOp::Write { fd, data, offset } => {
            let file = file_for_fd(fd, true)?;
            let bytes = file.write_at(&data, offset)?;
            Ok(IoResult::BytesWritten(bytes))
        }
        IoOp::Accept { listener_fd: _ } => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "accept is not supported by the non-Linux thread-pool fallback",
        )),
        IoOp::Connect { fd: _, addr: _ } => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "connect is not supported by the non-Linux thread-pool fallback",
        )),
        IoOp::Close { fd } => {
            close_fd(fd);
            Ok(IoResult::Closed)
        }
        IoOp::Fsync { fd } => {
            let file = file_for_fd(fd, true)?;
            file.sync_all()?;
            Ok(IoResult::Synced)
        }
        IoOp::Openat {
            dir_fd,
            path,
            flags,
            mode,
        } => {
            let file = open_path(dir_fd, Path::new(&path), flags, mode)?;
            let fd = std::os::fd::IntoRawFd::into_raw_fd(file);
            Ok(IoResult::Opened(fd))
        }
        IoOp::Statx {
            dir_fd,
            path,
            flags: _,
            mask: _,
        } => {
            let path = resolve_path(dir_fd, Path::new(&path));
            let metadata = std::fs::metadata(path)?;
            Ok(IoResult::StatResult(statx_from_metadata(&metadata)))
        }
        IoOp::Nop => Ok(IoResult::Completed),
    }
}

fn file_for_fd(fd: RawFd, write: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(write);
    options.open(format!("/dev/fd/{fd}"))
}

fn close_fd(fd: RawFd) {
    if let Ok(file) = file_for_fd(fd, true) {
        drop(file);
    }
}

fn open_path(dir_fd: RawFd, path: &Path, flags: i32, mode: u32) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(flags & 0x0001 != 0)
        .create(flags & 0x0200 != 0);
    options.mode(mode);
    options.open(resolve_path(dir_fd, path))
}

fn resolve_path(dir_fd: RawFd, path: &Path) -> PathBuf {
    if path.is_absolute() || dir_fd == AT_FDCWD {
        return path.to_path_buf();
    }

    PathBuf::from(format!("/dev/fd/{dir_fd}")).join(path)
}

fn statx_from_metadata(metadata: &std::fs::Metadata) -> StatxData {
    StatxData {
        mode: metadata.mode() as u16,
        nlink: metadata.nlink() as u32,
        uid: metadata.uid(),
        gid: metadata.gid(),
        ino: metadata.ino(),
        size: metadata.size(),
        blocks: metadata.blocks(),
        atime_sec: metadata.atime(),
        atime_nsec: metadata.atime_nsec() as u32,
        ctime_sec: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec() as u32,
        mtime_sec: metadata.mtime(),
        mtime_nsec: metadata.mtime_nsec() as u32,
        dev_major: metadata.dev() as u32,
        ..StatxData::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::AsRawFd;

    #[test]
    fn read_write_temp_file_completes() {
        let ring = ThreadPoolRing::new(2);
        let mut file = tempfile_like_file();
        let fd = file.as_raw_fd();

        let write_id = ring.submit(IoOp::Write {
            fd,
            data: b"beamr".to_vec(),
            offset: 0,
        });
        let completions = ring.poll_completions(Duration::from_secs(2));
        assert!(
            completions
                .iter()
                .any(|completion| completion.op_id == write_id)
        );

        file.seek(SeekFrom::Start(0)).expect("seek temp file");
        let mut check = String::new();
        file.read_to_string(&mut check).expect("read temp file");
        assert_eq!(check, "beamr");

        let read_id = ring.submit(IoOp::Read {
            fd,
            buf_len: 5,
            offset: 0,
        });
        let completions = ring.poll_completions(Duration::from_secs(2));
        let completion = completions
            .into_iter()
            .find(|completion| completion.op_id == read_id)
            .expect("read completion");
        match completion.result.expect("read result") {
            IoResult::BytesRead(count, bytes) => {
                assert_eq!(count, 5);
                assert_eq!(bytes, b"beamr");
            }
            other => panic!("unexpected result: {other:?}"),
        }

        ring.shutdown();
    }

    fn tempfile_like_file() -> File {
        let mut path = std::env::temp_dir();
        path.push(format!("beamr-thread-pool-ring-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("create temp file");
        let _ = std::fs::remove_file(path);
        file.write_all(&[]).expect("initialise temp file");
        file
    }
}
