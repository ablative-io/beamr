//! Supervision integration tests — verify that exit signal propagation,
//! DOWN message delivery, and cascade deaths work correctly through the
//! scheduler's cleanup_exited_process path.

use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};

use dashmap::{DashMap, DashSet};

use super::*;
use crate::atom::Atom;
use crate::distribution::DistributionConfig;
use crate::ets::{EtsTableMetadata, EtsTableType, Protection};
use crate::io::resource::{FD_RESOURCE_WORDS, FdInner, FdState, write_fd_resource};
use crate::process::registry::ProcessTable;
use crate::process::{ExitReason, ProcessStatus, RemotePid};
use crate::scheduler::execution::{
    cleanup_exited_process, cleanup_if_tombstoned_after_store, store_runnable_process,
};
use crate::supervision::link::LinkSet;
use crate::supervision::monitor::MonitorSet;
use crate::term::Term;
use crate::term::boxed::{self, Tuple};

mod ets_ownership;
mod executing_targets;
mod fd_resources;
mod links;
mod monitors;
mod remote_links;

/// Helper: insert a running process into shared state with the given pid.
pub(super) fn insert_process(shared: &SharedState, pid: u64) -> u64 {
    insert_process_in(shared, pid, NamespaceId::DEFAULT)
}

/// Helper: insert a running process assigned to `namespace`.
fn insert_process_in(shared: &SharedState, pid: u64, namespace: NamespaceId) -> u64 {
    shared.process_table.spawn_with_pid(pid);
    let mut process = Process::new(pid, 64);
    process.set_namespace_id(namespace);
    process
        .transition_to(ProcessStatus::Running)
        .unwrap_or_else(|error| panic!("process {pid} starts: {error}"));
    shared.process_bodies.insert(
        pid,
        std::sync::Mutex::new(ProcessSlot::Present(ScheduledProcess(process))),
    );
    pid
}

/// Helper: read a tuple from the front of a process's mailbox.
pub(super) fn read_mailbox_tuple(shared: &SharedState, pid: u64) -> Option<Vec<Term>> {
    let entry = shared.process_bodies.get(&pid)?;
    let mut slot = lock_or_recover(&entry);
    let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
        return None;
    };
    process.mailbox_mut().drain_arrival();
    let msg = process.mailbox().front_for_test()?;
    let tuple = Tuple::new(msg)?;
    let mut elems = Vec::with_capacity(tuple.arity());
    for i in 0..tuple.arity() {
        elems.push(tuple.get(i).unwrap_or(Term::NIL));
    }
    Some(elems)
}

/// Helper: check if a process is still in the process table.
pub(super) fn is_alive(shared: &SharedState, pid: u64) -> bool {
    shared.process_table.get(pid).is_some()
}

/// Helper: add a bidirectional link between two process bodies.
fn add_link(shared: &SharedState, a: u64, b: u64) {
    if let Some(entry) = shared.process_bodies.get(&a) {
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Present(ScheduledProcess(p)) = &mut *slot {
            p.add_link(b);
        }
    }
    if let Some(entry) = shared.process_bodies.get(&b) {
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Present(ScheduledProcess(p)) = &mut *slot {
            p.add_link(a);
        }
    }
}

pub(super) fn add_remote_link(shared: &SharedState, pid: u64, remote: RemotePid) {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process {pid} exists"));
    let mut slot = lock_or_recover(&entry);
    let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot else {
        panic!("process {pid} is present");
    };
    assert!(process.add_remote_link(remote));
}

/// Helper: set trap_exit on a process.
pub(super) fn set_trap_exit(shared: &SharedState, pid: u64, value: bool) {
    if let Some(entry) = shared.process_bodies.get(&pid) {
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Present(ScheduledProcess(p)) = &mut *slot {
            p.set_trap_exit(value);
        }
    }
}

/// Helper: add a monitor from watcher to target. Returns the reference.
pub(super) fn add_monitor(shared: &SharedState, watcher_pid: u64, target_pid: u64) -> u64 {
    let mut ms = lock_or_recover(&shared.monitor_set);
    let reference = ms.allocate_reference_pub();
    let monitor = crate::process::Monitor::new(reference, watcher_pid, target_pid);
    ms.register_monitor(reference, monitor, target_pid);
    drop(ms);

    if let Some(entry) = shared.process_bodies.get(&watcher_pid) {
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Present(ScheduledProcess(p)) = &mut *slot {
            p.add_monitor(monitor);
        }
    }
    if let Some(entry) = shared.process_bodies.get(&target_pid) {
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Present(ScheduledProcess(p)) = &mut *slot {
            p.add_monitor(monitor);
        }
    }

    reference
}

pub(super) fn make_executing(shared: &SharedState, pid: u64) -> Process {
    let entry = shared
        .process_bodies
        .get(&pid)
        .unwrap_or_else(|| panic!("process {pid} exists"));
    let mut slot = lock_or_recover(&entry);
    match std::mem::take(&mut *slot) {
        ProcessSlot::Present(ScheduledProcess(process)) => {
            let metadata = ProcessMetadata {
                namespace_id: process.namespace_id(),
                relations: process.shared_relations(),
                trap_exit: process.trap_exit(),
                priority: process.priority(),
                current_mfa: process.current_mfa(),
                heap_size: process.heap().total_used(),
                binary_heap_size: process.virtual_binary_heap(),
                message_queue_len: process.mailbox().message_count(),
                group_leader: process.group_leader(),
                logical_clock: process.logical_clock(),
                capabilities: process.capabilities().clone(),
                pending_exit_messages: Vec::new(),
                pending_down_messages: Vec::new(),
                pending_io_messages: Vec::new(),
                pending_distribution_payloads: Vec::new(),
                pending_local_messages: Vec::new(),
                pending_ets_transfer_messages: Vec::new(),
                pending_udp_messages: Vec::new(),
                pending_tcp_messages: Vec::new(),
            };
            *slot = ProcessSlot::Executing(metadata);
            process
        }
        other => {
            *slot = other;
            panic!("process {pid} is present before executing transition");
        }
    }
}

pub(super) fn make_shared_state() -> Arc<SharedState> {
    build_shared_state(false, "local@test")
}

/// [`make_shared_state`] plus a live [`DistSender`] (its owned runtime drives
/// the control-lane drain and, via the shared runtime handle, any test
/// connections' read loops) and a resolvable `local_node` name — the pieces
/// wire-level outbound-control tests need.
pub(super) fn make_shared_state_with_dist_sender() -> Arc<SharedState> {
    build_shared_state(true, "local@test")
}

/// [`make_shared_state_with_dist_sender`] under an explicit node name, for
/// tests that stand up TWO shared states as distinct nodes on a socket pair
/// (each side's origin/misaddressing checks need the names to differ).
pub(super) fn make_shared_state_with_dist_sender_named(node_name: &str) -> Arc<SharedState> {
    build_shared_state(true, node_name)
}

fn build_shared_state(with_dist_sender: bool, node_name: &str) -> Arc<SharedState> {
    let module_registry = Arc::new(ModuleRegistry::new());
    let namespace_store = DashMap::new();
    namespace_store.insert(NamespaceId::DEFAULT, Arc::clone(&module_registry));
    // The wire variant needs resolvable common atoms (exit-reason atoms are
    // encoded by name), matching production; the base fixture keeps the
    // historical empty table so existing atom-index assumptions hold.
    let atom_table = Arc::new(if with_dist_sender {
        crate::atom::AtomTable::with_common_atoms()
    } else {
        crate::atom::AtomTable::new()
    });
    // One owned distribution bundle (spec §3.6): a single heartbeat-enabled
    // manager backs the sender AND the net-kernel facade — no second manager.
    // The outbound sender is built only for the wire variant (`with_dist_sender`),
    // mirroring the historical fixture split.
    let distribution = super::service::ServiceMode::Owned(
        super::distribution_service::DistributionService::build(
            DistributionConfig::default(),
            Arc::clone(&atom_table),
            node_name,
            0,
            with_dist_sender,
        ),
    );
    let local_node = if with_dist_sender {
        crate::distribution::Node::new(atom_table.intern(node_name), 0)
    } else {
        crate::distribution::Node::new(crate::atom::Atom::new(0), 0)
    };

    let service_instances = super::inventory::ServiceInstances::mint(false);
    let (dirty_completion_shutdown_tx, dirty_completion_shutdown_rx) =
        crossbeam_channel::bounded::<()>(0);
    Arc::new(SharedState {
        shutdown: AtomicBool::new(false),
        failure: super::failure::FailureLatch::new(1),
        worker_exits: super::failure::WorkerExits::new(1),
        shutdown_skipped: Mutex::new(Vec::new()),
        process_table: ProcessTable::new(),
        module_registry,
        namespace_store,
        next_namespace_id: AtomicU64::new(1),
        spawn_counter: AtomicUsize::new(0),
        thread_count: 1,
        dirty_cpu: super::service::ServiceMode::Owned(dirty::DirtyPool::with_queue_depth(
            "dirty-test-cpu",
            1,
            1,
        )),
        dirty_io: super::service::ServiceMode::Owned(dirty::DirtyPool::with_queue_depth(
            "dirty-test-io",
            1,
            1,
        )),
        next_pid: AtomicU64::new(100),
        wait_set: std::sync::Mutex::new(WaitSet::default()),
        wake_condvar: std::sync::Condvar::new(),
        idle_wake: super::execution::IdleWake::default(),
        process_bodies: DashMap::new(),
        exit_tombstones: exit_tombstones::BoundedTombstones::new(),
        exit_completion_watches: Arc::new(exit_events::ExitCompletionRegistry::new()),
        exit_results: DashMap::new(),
        exit_errors: DashMap::new(),
        exit_exceptions: DashMap::new(),
        suspensions: DashMap::new(),
        suspension_results: DashMap::new(),
        pending_resumes: DashMap::new(),
        link_set: std::sync::Mutex::new(LinkSet::new()),
        monitor_set: std::sync::Mutex::new(MonitorSet::new()),
        hook: crate::hook::Hook::new(),
        distribution,
        process_registry: DashMap::new(),
        timers: Arc::new(std::sync::Mutex::new(crate::timer::TimerWheel::new())),
        expired_receive_timers: DashMap::new(),
        output_sink: std::sync::Mutex::new(Arc::new(crate::io::NullSink)),
        io_ring: super::service::ServiceMode::Disabled,
        io_registry: None,
        io_bridge: std::sync::Mutex::new(None),
        io_facility: None,
        atom_table: Arc::clone(&atom_table),
        ets_registry: Arc::new(crate::ets::EtsRegistry::new(atom_table)),
        pg_registry: Arc::new(crate::distribution::pg::PgRegistry::new(
            &crate::atom::AtomTable::with_common_atoms(),
        )),
        bif_registry: Arc::new(crate::native::BifRegistryImpl::new()),
        capability_policy: Arc::new(crate::native::AllCapabilitiesPolicy),
        idle_parks: AtomicUsize::new(0),
        park_seams: super::execution::ParkSeams::new(1),
        park_hooks: super::execution::ParkHooks::default(),
        suspension_mirror_registrations: AtomicU64::new(0),
        dirty_suspension_allocations: AtomicU64::new(0),
        park_gap_hook: Mutex::new(None),
        mailbox_admission_hook: std::sync::OnceLock::new(),
        terminal_admission_hook: std::sync::OnceLock::new(),
        terminal_spawn_visits: AtomicUsize::new(0),
        terminal_body_visits: AtomicUsize::new(0),
        spawn_admission_hook: std::sync::OnceLock::new(),
        teardown_wait_hook: std::sync::OnceLock::new(),
        shutdown_dispatch_hook: std::sync::OnceLock::new(),
        file_io_ring: super::service::ServiceMode::Disabled,
        file_io_pending: DashMap::new(),
        file_io_orphans: DashMap::new(),
        file_io_results: DashMap::new(),
        file_io_canceled: DashSet::new(),
        standard_io_pid: u64::MAX,
        #[cfg(feature = "readiness")]
        readiness: super::service::ServiceMode::Disabled,
        #[cfg(feature = "readiness")]
        readiness_consumer: None,
        service_instances,
        dirty_completion_spawns: AtomicU64::new(0),
        dirty_completions: Mutex::new(super::TeardownAdmissionRegistry::default()),
        dirty_completions_changed: Condvar::new(),
        dirty_completion_shutdown_tx: Mutex::new(Some(dirty_completion_shutdown_tx)),
        dirty_completion_shutdown_rx,
        standard_io: super::service::ServiceMode::Disabled,
        local_node,
        jit_profiler: Arc::new(crate::jit::JitProfiler::new(1000)),
        jit_compiler: Arc::new(
            crate::jit::JitCompiler::new(crate::jit::JitSettings)
                .expect("host JIT compiler should initialize"),
        ),
        jit_cache: Arc::new(crate::jit::JitCache::new()),
        jit_enabled: AtomicBool::new(true),
        replay_driver: None,
        replay_mode: false,
        nif_private_data: None,
        #[cfg(feature = "telemetry")]
        telemetry_metrics: TelemetryMetricState::new(std::time::Duration::from_millis(100)),
    })
}
