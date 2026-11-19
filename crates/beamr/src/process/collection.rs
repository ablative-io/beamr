//! What the garbage collector reads from and records on a process: the
//! roots it traces and forwards, and its count of collections attempted and
//! completed.

use crate::term::Term;

use super::Process;

impl Process {
    /// Snapshot every GC root owned by this process, treating all X registers as live.
    pub(crate) fn roots(&mut self) -> Vec<Term> {
        self.roots_with_live_x(256)
    }

    /// Snapshot every live GC root owned by this process.
    pub(crate) fn roots_with_live_x(&mut self, live_x: usize) -> Vec<Term> {
        self.mailbox.drain_arrival();
        let live_x = live_x.min(self.x_regs.len());
        let exception_roots = self
            .current_exception
            .into_iter()
            .flat_map(|exception| [exception.reason, exception.stacktrace]);
        let mut roots: Vec<Term> = self
            .x_regs
            .iter()
            .take(live_x)
            .chain(self.stack.y_regs())
            .chain(self.mailbox.scan_iter())
            .copied()
            .chain(exception_roots)
            .chain(
                self.dictionary
                    .iter()
                    .flat_map(|(key, value)| [*key, *value]),
            )
            .chain(std::iter::once(self.group_leader))
            .collect();
        roots.extend(self.native_roots.iter().copied());
        for pending in &self.native_continuations {
            pending
                .continuation
                .for_each_term(&mut |term| roots.push(term));
        }
        roots
    }

    /// Replace every GC root with the next term yielded by `roots`, in the same
    /// order as [`Process::roots`]. Extra yielded terms are ignored.
    pub(crate) fn replace_roots(&mut self, roots: &[Term]) {
        self.replace_roots_with_live_x(256, roots);
    }

    /// Replace every live GC root with the next term yielded by `roots`, in the
    /// same order as [`Process::roots_with_live_x`]. Extra yielded terms are ignored.
    pub(crate) fn replace_roots_with_live_x(&mut self, live_x: usize, roots: &[Term]) {
        let mut index = 0;
        let live_x = live_x.min(self.x_regs.len());
        for root in self.x_regs.iter_mut().take(live_x) {
            if let Some(value) = roots.get(index).copied() {
                *root = value;
            }
            index += 1;
        }
        for root in self.stack.y_regs_mut() {
            if let Some(value) = roots.get(index).copied() {
                *root = value;
            }
            index += 1;
        }
        for root in self.mailbox.scan_iter_mut() {
            if let Some(value) = roots.get(index).copied() {
                *root = value;
            }
            index += 1;
        }
        if let Some(exception) = &mut self.current_exception {
            if let Some(value) = roots.get(index).copied() {
                exception.reason = value;
            }
            index += 1;
            if let Some(value) = roots.get(index).copied() {
                exception.stacktrace = value;
            }
            index += 1;
        }
        for (key, value) in &mut self.dictionary {
            if let Some(root) = roots.get(index).copied() {
                *key = root;
            }
            index += 1;
            if let Some(root) = roots.get(index).copied() {
                *value = root;
            }
            index += 1;
        }
        if let Some(root) = roots.get(index).copied() {
            self.group_leader = root;
        }
        index += 1;
        for root in &mut self.native_roots {
            if let Some(value) = roots.get(index).copied() {
                *root = value;
            }
            index += 1;
        }
        for pending in &mut self.native_continuations {
            pending.continuation.for_each_term_mut(&mut |term| {
                if let Some(value) = roots.get(index).copied() {
                    *term = value;
                }
                index += 1;
            });
        }
    }

    /// Collections attempted on this process's heap, ever.
    ///
    /// ⭐ THIS IS A COLLECTION OBSERVABLE, AND [`Heap::total_capacity`] IS NOT.
    /// Capacity witnesses a heap RESIZE. The two come apart in both directions:
    /// a collection can succeed without the heap ever resizing, and the heap can
    /// resize with no collection at all. Anything asking "was this region under
    /// collection pressure?" must bracket these counters, not the capacity.
    ///
    /// [`Heap::total_capacity`]: crate::process::heap::Heap::total_capacity
    #[must_use]
    pub const fn gc_attempts(&self) -> u64 {
        self.gc_attempts
    }

    /// Collections that completed successfully on this process's heap, ever.
    ///
    /// `gc_attempts() - gc_completions()` is the number of collections that were
    /// entered and did NOT finish — the case a single counter cannot express.
    #[must_use]
    pub const fn gc_completions(&self) -> u64 {
        self.gc_completions
    }

    /// Record entry to a collection, before it can fail.
    pub(crate) fn note_gc_attempt(&mut self) {
        self.gc_attempts = self.gc_attempts.saturating_add(1);
    }

    /// Record a collection that completed successfully.
    pub(crate) fn note_gc_completion(&mut self) {
        self.gc_completions = self.gc_completions.saturating_add(1);
    }
}
