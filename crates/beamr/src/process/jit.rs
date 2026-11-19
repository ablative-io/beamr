//! The channels JIT-compiled code reports through: its runtime context,
//! the status it returns, and a transfer or execution error a runtime helper
//! parks on the process because compiled code can only return a word.

// `Box` is absent from the no_std prelude; guarded as its import in `mod.rs`.
#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

#[cfg(feature = "jit")]
use super::JitRuntimeContext;
use super::{JitStatus, Process};

impl Process {
    /// Runtime context visible to JIT helper calls for the current native invocation.
    #[cfg(feature = "jit")]
    #[must_use]
    pub const fn jit_runtime_context(&self) -> Option<JitRuntimeContext> {
        self.jit_runtime_context
    }

    /// Set the transient JIT runtime context for the duration of a native invocation.
    #[cfg(feature = "jit")]
    pub const fn set_jit_runtime_context(&mut self, context: Option<JitRuntimeContext>) {
        self.jit_runtime_context = context;
    }

    /// Mark the outcome status reported by JIT-generated code.
    pub const fn set_jit_status(&mut self, status: Option<JitStatus>) {
        self.jit_status = status;
    }

    /// Take and clear the current JIT status.
    pub fn take_jit_status(&mut self) -> Option<JitStatus> {
        self.jit_status.take()
    }

    /// Park a scheduler-level transfer produced by a nested interpreted run.
    ///
    /// The caller's contract, and the whole point of the channel: the resume
    /// position the nested run established must be LEFT IN PLACE. A transfer is
    /// not a value, so it cannot ride the return word out of compiled code, and
    /// the deopt return it rides instead means "restart this function
    /// interpreted from its entry" — which would re-execute the tail external
    /// call that produced the transfer. `call_native` therefore takes this
    /// BEFORE it reads the status, exactly as it does the pending exec error.
    pub fn set_jit_transfer(&mut self, transfer: crate::interpreter::ExecutionResult) {
        self.jit_transfer = Some(Box::new(transfer));
    }

    /// Take and clear the parked scheduler-level transfer.
    pub fn take_jit_transfer(&mut self) -> Option<crate::interpreter::ExecutionResult> {
        self.jit_transfer.take().map(|transfer| *transfer)
    }

    /// Park an execution error raised inside a JIT runtime helper.
    ///
    /// The helper that sets this must have mutated NOTHING observable first
    /// (or restored it, e.g. the send clock on a replay mismatch): the seam
    /// converts the pending error into the slice's `Err(ExecError)` without
    /// re-executing the function, so the arm's own invariant is the only
    /// thing standing between this channel and a double effect.
    pub fn set_jit_exec_error(&mut self, error: crate::error::ExecError) {
        self.jit_exec_error = Some(error);
    }

    /// Take and clear the pending JIT helper execution error.
    pub fn take_jit_exec_error(&mut self) -> Option<crate::error::ExecError> {
        self.jit_exec_error.take()
    }
}
