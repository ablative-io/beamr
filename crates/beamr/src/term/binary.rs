//! Binary and bitstring representation.
//!
//! This brief implements inline heap binaries: a boxed binary header, followed
//! by the byte length and byte data packed directly into heap words.

use crate::term::heap_borrow::HeapBorrow;
use crate::term::{
    Term,
    boxed::{BoxedHeader, BoxedTag},
};

const WORD_BYTES: usize = std::mem::size_of::<u64>();

/// Number of heap words required to store `byte_len` bytes.
pub const fn packed_word_count(byte_len: usize) -> usize {
    byte_len.div_ceil(WORD_BYTES)
}

/// Writes an inline binary layout (`header, byte_len, packed bytes...`) into `heap`.
pub fn write_binary(heap: &mut [u64], bytes: &[u8]) -> Option<Term> {
    let data_words = packed_word_count(bytes.len());
    let required_words = 2 + data_words;
    if heap.len() < required_words {
        return None;
    }

    heap[0] = BoxedHeader::new(BoxedTag::Binary, 1 + data_words);
    heap[1] = bytes.len() as u64;
    heap[2..required_words].fill(0);

    for (index, byte) in bytes.iter().copied().enumerate() {
        let word_index = 2 + index / WORD_BYTES;
        let shift = (index % WORD_BYTES) * u8::BITS as usize;
        heap[word_index] |= u64::from(byte) << shift;
    }

    Some(Term::boxed_ptr(heap.as_ptr()))
}

/// Borrowed accessor for an inline heap binary.
#[derive(Copy, Clone, Debug)]
pub struct Binary {
    ptr: *const u64,
}

impl Binary {
    pub fn new(term: Term) -> Option<Self> {
        if !term.is_boxed() {
            return None;
        }

        let ptr = term.heap_ptr()?;
        // SAFETY: boxed binary terms point at a header word in live heap storage.
        let header = unsafe { *ptr };
        if BoxedHeader::tag(header) == Some(BoxedTag::Binary) {
            Some(Self { ptr })
        } else {
            None
        }
    }

    pub fn len(self) -> usize {
        // SAFETY: binary length is the first payload word after the header.
        unsafe { *self.ptr.add(1) as usize }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Bytes of this inline binary, borrowed for the witness's borrow.
    ///
    /// Inline binary bytes are packed *into* heap words, so a collection that
    /// moves or reclaims this object invalidates them. `heap` is a shared
    /// borrow of that storage and every collecting path needs `&mut Heap`/`&mut
    /// Process`, so holding these bytes across one does not type-check.
    /// The bound is a type error, not a convention.
    ///
    /// The proof is a matched pair. First the positive control — the same
    /// program *without* the collection, which compiles and runs:
    ///
    /// ```
    /// use beamr::process::Process;
    /// use beamr::term::binary::{Binary, write_binary};
    ///
    /// let mut process = Process::new(1, 233);
    /// let words = process.heap_mut().alloc_slice(3).expect("words");
    /// let term = write_binary(words, b"hello").expect("binary");
    /// let binary = Binary::new(term).expect("accessor");
    /// let bytes = binary.as_bytes(process.borrow_terms());
    /// assert_eq!(bytes, b"hello");
    /// ```
    ///
    /// Now the same program with one line added, the collection, and it no
    /// longer type-checks. `term::accessor_proof_tests` asserts in-gate that
    /// the two blocks differ by exactly that line, because on this
    /// repository's pinned toolchain rustdoc IGNORES the `E0502` annotation
    /// below — measured, see that module.
    ///
    /// ```compile_fail,E0502
    /// use beamr::process::Process;
    /// use beamr::term::binary::{Binary, write_binary};
    ///
    /// let mut process = Process::new(1, 233);
    /// let words = process.heap_mut().alloc_slice(3).expect("words");
    /// let term = write_binary(words, b"hello").expect("binary");
    /// let binary = Binary::new(term).expect("accessor");
    /// let bytes = binary.as_bytes(process.borrow_terms());
    /// let _ = beamr::gc::collect_minor(&mut process);
    /// assert_eq!(bytes, b"hello");
    /// ```
    pub fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8] {
        let len = self.len();
        // SAFETY: inline binary data starts after the header and length words,
        // and `write_binary` reserved `packed_word_count(len)` words for it, so
        // `len` bytes follow `ptr.add(2)` inside this boxed object. Bytes are
        // packed in native little-endian word order by `write_binary`; tests
        // and consumers read the same in-process representation. `heap`
        // witnesses a live shared borrow of the storage holding those words, so
        // they stay valid and immovable for `'heap` and the returned slice
        // cannot outlive that borrow.
        unsafe { heap.slice(self.ptr.add(2).cast::<u8>(), len) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_byte_binary_occupies_three_words_and_round_trips_bytes() {
        let bytes = b"hello";
        let mut heap = [0_u64; 3];
        let term = write_binary(&mut heap, bytes).expect("binary should fit");
        let binary = Binary::new(term).expect("binary accessor");

        assert!(term.is_boxed());
        assert_eq!(BoxedHeader::tag(heap[0]), Some(BoxedTag::Binary));
        assert_eq!(BoxedHeader::size(heap[0]), 2);
        assert_eq!(packed_word_count(bytes.len()), 1);
        assert_eq!(heap.len(), 3);
        assert_eq!(binary.len(), 5);
        assert_eq!(binary.as_bytes(HeapBorrow::of_words(&heap)), bytes);
    }

    #[test]
    fn empty_binary_is_valid_with_zero_length() {
        let mut heap = [0_u64; 2];
        let term = write_binary(&mut heap, &[]).expect("empty binary should fit");
        let binary = Binary::new(term).expect("binary accessor");

        assert_eq!(BoxedHeader::tag(heap[0]), Some(BoxedTag::Binary));
        assert_eq!(BoxedHeader::size(heap[0]), 1);
        assert_eq!(binary.len(), 0);
        assert!(binary.is_empty());
        assert_eq!(binary.as_bytes(HeapBorrow::of_words(&heap)), b"");
    }

    #[test]
    fn binary_writer_rejects_too_small_heap_slice() {
        let mut heap = [0_u64; 2];
        assert_eq!(write_binary(&mut heap, b"hello"), None);
    }
}
