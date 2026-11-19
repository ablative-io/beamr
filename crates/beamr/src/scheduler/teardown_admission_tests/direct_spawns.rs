//! Direct spawns at joined shutdown, and joining a native slice before its
//! body is recorded or removed.

use super::*;

#[test]
fn joined_shutdown_refuses_direct_spawns_without_allocating_identity() {
    let fixture = TerminalFixture::new();
    let (module, function) = direct_spawn_module(&fixture.scheduler);
    fixture.scheduler.shutdown();
    let shared = &fixture.scheduler.shared;
    let before = (
        shared.next_pid.load(Ordering::Relaxed),
        shared.process_table.len(),
        shared.process_bodies.len(),
        shared.exit_tombstones.len(),
        fixture
            .scheduler
            .inject_queues
            .iter()
            .map(|queue| queue.len())
            .sum::<usize>(),
    );
    let results = [
        fixture.scheduler.spawn(module, function, Vec::new()),
        fixture
            .scheduler
            .spawn_trap_exit(module, function, Vec::new()),
    ];
    let after = (
        shared.next_pid.load(Ordering::Relaxed),
        shared.process_table.len(),
        shared.process_bodies.len(),
        shared.exit_tombstones.len(),
        fixture
            .scheduler
            .inject_queues
            .iter()
            .map(|queue| queue.len())
            .sum::<usize>(),
    );
    // Release baseline-admitted ownership before evaluating the recorded red.
    for queue in &fixture.scheduler.inject_queues {
        while let Some(request) = queue.pop() {
            drop(request);
        }
    }
    for pid in results.iter().flatten() {
        execution::cleanup_exited_process(shared, *pid, ExitReason::Killed);
    }
    for result in &results {
        assert!(result.is_err(), "joined scheduler admitted a direct spawn");
        assert_eq!(
            result.as_ref().err().map(ToString::to_string),
            Some("scheduler spawn admission is closed".to_owned()),
        );
    }
    assert_eq!(
        after, before,
        "refusal allocated process identity or terminal state"
    );
}

#[test]
fn shutdown_joins_native_slice_before_recording_or_removing_its_body() {
    let fixture = TerminalFixture::new();
    let (entered, inside) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let worker_release = Arc::clone(&release);
    let pid = fixture
        .scheduler
        .spawn_native(Box::new(move || {
            Box::new(BlockingSpawnHandler {
                entered: Some(entered.clone()),
                release: Arc::clone(&worker_release),
            })
        }))
        .unwrap_or_else(|error| panic!("held native spawn: {error}"));
    inside
        .recv()
        .unwrap_or_else(|error| panic!("native slice entry: {error}"));
    let (dispatch, closing) = mpsc::channel();
    let dispatch_signalled = AtomicBool::new(false);
    fixture
        .scheduler
        .shared
        .shutdown_dispatch_hook
        .set(Box::new(move || {
            if !dispatch_signalled.swap(true, Ordering::SeqCst) {
                dispatch
                    .send(())
                    .unwrap_or_else(|error| panic!("shutdown dispatch: {error}"));
            }
        }))
        .unwrap_or_else(|_| panic!("shutdown dispatch hook already installed"));
    let (stored, store_back) = mpsc::channel();
    let shared = Arc::downgrade(&fixture.scheduler.shared);
    let joined_signalled = AtomicBool::new(false);
    fixture
        .scheduler
        .shared
        .terminal_admission_hook
        .set(Box::new(move || {
            if joined_signalled.swap(true, Ordering::SeqCst) {
                return;
            }
            let shared = shared
                .upgrade()
                .unwrap_or_else(|| panic!("scheduler owner missing"));
            let present = shared
                .process_bodies
                .get(&pid)
                .is_some_and(|entry| matches!(&*lock_or_recover(&entry), ProcessSlot::Present(_)));
            stored
                .send((
                    present,
                    shared.exit_tombstones.contains_key(&pid),
                    shared.process_table.get(pid).is_some(),
                ))
                .unwrap_or_else(|error| panic!("joined store-back: {error}"));
        }))
        .unwrap_or_else(|_| panic!("terminal admission hook already installed"));
    let scheduler = Arc::clone(&fixture.scheduler);
    let closer = std::thread::spawn(move || scheduler.shutdown());
    closing
        .recv()
        .unwrap_or_else(|error| panic!("shutdown admission closed: {error}"));
    let before_release = (
        fixture.scheduler.shared.exit_tombstones.contains_key(&pid),
        fixture.scheduler.shared.process_table.get(pid).is_some(),
    );
    release.wait();
    let joined = store_back
        .recv()
        .unwrap_or_else(|error| panic!("joined body observation: {error}"));
    closer
        .join()
        .unwrap_or_else(|_| panic!("shutdown panicked"));
    assert_eq!(before_release, (false, true));
    assert_eq!(joined, (true, false, true));
    assert!(fixture.scheduler.shared.process_table.get(pid).is_none());
    assert!(fixture.scheduler.shared.exit_tombstones.contains_key(&pid));
}
