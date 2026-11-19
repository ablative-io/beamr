//! Exception state: the handler stack and its nesting floor, the raw
//! stacktrace of the latest raise, and the exception currently caught.

use super::{Exception, ExceptionHandler, Process, RawStackEntry};

impl Process {
    /// Install an exception handler.
    pub fn push_exception_handler(&mut self, handler: ExceptionHandler) {
        self.handlers.push(handler);
    }

    /// Remove the most recently installed exception handler.
    ///
    /// Unconditional: this is the paired counterpart of `push_exception_handler`
    /// for the `try_end`/`catch_end` opcodes, which retire the handler their own
    /// `try`/`catch` installed. A raise must use
    /// [`Self::pop_exception_handler_in_nest`] instead, which refuses to reach
    /// past the current nesting.
    pub fn pop_exception_handler(&mut self) -> Option<ExceptionHandler> {
        self.handlers.pop()
    }

    /// Remove the most recently installed exception handler, but only when it
    /// was installed inside the current interpreter nesting.
    ///
    /// Returns `None` when the handler stack has been drained down to
    /// [`Self::nested_handler_floor`] — every remaining handler belongs to an
    /// outer run, and jumping to one from here would resume outer code without
    /// ever returning through the native frames that entered this run.
    pub fn pop_exception_handler_in_nest(&mut self) -> Option<ExceptionHandler> {
        if self.handlers.len() <= self.nested_handler_floor {
            return None;
        }
        self.handlers.pop()
    }

    /// Lowest handler-stack index a raise may consume in the current nesting.
    #[must_use]
    pub const fn nested_handler_floor(&self) -> usize {
        self.nested_handler_floor
    }

    /// Install the handler floor for a nested interpreter run.
    ///
    /// Callers save the previous value and restore it on every exit from the
    /// nested region — the floor is scoped to that region exactly as
    /// `current_module`/`code_position` are.
    pub const fn set_nested_handler_floor(&mut self, floor: usize) {
        self.nested_handler_floor = floor;
    }

    /// Number of installed exception handlers.
    #[must_use]
    pub fn exception_handler_count(&self) -> usize {
        self.handlers.len()
    }

    /// Store the raw stacktrace captured when an exception is raised.
    pub fn set_raw_stacktrace(&mut self, raw_stacktrace: Vec<RawStackEntry>) {
        self.raw_stacktrace = raw_stacktrace;
    }

    /// Clear any raw stacktrace associated with a handled exception.
    pub fn clear_raw_stacktrace(&mut self) {
        self.raw_stacktrace.clear();
    }

    /// Raw stacktrace entries captured at the most recent raise.
    #[must_use]
    pub fn raw_stacktrace(&self) -> &[RawStackEntry] {
        &self.raw_stacktrace
    }

    /// Store the current caught exception.
    pub const fn set_current_exception(&mut self, exception: Option<Exception>) {
        self.current_exception = exception;
    }

    /// Current caught exception, when present.
    #[must_use]
    pub const fn current_exception(&self) -> Option<Exception> {
        self.current_exception
    }
}
