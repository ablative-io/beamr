//! Per-module constant pool for decoded BEAM literals.
//!
//! Literal table entries are materialised once while a module is loaded. Boxed
//! and list literals point into heap-compatible words owned by the module rather
//! than into process heaps; immediate literals are stored directly as terms.

use crate::error::LoadError;
use crate::loader::Literal;
use crate::term::Term;
use crate::term::binary::{packed_word_count, write_binary};
use crate::term::boxed::{write_bigint, write_cons, write_float, write_map, write_tuple};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum PoolRoot {
    Immediate(Term),
    Boxed { block: usize, offset: usize },
    List { block: usize, offset: usize },
}

impl PoolRoot {
    fn term(self, blocks: &[Box<[u64]>]) -> Option<Term> {
        match self {
            Self::Immediate(term) => Some(term),
            Self::Boxed { block, offset } => blocks
                .get(block)
                .and_then(|words| words.get(offset).map(|word| word as *const u64))
                .map(Term::boxed_ptr),
            Self::List { block, offset } => blocks
                .get(block)
                .and_then(|words| words.get(offset).map(|word| word as *const u64))
                .map(Term::list_ptr),
        }
    }
}

/// Module-owned storage for pre-materialised literal terms.
#[derive(Debug, Default)]
pub struct ConstantPool {
    blocks: Vec<Box<[u64]>>,
    roots: Vec<PoolRoot>,
}

impl Clone for ConstantPool {
    fn clone(&self) -> Self {
        Self {
            blocks: self.blocks.clone(),
            roots: self.roots.clone(),
        }
    }
}

impl ConstantPool {
    /// Creates an empty constant pool.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            blocks: Vec::new(),
            roots: Vec::new(),
        }
    }

    /// Returns the materialised term for literal table entry `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<Term> {
        self.roots
            .get(index)
            .and_then(|root| root.term(&self.blocks))
    }

    /// Returns the number of literal entries tracked by this pool.
    #[must_use]
    pub fn len(&self) -> usize {
        self.roots.len()
    }

    /// Returns true when the pool has no literal entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Returns the number of owned heap blocks in this pool.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    fn push_immediate(&mut self, term: Term) {
        self.roots.push(PoolRoot::Immediate(term));
    }

    fn push_block(&mut self, block: Box<[u64]>, root_offset: usize, is_list: bool) {
        let block_index = self.blocks.len();
        self.blocks.push(block);
        let root = if is_list {
            PoolRoot::List {
                block: block_index,
                offset: root_offset,
            }
        } else {
            PoolRoot::Boxed {
                block: block_index,
                offset: root_offset,
            }
        };
        self.roots.push(root);
    }
}

/// Converts decoded literals into a module-owned constant pool.
pub fn materialise_literals(literals: &[Literal]) -> Result<ConstantPool, LoadError> {
    let mut pool = ConstantPool::new();
    for literal in literals {
        let words = literal_word_count(literal)?;
        if words == 0 {
            let mut empty = [];
            let mut cursor = 0;
            pool.push_immediate(materialise_into(literal, &mut empty, &mut cursor)?);
            continue;
        }

        let mut block = vec![0_u64; words].into_boxed_slice();
        let mut cursor = 0;
        let root = materialise_into(literal, &mut block, &mut cursor)?;
        let root_offset = root_offset(root, &block)?;
        pool.push_block(block, root_offset, root.is_list());
    }
    Ok(pool)
}

fn immediate_term(literal: &Literal) -> Result<Term, LoadError> {
    match literal {
        Literal::Integer(value) => Term::try_small_int(*value)
            .ok_or_else(|| LoadError::ValidationError("literal integer is not small".into())),
        Literal::Atom(atom) => Ok(Term::atom(*atom)),
        Literal::Nil => Ok(Term::NIL),
        _ => Err(LoadError::ValidationError(
            "non-immediate literal has no constant-pool block".into(),
        )),
    }
}

fn root_offset(term: Term, block: &[u64]) -> Result<usize, LoadError> {
    let Some(ptr) = term.heap_ptr() else {
        return Err(LoadError::ValidationError(
            "constant-pool root is not a heap pointer".into(),
        ));
    };
    let base = block.as_ptr() as usize;
    let ptr = ptr as usize;
    let bytes = ptr.checked_sub(base).ok_or_else(|| {
        LoadError::ValidationError("constant-pool root points before block".into())
    })?;
    let word_size = std::mem::size_of::<u64>();
    if !bytes.is_multiple_of(word_size) {
        return Err(LoadError::ValidationError(
            "constant-pool root is not word aligned".into(),
        ));
    }
    let offset = bytes / word_size;
    if offset >= block.len() {
        return Err(LoadError::ValidationError(
            "constant-pool root points outside block".into(),
        ));
    }
    Ok(offset)
}

fn materialise_into(
    literal: &Literal,
    block: &mut [u64],
    cursor: &mut usize,
) -> Result<Term, LoadError> {
    match literal {
        Literal::Integer(_) | Literal::Atom(_) | Literal::Nil => immediate_term(literal),
        Literal::Float(value) => {
            let heap = reserve(block, cursor, 2)?;
            write_float(heap, *value).ok_or_else(write_error)
        }
        Literal::BigInteger(bytes) => {
            let limbs = limbs_to_u64(bytes)?;
            let heap = reserve(block, cursor, 3 + limbs.len())?;
            write_bigint(heap, false, &limbs).ok_or_else(write_error)
        }
        Literal::Binary(bytes) | Literal::String(bytes) => {
            let heap = reserve(block, cursor, 2 + packed_word_count(bytes.len()))?;
            write_binary(heap, bytes).ok_or_else(write_error)
        }
        Literal::Tuple(elements) => {
            let mut terms = Vec::with_capacity(elements.len());
            for element in elements {
                terms.push(materialise_into(element, block, cursor)?);
            }
            let heap = reserve(block, cursor, 1 + terms.len())?;
            write_tuple(heap, &terms).ok_or_else(write_error)
        }
        Literal::List(elements, tail) => {
            let mut result = materialise_into(tail, block, cursor)?;
            for element in elements.iter().rev() {
                let head = materialise_into(element, block, cursor)?;
                let heap = reserve(block, cursor, 2)?;
                result = write_cons(heap, head, result).ok_or_else(write_error)?;
            }
            Ok(result)
        }
        Literal::Map(entries) => {
            let mut pairs = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                pairs.push((
                    materialise_into(key, block, cursor)?,
                    materialise_into(value, block, cursor)?,
                ));
            }
            pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
            let keys: Vec<_> = pairs.iter().map(|(key, _)| *key).collect();
            let values: Vec<_> = pairs.iter().map(|(_, value)| *value).collect();
            let heap = reserve(block, cursor, 2 + keys.len() + values.len())?;
            write_map(heap, &keys, &values).ok_or_else(write_error)
        }
    }
}

fn reserve<'a>(
    block: &'a mut [u64],
    cursor: &mut usize,
    words: usize,
) -> Result<&'a mut [u64], LoadError> {
    let start = *cursor;
    let end = start
        .checked_add(words)
        .ok_or_else(|| LoadError::ValidationError("constant-pool block size overflow".into()))?;
    if end > block.len() {
        return Err(LoadError::ValidationError(
            "constant-pool block too small".into(),
        ));
    }
    *cursor = end;
    Ok(&mut block[start..end])
}

fn literal_word_count(literal: &Literal) -> Result<usize, LoadError> {
    match literal {
        Literal::Integer(_) | Literal::Atom(_) | Literal::Nil => Ok(0),
        Literal::Float(_) => Ok(2),
        Literal::BigInteger(bytes) => limbs_to_u64(bytes).map(|limbs| 3 + limbs.len()),
        Literal::Binary(bytes) | Literal::String(bytes) => Ok(2 + packed_word_count(bytes.len())),
        Literal::Tuple(elements) => {
            let mut words = 1 + elements.len();
            for element in elements {
                words = words
                    .checked_add(literal_word_count(element)?)
                    .ok_or_else(|| {
                        LoadError::ValidationError("constant-pool tuple size overflow".into())
                    })?;
            }
            Ok(words)
        }
        Literal::List(elements, tail) => {
            let mut words = literal_word_count(tail)?;
            for element in elements {
                words = words
                    .checked_add(literal_word_count(element)?)
                    .and_then(|count| count.checked_add(2))
                    .ok_or_else(|| {
                        LoadError::ValidationError("constant-pool list size overflow".into())
                    })?;
            }
            Ok(words)
        }
        Literal::Map(entries) => {
            let mut words = 2 + entries.len() * 2;
            for (key, value) in entries {
                words = words
                    .checked_add(literal_word_count(key)?)
                    .and_then(|count| count.checked_add(literal_word_count(value)?))
                    .ok_or_else(|| {
                        LoadError::ValidationError("constant-pool map size overflow".into())
                    })?;
            }
            Ok(words)
        }
    }
}

fn limbs_to_u64(bytes: &[u8]) -> Result<Vec<u64>, LoadError> {
    if !bytes.len().is_multiple_of(8) {
        return Err(LoadError::ValidationError(
            "unsupported bigint literal limb width".into(),
        ));
    }
    let mut limbs = Vec::with_capacity(bytes.len() / 8);
    for chunk in bytes.chunks_exact(8) {
        let mut limb = [0_u8; 8];
        limb.copy_from_slice(chunk);
        limbs.push(u64::from_le_bytes(limb));
    }
    Ok(limbs)
}

fn write_error() -> LoadError {
    LoadError::ValidationError("failed to write constant-pool literal".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atom::Atom;
    use crate::term::binary::Binary;
    use crate::term::boxed::{Cons, Float, Map, Tuple};

    #[test]
    fn materialises_immediate_and_boxed_literals() {
        let literals = vec![
            Literal::Integer(7),
            Literal::List(Vec::new(), Box::new(Literal::Nil)),
            Literal::Float(3.5),
            Literal::Binary(b"beam".to_vec()),
            Literal::Tuple(vec![Literal::Integer(1), Literal::Nil]),
            Literal::List(
                vec![Literal::Integer(2), Literal::Integer(3)],
                Box::new(Literal::Nil),
            ),
            Literal::Map(vec![(
                Literal::Atom(Atom::TRUE),
                Literal::String(b"ok".to_vec()),
            )]),
        ];
        let pool = materialise_literals(&literals).expect("pool materialises");

        assert_eq!(pool.len(), literals.len());
        assert_eq!(pool.block_count(), 5);
        assert_eq!(pool.get(0).and_then(Term::as_small_int), Some(7));
        assert!(pool.get(1).is_some_and(Term::is_nil));
        assert_eq!(
            Float::new(pool.get(2).expect("float")).map(Float::value),
            Some(3.5)
        );
        assert_eq!(
            Binary::new(pool.get(3).expect("binary")).map(|binary| binary.as_bytes()),
            Some(&b"beam"[..])
        );

        let tuple = Tuple::new(pool.get(4).expect("tuple")).expect("tuple accessor");
        assert_eq!(tuple.arity(), 2);
        assert_eq!(tuple.get(0).and_then(Term::as_small_int), Some(1));
        assert!(tuple.get(1).is_some_and(Term::is_nil));

        let first = Cons::new(pool.get(5).expect("list")).expect("first cons");
        assert_eq!(first.head().as_small_int(), Some(2));
        let second = Cons::new(first.tail()).expect("second cons");
        assert_eq!(second.head().as_small_int(), Some(3));
        assert!(second.tail().is_nil());

        let map = Map::new(pool.get(6).expect("map")).expect("map accessor");
        assert_eq!(map.len(), 1);
        assert_eq!(map.key(0).and_then(Term::as_atom), Some(Atom::TRUE));
        let value = map
            .value(0)
            .and_then(Binary::new)
            .expect("map binary value");
        assert_eq!(value.as_bytes(), b"ok");
    }

    #[test]
    fn repeated_get_returns_same_pointer() {
        let pool = materialise_literals(&[Literal::Tuple(vec![Literal::Integer(1)])])
            .expect("pool materialises");
        let first = pool.get(0).expect("first get");
        let second = pool.get(0).expect("second get");
        assert_eq!(first.raw(), second.raw());
        assert_eq!(first.heap_ptr(), second.heap_ptr());
    }

    #[test]
    fn cloned_pool_rebuilds_terms_to_point_at_cloned_blocks() {
        let pool =
            materialise_literals(&[Literal::Binary(b"owned".to_vec())]).expect("pool materialises");
        let cloned = pool.clone();
        let original = pool.get(0).expect("original term");
        let copied = cloned.get(0).expect("cloned term");

        assert_ne!(original.heap_ptr(), copied.heap_ptr());
        assert_eq!(
            Binary::new(copied).map(|binary| binary.as_bytes()),
            Some(&b"owned"[..])
        );
    }
}
