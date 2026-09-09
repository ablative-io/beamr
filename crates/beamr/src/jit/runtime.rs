//! Runtime helpers callable from JIT-generated code.

use crate::atom::Atom;
use crate::gc;
use crate::interpreter::{ExecutionResult, run_with_native_services};
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
    if context.module.is_null() || context.registry.is_null() || context.services.is_null() {
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

    // SAFETY: The interpreter installs pointers to borrowed dispatch state for
    // exactly the duration of the native JIT call. Helpers run synchronously
    // before that context is cleared.
    let current_module = unsafe { &*context.module };
    // SAFETY: See `current_module`; the registry pointer has the same lifetime.
    let registry = unsafe { &*context.registry };
    // SAFETY: See `current_module`; the services pointer has the same lifetime.
    let services = unsafe { &*context.services };
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

    // The nested run shares the process's handler stack with the code that
    // entered it, but not its Rust frames: a raise that jumped to an outer
    // handler from in here would resume outer bytecode without ever returning
    // through `invoke_jit`/this helper, leaking one native nesting per caught
    // exception. Floor the handler stack at its current depth so a raise with
    // no in-nest handler leaves through the exception return below instead —
    // the caller re-offers it to the outer handlers at the correct depth.
    // Saved and restored on every exit from the region — including a transfer,
    // where it is still correct: once this helper returns there is no Rust
    // nesting left to protect, so the transferred-out code resumes as ordinary
    // interpreted bytecode and must see the outer handlers at their real depth.
    let saved_handler_floor = process.nested_handler_floor();
    process.set_nested_handler_floor(process.exception_handler_count());
    let result = run_with_native_services(process, current_module, registry, services);
    process.set_nested_handler_floor(saved_handler_floor);

    // ONCE THE NESTED RUN HAS BEGUN, NO OUTCOME MAY LEAVE THROUGH DEOPT.
    //
    // A scheduler-level TRANSFER is not a value: the nested run has already
    // written the position it must resume at. `invoke_jit`'s callers set the
    // process's code position to the compiled function's ENTRY before entering
    // compiled code, so `saved_position` IS that entry — restoring it here
    // would overwrite the resume position, and the deopt return would then
    // re-execute the whole compiled function, replaying the tail external call
    // that produced the transfer. For `Waiting` it would also run a process
    // that is parked with a live suspension, which is aion#85: the park is
    // superseded by a second one and the publish for the handed-out call id is
    // refused, or the replayed call's return value goes nowhere.
    //
    // So a transfer parks itself on the process and leaves module and position
    // untouched; `call_native` takes it before it reads the deopt status, and
    // the deopt word this returns is inert. Every deopt ABOVE returns before
    // `run_with_native_services` is ever called, with nothing committed and the
    // caller's position never disturbed — restart-from-entry is exactly right
    // for those, and they are deliberately left alone.
    //
    // `Yielded` is a TRANSFER for exactly the same reason, MEASURED not assumed
    // (`tests/jit_yield_replay_probe.rs`): the interpreter writes the resume
    // position immediately before yielding, so restoring `saved_position` over
    // it re-enters the compiled function from its entry on the next slice. The
    // probe put a native effect before a slice-exhausting chain in the callee
    // and saw it run TWICE compiled against ONCE interpreted. It does not leave
    // through deopt — it has its own `JitStatus::Yield` channel, already used by
    // the pre-run yield above — so what has to change is the RESTORE, not the
    // return word.
    //
    // `Exited(_)`-without-exception is reached after the nested run too and
    // still takes the path below; its discriminator arm is still owed and it
    // does not move here on suspicion alone.
    let result = match result {
        Ok(transfer @ (ExecutionResult::Waiting | ExecutionResult::DirtyCall { .. })) => {
            process.set_jit_transfer(transfer);
            return JitReturn::deopt(JIT_DEOPT_SENTINEL as u64);
        }
        Ok(ExecutionResult::Yielded) => {
            process.set_jit_status(Some(JitStatus::Yield));
            return JitReturn::yield_(JIT_YIELD_SENTINEL as u64);
        }
        other => other,
    };

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
        // Abnormal exit with no exception recorded. Still a restart-from-entry
        // deopt, and still under measurement (an arm is owed): the replay
        // re-runs the callee interpreted and exits with the same reason, so the
        // outcome is right whenever the replayed prefix is effect-free.
        Ok(ExecutionResult::Exited(_)) => JitReturn::deopt(JIT_DEOPT_SENTINEL as u64),
        // Unreachable: taken by the transfer arm above, before the restore.
        Ok(ExecutionResult::Waiting) | Ok(ExecutionResult::DirtyCall { .. }) => {
            JitReturn::deopt(JIT_DEOPT_SENTINEL as u64)
        }
        // Unreachable: taken by the transfer arm above, before the restore.
        Ok(ExecutionResult::Yielded) => {
            debug_assert!(false, "a post-run yield must leave through the transfer arm");
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

/// Allocates `words` with `roots` held live across the allocation, writing the
/// post-collection values back into `roots`.
///
/// The doctrine this exists to relax, quoted from
/// `interpreter/opcodes/binary/matching.rs:41-42`:
///
/// > Reserve before reading the source: GC moves heap terms, and registers
/// > are the only roots, so no term may be held across a collection.
///
/// The interpreter obeys it by reserving first and then re-reading each term
/// from its operand — see `put_map` (`interpreter/opcodes/closures.rs:476-480`),
/// which calls `ensure_space` and immediately re-reads the source map. **A JIT
/// helper cannot do that.** It receives raw term VALUES rather than operands,
/// and generated code stages some of them into Cranelift stack slots
/// (`jit/ir_map.rs`, `stage_pairs`) that the collector cannot see at all — so
/// re-reading a staged term after a collection returns the same stale word.
///
/// That is what makes the native root stack the only available mechanism, not
/// merely the tidier one. `Process::roots_with_live_x` enumerates it at
/// `process/mod.rs:584` and `replace_roots_with_live_x` forwards it at `:646`,
/// so a term pushed there is a first-class root for the duration of the
/// allocation. The registers-are-the-only-roots half of the doctrine sentence
/// is precisely what this facility suspends, under a scope that always closes.
///
/// Root depth is restored on every exit path, including `words == 0` and
/// allocation failure. A null process cannot reach here: every caller resolves
/// its pointer through [`process_from_abi`] first, which rejects null before
/// any root is pushed.
///
/// If any slot cannot be read back, the allocation is REFUSED — a null return —
/// rather than handing the caller its pre-collection term. Continuing there
/// would reintroduce exactly the stale-`Term` defect this facility exists to
/// remove, so the unreadable case must be the loud one. The invariant that
/// makes it unreachable belongs to the collector, not to this function, and is
/// pinned by `collection_preserves_native_roots` below.
pub(super) fn alloc_words_rooted(
    process: &mut Process,
    words: usize,
    roots: &mut [Term],
) -> *mut u64 {
    let depth = process.native_root_depth();
    // Keep the indices the root stack hands back rather than deriving them as
    // `depth + n`: one fact, one source, and no arithmetic that could disagree
    // with it.
    let mut indices = Vec::with_capacity(roots.len());
    for root in roots.iter() {
        indices.push(process.push_native_root(*root));
    }

    let ptr = alloc_words(process, words);

    // Read back BEFORE truncating. A failed allocation can still have run a
    // collection that moved these terms, so the forwarded values are handed
    // back on every path, not only on success.
    let mut all_recovered = true;
    for (root, index) in roots.iter_mut().zip(indices.iter()) {
        match process.native_root(*index) {
            Some(forwarded) => *root = forwarded,
            None => all_recovered = false,
        }
    }
    process.truncate_native_roots(depth);

    if all_recovered {
        ptr
    } else {
        std::ptr::null_mut()
    }
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

#[cfg(test)]
mod rooting_tests {
    use super::*;
    use crate::atom::AtomTable;
    use crate::native::ProcessContext;
    use crate::term::binary_ref::BinaryRef;
    use crate::term::shared_binary::alloc_binary_word_count;
    use std::sync::Arc;

    fn test_context(process: &mut Process, live_x: u16) -> ProcessContext<'_> {
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::new(AtomTable::with_common_atoms())));
        context.attach_process(process, usize::from(live_x));
        context
    }

    fn fill_until(process: &mut Process, needed: usize) {
        let mut ctx = test_context(process, 1);
        while ctx.process_heap().expect("heap").available() >= needed {
            ctx.alloc_cons(Term::small_int(1), Term::NIL)
                .expect("filler");
        }
    }

    /// The invariant `alloc_words_rooted` must not merely ASSUME: a collection
    /// forwards the native root stack IN PLACE and never truncates or clears
    /// it, so a slot pushed before an allocation is still readable after it.
    ///
    /// This is a claim about the collector, not about the helper, so it is
    /// pinned here at the behaviour itself rather than left as a comment in the
    /// function that depends on it. If this test ever reds, the helper's
    /// refusal path becomes reachable and every JIT caller starts refusing.
    #[test]
    fn collection_preserves_native_roots() {
        let mut process = Process::new(1, 256);
        let raw: Vec<u8> = (1..=32).collect();
        let term = {
            let mut ctx = test_context(&mut process, 0);
            ctx.alloc_binary(&raw).expect("inline binary")
        };

        let depth_before = process.native_root_depth();
        let index = process.push_native_root(term);

        let words = alloc_binary_word_count(raw.len());
        fill_until(&mut process, words);
        assert!(
            process.heap().available() < words,
            "geometry must force a collection"
        );
        assert_eq!(process.heap().old_used(), 0);

        let ptr = alloc_words(&mut process, words);
        assert!(!ptr.is_null(), "allocation must succeed");
        assert!(
            process.heap().old_used() > 0,
            "a collection must actually have run"
        );

        assert_eq!(
            process.native_root_depth(),
            depth_before + 1,
            "a collection must not truncate the native root stack"
        );
        let forwarded = process
            .native_root(index)
            .expect("the slot must still be readable after a collection");
        assert_ne!(
            forwarded, term,
            "a rooted young-heap term must have been forwarded"
        );
        assert_eq!(
            BinaryRef::new(forwarded)
                .expect("forwarded root must still be a binary")
                .as_bytes(),
            raw.as_slice(),
            "forwarding must preserve the bytes"
        );

        process.truncate_native_roots(depth_before);
        assert_eq!(process.native_root_depth(), depth_before);
    }

    /// R2 acceptance: the handed-back terms are the POST-collection values.
    #[test]
    fn rooted_allocation_hands_back_forwarded_terms() {
        let mut process = Process::new(1, 256);
        let raw: Vec<u8> = (1..=40).collect();
        let original = {
            let mut ctx = test_context(&mut process, 0);
            ctx.alloc_binary(&raw).expect("inline binary")
        };

        let words = alloc_binary_word_count(raw.len());
        fill_until(&mut process, words);
        assert!(
            process.heap().available() < words,
            "geometry must force the rooted allocation to collect"
        );
        assert_eq!(process.heap().old_used(), 0);

        let depth_before = process.native_root_depth();
        let mut roots = [original];
        let ptr = alloc_words_rooted(&mut process, words, &mut roots);

        assert!(!ptr.is_null(), "allocation must succeed");
        assert!(
            process.heap().old_used() > 0,
            "the rooted allocation must have run a collection"
        );
        assert_eq!(
            process.native_root_depth(),
            depth_before,
            "root depth must be restored on the success path"
        );
        assert_ne!(
            roots[0], original,
            "the handed-back term must be the post-collection value"
        );
        assert_eq!(
            BinaryRef::new(roots[0])
                .expect("forwarded term must still be a binary")
                .as_bytes(),
            raw.as_slice(),
            "the forwarded term must resolve to the same bytes"
        );
    }

    /// R2 acceptance: depth restored on the `words == 0` path.
    #[test]
    fn rooted_allocation_restores_depth_on_zero_words() {
        let mut process = Process::new(1, 256);
        let term = Term::small_int(7);
        let depth_before = process.native_root_depth();

        let mut roots = [term];
        let ptr = alloc_words_rooted(&mut process, 0, &mut roots);

        assert!(ptr.is_null(), "a zero-word request allocates nothing");
        assert_eq!(process.native_root_depth(), depth_before);
        assert_eq!(roots[0], term, "an immediate is never moved");
    }

    /// R2 acceptance: depth restored when the reservation cannot be satisfied.
    #[test]
    fn rooted_allocation_restores_depth_on_allocation_failure() {
        let mut process = Process::new(1, 256);
        let raw: Vec<u8> = (1..=8).collect();
        let original = {
            let mut ctx = test_context(&mut process, 0);
            ctx.alloc_binary(&raw).expect("inline binary")
        };
        let depth_before = process.native_root_depth();

        // Far beyond any geometry this process can reach.
        let mut roots = [original];
        let ptr = alloc_words_rooted(&mut process, usize::MAX / 2, &mut roots);

        assert!(ptr.is_null(), "an unsatisfiable request must fail");
        assert_eq!(
            process.native_root_depth(),
            depth_before,
            "root depth must be restored on the failure path"
        );
    }

    /// R2 acceptance: nesting composes — an inner scope leaves an outer one
    /// exactly as it found it, and the outer roots are still forwarded.
    #[test]
    fn rooted_scopes_nest_without_leaking_depth() {
        let mut process = Process::new(1, 256);
        let outer_term = {
            let mut ctx = test_context(&mut process, 0);
            ctx.alloc_binary(&[9u8; 16]).expect("outer binary")
        };

        let base = process.native_root_depth();
        process.push_native_root(outer_term);
        let outer_index = process.native_root_depth() - 1;

        let words = alloc_binary_word_count(16);
        fill_until(&mut process, words);
        let mut roots = [outer_term];
        let _ = alloc_words_rooted(&mut process, words, &mut roots);

        assert_eq!(
            process.native_root_depth(),
            base + 1,
            "the inner scope must not disturb the outer root"
        );
        let outer_now = process.native_root(outer_index).expect("outer root");
        assert_eq!(
            outer_now, roots[0],
            "outer and inner views of the same term must agree after forwarding"
        );

        process.truncate_native_roots(base);
        assert_eq!(process.native_root_depth(), base);
    }
}
