//! The scheduler's `SupervisionFacility`: monitor, demonitor and exit
//! signals.

use crate::native::supervision::{MonitorResult, SupervisionError, SupervisionFacility};
use crate::process::{ExitReason, ProcessStatus};
use crate::scheduler::execution::{cleanup_exited_process, wake_process};
use crate::scheduler::process_slot::PendingExitSource;
use crate::scheduler::{ProcessSlot, ScheduledProcess, SharedState, lock_or_recover};
use crate::supervision::link;
use std::sync::Arc;

use super::{deliver_single_down_and_wake, shared_exit_tombstone};

/// Real `SupervisionFacility` backed by the scheduler's shared state.
pub(in crate::scheduler) struct SchedulerSupervisionFacility {
    pub(in crate::scheduler) shared: Arc<SharedState>,
}

impl SupervisionFacility for SchedulerSupervisionFacility {
    fn monitor(&self, caller_pid: u64, target_pid: u64) -> Result<MonitorResult, SupervisionError> {
        let mut ms = lock_or_recover(&self.shared.monitor_set);

        // Check if target is already dead.
        if let Some(reason) = self.shared.exit_tombstones.get(&target_pid) {
            let reference = ms.allocate_reference_pub();
            drop(ms);

            // Use the normal-exit admission and wake path. In particular, an
            // executing caller receives the DOWN through pending metadata.
            let immediate_down = deliver_single_down_and_wake(
                &self.shared,
                caller_pid,
                reference,
                target_pid,
                reason,
            );

            return Ok(MonitorResult {
                reference,
                immediate_down,
            });
        }

        // Both processes must exist.
        if self.shared.process_table.get(target_pid).is_none() {
            return Err(SupervisionError::NoProc);
        }

        // Allocate reference and register monitor in monitor_set.
        let reference = ms.allocate_reference_pub();
        let mon = crate::process::Monitor::new(reference, caller_pid, target_pid);
        ms.register_monitor(reference, mon, target_pid);
        drop(ms);

        // Add monitor to caller process.
        if let Some(entry) = self.shared.process_bodies.get(&caller_pid) {
            let mut slot = lock_or_recover(&entry);
            if let ProcessSlot::Present(ScheduledProcess(p)) = &mut *slot {
                p.add_monitor(mon);
            }
        }

        // Add monitor to target process.
        if let Some(entry) = self.shared.process_bodies.get(&target_pid) {
            let mut slot = lock_or_recover(&entry);
            if let ProcessSlot::Present(ScheduledProcess(p)) = &mut *slot {
                p.add_monitor(mon);
            }
        }

        #[cfg(feature = "telemetry")]
        crate::telemetry::lifecycle::record_process_monitored(caller_pid, target_pid, reference);

        Ok(MonitorResult {
            reference,
            immediate_down: false,
        })
    }

    fn demonitor(&self, caller_pid: u64, reference: u64) -> Result<(), SupervisionError> {
        let mut ms = lock_or_recover(&self.shared.monitor_set);

        // Get the monitor info before removing.
        let monitor = ms.get_monitor(reference);
        if let Some(monitor) = monitor {
            // Remove from both processes.
            if let Some(entry) = self.shared.process_bodies.get(&caller_pid) {
                let mut slot = lock_or_recover(&entry);
                if let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot {
                    process.remove_monitor(reference);
                }
            }
            if let Some(entry) = self.shared.process_bodies.get(&monitor.target()) {
                let mut slot = lock_or_recover(&entry);
                if let ProcessSlot::Present(ScheduledProcess(process)) = &mut *slot {
                    process.remove_monitor(reference);
                }
            }
            ms.remove_monitor(reference);
        }

        Ok(())
    }

    fn exit_signal(
        &self,
        _caller_pid: u64,
        target_pid: u64,
        reason: ExitReason,
    ) -> Result<(), SupervisionError> {
        let Some(_admission) = self.shared.try_reserve_teardown_admission() else {
            return Err(SupervisionError::NoProc);
        };
        // Deliver exit signal to target process.
        if let Some(entry) = self.shared.process_bodies.get(&target_pid) {
            let mut slot = lock_or_recover(&entry);
            match &mut *slot {
                ProcessSlot::Present(ScheduledProcess(target)) => {
                    if matches!(target.status(), ProcessStatus::Exited(_)) {
                        return Ok(());
                    }

                    let should_die = reason == ExitReason::Kill
                        || (reason != ExitReason::Normal && !target.trap_exit());

                    if should_die {
                        let terminal = link::terminal_reason(reason);
                        target.terminate(terminal);
                        drop(slot);
                        drop(entry);
                        cleanup_exited_process(&self.shared, target_pid, terminal);
                    } else if target.trap_exit() {
                        link::enqueue_exit_message_pub(target, _caller_pid, reason);
                        drop(slot);
                        drop(entry);
                        wake_process(&self.shared, target_pid);
                    }
                }
                ProcessSlot::Executing(metadata) => {
                    let should_die = reason == ExitReason::Kill
                        || (reason != ExitReason::Normal && !metadata.trap_exit);
                    if should_die {
                        let terminal = link::terminal_reason(reason);
                        shared_exit_tombstone(&self.shared, target_pid, terminal);
                    } else if metadata.trap_exit {
                        // Process traps exits: queue {EXIT, CallerPid, Reason} for
                        // delivery when the slice completes. This mirrors the Present
                        // arm's `else if target.trap_exit()` and MUST include NORMAL
                        // exits (OTP delivers `{'EXIT', Pid, normal}` for `erlang:exit/2`
                        // with reason `normal` to a trapping process). `should_die` has
                        // already peeled off Kill and abnormal-non-trapping cases, so
                        // reaching here means a trapping target for any non-kill reason.
                        metadata
                            .pending_exit_messages
                            .push((PendingExitSource::Local(_caller_pid), reason));
                        drop(slot);
                        drop(entry);
                        wake_process(&self.shared, target_pid);
                    }
                }
                ProcessSlot::Absent => {}
            }
        }
        Ok(())
    }
}
