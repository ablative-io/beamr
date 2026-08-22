//! Bignum ordering — heap bignums and owned `BigIntValue`s.
//!
//! Split out of `compare/mod.rs` so the heap-witness plumbing and its safety
//! comments do not grow that module, which is already past the repository's
//! 500-line file wall.

use core::cmp::Ordering;

use super::super::heap_borrow::HeapBorrow;
use super::super::{
    Term,
    bigint_math::{BigIntValue, cmp_abs},
    boxed::BigInt,
};

pub(super) fn compare_bigints(left: Term, right: Term) -> Ordering {
    match (BigInt::new(left), BigInt::new(right)) {
        (Some(left), Some(right)) => compare_bigint_values(left, right),
        _ => left.raw().cmp(&right.raw()),
    }
}

fn compare_bigint_values(left: BigInt, right: BigInt) -> Ordering {
    // SAFETY: as `compare_binaries` — the witness and both limb slices are
    // confined to the closure by its higher-ranked lifetime, and the body only
    // compares words. No allocation, collection or heap drop occurs inside.
    unsafe { HeapBorrow::with_frame(|heap| compare_bigint_limbs(left, right, heap)) }
}

fn compare_bigint_limbs(left: BigInt, right: BigInt, heap: HeapBorrow<'_>) -> Ordering {
    let left_limbs = normalized_limbs(left, heap);
    let right_limbs = normalized_limbs(right, heap);
    let left_negative = left.is_negative() && !left_limbs.is_empty();
    let right_negative = right.is_negative() && !right_limbs.is_empty();

    match (left_negative, right_negative) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => compare_magnitude(left_limbs, right_limbs),
        (true, true) => compare_magnitude(left_limbs, right_limbs).reverse(),
    }
}

fn compare_small_magnitude(left: u64, right_limbs: &[u64]) -> Ordering {
    match right_limbs.len().cmp(&1) {
        Ordering::Less => left.cmp(&0),
        Ordering::Equal => left.cmp(&right_limbs[0]),
        Ordering::Greater => Ordering::Less,
    }
}

fn compare_magnitude(left: &[u64], right: &[u64]) -> Ordering {
    match left.len().cmp(&right.len()) {
        Ordering::Equal => left.iter().rev().cmp(right.iter().rev()),
        order => order,
    }
}

fn normalized_limbs<'heap>(bigint: BigInt, heap: HeapBorrow<'heap>) -> &'heap [u64] {
    let limbs = bigint.limbs(heap);
    let significant_len = limbs
        .iter()
        .rposition(|limb| *limb != 0)
        .map_or(0, |index| index + 1);
    &limbs[..significant_len]
}

pub(super) fn bigint_value_to_f64(value: &BigIntValue) -> f64 {
    let mut result = 0.0_f64;
    for limb in value.limbs().iter().rev() {
        result = result.mul_add(18_446_744_073_709_551_616.0, *limb as f64);
    }

    if value.is_negative() && result != 0.0 {
        -result
    } else {
        result
    }
}

/// Compares a small integer against an owned bigint value by sign and magnitude.
pub(super) fn compare_small_int_to_bigint_value(small: i64, big: &BigIntValue) -> Ordering {
    let big_negative = big.is_negative();

    match (small.is_negative(), big_negative) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => compare_small_magnitude(small.unsigned_abs(), big.limbs()),
        (true, true) => compare_small_magnitude(small.unsigned_abs(), big.limbs()).reverse(),
    }
}

/// Compares two owned bigint values by sign and magnitude.
pub(super) fn compare_bigint_values_owned(left: &BigIntValue, right: &BigIntValue) -> Ordering {
    let left_negative = left.is_negative();
    let right_negative = right.is_negative();

    match (left_negative, right_negative) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => cmp_abs(left.limbs(), right.limbs()),
        (true, true) => cmp_abs(left.limbs(), right.limbs()).reverse(),
    }
}
