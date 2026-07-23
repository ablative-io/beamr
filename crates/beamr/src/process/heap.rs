//! Per-process generational heap allocator.
//!
//! Each process owns two private bump regions: a young generation (nursery) for
//! new allocations and an old generation for promoted/live data. Regions use
//! separate backing vectors so nursery and old objects are never mixed in the
//! same memory area. The backing vectors are pre-sized and never grow while live
//! pointers may refer into them; GC replaces regions only after rewriting roots.

use std::fmt;

use crate::term::boxed::{BoxedHeader, BoxedTag};

/// Default per-process heap capacity, in machine words.
pub const DEFAULT_HEAP_SIZE: usize = 233;

/// Default maximum young-generation heap capacity, in machine words (1 MiB).
pub const DEFAULT_MAX_HEAP_WORDS: usize = 131_072;

const DEFAULT_OLD_HEAP_SIZE: usize = DEFAULT_HEAP_SIZE;

/// Error returned when a heap allocation cannot be satisfied.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct HeapFull {
    requested: usize,
    available: usize,
}

impl HeapFull {
    /// Create a heap-full error for an unsatisfied word request.
    #[must_use]
    pub const fn new(requested: usize, available: usize) -> Self {
        Self {
            requested,
            available,
        }
    }

    /// Number of words requested by the failed allocation.
    #[must_use]
    pub const fn requested(self) -> usize {
        self.requested
    }

    /// Number of free words remaining when the allocation failed.
    #[must_use]
    pub const fn available(self) -> usize {
        self.available
    }
}

impl fmt::Display for HeapFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "heap full: requested {} words with {} available",
            self.requested, self.available
        )
    }
}

impl std::error::Error for HeapFull {}

/// Whether an allocation may hold a reference-counted off-heap resource.
///
/// Only `ProcBin` and `FdResource` own an `Arc` that the GC must release, and
/// only they are ever inspected by the refcounted-resource release walk. Every
/// other allocation — crucially including headerless cons cells, whose head
/// word can alias a boxed header tag (e.g. the atom `false` encodes to the same
/// word as `BoxedTag::ProcBin`) — defaults to [`AllocKind::NotRefcounted`] and
/// is skipped by the walk. This makes the walk's classification a recorded fact
/// rather than an inference over ambiguous word[0] contents: a cons cell can
/// never be mistaken for a live `ProcBin` and freed via `Arc::from_raw`.
///
/// The direction is fail-safe: a refcounted allocation that is mistakenly left
/// `NotRefcounted` leaks its `Arc` (caught by the no-leak tests), whereas the
/// reverse — a non-refcounted allocation treated as refcounted — is the
/// use-after-free this type exists to prevent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AllocKind {
    NotRefcounted,
    MaybeRefcounted,
}

/// Bookkeeping for one bump allocation: its offset, word count, and whether the
/// GC release walk must inspect it for a refcounted resource.
#[derive(Clone, Copy, Debug)]
struct Allocation {
    offset: usize,
    words: usize,
    kind: AllocKind,
}

/// One fixed-capacity bump region inside a process heap.
#[derive(Clone, Debug)]
pub(crate) struct HeapRegion {
    words: Vec<u64>,
    allocations: Vec<Allocation>,
    used: usize,
    high_water_mark: usize,
}

impl HeapRegion {
    fn new(capacity: usize) -> Self {
        Self {
            words: vec![0; capacity],
            allocations: Vec::new(),
            used: 0,
            high_water_mark: 0,
        }
    }

    fn alloc(&mut self, words: usize) -> Result<*mut u64, HeapFull> {
        self.alloc_with_kind(words, AllocKind::NotRefcounted)
    }

    fn alloc_with_kind(&mut self, words: usize, kind: AllocKind) -> Result<*mut u64, HeapFull> {
        let Some(end) = self.used.checked_add(words) else {
            return Err(HeapFull::new(words, self.available()));
        };

        if end > self.capacity() {
            return Err(HeapFull::new(words, self.available()));
        }

        let start = self.used;
        let ptr = self.words.as_mut_ptr().wrapping_add(start);
        self.used = end;
        self.high_water_mark = self.high_water_mark.max(self.used);
        self.allocations.push(Allocation {
            offset: start,
            words,
            kind,
        });
        Ok(ptr)
    }

    fn alloc_slice(&mut self, words: usize) -> Result<&mut [u64], HeapFull> {
        self.alloc_slice_with_kind(words, AllocKind::NotRefcounted)
    }

    fn alloc_slice_with_kind(
        &mut self,
        words: usize,
        kind: AllocKind,
    ) -> Result<&mut [u64], HeapFull> {
        let Some(end) = self.used.checked_add(words) else {
            return Err(HeapFull::new(words, self.available()));
        };

        if end > self.capacity() {
            return Err(HeapFull::new(words, self.available()));
        }

        let start = self.used;
        self.used = end;
        self.high_water_mark = self.high_water_mark.max(self.used);
        self.allocations.push(Allocation {
            offset: start,
            words,
            kind,
        });
        Ok(&mut self.words[start..end])
    }

    /// Mark the most recent allocation as possibly holding a refcounted
    /// resource, so the GC release walk inspects it. Used where a boxed
    /// `ProcBin`/`FdResource` is written into a slice obtained from a plain
    /// `alloc`/`alloc_slice`.
    fn mark_last_allocation_maybe_refcounted(&mut self) {
        if let Some(last) = self.allocations.last_mut() {
            last.kind = AllocKind::MaybeRefcounted;
        }
    }

    pub(crate) const fn used(&self) -> usize {
        self.used
    }

    pub(crate) fn capacity(&self) -> usize {
        self.words.len()
    }

    const fn high_water_mark(&self) -> usize {
        self.high_water_mark
    }

    fn available(&self) -> usize {
        self.capacity().saturating_sub(self.used)
    }

    fn reset(&mut self) {
        self.words[..self.used].fill(0);
        self.allocations.clear();
        self.used = 0;
    }

    pub(crate) fn contains(&self, ptr: *const u64) -> bool {
        let start = self.words.as_ptr().addr();
        let end = start.saturating_add(self.capacity() * std::mem::size_of::<u64>());
        let addr = ptr.addr();
        addr >= start && addr < end
    }

    pub(crate) fn visit_allocated_boxed_objects(
        &self,
        mut visit: impl FnMut(*const u64, BoxedTag, usize),
    ) {
        for allocation in &self.allocations {
            // Only refcounted allocations carry a real boxed header whose
            // word[0] may be read as a tag. Skipping the rest is what makes a
            // headerless cons cell (whose head word can alias a boxed tag)
            // unable to be misread as a live ProcBin and freed. See [`AllocKind`].
            if allocation.kind != AllocKind::MaybeRefcounted {
                continue;
            }
            let ptr = self.words.as_ptr().wrapping_add(allocation.offset);
            let header = self.words[allocation.offset];
            if let Some(tag) = BoxedHeader::tag(header) {
                let object_words = 1 + BoxedHeader::size(header);
                if object_words <= allocation.words {
                    visit(ptr, tag, object_words);
                }
            }
        }
    }

    /// Visit every allocation whose first word parses as a boxed header,
    /// regardless of [`AllocKind`]. A headerless cons cell whose head word
    /// aliases a boxed tag is misreported here, so this walk is for
    /// display/inspection only — it must never drive resource release.
    fn visit_allocated_boxed_objects_unfiltered(
        &self,
        mut visit: impl FnMut(*const u64, BoxedTag, usize),
    ) {
        for allocation in &self.allocations {
            let ptr = self.words.as_ptr().wrapping_add(allocation.offset);
            let header = self.words[allocation.offset];
            if let Some(tag) = BoxedHeader::tag(header) {
                let object_words = 1 + BoxedHeader::size(header);
                if object_words <= allocation.words {
                    visit(ptr, tag, object_words);
                }
            }
        }
    }
}

/// Generational bump allocator for one process heap.
#[derive(Clone, Debug)]
pub struct Heap {
    young: HeapRegion,
    old: HeapRegion,
    initial_capacity: usize,
    previous_capacity: usize,
    max_capacity: usize,
}

impl Heap {
    /// Create a heap with room for `capacity` machine words in the nursery.
    ///
    /// `capacity()` reports nursery capacity because raw `alloc` targets the
    /// nursery. The old generation is a distinct region with its own capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let previous_capacity = fibonacci_previous(capacity);
        Self {
            young: HeapRegion::new(capacity),
            old: HeapRegion::new(DEFAULT_OLD_HEAP_SIZE.max(capacity)),
            initial_capacity: capacity,
            previous_capacity,
            max_capacity: DEFAULT_MAX_HEAP_WORDS.max(capacity),
        }
    }

    /// Create a heap with an explicit maximum young-generation capacity.
    #[must_use]
    pub fn with_max_heap_size(capacity: usize, max_capacity: usize) -> Self {
        let mut heap = Self::new(capacity);
        heap.max_capacity = max_capacity.max(heap.young_capacity());
        heap
    }

    /// Allocate `words` contiguous machine words from the young generation.
    pub fn alloc(&mut self, words: usize) -> Result<*mut u64, HeapFull> {
        self.young.alloc(words)
    }

    /// Allocate `words` from the young generation for a boxed object that may
    /// own a refcounted off-heap resource (`ProcBin`/`FdResource`), so the GC
    /// release walk inspects it. See [`AllocKind`].
    pub fn alloc_maybe_refcounted(&mut self, words: usize) -> Result<*mut u64, HeapFull> {
        self.young
            .alloc_with_kind(words, AllocKind::MaybeRefcounted)
    }

    /// Allocate `words` contiguous machine words from the young generation.
    pub fn alloc_slice(&mut self, words: usize) -> Result<&mut [u64], HeapFull> {
        self.young.alloc_slice(words)
    }

    /// Slice counterpart of [`Heap::alloc_maybe_refcounted`].
    pub fn alloc_slice_maybe_refcounted(&mut self, words: usize) -> Result<&mut [u64], HeapFull> {
        self.young
            .alloc_slice_with_kind(words, AllocKind::MaybeRefcounted)
    }

    /// Mark the most recent young allocation as possibly refcounted. Used where
    /// a `ProcBin`/`FdResource` is written into a slice from a plain `alloc`.
    pub fn mark_last_young_allocation_maybe_refcounted(&mut self) {
        self.young.mark_last_allocation_maybe_refcounted();
    }

    /// Mark the most recent old-generation allocation as possibly refcounted.
    /// Used by the copying collector when it promotes a boxed object.
    pub(crate) fn mark_last_old_allocation_maybe_refcounted(&mut self) {
        self.old.mark_last_allocation_maybe_refcounted();
    }

    /// Allocate `words` contiguous machine words from the old generation.
    pub(crate) fn alloc_old(&mut self, words: usize) -> Result<*mut u64, HeapFull> {
        self.old.alloc(words)
    }

    /// Allocate from a standalone fresh old-space region used during major GC.
    pub(crate) fn alloc_in_region(
        region: &mut HeapRegion,
        words: usize,
    ) -> Result<*mut u64, HeapFull> {
        region.alloc(words)
    }

    /// Refcounted counterpart of [`Heap::alloc_in_region`]: the copied object may
    /// be a `ProcBin`/`FdResource`, so the compacted region must expose it to the
    /// release walk. See [`AllocKind`].
    pub(crate) fn alloc_in_region_maybe_refcounted(
        region: &mut HeapRegion,
        words: usize,
    ) -> Result<*mut u64, HeapFull> {
        region.alloc_with_kind(words, AllocKind::MaybeRefcounted)
    }

    /// Build a fresh old-space region for major compaction.
    pub(crate) fn fresh_old_region(&self, capacity: usize) -> HeapRegion {
        HeapRegion::new(capacity.max(self.initial_capacity))
    }

    /// Replace old generation with a compacted fresh region.
    pub(crate) fn replace_old(&mut self, region: HeapRegion) {
        self.old = region;
    }

    /// Number of words currently allocated in the young generation.
    #[must_use]
    pub const fn used(&self) -> usize {
        self.young.used()
    }

    /// Total words currently allocated across young and old generations.
    #[must_use]
    pub const fn total_used(&self) -> usize {
        self.young.used() + self.old.used()
    }

    /// Young-generation word capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.young.capacity()
    }

    /// Configured maximum young-generation word capacity for GC-triggered growth.
    #[must_use]
    pub const fn max_capacity(&self) -> usize {
        self.max_capacity
    }

    /// Set the maximum young-generation word capacity for future checked growth.
    pub fn set_max_capacity(&mut self, max_capacity: usize) {
        self.max_capacity = max_capacity.max(self.young_capacity());
    }

    /// Total capacity across young and old generations.
    #[must_use]
    pub fn total_capacity(&self) -> usize {
        self.young_capacity() + self.old_capacity()
    }

    /// Young-generation word capacity.
    #[must_use]
    pub fn young_capacity(&self) -> usize {
        self.young.capacity()
    }

    /// Old-generation word capacity.
    #[must_use]
    pub fn old_capacity(&self) -> usize {
        self.old.capacity()
    }

    /// Words currently allocated in the young generation.
    #[must_use]
    pub const fn young_used(&self) -> usize {
        self.young.used()
    }

    /// Words currently allocated in the old generation.
    #[must_use]
    pub const fn old_used(&self) -> usize {
        self.old.used()
    }

    /// Maximum nursery words allocated at once since heap creation or growth.
    #[must_use]
    pub const fn high_water_mark(&self) -> usize {
        self.young.high_water_mark()
    }

    /// Number of words available before nursery allocation reports [`HeapFull`].
    #[must_use]
    pub fn available(&self) -> usize {
        self.young.available()
    }

    /// Number of words available in old space before promotion fails.
    #[must_use]
    pub fn old_available(&self) -> usize {
        self.old.available()
    }

    /// True if `ptr` points into the currently allocated young region storage.
    #[must_use]
    pub fn young_contains(&self, ptr: *const u64) -> bool {
        self.young.contains(ptr)
    }

    /// True if `ptr` points into the currently allocated old region storage.
    #[must_use]
    pub fn old_contains(&self, ptr: *const u64) -> bool {
        self.old.contains(ptr)
    }

    /// True if `ptr` points into any current heap region storage.
    #[must_use]
    pub fn contains(&self, ptr: *const u64) -> bool {
        self.young_contains(ptr) || self.old_contains(ptr)
    }

    pub(crate) fn visit_young_boxed_objects(&self, visit: impl FnMut(*const u64, BoxedTag, usize)) {
        self.young.visit_allocated_boxed_objects(visit);
    }

    pub(crate) fn visit_boxed_objects(&self, mut visit: impl FnMut(*const u64, BoxedTag, usize)) {
        self.young.visit_allocated_boxed_objects(&mut visit);
        self.old.visit_allocated_boxed_objects(visit);
    }

    /// Header-sniffing census of both regions for debugger display. Unlike
    /// [`Heap::visit_boxed_objects`], this ignores [`AllocKind`], so a cons
    /// cell whose head aliases a boxed tag can be misreported — never use this
    /// walk for resource release.
    pub(crate) fn visit_boxed_objects_for_inspection(
        &self,
        mut visit: impl FnMut(*const u64, BoxedTag, usize),
    ) {
        self.young
            .visit_allocated_boxed_objects_unfiltered(&mut visit);
        self.old.visit_allocated_boxed_objects_unfiltered(visit);
    }

    pub(crate) fn rebase_snapshot_terms(&mut self, original: &Heap) {
        let mappings = original.rebase_mappings(self);
        self.rebase_embedded_terms(&mappings);
    }

    pub(crate) fn rebase_term_from(
        &self,
        term: crate::term::Term,
        original: &Heap,
    ) -> crate::term::Term {
        let mappings = original.rebase_mappings(self);
        match rebase_term(term, &mappings) {
            Some(rebased) => rebased,
            None => term,
        }
    }

    /// Reclaim the nursery wholesale after all live young objects are promoted.
    pub(crate) fn reset_young(&mut self) {
        self.young.reset();
    }

    /// Grow young generation to the next Fibonacci-like capacity.
    pub fn grow_to_next_capacity(&mut self) {
        let next = self.next_capacity();
        self.grow_young_to(next);
    }

    /// Grow young generation to the next Fibonacci-like capacity if within max.
    pub fn grow_to_next_capacity_with_max(&mut self) -> Result<(), HeapFull> {
        let next = self.next_capacity();
        if next > self.max_capacity {
            return Err(HeapFull::new(next, self.available()));
        }
        self.grow_young_to(next);
        Ok(())
    }

    fn next_capacity(&self) -> usize {
        let current = self.young_capacity();
        current
            .saturating_add(self.previous_capacity)
            .max(current.saturating_add(1))
    }

    fn grow_young_to(&mut self, next: usize) {
        self.previous_capacity = self.young_capacity();
        self.young = HeapRegion::new(next);
    }

    /// Capacity to use for compacted old space after major GC copied `live_words`.
    pub(crate) fn compacted_old_capacity_after_major(
        &self,
        live_words: usize,
        threshold: f64,
    ) -> usize {
        let minimum = live_words.max(self.initial_capacity);
        let target = fibonacci_capacity_for(minimum);
        let utilization = live_words as f64 / self.old_capacity() as f64;
        if utilization < threshold && self.old_capacity() > self.initial_capacity {
            target.min(self.old_capacity()).max(minimum)
        } else {
            self.old_capacity().max(minimum)
        }
    }

    /// Test helper: enlarge empty old space so shrink policy can be exercised.
    #[cfg(test)]
    pub(crate) fn grow_empty_old_to_for_test(&mut self, capacity: usize) {
        debug_assert_eq!(self.old.used(), 0);
        if capacity > self.old_capacity() {
            self.old = HeapRegion::new(capacity);
        }
    }

    pub(crate) fn copy_words_from_ptr(&self, src: *const u64, len: usize) -> Vec<u64> {
        // SAFETY: GC computes object sizes from valid object headers/cell tags;
        // `src..src+len` belongs to the source heap region while copying.
        unsafe { std::slice::from_raw_parts(src, len).to_vec() }
    }

    pub(crate) fn write_words(dst: *mut u64, words: &[u64]) {
        // SAFETY: destination is freshly allocated for exactly `words.len()`
        // words in a heap region and does not overlap the temporary source vec.
        unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), dst, words.len()) }
    }

    fn rebase_mappings(&self, cloned: &Self) -> [(*const u64, *const u64, usize); 2] {
        [
            (
                self.young.words.as_ptr(),
                cloned.young.words.as_ptr(),
                self.young.words.len(),
            ),
            (
                self.old.words.as_ptr(),
                cloned.old.words.as_ptr(),
                self.old.words.len(),
            ),
        ]
    }

    fn rebase_embedded_terms(&mut self, mappings: &[(*const u64, *const u64, usize)]) {
        rebase_region_embedded_terms(&mut self.young, mappings);
        rebase_region_embedded_terms(&mut self.old, mappings);
    }
}

fn rebase_region_embedded_terms(
    region: &mut HeapRegion,
    mappings: &[(*const u64, *const u64, usize)],
) {
    for allocation in region.allocations.clone() {
        let Some(block) = region
            .words
            .get_mut(allocation.offset..allocation.offset.saturating_add(allocation.words))
        else {
            continue;
        };
        rebase_heap_block_terms(block, mappings);
    }
}

fn rebase_heap_block_terms(block: &mut [u64], mappings: &[(*const u64, *const u64, usize)]) {
    let Some((header, payload)) = block.split_first_mut() else {
        return;
    };

    match BoxedHeader::tag(*header) {
        Some(BoxedTag::Tuple) => {
            for word in payload.iter_mut().take(BoxedHeader::size(*header)) {
                rebase_term_word(word, mappings);
            }
        }
        Some(BoxedTag::Map) => {
            let len = payload.first().copied().unwrap_or(0) as usize;
            for word in payload.iter_mut().skip(1).take(len.saturating_mul(2)) {
                rebase_term_word(word, mappings);
            }
        }
        Some(BoxedTag::Closure) => {
            let num_free = payload.get(3).copied().unwrap_or(0) as usize;
            for word in payload.iter_mut().skip(6).take(num_free) {
                rebase_term_word(word, mappings);
            }
        }
        Some(BoxedTag::MatchContext) => {
            if let Some(word) = payload.get_mut(2) {
                rebase_term_word(word, mappings);
            }
        }
        Some(BoxedTag::SubBinary) => {
            if let Some(word) = payload.first_mut() {
                rebase_term_word(word, mappings);
            }
        }
        Some(
            BoxedTag::Float
            | BoxedTag::BigInt
            | BoxedTag::Reference
            | BoxedTag::Binary
            | BoxedTag::BinaryBuilder
            | BoxedTag::ProcBin
            | BoxedTag::FdResource
            | BoxedTag::ExternalPid
            | BoxedTag::ExternalReference,
        ) => {}
        None if block.len() >= 2 => {
            rebase_term_word(&mut block[0], mappings);
            rebase_term_word(&mut block[1], mappings);
        }
        None => {}
    }
}

fn rebase_term_word(word: &mut u64, mappings: &[(*const u64, *const u64, usize)]) {
    if let Some(rebased) = rebase_term(crate::term::Term::from_raw(*word), mappings) {
        *word = rebased.raw();
    }
}

fn rebase_term(
    term: crate::term::Term,
    mappings: &[(*const u64, *const u64, usize)],
) -> Option<crate::term::Term> {
    if !term.is_boxed() && !term.is_list() {
        return None;
    }

    let ptr = term.heap_ptr()?;
    let word_size = std::mem::size_of::<u64>();
    for &(original, cloned, len) in mappings {
        let start = original as usize;
        let byte_len = len.checked_mul(word_size)?;
        let end = start.checked_add(byte_len)?;
        let ptr = ptr as usize;
        if ptr < start || ptr >= end {
            continue;
        }
        let offset = ptr.checked_sub(start)?;
        if !offset.is_multiple_of(word_size) {
            return None;
        }
        let rebased = cloned.wrapping_add(offset / word_size);
        return Some(if term.is_boxed() {
            crate::term::Term::boxed_ptr(rebased)
        } else {
            crate::term::Term::list_ptr(rebased)
        });
    }
    None
}

impl Default for Heap {
    fn default() -> Self {
        Self::new(DEFAULT_HEAP_SIZE)
    }
}

fn fibonacci_previous(capacity: usize) -> usize {
    let mut prev = 144;
    let mut current = DEFAULT_HEAP_SIZE;
    while current < capacity {
        let next = prev + current;
        prev = current;
        current = next;
    }
    prev.min(capacity)
}

fn fibonacci_capacity_for(needed: usize) -> usize {
    let mut previous = 144;
    let mut current = DEFAULT_HEAP_SIZE;
    while current < needed {
        let next = previous + current;
        previous = current;
        current = next;
    }
    current
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_HEAP_SIZE, Heap, HeapFull};

    #[test]
    fn new_heap_reports_young_capacity_and_zero_used() {
        let heap = Heap::new(1024);

        assert_eq!(heap.capacity(), 1024);
        assert_eq!(heap.young_capacity(), 1024);
        assert_eq!(heap.used(), 0);
        assert_eq!(heap.young_used(), 0);
        assert_eq!(heap.old_used(), 0);
        assert_eq!(heap.high_water_mark(), 0);
    }

    #[test]
    fn alloc_returns_pointer_in_young_and_advances_used() {
        let mut heap = Heap::new(8);

        let ptr = heap.alloc(3).expect("allocation should fit");

        assert!(!ptr.is_null());
        assert!(heap.young_contains(ptr));
        assert!(!heap.old_contains(ptr));
        assert_eq!(heap.used(), 3);
        assert_eq!(heap.high_water_mark(), 3);
    }

    #[test]
    fn allocation_regions_do_not_overlap() {
        let mut heap = Heap::new(8);

        let first = heap.alloc(3).expect("first allocation should fit");
        let second = heap.alloc(2).expect("second allocation should fit");

        assert_eq!(second.addr() - first.addr(), 3 * std::mem::size_of::<u64>());
    }

    #[test]
    fn heap_full_preserves_usage() {
        let mut heap = Heap::new(4);
        let _ = heap.alloc(3).expect("initial allocation should fit");

        let error = heap
            .alloc(2)
            .expect_err("allocation should exceed capacity");

        assert_eq!(
            error,
            HeapFull {
                requested: 2,
                available: 1
            }
        );
        assert_eq!(heap.used(), 3);
        assert_eq!(heap.high_water_mark(), 3);
    }

    #[test]
    fn zero_word_allocation_does_not_advance_bump_pointer() {
        let mut heap = Heap::new(1);

        let first = heap.alloc(0).expect("zero word allocation should succeed");
        let second = heap.alloc(0).expect("zero word allocation should succeed");

        assert_eq!(first, second);
        assert_eq!(heap.used(), 0);
    }

    #[test]
    fn young_and_old_are_distinct_regions() {
        let mut heap = Heap::new(DEFAULT_HEAP_SIZE);
        let young = heap.alloc(1).expect("young allocation fits");
        let old = heap.alloc_old(1).expect("old allocation fits");

        assert!(heap.young_contains(young));
        assert!(heap.old_contains(old));
        assert!(!heap.old_contains(young));
        assert!(!heap.young_contains(old));
    }

    #[test]
    fn grows_follow_fibonacci_like_sequence() {
        let mut heap = Heap::new(DEFAULT_HEAP_SIZE);

        heap.grow_to_next_capacity();
        assert_eq!(heap.capacity(), 377);
        heap.grow_to_next_capacity();
        assert_eq!(heap.capacity(), 610);
        heap.grow_to_next_capacity();
        assert_eq!(heap.capacity(), 987);
    }

    #[test]
    fn shrink_never_goes_below_initial_size() {
        let mut heap = Heap::new(DEFAULT_HEAP_SIZE);
        heap.grow_empty_old_to_for_test(987);
        let target = heap.compacted_old_capacity_after_major(0, 0.25);

        assert_eq!(target, DEFAULT_HEAP_SIZE);
        assert_eq!(heap.old_capacity(), 987);
    }
}
