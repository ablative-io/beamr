//! The scheduler's `LocalSendFacility`: delivering a local message to a
//! process body.

use crate::scheduler::execution::wake_process;
use crate::scheduler::{ProcessSlot, SharedState, lock_or_recover};
use std::sync::Arc;

/// Scheduler-side implementation of [`LocalSendFacility`]: delivers a local
/// message to a target process body held in `process_bodies`, mirroring the
/// I/O delivery template (lock slot → Present/Executing/Absent → push before
/// wake) and the distribution-payload deferral for the Executing case.
pub(super) struct SchedulerLocalSendFacility {
    pub(super) shared: Arc<SharedState>,
}

impl crate::native::local_send::LocalSendFacility for SchedulerLocalSendFacility {
    fn send_local(
        &self,
        request: crate::native::local_send::LocalSendRequest<'_>,
    ) -> Result<(), crate::native::local_send::LocalSendError> {
        // Self-send from the BYTECODE path is handled in-hand by
        // `messaging::send` against the sender's own body and never reaches the
        // facility. A NATIVE self-send (NATIVE-001 R7), by contrast, is routed
        // here deliberately: the sender's slot is `Executing` during its slice,
        // so the message lands in `pending_local_messages` and is merged into
        // the mailbox at store-back (visible on the next slice) — exactly the
        // existing Executing-slot deferral, with no native-specific code path.
        let Some(entry) = self.shared.process_bodies.get(&request.target_pid) else {
            // Absent/dead pid: silent drop, matching BEAM semantics.
            return Ok(());
        };
        let mut slot = lock_or_recover(&entry);
        match &mut *slot {
            ProcessSlot::Present(process) => {
                // The Present branch is the ONLY place replay can occur for a
                // cross-process send: on a single replay thread no other pid can
                // be Executing, so the receiver is always Present (or Absent).
                let previous_receiver_clock = process.0.logical_clock();
                let receiver_clock = process.0.observe_message_clock(request.sender_clock);
                if let Some(driver) = request.replay_driver {
                    let mut guard = match driver.lock() {
                        Ok(guard) => guard,
                        Err(error) => error.into_inner(),
                    };
                    let recorded = match guard.next_message_delivery(
                        crate::replay::RecordedDeliveryKind::Message,
                        Some(request.sender_pid),
                        request.target_pid,
                        request.message,
                    ) {
                        Ok(recorded) => recorded,
                        Err(error) => {
                            process.0.set_logical_clock(previous_receiver_clock);
                            return Err(crate::native::local_send::LocalSendError::ReplayMismatch(
                                error.to_string(),
                            ));
                        }
                    };
                    if recorded.sender_clock != request.sender_clock
                        || recorded.receiver_clock != receiver_clock
                    {
                        process.0.set_logical_clock(previous_receiver_clock);
                        return Err(crate::native::local_send::LocalSendError::ReplayMismatch(
                            format!(
                                "message delivery clock mismatch: expected sender/receiver clocks ({}, {}), recorded ({}, {})",
                                request.sender_clock,
                                receiver_clock,
                                recorded.sender_clock,
                                recorded.receiver_clock
                            ),
                        ));
                    }
                }
                #[cfg(feature = "telemetry")]
                {
                    if process
                        .0
                        .mailbox()
                        .sender()
                        .send_traced(
                            request.sender_pid,
                            request.target_pid,
                            request.message,
                            process.0.heap_mut(),
                        )
                        .is_err()
                    {
                        // BEAM `!` cannot fail, so we still return Ok and drop the
                        // message — but a copy-into-mailbox failure (e.g. the
                        // receiver's young heap is full; beamr's bump allocator
                        // returns HeapFull rather than GCing) should never be
                        // silent. Surface it via telemetry. NOT a debug_assert:
                        // HeapFull is a legitimate runtime condition, not an
                        // invariant violation, so it must not crash debug builds.
                        crate::telemetry::metrics::record_message_dropped("mailbox_present");
                        return Ok(());
                    }
                }
                #[cfg(not(feature = "telemetry"))]
                {
                    if process
                        .0
                        .mailbox()
                        .sender()
                        .send(request.message, process.0.heap_mut())
                        .is_err()
                    {
                        // See the telemetry branch above: keep `!` infallible and
                        // drop on copy failure (HeapFull). No telemetry symbol in a
                        // non-telemetry build; no debug_assert because HeapFull is a
                        // legitimate runtime condition. Matches the I/O delivery
                        // template's silent-drop posture.
                        return Ok(());
                    }
                }
                // The scheduler's `wake_process` (below) requeues a parked
                // receiver and the slice machinery flips its status on resume —
                // exactly as the I/O delivery template does. The facility must
                // NOT force the status transition itself (doing so leaves a
                // parked process in an inconsistent Running state).
            }
            ProcessSlot::Executing(metadata) => {
                // Live-mode-only path: the receiver is mid-slice on another
                // thread, so we cannot touch its heap. ETF-encode the message
                // here so it survives the heap crossing, then decode it onto the
                // receiver heap at store-back (see scheduler/execution/core.rs).
                //
                // Clock note: delivering to an Executing receiver does NOT advance
                // the receiver's Lamport clock here. That is intentional and
                // consistent with the I/O and distribution deferred-delivery paths
                // (pending_io_messages / pending_distribution_payloads), which also
                // defer the receiver-side clock work. It is replay-safe because
                // replay is single-threaded: a receiver is never Executing during
                // replay, so this arm is only ever taken in live mode and never
                // contributes to the recorded delivery ordering.
                let payload =
                    match crate::etf::encode::encode_term(request.message, &self.shared.atom_table)
                    {
                        Ok(payload) => payload,
                        Err(_error) => {
                            // BEAM `!` cannot fail, so we keep returning Ok and drop
                            // the message rather than erroring the send. Surface the
                            // drop via telemetry so a codec gap is never an invisible
                            // message loss and is countable in prod.
                            //
                            // KNOWN LIMITATION (follow-up): `encode_term` rejects
                            // free-variable closures (`num_free != 0`), so a closure
                            // capturing variables sent to an *Executing* receiver is
                            // dropped here, even though the Present/in-hand path
                            // (`copy_term`) would deliver it faithfully. This is a
                            // pre-existing ETF asymmetry, not introduced by this fix.
                            // It is NOT a debug_assert: a free-var-closure send is
                            // legitimate user code, not an invariant violation, so it
                            // must not crash debug builds. Closing the gap requires
                            // symmetric closure ETF (or a heap-fragment copy for the
                            // Executing case) — tracked separately. References, the
                            // common actor case, DO round-trip (see etf/decode.rs).
                            #[cfg(feature = "telemetry")]
                            crate::telemetry::metrics::record_message_dropped("etf_encode");
                            return Ok(());
                        }
                    };
                metadata.pending_local_messages.push(payload);
            }
            ProcessSlot::Absent => return Ok(()),
        }
        drop(slot);
        drop(entry);
        wake_process(&self.shared, request.target_pid);
        Ok(())
    }
}
