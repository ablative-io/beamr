//! Compile-time witness that boxed-term storage stays put for a borrow.
//!
//! Boxed terms are tagged pointers into storage a [`Term`](crate::term::Term)
//! does not name: a process heap region, or a plain word array in a test. The
//! byte- and limb-returning accessors need a lifetime for the slices they hand
//! out, and the only honest source for it is a *shared borrow of that storage*.
//!
//! [`HeapBorrow`] is that borrow, carried as a value. It is produced from a
//! shared reference — `Heap::borrow_terms`, `Process::borrow_terms`,
//! `ProcessContext::borrow_terms`, or [`HeapBorrow::of_words`] — so its
//! lifetime parameter is constrained by an *argument*, never inferred by the
//! caller. Every allocating or collecting path in the VM takes `&mut Heap` or
//! `&mut Process`, so a live `HeapBorrow` makes that `&mut` unobtainable: the
//! borrow checker rejects "hold a slice across a GC-triggering allocation"
//! before it can be written.

use core::marker::PhantomData;

/// A shared borrow of the word storage that backs boxed terms.
///
/// Copy and zero-sized: it carries no data, only the borrow region. Holding one
/// keeps the shared borrow it was made from alive, which is the whole mechanism
/// — see the module documentation.
#[derive(Copy, Clone, Debug)]
pub struct HeapBorrow<'heap> {
    storage: PhantomData<&'heap [u64]>,
}

impl<'heap> HeapBorrow<'heap> {
    /// Witnesses that `words` — the storage the terms being read live in — is
    /// shared-borrowed for `'heap`.
    ///
    /// This is the primitive the heap-owning types build on. Pass the storage
    /// the terms actually live in: the witness proves *a* borrow is live, it
    /// cannot check that a given term points inside it.
    #[must_use]
    pub const fn of_words(words: &'heap [u64]) -> Self {
        let _ = words;
        Self {
            storage: PhantomData,
        }
    }

    /// Runs `f` with a witness whose lifetime is bounded by this call frame.
    ///
    /// For readers that hold no heap handle at all because a `std` trait
    /// signature forbids one — `PartialEq`/`Ord` for `Term`, `Hash` for
    /// `EtsKey` — and therefore cannot be handed a real borrow. The witness is
    /// higher-ranked, so neither it nor any slice derived from it can escape
    /// `f`.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that the storage backing the terms read inside
    /// `f` is neither collected, reallocated nor dropped while `f` runs. In
    /// practice that means `f` must not allocate on a process heap, run a
    /// collection, or drop a `Heap`.
    pub(crate) unsafe fn with_frame<R>(f: impl for<'frame> FnOnce(HeapBorrow<'frame>) -> R) -> R {
        let frame: [u64; 0] = [];
        f(HeapBorrow::of_words(&frame))
    }

    /// Builds a borrowed slice whose lifetime is this witness's borrow.
    ///
    /// The single place the tied accessors turn a raw heap pointer into a
    /// slice. `'heap` comes from `self`, which was constrained by a real shared
    /// borrow at construction, so the returned lifetime cannot be widened by
    /// caller inference — that is the property the whole mechanism rests on.
    ///
    /// # Safety
    ///
    /// `ptr` must point at `len` consecutive initialised `T` values inside the
    /// storage this witness borrows, and that region must stay valid and
    /// unaliased-for-writes for `'heap`.
    pub(crate) const unsafe fn slice<T>(self, ptr: *const T, len: usize) -> &'heap [T] {
        // SAFETY: forwarded verbatim to the caller of this `unsafe fn` — the
        // pointer/length validity obligation is stated above; `'heap` is this
        // witness's own borrow region, so the reference cannot outlive it.
        unsafe { core::slice::from_raw_parts(ptr, len) }
    }
}
