//! ETS tables, created and owned through the scheduler's registry.

use super::{Scheduler, SharedState, supervision_integration};
use crate::ets::{EtsTable, EtsTableId, EtsTableMetadata};
use std::sync::Arc;

impl SharedState {
    pub(super) fn create_table(&self, metadata: EtsTableMetadata) -> EtsTableId {
        self.ets_registry.create_table(metadata)
    }

    pub(super) fn lookup_table(&self, id: EtsTableId) -> Option<Arc<dyn EtsTable>> {
        self.ets_registry.lookup_table(id)
    }

    pub(super) fn lookup_table_by_name(&self, name: crate::atom::Atom) -> Option<EtsTableId> {
        self.ets_registry.lookup_table_by_name(name)
    }

    pub(super) fn delete_table(&self, id: EtsTableId) -> bool {
        self.ets_registry.delete_table(id)
    }

    pub(super) fn transfer_or_delete_tables_owned_by(&self, owner: u64) -> usize {
        let before = self.ets_registry.table_count();
        let owned_ids = self.ets_registry.table_ids_owned_by(owner);
        for table_id in owned_ids {
            let Some(table) = self.ets_registry.lookup_table(table_id) else {
                continue;
            };
            let Some(heir) = &table.metadata().heir else {
                let _deleted = self.ets_registry.delete_table(table_id);
                continue;
            };
            if self.process_table.get(heir.pid).is_some()
                && supervision_integration::deliver_ets_transfer(
                    self,
                    heir.pid,
                    table_id,
                    owner,
                    heir.data.root(),
                    &self.atom_table,
                )
                && self.ets_registry.transfer_table_owner(table_id, heir.pid)
            {
                continue;
            }
            let _deleted = self.ets_registry.delete_table(table_id);
        }
        before.saturating_sub(self.ets_registry.table_count())
    }
}

impl Scheduler {
    /// Allocate and register an ETS table owned by a process.
    ///
    /// The provided metadata's `id` field is overwritten with the allocated,
    /// monotonically increasing table ID before the table is inserted.
    pub fn create_ets_table(&self, metadata: EtsTableMetadata) -> EtsTableId {
        self.shared.create_table(metadata)
    }

    /// Look up a registered ETS table by ID.
    pub fn lookup_ets_table(&self, id: EtsTableId) -> Option<Arc<dyn EtsTable>> {
        self.shared.lookup_table(id)
    }

    /// Look up a named ETS table by atom.
    pub fn lookup_ets_table_by_name(&self, name: crate::atom::Atom) -> Option<EtsTableId> {
        self.shared.lookup_table_by_name(name)
    }

    /// Delete a registered ETS table by ID.
    pub fn delete_ets_table(&self, id: EtsTableId) -> bool {
        self.shared.delete_table(id)
    }
}
