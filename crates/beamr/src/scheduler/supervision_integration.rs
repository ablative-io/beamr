//! Supervision integration for the scheduler — exit signal propagation
//! through links and DOWN message delivery through monitors.
//!
//! Extracted from `mod.rs` to keep per-file line counts within the project
//! constraint (500 lines).

use std::collections::VecDeque;

use crate::process::{ExitReason, ProcessStatus, RemotePid};
use crate::scheduler::process_slot::PendingExitSource;
use crate::supervision::link;
use crate::supervision::monitor;

use super::execution::wake_process;
use super::{ProcessSlot, ScheduledProcess, SharedState, dist_control_out, lock_or_recover};

mod child_insertion;
mod closure_spawn;
mod distribution_send;
mod ets_facility;
mod group_leader;
mod io_facilities;
mod links;
mod local_send;
mod native_services;
mod remote_links;
mod spawn;
mod spawn_native;
mod spawn_options;
mod supervision_facility;

pub(super) use distribution_send::SchedulerDistributionSendFacility;
#[cfg(test)]
pub(super) use distribution_send::block_on_distribution_send_with_write_deadline;
pub(super) use ets_facility::{build_ets_transfer_message, deliver_ets_transfer};
pub(super) use links::SchedulerLinkFacility;
pub(super) use native_services::build_native_services;
pub(super) use remote_links::{
    SchedulerDistributionControlFacility, establish_remote_link,
    register_distribution_control_handler, remove_remote_link,
};
pub(super) use spawn::SchedulerSpawnFacility;
pub(super) use supervision_facility::SchedulerSupervisionFacility;

/// Propagate exit signals through links and deliver DOWN messages through
/// monitors when a process exits. Uses a worklist pattern to handle cascade
/// deaths iteratively rather than recursively.
pub(super) fn propagate_exit(shared: &SharedState, pid: u64, reason: ExitReason) {
    // Collect linked PIDs from the exiting process.
    let linked_pids = take_links_from(shared, pid);
    let remote_links = take_remote_links_from(shared, pid);
    let terminal_reason = link::terminal_reason(reason);

    // Deliver DOWN messages to all monitors of this process.
    deliver_down_messages(shared, pid, reason);

    // Mark dead in link_set for future link_pid() calls.
    {
        let mut ls = lock_or_recover(&shared.link_set);
        ls.process_exited_tombstone(pid, terminal_reason);
    }

    // Send remote EXIT controls for cross-node links.
    for remote in remote_links {
        send_remote_exit(shared, pid, remote, terminal_reason);
    }

    // Process link cascade with worklist pattern.
    // The signal sent through links is always `terminal_reason`: Kill becomes
    // Killed, matching BEAM semantics where only a direct exit signal is
    // untrappable — propagation through links always uses the terminal reason.
    let mut worklist: VecDeque<(u64, u64, ExitReason)> = linked_pids
        .into_iter()
        .map(|linked_pid| (pid, linked_pid, terminal_reason))
        .collect();
    while let Some((source_pid, target_pid, signal_reason)) = worklist.pop_front() {
        let cascade = process_exit_signal(shared, source_pid, target_pid, signal_reason);
        worklist.extend(cascade);
    }
}

/// Take the link set from an exiting process. The process body may already
/// have been removed, absent, or executing, so handle each slot explicitly.
pub(super) fn take_links_from(shared: &SharedState, pid: u64) -> Vec<u64> {
    if let Some(entry) = shared.process_bodies.get(&pid) {
        let mut slot = lock_or_recover(&entry);
        match &mut *slot {
            ProcessSlot::Present(ScheduledProcess(process)) => {
                return process.take_links();
            }
            ProcessSlot::Executing(metadata) => return metadata.links().to_vec(),
            ProcessSlot::Absent => {}
        }
    }
    Vec::new()
}

pub(super) fn take_remote_links_from(shared: &SharedState, pid: u64) -> Vec<RemotePid> {
    if let Some(entry) = shared.process_bodies.get(&pid) {
        let mut slot = lock_or_recover(&entry);
        match &mut *slot {
            ProcessSlot::Present(ScheduledProcess(process)) => {
                return process.take_remote_links();
            }
            ProcessSlot::Executing(metadata) => {
                return metadata.take_remote_links();
            }
            ProcessSlot::Absent => {}
        }
    }
    Vec::new()
}

// The remote-supervision appliers live in `remote_supervision.rs` (moved out
// to keep this file inside the per-file line budget); re-exported here so the
// connection-event subscriber (`connection_lifecycle::handle_connection_event`)
// and existing tests keep their `supervision_integration::` paths.
pub(crate) use super::remote_supervision::{connection_down, process_remote_exit_signal};

fn send_remote_exit(shared: &SharedState, caller_pid: u64, target: RemotePid, reason: ExitReason) {
    dist_control_out::send_exit_linked(shared, caller_pid, target, reason);
}

/// Deliver a single exit signal to a linked process. Returns any cascade
/// entries (source_pid, linked_pid, reason) for processes that must also die.
fn process_exit_signal(
    shared: &SharedState,
    source_pid: u64,
    target_pid: u64,
    reason: ExitReason,
) -> Vec<(u64, u64, ExitReason)> {
    let Some(entry) = shared.process_bodies.get(&target_pid) else {
        return Vec::new();
    };
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(ScheduledProcess(target)) => {
            // Already exited? Nothing to do.
            if matches!(target.status(), ProcessStatus::Exited(_)) {
                return Vec::new();
            }

            // Remove the reverse link.
            target.remove_link(source_pid);

            let should_die =
                reason == ExitReason::Kill || (reason != ExitReason::Normal && !target.trap_exit());

            if should_die {
                // Kill signal bypasses trap_exit and propagates as 'killed'.
                let propagated_reason = link::terminal_reason(reason);
                if reason == ExitReason::Kill {
                    target.set_trap_exit(false);
                }

                // Collect this process's links for cascade before terminating
                // — BOTH halves: local links feed the worklist, remote links
                // each get a wire EXIT below (dropping them with the body
                // would silently sever the peer's half-link).
                let cascade_links: Vec<u64> = target
                    .take_links()
                    .into_iter()
                    .filter(|linked_pid| *linked_pid != source_pid)
                    .collect();
                let remote_links = target.take_remote_links();

                target.terminate(propagated_reason);

                // Record tombstone and remove resources owned by the terminated process.
                shared.insert_exit_tombstone(target_pid, propagated_reason);
                let _deleted_tables = shared.transfer_or_delete_tables_owned_by(target_pid);
                {
                    let mut ls = lock_or_recover(&shared.link_set);
                    ls.process_exited_tombstone(target_pid, propagated_reason);
                }

                // Deliver DOWN messages for monitors on the cascaded process.
                // Must drop the slot lock first to avoid deadlock.
                drop(slot);
                drop(entry);
                deliver_down_messages(shared, target_pid, propagated_reason);

                // Send the terminal EXIT to every remote half-link, exactly
                // as full propagation would (`propagate_exit`).
                for remote in remote_links {
                    send_remote_exit(shared, target_pid, remote, propagated_reason);
                }

                // Finalize exactly like a direct kill — table, BODY, owned
                // fd resources, wait set, timers, suspensions, pg, metrics.
                // Propagation is NOT re-run: this arm already took the
                // target's links for the cascade worklist and delivered its
                // DOWNs above. (The previous hand-written subset here left
                // the body, fds, pg memberships, and metric state stranded
                // on every cascade kill of a stored process.)
                super::execution::finalize_exited_process(shared, target_pid, propagated_reason);

                cascade_links
                    .into_iter()
                    .map(|linked_pid| (target_pid, linked_pid, propagated_reason))
                    .collect()
            } else if target.trap_exit() {
                // Process traps exits: deliver {EXIT, SourcePid, Reason} as message.
                link::enqueue_exit_message_pub(target, source_pid, reason);

                // Wake the process if it was waiting for a message.
                let target_pid_copy = target_pid;
                drop(slot);
                drop(entry);
                wake_process(shared, target_pid_copy);

                Vec::new()
            } else {
                // Normal exit to non-trapping process: no action needed.
                Vec::new()
            }
        }
        ProcessSlot::Executing(metadata) => {
            metadata.remove_link(source_pid);
            let should_die =
                reason == ExitReason::Kill || (reason != ExitReason::Normal && !metadata.trap_exit);

            if should_die {
                let propagated_reason = link::terminal_reason(reason);
                if reason == ExitReason::Kill {
                    metadata.trap_exit = false;
                }
                let cascade_links: Vec<u64> = metadata
                    .links()
                    .iter()
                    .copied()
                    .filter(|linked_pid| *linked_pid != source_pid)
                    .collect();
                shared.insert_exit_tombstone(target_pid, propagated_reason);
                let _deleted_tables = shared.transfer_or_delete_tables_owned_by(target_pid);
                {
                    let mut ls = lock_or_recover(&shared.link_set);
                    ls.process_exited_tombstone(target_pid, propagated_reason);
                }
                drop(slot);
                drop(entry);
                deliver_down_messages(shared, target_pid, propagated_reason);

                cascade_links
                    .into_iter()
                    .map(|linked_pid| (target_pid, linked_pid, propagated_reason))
                    .collect()
            } else if metadata.trap_exit {
                // Process traps exits: queue {EXIT, SourcePid, Reason} for
                // delivery when the slice completes. This mirrors the Present
                // arm's `else if target.trap_exit()` and MUST include NORMAL
                // exits (OTP delivers {'EXIT', Pid, normal} for a normal
                // linked exit to a trapping process). `should_die` has already
                // peeled off Kill and abnormal-non-trapping cases, so reaching
                // here means a trapping target for any non-kill reason.
                metadata
                    .pending_exit_messages
                    .push((PendingExitSource::Local(source_pid), reason));
                drop(slot);
                drop(entry);
                wake_process(shared, target_pid);
                Vec::new()
            } else {
                Vec::new()
            }
        }
        ProcessSlot::Absent => Vec::new(),
    }
}

/// Deliver DOWN messages to all watchers of `target_pid`.
fn deliver_down_messages(shared: &SharedState, target_pid: u64, reason: ExitReason) {
    // Collect monitor info under monitor_set lock, then release.
    let watcher_info: Vec<(u64, u64)> = {
        let mut ms = lock_or_recover(&shared.monitor_set);
        ms.collect_watchers_and_remove(target_pid, reason)
    };

    for (watcher_pid, reference) in watcher_info {
        deliver_single_down_and_wake(shared, watcher_pid, reference, target_pid, reason);
    }
}

/// Deliver a single DOWN message to a watcher process. Returns true if
/// the message was successfully enqueued.
fn deliver_single_down(
    shared: &SharedState,
    watcher_pid: u64,
    reference: u64,
    target_pid: u64,
    reason: ExitReason,
) -> bool {
    let Some(entry) = shared.process_bodies.get(&watcher_pid) else {
        return false;
    };
    let mut slot = lock_or_recover(&entry);
    match &mut *slot {
        ProcessSlot::Present(ScheduledProcess(watcher)) => {
            if matches!(watcher.status(), ProcessStatus::Exited(_)) {
                return false;
            }

            watcher.remove_monitor(reference);
            monitor::enqueue_down_message_pub(watcher, reference, target_pid, reason);
            true
        }
        ProcessSlot::Executing(metadata) => {
            metadata.remove_monitor(reference);
            metadata
                .pending_down_messages
                .push((reference, target_pid, reason));
            true
        }
        ProcessSlot::Absent => false,
    }
}

/// Admit one DOWN and perform the wake only after the mailbox or executing-slot
/// metadata contains it. Returns whether a live watcher accepted the message.
fn deliver_single_down_and_wake(
    shared: &SharedState,
    watcher_pid: u64,
    reference: u64,
    target_pid: u64,
    reason: ExitReason,
) -> bool {
    let delivered = deliver_single_down(shared, watcher_pid, reference, target_pid, reason);
    if delivered {
        // For Present this closes the enqueue-to-park race. For Executing the
        // wake is normally a no-op because the watcher is not registered in
        // the wait set yet; store-back merges pending_down_messages before the
        // Wait arm's post-registration mailbox recheck, which self-wakes it.
        wake_process(shared, watcher_pid);
    }
    delivered
}

pub(super) fn shared_exit_tombstone(shared: &SharedState, pid: u64, reason: ExitReason) {
    shared.insert_exit_tombstone(pid, reason);
    let _deleted_tables = shared.transfer_or_delete_tables_owned_by(pid);
    let mut ls = lock_or_recover(&shared.link_set);
    ls.process_exited_tombstone(pid, reason);
}
