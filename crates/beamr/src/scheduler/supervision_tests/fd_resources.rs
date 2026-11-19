//! FD resources a process owns are closed when it exits, and only those.

use super::*;

fn allocate_fd_resource_for_process(shared: &SharedState, pid: u64, inner: Arc<FdInner>) -> RawFd {
    let fd = inner.fd();
    let entry = shared
        .process_bodies
        .get(&pid)
        .expect("process body exists");
    let mut slot = lock_or_recover(&entry);
    let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
        panic!("process {pid} is present");
    };
    let ptr = process
        .heap_mut()
        .alloc(FD_RESOURCE_WORDS)
        .expect("fd resource allocation fits");
    // SAFETY: heap allocation returned the fixed FdResource word count.
    let words = unsafe { std::slice::from_raw_parts_mut(ptr, FD_RESOURCE_WORDS) };
    let term = write_fd_resource(words, inner).expect("fd resource writer fits");
    process
        .heap_mut()
        .mark_last_young_allocation_maybe_refcounted();
    process.set_x_reg(0, term);
    fd
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

/// Captures the kernel identity (`st_dev`, `st_ino`) of an open descriptor.
/// Each `pipe(2)` instance has a unique inode, so this uniquely identifies the
/// descriptor independently of its fd *number*.
fn fd_identity(fd: RawFd) -> (libc::dev_t, libc::ino_t) {
    // SAFETY: `fstat` fully writes the stat buffer on success; we zero-init it
    // first so a failure path never reads uninitialized memory.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::fstat(fd, &mut st) };
    assert_eq!(rc, 0, "expected an open fd to fstat successfully");
    (st.st_dev, st.st_ino)
}

/// Reuse-immune closure check: returns `true` iff the descriptor that had
/// identity `original` is now closed, even under cargo's parallel test runner
/// where another thread may concurrently recycle the freed fd number.
///
/// - `fstat` fails (EBADF) -> the number is closed -> original closed.
/// - `fstat` succeeds with a *different* identity -> the number was recycled to
///   a new file/pipe, which still proves the original was closed.
/// - `fstat` succeeds with the *same* identity -> genuinely still open.
fn fd_closed_since(fd: RawFd, original: (libc::dev_t, libc::ino_t)) -> bool {
    // SAFETY: see `fd_identity`.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::fstat(fd, &mut st) };
    if rc != 0 {
        return true;
    }
    (st.st_dev, st.st_ino) != original
}

#[test]
fn cleanup_exited_process_closes_fd_resources_owned_by_process() {
    let shared = make_shared_state();
    let pid = insert_process(&shared, 1);
    let inner = Arc::new(FdInner::new(pipe_read_fd(), pid));
    allocate_fd_resource_for_process(&shared, pid, Arc::clone(&inner));

    cleanup_exited_process(&shared, pid, ExitReason::Normal);

    assert_eq!(inner.state(), FdState::Closed);
}

#[test]
fn cleanup_exited_process_does_not_explicitly_close_fd_resources_owned_elsewhere() {
    let shared = make_shared_state();
    let pid = insert_process(&shared, 1);
    let inner = Arc::new(FdInner::new(pipe_read_fd(), 999));
    let fd = allocate_fd_resource_for_process(&shared, pid, Arc::clone(&inner));
    let fd_id = fd_identity(fd);

    cleanup_exited_process(&shared, pid, ExitReason::Normal);

    assert!(!fd_closed_since(fd, fd_id));
    drop(inner);
    assert!(fd_closed_since(fd, fd_id));
}

#[test]
fn process_terminate_closes_owned_fd_resources_before_heap_reset() {
    let mut process = Process::new(42, 64);
    let fd = pipe_read_fd();
    let fd_id = fd_identity(fd);
    let ptr = process
        .heap_mut()
        .alloc(FD_RESOURCE_WORDS)
        .expect("fd resource allocation fits");
    // SAFETY: heap allocation returned the fixed FdResource word count.
    let words = unsafe { std::slice::from_raw_parts_mut(ptr, FD_RESOURCE_WORDS) };
    let term =
        write_fd_resource(words, Arc::new(FdInner::new(fd, 42))).expect("fd resource writer fits");
    process
        .heap_mut()
        .mark_last_young_allocation_maybe_refcounted();
    process.set_x_reg(0, term);

    process.terminate(ExitReason::Normal);

    assert!(fd_closed_since(fd, fd_id));
}
