use super::run_queue::RunQueue;
use super::{
    DEFAULT_REDUCTION_BUDGET, ProcessMetadata, ProcessSlot, ScheduledProcess, Scheduler,
    SharedState, lock_or_recover, namespace_registry, supervision_integration, timer_integration,
};
use crate::error::ExecError;
use crate::hook::HookDecision;
use crate::interpreter::{self, ExecutionResult};
use crate::process::heap::DEFAULT_HEAP_SIZE;
use crate::process::{CodePosition, ExitReason, Process, ProcessStatus};
use crate::term::Term;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

impl Scheduler {
    pub fn wake_notifier(&self, pid: u64) -> impl Fn() + Send + Sync + 'static {
        let shared = Arc::clone(&self.shared);
        move || wake_process(&shared, pid)
    }
    pub fn wake_process(&self, pid: u64) {
        wake_process(&self.shared, pid);
    }
    pub fn resume_process(&self, pid: u64) -> bool {
        timer_integration::resume_suspended(&self.shared, pid)
    }
    pub fn shutdown(&self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.wake_condvar.notify_all();
        let mut threads = lock_or_recover(&self.threads);
        for handle in threads.drain(..) {
            if let Err(payload) = handle.join() {
                std::panic::resume_unwind(payload);
            }
        }
    }
    pub fn run_until_exit(&self, pid: u64) -> (ExitReason, Term) {
        loop {
            if let Some(entry) = self.shared.exit_tombstones.get(&pid) {
                let reason = *entry;
                let result = self
                    .shared
                    .exit_results
                    .remove(&pid)
                    .map(|(_, term)| term)
                    .unwrap_or(Term::NIL);
                return (reason, result);
            }
            let guard = lock_or_recover(&self.shared.wait_set);
            let _ = self
                .shared
                .wake_condvar
                .wait_timeout(guard, std::time::Duration::from_millis(10));
        }
    }
    pub fn take_exit_error(&self, pid: u64) -> Option<ExecError> {
        self.shared.exit_errors.remove(&pid).map(|(_, e)| e)
    }
    pub fn take_exit_exception(&self, pid: u64) -> Option<crate::process::Exception> {
        self.shared.exit_exceptions.remove(&pid).map(|(_, e)| e)
    }
    pub fn wake_with_result(&self, pid: u64, result: Term) {
        self.shared.async_results.insert(pid, result);
        wake_process(&self.shared, pid);
    }
    pub fn terminate_process(&self, pid: u64, reason: ExitReason) {
        if !self.shared.exit_tombstones.contains_key(&pid) {
            cleanup_exited_process(&self.shared, pid, reason);
        }
    }
}

enum SliceOutcome {
    Requeue(Process),
    Wait(Process),
    Suspended(Process),
    Exited(ExitReason, Term),
}

pub(super) fn run_process(shared: &Arc<SharedState>, queue: &RunQueue, pid: u64, my_index: usize) {
    if shared.process_table.get(pid).is_none() {
        return;
    }
    let Some(mut process) = take_runnable_process(shared, pid) else {
        return;
    };
    let outcome = execute_slice(shared, &mut process);
    if let Some(reason) = tombstone_reason(shared, pid) {
        store_runnable_process(shared, process);
        cleanup_exited_process(shared, pid, reason);
        return;
    }
    match outcome {
        SliceOutcome::Requeue(process) => {
            store_runnable_process(shared, process);
            if cleanup_if_tombstoned_after_store(shared, pid) {
                return;
            }
            queue.push(pid);
        }
        SliceOutcome::Wait(mut process) => {
            timer_integration::register_receive_timer(shared, &mut process);
            store_runnable_process(shared, process);
            if cleanup_if_tombstoned_after_store(shared, pid) {
                return;
            }
            lock_or_recover(&shared.wait_set)
                .waiting
                .insert(pid, my_index);
        }
        SliceOutcome::Suspended(process) => {
            store_runnable_process(shared, process);
            if cleanup_if_tombstoned_after_store(shared, pid) {
                return;
            }
            lock_or_recover(&shared.wait_set)
                .waiting
                .insert(pid, my_index);
        }
        SliceOutcome::Exited(reason, result) => {
            shared.exit_results.insert(pid, result);
            store_runnable_process(shared, process);
            cleanup_exited_process(shared, pid, reason);
        }
    }
}

fn take_runnable_process(shared: &SharedState, pid: u64) -> Option<Process> {
    let entry = shared.process_bodies.get(&pid)?;
    let mut slot = lock_or_recover(&entry);
    match std::mem::take(&mut *slot) {
        ProcessSlot::Present(scheduled) => {
            let process = scheduled.0;
            *slot = ProcessSlot::Executing(ProcessMetadata {
                namespace_id: process.namespace_id(),
                links: process.links().to_vec(),
                trap_exit: process.trap_exit(),
                pending_exit_messages: Vec::new(),
            });
            Some(process)
        }
        other => {
            *slot = other;
            None
        }
    }
}

fn store_runnable_process(shared: &SharedState, mut process: Process) {
    let pid = process.pid();
    if let Some(entry) = shared.process_bodies.get(&pid) {
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Executing(metadata) = &mut *slot {
            for linked_pid in &metadata.links {
                process.add_link(*linked_pid);
            }
            for (source_pid, reason) in metadata.pending_exit_messages.drain(..) {
                crate::supervision::link::enqueue_exit_message_pub(
                    &mut process,
                    source_pid,
                    reason,
                );
            }
        }
        *slot = ProcessSlot::Present(ScheduledProcess(process));
    } else {
        shared.process_bodies.insert(
            pid,
            Mutex::new(ProcessSlot::Present(ScheduledProcess(process))),
        );
    }
}

fn cleanup_if_tombstoned_after_store(shared: &SharedState, pid: u64) -> bool {
    if let Some(reason) = tombstone_reason(shared, pid) {
        cleanup_exited_process(shared, pid, reason);
        true
    } else {
        false
    }
}
fn tombstone_reason(shared: &SharedState, pid: u64) -> Option<ExitReason> {
    shared.exit_tombstones.get(&pid).map(|reason| *reason)
}

fn execute_slice(shared: &Arc<SharedState>, process: &mut Process) -> SliceOutcome {
    if !matches!(
        process.status(),
        ProcessStatus::New
            | ProcessStatus::Yielded
            | ProcessStatus::Waiting
            | ProcessStatus::Suspended
    ) {
        return SliceOutcome::Exited(exit_reason_from_status(process.status()), process.x_reg(0));
    }
    if process.transition_to(ProcessStatus::Running).is_err() {
        return SliceOutcome::Exited(exit_reason_from_status(process.status()), process.x_reg(0));
    }
    if let Some((_, result_term)) = shared.async_results.remove(&process.pid()) {
        process.set_x_reg(0, result_term);
        if let Some(pos) = process.code_position() {
            process.set_code_position(Some(CodePosition {
                module: pos.module,
                instruction_pointer: pos.instruction_pointer.saturating_add(1),
            }));
        }
    }
    process.reset_reductions(DEFAULT_REDUCTION_BUDGET);
    let module_atom = match process.code_position() {
        Some(position) => position.module,
        None => return exit_process(shared, process, ExitReason::Normal),
    };
    let registry = namespace_registry(shared, process.namespace_id())
        .unwrap_or_else(|| Arc::clone(&shared.module_registry));
    let module = if let Some(current) = process.current_module()
        && current.name == module_atom
        && current
            .code
            .get(
                process
                    .code_position()
                    .map_or(usize::MAX, |pos| pos.instruction_pointer),
            )
            .is_some()
    {
        Arc::clone(current)
    } else {
        let Some(module) = registry.lookup(module_atom) else {
            return exit_process(shared, process, ExitReason::Error);
        };
        process.set_current_module(Arc::clone(&module));
        module
    };
    let services = supervision_integration::build_native_services(shared, process.namespace_id());
    let result = interpreter::run_with_native_services(process, &module, &registry, &services);
    let reductions = DEFAULT_REDUCTION_BUDGET.saturating_sub(process.reduction_counter());
    if matches!(
        result,
        Ok(ExecutionResult::Yielded) | Ok(ExecutionResult::Waiting)
    ) && timer_integration::invoke_hook(shared, process, reductions) == HookDecision::Suspend
    {
        let _t = process.transition_to(ProcessStatus::Suspended);
        return SliceOutcome::Suspended(take_process(process));
    }
    match result {
        Ok(ExecutionResult::Yielded) => {
            let _t = process.transition_to(ProcessStatus::Yielded);
            process.reset_reductions(DEFAULT_REDUCTION_BUDGET);
            SliceOutcome::Requeue(take_process(process))
        }
        Ok(ExecutionResult::Waiting) => {
            let _t = process.transition_to(ProcessStatus::Waiting);
            SliceOutcome::Wait(take_process(process))
        }
        Ok(ExecutionResult::Exited(reason)) => exit_process(shared, process, reason),
        Err(error) => {
            let pid = process.pid();
            shared.exit_errors.insert(pid, error);
            exit_process(shared, process, ExitReason::Error)
        }
    }
}

fn exit_process(shared: &SharedState, process: &mut Process, reason: ExitReason) -> SliceOutcome {
    let pid = process.pid();
    let result = process.x_reg(0);
    if let Some(exception) = process.current_exception() {
        shared.exit_exceptions.insert(pid, exception);
    }
    process.terminate(reason);
    SliceOutcome::Exited(reason, result)
}
fn exit_reason_from_status(status: ProcessStatus) -> ExitReason {
    match status {
        ProcessStatus::Exited(reason) => reason,
        _ => ExitReason::Error,
    }
}

pub(crate) fn cleanup_exited_process(shared: &SharedState, pid: u64, reason: ExitReason) {
    shared.exit_tombstones.insert(pid, reason);
    supervision_integration::propagate_exit(shared, pid, reason);
    let _removed = shared.process_table.remove(pid);
    let _removed_body = shared.process_bodies.remove(&pid);
    let mut wait_set = lock_or_recover(&shared.wait_set);
    wait_set.waiting.remove(&pid);
    wait_set.woken.retain(|(woken_pid, _)| *woken_pid != pid);
}
fn take_process(process: &mut Process) -> Process {
    std::mem::replace(process, Process::new(u64::MAX, DEFAULT_HEAP_SIZE))
}
pub(crate) fn wake_process(shared: &SharedState, pid: u64) {
    timer_integration::cancel_receive_timer(shared, pid);
    let mut wait_set = lock_or_recover(&shared.wait_set);
    if let Some(scheduler_index) = wait_set.waiting.remove(&pid) {
        wait_set.woken.push((pid, scheduler_index));
        shared.wake_condvar.notify_all();
    }
}
pub(super) fn drain_woken(shared: &SharedState, queue: &RunQueue, my_index: usize) {
    let woken = {
        let mut wait_set = lock_or_recover(&shared.wait_set);
        let mut mine = Vec::new();
        wait_set.woken.retain(|(pid, sched_idx)| {
            if *sched_idx == my_index {
                mine.push(*pid);
                false
            } else {
                true
            }
        });
        mine
    };
    for pid in woken {
        if shared.process_table.get(pid).is_some() {
            queue.push(pid);
        }
    }
}
pub(super) fn park_thread(shared: &SharedState) {
    #[cfg(test)]
    shared.idle_parks.fetch_add(1, Ordering::Relaxed);
    if shared.shutdown.load(Ordering::Acquire) {
        return;
    }
    let guard = lock_or_recover(&shared.wait_set);
    match shared
        .wake_condvar
        .wait_timeout(guard, std::time::Duration::from_millis(5))
    {
        Ok(_) => {}
        Err(error) => {
            let _recovered = error.into_inner();
        }
    }
}
