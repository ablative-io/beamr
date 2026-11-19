//! Placing a spawned child: its pid, what it inherits from its caller, and
//! inserting it with any link or monitor before it is woken.

use crate::namespace::NamespaceId;
use crate::native::CapabilitySet;
use crate::native::spawn::{SpawnMonitorResult, SpawnOptions, SpawnOptionsResult};
use crate::process::Process;
use crate::scheduler::spawning::SpawnRequest;
use crate::scheduler::{ProcessSlot, ScheduledProcess, SharedState, lock_or_recover};
use crate::term::Term;

use super::links::add_link_to_slot;
use super::spawn::SchedulerSpawnFacility;

impl SchedulerSpawnFacility {
    pub(super) fn next_child_pid(&self) -> u64 {
        let child_pid = self
            .shared
            .next_pid
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.shared.process_table.spawn_with_pid(child_pid);
        child_pid
    }

    pub(super) fn insert_options_child(
        &self,
        caller_pid: u64,
        request: SpawnRequest,
        options: SpawnOptions,
    ) -> SpawnOptionsResult {
        let child_pid = request.pid;
        #[cfg(feature = "telemetry")]
        let parent_pid = request.parent_pid;
        #[cfg(feature = "telemetry")]
        let module = request.module;
        #[cfg(feature = "telemetry")]
        let function = request.function;
        #[cfg(feature = "telemetry")]
        let arity = request.arity;
        let mut child = crate::scheduler::spawning::build_process(request);
        if options.link {
            let child_linked = child.add_link(caller_pid);
            let caller_linked = add_link_to_slot(&self.shared, caller_pid, child_pid);
            if child_linked && caller_linked {
                #[cfg(feature = "telemetry")]
                crate::telemetry::lifecycle::record_process_linked(caller_pid, child_pid);
            }
        }
        if options.monitor {
            let result = self.register_monitor_insert_and_wake(caller_pid, child_pid, child);
            #[cfg(feature = "telemetry")]
            crate::telemetry::lifecycle::record_process_spawned(
                &self.shared.atom_table,
                child_pid,
                parent_pid,
                module,
                function,
                arity,
            );
            SpawnOptionsResult {
                pid: result.pid,
                reference: Some(result.reference),
            }
        } else {
            self.insert_and_wake(child_pid, child);
            #[cfg(feature = "telemetry")]
            crate::telemetry::lifecycle::record_process_spawned(
                &self.shared.atom_table,
                child_pid,
                parent_pid,
                module,
                function,
                arity,
            );
            SpawnOptionsResult {
                pid: child_pid,
                reference: None,
            }
        }
    }

    pub(super) fn register_monitor_insert_and_wake(
        &self,
        caller_pid: u64,
        child_pid: u64,
        mut child: crate::process::Process,
    ) -> SpawnMonitorResult {
        let reference = {
            let mut ms = lock_or_recover(&self.shared.monitor_set);
            let reference = ms.allocate_reference_pub();
            let mon = crate::process::Monitor::new(reference, caller_pid, child_pid);
            ms.register_monitor(reference, mon, child_pid);
            child.add_monitor(mon);
            drop(ms);
            add_monitor_to_slot(&self.shared, caller_pid, mon);
            reference
        };

        #[cfg(feature = "telemetry")]
        crate::telemetry::lifecycle::record_process_monitored(caller_pid, child_pid, reference);

        self.insert_and_wake(child_pid, child);

        SpawnMonitorResult {
            pid: child_pid,
            reference,
        }
    }

    pub(super) fn insert_and_wake(&self, child_pid: u64, child: Process) {
        self.shared.process_bodies.insert(
            child_pid,
            std::sync::Mutex::new(ProcessSlot::Present(ScheduledProcess(child))),
        );
        self.shared
            .spawn_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        {
            let mut ws = lock_or_recover(&self.shared.wait_set);
            ws.woken.push((child_pid, 0));
            // W9: advance inside the critical section, notify after it.
            self.shared.advance_wake_sequence(&ws);
        }
        self.shared.wake_condvar.notify_all();
    }

    pub(super) fn caller_namespace(&self, caller_pid: u64) -> NamespaceId {
        if let Some(parent_entry) = self.shared.process_bodies.get(&caller_pid) {
            let parent_slot = lock_or_recover(&parent_entry);
            match &*parent_slot {
                ProcessSlot::Present(ScheduledProcess(parent)) => return parent.namespace_id(),
                ProcessSlot::Executing(metadata) => return metadata.namespace_id,
                ProcessSlot::Absent => {}
            }
        }
        self.namespace_id
    }

    pub(super) fn caller_group_leader(&self, caller_pid: u64) -> Term {
        if let Some(parent_entry) = self.shared.process_bodies.get(&caller_pid) {
            let parent_slot = lock_or_recover(&parent_entry);
            match &*parent_slot {
                ProcessSlot::Present(ScheduledProcess(parent)) => return parent.group_leader(),
                ProcessSlot::Executing(metadata) => return metadata.group_leader,
                ProcessSlot::Absent => {}
            }
        }
        // No live caller: this is a synthetic/root spawn (e.g. the public
        // `spawn_native` uses caller pid 0). Seed it exactly like a top-level
        // bytecode spawn — process 0 when the standard ring is Owned, the
        // no-such-pid sentinel when Disabled (spec §3.4) — never the absent
        // caller's own pid, which would resurrect the self-leader hang shape.
        match Term::try_pid(self.shared.standard_io_pid) {
            Some(pid_term) => pid_term,
            None => Term::NIL,
        }
    }

    pub(super) fn caller_capabilities(&self, caller_pid: u64) -> CapabilitySet {
        if let Some(parent_entry) = self.shared.process_bodies.get(&caller_pid) {
            let parent_slot = lock_or_recover(&parent_entry);
            match &*parent_slot {
                ProcessSlot::Present(ScheduledProcess(parent)) => {
                    return parent.capabilities().clone();
                }
                ProcessSlot::Executing(metadata) => return metadata.capabilities.clone(),
                ProcessSlot::Absent => {}
            }
        }
        CapabilitySet::all()
    }
}

fn add_monitor_to_slot(shared: &SharedState, pid: u64, monitor: crate::process::Monitor) -> bool {
    let Some(entry) = shared.process_bodies.get(&pid) else {
        return false;
    };
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(ScheduledProcess(process)) => {
            process.add_monitor(monitor);
            true
        }
        ProcessSlot::Executing(metadata) => {
            metadata.add_monitor(monitor);
            true
        }
        ProcessSlot::Absent => false,
    }
}
