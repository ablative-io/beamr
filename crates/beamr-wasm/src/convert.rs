//! JavaScript value conversion for the WASM host boundary.
//!
//! This module deliberately converts by value. Non-UTF-8 binaries are copied
//! into `Uint8Array` instances and JavaScript objects are traversed into BEAM
//! maps rather than wrapped as opaque host references.

use std::sync::Arc;

use beamr::atom::{Atom, AtomTable};
use beamr::ets::OwnedTerm;
use beamr::native::ProcessContext;
use beamr::term::binary::Binary;
use beamr::term::boxed::{Cons, Float, Map, Tuple};
use beamr::term::{Tag, Term};
use js_sys::{Array, Object, Reflect, Uint8Array};
use serde_json::Value;
use wasm_bindgen::JsValue;

const MAX_CONVERSION_DEPTH: usize = 256;

/// Convert a direct JavaScript value into an owned BEAM term.
///
/// The returned [`OwnedTerm`] keeps any detached heap allocations alive until a
/// caller can copy them into the target process heap.
pub fn js_value_to_owned_term(
    value: JsValue,
    atom_table: &Arc<AtomTable>,
) -> Result<OwnedTerm, JsValue> {
    let mut context = ProcessContext::new();
    context.set_atom_table(Some(Arc::clone(atom_table)));
    let term = js_value_to_term_in_context(value, &mut context)?;
    Ok(context
        .take_detached_result(term)
        .unwrap_or_else(|| OwnedTerm::immediate(term)))
}

/// Convert a direct JavaScript value into a BEAM term allocated in `context`.
pub fn js_value_to_term_in_context(
    value: JsValue,
    context: &mut ProcessContext<'_>,
) -> Result<Term, JsValue> {
    value_to_term(value, context, 0)
}

/// Convert a JSON array into owned BEAM terms for the legacy `spawn` API.
pub fn terms_from_json_array(
    value: &Value,
    atom_table: &Arc<AtomTable>,
) -> Result<Vec<OwnedTerm>, JsValue> {
    let Value::Array(values) = value else {
        return Err(JsValue::from_str("arguments must be a JSON array"));
    };

    values
        .iter()
        .map(|value| {
            let mut context = ProcessContext::new();
            context.set_atom_table(Some(Arc::clone(atom_table)));
            let term = json_value_to_term(value, &mut context, 0)?;
            Ok(context
                .take_detached_result(term)
                .unwrap_or_else(|| OwnedTerm::immediate(term)))
        })
        .collect()
}

/// Convert BEAM terms to a JavaScript array of direct host values.
pub fn terms_to_js_array(args: &[Term], atom_table: &AtomTable) -> Result<JsValue, JsValue> {
    let array = Array::new();
    for term in args {
        array.push(&term_to_js_value(*term, atom_table)?);
    }
    Ok(array.into())
}

/// Convert a BEAM term into a JavaScript value.
pub fn term_to_js_value(term: Term, atom_table: &AtomTable) -> Result<JsValue, JsValue> {
    term_to_js_value_at_depth(term, atom_table, 0)
}

fn value_to_term(
    value: JsValue,
    context: &mut ProcessContext<'_>,
    depth: usize,
) -> Result<Term, JsValue> {
    check_depth(depth)?;

    if value.is_null() {
        return Ok(Term::atom(Atom::NIL));
    }
    if value.is_undefined() {
        return Err(JsValue::from_str(
            "cannot convert JavaScript undefined to a BEAM term",
        ));
    }
    if let Some(boolean) = value.as_bool() {
        return Ok(Term::atom(if boolean { Atom::TRUE } else { Atom::FALSE }));
    }
    if let Some(number) = value.as_f64() {
        return number_to_term(number, context);
    }
    if let Some(string) = value.as_string() {
        return context
            .alloc_binary(string.as_bytes())
            .map_err(|_| JsValue::from_str("failed to allocate binary term"));
    }
    if Array::is_array(&value) {
        return array_to_term(&Array::from(&value), context, depth + 1);
    }
    if value.is_object() {
        return object_to_term(value, context, depth + 1);
    }

    Err(JsValue::from_str(
        "unsupported JavaScript value for BEAM term conversion",
    ))
}

fn json_value_to_term(
    value: &Value,
    context: &mut ProcessContext<'_>,
    depth: usize,
) -> Result<Term, JsValue> {
    check_depth(depth)?;
    match value {
        Value::Null => Ok(Term::atom(Atom::NIL)),
        Value::Bool(true) => Ok(Term::atom(Atom::TRUE)),
        Value::Bool(false) => Ok(Term::atom(Atom::FALSE)),
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| JsValue::from_str("unsupported JSON number"))
            .and_then(|value| number_to_term(value, context)),
        Value::String(string) => context
            .alloc_binary(string.as_bytes())
            .map_err(|_| JsValue::from_str("failed to allocate binary term")),
        Value::Array(elements) => {
            let mut tail = Term::NIL;
            for value in elements.iter().rev() {
                let head = json_value_to_term(value, context, depth + 1)?;
                tail = context
                    .alloc_cons(head, tail)
                    .map_err(|_| JsValue::from_str("failed to allocate cons term"))?;
            }
            Ok(tail)
        }
        Value::Object(object) => {
            let mut pairs = Vec::with_capacity(object.len());
            for (key, value) in object {
                let key_term = context
                    .alloc_binary(key.as_bytes())
                    .map_err(|_| JsValue::from_str("failed to allocate map key binary"))?;
                let value_term = json_value_to_term(value, context, depth + 1)?;
                pairs.push((key_term, value_term));
            }
            alloc_sorted_map(pairs, context)
        }
    }
}

fn number_to_term(value: f64, context: &mut ProcessContext<'_>) -> Result<Term, JsValue> {
    if !value.is_finite() {
        return Err(JsValue::from_str(
            "cannot convert non-finite JavaScript number",
        ));
    }
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        let integer = value as i64;
        if let Some(term) = Term::try_small_int(integer) {
            return Ok(term);
        }
    }
    context
        .alloc_float(value)
        .map_err(|_| JsValue::from_str("failed to allocate float term"))
}

fn array_to_term(
    array: &Array,
    context: &mut ProcessContext<'_>,
    depth: usize,
) -> Result<Term, JsValue> {
    let mut tail = Term::NIL;
    for index in (0..array.length()).rev() {
        let head = value_to_term(array.get(index), context, depth)?;
        tail = context
            .alloc_cons(head, tail)
            .map_err(|_| JsValue::from_str("failed to allocate cons term"))?;
    }
    Ok(tail)
}

fn object_to_term(
    value: JsValue,
    context: &mut ProcessContext<'_>,
    depth: usize,
) -> Result<Term, JsValue> {
    let object = Object::from(value);
    let keys = Object::keys(&object);
    let mut pairs = Vec::with_capacity(keys.length() as usize);
    for index in 0..keys.length() {
        let key_value = keys.get(index);
        let key = key_value
            .as_string()
            .ok_or_else(|| JsValue::from_str("JavaScript object key was not a string"))?;
        let property = Reflect::get(&object, &key_value)?;
        let key_term = context
            .alloc_binary(key.as_bytes())
            .map_err(|_| JsValue::from_str("failed to allocate map key binary"))?;
        let value_term = value_to_term(property, context, depth)?;
        pairs.push((key_term, value_term));
    }
    alloc_sorted_map(pairs, context)
}

fn alloc_sorted_map(
    mut pairs: Vec<(Term, Term)>,
    context: &mut ProcessContext<'_>,
) -> Result<Term, JsValue> {
    pairs.sort_by_key(|(key, _)| *key);
    let keys = pairs.iter().map(|(key, _)| *key).collect::<Vec<_>>();
    let values = pairs.iter().map(|(_, value)| *value).collect::<Vec<_>>();
    context
        .alloc_map(&keys, &values)
        .map_err(|_| JsValue::from_str("failed to allocate map term"))
}

fn term_to_js_value_at_depth(
    term: Term,
    atom_table: &AtomTable,
    depth: usize,
) -> Result<JsValue, JsValue> {
    check_depth(depth)?;
    match term.tag() {
        Tag::SmallInt => term
            .as_small_int()
            .map(|value| JsValue::from_f64(value as f64))
            .ok_or_else(|| JsValue::from_str("invalid small integer term")),
        Tag::Atom => atom_to_js_value(term, atom_table),
        Tag::Nil => Ok(Array::new().into()),
        Tag::List => list_to_js_value(term, atom_table, depth + 1),
        Tag::Boxed => boxed_to_js_value(term, atom_table, depth + 1),
        Tag::Pid => Err(JsValue::from_str(
            "cannot convert pid term to JavaScript value",
        )),
    }
}

fn atom_to_js_value(term: Term, atom_table: &AtomTable) -> Result<JsValue, JsValue> {
    let atom = term
        .as_atom()
        .ok_or_else(|| JsValue::from_str("invalid atom term"))?;
    let name = atom_table
        .resolve(atom)
        .ok_or_else(|| JsValue::from_str("atom is not present in the atom table"))?;
    Ok(JsValue::from_str(name))
}

fn list_to_js_value(term: Term, atom_table: &AtomTable, depth: usize) -> Result<JsValue, JsValue> {
    let array = Array::new();
    let mut tail = term;
    loop {
        if tail.is_nil() {
            return Ok(array.into());
        }
        let cons = Cons::new(tail)
            .ok_or_else(|| JsValue::from_str("cannot convert improper list to JavaScript array"))?;
        array.push(&term_to_js_value_at_depth(cons.head(), atom_table, depth)?);
        tail = cons.tail();
    }
}

fn boxed_to_js_value(term: Term, atom_table: &AtomTable, depth: usize) -> Result<JsValue, JsValue> {
    if let Some(binary) = Binary::new(term) {
        return binary_to_js_value(binary);
    }
    if let Some(tuple) = Tuple::new(term) {
        return tuple_to_js_value(tuple, atom_table, depth);
    }
    if let Some(map) = Map::new(term) {
        return map_to_js_value(map, atom_table, depth);
    }
    if let Some(float) = Float::new(term) {
        return Ok(JsValue::from_f64(float.value()));
    }
    Err(JsValue::from_str(
        "unsupported boxed term for JavaScript conversion",
    ))
}

fn binary_to_js_value(binary: Binary) -> Result<JsValue, JsValue> {
    match std::str::from_utf8(binary.as_bytes()) {
        Ok(text) => Ok(JsValue::from_str(text)),
        Err(_) => {
            let length = u32::try_from(binary.len())
                .map_err(|_| JsValue::from_str("binary is too large for Uint8Array"))?;
            let array = Uint8Array::new_with_length(length);
            array.copy_from(binary.as_bytes());
            Ok(array.into())
        }
    }
}

fn tuple_to_js_value(
    tuple: Tuple,
    atom_table: &AtomTable,
    depth: usize,
) -> Result<JsValue, JsValue> {
    let array = Array::new();
    for index in 0..tuple.arity() {
        let element = tuple
            .get(index)
            .ok_or_else(|| JsValue::from_str("invalid tuple element"))?;
        array.push(&term_to_js_value_at_depth(element, atom_table, depth)?);
    }
    Ok(array.into())
}

fn map_to_js_value(map: Map, atom_table: &AtomTable, depth: usize) -> Result<JsValue, JsValue> {
    let object = Object::new();
    for index in 0..map.len() {
        let key = map
            .key(index)
            .ok_or_else(|| JsValue::from_str("invalid map key"))?;
        let key_name = map_key_to_string(key, atom_table)?;
        let value = map
            .value(index)
            .ok_or_else(|| JsValue::from_str("invalid map value"))?;
        Reflect::set(
            &object,
            &JsValue::from_str(&key_name),
            &term_to_js_value_at_depth(value, atom_table, depth)?,
        )?;
    }
    Ok(object.into())
}

fn map_key_to_string(term: Term, atom_table: &AtomTable) -> Result<String, JsValue> {
    if let Some(atom) = term.as_atom() {
        return atom_table
            .resolve(atom)
            .map(str::to_owned)
            .ok_or_else(|| JsValue::from_str("map atom key is not present in the atom table"));
    }
    if let Some(binary) = Binary::new(term) {
        return std::str::from_utf8(binary.as_bytes())
            .map(str::to_owned)
            .map_err(|_| JsValue::from_str("map binary key is not valid UTF-8"));
    }
    Err(JsValue::from_str(
        "map key cannot be converted to a JavaScript property name",
    ))
}

fn check_depth(depth: usize) -> Result<(), JsValue> {
    if depth > MAX_CONVERSION_DEPTH {
        Err(JsValue::from_str(
            "JavaScript/Term conversion exceeded maximum depth",
        ))
    } else {
        Ok(())
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn atom_table() -> Arc<AtomTable> {
        Arc::new(AtomTable::with_common_atoms())
    }

    fn binary_context_key(context: &mut ProcessContext<'_>, text: &str) -> Term {
        context
            .alloc_binary(text.as_bytes())
            .expect("test key binary allocation succeeds")
    }

    fn list_to_vec(mut term: Term) -> Vec<Term> {
        let mut values = Vec::new();
        while !term.is_nil() {
            let cons = Cons::new(term).expect("converted JavaScript array is a proper list");
            values.push(cons.head());
            term = cons.tail();
        }
        values
    }

    #[wasm_bindgen_test]
    fn converts_complex_nested_js_object_to_term() {
        let table = atom_table();
        let input = Object::new();
        let nested = Object::new();
        let array = Array::new();
        array.push(&JsValue::from_f64(1.0));
        array.push(&JsValue::from_bool(true));
        assert!(Reflect::set(&nested, &JsValue::from_str("items"), &array).is_ok());
        assert!(Reflect::set(&nested, &JsValue::from_str("missing"), &JsValue::NULL).is_ok());
        assert!(
            Reflect::set(
                &input,
                &JsValue::from_str("name"),
                &JsValue::from_str("beamr")
            )
            .is_ok()
        );
        assert!(Reflect::set(&input, &JsValue::from_str("nested"), &nested).is_ok());

        let owned = js_value_to_owned_term(input.into(), &table)
            .expect("complex JavaScript object converts to an owned term");
        let term = owned.root();
        let map = Map::new(term).expect("top-level object converts to map");
        assert_eq!(map.len(), 2);

        let mut key_context = ProcessContext::new();
        let name = map
            .get(binary_context_key(&mut key_context, "name"))
            .expect("name key is present");
        let name_binary = Binary::new(name).expect("string value converts to binary");
        assert_eq!(name_binary.as_bytes(), b"beamr");

        let nested = map
            .get(binary_context_key(&mut key_context, "nested"))
            .expect("nested key is present");
        let nested_map = Map::new(nested).expect("nested object converts to map");
        assert_eq!(nested_map.len(), 2);
        assert_eq!(
            nested_map.get(binary_context_key(&mut key_context, "missing")),
            Some(Term::atom(Atom::NIL))
        );

        let items = nested_map
            .get(binary_context_key(&mut key_context, "items"))
            .expect("array-valued key is present");
        let items = list_to_vec(items);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], Term::small_int(1));
        assert_eq!(items[1], Term::atom(Atom::TRUE));
    }

    #[wasm_bindgen_test]
    fn converts_terms_to_js_values() {
        let table = atom_table();
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        let utf8 = context
            .alloc_binary("hello".as_bytes())
            .unwrap_or(Term::NIL);
        let bytes = context.alloc_binary(&[0xff, 0x00]).unwrap_or(Term::NIL);
        let list = context
            .alloc_list(&[Term::small_int(7), Term::atom(Atom::TRUE)])
            .unwrap_or(Term::NIL);
        let tuple = context
            .alloc_tuple(&[utf8, Term::small_int(9)])
            .unwrap_or(Term::NIL);
        let key = context
            .alloc_binary("tuple".as_bytes())
            .unwrap_or(Term::NIL);
        let map = context.alloc_map(&[key], &[tuple]).unwrap_or(Term::NIL);

        let utf8_js = term_to_js_value(utf8, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
        assert_eq!(utf8_js.as_string().as_deref(), Some("hello"));

        let bytes_js = term_to_js_value(bytes, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
        assert!(bytes_js.is_instance_of::<Uint8Array>());
        let bytes_array = Uint8Array::from(bytes_js);
        assert_eq!(bytes_array.length(), 2);
        assert_eq!(bytes_array.get_index(0), 0xff);
        assert_eq!(bytes_array.get_index(1), 0x00);

        let list_js = term_to_js_value(list, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
        assert!(Array::is_array(&list_js));
        let list_array = Array::from(&list_js);
        assert_eq!(list_array.length(), 2);
        assert_eq!(list_array.get(0).as_f64(), Some(7.0));
        assert_eq!(list_array.get(1).as_string().as_deref(), Some("true"));

        let tuple_js = term_to_js_value(tuple, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
        assert!(Array::is_array(&tuple_js));
        let tuple_array = Array::from(&tuple_js);
        assert_eq!(tuple_array.length(), 2);
        assert_eq!(tuple_array.get(0).as_string().as_deref(), Some("hello"));
        assert_eq!(tuple_array.get(1).as_f64(), Some(9.0));

        let map_js = term_to_js_value(map, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
        let nested_tuple_js =
            Reflect::get(&map_js, &JsValue::from_str("tuple")).unwrap_or(JsValue::UNDEFINED);
        assert!(Array::is_array(&nested_tuple_js));
    }

    #[wasm_bindgen_test]
    fn documents_boolean_atom_round_trip_as_atom_names() {
        let table = atom_table();
        let owned = js_value_to_owned_term(JsValue::from_bool(true), &table)
            .expect("boolean converts to an owned atom term");
        let term = owned.root();
        assert_eq!(term, Term::atom(Atom::TRUE));
        let js = term_to_js_value(term, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
        assert_eq!(js.as_string().as_deref(), Some("true"));
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod ar1_row4_sites_12_16_tests {
    // ⛔ DEFECT-ASSERTING TESTS — READ THIS BEFORE TRUSTING A GREEN.
    //
    // These pin the MEASURED CORRUPT SURFACE of AR-1 row 4 at f993280. They do
    // NOT assert correct behaviour, so a green here means "the defect is still
    // present, exactly as measured" — never "this site is safe".
    //
    // ⇒ THEY GO RED WHEN AR-1 IS FIXED, AND THAT IS THE POINT. The fix lane
    // INVERTS them to assert correctness rather than deleting them; the pinned
    // counts below are the surface the fix has to move.

    use super::*;
    use beamr::process::Process;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// ⛔ `println!` IS INERT UNDER THIS RUNNER — MEASURED, NOT ASSUMED. The
    /// first run printed every cell and the log contained none of them; the
    /// obvious explanation was wasm-bindgen-test capturing output for passing
    /// tests, so I re-ran with `-- --nocapture` and the lines were STILL
    /// absent, including the ones from the test that passed. That refutes the
    /// capture hypothesis and leaves the channel itself dead.
    ///
    /// So the report goes out over `console.log`, reached through `js_sys`
    /// (no new dependency — the crate already reaches host globals this way),
    /// AND is carried in the assertion message, which is the one channel
    /// already proven to survive.
    fn emit(line: &str) {
        let global = js_sys::global();
        let Ok(console) = Reflect::get(&global, &JsValue::from_str("console")) else {
            return;
        };
        let Ok(log) = Reflect::get(&console, &JsValue::from_str("log")) else {
            return;
        };
        if let Ok(log) = log.dyn_into::<js_sys::Function>() {
            let _ = log.call1(&console, &JsValue::from_str(line));
        }
    }

    const KEY_WIDTH: usize = 12;

    fn element(index: usize) -> String {
        format!("e{index:0KEY_WIDTH$}")
    }

    /// Build the JSON array driving site 12 (`Value::Array` arm, carrier `tail`).
    fn array_input(count: usize) -> Value {
        Value::Array(
            (0..count)
                .map(|index| Value::String(element(index)))
                .collect(),
        )
    }

    /// Build the JSON object driving site 16 (`Value::Object` arm, carrier `pairs`).
    fn object_input(count: usize) -> Value {
        let mut map = serde_json::Map::new();
        for index in 0..count {
            map.insert(element(index), Value::String(element(index)));
        }
        Value::Object(map)
    }

    /// Read a cons list back BY CONTENTS, iteratively and hard-capped — a stale
    /// pointer can alias an enclosing object and make the list a CYCLE, which
    /// would abort the runner before it could report.
    fn check_list(term: Term, count: usize) -> Result<(), String> {
        let mut seen = 0usize;
        let mut tail = term;
        let cap = count * 2 + 16;
        while !tail.is_nil() {
            if seen > cap {
                return Err(format!(
                    "list did not terminate within {cap} cells — cyclic tail"
                ));
            }
            let cons = Cons::new(tail).ok_or_else(|| {
                format!("element {seen}: tail is not a cons — carrier `tail` went stale")
            })?;
            let binary = Binary::new(cons.head()).ok_or_else(|| {
                format!("element {seen}: head is not a binary — carrier `tail` went stale")
            })?;
            let want = element(seen);
            if binary.as_bytes() != want.as_bytes() {
                return Err(format!(
                    "element {seen}: contents differ — carrier `tail` went stale"
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

    /// Read the map back BY CONTENTS. Keys are compared as a SET, because this
    /// site sorts by the raw `Term` bit pattern (recorded adjacent at site 15).
    fn check_map(term: Term, count: usize) -> Result<(), String> {
        let map = Map::new(term)
            .ok_or_else(|| "result is not a map — carrier `pairs` went stale".to_string())?;
        if map.len() != count {
            return Err(format!("map has {} entries, put {count}", map.len()));
        }
        let mut names = Vec::with_capacity(count);
        for index in 0..map.len() {
            let key = map
                .key(index)
                .ok_or_else(|| format!("entry {index}: key slot absent"))?;
            let key = Binary::new(key).ok_or_else(|| {
                format!("entry {index}: key is not a binary — carrier `pairs` went stale")
            })?;
            let value = map
                .value(index)
                .ok_or_else(|| format!("entry {index}: value slot absent"))?;
            let value = Binary::new(value).ok_or_else(|| {
                format!("entry {index}: value is not a binary — carrier `pairs` went stale")
            })?;
            if key.as_bytes() != value.as_bytes() {
                return Err(format!(
                    "entry {index}: key and value differ — carrier `pairs` went stale"
                ));
            }
            names.push(key.as_bytes().to_vec());
        }
        names.sort();
        let mut want: Vec<Vec<u8>> = (0..count).map(|i| element(i).into_bytes()).collect();
        want.sort();
        if names != want {
            return Err("recovered key set differs from the input key set".to_string());
        }
        Ok(())
    }

    /// ARM A — ATTACHED PROCESS. This is the POSITIVE CONTROL and the whole
    /// probe rests on it: it proves these two arms are defective IN THEMSELVES
    /// and that this instrument can see it. Without a RED here, arm B's clean
    /// result is the asleep-instrument reading and proves nothing.
    fn attached(
        input: &Value,
        heap: usize,
        margin: usize,
        count: usize,
        is_map: bool,
    ) -> (usize, Result<(), String>) {
        let table = Arc::new(AtomTable::with_common_atoms());
        let mut process = Process::new(12, heap);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        context.attach_process(&mut process, 0);

        // Unrooted pre-fill to a MEASURED margin. The loop must be able to give
        // up: below one filler allocation (~6 words) the only thing that lands
        // is a collection, which frees this unrooted filler and pushes
        // `available` back up. The achieved margin is RETURNED so a cell that
        // missed its request is reported at what it actually got.
        let mut filler = Vec::new();
        let mut last_available = usize::MAX;
        // Loop-with-value so that EVERY exit carries its witness out. An
        // allocator refusal is also a give-up, and a `break` that reported
        // nothing would put this cell's margin back into the fabricated class
        // — the exact regression the site-5 hang taught.
        let achieved = loop {
            let available = context.process_heap().map(|h| h.available()).unwrap_or(0);
            if available <= margin || available >= last_available {
                break available;
            }
            last_available = available;
            match context.alloc_binary(&[0xA1; 32]) {
                Ok(term) => filler.push(term),
                Err(_) => break available,
            }
        };

        let outcome = match json_value_to_term(input, &mut context, 0) {
            Err(_) => Err("json_value_to_term returned an error term".to_string()),
            Ok(term) => {
                if is_map {
                    check_map(term, count)
                } else {
                    check_list(term, count)
                }
            }
        };
        (achieved, outcome)
    }

    /// ARM B — DETACHED CONTEXT, which is the shape EVERY production caller
    /// builds (`terms_from_json_array` makes a fresh `ProcessContext::new()`
    /// per element and never attaches a process).
    fn detached(input: &Value, count: usize, is_map: bool) -> Result<(), String> {
        let table = Arc::new(AtomTable::with_common_atoms());
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        match json_value_to_term(input, &mut context, 0) {
            Err(_) => Err("json_value_to_term returned an error term".to_string()),
            Ok(term) => {
                if is_map {
                    check_map(term, count)
                } else {
                    check_list(term, count)
                }
            }
        }
    }

    /// The report is both EMITTED per cell and RETURNED, so a passing run is as
    /// legible as a failing one. A verdict that is only visible when the assert
    /// fires cannot tell "no corruption" from "no pressure" — the same
    /// collapsed pair as hung-vs-slow-compile.
    fn sweep(label: &str, is_map: bool) -> (usize, usize, Vec<String>) {
        let mut corrupted = 0usize;
        let mut clean = 0usize;
        let mut report = Vec::new();
        // ⚠️ COUNTS ARE PER-ARM AND DERIVED FROM A MEASUREMENT, NOT SHARED.
        // Measured at the bytes: `alloc_cons` ROOTS ITS OWN ARGUMENTS
        // (`with_rooted(&[head, tail], ...)` before `ensure_heap_space(2)`), so
        // site 12's carrier is re-rooted and forwarded on EVERY iteration and
        // its only exposure window is the single `alloc_binary` for the next
        // head. At heap 4096 with ~7 words per element, the one collection
        // fires on the FIRST allocation — when `tail` is still `Term::NIL`, an
        // immediate with nothing to go stale — and the remaining 200 elements
        // then fit with no further collection. That is why the first sweep was
        // clean at all 24 cells, and it is a property of the INPUT SIZE, not of
        // the site. The list must be too long to fit in one post-collection
        // nursery: 4096 words / ~7 per element ⇒ a mid-list collection needs
        // more than ~580 elements.
        //
        // Site 16 needs no such correction: `pairs` accumulates unrooted across
        // the WHOLE loop and the collection lands at `alloc_sorted_map`, which
        // is why its reds name the LAST entries (48, 49).
        let counts: &[usize] = if is_map {
            &[50, 200]
        } else {
            &[50, 200, 1000, 2000]
        };
        for &count in counts {
            let input = if is_map {
                object_input(count)
            } else {
                array_input(count)
            };
            for margin in [0usize, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 4096] {
                let (achieved, result) = attached(&input, 4096, margin, count, is_map);
                let verdict = match result {
                    Ok(()) => "ok".to_string(),
                    Err(reason) => reason,
                };
                // BOTH margins printed: where achieved != requested the cell is
                // not evidence about the requested margin, and cells sharing an
                // achieved margin are ONE measurement, not two.
                let line = format!(
                    "{label} ATTACHED count {count:>4} margin req {margin:>5} got {achieved:>5} : {verdict}"
                );
                emit(&line);
                report.push(line);
                if verdict == "ok" {
                    clean += 1;
                } else if !verdict.contains("returned an error term") {
                    corrupted += 1;
                }
            }
        }
        (corrupted, clean, report)
    }

    /// Sites 12 + 16, arm A — **tier 1 of the two-tier verdict: the code IS
    /// defective.** MUST go RED, or the pair is UNRESOLVED, not defended.
    #[wasm_bindgen_test]
    fn ar1_sites_12_16_tier1_defective_red_with_a_process_attached() {
        let (list_red, list_ok, list_report) = sweep("site 12 (tail)", false);
        let (map_red, map_ok, map_report) = sweep("site 16 (pairs)", true);
        emit(&format!(
            "site 12: {list_red} corruption cells, {list_ok} clean cells"
        ));
        emit(&format!(
            "site 16: {map_red} corruption cells, {map_ok} clean cells"
        ));
        let report = format!(
            "site 12: {list_red} corruption cells, {list_ok} clean cells\n\
             site 16: {map_red} corruption cells, {map_ok} clean cells\n{}\n{}",
            list_report.join("\n"),
            map_report.join("\n")
        );

        assert!(
            list_ok > 0 && map_ok > 0,
            "control: some cell must be clean, or the probe never worked at all\n{report}"
        );
        assert!(
            list_red > 0,
            "site 12: no cell corrupted `tail` with a process ATTACHED. Under the site-14 law \
             that is UNRESOLVED, not defended — and it would void the detached arm, whose whole \
             meaning is that it is clean FOR A DIFFERENT REASON.\n{report}"
        );
        assert!(
            map_red > 0,
            "site 16: no cell corrupted `pairs` with a process ATTACHED. Same reading as \
             site 12.\n{report}"
        );

        // ⭐ PRE-REGISTERED EXACT COUNTS. The runner emits per-test output ONLY
        // for failures — measured three ways: `println!` absent, `println!`
        // under `-- --nocapture` still absent, `console.log` via js_sys also
        // absent; only a panic payload survives. So a bare `> 0` assertion
        // would PASS SILENTLY and the band would be invisible on every green
        // run. Pinning the counts makes any drift print the whole surface.
        assert_eq!(
            (list_red, list_ok, map_red, map_ok),
            (21, 27, 16, 8),
            "band drifted from the measured surface\n{report}"
        );
    }

    /// Sites 12 + 16, arm B — **tier 2 of the two-tier verdict: the production
    /// path CANNOT REACH the defect.** Expected CLEAN, and clean for a named
    /// structural reason rather than a lucky one: the sole caller of
    /// `json_value_to_term` is `terms_from_json_array`, which builds a
    /// `ProcessContext::new()` per element and never attaches a process; a
    /// detached context pushes a fresh `Box<[u64]>` per allocation that is
    /// never moved, never freed and never collected, and `ensure_heap_space`
    /// is a no-op returning `Ok`. **The defence is the CALLER'S, not the
    /// site's** — so this green may not be read as "site 12/16 is safe".
    #[wasm_bindgen_test]
    fn ar1_sites_12_16_tier2_production_path_unreachable_detached_context() {
        for count in [50usize, 200, 2000] {
            let list = detached(&array_input(count), count, false);
            let map = detached(&object_input(count), count, true);
            emit(&format!("site 12 DETACHED count {count} : {list:?}"));
            emit(&format!("site 16 DETACHED count {count} : {map:?}"));
            assert!(
                list.is_ok(),
                "site 12 detached arm failed at count {count}: {list:?} — if this is ever RED the \
                 production path is live and the two-tier verdict is wrong"
            );
            assert!(
                map.is_ok(),
                "site 16 detached arm failed at count {count}: {map:?} — same reading as site 12"
            );
        }
    }
}
