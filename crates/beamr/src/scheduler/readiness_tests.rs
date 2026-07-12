use super::process_slot::{ProcessMetadata, ProcessSlot};
use super::*;
use crate::atom::Atom;
use crate::module::ModuleRegistry;
use crate::namespace::NamespaceId;
use crate::process::Process;
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::term::Term;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn scheduler_with(services: SchedulerServices) -> Scheduler {
    Scheduler::with_services(
        SchedulerConfig {
            thread_count: Some(1),
            ..SchedulerConfig::default()
        },
        services,
        Arc::new(ModuleRegistry::new()),
    )
    .unwrap_or_else(|error| panic!("readiness test scheduler starts: {error}"))
}

fn add_executing_process(scheduler: &Scheduler, pid: u64) {
    scheduler.shared.process_table.spawn_with_pid(pid);
    let process = Process::new(pid, DEFAULT_HEAP_SIZE);
    scheduler.shared.process_bodies.insert(
        pid,
        Mutex::new(ProcessSlot::Executing(ProcessMetadata {
            namespace_id: NamespaceId::DEFAULT,
            capabilities: process.capabilities().clone(),
            links: Vec::new(),
            remote_links: Vec::new(),
            monitors: Vec::new(),
            trap_exit: false,
            priority: process.priority(),
            current_mfa: None,
            heap_size: 0,
            binary_heap_size: 0,
            message_queue_len: 0,
            group_leader: process.group_leader(),
            logical_clock: process.logical_clock(),
            pending_exit_messages: Vec::new(),
            pending_down_messages: Vec::new(),
            pending_io_messages: Vec::new(),
            pending_distribution_payloads: Vec::new(),
            pending_local_messages: Vec::new(),
            pending_ets_transfer_messages: Vec::new(),
            pending_udp_messages: Vec::new(),
            pending_tcp_messages: Vec::new(),
        })),
    );
}

fn pending_markers(scheduler: &Scheduler, pid: u64) -> Vec<Term> {
    let entry = scheduler
        .shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("pid {pid} body exists"));
    let slot = lock_or_recover(&entry);
    match &*slot {
        ProcessSlot::Executing(metadata) => metadata.pending_io_messages.clone(),
        _ => panic!("readiness test process remains checked out"),
    }
}

fn wait_for_marker_count(scheduler: &Scheduler, pid: u64, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if pending_markers(scheduler, pid).len() == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("pid {pid} did not receive {expected} readiness markers");
}

fn socket_pair() -> (UnixStream, UnixStream) {
    let pair = UnixStream::pair().unwrap_or_else(|error| panic!("socket pair: {error}"));
    pair.0
        .set_nonblocking(true)
        .unwrap_or_else(|error| panic!("nonblocking reader: {error}"));
    pair.1
        .set_nonblocking(true)
        .unwrap_or_else(|error| panic!("nonblocking writer: {error}"));
    pair
}

#[test]
fn shared_routes_home_survives_peer_shutdown_and_refuses_straggler() {
    let shared = SharedReadiness::new().unwrap_or_else(|error| panic!("shared poller: {error}"));
    let scheduler_a = scheduler_with(SchedulerServices::minimal().shared_readiness(shared.clone()));
    let scheduler_b = scheduler_with(SchedulerServices::minimal().shared_readiness(shared.clone()));
    add_executing_process(&scheduler_a, 11);
    add_executing_process(&scheduler_b, 22);
    let (reader_a, mut writer_a) = socket_pair();
    let (reader_b, mut writer_b) = socket_pair();
    let marker_a = Atom::new(401);
    let marker_b = Atom::new(402);
    let token_a = scheduler_a
        .shared
        .readiness_register(reader_a.as_raw_fd(), Interest::READABLE, 11, marker_a)
        .unwrap_or_else(|error| panic!("register A: {error}"));
    let token_b = scheduler_b
        .shared
        .readiness_register(reader_b.as_raw_fd(), Interest::READABLE, 22, marker_b)
        .unwrap_or_else(|error| panic!("register B: {error}"));
    writer_a
        .write_all(&[1])
        .unwrap_or_else(|error| panic!("fire A: {error}"));
    writer_b
        .write_all(&[1])
        .unwrap_or_else(|error| panic!("fire B: {error}"));
    wait_for_marker_count(&scheduler_a, 11, 1);
    wait_for_marker_count(&scheduler_b, 22, 1);
    assert_eq!(
        pending_markers(&scheduler_a, 11),
        vec![Term::atom(marker_a)]
    );
    assert_eq!(
        pending_markers(&scheduler_b, 22),
        vec![Term::atom(marker_b)]
    );

    scheduler_a.shutdown();
    assert_eq!(
        scheduler_a
            .shared
            .readiness_rearm(&token_a, Interest::READABLE),
        Err(ReadinessError::TeardownInProgress)
    );
    let (straggler, _peer) = socket_pair();
    assert_eq!(
        scheduler_a.shared.readiness_register(
            straggler.as_raw_fd(),
            Interest::READABLE,
            11,
            marker_a,
        ),
        Err(ReadinessError::TeardownInProgress)
    );
    scheduler_b
        .shared
        .readiness_rearm(&token_b, Interest::READABLE)
        .unwrap_or_else(|error| panic!("rearm B: {error}"));
    writer_b
        .write_all(&[2])
        .unwrap_or_else(|error| panic!("refire B: {error}"));
    wait_for_marker_count(&scheduler_b, 22, 2);
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(pending_markers(&scheduler_a, 11).len(), 1);
    scheduler_b.shutdown();
    drop(shared);
}

#[test]
fn failed_poller_refuses_and_deregister_recovers_poisoned_table() {
    let scheduler = scheduler_with(SchedulerServices::minimal().owned_readiness());
    add_executing_process(&scheduler, 33);
    let (reader, mut writer) = socket_pair();
    let marker = Atom::new(403);
    let token = scheduler
        .shared
        .readiness_register(reader.as_raw_fd(), Interest::READABLE, 33, marker)
        .unwrap_or_else(|error| panic!("register healthy: {error}"));
    scheduler
        .shared
        .readiness
        .service()
        .unwrap_or_else(|| panic!("owned readiness exists"))
        .panic_next_delivery();
    writer
        .write_all(&[1])
        .unwrap_or_else(|error| panic!("fire panic seam: {error}"));
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let entry = scheduler
            .service_inventory()
            .into_iter()
            .find(|entry| entry.service == inventory::READINESS)
            .unwrap_or_else(|| panic!("readiness inventory line"));
        if entry.actual == 0 {
            assert_eq!(entry.configured, 1);
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let (other, _peer) = socket_pair();
    assert_eq!(
        scheduler
            .shared
            .readiness_register(other.as_raw_fd(), Interest::READABLE, 33, marker,),
        Err(ReadinessError::ServiceFailed)
    );
    let started = Instant::now();
    scheduler.readiness_deregister(token);
    assert!(started.elapsed() < Duration::from_secs(1));
    scheduler.shutdown();

    let healthy = scheduler_with(SchedulerServices::minimal().owned_readiness());
    add_executing_process(&healthy, 44);
    let (reader, _writer) = socket_pair();
    let token = healthy
        .shared
        .readiness_register(reader.as_raw_fd(), Interest::READABLE, 44, marker)
        .unwrap_or_else(|error| panic!("healthy positive-control register: {error}"));
    healthy.readiness_deregister(token);
    healthy.shutdown();
}

#[test]
fn deregistration_is_bounded_and_invalid_fd_refuses_without_record() {
    let scheduler = scheduler_with(SchedulerServices::minimal().owned_readiness());
    add_executing_process(&scheduler, 55);
    let marker = Atom::new(404);
    assert!(matches!(
        scheduler
            .shared
            .readiness_register(-1, Interest::READABLE, 55, marker),
        Err(ReadinessError::Register { errno }) if errno != 0
    ));
    let (reader, mut writer) = socket_pair();
    let token = scheduler
        .shared
        .readiness_register(reader.as_raw_fd(), Interest::READABLE, 55, marker)
        .unwrap_or_else(|error| panic!("epoch register: {error}"));
    let started = Instant::now();
    scheduler.readiness_deregister(token);
    assert!(started.elapsed() < Duration::from_secs(1));
    writer
        .write_all(&[1])
        .unwrap_or_else(|error| panic!("post-dereg fire: {error}"));
    std::thread::sleep(Duration::from_millis(20));
    assert!(pending_markers(&scheduler, 55).is_empty());
    scheduler.shutdown();
}

#[test]
fn stale_generation_never_routes_to_recycled_slot_on_other_consumer() {
    let shared = SharedReadiness::new().unwrap_or_else(|error| panic!("shared poller: {error}"));
    let scheduler_a = scheduler_with(SchedulerServices::minimal().shared_readiness(shared.clone()));
    let scheduler_b = scheduler_with(SchedulerServices::minimal().shared_readiness(shared.clone()));
    add_executing_process(&scheduler_a, 66);
    add_executing_process(&scheduler_b, 77);
    let (old_reader, _old_writer) = socket_pair();
    let old = scheduler_a
        .shared
        .readiness_register(
            old_reader.as_raw_fd(),
            Interest::READABLE,
            66,
            Atom::new(405),
        )
        .unwrap_or_else(|error| panic!("old generation register: {error}"));
    scheduler_a.readiness_deregister(old);

    let (new_reader, mut new_writer) = socket_pair();
    let marker = Atom::new(406);
    let new = scheduler_b
        .shared
        .readiness_register(new_reader.as_raw_fd(), Interest::READABLE, 77, marker)
        .unwrap_or_else(|error| panic!("recycled slot register: {error}"));
    shared.0.inject_stale_readable(old);
    std::thread::sleep(Duration::from_millis(20));
    assert!(pending_markers(&scheduler_b, 77).is_empty());

    new_writer
        .write_all(&[1])
        .unwrap_or_else(|error| panic!("fire current generation: {error}"));
    wait_for_marker_count(&scheduler_b, 77, 1);
    assert_eq!(pending_markers(&scheduler_b, 77), vec![Term::atom(marker)]);
    scheduler_b.readiness_deregister(new);
    scheduler_a.shutdown();
    scheduler_b.shutdown();
    drop(shared);
}

#[test]
fn disabled_has_zero_threads_and_owned_idle_poller_is_quiescent() {
    let disabled = scheduler_with(SchedulerServices::minimal());
    let entry = disabled
        .service_inventory()
        .into_iter()
        .find(|entry| entry.service == inventory::READINESS)
        .unwrap_or_else(|| panic!("disabled readiness inventory line"));
    assert_eq!((entry.configured, entry.actual), (0, 0));
    assert!(entry.thread_names.is_empty());
    disabled.shutdown();

    let owned = scheduler_with(SchedulerServices::minimal().owned_readiness());
    let before = owned
        .service_inventory()
        .into_iter()
        .find(|entry| entry.service == inventory::READINESS)
        .unwrap_or_else(|| panic!("owned readiness inventory line"));
    std::thread::sleep(Duration::from_millis(10));
    let iterations_before = owned
        .shared
        .readiness
        .service()
        .unwrap_or_else(|| panic!("owned readiness service"))
        .poll_iterations();
    std::thread::sleep(Duration::from_millis(100));
    let iterations_after = owned
        .shared
        .readiness
        .service()
        .unwrap_or_else(|| panic!("owned readiness service"))
        .poll_iterations();
    let after = owned
        .service_inventory()
        .into_iter()
        .find(|entry| entry.service == inventory::READINESS)
        .unwrap_or_else(|| panic!("owned readiness inventory line after soak"));
    assert_eq!(before, after);
    assert_eq!(
        iterations_before, iterations_after,
        "an idle readiness poller must remain parked for the whole soak"
    );
    assert_eq!(after.thread_names, vec!["beamr-readiness-poll"]);
    assert_eq!(after.fd_classes, vec!["poll", "waker"]);
    owned.shutdown();
}
