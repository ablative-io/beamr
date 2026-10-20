//! Worker 0's park against the timer wheel (B-181, EMBEDDER-COMPOSITION-SPEC
//! §3.8.2 C2): the deadline it publishes when it parks, and the arm edge
//! that wakes it when a timer is armed earlier than that deadline.

use std::time::Instant;

use super::TimerWheel;

/// Called by the wheel when a timer is armed earlier than the deadline a
/// parked worker 0 waits for (§3.8.2 C2): the scheduler's edge E.
pub(crate) type ArmNotifier = std::sync::Arc<dyn Fn() + Send + Sync>;

/// The deadline the scheduler's worker 0 has published it parks until.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) enum ParkedUntil {
    /// Worker 0 is not parked: it reads the earliest deadline at its next
    /// park, so an arming now needs no edge.
    NotParked,
    /// Worker 0 is parked with no deadline: every arming is earlier.
    NoDeadline,
    /// Worker 0 is parked until this instant.
    Deadline(Instant),
}

impl ParkedUntil {
    /// Whether a timer expiring at `expires_at` would be overslept.
    fn is_later_than(self, expires_at: Instant) -> bool {
        match self {
            Self::NotParked => false,
            Self::NoDeadline => true,
            Self::Deadline(deadline) => expires_at < deadline,
        }
    }
}

impl TimerWheel {
    /// W11's edge (§3.8.2 C2): O(1), never a scan. Runs the scheduler's edge
    /// only when worker 0 is parked past `expires_at`, then records the new,
    /// earlier deadline so arms between the two need no second edge.
    pub(super) fn wake_parked_worker_for(&mut self, expires_at: Instant) {
        if !self.parked_until.is_later_than(expires_at) {
            return;
        }
        let Some(notifier) = self.arm_notifier.as_ref() else {
            return;
        };
        notifier();
        self.parked_until = ParkedUntil::Deadline(expires_at);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.arm_wake_edges = self.arm_wake_edges.saturating_add(1);
        }
    }

    /// Install the scheduler's edge, run when a timer is armed earlier than
    /// worker 0's published park deadline.
    pub(crate) fn set_arm_notifier(&mut self, notifier: ArmNotifier) {
        self.arm_notifier = Some(notifier);
    }

    /// Worker 0, about to park: read the earliest armed deadline (the first
    /// of the ordered index, O(log n)) and publish it as the deadline worker
    /// 0 parks until, both under this wheel's lock.
    pub(crate) fn publish_park(&mut self) -> Option<Instant> {
        let deadline = self.earliest_deadline();
        self.parked_until = deadline.map_or(ParkedUntil::NoDeadline, ParkedUntil::Deadline);
        deadline
    }

    /// Worker 0 left its park: arming needs no edge until it parks again.
    pub(crate) fn clear_park(&mut self) {
        self.parked_until = ParkedUntil::NotParked;
    }
}
