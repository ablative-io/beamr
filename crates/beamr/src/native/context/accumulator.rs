//! The rooted term accumulator — AR-1's remedy at the accumulation window.
//!
//! # What this fixes
//!
//! Every AR-1 crossing has the same shape: a heap [`Term`] is held in an
//! ordinary Rust local — a `Vec<Term>`, a `Vec<(Term, Term)>`, or a threaded
//! `tail` — **across a call that can collect**. The collection moves the
//! objects those terms point at, and the local is left holding stale pointers.
//!
//! The allocation sinks are not the defect. `alloc_tuple`, `alloc_cons`,
//! `alloc_list_with_tail` and `alloc_map` each root their own **arguments**
//! (`with_rooted(&rooted, ..)` before `ensure_heap_space`), so a term is safe
//! for exactly as long as it is an argument. The unsafe region is everything
//! before that: the loop that builds the collection up.
//!
//! [`TermAccumulator`] covers precisely that region. Its elements live in the
//! process native root stack, so a collection triggered anywhere inside the
//! accumulation traces and forwards them, and every read returns the current
//! (post-GC) value.
//!
//! # The invariant a caller still owes
//!
//! ⛔ **A term returned by an allocation must be pushed before the next
//! allocation.** The accumulator roots what it holds; it cannot root what is
//! still in flight. This is the one rule, and it is the rule the terminal
//! sinks already satisfy for their arguments.
//!
//! # Detached contexts
//!
//! A [`ProcessContext`] with no attached process pushes a fresh owned block
//! per allocation into `detached_allocations`. Those blocks are never moved,
//! freed or collected, and `ensure_heap_space` is a no-op — so there is
//! nothing to root and an ordinary `Vec` is sound. The accumulator carries
//! both backings for the same reason `alloc_tuple` and `alloc_cons` carry
//! both: ⚠️ `with_rooted` **returns `badarg` when no process is attached**, so
//! a remedy that reached for it unconditionally would convert working
//! detached calls into refusals. That is a behaviour change, not a fix, and
//! this module refuses to make it.
//!
//! # Scope of the claim
//!
//! This type is the **vehicle** for the remedy, not the wall. It makes the
//! rooted form available and idiomatic; it does not yet make the unrooted
//! form unwritable. The gate's criterion — *the shape CANNOT BE WRITTEN* —
//! is met only once the sinks stop accepting a bare `&[Term]`. Until that
//! lands, a site migrated to this type is `SAFE-ROOTED`, never
//! `STRUCTURALLY-ELIMINATED`.

use crate::term::Term;

use super::{ProcessContext, RootedTerms};

/// A term accumulator whose elements stay reachable across allocations.
///
/// Obtained only from [`ProcessContext::with_accumulator`], so the roots it
/// registers cannot outlive the scope that owns them.
pub struct TermAccumulator {
    backing: Backing,
}

enum Backing {
    /// Attached: elements live in the process native root stack, traced and
    /// forwarded by every collection.
    Rooted(RootedTerms),
    /// Detached: owned per-allocation blocks never move, so a plain `Vec`
    /// holds valid terms for as long as the context lives.
    Owned(Vec<Term>),
}

impl ProcessContext<'_> {
    /// Run `body` with an empty [`TermAccumulator`].
    ///
    /// On an attached context the accumulator opens a rooted scope; the roots
    /// are released when `body` returns, on both the success and error paths.
    /// On a detached context it accumulates into owned storage, matching what
    /// the allocators already do there.
    pub fn with_accumulator<R>(
        &mut self,
        body: impl FnOnce(&mut Self, &mut TermAccumulator) -> Result<R, Term>,
    ) -> Result<R, Term> {
        if self.process.is_none() {
            let mut accumulator = TermAccumulator {
                backing: Backing::Owned(Vec::new()),
            };
            return body(self, &mut accumulator);
        }
        self.with_rooted(&[], |context, roots| {
            let mut accumulator = TermAccumulator {
                backing: Backing::Rooted(*roots),
            };
            body(context, &mut accumulator)
        })
    }
}

impl TermAccumulator {
    /// Number of terms held.
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.backing {
            Backing::Rooted(handle) => handle.len,
            Backing::Owned(terms) => terms.len(),
        }
    }

    /// Whether the accumulator holds no terms.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Append `term`, rooting it for the rest of the scope.
    ///
    /// Pushing registers a root; it never allocates on the term heap and so
    /// cannot itself trigger a collection.
    pub fn push(&mut self, context: &mut ProcessContext<'_>, term: Term) -> Result<(), Term> {
        match &mut self.backing {
            Backing::Rooted(handle) => context.rooted_push(handle, term),
            Backing::Owned(terms) => {
                terms.push(term);
                Ok(())
            }
        }
    }

    /// Read the current (post-GC) value at `index`.
    ///
    /// Returns `badarg` for an out-of-range index — a caller bug, never user
    /// error.
    pub fn get(&self, context: &ProcessContext<'_>, index: usize) -> Result<Term, Term> {
        match &self.backing {
            Backing::Rooted(handle) => context.rooted(handle, index),
            Backing::Owned(terms) => terms
                .get(index)
                .copied()
                .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG)),
        }
    }

    /// Overwrite the slot at `index`, keeping it rooted.
    pub fn set(
        &mut self,
        context: &mut ProcessContext<'_>,
        index: usize,
        term: Term,
    ) -> Result<(), Term> {
        match &mut self.backing {
            Backing::Rooted(handle) => context.set_rooted(handle, index, term),
            Backing::Owned(terms) => {
                let slot = terms
                    .get_mut(index)
                    .ok_or_else(|| Term::atom(crate::atom::Atom::BADARG))?;
                *slot = term;
                Ok(())
            }
        }
    }

    /// Copy every element out, newest values first read.
    ///
    /// ⚠️ The returned terms are valid **only until the next allocation**.
    /// This is private on purpose: its one legitimate use is to hand the whole
    /// run straight to a sink that roots its own arguments, which the `to_*`
    /// terminals below do, with no allocation in between.
    fn snapshot(&self, context: &ProcessContext<'_>) -> Result<Vec<Term>, Term> {
        match &self.backing {
            Backing::Rooted(handle) => {
                let mut terms = Vec::with_capacity(handle.len);
                for index in 0..handle.len {
                    terms.push(context.rooted(handle, index)?);
                }
                Ok(terms)
            }
            Backing::Owned(terms) => Ok(terms.clone()),
        }
    }

    /// Build a proper list of the accumulated terms, in push order.
    pub fn to_list(&self, context: &mut ProcessContext<'_>) -> Result<Term, Term> {
        let elements = self.snapshot(context)?;
        context.alloc_list(&elements)
    }

    /// Build a list of the accumulated terms ending in `tail`.
    pub fn to_list_with_tail(
        &self,
        context: &mut ProcessContext<'_>,
        tail: Term,
    ) -> Result<Term, Term> {
        let elements = self.snapshot(context)?;
        context.alloc_list_with_tail(&elements, tail)
    }

    /// Build a tuple of the accumulated terms, in push order.
    pub fn to_tuple(&self, context: &mut ProcessContext<'_>) -> Result<Term, Term> {
        let elements = self.snapshot(context)?;
        context.alloc_tuple(&elements)
    }

    /// Build a flatmap, reading the run as alternating key, value, key, value.
    ///
    /// This is the S3e shape — `Vec<(Term, Term)>` — with both halves of every
    /// pair rooted instead of boxed inside a tuple no collection can see.
    ///
    /// Refuses with `badarg` on an odd length rather than dropping the unpaired
    /// tail: a half-written pair is a caller bug, and a silent truncation here
    /// would produce a map that is short by one key with no diagnostic.
    pub fn to_map_pairs(&self, context: &mut ProcessContext<'_>) -> Result<Term, Term> {
        let flat = self.snapshot(context)?;
        if flat.len() % 2 != 0 {
            return Err(Term::atom(crate::atom::Atom::BADARG));
        }
        let mut keys = Vec::with_capacity(flat.len() / 2);
        let mut values = Vec::with_capacity(flat.len() / 2);
        for pair in flat.chunks_exact(2) {
            keys.push(pair[0]);
            values.push(pair[1]);
        }
        context.alloc_map(&keys, &values)
    }

    /// Sort an alternating key/value run by key, in place and still rooted.
    ///
    /// Reading out, sorting and writing back is sound because none of those
    /// steps allocates: no collection can run between the read and the
    /// write-back, so the values cannot move underneath the sort.
    ///
    /// Refuses with `badarg` on an odd length, for the reason
    /// [`Self::to_map_pairs`] gives.
    pub fn sort_pairs_by_key(&mut self, context: &mut ProcessContext<'_>) -> Result<(), Term> {
        let flat = self.snapshot(context)?;
        if flat.len() % 2 != 0 {
            return Err(Term::atom(crate::atom::Atom::BADARG));
        }
        let mut pairs: Vec<(Term, Term)> = flat
            .chunks_exact(2)
            .map(|pair| (pair[0], pair[1]))
            .collect();
        pairs.sort_by_key(|(key, _)| *key);
        for (index, (key, value)) in pairs.into_iter().enumerate() {
            self.set(context, index * 2, key)?;
            self.set(context, index * 2 + 1, value)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Process;
    use crate::term::binary::Binary;
    use crate::term::boxed::Cons;

    const WIDTH: usize = 12;

    fn payload(index: usize) -> String {
        format!("a{index:0WIDTH$}")
    }

    /// Walk a proper list of binaries and check each against what was put.
    ///
    /// ⛔ ITERATIVE AND HARD-CAPPED. A stale carrier can make a cons tail alias
    /// an enclosing cell, turning the list into a CYCLE; a recursive or uncapped
    /// walk hangs instead of reporting, and a hang is the one failure mode this
    /// lane has already paid for once.
    fn check_list(list: Term, count: usize) -> Result<(), String> {
        let cap = count * 2 + 16;
        let mut seen = 0usize;
        let mut tail = list;
        while !tail.is_nil() {
            if seen > cap {
                return Err(format!(
                    "list did not terminate within {cap} cells — cyclic tail"
                ));
            }
            let cons = Cons::new(tail)
                .ok_or_else(|| format!("entry {seen}: tail is not a cons — carrier went stale"))?;
            let binary = Binary::new(cons.head()).ok_or_else(|| {
                format!("entry {seen}: head is not a binary — carrier went stale")
            })?;
            let want = payload(seen);
            if binary.as_bytes() != want.as_bytes() {
                return Err(format!(
                    "entry {seen}: contents {:?} != {want:?}",
                    String::from_utf8_lossy(binary.as_bytes())
                ));
            }
            seen += 1;
            tail = cons.tail();
        }
        if seen != count {
            return Err(format!("recovered {seen} entries, put {count}"));
        }
        Ok(())
    }

    /// What one cell of the sweep did.
    ///
    /// ⛔ `Refused` and `Corrupt` are DIFFERENT OUTCOMES and the difference is
    /// the whole instrument. A refusal is the allocator declining a request the
    /// heap cannot serve — correct behaviour under pressure, and evidence of
    /// nothing at all about rooting. Corruption is a carrier that went stale.
    /// The first version of this sweep counted any `Err` as red and reported a
    /// live positive control on six cells of which **zero** were corruption.
    /// The type keeps them apart so no future counter can merge them again.
    #[derive(Debug, PartialEq, Eq)]
    enum Cell {
        Clean,
        Refused,
        Corrupt(String),
    }

    /// Pre-fill the nursery with UNROOTED filler down to `margin` words free,
    /// so a collection during the accumulation is forced rather than hoped for.
    ///
    /// ⛔ THE LOOP MUST BE ABLE TO GIVE UP, AND THE CELL MUST SAY SO. The
    /// descent step is one filler allocation, so a margin finer than that is
    /// only reachable via a collection, which frees this unrooted filler and
    /// pushes `available` back up. The achieved margin is RETURNED so a
    /// give-up carries its own witness out with it.
    fn pre_fill(context: &mut ProcessContext<'_>, margin: usize) -> usize {
        let mut filler = Vec::new();
        let mut last_available = usize::MAX;
        loop {
            let available = context
                .process_heap()
                .map(|heap| heap.available())
                .unwrap_or(0);
            if available <= margin || available >= last_available {
                break available;
            }
            last_available = available;
            match context.alloc_binary(&[0xA1; 32]) {
                Ok(term) => filler.push(term),
                Err(_) => break available,
            }
        }
    }

    /// ⛔⛔ THE SYNTHETIC POSITIVE — the pre-fix shape, written ON PURPOSE.
    ///
    /// This is the AR-1 defect: a bare `Vec<Term>` accumulating heap terms
    /// across allocations that can collect. It is here because **inverting a
    /// probe kills the control that made its green mean anything**. Once the
    /// production sites are rooted, "no corruption" is the expected result at
    /// every one of them — and a sweep that has stopped applying pressure
    /// produces exactly the same output. This arm is the difference between
    /// those two states.
    ///
    /// ⇒ If this ever stops going red, the pressure regime is gone and every
    /// green in this module is uninterpretable. It is asserted, not hoped.
    fn unrooted_arm(count: usize, heap: usize, margin: usize) -> (usize, Cell) {
        let mut process = Process::new(11, heap);
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);
        let achieved = pre_fill(&mut context, margin);

        let mut terms: Vec<Term> = Vec::with_capacity(count);
        for index in 0..count {
            match context.alloc_binary(payload(index).as_bytes()) {
                Ok(term) => terms.push(term),
                Err(_) => return (achieved, Cell::Refused),
            }
        }
        let Ok(list) = context.alloc_list(&terms) else {
            return (achieved, Cell::Refused);
        };
        (achieved, cell_of(check_list(list, count)))
    }

    /// The same accumulation through [`TermAccumulator`] — identical inputs,
    /// identical heap, identical pressure. The ONLY difference is where the
    /// carrier lives.
    fn rooted_arm(count: usize, heap: usize, margin: usize) -> (usize, Cell) {
        let mut process = Process::new(11, heap);
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);
        let achieved = pre_fill(&mut context, margin);

        let outcome = context.with_accumulator(|context, accumulator| {
            for index in 0..count {
                let term = context.alloc_binary(payload(index).as_bytes())?;
                accumulator.push(context, term)?;
            }
            accumulator.to_list(context)
        });
        let Ok(list) = outcome else {
            return (achieved, Cell::Refused);
        };
        (achieved, cell_of(check_list(list, count)))
    }

    fn cell_of(result: Result<(), String>) -> Cell {
        match result {
            Ok(()) => Cell::Clean,
            Err(reason) => Cell::Corrupt(reason),
        }
    }

    /// ⭐ TWO ARMS, ONE SWEEP. The rooted arm must be clean or refused at every
    /// cell AND the unrooted arm must be CORRUPT at some cell. Either half
    /// alone is worthless: a clean rooted arm with no corrupt control proves
    /// only that nothing was tested.
    #[test]
    fn accumulator_survives_collections_that_break_a_bare_vec() {
        const HEAP: usize = 4096;

        let mut corrupt_control = 0usize;
        let mut clean_control = 0usize;
        let mut rooted_failures = Vec::new();
        for count in [20usize, 60, 120] {
            for margin in [0usize, 1, 2, 4, 8, 16, 32, 64, 128] {
                let (bare_at, bare) = unrooted_arm(count, HEAP, margin);
                let (acc_at, accumulated) = rooted_arm(count, HEAP, margin);
                eprintln!(
                    "accumulator len {count:>4} margin req {margin:>4}: \
                     bare Vec @{bare_at:>5} {bare:?} | accumulator @{acc_at:>5} {accumulated:?}"
                );
                match bare {
                    Cell::Corrupt(_) => corrupt_control += 1,
                    Cell::Clean => clean_control += 1,
                    Cell::Refused => {}
                }
                if let Cell::Corrupt(reason) = accumulated {
                    rooted_failures.push(format!("len {count} margin {margin}: {reason}"));
                }
            }
        }

        assert!(
            corrupt_control > 0,
            "POSITIVE CONTROL DEAD: the bare-Vec arm was never CORRUPTED (clean {clean_control}), \
             so this sweep applies no usable heap pressure and the rooted arm's green means \
             nothing. Repair the pressure regime — do NOT weaken this assertion, and do NOT \
             count refusals as reds."
        );
        assert!(
            clean_control > 0,
            "NEGATIVE CONTROL DEAD: no bare-Vec cell was clean, so the reader may be broken \
             rather than the carrier stale."
        );
        assert!(
            rooted_failures.is_empty(),
            "TermAccumulator failed to keep its elements alive: {rooted_failures:#?}"
        );

        // ⭐ THE CONTROL'S OWN SURFACE IS PINNED, not just asserted non-zero —
        // the row-4 precedent. A bare `> 0` passes while the regime quietly
        // decays from seventeen corrupting cells to one, and the day it reaches
        // zero is the day the assertion above starts lying about why.
        //
        // 17 corrupt + 10 clean + 0 refused = 27 cells. The ten clean cells are
        // the nine margin-0/1/2 rows, where the pre-fill gives up immediately
        // (its descent step is coarser than the requested margin) and so applies
        // no pressure at all — visible in the achieved column, which reads 4090
        // against a 4096-word heap — plus len 20 / margin 128, where the
        // accumulation is short enough to fit inside the margin left free.
        //
        // ⛔ THESE TWO NUMBERS COME FROM THE COUNTERS ABOVE, NOT FROM READING
        // THE PRINTED TABLE. The first version of this assertion pinned (20, 7),
        // hand-counted off that table while the program was already computing
        // the pair three lines up — so it tested my arithmetic rather than the
        // instrument, and failed on the next run for a reason that had nothing
        // to do with the code. Measured stable at (17, 10) over five runs.
        assert_eq!(
            (corrupt_control, clean_control),
            (17, 10),
            "the bare-Vec control's surface drifted; re-derive the pressure regime before \
             trusting any green in this module"
        );
    }

    /// A detached context has no process, so `with_rooted` would refuse. The
    /// accumulator must still work — see the module docs: refusing here would
    /// be a behaviour change wearing a fix's clothes.
    #[test]
    fn accumulator_works_on_a_detached_context() {
        let mut context = ProcessContext::new();
        let list = context
            .with_accumulator(|context, accumulator| {
                for index in 0..8 {
                    let term = context.alloc_binary(payload(index).as_bytes())?;
                    accumulator.push(context, term)?;
                }
                assert_eq!(accumulator.len(), 8);
                accumulator.to_list(context)
            })
            .expect("detached accumulation should succeed");
        check_list(list, 8).expect("detached list should round-trip");
    }

    /// An odd-length pair run is a caller bug and is REFUSED, never silently
    /// truncated to the even prefix — gate row 6, no silent arm.
    #[test]
    fn odd_pair_runs_are_refused_not_truncated() {
        let mut process = Process::new(11, 4096);
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);

        let map = context.with_accumulator(|context, accumulator| {
            let key = context.alloc_binary(b"k")?;
            accumulator.push(context, key)?;
            accumulator.to_map_pairs(context)
        });
        assert_eq!(map, Err(Term::atom(crate::atom::Atom::BADARG)));

        let sorted = context.with_accumulator(|context, accumulator| {
            let key = context.alloc_binary(b"k")?;
            accumulator.push(context, key)?;
            accumulator.sort_pairs_by_key(context)
        });
        assert_eq!(sorted, Err(Term::atom(crate::atom::Atom::BADARG)));
    }
}
