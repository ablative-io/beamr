//! Conversion between BEAM terms and `serde_json::Value`.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde_json::{Map as JsonObject, Number, Value};

use crate::atom::{Atom, AtomTable};
use crate::native::ProcessContext;
use crate::term::{
    Tag, Term,
    binary_ref::BinaryRef,
    boxed::{BigInt, Cons, Float, Map, Tuple},
};

/// Error raised while converting between BEAM terms and JSON values.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonTermError {
    /// An atom term could not be resolved through the provided atom table.
    UnknownAtom(Atom),
    /// A boxed term used a layout this bridge does not represent as JSON.
    UnsupportedTerm(&'static str),
    /// A list tail was neither another cons cell nor `Term::NIL`.
    ImproperListTail(Term),
    /// A BEAM map key converted to a JSON value that cannot be an object key.
    NonStringMapKey(Value),
    /// A boxed float was NaN or infinite, which JSON numbers cannot encode.
    NonFiniteFloat(f64),
    /// A JSON number cannot be represented with the supported BEAM numeric terms.
    UnsupportedNumber(Number),
    /// Object key conversion requires a configured atom table in the process context.
    MissingAtomTable,
    /// A process heap allocation unexpectedly failed to write its boxed layout.
    AllocationFailed(&'static str),
}

impl fmt::Display for JsonTermError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAtom(atom) => write!(formatter, "unknown atom {atom:?}"),
            Self::UnsupportedTerm(term_type) => {
                write!(formatter, "unsupported term type {term_type}")
            }
            Self::ImproperListTail(tail) => write!(formatter, "improper list tail {tail:?}"),
            Self::NonStringMapKey(key) => write!(
                formatter,
                "map key converted to non-string JSON value {key:?}"
            ),
            Self::NonFiniteFloat(value) => write!(
                formatter,
                "non-finite float cannot be represented as JSON: {value}"
            ),
            Self::UnsupportedNumber(number) => {
                write!(formatter, "unsupported JSON number {number}")
            }
            Self::MissingAtomTable => {
                formatter.write_str("process context is missing an atom table")
            }
            Self::AllocationFailed(term_type) => {
                write!(formatter, "failed to allocate {term_type} term")
            }
        }
    }
}

impl std::error::Error for JsonTermError {}

/// Convert a BEAM term to a `serde_json::Value`.
pub fn term_to_value(term: Term, atom_table: &AtomTable) -> Result<Value, JsonTermError> {
    match term.tag() {
        Tag::SmallInt => Ok(Value::Number(Number::from(
            term.as_small_int()
                .ok_or(JsonTermError::UnsupportedTerm("small_int"))?,
        ))),
        Tag::Atom => atom_to_value(
            term.as_atom()
                .ok_or(JsonTermError::UnsupportedTerm("atom"))?,
            atom_table,
        ),
        Tag::Pid => Ok(Value::String(format!(
            "<0.{}.0>",
            term.as_pid().ok_or(JsonTermError::UnsupportedTerm("pid"))?
        ))),
        Tag::Nil => Ok(Value::Array(Vec::new())),
        Tag::List => list_to_value(term, atom_table),
        Tag::Boxed => boxed_to_value(term, atom_table),
    }
}

/// Convert a `serde_json::Value` to a BEAM term.
pub fn value_to_term(value: &Value, context: &mut ProcessContext) -> Result<Term, JsonTermError> {
    match value {
        Value::Null => {
            let atom_table = context
                .atom_table()
                .ok_or(JsonTermError::MissingAtomTable)?;
            Ok(Term::atom(atom_table.intern("null")))
        }
        Value::Bool(true) => Ok(Term::atom(Atom::TRUE)),
        Value::Bool(false) => Ok(Term::atom(Atom::FALSE)),
        Value::Number(number) => number_to_term(number, context),
        Value::String(string) => string_to_binary_term(string, context),
        Value::Array(elements) => array_to_list_term(elements, context),
        Value::Object(object) => object_to_map_term(object, context),
    }
}

fn atom_to_value(atom: Atom, atom_table: &AtomTable) -> Result<Value, JsonTermError> {
    match atom {
        Atom::TRUE => Ok(Value::Bool(true)),
        Atom::FALSE => Ok(Value::Bool(false)),
        Atom::NIL | Atom::UNDEFINED => Ok(Value::Null),
        other => {
            let name = atom_table
                .resolve(other)
                .ok_or(JsonTermError::UnknownAtom(other))?;
            if name == "null" {
                Ok(Value::Null)
            } else {
                Ok(Value::String(name.to_owned()))
            }
        }
    }
}

fn list_to_value(term: Term, atom_table: &AtomTable) -> Result<Value, JsonTermError> {
    let mut elements = Vec::new();
    let mut tail = term;
    loop {
        if tail.is_nil() {
            return Ok(Value::Array(elements));
        }

        let cons = Cons::new(tail).ok_or(JsonTermError::ImproperListTail(tail))?;
        elements.push(term_to_value(cons.head(), atom_table)?);
        tail = cons.tail();
    }
}

fn boxed_to_value(term: Term, atom_table: &AtomTable) -> Result<Value, JsonTermError> {
    if let Some(binary) = BinaryRef::new(term) {
        return binary_to_value(binary);
    }
    if let Some(tuple) = Tuple::new(term) {
        return tuple_to_value(tuple, atom_table);
    }
    if let Some(map) = Map::new(term) {
        return map_to_value(map, atom_table);
    }
    if let Some(float) = Float::new(term) {
        return float_to_value(float.value());
    }
    if let Some(bigint) = BigInt::new(term) {
        return bigint_to_value(bigint);
    }

    Err(JsonTermError::UnsupportedTerm("boxed"))
}

fn binary_to_value(binary: BinaryRef) -> Result<Value, JsonTermError> {
    match std::str::from_utf8(binary.as_bytes()) {
        Ok(text) => Ok(Value::String(text.to_owned())),
        Err(_) => Ok(Value::String(BASE64_STANDARD.encode(binary.as_bytes()))),
    }
}

fn tuple_to_value(tuple: Tuple, atom_table: &AtomTable) -> Result<Value, JsonTermError> {
    let mut values = Vec::with_capacity(tuple.arity());
    for index in 0..tuple.arity() {
        let element = tuple
            .get(index)
            .ok_or(JsonTermError::UnsupportedTerm("tuple"))?;
        values.push(term_to_value(element, atom_table)?);
    }
    Ok(Value::Array(values))
}

fn map_to_value(map: Map, atom_table: &AtomTable) -> Result<Value, JsonTermError> {
    let mut object = JsonObject::new();
    for index in 0..map.len() {
        let key = map
            .key(index)
            .ok_or(JsonTermError::UnsupportedTerm("map"))?;
        let key_name = map_key_to_string(key, atom_table)?;
        let value = map
            .value(index)
            .ok_or(JsonTermError::UnsupportedTerm("map"))?;
        object.insert(key_name, term_to_value(value, atom_table)?);
    }
    Ok(Value::Object(object))
}

fn map_key_to_string(term: Term, atom_table: &AtomTable) -> Result<String, JsonTermError> {
    if let Some(atom) = term.as_atom() {
        return atom_table
            .resolve(atom)
            .map(str::to_owned)
            .ok_or(JsonTermError::UnknownAtom(atom));
    }

    let key_value = term_to_value(term, atom_table)?;
    let Value::String(key_name) = key_value else {
        return Err(JsonTermError::NonStringMapKey(key_value));
    };
    Ok(key_name)
}

fn float_to_value(value: f64) -> Result<Value, JsonTermError> {
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or(JsonTermError::NonFiniteFloat(value))
}

fn bigint_to_value(bigint: BigInt) -> Result<Value, JsonTermError> {
    if bigint.limb_count() == 0 {
        return Ok(Value::Number(Number::from(0)));
    }

    if let Some(value) = bigint_to_i128(bigint) {
        if let Ok(signed) = i64::try_from(value) {
            return Ok(Value::Number(Number::from(signed)));
        }
        if let Ok(unsigned) = u64::try_from(value) {
            return Ok(Value::Number(Number::from(unsigned)));
        }
        return Ok(Value::String(value.to_string()));
    }

    Ok(Value::String(bigint_to_decimal_string(bigint)))
}

fn bigint_to_i128(bigint: BigInt) -> Option<i128> {
    let mut magnitude = 0_u128;
    for (index, limb) in bigint.limbs().iter().copied().enumerate() {
        let shift = index.checked_mul(u64::BITS as usize)?;
        let shifted = u128::from(limb).checked_shl(shift as u32)?;
        magnitude = magnitude.checked_add(shifted)?;
    }

    if bigint.is_negative() {
        if magnitude == (i128::MAX as u128) + 1 {
            Some(i128::MIN)
        } else {
            i128::try_from(magnitude).ok().map(|value| -value)
        }
    } else {
        i128::try_from(magnitude).ok()
    }
}

fn bigint_to_decimal_string(bigint: BigInt) -> String {
    let mut limbs = bigint.limbs().to_vec();
    while limbs.last().copied() == Some(0) {
        limbs.pop();
    }

    if limbs.is_empty() {
        return "0".to_owned();
    }

    let mut digits = Vec::new();
    while limbs.iter().any(|limb| *limb != 0) {
        let remainder = div_rem_limbs_by_10(&mut limbs);
        digits.push(char::from(b'0' + remainder as u8));
        while limbs.last().copied() == Some(0) {
            limbs.pop();
        }
    }

    if bigint.is_negative() {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

fn div_rem_limbs_by_10(limbs: &mut [u64]) -> u64 {
    let mut remainder = 0_u128;
    for limb in limbs.iter_mut().rev() {
        let value = (remainder << u64::BITS) | u128::from(*limb);
        *limb = (value / 10) as u64;
        remainder = value % 10;
    }
    remainder as u64
}

fn number_to_term(number: &Number, context: &mut ProcessContext) -> Result<Term, JsonTermError> {
    if let Some(value) = number.as_i64() {
        if let Some(term) = Term::try_small_int(value) {
            return Ok(term);
        }
        return allocate_bigint_from_i128(i128::from(value), context);
    }

    if let Some(value) = number.as_u64() {
        if let Ok(signed) = i64::try_from(value)
            && let Some(term) = Term::try_small_int(signed)
        {
            return Ok(term);
        }
        return allocate_bigint_from_u64(value, context);
    }

    let value = number
        .as_f64()
        .ok_or_else(|| JsonTermError::UnsupportedNumber(number.clone()))?;
    allocate_float_term(value, context)
}

fn allocate_bigint_from_i128(
    value: i128,
    context: &mut ProcessContext,
) -> Result<Term, JsonTermError> {
    let negative = value.is_negative();
    let magnitude = value.unsigned_abs();
    let limbs = limbs_from_u128(magnitude);
    allocate_bigint_term(negative, &limbs, context)
}

fn allocate_bigint_from_u64(
    value: u64,
    context: &mut ProcessContext,
) -> Result<Term, JsonTermError> {
    allocate_bigint_term(false, &[value], context)
}

fn allocate_bigint_term(
    negative: bool,
    limbs: &[u64],
    context: &mut ProcessContext,
) -> Result<Term, JsonTermError> {
    context
        .alloc_bigint(negative, limbs)
        .map_err(|_| JsonTermError::AllocationFailed("bigint"))
}

fn limbs_from_u128(value: u128) -> Vec<u64> {
    let low = value as u64;
    let high = (value >> u64::BITS) as u64;
    if high == 0 {
        vec![low]
    } else {
        vec![low, high]
    }
}

fn allocate_float_term(value: f64, context: &mut ProcessContext) -> Result<Term, JsonTermError> {
    context
        .alloc_float(value)
        .map_err(|_| JsonTermError::AllocationFailed("float"))
}

fn string_to_binary_term(
    string: &str,
    context: &mut ProcessContext,
) -> Result<Term, JsonTermError> {
    context
        .alloc_binary(string.as_bytes())
        .map_err(|_| JsonTermError::AllocationFailed("binary"))
}

fn array_to_list_term(
    elements: &[Value],
    context: &mut ProcessContext,
) -> Result<Term, JsonTermError> {
    // AR-1 site 11. The threaded `tail` was a boxed cons held live across
    // `value_to_term`, which allocates. The accumulator holds the elements in
    // the process native root stack instead and builds the list once, at the
    // end, so nothing is carried across an allocating call.
    //
    // ⚠️ The iteration direction FLIPS — the old body walked `.rev()` and
    // prepended, this one walks forward and appends. The resulting list is in
    // the same order; what changes is the order the elements are ALLOCATED in.
    // That is a heap-layout difference, not a term difference.
    //
    // `with_accumulator`'s error channel is `Term`, but this module's is
    // `JsonTermError`, so a failure from `value_to_term` is parked here and
    // re-raised after the closure rather than being flattened into a generic
    // allocation failure that would name the wrong thing.
    let mut parked: Option<JsonTermError> = None;
    let built = context.with_accumulator(|context, terms| {
        for value in elements {
            let head = match value_to_term(value, context) {
                Ok(head) => head,
                Err(error) => {
                    parked = Some(error);
                    return Err(Term::NIL);
                }
            };
            terms.push(context, head)?;
        }
        terms.to_list(context)
    });
    match built {
        Ok(term) => Ok(term),
        Err(_) => Err(parked.unwrap_or(JsonTermError::AllocationFailed("cons"))),
    }
}

fn object_to_map_term(
    object: &JsonObject<String, Value>,
    context: &mut ProcessContext,
) -> Result<Term, JsonTermError> {
    let mut pairs = Vec::with_capacity(object.len());
    for (key, value) in object {
        let key_term = string_to_binary_term(key, context)?;
        let value_term = value_to_term(value, context)?;
        pairs.push((key_term, value_term));
    }
    pairs.sort_by_key(|(key, _)| *key);

    let keys = pairs.iter().map(|(key, _)| *key).collect::<Vec<_>>();
    let values = pairs.iter().map(|(_, value)| *value).collect::<Vec<_>>();
    context
        .alloc_map(&keys, &values)
        .map_err(|_| JsonTermError::AllocationFailed("map"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::process::Process;
    use crate::term::boxed::{write_bigint, write_cons, write_float, write_map, write_tuple};

    fn atom_table() -> AtomTable {
        AtomTable::with_common_atoms()
    }

    fn context() -> (Arc<AtomTable>, Process) {
        (
            Arc::new(AtomTable::with_common_atoms()),
            Process::new(42, 512),
        )
    }

    fn attach_context<'process>(
        table: &Arc<AtomTable>,
        process: &'process mut Process,
    ) -> ProcessContext<'process> {
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(table)));
        context.attach_process(process, 0);
        context
    }

    fn binary_term(process: &mut Process, bytes: &[u8]) -> Term {
        let table = Arc::new(AtomTable::with_common_atoms());
        let mut context = attach_context(&table, process);
        context
            .alloc_binary(bytes)
            .expect("test binary allocation should fit")
    }

    #[test]
    fn term_to_value_converts_immediates() {
        let table = atom_table();

        assert_eq!(term_to_value(Term::small_int(42), &table), Ok(json!(42)));
        assert_eq!(
            term_to_value(Term::atom(Atom::TRUE), &table),
            Ok(json!(true))
        );
        assert_eq!(
            term_to_value(Term::atom(Atom::FALSE), &table),
            Ok(json!(false))
        );
        assert_eq!(
            term_to_value(Term::atom(Atom::NIL), &table),
            Ok(Value::Null)
        );
        assert_eq!(
            term_to_value(Term::atom(Atom::UNDEFINED), &table),
            Ok(Value::Null)
        );
        assert_eq!(term_to_value(Term::atom(Atom::OK), &table), Ok(json!("ok")));
        assert_eq!(term_to_value(Term::NIL, &table), Ok(json!([])));
        assert_eq!(term_to_value(Term::pid(7), &table), Ok(json!("<0.7.0>")));
    }

    #[test]
    fn term_to_value_handles_unknown_atoms_without_panicking() {
        let table = atom_table();

        assert_eq!(
            term_to_value(Term::atom(Atom::new(999_999)), &table),
            Err(JsonTermError::UnknownAtom(Atom::new(999_999)))
        );
    }

    #[test]
    fn term_to_value_converts_binaries() {
        let table = atom_table();
        let mut process = Process::new(7, 64);

        assert_eq!(
            term_to_value(binary_term(&mut process, b"hello"), &table),
            Ok(json!("hello"))
        );
        assert_eq!(
            term_to_value(binary_term(&mut process, &[0xff, 0x00]), &table),
            Ok(json!("/wA="))
        );
    }

    #[test]
    fn term_to_value_converts_tuple_list_map_float_and_bigint() {
        let table = atom_table();
        let mut process = Process::new(8, 64);
        let mut tuple_heap = [0_u64; 3];
        let tuple = write_tuple(
            &mut tuple_heap,
            &[Term::atom(Atom::OK), Term::small_int(42)],
        )
        .expect("tuple should fit");
        assert_eq!(term_to_value(tuple, &table), Ok(json!(["ok", 42])));

        let mut second_cell = [0_u64; 2];
        let mut first_cell = [0_u64; 2];
        let second = write_cons(&mut second_cell, Term::small_int(2), Term::NIL)
            .expect("second cons should fit");
        let list =
            write_cons(&mut first_cell, Term::small_int(1), second).expect("first cons should fit");
        assert_eq!(term_to_value(list, &table), Ok(json!([1, 2])));

        let keys = [Term::atom(Atom::OK)];
        let values = [binary_term(&mut process, b"value")];
        let mut map_heap = [0_u64; 4];
        let map = write_map(&mut map_heap, &keys, &values).expect("map should fit");
        assert_eq!(term_to_value(map, &table), Ok(json!({"ok": "value"})));

        let mut float_heap = [0_u64; 2];
        let float = write_float(&mut float_heap, 1.5).expect("float should fit");
        assert_eq!(term_to_value(float, &table), Ok(json!(1.5)));

        let mut bigint_heap = [0_u64; 4];
        let bigint = write_bigint(&mut bigint_heap, false, &[Term::SMALL_INT_MAX as u64 + 1])
            .expect("bigint should fit");
        assert_eq!(
            term_to_value(bigint, &table),
            Ok(json!(Term::SMALL_INT_MAX + 1))
        );
    }

    #[test]
    fn term_to_value_converts_nested_structures_recursively() {
        let table = atom_table();
        let mut tuple_heap = [0_u64; 3];
        let tuple = write_tuple(
            &mut tuple_heap,
            &[Term::atom(Atom::OK), Term::small_int(42)],
        )
        .expect("tuple should fit");
        let keys = [Term::atom(Atom::INFO)];
        let values = [tuple];
        let mut map_heap = [0_u64; 4];
        let map = write_map(&mut map_heap, &keys, &values).expect("map should fit");

        assert_eq!(term_to_value(map, &table), Ok(json!({"info": ["ok", 42]})));
    }

    #[test]
    fn value_to_term_converts_json_scalars() {
        let (table, mut process) = context();
        let mut context = attach_context(&table, &mut process);

        assert_eq!(
            value_to_term(&json!(42), &mut context),
            Ok(Term::small_int(42))
        );
        let null_term = value_to_term(&Value::Null, &mut context).expect("null");
        assert!(null_term.is_atom());
        assert_eq!(table.resolve(null_term.as_atom().unwrap()), Some("null"));
        assert_eq!(
            value_to_term(&json!(true), &mut context),
            Ok(Term::atom(Atom::TRUE))
        );
        assert_eq!(
            value_to_term(&json!(false), &mut context),
            Ok(Term::atom(Atom::FALSE))
        );

        let binary = value_to_term(&json!("hello"), &mut context).expect("string to binary");
        assert_eq!(term_to_value(binary, &table), Ok(json!("hello")));

        let float = value_to_term(&json!(1.25), &mut context).expect("float to term");
        assert_eq!(term_to_value(float, &table), Ok(json!(1.25)));
    }

    #[test]
    fn value_to_term_converts_arrays_to_proper_lists() {
        let (table, mut process) = context();
        let mut context = attach_context(&table, &mut process);
        let term = value_to_term(&json!([1, 2, 3]), &mut context).expect("array to list");

        assert_eq!(term_to_value(term, &table), Ok(json!([1, 2, 3])));
        let first = Cons::new(term).expect("first cons");
        let second = Cons::new(first.tail()).expect("second cons");
        let third = Cons::new(second.tail()).expect("third cons");
        assert_eq!(first.head(), Term::small_int(1));
        assert_eq!(second.head(), Term::small_int(2));
        assert_eq!(third.head(), Term::small_int(3));
        assert_eq!(third.tail(), Term::NIL);
    }

    #[test]
    fn value_to_term_converts_objects_to_binary_keyed_maps() {
        let (table, mut process) = context();
        let mut context = attach_context(&table, &mut process);
        let term = value_to_term(&json!({"key": "value"}), &mut context).expect("object to map");
        let map = Map::new(term).expect("map accessor");
        let key = map.key(0).expect("first key");
        let key_binary = crate::term::binary::Binary::new(key).expect("key is a binary");
        assert_eq!(key_binary.as_bytes(), b"key");
    }

    #[test]
    fn map_atom_keys_use_atom_names_even_for_json_special_atoms() {
        let table = atom_table();
        let keys = [Term::atom(Atom::TRUE), Term::atom(Atom::NIL)];
        let values = [Term::small_int(1), Term::small_int(2)];
        let mut map_heap = [0_u64; 6];
        let map = write_map(&mut map_heap, &keys, &values).expect("map should fit");

        assert_eq!(term_to_value(map, &table), Ok(json!({"true": 1, "nil": 2})));
    }

    #[test]
    fn round_trip_preserves_object_keys_named_like_special_atoms() {
        let (table, mut process) = context();
        let mut context = attach_context(&table, &mut process);
        let value = json!({"true": "bool-name", "nil": "nil-name"});
        let term = value_to_term(&value, &mut context).expect("object to term");

        assert_eq!(term_to_value(term, &table), Ok(value));
    }

    #[test]
    fn value_to_term_requires_atom_table_for_null() {
        let mut context = ProcessContext::new();

        assert_eq!(
            value_to_term(&Value::Null, &mut context),
            Err(JsonTermError::MissingAtomTable)
        );
    }

    #[test]
    fn value_to_term_objects_work_without_atom_table() {
        let mut process = Process::new(43, 128);
        let mut context = ProcessContext::new();
        context.attach_process(&mut process, 0);
        let term = value_to_term(&json!({"key": "value"}), &mut context);
        assert!(term.is_ok());
    }

    #[test]
    fn round_trip_preserves_representable_json_shapes() {
        let (table, mut process) = context();
        let mut context = attach_context(&table, &mut process);
        let values = [
            json!(true),
            json!(false),
            json!(42),
            json!(1.25),
            json!("hello"),
            json!([1, "two", true]),
            json!({"key": "value", "nested": [1, 2]}),
        ];

        for value in values {
            let term = value_to_term(&value, &mut context).expect("value to term");
            assert_eq!(term_to_value(term, &table), Ok(value));
        }
    }

    #[test]
    fn null_round_trips_as_null_atom() {
        let (table, mut process) = context();
        let mut context = attach_context(&table, &mut process);
        let term = value_to_term(&Value::Null, &mut context).expect("null to atom");

        assert!(term.is_atom());
        assert_eq!(table.resolve(term.as_atom().unwrap()), Some("null"));
        assert_eq!(term_to_value(term, &table), Ok(Value::Null));
    }
}

#[cfg(test)]
mod ar1_row4_json_tests {
    // ⛔ DEFECT-ASSERTING TESTS — READ THIS BEFORE TRUSTING A GREEN.
    //
    // These pin the MEASURED CORRUPT SURFACE of AR-1 row 4 at f993280. They do
    // NOT assert correct behaviour, so a green here means "the defect is still
    // present, exactly as measured" — never "this site is safe".
    //
    // ⇒ THEY GO RED WHEN AR-1 IS FIXED, AND THAT IS THE POINT. The fix lane
    // INVERTS them to assert correctness rather than deleting them; the pinned
    // counts below are the surface the fix has to move.

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use serde_json::{Map as JsonMap, Value};

    use super::value_to_term;
    use crate::atom::AtomTable;
    use crate::native::ProcessContext;
    use crate::process::Process;
    use crate::term::Term;
    use crate::term::binary::Binary;
    use crate::term::boxed::{Cons, Map};

    const WIDTH: usize = 12;

    /// Which body the cell drives.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Arm {
        Fixed,
        UnrootedReplica,
    }

    /// ⛔⛔ THE SYNTHETIC POSITIVE — `array_to_list_term`'s body EXACTLY AS IT
    /// WAS BEFORE THE FIX, and it must stay that way: the threaded `tail` held
    /// live across `value_to_term`, which allocates.
    /// ⛔ Do NOT migrate it onto the accumulator.
    fn array_to_list_term_unrooted_replica(
        elements: &[Value],
        context: &mut ProcessContext,
    ) -> Result<Term, super::JsonTermError> {
        let mut tail = Term::NIL;
        for value in elements.iter().rev() {
            let head = value_to_term(value, context)?;
            tail = context
                .alloc_cons(head, tail)
                .map_err(|_| super::JsonTermError::AllocationFailed("cons"))?;
        }
        Ok(tail)
    }

    fn key_of(index: usize) -> String {
        format!("k{index:0WIDTH$}")
    }

    fn value_of(index: usize) -> String {
        format!("v{index:0WIDTH$}")
    }

    fn element_of(index: usize) -> String {
        format!("e{index:0WIDTH$}")
    }

    /// An array of `count` heap-allocated string elements. Never immediates: an
    /// immediate needs no allocation, so the carrier would never be live across
    /// one and the probe would be structurally incapable of failing.
    fn array_of(count: usize) -> Value {
        Value::Array(
            (0..count)
                .map(|index| Value::String(element_of(index)))
                .collect(),
        )
    }

    /// An object of `count` string-keyed, string-valued entries — both halves
    /// heap-allocated, which is what puts site 15's `(Term, Term)` carrier at
    /// risk on both sides of the pair.
    fn object_of(count: usize) -> Value {
        let mut map = JsonMap::new();
        for index in 0..count {
            map.insert(key_of(index), Value::String(value_of(index)));
        }
        Value::Object(map)
    }

    fn attach<'process>(
        table: &Arc<AtomTable>,
        process: &'process mut Process,
    ) -> ProcessContext<'process> {
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(table)));
        context.attach_process(process, 0);
        context
    }

    /// Build the array on a heap of exactly `heap` words and read it back
    /// iteratively. `Err` names the first position that is not what went in.
    fn array_round_trip(count: usize, heap: usize, arm: Arm) -> Result<(), String> {
        let table = Arc::new(AtomTable::with_common_atoms());
        let mut process = Process::new(42, heap);
        let mut context = attach(&table, &mut process);

        let value = array_of(count);
        let term = match arm {
            Arm::Fixed => value_to_term(&value, &mut context),
            Arm::UnrootedReplica => {
                // Drive the replica with the SAME elements the BIF would hand
                // its own loop, so the allocation sequence matches.
                let Value::Array(elements) = &value else {
                    unreachable!("array_of builds an array")
                };
                array_to_list_term_unrooted_replica(elements, &mut context)
            }
        }
        .map_err(|error| format!("construction refused: {error}"))?;

        let mut seen = 0usize;
        let mut tail = term;
        // HARD CAP: a stale tail can make the list cyclic. Without this the
        // reader spins forever instead of reporting.
        let cap = count * 2 + 16;
        while !tail.is_nil() {
            if seen > cap {
                return Err(format!(
                    "list did not terminate within {cap} cells — cyclic tail"
                ));
            }
            let cons = Cons::new(tail).ok_or_else(|| {
                format!("element {seen}: tail is not a cons — carrier went stale")
            })?;
            let binary = Binary::new(cons.head()).ok_or_else(|| {
                format!("element {seen}: head is not a binary — carrier went stale")
            })?;
            let want = element_of(seen);
            if binary.as_bytes() != want.as_bytes() {
                return Err(format!(
                    "element {seen}: contents {:?} != {want:?}",
                    String::from_utf8_lossy(binary.as_bytes())
                ));
            }
            seen += 1;
            tail = cons.tail();
        }
        if seen != count {
            return Err(format!("recovered {seen} elements, put {count}"));
        }
        Ok(())
    }

    /// Build the object on a heap of exactly `heap` words and read it back as a
    /// SET of key→value pairs (see the header note on sort-by-bit-pattern).
    fn object_round_trip(count: usize, heap: usize) -> Result<(), String> {
        let table = Arc::new(AtomTable::with_common_atoms());
        let mut process = Process::new(42, heap);
        let mut context = attach(&table, &mut process);

        let term = value_to_term(&object_of(count), &mut context)
            .map_err(|error| format!("construction refused: {error}"))?;

        let map =
            Map::new(term).ok_or_else(|| "result is not a map — carrier went stale".to_string())?;
        if map.len() != count {
            return Err(format!("map holds {} entries, put {count}", map.len()));
        }

        let mut recovered: BTreeMap<String, String> = BTreeMap::new();
        for index in 0..map.len() {
            let key = read_binary(map.key(index), index, "key")?;
            let value = read_binary(map.value(index), index, "value")?;
            recovered.insert(key, value);
        }

        for index in 0..count {
            let want_key = key_of(index);
            let want_value = value_of(index);
            match recovered.get(&want_key) {
                None => {
                    return Err(format!(
                        "entry {index}: key {want_key:?} absent from result"
                    ));
                }
                Some(got) if got != &want_value => {
                    return Err(format!(
                        "entry {index} ({want_key:?}): {got:?} != {want_value:?}"
                    ));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    fn read_binary(term: Option<Term>, index: usize, half: &str) -> Result<String, String> {
        let term = term.ok_or_else(|| format!("entry {index}: {half} slot absent"))?;
        let binary = Binary::new(term)
            .ok_or_else(|| format!("entry {index}: {half} is not a binary — carrier went stale"))?;
        Ok(String::from_utf8_lossy(binary.as_bytes()).into_owned())
    }

    /// AR-1 row 4, site 11 (`tail` in `array_to_list_term`) — ✅ INVERTED.
    ///
    /// TWO-ARMED IN BOTH DIRECTIONS on the control, which is what makes a
    /// failure attributable to the collection rather than to an allocator limit:
    /// hold the heap and grow the input, then hold the input and grow the heap.
    /// The fixed body is then measured over the same three cells.
    #[test]
    fn ar1_site11_array_to_list_term_two_armed() {
        const HEAP: usize = 4096;
        const BIG: usize = 2000;

        // ⛔⛔ POSITIVE CONTROL FIRST, and it licenses everything below it.
        let control = array_round_trip(50, HEAP, Arm::UnrootedReplica);
        assert!(
            control.is_ok(),
            "control arm: 50 elements on a {HEAP}-word heap must round-trip, got {control:?}. \
             A failure here would be an allocator limit and the red arm would prove nothing."
        );

        let red = array_round_trip(BIG, HEAP, Arm::UnrootedReplica);
        assert!(
            red.is_err(),
            "POSITIVE CONTROL DEAD: the unrooted replica no longer corrupts at {BIG} elements on \
             a {HEAP}-word heap (got {red:?}). The pressure regime is gone, so the fixed arm's \
             success below would mean nothing."
        );
        let reason = red.unwrap_err();
        assert!(
            !reason.contains("construction refused"),
            "POSITIVE CONTROL IS A REFUSAL, NOT CORRUPTION: {reason}. A refusal is evidence of \
             nothing about rooting — it is the exact ambiguity this arm exists to rule out."
        );

        // SECOND DIRECTION: same input, a heap large enough that nothing has to
        // collect. If this also failed, the input would simply be too big.
        let roomy = array_round_trip(BIG, 1 << 20, Arm::UnrootedReplica);
        assert!(
            roomy.is_ok(),
            "control second direction: {BIG} elements on a roomy heap must be clean, got {roomy:?}"
        );
        eprintln!("site 11 CONTROL still red: {reason}");

        // ✅ THE CLAIM. Same cells, same heaps, through the rooted body.
        for (count, heap) in [(50usize, HEAP), (BIG, HEAP), (BIG, 1usize << 20)] {
            let fixed = array_round_trip(count, heap, Arm::Fixed);
            assert!(
                fixed.is_ok(),
                "site 11 is NOT rooted: {count} elements on a {heap}-word heap lost the carrier, \
                 got {fixed:?}, while the replica corrupted in the same run"
            );
        }
    }

    /// AR-1 row 4, site 15 (`pairs` in `object_to_map_term`).
    #[test]
    fn ar1_site15_object_to_map_term_two_armed() {
        // MEASURED, not guessed. At heap 4096 the 500-entry case is clean and
        // the 2000-entry case is REFUSED by the allocator — refusal is
        // ambiguous by construction and proves nothing (Cally's site-17
        // warning). The admissible cell is the one where construction SUCCEEDS
        // and the result is still wrong: heap 256, 100 entries.
        const HEAP: usize = 256;
        const BIG: usize = 100;

        let control = object_round_trip(10, HEAP);
        assert!(
            control.is_ok(),
            "control arm: 10 entries on a {HEAP}-word heap must round-trip, got {control:?}."
        );

        let red = object_round_trip(BIG, HEAP);
        assert!(
            red.is_err(),
            "site 15 red-at-parent: {BIG} entries on a {HEAP}-word heap must corrupt the \
             carrier, got {red:?}"
        );

        let roomy = object_round_trip(BIG, 1 << 20);
        assert!(
            roomy.is_ok(),
            "site 15 second direction: {BIG} entries on a roomy heap must be clean, got {roomy:?}"
        );

        let reason = red.unwrap_err();
        println!("site 15 RED: {reason}");
        eprintln!("site 15 RED: {reason}");
    }

    /// The surface both verdicts are read off. Each cell is emitted AS IT RUNS
    /// and to BOTH streams: if a cell kills the process, the last line printed
    /// names the cell that did it, and a swallowed stdout cannot turn a measured
    /// result into "exited with no output".
    #[test]
    fn ar1_sites_11_15_sweep_surface() {
        let heaps = [256usize, 1024, 4096, 16384, 65536];
        let sizes = [10usize, 100, 500, 2000];
        for heap in heaps {
            for count in sizes {
                // Arrays are reported on BOTH arms now that site 11 is rooted:
                // the surface is only readable if the control's shape is beside
                // the fixed one. Objects stay single-armed until site 15 lands.
                for shape in ["array-control", "array-fixed", "object"] {
                    let outcome = match shape {
                        "array-control" => array_round_trip(count, heap, Arm::UnrootedReplica),
                        "array-fixed" => array_round_trip(count, heap, Arm::Fixed),
                        _ => object_round_trip(count, heap),
                    };
                    let verdict = match outcome {
                        Ok(()) => "ok".to_string(),
                        Err(reason) => reason,
                    };
                    let line = format!("heap {heap:>6} x {shape:<13} {count:>5} : {verdict}");
                    println!("{line}");
                    eprintln!("{line}");
                }
            }
        }
    }
}
