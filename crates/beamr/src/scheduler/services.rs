//! Composition entrypoint (spec §2.2): [`SchedulerServices`] + named profiles.
//!
//! [`Scheduler::with_services`](super::Scheduler::with_services) is the additive
//! composition entrypoint: the embedder gets exactly the ancillary services it
//! asks for (spec §0). Each service is described by a per-service *choice* whose
//! arms mirror [`ServiceMode`](super::service::ServiceMode) —
//! [`FromConfig`](DirtyChoice::FromConfig) defers to the matching legacy
//! [`SchedulerConfig`](super::SchedulerConfig) knob, and the explicit arms
//! (`Disabled`/`Owned`/`Shared`) override it.
//!
//! ## Precedence rule (spec §2.2)
//!
//! For every ancillary service, `with_services(config, services)` resolves the
//! slot from `services` FIRST and only falls back to the `config` knob when the
//! choice is `FromConfig`. Concretely: an explicit `Disabled`/`Owned`/`Shared`
//! choice WINS over `config.dirty_cpu_threads`, `config.io`,
//! `config.distribution`, etc.; a `FromConfig` choice is resolved exactly as
//! [`Scheduler::new`](super::Scheduler::new) (the legacy profile) would. There
//! is no third source, so the resolution is never ambiguous: a knob is read iff
//! its service's choice is `FromConfig`. `config`'s non-service knobs
//! (`thread_count`, `node_name`, `creation`, `dirty_queue_depth`,
//! `telemetry_sample_interval`, `nif_private_data`) are always honored — they
//! are not ancillary services (`thread_count` is the workers the embedder asked
//! for by constructing a scheduler at all, spec §2.3).
//!
//! ## Profiles
//!
//! - [`SchedulerServices::full_runtime`] — today's full standalone VM,
//!   explicitly requested: every ancillary service `FromConfig` PLUS
//!   distribution turned on with a default config (the one service that became
//!   honest-absent by default in commit 4, spec §3.6). This is what the CLI and
//!   any standalone embedder opts into.
//! - [`SchedulerServices::minimal`] — every ancillary service `Disabled`: zero
//!   dirty pools, no file/standard/generic ring, no process 0, no distribution
//!   (spec §5 permanent assertion 1). Only the requested normal workers run.
//! - [`SchedulerServices::from_config`] (the legacy profile) — every service
//!   `FromConfig`. `Scheduler::new` maps here, preserving today's per-knob
//!   defaults for one release (spec §2.2/§6).

use std::sync::Arc;

use super::dirty::DirtyPool;
#[cfg(feature = "readiness")]
use super::readiness::SharedReadiness;
use crate::distribution::DistributionConfig;
use crate::io::RingConfig;

/// Composition choice for a dirty pool (spec §2.2/§3.2).
#[derive(Clone)]
pub(super) enum DirtyChoice {
    /// Defer to the matching `SchedulerConfig` knob (`dirty_*_threads`).
    FromConfig,
    /// Zero threads, zero fds; a dirty dispatch is refused before any
    /// suspension side effect (spec §3.2).
    Disabled,
    /// An owned pool of `n` workers. `n == 0` disables the pool, matching the
    /// `Some(0)` config-knob semantics (spec §6).
    Owned(usize),
    /// A pool constructed OUTSIDE any scheduler and injected here — used but
    /// NEVER shut down by this scheduler (the embedder owns the join, spec
    /// §2.1). Safe because dirty completion routes purely by the oneshot the
    /// submission carries, not by any per-scheduler table (spec §5 / commit-5
    /// determination), so two schedulers can share one pool now.
    Shared(Arc<DirtyPool>),
}

/// Composition choice for the readiness service (spec §3.9).
#[cfg(feature = "readiness")]
#[derive(Clone)]
pub(super) enum ReadinessChoice {
    FromConfig,
    Disabled,
    Owned,
    Shared(SharedReadiness),
}

/// Composition choice for the file-IO ring (spec §2.2/§3.3).
#[derive(Clone)]
pub(super) enum FileRingChoice {
    /// Defer to the legacy default (live ⇒ Owned, replay ⇒ Disabled).
    FromConfig,
    /// No file facility; a file submit is refused before any suspension.
    Disabled,
    /// An owned file ring with the service-distinct thread name.
    Owned,
    /// An injected shared ring — REFUSED by `with_services` this release
    /// (routing gate is commit 6, spec §3.9).
    Shared(SharedIoRing),
}

/// Composition choice for the standard-IO ring + group-leader server (spec
/// §2.2/§3.4). No `Shared` arm: a shared standard ring would mean two process-0
/// servers draining one ring and discarding each other's completions
/// (`io/standard_io.rs`), so it is out of the model (spec §3.4, ring_service).
#[derive(Clone)]
pub(super) enum StandardRingChoice {
    /// Defer to the legacy default (live ⇒ Owned + process 0, replay ⇒
    /// Disabled).
    FromConfig,
    /// No ring, no process 0 (spec §3.4).
    Disabled,
    /// An owned standard ring with process 0 registered.
    Owned,
}

/// Composition choice for the optional generic-IO ring + bridge (spec
/// §2.2/§3.5).
#[derive(Clone)]
pub(super) enum GenericRingChoice {
    /// Defer to `config.io` (`Some` ⇒ Owned, `None` ⇒ Disabled).
    FromConfig,
    /// No generic ring (byte-identical to the former `config.io: None`).
    Disabled,
    /// An owned generic ring built from this config, plus its per-scheduler
    /// registry and completion bridge.
    Owned(RingConfig),
    /// An injected shared ring — REFUSED by `with_services` this release
    /// (routing gate is commit 6, spec §3.9).
    Shared(SharedIoRing),
}

/// Composition choice for the distribution bundle (spec §2.2/§3.6). No `Shared`
/// arm: cross-scheduler distribution sharing needs identity/compat validation
/// that is out of scope v1 (spec §3.6, recorded as future work).
#[derive(Clone)]
pub(super) enum DistributionChoice {
    /// Defer to `config.distribution` (`Some` ⇒ Owned, `None` ⇒ Disabled).
    FromConfig,
    /// Neither runtime exists — honest absence (spec §3.6).
    Disabled,
    /// An owned bundle built from this config.
    Owned(DistributionConfig),
}

/// Opaque, injectable handle for a shared IO ring (spec §2.2/§3.3/§3.5).
///
/// This is the *type* the [`SchedulerServices`] builder accepts for a shared
/// file or generic ring so the composition surface is complete, but
/// `with_services` REFUSES it this release with
/// [`WithServicesError::SharedRingRoutingDeferred`]: a shared ring's completions
/// route through per-`SharedState` registries and a per-scheduler poll loop, so
/// a second scheduler's completions would be swallowed by the ring owner until
/// the §3.9 routing gate lands with its mechanism in commit 6 (spec §3.3 / the
/// commit-3 deviation-2 tension, confirmed against the completion paths). The
/// handle deliberately carries no live ring — construction of a real shareable
/// ring is part of the commit-6 mechanism — only the intent to share and the
/// service it targets, so requesting one spawns nothing.
#[derive(Clone)]
pub struct SharedIoRing {
    service: &'static str,
    config: RingConfig,
}

impl std::fmt::Debug for SharedIoRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedIoRing")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl SharedIoRing {
    /// A shared file-IO ring handle (refused by `with_services` this release).
    #[must_use]
    pub fn file(config: RingConfig) -> Self {
        Self {
            service: super::inventory::FILE_IO_RING,
            config,
        }
    }

    /// A shared generic-IO ring handle (refused by `with_services` this
    /// release).
    #[must_use]
    pub fn generic(config: RingConfig) -> Self {
        Self {
            service: super::inventory::GENERIC_IO_RING,
            config,
        }
    }

    /// The inventory label of the ring this handle targets.
    pub(super) fn service(&self) -> &'static str {
        self.service
    }

    /// The ring configuration this handle was built with. Retained for the
    /// commit-6 routing mechanism, which will build the live shared ring from
    /// it; today `with_services` refuses the handle before it is read.
    #[must_use]
    pub fn config(&self) -> RingConfig {
        self.config
    }
}

/// Typed construction error for [`Scheduler::with_services`](super::Scheduler::with_services).
///
/// Distinct from the `String` returned for genuine spawn/OS failures: a
/// composition request that names a capability this release cannot deliver
/// SAFELY is refused loudly and by name, rather than silently degraded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WithServicesError {
    /// A shared file/generic IO ring was injected, but cross-scheduler
    /// completion routing does not ship until the §3.9 routing gate lands with
    /// its mechanism in **commit 6** (spec §3.9). `service` is the inventory
    /// label of the refused ring. Injecting it now would let the ring owner
    /// swallow the second scheduler's completions.
    SharedRingRoutingDeferred {
        /// Inventory label of the refused ring (e.g. `"file-io-ring"`).
        service: &'static str,
    },
    /// A [`SharedIoRing`] handle built for one ring service was passed to a
    /// DIFFERENT service's builder slot (e.g. `SharedIoRing::generic(..)` to
    /// [`SchedulerServices::shared_file_io`]). Reported distinctly so the
    /// diagnostic names the actual mistake instead of misattributing the
    /// deferred-routing refusal to the wrong service.
    SharedRingKindMismatch {
        /// The builder slot the handle was passed to.
        slot: &'static str,
        /// The service the handle itself was built for.
        handle: &'static str,
    },
}

impl std::fmt::Display for WithServicesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SharedRingRoutingDeferred { service } => write!(
                f,
                "shared {service} injection is refused this release: cross-scheduler \
                 completion routing lands with the §3.9 routing gate in commit 6"
            ),
            Self::SharedRingKindMismatch { slot, handle } => write!(
                f,
                "a shared-ring handle built for {handle} was passed to the {slot} \
                 slot; build the handle with the matching SharedIoRing constructor"
            ),
        }
    }
}

impl std::error::Error for WithServicesError {}

/// The ancillary-service composition request handed to
/// [`Scheduler::with_services`](super::Scheduler::with_services) (spec §2.2).
///
/// Construct one from a profile ([`full_runtime`](Self::full_runtime),
/// [`minimal`](Self::minimal), [`from_config`](Self::from_config)) and adjust
/// individual services with the builder methods. See this module's documentation
/// for the precedence rule and profile semantics.
#[derive(Clone)]
pub struct SchedulerServices {
    pub(super) dirty_cpu: DirtyChoice,
    pub(super) dirty_io: DirtyChoice,
    pub(super) file_io: FileRingChoice,
    pub(super) standard_io: StandardRingChoice,
    pub(super) generic_io: GenericRingChoice,
    pub(super) distribution: DistributionChoice,
    #[cfg(feature = "readiness")]
    pub(super) readiness: ReadinessChoice,
}

impl SchedulerServices {
    /// The legacy profile (spec §2.2/§6): every ancillary service `FromConfig`,
    /// so the resolved scheduler is byte-identical to
    /// [`Scheduler::new`](super::Scheduler::new). This is what `Scheduler::new`
    /// itself maps to, preserving today's per-knob defaults for one release.
    #[must_use]
    pub fn from_config() -> Self {
        Self {
            dirty_cpu: DirtyChoice::FromConfig,
            dirty_io: DirtyChoice::FromConfig,
            file_io: FileRingChoice::FromConfig,
            standard_io: StandardRingChoice::FromConfig,
            generic_io: GenericRingChoice::FromConfig,
            distribution: DistributionChoice::FromConfig,
            #[cfg(feature = "readiness")]
            readiness: ReadinessChoice::FromConfig,
        }
    }

    /// The full standalone-VM profile (spec §2.2/§3.6): every ancillary service
    /// `FromConfig` (so `config`'s dirty/IO knobs still apply) PLUS distribution
    /// explicitly turned on with a default config. Distribution is the one
    /// service that became honest-absent by default in commit 4, so opting back
    /// into it is exactly what "today's full behavior, explicitly requested"
    /// means — this is what the CLI uses.
    #[must_use]
    pub fn full_runtime() -> Self {
        Self {
            distribution: DistributionChoice::Owned(DistributionConfig::default()),
            #[cfg(feature = "readiness")]
            readiness: ReadinessChoice::Owned,
            ..Self::from_config()
        }
    }

    /// The minimal profile (spec §2.2/§5 assertion 1): every ancillary service
    /// `Disabled`. No dirty pools, no file/standard/generic ring, no process 0,
    /// no distribution runtime — only the requested normal workers run. Every
    /// disabled service refuses its use with a typed error (spec §3.2–§3.6).
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            dirty_cpu: DirtyChoice::Disabled,
            dirty_io: DirtyChoice::Disabled,
            file_io: FileRingChoice::Disabled,
            standard_io: StandardRingChoice::Disabled,
            generic_io: GenericRingChoice::Disabled,
            distribution: DistributionChoice::Disabled,
            #[cfg(feature = "readiness")]
            readiness: ReadinessChoice::Disabled,
        }
    }

    // ── Dirty CPU pool ──────────────────────────────────────────────────────

    /// Disable the dirty CPU pool (zero threads; dirty CPU calls refused).
    #[must_use]
    pub fn disable_dirty_cpu(mut self) -> Self {
        self.dirty_cpu = DirtyChoice::Disabled;
        self
    }

    /// Own a dirty CPU pool of `workers` threads (`0` disables it, spec §6).
    #[must_use]
    pub fn owned_dirty_cpu(mut self, workers: usize) -> Self {
        self.dirty_cpu = DirtyChoice::Owned(workers);
        self
    }

    /// Inject a shared dirty CPU pool the embedder owns and joins (spec §2.1).
    /// Safe now: dirty completion routes by the oneshot the submission carries.
    #[must_use]
    pub fn shared_dirty_cpu(mut self, pool: Arc<DirtyPool>) -> Self {
        self.dirty_cpu = DirtyChoice::Shared(pool);
        self
    }

    // ── Dirty IO pool ───────────────────────────────────────────────────────

    /// Disable the dirty IO pool (zero threads; dirty IO calls refused).
    #[must_use]
    pub fn disable_dirty_io(mut self) -> Self {
        self.dirty_io = DirtyChoice::Disabled;
        self
    }

    /// Own a dirty IO pool of `workers` threads (`0` disables it, spec §6).
    #[must_use]
    pub fn owned_dirty_io(mut self, workers: usize) -> Self {
        self.dirty_io = DirtyChoice::Owned(workers);
        self
    }

    /// Inject a shared dirty IO pool the embedder owns and joins (spec §2.1).
    #[must_use]
    pub fn shared_dirty_io(mut self, pool: Arc<DirtyPool>) -> Self {
        self.dirty_io = DirtyChoice::Shared(pool);
        self
    }

    // ── File-IO ring ────────────────────────────────────────────────────────

    /// Disable the file-IO ring (file submits refused before any suspension).
    #[must_use]
    pub fn disable_file_io(mut self) -> Self {
        self.file_io = FileRingChoice::Disabled;
        self
    }

    /// Own a file-IO ring.
    #[must_use]
    pub fn owned_file_io(mut self) -> Self {
        self.file_io = FileRingChoice::Owned;
        self
    }

    /// Inject a shared file-IO ring — REFUSED by `with_services` this release
    /// (routing gate is commit 6, [`WithServicesError::SharedRingRoutingDeferred`]).
    #[must_use]
    pub fn shared_file_io(mut self, ring: SharedIoRing) -> Self {
        self.file_io = FileRingChoice::Shared(ring);
        self
    }

    // ── Standard-IO ring ────────────────────────────────────────────────────

    /// Disable the standard-IO ring: no ring, no process 0 (spec §3.4).
    #[must_use]
    pub fn disable_standard_io(mut self) -> Self {
        self.standard_io = StandardRingChoice::Disabled;
        self
    }

    /// Own a standard-IO ring with process 0 registered.
    #[must_use]
    pub fn owned_standard_io(mut self) -> Self {
        self.standard_io = StandardRingChoice::Owned;
        self
    }

    // ── Generic-IO ring ─────────────────────────────────────────────────────

    /// Disable the generic-IO ring.
    #[must_use]
    pub fn disable_generic_io(mut self) -> Self {
        self.generic_io = GenericRingChoice::Disabled;
        self
    }

    /// Own a generic-IO ring built from `config`, plus its bridge.
    #[must_use]
    pub fn owned_generic_io(mut self, config: RingConfig) -> Self {
        self.generic_io = GenericRingChoice::Owned(config);
        self
    }

    /// Inject a shared generic-IO ring — REFUSED by `with_services` this release
    /// (routing gate is commit 6, [`WithServicesError::SharedRingRoutingDeferred`]).
    #[must_use]
    pub fn shared_generic_io(mut self, ring: SharedIoRing) -> Self {
        self.generic_io = GenericRingChoice::Shared(ring);
        self
    }

    // ── Distribution ────────────────────────────────────────────────────────

    /// Disable distribution: neither runtime exists (honest absence, spec §3.6).
    #[must_use]
    pub fn disable_distribution(mut self) -> Self {
        self.distribution = DistributionChoice::Disabled;
        self
    }

    /// Own a distribution bundle built from `config`.
    #[must_use]
    pub fn owned_distribution(mut self, config: DistributionConfig) -> Self {
        self.distribution = DistributionChoice::Owned(config);
        self
    }

    // ── Readiness ────────────────────────────────────────────────────────────

    #[cfg(feature = "readiness")]
    #[must_use]
    pub fn disable_readiness(mut self) -> Self {
        self.readiness = ReadinessChoice::Disabled;
        self
    }

    #[cfg(feature = "readiness")]
    #[must_use]
    pub fn owned_readiness(mut self) -> Self {
        self.readiness = ReadinessChoice::Owned;
        self
    }

    #[cfg(feature = "readiness")]
    #[must_use]
    pub fn shared_readiness(mut self, service: SharedReadiness) -> Self {
        self.readiness = ReadinessChoice::Shared(service);
        self
    }

    /// Validate the composition BEFORE any service is built (spec §3.9): a
    /// shared file/generic ring is refused this release because cross-scheduler
    /// completion routing lands with the §3.9 gate in commit 6. Called by
    /// [`Scheduler::with_services`](super::Scheduler::with_services); public so
    /// the refusal is testable as a typed value ahead of construction.
    pub fn validate(&self) -> Result<(), WithServicesError> {
        if let FileRingChoice::Shared(ring) = &self.file_io {
            // A crossed handle is its own mistake, named as such; the deferred
            // error is derived from the SLOT, never the unchecked handle label.
            if ring.service() != super::inventory::FILE_IO_RING {
                return Err(WithServicesError::SharedRingKindMismatch {
                    slot: super::inventory::FILE_IO_RING,
                    handle: ring.service(),
                });
            }
            return Err(WithServicesError::SharedRingRoutingDeferred {
                service: super::inventory::FILE_IO_RING,
            });
        }
        if let GenericRingChoice::Shared(ring) = &self.generic_io {
            if ring.service() != super::inventory::GENERIC_IO_RING {
                return Err(WithServicesError::SharedRingKindMismatch {
                    slot: super::inventory::GENERIC_IO_RING,
                    handle: ring.service(),
                });
            }
            return Err(WithServicesError::SharedRingRoutingDeferred {
                service: super::inventory::GENERIC_IO_RING,
            });
        }
        Ok(())
    }
}

impl Default for SchedulerServices {
    /// The legacy profile ([`from_config`](Self::from_config)).
    fn default() -> Self {
        Self::from_config()
    }
}
