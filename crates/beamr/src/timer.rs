//! Timer wheel for one-shot timeouts.
//!
//! The wheel uses millisecond ticks, bucketed timer references, and an index map.
//! Scheduling and cancellation update only one bucket/index entry each, giving
//! O(1) insertion and cancellation without a sorted list, binary heap, or
//! priority queue.

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use web_time::Instant;

use crate::term::Term;

const DEFAULT_BUCKETS: usize = 1024;

/// Unique timer reference used for cancellation.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TimerRef(u64);

impl TimerRef {
    /// Return the opaque integer id backing this reference.
    #[must_use]
    pub const fn id(self) -> u64 {
        self.0
    }

    /// Reconstruct a timer reference from an id term/payload.
    #[must_use]
    pub const fn from_id(id: u64) -> Self {
        Self(id)
    }
}

/// What the scheduler does with a timer when it fires.
///
/// The wheel itself is agnostic to this distinction — it stores the kind and
/// echoes it back on the [`ExpiredTimer`] so the scheduler's `expire_timers`
/// can route each fired timer to the correct path: a receive-timeout
/// code-position jump, or a real mailbox delivery of `message`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TimerKind {
    /// Backs `receive ... after`: firing marks the target's receive timer so
    /// the scheduler applies the timeout-label jump on the next slice. The
    /// timer's `message` is NOT delivered.
    ReceiveTimeout,
    /// Backs `send_after`/`start_timer` and native timer scheduling: firing
    /// delivers `message` to `target_pid`'s mailbox.
    Deliver,
}

/// Timer entry metadata stored in the wheel index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerEntry {
    /// Process that receives the message when the timer expires.
    pub target_pid: u64,
    /// Message to deliver.
    pub message: Term,
    /// Absolute expiry instant.
    pub expires_at: Instant,
    /// What the scheduler does with this timer when it fires.
    pub kind: TimerKind,
    bucket: usize,
    slot: usize,
}

/// Expired timer returned by [`TimerWheel::tick`] / [`TimerWheel::tick_at`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExpiredTimer {
    /// Reference of the fired timer.
    pub reference: TimerRef,
    /// Process that receives the message.
    pub target_pid: u64,
    /// Message to deliver.
    pub message: Term,
    /// Absolute expiry instant.
    pub expires_at: Instant,
    /// What the scheduler does with this timer when it fires.
    pub kind: TimerKind,
}

/// Millisecond-granularity O(1) timer wheel for one-shot timers.
pub struct TimerWheel {
    buckets: Vec<Vec<TimerRef>>,
    entries: HashMap<TimerRef, TimerEntry>,
    /// Every pending entry's `(expires_at, reference)`, ordered: the
    /// earliest deadline is the first, so worker 0's read at every park is
    /// O(log n), not a scan of `entries` (Hermes's F1 on B-181). Kept in
    /// step with `entries` on arm, cancel and expire.
    deadlines: BTreeSet<(Instant, TimerRef)>,
    next_ref: u64,
    start: Instant,
    current_tick: u128,
    /// Buckets walked by `tick_at` to date: the instrument for the one-lap
    /// catch-up bound (B-181 R3, EMBEDDER-COMPOSITION-SPEC §3.8.2 C4).
    #[cfg(any(test, feature = "test-support"))]
    bucket_visits: u64,
    /// Arming edges run to date: one per `schedule_reserved_at` that woke a
    /// parked worker 0 for an earlier deadline (§3.8.2 C2 instrument).
    #[cfg(any(test, feature = "test-support"))]
    arm_wake_edges: u64,
    /// Timer entries examined by `earliest_deadline` to date: the cost of
    /// the read worker 0 makes at every park, counted, never timed.
    #[cfg(any(test, feature = "test-support"))]
    deadline_reads: std::cell::Cell<u64>,
    /// Published by worker 0 under this wheel's lock when it parks, cleared
    /// when it leaves the park (§3.8.2 C2).
    #[cfg(feature = "threads")]
    parked_until: ParkedUntil,
    #[cfg(feature = "threads")]
    arm_notifier: Option<ArmNotifier>,
}

impl std::fmt::Debug for TimerWheel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("TimerWheel");
        debug
            .field("buckets", &self.buckets)
            .field("entries", &self.entries)
            .field("next_ref", &self.next_ref)
            .field("start", &self.start)
            .field("current_tick", &self.current_tick);
        #[cfg(feature = "threads")]
        debug
            .field("parked_until", &self.parked_until)
            .field("arm_notifier", &self.arm_notifier.is_some());
        debug.finish()
    }
}

impl TimerWheel {
    /// Create a wheel with the default bucket count.
    #[must_use]
    pub fn new() -> Self {
        Self::with_bucket_count(DEFAULT_BUCKETS)
    }

    /// Create a wheel with at least one bucket.
    #[must_use]
    pub fn with_bucket_count(bucket_count: usize) -> Self {
        let bucket_count = bucket_count.max(1);
        let buckets = (0..bucket_count).map(|_| Vec::new()).collect();
        Self {
            buckets,
            entries: HashMap::new(),
            deadlines: BTreeSet::new(),
            next_ref: 1,
            start: Instant::now(),
            current_tick: 0,
            #[cfg(any(test, feature = "test-support"))]
            bucket_visits: 0,
            #[cfg(any(test, feature = "test-support"))]
            arm_wake_edges: 0,
            #[cfg(any(test, feature = "test-support"))]
            deadline_reads: std::cell::Cell::new(0),
            #[cfg(feature = "threads")]
            parked_until: ParkedUntil::NotParked,
            #[cfg(feature = "threads")]
            arm_notifier: None,
        }
    }

    /// Schedule `message` for `target_pid` after `delay` from now.
    pub fn schedule(
        &mut self,
        delay: Duration,
        target_pid: u64,
        message: Term,
        kind: TimerKind,
    ) -> TimerRef {
        self.schedule_at(Instant::now(), delay, target_pid, message, kind)
    }

    /// Reserve a unique timer reference for callers that must include it in the message.
    pub fn reserve_reference(&mut self) -> TimerRef {
        self.allocate_ref()
    }

    /// Schedule `message` with a previously reserved reference.
    pub fn schedule_reserved(
        &mut self,
        reference: TimerRef,
        delay: Duration,
        target_pid: u64,
        message: Term,
        kind: TimerKind,
    ) -> Option<TimerRef> {
        self.schedule_reserved_at(reference, Instant::now(), delay, target_pid, message, kind)
    }

    /// Deterministic reserved-reference scheduling variant.
    pub fn schedule_reserved_at(
        &mut self,
        reference: TimerRef,
        now: Instant,
        delay: Duration,
        target_pid: u64,
        message: Term,
        kind: TimerKind,
    ) -> Option<TimerRef> {
        if self.entries.contains_key(&reference) {
            return None;
        }
        if now < self.start {
            self.start = now;
            self.current_tick = 0;
        }
        let expires_at = now.checked_add(delay).unwrap_or(now);
        let bucket = self.bucket_for(expires_at);
        let slot = self.buckets[bucket].len();
        self.buckets[bucket].push(reference);
        self.deadlines.insert((expires_at, reference));
        self.entries.insert(
            reference,
            TimerEntry {
                target_pid,
                message,
                expires_at,
                kind,
                bucket,
                slot,
            },
        );
        #[cfg(feature = "threads")]
        self.wake_parked_worker_for(expires_at);
        Some(reference)
    }

    /// Deterministic scheduling variant used by tests and scheduler ticks.
    pub fn schedule_at(
        &mut self,
        now: Instant,
        delay: Duration,
        target_pid: u64,
        message: Term,
        kind: TimerKind,
    ) -> TimerRef {
        let reference = self.allocate_ref();
        self.schedule_reserved_at(reference, now, delay, target_pid, message, kind)
            .unwrap_or(reference)
    }

    /// Cancel a pending timer and return its remaining duration from now.
    pub fn cancel(&mut self, reference: TimerRef) -> Option<Duration> {
        self.cancel_at(reference, Instant::now())
    }

    /// Deterministic cancellation variant returning remaining duration from `now`.
    pub fn cancel_at(&mut self, reference: TimerRef, now: Instant) -> Option<Duration> {
        let entry = self.remove_entry(reference)?;
        Some(entry.expires_at.saturating_duration_since(now))
    }

    /// Process timers expired at the current instant.
    pub fn tick(&mut self) -> Vec<ExpiredTimer> {
        self.tick_at(Instant::now())
    }

    /// Process timers expired at `now`.
    ///
    /// Walks the buckets from `current_tick` to the tick `now` falls in, at
    /// most one lap (§3.8.2 C4): one lap visits every bucket once, and a
    /// bucket fires each entry by `expires_at <= now` and leaves later ones in
    /// place, so a longer walk would only revisit buckets. A tick after a long
    /// park therefore costs at most one lap and never fires early. The walk
    /// ends with `current_tick` ON `now`'s tick, so the next walk revisits
    /// that bucket: a timer due later in the same millisecond is never left
    /// behind for a lap.
    pub fn tick_at(&mut self, now: Instant) -> Vec<ExpiredTimer> {
        let mut expired = Vec::new();
        let target_tick = self.tick_for(now);
        if target_tick < self.current_tick {
            return expired;
        }
        let lap = self.buckets.len() as u128;
        let steps = (target_tick - self.current_tick).saturating_add(1).min(lap);
        for step in 0..steps {
            let tick = self.current_tick.saturating_add(step);
            let bucket_index = (tick % lap) as usize;
            self.expire_bucket(bucket_index, now, &mut expired);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.bucket_visits = self.bucket_visits.saturating_add(1);
            }
        }
        self.current_tick = target_tick;
        expired
    }

    /// The tick the wheel's next walk starts from. Test instrument for the
    /// one-lap catch-up bound (§3.8.2 C4).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn current_tick(&self) -> u128 {
        self.current_tick
    }

    /// The tick `now` falls in, measured from the wheel's start. Test
    /// instrument paired with [`TimerWheel::current_tick`].
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn target_tick(&self, now: Instant) -> u128 {
        self.tick_for(now)
    }

    /// Buckets walked by every `tick_at` to date (§3.8.2 C4 instrument).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn bucket_visits(&self) -> u64 {
        self.bucket_visits
    }

    /// Arming edges run to date (§3.8.2 C2 instrument).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn arm_wake_edges(&self) -> u64 {
        self.arm_wake_edges
    }

    /// The bucket count of this wheel: one lap of the catch-up walk.
    #[must_use]
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// Number of pending timers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no timers are pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the earliest absolute deadline among pending timers.
    #[must_use]
    pub fn earliest_deadline(&self) -> Option<Instant> {
        self.deadlines.first().map(|(expires_at, _reference)| {
            self.count_deadline_read();
            *expires_at
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    fn count_deadline_read(&self) {
        self.deadline_reads
            .set(self.deadline_reads.get().saturating_add(1));
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn count_deadline_read(&self) {}

    /// Timer entries `earliest_deadline` has examined to date: the cost of
    /// the deadline read worker 0 makes at every park (test instrument).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn deadline_reads(&self) -> u64 {
        self.deadline_reads.get()
    }

    /// Inspect a pending timer entry.
    #[must_use]
    pub fn get(&self, reference: TimerRef) -> Option<&TimerEntry> {
        self.entries.get(&reference)
    }

    fn allocate_ref(&mut self) -> TimerRef {
        let reference = TimerRef(self.next_ref);
        self.next_ref = self.next_ref.checked_add(1).unwrap_or(1);
        reference
    }

    fn bucket_for(&self, expires_at: Instant) -> usize {
        (self.tick_for(expires_at) % self.buckets.len() as u128) as usize
    }

    fn tick_for(&self, instant: Instant) -> u128 {
        instant.saturating_duration_since(self.start).as_millis()
    }

    fn expire_bucket(
        &mut self,
        bucket_index: usize,
        now: Instant,
        expired: &mut Vec<ExpiredTimer>,
    ) {
        let mut slot = 0;
        while slot < self.buckets[bucket_index].len() {
            let reference = self.buckets[bucket_index][slot];
            let Some(entry) = self.entries.get(&reference) else {
                self.swap_remove_bucket_slot(bucket_index, slot);
                continue;
            };
            if entry.expires_at <= now {
                if let Some(entry) = self.remove_entry(reference) {
                    expired.push(ExpiredTimer {
                        reference,
                        target_pid: entry.target_pid,
                        message: entry.message,
                        expires_at: entry.expires_at,
                        kind: entry.kind,
                    });
                }
            } else {
                slot += 1;
            }
        }
    }

    fn remove_entry(&mut self, reference: TimerRef) -> Option<TimerEntry> {
        let entry = self.entries.remove(&reference)?;
        self.deadlines.remove(&(entry.expires_at, reference));
        self.swap_remove_bucket_slot(entry.bucket, entry.slot);
        Some(entry)
    }

    fn swap_remove_bucket_slot(&mut self, bucket: usize, slot: usize) {
        let Some(bucket_entries) = self.buckets.get_mut(bucket) else {
            return;
        };
        if slot >= bucket_entries.len() {
            return;
        }
        let moved = bucket_entries.swap_remove(slot);
        if slot < bucket_entries.len() {
            let replacement = bucket_entries[slot];
            if let Some(entry) = self.entries.get_mut(&replacement) {
                entry.slot = slot;
            }
        }
        if let Some(entry) = self.entries.get_mut(&moved) {
            entry.slot = slot;
        }
    }
}

impl Default for TimerWheel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "threads")]
mod park;
#[cfg(feature = "threads")]
pub(crate) use park::ArmNotifier;
#[cfg(feature = "threads")]
use park::ParkedUntil;

#[cfg(test)]
mod tests;
