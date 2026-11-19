//! Taking a process out of its slot for a slice and storing it back, and the
//! cleanup owed when a tombstone landed while it was checked out.

use crate::process::{ExitReason, Process};
use crate::scheduler::{
    ProcessMetadata, ProcessSlot, ScheduledProcess, SharedState, lock_or_recover,
};

use super::exit::{cleanup_exited_process, release_process_exit_resources};

pub(in crate::scheduler) fn take_runnable_process(
    shared: &SharedState,
    pid: u64,
) -> Option<Process> {
    let entry = shared.process_bodies.get(&pid)?;
    let mut slot = lock_or_recover(&entry);
    match std::mem::take(&mut *slot) {
        ProcessSlot::Present(scheduled) => {
            let process = scheduled.0;
            let metadata = ProcessMetadata {
                namespace_id: process.namespace_id(),
                capabilities: process.capabilities().clone(),
                relations: process.shared_relations(),
                trap_exit: process.trap_exit(),
                priority: process.priority(),
                current_mfa: process.current_mfa(),
                heap_size: process.heap().total_used(),
                binary_heap_size: process.virtual_binary_heap(),
                message_queue_len: process.mailbox().message_count(),
                group_leader: process.group_leader(),
                logical_clock: process.logical_clock(),
                pending_exit_messages: Vec::new(),
                pending_down_messages: Vec::new(),
                pending_io_messages: Vec::new(),
                pending_distribution_payloads: Vec::new(),
                pending_local_messages: Vec::new(),
                pending_ets_transfer_messages: Vec::new(),
                pending_udp_messages: Vec::new(),
                pending_tcp_messages: Vec::new(),
            };
            *slot = ProcessSlot::Executing(metadata);
            Some(process)
        }
        other => {
            *slot = other;
            None
        }
    }
}

pub(in crate::scheduler) fn store_runnable_process(shared: &SharedState, mut process: Process) {
    let pid = process.pid();
    if let Some(entry) = shared.process_bodies.get(&pid) {
        let mut slot = lock_or_recover(&entry);
        if let ProcessSlot::Executing(metadata) = &mut *slot {
            process.set_group_leader(metadata.group_leader);
            process.set_logical_clock(metadata.logical_clock);
            process.set_capabilities(metadata.capabilities.clone());
            // While the slot is Executing every remote-link and monitor
            // mutation lands on the slot's lists (the checked-out `Process`
            // copy is never touched), so they are taken back whole: merging
            // add-only would resurrect entries the DC-4 exactly-once gate
            // already consumed, double-firing the exit signal on the next
            // connection down and undoing unlink/1. Taking them back is a
            // pointer move, not a merge, so a process that watches many
            // others costs nothing extra each time it runs.
            process.restore_relations(std::mem::take(&mut metadata.relations));
            for (source, reason) in metadata.pending_exit_messages.drain(..) {
                match source {
                    crate::scheduler::process_slot::PendingExitSource::Local(source_pid) => {
                        crate::supervision::link::enqueue_exit_message_pub(
                            &mut process,
                            source_pid,
                            reason,
                        );
                    }
                    crate::scheduler::process_slot::PendingExitSource::Remote(remote_pid) => {
                        crate::supervision::link::enqueue_remote_exit_message_pub(
                            &mut process,
                            remote_pid,
                            reason,
                        );
                    }
                }
            }
            for (reference, target_pid, reason) in metadata.pending_down_messages.drain(..) {
                crate::supervision::monitor::enqueue_down_message_pub(
                    &mut process,
                    reference,
                    target_pid,
                    reason,
                );
            }
            for message in metadata.pending_io_messages.drain(..) {
                match message {
                    crate::scheduler::PendingMailboxMessage::TargetOwned(term) => {
                        process.mailbox_mut().push_owned(term);
                    }
                    crate::scheduler::PendingMailboxMessage::HostOwned {
                        message,
                        completion,
                    } => {
                        let copied = crate::scheduler::timer_integration::copy_owned_message(
                            &mut process,
                            &message,
                        );
                        let _receiver_gone = completion.send(copied);
                    }
                }
            }
            for payload in metadata.pending_distribution_payloads.drain(..) {
                let mut context = crate::native::ProcessContext::new();
                context.attach_process(&mut process, 0);
                let Ok(message) =
                    crate::etf::decode::decode_term(&payload, &mut context, &shared.atom_table)
                else {
                    continue;
                };
                process.mailbox_mut().push_owned(message);
            }
            for payload in metadata.pending_local_messages.drain(..) {
                let mut context = crate::native::ProcessContext::new();
                context.attach_process(&mut process, 0);
                let Ok(message) =
                    crate::etf::decode::decode_term(&payload, &mut context, &shared.atom_table)
                else {
                    continue;
                };
                process.mailbox_mut().push_owned(message);
            }
            let transfer_atom = shared.atom_table.intern("ETS-TRANSFER");
            for message in metadata.pending_ets_transfer_messages.drain(..) {
                let Some(message) =
                    crate::scheduler::supervision_integration::build_ets_transfer_message(
                        &mut process,
                        transfer_atom,
                        message.table_id,
                        message.from_pid,
                        message.data.root(),
                    )
                else {
                    continue;
                };
                process.mailbox_mut().push_owned(message);
            }
            for udp_msg in metadata.pending_udp_messages.drain(..) {
                if let Some(message) = super::super::build_udp_active_message_for_process(
                    &shared.atom_table,
                    &mut process,
                    &udp_msg.fd,
                    &udp_msg.bytes,
                    udp_msg.addr,
                ) {
                    process.mailbox_mut().push_owned(message);
                }
            }
            for tcp_msg in metadata.pending_tcp_messages.drain(..) {
                if let Some(message) = super::super::build_tcp_active_message_for_process(
                    &shared.atom_table,
                    &mut process,
                    &tcp_msg.fd,
                    &tcp_msg.bytes,
                ) {
                    process.mailbox_mut().push_owned(message);
                }
            }
        }
        *slot = ProcessSlot::Present(ScheduledProcess(process));
    } else if shared.process_table.get(pid).is_none() {
        // The entry vanished mid-execution. The only remover of a body entry
        // is `finalize_exited_process`, which wins the table token first —
        // so the pid is dead, the pid-keyed half is already done against our
        // Executing shadow, and this checked-out process is the LAST live
        // reference to the body. Dispose of it here: re-inserting (the
        // previous behavior) resurrected a body no later finalizer would
        // ever reap — the table token was already spent — and
        // `enqueue_atom_message` would deliver into it despite the dead pid.
        release_process_exit_resources(&mut process, pid);
    } else {
        shared.process_bodies.insert(
            pid,
            std::sync::Mutex::new(ProcessSlot::Present(ScheduledProcess(process))),
        );
    }
}

pub(in crate::scheduler) fn cleanup_if_tombstoned_after_store(
    shared: &SharedState,
    pid: u64,
) -> bool {
    if let Some(reason) = tombstone_reason(shared, pid) {
        cleanup_exited_process(shared, pid, reason);
        true
    } else {
        false
    }
}

pub(super) fn tombstone_reason(shared: &SharedState, pid: u64) -> Option<ExitReason> {
    shared.exit_tombstones.get(&pid)
}

pub(super) fn process_has_queued_messages(shared: &SharedState, pid: u64) -> bool {
    let Some(entry) = shared.process_bodies.get(&pid) else {
        return false;
    };
    let slot = lock_or_recover(&entry);
    match &*slot {
        ProcessSlot::Present(ScheduledProcess(process)) => !process.mailbox().is_empty(),
        ProcessSlot::Executing(_) | ProcessSlot::Absent => false,
    }
}
