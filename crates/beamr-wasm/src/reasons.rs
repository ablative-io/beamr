//! Structured refusal-reason mapping for errored completions
//! (WPORT-5 R2 item 7).
//!
//! An errored pid used to surface to JavaScript as completion kind
//! `"errored"` with a `null` result — refusal CLASS stable, refusal REASON
//! unobservable. This module maps the scheduler-retained
//! [`ExecError`](beamr::error::ExecError) to a stable JSON shape so
//! facility-absent `badarg`, `undef`-with-MFA, and
//! `unsupported_opcode` (the wasm dirty-call wall) are distinguishable from
//! JS. The reason RIDES ALONGSIDE the existing completion fields — additive
//! only, never replacing the `"errored"` kind or the `result` field.
//!
//! Reason vocabulary: `{"error": <snake_case variant>}` for every variant,
//! plus `module`/`function`/`arity` for `undef`, `name` for
//! `unsupported_opcode`, and a human-readable `detail` (the `Display` text)
//! for everything else.

use beamr::atom::AtomTable;
use beamr::error::ExecError;
use serde_json::{Value, json};

/// Map a retained interpreter error to the stable JS-facing reason shape.
pub(crate) fn exec_error_to_reason(error: &ExecError, atom_table: &AtomTable) -> Value {
    match error {
        ExecError::Badarg => json!({ "error": "badarg" }),
        ExecError::Undef {
            module,
            function,
            arity,
        } => json!({
            "error": "undef",
            "module": atom_table.resolve(*module).unwrap_or("#<unknown>"),
            "function": atom_table.resolve(*function).unwrap_or("#<unknown>"),
            "arity": arity,
        }),
        ExecError::UnsupportedOpcode { name } => json!({
            "error": "unsupported_opcode",
            "name": name,
        }),
        other => json!({
            "error": variant_name(other),
            "detail": other.to_string(),
        }),
    }
}

/// Stable snake_case name for every [`ExecError`] variant.
fn variant_name(error: &ExecError) -> &'static str {
    match error {
        ExecError::Badmatch => "badmatch",
        ExecError::FunctionClause => "function_clause",
        ExecError::Undef { .. } => "undef",
        ExecError::Badarith => "badarith",
        ExecError::Badarg => "badarg",
        ExecError::Badfun { .. } => "badfun",
        ExecError::Badarity { .. } => "badarity",
        ExecError::UserExit => "user_exit",
        ExecError::UnknownOpcode { .. } => "unknown_opcode",
        ExecError::UnsupportedOpcode { .. } => "unsupported_opcode",
        ExecError::InvalidOperand(_) => "invalid_operand",
        ExecError::InvalidLabel { .. } => "invalid_label",
        ExecError::InvalidImport { .. } => "invalid_import",
        ExecError::GcNeeded { .. } => "gc_needed",
        ExecError::UnsupportedLiteral => "unsupported_literal",
        ExecError::Stack(_) => "stack",
        ExecError::HeapFull { .. } => "heap_full",
        ExecError::NoConnection => "no_connection",
        ExecError::ServiceUnavailable { .. } => "service_unavailable",
        ExecError::GuardBifUnavailable { .. } => "guard_bif_unavailable",
        ExecError::ReplayMismatch(_) => "replay_mismatch",
    }
}
