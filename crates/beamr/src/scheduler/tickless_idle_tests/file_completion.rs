//! B-181 R3: a file completion wakes a parked worker 0 by an edge (W13,
//! W14).

use super::*;

/// A native that parks, then reports the slice it was woken into.
struct ReportsWake {
    woken: crossbeam_channel::Sender<u64>,
    slices: usize,
}

impl NativeHandler for ReportsWake {
    fn handle(&mut self, context: &mut NativeContext<'_>) -> NativeOutcome {
        self.slices += 1;
        if self.slices == 1 {
            return NativeOutcome::Wait;
        }
        self.woken
            .send(context.self_pid())
            .unwrap_or_else(|error| panic!("publish wake: {error}"));
        NativeOutcome::Stop(ExitReason::Normal)
    }
}

/// R3 red: a file completion published while worker 0 is parked wakes it by
/// an edge, not by its clock. The test submits a read on the scheduler's own
/// file ring and registers it for a parked process the way the file facility
/// does. The `waiting` hook completes the read from inside worker 0's next
/// wait-start, so worker 0 is parked, every time, when the ring publishes;
/// the `left_park` hook reports how worker 0 left that park. At the base
/// the ring had no notifier and worker 0 found the completion only on its
/// 5 ms timeout (W13, W14).
#[test]
fn a_file_completion_wakes_a_process_with_every_worker_parked() {
    let registry = Arc::new(ModuleRegistry::new());
    let scheduler = scheduler_with_workers(&registry, 2);
    let (registered_sender, registered) = mpsc::channel();
    let registered_sender = Mutex::new(registered_sender);
    *lock_or_recover(&scheduler.shared.park_gap_hook) = Some(Box::new(move |shared, gap, pid| {
        if gap == ParkGap::WaitRegistered && pid != shared.standard_io_pid {
            lock_or_recover(&registered_sender)
                .send(pid)
                .unwrap_or_else(|error| panic!("publish registration: {error}"));
        }
    }));
    let (woken_sender, woken) = crossbeam_channel::unbounded();
    let pid = scheduler
        .spawn_native(Box::new(move || {
            Box::new(ReportsWake {
                woken: woken_sender.clone(),
                slices: 0,
            })
        }))
        .unwrap_or_else(|error| panic!("spawn file waiter: {error}"));
    assert_eq!(
        registered
            .recv()
            .unwrap_or_else(|error| panic!("waiter registers: {error}")),
        pid
    );
    *lock_or_recover(&scheduler.shared.park_gap_hook) = None;
    // The read blocks in the ring's worker until the test side writes, so
    // the completion is published only when the waiting hook writes.
    let (held, ring_side) = std::os::unix::net::UnixStream::pair()
        .unwrap_or_else(|error| panic!("socket pair: {error}"));
    let ring = scheduler
        .shared
        .file_io_ring
        .service()
        .unwrap_or_else(|| panic!("the default scheduler owns a file ring"));
    let op_id = ring.ring().submit(crate::io::IoOp::Read {
        fd: std::os::fd::AsRawFd::as_raw_fd(&ring_side),
        buf_len: 1,
        offset: u64::MAX,
    });
    scheduler
        .shared
        .file_io_pending
        .insert(op_id, (pid, crate::native::FileIoContinuation::Open));
    // Worker 0 leaves its park after the write: how, reported once.
    let written = Arc::new(AtomicBool::new(false));
    let (exits_sender, exits) = mpsc::channel();
    let exits_sender = Mutex::new(exits_sender);
    let exit_written = Arc::clone(&written);
    let installed = scheduler
        .shared
        .park_hooks
        .left_park
        .set(Box::new(move |left| {
            if left.worker == 0 && exit_written.swap(false, Ordering::AcqRel) {
                lock_or_recover(&exits_sender)
                    .send(left.by_clock_without_edge)
                    .unwrap_or_else(|error| panic!("publish worker 0's exit: {error}"));
            }
        }));
    assert!(installed.is_ok(), "left-park hook installed once");
    // The write happens in worker 0's wait-start, with wait_set held and
    // before the wait: worker 0 is parked when the ring publishes.
    let held = Mutex::new(Some(held));
    let hook_written = Arc::clone(&written);
    let nudged = Arc::new(AtomicBool::new(false));
    let hook_nudged = Arc::clone(&nudged);
    let installed = scheduler
        .shared
        .park_hooks
        .waiting
        .set(Box::new(move |worker, _timeout| {
            // Only the wait-start after the nudge ran: its notify is spent,
            // so nothing but the completion can end the wait that follows.
            if worker != 0 || !hook_nudged.load(Ordering::Acquire) {
                return;
            }
            if let Some(stream) = lock_or_recover(&held).take() {
                std::io::Write::write_all(&mut &stream, b"x")
                    .unwrap_or_else(|error| panic!("complete the read: {error}"));
                hook_written.store(true, Ordering::Release);
            }
        }));
    assert!(installed.is_ok(), "waiting hook installed once");
    // Pass worker 0 through one more wait-start: a native runs on worker 0
    // and stops, and worker 0 parks after it.
    let nudge = scheduler
        .spawn_native(Box::new(move || {
            Box::new(StopsAtOnce {
                ran: Arc::clone(&nudged),
            })
        }))
        .unwrap_or_else(|error| panic!("spawn nudge: {error}"));
    assert_eq!(await_exit(&scheduler, nudge), ExitReason::Normal);
    let by_clock = exits
        .recv()
        .unwrap_or_else(|error| panic!("worker 0 leaves its park: {error}"));
    assert!(
        !by_clock,
        "worker 0 left its park on the clock with no wake edge after the file \
         completion was published: the completion has no wake edge"
    );
    assert_eq!(
        woken
            .recv()
            .unwrap_or_else(|error| panic!("completion wakes the waiter: {error}")),
        pid
    );
    assert_eq!(await_exit(&scheduler, pid), ExitReason::Normal);
    scheduler.shutdown();
}

/// A native that marks that it ran, then stops in its first slice.
struct StopsAtOnce {
    ran: Arc<AtomicBool>,
}

impl NativeHandler for StopsAtOnce {
    fn handle(&mut self, _context: &mut NativeContext<'_>) -> NativeOutcome {
        self.ran.store(true, Ordering::Release);
        NativeOutcome::Stop(ExitReason::Normal)
    }
}
