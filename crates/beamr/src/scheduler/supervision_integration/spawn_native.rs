//! Spawning a native process, with its `trap_exit` flag set before it can
//! run.

#[cfg(feature = "telemetry")]
use crate::atom::Atom;
use crate::native::spawn::SpawnError;
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::process::{Priority, Process};
use crate::scheduler::{ProcessSlot, ScheduledProcess, lock_or_recover};

use super::links::add_link_to_slot;
use super::spawn::SchedulerSpawnFacility;

impl SchedulerSpawnFacility {
    /// Build and make-runnable a native process, setting its `trap_exit` flag
    /// BEFORE it is published to `process_bodies` (i.e. before its first slice
    /// can run). Shared by [`SpawnFacility::spawn_native`] (`trap_exit=false`,
    /// the historical spelling) and the flag-before-runnable entry points
    /// [`Scheduler::spawn_native_trap_exit`](crate::scheduler::Scheduler) and
    /// [`Scheduler::spawn_native_link_trap_exit`](crate::scheduler::Scheduler).
    ///
    /// This is the native mirror of bytecode `spawn_trap_exit`
    /// (`scheduler/spawning.rs`): the flag is set on the `Process` while it is
    /// still owned locally, so it is observably live from the process's birth —
    /// there is no window in which the process exists but is not yet trapping.
    pub(super) fn spawn_native_flagged(
        &self,
        caller_pid: u64,
        factory: crate::native::native_process::NativeHandlerFactory,
        link_to: Option<u64>,
        trap_exit: bool,
    ) -> Result<u64, SpawnError> {
        let _admission = self.admit_or_refuse()?;
        // Mirror `spawn`, but build a native `Process` (no bytecode setup, no
        // instruction pointer) carrying the handler the factory produces.
        let namespace_id = self.caller_namespace(caller_pid);
        let group_leader = self.caller_group_leader(caller_pid);
        let capabilities = self.caller_capabilities(caller_pid);

        let child_pid = self
            .shared
            .next_pid
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.shared.process_table.spawn_with_pid(child_pid);

        let mut child = Process::with_capabilities(child_pid, DEFAULT_HEAP_SIZE, capabilities);
        child.set_group_leader(group_leader);
        child.set_namespace_id(namespace_id);
        child.set_priority(Priority::Normal);
        // Flag-before-runnable: set trap_exit while the process is still owned
        // locally, so it is live before the slot is published below.
        child.set_trap_exit(trap_exit);
        child.set_native_body(crate::native::native_process::NativeBody::new(factory));

        if let Some(parent_pid) = link_to {
            let child_linked = child.add_link(parent_pid);
            let parent_linked = add_link_to_slot(&self.shared, parent_pid, child_pid);
            if child_linked && parent_linked {
                #[cfg(feature = "telemetry")]
                crate::telemetry::lifecycle::record_process_linked(parent_pid, child_pid);
            }
        }

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

        #[cfg(feature = "telemetry")]
        crate::telemetry::lifecycle::record_process_spawned(
            &self.shared.atom_table,
            child_pid,
            caller_pid,
            Atom::NIL,
            Atom::NIL,
            0,
        );

        Ok(child_pid)
    }

    /// Native spawn with `trap_exit` set before the process is runnable.
    ///
    /// The scheduler-facing entry point behind
    /// [`Scheduler::spawn_native_trap_exit`](crate::scheduler::Scheduler) and
    /// its linked sibling: `caller_pid` seeds namespace/group-leader exactly as
    /// [`SpawnFacility::spawn_native`] does, and `link_to` links the child
    /// atomically at spawn.
    pub(in crate::scheduler) fn spawn_native_trap_exit(
        &self,
        caller_pid: u64,
        factory: crate::native::native_process::NativeHandlerFactory,
        link_to: Option<u64>,
    ) -> Result<u64, SpawnError> {
        self.spawn_native_flagged(caller_pid, factory, link_to, true)
    }
}
