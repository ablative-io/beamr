//! A process's side of native code: the handler body of a native process,
//! the continuations a higher-order native BIF resumes once its closure
//! returns, and the GC roots native code registers.

use crate::native::NativeContinuation;
use crate::native::native_process::NativeBody;
use crate::term::Term;

use super::{CodePosition, Process};

/// One pending native higher-order-BIF continuation.
///
/// Pending continuations form a stack so natives can nest (a closure called
/// by `lists:map` may itself call `lists:map`). Each entry records the frame
/// depth just below its trampoline return frame AND the code position the
/// trampoline returns to (the BIF call instruction); the continuation
/// resumes only when the stack is back at that depth at exactly that
/// position — i.e. when its closure call returned. Depth alone is not
/// enough: a process re-entering an await elsewhere at equal stack depth
/// (a tail-called receive inside the closure) would otherwise re-fire the
/// continuation with garbage in x0.
#[derive(Clone, Debug)]
pub struct PendingNativeContinuation {
    /// Saved native state to re-enter.
    pub continuation: NativeContinuation,
    /// Frame count below the trampoline return frame.
    pub resume_depth: usize,
    /// The BIF call instruction the trampoline return frame jumps back to.
    pub resume_position: Option<CodePosition>,
}

impl Process {
    /// True exactly when this process carries a native handler body (Shape B).
    ///
    /// A native process never sets a code position, x-registers, or stack;
    /// those stay default/empty and its scheduler machinery (status graph,
    /// `ProcessMetadata` swap, park-gap, exit-tombstones) is reused unchanged.
    ///
    /// `Process::clone` AUDIT (NATIVE-001 R2): the structural `Clone` impl
    /// sets `native: None`, so a clone is always non-native. This is safe
    /// because no live scheduler path clones a native process: spawn inserts a
    /// freshly built `Process` (never a clone); the store/take slot swap moves
    /// the `Process` by value through `std::mem::take` / `ProcessSlot`, never
    /// cloning it; and replay does not snapshot process bodies. A native
    /// restart (NATIVE-002) rebuilds the handler via the retained factory,
    /// never by cloning a handler instance — so a clone can never silently
    /// drop a live handler and leave a dead no-op process.
    #[must_use]
    pub fn is_native(&self) -> bool {
        self.native.is_some()
    }

    /// Install a native handler body, making this a native process. Called by
    /// the scheduler's `spawn_native` before the process is made runnable.
    pub(crate) fn set_native_body(&mut self, body: NativeBody) {
        self.native = Some(body);
    }

    /// Mutable access to the native body, for `run_native_slice` to take the
    /// handler out for a slice (and the restart path to reach the factory).
    pub(crate) fn native_body_mut(&mut self) -> Option<&mut NativeBody> {
        self.native.as_mut()
    }

    /// Push native continuation state for closure-return re-entry.
    ///
    /// `resume_depth` is the frame count just below the trampoline return
    /// frame: the continuation resumes once the stack is back at (or below)
    /// that depth, which happens exactly when the closure call returns.
    pub fn push_native_continuation(
        &mut self,
        continuation: NativeContinuation,
        resume_depth: usize,
    ) {
        self.native_continuations.push(PendingNativeContinuation {
            continuation,
            resume_depth,
            resume_position: self.code_position,
        });
    }

    /// Take the innermost native continuation after its closure returns.
    pub fn take_native_continuation(&mut self) -> Option<NativeContinuation> {
        self.native_continuations.pop().map(|p| p.continuation)
    }

    /// True when the innermost pending continuation's closure has returned
    /// (its trampoline return frame has been popped, landing the process
    /// back at the recorded call instruction) and the native must be
    /// re-entered before the next instruction executes.
    ///
    /// The position check prevents a stale re-fire: stack depth alone can
    /// also match at an unrelated await re-entered at equal depth, where
    /// x0 holds garbage rather than the closure's return value.
    #[must_use]
    pub fn native_continuation_ready(&self) -> bool {
        self.native_continuations.last().is_some_and(|pending| {
            self.stack.len() <= pending.resume_depth
                && pending.resume_position == self.code_position
        })
    }

    /// Drop pending continuations whose trampoline return frames were
    /// discarded by an exception-handler stack truncation to `depth`.
    pub fn prune_native_continuations(&mut self, depth: usize) {
        self.native_continuations
            .retain(|pending| pending.resume_depth < depth);
    }

    /// Register `term` as an explicit GC root, returning its stack index.
    ///
    /// The root is traced and forwarded by every collection until removed
    /// with [`Process::truncate_native_roots`].
    pub(crate) fn push_native_root(&mut self, term: Term) -> usize {
        self.native_roots.push(term);
        self.native_roots.len() - 1
    }

    /// Read the current (post-GC) value of the native root at `index`.
    pub(crate) fn native_root(&self, index: usize) -> Option<Term> {
        self.native_roots.get(index).copied()
    }

    /// Overwrite the native root at `index` with a new term.
    pub(crate) fn set_native_root(&mut self, index: usize, term: Term) {
        if let Some(slot) = self.native_roots.get_mut(index) {
            *slot = term;
        }
    }

    /// Number of registered native roots.
    pub(crate) fn native_root_depth(&self) -> usize {
        self.native_roots.len()
    }

    /// Drop native roots registered after `depth`.
    pub(crate) fn truncate_native_roots(&mut self, depth: usize) {
        self.native_roots.truncate(depth);
    }

    /// Check whether any native continuation is pending.
    #[must_use]
    pub fn has_native_continuation(&self) -> bool {
        !self.native_continuations.is_empty()
    }
}
