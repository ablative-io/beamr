//! Runtime helpers callable from JIT-generated code.

use crate::atom::Atom;
use crate::gc;
use crate::interpreter::{ExecutionResult, run_with_registry};
use crate::module::ResolvedImportTarget;
use crate::process::{CodePosition, ExitReason, JitStatus, Process};
use crate::term::Term;
use crate::term::boxed::write_float;

use super::ir_common::JIT_DEOPT_SENTINEL;
use super::ir_exceptions::JitReturn;

pub(crate) const JIT_YIELD_SENTINEL: i64 = -2;

/// Reserves heap words for a tuple and returns the first word to fill.
///
/// The generated code writes the tuple header and payload after this call. A
/// null return asks compiled code to deopt when allocation or GC cannot provide
/// enough space.
pub(crate) extern "C" fn jit_alloc_tuple(process: *mut Process, arity: u64) -> *mut u64 {
    let Some(process) = process_from_abi(process) else {
        return std::ptr::null_mut();
    };
    let Ok(arity) = usize::try_from(arity) else {
        return std::ptr::null_mut();
    };
    let Some(words) = arity.checked_add(1) else {
        return std::ptr::null_mut();
    };
    alloc_words(process, words)
}

/// Reserves heap words for one cons cell and returns the first word to fill.
///
/// The generated code writes the head/tail words and tags the returned pointer
/// as a list term. A null return asks compiled code to deopt.
pub(crate) extern "C" fn jit_alloc_cons(process: *mut Process) -> *mut u64 {
    let Some(process) = process_from_abi(process) else {
        return std::ptr::null_mut();
    };
    alloc_words(process, 2)
}

/// Allocates a boxed float and returns its tagged term, or `0` when allocation fails.
pub(crate) extern "C" fn jit_box_float(process: *mut Process, value: f64) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 0;
    };
    let heap = alloc_words(process, 2);
    if heap.is_null() {
        return 0;
    }

    // SAFETY: `alloc_words(process, 2)` returned a non-null pointer to exactly
    // two heap words owned by `process` for the duration of this helper call.
    let heap = unsafe { std::slice::from_raw_parts_mut(heap, 2) };
    write_float(heap, value).map_or(0, Term::raw)
}

/// Charges one reduction at compiled function entry.
///
/// Returns `0` when compiled execution can continue and `1` when the native
/// wrapper should yield back to the scheduler.
pub(crate) extern "C" fn jit_charge_reduction(process: *mut Process) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 1;
    };
    process.decrement_reductions(1);
    u64::from(process.reductions_exhausted())
}

/// Calls an interpreted external function from compiled code.
///
/// `module`, `function`, and `arity` identify the import MFA and `args` points
/// to the compiled register file containing the call arguments in x registers.
/// The helper returns `(status, value)`, where status `1` propagates an
/// exception left in the process exception state.
pub(crate) extern "C" fn jit_call_interpreted(
    process: *mut Process,
    module: u64,
    function: u64,
    arity: u64,
    args: *const u64,
) -> JitReturn {
    let Some(process) = process_from_abi(process) else {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    };
    let Some(context) = process.jit_runtime_context() else {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    };
    if context.module.is_null() || context.registry.is_null() {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    }
    let Ok(module_index) = u32::try_from(module) else {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    };
    let Ok(import_index) = usize::try_from(function) else {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    };
    let Ok(arity) = u8::try_from(arity) else {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    };
    if args.is_null() && arity != 0 {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    }

    let module_atom = Atom::new(module_index);

    for register in 0..arity {
        let raw = if arity == 0 {
            0
        } else {
            // SAFETY: Generated code passes its live register-file pointer as
            // `args`; the helper bounds reads by the call arity validated above.
            unsafe { *args.add(usize::from(register)) }
        };
        process.set_x_reg(u16::from(register), Term::from_raw(raw));
    }

    // SAFETY: The interpreter installs pointers to the current borrowed module
    // and registry for exactly the duration of the native JIT call. Helpers run
    // synchronously before that context is cleared.
    let current_module = unsafe { &*context.module };
    // SAFETY: See `current_module`; the registry pointer has the same lifetime.
    let registry = unsafe { &*context.registry };
    if current_module.name != module_atom {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    }
    let Some(resolved) = current_module.resolved_imports.get(import_index) else {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    };
    if resolved.arity != arity {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    }
    let (target_module_atom, target_function, target_arity) = match resolved.target {
        ResolvedImportTarget::Code { .. } | ResolvedImportTarget::Deferred { .. } => {
            (resolved.module, resolved.function, resolved.arity)
        }
        ResolvedImportTarget::Unresolved { .. }
        | ResolvedImportTarget::Native(_)
        | ResolvedImportTarget::Denied { .. } => {
            return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
        }
    };
    let Some(target_module) = registry.lookup(target_module_atom) else {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    };
    let Ok(instruction_pointer) = target_module.export_ip(target_function, target_arity) else {
        return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
    };
    let saved_module = process.current_module().cloned();
    let saved_position = process.code_position();
    process.set_current_module(target_module);
    process.set_code_position(Some(CodePosition {
        module: target_module_atom,
        instruction_pointer,
    }));
    process.decrement_reductions(1);
    if process.reductions_exhausted() {
        process.set_jit_status(Some(JitStatus::Yield));
        return JitReturn::yield_(JIT_YIELD_SENTINEL as u64);
    }

    let result = run_with_registry(process, current_module, registry);
    if let Some(module) = saved_module {
        process.set_current_module(module);
    }
    process.set_code_position(saved_position);
    match result {
        Ok(ExecutionResult::Exited(ExitReason::Normal)) => {
            JitReturn::normal(process.x_reg(0).raw())
        }
        Ok(ExecutionResult::Exited(_)) if process.current_exception().is_some() => {
            let reason = process
                .current_exception()
                .map_or(Term::NIL.raw(), |exception| exception.reason.raw());
            JitReturn::exception(reason)
        }
        Ok(ExecutionResult::Exited(_))
        | Ok(ExecutionResult::Waiting)
        | Ok(ExecutionResult::DirtyCall { .. }) => JitReturn::deopt(JIT_DEOPT_SENTINEL as u64),
        Ok(ExecutionResult::Yielded) => {
            process.set_jit_status(Some(JitStatus::Yield));
            JitReturn::yield_(JIT_YIELD_SENTINEL as u64)
        }
        Err(_error) if process.current_exception().is_some() => {
            let reason = process
                .current_exception()
                .map_or(Term::NIL.raw(), |exception| exception.reason.raw());
            JitReturn::exception(reason)
        }
        Err(_error) => JitReturn::deopt(JIT_DEOPT_SENTINEL as u64),
    }
}

/// Pushes a Y-register stack frame with `y_slots` NIL-initialized slots onto the
/// process's canonical call stack (BEAM `allocate`/`allocate_zero`).
///
/// The frame lands on `process.stack()` exactly as the interpreter's
/// `push_y_frame` does, so its Y registers are GC-rooted through
/// `process.stack().y_regs()`. Returns `0` on success and `1` when compiled code
/// must deopt (no live module to pin, or the frame limit was reached).
pub(crate) extern "C" fn jit_alloc_frame(process: *mut Process, y_slots: u64) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 1;
    };
    let Ok(y_slots) = u16::try_from(y_slots) else {
        return 1;
    };
    // Pin the currently-executing module the same way `push_y_frame` does. The
    // frame's return metadata is discarded on deallocate; only the pin (purge
    // protection) and the NIL-initialized Y slots are load-bearing.
    let Some(module) = process.current_module().cloned() else {
        return 1;
    };
    let name = module.name;
    let return_ip = process
        .code_position()
        .map_or(0, |position| position.instruction_pointer);
    match process
        .stack_mut()
        .push_frame(name, return_ip, module, y_slots)
    {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Pops the current Y-register stack frame (BEAM `deallocate`). Returns `0` on
/// success and `1` (deopt) when the stack is empty.
pub(crate) extern "C" fn jit_dealloc_frame(process: *mut Process) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 1;
    };
    match process.stack_mut().pop_frame() {
        Ok(_return_point) => 0,
        Err(_) => 1,
    }
}

/// Honors a heap-need guard (BEAM `test_heap`, and the heap component of
/// `allocate_heap`) by reusing `gc::ensure_space` — the same collector entry the
/// interpreter's `test_heap` uses. `live` bounds the X registers GC roots.
/// Returns `0` when space is available and `1` (deopt) on an unrecoverable
/// heap-full so the interpreter re-runs and raises.
pub(crate) extern "C" fn jit_test_heap(process: *mut Process, heap_need: u64, live: u64) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 1;
    };
    let Ok(heap_need) = usize::try_from(heap_need) else {
        return 1;
    };
    let Ok(live) = usize::try_from(live) else {
        return 1;
    };
    match gc::ensure_space(process, heap_need, live) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Shifts the current frame's Y window (BEAM `trim`). `expected_slots` is the
/// interpreter's `words + remaining` invariant; a mismatch is a malformed trim
/// and deopts. Returns `0` on success and `1` (deopt) otherwise.
pub(crate) extern "C" fn jit_trim_frame(
    process: *mut Process,
    expected_slots: u64,
    remaining: u64,
) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return 1;
    };
    let Ok(expected_slots) = u16::try_from(expected_slots) else {
        return 1;
    };
    let Ok(remaining) = u16::try_from(remaining) else {
        return 1;
    };
    let Ok(frame) = process.stack().current_frame() else {
        return 1;
    };
    if frame.y_slots() != expected_slots {
        return 1;
    }
    match process.stack_mut().trim_y_regs(remaining) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Reads Y register `index` from the current frame and returns its raw term.
///
/// Mirrors the unchecked, trusted nature of the JIT's X-register loads: a
/// well-formed compiled body only reaches a Y index its `allocate` reserved, so
/// the frame and index are always valid here. The safe stack API makes a
/// spurious out-of-bounds read benign (returns NIL) rather than corrupting
/// memory.
pub(crate) extern "C" fn jit_y_read(process: *mut Process, index: u64) -> u64 {
    let Some(process) = process_from_abi(process) else {
        return Term::NIL.raw();
    };
    let Ok(index) = u16::try_from(index) else {
        return Term::NIL.raw();
    };
    process
        .stack()
        .y_reg(index)
        .map_or(Term::NIL.raw(), |term| term.raw())
}

/// Writes `value` to Y register `index` in the current frame.
///
/// Trusted like the X-register stores: a well-formed body only writes a Y index
/// its frame reserved. A spurious out-of-bounds write is dropped by the safe
/// stack API rather than corrupting memory.
pub(crate) extern "C" fn jit_y_write(process: *mut Process, index: u64, value: u64) {
    let Some(process) = process_from_abi(process) else {
        return;
    };
    let Ok(index) = u16::try_from(index) else {
        return;
    };
    let _ = process.stack_mut().set_y_reg(index, Term::from_raw(value));
}

pub(crate) fn process_from_abi(process: *mut Process) -> Option<&'static mut Process> {
    if process.is_null() {
        return None;
    }

    // SAFETY: The JIT raw entry ABI passes the live `Process` pointer that owns
    // the heap for this invocation. The helper uses it only for the duration of
    // the call and rejects null pointers before constructing the reference.
    Some(unsafe { &mut *process })
}

pub(crate) fn alloc_words(process: &mut Process, words: usize) -> *mut u64 {
    if words == 0 {
        return std::ptr::null_mut();
    }

    if gc::ensure_space(process, words, 256).is_err() {
        return std::ptr::null_mut();
    }

    match process.heap_mut().alloc(words) {
        Ok(ptr) => ptr,
        Err(_heap_full) => std::ptr::null_mut(),
    }
}
