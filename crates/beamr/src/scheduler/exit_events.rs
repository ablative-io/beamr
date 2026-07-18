//! Bounded, single-subscriber process-exit event delivery.

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};

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

pub(super) struct ExitEventPublisher {
    sender: OnceLock<Sender<ExitEvent>>,
    overflowed: Arc<AtomicBool>,
    capacity: usize,
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
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => self.overflowed.store(true, Ordering::Release),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
