//! Bounded, single-subscriber process-exit event delivery, plus the
//! notification-only per-pid one-shot exit watches (EXIT-001).

use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};
use dashmap::DashMap;

use crate::process::ExitReason;

/// Maximum number of exit notifications buffered for the subscriber.
///
/// When the subscriber falls behind this bound, the queue remains bounded and
/// [`ExitEvent::Lagged`] reports that one or more notifications were not queued.
pub const EXIT_EVENT_CAPACITY: usize = 1_024;

/// A notification delivered by an [`ExitEventSubscription`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExitEvent {
    /// A process exited. Its outcome was published before this event, so an
    /// immediate `Scheduler::take_exit_outcome(pid)` can consume it.
    Exited {
        /// Exited process identifier.
        pid: u64,
        /// Process exit reason.
        reason: ExitReason,
    },
    /// At least one exit notification could not fit in the bounded queue.
    ///
    /// No outcome is discarded: pending notifications are reset when this is
    /// observed, and consumers can recover by calling
    /// `Scheduler::take_exit_outcome` for the process identifiers they track.
    /// Multiple overflows may be coalesced into one marker.
    Lagged,
}

/// Failure while waiting for the next exit event.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExitEventRecvError {
    /// The scheduler and its event publisher were dropped.
    Disconnected,
    /// No event arrived before the requested timeout.
    Timeout,
}

impl std::fmt::Display for ExitEventRecvError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Disconnected => "exit-event publisher disconnected",
            Self::Timeout => "timed out waiting for an exit event",
        })
    }
}

impl std::error::Error for ExitEventRecvError {}

/// The receiving handle for a scheduler's bounded exit-event stream.
///
/// A scheduler permits one subscription for its lifetime. The handle blocks on
/// the channel rather than polling, and can be shared between threads if the
/// consumer wants to move the single draining responsibility.
pub struct ExitEventSubscription {
    receiver: Receiver<ExitEvent>,
    overflowed: Arc<AtomicBool>,
}

impl ExitEventSubscription {
    /// Block until an exit event, overflow marker, or disconnection is observed.
    pub fn recv(&self) -> Result<ExitEvent, ExitEventRecvError> {
        if self.take_lag_marker() {
            return Ok(ExitEvent::Lagged);
        }
        self.receiver
            .recv()
            .map_err(|_| ExitEventRecvError::Disconnected)
    }

    /// Wait up to `timeout` for an exit event or overflow marker.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ExitEvent, ExitEventRecvError> {
        if self.take_lag_marker() {
            return Ok(ExitEvent::Lagged);
        }
        self.receiver
            .recv_timeout(timeout)
            .map_err(|error| match error {
                RecvTimeoutError::Timeout => ExitEventRecvError::Timeout,
                RecvTimeoutError::Disconnected => ExitEventRecvError::Disconnected,
            })
    }

    fn take_lag_marker(&self) -> bool {
        if !self.overflowed.swap(false, Ordering::AcqRel) {
            return false;
        }
        // Events already in the queue belong to the lagged batch. Discard only
        // the bounded snapshot currently present; a concurrent later publish is
        // left for the next receive and never turns this into a polling loop.
        for _ in 0..self.receiver.len() {
            let _ = self.receiver.try_recv();
        }
        true
    }
}

/// The registration-time answer from `Scheduler::watch_exit`.
///
/// Typed rather than watch-always (OQ-B): registration itself distinguishes
/// the three states a caller must never confuse, so an embedder can skip
/// arming a deadline against a pid that is already dead and can never block
/// forever on a pid that has no record.
#[derive(Debug)]
pub enum ExitWatchState {
    /// The process had no terminal record at registration and was present in
    /// the process table: the watch is armed and will fire exactly once when
    /// the process exits.
    Live(ExitWatch),
    /// The process already finalized. The authoritative additive reason is
    /// returned immediately from the durable outcome record (which survives
    /// both legacy-tombstone eviction and outcome consumption); no watch is
    /// armed and nothing further will fire.
    AlreadyExited(ExitReason),
    /// No live process and no durable record: the pid never ran under this
    /// scheduler (or predates the additive ledger). No watch is armed —
    /// nothing can ever fire for it — so the caller is never left blocking
    /// on an answer that cannot arrive.
    NoRecord,
}

/// A one-shot, notification-only handle for a single process's exit.
///
/// Contract, in the naming style of the connection-event hub:
///
/// * **Notification-only.** A watch carries `(pid, reason)` by value and
///   never consumes, inspects, or perturbs the retained outcome:
///   `Scheduler::take_exit_outcome` stays exactly-once for whoever owns the
///   draining subscription.
/// * **The exclusive subscription is unaffected.** `subscribe_exit_events`
///   keeps its one-per-scheduler-lifetime slot whether zero or thousands of
///   watches exist.
/// * **One-shot with duplicate absorption.** Registration happens before the
///   already-dead check (register-then-check); if a concurrent fire and the
///   retained-record answer both arrive, they land in the same single slot
///   and the second is a no-op by design.
/// * **Deregistration on drop.** Dropping an unfired watch removes its
///   registration; abandoned watches cannot accumulate.
pub struct ExitWatch {
    pid: u64,
    watch_id: u64,
    receiver: Receiver<(u64, ExitReason)>,
    registry: Arc<ExitWatchRegistry>,
}

impl std::fmt::Debug for ExitWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExitWatch")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl ExitWatch {
    /// The pid this watch is armed for.
    #[must_use]
    pub fn pid(&self) -> u64 {
        self.pid
    }

    /// Block until the watched process's exit notification arrives.
    pub fn recv(&self) -> Result<(u64, ExitReason), ExitEventRecvError> {
        self.receiver
            .recv()
            .map_err(|_| ExitEventRecvError::Disconnected)
    }

    /// Wait up to `timeout` for the watched process's exit notification.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<(u64, ExitReason), ExitEventRecvError> {
        self.receiver
            .recv_timeout(timeout)
            .map_err(|error| match error {
                RecvTimeoutError::Timeout => ExitEventRecvError::Timeout,
                RecvTimeoutError::Disconnected => ExitEventRecvError::Disconnected,
            })
    }
}

impl Drop for ExitWatch {
    fn drop(&mut self) {
        self.registry.deregister(self.pid, self.watch_id);
    }
}

/// One armed watch slot: the watch id plus its one-shot `(pid, reason)` sender.
type WatchSlot = (u64, Sender<(u64, ExitReason)>);

/// pid-keyed registry of one-shot notification slots (EXIT-001 R2).
///
/// Registration and deregistration touch only the sharded map — never the
/// tombstone writer mutex — and the fire path's cost grows only with the
/// number of watches on the exiting pid, not with watches in the system.
pub(super) struct ExitWatchRegistry {
    watches: DashMap<u64, Vec<WatchSlot>>,
    next_watch_id: AtomicU64,
}

/// Store-level registration answer: the typed `NoRecord`/liveness split is
/// composed at the `Scheduler` layer, which owns the process table.
pub(super) enum StoreWatch {
    /// No durable record existed at the check: the slot stays armed.
    Armed(ExitWatch),
    /// The durable record already existed: reported immediately, the
    /// just-registered slot deregistered (duplicate fires are absorbed).
    AlreadyExited(ExitReason),
}

impl ExitWatchRegistry {
    pub(super) fn new() -> Self {
        Self {
            watches: DashMap::new(),
            next_watch_id: AtomicU64::new(0),
        }
    }

    /// Arm a one-shot slot for `pid`. Registration only — the caller composes
    /// register-then-check on top (see `BoundedTombstones::watch`).
    pub(super) fn register(self: &Arc<Self>, pid: u64) -> ExitWatch {
        let watch_id = self.next_watch_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = crossbeam_channel::bounded(1);
        self.watches
            .entry(pid)
            .or_default()
            .push((watch_id, sender));
        ExitWatch {
            pid,
            watch_id,
            receiver,
            registry: Arc::clone(self),
        }
    }

    /// Deliver the exit notification to every watch on `pid` and clear the
    /// entry. Non-blocking sends into one-slot channels; no user code runs;
    /// nothing allocates on the steady no-watchers path (the map miss is the
    /// whole cost). Cost grows only with the watches on THIS pid.
    pub(super) fn fire(&self, pid: u64, reason: ExitReason) {
        let Some((_pid, watchers)) = self.watches.remove(&pid) else {
            return;
        };
        for (_watch_id, sender) in watchers {
            // A full slot (the register-then-check answer already landed) or
            // a dropped receiver (the watcher gave up) are both benign: the
            // one-shot slot absorbs duplicates by design, and send-and-drop
            // keeps the fire path non-blocking.
            let _ = sender.try_send((pid, reason));
        }
    }

    fn deregister(&self, pid: u64, watch_id: u64) {
        if let dashmap::mapref::entry::Entry::Occupied(mut entry) = self.watches.entry(pid) {
            entry.get_mut().retain(|(id, _)| *id != watch_id);
            if entry.get().is_empty() {
                entry.remove();
            }
        }
    }

    /// Number of pids with at least one live watch. Test/diagnostic helper in
    /// the style of `BoundedTombstones::len`.
    #[cfg(test)]
    pub(super) fn watched_pid_count(&self) -> usize {
        self.watches.len()
    }
}

#[cfg(test)]
#[derive(Clone)]
struct ExitEventPublicationGate {
    published: Sender<()>,
    observed: Receiver<()>,
}

#[cfg(test)]
pub(super) struct ExitEventPublicationObserver {
    published: Receiver<()>,
    observed: Sender<()>,
}

pub(super) struct ExitEventPublisher {
    sender: OnceLock<Sender<ExitEvent>>,
    overflowed: Arc<AtomicBool>,
    capacity: usize,
    #[cfg(test)]
    publication_gate: Mutex<Option<ExitEventPublicationGate>>,
}

impl ExitEventPublisher {
    pub(super) fn new() -> Self {
        Self::with_capacity(EXIT_EVENT_CAPACITY)
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            sender: OnceLock::new(),
            overflowed: Arc::new(AtomicBool::new(false)),
            capacity: capacity.max(1),
            #[cfg(test)]
            publication_gate: Mutex::new(None),
        }
    }

    pub(super) fn subscribe(&self) -> Option<ExitEventSubscription> {
        let (sender, receiver) = crossbeam_channel::bounded(self.capacity);
        self.sender.set(sender).ok()?;
        Some(ExitEventSubscription {
            receiver,
            overflowed: Arc::clone(&self.overflowed),
        })
    }

    pub(super) fn publish(&self, event: ExitEvent) {
        let Some(sender) = self.sender.get() else {
            return;
        };
        match sender.try_send(event) {
            Ok(()) => {
                #[cfg(test)]
                self.wait_at_publication_gate();
            }
            Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => self.overflowed.store(true, Ordering::Release),
        }
    }

    /// Install a zero-capacity, post-send rendezvous for one test phase.
    ///
    /// A successful publisher cannot return from [`Self::publish`] until the
    /// observer confirms that it received the event. This is deliberately
    /// after `try_send`: outcome installation and event publication retain
    /// their production order, while the observer can prove it contested the
    /// actual publication call rather than merely running before it.
    #[cfg(test)]
    pub(super) fn install_publication_gate(&self) -> ExitEventPublicationObserver {
        let (published, observe_publication) = crossbeam_channel::bounded(0);
        let (observation_complete, observed) = crossbeam_channel::bounded(0);
        let mut publication_gate = match self.publication_gate.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *publication_gate = Some(ExitEventPublicationGate {
            published,
            observed,
        });
        ExitEventPublicationObserver {
            published: observe_publication,
            observed: observation_complete,
        }
    }

    #[cfg(test)]
    pub(super) fn clear_publication_gate(&self) {
        let mut publication_gate = match self.publication_gate.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *publication_gate = None;
    }

    #[cfg(test)]
    fn wait_at_publication_gate(&self) {
        let gate = match self.publication_gate.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let Some(gate) = gate else {
            return;
        };
        if gate.published.send(()).is_ok() {
            // Disconnection means the observer failed and is unwinding; do not
            // turn its finite receive timeout into a stuck publisher thread.
            let _ = gate.observed.recv();
        }
    }
}

#[cfg(test)]
impl ExitEventPublicationObserver {
    pub(super) fn acknowledge_observed(&self, timeout: Duration) {
        self.wait_for_publication(timeout);
        self.release_publication(timeout);
    }

    /// Observe that a publisher reached the post-send gate WITHOUT releasing
    /// it, so a test can interleave work (e.g. registering a watch) with an
    /// in-flight publication deterministically. Pair with
    /// [`Self::release_publication`].
    pub(super) fn wait_for_publication(&self, timeout: Duration) {
        self.published
            .recv_timeout(timeout)
            .expect("event publisher must reach the post-send gate");
    }

    /// Release a publisher previously observed at the post-send gate.
    pub(super) fn release_publication(&self, timeout: Duration) {
        self.observed
            .send_timeout((), timeout)
            .expect("event publisher must remain at the post-send gate");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R4 — the exclusivity MECHANISM is pinned, not just the behaviour: the
    /// single-subscription guarantee rests on `OnceLock`'s set-once semantics.
    /// A "small refactor" to `Mutex<Option<Sender>>` could quietly permit
    /// re-subscription while every behavioural test still passes at first —
    /// this source check (the house prohibition-grep shape) catches it.
    #[test]
    fn exclusivity_mechanism_is_still_the_oncelock() {
        let source = include_str!("exit_events.rs");
        assert!(
            source.contains("sender: OnceLock<Sender<ExitEvent>>"),
            "the exclusive subscription must keep its OnceLock mechanism; \
             changing it re-opens aion's singleton-drainer contract and is a \
             STOP, not a refactor"
        );
    }

    #[test]
    fn overflow_is_typed_and_queue_stays_bounded() {
        let publisher = ExitEventPublisher::with_capacity(2);
        let subscription = publisher.subscribe().expect("first subscriber");
        for pid in 1..=3 {
            publisher.publish(ExitEvent::Exited {
                pid,
                reason: ExitReason::Normal,
            });
        }

        assert_eq!(subscription.recv(), Ok(ExitEvent::Lagged));
        assert!(
            publisher.subscribe().is_none(),
            "subscription is single-use"
        );
        assert_eq!(
            subscription.recv_timeout(Duration::ZERO),
            Err(ExitEventRecvError::Timeout),
            "lag resets the queued batch before recovery"
        );
        publisher.publish(ExitEvent::Exited {
            pid: 4,
            reason: ExitReason::Normal,
        });
        assert_eq!(
            subscription.recv(),
            Ok(ExitEvent::Exited {
                pid: 4,
                reason: ExitReason::Normal,
            })
        );
    }
}
