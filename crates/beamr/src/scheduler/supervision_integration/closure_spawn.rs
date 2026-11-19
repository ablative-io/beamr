//! Spawning a linked child that runs a zero-arity closure, with the
//! closure's environment copied into the child's heap.

use crate::atom::Atom;
use crate::namespace::NamespaceId;
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::process::{Priority, Process};
use crate::scheduler::namespace_registry;
use crate::scheduler::spawning::SpawnRequest;
use crate::term::Term;
use std::sync::Arc;

use super::links::add_link_to_slot;
use super::spawn::SchedulerSpawnFacility;

impl SchedulerSpawnFacility {
    /// Spawn a linked child that runs a zero-arity closure (thunk), deep-copying
    /// the closure's environment into the child's own heap.
    ///
    /// Unlike the plain spawn paths — whose `args: Vec<Term>` are written into
    /// x-registers WITHOUT a heap copy, so callers must keep any backing heap
    /// alive — this primitive owns the copy: every free variable is copied via
    /// the mailbox copy machinery into the child heap before the child becomes
    /// runnable, so the caller's heap may be collected, mutated, or freed the
    /// moment this returns. On [`crate::process::heap::HeapFull`] the child is
    /// rebuilt with a doubled heap up to [`CLOSURE_SPAWN_MAX_HEAP_WORDS`].
    ///
    /// The closure target resolves through the caller's namespace registry the
    /// same way `call_fun` resolves it (generation match, unique-id fallback,
    /// old-generation fallback); export funs (`fun m:f/0`) resolve through the
    /// module export table. Funs backed by a native (BIF/NIF) entry are not
    /// spawnable — there is no bytecode entry IP for them.
    pub(in crate::scheduler) fn spawn_closure_linked(
        &self,
        caller_pid: u64,
        closure_term: Term,
    ) -> Result<u64, crate::error::ExecError> {
        use crate::error::ExecError;
        use crate::term::boxed::Closure;

        let closure = Closure::new(closure_term).ok_or(ExecError::Badfun { term: closure_term })?;
        if closure.arity() != 0 {
            return Err(ExecError::Badarity {
                fun: closure_term,
                args: Vec::new(),
            });
        }
        if closure.num_free() > 256 {
            return Err(ExecError::InvalidOperand("closure free variables"));
        }
        let namespace_id = self.caller_namespace(caller_pid);
        let registry = namespace_registry(&self.shared, namespace_id)
            .unwrap_or_else(|| Arc::clone(&self.shared.module_registry));
        let target = resolve_thunk_target(&registry, closure, closure_term)?;
        let child_pid = self.next_child_pid();
        let mut child =
            self.build_thunk_child(caller_pid, namespace_id, child_pid, closure, &target)?;

        // Link atomically at spawn time: the child object gets the link before
        // it is inserted, so there is no unlinked window. Mirrors `spawn`'s
        // link handling (a caller that died in the meantime simply yields an
        // unlinked child that exits on its own).
        let child_linked = child.add_link(caller_pid);
        let caller_linked = add_link_to_slot(&self.shared, caller_pid, child_pid);
        if child_linked && caller_linked {
            #[cfg(feature = "telemetry")]
            crate::telemetry::lifecycle::record_process_linked(caller_pid, child_pid);
        }

        self.insert_and_wake(child_pid, child);

        #[cfg(feature = "telemetry")]
        crate::telemetry::lifecycle::record_process_spawned(
            &self.shared.atom_table,
            child_pid,
            caller_pid,
            target.module.name,
            Atom::NIL,
            0,
        );

        Ok(child_pid)
    }

    /// Build the thunk child process: entry at the resolved lambda/export IP,
    /// free variables deep-copied into the child heap and loaded into
    /// `x0..num_free-1` — exactly where `call_fun` places a zero-arity
    /// closure's environment.
    fn build_thunk_child(
        &self,
        caller_pid: u64,
        namespace_id: NamespaceId,
        child_pid: u64,
        closure: crate::term::boxed::Closure,
        target: &crate::interpreter::opcodes::closures::ResolvedClosureTarget,
    ) -> Result<Process, crate::error::ExecError> {
        use crate::error::ExecError;

        let instruction_pointer = target.module.label_ip(target.label)?;
        let group_leader = self.caller_group_leader(caller_pid);
        let capabilities = self.caller_capabilities(caller_pid);
        let mut heap_size = DEFAULT_HEAP_SIZE;
        loop {
            let mut child = crate::scheduler::spawning::build_process(SpawnRequest {
                pid: child_pid,
                module: target.module_name,
                module_version: Arc::clone(&target.module),
                instruction_pointer,
                args: Vec::new(),
                namespace_id,
                group_leader,
                capabilities: capabilities.clone(),
                priority: Priority::Normal,
                heap_size,
                parent_pid: caller_pid,
                function: Atom::NIL,
                arity: 0,
                #[cfg(feature = "telemetry")]
                trace_context: None,
            });
            match copy_closure_env(closure, &mut child) {
                Ok(()) => return Ok(child),
                Err(crate::mailbox::SendError::HeapFull(full)) => {
                    if heap_size >= CLOSURE_SPAWN_MAX_HEAP_WORDS {
                        return Err(ExecError::from(full));
                    }
                    heap_size = heap_size
                        .saturating_mul(2)
                        .min(CLOSURE_SPAWN_MAX_HEAP_WORDS);
                }
                Err(crate::mailbox::SendError::InvalidBoxedTerm) => {
                    return Err(ExecError::InvalidOperand("closure free variable"));
                }
            }
        }
    }
}

/// Initial-heap cap for a closure-spawned child (words). The environment copy
/// retries with a doubled heap on `HeapFull`; this bounds the doubling so a
/// pathological environment fails the spawn with a typed error instead of
/// exhausting memory. 2^26 words = 512 MiB.
const CLOSURE_SPAWN_MAX_HEAP_WORDS: usize = 1 << 26;

/// Resolve a thunk closure to the module/label its child process starts at.
///
/// Local closures resolve exactly as `call_fun` does (generation match with
/// unique-id validation, then unique-id search of the current and old module
/// generations). Export funs resolve through the export table at arity 0.
fn resolve_thunk_target(
    registry: &Arc<crate::module::ModuleRegistry>,
    closure: crate::term::boxed::Closure,
    closure_term: Term,
) -> Result<crate::interpreter::opcodes::closures::ResolvedClosureTarget, crate::error::ExecError> {
    use crate::error::ExecError;
    use crate::interpreter::opcodes::closures::{ResolvedClosureTarget, resolve_closure_target};

    let module_atom = closure
        .module()
        .ok_or(ExecError::Badfun { term: closure_term })?;
    if closure.is_export() {
        let function = closure
            .export_function()
            .ok_or(ExecError::Badfun { term: closure_term })?;
        let entry = registry.lookup_mfa(module_atom, function, 0)?;
        return Ok(ResolvedClosureTarget {
            module_name: entry.module.name,
            label: entry.label,
            module: entry.module,
        });
    }
    let current = registry
        .lookup(module_atom)
        .ok_or(ExecError::Badfun { term: closure_term })?;
    resolve_closure_target(closure, current.as_ref(), Some(registry), closure_term)
}

/// Deep-copy every free variable of `closure` into `child`'s heap and load the
/// copies into `x0..num_free-1` (a zero-arity closure's environment registers).
fn copy_closure_env(
    closure: crate::term::boxed::Closure,
    child: &mut Process,
) -> Result<(), crate::mailbox::SendError> {
    for index in 0..closure.num_free() {
        let free_var = closure
            .free_var(index)
            .ok_or(crate::mailbox::SendError::InvalidBoxedTerm)?;
        let copied = crate::mailbox::copy_term(free_var, child.heap_mut())?;
        let register =
            u16::try_from(index).map_err(|_| crate::mailbox::SendError::InvalidBoxedTerm)?;
        child.set_x_reg(register, copied);
    }
    Ok(())
}
