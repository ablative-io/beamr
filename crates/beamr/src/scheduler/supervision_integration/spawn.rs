//! The scheduler's `SpawnFacility`, and the teardown-admission gate every
//! spawn entry point passes.

use crate::atom::Atom;
use crate::namespace::NamespaceId;
use crate::native::spawn::{
    SpawnError, SpawnFacility, SpawnMonitorResult, SpawnOptions, SpawnOptionsResult,
};
use crate::process::Priority;
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::scheduler::{
    ProcessSlot, ScheduledProcess, SharedState, lock_or_recover, namespace_registry,
};
use crate::term::Term;
use std::sync::Arc;

use super::links::add_link_to_slot;

/// Real `SpawnFacility` backed by the scheduler's shared state.
/// Test-only control for the admitted-spawn/shutdown interleaving pin:
/// holds the `Arc::as_ptr` of the ONE `SharedState` whose spawns should park
/// at the admission gate (0 = disabled), so a parallel test's spawn on a
/// different scheduler can neither park nor satisfy the acknowledgment.
#[cfg(test)]
pub(in crate::scheduler) static SPAWN_HOLD_TARGET: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(in crate::scheduler) static SPAWN_HELD_AT_GATE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(in crate::scheduler) struct SchedulerSpawnFacility {
    pub(in crate::scheduler) shared: Arc<SharedState>,
    pub(in crate::scheduler) namespace_id: NamespaceId,
}

impl SpawnFacility for SchedulerSpawnFacility {
    fn spawn(
        &self,
        caller_pid: u64,
        module: Atom,
        function: Atom,
        args: Vec<Term>,
        link_to: Option<u64>,
    ) -> Result<u64, SpawnError> {
        let _admission = self.admit_or_refuse()?;
        self.admission_test_gate();
        let namespace_id = self.caller_namespace(caller_pid);
        let group_leader = self.caller_group_leader(caller_pid);
        let capabilities = self.caller_capabilities(caller_pid);
        let registry = namespace_registry(&self.shared, namespace_id)
            .unwrap_or_else(|| Arc::clone(&self.shared.module_registry));
        let arity = u8::try_from(args.len()).map_err(|_| SpawnError::UnresolvedMfa)?;
        let entry = registry
            .lookup_mfa(module, function, arity)
            .map_err(|_| SpawnError::UnresolvedMfa)?;
        let ip = entry
            .module
            .label_ip(entry.label)
            .map_err(|_| SpawnError::UnresolvedMfa)?;

        let child_pid = self
            .shared
            .next_pid
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.shared.process_table.spawn_with_pid(child_pid);

        let mut child =
            crate::scheduler::spawning::build_process(crate::scheduler::spawning::SpawnRequest {
                pid: child_pid,
                module: entry.module.name,
                module_version: Arc::clone(&entry.module),
                instruction_pointer: ip,
                args,
                namespace_id,
                group_leader,
                capabilities,
                priority: Priority::Normal,
                heap_size: DEFAULT_HEAP_SIZE,
                parent_pid: caller_pid,
                function,
                arity,
                #[cfg(feature = "telemetry")]
                trace_context: None,
            });

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

        // Enqueue to a scheduler thread's inject queue by notifying.
        self.shared
            .spawn_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Put on process table directly (already done above), and wake a thread.
        // NOTE: The child is already in process_bodies; the scheduler loop will
        // find it when it picks up the PID. We need to put the PID in a run queue.
        // Since we don't have direct access to inject queues from here, we put
        // the pid into the wait set as woken so it gets picked up.
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
            entry.module.name,
            function,
            arity,
        );

        Ok(child_pid)
    }

    fn spawn_native(
        &self,
        caller_pid: u64,
        factory: crate::native::native_process::NativeHandlerFactory,
        link_to: Option<u64>,
    ) -> Result<u64, SpawnError> {
        // Existing spelling: spawn a native process that does NOT trap exits.
        // Delegates to the flag-carrying inherent method with `trap_exit=false`
        // so behaviour is byte-identical to before.
        self.spawn_native_flagged(caller_pid, factory, link_to, false)
    }

    fn spawn_monitor(
        &self,
        caller_pid: u64,
        module: Atom,
        function: Atom,
        args: Vec<Term>,
    ) -> Result<SpawnMonitorResult, SpawnError> {
        self.spawn_mfa_with_monitor(caller_pid, module, function, args)
    }

    fn spawn_lambda(
        &self,
        caller_pid: u64,
        module: Atom,
        lambda_index: u32,
        link_to: Option<u64>,
    ) -> Result<u64, SpawnError> {
        let _admission = self.admit_or_refuse()?;
        let namespace_id = self.caller_namespace(caller_pid);
        let group_leader = self.caller_group_leader(caller_pid);
        let capabilities = self.caller_capabilities(caller_pid);
        let registry = namespace_registry(&self.shared, namespace_id)
            .unwrap_or_else(|| Arc::clone(&self.shared.module_registry));
        let loaded = registry.lookup(module).ok_or(SpawnError::UnresolvedMfa)?;
        let lambda = loaded
            .lambdas
            .get(lambda_index as usize)
            .ok_or(SpawnError::UnresolvedMfa)?;
        let ip = loaded
            .label_ip(lambda.label)
            .map_err(|_| SpawnError::UnresolvedMfa)?;

        let child_pid = self
            .shared
            .next_pid
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.shared.process_table.spawn_with_pid(child_pid);

        let mut child =
            crate::scheduler::spawning::build_process(crate::scheduler::spawning::SpawnRequest {
                pid: child_pid,
                module: loaded.name,
                module_version: Arc::clone(&loaded),
                instruction_pointer: ip,
                args: Vec::new(),
                namespace_id,
                group_leader,
                capabilities,
                priority: Priority::Normal,
                heap_size: DEFAULT_HEAP_SIZE,
                parent_pid: caller_pid,
                function: Atom::NIL,
                arity: 0,
                #[cfg(feature = "telemetry")]
                trace_context: None,
            });

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
            loaded.name,
            Atom::NIL,
            0,
        );

        Ok(child_pid)
    }

    fn spawn_lambda_monitor(
        &self,
        caller_pid: u64,
        module: Atom,
        lambda_index: u32,
    ) -> Result<SpawnMonitorResult, SpawnError> {
        self.spawn_lambda_with_monitor(caller_pid, module, lambda_index)
    }

    fn spawn_with_options(
        &self,
        caller_pid: u64,
        module: Atom,
        function: Atom,
        args: Vec<Term>,
        options: SpawnOptions,
    ) -> Result<SpawnOptionsResult, SpawnError> {
        self.spawn_mfa_with_options(caller_pid, module, function, args, options)
    }

    fn spawn_lambda_with_options(
        &self,
        caller_pid: u64,
        module: Atom,
        lambda_index: u32,
        options: SpawnOptions,
    ) -> Result<SpawnOptionsResult, SpawnError> {
        self.spawn_lambda_with_options_impl(caller_pid, module, lambda_index, options)
    }

    fn spawn_closure_link(&self, caller_pid: u64, closure_term: Term) -> Result<u64, SpawnError> {
        let _admission = self.admit_or_refuse()?;
        self.spawn_closure_linked(caller_pid, closure_term)
            .map_err(|_| SpawnError::UnresolvedMfa)
    }
}

impl SchedulerSpawnFacility {
    /// Teardown-admission gate (spec §4 step 3), shared by EVERY spawn entry
    /// point: an in-flight dirty native on an embedder-owned SHARED pool holds
    /// this facility past the owner's shutdown — no spawn path may allocate a
    /// pid or touch process state for a dead scheduler. Returns an RAII token
    /// the caller HOLDS across its whole mutation: admission is linearized
    /// with the shutdown drain (one lock), so an admitted spawn completes
    /// BEFORE shutdown returns and a later one refuses — a snapshot check
    /// would admit a delayed spawn that mutates after shutdown has returned.
    pub(super) fn admit_or_refuse(
        &self,
    ) -> Result<crate::scheduler::TeardownAdmission, SpawnError> {
        self.shared
            .try_reserve_teardown_admission()
            .ok_or(SpawnError::SchedulerTearingDown)
    }

    /// Test-only barrier immediately after spawn admission: lets a test hold
    /// an ADMITTED spawn while it starts shutdown, proving the drain waits.
    /// Instance-scoped: parks ONLY spawns on the SharedState the test armed,
    /// so parallel tests neither stall here nor satisfy the acknowledgment.
    #[cfg(test)]
    fn admission_test_gate(&self) {
        if let Some(hook) = self.shared.spawn_admission_hook.get() {
            hook();
        }
        let me = std::sync::Arc::as_ptr(&self.shared) as usize;
        if SPAWN_HOLD_TARGET.load(std::sync::atomic::Ordering::Acquire) == me {
            SPAWN_HELD_AT_GATE.store(true, std::sync::atomic::Ordering::Release);
            while SPAWN_HOLD_TARGET.load(std::sync::atomic::Ordering::Acquire) == me {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }

    #[cfg(not(test))]
    fn admission_test_gate(&self) {}
}
