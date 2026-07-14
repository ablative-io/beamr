// Cooperative-runtime modules. The single-threaded `WasmScheduler` and its
// native-slice driver do not depend on the threaded scheduler, so they (and the
// shared `exit_capture` term helper they reuse) build under `cooperative`.
pub mod exit_capture;
pub mod wasm;
mod wasm_native;
pub use exit_capture::OwnedException;
pub use wasm::{
    WasmAsyncCompletion, WasmRunState, WasmRunSummary, WasmScheduledTimer, WasmScheduler,
};

/// Default preemption budget for a process slice, shared by both the threaded
/// and the cooperative scheduler.
pub const DEFAULT_REDUCTION_BUDGET: u32 = crate::process::DEFAULT_REDUCTION_BUDGET;

/// Distinguishes the two BEAM-style dirty scheduler pools.
///
/// This is pure call-classification metadata carried on every `NativeEntry`, so
/// it must exist in every build (the cooperative build has no dirty *pool*, but
/// the registry types that reference this enum are platform-neutral).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DirtySchedulerKind {
    /// CPU-bound dirty work.
    Cpu,
    /// IO-bound dirty work.
    Io,
}

/// Typed failure returned by [`Scheduler::send_to_mailbox`].
#[cfg(feature = "threads")]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MailboxSendError {
    /// No live process body or retained exit tombstone exists for the PID.
    NoSuchProcess,
    /// The PID has a retained exit tombstone and can no longer accept messages.
    ProcessTerminated,
    /// The process body exists, but its slot cannot currently admit a message.
    ProcessSlotUnavailable,
    /// The target process heap cannot reserve enough space for the message.
    HeapAllocationFailed,
    /// The owned value does not contain a valid, copyable BEAM term.
    InvalidMessage,
}

#[cfg(feature = "threads")]
impl std::fmt::Display for MailboxSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoSuchProcess => "no such process",
            Self::ProcessTerminated => "process already terminated",
            Self::ProcessSlotUnavailable => "process slot unavailable for mailbox admission",
            Self::HeapAllocationFailed => "target heap cannot admit mailbox message",
            Self::InvalidMessage => "invalid owned mailbox message",
        })
    }
}

#[cfg(feature = "threads")]
impl std::error::Error for MailboxSendError {}

/// Result returned by a successful hot module load.
///
/// Plain metadata returned by the threaded code server. It is named in the
/// platform-neutral `CodeManagementFacility` trait, so it is defined here (always
/// available) even though only the threaded scheduler can produce one.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct HotLoadResult {
    pub module_name: crate::atom::Atom,
    pub generation: u64,
    pub had_old_version: bool,
    pub on_load_required: bool,
    pub on_load_succeeded: bool,
}

/// Result returned by safe or forced module purge.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PurgeResult {
    pub module_name: crate::atom::Atom,
    pub processes_killed: usize,
}

// ---------------------------------------------------------------------------
// Threaded (work-stealing, OS-thread) scheduler. Everything below requires the
// `threads` feature: it pulls in crossbeam, the io/jit/replay/distribution
// subsystems, and `std::thread`/`Condvar`, none of which exist on wasm32.
// ---------------------------------------------------------------------------
#[cfg(feature = "threads")]
mod connection_lifecycle;
#[cfg(feature = "threads")]
pub mod dirty;
#[cfg(feature = "threads")]
mod dist_control_out;
#[cfg(feature = "threads")]
mod distribution_service;
#[cfg(feature = "threads")]
mod execution;
#[cfg(feature = "threads")]
mod exit_tombstones;
#[cfg(feature = "threads")]
mod inventory;
#[cfg(feature = "readiness")]
mod readiness;
#[cfg(feature = "threads")]
mod ring_service;
#[cfg(feature = "threads")]
mod service;
#[cfg(feature = "threads")]
mod services;
#[cfg(all(feature = "threads", any(test, feature = "test-support")))]
pub mod thread_probe;
#[cfg(feature = "threads")]
pub use execution::IDLE_PARK_TIMEOUT;
#[cfg(all(feature = "threads", any(test, feature = "test-support")))]
pub use execution::IDLE_WAKES_PER_SEC_PER_WORKER;
#[cfg(feature = "threads")]
pub use inventory::{ServiceInventoryEntry, ServicePolicyLine, deduped_thread_aggregate};
#[cfg(feature = "readiness")]
pub use readiness::{
    Generation, Interest, ReadinessBuildError, ReadinessError, ReadinessToken, SharedReadiness,
};
#[cfg(feature = "threads")]
pub use service::{
    ServiceIdentity, ServiceInstanceId, ServiceMode, ServiceModeLabel, ShutdownService,
};
#[cfg(feature = "threads")]
pub use services::{SchedulerServices, SharedIoRing, WithServicesError};
#[cfg(feature = "threads")]
mod module_management;
#[cfg(feature = "threads")]
mod pg_propagation;
#[cfg(feature = "threads")]
mod process_slot;
#[cfg(all(test, feature = "readiness"))]
mod readiness_tests;
#[cfg(feature = "threads")]
mod remote_supervision;
#[cfg(feature = "threads")]
pub mod run_queue;
#[cfg(feature = "threads")]
mod spawning;
#[cfg(feature = "threads")]
pub mod steal;
#[cfg(feature = "threads")]
mod supervision_integration;
#[cfg(feature = "threads")]
mod suspension;
#[cfg(all(test, feature = "readiness"))]
mod teardown_admission_tests;
#[cfg(feature = "threads")]
mod test_helpers;
#[cfg(feature = "threads")]
mod timer_integration;
#[cfg(feature = "threads")]
use self::dirty::DirtyPool;
#[cfg(feature = "threads")]
use self::execution::scheduler_loop;
#[cfg(feature = "threads")]
use self::spawning::SpawnRequest;
#[cfg(feature = "threads")]
use crate::atom::AtomTable;
#[cfg(feature = "threads")]
use crate::distribution::DistributionConfig;
#[cfg(feature = "threads")]
use crate::distribution::connection::ConnectionManager;
#[cfg(feature = "threads")]
use crate::distribution::pg::PgRegistry;
#[cfg(feature = "threads")]
use crate::distribution::remote_link::{DistributionControlFacility, RemoteLinkError};
#[cfg(feature = "threads")]
use crate::distribution::{DEFAULT_NODE_NAME, Node};

#[cfg(feature = "threads")]
use crate::error::ExecError;
#[cfg(feature = "threads")]
use crate::ets::copy::OwnedTerm;
#[cfg(feature = "threads")]
use crate::ets::{EtsRegistry, EtsTable, EtsTableId, EtsTableMetadata};
#[cfg(feature = "threads")]
use crate::hook::Hook;
#[cfg(feature = "threads")]
use crate::io::{
    CompletionRing, CompletionRingIoFacility, FILE_IO_RING_THREAD_PREFIX,
    GENERIC_IO_RING_THREAD_PREFIX, IoCompletion, IoCompletionBridge, IoFacility, IoOp, IoSink,
    IoWakeTarget, NullSink, PendingIoRegistry, RingConfig, STANDARD_IO_RING_THREAD_PREFIX,
    StandardIoServer, create_ring_with_prefix,
};
#[cfg(feature = "threads")]
use crate::jit::{DEFAULT_JIT_THRESHOLD, JitCache, JitProfiler};
#[cfg(feature = "threads")]
use crate::module::ModuleRegistry;
#[cfg(feature = "threads")]
use crate::namespace::NamespaceId;
#[cfg(feature = "threads")]
use crate::native::{
    AllCapabilitiesPolicy, BifRegistryImpl, CapabilityPolicy, FileIoCompletion, FileIoContinuation,
    ProcessInfoItem, ProcessInfoStatus, ProcessInfoValue, ProcessMonitorInfo,
};
#[cfg(feature = "threads")]
use crate::process::registry::ProcessTable;
#[cfg(feature = "threads")]
use crate::process::{ExitReason, Process, ProcessStatus, RemotePid};
#[cfg(feature = "threads")]
use crate::replay::{ReplayDriver, ReplayLog};
#[cfg(feature = "threads")]
use crate::supervision::link::LinkSet;
#[cfg(feature = "threads")]
use crate::supervision::monitor::MonitorSet;
#[cfg(feature = "threads")]
use crate::term::Term;
#[cfg(feature = "threads")]
use crate::timer::TimerWheel;
#[cfg(feature = "threads")]
use crossbeam_queue::SegQueue;
#[cfg(feature = "threads")]
use dashmap::{DashMap, DashSet};
#[cfg(feature = "threads")]
use distribution_service::DistributionService;
#[cfg(feature = "threads")]
use process_slot::{PendingMailboxMessage, ProcessMetadata, ProcessSlot};
#[cfg(feature = "readiness")]
use readiness::{ReadinessConsumer, ReadinessService, RouteHome, ServiceConsumerId};
#[cfg(feature = "threads")]
use ring_service::{RingService, StandardIoService};
#[cfg(feature = "threads")]
use run_queue::{PriorityStealers, RunQueue};
#[cfg(feature = "threads")]
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[cfg(feature = "threads")]
use std::sync::{Arc, Condvar, Mutex};
#[cfg(feature = "threads")]
use std::thread::JoinHandle;
#[cfg(feature = "threads")]
use std::time::Duration;
#[cfg(all(feature = "threads", feature = "telemetry"))]
use std::time::Instant;

#[cfg(feature = "threads")]
enum ReplayMode {
    Live,
    Replay(ReplayLog),
}

/// Sentinel [`SharedState::standard_io_pid`] when the standard-IO ring is
/// `Disabled` (spec §3.4): no process 0 is registered, so no live pid matches.
/// [`Term::PID_MAX`] rather than `u64::MAX` because the sentinel is also used
/// as the dead GROUP LEADER of top-level processes — it must be representable
/// as an immediate pid term (`Term::pid` debug-asserts the payload range).
/// Pid allocation starts at 1 and increments; reaching this value would take
/// ~2^61 spawns, the same practical-impossibility class as `u64::MAX`.
#[cfg(feature = "threads")]
const NO_STANDARD_IO_PID: u64 = crate::term::Term::PID_MAX;

#[cfg(feature = "threads")]
#[derive(Clone, Default)]
pub struct SchedulerConfig {
    pub thread_count: Option<usize>,
    /// Dirty CPU pool sizing (spec §3.2).
    ///
    /// - `None` — legacy default: one worker per core (`num_cpus`).
    /// - `Some(n)` with `n > 0` — an owned pool of `n` workers.
    /// - `Some(0)` — the pool is **Disabled**: zero threads, zero fds, and a
    ///   dirty CPU call is refused with a typed service-unavailable error
    ///   before any suspension or queue side effect.
    ///
    /// **Behavior change (spec §6):** `Some(0)` previously coerced to a
    /// one-worker pool; it now disables the pool. Requesting zero workers and
    /// still dispatching dirty work is a composition error, surfaced loudly at
    /// the calling process rather than silently papered over.
    pub dirty_cpu_threads: Option<usize>,
    /// Dirty IO pool sizing. Same semantics as [`dirty_cpu_threads`](Self::dirty_cpu_threads):
    /// `None` is the legacy default (10 workers), `Some(n > 0)` owns `n`,
    /// `Some(0)` disables the pool (was coerced to 1 before — spec §6).
    pub dirty_io_threads: Option<usize>,
    pub dirty_queue_depth: Option<usize>,
    pub io: Option<RingConfig>,
    pub node_name: Option<String>,
    pub creation: Option<u32>,
    pub distribution: Option<DistributionConfig>,
    pub jit_threshold: Option<u32>,
    /// Minimum interval between per-process telemetry samples at scheduler slice boundaries.
    pub telemetry_sample_interval: Option<Duration>,
    /// Embedder-supplied private data handed to every native call via
    /// [`crate::native::ProcessContext::nif_private_data`] — the ERTS
    /// `enif_priv_data` equivalent, scoped to this scheduler instance so
    /// embedders hosting several runtimes in one OS process never need
    /// process-wide globals.
    pub nif_private_data: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

#[cfg(feature = "threads")]
impl std::fmt::Debug for SchedulerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchedulerConfig")
            .field("thread_count", &self.thread_count)
            .field("dirty_cpu_threads", &self.dirty_cpu_threads)
            .field("dirty_io_threads", &self.dirty_io_threads)
            .field("dirty_queue_depth", &self.dirty_queue_depth)
            .field("io", &self.io)
            .field("node_name", &self.node_name)
            .field("creation", &self.creation)
            .field("distribution", &self.distribution)
            .field("jit_threshold", &self.jit_threshold)
            .field("telemetry_sample_interval", &self.telemetry_sample_interval)
            .field(
                "nif_private_data",
                &self.nif_private_data.as_ref().map(|_| ".."),
            )
            .finish()
    }
}
#[cfg(feature = "threads")]
pub(super) struct SharedState {
    shutdown: AtomicBool,
    process_table: ProcessTable,
    module_registry: Arc<ModuleRegistry>,
    namespace_store: DashMap<NamespaceId, Arc<ModuleRegistry>>,
    next_namespace_id: AtomicU64,
    atom_table: Arc<AtomTable>,
    local_node: Node,
    ets_registry: Arc<EtsRegistry>,
    pg_registry: Arc<PgRegistry>,
    bif_registry: Arc<BifRegistryImpl>,
    capability_policy: Arc<dyn CapabilityPolicy>,
    spawn_counter: AtomicUsize,
    thread_count: usize,
    pub(super) dirty_cpu: ServiceMode<DirtyPool>,
    pub(super) dirty_io: ServiceMode<DirtyPool>,
    jit_profiler: Arc<JitProfiler>,
    jit_cache: Arc<JitCache>,
    next_pid: AtomicU64,
    wait_set: Mutex<WaitSet>,
    wake_condvar: Condvar,
    process_bodies: DashMap<u64, Mutex<ProcessSlot>>,
    exit_tombstones: exit_tombstones::BoundedTombstones,
    exit_results: DashMap<u64, OwnedTerm>,
    exit_errors: DashMap<u64, ExecError>,
    exit_exceptions: DashMap<u64, OwnedException>,
    /// pid → current result-gated suspension identity (call id + kind).
    /// Owning-thread written; read by completion publishers and the wake
    /// gate. See `suspension.rs`.
    suspensions: DashMap<u64, suspension::SuspensionMirror>,
    /// pid → completion published for a specific suspension call id. The
    /// owning thread applies it at slice start only when the id matches the
    /// process's current suspension record.
    suspension_results: DashMap<u64, suspension::PendingSuspensionResult>,
    /// pid → sticky embedder resume for a hook suspension (call id, or
    /// `RESUME_ANY_HOOK` when the resume raced the suspension's creation).
    pending_resumes: DashMap<u64, u64>,
    /// File-IO completion ring (spec §3.3). `Owned` in live mode; `Disabled`
    /// under replay, where the file facility is absent and file submission is
    /// refused before any suspension. `Shared` is a type capability; the
    /// injection API is commit 5.
    file_io_ring: ServiceMode<RingService>,
    file_io_pending: DashMap<u64, (u64, FileIoContinuation)>,
    file_io_orphans: DashMap<u64, IoCompletion>,
    file_io_results: DashMap<u64, FileIoCompletion>,
    file_io_canceled: DashSet<u64>,
    link_set: Mutex<LinkSet>,
    monitor_set: Mutex<MonitorSet>,
    hook: Hook,
    /// The distribution service bundle (spec §3.6): node config + ONE
    /// heartbeat-enabled `ConnectionManager` + outbound sender + net-kernel
    /// facade, held in a `ServiceMode`. `Owned` when `config.distribution` was
    /// `Some` (both runtimes live), `Disabled` when it was `None` (NEITHER
    /// runtime exists — honest absence). `Shared` is out of scope v1 (recorded,
    /// not silently absent). Read through [`SharedState::distribution`].
    distribution: ServiceMode<DistributionService>,
    process_registry: DashMap<crate::atom::Atom, u64>,
    timers: Arc<Mutex<TimerWheel>>,
    /// Receive timers that fired but could not be applied in place: pid →
    /// fired timer ids. `expire_timers` only marks and wakes; the woken
    /// process applies the timeout jump itself at the start of its next
    /// slice (and drops stale ids whose receive completed first). This keeps
    /// the timeout-label jump on the owning thread, so it can never race a
    /// slot that is `Executing` or a park gap.
    expired_receive_timers: DashMap<u64, Vec<u64>>,
    output_sink: Mutex<Arc<dyn IoSink>>,
    /// Optional generic-IO ring (spec §3.5). `Disabled` (byte-identical to the
    /// former `None`) when `config.io` is absent; `Owned` when requested.
    /// `Shared` is a type capability whose injection API is commit 5.
    io_ring: ServiceMode<RingService>,
    io_registry: Option<Arc<PendingIoRegistry>>,
    io_bridge: Mutex<Option<IoCompletionBridge>>,
    io_facility: Option<Arc<dyn IoFacility>>,
    /// Group-leader / standard-IO server pid, or [`NO_STANDARD_IO_PID`] when the
    /// standard ring is `Disabled` (no process 0 registered, spec §3.4).
    standard_io_pid: u64,
    /// Readiness poller ownership and this scheduler's route-home handle.
    #[cfg(feature = "readiness")]
    readiness: ServiceMode<ReadinessService>,
    #[cfg(feature = "readiness")]
    readiness_consumer: Option<ReadinessConsumer>,
    /// Process-unique identities minted for each ancillary service at
    /// construction, so `service_inventory()` reports a stable identity (§5).
    service_instances: inventory::ServiceInstances,
    /// Count of transient `dirty-complete-{pid}` completion threads spawned to
    /// date. Reported as a policy line, not a thread line (§5); incremented
    /// once per dirty call, so it is negligible on the dirty submit path.
    dirty_completion_spawns: AtomicU64,
    /// Teardown admission plus dirty-bridge join state (spec §4 step 3), behind
    /// ONE mutex so intake closure and every reservation are LINEARIZED. Dirty
    /// calls reserve before suspension and publish a retained `JoinHandle`;
    /// mutating facilities hold a reservation across their mutation. Shutdown
    /// closes intake, waits out every reservation, and OS-JOINS each bridge —
    /// neither a bridge nor a facility mutation can land behind the drain.
    dirty_completions: Mutex<TeardownAdmissionRegistry>,
    dirty_completions_changed: Condvar,
    /// Dropping this sender closes the channel every completion bridge
    /// selects on ([`dirty::oneshot::Receiver::recv_or_shutdown`]), waking
    /// them all to exit WITHOUT delivering. `None` once shutdown has drained.
    dirty_completion_shutdown_tx: Mutex<Option<crossbeam_channel::Sender<()>>>,
    /// The receiver half each completion bridge clones and selects on.
    dirty_completion_shutdown_rx: crossbeam_channel::Receiver<()>,
    replay_driver: Option<Arc<Mutex<ReplayDriver>>>,
    replay_mode: bool,
    pub(super) nif_private_data: Option<Arc<dyn std::any::Any + Send + Sync>>,
    #[cfg(feature = "telemetry")]
    telemetry_metrics: TelemetryMetricState,

    /// Standard-IO ring + group-leader server (spec §3.4). `Owned` in live
    /// mode, where process 0 is registered; `Disabled` under replay (no ring,
    /// no process 0). Its owner joins the ring at shutdown (`shutdown_owned`),
    /// so the ring is not leaked until the last `Arc` drop.
    standard_io: ServiceMode<StandardIoService>,

    #[cfg(any(test, feature = "test-support"))]
    idle_parks: AtomicUsize,

    /// Millisecond value of the timeout the park primitive most recently
    /// used, written by every `park_thread` entry. The deterministic linkage
    /// between the running code and the signed 5ms floor: the bound test
    /// asserts this equals `IDLE_PARK_TIMEOUT` rather than inferring it from
    /// a load-sensitive wake rate. 0 = no worker has parked yet.
    #[cfg(any(test, feature = "test-support"))]
    observed_park_timeout_millis: AtomicU64,

    /// Count of suspension mirrors registered to date, one per
    /// `register_suspension_mirror` call. Boundary instrument for the §3.2
    /// refusal-first ordering: a refused dirty call must leave this counter
    /// unmoved, so a refusal that happens AFTER registration (and then
    /// cleans up the mirror) fails the ordering gates instead of passing on
    /// the cleaned-up end state.
    #[cfg(any(test, feature = "test-support"))]
    suspension_mirror_registrations: AtomicU64,

    /// Count of entries into the dirty-call gated-suspension side-effect
    /// sequence, whose FIRST step is suspension-call-id allocation —
    /// incremented immediately before that allocation in the DirtyCall arm.
    /// The mirror counter above pins the sequence's LAST side effect; this
    /// one pins its first, so a refusal that regresses to anywhere inside
    /// the sequence (after allocation or `set_suspension`, before mirror
    /// registration) still moves an instrument and fails the §3.2 gates.
    #[cfg(any(test, feature = "test-support"))]
    dirty_suspension_allocations: AtomicU64,

    #[cfg(test)]
    park_gap_hook: Mutex<Option<ParkGapHook>>,
}

#[cfg(feature = "threads")]
#[cfg(feature = "telemetry")]
pub(super) struct TelemetryMetricState {
    sample_interval: Duration,
    last_process_samples: Mutex<std::collections::HashMap<u64, Instant>>,
    scheduler_executing_nanos: AtomicU64,
    scheduler_idle_nanos: AtomicU64,
}

#[cfg(feature = "threads")]
#[cfg(feature = "telemetry")]
impl TelemetryMetricState {
    fn new(sample_interval: Duration) -> Self {
        Self {
            sample_interval,
            last_process_samples: Mutex::new(std::collections::HashMap::new()),
            scheduler_executing_nanos: AtomicU64::new(0),
            scheduler_idle_nanos: AtomicU64::new(0),
        }
    }
}

#[cfg(feature = "threads")]
#[derive(Default)]
struct TeardownAdmissionRegistry {
    /// Set by the shutdown drain; reservations are refused from here on.
    closed: bool,
    /// Admissions that have not yet published a bridge handle or released.
    reserved: usize,
    /// Retained bridge thread handles, OS-joined by the drain; finished
    /// handles are reaped opportunistically at each publish.
    bridges: Vec<std::thread::JoinHandle<()>>,
}

/// RAII teardown-admission token (spec §4 step 3), LINEARIZED with the
/// shutdown drain under one lock. Two uses:
///
/// - A dirty submission acquires one BEFORE any suspension side effect and
///   CONVERTS it (via [`publish`](Self::publish)) into a retained, joinable
///   bridge handle at spawn; every pre-spawn error path releases it via
///   `Drop`.
/// - A mutating facility operation (spawn family, io-message delivery, link)
///   acquires one and HOLDS it across its whole mutation, releasing via
///   `Drop` — so a mutation admitted before the drain closes intake finishes
///   BEFORE shutdown returns (the drain waits), and one attempted after
///   refuses. A snapshot predicate cannot give this guarantee: it admits a
///   delayed mutation that lands after shutdown has returned.
#[cfg(feature = "threads")]
pub(in crate::scheduler) struct TeardownAdmission {
    shared: Arc<SharedState>,
    published: bool,
}

#[cfg(feature = "threads")]
impl TeardownAdmission {
    /// Convert this reservation into a retained bridge handle.
    pub(in crate::scheduler) fn publish(mut self, handle: std::thread::JoinHandle<()>) {
        let mut registry = lock_or_recover(&self.shared.dirty_completions);
        registry.reserved = registry.reserved.saturating_sub(1);
        // Opportunistic reap: completed bridges cost one `is_finished` check
        // here, so the retained vec tracks in-flight bridges, not history.
        let retained = std::mem::take(&mut registry.bridges);
        for bridge in retained {
            if bridge.is_finished() {
                let _ = bridge.join();
            } else {
                registry.bridges.push(bridge);
            }
        }
        registry.bridges.push(handle);
        drop(registry);
        self.shared.dirty_completions_changed.notify_all();
        self.published = true;
    }
}

#[cfg(feature = "threads")]
impl Drop for TeardownAdmission {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let mut registry = lock_or_recover(&self.shared.dirty_completions);
        registry.reserved = registry.reserved.saturating_sub(1);
        drop(registry);
        self.shared.dirty_completions_changed.notify_all();
    }
}

#[cfg(feature = "threads")]
impl SharedState {
    /// Reserve admission for one dirty submission, refused once intake is
    /// closed. Closure and reservation share ONE lock, so a reservation
    /// cannot slip behind the shutdown drain: the drain marks `closed` and
    /// waits out every earlier reservation under the same mutex.
    pub(in crate::scheduler) fn try_reserve_teardown_admission(
        self: &Arc<Self>,
    ) -> Option<TeardownAdmission> {
        let mut registry = lock_or_recover(&self.dirty_completions);
        if registry.closed {
            return None;
        }
        registry.reserved += 1;
        drop(registry);
        Some(TeardownAdmission {
            shared: Arc::clone(self),
            published: false,
        })
    }

    /// The shutdown channel each completion bridge selects on.
    pub(in crate::scheduler) fn dirty_completion_shutdown_channel(
        &self,
    ) -> crossbeam_channel::Receiver<()> {
        self.dirty_completion_shutdown_rx.clone()
    }

    /// §4 step 3 for the dirty pools: close intake (linearized with
    /// reservation — no new bridge can spawn behind this), wake every bridge
    /// by closing the shutdown channel, wait out in-flight reservations
    /// (each either releases on an error path or publishes its bridge
    /// handle), then OS-JOIN every bridge. Bounded: woken bridges exit
    /// without waiting for their jobs. After this returns, no bridge THREAD
    /// of this scheduler exists at the OS level, so nothing can deliver a
    /// completion into the scheduler being torn down. Idempotent.
    pub(in crate::scheduler) fn drain_dirty_completions(&self) {
        let mut registry = lock_or_recover(&self.dirty_completions);
        registry.closed = true;
        drop(registry);
        drop(lock_or_recover(&self.dirty_completion_shutdown_tx).take());
        let mut registry = lock_or_recover(&self.dirty_completions);
        while registry.reserved > 0 {
            registry = self
                .dirty_completions_changed
                .wait(registry)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let bridges = std::mem::take(&mut registry.bridges);
        drop(registry);
        for bridge in bridges {
            // A joined bridge has fully exited at the OS level; a panic
            // payload has nothing to recover during teardown.
            let _ = bridge.join();
        }
    }

    /// Unjoined-bridge count: in-flight reservations plus retained handles
    /// (finished handles are reaped opportunistically at each publish).
    /// 0 after a drained shutdown. Reached via
    /// [`Scheduler::dirty_completions_live_count`].
    #[cfg(any(test, feature = "test-support"))]
    fn dirty_completions_live_count(&self) -> usize {
        let registry = lock_or_recover(&self.dirty_completions);
        registry.reserved + registry.bridges.len()
    }

    /// The owned distribution bundle, or `None` when distribution is `Disabled`
    /// (spec §3.6). Every distribution read path — pg propagation, outbound
    /// controls, remote-send, the net-kernel facade — routes through this so a
    /// `Disabled` bundle refuses honestly rather than touching an absent runtime.
    pub(in crate::scheduler) fn distribution(&self) -> Option<&DistributionService> {
        self.distribution.service()
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn readiness_register(
        self: &Arc<Self>,
        fd: std::os::fd::RawFd,
        interest: Interest,
        pid: u64,
        marker: crate::atom::Atom,
    ) -> Result<ReadinessToken, ReadinessError> {
        self.readiness_consumer
            .as_ref()
            .ok_or(ReadinessError::Disabled)?
            .register(fd, interest, pid, marker)
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn readiness_rearm(
        self: &Arc<Self>,
        token: &ReadinessToken,
        interest: Interest,
    ) -> Result<(), ReadinessError> {
        self.readiness_consumer
            .as_ref()
            .ok_or(ReadinessError::Disabled)?
            .rearm(token, interest)
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn readiness_deregister(&self, token: ReadinessToken) {
        if let Some(consumer) = &self.readiness_consumer {
            consumer.deregister(token);
        }
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn purge_readiness_state(&self, pid: u64) {
        if let Some(consumer) = &self.readiness_consumer {
            consumer.deregister_pid(pid);
        }
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn deregister_shared_readiness(&self) {
        if matches!(self.readiness, ServiceMode::Shared(_))
            && let Some(consumer) = &self.readiness_consumer
        {
            consumer.deregister_all();
        }
    }

    #[cfg(feature = "readiness")]
    pub(in crate::scheduler) fn deliver_readiness_marker(
        &self,
        pid: u64,
        marker: crate::atom::Atom,
    ) -> bool {
        let delivered = timer_integration::deliver_term_to_mailbox(self, pid, Term::atom(marker));
        if delivered.is_ok() {
            execution::wake_process(self, pid);
        }
        delivered.is_ok()
    }

    /// Test-only: the owned distribution manager, panicking if `Disabled`. The
    /// in-crate tests that drive the connection table directly always construct
    /// an owned bundle.
    #[cfg(test)]
    pub(in crate::scheduler) fn distribution_connections_or_panic(&self) -> &ConnectionManager {
        self.distribution()
            .expect("distribution owned in test")
            .connections()
    }

    /// Test-only: the owned outbound sender, panicking if absent (the wire tests
    /// construct it via `make_shared_state_with_dist_sender`).
    #[cfg(test)]
    pub(in crate::scheduler) fn dist_sender_or_panic(
        &self,
    ) -> &crate::distribution::sender::DistSender {
        self.distribution()
            .and_then(|dist| dist.sender())
            .expect("dist sender present in test")
    }

    /// Insert an exit tombstone for `pid`, evicting the oldest tombstone (and
    /// its paired satellite entries) if the bounded store is over capacity.
    ///
    /// This is the single write path for tombstones. Eviction removes the
    /// evicted pid's `exit_results` / `exit_errors` / `exit_exceptions` along
    /// with its tombstone, so a satellite can never outlive the tombstone it
    /// pairs with and the "tombstone observed ⇒ paired result already present"
    /// invariant the readers rely on is preserved.
    pub(super) fn insert_exit_tombstone(&self, pid: u64, reason: ExitReason) {
        if let Some(evicted) = self.exit_tombstones.insert(pid, reason) {
            self.exit_results.remove(&evicted);
            self.exit_errors.remove(&evicted);
            self.exit_exceptions.remove(&evicted);
        }
    }

    /// Submit an operation to the file-IO ring, or `None` when it is `Disabled`
    /// (spec §3.3). The completion re-arm paths use this so a disabled ring
    /// stops re-arming instead of dispatching into a ring that was never built.
    pub(super) fn submit_file_ring_op(&self, op: IoOp) -> Option<u64> {
        self.file_io_ring
            .service()
            .map(|ring| ring.ring().submit(op))
    }

    pub(super) fn create_table(&self, metadata: EtsTableMetadata) -> EtsTableId {
        self.ets_registry.create_table(metadata)
    }

    pub(super) fn lookup_table(&self, id: EtsTableId) -> Option<Arc<dyn EtsTable>> {
        self.ets_registry.lookup_table(id)
    }

    pub(super) fn lookup_table_by_name(&self, name: crate::atom::Atom) -> Option<EtsTableId> {
        self.ets_registry.lookup_table_by_name(name)
    }

    pub(super) fn delete_table(&self, id: EtsTableId) -> bool {
        self.ets_registry.delete_table(id)
    }

    pub(super) fn transfer_or_delete_tables_owned_by(&self, owner: u64) -> usize {
        let before = self.ets_registry.table_count();
        let owned_ids = self.ets_registry.table_ids_owned_by(owner);
        for table_id in owned_ids {
            let Some(table) = self.ets_registry.lookup_table(table_id) else {
                continue;
            };
            let Some(heir) = &table.metadata().heir else {
                let _deleted = self.ets_registry.delete_table(table_id);
                continue;
            };
            if self.process_table.get(heir.pid).is_some()
                && supervision_integration::deliver_ets_transfer(
                    self,
                    heir.pid,
                    table_id,
                    owner,
                    heir.data.root(),
                    &self.atom_table,
                )
                && self.ets_registry.transfer_table_owner(table_id, heir.pid)
            {
                continue;
            }
            let _deleted = self.ets_registry.delete_table(table_id);
        }
        before.saturating_sub(self.ets_registry.table_count())
    }

    /// Return the number of alive processes tracked by the scheduler.
    #[must_use]
    pub(super) fn process_count(&self) -> usize {
        self.process_table.len()
    }

    /// Return the configured number of normal scheduler threads.
    #[must_use]
    pub(super) fn scheduler_count(&self) -> usize {
        self.thread_count
    }

    /// Return the current number of interned atoms.
    #[must_use]
    pub(super) fn atom_count(&self) -> usize {
        self.atom_table.len()
    }

    /// Return an approximate memory summary for OTP compatibility probes.
    #[must_use]
    pub(super) fn memory_summary(&self) -> crate::native::system_info_bifs::MemorySummary {
        let (process_heap_words, binary) = self.process_heap_and_binary_words();

        let processes =
            process_heap_words.saturating_mul(crate::native::system_info_bifs::WORDSIZE_BYTES);
        let atom = self
            .atom_count()
            .saturating_mul(crate::native::system_info_bifs::WORDSIZE_BYTES);
        crate::native::system_info_bifs::MemorySummary::from_components(processes, atom, binary)
    }

    /// Return approximate process heap and virtual binary memory words.
    #[must_use]
    pub(super) fn process_heap_and_binary_words(&self) -> (usize, usize) {
        let mut process_heap_words = 0usize;
        let mut binary = 0usize;

        for entry in &self.process_bodies {
            match &*lock_or_recover(&entry) {
                ProcessSlot::Present(scheduled) => {
                    if matches!(scheduled.0.status(), ProcessStatus::Exited(_)) {
                        continue;
                    }
                    process_heap_words =
                        process_heap_words.saturating_add(scheduled.0.heap().total_used());
                    binary = binary.saturating_add(scheduled.0.virtual_binary_heap());
                }
                ProcessSlot::Executing(metadata) => {
                    process_heap_words = process_heap_words.saturating_add(metadata.heap_size);
                    binary = binary.saturating_add(metadata.binary_heap_size);
                }
                ProcessSlot::Absent => {}
            }
        }

        (process_heap_words, binary)
    }

    #[cfg(feature = "telemetry")]
    pub(super) fn record_scheduler_executing(&self, duration: Duration) {
        self.add_scheduler_duration(&self.telemetry_metrics.scheduler_executing_nanos, duration);
        self.record_vm_health_metrics();
    }

    #[cfg(feature = "telemetry")]
    pub(super) fn record_scheduler_idle(&self, duration: Duration) {
        self.add_scheduler_duration(&self.telemetry_metrics.scheduler_idle_nanos, duration);
        self.record_vm_health_metrics();
    }

    #[cfg(feature = "telemetry")]
    pub(super) fn record_process_slice_metrics(&self, process: &Process, reductions_consumed: u32) {
        let now = Instant::now();
        {
            let mut last_samples = lock_or_recover(&self.telemetry_metrics.last_process_samples);
            if let Some(last_sample) = last_samples.get(&process.pid())
                && now.duration_since(*last_sample) < self.telemetry_metrics.sample_interval
            {
                return;
            }
            last_samples.insert(process.pid(), now);
        }
        crate::telemetry::metrics::record_process_slice(
            process.pid(),
            reductions_consumed,
            process.mailbox().message_count(),
        );
    }

    #[cfg(feature = "telemetry")]
    pub(super) fn remove_process_metric_state(&self, pid: u64) {
        lock_or_recover(&self.telemetry_metrics.last_process_samples).remove(&pid);
    }

    #[cfg(feature = "telemetry")]
    fn record_vm_health_metrics(&self) {
        let (heap_words, _) = self.process_heap_and_binary_words();
        crate::telemetry::metrics::record_vm_health(
            self.process_count(),
            heap_words,
            self.scheduler_utilization(),
        );
    }

    #[cfg(feature = "telemetry")]
    fn scheduler_utilization(&self) -> f64 {
        let executing = self
            .telemetry_metrics
            .scheduler_executing_nanos
            .load(Ordering::Relaxed);
        let idle = self
            .telemetry_metrics
            .scheduler_idle_nanos
            .load(Ordering::Relaxed);
        let total = executing.saturating_add(idle);
        if total == 0 {
            0.0
        } else {
            executing as f64 / total as f64
        }
    }

    #[cfg(feature = "telemetry")]
    fn add_scheduler_duration(&self, counter: &AtomicU64, duration: Duration) {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        let _previous = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_add(nanos))
        });
    }
}

#[cfg(feature = "threads")]
#[derive(Default)]
struct WaitSet {
    waiting: std::collections::HashMap<u64, usize>,
    woken: Vec<(u64, usize)>,
}

/// Test-only injection points inside the park sequences of `run_process`,
/// used to drive deliver/resume interleavings deterministically.
#[cfg(feature = "threads")]
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ParkGap {
    /// Wait arm: after the store-back, before wait-set registration.
    WaitStored,
    /// Wait arm: after wait-set registration, before the mailbox recheck.
    WaitRegistered,
    /// Suspended arm: after the store-back, before wait-set registration.
    SuspendStored,
}

#[cfg(feature = "threads")]
#[cfg(test)]
type ParkGapHook = Box<dyn Fn(&SharedState, ParkGap, u64) + Send + Sync>;
#[cfg(feature = "threads")]
pub(super) struct ScheduledProcess(Process);
// SAFETY: Process is not Send at the public API boundary. The scheduler is the
// sole owner of process execution, storing each body behind a mutex-protected
// ProcessSlot. Workers take exclusive ownership before executing a time slice.
#[cfg(feature = "threads")]
unsafe impl Send for ScheduledProcess {}
#[cfg(feature = "threads")]
pub struct Scheduler {
    shared: Arc<SharedState>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    inject_queues: Vec<Arc<SegQueue<SpawnRequest>>>,
    worker_names: Vec<String>,
}
#[cfg(feature = "threads")]
impl Scheduler {
    /// Allocate and register an ETS table owned by a process.
    ///
    /// The provided metadata's `id` field is overwritten with the allocated,
    /// monotonically increasing table ID before the table is inserted.
    pub fn create_ets_table(&self, metadata: EtsTableMetadata) -> EtsTableId {
        self.shared.create_table(metadata)
    }

    /// Look up a registered ETS table by ID.
    pub fn lookup_ets_table(&self, id: EtsTableId) -> Option<Arc<dyn EtsTable>> {
        self.shared.lookup_table(id)
    }

    /// Look up a named ETS table by atom.
    pub fn lookup_ets_table_by_name(&self, name: crate::atom::Atom) -> Option<EtsTableId> {
        self.shared.lookup_table_by_name(name)
    }

    /// Delete a registered ETS table by ID.
    pub fn delete_ets_table(&self, id: EtsTableId) -> bool {
        self.shared.delete_table(id)
    }

    /// Construct a scheduler on the **legacy profile** (spec §2.2/§6).
    ///
    /// Maps to [`SchedulerServices::from_config`]: every ancillary service is
    /// resolved from the matching [`SchedulerConfig`] knob, preserving today's
    /// per-knob defaults for one release as a migration bridge. Those defaults
    /// are EAGER — a dirty CPU pool sized to `num_cpus`, a dirty IO pool of
    /// [`DEFAULT_DIRTY_IO_THREADS`](dirty::DEFAULT_DIRTY_IO_THREADS), a live
    /// file-IO ring and a live standard-IO ring with process 0 — so a scheduler
    /// built this way pays for roughly a dozen ancillary threads whether or not
    /// it uses them (spec §1). Distribution, however, follows `config` honestly:
    /// `config.distribution: None` (the default) builds NEITHER distribution
    /// runtime (spec §3.6, commit-4 change). Embedders that want a specific
    /// service footprint should use [`Scheduler::with_services`] with
    /// [`SchedulerServices::minimal`] / [`SchedulerServices::full_runtime`]
    /// instead; this constructor is retained for source compatibility and will
    /// keep the legacy defaults for one release (see CHANGELOG migration note).
    pub fn new(
        config: SchedulerConfig,
        module_registry: Arc<ModuleRegistry>,
    ) -> Result<Self, String> {
        Self::with_services_and_code_server(
            config,
            SchedulerServices::from_config(),
            module_registry,
            Arc::new(AtomTable::with_common_atoms()),
            Arc::new(BifRegistryImpl::new()),
        )
    }

    /// The additive composition entrypoint (spec §2.2): build a scheduler whose
    /// ancillary services are exactly what `services` asks for, with `config`
    /// supplying the non-service knobs (thread count, node identity, queue
    /// depth, telemetry, private data). An explicit service choice WINS over the
    /// matching legacy `config` knob; a `FromConfig` choice defers to it (see
    /// [`SchedulerServices`] for the precedence rule and the profiles).
    ///
    /// Returns `Err` naming the offending service when the composition requests
    /// a capability this release cannot deliver safely — currently a shared
    /// file/generic IO ring, whose cross-scheduler routing lands with the §3.9
    /// gate in commit 6 ([`WithServicesError`]). Validate ahead of construction
    /// with [`SchedulerServices::validate`].
    pub fn with_services(
        config: SchedulerConfig,
        services: SchedulerServices,
        module_registry: Arc<ModuleRegistry>,
    ) -> Result<Self, String> {
        Self::with_services_and_code_server(
            config,
            services,
            module_registry,
            Arc::new(AtomTable::with_common_atoms()),
            Arc::new(BifRegistryImpl::new()),
        )
    }

    /// [`Scheduler::with_services`] sharing an explicit atom table and BIF
    /// registry (the load-time state the modules were compiled against), the
    /// composition analogue of [`Scheduler::with_code_server`].
    pub fn with_services_and_code_server(
        config: SchedulerConfig,
        services: SchedulerServices,
        module_registry: Arc<ModuleRegistry>,
        atom_table: Arc<AtomTable>,
        bif_registry: Arc<BifRegistryImpl>,
    ) -> Result<Self, String> {
        services.validate().map_err(|error| error.to_string())?;
        Self::construct_with_services(
            config,
            services,
            module_registry,
            atom_table,
            bif_registry,
            Arc::new(AllCapabilitiesPolicy),
            ReplayMode::Live,
        )
    }

    /// Create a scheduler in deterministic replay mode over `log`.
    pub fn new_replay(config: SchedulerConfig, log: ReplayLog) -> Result<Self, String> {
        Self::new_replay_with_registry(config, Arc::new(ModuleRegistry::new()), log)
    }

    /// Create a replay scheduler using an explicit module registry.
    pub fn new_replay_with_registry(
        config: SchedulerConfig,
        module_registry: Arc<ModuleRegistry>,
        log: ReplayLog,
    ) -> Result<Self, String> {
        Self::construct(config, module_registry, ReplayMode::Replay(log))
    }

    pub fn with_code_server(
        config: SchedulerConfig,
        module_registry: Arc<ModuleRegistry>,
        atom_table: Arc<AtomTable>,
        bif_registry: Arc<BifRegistryImpl>,
    ) -> Result<Self, String> {
        Self::with_code_server_and_policy(
            config,
            module_registry,
            atom_table,
            bif_registry,
            Arc::new(AllCapabilitiesPolicy),
        )
    }
    pub fn with_code_server_and_policy(
        config: SchedulerConfig,
        module_registry: Arc<ModuleRegistry>,
        atom_table: Arc<AtomTable>,
        bif_registry: Arc<BifRegistryImpl>,
        capability_policy: Arc<dyn CapabilityPolicy>,
    ) -> Result<Self, String> {
        Self::construct_with_services(
            config,
            SchedulerServices::from_config(),
            module_registry,
            atom_table,
            bif_registry,
            capability_policy,
            ReplayMode::Live,
        )
    }

    fn construct(
        config: SchedulerConfig,
        module_registry: Arc<ModuleRegistry>,
        replay_mode: ReplayMode,
    ) -> Result<Self, String> {
        Self::construct_with_services(
            config,
            SchedulerServices::from_config(),
            module_registry,
            Arc::new(AtomTable::with_common_atoms()),
            Arc::new(BifRegistryImpl::new()),
            Arc::new(AllCapabilitiesPolicy),
            replay_mode,
        )
    }

    fn construct_with_services(
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
            #[cfg(not(feature = "readiness"))]
            let _ = weak_shared;
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
                process_table: ProcessTable::new(),
                module_registry,
                namespace_store,
                next_namespace_id: AtomicU64::new(1),
                atom_table,
                local_node,
                ets_registry: Arc::new(EtsRegistry::new()),
                pg_registry,
                bif_registry,
                capability_policy,
                spawn_counter: AtomicUsize::new(0),
                thread_count,
                dirty_cpu,
                dirty_io,
                jit_profiler,
                jit_cache,
                next_pid: AtomicU64::new(1),
                wait_set: Mutex::new(WaitSet::default()),
                wake_condvar: Condvar::new(),
                process_bodies: DashMap::new(),
                exit_tombstones: exit_tombstones::BoundedTombstones::new(),
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
                timers: Arc::new(Mutex::new(TimerWheel::new())),
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
                observed_park_timeout_millis: AtomicU64::new(0),
                #[cfg(any(test, feature = "test-support"))]
                suspension_mirror_registrations: AtomicU64::new(0),
                #[cfg(any(test, feature = "test-support"))]
                dirty_suspension_allocations: AtomicU64::new(0),
                #[cfg(test)]
                park_gap_hook: Mutex::new(None),
            }
        });
        #[cfg(feature = "readiness")]
        if let Some(error) = readiness_error.into_inner() {
            return Err(error.to_string());
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
                    scheduler_loop(&shared_for_thread, &queue, index, &stealers, &inject);
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
        Ok(Self {
            shared,
            threads: Mutex::new(threads),
            inject_queues,
            worker_names,
        })
    }
    #[must_use]
    pub fn create_namespace(&self) -> NamespaceId {
        let id = NamespaceId(
            self.shared
                .next_namespace_id
                .fetch_add(1, Ordering::Relaxed),
        );
        debug_assert_ne!(id, NamespaceId::DEFAULT);
        self.shared
            .namespace_store
            .insert(id, Arc::new(ModuleRegistry::new()));
        id
    }
    pub fn set_trap_exit(
        &self,
        pid: u64,
        value: bool,
    ) -> Result<bool, crate::native::links::LinkError> {
        let facility = supervision_integration::SchedulerLinkFacility {
            shared: Arc::clone(&self.shared),
        };
        crate::native::LinkFacility::set_trap_exit(&facility, pid, value)
    }
    #[must_use]
    pub fn trap_exit(&self, pid: u64) -> Option<bool> {
        process_trap_exit(&self.shared, pid)
    }
    #[must_use]
    pub fn is_linked(&self, left: u64, right: u64) -> bool {
        process_links_contain(&self.shared, left, right)
            && process_links_contain(&self.shared, right, left)
    }
    /// Whether the process is native (carries a Rust handler).
    ///
    /// `Some(true)` for a parked native process, `Some(false)` for a parked
    /// bytecode process, and `None` when the process is absent (its body has
    /// been removed by `cleanup_exited_process`) or currently mid-slice (its
    /// `Process` is checked out, so native-ness is not observable from the
    /// metadata shadow). A `None` after an expected exit therefore confirms
    /// the body was removed from `process_bodies`.
    #[must_use]
    pub fn is_native(&self, pid: u64) -> Option<bool> {
        process_is_native(&self.shared, pid)
    }
    /// Establish a unidirectional monitor and return its complete result.
    ///
    /// [`crate::native::supervision::MonitorResult::immediate_down`] is true
    /// when the target already had a retained exit tombstone and the resulting
    /// DOWN was admitted to the watcher's mailbox (or to its executing-slot
    /// pending messages). It remains false for an ordinary live monitor and
    /// when a tombstoned target has no live watcher slot that can accept DOWN.
    pub fn monitor_with_result(
        &self,
        watcher_pid: u64,
        target_pid: u64,
    ) -> Result<
        crate::native::supervision::MonitorResult,
        crate::native::supervision::SupervisionError,
    > {
        let facility = supervision_integration::SchedulerSupervisionFacility {
            shared: Arc::clone(&self.shared),
        };
        crate::native::supervision::SupervisionFacility::monitor(&facility, watcher_pid, target_pid)
    }
    /// Establish a unidirectional monitor from `watcher_pid` to `target_pid`,
    /// returning the monitor reference.
    ///
    /// Delegates to the existing pid-keyed `SupervisionFacility` used by the
    /// `monitor/2` BIF, so a `{'DOWN', ref, process, pid, reason}` message is
    /// delivered to the watcher via the same `deliver_down_messages` path when
    /// the target exits — there is no native-specific monitor handling. Works
    /// uniformly for bytecode and native targets.
    pub fn monitor(
        &self,
        watcher_pid: u64,
        target_pid: u64,
    ) -> Result<u64, crate::native::supervision::SupervisionError> {
        self.monitor_with_result(watcher_pid, target_pid)
            .map(|result| result.reference)
    }
    /// Send an exit signal to `target_pid` with `reason`, the embedding-side
    /// equivalent of `erlang:exit/2`.
    ///
    /// Delegates to the existing pid-keyed `SupervisionFacility`: an abnormal
    /// reason terminates a non-trapping target through `cleanup_exited_process`
    /// (propagating to its links and monitors) or is delivered as an
    /// `{'EXIT', from, reason}` message to a trapping target. No native-specific
    /// path is involved.
    pub fn exit_signal(
        &self,
        from_pid: u64,
        target_pid: u64,
        reason: ExitReason,
    ) -> Result<(), crate::native::supervision::SupervisionError> {
        let facility = supervision_integration::SchedulerSupervisionFacility {
            shared: Arc::clone(&self.shared),
        };
        crate::native::supervision::SupervisionFacility::exit_signal(
            &facility, from_pid, target_pid, reason,
        )
    }
    /// Establish a cross-node link between local `local_pid` and `remote`,
    /// the embedding-side equivalent of `link/1` on an external pid.
    ///
    /// Records the local half-link and sends a wire LINK over the already
    /// established connection to `remote.node` (no auto-dial: connect first,
    /// then link). Errors: [`RemoteLinkError::BadTarget`] when `local_pid` is
    /// dead or absent, or when a `remote` pid component exceeds the wire's
    /// u32 range; [`RemoteLinkError::NoConnection`] when no connection to
    /// `remote.node` exists — in either error case no local half-link is left
    /// behind. `remote.serial` is normalized to 0 (the wire link identity):
    /// the peer mints every EXIT/UNLINK `from` pid with serial 0, so a
    /// nonzero serial could never match the stored link when the exit signal
    /// arrives. Delegates to the same facility the `link/1` BIF uses.
    pub fn link_remote(&self, local_pid: u64, remote: RemotePid) -> Result<(), RemoteLinkError> {
        let facility = supervision_integration::SchedulerDistributionControlFacility {
            shared: Arc::clone(&self.shared),
        };
        facility.link_remote(local_pid, remote)
    }
    /// Remove a cross-node link between local `local_pid` and `remote`, the
    /// embedding-side equivalent of `unlink/1` on an external pid.
    ///
    /// Removes the local half-link and sends a best-effort wire UNLINK (an
    /// absent connection drops it — the peer's own down handling severs its
    /// half). `remote.serial` is normalized to 0, mirroring
    /// [`Scheduler::link_remote`], so a nonzero serial cannot miss the stored
    /// half-link. Delegates to the same facility the `unlink/1` BIF uses.
    pub fn unlink_remote(&self, local_pid: u64, remote: RemotePid) -> Result<(), RemoteLinkError> {
        let facility = supervision_integration::SchedulerDistributionControlFacility {
            shared: Arc::clone(&self.shared),
        };
        facility.unlink_remote(local_pid, remote)
    }
    #[must_use]
    pub fn process_namespace(&self, pid: u64) -> Option<NamespaceId> {
        process_namespace(&self.shared, pid)
    }
    #[must_use]
    pub fn process_table(&self) -> &ProcessTable {
        &self.shared.process_table
    }
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.shared.scheduler_count()
    }
    #[must_use]
    pub fn process_count(&self) -> usize {
        self.shared.process_count()
    }
    #[must_use]
    pub fn scheduler_count(&self) -> usize {
        self.shared.scheduler_count()
    }
    #[must_use]
    pub fn atom_count(&self) -> usize {
        self.shared.atom_count()
    }
    #[must_use]
    pub fn atom_limit(&self) -> usize {
        self.shared.atom_table.limit()
    }
    #[must_use]
    pub fn local_node(&self) -> Node {
        self.shared.local_node
    }
    #[must_use]
    pub fn worker_names(&self) -> &[String] {
        &self.worker_names
    }
    /// The scheduler's ancillary-service thread inventory (spec §5).
    ///
    /// One entry per ancillary service — dirty CPU/IO pools, the file-IO and
    /// standard-IO rings, the (off-by-default) generic-IO ring, and the
    /// distribution sender and net-kernel runtimes — each carrying its live OS
    /// thread names and counts read straight from the service, so the report is
    /// what the process actually holds. Normal scheduler workers stay outside
    /// this model (spec §2.3); see [`worker_names`](Self::worker_names). The
    /// per-dirty-call transient completion thread is a policy, not a thread
    /// line; see [`service_policies`](Self::service_policies).
    ///
    /// **Process-wide dedup (Q2, spec §9):** to aggregate across co-resident
    /// schedulers, count each `Owned` entry once and each *distinct* `Shared`
    /// `instance` once — a shared ring serving N schedulers must not be counted
    /// N times. Grouping entries by [`ServiceInventoryEntry::instance`] makes
    /// this a plain group-by. In commit 1 every service is `Owned`, so the
    /// per-scheduler and deduped aggregates coincide.
    #[must_use]
    pub fn service_inventory(&self) -> Vec<ServiceInventoryEntry> {
        inventory::build_service_inventory(&self.shared)
    }

    /// Host-side acknowledged readiness deregistration.
    #[cfg(feature = "readiness")]
    pub fn readiness_deregister(&self, token: ReadinessToken) {
        self.shared.readiness_deregister(token);
    }

    /// The scheduler's transient-thread policy lines (spec §5).
    ///
    /// Classes that spawn and join OS threads in bursts — today just the
    /// per-dirty-call `dirty-complete-{pid}` thread — are reported with a spawn
    /// counter rather than as a point-in-time thread line, since a live count
    /// would under-report them.
    #[must_use]
    pub fn service_policies(&self) -> Vec<ServicePolicyLine> {
        inventory::build_service_policies(&self.shared)
    }
    /// The dirty CPU pool, or `None` when this scheduler was composed with the
    /// pool disabled (spec §3.2). Replaces `dirty_cpu_pool()` per the §6
    /// keep-old-working amendment (pair-ruled): the old `&DirtyPool` signature
    /// cannot honestly represent a Disabled pool, so the break is loud and
    /// `try_`-named at the call site.
    #[doc(alias = "dirty_cpu_pool")]
    #[must_use]
    pub fn try_dirty_cpu_pool(&self) -> Option<&DirtyPool> {
        self.shared.dirty_cpu.service()
    }
    /// The dirty IO pool, or `None` when disabled (spec §3.2/§6). Replaces
    /// `dirty_io_pool()` per the same §6 amendment as
    /// [`Self::try_dirty_cpu_pool`].
    #[doc(alias = "dirty_io_pool")]
    #[must_use]
    pub fn try_dirty_io_pool(&self) -> Option<&DirtyPool> {
        self.shared.dirty_io.service()
    }
    #[must_use]
    pub fn jit_profiler(&self) -> &Arc<JitProfiler> {
        &self.shared.jit_profiler
    }
    #[must_use]
    pub fn jit_cache(&self) -> &Arc<JitCache> {
        &self.shared.jit_cache
    }
    #[must_use]
    pub fn hook(&self) -> &Hook {
        &self.shared.hook
    }
    #[must_use]
    pub fn timers(&self) -> &Arc<Mutex<TimerWheel>> {
        &self.shared.timers
    }
    /// The distribution configuration, or `None` when distribution is `Disabled`
    /// (spec §3.6/§6). Replaces `distribution_config()` per the §6
    /// keep-old-working amendment: the old `&DistributionConfig` signature cannot
    /// honestly represent a `Disabled` bundle (there is no config to borrow and
    /// the no-panic rule forbids the only signature-preserving out), so the break
    /// is loud and `try_`-named — the old name stays discoverable via the alias.
    #[doc(alias = "distribution_config")]
    #[must_use]
    pub fn try_distribution_config(&self) -> Option<&DistributionConfig> {
        self.shared.distribution().map(DistributionService::config)
    }
    /// The distribution connection manager, or `None` when distribution is
    /// `Disabled` (spec §3.6/§6). Replaces `distribution_connections()` per the
    /// same §6 amendment as [`Self::try_distribution_config`]: a `Disabled`
    /// bundle has no manager to clone.
    #[doc(alias = "distribution_connections")]
    #[must_use]
    pub fn try_distribution_connections(&self) -> Option<ConnectionManager> {
        self.shared
            .distribution()
            .map(|dist| dist.connections().clone())
    }
    /// Start accepting inbound distribution connections on `addr`.
    ///
    /// Accepted peers run the OTP handshake (authenticated by the configured
    /// cookie) before being registered under their advertised node name. The
    /// returned [`AcceptHandle`](crate::distribution::connection::AcceptHandle)
    /// owns the accept loop: the caller must keep it alive, as dropping it aborts
    /// the loop and stops accepting new connections.
    ///
    /// Returns [`ErrorKind::Unsupported`](std::io::ErrorKind::Unsupported) when
    /// distribution is `Disabled` (spec §3.6): there is no manager to listen on,
    /// surfaced as a typed unavailable error rather than a silent absence.
    pub async fn start_distribution_listener(
        &self,
        addr: std::net::SocketAddr,
    ) -> std::io::Result<crate::distribution::connection::AcceptHandle> {
        let Some(dist) = self.shared.distribution() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "distribution is disabled on this scheduler",
            ));
        };
        dist.connections().listen(addr).await
    }
    #[must_use]
    pub fn pg_registry(&self) -> Arc<PgRegistry> {
        Arc::clone(&self.shared.pg_registry)
    }
    /// The scheduler's shared atom table.
    ///
    /// Distribution-facing embedders need this to intern names into the SAME
    /// atoms the scheduler uses internally: pg group/scope atoms and the node
    /// atoms returned by [`ConnectionManager::connected_nodes`] are indices into
    /// this table, so a separately-constructed table would not match. Mirrors the
    /// accessor [`WasmScheduler::atom_table`](crate::scheduler::WasmScheduler::atom_table)
    /// already exposes.
    #[must_use]
    pub fn atom_table(&self) -> &Arc<AtomTable> {
        &self.shared.atom_table
    }
    pub fn set_output_sink(&self, sink: Arc<dyn IoSink>) {
        *lock_or_recover(&self.shared.output_sink) = sink;
    }
    /// Cumulative idle parks across this scheduler's normal workers — one per
    /// `park_thread` entry. The sampling source for the signed §3.8 idle-wake
    /// bound; test-only instrumentation (`test-support`), absent from
    /// production builds.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn idle_park_count(&self) -> usize {
        self.shared.idle_parks.load(Ordering::Acquire)
    }
    /// The park timeout the workers are ACTUALLY using, in milliseconds —
    /// `None` until the first park. Deterministic linkage for the signed
    /// §3.8 floor: asserting this equals [`IDLE_PARK_TIMEOUT`] catches an
    /// implementation whose wait duration decoupled from the signed
    /// constant, with zero sensitivity to host load.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn observed_park_timeout_millis(&self) -> Option<u64> {
        match self
            .shared
            .observed_park_timeout_millis
            .load(Ordering::Acquire)
        {
            0 => None,
            millis => Some(millis),
        }
    }
    /// Cumulative suspension-mirror registrations across this scheduler —
    /// one per `register_suspension_mirror` call, counted at registration
    /// rather than inferred from the live mirror map. Boundary instrument
    /// for the §3.2 refusal ordering: a refused dirty call must leave this
    /// unmoved, which distinguishes refusal-before-registration from
    /// refusal-after-registration-plus-cleanup. Test-only instrumentation
    /// (`test-support`), absent from production builds.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn suspension_mirror_registration_count(&self) -> u64 {
        self.shared
            .suspension_mirror_registrations
            .load(Ordering::Acquire)
    }
    /// Cumulative entries into the dirty-call gated-suspension side-effect
    /// sequence (first step: call-id allocation). Paired with
    /// [`Self::suspension_mirror_registration_count`], the two instruments
    /// bracket the whole sequence, so the §3.2 refusal gates catch a check
    /// that regresses to anywhere inside it. Test-only instrumentation
    /// (`test-support`), absent from production builds.
    /// Live dirty completion-bridge threads this scheduler has spawned —
    /// 0 after a drained shutdown (spec §4 step 3). Test-support instrument.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn dirty_completions_live_count(&self) -> usize {
        self.shared.dirty_completions_live_count()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn dirty_suspension_allocation_count(&self) -> u64 {
        self.shared
            .dirty_suspension_allocations
            .load(Ordering::Acquire)
    }
}
#[cfg(feature = "threads")]
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.shutdown();
    }
}
#[cfg(feature = "threads")]
fn configured_thread_count(override_count: Option<usize>) -> usize {
    override_count
        .filter(|count| *count > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        })
}

/// Build a dirty pool slot from a config request (spec §3.2/§6):
/// `Some(0)` → `Disabled` (no channel, no workers); `Some(n>0)` → `Owned(n)`;
/// `None` → `Owned(default_threads)`. A zero request never constructs a pool,
/// so a disabled dirty dispatch has nothing to submit into.
/// Resolve a dirty pool slot from its composition choice (spec §2.2/§3.2).
///
/// `FromConfig` defers to the legacy `dirty_*_threads` knob; `Disabled` owns
/// nothing; `Owned(n)` builds `n` workers (`n == 0` disables, matching the
/// `Some(0)` knob semantics); `Shared` injects an embedder-owned pool this
/// scheduler uses but NEVER joins (spec §2.1). The `Shared` arm is safe now
/// because dirty completion routes by the oneshot the submission carries — the
/// pool worker never consults a per-scheduler table — so one pool can back two
/// schedulers without the cross-scheduler routing the rings still lack.
#[cfg(feature = "threads")]
fn resolve_dirty_pool(
    name: &str,
    choice: &services::DirtyChoice,
    config_knob: Option<usize>,
    default_threads: usize,
    queue_depth: usize,
) -> ServiceMode<DirtyPool> {
    match choice {
        services::DirtyChoice::FromConfig => {
            build_dirty_pool(name, config_knob, default_threads, queue_depth)
        }
        services::DirtyChoice::Disabled => ServiceMode::Disabled,
        services::DirtyChoice::Owned(workers) => {
            build_dirty_pool(name, Some(*workers), default_threads, queue_depth)
        }
        services::DirtyChoice::Shared(pool) => ServiceMode::Shared(Arc::clone(pool)),
    }
}

#[cfg(feature = "threads")]
fn build_dirty_pool(
    name: &str,
    requested: Option<usize>,
    default_threads: usize,
    queue_depth: usize,
) -> ServiceMode<DirtyPool> {
    match requested {
        Some(0) => ServiceMode::Disabled,
        Some(threads) => {
            ServiceMode::Owned(DirtyPool::with_queue_depth(name, threads, queue_depth))
        }
        None => ServiceMode::Owned(DirtyPool::with_queue_depth(
            name,
            default_threads,
            queue_depth,
        )),
    }
}
#[cfg(feature = "threads")]
fn process_namespace(shared: &SharedState, pid: u64) -> Option<NamespaceId> {
    let entry = shared.process_bodies.get(&pid)?;
    match &*lock_or_recover(&entry) {
        ProcessSlot::Present(scheduled) => Some(scheduled.0.namespace_id()),
        ProcessSlot::Executing(metadata) => Some(metadata.namespace_id),
        ProcessSlot::Absent => None,
    }
}
#[cfg(feature = "threads")]
fn process_trap_exit(shared: &SharedState, pid: u64) -> Option<bool> {
    let entry = shared.process_bodies.get(&pid)?;
    match &*lock_or_recover(&entry) {
        ProcessSlot::Present(scheduled) => Some(scheduled.0.trap_exit()),
        ProcessSlot::Executing(metadata) => Some(metadata.trap_exit),
        ProcessSlot::Absent => None,
    }
}
#[cfg(feature = "threads")]
fn process_is_native(shared: &SharedState, pid: u64) -> Option<bool> {
    let entry = shared.process_bodies.get(&pid)?;
    match &*lock_or_recover(&entry) {
        ProcessSlot::Present(scheduled) => Some(scheduled.0.is_native()),
        // Mid-slice the `Process` is checked out; native-ness lives on it, not
        // on the metadata shadow. Absent means the body is being swapped.
        ProcessSlot::Executing(_) | ProcessSlot::Absent => None,
    }
}
#[cfg(feature = "threads")]
fn process_links_contain(shared: &SharedState, pid: u64, linked_pid: u64) -> bool {
    let Some(entry) = shared.process_bodies.get(&pid) else {
        return false;
    };
    match &*lock_or_recover(&entry) {
        ProcessSlot::Present(scheduled) => scheduled.0.links().contains(&linked_pid),
        ProcessSlot::Executing(metadata) => metadata.links.contains(&linked_pid),
        ProcessSlot::Absent => false,
    }
}

#[cfg(feature = "threads")]
impl SharedState {
    pub(super) fn process_info(&self, pid: u64, item: ProcessInfoItem) -> Option<ProcessInfoValue> {
        self.process_table.get(pid)?;
        let entry = self.process_bodies.get(&pid)?;
        match &*lock_or_recover(&entry) {
            ProcessSlot::Present(scheduled) => process_info_from_process(&scheduled.0, item),
            ProcessSlot::Executing(metadata) => process_info_from_metadata(metadata, item),
            ProcessSlot::Absent => None,
        }
    }
}

#[cfg(feature = "threads")]
fn process_info_from_process(process: &Process, item: ProcessInfoItem) -> Option<ProcessInfoValue> {
    if matches!(process.status(), ProcessStatus::Exited(_)) {
        return None;
    }
    Some(match item {
        ProcessInfoItem::CurrentFunction => {
            ProcessInfoValue::CurrentFunction(process.current_mfa())
        }
        ProcessInfoItem::HeapSize => ProcessInfoValue::HeapSize(process.heap().total_used()),
        ProcessInfoItem::MessageQueueLen => {
            ProcessInfoValue::MessageQueueLen(process.mailbox().message_count())
        }
        ProcessInfoItem::RegisteredName => ProcessInfoValue::RegisteredName(None),
        ProcessInfoItem::Status => ProcessInfoValue::Status(status_from_process(process.status())?),
        ProcessInfoItem::TrapExit => ProcessInfoValue::TrapExit(process.trap_exit()),
        ProcessInfoItem::Priority => ProcessInfoValue::Priority(process.priority()),
        ProcessInfoItem::Links => ProcessInfoValue::Links(process.links().to_vec()),
        ProcessInfoItem::Monitors => ProcessInfoValue::Monitors(
            process
                .monitors()
                .iter()
                .map(|monitor| ProcessMonitorInfo {
                    watcher: monitor.watcher(),
                    target: monitor.target(),
                })
                .collect(),
        ),
    })
}

#[cfg(feature = "threads")]
fn process_info_from_metadata(
    metadata: &ProcessMetadata,
    item: ProcessInfoItem,
) -> Option<ProcessInfoValue> {
    Some(match item {
        ProcessInfoItem::CurrentFunction => ProcessInfoValue::CurrentFunction(metadata.current_mfa),
        ProcessInfoItem::HeapSize => ProcessInfoValue::HeapSize(metadata.heap_size),
        ProcessInfoItem::MessageQueueLen => {
            ProcessInfoValue::MessageQueueLen(metadata.message_queue_len)
        }
        ProcessInfoItem::RegisteredName => ProcessInfoValue::RegisteredName(None),
        ProcessInfoItem::Status => ProcessInfoValue::Status(ProcessInfoStatus::Running),
        ProcessInfoItem::TrapExit => ProcessInfoValue::TrapExit(metadata.trap_exit),
        ProcessInfoItem::Priority => ProcessInfoValue::Priority(metadata.priority),
        ProcessInfoItem::Links => ProcessInfoValue::Links(metadata.links.clone()),
        ProcessInfoItem::Monitors => ProcessInfoValue::Monitors(
            metadata
                .monitors
                .iter()
                .map(|monitor| ProcessMonitorInfo {
                    watcher: monitor.watcher(),
                    target: monitor.target(),
                })
                .collect(),
        ),
    })
}

#[cfg(feature = "threads")]
fn status_from_process(status: ProcessStatus) -> Option<ProcessInfoStatus> {
    match status {
        ProcessStatus::New | ProcessStatus::Running | ProcessStatus::Yielded => {
            Some(ProcessInfoStatus::Running)
        }
        ProcessStatus::Waiting => Some(ProcessInfoStatus::Waiting),
        ProcessStatus::Suspended => Some(ProcessInfoStatus::Suspended),
        ProcessStatus::Exited(_) => None,
    }
}

#[cfg(feature = "threads")]
pub(super) fn namespace_registry(
    shared: &SharedState,
    namespace: NamespaceId,
) -> Option<Arc<ModuleRegistry>> {
    shared
        .namespace_store
        .get(&namespace)
        .map(|entry| Arc::clone(entry.value()))
}
#[cfg(feature = "threads")]
pub(super) fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(feature = "threads")]
impl Scheduler {
    /// Enqueue an immediate atom message into a live process mailbox and wake
    /// the process if it is parked.
    ///
    /// Embedders use this as a host-to-process wake primitive (e.g. activity
    /// completion markers). Delivery must succeed in every live slot state: a
    /// process currently executing a slice receives the message through its
    /// pending metadata, which the scheduler merges into the mailbox at
    /// store-back and then resumes the process if it suspended meanwhile —
    /// otherwise a completion racing the suspend transition is lost and the
    /// process sleeps forever.
    ///
    /// The wake applies to plain receives and message-wakeable suspends
    /// (`ProcessContext::request_suspend`). A process parked under a *gated*
    /// suspension (`request_await_suspend`, an in-flight dirty call, a hook
    /// suspend) keeps the message in its mailbox but stays parked until its
    /// own completion event arrives — waking it would re-execute the parked
    /// call instruction and repeat its host side effect.
    ///
    /// Returns false only when no live process exists for `target_pid`.
    #[must_use]
    pub fn enqueue_atom_message(&self, target_pid: u64, atom: crate::atom::Atom) -> bool {
        let delivered =
            timer_integration::deliver_term_to_mailbox(&self.shared, target_pid, Term::atom(atom));
        if delivered.is_ok() {
            execution::wake_process(&self.shared, target_pid);
        }
        delivered.is_ok()
    }

    /// Send an owned arbitrary term to a process mailbox.
    ///
    /// The message is deep-copied from [`OwnedTerm`] storage into the target
    /// process heap before this method returns. A waiting target is woken after
    /// the message is visible. If the target is executing, admission is recorded
    /// in its executing-slot metadata, copied at slice store-back, and then
    /// woken; the receive park path's post-registration mailbox recheck closes
    /// the store-back-to-wait-set gap, so the next receive observes the message
    /// without a lost wake. Each successful send contributes at most one wake.
    ///
    /// This supports event-driven host consumers such as R-B-1 supervisors:
    /// encode command identity and payload in a structurally disjoint tagged
    /// tuple, own it with [`crate::ets::copy_term_to_ets`], then send that value
    /// here and handle [`MailboxSendError`] instead of maintaining a side queue
    /// plus a wake-only notification.
    ///
    /// Deliveries are appended under the target slot lock, preserving FIFO order
    /// with this scheduler's atom and timer mailbox deliveries.
    pub fn send_to_mailbox(
        &self,
        target_pid: u64,
        message: OwnedTerm,
    ) -> Result<(), MailboxSendError> {
        timer_integration::deliver_owned_term_to_mailbox(&self.shared, target_pid, message)?;
        execution::wake_process(&self.shared, target_pid);
        Ok(())
    }
}

#[cfg(feature = "threads")]
impl IoWakeTarget for SharedState {
    fn wake_with_io_result(&self, pid: u64, term: Term) {
        // Identity-resolved at publish time: the bridge completes the
        // host-await suspension the submitting native registered. A stale
        // completion (the await already timed out and re-entered) is
        // dropped instead of being applied blind.
        let Some(payload) = suspension::SuspensionResultPayload::host(term) else {
            return;
        };
        let _published = self.publish_suspension_result_current(
            pid,
            crate::process::SuspensionKind::HostAwait,
            payload,
        );
        execution::wake_process(self, pid);
    }

    fn send_io_message(&self, pid: u64, term: Term) {
        let Some(entry) = self.process_bodies.get(&pid) else {
            return;
        };
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Present(process) = &mut *slot {
            process.0.mailbox_mut().push_owned(term);
        } else if let ProcessSlot::Executing(metadata) = &mut *slot {
            metadata
                .pending_io_messages
                .push(PendingMailboxMessage::TargetOwned(term));
        }
        drop(slot);
        drop(entry);
        if pid == self.standard_io_pid {
            let mut wait_set = lock_or_recover(&self.wait_set);
            wait_set.woken.push((pid, 0));
            self.wake_condvar.notify_all();
        } else {
            execution::wake_process(self, pid);
        }
    }
}

#[cfg(feature = "threads")]
#[cfg(test)]
mod closure_spawn_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod connection_lifecycle_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod inventory_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod mailbox_send_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod monitor_immediate_down_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod readiness_contract_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod remote_supervision_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod supervision_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod tests;
