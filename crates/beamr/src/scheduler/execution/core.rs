//! Private process time-slice execution helpers.

use crate::ets::copy::OwnedTerm;
use crate::process::{ExitReason, Process};
#[cfg(test)]
use crate::scheduler::ParkGap;
use crate::scheduler::{RunQueue, SharedState, lock_or_recover, timer_integration};
use std::sync::Arc;

use replay_schedule::take_replay_schedule;
use slice::execute_slice_with_recorded_schedule;
use store::{process_has_queued_messages, tombstone_reason};

mod dirty_resume;
mod dirty_submit;
mod exit;
mod replay_schedule;
mod slice;
mod store;
mod suspension_gate;

pub(in crate::scheduler) use exit::{cleanup_exited_process, finalize_exited_process};
pub(in crate::scheduler) use slice::execute_slice;
pub(in crate::scheduler) use store::{
    cleanup_if_tombstoned_after_store, store_runnable_process, take_runnable_process,
};

/// Runs the test-only park-gap hook, if installed, at an interleaving point
/// inside `run_process`'s park sequences.
#[cfg(test)]
fn invoke_park_gap_hook(shared: &SharedState, gap: ParkGap, pid: u64) {
    let hook = lock_or_recover(&shared.park_gap_hook);
    if let Some(hook) = hook.as_ref() {
        hook(shared, gap, pid);
    }
}

pub(in crate::scheduler) enum SliceOutcome {
    Requeue(Process),
    Wait(Process),
    Suspended(Process),
    /// The result is captured as an owning copy before `Process::terminate`
    /// frees the heap it pointed into.
    Exited(ExitReason, OwnedTerm),
}

pub(super) fn run_process(shared: &Arc<SharedState>, queue: &RunQueue, pid: u64, my_index: usize) {
    if shared.process_table.get(pid).is_none() {
        return;
    }
    let Some(mut process) = take_runnable_process(shared, pid) else {
        return;
    };
    let outcome = if process.is_native() {
        // Native dispatch seam (NATIVE-001): a process carrying a Rust handler
        // runs the handler instead of the bytecode interpreter. The replay and
        // bytecode branches below are untouched, and the Requeue / Wait /
        // Suspended / Exited handling that follows is shared by both paths.
        super::native_slice::run_native_slice(shared, &mut process)
    } else if shared.replay_mode {
        let Some(recorded_schedule) = take_replay_schedule(shared, pid, my_index) else {
            store_runnable_process(shared, process);
            cleanup_exited_process(shared, pid, ExitReason::Error);
            return;
        };
        execute_slice_with_recorded_schedule(shared, &mut process, recorded_schedule)
    } else {
        execute_slice(shared, &mut process)
    };
    if let Some(reason) = tombstone_reason(shared, pid) {
        store_runnable_process(shared, process);
        cleanup_exited_process(shared, pid, reason);
        return;
    }
    match outcome {
        SliceOutcome::Requeue(process) => {
            let priority = process.priority();
            store_runnable_process(shared, process);
            if cleanup_if_tombstoned_after_store(shared, pid) {
                return;
            }
            queue.push_with_priority(pid, priority);
            // W23: a Requeue with others queued behind it is surplus for an
            // idle worker (§3.8.2 C1); alone, it is not.
            super::surplus_edge(shared, queue, false);
        }
        SliceOutcome::Wait(mut process) => {
            timer_integration::register_receive_timer(shared, &mut process);
            let priority = process.priority();
            store_runnable_process(shared, process);
            if cleanup_if_tombstoned_after_store(shared, pid) {
                return;
            }
            // Park ordering against a concurrent deliver→wake (the sender
            // pushes the message under the slot lock, then calls
            // `wake_process`, which is a no-op unless the pid is registered
            // in `waiting`). Register BEFORE the final mailbox recheck so
            // every interleaving resolves to a scheduled process:
            //
            // 1. Delivery completes before the registration below: the wake
            //    no-ops, but the message is already visible (pushed into the
            //    mailbox, or merged from pending metadata by the store-back
            //    above), so the recheck sees it and self-wakes.
            // 2. Delivery lands between registration and the recheck: the
            //    wake moves the pid from `waiting` to `woken`; the recheck
            //    also sees the message, but its `waiting` removal finds
            //    nothing, so the process is scheduled exactly once (by the
            //    woken drain).
            // 3. Delivery lands after the recheck: the wake finds the pid in
            //    `waiting` and schedules it.
            //
            // Rechecking before registering (the previous order) lost
            // interleaving 1: a delivery in that gap woke nobody and the
            // process parked forever.
            // Gap hook: must stay between the store-back above and the
            // wait-set registration below — move it with them.
            #[cfg(test)]
            invoke_park_gap_hook(shared, ParkGap::WaitStored, pid);
            {
                let mut ws = lock_or_recover(&shared.wait_set);
                ws.waiting.insert(pid, my_index);
            }
            // Gap hook: must stay between the registration above and the
            // recheck below — move it with them.
            #[cfg(test)]
            invoke_park_gap_hook(shared, ParkGap::WaitRegistered, pid);
            // Death recheck AFTER registering: a kill landing in the
            // store→register gap (direct exit_signal or a link-cascade
            // process_exit_signal — both finalize on the Present slot) runs
            // its wait-set sweep before the registration above exists, so
            // the sweep finds nothing and the insert would leave a dead pid
            // in `waiting` forever (one leaked entry per kill that threads
            // the gap; found while pinning C3). The durable dead signal is
            // PROCESS-TABLE absence — the table entry is removed exactly
            // once, at finalization, and pids are never reused — NOT the
            // exit tombstone, which is FIFO-bounded and can be evicted.
            // Finalization has already completed in that ordering, so
            // withdrawing the stale registration is the only outstanding
            // obligation — re-running cleanup here would double-propagate
            // exits.
            if shared.process_table.get(pid).is_none() {
                let mut ws = lock_or_recover(&shared.wait_set);
                ws.waiting.remove(&pid);
                return;
            }
            // The recheck must notice EVERY wake source that can land before
            // the registration above: a delivered message, a receive timer
            // that fired while the slot was Executing or in the
            // store-to-register gap, and — for a host-await suspension — a
            // completion published while the slot was Executing
            // (expire_timers/wake_process found nothing in `waiting`, so
            // only this recheck can schedule the process; the event is
            // consumed at the start of the next slice). A stale mark costs
            // one benign spurious wake. A gated host-await park is treated
            // exactly like wake_process treats it: a queued message alone
            // must NOT self-wake it, or the await native would re-execute
            // and re-submit its host call. Message-wakeable parks (plain
            // receives, select, marker awaits) additionally self-wake on a
            // completion published while the slot was Executing.
            let gated = shared
                .suspensions
                .get(&pid)
                .is_some_and(|mirror| !mirror.wake_on_message);
            let wake_worthy = if gated {
                shared.has_consumable_suspension_event(pid)
            } else {
                process_has_queued_messages(shared, pid)
                    || timer_integration::has_pending_expired_timer(shared, pid)
                    || shared.has_consumable_suspension_event(pid)
            };
            if wake_worthy {
                let self_woke = {
                    let mut ws = lock_or_recover(&shared.wait_set);
                    ws.waiting.remove(&pid).is_some()
                };
                if self_woke {
                    queue.push_with_priority(pid, priority);
                    // W23: the self-wake push, same rule as the Requeue.
                    super::surplus_edge(shared, queue, false);
                }
            }
        }
        SliceOutcome::Suspended(process) => {
            // Suspended means a dirty native call is in flight or the hook
            // suspended the process (host-side request_suspend parks through
            // Waiting/Wait instead). Mailbox arrivals while parked here are
            // normal and must NOT resume the process — only the suspension's
            // own event may: the dirty completion published under the
            // suspension's call id, or a matching embedder resume. That
            // event arrives via resume_suspended or, when it landed before
            // this registration, the check below.
            store_runnable_process(shared, process);
            if cleanup_if_tombstoned_after_store(shared, pid) {
                return;
            }
            // Gap hook: must stay between the store-back above and the
            // wait-set registration below — move it with them.
            #[cfg(test)]
            invoke_park_gap_hook(shared, ParkGap::SuspendStored, pid);
            {
                let mut ws = lock_or_recover(&shared.wait_set);
                ws.waiting.insert(pid, my_index);
            }
            // The completion bridge publishes the dirty result, then calls
            // `resume_suspended` (flip status Suspended→Yielded, move the
            // pid from `waiting` to `woken`). Interleavings against the
            // store-back and registration above:
            //
            // 1. Bridge resume before the store-back: the slot is still
            //    Executing, so the status flip is refused and nothing is
            //    resumed; the resume below (status Suspended, pid
            //    registered) succeeds.
            // 2. Bridge resume between store-back and registration: the
            //    status flips to Yielded but the `waiting` removal finds
            //    nothing; the resume below then refuses because the status
            //    is no longer Suspended. The result is already published,
            //    so the only missing step is the unpark — performed by the
            //    fallback below.
            // 3. Bridge resume after registration: it fully succeeds; the
            //    resume below refuses (status already Yielded) and the
            //    fallback's `waiting` removal finds the pid already moved
            //    to `woken` — a no-op.
            //
            // The fallback re-verifies the pending event under the wait-set
            // lock: if a woken slice already consumed it (and possibly
            // parked again under a NEW suspension with a fresh call id),
            // the unpark must not fire. The event check is identity-keyed,
            // so a completion for the OLD suspension can never unpark the
            // NEW one — and even if an interleaving slips a stray unpark
            // through, the slice-start gate re-parks without executing.
            if shared.has_consumable_suspension_event(pid)
                && !timer_integration::resume_suspended(shared, pid)
            {
                let mut ws = lock_or_recover(&shared.wait_set);
                if shared.has_consumable_suspension_event(pid)
                    && let Some(index) = ws.waiting.remove(&pid)
                {
                    ws.woken.push((pid, index));
                    // W19: edge E in the same critical section.
                    shared.wake_edge_locked(&ws);
                }
            }
        }
        SliceOutcome::Exited(reason, result) => {
            shared.exit_results.insert(pid, result);
            store_runnable_process(shared, process);
            cleanup_exited_process(shared, pid, reason);
        }
    }
}
