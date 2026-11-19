//! Sending a message into a process's mailbox from outside the scheduler,
//! and the typed failure such a send returns.

use super::{Scheduler, execution, timer_integration};
use crate::ets::copy::OwnedTerm;
use crate::term::Term;

/// Typed failure returned by [`Scheduler::send_to_mailbox`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MailboxSendError {
    /// No live process body or retained exit tombstone exists for the PID.
    NoSuchProcess,
    /// The PID has a retained exit tombstone and can no longer accept messages.
    ProcessTerminated,
    /// The process body exists, but its slot cannot currently admit a message.
    ProcessSlotUnavailable,
    /// The target process heap cannot reserve enough space for the message.
    HeapAllocationFailed,
    /// The owned value does not contain a valid, copyable BEAM term.
    InvalidMessage,
    /// Mailbox admission has closed during scheduler shutdown.
    SchedulerTerminated,
}

impl std::fmt::Display for MailboxSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoSuchProcess => "no such process",
            Self::ProcessTerminated => "process already terminated",
            Self::ProcessSlotUnavailable => "process slot unavailable for mailbox admission",
            Self::HeapAllocationFailed => "target heap cannot admit mailbox message",
            Self::InvalidMessage => "invalid owned mailbox message",
            Self::SchedulerTerminated => "scheduler has shut down",
        })
    }
}

impl std::error::Error for MailboxSendError {}

impl Scheduler {
    /// Enqueue an immediate atom message into a live process mailbox and wake
    /// the process if it is parked.
    ///
    /// Embedders use this as a host-to-process wake primitive (e.g. activity
    /// completion markers). Delivery must succeed in every live slot state: a
    /// process currently executing a slice receives the message through its
    /// pending metadata, which the scheduler merges into the mailbox at
    /// store-back and then resumes the process if it suspended meanwhile —
    /// otherwise a completion racing the suspend transition is lost and the
    /// process sleeps forever.
    ///
    /// The wake applies to plain receives and message-wakeable suspends
    /// (`ProcessContext::request_suspend`). A process parked under a *gated*
    /// suspension (`request_await_suspend`, an in-flight dirty call, a hook
    /// suspend) keeps the message in its mailbox but stays parked until its
    /// own completion event arrives — waking it would re-execute the parked
    /// call instruction and repeat its host side effect.
    ///
    /// Returns false only when no live process exists for `target_pid`.
    #[must_use]
    pub fn enqueue_atom_message(&self, target_pid: u64, atom: crate::atom::Atom) -> bool {
        let delivered =
            timer_integration::deliver_term_to_mailbox(&self.shared, target_pid, Term::atom(atom));
        if delivered.is_ok() {
            execution::wake_process(&self.shared, target_pid);
        }
        delivered.is_ok()
    }

    /// Send an owned arbitrary term to a process mailbox.
    ///
    /// The message is deep-copied from [`OwnedTerm`] storage into the target
    /// process heap before this method returns. A waiting target is woken after
    /// the message is visible. If the target is executing, admission is recorded
    /// in its executing-slot metadata, copied at slice store-back, and then
    /// woken; the receive park path's post-registration mailbox recheck closes
    /// the store-back-to-wait-set gap, so the next receive observes the message
    /// without a lost wake. Each successful send contributes at most one wake.
    ///
    /// This supports event-driven host consumers such as R-B-1 supervisors:
    /// encode command identity and payload in a structurally disjoint tagged
    /// tuple, own it with [`crate::ets::copy_term_to_ets`], then send that value
    /// here and handle [`MailboxSendError`] instead of maintaining a side queue
    /// plus a wake-only notification.
    ///
    /// Deliveries are appended under the target slot lock, preserving FIFO order
    /// with this scheduler's atom and timer mailbox deliveries.
    pub fn send_to_mailbox(
        &self,
        target_pid: u64,
        message: OwnedTerm,
    ) -> Result<(), MailboxSendError> {
        timer_integration::deliver_owned_term_to_mailbox(&self.shared, target_pid, message)?;
        execution::wake_process(&self.shared, target_pid);
        Ok(())
    }
}
