//! What a parked process is waiting for: its receive timeout and the timer
//! behind it, and the result-gated suspension only a matching completion may
//! resume.

use super::{CodePosition, Process, ReceiveTimeout};

/// Why a process is parked beyond a plain receive.
///
/// Plain receives (BEAM `wait`/`wait_timeout`, and select-style native
/// suspends that re-execute on message arrival) carry no suspension record:
/// any message may wake them. The kinds below are *result-gated*: only the
/// matching completion event (identified by the record's call id) may resume
/// the process, because re-executing the parked instruction would repeat its
/// side effect (re-submit a host call, re-enter a dirty native, …).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuspensionKind {
    /// A native requested suspension and a host-side completion will deliver
    /// the result (`Scheduler::wake_with_result*`, file-I/O completions, or a
    /// receive-timeout re-entry).
    HostAwait,
    /// A dirty native call is in flight on a dirty scheduler pool.
    DirtyCall,
    /// The reduction-boundary hook suspended the process; an embedder
    /// `resume_process` call resumes it.
    Hook,
}

/// How execution continues when a host completion is applied into x0 at a
/// parked call instruction (which is then NOT re-executed).
///
/// Body-position native calls resume at the next instruction. Tail-position
/// native calls (`call_ext_only`, `call_ext_last`) have no next instruction
/// — the applied result IS the function's return value, so the process must
/// return to the caller instead of running off the end of the function.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResumeContinuation {
    /// Body-position call: resume at the instruction after the park.
    #[default]
    Advance,
    /// Tail call with no live stack frame at the park (`call_ext_only`, or
    /// a `call_ext_last` whose frame was already popped before the park):
    /// return to the caller.
    Return,
    /// Tail call parked with its y-frame intact (`call_ext_last`, whose
    /// frame pop is deferred across the suspension so wake re-execution is
    /// idempotent): pop the frame, then return to the caller.
    DeallocateAndReturn,
}

/// Identity of one result-gated suspension.
///
/// `call_id` is allocated from a per-process monotonic counter at suspend
/// time and never reused, so a completion produced for an earlier suspension
/// can always be recognized as stale and dropped instead of being applied at
/// the wrong park position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SuspensionRecord {
    /// Per-process monotonically increasing suspension identity.
    pub call_id: u64,
    /// What kind of completion may consume this suspension.
    pub kind: SuspensionKind,
    /// Code position at suspend time (the parked call instruction), used to
    /// validate that a result is applied exactly where it was produced.
    pub position: Option<CodePosition>,
    /// True for message-wakeable host awaits (`request_suspend`: select and
    /// marker-style awaits, whose natives are re-entrant and re-execute on
    /// any wake). False for gated awaits (`request_await_suspend`, dirty
    /// calls, hook suspends), which only their own completion may resume.
    pub wake_on_message: bool,
    /// How a host completion applied at this park continues execution.
    pub continuation: ResumeContinuation,
}

impl Process {
    /// Record receive timeout state for scheduler/timer integration.
    pub const fn set_receive_timeout(&mut self, timeout: Option<ReceiveTimeout>) {
        self.receive_timeout = timeout;
    }

    /// Receive timeout state, when waiting with a deadline.
    #[must_use]
    pub const fn receive_timeout(&self) -> Option<ReceiveTimeout> {
        self.receive_timeout
    }

    /// Store the timer reference for the active receive timeout, used by the
    /// scheduler to cancel the timer when a message arrives first.
    pub const fn set_receive_timer_ref(&mut self, timer_ref: Option<u64>) {
        self.receive_timer_ref = timer_ref;
    }

    /// Active receive timer reference, when a timeout timer is outstanding.
    #[must_use]
    pub const fn receive_timer_ref(&self) -> Option<u64> {
        self.receive_timer_ref
    }

    /// Allocate the next suspension call id from this process's monotonic
    /// counter. Ids start at 1 and are never reused; 0 is reserved as the
    /// embedder wildcard in `Scheduler::resume_process`.
    pub const fn allocate_suspension_call_id(&mut self) -> u64 {
        self.next_suspension_call_id += 1;
        self.next_suspension_call_id
    }

    /// Install the current result-gated suspension record.
    pub const fn set_suspension(&mut self, record: Option<SuspensionRecord>) {
        self.suspension = record;
    }

    /// Current result-gated suspension record, when parked beyond a plain
    /// receive.
    #[must_use]
    pub const fn suspension(&self) -> Option<SuspensionRecord> {
        self.suspension
    }

    /// Consume the current suspension record.
    pub const fn take_suspension(&mut self) -> Option<SuspensionRecord> {
        self.suspension.take()
    }
}
