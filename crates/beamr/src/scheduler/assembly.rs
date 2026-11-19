//! Assembling a scheduler from its services: the one path every constructor
//! reaches, which builds the shared state and starts the workers.

use super::distribution_service::DistributionService;
use super::execution::scheduler_loop;
use super::process_slot::ProcessSlot;
#[cfg(feature = "readiness")]
use super::readiness::ReadinessBuildError;
#[cfg(feature = "readiness")]
use super::readiness::{ReadinessService, RouteHome, ServiceConsumerId};
use super::ring_service::{RingService, StandardIoService};
use super::run_queue::{PriorityStealers, RunQueue};
use super::service::ServiceMode;
use super::services::SchedulerServices;
use super::{
    NO_STANDARD_IO_PID, ReplayMode, ScheduledProcess, Scheduler, SharedState, WaitSet,
    connection_lifecycle, dirty, execution, exit_events, exit_tombstones, failure, inventory,
    lock_or_recover, pg_propagation, services, supervision_integration,
};
use crate::atom::AtomTable;
use crate::distribution::DistributionConfig;
use crate::distribution::pg::PgRegistry;
use crate::distribution::{DEFAULT_NODE_NAME, Node};
use crate::ets::EtsRegistry;
use crate::hook::Hook;
use crate::io::{
    CompletionRing, CompletionRingIoFacility, FILE_IO_RING_THREAD_PREFIX,
    GENERIC_IO_RING_THREAD_PREFIX, IoCompletionBridge, IoFacility, IoWakeTarget, NullSink,
    PendingIoRegistry, RingConfig, STANDARD_IO_RING_THREAD_PREFIX, StandardIoServer,
    create_ring_with_prefix,
};
use crate::jit::{DEFAULT_JIT_THRESHOLD, JitCache, JitProfiler};
use crate::module::ModuleRegistry;
use crate::namespace::NamespaceId;
use crate::native::{BifRegistryImpl, CapabilityPolicy};
use crate::process::registry::ProcessTable;
use crate::replay::ReplayDriver;
use crate::supervision::link::LinkSet;
use crate::supervision::monitor::MonitorSet;
use crate::timer::TimerWheel;
use crossbeam_queue::SegQueue;
use dashmap::{DashMap, DashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, Condvar, Mutex};
#[cfg(feature = "telemetry")]
use std::time::Duration;

use super::config::SchedulerConfig;
use super::dirty_pools::resolve_dirty_pool;
use super::jit_control::build_jit_compiler;
use super::teardown_admission::TeardownAdmissionRegistry;
#[cfg(feature = "telemetry")]
use super::telemetry_metrics::TelemetryMetricState;

impl Scheduler {
    pub(super) fn construct_with_services(
        config: SchedulerConfig,
        services: SchedulerServices,
        module_registry: Arc<ModuleRegistry>,
        atom_table: Arc<AtomTable>,
        bif_registry: Arc<BifRegistryImpl>,
        capability_policy: Arc<dyn CapabilityPolicy>,
        replay_mode: ReplayMode,
    ) -> Result<Self, String> {
        let replay_driver = match replay_mode {
            ReplayMode::Live => None,
            ReplayMode::Replay(log) => Some(Arc::new(Mutex::new(ReplayDriver::new(log)))),
        };
        let replay_enabled = replay_driver.is_some();
        let thread_count = if replay_enabled {
            1
        } else {
            configured_thread_count(config.thread_count)
        };
        let dirty_queue_depth = config
            .dirty_queue_depth
            .unwrap_or(dirty::DEFAULT_DIRTY_QUEUE_DEPTH);
        // Dirty pools resolve from the composition choice, falling back to the
        // legacy `dirty_*_threads` knob (spec §2.2 precedence). `FromConfig`/
        // `Some(0)` still disables (no channel, no workers — refusal before any
        // suspension, spec §3.2/§6); an explicit `Owned(n)` overrides the knob;
        // an explicit `Shared` pool is injected and NEVER joined here (spec
        // §2.1 — safe now because dirty completion routes by the oneshot the
        // submission carries, not by any per-scheduler table).
        let dirty_cpu = resolve_dirty_pool(
            "dirty-cpu",
            &services.dirty_cpu,
            config.dirty_cpu_threads,
            num_cpus::get(),
            dirty_queue_depth,
        );
        let dirty_io = resolve_dirty_pool(
            "dirty-io",
            &services.dirty_io,
            config.dirty_io_threads,
            dirty::DEFAULT_DIRTY_IO_THREADS,
            dirty_queue_depth,
        );
        let jit_profiler = Arc::new(JitProfiler::new(
            config.jit_threshold.unwrap_or(DEFAULT_JIT_THRESHOLD),
        ));
        // One compiler per scheduler; a construction failure is fatal-loud
        // from the constructor (the ReadinessBuildError precedent), never a
        // silent degradation to interpreter-only.
        let jit_compiler = Arc::new(
            build_jit_compiler()
                .map_err(|error| format!("JIT compiler construction failed: {error}"))?,
        );
        #[cfg(feature = "telemetry")]
        let telemetry_sample_interval = config
            .telemetry_sample_interval
            .unwrap_or_else(|| Duration::from_millis(100));
        #[cfg(not(feature = "telemetry"))]
        let _telemetry_sample_interval = config.telemetry_sample_interval;
        let jit_cache = Arc::new(JitCache::new());
        // Generic ring config resolves from the composition choice, falling back
        // to `config.io` (spec §2.2 precedence / §3.5). Replay never builds a
        // live ring (commit-3 precedent). A `Shared` generic ring was already
        // refused by `validate()` before construction, so it cannot reach here.
        let generic_ring_config: Option<RingConfig> = if replay_enabled {
            None
        } else {
            match &services.generic_io {
                services::GenericRingChoice::FromConfig => config.io,
                services::GenericRingChoice::Disabled | services::GenericRingChoice::Shared(_) => {
                    None
                }
                services::GenericRingChoice::Owned(ring_config) => Some(*ring_config),
            }
        };
        let io_runtime = generic_ring_config.map(|ring_config| {
            let ring: Arc<dyn CompletionRing> = Arc::from(create_ring_with_prefix(
                ring_config,
                GENERIC_IO_RING_THREAD_PREFIX,
            ));
            let registry = Arc::new(PendingIoRegistry::default());
            let facility: Arc<dyn IoFacility> = Arc::new(CompletionRingIoFacility::new(
                Arc::clone(&ring),
                Arc::clone(&registry),
            ));
            (ring, registry, facility)
        });
        // Generic ring ownership (spec §3.5): `Disabled` (byte-identical to the
        // former `None` — no facility, no bridge) when unconfigured, `Owned`
        // when built here. The completion bridge and pending registry stay
        // scheduler-owned regardless of ring ownership.
        let (io_ring, io_registry, io_facility) = match io_runtime {
            Some((ring, registry, facility)) => (
                ServiceMode::Owned(RingService::new(ring)),
                Some(registry),
                Some(facility),
            ),
            None => (ServiceMode::Disabled, None, None),
        };
        // Distribution bundle (spec §3.6): resolves from the composition choice,
        // falling back to `config.distribution` (spec §2.2 precedence). `Owned`
        // builds ONE heartbeat-enabled manager backing listener/send/pg/control
        // traffic AND the net-kernel facade (the second disjoint manager is
        // deleted — the two-site acceptance line); `Disabled` builds NEITHER
        // runtime (honest absence — `SchedulerConfig::default()` carries no
        // distribution, so the legacy/default profile builds none; full-runtime
        // and the CLI opt in). The proactive net-tick detects a silently-
        // partitioned peer (no FIN/RST) within the liveness deadline so the
        // connection-down / pg-purge / monitor-DOWN machinery fires instead of
        // the link hanging "up" forever.
        //
        // Replay ⇒ Disabled bundle (pair ruling, commit 5): under replay the
        // bundle is effectively absent — NEITHER runtime is built, not just the
        // sender. This resolves the commit-4 inconsistency (replay skipped only
        // the sender yet still built the net-kernel runtime, whose live
        // `connect_node` dial performed real IO behind a disabled facade — the
        // exact commit-3 anti-pattern). No replay path reads live distribution
        // state; every distribution BIF already collapses to absence
        // (noconnection / false / []). The one flip is `is_alive/0`, which under
        // replay now reports `false` — spec-§3.6-consistent for a node with no
        // distribution service. See CHANGELOG and this commit's report.
        let dist_node_name = config.node_name.as_deref().unwrap_or(DEFAULT_NODE_NAME);
        let dist_creation = config.creation.unwrap_or(0);
        let distribution_config: Option<DistributionConfig> = if replay_enabled {
            None
        } else {
            match &services.distribution {
                services::DistributionChoice::FromConfig => config.distribution.clone(),
                services::DistributionChoice::Disabled => None,
                services::DistributionChoice::Owned(dist_config) => Some(dist_config.clone()),
            }
        };
        let distribution: ServiceMode<DistributionService> = match distribution_config {
            Some(dist_config) => ServiceMode::Owned(DistributionService::build(
                dist_config,
                Arc::clone(&atom_table),
                dist_node_name,
                dist_creation,
                // Owned distribution is only ever built live here (the replay
                // arm forced `None` above), so the outbound sender is always
                // wanted when a bundle exists.
                true,
            )),
            None => ServiceMode::Disabled,
        };
        let namespace_store = DashMap::new();
        namespace_store.insert(NamespaceId::DEFAULT, Arc::clone(&module_registry));
        // File-IO ring (spec §3.3): resolves from the composition choice,
        // falling back to the legacy default (spec §2.2 precedence). `Disabled`
        // — under replay (the file facility is then absent from native services,
        // so file submission refuses before registering any suspension) OR an
        // explicit `Disabled`/`minimal()` request — and `Owned` (a live ring
        // with the service-distinct thread name) for `FromConfig`/`Owned` in
        // live mode. A `Shared` file ring was refused by `validate()` before
        // construction, so it cannot reach here.
        let file_io_owned = !replay_enabled
            && match &services.file_io {
                services::FileRingChoice::FromConfig | services::FileRingChoice::Owned => true,
                services::FileRingChoice::Disabled | services::FileRingChoice::Shared(_) => false,
            };
        let file_io_ring: ServiceMode<RingService> = if file_io_owned {
            ServiceMode::Owned(RingService::new(Arc::from(create_ring_with_prefix(
                RingConfig::default(),
                FILE_IO_RING_THREAD_PREFIX,
            ))))
        } else {
            ServiceMode::Disabled
        };
        // Standard-IO ring + group-leader server (spec §3.4): resolves from the
        // composition choice, falling back to the legacy default. `Disabled` —
        // under replay OR an explicit `Disabled`/`minimal()` request — means NO
        // ring and NO process 0 (never a live ring behind a disabled facade,
        // whose completion poll loop would hang a normal worker forever); `Owned`
        // registers process 0. `standard_io_pid` follows: process 0 when Owned,
        // the no-such-pid sentinel otherwise. `minimal()` makes the Disabled
        // arm reachable on a LIVE scheduler for the first time (spec §3.4).
        let standard_io_owned = !replay_enabled
            && match &services.standard_io {
                services::StandardRingChoice::FromConfig | services::StandardRingChoice::Owned => {
                    true
                }
                services::StandardRingChoice::Disabled => false,
            };
        let standard_io_pid = if standard_io_owned {
            0u64
        } else {
            NO_STANDARD_IO_PID
        };
        let standard_io: ServiceMode<StandardIoService> = if standard_io_owned {
            let standard_io_ring: Arc<dyn CompletionRing> = Arc::from(create_ring_with_prefix(
                RingConfig::default(),
                STANDARD_IO_RING_THREAD_PREFIX,
            ));
            ServiceMode::Owned(StandardIoService::new(StandardIoServer::new(
                standard_io_pid,
                standard_io_ring,
                atom_table.as_ref(),
            )))
        } else {
            ServiceMode::Disabled
        };
        // The local node identity is retained and PASSIVE regardless of whether
        // distribution is owned (spec §3.6): `node/0` reports it even when the
        // bundle is `Disabled`. It shares the bundle's node name/creation.
        let local_node = Node::new(atom_table.intern(dist_node_name), dist_creation);
        let pg_registry = Arc::new(PgRegistry::new(atom_table.as_ref()));
        // The distribution bundle and each ring mint and carry their own identity
        // through their `ServiceMode`, so only the generic-IO bridge flag is
        // recorded here; the bridge is requested exactly when the generic registry
        // was built (spec §5).
        let service_instances = inventory::ServiceInstances::mint(io_registry.is_some());
        // Zero-capacity channel closed (sender dropped) by
        // `drain_dirty_completions`: every completion bridge selects on the
        // receiver, so closing it wakes them all to exit without delivering.
        let (dirty_completion_shutdown_tx, dirty_completion_shutdown_rx) =
            crossbeam_channel::bounded::<()>(0);
        #[cfg(feature = "readiness")]
        let readiness_error: std::cell::RefCell<Option<ReadinessBuildError>> =
            std::cell::RefCell::new(None);
        let shared = Arc::new_cyclic(|weak_shared| {
            #[cfg(feature = "readiness")]
            let route_home = RouteHome {
                scheduler: weak_shared.clone(),
                consumer: ServiceConsumerId::mint(),
            };
            #[cfg(feature = "readiness")]
            let readiness: ServiceMode<ReadinessService> = match &services.readiness {
                services::ReadinessChoice::FromConfig | services::ReadinessChoice::Disabled => {
                    ServiceMode::Disabled
                }
                services::ReadinessChoice::Owned => {
                    match ReadinessService::build_owned(route_home.clone()) {
                        Ok(service) => ServiceMode::Owned(service),
                        Err(error) => {
                            *readiness_error.borrow_mut() = Some(error);
                            ServiceMode::Disabled
                        }
                    }
                }
                services::ReadinessChoice::Shared(service) => {
                    ServiceMode::Shared(Arc::clone(&service.0))
                }
            };
            #[cfg(feature = "readiness")]
            let readiness_consumer = readiness
                .service()
                .map(|service| service.consumer(route_home));
            SharedState {
                shutdown: AtomicBool::new(false),
                failure: failure::FailureLatch::new(thread_count),
                worker_exits: failure::WorkerExits::new(thread_count),
                shutdown_skipped: Mutex::new(Vec::new()),
                process_table: ProcessTable::new(),
                module_registry,
                namespace_store,
                next_namespace_id: AtomicU64::new(1),
                // Cloned, not moved: the ETS registry two fields below needs
                // the SAME table. Struct-literal fields evaluate in source
                // order, so a shorthand `atom_table,` here would move the
                // local out of scope before `ets_registry` could name it.
                atom_table: Arc::clone(&atom_table),
                local_node,
                ets_registry: Arc::new(EtsRegistry::new(atom_table)),
                pg_registry,
                bif_registry,
                capability_policy,
                spawn_counter: AtomicUsize::new(0),
                thread_count,
                dirty_cpu,
                dirty_io,
                jit_profiler,
                jit_compiler,
                jit_cache,
                jit_enabled: AtomicBool::new(true),
                next_pid: AtomicU64::new(1),
                wait_set: Mutex::new(WaitSet::default()),
                wake_condvar: Condvar::new(),
                idle_wake: execution::IdleWake::default(),
                process_bodies: DashMap::new(),
                exit_tombstones: exit_tombstones::BoundedTombstones::new(),
                exit_completion_watches: Arc::new(exit_events::ExitCompletionRegistry::new()),
                exit_results: DashMap::new(),
                exit_errors: DashMap::new(),
                exit_exceptions: DashMap::new(),
                suspensions: DashMap::new(),
                suspension_results: DashMap::new(),
                pending_resumes: DashMap::new(),
                file_io_ring,
                file_io_pending: DashMap::new(),
                file_io_orphans: DashMap::new(),
                file_io_results: DashMap::new(),
                file_io_canceled: DashSet::new(),
                link_set: Mutex::new(LinkSet::new()),
                monitor_set: Mutex::new(MonitorSet::new()),
                hook: Hook::new(),
                distribution,
                process_registry: DashMap::new(),
                timers: Arc::new(Mutex::new(armed_wheel(weak_shared))),
                expired_receive_timers: DashMap::new(),
                output_sink: Mutex::new(Arc::new(NullSink)),
                io_ring,
                io_registry,
                io_bridge: Mutex::new(None),
                io_facility,
                standard_io_pid,
                #[cfg(feature = "readiness")]
                readiness,
                #[cfg(feature = "readiness")]
                readiness_consumer,
                service_instances,
                dirty_completion_spawns: AtomicU64::new(0),
                dirty_completions: Mutex::new(TeardownAdmissionRegistry::default()),
                dirty_completions_changed: Condvar::new(),
                dirty_completion_shutdown_tx: Mutex::new(Some(dirty_completion_shutdown_tx)),
                dirty_completion_shutdown_rx,
                replay_driver,
                replay_mode: replay_enabled,
                nif_private_data: config.nif_private_data,
                #[cfg(feature = "telemetry")]
                telemetry_metrics: TelemetryMetricState::new(telemetry_sample_interval),
                standard_io,
                #[cfg(any(test, feature = "test-support"))]
                idle_parks: AtomicUsize::new(0),
                #[cfg(any(test, feature = "test-support"))]
                park_seams: execution::ParkSeams::new(thread_count),
                #[cfg(test)]
                park_hooks: execution::ParkHooks::default(),
                #[cfg(any(test, feature = "test-support"))]
                suspension_mirror_registrations: AtomicU64::new(0),
                #[cfg(any(test, feature = "test-support"))]
                dirty_suspension_allocations: AtomicU64::new(0),
                #[cfg(test)]
                park_gap_hook: Mutex::new(None),
                #[cfg(test)]
                mailbox_admission_hook: std::sync::OnceLock::new(),
                #[cfg(test)]
                terminal_admission_hook: std::sync::OnceLock::new(),
                #[cfg(test)]
                terminal_spawn_visits: AtomicUsize::new(0),
                #[cfg(test)]
                terminal_body_visits: AtomicUsize::new(0),
                #[cfg(test)]
                spawn_admission_hook: std::sync::OnceLock::new(),
                #[cfg(test)]
                teardown_wait_hook: std::sync::OnceLock::new(),
                #[cfg(test)]
                shutdown_dispatch_hook: std::sync::OnceLock::new(),
            }
        });
        #[cfg(feature = "readiness")]
        if let Some(error) = readiness_error.into_inner() {
            return Err(error.to_string());
        }
        // W13 (B-181 R3): the Owned file ring runs edge E as it publishes each
        // completion, so worker 0 parks without polling it. A ring that cannot
        // notify is refused here, never waited on.
        if let Some(file_ring) = shared.file_io_ring.service() {
            let weak = Arc::downgrade(&shared);
            let installed = file_ring
                .ring()
                .install_completion_notifier(Arc::new(move || {
                    if let Some(shared) = weak.upgrade() {
                        shared.wake_edge();
                    }
                }));
            if !installed {
                return Err(
                    "the file-IO ring cannot notify its completions, so a parked worker 0 \
                     would never drain them"
                        .to_string(),
                );
            }
        }
        // Process 0 is registered exactly when the standard-IO ring is Owned
        // (spec §3.4): a Disabled standard ring (replay today) registers no
        // process 0, so group-leader IO has no server and the completion poll
        // loop can never hang a normal worker.
        if let Some(standard) = shared.standard_io.service() {
            let standard_io_pid = standard.server().pid();
            shared.process_table.spawn_with_pid(standard_io_pid);
            shared.process_bodies.insert(
                standard_io_pid,
                Mutex::new(ProcessSlot::Present(ScheduledProcess(
                    StandardIoServer::process(standard_io_pid),
                ))),
            );
        }
        #[cfg(feature = "telemetry")]
        shared.record_vm_health_metrics();
        supervision_integration::register_distribution_control_handler(&shared);
        // Install the real cross-node pg propagation now that `shared` exists.
        // Both the propagation backend and the connection-down hook hold a
        // `Weak<SharedState>` (not `Arc`) so they never keep the scheduler alive:
        // `SharedState` owns `pg_registry`, which owns the propagation, which would
        // otherwise own `SharedState` back and form a leak-forever cycle.
        shared
            .pg_registry
            .set_propagation(Arc::new(pg_propagation::SchedulerPgPropagation {
                shared: Arc::downgrade(&shared),
            }));
        // On node failure, the composed connection-event subscriber drops every
        // remote pg member that belonged to the lost node so group membership
        // reflects the surviving cluster. Registered at construction, before
        // any embedder can subscribe (INV-SCHED-FIRST).
        connection_lifecycle::register_scheduler_connection_subscriber(&shared);
        if !shared.replay_mode
            && let (Some(ring), Some(registry)) = (shared.io_ring.service(), &shared.io_registry)
        {
            let target: Arc<dyn IoWakeTarget> = shared.clone();
            let bridge =
                IoCompletionBridge::start(Arc::clone(ring.ring()), Arc::clone(registry), target)
                    .map_err(|error| {
                        format!("failed to spawn beamr-io-completion thread: {error}")
                    })?;
            *lock_or_recover(&shared.io_bridge) = Some(bridge);
        }
        let inject_queues: Vec<_> = (0..thread_count)
            .map(|_| Arc::new(SegQueue::new()))
            .collect();
        let barrier = Arc::new(std::sync::Barrier::new(thread_count + 1));
        let stealers_ready: Arc<Mutex<Option<Vec<PriorityStealers>>>> = Arc::new(Mutex::new(None));
        let mut stealer_receivers = Vec::with_capacity(thread_count);
        let mut threads = Vec::with_capacity(thread_count);
        let mut worker_names = Vec::with_capacity(thread_count);
        for (index, inject_queue) in inject_queues.iter().enumerate() {
            let (tx, rx) = std::sync::mpsc::channel();
            stealer_receivers.push(rx);
            let shared_for_thread = Arc::clone(&shared);
            let barrier_for_thread = Arc::clone(&barrier);
            let ready_for_thread = Arc::clone(&stealers_ready);
            let inject = Arc::clone(inject_queue);
            let thread_name = format!("beamr-sched-{index}");
            worker_names.push(thread_name.clone());
            let guard_name = thread_name.clone();
            let handle = std::thread::Builder::new()
                .name(thread_name.clone())
                .spawn(move || {
                    let queue = RunQueue::new();
                    if tx.send(queue.stealer()).is_err() {
                        return;
                    }
                    barrier_for_thread.wait();
                    let stealers = {
                        let guard = lock_or_recover(&ready_for_thread);
                        guard.as_ref().cloned().unwrap_or_default()
                    };
                    // B-184: every worker's loop runs under the guard. A death
                    // the recovery can account for restarts the loop here, on
                    // this thread, with this queue (the one its stealer reads)
                    // and this inject queue (R5); any other latches the
                    // scheduler's failure (R2).
                    failure::run_worker(&shared_for_thread, index, &guard_name, &inject, || {
                        scheduler_loop(&shared_for_thread, &queue, index, &stealers, &inject);
                    });
                })
                .map_err(|error| format!("failed to spawn {thread_name}: {error}"))?;
            threads.push(handle);
        }
        let mut stealers = Vec::with_capacity(thread_count);
        for rx in stealer_receivers {
            let stealer = rx
                .recv()
                .map_err(|error| format!("failed to receive scheduler stealer: {error}"))?;
            stealers.push(stealer);
        }
        {
            let mut guard = lock_or_recover(&stealers_ready);
            *guard = Some(stealers);
        }
        barrier.wait();
        // Census packing is u32-per-half; a worker count that cannot fit is
        // raised from the constructor (fatal-loud precedent), never silently
        // saturated into a corrupt census.
        let census_workers = u32::try_from(threads.len()).map_err(|_| {
            format!(
                "scheduler worker count {} overflows the census",
                threads.len()
            )
        })?;
        Ok(Self {
            shared,
            threads: Mutex::new(threads),
            inject_queues,
            worker_names,
            _vm_census: inventory::VmCensusRegistration::register(census_workers),
        })
    }
}
/// The scheduler's timer wheel, carrying W11's arming edge (§3.8.2 C2): an
/// arm earlier than the deadline a parked worker 0 published runs edge E.
fn armed_wheel(shared: &std::sync::Weak<SharedState>) -> TimerWheel {
    let mut wheel = TimerWheel::new();
    let weak = shared.clone();
    wheel.set_arm_notifier(Arc::new(move || {
        if let Some(shared) = weak.upgrade() {
            shared.wake_edge();
        }
    }));
    wheel
}

fn configured_thread_count(override_count: Option<usize>) -> usize {
    override_count
        .filter(|count| *count > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        })
}
