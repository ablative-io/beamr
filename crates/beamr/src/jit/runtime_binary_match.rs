//! Binary matching runtime helpers callable from JIT-generated code.
use super::runtime::{alloc_words, alloc_words_rooted, process_from_abi};
use crate::process::Process;
use crate::term::Term;
use crate::term::heap_borrow::HeapBorrow;
use crate::term::shared_binary::{alloc_binary, alloc_binary_word_count};
use crate::term::sub_binary::{SUB_BINARY_WORDS, write_sub_binary};
use crate::term::{
    binary_ref::BinaryRef,
    boxed::{BoxedHeader, BoxedTag, ProcBin},
};

const MATCH_CONTEXT_WORDS: usize = 4;
pub(super) const BINARY_HELPER_FAILURE: u64 = u64::MAX;

pub(crate) extern "C" fn jit_bs_start_match(process: *mut Process, binary: u64) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 0;
    };
    let source = Term::from_raw(binary);
    let Some(binary) = BinaryRef::new(source) else {
        return BINARY_HELPER_FAILURE;
    };
    let Some(total_bits) = binary.len().checked_mul(u8::BITS as usize) else {
        return BINARY_HELPER_FAILURE;
    };
    // Root the source across the allocation: it can collect, and the local
    // `source` above is a bare copy that no collection can reach. Writing the
    // pre-move value into the context at the end of this function is H3.
    let mut roots = [source];
    let ptr = alloc_words_rooted(process, MATCH_CONTEXT_WORDS, &mut roots);
    if ptr.is_null() {
        return 0;
    }
    let [source] = roots;
    let heap = unsafe { std::slice::from_raw_parts_mut(ptr, MATCH_CONTEXT_WORDS) };
    heap[0] = BoxedHeader::new(BoxedTag::MatchContext, MATCH_CONTEXT_WORDS - 1);
    heap[1] = 0;
    heap[2] = total_bits as u64;
    heap[3] = source.raw();
    Term::boxed_ptr(heap.as_ptr()).raw()
}

pub(crate) extern "C" fn jit_bs_get_integer(match_ctx: u64, size_bits: u64, flags: u64) -> u64 {
    let Some(context) = JitMatchContext::new(Term::from_raw(match_ctx)) else {
        return BINARY_HELPER_FAILURE;
    };
    let Ok(size_bits) = usize::try_from(size_bits) else {
        return BINARY_HELPER_FAILURE;
    };
    if !size_bits.is_multiple_of(u8::BITS as usize)
        || !context.position_bits().is_multiple_of(u8::BITS as usize)
        || !context.has_bits(size_bits)
    {
        return BINARY_HELPER_FAILURE;
    }
    // SAFETY: this ABI entry receives no process pointer at all — the JIT
    // calls it with only the match context — so no witness can be built. Like
    // the `std` trait readers in `docs/design/accessor-lifetimes.md` §4, this
    // is a structurally witness-less reader: `with_frame`'s higher-ranked
    // witness keeps the slice inside the closure, and the closure only decodes
    // an integer — it performs no allocation, runs no collection, and drops no
    // heap, so nothing can invalidate the bytes while they are read.
    let Some(value) = (unsafe {
        HeapBorrow::with_frame(|heap| {
            context
                .slice(size_bits, heap)
                .and_then(|bytes| decode_integer(bytes, SegmentFlags::from_raw(flags)))
        })
    }) else {
        return BINARY_HELPER_FAILURE;
    };
    let Some(term) = Term::try_small_int(value) else {
        return BINARY_HELPER_FAILURE;
    };
    context.set_position_bits(context.position_bits() + size_bits);
    term.raw()
}

pub(crate) extern "C" fn jit_bs_get_binary(
    process: *mut Process,
    match_ctx: u64,
    size_bits: u64,
) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 0;
    };
    let Some(context) = JitMatchContext::new(Term::from_raw(match_ctx)) else {
        return BINARY_HELPER_FAILURE;
    };
    let bits = if size_bits == u64::MAX {
        context.remaining_bits()
    } else {
        let Ok(bits) = usize::try_from(size_bits) else {
            return BINARY_HELPER_FAILURE;
        };
        bits
    };
    if !bits.is_multiple_of(u8::BITS as usize)
        || !context.position_bits().is_multiple_of(u8::BITS as usize)
        || !context.has_bits(bits)
    {
        return BINARY_HELPER_FAILURE;
    }
    let Some(bytes) = context.slice(bits, process.borrow_terms()) else {
        return BINARY_HELPER_FAILURE;
    };
    let source = context.source_term();
    if ProcBin::new(source).is_some() {
        // O(1) extraction. A ProcBin keeps its bytes off-heap and Arc-retained,
        // so the result can SHARE them through a sub-binary rather than copy
        // them. `bytes` is deliberately NOT read on this path — that borrow is
        // the O(m) copy this arm exists to avoid.
        //
        // 0.16.3 deleted this arm rather than repairing it, because the repair
        // needs real rooting: the ProcBin BOX lives on the young heap and moves
        // under the allocation below, and writing a pre-move box address into
        // the sub-binary is the same stale-Term defect as H3. `source` is
        // therefore rooted across the allocation and the FORWARDED value is
        // what gets written.
        let start = context.position_bits() / u8::BITS as usize;
        let length = bits / u8::BITS as usize;
        // Advance before allocating, for the same reason the copy path does —
        // see the note below, which covers this arm's refusal too.
        context.set_position_bits(context.position_bits() + bits);
        let mut roots = [source];
        let ptr = alloc_words_rooted(process, SUB_BINARY_WORDS, &mut roots);
        if ptr.is_null() {
            return 0;
        }
        let [source] = roots;
        // SAFETY: `alloc_words_rooted` returned a non-null pointer to exactly
        // `SUB_BINARY_WORDS` heap words owned by `process` for this call.
        let heap = unsafe { std::slice::from_raw_parts_mut(ptr, SUB_BINARY_WORDS) };
        return write_sub_binary(heap, source, start, length).map_or(0, Term::raw);
    }

    // Own the bytes: the allocation below can collect, moving (and
    // zero-filling) a young-heap source under this borrow. The copy is taken
    // before anything can move.
    let bytes = bytes.to_vec();
    // Advance the position BEFORE the allocation: the allocation can collect
    // and move this match context, and a post-collection write through the
    // pre-collection pointer is a wild read-modify-write of whatever now
    // occupies that address (observed corrupting the freshly allocated
    // result). Every refusal in this function returns 0 and the caller abandons
    // the match rather than resuming it, so the early advance is unobservable.
    // That rests on the CALLER'S response, not on the cause: it covers a failed
    // allocation here, and equally `alloc_words_rooted` declining to return an
    // unrecoverable root on the sharing arm above.
    context.set_position_bits(context.position_bits() + bits);
    let Some(binary) = allocate_binary(process, &bytes) else {
        return 0;
    };
    binary.raw()
}

pub(crate) extern "C" fn jit_bs_test_tail(match_ctx: u64, expected_bits: u64) -> u8 {
    let Some(context) = JitMatchContext::new(Term::from_raw(match_ctx)) else {
        return 0;
    };
    let Ok(expected_bits) = usize::try_from(expected_bits) else {
        return 0;
    };
    u8::from(context.remaining_bits() == expected_bits)
}

pub(crate) extern "C" fn jit_bs_test_unit(match_ctx: u64, unit: u64) -> u8 {
    let Some(context) = JitMatchContext::new(Term::from_raw(match_ctx)) else {
        return 0;
    };
    let Ok(unit) = usize::try_from(unit) else {
        return 0;
    };
    u8::from(unit != 0 && context.remaining_bits().is_multiple_of(unit))
}

pub(crate) extern "C" fn jit_bs_get_utf8(match_ctx: u64, flags: u64) -> u64 {
    get_utf(match_ctx, flags, decode_utf8)
}

pub(crate) extern "C" fn jit_bs_get_utf16(match_ctx: u64, flags: u64) -> u64 {
    get_utf(match_ctx, flags, decode_utf16)
}

pub(crate) extern "C" fn jit_bs_get_utf32(match_ctx: u64, flags: u64) -> u64 {
    get_utf(match_ctx, flags, decode_utf32)
}

#[derive(Copy, Clone)]
struct JitMatchContext {
    ptr: *mut u64,
}

impl JitMatchContext {
    fn new(term: Term) -> Option<Self> {
        let ptr = term.heap_ptr()? as *mut u64;
        (boxed_tag(ptr) == Some(BoxedTag::MatchContext)).then_some(Self { ptr })
    }
    fn position_bits(self) -> usize {
        read_word(self.ptr, 1) as usize
    }
    fn set_position_bits(self, bits: usize) {
        write_word(self.ptr, 1, bits as u64);
    }
    fn total_bits(self) -> usize {
        read_word(self.ptr, 2) as usize
    }
    fn source_term(self) -> Term {
        Term::from_raw(read_word(self.ptr, 3))
    }
    fn source(self) -> Option<BinaryRef> {
        BinaryRef::new(self.source_term())
    }
    fn remaining_bits(self) -> usize {
        self.total_bits().saturating_sub(self.position_bits())
    }
    fn has_bits(self, bits: usize) -> bool {
        self.position_bits()
            .checked_add(bits)
            .is_some_and(|end| end <= self.total_bits())
    }
    fn slice<'heap>(self, bits: usize, heap: HeapBorrow<'heap>) -> Option<&'heap [u8]> {
        if !bits.is_multiple_of(u8::BITS as usize)
            || !self.position_bits().is_multiple_of(u8::BITS as usize)
        {
            return None;
        }
        let start = self.position_bits() / u8::BITS as usize;
        let len = bits / u8::BITS as usize;
        let end = start.checked_add(len)?;
        self.source()?.as_bytes(heap).get(start..end)
    }
}

#[derive(Copy, Clone)]
pub(super) enum Endian {
    Big,
    Little,
}

impl Endian {
    pub(super) fn from_raw(flags: u64) -> Self {
        if flags & 0x02 != 0 || flags & 0x01 != 0 {
            Self::Little
        } else {
            Self::Big
        }
    }
}

#[derive(Copy, Clone)]
struct SegmentFlags {
    endian: Endian,
    signed: bool,
}

impl SegmentFlags {
    fn from_raw(flags: u64) -> Self {
        Self {
            endian: Endian::from_raw(flags),
            signed: flags & 0x04 != 0,
        }
    }
}

pub(super) fn boxed_tag(ptr: *const u64) -> Option<BoxedTag> {
    BoxedHeader::tag(read_word(ptr.cast_mut(), 0))
}

pub(super) fn read_word(ptr: *mut u64, offset: usize) -> u64 {
    unsafe { *ptr.add(offset) }
}

pub(super) fn write_word(ptr: *mut u64, offset: usize, value: u64) {
    unsafe { *ptr.add(offset) = value }
}

fn decode_integer(bytes: &[u8], flags: SegmentFlags) -> Option<i64> {
    if bytes.len() > std::mem::size_of::<i64>() {
        return None;
    }
    let msb = match flags.endian {
        Endian::Big => bytes.first(),
        Endian::Little => bytes.last(),
    };
    let negative = flags.signed && msb.is_some_and(|byte| byte & 0x80 != 0);
    let fill = if negative { 0xff_u8 } else { 0x00_u8 };
    let mut full = [fill; 8];
    match flags.endian {
        Endian::Big => full[8 - bytes.len()..].copy_from_slice(bytes),
        Endian::Little => full[..bytes.len()].copy_from_slice(bytes),
    }
    Some(match flags.endian {
        Endian::Big => u64::from_be_bytes(full) as i64,
        Endian::Little => u64::from_le_bytes(full) as i64,
    })
}

pub(super) fn allocate_binary(process: &mut Process, bytes: &[u8]) -> Option<Term> {
    let words = alloc_binary_word_count(bytes.len());
    let ptr = alloc_words(process, words);
    if ptr.is_null() {
        return None;
    }
    // A large extraction lands as a refcounted ProcBin; mark the allocation so
    // the GC release walk drops its Arc. See `process::heap::AllocKind`.
    process
        .heap_mut()
        .mark_last_young_allocation_maybe_refcounted();
    let heap = unsafe { std::slice::from_raw_parts_mut(ptr, words) };
    alloc_binary(heap, bytes)
}

fn get_utf(
    match_ctx: u64,
    flags: u64,
    decoder: fn(JitMatchContext, Endian, HeapBorrow<'_>) -> Option<(u32, usize)>,
) -> u64 {
    let Some(context) = JitMatchContext::new(Term::from_raw(match_ctx)) else {
        return BINARY_HELPER_FAILURE;
    };
    // SAFETY: `jit_bs_get_utf{8,16,32}` are ABI entries that receive no process
    // pointer, so no witness can be built — a structurally witness-less reader
    // in the sense of `docs/design/accessor-lifetimes.md` §4. `with_frame`'s
    // higher-ranked witness keeps every slice the decoder reads inside the
    // closure, and the decoders only read bytes and return a codepoint: no
    // allocation, no collection, no heap drop happens while they run.
    let Some((codepoint, bits)) =
        (unsafe { HeapBorrow::with_frame(|heap| decoder(context, Endian::from_raw(flags), heap)) })
    else {
        return BINARY_HELPER_FAILURE;
    };
    let Some(term) = Term::try_small_int(i64::from(codepoint)) else {
        return BINARY_HELPER_FAILURE;
    };
    context.set_position_bits(context.position_bits() + bits);
    term.raw()
}

fn decode_utf8(
    context: JitMatchContext,
    _endian: Endian,
    heap: HeapBorrow<'_>,
) -> Option<(u32, usize)> {
    if !context.position_bits().is_multiple_of(u8::BITS as usize) {
        return None;
    }
    let bytes = context.slice(context.remaining_bits(), heap)?;
    let first = bytes.first().copied()?;
    let (needed, mut codepoint, min) = if first <= 0x7f {
        (1, u32::from(first), 0)
    } else if (0xc2..=0xdf).contains(&first) {
        (2, u32::from(first & 0x1f), 0x80)
    } else if (0xe0..=0xef).contains(&first) {
        (3, u32::from(first & 0x0f), 0x800)
    } else if (0xf0..=0xf4).contains(&first) {
        (4, u32::from(first & 0x07), 0x10000)
    } else {
        return None;
    };
    if bytes.len() < needed {
        return None;
    }
    for byte in &bytes[1..needed] {
        if byte & 0xc0 != 0x80 {
            return None;
        }
        codepoint = (codepoint << 6) | u32::from(byte & 0x3f);
    }
    (codepoint >= min && valid_codepoint(codepoint))
        .then_some((codepoint, needed * u8::BITS as usize))
}

fn decode_utf16(
    context: JitMatchContext,
    endian: Endian,
    heap: HeapBorrow<'_>,
) -> Option<(u32, usize)> {
    let first = read_u16(context, 0, endian, heap)?;
    if (0xd800..=0xdbff).contains(&first) {
        let second = read_u16(context, 2, endian, heap)?;
        if !(0xdc00..=0xdfff).contains(&second) {
            return None;
        }
        let codepoint =
            0x10000 + (((u32::from(first) - 0xd800) << 10) | (u32::from(second) - 0xdc00));
        valid_codepoint(codepoint).then_some((codepoint, 32))
    } else if (0xdc00..=0xdfff).contains(&first) {
        None
    } else {
        Some((u32::from(first), 16))
    }
}

fn decode_utf32(
    context: JitMatchContext,
    endian: Endian,
    heap: HeapBorrow<'_>,
) -> Option<(u32, usize)> {
    if !context.position_bits().is_multiple_of(u8::BITS as usize) || !context.has_bits(32) {
        return None;
    }
    let bytes = context.slice(32, heap)?;
    let codepoint = match endian {
        Endian::Big => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        Endian::Little => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    };
    valid_codepoint(codepoint).then_some((codepoint, 32))
}

fn read_u16(
    context: JitMatchContext,
    byte_offset: usize,
    endian: Endian,
    heap: HeapBorrow<'_>,
) -> Option<u16> {
    if !context.position_bits().is_multiple_of(u8::BITS as usize) {
        return None;
    }
    let bits = (byte_offset + 2) * u8::BITS as usize;
    if !context.has_bits(bits) {
        return None;
    }
    let bytes = context.slice(bits, heap)?;
    let pair = [bytes[byte_offset], bytes[byte_offset + 1]];
    Some(match endian {
        Endian::Big => u16::from_be_bytes(pair),
        Endian::Little => u16::from_le_bytes(pair),
    })
}

pub(super) fn valid_codepoint(codepoint: u32) -> bool {
    codepoint <= 0x10ffff && !(0xd800..=0xdfff).contains(&codepoint)
}

pub(super) fn set_badarg(process: &mut Process) {
    process.set_current_exception(Some(crate::process::Exception {
        class: Term::atom(crate::atom::Atom::ERROR),
        reason: Term::atom(crate::atom::Atom::BADARG),
        stacktrace: Term::NIL,
    }));
}

#[cfg(test)]
mod gc_release_tests {
    use super::*;
    use crate::term::boxed::ProcBin;

    #[test]
    fn large_extracted_binary_is_released_by_minor_gc() {
        let mut process = Process::new(1, 32);
        let bytes = vec![0x6B; 4096];

        let term = allocate_binary(&mut process, &bytes).expect("binary allocates");
        let observer = ProcBin::new(term)
            .expect("a large extracted binary lands as a refc binary")
            .shared_binary();
        assert_eq!(observer.ref_count(), 2);

        crate::gc::collect_minor(&mut process).expect("minor GC succeeds");

        assert_eq!(
            observer.ref_count(),
            1,
            "GC must release the extracted binary's shared-bytes Arc"
        );
    }
}

// --- as_bytes borrow-across-alloc walls (0.16.3 fix lane 3, site 12 + sibling) ---
// AUDIT.md AMENDMENT 1: `jit_bs_get_binary` borrows the source bytes via
// `context.slice`, then allocates (collecting) before the copy — inline
// sources are moved and zero-filled under the live borrow. The ProcBin arm
// additionally captures the source Term before its allocation and writes the
// stale capture into the new sub-binary after. These walls force that
// geometry and assert the extraction's bytes; red at the unfixed base.
#[cfg(test)]
mod gc_hazard_tests {
    use super::*;
    use crate::atom::AtomTable;
    use crate::native::ProcessContext;
    use crate::term::boxed::SubBinary;
    use crate::term::sub_binary::SUB_BINARY_WORDS;
    use std::sync::Arc;

    fn test_context(process: &mut Process, live_x: u16) -> ProcessContext<'_> {
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::new(AtomTable::with_common_atoms())));
        context.attach_process(process, usize::from(live_x));
        context
    }

    /// Fills the nursery with live cons cells until fewer than `needed` words
    /// remain, so the next allocation of that size must collect. Never
    /// collects itself (it stops while `needed` still fits).
    fn fill_until(process: &mut Process, needed: usize) {
        let mut ctx = test_context(process, 2);
        while ctx.process_heap().expect("heap").available() >= needed {
            ctx.alloc_cons(Term::small_int(1), Term::NIL)
                .expect("filler");
        }
    }

    fn extracted_bytes(term: Term, heap: HeapBorrow<'_>) -> Vec<u8> {
        BinaryRef::new(term)
            .expect("extraction result must stay a readable binary")
            .as_bytes(heap)
            .to_vec()
    }

    fn start_match_rooted(process: &mut Process, source: Term) -> Term {
        process.set_x_reg(0, source);
        let match_raw = jit_bs_start_match(process, source.raw());
        assert_ne!(match_raw, 0, "start_match allocation must succeed");
        assert_ne!(match_raw, BINARY_HELPER_FAILURE);
        let match_term = Term::from_raw(match_raw);
        process.set_x_reg(1, match_term);
        match_term
    }

    /// W1 (H3). `jit_bs_start_match` captures the source `Term` at entry
    /// (`:18`) and writes it into the match context (`:33`) AFTER
    /// `alloc_words` (`:25`), which can collect. Forced geometry makes that
    /// allocation collect while the source is live in X0, so the context is
    /// built around a pre-move pointer into the zero-filled young region.
    ///
    /// The register copy IS forwarded; the helper's local copy is not. That
    /// asymmetry is the whole defect.
    #[test]
    fn start_match_source_survives_forced_collection() {
        let mut process = Process::new(1, 256);
        let raw: Vec<u8> = (1..=40).collect();
        let source = {
            let mut ctx = test_context(&mut process, 0);
            ctx.alloc_binary(&raw).expect("inline source")
        };
        process.set_x_reg(0, source);

        fill_until(&mut process, MATCH_CONTEXT_WORDS);
        assert!(
            process.heap().available() < MATCH_CONTEXT_WORDS,
            "geometry must force the match-context allocation to collect"
        );
        assert_eq!(
            process.heap().old_used(),
            0,
            "nothing may be promoted before the subject call"
        );

        let match_raw = jit_bs_start_match(&mut process, source.raw());
        assert_ne!(match_raw, 0, "match-context allocation must succeed");
        assert_ne!(match_raw, BINARY_HELPER_FAILURE);
        assert!(
            process.heap().old_used() > 0,
            "the match-context allocation must have run a collection"
        );
        let moved = process.x_reg(0);
        assert_ne!(
            moved, source,
            "live source should be promoted by the collection"
        );

        // The direct face of H3: the context must store the FORWARDED source,
        // not the pre-move capture. Both raws are reported so the evidence log
        // carries the observed wrong term, not just a refusal code.
        let context = JitMatchContext::new(Term::from_raw(match_raw)).expect("match context");
        let stored = context.source_term();
        assert_eq!(
            stored,
            moved,
            "context stored the pre-move source: stored={:#018x} forwarded={:#018x} original={:#018x}",
            stored.raw(),
            moved.raw(),
            source.raw()
        );

        process.set_x_reg(1, Term::from_raw(match_raw));
        let out_raw = jit_bs_get_binary(&mut process, match_raw, 160);
        assert_ne!(
            out_raw, BINARY_HELPER_FAILURE,
            "the context must still reach its source after the collection"
        );
        assert_ne!(out_raw, 0, "extraction allocation must succeed");
        let expected: Vec<u8> = (1..=20).collect();
        assert_eq!(
            extracted_bytes(Term::from_raw(out_raw), process.borrow_terms()),
            expected
        );
    }

    #[test]
    fn bs_get_binary_inline_source_survives_forced_collection() {
        let mut process = Process::new(1, 256);
        let raw: Vec<u8> = (1..=40).collect();
        let source = {
            let mut ctx = test_context(&mut process, 0);
            ctx.alloc_binary(&raw).expect("inline source")
        };
        let match_term = start_match_rooted(&mut process, source);
        fill_until(&mut process, alloc_binary_word_count(20));
        let out_raw = jit_bs_get_binary(&mut process, match_term.raw(), 160);
        assert_ne!(out_raw, 0, "extraction allocation must succeed");
        assert_ne!(out_raw, BINARY_HELPER_FAILURE);
        assert!(
            process.heap().old_used() > 0,
            "geometry must have collected"
        );
        let expected: Vec<u8> = (1..=20).collect();
        assert_eq!(
            extracted_bytes(Term::from_raw(out_raw), process.borrow_terms()),
            expected
        );
    }

    #[test]
    fn bs_get_binary_procbin_source_box_referent_survives_forced_collection() {
        let mut process = Process::new(1, 256);
        // 100 bytes — above the inline threshold, so the source is a ProcBin:
        // off-heap bytes, but the BOX on the young heap moves under collection.
        let raw: Vec<u8> = (0..100).map(|byte| byte as u8).collect();
        let source = {
            let mut ctx = test_context(&mut process, 0);
            ctx.alloc_binary(&raw).expect("procbin source")
        };
        let match_term = start_match_rooted(&mut process, source);
        // Force collection under EITHER extraction representation: the
        // sub-binary allocation (SUB_BINARY_WORDS) and a copied 20-byte
        // binary (alloc_binary_word_count) — whichever is smaller bounds
        // the fill, so the extraction's allocation must collect regardless.
        fill_until(
            &mut process,
            SUB_BINARY_WORDS.min(alloc_binary_word_count(20)),
        );
        let out_raw = jit_bs_get_binary(&mut process, match_term.raw(), 160);
        assert_ne!(out_raw, 0, "extraction allocation must succeed");
        assert_ne!(out_raw, BINARY_HELPER_FAILURE);
        assert!(
            process.heap().old_used() > 0,
            "geometry must have collected"
        );
        // The box referent: the extraction must still reach live parent bytes
        // after the collection moved the ProcBin box — a stale pre-alloc Term
        // capture leaves the result referencing the zeroed old young region.
        assert_eq!(
            extracted_bytes(Term::from_raw(out_raw), process.borrow_terms()),
            raw[..20].to_vec()
        );
    }

    /// W4. Guards the O(1) sharing arm of `jit_bs_get_binary` — the arm
    /// `0.16.3` deleted rather than repaired, whose *deletion* is the
    /// O(1)→O(m) extraction regression.
    ///
    /// Two distinct claims, and the wall asserts both because the arm can fail
    /// either way:
    ///
    /// 1. **Sharing, not copying.** A ProcBin keeps its bytes off-heap and
    ///    Arc-retained, so an extraction can be a `SubBinary` view. Reverting
    ///    to the `to_vec` path still returns byte-correct data — it is
    ///    invisible to every other wall here — so the result's *shape* has to
    ///    be asserted or the regression returns silently.
    /// 2. **The forwarded parent is stored.** The ProcBin box itself lives on
    ///    the young heap and moves under the sub-binary allocation. Writing the
    ///    pre-move box address into `heap[1]` is the identical stale-`Term`
    ///    defect as H3, and it is why this arm could not simply be restored
    ///    before `alloc_words_rooted` existed.
    #[test]
    fn bs_get_binary_procbin_extraction_shares_forwarded_parent() {
        let mut process = Process::new(1, 256);
        // 100 bytes — above the inline threshold, so the source is a ProcBin.
        let raw: Vec<u8> = (0..100).map(|byte| byte as u8).collect();
        let source = {
            let mut ctx = test_context(&mut process, 0);
            ctx.alloc_binary(&raw).expect("procbin source")
        };
        let match_term = start_match_rooted(&mut process, source);
        let before = process.x_reg(0);

        fill_until(&mut process, SUB_BINARY_WORDS);
        assert!(
            process.heap().available() < SUB_BINARY_WORDS,
            "geometry must force the sub-binary allocation to collect"
        );
        assert_eq!(
            process.heap().old_used(),
            0,
            "nothing may be promoted before the subject call"
        );

        let out_raw = jit_bs_get_binary(&mut process, match_term.raw(), 160);
        assert_ne!(out_raw, 0, "sub-binary allocation must succeed");
        assert_ne!(out_raw, BINARY_HELPER_FAILURE);
        assert!(
            process.heap().old_used() > 0,
            "the extraction allocation must have run a collection"
        );
        let forwarded = process.x_reg(0);
        assert_ne!(
            forwarded, before,
            "live ProcBin box should be promoted by the collection"
        );

        // Claim 1. A copy comes back as a Binary or a ProcBin, never a
        // SubBinary, so this is the shape assertion the byte checks cannot make.
        //
        // The tag is read off the header rather than through `SubBinary::new`
        // ON PURPOSE. That constructor also VALIDATES the parent, so it refuses
        // a claim-2 violation too — and reports it with claim 1's message. Two
        // different defects arriving under one name is exactly the misdirection
        // this wall exists to prevent, so the claims are separated at the
        // instrument, not just in the comment. (Found by the M4 mutation, which
        // red through the wrong face until this was split.)
        let out = Term::from_raw(out_raw);
        let out_ptr = out.heap_ptr().expect("extraction result must be boxed") as *mut u64;
        assert_eq!(
            boxed_tag(out_ptr),
            Some(BoxedTag::SubBinary),
            "ProcBin extraction must SHARE through a sub-binary, not copy: \
             the O(1) arm is gone"
        );

        // Claim 2, stated directly and read straight out of the layout so no
        // validating constructor stands between the defect and its face. Raws
        // are reported so the evidence log carries the observed wrong term.
        let parent = Term::from_raw(read_word(out_ptr, 1));
        assert_eq!(
            parent,
            forwarded,
            "sub-binary stored a pre-move parent: stored={:#018x} forwarded={:#018x} original={:#018x}",
            parent.raw(),
            forwarded.raw(),
            before.raw()
        );

        // Only now the validating constructor: with both claims already
        // established, a refusal here means the VIEW itself is malformed.
        let extracted =
            SubBinary::new(out).expect("sub-binary must resolve against its parent binary");
        assert_eq!(extracted.len(), 20, "160 bits were requested");
        assert_eq!(extracted.parent(), forwarded);

        assert_eq!(
            extracted_bytes(out, process.borrow_terms()),
            raw[..20].to_vec()
        );
    }
}
