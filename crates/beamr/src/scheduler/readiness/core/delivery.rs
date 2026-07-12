use super::*;

impl ReadinessCore {
    pub(super) fn deliver_event(&self, event: &mio::event::Event) -> std::io::Result<()> {
        let (slot_index, generation_low) = ReadinessToken::decode(event.token());
        let error_or_hup = event.is_error() || event.is_read_closed() || event.is_write_closed();
        self.deliver_decoded(
            slot_index,
            generation_low,
            event.is_readable(),
            event.is_writable(),
            error_or_hup,
        )
    }

    fn deliver_decoded(
        &self,
        slot_index: u32,
        generation_low: u32,
        readable: bool,
        writable: bool,
        error_or_hup: bool,
    ) -> std::io::Result<()> {
        let delivery = {
            let mut table = match self.table.lock() {
                Ok(table) => table,
                Err(_) => return Err(std::io::Error::other("readiness table poisoned")),
            };
            #[cfg(test)]
            if self.panic_in_delivery.swap(false, Ordering::AcqRel) {
                panic!("readiness delivery critical-section seam");
            }
            let Some(record) = table
                .slots
                .get_mut(slot_index as usize)
                .and_then(|slot| slot.record.as_mut())
            else {
                return Ok(());
            };
            if record.state != RecordState::Live
                || record.generation.0 as u32 != generation_low
                || record.armed.0 == 0
            {
                return Ok(());
            }
            let triggered = if error_or_hup {
                record.armed
            } else {
                Interest(
                    ((u8::from(readable) * Interest::READABLE.0)
                        | (u8::from(writable) * Interest::WRITABLE.0))
                        & record.armed.0,
                )
            };
            if triggered.0 == 0 {
                return Ok(());
            }
            let Some(scheduler) = record.route.scheduler.upgrade() else {
                return Ok(());
            };
            let pid = record.pid;
            let marker = record.marker;
            record.armed = Interest(record.armed.0 & !triggered.0);
            Some((scheduler, pid, marker))
        };
        if let Some((scheduler, pid, marker)) = delivery {
            scheduler.deliver_readiness_marker(pid, marker);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::scheduler::readiness) fn inject_stale_readable(&self, token: ReadinessToken) {
        let _ = self.deliver_decoded(token.slot, token.generation.0 as u32, true, false, false);
    }
}
