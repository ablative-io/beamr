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
use super::service::{ServiceInstanceId, ServiceModeLabel};

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
pub(super) struct ServiceInstances {
    pub(super) dirty_cpu: ServiceInstanceId,
    pub(super) dirty_io: ServiceInstanceId,
    pub(super) file_io_ring: ServiceInstanceId,
    pub(super) standard_io_ring: ServiceInstanceId,
    pub(super) generic_io_ring: ServiceInstanceId,
    pub(super) dist_sender: ServiceInstanceId,
    pub(super) net_kernel: ServiceInstanceId,
    /// Whether the generic-IO completion bridge was part of the construction
    /// request (ring present, not replay). Captured here — NOT read from the
    /// live `Option<IoCompletionBridge>` — so the entry's `configured` count
    /// stays the construction request after shutdown stops the bridge.
    pub(super) generic_bridge_requested: bool,
}

impl ServiceInstances {
    /// Mint identities for every service slot. `generic_io_present`/
    /// `dist_sender_present` gate the two slots that are legitimately absent
    /// today (generic IO off by default; the sender skipped under replay): an
    /// absent slot gets the [`DISABLED`](ServiceInstanceId::DISABLED) sentinel,
    /// never a live identity.
    pub(super) fn mint(
        generic_io_present: bool,
        generic_bridge_requested: bool,
        dist_sender_present: bool,
    ) -> Self {
        Self {
            dirty_cpu: ServiceInstanceId::mint(),
            dirty_io: ServiceInstanceId::mint(),
            file_io_ring: ServiceInstanceId::mint(),
            standard_io_ring: ServiceInstanceId::mint(),
            generic_io_ring: presence_id(generic_io_present),
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

    entries.push(owned_entry(
        DIRTY_CPU,
        instances.dirty_cpu,
        shared.dirty_cpu.requested_threads(),
        shared.dirty_cpu.live_worker_names(),
    ));
    entries.push(owned_entry(
        DIRTY_IO,
        instances.dirty_io,
        shared.dirty_io.requested_threads(),
        shared.dirty_io.live_worker_names(),
    ));
    entries.push(owned_entry(
        FILE_IO_RING,
        instances.file_io_ring,
        shared.file_io_ring.requested_worker_count(),
        shared.file_io_ring.worker_thread_names(),
    ));
    entries.push(owned_entry(
        STANDARD_IO_RING,
        instances.standard_io_ring,
        shared._standard_io_server.requested_worker_count(),
        shared._standard_io_server.worker_thread_names(),
    ));

    // Generic IO ring + its completion bridge, folded into one line (spec §1
    // row 6). Off by default: `config.io: None` is a true absence, reported
    // Disabled.
    match &shared.io_ring {
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
            entries.push(owned_entry(
                GENERIC_IO_RING,
                instances.generic_io_ring,
                configured,
                names,
            ));
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
    vec![ServicePolicyLine {
        policy: DIRTY_COMPLETE,
        // The dirty pools are Owned today, so the burst thread can spawn; a
        // disabled pool refuses before it (commit 2, spec §3.2).
        mode: ServiceModeLabel::Owned,
        spawned_total: shared
            .dirty_completion_spawns
            .load(std::sync::atomic::Ordering::Relaxed),
    }]
}
