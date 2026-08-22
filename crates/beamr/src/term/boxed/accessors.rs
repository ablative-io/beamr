//! Borrowed accessor structs for reading boxed term layouts.

use crate::atom::Atom;
use crate::term::Term;
use crate::term::heap_borrow::HeapBorrow;

use super::{BoxedHeader, BoxedTag};

/// Borrowed accessor for a tuple boxed term.
#[derive(Copy, Clone, Debug)]
pub struct Tuple {
    ptr: *const u64,
}

impl Tuple {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::Tuple)?;
        Some(Self { ptr })
    }

    pub fn arity(self) -> usize {
        BoxedHeader::size(self.header())
    }

    pub fn get(self, index: usize) -> Option<Term> {
        if index < self.arity() {
            Some(Term::from_raw(self.word(1 + index)))
        } else {
            None
        }
    }

    fn header(self) -> u64 {
        self.word(0)
    }

    fn word(self, offset: usize) -> u64 {
        // SAFETY: instances are only built from term pointers to stack/heap word
        // arrays created by this module; callers must keep the backing storage
        // alive while using the borrowed accessor.
        unsafe { *self.ptr.add(offset) }
    }
}

/// Borrowed accessor for a list cons cell.
#[derive(Copy, Clone, Debug)]
pub struct Cons {
    ptr: *const u64,
}

impl Cons {
    pub fn new(term: Term) -> Option<Self> {
        if !term.is_list() {
            return None;
        }

        Some(Self {
            ptr: term.heap_ptr()?,
        })
    }

    pub fn head(self) -> Term {
        Term::from_raw(self.word(0))
    }

    pub fn tail(self) -> Term {
        Term::from_raw(self.word(1))
    }

    fn word(self, offset: usize) -> u64 {
        // SAFETY: see Tuple::word; cons accessors read the fixed two-word cell.
        unsafe { *self.ptr.add(offset) }
    }
}

/// Borrowed accessor for a boxed float.
#[derive(Copy, Clone, Debug)]
pub struct Float {
    ptr: *const u64,
}

impl Float {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::Float)?;
        Some(Self { ptr })
    }

    pub fn value(self) -> f64 {
        // SAFETY: float payload is one u64 word immediately after the header.
        f64::from_bits(unsafe { *self.ptr.add(1) })
    }
}

/// Borrowed accessor for a boxed big integer storage layout.
#[derive(Copy, Clone, Debug)]
pub struct BigInt {
    ptr: *const u64,
}

impl BigInt {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::BigInt)?;
        Some(Self { ptr })
    }

    pub fn is_negative(self) -> bool {
        self.word(1) == super::BIGINT_NEGATIVE_SIGN
    }

    pub fn limb_count(self) -> usize {
        self.word(2) as usize
    }

    /// Limb words of this bignum, borrowed for the witness's borrow.
    ///
    /// The limbs live *in* the process heap, so a collection that moves or
    /// reclaims this object invalidates them. `heap` is a shared borrow of that
    /// storage and every collecting path needs `&mut Heap`/`&mut Process`, so
    /// the borrow checker rejects any attempt to hold these limbs across one.
    /// The bound is a type error, not a convention.
    ///
    /// The proof is a matched pair. First the positive control — the same
    /// program *without* the collection, which compiles and runs:
    ///
    /// ```
    /// use beamr::process::Process;
    /// use beamr::term::boxed::{BigInt, write_bigint};
    ///
    /// let mut process = Process::new(1, 233);
    /// let words = process.heap_mut().alloc_slice(5).expect("words");
    /// let term = write_bigint(words, false, &[7, 9]).expect("bigint");
    /// let bigint = BigInt::new(term).expect("accessor");
    /// let limbs = bigint.limbs(process.borrow_terms());
    /// assert_eq!(limbs, &[7, 9]);
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
    /// use beamr::term::boxed::{BigInt, write_bigint};
    ///
    /// let mut process = Process::new(1, 233);
    /// let words = process.heap_mut().alloc_slice(5).expect("words");
    /// let term = write_bigint(words, false, &[7, 9]).expect("bigint");
    /// let bigint = BigInt::new(term).expect("accessor");
    /// let limbs = bigint.limbs(process.borrow_terms());
    /// let _ = beamr::gc::collect_minor(&mut process);
    /// assert_eq!(limbs, &[7, 9]);
    /// ```
    pub fn limbs<'heap>(self, heap: HeapBorrow<'heap>) -> &'heap [u64] {
        let count = self.limb_count();
        // SAFETY: `limb_count` is the word written by `write_bigint`, so `count`
        // limb words follow the three-word header inside this bignum's boxed
        // object. `heap` witnesses a live shared borrow of the storage that
        // object sits in, which keeps the words valid and immovable for
        // `'heap`; the returned slice cannot outlive that borrow.
        unsafe { heap.slice(self.ptr.add(3), count) }
    }

    fn word(self, offset: usize) -> u64 {
        // SAFETY: see Tuple::word.
        unsafe { *self.ptr.add(offset) }
    }
}

/// Borrowed accessor for a boxed closure.
#[derive(Copy, Clone, Debug)]
pub struct Closure {
    ptr: *const u64,
}

impl Closure {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::Closure)?;
        // SAFETY: `header_ptr` returned a boxed closure header pointer.
        let header = unsafe { *ptr };
        let size = BoxedHeader::size(header);
        if size < 6 {
            return None;
        }

        // SAFETY: closure payloads of size at least six contain the num_free
        // word at offset four. Reject inconsistent sizes before exposing the
        // accessor so metadata/free-var reads stay within the boxed object.
        let num_free = unsafe { *ptr.add(4) } as usize;
        if size != 6 + num_free {
            return None;
        }

        Some(Self { ptr })
    }

    pub fn module(self) -> Option<Atom> {
        Term::from_raw(self.word(1)).as_atom()
    }

    pub fn function_index(self) -> u64 {
        self.word(2)
    }

    pub fn arity(self) -> u8 {
        self.word(3) as u8
    }

    pub fn num_free(self) -> usize {
        self.word(4) as usize
    }

    pub fn generation(self) -> u64 {
        self.word(5)
    }

    pub fn unique_id(self) -> u64 {
        self.word(6)
    }

    pub fn free_var(self, index: usize) -> Option<Term> {
        if index < self.num_free() {
            Some(Term::from_raw(self.word(7 + index)))
        } else {
            None
        }
    }

    /// True when this closure is an export fun (`fun M:F/A`) written by
    /// `write_export_fun`, marked by the sentinel generation.
    pub fn is_export(self) -> bool {
        self.generation() == super::EXPORT_FUN_GENERATION
    }

    /// Function atom of an export fun; `None` for ordinary closures.
    pub fn export_function(self) -> Option<Atom> {
        if self.is_export() {
            Term::from_raw(self.word(2)).as_atom()
        } else {
            None
        }
    }

    fn word(self, offset: usize) -> u64 {
        // SAFETY: see Tuple::word.
        unsafe { *self.ptr.add(offset) }
    }
}

/// Borrowed accessor for a flatmap boxed term.
#[derive(Copy, Clone, Debug)]
pub struct Map {
    ptr: *const u64,
}

impl Map {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::Map)?;
        Some(Self { ptr })
    }

    pub fn len(self) -> usize {
        self.word(1) as usize
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn key(self, index: usize) -> Option<Term> {
        if index < self.len() {
            Some(Term::from_raw(self.word(2 + index)))
        } else {
            None
        }
    }

    pub fn value(self, index: usize) -> Option<Term> {
        if index < self.len() {
            Some(Term::from_raw(self.word(2 + self.len() + index)))
        } else {
            None
        }
    }

    pub fn get(self, key: Term) -> Option<Term> {
        (0..self.len()).find_map(|index| {
            if self.key(index) == Some(key) {
                self.value(index)
            } else {
                None
            }
        })
    }

    fn word(self, offset: usize) -> u64 {
        // SAFETY: see Tuple::word.
        unsafe { *self.ptr.add(offset) }
    }
}

/// Borrowed accessor for a boxed reference.
#[derive(Copy, Clone, Debug)]
pub struct Reference {
    ptr: *const u64,
}

impl Reference {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::Reference)?;
        if BoxedHeader::size(word_at(ptr, 0)) != 1 {
            return None;
        }

        Some(Self { ptr })
    }

    pub fn id(self) -> u64 {
        // SAFETY: reference payload is one u64 id immediately after the header.
        unsafe { *self.ptr.add(1) }
    }
}

/// Borrowed accessor for a boxed remote PID.
#[derive(Copy, Clone, Debug)]
pub struct ExternalPid {
    ptr: *const u64,
}

impl ExternalPid {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::ExternalPid)?;
        if BoxedHeader::size(word_at(ptr, 0)) != 3
            || Term::from_raw(word_at(ptr, 1)).as_atom().is_none()
        {
            return None;
        }

        Some(Self { ptr })
    }

    pub fn node(self) -> Option<Atom> {
        Term::from_raw(self.word(1)).as_atom()
    }

    pub fn pid_number(self) -> u64 {
        self.word(2)
    }

    pub fn serial(self) -> u64 {
        self.word(3)
    }

    fn word(self, offset: usize) -> u64 {
        // SAFETY: external PID payload contains fixed words after the header.
        unsafe { *self.ptr.add(offset) }
    }
}

/// Borrowed accessor for a boxed remote reference.
#[derive(Copy, Clone, Debug)]
pub struct ExternalReference {
    ptr: *const u64,
}

impl ExternalReference {
    pub fn new(term: Term) -> Option<Self> {
        let ptr = header_ptr(term, BoxedTag::ExternalReference)?;
        if BoxedHeader::size(word_at(ptr, 0)) != 2
            || Term::from_raw(word_at(ptr, 1)).as_atom().is_none()
        {
            return None;
        }

        Some(Self { ptr })
    }

    pub fn node(self) -> Option<Atom> {
        Term::from_raw(self.word(1)).as_atom()
    }

    pub fn id(self) -> u64 {
        self.word(2)
    }

    fn word(self, offset: usize) -> u64 {
        // SAFETY: external reference payload contains fixed words after the header.
        unsafe { *self.ptr.add(offset) }
    }
}

fn word_at(ptr: *const u64, offset: usize) -> u64 {
    // SAFETY: caller has verified that `ptr` is a boxed object header pointer.
    unsafe { *ptr.add(offset) }
}

pub(super) fn header_ptr(term: Term, expected_tag: BoxedTag) -> Option<*const u64> {
    if !term.is_boxed() {
        return None;
    }

    let ptr = term.heap_ptr()?;
    // SAFETY: boxed terms point to a header word in caller-owned heap storage.
    let header = unsafe { *ptr };
    if BoxedHeader::tag(header) == Some(expected_tag) {
        Some(ptr)
    } else {
        None
    }
}
