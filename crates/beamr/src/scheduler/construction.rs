//! The public ways to construct a threaded scheduler, live or replaying.

use super::services::{NativeBifs, SchedulerServices};
use super::{ReplayMode, Scheduler};
use crate::atom::AtomTable;
use crate::module::ModuleRegistry;
use crate::native::{AllCapabilitiesPolicy, BifRegistryImpl, CapabilityPolicy};
use crate::replay::ReplayLog;
use std::sync::Arc;

use super::config::SchedulerConfig;

impl Scheduler {
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
    /// service footprint should use [`Scheduler::with_services_and_code_server`]
    /// with [`SchedulerServices::minimal`] /
    /// [`SchedulerServices::full_runtime`] instead; this constructor is retained
    /// for source compatibility and will keep the legacy defaults for one
    /// release (see CHANGELOG migration note).
    ///
    /// # Declared native BIF surface
    ///
    /// `natives` is the native BIF surface this scheduler resolves against, and
    /// it is required: there is no default, so an empty surface is a value the
    /// call site wrote down.
    ///
    /// [`NativeBifs::none()`](NativeBifs::none) declares that this scheduler
    /// resolves no natives. Under that declaration — and absent a loaded
    /// bytecode `erlang` module — every `erlang:*` import (essentially every
    /// erlc-compiled module: `module_info` alone imports
    /// `erlang:get_module_info`) resolves
    /// [`Deferred`](crate::error::GuardBifResolution::Deferred), and the first
    /// guard-BIF execution (any arithmetic, comparison, or type guard) refuses
    /// with a process-fatal
    /// [`ExecError::GuardBifUnavailable`](crate::error::ExecError). If a bytecode
    /// `erlang` module IS loaded the same imports resolve `Code` (exported) or
    /// `Unresolved` (missing export) instead — still non-native, still
    /// guard-refused. That is a legitimate composition for a scheduler running
    /// only native actors; it is a defect for one that loads a `.beam`.
    ///
    /// To execute real BEAM bytecode, declare
    /// [`NativeBifs::registry`](NativeBifs::registry) with at least
    /// [`register_gate1_bifs`](crate::native::bifs::register_gate1_bifs) applied
    /// to it, hand the SAME registry to
    /// [`load_module`](crate::loader::load_module) — imports bind at LOAD time —
    /// and consult the returned [`HotLoadResult`]'s unresolved-import report
    /// after every load (see [`Scheduler::with_services`] for a worked example).
    pub fn new(
        config: SchedulerConfig,
        module_registry: Arc<ModuleRegistry>,
        natives: NativeBifs,
    ) -> Result<Self, String> {
        Self::with_services_and_code_server(
            config,
            SchedulerServices::from_config(),
            module_registry,
            Arc::new(AtomTable::with_common_atoms()),
            natives.into_registry(),
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
    ///
    /// # Declared native BIF surface
    ///
    /// Like [`Scheduler::new`], this constructor takes its native surface as a
    /// required `natives` argument: it composes the ancillary SERVICES you ask
    /// for, and the natives are a separate declaration it will not make for you.
    ///
    /// Under [`NativeBifs::none()`](NativeBifs::none) — and absent a loaded
    /// bytecode `erlang` module — every `erlang:*` import resolves
    /// [`Deferred`](crate::error::GuardBifResolution::Deferred) and the first
    /// guard-BIF execution refuses, process-fatal, with
    /// [`ExecError::GuardBifUnavailable`](crate::error::ExecError); a loaded
    /// bytecode `erlang` module shifts that to `Code`/`Unresolved`, still
    /// non-native and still refused.
    ///
    /// Embedders executing real BEAM bytecode declare
    /// [`NativeBifs::registry`](NativeBifs::registry) with at least
    /// [`register_gate1_bifs`](crate::native::bifs::register_gate1_bifs)
    /// applied, load against that same registry — imports bind at LOAD time, so
    /// a scheduler carrying a different registry object does not repair a load
    /// performed against an empty one — and consult the [`HotLoadResult`]'s
    /// unresolved-import report after every load:
    ///
    /// ```rust,no_run
    /// # fn compose() -> Result<(), String> {
    /// use std::sync::Arc;
    /// use beamr::atom::AtomTable;
    /// use beamr::module::ModuleRegistry;
    /// use beamr::native::BifRegistryImpl;
    /// use beamr::native::bifs::register_gate1_bifs;
    /// use beamr::scheduler::{Scheduler, SchedulerConfig, SchedulerServices};
    ///
    /// let atoms = Arc::new(AtomTable::with_common_atoms());
    /// let bifs = BifRegistryImpl::new();
    /// // Register the gate1 native BIFs (erlang:'+'/2 and friends) BEFORE loading
    /// // any module that imports erlang:* — which is every erlc-compiled module.
    /// register_gate1_bifs(&bifs, &atoms).map_err(|error| error.to_string())?;
    ///
    /// let scheduler = Scheduler::with_services_and_code_server(
    ///     SchedulerConfig::default(),
    ///     SchedulerServices::minimal(),
    ///     Arc::new(ModuleRegistry::new()),
    ///     atoms,
    ///     Arc::new(bifs),
    /// )?;
    /// // After each hot-load, treat a non-empty `erlang:*` deferred report as a
    /// // composition error rather than waiting for the process-fatal refusal.
    /// let _ = &scheduler;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_services(
        config: SchedulerConfig,
        services: SchedulerServices,
        module_registry: Arc<ModuleRegistry>,
        natives: NativeBifs,
    ) -> Result<Self, String> {
        Self::with_services_and_code_server(
            config,
            services,
            module_registry,
            Arc::new(AtomTable::with_common_atoms()),
            natives.into_registry(),
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
    ///
    /// # Declared native BIF surface
    ///
    /// `natives` is required and carries through to the replay scheduler
    /// unchanged. Declared [`NativeBifs::none()`](NativeBifs::none), a replayed
    /// module importing `erlang:*` refuses at its first guard-BIF execution
    /// exactly as under [`Scheduler::new`] — see that constructor for the full
    /// explanation.
    ///
    /// Embedders replaying real BEAM bytecode want
    /// [`Scheduler::replay_with_services_and_code_server`], which carries an
    /// atom table alongside the BIF registry into replay mode.
    pub fn new_replay(
        config: SchedulerConfig,
        log: ReplayLog,
        natives: NativeBifs,
    ) -> Result<Self, String> {
        Self::new_replay_with_registry(config, Arc::new(ModuleRegistry::new()), log, natives)
    }

    /// Create a replay scheduler using an explicit module registry.
    ///
    /// # Declared native BIF surface
    ///
    /// The "registry" this constructor names is the MODULE registry; it carries
    /// no natives. The native surface is the separate, required `natives`
    /// argument. Declared [`NativeBifs::none()`](NativeBifs::none), `erlang:*`
    /// imports resolve non-native and refuse process-fatal at the first
    /// guard-BIF execution, the same shape as [`Scheduler::new`].
    ///
    /// Embedders replaying real BEAM bytecode declare
    /// [`NativeBifs::registry`](NativeBifs::registry) with at least
    /// [`register_gate1_bifs`](crate::native::bifs::register_gate1_bifs)
    /// applied, or use [`Scheduler::replay_with_services_and_code_server`] — the
    /// replay counterpart of [`Scheduler::with_services_and_code_server`], which
    /// also carries the load-time atom table. See [`Scheduler::new`] for the
    /// full explanation.
    pub fn new_replay_with_registry(
        config: SchedulerConfig,
        module_registry: Arc<ModuleRegistry>,
        log: ReplayLog,
        natives: NativeBifs,
    ) -> Result<Self, String> {
        Self::construct(config, module_registry, ReplayMode::Replay(log), natives)
    }

    /// [`Scheduler::with_services_and_code_server`] in deterministic replay
    /// mode over `log`.
    ///
    /// The replay counterpart of the live full-composition constructor: it
    /// takes the same services, module registry, atom table and BIF registry,
    /// so a replayed module resolves atoms and dispatches natives against the
    /// same load-time state the modules were loaded against. This is the
    /// full-composition replay path [`Scheduler::new_replay`] and
    /// [`Scheduler::new_replay_with_registry`] direct embedders to. Those two
    /// require a declared native surface of their own, so whether `erlang:*`
    /// refuses is a property of the caller's declaration rather than of which
    /// constructor was used; what this one adds is the REST of the load-time
    /// state — an explicit [`SchedulerServices`] and the atom table the modules
    /// were loaded against, both of which the other two mint fresh.
    ///
    /// As with every replay-mode scheduler the worker count is forced to 1 and
    /// the ancillary services that would introduce nondeterminism (generic and
    /// distribution rings, owned file and standard I/O) are disabled.
    ///
    /// # Errors
    ///
    /// Returns the composition error when `services` does not validate.
    pub fn replay_with_services_and_code_server(
        config: SchedulerConfig,
        services: SchedulerServices,
        module_registry: Arc<ModuleRegistry>,
        atom_table: Arc<AtomTable>,
        bif_registry: Arc<BifRegistryImpl>,
        log: ReplayLog,
    ) -> Result<Self, String> {
        services.validate().map_err(|error| error.to_string())?;
        Self::construct_with_services(
            config,
            services,
            module_registry,
            atom_table,
            bif_registry,
            Arc::new(AllCapabilitiesPolicy),
            ReplayMode::Replay(log),
        )
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
        natives: NativeBifs,
    ) -> Result<Self, String> {
        Self::construct_with_services(
            config,
            SchedulerServices::from_config(),
            module_registry,
            Arc::new(AtomTable::with_common_atoms()),
            natives.into_registry(),
            Arc::new(AllCapabilitiesPolicy),
            replay_mode,
        )
    }
}
