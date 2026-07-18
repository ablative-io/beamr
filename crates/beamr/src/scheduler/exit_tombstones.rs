//! Bounded, insertion-ordered store of process exit tombstones.
//!
//! A *tombstone* records the [`ExitReason`] of a process that has died. It is
//! the load-bearing exit-detection signal: [`Scheduler::run_until_exit`] parks
//! on a condvar and only returns once it observes the dead pid's tombstone, and
//! [`Scheduler::peek_exit_reason`] / the link/monitor already-dead guards read
//! it to discover a process has gone.
//!
//! Historically this was an unbounded `DashMap<u64, ExitReason>`: a tombstone
//! was written on every process death and *never* removed for the lifetime of
//! the scheduler. Under a workload that spawns a fresh process per connection
//! (or per request), that map grows without bound — a slow but real leak.
//!
//! [`BoundedTombstones`] caps the live tombstone count at [`TOMBSTONE_CAPACITY`]
//! entries using a pure insertion-order (FIFO) eviction policy: when a new
//! tombstone would push the count past the cap, the *oldest* tombstone is
//! evicted. The cap is deliberately huge (64Ki entries, low single-digit MB)
//! so that eviction is invisible to every legitimate reader:
//!
//! * `run_until_exit` always targets a pid whose tombstone was *just* inserted
//!   to wake that very caller; FIFO eviction only reclaims the oldest entries
//!   once [`TOMBSTONE_CAPACITY`] *newer* exits have accumulated, which cannot
//!   happen inside the sub-10ms condvar wake window — so a blocked
//!   `run_until_exit` can never miss its real exit.
//! * `peek_exit_reason` and the link/monitor guards observe recently-dead pids
//!   in practice (a just-closed connection, never one buried 64Ki exits deep),
//!   so for them too the cap is effectively unreachable.
//!
//! The additive finalization ledger has a different retention contract: its
//! complete owned `(reason, term)` value is retained until consumed, even if the
//! legacy tombstone is evicted. Taking releases the owned term but deliberately
//! leaves a compact per-pid token for the scheduler lifetime. That token makes
//! outcome installation and event publication exactly-once across both outcome
//! consumption and tombstone eviction. Callers must drain outcomes to bound the
//! retained owned-term payload; the token ledger itself grows by one entry per
//! finalized pid and is the same map, not a second unbounded store. The legacy
//! result and diagnostic satellites remain bounded with the legacy tombstone,
//! preserving their existing semantics.

use super::exit_events::{ExitEvent, ExitEventPublisher, ExitEventSubscription};
use crate::ets::copy::OwnedTerm;
use crate::process::ExitReason;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::Mutex;

/// Maximum number of live exit tombstones retained at once.
///
/// At ~16 bytes per entry (a `u64` pid plus a `Copy` [`ExitReason`]) plus
/// DashMap overhead, 65536 entries caps the tombstone map at low single-digit
/// MB while leaving an enormous safety margin: a process that exited would have
/// to be followed by 65,536 *further* exits before its tombstone is reclaimed.
/// That dwarfs any plausible window of concurrently-interesting recently-dead
/// pids (a server with thousands of in-flight connections still has its
/// just-closed connection's tombstone well within the most-recent 64Ki), so the
/// FIFO eviction policy is effectively invisible to every legitimate reader
/// while still hard-bounding memory.
pub(super) const TOMBSTONE_CAPACITY: usize = 65_536;

/// Durable additive state for one process's first terminal transition.
///
/// `outcome` becomes `None` when taken, but the entry itself is the permanent
/// publication token. `reason` is therefore the authoritative additive reason
/// used by both the outcome and event even if a later cleanup overwrites the
/// bounded legacy tombstone's compatibility value.
struct FinalizedOutcome {
    reason: ExitReason,
    outcome: Option<OwnedTerm>,
}

/// A bounded, insertion-ordered concurrent map from pid to [`ExitReason`].
///
/// Reads are lock-free via the inner [`DashMap`] and preserve the exact
/// `Option`-returning semantics callers rely on (a miss returns `None`, same as
/// an unknown pid). Inserts additionally record the pid in a FIFO order queue
/// and, on overflow, evict the oldest pid — returning it so the caller can
/// evict the paired satellite entries.
pub(super) struct BoundedTombstones {
    reasons: DashMap<u64, ExitReason>,
    /// Complete take-once outcomes plus their durable publication tokens. The
    /// owned term is retained until consumed; the compact token remains for the
    /// scheduler lifetime and closes the eviction/consumption TOCTOU.
    outcomes: DashMap<u64, FinalizedOutcome>,
    /// Insertion order of currently-live pids, oldest at the front. Guarded
    /// independently of the DashMap shards; it also serializes writers so an
    /// complete outcome is always visible before its legacy tombstone.
    order: Mutex<VecDeque<u64>>,
    capacity: usize,
    events: ExitEventPublisher,
}

impl BoundedTombstones {
    /// Create a store with the default [`TOMBSTONE_CAPACITY`].
    pub(super) fn new() -> Self {
        Self::with_capacity(TOMBSTONE_CAPACITY)
    }

    /// Create a store with an explicit capacity. `capacity` must be non-zero;
    /// a zero capacity is clamped to 1 so the structure always stores at least
    /// the most recent tombstone.
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            reasons: DashMap::new(),
            outcomes: DashMap::new(),
            order: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
            events: ExitEventPublisher::new(),
        }
    }

    /// Read the exit reason for `pid`, or `None` if no tombstone is present.
    ///
    /// Lock-free and non-consuming: the tombstone is left in place. Takes the
    /// pid by reference to mirror the [`DashMap`] this replaced, keeping call
    /// sites unchanged.
    pub(super) fn get(&self, pid: &u64) -> Option<ExitReason> {
        self.reasons.get(pid).map(|entry| *entry)
    }

    /// Whether a tombstone exists for `pid`.
    pub(super) fn contains_key(&self, pid: &u64) -> bool {
        self.reasons.contains_key(pid)
    }

    /// Consume the complete retained outcome for `pid` exactly once.
    pub(super) fn take_outcome(&self, pid: &u64) -> Option<(ExitReason, OwnedTerm)> {
        let mut finalized = self.outcomes.get_mut(pid)?;
        let outcome = finalized.outcome.take()?;
        Some((finalized.reason, outcome))
    }

    /// Create the scheduler's sole exit-event subscription.
    pub(super) fn subscribe(&self) -> Option<ExitEventSubscription> {
        self.events.subscribe()
    }

    #[cfg(test)]
    pub(super) fn install_event_publication_gate(
        &self,
    ) -> super::exit_events::ExitEventPublicationObserver {
        self.events.install_publication_gate()
    }

    #[cfg(test)]
    pub(super) fn clear_event_publication_gate(&self) {
        self.events.clear_publication_gate();
    }

    /// Insert a legacy tombstone without publishing an additive outcome.
    ///
    /// Used by internal lifecycle tests that need to simulate an already-dead
    /// process. Production exits use [`Self::insert_outcome`].
    #[cfg(test)]
    pub(super) fn insert(&self, pid: u64, reason: ExitReason) -> Option<u64> {
        self.insert_inner(pid, reason, None)
    }

    /// Insert a tombstone together with a complete retained outcome.
    ///
    /// The first terminal caller atomically owns the durable additive token and
    /// publishes one retained outcome followed by one event. Later callers may
    /// preserve the historical legacy behavior by overwriting or restoring the
    /// bounded tombstone, but cannot change the authoritative additive reason,
    /// re-arm a consumed outcome, or publish another event. Consequently a
    /// legacy `get` after overlapping cleanup may report that later cleanup's
    /// compatibility reason; the additive outcome and event remain coherent.
    pub(super) fn insert_outcome(
        &self,
        pid: u64,
        reason: ExitReason,
        outcome: OwnedTerm,
    ) -> Option<u64> {
        self.insert_inner(pid, reason, Some(outcome))
    }

    fn insert_inner(
        &self,
        pid: u64,
        reason: ExitReason,
        outcome: Option<OwnedTerm>,
    ) -> Option<u64> {
        let mut order = match self.order.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if self.reasons.contains_key(&pid) {
            self.reasons.insert(pid, reason);
            return None;
        }

        // The writer mutex makes this durable-token check-and-install atomic
        // across competing terminal callers. Unlike the bounded legacy reason,
        // the token survives both eviction and outcome consumption.
        let publish_event = if let Some(outcome) = outcome {
            if self.outcomes.contains_key(&pid) {
                false
            } else {
                // Keep this order: a reader can never observe the legacy
                // tombstone before the exactly-once outcome, and the event
                // follows both.
                self.outcomes.insert(
                    pid,
                    FinalizedOutcome {
                        reason,
                        outcome: Some(outcome),
                    },
                );
                true
            }
        } else {
            false
        };
        self.reasons.insert(pid, reason);
        order.push_back(pid);
        let mut evicted = None;
        if order.len() > self.capacity {
            // Loop to skip any pid already removed from the legacy map. The
            // finalized entry deliberately remains; taking only releases its
            // owned term and never removes its publication token.
            while let Some(oldest) = order.pop_front() {
                if let Some((evicted_pid, _)) = self.reasons.remove(&oldest) {
                    evicted = Some(evicted_pid);
                    break;
                }
            }
        }
        drop(order);

        if publish_event {
            self.events.publish(ExitEvent::Exited { pid, reason });
        }
        evicted
    }

    /// Number of live tombstones. Test/diagnostic helper.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.reasons.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::Term;
    use std::sync::Barrier;
    use std::time::Duration;

    const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

    fn insert(store: &BoundedTombstones, pid: u64, reason: ExitReason) -> Option<u64> {
        store.insert_outcome(pid, reason, OwnedTerm::immediate(Term::NIL))
    }

    /// (a) Inserting far more than the cap keeps the live count bounded at the
    /// cap and never above it.
    #[test]
    fn insert_over_cap_stays_bounded() {
        let cap = 8;
        let store = BoundedTombstones::with_capacity(cap);
        for pid in 0..1_000u64 {
            insert(&store, pid, ExitReason::Normal);
            assert!(
                store.len() <= cap,
                "len {} exceeded cap {} after inserting pid {}",
                store.len(),
                cap,
                pid
            );
        }
        assert_eq!(store.len(), cap, "store settles exactly at the cap");
    }

    /// (b) The most-recent tombstones survive and read back their reason.
    #[test]
    fn most_recent_survive_and_are_readable() {
        let cap = 8;
        let store = BoundedTombstones::with_capacity(cap);
        for pid in 0..100u64 {
            // Vary the reason so we also confirm the right value comes back.
            let reason = if pid % 2 == 0 {
                ExitReason::Normal
            } else {
                ExitReason::Kill
            };
            insert(&store, pid, reason);
        }
        // The last `cap` pids (92..=99) must all be present with their reason.
        for pid in 92..100u64 {
            let expected = if pid % 2 == 0 {
                ExitReason::Normal
            } else {
                ExitReason::Kill
            };
            assert_eq!(
                store.get(&pid),
                Some(expected),
                "recent pid {pid} must survive with its reason"
            );
            assert!(store.contains_key(&pid));
        }
    }

    /// (c) The oldest tombstones are evicted — `get` returns `None` for them —
    /// while recent ones still return `Some`, preserving exact Option
    /// semantics (a miss is indistinguishable from an unknown pid).
    #[test]
    fn oldest_are_evicted_recent_retained() {
        let cap = 4;
        let store = BoundedTombstones::with_capacity(cap);
        for pid in 0..10u64 {
            insert(&store, pid, ExitReason::Normal);
        }
        // Oldest 6 (0..=5) evicted.
        for pid in 0..6u64 {
            assert_eq!(store.get(&pid), None, "old pid {pid} must be evicted");
            assert!(!store.contains_key(&pid));
        }
        // Newest 4 (6..=9) retained.
        for pid in 6..10u64 {
            assert_eq!(
                store.get(&pid),
                Some(ExitReason::Normal),
                "recent pid {pid} must be retained"
            );
        }
    }

    /// A re-insert (overwrite) of a live pid must not duplicate it in the FIFO
    /// order, must update the reason, and must not evict a different live pid.
    #[test]
    fn overwrite_does_not_duplicate_or_misevict() {
        let cap = 3;
        let store = BoundedTombstones::with_capacity(cap);
        insert(&store, 1, ExitReason::Normal);
        insert(&store, 2, ExitReason::Normal);
        insert(&store, 3, ExitReason::Normal);
        // Overwrite the oldest; reason updates, order is unchanged.
        insert(&store, 1, ExitReason::Kill);
        assert_eq!(store.get(&1), Some(ExitReason::Kill));
        assert_eq!(store.len(), cap);
        // Next fresh insert evicts pid 1 (still the oldest by first-insert
        // order), not pid 2 or 3.
        let insertion = insert(&store, 4, ExitReason::Normal);
        assert_eq!(insertion, Some(1));
        assert_eq!(store.get(&1), None, "first-inserted pid is the one evicted");
        assert_eq!(store.get(&2), Some(ExitReason::Normal));
        assert_eq!(store.get(&3), Some(ExitReason::Normal));
        assert_eq!(store.get(&4), Some(ExitReason::Normal));
        assert_eq!(store.len(), cap);
        let (reason, _term) = store
            .take_outcome(&1)
            .expect("legacy overwrite and eviction leave outcome retained");
        assert_eq!(reason, ExitReason::Normal);
        assert!(store.take_outcome(&1).is_none(), "outcome is take-once");
    }

    #[test]
    fn duplicate_after_eviction_preserves_original_untaken_outcome_and_emits_no_event() {
        let store = BoundedTombstones::with_capacity(2);
        let subscription = store.subscribe().expect("first subscriber");

        store.insert_outcome(
            1,
            ExitReason::Normal,
            OwnedTerm::immediate(Term::small_int(11)),
        );
        assert_eq!(
            subscription.recv_timeout(EVENT_TIMEOUT),
            Ok(ExitEvent::Exited {
                pid: 1,
                reason: ExitReason::Normal,
            })
        );
        for pid in 2..=3 {
            store.insert_outcome(
                pid,
                ExitReason::Normal,
                OwnedTerm::immediate(Term::small_int(pid as i64)),
            );
            match subscription.recv_timeout(EVENT_TIMEOUT) {
                Ok(ExitEvent::Exited { pid: event_pid, .. }) if event_pid == pid => {}
                other => {
                    panic!("expected exit event for pid {pid}, got {other:?}")
                }
            }
            assert!(store.take_outcome(&pid).is_some());
        }
        assert_eq!(store.get(&1), None, "pid 1 tombstone was evicted");

        store.insert_outcome(
            1,
            ExitReason::Kill,
            OwnedTerm::immediate(Term::small_int(99)),
        );

        let (reason, outcome) = store
            .take_outcome(&1)
            .expect("first terminal transition remains takeable");
        assert_eq!(reason, ExitReason::Normal);
        assert_eq!(outcome.root().as_small_int(), Some(11));
        assert_eq!(
            subscription.recv_timeout(Duration::ZERO),
            Err(super::super::ExitEventRecvError::Timeout),
            "duplicate finalization cannot emit another event"
        );
    }

    #[test]
    fn concurrent_terminal_callers_publish_one_authoritative_outcome_and_event() {
        let store = BoundedTombstones::with_capacity(4);
        let subscription = store.subscribe().expect("first subscriber");
        let start = Barrier::new(3);

        std::thread::scope(|scope| {
            let normal = scope.spawn(|| {
                start.wait();
                store.insert_outcome(
                    1,
                    ExitReason::Normal,
                    OwnedTerm::immediate(Term::small_int(10)),
                );
            });
            let killed = scope.spawn(|| {
                start.wait();
                store.insert_outcome(
                    1,
                    ExitReason::Kill,
                    OwnedTerm::immediate(Term::small_int(20)),
                );
            });
            start.wait();
            normal.join().expect("normal finalizer completes");
            killed.join().expect("kill finalizer completes");
        });

        let (event_reason, expected_value) = match subscription.recv_timeout(EVENT_TIMEOUT) {
            Ok(ExitEvent::Exited { pid: 1, reason }) => match reason {
                ExitReason::Normal => (reason, 10),
                ExitReason::Kill => (reason, 20),
                other => panic!("unexpected authoritative reason {other:?}"),
            },
            other => panic!("expected one exit event, got {other:?}"),
        };
        let (outcome_reason, outcome) = store
            .take_outcome(&1)
            .expect("authoritative outcome is installed once");
        assert_eq!(outcome_reason, event_reason);
        assert_eq!(outcome.root().as_small_int(), Some(expected_value));
        assert!(
            store.take_outcome(&1).is_none(),
            "the losing finalizer cannot install another outcome"
        );
        assert_eq!(
            subscription.recv_timeout(Duration::ZERO),
            Err(super::super::ExitEventRecvError::Timeout),
            "the losing finalizer cannot emit an event"
        );
        let later_legacy_reason = match event_reason {
            ExitReason::Normal => ExitReason::Kill,
            ExitReason::Kill => ExitReason::Normal,
            _ => unreachable!("event reason was restricted above"),
        };
        assert_eq!(
            store.get(&1),
            Some(later_legacy_reason),
            "legacy overwrite is compatibility-only; the additive reason is authoritative"
        );
    }
}
