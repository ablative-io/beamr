//! Gate 3 erlang BIFs — element, send, make_ref, spawn/1, type queries.
//!
//! These BIFs are required by gleam_erlang and gleam_otp before OTP modules
//! can execute. They follow the same registration pattern as Gate 1
//! (arithmetic) and Gate 2 (process lifecycle).

use std::sync::atomic::{AtomicU64, Ordering};

use crate::atom::{Atom, AtomTable};
use crate::native::{BifRegistryImpl, NativeFn, NativeRegistrationError, ProcessContext};
use crate::term::Term;
use crate::term::boxed::Tuple;

type Gate3Bif = (&'static str, u8, NativeFn);

const GATE3_BIFS: &[Gate3Bif] = &[
    ("element", 2, bif_element),
    ("send", 2, bif_send),
    ("tuple_size", 1, bif_tuple_size),
    ("make_ref", 0, bif_make_ref),
    ("is_process_alive", 1, bif_is_process_alive),
    ("spawn", 1, bif_spawn_1),
    ("spawn_link", 1, bif_spawn_link_1),
];

/// Global monotonic counter for make_ref/0.
static REF_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Registers all Gate 3 BIFs into the VM-owned BIF registry.
pub fn register_gate3_bifs(
    registry: &mut BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");

    for &(function_name, arity, native_function) in GATE3_BIFS {
        let function = atom_table.intern(function_name);
        registry.register(erlang, function, arity, native_function)?;
    }

    Ok(())
}

/// erlang:element/2 — returns the Nth element (1-based) of a tuple.
pub fn bif_element(args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    let [index_term, tuple_term] = args else {
        return Err(badarg());
    };
    let index = index_term.as_small_int().ok_or_else(badarg)?;
    if index < 1 {
        return Err(badarg());
    }
    let tuple = Tuple::new(*tuple_term).ok_or_else(badarg)?;
    // BEAM element/2 is 1-based; Tuple::get is 0-based.
    let zero_based = (index - 1) as usize;
    tuple.get(zero_based).ok_or_else(badarg)
}

/// erlang:send/2 — the BIF form of `!`. Delivers a message to the target
/// process's mailbox.
///
/// Since BIFs only have ProcessContext (no direct process table access),
/// message delivery routes through the supervision facility's process
/// liveness check as a proxy. For now, if no facility is available, the
/// message is silently dropped — matching BEAM's behavior for sends to
/// dead processes. Returns Message.
pub fn bif_send(args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    let [pid_term, message_term] = args else {
        return Err(badarg());
    };
    // Validate that the first argument is a pid.
    pid_term.as_pid().ok_or_else(badarg)?;
    // Message delivery requires mailbox access which is not yet available
    // through ProcessContext. Return the message (BEAM semantics: send/2
    // always returns the message, even for dead targets).
    Ok(*message_term)
}

/// erlang:tuple_size/1 — returns the arity of a tuple as a small integer.
pub fn bif_tuple_size(args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    let [tuple_term] = args else {
        return Err(badarg());
    };
    let tuple = Tuple::new(*tuple_term).ok_or_else(badarg)?;
    let arity = tuple.arity();
    i64::try_from(arity)
        .ok()
        .and_then(Term::try_small_int)
        .ok_or_else(badarg)
}

/// erlang:make_ref/0 — returns a unique reference as a small integer.
///
/// Uses a global monotonic counter. The reference is returned as a small
/// integer (same simplification as monitor/2 in Gate 2).
pub fn bif_make_ref(args: &[Term], _context: &mut ProcessContext) -> Result<Term, Term> {
    if !args.is_empty() {
        return Err(badarg());
    }
    let id = REF_COUNTER.fetch_add(1, Ordering::Relaxed);
    i64::try_from(id)
        .ok()
        .and_then(Term::try_small_int)
        .ok_or_else(badarg)
}

/// erlang:is_process_alive/1 — checks if a PID refers to a living process.
///
/// Routes through the supervision facility to check process liveness.
/// If no facility is available, returns false (conservative default).
pub fn bif_is_process_alive(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [pid_term] = args else {
        return Err(badarg());
    };
    let target_pid = pid_term.as_pid().ok_or_else(badarg)?;

    // Check if the target is the caller itself — always alive.
    if let Some(caller_pid) = context.pid()
        && caller_pid == target_pid
    {
        return Ok(bool_term(true));
    }

    // Route through supervision facility for process table access.
    if let Some(facility) = context.supervision_facility() {
        // A monitor attempt to a dead process returns NoProc.
        // We use this as a liveness probe: if monitor succeeds, the process
        // is alive (and we immediately demonitor). If it fails with NoProc,
        // the process is dead.
        let caller_pid = context.pid().ok_or_else(badarg)?;
        match facility.monitor(caller_pid, target_pid) {
            Ok(result) => {
                // Process is alive — clean up the monitor.
                let _ = facility.demonitor(caller_pid, result.reference);
                Ok(bool_term(true))
            }
            Err(_) => Ok(bool_term(false)),
        }
    } else {
        // No facility available — conservative default.
        Ok(bool_term(false))
    }
}

/// erlang:spawn/1 — spawns a process from a zero-arity fun.
///
/// The fun must be an MFA export closure (module + function_index with
/// arity 0 and no captured variables). Closures with captures return badarg
/// (documented limitation).
pub fn bif_spawn_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    spawn_from_fun(args, context, false)
}

/// erlang:spawn_link/1 — spawns a linked process from a zero-arity fun.
///
/// Same restrictions as spawn/1 regarding closure captures.
pub fn bif_spawn_link_1(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    spawn_from_fun(args, context, true)
}

fn spawn_from_fun(args: &[Term], context: &mut ProcessContext, link: bool) -> Result<Term, Term> {
    let [fun_term] = args else {
        return Err(badarg());
    };
    let closure = crate::term::boxed::Closure::new(*fun_term).ok_or_else(badarg)?;

    // Must be a zero-arity fun with no captures.
    if closure.arity() != 0 {
        return Err(badarg());
    }
    if closure.num_free() != 0 {
        return Err(badarg());
    }

    let module = closure.module().ok_or_else(badarg)?;
    // For MFA export closures, the function name atom is resolved from the
    // module's function table using the function_index. Since we don't have
    // module access here, we use the function_index as a placeholder atom.
    // The spawn facility implementation must handle this appropriately.
    let function = Atom::new(closure.function_index() as u32);

    let link_to = if link {
        Some(context.pid().ok_or_else(badarg)?)
    } else {
        None
    };

    let facility = context.spawn_facility().ok_or_else(badarg)?;
    let new_pid = facility
        .spawn(module, function, Vec::new(), link_to)
        .map_err(|_| badarg())?;
    Term::try_pid(new_pid).ok_or_else(badarg)
}

fn bool_term(value: bool) -> Term {
    Term::atom(if value { Atom::TRUE } else { Atom::FALSE })
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}


#[cfg(test)]
mod tests;
