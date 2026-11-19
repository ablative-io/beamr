//! The spawn variants that carry a monitor or spawn options, for MFA and
//! lambda children.

use crate::atom::Atom;
use crate::native::spawn::{SpawnError, SpawnMonitorResult, SpawnOptions, SpawnOptionsResult};
use crate::process::Priority;
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::scheduler::namespace_registry;
use crate::scheduler::spawning::SpawnRequest;
use crate::term::Term;
use std::sync::Arc;

use super::spawn::SchedulerSpawnFacility;

impl SchedulerSpawnFacility {
    pub(super) fn spawn_mfa_with_monitor(
        &self,
        caller_pid: u64,
        module: Atom,
        function: Atom,
        args: Vec<Term>,
    ) -> Result<SpawnMonitorResult, SpawnError> {
        let _admission = self.admit_or_refuse()?;
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

        let child =
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

        let result = self.register_monitor_insert_and_wake(caller_pid, child_pid, child);
        #[cfg(feature = "telemetry")]
        crate::telemetry::lifecycle::record_process_spawned(
            &self.shared.atom_table,
            child_pid,
            caller_pid,
            entry.module.name,
            function,
            arity,
        );
        Ok(result)
    }

    pub(super) fn spawn_mfa_with_options(
        &self,
        caller_pid: u64,
        module: Atom,
        function: Atom,
        args: Vec<Term>,
        options: SpawnOptions,
    ) -> Result<SpawnOptionsResult, SpawnError> {
        let _admission = self.admit_or_refuse()?;
        let namespace_id = self.caller_namespace(caller_pid);
        let group_leader = self.caller_group_leader(caller_pid);
        let capabilities = options
            .capabilities
            .clone()
            .unwrap_or_else(|| self.caller_capabilities(caller_pid));
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

        let request = SpawnRequest {
            pid: self.next_child_pid(),
            module: entry.module.name,
            module_version: Arc::clone(&entry.module),
            instruction_pointer: ip,
            args,
            namespace_id,
            group_leader,
            capabilities,
            priority: options.priority.unwrap_or(Priority::Normal),
            heap_size: options.min_heap_size.unwrap_or(DEFAULT_HEAP_SIZE),
            parent_pid: caller_pid,
            function,
            arity,
            #[cfg(feature = "telemetry")]
            trace_context: None,
        };
        Ok(self.insert_options_child(caller_pid, request, options))
    }

    pub(super) fn spawn_lambda_with_options_impl(
        &self,
        caller_pid: u64,
        module: Atom,
        lambda_index: u32,
        options: SpawnOptions,
    ) -> Result<SpawnOptionsResult, SpawnError> {
        let _admission = self.admit_or_refuse()?;
        let namespace_id = self.caller_namespace(caller_pid);
        let group_leader = self.caller_group_leader(caller_pid);
        let capabilities = options
            .capabilities
            .clone()
            .unwrap_or_else(|| self.caller_capabilities(caller_pid));
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

        let request = SpawnRequest {
            pid: self.next_child_pid(),
            module: loaded.name,
            module_version: Arc::clone(&loaded),
            instruction_pointer: ip,
            args: Vec::new(),
            namespace_id,
            group_leader,
            capabilities,
            priority: options.priority.unwrap_or(Priority::Normal),
            heap_size: options.min_heap_size.unwrap_or(DEFAULT_HEAP_SIZE),
            parent_pid: caller_pid,
            function: Atom::NIL,
            arity: 0,
            #[cfg(feature = "telemetry")]
            trace_context: None,
        };
        Ok(self.insert_options_child(caller_pid, request, options))
    }

    pub(super) fn spawn_lambda_with_monitor(
        &self,
        caller_pid: u64,
        module: Atom,
        lambda_index: u32,
    ) -> Result<SpawnMonitorResult, SpawnError> {
        let _admission = self.admit_or_refuse()?;
        let namespace_id = self.caller_namespace(caller_pid);
        let group_leader = self.caller_group_leader(caller_pid);
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

        let child =
            crate::scheduler::spawning::build_process(crate::scheduler::spawning::SpawnRequest {
                pid: child_pid,
                module: loaded.name,
                module_version: Arc::clone(&loaded),
                instruction_pointer: ip,
                args: Vec::new(),
                namespace_id,
                group_leader,
                capabilities: self.caller_capabilities(caller_pid),
                priority: Priority::Normal,
                heap_size: DEFAULT_HEAP_SIZE,
                parent_pid: caller_pid,
                function: Atom::NIL,
                arity: 0,
                #[cfg(feature = "telemetry")]
                trace_context: None,
            });

        let result = self.register_monitor_insert_and_wake(caller_pid, child_pid, child);
        #[cfg(feature = "telemetry")]
        crate::telemetry::lifecycle::record_process_spawned(
            &self.shared.atom_table,
            child_pid,
            caller_pid,
            loaded.name,
            Atom::NIL,
            0,
        );
        Ok(result)
    }
}
