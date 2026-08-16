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
            // AR-1 site 12 — the threaded-tail shape, the same carrier as site
            // 11 in beamr's own `term::json`. The boxed cons `tail` was held
            // live across the recursive call, which allocates; the accumulator
            // holds the elements in the process native root stack and builds
            // the list once, at the end.
            //
            // ⚠️ THE ITERATION DIRECTION FLIPS — the old body walked `.rev()`
            // and prepended, this one walks forward and appends. The resulting
            // list is in the SAME ORDER; what changes is the order the elements
            // are ALLOCATED in, a heap-layout difference and not a term one.
            //
            // ⭐ THIS ARM RECURSES INTO ITSELF, so a nested array opens an
            // accumulator scope INSIDE an open one. That is sound for the same
            // reason the `with_rooted`-inside-`with_accumulator` nesting at the
            // URI sites is: the inner scope is innermost while it is open, and
            // the outer only pushes after the inner has popped.
            //
            // `with_accumulator`'s error channel is `Term` while this module's
            // is `JsValue`, so a failure from the recursive call is parked here
            // and re-raised after the closure rather than being flattened into
            // a generic cons-allocation message that would name the wrong thing.
            let mut parked: Option<JsValue> = None;
            let built = context.with_accumulator(|context, terms| {
                for value in elements {
                    let head = match json_value_to_term(value, context, depth + 1) {
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
                Err(_) => {
                    Err(parked.unwrap_or_else(|| JsValue::from_str("failed to allocate cons term")))
                }
            }
        }
        Value::Object(object) => {
            // AR-1 site 16 — the S3e `Vec<(Term, Term)>` shape, the same carrier
            // as site 15 in beamr's own `term::json`. Both halves were at risk:
            // the key was held live across its own value's recursive call, and
            // every pair already in the vector was held live across every later
            // allocation.
            //
            // The pairs now accumulate as ONE ALTERNATING key/value run in the
            // native root stack, with the key pushed BEFORE its value is built —
            // that ordering is the whole point. `to_map_pairs` and
            // `sort_pairs_by_key` both refuse `badarg` on an odd-length run, so
            // a future edit that pushes a key without its value is caught rather
            // than silently mis-pairing.
            //
            // ⚠️ `sort_pairs_by_key` sorts by RAW TERM VALUE exactly as
            // `alloc_sorted_map` does — but on pointers that are live rather
            // than possibly stale, so the resulting key ORDER can differ from
            // the pre-fix order. This does NOT resolve the ordering-by-raw-value
            // hazard; it inherits it, on valid pointers.
            //
            // ⛔ SUPERSEDED BY TRANCHE 3, and left here as a correction rather
            // than quietly overwritten: this comment used to read
            // "`alloc_sorted_map` STAYS: `object_to_term` (the JsValue path, a
            // separate crossing) is still its caller." That was true when site
            // 16 landed and stopped being true when site 17 was rooted. The
            // helper's last PRODUCTION caller is gone and it now lives in the
            // test module, serving only the pre-fix replica.
            let mut parked: Option<JsValue> = None;
            let built = context.with_accumulator(|context, terms| {
                for (key, value) in object {
                    let key_term = match context.alloc_binary(key.as_bytes()) {
                        Ok(term) => term,
                        Err(_) => {
                            parked = Some(JsValue::from_str("failed to allocate map key binary"));
                            return Err(Term::NIL);
                        }
                    };
                    terms.push(context, key_term)?;
                    let value_term = match json_value_to_term(value, context, depth + 1) {
                        Ok(term) => term,
                        Err(error) => {
                            parked = Some(error);
                            return Err(Term::NIL);
                        }
                    };
                    terms.push(context, value_term)?;
                }
                terms.sort_pairs_by_key(context)?;
                terms.to_map_pairs(context)
            });
            match built {
                Ok(term) => Ok(term),
                Err(_) => {
                    Err(parked.unwrap_or_else(|| JsValue::from_str("failed to allocate map term")))
                }
            }
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
    // AR-1 site 13 — the JsValue path's twin of site 12, and the reason
    // tranche 2 alone left this file HALF-FIXED: `json_value_to_term`'s Array
    // arm was rooted while this one — reachable from every embedder that enters
    // through `JsValue` rather than `serde_json` — kept the boxed cons `tail`
    // live across the recursive `value_to_term`, which allocates.
    //
    // Same remedy and the same direction flip as site 12: the old body walked
    // `.rev()` and prepended, this one walks forward and appends. The resulting
    // list is in the SAME ORDER; what changes is the order the elements are
    // ALLOCATED in, a heap-layout difference and not a term one.
    //
    // `depth` is passed through unchanged, exactly as the pre-fix body did —
    // the depth wall belongs to `value_to_term` and this remedy does not move it.
    let mut parked: Option<JsValue> = None;
    let built = context.with_accumulator(|context, terms| {
        for index in 0..array.length() {
            let element = match value_to_term(array.get(index), context, depth) {
                Ok(element) => element,
                Err(error) => {
                    parked = Some(error);
                    return Err(Term::NIL);
                }
            };
            terms.push(context, element)?;
        }
        terms.to_list(context)
    });
    match built {
        Ok(term) => Ok(term),
        Err(_) => Err(parked.unwrap_or_else(|| JsValue::from_str("failed to allocate cons term"))),
    }
}

fn object_to_term(
    value: JsValue,
    context: &mut ProcessContext<'_>,
    depth: usize,
) -> Result<Term, JsValue> {
    // AR-1 site 17 — the JsValue path's twin of site 16, and the site the
    // landing gate's row 4 names explicitly. The `Vec<(Term, Term)>` of boxed
    // key/value pairs was held live across BOTH `alloc_binary` and the
    // recursive `value_to_term`, either of which can collect and move them.
    //
    // ⚠️ `sort_pairs_by_key` sorts by RAW TERM VALUE, exactly as the pre-fix
    // `alloc_sorted_map` did — but on pointers that are LIVE rather than
    // possibly stale. This inherits the ground pack's sibling
    // ordering-by-raw-value hazard on valid pointers; it does not settle it.
    //
    // ⛔ `alloc_sorted_map` had its LAST PRODUCTION CALLER here. It survives only
    // to serve the pre-fix replicas that keep this site's control pressed, and
    // now sits at file scope behind a `#[cfg(test)]` gate — TWO sibling test
    // modules need it (sites 13/17 and sites 12/16), and duplicating a
    // control's helper is how two copies drift apart.
    let object = Object::from(value);
    let keys = Object::keys(&object);
    let mut parked: Option<JsValue> = None;
    let built = context.with_accumulator(|context, terms| {
        for index in 0..keys.length() {
            let key_value = keys.get(index);
            let Some(key) = key_value.as_string() else {
                parked = Some(JsValue::from_str("JavaScript object key was not a string"));
                return Err(Term::NIL);
            };
            let property = match Reflect::get(&object, &key_value) {
                Ok(property) => property,
                Err(error) => {
                    parked = Some(error);
                    return Err(Term::NIL);
                }
            };
            let key_term = match context.alloc_binary(key.as_bytes()) {
                Ok(term) => term,
                Err(_) => {
                    parked = Some(JsValue::from_str("failed to allocate map key binary"));
                    return Err(Term::NIL);
                }
            };
            terms.push(context, key_term)?;
            let value_term = match value_to_term(property, context, depth) {
                Ok(term) => term,
                Err(error) => {
                    parked = Some(error);
                    return Err(Term::NIL);
                }
            };
            terms.push(context, value_term)?;
        }
        terms.sort_pairs_by_key(context)?;
        terms.to_map_pairs(context)
    });
    match built {
        Ok(term) => Ok(term),
        Err(_) => Err(parked.unwrap_or_else(|| JsValue::from_str("failed to allocate map term"))),
    }
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

// RE-HOMED FROM PRODUCTION BY TRANCHE 3, byte-for-byte. Site 17's remedy removed
// its LAST PRODUCTION CALLER; the only remaining callers are the pre-fix replicas
// in the two test modules below, which must keep the unrooted shape verbatim for
// their controls to stay pressed.
//
// It sits at file scope behind a `#[cfg(test)]` gate rather than inside either
// test module because BOTH sibling test modules need it — `tests` for the site
// 13/17 replicas and `ar1_row4_sites_12_16_tests` for the site 12/16 one — and
// duplicating a control's helper is how two copies drift apart.
#[cfg(all(test, target_arch = "wasm32"))]
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

    // ================= AR-1 TRANCHE 3 — SITES 13 + 17, TWO-ARMED =================
    //
    // Cally Ray's row-4 probes established RED AT PARENT for these two sites at
    // `308b448`. That red was re-established AT THE SHIP TREE `ae29d2c` before
    // either site was touched — 83 passed / 2 failed, both probes failing, the
    // 83 being exactly the pre-existing baseline so the failures were
    // attributable to the probes and not to collateral breakage. Transcript:
    // `gate-logs/111/tranche3/red-at-ae29d2c.log`.
    //
    // ⭐ The probes are INVERTED here rather than left as they were. Her probes
    // drive PRODUCTION `value_to_term`, so the remedy that makes them pass also
    // destroys them as evidence: a green from a single-armed probe cannot tell
    // "the accumulator survived a move" from "no move happened" or "the input
    // stopped pressing". The replicas below carry the PRE-FIX bodies verbatim so
    // the control survives its own remedy.
    //
    // ⛔ `*_unrooted_replica` MUST NEVER BE FIXED. They are the positive control.

    fn value_to_term_unrooted_replica(
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
        // ⭐ RECURSES INTO THE REPLICA, not into production. A replica that
        // recursed into the fixed body would be pre-fix only at the top level
        // and would quietly stop pressing at depth.
        if Array::is_array(&value) {
            return array_to_term_unrooted_replica(&Array::from(&value), context, depth + 1);
        }
        if value.is_object() {
            return object_to_term_unrooted_replica(value, context, depth + 1);
        }

        Err(JsValue::from_str(
            "unsupported JavaScript value for BEAM term conversion",
        ))
    }

    // Site 13's PRE-FIX body, verbatim from `ae29d2c`.
    fn array_to_term_unrooted_replica(
        array: &Array,
        context: &mut ProcessContext<'_>,
        depth: usize,
    ) -> Result<Term, JsValue> {
        let mut tail = Term::NIL;
        for index in (0..array.length()).rev() {
            let head = value_to_term_unrooted_replica(array.get(index), context, depth)?;
            tail = context
                .alloc_cons(head, tail)
                .map_err(|_| JsValue::from_str("failed to allocate cons term"))?;
        }
        Ok(tail)
    }

    // Site 17's PRE-FIX body, verbatim from `ae29d2c`, including its call to
    // the re-homed `alloc_sorted_map`.
    fn object_to_term_unrooted_replica(
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
            let value_term = value_to_term_unrooted_replica(property, context, depth)?;
            pairs.push((key_term, value_term));
        }
        alloc_sorted_map(pairs, context)
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum JsArm {
        Fixed,
        UnrootedReplica,
    }

    fn convert_js(
        value: JsValue,
        context: &mut ProcessContext<'_>,
        arm: JsArm,
    ) -> Result<Term, JsValue> {
        match arm {
            JsArm::Fixed => value_to_term(value, context, 0),
            JsArm::UnrootedReplica => value_to_term_unrooted_replica(value, context, 0),
        }
    }

    fn arm_name(arm: JsArm) -> &'static str {
        match arm {
            JsArm::Fixed => "FIXED",
            JsArm::UnrootedReplica => "CONTROL",
        }
    }

    // A refusal is NOT corruption and is scored as its own named outcome — a
    // broken rooting can produce either, and folding them together would let a
    // refusal be read as a clean run.
    #[derive(PartialEq, Eq, Debug)]
    enum Outcome {
        Clean,
        Corrupt,
        Refused,
    }

    fn list_to_vec_checked(mut term: Term) -> Option<Vec<Term>> {
        let mut values = Vec::new();
        while !term.is_nil() {
            let cons = Cons::new(term)?;
            values.push(cons.head());
            term = cons.tail();
            if values.len() > 10_000 {
                return None;
            }
        }
        Some(values)
    }

    fn array_round_trip(count: u32, pid: u64, arm: JsArm) -> Outcome {
        let table = atom_table();
        let mut process =
            beamr::process::Process::new(pid, beamr::process::heap::DEFAULT_HEAP_SIZE);

        // The elements MUST be strings, not numbers: a small integer is an
        // IMMEDIATE, so `value_to_term` returns it without allocating and the
        // carrier is never live across an allocation.
        let array = Array::new();
        for index in 0..count {
            array.push(&JsValue::from_str(&format!("element-{index}")));
        }

        let before = process.heap().total_capacity();
        let built = {
            let mut context = ProcessContext::new();
            context.set_atom_table(Some(Arc::clone(&table)));
            context.attach_process(&mut process, 0);
            convert_js(JsValue::from(array), &mut context, arm)
        };
        let after = process.heap().total_capacity();

        let classify = || {
            let Ok(term) = built else {
                return Outcome::Refused;
            };
            let Some(values) = list_to_vec_checked(term) else {
                return Outcome::Corrupt;
            };
            if values.len() != count as usize {
                return Outcome::Corrupt;
            }
            for (index, value) in values.iter().enumerate() {
                let js = term_to_js_value(*value, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
                if js.as_string().as_deref() != Some(format!("element-{index}").as_str()) {
                    return Outcome::Corrupt;
                }
            }
            Outcome::Clean
        };
        let outcome = classify();

        assert_pressed(before, after, count, arm, &outcome);
        outcome
    }

    // POSITIVE CONTROL, per cell — and it is deliberately conditioned on the
    // outcome rather than demanded unconditionally.
    //
    // ⭐ `after > before` on `total_capacity()` witnesses a heap RESIZE, not a
    // COLLECTION. There is no collection counter to ask: `total_capacity` is the
    // heap's only observable. A pre-fix replica can REFUSE without the heap ever
    // resizing, so an unconditional resize demand mis-scores the exact arm the
    // control exists to grade — measured here as `466 -> 466, arm CONTROL,
    // built_ok=false`, unchanged when the input was raised 40 -> 60, which is
    // what identified it as structural rather than a threshold.
    //
    // What the control is FOR is preventing a FALSE CLEAN. A Refused or Corrupt
    // outcome is self-evidently pressed — the body failed. So the witness is
    // required exactly where a green could otherwise be bought by an input too
    // small to press anything, and a Clean cell with no pressure still fails
    // loudly on EITHER arm.
    fn assert_pressed(before: usize, after: usize, count: u32, arm: JsArm, outcome: &Outcome) {
        if *outcome != Outcome::Clean {
            return;
        }
        assert!(
            after > before,
            "heap never grew ({before} -> {after}) at count {count} on the {} arm, \
             yet the round trip came back Clean -- this cell applied NO memory \
             pressure, so its Clean is not evidence",
            arm_name(arm)
        );
    }

    fn object_round_trip(count: u32, pid: u64, arm: JsArm) -> Outcome {
        let table = atom_table();
        let mut process =
            beamr::process::Process::new(pid, beamr::process::heap::DEFAULT_HEAP_SIZE);

        let object = Object::new();
        for index in 0..count {
            assert!(
                Reflect::set(
                    &object,
                    &JsValue::from_str(&format!("key-{index:03}")),
                    &JsValue::from_str(&format!("value-{index}")),
                )
                .is_ok()
            );
        }

        let before = process.heap().total_capacity();
        let built = {
            let mut context = ProcessContext::new();
            context.set_atom_table(Some(Arc::clone(&table)));
            context.attach_process(&mut process, 0);
            convert_js(JsValue::from(object), &mut context, arm)
        };
        let after = process.heap().total_capacity();

        let classify = || {
            let Ok(term) = built else {
                return Outcome::Refused;
            };
            let js = term_to_js_value(term, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
            for index in 0..count {
                let got = Reflect::get(&js, &JsValue::from_str(&format!("key-{index:03}")))
                    .unwrap_or(JsValue::UNDEFINED);
                if got.as_string().as_deref() != Some(format!("value-{index}").as_str()) {
                    return Outcome::Corrupt;
                }
            }
            Outcome::Clean
        };
        let outcome = classify();

        assert_pressed(before, after, count, arm, &outcome);
        outcome
    }

    // AR-1 SITE 13 — `array_to_term`, the JsValue path's threaded tail.
    #[wasm_bindgen_test]
    fn ar1_site13_array_to_term_two_armed() {
        let mut pid = 100;
        let mut control_pressed = 0;
        for count in [200_u32, 400, 800] {
            pid += 1;
            let control = array_round_trip(count, pid, JsArm::UnrootedReplica);
            if control != Outcome::Clean {
                control_pressed += 1;
            }
            pid += 1;
            let fixed = array_round_trip(count, pid, JsArm::Fixed);
            assert_eq!(
                fixed,
                Outcome::Clean,
                "SHIPPED site 13 must be clean at count {count}, got {fixed:?}"
            );
        }
        assert!(
            control_pressed > 0,
            "the site-13 CONTROL was Clean on EVERY cell -- the pre-fix shape is \
             not being pressed, so the shipped arm's green is worth nothing"
        );
    }

    // AR-1 SITE 17 — `object_to_term`, the JsValue path's pair accumulator, and
    // the site the landing gate's row 4 names explicitly.
    #[wasm_bindgen_test]
    fn ar1_site17_object_to_term_two_armed() {
        let mut pid = 200;
        let mut control_pressed = 0;
        for count in [60_u32, 90, 120] {
            pid += 1;
            let control = object_round_trip(count, pid, JsArm::UnrootedReplica);
            if control != Outcome::Clean {
                control_pressed += 1;
            }
            pid += 1;
            let fixed = object_round_trip(count, pid, JsArm::Fixed);
            assert_eq!(
                fixed,
                Outcome::Clean,
                "SHIPPED site 17 must be clean at count {count}, got {fixed:?}"
            );
        }
        assert!(
            control_pressed > 0,
            "the site-17 CONTROL was Clean on EVERY cell -- the pre-fix shape is \
             not being pressed, so the shipped arm's green is worth nothing"
        );
    }

    // ⭐ NESTING, which neither flat sweep above can reach: both fixed arms
    // recurse into themselves, so a nested array-of-objects opens an
    // accumulator scope INSIDE an open one, and `rooted_push` refuses unless
    // its handle is innermost. The flat sweeps use string leaves and never
    // recurse, so this is the only cell that exercises the nesting.
    #[wasm_bindgen_test]
    fn ar1_sites_13_17_nested_scopes_open_inside_one_another() {
        let table = atom_table();
        let mut process =
            beamr::process::Process::new(300, beamr::process::heap::DEFAULT_HEAP_SIZE);

        let outer = Array::new();
        for group in 0..40 {
            let object = Object::new();
            for entry in 0..6 {
                assert!(
                    Reflect::set(
                        &object,
                        &JsValue::from_str(&format!("k{entry}")),
                        &JsValue::from_str(&format!("g{group}-e{entry}")),
                    )
                    .is_ok()
                );
            }
            outer.push(&object);
        }

        let before = process.heap().total_capacity();
        let built = {
            let mut context = ProcessContext::new();
            context.set_atom_table(Some(Arc::clone(&table)));
            context.attach_process(&mut process, 0);
            convert_js(JsValue::from(outer), &mut context, JsArm::Fixed)
        };
        let after = process.heap().total_capacity();

        // Same ordering rule as `assert_pressed`, and it is the inversion of the
        // flaw in the probe this test descends from: a refusal is graded on its
        // own terms FIRST, because a resize guard placed ahead of it would panic
        // about pressure while saying nothing about the hazard that actually
        // fired. The pressure witness then guards the success path, which is the
        // only place a false clean can hide.
        let term = built.expect(
            "nested array-of-objects converts -- a refusal here is the \
             accumulator-inside-an-accumulator hazard, NOT an allocation limit",
        );

        assert!(
            after > before,
            "heap never grew ({before} -> {after}) -- the nested cell converted \
             cleanly without applying any memory pressure, so it proves nothing \
             about scope nesting"
        );
        let groups = list_to_vec_checked(term).expect("nested conversion is a proper list");
        assert_eq!(groups.len(), 40, "outer list length after the collection");
        for (group, value) in groups.iter().enumerate() {
            let js = term_to_js_value(*value, table.as_ref()).unwrap_or(JsValue::UNDEFINED);
            for entry in 0..6 {
                let got = Reflect::get(&js, &JsValue::from_str(&format!("k{entry}")))
                    .unwrap_or(JsValue::UNDEFINED);
                assert_eq!(
                    got.as_string().as_deref(),
                    Some(format!("g{group}-e{entry}").as_str()),
                    "group {group} entry {entry} survived nested scopes intact"
                );
            }
        }
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

    /// Which body a cell drives. `Fixed` is the shipped `json_value_to_term`;
    /// `UnrootedReplica` is the pre-fix body kept alive as the positive control.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Arm {
        Fixed,
        UnrootedReplica,
    }

    /// ⛔⛔ THE SYNTHETIC POSITIVE — `json_value_to_term`'s body EXACTLY AS IT
    /// WAS BEFORE THE FIX, and it must stay that way. It is RECURSIVE and
    /// carries BOTH defects at once: the threaded `tail` of site 12's Array arm
    /// and the `Vec<(Term, Term)>` of site 16's Object arm. That is why the two
    /// sites share one control — a replica of either arm alone would recurse
    /// back into the fixed body and stop being a replica.
    /// ⛔ Do NOT migrate it onto the accumulator, and do NOT fix its Object arm
    /// when site 16 is fixed in production: this function is the control.
    fn json_value_to_term_unrooted_replica(
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
                    let head = json_value_to_term_unrooted_replica(value, context, depth + 1)?;
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
                    let value_term =
                        json_value_to_term_unrooted_replica(value, context, depth + 1)?;
                    pairs.push((key_term, value_term));
                }
                alloc_sorted_map(pairs, context)
            }
        }
    }

    fn convert(value: &Value, context: &mut ProcessContext<'_>, arm: Arm) -> Result<Term, JsValue> {
        match arm {
            Arm::Fixed => json_value_to_term(value, context, 0),
            Arm::UnrootedReplica => json_value_to_term_unrooted_replica(value, context, 0),
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

    /// An array OF ARRAYS — `outer` inner arrays of `inner` string elements
    /// each. This is the only fixture that makes the Array arm recurse into
    /// ITSELF, which is what opens an accumulator scope inside an open one.
    fn nested_input(outer: usize, inner: usize) -> Value {
        Value::Array(
            (0..outer)
                .map(|_| {
                    Value::Array(
                        (0..inner)
                            .map(|index| Value::String(element(index)))
                            .collect(),
                    )
                })
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
        arm: Arm,
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

        let outcome = match convert(input, &mut context, arm) {
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
    fn detached(input: &Value, count: usize, is_map: bool, arm: Arm) -> Result<(), String> {
        let table = Arc::new(AtomTable::with_common_atoms());
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        match convert(input, &mut context, arm) {
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
    fn sweep(label: &str, is_map: bool, arm: Arm) -> (usize, usize, Vec<String>) {
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
                let (achieved, result) = attached(&input, 4096, margin, count, is_map, arm);
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
        // ⛔⛔ CONTROL FIRST, and it licenses everything below it. The replica
        // carries BOTH pre-fix arms, so it is still the live positive control
        // for site 16 as well as the retired one for site 12.
        let (ctl_list_red, ctl_list_ok, ctl_list_report) =
            sweep("CONTROL site 12 (tail)", false, Arm::UnrootedReplica);
        let (ctl_map_red, ctl_map_ok, ctl_map_report) =
            sweep("CONTROL site 16 (pairs)", true, Arm::UnrootedReplica);
        let (fix_list_red, fix_list_ok, fix_list_report) =
            sweep("SHIPPED site 12 (tail)", false, Arm::Fixed);
        let (fix_map_red, fix_map_ok, fix_map_report) =
            sweep("SHIPPED site 16 (pairs)", true, Arm::Fixed);
        let report = format!(
            "CONTROL site 12: {ctl_list_red} corrupt, {ctl_list_ok} clean\n\
             CONTROL site 16: {ctl_map_red} corrupt, {ctl_map_ok} clean\n\
             SHIPPED site 12: {fix_list_red} corrupt, {fix_list_ok} clean\n\
             SHIPPED site 16: {fix_map_red} corrupt, {fix_map_ok} clean\n{}\n{}\n{}\n{}",
            ctl_list_report.join("\n"),
            ctl_map_report.join("\n"),
            fix_list_report.join("\n"),
            fix_map_report.join("\n")
        );
        emit(&report);

        assert!(
            ctl_list_ok > 0 && ctl_map_ok > 0,
            "control: some cell must be clean, or the probe never worked at all\n{report}"
        );
        assert!(
            ctl_list_red > 0,
            "POSITIVE CONTROL DEAD at site 12: the unrooted replica no longer corrupts `tail` \
             with a process ATTACHED, so the pressure regime is gone and the shipped arm's \
             success below would mean nothing.\n{report}"
        );
        assert!(
            ctl_map_red > 0,
            "POSITIVE CONTROL DEAD at site 16: same reading as site 12.\n{report}"
        );

        // ⭐ PRE-REGISTERED EXACT COUNTS. The runner emits per-test output ONLY
        // for failures — measured three ways: `println!` absent, `println!`
        // under `-- --nocapture` still absent, `console.log` via js_sys also
        // absent; only a panic payload survives. So a bare `> 0` assertion
        // would PASS SILENTLY and the band would be invisible on every green
        // run. Pinning the counts makes any drift print the whole surface.
        //
        // ⭐ THE CONTROL BAND IS THE REPLICA-FIDELITY CHECK: (21, 27, 16, 8) is
        // the band the PRODUCTION body measured before the fix, so a replica
        // that reproduces it cell for cell is calibrated rather than merely
        // plausible.
        assert_eq!(
            (ctl_list_red, ctl_list_ok, ctl_map_red, ctl_map_ok),
            (21, 27, 16, 8),
            "control band drifted from the pre-fix production surface\n{report}"
        );

        // ✅ THE CLAIM — site 12 only. Same cells, same margins, same input.
        assert_eq!(
            (fix_list_red, fix_list_ok),
            (0, 48),
            "site 12 is NOT rooted: the shipped body lost the carrier on some cell while the \
             replica corrupted 21 of the same cells in the same run\n{report}"
        );

        // ✅ THE CLAIM — site 16. Same cells, same margins, same input. The
        // control band above is unchanged by this, which is the point: the
        // replica still carries the pre-fix Object arm.
        assert_eq!(
            (fix_map_red, fix_map_ok),
            (0, 24),
            "site 16 is NOT rooted: the shipped body lost the carrier on some cell while the \
             replica corrupted 16 of the same cells in the same run\n{report}"
        );
    }

    /// Site 12's fix rests on a claim the sweep above CANNOT TEST, because its
    /// arrays are flat: that an accumulator scope opened INSIDE an open one is
    /// accepted. `rooted_push` refuses unless its handle is innermost, so if the
    /// nesting were wrong the recursive arm would return a refusal rather than a
    /// wrong value — and a refusal is exactly the outcome the sweep scores as
    /// "not corruption". ⭐ NAMING THE HAZARD IN A COMMENT IS NOT MEASURING IT.
    ///
    /// The replica runs the SAME cells, for two reasons at once: a red there
    /// proves this fixture actually applies collection pressure at the nesting
    /// depth (so a clean fixed arm is not merely an unpressed one), and it
    /// proves the reader below can report a failure at all.
    fn nested_round_trip(outer: usize, inner: usize, margin: usize, arm: Arm) -> (usize, String) {
        let input = nested_input(outer, inner);
        let table = Arc::new(AtomTable::with_common_atoms());
        let mut process = Process::new(12, 4096);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        context.attach_process(&mut process, 0);

        let mut filler = Vec::new();
        let mut last_available = usize::MAX;
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

        let outcome = match convert(&input, &mut context, arm) {
            // ⛔ A REFUSAL IS THE FAILURE MODE THIS TEST EXISTS FOR, so it is
            // named as one rather than folded into "not clean".
            Err(_) => Err("REFUSED — the nested scope was rejected".to_string()),
            Ok(term) => {
                let mut seen = 0usize;
                let mut tail = term;
                let cap = outer * 2 + 16;
                let mut stale = None;
                while !tail.is_nil() && seen <= cap {
                    match Cons::new(tail) {
                        None => {
                            stale = Some(format!("outer cell {seen} is not a cons"));
                            break;
                        }
                        Some(cons) => {
                            if let Err(reason) = check_list(cons.head(), inner) {
                                stale = Some(format!("outer cell {seen}: {reason}"));
                                break;
                            }
                            seen += 1;
                            tail = cons.tail();
                        }
                    }
                }
                match stale {
                    Some(reason) => Err(reason),
                    None if seen == outer => Ok(()),
                    None => Err(format!("recovered {seen} inner lists, put {outer}")),
                }
            }
        };
        let verdict = match outcome {
            Ok(()) => "ok".to_string(),
            Err(reason) => reason,
        };
        (achieved, verdict)
    }

    #[wasm_bindgen_test]
    fn ar1_site12_nested_arrays_open_an_accumulator_inside_an_open_one() {
        let mut report = Vec::new();
        let mut control_red = 0usize;
        let mut control_refused = 0usize;
        let mut fixed_bad = Vec::new();
        for (outer, inner) in [(2usize, 4usize), (20, 8), (100, 12)] {
            for margin in [0usize, 8, 64, 512] {
                let (ctl_margin, ctl) =
                    nested_round_trip(outer, inner, margin, Arm::UnrootedReplica);
                let (fix_margin, fix) = nested_round_trip(outer, inner, margin, Arm::Fixed);
                let line = format!(
                    "nested {outer}x{inner} margin req {margin:>4} | CONTROL got {ctl_margin:>5} : \
                     {ctl} | SHIPPED got {fix_margin:>5} : {fix}"
                );
                emit(&line);
                report.push(line);
                if ctl != "ok" {
                    if ctl.starts_with("REFUSED") {
                        control_refused += 1;
                    } else {
                        control_red += 1;
                    }
                }
                if fix != "ok" {
                    fixed_bad.push(format!("{outer}x{inner} margin {margin}: {fix}"));
                }
            }
        }
        let report = report.join("\n");

        assert!(
            control_red > 0,
            "NO PRESSURE: the unrooted replica round-tripped every nested cell cleanly, so the \
             shipped arm's success proves nothing about the nested scope. {control_refused} cells \
             were refusals, which are not evidence either.\n{report}"
        );
        assert!(
            fixed_bad.is_empty(),
            "the shipped body failed on nested arrays — an accumulator scope inside an open one \
             was rejected or lost its carrier: {fixed_bad:?}\n{report}"
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
        // ⛔⛔ THE HAZARD THIS ARM NOW ALSO GUARDS: `with_rooted` REFUSES with
        // `badarg` when no process is attached, and every production caller here
        // is detached. If `with_accumulator` did not fall back to owned storage
        // on a detached context, the fix would have turned the whole production
        // path into a refusal — a behaviour change dressed as a repair. The
        // shipped arm below is what proves it did not.
        for count in [50usize, 200, 2000] {
            for arm in [Arm::UnrootedReplica, Arm::Fixed] {
                let which = if arm == Arm::Fixed {
                    "SHIPPED"
                } else {
                    "CONTROL"
                };
                let list = detached(&array_input(count), count, false, arm);
                let map = detached(&object_input(count), count, true, arm);
                emit(&format!(
                    "{which} site 12 DETACHED count {count} : {list:?}"
                ));
                emit(&format!("{which} site 16 DETACHED count {count} : {map:?}"));
                assert!(
                    list.is_ok(),
                    "{which} site 12 detached arm failed at count {count}: {list:?} — if this is \
                     ever RED the production path is live and the two-tier verdict is wrong"
                );
                assert!(
                    map.is_ok(),
                    "{which} site 16 detached arm failed at count {count}: {map:?} — same reading \
                     as site 12"
                );
            }
        }
    }
}
