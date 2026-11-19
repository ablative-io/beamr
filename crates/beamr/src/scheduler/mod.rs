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

/// The loader's grouped unresolved/deferred/denied import report, re-exported
/// at the scheduler surface so a [`HotLoadResult`] consumer can read it without
/// reaching into `loader`.
pub use crate::loader::UnresolvedImportReport;

/// Result returned by a successful hot module load.
///
/// Plain metadata returned by the threaded code server. It is named in the
/// platform-neutral `CodeManagementFacility` trait, so it is defined here (always
/// available) even though only the threaded scheduler can produce one.
///
/// `unresolved` carries the import resolution the load actually performed —
/// deferred/denied/unresolved by module, byte-identical to what
/// [`loader::load_module`](crate::loader::load_module) returns. When the load
/// resolved against a registry holding no `erlang:*` natives — a value the
/// embedder declares at scheduler construction, never a default — and absent a
/// loaded bytecode `erlang` module, a module importing `erlang:*`
/// lands those imports under
/// [`deferred_by_module`](UnresolvedImportReport::deferred_by_module); reading
/// it after every load is how an embedder catches the composition footgun
/// before the process-fatal guard-BIF refusal instead of after. This is the
/// honest API surface for that signal — carrying the report (not a getter on
/// mutable scheduler state) keeps it bound to the load RESULT.
///
/// Because the report owns per-module maps it is not `Copy`; `HotLoadResult` is
/// therefore `Clone` but no longer `Copy`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotLoadResult {
    pub module_name: crate::atom::Atom,
    pub generation: u64,
    pub had_old_version: bool,
    pub on_load_required: bool,
    pub on_load_succeeded: bool,
    /// The import resolution this load performed, grouped by module.
    pub unresolved: UnresolvedImportReport,
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
mod exit_events;
#[cfg(feature = "threads")]
mod failure;
#[cfg(feature = "threads")]
pub use failure::{
    DeathTier, PoisonCheck, SchedulerFailure, WaitSetCheck, WorkerDeath, WorkerEnd, WorkerSite,
};
#[cfg(feature = "threads")]
mod exit_tombstones;
#[cfg(feature = "threads")]
mod inventory;
#[cfg(feature = "threads")]
pub use exit_events::{
    EXIT_EVENT_CAPACITY, ExitCompletionWatch, ExitCompletionWatchState, ExitEvent,
    ExitEventRecvError, ExitEventSubscription, ExitWatch, ExitWatchState,
};
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
#[cfg(all(feature = "threads", any(test, feature = "test-support")))]
pub use execution::{
    ObservedParkTimeout, WakeCauses, WorkerHold, WorkerParkCounts, park_seam_panic_message,
    slice_seam_panic_message,
};
#[cfg(feature = "threads")]
pub use inventory::{
    ProcessVmCensus, ServiceInventoryEntry, ServicePolicyLine, deduped_thread_aggregate,
    process_vm_census,
};
#[cfg(feature = "readiness")]
pub use readiness::{
    Generation, Interest, ReadinessBuildError, ReadinessError, ReadinessToken, SharedReadiness,
};
#[cfg(feature = "threads")]
pub use service::{
    ServiceIdentity, ServiceInstanceId, ServiceMode, ServiceModeLabel, ShutdownService,
};
#[cfg(feature = "threads")]
pub use services::{NativeBifs, SchedulerServices, SharedIoRing, WithServicesError};
#[cfg(feature = "threads")]
mod accessors;
#[cfg(feature = "threads")]
mod assembly;
#[cfg(feature = "threads")]
mod config;
#[cfg(feature = "threads")]
mod construction;
#[cfg(feature = "threads")]
mod dirty_pools;
#[cfg(feature = "threads")]
mod distribution_access;
#[cfg(feature = "threads")]
mod ets_tables;
#[cfg(feature = "threads")]
mod exit_publication;
#[cfg(feature = "threads")]
mod failure_observers;
#[cfg(feature = "threads")]
mod io_bridge;
#[cfg(feature = "threads")]
mod jit_control;
#[cfg(feature = "threads")]
mod mailbox_send;
#[cfg(feature = "threads")]
mod module_management;
#[cfg(feature = "threads")]
mod pg_propagation;
#[cfg(feature = "threads")]
mod process_queries;
#[cfg(feature = "threads")]
mod process_slot;
#[cfg(feature = "readiness")]
mod readiness_registration;
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
mod supervision_api;
#[cfg(feature = "threads")]
mod supervision_integration;
#[cfg(feature = "threads")]
mod suspension;
#[cfg(feature = "threads")]
mod system_info;
#[cfg(feature = "threads")]
mod teardown_admission;
#[cfg(all(test, feature = "readiness"))]
pub(crate) mod teardown_admission_tests;
#[cfg(all(feature = "threads", feature = "telemetry"))]
mod telemetry_metrics;
#[cfg(feature = "threads")]
mod test_helpers;
#[cfg(feature = "threads")]
mod timer_integration;
#[cfg(all(feature = "threads", any(test, feature = "test-support")))]
mod worker_probes;
#[cfg(feature = "threads")]
use self::dirty::DirtyPool;
#[cfg(feature = "threads")]
use self::spawning::SpawnRequest;
#[cfg(feature = "threads")]
use crate::atom::AtomTable;
#[cfg(feature = "threads")]
use crate::distribution::Node;
#[cfg(feature = "threads")]
use crate::distribution::pg::PgRegistry;
#[cfg(feature = "threads")]
pub use config::SchedulerConfig;
#[cfg(all(feature = "threads", test))]
use jit_control::INJECT_JIT_COMPILER_FAILURE;
#[cfg(feature = "threads")]
pub use mailbox_send::MailboxSendError;
#[cfg(all(feature = "threads", test))]
use process_queries::process_links_contain;
#[cfg(feature = "threads")]
use teardown_admission::{TeardownAdmission, TeardownAdmissionRegistry};
#[cfg(all(feature = "threads", feature = "telemetry"))]
use telemetry_metrics::TelemetryMetricState;

#[cfg(feature = "threads")]
use crate::error::ExecError;
#[cfg(feature = "threads")]
use crate::ets::EtsRegistry;
#[cfg(feature = "threads")]
use crate::ets::copy::OwnedTerm;
#[cfg(feature = "threads")]
use crate::hook::Hook;
#[cfg(feature = "threads")]
use crate::io::{IoCompletion, IoCompletionBridge, IoFacility, IoSink, PendingIoRegistry};
#[cfg(feature = "threads")]
use crate::jit::{JitCache, JitCompiler, JitProfiler};
#[cfg(feature = "threads")]
use crate::module::ModuleRegistry;
#[cfg(feature = "threads")]
use crate::namespace::NamespaceId;
#[cfg(feature = "threads")]
use crate::native::{BifRegistryImpl, CapabilityPolicy, FileIoCompletion, FileIoContinuation};
#[cfg(feature = "threads")]
use crate::process::Process;
#[cfg(feature = "threads")]
use crate::process::registry::ProcessTable;
#[cfg(feature = "threads")]
use crate::replay::{ReplayDriver, ReplayLog};
#[cfg(feature = "threads")]
use crate::supervision::link::LinkSet;
#[cfg(feature = "threads")]
use crate::supervision::monitor::MonitorSet;
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
use readiness::{ReadinessConsumer, ReadinessService};
#[cfg(feature = "threads")]
use ring_service::{RingService, StandardIoService};
#[cfg(feature = "threads")]
use run_queue::{PriorityStealers, RunQueue};
#[cfg(feature = "threads")]
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
#[cfg(feature = "threads")]
use std::sync::{Arc, Condvar, Mutex};
#[cfg(feature = "threads")]
use std::thread::JoinHandle;

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
pub(super) struct SharedState {
    shutdown: AtomicBool,
    /// B-184: the permanent failure a dead worker latches.
    failure: failure::FailureLatch,
    /// B-184: each worker's report of its own thread's exit.
    worker_exits: failure::WorkerExits,
    /// B-184: the workers shutdown did not join after the latch, as its log
    /// line named them.
    shutdown_skipped: Mutex<Vec<String>>,
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
    jit_compiler: Arc<JitCompiler>,
    jit_cache: Arc<JitCache>,
    /// Operator-set JIT master switch (`true` at construction, preserving the
    /// historical always-on behaviour). Read once per slice when the native
    /// services are assembled: `false` withholds BOTH the cache and the
    /// profiling handle from the call edges, so compiled code is neither
    /// produced nor entered. Never written by the engine itself — an engine
    /// that disabled its own JIT on a detected fault would be a silent
    /// fallback, which is a design decision and not this switch's job.
    jit_enabled: AtomicBool,
    next_pid: AtomicU64,
    wait_set: Mutex<WaitSet>,
    wake_condvar: Condvar,
    /// The wake sequence edge E advances and the idle count of §3.8.2 C1
    /// (B-181 R2): what orders every wake against a worker's park.
    idle_wake: execution::IdleWake,
    process_bodies: DashMap<u64, Mutex<ProcessSlot>>,
    exit_tombstones: exit_tombstones::BoundedTombstones,
    exit_completion_watches: Arc<exit_events::ExitCompletionRegistry>,
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

    /// Per-worker park instruments: how often each worker parked, how each
    /// park ended, and the timeout it last waited with — the value actually
    /// passed to the wait, never a constant beside it (§3.8, B-181).
    #[cfg(any(test, feature = "test-support"))]
    park_seams: execution::ParkSeams,

    /// Test-only controls on the park: the unbounded-wait seam and the hooks
    /// at the idle gap and at the start of a wait (B-181's reds).
    #[cfg(test)]
    park_hooks: execution::ParkHooks,

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

    #[cfg(test)]
    mailbox_admission_hook: std::sync::OnceLock<MailboxAdmissionHook>,
    #[cfg(test)]
    terminal_admission_hook: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    terminal_spawn_visits: AtomicUsize,
    #[cfg(test)]
    terminal_body_visits: AtomicUsize,
    #[cfg(test)]
    spawn_admission_hook: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    teardown_wait_hook: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    shutdown_dispatch_hook: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
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

#[cfg(all(test, feature = "threads"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MailboxAdmissionPoint {
    BeforeSlot,
    Queued,
}

#[cfg(all(test, feature = "threads"))]
type MailboxAdmissionHook = Box<dyn Fn(u64, MailboxAdmissionPoint) + Send + Sync>;
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
    /// Process-global census registration (#106). Held for the scheduler's
    /// whole lifetime — its Drop is what removes this VM from
    /// [`process_vm_census`], so the census population is "constructed and
    /// not yet dropped" by construction.
    _vm_census: inventory::VmCensusRegistration,
}
#[cfg(feature = "threads")]
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.shutdown();
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
#[cfg(test)]
mod closure_spawn_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod connection_lifecycle_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod exit_observation_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod failure_tests;
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
#[cfg(feature = "threads")]
#[cfg(test)]
mod tickless_idle_tests;
#[cfg(feature = "threads")]
#[cfg(test)]
mod watcher_scale_tests;
