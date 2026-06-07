#![cfg(target_os = "linux")]

//! Linux `io_uring` completion ring backend.

use super::ring::{CompletionRing, IoCompletion, IoOp, IoResult, StatxData};
use crossbeam_channel::{Receiver, Sender};
use io_uring::{IoUring, opcode, types};
use std::collections::HashMap;
use std::ffi::CString;
use std::io;
use std::mem::MaybeUninit;
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEFAULT_RING_DEPTH: u32 = 256;

enum RingMessage {
    Submit(u64, IoOp),
    Shutdown,
}

enum PendingOp {
    Read {
        buffer: Vec<u8>,
    },
    Write {
        data: Vec<u8>,
    },
    Accept {
        storage: Box<MaybeUninit<libc::sockaddr_storage>>,
        len: Box<libc::socklen_t>,
    },
    Connect {
        storage: Box<libc::sockaddr_storage>,
    },
    Close,
    Fsync,
    Openat {
        _path: CString,
    },
    Statx {
        _path: CString,
        data: Box<MaybeUninit<libc::statx>>,
    },
    Nop,
}

/// Linux `io_uring` implementation of [`CompletionRing`].
pub struct IoUringRing {
    submit_sender: Sender<RingMessage>,
    completion_sender: Sender<IoCompletion>,
    completion_receiver: Receiver<IoCompletion>,
    next_op_id: AtomicU64,
    pending: Arc<AtomicUsize>,
    shutdown: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl IoUringRing {
    /// Create an `io_uring` backend with the requested ring depth.
    pub fn new(ring_depth: u32) -> io::Result<Self> {
        let depth = if ring_depth == 0 {
            DEFAULT_RING_DEPTH
        } else {
            ring_depth
        };
        let ring = IoUring::new(depth)?;
        let (submit_sender, submit_receiver) = crossbeam_channel::unbounded();
        let (completion_sender, completion_receiver) = crossbeam_channel::unbounded();
        let thread_completions = completion_sender.clone();
        let pending = Arc::new(AtomicUsize::new(0));
        let thread_pending = Arc::clone(&pending);
        let handle = thread::Builder::new()
            .name("beamr-io-uring".to_string())
            .spawn(move || ring_thread(ring, submit_receiver, thread_completions, thread_pending))
            .map_err(|error| {
                io::Error::other(format!("failed to spawn io_uring thread: {error}"))
            })?;

        Ok(Self {
            submit_sender,
            completion_sender,
            completion_receiver,
            next_op_id: AtomicU64::new(1),
            pending,
            shutdown: AtomicBool::new(false),
            thread: Mutex::new(Some(handle)),
        })
    }
}

impl CompletionRing for IoUringRing {
    fn submit(&self, op: IoOp) -> u64 {
        let op_id = self.next_op_id.fetch_add(1, Ordering::Relaxed);
        self.pending.fetch_add(1, Ordering::Relaxed);
        let result = if self.shutdown.load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "completion ring is shut down",
            ))
        } else {
            self.submit_sender
                .send(RingMessage::Submit(op_id, op))
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "io_uring thread closed"))
        };

        if let Err(error) = result {
            self.pending.fetch_sub(1, Ordering::Relaxed);
            let _ = self.completion_sender.send(IoCompletion {
                op_id,
                result: Err(error),
            });
        }

        op_id
    }

    fn poll_completions(&self, timeout: Duration) -> Vec<IoCompletion> {
        let mut completions = Vec::new();
        match self.completion_receiver.recv_timeout(timeout) {
            Ok(completion) => completions.push(completion),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => return completions,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return completions,
        }
        while let Ok(completion) = self.completion_receiver.try_recv() {
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
        let _ = self.submit_sender.send(RingMessage::Shutdown);
        if let Ok(mut thread) = self.thread.lock() {
            if let Some(handle) = thread.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for IoUringRing {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn ring_thread(
    mut ring: IoUring,
    receiver: Receiver<RingMessage>,
    completions: Sender<IoCompletion>,
    pending_count: Arc<AtomicUsize>,
) {
    let mut pending = HashMap::new();
    let mut shutting_down = false;

    while !shutting_down || !pending.is_empty() {
        while let Ok(message) = receiver.try_recv() {
            match message {
                RingMessage::Submit(op_id, op) => submit_to_ring(
                    &mut ring,
                    op_id,
                    op,
                    &mut pending,
                    &completions,
                    &pending_count,
                ),
                RingMessage::Shutdown => shutting_down = true,
            }
        }

        let _ = ring.submit();
        for cqe in ring.completion() {
            let op_id = cqe.user_data();
            let result_code = cqe.result();
            if let Some(op) = pending.remove(&op_id) {
                let result = decode_completion(result_code, op);
                pending_count.fetch_sub(1, Ordering::Relaxed);
                let _ = completions.send(IoCompletion { op_id, result });
            }
        }

        if !shutting_down && pending.is_empty() {
            match receiver.recv_timeout(Duration::from_millis(1)) {
                Ok(RingMessage::Submit(op_id, op)) => submit_to_ring(
                    &mut ring,
                    op_id,
                    op,
                    &mut pending,
                    &completions,
                    &pending_count,
                ),
                Ok(RingMessage::Shutdown) => shutting_down = true,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => shutting_down = true,
            }
        }
    }
}

fn submit_to_ring(
    ring: &mut IoUring,
    op_id: u64,
    op: IoOp,
    pending: &mut HashMap<u64, PendingOp>,
    completions: &Sender<IoCompletion>,
    pending_count: &AtomicUsize,
) {
    match build_entry(op) {
        Ok((entry, pending_op)) => {
            let entry = entry.user_data(op_id);
            // SAFETY: The entry references buffers/path/socket storage stored in
            // `pending` before the kernel can complete the SQE, and that storage
            // remains alive until the matching CQE is decoded and removed.
            let push_result = unsafe { ring.submission().push(&entry) };
            match push_result {
                Ok(()) => {
                    pending.insert(op_id, pending_op);
                    let _ = ring.submit();
                }
                Err(_) => send_error(
                    op_id,
                    io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "io_uring submission queue is full",
                    ),
                    completions,
                    pending_count,
                ),
            }
        }
        Err(error) => send_error(op_id, error, completions, pending_count),
    }
}

fn build_entry(op: IoOp) -> io::Result<(io_uring::squeue::Entry, PendingOp)> {
    match op {
        IoOp::Read {
            fd,
            buf_len,
            offset,
        } => {
            let len = u32::try_from(buf_len).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "read buffer too large")
            })?;
            let mut buffer = vec![0; buf_len];
            let entry = opcode::Read::new(types::Fd(fd), buffer.as_mut_ptr(), len)
                .offset(offset)
                .build();
            Ok((entry, PendingOp::Read { buffer }))
        }
        IoOp::Write { fd, data, offset } => {
            let len = u32::try_from(data.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "write buffer too large")
            })?;
            let entry = opcode::Write::new(types::Fd(fd), data.as_ptr(), len)
                .offset(offset)
                .build();
            Ok((entry, PendingOp::Write { data }))
        }
        IoOp::Accept { listener_fd } => {
            let mut storage = Box::new(MaybeUninit::<libc::sockaddr_storage>::uninit());
            let mut len =
                Box::new(std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t);
            let entry = opcode::Accept::new(
                types::Fd(listener_fd),
                storage.as_mut_ptr().cast::<libc::sockaddr>(),
                len.as_mut(),
            )
            .build();
            Ok((entry, PendingOp::Accept { storage, len }))
        }
        IoOp::Connect { fd, addr } => {
            let (storage, len) = socket_addr_storage(addr);
            let entry = opcode::Connect::new(
                types::Fd(fd),
                storage.as_ref() as *const libc::sockaddr_storage as *const libc::sockaddr,
                len,
            )
            .build();
            Ok((entry, PendingOp::Connect { storage }))
        }
        IoOp::Close { fd } => Ok((opcode::Close::new(types::Fd(fd)).build(), PendingOp::Close)),
        IoOp::Fsync { fd } => Ok((opcode::Fsync::new(types::Fd(fd)).build(), PendingOp::Fsync)),
        IoOp::Openat {
            dir_fd,
            path,
            flags,
            mode,
        } => {
            let path = CString::new(path)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
            let entry = opcode::OpenAt::new(dir_fd, path.as_ptr())
                .flags(flags)
                .mode(mode as libc::mode_t)
                .build();
            Ok((entry, PendingOp::Openat { _path: path }))
        }
        IoOp::Statx {
            dir_fd,
            path,
            flags,
            mask,
        } => {
            let path = CString::new(path)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
            let mut data = Box::new(MaybeUninit::<libc::statx>::uninit());
            let entry = opcode::Statx::new(
                dir_fd,
                path.as_ptr(),
                data.as_mut_ptr().cast::<io_uring::types::statx>(),
            )
            .flags(flags)
            .mask(mask)
            .build();
            Ok((entry, PendingOp::Statx { _path: path, data }))
        }
        IoOp::Nop => Ok((opcode::Nop::new().build(), PendingOp::Nop)),
    }
}

fn decode_completion(result_code: i32, op: PendingOp) -> io::Result<IoResult> {
    if result_code < 0 {
        return Err(io::Error::from_raw_os_error(-result_code));
    }

    match op {
        PendingOp::Read { mut buffer } => {
            let count = result_code as usize;
            buffer.truncate(count);
            Ok(IoResult::BytesRead(count, buffer))
        }
        PendingOp::Write { data: _ } => Ok(IoResult::BytesWritten(result_code as usize)),
        PendingOp::Accept { storage, len } => {
            // SAFETY: A successful accept CQE means the kernel initialised the
            // sockaddr buffer before posting completion.
            let storage = unsafe { storage.assume_init() };
            let addr = socket_addr_from_storage(&storage, *len)?;
            Ok(IoResult::Accepted(result_code, addr))
        }
        PendingOp::Connect { storage: _ } => Ok(IoResult::Connected),
        PendingOp::Close => Ok(IoResult::Closed),
        PendingOp::Fsync => Ok(IoResult::Synced),
        PendingOp::Openat { _path: _ } => Ok(IoResult::Opened(result_code)),
        PendingOp::Statx { _path: _, data } => {
            // SAFETY: A successful statx CQE means the kernel initialised the
            // statx buffer supplied with the SQE before completion was posted.
            let stat = unsafe { data.assume_init() };
            Ok(IoResult::StatResult(statx_from_linux(stat)))
        }
        PendingOp::Nop => Ok(IoResult::Completed),
    }
}

fn send_error(
    op_id: u64,
    error: io::Error,
    completions: &Sender<IoCompletion>,
    pending_count: &AtomicUsize,
) {
    pending_count.fetch_sub(1, Ordering::Relaxed);
    let _ = completions.send(IoCompletion {
        op_id,
        result: Err(error),
    });
}

fn socket_addr_storage(addr: SocketAddr) -> (Box<libc::sockaddr_storage>, libc::socklen_t) {
    match addr {
        SocketAddr::V4(addr) => {
            let raw = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: addr.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_be_bytes(addr.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            let storage = sockaddr_storage_from(raw);
            (
                storage,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        }
        SocketAddr::V6(addr) => {
            let raw = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: addr.port().to_be(),
                sin6_flowinfo: addr.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: addr.ip().octets(),
                },
                sin6_scope_id: addr.scope_id(),
            };
            let storage = sockaddr_storage_from(raw);
            (
                storage,
                std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
            )
        }
    }
}

fn sockaddr_storage_from<T>(raw: T) -> Box<libc::sockaddr_storage> {
    // SAFETY: A zeroed sockaddr_storage is valid plain C storage. It is then
    // overwritten at the front with a concrete sockaddr_in or sockaddr_in6.
    let mut storage =
        Box::new(unsafe { MaybeUninit::<libc::sockaddr_storage>::zeroed().assume_init() });
    // SAFETY: sockaddr_storage is large/aligned enough for both sockaddr_in and
    // sockaddr_in6, the only types passed by callers of this helper.
    unsafe {
        std::ptr::write(
            (&mut *storage as *mut libc::sockaddr_storage).cast::<T>(),
            raw,
        )
    };
    storage
}

fn socket_addr_from_storage(
    storage: &libc::sockaddr_storage,
    len: libc::socklen_t,
) -> io::Result<SocketAddr> {
    match storage.ss_family as i32 {
        libc::AF_INET if len as usize >= std::mem::size_of::<libc::sockaddr_in>() => {
            // SAFETY: Family and length confirm that storage contains sockaddr_in.
            let raw =
                unsafe { *(storage as *const libc::sockaddr_storage).cast::<libc::sockaddr_in>() };
            Ok(SocketAddr::V4(SocketAddrV4::new(
                std::net::Ipv4Addr::from(raw.sin_addr.s_addr.to_be_bytes()),
                u16::from_be(raw.sin_port),
            )))
        }
        libc::AF_INET6 if len as usize >= std::mem::size_of::<libc::sockaddr_in6>() => {
            // SAFETY: Family and length confirm that storage contains sockaddr_in6.
            let raw =
                unsafe { *(storage as *const libc::sockaddr_storage).cast::<libc::sockaddr_in6>() };
            Ok(SocketAddr::V6(SocketAddrV6::new(
                std::net::Ipv6Addr::from(raw.sin6_addr.s6_addr),
                u16::from_be(raw.sin6_port),
                raw.sin6_flowinfo,
                raw.sin6_scope_id,
            )))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported accepted socket address family",
        )),
    }
}

fn statx_from_linux(stat: libc::statx) -> StatxData {
    StatxData {
        mask: stat.stx_mask,
        block_size: stat.stx_blksize,
        attributes: stat.stx_attributes,
        nlink: stat.stx_nlink,
        uid: stat.stx_uid,
        gid: stat.stx_gid,
        mode: stat.stx_mode,
        ino: stat.stx_ino,
        size: stat.stx_size,
        blocks: stat.stx_blocks,
        attributes_mask: stat.stx_attributes_mask,
        atime_sec: stat.stx_atime.tv_sec,
        atime_nsec: stat.stx_atime.tv_nsec,
        btime_sec: stat.stx_btime.tv_sec,
        btime_nsec: stat.stx_btime.tv_nsec,
        ctime_sec: stat.stx_ctime.tv_sec,
        ctime_nsec: stat.stx_ctime.tv_nsec,
        mtime_sec: stat.stx_mtime.tv_sec,
        mtime_nsec: stat.stx_mtime.tv_nsec,
        rdev_major: stat.stx_rdev_major,
        rdev_minor: stat.stx_rdev_minor,
        dev_major: stat.stx_dev_major,
        dev_minor: stat.stx_dev_minor,
    }
}
