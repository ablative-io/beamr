//! Sending to remote processes: the scheduler's
//! `DistributionSendFacility`, its control delivery, and the blocking send
//! with a write deadline.

use crate::atom::Atom;
use crate::distribution::control::{
    ControlDelivery, ControlRegistry, DistributionSendError, DistributionSendFacility,
    encode_send_frame,
};
use crate::scheduler::execution::wake_process;
use crate::scheduler::{ProcessSlot, SharedState, lock_or_recover};
use crate::term::Term;
use crate::term::pid_ref::PidRef;
use std::sync::Arc;

pub(in crate::scheduler) struct SchedulerDistributionSendFacility {
    pub(in crate::scheduler) shared: Arc<SharedState>,
}

impl DistributionSendFacility for SchedulerDistributionSendFacility {
    fn send_remote(&self, target: Term, message: Term) -> Result<(), DistributionSendError> {
        let pid = PidRef::new(target).ok_or(DistributionSendError::Encode)?;
        let node = pid.node().ok_or(DistributionSendError::Encode)?;
        let node_name = self
            .shared
            .atom_table
            .resolve(node)
            .ok_or(DistributionSendError::NoConnection)?
            .to_owned();
        let frame = encode_send_frame(
            Term::atom(Atom::OK),
            target,
            message,
            &self.shared.atom_table,
        )
        .map_err(|_| DistributionSendError::Encode)?;
        // This facility is offered only when the bundle is Owned, but route the
        // manager access through it so absence is `noconnection`, never a panic.
        let dist = self
            .shared
            .distribution()
            .ok_or(DistributionSendError::NoConnection)?;
        block_on_distribution_send(dist.connections(), node, &node_name, &frame)
    }
}

pub(in crate::scheduler) fn block_on_distribution_send(
    manager: &crate::distribution::connection::ConnectionManager,
    node: Atom,
    node_name: &str,
    frame: &[u8],
) -> Result<(), DistributionSendError> {
    block_on_distribution_send_with_write_deadline(
        manager,
        node,
        node_name,
        frame,
        crate::distribution::sender::WRITE_TIMEOUT,
    )
}

pub(in crate::scheduler) fn block_on_distribution_send_with_write_deadline(
    manager: &crate::distribution::connection::ConnectionManager,
    node: Atom,
    node_name: &str,
    frame: &[u8],
    write_deadline: std::time::Duration,
) -> Result<(), DistributionSendError> {
    let manager = manager.clone();
    let node_name = node_name.to_owned();
    let frame = frame.to_vec();
    let future = async move {
        let connection = match manager.get_connection(node) {
            Some(connection) => connection,
            None => manager
                .connect(&node_name)
                .await
                .map_err(|_| DistributionSendError::NoConnection)?,
        };
        // The write is BOUNDED with the same deadline the sender drain uses:
        // this path blocks a scheduler thread synchronously, and a peer that
        // is TCP-connected but never reads would otherwise park it
        // indefinitely — no other machinery can end the wait (the heartbeat's
        // own writes queue behind this same writer mutex, and inbound
        // keepalives keep inbound liveness fresh while outbound is wedged).
        // On elapse the connection is retired through the same
        // mark_down_write_timeout path the drain uses, whose socket shutdown
        // also unblocks the erroring write future being dropped here.
        match tokio::time::timeout(write_deadline, connection.write_raw(&frame)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(DistributionSendError::NoConnection),
            Err(_elapsed) => {
                connection.mark_down_write_timeout();
                Err(DistributionSendError::NoConnection)
            }
        }
    };
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if matches!(
            handle.runtime_flavor(),
            tokio::runtime::RuntimeFlavor::MultiThread
        ) {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                    .map_err(|_| DistributionSendError::NoConnection)?
                    .block_on(future)
            })
            .join()
            .map_err(|_| DistributionSendError::NoConnection)?
        }
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| DistributionSendError::NoConnection)?
            .block_on(future)
    }
}

impl ControlDelivery for SchedulerDistributionSendFacility {
    fn deliver_payload(&self, target_pid: u64, payload_etf: &[u8]) -> bool {
        let Some(entry) = self.shared.process_bodies.get(&target_pid) else {
            return false;
        };
        let mut slot = lock_or_recover(&entry);
        match &mut *slot {
            ProcessSlot::Present(process) => {
                let mut context = crate::native::ProcessContext::new();
                context.attach_process(&mut process.0, 0);
                let Ok(message) = crate::etf::decode::decode_term(
                    payload_etf,
                    &mut context,
                    &self.shared.atom_table,
                ) else {
                    return false;
                };
                process.0.mailbox_mut().push_owned(message);
            }
            ProcessSlot::Executing(metadata) => {
                metadata
                    .pending_distribution_payloads
                    .push(payload_etf.to_vec());
            }
            ProcessSlot::Absent => return false,
        }
        drop(slot);
        drop(entry);
        wake_process(&self.shared, target_pid);
        true
    }
}

impl ControlRegistry for SchedulerDistributionSendFacility {
    fn whereis(&self, name: Atom) -> Option<u64> {
        self.shared.process_registry.get(&name).map(|entry| *entry)
    }
}

impl crate::distribution::control::PgDelivery for SchedulerDistributionSendFacility {
    fn apply_pg_join(&self, scope: Atom, group: Atom, node: Atom, pid_number: u64, serial: u64) {
        self.shared
            .pg_registry
            .apply_remote_join(scope, group, node, pid_number, serial);
    }

    fn apply_pg_leave(&self, scope: Atom, group: Atom, node: Atom, pid_number: u64, serial: u64) {
        self.shared
            .pg_registry
            .apply_remote_leave(scope, group, node, pid_number, serial);
    }
}
