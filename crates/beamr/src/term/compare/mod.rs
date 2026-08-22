//! Term ordering and equality — `==` (number coercion) and `=:=` (exact).
//! BEAM order: number < atom < reference < fun < port < pid <
//! tuple < map < nil < list < binary.

use std::cmp::Ordering;

mod bigint;

use super::heap_borrow::HeapBorrow;
use super::pid_ref::PidRef;
use super::reference_ref::ReferenceRef;
use super::{
    Term,
    bigint_math::BigIntValue,
    binary_ref::BinaryRef,
    boxed::{BigInt, Closure, Cons, Float, Map, Tuple},
};
use crate::atom::AtomTable;
use bigint::{
    bigint_value_to_f64, compare_bigint_values_owned, compare_bigints,
    compare_small_int_to_bigint_value,
};

/// Compares two terms using Erlang `=:=` exact equality semantics.
#[must_use]
pub fn exact_eq(left: Term, right: Term) -> bool {
    exact_cmp(left, right) == Ordering::Equal
}

/// Orders two terms using Erlang `=:=` exact term semantics.
#[must_use]
pub(crate) fn exact_cmp(left: Term, right: Term) -> Ordering {
    compare_exact(left, right)
}

/// Compares two terms using Erlang `==` semantics.
///
/// Numeric pairs compare by value across small integers, bignums and floats.
/// Mixed integer/float pairs compare after converting the integer operand to
/// `f64`, which matches the conversion [`compare_numbers`] uses so `==` and the
/// ordering operators agree. All non-numeric pairs use exact equality.
#[must_use]
pub fn numeric_eq(left: Term, right: Term) -> bool {
    match (number_value(left), number_value(right)) {
        (Some(NumberValue::SmallInt(left)), Some(NumberValue::SmallInt(right))) => left == right,
        (Some(NumberValue::SmallInt(left)), Some(NumberValue::Float(right))) => {
            left as f64 == right
        }
        (Some(NumberValue::SmallInt(left)), Some(NumberValue::BigInt(right))) => {
            BigIntValue::from_i64(left) == right
        }
        (Some(NumberValue::Float(left)), Some(NumberValue::SmallInt(right))) => {
            left == right as f64
        }
        (Some(NumberValue::Float(left)), Some(NumberValue::Float(right))) => left == right,
        (Some(NumberValue::Float(left)), Some(NumberValue::BigInt(right))) => {
            left == bigint_value_to_f64(&right)
        }
        (Some(NumberValue::BigInt(left)), Some(NumberValue::SmallInt(right))) => {
            left == BigIntValue::from_i64(right)
        }
        (Some(NumberValue::BigInt(left)), Some(NumberValue::Float(right))) => {
            bigint_value_to_f64(&left) == right
        }
        (Some(NumberValue::BigInt(left)), Some(NumberValue::BigInt(right))) => left == right,
        (None, _) | (_, None) => exact_eq(left, right),
    }
}

/// Compares two terms using the BEAM term order.
#[must_use]
pub fn cmp(left: Term, right: Term, atom_table: &AtomTable) -> Ordering {
    let left_rank = rank(left);
    let right_rank = rank(right);
    match left_rank.cmp(&right_rank) {
        Ordering::Equal => compare_same_rank(left, right, left_rank, atom_table),
        order => order,
    }
}

/// Legacy table-free ordering used only by [`Term`]'s `Ord` implementation.
///
/// VM-visible ordering must call [`cmp`] with the runtime atom table so atom
/// names are compared instead of raw intern indices.
#[must_use]
pub(crate) fn raw_cmp(left: Term, right: Term) -> Ordering {
    let left_rank = rank(left);
    let right_rank = rank(right);
    match left_rank.cmp(&right_rank) {
        Ordering::Equal => compare_same_rank_raw(left, right, left_rank),
        order => order,
    }
}

pub(crate) fn partial_eq(left: &Term, right: &Term) -> bool {
    exact_eq(*left, *right)
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
#[allow(dead_code)]
enum TermRank {
    Number,
    Atom,
    Reference,
    Fun,
    // No port representation exists yet; keep the BEAM rank slot reserved so
    // future port terms sort between fun and pid without renumbering ranks.
    Port,
    Pid,
    Tuple,
    Map,
    Nil,
    List,
    Binary,
    OtherBoxed,
}

#[derive(Clone)]
enum NumberValue {
    SmallInt(i64),
    Float(f64),
    BigInt(BigIntValue),
}

fn rank(term: Term) -> TermRank {
    if term.is_small_int() || Float::new(term).is_some() || BigInt::new(term).is_some() {
        TermRank::Number
    } else if term.is_atom() {
        TermRank::Atom
    } else if ReferenceRef::new(term).is_some() {
        TermRank::Reference
    } else if Closure::new(term).is_some() {
        TermRank::Fun
    } else if PidRef::new(term).is_some() {
        TermRank::Pid
    } else if Tuple::new(term).is_some() {
        TermRank::Tuple
    } else if Map::new(term).is_some() {
        TermRank::Map
    } else if term.is_nil() {
        TermRank::Nil
    } else if term.is_list() {
        TermRank::List
    } else if BinaryRef::new(term).is_some() {
        TermRank::Binary
    } else {
        TermRank::OtherBoxed
    }
}

fn number_value(term: Term) -> Option<NumberValue> {
    if let Some(value) = term.as_small_int() {
        Some(NumberValue::SmallInt(value))
    } else if let Some(float) = Float::new(term) {
        Some(NumberValue::Float(float.value()))
    } else {
        BigInt::new(term).map(|bigint| {
            // SAFETY: `with_frame` hands out a witness bounded to this call and
            // `from_bigint` copies the limbs into owned storage before it
            // returns, so no borrow of heap words survives the closure. Nothing
            // inside allocates on, collects, or drops a process heap — the
            // whole body is a `Vec<u64>` copy. `Term`'s `PartialEq`/`Ord`
            // reach here through signatures that cannot carry a witness.
            let value =
                unsafe { HeapBorrow::with_frame(|heap| BigIntValue::from_bigint(bigint, heap)) };
            NumberValue::BigInt(value)
        })
    }
}

fn compare_same_rank(
    left: Term,
    right: Term,
    term_rank: TermRank,
    atom_table: &AtomTable,
) -> Ordering {
    match term_rank {
        TermRank::Number => compare_numbers(left, right),
        TermRank::Atom => compare_atoms_by_name(left, right, atom_table),
        TermRank::Reference => reference_key(left).cmp(&reference_key(right)),
        TermRank::Fun => compare_closures(left, right, atom_table),
        TermRank::Port => Ordering::Equal,
        TermRank::Pid => pid_key(left).cmp(&pid_key(right)),
        TermRank::Tuple => compare_tuples(left, right, atom_table),
        TermRank::Map => compare_maps(left, right, atom_table),
        TermRank::Nil => Ordering::Equal,
        TermRank::List => compare_lists(left, right, atom_table),
        TermRank::Binary => compare_binaries(left, right),
        TermRank::OtherBoxed => left.raw().cmp(&right.raw()),
    }
}

fn compare_same_rank_raw(left: Term, right: Term, term_rank: TermRank) -> Ordering {
    match term_rank {
        TermRank::Number => compare_numbers(left, right),
        TermRank::Atom => left.raw().cmp(&right.raw()),
        TermRank::Reference => reference_key(left).cmp(&reference_key(right)),
        TermRank::Fun => compare_closures_raw(left, right),
        TermRank::Port => Ordering::Equal,
        TermRank::Pid => pid_key(left).cmp(&pid_key(right)),
        TermRank::Tuple => compare_tuples_raw(left, right),
        TermRank::Map => compare_maps_raw(left, right),
        TermRank::Nil => Ordering::Equal,
        TermRank::List => compare_lists_raw(left, right),
        TermRank::Binary => compare_binaries(left, right),
        TermRank::OtherBoxed => left.raw().cmp(&right.raw()),
    }
}

fn compare_atoms_by_name(left: Term, right: Term, atom_table: &AtomTable) -> Ordering {
    match (left.as_atom(), right.as_atom()) {
        (Some(left_atom), Some(right_atom)) => {
            match (
                atom_table.resolve(left_atom),
                atom_table.resolve(right_atom),
            ) {
                (Some(left_name), Some(right_name)) => left_name.cmp(right_name),
                _ => left.raw().cmp(&right.raw()),
            }
        }
        _ => left.raw().cmp(&right.raw()),
    }
}

fn compare_exact(left: Term, right: Term) -> Ordering {
    let left_kind = exact_kind(left);
    let right_kind = exact_kind(right);
    match left_kind.cmp(&right_kind) {
        Ordering::Equal => compare_same_exact_kind(left, right, left_kind),
        order => order,
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
enum ExactKind {
    SmallInt,
    Atom,
    Pid,
    Nil,
    Tuple,
    Float,
    BigInt,
    Closure,
    Map,
    Reference,
    Binary,
    List,
    Other,
}

fn exact_kind(term: Term) -> ExactKind {
    if term.is_small_int() {
        ExactKind::SmallInt
    } else if term.is_atom() {
        ExactKind::Atom
    } else if term.is_pid() {
        ExactKind::Pid
    } else if term.is_nil() {
        ExactKind::Nil
    } else if Tuple::new(term).is_some() {
        ExactKind::Tuple
    } else if Float::new(term).is_some() {
        ExactKind::Float
    } else if BigInt::new(term).is_some() {
        ExactKind::BigInt
    } else if Closure::new(term).is_some() {
        ExactKind::Closure
    } else if Map::new(term).is_some() {
        ExactKind::Map
    } else if ReferenceRef::new(term).is_some() {
        ExactKind::Reference
    } else if BinaryRef::new(term).is_some() {
        ExactKind::Binary
    } else if term.is_list() {
        ExactKind::List
    } else {
        ExactKind::Other
    }
}

fn compare_same_exact_kind(left: Term, right: Term, kind: ExactKind) -> Ordering {
    match kind {
        ExactKind::SmallInt => left.as_small_int().cmp(&right.as_small_int()),
        ExactKind::Atom => left.raw().cmp(&right.raw()),
        ExactKind::Pid => pid_key(left).cmp(&pid_key(right)),
        ExactKind::Nil => Ordering::Equal,
        ExactKind::Tuple => compare_tuples_exact(left, right),
        ExactKind::Float => float_bits(left).cmp(&float_bits(right)),
        ExactKind::BigInt => compare_bigints(left, right),
        ExactKind::Closure => compare_closures_exact(left, right),
        ExactKind::Map => compare_maps_exact(left, right),
        ExactKind::Reference => reference_key(left).cmp(&reference_key(right)),
        ExactKind::Binary => compare_binaries(left, right),
        ExactKind::List => compare_lists_exact(left, right),
        ExactKind::Other => left.raw().cmp(&right.raw()),
    }
}

fn compare_numbers(left: Term, right: Term) -> Ordering {
    match (number_value(left), number_value(right)) {
        (Some(NumberValue::SmallInt(left)), Some(NumberValue::SmallInt(right))) => left.cmp(&right),
        (Some(NumberValue::SmallInt(left)), Some(NumberValue::Float(right))) => {
            compare_f64(left as f64, right)
        }
        (Some(NumberValue::SmallInt(left)), Some(NumberValue::BigInt(right))) => {
            compare_small_int_to_bigint_value(left, &right)
        }
        (Some(NumberValue::Float(left)), Some(NumberValue::SmallInt(right))) => {
            compare_f64(left, right as f64)
        }
        (Some(NumberValue::Float(left)), Some(NumberValue::Float(right))) => {
            compare_f64(left, right)
        }
        (Some(NumberValue::Float(left)), Some(NumberValue::BigInt(right))) => {
            compare_f64(left, bigint_value_to_f64(&right))
        }
        (Some(NumberValue::BigInt(left)), Some(NumberValue::SmallInt(right))) => {
            compare_small_int_to_bigint_value(right, &left).reverse()
        }
        (Some(NumberValue::BigInt(left)), Some(NumberValue::Float(right))) => {
            compare_f64(bigint_value_to_f64(&left), right)
        }
        (Some(NumberValue::BigInt(left)), Some(NumberValue::BigInt(right))) => {
            compare_bigint_values_owned(&left, &right)
        }
        (None, _) | (_, None) => compare_bigints(left, right),
    }
}

fn compare_f64(left: f64, right: f64) -> Ordering {
    left.total_cmp(&right)
}

fn float_bits(term: Term) -> Option<u64> {
    let float = Float::new(term)?;
    Some(float.value().to_bits())
}

fn pid_key(term: Term) -> Option<(Option<u32>, u64, u64)> {
    PidRef::new(term).map(|pid| {
        (
            pid.node().map(|node| node.index()),
            pid.pid_number(),
            pid.serial(),
        )
    })
}

fn reference_key(term: Term) -> Option<(Option<u32>, u64)> {
    ReferenceRef::new(term)
        .map(|reference| (reference.node().map(|node| node.index()), reference.id()))
}

fn binary_bytes<'heap>(term: Term, heap: HeapBorrow<'heap>) -> &'heap [u8] {
    BinaryRef::new(term).map_or(&[], |binary| binary.as_bytes(heap))
}

/// Orders two binary terms by content.
///
/// Term comparison is reached from `impl PartialEq for Term` (`term/mod.rs:65`)
/// and `impl Ord for Term` (`:79`), whose signatures have no room for a heap
/// witness. This is one of the three sites the design names as structurally
/// witness-less; see `docs/design/accessor-lifetimes.md` §4.
fn compare_binaries(left: Term, right: Term) -> Ordering {
    // SAFETY: `with_frame`'s witness is higher-ranked, so neither it nor the
    // two byte slices can escape the closure. The closure body is a slice
    // comparison: it performs no allocation, runs no collection, and drops no
    // heap, so nothing can invalidate the bytes while they are read.
    unsafe {
        HeapBorrow::with_frame(|heap| binary_bytes(left, heap).cmp(binary_bytes(right, heap)))
    }
}

fn compare_tuples(left: Term, right: Term, atom_table: &AtomTable) -> Ordering {
    match (Tuple::new(left), Tuple::new(right)) {
        (Some(left), Some(right)) => match left.arity().cmp(&right.arity()) {
            Ordering::Equal => compare_tuple_elements(left, right, atom_table),
            order => order,
        },
        _ => left.raw().cmp(&right.raw()),
    }
}

fn compare_tuples_raw(left: Term, right: Term) -> Ordering {
    match (Tuple::new(left), Tuple::new(right)) {
        (Some(left), Some(right)) => match left.arity().cmp(&right.arity()) {
            Ordering::Equal => compare_tuple_elements_raw(left, right),
            order => order,
        },
        _ => left.raw().cmp(&right.raw()),
    }
}

fn compare_tuples_exact(left: Term, right: Term) -> Ordering {
    match (Tuple::new(left), Tuple::new(right)) {
        (Some(left), Some(right)) => match left.arity().cmp(&right.arity()) {
            Ordering::Equal => compare_tuple_elements_exact(left, right),
            order => order,
        },
        _ => left.raw().cmp(&right.raw()),
    }
}

fn compare_tuple_elements(left: Tuple, right: Tuple, atom_table: &AtomTable) -> Ordering {
    for index in 0..left.arity() {
        if let (Some(left_element), Some(right_element)) = (left.get(index), right.get(index)) {
            match cmp(left_element, right_element, atom_table) {
                Ordering::Equal => {}
                order => return order,
            }
        }
    }
    Ordering::Equal
}

fn compare_tuple_elements_raw(left: Tuple, right: Tuple) -> Ordering {
    for index in 0..left.arity() {
        if let (Some(left_element), Some(right_element)) = (left.get(index), right.get(index)) {
            match raw_cmp(left_element, right_element) {
                Ordering::Equal => {}
                order => return order,
            }
        }
    }
    Ordering::Equal
}

fn compare_tuple_elements_exact(left: Tuple, right: Tuple) -> Ordering {
    for index in 0..left.arity() {
        if let (Some(left_element), Some(right_element)) = (left.get(index), right.get(index)) {
            match compare_exact(left_element, right_element) {
                Ordering::Equal => {}
                order => return order,
            }
        }
    }
    Ordering::Equal
}

fn compare_lists(left: Term, right: Term, atom_table: &AtomTable) -> Ordering {
    compare_lists_with(left, right, |left, right| cmp(left, right, atom_table))
}

fn compare_lists_raw(left: Term, right: Term) -> Ordering {
    compare_lists_with(left, right, raw_cmp)
}

fn compare_lists_exact(left: Term, right: Term) -> Ordering {
    compare_lists_with(left, right, compare_exact)
}

fn compare_lists_with(
    mut left: Term,
    mut right: Term,
    mut element_cmp: impl FnMut(Term, Term) -> Ordering,
) -> Ordering {
    loop {
        match (Cons::new(left), Cons::new(right)) {
            (Some(left_cons), Some(right_cons)) => {
                match element_cmp(left_cons.head(), right_cons.head()) {
                    Ordering::Equal => {
                        left = left_cons.tail();
                        right = right_cons.tail();
                    }
                    order => return order,
                }
            }
            _ => return element_cmp(left, right),
        }
    }
}

fn compare_maps(left: Term, right: Term, atom_table: &AtomTable) -> Ordering {
    compare_maps_with(left, right, |left, right| cmp(left, right, atom_table))
}

fn compare_maps_raw(left: Term, right: Term) -> Ordering {
    compare_maps_with(left, right, raw_cmp)
}

fn compare_maps_exact(left: Term, right: Term) -> Ordering {
    compare_maps_with(left, right, compare_exact)
}

fn compare_maps_with(
    left: Term,
    right: Term,
    mut element_cmp: impl FnMut(Term, Term) -> Ordering,
) -> Ordering {
    match (Map::new(left), Map::new(right)) {
        (Some(left), Some(right)) => {
            let left_entries = sorted_map_entries(left, &mut element_cmp);
            let right_entries = sorted_map_entries(right, &mut element_cmp);
            match left_entries.len().cmp(&right_entries.len()) {
                Ordering::Equal => compare_map_entries(&left_entries, &right_entries, element_cmp),
                order => order,
            }
        }
        _ => left.raw().cmp(&right.raw()),
    }
}

#[derive(Copy, Clone)]
struct MapEntry {
    key: Term,
    value: Term,
}

fn sorted_map_entries(
    map: Map,
    element_cmp: &mut impl FnMut(Term, Term) -> Ordering,
) -> Vec<MapEntry> {
    let mut entries = Vec::with_capacity(map.len());
    for index in 0..map.len() {
        if let (Some(key), Some(value)) = (map.key(index), map.value(index)) {
            entries.push(MapEntry { key, value });
        }
    }
    entries.sort_by(|left, right| element_cmp(left.key, right.key));
    entries
}

fn compare_map_entries(
    left_entries: &[MapEntry],
    right_entries: &[MapEntry],
    mut element_cmp: impl FnMut(Term, Term) -> Ordering,
) -> Ordering {
    for (left, right) in left_entries.iter().zip(right_entries.iter()) {
        match element_cmp(left.key, right.key) {
            Ordering::Equal => match element_cmp(left.value, right.value) {
                Ordering::Equal => {}
                order => return order,
            },
            order => return order,
        }
    }
    Ordering::Equal
}

fn compare_closures(left: Term, right: Term, atom_table: &AtomTable) -> Ordering {
    compare_closures_with(left, right, atom_table, |left, right| {
        cmp(left, right, atom_table)
    })
}

fn compare_closures_raw(left: Term, right: Term) -> Ordering {
    compare_closures_with_raw(left, right, raw_cmp)
}

fn compare_closures_exact(left: Term, right: Term) -> Ordering {
    compare_closures_with_raw(left, right, compare_exact)
}

fn compare_closures_with(
    left: Term,
    right: Term,
    atom_table: &AtomTable,
    mut element_cmp: impl FnMut(Term, Term) -> Ordering,
) -> Ordering {
    match (Closure::new(left), Closure::new(right)) {
        (Some(left), Some(right)) => {
            match match (left.module(), right.module()) {
                (Some(left_module), Some(right_module)) => compare_atoms_by_name(
                    Term::atom(left_module),
                    Term::atom(right_module),
                    atom_table,
                ),
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            } {
                Ordering::Equal => {}
                order => return order,
            }
            match left.function_index().cmp(&right.function_index()) {
                Ordering::Equal => {}
                order => return order,
            }
            match left.arity().cmp(&right.arity()) {
                Ordering::Equal => {}
                order => return order,
            }
            match left.generation().cmp(&right.generation()) {
                Ordering::Equal => {}
                order => return order,
            }
            match left.unique_id().cmp(&right.unique_id()) {
                Ordering::Equal => {}
                order => return order,
            }
            match left.num_free().cmp(&right.num_free()) {
                Ordering::Equal => {}
                order => return order,
            }
            for index in 0..left.num_free() {
                if let (Some(left_free), Some(right_free)) =
                    (left.free_var(index), right.free_var(index))
                {
                    match element_cmp(left_free, right_free) {
                        Ordering::Equal => {}
                        order => return order,
                    }
                }
            }
            Ordering::Equal
        }
        _ => left.raw().cmp(&right.raw()),
    }
}

fn compare_closures_with_raw(
    left: Term,
    right: Term,
    mut element_cmp: impl FnMut(Term, Term) -> Ordering,
) -> Ordering {
    match (Closure::new(left), Closure::new(right)) {
        (Some(left), Some(right)) => {
            match left
                .module()
                .map(|module| Term::atom(module).raw())
                .cmp(&right.module().map(|module| Term::atom(module).raw()))
            {
                Ordering::Equal => {}
                order => return order,
            }
            match left.function_index().cmp(&right.function_index()) {
                Ordering::Equal => {}
                order => return order,
            }
            match left.arity().cmp(&right.arity()) {
                Ordering::Equal => {}
                order => return order,
            }
            match left.generation().cmp(&right.generation()) {
                Ordering::Equal => {}
                order => return order,
            }
            match left.unique_id().cmp(&right.unique_id()) {
                Ordering::Equal => {}
                order => return order,
            }
            match left.num_free().cmp(&right.num_free()) {
                Ordering::Equal => {}
                order => return order,
            }
            for index in 0..left.num_free() {
                if let (Some(left_free), Some(right_free)) =
                    (left.free_var(index), right.free_var(index))
                {
                    match element_cmp(left_free, right_free) {
                        Ordering::Equal => {}
                        order => return order,
                    }
                }
            }
            Ordering::Equal
        }
        _ => left.raw().cmp(&right.raw()),
    }
}

#[cfg(test)]
mod tests;
