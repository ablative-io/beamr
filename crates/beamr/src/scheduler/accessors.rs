//! Handles on what the scheduler owns: namespaces, the hook, the timer
//! wheel, the atom table, the output sink, and the service inventory and
//! policies.

use super::inventory::{ServiceInventoryEntry, ServicePolicyLine};
use super::{Scheduler, inventory, lock_or_recover};
use crate::atom::AtomTable;
use crate::hook::Hook;
use crate::io::IoSink;
use crate::module::ModuleRegistry;
use crate::namespace::NamespaceId;
use crate::timer::TimerWheel;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

impl Scheduler {
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

    #[must_use]
    pub fn hook(&self) -> &Hook {
        &self.shared.hook
    }
    #[must_use]
    pub fn timers(&self) -> &Arc<Mutex<TimerWheel>> {
        &self.shared.timers
    }

    /// The scheduler's shared atom table.
    ///
    /// Distribution-facing embedders need this to intern names into the SAME
    /// atoms the scheduler uses internally: pg group/scope atoms and the node
    /// atoms returned by [`ConnectionManager::connected_nodes`](crate::distribution::connection::ConnectionManager::connected_nodes) are indices into
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
}
