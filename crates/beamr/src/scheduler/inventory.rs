//! Service inventory — the VM's own honest thread bill (spec §5).
//!
//! [`Scheduler::service_inventory`](super::Scheduler::service_inventory) reports
//! one entry per ancillary service, with its live OS thread names and counts,
//! so lens Q3 is a mechanical comparison against the OS thread probe rather than
//! an eyeballed guess (spec §9). Commit 1 reports the services exactly as they
//! are built today — all eager, all `Owned` — pinning the as-built budget; the
//! later commits (spec §11) flip individual services to `Disabled`/`Shared` and
//! the same assertions catch any drift.

use super::SharedState;
use super::dirty::DirtyPool;
use super::distribution_service::DistributionService;
use super::ring_service::{RingService, StandardIoService};
use super::service::{ServiceInstanceId, ServiceMode, ServiceModeLabel};

/// Service label: the dirty CPU pool.
pub(super) const DIRTY_CPU: &str = "dirty-cpu";
/// Service label: the dirty IO pool.
pub(super) const DIRTY_IO: &str = "dirty-io";
/// Service label: the file-IO completion ring.
pub(super) const FILE_IO_RING: &str = "file-io-ring";
/// Service label: the standard-IO completion ring (backs process 0).
pub(super) const STANDARD_IO_RING: &str = "standard-io-ring";
/// Service label: the optional generic-IO ring plus its completion bridge.
pub(super) const GENERIC_IO_RING: &str = "generic-io-ring";
/// Service label: the distribution bundle — one coherent service (spec §3.6),
/// whose §5 line lists BOTH runtime worker names (the outbound sender's and the
/// net-kernel's) under a single entry.
pub(super) const DISTRIBUTION: &str = "distribution";
/// Policy label: the per-dirty-call transient completion thread.
pub(super) const DIRTY_COMPLETE: &str = "dirty-complete";
/// Policy label: the per-connection distribution heartbeat (net-tick) task —
/// an async task with no OS thread, inventoried as a task-class counter (§3.7).
pub(super) const HEARTBEAT: &str = "heartbeat";

/// One ancillary service's line in the thread inventory (spec §5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceInventoryEntry {
    /// Stable service label, e.g. `"dirty-cpu"`, `"file-io-ring"`.
    pub service: &'static str,
    /// Whether the service is disabled, owned here, or a shared injection.
    pub mode: ServiceModeLabel,
    /// Process-wide identity of the underlying instance. Two entries with equal
    /// `instance` (from any schedulers) ARE the same service, which is what
    /// makes the Shared dedup mechanical.
    /// [`DISABLED`](ServiceInstanceId::DISABLED) for a disabled slot.
    pub instance: ServiceInstanceId,
    /// Worker threads the service was ASKED to run (post-coercion, e.g. the
    /// dirty pools' `.max(1)`). Independent of spawn success, so a partial
    /// spawn shows as `actual < configured` — capacity loss is visible, not
    /// silently renormalized.
    pub configured: usize,
    /// OS threads the service holds LIVE right now, as attributed by its
    /// spawn/join records: a joined pool reports zero. Services that today's
    /// `shutdown()` does not stop — the standard-IO ring and both
    /// distribution runtimes, the §1 as-built leaks — truthfully keep
    /// reporting their still-live threads until the §4 teardown rewrite
    /// (commits 3–5, spec §11).
    pub actual: usize,
    /// Exact OS thread names, in spawn order. Not yet service-distinct: rings
    /// 4/5/6 still collide on `beamr-io-thread-pool-*` (renaming is commit 3,
    /// spec §5).
    pub thread_names: Vec<String>,
    /// Persistent fd classes held by the service, e.g. `"io_uring"`,
    /// `"listener"`. Empty in commit 1 on macOS (no cheap fd probe); populated
    /// alongside the Linux fd probe in a later commit.
    pub fd_classes: Vec<&'static str>,
}

/// A transient thread *policy*, reported with a spawn counter rather than as a
/// thread line (spec §5): the class spawns and joins OS threads in bursts, so a
/// point-in-time thread count would under-report it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServicePolicyLine {
    /// Stable policy label, e.g. `"dirty-complete"`.
    pub policy: &'static str,
    /// The owning service's mode. `Owned` today; the disabled dirty pool
    /// refuses before this thread ever spawns (commit 2, spec §3.2).
    pub mode: ServiceModeLabel,
    /// Transient execution units this policy has spawned since construction.
    /// The unit is class-specific and named on the policy label: OS threads for
    /// a thread-class policy (`"dirty-complete"`), async tasks — no OS thread —
    /// for a task-class policy (`"heartbeat"`, spec §3.7).
    pub spawned_total: u64,
}

/// Construction-request facts held so `service_inventory()` reports a *stable*
/// `configured` count across calls, even after shutdown mutates live state.
///
/// The dirty pools, the three IO rings, and the distribution bundle are NOT
/// identified here: each mints and carries its own [`ServiceInstanceId`] through
/// its [`ServiceMode`], so a Disabled slot reports the
/// [`DISABLED`](ServiceInstanceId::DISABLED) sentinel and a Shared instance
/// propagates the owner's id — rather than a live id minted here for a service
/// that was never built on this scheduler (spec §3.2–§3.6/§5).
pub(super) struct ServiceInstances {
    /// Whether the generic-IO completion bridge was part of the construction
    /// request (ring present, not replay). Captured here — NOT read from the
    /// live `Option<IoCompletionBridge>` — so the entry's `configured` count
    /// stays the construction request after shutdown stops the bridge.
    pub(super) generic_bridge_requested: bool,
}

impl ServiceInstances {
    /// Record the construction request. `generic_bridge_requested` captures
    /// whether the generic ring's completion bridge was built.
    pub(super) fn mint(generic_bridge_requested: bool) -> Self {
        Self {
            generic_bridge_requested,
        }
    }
}

/// A dirty pool's line, reading through its [`ServiceMode`]. An `Owned` pool
/// reports its requested-vs-live split and its own instance id; a `Disabled`
/// slot reports the §5 disabled entry (DISABLED sentinel, zero threads, zero
/// configured — it was explicitly requested off, spec §3.2). Dirty pools are
/// never `Shared` this commit; a future `Shared` arm would carry the same
/// borrowed pool and its propagated id.
fn dirty_pool_entry(service: &'static str, mode: &ServiceMode<DirtyPool>) -> ServiceInventoryEntry {
    match mode.service() {
        Some(pool) => {
            let thread_names = pool.live_worker_names();
            ServiceInventoryEntry {
                service,
                mode: mode.label(),
                instance: mode.instance_id(),
                configured: pool.requested_threads(),
                actual: thread_names.len(),
                thread_names,
                fd_classes: Vec::new(),
            }
        }
        None => disabled_entry(service),
    }
}

/// A completion ring's line, read through its [`ServiceMode`] (spec §3.3/§3.5).
/// An `Owned` or `Shared` ring reports its requested-vs-live thread split and
/// the instance id it carries — a `Shared` ring propagates the owner's id, so
/// two schedulers reporting the same shared ring dedup in the process-wide
/// aggregate. A `Disabled` slot reports the §5 disabled entry.
fn ring_entry(service: &'static str, mode: &ServiceMode<RingService>) -> ServiceInventoryEntry {
    match mode.service() {
        Some(ring) => {
            let thread_names = ring.worker_thread_names();
            ServiceInventoryEntry {
                service,
                mode: mode.label(),
                instance: mode.instance_id(),
                configured: ring.requested_worker_count(),
                actual: thread_names.len(),
                thread_names,
                fd_classes: Vec::new(),
            }
        }
        None => disabled_entry(service),
    }
}

/// The standard-IO ring's line, read through its [`ServiceMode`] (spec §3.4).
/// `Owned` reports the group-leader server's ring worker split; `Disabled` (no
/// ring, no process 0) reports the §5 disabled entry.
fn standard_io_entry(
    service: &'static str,
    mode: &ServiceMode<StandardIoService>,
) -> ServiceInventoryEntry {
    match mode.service() {
        Some(standard) => {
            let thread_names = standard.server().worker_thread_names();
            ServiceInventoryEntry {
                service,
                mode: mode.label(),
                instance: mode.instance_id(),
                configured: standard.server().requested_worker_count(),
                actual: thread_names.len(),
                thread_names,
                fd_classes: Vec::new(),
            }
        }
        None => disabled_entry(service),
    }
}

/// The distribution bundle's line, read through its [`ServiceMode`] (spec §3.6).
/// An `Owned` bundle reports BOTH runtime workers in one entry — the sender's
/// "beamr-dist-send" worker and the net-kernel's "beamr-net-kernel" worker —
/// with `configured` the construction request (stable across shutdown) and
/// `actual` the live count (zero after the §4 join). A `Disabled` slot (honest
/// `distribution: None`) reports the §5 disabled entry: no instance, no threads.
fn distribution_entry(mode: &ServiceMode<DistributionService>) -> ServiceInventoryEntry {
    match mode.service() {
        Some(dist) => {
            let thread_names = dist.runtime_thread_names();
            ServiceInventoryEntry {
                service: DISTRIBUTION,
                mode: mode.label(),
                instance: mode.instance_id(),
                configured: dist.configured_runtimes(),
                actual: thread_names.len(),
                thread_names,
                fd_classes: Vec::new(),
            }
        }
        None => disabled_entry(DISTRIBUTION),
    }
}

/// A disabled service line: no instance, no threads, no fds.
fn disabled_entry(service: &'static str) -> ServiceInventoryEntry {
    ServiceInventoryEntry {
        service,
        mode: ServiceModeLabel::Disabled,
        instance: ServiceInstanceId::DISABLED,
        configured: 0,
        actual: 0,
        thread_names: Vec::new(),
        fd_classes: Vec::new(),
    }
}

/// Build the thread inventory for one scheduler (spec §5).
///
/// Reads every ancillary service's live thread names directly from the service,
/// so the report is what the process actually holds, not what a config claims.
pub(super) fn build_service_inventory(shared: &SharedState) -> Vec<ServiceInventoryEntry> {
    let instances = &shared.service_instances;
    let mut entries = Vec::with_capacity(6);

    entries.push(dirty_pool_entry(DIRTY_CPU, &shared.dirty_cpu));
    entries.push(dirty_pool_entry(DIRTY_IO, &shared.dirty_io));
    entries.push(ring_entry(FILE_IO_RING, &shared.file_io_ring));
    entries.push(standard_io_entry(STANDARD_IO_RING, &shared.standard_io));

    // Generic IO ring + its completion bridge, folded into one line (spec §1
    // row 6). Off by default: `config.io: None` is a true absence (Disabled).
    match shared.io_ring.service() {
        Some(ring) => {
            let mut names = ring.worker_thread_names();
            // `configured` counts the bridge from the CONSTRUCTION request
            // (stable across shutdown); `actual`/names read the live Option,
            // which shutdown takes — so post-shutdown the bridge drops from
            // `actual` but never from `configured`.
            let configured =
                ring.requested_worker_count() + usize::from(instances.generic_bridge_requested);
            if super::lock_or_recover(&shared.io_bridge).is_some() {
                names.push(crate::io::bridge::IO_COMPLETION_THREAD_NAME.to_owned());
            }
            entries.push(ServiceInventoryEntry {
                service: GENERIC_IO_RING,
                mode: shared.io_ring.label(),
                instance: shared.io_ring.instance_id(),
                configured,
                actual: names.len(),
                thread_names: names,
                fd_classes: Vec::new(),
            });
        }
        None => entries.push(disabled_entry(GENERIC_IO_RING)),
    }

    // Distribution bundle — one coherent §5 line whose `thread_names` lists BOTH
    // runtime workers (spec §3.6): `Owned` when `config.distribution` was `Some`,
    // `Disabled` (honest absence, NEITHER runtime built) when it was `None`.
    entries.push(distribution_entry(&shared.distribution));

    entries
}

/// Process-wide thread aggregate over inventory entries from ANY number of
/// co-resident schedulers (spec §9 Q2): each `Owned` entry counts once, each
/// distinct `Shared` instance counts ONCE regardless of how many schedulers
/// report it, and `Disabled` slots contribute nothing. This is the enforcement
/// form of the signed Q2 bound — a shared 4-thread ring serving N schedulers
/// adds 4, never 4N.
#[must_use]
pub fn deduped_thread_aggregate(entries: &[ServiceInventoryEntry]) -> usize {
    let mut counted = std::collections::BTreeSet::new();
    let mut aggregate = 0;
    for entry in entries {
        match entry.mode {
            ServiceModeLabel::Disabled => {}
            // Owned instances are unique by construction, but dedup them the
            // same way so double-listing one scheduler's inventory cannot
            // double-bill it either.
            ServiceModeLabel::Owned | ServiceModeLabel::Shared => {
                if counted.insert(entry.instance) {
                    aggregate += entry.actual;
                }
            }
        }
    }
    aggregate
}

/// Build the transient-thread policy lines for one scheduler (spec §5).
pub(super) fn build_service_policies(shared: &SharedState) -> Vec<ServicePolicyLine> {
    // The dirty-complete burst thread spawns only when a dirty call is
    // submitted, so its mode follows the dirty pools (spec §3.2/§5): Owned
    // while any dirty pool can accept work, Disabled when both pools are off
    // and the completion thread can therefore never spawn.
    let dirty_complete_mode =
        if shared.dirty_cpu.service().is_some() || shared.dirty_io.service().is_some() {
            ServiceModeLabel::Owned
        } else {
            ServiceModeLabel::Disabled
        };
    // The distribution heartbeat (net-tick) is an async task per connection with
    // NO OS thread (spec §3.7), so it is a task-class policy line, never a thread
    // line: `Owned` while an owned bundle has the net-tick enabled, `Disabled`
    // otherwise (distribution absent, or the net-tick off). `spawned_total`
    // counts heartbeat tasks spawned to date — zero at rest, one per link.
    let (heartbeat_mode, heartbeat_spawned) = match shared.distribution.service() {
        Some(dist) if dist.heartbeat_enabled() => {
            (shared.distribution.label(), dist.heartbeat_tasks_spawned())
        }
        _ => (ServiceModeLabel::Disabled, 0),
    };
    vec![
        ServicePolicyLine {
            policy: DIRTY_COMPLETE,
            mode: dirty_complete_mode,
            spawned_total: shared
                .dirty_completion_spawns
                .load(std::sync::atomic::Ordering::Relaxed),
        },
        ServicePolicyLine {
            policy: HEARTBEAT,
            mode: heartbeat_mode,
            spawned_total: heartbeat_spawned,
        },
    ]
}
