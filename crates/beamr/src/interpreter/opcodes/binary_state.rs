//! Heap-backed state objects for binary construction and matching opcodes.

use crate::term::Term;
use crate::term::binary::Binary;
use crate::term::boxed::{BoxedHeader, BoxedTag};

pub(crate) const BUILDER_META_WORDS: usize = 3;
pub(crate) const MATCH_CONTEXT_WORDS: usize = 4;

/// Heap-backed binary construction context.
#[derive(Copy, Clone)]
pub(crate) struct BinaryBuilder {
    ptr: *mut u64,
}

impl BinaryBuilder {
    pub(crate) fn new(term: Term) -> Option<Self> {
        let ptr = term.heap_ptr()? as *mut u64;
        if boxed_tag(ptr) == Some(BoxedTag::BinaryBuilder) {
            Some(Self { ptr })
        } else {
            None
        }
    }

    pub(crate) fn write_position_bits(self) -> usize {
        read_word(self.ptr, 1) as usize
    }

    pub(crate) fn set_write_position_bits(self, bits: usize) {
        write_word(self.ptr, 1, bits as u64);
    }

    pub(crate) fn capacity_bytes(self) -> usize {
        read_word(self.ptr, 2) as usize
    }

    pub(crate) fn can_append(self, bits: usize) -> bool {
        self.write_position_bits()
            .checked_add(bits)
            .is_some_and(|end| end <= self.capacity_bytes() * u8::BITS as usize)
    }

    pub(crate) fn write_bytes(self, start: usize, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            let index = start + offset;
            let word_offset = BUILDER_META_WORDS + index / std::mem::size_of::<u64>();
            let shift = (index % std::mem::size_of::<u64>()) * u8::BITS as usize;
            let mut word = read_word(self.ptr, word_offset);
            word &= !(0xff_u64 << shift);
            word |= u64::from(byte) << shift;
            write_word(self.ptr, word_offset, word);
        }
    }

    pub(crate) fn bytes(self, len: usize) -> Option<&'static [u8]> {
        if len > self.capacity_bytes() {
            return None;
        }
        Some(slice_from_words(self.ptr, BUILDER_META_WORDS, len))
    }
}

/// Heap-backed binary match context that keeps the source binary term alive.
#[derive(Copy, Clone)]
pub(crate) struct MatchContext {
    ptr: *mut u64,
}

impl MatchContext {
    pub(crate) fn new(term: Term) -> Option<Self> {
        let ptr = term.heap_ptr()? as *mut u64;
        if boxed_tag(ptr) == Some(BoxedTag::MatchContext) {
            Some(Self { ptr })
        } else {
            None
        }
    }

    pub(crate) fn position_bits(self) -> usize {
        read_word(self.ptr, 1) as usize
    }

    pub(crate) fn set_position_bits(self, bits: usize) {
        write_word(self.ptr, 1, bits as u64);
    }

    pub(crate) fn total_bits(self) -> usize {
        read_word(self.ptr, 2) as usize
    }

    pub(crate) fn remaining_bits(self) -> usize {
        self.total_bits().saturating_sub(self.position_bits())
    }

    pub(crate) fn has_bits(self, bits: usize) -> bool {
        self.position_bits()
            .checked_add(bits)
            .is_some_and(|end| end <= self.total_bits())
    }

    pub(crate) fn slice(self, bits: usize) -> Option<&'static [u8]> {
        if !bits.is_multiple_of(u8::BITS as usize)
            || !self.position_bits().is_multiple_of(u8::BITS as usize)
        {
            return None;
        }
        let start = self.position_bits() / u8::BITS as usize;
        let len = bits / u8::BITS as usize;
        let end = start.checked_add(len)?;
        let bytes = self.source()?.as_bytes();
        bytes.get(start..end)
    }

    fn source(self) -> Option<Binary> {
        Binary::new(Term::from_raw(read_word(self.ptr, 3)))
    }
}

pub(crate) fn boxed_header(tag: BoxedTag, payload_words: usize) -> u64 {
    BoxedHeader::new(tag, payload_words)
}

pub(crate) fn heap_slice<'a>(ptr: *mut u64, words: usize) -> &'a mut [u64] {
    // SAFETY: `Heap::alloc(words)` returned a unique allocation with exactly
    // `words` contiguous words that this handler immediately initialises.
    unsafe { std::slice::from_raw_parts_mut(ptr, words) }
}

fn boxed_tag(ptr: *const u64) -> Option<BoxedTag> {
    BoxedHeader::tag(read_word(ptr.cast_mut(), 0))
}

fn read_word(ptr: *mut u64, offset: usize) -> u64 {
    // SAFETY: callers construct these accessors only from live boxed heap terms
    // with a known layout and then read in-bounds metadata/data words.
    unsafe { *ptr.add(offset) }
}

fn write_word(ptr: *mut u64, offset: usize, value: u64) {
    // SAFETY: callers construct these accessors only from live mutable process
    // heap objects and write in-bounds metadata/data words.
    unsafe { *ptr.add(offset) = value }
}

fn slice_from_words(ptr: *const u64, word_offset: usize, len: usize) -> &'static [u8] {
    // SAFETY: inline data starts at `word_offset`; callers have checked that
    // `len` stays within the object's capacity. The returned slice is borrowed
    // only while the process heap object is live.
    unsafe { std::slice::from_raw_parts(ptr.add(word_offset).cast::<u8>(), len) }
}
