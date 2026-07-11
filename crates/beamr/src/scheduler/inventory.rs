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
/// Service label: the outbound distribution sender runtime.
pub(super) const DIST_SENDER: &str = "dist-sender";
/// Service label: the net-kernel runtime.
pub(super) const NET_KERNEL: &str = "net-kernel";
/// Policy label: the per-dirty-call transient completion thread.
pub(super) const DIRTY_COMPLETE: &str = "dirty-complete";

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
    /// Transient OS threads this policy has spawned since construction.
    pub spawned_total: u64,
}

/// Process-unique identities minted for each ancillary service at construction
/// (spec §5). Held so `service_inventory()` reports a *stable* identity across
/// calls, and so a service shared into another scheduler reports the same one.
///
/// The dirty pools and the three IO rings are NOT here: each mints and carries
/// its own [`ServiceInstanceId`] through its [`ServiceMode`], so a Disabled slot
/// reports the [`DISABLED`](ServiceInstanceId::DISABLED) sentinel and a Shared
/// ring propagates the owner's id — rather than a live id minted here for a
/// service that was never built on this scheduler (spec §3.2–§3.5/§5).
pub(super) struct ServiceInstances {
    pub(super) dist_sender: ServiceInstanceId,
    pub(super) net_kernel: ServiceInstanceId,
    /// Whether the generic-IO completion bridge was part of the construction
    /// request (ring present, not replay). Captured here — NOT read from the
    /// live `Option<IoCompletionBridge>` — so the entry's `configured` count
    /// stays the construction request after shutdown stops the bridge.
    pub(super) generic_bridge_requested: bool,
}

impl ServiceInstances {
    /// Mint identities for the still-eager distribution runtimes.
    /// `dist_sender_present` gates the sender (skipped under replay): an absent
    /// slot gets the [`DISABLED`](ServiceInstanceId::DISABLED) sentinel, never a
    /// live identity. `generic_bridge_requested` records whether the generic
    /// ring's completion bridge was built.
    pub(super) fn mint(dist_sender_present: bool, generic_bridge_requested: bool) -> Self {
        Self {
            dist_sender: presence_id(dist_sender_present),
            net_kernel: ServiceInstanceId::mint(),
            generic_bridge_requested,
        }
    }
}

fn presence_id(present: bool) -> ServiceInstanceId {
    if present {
        ServiceInstanceId::mint()
    } else {
        ServiceInstanceId::DISABLED
    }
}

/// An owned, thread-bearing service line. `configured` is the REQUESTED
/// count, passed explicitly by each arm; `actual` is the live thread names'
/// length — the two diverge on partial spawn or after the service is joined.
fn owned_entry(
    service: &'static str,
    instance: ServiceInstanceId,
    configured: usize,
    thread_names: Vec<String>,
) -> ServiceInventoryEntry {
    ServiceInventoryEntry {
        service,
        mode: ServiceModeLabel::Owned,
        instance,
        configured,
        actual: thread_names.len(),
        thread_names,
        fd_classes: Vec::new(),
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
    let mut entries = Vec::with_capacity(7);

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

    // Distribution runtimes. Both are built today even under `distribution:
    // None` (`unwrap_or_default()`), which is exactly what this pins; they turn
    // absent in commit 4 (spec §3.6). The sender's `None` arm covers replay
    // (deliberate absence) AND a runtime-build failure — the two are not
    // distinguishable here today; the refused-vs-failed split is commit 4
    // work (spec §3.6/Q-B) where distribution construction is rewritten.
    match &shared.dist_sender {
        Some(sender) => entries.push(owned_entry(
            DIST_SENDER,
            instances.dist_sender,
            1,
            sender.worker_thread_names(),
        )),
        None => entries.push(disabled_entry(DIST_SENDER)),
    }
    entries.push(owned_entry(
        NET_KERNEL,
        instances.net_kernel,
        1,
        shared.net_kernel.worker_thread_names(),
    ));

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
    vec![ServicePolicyLine {
        policy: DIRTY_COMPLETE,
        mode: dirty_complete_mode,
        spawned_total: shared
            .dirty_completion_spawns
            .load(std::sync::atomic::Ordering::Relaxed),
    }]
}
