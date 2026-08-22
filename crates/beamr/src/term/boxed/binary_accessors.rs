//! Borrowed accessors for the boxed binary families.
//!
//! Split out of `accessors.rs` so the byte-returning accessors, their witness
//! plumbing and their re-derived safety comments have room without pushing that
//! module past the repository's 500-line file wall.

use crate::term::heap_borrow::HeapBorrow;
use crate::term::{Term, binary::Binary, shared_binary::SharedBinary};

use super::accessors::header_ptr;
use super::{BoxedHeader, BoxedTag};

/// Borrowed accessor for an off-heap reference-counted binary.
#[derive(Copy, Clone, Debug)]
pub struct ProcBin {
    ptr: *const u64,
}

impl ProcBin {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::ProcBin)?;
        // SAFETY: `header_ptr` returned a boxed ProcBin header pointer.
        let header = unsafe { *ptr };
        if BoxedHeader::size(header) != 2 {
            return None;
        }
        // SAFETY: validated ProcBin layout has two payload words; word two is
        // the raw Arc pointer and must be present/non-null before access.
        if unsafe { *ptr.add(2) } == 0 {
            return None;
        }

        Some(Self { ptr })
    }

    /// Bytes of the off-heap buffer, borrowed for the witness's borrow.
    ///
    /// The buffer itself is an `Arc<Vec<u8>>` that GC never moves, but GC does
    /// *release* it — `gc::release_refcounted_resources_in_young` and its
    /// siblings drop the strong reference this ProcBin owns when the ProcBin
    /// dies in a collection. The bound is therefore the ProcBin's liveness,
    /// which is the heap's, which is exactly what `heap` witnesses.
    /// The bound is a type error, not a convention — and it covers the
    /// `Arc` release, not just object motion:
    ///
    /// ```compile_fail,E0502
    /// use beamr::process::Process;
    /// use beamr::term::boxed::ProcBin;
    /// use beamr::term::shared_binary::{SharedBinary, write_proc_bin};
    ///
    /// let mut process = Process::new(1, 233);
    /// let shared = SharedBinary::new(b"off-heap".to_vec());
    /// let words = process.heap_mut().alloc_slice_maybe_refcounted(3).expect("words");
    /// let term = write_proc_bin(words, &shared).expect("proc bin");
    /// let proc_bin = ProcBin::new(term).expect("accessor");
    /// let bytes = proc_bin.as_bytes(process.borrow_terms());
    /// let _ = beamr::gc::collect_minor(&mut process);
    /// assert_eq!(bytes, b"off-heap");
    /// ```
    pub fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8] {
        SharedBinary::bytes_from_raw_word(self.arc_ptr_word(), heap)
    }

    pub fn len(self) -> usize {
        SharedBinary::len_from_raw_word(self.arc_ptr_word())
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn shared_binary(self) -> SharedBinary {
        SharedBinary::clone_from_raw_word(self.arc_ptr_word())
    }

    fn arc_ptr_word(self) -> u64 {
        // SAFETY: ProcBin payload word two stores the raw `Arc<Vec<u8>>` pointer.
        unsafe { *self.ptr.add(2) }
    }
}

/// Borrowed accessor for a sub-binary view into an inline Binary or ProcBin.
#[derive(Copy, Clone, Debug)]
pub struct SubBinary {
    ptr: *const u64,
}

impl SubBinary {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::SubBinary)?;
        // SAFETY: `header_ptr` returned a boxed SubBinary header pointer.
        let header = unsafe { *ptr };
        if BoxedHeader::size(header) != 4 {
            return None;
        }

        let sub_binary = Self { ptr };
        let parent_len = parent_len(sub_binary.parent())?;
        let end = sub_binary.offset().checked_add(sub_binary.len())?;
        if end > parent_len {
            return None;
        }

        Some(sub_binary)
    }

    pub fn parent(self) -> Term {
        Term::from_raw(self.word(1))
    }

    pub fn len(self) -> usize {
        self.word(3) as usize
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Bytes of the parent binary this sub-binary views, borrowed for the
    /// witness's borrow: the view is a sub-slice of the parent's storage, so it
    /// carries the parent's bound unchanged.
    /// The bound is a type error, not a convention. This path also proves the
    /// private `parent_bytes` helper, which is the only other reader of the
    /// parent's storage:
    ///
    /// ```compile_fail,E0502
    /// use beamr::process::Process;
    /// use beamr::term::binary::write_binary;
    /// use beamr::term::boxed::SubBinary;
    /// use beamr::term::sub_binary::write_sub_binary;
    ///
    /// let mut process = Process::new(1, 233);
    /// let parent_words = process.heap_mut().alloc_slice(4).expect("parent");
    /// let parent = write_binary(parent_words, b"0123456789abcdef").expect("binary");
    /// let sub_words = process.heap_mut().alloc_slice(5).expect("sub");
    /// let term = write_sub_binary(sub_words, parent, 4, 6).expect("sub binary");
    /// let sub_binary = SubBinary::new(term).expect("accessor");
    /// let bytes = sub_binary.as_bytes(process.borrow_terms());
    /// let _ = beamr::gc::collect_minor(&mut process);
    /// assert_eq!(bytes, b"456789");
    /// ```
    pub fn as_bytes<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u8] {
        let bytes = parent_bytes(self.parent(), heap).unwrap_or(&[]);
        let start = self.offset();
        let end = start.checked_add(self.len()).unwrap_or(start);
        bytes.get(start..end).unwrap_or(&[])
    }

    fn offset(self) -> usize {
        self.word(2) as usize
    }

    fn word(self, offset: usize) -> u64 {
        // SAFETY: validated SubBinary layout contains fixed payload words.
        unsafe { *self.ptr.add(offset) }
    }
}

fn parent_bytes<'heap>(parent: Term, heap: HeapBorrow<'heap>) -> Option<&'heap [u8]> {
    if let Some(binary) = Binary::new(parent) {
        return Some(binary.as_bytes(heap));
    }
    ProcBin::new(parent).map(|proc_bin| proc_bin.as_bytes(heap))
}

/// Byte length of a sub-binary's parent, without materialising a borrow.
///
/// `SubBinary::new` only needs the parent's length to bounds-check the view.
/// Reading it through the length accessors instead of `parent_bytes` keeps the
/// constructor witness-free, so building the accessor stays as cheap as before
/// and only *reading bytes* needs a live heap borrow.
fn parent_len(parent: Term) -> Option<usize> {
    if let Some(binary) = Binary::new(parent) {
        return Some(binary.len());
    }
    ProcBin::new(parent).map(ProcBin::len)
}
