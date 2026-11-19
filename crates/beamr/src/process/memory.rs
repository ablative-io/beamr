//! The memory a process owns: its heap, the off-heap binary bytes that
//! heap references, its stack and its mailbox.

use crate::mailbox::Mailbox;
use crate::process::heap::Heap;
use crate::process::stack::Stack;
use crate::term::heap_borrow::HeapBorrow;

use super::Process;

impl Process {
    /// Immutable access to this process heap.
    #[must_use]
    pub const fn heap(&self) -> &Heap {
        &self.heap
    }

    /// Mutable access to this process heap.
    pub fn heap_mut(&mut self) -> &mut Heap {
        &mut self.heap
    }

    /// A witness that this process's term storage is shared-borrowed.
    ///
    /// See [`Heap::borrow_terms`] for the mechanism.
    #[must_use]
    pub fn borrow_terms(&self) -> HeapBorrow<'_> {
        self.heap.borrow_terms()
    }

    /// Bytes of off-heap binary data currently referenced by ProcBins on this heap.
    #[must_use]
    pub const fn virtual_binary_heap(&self) -> usize {
        self.virtual_binary_heap
    }

    /// Record a newly allocated heap ProcBin's off-heap byte ownership.
    pub fn increase_virtual_binary_heap(&mut self, bytes: usize) {
        self.virtual_binary_heap = self.virtual_binary_heap.saturating_add(bytes);
    }

    /// Record removal of a heap ProcBin's off-heap byte ownership.
    pub(crate) fn decrease_virtual_binary_heap(&mut self, bytes: usize) {
        self.virtual_binary_heap = self.virtual_binary_heap.saturating_sub(bytes);
    }

    /// Immutable access to this process stack.
    #[must_use]
    pub const fn stack(&self) -> &Stack {
        &self.stack
    }

    /// Mutable access to this process stack.
    pub fn stack_mut(&mut self) -> &mut Stack {
        &mut self.stack
    }

    /// Immutable placeholder mailbox access for future receive support.
    #[must_use]
    pub const fn mailbox(&self) -> &Mailbox {
        &self.mailbox
    }

    /// Mutable placeholder mailbox access for message enqueue/receive support.
    pub fn mailbox_mut(&mut self) -> &mut Mailbox {
        &mut self.mailbox
    }
}
