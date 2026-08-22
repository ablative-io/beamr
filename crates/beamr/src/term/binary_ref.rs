//! Unified accessor for inline and off-heap binaries.

use crate::term::heap_borrow::HeapBorrow;
use crate::term::{
    Term,
    binary::Binary,
    boxed::{ProcBin, SubBinary},
};

/// Borrowed binary accessor that hides whether bytes are inline or off-heap.
#[derive(Copy, Clone, Debug)]
pub enum BinaryRef {
    Inline(Binary),
    Refc(ProcBin),
    Sub(SubBinary),
}

impl BinaryRef {
    /// Creates a binary accessor for inline binaries, ProcBins, or SubBinaries.
    pub fn new(term: Term) -> Option<Self> {
        if let Some(binary) = Binary::new(term) {
            return Some(Self::Inline(binary));
        }
        if let Some(proc_bin) = ProcBin::new(term) {
            return Some(Self::Refc(proc_bin));
        }
        SubBinary::new(term).map(Self::Sub)
    }

    /// Returns binary bytes without copying the backing storage, borrowed for
    /// the witness's borrow.
    ///
    /// The fan-in over all three binary families: whichever family backs this
    /// reference, the bytes are bounded by the process heap that owns the boxed
    /// object, and `heap` is the shared borrow of it that makes the bound
    /// enforceable.
    /// The bound is a type error, not a convention, for every family this
    /// reference can hold:
    ///
    /// ```compile_fail,E0502
    /// use beamr::process::Process;
    /// use beamr::term::binary::write_binary;
    /// use beamr::term::binary_ref::BinaryRef;
    ///
    /// let mut process = Process::new(1, 233);
    /// let words = process.heap_mut().alloc_slice(3).expect("words");
    /// let term = write_binary(words, b"hello").expect("binary");
    /// let binary = BinaryRef::new(term).expect("accessor");
    /// let bytes = binary.as_bytes(process.borrow_terms());
    /// let _ = beamr::gc::collect_minor(&mut process);
    /// assert_eq!(bytes, b"hello");
    /// ```
    pub fn as_bytes<'heap>(&self, heap: HeapBorrow<'heap>) -> &'heap [u8] {
        match self {
            Self::Inline(binary) => binary.as_bytes(heap),
            Self::Refc(proc_bin) => proc_bin.as_bytes(heap),
            Self::Sub(sub_binary) => sub_binary.as_bytes(heap),
        }
    }

    /// Returns the number of bytes in this binary.
    pub fn len(&self) -> usize {
        match self {
            Self::Inline(binary) => binary.len(),
            Self::Refc(proc_bin) => proc_bin.len(),
            Self::Sub(sub_binary) => sub_binary.len(),
        }
    }

    /// Returns true when this binary contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{
        binary::write_binary,
        shared_binary::{SharedBinary, write_proc_bin},
        sub_binary::write_sub_binary,
    };

    #[test]
    fn binary_ref_wraps_inline_binary() {
        let mut heap = [0_u64; 3];
        let term = write_binary(&mut heap, b"hello").expect("inline binary fits");
        let binary = BinaryRef::new(term).expect("binary ref");

        assert!(matches!(binary, BinaryRef::Inline(_)));
        assert_eq!(binary.len(), 5);
        assert_eq!(binary.as_bytes(HeapBorrow::of_words(&heap)), b"hello");
    }

    #[test]
    fn binary_ref_wraps_proc_bin() {
        let shared = SharedBinary::new(b"off-heap".to_vec());
        let mut heap = [0_u64; 3];
        let term = write_proc_bin(&mut heap, &shared).expect("proc bin fits");
        let binary = BinaryRef::new(term).expect("binary ref");

        assert!(matches!(binary, BinaryRef::Refc(_)));
        assert_eq!(binary.len(), 8);
        assert_eq!(binary.as_bytes(HeapBorrow::of_words(&heap)), b"off-heap");
    }

    #[test]
    fn binary_ref_wraps_sub_binary() {
        let shared = SharedBinary::new(b"0123456789abcdef".to_vec());
        let mut proc_heap = [0_u64; 3];
        let parent = write_proc_bin(&mut proc_heap, &shared).expect("proc bin fits");
        let mut sub_heap = [0_u64; 5];
        let term = write_sub_binary(&mut sub_heap, parent, 4, 6).expect("sub binary fits");
        let binary = BinaryRef::new(term).expect("binary ref");

        assert!(matches!(binary, BinaryRef::Sub(_)));
        assert_eq!(binary.len(), 6);
        assert_eq!(binary.as_bytes(HeapBorrow::of_words(&sub_heap)), b"456789");
    }

    #[test]
    fn binary_ref_rejects_non_binary_terms() {
        assert!(BinaryRef::new(Term::small_int(1)).is_none());
    }
}
