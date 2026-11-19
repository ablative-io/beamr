//! ETS through the scheduler: delivering an `ETS-TRANSFER` message to a
//! table's new owner, and the scheduler's `EtsFacility`.

use crate::atom::Atom;
use crate::ets::{EtsError, EtsTable, EtsTableId, EtsTableMetadata};
use crate::native::ets_bifs::EtsFacility;
use crate::process::Process;
use crate::scheduler::execution::wake_process;
use crate::scheduler::{ProcessSlot, ScheduledProcess, SharedState, lock_or_recover};
use crate::term::Term;
use crate::term::boxed;
use std::sync::Arc;

pub(in crate::scheduler) fn deliver_ets_transfer(
    shared: &SharedState,
    recipient_pid: u64,
    table_id: EtsTableId,
    from_pid: u64,
    data: Term,
    atom_table: &crate::atom::AtomTable,
) -> bool {
    let Some(entry) = shared.process_bodies.get(&recipient_pid) else {
        return false;
    };
    let transfer_atom = atom_table.intern("ETS-TRANSFER");
    let mut slot = lock_or_recover(&entry);
    let delivered = match &mut *slot {
        ProcessSlot::Present(ScheduledProcess(process)) => {
            let Some(message) =
                build_ets_transfer_message(process, transfer_atom, table_id, from_pid, data)
            else {
                return false;
            };
            process.mailbox_mut().push_owned(message);
            true
        }
        ProcessSlot::Executing(metadata) => {
            let Ok(data) = crate::ets::copy_term_to_ets(data) else {
                return false;
            };
            metadata.pending_ets_transfer_messages.push(
                crate::scheduler::process_slot::PendingEtsTransferMessage {
                    table_id,
                    from_pid,
                    data,
                },
            );
            true
        }
        ProcessSlot::Absent => false,
    };
    drop(slot);
    if delivered {
        wake_process(shared, recipient_pid);
    }
    delivered
}

pub(in crate::scheduler) fn build_ets_transfer_message(
    process: &mut Process,
    transfer_atom: Atom,
    table_id: EtsTableId,
    from_pid: u64,
    data: Term,
) -> Option<Term> {
    let table = Term::try_small_int(i64::try_from(table_id).ok()?)?;
    let from = Term::try_pid(from_pid)?;
    let data = crate::ets::copy_term_to_heap(data, process.heap_mut()).ok()?;
    let words = process.heap_mut().alloc_slice(5).ok()?;
    boxed::write_tuple(words, &[Term::atom(transfer_atom), table, from, data])
}

pub(super) struct SchedulerEtsFacility {
    pub(super) shared: Arc<SharedState>,
}

impl EtsFacility for SchedulerEtsFacility {
    fn create_table(&self, metadata: EtsTableMetadata) -> Result<EtsTableId, EtsError> {
        let Some(_admission) = self.shared.try_reserve_teardown_admission() else {
            return Err(EtsError::Badarg);
        };
        self.shared.ets_registry.try_create_table(metadata)
    }

    fn lookup_table(&self, id: EtsTableId) -> Option<Arc<dyn EtsTable>> {
        self.shared.ets_registry.lookup_table(id)
    }

    fn lookup_named_table(&self, name: Atom) -> Option<Arc<dyn EtsTable>> {
        self.shared.ets_registry.lookup_named_table(name)
    }

    fn lookup_table_by_name(&self, name: Atom) -> Option<EtsTableId> {
        self.shared.ets_registry.lookup_table_by_name(name)
    }

    fn delete_table(&self, id: EtsTableId) -> bool {
        let Some(_admission) = self.shared.try_reserve_teardown_admission() else {
            return false;
        };
        self.shared.ets_registry.delete_table(id)
    }

    fn give_away_table(
        &self,
        table_id: EtsTableId,
        new_owner: u64,
        from_pid: u64,
        gift_data: Term,
        atom_table: &crate::atom::AtomTable,
    ) -> Result<(), EtsError> {
        let Some(_admission) = self.shared.try_reserve_teardown_admission() else {
            return Err(EtsError::Badarg);
        };
        if !deliver_ets_transfer(
            &self.shared,
            new_owner,
            table_id,
            from_pid,
            gift_data,
            atom_table,
        ) {
            return Err(EtsError::Badarg);
        }
        if self
            .shared
            .ets_registry
            .transfer_table_owner(table_id, new_owner)
        {
            Ok(())
        } else {
            Err(EtsError::Badarg)
        }
    }
}
